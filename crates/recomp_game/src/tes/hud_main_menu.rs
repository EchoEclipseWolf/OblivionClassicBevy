//! Phase 17: HUDMainMenu (1004), built by 0057D480 through 005A6B00. A new game's first start stops short of what the class is for: its vt+0x2C 005A6DE0 returns at once with no parent cell, and the tiles stay hidden behind the MainMenu. Only the fields and arms a start reaches are ported, and every other arm is a stop. Main thread: 24-bit x87.
//!
//! Evidence: disassembly; `run-h-phase17/snapshot-phase17_end`, seed 6AEE4398.

use bevy_ecs::prelude::*;
use recomp_core::reader::StreamError;
use recomp_core::x87;

use super::actor::{MobileObject, TesObjectRefr};
use super::actor_process::{MiddleHighProcess, process_level};
use super::ai_conditions::the_player;
use super::form::FormIdentity;
use super::form_components::{FormRef, TesActorBaseData, TesRaceForm};
use super::forms_actors::TesRace;
use super::interface_frame::{
    MENU_STATE_FADING_IN, MENU_STATE_FADING_OUT, MENU_STATE_HIDDEN, MENU_STATE_VISIBLE,
    menu_begin_fade_in, menu_begin_fade_out, menu_destroy, menu_enable, top_stacked_menu_mode,
};
use super::interface_manager::{
    InterfaceManager, menu_register_tile, menu_update_cursor_depth, ui_virtual_height,
    ui_virtual_width,
};
use super::player::{PlayerCharacter, floor_calculated_actor_value, player_get_actor_value};
use super::tile::{InterfaceCx, MenuClass, MenuId, TileId};
use super::tile_values::{
    TRAIT_DEPTH, TRAIT_STACKING, TRAIT_USER0, TRAIT_USER1, TRAIT_USER2, TRAIT_USER3, TRAIT_USER6,
    TRAIT_VISIBLE, tile_numeric_trait, tile_set_numeric_trait, tile_set_string_trait,
};
use super::tile_xml::tile_read_xml;
use crate::gamebryo::scene_node::SceneNodePayload;
use crate::gamebryo::scene_update::{SceneUpdateStatus, update_scene_object};
use crate::gamebryo::transform::{matrix_from_euler_xyz, matrix_to_euler_xyz};

/// IDA: MenuID_HUDMain.
pub const HUD_MAIN_MENU_ID: u32 = 1004;
// IDA: 005A62B8's trait.
const TRAIT_FILENAME_FE6: u16 = 0xFE6;

/// IDA: HUDMainMenu (0x94): 005A6780's fields, and 005A55F0's IDs. +0x74 and +0x78..+0x90 are the IconArray and what 005A6DE0 uses, unread here. Completion: 100% of the fields a start reaches.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HudMainMenu {
    /// +0x28, ID 1.
    pub tile_id1_28: Option<TileId>,
    /// +0x2C, ID 2: the health bar, 005A5B50(8).
    pub health_2c: Option<TileId>,
    /// +0x30, ID 3: the magicka bar, 005A5B50(9).
    pub magicka_30: Option<TileId>,
    /// +0x34, ID 4: the fatigue bar, 005A5B50(10).
    pub fatigue_34: Option<TileId>,
    /// +0x38, ID 5.
    pub tile_id5_38: Option<TileId>,
    /// +0x3C, ID 6.
    pub tile_id6_3c: Option<TileId>,
    /// +0x40, ID 7.
    pub tile_id7_40: Option<TileId>,
    /// +0x44, ID 14: 005A6B00 stores user2 and user3 on it.
    pub tile_id14_44: Option<TileId>,
    /// +0x48, ID 8.
    pub tile_id8_48: Option<TileId>,
    /// +0x4C, ID 91: `player_grab_zone`.
    pub grab_zone_4c: Option<TileId>,
    /// +0x50, ID 16.
    pub tile_id16_50: Option<TileId>,
    /// +0x54, ID 18.
    pub tile_id18_54: Option<TileId>,
    /// +0x58, ID 20: a Tile3D, hence a `nif` the port doesn't build.
    pub tile_id20_58: Option<TileId>,
    /// +0x68, 005A5890's pointer x.
    pub pointer_x_68: f32,
    /// +0x6C, 005A5890's pointer y.
    pub pointer_y_6c: f32,
    /// +0x70, how far the player's view is zoomed, 0.6 in 005A6780.
    pub zoom_70: f32,
    /// +0x8C, 8 in 005A6780.
    pub unknown_8c: u32,
}

