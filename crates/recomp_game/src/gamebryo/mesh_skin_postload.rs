//! NiSkinInstance::PostLoad: the legacy transfer of NiSkinData's partition.

use bevy_ecs::prelude::*;
use recomp_core::reader::StreamError;

use super::mesh_skin_owners::MeshSkinInstance;

/// IDA: 0072F210 NiSkinData +0x08, the legacy NiSkinPartition pointer: a companion slot, because the modern `MeshSkinData`
/// stores no legacy partition. Completion: 100% of the slot.
/// TODO: [ ] Fold this into a verified legacy NiSkinData LoadBinary owner.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MeshSkinLegacyPartitionSlot {
    pub data_identity: Option<Entity>,
    pub partition: Option<Entity>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MeshSkinPostLoadStatus {
    Skipped,
    Applied,
    UnsupportedOwner,
}

/// IDA: 0072BE70 __thiscall NiSkinInstance::PostLoad(NiStream*). At >= 0A010065 or with a null +0x08 data it returns
/// unchanged. Otherwise it assigns data +0x08 into instance +0x0C through 0075FA90 NiRefObjectPtr_Assign (equal pointers
/// skip the store; otherwise the old slot releases first), then releases and clears data +0x08 when nonnull. Completion:
/// 100% of the branch and pointer-store order; entities have no refcount or destructor timing.
/// TODO: [ ] The exact NiRefObjectPtr refcounts and the pre-modern data loader.
pub fn postload_mesh_skin_instance(
    instance: &mut MeshSkinInstance,
    stream_version: u32,
    legacy_slot: Option<&mut MeshSkinLegacyPartitionSlot>,
) -> Result<MeshSkinPostLoadStatus, StreamError> {
    if stream_version >= 0x0A01_0065 || instance.data_08.is_none() {
        return Ok(MeshSkinPostLoadStatus::Skipped);
    }
    let Some(legacy_slot) = legacy_slot else {
        return Ok(MeshSkinPostLoadStatus::UnsupportedOwner);
    };
    if legacy_slot.data_identity != instance.data_08 {
        return Err(StreamError("NiSkinInstance legacy partition data mismatch"));
    }
    if instance.partition_0c != legacy_slot.partition {
        instance.partition_0c = legacy_slot.partition;
    }
    legacy_slot.partition = None;
    Ok(MeshSkinPostLoadStatus::Applied)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEGACY: u32 = 0x0A01_0064;
    const MODERN: u32 = 0x0A01_0065;

    /// Ported from `gamebryo_mesh_skin_postload` `verify_version_and_null_data_gates`.
    #[test]
    fn version_and_null_data_gates() {
        let mut world = World::new();
        let (data, partition) = (world.spawn_empty().id(), world.spawn_empty().id());
        let mut instance = MeshSkinInstance::default();
        assert_eq!(
            postload_mesh_skin_instance(&mut instance, LEGACY, None).unwrap(),
            MeshSkinPostLoadStatus::Skipped,
            "legacy SkinInstance with null data did not skip"
        );
        instance.data_08 = Some(data);
        let mut slot = MeshSkinLegacyPartitionSlot {
            data_identity: Some(data),
            partition: Some(partition),
        };
        assert_eq!(
            postload_mesh_skin_instance(&mut instance, MODERN, Some(&mut slot)).unwrap(),
            MeshSkinPostLoadStatus::Skipped
        );
        assert_eq!(instance.partition_0c, None);
        assert_eq!(
            slot.partition,
            Some(partition),
            "modern left the slot alone"
        );
        assert_eq!(
            postload_mesh_skin_instance(&mut instance, LEGACY, None).unwrap(),
            MeshSkinPostLoadStatus::UnsupportedOwner
        );
        assert_eq!(instance.partition_0c, None);
    }

    /// Ported from `gamebryo_mesh_skin_postload` `verify_legacy_transfer_equal_and_null`. `old.expired()` tests a
    /// `shared_ptr` release and is dropped; the store it follows stays checked.
    #[test]
    fn legacy_transfer_equal_and_null() {
        let mut world = World::new();
        let (data, partition) = (world.spawn_empty().id(), world.spawn_empty().id());
        let mut instance = MeshSkinInstance {
            data_08: Some(data),
            ..MeshSkinInstance::default()
        };
        let mut slot = MeshSkinLegacyPartitionSlot {
            data_identity: Some(data),
            partition: Some(partition),
        };
        assert_eq!(
            postload_mesh_skin_instance(&mut instance, LEGACY, Some(&mut slot)).unwrap(),
            MeshSkinPostLoadStatus::Applied
        );
        assert_eq!(instance.partition_0c, Some(partition));
        assert_eq!(slot.partition, None);

        slot.partition = Some(partition);
        assert_eq!(
            postload_mesh_skin_instance(&mut instance, LEGACY, Some(&mut slot)).unwrap(),
            MeshSkinPostLoadStatus::Applied
        );
        assert_eq!(instance.partition_0c, Some(partition));
        assert_eq!(
            slot.partition, None,
            "the equal pointer kept while the slot cleared"
        );

        assert_eq!(
            postload_mesh_skin_instance(&mut instance, LEGACY, Some(&mut slot)).unwrap(),
            MeshSkinPostLoadStatus::Applied
        );
        assert_eq!(
            instance.partition_0c, None,
            "a null partition clears the owner"
        );

        slot.data_identity = Some(world.spawn_empty().id());
        assert!(postload_mesh_skin_instance(&mut instance, LEGACY, Some(&mut slot)).is_err());
        assert_eq!(instance.partition_0c, None);
    }
}
