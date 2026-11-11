//! Phase 12a: the 004067C7 call of 00406360, DataHandler_LoadSelectedFiles 0044F3D0(0, 0), whose result is ignored: the loader-thread TLS byte, the TES reset (00442630, 0043F220), the Loaded-file validation with the version gate 004504B0, the compiled-file array with mod indices (0044FB20), the master-size
//! check (0044FDE0), +0x8C0 = 0xFF000800, the record totals (00B33A90), the per-file record loop 0044F0C0 (groups 00448C60 and 00451580, TOFT, the save-load arm, g_formsByEditorID, the 00B33A94 counter, "Loading Files" percent), and the DataHandler_ConstructObject 0044DCF0 frame with its TES4, GMST and SKIL arms.
//! Its list and default arms are here too. Everything else reaches a class through `form_arms`; 0044AC20 is `default_forms` and the post-load tail `post_load`.
//!
//! The loop is `LoadSelectedFiles::step`, a cursor stopping at the original's record boundary, so a host can draw between steps. Game-visible order doesn't change. Tests run it straight through. Evidence: evidence/phase12-window-2026-10-09/ and objdump of each address named here; P12-RUN-001 and P12-RUN-003.

use std::collections::BTreeMap;

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;
use oblivion_core::x87;

use super::data_handler::{
    DataHandler, DataHandlerGlobals, DataHandlerLists, EditorIdMap, FormsById, TES_SKILL_VTABLE,
};
use super::tes_file::{
    FORM_TYPE_TABLE, RecordReader, TesFileId, TesFileServices, TesFileStore, close_tes_file,
    master_at, open_tes_file, read_tes_file_header, reserved_form_id, resolve_tes_file_masters,
    skip_group, skip_record,
};
use crate::tes::actor::{FORM_FLAG_PLAYER_INIT, MobileObject, TesObjectRefr};
use crate::tes::actor_process::actor_process_manager_remove;
use crate::tes::base_extra_list::EXTRA_EDITOR_ID;
use crate::tes::data_handler::FreedCellKey;
use crate::tes::form::{
    FORM_FLAG_PARTIAL, FormIdentity, load_form_from_file, set_form_id_release,
    update_mod_references_from,
};
use crate::tes::form_arms::{
    AbsentArm, DataLoadHost, FormClass, FormLoadCx, default_editor_id, editor_id_of, form_class,
    no_init_item, reset_18_components,
};
use crate::tes::form_components::{TesFullName, TesObjectLinks, append_tes_object};
use crate::tes::form_type_map::{ConstructArm, form_type_row};
use crate::tes::forms_ai::FORM_TYPE_IDLE;
use crate::tes::forms_dialogue::{TesTopicInfo, construct_topic_info, construct_topic_of_type};
use crate::tes::forms_magic::{construct_effect_setting_with, find_setting_by_name};
use crate::tes::forms_misc::{TesSkill, init_skill_form, load_skill_form, reset_skill_14};
use crate::tes::forms_references::{
    create_reference, form_flags, reference_cell, reference_is_persistent, save_load_active,
    set_parent_cell,
};
use crate::tes::forms_script::construct_script_in_place;
use crate::tes::forms_world::{
    CellInsertRest, FORM_FLAG_PERSISTENT, TesObjectCell, TesPathGrid, TesRoad, TesWorldSpace,
    add_interior_cell, cell_grid_x, cell_grid_y, cell_insert_reference_head, cell_is_interior,
    cell_remove_reference, cell_set_toft, construct_cell, construct_land, construct_path_grid,
    construct_road, construct_world, current_or_first_world, land_set_cell, world_add_cell,
    world_apply_toft_delta,
};
use crate::tes::game_settings::GameSettingCollection;
use crate::tes::tes::{
    GameThreadLocals, TesGlobals, tes_reset_exterior_coordinates, tes_unload_cells,
};

// IDA: the chunk fourccs the 12a arms compare.
pub(crate) const EDID: u32 = 0x4449_4445;
const DELE: u32 = 0x454C_4544;
// IDA: the record flag 0044DE05 tests, which skips the reset pair.
const RECORD_FLAG_NO_RESET: u32 = 0x20;
// IDA: g_saveLoadGame +0x18 bit 12.
const SAVE_LOAD_FLAG: u32 = 0x1000;
// IDA: the form types of 0044F0C0's loop.
const FORM_TYPE_GROUP: u32 = 0x02;
const FORM_TYPE_TOFT: u32 = 0x44;
/// IDA: the form type 0052E85D tests.
pub const FORM_TYPE_SKILL: u32 = 0x0B;

/// IDA: the Phase 12 loader globals.
#[derive(Resource, Debug, Default)]
pub struct DataLoadGlobals {
    /// 00B33A90: the sum of the compiled files' HEDR record counts.
    pub record_total_b33a90: u32,
    /// 00B33A94: the records 0044F0C0 visited.
    pub record_count_b33a94: u32,
    /// 00B33A9C: the current TESObjectCELL (CELL arm, TOFT).
    pub current_cell_b33a9c: Option<Entity>,
    /// 00B33AA0: the current TESWorldSpace (WRLD, CELL and ROAD arms).
    pub current_world_b33aa0: Option<Entity>,
    /// 00B33AA4: the current TESTopicInfo (INFO arm, TOFT).
    pub current_info_b33aa4: Option<Entity>,
    /// 00B33AD8 (the save-load arm).
    pub save_load_count_b33ad8: u32,
    /// 00B05DC0, which skips 00468590's packages: clear at startup.
    pub skip_ai_packages_b05dc0: u8,
    /// 00B14E98 bEssentialTakeNoDamage:GamePlay, which 005E6680 tests. The image holds 1.
    pub essential_take_no_damage_b14e98: u8,
    /// 00B10CA0, which 00529898 tests: clear at startup.
    pub quest_flag_b10ca0: u8,
    /// 00B06B18 g_disableWarningMessages (bDisableWarning:MESSAGES, 1 in P12-RUN-001). Set by the caller from the setting.
    pub disable_warning_messages_b06b18: u8,
    /// IDA: N/A the last of 0044F0C0's "Loading Files %d%% (%s)" buffers, which the original never reads.
    pub last_progress_text: String,
    /// IDA: N/A the calls of 0057B950.
    pub progress_reports: u32,
    /// IDA: N/A each 0044F0C0's result, in call order.
    pub file_results: Vec<u32>,
    /// IDA: N/A 00B33A94 at each 0044F0C0 entry, in call order (P12-RUN-001's FILE_ENTRY records).
    pub file_entry_counts: Vec<u32>,
    /// IDA: N/A `AbsentArm::CountAndSkip`'s counts per form type.
    pub pending_arms: BTreeMap<u32, u32>,
}

/// IDA: N/A what the load needs from outside the world.
pub struct DataLoadServices<'a> {
    /// The directory "Data\\" resolves against, and that 0044FDE0's bare names resolve against: the original's process directory.
    pub working_directory: &'a std::path::Path,
    /// 004A7A60 DebugMessage_Warning (D4: the host log; the bool is g_disableWarningMessages at the call).
    pub warning: &'a mut dyn FnMut(String, bool),
    pub host: &'a mut dyn DataLoadHost,
}

// IDA: N/A the world's parts a file call needs, out of their resources for the call: the TESFile store, so a LoadForm can hold a file and the World together (`DataHandler` stays in the world with an empty store), the 0046AF70 cache and the warning flag. Everything goes back on every exit.
pub(crate) fn with_files<R>(
    world: &mut World,
    services: &mut DataLoadServices<'_>,
    call: impl FnOnce(
        &mut World,
        &mut TesFileStore,
        &mut TesFileServices<'_>,
        &mut dyn DataLoadHost,
    ) -> Result<R, GameError>,
) -> Result<R, GameError> {
    let DataLoadServices {
        working_directory,
        warning,
        host,
    } = services;
    let mut data = world
        .get_resource_mut::<DataHandler>()
        .ok_or(GameError::unclassified(
            0x0044F3D0,
            "0044F3D0 on a null DataHandler",
        ))?;
    let close_suppressed = data.bytes_cd0[4] == 0;
    let mut files = std::mem::take(&mut data.file_store);
    let mut cache = world.resource::<DataHandlerGlobals>().form_type_cache;
    let flag = world
        .resource::<DataLoadGlobals>()
        .disable_warning_messages_b06b18;
    let mut file_services = TesFileServices {
        working_directory,
        warning: &mut **warning,
        disable_warning_messages_b06b18: flag,
        close_suppressed,
        form_type_cache: &mut cache,
    };
    let result = call(world, &mut files, &mut file_services, &mut **host);
    let flag = file_services.disable_warning_messages_b06b18;
    world.resource_mut::<DataHandler>().file_store = files;
    world.resource_mut::<DataHandlerGlobals>().form_type_cache = cache;
    world
        .resource_mut::<DataLoadGlobals>()
        .disable_warning_messages_b06b18 = flag;
    result
}

