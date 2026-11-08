//! Phase 15 (15a, 15b): the DataHandler's world queries and the master cell loads the initial area calls: 004476B0 with 004F06A0, 00447740 with 004471D0, 00447DB0, 004478B0 the plugin re-read, 004F1630 with 004EF520 and 004D1340, 004F03F0, and 004D4310 for an interior.
//!
//! Evidence: objdump of each address named here; P15-RUN-001..009. Every function here was read whole.

use bevy_ecs::prelude::*;
use recomp_core::reader::StreamError;
use recomp_core::x87;

use super::ai_package::parent_cell;
use super::data_handler::{DataHandler, DataHandlerGlobals, DataHandlerLists, FormsById};
use super::data_load::{
    DataLoadGlobals, DataLoadServices, construct_object, load_form_from_file, with_files,
};
use super::form::{FormIdentity, ModReferences};
use super::form_arms::{FormLoadCx, editor_id_of, form_class};
use super::forms_world::{
    TesObjectCell, TesWorldSpace, allocate_cell_data, cell_grid_x, cell_grid_y, cell_is_interior,
    construct_cell, world_add_cell,
};
use super::tes::{GameThreadLocals, TesGlobals};
use super::tes_file::{
    FORM_TYPE_TABLE, GRUP_FOURCC, RecordReader, TesFileData, TesFileId, TesFileServices,
    TesFileStore, advance_chunk, close_tes_file, current_chunk_type, current_record_form_type,
    form_type_from_record_fourcc, master_at, open_tes_file, read_current_chunk_data, record_fourcc,
    rewind_tes_file, seek_record_chunks, set_tes_file_offset, skip_group, skip_record,
};
use crate::bs::file::GameFile;

const TES4: u32 = 0x3453_4554;
const WRLD: u32 = 0x444C_5257;
const CELL: u32 = 0x4C4C_4543;
const ROAD: u32 = 0x4441_4F52;
const TLOD: u32 = 0x444F_4C54;
const EDID: u32 = 0x4449_4445;
const XCLC: u32 = 0x434C_4358;

/// IDA: 0040FEC0's neighbour: the exterior warning 004EFE80 and 00447740 share, A37AD8.
#[must_use]
pub fn invalid_coordinate_warning() -> String {
    "Trying to get exterior cell for invalid cell coordinate. Values must be between -32768 and 32767."
        .to_owned()
}

// IDA: N/A a warning through the load's seam, with g_disableWarningMessages at the call.
fn warn(world: &World, services: &mut DataLoadServices<'_>, text: String) {
    let flag = world
        .get_resource::<DataLoadGlobals>()
        .is_some_and(|load| load.disable_warning_messages_b06b18 != 0);
    (services.warning)(text, flag);
}

// IDA: 004EFEA9..004EFEB8, 004EF1D0 the grid key: movsx of x's low word << 16 | movzx of y's. Completion: 100%.
#[must_use]
pub const fn grid_key(x: i32, y: i32) -> u32 {
    (((x as i16 as i32) << 16) as u32) | (y as u16 as u32)
}

/// IDA: 004F06A0 TESWorldSpace_FindCellByEditorID(name): the +0x30 map in 00452600's order, bucket by bucket and each chain from its head, the first cell whose vt+0xD4 matches (`_stricmp`). A null value is skipped. Completion: 100%.
#[must_use]
pub fn world_find_cell_by_editor_id(world: &World, space: Entity, name: &[u8]) -> Option<Entity> {
    let data = world.get::<TesWorldSpace>(space).expect("a world");
    data.cells_30
        .buckets_08
        .iter()
        .flatten()
        .find_map(|&(_, cell)| {
            (recomp_core::crt::stricmp(&editor_id_of(world, cell), name) == 0).then_some(cell)
        })
}

/// IDA: 004476B0 DataHandler_FindCellByEditorID(name). The caller has tested the null name (004476B6). +0xCC interior cells of +0xC4, signed: a null slot faults at 004476DD. Then each nonnull +0x0C world through 004F06A0. Completion: 100%.
/// # Errors
/// A null interior slot.
pub fn find_cell_by_editor_id(world: &World, name: &[u8]) -> Result<Option<Entity>, StreamError> {
    let handler = world.resource::<DataHandler>();
    let array = &handler.cells_c0;
    for index in 0..array.size_0c as i32 {
        let cell = array.cells[index as usize]
            .ok_or(StreamError("004476DD: vt+0xD4 on a null interior slot"))?;
        if recomp_core::crt::stricmp(&editor_id_of(world, cell), name) == 0 {
            return Ok(Some(cell));
        }
    }
    let spaces: Vec<Entity> = world
        .resource::<DataHandlerLists>()
        .world_spaces_0c
        .items()
        .collect();
    Ok(spaces
        .into_iter()
        .find_map(|space| world_find_cell_by_editor_id(world, space, name)))
}

/// IDA: 004EFE80 TESWorldSpace_LookupCell(x, y): out of range warns (A37AD8) and answers null; otherwise the +0x30 map. Answers whether it warned, for the callers with their own seam. Completion: 100%.
#[must_use]
pub fn world_lookup_cell(world: &World, space: Entity, x: i32, y: i32) -> (Option<Entity>, bool) {
    if x > 0x7FFF || y > 0x7FFF || x < -0x8000 || y < -0x8000 {
        return (None, true);
    }
    let found = world
        .get::<TesWorldSpace>(space)
        .expect("a world")
        .cells_30
        .get(grid_key(x, y))
        .copied();
    (found, false)
}

