//! Phase 11: PlayerCharacter_ctor 0066A740, which the PlayerCharacter block of 00406360 Startup_InitializeGameData (00406774..004067B9, `crate::tes::winmain::initialize_game_data`) runs on FormHeap(0x804) before g_thePlayer
//! 00B333C4 and TESForm_SetFormID 0046C300(0x14, 1). The constructor builds the Character chain (`tes::actor`), swaps the LowProcess for a HighProcess (`tes::actor_process`, D2), creates the Havok character controller
//! and the two collision helpers (`tes::character_controller`, D3), sets the camera FOV on g_worldSceneGraph (the 05-C port of 00411160) and the particle FOV scale 00B2D80C (D5), and reads GetTickCount / _time64 (D6).
//!
//! The player is one entity: the head components, `TesObjectRefr`, `MobileObject`, `Actor`, `Character` and `PlayerCharacter`, the last with the IDB PlayerCharacter_mbrs names and physical offsets. Bytes the constructor
//! does not store stay host zero (uninitialized FormHeap bytes, recorded substitution; the one read of such a byte, +0x71D at 0066AEF7, reads 0 in P11-RUN-001 as on the host).
//! Evidence: evidence/phase11-scope-2026-10-06/disasm-window.txt (0066A740), evidence/phase11-window-2026-10-06/*.txt, P11-RUN-001.

use std::ops::{Deref, DerefMut};

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;
use oblivion_core::x87;

use super::actor::{
    Actor, FORM_FLAG_INITIALIZED, FORM_FLAG_PLAYER_INIT, MobileObject, NI_POINT3_ZERO,
    TesObjectRefr, actor_process_level, construct_character,
};
use super::actor_process::{
    ProcessInputs, actor_process_manager_remove, construct_high_process, copy_process_from,
    delete_low_process,
};
use super::character_controller::{
    ControllerInputs, actor_character_controller, actor_create_character_controller,
    actor_set_character_controller_enabled, construct_camera_collision_helper,
    construct_collision_query_helper, set_collision_layer,
};
use crate::bs::character_controller::BhkCharacterController;
use crate::bs::scene_graphs::{SceneGraphSettings, scene_graph_set_camera_fov};
use crate::gamebryo::scene_update::SceneUpdateRuntime;
use crate::tes::actor::{actor_or_reference_set_parent_cell, write_actor};
use crate::tes::base_extra_list::cell_region_list_or_new;
use crate::tes::equipment::{equipped_in_slot, invalidate_armor_rating};
use crate::tes::form::{FormIdentity, OriginalVtable};
use crate::tes::form_components::{FormRef, TesFullName};
use crate::tes::forms_actors::{TesNpc, npc_base_actor_value, write_npc_face};
use crate::tes::forms_init::{form_type, lookup};
use crate::tes::forms_magic::saved_effect_settings;
use crate::tes::forms_references::reference_position;
use crate::tes::forms_references::reference_world;
use crate::tes::forms_region::{
    RegionDataKind, TesRegion, TesRegionData, TesRegionSound, region_data_of_type,
};
use crate::tes::forms_world::{
    TesObjectCell, TesWorldSpace, cell_grid_x, cell_grid_y, cell_is_interior,
};
use crate::tes::game_settings::{GameSettingCollection, float_at_or_zero};
use crate::tes::magic_menu::inventory_forms;
use crate::tes::mobile_move::process_save_size;
use crate::tes::reference_3d::{Reference3D, no_node_or_stop};
use crate::tes::save_game::{ProcessView, SaveWriter, empty_list, form_id, null_id};
use crate::tes::settings::SettingCollection;
use crate::tes::stats_menu::player;
use crate::tes::tes::TesGlobals;
use crate::tes::tes_grid::{TesGridBoundaries, TesGridGlobals};
use crate::tes::tile::InterfaceCx;
use oblivion_core::pointer_list::PointerList;

/// IDA: the four PlayerCharacter vtables (??_7PlayerCharacter@@6B@, _0, _1, _2).
pub const PLAYER_CHARACTER_VTABLES: [u32; 4] = [0x00A7_3A0C, 0x00A7_3A04, 0x00A7_39BC, 0x00A7_3988];
/// IDA: ??_7?$NiTMapBase@V?$DFALL@E@@IE@@6B@ then ??_7?$NiTMap@IE@@6B@ (PlayerCharacter +0x788).
pub const NI_T_MAP_BASE_U8_VTABLE: u32 = 0x00A7_35A8;
pub const NI_T_MAP_U8_VTABLE: u32 = 0x00A7_3744;
/// IDA: 00662EE0 PlayerCharacter_LoadIdleAnimations (ecx = g_thePlayer, unread): the 00B36BB8 sFirstPersonSkeleton text copied into `char[260]`; `strrchr` for '\\', and a hit takes the 10 bytes of A538D0 "IdleAnims" after it; 005215C0(Str) on 00B362C0. Answers the root's index. Completion: 100%; a text past the buffer overruns it in the exe, and stops here.
/// # Errors
/// A setting with no text, an overrun, or 005215C0's stops.
pub fn player_load_idle_animations(
    world: &mut World,
    host: &mut dyn crate::tes::form_arms::DataLoadHost,
) -> Result<Option<usize>, GameError> {
    const BUFFER: usize = 260;
    const NAME: &[u8] = b"IdleAnims";
    let mut path = world
        .resource::<GameSettingCollection>()
        .text_at(0x00B3_6BB8)
        .ok_or(GameError::unclassified(
            0x00662F00,
            "00662F00: 00B36BB8 has no text",
        ))?;
    if path.len() >= BUFFER {
        return Err(GameError::unclassified(
            0x00662F02,
            "00662F02: sFirstPersonSkeleton overruns Str",
        ));
    }
    if let Some(at) = path.iter().rposition(|&byte| byte == b'\\') {
        if at + 1 + NAME.len() + 1 > BUFFER {
            return Err(GameError::unclassified(
                0x00662F24,
                "00662F24: \"IdleAnims\" overruns Str",
            ));
        }
        path.truncate(at + 1);
        path.extend_from_slice(NAME);
    }
    crate::tes::forms_ai::find_or_create_idle_root(world, &path, host)
}

/// IDA: 004067A0 TESForm_SetFormID(0x14, 1).
pub const PLAYER_FORM_ID: u32 = 0x14;

/// IDA: N/A a fixed array too long for `#[derive(Default)]`, zeroed by default.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ZeroArray<T, const N: usize>(pub [T; N]);

impl<T: Copy + Default, const N: usize> Default for ZeroArray<T, N> {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self([T::default(); N])
    }
}

impl<T, const N: usize> Deref for ZeroArray<T, N> {
    type Target = [T; N];

    /// IDA: N/A Rust's `Deref`.
    fn deref(&self) -> &[T; N] {
        &self.0
    }
}

impl<T, const N: usize> DerefMut for ZeroArray<T, N> {
    /// IDA: N/A Rust's `DerefMut`.
    fn deref_mut(&mut self) -> &mut [T; N] {
        &mut self.0
    }
}

/// IDA: NiTMap<uint, uchar> at PlayerCharacter +0x788 (0x10): +0x00 vtable, +0x04 hash size 37, +0x08 buckets (FormHeap 0x94, memset 0), +0x0C count.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NiTMapU8 {
    pub original_vtable: u32,
    pub hash_size_04: u32,
    pub buckets_08: Vec<u32>,
    pub count_0c: u32,
}

/// IDA: an 8-byte FormHeap list head block {item, next} (0, 0).
pub type ListHeadBlock = [u32; 2];

/// IDA: PlayerCharacter +0x5B4 (0065FBB0): a FormHeap(8) list head of FormHeap(8) attribute-bonus blocks, in list order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AttributeBonusList {
    pub items: Vec<ListHeadBlock>,
}

