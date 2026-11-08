//! Phase 15: the WinMain block that selects the initial area, 0040EBB9..0040EE79. 0040EE7A's "Placing player..." starts Phase 16. The arms are a trait, `InitialAreaArms`, whose real form is `RealInitialAreaArms` and whose recording form (a test's) stands in for any of them, so that each INI path stays testable. The callees live in their layers: `tes::data_handler_world`, `tes::tes_grid`, `tes::tes`.
//!
//! x87: the Rect stores run at 24 bits: `fild`, `fadd` qword 2048.0, `fstp` float: one rounding of the exact sum.
//! Evidence: objdump 0040EBB9..0040EE7F; evidence/phase15-scope-2026-10-10; P15-RUN-001..005.

use bevy_ecs::prelude::*;
use recomp_core::crt::stricmp;
use recomp_core::reader::StreamError;
use recomp_game::tes::ai_package::cell_world;
use recomp_game::tes::data_handler::DataHandlerLists;
use recomp_game::tes::data_handler_world::{
    find_cell_by_editor_id, find_master_exterior_cell, find_worldspace_grid_in_plugins,
    lookup_or_create_exterior_cell, propagate_runtime_field_to_masters,
};
use recomp_game::tes::form_arms::editor_id_of;
use recomp_game::tes::forms_world::{cell_grid_x, cell_grid_y, cell_is_interior};
use recomp_game::tes::settings::SettingCollection;
use recomp_game::tes::tes_grid::{
    TesGridServices, enter_interior_cell, exit_interior_cell, initialize_loaded_cells,
    update_exterior_grid_from_position,
};

/// IDA: the block's four settings, each a `char*` read where the block reads it.
const STARTING_CELL: u32 = 0x00B0_2CE0;
const STARTING_CELL_X: u32 = 0x00B0_2CF8;
const STARTING_CELL_Y: u32 = 0x00B0_2D00;
const STARTING_WORLD: u32 = 0x00B0_2CF0;

/// IDA: N/A what the block calls, by address. `RealInitialAreaArms` runs them; a test's recorder can answer any of them.
pub trait InitialAreaArms {
    /// IDA: 004478B0(name, &x, &y).
    /// # Errors
    /// A stop in the arm.
    fn find_worldspace_grid_in_plugins(
        &mut self,
        world: &mut World,
        name: &[u8],
    ) -> Result<(Option<Entity>, i32, i32), StreamError>;
    /// IDA: 004F1630(world, x, y).
    /// # Errors
    /// A stop in the arm.
    fn find_master_exterior_cell(
        &mut self,
        world: &mut World,
        space: Entity,
        x: i32,
        y: i32,
    ) -> Result<Option<Entity>, StreamError>;
    /// IDA: 00447740(x, y, world, create).
    /// # Errors
    /// A stop in the arm.
    fn lookup_or_create_exterior_cell(
        &mut self,
        world: &mut World,
        x: i32,
        y: i32,
        space: Option<Entity>,
        create: bool,
    ) -> Result<Option<Entity>, StreamError>;
    /// IDA: 004431F0(world).
    /// # Errors
    /// A stop in the arm.
    fn set_current_worldspace(
        &mut self,
        world: &mut World,
        space: Option<Entity>,
    ) -> Result<(), StreamError>;
    /// IDA: 004455E0(cell, &Rect).
    /// # Errors
    /// A stop in the arm.
    fn enter_interior_cell(
        &mut self,
        world: &mut World,
        cell: Entity,
        rect: &mut [f32; 3],
    ) -> Result<(), StreamError>;
    /// IDA: 00445A10(&Rect).
    /// # Errors
    /// A stop in the arm.
    fn exit_interior_cell(
        &mut self,
        world: &mut World,
        rect: &mut [f32; 3],
    ) -> Result<(), StreamError>;
    /// IDA: 00444EC0(&Rect, reload).
    /// # Errors
    /// A stop in the arm.
    fn update_exterior_grid_from_position(
        &mut self,
        world: &mut World,
        rect: &[f32; 3],
        reload: bool,
    ) -> Result<(), StreamError>;
    /// IDA: 00482310(g_tes +0x08).
    /// # Errors
    /// A stop in the arm.
    fn initialize_loaded_cells(&mut self, world: &mut World) -> Result<(), StreamError>;
    /// IDA: 004A7A60 DebugMessage_Warning (D4: the host log).
    fn warning(&mut self, world: &World, text: String);
}

