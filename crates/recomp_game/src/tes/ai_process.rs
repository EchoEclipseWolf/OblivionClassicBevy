//! Phase 14c: 00675310 ActorProcessManager_ProcessTrackedActors and the AI package pass it runs over the level-3 list: 004F9EC0, the death and removal arms, and 005ED860: the stamps 00643640, the scheduled packages 0064A380, the evaluation 00649340, 00648E40 and the cell moves; with the actor values (00643340, 005F1970, 005F1A60) and weights they read.
//!
//! P14-RUN-001..004: 1,998 actors. The original's `rand` and the interface's menu mode come through `AiArms`: `AiRealArms` for a driver, `AiRecordedArms` for a test replaying a run's draws and logging what asked. This is this slice's `bind_real_arms` and recording arms: the C++'s Phase 15 function of that name binds 15c and 15d's arms, which come with that port.
//!
//! The pass reads no world setup output (TES +0x74, the LOD maps, the idle roots, the archives). With TLS +0x185 set, as WinMain 0040EB8B leaves it, `set_change_flags` records changes.

use bevy_ecs::prelude::*;
use recomp_core::reader::StreamError;
use recomp_core::x87;

use super::actor::{Actor, Creature, TesObjectRefr};
use super::actor_process::{
    ActorListId, ActorListNode, ActorProcessGlobals, BaseProcess, LowProcess, RandomSource,
    actor_process_list_remove, actor_values_get, random_next, release_deleted_process,
};
use super::ai_cell_moves::{doors_near, move_to_cell};
use super::ai_conditions::{
    ConditionCaches, evaluate_conditions, form_flags, process_of, the_player,
};
use super::ai_package::{
    actor_is_dead, actor_package_list, cell_first_reference, collect_scheduled_packages,
    game_day_byte, game_hour, location_reference, package_at_location, package_data,
    package_init_procedure, package_is_combat_like, package_location_cell,
    package_location_position, package_location_world, parent_cell, procedure_at, process_stamps,
    reference_position, reference_world, script_event_or, select_from_list, target_object,
    target_object_type, target_reference,
};
use super::base_extra_list::{
    EXTRA_CONTAINER_CHANGES, EXTRA_FOLLOWER, EXTRA_HORSE, EXTRA_PACKAGE_START,
    EXTRA_REFERENCE_POINTER, EXTRA_RUN_ONCE_PACKS, ExtraData, new_extra_entry,
};
use super::character_controller::actor_set_position;
use super::condition_tables::CONDITION_FUNCTIONS;
use super::form::FormIdentity;
use super::form_components::{
    FormRef, TesActorBaseData, TesAiForm, TesAttributes, TesContainer, TesHealthForm, TesWeightForm,
};
use super::forms_actors::{ActorBaseValues, TesCreature, TesNpc};
use super::forms_init::{effective_level, form_type};
use super::forms_init_world::set_death_state;
use super::forms_items::{clear_change_flags, set_change_flags};
use super::forms_references::{reference_is_persistent, save_load_active, set_reference_position};
use super::forms_world::TesObjectCell;
use super::game_settings::GameSettingCollection;
use super::inventory::{ContainerChanges, reference_container};

/// IDA: N/A what the pass asks outside the game state. A driver and a test bind these differently.
pub trait AiArms {
    /// IDA: 0047DF80 Random_Next(0) (004F4EF0 GetRandomPercent).
    /// # Errors
    /// A recording arm out of draws.
    fn random_next(&mut self, world: &mut World) -> Result<i32, StreamError>;
    /// IDA: 00578F60 InterfaceManager_IsMenuMode.
    /// # Errors
    /// An arm with no answer.
    fn menu_mode(&mut self, world: &mut World) -> Result<bool, StreamError>;
}

/// IDA: N/A the real arms: 0047DF80 over a CRT and the interface's answer, which Phase 21's interface fills.
pub struct AiRealArms<'a> {
    pub random: &'a mut dyn RandomSource,
    pub menu_mode: &'a mut dyn FnMut(&World) -> bool,
}

impl AiArms for AiRealArms<'_> {
    /// IDA: N/A `AiRealArms`'s side of `AiArms`.
    fn random_next(&mut self, world: &mut World) -> Result<i32, StreamError> {
        let mut globals = world.resource_mut::<ActorProcessGlobals>();
        Ok(random_next(0, &mut globals.random, &mut *self.random))
    }
    /// IDA: N/A `AiRealArms`'s side of `AiArms`.
    fn menu_mode(&mut self, world: &mut World) -> Result<bool, StreamError> {
        Ok((self.menu_mode)(world))
    }
}

/// IDA: N/A the recording arms: a run's draws in order, and how many went. P14-RUN-004's 157 `GetRandomPercent` values go through here, and `next` tells a test if the host drew a different number.
#[derive(Debug, Default)]
pub struct AiRecordedArms {
    pub draws: Vec<i32>,
    pub next: usize,
    pub menu_mode: bool,
    pub menu_queries: u32,
}

impl AiArms for AiRecordedArms {
    /// IDA: N/A no address of its own; its comments cite 004F4EF0.
    fn random_next(&mut self, _world: &mut World) -> Result<i32, StreamError> {
        let value = *self
            .draws
            .get(self.next)
            .ok_or(StreamError("004F4EF0: more draws than the recorded run"))?;
        self.next += 1;
        Ok(value)
    }
    /// IDA: N/A `AiRecordedArms`'s side of `AiArms`.
    fn menu_mode(&mut self, _world: &mut World) -> Result<bool, StreamError> {
        self.menu_queries += 1;
        Ok(self.menu_mode)
    }
}

/// IDA: the handler caches, and the host's counts per arm (N/A), which a test and the log read. The warnings are 004A7A60's, which a driver hands on.
#[derive(Resource, Debug)]
pub struct AiProcessGlobals {
    pub caches: ConditionCaches,
    /// IDA: N/A the 004F9EC0 -> 00517950 runs.
    pub script_event_runs_not_executed: u32,
    pub actors_visited: u32,
    pub actors_removed: u32,
    pub packages_evaluated: u32,
    pub cell_moves: u32,
    pub random_calls: u32,
    /// IDA: N/A the form ID behind each draw, in draw order.
    pub random_subjects: Vec<u32>,
    pub doors_unlocked: u32,
    pub doors_locked: u32,
    pub condition_calls: Vec<u32>,
    pub warnings: Vec<String>,
}

impl Default for AiProcessGlobals {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            caches: ConditionCaches::default(),
            script_event_runs_not_executed: 0,
            actors_visited: 0,
            actors_removed: 0,
            packages_evaluated: 0,
            cell_moves: 0,
            random_calls: 0,
            random_subjects: Vec::new(),
            doors_unlocked: 0,
            doors_locked: 0,
            condition_calls: vec![0; CONDITION_FUNCTIONS.len()],
            warnings: Vec::new(),
        }
    }
}

// ---- the actor values ----

