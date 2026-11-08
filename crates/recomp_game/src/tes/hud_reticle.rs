//! Phase 19: 0057D940 InterfaceManager_UpdateReticle, with 005A8000 and 005A8710: the crosshair and sneak tiles of `hud_reticle.xml`, which the first 00583F40 after the LoadingMenu's mode goes builds.
//!
//! Evidence: disassembly; `run-k-phase19`, `snapshot-phase19_end`, seed 6AEE7210.

use bevy_ecs::prelude::*;
use recomp_core::reader::StreamError;

use super::actor_process::HighProcess;
use super::ai_conditions::{process_of, the_player};
use super::hud_main_menu::interface_active_top_menu_id;
use super::interface_frame::{tile_destroy, tile_transition_numeric_trait_to};
use super::interface_manager::InterfaceManager;
use super::player::PlayerCharacter;
use super::tile::{InterfaceCx, TileId};
use super::tile_values::{
    TRAIT_ID, TRAIT_USER2, TRAIT_USER3, TRAIT_VISIBLE, tile_numeric_trait, tile_set_numeric_trait,
};
use super::tile_xml::tile_read_xml;

/// IDA: the 0058A1C0 table.
const TRAIT_USER8: u16 = 4022;
const TRAIT_USER9: u16 = 4023;
/// IDA: 005A8730's 0x414.
const MAIN_MENU_ID: u32 = 1044;

/// IDA: 005A8000's and 005A8710's globals. The floats start as the file holds them, until 005A8000 reads them from the tiles.
#[derive(Resource, Clone, Debug, PartialEq)]
pub struct HudReticle {
    /// 00B3B350: id 1.
    pub crosshair_b3b350: Option<TileId>,
    /// 00B3B354: id 3.
    pub sneak_b3b354: Option<TileId>,
    /// 00B3B358: id 2.
    pub detect_b3b358: Option<TileId>,
    pub sneak_alpha_b140b8: f32,
    pub crosshair_seconds_b140bc: f32,
    pub detect_seconds_b140c0: f32,
    pub sneak_seconds_b140c4: f32,
    pub sneaking_b3b361: u8,
    pub byte_b14e90: u8,
}

impl Default for HudReticle {
    /// IDA: the file's values at 00B140B8..00B140C4; 00B3B350..00B3B361 and 00B14E90 are zero.
    fn default() -> Self {
        Self {
            crosshair_b3b350: None,
            sneak_b3b354: None,
            detect_b3b358: None,
            sneak_alpha_b140b8: 200.0,
            crosshair_seconds_b140bc: 0.25,
            detect_seconds_b140c0: 0.25,
            sneak_seconds_b140c4: 0.25,
            sneaking_b3b361: 0,
            byte_b14e90: 0,
        }
    }
}

/// IDA: 005894F0: the tile itself when its `id` trait is `id`, else the first match down its children in list order. Completion: 100%.
#[must_use]
pub fn tile_find_by_id(cx: &InterfaceCx, tile: TileId, id: i32) -> Option<TileId> {
    if tile_numeric_trait(cx, tile, TRAIT_ID) as i32 == id {
        return Some(tile);
    }
    cx.engine
        .tile(tile)
        .children_30
        .iter()
        .find_map(|&child| tile_find_by_id(cx, child, id))
}

/// IDA: 005894A0: a running transition's destination, else the trait. Completion: 100%.
fn tile_trait_destination(cx: &InterfaceCx, tile: TileId, trait_id: u16) -> f32 {
    cx.engine
        .tile(tile)
        .transitions_28
        .iter()
        .find(|running| running.trait_04 == trait_id)
        .map_or_else(
            || tile_numeric_trait(cx, tile, trait_id),
            |running| running.to_0c,
        )
}

/// IDA: 005E0550 Actor_IsSneaking on the player: a process whose vt+0x2C0 has 0x400 and not 0x800. High 006285A0 answers its word +0x1FC, and the others 0. Completion: 100%.
fn player_is_sneaking(cx: &InterfaceCx, player: Entity) -> bool {
    process_of(cx.world, player)
        .and_then(|process| cx.world.get::<HighProcess>(process))
        .is_some_and(|high| {
            high.movement_flags_1fc & 0x400 != 0 && high.movement_flags_1fc & 0x800 == 0
        })
}

