//! Phase 12c: the references: TESObjectREFR_LoadForm 004D9D00, which REFR, ACHR and ACRE share, its reset pair 004D6BF0 and 004D9B50, the 004DB260 creation (REFR 004D9A70, Character 0060E540, Creature 00625100, the last two over Actor 005E1880(1)) and 0044DCF0's 0044E001 arm with the cell insertion 004D35D0 and removal 004CECD0. In the C++, forms_references.
//!
//! Evidence: objdump of each address named here; P12-RUN-001's COUNT and SAMPLE rows of types 0x31..0x33, and LIST 8B8.
//! TODO: [ ] vt+0x6C 004DF820 InitItem, 12d (Phase 12's post-load pass; `b3c37ed`, `a225ed6` in the C++).

use bevy_ecs::prelude::*;
use recomp_core::reader::StreamError;
use recomp_core::x87;

use super::actor::{
    Actor, CHARACTER_VTABLES, CREATURE_VTABLES, FORM_FLAG_PLAYER_INIT, FORM_TYPE_CHARACTER,
    FORM_TYPE_CREATURE, FORM_TYPE_REFERENCE, MobileObject, TES_OBJECT_REFR_VTABLE, TesObjectRefr,
    actor_process_level, construct_character_with_add, construct_creature_with_add,
    construct_reference,
};
use super::actor_process::{
    MiddleHighProcess, ProcessInputs, RandomSource, actor_process_manager_refresh,
    actor_process_manager_remove, delete_low_process,
};
use super::base_extra_list::{EXTRA_LIGHT, ExtraDataList, is_extra_chunk, load_extra_chunk};
use super::character_controller::{
    BhkCharacterController, HAVOK_SCALE, actor_character_controller,
};
use super::data_handler::{DataHandler, DataHandlerLists, FormsById};
use super::data_load::{DataLoadGlobals, initialize_form_from_current_record, load_form_from_file};
use super::form::{FORM_FLAG_PARTIAL, FormIdentity, ModReferences};
use super::form_arms::{FormClass, FormLoadCx, default_editor_id, editor_id_of};
use super::form_components::TesObjectLinks;
use super::forms_items::set_change_flags;
use super::forms_misc::set_initialized;
use super::forms_script::map_file_form_id;
use super::forms_world::{
    TesObjectCell, cell_grid_x, cell_grid_y, cell_is_interior, world_add_reference,
    world_remove_reference,
};
use super::player::{PlayerCharacter, player_update_cell_regions};
use super::tes::{GameThreadLocals, TesGlobals};
use super::tes_grid::cell_water_height;

// IDA: the chunks 004D9D00 dispatches on itself.
const EDID: u32 = 0x4449_4445;
const DATA: u32 = 0x4154_4144;
const NAME: u32 = 0x454D_414E;
const ONAM: u32 = 0x4D41_4E4F;
const XSCL: u32 = 0x4C43_5358;

/// IDA: TESForm +0x08's bits the arm tests: 0x20 deleted, 0x800 disabled.
const FORM_FLAG_DELETED: u32 = 0x20;
const FORM_FLAG_DISABLED: u32 = 0x800;

// IDA: 00B33B00 g_saveLoadGame's +0x18, read with no null test at each site here: null faults in the original, a host stop.
pub(crate) fn save_load_flags(world: &World) -> Result<u32, StreamError> {
    world
        .get_resource::<TesGlobals>()
        .and_then(|globals| globals.save_load_game.as_ref())
        .map(|game| game.fields_14[1])
        .ok_or(StreamError("00B33B00: a null g_saveLoadGame faults"))
}

/// IDA: 0045A500 TESSaveLoadGame_TestCurrentThreadActiveFlag on the main thread (+0x18 bit 0; the loader's thread is the main one at startup, whose +0x18 bit 0x12 arm no load reaches). Completion: 100% of the main-thread arm.
pub fn save_load_active(world: &World) -> Result<bool, StreamError> {
    Ok(save_load_flags(world)? & 1 != 0)
}

// IDA: N/A a form's +0x08.
fn form_flags(world: &World, form: Entity) -> u32 {
    world.get::<FormIdentity>(form).expect("a form").flags_08
}

