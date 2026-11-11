//! M6: what the interface does each frame outside the render: the 0x14 timed transitions of `InterfaceManager` +0x130, a tile's trait transitions, the menu fades, the menu modes, the cursor from the window, and the menu branch of `InterfaceManager_UpdateFrame`. Main thread after CreateDevice: 24-bit x87, so `f32` arithmetic in the original order.

use oblivion_core::error::GameError;
use oblivion_core::x87;

use super::interface_draw::interface_pick;
use super::interface_manager::{
    InterfaceManager, MenuModeChanged, menu_update_cursor_depth, ui_virtual_height,
    ui_virtual_width,
};
use super::interface_scene::{interface_update_tile_scene, release_node, tile_scene_node};
use super::menu_slots::{
    menu_handle_click, menu_handle_keyboard, menu_handle_mouseout, menu_handle_mouseover,
    menu_handle_navigation, menu_update_frame,
};
use super::tile::{InterfaceCx, MenuClass, MenuId, TileId};
use super::tile_values::*;
use crate::gamebryo::scene_node::SceneNodePayload;
use crate::gamebryo::scene_update::{SceneUpdateStatus, update_scene_object};
use crate::tes::actor::TesObjectRefr;
use crate::tes::hud_main_menu::hud_update_visibility;
use crate::tes::interface_manager::interface_update_reticle;
use crate::tes::player::the_player;
use crate::tes::tes::TesGlobals;

/// IDA: Trait IDs this file names, from the 0058A1C0 table.
pub const TRAIT_DISABLE_FADE: u16 = 4009;
pub const TRAIT_MOUSEOVER: u16 = 4061;
pub const TRAIT_SHIFT_CLICKED: u16 = 4063;
pub const TRAIT_CLICKED: u16 = 4065;
pub const TRAIT_CLICK_COUNT_BEFORE: u16 = 4066;
pub const TRAIT_CLICK_COUNT_AFTER: u16 = 4067;
pub const TRAIT_CLICK_SOUND: u16 = 4069;
pub const TRAIT_TARGET: u16 = 4041;
pub const TRAIT_XDEFAULT: u16 = 4080;
pub const TRAIT_XUP: u16 = 4081;
pub const TRAIT_XDOWN: u16 = 4082;
pub const TRAIT_XLEFT: u16 = 4083;
pub const TRAIT_XRIGHT: u16 = 4084;
pub const TRAIT_XSCROLL: u16 = 4085;
pub const TRAIT_XLIST: u16 = 4086;
/// IDA: 005843A0 `push 0FDBh` and 005843B9 `push 0FDCh`, the token table's menufade and explorefade. 0x1772 is 00583F40's hide or destroy test.
pub const TRAIT_FADE_IN: u16 = 0xFDB;
pub const TRAIT_FADE_OUT: u16 = 0xFDC;
pub const TRAIT_DESTROY_ON_CLOSE: u16 = 0x1772;

/// IDA: Menu +0x24 MenuVisibilityState. Completion: the values this port reaches.
pub const MENU_STATE_FADING_OUT: u32 = 2;
pub const MENU_STATE_HIDDEN: u32 = 4;
pub const MENU_STATE_VISIBLE: u32 = 1;
pub const MENU_STATE_FADING_IN: u32 = 8;

/// IDA: N/A what a 00584300 timer points at: an opaque pointer in the exe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransitionTarget {
    Menu(MenuId),
    Trait(u32),
}

/// IDA: TimedTransitionNode (0x14): +0 target, +4 elapsed, +8 duration, +C previous, +10 next (00584300, 00583C41, 00583D67). The list is a `Vec` in order. Completion: 100%.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimedTransitionNode {
    pub elapsed_04: f32,
    pub duration_08: f32,
    pub target_00: TransitionTarget,
}

/// IDA: TileTraitTransition (0x18): +0 owner (the tile that holds it), +4 trait, +8 from, +C to, +10 duration, +14 next. `id` stands for its address, the timer's target. Completion: 100%.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TileTraitTransition {
    pub id: u32,
    pub trait_04: u16,
    pub from_08: f32,
    pub to_0c: f32,
    pub duration_10: f32,
}

/// IDA: N/A a host's frame for the interface: GetTickCount's frame time, the window's pointer, what the mouse buttons did and the keys pressed. A recorded substitution for DirectInput's buffers (OSInputGlobals), which the windowed path of 0057E7C0 doesn't read for the cursor anyway.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InterfaceInputFrame {
    /// g_timeInfo.frameTime, seconds.
    pub frame_time: f32,
    /// The pointer over the client area, 0 to 1 on each axis; 0057F490's arguments. `None` when it is outside.
    pub cursor: Option<[f32; 2]>,
    /// 004031E0 OSInputGlobals_QueryMouseButtonState(0, 1): the left button went down this frame.
    pub left_pressed: bool,
    /// (0, 2): it came up this frame.
    pub left_released: bool,
    /// (0, 0): it is held.
    pub left_held: bool,
    /// What 0057F7A0 returned for each key press, in order, and the shift state after them.
    pub keys: Vec<i32>,
    pub shift: bool,
    /// 00403190(3): the wheel's lZ this frame.
    pub wheel: i32,
    /// 00403520(0x1D, 1): Escape went down this frame (scan code 1, 004035A8).
    pub escape: bool,
}

/// IDA: 0057F7A0's answers for the keys the menus read.
pub const KEY_LEFT: i32 = -2147483647;
pub const KEY_RIGHT: i32 = -2147483646;
pub const KEY_UP: i32 = -2147483645;
pub const KEY_DOWN: i32 = -2147483644;
pub const KEY_ENTER: i32 = -2147483640;

/// IDA: N/A host helper. Completion: 100%.
fn manager<'a>(cx: &'a mut InterfaceCx) -> bevy_ecs::prelude::Mut<'a, InterfaceManager> {
    cx.world.resource_mut::<InterfaceManager>()
}

/// IDA: 00583CF0 TimedTransition_Remove. Completion: 100%.
pub fn timed_transition_remove(cx: &mut InterfaceCx, target: TransitionTarget) {
    let mut manager = manager(cx);
    if let Some(block) = manager.block_130.as_mut()
        && let Some(index) = block.nodes.iter().position(|node| node.target_00 == target)
    {
        block.nodes.remove(index);
    }
}

/// IDA: 00584300 TimedTransition_Start: the target's old timer removed, then a node with elapsed 0, duration max(0, seconds), appended. `fld`-compare `> 0.0`, so a NaN stores 0. Completion: 100%.
pub fn timed_transition_start(cx: &mut InterfaceCx, target: TransitionTarget, seconds: f32) {
    timed_transition_remove(cx, target);
    let duration = if seconds > 0.0 { seconds } else { 0.0 };
    if let Some(block) = manager(cx).block_130.as_mut() {
        block.nodes.push(TimedTransitionNode {
            elapsed_04: 0.0,
            duration_08: duration,
            target_00: target,
        });
    }
}

/// IDA: 00583D50 TimedTransition_GetProgress: 1.0 when absent, -1.0 for a duration of 0 or below, else elapsed / duration clamped to 0..1. Completion: 100%.
#[must_use]
pub fn timed_transition_progress(cx: &InterfaceCx, target: TransitionTarget) -> f32 {
    let Some(node) = cx
        .world
        .get_resource::<InterfaceManager>()
        .and_then(|manager| manager.block_130.as_ref())
        .and_then(|block| block.nodes.iter().find(|node| node.target_00 == target))
    else {
        return 1.0;
    };
    if node.duration_08 <= 0.0 {
        return -1.0;
    }
    let progress = node.elapsed_04 / node.duration_08;
    // The exe tests `<= 1.0` and then `< 0.0`: the first adds nothing.
    if progress < 0.0 {
        return 0.0;
    }
    if progress <= 1.0 { progress } else { 1.0 }
}

/// IDA: 00583C30 InterfaceManager_AdvanceTimedTransitions(g_timeInfo.frameTime): elapsed + frame time at 24 bits; a node whose ratio reaches 1 goes. Completion: 100%.
pub fn interface_advance_timed_transitions(cx: &mut InterfaceCx, frame_time: f32) {
    if let Some(block) = manager(cx).block_130.as_mut() {
        block.nodes.retain_mut(|node| {
            node.elapsed_04 += frame_time;
            let ratio = node.elapsed_04 / node.duration_08;
            // 00583C5E..00583C99: only a ratio of 1 or more goes. An unordered one stays: 0 / 0, a zero duration in a zero frame time.
            ratio < 1.0 || ratio.is_nan()
        });
    }
}

/// IDA: 00589980 Tile_TransitionNumericTraitTo(trait, from, to, seconds): an equal destination keeps the running transition, a different one on the trait goes, and a new one starts when `to` differs from the trait. 00588A70 TileTraitTransition_ctor starts its timer, prepends it, and sets flag 0x80. Completion: 100%.
pub fn tile_transition_numeric_trait_to(
    cx: &mut InterfaceCx,
    tile: TileId,
    trait_id: u16,
    from: f32,
    to: f32,
    seconds: f32,
) {
    let list = &mut cx.engine.tile_mut(tile).transitions_28;
    if list
        .iter()
        .any(|running| running.trait_04 == trait_id && running.to_0c == to)
    {
        return;
    }
    list.retain(|running| running.trait_04 != trait_id);
    if to == tile_numeric_trait(cx, tile, trait_id) {
        return;
    }
    let id = cx.engine.next_transition_id;
    cx.engine.next_transition_id += 1;
    timed_transition_start(cx, TransitionTarget::Trait(id), seconds);
    let tile_data = cx.engine.tile_mut(tile);
    tile_data.transitions_28.insert(
        0,
        TileTraitTransition {
            id,
            trait_04: trait_id,
            from_08: from,
            to_0c: to,
            duration_10: seconds,
        },
    );
    tile_data.flags_2c |= 0x80;
}

