//! TESForm's state as components. Original object offsets are in the field names.

use bevy_ecs::prelude::*;

use super::forms_references::form_flags;
use crate::tes::actor_enable::{TYPE_ARMOR, TYPE_CLOTHING};
use crate::tes::data_handler::{
    DataHandler, DataHandlerGlobals, FormsById, TES_FORM_VTABLE, allocate_form_id, release_form_id,
};
use crate::tes::form_arms::{FormLoadCx, class_of};
use crate::tes::form_components::{TesBipedModelForm, TesValueForm, covers};
use crate::tes::forms_init::form_type;
use crate::tes::tes::GameThreadLocals;
use crate::tes::tes_file::{FORM_TYPE_TABLE, TesFileId};
use oblivion_core::error::GameError;

/// IDA: 0046C470 TESForm's identity: +0x04 form type, +0x08 flags, +0x0C form ID.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct FormIdentity {
    pub form_type_04: u8,
    pub flags_08: u32,
    pub form_id_0c: u32,
}

/// IDA: the most-derived vtable the constructor stores at +0x00. Evidence only, never called; the
/// rows assert it. Completion: 100%.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct OriginalVtable(pub u32);

/// IDA: TESForm +0x10 the files that touched the form, as mod indexes, in list order.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct ModReferences(pub Vec<u8>);

impl ModReferences {
    /// IDA: 0046B6CC TESForm_UpdateModReferenceList(file): a master clears the list and is
    /// prepended; a file already present returns; otherwise it goes after the last entry.
    /// Completion: 90%.
    /// TODO: [ ] The ghost-parent test (M4). `update_mod_reference_list` stops on a ghost file.
    pub fn update(&mut self, mod_index: u8, is_master: bool) {
        if is_master {
            self.0.clear();
            self.0.push(mod_index);
        } else if !self.0.contains(&mod_index) {
            self.0.push(mod_index);
        }
    }
}

impl ModReferences {
    /// IDA: 0046B6DD..0046B74B UpdateModReferenceList(null): remove the last non-empty entry (the head when none): its next node is pulled in, or its item cleared.
    /// Completion: 100%.
    pub fn remove_last(&mut self) {
        self.0.pop();
    }
}

/// IDA: 0044DD69 the flag that keeps a form's map entry, and marks a partial form.
pub const FORM_FLAG_PARTIAL: u32 = 0x4000;

/// IDA: the form flag TESForm vt+0x78 (004D7030) tests: 0x400.
const FORM_FLAG_PERSISTENT: u32 = 0x400;

// IDA: 004D7030 TESForm_IsPersistent (vt+0x78): the flag 0x400.
pub(in crate::tes) fn persistent(world: &World, form: Entity) -> bool {
    world
        .get::<FormIdentity>(form)
        .is_some_and(|id| id.flags_08 & FORM_FLAG_PERSISTENT != 0)
}

/// IDA: 00B0614C the altered-form NiTPrimitiveArray: +0x04 data, +0x0C size, +0x10 count.
#[derive(Resource, Debug, Default)]
pub struct AlteredForms {
    pub data: Vec<Option<Entity>>,
    pub count_10: u32,
}

/// IDA: 0046C640 TESForm_SetAltered(1): a form without bit 1 or 0x4000 not in the array joins it (00449070: the first empty slot, else appended), then bit 1. Completion: 100% of the argument 1.
pub fn set_altered(world: &mut World, form: Entity) {
    let flags = form_flags(world, form);
    if flags & 2 == 0 && flags & 0x4000 == 0 {
        let mut array = world.get_resource_or_init::<AlteredForms>();
        if !array.data.contains(&Some(form)) {
            match array.data.iter().position(Option::is_none) {
                Some(slot) => array.data[slot] = Some(form),
                None => array.data.push(Some(form)),
            }
            array.count_10 += 1;
        }
    }
    world
        .get_mut::<FormIdentity>(form)
        .expect("a form")
        .flags_08 |= 2;
}

/// IDA: 0046B680 TESForm_GetModFileDataAtIndex(index): item `index` of +0x10, or with -1 the last nonzero one, by the
/// compiled array since this list holds mod indices; its file from DataHandler +0x8D4. Completion: 100%.
#[must_use]
pub fn mod_file_at_index(
    world: &World,
    form: Entity,
    index: i32,
) -> Option<crate::tes::tes_file::TesFileId> {
    let mods = &world.get::<ModReferences>(form)?.0;
    let mod_index = if index == -1 {
        *mods.last()?
    } else {
        *mods.get(usize::try_from(index).ok()?)?
    };
    *world
        .resource::<crate::tes::data_handler::DataHandler>()
        .mods_by_id_8d4
        .get(usize::from(mod_index))?
}