/// IDA: 0066A740 PlayerCharacter_Constructor (0x804; PlayerCharacter_mbrs). Fields past Character.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct PlayerCharacter {
    pub force_direct_activation_10c: u8,
    pub load_door_follower_move_count_110: u32,
    pub cell_transition_in_progress_114: u8,
    /// +0x115 (1).
    pub follower_relocation_latch_115: u8,
    pub is_in_se_world_116: u8,
    /// +0x117 (not stored).
    pub unset_active_quest_targets_dirty_117: u8,
    pub dialogue_package_118: u32,
    pub accepted_haggle_adjustment_11c: i32,
    pub field_120: u32,
    pub byte_124: u8,
    pub field_128: u32,
    pub is_moving_into_new_space_12c: u8,
    /// +0x130..+0x183 (00668030).
    pub skill_exp_130: [f32; 21],
    pub major_skill_advances_184: i32,
    /// +0x188..+0x1DB (0065D560).
    pub skill_advances_188: [u32; 21],
    pub can_level_up_1dc: u8,
    pub last_ridden_horse_1e0: u32,
    /// +0x1E4 (FormHeap 8).
    pub active_effects_1e4: Option<ListHeadBlock>,
    pub casting_magic_item_1e8: u32,
    pub casting_target_1ec: u32,
    /// +0x1F0 a `CameraCollisionHelper`.
    pub camera_collision_helper_1f0: Option<Entity>,
    /// +0x1F4 a `CollisionQueryHelper`.
    pub collision_query_helper_1f4: Option<Entity>,
    /// +0x1F8 (FormHeap 8).
    pub pending_potion_list_1f8: Option<ListHeadBlock>,
    pub pending_enchanted_equipment_1fc: u32,
    pub jail_sentence_active_200: u8,
    /// +0x204..+0x323.
    pub max_av_modifiers_204: ZeroArray<f32, 72>,
    /// +0x324..+0x443.
    pub script_av_modifiers_324: ZeroArray<f32, 72>,
    pub health_444: f32,
    pub magicka_448: f32,
    pub stamina_44c: f32,
    /// +0x450..+0x56F.
    pub damage_av_modifiers_450: ZeroArray<f32, 72>,
    pub last_activated_load_door_570: u32,
    /// +0x574 (NiPointer, null).
    pub grab_spring_action_574: u32,
    pub grabbed_reference_578: u32,
    pub grab_mode_57c: u32,
    pub field_580: f32,
    pub grab_distance_584: f32,
    /// +0x588..+0x58C third person ...
    pub view_flags_588: [u8; 5],
    pub pc_sleep_hours_590: i32,
    pub pc_sleep_hours_set_594: u8,
    /// +0x598 (fDefaultFOV).
    pub camera_fov_598: f32,
    pub athletics_run_use_timer_59c: f32,
    pub athletics_swim_use_timer_5a0: f32,
    pub sneak_use_timer_5a4: f32,
    /// +0x5A8 (not stored).
    pub unset_sneak_skill_use_actor_nearby_5a8: u8,
    /// +0x5A9 (1).
    pub fast_travel_enabled_5a9: u8,
    pub combat_opponents_5ac: u32,
    /// +0x5B0 (FormHeap 0x54).
    pub chargen_skill_exp_5b0: Option<[f32; 21]>,
    /// +0x5B4 (0065FBB0).
    pub attribute_bonuses_5b4: Option<AttributeBonusList>,
    /// +0x5B8/+0x5B9 (word store).
    pub skill_advance_counts_5b8: u16,
    pub stealth_skill_advance_count_5ba: u8,
    pub training_sessions_used_5bc: u32,
    /// +0x5C0 (byte store).
    pub byte_5c0: u8,
    pub pending_activation_ref_5c4: u32,
    /// +0x5C8 the first-person skin info (00667BE0's 00478730).
    pub first_person_skin_info_5c8: Option<Entity>,
    /// +0x5CC the first-person ActorAnimData (00667BE0's 00473EB0).
    pub first_person_anim_data_5cc: Option<Entity>,
    /// +0x5D0 the first-person node "Player1stPerson" (NiPointer; 00667BE0).
    pub first_person_node_5d0: Option<Entity>,
    /// +0x5D4 (118.0).
    pub camera_eye_height_offset_5d4: f32,
    /// +0x5D8 (NiPointer, null).
    pub first_person_biped_root_5d8: u32,
    pub first_person_biped_anim_data_5dc: u32,
    pub active_magic_shader_hit_effect_5e0: u32,
    pub known_topics_5e4: [u32; 2],
    /// +0x5E4's topics, as a list.
    pub known_topics: Vec<Entity>,
    pub known_quest_stage_items_5ec: [u32; 2],
    pub active_quest_5f4: u32,
    pub active_quest_targets_5f8: [u32; 2],
    pub dialogue_cooldown_active_600: u8,
    /// +0x604 (not stored).
    pub unset_dialogue_cooldown_elapsed_604: f32,
    pub prison_days_remaining_608: i32,
    pub field_60c: u32,
    pub jail_guard_pursuit_active_610: u8,
    pub is_a_murderer_611: u8,
    pub amount_sold_stolen_614: i32,
    pub actor_distance_list_sort_timer_618: f32,
    pub z_rotation_offset_61c: f32,
    pub byte_620: u8,
    /// +0x624 the spell's MagicItem part, `spell + 0x18` (0040F03A).
    pub active_magic_item_624: Option<Entity>,
    pub book_628: u32,
    pub custom_marker_position_62c: [u32; 3],
    pub custom_marker_location_638: u32,
    pub custom_marker_route_reference_63c: u32,
    pub bow_draw_timer_640: f32,
    pub birthsign_644: u32,
    pub field_648: u32,
    /// +0x64C (0x48).
    pub field_64c: u32,
    pub chargen_suggested_class_650: u32,
    pub previous_chargen_class_654: u32,
    /// +0x658..+0x6DF.
    pub misc_stats_658: ZeroArray<u32, 34>,
    pub alchemy_item_6e0: u32,
    pub vampire_has_fed_6e4: u8,
    pub is_in_chargen_6e5: u8,
    pub telekinesis_targeting_enabled_6e6: u8,
    /// +0x6E8 (00667947): the region whose weather data ranks highest.
    pub region_6e8: Option<Entity>,
    /// +0x6EC the BSSimpleList of the regions' RDSD entries, as 00446CB0 prepends them.
    pub region_sound_entries_6ec: oblivion_core::pointer_list::PointerList<TesRegionSound>,
    pub fame_6f4: i32,
    pub infamy_6f8: i32,
    pub infamy_bounty_accumulator_6fc: i32,
    pub se_world_crime_gold_700: f32,
    /// +0x704/+0x708 (0065FD20): the BSSimpleList head, empty here.
    pub jail_marker_candidates_704: [u32; 2],
    /// IDA: N/A the items of +0x704's list, which 0065FD20 fills. `jail_marker_candidates_704` is its head as the constructor leaves it.
    pub jail_markers_704: oblivion_core::pointer_list::PointerList<Entity>,
    pub initial_state_buffer_70c: u32,
    /// IDA: N/A the buffer +0x70C points to, 006632A0's.
    pub initial_state: Option<Vec<u8>>,
    /// +0x710 (GetTickCount).
    pub play_time_last_tick_710: u32,
    pub play_time_milliseconds_714: u32,
    /// +0x718 (_time64 low dword).
    pub initial_timestamp_low_718: u32,
    pub sit_sleep_entry_pending_71c: u8,
    /// +0x71D (read before stored).
    pub sit_sleep_exit_pending_71d: u8,
    pub cell_fully_inside_border_region_71e: u8,
    pub border_recovery_pending_71f: u8,
    pub last_valid_border_position_720: [u32; 3],
    pub last_valid_border_location_72c: u32,
    /// +0x730 (006679B5): a FormHeap BSSimpleArray of the cell's regions with form flag 0x40.
    pub border_region_candidates_730: Option<Vec<Entity>>,
    /// +0x734 (fDifficulty:GamePlay).
    pub game_difficulty_level_734: f32,
    pub guard_combat_active_738: u8,
    pub map_marker_list_73c: [u32; 2],
    pub map_marker_list_worldspace_744: u32,
    pub fly_camera_enabled_748: u8,
    pub fly_camera_yaw_74c: f32,
    pub fly_camera_pitch_750: f32,
    pub fly_camera_position_754: [u32; 3],
    pub cached_sound_form_id_760: u32,
    pub cached_sound_handle_764: u32,
    pub magic_failure_sound_handles_768: [u32; 6],
    pub pending_dropped_references_780: [u32; 2],
    /// +0x788..+0x797.
    pub random_teleport_index_by_door_788: NiTMapU8,
    /// +0x798 (NiPointer, null).
    pub first_person_light_798: u32,
    /// +0x79C (NiPointer, null).
    pub third_person_light_79c: u32,
    pub combat_opponent_count_7a0: i32,
    /// +0x7A4..+0x7F7.
    pub required_skill_exp_7a4: [u32; 21],
    pub new_game_intro_pending_7f8: u8,
    pub no_charge_message_shown_7f9: u8,
    pub drop_angle_accumulator_7fc: f32,
    pub last_drop_placement_angle_800: f32,
}

/// IDA: the player-side globals the constructor writes, and 00B333C4 g_thePlayer.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct PlayerGlobals {
    /// 00B333C4 (004067A6).
    pub the_player_b333c4: Option<Entity>,
    /// 00B3BB24 (GMST fVanityModeWheelDefault).
    pub vanity_wheel_b3bb24: f32,
    /// 00B3BACC.
    pub vanity_wheel_copy_b3bacc: f32,
    /// 00B3BB38..00B3BB40 (g_niPoint3Zero).
    pub point_b3bb38: [u32; 3],
    /// 00B14E4D (image 1; stored 1).
    pub byte_b14e4d: u8,
    /// 00B3BAFC.
    pub b3bafc: f32,
    /// 00B2D80C g_particleFOVScale (image 1.0).
    pub particle_fov_scale_b2d80c: f32,
}

impl Default for PlayerGlobals {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            the_player_b333c4: None,
            vanity_wheel_b3bb24: 0.0,
            vanity_wheel_copy_b3bacc: 0.0,
            point_b3bb38: [0; 3],
            byte_b14e4d: 1,
            b3bafc: 0.0,
            particle_fov_scale_b2d80c: 1.0,
        }
    }
}

/// IDA: GMSTs and INI values the constructor reads by value, raw bits: 00B36B70 fVanityModeWheelDefault, 00B36940 fIdleChatterCommentTimer (HighProcess +0x19C), 00B14EB0 fDifficulty:GamePlay, 00B0313C fDefaultFOV.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlayerSettingInputs {
    pub vanity_mode_wheel_default: u32,
    pub idle_chatter_comment_timer: u32,
    pub difficulty: u32,
    pub default_fov: u32,
}

// IDA: N/A reads `PlayerSettingInputs` from the GMST collection and the 03-E INI collection. Completion: 100% of the sources.
pub fn player_setting_inputs(
    settings: &GameSettingCollection,
    ini: &SettingCollection,
) -> Result<PlayerSettingInputs, GameError> {
    let gmst = |key: &str| {
        settings
            .find(key)
            .map(|setting| setting.raw)
            .ok_or(GameError::unclassified(
                0,
                "PlayerCharacter_ctor: missing GMST",
            ))
    };
    let setting = |address: u32| {
        ini.find_address(address)
            .map(|setting| setting.raw)
            .ok_or(GameError::unclassified(
                0,
                "PlayerCharacter_ctor: missing INI setting",
            ))
    };
    Ok(PlayerSettingInputs {
        vanity_mode_wheel_default: gmst("fVanityModeWheelDefault")?,
        idle_chatter_comment_timer: gmst("fIdleChatterCommentTimer")?,
        difficulty: setting(0x00B1_4EB0)?,
        default_fov: setting(0x00B0_313C)?,
    })
}

/// IDA: N/A the inputs and seams of the constructor and the window.
pub struct PlayerInputs<'a> {
    pub process: ProcessInputs<'a>,
    pub controller: ControllerInputs,
    pub settings: PlayerSettingInputs,
    /// IDA: 00B333CC g_worldSceneGraph and what 00411160 reads (05-C port). A null graph faults in the original ([ecx+0xEC]): host stop.
    pub world_scene_graph: Option<Entity>,
    pub scene_settings: &'a SceneGraphSettings,
    pub scene_runtime: &'a mut dyn SceneUpdateRuntime,
    /// IDA: 0066AD96 GetTickCount (D6; tests inject it).
    pub get_tick_count: &'a mut dyn FnMut() -> u32,
    /// IDA: 0066AEED _time64(0) (D6; tests inject it).
    pub time64: &'a mut dyn FnMut() -> i64,
}

// IDA: 007B70E0 Particle_UpdateFOVScale(fovDegrees) (__cdecl): 00B2D80C = tan(fov * 0.5 * (pi / 180)) / tan(0.6544984579086304) with float stores between the steps (24-bit x87): __CIsin and __CIcos return full precision, whatever the control word, and each result is then stored as a float, so here double-precision
// sin/cos rounded once (host CRT, substitution). The C++ uses sinf and cosf. Completion: 100% of the order.
pub fn particle_update_fov_scale(fov_degrees: f32, globals: &mut PlayerGlobals) {
    // 007B70E3..007B70F3 fov * 0.5 (A2FAA0) * (pi / 180) (A31C78, float pi / 180 widened): two 24-bit products, then the float store.
    let half = fov_degrees * 0.5;
    let angle = half * f32::from_bits(0x3C8E_FA35);
    // 007B70F7..007B7125 sin and cos of the float angle, each stored as a float, the quotient stored as a float.
    let tangent = (f64::from(angle).sin() as f32) / (f64::from(angle).cos() as f32);
    // 007B7129..007B7155 the same with `fld qword` of the double 0.6544984579086304 (A690D0, 37.5 degrees) as the argument, not rounded to a float first.
    let reference = 0.654_498_457_908_630_4_f64;
    let reference_tangent = (reference.sin() as f32) / (reference.cos() as f32);
    // 007B7159 00B2D80C = tangent / reference tangent.
    globals.particle_fov_scale_b2d80c = tangent / reference_tangent;
}

// IDA: N/A a host stop on a pointer field the constructor releases, which is null on every reached path.
const fn stop_if_set(value: u32, site: &'static str) -> Result<(), GameError> {
    if value != 0 {
        return Err(GameError::from_legacy_message(site));
    }
    Ok(())
}

