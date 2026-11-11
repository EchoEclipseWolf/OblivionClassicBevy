//! Phase 12c: the references: TESObjectREFR_LoadForm 004D9D00, which REFR, ACHR and ACRE share, its reset pair 004D6BF0 and 004D9B50, the 004DB260 creation (REFR 004D9A70, Character 0060E540, Creature 00625100, the last two over Actor 005E1880(1)) and 0044DCF0's 0044E001 arm, which calls the cell's insertion 004D35D0 and removal 004CECD0 (`forms_world`). In the C++, forms_references.
//!
//! Evidence: objdump of each address named here; P12-RUN-001's COUNT and SAMPLE rows of types 0x31..0x33, and LIST 8B8.
//! TODO: [ ] vt+0x6C 004DF820 InitItem, 12d (Phase 12's post-load pass; `b3c37ed`, `a225ed6` in the C++).

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;
use oblivion_core::x87;

use super::base_extra_list::{EXTRA_LIGHT, ExtraDataList, is_extra_chunk, load_extra_chunk};
use super::form::{FORM_FLAG_PARTIAL, FormIdentity, ModReferences};
use super::form_arms::{FormClass, FormLoadCx, default_editor_id};
use super::form_components::TesObjectLinks;
use super::forms_actors::{base_race, npc_sex, race_scale};
use super::forms_items::set_change_flags;
use super::forms_misc::set_initialized;
use super::forms_world::{cell_is_interior, cell_remove_reference, cell_world};
use crate::gamebryo::transform::multiply;
use crate::tes::actor::{
    Actor, CHARACTER_VTABLES, CREATURE_VTABLES, Character, EXTRA_DATA_LIST_VTABLE,
    FORM_FLAG_PLAYER_INIT, FORM_TYPE_CHARACTER, FORM_TYPE_CREATURE, FORM_TYPE_REFERENCE,
    MobileObject, TES_CHILD_CELL_VTABLE, TES_OBJECT_REFR_CHILD_CELL_VTABLE, TES_OBJECT_REFR_VTABLE,
    TesObjectRefr, actor_is_dead, actor_or_reference_set_parent_cell, actor_process_level,
    construct_character_with_add, construct_creature_with_add, process_of,
};
use crate::tes::actor_package::desired_level_is_3;
use crate::tes::actor_process::{
    HighProcess, ProcessInputs, RandomSource, actor_process_manager_refresh,
    actor_process_manager_remove, delete_low_process,
};
use crate::tes::ai_cell_moves::{cell_list, insert_reference_runtime, list_of, set_3d_null};
use crate::tes::ai_conditions::lock_block;
use crate::tes::ai_package::FLT_MAX;
use crate::tes::ai_process::{AiProcessGlobals, actor_get_actor_value_float};
use crate::tes::base_extra_list::{
    EXTRA_CONTAINER_CHANGES, EXTRA_GLOBAL, EXTRA_LOCK, EXTRA_OWNER, EXTRA_RANK, EXTRA_TELEPORT,
    ExtraData, extra_form, set_package_start_location, teleport_door,
};
use crate::tes::character_controller::actor_character_controller;
use crate::tes::data_handler::{DataHandler, FormsById};
use crate::tes::data_load::{initialize_form_from_current_record, reference_arm};
use crate::tes::default_forms::DefaultFormStatics;
use crate::tes::form::{OriginalVtable, construct_tes_form};
use crate::tes::form_components::{
    FormRef, TesAiForm, TesContainer, base_faction_rank, container_weight,
};
use crate::tes::forms_init::form_type;
use crate::tes::forms_items::{TesActivator, TesDoor, TesLight};
use crate::tes::forms_world::{cell_attached, world_insert_persistent, world_remove_persistent};
use crate::tes::game_settings::GameSettingCollection;
use crate::tes::game_sounds::{GameSoundServices, create_sound};
use crate::tes::global::TESGlobal;
use crate::tes::inventory::{
    container_changes_total_weight, ensure_container_changes, put_container_changes,
    reference_container,
};
use crate::tes::inventory_menu::form_id;
use crate::tes::player::{
    PlayerCharacter, player_is_sleeping, player_update_cell_regions, the_player,
};
use crate::tes::stats_menu::player;
use crate::tes::tes::{GameThreadLocals, TesGlobals};
use crate::tes::tes_file::map_file_form_id;
use crate::tes::tile::InterfaceCx;

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
pub(crate) fn save_load_flags(world: &World) -> Result<u32, GameError> {
    world
        .get_resource::<TesGlobals>()
        .and_then(|globals| globals.save_load_game.as_ref())
        .map(|game| game.fields_14[1])
        .ok_or(GameError::unclassified(
            0x00B33B00,
            "00B33B00: a null g_saveLoadGame faults",
        ))
}

/// IDA: 0045A500 TESSaveLoadGame_TestCurrentThreadActiveFlag on the main thread (+0x18 bit 0; the loader's thread is the main one at startup, whose +0x18 bit 0x12 arm no load reaches). Completion: 100% of the main-thread arm.
pub fn save_load_active(world: &World) -> Result<bool, GameError> {
    Ok(save_load_flags(world)? & 1 != 0)
}

/// IDA: N/A a form's +0x08.
#[must_use]
pub fn form_flags(world: &World, form: Entity) -> u32 {
    world.get::<FormIdentity>(form).expect("a form").flags_08
}

/// IDA: 004DB4A0 TESObjectREFR_IsPersistent: the last mod reference (0046B680(-1)); with one, TLS +0x184 clear, no active save or +0x18 bit 4 set, DataHandler +0xCD4 clear and a non-master file, true; otherwise flag 0x400. In a load TLS +0x184 is set, so it's the flag. Completion: 100%.
pub fn reference_is_persistent(world: &World, reference: Entity) -> Result<bool, GameError> {
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
        .ok_or(GameError::unclassified(
            0x004DB4EE,
            "004DB4EE: a null DataHandler faults",
        ))?;
    if data.bytes_cd0[4] != 0 {
        return Ok(flagged);
    }
    let file = data
        .mods_by_id_8d4
        .get(usize::from(last))
        .copied()
        .flatten()
        .and_then(|id| data.file_store.get(id))
        .ok_or(GameError::unclassified(
            0x004DB4F9,
            "004DB4F9: 0044FAA0 on a null TESFile",
        ))?;
    if file.is_master() {
        return Ok(flagged);
    }
    Ok(true)
}