// IDA: 004691B0: the biped part of an ARMO (+0x64) or a CLOT (+0x5C), else null; its +0x04 is the slot mask.
pub(crate) fn biped_mask(world: &World, form: Entity) -> Option<u16> {
    matches!(form_type(world, form), TYPE_ARMOR | TYPE_CLOTHING)
        .then(|| world.get::<TesBipedModelForm>(form))
        .flatten()
        .map(|biped| biped.slot_mask_04)
}

/// IDA: 004691B0 then 00468FF0(biped, slot, 0): a form's biped covers the slot. Completion: 100%.
pub(crate) fn biped_covers(world: &World, form: Entity, slot: u32) -> bool {
    biped_mask(world, form).is_some_and(|mask| covers(mask, slot))
}

// IDA: 00470520 TESValueForm vt+0x10 (+0x04), as an unsigned through 005479B0 (its double stored a float).
pub(crate) fn clothing_value(world: &World, form: Entity) -> f32 {
    let value = world
        .get::<TesValueForm>(form)
        .map_or(-1, |value| value.value_04);
    f64::from(value as u32) as f32
}

/// IDA: 0046C2F0 TESForm_SetFormID(key, 1), without the 00446B40 hand-back: the C++ startup callers pass release = 1, and the paths differ only for an old ID with high byte 0xFF, which none of these callers has. The
/// other callers use `set_form_id_release`. Completion: 100% of its callers' arms.
pub fn set_form_id(world: &mut World, form: Entity, key: u32) {
    let identity = *world
        .get::<FormIdentity>(form)
        .expect("a form has an identity");
    let old = identity.form_id_0c;
    if key == old {
        return;
    }
    // 0046C30F..0046C318 forms with flag 0x4000 keep their map entries.
    if identity.flags_08 & FORM_FLAG_PARTIAL == 0 {
        let mut forms = world.resource_mut::<FormsById>();
        if old != 0 {
            forms.remove(old);
        }
        if key != 0 {
            forms.set_at(key, form);
        }
    }
    world
        .get_mut::<FormIdentity>(form)
        .expect("checked above")
        .form_id_0c = key;
}

/// IDA: 0046C300 TESForm_SetFormID(key, release): an equal key returns; without flag 0x4000 the old key leaves the map, a set `release` hands a nonzero old ID back (00446B40) and a nonzero key is inserted; then
/// +0x0C = key. Completion: 100%.
pub fn set_form_id_release(
    world: &mut World,
    form: Entity,
    key: u32,
    release: bool,
) -> Result<(), GameError> {
    let identity = *world
        .get::<FormIdentity>(form)
        .expect("a form has an identity");
    let old = identity.form_id_0c;
    if key == old {
        return Ok(());
    }
    // 0046C30F..0046C318 forms with flag 0x4000 keep their map entries.
    if identity.flags_08 & FORM_FLAG_PARTIAL == 0 {
        let mut forms = world.resource_mut::<FormsById>();
        if old != 0 {
            forms.remove(old);
        }
        // 0046C329..0046C33E 00446B40(g_dataHandler, old) for a nonzero old ID when `release` is set.
        if release && old != 0 {
            release_form_id(world.get_resource_mut::<DataHandler>().as_deref_mut(), old)?;
        }
        if key != 0 {
            world.resource_mut::<FormsById>().set_at(key, form);
        }
    }
    world
        .get_mut::<FormIdentity>(form)
        .expect("checked above")
        .form_id_0c = key;
    Ok(())
}

/// IDA: 0046B6C0 with the file in hand, for the 0044F0C0 loop, where the store is out of `DataHandler` (`data_load::with_file_store`). Completion: 100% of the modelled files.
pub fn update_mod_references_from(
    world: &mut World,
    form: Entity,
    file: &crate::tes::data::tes_file::TesFileData,
) -> Result<(), GameError> {
    if file.ghost_parent_04 != 0 {
        return Err(GameError::unclassified(
            0x0046B6CE,
            "0046B6CE: ghost TESFiles are not modelled by the host",
        ));
    }
    world
        .get_mut::<ModReferences>(form)
        .expect("a form has a mod reference list")
        .update(file.mod_index_400, file.is_master());
    Ok(())
}