/// IDA: 0058E870's +0x28 arm with 0058D960 TileTraitTransition_Update: the value is from + (to - from) * progress; flag 0x80 stays while any runs. A transition at progress 1 stores `to` and leaves the list, and the walk starts over from the head. Completion: 100%.
pub fn tile_run_trait_transitions(cx: &mut InterfaceCx, tile: TileId) -> Result<(), GameError> {
    'restart: loop {
        for transition in cx.engine.tile(tile).transitions_28.clone() {
            let progress = timed_transition_progress(cx, TransitionTarget::Trait(transition.id));
            let number = (transition.to_0c - transition.from_08) * progress + transition.from_08;
            tile_set_numeric_trait(cx, tile, transition.trait_04, number)?;
            cx.engine.tile_mut(tile).flags_2c |= 0x80;
            if progress == 1.0 {
                tile_set_numeric_trait(cx, tile, transition.trait_04, transition.to_0c)?;
                cx.engine
                    .tile_mut(tile)
                    .transitions_28
                    .retain(|running| running.id != transition.id);
                continue 'restart;
            }
        }
        return Ok(());
    }
}

/// IDA: 0057EA20(object, progress, inherited) over a tile and its node's children. A tile with disablefade 2 only shows or hides. Otherwise the alpha is min(trait alpha / 255 * progress, trait alpha / 255) and 0 below 0.0001 progress, on the vertex colours of a shape that has them, or on its material. A tile whose ID is 9000 stops the recursion. Completion: 100% on the D3 records.
pub fn interface_apply_fade(
    cx: &mut InterfaceCx,
    tile: TileId,
    progress: f32,
) -> Result<(), GameError> {
    if tile_numeric_trait(cx, tile, TRAIT_DISABLE_FADE) == 2.0 {
        let visible = if progress.abs() >= 1.0 { 2.0 } else { 1.0 };
        return tile_set_numeric_trait(cx, tile, TRAIT_VISIBLE, visible);
    }
    let number = tile_numeric_trait(cx, tile, TRAIT_ALPHA) / 255.0;
    let alpha = if progress.abs() >= 0.000_1_f32 {
        let product = number * progress;
        let bounded = if product >= number { number } else { product };
        if bounded >= 0.0 { bounded } else { 0.0 }
    } else {
        0.0
    };
    if let Some(record) = cx.engine.tile_mut(tile).scene_24.as_mut() {
        for geometry in &mut record.geometry {
            if geometry.has_vertex_colors {
                geometry.vertex_color[3] = alpha;
                geometry.data_flags_2e |= 4;
            } else if geometry.has_material {
                geometry.alpha = alpha;
                geometry.material_revision_54 += 1;
            }
        }
    }
    if tile_numeric_trait(cx, tile, TRAIT_ID) == 9000.0 {
        return Ok(());
    }
    for child in cx.engine.tile(tile).children_30.clone() {
        interface_apply_fade(cx, child, progress)?;
    }
    Ok(())
}

/// IDA: 00584390 Menu_BeginFadeIn: fadein, else fadeout when it is 0.0; 0.25 without a tile; the timer, state FadingIn, and 00583C30. Completion: 100%.
pub fn menu_begin_fade_in(cx: &mut InterfaceCx, menu: MenuId, frame_time: f32) {
    let seconds = cx.engine.menu(menu).tile_04.map_or(0.25, |tile| {
        let fade_in = tile_numeric_trait(cx, tile, TRAIT_FADE_IN);
        if fade_in == 0.0 {
            tile_numeric_trait(cx, tile, TRAIT_FADE_OUT)
        } else {
            fade_in
        }
    });
    timed_transition_start(cx, TransitionTarget::Menu(menu), seconds);
    cx.engine.menu_mut(menu).state_24 = MENU_STATE_FADING_IN;
    interface_advance_timed_transitions(cx, frame_time);
}

/// IDA: 00585190 Menu_Enable(skipFadeIn): 0058FBA0, 0057EA20(node, 0, 0), visible 1, 00584390 unless skipped, 0057F9F0(null, mouseover, 0), 00584980. Completion: 100%.
pub fn menu_enable(
    cx: &mut InterfaceCx,
    menu: MenuId,
    skip_fade_in: bool,
    frame_time: f32,
) -> Result<(), GameError> {
    let Some(tile) = cx.engine.menu(menu).tile_04 else {
        return Ok(());
    };
    tile_update_scene_graph(cx, tile, false)?;
    interface_apply_fade(cx, tile, 0.0)?;
    tile_set_numeric_trait(cx, tile, TRAIT_VISIBLE, 1.0)?;
    if !skip_fade_in {
        menu_begin_fade_in(cx, menu, frame_time);
    }
    interface_dispatch_pointer_event(cx, None, TRAIT_MOUSEOVER)?;
    menu_update_cursor_depth(cx)?;
    Ok(())
}

/// IDA: 00584740 Menu_BeginFadeOutAndRemoveMode: only from visible 2: the timer, state FadingOut; with a mode and destroyOnClose 2, 0057CFE0(mode, 0) and 0057F9F0(null, mouseover, 0); then 00583C30. Completion: 100%.
pub fn menu_begin_fade_out(
    cx: &mut InterfaceCx,
    menu: MenuId,
    frame_time: f32,
) -> Result<(), GameError> {
    let Some(tile) = cx.engine.menu(menu).tile_04 else {
        return Ok(());
    };
    if tile_numeric_trait(cx, tile, TRAIT_VISIBLE) != 2.0 {
        return Ok(());
    }
    let mut seconds = tile_numeric_trait(cx, tile, TRAIT_FADE_IN);
    if seconds == 0.0 {
        seconds = tile_numeric_trait(cx, tile, TRAIT_FADE_OUT);
    }
    timed_transition_start(cx, TransitionTarget::Menu(menu), seconds);
    let mode = cx.engine.menu(menu).menu_mode_14;
    cx.engine.menu_mut(menu).state_24 = MENU_STATE_FADING_OUT;
    if mode != 0 && tile_numeric_trait(cx, tile, TRAIT_DESTROY_ON_CLOSE) == 2.0 {
        interface_remove_menu_mode(cx, mode, false);
        interface_dispatch_pointer_event(cx, None, TRAIT_MOUSEOVER)?;
    }
    interface_advance_timed_transitions(cx, frame_time);
    Ok(())
}

/// IDA: 0057CFE0 InterfaceManager_RemoveMenuMode(mode, force). Without force, a later entry above 1001 blocks (-2), and -1 when the mode is absent or the stack empty. The compaction moves one destination slot at a time, so adjacent duplicates can remain. Removing the last sets status 4 and 00B42D54 unless the mode is Generic or Message. Completion: 100% except the console arm, a host stop by return: mode 3 isn't reached.
pub fn interface_remove_menu_mode(cx: &mut InterfaceCx, mode: u32, force: bool) -> i32 {
    let mut stack = manager(cx).menu_mode_stack_0e0;
    if stack[0] == 0 {
        return -1;
    }
    if !force {
        let mut seen = false;
        let mut index = 0;
        while index < 10 {
            if stack[index] == mode {
                seen = true;
            }
            if seen && index + 1 < 10 && stack[index + 1] > 1001 {
                // The break with the count below ten.
                return if mode == 3 { -3 } else { -2 };
            }
            index += 1;
        }
        if !seen {
            return -1;
        }
    }
    let mut remaining = 0usize;
    let mut read = 0usize;
    for write in 0..10 {
        if stack[write] == mode {
            read += 1;
        }
        let value = stack.get(read).copied().unwrap_or(0);
        stack[write] = value;
        if value == 0 {
            break;
        }
        remaining += 1;
        read += 1;
    }
    manager(cx).menu_mode_stack_0e0 = stack;
    if remaining == 0 {
        manager(cx).menu_loading_status_008 = 4;
        cx.world.resource_mut::<MenuModeChanged>().0 = u8::from(mode != 1011 && mode != 1001);
        return 0;
    }
    if remaining == 1 && (stack[0] == 1011 || stack[0] == 1001) {
        cx.world.resource_mut::<MenuModeChanged>().0 = 1;
    }
    stack[remaining - 1] as i32
}

/// IDA: 0058FAB0 Tile_Destructor, with its 0058DA70 release (not read: a host reading of what it must do): the children first, the active pointers cleared, the node off its parent, the tile off its parent's list. The arena keeps the slot, a recorded substitution, and values that other tiles' expressions read stay readable. Completion: 100% of what's visible.
pub fn tile_destroy(cx: &mut InterfaceCx, tile: TileId) -> Result<(), GameError> {
    for child in cx.engine.tile(tile).children_30.clone() {
        tile_destroy(cx, child)?;
    }
    {
        let mut manager = manager(cx);
        if manager.active_tile_098 == Some(tile) {
            manager.active_tile_098 = None;
            manager.active_menu_09c = None;
        }
        if manager.alt_active_tile_088 == Some(tile) {
            manager.alt_active_tile_088 = None;
        }
    }
    for transition in std::mem::take(&mut cx.engine.tile_mut(tile).transitions_28) {
        timed_transition_remove(cx, TransitionTarget::Trait(transition.id));
    }
    release_node(cx, tile);
    tile_set_parent(cx, tile, None, None)?;
    cx.engine.tile_mut(tile).released_04 = 1;
    Ok(())
}

