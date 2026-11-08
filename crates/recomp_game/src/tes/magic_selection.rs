//! Phase 18: the player's selected magic item. WinMain's spell search (0040F00F..0040F025) and PlayerCharacter_SetSelectedMagicItem (00664700) with what a first start reaches below it.
//!
//! Evidence: disassembly; `run-i-phase18` and `run-j-phase18-cover`, the default path under Wine. An arm no run reached is a stop at its address.

use bevy_ecs::prelude::*;
use recomp_core::reader::StreamError;

use crate::bs::archive::ArchiveAccess;
use crate::bs::model_loader::{ModelRequest, load_model, model_map_lookup, release_task};
use crate::bs::reference_tasks::{QueuedReferenceClass, ReferenceTaskId};
use crate::bs::task_manager::TaskGlobals;

use super::actor::{MobileObject, TesObjectRefr};
use super::actor_process::MiddleHighProcess;
use super::data_handler::TesTexture;
use super::form_components::{FormRef, TesModel};
use super::forms_actors::ActorBaseValues;
use super::forms_items::set_change_flags;
use super::forms_magic::{EffectItemList, EffectSetting, MagicFormData};
use super::game_settings::GameSettingCollection;
use super::hud_main_menu::hud_set_selected_magic_icon_path;
use super::player::PlayerCharacter;
use super::tile::InterfaceCx;

/// IDA: the school actor value 0040F019 compares 00415360's answer with: 0x19, Restoration.
pub const SCHOOL_RESTORATION: i32 = 0x19;
// IDA: the flag of EffectSetting +0x58 that 004152C0 and 00419B30 skip.
const EFFECT_FLAG_NO_COST_PICK: u32 = 0x40_0000;
const EFFECT_FLAGS_SUMMON: u32 = 0x7_0000;
const EFFECT_CODE_TELE: u32 = 0x454C_4554;
const EFFECT_CODE_SEFF: u32 = 0x4646_4553;
// IDA: 006610F0's actor value 60, the telekinesis one 00662EBB sets.
const AV_TELEKINESIS: u8 = 60;
const STATS_MENU_ID: u32 = 1003;

// IDA: 00B37DD0, 00B37DD8, 00B38270, 00B37DE0, the settings 00548B50 reads, as floats.
fn cost_setting(world: &World, address: u32) -> f32 {
    f32::from_bits(
        world
            .resource::<GameSettingCollection>()
            .raw_at(address)
            .unwrap_or(0),
    )
}

// IDA: 00548B50(cost, area, duration, magnitude, on target). `pow` is __CIpow, whose double goes straight to a float; each other step is at 24 bits (D6).
fn base_effect_cost(
    world: &World,
    cost: f32,
    area: u32,
    duration: u32,
    magnitude: u32,
    on_target: bool,
) -> f32 {
    let mut area_factor = (area as i32) as f32 * cost_setting(world, 0x00B3_7DD0);
    if area_factor <= 1.0 {
        area_factor = 1.0;
    }
    let duration_factor = if duration != 0 {
        (duration as i32) as f32
    } else {
        1.0
    };
    let base = cost_setting(world, 0x00B3_7DD8) * cost * duration_factor;
    let magnitude_factor = if magnitude != 0 {
        f64::from((magnitude as i32) as f32).powf(f64::from(cost_setting(world, 0x00B3_8270)))
            as f32
    } else {
        1.0
    };
    let total = base * area_factor * magnitude_factor;
    let range_factor = if on_target {
        cost_setting(world, 0x00B3_7DE0)
    } else {
        1.0
    };
    range_factor * total
}

// IDA: 00413810: a cost below 0.0 is worked out and stored at the effect's +0x20. Flags 0x100, 0x80 and 0x200 of the setting drop magnitude, duration and area; a self range drops area too. The null setting faults.
pub(crate) fn effect_cost(
    world: &mut World,
    spell: Entity,
    index: usize,
) -> Result<f32, StreamError> {
    let list = world.get::<EffectItemList>(spell).expect("a magic item");
    let item = &list.items_04[index];
    if item.cost_20 >= 0.0 {
        return Ok(item.cost_20);
    }
    let setting = item
        .setting_1c
        .ok_or(StreamError("00413820: a null effect setting faults"))?;
    let data = world
        .get::<EffectSetting>(setting)
        .expect("a setting")
        .data_58;
    let flags = data.flags_58;
    let [_, magnitude, area, duration, range, _] = item.data_00;
    let magnitude = if flags & 0x100 != 0 { 0 } else { magnitude };
    let duration = if flags & 0x80 != 0 { 0 } else { duration };
    let area = if flags & 0x200 != 0 || range == 0 {
        0
    } else {
        area
    };
    let cost = base_effect_cost(
        world,
        data.base_cost_5c,
        area,
        duration,
        magnitude,
        range == 2,
    );
    world
        .get_mut::<EffectItemList>(spell)
        .expect("a magic item")
        .items_04[index]
        .cost_20 = cost;
    Ok(cost)
}

