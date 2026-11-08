//! Phase 12: the table `0044DCF0` dispatches through, one `FormClass` per form type, and what a class's virtual slots receive. Virtual dispatch on the original's vtable is a lookup on the form's type byte here.
//!
//! To add a class, append its `FormClass` to its family file's `CLASSES`. Nothing here or in `data_load` changes. `form_class` errors at test time if a type appears twice or disagrees with the exe's form type map.

use std::sync::LazyLock;

use bevy_ecs::prelude::*;
use recomp_core::reader::StreamError;

use super::data_load::DataLoadServices;
use super::form::FormIdentity;
use super::form_components::{
    ComponentSlot, TextureFileEntry, destroy_form_components, reset_form_components,
};
use super::form_type_map::{ConstructArm, construct_arm, form_type_row};
use super::tes_file::RecordReader;

/// IDA: N/A what a class's virtual slots and special arms receive: the current file and the host.
pub struct FormLoadCx<'a, 'b> {
    pub file: RecordReader<'a, 'b>,
    pub host: &'a mut dyn DataLoadHost,
}

/// IDA: a class's vt+0x1C LoadForm(file). `Ok(false)` is the original's 0 (the record isn't this class's, or the load failed). `Err` is a host stop.
pub type LoadFormFn = fn(&mut World, Entity, &mut FormLoadCx<'_, '_>) -> Result<bool, StreamError>;
/// IDA: a class's vt+0x14, vt+0x18 or vt+0x6C, on a form that exists.
pub type FormFn = fn(&mut World, Entity) -> Result<(), StreamError>;
/// IDA: a class's vt+0x6C InitItem, with the warnings and files of the post-load tail. `Err` is a host stop.
pub type InitItemFn = fn(&mut World, Entity, &mut DataLoadServices<'_>) -> Result<(), StreamError>;
/// IDA: a class's vt+0xD4 GetEditorID: the text, with an empty one standing for `Default` 00A2F7EC.
pub type EditorIdFn = fn(&World, Entity) -> Vec<u8>;
/// IDA: 0044DCF0's own arm for a type (`ConstructArm::Special`): its eax. `existing` is esi, `first` is n3c.
pub type SpecialArmFn =
    fn(&mut World, &mut FormLoadCx<'_, '_>, Option<Entity>, bool) -> Result<u32, StreamError>;
/// IDA: 00447DF0's arm for a type, past FormHeap: the constructor, under `construct_tes_form`.
pub type ConstructFn = fn(&mut World) -> Result<Entity, StreamError>;

/// IDA: one TESForm class, as 00447DF0 and 0044DCF0 reach it. `form_type_map.rs` holds what the exe says for each type.
#[derive(Clone, Copy)]
pub struct FormClass {
    pub form_type: u8,
    /// IDA: the most-derived vtable, checked against the map by a test.
    pub vtable: u32,
    /// IDA: 00447DF0's arm.
    pub construct: ConstructFn,
    /// IDA: vt+0x1C.
    pub load_form: LoadFormFn,
    /// IDA: vt+0x14. Use `reset_14_components` where the map says 0046C2E0.
    pub reset_14: FormFn,
    /// IDA: vt+0x18. Use `reset_18_components` where the map says 0046C2F0.
    pub reset_18: FormFn,
    /// IDA: vt+0xD4. Use `default_editor_id` where the map says 004129A0.
    pub editor_id: EditorIdFn,
    /// IDA: vt+0x6C. Use `no_init_item` where the map says 0060D0A0.
    pub init_item: InitItemFn,
    /// IDA: 00466E80's filled slots for this class, ascending.
    pub slots: &'static [ComponentSlot],
    /// IDA: 0044DCF0's own arm, for a type whose `construct_arm` is `Special`.
    pub special_arm: Option<SpecialArmFn>,
}

/// IDA: 0046C2E0 as vt+0x14: 0046B090 over the class's slots. Completion: 100% of the ported components.
pub fn reset_14_components(world: &mut World, form: Entity) -> Result<(), StreamError> {
    let slots = class_of(world, form)?.slots;
    reset_form_components(world, form, slots)
}

/// IDA: 0046C2F0 as vt+0x18: 0046B170 over the class's slots. Completion: 100% of the ported components.
pub fn reset_18_components(world: &mut World, form: Entity) -> Result<(), StreamError> {
    let slots = class_of(world, form)?.slots;
    destroy_form_components(world, form, slots)
}

/// IDA: 004129A0, vt+0xD4 of most classes: `Default` 00A2F7EC, an empty string (bytes read from the exe). Completion: 100%.
#[must_use]
pub fn default_editor_id(_world: &World, _form: Entity) -> Vec<u8> {
    Vec::new()
}

/// IDA: 0060D0A0, a bare `retn`. Completion: 100%.
pub fn no_init_item(
    _world: &mut World,
    _form: Entity,
    _services: &mut DataLoadServices<'_>,
) -> Result<(), StreamError> {
    Ok(())
}

/// IDA: N/A what 0044DCF0 does for a type whose class isn't ported yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AbsentArm {
    /// A host stop: `Err("Phase-12x pending: ...")`.
    Stop,
    /// Count it in `DataLoadGlobals::pending_arms` and return 1, as a test harness's skip. Never for a driver.
    CountAndSkip,
}