/// IDA: 005A8000: ids 1, 2 and 3 under the tile, and with all three their user3, user9 and user9. Completion: 100%.
fn hud_reticle_bind(cx: &mut InterfaceCx, xml: Option<TileId>) {
    let Some(xml) = xml else {
        return;
    };
    let crosshair = tile_find_by_id(cx, xml, 1);
    let detect = tile_find_by_id(cx, xml, 2);
    let sneak = tile_find_by_id(cx, xml, 3);
    let seconds = crosshair
        .zip(detect)
        .zip(sneak)
        .map(|((crosshair, detect), sneak)| {
            (
                tile_numeric_trait(cx, crosshair, TRAIT_USER3),
                tile_numeric_trait(cx, detect, TRAIT_USER9),
                tile_numeric_trait(cx, sneak, TRAIT_USER9),
            )
        });
    let mut state = cx.world.get_resource_or_init::<HudReticle>();
    state.crosshair_b3b350 = crosshair;
    state.detect_b3b358 = detect;
    state.sneak_b3b354 = sneak;
    if let Some((first, second, third)) = seconds {
        state.crosshair_seconds_b140bc = first;
        state.detect_seconds_b140c0 = second;
        state.sneak_seconds_b140c4 = third;
    }
}

/// IDA: 005A8710(mode), with 00B3B350 and 00B3B354: on the main menu all three hide, else the crosshair shows by bCrossHair (00B13210) and the other two show. Then the crosshair's alpha and the sneak meter go toward what `mode` asks, through 00589980. Completion: 100%.
/// # Errors
/// A stop in a tile update.
fn hud_reticle_update(cx: &mut InterfaceCx, player: Entity, mode: u32) -> Result<(), StreamError> {
    let state = cx.world.get_resource_or_init::<HudReticle>().clone();
    let (Some(crosshair), Some(sneak)) = (state.crosshair_b3b350, state.sneak_b3b354) else {
        return Ok(());
    };
    let cross_hair_setting = cx.settings.cross_hair_b13210;
    let detect_visible = if interface_active_top_menu_id(cx) == MAIN_MENU_ID {
        tile_set_numeric_trait(cx, crosshair, TRAIT_VISIBLE, 1.0)?;
        tile_set_numeric_trait(cx, sneak, TRAIT_VISIBLE, 1.0)?;
        1.0
    } else {
        let shown = f32::from(u8::from(cross_hair_setting)) + 1.0;
        tile_set_numeric_trait(cx, crosshair, TRAIT_VISIBLE, shown)?;
        tile_set_numeric_trait(cx, sneak, TRAIT_VISIBLE, 2.0)?;
        2.0
    };
    if let Some(detect) = state.detect_b3b358 {
        tile_set_numeric_trait(cx, detect, TRAIT_VISIBLE, detect_visible)?;
    }
    let at_full = tile_trait_destination(cx, crosshair, TRAIT_USER2) == 255.0;
    let wanted = if mode == 2 { at_full } else { mode > 0 };
    if at_full {
        if !wanted {
            if cross_hair_setting {
                let from = tile_numeric_trait(cx, crosshair, TRAIT_USER2);
                let seconds = state.crosshair_seconds_b140bc;
                tile_transition_numeric_trait_to(cx, crosshair, TRAIT_USER2, from, 0.0, seconds);
            }
            let from = tile_numeric_trait(cx, sneak, TRAIT_USER8);
            let seconds = state.sneak_seconds_b140c4;
            tile_transition_numeric_trait_to(cx, sneak, TRAIT_USER8, from, 0.0, seconds);
            cx.world.resource_mut::<HudReticle>().sneaking_b3b361 = 1;
            if let Some(detect) = state.detect_b3b358.filter(|_| state.byte_b14e90 == 0) {
                let from = tile_numeric_trait(cx, detect, TRAIT_USER8);
                let seconds = state.detect_seconds_b140c0;
                tile_transition_numeric_trait_to(cx, detect, TRAIT_USER8, from, 0.0, seconds);
            }
        }
    } else if wanted
        && cx
            .world
            .get::<PlayerCharacter>(player)
            .is_some_and(|body| body.byte_5c0 == 0)
    {
        let sneaking = player_is_sneaking(cx, player);
        let to = if sneaking {
            state.sneak_alpha_b140b8
        } else {
            0.0
        };
        let from = tile_numeric_trait(cx, sneak, TRAIT_USER8);
        let seconds = state.sneak_seconds_b140c4;
        tile_transition_numeric_trait_to(cx, sneak, TRAIT_USER8, from, to, seconds);
        cx.world.resource_mut::<HudReticle>().sneaking_b3b361 = u8::from(sneaking);
        if cross_hair_setting {
            let from = tile_numeric_trait(cx, crosshair, TRAIT_USER2);
            let seconds = state.crosshair_seconds_b140bc;
            tile_transition_numeric_trait_to(cx, crosshair, TRAIT_USER2, from, 255.0, seconds);
        }
    }
    Ok(())
}