// IDA: 0066A740 PlayerCharacter_Constructor (__thiscall, FormHeap 0x804). Completion: 100% of the store order of the reached branches; unreached arms stop.
// TODO: [ ] 0060CDB0's other arms and the light, first-person and grab-spring releases, when a later phase reaches them (M4 or later).
#[expect(
    clippy::field_reassign_with_default,
    reason = "0066A740's stores stay one statement each, in original store order"
)]
pub fn construct_player_character(
    world: &mut World,
    inputs: &mut PlayerInputs<'_>,
) -> Result<Entity, GameError> {
    // 0066A76D Character_Constructor.
    let this = construct_character(world, &inputs.process)?;
    // 0066A774..0066A788 the four PlayerCharacter vtables.
    world
        .get_mut::<OriginalVtable>(this)
        .expect("just spawned")
        .0 = PLAYER_CHARACTER_VTABLES[0];
    world
        .get_mut::<TesObjectRefr>(this)
        .expect("just inserted")
        .child_cell_vtable_18 = PLAYER_CHARACTER_VTABLES[1];
    {
        let mut actor = world.get_mut::<Actor>(this).expect("just inserted");
        actor.magic_caster_5c.original_vtable = PLAYER_CHARACTER_VTABLES[2];
        actor.magic_target_68.original_vtable = PLAYER_CHARACTER_VTABLES[3];
    }
    let mut p = PlayerCharacter::default();
    // 0066A793..0066A7F3 the pointer / list clears in store order.
    p.grab_spring_action_574 = 0;
    p.first_person_node_5d0 = None;
    p.first_person_biped_root_5d8 = 0;
    p.known_topics_5e4 = [0; 2];
    p.known_quest_stage_items_5ec = [0; 2];
    p.active_quest_targets_5f8 = [0; 2];
    p.region_sound_entries_6ec = oblivion_core::pointer_list::PointerList::default();
    p.jail_marker_candidates_704 = [0; 2];
    p.map_marker_list_73c = [0; 2];
    p.pending_dropped_references_780 = [0; 2];
    // 0066A7FB..0066A85A NiTMap<uint, uchar> at +0x788: +0x78C = 37, the NiTMapBase vtable, +0x794 = 0, FormHeap(0x94) -> +0x790 memset 0, then the NiTMap vtable; +0x798/+0x79C = 0.
    let map = &mut p.random_teleport_index_by_door_788;
    map.hash_size_04 = 37;
    map.original_vtable = NI_T_MAP_BASE_U8_VTABLE;
    map.count_0c = 0;
    map.buckets_08 = vec![0; 37];
    map.original_vtable = NI_T_MAP_U8_VTABLE;
    p.first_person_light_798 = 0;
    p.third_person_light_79c = 0;
    // 0066A868 0046AB80(0): flags &= ~8 (TESForm_SetInitialized(flag): flags |= 8 or &= ~8).
    world
        .get_mut::<FormIdentity>(this)
        .expect("just spawned")
        .flags_08 &= !FORM_FLAG_INITIALIZED;
    // 0066A86D 00B3BB24 = GMST fVanityModeWheelDefault (00B36B70).
    {
        let vanity = x87::float_copy(inputs.settings.vanity_mode_wheel_default);
        world.resource_mut::<PlayerGlobals>().vanity_wheel_b3bb24 = vanity;
    }
    // 0066A879..0066A89B 72 iterations: +0x204 + 4i, +0x324 + 4i, +0x450 + 4i = 0.0.
    for index in 0..72 {
        p.max_av_modifiers_204[index] = 0.0;
        p.script_av_modifiers_324[index] = 0.0;
        p.damage_av_modifiers_450[index] = 0.0;
    }
    // 0066A89D..0066A910 in store order.
    p.stamina_44c = 0.0;
    p.health_444 = 0.0;
    p.pc_sleep_hours_590 = 0;
    p.magicka_448 = 0.0;
    p.accepted_haggle_adjustment_11c = 0;
    p.athletics_run_use_timer_59c = 0.0;
    p.field_120 = 0;
    p.athletics_swim_use_timer_5a0 = 0.0;
    p.field_128 = 0;
    p.sneak_use_timer_5a4 = 0.0;
    p.force_direct_activation_10c = 0;
    p.amount_sold_stolen_614 = 0;
    p.field_648 = 0;
    p.is_moving_into_new_space_12c = 0;
    p.fast_travel_enabled_5a9 = 1;
    p.is_in_chargen_6e5 = 0;
    p.last_ridden_horse_1e0 = 0;
    p.field_64c = 0x48;
    p.chargen_suggested_class_650 = 0;
    // 0066A916..0066A937 FormHeap(0x54) -> +0x5B0 (no null check: a failed allocation faults on the first store); its 21 floats = 0.0.
    p.chargen_skill_exp_5b0 = Some([0.0; 21]);
    // 0066A941..0066A953 +0x6F4, +0x6F8, +0x654 = 0, +0x114 = 0.
    p.fame_6f4 = 0;
    p.infamy_6f8 = 0;
    p.previous_chargen_class_654 = 0;
    p.cell_transition_in_progress_114 = 0;
    // 0066A959..0066A9A3 release the two lights (+0x798, +0x79C: null, skipped).
    stop_if_set(
        p.first_person_light_798 | p.third_person_light_79c,
        "0066A963: releasing a player light is not reached",
    )?;
    // 0066A9AE +0x115 = 1 (before the HighProcess allocation).
    p.follower_relocation_latch_115 = 1;
    // 0066A9A9..0066A9D7 FormHeap(0x2EC) HighProcess_ctor (a failed allocation leaves edi = 0 and the vt+4 call faults: host stop).
    let high = construct_high_process(world, &mut inputs.process);
    // 0066A9E7 HighProcess vt+4 0060CDB0(old process).
    let old_process = world
        .get::<MobileObject>(this)
        .expect("just inserted")
        .process_58;
    copy_process_from(world, high, old_process)?;
    // 0066A9EB..0066A9F7 00674550(g_actorProcessManager, this, 00659A00(this)) (level 3).
    let save_load_active = inputs.process.save_load_active;
    let level = actor_process_level(world, this);
    actor_process_manager_remove(world, this, level, save_load_active);
    // 0066A9FC..0066AA09 the old process's deleting destructor (vt+0, 1) when set.
    let old_process = world
        .get::<MobileObject>(this)
        .expect("just inserted")
        .process_58;
    if let Some(old_process) = old_process {
        delete_low_process(world, old_process)?;
    }
    // 0066AA0D +0x58 = the HighProcess; 0066AA10..0066AA1C 00674550 with its level (0).
    world
        .get_mut::<MobileObject>(this)
        .expect("just inserted")
        .process_58 = Some(high);
    let level = actor_process_level(world, this);
    actor_process_manager_remove(world, this, level, save_load_active);
    // 0066AA21..0066AA46 +0x1E8 = +0x1EC = 0; FormHeap(8) {0, 0} -> +0x1E4.
    p.casting_magic_item_1e8 = 0;
    p.casting_target_1ec = 0;
    p.active_effects_1e4 = Some([0; 2]);
    // The component goes in here: 0065B750 and everything below read the actor through the world.
    world.entity_mut(this).insert(p);
    // 0066AA4C 0065B750 Actor_CreateCharacterController.
    actor_create_character_controller(world, this, inputs.controller)?;
    // 0066AA53 0065A2C0: with a controller, +0x3B0 = 1000, 0065A310(1), collision layer 9 on the +0x364 / +0x368 objects, the camera helper (8.0, 9) and the query helper (2.0, 9).
    let (camera_helper, query_helper) =
        if let Some(controller) = actor_character_controller(world, this) {
            let (phantom, proxy) = {
                let mut record = world
                    .get_mut::<BhkCharacterController>(controller)
                    .expect("+0x118 is a controller");
                record.field_3b0 = 1000;
                (record.phantom_364, record.avoid_box_368)
            };
            actor_set_character_controller_enabled(world, this, true);
            if let Some(phantom) = phantom {
                set_collision_layer(world, phantom, 9);
            }
            if let Some(proxy) = proxy {
                set_collision_layer(world, proxy, 9);
            }
            // 0066AA9D..0066AADC FormHeap(8) 00532BC0(8.0, 9) -> +0x1F0.
            let camera = construct_camera_collision_helper(world, 8.0, 9);
            // 0066AACB..0066AB19 j_MemoryHeap_Allocate(0x1C0, 1), 16-aligned, 005358F0(2.0, 9) -> +0x1F4.
            let query = construct_collision_query_helper(
                world,
                2.0,
                9,
                inputs.controller.query_helper_alignment,
            );
            (Some(camera), Some(query))
        } else {
            // 0066AB21/0066AB27 both helpers = 0.
            (None, None)
        };
    // 0066AB4B..0066AB6C fov = fDefaultFOV; +0x598 = fov.
    let fov = x87::float_copy(inputs.settings.default_fov);
    {
        let mut player = world
            .get_mut::<PlayerCharacter>(this)
            .expect("just inserted");
        player.camera_collision_helper_1f0 = camera_helper;
        player.collision_query_helper_1f4 = query_helper;
        // 0066AB2D..0066AB45 +0x588..+0x58C = 0.
        player.view_flags_588 = [0; 5];
        player.camera_fov_598 = fov;
    }
    // SceneGraph_SetCameraFOV(g_worldSceneGraph, +0x598, 0) (05-C port).
    let Some(graph) = inputs.world_scene_graph else {
        return Err(GameError::unclassified(
            0x0066AB6C,
            "0066AB6C: SceneGraph_SetCameraFOV on a null g_worldSceneGraph",
        ));
    };
    scene_graph_set_camera_fov(
        world,
        graph,
        fov,
        false,
        inputs.scene_settings,
        inputs.scene_runtime,
    )?;
    // 0066AB71..0066AB79 Particle_UpdateFOVScale(fov).
    particle_update_fov_scale(fov, &mut world.resource_mut::<PlayerGlobals>());
    // The rest of the stores, on the component in the world.
    let mut p = world
        .get::<PlayerCharacter>(this)
        .expect("just inserted")
        .clone();
    // 0066AB7E..0066ABEF +0x5C8 = +0x5CC = 0; release +0x5D0 (null); +0x5D4 = 118.0; release +0x5D8 (null); +0x5DC = 0; +0x124 = 0.
    p.first_person_skin_info_5c8 = None;
    p.first_person_anim_data_5cc = None;
    if p.first_person_node_5d0.is_some() {
        return Err(GameError::unclassified(
            0x0066AB97,
            "0066AB97: releasing the first-person node is not reached",
        ));
    }
    p.camera_eye_height_offset_5d4 = 118.0;
    stop_if_set(
        p.first_person_biped_root_5d8,
        "0066ABCB: releasing the first-person biped root is not reached",
    )?;
    p.first_person_biped_anim_data_5dc = 0;
    p.byte_124 = 0;
    // 0066ABF5..0066AC34 00B3BACC = 00B3BB24; 00B3BB38 = g_niPoint3Zero; 00B14E4D = 1; +0x700 = 0.0; +0x6FC = 0.
    {
        let mut globals = world.resource_mut::<PlayerGlobals>();
        globals.vanity_wheel_copy_b3bacc = globals.vanity_wheel_b3bb24;
        globals.point_b3bb38 = NI_POINT3_ZERO;
        globals.byte_b14e4d = 1;
    }
    p.se_world_crime_gold_700 = 0.0;
    p.infamy_bounty_accumulator_6fc = 0;
    // 0066AC3A 00668030 PlayerCharacter_ClearSkillExperience: for the 21 skill indices of group 2 (00565BE0: code = 12 + index; 00565C50: index = code - 12), a code in 12..32 clears the float at +0x130 + 4 * index.
    for index in 0..21 {
        p.skill_exp_130[index] = 0.0;
    }
    // 0066AC41 +0x5B4 = 0; 0066AC47 0065FBB0 PlayerCharacter_PushAttributeBonusLevel: without a head block allocate one {0, 0} (0065FC34..0065FC53); 0065FC59..0065FC86 an empty head takes a new {0, 0} block.
    // TODO: [ ] 0065FBBE's existing-list arm (M4, Phase 13).
    p.attribute_bonuses_5b4 = Some(AttributeBonusList {
        items: vec![[0; 2]],
    });
    // +0x5B8 word = 0, +0x5BA = 0.
    p.skill_advance_counts_5b8 = 0;
    p.stealth_skill_advance_count_5ba = 0;
    // 0066AC5B 0065D560 PlayerCharacter_ClearSkillAdvances: +0x188..+0x1D8 (21 dwords) = 0.
    p.skill_advances_188 = [0; 21];
    // 0066AC60..0066ACE1 in store order.
    p.z_rotation_offset_61c = 0.0;
    p.major_skill_advances_184 = 0;
    p.can_level_up_1dc = 0;
    p.dialogue_cooldown_active_600 = 0;
    p.byte_5c0 = 0;
    p.dialogue_package_118 = 0;
    p.prison_days_remaining_608 = 0;
    p.field_60c = 0;
    p.jail_guard_pursuit_active_610 = 0;
    p.is_a_murderer_611 = 0;
    p.byte_620 = 0;
    p.active_magic_item_624 = None;
    p.book_628 = 0;
    p.custom_marker_position_62c = NI_POINT3_ZERO;
    p.custom_marker_location_638 = 0;
    p.custom_marker_route_reference_63c = 0;
    p.active_quest_5f4 = 0;
    // 0066ACE7..0066ACFE FormHeap(8) {0, 0} -> +0x1F8.
    p.pending_potion_list_1f8 = Some([0; 2]);
    // 0066AD04..0066AD40 in store order (00B3BAFC = 0.0).
    p.bow_draw_timer_640 = 0.0;
    p.pending_enchanted_equipment_1fc = 0;
    world.resource_mut::<PlayerGlobals>().b3bafc = 0.0;
    p.birthsign_644 = 0;
    p.training_sessions_used_5bc = 0;
    p.pending_activation_ref_5c4 = 0;
    p.pc_sleep_hours_set_594 = 0;
    p.alchemy_item_6e0 = 0;
    p.vampire_has_fed_6e4 = 0;
    p.telekinesis_targeting_enabled_6e6 = 0;
    p.region_6e8 = None;
    // 0066AD46..0066AD6C release +0x574 (null, skipped with its clear).
    stop_if_set(
        p.grab_spring_action_574,
        "0066AD50: releasing the grab spring action is not reached",
    )?;
    // 0066AD72..0066AD90 +0x580 = 0.0, +0x578 = 0, +0x584 = 0.0, +0x57C, +0x70C, +0x714 = 0.
    p.field_580 = 0.0;
    p.grabbed_reference_578 = 0;
    p.grab_distance_584 = 0.0;
    p.grab_mode_57c = 0;
    p.initial_state_buffer_70c = 0;
    p.play_time_milliseconds_714 = 0;
    // 0066AD96 +0x710 = GetTickCount().
    p.play_time_last_tick_710 = (inputs.get_tick_count)();
    // 0066ADA2..0066AEE7 +0x6C8..+0x6DC, +0x658..+0x6C4, +0x7A4..+0x7F0, +0x7F4 = 0.
    for index in 28..34 {
        p.misc_stats_658[index] = 0;
    }
    for index in 0..28 {
        p.misc_stats_658[index] = 0;
    }
    p.required_skill_exp_7a4 = [0; 21];
    // 0066AEED _time64(0); 0066AEF7 the unset +0x71D byte is read (P11-RUN-001: 0); +0x718 = the low dword; +0x71C = 0; vt+0x1BC (0065A310) with ([+0x71D] == 0).
    let time = (inputs.time64)();
    let exit_pending_clear = p.sit_sleep_exit_pending_71d == 0;
    p.initial_timestamp_low_718 = time as u32;
    p.sit_sleep_entry_pending_71c = 0;
    actor_set_character_controller_enabled(world, this, exit_pending_clear);
    // 0066AF1A..0066AF37 +0x71D = 0; vt+0x1BC with ([+0x71C] == 0), i.e. 1.
    let entry_pending_clear = p.sit_sleep_entry_pending_71c == 0;
    p.sit_sleep_exit_pending_71d = 0;
    actor_set_character_controller_enabled(world, this, entry_pending_clear);
    // 0066AF39..0066AFFB in store order.
    p.cell_fully_inside_border_region_71e = 0;
    p.border_recovery_pending_71f = 0;
    p.last_valid_border_position_720 = NI_POINT3_ZERO;
    p.last_valid_border_location_72c = 0;
    p.border_region_candidates_730 = None;
    p.game_difficulty_level_734 = x87::float_copy(inputs.settings.difficulty);
    p.active_magic_shader_hit_effect_5e0 = 0;
    p.guard_combat_active_738 = 0;
    p.load_door_follower_move_count_110 = 0;
    p.map_marker_list_worldspace_744 = 0;
    p.fly_camera_enabled_748 = 0;
    p.fly_camera_yaw_74c = 0.0;
    p.fly_camera_pitch_750 = 0.0;
    p.fly_camera_position_754 = NI_POINT3_ZERO;
    p.jail_sentence_active_200 = 0;
    p.cached_sound_form_id_760 = 0;
    p.cached_sound_handle_764 = 0;
    p.magic_failure_sound_handles_768 = [0; 6];
    p.last_activated_load_door_570 = 0;
    // 0066B00D..0066B033 free the +0x784 dropped-reference nodes (empty).
    stop_if_set(
        p.pending_dropped_references_780[1],
        "0066B015: freeing dropped-reference nodes is not reached",
    )?;
    // 0066B035..0066B06D in store order; flags |= 0x400.
    p.pending_dropped_references_780[0] = 0;
    p.combat_opponent_count_7a0 = 0;
    p.new_game_intro_pending_7f8 = 0;
    p.combat_opponents_5ac = 0;
    p.no_charge_message_shown_7f9 = 0;
    p.actor_distance_list_sort_timer_618 = 0.0;
    p.drop_angle_accumulator_7fc = 0.0;
    p.last_drop_placement_angle_800 = 0.0;
    p.is_in_se_world_116 = 0;
    world.entity_mut(this).insert(p);
    world
        .get_mut::<FormIdentity>(this)
        .expect("just spawned")
        .flags_08 |= FORM_FLAG_PLAYER_INIT;
    Ok(this)
}

