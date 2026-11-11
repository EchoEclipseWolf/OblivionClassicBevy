//! An actor's idle pick from the IdleAnims tree: process vt+0x384 (`0064B0B0` on a MiddleHigh or High process), `00521450`'s
//! root by the actor's model path and `005206B0`'s depth-first walk over the TESIdleForms, whose conditions draw (a
//! GetRandomPercent per `004F4EF0`). RaceSexMenu_Create reaches it for the player at `005CC1A1`: run-ab's 11 draws.
//!
//! `004762B0` plays a pick on the actor's animation data: the save-load arm, `00474C50`'s object animations and KF lookup are
//! ported; the KF's add (`004753F0` → `00474070`, which wants the model services the interface doesn't lend) and a KF the
//! map lacks (`004383B0`'s QueuedAnimIdle) stay host deferral D22, recorded in `IdlePlayDeferrals`. New Game's race menu
//! doesn't reach it: run-af's 0064B0B0 returns 0 at 005CC1A7 (its walk finds no idle), and so does the port's.

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;

use super::actor::MobileObject;
use crate::tes::ai_process::AiArms;
use crate::tes::form_components::TesModel;
use crate::tes::forms_ai::{TesIdleForm, find_actor_idle};

/// IDA: N/A 0057C140 InterfaceManager_IsMenuInputActive's inputs, read from the interface before the walk: whether the
/// manager has a cursor and a menu root whose user0 is 2.0, the top stacked menu mode (0057CF60) and, for the Big Four
/// (Stats 3EB, Inventory 3EA, Magic 3FE, Map 3FF) under mode 1, 005790E0's answer for each.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MenuInputAnswers {
    pub ready: bool,
    pub top_mode: u32,
    pub big_four_shown: [(u32, bool); 4],
}

impl MenuInputAnswers {
    /// IDA: 0057C140 InterfaceManager_IsMenuInputActive(menu). Completion: 100%.
    #[must_use]
    pub fn active(&self, menu: u32) -> bool {
        if !self.ready {
            return false;
        }
        if self.top_mode == 1
            && let Some(&(_, shown)) = self.big_four_shown.iter().find(|(id, _)| *id == menu)
        {
            return shown;
        }
        self.top_mode == menu
    }
}

// IDA: N/A the script's arms with the interface's answer for MenuMode with an ID.
pub(crate) struct IdleArms(MenuInputAnswers);

impl AiArms for IdleArms {
    /// IDA: N/A `IdleArms`'s side of `AiArms`.
    fn random_next(&mut self, world: &mut World) -> Result<i32, GameError> {
        crate::tes::ai_conditions::ScriptArms.random_next(world)
    }
    /// IDA: N/A `IdleArms`'s side of `AiArms`.
    fn menu_mode(&mut self, world: &mut World) -> Result<bool, GameError> {
        crate::tes::ai_conditions::ScriptArms.menu_mode(world)
    }
    /// IDA: N/A `IdleArms`'s side of `AiArms`.
    fn menu_input_active(&mut self, _world: &mut World, menu: u32) -> Result<bool, GameError> {
        Ok(self.0.active(menu))
    }
}

/// IDA: N/A which arm of `004762B0` a deferred play would take: the KF in the map (`00474C50` sets +0x00 = 1 and 004753F0
/// adds it through `00474070`), or a miss (004383B0 queues a QueuedAnimIdle whose finish plays it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdlePlayArm {
    AddKf(crate::bs::kf_model::KfModelId),
    QueuedAnimIdle,
}

/// IDA: N/A host deferral D22: each `004762B0(anim data, idle, actor, group)` the port reaches but doesn't finish.
#[derive(Resource, Clone, Debug, Default, PartialEq, Eq)]
pub struct IdlePlayDeferrals {
    /// The actor, the idle, its group (`00520200`) and the arm.
    pub picks: Vec<(Entity, Entity, u8, IdlePlayArm)>,
}