/// IDA: 005A6780 HUDMainMenu_ctor's stores past 00584640. Completion: 100%.
#[must_use]
pub fn hud_main_menu_constructed() -> HudMainMenu {
    HudMainMenu {
        zoom_70: f32::from_bits(0x3F19_999A),
        unknown_8c: 8,
        ..HudMainMenu::default()
    }
}

/// IDA: 005A55F0 HUDMainMenu_SetField. Completion: 100%.
pub fn hud_set_field(hud: &mut HudMainMenu, id: i32, tile: TileId) {
    let slot = match id {
        1 => &mut hud.tile_id1_28,
        2 => &mut hud.health_2c,
        3 => &mut hud.magicka_30,
        4 => &mut hud.fatigue_34,
        5 => &mut hud.tile_id5_38,
        6 => &mut hud.tile_id6_3c,
        7 => &mut hud.tile_id7_40,
        0x0E => &mut hud.tile_id14_44,
        8 => &mut hud.tile_id8_48,
        0x14 => &mut hud.tile_id20_58,
        0x5B => &mut hud.grab_zone_4c,
        0x10 => &mut hud.tile_id16_50,
        0x12 => &mut hud.tile_id18_54,
        _ => return,
    };
    *slot = Some(tile);
}

/// IDA: 005A55A0: +0x28..+0x48 and +0x58 are all set. +0x4C, +0x50 and +0x54 aren't tested. Completion: 100%.
#[must_use]
pub const fn hud_is_complete(hud: &HudMainMenu) -> bool {
    hud.tile_id1_28.is_some()
        && hud.health_2c.is_some()
        && hud.magicka_30.is_some()
        && hud.fatigue_34.is_some()
        && hud.tile_id5_38.is_some()
        && hud.tile_id6_3c.is_some()
        && hud.tile_id7_40.is_some()
        && hud.tile_id14_44.is_some()
        && hud.tile_id8_48.is_some()
        && hud.tile_id20_58.is_some()
}

// IDA: N/A a helper, with no address of its own.
fn hud_of(cx: &InterfaceCx) -> Option<(TileId, MenuId)> {
    let tile = cx.engine.tile_menu_by_id(HUD_MAIN_MENU_ID)?;
    let menu = cx.engine.containing_menu(tile)?;
    matches!(cx.engine.menu(menu).class, MenuClass::Hud(_)).then_some((tile, menu))
}

// IDA: N/A a helper, with no address of its own.
fn fields(cx: &InterfaceCx, menu: MenuId) -> HudMainMenu {
    let MenuClass::Hud(hud) = &cx.engine.menu(menu).class else {
        unreachable!("hud_of checked the class");
    };
    hud.clone()
}

/// IDA: 0065D270 PlayerCharacter_GetAVModifier(0, av): +0x204 + 4av. Categories 1 and 2 have no caller here. Completion: 100% of category 0.
#[must_use]
pub(crate) fn player_max_modifier(world: &World, player: Entity, actor_value: u32) -> f32 {
    world
        .get::<PlayerCharacter>(player)
        .expect("g_thePlayer is a PlayerCharacter")
        .max_av_modifiers_204[actor_value as usize]
}