// ---- 13c: the actor values 00406360's tail reads (0040687B, 004068AA vt+0x284) ----

/// IDA: 005E0260..: base-form GMST 00B37720 fPCBaseHealthMult, by its address.
const PC_BASE_MAGICKA_MULT: u32 = 0x00B3_7710;
const PC_BASE_HEALTH_MULT: u32 = 0x00B3_7720;

// IDA: 00548020(strength, endurance): `fild` the second argument, `fmul` the GMST float, 009828C0. At 24 bits that's one rounding to float, then truncation. Completion: 100%.
fn pc_base_health(endurance: i32, multiplier: f32) -> i32 {
    x87::ftol((f64::from(endurance) * f64::from(multiplier)) as f32)
}

// IDA: 005E2210(av, &additive, &multiplier): additive = 0.0, multiplier = 1.0, then by 005E234C's table. 8: vt+0x284(5), vt+0x284(0), 00548020; 10: vt+0x284(2), (3), (5), (0), 005479D0's `add`s. Each result goes `fild`, `fstp` to the additive. Completion: 100% of 8 and 10.
// 9: vt+0x288(40) / 10.0 is the multiplier, then vt+0x284(1), 005482B0 with the byte `this == g_thePlayer`, which is set here: `fild`, `fmul` fPCBaseMagickaMult, `fiadd`, `__ftol2_sse`, all at 24 bits.
// TODO: [ ] 005E2321 the Encumbrance arm (00547ED0), when a caller reads it.
fn player_derived_actor_value(
    world: &World,
    player: Entity,
    actor_value: u32,
    multiplier: &mut f32,
) -> Result<f32, GameError> {
    match actor_value {
        9 => {
            let scale = player_get_actor_value_float(world, player, 40)?;
            *multiplier = x87::stored_float(f64::from(scale) / 10.0);
            let intelligence = player_get_actor_value(world, player, 1)?;
            let base = world
                .resource::<GameSettingCollection>()
                .raw_at(PC_BASE_MAGICKA_MULT)
                .map(f32::from_bits)
                .ok_or(GameError::unclassified(
                    0x005482B7,
                    "005482B7: no fPCBaseMagickaMult",
                ))?;
            let product = x87::stored_float(f64::from(intelligence) * f64::from(base));
            let sum = x87::stored_float(f64::from(product) + f64::from(intelligence));
            Ok(x87::ftol(sum) as f32)
        }
        8 => {
            let endurance = player_get_actor_value(world, player, 5)?;
            let _strength = player_get_actor_value(world, player, 0)?;
            let multiplier = world
                .resource::<GameSettingCollection>()
                .raw_at(PC_BASE_HEALTH_MULT)
                .map(f32::from_bits)
                .ok_or(GameError::unclassified(
                    0x00548024,
                    "00548024: no fPCBaseHealthMult",
                ))?;
            Ok(pc_base_health(endurance, multiplier) as f32)
        }
        10 => {
            let willpower = player_get_actor_value(world, player, 2)?;
            let agility = player_get_actor_value(world, player, 3)?;
            let endurance = player_get_actor_value(world, player, 5)?;
            let strength = player_get_actor_value(world, player, 0)?;
            Ok(strength
                .wrapping_add(endurance)
                .wrapping_add(agility)
                .wrapping_add(willpower) as f32)
        }
        _ => Err(GameError::unclassified(
            0x005E2241,
            "005E2241: player actor value 11",
        )),
    }
}

// IDA: 005EAD00 the calculated actor value. Multiplier = 1.0, additive = 0.0. For 8..=11 on g_thePlayer, 005E2210; its multiplier reading exactly 0.0 becomes 1.0 (`test ah, 0x44`, so a NaN stays). 37..=39 stop. Then vt+0x170's base when vt+0x190 (null faults at 005EAD78), vt+0x128(av), and `fild` + additive, `fmul` multiplier at 24 bits, each rounded to float. Completion: 100% of what the checks reach.
// TODO: [ ] 005EADA6 37..=39 (vt+0x208, vt+0x204, 0041FC90), when a caller reads them.
fn calculated_actor_value(
    world: &World,
    player: Entity,
    actor_value: u32,
) -> Result<f32, GameError> {
    let mut multiplier = 1.0_f32;
    let mut additive = 0.0_f32;
    if actor_value.wrapping_sub(8) <= 3
        && world
            .get_resource::<PlayerGlobals>()
            .is_some_and(|globals| globals.the_player_b333c4 == Some(player))
    {
        additive = player_derived_actor_value(world, player, actor_value, &mut multiplier)?;
        if multiplier == 0.0 {
            multiplier = 1.0;
        }
    } else if actor_value.wrapping_sub(0x25) <= 2 {
        return Err(GameError::unclassified(
            0x005EADA6,
            "005EADA6: actor values 37..=39",
        ));
    }
    let base = world
        .get::<TesObjectRefr>(player)
        .expect("a reference")
        .base_form_1c
        .ok_or(GameError::unclassified(
            0x005EAD78,
            "005EAD78: vt+0x128 on a null base form",
        ))?;
    let value = npc_base_actor_value(world, base, actor_value)?;
    let sum = (f64::from(value) + f64::from(additive)) as f32;
    Ok(sum * multiplier)
}