/// IDA: 00413890 with a null actor: the cost, truncated and stepped down below itself, and at least 1.0. Completion: 100% of the null-actor arm, the only one 004152C0 passes here.
/// # Errors
/// A null effect setting.
pub fn effect_magicka_cost(
    world: &mut World,
    spell: Entity,
    index: usize,
) -> Result<f32, StreamError> {
    let cost = effect_cost(world, spell, index)?;
    let whole = (cost as i32) as f32;
    let floor = if cost - whole < 0.0 {
        whole - 1.0
    } else {
        whole
    };
    Ok(if floor <= 1.0 { 1.0 } else { floor })
}

/// IDA: 004152C0 EffectItemList_FindHighestCostEffect(list, 3, false): strictly highest, skipping the setting flag 0x400000. Each cost goes through __ftol2. Completion: 100% of the any-range, no-area call.
/// # Errors
/// A null effect setting.
pub fn highest_cost_effect(world: &mut World, spell: Entity) -> Result<Option<usize>, StreamError> {
    let count = world
        .get::<EffectItemList>(spell)
        .expect("a magic item")
        .items_04
        .len();
    let mut best = -1_i32;
    let mut found = None;
    for index in 0..count {
        let cost = effect_magicka_cost(world, spell, index)? as i32;
        let setting = world
            .get::<EffectItemList>(spell)
            .expect("a magic item")
            .items_04[index]
            .setting_1c
            .ok_or(StreamError("00415303: a null effect setting faults"))?;
        let flags = world
            .get::<EffectSetting>(setting)
            .expect("a setting")
            .data_58
            .flags_58;
        if flags & EFFECT_FLAG_NO_COST_PICK == 0 && cost > best {
            found = Some(index);
            best = cost;
        }
    }
    Ok(found)
}

/// IDA: 00415360 MagicItem_GetHighestCostEffectSchoolActorValue: no effect answers -1; 00412F20, the script's school, else the setting's; 0041B770 maps 0..5 to 20..25 and anything else to -1. Completion: 100%.
/// # Errors
/// A null effect setting.
pub fn magic_item_school_actor_value(world: &mut World, spell: Entity) -> Result<i32, StreamError> {
    let Some(index) = highest_cost_effect(world, spell)? else {
        return Ok(-1);
    };
    let item = &world
        .get::<EffectItemList>(spell)
        .expect("a magic item")
        .items_04[index];
    let school = match &item.script_18 {
        Some(script) => script.school_04,
        None => {
            let setting = item
                .setting_1c
                .ok_or(StreamError("00412F2B: a null effect setting faults"))?;
            world
                .get::<EffectSetting>(setting)
                .expect("a setting")
                .data_58
                .school_64
        }
    };
    Ok(if school <= 5 { school as i32 + 20 } else { -1 })
}

/// IDA: 00419B30 with 0: the art setting of a plain spell, by highest range, then highest cost, past the flag 0x400000. Types 5, 7 and 1 look up POSN and DISE. Completion: 100% of a spell with a type other than those.
/// # Errors
/// Those types, a script effect, or a null setting.
pub fn magic_item_art_setting(
    world: &mut World,
    spell: Entity,
) -> Result<Option<Entity>, StreamError> {
    let kind = world
        .get::<MagicFormData>(spell)
        .map_or(0, |data| data.type_34);
    if matches!(kind, 1 | 5 | 7) {
        return Err(StreamError(
            "00419B55: a poison or disease's art setting, 00414EB0 and the collection",
        ));
    }
    let count = world
        .get::<EffectItemList>(spell)
        .expect("a magic item")
        .items_04
        .len();
    let mut best_cost = -1_i32;
    let mut best_range = 0_i32;
    let mut found = None;
    for index in 0..count {
        let cost = effect_magicka_cost(world, spell, index)? as i32;
        let item = &world
            .get::<EffectItemList>(spell)
            .expect("a magic item")
            .items_04[index];
        let range = item.data_00[4] as i32;
        let setting = item
            .setting_1c
            .ok_or(StreamError("00419BC1: a null effect setting faults"))?;
        let skipped = world
            .get::<EffectSetting>(setting)
            .expect("a setting")
            .data_58
            .flags_58
            & EFFECT_FLAG_NO_COST_PICK
            != 0;
        if !skipped && (range > best_range || (range == best_range && cost > best_cost)) {
            found = Some(index);
            best_cost = cost;
            best_range = range;
        }
    }
    let Some(index) = found else {
        return Ok(None);
    };
    let item = &world
        .get::<EffectItemList>(spell)
        .expect("a magic item")
        .items_04[index];
    if item.data_00[0] == EFFECT_CODE_SEFF {
        return Err(StreamError("00419C08: a script effect's art, 00412CB0"));
    }
    Ok(item.setting_1c)
}