/// IDA: N/A host substitution: 0057D940's modes asked for where no interface context is held (00667BE0's 00578CF0 inside the
/// drain), in order, for the driver to run when the drain returns.
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct ReticleRequests {
    pub modes: Vec<u32>,
}

/// IDA: N/A runs the queued 0057D940 calls in order.
/// # Errors
/// A stop in 0057D940.
pub fn run_reticle_requests(cx: &mut InterfaceCx) -> Result<(), StreamError> {
    let modes = cx
        .world
        .get_resource_mut::<ReticleRequests>()
        .map(|mut requests| std::mem::take(&mut requests.modes))
        .unwrap_or_default();
    for mode in modes {
        interface_update_reticle(cx, mode)?;
    }
    Ok(())
}

/// IDA: 0057D940 InterfaceManager_UpdateReticle(mode): no player returns. A held reticle's visible = 2, and mode 2 keeps the crosshair's showing as the ask. Mode 3 deletes the reticle. None held and asked for: 00B3B0A2 = 1, `hud_reticle.xml` under the menu root at +0x80, 005A8000, 00B3B0A2 = 0. Then 005A8710. Completion: 100%, with its Tile3D as the counted boundary it is elsewhere.
/// # Errors
/// A stop in a tile's build or update.
pub fn interface_update_reticle(cx: &mut InterfaceCx, mode: u32) -> Result<(), StreamError> {
    let Some(player) = the_player(cx.world) else {
        return Ok(());
    };
    let mut wanted = false;
    let held = cx.world.resource::<InterfaceManager>().hud_reticule_080;
    if let Some(held) = held {
        tile_set_numeric_trait(cx, held, TRAIT_VISIBLE, 2.0)?;
        if let Some(crosshair) = cx
            .world
            .get_resource_or_init::<HudReticle>()
            .crosshair_b3b350
        {
            wanted = tile_numeric_trait(cx, crosshair, TRAIT_VISIBLE) != 1.0;
        }
    }
    if held.is_none() || mode != 2 {
        wanted = mode > 0;
    }
    if let Some(held) = held.filter(|_| mode == 3) {
        tile_destroy(cx, held)?;
        cx.world.resource_mut::<InterfaceManager>().hud_reticule_080 = None;
    }
    if cx
        .world
        .resource::<InterfaceManager>()
        .hud_reticule_080
        .is_none()
        && wanted
    {
        let saved = cx.engine.in_scene_update;
        cx.engine.in_scene_update = true;
        let root = cx.world.resource::<InterfaceManager>().menu_root_068;
        let xml = tile_read_xml(cx, root, "Data\\Menus\\Main\\hud_reticle.xml")?;
        cx.world.resource_mut::<InterfaceManager>().hud_reticule_080 = xml;
        hud_reticle_bind(cx, xml);
        cx.engine.in_scene_update = saved;
    }
    hud_reticle_update(cx, player, mode)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gamebryo::scene_update::NoSceneControllers;
    use crate::tes::interface_frame::interface_update_menu_visibility;
    use crate::tes::interface_manager::{
        InterfaceBlock130, MenuModeChanged, interface_settings_from,
    };
    use crate::tes::settings::{SettingCollections, register_static_settings};
    use crate::tes::tile::{NullInterfaceHost, TileKind, with_interface_cx};
    use crate::tes::tile_values::tile_initialize;

    fn run(cross_hair: bool, body: impl FnOnce(&mut InterfaceCx, Entity)) {
        let mut collections = SettingCollections::default();
        register_static_settings(&mut collections);
        let mut settings =
            interface_settings_from(&collections.ini, 1920, 1080, 8, Default::default());
        assert!(settings.cross_hair_b13210, "the file's bCrossHair");
        settings.cross_hair_b13210 = cross_hair;
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
        let player = world.spawn(PlayerCharacter::default()).id();
        let mut host = NullInterfaceHost::default();
        with_interface_cx(
            &mut world,
            &settings,
            &mut host,
            &mut NoSceneControllers,
            |cx| body(cx, player),
        );
    }

    fn reticle(cx: &mut InterfaceCx) -> (TileId, TileId, TileId, TileId) {
        let root = tile_initialize(cx, TileKind::Rect, None, None, None).unwrap();
        let group = tile_initialize(cx, TileKind::Rect, Some(root), None, None).unwrap();
        let mut made = Vec::new();
        for (parent, id) in [(group, 3.0), (root, 2.0), (group, 1.0)] {
            let tile = tile_initialize(cx, TileKind::Image, Some(parent), None, None).unwrap();
            tile_set_numeric_trait(cx, tile, TRAIT_ID, id).unwrap();
            made.push(tile);
        }
        (root, made[2], made[1], made[0])
    }

    // 005894F0 goes down the first child before the next.
    #[test]
    fn a_tile_is_found_by_its_id_at_any_depth() {
        run(true, |cx, _| {
            let (root, crosshair, detect, sneak) = reticle(cx);
            assert_eq!(tile_find_by_id(cx, root, 1), Some(crosshair));
            assert_eq!(tile_find_by_id(cx, root, 2), Some(detect));
            assert_eq!(tile_find_by_id(cx, root, 3), Some(sneak));
            assert_eq!(tile_find_by_id(cx, root, 9), None);
        });
    }

    // 005A8000 needs all three for the floats.
    #[test]
    fn the_seconds_come_from_the_tiles() {
        run(true, |cx, _| {
            let (root, crosshair, detect, sneak) = reticle(cx);
            tile_set_numeric_trait(cx, crosshair, TRAIT_USER3, 0.5).unwrap();
            tile_set_numeric_trait(cx, detect, TRAIT_USER9, 0.75).unwrap();
            tile_set_numeric_trait(cx, sneak, TRAIT_USER9, 1.5).unwrap();
            hud_reticle_bind(cx, Some(root));
            let state = cx.world.resource::<HudReticle>().clone();
            assert_eq!(state.crosshair_b3b350, Some(crosshair));
            assert_eq!(state.detect_b3b358, Some(detect));
            assert_eq!(state.sneak_b3b354, Some(sneak));
            assert_eq!(state.crosshair_seconds_b140bc, 0.5);
            assert_eq!(state.detect_seconds_b140c0, 0.75);
            assert_eq!(state.sneak_seconds_b140c4, 1.5);
            assert_eq!(state.sneak_alpha_b140b8, 200.0, "the file's");
        });
    }

    // 005A8710: mode 1 asks for the crosshair, then mode 0 takes it and the sneak meter back, and marks 00B3B361.
    #[test]
    fn the_crosshair_goes_up_and_back() {
        run(true, |cx, player| {
            let (root, crosshair, detect, sneak) = reticle(cx);
            hud_reticle_bind(cx, Some(root));
            hud_reticle_update(cx, player, 1).unwrap();
            assert_eq!(tile_numeric_trait(cx, crosshair, TRAIT_VISIBLE), 2.0);
            assert_eq!(tile_numeric_trait(cx, sneak, TRAIT_VISIBLE), 2.0);
            assert_eq!(tile_numeric_trait(cx, detect, TRAIT_VISIBLE), 2.0);
            assert_eq!(tile_trait_destination(cx, crosshair, TRAIT_USER2), 255.0);
            assert_eq!(cx.world.resource::<HudReticle>().sneaking_b3b361, 0);
            hud_reticle_update(cx, player, 0).unwrap();
            assert_eq!(tile_trait_destination(cx, crosshair, TRAIT_USER2), 0.0);
            assert_eq!(cx.world.resource::<HudReticle>().sneaking_b3b361, 1);
        });
    }

    // bCrossHair off: visible 1, and no transition.
    #[test]
    fn no_crosshair_setting_hides_it() {
        run(false, |cx, player| {
            let (root, crosshair, _, _) = reticle(cx);
            hud_reticle_bind(cx, Some(root));
            hud_reticle_update(cx, player, 1).unwrap();
            assert_eq!(tile_numeric_trait(cx, crosshair, TRAIT_VISIBLE), 1.0);
            assert!(cx.engine.tile(crosshair).transitions_28.is_empty());
        });
    }

    // 00583F40's end with no player: status 1, and 0057D940 returns at its first test.
    #[test]
    fn status_four_ends_without_a_reticle_when_there_is_no_player() {
        run(true, |cx, player| {
            cx.world.despawn(player);
            let mut manager = cx.world.resource_mut::<InterfaceManager>();
            manager.menu_loading_status_008 = 4;
            manager.input_modifier_flags = 7;
            interface_update_menu_visibility(cx).unwrap();
            let manager = cx.world.resource::<InterfaceManager>();
            assert_eq!(manager.menu_loading_status_008, 1);
            assert_eq!(manager.hud_reticule_080, None);
            assert_eq!(manager.input_modifier_flags, 0);
        });
    }
}
