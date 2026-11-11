//! 00406360 Startup_InitializeGameData (WinMain 0040EB2C), one function per block, in original order:
//! - Phase 08: the sky prelude 00406390..004065FF (`run_sky_prelude`).
//! - Phase 09: the TES block 00406604..00406687 (`run_tes_block`), which takes this block's `SkyPreludeResult`.
//! - Phase 11: the player block 00406774..004067B9 (`run_player_window`).
//! - Phase 12: the 004067C7 call (`run_load_selected_files`), after the player block. Phase 10's plugin selection comes before that block.
//! - Phase 13: `run_tes_preload` 004067CC..004067D2, `run_player_base` 004067D7..00406861, `run_player_init` 00406866..004068CE and `run_scripts_sound_and_clamp` 004068D1..00406944, the end of the function.
//!
//! The Sky, its parts and the shadow light are in `crate::tes::sky`, and the shader prepare is in `crate::bs::sky_shader`.
//! Evidence: evidence/phase08-scope-2026-10-06, evidence/phase08-window-2026-10-06, P08-RUN-001, evidence/phase09-scope-2026-10-06, P09-RUN-001.

use crate::bs::character_controller::CharacterControllerGlobals;
use crate::bs::scene_graphs::{
    AttachChildStatus, SceneGraphGlobals, ShadowSceneNode, attach_scene_child,
};
use crate::bs::scene_lighting::set_shadow_scene_sun_light;
use crate::bs::sky_shader::{
    PrepareGeometryContext, SKY_SHADER_SLOT_INDEX, prepare_object_geometry,
};
use crate::bs::tree_manager::reset_tree_manager;
use crate::gamebryo::av_object::{find_property_of_type, refresh_and_update};
use crate::gamebryo::properties::SkyShaderProperty;
use crate::gamebryo::scene_derived::attach_affected_node;
use crate::gamebryo::scene_node::NiNodeChildren;
use crate::gamebryo::scene_state::ScenePropertyDefaults;
use crate::gamebryo::scene_update::SceneUpdateRuntime;
use crate::gamebryo::stream_objects::NiClass;
use crate::tes::actor_process::ActorProcessGlobals;
use crate::tes::character_controller::actor_set_position;
use crate::tes::data_handler::{DataHandler, init_scripts_and_cells};
use crate::tes::data_load::{DataLoadGlobals, DataLoadServices, LoadSelectedFiles};
use crate::tes::form::set_form_id_release;
use crate::tes::form_hierarchy::RttiClass;
use crate::tes::forms_init::{cast, init_item_of, lookup};
use crate::tes::forms_magic::{EffectSettingCollection, construct_effect_setting_collection};
use crate::tes::forms_references::{set_reference_base_form, set_reference_rotation};
use crate::tes::game_settings::GameSettingCollection;
use crate::tes::os_globals::{OsGlobalsData, SoundSourceGlobal, set_global_sound_source};
use crate::tes::player::{
    PLAYER_FORM_ID, PlayerGlobals, PlayerInputs, construct_player_character, player_get_actor_value,
};
use crate::tes::settings::SettingCollection;
use crate::tes::sky::{SkyGlobals, SkyInitServices, get_sky_singleton, initialize_sky};
use crate::tes::tes::{
    TesConstructionInputs, TesGlobals, TesPreloadServices, construct_tes, preload_default_assets,
};
use crate::tes::water::{WaterServices, initialize_water};
use crate::tes::window::TimeInfoState;
use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;
use oblivion_core::x87::stored_float;

/// IDA: N/A inputs of the 00406390..004065FF prelude.
pub struct SkyPreludeInputs<'a, 'b> {
    pub scenes: &'a SceneGraphGlobals,
    pub init: &'a mut SkyInitServices<'b>,
    pub prepare: &'a mut PrepareGeometryContext<'b>,
    pub property_defaults: ScenePropertyDefaults,
    pub update: &'a mut dyn SceneUpdateRuntime,
    /// 00404EE0 Startup_LogInfo (host logging; the Phase 04 / 05 disposition).
    pub log: &'a mut dyn FnMut(&'static str),
}

/// IDA: the prelude's outputs for 0040662D TES_Constructor: ebp = g_objectLODRoot (scene root), esi = the land-LOD node (child 0's vtable +8 GetAsNiNode when
/// g_lodRoot has a nonnull child 0, else 0), and [00B333B0], which is `SkyGlobals`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SkyPreludeResult {
    pub scene_root: Option<Entity>,
    pub land_lod: Option<Entity>,
    pub lod_branch: bool,
}