/// IDA: 00451210 TESFile_InitializeFormFromCurrentRecord(file, form): the record's form type into +0x04 (0 when no header can be read); +0x08 = the record flags, | 0x4000 when the form had it; 0046C300(record ID, 1); 0046B6C0(file). Every LoadForm starts with it. Completion: 100%.
pub fn initialize_form_from_current_record(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<(), GameError> {
    let form_type = cx.file.record_form_type()?;
    let record = cx.file.file()?.record_23c;
    let mut identity = world.get_mut::<FormIdentity>(form).expect("a form");
    identity.form_type_04 = form_type as u8;
    identity.flags_08 = if identity.flags_08 & FORM_FLAG_PARTIAL != 0 {
        record.flags_244 | FORM_FLAG_PARTIAL
    } else {
        record.flags_244
    };
    set_form_id_release(world, form, record.form_id_248, true)?;
    update_mod_references_from(world, form, cx.file.file()?)
}

/// IDA: 00447DF0 DataHandler_CreateFormByType(type) (__cdecl), called with g_disableWarningMessages as its caller leaves it. A type outside the factory's jump table (`form_type_row`) warns "TESDataHandler trying to create TESForm for unknown type '%s'." (the table's name below 0x45, else `Default`) and returns null. FormHeap failing returns null too, and isn't modelled.
/// A factory type with no class yet is a host stop here: the arms test for one first. Completion: 100% of the arms.
pub fn create_form_by_type(
    world: &mut World,
    form_type: u32,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<Option<Entity>, GameError> {
    if form_type > 0xFF || form_type_row(form_type as u8).is_none() {
        // 00448B26..00448B42.
        let name = FORM_TYPE_TABLE
            .get(form_type as usize)
            .map_or("", |(_, name)| *name);
        cx.file.services.warn(format!(
            "TESDataHandler trying to create TESForm for unknown type '{name}'."
        ));
        return Ok(None);
    }
    let Some(class) = form_class(form_type) else {
        return Err(pending_arm(form_type));
    };
    (class.construct)(world).map(Some)
}

// IDA: N/A "Phase-12x pending" for a type, naming the record. The text lives for the process: a host stop ends the load.
fn pending_arm(form_type: u32) -> GameError {
    let name = FORM_TYPE_TABLE
        .get(form_type as usize)
        .map_or("", |(_, name)| *name);
    GameError::stop(
        0x0044DE2E,
        Box::leak(format!("Phase-12 pending: 0044DE2E arm of form type {name}").into_boxed_str()),
    )
}

/// IDA: 0044D950 DataHandler_AddFormToList(form) (__thiscall on the DataHandler), with bytes 0044DCA8 over 0044DC44 read from the exe: a form type with a list prepends there (00446CB0) and returns 1. 0x31 is its REFR arm: the record loop never brings one, since REFR has its own arm in 0044DCF0; a later caller's arm needs 0044A270's exterior cell, so it stays a host stop. Any other form that casts to TESObject appends to +0x00 (004B2B80) and returns 1; anything else warns "Unknown form type '%s' encountered in
/// AddFormToDataHandler." and returns 0. Completion: 100% of the arms but 0x31.
/// TODO: [ ] 0044DB24 the REFR arm, with 0043F270, 0044A270 and the cell grid (Phase 14).
pub fn add_form_to_list(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<bool, GameError> {
    let form_type = world
        .get::<FormIdentity>(form)
        .expect("a form")
        .form_type_04;
    if form_type == 0x31 {
        return Err(GameError::stop(
            0x0044DB24,
            "Phase-14 pending: 0044DB24 the REFR arm of 0044D950",
        ));
    }
    if let Some(list) = world
        .resource_mut::<DataHandlerLists>()
        .list_for_type(form_type)
    {
        list.prepend_non_null(form);
        return Ok(true);
    }
    if world.get::<TesObjectLinks>(form).is_some() {
        append_tes_object(world, form);
        return Ok(true);
    }
    // 0044DC13..0044DC34.
    let name = FORM_TYPE_TABLE
        .get(usize::from(form_type))
        .map_or("", |(_, name)| *name);
    cx.file.services.warn(format!(
        "Unknown form type '{name}' encountered in AddFormToDataHandler."
    ));
    Ok(false)
}

// IDA: 0044E578 the list arm: an existing form reloads through 00447050 (0044E291) and returns 1. Otherwise 00447DF0, 00447050 and 0044D950, returning 1 whatever those say; a null form from 00447DF0 faults in 00447050, a host stop. Completion: 100%.
fn list_arm(
    world: &mut World,
    cx: &mut FormLoadCx<'_, '_>,
    form_type: u32,
    existing: Option<Entity>,
) -> Result<u32, GameError> {
    if let Some(existing) = existing {
        load_form_from_file(world, existing, cx)?;
        return Ok(1);
    }
    let form = create_form_by_type(world, form_type, cx)?.ok_or(GameError::unclassified(
        0x0044E589,
        "0044E589: 00447050 on a null form faults",
    ))?;
    load_form_from_file(world, form, cx)?;
    add_form_to_list(world, form, cx)?;
    Ok(1)
}

// IDA: 0044E5A5 the default arm: the flag g_disableWarningMessages saved and set; an existing form is kept, or 00447DF0 builds one. A null form warns: "Unknown %s_ID in ConstructObject." when +0x23C is nonzero, else "NO_FORM trying to load in ConstructObject." for type 0 (no warning for another type); the flag goes back; returns 0.
// With a form, the flag goes back before 00447050. Its result nonzero: a new form appends to +0x00 (004B2B80, on [DH+0] with no cast) and the arm returns 1. Zero: vt+0x10(1) deletes the form, new or existing, and the arm returns 0. The destructors aren't ported, so that delete is a host stop. Completion: 100% of the control flow.
// TODO: [ ] 0044E667 vt+0x10(1) (not reached at startup: P12-RUN-001), with each class's destructor.
fn default_arm(
    world: &mut World,
    cx: &mut FormLoadCx<'_, '_>,
    form_type: u32,
    existing: Option<Entity>,
) -> Result<u32, GameError> {
    let saved = cx.file.services.disable_warning_messages_b06b18;
    cx.file.services.disable_warning_messages_b06b18 = 1;
    let created = existing.is_none();
    let form = match existing {
        Some(form) => Some(form),
        None => create_form_by_type(world, form_type, cx)?,
    };
    let Some(form) = form else {
        let fourcc = cx.file.file()?.record_23c.type_23c;
        if fourcc != 0 {
            let name: String = fourcc
                .to_le_bytes()
                .iter()
                .map(|&byte| char::from(byte))
                .collect();
            cx.file
                .services
                .warn(format!("Unknown {name}_ID in ConstructObject."));
        } else if form_type == 0 {
            cx.file
                .services
                .warn("NO_FORM trying to load in ConstructObject.".to_owned());
        }
        cx.file.services.disable_warning_messages_b06b18 = saved;
        return Ok(0);
    };
    cx.file.services.disable_warning_messages_b06b18 = saved;
    if !load_form_from_file(world, form, cx)? {
        return Err(GameError::unclassified(
            0x0044E670,
            "0044E670: a failed LoadForm deletes the form (vt+0x10)",
        ));
    }
    if created {
        if world.get::<TesObjectLinks>(form).is_none() {
            return Err(GameError::unclassified(
                0x0044E65B,
                "0044E65B: 004B2B80 needs TESObject links this form lacks",
            ));
        }
        append_tes_object(world, form);
    }
    Ok(1)
}

// IDA: 0044DE35 the TES4 arm: the chunks, advancing (result ignored) until 0 (returns 1) or DELE: its 8 bytes go in a local (00450C20), which 0044FA50 copies to +0x3F8 and +0x3FC. Returns 1. A short chunk leaves the local's stack bytes in the original; the host has zero. Completion: 100%.
fn tes4_arm(cx: &mut FormLoadCx<'_, '_>) -> Result<u32, GameError> {
    let mut chunk = cx.file.chunk_type()?;
    while chunk != 0 {
        if chunk == DELE {
            let mut bytes = [0_u8; 8];
            cx.file.read_chunk(&mut bytes, 8)?;
            let file = cx.file.file()?;
            file.field_3f8 = u32::from_le_bytes(bytes[..4].try_into().expect("four bytes"));
            file.field_3fc = u32::from_le_bytes(bytes[4..].try_into().expect("four bytes"));
            return Ok(1);
        }
        cx.file.advance_chunk()?;
        chunk = cx.file.chunk_type()?;
    }
    Ok(1)
}

// IDA: 0044E326 the GMST arm: an EDID's text (alloca, 00450C20 size 0) names the setting for 00B35468 vt+0x24, and the result is ignored; returns 1. A chunk with no NUL runs on in the original; the host stops at the chunk. Completion: 100%.
fn gmst_arm(world: &mut World, cx: &mut FormLoadCx<'_, '_>) -> Result<u32, GameError> {
    if cx.file.chunk_type()? == EDID {
        let key = cx.file.read_text(0)?;
        let mut settings =
            world
                .get_resource_mut::<GameSettingCollection>()
                .ok_or(GameError::unclassified(
                    0x0044E356,
                    "0044E356: no GameSettingCollection",
                ))?;
        settings.load_setting(&key, &mut cx.file)?;
    }
    Ok(1)
}

// IDA: 0044E512 the SKIL arm: an existing form that casts to TESSkill gets 0052EAF0 and 00447050; returns 1 with or without one. Completion: 100%.
fn skill_arm(
    world: &mut World,
    cx: &mut FormLoadCx<'_, '_>,
    existing: Option<Entity>,
) -> Result<u32, GameError> {
    let Some(form) = existing.filter(|&form| world.get::<TesSkill>(form).is_some()) else {
        return Ok(1);
    };
    init_skill_form(world, form);
    load_form_from_file(world, form, cx)?;
    Ok(1)
}

/// IDA: 0044DCF0 DataHandler_ConstructObject(file, first). The record's form type (ebx); `first` limits the lookup to reserved IDs (0046ABD0); a nonzero ID looks up g_formsByID (esi); a form of another type warns "Form (%08X) in file \"%s\" has wrong type..." and returns 0. A partial record (0x4000) clears that flag on the
/// record, and without a form warns "Partial Form ..." and returns 0. Otherwise a form, unless the record has flag 0x20, runs vt+0x18 then vt+0x14. Then the switch. Completion: 100% of the frame and the TES4, GMST, SKIL, list and default arms; other arms through `FormClass::special_arm`.
pub fn construct_object(
    world: &mut World,
    cx: &mut FormLoadCx<'_, '_>,
    first: bool,
) -> Result<u32, GameError> {
    let form_type = cx.file.record_form_type()?;
    let record = cx.file.file()?.record_23c;
    let id = record.form_id_248;
    // 0044DD3F..0044DDC4.
    let mut existing = None;
    if (!first || reserved_form_id(id)) && id != 0 {
        existing = world.resource::<FormsById>().lookup(id);
        if let Some(form) = existing {
            // 0044DDC6..0044DDFC.
            let found = u32::from(
                world
                    .get::<FormIdentity>(form)
                    .expect("a form")
                    .form_type_04,
            );
            if found != form_type {
                let name = |index: u32| {
                    FORM_TYPE_TABLE
                        .get(index as usize)
                        .map_or("", |(_, name)| *name)
                };
                let file_name = cx.file.file()?.name_1c.clone();
                cx.file.services.warn(format!(
                    "Form ({id:08X}) in file \"{file_name}\" has wrong type.\n\nShould be:\t{}\nIs:\t\t{}",
                    name(found),
                    name(form_type),
                ));
                return Ok(0);
            }
        }
    }
    let arm = construct_arm(form_type);
    // 0044DD6F..0044DDA4 and 0044DE01..0044DE19.
    if record.flags_244 & FORM_FLAG_PARTIAL != 0 {
        let file = cx.file.file()?;
        file.record_23c.flags_244 &= !FORM_FLAG_PARTIAL;
        if existing.is_none() {
            let file_name = file.name_1c.clone();
            let id = file.record_23c.form_id_248;
            cx.file.services.warn(format!(
                "Partial Form ({id:08X}) in file \"{file_name}\" could not find base form."
            ));
            return Ok(0);
        }
    } else if let Some(form) = existing
        && record.flags_244 & RECORD_FLAG_NO_RESET == 0
    {
        // A type with no class yet takes `AbsentArm` below, with the pair it can't run.
        if let Some(class) = enabled_class(cx, form_type) {
            note_cell_override_frees(world, form);
            (class.reset_18)(world, form)?;
            (class.reset_14)(world, form)?;
        }
    }
    // 0044DE1B the switch.
    match arm {
        ConstructArm::Tes4 => tes4_arm(cx),
        ConstructArm::Gmst => gmst_arm(world, cx),
        ConstructArm::Skill => skill_arm(world, cx, existing),
        ConstructArm::List | ConstructArm::Special => {
            let Some(class) = enabled_class(cx, form_type) else {
                return absent_arm(world, cx, form_type);
            };
            if arm == ConstructArm::List {
                return list_arm(world, cx, form_type, existing);
            }
            let special = class.special_arm.ok_or_else(|| pending_arm(form_type))?;
            special(world, cx, existing, first)
        }
        ConstructArm::Default => {
            // A type outside the factory runs the arm's warnings; a factory type needs its class.
            if form_type_row(form_type as u8).is_some()
                && form_type <= 0xFF
                && enabled_class(cx, form_type).is_none()
            {
                return absent_arm(world, cx, form_type);
            }
            default_arm(world, cx, form_type, existing)
        }
    }
}

// IDA: N/A a type's class, unless a test's filter treats it as absent.
fn enabled_class(cx: &mut FormLoadCx<'_, '_>, form_type: u32) -> Option<&'static FormClass> {
    if !cx.host.arm_enabled(form_type) {
        return None;
    }
    form_class(form_type)
}

// IDA: N/A the C++ `services.construct_arm` being null or a test's counting hook: the host decides.
fn absent_arm(
    world: &mut World,
    cx: &mut FormLoadCx<'_, '_>,
    form_type: u32,
) -> Result<u32, GameError> {
    match cx.host.absent_arm(form_type) {
        AbsentArm::Stop => Err(pending_arm(form_type)),
        AbsentArm::CountAndSkip => {
            *world
                .resource_mut::<DataLoadGlobals>()
                .pending_arms
                .entry(form_type)
                .or_insert(0) += 1;
            Ok(1)
        }
    }
}

/// IDA: 00448C60 DataHandler_ShouldEnterGroup(file, header) (__stdcall). A non-master file with a null g_saveLoadGame or without its bit 12 returns 1. Otherwise, with a null header 1; group types 4, 5 and 9 return 0; without the save-load bit 1; with it, the label's form type through bytes 00448CF0 over 00448CE8, read from the exe:
/// {3, 5, 7..0x12, 0x17..0x19, 0x1C..0x20, 0x25, 0x28, 0x29, 0x2B, 0x2D..0x2F, 0x39, 0x3A, 0x3C..0x44} return 0, as anything below 3 returns 1. Completion: 100%.
pub fn should_enter_group(
    is_master: bool,
    group_type_248: u32,
    label_244: u32,
    save_load_flags: Option<u32>,
    services: &mut TesFileServices<'_>,
) -> bool {
    let save_load = save_load_flags.is_some_and(|flags| flags & SAVE_LOAD_FLAG != 0);
    if !is_master && !save_load {
        return true;
    }
    if group_type_248 >= 4 && (group_type_248 <= 5 || group_type_248 == 9) {
        return false;
    }
    if !save_load {
        return true;
    }
    !matches!(
        super::tes_file::form_type_from_record_fourcc(label_244, services.form_type_cache),
        0x03 | 0x05
            | 0x07..=0x12
            | 0x17..=0x19
            | 0x1C..=0x20
            | 0x25
            | 0x28
            | 0x29
            | 0x2B
            | 0x2D..=0x2F
            | 0x39
            | 0x3A
            | 0x3C..=0x44
    )
}

/// IDA: 0044F2E0..0044F31D the percent: `fild` holds the counter exactly, a value with the top bit adds 2^32 (00A2FC78) rounded to float at this 24-bit site, `fdivp` rounds to float, `fmul qword` 00A309F0 = 100.0 rounds to float, 009828C0 truncates (0x80000000 out of range, as a total of 0 gives). Below 2^24, as every real count is, `f64` division then rounding once is the float quotient. Completion: 100%.
#[must_use]
pub fn record_percent(count: u32, total: u32) -> i32 {
    let exact = |value: u32| {
        let signed = value as i32;
        if signed < 0 {
            f64::from((f64::from(signed) + 4_294_967_296.0) as f32)
        } else {
            f64::from(signed)
        }
    };
    let quotient = (exact(count) / exact(total)) as f32;
    let scaled = (f64::from(quotient) * 100.0) as f32;
    x87::ftol(scaled)
}

// IDA: 0044F243..0044F27F the save-load record arm: 00B33AD8 counts; every 256th 004523A0(0, 1.0); the record's form goes to 00461FA0. Those two are host stops. Completion: 100% of the counter.
// TODO: [ ] 004523A0 and 00461FA0 (not reached at startup: the flag is clear in P11-RUN-001).
fn save_load_record(world: &mut World, id: u32) -> Result<(), GameError> {
    let mut load = world.resource_mut::<DataLoadGlobals>();
    load.save_load_count_b33ad8 = load.save_load_count_b33ad8.wrapping_add(1);
    if load.save_load_count_b33ad8 as u8 == 0 {
        return Err(GameError::unclassified(
            0x0044F25B,
            "0044F25B: 004523A0 (the save-load arm)",
        ));
    }
    if world.resource::<FormsById>().lookup(id).is_some() {
        return Err(GameError::unclassified(
            0x0044F27A,
            "0044F27A: 00461FA0 (the save-load arm)",
        ));
    }
    Ok(())
}

// IDA: g_saveLoadGame's +0x18, `None` for a null 00B33B00.
fn save_load_flags(world: &World) -> Option<u32> {
    world
        .get_resource::<TesGlobals>()
        .and_then(|globals| globals.save_load_game.as_ref())
        .map(|game| game.fields_14[1])
}

/// IDA: 0044F0C0's locals across its loop.
#[derive(Clone, Copy, Debug)]
struct FileRecords {
    id: TesFileId,
    /// n3c, the argument.
    first: bool,
    /// [esp+0x18], the result.
    result_18: u32,
    /// [esp+0x1C].
    last_percent_1c: i32,
}

// IDA: 0044F0EB..0044F119 the file's open, with "DataHandler: internal error" on a failure, and the locals and three statics. Completion: 100%.
fn enter_file(
    world: &mut World,
    id: TesFileId,
    first: bool,
    services: &mut DataLoadServices<'_>,
) -> Result<FileRecords, GameError> {
    {
        let mut load = world.resource_mut::<DataLoadGlobals>();
        let count = load.record_count_b33a94;
        load.file_entry_counts.push(count);
    }
    with_files(world, services, |_world, files, file_services, _host| {
        let (directory, name) = {
            let file = files.get_mut(id)?;
            (file.directory_120.clone(), file.name_1c.clone())
        };
        if !open_tes_file(files, id, &directory, &name, 0, false, file_services)? {
            file_services.warn("DataHandler: internal error".to_owned());
        }
        Ok(())
    })?;
    let mut load = world.resource_mut::<DataLoadGlobals>();
    load.current_cell_b33a9c = None;
    load.current_world_b33aa0 = None;
    load.current_info_b33aa4 = None;
    Ok(FileRecords {
        id,
        first,
        result_18: 1,
        last_percent_1c: 0,
    })
}

// IDA: 0044F1B9..0044F20C a TOFT with any other ID, by the first current form of three: INFO's +0x34 = the ID; a world's 004EF030(ID - +0x25C); an interior cell's 004C9D20(ID). Completion: 100%.
pub fn toft_offset_arm(world: &mut World, id: u32, record_offset: u32) -> Result<(), GameError> {
    let load = world.resource::<DataLoadGlobals>();
    let (info, space, cell) = (
        load.current_info_b33aa4,
        load.current_world_b33aa0,
        load.current_cell_b33a9c,
    );
    if let Some(info) = info {
        // 0044F1C2 `mov [eax+0x34], edi`: no cast, and 00B33AA4 only ever holds an info.
        world
            .get_mut::<TesTopicInfo>(info)
            .expect("00B33AA4 holds an info")
            .record_offset_34 = id;
    } else if let Some(space) = space {
        world_apply_toft_delta(world, space, id.wrapping_sub(record_offset))?;
    } else if let Some(cell) = cell
        && cell_is_interior(world, cell)
    {
        cell_set_toft(world, cell, id);
    }
    Ok(())
}

// IDA: one pass of 0044F0C0's loop, 0044F125..0044F36B: the group, TOFT or record, the counter and percent, the next record. Returns `bl`, whether another pass runs. Completion: 100%, with the 0044F179 TOFT arms.
fn step_file_record(
    world: &mut World,
    state: &mut FileRecords,
    services: &mut DataLoadServices<'_>,
) -> Result<bool, GameError> {
    with_files(world, services, |world, files, file_services, host| {
        let id = state.id;
        // 0044F127 [esp+0x13] = 1; 0044F12C bl = 1.
        let mut skip_next = true;
        let mut more = true;
        let mut cx = FormLoadCx {
            file: RecordReader {
                files,
                id,
                services: file_services,
            },
            host,
        };
        let form_type = cx.file.record_form_type()?;
        if form_type == FORM_TYPE_GROUP {
            // 0044F138..0044F164.
            let file = cx.file.file()?;
            let (is_master, group_type, label) = (
                file.is_master(),
                file.record_23c.form_id_248,
                file.record_23c.flags_244,
            );
            let flags = save_load_flags(world);
            if !should_enter_group(is_master, group_type, label, flags, cx.file.services) {
                skip_next = false;
                more = skip_group(cx.file.files, id, cx.file.services)?;
            }
        } else if form_type == FORM_TYPE_TOFT {
            // 0044F179..0044F20C.
            let record_id = cx.file.file()?.record_23c.form_id_248;
            if record_id == 0xFFFF_FFFF {
                skip_next = false;
                more = false;
            } else if record_id == 0xFFFF_FFFE {
                // 0044F195..0044F1A8 00738500(+0x25C + 0x14, +0x240) is `mov al, 1; retn 8`.
                skip_next = false;
                more = false;
            } else {
                toft_offset_arm(world, record_id, cx.file.record_offset()?)?;
            }
        } else {
            // 0044F211..0044F2DE.
            if construct_object(world, &mut cx, state.first)? == 0 {
                state.result_18 = 0;
            } else {
                let record_id = cx.file.file()?.record_23c.form_id_248;
                if save_load_flags(world).is_some_and(|flags| flags & SAVE_LOAD_FLAG != 0) {
                    save_load_record(world, record_id)?;
                } else if let Some(form) = world.resource::<FormsById>().lookup(record_id) {
                    // 0044F296..0044F2A9: NPC_, CREA, REFR, ACHR and ACRE aren't mapped (bytes 0044F3B0 over 0044F3A8).
                    let found = world
                        .get::<FormIdentity>(form)
                        .expect("a form")
                        .form_type_04;
                    if !matches!(found, 0x23 | 0x24 | 0x31 | 0x32 | 0x33) {
                        let name = crate::tes::form_arms::editor_id_of(world, form);
                        if !name.is_empty() {
                            let full = world
                                .get::<crate::tes::form_components::TesFullName>(form)
                                .and_then(|full| full.name_04.data_00.clone())
                                .unwrap_or_default();
                            world
                                .resource_mut::<EditorIdMap>()
                                .insert_after_cell_override(&name, &full, form);
                        }
                    }
                }
            }
        }
        // 0044F2E0..0044F32A the counter and percent.
        let (count, total) = {
            let mut load = world.resource_mut::<DataLoadGlobals>();
            load.record_count_b33a94 = load.record_count_b33a94.wrapping_add(1);
            (load.record_count_b33a94, load.record_total_b33a90)
        };
        let percent = record_percent(count, total);
        if percent != state.last_percent_1c {
            // 0044F32C..0044F354: the sprintf, the unused buffer, and 0057B950(0, float(percent)).
            state.last_percent_1c = percent;
            let text = format!("Loading Files {percent}% ({})", cx.file.file()?.name_1c);
            {
                let mut load = world.resource_mut::<DataLoadGlobals>();
                load.last_progress_text = text;
                load.progress_reports += 1;
            }
            crate::tes::loading_menu::loading_menu_slides_at_load_progress(world, &mut || {
                cx.host.random_next()
            })?;
            cx.host.section_progress(0, percent as f32);
        }
        // 0044F357..0044F367.
        if skip_next {
            more = skip_record(cx.file.files, id, true, cx.file.services)?;
        }
        Ok(more)
    })
}

// IDA: 0044F373..0044F385 the close, with its warning; the function returns [esp+0x18]. Completion: 100%.
fn leave_file(
    world: &mut World,
    state: &FileRecords,
    services: &mut DataLoadServices<'_>,
) -> Result<u32, GameError> {
    with_files(world, services, |_world, files, file_services, _host| {
        if !close_tes_file(files.get_mut(state.id)?, file_services)? {
            file_services.warn("DataHandler: internal error".to_owned());
        }
        Ok(())
    })?;
    world
        .resource_mut::<DataLoadGlobals>()
        .file_results
        .push(state.result_18);
    Ok(state.result_18)
}

/// IDA: N/A a resumable 0044F3D0: the cursor between its original loop boundaries.
#[derive(Debug)]
pub struct LoadSelectedFiles {
    /// a2 of 0044F3D0.
    a2: bool,
    /// n3 of 0044F3D0.
    n3: bool,
    stage: LoadStage,
}

#[derive(Debug)]
enum LoadStage {
    Prefix,
    /// edi of 0044F735's loop, between files.
    NextFile(u32),
    /// 0044F0C0 running, on compiled file `index`.
    Records {
        index: u32,
        state: FileRecords,
    },
    /// 0044F780: the tail.
    Tail,
    Done(u32),
}

impl LoadSelectedFiles {
    /// IDA: N/A the cursor before 0044F3F9.
    #[must_use]
    pub const fn new(a2: bool, n3: bool) -> Self {
        Self {
            a2,
            n3,
            stage: LoadStage::Prefix,
        }
    }

    /// IDA: N/A one step, at most one record of 0044F0C0. Returns 0044F3D0's eax when the function has returned. Completion: 100%.
    pub fn step(
        &mut self,
        world: &mut World,
        services: &mut DataLoadServices<'_>,
    ) -> Result<Option<u32>, GameError> {
        match self.stage {
            LoadStage::Prefix => {
                if run_prefix(world, self.a2, self.n3, services)? {
                    self.stage = LoadStage::NextFile(0);
                } else {
                    self.stage = LoadStage::Done(0);
                }
            }
            LoadStage::NextFile(index) => {
                let data = world.resource::<DataHandler>();
                if index >= data.loaded_mod_count_8d0 {
                    // 0044F785 +0xCD7 = 0.
                    world.resource_mut::<DataHandler>().loading_cd7 = 0;
                    self.stage = LoadStage::Tail;
                } else {
                    let id = data.mods_by_id_8d4[index as usize].ok_or(GameError::unclassified(
                        0x0044F74E,
                        "0044F74E: a null compiled file",
                    ))?;
                    // 0044F735..0044F745 the first file, unless a2.
                    let first = index == 0 && !self.a2;
                    let state = enter_file(world, id, first, services)?;
                    self.stage = LoadStage::Records { index, state };
                }
            }
            LoadStage::Records {
                index,
                ref mut state,
            } => {
                if !step_file_record(world, state, services)? {
                    let state = *state;
                    if leave_file(world, &state, services)? == 0 {
                        // 0044F762..0044F76D.
                        let path = services.host.warnings_log_path();
                        let flag = world
                            .resource::<DataLoadGlobals>()
                            .disable_warning_messages_b06b18;
                        (services.warning)(
                            format!(
                                "DataHandler: unrecognized form\r\nLook in the {path} file for more info.\r\n"
                            ),
                            flag != 0,
                        );
                    }
                    self.stage = LoadStage::NextFile(index + 1);
                }
            }
            LoadStage::Tail => {
                // 0044F780..0044FA38: the passes, TLS +0x184 = 0 at 0044F95A and 00B33A9C = 0.
                if services.host.run_post_load_passes(world) {
                    super::post_load::run_post_load_passes(world, services)?;
                } else {
                    world.resource_mut::<GameThreadLocals>().byte_184 = 0;
                    world.resource_mut::<DataLoadGlobals>().current_cell_b33a9c = None;
                }
                self.stage = LoadStage::Done(1);
            }
            LoadStage::Done(result) => return Ok(Some(result)),
        }
        if let LoadStage::Done(result) = self.stage {
            return Ok(Some(result));
        }
        Ok(None)
    }

    /// IDA: N/A true once the next step is the tail: every record is in and no InitItem has run. A test reads the rows of that moment here.
    #[must_use]
    pub const fn at_tail(&self) -> bool {
        matches!(self.stage, LoadStage::Tail)
    }

    /// IDA: N/A every step in order, as a test runs it.
    pub fn run(
        &mut self,
        world: &mut World,
        services: &mut DataLoadServices<'_>,
    ) -> Result<u32, GameError> {
        loop {
            if let Some(result) = self.step(world, services)? {
                return Ok(result);
            }
        }
    }
}

/// IDA: 0044F3D0 DataHandler_LoadSelectedFiles(a2, n3), straight through. Returns 1, or 0 when a master can't be found. Completion: 100%.
pub fn load_selected_files(
    world: &mut World,
    a2: bool,
    n3: bool,
    services: &mut DataLoadServices<'_>,
) -> Result<u32, GameError> {
    LoadSelectedFiles::new(a2, n3).run(world, services)
}

// IDA: 004504B0 TESFile_IsNewerVersion: `fld1`, `fcomp` with the HEDR float at +0x3D0, `test ah, 5`, `jp`: exactly C0 set, so 1.0 < version with both ordered. A NaN version is not newer (the C++ treats it as newer). g_disableWarningMessages is saved and cleared around "File %s is a higher version than this EXE can load.". Completion: 100%.
fn file_is_newer_version(
    files: &TesFileStore,
    id: TesFileId,
    services: &mut TesFileServices<'_>,
) -> Result<bool, GameError> {
    let file = files.get(id).ok_or(GameError::unclassified(
        0x004504C8,
        "004504C8: a null TESFile faults",
    ))?;
    if f32::from_bits(file.header_3d0[0]) > 1.0 {
        let text = format!(
            "File {} is a higher version than this EXE can load.",
            file.name_1c
        );
        let saved = services.disable_warning_messages_b06b18;
        services.disable_warning_messages_b06b18 = 0;
        services.warn(text);
        services.disable_warning_messages_b06b18 = saved;
        return Ok(true);
    }
    Ok(false)
}

// IDA: 0044F3F9..0044F6C9 up to 0044AC20, then the totals and the +0xCD7 set up to 0044F733. Returns false for the master-missing return. Completion: 100%, with a 256th compiled file a host stop.
fn run_prefix(
    world: &mut World,
    a2: bool,
    n3: bool,
    services: &mut DataLoadServices<'_>,
) -> Result<bool, GameError> {
    world.init_resource::<DataLoadGlobals>();
    // 0044F3F9 the loader thread.
    world.resource_mut::<GameThreadLocals>().byte_184 = 1;
    let mut data = world
        .get_resource_mut::<DataHandler>()
        .ok_or(GameError::unclassified(
            0x004067C7,
            "004067C7: 0044F3D0 on a null DataHandler",
        ))?;
    // 0044F400..0044F408.
    if a2 || n3 {
        data.loaded_mod_count_8d0 = 0;
    }
    // 0044F41A 00442630(g_tes, 0, 0).
    tes_unload_cells(world)?;
    {
        let mut globals = world.resource_mut::<TesGlobals>();
        let tes = globals.tes.as_mut().expect("00442630 checked it");
        // 0044F41F..0044F441 +0x74 unless a2 or n3, +0x34, then 0043F220.
        if !a2 && !n3 {
            tes.current_world_space_74 = None;
        }
        tes.current_interior_cell_34 = None;
        tes_reset_exterior_coordinates(tes);
    }
    let active = world.resource::<DataHandler>().active_file_8c4;
    let cd1 = world.resource::<DataHandler>().bytes_cd0[1];
    let list = world.resource::<DataHandler>().files_8c8.clone();
    let compile_flag = world.resource::<DataHandler>().bytes_cd0[0];
    let validated = with_files(world, services, |_world, files, file_services, host| {
        // 0044F446..0044F46D the active file's header when +0xCD1 is set.
        if cd1 != 0 {
            let active = active.ok_or(GameError::unclassified(
                0x0044F459,
                "0044F459: +0xCD1 with a null +0x8C4 faults",
            ))?;
            read_tes_file_header(files, active, file_services)?;
            resolve_tes_file_masters(active, &list, files, true, file_services)?;
        }
        // 0044F480..0044F51B validate every Loaded file and mark its masters Loaded. An empty list item ends the walk; a slot whose file is destroyed stands for one, as in `lookup_file_by_name`.
        for &id in &list {
            let Some(file) = files.get(id) else {
                break;
            };
            if !file.is_loaded() {
                continue;
            }
            read_tes_file_header(files, id, file_services)?;
            resolve_tes_file_masters(id, &list, files, true, file_services)?;
            if file_is_newer_version(files, id, file_services)? {
                files.get_mut(id)?.set_loaded(false);
                continue;
            }
            let count = files.get_mut(id)?.master_count_3f0;
            for index in 0..count {
                let master = master_at(files.get_mut(id)?, id, index + 1);
                let usable = match master {
                    Some(master) => !file_is_newer_version(files, master, file_services)?,
                    None => false,
                };
                if !usable {
                    // 0044F5B2..0044F609. 0044FD90 is called twice, with the same result.
                    let text = files.get_mut(id)?.master_name(index + 1).map_or_else(
                        || "Unable to find masterfile: <unknown>".to_owned(),
                        |name| {
                            let name: String = name.iter().map(|&byte| char::from(byte)).collect();
                            format!("Unable to find masterfile: {name}")
                        },
                    );
                    host.debug_log(text);
                    return Ok(false);
                }
                files.get_mut(master.expect("usable"))?.set_loaded(true);
            }
        }
        Ok(true)
    })?;
    if !validated {
        // 0044F5DE / 0044F602.
        world.resource_mut::<GameThreadLocals>().byte_184 = 0;
        return Ok(false);
    }
    // 0044F525..0044F676 the compiled array, with g_disableWarningMessages set.
    let compiled = with_files(world, services, |world, files, file_services, host| {
        let saved = file_services.disable_warning_messages_b06b18;
        file_services.disable_warning_messages_b06b18 = 1;
        let mut changed = false;
        for &id in &list {
            let Some(file) = files.get(id) else {
                break;
            };
            if !file.is_loaded() {
                continue;
            }
            if !file.is_master()
                && compile_flag == 0
                && file.masters_changed(file_services.working_directory)
            {
                let text = format!(
                    "One of the files that \"{}\" is dependent on has changed since the last save.",
                    file.name_1c
                );
                file_services.warn(text);
                changed = true;
            }
            if files.get_mut(id)?.is_active() {
                world.resource_mut::<DataHandler>().active_file_8c4 = Some(id);
                continue;
            }
            compile_file(world, files, id, host)?;
        }
        // 0044F65B.
        *world.resource_mut::<DataHandler>().next_form_id_8c0_mut() = 0xFF00_0800;
        if changed {
            // 0044F667..0044F673 *(00B0557C).
            file_services.warn("One or more plugins could not find the correct versions of the master files they depend on. Errors may occur during load or game play. Check the \"Warnings.txt\" file for more information.\r\n".to_owned());
        }
        file_services.disable_warning_messages_b06b18 = saved;
        // 0044F67F..0044F6C6 the Active file goes last.
        if let Some(active) = world.resource::<DataHandler>().active_file_8c4 {
            compile_file(world, files, active, host)?;
        }
        Ok(())
    });
    compiled?;
    // 0044F6CB 0044AC20.
    if services.host.run_pre_load_pass(world) {
        crate::tes::data_handler::create_default_forms(world)?;
    }
    // 0044F6D0..0044F718 open every compiled file and add its HEDR count (+0x3D4) to 00B33A90: the count is +0x8D0 re-read each pass.
    let compiled: Vec<TesFileId> = {
        let data = world.resource::<DataHandler>();
        data.mods_by_id_8d4
            .iter()
            .take(data.loaded_mod_count_8d0 as usize)
            .copied()
            .collect::<Option<Vec<_>>>()
            .ok_or(GameError::unclassified(
                0x0044F6E0,
                "0044F6E0: a null compiled file",
            ))?
    };
    with_files(world, services, |world, files, file_services, _host| {
        for &id in &compiled {
            let (directory, name) = {
                let file = files.get_mut(id)?;
                (file.directory_120.clone(), file.name_1c.clone())
            };
            if open_tes_file(files, id, &directory, &name, 0, false, file_services)? {
                let count = files.get_mut(id)?.header_3d0[1];
                let mut load = world.resource_mut::<DataLoadGlobals>();
                load.record_total_b33a90 = load.record_total_b33a90.wrapping_add(count);
            } else {
                file_services.warn("DataHandler: internal error".to_owned());
            }
        }
        Ok(())
    })?;
    // 0044F71A..0044F72C.
    world.resource_mut::<DataLoadGlobals>().record_count_b33a94 = 0;
    world.resource_mut::<DataHandler>().loading_cd7 = 1;
    Ok(true)
}

// IDA: 0044F612..0044F64B / 0044F689..0044F6C6: [+0x8D4 + 4 * +0x8D0] = file; the old count's low byte goes to 0044FB20; the count at or past 0xFF logs "Too many selected files to compile!". The next file past 255 writes over +0xCD0 in the original: a host stop. Completion: 100%.
fn compile_file(
    world: &mut World,
    files: &mut TesFileStore,
    id: TesFileId,
    host: &mut dyn DataLoadHost,
) -> Result<(), GameError> {
    let mut data = world.resource_mut::<DataHandler>();
    let index = data.loaded_mod_count_8d0;
    let slot = data
        .mods_by_id_8d4
        .get_mut(index as usize)
        .ok_or(GameError::unclassified(
            0x0044F618,
            "0044F618: a compiled file past 0xFF overwrites +0xCD0",
        ))?;
    *slot = Some(id);
    data.loaded_mod_count_8d0 = index + 1;
    let count = data.loaded_mod_count_8d0;
    files.get_mut(id)?.set_mod_index(index as u8);
    if count >= 0xFF {
        host.debug_log("Too many selected files to compile!".to_owned());
    }
    Ok(())
}

// IDA: the factory's arm for TESSkill exists (00447DF0, 0x60), though no record reaches it: 0044E512 only reloads. A host stop stands in for 0052EB80's unreachable call here; `data_handler` constructs the 21.
fn construct_skill_form(_world: &mut World) -> Result<Entity, GameError> {
    Err(GameError::unclassified(
        0x00447DF0,
        "00447DF0: the TESSkill arm isn't reached: 0044E512 only reloads",
    ))
}

/// IDA: the classes of this file: TESSkill, whose arm is in the frame. 00466E80's slots for it are `TesSkill`'s own fields, so `slots` is empty and its vt+0x18, whose components' vt+4 are nullsub_2, has nothing to do.
pub const CLASSES: &[FormClass] = &[FormClass {
    form_type: FORM_TYPE_SKILL as u8,
    vtable: TES_SKILL_VTABLE,
    construct: construct_skill_form,
    load_form: load_skill_form,
    reset_14: reset_skill_14,
    reset_18: reset_18_components,
    editor_id: default_editor_id,
    init_item: no_init_item,
    slots: &[],
    special_arm: None,
}];

/// IDA: 0044DE1B..0044DE2E: `lea eax, [ebx-1]`, unsigned above 0x42 is the default arm, then byte table 0044E6DC. Completion: 100%.
#[must_use]
pub const fn construct_arm(form_type: u32) -> ConstructArm {
    match form_type {
        0x01 => ConstructArm::Tes4,
        0x03 => ConstructArm::Gmst,
        0x0B => ConstructArm::Skill,
        0x04..=0x0A | 0x0D..=0x11 | 0x2D..=0x2F | 0x3B | 0x3D..=0x3F | 0x41..=0x43 => {
            ConstructArm::List
        }
        0x0C | 0x30..=0x36 | 0x38..=0x3A | 0x3C => ConstructArm::Special,
        _ => ConstructArm::Default,
    }
}

// IDA: 0044E556 the IDLE arm of 0044DCF0: an existing form goes through 00447050 at 0044E291; otherwise 00447DF0 and 00447050. No list add. Returns 1. A null form faults in 00447050: a host stop. Completion: 100%.
pub(crate) fn idle_arm(
    world: &mut World,
    cx: &mut FormLoadCx<'_, '_>,
    existing: Option<Entity>,
    _first: bool,
) -> Result<u32, GameError> {
    let form = match existing {
        Some(form) => form,
        None => create_form_by_type(world, u32::from(FORM_TYPE_IDLE), cx)?.ok_or(
            GameError::unclassified(0x0044E566, "0044E566: 00447050 on a null form faults"),
        )?,
    };
    load_form_from_file(world, form, cx)?;
    Ok(1)
}

// IDA: 0044E494 0052FB40(0). 00447DF0 has no DIAL arm.
pub(crate) fn construct_topic(world: &mut World) -> Result<Entity, GameError> {
    construct_topic_of_type(world, 0)
}

// IDA: 0044E474 the DIAL arm of 0044DCF0: an existing topic reloads (0044E291); a new one, 0052FB40(0), loads, whatever 00447050 answers, and 0067B1E0 appends it to DataHandler +0x7C. Returns 1. FormHeap failing isn't modelled. Completion: 100%.
pub(crate) fn topic_arm(
    world: &mut World,
    cx: &mut FormLoadCx<'_, '_>,
    existing: Option<Entity>,
    _first: bool,
) -> Result<u32, GameError> {
    if let Some(form) = existing {
        load_form_from_file(world, form, cx)?;
        return Ok(1);
    }
    let form = construct_topic(world)?;
    load_form_from_file(world, form, cx)?;
    world
        .resource_mut::<DataHandlerLists>()
        .topics_7c
        .append_non_null(form);
    Ok(1)
}

// IDA: 0044E4C9 the INFO arm of 0044DCF0: a new 00530AE0(0) when there is none, 00B33AA4 = the info, then 00447050's byte. Completion: 100%.
pub(crate) fn topic_info_arm(
    world: &mut World,
    cx: &mut FormLoadCx<'_, '_>,
    existing: Option<Entity>,
    _first: bool,
) -> Result<u32, GameError> {
    let form = match existing {
        Some(form) => form,
        None => construct_topic_info(world)?,
    };
    world.resource_mut::<DataLoadGlobals>().current_info_b33aa4 = Some(form);
    Ok(u32::from(load_form_from_file(world, form, cx)?))
}

// IDA: 00447DF0's MGEF arm and 0044E3CE: FormHeap(0xA8), 00416430, reading the two GameSetting floats as they stand now. The C++ stores 0.0, what they hold before cinit 201 and 202 only.
pub(crate) fn construct_effect_setting(world: &mut World) -> Result<Entity, GameError> {
    let settings = world.get_resource::<GameSettingCollection>();
    let (factor_90, factor_94) = settings.map_or((0, 0), |settings| {
        (
            settings.raw_at(0x00B3_3704).unwrap_or(0),
            settings.raw_at(0x00B3_370C).unwrap_or(0),
        )
    });
    construct_effect_setting_with(world, factor_90, factor_94)
}

// IDA: 0044E367 the MGEF arm of 0044DCF0. A first chunk other than EDID returns 1. The EDID text (alloca, size 0) names the code: a setting found goes through 00447050; otherwise FormHeap(0xA8) 00416430, 00451210 and a prepend to +0x8B8 (00446CB0), with no LoadForm. Returns 1. Completion: 100%.
pub(crate) fn effect_setting_arm(
    world: &mut World,
    cx: &mut FormLoadCx<'_, '_>,
    _existing: Option<Entity>,
    _first: bool,
) -> Result<u32, GameError> {
    if cx.file.chunk_type()? != crate::tes::forms_magic::EDID {
        return Ok(1);
    }
    let text = cx.file.read_text(0)?;
    if let Some(found) = find_setting_by_name(world, &text)? {
        load_form_from_file(world, found, cx)?;
        return Ok(1);
    }
    let form = construct_effect_setting(world)?;
    initialize_form_from_current_record(world, form, cx)?;
    world
        .resource_mut::<DataHandlerLists>()
        .list_8b8
        .prepend_non_null(form);
    Ok(1)
}

/// IDA: 0044E001, 0044DCF0's arm for types 0x31..0x33. No current cell returns 0. An existing reference: vt+0x170 and the TESChildCell's vt+0: the current cell clears `[ebp-0xD]`, the insert flag; another nonnull one runs 004CECD0. A new one: 004DB260(type, 1). 00447050. On a false load: a null base restores +0x04 with the record's type; a persistent cell takes 004247B0, any other vt+0x194; +0x8B8 takes the reference unless present; a MobileObject leaves process level 3 (00674550); no insert. With a base: the insert (004D35D0), then the base's vt+0x108 or vt+0x10C, which are `xor eax,eax` in every TESBoundObject factory class (006F7070, from each vtable); a set g_saveLoadGame +0x18 bit 12 with a 3D runs vt+0x150(0), a host stop. Finally 004DB4A0 with a file version below qword 00A38538 (0.2 rounded to float, compared with `fcomp`; NaN isn't below) warns and clears flag 0x400. Returns 1. Completion: 100%.
pub fn reference_arm(
    world: &mut World,
    cx: &mut FormLoadCx<'_, '_>,
    existing: Option<Entity>,
    _first: bool,
) -> Result<u32, GameError> {
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
                cell_remove_reference(world, previous, reference)?;
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
        // 0044E113 004D35D0. Its attached arms (004D3753..004D38D2) call above this layer: reaching them from the record's
        // load is a host stop.
        if cell_insert_reference_head(world, cell, reference)? == CellInsertRest::AttachedArms {
            return Err(GameError::unclassified(
                0x004D3753,
                "004D3753: the attached arms, from 0044E001",
            ));
        }
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
        return Err(GameError::unclassified(
            0x0044E183,
            "0044E183: vt+0x150(0) with a loaded 3D",
        ));
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
) -> Result<String, GameError> {
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

// IDA: 00447DF0's SCPT arm, past FormHeap: 004FBAA0.
pub(crate) fn construct_script(world: &mut World) -> Result<Entity, GameError> {
    construct_script_in_place(world)
}

/// IDA: 0044DE09's vt+0x18 and 0044DE12's vt+0x14 on a cell with a name: 0041E450 frees the removed extras in list order, the type 0x0A entry's string and then its 0x14, the others' 0x10; then 004C9780 the +0x3C block (0x28 interior, 8 exterior), and vt+0x14 FULL's string. Run before them. Evidence: `run-c1-log`'s 84 overrides. Completion: 100% of the 84, taking 0x10 for every other extra's blocks, which those overrides don't contradict but don't each show.
pub fn note_cell_override_frees(world: &mut World, form: Entity) {
    let Some(cell) = world.get::<TesObjectCell>(form) else {
        return;
    };
    let Some(key) = cell.extra_28.editor_id().map(<[u8]>::to_vec) else {
        return;
    };
    let mut freed_after = Vec::new();
    let mut after_key = false;
    for entry in &cell.extra_28.entries {
        if entry.type_04 == EXTRA_EDITOR_ID {
            after_key = true;
            freed_after.push(0x14);
        } else if after_key && entry.type_04 != 8 && !matches!(entry.type_04, 2 | 3) {
            freed_after.push(0x10);
        }
    }
    if cell.data_3c.is_some() {
        freed_after.push(if cell.flags_24 & 1 != 0 { 0x28 } else { 8 });
    }
    if let Some(text) = world
        .get::<TesFullName>(form)
        .and_then(|full| full.name_04.data_00.as_deref())
        && !text.is_empty()
    {
        freed_after.push(text.len() as u32 + 1);
    }
    world
        .resource_mut::<EditorIdMap>()
        .note_freed_cell_key(FreedCellKey {
            form,
            key,
            freed_after,
        });
}

/// IDA: 0044DEF0, 0044DCF0's CELL arm. 004D3210 for a new cell; 00447050; +0xCD1 runs vt+0x90(1), a host stop; 0046A9A0(1). A new interior goes in the +0xC0 array, a new exterior in the world (a null world faults), whose failure warns "Error adding %scell (%08X) to world space (%08X). Cell will be destroyed.", deletes the cell (vt+0x10, a host stop) and clears 00B33A9C; 00B33A9C = the cell; returns 1. Completion: 100% of the control flow.
/// TODO: [ ] 0044DF4C the cell's vt+0x90(1) 004CCBF0; 0044DFED its destructor.
pub fn cell_arm(
    world: &mut World,
    cx: &mut FormLoadCx<'_, '_>,
    existing: Option<Entity>,
    _first: bool,
) -> Result<u32, GameError> {
    let cell = match existing {
        Some(cell) => cell,
        None => construct_cell(world)?,
    };
    load_form_from_file(world, cell, cx)?;
    if world.resource::<DataHandler>().bytes_cd0[1] != 0 {
        return Err(GameError::unclassified(
            0x0044DF4C,
            "0044DF4C: vt+0x90(1) with DataHandler +0xCD1",
        ));
    }
    crate::tes::form::set_master_flag(world, cell, true);
    if existing.is_none() {
        if cell_is_interior(world, cell) {
            let mut data = world.resource_mut::<DataHandler>();
            add_interior_cell(&mut data.cells_c0, cell);
        } else {
            let space = current_or_first_world(world).ok_or(GameError::unclassified(
                0x0044DFAA,
                "0044DFAA: a cell with no world faults",
            ))?;
            if !world_add_cell(world, space, cell, &mut |text| cx.file.services.warn(text))? {
                let identity = *world.get::<FormIdentity>(cell).expect("a form");
                let persistent = if identity.flags_08 & FORM_FLAG_PERSISTENT != 0 {
                    "Persistent "
                } else {
                    ""
                };
                let id = world.get::<FormIdentity>(space).expect("a form").form_id_0c;
                cx.file.services.warn(format!(
                    "Error adding {persistent}cell ({:08X}) to world space ({id:08X}). Cell will be destroyed.",
                    identity.form_id_0c
                ));
                return Err(GameError::unclassified(
                    0x0044DFED,
                    "0044DFED: deleting a cell that failed to join its world (vt+0x10)",
                ));
            }
        }
    }
    world.resource_mut::<DataLoadGlobals>().current_cell_b33a9c = Some(cell);
    Ok(1)
}

/// IDA: 0044E401, 0044DCF0's WRLD arm. A new world: 004F2A10, 00447050, then appended to +0x0C (0067B1E0). An existing one: 00447050. 00B33AA0 = the world; returns 1. Completion: 100%.
pub fn world_arm(
    world: &mut World,
    cx: &mut FormLoadCx<'_, '_>,
    existing: Option<Entity>,
    _first: bool,
) -> Result<u32, GameError> {
    let form = if let Some(form) = existing {
        load_form_from_file(world, form, cx)?;
        form
    } else {
        let form = construct_world(world)?;
        load_form_from_file(world, form, cx)?;
        world
            .resource_mut::<DataHandlerLists>()
            .world_spaces_0c
            .append_non_null(form);
        form
    };
    world.resource_mut::<DataLoadGlobals>().current_world_b33aa0 = Some(form);
    Ok(1)
}

/// IDA: 0044DE83, 0044DCF0's ROAD arm. A new road: 004E8FA0. 00447050. For a new one, 00B33AA0 (or DataHandler +0x0C's head) takes it through 004EF100, which deletes an old road (vt+0x10, a host stop), and +0x2C = the world. Returns 1. Completion: 100% of the control flow.
/// TODO: [ ] 004EF10C an old road's destructor.
pub fn road_arm(
    world: &mut World,
    cx: &mut FormLoadCx<'_, '_>,
    existing: Option<Entity>,
    _first: bool,
) -> Result<u32, GameError> {
    let road = match existing {
        Some(road) => road,
        None => construct_road(world)?,
    };
    load_form_from_file(world, road, cx)?;
    if existing.is_none() {
        let space = current_or_first_world(world).ok_or(GameError::unclassified(
            0x0044DED8,
            "0044DED8: a road with no world faults",
        ))?;
        let mut data = world.get_mut::<TesWorldSpace>(space).expect("a world");
        if data.road_54.is_some() {
            return Err(GameError::unclassified(
                0x004EF10C,
                "004EF10C: replacing a world's road (vt+0x10)",
            ));
        }
        data.road_54 = Some(road);
        world.get_mut::<TesRoad>(road).expect("a road").world_2c = Some(space);
    }
    Ok(1)
}

/// IDA: 0044E237, 0044DCF0's LAND arm: no current cell returns 1. 004CE3C0: an exterior, non-persistent cell's +0x40, made and set (004BFDC0) when null; null for any other cell, which takes a new land (004C4C00), 004BFDC0 and 004C9AE0: an interior ignores it, a persistent exterior stores it. The form the frame found isn't used. 00447050. Returns 1. Completion: 100% of the control flow.
/// TODO: [ ] 004C9AF9 a cell's old land's destructor.
pub fn land_arm(
    world: &mut World,
    cx: &mut FormLoadCx<'_, '_>,
    _existing: Option<Entity>,
    _first: bool,
) -> Result<u32, GameError> {
    let Some(cell) = world.resource::<DataLoadGlobals>().current_cell_b33a9c else {
        return Ok(1);
    };
    let persistent =
        world.get::<FormIdentity>(cell).expect("a form").flags_08 & FORM_FLAG_PERSISTENT != 0;
    let interior = cell_is_interior(world, cell);
    let land = if !interior && !persistent {
        let current = world.get::<TesObjectCell>(cell).expect("a cell").land_40;
        match current {
            Some(land) => land,
            None => {
                let land = construct_land(world)?;
                world
                    .get_mut::<TesObjectCell>(cell)
                    .expect("a cell")
                    .land_40 = Some(land);
                land_set_cell(world, land, cell);
                land
            }
        }
    } else {
        let land = construct_land(world)?;
        land_set_cell(world, land, cell);
        if !interior {
            // 004C9AE0: a persistent exterior takes it, and deletes an old one.
            let mut data = world.get_mut::<TesObjectCell>(cell).expect("a cell");
            if data.land_40.is_some() {
                return Err(GameError::unclassified(
                    0x004C9AF9,
                    "004C9AF9: replacing a cell's land (vt+0x10)",
                ));
            }
            data.land_40 = Some(land);
        }
        land
    };
    load_form_from_file(world, land, cx)?;
    Ok(1)
}

/// IDA: 0044E2A5, 0044DCF0's PGRD arm. No current cell warns "PathGrid not associated with cell." and returns 0. The cell's +0x44 (004AF170), or a new grid (004E6070), +0x20 = the cell (004A6D70), 004C9B10 stores it in the cell, deleting a different old one (it never differs here), 00447050. Returns 1. Completion: 100%.
pub fn path_grid_arm(
    world: &mut World,
    cx: &mut FormLoadCx<'_, '_>,
    _existing: Option<Entity>,
    _first: bool,
) -> Result<u32, GameError> {
    let Some(cell) = world.resource::<DataLoadGlobals>().current_cell_b33a9c else {
        cx.file
            .services
            .warn("PathGrid not associated with cell.".to_owned());
        return Ok(0);
    };
    let current = world
        .get::<TesObjectCell>(cell)
        .expect("a cell")
        .path_grid_44;
    let grid = match current {
        Some(grid) => grid,
        None => construct_path_grid(world)?,
    };
    world.get_mut::<TesPathGrid>(grid).expect("a grid").cell_20 = Some(cell);
    world
        .get_mut::<TesObjectCell>(cell)
        .expect("a cell")
        .path_grid_44 = Some(grid);
    load_form_from_file(world, grid, cx)?;
    Ok(1)
}