/// IDA: N/A the real arms: `tes_grid`'s, over one service block. 15a's own calls take the load's seams from it.
pub struct RealInitialAreaArms<'a, 'b, 'c> {
    pub grid: &'a mut TesGridServices<'b, 'c>,
}

impl InitialAreaArms for RealInitialAreaArms<'_, '_, '_> {
    /// IDA: N/A `RealInitialAreaArms`'s side of `InitialAreaArms`.
    fn find_worldspace_grid_in_plugins(
        &mut self,
        world: &mut World,
        name: &[u8],
    ) -> Result<(Option<Entity>, i32, i32), StreamError> {
        find_worldspace_grid_in_plugins(world, name, self.grid.load)
    }

    /// IDA: N/A `RealInitialAreaArms`'s side of `InitialAreaArms`.
    fn find_master_exterior_cell(
        &mut self,
        world: &mut World,
        space: Entity,
        x: i32,
        y: i32,
    ) -> Result<Option<Entity>, StreamError> {
        find_master_exterior_cell(world, space, x, y, self.grid.load)
    }

    /// IDA: N/A `RealInitialAreaArms`'s side of `InitialAreaArms`.
    fn lookup_or_create_exterior_cell(
        &mut self,
        world: &mut World,
        x: i32,
        y: i32,
        space: Option<Entity>,
        create: bool,
    ) -> Result<Option<Entity>, StreamError> {
        lookup_or_create_exterior_cell(world, x, y, space, create, self.grid.load)
    }

    /// IDA: N/A `RealInitialAreaArms`'s side of `InitialAreaArms`.
    fn set_current_worldspace(
        &mut self,
        world: &mut World,
        space: Option<Entity>,
    ) -> Result<(), StreamError> {
        self.grid.set_world(world, space)
    }

    /// IDA: N/A `RealInitialAreaArms`'s side of `InitialAreaArms`.
    fn enter_interior_cell(
        &mut self,
        world: &mut World,
        cell: Entity,
        rect: &mut [f32; 3],
    ) -> Result<(), StreamError> {
        enter_interior_cell(world, Some(cell), rect, self.grid)
    }

    /// IDA: N/A `RealInitialAreaArms`'s side of `InitialAreaArms`.
    fn exit_interior_cell(
        &mut self,
        world: &mut World,
        rect: &mut [f32; 3],
    ) -> Result<(), StreamError> {
        exit_interior_cell(world, rect, self.grid)
    }

    /// IDA: N/A `RealInitialAreaArms`'s side of `InitialAreaArms`.
    fn update_exterior_grid_from_position(
        &mut self,
        world: &mut World,
        rect: &[f32; 3],
        reload: bool,
    ) -> Result<(), StreamError> {
        update_exterior_grid_from_position(world, Some(*rect), reload, self.grid).map(|_| ())
    }

    /// IDA: N/A `RealInitialAreaArms`'s side of `InitialAreaArms`.
    fn initialize_loaded_cells(&mut self, world: &mut World) -> Result<(), StreamError> {
        initialize_loaded_cells(world)
    }

    /// IDA: N/A `RealInitialAreaArms`'s side of `InitialAreaArms`.
    fn warning(&mut self, world: &World, text: String) {
        let disabled = world
            .get_resource::<recomp_game::tes::data_load::DataLoadGlobals>()
            .is_some_and(|load| load.disable_warning_messages_b06b18 != 0);
        (self.grid.load.warning)(text, disabled);
    }
}

/// IDA: N/A which way through the block the run took.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InitialAreaPath {
    /// 0040ECA2, 0040ECBB: 0040EE7A with no tail.
    #[default]
    NoSettings,
    /// 004476B0 found a cell.
    EditorIdCell,
    /// 004478B0 found a world's grid: 0040EE7A with no tail.
    PluginWorldspace,
    /// 004F1630 or 00447740 found a cell.
    CoordinatesCell,
    /// The warnings, then the tail.
    NotFound,
}