/// IDA: what 0041A610 asks for when it isn't loaded: a QueuedMagicItem, 00438300(item, setting, 0). The driver queues the model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MagicItemModelRequest {
    pub spell: Entity,
    pub setting: Entity,
    /// `TESModel` +0x04 of the setting; a null when its path is empty.
    pub path: Option<Vec<u8>>,
}

// IDA: 00419CF0 and 00419E50 on a spell none of whose effects has a summon flag, with the art setting's +0xA0 at 0: the first false, the second false, so 0041A6A7's arm. Any other state of +0xA0, or a summon effect, is an arm no run reached.
fn magic_item_load_request(
    world: &mut World,
    spell: Entity,
) -> Result<Option<MagicItemModelRequest>, StreamError> {
    let Some(setting) = magic_item_art_setting(world, spell)? else {
        return Ok(None);
    };
    let list = world.get::<EffectItemList>(spell).expect("a magic item");
    for item in &list.items_04 {
        let flags = item
            .setting_1c
            .and_then(|effect| world.get::<EffectSetting>(effect))
            .map_or(0, |effect| effect.data_58.flags_58);
        if flags & EFFECT_FLAGS_SUMMON != 0 {
            return Err(StreamError("00419D5C: an effect with a summon flag"));
        }
    }
    let counter = world
        .get::<EffectSetting>(setting)
        .expect("a setting")
        .art_counter_a0;
    if counter != 0 {
        return Err(StreamError(
            "00419D2C: an art setting already queued or loaded",
        ));
    }
    let model = world.get::<TesModel>(setting).expect("a setting's model");
    let path = (model.path_04.length_04 != 0)
        .then(|| model.path_04.data_00.clone())
        .flatten();
    Ok(Some(MagicItemModelRequest {
        spell,
        setting,
        path,
    }))
}

/// IDA: 0041A610 MagicItem_QueueResources(item, 0). The art setting (00419B30). Loaded (00419CF0 MagicItem_IsResourcesLoaded: the art has no path or is loaded, +0xA0 above 0, and every associated item of an effect with 0x70000 is loaded, +0xA4 above 0); otherwise neither loaded nor queued (00419E50 false) takes 00438300 QueuedMagicItem_Start (`MagicItemModelRequest`, the caller's to start) and returns; otherwise queued. Loaded with an art path loads the art now (0041A6E5 ModelLoader_LoadModelNow), a stop; then 004157A0 steps +0xA0 away from 0. Each effect with 0x70000: loaded, its associated item's model loads now (0041A747), a stop; a summon (0x40000) neither loaded nor queued counts 00B33518, a stop; then 00415740 steps +0xA4 away from 0. Completion: 100% but the loaded arms and summons.
/// # Errors
/// A loaded arm's synchronous loads, a summon, or the art setting's stops.
pub fn magic_item_queue_resources(
    world: &mut World,
    spell: Entity,
) -> Result<Option<MagicItemModelRequest>, StreamError> {
    let setting = magic_item_art_setting(world, spell)?;
    let associated = associated_settings(world, spell);
    let art = setting.map(|setting| art_state(world, setting));
    let loaded = art.is_none_or(|(path, counter)| path == 0 || counter > 0)
        && associated
            .iter()
            .all(|&effect| associated_counter(world, effect) > 0);
    if !loaded && !resources_queued_or_loaded(world, art, &associated) {
        let Some(setting) = setting else {
            return Ok(None);
        };
        return Ok(Some(MagicItemModelRequest {
            spell,
            setting,
            path: art_path(world, setting),
        }));
    }
    if let (Some(setting), Some((path, counter))) = (setting, art) {
        if loaded && path != 0 {
            return Err(StreamError("0041A6E5: loading a loaded art model now"));
        }
        let next = if counter >= 0 {
            counter + 1
        } else {
            counter - 1
        };
        world
            .get_mut::<EffectSetting>(setting)
            .expect("a setting")
            .art_counter_a0 = next as u32;
    }
    for effect in associated {
        if loaded {
            return Err(StreamError(
                "0041A747: loading a loaded associated item now",
            ));
        }
        let counter = associated_counter(world, effect);
        let flags = effect_flags(world, effect);
        if counter == 0 && flags & 0x4_0000 != 0 {
            return Err(StreamError("0041AAE5: a summon counted in 00B33518"));
        }
        let next = if counter >= 0 {
            counter + 1
        } else {
            counter - 1
        };
        world
            .get_mut::<EffectSetting>(effect)
            .expect("a setting")
            .associated_counter_a4 = next as u32;
    }
    Ok(None)
}