/// IDA: 005853F0 / 00585330 Menu_Destructor (the class's own runs first, of which the port has MainMenu's 005B5870): 0057CFE0(mode, 1) with a mode; 00B13970[id - 1001] cleared; the tile deleted with +0x44 null. 00583CF0 for its timer: nothing in the destructor removes it, and a dead pointer's node only ever times out, so the arena's slot is as harmless. Completion: 100% of the order.
pub fn menu_destroy(cx: &mut InterfaceCx, menu: MenuId) -> Result<(), GameError> {
    if matches!(cx.engine.menu(menu).class, MenuClass::Main(_)) {
        crate::tes::menus::main_menu_destructor(cx);
    }
    let mode = cx.engine.menu(menu).menu_mode_14;
    if mode != 0 {
        interface_remove_menu_mode(cx, mode, true);
    }
    let id = cx.engine.menu(menu).id_20;
    if let Some(slot) = cx.engine.tile_menus.get_mut(id.wrapping_sub(1001) as usize) {
        *slot = None;
    }
    if let Some(reads) = cx
        .world
        .get_resource::<super::interface_manager::InterfaceMovieReads>()
    {
        reads.set_tile_menu(id, false);
    }
    if let Some(tile) = cx.engine.menu(menu).tile_04 {
        cx.engine.tile_mut(tile).menu_44 = None;
        tile_destroy(cx, tile)?;
    }
    let owner = cx.engine.menu_mut(menu);
    owner.tile_04 = None;
    owner.templates_08.clear();
    Ok(())
}

/// IDA: 00583E60: 00583C30, then status 3 with no transition running becomes 5: buffered events go, the cursor node shows with visible 2 and cursor.dds, 0058E870(cursor); 0057D940(0) with no reticle does nothing here; 005A4980, 005A6040 return with no HUD; +0x88 = 0. Status 5 becomes 2. Completion: 100% with no HUD and no console.
pub fn interface_advance_frame(cx: &mut InterfaceCx, frame_time: f32) -> Result<(), GameError> {
    manager(cx).frame_seconds_host = frame_time;
    interface_advance_timed_transitions(cx, frame_time);
    let (status, running, cursor) = {
        let manager = manager(cx);
        (
            manager.menu_loading_status_008,
            manager.menu_transition_in_progress_009,
            manager.cursor_01c,
        )
    };
    if status == 3 && running == 0 {
        manager(cx).menu_loading_status_008 = 5;
        if let Some(cursor) = cursor {
            if let Some(node) = tile_scene_node(cx, cursor) {
                cx.world
                    .get_mut::<SceneNodePayload>(node)
                    .expect("a tile node has a payload")
                    .effective_av_flags &= !1;
            }
            tile_set_numeric_trait(cx, cursor, TRAIT_VISIBLE, 2.0)?;
            tile_set_string_trait(cx, cursor, TRAIT_FILENAME, b"Menus\\Misc\\cursor.dds")?;
            interface_update_tile_scene(cx, cursor)?;
        }
        manager(cx).alt_active_tile_088 = None;
    } else if status == 5 {
        manager(cx).menu_loading_status_008 = 2;
    }
    Ok(())
}

/// IDA: 00583F40 InterfaceManager_UpdateMenuVisibilityTransitions: over the menu root's children, FadingIn and FadingOut advance. A fade's end destroys a tile with destroyOnClose 2 and hides any other. Status 4 with nothing running restores the game's input (cursor hidden, status 1). 00B42D54's blend value is kept for a later reader. Completion: 100% with no Dialog or Generic menu and no first-person node: those arms host stop.
#[allow(clippy::too_many_lines)]
pub fn interface_update_menu_visibility(cx: &mut InterfaceCx) -> Result<(), GameError> {
    let root = manager(cx).menu_root_068;
    manager(cx).menu_transition_in_progress_009 = 0;
    let children = root.map_or_else(Vec::new, |root| cx.engine.tile(root).children_30.clone());
    for child in children {
        let Some(menu) = cx.engine.containing_menu(child) else {
            continue;
        };
        let Some(tile) = cx.engine.menu(menu).tile_04 else {
            continue;
        };
        let progress = timed_transition_progress(cx, TransitionTarget::Menu(menu));
        let absolute = progress.abs();
        let state = cx.engine.menu(menu).state_24;
        let stack = manager(cx).menu_mode_stack_0e0;
        let top = top_stacked_menu_mode(&stack);
        let (first, second) = (stack[0], stack[1]);
        if state == MENU_STATE_FADING_IN {
            let opening = (top != 0 && first == top && top != 1011 && top != 1001)
                || (second != 0 && second == top && (first == 1011 || first == 1001));
            tile_set_numeric_trait(cx, tile, TRAIT_VISIBLE, 2.0)?;
            if absolute == 1.0 {
                interface_apply_fade(cx, tile, 1.0)?;
                cx.engine.menu_mut(menu).state_24 = MENU_STATE_VISIBLE;
                if opening {
                    let mut changed = cx.world.resource_mut::<MenuModeChanged>();
                    if changed.0 != 0 {
                        changed.0 = 0;
                        manager(cx)
                            .block_130
                            .as_mut()
                            .expect("the ctor made it")
                            .blend_progress = 1.0;
                    }
                }
            } else {
                manager(cx).menu_transition_in_progress_009 = 1;
                interface_apply_fade(cx, tile, progress)?;
                if opening && cx.world.resource::<MenuModeChanged>().0 != 0 {
                    manager(cx)
                        .block_130
                        .as_mut()
                        .expect("the ctor made it")
                        .blend_progress = progress;
                }
            }
        } else if state == MENU_STATE_FADING_OUT {
            let controls_blend =
                first == 0 && top == 0 || (second == 0 && (first == 1011 || first == 1001));
            if absolute != 1.0 {
                manager(cx).menu_transition_in_progress_009 = 1;
                tile_set_numeric_trait(cx, tile, TRAIT_VISIBLE, 2.0)?;
                let remaining = 1.0 - progress;
                interface_apply_fade(cx, tile, remaining)?;
                if controls_blend && cx.world.resource::<MenuModeChanged>().0 != 0 {
                    manager(cx)
                        .block_130
                        .as_mut()
                        .expect("the ctor made it")
                        .blend_progress = remaining;
                }
                continue;
            }
            cx.engine.menu_mut(menu).state_24 = MENU_STATE_HIDDEN;
            if tile_numeric_trait(cx, tile, TRAIT_DESTROY_ON_CLOSE) != 2.0 {
                tile_set_numeric_trait(cx, tile, TRAIT_VISIBLE, 1.0)?;
                interface_apply_fade(cx, tile, 0.0)?;
                if controls_blend {
                    let mut changed = cx.world.resource_mut::<MenuModeChanged>();
                    if changed.0 != 0 {
                        changed.0 = 0;
                        manager(cx)
                            .block_130
                            .as_mut()
                            .expect("the ctor made it")
                            .blend_progress = 0.0;
                    }
                }
                continue;
            }
            let id = cx.engine.menu(menu).id();
            if id == 1009 || id == 1011 {
                return Err(GameError::unclassified(
                    0x00583F40,
                    "00583F40: the Dialog and Generic fade ends are carried",
                ));
            }
            menu_destroy(cx, menu)?;
            // 00584257..00584260: the walk ends here, as the list has changed. The next pass takes the rest.
            break;
        }
    }
    // 00583F40's end.
    let (status, running, cursor) = {
        let manager = manager(cx);
        (
            manager.menu_loading_status_008,
            manager.menu_transition_in_progress_009,
            manager.cursor_01c,
        )
    };
    let tes_override = cx
        .world
        .get_resource::<TesGlobals>()
        .and_then(|globals| globals.tes.as_ref())
        .is_some_and(|tes| tes.cell_test_active_51 != 0 || tes.byte_52 != 0);
    if status == 4 && (running == 0 || tes_override) {
        manager(cx).menu_loading_status_008 = 1;
        if let Some(cursor) = cursor {
            if let Some(node) = tile_scene_node(cx, cursor) {
                cx.world
                    .get_mut::<SceneNodePayload>(node)
                    .expect("a tile node has a payload")
                    .effective_av_flags |= 1;
            }
            tile_set_numeric_trait(cx, cursor, TRAIT_VISIBLE, 1.0)?;
            interface_update_tile_scene(cx, cursor)?;
        }
        // 005842B7 0057D940(1). 005842D5: 005A6040(0, 0) with the player's third-person root, null on the default path.
        interface_update_reticle(cx, 1)?;
        let has_root = the_player(cx.world)
            .and_then(|player| cx.world.get::<TesObjectRefr>(player))
            .is_some_and(|refr| refr.loaded_3d_3c != 0);
        if has_root {
            let frame_time = manager(cx).frame_seconds_host;
            hud_update_visibility(cx, false, false, frame_time)?;
        }
        manager(cx).input_modifier_flags = 0;
    }
    Ok(())
}