// IDA: 0040647F..004065A3's NiDynamicCast / IsKindOf 00B4335C of an object's type-4 property. A null object gives null. Completion: 100% for the bound classes.
fn sky_property_of(world: &World, object: Option<Entity>) -> Result<Option<Entity>, GameError> {
    let Some(object) = object else {
        return Ok(None);
    };
    Ok(find_property_of_type(world, object, 4)?
        .filter(|&property| world.get::<SkyShaderProperty>(property).is_some()))
}

// IDA: N/A the +0x88 store of one pass kind.
fn set_pass_kind(
    world: &mut World,
    object: Option<Entity>,
    kind: u32,
) -> Result<Option<Entity>, GameError> {
    let property = sky_property_of(world, object)?;
    if let Some(property) = property {
        world
            .get_mut::<SkyShaderProperty>(property)
            .expect("just checked")
            .pass_kind_88 = kind;
    }
    Ok(property)
}

// IDA: 00406360 Startup_InitializeGameData prefix 00406390..004065FF. WinMain's 0040EB1D..0040EB2C pushes g_worldSceneGraph and sets ecx = g_osGlobals, saved at
// 00406387 for later blocks; this block reads neither. Completion: 100% of the order of the reached branches (both g_lodRoot arms ported).
pub fn run_sky_prelude(
    world: &mut World,
    inputs: SkyPreludeInputs<'_, '_>,
) -> Result<SkyPreludeResult, GameError> {
    world.init_resource::<SkyGlobals>();
    world.resource_scope(|world, mut sky_globals: Mut<SkyGlobals>| {
        run_sky_prelude_with(world, &mut sky_globals, inputs)
    })
}

