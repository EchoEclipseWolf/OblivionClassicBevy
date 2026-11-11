//! Phase 14c: the cell moves of the AI pass, outside a load: 004DD4B0 TESObjectREFR_MoveToCell with 004D35D0's attached arms (its head and 004CECD0 are `forms_world`'s) and the worldspace's persistent cell, and the door callbacks 00645A30 and 00645AF0 with their ownership and lock tests.
//!
//! Evidence: objdump of each address named here; P14-RUN-004.

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;

use crate::tes::actor::TesObjectRefr;
use crate::tes::base_extra_list::ExtraDataList;
use crate::tes::forms_world::{
    CellInsertRest, TesObjectCell, cell_insert_reference_head, insert_reference_attached_arms,
};
use crate::tes::player::PlayerCharacter;

// IDA: vt+0x150(0): Character 0060E430, Creature 00625020, with no 3D: 004E0F80(null): 00439DC0 drops the reference's queued tasks; the host queues none (recorded substitution, while 00438060 stays counted). The player's 0065F950 is not ported.
pub(crate) fn set_3d_null(world: &World, actor: Entity) -> Result<(), GameError> {
    if world
        .get::<TesObjectRefr>(actor)
        .expect("an actor")
        .loaded_3d_3c
        != 0
    {
        return Err(GameError::unclassified(
            0x004E0FCD,
            "004E0FCD: Set3D(null) with loaded 3D",
        ));
    }
    if world.get::<PlayerCharacter>(actor).is_some() {
        return Err(GameError::unclassified(
            0x0065F950,
            "0065F950: the player's Set3D",
        ));
    }
    Ok(())
}

// IDA: N/A 004D35D0 TESObjectCELL_InsertPlayerAndRefresh's two parts in order, with no services lent for 004D3882's 00438060 (a counted boundary: startup's calls).
pub fn insert_reference_runtime(
    world: &mut World,
    cell: Entity,
    reference: Entity,
) -> Result<(), GameError> {
    insert_reference_queued(world, cell, reference, None)
}

// IDA: N/A 004D35D0's two parts in order: its head (`forms_world::cell_insert_reference_head`, 004D35D0..004D374D), then, when no jump of 004D3731..004D374D is taken, the attached arms below, with the reference load's services for 004D3882's 00438060.
/// # Errors
/// As the head, the arms and 00438060.
pub fn insert_reference_queued(
    world: &mut World,
    cell: Entity,
    reference: Entity,
    queue: Option<&mut crate::tes::reference_load::ReferenceLoadServices<'_, '_>>,
) -> Result<(), GameError> {
    match cell_insert_reference_head(world, cell, reference)? {
        CellInsertRest::Done => Ok(()),
        CellInsertRest::AttachedArms => {
            insert_reference_attached_arms(world, cell, reference, queue)
        }
    }
}

// ---- 00645A30, 00645AF0 and what they read ----

// IDA: N/A a helper, with no address of its own.
pub(crate) fn list_of(world: &World, reference: Entity) -> &ExtraDataList {
    &world
        .get::<TesObjectRefr>(reference)
        .expect("a reference")
        .extra_data_44
}

// IDA: N/A a helper, with no address of its own.
pub(crate) fn cell_list(world: &World, cell: Entity) -> &ExtraDataList {
    &world.get::<TesObjectCell>(cell).expect("a cell").extra_28
}