/// IDA: 0046B6C0 TESForm_UpdateModReferenceList(file) (__thiscall, retn 4): a file is first replaced by its root ghost parent (0044FB90, TESFile +0x04; no ghost file is modelled, so a nonzero +0x04 stops); a master
/// (0044FAA0: +0x3DC bit 0) clears the nodes (004526E0) and is prepended; another file is skipped when present, else appended. A null file removes the last non-empty entry. Completion: 100% of the modelled files.
/// TODO: [ ] Ghost parents (M4).
pub fn update_mod_reference_list(
    world: &mut World,
    form: Entity,
    file: Option<TesFileId>,
) -> Result<(), GameError> {
    let Some(file) = file else {
        world
            .get_mut::<ModReferences>(form)
            .expect("a form has a mod reference list")
            .remove_last();
        return Ok(());
    };
    let data = world.resource::<DataHandler>();
    let file = data.file_store.get(file).ok_or(GameError::unclassified(
        0x0046B6C0,
        "0046B6C0: a destroyed TESFile",
    ))?;
    if file.ghost_parent_04 != 0 {
        return Err(GameError::unclassified(
            0x0046B6CE,
            "0046B6CE: ghost TESFiles are not modelled by the host",
        ));
    }
    let (mod_index, is_master) = (file.mod_index_400, file.flags_3dc & 1 != 0);
    world
        .get_mut::<ModReferences>(form)
        .expect("a form has a mod reference list")
        .update(mod_index, is_master);
    Ok(())
}

// IDA: 0046C486..0046C550 the first-form validation of the form-type table: each entry's type byte must equal its index and its fourcc must be unique ("formEnumString[ %d ]..."
// warnings; "You must fix the problems in TESForm.cpp" when any), then 00B33C10 = 1. Completion: 100% of the checks (the fourccs are the constant table's names).
fn validate_form_type_table(globals: &mut DataHandlerGlobals) {
    let mut failed = false;
    for (index, (kind, name)) in FORM_TYPE_TABLE.iter().enumerate() {
        if usize::from(*kind) != index {
            globals.warnings.push(format!(
                "formEnumString[ {index} ].cFormID in TESForm.cpp does not match its index"
            ));
            failed = true;
        }
        for (other, (_, other_name)) in FORM_TYPE_TABLE.iter().enumerate() {
            if other != index && record_fourcc(name) == record_fourcc(other_name) {
                globals.warnings.push(format!(
                    "formEnumString[ {index} ] and formEnumString[ {other} ] share {name}"
                ));
                failed = true;
            }
        }
    }
    if failed {
        globals
            .warnings
            .push("You must fix the problems in TESForm.cpp".to_owned());
    }
    globals.form_table_validated_b33c10 = true;
}