/// IDA: 0057F490(x, y), the window's pointer as 0..1 of the client. The cursor node goes to x_v * w_v - w_v / 2, y_v / 2 - y * h_v (the C++ doesn't have this). +0x2C..+0x34 and +0xB9 change when the point in pixels does; the hot spot is 3 virtual units right of the node. In status 2 the cursor shows. Completion: 100%.
pub fn interface_place_cursor(cx: &mut InterfaceCx, x: f32, y: f32) -> Result<(), GameError> {
    let Some(cursor) = manager(cx).cursor_01c else {
        return Ok(());
    };
    let Some(node) = tile_scene_node(cx, cursor) else {
        return Ok(());
    };
    let virtual_width = ui_virtual_width(cx.settings);
    let virtual_height = ui_virtual_height(cx.settings);
    let width = cx.settings.renderer_width as f32;
    let height = cx.settings.renderer_height as f32;
    let mut translation = cx
        .world
        .get::<SceneNodePayload>(node)
        .expect("a tile node has a payload")
        .local_transform
        .translation;
    translation[0] = virtual_width * x - virtual_width * 0.5;
    translation[2] = 0.5 * virtual_height - virtual_height * y;
    cx.world
        .get_mut::<SceneNodePayload>(node)
        .expect("a tile node has a payload")
        .local_transform
        .translation = translation;
    let status = update_scene_object(cx.world, node, 0.0, true, cx.runtime)?;
    if status.status != SceneUpdateStatus::Updated {
        return Err(GameError::unclassified(
            0x0057F490,
            "0057F490: NiAVObject_Update reported an unsupported state",
        ));
    }
    let pixel_x = width * 0.5 + width / virtual_width * (translation[0] + 3.0);
    let pixel_y = 0.5 * height - height / virtual_height * translation[2];
    let depth = tile_numeric_trait(cx, cursor, TRAIT_DEPTH);
    let moved = {
        let mut manager = manager(cx);
        let was = manager.cursor_previous_b3a6e8;
        let state = &mut manager.cursor_state_020;
        state[0] = translation[0];
        state[1] = translation[1];
        state[2] = translation[2];
        if was != [pixel_x, pixel_y] {
            state[3] = pixel_x;
            state[4] = depth * -0.008;
            state[5] = pixel_y;
            manager.cursor_previous_b3a6e8 = [pixel_x, pixel_y];
            manager.cursor_moved_0b9 = 1;
            true
        } else {
            false
        }
    };
    if moved && manager(cx).menu_loading_status_008 == 2 {
        cx.world
            .get_mut::<SceneNodePayload>(node)
            .expect("a tile node has a payload")
            .effective_av_flags &= !1;
        tile_set_numeric_trait(cx, cursor, TRAIT_VISIBLE, 2.0)?;
    }
    Ok(())
}

/// IDA: 005893B0 Tile_IsSceneHierarchyNotAppCulled. Completion: 100%.
#[must_use]
pub fn tile_hierarchy_visible(cx: &InterfaceCx, tile: TileId) -> bool {
    let mut current = Some(tile);
    while let Some(id) = current {
        if let Some(node) = tile_scene_node(cx, id)
            && cx
                .world
                .get::<SceneNodePayload>(node)
                .is_some_and(|payload| payload.effective_av_flags & 1 != 0)
        {
            return false;
        }
        current = cx.engine.tile(id).parent;
    }
    true
}

/// IDA: 0057F9F0 InterfaceManager_DispatchTilePointerEvent's mouseover and null arms, and its click arm: a clicked tile's sound, click counts, `clicked` 1 then 0, the class's vt+0xC, 0058FBA0, and a mouseout when the focus has gone. The &xitem; arm of 4086 (list items) moves the list's scroll. Completion: 100% of the arms.
pub fn interface_dispatch_pointer_event(
    cx: &mut InterfaceCx,
    target: Option<TileId>,
    event: u16,
) -> Result<(), GameError> {
    if let Some(item) = target
        && tile_numeric_trait(cx, item, TRAIT_XLIST) == 109.0
        && let Some(parent) = cx.engine.tile(item).parent
    {
        // 0058E3B0(parent, 0xFF5): the tile the parent's xscroll names.
        if let (Some(list), _) = navigation_target(cx, parent, TRAIT_XSCROLL)? {
            tile_set_numeric_trait(cx, list, TRAIT_USER5, -999_999.0)?;
            let scroll = tile_numeric_trait(cx, item, TRAIT_XSCROLL);
            tile_set_numeric_trait(cx, list, TRAIT_USER5, scroll)?;
            tile_set_numeric_trait(cx, list, TRAIT_USER5, 0.0)?;
        }
    }
    let alt = manager(cx).alt_active_tile_088;
    if let Some(alt) = alt
        && Some(alt) != target
        && tile_numeric_trait(cx, alt, TRAIT_XDEFAULT) > 0.0
    {
        let priority = {
            let mut manager = manager(cx);
            manager.default_selection_priority_08c += 1;
            manager.default_selection_priority_08c
        };
        tile_set_numeric_trait(cx, alt, TRAIT_XDEFAULT, priority as f32)?;
    }
    let Some(target) = target else {
        if let Some(alt) = alt {
            mouseout(cx, alt)?;
        }
        let mut manager = manager(cx);
        manager.alt_active_tile_088 = None;
        manager.active_menu_09c = None;
        manager.active_tile_098 = None;
        return Ok(());
    };
    if let Some(active) = manager(cx).active_tile_098 {
        mouseout(cx, active)?;
    }
    {
        let mut manager = manager(cx);
        manager.active_tile_098 = None;
        manager.active_menu_09c = None;
    }
    if event == TRAIT_MOUSEOVER {
        let alt = manager(cx).alt_active_tile_088;
        if alt != Some(target) {
            if let Some(alt) = alt {
                mouseout(cx, alt)?;
            }
            manager(cx).alt_active_tile_088 = Some(target);
            if tile_numeric_trait(cx, target, TRAIT_XDEFAULT) > 0.0 {
                let priority = {
                    let mut manager = manager(cx);
                    manager.default_selection_priority_08c += 1;
                    manager.default_selection_priority_08c
                };
                tile_set_numeric_trait(cx, target, TRAIT_XDEFAULT, priority as f32)?;
            }
            tile_set_numeric_trait(cx, target, TRAIT_MOUSEOVER, 1.0)?;
            if let Some(menu) = cx.engine.containing_menu(target) {
                menu_handle_mouseover(cx, menu, tile_id_of(cx, target), Some(target))?;
            }
        }
        return Ok(());
    }
    if event == TRAIT_CLICKED {
        tile_click(cx, target, target)?;
        tile_update_scene_graph(cx, target, false)?;
        let alt = manager(cx).alt_active_tile_088;
        if let Some(alt) = alt
            && (!tile_hierarchy_visible(cx, alt)
                || tile_numeric_trait(cx, alt, TRAIT_TARGET) != 2.0)
        {
            mouseout(cx, alt)?;
            let mut manager = manager(cx);
            manager.alt_active_tile_088 = None;
            manager.active_menu_09c = None;
            manager.active_tile_098 = None;
        }
    }
    Ok(())
}

/// IDA: N/A the ftol of a tile's id trait, the first argument of the class's Handle slots.
fn tile_id_of(cx: &InterfaceCx, tile: TileId) -> i32 {
    x87::ftol(tile_numeric_trait(cx, tile, TRAIT_ID))
}

/// IDA: the block 0057F9F0 and 005821F0 share: `mouseover` 0, the class's vt+0x14 HandleMouseout(id, tile). Completion: 100%.
fn mouseout(cx: &mut InterfaceCx, tile: TileId) -> Result<(), GameError> {
    tile_set_numeric_trait(cx, tile, TRAIT_MOUSEOVER, 0.0)?;
    if let Some(menu) = cx.engine.containing_menu(tile) {
        menu_handle_mouseout(cx, menu, tile_id_of(cx, tile), Some(tile))?;
    }
    Ok(())
}

/// IDA: the block 0057F9F0, 005821F0 and 00580BA0 share: the tile's clicksound through 0057DE50, clickcountafter = before + 1, clicked 1, then 0, and the class's vt+0xC HandleClick(id, `reported`). Completion: 100%.
fn tile_click(cx: &mut InterfaceCx, tile: TileId, reported: TileId) -> Result<(), GameError> {
    let sound = tile_numeric_trait(cx, tile, TRAIT_CLICK_SOUND);
    if sound != 0.0 {
        cx.host.play_ui_sound(x87::ftol(sound) as u32);
    }
    let count = tile_numeric_trait(cx, tile, TRAIT_CLICK_COUNT_BEFORE) + 1.0;
    tile_set_numeric_trait(cx, tile, TRAIT_CLICK_COUNT_AFTER, count)?;
    tile_set_numeric_trait(cx, tile, TRAIT_CLICKED, 1.0)?;
    tile_set_numeric_trait(cx, tile, TRAIT_CLICKED, 0.0)?;
    if let Some(menu) = cx.engine.containing_menu(tile) {
        menu_handle_click(cx, menu, tile_id_of(cx, tile), Some(reported))?;
    }
    Ok(())
}

/// IDA: 0057CF60 InterfaceManager_GetTopStackedMenuMode: the entry before the first 0, slot 9 when all ten are set, 0 when empty. Completion: 100%.
#[must_use]
pub fn top_stacked_menu_mode(stack: &[u32; 10]) -> u32 {
    match stack.iter().position(|&mode| mode == 0) {
        None => stack[9],
        Some(0) => 0,
        Some(length) => stack[length - 1],
    }
}

/// IDA: 005877D0: over the menu root's children, the menu with the greatest depth whose tile isn't visible 1, isn't stacking 6008 and isn't fading out; a tie keeps the earlier. Completion: 100%.
#[must_use]
pub fn interface_top_menu(cx: &InterfaceCx) -> Option<MenuId> {
    let root = cx.world.resource::<InterfaceManager>().menu_root_068?;
    let mut deepest = -100_000_000.0f32;
    let mut best = None;
    for &child in &cx.engine.tile(root).children_30 {
        let Some(menu) = cx.engine.containing_menu(child) else {
            continue;
        };
        let owner = cx.engine.menu(menu);
        let Some(tile) = owner.tile_04 else {
            continue;
        };
        let depth = tile_numeric_trait(cx, tile, TRAIT_DEPTH);
        if tile_numeric_trait(cx, tile, TRAIT_VISIBLE) != 1.0
            && deepest < depth
            && tile_numeric_trait(cx, tile, TRAIT_STACKING) != 6008.0
            && owner.state_24 != MENU_STATE_FADING_OUT
        {
            deepest = depth;
            best = Some(menu);
        }
    }
    best
}

