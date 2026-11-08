//! Phase 11: the PlayerCharacter block of 00406360 (00406774..004067B9) against the P11-RUN-001 dump: the TESForm DataHandler arm and the 0x800 / 0x14 form IDs, the whole 0x804 player, the LowProcess and HighProcess bytes,
//! the ActorProcessManager lists before and after the swap, the Havok controller's Bethesda-side records and the two collision helpers, the globals, the FOV call and the particle FOV scale.
//!
//! Images are compared byte by byte with pointers by role; a byte the original left unchanged after its FormHeap allocation may be host zero (uninitialized FormHeap bytes, the recorded substitution). The clock, the CRT rand
//! result, the GMST / INI values and the LowProcess +0x88 FormHeap bytes are injected from the run. Then the arms the run did not take, which only the C++ port has exercised. Host tests earn no validation bonus.
//!
//! Each `phase11.rows` line is read by a check, and `every_row_is_read` fails otherwise. The serializers live here: components carry the field offsets in their names, and this file puts them back at those offsets.

use std::cell::RefCell;

use bevy_ecs::prelude::*;
use recomp_core::crt::{MsvcRand, RandomGlobals};
use recomp_core::fixtures;
use recomp_game::bs::scene_graphs::{
    SceneCamera, SceneGraph, SceneGraphSettings, construct_scene_graph,
};
use recomp_game::gamebryo::scene_update::NoSceneControllers;
use recomp_game::tes::actor::{
    Actor, Character, MobileObject, TesObjectRefr, actor_process_level, construct_character,
};
use recomp_game::tes::actor_process::{
    ActorListCursor, ActorListId, ActorListNode, ActorProcessGlobals, ActorProcessList,
    ActorValueEntry, ActorValues, BaseProcess, CrtRandomSource, HighProcess, LowProcess,
    MiddleHighProcess, MiddleLowProcess, ProcessInputs, RandomSource, actor_process_manager_add,
    actor_process_manager_remove, construct_actor_process_manager, construct_actor_values,
    construct_high_process, construct_low_process, copy_process_from, delete_low_process,
    process_level, random_next,
};
use recomp_game::tes::character_controller::{
    BhkCharacterController, BhkSphereShapeRecord, BhkWorldObjectRecord, CameraCollisionHelper,
    CharacterControllerGlobals, CollisionQueryHelper, ControllerEnableCall, ControllerInputs,
    HAVOK_SCALE, actor_character_controller, actor_create_character_controller,
};
use recomp_game::tes::data_handler::{
    DataHandler, DataHandlerGlobals, FormsById, construct_data_handler, construct_tes_form,
    insert_form_indexes, release_form_id, set_form_id_release, update_mod_reference_list,
};
use recomp_game::tes::form::{FormIdentity, ModReferences, OriginalVtable};
use recomp_game::tes::game_settings::GameSettingCollection;
use recomp_game::tes::player::{
    PLAYER_FORM_ID, PlayerCharacter, PlayerGlobals, PlayerInputs, PlayerSettingInputs,
    construct_player_character, particle_update_fov_scale, player_setting_inputs,
};
use recomp_game::tes::settings::{SettingCollections, register_static_settings};
use recomp_game::tes::tes::GameThreadLocals;
use recomp_game::tes::tes_file::{TesFileData, TesFileId};
use recomp_startup::initialize_game_data::run_player_window;

/// IDA: N/A `phase11.rows`, recording which lines a check has read.
struct Rows {
    lines: Vec<String>,
    read: RefCell<Vec<bool>>,
}

impl Drop for Rows {
    fn drop(&mut self) {
        fixtures::ledger_read("phase11", &self.read.borrow());
    }
}

impl Rows {
    fn load() -> Self {
        let lines = fixtures::rows("phase11");
        let read = RefCell::new(vec![false; lines.len()]);
        Self { lines, read }
    }

    fn expected(&self, prefix: &str) -> &str {
        let index = self
            .lines
            .iter()
            .position(|line| line.starts_with(prefix))
            .unwrap_or_else(|| panic!("missing expected line {prefix}"));
        self.read.borrow_mut()[index] = true;
        &self.lines[index]
    }

    fn compare(&self, host: &str, prefix: &str) {
        assert_eq!(host, self.expected(prefix), "mismatch for {prefix}");
    }

    fn unread(&self) -> Vec<&str> {
        self.lines
            .iter()
            .zip(self.read.borrow().iter())
            .filter(|(_, read)| !**read)
            .map(|(line, _)| &line[..line.len().min(40)])
            .collect()
    }
}

// IDA: N/A little-endian byte text of a dword ("0000803f").
fn le(value: u32) -> String {
    value
        .to_le_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// IDA: N/A an object image: one cell per byte ("xx"), a pointer role "<name>" in its first byte (the next three cells empty), "??" for bytes the host does not model.
struct Image {
    cells: Vec<String>,
}

impl Image {
    fn new(size: usize, modeled: bool) -> Self {
        Self {
            cells: vec![if modeled { "00" } else { "??" }.to_owned(); size],
        }
    }

    fn u8(&mut self, at: usize, value: u8) {
        self.cells[at] = format!("{value:02x}");
    }

    fn u16(&mut self, at: usize, value: u16) {
        self.u8(at, value as u8);
        self.u8(at + 1, (value >> 8) as u8);
    }

    fn u32(&mut self, at: usize, value: u32) {
        for (index, byte) in value.to_le_bytes().into_iter().enumerate() {
            self.u8(at + index, byte);
        }
    }

    fn f32(&mut self, at: usize, value: f32) {
        self.u32(at, value.to_bits());
    }

    fn ptr(&mut self, at: usize, present: bool, role: &str) {
        if !present {
            self.u32(at, 0);
            return;
        }
        self.cells[at] = format!("<{role}>");
        self.cells[at + 1].clear();
        self.cells[at + 2].clear();
        self.cells[at + 3].clear();
    }

    fn floats(&mut self, at: usize, values: &[f32]) {
        for (index, value) in values.iter().enumerate() {
            self.f32(at + 4 * index, *value);
        }
    }

    fn dwords(&mut self, at: usize, values: &[u32]) {
        for (index, value) in values.iter().enumerate() {
            self.u32(at + 4 * index, *value);
        }
    }

    fn text(&self) -> String {
        self.cells.concat()
    }
}

fn parse_cells(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut cells = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'<' {
            let end = at + text[at..].find('>').unwrap();
            cells.push(text[at..=end].to_owned());
            cells.extend([String::new(), String::new(), String::new()]);
            at = end + 1;
        } else {
            cells.push(text[at..at + 2].to_owned());
            at += 2;
        }
    }
    cells
}

/// IDA: N/A compares a host image with "IMG <name> ..." (and "INIT <init> ..." for the unset rule).
fn compare_image(rows: &Rows, host: &Image, name: &str, init: Option<&str>) {
    let prefix = format!("IMG {name} ");
    let run = parse_cells(&rows.expected(&prefix)[prefix.len()..]);
    let initial = init.map_or_else(Vec::new, |init| {
        let prefix = format!("INIT {init} ");
        fixtures::ledger_relaxed(
            "phase11",
            &format!("INIT {init}"),
            1,
            "not a value to compare: the bytes a constructor left as its FormHeap bytes, which excuse a host zero in the IMG row",
        );
        parse_cells(&rows.expected(&prefix)[prefix.len()..])
    });
    assert_eq!(run.len(), host.cells.len(), "{name}: image size");
    let mut failures = String::new();
    let mut count = 0;
    for (at, (h, r)) in host.cells.iter().zip(&run).enumerate() {
        if h == "??" || h == r {
            continue;
        }
        // A byte the constructor left as its FormHeap bytes: host zero.
        if h == "00" && r.len() == 2 && initial.get(at) == Some(r) {
            continue;
        }
        count += 1;
        if count <= 24 {
            failures += &format!(" +{at:X} host={h} run={r}");
        }
    }
    assert_eq!(count, 0, "{name}: {count} byte(s) differ:{failures}");
}

/// IDA: N/A the run's injected inputs ("I ..." line).
#[derive(Clone, Copy)]
struct RunInputs {
    tick: u32,
    time64: u32,
    rand: i32,
    low88: u32,
    settings: PlayerSettingInputs,
}

fn field<'a>(line: &'a str, key: &str) -> &'a str {
    let at = line
        .find(&format!(" {key}="))
        .unwrap_or_else(|| panic!("missing key {key}"));
    let rest = &line[at + key.len() + 2..];
    rest.split(' ').next().unwrap()
}

fn field_hex(line: &str, key: &str) -> u32 {
    u32::from_str_radix(field(line, key), 16).unwrap()
}

fn field_le(line: &str, key: &str) -> u32 {
    u32::from_str_radix(field(line, key), 16)
        .unwrap()
        .swap_bytes()
}

fn run_inputs(rows: &Rows) -> RunInputs {
    let line = rows.expected("I ");
    RunInputs {
        tick: field_hex(line, "tick"),
        time64: field_hex(line, "time64"),
        rand: field_hex(line, "rand") as i32,
        low88: field_hex(line, "low88"),
        settings: PlayerSettingInputs {
            vanity_mode_wheel_default: field_le(line, "vanity"),
            idle_chatter_comment_timer: field_le(line, "chatter"),
            difficulty: field_le(line, "difficulty"),
            default_fov: field_le(line, "fov"),
        },
    }
}

/// IDA: N/A a scripted 0047DF80 CRT: the run's _time64 and rand.
#[derive(Default)]
struct ScriptedRandom {
    time64: i64,
    rand: i32,
    seeds: Vec<u32>,
    time_calls: u32,
}

impl RandomSource for ScriptedRandom {
    fn time64(&mut self) -> i64 {
        self.time_calls += 1;
        self.time64
    }

