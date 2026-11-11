//! 004D5BD0's reference loop (004D5C2E..004D5D3F) for a cell attached while the game runs, and the actor arm it takes: 005FA320 Actor_RebuildHighProcess and 00674E10.
//! Evidence: `evidence/gameplay-run002-2026-11-07/`: run-ab recorded the loop on the Imperial Prison (`1FBB9`), run-ac every call and return inside 005FA320 for its 11 actors, run-ad those inside 005EB400, 005EB370, 005E2E20 and the path grid's 004E5730.

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;

use crate::tes::actor::{Actor, TesObjectRefr, actor_rebuild_high_process};
use crate::tes::actors::actor_process_frame::process_manager_drop_queued;
use crate::tes::form::FormIdentity;
use crate::tes::form_components::FormRef;
use crate::tes::forms_world::TesObjectCell;

/// IDA: N/A what the loop reaches that the port doesn't run: 00438060 ModelLoader_QueueReference with the cell's priority, the IO manager's queue, which the host's drain doesn't fill yet.
pub type QueueReferenceCall<'f> = dyn FnMut(&mut World, Entity, i32) + 'f;

/// IDA: 004D5C2E..004D5D3F, 004D5BD0's loop over the cell's +0x48 list under 00496EA0's region lock (the host has no other thread in the cell's list). The walk reads each node's next before its body, so the list is taken first. Per reference: unless a save is loading, 004F9EC0 and 004FBF90(+0x44, 0x1000); flag 0x800 or 0x20 skips the rest; an actor, unless a save is loading without +0x18 bit 4, gets vt+0x1A4 (005FA320) and 00674E10; then 00438060 with `priority`, and vt+0x154 non-null runs 004CEAD0, a stop here. Completion: 100% of the order; 00438060 is the caller's.
/// TODO: [ ] 004CEAD0, for a reference whose 3D is already loaded.
/// # Errors
/// A stop in the actor arm or in a script.
pub fn finish_attach_reference_loop(
    world: &mut World,
    cell: Entity,
    priority: i32,
    queue: &mut QueueReferenceCall<'_>,
) -> Result<(), GameError> {
    let references: Vec<Entity> = world
        .get::<TesObjectCell>(cell)
        .expect("a cell")
        .objects_48
        .items()
        .collect();
    for reference in references {
        let loading = crate::tes::forms_references::save_load_active(world)?;
        if !loading {
            // 004D5C7A 004F9EC0; 004D5C86 004FBF90(reference, +0x44, 0x1000).
            crate::tes::forms_references::run_pending_script(world, reference)?;
            crate::tes::ai_package::script_event_or(world, reference, 0x1000, None);
        }
        let flags = world
            .get::<FormIdentity>(reference)
            .expect("a form")
            .flags_08;
        if flags & 0x800 != 0 || flags & 0x20 != 0 {
            continue;
        }
        // 004D5CAA vt+0x190 IsActor; 004D5CBA a save loading needs +0x18 bit 4.
        if world.get::<Actor>(reference).is_some() {
            if loading {
                return Err(GameError::unclassified(
                    0x004D5CC9,
                    "004D5CC9: the actor arm while a save loads",
                ));
            }
            // 004D5CF7 vt+0x1A4; 004D5D09 00674E10.
            actor_rebuild_high_process(world, reference)?;
            process_manager_drop_queued(world, reference);
        }
        queue(world, reference, priority);
        if world
            .get::<TesObjectRefr>(reference)
            .expect("a reference")
            .loaded_3d_3c
            != 0
        {
            return Err(GameError::unclassified(
                0x004D5D32,
                "004D5D32: 004CEAD0 on a loaded reference",
            ));
        }
    }
    Ok(())
}

/// IDA: N/A a 0047DF80 draw made just before 00628EE0 runs, handed to it: nothing between them draws, and the first-use flag is already clear by the time a game runs.
pub(crate) struct DrawnRandom(pub(crate) Option<i32>);

impl crate::tes::actor_process::RandomSource for DrawnRandom {
    /// IDA: N/A `DrawnRandom`'s side of `RandomSource`.
    fn time64(&mut self) -> i64 {
        0
    }
    /// IDA: N/A `DrawnRandom`'s side of `RandomSource`.
    fn srand(&mut self, _seed: u32) {}
    /// IDA: N/A `DrawnRandom`'s side of `RandomSource`.
    fn rand(&mut self) -> i32 {
        self.0.take().expect("one draw for 00628EE0")
    }
}

/// IDA: N/A each 0069AF30 cast, in order (the port makes them through `magic_cast::cast_on_self`): the caster's actor and the spell. 005EDA20 reaches it through the caster's vt+0 (005E51B0 -> process vt+0x2A0 0064AE50 -> 0069BE00, an ability, type 4) and vt+4 (005E51E0 -> vt+0x2A4 0064AE70 -> 0069BE40, a disease, type 1, with the actor's +0x68 target), each after the caster's vt+0x34 on the spell's +0x18 part.
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct CastBoundaries {
    pub casts: Vec<(Entity, Entity)>,
}

/// IDA: the walk 005EDA20(0) and 005E4DD0 share for a non-player: the base's spell list (+0x58) when vt+0x190 says an actor, then (a Character's birth sign, 006F7070, is null) an NPC's race's (+0xE8, its +0x30), in list order, keeping each spell whose type (the +0x18 part's vt+0x18: +0x34) is 4 (an ability) or 1 (a disease). Completion: 100% of a non-player.
#[must_use]
pub(crate) fn base_and_race_abilities(world: &World, actor: Entity) -> Vec<Entity> {
    use crate::tes::form_components::{TesRaceForm, TesSpellList};
    let base = world
        .get::<TesObjectRefr>(actor)
        .expect("an actor")
        .base_form_1c;
    let spells_of = |world: &World, form: Option<Entity>| -> Vec<Entity> {
        form.and_then(|form| world.get::<TesSpellList>(form))
            .map(|list| {
                list.spells_04
                    .iter()
                    .filter_map(|spell| match spell {
                        FormRef::Resolved(spell) => *spell,
                        FormRef::Raw(_) => None,
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    let mut spells = spells_of(world, base);
    if base.is_some_and(|base| crate::tes::forms_init::form_type(world, base) == 0x23) {
        let race = base
            .and_then(|base| world.get::<TesRaceForm>(base))
            .and_then(|form| match form.race_04 {
                FormRef::Resolved(race) => race,
                FormRef::Raw(_) => None,
            });
        if race.is_some() {
            spells.extend(spells_of(world, race));
        }
    }
    spells.retain(|&spell| {
        let spell_type = world
            .get::<crate::tes::forms_magic::MagicFormData>(spell)
            .map_or(u32::MAX, |data| data.type_34);
        spell_type == 4 || spell_type == 1
    });
    spells
}