/// IDA: 0058E7D0: the scroll of the list an item is in: the parent's xscroll tile takes user5 = -999999, then the item's xscroll, then 0. Completion: 100%.
fn scroll_list_to_item(cx: &mut InterfaceCx, item: TileId) -> Result<(), GameError> {
    let Some(parent) = cx.engine.tile(item).parent else {
        return Ok(());
    };
    if let (Some(list), _) = navigation_target(cx, parent, TRAIT_XSCROLL)? {
        tile_set_numeric_trait(cx, list, TRAIT_USER5, -999_999.0)?;
        let scroll = tile_numeric_trait(cx, item, TRAIT_XSCROLL);
        tile_set_numeric_trait(cx, list, TRAIT_USER5, scroll)?;
        tile_set_numeric_trait(cx, list, TRAIT_USER5, 0.0)?;
    }
    Ok(())
}

// IDA: N/A the ftol of a tile's listindex.
fn list_index(cx: &InterfaceCx, tile: TileId) -> i32 {
    x87::ftol(tile_numeric_trait(cx, tile, 4010))
}

/// IDA: 0058E3B0(tile, trait, &event): the tile's or the nearest ancestor's value for the trait. 105 and 104 pick a sibling by listindex (not visible 1, with xdefault), and an empty search moves up to the parent. 106 and 107 pick a child by listindex, else clear xdefault and take 0057DA90 from the top tile. Any other value is its first action 2027's source: the dependency chain's head names the source value, whose tile comes back with its trait as the event. Completion: 100%.
pub fn navigation_target(
    cx: &mut InterfaceCx,
    tile: TileId,
    trait_id: u16,
) -> Result<(Option<TileId>, Option<u16>), GameError> {
    let mut current = tile;
    loop {
        let value = cx
            .engine
            .tile(current)
            .values_14
            .iter()
            .copied()
            .find(|&value| cx.engine.value(value).trait_18 == trait_id);
        let Some(value) = value else {
            let Some(parent) = cx.engine.tile(current).parent else {
                return Ok((None, None));
            };
            current = parent;
            continue;
        };
        let number = cx.engine.value(value).number_04;
        if number == 105.0 || number == 104.0 {
            let Some(parent) = cx.engine.tile(current).parent else {
                return Ok((None, None));
            };
            let own = list_index(cx, current);
            let mut best = None;
            let mut limit = if number == 105.0 { i32::MAX } else { i32::MIN };
            for &child in &cx.engine.tile(parent).children_30 {
                let hidden = find_value(cx, child, TRAIT_VISIBLE)
                    .is_some_and(|visible| cx.engine.value(visible).number_04 == 1.0);
                if hidden || find_value(cx, child, TRAIT_XDEFAULT).is_none() {
                    continue;
                }
                let index = list_index(cx, child);
                let closer = if number == 105.0 {
                    index > own && index < limit
                } else {
                    index < own && index > limit
                };
                if closer {
                    best = Some(child);
                    limit = index;
                }
            }
            if let Some(best) = best {
                scroll_list_to_item(cx, best)?;
                return Ok((Some(best), None));
            }
            current = parent;
            continue;
        }
        if number == 106.0 || number == 107.0 {
            let mut best = None;
            let mut limit = if number == 106.0 { i32::MAX } else { i32::MIN };
            for &child in &cx.engine.tile(current).children_30 {
                if tile_numeric_trait(cx, child, TRAIT_VISIBLE) != 1.0
                    && find_value(cx, child, TRAIT_XDEFAULT).is_some()
                {
                    let index = list_index(cx, child);
                    let closer = if number == 106.0 {
                        index < limit
                    } else {
                        index > limit
                    };
                    if closer {
                        best = Some(child);
                        limit = index;
                    }
                }
            }
            if let Some(best) = best {
                scroll_list_to_item(cx, best)?;
                return Ok((Some(best), None));
            }
            if find_value(cx, current, TRAIT_XDEFAULT).is_some() {
                tile_set_numeric_trait(cx, current, TRAIT_XDEFAULT, 0.0)?;
            }
            let mut top = current;
            while let Some(parent) = cx.engine.tile(top).parent {
                if cx.engine.tile(parent).parent.is_none() {
                    break;
                }
                top = parent;
            }
            let mut best_value = i32::MIN;
            return Ok((
                interface_default_selection(cx, Some(top), &mut best_value),
                None,
            ));
        }
        // The first action 2027 with no high word, then +0x10 back to the chain's head, whose +0x08 is the source value.
        let mut node = Some(cx.engine.value(value).expression_head_10);
        while let Some(id) = node {
            let expression = cx.engine.expression(id);
            if expression.opcode_0c == 0x7EB && expression.high_0e == 0 {
                break;
            }
            node = expression.next_04;
        }
        let Some(mut link) = node else {
            return Ok((None, None));
        };
        while let Some(previous) = cx.engine.expression(link).dependency_prev_10 {
            link = previous;
        }
        let Some(source) = cx.engine.expression(link).owner_08 else {
            return Ok((None, None));
        };
        let source = cx.engine.value(source);
        return Ok((source.parent_tile_00, Some(source.trait_18)));
    }
}

/// IDA: 0057FD60 InterfaceManager_SelectDefaultTile: 0057DA90 from the top menu; 0057F9F0(best, mouseover); with a tile the
/// cursor's node is app-culled, its visible 1 and +0xB9 0. Completion: 100%.
/// # Errors
/// A stop in the pointer dispatch.
pub fn interface_select_default_tile(cx: &mut InterfaceCx) -> Result<(), GameError> {
    let mut value = i32::MIN;
    let best = interface_default_selection(cx, None, &mut value);
    interface_dispatch_pointer_event(cx, best, TRAIT_MOUSEOVER)?;
    if best.is_some()
        && let Some(cursor) = manager(cx).cursor_01c
    {
        if let Some(node) = tile_scene_node(cx, cursor) {
            cx.world
                .get_mut::<SceneNodePayload>(node)
                .expect("a tile node has a payload")
                .effective_av_flags |= 1;
        }
        tile_set_numeric_trait(cx, cursor, TRAIT_VISIBLE, 1.0)?;
        manager(cx).cursor_moved_0b9 = 0;
    }
    Ok(())
}

/// IDA: 0057DA90(&best, tile): with no tile, the top menu's, which must be Visible or FadingIn. A hidden tile gives null. Depth first over the children, the highest xdefault wins, a tie going to the lower listindex; then the tile itself if it's a target that shows. xdefault and listindex go through `__ftol2_sse` (57dbda, 57dc0f); the decompiler's float-as-int reading isn't what runs. Completion: 100%.
pub fn interface_default_selection(
    cx: &InterfaceCx,
    start: Option<TileId>,
    best: &mut i32,
) -> Option<TileId> {
    let mut top = *best;
    let mut chosen = None;
    let mut lowest_index = i32::MAX;
    *best = i32::MIN;
    let tile = match start {
        Some(tile) => tile,
        None => {
            let menu = interface_top_menu(cx)?;
            let tile = cx.engine.menu(menu).tile_04?;
            let state = cx.engine.menu(menu).state_24;
            if cx.engine.tile(tile).type_id() == 905
                && state != MENU_STATE_VISIBLE
                && state != MENU_STATE_FADING_IN
            {
                return None;
            }
            tile
        }
    };
    if tile_numeric_trait(cx, tile, TRAIT_VISIBLE) == 1.0 {
        return None;
    }
    for &child in &cx.engine.tile(tile).children_30 {
        let mut value = i32::MIN;
        let Some(found) = interface_default_selection(cx, Some(child), &mut value) else {
            continue;
        };
        if value > top {
            top = value;
            chosen = Some(found);
        } else if value == top
            && let Some(index) = find_value(cx, found, 4010)
        {
            let index = x87::ftol(cx.engine.value(index).number_04);
            if index < lowest_index {
                lowest_index = index;
                chosen = Some(found);
            }
        }
    }
    if tile_numeric_trait(cx, tile, TRAIT_TARGET) == 2.0
        && tile_numeric_trait(cx, tile, TRAIT_VISIBLE) != 1.0
        && let Some(value) = find_value(cx, tile, TRAIT_XDEFAULT)
    {
        let number = x87::ftol(cx.engine.value(value).number_04);
        if number > top {
            *best = number;
            return Some(tile);
        }
        if number == top
            && let Some(index) = find_value(cx, tile, 4010)
            && x87::ftol(cx.engine.value(index).number_04) < lowest_index
        {
            chosen = Some(tile);
        }
    }
    *best = top;
    chosen
}

/// IDA: 005878B0 InterfaceManager_HasVisibleNonFadingMenuByID: the first of the menu root's menus with that ID; visible 1 or fading out says no. Completion: 100%.
pub(crate) fn interface_visible_menu(cx: &InterfaceCx, id: u32) -> Option<MenuId> {
    let root = cx.world.resource::<InterfaceManager>().menu_root_068?;
    let menu = cx.engine.tile(root).children_30.iter().find_map(|&child| {
        let menu = cx.engine.containing_menu(child)?;
        let owner = cx.engine.menu(menu);
        (owner.tile_04.is_some() && owner.id() == id).then_some(menu)
    })?;
    let owner = cx.engine.menu(menu);
    let visible = tile_numeric_trait(cx, owner.tile_04?, TRAIT_VISIBLE);
    (visible != 1.0 && owner.state_24 != MENU_STATE_FADING_OUT).then_some(menu)
}