// IDA: 0051CA10, 0051CAA0, 0051CAD0 on a creature: with TESActorBaseData flag 0x80 the 16-bit value times the level, forced to at least 1, in 16 bits; otherwise the value. The C++ reads the raw fields, which is wrong for a PC-level creature.
fn scaled_creature_value(world: &World, base: Entity, value: u16) -> Result<u16, StreamError> {
    let data = world.get::<TesActorBaseData>(base).expect("an actor base");
    if data.flags_04 & 0x80 == 0 {
        return Ok(value);
    }
    let level = effective_level(world, data)?.max(1) as u16;
    Ok(value.wrapping_mul(level))
}

// IDA: 0051CC00, 0051CB80, 0051CB00 a creature's skill groups: with flag 0x80, `(u8)chop(f32(f32(setting * level) + skill))`, else the byte. The settings' cinit default is 2.0.
fn creature_skill(
    world: &World,
    base: Entity,
    index: usize,
    address: u32,
) -> Result<i32, StreamError> {
    let data = world.get::<TesActorBaseData>(base).expect("an actor base");
    let byte = world.get::<TesCreature>(base).expect("a creature").data_104[index];
    if data.flags_04 & 0x80 == 0 {
        return Ok(i32::from(byte));
    }
    let level = effective_level(world, data)?.max(1);
    let setting = world
        .get_resource::<GameSettingCollection>()
        .and_then(|settings| settings.raw_at(address))
        .map_or(2.0, f32::from_bits);
    let value = setting * f32::from(level) + f32::from(byte);
    Ok(i32::from(x87::ftol(value) as u8))
}

// IDA: 00519D90 GetActorValueFromBaseForm: 0..=7 the +0x88 attributes; 8 00519C10; 9, 10 TESActorBaseData's; 11 ftol(004698B0); 12..=32 and 37..=39 zero; 33..=36 the +0x68 bytes; above, ftol(0065CB80 on +0xD0).
pub(super) fn base_form_actor_value(
    world: &World,
    base: Entity,
    code: u32,
) -> Result<i32, StreamError> {
    let creature = world.get::<TesCreature>(base).is_some();
    let data = world.get::<TesActorBaseData>(base);
    Ok(match code {
        0..=7 => i32::from(
            world
                .get::<TesAttributes>(base)
                .map_or(0, |attributes| attributes.values_04[code as usize]),
        ),
        8 => {
            let health = world
                .get::<TesHealthForm>(base)
                .map_or(0, |health| health.health_04);
            if creature {
                i32::from(scaled_creature_value(world, base, health as u16)?)
            } else {
                health as i32
            }
        }
        9 | 10 => {
            let data = data.ok_or(StreamError("00519E5C: no TESActorBaseData faults"))?;
            let value = if code == 9 {
                data.magicka_08
            } else {
                data.fatigue_0a
            };
            if creature {
                i32::from(scaled_creature_value(world, base, value)?)
            } else {
                i32::from(value)
            }
        }
        11 => x87::ftol(
            world
                .get::<TesContainer>(base)
                .map_or(Ok(0.0), |container| container_weight(world, container))?,
        ),
        12..=32 | 37..=39 => 0,
        33..=36 => i32::from(
            world
                .get::<TesAiForm>(base)
                .map_or(0, |ai| ai.ai_04[(code - 33) as usize]),
        ),
        _ => x87::ftol(
            world
                .get::<ActorBaseValues>(base)
                .map_or(0.0, |values| actor_values_get(&values.0, code as u8)),
        ),
    })
}

/// IDA: base vt+0x128: TESNPC 005232D0 (skills 12..=32 from +0xEC, 37..=39 zero, else 00519D90), TESCreature 0051D540 (the three skill groups, else 00519D90). Completion: 100%.
pub fn base_actor_value(world: &World, base: Entity, code: u32) -> Result<i32, StreamError> {
    if let Some(npc) = world.get::<TesNpc>(base) {
        if (12..=32).contains(&code) {
            return Ok(i32::from(npc.skills_ec[(code - 12) as usize]));
        }
        return base_form_actor_value(world, base, code);
    }
    if world.get::<TesCreature>(base).is_some() {
        return match code {
            // 0051D544..0051D572: plain ranges. Only the actor folds at 006253C0, 00625410 send 28 to 12.
            12..=18 => creature_skill(world, base, 1, 0x00B3_62A4),
            19..=25 => creature_skill(world, base, 2, 0x00B3_62AC),
            26..=32 => creature_skill(world, base, 3, 0x00B3_62B4),
            _ => base_form_actor_value(world, base, code),
        };
    }
    Err(StreamError("006433B7: base vt+0x128 on a non-actor base"))
}

// IDA: 006253C0, 00625410 the Creature folds: 12..=18 and 28 are 12, 19..=25 are 19, 26..=32 are 26.
fn fold_creature_code(world: &World, actor: Entity, code: u32) -> u32 {
    if world.get::<Creature>(actor).is_none() {
        return code;
    }
    match code {
        12..=18 | 28 => 12,
        19..=25 => 19,
        26..=32 => 26,
        _ => code,
    }
}

// IDA: 00470620 Form_GetItemWeight: the weighted form types' TESWeightForm +0x04, else -1.0.
fn item_weight(world: &World, form: Entity) -> Result<f32, StreamError> {
    match form_type(world, form) {
        0x13 | 0x14 | 0x15 | 0x16 | 0x17 | 0x19 | 0x1A | 0x1B | 0x21 | 0x22 | 0x26 | 0x27
        | 0x28 | 0x2A => world
            .get::<TesWeightForm>(form)
            .map(|weight| weight.weight_04)
            .ok_or(StreamError(
                "00470620: a weighted type with no TESWeightForm",
            )),
        _ => Ok(-1.0),
    }
}

// IDA: N/A no address of its own; its comments cite 004698CE.
fn entry_item(slot: FormRef) -> Result<Entity, StreamError> {
    match slot {
        FormRef::Resolved(Some(form)) => Ok(form),
        _ => Err(StreamError(
            "004698CE: a container entry with no item faults",
        )),
    }
}

// IDA: 004698B0 TESContainer_GetWeight: with +0x04 bit 0, `f32(f32(w * |count|) + total)` over the items not flagged 0x20; a -1.0 from a non-weighted form isn't mapped to 0 here. `fimul` is exact on the int.
fn container_weight(world: &World, container: &TesContainer) -> Result<f32, StreamError> {
    let mut total = 0.0_f32;
    if container.flags_04 & 1 == 0 {
        return Ok(total);
    }
    for entry in &container.entries_08 {
        let item = entry_item(entry.item)?;
        if form_flags(world, item) & 0x20 == 0 {
            let product = (f64::from(item_weight(world, item)?)
                * f64::from(entry.count.wrapping_abs())) as f32;
            total += product;
        }
    }
    Ok(total)
}

// IDA: 00484E80 on an entry: a list has either of the extras 0x1B, 0x1C: `[list+0xB] & 0x18`.
fn entry_is_worn(entry: &super::inventory::InventoryEntry) -> bool {
    entry
        .lists_00
        .as_ref()
        .is_some_and(|lists| lists.iter().any(|list| list.presence_08[3] & 0x18 != 0))
}

