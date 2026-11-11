//! Phase 12b: Script (SCPT 0x50, 004FBAA0) with its LoadForm 004FC0D0, the reset pair 004FA510 / 004FC940, and the SCRI resolution every scriptable form runs while loading: 0046F020 with 0046BB20. Evidence: objdump of each address named here; P12-RUN-003 samples.

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;

use super::form::{FORM_FLAG_PARTIAL, FormIdentity, OriginalVtable};
use super::form_arms::{
    FormClass, FormLoadCx, default_editor_id, editor_id_of, reset_18_components,
};
use super::form_components::{FormRef, TesScriptableForm, reset_form_components};
use super::forms_misc::set_initialized;
use crate::bs::string::BsStringT;
use crate::tes::data_handler::{DataHandler, FormsById, release_form_id};
use crate::tes::data_load::{
    DataLoadServices, construct_script, initialize_form_from_current_record,
};
use crate::tes::form::construct_tes_form;
use crate::tes::forms_init::{QuestDelayGlobals, form_id, initialized, lookup, name_of};
use crate::tes::inventory::InventoryGlobals;
use crate::tes::save_load_game::remove_form_changes;
use crate::tes::tes::TesGlobals;
use crate::tes::tes_file::{TesFileId, map_form_id_in, map_id};
use oblivion_core::{crt, x87};

/// IDA: the form type of Script.
pub const FORM_TYPE_SCRIPT: u8 = 0x0D;
/// IDA: 004FBAD1 the Script vtable.
pub const SCRIPT_VTABLE: u32 = 0x00A4_9DA4;

// IDA: the chunks 004FC0D0 dispatches on.
const EDID: u32 = 0x4449_4445;
const RNAM: u32 = 0x4D41_4E52;
const SCDA: u32 = 0x4144_4353;
const SCHR: u32 = 0x5248_4353;
const SCRO: u32 = 0x4F52_4353;
const SCRV: u32 = 0x5652_4353;
const SCVR: u32 = 0x5256_4353;
const SLSD: u32 = 0x4453_4C53;

/// IDA: a 0x10 block of Script +0x40 (004FC25D): +0x00 BSStringT, cleared and never set here, +0x08 the SCRO form ID, +0x0C the SCRV variable.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScriptReference {
    pub name_00: BsStringT,
    pub form_08: u32,
    pub variable_0c: u32,
    /// IDA: +0x08 after 004FBB60, which stores the form there: `Some` once it has run over a nonzero word.
    pub resolved_08: Option<Option<Entity>>,
}

/// IDA: an 0x18 block of 004FA910's list: +0x00 the variable's dword, +0x08 its qword, +0x10 its byte.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScriptEventVariable {
    pub id_00: u32,
    pub value_08: u64,
    pub byte_10: u8,
}

/// IDA: an 8-byte entry of ScriptEventList +0x08 (004FB510): +0x00 the ID, +0x04 the mask.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScriptEvent {
    pub id_00: u32,
    pub mask_04: u32,
}

/// IDA: 004FBDC0 ScriptEventList (0x14): +0x00 the script, +0x08 the event list, null until 004FB510 makes it, +0x0C the variable list; +0x04 and +0x10 stay clear.
#[derive(Clone, Debug, PartialEq)]
pub struct ScriptEventList {
    pub script_00: Entity,
    /// +0x04, whose bit 0 Enable and Disable keep for a run with no reference.
    pub flags_04: u32,
    pub events_08: Option<Vec<ScriptEvent>>,
    pub variables_0c: Vec<ScriptEventVariable>,
}

/// IDA: 00517A80 a 0x20 block of Script +0x48: +0x00 the SLSD bytes, of which 00517A80 clears +0x00, +0x08..+0x0F and +0x10 (the rest is FormHeap, host zero), +0x18 BSStringT name (SCVR).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptVariable {
    pub data_00: [u8; 0x18],
    pub name_18: BsStringT,
}