    fn srand(&mut self, seed: u32) {
        self.seeds.push(seed);
    }

    fn rand(&mut self) -> i32 {
        self.rand
    }
}

/// IDA: N/A the whole Phase 11 environment: the Phase 09 DataHandler (21 skills), the manager static, the controller globals, a World scene graph at fDefaultFOV and the injected inputs.
struct Harness {
    world: World,
    inputs: RunInputs,
    random: ScriptedRandom,
    settings: SceneGraphSettings,
    graph: Entity,
}

impl Harness {
    fn new(inputs: RunInputs) -> Self {
        let mut world = World::new();
        world.insert_resource(GameSettingCollection::default());
        world.init_resource::<PlayerGlobals>();
        world.init_resource::<CharacterControllerGlobals>();
        construct_data_handler(&mut world).unwrap();
        construct_actor_process_manager(&mut world);
        let settings = SceneGraphSettings {
            renderer_width: 1920,
            renderer_height: 1080,
            default_fov: f32::from_bits(inputs.settings.default_fov),
            ..SceneGraphSettings::default()
        };
        let graph = construct_scene_graph(
            &mut world,
            b"World",
            false,
            None,
            &settings,
            &mut NoSceneControllers,
        )
        .unwrap();
        let random = ScriptedRandom {
            time64: i64::from(inputs.time64),
            rand: inputs.rand,
            ..ScriptedRandom::default()
        };
        Self {
            world,
            inputs,
            random,
            settings,
            graph,
        }
    }

    fn process_inputs(&mut self) -> (&mut World, ProcessInputs<'_>) {
        let process = ProcessInputs {
            random: &mut self.random,
            idle_chatter_comment_timer: self.inputs.settings.idle_chatter_comment_timer,
            low_process_unset_88: self.inputs.low88,
            save_load_active: false,
        };
        (&mut self.world, process)
    }

    fn character(&mut self) -> Entity {
        let (world, process) = self.process_inputs();
        construct_character(world, &process).unwrap()
    }