/// IDA: N/A the seams of the load, like the C++ `DataLoadServices`. Warnings go through `TesFileServices::warn`. Each default is the absent hook's C++ behaviour.
pub trait DataLoadHost {
    /// IDA: 00404EC0 DebugLog_Printf (D4: the host log).
    fn debug_log(&mut self, _text: String) {}
    /// IDA: 0057B950 LoadingMenu_SetSectionProgressIfReady(section, percent). The menu gate is the host's.
    fn section_progress(&mut self, _section: i32, _percent: f32) {}
    /// IDA: N/A a test's seam: false skips 0044F6CB 0044AC20. A driver leaves it.
    fn run_pre_load_pass(&mut self, _world: &mut World) -> bool {
        true
    }
    /// IDA: N/A a test's seam: false skips the passes of 0044F780..0044FA38, leaving TLS +0x184 and 00B33A9C cleared. A driver leaves it.
    fn run_post_load_passes(&mut self, _world: &mut World) -> bool {
        true
    }
    /// IDA: 0047DF80(0): `random_next` with the host's source, as `InventoryGlobals`'s rolls take it. `None` is a host stop: a driver holds one.
    fn random_next(&mut self) -> Option<i32> {
        None
    }
    /// IDA: 00431690(path, 0, 0x800), the whole file, as 004F28B0 reads it. `None` is no file.
    fn read_data_file(&mut self, _path: &str) -> Option<Vec<u8>> {
        None
    }
    /// IDA: N/A a type with no class yet.
    fn absent_arm(&mut self, _form_type: u32) -> AbsentArm {
        AbsentArm::Stop
    }
    /// IDA: N/A a test's filter: false treats a ported class as absent.
    fn arm_enabled(&mut self, _form_type: u32) -> bool {
        true
    }
    /// IDA: 0042DB10 ArchiveManager_FindFileEntryByType(1, folder hash, file hash, 0). The default is no archive.
    fn find_texture_entry(&mut self, _folder_hash: u64, _file_hash: u64) -> TextureFileEntry {
        None
    }
    /// IDA: 00431020 FileFinder_FindFile(path, 0, flags, mask) through g_FileFinder's vt+4: the status. 0 is none, as a null g_FileFinder gives at 004EB22A.
    fn find_file(&mut self, _path: &str, _flags: u32, _mask: i32) -> i32 {
        0
    }
    /// IDA: 0042C970 on 00B338E0 g_ArchiveList: a null list returns at 0042C97B, the default.
    fn apply_archive_cache_settings(&mut self) {}
    /// IDA: 00431970 FileFinder_FindMatchingFiles(pattern, source, mask, 0), the list head first, as 00435830 reaches it: `ArchiveManager::find_matching_files` where a host holds one. The default is no archive and no loose file.
    fn find_matching_files(&mut self, _pattern: &str, _source: &str, _mask: u32) -> Vec<String> {
        Vec::new()
    }
    /// IDA: 00B06310 and 00B04428 g_UseArchives, 0046D6BB's gates.
    fn texture_warning_gates(&self) -> (bool, bool) {
        (false, false)
    }
    /// IDA: 00494480 Startup_GetWarningsLogPath (the text only).
    fn warnings_log_path(&self) -> String {
        "Warnings.txt".to_owned()
    }
}

