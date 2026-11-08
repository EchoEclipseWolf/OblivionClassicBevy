//! MagicMenu (1022), as 0057E150's third creation builds it: 0057A180 then 005B3760 over `Data\Menus\Main\magic_menu.xml`,
//! the XML's fields (005B13C0 SetField) and 005B2B70's list: the player base's spells, sorted in place (005B27A0 with
//! 005B2430), each a row from the menu's only template, with its magicka cost for the player; then 005B1A40's filter and
//! the spell effectiveness line. The run's player holds Flare and Heal Minor Wounds, clothing and no active effect, so
//! the scroll and effect arms find nothing; a book, armor, an effect or a matched old row stops at its address.

use bevy_ecs::prelude::*;
use recomp_core::reader::StreamError;
use recomp_core::x87;

use super::actor::TesObjectRefr;
use super::ai_conditions::the_player;
use super::form_components::{FormRef, TesContainer, TesFullName, TesSpellList};
use super::forms_init::form_type;
use super::forms_magic::{EffectItemList, EffectSetting, MagicFormData};
use super::interface_frame::{
    interface_dispatch_pointer_event, menu_destroy, menu_enable, tile_destroy,
};
use super::interface_manager::{InterfaceManager, menu_register_tile, menu_update_cursor_depth};
use super::magic_selection::{effect_cost, highest_cost_effect};
use super::player::{PlayerCharacter, floor_calculated_actor_value, player_get_actor_value};
use super::tile::{InterfaceCx, MenuClass, MenuId, TileId};
use super::tile_values::{
    TRAIT_DEPTH, TRAIT_ID, TRAIT_STACKING, TRAIT_USER0, tile_numeric_trait, tile_set_numeric_trait,
    tile_set_string_trait, tile_string_trait,
};
use super::tile_xml::{tile_build_template, tile_read_xml, tile_xml_clear_cached_file};

/// IDA: MenuID_Magic.
pub const MENU_ID_MAGIC: u32 = 1022;
const MAGIC_XML: &str = "Data\\Menus\\Main\\magic_menu.xml";
/// IDA: TileTrait 0xFAA, listindex.
const TRAIT_LIST_INDEX: u16 = 4010;
/// IDA: TileTrait_mouseover.
const TRAIT_MOUSEOVER: u16 = super::interface_frame::TRAIT_MOUSEOVER;
// IDA: the EffectSetting flag 004152C0, 00414FE0 and 00415030 skip.
const EFFECT_FLAG_NO_COST_PICK: u32 = 0x40_0000;
const FORM_TYPE_ARMOR: u8 = 0x14;
const FORM_TYPE_BOOK: u8 = 0x15;

/// IDA: 00B3B404, the sort byte 005B2430 reads: in .bss, so 0 at the start; only 005B3990 (a click) writes it.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MagicMenuStatics {
    pub sort_b3b404: i8,
}

// IDA: TileTrait userN, 4014 + N.
const fn user(n: u16) -> u16 {
    TRAIT_USER0 + n
}

/// IDA: 005B1F20 MagicMenu_ctor (0x5C, via 00587D20's 1022 case): Menu_Constructor, vtable A6C974; +0x28..+0x34 the four
/// field tiles null; the list heads +0x38 and +0x40 empty; +0x48 null; +0x50 the filter, 7; +0x54 0; +0x58 0.0. +0x4C
/// isn't set by the constructor; 005B2B70 clears it before reading. Completion: 100%.
#[derive(Clone, Debug, PartialEq)]
pub struct MagicMenu {
    /// +0x28..+0x34: fields 9, 10 (the list pane), 11 (its scroll bar), 12.
    pub tiles_28: [Option<TileId>; 4],
    /// +0x38: the magic items listed, in order (PointerList_AppendNonNull).
    pub items_38: Vec<Entity>,
    /// +0x48: a tile whose id goes to vt+0x14 before a refresh.
    pub tile_48: Option<TileId>,
    /// +0x4C: the row of the player's selected magic item.
    pub selected_row_4c: Option<TileId>,
    /// +0x50: the type filter 005B1A40 takes.
    pub filter_50: u32,
    pub count_54: u32,
    pub float_58: f32,
}

impl Default for MagicMenu {
    fn default() -> Self {
        Self {
            tiles_28: [None; 4],
            items_38: Vec::new(),
            tile_48: None,
            selected_row_4c: None,
            filter_50: 7,
            count_54: 0,
            float_58: 0.0,
        }
    }
}

