//! InventoryMenu (1002), as 0057E150's second creation builds it: 0057A2D0 over `Data\Menus\Main\inventory_menu.xml`, the
//! XML's fields (005A9B40 SetField) and 005AADC0's list: the player's inventory entries (`inventory_entries`) that pass
//! 004854F0, sorted (005AA2A0), each a row from the menu's only template with its strings (0048F450), then 005AA3A0's tab
//! filter and the root's weight, armor rating and gold. The run's player holds the four prison clothes, unworn; an arm
//! they don't reach (an enchanted, owned, poisoned or damaged item, armor, gold, a matched old row) stops at its address.

use bevy_ecs::prelude::*;
use recomp_core::reader::StreamError;
use recomp_core::x87;

use super::actor::{Character, MobileObject, TesObjectRefr};
use super::actor_process::HighProcess;
use super::ai_conditions::the_player;
use super::form::FormIdentity;
use super::form_components::{
    FormRef, TesBipedModelForm, TesContainer, TesEnchantableForm, TesFullName, TesHealthForm,
    TesQualityForm, TesUsesForm, TesValueForm, TesWeightForm,
};
use super::forms_init::form_type;
use super::hud_main_menu::npc_sex;
use super::interface_frame::{TRAIT_XDEFAULT, menu_destroy, menu_enable, tile_destroy};
use super::interface_manager::{InterfaceManager, menu_register_tile, menu_update_cursor_depth};
use super::inventory::{
    ContainerChanges, InventoryEntry, ensure_container_changes, put_container_changes,
};
use super::inventory_entries::{
    entry_is_worn, reference_inventory_count, reference_inventory_entry,
};
use super::player::{PlayerCharacter, floor_calculated_actor_value, player_get_actor_value_float};
use super::tile::{InterfaceCx, MenuClass, MenuId, TileId};
use super::tile_values::{
    TRAIT_DEPTH, TRAIT_ID, TRAIT_STACKING, TRAIT_USER0, TRAIT_VISIBLE, find_value,
    tile_numeric_trait, tile_set_numeric_trait, tile_set_string_trait, tile_string_trait,
};
use super::tile_xml::{tile_build_template, tile_read_xml, tile_xml_clear_cached_file};

/// IDA: MenuID_Inventory.
pub const MENU_ID_INVENTORY: u32 = 1002;
const INVENTORY_XML: &str = "Data\\Menus\\Main\\inventory_menu.xml";
/// IDA: TileTrait 0xFAA, listindex.
const TRAIT_LIST_INDEX: u16 = 4010;
/// IDA: the form flag TESForm vt+0x78 (004D7030) tests: 0x400.
const FORM_FLAG_PERSISTENT: u32 = 0x400;

// IDA: TileTrait userN, 4014 + N.
const fn user(n: u16) -> u16 {
    TRAIT_USER0 + n
}

/// IDA: the statics 005AADC0 and 005AA2A0 touch: 00B3B3D8 (cleared), 00B3B3D9 (0057A2D0 clears it), 00B3B3DB the sort byte
/// (.bss, 0 at the start; only 005ABBC0, a click, writes it) and p_quantity (-1).
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InventoryMenuStatics {
    pub byte_b3b3d8: u8,
    pub byte_b3b3d9: u8,
    pub sort_b3b3db: i8,
    pub quantity: i32,
}

/// IDA: 005A9B00 InventoryMenu_ctor (0x58, via 00587D20's 1002 case): Menu_Constructor; +0x28..+0x34 and +0x3C zero, +0x48
/// and +0x4C 0.0, +0x50 null, B3B3D9 0, vtable A6C18C, +0x40 the filter 31, the byte +0x44 0xFF. +0x38 and +0x54 aren't
/// set. Completion: 100%.
#[derive(Clone, Debug, PartialEq)]
pub struct InventoryMenu {
    /// +0x28..+0x38: fields 9..13 (9 the tabs, 10 the list pane, 11 its scroll bar, 12, 13 the second heading tile).
    pub tiles_28: [Option<TileId>; 5],
    pub field_3c: u32,
    /// +0x40: the filter 005AA3A0 takes.
    pub filter_40: u32,
    pub byte_44: u8,
    pub float_48: f32,
    pub float_4c: f32,
    /// +0x54: never written before 005AA3A0 compares indices with it. A recorded substitution: run-af's bytes there,
    /// EA727BD7, a negative int, so the first row shown takes the xdefault as it did in that run.
    pub unset_54: i32,
}

impl Default for InventoryMenu {
    fn default() -> Self {
        Self {
            tiles_28: [None; 5],
            field_3c: 0,
            filter_40: 31,
            byte_44: 0xFF,
            float_48: 0.0,
            float_4c: 0.0,
            unset_54: 0xEA72_7BD7_u32 as i32,
        }
    }
}

impl InventoryMenu {
    /// IDA: 005A9B40 InventoryMenu_SetField: ids 9..13 to +0x28..+0x38. Completion: 100%.
    pub fn set_field(&mut self, id: i32, tile: TileId) {
        if (9..=13).contains(&id) {
            self.tiles_28[id as usize - 9] = Some(tile);
        }
    }

    /// IDA: 0057A2D0's test: +0x2C, +0x28, +0x30, +0x34 and +0x38 nonnull. Completion: 100%.
    #[must_use]
    pub fn has_required_tiles(&self) -> bool {
        self.tiles_28.iter().all(Option::is_some)
    }
}

// IDA: N/A the menu's InventoryMenu part.
fn inventory<'a>(cx: &'a InterfaceCx, menu: MenuId) -> &'a InventoryMenu {
    match &cx.engine.menu(menu).class {
        MenuClass::Inventory(inventory) => inventory,
        _ => unreachable!("an InventoryMenu"),
    }
}

