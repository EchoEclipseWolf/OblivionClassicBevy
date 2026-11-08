//! Phase 07: WinMain 0040EB16..0040EB2C, `00578CC0 InterfaceManager_GetSingleton_w(1)` -> 00582160
//! InterfaceManager_GetSingleton(1, 1): FormHeap(0x134), 005802B0 InterfaceManager_ctor (menu scene graphs,
//! FontManager, look-at
//! helper), the 00B3A6E0 publish **before** 00581CC0 InterfaceManager_Initialize (Menus Root, strings.xml, DebugText
//! root, fog, cursor) and, with the flag 1, 00578E10 -> 005ADCF0 LoadingMenu_Create.
//!
//! The MoviePlayer thread reads 00B3A6E0 and the TileMenu table in parallel (P06-RUN-001, P07-RUN-001: no lock in the
//! original). `InterfaceMovieReads` is all it holds: presence atomics, as 00410390 only tests for null (the plan's
//! "Worker
//! threads cannot hold World", the handoff's D6). The C++ loads are acquire loads too; the original's are plain reads.
//! The objects themselves are main-thread resources: `InterfaceManager` is present exactly when 00B3A6E0 is non-null.
//!
//! Main-thread x87 after CreateDevice: 24-bit precision, so `f32` arithmetic with the original grouping. D3: scene
//! state ported, tile geometry and the 0057F170 render are host substitutions behind `InterfaceHost::render_interface`.
//! Evidence: evidence/phase07-window-2026-10-06 (exports, run001-records.txt).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use bevy_ecs::prelude::*;
use recomp_core::reader::StreamError;
use recomp_core::x87;

use super::fonts::font_manager_get_singleton;
use super::interface_scene::{TileSceneRecord, interface_update_tile_scene};
use super::loading_menu::{
    construct_loading_menu, loading_menu_advance_slide, loading_menu_build_slide_list,
};
use super::settings::SettingCollection;
use super::tile::{
    InterfaceCx, InterfaceHost, MenuClass, MenuId, TileId, TileKind, with_interface_cx,
};
use super::tile_values::*;
use super::tile_xml::{tile_read_xml, tile_xml_clear_cached_file};
use crate::bs::scene_graphs::{
    AttachChildStatus, SceneCamera, SceneGraph, SceneGraphGlobals, SceneGraphSettings,
    attach_affected_node, attach_scene_child, attach_scene_property, construct_scene_graph,
    scene_graph_set_camera_fov, set_object_name,
};
use crate::gamebryo::object_net::{AlphaProperty, NiObjectNet};
use crate::gamebryo::properties::{FogProperty, VertexColorProperty, ZBufferProperty};
use crate::gamebryo::scene_derived::{DirectionalLight, spawn_directional_light};
use crate::gamebryo::scene_node::{SceneNodePayload, spawn_scene_node};
use crate::gamebryo::scene_update::{
    SceneUpdateResult, SceneUpdateRuntime, SceneUpdateStatus, update_scene_object,
};
use crate::gamebryo::stream_objects::NiClass;

/// IDA: 00587B20's array size, the host's fixed slots (the C++ `kTileMenuSlots`).
pub const TILE_MENU_SLOTS: usize = 0x33;
/// IDA: 00589B70's first menu ID: `lea ecx, [eax-3E9h]`.
pub const FIRST_TILE_MENU_ID: u32 = 0x3E9;
/// IDA: 0041042B MenuID_Message.
pub const MENU_ID_MESSAGE: u32 = 1001;

struct InterfaceReads {
    /// 00B3A6E0 != null.
    manager_b3a6e0: AtomicBool,
    /// The table 00589B70 reads: whether each slot is non-null.
    tile_menus: [AtomicBool; TILE_MENU_SLOTS],
}

/// IDA: N/A what another thread may read of 00B3A6E0 and the TileMenu table: presence only, since 00410390 only tests
/// for
/// null. The main thread owns the objects themselves. A clone names the same state. Completion: 100% of those reads.
#[derive(Resource, Clone)]
pub struct InterfaceMovieReads(Arc<InterfaceReads>);

impl Default for InterfaceMovieReads {
    /// IDA: N/A host helper. Completion: 100%.
    fn default() -> Self {
        Self(Arc::new(InterfaceReads {
            manager_b3a6e0: AtomicBool::new(false),
            tile_menus: std::array::from_fn(|_| AtomicBool::new(false)),
        }))
    }
}

impl InterfaceMovieReads {
    /// IDA: 005821C7 the store of 00B3A6E0 (Phase 07), and a host reset. Completion: 100% of the store.
    pub fn publish_interface_manager(&self, present: bool) {
        self.0.manager_b3a6e0.store(present, Ordering::Release);
    }

    /// IDA: 00582160 InterfaceManager_GetSingleton(0, 1), as 00410410 calls it: it never creates, so a plain read of
    /// 00B3A6E0. Completion: 100% of the non-creating call.
    #[must_use]
    pub fn interface_manager_present(&self) -> bool {
        self.0.manager_b3a6e0.load(Ordering::Acquire)
    }

    /// IDA: N/A a store into the TileMenu table (Phase 07's menus). An ID outside the table has no slot. Completion: 100%.
    pub fn set_tile_menu(&self, menu_id: u32, present: bool) {
        let slot = menu_id.wrapping_sub(FIRST_TILE_MENU_ID) as usize;
        if let Some(entry) = self.0.tile_menus.get(slot) {
            entry.store(present, Ordering::Release);
        }
    }

    /// IDA: 00589B70 TileMenu_GetByMenuID: `cmp ecx, 32h; ja` null, then `cmp` with the array's size word 00B1397A;
    /// `jbe`
    /// null. Completion: 100% with one host value: the array has its fixed 0x33 slots from the start, where the
    /// original's
    /// size word is 0 until 00587B20 (Phase 07). Every slot is empty until then, so the answer is the same.
    #[must_use]
    pub fn tile_menu_present(&self, menu_id: u32) -> bool {
        let slot = menu_id.wrapping_sub(FIRST_TILE_MENU_ID) as usize;
        self.0
            .tile_menus
            .get(slot)
            .is_some_and(|entry| entry.load(Ordering::Acquire))
    }
}

/// IDA: 00538B20 InterfaceLookAtHelper_ctor (0xC): +0 null, +4 byte 0, +8 0. Completion: 100%.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InterfaceLookAtHelper {
    pub unknown_0: u32,
    pub unknown_4: u8,
    pub unknown_8: u32,
}

/// IDA: 005802B0 the 0x14 block at +0x130: +0 0, +4 0.0, +8 0.001 (A37080), +0xC itself, +0x10 0. `self_c` stands for
/// the +0xC store, a pointer to the block. Completion: 100%.
#[derive(Clone, Debug, PartialEq)]
pub struct InterfaceBlock130 {
    pub unknown_0: u32,
    pub unknown_4: f32,
    pub unknown_8: f32,
    pub self_c: bool,
    pub unknown_10: u32,
    /// The list the block heads, in order (00584300 appends).
    pub nodes: Vec<super::interface_frame::TimedTransitionNode>,
    /// 00583F40's blend constant: `transitionProgress`, kept for the image-space reader no screen reaches.
    pub blend_progress: f32,
}

/// IDA: g_rendererUIState (005730B0 `this`, 00B3A6B0): +0 "FaderNode Below Menus", +4 "FaderNode Above Menus". These
/// are the two words `RendererUiState` keeps as `unknown_00` and `unknown_04`. Completion: 100%.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RendererUiFaderNodes {
    pub below_00: Option<Entity>,
    pub above_04: Option<Entity>,
}

/// IDA: N/A what the window reads outside the interface code, as plain values: g_rendererWidth and Height 00B06C4C and
/// 00B06C50, the safe zone settings 00B135F8 / 00B13600 and 00B13608 / 00B13610, OSInputGlobals' first byte,
/// bFullScreen 00B06C74,
/// sFontFile_1..5 00B12E1C.., [LoadingBar] 00B14138.. and the presence of 00B333C4 g_thePlayer, 00B3B33C HUDInfoMenu,
/// 00B33A98 g_dataHandler and g_osGlobals' sound object. Completion: 100% of the sources.
#[derive(Clone, Debug, PartialEq)]
pub struct InterfaceSettings {
    pub renderer_width: i32,
    pub renderer_height: i32,
    /// iSafeZoneX, iSafeZoneY, iSafeZoneXWide, iSafeZoneYWide.
    pub safe_zone: [i32; 4],
    pub input_flags: u8,
    pub full_screen: bool,
    pub font_paths: [Option<Vec<u8>>; 5],
    pub loading_bar_percentages: [f32; 4],
    pub loading_slide_delay: f32,
    /// iMoveBarMaxMilliseconds, iMoveBarChaseMilliseconds and iMoveBarWaitingMilliseconds:LoadingBar (00B14160, 00B14168,
    /// 00B14170): signed dwords that 005AD980 compares with `jle` and `jg`.
    pub loading_move_bar_ms: [i32; 3],
    /// bCrossHair:GamePlay (00B13210), which 005A8769 reads as a byte.
    pub cross_hair_b13210: bool,
    /// bDialogueSubtitles, bGeneralSubtitles, bSaveOnRest, bSaveOnWait and bSaveOnTravel:GamePlay (00B13200, 00B13208,
    /// 00B13218, 00B13220, 00B13228), which 005A3540 reads as bytes. Taken when the interface starts: nothing ported writes
    /// them later (the Gameplay menu's click 005A3860 would).
    pub dialogue_subtitles_b13200: bool,
    pub general_subtitles_b13208: bool,
    pub save_on_rest_b13218: bool,
    pub save_on_wait_b13220: bool,
    pub save_on_travel_b13228: bool,
    /// fMouseSensitivity and bInvertYValues:Controls (00B14EE8, 00B14F38, the latter read as a byte by 0059BBAD), taken
    /// when the interface starts, as the GamePlay flags are.
    pub mouse_sensitivity_b14ee8: f32,
    pub invert_y_values_b14f38: bool,
    pub scene: SceneGraphSettings,
    pub player_present: bool,
    pub hud_info_menu_present: bool,
    pub data_handler_present: bool,
    pub sound_present: bool,
}