// IDA: 005F1910 the floor: 005EAD00, `fstp` float; 009828C0, `fild`, `fstp` float t; value - t `fcomp` the qword 0.0 at A2FC68, `test ah, 1`: below or unordered takes `fsub` the qword 1.0 at A2F928; `fstp` float; 009828C0. Completion: 100%.
pub fn floor_calculated_actor_value(
    world: &World,
    player: Entity,
    actor_value: u32,
) -> Result<i32, GameError> {
    let value = calculated_actor_value(world, player, actor_value)?;
    let mut truncated = x87::ftol(value) as f32;
    let difference = value - truncated;
    // 005F1945 `test ah, 1`: C0 is set for below and for unordered, so a NaN takes the `fsub` too.
    #[allow(
        clippy::neg_cmp_op_on_partial_ord,
        reason = "005F1947 takes the fsub when the fcomp is below or unordered"
    )]
    if !(difference >= 0.0) {
        truncated -= 1.0;
    }
    Ok(x87::ftol(truncated))
}

/// IDA: 0065E110 PlayerCharacter_GetActorValueFloat(av), vt+0x288: 005EAD00 + the +0x204, +0x324 and damage slots, in that order, at 24 bits. 11 is the encumbrance arm, a stop. Completion: 100% of the other values.
/// # Errors
/// An arm that stops.
pub fn player_get_actor_value_float(
    world: &World,
    player: Entity,
    actor_value: u32,
) -> Result<f32, GameError> {
    if actor_value == 11 {
        return Err(GameError::unclassified(
            0x0065E139,
            "0065E139: encumbrance (004D8FB0)",
        ));
    }
    if actor_value >= 72 {
        return Err(GameError::unclassified(
            0x0065E157,
            "0065E157: an actor value past the 72 slots",
        ));
    }
    let body = world.get::<PlayerCharacter>(player).expect("the player");
    let at = actor_value as usize;
    let maximum = body.max_av_modifiers_204[at];
    let script = body.script_av_modifiers_324[at];
    let damage = match actor_value {
        8 => body.health_444,
        9 => body.magicka_448,
        10 => body.stamina_44c,
        _ => body.damage_av_modifiers_450[at],
    };
    Ok(calculated_actor_value(world, player, actor_value)? + maximum + script + damage)
}

/// IDA: 0065E030 PlayerCharacter_GetActorValue(av), vt+0x284, at 24 bits. 11 goes through 004D8FB0 and stops. 005E02E0(0): the base form when vt+0x190, and a null answers 0. Then +0x204 + 4av, +0x324 + 4av, the damage slot (8, 9, 10: +0x444, +0x448, +0x44C; else +0x450 + 4av), all read through `fld`, `fstp` before 005F1910; 009828C0 of ((`fild` + max) + script) + damage. Completion: 100% of 0..=10 but 9. The modifier arrays have 72 slots, so `av` past them is the same stop.
/// TODO: [ ] 0065E05E 004D8FB0 encumbrance.
/// # Errors
/// An arm that stops.
pub fn player_get_actor_value(
    world: &World,
    player: Entity,
    actor_value: u32,
) -> Result<i32, GameError> {
    if actor_value == 11 {
        return Err(GameError::unclassified(
            0x0065E05E,
            "0065E05E: encumbrance (004D8FB0)",
        ));
    }
    if actor_value >= 72 {
        return Err(GameError::unclassified(
            0x0065E087,
            "0065E087: an actor value past the 72 slots",
        ));
    }
    let refr = world.get::<TesObjectRefr>(player).expect("a reference");
    if refr.base_form_1c.is_none() {
        return Ok(0);
    }
    let body = world.get::<PlayerCharacter>(player).expect("the player");
    let at = actor_value as usize;
    let maximum = x87::fld_fstp_copy(body.max_av_modifiers_204[at].to_bits());
    let script = x87::fld_fstp_copy(body.script_av_modifiers_324[at].to_bits());
    let damage = x87::fld_fstp_copy(
        match actor_value {
            8 => body.health_444,
            9 => body.magicka_448,
            10 => body.stamina_44c,
            _ => body.damage_av_modifiers_450[at],
        }
        .to_bits(),
    );
    let floored = floor_calculated_actor_value(world, player, actor_value)?;
    let sum = (f64::from(floored) + f64::from(maximum)) as f32;
    Ok(x87::ftol(sum + script + damage))
}

/// IDA: 00667600 PlayerCharacter_UpdateCellRegions(cell), vt+0x194 on the player. 006ECC80's +0x40 equal to the cell returns; 0065A330; a null cell clears the +0x6EC list (004526E0) and returns; an exterior cell (004C97F0) clears +0x6E8. Counted boundaries: vt+0x178(0) 0065B5C0, the character controller's Havok world; 0043DF10; 00543200, reached by all but an interior without +0x24 bit 7. +0x71E = 0, +0x730's array emptied (0065DD90) and destroyed. +0x6EC cleared. The cell's region list (004C9B40(1): none for an interior; an exterior with none gets an empty one, an 0x08 extra). Per region, ending at a null item: form flag 0x20, a +0x20 that's another world than 004D6670's, or no areas skip it. The first area holding the player's x and y (004A7330) runs: 4A4460(7)'s RDSD entries each prepended to +0x6EC. With form flag 0x40, the border is seen; all of the cell's four corners, (x, y), (x, y + 4096), (x + 4096, y), (x + 4096, y + 4096), in that area sets +0x71E. A null +0x6E8 takes the region; otherwise with type 3 data on this one, none on the old takes it; flags on this and none on the old take it; equal flags with a higher +0x06 take it. Then, past the areas, a region with form flag 0x40 joins +0x730, inside or not. 00B14F58 bBorderRegionsEnabled, the world's +0x5C bit 3, TES +0x51 and +0x52 clear, with no border seen: +0x72C unset, the length from +0x720 above 2048.0 sets +0x71F. 005A64B0, the HUD's region name, and 0065E860 are counted boundaries. Completion: 100% of the Bethesda state, with the boundaries' stores absent: +0x318 and +0x2A0 under 0065B5C0, the sky's under 00543200, +0x744 and the map markers under 0065E860.
/// TODO: [ ] 0065B5C0, 0043DF10, 00543200, 005A64B0, 0065E860; 00667A46 a set +0x72C, a stop.
/// # Errors
/// A stop in 0065A330, a null g_tes, or a block before 0040EE7A published 00B14F58.
pub fn player_update_cell_regions(
    world: &mut World,
    player: Entity,
    cell: Option<Entity>,
) -> Result<(), GameError> {
    let parent = world
        .get::<TesObjectRefr>(player)
        .expect("a reference")
        .parent_cell_40;
    if parent == cell {
        return Ok(());
    }
    actor_or_reference_set_parent_cell(world, player, cell)?;
    let Some(cell) = cell else {
        player_mut(world, player).region_sound_entries_6ec = PointerList::default();
        return Ok(());
    };
    let interior = cell_is_interior(world, cell);
    if !interior {
        player_mut(world, player).region_6e8 = None;
    }
    let boundaries = |world: &mut World, address: u32| {
        world
            .get_resource_or_init::<TesGridBoundaries>()
            .count(address);
    };
    boundaries(world, 0x0065_B5C0);
    boundaries(world, 0x0043_DF10);
    let (flags_24, cell_grid) = {
        let data = world.get::<TesObjectCell>(cell).expect("a cell");
        (
            data.flags_24,
            (cell_grid_x(world, cell), cell_grid_y(world, cell)),
        )
    };
    if !interior || flags_24 & 0x80 != 0 {
        boundaries(world, 0x0054_3200);
    }
    let cell_world = if interior {
        None
    } else {
        world.get::<TesObjectCell>(cell).expect("a cell").world_50
    };
    {
        let data = player_mut(world, player);
        data.cell_fully_inside_border_region_71e = 0;
        data.border_region_candidates_730 = None;
        data.region_sound_entries_6ec = PointerList::default();
    }
    let mut border_seen = false;
    if !interior {
        // 6676F5..6677A4: `fild` the shifted grid, stored float; plus the qword 4096.0, one rounding.
        let x0 = cell_grid.0.wrapping_shl(12) as f32;
        let y0 = cell_grid.1.wrapping_shl(12) as f32;
        let (x1, y1) = (x0 + 4096.0, y0 + 4096.0);
        let corners = [(x0, y0), (x0, y1), (x1, y0), (x1, y1)];
        let regions: Vec<Entity> = cell_region_list_or_new(
            &mut world
                .get_mut::<TesObjectCell>(cell)
                .expect("a cell")
                .extra_28,
        )
        .regions_04
        .items()
        .collect();
        let at = world
            .get::<TesObjectRefr>(player)
            .expect("a reference")
            .position_2c;
        let space = reference_world(world, player);
        for region in regions {
            if world.get::<FormIdentity>(region).expect("a form").flags_08 & 0x20 != 0 {
                continue;
            }
            let data = world.get::<TesRegion>(region).expect("a region");
            let elsewhere = match data.worldspace_20 {
                FormRef::Resolved(Some(found)) => Some(found) != space,
                FormRef::Raw(raw) => raw != 0,
                FormRef::Resolved(None) => false,
            };
            if elsewhere || data.areas_1c.is_empty() {
                continue;
            }
            let flags = world.get::<FormIdentity>(region).expect("a form").flags_08;
            let inside = data
                .areas_1c
                .iter()
                .find(|area| area.contains_point(at[0], at[1]))
                .cloned();
            if let Some(area) = inside {
                let sounds: Vec<TesRegionSound> = region_data_of_type(world, region, 7)
                    .and_then(|kind| match kind {
                        RegionDataKind::Sound { sounds_0c, .. } => Some(sounds_0c.clone()),
                        _ => None,
                    })
                    .unwrap_or_default();
                for sound in sounds {
                    player_mut(world, player)
                        .region_sound_entries_6ec
                        .prepend_non_null(sound);
                }
                if flags & 0x40 != 0 {
                    border_seen = true;
                    if corners.iter().all(|&(x, y)| area.contains_point(x, y)) {
                        player_mut(world, player).cell_fully_inside_border_region_71e = 1;
                    }
                }
                if region_outranks_current(world, player, region) {
                    player_mut(world, player).region_6e8 = Some(region);
                }
            }
            if flags & 0x40 != 0 {
                player_mut(world, player)
                    .border_region_candidates_730
                    .get_or_insert_default()
                    .push(region);
            }
        }
    }
    // 006679FE..00667AA5.
    let enabled = world
        .get_resource::<TesGridGlobals>()
        .and_then(|globals| globals.border_regions_b14f58)
        .ok_or(GameError::unclassified(
            0x006679FE,
            "006679FE: the region block before 0040EE7A publishes 00B14F58",
        ))?;
    let tes = world
        .resource::<TesGlobals>()
        .tes
        .as_deref()
        .ok_or(GameError::unclassified(
            0x00667A24,
            "00667A24: a null g_tes",
        ))?;
    if enabled != 0
        && let Some(space) = cell_world
        && world.get::<TesWorldSpace>(space).expect("a world").flags_5c & 8 != 0
        && tes.cell_test_active_51 == 0
        && tes.byte_52 == 0
        && !border_seen
    {
        let data = world.get::<PlayerCharacter>(player).expect("the player");
        if data.last_valid_border_location_72c != 0 {
            return Err(GameError::unclassified(
                0x00667A46,
                "00667A46: a set +0x72C",
            ));
        }
        let from = data.last_valid_border_position_720.map(f32::from_bits);
        let at = reference_position(world, player)?;
        let (x, y, z) = (at[0] - from[0], at[1] - from[1], at[2] - from[2]);
        if ((y * y + x * x) + z * z).sqrt() > 2048.0 {
            player_mut(world, player).border_recovery_pending_71f = 1;
        }
    }
    boundaries(world, 0x005A_64B0);
    boundaries(world, 0x0065_E860);
    Ok(())
}