    // IDA: N/A `body` with the `PlayerInputs` of the run.
    fn with_player_inputs<R>(
        &mut self,
        body: impl FnOnce(&mut World, &mut PlayerInputs<'_>) -> R,
    ) -> R {
        let inputs = self.inputs;
        let mut tick = move || inputs.tick;
        let mut time = move || i64::from(inputs.time64);
        let mut runtime = NoSceneControllers;
        let mut player_inputs = PlayerInputs {
            process: ProcessInputs {
                random: &mut self.random,
                idle_chatter_comment_timer: inputs.settings.idle_chatter_comment_timer,
                low_process_unset_88: inputs.low88,
                save_load_active: false,
            },
            controller: ControllerInputs::default(),
            settings: inputs.settings,
            world_scene_graph: Some(self.graph),
            scene_settings: &self.settings,
            scene_runtime: &mut runtime,
            get_tick_count: &mut tick,
            time64: &mut time,
        };
        body(&mut self.world, &mut player_inputs)
    }

    fn manager(&self) -> &ActorProcessGlobals {
        self.world.resource::<ActorProcessGlobals>()
    }

    fn scene_camera(&self) -> SceneCamera {
        let camera = self
            .world
            .get::<SceneGraph>(self.graph)
            .unwrap()
            .camera_dc
            .unwrap();
        *self.world.get::<SceneCamera>(camera).unwrap()
    }
}

// IDA: N/A TESForm +0x00..+0x17.
fn write_form(image: &mut Image, world: &World, form: Entity) {
    let identity = world.get::<FormIdentity>(form).unwrap();
    let mods = world.get::<ModReferences>(form).unwrap();
    image.u32(0x00, world.get::<OriginalVtable>(form).unwrap().0);
    image.u8(0x04, identity.form_type_04);
    image.u32(0x08, identity.flags_08);
    image.u32(0x0C, identity.form_id_0c);
    image.ptr(0x10, !mods.0.is_empty(), "file");
    image.ptr(0x14, mods.0.len() > 1, "node");
}

fn write_actor_values(image: &mut Image, at: usize, values: &ActorValues) {
    image.ptr(at, !values.list_00.is_empty(), "blk");
    image.ptr(at + 0x04, values.list_00.len() > 1, "blk");
    image.ptr(at + 0x08, values.magicka_08.is_some(), "blk");
    image.ptr(at + 0x0C, values.fatigue_0c.is_some(), "blk");
    image.ptr(at + 0x10, values.indexed_10.is_some(), "blk");
}

// IDA: N/A TESObjectREFR .. Character (+0x18..+0x10B).
fn write_character(image: &mut Image, world: &World, form: Entity) {
    write_form(image, world, form);
    let r = world.get::<TesObjectRefr>(form).unwrap();
    let m = world.get::<MobileObject>(form).unwrap();
    let c = world.get::<Actor>(form).unwrap();
    let k = world.get::<Character>(form).unwrap();
    image.u32(0x18, r.child_cell_vtable_18);
    image.ptr(0x1C, r.base_form_1c.is_some(), "base");
    image.floats(0x20, &r.rotation_20);
    image.floats(0x2C, &r.position_2c);
    image.f32(0x38, r.scale_38);
    image.u32(0x3C, r.loaded_3d_3c);
    image.u32(0x40, u32::from(r.parent_cell_40.is_some()));
    image.u32(0x44, r.extra_data_44.original_vtable);
    image.u32(0x48, r.extra_data_44.head_04());
    for (index, byte) in r.extra_data_44.presence_08.iter().enumerate() {
        image.u8(0x4C + index, *byte);
    }
    image.ptr(0x58, m.process_58.is_some(), "H");
    image.u32(0x5C, c.magic_caster_5c.original_vtable);
    image.u32(0x60, c.magic_caster_5c.magic_node_04);
    image.u32(0x64, c.magic_caster_5c.state_08);
    image.u32(0x68, c.magic_target_68.original_vtable);
    image.u8(0x6C, c.magic_target_68.byte_04);
    image.u32(0x70, c.expression_override_70);
    image.f32(0x74, c.expression_override_strength_74);
    image.u8(0x78, c.processing_enabled_78);
    image.u32(0x7C, c.death_killer_7c);
    image.u8(0x80, c.murder_candidate_80);
    image.f32(0x84, c.unset_check_dead_body_timer_84);
    write_actor_values(image, 0x88, &c.av_modifiers_88);
    image.dwords(0x9C, &c.greater_power_list_9c);
    image.dwords(0xA4, &c.disposition_modifiers_a4);
    image.f32(0xAC, c.combat_target_reevaluation_timer_ac);
    image.u32(0xB0, c.dead_state_b0);
    image.dwords(0xB4, &c.reserved_b4);
    image.f32(0xBC, c.last_process_update_time_bc);
    image.u8(0xC0, c.deferred_death_processing_c0);
    image.u32(0xC4, c.render_visibility_flags_c4);
    image.u8(0xC8, c.force_run_c8);
    image.u8(0xC9, c.force_sneak_c9);
    image.u8(0xCA, c.allow_quest_process_reschedule_ca);
    image.u32(0xCC, c.pending_movement_blocked_ref_cc);
    image.u32(0xD0, c.leveled_creature_template_d0);
    image.u32(0xD4, c.horse_or_rider_d4);
    image.u8(0xD8, c.unset_in_water_d8);
    image.f32(0xDC, c.head_track_target_hold_timer_dc);
    image.u8(0xE0, c.head_track_visibility_active_e0);
    image.u32(0xE4, c.pending_dialogue_target_e4);
    image.dwords(0xE8, &c.starting_position_e8);
    image.f32(0xF4, c.starting_z_rotation_f4);
    image.u32(0xF8, u32::from(c.starting_location_f8.is_some()));
    image.u8(0xFC, c.death_effects_processed_fc);
    image.u8(0xFD, c.merchant_respawn_day_processed_fd);
    image.f32(0x100, c.head_track_target_scan_timer_100);
    image.u32(0x104, u32::from(k.skin_info_104.is_some()));
    image.f32(0x108, k.armor_rating_cache_108);
}

// IDA: N/A the 0x804-byte PlayerCharacter.
fn player_image(world: &World, player: Entity) -> Image {
    let mut image = Image::new(0x804, true);
    write_character(&mut image, world, player);
    let p = world.get::<PlayerCharacter>(player).unwrap();
    image.u8(0x10C, p.force_direct_activation_10c);
    image.u32(0x110, p.load_door_follower_move_count_110);
    image.u8(0x114, p.cell_transition_in_progress_114);
    image.u8(0x115, p.follower_relocation_latch_115);
    image.u8(0x116, p.is_in_se_world_116);
    image.u8(0x117, p.unset_active_quest_targets_dirty_117);
    image.u32(0x118, p.dialogue_package_118);
    image.u32(0x11C, p.accepted_haggle_adjustment_11c as u32);
    image.u32(0x120, p.field_120);
    image.u8(0x124, p.byte_124);
    image.u32(0x128, p.field_128);
    image.u8(0x12C, p.is_moving_into_new_space_12c);
    image.floats(0x130, &p.skill_exp_130[..21]);
    image.u32(0x184, p.major_skill_advances_184 as u32);
    image.dwords(0x188, &p.skill_advances_188[..21]);
    image.u8(0x1DC, p.can_level_up_1dc);
    image.u32(0x1E0, p.last_ridden_horse_1e0);
    image.u32(0x1E8, p.casting_magic_item_1e8);
    image.u32(0x1EC, p.casting_target_1ec);
    image.u32(0x1FC, p.pending_enchanted_equipment_1fc);
    image.u8(0x200, p.jail_sentence_active_200);
    image.floats(0x204, &p.max_av_modifiers_204[..72]);
    image.floats(0x324, &p.script_av_modifiers_324[..72]);
    image.f32(0x444, p.health_444);
    image.f32(0x448, p.magicka_448);
    image.f32(0x44C, p.stamina_44c);
    image.floats(0x450, &p.damage_av_modifiers_450[..72]);
    image.u32(0x570, p.last_activated_load_door_570);
    image.u32(0x574, p.grab_spring_action_574);
    image.u32(0x578, p.grabbed_reference_578);
    image.u32(0x57C, p.grab_mode_57c);
    image.f32(0x580, p.field_580);
    image.f32(0x584, p.grab_distance_584);
    image.u32(0x590, p.pc_sleep_hours_590 as u32);
    image.u8(0x594, p.pc_sleep_hours_set_594);
    image.f32(0x598, p.camera_fov_598);
    image.f32(0x59C, p.athletics_run_use_timer_59c);
    image.f32(0x5A0, p.athletics_swim_use_timer_5a0);
    image.f32(0x5A4, p.sneak_use_timer_5a4);
    image.u8(0x5A8, p.unset_sneak_skill_use_actor_nearby_5a8);
    image.u8(0x5A9, p.fast_travel_enabled_5a9);
    image.u32(0x5AC, p.combat_opponents_5ac);
    image.u16(0x5B8, p.skill_advance_counts_5b8);
    image.u8(0x5BA, p.stealth_skill_advance_count_5ba);
    image.u32(0x5BC, p.training_sessions_used_5bc);
    image.u8(0x5C0, p.byte_5c0);
    image.u32(0x5C4, p.pending_activation_ref_5c4);
    image.u32(0x5C8, u32::from(p.first_person_skin_info_5c8.is_some()));
    image.u32(0x5CC, u32::from(p.first_person_anim_data_5cc.is_some()));
    image.u32(0x5D0, u32::from(p.first_person_node_5d0.is_some()));
    image.f32(0x5D4, p.camera_eye_height_offset_5d4);
    image.u32(0x5D8, p.first_person_biped_root_5d8);
    image.u32(0x5DC, p.first_person_biped_anim_data_5dc);
    image.u32(0x5E0, p.active_magic_shader_hit_effect_5e0);
    image.dwords(0x5E4, &p.known_topics_5e4[..2]);
    image.dwords(0x5EC, &p.known_quest_stage_items_5ec[..2]);
    image.u32(0x5F4, p.active_quest_5f4);
    image.dwords(0x5F8, &p.active_quest_targets_5f8[..2]);
    image.u8(0x600, p.dialogue_cooldown_active_600);
    image.f32(0x604, p.unset_dialogue_cooldown_elapsed_604);
    image.u32(0x608, p.prison_days_remaining_608 as u32);
    image.u32(0x60C, p.field_60c);
    image.u8(0x610, p.jail_guard_pursuit_active_610);
    image.u8(0x611, p.is_a_murderer_611);
    image.u32(0x614, p.amount_sold_stolen_614 as u32);
    image.f32(0x618, p.actor_distance_list_sort_timer_618);
    image.f32(0x61C, p.z_rotation_offset_61c);
    image.u8(0x620, p.byte_620);
    image.u32(0x624, u32::from(p.active_magic_item_624.is_some()));
    image.u32(0x628, p.book_628);
    image.dwords(0x62C, &p.custom_marker_position_62c[..3]);
    image.u32(0x638, p.custom_marker_location_638);
    image.u32(0x63C, p.custom_marker_route_reference_63c);
    image.f32(0x640, p.bow_draw_timer_640);
    image.u32(0x644, p.birthsign_644);
    image.u32(0x648, p.field_648);
    image.u32(0x64C, p.field_64c);
    image.u32(0x650, p.chargen_suggested_class_650);
    image.u32(0x654, p.previous_chargen_class_654);
    image.dwords(0x658, &p.misc_stats_658[..34]);
    image.u32(0x6E0, p.alchemy_item_6e0);
    image.u8(0x6E4, p.vampire_has_fed_6e4);
    image.u8(0x6E5, p.is_in_chargen_6e5);
    image.u8(0x6E6, p.telekinesis_targeting_enabled_6e6);
    assert_eq!(p.region_6e8, None);
    image.u32(0x6E8, 0);
    assert!(p.region_sound_entries_6ec.is_empty());
    image.dwords(0x6EC, &[0, 0]);
    image.u32(0x6F4, p.fame_6f4 as u32);
    image.u32(0x6F8, p.infamy_6f8 as u32);
    image.u32(0x6FC, p.infamy_bounty_accumulator_6fc as u32);
    image.f32(0x700, p.se_world_crime_gold_700);
    image.u32(0x70C, p.initial_state_buffer_70c);
    image.u32(0x710, p.play_time_last_tick_710);
    image.u32(0x714, p.play_time_milliseconds_714);
    image.u32(0x718, p.initial_timestamp_low_718);
    image.u8(0x71C, p.sit_sleep_entry_pending_71c);
    image.u8(0x71D, p.sit_sleep_exit_pending_71d);
    image.u8(0x71E, p.cell_fully_inside_border_region_71e);
    image.u8(0x71F, p.border_recovery_pending_71f);
    image.dwords(0x720, &p.last_valid_border_position_720[..3]);
    image.u32(0x72C, p.last_valid_border_location_72c);
    assert_eq!(p.border_region_candidates_730, None);
    image.u32(0x730, 0);
    image.f32(0x734, p.game_difficulty_level_734);
    image.u8(0x738, p.guard_combat_active_738);
    image.dwords(0x73C, &p.map_marker_list_73c[..2]);
    image.u32(0x744, p.map_marker_list_worldspace_744);
    image.u8(0x748, p.fly_camera_enabled_748);
    image.f32(0x74C, p.fly_camera_yaw_74c);
    image.f32(0x750, p.fly_camera_pitch_750);
    image.dwords(0x754, &p.fly_camera_position_754[..3]);
    image.u32(0x760, p.cached_sound_form_id_760);
    image.u32(0x764, p.cached_sound_handle_764);
    image.dwords(0x768, &p.magic_failure_sound_handles_768[..6]);
    image.dwords(0x780, &p.pending_dropped_references_780[..2]);
    image.u32(0x798, p.first_person_light_798);
    image.u32(0x79C, p.third_person_light_79c);
    image.u32(0x7A0, p.combat_opponent_count_7a0 as u32);
    image.dwords(0x7A4, &p.required_skill_exp_7a4[..21]);
    image.u8(0x7F8, p.new_game_intro_pending_7f8);
    image.u8(0x7F9, p.no_charge_message_shown_7f9);
    image.f32(0x7FC, p.drop_angle_accumulator_7fc);
    image.f32(0x800, p.last_drop_placement_angle_800);
    image.ptr(0x1E4, p.active_effects_1e4.is_some(), "blk");
    image.ptr(0x1F0, p.camera_collision_helper_1f0.is_some(), "camhelper");
    image.ptr(0x1F4, p.collision_query_helper_1f4.is_some(), "query");
    image.ptr(0x1F8, p.pending_potion_list_1f8.is_some(), "blk");
    for (index, byte) in p.view_flags_588.iter().enumerate() {
        image.u8(0x588 + index, *byte);
    }
    image.ptr(0x5B0, p.chargen_skill_exp_5b0.is_some(), "blk");
    image.ptr(0x5B4, p.attribute_bonuses_5b4.is_some(), "blk");
    image.dwords(0x704, &p.jail_marker_candidates_704);
    let map = &p.random_teleport_index_by_door_788;
    image.u32(0x788, map.original_vtable);
    image.u32(0x78C, map.hash_size_04);
    image.ptr(0x790, !map.buckets_08.is_empty(), "blk");
    image.u32(0x794, map.count_0c);
    image
}

// IDA: N/A LowProcess +0x00..+0x8F.
fn write_low(image: &mut Image, world: &World, process: Entity) {
    let b = world.get::<BaseProcess>(process).unwrap();
    let l = world.get::<LowProcess>(process).unwrap();
    image.u32(0x00, world.get::<OriginalVtable>(process).unwrap().0);
    image.u32(0x04, b.edit_pack_procedure_type_04);
    image.u32(
        0x08,
        if b.editor_package_08.is_some() {
            0xFFFF_FFFF
        } else {
            0
        },
    );
    image.f32(0x0C, l.field_0c);
    image.f32(0x10, l.field_10);
    image.f32(0x14, l.cur_hour_14);
    image.u32(0x18, l.cur_packed_date_18 as u32);
    image.u8(0x1C, l.byte_1c);
    image.u8(0x1D, l.byte_1d);
    image.u8(0x1E, l.byte_1e);
    image.u8(0x1F, l.byte_1f);
    image.u8(0x20, l.byte_20);
    image.u32(0x24, l.used_item_24);
    image.f32(0x28, l.float_28);
    image.u32(0x2C, if l.field_2c.is_some() { 0xFFFF_FFFF } else { 0 });
    image.u32(0x30, if l.field_30.is_some() { 0xFFFF_FFFF } else { 0 });
    image.u32(0x34, l.pathing_34);
    image.u32(0x38, l.field_38);
    image.dwords(0x3C, &l.list_3c);
    image.u32(0x44, l.block_44);
    image.u32(0x48, l.block_48);
    image.dwords(0x4C, &l.list_4c);
    image.dwords(0x54, &l.list_54);
    image.dwords(0x5C, &l.list_5c);
    image.u32(0x64, l.field_64);
    image.u32(0x68, l.field_68);
    image.u32(0x6C, l.field_6c);
    write_actor_values(image, 0x70, &l.damage_modifiers_70);
    image.u8(0x84, l.byte_84);
    image.u32(0x88, l.unset_88);
    image.f32(0x8C, l.float_8c);
}

fn low_image(world: &World, process: Entity) -> Image {
    let mut image = Image::new(0x90, true);
    write_low(&mut image, world, process);
    image
}

// IDA: N/A HighProcess +0x00..+0x2EB.
fn high_image(world: &World, process: Entity) -> Image {
    let mut image = Image::new(0x2EC, true);
    write_low(&mut image, world, process);
    let ml = world.get::<MiddleLowProcess>(process).unwrap();
    let mh = world.get::<MiddleHighProcess>(process).unwrap();
    let h = world.get::<HighProcess>(process).unwrap();
    write_actor_values(&mut image, 0x94, &ml.max_modifiers_94);
    image.u32(0x90, ml.field_90 as u32);
    image.dwords(0xA8, &mh.fields_a8[..4]);
    image.f32(0xB8, mh.float_b8);
    image.f32(0xBC, mh.float_bc);
    image.u32(0xC0, mh.current_package_c0);
    image.f32(0xC4, mh.float_c4);
    image.u8(0xC8, mh.byte_c8);
    image.u32(0xCC, mh.procedure_type_cc);
    image.u8(0xD0, mh.byte_d0);
    image.u32(0xE0, mh.field_e0);
    image.dwords(
        0xE4,
        &[mh.weapon_e4, mh.light_e8, mh.ammo_ec, mh.shield_f0]
            .map(|held| u32::from(held.is_some())),
    );
    image.u8(0xF4, mh.bow_f4);
    image.u8(0xF5, mh.staff_f5);
    image.f32(0xF8, mh.float_f8);
    let nodes = &mh.weapon_nodes_fc;
    image.dwords(
        0xFC,
        &[
            nodes.weapon_fc,
            nodes.torch_100,
            nodes.forearm_twist_104,
            nodes.sheath_108,
            nodes.quiver_10c,
            nodes.arrow_bone_110,
        ]
        .map(|node| u32::from(node.is_some())),
    );
    image.u8(0x114, mh.byte_114);
    image.u8(0x115, mh.byte_115);
    image.u8(0x11C, mh.knocked_state_11c);
    image.u8(0x11D, mh.byte_11d);
    image.u32(0x120, mh.field_120);
    image.u8(0x124, mh.byte_124);
    image.dwords(0x128, &mh.point_128[..3]);
    image.u16(0x134, mh.word_134);
    image.u8(0x136, mh.byte_136);
    image.u16(0x138, mh.word_138);
    image.u32(0x13C, mh.field_13c);
    image.u32(0x140, mh.field_140);
    image.u32(0x144, mh.queued_magic_item_144);
    image.u32(0x148, mh.field_148);
    image.u8(0x14C, mh.byte_14c);
    image.u32(0x150, mh.field_150);
    image.f32(0x154, mh.actor_alpha_154);
    image.f32(0x158, mh.float_158);
    image.u32(0x15C, mh.field_15c);
    image.u8(0x160, mh.byte_160);
    image.u8(0x161, mh.byte_161);
    image.u32(0x164, mh.field_164);
    image.u32(0x170, mh.field_170);
    image.u32(0x178, mh.field_178);
    image.u32(0x17C, u32::from(mh.anim_data_17c.is_some()));
    image.u8(0x180, mh.byte_180);
    image.u32(0x184, mh.skin_property_184);
    image.u32(0x188, mh.bounding_box_188);
    image.u32(0x190, h.field_190);
    image.u32(0x194, h.field_194);
    image.f32(0x198, h.float_198);
    image.f32(0x19C, h.float_19c);
    image.u8(0x1A0, h.byte_1a0);
    image.u32(0x1A4, h.field_1a4);
    image.f32(0x1A8, h.float_1a8);
    image.f32(0x1AC, h.float_1ac);
    image.f32(0x1B0, h.float_1b0);
    image.f32(0x1B4, h.float_1b4);
    image.f32(0x1B8, h.float_1b8);
    image.dwords(0x1BC, &h.fields_1bc[..4]);
    image.u32(0x1CC, h.field_1cc);
    image.u8(0x1D0, h.byte_1d0);
    image.u8(0x1D1, h.byte_1d1);
    image.u32(0x1D4, h.field_1d4);
    image.f32(0x1D8, h.float_1d8);
    image.f32(0x1DC, h.float_1dc);
    image.f32(0x1E0, h.float_1e0);
    image.u8(0x1E4, h.byte_1e4);
    image.f32(0x1E8, h.float_1e8);
    image.u32(0x1EC, h.field_1ec);
    image.u32(0x1F0, h.field_1f0);
    image.u16(0x1F4, h.current_action_1f4);
    image.u32(0x1F8, h.field_1f8);
    image.u16(0x1FC, h.movement_flags_1fc);
    image.u32(0x200, h.field_200);
    image.f32(0x204, h.float_204);
    image.u16(0x208, h.word_208);
    image.dwords(0x20C, &h.point_20c[..3]);
    image.u32(0x218, h.field_218);
    image.f32(0x21C, h.float_21c);
    image.dwords(0x220, &h.fields_220[..2]);
    image.u8(0x228, h.byte_228);
    image.f32(0x22C, h.float_22c);
    image.f32(0x230, h.float_230);
    image.f32(0x234, h.float_234);
    image.f32(0x238, h.swim_breath_238);
    image.u8(0x23C, h.byte_23c);
    image.f32(0x240, h.float_240);
    image.u8(0x244, h.byte_244);
    image.f32(0x248, h.float_248);
    image.u8(0x24C, h.byte_24c);
    image.u32(0x250, h.field_250);
    image.u32(0x254, h.unset_254);
    image.u32(0x258, h.field_258);
    image.u8(0x25C, h.byte_25c);
    image.u8(0x25D, h.byte_25d);
    image.f32(0x260, h.float_260);
    image.f32(0x264, h.float_264);
    image.u32(0x268, h.field_268);
    image.f32(0x26C, h.float_26c);
    image.f32(0x270, h.float_270);
    image.u32(0x274, h.field_274);
    image.u8(0x278, h.byte_278);
    image.dwords(0x27C, &h.point_27c[..3]);
    image.u32(0x288, h.field_288);
    image.f32(0x28C, h.float_28c);
    image.u8(0x290, h.byte_290);
    image.f32(0x294, h.float_294);
    image.u32(0x2A4, h.field_2a4);
    image.u8(0x2A8, h.byte_2a8);
    image.u8(0x2A9, h.byte_2a9);
    image.f32(0x2AC, h.float_2ac);
    image.f32(0x2B0, h.float_2b0);
    image.u32(0x2B4, h.field_2b4);
    image.u8(0x2B8, h.byte_2b8);
    image.u8(0x2B9, h.byte_2b9);
    image.u32(0x2BC, h.field_2bc);
    image.f32(0x2C0, h.float_2c0);
    image.u32(0x2C4, h.field_2c4);
    image.dwords(0x2C8, &h.action_targets_2c8[..5]);
    image.u32(0x2E4, h.field_2e4);
    image.u8(0x2E8, h.byte_2e8);
    image.ptr(0x118, mh.char_controller_118.is_some(), "C");
    for (index, byte) in mh.bytes_168.iter().enumerate() {
        image.u8(0x168 + index, *byte);
    }
    image.u8(0x16C, mh.equipment_update_pending_16c);
    image.u8(0x16D, mh.byte_16d);
    image.ptr(0x174, mh.effect_list_174.is_some(), "blk");
    image.ptr(0x18C, h.detection_list_18c.is_some(), "blk");
    image.dwords(0x298, &h.fields_298.map(|value| value as u32));
    for (index, byte) in h.action_active_2dc.iter().enumerate() {
        image.u8(0x2DC + index, *byte);
    }
    image
}

// IDA: N/A one list at its image offset.
fn write_process_list(image: &mut Image, at: usize, list: &ActorProcessList, tail_role: &str) {
    image.ptr(at, list.list.item(ActorListNode::HEAD).is_some(), "P");
    image.ptr(
        at + 4,
        list.list.next(ActorListNode::HEAD).is_some(),
        "node",
    );
    let tail = if list.tail_08 == ActorListNode::HEAD {
        tail_role
    } else {
        "node"
    };
    image.ptr(at + 8, true, tail);
}

// IDA: N/A the 0xB0-byte manager static (BSS: unmodeled bytes are zero).
fn manager_image(world: &World) -> String {
    let m = &world.resource::<ActorProcessGlobals>().manager;
    let mut image = Image::new(0xB0, true);
    write_process_list(&mut image, 0x00, &m.middle_high_00, "M+00");
    write_process_list(&mut image, 0x0C, &m.low_0c, "M+0C");
    write_process_list(&mut image, 0x18, &m.low_18, "M+18");
    image.u32(0x28, m.field_28);
    image.dwords(0x40, &m.pointer_40);
    image.dwords(0x48, &m.pointer_48);
    image.dwords(0x50, &m.fields_50);
    write_process_list(&mut image, 0x68, &m.high_68, "M+68");
    image.ptr(0x74, m.cursor_74.is_some(), "node");
    image.ptr(0x78, m.cursor_78.is_some(), "node");
    let map = &m.lip_tasks_7c;
    image.u32(0x7C, map.original_vtable);
    image.ptr(0x80, map.entry_count != 0, "blk");
    image.u32(0x84, map.bucket_count_08);
    image.ptr(0x88, !map.buckets_0c.is_empty(), "blk");
    image.u32(0x8C, map.field_10);
    image.ptr(0x90, true, "blk");
    image.u32(0x94, map.field_18);
    image.u8(0x98, m.byte_98);
    image.u8(0x99, m.byte_99);
    image.u8(0x9A, m.byte_9a);
    image.u32(0x9C, m.field_9c);
    for (index, byte) in m.bytes_a0.iter().enumerate() {
        image.u8(0xA0 + index, *byte);
    }
    image.u32(0xA8, m.field_a8);
    image.f32(0xAC, m.remove_excess_dead_time_ac);
    image.text()
}

// IDA: N/A the controller's Bethesda-side offsets (the rest is SDK / substituted: "??").
fn controller_image(c: &BhkCharacterController) -> Image {
    let mut image = Image::new(0x3E0, false);
    image.u32(0x00, c.original_vtable);
    image.u32(0x04, c.reference_count_04);
    image.u32(0x0C, c.field_0c);
    image.u32(0x10, c.collector_vtable_10);
    image.u32(0x1E0, c.context_vtable_1e0);
    image.u16(0x1E6, c.context_reference_count_1e6);
    image.ptr(0x1E8, c.context_state_manager_1e8, "sm");
    image.u32(0x1EC, c.context_field_1ec);
    image.u32(0x1F0, c.listener_vtable_1f0);
    image.u32(0x1F4, c.flags_1f4);
    image.floats(0x200, &c.listener_up_200);
    image.f32(0x210, c.listener_cos_210);
    image.u32(0x214, c.listener_field_214);
    image.u32(0x218, c.listener_field_218);
    image.dwords(0x220, &c.listener_zero_220);
    image.f32(0x240, c.listener_float_240);
    image.f32(0x244, c.listener_float_244);
    image.f32(0x248, c.listener_float_248);
    image.f32(0x24C, c.listener_float_24c);
    image.u8(0x250, c.listener_byte_250);
    image.u8(0x251, c.listener_byte_251);
    image.u32(0x254, c.listener_field_254);
    image.u32(0x2A0, c.field_2a0);
    image.floats(0x2B0, &c.floats_2b0);
    image.dwords(0x2F0, &c.zero_2f0);
    image.floats(0x300, &c.floats_300);
    image.floats(0x320, &c.floats_320[..6]);
    image.f32(0x33C, c.floats_320[7]);
    image.floats(0x340, &c.floats_340);
    image.ptr(0x364, c.phantom_364.is_some(), "Ph");
    image.ptr(0x368, c.proxy_object_368.is_some(), "Px");
    image.u32(0x36C, c.shape_index_36c);
    image.u32(0x370, c.field_370);
    image.ptr(0x374, c.shapes_374[0].is_some(), "cap");
    image.ptr(0x378, c.shapes_374[1].is_some(), "shared");
    image.f32(0x3A0, c.float_3a0);
    image.f32(0x3A4, c.float_3a4);
    image.f32(0x3A8, c.float_3a8);
    image.u32(0x3AC, c.field_3ac);
    image.u32(0x3B0, c.field_3b0);
    image.u32(0x3B4, c.field_3b4);
    image.u32(0x3B8, c.field_3b8);
    image.ptr(0x3BC, c.havok_array_3bc, "havok");
    image.u32(0x3C0, c.field_3c0);
    image.u32(0x3C4, c.field_3c4);
    image
}

/// IDA: N/A a 0x14 bhk wrapper.
fn world_object_image(object: &BhkWorldObjectRecord, compare_count: bool) -> Image {
    let mut image = Image::new(0x14, false);
    image.u32(0x00, object.original_vtable);
    if compare_count {
        image.u32(0x04, object.reference_count_04);
    }
    image.ptr(0x08, object.havok_object_08, "hk");
    image.u32(0x0C, object.field_0c);
    image.u8(0x10, object.byte_10);
    image
}

fn check_window_state(rows: &Rows, inputs: RunInputs) {
    let mut h = Harness::new(inputs);
    // W: the forms map and the DataHandler counter before the window.
    let forms = h.world.resource::<FormsById>();
    let data = h.world.resource::<DataHandler>();
    let set = |key: u32| {
        if forms.lookup(key).is_some() {
            "set"
        } else {
            "absent"
        }
    };
    rows.compare(
        &format!(
            "W count={:X} dh8C0={:X} dh8C4={:X} k800={} k801={} k14={} k3D={} tls184=0{:X}",
            forms.count(),
            data.next_form_id_8c0(),
            u32::from(data.active_file_8c4.is_some()),
            set(0x800),
            set(0x801),
            set(0x14),
            set(0x3D),
            h.world.resource::<GameThreadLocals>().byte_184,
        ),
        "W ",
    );
    rows.compare(
        &format!("G window {}", manager_image(&h.world)),
        "G window ",
    );
    // B: the globals before the window.
    rows.compare(&globals_line(&h, "window"), "B window ");
    // I: Random_Next(0) with 00B069C3 still set: the first-use seed arm.
    let line = rows.expected("I ");
    assert_eq!(field(line, "seed"), "0");
    assert_eq!(
        field_hex(line, "b069c3"),
        u32::from(h.manager().random.first_use_b069c3)
    );
    // R: the first form after the DataHandler (0046C58F) takes 0x800; the counter moves on.
    let form = construct_tes_form(&mut h.world).unwrap();
    rows.compare(
        &format!(
            "R refid={:X} counter={:X} file=0",
            h.world.get::<FormIdentity>(form).unwrap().form_id_0c,
            h.world.resource::<DataHandler>().next_form_id_8c0(),
        ),
        "R ",
    );
}

// IDA: N/A the "B ..." line of the host's globals.
fn globals_line(h: &Harness, label: &str) -> String {
    let g = h.world.resource::<PlayerGlobals>();
    let processes = h.manager();
    let controllers = h.world.resource::<CharacterControllerGlobals>();
    let point: String = g.point_b3bb38.iter().map(|value| le(*value)).collect();
    format!(
        "B {label} thePlayer={} B3BB24={} B3BACC={} B3BB38={point} B14E4D=0{:X} B3BAFC={} B3B928={} B2D80C={} B2EB3C={} B069C3=0{:X}",
        if g.the_player_b333c4.is_some() {
            "set"
        } else {
            "null"
        },
        le(g.vanity_wheel_b3bb24.to_bits()),
        le(g.vanity_wheel_copy_b3bacc.to_bits()),
        g.byte_b14e4d,
        le(g.b3bafc.to_bits()),
        le(processes.high_process_b3b928),
        le(g.particle_fov_scale_b2d80c.to_bits()),
        le(controllers.collision_group_b2eb3c),
        processes.random.first_use_b069c3,
    )
}

fn run_window(h: &mut Harness) -> (Entity, Vec<&'static str>) {
    let mut logs = Vec::new();
    let result = h.with_player_inputs(|world, inputs| {
        run_player_window(world, inputs, false, &mut |text| logs.push(text))
    });
    (result.unwrap().player.unwrap(), logs)
}

// IDA: N/A the constructor alone, as P11-RUN-001 dumped it at 00406798: g_thePlayer still null.
fn check_after_constructor(rows: &Rows, inputs: RunInputs) {
    let mut h = Harness::new(inputs);
    h.with_player_inputs(construct_player_character).unwrap();
    rows.compare(&globals_line(&h, "done"), "B done ");
}

fn check_run001(rows: &Rows, inputs: RunInputs) {
    let mut h = Harness::new(inputs);
    let camera_before = h.scene_camera();
    let fov_before = h.world.get::<SceneGraph>(h.graph).unwrap().camera_fov_ec;
    let (player, logs) = run_window(&mut h);
    assert_eq!(
        h.world.resource::<PlayerGlobals>().the_player_b333c4,
        Some(player),
        "004067A6 g_thePlayer"
    );
    assert_eq!(logs, ["Loading Files..."], "004067B5 log");
    // F: the final map state (0x800 gone, 0x14 the player).
    {
        let forms = h.world.resource::<FormsById>();
        let mut form = Image::new(0x18, true);
        write_form(&mut form, &h.world, player);
        rows.compare(
            &format!(
                "F final count={:X} dh8C0={:X} k800={} k14={} form={}",
                forms.count(),
                h.world.resource::<DataHandler>().next_form_id_8c0(),
                if forms.lookup(0x800).is_some() {
                    "set"
                } else {
                    "absent"
                },
                if forms.lookup(PLAYER_FORM_ID) == Some(player) {
                    "P"
                } else {
                    "other"
                },
                form.text(),
            ),
            "F final ",
        );
    }
    // The whole player as P11-RUN-001 dumped it at 00406798 (before 004067AB set 0x14).
    let mut player_bytes = player_image(&h.world, player);
    player_bytes.u32(0x0C, 0x800);
    compare_image(rows, &player_bytes, "player", Some("player"));
    // The HighProcess after the constructor; its controller pointer is the controller.
    let high = h
        .world
        .get::<MobileObject>(player)
        .unwrap()
        .process_58
        .unwrap();
    assert!(
        h.world.get::<HighProcess>(high).is_some(),
        "+0x58 HighProcess"
    );
    assert_eq!(process_level(&h.world, high), 0, "006F7070");
    compare_image(rows, &high_image(&h.world, high), "high_done", Some("high"));
    // Manager after the constructor.
    rows.compare(&format!("G done {}", manager_image(&h.world)), "G done ");
    // Random_Next seeded once with _time64 (00B069C3 was 1).
    assert_eq!(
        h.random.seeds,
        [inputs.time64],
        "0047DFAE srand(_time64) once"
    );
    assert_eq!(h.random.time_calls, 1);
    // V: +0x71C/+0x71D and the +0x368 enabled byte, through the three 0065A310 calls.
    let controller =
        actor_character_controller(&h.world, player).expect("0065B750 created the controller");
    let c = h.world.get::<BhkCharacterController>(controller).unwrap();
    let p = h.world.get::<PlayerCharacter>(player).unwrap();
    let globals = h.world.resource::<CharacterControllerGlobals>();
    let calls = &globals.enable_calls;
    assert_eq!(calls.len(), 3, "0066AA70, 0066AF18, 0066AF37");
    rows.compare(
        &format!(
            "V b71c=0{:X} b71d=0{:X} argA={} argB={} o368_before={:02X} o368_after={:02X}",
            p.sit_sleep_entry_pending_71c,
            p.sit_sleep_exit_pending_71d,
            u8::from(calls[1].argument),
            u8::from(calls[2].argument),
            calls[0].byte_before,
            calls[0].byte_after,
        ),
        "V ",
    );
    assert_eq!(
        calls[1],
        ControllerEnableCall {
            argument: true,
            byte_before: 1,
            byte_after: 1
        },
        "0066AF18: already enabled"
    );
    // The controller, its cinfo, the two objects and their layers.
    rows.compare(
        &format!("A controller align=0{:X}", c.alignment_m1),
        "A controller ",
    );
    compare_image(rows, &controller_image(c), "controller", None);
    let cinfo = globals.last_cinfo.unwrap();
    let mut image = Image::new(0xA4, false);
    image.floats(0x00, &cinfo.position_00);
    image.u32(0x0C, cinfo.field_0c);
    image.dwords(0x10, &cinfo.vector_10);
    image.f32(0x20, cinfo.float_20);
    image.f32(0x24, cinfo.float_24);
    image.f32(0x28, cinfo.float_28);
    image.f32(0x30, cinfo.float_30);
    image.f32(0x34, cinfo.float_34);
    image.f32(0x38, cinfo.float_38);
    image.f32(0x3C, cinfo.float_3c);
    image.f32(0x40, cinfo.float_40);
    image.f32(0x44, cinfo.float_44);
    image.f32(0x4C, cinfo.float_4c);
    image.f32(0x50, cinfo.float_50);
    image.u32(0x54, cinfo.field_54);
    image.f32(0x58, cinfo.float_58);
    image.f32(0x5C, cinfo.float_5c);
    image.f32(0x60, cinfo.float_60);
    image.f32(0x64, cinfo.float_64);
    image.f32(0x68, cinfo.float_68);
    image.u32(0x70, cinfo.loaded_3d_70);
    image.u32(0x74, cinfo.collision_filter_74);
    image.u32(0x78, cinfo.field_78);
    image.f32(0x7C, cinfo.jump_scale_7c);
    image.f32(0x80, cinfo.float_80);
    image.u8(0x84, cinfo.byte_84);
    image.u8(0x85, cinfo.no_base_bound_85);
    image.u32(0x88, cinfo.field_88);
    image.u32(0x8C, cinfo.field_8c);
    image.ptr(0x90, cinfo.state_manager_90, "sm");
    image.f32(0x94, cinfo.float_94);
    image.f32(0x98, cinfo.float_98);
    image.u32(0x9C, cinfo.field_9c);
    image.u32(0xA0, cinfo.field_a0);
    compare_image(rows, &image, "cinfo", None);
    // As 00890F70 releases it: 00897040 stored +0x48 (the phantom's hk object) and 00895190 +0x50 (the radius in Havok units).
    let released = globals.last_cinfo_released.unwrap();
    image.ptr(0x48, released.phantom_48 != 0, "hk");
    image.f32(0x50, released.float_50);
    compare_image(rows, &image, "cinfo_released", None);
    let phantom = h
        .world
        .get::<BhkWorldObjectRecord>(c.phantom_364.unwrap())
        .unwrap();
    compare_image(rows, &world_object_image(phantom, true), "Ph", None);
    assert_eq!(
        phantom.reference_count_04, 2,
        "00897105 NiPointer store, 008B9C1B the bhk wrapper"
    );
    assert_eq!(phantom.property_keys, [1000], "008910F0 property 1000");
    // The bhkAvoidBox (0x80 aligned block): the wrapper stores, the AABB and the vectors.
    let proxy = h
        .world
        .get::<BhkWorldObjectRecord>(c.proxy_object_368.unwrap())
        .unwrap();
    let mut image = Image::new(0x70, false);
    image.u32(0x00, proxy.original_vtable);
    image.u32(0x04, proxy.reference_count_04);
    image.ptr(0x08, proxy.havok_object_08, "hk");
    image.u32(0x0C, proxy.field_0c);
    image.u8(0x10, proxy.byte_10);
    image.floats(0x20, &proxy.aabb_20);
    image.floats(0x40, &proxy.vectors_40);
    image.f32(0x60, proxy.float_60);
    image.f32(0x64, proxy.float_64);
    image.u8(0x68, proxy.enabled_68);
    image.u8(0x69, proxy.byte_69);
    compare_image(rows, &image, "Px", None);
    // The two 0089F4D0 calls, with the filter info before and after.
    assert_eq!(globals.layer_calls.len(), 2, "0066AA81, 0066AA92");
    let (before, after) = globals.layer_calls[0];
    assert_eq!(
        before, cinfo.collision_filter_74,
        "the phantom's filter is the cinfo's"
    );
    rows.compare(
        &format!("L layer364 before={before:08X} after={after:08X}"),
        "L layer364 ",
    );
    assert_eq!(after, phantom.collision_filter_info);
    let (before, after) = globals.layer_calls[1];
    rows.compare(
        &format!("L layer368 before={before:08X} after={after:08X}"),
        "L layer368 ",
    );
    assert_eq!(after, proxy.collision_filter_info);
    // The camera collision helper and its phantoms.
    {
        let helper_entity = p.camera_collision_helper_1f0.unwrap();
        let helper = h.world.get::<CameraCollisionHelper>(helper_entity).unwrap();
        let mut image = Image::new(8, true);
        image.ptr(0, true, "Pha");
        image.ptr(4, true, "Phb");
        compare_image(rows, &image, "camhelper", None);
        let a = h
            .world
            .get::<BhkWorldObjectRecord>(helper.phantom_00)
            .unwrap();
        let b = h
            .world
            .get::<BhkWorldObjectRecord>(helper.phantom_04)
            .unwrap();
        compare_image(rows, &world_object_image(a, true), "Pha", None);
        compare_image(rows, &world_object_image(b, true), "Phb", None);
        // These three are C++ values: no run dump holds the hk objects.
        assert_eq!(a.collision_filter_info, 0x0009_0018, "00532370 layer 9");
        assert_eq!(b.collision_filter_info, 0x000C_0018, "00532370 group 0x0C");
        assert_eq!(
            h.world
                .get::<BhkSphereShapeRecord>(helper.shape)
                .unwrap()
                .havok_radius,
            8.0 * HAVOK_SCALE,
            "the 8.0 sphere"
        );
    }
    {
        let helper = h
            .world
            .get::<CollisionQueryHelper>(p.collision_query_helper_1f4.unwrap())
            .unwrap();
        rows.compare(
            &format!("A query align=0{:X}", helper.alignment_m1),
            "A query ",
        );
        let mut head = Image::new(0x1C, false);
        head.u32(0x00, helper.original_vtable);
        head.f32(0x04, helper.early_out_distance_04);
        head.ptr(0x10, helper.inline_storage_10, "Q+20");
        head.u32(0x14, helper.size_14);
        head.u32(0x18, helper.capacity_18);
        compare_image(rows, &head, "query_head", None);
        let mut tail = Image::new(0x0C, false);
        tail.ptr(0x00, true, "Phq");
        tail.u8(0x04, helper.field_1a4);
        tail.u32(0x08, helper.layer_1a8);
        compare_image(rows, &tail, "query_tail", None);
        let phantom = h
            .world
            .get::<BhkWorldObjectRecord>(helper.phantom_1a0)
            .unwrap();
        compare_image(rows, &world_object_image(phantom, true), "Phq", None);
        // C++ values, as above.
        assert_eq!(
            phantom.collision_filter_info, 0x0009_001C,
            "00535730 filter info"
        );
        assert_eq!(
            h.world
                .get::<BhkSphereShapeRecord>(helper.shape)
                .unwrap()
                .havok_radius,
            2.0 * HAVOK_SCALE,
            "the 2.0 sphere"
        );
    }
    // The group counter through the window: 10 in the image, the cinfo's high word, then the camera helper's.
    rows.compare(
        &format!(
            "K b2eb3c window={} cinfo={} camhelper={}",
            le(CharacterControllerGlobals::default().collision_group_b2eb3c),
            le(cinfo.collision_filter_74 >> 16),
            le(globals.collision_group_b2eb3c),
        ),
        "K b2eb3c ",
    );
    // The small blocks.
    let mut block = Image::new(8, true);
    block.dwords(0, &p.active_effects_1e4.unwrap());
    compare_image(rows, &block, "sub1E4", None);
    block.dwords(0, &p.pending_potion_list_1f8.unwrap());
    compare_image(rows, &block, "sub1F8", None);
    let mut chargen = Image::new(0x54, true);
    chargen.floats(0, &p.chargen_skill_exp_5b0.unwrap());
    compare_image(rows, &chargen, "sub5B0", None);
    let bonuses = p.attribute_bonuses_5b4.as_ref().unwrap();
    let mut image = Image::new(8, true);
    image.ptr(0, !bonuses.items.is_empty(), "blk");
    image.ptr(4, bonuses.items.len() > 1, "node");
    compare_image(rows, &image, "sub5B4", None);
    let mut buckets = Image::new(0x94, true);
    buckets.dwords(0, &p.random_teleport_index_by_door_788.buckets_08);
    compare_image(rows, &buckets, "sub790", None);
    // The FOV call returned early (75 == the World graph's +0xEC) and the particle scale.
    let camera_after = h.scene_camera();
    rows.compare(
        &format!(
            "C fov={:08X} force=0 sgfov_before={:08X} sgfov_after={:08X} camera_changed={}",
            p.camera_fov_598.to_bits(),
            fov_before.to_bits(),
            h.world
                .get::<SceneGraph>(h.graph)
                .unwrap()
                .camera_fov_ec
                .to_bits(),
            if camera_before == camera_after {
                "no"
            } else {
                "yes"
            },
        ),
        "C fov=",
    );
    rows.compare(
        &format!(
            "C b2d80c={}",
            le(h.world
                .resource::<PlayerGlobals>()
                .particle_fov_scale_b2d80c
                .to_bits())
        ),
        "C b2d80c=",
    );
}

// IDA: N/A the LowProcess (built and before deletion) and the HighProcess built / copied.
fn check_processes(rows: &Rows, inputs: RunInputs) {
    let mut h = Harness::new(inputs);
    let (world, mut process) = h.process_inputs();
    let low = construct_low_process(world, &process);
    compare_image(rows, &low_image(world, low), "low_built", Some("low"));
    let high = construct_high_process(world, &mut process);
    compare_image(rows, &high_image(world, high), "high_built", Some("high"));
    copy_process_from(world, high, Some(low)).unwrap();
    compare_image(rows, &low_image(world, low), "low_old", Some("low"));
    compare_image(rows, &high_image(world, high), "high_copied", Some("high"));
    delete_low_process(world, low).unwrap();
    assert!(world.get_entity(low).is_err(), "0064ACDF FormHeap_Free");
}

// IDA: N/A the manager after the Character constructor's level-3 add (005E16C8).
fn check_manager_after_add(rows: &Rows, inputs: RunInputs) {
    let mut h = Harness::new(inputs);
    let character = h.character();
    // The run's list item is the player; the host's is this character: same role.
    rows.compare(
        &format!("G after_add {}", manager_image(&h.world)),
        "G after_add ",
    );
    assert_eq!(
        h.manager().manager.low_18.list.items(),
        [character],
        "0067B283 prepend"
    );
    assert!(actor_process_manager_remove(
        &mut h.world,
        character,
        3,
        false
    ));
    rows.compare(
        &format!("G old_dtor {}", manager_image(&h.world)),
        "G old_dtor ",
    );
}

/// Every line of `phase11.rows` read by a check. One test, as the accounting spans them all.
#[test]
fn every_row_is_read() {
    let rows = Rows::load();
    let inputs = run_inputs(&rows);
    check_window_state(&rows, inputs);
    check_processes(&rows, inputs);
    check_manager_after_add(&rows, inputs);
    check_after_constructor(&rows, inputs);
    check_run001(&rows, inputs);
    assert_eq!(rows.unread(), Vec::<&str>::new(), "unread rows");
    assert_eq!(rows.lines.len(), 43);
}

fn harness() -> Harness {
    Harness::new(run_inputs(&Rows::load()))
}

// The GMST and INI sources of `PlayerSettingInputs` against the values P11-RUN-001 recorded, on the run's pinned INI. The C++ test injected these and did not read them this way.
#[test]
fn setting_inputs_come_from_the_collections() {
    let inputs = run_inputs(&Rows::load());
    let mut collections = SettingCollections::default();
    register_static_settings(&mut collections);
    collections
        .ini
        .set_path(fixtures::root().join("config/Oblivion.ini"));
    assert!(collections.ini.open_ini());
    collections.ini.read_all();
    collections.ini.close();
    let read = player_setting_inputs(&GameSettingCollection::default(), &collections.ini).unwrap();
    assert_eq!(read, inputs.settings);
}

// IDA: N/A 004067AB's host stop on a failed 0x804 allocation (g_thePlayer null).
#[test]
fn allocation_failure_stops() {
    let mut h = harness();
    let count = h.world.resource::<FormsById>().count();
    let result = h.with_player_inputs(|world, inputs| {
        run_player_window(world, inputs, true, &mut |_| {
            panic!("no log after the stop")
        })
    });
    assert!(result.is_err());
    assert_eq!(h.world.resource::<PlayerGlobals>().the_player_b333c4, None);
    assert_eq!(h.world.resource::<FormsById>().count(), count);
}

// IDA: N/A the 00B3BD00 static has no driver yet, so a missing manager stops rather than panics.
#[test]
fn missing_manager_static_stops() {
    let mut h = harness();
    h.world.remove_resource::<ActorProcessGlobals>();
    let result = h.with_player_inputs(|world, inputs| {
        run_player_window(world, inputs, false, &mut |_| {
            panic!("no log after the stop")
        })
    });
    assert!(result.is_err());
}

fn form_id(world: &World, form: Entity) -> u32 {
    world.get::<FormIdentity>(form).unwrap().form_id_0c
}

// IDA: N/A a world with a built DataHandler, as the arms use it.
fn data_world() -> World {
    let mut world = World::new();
    construct_data_handler(&mut world).unwrap();
    world
}

#[test]
fn form_without_data_handler_gets_no_id() {
    let mut world = World::new();
    insert_form_indexes(&mut world);
    world.init_resource::<DataHandlerGlobals>();
    world.init_resource::<GameThreadLocals>();
    let form = construct_tes_form(&mut world).unwrap();
    assert_eq!(form_id(&world, form), 0, "0046C571 no DataHandler");
    assert_eq!(world.resource::<FormsById>().count(), 0);
}

#[test]
fn form_with_tls_flag_gets_no_id() {
    let mut world = data_world();
    let count = world.resource::<FormsById>().count();
    world.resource_mut::<GameThreadLocals>().byte_184 = 1;
    let form = construct_tes_form(&mut world).unwrap();
    assert_eq!(form_id(&world, form), 0, "0046C586 TLS +0x184");
    assert_eq!(world.resource::<FormsById>().count(), count);
}

#[test]
fn allocator_skips_taken_ids() {
    let mut world = data_world();
    let first = construct_tes_form(&mut world).unwrap();
    *world.resource_mut::<DataHandler>().next_form_id_8c0_mut() = 0x800;
    let second = construct_tes_form(&mut world).unwrap();
    assert_eq!(form_id(&world, first), 0x800);
    assert_eq!(form_id(&world, second), 0x801, "00448D40 taken key");
    assert_eq!(world.resource::<DataHandler>().next_form_id_8c0(), 0x802);
    assert_eq!(world.resource::<FormsById>().lookup(0x801), Some(second));
    // 00453450: the save/load map counts as taken.
    world
        .resource_mut::<DataHandlerGlobals>()
        .save_load_form_ids
        .push(0x803);
    let third = construct_tes_form(&mut world).unwrap();
    assert_eq!(form_id(&world, third), 0x802);
    assert_eq!(world.resource::<DataHandler>().next_form_id_8c0(), 0x804);
}

#[test]
fn allocator_wraps_and_low_ids_move() {
    let mut world = data_world();
    *world.resource_mut::<DataHandler>().next_form_id_8c0_mut() = 0x01FF_FFFF;
    let wrapped = construct_tes_form(&mut world).unwrap();
    assert_eq!(form_id(&world, wrapped), 0x01FF_FFFF);
    assert_eq!(
        world.resource::<DataHandler>().next_form_id_8c0(),
        0x0100_0800,
        "00448D76 wrap"
    );
    // An ID at or below 0x7FF moves to 0x800.
    *world.resource_mut::<DataHandler>().next_form_id_8c0_mut() = 0x10;
    let low = construct_tes_form(&mut world).unwrap();
    assert_eq!(form_id(&world, low), 0x800, "0046C5B9");
    let forms = world.resource::<FormsById>();
    assert_eq!(forms.lookup(0x800), Some(low));
    assert_eq!(forms.lookup(0x10), None);
}

#[test]
fn active_file_updates_the_mod_references() {
    let mut world = data_world();
    let (master, plugin) = {
        let mut data = world.resource_mut::<DataHandler>();
        data.file_store.files.push(Some(TesFileData {
            flags_3dc: 1,
            mod_index_400: 0,
            ..TesFileData::default()
        }));
        data.file_store.files.push(Some(TesFileData {
            mod_index_400: 5,
            ..TesFileData::default()
        }));
        let plugin = TesFileId(data.file_store.files.len() - 1);
        data.active_file_8c4 = Some(plugin);
        (TesFileId(plugin.0 - 1), plugin)
    };
    let form = construct_tes_form(&mut world).unwrap();
    let mods = |world: &World| world.get::<ModReferences>(form).unwrap().0.clone();
    assert_eq!(mods(&world), [5], "0046C597 the active file");
    update_mod_reference_list(&mut world, form, Some(plugin)).unwrap();
    assert_eq!(mods(&world), [5], "0046B770 plugin appended once");
    update_mod_reference_list(&mut world, form, Some(master)).unwrap();
    assert_eq!(mods(&world), [0], "0046B75C master clears and prepends");
    update_mod_reference_list(&mut world, form, Some(plugin)).unwrap();
    update_mod_reference_list(&mut world, form, None).unwrap();
    assert_eq!(mods(&world), [0], "0046B6DD null removes the last entry");
    // 0044FB90: no ghost files.
    world
        .resource_mut::<DataHandler>()
        .file_store
        .files
        .last_mut()
        .unwrap()
        .as_mut()
        .unwrap()
        .ghost_parent_04 = 1;
    assert!(update_mod_reference_list(&mut world, form, Some(plugin)).is_err());
}

#[test]
fn set_form_id_and_release() {
    let mut world = data_world();
    // 0046C300 with a present key and release 0.
    let a = world
        .spawn(FormIdentity {
            form_type_04: 0,
            flags_08: 8,
            form_id_0c: 0,
        })
        .id();
    let b = world
        .spawn(FormIdentity {
            form_type_04: 0,
            flags_08: 8,
            form_id_0c: 0,
        })
        .id();
    set_form_id_release(&mut world, a, 0x14, false).unwrap();
    set_form_id_release(&mut world, b, 0x14, false).unwrap();
    assert_eq!(
        world.resource::<FormsById>().lookup(0x14),
        Some(b),
        "00452570 a present key takes the new form"
    );
    assert_eq!(form_id(&world, a), 0x14);
    // 00446B40.
    let mut data = world.resource_mut::<DataHandler>();
    *data.next_form_id_8c0_mut() = 0xFF00_0901;
    assert_eq!(
        release_form_id(Some(&mut data), 0xFF00_0900),
        Ok(0xFF00_0900)
    );
    assert_eq!(
        data.next_form_id_8c0(),
        0xFF00_0900,
        "00446B75 the last dynamic ID is handed back"
    );
    assert_eq!(
        release_form_id(None, 0x800),
        Ok(0x800),
        "00446B56 no 0xFF high byte"
    );
    assert!(
        release_form_id(None, 0xFF00_0900).is_err(),
        "00446B58 null DataHandler host stop"
    );
    // 0046C33E: a set `release` hands the old ID back.
    *data.next_form_id_8c0_mut() = 0xFF00_0901;
    world.get_mut::<FormIdentity>(a).unwrap().form_id_0c = 0xFF00_0900;
    set_form_id_release(&mut world, a, 0x15, true).unwrap();
    assert_eq!(
        world.resource::<DataHandler>().next_form_id_8c0(),
        0xFF00_0900
    );
    // 0046C30F: flag 0x4000 keeps the map.
    world.get_mut::<FormIdentity>(a).unwrap().flags_08 |= 0x4000;
    set_form_id_release(&mut world, a, 0x16, false).unwrap();
    assert_eq!(form_id(&world, a), 0x16);
    let forms = world.resource::<FormsById>();
    assert_eq!(forms.lookup(0x15), Some(a));
    assert_eq!(forms.lookup(0x16), None);
}

// IDA: N/A manager list arms.
#[test]
fn manager_arms() {
    let mut h = harness();
    let a = h.character();
    // g_thePlayer is never added: 0067B266 (built, taken off, then the add under test).
    let b = h.character();
    assert!(actor_process_manager_remove(&mut h.world, b, 3, false));
    h.world.resource_mut::<PlayerGlobals>().the_player_b333c4 = Some(b);
    actor_process_manager_add(&mut h.world, b, 3, false, false).unwrap();
    assert_eq!(
        h.manager().manager.low_18.list.items(),
        [a],
        "0067B266 g_thePlayer skipped"
    );
    h.world.resource_mut::<PlayerGlobals>().the_player_b333c4 = None;
    // 00673ABE: a level other than the process's adds nothing, and 0067B2CB stops.
    actor_process_manager_add(&mut h.world, b, 2, false, false).unwrap();
    assert!(h.manager().manager.low_0c.list.items().is_empty());
    assert_eq!(actor_process_level(&h.world, b), 3);
    assert!(actor_process_manager_add(&mut h.world, b, 3, false, true).is_err());
    // a is in the level-3 list; the cursors pointing at it are cleared by the remove.
    let head = ActorListCursor {
        list: ActorListId::Low18,
        node: ActorListNode::HEAD,
    };
    {
        let mut globals = h.world.resource_mut::<ActorProcessGlobals>();
        globals.manager.cursor_74 = Some(head);
        globals.manager.cursor_78 = Some(head);
    }
    assert!(actor_process_manager_remove(&mut h.world, a, 3, false));
    let manager = &h.manager().manager;
    assert_eq!(manager.cursor_74, None, "00674588 cursors");
    assert_eq!(manager.cursor_78, None);
    assert!(
        manager.low_18.list.items().is_empty(),
        "0067B3D8 head removal"
    );
    assert_eq!(manager.low_18.tail_08, ActorListNode::HEAD);
    assert!(
        !actor_process_manager_remove(&mut h.world, a, 3, false),
        "0067B3B8 absent actor"
    );
    // Two actors prepended: removing the second-node one moves the tail back.
    let c = h.character();
    let d = h.character();
    let manager = &h.manager().manager;
    assert_eq!(manager.low_18.list.items(), [d, c], "prepend order");
    assert_ne!(
        manager.low_18.tail_08,
        ActorListNode::HEAD,
        "0067B307 tail advanced"
    );
    assert!(actor_process_manager_remove(&mut h.world, c, 3, false));
    let manager = &h.manager().manager;
    assert_eq!(
        manager.low_18.list.items(),
        [d],
        "0067B3C2 later node removed"
    );
    assert_eq!(
        manager.low_18.tail_08,
        ActorListNode::HEAD,
        "the tail moves back"
    );
    // The High list with two items clears 00B3BD74 on a remove.
    {
        let mut globals = h.world.resource_mut::<ActorProcessGlobals>();
        let manager = &mut globals.manager;
        manager.high_68.list.prepend_non_null(c);
        manager.high_68.list.prepend_non_null(d);
        manager.cursor_74 = Some(ActorListCursor {
            list: ActorListId::Low0c,
            node: ActorListNode::HEAD,
        });
    }
    actor_process_manager_remove(&mut h.world, a, 0, false);
    assert_eq!(
        h.manager().manager.cursor_74,
        None,
        "0067B34E two High items reset 00B3BD74"
    );
    // Flag 0x20 releases the process unless a save or load is active (0067B403).
    h.world.get_mut::<FormIdentity>(d).unwrap().flags_08 |= 0x20;
    assert!(actor_process_manager_remove(&mut h.world, d, 3, true));
    assert!(h.world.get::<MobileObject>(d).unwrap().process_58.is_some());
    actor_process_manager_add(&mut h.world, d, 3, true, false).unwrap();
    assert_eq!(
        h.manager().manager.low_18.list.items(),
        [d],
        "0067B2A7 append"
    );
    assert!(actor_process_manager_remove(&mut h.world, d, 3, false));
    assert!(
        h.world.get::<MobileObject>(d).unwrap().process_58.is_none(),
        "00659BC0"
    );
    assert_eq!(
        actor_process_level(&h.world, d),
        -1,
        "00659A00 without a process"
    );
}

#[test]
fn low_process_gets_no_controller() {
    let mut h = harness();
    let character = h.character();
    // A LowProcess (level 3): 0065B750 returns without a controller.
    actor_create_character_controller(&mut h.world, character, ControllerInputs::default())
        .unwrap();
    let globals = h.world.resource::<CharacterControllerGlobals>();
    assert_eq!(
        globals.collision_group_b2eb3c, 10,
        "0065B7B7 level 3: no controller"
    );
    assert!(!globals.state_manager_ba7a54);
    assert_eq!(actor_character_controller(&h.world, character), None);
}

#[test]
fn random_next_seeds() {
    let mut globals = RandomGlobals::default();
    let mut source = ScriptedRandom {
        time64: 0x1_2345_6789,
        rand: 7,
        ..ScriptedRandom::default()
    };
    assert_eq!(random_next(0, &mut globals, &mut source), 7);
    assert_eq!(
        source.seeds,
        [0x2345_6789],
        "0047DFA6 first use seeds with the low _time64 dword"
    );
    assert_eq!(globals.first_use_b069c3, 0);
    assert_eq!(random_next(0, &mut globals, &mut source), 7);
    assert_eq!(source.seeds.len(), 1, "0047DF9D seeded once");
    assert_eq!(random_next(5, &mut globals, &mut source), 7);
    assert_eq!(source.seeds.last(), Some(&5), "0047DF89 explicit seed");
    // The MSVC source: P11's draw fits a seed 11 seconds before the recorded _time64 (a fit, not a record).
    let inputs = run_inputs(&Rows::load());
    let mut time = || i64::from(inputs.time64) - 11;
    let mut source = CrtRandomSource {
        rand: MsvcRand::default(),
        time64: &mut time,
    };
    assert_eq!(
        random_next(0, &mut RandomGlobals::default(), &mut source),
        inputs.rand
    );
}

#[test]
fn actor_values_copy_and_modifiers() {
    let mut values = construct_actor_values();
    assert_eq!(
        values.magicka_08,
        Some(ActorValueEntry {
            code_00: 9,
            value_04: 0.0
        })
    );
    assert_eq!(
        values.fatigue_0c,
        Some(ActorValueEntry {
            code_00: 10,
            value_04: 0.0
        })
    );
    let mut source = construct_actor_values();
    source.magicka_08.as_mut().unwrap().value_04 = 3.0;
    source.fatigue_0c.as_mut().unwrap().value_04 = 4.0;
    recomp_game::tes::actor_process::copy_actor_values(&mut values, Some(&source)).unwrap();
    assert_eq!(values.magicka_08.unwrap().value_04, 3.0);
    assert_eq!(values.fatigue_0c.unwrap().value_04, 4.0);
    recomp_game::tes::actor_process::copy_actor_values(&mut values, None).unwrap();
    assert_eq!(
        values.magicka_08.unwrap().value_04,
        3.0,
        "0065CD1D a null source returns"
    );
    source.list_00.push(ActorValueEntry {
        code_00: 20,
        value_04: 1.0,
    });
    assert!(
        recomp_game::tes::actor_process::copy_actor_values(&mut values, Some(&source)).is_err()
    );
}

#[test]
fn particle_fov_scale_at_other_angles() {
    let mut globals = PlayerGlobals::default();
    particle_update_fov_scale(75.0, &mut globals);
    assert_eq!(
        globals.particle_fov_scale_b2d80c, 1.0,
        "the 37.5-degree reference"
    );
    particle_update_fov_scale(90.0, &mut globals);
    assert!((globals.particle_fov_scale_b2d80c - 1.303_225_4).abs() < 1e-5);
}

// IDA: 0066AB6C: the constructor stops on a null g_worldSceneGraph.
#[test]
fn null_world_scene_graph_stops() {
    let mut h = harness();
    let result = h.with_player_inputs(|world, inputs| {
        inputs.world_scene_graph = None;
        construct_player_character(world, inputs)
    });
    assert!(result.is_err());
}