// IDA: N/A `run_sky_prelude` with `SkyGlobals` held out of the world.
fn run_sky_prelude_with(
    world: &mut World,
    sky_globals: &mut SkyGlobals,
    inputs: SkyPreludeInputs<'_, '_>,
) -> Result<SkyPreludeResult, GameError> {
    let SkyPreludeInputs {
        scenes,
        init,
        prepare,
        property_defaults,
        update,
        log,
    } = inputs;
    let mut result = SkyPreludeResult::default();
    // 0040638B..00406390 Startup_LogInfo("Initializing Sky...").
    log("Initializing Sky...");
    // 00406398..004063AD [00B333B0] = Sky_GetSingleton().
    let settings = world
        .get_resource::<GameSettingCollection>()
        .ok_or(GameError::unclassified(
            0x00541165,
            "00541165: no GameSettingCollection",
        ))?;
    get_sky_singleton(sky_globals, settings)?;
    sky_globals.published_b333b0 = true;
    sky_globals.trace(0x00B3_33B0, 4);
    // 004063B2 005411D0(sky, g_skyNode, g_fogProperty).
    initialize_sky(world, sky_globals, scenes.sky, scenes.fog, init, prepare)?;
    let sky = sky_globals
        .singleton_b365c4
        .as_deref()
        .expect("the singleton was just constructed");
    let sun = sky.sun_28.as_ref();
    // 004063B7..004063D4 g_weatherNode vtable +0x84 AttachChild(sun +0x0C, 0).
    let (Some(weather), Some(sun)) = (scenes.weather, sun) else {
        return Err(GameError::unclassified(
            0x004063D4,
            "004063D4: null g_weatherNode or sun",
        ));
    };
    if attach_scene_child(world, weather, sun.glare_billboard_0c, false)
        != AttachChildStatus::Attached
    {
        return Err(GameError::unclassified(0x004063D4, "004063D4"));
    }
    // 004063D6..004063EA 00708E40(005411C0 Sky_GetSunDirectionalLight, g_objectLODRoot).
    result.scene_root = scenes.object_lod_root;
    let (Some(sun_light), Some(object_lod_root)) = (sun.light_1c, scenes.object_lod_root) else {
        return Err(GameError::unclassified(
            0x004063EA,
            "004063EA: null sun light or g_objectLODRoot",
        ));
    };
    attach_affected_node(world, sun_light, object_lod_root);
    // 004063EF..00406406 007C5850(007B4280 Renderer_GetShaderResourceByIndex(0), light).
    let slot0 = scenes.shadow_slots[0].and_then(|slot| world.get_mut::<ShadowSceneNode>(slot));
    let Some(mut slot0) = slot0 else {
        return Err(GameError::unclassified(
            0x00406406,
            "00406406: shadow scene slot 0 is not a ShadowSceneNode",
        ));
    };
    set_shadow_scene_sun_light(&mut slot0, sun_light);
    // 0040640B..00406458 g_skyNode: 00707610, 00707980, NiAVObject_Update(0, 1).
    let sky_node = scenes.sky.ok_or(GameError::unclassified(
        0x0040640B,
        "0040640B: null g_skyNode",
    ))?;
    refresh_and_update(world, sky_node, &property_defaults, update, "0040642F")?;
    // 00406434..00406458 the same on g_weatherNode.
    refresh_and_update(world, weather, &property_defaults, update, "00406458")?;
    // 0040645D..0040647A 007B8940(g_skyNode, 10, 0, 1), then g_weatherNode.
    prepare_object_geometry(world, Some(sky_node), SKY_SHADER_SLOT_INDEX, 0, 1, prepare)?;
    prepare_object_geometry(world, Some(weather), SKY_SHADER_SLOT_INDEX, 0, 1, prepare)?;
    // 0040647F..004065A3 the SkyShaderProperty (type 4, NiDynamicCast / IsKindOf 00B4335C) +0x88 pass kinds.
    let (Some(atmosphere), Some(clouds)) = (sky.atmosphere_20.as_ref(), sky.clouds_2c.as_ref())
    else {
        return Err(GameError::unclassified(
            0x0040647F,
            "0040647F: null atmosphere or clouds",
        ));
    };
    // 0040648A atmosphere +0x08 (00452A60) -> 2.
    set_pass_kind(world, atmosphere.mesh_08, 2)?;
    // 004064C1 atmosphere +0x14 (0095F870) -> 4.
    set_pass_kind(world, atmosphere.quad_14, 4)?;
    // 004064F8 sun +0x10 -> 0; 00406526 sun +0x14 -> 1.
    set_pass_kind(world, sun.geometry_10, 0)?;
    set_pass_kind(world, sun.glare_geometry_14, 1)?;
    // 00406550..004065B3 clouds +0x08 + 4 * i (i = 0, 1; (i + 1) % 3 < 2): vtable +0x04 (007C5260) RTTI walk to 00B4335C, then +0x88 = 3 and +0x84 = (u16) i.
    let mut layer = 0;
    while layer < 2 {
        if let Some(property) = set_pass_kind(world, clouds.layers_08[layer], 3)? {
            world
                .get_mut::<SkyShaderProperty>(property)
                .expect("just checked")
                .layer_84 = layer as u16;
        }
        layer = (layer + 1) % 3;
    }
    // 004065B5..004065F8 the g_lodRoot branch.
    if let Some(lod_root) = scenes.lod_root {
        result.lod_branch = true;
        // 004065C8..004065CF 00708E40(005411C0 Sky_GetSunDirectionalLight, g_lodRoot).
        attach_affected_node(world, sun_light, lod_root);
        // 004065D4..004065F0: size +0xB6 nonzero and slot 0 nonnull -> its vtable +0x08 (GetAsNiNode: 007616D0 returns this for NiNode).
        let child = world
            .get::<NiNodeChildren>(lod_root)
            .and_then(|children| children.slots_ac.first().copied().flatten());
        result.land_lod = child.filter(|&child| {
            world
                .get::<NiClass>(child)
                .is_some_and(|class| class.is_node())
        });
    }
    // 004065FA..004065FF Startup_LogInfo("Initializing TES...").
    log("Initializing TES...");
    Ok(result)
}

/// IDA: N/A the inputs of the 00406604..00406687 block.
pub struct TesBlockInputs<'a, 'b> {
    /// 00B07BF0: the INI values and their write-backs.
    pub ini: &'a mut SettingCollection,
    pub construction: TesConstructionInputs<'a>,
    pub water: WaterServices<'a, 'b>,
    /// 00406360's g_worldSceneGraph argument.
    pub world_scene: Option<Entity>,
    /// 00707610.
    pub property_defaults: ScenePropertyDefaults,
    /// 00707370.
    pub update: &'a mut dyn SceneUpdateRuntime,
    /// 00404EE0 Startup_LogInfo (host logging; the Phase 04 / 05 disposition).
    pub log: &'a mut dyn FnMut(&'static str),
}