/// IDA: N/A what the block leaves besides the world and Rect.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InitialAreaResult {
    pub path: InitialAreaPath,
    /// esi at 0040ED5B.
    pub cell: Option<Entity>,
    /// edi of the coordinate or plugin path.
    pub space: Option<Entity>,
    pub x: i32,
    pub y: i32,
    /// 0040EE4C..0040EE78 ran.
    pub tail: bool,
    /// The 004A7A60 text.
    pub warning: Option<String>,
}

/// IDA: 0040EC2A..0040EC61 and 0040ED91..0040EDC3: `shl 12` on the register, wrapping; `fild`; `fadd` the qword 2048.0 at A30F70; `fstp` float. Completion: 100%.
#[must_use]
pub fn grid_origin_center(grid: i32) -> f32 {
    (f64::from(grid.wrapping_shl(12)) + 2048.0) as f32
}

// IDA: N/A a setting's text, which the static initializers leave as `Default`.
fn setting_text(ini: &SettingCollection, address: u32) -> Result<Vec<u8>, StreamError> {
    let setting = ini
        .find_address(address)
        .ok_or(StreamError("0040EBB9: a starting-cell setting is missing"))?;
    Ok(setting.text.clone().unwrap_or_default())
}

// IDA: 00983479 is `jmp 00983468`: strtol(text, 0, 10) in the C locale: spaces, one sign, digits. Overflow clamps to 0x7FFFFFFF, or 0x80000000 when negative (982B0A, 982B45), rather than wrapping. Completion: 100% of the C locale.
fn atol(text: &[u8]) -> i32 {
    let mut bytes = text
        .iter()
        .copied()
        .skip_while(|&byte| matches!(byte, b' ' | 0x09..=0x0D))
        .peekable();
    let negative = match bytes.peek() {
        Some(b'-') => {
            bytes.next();
            true
        }
        Some(b'+') => {
            bytes.next();
            false
        }
        _ => false,
    };
    let mut value = 0_i64;
    for byte in bytes {
        if !byte.is_ascii_digit() {
            break;
        }
        value = (value * 10 + i64::from(byte - b'0')).min(i64::from(u32::MAX) + 1);
    }
    let value = if negative { -value } else { value };
    value.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

// IDA: N/A a helper, with no address of its own.
fn latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|&byte| char::from(byte)).collect()
}