/// IDA: 004DB4A0 TESObjectREFR_IsPersistent: the last mod reference (0046B680(-1)); with one, TLS +0x184 clear, no active save or +0x18 bit 4 set, DataHandler +0xCD4 clear and a non-master file, true; otherwise flag 0x400. In a load TLS +0x184 is set, so it's the flag. Completion: 100%.
pub fn reference_is_persistent(world: &World, reference: Entity) -> Result<bool, StreamError> {
    let flagged = form_flags(world, reference) & FORM_FLAG_PLAYER_INIT != 0;
    let references = &world.get::<ModReferences>(reference).expect("a form").0;
    let Some(&last) = references.last() else {
        return Ok(flagged);
    };
    if world.resource::<GameThreadLocals>().byte_184 != 0 {
        return Ok(flagged);
    }
    let flags = save_load_flags(world)?;
    if flags & 1 != 0 && flags & 0x10 == 0 {
        return Ok(flagged);
    }
    let data = world
        .get_resource::<DataHandler>()
        .ok_or(StreamError("004DB4EE: a null DataHandler faults"))?;
    if data.bytes_cd0[4] != 0 {
        return Ok(flagged);
    }
    let file = data
        .mods_by_id_8d4
        .get(usize::from(last))
        .copied()
        .flatten()
        .and_then(|id| data.file_store.get(id))
        .ok_or(StreamError("004DB4F9: 0044FAA0 on a null TESFile"))?;
    if file.is_master() {
        return Ok(flagged);
    }
    Ok(true)
}

// IDA: 004DDCB0 TESChildCell vt+0 GetPersistentCell, the slot in all three classes (bytes of each _0 vtable): +0x40 when 004DB4A0 is false and flag 0x4000 is clear; with either, +0x40 when it's an interior cell; otherwise 0041FF80. Completion: 100%.
pub(super) fn reference_cell(
    world: &World,
    reference: Entity,
) -> Result<Option<Entity>, StreamError> {
    let parent = world
        .get::<TesObjectRefr>(reference)
        .expect("a reference")
        .parent_cell_40;
    if !reference_is_persistent(world, reference)?
        && form_flags(world, reference) & FORM_FLAG_PARTIAL == 0
    {
        return Ok(parent);
    }
    if let Some(cell) = parent
        && cell_is_interior(world, cell)
    {
        return Ok(parent);
    }
    Ok(world
        .get::<TesObjectRefr>(reference)
        .expect("a reference")
        .extra_data_44
        .persistent_cell())
}

// IDA: N/A the form's extra list out of the world for a call that reads the world, and back.
fn with_extra_list<R>(
    world: &mut World,
    reference: Entity,
    call: impl FnOnce(&World, &mut ExtraDataList) -> R,
) -> R {
    let mut list = std::mem::take(
        &mut world
            .get_mut::<TesObjectRefr>(reference)
            .expect("a reference")
            .extra_data_44,
    );
    let result = call(world, &mut list);
    world
        .get_mut::<TesObjectRefr>(reference)
        .expect("a reference")
        .extra_data_44 = list;
    result
}

// IDA: vt+0x8C SetDeleted(0): REFR 004D65D0, which jumps to 0046A9E0: flag 0x20 clears, vt+0x40(0x40000001). MobileObject and up 0065AA60: that, then 006748B0 on g_actorProcessManager; its other arm needs a set argument. The C++ runs only 0046A9E0, for every class. Completion: 100%.
fn clear_deleted(world: &mut World, reference: Entity) -> Result<(), StreamError> {
    world
        .get_mut::<FormIdentity>(reference)
        .expect("a form")
        .flags_08 &= !FORM_FLAG_DELETED;
    set_change_flags(world, reference, 0x4000_0001)?;
    if world.get::<MobileObject>(reference).is_some() {
        actor_process_manager_refresh(world, reference)?;
    }
    Ok(())
}

/// IDA: 006C5420 (identical-code-folded with NiDefaultAVObjectPalette_SetScene: `mov [ecx+1Ch], eax`), on a reference: the base form. Completion: 100%.
pub fn set_reference_base_form(world: &mut World, reference: Entity, base: Option<Entity>) {
    world
        .get_mut::<TesObjectRefr>(reference)
        .expect("a reference")
        .base_form_1c = base;
}

/// IDA: 004D8A30 TESObjectREFR_SetPosition(x, y, z): dwords to +0x2C, +0x30, +0x34 in order (the raw words, so an sNaN stays), then vt+0x40(4). Completion: 100%.
/// # Errors
/// A stop under vt+0x40 (0045B670's null globals).
pub fn set_reference_position(
    world: &mut World,
    reference: Entity,
    position: [f32; 3],
) -> Result<(), StreamError> {
    world
        .get_mut::<TesObjectRefr>(reference)
        .expect("a reference")
        .position_2c = position;
    set_change_flags(world, reference, 4)
}