// IDA: 00459790 TESSaveLoadGame_TakeCreatedExteriorCellID(world ID, x, y): the +0x10 map. Empty with no save. Completion: 100% of a miss.
// TODO: [ ] 004597AF a hit, with save loading.
fn take_created_exterior_cell_id(world: &World) -> Result<u32, StreamError> {
    let game = world
        .resource::<TesGlobals>()
        .save_load_game
        .as_ref()
        .ok_or(StreamError("00459790: a null g_saveLoadGame"))?;
    if game.exterior_references_10.count_0c != 0 {
        return Err(StreamError("004597AF: a saved created-cell list"));
    }
    Ok(0)
}

/// IDA: 0045A530 TESSaveLoadGame_SetCurrentThreadActiveFlag(set): +0x18 bit 0 on the thread 00B33398 +0x10 names, bit 18 on another. Host substitution as `TesSaveLoadGame::active`: `World` code all runs on the main thread. Completion: 100% of that arm.
/// # Errors
/// A null g_saveLoadGame.
pub fn set_save_load_thread_active(world: &mut World, set: bool) -> Result<(), StreamError> {
    let mut globals = world.resource_mut::<TesGlobals>();
    let game = globals
        .save_load_game
        .as_mut()
        .ok_or(StreamError("0045A54D: a null g_saveLoadGame"))?;
    if set {
        game.fields_14[1] |= 1;
    } else {
        game.fields_14[1] &= !1;
    }
    Ok(())
}

// IDA: 004472AB..004472DF, 004F17AD..004F17E1, 004D43AC..004D43E0: 0045A500, 0045A530(!flag), vt+0x6C, 0045A530(flag). Completion: 100%.
fn init_item_toggled(
    world: &mut World,
    form: Entity,
    services: &mut DataLoadServices<'_>,
) -> Result<(), StreamError> {
    let active = world
        .resource::<TesGlobals>()
        .save_load_game
        .as_ref()
        .ok_or(StreamError("004472B1: 0045A500 on a null g_saveLoadGame"))?
        .active();
    set_save_load_thread_active(world, !active)?;
    let form_type = u32::from(
        world
            .get::<FormIdentity>(form)
            .expect("a form")
            .form_type_04,
    );
    let class = form_class(form_type).ok_or(StreamError("vt+0x6C: a form type with no class"))?;
    (class.init_item)(world, form, services)?;
    set_save_load_thread_active(world, active)
}

/// IDA: 004471D0 DataHandler_CreateExteriorCell(name, x, y, world). A null world answers null. FormHeap(0x58) 004D3210, 00459790, 0046C300 on a hit. A nonnull name runs vt+0xD8 004C9FC0, which keeps only a nonempty text. 004C9800(0), 004C97E0(2), 004CA710, 004C9AC0, 004EFEF0: a failure deletes the cell and answers null. Then the toggle around vt+0x6C. Completion: 100% with a save's hit and the delete as stops.
/// # Errors
/// As above, or a stop in vt+0x6C.
pub fn create_exterior_cell(
    world: &mut World,
    name: &[u8],
    x: i32,
    y: i32,
    space: Option<Entity>,
    services: &mut DataLoadServices<'_>,
) -> Result<Option<Entity>, StreamError> {
    let Some(space) = space else {
        return Ok(None);
    };
    let cell = construct_cell(world)?;
    take_created_exterior_cell_id(world)?;
    {
        let mut data = world.get_mut::<TesObjectCell>(cell).expect("a cell");
        if !name.is_empty() {
            data.extra_28.set_editor_id(Some(name));
        }
        data.flags_24 &= !1;
        data.flags_24 = 2;
        allocate_cell_data(&mut data);
        let block = data.data_3c.as_mut().expect("just allocated");
        block[0] = x as u32;
        block[1] = y as u32;
    }
    let mut warnings = Vec::new();
    let added = world_add_cell(world, space, cell, &mut |text| warnings.push(text))?;
    for text in warnings {
        warn(world, services, text);
    }
    if !added {
        return Err(StreamError("0044729E: deleting a new cell (vt+0x10)"));
    }
    init_item_toggled(world, cell, services)?;
    Ok(Some(cell))
}

/// IDA: 00447740 DataHandler_LookupOrCreateExteriorCell(x, y, world, create). A null world takes the +0x0C head, none answers null. Out of range warns, null. 004EFE80; a miss with TLS +0x184 clear and `create` runs 004471D0(`Default`, x, y, world). Completion: 100%.
/// # Errors
/// A stop in 004471D0.
pub fn lookup_or_create_exterior_cell(
    world: &mut World,
    x: i32,
    y: i32,
    space: Option<Entity>,
    create: bool,
    services: &mut DataLoadServices<'_>,
) -> Result<Option<Entity>, StreamError> {
    let space = space.or_else(|| {
        world
            .resource::<DataHandlerLists>()
            .world_spaces_0c
            .items()
            .next()
    });
    let Some(space) = space else {
        return Ok(None);
    };
    let (found, out_of_range) = world_lookup_cell(world, space, x, y);
    if out_of_range {
        warn(world, services, invalid_coordinate_warning());
        return Ok(None);
    }
    if found.is_some() {
        return Ok(found);
    }
    if world.resource::<GameThreadLocals>().byte_184 == 0 && create {
        return create_exterior_cell(world, b"", x, y, Some(space), services);
    }
    Ok(None)
}