/// IDA: 005A5B50(av): the bar's user0 = max(0, vt+0x284(av)) / max(0, 005F1910(av) + +0x204 modifier), or 0 when that's 0. `fild`, `fadd` the float at 24 bits; 005A5BA9 `test ah, 41h` keeps a sum at or above 0, and a NaN. Other values do nothing. Completion: 100%.
/// # Errors
/// A null g_thePlayer or bar, or a stop in the actor value.
pub fn hud_update_stat_bar(
    cx: &mut InterfaceCx,
    menu: MenuId,
    actor_value: u32,
) -> Result<(), StreamError> {
    let hud = fields(cx, menu);
    let bar = match actor_value {
        8 => hud.health_2c,
        9 => hud.magicka_30,
        10 => hud.fatigue_34,
        _ => return Ok(()),
    };
    let player = the_player(cx.world).ok_or(StreamError("005A5B75: a null g_thePlayer"))?;
    let maximum = |world: &World| -> Result<f32, StreamError> {
        let modifier = player_max_modifier(world, player, actor_value);
        let floor = floor_calculated_actor_value(world, player, actor_value)?;
        Ok(x87::stored_float(f64::from(floor) + f64::from(modifier)))
    };
    let first = maximum(cx.world)?;
    #[allow(
        clippy::neg_cmp_op_on_partial_ord,
        reason = "005A5BA9 `test ah, 41h`: 0 above the sum goes to the 0, unordered doesn't"
    )]
    let total = if !(0.0 > first) {
        maximum(cx.world)?
    } else {
        0.0
    };
    let mut ratio = 0.0_f32;
    if total != 0.0 {
        let current = player_get_actor_value(cx.world, player, actor_value)?;
        let current = if current >= 0 {
            player_get_actor_value(cx.world, player, actor_value)?
        } else {
            0
        };
        ratio = x87::stored_float(f64::from(current) / f64::from(total));
    }
    // 0058CEB0 on a null tile faults, in 0058B220: only an XML that 005A55A0 warned about reaches this.
    let bar = bar.ok_or(StreamError("005A5C41: a null stat bar faults"))?;
    tile_set_numeric_trait(cx, bar, TRAIT_USER0, ratio)
}

/// IDA: 00611D00 Actor_GetHeightScale, vt+0xEC: 004D7260's +0x38, times the race's height at +0x60 + 4 * sex when the base has a race. 004D7260's creature arm doesn't apply to the player's base. Completion: 100% for an NPC base.
/// # Errors
/// A base that isn't an NPC.
pub fn actor_height_scale(world: &World, actor: Entity) -> Result<f32, StreamError> {
    let refr = world.get::<TesObjectRefr>(actor).expect("a reference");
    let scale = refr.scale_38;
    let Some(base) = refr.base_form_1c else {
        return Ok(scale);
    };
    let Some(race) = base_race(world, base)? else {
        return Ok(scale);
    };
    let Some(sex) = npc_sex(world, base) else {
        return Ok(0.0 * scale);
    };
    Ok(race_scale(world, race, 0x60 + 4 * sex) * scale)
}

/// IDA: +0xE8, TESRaceForm's +0x04 in a TESNPC. Completion: 100%.
/// # Errors
/// A base that isn't an NPC, an arm no player reaches.
pub fn base_race(world: &World, base: Entity) -> Result<Option<Entity>, StreamError> {
    if world
        .get::<FormIdentity>(base)
        .expect("a form")
        .form_type_04
        != 0x23
    {
        return Err(StreamError("004D7260: a creature base's scale"));
    }
    Ok(
        match world.get::<TesRaceForm>(base).map(|part| part.race_04) {
            Some(FormRef::Resolved(race)) => race,
            _ => None,
        },
    )
}

/// IDA: 00519D20 TESActorBase_GetNPCSexOrMinusOne: an NPC's flags bit 0, and -1 for anything else. Completion: 100%.
#[must_use]
pub fn npc_sex(world: &World, base: Entity) -> Option<usize> {
    (world
        .get::<FormIdentity>(base)
        .expect("a form")
        .form_type_04
        == 0x23)
        .then(|| {
            (world
                .get::<TesActorBaseData>(base)
                .expect("an NPC has its data")
                .flags_04
                & 1) as usize
        })
}

/// IDA: a float of TESRace's +0x50 block, by its offset from the object's start. Completion: 100%.
#[must_use]
pub fn race_scale(world: &World, race: Entity, offset: usize) -> f32 {
    let data = &world.get::<TesRace>(race).expect("a race").data_50;
    f32::from_le_bytes(
        data[offset - 0x50..offset - 0x50 + 4]
            .try_into()
            .expect("four bytes"),
    )
}