/// IDA: 004D89A0 TESObjectREFR_SetRotation(x, y, z): dwords to +0x20, +0x24, +0x28 in order, then vt+0x40(4). Completion: 100%.
/// # Errors
/// A stop under vt+0x40 (0045B670's null globals).
pub fn set_reference_rotation(
    world: &mut World,
    reference: Entity,
    rotation: [f32; 3],
) -> Result<(), StreamError> {
    world
        .get_mut::<TesObjectRefr>(reference)
        .expect("a reference")
        .rotation_20 = rotation;
    set_change_flags(world, reference, 4)
}

/// IDA: 004D8A10 TESObjectREFR_SetRotationZ(value): `fld` the float, `fstp` +0x28 (a signaling NaN comes out quiet), then a tail jump to vt+0x40(4). Completion: 100%.
/// # Errors
/// A stop under vt+0x40 (0045B670's null globals).
pub fn set_reference_rotation_z(
    world: &mut World,
    reference: Entity,
    value: f32,
) -> Result<(), StreamError> {
    world
        .get_mut::<TesObjectRefr>(reference)
        .expect("a reference")
        .rotation_20[2] = x87::fld_fstp_copy(value.to_bits());
    set_change_flags(world, reference, 4)
}

// IDA: vt+0x194 SetParentCell: PlayerCharacter 00667600 (`player::player_update_cell_regions`), Character and Creature 0065A330, TESObjectREFR 004D8AA0. Completion: 100% of the dispatch.
pub(crate) fn set_parent_cell(
    world: &mut World,
    reference: Entity,
    cell: Option<Entity>,
) -> Result<(), StreamError> {
    if world.get::<PlayerCharacter>(reference).is_some() {
        return player_update_cell_regions(world, reference, cell);
    }
    actor_or_reference_set_parent_cell(world, reference, cell)
}

// IDA: 0065A330 Actor_SetParentCell, whose first store is on the controller: an unchanged cell returns; for a nonnull cell with a controller (0065A2C0), +0x318 = `fmul` the qword 0.1428767293691635 on 004CACE0's float, one rounding; then 004D8AA0: for a nonnull, changed cell vt+0x40(4); +0x40 = cell; vt+0x190 with a process runs its vt+0x500: 0068F970, a bare `retn`, on Low and MiddleLow; 00651EF0 on the others, which returns with a null or empty +0x170 and is a host stop past that. A form with no Actor goes straight to 004D8AA0. Completion: 100%, with 00651EF0's list a stop.
// TODO: [ ] 00651F10 the process's +0x170 list.
pub(crate) fn actor_or_reference_set_parent_cell(
    world: &mut World,
    reference: Entity,
    cell: Option<Entity>,
) -> Result<(), StreamError> {
    let parent = world
        .get::<TesObjectRefr>(reference)
        .expect("a reference")
        .parent_cell_40;
    let process = world
        .get::<MobileObject>(reference)
        .and_then(|mobile| mobile.process_58);
    let actor = world.get::<Actor>(reference).is_some();
    if actor {
        if cell == parent {
            return Ok(());
        }
        if let Some(cell) = cell
            && let Some(controller) = actor_character_controller(world, reference)
        {
            let water = cell_water_height(world, cell);
            world
                .get_mut::<BhkCharacterController>(controller)
                .expect("the process's controller")
                .water_height_318 = (f64::from(water) * f64::from(HAVOK_SCALE)) as f32;
        }
    }
    if cell.is_some() && parent != cell {
        set_change_flags(world, reference, 4)?;
    }
    world
        .get_mut::<TesObjectRefr>(reference)
        .expect("a reference")
        .parent_cell_40 = cell;
    if actor
        && let Some(process) = process
        && let Some(middle) = world.get::<MiddleHighProcess>(process)
        && middle.field_170 != 0
    {
        return Err(StreamError("00651F10: 00651EF0, a process's +0x170 list"));
    }
    Ok(())
}