// IDA: N/A the menu's InventoryMenu part, mutable.
fn inventory_mut<'a>(cx: &'a mut InterfaceCx, menu: MenuId) -> &'a mut InventoryMenu {
    match &mut cx.engine.menu_mut(menu).class {
        MenuClass::Inventory(inventory) => inventory,
        _ => unreachable!("an InventoryMenu"),
    }
}

// IDA: N/A a field the five-tile check has guaranteed.
fn field(cx: &InterfaceCx, menu: MenuId, id: usize) -> TileId {
    inventory(cx, menu).tiles_28[id - 9].expect("the five tiles")
}

/// IDA: 0057A2D0 InterfaceManager_CreateInventoryMenuIfReady: with a cursor (+0x1C) and +0x60, an old 1002 menu deleted;
/// the depth (00584980); Tile_ReadXML(inventory_menu.xml) under the menu root; its containing menu; 00584670 the cache; null
/// answers null; a menu that isn't 1002 is deleted when it has a tile; 00584880 registers the tile; without the five tiles
/// the warning and null; a stacking type 6006 or 102 takes the depth; the root's user1..user4 -2, user5..user9 0;
/// 005AADC0; B3B3D9 0; InterfaceManager +0x0B other than -1 to 005AACF0; field 12's user5 -999999 then 0; Menu_Enable(1);
/// the root answers. Completion: 100% but the tab arm.
/// TODO: [ ] 005AACF0, a remembered tab.
/// # Errors
/// A stop in the XML, the list or a remembered tab.
pub fn inventory_menu_create(cx: &mut InterfaceCx) -> Result<Option<TileId>, StreamError> {
    let ready = {
        let manager = cx.world.resource::<InterfaceManager>();
        manager.cursor_01c.is_some() && manager.player_scene_node_060.is_some()
    };
    if !ready {
        return Ok(None);
    }
    if let Some(old) = cx.engine.tile_menu_by_id(MENU_ID_INVENTORY)
        && let Some(menu) = cx.engine.containing_menu(old)
    {
        menu_destroy(cx, menu)?;
    }
    let depth = menu_update_cursor_depth(cx)?;
    let root = cx.world.resource::<InterfaceManager>().menu_root_068;
    let created = tile_read_xml(cx, root, INVENTORY_XML)?;
    let menu = created.and_then(|created| cx.engine.containing_menu(created));
    tile_xml_clear_cached_file(cx, INVENTORY_XML, menu);
    let (Some(created), Some(menu)) = (created, menu) else {
        return Ok(None);
    };
    if cx.engine.menu(menu).id() != MENU_ID_INVENTORY {
        if cx.engine.menu(menu).tile_04.is_some() {
            menu_destroy(cx, menu)?;
        }
        return Ok(None);
    }
    menu_register_tile(cx, menu, Some(created))?;
    if !inventory(cx, menu).has_required_tiles() {
        cx.host.warning(
            "Inventory Menu Creation Failed... Are your menu and art resources up to date?",
        );
        return Ok(None);
    }
    let stacking = tile_numeric_trait(cx, created, TRAIT_STACKING);
    if stacking == 6006.0 || stacking == 102.0 {
        tile_set_numeric_trait(cx, created, TRAIT_DEPTH, depth)?;
    }
    for n in 1..=4 {
        tile_set_numeric_trait(cx, created, user(n), -2.0)?;
    }
    for n in 5..=9 {
        tile_set_numeric_trait(cx, created, user(n), 0.0)?;
    }
    inventory_menu_refresh(cx)?;
    cx.world
        .get_resource_or_init::<InventoryMenuStatics>()
        .byte_b3b3d9 = 0;
    if cx.world.resource::<InterfaceManager>().tab_ids_00a[1] as i8 != -1 {
        return Err(StreamError(
            "0057A2D0: a remembered inventory tab (005AACF0) is carried",
        ));
    }
    let scroll = field(cx, menu, 12);
    tile_set_numeric_trait(cx, scroll, user(5), -999_999.0)?;
    tile_set_numeric_trait(cx, scroll, user(5), 0.0)?;
    let frame_time = cx.world.resource::<InterfaceManager>().frame_seconds_host;
    menu_enable(cx, menu, true, frame_time)?;
    Ok(Some(created))
}