/// IDA: 00447DB0 DataHandler_PropagateRuntimeFieldToMasters(a2), a2 unused: each +0x8C8 file until a null item; a master's +0x228 = 00B055CC (0044FBB0). Completion: 100%.
pub fn propagate_runtime_field_to_masters(world: &mut World) {
    let size = world
        .resource::<DataHandlerGlobals>()
        .tes_file_buffer_size_b055cc;
    let mut data = world.resource_mut::<DataHandler>();
    let data = &mut *data;
    for id in &data.files_8c8 {
        if let Some(file) = data.file_store.files.get_mut(id.0).and_then(Option::as_mut)
            && file.is_master()
        {
            file.buffer_size_228 = size;
        }
    }
}

// IDA: 004518B0(1).
fn next_record(
    files: &mut TesFileStore,
    id: TesFileId,
    services: &mut TesFileServices<'_>,
) -> Result<bool, StreamError> {
    skip_record(files, id, true, services)
}

// IDA: 004478B0's walk inside one opened file. Answers (matched, world, x, y). `world` is [esp+0x14], which a file with no match clears at 00447B16.
fn search_plugin_file(
    world: &World,
    files: &mut TesFileStore,
    id: TesFileId,
    name: &[u8],
    services: &mut TesFileServices<'_>,
) -> Result<(bool, Option<Entity>, i32, i32), StreamError> {
    let mut found = None;
    let (mut x, mut y) = (0, 0);
    let mut matched = false;
    // 00447936..00447978 the top level: the first WRLD top group is entered, other groups skipped, records skipped.
    let mut entered = false;
    loop {
        let header = files.get_mut(id)?.record_23c;
        let next = if header.type_23c == GRUP_FOURCC {
            if header.form_id_248 != 0 || header.flags_244 != WRLD {
                skip_group(files, id, services)?
            } else {
                entered = true;
                next_record(files, id, services)?
            }
        } else {
            next_record(files, id, services)?
        };
        if !next || entered {
            break;
        }
    }
    if entered {
        // 00447983..00447B6E.
        loop {
            let header = files.get_mut(id)?.record_23c;
            let kind = header.type_23c;
            if kind == WRLD {
                // 004479A9..004479F4: the record ID's file through 0044FD60, null keeping this one; 0044FB50's byte over the low 24 bits; 0046B250 and the cast.
                let file = files.get_mut(id)?;
                let raw = header.form_id_248;
                let master = master_at(file, id, (raw >> 24) + 1).unwrap_or(id);
                let index = files.get_mut(master)?.mod_index_400;
                let form = (raw & 0x00FF_FFFF) | (u32::from(index) << 24);
                found = world
                    .resource::<FormsById>()
                    .lookup(form)
                    .filter(|&entity| world.get::<TesWorldSpace>(entity).is_some());
            } else if kind == CELL {
                // 00447A18..00447AD8: only a first chunk of EDID; a match goes on to XCLC, whose 8 bytes go to the out-params after a zeroed local.
                let file = files.get_mut(id)?;
                let mut chunk = current_chunk_type(file, services)?;
                if chunk == EDID {
                    let mut text = vec![0_u8; file.chunk_size_254 as usize + 1];
                    read_current_chunk_data(file, &mut text, 0, services)?;
                    if recomp_core::crt::stricmp(super::tes_file::c_bytes(&text), name) == 0 {
                        matched = true;
                        loop {
                            if chunk == XCLC {
                                let mut grid = [0_u8; 8];
                                read_current_chunk_data(file, &mut grid, 8, services)?;
                                x = i32::from_le_bytes([grid[0], grid[1], grid[2], grid[3]]);
                                y = i32::from_le_bytes([grid[4], grid[5], grid[6], grid[7]]);
                                break;
                            }
                            if !advance_chunk(file, services)? {
                                break;
                            }
                            chunk = current_chunk_type(file, services)?;
                            if chunk == 0 {
                                break;
                            }
                        }
                    }
                }
            } else if kind == GRUP_FOURCC {
                // 00447AEC the table at 00447B80: 1, 4, 5 enter; 6 skips; 0 enters a WRLD label, and any other label (0046AF70's cache only) ends the walk, as 2, 3 and past 6 do.
                match header.form_id_248 {
                    0 => {
                        if header.flags_244 != WRLD {
                            form_type_from_record_fourcc(
                                header.flags_244,
                                services.form_type_cache,
                            );
                            break;
                        }
                    }
                    1 | 4 | 5 => {}
                    6 => {
                        if !skip_group(files, id, services)? {
                            break;
                        }
                        continue;
                    }
                    _ => break,
                }
            } else if kind != ROAD {
                break;
            }
            // 00447B59..00447B6E; 00447992 then tests the flag.
            if !next_record(files, id, services)? || matched {
                break;
            }
        }
    }
    Ok((matched, found, x, y))
}