/// IDA: N/A reads `InterfaceSettings` from the 03-E INI collection by address; the renderer and input values arrive as
/// plain values, as in `scene_graph_settings_from`. Completion: 100% of the addresses.
#[must_use]
pub fn interface_settings_from(
    ini: &SettingCollection,
    renderer_width: i32,
    renderer_height: i32,
    input_flags: u8,
    scene: SceneGraphSettings,
) -> InterfaceSettings {
    let raw = |address| ini.find_address(address).map_or(0, |setting| setting.raw);
    let float = |address| f32::from_bits(raw(address));
    let path = |address| {
        ini.find_address(address)
            .and_then(|setting| setting.text.clone())
    };
    InterfaceSettings {
        renderer_width,
        renderer_height,
        safe_zone: [
            raw(0x00B1_35F8) as i32,
            raw(0x00B1_3600) as i32,
            raw(0x00B1_3608) as i32,
            raw(0x00B1_3610) as i32,
        ],
        input_flags,
        full_screen: raw(0x00B0_6C74) != 0,
        font_paths: [
            path(0x00B1_2E1C),
            path(0x00B1_2E24),
            path(0x00B1_2E2C),
            path(0x00B1_2E34),
            path(0x00B1_2E3C),
        ],
        loading_bar_percentages: [
            float(0x00B1_4138),
            float(0x00B1_4140),
            float(0x00B1_4148),
            float(0x00B1_4150),
        ],
        loading_slide_delay: float(0x00B1_4158),
        loading_move_bar_ms: [
            raw(0x00B1_4160) as i32,
            raw(0x00B1_4168) as i32,
            raw(0x00B1_4170) as i32,
        ],
        cross_hair_b13210: raw(0x00B1_3210) & 0xFF != 0,
        dialogue_subtitles_b13200: raw(0x00B1_3200) & 0xFF != 0,
        general_subtitles_b13208: raw(0x00B1_3208) & 0xFF != 0,
        save_on_rest_b13218: raw(0x00B1_3218) & 0xFF != 0,
        save_on_wait_b13220: raw(0x00B1_3220) & 0xFF != 0,
        save_on_travel_b13228: raw(0x00B1_3228) & 0xFF != 0,
        mouse_sensitivity_b14ee8: float(0x00B1_4EE8),
        invert_y_values_b14f38: raw(0x00B1_4F38) & 0xFF != 0,
        scene,
        player_present: false,
        hud_info_menu_present: false,
        data_handler_present: false,
        sound_present: true,
    }
}

/// IDA: InterfaceManager (0x134; xOBSE GameAPI.h). A resource: present when 00B3A6E0 is non-null, so
/// `world.get_resource::<InterfaceManager>()` is 00582160(0, 1). Fields the constructor leaves unwritten keep FormHeap
/// residue in the original (P07-RUN-001: +0x0E, +0x0F,
/// +0x14, +0x20..+0x34, +0x40..+0x4C, +0x5C and +0x60 written, +0x64, +0x7C, +0x90, +0xA9..+0xAF, +0xB1..+0xB3, +0xBA,
/// +0xBB, +0xDD..+0xDF, +0x115..+0x117, +0x124, +0x12C) and are 0 on the host. Completion: 100% of the fields the
/// window writes.
#[derive(Resource, Clone, Debug, PartialEq)]
pub struct InterfaceManager {
    /// +0x000 NiPointer ("MenuRoot").
    pub menu_scene_graph_000: Option<Entity>,
    /// +0x004 NiPointer ("Menu3DRoot").
    pub menu_3d_scene_graph_004: Option<Entity>,
    pub menu_loading_status_008: u8,
    pub menu_transition_in_progress_009: u8,
    /// +0x00A..+0x00D.
    pub tab_ids_00a: [u8; 4],
    pub unknown_010: u32,
    pub light_018: Option<Entity>,
    pub cursor_01c: Option<TileId>,
    /// +0x020..+0x034 (0057E7C0).
    pub cursor_state_020: [f32; 6],
    pub unknown_038: f32,
    pub mouse_held_timer_03c: f32,
    pub debug_text_on_050: u8,
    /// +0x054 "InterfaceManager: Main Root".
    pub main_root_054: Option<Entity>,
    /// +0x058 "InterfaceManager: Cursor Root".
    pub cursor_root_058: Option<Entity>,
    pub unknown_05c: u32,
    /// +0x060 NiNode, 0057E585: the player's view in the Stats tab. Not written before 0057E510.
    pub player_scene_node_060: Option<Entity>,
    /// +0x064 ShadowSceneNode, 0057E5CB. Not written before 0057E510: the exe's heap bytes, 0 here.
    pub player_shadow_scene_064: Option<Entity>,
    /// +0x068 TileRect "MenuRoot".
    pub menu_root_068: Option<TileId>,
    /// +0x06C strings.xml root.
    pub strings_06c: Option<TileId>,
    pub debug_text_root_070: Option<Entity>,
    pub camera_distance_074: f32,
    /// +0x078 NiPointer.
    pub alpha_property_078: Option<Entity>,
    /// +0x07C (vt+8 builders 1, render 0).
    pub geometry_changed_07c: u8,
    pub unknown_07e: u16,
    pub hud_reticule_080: Option<TileId>,
    pub unknown_084: u32,
    pub alt_active_tile_088: Option<TileId>,
    pub default_selection_priority_08c: u32,
    pub big_four_rebuild_pending_094: u8,
    pub active_tile_098: Option<TileId>,
    pub active_menu_09c: Option<MenuId>,
    pub unknown_0a0: u32,
    pub unknown_0a4: u32,
    pub unknown_0a8: u8,
    pub msg_box_button_pressed_0b0: u8,
    /// +0xB4: 005BC4A0 stores a closed MessageMenu's callback; 005821F0's end runs it once.
    pub message_callback_0b4: Option<super::menus::MessageCallback>,
    /// +0x118, bit 4 shift (0057F9D9 for scan codes 2A and 36; 005AEFB7 and 00583644 read it).
    pub input_modifier_flags: u32,
    /// N/A g_timeInfo.frameTime at this frame's 00583E60, for the fades that start from a click (00584390, 00584740): their 00583C30 reads the global.
    pub frame_seconds_host: f32,
    /// flt_B3A6E8, flt_B3A6F0: 0057F490's last point.
    pub cursor_previous_b3a6e8: [f32; 2],
    pub purge_textures_0b8: u8,
    pub cursor_moved_0b9: u8,
    /// +0x0BC..+0x0CC.
    pub references_0bc: [u32; 5],
    /// +0x0D0..+0x0D8.
    pub unknown_0d0: [f32; 3],
    pub unknown_0dc: u8,
    /// +0x0E0..+0x104.
    pub menu_mode_stack_0e0: [u32; 10],
    pub look_at_108: Option<InterfaceLookAtHelper>,
    pub unknown_10c: u32,
    pub unknown_110: u32,
    pub unknown_114: u8,
    /// +0x118..+0x120.
    pub unknown_118: [u32; 3],
    pub unknown_128: u32,
    pub block_130: Option<InterfaceBlock130>,
}

impl Default for InterfaceManager {
    /// IDA: N/A host helper. Completion: 100%.
    fn default() -> Self {
        Self {
            menu_scene_graph_000: None,
            menu_3d_scene_graph_004: None,
            menu_loading_status_008: 1,
            menu_transition_in_progress_009: 0,
            tab_ids_00a: [0xFF; 4],
            unknown_010: 0x50,
            light_018: None,
            cursor_01c: None,
            cursor_state_020: [0.0; 6],
            unknown_038: 0.0,
            mouse_held_timer_03c: 0.0,
            debug_text_on_050: 0,
            main_root_054: None,
            cursor_root_058: None,
            unknown_05c: 0,
            player_scene_node_060: None,
            player_shadow_scene_064: None,
            menu_root_068: None,
            strings_06c: None,
            debug_text_root_070: None,
            camera_distance_074: 0.0,
            alpha_property_078: None,
            geometry_changed_07c: 0,
            unknown_07e: 0,
            hud_reticule_080: None,
            unknown_084: 0,
            alt_active_tile_088: None,
            default_selection_priority_08c: 100,
            big_four_rebuild_pending_094: 0,
            active_tile_098: None,
            active_menu_09c: None,
            unknown_0a0: 0,
            unknown_0a4: 0,
            unknown_0a8: 0,
            msg_box_button_pressed_0b0: 0xFF,
            message_callback_0b4: None,
            input_modifier_flags: 0,
            frame_seconds_host: 0.0,
            cursor_previous_b3a6e8: [0.0; 2],
            purge_textures_0b8: 0,
            cursor_moved_0b9: 1,
            references_0bc: [0; 5],
            unknown_0d0: [0.0; 3],
            unknown_0dc: 0,
            menu_mode_stack_0e0: [0; 10],
            look_at_108: None,
            unknown_10c: 0,
            unknown_110: 0,
            unknown_114: 0,
            unknown_118: [0; 3],
            unknown_128: 0,
            block_130: None,
        }
    }
}

/// IDA: 00B42D54, set by 0057D640 when the menu mode changes; read and cleared by later phases (0040552C, 0058406D).
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MenuModeChanged(pub u8);

/// IDA: N/A host helper. Completion: 100%.
fn require_attached(status: AttachChildStatus, message: &'static str) -> Result<(), StreamError> {
    if status != AttachChildStatus::Attached {
        return Err(StreamError(message));
    }
    Ok(())
}

/// IDA: N/A host helper. Completion: 100%.
fn require_updated(result: SceneUpdateResult, message: &'static str) -> Result<(), StreamError> {
    if result.status != SceneUpdateStatus::Updated {
        return Err(StreamError(message));
    }
    Ok(())
}

/// IDA: N/A host helper. Completion: 100%.
fn name_object(world: &mut World, entity: Entity, name: &[u8]) {
    set_object_name(
        &mut world
            .get_mut::<NiObjectNet>(entity)
            .expect("a NiObjectNET has its component")
            .name,
        Some(name),
    );
}

