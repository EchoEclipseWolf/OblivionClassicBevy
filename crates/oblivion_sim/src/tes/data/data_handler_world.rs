//! Phase 15 (15a, 15b): the DataHandler's world queries and the master cell loads the initial area calls: 004476B0 with 004F06A0, 00447740 with 004471D0, 00447DB0, 004478B0 the plugin re-read, 004F1630 with 004EF520 and 004D1340, 004F03F0, and 004D4310 for an interior.
//!
//! Evidence: objdump of each address named here; P15-RUN-001..009. Every function here was read whole.

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;

use super::data_handler::{DataHandler, DataHandlerGlobals, DataHandlerLists, FormsById};
use super::data_load::{DataLoadGlobals, DataLoadServices, with_files};
use super::tes_file::{
    FORM_TYPE_TABLE, GRUP_FOURCC, TesFileData, TesFileId, TesFileServices, TesFileStore,
    advance_chunk, close_tes_file, current_chunk_type, current_record_form_type,
    form_type_from_record_fourcc, master_at, open_tes_file, read_current_chunk_data,
    rewind_tes_file, set_tes_file_offset, skip_group, skip_record,
};
use crate::bs::file::GameFile;
use crate::tes::actor::door_lock_callback;
use crate::tes::ai_package::compare_length;
use crate::tes::form::{FormIdentity, record_fourcc};
use crate::tes::form_arms::{editor_id_of, form_class};
use crate::tes::forms_references::{parent_cell, reference_position};
use crate::tes::forms_world::{
    TesObjectCell, TesWorldSpace, allocate_cell_data, cell_grid_x, cell_grid_y, cell_is_interior,
    cell_region_lock, cell_region_unlock, cell_world, construct_cell, grid_key,
    seek_exterior_cell_record, world_add_cell, world_find_cell_by_editor_id, world_lookup_cell,
};
use crate::tes::tes::{GameThreadLocals, TesGlobals};

const TES4: u32 = 0x3453_4554;
const WRLD: u32 = 0x444C_5257;
pub(crate) const CELL: u32 = 0x4C4C_4543;
pub(crate) const ROAD: u32 = 0x4441_4F52;
const EDID: u32 = 0x4449_4445;
pub(crate) const XCLC: u32 = 0x434C_4358;

/// IDA: 0040FEC0's neighbour: the exterior warning 004EFE80 and 00447740 share, A37AD8.
#[must_use]
pub fn invalid_coordinate_warning() -> String {
    "Trying to get exterior cell for invalid cell coordinate. Values must be between -32768 and 32767."
        .to_owned()
}

// IDA: N/A a warning through the load's seam, with g_disableWarningMessages at the call.
pub(crate) fn warn(world: &World, services: &mut DataLoadServices<'_>, text: String) {
    let flag = world
        .get_resource::<DataLoadGlobals>()
        .is_some_and(|load| load.disable_warning_messages_b06b18 != 0);
    (services.warning)(text, flag);
}

