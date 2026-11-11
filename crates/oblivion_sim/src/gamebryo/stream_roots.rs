//! NiStream's root list.

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;
use oblivion_core::reader::ByteReader;

use super::stream_objects::{NULL_REFERENCE, StreamObjectTable};

/// IDA: 00712F10's root array: raw indices and the object in each slot, null slots kept in order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StreamRoots {
    pub original_indices: Vec<u32>,
    pub owners: Vec<Option<Entity>>,
    pub bytes_consumed: usize,
}

/// IDA: 00712F10 sets the root array capacity, then reads and retains each object in order. Completion: 100% of the
/// successful modern root, null and identity order.
/// TODO: [ ] The original's unchecked invalid index and allocator failure effects.
pub fn read_stream_roots(
    bytes: &[u8],
    table: &StreamObjectTable,
) -> Result<StreamRoots, GameError> {
    let mut reader = ByteReader::new(bytes);
    let count = reader.u32("truncated stream root records")? as usize;
    if count > reader.remaining() / 4 {
        return Err(GameError::stream("truncated stream root index array"));
    }
    let mut roots = StreamRoots {
        original_indices: Vec::with_capacity(count),
        owners: Vec::with_capacity(count),
        bytes_consumed: 0,
    };
    for _ in 0..count {
        let index = reader.u32("truncated stream root records")?;
        roots.original_indices.push(index);
        if index == NULL_REFERENCE {
            roots.owners.push(None);
        } else {
            let entry = table
                .objects
                .get(index as usize)
                .ok_or(GameError::unclassified(0, "stream root index out of range"))?;
            roots.owners.push(Some(entry.entity));
        }
    }
    roots.bytes_consumed = reader.position();
    Ok(roots)
}