// IDA: 00406604..0040663F FormHeap_Allocate(0xAC), a null result skipping the constructor, 0040662D TES_Constructor and 0040663F 00B333A0 = eax. Returns whether the TES exists. Completion: 100%.
pub fn create_tes(
    world: &mut World,
    globals: &mut TesGlobals,
    ini: &mut SettingCollection,
    inputs: &mut TesConstructionInputs<'_>,
) -> Result<bool, GameError> {
    if inputs.fail_tes_allocation {
        globals.tes = None;
        globals.trace(0x00B3_33A0, 4);
        return Ok(false);
    }
    let tes = construct_tes(world, globals, ini, inputs)?;
    globals.tes = Some(Box::new(tes));
    globals.trace(0x00B3_33A0, 4);
    Ok(true)
}

// IDA: 00406360 Startup_InitializeGameData block 00406604..00406687: TES, 00406644 TES_InitializeWater, 00406654 0055F750(0), then 0040666C, 00406673, 00406682 on g_worldSceneGraph. Reads ebp and esi
// from the prelude's result and the Sky from `SkyGlobals`. Completion: 100% of the order.
pub fn run_tes_block(
    world: &mut World,
    prelude: &SkyPreludeResult,
    inputs: TesBlockInputs<'_, '_>,
) -> Result<(), GameError> {
    world.init_resource::<TesGlobals>();
    world.resource_scope(|world, mut globals: Mut<TesGlobals>| {
        let TesBlockInputs {
            ini,
            mut construction,
            mut water,
            world_scene,
            property_defaults,
            update,
            log,
        } = inputs;
        // 00406609..0040663F FormHeap(0xAC), TES_Constructor, g_tes.
        construction.scene_root = prelude.scene_root;
        construction.land_lod = prelude.land_lod;
        create_tes(world, &mut globals, ini, &mut construction)?;
        // 00406644 TES_InitializeWater(g_tes): a failed allocation leaves g_tes null and the original dereferences it.
        let mut tes = globals.tes.take().ok_or(GameError::unclassified(
            0x00406644,
            "00406644: TES_InitializeWater on a null g_tes",
        ))?;
        let initialized = initialize_water(world, &mut tes, ini, &mut water, update);
        globals.tes = Some(tes);
        initialized?;
        // 0040664E "Initializing TreeManager..."; 00406654 0055F750(0).
        log("Initializing TreeManager...");
        reset_tree_manager(world, false, ini, &mut globals.grid_changed_b06a28)?;
        // 0040665E "Initializing Menus...".
        log("Initializing Menus...");
        // 0040666C 00707610, 00406673 00707980, 00406682 00707370(0.0, 1) on the world scene graph.
        let world_scene = world_scene.ok_or(GameError::unclassified(
            0x0040666C,
            "0040666C: null g_worldSceneGraph",
        ))?;
        refresh_and_update(world, world_scene, &property_defaults, update, "00406682")
    })
}

/// IDA: N/A the inputs of the 00406687..00406773 block.
pub struct PluginBlockInputs<'a> {
    /// 00B07BF0: the sTestFile1..10:General values.
    pub ini: &'a SettingCollection,
    /// 00B3F178.
    pub paths: &'a crate::tes::paths::GamePaths,
    /// The host's report of the original's FileName overflow.
    pub warning: &'a mut dyn FnMut(String),
}

// IDA: 00406360 Startup_InitializeGameData block 00406687..00406773, after `run_tes_block` and before Phase 11's 00406774: see `plugin_selection::run_plugin_selection`. The block reads only the DataHandler that
// `run_tes_block` published. Nothing reads what it stores until 0044F3D0 (Phases 11 and 12), so the returned record is for the caller's log and the tests. Completion: 100% of the order.
pub fn run_plugin_block(
    world: &mut World,
    inputs: PluginBlockInputs<'_>,
) -> Result<crate::tes::winmain::plugin_selection::PluginSelection, GameError> {
    crate::tes::winmain::plugin_selection::run_plugin_selection(
        world,
        inputs.ini,
        inputs.paths,
        inputs.warning,
    )
}

/// IDA: N/A what the 00406774..004067B9 window produced. `player` is g_thePlayer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlayerWindowResult {
    pub player: Option<Entity>,
}

