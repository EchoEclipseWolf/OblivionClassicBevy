//! A raw NiObject pointer stored without a reference by a LinkObject (13e).

use bevy_ecs::prelude::*;
use recomp_core::reader::StreamError;

use super::stream_objects::{NULL_REFERENCE, StreamObjectTable};

/// IDA: a raw pointer stored without a reference by NiStream_ResolveNextDeferredReference through 00753030
/// (NiPSysVolumeEmitter +0x50) and 0075E870 (the drag / gravity modifier +0x18). The C++ weak handle and identity
/// are one `Entity` here. An index of 0xFFFFFFFF stores null. Completion: 100% (no type check, as in the original).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AvObjectRawRef(pub Option<Entity>);

/// IDA: NiStream_ResolveNextDeferredReference as a raw pointer (00753030, 0075E870): any table object, no reference
/// taken; 0xFFFFFFFF is null. Completion: 100%.
pub fn raw_object_reference(
    table: &StreamObjectTable,
    index: u32,
) -> Result<AvObjectRawRef, StreamError> {
    if index == NULL_REFERENCE {
        return Ok(AvObjectRawRef(None));
    }
    let entry = table
        .objects
        .get(index as usize)
        .ok_or(StreamError("particle dependency link out of range"))?;
    Ok(AvObjectRawRef(Some(entry.entity)))
}
