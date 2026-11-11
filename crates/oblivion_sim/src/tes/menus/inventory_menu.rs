//! InventoryMenu (1002), as 0057E150's second creation builds it: 0057A2D0 over `Data\Menus\Main\inventory_menu.xml`, the
//! XML's fields (005A9B40 SetField) and 005AADC0's list: the player's inventory entries (`inventory_entries`) that pass
//! 004854F0, sorted (005AA2A0), each a row from the menu's only template with its strings (0048F450), then 005AA3A0's tab
//! filter and the root's weight, armor rating and gold. The run's player holds the four prison clothes, unworn; an arm
//! they don't reach (an enchanted, owned, poisoned or damaged item, armor, gold, a matched old row) stops at its address.

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;
use oblivion_core::x87;

use crate::tes::actor::TesObjectRefr;
use crate::tes::equipment::character_armor_rating;
use crate::tes::form::FormIdentity;
use crate::tes::form_components::TesBipedModelForm;
use crate::tes::forms_actors::npc_sex;
use crate::tes::forms_init::form_type;
use crate::tes::forms_references::reference_gold;
use crate::tes::interface_frame::{TRAIT_XDEFAULT, tile_destroy};
use crate::tes::interface_manager::InterfaceManager;
use crate::tes::inventory::InventoryEntry;
use crate::tes::inventory_entries::{
    entry_is_worn, entry_usable, flag_icon, item_category, item_flags, item_health_percent,
    item_name, item_string, reference_inventory_count, reference_inventory_entry,
};
use crate::tes::player::{player_weight_and_limit, the_player};
use crate::tes::tile::{InterfaceCx, MenuClass, MenuId, TileId};
use crate::tes::tile_values::{
    TRAIT_ID, TRAIT_USER0, TRAIT_VISIBLE, find_value, tile_numeric_trait, tile_set_numeric_trait,
    tile_set_string_trait, tile_string_trait,
};
use crate::tes::tile_xml::tile_build_template;

/// IDA: MenuID_Inventory.
pub const MENU_ID_INVENTORY: u32 = 1002;
/// IDA: TileTrait 0xFAA, listindex.
const TRAIT_LIST_INDEX: u16 = 4010;

// IDA: TileTrait userN, 4014 + N.
pub(crate) const fn user(n: u16) -> u16 {
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
pub(crate) fn inventory<'a>(cx: &'a InterfaceCx, menu: MenuId) -> &'a InventoryMenu {
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
pub(crate) fn field(cx: &InterfaceCx, menu: MenuId, id: usize) -> TileId {
    inventory(cx, menu).tiles_28[id - 9].expect("the five tiles")
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
pub fn inventory_menu_refresh(cx: &mut InterfaceCx) -> Result<(), GameError> {
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
    // 00506F00's shell sort.
    oblivion_core::sort::shell_sort(&mut listed, |a, b| compare_entries(cx.world, &a.0, &b.0))?;
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
                return Err(GameError::unclassified(
                    0x005AADC0,
                    "005AADC0: an old row of the same name is carried",
                ));
            }
        }
        let Some(row) = create_row(cx, menu, &icon_path, &name, category, group, group + 1001)?
        else {
            return Err(GameError::unclassified(
                0x005AAB60,
                "005AAB60: no row template",
            ));
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
) -> Result<Option<TileId>, GameError> {
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
) -> Result<(), GameError> {
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
pub(crate) fn form_id(world: &World, form: Entity) -> u32 {
    world
        .get::<FormIdentity>(form)
        .map_or(0, |id| id.form_id_0c)
}

// IDA: 005AA210 InventoryMenu_RefineCategory: types 0x19 6, 0x28 5, 0x13 4, 0x15 8, 0x27 9, 0x26 and 0x2A or the Welkynd
// and Varla stones 10; any other keeps the category, 8 becoming 11.
fn adjust_category(world: &World, category: i32, form: Entity) -> Result<i32, GameError> {
    Ok(match form_type(world, form) {
        0x19 => 6,
        0x28 => 5,
        0x13 => 4,
        0x15 => 8,
        0x27 => 9,
        0x26 | 0x2A => 10,
        // The Welkynd and Varla stones are misc items (00B3...); their globals aren't held, so a misc item stops.
        0x22 => {
            return Err(GameError::unclassified(
                0x005AA210,
                "005AA210: a misc item's Welkynd/Varla test is carried",
            ));
        }
        _ if category == 8 => 11,
        _ => category,
    })
}

// IDA: 005C0C50 InventoryMenu_GetItemIcon: a TESBipedModelForm's icon for the player's sex (00469380: +0x6C + 12 * sex,
// a female's empty icon falling back to the male's); else 004702D0; an empty answer is null.
fn item_icon(world: &World, form: Entity, player: Entity) -> Result<Vec<u8>, GameError> {
    if let Some(biped) = world.get::<TesBipedModelForm>(form) {
        let base = world
            .get::<TesObjectRefr>(player)
            .and_then(|refr| refr.base_form_1c)
            .ok_or(GameError::unclassified(
                0x005C0C50,
                "005C0C50: a player with no base form",
            ))?;
        let Some(mut sex) = npc_sex(world, base) else {
            return Err(GameError::unclassified(
                0x00469384,
                "00469384: a sex of -1 reads before the icons",
            ));
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
    Err(GameError::unclassified(
        0x005C0C50,
        "005C0C50: an icon through 004702D0, or none, is carried",
    ))
}

// IDA: 005AA2A0 InventoryMenu_CompareEntries(a, b): the refined categories (higher after); then 00584500 with the sort
// byte 00B3B3DB (key 0: _mbsicmp of the names, its sign the order); then 004852B0's health percentages (lower after);
// then a worn entry first. Only key 0 is reached; another key stops.
fn compare_entries(
    world: &World,
    a: &InventoryEntry,
    b: &InventoryEntry,
) -> Result<i32, GameError> {
    let left = adjust_category(world, item_category(world, a.form_08), a.form_08)?;
    let right = adjust_category(world, item_category(world, b.form_08), b.form_08)?;
    if left != right {
        return Ok(if left > right { 1 } else { -1 });
    }
    let sort = world
        .get_resource::<InventoryMenuStatics>()
        .map_or(0, |statics| statics.sort_b3b3db);
    if sort & 0x7F != 0 {
        return Err(GameError::unclassified(
            0x00584500,
            "00584500: an inventory sort key other than the name is carried",
        ));
    }
    let mut by_name =
        oblivion_core::crt::stricmp(&item_name(world, a.form_08), &item_name(world, b.form_08))
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

/// IDA: 005AA1B0 InventoryMenu vt+0x2C: a +0x44 count of 0 hides a tile (the count drops to -1 and stays); then 0057C140
/// for 1002, and 005C1F70 or QuickKeysMenu_Close with 00B3B43D. Completion: the arms with the count set and no input.
/// TODO: [ ] the count's arm; 005C1F70 with input.
/// # Errors
/// Those arms.
pub fn inventory_frame(
    cx: &mut InterfaceCx,
    menu: MenuId,
    _frame_time: f32,
) -> Result<(), GameError> {
    if let MenuClass::Inventory(inventory) = &cx.engine.menu(menu).class
        && inventory.byte_44 == 0
    {
        return Err(GameError::unclassified(
            0x005AA1B9,
            "005AA1B9: InventoryMenu's +0x44 count at 0",
        ));
    }
    if crate::tes::interface_manager::is_menu_input_active(cx.world, cx.engine, 1002) {
        return Err(GameError::unclassified(
            0x005AA1F3,
            "005AA1F3: 005C1F70 for the Inventory menu",
        ));
    }
    Ok(())
}