// IDA: 00487E40 ContainerChanges_GetTotalWeight: the +0x08 cache unless -1.0; the base container's non-quest items (vt+0x78 flag 0x400; -1.0 is 0.0 here) with their deltas; the change entries not in the base container; the store to +0x08. Completion: 100% past the worn arm.
// TODO: [ ] 00487FF2 the worn armor reduction (5F23B0's skill levels, fPerk*ArmorSpeedMult), which needs an equipped entry outside the base container.
fn container_changes_weight(world: &World, changes: &ContainerChanges) -> Result<f32, StreamError> {
    if changes.float_08 != -1.0 {
        return Ok(changes.float_08);
    }
    let mut total = 0.0_f32;
    let container = reference_container(world, changes.owner_04)
        .and_then(|base| world.get::<TesContainer>(base));
    let find_change = |item: Entity| {
        changes
            .entries_00
            .iter()
            .find(|entry| entry.form_08 == item)
    };
    if let Some(container) = container {
        for entry in &container.entries_08 {
            let item = entry_item(entry.item)?;
            if form_flags(world, item) & 0x400 != 0 {
                continue;
            }
            let mut weight = item_weight(world, item)?;
            if weight == -1.0 {
                weight = 0.0;
            }
            let count = match find_change(item) {
                Some(change) => {
                    let count = entry.count.wrapping_add(change.count_04);
                    if count == 0 {
                        continue;
                    }
                    count
                }
                None => entry.count,
            };
            total += (f64::from(weight) * f64::from(count)) as f32;
        }
    }
    let actor_owner = world.get::<Actor>(changes.owner_04).is_some();
    for entry in &changes.entries_00 {
        let item = entry.form_08;
        let in_base = container.is_some_and(|container| {
            container.flags_04 & 1 != 0
                && container
                    .entries_08
                    .iter()
                    .any(|base| matches!(base.item, FormRef::Resolved(Some(form)) if form == item))
        });
        if in_base || form_flags(world, item) & 0x400 != 0 {
            continue;
        }
        let weight = item_weight(world, item)?;
        if actor_owner && form_type(world, item) == 0x14 && entry_is_worn(entry) {
            return Err(StreamError("00487FF2: the worn armor weight"));
        }
        if entry.count_04 > 0 {
            total += (f64::from(entry.count_04) * f64::from(weight)) as f32;
        }
    }
    Ok(total)
}