/// IDA: the scene-graph light builds (0057EF14..0057EF65): NiDirectionalLight (0x114), SetName, +0xB8 += 1,
/// +0xE0..+0xE8 ambient 1.0, 00708E40, then the node's AttachChild(light, 1). Completion: 100% of the order.
fn attach_scene_light(world: &mut World, node: Entity, name: &[u8]) -> Result<(), StreamError> {
    let light = spawn_directional_light(world);
    name_object(world, light, name);
    let mut state = world
        .get_mut::<DirectionalLight>(light)
        .expect("just spawned");
    state.unknown_b8 += 1;
    state.payload.ambient = [1.0, 1.0, 1.0];
    attach_affected_node(world, light, node);
    require_attached(
        attach_scene_child(world, node, Some(light), true),
        "light AttachChild: the child was not attached",
    )
}

/// IDA: N/A host helper. Completion: 100%.
fn spawn_property(world: &mut World, class: NiClass, property: impl Bundle) -> Entity {
    world.spawn((class, NiObjectNet::default(), property)).id()
}

/// IDA: the inline NiVertexColorProperty (0x1C, flags 8) with flags (8 & 0xFFEF) | 0x28 attached to the node.
/// Completion: 100%.
fn attach_vertex_color(world: &mut World, node: Entity) -> Result<(), StreamError> {
    let mut value = VertexColorProperty::default();
    value.flags_18 = (value.flags_18 & 0xFFEF) | 0x28;
    let property = spawn_property(world, NiClass::VertexColorProperty, value);
    attach_scene_property(world, node, Some(property))?;
    Ok(())
}

/// IDA: 0057D7A0 UI_GetVirtualWidthForAspectRatio: 1280 when height >= width, else float(width) / float(height) * 960.0
/// (24-bit). Completion: 100%.
#[must_use]
pub fn ui_virtual_width(settings: &InterfaceSettings) -> f32 {
    let width = settings.renderer_width as f32;
    let height = settings.renderer_height as f32;
    if height >= width {
        return 1280.0;
    }
    width / height * 960.0
}

/// IDA: 0057D7F0: 960 when width >= height, else height / width * 1280. Completion: 100%.
#[must_use]
pub fn ui_virtual_height(settings: &InterfaceSettings) -> f32 {
    let width = settings.renderer_width as f32;
    let height = settings.renderer_height as f32;
    if width >= height {
        return 960.0;
    }
    height / width * 1280.0
}

/// IDA: 0057D330: H / W (fild, fidiv, 24-bit) compared with 0.74 (A68D08): >= 0.74 takes 00B135F8, below takes 00B13608
/// (integer settings, fild). Completion: 100%.
#[must_use]
pub fn interface_safe_zone_x(settings: &InterfaceSettings) -> f32 {
    let ratio = settings.renderer_height as f32 / settings.renderer_width as f32;
    let wide = f64::from(ratio) < 0.74;
    (if wide {
        settings.safe_zone[2]
    } else {
        settings.safe_zone[0]
    }) as f32
}

/// IDA: 0057D390: the same test, 00B13600 and 00B13610. Completion: 100%.
#[must_use]
pub fn interface_safe_zone_y(settings: &InterfaceSettings) -> f32 {
    let ratio = settings.renderer_height as f32 / settings.renderer_width as f32;
    let wide = f64::from(ratio) < 0.74;
    (if wide {
        settings.safe_zone[3]
    } else {
        settings.safe_zone[1]
    }) as f32
}

/// IDA: 005730B0 (graph, distance, below): NiNode (0xDC) stored in faders +0 (below, "FaderNode Below Menus") or +4
/// (above, "FaderNode Above Menus", distance - 20.0); "FaderNodeLight"; vertex color property; scale = |s| with s =
/// 1.0, or 1.3333334 (A688AC)
/// when H/W != 0.75, times 1280/W when W != 1280; translate (0, distance, 0); graph AttachChild(node, 1). Completion:
/// 100%.
#[allow(clippy::assign_op_pattern)] // 005730B0's operand order: 1280/W first.
pub fn interface_create_fader_node(
    cx: &mut InterfaceCx,
    graph: Entity,
    mut distance: f32,
    below: bool,
) -> Result<(), StreamError> {
    let node = spawn_scene_node(cx.world, SceneNodePayload::default());
    cx.world.init_resource::<RendererUiFaderNodes>();
    let mut faders = cx.world.resource_mut::<RendererUiFaderNodes>();
    if below {
        faders.below_00 = Some(node);
        name_object(cx.world, node, b"FaderNode Below Menus");
    } else {
        faders.above_04 = Some(node);
        name_object(cx.world, node, b"FaderNode Above Menus");
        distance -= 20.0;
    }
    attach_scene_light(cx.world, node, b"FaderNodeLight")?;
    attach_vertex_color(cx.world, node)?;
    let width = cx.settings.renderer_width as f32;
    let mut scale = 1.0f32;
    let ratio = cx.settings.renderer_height as f32 / width;
    if ratio != 0.75 {
        scale = 1.333_333_4;
    }
    if width != 1280.0 {
        scale = 1280.0 / width * scale;
    }
    let mut payload = cx
        .world
        .get_mut::<SceneNodePayload>(node)
        .expect("just spawned");
    payload.local_transform.scale = scale.abs();
    payload.local_transform.translation = [0.0, distance, 0.0];
    require_attached(
        attach_scene_child(cx.world, graph, Some(node), true),
        "005732B0: the child was not attached",
    )
}

/// IDA: 0057ED20 (graph, name, menu3D). A null graph (not reached: the Phase 05 graphs exist) builds a SceneGraph
/// (0xF0) through 004114E0. Completion: 100% of the order.
pub fn interface_setup_scene_graph(
    cx: &mut InterfaceCx,
    manager: &mut InterfaceManager,
    graph: Option<Entity>,
    name: &[u8],
    menu_3d: bool,
) -> Result<Entity, StreamError> {
    let graph = match graph {
        Some(graph) => graph,
        None => construct_scene_graph(cx.world, name, false, None, &cx.settings.scene, cx.runtime)?,
    };
    // 0057ED86..0057EDC2: camera +0x30 rotation {0,0,1, 1,0,0, 0,1,0}.
    if let Some(camera) = cx
        .world
        .get::<SceneGraph>(graph)
        .and_then(|state| state.camera_dc)
    {
        cx.world
            .get_mut::<SceneCamera>(camera)
            .expect("+0xDC is a NiCamera")
            .local_transform_30
            .rotation = [0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
    }
    // 0057EDC4..0057EE12: NiZBufferProperty (flags 15 & 0xFFFD | 1 = 0x0D).
    let mut z_buffer = ZBufferProperty::default();
    z_buffer.flags_18 = (z_buffer.flags_18 & 0xFFFD) | 1;
    let z_buffer = spawn_property(cx.world, NiClass::ZBufferProperty, z_buffer);
    attach_scene_property(cx.world, graph, Some(z_buffer))?;
    // 0057EE1D the manager's alpha property (+0x78).
    attach_scene_property(cx.world, graph, manager.alpha_property_078)?;
    if menu_3d {
        // 0057F143 SceneGraph_SetCameraFOV(35.0, 0).
        require_updated(
            scene_graph_set_camera_fov(
                cx.world,
                graph,
                35.0,
                false,
                &cx.settings.scene,
                cx.runtime,
            )?,
            "0057F151: the scene update reported an unsupported state",
        )?;
        return Ok(graph);
    }
    // 0057EE3D 005730B0(g_rendererUIState, graph, +0x74, 1).
    interface_create_fader_node(cx, graph, manager.camera_distance_074, true)?;
    // 0057EE42..0057EF65 Main Root: NiNode (0xDC) -> +0x54, name, AttachChild(node, 0), translate (0, +0x74, 0),
    // MainSceneLight.
    let main_root = spawn_scene_node(cx.world, SceneNodePayload::default());
    manager.main_root_054 = Some(main_root);
    name_object(cx.world, main_root, b"InterfaceManager: Main Root");
    require_attached(
        attach_scene_child(cx.world, graph, Some(main_root), false),
        "0057EEAC: the child was not attached",
    )?;
    cx.world
        .get_mut::<SceneNodePayload>(main_root)
        .expect("just spawned")
        .local_transform
        .translation = [0.0, manager.camera_distance_074, 0.0];
    attach_scene_light(cx.world, main_root, b"MainSceneLight")?;
    attach_vertex_color(cx.world, main_root)?;
    // 0057EFB7..0057F127 Cursor Root (+0x58) and CursorSceneLight.
    let cursor_root = spawn_scene_node(cx.world, SceneNodePayload::default());
    manager.cursor_root_058 = Some(cursor_root);
    name_object(cx.world, cursor_root, b"InterfaceManager: Cursor Root");
    require_attached(
        attach_scene_child(cx.world, graph, Some(cursor_root), false),
        "0057F021: the child was not attached",
    )?;
    cx.world
        .get_mut::<SceneNodePayload>(cursor_root)
        .expect("just spawned")
        .local_transform
        .translation = [0.0, manager.camera_distance_074, 0.0];
    attach_scene_light(cx.world, cursor_root, b"CursorSceneLight")?;
    attach_vertex_color(cx.world, cursor_root)?;
    // 0057F13C 005730B0(g_rendererUIState, graph, +0x74, 0).
    interface_create_fader_node(cx, graph, manager.camera_distance_074, false)?;
    Ok(graph)
}

/// IDA: 005802B0 InterfaceManager_ctor. Host: the field defaults are the zeroing stores; the non-zero stores and the
/// calls follow the disassembly. Completion: 100% of the store order and widths.
pub fn construct_interface_manager(cx: &mut InterfaceCx) -> Result<InterfaceManager, StreamError> {
    // 005802DF..00580386: +0x00, +0x04, +0x78 null, the 0.0 floats, +0x08 = 1, +0x09 = 0, +0x0A..+0x0D = 0xFF, +0x7E
    // word 0, +0x88, +0x98..+0xA4 0. 005803BB +0x38 = 0.0, 005803BE +0x10 = 0x50, 005803EF +0x3C = 0.0.
    // 005803F2..00580485: +0x10C, +0x1C,
    // +0x114, +0x118..+0x128, +0x80, +0x84, +0xA8 byte 0, +0xB0 0xFF, +0xB4 0, +0xB8 0, +0xB9 1, +0x8C 100, +0x94 0,
    // +0xE0..+0x104 0.
    let mut manager = InterfaceManager::default();
    // 00580491..00580511 +0x74 = w_v * 0.5 / sin(a) * cos(a), a = 0.6544985 (A690D0): each product and quotient rounds
    // to float; __CIsin and __CIcos are double results stored as floats. The last bit of the host's trig is decision
    // D6's: fixed only if a row
    // disagrees, and P07-RUN-001's 448B02BD agrees.
    let virtual_width = ui_virtual_width(cx.settings);
    let half = f64::from(virtual_width * 0.5);
    let angle = 0.654_498_457_908_630_4_f64;
    let sine = angle.sin() as f32;
    let quotient = (half / f64::from(sine)) as f32;
    let cosine = angle.cos() as f32;
    manager.camera_distance_074 = cosine * quotient;
    // 0058051A..00580597: NiAlphaProperty (0x1C, flags 0xEC, +0x1A 0) when +0x78 is null; NiPointer swap; flags |= 1
    // then &= 0xDFFF.
    let mut alpha = AlphaProperty::default();
    alpha.flags_18 |= 1;
    alpha.flags_18 &= 0xDFFF;
    manager.alpha_property_078 = Some(spawn_property(cx.world, NiClass::AlphaProperty, alpha));
    // 0058059D 0057ED20(Menu3DRoot, 1) -> +0x04; 005805EC 0057ED20(MenuRoot, 0) -> +0x00.
    let graphs = cx.world.get_resource::<SceneGraphGlobals>();
    let (menu, menu_3d) = (
        graphs.and_then(|graphs| graphs.menu),
        graphs.and_then(|graphs| graphs.menu_3d),
    );
    manager.menu_3d_scene_graph_004 = Some(interface_setup_scene_graph(
        cx,
        &mut manager,
        menu_3d,
        b"Menu3DRoot",
        true,
    )?);
    manager.menu_scene_graph_000 = Some(interface_setup_scene_graph(
        cx,
        &mut manager,
        menu,
        b"MenuRoot",
        false,
    )?);
    // 00580637 FontManager_GetSingleton.
    font_manager_get_singleton(cx.world, &cx.settings.font_paths, cx.host);
    // 0058063E 00587B20(1): 00B3A708 one-byte allocation, the TileMenu array sized to 0x33 (host fixed slots).
    cx.engine.menu_array_allocated_b3a708 = true;
    // 00580643 005888A0: clear the 00B3AF10 evaluation stack and 00B3B0AC.
    cx.engine.evaluation_stack = [None; 100];
    cx.engine.evaluation_depth = 0;
    // 00580648..00580671 InterfaceLookAtHelper (0xC) -> +0x108.
    manager.look_at_108 = Some(InterfaceLookAtHelper::default());
    // 0058066A..0058069D the 0x14 block -> +0x130; +0x130->+0xC = itself. The original stores through it even on a null
    // allocation (a crash path); host allocation does not fail.
    manager.block_130 = Some(InterfaceBlock130 {
        unknown_0: 0,
        unknown_4: 0.0,
        unknown_8: 0.001,
        self_c: true,
        unknown_10: 0,
        nodes: Vec::new(),
        blend_progress: 0.0,
    });
    // 005806A6 +0x10C = 0, 005806AC +0x110 = 0.
    manager.unknown_10c = 0;
    manager.unknown_110 = 0;
    Ok(manager)
}

/// IDA: N/A host helper. Completion: 100%.
fn manager_mut<'a>(cx: &'a mut InterfaceCx) -> Mut<'a, InterfaceManager> {
    cx.world.resource_mut::<InterfaceManager>()
}