/// IDA: 004FBAA0 Script (0x50): +0x18 the SCHR header, +0x2C source text (null, SCTX has no arm), +0x30 the compiled data (SCDA, FormHeap), +0x34..+0x3F floats 0.0, +0x40/+0x44 the reference list, +0x48/+0x4C the variable list.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct Script {
    pub header_18: [u8; 0x14],
    pub text_2c: u32,
    pub data_30: Option<Vec<u8>>,
    pub floats_34: [f32; 3],
    pub references_40: Vec<ScriptReference>,
    pub variables_48: Vec<ScriptVariable>,
}

/// IDA: the two script globals these functions touch: 00B361AC, which the constructor and reset clear, and 00B361B0, which 004FC730 clears when it holds the form.
#[derive(Resource, Debug, Default)]
pub struct ScriptGlobals {
    pub byte_b361ac: u8,
    pub current_b361b0: Option<Entity>,
}

// IDA: 004FBAF8 / 004FA529 `mov ds:0xb361ac, al`.
fn clear_script_byte(world: &mut World) {
    world.init_resource::<ScriptGlobals>();
    world.resource_mut::<ScriptGlobals>().byte_b361ac = 0;
}

/// IDA: 004FBAA0 Script_Constructor: 0046C460, the vtable, the two lists, +0x18..+0x28, +0x30, +0x2C, 00B361AC, the floats, +0x04 = 0x0D, then 0046C2E0, which type 0x0D skips. The original also runs it inside another block (a quest stage item's +0x0C, the 00530E1A stack local, the static 00B36530, 009E4220); here such a Script is an entity of its own, which its owner holds and `destroy_script_in_place` removes. Completion: 100% of the store order.
pub fn construct_script_in_place(world: &mut World) -> Result<Entity, GameError> {
    let form = construct_tes_form(world)?;
    world
        .entity_mut(form)
        .insert((OriginalVtable(SCRIPT_VTABLE), Script::default()));
    clear_script_byte(world);
    world
        .get_mut::<FormIdentity>(form)
        .expect("a form")
        .form_type_04 = FORM_TYPE_SCRIPT;
    reset_form_components(world, form, &[])?;
    Ok(form)
}

// IDA: 004FA510 Script vt+0x14: the SCHR words, +0x30, +0x2C, 00B361AC, the floats, +0x04 = 0x0D, then 0046C2E0. Completion: 100%.
fn reset_script(world: &mut World, form: Entity) -> Result<(), GameError> {
    {
        let mut script = world.get_mut::<Script>(form).expect("a script");
        script.header_18 = [0; 0x14];
        script.data_30 = None;
        script.text_2c = 0;
        script.floats_34 = [0.0; 3];
    }
    clear_script_byte(world);
    world
        .get_mut::<FormIdentity>(form)
        .expect("a form")
        .form_type_04 = FORM_TYPE_SCRIPT;
    reset_form_components(world, form, &[])
}

// IDA: 004FC940 Script vt+0x18: frees +0x30 and +0x2C (their words stay until vt+0x14, which 0044DE0F runs next; the host drops the block at once), 004FC6C0 the variables, 004FC730 the references with its 00B361B0 test, then 0046C2F0, which type 0x0D skips. Completion: 100%.
fn destroy_script(world: &mut World, form: Entity) -> Result<(), GameError> {
    {
        let mut script = world.get_mut::<Script>(form).expect("a script");
        script.data_30 = None;
        script.variables_48.clear();
    }
    world.init_resource::<ScriptGlobals>();
    {
        let mut globals = world.resource_mut::<ScriptGlobals>();
        if globals.current_b361b0 == Some(form) {
            globals.current_b361b0 = None;
        }
    }
    world
        .get_mut::<Script>(form)
        .expect("a script")
        .references_40
        .clear();
    reset_18_components(world, form)
}