// IDA: 00419E50 MagicItem_IsResourcesQueuedOrLoaded: an art with a path that isn't loaded clears the all-loaded flag and must be queued (+0xA0 below 0, 004157F0), else false; each effect with 0x70000 whose item isn't loaded (00415780) the same on +0xA4 (00415790); true when something was only queued.
fn resources_queued_or_loaded(
    world: &World,
    art: Option<(usize, i32)>,
    associated: &[Entity],
) -> bool {
    let mut all_loaded = true;
    if let Some((path, counter)) = art
        && path != 0
        && counter <= 0
    {
        all_loaded = false;
        if counter >= 0 {
            return false;
        }
    }
    for &effect in associated {
        let counter = associated_counter(world, effect);
        if counter <= 0 {
            all_loaded = false;
            if counter >= 0 {
                return false;
            }
        }
    }
    !all_loaded
}

// IDA: N/A each effect's setting with an associated item (+0x58 & 0x70000), in effect order.
fn associated_settings(world: &World, spell: Entity) -> Vec<Entity> {
    world
        .get::<EffectItemList>(spell)
        .expect("a magic item")
        .items_04
        .iter()
        .filter_map(|item| item.setting_1c)
        .filter(|&setting| effect_flags(world, setting) & EFFECT_FLAGS_SUMMON != 0)
        .collect()
}

// IDA: N/A an effect setting's +0x58 flags.
fn effect_flags(world: &World, setting: Entity) -> u32 {
    world
        .get::<EffectSetting>(setting)
        .map_or(0, |effect| effect.data_58.flags_58)
}

// IDA: N/A an effect setting's +0xA4, signed.
fn associated_counter(world: &World, setting: Entity) -> i32 {
    world
        .get::<EffectSetting>(setting)
        .expect("a setting")
        .associated_counter_a4 as i32
}

// IDA: N/A an art setting's model path length (00449190) and its +0xA0, signed.
fn art_state(world: &World, setting: Entity) -> (usize, i32) {
    let model = world.get::<TesModel>(setting).expect("a setting's model");
    (
        super::character_3d::path_length(&model.path_04),
        world
            .get::<EffectSetting>(setting)
            .expect("a setting")
            .art_counter_a0 as i32,
    )
}

// IDA: N/A an art setting's model path, or none when it's empty.
fn art_path(world: &World, setting: Entity) -> Option<Vec<u8>> {
    let model = world.get::<TesModel>(setting).expect("a setting's model");
    (super::character_3d::path_length(&model.path_04) != 0)
        .then(|| model.path_04.data_00.clone())
        .flatten()
}

/// IDA: N/A the QueuedMagicItems an actor's 005E4DD0 started (00438300 with no parent), by their task in the reference task store (class `MagicItem`, the item at +0x20), and their requests.
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct ActorMagicItemTasks {
    pub tasks: Vec<(ReferenceTaskId, MagicItemModelRequest)>,
}