/// IDA: 005AADC0 InventoryMenu_RefreshContentTiles: +0x50 freed; B3B3D8 0 and p_quantity -1; with g_thePlayer: each of
/// its 004D8950 entries (004D88F0) that passes 004854F0(player, 0, 1, 1, 0) and isn't gold (form ID 0xF, 00469980) is
/// prepended with its index; the list sorted (005B27A0, 005AA2A0); a focused tile with id 1001 or more clears +0x3C, shows
/// field 9 and drops +0x88; field 10's children with a user10 get listindex -99; the root's user1..user4 -2. Each entry,
/// its group index counting up when its category (00485150, 005AA210) changes: the icon `Icons\` and 005C0C50; an old row
/// of the name updated and moved, else 005AAB60's row (the name, the icon, user9 the category, listindex and id 1001 plus
/// the group index); user2..user5 and user7 the strings 0048F450(1..4, 0), user10 1 + worn, user11 the entry's index,
/// user12 00485C00's flags, user13 0048F6A0 of them; the index up. Field 10's user0 the last index; the -99 rows deleted;
/// 005AA3A0(+0x40); the root's user6 / user7 the weight and its limit (0065DFA0), user8 the armor rating (vt+0x348) and
/// user9 the gold (005E4420), each through __ftol2. Completion: 100% of new rows.
/// TODO: [ ] An old row of the name (005AB234's update and move).
/// # Errors
/// An arm below that stops.
#[allow(clippy::too_many_lines)]
pub fn inventory_menu_refresh(cx: &mut InterfaceCx) -> Result<(), StreamError> {
    let Some(root) = cx.engine.tile_menu_by_id(MENU_ID_INVENTORY) else {
        return Ok(());
    };
    let menu = cx.engine.containing_menu(root).expect("a menu tile");
    {
        let mut statics = cx.world.get_resource_or_init::<InventoryMenuStatics>();
        statics.byte_b3b3d8 = 0;
        statics.quantity = -1;
    }
    let Some(player) = the_player(cx.world) else {
        return Ok(());
    };
    let pane = field(cx, menu, 10);
    let count = reference_inventory_count(cx.world, player)?;
    let mut listed: Vec<(InventoryEntry, i32)> = Vec::new();
    for index in 0..count {
        let Some(entry) = reference_inventory_entry(cx.world, player, index)? else {
            continue;
        };
        if !entry_usable(cx.world, &entry, player)? || form_id(cx.world, entry.form_08) == 0xF {
            continue;
        }
        listed.insert(0, (entry, index));
    }
    shell_sort(&mut listed, |a, b| compare_entries(cx.world, &a.0, &b.0))?;
    let focused = cx.world.resource::<InterfaceManager>().alt_active_tile_088;
    if let Some(focused) = focused
        && tile_numeric_trait(cx, focused, TRAIT_ID) >= 1001.0
    {
        inventory_mut(cx, menu).field_3c = 0;
        let tabs = field(cx, menu, 9);
        tile_set_numeric_trait(cx, tabs, TRAIT_VISIBLE, 1.0)?;
        cx.world
            .resource_mut::<InterfaceManager>()
            .alt_active_tile_088 = None;
    }
    for child in cx.engine.tile(pane).children_30.clone() {
        if find_value(cx, child, user(10)).is_some() {
            tile_set_numeric_trait(cx, child, TRAIT_LIST_INDEX, -99.0)?;
        }
    }
    for n in 1..=4 {
        tile_set_numeric_trait(cx, root, user(n), -2.0)?;
    }
    let mut group = -1_i32;
    let mut last: Option<i32> = None;
    for (entry, index) in &listed {
        let category = item_category(cx.world, entry.form_08);
        let adjusted = adjust_category(cx.world, category, entry.form_08)?;
        if last != Some(adjusted) {
            group += 1;
            last = Some(adjusted);
        }
        let icon = item_icon(cx.world, entry.form_08, player)?;
        let mut icon_path = b"Icons\\".to_vec();
        icon_path.extend_from_slice(&icon);
        let name = item_name(cx.world, entry.form_08);
        for row in cx.engine.tile(pane).children_30.clone().into_iter().rev() {
            if tile_numeric_trait(cx, row, TRAIT_LIST_INDEX) == -99.0
                && tile_string_trait(cx, row, user(1)).is_some_and(|text| text == name)
            {
                return Err(StreamError(
                    "005AADC0: an old row of the same name is carried",
                ));
            }
        }
        let Some(row) = create_row(cx, menu, &icon_path, &name, category, group, group + 1001)?
        else {
            return Err(StreamError("005AAB60: no row template"));
        };
        for (n, kind) in [(2, 1), (3, 2), (4, 3), (5, 4), (7, 0)] {
            let text = item_string(cx.world, entry, kind)?;
            tile_set_string_trait(cx, row, user(n), text.as_deref().unwrap_or_default())?;
        }
        tile_set_numeric_trait(
            cx,
            row,
            user(10),
            f32::from(u8::from(entry_is_worn(entry, false)) + 1),
        )?;
        tile_set_numeric_trait(cx, row, user(11), *index as f32)?;
        let flags = item_flags(cx.world, entry)?;
        tile_set_numeric_trait(cx, row, user(12), flags as f32)?;
        let flag_icon = flag_icon(flags);
        tile_set_string_trait(cx, row, user(13), flag_icon.as_deref().unwrap_or_default())?;
        group += 1;
    }
    tile_set_numeric_trait(cx, pane, user(0), (group - 1) as f32)?;
    for child in cx.engine.tile(pane).children_30.clone() {
        if tile_numeric_trait(cx, child, TRAIT_LIST_INDEX) == -99.0 {
            tile_destroy(cx, child)?;
        }
    }
    let filter = inventory(cx, menu).filter_40;
    inventory_menu_apply_filter(cx, menu, filter)?;
    let (weight, limit) = player_weight_and_limit(cx.world, player)?;
    tile_set_numeric_trait(cx, root, user(6), x87::ftol(weight) as f32)?;
    tile_set_numeric_trait(cx, root, user(7), x87::ftol(limit) as f32)?;
    let rating = character_armor_rating(cx.world, player)?;
    tile_set_numeric_trait(cx, root, user(8), x87::ftol(rating) as f32)?;
    let gold = reference_gold(cx.world, player)?;
    tile_set_numeric_trait(cx, root, user(9), gold as f32)?;
    Ok(())
}