/// IDA: 004478B0 DataHandler_FindWorldspaceGridByEditorIDInPlugins(name, &x, &y): x = y = 0; a null or empty name answers null; each file of +0x8D4 below +0x8D0 that opens (00451A40(0, 0)), walked and closed (004507D0); the first match answers its last WRLD's world. Completion: 100%.
/// # Errors
/// A stop in the readers.
pub fn find_worldspace_grid_in_plugins(
    world: &mut World,
    name: &[u8],
    services: &mut DataLoadServices<'_>,
) -> Result<(Option<Entity>, i32, i32), StreamError> {
    if name.is_empty() {
        return Ok((None, 0, 0));
    }
    let handler = world.resource::<DataHandler>();
    let list: Vec<Option<TesFileId>> =
        handler.mods_by_id_8d4[..handler.loaded_mod_count_8d0 as usize].to_vec();
    with_files(world, services, |world, files, file_services, _host| {
        for id in list.into_iter().flatten() {
            let file = files.get_mut(id)?;
            let (directory, file_name) = (file.directory_120.clone(), file.name_1c.clone());
            if !open_tes_file(files, id, &directory, &file_name, 0, false, file_services)? {
                continue;
            }
            let (matched, found, x, y) = search_plugin_file(world, files, id, name, file_services)?;
            close_tes_file(files.get_mut(id)?, file_services)?;
            if matched {
                return Ok((found, x, y));
            }
        }
        Ok((None, 0, 0))
    })
}

// IDA: 004CA5F0 the block key: an exterior's 004EF1D0(x >> 5, y >> 5), an interior's ID low 24 % 10. 004CA640 the sub-block: (x >> 3, y >> 3), or (ID low 24 % 100) / 10. Completion: 100%.
fn cell_group_labels(world: &World, cell: Entity) -> (u32, u32) {
    if cell_is_interior(world, cell) {
        let id = world.get::<FormIdentity>(cell).expect("a form").form_id_0c & 0x00FF_FFFF;
        (id % 10, (id % 100) / 10)
    } else {
        let (x, y) = (cell_grid_x(world, cell), cell_grid_y(world, cell));
        (grid_key(x >> 5, y >> 5), grid_key(x >> 3, y >> 3))
    }
}

// IDA: a form's vt+0xBC(header, 1, 0) on a GRUP. TESForm 0046B910 (TESWorldSpace's too): a top group labelled with the type's fourcc. TESObjectCELL 004CE0A0, which the C++ didn't have: type 0, an interior 0046B910, an exterior the world's; 1, not interior, 0046AF50's low 24 bits of the world's ID; 2 and 4, not persistent, the block label; 3 and 5, not persistent, the sub-block label. Completion: 100% of the arguments the walk passes.
fn accepts_group(world: &World, form: Entity, header_type: u32, label: u32) -> bool {
    let identity = world.get::<FormIdentity>(form).expect("a form");
    let own = u32::from(identity.form_type_04);
    let type_fourcc = |form_type: u32| {
        FORM_TYPE_TABLE
            .get(form_type as usize)
            .map_or(0, |(_, name)| record_fourcc(name))
    };
    let Some(cell) = world.get::<TesObjectCell>(form) else {
        return header_type == 0 && label == type_fourcc(own);
    };
    let persistent = identity.flags_08 & 0x400 != 0;
    let interior = cell.flags_24 & 1 != 0;
    match header_type {
        0 => {
            // An exterior's call is the world's vt+0xBC; a null world faults, which no caller has.
            let holder = if interior {
                own
            } else {
                u32::from(super::forms_world::FORM_TYPE_WORLD_SPACE)
            };
            label == type_fourcc(holder)
        }
        1 => {
            !interior
                && cell.world_50.is_some_and(|space| {
                    let id = world.get::<FormIdentity>(space).expect("a form").form_id_0c;
                    (id ^ label) & 0x00FF_FFFF == 0
                })
        }
        2 | 4 => !persistent && cell_group_labels(world, form).0 == label,
        3 | 5 => !persistent && cell_group_labels(world, form).1 == label,
        _ => false,
    }
}

// IDA: 00451955..00451A23, 00451920's walk past a failed vt+0x68: 004512A0(1), the TES4 records skipped, the form type's fourcc by vt+0x70 and table 00B05E08; a GRUP enters by vt+0xBC or skips; the form's record ends it; anything else is skipped. Completion: 100%.
fn seek_form_walk(
    world: &World,
    files: &mut TesFileStore,
    id: TesFileId,
    form: Entity,
    services: &mut TesFileServices<'_>,
) -> Result<bool, StreamError> {
    rewind_tes_file(files, id, true, services)?;
    while files.get_mut(id)?.record_23c.type_23c == TES4 {
        next_record(files, id, services)?;
    }
    let identity = *world.get::<FormIdentity>(form).expect("a form");
    let wanted = FORM_TYPE_TABLE
        .get(identity.form_type_04 as usize)
        .map_or(0, |(_, name)| record_fourcc(name));
    loop {
        let header = files.get_mut(id)?.record_23c;
        if header.type_23c == 0 {
            return Ok(false);
        }
        if header.type_23c == GRUP_FOURCC {
            if accepts_group(world, form, header.form_id_248, header.flags_244) {
                next_record(files, id, services)?;
            } else {
                skip_group(files, id, services)?;
            }
        } else if header.type_23c == wanted && header.form_id_248 == identity.form_id_0c {
            return Ok(true);
        } else {
            next_record(files, id, services)?;
        }
    }
}

// IDA: 0045192B..00451945 a read header and a good BSFile.
fn seekable(file: &mut TesFileData) -> bool {
    file.header_read_28c != 0 && file.bs_file_10.as_ref().is_some_and(GameFile::good)
}