// IDA: N/A the table the families feed, indexed by the type byte. Built once from constants; nothing writes it after.
static CLASS_TABLE: LazyLock<[Option<&'static FormClass>; 0x45]> = LazyLock::new(|| {
    let mut table = [None; 0x45];
    for family in FAMILIES {
        for class in *family {
            let slot = &mut table[usize::from(class.form_type)];
            assert!(
                slot.is_none(),
                "form type {:02X} appears twice",
                class.form_type
            );
            *slot = Some(class);
        }
    }
    table
});

/// IDA: N/A every family, in the handoff's order. Each is one file holding `pub const CLASSES`.
pub const FAMILIES: &[&[FormClass]] = &[
    super::data_load::CLASSES,
    super::global::CLASSES,
    super::forms_items::CLASSES,
    super::forms_misc::CLASSES,
    super::forms_script::CLASSES,
    super::forms_magic::CLASSES,
    super::forms_actors::CLASSES,
    super::forms_ai::CLASSES,
    super::forms_dialogue::CLASSES,
    super::forms_environment::CLASSES,
    super::forms_region::CLASSES,
    super::forms_world::CLASSES,
    super::forms_references::CLASSES,
];

/// IDA: N/A the class of a form type, if ported.
#[must_use]
pub fn form_class(form_type: u32) -> Option<&'static FormClass> {
    CLASS_TABLE.get(form_type as usize).copied().flatten()
}

/// IDA: N/A a form's class: the dispatch a virtual call does. A form of a class not yet ported is a host stop.
pub fn class_of(world: &World, form: Entity) -> Result<&'static FormClass, StreamError> {
    let form_type = world
        .get::<FormIdentity>(form)
        .expect("a form")
        .form_type_04;
    form_class(u32::from(form_type)).ok_or(StreamError(
        "a virtual call on a form whose class isn't ported",
    ))
}

/// IDA: vt+0xD4 of any form; a class not yet ported answers as 004129A0 does. Completion: 100% of the ported classes.
#[must_use]
pub fn editor_id_of(world: &World, form: Entity) -> Vec<u8> {
    class_of(world, form).map_or_else(|_| Vec::new(), |class| (class.editor_id)(world, form))
}

/// IDA: N/A the checks a family can't make on its own. Called by a test.
pub fn check_classes_against_the_map() -> Result<(), String> {
    for family in FAMILIES {
        for class in *family {
            // 00447DF0's table has no arm for ACHR, ACRE, DIAL or INFO, which 0044DCF0 builds through 004DB260 or itself: such a class's `special_arm` is all that reaches it.
            let Some(row) = form_type_row(class.form_type) else {
                if construct_arm(u32::from(class.form_type)) == ConstructArm::Special
                    && class.special_arm.is_some()
                {
                    continue;
                }
                return Err(format!("{:02X}: not a factory type", class.form_type));
            };
            if (row.init_item_6c == 0x0060_D0A0)
                != std::ptr::fn_addr_eq(class.init_item, no_init_item as InitItemFn)
            {
                return Err(format!(
                    "{}: vt+0x6C is {:08X}, against the class's",
                    row.name, row.init_item_6c
                ));
            }
            if row.vtable != class.vtable {
                return Err(format!(
                    "{}: vtable {:08X}, the exe's {:08X}",
                    row.name, class.vtable, row.vtable
                ));
            }
        }
    }
    Ok(())
}