// IDA: 005AAB60 InventoryMenu_CreateRow(icon, name, category, index, id): `item_template` under field 10 (00585410; the
// menu's one template), user1 the name, the tile's name the first 255 bytes with ' ' as '_', user6 the icon, user9 the
// category, listindex the index, then the id.
fn create_row(
    cx: &mut InterfaceCx,
    menu: MenuId,
    icon: &[u8],
    name: &[u8],
    category: i32,
    index: i32,
    id: i32,
) -> Result<Option<TileId>, StreamError> {
    let pane = field(cx, menu, 10);
    let Some(row) = tile_build_template(cx, menu, pane, b"item_template")? else {
        return Ok(None);
    };
    tile_set_string_trait(cx, row, user(1), name)?;
    cx.engine.tile_mut(row).name_08 = name
        .iter()
        .take(255)
        .map(|&byte| if byte == b' ' { b'_' } else { byte })
        .collect();
    tile_set_string_trait(cx, row, user(6), icon)?;
    tile_set_numeric_trait(cx, row, user(9), category as f32)?;
    tile_set_numeric_trait(cx, row, TRAIT_LIST_INDEX, index as f32)?;
    tile_set_numeric_trait(cx, row, TRAIT_ID, id as f32)?;
    Ok(Some(row))
}

/// IDA: 005AA3A0 InventoryMenu_ApplyFilter(mask): the root's user1..user4 and user13..user17 and field 13's user1..user4
/// -2; field 10's rows from the tail, each its user9 (__ftol2) as the category, refined by 005AA210 over its 004D88F0 entry
/// (user11): a category other than the last that the mask takes first gives its heading the current index (1 user1, 2
/// user2; with the mask 4 or 8 field 13's user1..user4 for 4/8, 5/9, 6/10, 11; from 15 the root's user3, user4, user13..
/// user17 for 4, 5, 6, 8, 9, 10, 11) and takes one; then a row the mask takes by its raw user9 is shown (user8 2, listindex
/// the index), the first past +0x54 also taking InterfaceManager +0x8C's next value as xdefault; any other is hidden
/// (user8 1, listindex -2 when its category is at most the mask, else 99999). Field 10's user0 and the root's user5 the
/// last index (0 below that); an unset heading takes the next one's. Completion: 100%.
/// # Errors
/// A stop in the entry walk or the trait writes.
#[allow(clippy::too_many_lines)]
pub fn inventory_menu_apply_filter(
    cx: &mut InterfaceCx,
    menu: MenuId,
    mask: u32,
) -> Result<(), StreamError> {
    let root = cx.engine.menu(menu).tile_04.expect("a registered menu");
    let pane = field(cx, menu, 10);
    let second = field(cx, menu, 13);
    let threshold = inventory(cx, menu).unset_54;
    let mask = mask as i32;
    for n in [1, 2, 3, 4, 13, 14, 15, 16, 17] {
        tile_set_numeric_trait(cx, root, user(n), -2.0)?;
    }
    for n in 1..=4 {
        tile_set_numeric_trait(cx, second, user(n), -2.0)?;
    }
    let player = the_player(cx.world);
    let mut next = -1_i32;
    let mut index = -1_i32;
    let mut last = 0_i32;
    let mut defaulted = false;
    for row in cx.engine.tile(pane).children_30.clone().into_iter().rev() {
        let raw = x87::ftol(tile_numeric_trait(cx, row, user(9)));
        let mut category = raw;
        if let Some(player) = player {
            let at = x87::ftol(tile_numeric_trait(cx, row, user(11)));
            if let Some(entry) = reference_inventory_entry(cx.world, player, at)? {
                category = adjust_category(cx.world, category, entry.form_08)?;
            }
        }
        if category != last && category & mask != 0 {
            let heading = match category {
                1 => Some((root, 1)),
                2 => Some((root, 2)),
                _ if mask == 4 || mask == 8 => match category {
                    4 | 8 => Some((second, 1)),
                    5 | 9 => Some((second, 2)),
                    6 | 10 => Some((second, 3)),
                    11 => Some((second, 4)),
                    _ => None,
                },
                _ if mask < 15 => None,
                4 => Some((root, 3)),
                5 => Some((root, 4)),
                6 => Some((root, 13)),
                8 => Some((root, 14)),
                9 => Some((root, 15)),
                10 => Some((root, 16)),
                11 => Some((root, 17)),
                _ => None,
            };
            if let Some((tile, n)) = heading {
                tile_set_numeric_trait(cx, tile, user(n), index as f32)?;
                next += 1;
                index = next;
            }
        }
        last = category;
        if mask & raw != 0 {
            tile_set_numeric_trait(cx, row, user(8), 2.0)?;
            tile_set_numeric_trait(cx, row, TRAIT_LIST_INDEX, index as f32)?;
            next += 1;
            index = next;
            if next > threshold && !defaulted {
                let priority = {
                    let mut manager = cx.world.resource_mut::<InterfaceManager>();
                    manager.default_selection_priority_08c += 1;
                    manager.default_selection_priority_08c
                };
                tile_set_numeric_trait(cx, row, TRAIT_XDEFAULT, priority as f32)?;
                defaulted = true;
            }
        } else {
            tile_set_numeric_trait(cx, row, user(8), 1.0)?;
            let hidden = if category <= mask { -2.0 } else { 99_999.0 };
            tile_set_numeric_trait(cx, row, TRAIT_LIST_INDEX, hidden)?;
        }
    }
    let shown = (next - 1).max(0) as f32;
    tile_set_numeric_trait(cx, pane, user(0), shown)?;
    tile_set_numeric_trait(cx, root, user(5), shown)?;
    for (n, from) in [
        (17, 5),
        (16, 17),
        (15, 16),
        (14, 15),
        (13, 14),
        (4, 13),
        (3, 4),
        (2, 3),
        (1, 2),
    ] {
        if tile_numeric_trait(cx, root, user(n)) == -2.0 {
            let value = tile_numeric_trait(cx, root, user(from));
            tile_set_numeric_trait(cx, root, user(n), value)?;
        }
    }
    if tile_numeric_trait(cx, second, user(4)) == -2.0 {
        let value = tile_numeric_trait(cx, root, user(5));
        tile_set_numeric_trait(cx, second, user(4), value)?;
    }
    for n in [3, 2, 1] {
        if tile_numeric_trait(cx, second, user(n)) == -2.0 {
            let value = tile_numeric_trait(cx, second, user(n + 1));
            tile_set_numeric_trait(cx, second, user(n), value)?;
        }
    }
    Ok(())
}