/// IDA: 00438300 QueuedMagicItem_Start(item, setting, 0): FormHeap(0x28) 00437760 (00436500's 00B33A20 + 1; +0x20 the item, +0x24 the setting), vt+0x20 0043D770 QueuedMagicItem_Run on this thread: an art path not queued (004157F0) loads through ModelLoader_LoadTESModel(art, priority 0, this, 0, 0, 1, 0) as a child; 00415EB0; each effect with 0x70000 (when the art was loaded here, only one whose item isn't queued, 00415790): the associated form (+0x60): a bound weapon (0x10000, type 0x21) with a model path through LoadTESModel(+0x30, .., this, its IO stage, 0, 1, 0), a bound armor (0x20000, type 0x14) through both sexes' biped models (00469290), a summon (0x40000) a stop; then 00415E50. +0x0C = 5 and vt+0x28 (`magic_item_task_children_finished`). Completion: 100% but summons.
/// # Errors
/// A summon, or a stop in `load_model`.
pub fn queued_magic_item_start(
    world: &mut World,
    request: MagicItemModelRequest,
    mut archives: Option<&mut (dyn ArchiveAccess + '_)>,
) -> Result<(), StreamError> {
    let id = {
        let mut tasks = world.resource_mut::<TaskGlobals>();
        tasks.counters.live_tasks_b33a20 = tasks.counters.live_tasks_b33a20.wrapping_add(1);
        tasks
            .model_loader_b33a1c
            .as_mut()
            .ok_or(StreamError("00438300: a null g_modelLoader"))?
            .references
            .insert(crate::bs::reference_tasks::QueuedReference {
                class: QueuedReferenceClass::MagicItem,
                references_08: 1,
                state_0c: 0,
                key_10: 0,
                children_1c: Vec::new(),
                children_finished_1c_10: 0,
                reference_20: request.spell,
                child_24: None,
                model_28: None,
                clone_2c: None,
            })
    };
    world
        .get_resource_or_init::<ActorMagicItemTasks>()
        .tasks
        .push((id, request.clone()));
    let (path, counter) = art_state(world, request.setting);
    let art_loaded_here = path != 0 && counter >= 0;
    if art_loaded_here && let Some(art) = &request.path {
        load_child(world, archives.as_deref_mut(), art, id, 0)?;
    }
    queued_magic_item_mark_queued(world, request.setting)?;
    for effect in associated_settings(world, request.spell) {
        if art_loaded_here && associated_counter(world, effect) < 0 {
            continue;
        }
        let flags = effect_flags(world, effect);
        let associated = world
            .get::<EffectSetting>(effect)
            .expect("a setting")
            .data_58
            .associated_60;
        if let Some(form) = super::forms_init::lookup(world, associated) {
            let stage =
                super::reference_load::model_stage(super::forms_init::form_type(world, form));
            if flags & 0x1_0000 != 0 {
                if super::forms_init::form_type(world, form) == 0x21
                    && let Some(model) = world.get::<TesModel>(form)
                    && super::character_3d::path_length(&model.path_04) != 0
                {
                    let path = model.path_04.data_00.clone().unwrap_or_default();
                    load_child(world, archives.as_deref_mut(), &path, id, stage)?;
                }
            } else if flags & 0x2_0000 != 0 {
                if super::forms_init::form_type(world, form) == 0x14 {
                    for sex in 0..2 {
                        let path = armor_biped_model_path(world, form, sex);
                        load_child(world, archives.as_deref_mut(), &path, id, stage)?;
                    }
                }
            } else if flags & 0x4_0000 != 0 {
                return Err(StreamError("0043D8C4: a summon's models"));
            }
        }
        let mut setting = world.get_mut::<EffectSetting>(effect).expect("a setting");
        let counter = setting.associated_counter_a4 as i32;
        if counter > 0 {
            return Err(StreamError(
                "00415E62: queueing an associated item that's already loaded",
            ));
        }
        setting.associated_counter_a4 = (counter - 1) as u32;
    }
    reference_task_mut(world, id)?.state_0c = 5;
    magic_item_task_children_finished(world, id)
}

// IDA: 00469290(sex): a TESBipedModelForm's biped model (+0x08 + 0x18 * sex), the male's for a female with an empty path; its path text.
fn armor_biped_model_path(world: &World, form: Entity, sex: usize) -> Vec<u8> {
    let biped = world
        .get::<super::form_components::TesBipedModelForm>(form)
        .expect("an armor");
    let sex =
        if sex == 1 && super::character_3d::path_length(&biped.biped_models_08[1].path_04) == 0 {
            0
        } else {
            sex
        };
    biped.biped_models_08[sex]
        .path_04
        .data_00
        .clone()
        .unwrap_or_default()
}

// IDA: ModelLoader_LoadTESModel(model, priority 0, task, stage, 0, 1, 0) for a QueuedMagicItem child: the path fixed up, `load_model` with the task as its parent; a cached model's child is finished at once (004374A0's notification). The out reference is dropped (the port's counts leave out the parent's and the IO queue's).
fn load_child(
    world: &mut World,
    archives: Option<&mut (dyn ArchiveAccess + '_)>,
    path: &[u8],
    parent: ReferenceTaskId,
    stage: u32,
) -> Result<(), StreamError> {
    let request = ModelRequest {
        priority: 0,
        parent: Some(parent),
        lod: stage,
        a7: false,
        a8: true,
        a9: false,
    };
    let path = crate::bs::model_loader::fixup_model_path(path);
    let mut tasks = world.resource_mut::<TaskGlobals>();
    let child = match archives {
        Some(archives) => {
            let mut out = None;
            archives.with_archives(&mut |manager| {
                out = Some(load_model(&mut tasks, manager, &path, request));
            });
            out.expect("with_archives runs its body")?
        }
        None => load_model(&mut tasks, None, &path, request)?,
    };
    if let Some(child) = child {
        let finished = world
            .resource::<TaskGlobals>()
            .io_manager_b33a10
            .as_ref()
            .and_then(|io| io.tasks.store[child.0].as_ref())
            .is_some_and(|task| task.state_0c == 5);
        if finished {
            super::reference_load::queued_file_notify_parent(world, child)?;
        }
    }
    Ok(())
}

// IDA: N/A a reference task by ID, mutable.
fn reference_task_mut(
    world: &mut World,
    id: ReferenceTaskId,
) -> Result<Mut<'_, crate::bs::reference_tasks::QueuedReference>, StreamError> {
    let tasks = world.resource_mut::<TaskGlobals>();
    if tasks
        .model_loader_b33a1c
        .as_ref()
        .and_then(|loader| loader.references.get(id))
        .is_none()
    {
        return Err(StreamError("a released QueuedMagicItem"));
    }
    Ok(tasks.map_unchanged(|tasks| {
        tasks
            .model_loader_b33a1c
            .as_mut()
            .and_then(|loader| loader.references.get_mut(id))
            .expect("tested above")
    }))
}

/// IDA: 00437790 QueuedMagicItem_FinalizeWhenChildrenDone, vt+0x28: +0x0C at 4 or more and every child finished: 0041AB90 MagicItem_ResourcesLoaded, then the task goes when nothing holds it (00436670: 00B33A20 - 1). 0041AB90: the item neither loaded nor queued (00419E50) re-queues itself, a stop; a queued art takes 00415EE0's absolute value and adds that many references to its model (0043E680 ModelLoader_FindScaledModel, 00434C00); each effect whose item is queued (00415790) takes 00415E80's absolute value onto +0xA4 and as many references to a bound weapon's model, a bound armor's two biped models (00469330), a summon's a stop. Completion: 100% but summons.
/// # Errors
/// A released task, or those stops.
pub fn magic_item_task_children_finished(
    world: &mut World,
    id: ReferenceTaskId,
) -> Result<(), StreamError> {
    let task = world
        .resource::<TaskGlobals>()
        .model_loader_b33a1c
        .as_ref()
        .and_then(|loader| loader.references.get(id))
        .cloned()
        .ok_or(StreamError("00437790: a released QueuedMagicItem"))?;
    if task.state_0c < 4 || !task.children_done() {
        return Ok(());
    }
    let request = world
        .resource::<ActorMagicItemTasks>()
        .tasks
        .iter()
        .find(|(task, _)| *task == id)
        .map(|(_, request)| request.clone())
        .ok_or(StreamError("00437790: a QueuedMagicItem with no request"))?;
    let art = Some(art_state(world, request.setting));
    let associated = associated_settings(world, request.spell);
    if !resources_queued_or_loaded(world, art, &associated) {
        return Err(StreamError("0041ABA4: 0041A610(1) and 0041A3C0(1)"));
    }
    queued_magic_item_finish(world, &request)?;
    for effect in associated {
        let counter = associated_counter(world, effect);
        if counter >= 0 {
            continue;
        }
        let count = counter.unsigned_abs();
        let (flags, associated_id) = {
            let setting = world.get::<EffectSetting>(effect).expect("a setting");
            (setting.data_58.flags_58, setting.data_58.associated_60)
        };
        world
            .get_mut::<EffectSetting>(effect)
            .expect("a setting")
            .associated_counter_a4 = count;
        let Some(form) = super::forms_init::lookup(world, associated_id) else {
            continue;
        };
        let mut paths = Vec::new();
        if flags & 0x1_0000 != 0 {
            if let Some(model) = world.get::<TesModel>(form)
                && super::forms_init::form_type(world, form) == 0x21
                && super::character_3d::path_length(&model.path_04) != 0
            {
                paths.push(model.path_04.data_00.clone().unwrap_or_default());
            }
        } else if flags & 0x2_0000 != 0 {
            if super::forms_init::form_type(world, form) == 0x14 {
                paths.push(armor_biped_model_path(world, form, 0));
                paths.push(armor_biped_model_path(world, form, 1));
            }
        } else if flags & 0x4_0000 != 0 {
            return Err(StreamError("0041AD21: a summon's models"));
        }
        let mut tasks = world.resource_mut::<TaskGlobals>();
        let loader = tasks
            .model_loader_b33a1c
            .as_mut()
            .ok_or(StreamError("0041AC4C: a null g_modelLoader"))?;
        for path in paths {
            let key: Vec<u8> = path
                .split(|&byte| byte == 0)
                .next()
                .unwrap_or_default()
                .to_vec();
            if let Some(model) = model_map_lookup(loader, &key)? {
                loader.models.store[model.0].references_04 += count;
            }
        }
    }
    // 00437790: the task's reference goes, and with it the task (`queued_magic_item_finish` took 00B33A20 down).
    let mut tasks = world.resource_mut::<TaskGlobals>();
    if let Some(loader) = tasks.model_loader_b33a1c.as_mut() {
        loader.references.store[id.0] = None;
    }
    world
        .resource_mut::<ActorMagicItemTasks>()
        .tasks
        .retain(|(task, _)| *task != id);
    Ok(())
}

/// IDA: 0043D770's head, QueuedMagicItem vt+0x20: 00415EB0 takes the art setting's +0xA0 down by one. The stage to state 5 and its vt+0x28 are the driver's. Completion: 100% of a counter at or below 0.
/// # Errors
/// A positive counter, 00415EB0's warning.
pub fn queued_magic_item_mark_queued(
    world: &mut World,
    setting: Entity,
) -> Result<(), StreamError> {
    let mut effect = world.get_mut::<EffectSetting>(setting).expect("a setting");
    if (effect.art_counter_a0 as i32) > 0 {
        return Err(StreamError(
            "00415EC2: queueing an art setting that's already loaded",
        ));
    }
    effect.art_counter_a0 = effect.art_counter_a0.wrapping_sub(1);
    Ok(())
}

/// IDA: 00664850 PlayerCharacter_SetActiveBook(0) on a player with no book: +0x628 = 0, then 00664700(0), whose first compare returns while nothing is selected. Completion: 100% of that state.
/// # Errors
/// A book, or an item, already set.
pub fn player_clear_active_book(world: &mut World, player: Entity) -> Result<(), StreamError> {
    let mut data = world
        .get_mut::<PlayerCharacter>(player)
        .expect("the player");
    if data.book_628 != 0 {
        return Err(StreamError("00664883: a book already open"));
    }
    if data.active_magic_item_624.is_some() {
        return Err(StreamError("006648BF: clearing a selected magic item"));
    }
    data.book_628 = 0;
    Ok(())
}

// IDA: 00662E0A, process vt+0x13C: 006293D0 reads the byte at +0xF5 (a staff is the weapon, 00658550). A first start has it clear, so 00662DA0's second sum stays 0.
fn player_has_staff_equipped(world: &World, player: Entity) -> bool {
    world
        .get::<MobileObject>(player)
        .and_then(|mobile| mobile.process_58)
        .and_then(|process| world.get::<MiddleHighProcess>(process))
        .is_some_and(|process| process.staff_f5 != 0)
}

// IDA: 00662DA0 PlayerCharacter_UpdateTelekinesisTargeting for an item with no TELE effect and no weapon out: +0x6E6 = 0 and vt+0x290, 0065D1E0(60, 0.0): 00523310's default arm, ActorValues_SetModifier on the base's +0xD0, nothing with no entry and 0.0, then TESForm_SetChangeFlags(base, 0x10000000); 0057A6F0(60) reaches no menu for 60; 005E2670 ignores it.
fn player_update_telekinesis(
    cx: &mut InterfaceCx,
    player: Entity,
    spell: Entity,
) -> Result<(), StreamError> {
    let list = cx.world.get::<EffectItemList>(spell).expect("a magic item");
    for item in &list.items_04 {
        let code = item
            .setting_1c
            .and_then(|setting| cx.world.get::<EffectSetting>(setting))
            .map_or(0, |setting| setting.code_98);
        if code == EFFECT_CODE_TELE {
            return Err(StreamError("00662DEC: a telekinesis effect's magnitude"));
        }
    }
    if player_has_staff_equipped(cx.world, player) {
        return Err(StreamError("00662E04: an equipped weapon's enchantment"));
    }
    cx.world
        .get_mut::<PlayerCharacter>(player)
        .expect("the player")
        .telekinesis_targeting_enabled_6e6 = 0;
    if cx.engine.tile_menu_by_id(STATS_MENU_ID).is_some() {
        return Err(StreamError(
            "0057A78F: StatsMenu_RefreshAttributesAndSkills",
        ));
    }
    let base = cx
        .world
        .get::<TesObjectRefr>(player)
        .and_then(|refr| refr.base_form_1c)
        .ok_or(StreamError("0065D1E6: a null base form faults"))?;
    let values = cx
        .world
        .get::<ActorBaseValues>(base)
        .expect("an NPC's values");
    if values
        .0
        .list_00
        .iter()
        .any(|entry| entry.code_00 == AV_TELEKINESIS)
    {
        return Err(StreamError("0065CB68: clearing the base's actor value 60"));
    }
    set_change_flags(cx.world, base, 0x1000_0000)
}

/// IDA: 00664700 PlayerCharacter_SetSelectedMagicItem(item), with no item before it and no cached sound: +0x624, 0057B2D0 with `Icons\` and the highest cost effect's icon, 0041A610(item, 0), 00662DA0. The item stands for `spell + 0x18`. Returns what 0041A610 asks the driver to queue. Completion: 100% of a first start.
/// # Errors
/// An item already selected, a selected effect with no setting, or an arm below.
pub fn player_set_selected_magic_item(
    cx: &mut InterfaceCx,
    player: Entity,
    spell: Entity,
) -> Result<Option<MagicItemModelRequest>, StreamError> {
    {
        let data = cx.world.get::<PlayerCharacter>(player).expect("the player");
        if data.active_magic_item_624 == Some(spell) {
            return Ok(None);
        }
        if data.active_magic_item_624.is_some() {
            return Err(StreamError("00664733: 00419F10 on the previous item"));
        }
        if data.cached_sound_handle_764 != 0 {
            return Err(StreamError("0066480A: the cached sound handle"));
        }
    }
    cx.world
        .get_mut::<PlayerCharacter>(player)
        .expect("the player")
        .active_magic_item_624 = Some(spell);
    // 0066474E: a null answer from 004152C0 faults on +0x1C.
    let index = highest_cost_effect(cx.world, spell)?.ok_or(StreamError(
        "0066474E: an item with no costed effect faults",
    ))?;
    let setting = cx
        .world
        .get::<EffectItemList>(spell)
        .expect("a magic item")
        .items_04[index]
        .setting_1c
        .ok_or(StreamError("00664751: a null effect setting faults"))?;
    let icon = cx
        .world
        .get::<TesTexture>(setting)
        .expect("a setting's icon");
    let mut path = b"Icons\\".to_vec();
    if let Some(name) = &icon.path_04.data_00 {
        path.extend_from_slice(name);
    }
    hud_set_selected_magic_icon_path(cx, &path)?;
    let request = magic_item_load_request(cx.world, spell)?;
    player_update_telekinesis(cx, player, spell)?;
    Ok(request)
}

/// IDA: WinMain 0040EFFF..0040F025: base +0x58, each spell through 00415360 until one answers 0x19. A list with no item answers none. Each slot is a resolved form by now. Completion: 100%.
/// # Errors
/// A slot still holding a raw ID, or as `magic_item_school_actor_value`.
pub fn find_starting_spell(
    world: &mut World,
    spells: &[FormRef],
) -> Result<Option<Entity>, StreamError> {
    for slot in spells {
        let FormRef::Resolved(Some(spell)) = *slot else {
            return Err(StreamError("0040F00F: a spell slot that isn't a form"));
        };
        if magic_item_school_actor_value(world, spell)? == SCHOOL_RESTORATION {
            return Ok(Some(spell));
        }
    }
    Ok(None)
}

/// IDA: 00438300(item, setting, 0): FormHeap(0x28) 00437760, whose 00436500 adds one to 00B33A20, and vt+0x20, 0043D770, on this thread: 0043B280(model, priority 0, this, 0, 0, 1, 0), which misses the map and queues a QueuedModel (00437350), then 00415EB0. The task is the parent of the model, so it's finished by `queued_magic_item_finish`. A hit with a parent is an arm no run reached. Completion: 100% of a first start.
/// TODO: [ ] 00437350's +0x2C, the TESModel: the port's `QueuedModel` has no use for it yet.
/// # Errors
/// A cached model, or a stop in `load_model`.
pub fn queued_magic_item_run(
    world: &mut World,
    request: &MagicItemModelRequest,
    archives: Option<&mut (dyn ArchiveAccess + '_)>,
) -> Result<(), StreamError> {
    let mut tasks = world.resource_mut::<TaskGlobals>();
    tasks.counters.live_tasks_b33a20 = tasks.counters.live_tasks_b33a20.wrapping_add(1);
    if let Some(path) = &request.path {
        let model = ModelRequest {
            priority: 0,
            parent: None,
            lod: 0,
            a7: false,
            a8: true,
            a9: false,
        };
        let out = match archives {
            Some(archives) => {
                let mut out = None;
                archives.with_archives(&mut |manager| {
                    out = Some(load_model(&mut tasks, manager, path, model));
                });
                out.expect("with_archives runs its body")?
            }
            None => load_model(&mut tasks, None, path, model)?,
        };
        if out.is_none() {
            return Err(StreamError("0043B30F: an art model already in the map"));
        }
        release_task(&mut tasks, out)?;
    }
    queued_magic_item_mark_queued(world, request.setting)
}

/// IDA: 00437790, the IO thread's call when the model's done, and what 0041AB90 reaches with no summon effect: 00415EE0 turns the setting's negative +0xA0 positive and 00434C00 adds that many references to the model; then the task's 00B33A20 goes. A recorded substitution: the driver calls this after its drain pass, and the original's thread at any point past 0040F04A. Completion: 100% of a first start.
/// # Errors
/// A null g_modelLoader.
pub fn queued_magic_item_finish(
    world: &mut World,
    request: &MagicItemModelRequest,
) -> Result<(), StreamError> {
    let mut effect = world
        .get_mut::<EffectSetting>(request.setting)
        .expect("a setting");
    let count = (effect.art_counter_a0 as i32).unsigned_abs();
    effect.art_counter_a0 = count;
    let mut tasks = world.resource_mut::<TaskGlobals>();
    tasks.counters.live_tasks_b33a20 = tasks.counters.live_tasks_b33a20.wrapping_sub(1);
    let Some(path) = &request.path else {
        return Ok(());
    };
    let loader = tasks
        .model_loader_b33a1c
        .as_mut()
        .ok_or(StreamError("0041AC4C: a null g_modelLoader"))?;
    if let Some(model) = model_map_lookup(loader, path)? {
        loader.models.store[model.0].references_04 += count;
    }
    Ok(())
}