impl MagicMenu {
    /// IDA: 005B13C0 MagicMenu_SetField: ids 9..12 to +0x28..+0x34. Completion: 100%.
    pub fn set_field(&mut self, id: i32, tile: TileId) {
        if (9..=12).contains(&id) {
            self.tiles_28[id as usize - 9] = Some(tile);
        }
    }

    /// IDA: 005B3810..005B3826: +0x2C, +0x28, +0x30 and +0x34 nonnull. Completion: 100%.
    #[must_use]
    pub fn has_required_tiles(&self) -> bool {
        self.tiles_28.iter().all(Option::is_some)
    }
}

// IDA: N/A the menu's MagicMenu part.
fn magic<'a>(cx: &'a InterfaceCx, menu: MenuId) -> &'a MagicMenu {
    match &cx.engine.menu(menu).class {
        MenuClass::Magic(magic) => magic,
        _ => unreachable!("a MagicMenu"),
    }
}

// IDA: N/A the menu's MagicMenu part, mutable.
fn magic_mut<'a>(cx: &'a mut InterfaceCx, menu: MenuId) -> &'a mut MagicMenu {
    match &mut cx.engine.menu_mut(menu).class {
        MenuClass::Magic(magic) => magic,
        _ => unreachable!("a MagicMenu"),
    }
}

// IDA: N/A a field the four-tile check has guaranteed.
fn field(cx: &InterfaceCx, menu: MenuId, id: usize) -> TileId {
    magic(cx, menu).tiles_28[id - 9].expect("the four tiles")
}

/// IDA: 0057A180 InterfaceManager_CreateMagicMenuIfReady: with a cursor (+0x1C) and +0x60, 005B3760. Completion: 100%.
/// # Errors
/// As `magic_menu_create`.
pub fn magic_menu_create_if_ready(cx: &mut InterfaceCx) -> Result<Option<TileId>, StreamError> {
    let ready = {
        let manager = cx.world.resource::<InterfaceManager>();
        manager.cursor_01c.is_some() && manager.player_scene_node_060.is_some()
    };
    if !ready {
        return Ok(None);
    }
    magic_menu_create(cx)
}

/// IDA: 005B3760 MagicMenu_CreateFromXML: an old 1022 menu deleted; the depth (00584980); Tile_ReadXML(magic_menu.xml)
/// under the menu root; its containing menu; 00584670 the cache; null answers null; a menu that isn't 1022 is deleted when
/// it has a tile; 00584880 registers the tile; without the four tiles the warning and null; a stacking type 6006 or 102
/// takes the depth; field 11's user5 0; the root's user1..user4 -2, user5 0, user6 " " (00A36430); 005B2B70; InterfaceManager
/// +0x0C other than -1 to 005B2060; field 11's user5 -99999 then 0; Menu_Enable(1); the root answers. Completion: 100%
/// but the tab arm.
/// TODO: [ ] 005B2060, a remembered tab.
/// # Errors
/// A stop in the XML, the list or a remembered tab.
pub fn magic_menu_create(cx: &mut InterfaceCx) -> Result<Option<TileId>, StreamError> {
    if let Some(old) = cx.engine.tile_menu_by_id(MENU_ID_MAGIC)
        && let Some(menu) = cx.engine.containing_menu(old)
    {
        menu_destroy(cx, menu)?;
    }
    let depth = menu_update_cursor_depth(cx)?;
    let root = cx.world.resource::<InterfaceManager>().menu_root_068;
    let created = tile_read_xml(cx, root, MAGIC_XML)?;
    let menu = created.and_then(|created| cx.engine.containing_menu(created));
    tile_xml_clear_cached_file(cx, MAGIC_XML, menu);
    let (Some(created), Some(menu)) = (created, menu) else {
        return Ok(None);
    };
    if cx.engine.menu(menu).id() != MENU_ID_MAGIC {
        if cx.engine.menu(menu).tile_04.is_some() {
            menu_destroy(cx, menu)?;
        }
        return Ok(None);
    }
    menu_register_tile(cx, menu, Some(created))?;
    if !magic(cx, menu).has_required_tiles() {
        cx.host
            .warning("Magic Menu Creation Failed... Are your menu and art resources up to date?");
        return Ok(None);
    }
    let stacking = tile_numeric_trait(cx, created, TRAIT_STACKING);
    if stacking == 6006.0 || stacking == 102.0 {
        tile_set_numeric_trait(cx, created, TRAIT_DEPTH, depth)?;
    }
    let scroll = field(cx, menu, 11);
    tile_set_numeric_trait(cx, scroll, user(5), 0.0)?;
    for n in 1..=4 {
        tile_set_numeric_trait(cx, created, user(n), -2.0)?;
    }
    tile_set_numeric_trait(cx, created, user(5), 0.0)?;
    tile_set_string_trait(cx, created, user(6), b" ")?;
    magic_menu_refresh(cx)?;
    if cx.world.resource::<InterfaceManager>().tab_ids_00a[2] as i8 != -1 {
        return Err(StreamError(
            "005B3930: a remembered magic tab (005B2060) is carried",
        ));
    }
    tile_set_numeric_trait(cx, scroll, user(5), -99_999.0)?;
    tile_set_numeric_trait(cx, scroll, user(5), 0.0)?;
    let frame_time = cx.world.resource::<InterfaceManager>().frame_seconds_host;
    menu_enable(cx, menu, true, frame_time)?;
    Ok(Some(created))
}