// IDA: 00451920 TESFileData_SeekForm(world) with vt+0x68 004EF740: a master file, +0xBC, 00451460 that answers true, the record a WRLD (the byte 00B0607C, 0x35) of the ID. Completion: 100%.
fn seek_world_record(
    world: &World,
    files: &mut TesFileStore,
    id: TesFileId,
    space: Entity,
    services: &mut TesFileServices<'_>,
) -> Result<bool, StreamError> {
    if !seekable(files.get_mut(id)?) {
        return Ok(false);
    }
    let data = world.get::<TesWorldSpace>(space).expect("a world");
    let own = world.get::<FormIdentity>(space).expect("a form").form_id_0c;
    let offset = data.record_offset_bc;
    if files.get_mut(id)?.is_master()
        && offset != 0
        && set_tes_file_offset(files, id, offset, services)?
        && current_record_form_type(files, id, services)? == 0x35
        && files.get_mut(id)?.record_23c.form_id_248 == own
    {
        return Ok(true);
    }
    seek_form_walk(world, files, id, space, services)
}

/// IDA: 00451920 with TESObjectCELL vt+0x68 004CC5D0: a master file; an interior's block +0x24 offset if nonzero (00451460, its answer dropped), an exterior's world 004EF520 at its grid (its answer dropped), then the record must be the CELL of the ID. Otherwise the walk. Completion: 100%.
/// # Errors
/// A read failure in the host.
pub fn seek_cell_record(
    world: &World,
    files: &mut TesFileStore,
    id: TesFileId,
    cell: Entity,
    services: &mut TesFileServices<'_>,
) -> Result<bool, StreamError> {
    if !seekable(files.get_mut(id)?) {
        return Ok(false);
    }
    if files.get_mut(id)?.is_master() {
        let data = world.get::<TesObjectCell>(cell).expect("a cell");
        if data.flags_24 & 1 != 0 {
            let offset = data.data_3c.map_or(0, |block| block[9]);
            if offset != 0 {
                set_tes_file_offset(files, id, offset, services)?;
            }
        } else if let Some(space) = data.world_50 {
            let (x, y) = (cell_grid_x(world, cell), cell_grid_y(world, cell));
            seek_exterior_cell_record(world, space, Some(id), x, y, files, services)?;
        }
        let own = world.get::<FormIdentity>(cell).expect("a form").form_id_0c;
        if current_record_form_type(files, id, services)? == 0x30
            && files.get_mut(id)?.record_23c.form_id_248 == own
        {
            return Ok(true);
        }
    }
    seek_form_walk(world, files, id, cell, services)
}

// IDA: 004EEEF0 the OFST index over +0xAC..+0xB8, each `__ftol2_sse` truncated and >> 12; out of range -1; (max x - min x + 1) * (y - min y) - min x + x, with wrapping `imul`. The +0xB8 conversion comes after x's test. Completion: 100%.
fn ofst_index(data: &TesWorldSpace, x: i32, y: i32) -> i32 {
    let bound = |index: usize| x87::ftol(f32::from_bits(data.master_bounds_ac[index])) >> 12;
    let (min_x, min_y, max_x) = (bound(0), bound(1), bound(2));
    if x > max_x || x < min_x {
        return -1;
    }
    let max_y = bound(3);
    if y > max_y || y < min_y {
        return -1;
    }
    (max_x.wrapping_sub(min_x).wrapping_add(1))
        .wrapping_mul(y.wrapping_sub(min_y))
        .wrapping_sub(min_x)
        .wrapping_add(x)
}

/// IDA: 004EF520 TESWorldSpace_SeekExteriorCellRecord(file, x, y). A master with +0xA8: 004EEEF0's index, -1 or a zero entry false, else 00451460(entry + +0xBC), its answer dropped, true. Otherwise 00451920, the WRLD record skipped into, then the walk: a GRUP of type 1 enters; 4 by the block key, 5 by the sub-block key (the block key (x >> 5, y >> 5) and sub-block (x >> 3, y >> 3) compared in fallthrough, so a type 1 compares too and can't be refused); 6, 8, 9, 10 skip; 2, 3, 7 and past 10 end it. A CELL's grid, persistent 0x7FFFFFFF, from XCLC; a match runs 00451160. TLOD and ROAD skip; any other record ends it. Completion: 100%.
/// # Errors
/// A read failure in the host.
pub fn seek_exterior_cell_record(
    world: &World,
    space: Entity,
    file: Option<TesFileId>,
    x: i32,
    y: i32,
    files: &mut TesFileStore,
    services: &mut TesFileServices<'_>,
) -> Result<bool, StreamError> {
    let data = world.get::<TesWorldSpace>(space).expect("a world");
    if let Some(id) = file
        && files.get_mut(id)?.is_master()
        && let Some(table) = &data.offsets_a8
    {
        let index = ofst_index(data, x, y);
        if index == -1 {
            return Ok(false);
        }
        let entry = *table
            .get(index as usize)
            .ok_or(StreamError("004EF55E: an OFST index past the table"))?;
        if entry == 0 {
            return Ok(false);
        }
        set_tes_file_offset(
            files,
            id,
            entry.wrapping_add(data.record_offset_bc),
            services,
        )?;
        return Ok(true);
    }
    let block = grid_key(x >> 5, y >> 5);
    let sub_block = grid_key(x >> 3, y >> 3);
    let Some(id) = file else {
        return Ok(false);
    };
    if !seek_world_record(world, files, id, space, services)? {
        return Ok(false);
    }
    next_record(files, id, services)?;
    let mut found = false;
    loop {
        let header = files.get_mut(id)?.record_23c;
        if header.type_23c == GRUP_FOURCC {
            // The table 004EF730 over 004EF71C: 1 clears `skip` and compares both; 4 compares both; 5 compares the sub-block; 6, 8, 9, 10 skip. 2, 3, 7 and past 10 leave done set.
            let mut skip = true;
            match header.form_id_248 {
                1 => skip = false,
                4 => {
                    if block == header.flags_244 || sub_block == header.flags_244 {
                        skip = false;
                    }
                }
                5 => {
                    if sub_block == header.flags_244 {
                        skip = false;
                    }
                }
                6 | 8..=10 => {}
                _ => break,
            }
            if skip {
                skip_group(files, id, services)?;
            } else {
                next_record(files, id, services)?;
            }
        } else if header.type_23c == CELL {
            let mut grid = [0_i32; 2];
            if header.flags_244 & 0x400 != 0 {
                grid[0] = 0x7FFF_FFFF;
            } else {
                let file = files.get_mut(id)?;
                let mut chunk = current_chunk_type(file, services)?;
                let mut read = false;
                while chunk != 0 && !read {
                    if chunk == XCLC {
                        let mut bytes = [0_u8; 8];
                        read_current_chunk_data(file, &mut bytes, 8, services)?;
                        grid[0] = i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
                        grid[1] = i32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
                        read = true;
                    }
                    if !advance_chunk(file, services)? {
                        break;
                    }
                    chunk = current_chunk_type(file, services)?;
                }
            }
            if grid == [x, y] {
                found = true;
                seek_record_chunks(files.get_mut(id)?, services)?;
                break;
            }
            next_record(files, id, services)?;
        } else if header.type_23c == TLOD || header.type_23c == ROAD {
            next_record(files, id, services)?;
        } else {
            break;
        }
    }
    Ok(found)
}