// IDA: N/A a form's ID.
fn form_id(world: &World, form: Entity) -> u32 {
    world
        .get::<FormIdentity>(form)
        .map_or(0, |id| id.form_id_0c)
}

// IDA: 004D7030 TESForm_IsPersistent (vt+0x78): the flag 0x400.
fn persistent(world: &World, form: Entity) -> bool {
    world
        .get::<FormIdentity>(form)
        .is_some_and(|id| id.flags_08 & FORM_FLAG_PERSISTENT != 0)
}

// IDA: 00485150 InventoryEntry_GetCategory: types 0x13, 0x19, 0x28 4; 0x14, 0x16 2; 0x21, 0x22 1; else 8.
fn item_category(world: &World, form: Entity) -> i32 {
    match form_type(world, form) {
        0x13 | 0x19 | 0x28 => 4,
        0x14 | 0x16 => 2,
        0x21 | 0x22 => 1,
        _ => 8,
    }
}

// IDA: 005AA210 InventoryMenu_RefineCategory: types 0x19 6, 0x28 5, 0x13 4, 0x15 8, 0x27 9, 0x26 and 0x2A or the Welkynd
// and Varla stones 10; any other keeps the category, 8 becoming 11.
fn adjust_category(world: &World, category: i32, form: Entity) -> Result<i32, StreamError> {
    Ok(match form_type(world, form) {
        0x19 => 6,
        0x28 => 5,
        0x13 => 4,
        0x15 => 8,
        0x27 => 9,
        0x26 | 0x2A => 10,
        // The Welkynd and Varla stones are misc items (00B3...); their globals aren't held, so a misc item stops.
        0x22 => {
            return Err(StreamError(
                "005AA210: a misc item's Welkynd/Varla test is carried",
            ));
        }
        _ if category == 8 => 11,
        _ => category,
    })
}

// IDA: 00488DF0 InventoryEntry_GetName: the form's TESFullName, its empty or absent text the setting at 00B38D30, null
// data "".
fn item_name(world: &World, form: Entity) -> Vec<u8> {
    let name = world
        .get::<TesFullName>(form)
        .and_then(|name| name.name_04.data_00.clone())
        .map(|name| {
            let end = name
                .iter()
                .position(|&byte| byte == 0)
                .unwrap_or(name.len());
            name[..end].to_vec()
        });
    match name {
        Some(name) if !name.is_empty() => name,
        _ => world
            .get_resource::<super::game_settings::GameSettingCollection>()
            .and_then(|settings| settings.text_at(0x00B3_8D30))
            .unwrap_or_default(),
    }
}

// IDA: 005C0C50 InventoryMenu_GetItemIcon: a TESBipedModelForm's icon for the player's sex (00469380: +0x6C + 12 * sex,
// a female's empty icon falling back to the male's); else 004702D0; an empty answer is null.
fn item_icon(world: &World, form: Entity, player: Entity) -> Result<Vec<u8>, StreamError> {
    if let Some(biped) = world.get::<TesBipedModelForm>(form) {
        let base = world
            .get::<TesObjectRefr>(player)
            .and_then(|refr| refr.base_form_1c)
            .ok_or(StreamError("005C0C50: a player with no base form"))?;
        let Some(mut sex) = npc_sex(world, base) else {
            return Err(StreamError("00469384: a sex of -1 reads before the icons"));
        };
        let path = |sex: usize| {
            biped.icons_68[sex]
                .path_04
                .data_00
                .clone()
                .map(|path| {
                    let end = path
                        .iter()
                        .position(|&byte| byte == 0)
                        .unwrap_or(path.len());
                    path[..end].to_vec()
                })
                .unwrap_or_default()
        };
        if path(sex).is_empty() && sex == 1 {
            sex = 0;
        }
        let icon = path(sex);
        if !icon.is_empty() {
            return Ok(icon);
        }
    }
    Err(StreamError(
        "005C0C50: an icon through 004702D0, or none, is carried",
    ))
}

// IDA: 004854F0(player, 0, 1, 1, 0): an armor's or clothing's biped flag 0x40 (not playable, 00469080) refuses; a
// persistent form (vt+0x78) while the player's +0x200 jail flag is set refuses; with a5 set, the rest is skipped.
fn entry_usable(
    world: &World,
    entry: &InventoryEntry,
    player: Entity,
) -> Result<bool, StreamError> {
    let form = entry.form_08;
    if matches!(form_type(world, form), 0x14 | 0x16)
        && world
            .get::<TesBipedModelForm>(form)
            .is_some_and(|biped| biped.flags_06 & 0x40 != 0)
    {
        return Ok(false);
    }
    if persistent(world, form)
        && world
            .get::<PlayerCharacter>(player)
            .is_some_and(|data| data.jail_sentence_active_200 != 0)
    {
        return Ok(false);
    }
    Ok(true)
}