/// IDA: 005A5900(turn, zoom): the manager's +0x60 turns about z by turn / 2 degrees, the rotation going through 00711300 and 00711580, and moves between two fixed points by the clamped +0x70 + zoom / 400, each times vt+0xEC, then 00707370(node, 0.0, 1). No HUD returns. Each `fstp` is a 24-bit float. Completion: 100%.
/// # Errors
/// A null g_thePlayer or +0x60, or a stop in the update.
pub fn hud_update_player_view(
    cx: &mut InterfaceCx,
    turn: f32,
    zoom: f32,
) -> Result<(), StreamError> {
    let node = cx
        .world
        .resource::<InterfaceManager>()
        .player_scene_node_060;
    let Some((_, menu)) = hud_of(cx) else {
        return Ok(());
    };
    let node = node.ok_or(StreamError("005A5951: a null +0x60 faults"))?;
    let local = cx
        .world
        .get::<SceneNodePayload>(node)
        .expect("a scene node")
        .local_transform;
    let [x, y, z] = matrix_to_euler_xyz(&local.rotation);
    let half = x87::stored_float(f64::from(turn) * 0.5);
    let z = x87::stored_float(f64::from(z) - f64::from(half) * 0.017_453_292_384_743_69);
    let rotation = matrix_from_euler_xyz(x, y, z);
    let player = the_player(cx.world).ok_or(StreamError("005A59B7: a null g_thePlayer"))?;
    let scale = actor_height_scale(cx.world, player)?;
    let near = [14.0 * scale, 70.0 * scale, -117.0 * scale];
    let scale = actor_height_scale(cx.world, player)?;
    let far = [69.0 * scale, 350.0 * scale, -65.0 * scale];
    let hud = fields(cx, menu);
    let sum = x87::stored_float(f64::from(zoom) / 400.0 + f64::from(hud.zoom_70));
    // 005A5A49 `test ah, 41h`, 005A5A6A and 005A5A7F `test ah, 5`.
    let capped = if 1.0 > sum { sum } else { 1.0 };
    let amount = if capped < 0.0 {
        0.0
    } else if sum < 1.0 {
        sum
    } else {
        1.0
    };
    if let MenuClass::Hud(hud) = &mut cx.engine.menu_mut(menu).class {
        hud.zoom_70 = amount;
    }
    let translation = [
        near[0] + (far[0] - near[0]) * amount,
        near[1] + (far[1] - near[1]) * amount,
        near[2] + (far[2] - near[2]) * amount,
    ];
    {
        let mut payload = cx
            .world
            .get_mut::<SceneNodePayload>(node)
            .expect("a scene node");
        payload.local_transform.rotation = rotation;
        payload.local_transform.translation = translation;
    }
    let status = update_scene_object(cx.world, node, 0.0, true, cx.runtime)?;
    if status.status != SceneUpdateStatus::Updated {
        return Err(StreamError(
            "005A5B36: NiAVObject_Update reported an unsupported state",
        ));
    }
    Ok(())
}

/// IDA: 0057B190 HUD_SetSelectedMagicIcon: with no manager, cursor or root, or the root's user0 not 2.0, nothing. On a first start 005A6B00 runs before 0057D521 sets it, so this does nothing there. Completion: 100% of the arm a start reaches.
/// TODO: [ ] 005A6A40, with the magic icon.
/// # Errors
/// The unported arm.
pub fn hud_set_active_magic_icon(cx: &InterfaceCx) -> Result<(), StreamError> {
    let manager = cx.world.resource::<InterfaceManager>();
    let (Some(_), Some(root)) = (manager.cursor_01c, manager.menu_root_068) else {
        return Ok(());
    };
    if tile_numeric_trait(cx, root, TRAIT_USER0) == 2.0 {
        return Err(StreamError("0057B1F0: 005A6A40 the selected magic icon"));
    }
    Ok(())
}

/// IDA: 0057B2D0 HUD_SetSelectedMagicIcon, a different function from 0057B190: with a manager, cursor and root, and the root's user0 at 2.0, and the HUD up, 005A62B0: `Tile_SetStringTrait(hud +0x3C, 0xFE6, path)`. Completion: 100%.
/// # Errors
/// A stop in the trait's update.
pub fn hud_set_selected_magic_icon_path(
    cx: &mut InterfaceCx,
    path: &[u8],
) -> Result<(), StreamError> {
    let manager = cx.world.resource::<InterfaceManager>();
    let (Some(_), Some(root)) = (manager.cursor_01c, manager.menu_root_068) else {
        return Ok(());
    };
    if tile_numeric_trait(cx, root, TRAIT_USER0) != 2.0 {
        return Ok(());
    }
    let Some((_, menu)) = hud_of(cx) else {
        return Ok(());
    };
    let tile = fields(cx, menu)
        .tile_id6_3c
        .ok_or(StreamError("005A62B4: a null HUD tile faults"))?;
    tile_set_string_trait(cx, tile, TRAIT_FILENAME_FE6, path)
}