// IDA: 004DDCB0 TESChildCell vt+0 GetPersistentCell, the slot in all three classes (bytes of each _0 vtable): +0x40 when 004DB4A0 is false and flag 0x4000 is clear; with either, +0x40 when it's an interior cell; otherwise 0041FF80. Completion: 100%.
pub(crate) fn reference_cell(
    world: &World,
    reference: Entity,
) -> Result<Option<Entity>, GameError> {
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
fn clear_deleted(world: &mut World, reference: Entity) -> Result<(), GameError> {
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
) -> Result<(), GameError> {
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
) -> Result<(), GameError> {
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
) -> Result<(), GameError> {
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
) -> Result<(), GameError> {
    if world.get::<PlayerCharacter>(reference).is_some() {
        return player_update_cell_regions(world, reference, cell);
    }
    actor_or_reference_set_parent_cell(world, reference, cell)
}

// IDA: 004D80C0(0): a type 0x30 extra's block needs its pointer and that's +0x00 nonnull to go on. `ExtraData` holds it as a dword: nonzero stops.
pub(crate) fn refresh_light(world: &World, reference: Entity) -> Result<(), GameError> {
    if form_flags(world, reference) & (0x20 | 0x800) != 0 {
        return Ok(());
    }
    let list = &world
        .get::<TesObjectRefr>(reference)
        .expect("a reference")
        .extra_data_44;
    if list
        .get(EXTRA_LIGHT)
        .is_some_and(|entry| !matches!(entry.data, super::base_extra_list::ExtraData::Dword(0)))
    {
        return Err(GameError::unclassified(
            0x004D80F7,
            "004D80F7: the ExtraLight shader update",
        ));
    }
    Ok(())
}

// IDA: 004D9D00 TESObjectREFR_LoadForm. It doesn't test the record's type. 0046AB80(0); 00451210; 0046AB80(0). A deleted record (flag 0x20) keeps only its persistent cell, and returns whether the base is set. Then the chunks. Past them an actor (vt+0x190) with flag 0x800 and g_saveLoadGame +0x18 bit 2 clear leaves the process manager at its level (00659A00, 00674550) and loses its process (vt+0(1), +0x58 = 0). Returns whether +0x1C is set. Completion: 100%.
fn load_reference_form(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<bool, GameError> {
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
fn reset_reference_14(world: &mut World, form: Entity) -> Result<(), GameError> {
    let flags = world
        .get_resource::<TesGlobals>()
        .and_then(|globals| globals.save_load_game.as_ref())
        .map_or(0, |game| game.fields_14[1]);
    let mut reference = world.get_mut::<TesObjectRefr>(form).expect("a reference");
    reference.scale_38 = 1.0;
    if flags & 4 == 0 && reference.loaded_3d_3c != 0 {
        return Err(GameError::unclassified(
            0x004D6C13,
            "004D6C13: releasing a loaded 3D is not reached",
        ));
    }
    Ok(())
}

// IDA: 004D9B50 vt+0x18: a light in the type 0x30 extra is released, a host stop. 0041FF80 the persistent cell. g_saveLoadGame +0x18 bit 2 keeps the type 0x40 entry, unlinked with 00422E20(entry, 0). 0041DFF0(1). The kept entry back (0041E0A0), and 004247B0(cell) when there's a cell. Without bit 2 vt+0x150(0): 004E0F80 (Character 0060E430, Creature 00625020, whose own arm needs a 3D) with a null +0x3C and a null argument runs 00439DC0 on 00B33A1C, which drops the reference from two maps that hold only references with 3D, so nothing here; a nonzero +0x3C is a host stop. 0046C2F0 passes over these types. Completion: 100% of the control flow.
// TODO: [ ] 004E0FCD vt+0x150's 3D release (Phase 15). 00439DC0's maps, with 00B33A1C's owner.
fn reset_reference_18(world: &mut World, form: Entity) -> Result<(), GameError> {
    let flags = save_load_flags(world)?;
    let mut reference = world.get_mut::<TesObjectRefr>(form).expect("a reference");
    let list = &mut reference.extra_data_44;
    if list.get(EXTRA_LIGHT).is_some() {
        return Err(GameError::unclassified(
            0x004D9B6A,
            "004D9B6A: releasing the light extra's light",
        ));
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
        return Err(GameError::unclassified(
            0x004E0FCD,
            "004E0FCD: vt+0x150(0) with a loaded 3D",
        ));
    }
    Ok(())
}

// IDA: N/A the random draws 005E1880's path never makes: `construct_low_process` reads only the inputs' scalars.
pub(in crate::tes) struct NoRandom;

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
pub(crate) fn construct_loaded_character(world: &mut World) -> Result<Entity, GameError> {
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
pub(crate) fn construct_loaded_creature(world: &mut World) -> Result<Entity, GameError> {
    let mut random = NoRandom;
    let inputs = ProcessInputs {
        random: &mut random,
        idle_chatter_comment_timer: 0,
        low_process_unset_88: 0,
        save_load_active: save_load_active(world)?,
    };
    construct_creature_with_add(world, &inputs, true)
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

/// IDA: 006ECC80 +0x40.
#[must_use]
pub fn parent_cell(world: &World, reference: Entity) -> Option<Entity> {
    world.get::<TesObjectRefr>(reference)?.parent_cell_40
}

/// IDA: 005E02E0 TESObjectREFR_GetActorBaseForm(useTemplate): with the argument, edi = vt+0x128 (00447550 on Actor,
/// Character, Creature and PlayerCharacter: +0xD0, the leveled template; 006F7070, 0, on TESObjectREFR and
/// MobileObject); always ebx = vt+0x170 (004D9B40: +0x1C, the base, no cast); ebx is the answer only when edi is null,
/// ebx nonnull and vt+0x190 IsActor (00977C50 true; 0069D990 false) holds (005E030A..005E0320), else edi. So a
/// non-actor answers null. +0xD0 is a raw word the host never sets, so a nonzero one stops. Completion: 100%, past that.
/// # Errors
/// The leveled template's host stop.
pub fn actor_base_form(
    world: &World,
    reference: Entity,
    use_template: bool,
) -> Result<Option<Entity>, GameError> {
    let actor = world.get::<Actor>(reference);
    if use_template
        && let Some(actor) = actor
        && actor.leveled_creature_template_d0 != 0
    {
        return Err(GameError::unclassified(
            0x00447550,
            "00447550: a leveled template (+0xD0)",
        ));
    }
    let base = world
        .get::<TesObjectRefr>(reference)
        .expect("a reference")
        .base_form_1c;
    Ok(base.filter(|_| actor.is_some()))
}

/// IDA: 004D7AF0 TESObjectREFR_BuildInitialRotationMatrix: the identity (g_niMatrix3Identity); unless vt+0x190 IsActor
/// (004D7B1A: 00977C50 true on the actor classes, 0069D990 false on TESObjectREFR and MobileObject), 0070FD30 of +0x20
/// and 0070FD80 of +0x24 multiplied in; then 0070FDD0 of +0x28, for every reference. Each `cos`/`sin` is full precision,
/// stored as a float (D6); each 007100A0 NiMatrix33_Multiply is per step at 24 bits. Then the base (+0x1C, read with no
/// null test: a null one faults at 004D7BA3) when it is an NPC_ (type 0x23) with a race (+0xE8): diag(weight, weight,
/// height) from the race's +0x68 and +0x60 by 00519D20's sex (004D7BD6: above 1 is 0.0 for both) multiplied in.
/// Completion: 100%.
/// # Errors
/// A null base.
pub fn reference_initial_rotation(world: &World, reference: Entity) -> Result<[f32; 9], GameError> {
    let refr = world.get::<TesObjectRefr>(reference).expect("a reference");
    let turn = |angle: f32| (f64::from(angle).cos() as f32, f64::from(angle).sin() as f32);
    let identity = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
    let mut matrix = identity;
    if world.get::<Actor>(reference).is_none() {
        let (cos_x, sin_x) = turn(refr.rotation_20[0]);
        let (cos_y, sin_y) = turn(refr.rotation_20[1]);
        let x = [1.0, 0.0, 0.0, 0.0, cos_x, sin_x, 0.0, -sin_x, cos_x];
        let y = [cos_y, 0.0, -sin_y, 0.0, 1.0, 0.0, sin_y, 0.0, cos_y];
        matrix = multiply(&multiply(&matrix, &x), &y);
    }
    let (cos_z, sin_z) = turn(refr.rotation_20[2]);
    let z = [cos_z, sin_z, 0.0, -sin_z, cos_z, 0.0, 0.0, 0.0, 1.0];
    matrix = multiply(&matrix, &z);
    let base = refr.base_form_1c.ok_or(GameError::unclassified(
        0x004D7BA3,
        "004D7BA3: a null base faults",
    ))?;
    // 004D7BA3 the type byte; a host entity without a FormIdentity isn't an NPC_ (unit fixtures spawn bare bases).
    if world
        .get::<FormIdentity>(base)
        .is_some_and(|identity| identity.form_type_04 == 0x23)
        && let Some(race) = base_race(world, base)?
    {
        let (weight, height) = match npc_sex(world, base) {
            Some(sex) if sex <= 1 => (
                race_scale(world, race, 0x68 + 4 * sex),
                race_scale(world, race, 0x60 + 4 * sex),
            ),
            _ => (0.0, 0.0),
        };
        let scale = [weight, 0.0, 0.0, 0.0, weight, 0.0, 0.0, 0.0, height];
        matrix = multiply(&matrix, &scale);
    }
    Ok(matrix)
}

/// IDA: vt+0x174 Actor 005F10E0: a process with vt+0x2C0 bit 0x800 (High 006285A0: word +0x1FC; the others 0) takes the ring buffer, a host stop; otherwise +0x2C. REFR 004C8D00 +0x2C. Completion: 100%, past the ring buffer. The C++ stops on every non-Low process, so on the player.
pub fn reference_position(world: &World, reference: Entity) -> Result<[f32; 3], GameError> {
    if let Some(process) = process_of(world, reference)
        && world
            .get::<HighProcess>(process)
            .is_some_and(|high| high.movement_flags_1fc & 0x800 != 0)
    {
        return Err(GameError::unclassified(
            0x005F10F1,
            "005F10F1: the ring buffer of a moving process",
        ));
    }
    Ok(world
        .get::<TesObjectRefr>(reference)
        .expect("a reference")
        .position_2c)
}

/// IDA: 004D6670 TESObjectREFR_GetExteriorWorldspace: +0x40, else the TESChildCell's vt+0 004DDCB0, which is +0x40 or, for a persistent or 0x4000 reference, the 0x11 extra's. Completion: 100%.
#[must_use]
pub fn reference_world(world: &World, reference: Entity) -> Option<Entity> {
    if let Some(cell) = parent_cell(world, reference) {
        return cell_world(world, Some(cell));
    }
    let persistent = crate::tes::forms_references::reference_is_persistent(world, reference)
        .expect("004DB4A0: a null g_saveLoadGame or TESFile faults")
        || form_flags(world, reference) & 0x4000 != 0;
    if !persistent {
        return None;
    }
    let cell = world
        .get::<TesObjectRefr>(reference)
        .expect("a reference")
        .extra_data_44
        .persistent_cell();
    cell_world(world, cell)
}

/// IDA: 004D7E30: the point minus the position per axis, each a float store; `(x*x + y*y) + z*z`; `__CIsqrt`, which keeps the precision bits, so the root rounds to float. Completion: 100%.
#[must_use]
pub fn distance_to(position: [f32; 3], point: [f32; 3]) -> f32 {
    let dx = point[0] - position[0];
    let dy = point[1] - position[1];
    let dz = point[2] - position[2];
    ((dx * dx + dy * dy) + dz * dz).sqrt()
}

/// IDA: 004D74D0 TESObjectREFR_IsCreatureOfType4: +0x1C nonnull and its form type (vt+0x170 GetBaseForm, +0x04) 0x24
/// (a creature) whose +0x104 is 4. Completion: the base type and creature type checks.
/// # Errors
/// A CREA form missing its creature data.
pub fn reference_is_creature_type_4(world: &World, reference: Entity) -> Result<bool, GameError> {
    let Some(base) = world
        .get::<TesObjectRefr>(reference)
        .and_then(|refr| refr.base_form_1c)
    else {
        return Ok(false);
    };
    if crate::tes::forms_init::form_type(world, base) != 0x24 {
        return Ok(false);
    }
    world
        .get::<crate::tes::forms_actors::TesCreature>(base)
        .map(|creature| creature.data_104[0] == 4)
        .ok_or(GameError::unclassified(
            0x004D74F4,
            "a CREA base without its creature data",
        ))
}

// IDA: 004D9A70 TESObjectREFR_Constructor: TESForm_Constructor, +0x18 TESChildCell then REFR vtables, +0x3C = 0, 00422EE0 on +0x44, +0x04 = 0x31, 004D6BF0 (+0x38 = 1.0; +0x3C released unless g_saveLoadGame +0x18 has
// bit 2; 0046B090 for type 0x31 is a no-op case), then +0x1C..+0x34 and +0x40 = 0. Completion: 100% of the store order; the 004D6C13 release stops.
#[expect(
    clippy::field_reassign_with_default,
    reason = "004D9A70's stores stay one statement each, in original store order"
)]
pub fn construct_reference(world: &mut World) -> Result<Entity, GameError> {
    // 004D9A98 TESForm_Constructor (the DataHandler arm runs once g_dataHandler is published).
    let form = construct_tes_form(world)?;
    // 004D9A9D..004D9AAA +0x18 TESChildCell, +0x00 / +0x18 the TESObjectREFR vtables.
    let mut reference = TesObjectRefr::default();
    reference.child_cell_vtable_18 = TES_CHILD_CELL_VTABLE;
    world
        .get_mut::<OriginalVtable>(form)
        .expect("just spawned")
        .0 = TES_OBJECT_REFR_VTABLE;
    reference.child_cell_vtable_18 = TES_OBJECT_REFR_CHILD_CELL_VTABLE;
    // 004D9AB9 +0x3C = 0; 004D9AC8 00422EE0 on +0x44.
    reference.loaded_3d_3c = 0;
    reference.extra_data_44.entries.clear();
    reference.extra_data_44.presence_08 = [0; 12];
    reference.extra_data_44.original_vtable = EXTRA_DATA_LIST_VTABLE;
    // 004D9AD4 +0x04 = 0x31 (REFR).
    world
        .get_mut::<FormIdentity>(form)
        .expect("just spawned")
        .form_type_04 = FORM_TYPE_REFERENCE;
    // 004D9AD8 004D6BF0: +0x38 = 1.0; unless g_saveLoadGame +0x18 has bit 2 a nonzero +0x3C is released and cleared (null here); then j_TESForm_ResetComponents (0046B090: type 0x31 is in the no-op case group 13,
    // 49..52, 54, 61). A world with no g_saveLoadGame reads as flags 0.
    reference.scale_38 = 1.0;
    let save_load_flags_18 = world
        .get_resource::<TesGlobals>()
        .and_then(|globals| globals.save_load_game.as_ref())
        .map_or(0, |game| game.fields_14[1]);
    let keep_3d = save_load_flags_18 >> 2 & 1 != 0;
    if !keep_3d && reference.loaded_3d_3c != 0 {
        return Err(GameError::unclassified(
            0x004D6C13,
            "004D6C13: releasing a loaded 3D is not reached",
        ));
    }
    // 004D9ADD..004D9AF4 +0x1C..+0x34 and +0x40 = 0.
    reference.base_form_1c = None;
    reference.rotation_20 = [0.0; 3];
    reference.position_2c = [0.0; 3];
    reference.parent_cell_40 = None;
    world.entity_mut(form).insert(reference);
    Ok(form)
}

// IDA: 005E1C70 TESObjectREFR_IsFlyingCreature (the reference's actor base through 00519CE0 TESActorBase_IsFlyingCreature):
// a creature (type 0x24) whose +0x28 flags hold 0x20.
pub(crate) fn is_flying_creature(world: &World, actor: Entity) -> bool {
    world
        .get::<TesObjectRefr>(actor)
        .and_then(|reference| reference.base_form_1c)
        .filter(|&base| crate::tes::forms_init::form_type(world, base) == 0x24)
        .and_then(|base| world.get::<crate::tes::form_components::TesActorBaseData>(base))
        .is_some_and(|data| data.flags_04 & 0x20 != 0)
}

// IDA: vt+0x178(0) on an actor or a plain reference: Character 0065B5C0 returns at once with no controller; TESObjectREFR 004D66E0 hands vt+0x154's node to 00897A20, which returns on null.
fn refresh_node(world: &World, reference: Entity, where_: &'static str) -> Result<(), GameError> {
    if world.get::<Actor>(reference).is_some() {
        return no_controller(world, reference, where_);
    }
    if world
        .get::<TesObjectRefr>(reference)
        .expect("a reference")
        .loaded_3d_3c
        != 0
    {
        return Err(GameError::from_legacy_message(where_));
    }
    Ok(())
}

// IDA: vt+0x178(0) 0065B5C0: returns at once with no controller.
fn no_controller(world: &World, actor: Entity, where_: &'static str) -> Result<(), GameError> {
    if actor_character_controller(world, actor).is_some() {
        return Err(GameError::from_legacy_message(where_));
    }
    Ok(())
}

/// IDA: 004DD4B0 TESObjectREFR_MoveToCell(reference, cell, world) (cdecl), for an actor or a plain reference, which skips the mount, death, process and level arms and takes TESObjectREFR's vt+0x178 and vt+0x144. In exe order: an exterior cell is dropped; neither returns; the horse's process vt+0x474(1) is `ret 4` on Low and MiddleLow, so a mounted actor stops only past them; the old cell and world; a dead actor loses its 0x4B extra and flag 0x10000 (vt+0x9C(0)); then the interior or exterior arm and the tail. Stops that the C++ has at 004DD832 and 004DD707 aren't what the exe does. Completion: 100% with no controller and no 3D.
/// TODO: [ ] 004DD523 a mounted actor (+0xD4 is a raw word nothing sets), 004DD5E0 and 0065AB80 with a controller or 3D, 004DD7EE 00463A90.
pub fn move_to_cell(
    world: &mut World,
    actor: Entity,
    cell: Option<Entity>,
    space: Option<Entity>,
) -> Result<(), GameError> {
    let cell = cell.filter(|&cell| cell_is_interior(world, cell));
    if cell.is_none() && space.is_none() {
        return Ok(());
    }
    world.resource_mut::<AiProcessGlobals>().cell_moves += 1;
    let mut refresh = false;
    let mut changed = false;
    if world.get::<Character>(actor).is_some()
        && world
            .get::<Actor>(actor)
            .expect("an actor")
            .horse_or_rider_d4
            != 0
    {
        return Err(GameError::unclassified(
            0x004DD523,
            "004DD523: a mounted actor's horse",
        ));
    }
    let old_cell = parent_cell(world, actor);
    let old_world = reference_world(world, actor);
    let is_actor = world.get::<Actor>(actor).is_some();
    // vt+0x198(0): a plain reference's 004D7DD0 answers 0 past vt+0x190.
    if is_actor && actor_is_dead(world, actor, false) {
        world
            .get_mut::<TesObjectRefr>(actor)
            .expect("an actor")
            .extra_data_44
            .remove_by_type(0x4B);
        world
            .get_mut::<FormIdentity>(actor)
            .expect("a form")
            .flags_08 &= !0x1_0000;
        set_change_flags(world, actor, 1)?;
    }
    let mut new_cell = cell;
    if let Some(cell) = cell {
        if old_cell == Some(cell) {
            refresh_node(
                world,
                actor,
                "004DD5E0: an actor's controller staying in its cell",
            )?;
        } else {
            changed = true;
            if let Some(old_world) = old_world
                && reference_is_persistent(world, actor)?
            {
                world_remove_persistent(world, old_world, actor)?;
            }
            insert_reference_runtime(world, cell, actor)?;
        }
        if cell_attached(world, Some(cell), true) {
            refresh = true;
        }
    } else {
        let space = space.expect("tested above");
        if old_cell.is_some_and(|cell| cell_is_interior(world, cell)) {
            changed = true;
        }
        if reference_is_persistent(world, actor)? && old_world != Some(space) {
            if let Some(old_world) = old_world {
                world_remove_persistent(world, old_world, actor)?;
            }
            world_insert_persistent(world, space, actor)?;
        }
        let position = reference_position(world, actor)?;
        let x = x87::fistp(position[0]);
        let y = x87::fistp(position[1]);
        // 004EFE80 TESWorldSpace_LookupCell: out of range warns (004EFED4) and answers null. The C++ stops.
        let (found, warned) =
            crate::tes::forms_world::world_lookup_cell(world, space, x >> 12, y >> 12);
        if warned {
            world
                .resource_mut::<AiProcessGlobals>()
                .warnings
                .push(crate::tes::data_handler_world::invalid_coordinate_warning());
        }
        new_cell = found;
        if let Some(new) = new_cell {
            insert_reference_runtime(world, new, actor)?;
            if cell_attached(world, Some(new), true) {
                if old_world == Some(space) {
                    let old_attached = cell_attached(world, old_cell, true);
                    if changed && old_attached {
                        refresh_node(world, actor, "004DD732: vt+0x178 with a controller")?;
                    } else if !old_attached
                        && world
                            .get::<TesObjectRefr>(actor)
                            .expect("an actor")
                            .loaded_3d_3c
                            != 0
                    {
                        refresh = true;
                    }
                } else {
                    refresh = true;
                    changed = true;
                    // 004DD6FD process vt+0x20, Low 00643640.
                    if let Some(process) = crate::tes::actor::process_of(world, actor) {
                        crate::tes::actor_process::process_update_stamps(world, process);
                    }
                }
            }
        } else {
            if let Some(old_cell) = old_cell {
                cell_remove_reference(world, old_cell, actor)?;
            }
            if is_actor && !player_is_sleeping(world)? && save_load_flags(world)? & 0x20 == 0 {
                set_3d_null(world, actor)?;
            }
        }
    }
    // 004DD7B0, vt+0x188.
    if is_actor && !refresh {
        actor_process_manager_refresh(world, actor)?;
        if new_cell.is_none()
            && desired_level_is_3(world, actor)?
            && !reference_is_persistent(world, actor)?
        {
            return Err(GameError::unclassified(
                0x004DD7EE,
                "004DD7EE: 00463A90 unloading a moved actor",
            ));
        }
    }
    if changed && !is_actor {
        // vt+0x144 004E48B0: the node's 0088CD50, and the parent cell's Havok world's 005369B0, 00536D30 and 005374F0.
        let loaded = world
            .get::<TesObjectRefr>(actor)
            .expect("a reference")
            .loaded_3d_3c
            != 0;
        if loaded
            || parent_cell(world, actor)
                .is_some_and(|cell| crate::tes::tes_grid::cell_has_havok_world(world, cell))
        {
            return Err(GameError::unclassified(
                0x004E48F9,
                "004E48F9: a moved reference's node or Havok world",
            ));
        }
    } else if changed {
        // vt+0x144 0065AB80: vt+0x1C8 0065A380 with a controller, and the +0x3C Havok arm.
        no_controller(world, actor, "0065AB80: the controller of a moved actor")?;
        if world
            .get::<TesObjectRefr>(actor)
            .expect("an actor")
            .loaded_3d_3c
            != 0
        {
            return Err(GameError::unclassified(
                0x0065AB80,
                "0065AB80: the 3D of a moved actor",
            ));
        }
    }
    if refresh {
        refresh_node(world, actor, "004DD832: vt+0x178 after a refresh")?;
    }
    Ok(())
}

// IDA: 0041E7F0 the 0x29 value, -1 with none.
fn extra_rank(list: &ExtraDataList) -> i32 {
    match list.get(EXTRA_RANK).map(|entry| &entry.data) {
        Some(ExtraData::Dword(value)) => *value as i32,
        _ => -1,
    }
}

// IDA: 004DB6B0 TESObjectREFR_GetEffectiveOwner: its own; a non-actor without one takes the linked door's, then, unless its base is of type 0x20, 0x18 or 0x12 (004DB738 `cmp byte [eax+4],12h`, where the C++ has 0x15), the cell's.
pub(crate) fn effective_owner(
    world: &World,
    reference: Entity,
) -> Result<Option<Entity>, GameError> {
    let owner = extra_form(list_of(world, reference), EXTRA_OWNER)?;
    if world.get::<Actor>(reference).is_some() || owner.is_some() {
        return Ok(owner);
    }
    if let Some(door) = teleport_door(world, reference)
        && let Some(linked) = extra_form(list_of(world, door), EXTRA_OWNER)?
    {
        return Ok(Some(linked));
    }
    let kind = world
        .get::<TesObjectRefr>(reference)
        .expect("a reference")
        .base_form_1c
        .map_or(0, |base| form_type(world, base));
    if !matches!(kind, 0x20 | 0x18 | 0x12)
        && let Some(cell) = parent_cell(world, reference)
    {
        return extra_form(cell_list(world, cell), EXTRA_OWNER);
    }
    Ok(None)
}

// IDA: 004DB830 the required rank: its own, the linked door's, the cell's; -1 is 0.
fn required_rank(world: &World, reference: Entity) -> i32 {
    let rank = extra_rank(list_of(world, reference));
    if rank != -1 {
        return rank;
    }
    if let Some(door) = teleport_door(world, reference) {
        let rank = extra_rank(list_of(world, door));
        if rank != -1 {
            return rank;
        }
    }
    let Some(cell) = parent_cell(world, reference) else {
        return 0;
    };
    let rank = extra_rank(cell_list(world, cell));
    if rank == -1 { 0 } else { rank }
}

// IDA: 004DB7D0 the ownership global: its own, the linked door's, the cell's.
fn ownership_global(world: &World, reference: Entity) -> Result<Option<Entity>, GameError> {
    if let Some(global) = extra_form(list_of(world, reference), EXTRA_GLOBAL)? {
        return Ok(Some(global));
    }
    if let Some(door) = teleport_door(world, reference)
        && let Some(global) = extra_form(list_of(world, door), EXTRA_GLOBAL)?
    {
        return Ok(Some(global));
    }
    if let Some(cell) = parent_cell(world, reference) {
        return extra_form(cell_list(world, cell), EXTRA_GLOBAL);
    }
    Ok(None)
}

// IDA: 004DE770 TESObjectREFR_IsOwnedBy(actor, factionRanks). vt+0x128 00447550's +0xD0 is a raw word nothing sets, so it stops where the exe uses it as the own form.
pub(crate) fn reference_owned_by(
    world: &World,
    reference: Entity,
    actor: Entity,
    faction_ok: bool,
) -> Result<bool, GameError> {
    let Some(owner) = effective_owner(world, reference)? else {
        return Ok(false);
    };
    let rank = required_rank(world, reference);
    let global = ownership_global(world, reference)?;
    if world
        .get::<Actor>(actor)
        .expect("an actor")
        .leveled_creature_template_d0
        != 0
    {
        return Err(GameError::unclassified(
            0x004DE7C3,
            "004DE7C3: a leveled template owner test",
        ));
    }
    let own = world
        .get::<TesObjectRefr>(actor)
        .expect("an actor")
        .base_form_1c;
    let mut owned = true;
    if Some(owner) != own {
        let is_faction = form_type(world, owner) == 6;
        if !is_faction || !faction_ok {
            let value = global
                .and_then(|global| world.get::<TESGlobal>(global))
                .map(|global| f32::from_bits(global.value_bits_24));
            if value.is_none_or(|value| value == 0.0) {
                owned = false;
            }
        }
        if is_faction
            && let Some(base) = own.filter(|&own| matches!(form_type(world, own), 0x23 | 0x24))
            && base_faction_rank(world, base, owner, Some(actor) == the_player(world))? < rank
        {
            return Ok(false);
        }
    }
    Ok(owned)
}

// IDA: 004DBEA0 Unlock, 004DBE40 Lock: 004D77D0 follows the linked doors to the first reference with a lock; a byte `and`, `or` on its +0x08 bit 0; 004D9070 marks the reference's own lock, or else the linked door's, one level (vt+0x40(0x40)).
pub(crate) fn set_reference_locked(
    world: &mut World,
    reference: Entity,
    locked: bool,
) -> Result<(), GameError> {
    let mut holder = Some(reference);
    while let Some(form) = holder {
        if list_of(world, form).get(EXTRA_LOCK).is_some() {
            break;
        }
        holder = teleport_door(world, form);
    }
    let Some(holder) = holder else {
        return Ok(());
    };
    if let Some(entry) = world
        .get_mut::<TesObjectRefr>(holder)
        .expect("a reference")
        .extra_data_44
        .get_mut(EXTRA_LOCK)
        && let ExtraData::Lock(block) = &mut entry.data
    {
        if locked {
            block[8] |= 1;
        } else {
            block[8] &= !1;
        }
    }
    let mut globals = world.resource_mut::<AiProcessGlobals>();
    if locked {
        globals.doors_locked += 1;
    } else {
        globals.doors_unlocked += 1;
    }
    if list_of(world, reference).get(EXTRA_LOCK).is_some() {
        return set_change_flags(world, reference, 0x40);
    }
    if let Some(door) = teleport_door(world, reference)
        && list_of(world, door).get(EXTRA_LOCK).is_some()
    {
        return set_change_flags(world, door, 0x40);
    }
    Ok(())
}

/// IDA: 004D7740 TESObjectREFR_GetLockData: the reference's lock; without one the linked door's. Completion: 100%.
#[must_use]
pub fn reference_lock(world: &World, reference: Entity) -> Option<&[u8; 12]> {
    if let Some(block) = lock_block(world, reference) {
        return Some(block);
    }
    lock_block(world, teleport_door(world, reference)?)
}

/// IDA: 004D7E90 TESObjectREFR_GetDistance(other, includeDisabled): FLT_MAX for a disabled (without the flag) or deleted other, different interiors, or different or null exterior worlds; otherwise 004D7E30 to the other's vt+0x174. Completion: 100%.
pub fn reference_distance(
    world: &World,
    this: Entity,
    other: Entity,
    include_disabled: bool,
) -> Result<f32, GameError> {
    let flags = form_flags(world, other);
    if (flags & 0x800 != 0 && !include_disabled) || flags & 0x20 != 0 {
        return Ok(FLT_MAX);
    }
    let other_cell = parent_cell(world, other);
    let this_cell = parent_cell(world, this);
    if let (Some(other_cell), Some(this_cell)) = (other_cell, this_cell)
        && (cell_is_interior(world, other_cell) || cell_is_interior(world, this_cell))
    {
        if this_cell != other_cell {
            return Ok(FLT_MAX);
        }
    } else {
        let other_world = reference_world(world, other);
        let this_world = reference_world(world, this);
        if this_world != other_world || this_world.is_none() {
            return Ok(FLT_MAX);
        }
    }
    let position = world.get::<TesObjectRefr>(this).expect("a ref").position_2c;
    Ok(distance_to(position, reference_position(world, other)?))
}

// IDA: 004D78E0: an XMarker or XMarkerHeading reference's 0x4D extra's form.
pub(crate) fn marker_target(world: &World, form: Entity) -> Result<Option<Entity>, GameError> {
    let Some(reference) = world.get::<TesObjectRefr>(form) else {
        return Ok(None);
    };
    let statics = world.resource::<DefaultFormStatics>();
    if reference.base_form_1c.is_none()
        || (reference.base_form_1c != statics.x_marker_b35eac
            && reference.base_form_1c != statics.x_marker_heading_b35eb0)
    {
        return Ok(None);
    }
    let Some(entry) = reference.extra_data_44.get(0x4D) else {
        return Ok(None);
    };
    match (entry.resolved_0c, &entry.data) {
        (Some(found), _) => Ok(found),
        (None, ExtraData::Dword(0)) => Ok(None),
        _ => Err(GameError::unclassified(
            0x00420D20,
            "00420D20: an unresolved XTarget",
        )),
    }
}

/// IDA: 00468670 the actor's package list: 005E02E0(1), or 005E02E0(0) when that's empty; null with no base. As a copy of the slots. Completion: 100%.
pub fn actor_package_list(world: &World, actor: Entity) -> Result<Option<Vec<FormRef>>, GameError> {
    let Some(base) = actor_base_form(world, actor, true)? else {
        return Ok(None);
    };
    let mut list = world.get::<TesAiForm>(base).map(|ai| &ai.packages_10);
    if list.is_some_and(Vec::is_empty) {
        let Some(plain) = actor_base_form(world, actor, false)? else {
            return Ok(None);
        };
        list = world.get::<TesAiForm>(plain).map(|ai| &ai.packages_10);
    }
    Ok(list.cloned())
}

// IDA: 004D8FB0 TESObjectREFR_GetEncumbrance: with a base container (004D6D40), the changes' weight (00487E40, which stores its +0x08 cache itself), or with no changes block the base's (004698B0); 0.0 without one; a non-player actor is capped by 005E0D20: `v = f32(fActorStrengthEncumbranceMult * vt+0x288(0))`, `v < 0` is 0, a NaN stays NaN (00547EE8; the C++ gives 0) and then keeps the weight (`fcomp q; test ah,5; jp`). Default of the setting: 5.0 (009EB570).
pub(crate) fn reference_encumbrance(
    world: &mut World,
    reference: Entity,
) -> Result<f32, GameError> {
    let mut weight = 0.0_f32;
    if let Some(base) = reference_container(world, reference) {
        let list = &world
            .get::<TesObjectRefr>(reference)
            .expect("a reference")
            .extra_data_44;
        match list.get(EXTRA_CONTAINER_CHANGES).map(|entry| &entry.data) {
            Some(ExtraData::ContainerChanges(_)) => {
                // 004D8FD3: 00487E40 on the block, which stores its own +0x08; the block out of the list for the call.
                let mut changes = ensure_container_changes(world, reference)?;
                let total = container_changes_total_weight(world, &mut changes);
                put_container_changes(world, reference, changes);
                weight = total?;
            }
            _ => {
                if let Some(container) = world.get::<TesContainer>(base) {
                    weight = container_weight(world, container)?;
                }
            }
        }
    }
    if world.get::<Actor>(reference).is_some() && Some(reference) != the_player(world) {
        let setting = world
            .get_resource::<GameSettingCollection>()
            .and_then(|settings| settings.raw_at(0x00B3_7570))
            .map_or(5.0, f32::from_bits);
        let strength = actor_get_actor_value_float(world, reference, 0)?;
        let value = setting * strength;
        let limit = if value < 0.0 { 0.0 } else { value };
        if limit < weight {
            return Ok(limit);
        }
    }
    Ok(weight)
}

// IDA: 004D7A20 (actor, world, cell, position, rotZ) -> 0041F4C0, as 00648EC0 and 00649772 push them: +0x28, vt+0x174, 006ECC80, 004D6670.
pub(crate) fn record_start_location(world: &mut World, actor: Entity) -> Result<(), GameError> {
    let position = reference_position(world, actor)?;
    let rotation = world
        .get::<TesObjectRefr>(actor)
        .expect("an actor")
        .rotation_20[2];
    let cell = parent_cell(world, actor);
    let space = reference_world(world, actor);
    set_package_start_location(world, actor, space, cell, position, rotation);
    Ok(())
}

// IDA: 004D89D0 (float): +0x20, then vt+0x40(4).
pub(crate) fn set_rotation_x(
    world: &mut World,
    reference: Entity,
    value: f32,
) -> Result<(), GameError> {
    world
        .get_mut::<TesObjectRefr>(reference)
        .expect("a reference")
        .rotation_20[0] = value;
    set_change_flags(world, reference, 4)
}

// IDA: 004DD260 TESObjectREFR_Activate(activator, 0, 0, 1) for an NPC activator with a LowProcess: 00B35F04 = 0 (only read by the script arm); flag 0x2000 returns; a mounted activator skips base type 0x12; an 0x13 extra without bit 0 runs the activation scripts, a host stop; types 0x1C, 0x1E return; base vt+0xCC: TESFurniture 004AE8C0's player cast fails, 0; others stop. The return isn't read.
// TODO: [ ] 004DD3AC the activation script; base vt+0xCC of doors 004B8A60, activators 004B3CA0 and so on.
pub(crate) fn activate_reference(
    world: &World,
    reference: Entity,
    activator: Entity,
) -> Result<(), GameError> {
    if form_flags(world, reference) & 0x2000 != 0 {
        return Ok(());
    }
    let data = world.get::<TesObjectRefr>(reference).expect("a reference");
    let base = data.base_form_1c.ok_or(GameError::unclassified(
        0x004DD3E7,
        "004DD3E7: a base-less activation target",
    ))?;
    let kind = form_type(world, base);
    if world
        .get::<Actor>(activator)
        .expect("an actor")
        .horse_or_rider_d4
        != 0
        && kind == 0x12
    {
        return Ok(());
    }
    if Some(activator) == the_player(world) {
        return Err(GameError::unclassified(
            0x004DD38F,
            "004DD38F: an activation by the player",
        ));
    }
    if let Some(entry) = data.extra_data_44.get(0x13)
        && matches!(&entry.data, ExtraData::Action { flags_0c, .. } if flags_0c & 1 == 0)
    {
        return Err(GameError::unclassified(
            0x004DD3AC,
            "004DD3AC: the activation script run",
        ));
    }
    match kind {
        0x1C | 0x1E | 0x20 => Ok(()),
        _ => Err(GameError::unclassified(
            0x004DD469,
            "004DD469: base vt+0xCC Activate",
        )),
    }
}

// IDA: 004F9EC0 (actor, +0x44): a 0x12 extra with an event list whose +0x08 is null runs 00516970 and 00517950. The list comes out of the extra for the run. Completion: 100%.
pub(crate) fn run_pending_script(world: &mut World, actor: Entity) -> Result<(), GameError> {
    let extras = &mut world
        .get_mut::<TesObjectRefr>(actor)
        .expect("an actor")
        .into_inner()
        .extra_data_44;
    let Some(entry) = extras.get_mut(0x12) else {
        return Ok(());
    };
    let ExtraData::Script {
        script_0c,
        events_10,
    } = &mut entry.data
    else {
        return Ok(());
    };
    let script = *script_0c;
    if events_10
        .as_ref()
        .is_none_or(|events| events.events_08.is_some())
    {
        return Ok(());
    }
    let mut events = events_10.take().expect("checked above");
    world
        .resource_mut::<AiProcessGlobals>()
        .script_event_runs_not_executed += 1;
    let result = crate::tes::script_vm::run_blocks_only(
        world,
        script,
        Some(actor),
        &mut events,
        Some(actor),
    );
    if let Some(entry) = world
        .get_mut::<TesObjectRefr>(actor)
        .expect("an actor")
        .into_inner()
        .extra_data_44
        .get_mut(0x12)
        && let ExtraData::Script { events_10, .. } = &mut entry.data
    {
        *events_10 = Some(events);
    }
    result
}

// IDA: 005E4420 / 004875C0: the gold (form ID 0xF) in the inventory; any stops, none is 0.
pub(crate) fn reference_gold(world: &World, player: Entity) -> Result<i32, GameError> {
    if crate::tes::menus::magic_menu::inventory_forms(world, player)
        .into_iter()
        .any(|form| form_id(world, form) == 0xF)
    {
        return Err(GameError::unclassified(
            0x004875C0,
            "004875C0: gold in the inventory is carried",
        ));
    }
    Ok(0)
}

/// IDA: 005E3320 TESObjectREFR_GetNPCClassOrNull: an NPC reference's base +0x104 class. Completion: 100%.
pub(crate) fn npc_class(cx: &InterfaceCx) -> Result<Option<Entity>, GameError> {
    let player = player(cx)?;
    let Some(base) = cx
        .world
        .get::<crate::tes::actor::TesObjectRefr>(player)
        .and_then(|refr| refr.base_form_1c)
    else {
        return Ok(None);
    };
    Ok(cx
        .world
        .get::<crate::tes::forms_actors::TesNpc>(base)
        .and_then(|npc| match npc.class_104 {
            FormRef::Resolved(class) => class,
            FormRef::Raw(_) => None,
        }))
}

// IDA: N/A a FormRef's form.
fn resolved(form: FormRef) -> Option<Entity> {
    match form {
        FormRef::Resolved(form) => form,
        FormRef::Raw(_) => None,
    }
}

/// IDA: 004D9310(reference, 1): a base, OSSoundGlobals +0x24 and the reference's flag 0x800 clear. An activator (0x12) its +0x54 sound, a door (0x18, the reference without flag 0x2000) its +0x60 loop, a light (0x1A, likewise) its +0x8C sound: each first stops and drops a held 0x5B handle (none on a first attach: a stop), then 006AE4B0 at the position (vt+0x174) with a new ID and the 3D flag (a door's 0), and a handle goes to 0x5B (00423B10). A sound (0x0A): a playing 0x5B handle is stopped, then 006AE4B0 with the reference's ID and the base, whose handle 006B73E0 releases. Completion: 100% of a first attach.
pub(crate) fn reference_start_sound(
    world: &mut World,
    reference: Entity,
    base: Entity,
    services: &mut GameSoundServices<'_>,
) -> Result<(), GameError> {
    let refr = world.get::<TesObjectRefr>(reference).expect("a reference");
    if refr.extra_data_44.get(0x5B).is_some() {
        return Err(GameError::unclassified(
            0x004D9348,
            "004D9348: a reference holding a sound handle",
        ));
    }
    let position = refr.position_2c;
    let flags = world
        .get::<FormIdentity>(reference)
        .expect("a form")
        .flags_08;
    if flags & 0x800 != 0 {
        return Ok(());
    }
    let kind = form_type(world, base);
    let (sound, id, new_id, three_d) = match kind {
        0x12 => (
            world
                .get::<TesActivator>(base)
                .and_then(|a| resolved(a.sound_54)),
            0,
            true,
            true,
        ),
        0x18 if flags & 0x2000 == 0 => (
            world.get::<TesDoor>(base).and_then(|d| d.loop_sound_60),
            0,
            true,
            false,
        ),
        0x1A if flags & 0x2000 == 0 => (
            world
                .get::<TesLight>(base)
                .and_then(|l| resolved(l.sound_8c)),
            0,
            true,
            true,
        ),
        0x0A => (
            Some(base),
            crate::tes::forms_init::form_id(world, reference),
            false,
            true,
        ),
        _ => return Ok(()),
    };
    let handle = create_sound(world, position, sound, id, new_id, three_d, services)?;
    if handle.is_some() && kind != 0x0A {
        return Err(GameError::unclassified(
            0x004D93D4,
            "004D93D4: 00423B10, a sound handle on the reference",
        ));
    }
    Ok(())
}

// IDA: 004D7630 -> 0041E6B0: the 0x32 extra's +0x0C block, null without one: the seven XTEL dwords.
pub(crate) fn teleport_block(world: &World, reference: Entity) -> Option<[u32; 7]> {
    let entry = world
        .get::<TesObjectRefr>(reference)?
        .extra_data_44
        .get(EXTRA_TELEPORT)?;
    match &entry.data {
        ExtraData::Teleport(block) => Some(**block),
        _ => None,
    }
}

// IDA: 004DB260(type, 1): 0x31 004D9A70, 0x32 0060E540(1), 0x33 00625100(1); these arms are the only types the 0044E001 arm brings. Completion: 100%.
pub(crate) fn create_reference(world: &mut World, form_type: u32) -> Result<Entity, GameError> {
    match form_type as u8 {
        FORM_TYPE_REFERENCE => construct_reference(world),
        FORM_TYPE_CHARACTER => construct_loaded_character(world),
        FORM_TYPE_CREATURE => construct_loaded_creature(world),
        _ => Err(GameError::unclassified(
            0x004DB260,
            "004DB260: a null form faults in 00447050",
        )),
    }
}