// IDA: 00406360 Startup_InitializeGameData block 00406774..004067B9: FormHeap_Allocate(0x804) (a null block skips the constructor, eax = 0), 00406793 PlayerCharacter_ctor, 004067A6 g_thePlayer = eax (before the form
// ID is set), 004067AB TESForm_SetFormID(0x14, 1), 004067B5 Startup_LogInfo("Loading Files...") (obsolete_runtime: host log, D8). 004067BA..004067C7 0044F3D0 is Phase 12. `fail_allocation` is the host seam of
// 0040677E. A null player reads [0 + 0xC] at 004067AB: a host stop. The caller provides the 00B3BD00 static (`construct_actor_process_manager`). Completion: 100% of the order.
pub fn run_player_window(
    world: &mut World,
    inputs: &mut PlayerInputs<'_>,
    fail_allocation: bool,
    log: &mut dyn FnMut(&'static str),
) -> Result<PlayerWindowResult, GameError> {
    if !world.contains_resource::<ActorProcessGlobals>() {
        return Err(GameError::unclassified(
            0x00B3BD00,
            "00B3BD00: g_actorProcessManager not constructed (cinit 009FE5F0 has no driver yet)",
        ));
    }
    world.init_resource::<PlayerGlobals>();
    world.init_resource::<CharacterControllerGlobals>();
    let player = if fail_allocation {
        None
    } else {
        Some(construct_player_character(world, inputs)?)
    };
    world.resource_mut::<PlayerGlobals>().the_player_b333c4 = player;
    let Some(player) = player else {
        return Err(GameError::unclassified(
            0x004067AB,
            "004067AB: TESForm_SetFormID on a null g_thePlayer (FormHeap allocation of the PlayerCharacter failed)",
        ));
    };
    set_form_id_release(world, player, PLAYER_FORM_ID, true)?;
    log("Loading Files...");
    Ok(PlayerWindowResult {
        player: Some(player),
    })
}

// IDA: N/A 00B06B18 bDisableWarning:MESSAGES, g_disableWarningMessages, from the INI collection (image value 1).
const DISABLE_WARNING_MESSAGES_B06B18: u32 = 0x00B0_6B18;

/// IDA: 00B09E28 fQuestScriptDelayTime:MAIN.
const QUEST_SCRIPT_DELAY_TIME_B09E28: u32 = 0x00B0_9E28;

/// IDA: 00406360 Startup_InitializeGameData block 004067BA..004067C7, set up: ecx = g_dataHandler, 0044F3D0(0, 0). The cursor lets a host draw between records; `run_load_selected_files` runs it straight through. Completion: 100% of the block.
pub fn begin_load_selected_files(
    world: &mut World,
    ini: &SettingCollection,
) -> Result<LoadSelectedFiles, GameError> {
    let flag = ini
        .find_address(DISABLE_WARNING_MESSAGES_B06B18)
        .ok_or(GameError::unclassified(
            0x00B06B18,
            "00B06B18: bDisableWarning:MESSAGES is not registered",
        ))?
        .raw;
    // These fields stand for INI settings the exe reads where it tests them, so each takes the live value.
    let byte_setting = |address: u32, error: &'static str| {
        ini.find_address(address)
            .map(|setting| setting.raw as u8)
            .ok_or(GameError::from_legacy_message(error))
    };
    world.insert_resource(DataLoadGlobals {
        disable_warning_messages_b06b18: flag as u8,
        skip_ai_packages_b05dc0: byte_setting(
            0x00B0_5DC0,
            "00B05DC0: bFixAIPackagesOnLoad:General is not registered",
        )?,
        essential_take_no_damage_b14e98: byte_setting(
            0x00B1_4E98,
            "00B14E98: bEssentialTakeNoDamage:GamePlay is not registered",
        )?,
        quest_flag_b10ca0: byte_setting(
            0x00B1_0CA0,
            "00B10CA0: bActivateAllQuestScripts:General is not registered",
        )?,
        ..DataLoadGlobals::default()
    });
    // 004FBCB2, 004FBD3D read fQuestScriptDelayTime:MAIN's live value. The default is its image value, for a world with no INI.
    world.insert_resource(crate::tes::forms_init::QuestDelayGlobals {
        quest_script_delay_time_b09e28: f32::from_bits(
            ini.find_address(QUEST_SCRIPT_DELAY_TIME_B09E28)
                .ok_or(GameError::unclassified(
                    0x00B09E28,
                    "00B09E28: fQuestScriptDelayTime:MAIN is not registered",
                ))?
                .raw,
        ),
        ..Default::default()
    });
    // 004EA450 reads 00B09B20, 00B09B28 and 00B09B30 live, for every worldspace this load constructs.
    let raw = |address: u32, name: &'static str| {
        ini.find_address(address)
            .map(|setting| setting.raw)
            .ok_or(GameError::from_legacy_message(name))
    };
    world.insert_resource(crate::tes::forms_world::TerrainLodSettings {
        min_grass_size_b09b20: raw(0x00B0_9B20, "00B09B20 is not registered")? as i32,
        b09b28: f32::from_bits(raw(0x00B0_9B28, "00B09B28 is not registered")?),
        b09b30: f32::from_bits(raw(0x00B0_9B30, "00B09B30 is not registered")?),
    });
    // 0043961E's a7 arm reads 00B05148, 00B05150 and 00B07280 live, for every fade node the model loader builds.
    world.insert_resource(crate::bs::fade_node::FadeNodeSettings {
        bound_radius_mult_b05148: f32::from_bits(raw(0x00B0_5148, "00B05148 is not registered")?),
        fade_out_percent_b05150: f32::from_bits(raw(0x00B0_5150, "00B05150 is not registered")?),
        min_near_distance_b07280: raw(0x00B0_7280, "00B07280 is not registered")? as i32,
        actor_lod_multiplier_b0762c: f32::from_bits(raw(
            0x00B0_762C,
            "00B0762C is not registered",
        )?),
    });
    // 005269C5 and 004788E0 read the FaceGen and helmet bytes live, for every actor task.
    world.insert_resource(crate::tes::character_3d::FaceGenSettings {
        use_multi_threaded_face_gen_b120dc: raw(0x00B1_20DC, "00B120DC is not registered")? as u8
            != 0,
        use_face_gen_heads_b120b4: raw(0x00B1_20B4, "00B120B4 is not registered")? as u8 != 0,
        load_helmets_in_background_b06608: raw(0x00B0_6608, "00B06608 is not registered")? as u8
            != 0,
    });
    // 00475D80 and 00476D10 read 00B06548 live.
    world.insert_resource(crate::tes::actor_animation::AnimationCloneSettings {
        animation_clone_per_loop_b06548: raw(0x00B0_6548, "00B06548 is not registered")?,
    });
    // 004B1210 reads the bLightAttenuation values live, for every light 004B1600 builds; the bits are 004B1160's at CRT init.
    world.insert_resource(crate::tes::light_3d::LightAttenuationSettings {
        bits_b35ac0: crate::tes::light_3d::crt_attenuation_bits(),
        linear_method_b08158: raw(0x00B0_8158, "00B08158 is not registered")?,
        quadratic_method_b08160: raw(0x00B0_8160, "00B08160 is not registered")?,
        linear_value_b08168: f32::from_bits(raw(0x00B0_8168, "00B08168 is not registered")?),
        quadratic_value_b08170: f32::from_bits(raw(0x00B0_8170, "00B08170 is not registered")?),
        constant_value_b08178: f32::from_bits(raw(0x00B0_8178, "00B08178 is not registered")?),
        out_quad_in_lin_b08180: raw(0x00B0_8180, "00B08180 is not registered")? as u8,
        linear_radius_mult_b08188: f32::from_bits(raw(0x00B0_8188, "00B08188 is not registered")?),
        quadratic_radius_mult_b08190: f32::from_bits(raw(
            0x00B0_8190,
            "00B08190 is not registered",
        )?),
    });
    // Cinit slot 151, before WinMain in the original: a recorded timing substitution, as nothing reads 00B33508 before this call.
    if !world.contains_resource::<EffectSettingCollection>() {
        construct_effect_setting_collection(world)?;
    }
    Ok(LoadSelectedFiles::new(false, false))
}