/// IDA: 00580BA0 InterfaceManager_HandleMenuNavigationAction, the arms a key reaches on these menus. No top menu returns. Actions 1 to 4: +0xB9 = 0, then from +0x88 walk the direction trait (4081, 4082, 4083, 4084) over tiles seen once, stopping at one that shows, is a target and whose xdefault isn't -99; an &xlist; (4086 = 108) enters by its child with the highest xdefault. A walk that comes back or runs out keeps the start. With no +0x88, 0057DA90 from the top menu, and if that finds nothing the top tile's direction is clicked. Action -2 clicks +0x88 (or +0x98, or the top tile), climbing to one that shows. The menu's vt+0x38 is asked first (`menu_slots`): the base 00725DA0 returns 0, and LoadgameMenu's 005AEDF0 only takes action 11, the controller's delete, which no key here sends. Completion: 100% of the two arms. Actions 5 to 16 belong to controllers and the pause key.
#[allow(clippy::too_many_lines)]
pub fn interface_navigate(cx: &mut InterfaceCx, action: i32) -> Result<(), GameError> {
    let Some(top_menu) = interface_top_menu(cx) else {
        return Ok(());
    };
    // 00580BF5: the menu's vt+0x38(action, 1.0) first.
    if menu_handle_navigation(cx, top_menu, action)? {
        return Ok(());
    }
    let top_tile = cx.engine.menu(top_menu).tile_04;
    if action == -2 {
        let mut focus = manager(cx).alt_active_tile_088;
        if focus.is_none() {
            focus = manager(cx).active_tile_098;
        }
        let mut focus = focus.or(top_tile);
        while let Some(tile) = focus {
            if tile_hierarchy_visible(cx, tile) {
                break;
            }
            focus = cx.engine.tile(tile).parent;
        }
        let Some(focus) = focus else {
            return Ok(());
        };
        if tile_numeric_trait(cx, focus, TRAIT_TARGET) == 2.0 {
            tile_click(cx, focus, focus)?;
        }
        return Ok(());
    }
    let direction = match action {
        1 => TRAIT_XUP,
        2 => TRAIT_XDOWN,
        4 => TRAIT_XLEFT,
        3 => TRAIT_XRIGHT,
        _ => return Ok(()),
    };
    manager(cx).cursor_moved_0b9 = 0;
    let Some(start) = manager(cx).alt_active_tile_088 else {
        let mut value = i32::MIN;
        let best = interface_default_selection(cx, None, &mut value);
        interface_dispatch_pointer_event(cx, best, TRAIT_MOUSEOVER)?;
        if best.is_none()
            && let Some(top) = top_tile
            && let (Some(target), _) = navigation_target(cx, top, direction)?
        {
            tile_click(cx, target, target)?;
        }
        return Ok(());
    };
    let mut event = TRAIT_MOUSEOVER;
    let (mut next, source_trait) = navigation_target(cx, start, direction)?;
    if let Some(source_trait) = source_trait {
        event = source_trait;
    }
    let mut seen = vec![start];
    while let Some(candidate) = next {
        let usable = tile_numeric_trait(cx, candidate, TRAIT_VISIBLE) != 1.0
            && tile_numeric_trait(cx, candidate, TRAIT_TARGET) == 2.0
            && tile_numeric_trait(cx, candidate, TRAIT_XDEFAULT) != -99.0;
        if seen.contains(&candidate) || usable {
            if seen.contains(&candidate) && !usable {
                next = None;
            }
            break;
        }
        seen.insert(0, candidate);
        let mut entered = None;
        if tile_numeric_trait(cx, candidate, TRAIT_XLIST) == 108.0 {
            let mut highest = 0;
            for &child in cx.engine.tile(candidate).children_30.iter().rev() {
                if tile_hierarchy_visible(cx, child)
                    && find_value(cx, child, TRAIT_XDEFAULT).is_some()
                {
                    let value = x87::ftol(tile_numeric_trait(cx, child, TRAIT_XDEFAULT));
                    if value > highest {
                        highest = value;
                        entered = Some(child);
                    }
                }
            }
        }
        next = if entered.is_some() {
            entered
        } else {
            let (found, source_trait) = navigation_target(cx, candidate, direction)?;
            if let Some(source_trait) = source_trait {
                event = source_trait;
            }
            found
        };
    }
    match next {
        Some(target) => {
            if target != start && event == TRAIT_MOUSEOVER {
                // The original looks UIMenuFocus up by name (006ADE50); 0057DE50's id 4 is that sound.
                cx.host.play_ui_sound(4);
            }
            interface_dispatch_pointer_event(cx, Some(target), event)
        }
        None => interface_dispatch_pointer_event(cx, Some(start), event),
    }
}

/// IDA: N/A the interface frame's calls that are counted rather than made.
#[derive(bevy_ecs::prelude::Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct InterfaceFrameBoundaries {
    pub calls: std::collections::BTreeMap<u32, u32>,
}

// IDA: N/A a helper, with no address of its own.
pub(crate) fn count_interface_call(cx: &mut InterfaceCx, address: u32) {
    *cx.world
        .get_resource_or_init::<InterfaceFrameBoundaries>()
        .calls
        .entry(address)
        .or_default() += 1;
}