/// IDA: 004476B0 DataHandler_FindCellByEditorID(name). The caller has tested the null name (004476B6). +0xCC interior cells of +0xC4, signed: a null slot faults at 004476DD. Then each nonnull +0x0C world through 004F06A0. Completion: 100%.
/// # Errors
/// A null interior slot.
pub fn find_cell_by_editor_id(world: &World, name: &[u8]) -> Result<Option<Entity>, GameError> {
    let handler = world.resource::<DataHandler>();
    let array = &handler.cells_c0;
    for index in 0..array.size_0c as i32 {
        let cell = array.cells[index as usize].ok_or(GameError::unclassified(
            0x004476DD,
            "004476DD: vt+0xD4 on a null interior slot",
        ))?;
        if oblivion_core::crt::stricmp(&editor_id_of(world, cell), name) == 0 {
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

// IDA: 00459790 TESSaveLoadGame_TakeCreatedExteriorCellID(world ID, x, y): the +0x10 map. Empty with no save. Completion: 100% of a miss.
// TODO: [ ] 004597AF a hit, with save loading.
pub(crate) fn take_created_exterior_cell_id(world: &World) -> Result<u32, GameError> {
    let game = world
        .resource::<TesGlobals>()
        .save_load_game
        .as_ref()
        .ok_or(GameError::unclassified(
            0x00459790,
            "00459790: a null g_saveLoadGame",
        ))?;
    if game.exterior_references_10.count_0c != 0 {
        return Err(GameError::unclassified(
            0x004597AF,
            "004597AF: a saved created-cell list",
        ));
    }
    Ok(0)
}

/// IDA: 0045A530 TESSaveLoadGame_SetCurrentThreadActiveFlag(set): +0x18 bit 0 on the thread 00B33398 +0x10 names, bit 18 on another. Host substitution as `TesSaveLoadGame::active`: `World` code all runs on the main thread. Completion: 100% of that arm.
/// # Errors
/// A null g_saveLoadGame.
pub fn set_save_load_thread_active(world: &mut World, set: bool) -> Result<(), GameError> {
    let mut globals = world.resource_mut::<TesGlobals>();
    let game = globals
        .save_load_game
        .as_mut()
        .ok_or(GameError::unclassified(
            0x0045A54D,
            "0045A54D: a null g_saveLoadGame",
        ))?;
    if set {
        game.fields_14[1] |= 1;
    } else {
        game.fields_14[1] &= !1;
    }
    Ok(())
}

// IDA: 004472AB..004472DF, 004F17AD..004F17E1, 004D43AC..004D43E0: 0045A500, 0045A530(!flag), vt+0x6C, 0045A530(flag). Completion: 100%.
pub(crate) fn init_item_toggled(
    world: &mut World,
    form: Entity,
    services: &mut DataLoadServices<'_>,
) -> Result<(), GameError> {
    let active = world
        .resource::<TesGlobals>()
        .save_load_game
        .as_ref()
        .ok_or(GameError::unclassified(
            0x004472B1,
            "004472B1: 0045A500 on a null g_saveLoadGame",
        ))?
        .active();
    set_save_load_thread_active(world, !active)?;
    let form_type = u32::from(
        world
            .get::<FormIdentity>(form)
            .expect("a form")
            .form_type_04,
    );
    let class = form_class(form_type).ok_or(GameError::unclassified(
        0,
        "vt+0x6C: a form type with no class",
    ))?;
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
) -> Result<Option<Entity>, GameError> {
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
        return Err(GameError::unclassified(
            0x0044729E,
            "0044729E: deleting a new cell (vt+0x10)",
        ));
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
) -> Result<Option<Entity>, GameError> {
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
pub(crate) fn next_record(
    files: &mut TesFileStore,
    id: TesFileId,
    services: &mut TesFileServices<'_>,
) -> Result<bool, GameError> {
    skip_record(files, id, true, services)
}

// IDA: 004478B0's walk inside one opened file. Answers (matched, world, x, y). `world` is [esp+0x14], which a file with no match clears at 00447B16.
fn search_plugin_file(
    world: &World,
    files: &mut TesFileStore,
    id: TesFileId,
    name: &[u8],
    services: &mut TesFileServices<'_>,
) -> Result<(bool, Option<Entity>, i32, i32), GameError> {
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
                    if oblivion_core::crt::stricmp(super::tes_file::c_bytes(&text), name) == 0 {
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
) -> Result<(Option<Entity>, i32, i32), GameError> {
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
                u32::from(crate::tes::forms_world::FORM_TYPE_WORLD_SPACE)
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
) -> Result<bool, GameError> {
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
pub(crate) fn seek_world_record(
    world: &World,
    files: &mut TesFileStore,
    id: TesFileId,
    space: Entity,
    services: &mut TesFileServices<'_>,
) -> Result<bool, GameError> {
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
) -> Result<bool, GameError> {
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

// IDA: 00453970 TESSaveLoadGame_IsDeletedReference(cell, reference, key): +0x18 bit 11 moves a reference out of another cell (a host stop), then whether the +0x20 list holds the key. Completion: 100% without a loading save, and of a one-node list.
// TODO: [ ] 00453986 the bit 11 arm; 004539E8 a list past its head, with save loading.
pub(crate) fn save_load_lists_reference(world: &World, key: u32) -> Result<bool, GameError> {
    let game = world
        .resource::<TesGlobals>()
        .save_load_game
        .as_ref()
        .ok_or(GameError::unclassified(
            0x00453973,
            "00453973: a null g_saveLoadGame",
        ))?;
    if game.fields_14[1] & 0x800 != 0 {
        return Err(GameError::unclassified(
            0x00453986,
            "00453986: 00453970's bit 11 arm",
        ));
    }
    if game.fields_14[3] == key {
        return Ok(true);
    }
    if game.fields_14[4] != 0 {
        return Err(GameError::unclassified(
            0x004539E8,
            "004539E8: a +0x20 list past its head",
        ));
    }
    Ok(false)
}

// IDA: 004CCD00 the cell-children test: a GRUP of type 6, 8, 9, 10; otherwise a record of form type 0x31..0x34 or 0x36. Completion: 100%.
pub(crate) fn cell_child_record(
    header: super::tes_file::TesRecordHeader,
    services: &mut TesFileServices<'_>,
) -> bool {
    if header.type_23c == GRUP_FOURCC {
        return matches!(header.form_id_248, 6 | 8..=10);
    }
    let kind = form_type_from_record_fourcc(header.type_23c, services.form_type_cache) as u8;
    (0x31..=0x34).contains(&kind) || kind == 0x36
}

/// IDA: 00446B90 DataHandler_ForEachReferenceInRange(cell, position, 5000, position, 5000, callback, actor): an interior through 004CBC20, under the cell's region lock (which the C++ doesn't take), with +0x48's next read before the callback: a reference at or past the radius (00480520, each delta a float) is skipped. An exterior goes through 004F0750, a host stop. `lock` picks 00645A30 over 00645AF0. Completion: 100% of the interior arm.
/// TODO: [ ] 004F0750, the exterior search: no shipped package reached it in P14-RUN-004.
pub fn doors_near(world: &mut World, actor: Entity, lock: bool) -> Result<(), GameError> {
    let Some(cell) = parent_cell(world, actor) else {
        return Ok(());
    };
    if !cell_is_interior(world, cell) {
        if cell_world(world, Some(cell)).is_some() {
            return Err(GameError::unclassified(
                0x004F0750,
                "004F0750: the exterior range search",
            ));
        }
        return Ok(());
    }
    let center = reference_position(world, actor)?;
    cell_region_lock(world, cell)?;
    let items: Vec<Entity> = world
        .get::<TesObjectCell>(cell)
        .expect("a cell")
        .objects_48
        .items()
        .collect();
    for reference in items {
        let position = reference_position(world, reference)?;
        let delta = [
            position[0] - center[0],
            position[1] - center[1],
            position[2] - center[2],
        ];
        if compare_length(delta, 5000.0) >= 0 {
            continue;
        }
        door_lock_callback(world, reference, actor, lock)?;
    }
    cell_region_unlock(world, cell);
    Ok(())
}