/// IDA: 00406360 Startup_InitializeGameData block 004067BA..004067C7, after `run_player_window`: 0044F3D0(0, 0). The caller ignores eax, as 004067CC does; a test reads it. `Err` is a host stop. Completion: 100% of the block; `crate::tes::data_load` has the function.
pub fn run_load_selected_files(
    world: &mut World,
    ini: &SettingCollection,
    services: &mut DataLoadServices<'_>,
) -> Result<u32, GameError> {
    begin_load_selected_files(world, ini)?.run(world, services)
}

// IDA: N/A 00404EC0's first argument: the same text with `%s` filled, for a host log.
fn player_check_text(name: &str, value: &str) -> String {
    format!("ERROR: {value} value is 0 on the Player. Fix '{name}' with the editor.")
}

/// IDA: 00406360 Startup_InitializeGameData 004067CC..004067D2: ecx = g_tes, 00443550. Completion: 100% of the block; `crate::tes::tes` has the function.
/// # Errors
/// A stop in 00443550.
pub fn run_tes_preload(
    world: &mut World,
    services: &mut TesPreloadServices<'_, '_>,
) -> Result<(), GameError> {
    preload_default_assets(world, services)
}

/// IDA: 00406360 004067D7..00406861, in original order: Startup_LogInfo("Initializing Player...") (the host log, D4); 0046B250(7) and ___RTDynamicCast from TESForm to TESNPC, null when absent or another class; on g_thePlayer, 006C5420 the +0x1C store, vt+0x1CC 0065A3E0 with the stack NiPoint3 {A2FF44, A2FF44, fldz} = {2048, 2048, 0}, and 004D89A0 with g_niPoint3Zero. Returns esi, which 0040688A and 004068B9 read again. Completion: 100%.
/// # Errors
/// A null g_thePlayer (00406802 passes it on and 006C5424 faults).
pub fn run_player_base(
    world: &mut World,
    log: &mut dyn FnMut(&'static str),
) -> Result<Option<Entity>, GameError> {
    run_player_base_observed(world, log, &mut |_, _| {})
}

/// IDA: N/A `run_player_base` with a call after each of its three stores, named as P13-RUN-001 dumps them: "after_base" at 00406813, "after_1cc" at 0040683C, "after_89a0" at 00406866. A test reads its rows there.
/// # Errors
/// As `run_player_base`.
pub fn run_player_base_observed(
    world: &mut World,
    log: &mut dyn FnMut(&'static str),
    after: &mut dyn FnMut(&World, &'static str),
) -> Result<Option<Entity>, GameError> {
    log("Initializing Player...");
    let base = cast(world, lookup(world, 7), RttiClass::TESNPC);
    let player = world
        .get_resource::<PlayerGlobals>()
        .and_then(|globals| globals.the_player_b333c4)
        .ok_or(GameError::unclassified(
            0x0040680E,
            "0040680E: 006C5420 on a null g_thePlayer",
        ))?;
    set_reference_base_form(world, player, base);
    after(world, "after_base");
    let start = f32::from_bits(PLAYER_START_XY_A2FF44);
    actor_set_position(world, player, [start, start, 0.0])?;
    after(world, "after_1cc");
    set_reference_rotation(world, player, [0.0; 3])?;
    after(world, "after_89a0");
    Ok(base)
}

/// IDA: 00A2FF44, 0x45000000: 2048.0, read from the exe's bytes.
pub const PLAYER_START_XY_A2FF44: u32 = 0x4500_0000;

/// IDA: N/A the two vt+0x284 values 00406873..004068CE read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlayerChecks {
    /// vt+0x284(10).
    pub fatigue: i32,
    /// vt+0x284(8).
    pub health: i32,
}

// IDA: 00406885..0040689F and 004068B4..004068CE: a zero value's 0046B680(0) on esi, +0x1C, and 00404EC0. 0046B680(0) walks +0x10 to the first nonzero item (`cmp edi, esi` then `jg` after one count). A null base or no file reads a bad address in the original.
fn check_player_value(
    world: &World,
    value: i32,
    base: Option<Entity>,
    what: &str,
    services: &mut DataLoadServices<'_>,
) -> Result<(), GameError> {
    if value != 0 {
        return Ok(());
    }
    let base = base.ok_or(GameError::unclassified(
        0x0040688C,
        "0040688C: 0046B680 on a null base form",
    ))?;
    let data = world.resource::<DataHandler>();
    let name = crate::tes::form::mod_file_at_index(world, base, 0)
        .and_then(|id| data.file_store.get(id))
        .map(|file| file.name_1c.clone())
        .ok_or(GameError::unclassified(
            0x00406891,
            "00406891: +0x1C of no mod file",
        ))?;
    services.host.debug_log(player_check_text(&name, what));
    Ok(())
}

/// IDA: 00406360 00406866..004068CE, in original order: g_thePlayer vt+0x6C (A73A0C's slot is 004DF820, the reference InitItem: no actor override), vt+0x284(10) with its zero check, vt+0x284(8) with its. `base` is `run_player_base`'s esi. Completion: 100%.
/// # Errors
/// A null g_thePlayer, or a stop below.
pub fn run_player_init(
    world: &mut World,
    base: Option<Entity>,
    services: &mut DataLoadServices<'_>,
) -> Result<PlayerChecks, GameError> {
    let player = world
        .get_resource::<PlayerGlobals>()
        .and_then(|globals| globals.the_player_b333c4)
        .ok_or(GameError::unclassified(
            0x00406866,
            "00406866: vt+0x6C on a null g_thePlayer",
        ))?;
    init_item_of(world, player, services)?;
    let fatigue = player_get_actor_value(world, player, 10)?;
    check_player_value(world, fatigue, base, "Fatigue", services)?;
    let health = player_get_actor_value(world, player, 8)?;
    check_player_value(world, health, base, "Health", services)?;
    Ok(PlayerChecks { fatigue, health })
}

/// IDA: N/A what 004068D1..00406944 reads and stores past the world.
pub struct ScriptsSoundInputs<'a, 'b> {
    pub services: &'a mut DataLoadServices<'b>,
    /// 00B07BF0: 00B02D10 iFPSClamp:General.
    pub ini: &'a SettingCollection,
    /// 00B33398 g_osGlobals.
    pub os_globals: Option<&'a OsGlobalsData>,
    /// 00B33E90 g_timeInfo, shared with the movie thread, so the caller holds its lock.
    pub time: &'a mut TimeInfoState,
    pub log: &'a mut dyn FnMut(&'static str),
}

/// IDA: 00406360 004068D1..00406944, in original order: Startup_LogInfo("Initializing Scripts..."), 00447D80 on g_dataHandler, Startup_LogInfo("Initializing Sound System..."), 006AF850(0), the inline FPS clamp, and the return. Returns whether the clamp stored. Completion: 100%.
/// # Errors
/// A stop in 00447D80, or a null g_osGlobals (006AF863 faults).
pub fn run_scripts_sound_and_clamp(
    world: &mut World,
    inputs: ScriptsSoundInputs<'_, '_>,
) -> Result<bool, GameError> {
    let ScriptsSoundInputs {
        services,
        ini,
        os_globals,
        time,
        log,
    } = inputs;
    log("Initializing Scripts...");
    init_scripts_and_cells(world, services)?;
    log("Initializing Sound System...");
    let mut slot = world.get_resource_or_init::<SoundSourceGlobal>();
    set_global_sound_source(&mut slot, None, os_globals).map_err(|_| {
        GameError::unclassified(0x006AF863, "006AF863: 006AF850(0) with a null g_osGlobals")
    })?;
    let setting = ini
        .find_address(FPS_CLAMP_B02D10)
        .ok_or(GameError::unclassified(
            0x00B02D10,
            "00B02D10: iFPSClamp:General is not registered",
        ))?
        .raw as i32;
    Ok(apply_fps_clamp(time, setting))
}

// IDA: N/A 00B02D10 iFPSClamp:General (image 0).
const FPS_CLAMP_B02D10: u32 = 0x00B0_2D10;

/// IDA: 004068FA..0040692B the inline FPS clamp of 00406360: n = iFPSClamp:General (00B02D10); `lea ecx, [n - 1]; cmp ecx,
/// 0xC6; ja` keeps 1..199 only (0 and negatives wrap above 0xC6, so the `fldz` arm is unreachable), then fild n (the `fadd
/// 2^32` fix-up is dead: n > 0), fdivr 1000.0 (dbl_A2FC70) and fstp into TimeInfo +0x04 at 24-bit x87 precision: the
/// correctly rounded float of 1000.0 / n. Returns whether the store ran. Completion: 100%.
pub fn apply_fps_clamp(time: &mut TimeInfoState, fps_clamp_setting: i32) -> bool {
    let n = fps_clamp_setting as u32;
    if n.wrapping_sub(1) > 0xC6 {
        return false;
    }
    // 24-bit fdivr of the exact operands, stored as float: the division rounded once to float. The f64 quotient rounds
    // again; with 53 >= 2 * 24 + 2 bits the double rounding is innocuous for division.
    time.fps_clamp_04 = stored_float(1000.0 / f64::from(n));
    true
}