// IDA: 004CECD0 TESObjectCELL_RemoveReference(reference): unlink from +0x48 (0065C620). A persistent cell: 004247B0(null), and an exterior one with a world 004F01F0. Any other: vt+0x194(null), and a reference with neither flag 0x4000 nor 004DB4A0, with DataHandler +0xCD4 clear, runs the cell's vt+0x90(1), a host stop. The 00B35C80 lock holds no game state. Completion: 100% of the control flow.
// TODO: [ ] 004CED63 the cell's vt+0x90(1) 004CCBF0, as the C++ stops (no shipped record reaches it).
fn remove_reference(world: &mut World, cell: Entity, reference: Entity) -> Result<(), StreamError> {
    world
        .get_mut::<TesObjectCell>(cell)
        .expect("a cell")
        .objects_48
        .remove_first_matching(reference);
    if form_flags(world, cell) & FORM_FLAG_PLAYER_INIT != 0 {
        world
            .get_mut::<TesObjectRefr>(reference)
            .expect("a reference")
            .extra_data_44
            .set_persistent_cell(None);
        let data = world.get::<TesObjectCell>(cell).expect("a cell");
        if data.flags_24 & 1 == 0
            && let Some(space) = data.world_50
        {
            world_remove_reference(world, space, reference)?;
        }
        return Ok(());
    }
    set_parent_cell(world, reference, None)?;
    if form_flags(world, reference) & FORM_FLAG_PARTIAL != 0 {
        return Ok(());
    }
    if reference_is_persistent(world, reference)? {
        return Ok(());
    }
    let handler = world
        .get_resource::<DataHandler>()
        .ok_or(StreamError("004CED4B: a null DataHandler faults"))?;
    if handler.bytes_cd0[4] == 0 {
        return Err(StreamError(
            "004CED63: the cell's vt+0x90(1) 004CCBF0 isn't ported",
        ));
    }
    Ok(())
}

// IDA: 004D80C0(0): a reference with neither flag 0x20 nor 0x800, with a type 0x30 extra (0041E650) holding a light, updates its shader, a host stop. A load makes no such extra. Completion: 100% of the control flow.
fn update_reference_light(world: &World, reference: Entity) -> Result<(), StreamError> {
    if form_flags(world, reference) & (FORM_FLAG_DELETED | FORM_FLAG_DISABLED) != 0 {
        return Ok(());
    }
    let list = &world
        .get::<TesObjectRefr>(reference)
        .expect("a reference")
        .extra_data_44;
    if list.get(EXTRA_LIGHT).is_some() {
        return Err(StreamError("004D80F7: the ExtraLight shader update"));
    }
    Ok(())
}

// IDA: 004D35D0 TESObjectCELL_InsertPlayerAndRefresh(reference): vt+0x170, null returns. A persistent cell: prepend to +0x48, 004247B0(cell), an exterior one with a world 004F0120; TLS +0x184 set returns. Any other: 006ECC80's previous cell runs 004CECD0; prepend; vt+0x194(cell); without flag 0x800, a non-null vt+0x154 (the 3D: Character's 00612220 is +0x3C too unless it's culled) is a host stop; TLS +0x184 set skips the refresh arms; 004D80C0(0). Completion: 100% of the load's arms.
// TODO: [ ] 004D3654, 004D3718 the cell's vt+0x90(1); 004D3753..004D38D0 the refresh, when TLS +0x184 is clear (Phase 15).
fn insert_reference(world: &mut World, cell: Entity, reference: Entity) -> Result<(), StreamError> {
    let data = world.get::<TesObjectRefr>(reference).expect("a reference");
    if data.base_form_1c.is_none() {
        return Ok(());
    }
    let loading = world.resource::<GameThreadLocals>().byte_184 != 0;
    if form_flags(world, cell) & FORM_FLAG_PLAYER_INIT != 0 {
        world
            .get_mut::<TesObjectCell>(cell)
            .expect("a cell")
            .objects_48
            .prepend_non_null(reference);
        world
            .get_mut::<TesObjectRefr>(reference)
            .expect("a reference")
            .extra_data_44
            .set_persistent_cell(Some(cell));
        let data = world.get::<TesObjectCell>(cell).expect("a cell");
        if data.flags_24 & 1 == 0
            && let Some(space) = data.world_50
        {
            world_add_reference(world, space, reference)?;
        }
        if !loading {
            return Err(StreamError("004D3654: the cell's vt+0x90(1), past a load"));
        }
        return Ok(());
    }
    let previous = world
        .get::<TesObjectRefr>(reference)
        .expect("a reference")
        .parent_cell_40;
    if let Some(previous) = previous {
        remove_reference(world, previous, reference)?;
    }
    world
        .get_mut::<TesObjectCell>(cell)
        .expect("a cell")
        .objects_48
        .prepend_non_null(reference);
    set_parent_cell(world, reference, Some(cell))?;
    if form_flags(world, reference) & FORM_FLAG_DISABLED == 0
        && world
            .get::<TesObjectRefr>(reference)
            .expect("a reference")
            .loaded_3d_3c
            != 0
    {
        return Err(StreamError("004D36DD: vt+0x148 with a loaded 3D"));
    }
    if !loading {
        return Err(StreamError("004D3731: the refresh arms, past a load"));
    }
    update_reference_light(world, reference)
}