// IDA: 00453970 TESSaveLoadGame_IsDeletedReference(cell, reference, key): +0x18 bit 11 moves a reference out of another cell (a host stop), then whether the +0x20 list holds the key. Completion: 100% without a loading save, and of a one-node list.
// TODO: [ ] 00453986 the bit 11 arm; 004539E8 a list past its head, with save loading.
fn save_load_lists_reference(world: &World, key: u32) -> Result<bool, StreamError> {
    let game = world
        .resource::<TesGlobals>()
        .save_load_game
        .as_ref()
        .ok_or(StreamError("00453973: a null g_saveLoadGame"))?;
    if game.fields_14[1] & 0x800 != 0 {
        return Err(StreamError("00453986: 00453970's bit 11 arm"));
    }
    if game.fields_14[3] == key {
        return Ok(true);
    }
    if game.fields_14[4] != 0 {
        return Err(StreamError("004539E8: a +0x20 list past its head"));
    }
    Ok(false)
}

// IDA: 004CCD00 the cell-children test: a GRUP of type 6, 8, 9, 10; otherwise a record of form type 0x31..0x34 or 0x36. Completion: 100%.
fn cell_child_record(
    header: super::tes_file::TesRecordHeader,
    services: &mut TesFileServices<'_>,
) -> bool {
    if header.type_23c == GRUP_FOURCC {
        return matches!(header.form_id_248, 6 | 8..=10);
    }
    let kind = form_type_from_record_fourcc(header.type_23c, services.form_type_cache) as u8;
    (0x31..=0x34).contains(&kind) || kind == 0x36
}