/// IDA: what 005A6B00 needs from outside the interface: 00B37BC0 iMapMarkerRevealDistance and 00B37BC8 iMapMarkerVisibleDistance, as `fild` of their dwords.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HudSettings {
    pub map_marker_reveal_b37bc0: i32,
    pub map_marker_visible_b37bc8: i32,
}

/// IDA: 005A6B00: with a manager and its +0x60. 00588890, then Tile_ReadXML. Not a 1004 menu returns null, destroying what it made. 00584880, 005A55A0's warning, the depth for a stacking type, 005A5B50(8, 9, 10). The magic icon goes through 0057B190, so what it would show isn't read. user2 and user3 on +0x44, +0x70 = 1.0 - user6 / 100.0, 005A5900(0, 0), Menu_Enable(1). Completion: 100% of what a start reaches.
/// TODO: [ ] 005A6C94..005A6D19, the equipped magic item's icon.
/// # Errors
/// An existing HUD, an equipped magic item, a null tile, or a stop in a callee.
pub fn hud_main_menu_create(
    cx: &mut InterfaceCx,
    settings: HudSettings,
    frame_time: f32,
) -> Result<Option<TileId>, StreamError> {
    let Some(manager) = cx.world.get_resource::<InterfaceManager>() else {
        return Ok(None);
    };
    if manager.player_scene_node_060.is_none() {
        return Ok(None);
    }
    let root = manager.menu_root_068;
    if cx.engine.tile_menu_by_id(HUD_MAIN_MENU_ID).is_some() {
        return Err(StreamError(
            "005A6B62: destroying an existing HUD is carried",
        ));
    }
    let depth = menu_update_cursor_depth(cx)?;
    let Some(created) = tile_read_xml(cx, root, "Data\\Menus\\Main\\hud_main_menu.xml")? else {
        return Ok(None);
    };
    let Some(menu) = cx.engine.containing_menu(created) else {
        return Ok(None);
    };
    if cx.engine.menu(menu).id() != HUD_MAIN_MENU_ID {
        if cx.engine.menu(menu).tile_04.is_some() {
            menu_destroy(cx, menu)?;
        }
        return Ok(None);
    }
    menu_register_tile(cx, menu, Some(created))?;
    let hud = fields(cx, menu);
    if !hud_is_complete(&hud) {
        cx.host
            .warning("Hud Menu Creation Failed... Are your menu and art resources up to date?");
    }
    let stacking = tile_numeric_trait(cx, created, TRAIT_STACKING);
    if stacking == 6006.0 || stacking == 102.0 {
        tile_set_numeric_trait(cx, created, TRAIT_DEPTH, depth)?;
    }
    hud_update_stat_bar(cx, menu, 8)?;
    hud_update_stat_bar(cx, menu, 9)?;
    hud_update_stat_bar(cx, menu, 10)?;
    if let Some(player) = the_player(cx.world)
        && let Some(process) = cx
            .world
            .get::<MobileObject>(player)
            .and_then(|mobile| mobile.process_58)
        && process_level(cx.world, process) == 0
        && cx
            .world
            .get::<MiddleHighProcess>(process)
            .ok_or(StreamError("005A6CA7: a High process with no equipment"))?
            .weapon_e4
            .is_some()
    {
        return Err(StreamError("005A6CAD: an equipped magic item's icon"));
    }
    let target = hud
        .tile_id14_44
        .ok_or(StreamError("005A6D25: a null +0x44 faults"))?;
    tile_set_numeric_trait(
        cx,
        target,
        TRAIT_USER2,
        settings.map_marker_reveal_b37bc0 as f32,
    )?;
    tile_set_numeric_trait(
        cx,
        target,
        TRAIT_USER3,
        settings.map_marker_visible_b37bc8 as f32,
    )?;
    hud_set_active_magic_icon(cx)?;
    let user6 = tile_numeric_trait(cx, created, TRAIT_USER6);
    let fraction = x87::stored_float(f64::from(user6) / 100.0);
    let zoom = x87::stored_float(1.0 - f64::from(fraction));
    if let MenuClass::Hud(hud) = &mut cx.engine.menu_mut(menu).class {
        hud.zoom_70 = zoom;
    }
    hud_update_player_view(cx, 0.0, 0.0)?;
    menu_enable(cx, menu, true, frame_time)?;
    Ok(Some(created))
}