// IDA: 004FC0D0 Script vt+0x1C LoadForm. The record must be SCPT (else 0); 00451210; then the chunks while nonzero: EDID (alloca, size 0x200, vt+0xD8 006FDFF0 keeps nothing); SCDA (a cleared FormHeap block of the chunk's size, read whole, an earlier block leaks); SLSD (00517A80, 00517A50 reads 0x18 when the chunk type is SLSD, 0067B1E0 onto +0x48, and it becomes the current variable); SCVR (with a current variable, the text with size 0x200 into its +0x18); RNAM (4 bytes, dropped); SCHR (the whole chunk into +0x18, no limit); SCRO / SCRV (a cleared 0x10 block, +0x08 / +0x0C from 4 bytes, 0067B1E0 onto +0x40). A null +0x30 then warns. Returns 1. Completion: 100%.
// TODO: [ ] 004FC2F8 an SCHR past 0x14 bytes writes on over +0x2C and +0x30 in the original: a host stop here.
fn load_script_form(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<bool, GameError> {
    if u32::from(FORM_TYPE_SCRIPT) != cx.file.record_form_type()? {
        return Ok(false);
    }
    initialize_form_from_current_record(world, form, cx)?;
    let mut current: Option<usize> = None;
    let mut chunk = cx.file.chunk_type()?;
    while chunk != 0 {
        match chunk {
            EDID => {
                cx.file.read_text(0x200)?;
            }
            SCDA => {
                let size = cx.file.chunk_size()?;
                let mut data = vec![0_u8; size as usize];
                cx.file.read_chunk(&mut data, 0)?;
                world.get_mut::<Script>(form).expect("a script").data_30 = Some(data);
            }
            SLSD => {
                let mut variable = ScriptVariable {
                    data_00: [0; 0x18],
                    name_18: BsStringT::default(),
                };
                cx.file.read_chunk(&mut variable.data_00, 0x18)?;
                let mut script = world.get_mut::<Script>(form).expect("a script");
                script.variables_48.push(variable);
                current = Some(script.variables_48.len() - 1);
            }
            SCVR => {
                if let Some(index) = current {
                    let text = cx.file.read_text(0x200)?;
                    world
                        .get_mut::<Script>(form)
                        .expect("a script")
                        .variables_48[index]
                        .name_18
                        .set(Some(&text), 0);
                }
            }
            RNAM => {
                cx.file.read_dword(0)?;
            }
            SCHR => {
                if cx.file.chunk_size()? > 0x14 {
                    return Err(GameError::unclassified(
                        0x004FC2F8,
                        "004FC2F8: an SCHR chunk past 0x14 bytes overruns +0x2C",
                    ));
                }
                let mut script = world.get_mut::<Script>(form).expect("a script");
                cx.file.read_chunk(&mut script.header_18, 0)?;
            }
            SCRO | SCRV => {
                let (value, _) = cx.file.read_dword(0)?;
                let mut reference = ScriptReference::default();
                if chunk == SCRO {
                    reference.form_08 = value;
                } else {
                    reference.variable_0c = value;
                }
                world
                    .get_mut::<Script>(form)
                    .expect("a script")
                    .references_40
                    .push(reference);
            }
            _ => {}
        }
        if !cx.file.advance_chunk()? {
            break;
        }
        chunk = cx.file.chunk_type()?;
    }
    // 004FC319..004FC33A A49FB8 with vt+0xD4 and the file's +0x1C.
    if world
        .get::<Script>(form)
        .expect("a script")
        .data_30
        .is_none()
    {
        let name = latin1(&editor_id_of(world, form));
        let file = cx.file.file()?.name_1c.clone();
        cx.file.services.warn(format!(
            "Script '{name}' in file '{file}' has not been compiled.\r\n"
        ));
    }
    Ok(true)
}

/// IDA: 004F9DF0 Script_SetCompiledData(size, source): the old block freed, +0x30 = 0, +0x20 (the header's +0x08) = size, and for a nonzero size a new cleared block with the bytes copied. Completion: 100%.
pub fn set_compiled_data(script: &mut Script, source: &[u8]) {
    script.data_30 = None;
    script.header_18[8..12].copy_from_slice(&(source.len() as u32).to_le_bytes());
    if !source.is_empty() {
        script.data_30 = Some(source.to_vec());
    }
}

// IDA: 004FC730: 00B361B0 cleared when it holds this script, then the +0x40 list freed.
fn clear_script_references(world: &mut World, form: Entity) {
    world.init_resource::<ScriptGlobals>();
    {
        let mut globals = world.resource_mut::<ScriptGlobals>();
        if globals.current_b361b0 == Some(form) {
            globals.current_b361b0 = None;
        }
    }
    world
        .get_mut::<Script>(form)
        .expect("a script")
        .references_40
        .clear();
}

/// IDA: 004FC7A0 Script_CopyFrom(source), as 00530E31 calls it with a fresh Script. The +0x18..+0x2B header copied and +0x18 cleared; 004F9DF0 with the copied size and the source's data; 004FC730; 004FC040 (the source's list is empty, so nothing); 004FC6C0; the source's null +0x2C frees and clears this one's; 0046AB80 with the source's bit 3. A source holding references, variables or text is a host stop: no caller reaches one yet. Completion: 100% of the arms a fresh source takes.
/// TODO: [ ] 004FC80A 004F9E50 and the 004FC040 copy, with a caller whose source holds them.
pub fn copy_script_from(world: &mut World, form: Entity, source: Entity) -> Result<(), GameError> {
    let from = world.get::<Script>(source).expect("a script").clone();
    if !from.references_40.is_empty() || !from.variables_48.is_empty() || from.text_2c != 0 {
        return Err(GameError::unclassified(
            0x004FC7F4,
            "004FC7F4: 004FC7A0 from a Script holding references, variables or text",
        ));
    }
    let bit = world.get::<FormIdentity>(source).expect("a form").flags_08 & 8 != 0;
    {
        let mut script = world.get_mut::<Script>(form).expect("a script");
        script.header_18 = from.header_18;
        script.header_18[..4].fill(0);
        set_compiled_data(&mut script, from.data_30.as_deref().unwrap_or_default());
    }
    clear_script_references(world, form);
    {
        let mut script = world.get_mut::<Script>(form).expect("a script");
        script.variables_48.clear();
        script.text_2c = 0;
    }
    set_initialized(world, form, bit);
    Ok(())
}

/// IDA: 0046B2C0 TESForm_RemoveFromRegistry: without flag 0x4000, g_formsByID's RemoveAt, the form's slot in the 00B06150 array cleared (no form in this slice's load is ever in it: `clear_altered` stops first), and 00446B40 with a non-null DataHandler. Completion: 100% of the arms reached.
/// TODO: [ ] 0046B2DB the 00B06150 walk, with the array.
pub fn remove_form_from_registry(world: &mut World, form: Entity) -> Result<(), GameError> {
    let identity = *world.get::<FormIdentity>(form).expect("a form");
    if identity.flags_08 & FORM_FLAG_PARTIAL != 0 {
        return Ok(());
    }
    world
        .resource_mut::<FormsById>()
        .remove(identity.form_id_0c);
    if let Some(mut data) = world.get_resource_mut::<DataHandler>() {
        release_form_id(Some(&mut data), identity.form_id_0c)?;
    }
    Ok(())
}

/// IDA: 0046B590 TESForm_MarkAsTemporary: 0046B2C0; with a g_saveLoadGame, 0045B780(form, 0), which removes the ID from the +0x00 map (for 00B34428 and the dialogue scripts, a miss); flag 0x4000. Completion: 100%.
/// # Errors
/// As `remove_form_from_registry` and `remove_form_changes`.
pub fn mark_temporary(world: &mut World, form: Entity) -> Result<(), GameError> {
    remove_form_from_registry(world, form)?;
    if world
        .get_resource::<TesGlobals>()
        .is_some_and(|globals| globals.save_load_game.is_some())
    {
        remove_form_changes(world, form, false)?;
    }
    world
        .get_mut::<FormIdentity>(form)
        .expect("a form")
        .flags_08 |= FORM_FLAG_PARTIAL;
    Ok(())
}

/// IDA: 004FC980 Script's destructor with 0046C280 under it: the vtable, +0x30 and +0x2C freed, 004FC6C0, 004FC730, 0046C2F0 (type 0x0D skips), then 0046B2C0, the mod reference list, and without flag 0x4000 and with a g_saveLoadGame, 0045B780(form, 0) and 00453940(form), neither of which holds this form through the load. The entity goes where the original's block does. Completion: 100% of the arms reached.
pub fn destroy_script_in_place(world: &mut World, form: Entity) -> Result<(), GameError> {
    {
        let mut script = world.get_mut::<Script>(form).expect("a script");
        script.data_30 = None;
        script.text_2c = 0;
        script.variables_48.clear();
    }
    clear_script_references(world, form);
    remove_form_from_registry(world, form)?;
    // 0046C2B3..0046C2D6: without flag 0x4000 and with a g_saveLoadGame, 0045B780(form, 0) and 00453940(form): a walk of the +0x30 list for the form, whose hit is 0065C620. No host path adds to that list.
    let temporary =
        world.get::<FormIdentity>(form).expect("a form").flags_08 & FORM_FLAG_PARTIAL != 0;
    let game = world
        .get_resource::<TesGlobals>()
        .and_then(|globals| globals.save_load_game.as_ref());
    if !temporary && let Some(game) = game {
        let list_30 = game.fields_14[7];
        remove_form_changes(world, form, false)?;
        if list_30 != 0 {
            return Err(GameError::unclassified(
                0x00453950,
                "00453950: 00453940 walks a nonempty +0x30 list",
            ));
        }
    }
    world.despawn(form);
    Ok(())
}

/// IDA: N/A a byte string in a warning's text.
#[must_use]
pub fn latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|&byte| char::from(byte)).collect()
}