// IDA: 005AA2A0 InventoryMenu_CompareEntries(a, b): the refined categories (higher after); then 00584500 with the sort
// byte 00B3B3DB (key 0: _mbsicmp of the names, its sign the order); then 004852B0's health percentages (lower after);
// then a worn entry first. Only key 0 is reached; another key stops.
fn compare_entries(
    world: &World,
    a: &InventoryEntry,
    b: &InventoryEntry,
) -> Result<i32, StreamError> {
    let left = adjust_category(world, item_category(world, a.form_08), a.form_08)?;
    let right = adjust_category(world, item_category(world, b.form_08), b.form_08)?;
    if left != right {
        return Ok(if left > right { 1 } else { -1 });
    }
    let sort = world
        .get_resource::<InventoryMenuStatics>()
        .map_or(0, |statics| statics.sort_b3b3db);
    if sort & 0x7F != 0 {
        return Err(StreamError(
            "00584500: an inventory sort key other than the name is carried",
        ));
    }
    let mut by_name =
        recomp_core::crt::stricmp(&item_name(world, a.form_08), &item_name(world, b.form_08))
            .signum();
    if sort < 0 {
        by_name = -by_name;
    }
    if by_name != 0 {
        return Ok(by_name);
    }
    let (first, second) = (
        item_health_percent(world, a)?,
        item_health_percent(world, b)?,
    );
    if first < second {
        return Ok(1);
    }
    if first > second {
        return Ok(-1);
    }
    let (worn_a, worn_b) = (entry_is_worn(a, false), entry_is_worn(b, false));
    Ok(match (worn_a, worn_b) {
        (false, true) => 1,
        (true, false) => -1,
        _ => 0,
    })
}

// IDA: 00506F00's shell sort (h = 3h + 1 from 1 while h <= (n - 1) / 9, then down by thirds; each pass an insertion that
// stops at a compare of 0 or more).
fn shell_sort<T: Clone>(
    items: &mut [T],
    mut compare: impl FnMut(&T, &T) -> Result<i32, StreamError>,
) -> Result<(), StreamError> {
    let count = items.len() as i32;
    let mut gap = 1_i32;
    if (count - 1) / 9 >= 1 {
        while gap <= (count - 1) / 9 {
            gap = 3 * gap + 1;
        }
    }
    while gap > 0 {
        for start in gap..count {
            let held = items[start as usize].clone();
            let mut at = start;
            while at >= gap {
                if compare(&held, &items[(at - gap) as usize])? >= 0 {
                    break;
                }
                items[at as usize] = items[(at - gap) as usize].clone();
                at -= gap;
            }
            items[at as usize] = held;
        }
        gap /= 3;
    }
    Ok(())
}

// IDA: 004852B0 InventoryEntry_GetHealthPercent(entry, 1): a TESHealthForm's health percentage (not ported: it stops); a
// TESUsesForm that isn't a sigil stone, 00484970's uses (stops); else -1.0.
fn item_health_percent(world: &World, entry: &InventoryEntry) -> Result<f32, StreamError> {
    if world.get::<TesHealthForm>(entry.form_08).is_some() {
        return Err(StreamError(
            "004852B0: an item's health percentage (InventoryEntry_GetHealth) is carried",
        ));
    }
    if world.get::<TesUsesForm>(entry.form_08).is_some() && form_type(world, entry.form_08) != 0x2D
    {
        return Err(StreamError(
            "004852B0: an item's uses (00484970) are carried",
        ));
    }
    Ok(-1.0)
}

// IDA: 0048F450 InventoryEntry_FormatStat(kind, ...): 0 the count ("%i" above 1, " " at or below, the K and M forms
// above 999 and 999999 stop); 1 the value (00488E50), 2 the weight (00485260), 3 the quality (004891C0), 4 the health
// (004852B0): below 0.0 (or NaN) null; between 0 and 1 rounded to tenths (004843B0) and "%.1f" if still between; else
// __ftol2 and "%d".
fn item_string(
    world: &World,
    entry: &InventoryEntry,
    kind: u32,
) -> Result<Option<Vec<u8>>, StreamError> {
    let value = match kind {
        0 => {
            let count = entry.count_04;
            if count > 999 {
                return Err(StreamError(
                    "0048F4BE: a count above 999's K or M form is carried",
                ));
            }
            return Ok(Some(if count > 1 {
                count.to_string().into_bytes()
            } else {
                b" ".to_vec()
            }));
        }
        1 => item_value(world, entry)?,
        2 => item_weight(world, entry.form_08),
        3 => item_quality(world, entry)?,
        4 => item_health_percent(world, entry)?,
        _ => return Ok(None),
    };
    // 0048F5E7: `fcom` 0.0 with C0 set below it or unordered, so a NaN is null too.
    if value < 0.0 || value.is_nan() {
        return Ok(None);
    }
    let mut value = value;
    if value > 0.0 && value < 1.0 {
        value = round_to(value, 0.1);
    }
    if value > 0.0 && value < 1.0 {
        // 0048F646: "%.1f" of the double. Rust rounds a tie to even.
        return Ok(Some(format!("{:.1}", f64::from(value)).into_bytes()));
    }
    Ok(Some(x87::ftol(value).to_string().into_bytes()))
}

// IDA: 004843B0(value, step): value / step through __ftol2, one more when the fraction is 0.5 or more, times the step.
fn round_to(value: f32, step: f32) -> f32 {
    let scaled = value / step;
    let mut whole = x87::ftol(scaled);
    if scaled - whole as f32 >= 0.5 {
        whole += 1;
    }
    step * whole as f32
}