/// IDA: 004D1340 TESObjectCELL_LoadTemporaryReferences(file). A null file answers 0, not a master 1. On the cell's CELL record: another ID answers 0; 004518B0(1); not a child record answers 1; not a type 6 GRUP answers 0; 004518B0(1); not a GRUP answers 0; a type 8 is skipped (00451580), and a following non-child answers 1. Any other record falls to the same test: a GRUP of type 9 or 10, else 0. 004518B0(1); 00B33A9C = the cell; TLS +0x184 = 1; a type 9 GRUP inside is entered; TLS +0x186 = 1; each record of form type 0x31..0x34 or 0x36 whose form isn't loaded and isn't listed (00453970) runs 0044DCF0(file, 0), whose 0 clears the answer; 004518B0(1), and a type 9 GRUP labelled with this cell is entered. TLS +0x186, +0x184 and 00B33A9C clear whatever they held. Completion: 100%.
/// # Errors
/// A stop in 0044DCF0.
pub fn load_cell_temporary_references(
    world: &mut World,
    cell: Entity,
    file: Option<TesFileId>,
    cx_files: &mut TesFileStore,
    services: &mut TesFileServices<'_>,
    host: &mut dyn super::form_arms::DataLoadHost,
) -> Result<bool, StreamError> {
    let Some(id) = file else {
        return Ok(false);
    };
    if !cx_files.get_mut(id)?.is_master() {
        return Ok(true);
    }
    let own = world.get::<FormIdentity>(cell).expect("a form").form_id_0c;
    let header = cx_files.get_mut(id)?.record_23c;
    if header.type_23c == CELL {
        if header.form_id_248 != own {
            return Ok(false);
        }
        next_record(cx_files, id, services)?;
        let header = cx_files.get_mut(id)?.record_23c;
        if !cell_child_record(header, services) {
            return Ok(true);
        }
        if header.type_23c != GRUP_FOURCC || header.form_id_248 != 6 {
            return Ok(false);
        }
        next_record(cx_files, id, services)?;
        let header = cx_files.get_mut(id)?.record_23c;
        if header.type_23c != GRUP_FOURCC {
            return Ok(false);
        }
        if header.form_id_248 == 8 {
            skip_group(cx_files, id, services)?;
            let header = cx_files.get_mut(id)?.record_23c;
            if !cell_child_record(header, services) {
                return Ok(true);
            }
        }
    }
    let header = cx_files.get_mut(id)?.record_23c;
    if header.type_23c != GRUP_FOURCC || !matches!(header.form_id_248, 9 | 10) {
        return Ok(false);
    }
    let mut have = next_record(cx_files, id, services)?;
    world.resource_mut::<DataLoadGlobals>().current_cell_b33a9c = Some(cell);
    world.resource_mut::<GameThreadLocals>().byte_184 = 1;
    if have {
        let header = cx_files.get_mut(id)?.record_23c;
        if header.type_23c == GRUP_FOURCC && header.form_id_248 == 9 {
            have = next_record(cx_files, id, services)?;
        }
    }
    world.resource_mut::<GameThreadLocals>().byte_186 = 1;
    let mut result = true;
    let mut failure = None;
    loop {
        if !have {
            break;
        }
        let header = cx_files.get_mut(id)?.record_23c;
        let kind = form_type_from_record_fourcc(header.type_23c, services.form_type_cache) as u8;
        if !((0x31..=0x34).contains(&kind) || kind == 0x36) {
            break;
        }
        let key = header.form_id_248;
        let existing = world.resource::<FormsById>().lookup(key);
        let listed = match save_load_lists_reference(world, key) {
            Ok(listed) => listed,
            Err(stop) => {
                failure = Some(stop);
                break;
            }
        };
        if existing.is_none() && !listed {
            let mut cx = FormLoadCx {
                file: RecordReader {
                    files: cx_files,
                    id,
                    services,
                },
                host,
            };
            match construct_object(world, &mut cx, false) {
                Ok(0) => result = false,
                Ok(_) => {}
                Err(stop) => {
                    failure = Some(stop);
                    break;
                }
            }
        }
        if !next_record(cx_files, id, services)? {
            break;
        }
        let header = cx_files.get_mut(id)?.record_23c;
        if header.type_23c == GRUP_FOURCC
            && header.form_id_248 == 9
            && header.flags_244 == own
            && !next_record(cx_files, id, services)?
        {
            break;
        }
    }
    let mut tls = world.resource_mut::<GameThreadLocals>();
    tls.byte_186 = 0;
    tls.byte_184 = 0;
    world.resource_mut::<DataLoadGlobals>().current_cell_b33a9c = None;
    match failure {
        Some(stop) => Err(stop),
        None => Ok(result),
    }
}

// IDA: 004CC540 TESObjectCELL_ContainsPosition(pos): an interior false; each coordinate `fstp` float, `fistp` under the round-to-nearest control word, `sar 12`, against the block's grid (0 with no block). Completion: 100%.
pub(crate) fn cell_contains_position(world: &World, cell: Entity, position: [f32; 3]) -> bool {
    if cell_is_interior(world, cell) {
        return false;
    }
    x87::fistp(position[0]) >> 12 == cell_grid_x(world, cell)
        && x87::fistp(position[1]) >> 12 == cell_grid_y(world, cell)
}

/// IDA: 004F03F0 TESWorldSpace_InsertGridReferences(cell). 00401020 on the 00B36100 lock (one thread here). The +0x64 list at the cell's key goes into the cell (004D35D0); then every +0x74 reference whose parent is another cell and whose vt+0x174 position lies in the cell (004CC540). Completion: 100%.
/// # Errors
/// A stop in 004D35D0.
pub fn world_insert_grid_references(
    world: &mut World,
    space: Entity,
    cell: Entity,
) -> Result<(), StreamError> {
    let key = grid_key(cell_grid_x(world, cell), cell_grid_y(world, cell));
    let data = world.get::<TesWorldSpace>(space).expect("a world");
    let keyed: Vec<Entity> = data
        .references_64
        .get(key)
        .map(|list| list.items().collect())
        .unwrap_or_default();
    for reference in keyed {
        super::ai_cell_moves::insert_reference_runtime(world, cell, reference)?;
    }
    let others: Vec<Entity> = world
        .get::<TesWorldSpace>(space)
        .expect("a world")
        .unsorted_74
        .items()
        .collect();
    for reference in others {
        if parent_cell(world, reference) == Some(cell) {
            continue;
        }
        let position = super::ai_package::reference_position(world, reference)?;
        if cell_contains_position(world, cell, position) {
            super::ai_cell_moves::insert_reference_runtime(world, cell, reference)?;
        }
    }
    Ok(())
}

// IDA: 0046B680 TESForm_GetModFileDataAtIndex(index) over the nonnull mod references, then 004520F0 the root and this thread's file. Ghost parents and another thread's clone are the host's stops elsewhere (`update_mod_references_from`): nothing makes one. Completion: 100% of those.
fn mod_file_at(world: &World, form: Entity, index: usize) -> Option<TesFileId> {
    let mods = &world.get::<ModReferences>(form)?.0;
    let &mod_index = mods.get(index)?;
    world.resource::<DataHandler>().mods_by_id_8d4[mod_index as usize]
}