/// IDA: 0057E7C0 InterfaceManager_UpdateCursorPosition. Returns at once when the input byte has 8 and the display is
/// windowed (this configuration, P07-RUN-001: the cursor kept +0xFA1 1.0 and the (0, -1, 0) translate). The positioning
/// branch follows the
/// disassembly (float, 24-bit). Completion: 100% for the reached branch.
/// TODO: [ ] Compare the positioning branch with an original run (full screen).
pub fn interface_update_cursor_position(cx: &mut InterfaceCx) -> Result<(), StreamError> {
    if cx.settings.input_flags & 8 != 0 && !cx.settings.full_screen {
        return Ok(());
    }
    let Some(cursor) = manager_mut(cx).cursor_01c else {
        return Ok(());
    };
    let Some(node) = super::interface_scene::tile_scene_node(cx, cursor) else {
        return Ok(());
    };
    cx.world
        .get_mut::<SceneNodePayload>(node)
        .expect("a tile node has a payload")
        .effective_av_flags &= !1;
    tile_set_numeric_trait(cx, cursor, TRAIT_VISIBLE, 2.0)?;
    let mouse_x = cx.host.mouse_axis(1);
    let mouse_y = cx.host.mouse_axis(2);
    let mut translate = cx
        .world
        .get::<SceneNodePayload>(node)
        .expect("a tile node has a payload")
        .local_transform
        .translation;
    let virtual_width = ui_virtual_width(cx.settings);
    let virtual_height = ui_virtual_height(cx.settings);
    let renderer_width = cx.settings.renderer_width as f32;
    let renderer_height = cx.settings.renderer_height as f32;
    // 0057E83F..0057E8B2: x = clamp(x + w_v / W * dx) within the borders.
    let right = virtual_width * 0.5 - 2.0;
    let left = -virtual_width * 0.5 - 2.0;
    let mut x = virtual_width / renderer_width * mouse_x as f32 + translate[0];
    x = if right < x { right } else { x };
    x = if left < x { x } else { left };
    translate[0] = x;
    // 0057E8AD..0057E8D6: y = depth * -0.008.
    translate[1] = tile_numeric_trait(cx, cursor, TRAIT_DEPTH) * -0.008;
    // 0057E8C7..0057E93D: z = clamp(z - h_v / H * dy) within the borders.
    let top = virtual_height * 0.5 + 1.0;
    let bottom = -virtual_height * 0.5 + 1.0;
    let mut z = translate[2] - virtual_height / renderer_height * mouse_y as f32;
    z = if top < z { top } else { z };
    z = if bottom < z { z } else { bottom };
    translate[2] = z;
    cx.world
        .get_mut::<SceneNodePayload>(node)
        .expect("a tile node has a payload")
        .local_transform
        .translation = translate;
    let depth = tile_numeric_trait(cx, cursor, TRAIT_DEPTH);
    let mut manager = manager_mut(cx);
    if manager.cursor_state_020[0] != translate[0] || manager.cursor_state_020[2] != translate[2] {
        manager.cursor_state_020[0] = translate[0];
        manager.cursor_state_020[1] = translate[1];
        manager.cursor_state_020[2] = translate[2];
        manager.cursor_moved_0b9 = 1;
        let scale_x = renderer_width / virtual_width;
        manager.cursor_state_020[3] = scale_x * manager.cursor_state_020[0] + renderer_width * 0.5;
        manager.cursor_state_020[4] = depth * -0.008;
        let scale_y = renderer_height / virtual_height;
        manager.cursor_state_020[5] = renderer_height * 0.5 - scale_y * manager.cursor_state_020[2];
    }
    require_updated(
        update_scene_object(cx.world, node, 0.0, true, cx.runtime)?,
        "0057EA14: NiAVObject_Update reported an unsupported state",
    )
}

/// IDA: 005903E0: 0058FBA0(menuRoot, 0) when present, then the texture palette purge when +0xB8 is set (it is 0 at
/// startup). Completion: 100%.
/// TODO: [ ] The palette purge, when a later phase sets +0xB8.
pub fn interface_update_menu_root(cx: &mut InterfaceCx) -> Result<(), StreamError> {
    if !cx.world.contains_resource::<InterfaceManager>() {
        return Ok(());
    }
    if let Some(root) = manager_mut(cx).menu_root_068 {
        tile_update_scene_graph(cx, root, false)?;
    }
    if manager_mut(cx).purge_textures_0b8 != 0 {
        manager_mut(cx).purge_textures_0b8 = 0;
        cx.host
            .warning("005903E0: texture palette purge is carried (host palette)");
    }
    Ok(())
}