// IDA: 00488E50(entry, 0): a TESValueForm's value (`fild`), unless it's enchantable with an enchantment (00484340, stops);
// a magic item's cost (stops); else -1.0.
fn item_value(world: &World, entry: &InventoryEntry) -> Result<f32, StreamError> {
    let form = entry.form_08;
    let Some(value) = world.get::<TesValueForm>(form) else {
        if world
            .get::<super::forms_magic::EffectItemList>(form)
            .is_some()
        {
            return Err(StreamError("00488E50: a magic item's value is carried"));
        }
        return Ok(-1.0);
    };
    if world
        .get::<TesEnchantableForm>(form)
        .is_some_and(|enchantable| {
            !matches!(
                enchantable.enchantment_04,
                FormRef::Resolved(None) | FormRef::Raw(0)
            )
        })
    {
        return Err(StreamError(
            "00488E50: an enchanted item's value is carried",
        ));
    }
    Ok(value.value_04 as f32)
}

// IDA: 00485260 / 00470620 Form_GetItemWeight: the TESWeightForm's weight, 0.0 for a persistent form; -1.0 with none.
fn item_weight(world: &World, form: Entity) -> f32 {
    match world.get::<TesWeightForm>(form) {
        Some(_) if persistent(world, form) => 0.0,
        Some(weight) => weight.weight_04,
        None => -1.0,
    }
}

// IDA: 004891C0 InventoryEntry_GetQuality: a TESQualityForm's quality; a weapon (category 1) or armor (0x14) stops;
// else -1.0.
fn item_quality(world: &World, entry: &InventoryEntry) -> Result<f32, StreamError> {
    if let Some(quality) = world.get::<TesQualityForm>(entry.form_08) {
        return Ok(quality.quality_04);
    }
    if item_category(world, entry.form_08) == 1 || form_type(world, entry.form_08) == 0x14 {
        return Err(StreamError(
            "004891C0: a weapon's or armor's rating is carried",
        ));
    }
    Ok(-1.0)
}

// IDA: 00485C00 InventoryEntry_GetStatusFlags: 1 another owner on the first list, 2 an enchantment, 4 poison (0x48), 8 a
// health of 0. Only the empty cases are ported; any of these stops.
fn item_flags(world: &World, entry: &InventoryEntry) -> Result<u32, StreamError> {
    if entry
        .lists_00
        .as_ref()
        .and_then(|lists| lists.first())
        .is_some_and(|list| list.get(0x27).is_some() || list.get(0x48).is_some())
    {
        return Err(StreamError(
            "00485C00: an owned or poisoned item's flags are carried",
        ));
    }
    let mut flags = 0;
    if world
        .get::<TesEnchantableForm>(entry.form_08)
        .is_some_and(|enchantable| {
            !matches!(
                enchantable.enchantment_04,
                FormRef::Resolved(None) | FormRef::Raw(0)
            )
        })
    {
        flags |= 2;
    }
    if world.get::<TesHealthForm>(entry.form_08).is_some() {
        return Err(StreamError("00485C00: an item's health flag is carried"));
    }
    Ok(flags)
}

// IDA: 0048F6A0: 1 "Icons\icon_small_item_pickup.dds", 2 "...icon_enchanted_item.dds", 4 "...icon_poisoned_weapon.dds",
// 8 "...icon_broken_item.dds", another positive "Icons\icon_image_set_%d.dds"; 0 null.
fn flag_icon(flags: u32) -> Option<Vec<u8>> {
    let name = match flags {
        0 => return None,
        1 => "icon_small_item_pickup.dds".to_owned(),
        2 => "icon_enchanted_item.dds".to_owned(),
        4 => "icon_poisoned_weapon.dds".to_owned(),
        8 => "icon_broken_item.dds".to_owned(),
        other => format!("icon_image_set_{other}.dds"),
    };
    Some(format!("Icons\\{name}").into_bytes())
}

/// IDA: 0065DFA0 PlayerCharacter_GetWeightAndLimit: the limit 005E0D20 (fActorStrengthEncumbranceMult 00B37570 times
/// Strength through vt+0x288, at least 0); the weight 004D8FB0; the modifiers of 11 ((+0x350 + +0x230) + +0x47C): below 0
/// the limit less them, else the weight plus them. Completion: 100%.
/// # Errors
/// As the weight and the actor value.
pub fn player_weight_and_limit(
    world: &mut World,
    player: Entity,
) -> Result<(f32, f32), StreamError> {
    let strength = player_get_actor_value_float(world, player, 0)?;
    let scaled = setting_float(world, 0x00B3_7570) * strength;
    let limit = if scaled >= 0.0 { scaled } else { 0.0 };
    let weight = reference_weight(world, player)?;
    let body = world.get::<PlayerCharacter>(player).expect("the player");
    let modifiers = (body.script_av_modifiers_324[11] + body.max_av_modifiers_204[11])
        + body.damage_av_modifiers_450[11];
    Ok(if modifiers < 0.0 {
        (weight, limit - modifiers)
    } else {
        (modifiers + weight, limit)
    })
}

// IDA: 004D8FB0 TESObjectREFR_GetEncumbrance for the player: with a base container, the block's total (00487E40) or, with
// none, the container's (004698B0, stops); 0.0 without one.
fn reference_weight(world: &mut World, player: Entity) -> Result<f32, StreamError> {
    let Some(base) = super::inventory::reference_container(world, player) else {
        return Ok(0.0);
    };
    let has_changes = world.get::<TesObjectRefr>(player).is_some_and(|refr| {
        refr.extra_data_44
            .get(super::base_extra_list::EXTRA_CONTAINER_CHANGES)
            .is_some()
    });
    if !has_changes {
        return Err(StreamError(
            "004D8FB0: a base container's weight (004698B0) is carried",
        ));
    }
    let mut changes = ensure_container_changes(world, player)?;
    let total = changes_total_weight(world, &mut changes, base);
    put_container_changes(world, player, changes);
    let total = total?;
    Ok(total)
}