// IDA: N/A the player's component.
fn player_mut(world: &mut World, player: Entity) -> &mut PlayerCharacter {
    world
        .get_mut::<PlayerCharacter>(player)
        .expect("the player")
        .into_inner()
}

// IDA: 00667905..00667945 whether +0x6E8 takes this region.
fn region_outranks_current(world: &World, player: Entity, region: Entity) -> bool {
    let current = world
        .get::<PlayerCharacter>(player)
        .expect("the player")
        .region_6e8;
    let Some(current) = current else {
        return true;
    };
    let rank = |region: Entity| {
        world
            .get::<TesRegion>(region)?
            .data_18
            .items
            .iter()
            .filter_map(|&data| world.get::<TesRegionData>(data))
            .find(|data| data.kind.type_number() == 3)
            .map(|data| (data.flags_04, data.priority_06))
    };
    let Some(new) = rank(region) else {
        return false;
    };
    let Some(old) = rank(current) else {
        return true;
    };
    if new.0 != 0 && old.0 == 0 {
        return true;
    }
    new.0 == old.0 && new.1 > old.1
}

// IDA: 005F23B0 Actor_GetSkillMasteryLevel: skills 12..32 by their floored calculated value through 0056A300 (below
// iSkillApprenticeMin 0, JourneymanMin 1, ExpertMin 2, then MasterMin 4 or 3); others 0.
pub(crate) fn actor_skill_mastery_level(
    world: &World,
    actor: Entity,
    skill: u32,
) -> Result<i32, GameError> {
    if !(12..=32).contains(&skill) {
        return Ok(0);
    }
    let value = floor_calculated_actor_value(world, actor, skill)?;
    let at = |address: u32| {
        world
            .resource::<crate::tes::game_settings::GameSettingCollection>()
            .raw_at(address)
            .unwrap_or(0) as i32
    };
    Ok(if value < at(0x00B3_A4F8) {
        0
    } else if value < at(0x00B3_A500) {
        1
    } else if value < at(0x00B3_A508) {
        2
    } else if value >= at(0x00B3_A510) {
        4
    } else {
        3
    })
}

/// IDA: N/A the value of g_thePlayer (the global at 00B333C4).
#[must_use]
pub fn the_player(world: &World) -> Option<Entity> {
    world
        .get_resource::<PlayerGlobals>()
        .and_then(|globals| globals.the_player_b333c4)
}

// IDA: 0065D550 on g_thePlayer: +0x590 above 0. A null player faults.
pub(crate) fn player_is_sleeping(world: &World) -> Result<bool, GameError> {
    world
        .get_resource::<PlayerGlobals>()
        .and_then(|globals| globals.the_player_b333c4)
        .and_then(|player| world.get::<PlayerCharacter>(player))
        .map(|player| player.pc_sleep_hours_590 > 0)
        .ok_or(GameError::unclassified(
            0x0065D557,
            "0065D557: a null g_thePlayer faults",
        ))
}

// IDA: the format 0045B3CE stores at +0x7C.
const SAVE_FORMAT_VERSION: u8 = 125;

// IDA: N/A the entity's components, under `MobileObject` +0x58. The player's process is high.
fn process_view(world: &World, player: Entity) -> Result<ProcessView<'_>, GameError> {
    let process = world
        .get::<MobileObject>(player)
        .and_then(|mobile| mobile.process_58)
        .ok_or(GameError::unclassified(
            0x00659C34,
            "00659C34: a player with no process",
        ))?;
    Ok(ProcessView {
        base: world.get(process).expect("a process"),
        low: world.get(process).expect("a low process"),
        middle_low: world.get(process).expect("a middle low process"),
        middle_high: world.get(process).expect("a middle high process"),
        high: world.get(process).ok_or(GameError::unclassified(
            0x00659C7B,
            "00659C7B: a player without a high process",
        ))?,
    })
}

// IDA: N/A the base's full name, through `TesFullName` or empty, as 004DA2A0.
fn base_full_name(world: &World, base: Entity) -> Vec<u8> {
    world
        .get::<TesFullName>(base)
        .and_then(|name| name.name_04.data_00.clone())
        .unwrap_or_default()
}

// IDA: base +0x104, resolved; 0x00B37C88 is the setting 00662244 looks a class up by.
fn base_class(world: &World, base: Entity) -> Result<u32, GameError> {
    let npc = world.get::<TesNpc>(base).expect("the player's base");
    let class = match npc.class_104 {
        FormRef::Raw(id) => id,
        FormRef::Resolved(form) => form_id(world, form),
    };
    let default = world
        .resource::<GameSettingCollection>()
        .raw_at(0x00B3_7C88)
        .unwrap_or(0);
    if class != 0 && class == default {
        return Err(GameError::unclassified(
            0x00662276,
            "00662276: 0051C3B0, the default class's record",
        ));
    }
    Ok(class)
}

// IDA: 0060E360 -> 005E7D00 at version 125, with 00659BE0 and 004E2BF0 (2 for the extras' count).
fn actor_save_size(world: &World, process: &ProcessView<'_>) -> u32 {
    (2 + 1) + process_save_size(world, process) + 7 + 2 + 4 + 4 + 5 + 4 + 4 + 14 + 1 + 1
}

// IDA: 006610F0 PlayerCharacter_GetSaveSize at version 125, with flags 4 and empty lists.
fn player_save_size(
    world: &World,
    player: &PlayerCharacter,
    process: &ProcessView<'_>,
    base: Entity,
) -> Result<u32, GameError> {
    let npc = world.get::<TesNpc>(base).expect("the player's base");
    let chargen = if player.chargen_skill_exp_5b0.is_some() {
        21
    } else {
        0
    };
    let bonuses = player
        .attribute_bonuses_5b4
        .as_ref()
        .map_or(0, |list| list.items.len()) as u32;
    let mut size = 876 + actor_save_size(world, process);
    size += 102 + 136 + 1 + 12 + 5 + (4 * chargen + 1) + 10 + 5 + 4;
    size += 44 + 4 + 4 + 4 + 4 + 2 + 4 + 2 + 2;
    // 98 takes in the known topics' count, and 2 is the quest items'.
    size += 2 + 84 + 4 + 8 * bonuses + 98 + 2;
    size += 4 + 4 * saved_effect_settings(world).len() as u32;
    // 00523440: the four matrices at +0x108, and 21.
    size += 21
        + npc
            .facegen_108
            .iter()
            .map(|m| 4 * m.rows * m.cols)
            .sum::<u32>();
    size += base_full_name(world, base).len() as u32 + 1 + 1;
    size += 4 + 4;
    Ok(size)
}