/// IDA: HUDInfoMenu (0x5C): 005A4660's fields, and 005A4700's IDs 1 to 11 at +0x28..+0x50. +0x54 is the crosshair's reference and +0x58 a frame count. Completion: 100% of what a new game reaches.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HudInfoMenu {
    /// +0x28..+0x50.
    pub tiles_28: [Option<TileId>; 11],
    /// +0x54, the crosshair's reference.
    pub crosshair_54: Option<Entity>,
    /// +0x58.
    pub frames_58: i32,
}

/// IDA: 005A4700 vt+4. Completion: 100%.
pub fn hud_info_set_field(info: &mut HudInfoMenu, id: i32, tile: TileId) {
    if (1..=11).contains(&id) {
        info.tiles_28[(id - 1) as usize] = Some(tile);
    }
}

/// IDA: 005A4840 HUDInfoMenu_Create(0), from 00582287: 00B3B33C's menu is null here, so nothing is destroyed. 00588890, Tile_ReadXML. Not a 1005 menu returns null, destroying what it made. 00584880, 005A46B0's warning over +0x28..+0x4C, the depth for a stacking type, the manager's +0x10 = user1 through `__ftol2_sse`, Menu_Enable(0). Completion: 100%.
/// # Errors
/// A stop in a callee.
pub fn hud_info_menu_create(
    cx: &mut InterfaceCx,
    frame_time: f32,
) -> Result<Option<TileId>, StreamError> {
    let root = cx.world.resource::<InterfaceManager>().menu_root_068;
    let depth = menu_update_cursor_depth(cx)?;
    let Some(created) = tile_read_xml(cx, root, "Data\\Menus\\Main\\hud_info_menu.xml")? else {
        return Ok(None);
    };
    let Some(menu) = cx.engine.containing_menu(created) else {
        return Ok(None);
    };
    if cx.engine.menu(menu).id() != 1005 {
        if cx.engine.menu(menu).tile_04.is_some() {
            menu_destroy(cx, menu)?;
        }
        return Ok(None);
    }
    menu_register_tile(cx, menu, Some(created))?;
    let complete = match &cx.engine.menu(menu).class {
        MenuClass::HudInfo(info) => info.tiles_28[..10].iter().all(Option::is_some),
        _ => false,
    };
    if !complete {
        cx.host.warning(
            "HUD-Info Menu Creation Failed... Are your menu and art resources up to date?",
        );
    }
    let stacking = tile_numeric_trait(cx, created, TRAIT_STACKING);
    if stacking == 6006.0 || stacking == 102.0 {
        tile_set_numeric_trait(cx, created, TRAIT_DEPTH, depth)?;
    }
    let user1 = tile_numeric_trait(cx, created, TRAIT_USER1);
    cx.world.resource_mut::<InterfaceManager>().unknown_010 = x87::ftol(user1) as u32;
    menu_enable(cx, menu, false, frame_time)?;
    Ok(Some(created))
}

/// IDA: 005A5540 vt+0x2C: +0x58 counts to 60, then 0. A crosshair reference at +0x54 refreshes the text there, and no reached code sets one. Completion: 100% with no reference.
pub fn hud_info_update_frame(cx: &mut InterfaceCx) {
    let Some(menu) = cx.engine.tile_menu_by_id(1005) else {
        return;
    };
    let Some(menu) = cx.engine.containing_menu(menu) else {
        return;
    };
    if let MenuClass::HudInfo(info) = &mut cx.engine.menu_mut(menu).class {
        info.frames_58 += 1;
        if info.frames_58 > 60 {
            info.frames_58 = 0;
        }
    }
}

/// IDA: 005A4980 HUDInfoMenu_SetCrosshairRef(null, 0, 0), as LoadingMenu_Create calls it: no 00B3B33C menu, no player, a set player +0x5C0, or no tile returns 0. A null +0x54 with the menu fading in or visible returns 1 at once; otherwise +0x54 = null and, fading in or visible, 00584740; 1. Completion: 100% of the null reference's arm.
/// # Errors
/// A stop in 00584740.
pub fn hud_info_clear_crosshair(
    cx: &mut InterfaceCx,
    frame_time: f32,
) -> Result<bool, StreamError> {
    let Some(menu) = cx
        .engine
        .tile_menu_by_id(1005)
        .and_then(|tile| cx.engine.containing_menu(tile))
    else {
        return Ok(false);
    };
    let Some(player) = the_player(cx.world) else {
        return Ok(false);
    };
    if cx
        .world
        .get::<PlayerCharacter>(player)
        .is_some_and(|body| body.byte_5c0 != 0)
        || cx.engine.menu(menu).tile_04.is_none()
    {
        return Ok(false);
    }
    let state = cx.engine.menu(menu).state_24;
    let shown = state == MENU_STATE_FADING_IN || state == MENU_STATE_VISIBLE;
    let MenuClass::HudInfo(info) = &mut cx.engine.menu_mut(menu).class else {
        return Err(StreamError("005A4980: 00B3B33C isn't a HUDInfoMenu"));
    };
    if info.crosshair_54.is_none() && shown {
        return Ok(true);
    }
    info.crosshair_54 = None;
    if shown {
        menu_begin_fade_out(cx, menu, frame_time)?;
    }
    Ok(true)
}

