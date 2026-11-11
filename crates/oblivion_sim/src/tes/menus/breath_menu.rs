//! BreathMenu (1045): 00596420 BreathMenu_ctor's fields, 00596440 SetField while `Data\Menus\breath_meter_menu.xml` is
//! built, 005965C0, which builds it full, and 00596550, which sets its percent. Its caller (00603CA0, the actor's
//! underwater update) isn't ported.

use oblivion_core::error::GameError;
use oblivion_core::x87;

use crate::tes::interface_frame::menu_destroy;
use crate::tes::interface_manager::{InterfaceManager, menu_register_tile};
use crate::tes::tile::{InterfaceCx, MenuClass, MenuId, TileId};
use crate::tes::tile_values::{TRAIT_USER1, tile_set_numeric_trait, tile_update_scene_graph};
use crate::tes::tile_xml::tile_read_xml;

/// IDA: MenuID_Breath.
pub const MENU_ID_BREATH: u32 = 1045;
const BREATH_XML: &str = "Data\\Menus\\breath_meter_menu.xml";

/// IDA: 00596420 BreathMenu_ctor (via 00587D20's 1045 case, FormHeap 0x34): Menu_Constructor, +0x28, +0x2C and +0x30
/// zero, vtable. Completion: 100%.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BreathMenu {
    /// +0x28: field 1, the menu's own tile.
    pub root_28: Option<TileId>,
    /// +0x2C: field 2, `breath_progress_ribbon`.
    pub ribbon_2c: Option<TileId>,
    /// +0x30: the percent last shown (00596550).
    pub percent_30: i32,
}

impl BreathMenu {
    /// IDA: 00596440 BreathMenu_SetField: id 1 to +0x28, id 2 to +0x2C. Completion: 100%.
    pub fn set_field(&mut self, id: i32, tile: TileId) {
        match id {
            1 => self.root_28 = Some(tile),
            2 => self.ribbon_2c = Some(tile),
            _ => {}
        }
    }
}

// IDA: N/A the menu's BreathMenu part.
fn breath<'a>(cx: &'a InterfaceCx, menu: MenuId) -> &'a BreathMenu {
    match &cx.engine.menu(menu).class {
        MenuClass::Breath(breath) => breath,
        _ => unreachable!("a BreathMenu"),
    }
}

/// IDA: 00596550 BreathMenu_SetPercent(fraction): with a 1045 menu, fraction `fmul` 100.0 (a double, at the main
/// thread's 24 bits) through __ftol2; when it differs from +0x30, +0x30 takes it and field 2's user1 its `fild`. Completion:
/// 100%.
/// # Errors
/// A stop in the trait write; a 1045 tile whose menu isn't a BreathMenu (the original reads the cast's null).
pub fn breath_menu_set_percent(cx: &mut InterfaceCx, fraction: f32) -> Result<(), GameError> {
    let Some(tile) = cx.engine.tile_menu_by_id(MENU_ID_BREATH) else {
        return Ok(());
    };
    let menu = cx
        .engine
        .containing_menu(tile)
        .filter(|&menu| matches!(cx.engine.menu(menu).class, MenuClass::Breath(_)))
        .ok_or(GameError::unclassified(0x00596592,
            "00596592: the 1045 tile's menu isn't a BreathMenu, and the original reads the null cast",
        ))?;
    let percent = x87::ftol(fraction * 100.0);
    let MenuClass::Breath(state) = &mut cx.engine.menu_mut(menu).class else {
        unreachable!("a BreathMenu")
    };
    if state.percent_30 == percent {
        return Ok(());
    }
    state.percent_30 = percent;
    let ribbon = state.ribbon_2c.ok_or(GameError::unclassified(
        0x005965A0,
        "005965A0: BreathMenu +0x2C is null, and the original reads it",
    ))?;
    tile_set_numeric_trait(cx, ribbon, TRAIT_USER1, percent as f32)
}

/// IDA: 005965C0 BreathMenu_Create: an existing 1045 menu deleted (TileMenu vt+0, ported as `map_menu_create` does it);
/// Tile_ReadXML(breath_meter_menu.xml) under the root; its containing menu, else null; a menu that isn't 1045 is deleted
/// when it has a tile, and null; 00584880 registers the tile. With field 2: 00596550(1.0), Tile_UpdateSceneGraph(0), and
/// the root answers; without it the warning, the menu deleted (vt+0) and null. No depth is taken and the menu isn't
/// enabled. Completion: 100%.
/// # Errors
/// A stop in the XML, the menu calls or the scene update.
pub fn breath_menu_create(cx: &mut InterfaceCx) -> Result<Option<TileId>, GameError> {
    if let Some(old) = cx.engine.tile_menu_by_id(MENU_ID_BREATH)
        && let Some(menu) = cx.engine.containing_menu(old)
    {
        menu_destroy(cx, menu)?;
    }
    let root = cx.world.resource::<InterfaceManager>().menu_root_068;
    let created = tile_read_xml(cx, root, BREATH_XML)?;
    let menu = created.and_then(|created| cx.engine.containing_menu(created));
    let (Some(created), Some(menu)) = (created, menu) else {
        return Ok(None);
    };
    if cx.engine.menu(menu).id() != MENU_ID_BREATH {
        if cx.engine.menu(menu).tile_04.is_some() {
            menu_destroy(cx, menu)?;
        }
        return Ok(None);
    }
    menu_register_tile(cx, menu, Some(created))?;
    if breath(cx, menu).ribbon_2c.is_none() {
        cx.host.warning(
            "Breath Meter Menu Creation Failed... Are your menu and art resources up to date?",
        );
        menu_destroy(cx, menu)?;
        return Ok(None);
    }
    breath_menu_set_percent(cx, 1.0)?;
    tile_update_scene_graph(cx, created, false)?;
    Ok(Some(created))
}