/// IDA: 00581CC0 InterfaceManager_Initialize(createLoadingMenu). Completion: 100% of the order.
pub fn initialize_interface_manager(
    cx: &mut InterfaceCx,
    create_loading_menu: bool,
) -> Result<(), StreamError> {
    // 00581CE9..00581D59 Menus Root: TileRect (0x44) inline -> +0x68, vt+4 (0, "InterfaceManager: Menus Root", 0), then
    // the name "MenuRoot".
    let root = tile_initialize(
        cx,
        TileKind::Rect,
        None,
        Some(b"InterfaceManager: Menus Root"),
        None,
    )?;
    manager_mut(cx).menu_root_068 = Some(root);
    cx.engine.tile_mut(root).name_08 = b"MenuRoot".to_vec();
    // 00581D5E..00581E6A the seven traits.
    tile_set_numeric_trait(cx, root, 0xFA6, 2.0)?;
    tile_set_numeric_trait(cx, root, TRAIT_SELECTED_BIG_FOUR, 1003.0)?;
    tile_set_numeric_trait(cx, root, TRAIT_ALPHA, 0.0)?;
    let value = ui_virtual_width(cx.settings);
    tile_set_numeric_trait(cx, root, TRAIT_WIDTH, value)?;
    let value = ui_virtual_height(cx.settings);
    tile_set_numeric_trait(cx, root, TRAIT_HEIGHT, value)?;
    let value = interface_safe_zone_x(cx.settings);
    tile_set_numeric_trait(cx, root, 0xFDA, value)?;
    let value = interface_safe_zone_y(cx.settings);
    tile_set_numeric_trait(cx, root, 0xFD9, value)?;
    // 00581E6F 005903E0.
    interface_update_menu_root(cx)?;
    // 00581E7C Tile_ReadXML(menuRoot, strings.xml) -> +0x6C.
    let strings = tile_read_xml(cx, Some(root), "Data\\Menus\\strings.xml")?;
    manager_mut(cx).strings_06c = strings;
    // 00581E8A TileXML_ClearCachedFile(path, 0).
    tile_xml_clear_cached_file(cx, "Data\\Menus\\strings.xml", None);
    // 00581E97 0058D1C0(strings, 0, 0): detach (the original keeps the tile through +0x6C only).
    if let Some(strings) = strings {
        tile_set_parent(cx, strings, None, None)?;
    }
    // 00581E9C..00581F0D DebugText root (+0x70): NiNode, name, flags &= ~1, Main Root AttachChild(node, 0), then [+0]
    // RefreshPropertyState and RefreshEffectState (no persistent host state: the tile geometry is substituted, D3) and
    // NiAVObject_Update(0.0, 1).
    let debug_text = spawn_scene_node(cx.world, SceneNodePayload::default());
    manager_mut(cx).debug_text_root_070 = Some(debug_text);
    name_object(cx.world, debug_text, b"InterfaceManager: DebugText Root");
    cx.world
        .get_mut::<SceneNodePayload>(debug_text)
        .expect("just spawned")
        .effective_av_flags &= !1;
    if let Some(main_root) = manager_mut(cx).main_root_054 {
        require_attached(
            attach_scene_child(cx.world, main_root, Some(debug_text), false),
            "00581EF1: the child was not attached",
        )?;
    }
    if let Some(graph) = manager_mut(cx).menu_scene_graph_000 {
        require_updated(
            update_scene_object(cx.world, graph, 0.0, true, cx.runtime)?,
            "00581F0D: NiAVObject_Update reported an unsupported state",
        )?;
    }
    // 00581F12..00581F66 BSFogProperty (+0x2C = +0x30 = 0) on [+4]; +0x50 = 0.
    let fog = FogProperty {
        start_distance_2c: 0.0,
        end_distance_30: 0.0,
        ..FogProperty::default()
    };
    let fog = cx.world.spawn((NiObjectNet::default(), fog)).id();
    if let Some(graph) = manager_mut(cx).menu_3d_scene_graph_004 {
        attach_scene_property(cx.world, graph, Some(fog))?;
    }
    manager_mut(cx).debug_text_on_050 = 0;
    // 00581F64..00582083 the cursor TileImage (0x4C) with vt+4 (0, "Cursor", 0) and its traits, then 0058E870.
    let cursor = tile_initialize(cx, TileKind::Image, None, Some(b"Cursor"), None)?;
    tile_set_numeric_trait(cx, cursor, TRAIT_DEPTH, 2000.0)?;
    tile_set_numeric_trait(cx, cursor, TRAIT_WIDTH, 64.0)?;
    tile_set_numeric_trait(cx, cursor, TRAIT_HEIGHT, 64.0)?;
    tile_set_string_trait(cx, cursor, TRAIT_FILENAME, b"Menus\\Misc\\cursor.dds")?;
    tile_set_numeric_trait(cx, cursor, TRAIT_X, 0.0)?;
    tile_set_numeric_trait(cx, cursor, TRAIT_Y, 0.0)?;
    tile_set_numeric_trait(cx, cursor, TRAIT_VISIBLE, 1.0)?;
    tile_set_numeric_trait(cx, cursor, TRAIT_ALPHA, 255.0)?;
    tile_set_numeric_trait(cx, cursor, TRAIT_RED, 255.0)?;
    tile_set_numeric_trait(cx, cursor, TRAIT_GREEN, 255.0)?;
    tile_set_numeric_trait(cx, cursor, TRAIT_BLUE, 255.0)?;
    interface_update_tile_scene(cx, cursor)?;
    let Some(node) = super::interface_scene::tile_scene_node(cx, cursor) else {
        // 0058208D 00533D30(1, ...) -> DebugLog_Dispatch (host log). The original then dereferences the null node; the
        // host stops here.
        cx.host
            .warning("The Cursor could not be created. Check art resources.");
        return Err(StreamError(
            "00582094: the cursor scene node was not created",
        ));
    };
    // 0058209C Cursor Root AttachChild(node, 1); translate (0, -1, 0); refresh and NiAVObject_Update(0.0, 1).
    if let Some(cursor_root) = manager_mut(cx).cursor_root_058 {
        require_attached(
            attach_scene_child(cx.world, cursor_root, Some(node), true),
            "005820AD: the child was not attached",
        )?;
    }
    cx.world
        .get_mut::<SceneNodePayload>(node)
        .expect("a tile node has a payload")
        .local_transform
        .translation = [0.0, -1.0, 0.0];
    require_updated(
        update_scene_object(cx.world, node, 0.0, true, cx.runtime)?,
        "005820F6: NiAVObject_Update reported an unsupported state",
    )?;
    // 005820FD +0x1C = cursor; 00582100 0057E7C0; the node hidden (flags |= 1).
    manager_mut(cx).cursor_01c = Some(cursor);
    interface_update_cursor_position(cx)?;
    cx.world
        .get_mut::<SceneNodePayload>(node)
        .expect("a tile node has a payload")
        .effective_av_flags |= 1;
    // 00582110 the LoadingMenu when the flag is set.
    if create_loading_menu {
        loading_menu_ensure_created(cx)?;
    }
    // 0058211F the fog reference is released (the scene graph keeps it).
    Ok(())
}

/// IDA: 00582160 InterfaceManager_GetSingleton(create, initializationFlag). The create branch allocates, constructs,
/// publishes 00B3A6E0 and then initializes; every other call is a plain read. 00582190 FormHeap_Allocate(0x134),
/// 005821AF the ctor (an exception frees the block
/// through the 009BECE0 unwind funclet; the host owner is the local). A null allocation calls 00581CC0(null) in the
/// original (a crash path); host allocation does not fail. Returns whether 00B3A6E0 is non-null. Completion: 100%.
pub fn interface_manager_get_singleton(
    cx: &mut InterfaceCx,
    create: bool,
    initialization_flag: bool,
) -> Result<bool, StreamError> {
    if !create || cx.world.contains_resource::<InterfaceManager>() {
        return Ok(cx.world.contains_resource::<InterfaceManager>());
    }
    // The MoviePlayer thread's clone must already be in the World: a new one here would be a store nothing reads.
    if !cx.world.contains_resource::<InterfaceMovieReads>() {
        return Err(StreamError(
            "005821C7: InterfaceMovieReads is not in the World",
        ));
    }
    let manager = construct_interface_manager(cx)?;
    // 005821C7 publish before the initializer: the movie thread may see the half-initialized manager, as in the
    // original. It only reads presence, so D6 holds.
    cx.world.insert_resource(manager);
    cx.world
        .resource::<InterfaceMovieReads>()
        .publish_interface_manager(true);
    initialize_interface_manager(cx, initialization_flag)?;
    Ok(cx.world.contains_resource::<InterfaceManager>())
}

/// IDA: N/A `interface_manager_get_singleton` from outside an `InterfaceCx`. Completion: 100%.
pub fn run_interface_manager_get_singleton(
    world: &mut World,
    settings: &InterfaceSettings,
    host: &mut dyn InterfaceHost,
    runtime: &mut dyn SceneUpdateRuntime,
    create: bool,
    initialization_flag: bool,
) -> Result<bool, StreamError> {
    with_interface_cx(world, settings, host, runtime, |cx| {
        interface_manager_get_singleton(cx, create, initialization_flag)
    })
}

/// IDA: 00579260 (argument): with a manager and a cursor, the interface render 0057F170 unless MoviePlayer_IsRunning. The render's game side runs here, the drawing is the host's (D3): 00B42CE3 = 1; g_tileMenuArray[6] (1007) 0058FBA0(tile, 0) when the array holds one, else 005903E0; 00707370(+0x04, 0, 0) when +0x7C is set (57f38b `mov ecx, ebp`, and 0057F170's `v2 = this[1]`: the Menu3DRoot graph, not +0x00); +0x7C = 0. The accumulator, render targets and 00B42CE3's clear belong to the device. Completion: 100% of the game side.
pub fn interface_render_if_no_movie(
    cx: &mut InterfaceCx,
    argument: u32,
) -> Result<(), StreamError> {
    let ready = cx
        .world
        .get_resource::<InterfaceManager>()
        .is_some_and(|manager| manager.cursor_01c.is_some());
    if !ready || cx.host.movie_player_is_running() {
        return Ok(());
    }
    if let Some(loading) = cx.engine.tile_menu_by_id(1007) {
        tile_update_scene_graph(cx, loading, false)?;
    } else {
        interface_update_menu_root(cx)?;
    }
    let (changed, graph) = {
        let manager = manager_mut(cx);
        (
            manager.geometry_changed_07c,
            manager.menu_3d_scene_graph_004,
        )
    };
    if changed != 0
        && let Some(graph) = graph
    {
        require_updated(
            update_scene_object(cx.world, graph, 0.0, false, cx.runtime)?,
            "0057F170: NiAVObject_Update reported an unsupported state",
        )?;
    }
    // 0057F392 the render clears +0x7C.
    manager_mut(cx).geometry_changed_07c = 0;
    cx.host.render_interface(argument);
    Ok(())
}

/// IDA: 005792B0: false without a manager or a cursor, else IsRunning. Completion: 100%.
pub fn interface_movie_gate(cx: &mut InterfaceCx) -> bool {
    let ready = cx
        .world
        .get_resource::<InterfaceManager>()
        .is_some_and(|manager| manager.cursor_01c.is_some());
    ready && cx.host.movie_player_is_running()
}