/// IDA: 005821F0 InterfaceManager_UpdateFrame's branch for status 2 and 5: the cursor, the pointer's press, release and hover, then the keys, then the +0xB4 callback a closed MessageMenu left. The rest of the function needs a player HUD, the console or the debug text, none of which a menu screen reaches. Completion: 100% of the branch the screens reach; held-mouse slots vt+0x08 and +0x24, the wheel's vt+0x28 and the Unk_02 tail are the two menus' own, listed in `menus`.
#[allow(clippy::too_many_lines)]
pub fn interface_update_frame(
    cx: &mut InterfaceCx,
    input: &InterfaceInputFrame,
) -> Result<(), GameError> {
    let status = manager(cx).menu_loading_status_008;
    // 00582249 HUDMainMenu_UpdateVisibility(status != 1, 0), first thing, whatever the status. 00582251 005A82D0's two arms test 00B3B350 and 00B3B358, tiles whose writers 005A8260 and 005A8280 aren't reached: `run-h-phase17` holds both null.
    crate::tes::hud_main_menu::hud_update_visibility(cx, status != 1, false, input.frame_time)?;
    if status == 1 {
        // 00582260..00582287. HUDMain is up from Phase 17, so its create is a stop.
        if cx
            .engine
            .tile_menu_by_id(crate::tes::hud_main_menu::HUD_MAIN_MENU_ID)
            .is_none()
        {
            return Err(GameError::unclassified(
                0x00582271,
                "00582271: HUDMainMenu_Create in a running game",
            ));
        }
        if cx.engine.tile_menu_by_id(0x3ED).is_none() {
            crate::tes::hud_info_menu::hud_info_menu_create(cx, input.frame_time)?;
        }
        // 0058228C..005822BD: +0x10C and +0x110, written by what puts the player in jail.
        let (jail, released) = {
            let manager = manager(cx);
            (manager.unknown_10c, manager.unknown_110)
        };
        if jail & 0xFF != 0 || released & 0xFF != 0 {
            return Err(GameError::unclassified(
                0x0058228C,
                "0058228C: the jail arms of a running game",
            ));
        }
        // 005822C2..005822FA: 00B3A6D0's low byte and 00B3A6D1: writers not ported. 00582307 006623A0 and 00582312 00662D10 find nothing to clear: the port has neither list, and no reached code fills them.
    }
    if status == 2 || status == 5 {
        // 00582334..00582359: the cursor shows at the top of each frame, whatever hid it last.
        if let Some(cursor) = manager(cx).cursor_01c {
            if let Some(node) = tile_scene_node(cx, cursor) {
                cx.world
                    .get_mut::<SceneNodePayload>(node)
                    .expect("a tile node has a payload")
                    .effective_av_flags &= !1;
            }
            tile_set_numeric_trait(cx, cursor, TRAIT_VISIBLE, 2.0)?;
        }
        if let Some(cursor) = input.cursor {
            interface_place_cursor(cx, cursor[0], cursor[1])?;
        }
        if manager(cx).cursor_moved_0b9 == 0 {
            let alt = manager(cx).alt_active_tile_088;
            let lost = alt.is_none_or(|alt| {
                !tile_hierarchy_visible(cx, alt) || tile_numeric_trait(cx, alt, TRAIT_TARGET) != 2.0
            });
            if lost {
                interface_select_default_tile(cx)?;
            }
        }
        manager(cx).mouse_held_timer_03c = if input.left_held {
            manager(cx).mouse_held_timer_03c + input.frame_time
        } else {
            0.0
        };
        if manager(cx).cursor_moved_0b9 != 0 {
            let pixel = {
                let manager = manager(cx);
                [manager.cursor_state_020[3], manager.cursor_state_020[5]]
            };
            let picked = interface_pick(cx, pixel);
            let mut picked_menu = picked.and_then(|tile| cx.engine.containing_menu(tile));
            let mut picked = picked;
            if picked.is_some()
                && let Some(alt) = manager(cx).alt_active_tile_088
            {
                tile_set_numeric_trait(cx, alt, TRAIT_MOUSEOVER, 0.0)?;
                if let Some(menu) = cx.engine.containing_menu(alt) {
                    let active = manager(cx).active_tile_098;
                    menu_handle_mouseout(cx, menu, tile_id_of(cx, alt), active)?;
                }
                manager(cx).alt_active_tile_088 = None;
            }
            let active = manager(cx).active_tile_098;
            if picked != active && input.left_held {
                manager(cx).unknown_0a0 = picked.map_or(0, |tile| tile.0 + 1);
                manager(cx).unknown_0a4 = picked_menu.map_or(0, |menu| menu.0 + 1);
            }
            if input.left_released {
                let (active, menu) = {
                    let manager = manager(cx);
                    (manager.active_tile_098, manager.active_menu_09c)
                };
                if let (Some(active), Some(menu)) = (active, menu) {
                    let held = manager(cx).unknown_0a0;
                    if (held == 0 || held == active.0 + 1)
                        && find_value(cx, active, TRAIT_ID).is_some()
                        && cx.engine.menu(menu).state_24 == MENU_STATE_VISIBLE
                    {
                        tile_click(cx, active, active)?;
                        if manager(cx).active_tile_098.is_none() {
                            picked = None;
                            picked_menu = None;
                        }
                    }
                }
                manager(cx).unknown_0a0 = 0;
                manager(cx).unknown_0a4 = 0;
            }
            let active = manager(cx).active_tile_098;
            if picked != active && !input.left_held {
                let active_menu = manager(cx).active_menu_09c;
                if let (Some(active), Some(menu)) = (active, active_menu)
                    && cx.engine.menu(menu).state_24 == MENU_STATE_VISIBLE
                {
                    tile_set_numeric_trait(cx, active, TRAIT_MOUSEOVER, 0.0)?;
                    menu_handle_mouseout(cx, menu, tile_id_of(cx, active), Some(active))?;
                }
                let accepted =
                    picked_menu.filter(|&menu| cx.engine.menu(menu).state_24 == MENU_STATE_VISIBLE);
                {
                    let mut manager = manager(cx);
                    manager.active_tile_098 = accepted.and(picked);
                    manager.active_menu_09c = accepted;
                }
                if let (Some(menu), Some(tile)) = (accepted, picked) {
                    manager(cx).alt_active_tile_088 = Some(tile);
                    tile_set_numeric_trait(cx, tile, TRAIT_MOUSEOVER, 1.0)?;
                    menu_handle_mouseover(cx, menu, tile_id_of(cx, tile), Some(tile))?;
                }
                if manager(cx).cursor_moved_0b9 != 0 {
                    manager(cx).alt_active_tile_088 = None;
                }
            }
            if manager(cx).cursor_moved_0b9 != 0 && input.left_pressed {
                let (active, menu) = {
                    let manager = manager(cx);
                    (manager.active_tile_098, manager.active_menu_09c)
                };
                if let (Some(active), Some(menu)) = (active, menu)
                    && cx.engine.menu(menu).state_24 == MENU_STATE_VISIBLE
                {
                    super::menu_slots::menu_handle_press(cx, menu, tile_id_of(cx, active), active)?;
                }
            }
        }
        // 005821F0 past LABEL_154: held, the class's vt+0x20; else with a wheel, its vt+0x28.
        let (active, menu) = {
            let manager = manager(cx);
            (manager.active_tile_098, manager.active_menu_09c)
        };
        if let (Some(active), Some(menu)) = (active, menu) {
            if input.left_held {
                super::menu_slots::menu_handle_held(cx, menu, tile_id_of(cx, active), active)?;
            } else if input.wheel != 0 {
                super::menu_slots::menu_handle_wheel(
                    cx,
                    menu,
                    tile_id_of(cx, active),
                    active,
                    input.wheel,
                )?;
            }
        }
        // 005821F0's keys. An arrow with shift held gives actions 13 and 14 for left and right (00583644, 0058365E), whose traits no menu here holds. One action a frame: 00583685 discards the buffered events after it.
        {
            let mut manager = manager(cx);
            manager.input_modifier_flags = if input.shift {
                manager.input_modifier_flags | 4
            } else {
                manager.input_modifier_flags & !4
            };
        }
        for &key in &input.keys {
            // 00583466..005834A4: the top stacked menu's vt+0x30 HandleKeyboardInput(key) first; a handled key is the
            // frame's one action.
            let top_mode = top_stacked_menu_mode(&manager(cx).menu_mode_stack_0e0);
            let top_menu = cx
                .engine
                .tile_menu_by_id(top_mode)
                .and_then(|tile| cx.engine.containing_menu(tile));
            if let Some(top_menu) = top_menu
                && menu_handle_keyboard(cx, top_menu, key as u32)?
            {
                break;
            }
            let action = match key {
                KEY_LEFT if input.shift => 13,
                KEY_RIGHT if input.shift => 14,
                KEY_LEFT => 4,
                KEY_RIGHT => 3,
                KEY_UP => 1,
                KEY_DOWN => 2,
                KEY_ENTER => {
                    if manager(cx).cursor_moved_0b9 != 0 {
                        0
                    } else {
                        -2
                    }
                }
                _ => 0,
            };
            if action != 0 {
                interface_navigate(cx, action)?;
                break;
            }
        }
    }
    // 0058377C..00583929: Escape. A visible Load menu takes 005AEED0(1, null) when its +0x28 shows (0057D3F0); the pause and Save menu arms aren't reached here.
    if input.escape
        && let Some(menu) = interface_visible_menu(cx, 1038)
        && let MenuClass::Load(load) = &cx.engine.menu(menu).class
        && load
            .field_id1_28
            .is_some_and(|tile| tile_numeric_trait(cx, tile, TRAIT_VISIBLE) == 2.0)
    {
        menu_handle_click(cx, menu, 1, None)?;
    }
    // 00583975 00572170 (non-menu-mode only: the debug text's timers), then 0058397A..005839A4: vt+0x2C for each of the
    // root's children, in list order, through Tile_GetContainingMenu; each child's next is read before the call.
    if !crate::tes::interface_manager::is_menu_mode(cx.world) {
        count_interface_call(cx, 0x0057_2170);
    }
    let children = manager(cx)
        .menu_root_068
        .map_or_else(Vec::new, |root| cx.engine.tile(root).children_30.clone());
    for child in children {
        if let Some(menu) = cx.engine.containing_menu(child) {
            menu_update_frame(cx, menu, input.frame_time)?;
        }
    }
    // 005839A6..00583A9B: the Big Four keys and control 15 take no input here. 00583AF2: status other than 1 with a player,
    // +0x60 (the menus' player scene node) set and not app culled: 00664C40, the inventory player's animation.
    let scene = manager(cx).player_scene_node_060;
    if manager(cx).menu_loading_status_008 != 1
        && the_player(cx.world).is_some()
        && scene.is_some_and(|node| {
            cx.world
                .get::<SceneNodePayload>(node)
                .is_some_and(|payload| payload.effective_av_flags & 1 == 0)
        })
    {
        return Err(GameError::unclassified(
            0x00583B10,
            "00583B10: 00664C40, the menus' player scene",
        ));
    }
    // 005821F0's end: +0xB4, once.
    let callback = manager(cx).message_callback_0b4.take();
    if let Some(callback) = callback {
        crate::tes::main_menu::run_message_callback(cx, callback)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gamebryo::scene_update::NoSceneControllers;
    use crate::tes::interface_manager::{InterfaceBlock130, interface_settings_from};
    use crate::tes::settings::{SettingCollections, register_static_settings};
    use crate::tes::tile::{NullInterfaceHost, with_interface_cx};
    use bevy_ecs::prelude::*;

    fn run(body: impl FnOnce(&mut InterfaceCx)) {
        let mut collections = SettingCollections::default();
        register_static_settings(&mut collections);
        let settings = interface_settings_from(&collections.ini, 1920, 1080, 8, Default::default());
        let mut world = World::new();
        world.insert_resource(InterfaceManager {
            block_130: Some(InterfaceBlock130 {
                unknown_0: 0,
                unknown_4: 0.0,
                unknown_8: 0.001,
                self_c: true,
                unknown_10: 0,
                nodes: Vec::new(),
                blend_progress: 0.0,
            }),
            ..InterfaceManager::default()
        });
        world.init_resource::<MenuModeChanged>();
        let mut host = NullInterfaceHost::default();
        with_interface_cx(
            &mut world,
            &settings,
            &mut host,
            &mut NoSceneControllers,
            body,
        );
    }

    // 00583D50's three answers and 00584300's restart, 00583C30's removal at a ratio of 1.
    #[test]
    fn timers_run_and_end() {
        run(|cx| {
            let target = TransitionTarget::Trait(1);
            assert_eq!(timed_transition_progress(cx, target), 1.0, "absent");
            timed_transition_start(cx, target, 0.5);
            assert_eq!(timed_transition_progress(cx, target), 0.0);
            interface_advance_timed_transitions(cx, 0.25);
            assert_eq!(timed_transition_progress(cx, target), 0.5);
            timed_transition_start(cx, target, 0.5);
            assert_eq!(
                timed_transition_progress(cx, target),
                0.0,
                "the old timer went"
            );
            interface_advance_timed_transitions(cx, 0.5);
            assert_eq!(
                timed_transition_progress(cx, target),
                1.0,
                "removed at its end"
            );
            timed_transition_start(cx, target, 0.0);
            assert_eq!(timed_transition_progress(cx, target), -1.0, "no duration");
        });
    }

    // 0057CFE0: the compaction, the block by a later mode above 1001, and the last removal's status 4.
    #[test]
    fn menu_modes_compact_and_block() {
        run(|cx| {
            manager(cx).menu_mode_stack_0e0[..2].copy_from_slice(&[1044, 1038]);
            assert_eq!(
                interface_remove_menu_mode(cx, 1044, false),
                -2,
                "1038 is above it"
            );
            assert_eq!(interface_remove_menu_mode(cx, 7, false), -1, "absent");
            assert_eq!(interface_remove_menu_mode(cx, 1044, true), 1038);
            assert_eq!(manager(cx).menu_mode_stack_0e0[0], 1038);
            assert_eq!(interface_remove_menu_mode(cx, 1038, false), 0);
            assert_eq!(manager(cx).menu_loading_status_008, 4);
            assert_eq!(cx.world.resource::<MenuModeChanged>().0, 1);
        });
    }

    // 0057CF60 reads up to the first zero, not the last non-zero.
    #[test]
    fn the_top_mode_stops_at_the_first_zero() {
        assert_eq!(top_stacked_menu_mode(&[0; 10]), 0);
        let mut stack = [0; 10];
        stack[0] = 1044;
        stack[2] = 9;
        assert_eq!(top_stacked_menu_mode(&stack), 1044);
        assert_eq!(top_stacked_menu_mode(&[3; 10]), 3);
    }

    // 0057EA20: min(alpha * progress, alpha), 0 below 0.0001, and disablefade only shows or hides.
    #[test]
    fn a_fade_scales_the_alpha() {
        run(|cx| {
            let tile =
                tile_initialize(cx, crate::tes::tile::TileKind::Image, None, None, None).unwrap();
            tile_set_numeric_trait(cx, tile, TRAIT_ALPHA, 255.0).unwrap();
            cx.engine.tile_mut(tile).scene_24 = Some(Default::default());
            let record = cx.engine.tile_mut(tile).scene_24.as_mut().unwrap();
            record
                .geometry
                .push(crate::tes::interface_scene::TileSceneGeometry {
                    has_material: true,
                    ..Default::default()
                });
            let alpha = |cx: &InterfaceCx| {
                cx.engine.tile(tile).scene_24.as_ref().unwrap().geometry[0].alpha
            };
            interface_apply_fade(cx, tile, 0.5).unwrap();
            assert_eq!(alpha(cx), 0.5);
            interface_apply_fade(cx, tile, 0.00005).unwrap();
            assert_eq!(alpha(cx), 0.0);
            interface_apply_fade(cx, tile, 2.0).unwrap();
            assert_eq!(alpha(cx), 1.0, "bounded by the trait");
            tile_set_numeric_trait(cx, tile, TRAIT_DISABLE_FADE, 2.0).unwrap();
            interface_apply_fade(cx, tile, 0.5).unwrap();
            assert_eq!(tile_numeric_trait(cx, tile, TRAIT_VISIBLE), 1.0);
        });
    }

    fn menu_with_button(cx: &mut InterfaceCx) -> (TileId, TileId, MenuId) {
        let root = tile_initialize(cx, crate::tes::tile::TileKind::Rect, None, None, None).unwrap();
        manager(cx).menu_root_068 = Some(root);
        let menu = add_menu(cx, root);
        let tile = cx.engine.menu(menu).tile_04.unwrap();
        let button =
            tile_initialize(cx, crate::tes::tile::TileKind::Rect, Some(tile), None, None).unwrap();
        tile_set_numeric_trait(cx, button, TRAIT_TARGET, 2.0).unwrap();
        tile_set_numeric_trait(cx, button, TRAIT_XDEFAULT, 5.0).unwrap();
        (root, button, menu)
    }

    fn add_menu(cx: &mut InterfaceCx, root: TileId) -> MenuId {
        let tile =
            tile_initialize(cx, crate::tes::tile::TileKind::Menu, Some(root), None, None).unwrap();
        let menu = MenuId(cx.engine.menus.len() as u32);
        let mut owner = crate::tes::tile::Menu::new(MenuClass::Main(Default::default()));
        owner.tile_04 = Some(tile);
        owner.state_24 = MENU_STATE_VISIBLE;
        cx.engine.menus.push(owner);
        cx.engine.tile_mut(tile).menu_44 = Some(menu);
        menu
    }

    // 00583C5E..00583C99: a node goes at a ratio of 1 or more, and an unordered ratio stays: a zero duration with a zero frame time gives 0 / 0, and 00583D50 goes on answering -1.0.
    #[test]
    fn a_zero_timer_with_no_time_stays() {
        run(|cx| {
            let target = TransitionTarget::Trait(1);
            timed_transition_start(cx, target, 0.0);
            interface_advance_timed_transitions(cx, 0.0);
            assert_eq!(timed_transition_progress(cx, target), -1.0);
            interface_advance_timed_transitions(cx, 0.25);
            assert_eq!(timed_transition_progress(cx, target), 1.0, "inf went");
        });
    }

    // 0057DB66..0057DB7E: on a tie the listindex read is the found tile's (`mov ecx, esi`), not the child's.
    #[test]
    fn a_tie_reads_the_found_tiles_index() {
        run(|cx| {
            let new = |cx: &mut InterfaceCx, parent| {
                let tile =
                    tile_initialize(cx, crate::tes::tile::TileKind::Rect, parent, None, None)
                        .unwrap();
                tile_set_numeric_trait(cx, tile, TRAIT_VISIBLE, 2.0).unwrap();
                tile
            };
            let top = new(cx, None);
            // Containers with no index, whose targets have one: the second walked wins the tie only by its target's.
            let (first, second) = (new(cx, Some(top)), new(cx, Some(top)));
            let (inner_a, inner_b) = (new(cx, Some(first)), new(cx, Some(second)));
            for (tile, index) in [(inner_a, 8.0), (inner_b, 2.0)] {
                tile_set_numeric_trait(cx, tile, TRAIT_TARGET, 2.0).unwrap();
                tile_set_numeric_trait(cx, tile, TRAIT_XDEFAULT, 5.0).unwrap();
                tile_set_numeric_trait(cx, tile, 4010, index).unwrap();
            }
            let last_walked = *cx.engine.tile(top).children_30.last().unwrap();
            let expected = if last_walked == first {
                inner_a
            } else {
                inner_b
            };
            let mut best = i32::MIN;
            assert_eq!(
                interface_default_selection(cx, Some(top), &mut best),
                Some(expected)
            );
            assert_eq!(best, 5);
        });
    }

    // 00582334..00582359: every frame in status 2 or 5 starts with the cursor's node shown and visible 2. 0057FD60's hide only lasts the frame.
    #[test]
    fn the_frame_shows_the_cursor_again() {
        run(|cx| {
            let cursor =
                tile_initialize(cx, crate::tes::tile::TileKind::Rect, None, None, None).unwrap();
            tile_set_numeric_trait(cx, cursor, TRAIT_VISIBLE, 1.0).unwrap();
            {
                let mut manager = manager(cx);
                manager.cursor_01c = Some(cursor);
                manager.menu_loading_status_008 = 2;
                manager.cursor_moved_0b9 = 1;
            }
            interface_update_frame(cx, &InterfaceInputFrame::default()).unwrap();
            assert_eq!(tile_numeric_trait(cx, cursor, TRAIT_VISIBLE), 2.0);
        });
    }

    // 00583644 and 0058365E: an arrow with shift is action 13 or 14, whose trait no menu tile holds, and 00580C0F's direction arm never runs. +0xB9 stays.
    #[test]
    fn shift_arrows_do_not_move_the_focus() {
        run(|cx| {
            let _ = menu_with_button(cx);
            {
                let mut manager = manager(cx);
                manager.menu_loading_status_008 = 2;
                manager.cursor_moved_0b9 = 1;
            }
            interface_update_frame(
                cx,
                &InterfaceInputFrame {
                    keys: vec![KEY_LEFT],
                    shift: true,
                    ..InterfaceInputFrame::default()
                },
            )
            .unwrap();
            assert_eq!(manager(cx).cursor_moved_0b9, 1);
            assert_eq!(manager(cx).alt_active_tile_088, None);
        });
    }

    // 00583677..00583685: after an action 005821F0 discards the buffered events, so a frame acts once.
    #[test]
    fn a_frame_takes_one_action() {
        run(|cx| {
            let (_, button, _) = menu_with_button(cx);
            {
                let mut manager = manager(cx);
                manager.menu_loading_status_008 = 2;
                manager.cursor_moved_0b9 = 1;
            }
            // Up focuses the default; the Enter behind it in the same frame would click.
            interface_update_frame(
                cx,
                &InterfaceInputFrame {
                    keys: vec![KEY_UP, KEY_ENTER],
                    ..InterfaceInputFrame::default()
                },
            )
            .unwrap();
            assert_eq!(manager(cx).alt_active_tile_088, Some(button));
            assert_eq!(tile_numeric_trait(cx, button, TRAIT_CLICK_COUNT_AFTER), 0.0);
        });
    }

    // 005841DE..00584260: a destroyed menu ends the walk (the list changed); the next pass takes the other.
    #[test]
    fn a_fade_out_end_ends_the_pass() {
        run(|cx| {
            let root =
                tile_initialize(cx, crate::tes::tile::TileKind::Rect, None, None, None).unwrap();
            manager(cx).menu_root_068 = Some(root);
            let (first, second) = (add_menu(cx, root), add_menu(cx, root));
            for menu in [first, second] {
                let tile = cx.engine.menu(menu).tile_04.unwrap();
                tile_set_numeric_trait(cx, tile, TRAIT_DESTROY_ON_CLOSE, 2.0).unwrap();
                cx.engine.menu_mut(menu).state_24 = MENU_STATE_FADING_OUT;
            }
            interface_update_menu_visibility(cx).unwrap();
            let left = [first, second]
                .iter()
                .filter(|&&menu| cx.engine.menu(menu).tile_04.is_some())
                .count();
            assert_eq!(left, 1, "one a pass");
            interface_update_menu_visibility(cx).unwrap();
            assert_eq!(cx.engine.menu(first).tile_04, None);
            assert_eq!(cx.engine.menu(second).tile_04, None);
        });
    }
}