/// IDA: 004F1630 TESWorldSpace_FindMasterExteriorCellAtGrid(x, y). 004EFE80; 00B33AA0 = this, whatever follows; a cell with +0x24 bit 0x10 answers at once. For each nonnull mod file (the count taken before the loop) that is a master and seeks (004EF520): with no cell yet, 004D3210, 004C9800(0), 004CA710, 004C9AC0, 004EFEF0 (its answer dropped), 00459790 with 0046C300, 00447050, whose byte is the answer so far; then 004D1340, whose false clears it. A cell: 004C9F90(1), the toggle around vt+0x6C, 004F03F0. A false answer warns A48380. Completion: 100%, with a save's hit a stop.
/// # Errors
/// A stop below.
pub fn find_master_exterior_cell(
    world: &mut World,
    space: Entity,
    x: i32,
    y: i32,
    services: &mut DataLoadServices<'_>,
) -> Result<Option<Entity>, StreamError> {
    let (mut cell, out_of_range) = world_lookup_cell(world, space, x, y);
    if out_of_range {
        warn(world, services, invalid_coordinate_warning());
    }
    world.resource_mut::<DataLoadGlobals>().current_world_b33aa0 = Some(space);
    if let Some(found) = cell
        && world.get::<TesObjectCell>(found).expect("a cell").flags_24 & 0x10 != 0
    {
        return Ok(cell);
    }
    let mut loaded = true;
    let count = world
        .get::<ModReferences>(space)
        .map_or(0, |mods| mods.0.len());
    for index in 0..count {
        let file = mod_file_at(world, space, index);
        let Some(id) = file else {
            continue;
        };
        let step = with_files(world, services, |world, files, file_services, host| {
            if !files.get_mut(id)?.is_master()
                || !seek_exterior_cell_record(world, space, Some(id), x, y, files, file_services)?
            {
                return Ok((cell, None));
            }
            let mut answer = None;
            let current = match cell {
                Some(found) => found,
                None => {
                    let new = construct_cell(world)?;
                    {
                        let mut data = world.get_mut::<TesObjectCell>(new).expect("a cell");
                        data.flags_24 &= !1;
                        allocate_cell_data(&mut data);
                        let block = data.data_3c.as_mut().expect("just allocated");
                        block[0] = x as u32;
                        block[1] = y as u32;
                    }
                    world_add_cell(world, space, new, &mut |text| file_services.warn(text))?;
                    take_created_exterior_cell_id(world)?;
                    let mut cx = FormLoadCx {
                        file: RecordReader {
                            files: &mut *files,
                            id,
                            services: &mut *file_services,
                        },
                        host: &mut *host,
                    };
                    answer = Some(load_form_from_file(world, new, &mut cx)?);
                    new
                }
            };
            if !load_cell_temporary_references(
                world,
                current,
                Some(id),
                files,
                file_services,
                host,
            )? {
                answer = Some(false);
            }
            Ok((Some(current), answer))
        })?;
        cell = step.0;
        if let Some(answer) = step.1 {
            loaded = answer;
        }
    }
    if let Some(found) = cell {
        world
            .get_mut::<TesObjectCell>(found)
            .expect("a cell")
            .flags_24 |= 0x10;
        init_item_toggled(world, found, services)?;
        world_insert_grid_references(world, space, found)?;
    }
    if !loaded {
        let name: String = editor_id_of(world, space)
            .iter()
            .map(|&byte| char::from(byte))
            .collect();
        let id = world.get::<FormIdentity>(space).expect("a form").form_id_0c;
        warn(
            world,
            services,
            format!(
                "Failed to load temporary data for cell ({x}, {y}) in worldspace '{name}' ({id:08X})."
            ),
        );
    }
    Ok(cell)
}

/// IDA: 004D4310 TESObjectCELL_LoadTemporaryData. +0x24 bit 0x10 answers true. For each nonnull mod file that is a master: an interior's +0x50 (004C9D10) nonzero goes to 00451460, else 00451920 with 004CC5D0; a false seek or a false 004D1340 clears the answer. +0x24 |= 0x10; the toggle around vt+0x6C. Completion: 100%.
/// # Errors
/// A stop below.
pub fn load_interior_cell_temporary_data(
    world: &mut World,
    cell: Entity,
    services: &mut DataLoadServices<'_>,
) -> Result<bool, StreamError> {
    if world.get::<TesObjectCell>(cell).expect("a cell").flags_24 & 0x10 != 0 {
        return Ok(true);
    }
    let mut loaded = true;
    let count = world
        .get::<ModReferences>(cell)
        .map_or(0, |mods| mods.0.len());
    for index in 0..count {
        let Some(id) = mod_file_at(world, cell, index) else {
            continue;
        };
        let good = with_files(world, services, |world, files, file_services, host| {
            if !files.get_mut(id)?.is_master() {
                return Ok(true);
            }
            let data = world.get::<TesObjectCell>(cell).expect("a cell");
            let offset = if data.flags_24 & 1 != 0 {
                data.toft_50
            } else {
                0
            };
            let found = if offset != 0 {
                set_tes_file_offset(files, id, offset, file_services)?
            } else {
                seek_cell_record(world, files, id, cell, file_services)?
            };
            Ok(found
                && load_cell_temporary_references(
                    world,
                    cell,
                    Some(id),
                    files,
                    file_services,
                    host,
                )?)
        })?;
        if !good {
            loaded = false;
        }
    }
    world
        .get_mut::<TesObjectCell>(cell)
        .expect("a cell")
        .flags_24 |= 0x10;
    init_item_toggled(world, cell, services)?;
    Ok(loaded)
}