// IDA: 004D9D00 TESObjectREFR_LoadForm. It doesn't test the record's type. 0046AB80(0); 00451210; 0046AB80(0). A deleted record (flag 0x20) keeps only its persistent cell, and returns whether the base is set. Then the chunks. Past them an actor (vt+0x190) with flag 0x800 and g_saveLoadGame +0x18 bit 2 clear leaves the process manager at its level (00659A00, 00674550) and loses its process (vt+0(1), +0x58 = 0). Returns whether +0x1C is set. Completion: 100%.
fn load_reference_form(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<bool, StreamError> {
    set_initialized(world, form, false);
    initialize_form_from_current_record(world, form, cx)?;
    set_initialized(world, form, false);
    if form_flags(world, form) & FORM_FLAG_DELETED != 0 {
        // 004D9D41..004D9D74.
        let mut reference = world.get_mut::<TesObjectRefr>(form).expect("a reference");
        let cell = reference.extra_data_44.persistent_cell();
        reference.extra_data_44.remove_all();
        if cell.is_some() {
            reference.extra_data_44.set_persistent_cell(cell);
        }
        return Ok(reference.base_form_1c.is_some());
    }
    let mut missing = false;
    let mut chunk = cx.file.chunk_type()?;
    while chunk != 0 {
        if is_extra_chunk(chunk) {
            // 004DA017.
            with_extra_list(world, form, |world, list| load_extra_chunk(world, list, cx))?;
        } else {
            match chunk {
                DATA => {
                    if !missing {
                        // 004D9DE6 vt+0x8C(0), then 0x18 bytes into a local (its bytes past a short chunk are the stack's; host zero), copied with `mov` in two groups.
                        clear_deleted(world, form)?;
                        let mut bytes = [0_u8; 0x18];
                        cx.file.read_chunk(&mut bytes, 0x18)?;
                        let word = |at: usize| {
                            f32::from_bits(u32::from_le_bytes(
                                bytes[at..at + 4].try_into().expect("four bytes"),
                            ))
                        };
                        let mut reference =
                            world.get_mut::<TesObjectRefr>(form).expect("a reference");
                        reference.position_2c = [word(0), word(4), word(8)];
                        reference.rotation_20 = [word(12), word(16), word(20)];
                    }
                }
                EDID => {
                    // 004D9E56: 0x200 limit, vt+0xD8 006FDFF0 keeps nothing, in all three classes.
                    cx.file.read_text(0x200)?;
                }
                NAME => {
                    // 004D9E8F..004D9EF4: the mapped ID, g_formsByID, and the __RTDynamicCast to TESBoundObject. Every factory class with that base has TESObject too (each class's RTTI read from the exe), and no other does, so `TesObjectLinks` is the cast.
                    let raw = cx.file.read_dword(0)?.0;
                    let file = cx.file.id;
                    let id = map_file_form_id(world, cx, raw, Some(file))?;
                    let found = world
                        .resource::<FormsById>()
                        .lookup(id)
                        .filter(|&base| world.get::<TesObjectLinks>(base).is_some());
                    world
                        .get_mut::<TesObjectRefr>(form)
                        .expect("a reference")
                        .base_form_1c = found;
                    if found.is_none() {
                        let own = world.get::<FormIdentity>(form).expect("a form").form_id_0c;
                        cx.file.services.warn(format!(
                            "Missing object ({id:08X}) for reference ({own:08X})"
                        ));
                        missing = true;
                    }
                }
                XSCL => {
                    // 004D9F52 004510E0 straight onto +0x38: an empty chunk leaves it.
                    let current = world
                        .get::<TesObjectRefr>(form)
                        .expect("a reference")
                        .scale_38;
                    let (value, _) = cx.file.read_dword(current.to_bits())?;
                    world
                        .get_mut::<TesObjectRefr>(form)
                        .expect("a reference")
                        .scale_38 = f32::from_bits(value);
                }
                ONAM => {
                    // 004D9F9B 00423DF0(8).
                    world
                        .get_mut::<TesObjectRefr>(form)
                        .expect("a reference")
                        .extra_data_44
                        .or_action_flags(8);
                }
                _ => {
                    // 004DA008.
                    cx.file
                        .services
                        .warn("TESObjectREFR::Load abnormally terminated.\n\n".to_owned());
                }
            }
        }
        if !cx.file.advance_chunk()? {
            break;
        }
        chunk = cx.file.chunk_type()?;
    }
    // 004DA030..004DA07D.
    if world.get::<Actor>(form).is_some()
        && form_flags(world, form) & FORM_FLAG_DISABLED != 0
        && save_load_flags(world)? & 4 == 0
    {
        let active = save_load_active(world)?;
        let level = actor_process_level(world, form);
        actor_process_manager_remove(world, form, level, active);
        let process = world
            .get_mut::<MobileObject>(form)
            .expect("an actor")
            .process_58;
        if let Some(process) = process {
            delete_low_process(world, process)?;
        }
        world
            .get_mut::<MobileObject>(form)
            .expect("an actor")
            .process_58 = None;
    }
    Ok(world
        .get::<TesObjectRefr>(form)
        .expect("a reference")
        .base_form_1c
        .is_some())
}

// IDA: 004D6BF0 vt+0x14: +0x38 = 1.0; unless g_saveLoadGame (tested here) has +0x18 bit 2, a nonzero +0x3C is released and cleared, a host stop; 0046C2E0 passes over these types. Completion: 100% of the control flow.
fn reset_reference_14(world: &mut World, form: Entity) -> Result<(), StreamError> {
    let flags = world
        .get_resource::<TesGlobals>()
        .and_then(|globals| globals.save_load_game.as_ref())
        .map_or(0, |game| game.fields_14[1]);
    let mut reference = world.get_mut::<TesObjectRefr>(form).expect("a reference");
    reference.scale_38 = 1.0;
    if flags & 4 == 0 && reference.loaded_3d_3c != 0 {
        return Err(StreamError(
            "004D6C13: releasing a loaded 3D is not reached",
        ));
    }
    Ok(())
}

// IDA: 004D9B50 vt+0x18: a light in the type 0x30 extra is released, a host stop. 0041FF80 the persistent cell. g_saveLoadGame +0x18 bit 2 keeps the type 0x40 entry, unlinked with 00422E20(entry, 0). 0041DFF0(1). The kept entry back (0041E0A0), and 004247B0(cell) when there's a cell. Without bit 2 vt+0x150(0): 004E0F80 (Character 0060E430, Creature 00625020, whose own arm needs a 3D) with a null +0x3C and a null argument runs 00439DC0 on 00B33A1C, which drops the reference from two maps that hold only references with 3D, so nothing here; a nonzero +0x3C is a host stop. 0046C2F0 passes over these types. Completion: 100% of the control flow.
// TODO: [ ] 004E0FCD vt+0x150's 3D release (Phase 15). 00439DC0's maps, with 00B33A1C's owner.
fn reset_reference_18(world: &mut World, form: Entity) -> Result<(), StreamError> {
    let flags = save_load_flags(world)?;
    let mut reference = world.get_mut::<TesObjectRefr>(form).expect("a reference");
    let list = &mut reference.extra_data_44;
    if list.get(EXTRA_LIGHT).is_some() {
        return Err(StreamError("004D9B6A: releasing the light extra's light"));
    }
    let cell = list.persistent_cell();
    let kept = if flags & 4 != 0 {
        let entry = list.get(0x40).cloned();
        if entry.is_some() {
            list.remove_entry(0x40);
        }
        entry
    } else {
        None
    };
    list.remove_all();
    if let Some(entry) = kept {
        list.add(entry);
    }
    if cell.is_some() {
        list.set_persistent_cell(cell);
    }
    if flags & 4 == 0 && reference.loaded_3d_3c != 0 {
        return Err(StreamError("004E0FCD: vt+0x150(0) with a loaded 3D"));
    }
    Ok(())
}

// IDA: N/A the random draws 005E1880's path never makes: `construct_low_process` reads only the inputs' scalars.
pub(super) struct NoRandom;

impl RandomSource for NoRandom {
    /// IDA: N/A `NoRandom`'s side of `RandomSource`.
    fn time64(&mut self) -> i64 {
        0
    }
    /// IDA: N/A `NoRandom`'s side of `RandomSource`.
    fn srand(&mut self, _seed: u32) {}
    /// IDA: N/A `NoRandom`'s side of `RandomSource`.
    fn rand(&mut self) -> i32 {
        0
    }
}

// IDA: 004DB2DB's arm, 0060E540(1), through `FormClass::construct`. LowProcess_ctor 006438F0 reads nothing the load has: +0x88's FormHeap bytes are host zero.
fn construct_loaded_character(world: &mut World) -> Result<Entity, StreamError> {
    let mut random = NoRandom;
    let inputs = ProcessInputs {
        random: &mut random,
        idle_chatter_comment_timer: 0,
        low_process_unset_88: 0,
        save_load_active: save_load_active(world)?,
    };
    construct_character_with_add(world, &inputs, true)
}

// IDA: 004DB29E's arm, 00625100(1), through `FormClass::construct`.
fn construct_loaded_creature(world: &mut World) -> Result<Entity, StreamError> {
    let mut random = NoRandom;
    let inputs = ProcessInputs {
        random: &mut random,
        idle_chatter_comment_timer: 0,
        low_process_unset_88: 0,
        save_load_active: save_load_active(world)?,
    };
    construct_creature_with_add(world, &inputs, true)
}

// IDA: 004DB260(type, 1): 0x31 004D9A70, 0x32 0060E540(1), 0x33 00625100(1); these arms are the only types the 0044E001 arm brings. Completion: 100%.
fn create_reference(world: &mut World, form_type: u32) -> Result<Entity, StreamError> {
    match form_type as u8 {
        FORM_TYPE_REFERENCE => construct_reference(world),
        FORM_TYPE_CHARACTER => construct_loaded_character(world),
        FORM_TYPE_CREATURE => construct_loaded_creature(world),
        _ => Err(StreamError("004DB260: a null form faults in 00447050")),
    }
}

/// IDA: 0044E001, 0044DCF0's arm for types 0x31..0x33. No current cell returns 0. An existing reference: vt+0x170 and the TESChildCell's vt+0: the current cell clears `[ebp-0xD]`, the insert flag; another nonnull one runs 004CECD0. A new one: 004DB260(type, 1). 00447050. On a false load: a null base restores +0x04 with the record's type; a persistent cell takes 004247B0, any other vt+0x194; +0x8B8 takes the reference unless present; a MobileObject leaves process level 3 (00674550); no insert. With a base: the insert (004D35D0), then the base's vt+0x108 or vt+0x10C, which are `xor eax,eax` in every TESBoundObject factory class (006F7070, from each vtable); a set g_saveLoadGame +0x18 bit 12 with a 3D runs vt+0x150(0), a host stop. Finally 004DB4A0 with a file version below qword 00A38538 (0.2 rounded to float, compared with `fcomp`; NaN isn't below) warns and clears flag 0x400. Returns 1. Completion: 100%.
pub fn reference_arm(
    world: &mut World,
    cx: &mut FormLoadCx<'_, '_>,
    existing: Option<Entity>,
    _first: bool,
) -> Result<u32, StreamError> {
    let form_type = cx.file.record_form_type()?;
    let Some(cell) = world.resource::<DataLoadGlobals>().current_cell_b33a9c else {
        return Ok(0);
    };
    let mut insert = true;
    let reference = match existing {
        Some(reference) => {
            let previous = reference_cell(world, reference)?;
            if previous == Some(cell) {
                insert = false;
            } else if let Some(previous) = previous {
                remove_reference(world, previous, reference)?;
            }
            reference
        }
        None => create_reference(world, form_type)?,
    };
    if !load_form_from_file(world, reference, cx)? {
        // 0044E063..0044E0EF.
        if world
            .get::<TesObjectRefr>(reference)
            .expect("a reference")
            .base_form_1c
            .is_none()
        {
            world
                .get_mut::<FormIdentity>(reference)
                .expect("a form")
                .form_type_04 = form_type as u8;
        }
        if form_flags(world, cell) & FORM_FLAG_PLAYER_INIT != 0 {
            world
                .get_mut::<TesObjectRefr>(reference)
                .expect("a reference")
                .extra_data_44
                .set_persistent_cell(Some(cell));
        } else {
            set_parent_cell(world, reference, Some(cell))?;
        }
        let mut lists = world.resource_mut::<DataHandlerLists>();
        if !lists.list_8b8.items().any(|item| item == reference) {
            lists.list_8b8.prepend_non_null(reference);
        }
        if world.get::<MobileObject>(reference).is_some() {
            let active = save_load_active(world)?;
            actor_process_manager_remove(world, reference, 3, active);
        }
        insert = false;
    }
    let base = world
        .get::<TesObjectRefr>(reference)
        .expect("a reference")
        .base_form_1c;
    let Some(base) = base else {
        return Ok(1);
    };
    if insert {
        insert_reference(world, cell, reference)?;
    }
    // 0044E15D..0044E183.
    if world
        .get_resource::<TesGlobals>()
        .and_then(|globals| globals.save_load_game.as_ref())
        .is_some_and(|game| game.fields_14[1] & 0x1000 != 0)
        && world
            .get::<TesObjectRefr>(reference)
            .expect("a reference")
            .loaded_3d_3c
            != 0
    {
        return Err(StreamError("0044E183: vt+0x150(0) with a loaded 3D"));
    }
    // 0044E185..0044E226.
    if reference_is_persistent(world, reference)?
        && f64::from(f32::from_bits(cx.file.file()?.header_3d0[0])) < f64::from(0.2_f32)
    {
        let text = old_file_warning(world, reference, base, cell, cx)?;
        cx.file.services.warn(text);
        world
            .get_mut::<FormIdentity>(reference)
            .expect("a form")
            .flags_08 &= !FORM_FLAG_PLAYER_INIT;
    }
    Ok(1)
}

// IDA: 0044E1AB..0044E21E the text at 00A384C8, with the editor IDs as vt+0xD4 gives them.
fn old_file_warning(
    world: &World,
    reference: Entity,
    base: Entity,
    cell: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<String, StreamError> {
    let id = |form: Entity| world.get::<FormIdentity>(form).expect("a form").form_id_0c;
    let latin1 =
        |bytes: Vec<u8>| -> String { bytes.iter().map(|&byte| char::from(byte)).collect() };
    Ok(format!(
        "ref ({:08X}) to base object {} ({:08X}) in cell {} ({}, {}) ({:08X}) is persistent in the non-persistent file {}.",
        id(reference),
        latin1(editor_id_of(world, base)),
        id(base),
        latin1(editor_id_of(world, cell)),
        cell_grid_x(world, cell),
        cell_grid_y(world, cell),
        id(cell),
        cx.file.file()?.name_1c,
    ))
}

// IDA: 004D6DF0 vt+0xD4, in all three classes. With 00B333F4 clear it sets the guard and calls itself through the slot, whose inner run answers the base's vt+0xD4 (or "" with no base, or DataHandler +0xCD4 set); a nonempty answer returns `Default` "", an empty one the base's again, which is empty. So the top-level answer is always "": 004129A0's. Completion: 100%.
fn reference_editor_id(world: &World, form: Entity) -> Vec<u8> {
    default_editor_id(world, form)
}

/// IDA: this family's classes: REFR, and the two whose vtables hold the same 004D9D00, 004D6BF0 and 004D9B50 (read from the exe). The factory 00447DF0 has no arm for 0x32 and 0x33: 004DB260 builds them. InitItem 004DF820 is 12d's.
pub const CLASSES: &[FormClass] = &[
    FormClass {
        form_type: FORM_TYPE_REFERENCE,
        vtable: TES_OBJECT_REFR_VTABLE,
        construct: construct_reference,
        load_form: load_reference_form,
        reset_14: reset_reference_14,
        reset_18: reset_reference_18,
        editor_id: reference_editor_id,
        init_item: super::forms_init_world::init_reference,
        slots: &[],
        special_arm: Some(reference_arm),
    },
    FormClass {
        form_type: FORM_TYPE_CHARACTER,
        vtable: CHARACTER_VTABLES[0],
        construct: construct_loaded_character,
        load_form: load_reference_form,
        reset_14: reset_reference_14,
        reset_18: reset_reference_18,
        editor_id: reference_editor_id,
        init_item: super::forms_init_world::init_reference,
        slots: &[],
        special_arm: Some(reference_arm),
    },
    FormClass {
        form_type: FORM_TYPE_CREATURE,
        vtable: CREATURE_VTABLES[0],
        construct: construct_loaded_creature,
        load_form: load_reference_form,
        reset_14: reset_reference_14,
        reset_18: reset_reference_18,
        editor_id: reference_editor_id,
        init_item: super::forms_init_world::init_reference,
        slots: &[],
        special_arm: Some(reference_arm),
    },
];