// IDA: 00661540 PlayerCharacter_SaveGame at version 125 with flags 4.
fn write_player(
    out: &mut SaveWriter,
    world: &World,
    player_entity: Entity,
    process: &ProcessView<'_>,
    base: Entity,
) -> Result<(), GameError> {
    let p = world
        .get::<PlayerCharacter>(player_entity)
        .expect("the player");
    let globals = world.resource::<PlayerGlobals>();
    out.floats(&p.max_av_modifiers_204.0);
    out.floats(&p.script_av_modifiers_324.0);
    out.floats(&p.damage_av_modifiers_450.0);
    out.f32(p.health_444);
    out.f32(p.magicka_448);
    out.f32(p.stamina_44c);
    write_actor(out, world, player_entity, process)?;
    out.u8(p.view_flags_588[0]);
    out.u8(p.view_flags_588[1]);
    out.u8(p.view_flags_588[3]);
    out.u8(p.view_flags_588[4]);
    out.u32(p.pc_sleep_hours_590 as u32);
    out.f32(p.athletics_run_use_timer_59c);
    out.f32(p.athletics_swim_use_timer_5a0);
    out.f32(p.sneak_use_timer_5a4);
    out.u8(p.unset_sneak_skill_use_actor_nearby_5a8);
    out.u8(p.dialogue_cooldown_active_600);
    out.f32(p.unset_dialogue_cooldown_elapsed_604);
    out.u32(p.prison_days_remaining_608 as u32);
    out.u32(p.field_60c);
    out.u8(p.jail_guard_pursuit_active_610);
    out.u8(p.is_a_murderer_611);
    out.u32(p.amount_sold_stolen_614 as u32);
    out.u8(p.byte_620);
    out.f32(p.z_rotation_offset_61c);
    out.words(&p.custom_marker_position_62c);
    out.f32(p.camera_eye_height_offset_5d4);
    out.f32(p.bow_draw_timer_640);
    out.u32(p.training_sessions_used_5bc);
    out.u32(p.field_648);
    out.u32(p.field_64c);
    out.u8(p.pc_sleep_hours_set_594);
    out.f32(p.camera_fov_598);
    out.u8(p.vampire_has_fed_6e4);
    out.u8(p.is_in_chargen_6e5);
    out.u8(p.telekinesis_targeting_enabled_6e6);
    out.u32(p.fame_6f4 as u32);
    out.u32(p.infamy_6f8 as u32);
    out.u32(p.infamy_bounty_accumulator_6fc as u32);
    out.u32(p.play_time_milliseconds_714);
    out.u8(p.byte_5c0);
    out.words(&p.misc_stats_658.0[..28]);
    out.words(&p.misc_stats_658.0[28..]);
    out.u8(p.guard_combat_active_738);
    out.u32(p.grab_mode_57c);
    out.f32(p.field_580);
    out.f32(p.grab_distance_584);
    out.u32(p.load_door_follower_move_count_110);
    out.u8(p.jail_sentence_active_200);
    match &p.chargen_skill_exp_5b0 {
        Some(experience) => {
            out.u8(21);
            out.floats(experience);
        }
        None => out.u8(0),
    }
    out.u8(p.view_flags_588[2]);
    out.u8(p.fast_travel_enabled_5a9);
    out.u32(p.initial_timestamp_low_718);
    out.u32(p.cached_sound_form_id_760);
    // 00B3BAEA and 00B3BAFC.
    out.u8(0);
    out.f32(globals.b3bafc);
    out.f32(globals.vanity_wheel_b3bb24);
    out.u32(null_id(
        p.dialogue_package_118,
        "00661B1E: the dialogue package",
    )?);
    out.u32(null_id(p.birthsign_644, "00661B41: the birthsign")?);
    // 00B3BAD0, null at 0040F050 in the run.
    out.u32(0);
    out.u32(form_id(world, p.active_magic_item_624));
    out.u32(null_id(
        p.casting_magic_item_1e8,
        "00661BD0: the casting item",
    )?);
    out.u32(null_id(
        p.casting_target_1ec,
        "00661BF5: the casting target",
    )?);
    out.u32(null_id(
        p.last_ridden_horse_1e0,
        "00661C0C: the last horse",
    )?);
    if p.region_6e8.is_some() {
        return Err(GameError::unclassified(
            0x00661C2F,
            "00661C2F: the player's region",
        ));
    }
    out.u32(0);
    out.u32(null_id(p.book_628, "00661C52: the book")?);
    out.u32(null_id(
        p.chargen_suggested_class_650,
        "00661C75: the suggested class",
    )?);
    out.u32(null_id(p.alchemy_item_6e0, "00661C98: the alchemy item")?);
    out.u32(null_id(
        p.grabbed_reference_578,
        "00661CFA: the grabbed reference",
    )?);
    // 00B3BAD4, null at 0040F050 in the run.
    out.u32(0);
    out.u32(null_id(
        p.last_activated_load_door_570,
        "00661D55: the load door",
    )?);
    out.u32(null_id(
        p.custom_marker_location_638,
        "00661D83: the marker's location",
    )?);
    if p.combat_opponents_5ac != 0 {
        return Err(GameError::unclassified(
            0x00661DC8,
            "00661DC8: a combat opponent",
        ));
    }
    out.u16(0);
    out.u32(0);
    if p.random_teleport_index_by_door_788.count_0c != 0 {
        return Err(GameError::unclassified(
            0x00661EBA,
            "00661EBA: a random teleport entry",
        ));
    }
    out.u16(0);
    // 00B3BB44's list, empty at 0040F050 in the run.
    out.u16(0);
    empty_list(
        p.active_effects_1e4.unwrap_or_default(),
        "0068DD30: a player's active effect",
    )?;
    out.u16(0);
    out.floats(&p.skill_exp_130);
    // 00661F8F counts a node with a block, whatever the block holds.
    let bonuses: &[[u32; 2]] = p
        .attribute_bonuses_5b4
        .as_ref()
        .map_or(&[], |list| &list.items);
    out.u32(bonuses.len() as u32);
    for bonus in bonuses {
        out.words(bonus);
    }
    out.u16(p.skill_advance_counts_5b8);
    out.u8(p.stealth_skill_advance_count_5ba);
    out.words(&p.skill_advances_188);
    out.u32(p.major_skill_advances_184 as u32);
    out.u8(p.can_level_up_1dc);
    out.u32(null_id(p.active_quest_5f4, "00662071: the active quest")?);
    empty_list(p.known_topics_5e4, "006620AE: a known topic")?;
    out.u16(0);
    empty_list(
        p.known_quest_stage_items_5ec,
        "00662150: a quest stage item",
    )?;
    out.u16(0);
    // 00416BA0.
    let settings = saved_effect_settings(world);
    out.u32(settings.len() as u32);
    out.words(&settings);
    write_npc_face(out, world, base)?;
    let name = base_full_name(world, base);
    out.u8((name.len() + 1) as u8);
    out.bytes.extend_from_slice(&name);
    out.u8(0);
    out.u32(base_class(world, base)?);
    out.u32(null_id(
        p.previous_chargen_class_654,
        "0066229C: the previous class",
    )?);
    Ok(())
}

/// IDA: 006632A0 PlayerCharacter_SaveInitialState: +0x70C set warns and returns. 004533F0 the player's change flags, `0x4` in the run; vt+0x4C 006610F0 for the size; 00453500 the buffer of size + 4 into +0x70C, 0046AC70 the flags, +0x7D = 0, vt+0x50 00661540, +0x7D = 1 and +0x14 = 0. The buffer is `PlayerCharacter::initial_state`. Completion: 100% of a first start at version 125 with flags 4.
/// # Errors
/// A flag or a list other than a first start's, or a size that disagrees with the bytes, which the original would overrun.
pub fn player_save_initial_state(world: &mut World, player: Entity) -> Result<(), GameError> {
    {
        let data = world.get::<PlayerCharacter>(player).expect("the player");
        if data.initial_state.is_some() {
            return Err(GameError::unclassified(
                0x006632B2,
                "006632B2: the initial state already exists",
            ));
        }
    }
    let id = world
        .get::<FormIdentity>(player)
        .expect("a form")
        .form_id_0c;
    let globals = world.resource::<TesGlobals>();
    let save = globals
        .save_load_game
        .as_ref()
        .ok_or(GameError::unclassified(
            0x006632C6,
            "006632C6: a null g_saveLoadGame faults",
        ))?;
    if save.byte_7c != SAVE_FORMAT_VERSION {
        return Err(GameError::unclassified(
            0x0045A170,
            "0045A170: a format version other than 125",
        ));
    }
    if save.field_04 != 0 {
        return Err(GameError::unclassified(
            0x004533FF,
            "004533FF: the save load's own map",
        ));
    }
    let flags = save.changes_00.get(id).map_or(0, |record| record.flags_00);
    if flags != 4 {
        return Err(GameError::unclassified(
            0x00661540,
            "00661540: player change flags other than 4",
        ));
    }
    let base = world
        .get::<TesObjectRefr>(player)
        .and_then(|refr| refr.base_form_1c)
        .ok_or(GameError::unclassified(
            0x006613F3,
            "006613F3: a null base form faults",
        ))?;
    let bytes = {
        let process = process_view(world, player)?;
        let data = world.get::<PlayerCharacter>(player).expect("the player");
        let size = player_save_size(world, data, &process, base)? & 0xFFFF;
        let mut out = SaveWriter::default();
        out.u32(flags);
        write_player(&mut out, world, player, &process, base)?;
        if out.bytes.len() as u32 != size + 4 {
            return Err(GameError::unclassified(
                0x006632E9,
                "006632E9: the save's size and bytes disagree",
            ));
        }
        out.bytes
    };
    {
        let mut data = world
            .get_mut::<PlayerCharacter>(player)
            .expect("the player");
        data.initial_state = Some(bytes);
        data.initial_state_buffer_70c = 1;
    }
    let mut globals = world.resource_mut::<TesGlobals>();
    let save = globals.save_load_game.as_mut().expect("read above");
    save.byte_7d = 1;
    save.fields_14[0] = 0;
    Ok(())
}

/// IDA: 0065D270 PlayerCharacter_GetAVModifier(0, av): +0x204 + 4av. Categories 1 and 2 have no caller here. Completion: 100% of category 0.
#[must_use]
pub(crate) fn player_max_modifier(world: &World, player: Entity, actor_value: u32) -> f32 {
    world
        .get::<PlayerCharacter>(player)
        .expect("g_thePlayer is a PlayerCharacter")
        .max_av_modifiers_204[actor_value as usize]
}

/// IDA: 0065DFA0 PlayerCharacter_GetWeightAndLimit: the limit 005E0D20 (fActorStrengthEncumbranceMult 00B37570 times
/// Strength through vt+0x288, at least 0); the weight 004D8FB0; the modifiers of 11 ((+0x350 + +0x230) + +0x47C): below 0
/// the limit less them, else the weight plus them. Completion: 100%.
/// # Errors
/// As the weight and the actor value.
pub fn player_weight_and_limit(world: &mut World, player: Entity) -> Result<(f32, f32), GameError> {
    let strength = player_get_actor_value_float(world, player, 0)?;
    let scaled = float_at_or_zero(world, 0x00B3_7570) * strength;
    let limit = if scaled >= 0.0 { scaled } else { 0.0 };
    let weight = crate::tes::forms_references::reference_encumbrance(world, player)?;
    let body = world.get::<PlayerCharacter>(player).expect("the player");
    let modifiers = (body.script_av_modifiers_324[11] + body.max_av_modifiers_204[11])
        + body.damage_av_modifiers_450[11];
    Ok(if modifiers < 0.0 {
        (weight, limit - modifiers)
    } else {
        (modifiers + weight, limit)
    })
}

const FORM_TYPE_ARMOR: u8 = 0x14;

/// IDA: 0065DAF0 PlayerCharacter's MagicCaster vt+0x2C (0, 0.0), the spell effectiveness: with god mode (00B3BB06, set
/// only by the console's 0065D810) 005E5630; the fatigue factor 1.0, or with a nonzero calculated fatigue other than the
/// current (vt+0x284(10)) 00548B30 of current / calculated (fB37DC8 * x + fB37DC0); then 00548DC0 of Light Armor (27),
/// its coverage (005E5C80(0)), Heavy Armor (18), its coverage (005E5C80(1)): 1.0 when the coverages sum to 0 or less.
/// Completion: 100% of no god mode and no armor worn.
/// TODO: [ ] 00548DC0's armor arm.
/// # Errors
/// Armor in the player's inventory, or an actor value that stops.
pub fn player_spell_effectiveness(world: &World, player: Entity) -> Result<f32, GameError> {
    if inventory_forms(world, player)
        .into_iter()
        .any(|form| form_type(world, form) == FORM_TYPE_ARMOR)
    {
        return Err(GameError::unclassified(
            0x005E5C80,
            "005E5C80: armor in the player's inventory (00548DC0's armor arm) is carried",
        ));
    }
    let current = player_get_actor_value(world, player, 10)?;
    let calculated = floor_calculated_actor_value(world, player, 10)?;
    let mut fatigue = 1.0_f32;
    if calculated != 0 && calculated != current {
        let ratio = current as f32 / calculated as f32;
        fatigue =
            float_at_or_zero(world, 0x00B3_7DC8) * ratio + float_at_or_zero(world, 0x00B3_7DC0);
    }
    Ok(1.0 * fatigue)
}

/// IDA: 0065FCA0 PlayerCharacter_GetSkillProgressFraction: for codes 12..=32, +0x130's experience over +0x7A4's required
/// (both floats, no zero guard), else 0. Completion: 100%.
pub(crate) fn skill_progress(cx: &InterfaceCx, code: u32) -> Result<f32, GameError> {
    if !(12..=32).contains(&code) {
        return Ok(0.0);
    }
    let player = player(cx)?;
    let state = cx
        .world
        .get::<PlayerCharacter>(player)
        .expect("a PlayerCharacter");
    let index = (code - 12) as usize;
    Ok(state.skill_exp_130[index] / f32::from_bits(state.required_skill_exp_7a4[index]))
}

/// IDA: 0065E900 (0040DC70 in menu mode, for the player): the third-person animation data (004D8370) and +0x5CC both set,
/// the process's vt+0x138 (+0xF4, a bow) and vt+0x304 (+0x115) both set: `00470750(3)` == 2 runs `004706D0(3)` twice (a
/// stop). Completion: 100% but that arm.
/// # Errors
/// The bow arm.
pub fn player_menu_bow_check(world: &mut World) -> Result<(), GameError> {
    let Some(player) = the_player(world) else {
        return Err(GameError::unclassified(
            0x0065E900,
            "0065E900: a null g_thePlayer",
        ));
    };
    let Some(process) = world
        .get::<crate::tes::actor::MobileObject>(player)
        .and_then(|mobile| mobile.process_58)
    else {
        return Ok(());
    };
    let Some(middle) = world.get::<crate::tes::actor_process::MiddleHighProcess>(process) else {
        return Ok(());
    };
    let first_person = world
        .get::<PlayerCharacter>(player)
        .and_then(|data| data.first_person_anim_data_5cc);
    if middle.anim_data_17c.is_some()
        && first_person.is_some()
        && middle.bow_f4 != 0
        && middle.weapon_drawn_115 != 0
    {
        return Err(GameError::unclassified(
            0x0065E945,
            "0065E945: 0065E900's bow arm",
        ));
    }
    Ok(())
}