// IDA: 0046C460 TESForm_Constructor on a new entity (the head components, which the derived constructors then rewrite): 0046C470 vtable A3BE3C; 0046C47A/0046C47D +0x10 = +0x14 = 0; 0046C469..0046C550 the
// once-per-process validation; 0046C557..0046C562 +0x04 = 0, +0x08 = 8, +0x0C = 0; 0046C569..0046C5FD with g_dataHandler set and the TLS +0x184 byte clear, a new form ID from 00448D40, TESForm_UpdateModReferenceList
// (+0x8C4) and an ID at or below 0x7FF moving to 0x800; 0046C604..0046C612 a nonzero ID is registered in g_formsByID. Completion: 100% of the store order.
pub fn construct_tes_form(world: &mut World) -> Result<Entity, GameError> {
    let validated = world
        .resource::<DataHandlerGlobals>()
        .form_table_validated_b33c10;
    if !validated {
        validate_form_type_table(&mut world.resource_mut::<DataHandlerGlobals>());
    }
    let form = world
        .spawn((
            FormIdentity {
                form_type_04: 0,
                flags_08: 8,
                form_id_0c: 0,
            },
            OriginalVtable(TES_FORM_VTABLE),
            ModReferences::default(),
        ))
        .id();
    if world.contains_resource::<DataHandler>()
        && world.resource::<GameThreadLocals>().byte_184 == 0
    {
        // 0046C58F 00448D40 on the DataHandler (ecx = g_dataHandler).
        let id = world.resource_scope(|world, mut data: Mut<DataHandler>| {
            allocate_form_id(
                data.next_form_id_8c0_mut(),
                world.resource::<FormsById>(),
                &world.resource::<DataHandlerGlobals>().save_load_form_ids,
                world
                    .get_resource::<crate::tes::tes::TesGlobals>()
                    .and_then(|globals| globals.save_load_game.as_ref())
                    .map(|game| &game.changes_00),
            )
        });
        set_identity_id(world, form, id);
        // 0046C597..0046C5AA TESForm_UpdateModReferenceList(+0x8C4) when set.
        if let Some(file) = world.resource::<DataHandler>().active_file_8c4 {
            update_mod_reference_list(world, form, Some(file))?;
        }
        // 0046C5AF..0046C5FD an ID at or below 0x7FF (the 0x800 compare that follows can never match) moves to 0x800: without flag 0x4000 the old key leaves the map, 00446B40 gives it back and 0x800 is inserted.
        let identity = *world.get::<FormIdentity>(form).expect("just spawned");
        if identity.form_id_0c <= 0x7FF {
            if identity.flags_08 & FORM_FLAG_PARTIAL == 0 {
                if identity.form_id_0c != 0 {
                    world
                        .resource_mut::<FormsById>()
                        .remove(identity.form_id_0c);
                    release_form_id(
                        Some(&mut world.resource_mut::<DataHandler>()),
                        identity.form_id_0c,
                    )?;
                }
                world.resource_mut::<FormsById>().set_at(0x800, form);
            }
            set_identity_id(world, form, 0x800);
        }
    }
    // 0046C604..0046C612 a nonzero form ID is registered.
    let id = world
        .get::<FormIdentity>(form)
        .expect("just spawned")
        .form_id_0c;
    if id != 0 {
        world.resource_mut::<FormsById>().set_at(id, form);
    }
    Ok(form)
}

// IDA: N/A a store to +0x0C.
fn set_identity_id(world: &mut World, form: Entity, id: u32) {
    world
        .get_mut::<FormIdentity>(form)
        .expect("just spawned")
        .form_id_0c = id;
}

/// IDA: 0046A9A0 TESForm_SetMasterFlag(set): +0x08 bit 0. Completion: 100%.
pub fn set_master_flag(world: &mut World, form: Entity, set: bool) {
    let mut identity = world.get_mut::<FormIdentity>(form).expect("a form");
    if set {
        identity.flags_08 |= 1;
    } else {
        identity.flags_08 &= !1;
    }
}

/// IDA: 00447050 TESForm_LoadFromFile(form, file) (__cdecl): bit 0 of +0x08 before vt+0x1C; 0046A9A0(that bit ? 1 : 0044FAA0 of the file) after it; an Active file then calls vt+0x90(1). Returns LoadForm's byte. Completion: 100%, with vt+0x90 a host stop.
/// TODO: [ ] 00447091 vt+0x90(1) and the 00B06150 array, when an Active file is reachable.
pub fn load_form_from_file(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<bool, GameError> {
    let master = world.get::<FormIdentity>(form).expect("a form").flags_08 & 1 != 0;
    let class = class_of(world, form)?;
    let result = (class.load_form)(world, form, cx)?;
    let file = cx.file.file()?;
    let (is_master, is_active) = (file.is_master(), file.is_active());
    set_master_flag(world, form, master || is_master);
    if is_active {
        return Err(GameError::unclassified(
            0x00447091,
            "00447091: vt+0x90(1) for the Active file (not at startup)",
        ));
    }
    Ok(result)
}

/// IDA: 004A2A30 TESForm_GetIOStage: the IO stage by a base's type: 2 for the small-item types (0x13..0x16, 0x19, 0x1B, 0x21, 0x22, 0x26..0x28, 0x2A), 3 for NPCs and creatures, otherwise 1. Completion: 100%.
#[must_use]
pub const fn model_stage(base_type: u8) -> u32 {
    match base_type {
        0x13..=0x16 | 0x19 | 0x1B | 0x21 | 0x22 | 0x26..=0x28 | 0x2A => 2,
        0x23 | 0x24 => 3,
        _ => 1,
    }
}

// IDA: 0046C4B0 builds the fourcc from the four name bytes. Completion: 100%.
#[must_use]
pub fn record_fourcc(name: &str) -> u32 {
    let bytes = name.as_bytes();
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}