/// IDA: 004762B0 ActorAnimData_PlayIdle(anim, idle, actor, group): FormHeap(0x2C) and `00474C50`(idle, group, 0, actor,
/// g_saveLoadGame +0x18 bit 0x800) into +0xD0, then `004753F0`. `00474C50`: +0x10 null, +0x0C the group, +0x04 0, +0x24 the
/// idle, +0x00 0; +0x14 and +0x18 DataHandler_FindIdleObjectAnimation (`00449040`: the first and second TESObjectANIO in
/// DataHandler +0xB4 whose +0x30 is the idle); +0x28 the actor; the KF "Meshes\%s" of the idle's TESModelAnim (+0x18 vt+0x14);
/// with the save-load bit 00439FF0 LoadKFNow, else `004383B0` (the map's KFModel, or a queued QueuedAnimIdle and null); with
/// a KF, each object animation's model through 00479450 under the actor, a reference (+0x0C) on the KF and +0x00 = 1.
/// `004753F0`: a loaded object's KF through 00474070, then 004729F0 and +0xD0 = null. Completion: the arms up to the add;
/// the save-load arm and a loaded KF's object animations are stops, and the add and the queue are D22's record (no state
/// changes: +0xD0 and the KF's reference aren't stored).
/// # Errors
/// The stops, or no ModelLoader.
pub fn anim_data_play_idle(
    world: &mut World,
    idle: Entity,
    actor: Entity,
    group: u8,
) -> Result<(), GameError> {
    if crate::tes::forms_references::save_load_flags(world)? & 0x800 != 0 {
        return Err(GameError::unclassified(
            0x004762C4,
            "004762C4: 004762B0's save-load arm (00439FF0 LoadKFNow)",
        ));
    }
    let animations = world
        .get_resource::<crate::tes::data_handler::DataHandlerLists>()
        .map(|lists| lists.object_animations_b4.items().collect::<Vec<_>>())
        .unwrap_or_default();
    let with_idle = animations.into_iter().any(|anio| {
        world
            .get::<crate::tes::forms_environment::TesObjectAnio>(anio)
            .is_some_and(|data| {
                data.idle_30 == crate::tes::form_components::FormRef::Resolved(Some(idle))
            })
    });
    let model = world
        .get::<TesModel>(idle)
        .map(|model| text(model.path_04.data_00.as_deref()).to_vec())
        .unwrap_or_default();
    let path = [b"Meshes\\".as_slice(), &model].concat();
    let arm = match crate::bs::kf_model::kf_map_find(world, &path)? {
        Some(kf) => IdlePlayArm::AddKf(kf),
        None => IdlePlayArm::QueuedAnimIdle,
    };
    if with_idle && matches!(arm, IdlePlayArm::AddKf(_)) {
        return Err(GameError::unclassified(
            0x00474D5E,
            "00474D5E: an idle's object animation under the actor (00479450)",
        ));
    }
    world
        .get_resource_or_init::<IdlePlayDeferrals>()
        .picks
        .push((actor, idle, group, arm));
    Ok(())
}

// IDA: N/A a BSStringT's bytes without the terminator.
pub(crate) fn text(bytes: Option<&[u8]>) -> &[u8] {
    let bytes = bytes.unwrap_or_default();
    bytes.split(|&byte| byte == 0).next().unwrap_or_default()
}

/// IDA: 0064B0B0 MiddleHighProcess_PlayIdleForActor(actor), process vt+0x384: the actor's vt+0x164 (the player's 0065D720:
/// +0x5DC, else the third person's animation data with +0x588 set, else +0x5CC), 00521450 with the process's +0x120 as the
/// target; with an idle and animation data, 004762B0(idle, actor, the idle's group 00520200) (`anim_data_play_idle`) and 1;
/// otherwise 0. Completion: 100% of the pick; the play's add is D22's.
/// # Errors
/// A process +0x120 the port can't resolve, or a stop in the conditions.
pub fn process_play_idle(
    world: &mut World,
    actor: Entity,
    answers: MenuInputAnswers,
) -> Result<bool, GameError> {
    let process = world
        .get::<MobileObject>(actor)
        .and_then(|mobile| mobile.process_58)
        .ok_or(GameError::unclassified(
            0x005CC19B,
            "005CC19B: the player with no process faults",
        ))?;
    let target = world
        .get::<super::actor_process::MiddleHighProcess>(process)
        .map(|middle| middle.furniture_reference_120)
        .ok_or(GameError::unclassified(
            0x0064B0CD,
            "0064B0CD: vt+0x384 on a process below MiddleHigh",
        ))?;
    if target.is_some() {
        return Err(GameError::unclassified(
            0x0064B0CD,
            "0064B0CD: a process +0x120 target",
        ));
    }
    // 0065D720 PlayerCharacter vt+0x164: +0x5DC, else with +0x588 set the third person's (004D8370: the process's +0x17C),
    // else +0x5CC or the third person's.
    let player =
        world
            .get::<super::player::PlayerCharacter>(actor)
            .ok_or(GameError::unclassified(
                0x0064B0B8,
                "0064B0B8: vt+0x164 on an actor that isn't the player",
            ))?;
    if player.first_person_biped_anim_data_5dc != 0 {
        return Err(GameError::unclassified(
            0x0065D726,
            "0065D726: the player's +0x5DC animation data",
        ));
    }
    let first_person = player.first_person_anim_data_5cc;
    let third = player.view_flags_588[0] != 0;
    let third_person = world
        .get::<super::actor_process::MiddleHighProcess>(process)
        .and_then(|middle| middle.anim_data_17c);
    let has_anim_data = if third {
        third_person.is_some()
    } else {
        first_person.or(third_person).is_some()
    };
    let Some(idle) = find_actor_idle(world, actor, None, &mut IdleArms(answers))? else {
        return Ok(false);
    };
    if !has_anim_data {
        return Ok(false);
    }
    let group = world.get::<TesIdleForm>(idle).expect("an idle").group_38 & 0x7F;
    anim_data_play_idle(world, idle, actor, group)?;
    Ok(true)
}