/// IDA: 0057D640 InterfaceManager_PushMenuMode. The console (3) shifts the stack down (a full stack warns and returns
/// 9); any other mode is appended (-1 when ten are set). The first mode sets +0x08 = 3, and 00B42D54 = 1 unless the
/// append is Generic (1011) or Message
/// (1001) at slot 0, or at slot 1 only over a Generic or Message slot 0. Completion: 100%.
pub fn interface_push_menu_mode(cx: &mut InterfaceCx, mode: u32) -> i32 {
    let mut changed = cx.world.resource::<MenuModeChanged>().0;
    let mut manager = manager_mut(cx);
    let stack = &mut manager.menu_mode_stack_0e0;
    let result = if mode == 3 {
        let mut displaced = stack[0];
        stack[0] = 3;
        let mut count = 1;
        loop {
            let next = stack[count];
            let terminator = next == 0;
            stack[count] = displaced;
            displaced = next;
            if terminator {
                break;
            }
            count += 1;
            if count >= 9 {
                break;
            }
        }
        if count == 9 {
            cx.host
                .warning("### Menu Stack Size is too small - a menu may have been lost");
            return 9;
        }
        if count == 1 {
            manager_mut(cx).menu_loading_status_008 = 3;
            changed = 1;
        }
        count as i32
    } else {
        let mut index = 0;
        for &slot in stack.iter() {
            if slot == 0 {
                break;
            }
            index += 1;
            if index >= 10 {
                return -1;
            }
        }
        stack[index] = mode;
        if index == 0 {
            manager.menu_loading_status_008 = 3;
            if mode != 1011 && mode != 1001 {
                changed = 1;
            }
        } else if index == 1 {
            let first = manager.menu_mode_stack_0e0[0];
            if first == 1011 || first == 1001 {
                changed = 1;
            }
        }
        index as i32
    };
    cx.world.resource_mut::<MenuModeChanged>().0 = changed;
    result
}

/// IDA: 00584880 Menu_RegisterTile. Completion: 100%.
pub fn menu_register_tile(
    cx: &mut InterfaceCx,
    menu: MenuId,
    tile: Option<TileId>,
) -> Result<(), StreamError> {
    cx.engine.menu_mut(menu).tile_04 = tile;
    let Some(tile) = tile else {
        return Ok(());
    };
    let stacking = tile_numeric_trait(cx, tile, TRAIT_STACKING);
    if stacking == 102.0 || stacking == 6006.0 || stacking == 103.0 || stacking == 6007.0 {
        let id = cx.engine.menu(menu).id();
        cx.engine.menu_mut(menu).menu_mode_14 = id;
        if !cx.world.contains_resource::<InterfaceManager>() {
            return Ok(());
        }
        cx.world.init_resource::<MenuModeChanged>();
        interface_push_menu_mode(cx, id);
        if manager_mut(cx).active_menu_09c != Some(menu) {
            if let Some(active) = manager_mut(cx).active_tile_098 {
                tile_set_numeric_trait(cx, active, 0xFDD, 0.0)?;
            }
            let mut manager = manager_mut(cx);
            manager.active_tile_098 = None;
            manager.active_menu_09c = None;
        }
    }
    Ok(())
}

/// IDA: 00584980 Menu_UpdateCursorDepthAndGetNextDepth (00588890 thunk): the deepest (depth + menu +0x18, `fiadd`) over
/// the menu root's children except the MagicPopup, HUDInfo and HUDSubtitle menus (kept when not below it, so also a NaN:
/// 584a39 `test ah, 5; jnp`). Then 584a4e..584a67 m = float(ftol(deepest)), m < 10 -> 10.0; the cursor's depth is m + 100
/// and its node y that * -0.008; HUDSubtitle's is (m + 100) - 10. Returns deepest + 2 untruncated (584b18). The C++
/// differs: it applies no ftol. Completion: 100%.
pub fn menu_update_cursor_depth(cx: &mut InterfaceCx) -> Result<f32, StreamError> {
    let manager = cx.world.get_resource::<InterfaceManager>();
    let root = manager.and_then(|manager| manager.menu_root_068);
    let cursor = manager.and_then(|manager| manager.cursor_01c);
    let mut deepest = 0.0f32;
    if let Some(root) = root {
        for &child in &cx.engine.tile(root).children_30 {
            let Some(menu) = cx.engine.containing_menu(child) else {
                continue;
            };
            let owner = cx.engine.menu(menu);
            let id = owner.id();
            if id == 1024 || id == 1005 || id == 1010 {
                continue;
            }
            // 584a23 fiadd: the int is not rounded to 24 bits first.
            let depth = x87::stored_float(
                f64::from(tile_numeric_trait(cx, child, TRAIT_DEPTH)) + f64::from(owner.depth_18),
            );
            if !depth.lt(&deepest) {
                deepest = depth;
            }
        }
    }
    // 584a52..584a63 ftol, fild, fstp dword: an int above 2^24 rounds here.
    let mut truncated = x87::ftol(deepest) as f32;
    if truncated < 10.0 {
        truncated = 10.0;
    }
    let cursor_depth = truncated + 100.0;
    if let Some(cursor) = cursor {
        tile_set_numeric_trait(cx, cursor, TRAIT_DEPTH, cursor_depth)?;
        if let Some(node) = super::interface_scene::tile_scene_node(cx, cursor) {
            cx.world
                .get_mut::<SceneNodePayload>(node)
                .expect("a tile node has a payload")
                .local_transform
                .translation[1] = cursor_depth * -0.008;
        }
    }
    if let Some(subtitle) = cx.engine.tile_menu_by_id(1010) {
        tile_set_numeric_trait(cx, subtitle, TRAIT_DEPTH, cursor_depth - 10.0)?;
    }
    Ok(deepest + 2.0)
}

/// IDA: 00583DF0 (alpha): the cursor node's first geometry child's material alpha (and its revision). Completion: 100%
/// on the substituted record.
pub fn interface_set_cursor_alpha(cx: &mut InterfaceCx, alpha: f32) {
    let Some(cursor) = manager_mut(cx).cursor_01c else {
        return;
    };
    let record: Option<&mut TileSceneRecord> = cx.engine.tile_mut(cursor).scene_24.as_mut();
    if let Some(geometry) = record
        .filter(|record| record.node.is_some())
        .and_then(|record| record.geometry.first_mut())
    {
        geometry.material_revision_54 += 1;
        geometry.alpha = alpha;
    }
}

/// IDA: 00587D20 Menu_CreateByID: case 1007 -> FormHeap and 005AD0D0. The other classes are carried to the host's
/// answer. Completion: 100% of the reached case.
pub fn menu_create_by_id(
    cx: &mut InterfaceCx,
    menu_id: u32,
) -> Result<Option<MenuClass>, StreamError> {
    if menu_id == 1007 {
        return Ok(Some(MenuClass::Loading(construct_loading_menu(cx))));
    }
    // 005BC450 and 005AE450 store nothing that a default doesn't.
    match menu_id {
        1001 => return Ok(Some(MenuClass::Message(Default::default()))),
        1038 => return Ok(Some(MenuClass::Load(Default::default()))),
        // 005D9AD0 StatsMenu_ctor.
        1003 => return Ok(Some(MenuClass::Stats(Default::default()))),
        // 005A9B00 InventoryMenu_ctor.
        1002 => return Ok(Some(MenuClass::Inventory(Default::default()))),
        // 005B1F20 MagicMenu_ctor.
        1022 => return Ok(Some(MenuClass::Magic(Default::default()))),
        // 005B7360 MapMenu_ctor.
        1023 => return Ok(Some(MenuClass::Map(Default::default()))),
        // 005C0B50 QuickKeysMenu_ctor.
        1046 => {
            return Ok(Some(MenuClass::QuickKeys(
                super::quick_keys_menu::quick_keys_menu_constructed(cx.world),
            )));
        }
        1044 => {
            let movie = cx.host.main_menu_ctor_media(cx.world);
            cx.engine.main_menu_statics.movie_b3b408 = movie;
            return Ok(Some(MenuClass::Main(super::menus::main_menu_constructed())));
        }
        // 005A4660 HUDInfoMenu_ctor.
        1005 => {
            return Ok(Some(MenuClass::HudInfo(
                super::hud_main_menu::HudInfoMenu::default(),
            )));
        }
        // 005A8DF0 HUDSubtitleMenu_ctor.
        1010 => return Ok(Some(MenuClass::HudSubtitle(Default::default()))),
        // 005D6870 SleepWaitMenu_ctor.
        1012 => return Ok(Some(MenuClass::SleepWait(Default::default()))),
        // 005BD960 PauseMenu_ctor.
        1013 => return Ok(Some(MenuClass::Pause(Default::default()))),
        // 005BD580 OptionsMenu_ctor.
        1015 => return Ok(Some(MenuClass::Options(Default::default()))),
        // 005C04F0 QuantityMenu_ctor.
        1016 => return Ok(Some(MenuClass::Quantity(Default::default()))),
        // 00595150 AudioMenu_ctor.
        1017 => return Ok(Some(MenuClass::Audio(Default::default()))),
        // 005A3310 GameplayMenu_ctor.
        1020 => return Ok(Some(MenuClass::Gameplay(Default::default()))),
        // 005874A0 ControlsMenu_ctor.
        1021 => return Ok(Some(MenuClass::Controls(Box::default()))),
        // 005D3290 SaveMenu_ctor.
        1039 => return Ok(Some(MenuClass::Save(Default::default()))),
        // 005879C0 TextEditMenu_ctor.
        1051 => {
            return Ok(Some(MenuClass::TextEdit(
                super::text_edit_menu::text_edit_menu_constructed(cx.world),
            )));
        }
        // 005CE680 RechargeMenu_ctor.
        1049 => return Ok(Some(MenuClass::Recharge(Default::default()))),
        // 00596420 BreathMenu_ctor.
        1045 => return Ok(Some(MenuClass::Breath(Default::default()))),
        // 005B3D30 MagicPopupMenu_ctor, with its GetTickCount.
        1024 => {
            let tick = cx.host.tick_count();
            return Ok(Some(MenuClass::MagicPopup(
                super::magic_popup_menu::MagicPopupMenu::constructed(tick),
            )));
        }
        1004 => {
            return Ok(Some(MenuClass::Hud(
                super::hud_main_menu::hud_main_menu_constructed(),
            )));
        }
        _ => {}
    }
    cx.host.create_unported_menu(menu_id)
}

/// IDA: 00578E10 LoadingMenu_EnsureCreated(0): TileMenu_GetByMenuID(1007), else a tail jump to 005ADCF0. Completion:
/// 100%.
pub fn loading_menu_ensure_created(cx: &mut InterfaceCx) -> Result<Option<TileId>, StreamError> {
    loading_menu_ensure_created_for(cx, None, 0.0)
}