/// IDA: WinMain 0040EBB9..0040EE79. `rect` is the stack NiPoint3 Phase 14 filled with the default point (D5). A nonempty sStartingCell: 004476B0; a found cell goes to 0040ED5B. A miss: 004478B0; none goes to the warnings; found: 004431F0, the Rect stores with z 0, 00445A10, 00447740(x, y, world, 0), then 0040EE7A with no tail. An empty one: sStartingCellX then Y, either empty goes to 0040EE7A with no tail; `atol` both; edi = the +0x0C head before the walk, which a match replaces, and which an empty node (no next, no item) ends; a null edi warns. 004F1630, then 00447740(create 1), then the warnings. A found cell: interior 004455E0(cell, &Rect); exterior 004C9CF0, 004431F0, the Rect stores. The tail: 00447DB0(-1), 00444EC0(&Rect, 1), 00482310. Completion: 100% of the block.
/// # Errors
/// A stop in an arm, or a null item in the walk's vt+0xD4.
pub fn run_initial_area(
    world: &mut World,
    ini: &SettingCollection,
    arms: &mut dyn InitialAreaArms,
    rect: &mut [f32; 3],
) -> Result<InitialAreaResult, StreamError> {
    let mut result = InitialAreaResult::default();
    let name = setting_text(ini, STARTING_CELL)?;
    let mut cell = None;
    if name.is_empty() {
        let x_text = setting_text(ini, STARTING_CELL_X)?;
        if x_text.is_empty() {
            return Ok(result);
        }
        let y_text = setting_text(ini, STARTING_CELL_Y)?;
        if y_text.is_empty() {
            return Ok(result);
        }
        let (x, y) = (atol(&x_text), atol(&y_text));
        (result.x, result.y) = (x, y);
        // 0040ECD5..0040ED26.
        let items: Vec<Entity> = world
            .resource::<DataHandlerLists>()
            .world_spaces_0c
            .items()
            .collect();
        let mut space = items.first().copied();
        let world_name = setting_text(ini, STARTING_WORLD)?;
        for &item in &items {
            if stricmp(&editor_id_of(world, item), &world_name) == 0 {
                space = Some(item);
                break;
            }
        }
        result.space = space;
        if let Some(space) = space {
            cell = arms.find_master_exterior_cell(world, space, x, y)?;
            if cell.is_none() {
                cell = arms.lookup_or_create_exterior_cell(world, x, y, Some(space), true)?;
            }
        }
        if cell.is_some() {
            result.path = InitialAreaPath::CoordinatesCell;
        }
    } else {
        cell = find_cell_by_editor_id(world, &name)?;
        if cell.is_some() {
            result.path = InitialAreaPath::EditorIdCell;
        } else {
            let (space, x, y) = arms.find_worldspace_grid_in_plugins(world, &name)?;
            if let Some(space) = space {
                result.path = InitialAreaPath::PluginWorldspace;
                result.space = Some(space);
                (result.x, result.y) = (x, y);
                arms.set_current_worldspace(world, Some(space))?;
                *rect = [grid_origin_center(x), grid_origin_center(y), 0.0];
                arms.exit_interior_cell(world, rect)?;
                arms.lookup_or_create_exterior_cell(world, x, y, Some(space), false)?;
                return Ok(result);
            }
        }
    }
    if let Some(found) = cell {
        result.cell = Some(found);
        if cell_is_interior(world, found) {
            arms.enter_interior_cell(world, found, rect)?;
        } else {
            let space = cell_world(world, Some(found));
            arms.set_current_worldspace(world, space)?;
            *rect = [
                grid_origin_center(cell_grid_x(world, found)),
                grid_origin_center(cell_grid_y(world, found)),
                0.0,
            ];
        }
    } else {
        // 0040EDD2..0040EE49.
        result.path = InitialAreaPath::NotFound;
        let x_text = setting_text(ini, STARTING_CELL_X)?;
        let y_text = setting_text(ini, STARTING_CELL_Y)?;
        let world_name = setting_text(ini, STARTING_WORLD)?;
        let text = if !name.is_empty() {
            format!("Could not find starting cell '{}'.", latin1(&name))
        } else if x_text.is_empty() || y_text.is_empty() {
            "Could not find starting cell for INI data.".to_owned()
        } else if world_name.is_empty() {
            format!(
                "Could not find starting cell ({}, {}) in default Tamriel worldspace",
                latin1(&x_text),
                latin1(&y_text),
            )
        } else {
            format!(
                "Could not find starting cell ({}, {}) in worldspace '{}'.",
                latin1(&x_text),
                latin1(&y_text),
                latin1(&world_name),
            )
        };
        arms.warning(world, text.clone());
        result.warning = Some(text);
    }
    // 0040EE4C..0040EE78.
    result.tail = true;
    propagate_runtime_field_to_masters(world);
    arms.update_exterior_grid_from_position(world, rect, true)?;
    arms.initialize_loaded_cells(world)?;
    Ok(result)
}

/// IDA: N/A main.cpp's line for the block, `Phase-15: path ..., cell ..., Rect {...}, tail ...`, as `fixtures/cpp-baseline` holds it. No original run prints it: it's a record of the host's result.
#[must_use]
pub fn phase_15_line(result: &InitialAreaResult, cell_id: u32, rect: &[f32; 3]) -> String {
    let path = match result.path {
        InitialAreaPath::NoSettings => "no settings",
        InitialAreaPath::EditorIdCell => "editor id",
        InitialAreaPath::PluginWorldspace => "plugin worldspace",
        InitialAreaPath::CoordinatesCell => "coordinates",
        InitialAreaPath::NotFound => "not found",
    };
    format!(
        "Phase-15: path {path}, cell {cell_id:X}, Rect {{{}, {}, {}}}, tail {}",
        rect[0],
        rect[1],
        rect[2],
        if result.tail { "ran" } else { "skipped" }
    )
}

#[cfg(test)]
mod atol_tests {
    use super::atol;

    #[test]
    fn overflow_clamps_as_strtol_does() {
        assert_eq!(atol(b"4294967297"), i32::MAX);
        assert_eq!(atol(b"-4294967297"), i32::MIN);
        assert_eq!(atol(b"  -7x"), -7);
        assert_eq!(atol(b""), 0);
    }
}