/// IDA: 0046F020 TESScriptableForm_ResolveScript(owner) (__thiscall on the component). A set +0x08 returns, leaving +0x04 as it is. A zero +0x04 only sets +0x08. Otherwise the ID maps through the owner's last mod reference (0046B680(-1), 0046BB20), 0046B250 looks it up and the Script cast goes into +0x04; a miss warns "Unable to find script (%08X) on owner object ..." with vt+0xD4 when 00404E50 says it's nonempty, else the form ID. Owner is never null here, so a3c4a0 has no arm. +0x08 = 1. Completion: 100%.
pub fn resolve_script(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<(), GameError> {
    let files = &*cx.file.files;
    let services = &mut *cx.file.services;
    resolve_script_with(
        world,
        form,
        &|world, id, file| map_form_id_in(world, files, id, file),
        &mut |text| services.warn(text),
    )
}

/// IDA: N/A 0046BB20 as a caller reaches it.
pub type FormIdMapper<'a> = dyn Fn(&World, u32, Option<TesFileId>) -> Result<u32, GameError> + 'a;

/// IDA: 0046F020's body, with 0046BB20 and 004A7A60 as the caller reaches them. Completion: 100%.
pub fn resolve_script_with(
    world: &mut World,
    form: Entity,
    map: &FormIdMapper<'_>,
    warn: &mut dyn FnMut(String),
) -> Result<(), GameError> {
    let component = *world
        .get::<TesScriptableForm>(form)
        .expect("a scriptable form");
    if component.resolved_08 != 0 {
        return Ok(());
    }
    if component.script_04.is_empty() {
        world
            .get_mut::<TesScriptableForm>(form)
            .expect("a scriptable form")
            .resolved_08 = 1;
        return Ok(());
    }
    let FormRef::Raw(raw) = component.script_04 else {
        return Err(GameError::unclassified(
            0x0046F02E,
            "0046F02E: an unresolved script slot holding a pointer",
        ));
    };
    let file = crate::tes::form::mod_file_at_index(world, form, -1);
    let id = map(world, raw, file)?;
    let found = world
        .resource::<FormsById>()
        .lookup(id)
        .filter(|&found| world.get::<Script>(found).is_some());
    world
        .get_mut::<TesScriptableForm>(form)
        .expect("a scriptable form")
        .script_04 = FormRef::Resolved(found);
    if found.is_none() {
        let name = editor_id_of(world, form);
        let text = if name.is_empty() {
            let owner = world.get::<FormIdentity>(form).expect("a form").form_id_0c;
            format!("Unable to find script ({id:08X}) on owner object ({owner:08X}).")
        } else {
            format!(
                "Unable to find script ({id:08X}) on owner object \"{}\".",
                latin1(&name)
            )
        };
        warn(text);
    }
    world
        .get_mut::<TesScriptableForm>(form)
        .expect("a scriptable form")
        .resolved_08 = 1;
    Ok(())
}

/// IDA: the SCRI arm every scriptable LoadForm shares (read here for each class): 4 bytes into a zeroed local, the store to the component's +0x04, then 0046F020. Completion: 100%.
pub fn load_script_reference(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<(), GameError> {
    let (id, _) = cx.file.read_dword(0)?;
    world
        .get_mut::<TesScriptableForm>(form)
        .expect("a scriptable form")
        .script_04 = FormRef::Raw(id);
    resolve_script(world, form, cx)
}

/// IDA: this family's class.
pub const CLASSES: &[FormClass] = &[FormClass {
    form_type: FORM_TYPE_SCRIPT,
    vtable: SCRIPT_VTABLE,
    construct: construct_script,
    load_form: load_script_form,
    reset_14: reset_script,
    reset_18: destroy_script,
    editor_id: default_editor_id,
    init_item: init_script_item,
    slots: &[],
    special_arm: None,
}];

// IDA: 004FBE00 Script_Run(script, reference, events, 0, 0), with the host's count of the runs past its size test.
pub(crate) fn run_script(
    world: &mut World,
    script: Entity,
    reference: Option<Entity>,
    events: Option<&mut ScriptEventList>,
    owner: Option<Entity>,
) -> Result<bool, GameError> {
    let body = world.get::<Script>(script).expect("a script");
    let compiled = u32::from_le_bytes(body.header_18[8..12].try_into().expect("four bytes"));
    if compiled > 4 {
        world
            .get_resource_or_init::<InventoryGlobals>()
            .script_runs_not_executed += 1;
    }
    crate::tes::script_vm::script_run(world, script, reference, events, owner)
}

/// IDA: 004FBB60 Script_InitItem(owner). The flag returns; a null owner returns. Each reference: a nonzero +0x08 maps and looks up, stored; a zero tests +0x0C. A zero answer unlinks the node (0065C620 with the node's block when a previous exists, otherwise the next is pulled in or the head item cleared), warns a49f48 for a form that wasn't found or a49ef0 otherwise, and clears the compiled size +0x20. A count that differs from +0x1C with a nonzero +0x20 warns a49e90 and clears it too. A quest script (+0x28 set) with 00B09E28 above 0.0 takes a delay: a zero counter first fills the table; a variable named fQuestDelayTime (004FAA90, _stricmp) gives +0x38 = 0.0; a zero low byte gives the setting; otherwise the table entries of the set bits add onto +0x38. Each counts. The flag is set. Completion: 100%.
pub fn init_script_form(
    world: &mut World,
    form: Entity,
    owner: Option<Entity>,
    services: &mut DataLoadServices<'_>,
) -> Result<(), GameError> {
    if initialized(world, form) {
        return Ok(());
    }
    let Some(owner) = owner else {
        return Ok(());
    };
    let file = crate::tes::form::mod_file_at_index(world, owner, -1);
    let mut references = std::mem::take(
        &mut world
            .get_mut::<Script>(form)
            .expect("a script")
            .references_40,
    );
    let mut count = 0_u32;
    let mut index = 0;
    while index < references.len() {
        let raw = references[index].form_08;
        let mapped = if raw != 0 {
            map_id(world, raw, file)?
        } else {
            0
        };
        let good = if raw != 0 {
            let found = lookup(world, mapped);
            references[index].resolved_08 = Some(found);
            found.is_some()
        } else {
            references[index].variable_0c != 0
        };
        if good {
            index += 1;
        } else {
            // The block unlinks by itself either way: 0065C620 compares the block pointer.
            let (name, id) = (name_of(world, form), form_id(world, form));
            references.remove(index);
            let text = if raw != 0 {
                format!(
                    "Could not find referenced object ({mapped:08X}) on Script ({id:08X}) '{name}'.  Script will not be executed."
                )
            } else {
                format!(
                    "Referenced object {count} on Script ({id:08X}) '{name}' is invalid.  Script will not be executed."
                )
            };
            crate::tes::forms_init::warn(world, services, text);
            world.get_mut::<Script>(form).expect("a script").header_18[8..12].fill(0);
        }
        count += 1;
    }
    let (expected, compiled, is_quest) = {
        let mut script = world.get_mut::<Script>(form).expect("a script");
        script.references_40 = references;
        (
            u32::from_le_bytes(script.header_18[4..8].try_into().expect("four bytes")),
            u32::from_le_bytes(script.header_18[8..12].try_into().expect("four bytes")),
            script.header_18[0x10] != 0,
        )
    };
    if count != expected && compiled != 0 {
        let text = format!(
            "Referenced object list on Script ({:08X}) '{}' is corrupt.  Expected {expected} objects, found {count}.",
            form_id(world, form),
            name_of(world, form)
        );
        crate::tes::forms_init::warn(world, services, text);
        world.get_mut::<Script>(form).expect("a script").header_18[8..12].fill(0);
    }
    if is_quest {
        apply_quest_delay(world, form);
    }
    set_initialized(world, form, true);
    Ok(())
}

// IDA: 004FBCB0..004FBD8C, at 24 bits: `fmul` by the qword 0.5 of A2FAA0 is exact, and each halving is stored as a float before the next. +0x38 is `floats_34[1]`. `fcom` with `test ah, 0x41`: a NaN or a value at or below 0.0 skips.
fn apply_quest_delay(world: &mut World, form: Entity) {
    world.init_resource::<QuestDelayGlobals>();
    let mut globals = world.resource_mut::<QuestDelayGlobals>();
    let setting = globals.quest_script_delay_time_b09e28;
    if setting.is_nan() || setting <= 0.0 {
        return;
    }
    if globals.counter_b361f4 == 0 {
        let mut value = setting;
        for slot in &mut globals.delays_b361d4 {
            value = (f64::from(value) * 0.5) as f32;
            *slot = value;
        }
    }
    let low = globals.counter_b361f4 & 0xFF;
    let delays = globals.delays_b361d4;
    globals.counter_b361f4 = globals.counter_b361f4.wrapping_add(1);
    let mut script = world.get_mut::<Script>(form).expect("a script");
    let named = script.variables_48.iter().any(|variable| {
        variable
            .name_18
            .data_00
            .as_deref()
            .is_some_and(|name| crt::stricmp(name, b"fQuestDelayTime") == 0)
    });
    if named {
        script.floats_34[1] = 0.0;
    } else if low == 0 {
        script.floats_34[1] = setting;
    } else {
        let mut bits = low;
        let mut at = 0;
        while bits != 0 {
            if bits & 1 != 0 {
                script.floats_34[1] += delays[at];
            }
            at += 1;
            bits >>= 1;
        }
    }
}

/// IDA: 004FBDB0 Script vt+0x6C: 004FBB60 with the stack word `push ecx` leaves as its owner: `this`. Completion: 100%.
pub fn init_script_item(
    world: &mut World,
    form: Entity,
    services: &mut DataLoadServices<'_>,
) -> Result<(), GameError> {
    init_script_form(world, form, Some(form), services)
}

/// IDA: 004FBDC0 ScriptEventList_Create(script): FormHeap(0x14): +0x04, +0x08, +0x0C, +0x00 and +0x10 clear; 004FA910's list of 0x18 copies of each +0x48 block, each its +0x10 byte, +0x00 dword and +0x08 qword, in list order; +0x00 the script; +0x0C the list. Completion: 100%.
#[must_use]
pub fn create_script_event_list(world: &World, script: Entity) -> ScriptEventList {
    let body = world.get::<Script>(script).expect("a script");
    let variables = body
        .variables_48
        .iter()
        .map(|variable| ScriptEventVariable {
            id_00: u32::from_le_bytes(variable.data_00[..4].try_into().expect("four bytes")),
            // 004FA963 `fld qword`, `fstp qword`, which quiets an sNaN.
            value_08: x87::fld_fstp_copy_f64(u64::from_le_bytes(
                variable.data_00[8..16].try_into().expect("eight bytes"),
            )),
            byte_10: variable.data_00[0x10],
        })
        .collect();
    ScriptEventList {
        flags_04: 0,
        script_00: script,
        events_08: None,
        variables_0c: variables,
    }
}