/// IDA: 00578E10 LoadingMenu_EnsureCreated(slideContext), as 00440AF0 calls it with a cell. Completion: 100%.
/// # Errors
/// A stop in 005ADCF0.
pub fn loading_menu_ensure_created_for(
    cx: &mut InterfaceCx,
    context: Option<Entity>,
    frame_time: f32,
) -> Result<Option<TileId>, StreamError> {
    if let Some(existing) = cx.engine.tile_menu_by_id(1007) {
        return Ok(Some(existing));
    }
    loading_menu_create_for(cx, context, frame_time)
}

/// IDA: 005ADCF0 LoadingMenu_Create(slideContext 0). The player, HUD and DataHandler branches take their startup arms
/// (P07-RUN-001) and are host stops otherwise. 005ADEA6: a wrong menu is destroyed (vt+0) when it has a tile; the
/// destruction is carried, as in the C++. Completion: 100% of the
/// order for the startup state.
/// TODO: [ ] The player, HUD and DataHandler arms, and destroying an existing LoadingMenu or SleepWait menu (later
/// phases).
pub fn loading_menu_create(cx: &mut InterfaceCx) -> Result<Option<TileId>, StreamError> {
    loading_menu_create_for(cx, None, 0.0)
}

/// IDA: 005ADCF0 LoadingMenu_Create(slideContext), with a slide context and the player's arms: 005A6040(1, 1), 005A4980(null, 0, 0) and 00578D50(0), each of which returns at once without a player, and with one MoviePlayer_FinishCurrentMovie when no movie runs. Completion: as `loading_menu_create`, with those arms.
/// # Errors
/// As `loading_menu_create`, or 00578D50 with the player's 3D.
pub fn loading_menu_create_for(
    cx: &mut InterfaceCx,
    context: Option<Entity>,
    frame_time: f32,
) -> Result<Option<TileId>, StreamError> {
    // 005ADCF6 an existing LoadingMenu is destroyed (none at startup).
    if cx.engine.tile_menu_by_id(1007).is_some() {
        return Err(StreamError(
            "005ADD0A: destroying an existing LoadingMenu is carried",
        ));
    }
    // 005ADD0E 005A8FD0: the HUDSubtitle menu's visible = 1 (none at startup).
    if let Some(subtitle) = cx.engine.tile_menu_by_id(1010)
        && cx.engine.containing_menu(subtitle).is_some()
    {
        tile_set_numeric_trait(cx, subtitle, TRAIT_VISIBLE, 1.0)?;
    }
    let root = cx
        .world
        .get_resource::<InterfaceManager>()
        .and_then(|manager| manager.menu_root_068)
        .ok_or(StreamError("005ADD2A: no InterfaceManager menu root"))?;
    // 005ADD21 Menu_GetNextDepth.
    let depth = menu_update_cursor_depth(cx)?;
    // 005ADD32 Tile_ReadXML(menuRoot, loading_menu.xml).
    let Some(tile) = tile_read_xml(cx, Some(root), "Data\\Menus\\loading_menu.xml")? else {
        return Ok(None);
    };
    let Some(menu) = cx.engine.containing_menu(tile) else {
        return Ok(None);
    };
    if cx.engine.menu(menu).id() != 1007 {
        return Ok(None);
    }
    // 005ADD60 00583DF0(0): the cursor's alpha 0.
    interface_set_cursor_alpha(cx, 0.0);
    // 005ADD7F Menu_RegisterTile(menu, tile as TileMenu).
    let tile_menu = (cx.engine.tile(tile).kind == TileKind::Menu).then_some(tile);
    menu_register_tile(cx, menu, tile_menu)?;
    // 005ADD9B..005ADDDE the depth trait when the stacking type is 6006 or 102.
    let stacking = tile_numeric_trait(cx, tile, TRAIT_STACKING);
    if stacking == 6006.0 || stacking == 102.0 {
        tile_set_numeric_trait(cx, tile, TRAIT_DEPTH, depth)?;
    }
    // 005ADDF0 +0x08 menuLoadingStatus = 2.
    manager_mut(cx).menu_loading_status_008 = 2;
    // 005ADDF4 005A6040(1, 1) and 005ADDFF 005A4980(0, 0, 0): each returns at once without g_thePlayer or the HUD's menus.
    super::hud_main_menu::hud_update_visibility(cx, true, true, frame_time)?;
    super::hud_main_menu::hud_info_clear_crosshair(cx, frame_time)?;
    // 005ADE06 00578D50(0): the player's node (vt+0x154), null returning; with one, 005A8C90.
    if let Some(player) = super::ai_conditions::the_player(cx.world)
        && cx
            .world
            .get::<super::actor::TesObjectRefr>(player)
            .is_some_and(|refr| refr.loaded_3d_3c != 0)
    {
        return Err(StreamError("00578D68: 005A8C90 with the player's 3D"));
    }
    // 005ADE17 +0x44 = slideContext; 005ADE21 user5 (0xFB3) = " ".
    let context_id = context.map_or(0, |form| super::forms_init::form_id(cx.world, form));
    if let MenuClass::Loading(loading) = &mut cx.engine.menu_mut(menu).class {
        loading.slide_context_44 = context_id;
        loading.slide_context = context;
    }
    tile_set_string_trait(cx, tile, TRAIT_USER5, b" ")?;
    // 005ADE2C LoadingMenu_BuildSlideList, 005ADE34 AdvanceSlide.
    loading_menu_build_slide_list(cx, tile)?;
    loading_menu_advance_slide(cx, menu)?;
    // 005ADE39 with a player and no movie running, MoviePlayer_FinishCurrentMovie.
    if super::ai_conditions::the_player(cx.world).is_some()
        && let Some(movies) = cx
            .world
            .get_resource::<super::movie_player::MoviePlayer>()
            .cloned()
        && !super::movie_player::movie_player_is_running(&movies)
    {
        super::movie_player::finish_current_movie(&movies);
    }
    // 005ADE53 0058FBA0(tile->parent, 0), 005ADE5E 0058FBA0(tile, 0).
    if let Some(parent) = cx.engine.tile(tile).parent {
        tile_update_scene_graph(cx, parent, false)?;
    }
    tile_update_scene_graph(cx, tile, false)?;
    // 005ADE68 the SleepWait menu (1012) is destroyed when present (none).
    if cx.engine.tile_menu_by_id(1012).is_some() {
        return Err(StreamError(
            "005ADE7C: destroying the SleepWait menu is carried",
        ));
    }
    // 005ADE80..005ADE91 the two render and gate pairs; 005ADE98 the shared stub.
    interface_render_if_no_movie(cx, 0)?;
    let _ = interface_movie_gate(cx);
    interface_render_if_no_movie(cx, 0)?;
    let _ = interface_movie_gate(cx);
    Ok(Some(tile))
}