/// IDA: 005878B0 InterfaceManager_HasVisibleNonFadingMenuByID: the root's children for a menu of this ID: visible isn't 1.0, and it isn't fading out. Completion: 100%.
#[must_use]
pub fn interface_has_visible_non_fading_menu(cx: &InterfaceCx, id: u32) -> bool {
    let Some(root) = cx.world.resource::<InterfaceManager>().menu_root_068 else {
        return false;
    };
    for &child in &cx.engine.tile(root).children_30 {
        let Some(menu) = cx.engine.containing_menu(child) else {
            continue;
        };
        let owner = cx.engine.menu(menu);
        if let Some(tile) = owner.tile_04
            && owner.id() == id
        {
            return tile_numeric_trait(cx, tile, TRAIT_VISIBLE) != 1.0
                && owner.state_24 != MENU_STATE_FADING_OUT;
        }
    }
    false
}

/// IDA: 00578FE0 Interface_GetActiveTopMenuID: no manager, cursor or root, or the root's user0 not 2.0: 0; otherwise 0057CF60. Completion: 100%.
#[must_use]
pub fn interface_active_top_menu_id(cx: &InterfaceCx) -> u32 {
    let manager = cx.world.resource::<InterfaceManager>();
    let (Some(_), Some(root)) = (manager.cursor_01c, manager.menu_root_068) else {
        return 0;
    };
    let stack = manager.menu_mode_stack_0e0;
    if tile_numeric_trait(cx, root, TRAIT_USER0) == 2.0 {
        top_stacked_menu_mode(&stack)
    } else {
        0
    }
}

/// IDA: 005A6040 HUDMainMenu_UpdateVisibility(a1, instant): no player, player +0x5C0 set, or no HUD returns. With `a1`: any of Stats, Inventory, Magic or Map showing puts user3 at 0 and fades or shows it; otherwise a showing HUD under a stacked menu mode hides or fades out, and user3 goes to 1. Without `a1`: user3 1, then the same show. +0x24 tests the states 1 and 8, 4 and 2. 00587B20's table growth is the host's vector. Completion: 100%.
/// # Errors
/// A stop in a tile update.
pub fn hud_update_visibility(
    cx: &mut InterfaceCx,
    a1: bool,
    instant: bool,
    frame_time: f32,
) -> Result<(), StreamError> {
    let Some(player) = the_player(cx.world) else {
        return Ok(());
    };
    if cx
        .world
        .get::<PlayerCharacter>(player)
        .is_some_and(|body| body.byte_5c0 != 0)
    {
        return Ok(());
    }
    let Some((tile, menu)) = hud_of(cx) else {
        return Ok(());
    };
    let own = cx.engine.menu(menu).tile_04.expect("hud_of found its tile");
    let shown = |state: u32| state == MENU_STATE_VISIBLE || state == MENU_STATE_FADING_IN;
    if !a1 {
        tile_set_numeric_trait(cx, own, TRAIT_USER3, 1.0)?;
        if !shown(cx.engine.menu(menu).state_24) {
            if instant {
                tile_set_numeric_trait(cx, tile, TRAIT_VISIBLE, 2.0)?;
            } else {
                menu_begin_fade_in(cx, menu, frame_time);
            }
        }
        return Ok(());
    }
    let big_four = [1003, 1002, 1022, 1023]
        .iter()
        .any(|&id| interface_has_visible_non_fading_menu(cx, id));
    if big_four {
        tile_set_numeric_trait(cx, own, TRAIT_USER3, 0.0)?;
        if !shown(cx.engine.menu(menu).state_24) {
            if instant {
                tile_set_numeric_trait(cx, tile, TRAIT_VISIBLE, 2.0)?;
            } else {
                menu_begin_fade_in(cx, menu, frame_time);
            }
        }
        return Ok(());
    }
    let state = cx.engine.menu(menu).state_24;
    if state != MENU_STATE_HIDDEN
        && state != MENU_STATE_FADING_OUT
        && interface_active_top_menu_id(cx) != 0
    {
        if instant {
            tile_set_numeric_trait(cx, tile, TRAIT_VISIBLE, 1.0)?;
            return tile_set_numeric_trait(cx, own, TRAIT_USER3, 1.0);
        }
        menu_begin_fade_out(cx, menu, frame_time)?;
    }
    tile_set_numeric_trait(cx, own, TRAIT_USER3, 1.0)
}