// IDA: 004D8FB0 TESObjectREFR_GetEncumbrance: the changes' weight, stored at +0x08 (its cache), or with no changes block the base's; a non-player actor is capped by 005E0D20: `v = f32(fActorStrengthEncumbranceMult * vt+0x288(0))`, `v < 0` is 0, a NaN stays NaN (00547EE8; the C++ gives 0) and then keeps the weight (`fcomp q; test ah,5; jp`). Default of the setting: 5.0 (009EB570).
fn reference_encumbrance(world: &mut World, reference: Entity) -> Result<f32, StreamError> {
    let mut weight = 0.0_f32;
    if let Some(base) = reference_container(world, reference) {
        let list = &world
            .get::<TesObjectRefr>(reference)
            .expect("a reference")
            .extra_data_44;
        match list.get(EXTRA_CONTAINER_CHANGES).map(|entry| &entry.data) {
            Some(ExtraData::ContainerChanges(changes)) => {
                weight = container_changes_weight(world, changes)?;
                if let Some(entry) = world
                    .get_mut::<TesObjectRefr>(reference)
                    .expect("a reference")
                    .extra_data_44
                    .get_mut(EXTRA_CONTAINER_CHANGES)
                    && let ExtraData::ContainerChanges(changes) = &mut entry.data
                {
                    changes.float_08 = weight;
                }
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

// IDA: the shared sum of 00643340 and 006433E0: fild base + actor +0x88 + process +0x70, a float at each add; code 11 starts from 004D8FB0. `fild` is exact and rounds at the first fadd, so the C++'s int to float first differs past 2^24.
fn low_process_value(
    world: &mut World,
    actor: Entity,
    process: Entity,
    code: u32,
) -> Result<f32, StreamError> {
    let a = actor_values_get(
        &world.get::<Actor>(actor).expect("an actor").av_modifiers_88,
        code as u8,
    );
    let b = world.get::<LowProcess>(process).map_or(0.0, |low| {
        actor_values_get(&low.damage_modifiers_70, code as u8)
    });
    if code == 11 {
        let encumbrance = reference_encumbrance(world, actor)?;
        return Ok((encumbrance + a) + b);
    }
    let base = world
        .get::<TesObjectRefr>(actor)
        .expect("an actor")
        .base_form_1c
        .ok_or(StreamError("006433B7: vt+0x170 with no base faults"))?;
    let value = base_actor_value(world, base, code)?;
    Ok(((f64::from(value) + f64::from(a)) as f32) + b)
}

// IDA: 005F19A7: `fv = vt+0x288(40)`; `f32(fv / 10.0)`; exactly 0 (-0.0 too) is 1.0, a NaN stays.
fn magicka_multiplier(world: &mut World, actor: Entity) -> Result<f32, StreamError> {
    let multiplier = (f64::from(actor_get_actor_value_float(world, actor, 40)?) / 10.0) as f32;
    Ok(if multiplier == 0.0 { 1.0 } else { multiplier })
}

// IDA: the no-process arms, 5F1A42 and 005F1B2A: 005F1910's floor of 005EAD00: the base value for a non-player's codes outside 37..=39.
fn no_process_value(world: &World, actor: Entity, code: u32) -> Result<i32, StreamError> {
    if (37..=39).contains(&code) {
        return Err(StreamError(
            "005EAD00: codes 37..=39 of an actor with no process",
        ));
    }
    let base = world
        .get::<TesObjectRefr>(actor)
        .expect("an actor")
        .base_form_1c
        .ok_or(StreamError("005EAD00: no base faults"))?;
    base_actor_value(world, base, code)
}

/// IDA: 005F1A60 Actor vt+0x288 GetActorValueFloat (Creature 00625410 folds first). AV 9 multiplies the 006433E0 sum by 005F19A7's. Player 0065E110 stops: it has its own sum. Completion: 100% past the player.
pub fn actor_get_actor_value_float(
    world: &mut World,
    actor: Entity,
    actor_value: u32,
) -> Result<f32, StreamError> {
    if Some(actor) == the_player(world) {
        return Err(StreamError("0065E110: the player's GetActorValueFloat"));
    }
    let code = fold_creature_code(world, actor, actor_value);
    let Some(process) = process_of(world, actor) else {
        return Ok(no_process_value(world, actor, code)? as f32);
    };
    if code == 9 {
        let multiplier = magicka_multiplier(world, actor)?;
        return Ok(low_process_value(world, actor, process, 9)? * multiplier);
    }
    low_process_value(world, actor, process, code)
}

/// IDA: 005F1970 Actor vt+0x284 GetActorValue (Creature 006253C0 folds first). AV 9: `ftol(f32(fild(ftol(00643340)) * multiplier))`: the sum is an int before the multiply (005F19EB), where the C++ keeps its fraction. The others `ftol` the sum. The player is 0065E030's (`player::player_get_actor_value`). Completion: 100%.
pub fn actor_get_actor_value(
    world: &mut World,
    actor: Entity,
    actor_value: u32,
) -> Result<i32, StreamError> {
    // PlayerCharacter's vt+0x284 is 0065E030.
    if Some(actor) == the_player(world) {
        return super::player::player_get_actor_value(world, actor, actor_value);
    }
    let code = fold_creature_code(world, actor, actor_value);
    let Some(process) = process_of(world, actor) else {
        return no_process_value(world, actor, code);
    };
    if code == 9 {
        let multiplier = magicka_multiplier(world, actor)?;
        let value = x87::ftol(low_process_value(world, actor, process, 9)?);
        return Ok(x87::ftol((f64::from(value) * f64::from(multiplier)) as f32));
    }
    Ok(x87::ftol(low_process_value(world, actor, process, code)?))
}

// ---- the LowProcess package slots ----

/// IDA: 00643640 LowProcess vt+0x20.
pub fn process_update_stamps(world: &mut World, process: Entity) {
    let (hour, date) = process_stamps(world);
    if let Some(mut low) = world.get_mut::<LowProcess>(process) {
        low.cur_hour_14 = hour;
        low.cur_packed_date_18 = date;
    }
}

// IDA: N/A a helper, with no address of its own.
fn base_of(world: &World, process: Entity) -> BaseProcess {
    *world.get::<BaseProcess>(process).expect("a process")
}

// IDA: N/A a helper, with no address of its own.
fn low_mut(world: &mut World, process: Entity) -> Mut<'_, LowProcess> {
    world.get_mut::<LowProcess>(process).expect("a LowProcess")
}

// IDA: 0041F4C0 the 0x1E extra: a new 0x20 one (0042AE70: +0x0C the world or else the cell, the position, rotZ), or on an existing one +0x0C and the position alone.
fn set_package_start_location(
    world: &mut World,
    reference: Entity,
    space: Option<Entity>,
    cell: Option<Entity>,
    position: [f32; 3],
    rotation_z: f32,
) {
    let location = space.or(cell);
    let bits = position.map(f32::to_bits);
    let list = &mut world
        .get_mut::<TesObjectRefr>(reference)
        .expect("a reference")
        .into_inner()
        .extra_data_44;
    if let Some(entry) = list.get_mut(EXTRA_PACKAGE_START) {
        let time = match entry.data {
            ExtraData::PackageStart { time_1c, .. } => time_1c,
            _ => 0,
        };
        entry.data = ExtraData::PackageStart {
            target_0c: 0,
            location_10: bits,
            time_1c: time,
        };
        entry.resolved_0c = Some(location);
        return;
    }
    let mut entry = new_extra_entry(
        EXTRA_PACKAGE_START,
        ExtraData::PackageStart {
            target_0c: 0,
            location_10: bits,
            time_1c: rotation_z.to_bits(),
        },
    );
    entry.resolved_0c = Some(location);
    list.add(entry);
}

// IDA: 004D7A20 (actor, world, cell, position, rotZ) -> 0041F4C0, as 00648EC0 and 00649772 push them: +0x28, vt+0x174, 006ECC80, 004D6670.
fn record_start_location(world: &mut World, actor: Entity) -> Result<(), StreamError> {
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

// IDA: 0041FFC0, 0042A440: the 0x21 extra when missing; an entry takes the day; otherwise prepended (00446CB0), then the package and the day byte.
fn run_once_add(world: &mut World, actor: Entity, package: Entity, day: u8) {
    let list = &mut world
        .get_mut::<TesObjectRefr>(actor)
        .expect("an actor")
        .into_inner()
        .extra_data_44;
    if list.get(EXTRA_RUN_ONCE_PACKS).is_none() {
        list.add(new_extra_entry(
            EXTRA_RUN_ONCE_PACKS,
            ExtraData::RunOncePacks(Vec::new()),
        ));
    }
    let entry = list.get_mut(EXTRA_RUN_ONCE_PACKS).expect("just added");
    if let ExtraData::RunOncePacks(items) = &mut entry.data {
        if let Some(item) = items.iter_mut().find(|item| item.0 == package) {
            item.1 = day;
            return;
        }
        items.insert(0, (package, day));
    }
}

/// IDA: 00648E40 LowProcess_SelectPackage(actor): 005E0330 (00468670 -> 00569020); a change from a nonnull package to none or one below 0xFF000000 clears the actor's change flags 0x30000; +0x08 the new one; a new one with no location or of type 2 records the start location. Completion: 100%.
fn process_select_package(
    world: &mut World,
    process: Entity,
    actor: Entity,
    arms: &mut dyn AiArms,
) -> Result<(), StreamError> {
    let selected = match actor_package_list(world, actor)? {
        Some(list) => select_from_list(world, &list, actor, arms)?,
        None => None,
    };
    let old = base_of(world, process).editor_package_08;
    if old.is_some() && selected != old {
        let temporary = selected.is_some_and(|selected| {
            world
                .get::<FormIdentity>(selected)
                .expect("a form")
                .form_id_0c
                >= 0xFF00_0000
        });
        if !temporary {
            clear_change_flags(world, actor, 0x3_0000)?;
        }
    }
    world
        .get_mut::<BaseProcess>(process)
        .expect("a process")
        .editor_package_08 = selected;
    if let Some(selected) = selected {
        let location = package_data(world, selected).location_24;
        if location.is_none_or(|location| location.type_00 == 2) {
            record_start_location(world, actor)?;
        }
    }
    Ok(())
}

// IDA: 00673980 the sequence length by the table at 006739AC, with the 0x2C end; -1 past 0x27.
const fn procedure_count(kind: i32) -> i32 {
    match kind {
        0 | 7 | 12 | 14 | 15 | 16 | 19 | 21 | 24 | 25 | 29 | 31 | 34..=39 => 2,
        1 | 2 | 3 | 6 | 9 | 13 | 17 | 18 | 20 | 23 => 3,
        4 | 5 | 10 | 22 | 26 | 28 | 30 | 32 => 4,
        8 | 11 | 27 | 33 => 5,
        _ => -1,
    }
}

// IDA: 00648120 LowProcess vt+0x188(actor, step): +0x04 += step, floored at 0; past the sequence the last index, and an end procedure raises 0x400.
pub(super) fn process_advance_procedure(
    world: &mut World,
    process: Entity,
    actor: Entity,
    step: i32,
) -> Result<(), StreamError> {
    let mut base = base_of(world, process);
    let index = (base.edit_pack_procedure_type_04 as i32 + step).max(0);
    base.edit_pack_procedure_type_04 = index as u32;
    world.entity_mut(process).insert(base);
    let Some(package) = base.editor_package_08 else {
        return Ok(());
    };
    let count = procedure_count(package_data(world, package).field_18 as i32);
    if index >= count {
        if count < 1 {
            return Err(StreamError("0064816F: a -1 count reads before the row"));
        }
        let last = (count - 1) as u32;
        world
            .get_mut::<BaseProcess>(process)
            .expect("a process")
            .edit_pack_procedure_type_04 = last;
        if procedure_at(world, package, last)? == 0x2C {
            script_event_or(world, actor, 0x400, Some(package));
        }
    }
    Ok(())
}

// IDA: 00649190 LowProcess vt+0x55C(actor): a type 0 location's reference into +0x30; 00569A10 `cmp byte,3; setg` is signed, so types up to 3 and any negative do nothing (the C++ casts to unsigned); above, without flag 4 the procedure advances. With flag 4 the 00646A80 search, a host stop.
// TODO: [ ] 006491ED the +0x4C candidates from 00446B90 and 00646A80: a flag 4 package with a type 4 or 5 location.
fn process_update_location_target(
    world: &mut World,
    process: Entity,
    actor: Entity,
) -> Result<(), StreamError> {
    let Some(package) = base_of(world, process).editor_package_08 else {
        return Ok(());
    };
    let data = package_data(world, package);
    let flags = data.flags_1c;
    let Some(location) = data.location_24 else {
        return Ok(());
    };
    if location.type_00 == 0 {
        let reference = location_reference(world, &location)?;
        low_mut(world, process).field_30 = reference;
        return Ok(());
    }
    if location.type_00 <= 3 {
        return Ok(());
    }
    if flags & 4 == 0 {
        return process_advance_procedure(world, process, actor, 1);
    }
    Err(StreamError("006491ED: the 00646A80 candidate search"))
}

// IDA: 005E6E00 Actor_UpdatePackageArrival: a living actor with a process, a package and a cell, not combat-like; flag 8 locks, else 0x40 unlocks; at the location with +0x84 clear, flag 0x20 locks and stores +0x84 = 1, else 0x100 unlocks. The process is re-read at each of its calls.
pub(super) fn package_arrival(world: &mut World, actor: Entity) -> Result<(), StreamError> {
    let Some(process) = process_of(world, actor) else {
        return Ok(());
    };
    if actor_is_dead(world, actor, false) {
        return Ok(());
    }
    let Some(package) = base_of(world, process).editor_package_08 else {
        return Ok(());
    };
    if parent_cell(world, actor).is_none() || package_is_combat_like(package_data(world, package)) {
        return Ok(());
    }
    let flags = package_data(world, package).flags_1c;
    if flags & 8 != 0 {
        doors_near(world, actor, true)?;
    } else if flags & 0x40 != 0 {
        doors_near(world, actor, false)?;
    }
    if !package_at_location(world, package, actor, false, -1.0)? {
        return Ok(());
    }
    let Some(process) = process_of(world, actor) else {
        return Ok(());
    };
    if world
        .get::<LowProcess>(process)
        .is_some_and(|low| low.byte_84 != 0)
    {
        return Ok(());
    }
    if flags & 0x20 != 0 {
        doors_near(world, actor, true)?;
        low_mut(world, process).byte_84 = 1;
        return Ok(());
    }
    if flags & 0x100 != 0 {
        doors_near(world, actor, false)?;
    }
    Ok(())
}

// IDA: 004D89D0 (float): +0x20, then vt+0x40(4).
fn set_rotation_x(world: &mut World, reference: Entity, value: f32) -> Result<(), StreamError> {
    world
        .get_mut::<TesObjectRefr>(reference)
        .expect("a reference")
        .rotation_20[0] = value;
    set_change_flags(world, reference, 4)
}

// IDA: 004DD260 TESObjectREFR_Activate(activator, 0, 0, 1) for an NPC activator with a LowProcess: 00B35F04 = 0 (only read by the script arm); flag 0x2000 returns; a mounted activator skips base type 0x12; an 0x13 extra without bit 0 runs the activation scripts, a host stop; types 0x1C, 0x1E return; base vt+0xCC: TESFurniture 004AE8C0's player cast fails, 0; others stop. The return isn't read.
// TODO: [ ] 004DD3AC the activation script; base vt+0xCC of doors 004B8A60, activators 004B3CA0 and so on.
fn activate_reference(
    world: &World,
    reference: Entity,
    activator: Entity,
) -> Result<(), StreamError> {
    if form_flags(world, reference) & 0x2000 != 0 {
        return Ok(());
    }
    let data = world.get::<TesObjectRefr>(reference).expect("a reference");
    let base = data
        .base_form_1c
        .ok_or(StreamError("004DD3E7: a base-less activation target"))?;
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
        return Err(StreamError("004DD38F: an activation by the player"));
    }
    if let Some(entry) = data.extra_data_44.get(0x13)
        && matches!(&entry.data, ExtraData::Action { flags_0c, .. } if flags_0c & 1 == 0)
    {
        return Err(StreamError("004DD3AC: the activation script run"));
    }
    match kind {
        0x1C | 0x1E | 0x20 => Ok(()),
        _ => Err(StreamError("004DD469: base vt+0xCC Activate")),
    }
}

/// IDA: 0064A380 LowProcess vt+0x14(reference, events): the scheduled packages since the last stamp: each whose conditions pass becomes current in turn and places the actor; the final cell move (004DD4B0) and +0x08 cleared on `this`; then +0x84 = 0 on the actor's current +0x58, re-read at 0064A878. With `events` 0x200 and 0x400 are raised before each store; 005ED860 passes 0. Completion: 100%, with `activate_reference`'s stops.
pub(super) fn process_run_scheduled(
    world: &mut World,
    process: Entity,
    actor: Entity,
    events: bool,
    arms: &mut dyn AiArms,
) -> Result<(), StreamError> {
    let current = base_of(world, process).editor_package_08;
    if let Some(current) = current
        && package_data(world, current).flags_1c & (4 | 0x4000 | 2) != 0
    {
        low_mut(world, process).byte_84 = 0;
        return Ok(());
    }
    let scheduled = match actor_package_list(world, actor)? {
        Some(list) => {
            let hour = game_hour(world);
            collect_scheduled_packages(world, &list, process, hour)?
        }
        None => Vec::new(),
    };
    let mut cell = None;
    let mut space = None;
    let mut rotate = false;
    for candidate in scheduled {
        let data = package_data(world, candidate);
        let flags = data.flags_1c;
        let kind = data.type_20;
        let location = data.location_24;
        let target = data.target_28;
        let conditions = data.conditions_34.clone();
        let target_ref = match &target {
            Some(target) => target_reference(world, target)?,
            None => None,
        };
        low_mut(world, process).byte_84 = 0;
        if !evaluate_conditions(world, &conditions, Some(actor), target_ref, arms)? {
            continue;
        }
        if events {
            let old = base_of(world, process).editor_package_08;
            script_event_or(world, actor, 0x200, Some(candidate));
            script_event_or(world, actor, 0x400, old);
        }
        world
            .get_mut::<BaseProcess>(process)
            .expect("a process")
            .editor_package_08 = Some(candidate);
        package_arrival(world, actor)?;
        process_update_location_target(world, process, actor)?;
        let mut place = |world: &mut World| -> Result<(), StreamError> {
            let where_ = package_location_position(world, candidate, actor)?;
            set_reference_position(world, actor, where_)?;
            cell = package_location_cell(world, candidate, actor)?;
            space = package_location_world(world, candidate, actor)?;
            Ok(())
        };
        // 0064A894 by +0x20, unsigned: a negative type takes the default.
        match kind {
            5 | 6 => {
                place(world)?;
                rotate = location.is_some_and(|location| location.type_00 == 1);
            }
            0 => {
                if let Some(reference) =
                    target_ref.filter(|&form| form_flags(world, form) & 0x20 == 0)
                {
                    let position = reference_position(world, reference)?;
                    set_reference_position(world, actor, position)?;
                    rotate = true;
                    if world.get::<Actor>(reference).is_none()
                        && form_flags(world, reference) & (0x800 | 0x20) == 0
                    {
                        activate_reference(world, reference, actor)?;
                    }
                    cell = parent_cell(world, reference);
                    space = reference_world(world, reference);
                } else {
                    place(world)?;
                    rotate = false;
                }
            }
            3 | 4 => {
                place(world)?;
                rotate = match &location {
                    Some(location) => location_reference(world, location)?.is_some(),
                    None => false,
                };
            }
            1 => {
                if let Some(target) = &target
                    && (target_ref.is_some()
                        || target_object(target)?.is_some()
                        || target_object_type(target) != 0)
                    && let Some(reference) = target_ref.filter(|&form| {
                        form_flags(world, form) & 0x20 == 0 && Some(form) != the_player(world)
                    })
                {
                    let position = reference_position(world, reference)?;
                    set_reference_position(world, actor, position)?;
                    cell = parent_cell(world, reference);
                    space = reference_world(world, reference);
                    rotate = false;
                }
            }
            _ => {}
        }
        package_arrival(world, actor)?;
        if flags & 0x400 != 0 {
            let day = game_day_byte(world);
            run_once_add(world, actor, candidate, day);
        }
        if flags & 0x10 != 0 {
            doors_near(world, actor, true)?;
        } else if flags & 0x80 != 0 {
            doors_near(world, actor, false)?;
        }
    }
    if cell.is_some() || space.is_some() {
        if rotate {
            set_rotation_x(world, actor, super::ai_package::FLT_MAX)?;
        }
        move_to_cell(world, actor, cell, space)?;
        world
            .get_mut::<BaseProcess>(process)
            .expect("a process")
            .editor_package_08 = None;
    }
    if let Some(current) = process_of(world, actor) {
        low_mut(world, current).byte_84 = 0;
    }
    Ok(())
}

// IDA: 00424C50 the 0x23 extra when missing (00429B20); a follower not listed on any node is prepended (the C++ stops looking at a null item).
fn follower_add(world: &mut World, list_owner: Entity, follower: Entity) {
    let list = &mut world
        .get_mut::<TesObjectRefr>(list_owner)
        .expect("a reference")
        .into_inner()
        .extra_data_44;
    if list.get(EXTRA_FOLLOWER).is_none() {
        list.add(new_extra_entry(
            EXTRA_FOLLOWER,
            ExtraData::Followers(Vec::new()),
        ));
    }
    if let ExtraData::Followers(items) = &mut list.get_mut(EXTRA_FOLLOWER).expect("just added").data
        && !items.contains(&follower)
    {
        items.insert(0, follower);
    }
}

// IDA: 005E03C0 process vt+0xD0 (Low 00643890): a nonnull target gets 0x40000 (004D6F80(1)); +0x2C.
fn set_package_target(world: &mut World, actor: Entity, target: Option<Entity>) {
    let Some(process) = process_of(world, actor) else {
        return;
    };
    if let Some(target) = target {
        world
            .get_mut::<FormIdentity>(target)
            .expect("a form")
            .flags_08 |= 0x4_0000;
    }
    low_mut(world, process).field_2c = target;
}

// IDA: 00568BB0 TESPackage_ApplyTarget(actor): type 0 the reference, a deleted one through its 0x22 extra; a Follow or Escort of an actor lists the actor as its follower, an Accompany the reverse; 005E03C0 records it, null too.
// TODO: [ ] 00568C45, 00568BE1 the type 1 and 2 searches through 00446B90 and 00567730; 00568D88 following the player.
fn package_apply_target(
    world: &mut World,
    package: Entity,
    actor: Entity,
) -> Result<(), StreamError> {
    let data = package_data(world, package);
    let kind = data.type_20 as u8;
    let flags = data.flags_1c;
    let Some(target) = data.target_28 else {
        return Ok(());
    };
    let mut reference = None;
    match target.type_00 {
        0 => {
            reference = target_reference(world, &target)?;
            if let Some(form) = reference
                && form_flags(world, form) & 0x20 != 0
            {
                let list = &world
                    .get::<TesObjectRefr>(form)
                    .expect("a reference")
                    .extra_data_44;
                reference = list
                    .get(EXTRA_REFERENCE_POINTER)
                    .and_then(|entry| entry.resolved_0c.flatten())
                    .filter(|&form| world.get::<TesObjectRefr>(form).is_some());
            }
        }
        1 => return Err(StreamError("00568C45: the type 1 target search")),
        2 => return Err(StreamError("00568BE1: the type 2 target search")),
        _ => {}
    }
    if let Some(form) = reference
        && world.get::<Actor>(form).is_some()
        && flags & 0x800 == 0
    {
        if kind == 1 || kind == 7 {
            if Some(form) == the_player(world) {
                return Err(StreamError("00568D88: following the player"));
            }
            follower_add(world, form, actor);
        } else if kind == 2 {
            follower_add(world, actor, form);
        }
    }
    set_package_target(world, actor, reference);
    Ok(())
}

// IDA: 00567CA0 whether the target is a follow-able actor.
fn package_target_is_actor(world: &World, package: Entity) -> Result<bool, StreamError> {
    let Some(target) = package_data(world, package).target_28 else {
        return Ok(false);
    };
    Ok(match target.type_00 {
        0 => {
            target_reference(world, &target)?.is_some_and(|form| world.get::<Actor>(form).is_some())
        }
        1 => target_object(&target)?.is_some_and(|form| matches!(form_type(world, form), 35 | 36)),
        2 => target_object_type(&target).wrapping_sub(15) <= 1,
        _ => false,
    })
}

/// IDA: 00649340 LowProcess vt+0x18(reference, force). Completion: 100%, past the 0x800 arm 0064958F.
/// TODO: [ ] 0064959C 005EAE70, a flag 0x800 package ending; 006497C4 draining a nonempty +0x3C.
#[allow(
    clippy::too_many_lines,
    reason = "one function in the exe, in its order"
)]
pub(super) fn process_evaluate_package(
    world: &mut World,
    process: Entity,
    actor: Entity,
    force: bool,
    arms: &mut dyn AiArms,
) -> Result<bool, StreamError> {
    let hour = game_hour(world);
    let mut force = force;
    let mut old = base_of(world, process).editor_package_08;
    if !force && let Some(old) = old {
        // 005E6BA0 a type 0x0F; 00649377 vt+0x334(1) 005E6110: 0x0C or 0x0D. Both return false; the C++ stops on the second.
        let kind = package_data(world, old).type_20;
        if kind as u8 == 15 || kind == 12 || kind == 13 {
            return Ok(false);
        }
    }
    let mut result = false;
    if let Some(previous) = old {
        if package_data(world, previous).field_18 == 0xFFFF_FFFF {
            package_init_procedure(world, previous)?;
        }
        let data = package_data(world, previous);
        let flags = data.flags_1c;
        let kind = data.type_20 as u8;
        let schedule = data.schedule_2c;
        let duration = schedule.duration_04 as i32;
        let index = base_of(world, process).edit_pack_procedure_type_04;
        if duration != 0 && flags & 4 != 0 {
            let mut check = true;
            if kind != 5 && kind != 4 && kind != 3 && procedure_at(world, previous, index)? != 0x2C
            {
                check = false;
            }
            let mut done = false;
            if check {
                let start = i32::from(schedule.data_00[3] as i8);
                let mut end = if start == -1 {
                    x87::ftol(
                        world
                            .get::<LowProcess>(process)
                            .expect("a LowProcess")
                            .field_0c,
                    )
                    .wrapping_add(duration)
                } else {
                    start.wrapping_add(duration)
                };
                if end > 23 {
                    end -= 23;
                } else if start as f32 > hour {
                    result = true;
                }
                if x87::ftol(hour) >= end {
                    result = true;
                } else if !result && force {
                    force = false;
                }
                script_event_or(world, actor, 0x400, Some(previous));
                if flags & 0x400 != 0 {
                    let day = game_day_byte(world);
                    run_once_add(world, actor, previous, day);
                }
                done = result;
            }
            // 0064957D.
            if !done && (!force || flags & 0x4000 != 0) {
                return Ok(result);
            }
        } else if flags & 4 != 0 {
            if procedure_at(world, previous, index)? != 0x2C {
                return Ok(false);
            }
            result = true;
        } else if flags & 2 == 0 {
            result = true;
        } else {
            if !package_at_location(world, previous, actor, false, -1.0)?
                && procedure_at(world, previous, index)? != 0x2C
            {
                return Ok(result);
            }
            result = true;
        }
        // 0064958F.
        if flags & 0x800 != 0 {
            return Err(StreamError(
                "0064959C: 005EAE70, a flag 0x800 package ending",
            ));
        }
        process_select_package(world, process, actor, arms)?;
        if base_of(world, process).editor_package_08 == Some(previous) {
            return Ok(false);
        }
        let player_cell = the_player(world).and_then(|player| parent_cell(world, player));
        if flags & 0x200 != 0 && player_cell == parent_cell(world, actor) {
            // vt+0x38C(1) on LowProcess 0068F970 is a no-op.
            world
                .get_mut::<BaseProcess>(process)
                .expect("a process")
                .editor_package_08 = Some(previous);
            return Ok(false);
        }
        low_mut(world, process).field_0c = hour;
        world
            .get_mut::<BaseProcess>(process)
            .expect("a process")
            .edit_pack_procedure_type_04 = 0;
    } else {
        process_select_package(world, process, actor, arms)?;
        old = base_of(world, process).editor_package_08;
        result = old.is_some();
        low_mut(world, process).field_0c = hour;
        world
            .get_mut::<BaseProcess>(process)
            .expect("a process")
            .edit_pack_procedure_type_04 = 0;
        let mut low = low_mut(world, process);
        low.field_2c = None;
        low.byte_1c = 0; // vt+0x394(0) 006292D0.
    }
    if !result {
        return Ok(result);
    }
    // 006493EC.
    let current = base_of(world, process).editor_package_08;
    if let Some(old_package) = old
        && current != old
    {
        let flags = package_data(world, old_package).flags_1c;
        if flags & 0x10 != 0 {
            doors_near(world, actor, true)?;
        } else if flags & 0x80 != 0 {
            doors_near(world, actor, false)?;
        }
    }
    let current = base_of(world, process).editor_package_08;
    if let Some(package) = current
        && current != old
    {
        let flags = package_data(world, package).flags_1c;
        if flags & 8 != 0 {
            doors_near(world, actor, true)?;
        } else if flags & 0x40 != 0 {
            doors_near(world, actor, false)?;
        }
    }
    let current = base_of(world, process).editor_package_08;
    if let Some(package) = current {
        // 00649702 005EAE10: with old == current, vt+0x334(1) false gives the zeroed +0x2C; a 0x0C or 0x0D goes to vt+0x338's chain.
        let kind = package_data(world, package).type_20;
        if old == current && (kind == 12 || kind == 13) {
            return Err(StreamError("00649702: 005EAE10's 005E0AF0 chain"));
        }
        if package_target_is_actor(world, package)? && kind != 0 {
            package_apply_target(world, package, actor)?;
        }
    }
    let current = base_of(world, process)
        .editor_package_08
        .ok_or(StreamError("00566DDD: 00566DC0 on a null package faults"))?;
    if package_at_location(world, current, actor, false, -1.0)? {
        package_arrival(world, actor)?;
    }
    low_mut(world, process).byte_84 = 0;
    record_start_location(world, actor)?;
    let mut low = low_mut(world, process);
    low.field_2c = None;
    // vt+0x174 0 skips vt+0x178; vt+0xBC(0) 007790C0 and vt+0x394(0) 006292D0.
    low.byte_1d = 0;
    low.byte_1c = 0;
    if low.list_3c != [0; 2] {
        return Err(StreamError("006497C4: freeing the +0x3C list"));
    }
    Ok(result)
}

// IDA: 004F9EC0 (actor, +0x44): a 0x12 extra with an event list whose +0x08 is null runs 00516970 and 00517950. The list comes out of the extra for the run. Completion: 100%.
pub(crate) fn run_pending_script(world: &mut World, actor: Entity) -> Result<(), StreamError> {
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
    let result =
        super::script_vm::run_blocks_only(world, script, Some(actor), &mut events, Some(actor));
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

// IDA: 004CBB20(0x1C, 1) as 005ED965 calls it, inline in the C++: the first non-deleted reference of base type 0x1C in +0x48.
fn cell_find_marker(world: &World, cell: Entity) -> Option<Entity> {
    world
        .get::<TesObjectCell>(cell)?
        .objects_48
        .items()
        .find(|&form| {
            let Some(data) = world.get::<TesObjectRefr>(form) else {
                return false;
            };
            form_flags(world, form) & 0x20 == 0
                && data
                    .base_form_1c
                    .is_some_and(|base| form_type(world, base) == 0x1C)
        })
}

/// IDA: 005ED860 Actor_UpdatePackageLocation (`this` the actor): the process is re-read at each of the exe's nine reads, since a move could change it. Completion: 100%.
pub fn evaluate_actor_package(
    world: &mut World,
    actor: Entity,
    arms: &mut dyn AiArms,
) -> Result<(), StreamError> {
    if form_flags(world, actor) & 0x800 != 0 {
        return Ok(());
    }
    let process = process_of(world, actor).ok_or(StreamError("005ED874: no process faults"))?;
    world.resource_mut::<AiProcessGlobals>().packages_evaluated += 1;
    process_update_stamps(world, process);
    if reference_is_persistent(world, actor)? {
        process_run_scheduled(world, process, actor, false, arms)?;
    }
    let process = process_of(world, actor).ok_or(StreamError("005ED896: no process faults"))?;
    process_evaluate_package(world, process, actor, false, arms)?;
    let process = process_of(world, actor).ok_or(StreamError("005ED8A3: no process faults"))?;
    let package = base_of(world, process).editor_package_08;
    if let Some(package) = package
        && package_data(world, package).type_20 > 2
        && !package_at_location(world, package, actor, false, -1.0)?
    {
        let mut horse = None;
        if package_data(world, package).flags_1c & 0x80_0000 != 0 {
            let list = &world
                .get::<TesObjectRefr>(actor)
                .expect("an actor")
                .extra_data_44;
            if let Some(entry) = list.get(EXTRA_HORSE) {
                horse = match entry.resolved_0c {
                    Some(form) => form.filter(|&form| world.get::<Actor>(form).is_some()),
                    None => return Err(StreamError("00420840: an unresolved travel horse")),
                };
            }
        }
        let cell = package_location_cell(world, package, actor)?;
        let space = package_location_world(world, package, actor)?;
        move_to_cell(world, actor, cell, space)?;
        set_rotation_x(world, actor, super::ai_package::FLT_MAX)?;
        let process = process_of(world, actor).ok_or(StreamError("005ED939: no process faults"))?;
        let package = base_of(world, process).editor_package_08;
        let location = package.and_then(|package| package_data(world, package).location_24);
        if location.is_some_and(|location| location.type_00 == 1) {
            if let Some(cell) =
                cell.filter(|&cell| super::forms_world::cell_is_interior(world, cell))
            {
                let marker =
                    cell_find_marker(world, cell).or_else(|| cell_first_reference(world, cell));
                if let Some(marker) = marker {
                    let position = reference_position(world, marker)?;
                    actor_set_position(world, actor, position)?;
                }
            }
        } else if let Some(package) = package {
            let position = package_location_position(world, package, actor)?;
            actor_set_position(world, actor, position)?;
            if let Some(horse) = horse {
                move_to_cell(world, horse, cell, space)?;
                set_rotation_x(world, horse, super::ai_package::FLT_MAX)?;
                let position = package_location_position(world, package, actor)?;
                actor_set_position(world, horse, position)?;
            }
        }
    }
    package_arrival(world, actor)
}

/// IDA: 00675310 ActorProcessManager_ProcessTrackedActors (`__thiscall` on 00B3BD00) over the +0x18 list from its head: an empty head returns; 004F9EC0; a live, based actor with Health above 0 runs 005ED860 and the walk goes on; a based actor at Health <= 0 gets 005E6680(2); then with a process, 0067B320(list, actor, previous) and the walk resumes after the previous node, or at the head; with none the walk goes on. A non-actor item spins on the same node in the exe, which the list's entities can't be. Completion: 100%.
pub fn run_ai_package_pass(world: &mut World, arms: &mut dyn AiArms) -> Result<(), StreamError> {
    world.init_resource::<AiProcessGlobals>();
    let mut node = Some(ActorListNode::HEAD);
    let mut previous: Option<ActorListNode> = None;
    while let Some(current) = node {
        let manager = &world.resource::<ActorProcessGlobals>().manager;
        let list = &manager.low_18.list;
        let Some(actor) = list.item(current) else {
            if list.next(current).is_none() {
                return Ok(());
            }
            return Err(StreamError(
                "00675333: a null actor in the level-3 list faults",
            ));
        };
        world.resource_mut::<AiProcessGlobals>().actors_visited += 1;
        run_pending_script(world, actor)?;
        let deleted = form_flags(world, actor) & 0x20 != 0;
        let based = world
            .get::<TesObjectRefr>(actor)
            .expect("an actor")
            .base_form_1c
            .is_some();
        if !deleted && based && actor_get_actor_value(world, actor, 8)? > 0 {
            evaluate_actor_package(world, actor, arms)?;
            previous = Some(current);
            node = world
                .resource::<ActorProcessGlobals>()
                .manager
                .low_18
                .list
                .next(current);
            continue;
        }
        if !deleted && based && actor_get_actor_value(world, actor, 8)? <= 0 {
            set_death_state(world, actor, 2)?;
        }
        if process_of(world, actor).is_none() {
            previous = Some(current);
            node = world
                .resource::<ActorProcessGlobals>()
                .manager
                .low_18
                .list
                .next(current);
            continue;
        }
        world.resource_mut::<AiProcessGlobals>().actors_removed += 1;
        let active = save_load_active(world)?;
        let mut globals = world.resource_mut::<ActorProcessGlobals>();
        actor_process_list_remove(&mut globals.manager, ActorListId::Low18, actor, previous);
        release_deleted_process(world, actor, active);
        let list = &world.resource::<ActorProcessGlobals>().manager.low_18.list;
        node = match previous {
            Some(previous) => list.next(previous),
            None => Some(ActorListNode::HEAD),
        };
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // 0051CA10: a PC-level creature's health is the 16-bit value times the level, at least 1.
    #[test]
    fn a_pc_level_creature_scales_its_health() {
        let mut world = World::new();
        let player = world.spawn(TesObjectRefr::default()).id();
        world.init_resource::<super::super::player::PlayerGlobals>();
        world
            .resource_mut::<super::super::player::PlayerGlobals>()
            .the_player_b333c4 = Some(player);
        let base = world
            .spawn((
                TesCreature::default(),
                TesHealthForm {
                    original_vtable: 0,
                    health_04: 7,
                },
                TesActorBaseData {
                    flags_04: 0x80,
                    level_0e: 3,
                    ..TesActorBaseData::default()
                },
            ))
            .id();
        assert_eq!(base_actor_value(&world, base, 8).unwrap(), 21);
        world.get_mut::<TesActorBaseData>(base).unwrap().flags_04 = 0;
        assert_eq!(base_actor_value(&world, base, 8).unwrap(), 7);
        // Two bytes only.
        world.get_mut::<TesHealthForm>(base).unwrap().health_04 = 0x1_0007;
        assert_eq!(base_actor_value(&world, base, 8).unwrap(), 7);
    }

    #[test]
    fn the_procedure_counts_follow_the_table() {
        assert_eq!(procedure_count(0), 2);
        assert_eq!(procedure_count(1), 3);
        assert_eq!(procedure_count(8), 5);
        assert_eq!(procedure_count(40), -1);
        assert_eq!(procedure_count(-1), -1);
    }
}