/// IDA: 005B2B70 MagicMenu_RefreshContentTiles over the 1022 menu: the scroll list 00B1435C emptied; with g_thePlayer:
/// +0x4C null; the base's +0x58 spell list sorted in place (005B27A0 by 005B2430); the inventory's enchanted scrolls into
/// 00B1435C (004D8950, 004D88F0, the book's +0x88 bit 0 and +0x64); +0x48's id to vt+0x14; field 10's children with id
/// 1001 or more get listindex -99; the root's user1..user4 -2; +0x38 cleared; 005B2B30 the active effects into +0x40. Then
/// each spell with a highest cost effect and a type other than 1 and 4: appended to +0x38, its range bits (00414FE0 1,
/// 00415030 2, else 4), its type bits (2: 1, 3: 2, 0 and 5: 4), the icon `Icons\` and the highest cost effect's setting
/// +0x48, the name, the cost for the player (vt+0 of +0x24, 0041D320) through __ftol2, whether it's 0065D4A0's item; an
/// old row of the name (listindex -99, _stricmp of user1) is updated and moved, else 005B2A10 makes one with the id
/// 1001 + i; 005B1430(row, -1, -1, cost); user13 the count, user6 the icon, user7 the type bits, user9 the range bits,
/// user10 1 + selected, user11 the count less 1, listindex i; the selected row to +0x4C. Field 10's user0 the rows less
/// one; the -99 rows deleted; 005B1A40(+0x50); the root's user6 "%s: %0.0f%%" of sSpellEffectiveness (00B39518) and the
/// caster's vt+0x2C(0, 0.0) times 100; 0057D730(0). Completion: 100% of the spell arm.
/// TODO: [ ] The scroll arm (005B31D9) and the active effect arm (005B32E9); a matched old row (005B33C9).
/// # Errors
/// A book in the player's inventory, an active effect, an old row of the name, or a stop below.
pub fn magic_menu_refresh(cx: &mut InterfaceCx) -> Result<(), StreamError> {
    let Some(root) = cx.engine.tile_menu_by_id(MENU_ID_MAGIC) else {
        return Ok(());
    };
    let menu = cx.engine.containing_menu(root).expect("a menu tile");
    let Some(player) = the_player(cx.world) else {
        return Ok(());
    };
    magic_mut(cx, menu).selected_row_4c = None;
    let base = cx
        .world
        .get::<TesObjectRefr>(player)
        .and_then(|refr| refr.base_form_1c)
        .ok_or(StreamError("005B2C5B: a player with no base form"))?;
    sort_spells(cx.world, base, player)?;
    // 005B2C76..005B2D95: the inventory's scrolls. Only a book can be one; no book, no entry.
    if inventory_forms(cx.world, player)
        .into_iter()
        .any(|form| form_type(cx.world, form) == FORM_TYPE_BOOK)
    {
        return Err(StreamError(
            "005B2CBD: a book in the player's inventory (the scroll arm) is carried",
        ));
    }
    if magic(cx, menu).tile_48.is_some() {
        return Err(StreamError("005B2DC0: +0x48's vt+0x14 is carried"));
    }
    let pane = field(cx, menu, 10);
    for child in cx.engine.tile(pane).children_30.clone() {
        if tile_numeric_trait(cx, child, TRAIT_ID) >= 1001.0 {
            tile_set_numeric_trait(cx, child, TRAIT_LIST_INDEX, -99.0)?;
        }
    }
    for n in 1..=4 {
        tile_set_numeric_trait(cx, root, user(n), -2.0)?;
    }
    magic_mut(cx, menu).items_38.clear();
    // 005B2B30: the player's MagicTarget vt+8 (0066B0B0, +0x17C of +0x68: +0x1E4) into +0x40.
    let effects = cx
        .world
        .get::<PlayerCharacter>(player)
        .and_then(|data| data.active_effects_1e4);
    if effects.is_some_and(|list| list != [0; 2]) {
        return Err(StreamError(
            "005B2B4B: an active effect on the player is carried",
        ));
    }
    let spells: Vec<Entity> = cx
        .world
        .get::<TesSpellList>(base)
        .map(|list| {
            list.spells_04
                .iter()
                .filter_map(|slot| match slot {
                    FormRef::Resolved(spell) => *spell,
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
    let selected = cx
        .world
        .get::<PlayerCharacter>(player)
        .and_then(|data| data.active_magic_item_624);
    let mut index = 0_i32;
    let mut listed = 0_i32;
    for spell in spells {
        let Some(highest) = highest_cost_effect(cx.world, spell)? else {
            continue;
        };
        let kind = magic_type(cx.world, spell);
        if kind == 1 || kind == 4 {
            continue;
        }
        magic_mut(cx, menu).items_38.push(spell);
        listed += 1;
        let hostile = has_range(cx.world, spell, 2);
        let range_bits = if hostile {
            1
        } else if has_range(cx.world, spell, 1) {
            2
        } else {
            4
        };
        let type_bits = match kind {
            2 => 1,
            3 => 2,
            0 | 5 => 4,
            _ => 0,
        };
        let icon = effect_icon(cx.world, spell, highest);
        let name = full_name(cx.world, spell);
        let cost = x87::ftol(magic_item_cost_for_caster(cx.world, spell, Some(player))?);
        let is_selected = selected == Some(spell);
        for row in cx.engine.tile(pane).children_30.clone().into_iter().rev() {
            if tile_numeric_trait(cx, row, TRAIT_LIST_INDEX) == -99.0
                && let Some(text) = tile_string_trait(cx, row, user(1))
                && recomp_core::crt::stricmp(&name, &text) == 0
            {
                return Err(StreamError(
                    "005B313B: an old row of the same name is carried",
                ));
            }
        }
        let name = if name.is_empty() {
            cx.world
                .get_resource::<super::game_settings::GameSettingCollection>()
                .and_then(|settings| settings.text_at(0x00B3_8D30))
                .unwrap_or_default()
        } else {
            name
        };
        let Some(row) = create_row(cx, menu, &name, 1001 + index)? else {
            return Err(StreamError("005B2A10: no row template"));
        };
        set_row_numbers(cx, row, -1, -1, cost)?;
        tile_set_numeric_trait(cx, row, user(13), listed as f32)?;
        tile_set_string_trait(cx, row, user(6), &icon)?;
        tile_set_numeric_trait(cx, row, user(7), type_bits as f32)?;
        tile_set_numeric_trait(cx, row, user(9), range_bits as f32)?;
        tile_set_numeric_trait(cx, row, user(10), f32::from(u8::from(is_selected) + 1))?;
        tile_set_numeric_trait(cx, row, user(11), (listed - 1) as f32)?;
        tile_set_numeric_trait(cx, row, TRAIT_LIST_INDEX, index as f32)?;
        if is_selected {
            magic_mut(cx, menu).selected_row_4c = Some(row);
        }
        index += 1;
    }
    tile_set_numeric_trait(cx, pane, user(0), (index - 1) as f32)?;
    for child in cx.engine.tile(pane).children_30.clone() {
        if tile_numeric_trait(cx, child, TRAIT_LIST_INDEX) == -99.0 {
            tile_destroy(cx, child)?;
        }
    }
    let filter = magic(cx, menu).filter_50;
    magic_menu_apply_filter(cx, menu, filter)?;
    let effectiveness = player_spell_effectiveness(cx.world, player)?;
    let label = cx
        .world
        .get_resource::<super::game_settings::GameSettingCollection>()
        .and_then(|settings| settings.text_at(0x00B3_9518))
        .unwrap_or_default();
    let mut text = label;
    // 005B36E1: `fmul` 100.0 at 24 bits, then a double to BSStringT_Format. Rust's {:.0} rounds a tie to even.
    text.extend_from_slice(format!(": {:.0}%", f64::from(effectiveness * 100.0)).as_bytes());
    tile_set_string_trait(cx, root, user(6), &text)?;
    // 0057D730(0): +0x98 and +0x9C cleared.
    let mut manager = cx.world.resource_mut::<InterfaceManager>();
    manager.active_tile_098 = None;
    manager.active_menu_09c = None;
    Ok(())
}

// IDA: 005B2A10 MagicMenu_CreateRow(name, id): `item_template` under field 10 (00585410; the menu's one template, since a
// miss builds the last), user1 the name, the tile's name the first 255 bytes with ' ' as '_', then the id.
fn create_row(
    cx: &mut InterfaceCx,
    menu: MenuId,
    name: &[u8],
    id: i32,
) -> Result<Option<TileId>, StreamError> {
    let pane = field(cx, menu, 10);
    let Some(row) = tile_build_template(cx, menu, pane, b"item_template")? else {
        return Ok(None);
    };
    tile_set_string_trait(cx, row, user(1), name)?;
    let tile_name: Vec<u8> = name
        .iter()
        .take(255)
        .map(|&byte| if byte == b' ' { b'_' } else { byte })
        .collect();
    cx.engine.tile_mut(row).name_08 = tile_name;
    tile_set_numeric_trait(cx, row, TRAIT_ID, id as f32)?;
    Ok(Some(row))
}

// IDA: 005B1430(row, a, b, c): user3, user4 and user5 each "%i", or " " (00A36430) below 0.
fn set_row_numbers(
    cx: &mut InterfaceCx,
    row: TileId,
    a: i32,
    b: i32,
    c: i32,
) -> Result<(), StreamError> {
    for (n, value) in [(3, a), (4, b), (5, c)] {
        if value < 0 {
            tile_set_string_trait(cx, row, user(n), b" ")?;
        } else {
            tile_set_string_trait(cx, row, user(n), value.to_string().as_bytes())?;
        }
    }
    Ok(())
}

/// IDA: 005B1A40 MagicMenu_ApplyFilter(mask): the root's user1..user4 -2; field 10's children from the tail: one whose
/// user9 (__ftol2) has a bit of the mask and whose id is 1001 or more is shown (user8 2, listindex the next index), any
/// other hidden (user8 1). Without the mask's 8, a row whose user7 differs from the last shown one first gives its first
/// set bit's heading (user1..user4 by bits 1, 2, 4, 8) the current index and takes one. Then field 10's user0 and the
/// root's user5 the index less one (0 below that); an unset heading takes the next one's (user4 999999); with
/// InterfaceManager +0xB9 clear, 0057F9F0(null, mouseover) and 0057FD60. Completion: 100%.
/// # Errors
/// A stop in the trait writes or the pointer dispatch.
pub fn magic_menu_apply_filter(
    cx: &mut InterfaceCx,
    menu: MenuId,
    mask: u32,
) -> Result<(), StreamError> {
    let root = cx.engine.menu(menu).tile_04.expect("a registered menu");
    let pane = field(cx, menu, 10);
    let mut next = -1_i32;
    let mut last_kind = -1_i32;
    let mut index = -1_i32;
    if mask & 8 != 0 {
        next = 0;
        index = 0;
    }
    for n in 1..=4 {
        tile_set_numeric_trait(cx, root, user(n), -2.0)?;
    }
    for row in cx.engine.tile(pane).children_30.clone().into_iter().rev() {
        let bits = x87::ftol(tile_numeric_trait(cx, row, user(9))) as u32;
        if bits & mask == 0 || tile_numeric_trait(cx, row, TRAIT_ID) < 1001.0 {
            tile_set_numeric_trait(cx, row, user(8), 1.0)?;
            continue;
        }
        let kind = x87::ftol(tile_numeric_trait(cx, row, user(7)));
        if mask & 8 == 0 && kind != last_kind {
            let heading = [(1, 1), (2, 2), (4, 3), (8, 4)]
                .into_iter()
                .find(|&(bit, _)| kind & bit != 0)
                .map(|(_, n)| n);
            if let Some(n) = heading {
                tile_set_numeric_trait(cx, root, user(n), index as f32)?;
                next += 1;
                index = next;
            }
        }
        last_kind = kind;
        tile_set_numeric_trait(cx, row, user(8), 2.0)?;
        tile_set_numeric_trait(cx, row, TRAIT_LIST_INDEX, index as f32)?;
        next += 1;
        index = next;
    }
    let last = (next - 1).max(0) as f32;
    tile_set_numeric_trait(cx, pane, user(0), last)?;
    tile_set_numeric_trait(cx, root, user(5), last)?;
    if tile_numeric_trait(cx, root, user(4)) == -2.0 {
        tile_set_numeric_trait(cx, root, user(4), 999_999.0)?;
    }
    for n in [3, 2, 1] {
        if tile_numeric_trait(cx, root, user(n)) == -2.0 {
            let after = tile_numeric_trait(cx, root, user(n + 1));
            tile_set_numeric_trait(cx, root, user(n), after)?;
        }
    }
    if cx.world.resource::<InterfaceManager>().cursor_moved_0b9 == 0 {
        interface_dispatch_pointer_event(cx, None, TRAIT_MOUSEOVER)?;
        super::interface_frame::interface_select_default_tile(cx)?;
    }
    Ok(())
}

// IDA: the magic item's vt+0x18 (a folded `mov eax,[ecx+1Ch]`): the type at +0x34.
fn magic_type(world: &World, spell: Entity) -> u32 {
    world
        .get::<MagicFormData>(spell)
        .map_or(0, |data| data.type_34)
}

// IDA: 00414FE0 (range 2) and 00415030 (range 1): an effect of the range whose setting lacks 0x400000; an empty list is
// false.
fn has_range(world: &World, spell: Entity, range: u32) -> bool {
    let Some(list) = world.get::<EffectItemList>(spell) else {
        return false;
    };
    list.items_04.iter().any(|item| {
        item.data_00[4] == range
            && item
                .setting_1c
                .and_then(|setting| world.get::<EffectSetting>(setting))
                .is_some_and(|setting| setting.data_58.flags_58 & EFFECT_FLAG_NO_COST_PICK == 0)
    })
}

// IDA: 005B306F..005B3091: "%s\\%s" of "Icons" and the effect's setting +0x48 (TESIcon's path), "" when null.
fn effect_icon(world: &World, spell: Entity, index: usize) -> Vec<u8> {
    let path = world
        .get::<EffectItemList>(spell)
        .and_then(|list| list.items_04[index].setting_1c)
        .and_then(|setting| world.get::<super::data_handler::TesTexture>(setting))
        .and_then(|icon| icon.path_04.data_00.clone())
        .unwrap_or_default();
    let end = path
        .iter()
        .position(|&byte| byte == 0)
        .unwrap_or(path.len());
    let mut text = b"Icons\\".to_vec();
    text.extend_from_slice(&path[..end]);
    text
}

// IDA: the item's TESFullName data at +0x1C, null as "".
fn full_name(world: &World, form: Entity) -> Vec<u8> {
    world
        .get::<TesFullName>(form)
        .and_then(|name| name.name_04.data_00.clone())
        .map(|name| {
            let end = name
                .iter()
                .position(|&byte| byte == 0)
                .unwrap_or(name.len());
            name[..end].to_vec()
        })
        .unwrap_or_default()
}

// IDA: N/A the forms of the player's inventory: the base container's entries and the ExtraContainerChanges entries.
pub(crate) fn inventory_forms(world: &World, player: Entity) -> Vec<Entity> {
    let mut forms = Vec::new();
    if let Some(base) = world
        .get::<TesObjectRefr>(player)
        .and_then(|refr| refr.base_form_1c)
        && let Some(container) = world.get::<TesContainer>(base)
    {
        forms.extend(
            container
                .entries_08
                .iter()
                .filter_map(|entry| match entry.item {
                    FormRef::Resolved(form) => form,
                    _ => None,
                }),
        );
    }
    if let Some(refr) = world.get::<TesObjectRefr>(player)
        && let Some(entry) = refr
            .extra_data_44
            .get(super::base_extra_list::EXTRA_CONTAINER_CHANGES)
        && let super::base_extra_list::ExtraData::ContainerChanges(changes) = &entry.data
    {
        forms.extend(changes.entries_00.iter().map(|entry| entry.form_08));
    }
    forms
}

// IDA: 005B27A0 MagicMenu_SortList(compare) over the base's +0x58 list, in place: the items into an array, 00506F00's
// shell sort (h = 3h + 1 from 1 while h <= (n - 1) / 9, then down by thirds; each pass an insertion that stops at a compare
// of 0 or more), the nodes rebuilt in that order.
fn sort_spells(world: &mut World, base: Entity, player: Entity) -> Result<(), StreamError> {
    let Some(list) = world.get::<TesSpellList>(base) else {
        return Ok(());
    };
    let mut items = list.spells_04.clone();
    let count = items.len() as i32;
    let mut gap = 1_i32;
    if (count - 1) / 9 >= 1 {
        while gap <= (count - 1) / 9 {
            gap = 3 * gap + 1;
        }
    }
    while gap > 0 {
        for start in gap..count {
            let held = items[start as usize];
            let mut at = start;
            while at >= gap {
                if compare_spells(world, held, items[(at - gap) as usize], player)? >= 0 {
                    break;
                }
                items[at as usize] = items[(at - gap) as usize];
                at -= gap;
            }
            items[at as usize] = held;
        }
        gap /= 3;
    }
    world
        .get_mut::<TesSpellList>(base)
        .expect("checked")
        .spells_04 = items;
    Ok(())
}

// IDA: 005B2430 MagicMenu_CompareMagicItems(a, b) by the byte at 00B3B404 (zero at the start: .bss): its sign the order
// (1 or -1), its low seven bits the key. Key 0: "%c%.50s" of the type's letter (ascending: 2 'A', 3 'B', 0 and 5 'C', 6
// 'D', else 'E'; descending: 2 'E', 3 'D', 0 and 5 'C', 6 'B', else 'A') and the name, _mbsicmp, times the order. Any
// other key: the ascending letters as signed bytes, then the costs for the player (vt+0 of +0x24, __ftol2).
fn compare_spells(
    world: &mut World,
    a: FormRef,
    b: FormRef,
    player: Entity,
) -> Result<i32, StreamError> {
    let (FormRef::Resolved(Some(a)), FormRef::Resolved(Some(b))) = (a, b) else {
        return Err(StreamError("005B2430: a spell slot that isn't a form"));
    };
    let mode = world
        .get_resource::<MagicMenuStatics>()
        .map_or(0, |statics| statics.sort_b3b404);
    let order = if mode >= 0 { 1 } else { -1 };
    let letter = |kind: u32, descending: bool| -> u8 {
        if descending {
            match kind {
                2 => b'E',
                3 => b'D',
                0 | 5 => b'C',
                6 => b'B',
                _ => b'A',
            }
        } else {
            match kind {
                2 => b'A',
                3 => b'B',
                0 | 5 => b'C',
                6 => b'D',
                _ => b'E',
            }
        }
    };
    if mode & 0x7F == 0 {
        let key = |spell: Entity| {
            let mut text = vec![letter(magic_type(world, spell), mode < 0)];
            text.extend(full_name(world, spell).into_iter().take(50));
            text
        };
        let (left, right) = (key(a), key(b));
        return Ok(order * recomp_core::crt::stricmp(&left, &right).signum());
    }
    let (left, right) = (
        letter(magic_type(world, a), false) as i8,
        letter(magic_type(world, b), false) as i8,
    );
    let result = if left < right {
        -order
    } else if left > right {
        order
    } else {
        let first = x87::ftol(magic_item_cost_for_caster(world, a, Some(player))?);
        let second = x87::ftol(magic_item_cost_for_caster(world, b, Some(player))?);
        first.cmp(&second) as i32
    };
    Ok(order * result)
}

/// IDA: 0041D320, a spell's EffectItemList vt+0 (MagicItem_GetMagickaCost) with a caster: a power (2) or lesser power (3)
/// drops the caster; the +0x40 bit 0 (a set cost) takes +0x38 through 00548C00; else 004151C0: the effects past the
/// setting flag 0x400000 summed through 00413890, an empty list 0.0, the sum floored by 00404F70 when not negative, else
/// 0.0. Completion: 100% of the computed cost.
/// TODO: [ ] The set-cost arm (+0x40 bit 0).
/// # Errors
/// A set cost, a null effect setting, or an actor value that stops.
pub fn magic_item_cost_for_caster(
    world: &mut World,
    spell: Entity,
    caster: Option<Entity>,
) -> Result<f32, StreamError> {
    let data = world
        .get::<MagicFormData>(spell)
        .copied()
        .unwrap_or_default();
    let caster = if matches!(data.type_34, 2 | 3) {
        None
    } else {
        caster
    };
    if data.flags_40 & 1 != 0 {
        return Err(StreamError("0041D34C: a magic item's set cost is carried"));
    }
    let count = world
        .get::<EffectItemList>(spell)
        .map_or(0, |list| list.items_04.len());
    if count == 0 {
        return Ok(0.0);
    }
    let mut sum = 0.0_f32;
    for index in 0..count {
        let setting = world
            .get::<EffectItemList>(spell)
            .expect("a magic item")
            .items_04[index]
            .setting_1c
            .ok_or(StreamError("004151EC: a null effect setting faults"))?;
        let flags = world
            .get::<EffectSetting>(setting)
            .expect("a setting")
            .data_58
            .flags_58;
        if flags & EFFECT_FLAG_NO_COST_PICK == 0 {
            sum += effect_cost_for_caster(world, spell, index, caster)?;
        }
    }
    // 00415211..0041521D: `fcom` of 0.0 with the sum; only 0.0 above it gives 0.0, so a NaN is floored.
    Ok(if sum < 0.0 { 0.0 } else { floor_00404f70(sum) })
}

// IDA: 00404F70: __ftol2, `fild`, the value less that below 0.0 steps the whole number down by 1.0.
fn floor_00404f70(value: f32) -> f32 {
    let whole = x87::ftol(value) as f32;
    if value - whole < 0.0 {
        whole - 1.0
    } else {
        whole
    }
}

// IDA: 00413890 EffectItem_GetMagickaCostForCaster with an actor: the cached cost (00413810); the effect's school (the
// script info's +4, else the setting's +0x64) as an actor value (0041B770: 0..5 to 20..25); Luck (7) and that skill
// through vt+0x284; 00548C00(cost, skill, luck); then floored as 00404F70 does and at least 1.0.
fn effect_cost_for_caster(
    world: &mut World,
    spell: Entity,
    index: usize,
    caster: Option<Entity>,
) -> Result<f32, StreamError> {
    let mut cost = effect_cost(world, spell, index)?;
    if let Some(caster) = caster {
        let item = &world
            .get::<EffectItemList>(spell)
            .expect("a magic item")
            .items_04[index];
        let school = match &item.script_18 {
            Some(script) => script.school_04,
            None => {
                let setting = item
                    .setting_1c
                    .ok_or(StreamError("004138B5: a null effect setting faults"))?;
                world
                    .get::<EffectSetting>(setting)
                    .expect("a setting")
                    .data_58
                    .school_64
            }
        };
        if school > 5 {
            return Err(StreamError(
                "004138CB: a school with no actor value is carried",
            ));
        }
        let luck = player_get_actor_value(world, caster, 7)?;
        let skill = player_get_actor_value(world, caster, 20 + school)?;
        cost = caster_skill_cost(world, cost, skill, luck);
    }
    let floor = floor_00404f70(cost);
    Ok(if floor <= 1.0 { 1.0 } else { floor })
}

// IDA: 00548C00(cost, skill, luck): (1 - 00547B90(skill, luck) / 100.0) * fMagicCasterSkillCostMult (00B37DF0) +
// fMagicCasterSkillCostBase (00B37DE8), times the cost; each stored step a float.
fn caster_skill_cost(world: &World, cost: f32, skill: i32, luck: i32) -> f32 {
    let adjusted = luck_adjusted_skill(world, skill, luck) / 100.0;
    let factor =
        (1.0 - adjusted) * setting_float(world, 0x00B3_7DF0) + setting_float(world, 0x00B3_7DE8);
    factor * cost
}

// IDA: 00547B90 CalculateLuckAdjustedSkill(skill, luck): luck * fActorLuckSkillMult (00B37390) + iActorLuckSkillBase
// (00B37388, `fiadd`), each stored a float, plus the skill; 0 at or below 0.0, 100 at or above 100.0.
pub(crate) fn luck_adjusted_skill(world: &World, skill: i32, luck: i32) -> f32 {
    let scaled = luck as f32 * setting_float(world, 0x00B3_7390);
    let luck = scaled + setting_int(world, 0x00B3_7388) as f32;
    let value = skill as f32 + luck;
    // 00547BB4..00547BD0: below 100.0, then at or below 0.0; a NaN fails both and gives 100.
    if value <= 0.0 {
        0.0
    } else if value >= 100.0 || value.is_nan() {
        100.0
    } else {
        value
    }
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

/// IDA: 0065DAF0 PlayerCharacter's MagicCaster vt+0x2C (0, 0.0), the spell effectiveness: with god mode (00B3BB06, set
/// only by the console's 0065D810) 005E5630; the fatigue factor 1.0, or with a nonzero calculated fatigue other than the
/// current (vt+0x284(10)) 00548B30 of current / calculated (fB37DC8 * x + fB37DC0); then 00548DC0 of Light Armor (27),
/// its coverage (005E5C80(0)), Heavy Armor (18), its coverage (005E5C80(1)): 1.0 when the coverages sum to 0 or less.
/// Completion: 100% of no god mode and no armor worn.
/// TODO: [ ] 00548DC0's armor arm.
/// # Errors
/// Armor in the player's inventory, or an actor value that stops.
pub fn player_spell_effectiveness(world: &World, player: Entity) -> Result<f32, StreamError> {
    if inventory_forms(world, player)
        .into_iter()
        .any(|form| form_type(world, form) == FORM_TYPE_ARMOR)
    {
        return Err(StreamError(
            "005E5C80: armor in the player's inventory (00548DC0's armor arm) is carried",
        ));
    }
    let current = player_get_actor_value(world, player, 10)?;
    let calculated = floor_calculated_actor_value(world, player, 10)?;
    let mut fatigue = 1.0_f32;
    if calculated != 0 && calculated != current {
        let ratio = current as f32 / calculated as f32;
        fatigue = setting_float(world, 0x00B3_7DC8) * ratio + setting_float(world, 0x00B3_7DC0);
    }
    Ok(1.0 * fatigue)
}