// IDA: 00487E40 ContainerChanges_GetTotalWeight: the block's +0x08 when it isn't -1.0; else the base items that aren't
// persistent, weight (-1.0 as 0.0) times their count merged with the block's (a zero sum adds nothing), then the block's
// own items the container doesn't list (armor worn by the owner stops); stored at +0x08, and for an actor owner its
// process's vt+0x290 (006298C0: +0x294 = -1.0).
fn changes_total_weight(
    world: &mut World,
    changes: &mut ContainerChanges,
    base: Entity,
) -> Result<f32, StreamError> {
    if changes.float_08 != -1.0 {
        return Ok(changes.float_08);
    }
    let container = world.get::<TesContainer>(base).cloned().unwrap_or_default();
    let mut total = 0.0_f32;
    for item in &container.entries_08 {
        let FormRef::Resolved(Some(form)) = item.item else {
            break;
        };
        if persistent(world, form) {
            continue;
        }
        let mut weight = item_weight(world, form);
        if weight == -1.0 {
            weight = 0.0;
        }
        match changes
            .entries_00
            .iter()
            .find(|entry| entry.form_08 == form)
        {
            Some(entry) => {
                let sum = item.count + entry.count_04;
                if sum != 0 {
                    total += weight * sum as f32;
                }
            }
            None => total += weight * item.count as f32,
        }
    }
    for entry in &changes.entries_00 {
        let form = entry.form_08;
        let listed = container.flags_04 & 1 != 0
            && container
                .entries_08
                .iter()
                .any(|item| item.item == FormRef::Resolved(Some(form)));
        if listed || persistent(world, form) {
            continue;
        }
        if form_type(world, form) == 0x14 && entry_is_worn(entry, false) {
            return Err(StreamError(
                "00487FF2: worn armor's weight perks are carried",
            ));
        }
        if entry.count_04 > 0 {
            total += entry.count_04 as f32 * item_weight(world, form).max(0.0);
        }
    }
    changes.float_08 = total;
    let process = world
        .get::<MobileObject>(changes.owner_04)
        .and_then(|mobile| mobile.process_58);
    if let Some(process) = process
        && let Some(mut high) = world.get_mut::<HighProcess>(process)
    {
        high.float_294 = -1.0;
    }
    Ok(total)
}

/// IDA: 0060E580 Character_GetArmorRating (vt+0x348): the cache +0x108 recomputed when it's below 0.0 (or NaN) from the
/// equipped armor (none here: the cache becomes 0.0; any armor stops); the light-armor master arm (Light Armor's mastery 4,
/// 005F23B0) stops; plus DefendBonus (43, vt+0x288); capped at fMaxArmorRating (00B37D20) when that's above 0.
/// Completion: 100% with no armor.
/// # Errors
/// Armor in the inventory, the master arm, or an actor value that stops.
pub fn character_armor_rating(world: &mut World, player: Entity) -> Result<f32, StreamError> {
    let cache = world
        .get::<Character>(player)
        .map_or(0.0, |data| data.armor_rating_cache_108);
    // 0060E580: neither above nor equal to 0.0 (below, or NaN) recomputes.
    if cache < 0.0 || cache.is_nan() {
        let forms = super::magic_menu::inventory_forms(world, player);
        if forms.into_iter().any(|form| form_type(world, form) == 0x14) {
            return Err(StreamError("0060E580: equipped armor's rating is carried"));
        }
        if let Some(mut data) = world.get_mut::<Character>(player) {
            data.armor_rating_cache_108 = 0.0;
        }
    }
    let rating = world
        .get::<Character>(player)
        .map_or(0.0, |data| data.armor_rating_cache_108);
    if skill_mastery(world, player, 27)? == 4 {
        return Err(StreamError(
            "0060E580: the light-armor master arm is carried",
        ));
    }
    let total = player_get_actor_value_float(world, player, 43)? + rating;
    let cap = setting_float(world, 0x00B3_7D20);
    Ok(if cap <= 0.0 || total < cap {
        total
    } else {
        cap
    })
}

// IDA: 005F23B0 Actor_GetSkillMasteryLevel: skills 12..32 by their floored calculated value through 0056A300 (below
// iSkillApprenticeMin 0, JourneymanMin 1, ExpertMin 2, then MasterMin 4 or 3); others 0.
fn skill_mastery(world: &World, player: Entity, skill: u32) -> Result<i32, StreamError> {
    if !(12..=32).contains(&skill) {
        return Ok(0);
    }
    let value = floor_calculated_actor_value(world, player, skill)?;
    let at = |address: u32| setting_int(world, address);
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

// IDA: 005E4420 / 004875C0: the gold (form ID 0xF) in the inventory; any stops, none is 0.
fn reference_gold(world: &World, player: Entity) -> Result<i32, StreamError> {
    if super::magic_menu::inventory_forms(world, player)
        .into_iter()
        .any(|form| form_id(world, form) == 0xF)
    {
        return Err(StreamError("004875C0: gold in the inventory is carried"));
    }
    Ok(0)
}

// IDA: a float setting's bits at its address.
fn setting_float(world: &World, address: u32) -> f32 {
    f32::from_bits(
        world
            .resource::<super::game_settings::GameSettingCollection>()
            .raw_at(address)
            .unwrap_or(0),
    )
}

// IDA: an int setting's bits at its address.
fn setting_int(world: &World, address: u32) -> i32 {
    world
        .resource::<super::game_settings::GameSettingCollection>()
        .raw_at(address)
        .unwrap_or(0) as i32
}