// IDA: 00669690 PlayerCharacter_AddKnownTopic(topic, sort = 1, notify = 0), through 00503B60: a topic already on +0x5E4 answers false. 0052F6D0 is a bubble sort by `_mbsicmp` over +0x1C, the name, null reading as empty, so a stable sort. Completion: 100%.
pub(crate) fn add_topic(world: &mut World, id: u32) -> Result<(), GameError> {
    let Some(topic) = lookup(world, id) else {
        return Ok(());
    };
    let player = the_player(world).ok_or(GameError::unclassified(
        0x00503B60,
        "00503B60: a null g_thePlayer",
    ))?;
    if world
        .get::<PlayerCharacter>(player)
        .expect("the player")
        .known_topics
        .contains(&topic)
    {
        return Ok(());
    }
    let mut topics = std::mem::take(
        &mut world
            .get_mut::<PlayerCharacter>(player)
            .expect("the player")
            .known_topics,
    );
    topics.insert(0, topic);
    let name = |world: &World, form: Entity| -> Vec<u8> {
        world
            .get::<TesFullName>(form)
            .and_then(|full| full.name_04.data_00.clone())
            .unwrap_or_default()
            .to_ascii_lowercase()
    };
    let mut keyed: Vec<(Vec<u8>, Entity)> = topics
        .iter()
        .map(|&form| (name(world, form), form))
        .collect();
    keyed.sort_by(|a, b| a.0.cmp(&b.0));
    world
        .get_mut::<PlayerCharacter>(player)
        .expect("the player")
        .known_topics = keyed.into_iter().map(|(_, form)| form).collect();
    Ok(())
}

// IDA: 00665260 with another reference and no active quest: nothing. The first MoveTo is the first reference it could matter for.
pub(crate) fn stop_on_active_quest(world: &World) -> Result<(), GameError> {
    let player = the_player(world).ok_or(GameError::unclassified(
        0x00665260,
        "00665260: a null g_thePlayer",
    ))?;
    if world
        .get::<PlayerCharacter>(player)
        .expect("the player")
        .active_quest_5f4
        != 0
    {
        return Err(GameError::unclassified(
            0x00665260,
            "00665260: an active quest's targets",
        ));
    }
    Ok(())
}

// IDA: 0065D620 PlayerCharacter_SetInCharGen: off frees +0x5B0. +0x6E5. Completion: 100%.
pub(crate) fn set_in_chargen(world: &mut World, on: bool) {
    let Some(player) = the_player(world) else {
        return;
    };
    let mut data = world
        .get_mut::<PlayerCharacter>(player)
        .expect("the player");
    if !on {
        data.chargen_skill_exp_5b0 = None;
    }
    data.is_in_chargen_6e5 = u8::from(on);
}

/// IDA: 005E7230 PlayerCharacter_RefreshEquippedItemVisuals(null, 0): no process, or DataHandler +0xCD4, returns. With the process's light (+0xE8), shield (+0xF0) and weapon (+0xE4) all null, every arm reaches 005E7A06: the shield and torch lookups (00486790) are freed, and with no armor change vt+0x... isn't called. The menu-mode and sleep reads change nothing. Completion: 100% with nothing equipped; an equipped item stops.
/// TODO: [ ] 005E7230's arms with a weapon, a shield or a light.
pub(crate) fn refresh_equipped_visuals(world: &mut World, actor: Entity) -> Result<(), GameError> {
    let Some(process) = world.get::<MobileObject>(actor).and_then(|m| m.process_58) else {
        return Ok(());
    };
    let Some(middle) = world.get::<crate::tes::actor_process::MiddleHighProcess>(process) else {
        return Err(GameError::unclassified(
            0x005E7262,
            "005E7262: 005E7230 on a Low process",
        ));
    };
    if middle.weapon_e4.is_some() || middle.light_e8.is_some() || middle.shield_f0.is_some() {
        return Err(GameError::unclassified(
            0x005E72C4,
            "005E72C4: 005E7230 with an equipped weapon, shield or light",
        ));
    }
    Ok(())
}

/// IDA: 005E7230 PlayerCharacter_RefreshEquippedItemVisuals(changed, 0) for a WEAP or an ARMO covering slot 13, after
/// 005F3140's equip: no process, or DataHandler +0xCD4, returns. The process's light (+0xE8), shield (+0xF0) and weapon (+0xE4)
/// at vt+0xF0/0xF8/0xFC(0); the menu test is vt+0x304 (+0x115), vt+0x2FC (+0x114), then InterfaceManager_IsMenuMode; a weapon of
/// type 1, 3, 4 or 5 takes both hands. A WEAP: with both hands, a worn shield unequips (004853B0, 004DC8F0) and a worn light
/// with menu mode too, stops; otherwise with menu mode or no light: no weapon, or a light outside menu mode, ends; an unworn
/// shield of a living actor is shown (004E1B40), a stop; no light ends; the rest are stops. An ARMO in slot 13: the light arms
/// are stops; then a one-hand weapon, or no shield, ends; otherwise the shield unequips (004853B0, 004DC8F0(0)), a stop. The
/// shield and torch lookups (00486790) are only freed, and with no unequip the armor rating isn't invalidated. Completion: 100%
/// of the arms with no light; the unequips and the shield's 3D are stops.
pub(crate) fn refresh_equipped_visuals_for(
    world: &mut World,
    actor: Entity,
    changed: Entity,
) -> Result<(), GameError> {
    let Some(process) = world.get::<MobileObject>(actor).and_then(|m| m.process_58) else {
        return Ok(());
    };
    if world
        .get_resource::<crate::tes::data_handler::DataHandler>()
        .is_some_and(|data| data.bytes_cd0[4] != 0)
    {
        return Ok(());
    }
    let Some(middle) = world.get::<crate::tes::actor_process::MiddleHighProcess>(process) else {
        return Err(GameError::unclassified(
            0x005E7262,
            "005E7262: 005E7230 on a Low process",
        ));
    };
    let (light, shield, weapon) = (middle.light_e8, middle.shield_f0, middle.weapon_e4);
    let menu = middle.weapon_drawn_115 != 0
        || middle.weapon_draw_requested_114 != 0
        || crate::tes::interface_manager::is_menu_mode(world);
    let both_hands = weapon.is_some_and(|weapon| {
        matches!(
            crate::tes::forms_items::weapon_type_of(world, weapon.form_08),
            1 | 3 | 4 | 5
        )
    });
    let dead = crate::tes::actor::actor_is_dead(world, actor, false);
    match crate::tes::forms_init::form_type(world, changed) {
        0x21 => {
            if both_hands {
                if shield.is_some_and(|shield| shield.worn) {
                    return Err(GameError::unclassified(
                        0x005E7AA7,
                        "005E7AA7: a two-hand weapon unequips the worn shield",
                    ));
                }
                if light.is_some_and(|light| light.worn) {
                    return Err(GameError::unclassified(
                        0x005E7ABF,
                        "005E7ABF: a two-hand weapon over a worn light",
                    ));
                }
                return Ok(());
            }
            if menu || light.is_none() {
                if weapon.is_none() || (!menu && light.is_some()) {
                    return Ok(());
                }
                if shield.is_some_and(|shield| !shield.worn) && !dead {
                    // 005E7B1A: the changes' worn stack in slot 13 (00486790) goes back onto the shield's entry; 004E1B40 with
                    // no node returns; the armor rating is invalidated.
                    if equipped_in_slot(world, actor, 13)?.is_some() {
                        let mut middle = world
                            .get_mut::<crate::tes::actor_process::MiddleHighProcess>(process)
                            .expect("checked above");
                        if let Some(entry) = middle.shield_f0.as_mut() {
                            entry.worn = true;
                        }
                    }
                    no_node_or_stop(world, actor, "004E1B40: the shield's 3D")?;
                    invalidate_armor_rating(world, actor);
                }
                if light.is_none() {
                    return Ok(());
                }
                return Err(GameError::unclassified(
                    0x005E7B40,
                    "005E7B40: a weapon's light arms",
                ));
            }
            Err(GameError::unclassified(
                0x005E7B83,
                "005E7B83: a weapon with a light outside menu mode",
            ))
        }
        _ => {
            if light.is_some() {
                return Err(GameError::unclassified(
                    0x005E7A33,
                    "005E7A33: a shield's light arms",
                ));
            }
            if (weapon.is_some() && !both_hands) || shield.is_none() {
                return Ok(());
            }
            // 005E7A50: 004853B0(shield, 0, 0, 0) drops the worn stack from the process's entry (the changes keep it worn);
            // 004DC8F0(0) with no node returns; the armor rating is invalidated.
            {
                let mut middle = world
                    .get_mut::<crate::tes::actor_process::MiddleHighProcess>(process)
                    .expect("checked above");
                if let Some(entry) = middle.shield_f0.as_mut() {
                    entry.worn = false;
                }
            }
            no_node_or_stop(world, actor, "004DC8F0: the shield's 3D detached")?;
            invalidate_armor_rating(world, actor);
            Ok(())
        }
    }
}

/// IDA: `00667420`(player, ref) PlayerCharacter_CheckReferenceOverlap: a reference that isn't an actor, with the player's
/// node (Character_GetNiNode, +0x3C): the distance from that node's +0x54 local translation to the reference node's world
/// bound centre, float steps, below the bound's radius asks `006670F0`'s pick tests, which with a hit cull the node, run
/// `0088CF20` and append the reference to 00B3BB44 (a stop: those tests aren't ported). Completion: 100% outside the bound.
/// # Errors
/// The player inside the reference's bound.
pub(crate) fn player_check_reference_overlap(
    world: &World,
    reference: Entity,
    node: Entity,
) -> Result<(), GameError> {
    if world.get::<crate::tes::actor::Actor>(reference).is_some() {
        return Ok(());
    }
    let Some(player_node) = the_player(world)
        .and_then(|player| world.get::<Reference3D>(player))
        .map(|loaded| loaded.0)
    else {
        return Ok(());
    };
    let bound = world
        .get::<crate::gamebryo::scene_node::NiAvWorld>(node)
        .map_or([0.0; 4], |state| state.bound);
    let local = world
        .get::<crate::gamebryo::scene_node::SceneNodePayload>(player_node)
        .map(|payload| payload.local_transform.translation)
        .unwrap_or_default();
    let dx = local[0] - bound[0];
    let dy = local[1] - bound[1];
    let dz = local[2] - bound[2];
    let distance = (dy * dy + dx * dx + dz * dz).sqrt();
    if distance < bound[3] {
        return Err(GameError::unclassified(
            0x006670F0,
            "006670F0: the player's node inside a finished reference's bound (its pick tests)",
        ));
    }
    Ok(())
}