/// IDA: 0057A0D0 Interface_RefreshMapMenu (Phase 14's 004431F0's last call): InterfaceManager (00582160(0, 1); null
/// returns 0), its +0x1C cursor and +0x68 menu root (null returns 0), the root's user0 trait (0xFAE) compared with 2.0
/// (A379B4; not equal or unordered returns 0), TileMenu_GetByMenuID(0x3FF)
/// and its containing menu (null returns 0), then 005B91E0 (the Map menu's world map tile) with 00B3B0A2 around
/// Tile_UpdateSceneGraph(0) and 1. P14-RUN-001: the root and cursor exist and 005B91E0 is not reached. Completion: 100%
/// to the 005B91E0 call (a host stop).
/// TODO: [ ] 005B91E0, when the Map menu exists (later phases).
pub fn interface_refresh_map_menu(cx: &mut InterfaceCx) -> Result<bool, StreamError> {
    let Some(manager) = cx.world.get_resource::<InterfaceManager>() else {
        return Ok(false);
    };
    let (Some(_), Some(root)) = (manager.cursor_01c, manager.menu_root_068) else {
        return Ok(false);
    };
    if tile_numeric_trait(cx, root, TRAIT_USER0) != 2.0 {
        return Ok(false);
    }
    let Some(map) = cx.engine.tile_menu_by_id(0x3FF) else {
        return Ok(false);
    };
    if cx.engine.containing_menu(map).is_none() {
        return Ok(false);
    }
    Err(StreamError(
        "0057A14C: 005B91E0 the Map menu world map refresh (no Map menu at startup)",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::gamebryo::scene_update::NoSceneControllers;
    use crate::tes::tile::NullInterfaceHost;

    fn settings() -> InterfaceSettings {
        let mut collections = crate::tes::settings::SettingCollections::default();
        crate::tes::settings::register_static_settings(&mut collections);
        interface_settings_from(
            &collections.ini,
            1920,
            1080,
            8,
            SceneGraphSettings::default(),
        )
    }

    // 0057D640: appends, the first mode's status byte, 00B42D54, and the console's shift. Against the C++ only.
    #[test]
    fn menu_mode_stack() {
        let mut world = World::new();
        world.insert_resource(InterfaceManager::default());
        world.init_resource::<MenuModeChanged>();
        let mut host = NullInterfaceHost::default();
        with_interface_cx(
            &mut world,
            &settings(),
            &mut host,
            &mut NoSceneControllers,
            |cx| {
                assert_eq!(interface_push_menu_mode(cx, 1001), 0);
                assert_eq!(manager_mut(cx).menu_loading_status_008, 3);
                assert_eq!(
                    cx.world.resource::<MenuModeChanged>().0,
                    0,
                    "Message at slot 0 does not set it"
                );
                assert_eq!(interface_push_menu_mode(cx, 1007), 1);
                assert_eq!(
                    cx.world.resource::<MenuModeChanged>().0,
                    1,
                    "slot 1 over Message"
                );
                // The console goes to the front and shifts the rest down.
                assert_eq!(interface_push_menu_mode(cx, 3), 2);
                assert_eq!(manager_mut(cx).menu_mode_stack_0e0[..4], [3, 1001, 1007, 0]);
                for mode in 2000..2007 {
                    assert!(interface_push_menu_mode(cx, mode) > 0);
                }
                assert_eq!(interface_push_menu_mode(cx, 9999), -1, "ten set");
                assert_eq!(interface_push_menu_mode(cx, 3), 9, "a full stack warns");
            },
        );
        assert_eq!(
            host.warnings,
            ["### Menu Stack Size is too small - a menu may have been lost"]
        );
    }

    // 0057A0D0's early exits. Its only reached arm (P14-RUN-001) returns 0 at the user0 test.
    #[test]
    fn refresh_map_menu_exits() {
        let mut world = World::new();
        let mut host = NullInterfaceHost::default();
        with_interface_cx(
            &mut world,
            &settings(),
            &mut host,
            &mut NoSceneControllers,
            |cx| {
                assert_eq!(interface_refresh_map_menu(cx), Ok(false));
                let root = tile_initialize(cx, TileKind::Rect, None, None, None).unwrap();
                let cursor = tile_initialize(cx, TileKind::Image, None, None, None).unwrap();
                cx.world.insert_resource(InterfaceManager {
                    menu_root_068: Some(root),
                    cursor_01c: Some(cursor),
                    ..InterfaceManager::default()
                });
                assert_eq!(interface_refresh_map_menu(cx), Ok(false));
                tile_set_numeric_trait(cx, root, TRAIT_USER0, 2.0).unwrap();
                assert_eq!(interface_refresh_map_menu(cx), Ok(false), "no Map menu");
            },
        );
    }

    // 584a4e..584a67 m = float(ftol(deepest)); the return is deepest + 2 untruncated.
    #[test]
    fn cursor_depth_truncates_but_the_return_does_not() {
        let mut world = World::new();
        let mut host = NullInterfaceHost::default();
        with_interface_cx(
            &mut world,
            &settings(),
            &mut host,
            &mut NoSceneControllers,
            |cx| {
                let root = tile_initialize(cx, TileKind::Rect, None, None, None).unwrap();
                let cursor = tile_initialize(cx, TileKind::Image, None, None, None).unwrap();
                let child = tile_initialize(cx, TileKind::Menu, Some(root), None, None).unwrap();
                let menu = MenuId(cx.engine.menus.len() as u32);
                cx.engine
                    .menus
                    .push(crate::tes::tile::Menu::new(MenuClass::Unported(1001)));
                cx.engine.tile_mut(child).menu_44 = Some(menu);
                cx.world.insert_resource(InterfaceManager {
                    menu_root_068: Some(root),
                    cursor_01c: Some(cursor),
                    ..InterfaceManager::default()
                });
                tile_set_numeric_trait(cx, child, TRAIT_DEPTH, 20.75).unwrap();
                assert_eq!(menu_update_cursor_depth(cx), Ok(22.75));
                assert_eq!(tile_numeric_trait(cx, cursor, TRAIT_DEPTH), 120.0);
                // Below 10: the floor for the cursor, the plain value for the return.
                tile_set_numeric_trait(cx, child, TRAIT_DEPTH, 3.5).unwrap();
                assert_eq!(menu_update_cursor_depth(cx), Ok(5.5));
                assert_eq!(tile_numeric_trait(cx, cursor, TRAIT_DEPTH), 110.0);
            },
        );
    }

    #[test]
    fn move_bar_settings_come_from_their_addresses() {
        assert_eq!(settings().loading_move_bar_ms, [2500, 100, 10]);
    }

    // 005898F0: the top tile decides. A menu below the top is not found, and a non-menu top gives null.
    #[test]
    fn containing_menu_looks_only_at_the_top() {
        let mut world = World::new();
        let mut host = NullInterfaceHost::default();
        with_interface_cx(
            &mut world,
            &settings(),
            &mut host,
            &mut NoSceneControllers,
            |cx| {
                let root = tile_initialize(cx, TileKind::Rect, None, None, None).unwrap();
                let rect = tile_initialize(cx, TileKind::Rect, Some(root), None, None).unwrap();
                let nested = tile_initialize(cx, TileKind::Menu, Some(rect), None, None).unwrap();
                let menu = MenuId(cx.engine.menus.len() as u32);
                cx.engine
                    .menus
                    .push(crate::tes::tile::Menu::new(MenuClass::Unported(1001)));
                cx.engine.tile_mut(nested).menu_44 = Some(menu);
                assert_eq!(cx.engine.containing_menu(nested), None);
                let top = tile_initialize(cx, TileKind::Menu, Some(root), None, None).unwrap();
                let below = tile_initialize(cx, TileKind::Rect, Some(top), None, None).unwrap();
                cx.engine.tile_mut(top).menu_44 = Some(menu);
                assert_eq!(cx.engine.containing_menu(below), Some(menu));
                assert_eq!(cx.engine.containing_menu(top), Some(menu));
            },
        );
    }

    #[test]
    fn tile_menu_table_bounds() {
        let reads = InterfaceMovieReads::default();
        assert!(!reads.interface_manager_present());
        assert!(!reads.tile_menu_present(MENU_ID_MESSAGE));
        reads.set_tile_menu(MENU_ID_MESSAGE, true);
        assert!(reads.clone().tile_menu_present(MENU_ID_MESSAGE));
        assert!(!reads.tile_menu_present(1002));
        // 00589B7A `cmp ecx, 32h; ja`: the last slot, then past it and below it.
        reads.set_tile_menu(1001 + 0x32, true);
        assert!(reads.tile_menu_present(1001 + 0x32));
        reads.set_tile_menu(1001 + 0x33, true);
        assert!(!reads.tile_menu_present(1001 + 0x33));
        assert!(!reads.tile_menu_present(1000));
    }
}

/// IDA: N/A the counts of 0057E150's menu creations, which the port doesn't make.
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct BigFourMenuBoundaries {
    pub calls: std::collections::BTreeMap<u32, u32>,
    /// IDA: N/A the main thread's CRT after 0057A440's StatsMenu, with a host `CrtProbe`.
    pub crt_after_stats: Option<(u32, u64)>,
}

/// IDA: 0057E150 InterfaceManager_RebuildBigFourMenusIfPending: with g_thePlayer (00B333C4), +0x94 and TLS +0x184 clear, 00B3B0A2 = 1 and +0x08 kept; 005AD980(0); 0057A440 (005DCB70 StatsMenu_Create), 0057A2D0 (Inventory), 0057A180 (Magic) and 00579F80 (Map), each followed by 005AD980(0) and, with a tile, visible 1.0; 005C1290 (QuickKeys) and 005AD980(0); 00B3B0A2 = 0, +0x08 restored, +0x94 = 0, 005AD980(0). Otherwise it returns. Completion: 100% of the order; all five are built: StatsMenu (0057A440 → 005DCB70, `stats_menu`), InventoryMenu (0057A2D0, `inventory_menu`), MagicMenu (0057A180, `magic_menu`), MapMenu (00579F80, `map_menu`) and QuickKeysMenu (005C1290, `quick_keys_menu`).
/// # Errors
/// A stop in 005AD980.
pub fn interface_rebuild_big_four_menus_if_pending(
    cx: &mut InterfaceCx,
) -> Result<(), StreamError> {
    let pending = cx
        .world
        .get_resource::<InterfaceManager>()
        .is_some_and(|manager| manager.big_four_rebuild_pending_094 != 0);
    if super::ai_conditions::the_player(cx.world).is_none()
        || !pending
        || cx.world.resource::<super::tes::GameThreadLocals>().byte_184 != 0
    {
        return Ok(());
    }
    cx.engine.in_scene_update = true;
    let status = manager_mut(cx).menu_loading_status_008;
    super::loading_menu::loading_menu_update_display(cx, false)?;
    // 0057A440: with a cursor (+0x1C) and +0x60, 005DCB70 StatsMenu_Create.
    let ready = {
        let manager = cx.world.resource::<InterfaceManager>();
        manager.cursor_01c.is_some() && manager.player_scene_node_060.is_some()
    };
    let stats = if ready {
        super::stats_menu::stats_menu_create(cx)?
    } else {
        None
    };
    let crt = super::ai_conditions::crt_probe(cx.world);
    cx.world
        .get_resource_or_init::<BigFourMenuBoundaries>()
        .crt_after_stats = crt;
    super::loading_menu::loading_menu_update_display(cx, false)?;
    // 0057E1BE: the created tile's visible, 1.0.
    if let Some(stats) = stats {
        super::tile_values::tile_set_numeric_trait(
            cx,
            stats,
            super::tile_values::TRAIT_VISIBLE,
            1.0,
        )?;
    }
    // 0057E1C3: 0057A2D0, then 005AD980(0) and the tile's visible 1.0 (0057E1E5).
    let inventory = super::inventory_menu::inventory_menu_create(cx)?;
    super::loading_menu::loading_menu_update_display(cx, false)?;
    if let Some(inventory) = inventory {
        super::tile_values::tile_set_numeric_trait(
            cx,
            inventory,
            super::tile_values::TRAIT_VISIBLE,
            1.0,
        )?;
    }
    // 0057E1EA: 0057A180, then 005AD980(0) and the tile's visible 1.0 (0057E20C).
    let magic = super::magic_menu::magic_menu_create_if_ready(cx)?;
    super::loading_menu::loading_menu_update_display(cx, false)?;
    if let Some(magic) = magic {
        super::tile_values::tile_set_numeric_trait(
            cx,
            magic,
            super::tile_values::TRAIT_VISIBLE,
            1.0,
        )?;
    }
    // 0057E211: 00579F80, then 005AD980(0) and the tile's visible 1.0 (0057E233).
    let map = super::map_menu::map_menu_create(cx)?;
    super::loading_menu::loading_menu_update_display(cx, false)?;
    if let Some(map) = map {
        super::tile_values::tile_set_numeric_trait(
            cx,
            map,
            super::tile_values::TRAIT_VISIBLE,
            1.0,
        )?;
    }
    // 0057E238: 005C1290, its answer unused.
    super::quick_keys_menu::quick_keys_menu_create(cx)?;
    super::loading_menu::loading_menu_update_display(cx, false)?;
    cx.engine.in_scene_update = false;
    let mut manager = manager_mut(cx);
    manager.menu_loading_status_008 = status;
    manager.big_four_rebuild_pending_094 = 0;
    super::loading_menu::loading_menu_update_display(cx, false)?;
    Ok(())
}
