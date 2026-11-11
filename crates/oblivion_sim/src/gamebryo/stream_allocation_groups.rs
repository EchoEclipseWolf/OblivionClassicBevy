//! NiStream's geometry allocation groups: the blocks that NiGeometryData arrays claim while loading.

use oblivion_core::error::GameError;
use oblivion_core::reader::ByteReader;

/// IDA: 007329D0 record: +0 size, +4 backing, +8 cursor, +C allocation count. Host ownership replaces FormHeap; the
/// cursor is a byte offset into `backing`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamAllocationGroup {
    size_00: u32,
    used_08: u32,
    allocation_count_0c: u32,
    backing_04: Vec<u8>,
}

/// IDA: N/A where a claimed array lies in its group's backing, in place of the original array pointer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StreamGroupArray {
    pub offset: u32,
    pub size: u32,
}

impl StreamAllocationGroup {
    /// IDA: 007329D0 allocates only for a nonzero size and clears the count. Backing bytes are zero here, where the
    /// original's are uninitialized. Completion: 100% for a successful allocation.
    /// TODO: [ ] Match the original's nullable allocation failure.
    #[must_use]
    pub fn new(size: u32) -> Self {
        Self {
            size_00: size,
            used_08: 0,
            allocation_count_0c: 0,
            backing_04: vec![0; size as usize],
        }
    }

    /// IDA: 007295AB/AE/B5 and the normal, color and UV paths advance the byte cursor and increment +C.
    /// Completion: 100% of bounded successful claims, including width zero.
    /// TODO: [ ] Compare the original's unchecked overflow and pointer identity.
    pub fn allocate(&mut self, byte_count: u32) -> Result<StreamGroupArray, GameError> {
        if byte_count > self.size_00 - self.used_08 {
            return Err(GameError::unclassified(
                0,
                "stream allocation group exhausted",
            ));
        }
        let offset = self.used_08;
        self.allocation_count_0c += 1;
        self.used_08 += byte_count;
        Ok(StreamGroupArray {
            offset,
            size: byte_count,
        })
    }

    /// IDA: N/A the claimed bytes of one array, mutable for the copy that follows a claim.
    pub fn bytes_mut(&mut self, array: StreamGroupArray) -> &mut [u8] {
        let start = array.offset as usize;
        &mut self.backing_04[start..start + array.size as usize]
    }

    /// IDA: N/A the claimed bytes of one array.
    #[must_use]
    pub fn bytes(&self, array: StreamGroupArray) -> &[u8] {
        let start = array.offset as usize;
        &self.backing_04[start..start + array.size as usize]
    }

    /// IDA: 007329D9 the stored +0 size. Completion: 100%.
    #[must_use]
    pub const fn size(&self) -> u32 {
        self.size_00
    }

    /// IDA: 00729450 advances +8 by each exact array width. Completion: 100%.
    #[must_use]
    pub const fn used_bytes(&self) -> u32 {
        self.used_08
    }

    /// IDA: 00729450 increments +C once per allocation. Completion: 100%.
    /// TODO: [ ] Recover the release and reference-count consumers.
    #[must_use]
    pub const fn allocation_count(&self) -> u32 {
        self.allocation_count_0c
    }
}

/// IDA: 0071309F/A7 inserts a null slot zero before the nonnull sized groups.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StreamAllocationGroups {
    pub groups: Vec<Option<StreamAllocationGroup>>,
    pub bytes_consumed: usize,
}

impl StreamAllocationGroups {
    /// IDA: 00712550 returns array[index], unchecked. Completion: 100% of the checked lookup.
    /// TODO: [ ] Compare the original's invalid-index behaviour.
    pub fn resolve(&self, index: u32) -> Result<Option<&StreamAllocationGroup>, GameError> {
        self.groups
            .get(index as usize)
            .map(Option::as_ref)
            .ok_or(GameError::unclassified(
                0,
                "stream allocation group index out of range",
            ))
    }

    /// IDA: 00712550, for a claim. Completion: 100% of the checked lookup.
    pub fn resolve_mut(
        &mut self,
        index: u32,
    ) -> Result<Option<&mut StreamAllocationGroup>, GameError> {
        self.groups
            .get_mut(index as usize)
            .map(Option::as_mut)
            .ok_or(GameError::unclassified(
                0,
                "stream allocation group index out of range",
            ))
    }
}

const TRUNCATED_GROUPS: &str = "truncated stream allocation groups";

/// IDA: 00713030, called from 007143E7/EB at version >= 05000006, after the factories and before the object payloads:
/// the count, the null slot, then count u32 sizes. Completion: 100% of the header and successful construction.
/// TODO: [ ] Match the NiTArray u16 capacity overflow and the original's stream errors.
pub fn read_stream_allocation_groups(
    bytes: &[u8],
    stream_version: u32,
) -> Result<StreamAllocationGroups, GameError> {
    let mut result = StreamAllocationGroups::default();
    if stream_version < 0x0500_0006 {
        return Ok(result);
    }
    let mut reader = ByteReader::new(bytes);
    let count = reader.u32(TRUNCATED_GROUPS)?;
    if count == u32::MAX {
        return Err(GameError::unclassified(
            0,
            "stream allocation group count overflow",
        ));
    }
    if count as usize > reader.remaining() / 4 {
        return Err(GameError::stream("truncated stream allocation group sizes"));
    }
    result.groups.reserve(count as usize + 1);
    result.groups.push(None);
    for _ in 0..count {
        result.groups.push(Some(StreamAllocationGroup::new(
            reader.u32(TRUNCATED_GROUPS)?,
        )));
    }
    result.bytes_consumed = reader.position();
    Ok(result)
}