/// IDA: 005A6DE0 vt+0x2C, each frame: 005A6E19 tests the player's parent cell, and with none returns at once. Completion: the head.
/// TODO: [ ] 005A6E26.. the HUD's frame, when a game runs in a cell.
/// # Errors
/// The unported frame.
pub fn hud_update_frame(cx: &InterfaceCx) -> Result<(), StreamError> {
    let Some(player) = the_player(cx.world) else {
        return Err(StreamError("005A6E13: a null g_thePlayer"));
    };
    if cx
        .world
        .get::<TesObjectRefr>(player)
        .expect("a reference")
        .parent_cell_40
        .is_some()
    {
        return Err(StreamError(
            "005A6E26: the HUD's frame in a cell is carried",
        ));
    }
    Ok(())
}

// IDA: 005A58AC..005A58C7 and 005A58CA..005A58F3: the pointer in virtual pixels, each through `__ftol2_sse`. Both are 005A6940's too.
fn hud_pointer(cx: &InterfaceCx) -> (i32, i32) {
    let state = cx.world.resource::<InterfaceManager>().cursor_state_020;
    let width = ui_virtual_width(cx.settings);
    let x = x87::ftol(x87::stored_float(
        f64::from(width) * 0.5 + f64::from(state[0]),
    ));
    let height = ui_virtual_height(cx.settings);
    let middle = x87::stored_float(f64::from(height) * 0.5 + f64::from(state[2]));
    let y = x87::ftol(x87::stored_float(f64::from(height) - f64::from(middle)));
    (x, y)
}

/// IDA: 005A5890 vt+8: the grab zone's press keeps the pointer in +0x68, +0x6C. Completion: 100%.
pub fn hud_handle_press(cx: &mut InterfaceCx, menu: MenuId, id: i32) {
    if id != 0x5B {
        return;
    }
    let (x, y) = hud_pointer(cx);
    if let MenuClass::Hud(hud) = &mut cx.engine.menu_mut(menu).class {
        hud.pointer_x_68 = x as f32;
        hud.pointer_y_6c = y as f32;
    }
}

/// IDA: 005A6940 vt+0x20: holding the grab zone turns and zooms the view by the pointer's move, then keeps the pointer. `fisub`, `fsub`, each stored. Completion: 100%.
/// # Errors
/// A stop in 005A5900.
pub fn hud_handle_held(cx: &mut InterfaceCx, menu: MenuId, id: i32) -> Result<(), StreamError> {
    if id != 0x5B {
        return Ok(());
    }
    let (x, y) = hud_pointer(cx);
    let hud = fields(cx, menu);
    let zoom = x87::stored_float(f64::from(hud.pointer_y_6c) - f64::from(y));
    let turn = x87::stored_float(f64::from(x) - f64::from(hud.pointer_x_68));
    hud_update_player_view(cx, turn, zoom)?;
    hud_handle_press(cx, menu, id);
    Ok(())
}

/// IDA: 005A68C0 vt+0xC: with the HUD's user3 at 0, IDs 9 to 12 pick a tab; otherwise nothing. Completion: the arm a menu screen reaches.
/// TODO: [ ] 005A6914.. the tabs, with the Stats, Inventory, Magic and Map menus.
/// # Errors
/// The unported tabs.
pub fn hud_handle_click(cx: &InterfaceCx, id: i32) -> Result<(), StreamError> {
    let Some((tile, _)) = hud_of(cx) else {
        return Ok(());
    };
    if tile_numeric_trait(cx, tile, TRAIT_USER3) == 0.0 && (9..=12).contains(&id) {
        return Err(StreamError("005A6914: the HUD's tabs are carried"));
    }
    Ok(())
}
