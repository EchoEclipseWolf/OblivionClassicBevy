//! NiPSysData: the particle arrays that the modifiers update.

use bevy_ecs::prelude::*;
use recomp_core::reader::{ByteReader, StreamError};

use super::stream_allocation_groups::{
    StreamAllocationGroup, StreamAllocationGroups, StreamGroupArray,
};

/// IDA: the 0x1C-byte record observed in 007595xx / 00755Fxx. Host storage; no ABI overlay.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ParticleInfo {
    pub velocity_00: [f32; 3],
    pub age_0c: f32,
    pub lifespan_10: f32,
    pub last_update_time_14: f32,
    pub generation_18: u16,
    pub unresolved_flags_1a: u16,
}

/// IDA: 00729450 NiGeometryData::LoadBinary and 00759C50 NiPSysData. Host-owned values, not an overlay of the original
/// 0x68-byte object. An empty optional array stands for an original null pointer.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct ParticleData {
    pub allocation_group_index: u32,
    /// IDA: 00729450 backing ownership and claim order. The group's bytes stay in the stream's table, named by this
    /// index; the vectors below stay the live simulation storage until the original array aliases are reconstructed.
    pub allocation_group: Option<u32>,
    pub geometry_group_arrays: [StreamGroupArray; 4],
    pub geometry_positions_present: bool,
    pub geometry_normals_present: bool,
    pub geometry_colors_present: bool,
    pub vertex_count: u16,
    pub geometry_byte_30: u8,
    pub geometry_byte_31: u8,
    pub geometry_flags: u16,
    /// IDA: 00728770 clears +0x2E, then 0073EE80 sets bit 15.
    pub geometry_extra_flags_2e: u16,
    pub bounds: [f32; 4],
    pub texture_coordinates: Vec<[f32; 2]>,
    pub deferred_geometry_link: Option<u32>,
    pub positions: Vec<[f32; 3]>,
    pub info_5c: Vec<ParticleInfo>,
    pub colors: Vec<[f32; 4]>,
    pub normals: Vec<[f32; 3]>,
    pub radii: Vec<f32>,
    pub sizes_4c: Vec<f32>,
    pub rotations: Vec<[f32; 4]>,
    pub rotation_angles_54: Vec<f32>,
    pub rotation_axes: Vec<[f32; 3]>,
    pub rotation_speeds_60: Vec<f32>,
    pub active_count_48: u16,
    pub added_count: u16,
    pub added_base: u16,
}

impl Default for ParticleData {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            allocation_group_index: 0,
            allocation_group: None,
            geometry_group_arrays: [StreamGroupArray::default(); 4],
            geometry_positions_present: false,
            geometry_normals_present: false,
            geometry_colors_present: false,
            vertex_count: 0,
            geometry_byte_30: 0,
            geometry_byte_31: 0,
            geometry_flags: 0,
            geometry_extra_flags_2e: 0x8000,
            bounds: [0.0; 4],
            texture_coordinates: Vec::new(),
            deferred_geometry_link: None,
            positions: Vec::new(),
            info_5c: Vec::new(),
            colors: Vec::new(),
            normals: Vec::new(),
            radii: Vec::new(),
            sizes_4c: Vec::new(),
            rotations: Vec::new(),
            rotation_angles_54: Vec::new(),
            rotation_axes: Vec::new(),
            rotation_speeds_60: Vec::new(),
            active_count_48: 0,
            added_count: 0,
            added_base: 0,
        }
    }
}

/// IDA: N/A host preflight for the optional arrays in 0073EFB0 / 00759860. Completion: 100% of the extent checks; the
/// original's unchecked pointers are not reproduced.
/// TODO: [ ] Match the original allocation capacity through the particle-data loader.
fn check_optional<T>(values: &[T], required: usize) -> Result<(), StreamError> {
    if !values.is_empty() && values.len() < required {
        return Err(StreamError("particle optional array is too short"));
    }
    Ok(())
}

/// IDA: 0073EFB0 / 00759860 copy complete stored elements. Completion: 100%.
/// TODO: [ ] Verify the original derived arrays before extending this field set.
fn copy_slot<T: Copy>(values: &mut [T], target: usize, source: usize) {
    if !values.is_empty() {
        values[target] = values[source];
    }
}

impl ParticleData {
    /// IDA: N/A host preflight for 00759860 / 007598C0. Completion: 100% of the supported arrays.
    /// TODO: [ ] Validate the capacities of derived particle-data arrays when implemented.
    fn check_arrays(&self, required: usize) -> Result<(), StreamError> {
        if self.positions.len() < required || self.info_5c.len() < required {
            return Err(StreamError("particle position/info array is too short"));
        }
        check_optional(&self.colors, required)?;
        check_optional(&self.normals, required)?;
        check_optional(&self.radii, required)?;
        check_optional(&self.sizes_4c, required)?;
        check_optional(&self.rotations, required)?;
        check_optional(&self.rotation_angles_54, required)?;
        check_optional(&self.rotation_axes, required)?;
        check_optional(&self.rotation_speeds_60, required)
    }

    /// IDA: 00759860 -> 0073EFB0 swaps the last active record in, then decrements the count. Completion: 100% of the
    /// base arrays after the caller's preflight; no container resizes.
    /// TODO: [ ] Dispatch derived NiPSysData removal effects when supported.
    fn remove_unchecked(&mut self, index: u16) {
        let last = self.active_count_48.wrapping_sub(1);
        if index != last {
            let (target, source) = (usize::from(index), usize::from(last));
            copy_slot(&mut self.positions, target, source);
            copy_slot(&mut self.colors, target, source);
            copy_slot(&mut self.normals, target, source);
            copy_slot(&mut self.radii, target, source);
            copy_slot(&mut self.sizes_4c, target, source);
            copy_slot(&mut self.rotations, target, source);
            copy_slot(&mut self.rotation_angles_54, target, source);
            copy_slot(&mut self.rotation_axes, target, source);
            copy_slot(&mut self.info_5c, target, source);
            copy_slot(&mut self.rotation_speeds_60, target, source);
        }
        self.active_count_48 = self.active_count_48.wrapping_sub(1);
    }

    /// IDA: 00759860 / 0073EFB0 Remove(index). Completion: 100% of the bounded base swap-last removal.
    /// TODO: [ ] Bind subclass overrides, storage and renderer ownership, and validate runtime array alignment.
    pub fn remove(&mut self, index: u16) -> Result<(), StreamError> {
        if index >= self.active_count_48 {
            return Err(StreamError("particle removal index out of range"));
        }
        self.check_arrays(usize::from(self.active_count_48))?;
        self.remove_unchecked(index);
        Ok(())
    }

    /// IDA: 007598C0 NiPSysData virtual +5C. Completion: 100% of the checked count, append and gap compaction for the
    /// base class.
    /// TODO: [ ] Integrate emitter allocation, virtual +58's derived removals and runtime checks.
    pub fn finalize_added_particles(&mut self) -> Result<(), StreamError> {
        let base = if self.active_count_48 >= self.added_base {
            self.active_count_48
        } else {
            self.added_base
        };
        let total = u32::from(base) + u32::from(self.added_count);
        if total > u32::from(u16::MAX) {
            return Err(StreamError("particle active count overflow"));
        }
        self.check_arrays(total as usize)?;
        if self.active_count_48 >= self.added_base {
            self.active_count_48 = total as u16;
        } else {
            let mut hole = self.active_count_48;
            self.active_count_48 = total as u16;
            while hole < self.added_base && self.active_count_48 > self.added_base {
                self.remove_unchecked(hole);
                hole += 1;
            }
            if hole < self.added_base {
                self.active_count_48 = hole;
            }
        }
        self.added_count = 0;
        self.added_base = self.active_count_48;
        Ok(())
    }

    /// IDA: 0074ED40 returns FFFF when the fixed vertex capacity cannot hold another added particle; +0x64/+0x66 track
    /// the pending run. 0074EE70 inlines the same counters. Completion: 100% of the bounded counters.
    /// TODO: [ ] Compare malformed counter overflow and the original's fixed heap layout.
    pub fn reserve_added_slot(&mut self) -> u16 {
        if self.added_count != 0 {
            let slot = u32::from(self.added_base) + u32::from(self.added_count);
            if slot >= u32::from(self.vertex_count) {
                return 0xFFFF;
            }
            self.added_count += 1;
            return slot as u16;
        }
        self.added_base = self.active_count_48;
        if self.active_count_48 >= self.vertex_count {
            return 0xFFFF;
        }
        self.added_count = 1;
        self.active_count_48
    }
}

/// The parsed payload and what it consumed.
#[derive(Clone, Debug, PartialEq)]
pub struct ParticleDataParseResult {
    pub data: ParticleData,
    pub bytes_consumed: usize,
}

const TRUNCATED: &str = "truncated NiPSysData payload";

// IDA: N/A a helper, with no address of its own.
fn f32_array<const N: usize>(reader: &mut ByteReader<'_>) -> Result<[f32; N], StreamError> {
    let mut result = [0.0; N];
    for value in &mut result {
        *value = reader.f32(TRUNCATED)?;
    }
    Ok(result)
}

/// IDA: 00729450 claims exact bytes before the callback reads. Completion: 100% of the successful cursor, count and
/// raw-copy order, including width zero.
/// TODO: [ ] Replace the simulation vectors with the original backing aliases.
fn f32_vectors<const N: usize>(
    reader: &mut ByteReader<'_>,
    count: usize,
    group: Option<&mut StreamAllocationGroup>,
    group_array: Option<&mut StreamGroupArray>,
) -> Result<Vec<[f32; N]>, StreamError> {
    let width = count * N * 4;
    if width > reader.remaining() {
        return Err(StreamError(TRUNCATED));
    }
    if let Some(group) = group {
        let storage = group.allocate(width as u32)?;
        let mut peek = *reader;
        group
            .bytes_mut(storage)
            .copy_from_slice(peek.take(width, TRUNCATED)?);
        if let Some(group_array) = group_array {
            *group_array = storage;
        }
    }
    let mut result = Vec::with_capacity(count);
    for _ in 0..count {
        result.push(f32_array::<N>(reader)?);
    }
    Ok(result)
}

// IDA: N/A a helper, with no address of its own.
fn f32_values(reader: &mut ByteReader<'_>, count: usize) -> Result<Vec<f32>, StreamError> {
    if count * 4 > reader.remaining() {
        return Err(StreamError(TRUNCATED));
    }
    let mut result = Vec::with_capacity(count);
    for _ in 0..count {
        result.push(reader.f32(TRUNCATED)?);
    }
    Ok(result)
}

/// IDA: N/A the resolved group for one more claim; the index was checked when the payload named it.
fn claim_group<'g>(
    groups: &'g mut Option<&mut StreamAllocationGroups>,
    index: u32,
) -> Option<&'g mut StreamAllocationGroup> {
    groups
        .as_deref_mut()
        .and_then(|groups| groups.resolve_mut(index).ok().flatten())
}

/// IDA: 00729450 geometry payload -> 0073F460 particle arrays -> 00759DE0 NiPSysData information, with 00716EA0 bounds
/// and 0075F840 particle info. Completion: 100% of the modern >= 0A030005 byte order for one fresh owner; a failure
/// publishes no object, though group claims already made stay made, as in C++.
/// TODO: [ ] Resolve the geometry's deferred reference and allocation-group index in NiStream.
/// TODO: [ ] 00715000 for a legacy quaternion array, and the allocation failures.
pub fn parse_particle_data_payload(
    bytes: &[u8],
    stream_version: u32,
    mut groups: Option<&mut StreamAllocationGroups>,
) -> Result<ParticleDataParseResult, StreamError> {
    if stream_version < 0x0A01_0072 {
        return Err(StreamError("NiPSysData stream version unsupported"));
    }
    let mut reader = ByteReader::new(bytes);
    // 00729450: modern NiObject_LoadBinary is empty; the group index precedes the 16-bit vertex capacity and the
    // two format bytes.
    let group_index = reader.u32(TRUNCATED)?;
    let mut data = ParticleData {
        allocation_group_index: group_index,
        ..ParticleData::default()
    };
    if let Some(groups) = groups.as_deref() {
        data.allocation_group = groups.resolve(group_index)?.map(|_| group_index);
    }
    data.vertex_count = reader.u16(TRUNCATED)?;
    let count = usize::from(data.vertex_count);
    data.geometry_byte_30 = reader.u8(TRUNCATED)?;
    data.geometry_byte_31 = reader.u8(TRUNCATED)?;
    data.geometry_positions_present = reader.u8(TRUNCATED)? != 0;
    if data.geometry_positions_present {
        data.positions = f32_vectors::<3>(
            &mut reader,
            count,
            claim_group(&mut groups, group_index),
            Some(&mut data.geometry_group_arrays[0]),
        )?;
    }
    data.geometry_flags = reader.u16(TRUNCATED)?;
    let normal_count = count
        * if data.geometry_flags & 0xF000 != 0 {
            3
        } else {
            1
        };
    data.geometry_normals_present = reader.u8(TRUNCATED)? != 0;
    if data.geometry_normals_present {
        data.normals = f32_vectors::<3>(
            &mut reader,
            normal_count,
            claim_group(&mut groups, group_index),
            Some(&mut data.geometry_group_arrays[1]),
        )?;
    }
    // 00716EA0: sphere center (00709430) followed by radius.
    data.bounds = f32_array::<4>(&mut reader)?;
    data.geometry_colors_present = reader.u8(TRUNCATED)? != 0;
    if data.geometry_colors_present {
        data.colors = f32_vectors::<4>(
            &mut reader,
            count,
            claim_group(&mut groups, group_index),
            Some(&mut data.geometry_group_arrays[2]),
        )?;
    }
    // IDA: 007298B4 skips UV allocation entirely when the set count is zero.
    if data.geometry_flags & 0x3F != 0 {
        data.texture_coordinates = f32_vectors::<2>(
            &mut reader,
            count * usize::from(data.geometry_flags & 0x3F),
            claim_group(&mut groups, group_index),
            Some(&mut data.geometry_group_arrays[3]),
        )?;
    }
    data.geometry_extra_flags_2e = reader.u16(TRUNCATED)?;
    // 00729450 >= 0A030007 queues a geometry reference for later fix-up.
    if stream_version >= 0x0A03_0007 {
        data.deferred_geometry_link = Some(reader.u32(TRUNCATED)?);
    }

    // 0073F460: radii and sizes are allocated even when the stream supplies no array, in which case every element
    // defaults to 1.0.
    data.radii = vec![1.0; count];
    if reader.u8(TRUNCATED)? != 0 {
        data.radii = f32_values(&mut reader, count)?;
    }
    data.active_count_48 = reader.u16(TRUNCATED)?;
    data.sizes_4c = vec![1.0; count];
    if reader.u8(TRUNCATED)? != 0 {
        data.sizes_4c = f32_values(&mut reader, count)?;
    }
    if reader.u8(TRUNCATED)? != 0 {
        data.rotations = f32_vectors::<4>(&mut reader, count, None, None)?;
    }
    // 0073F460: the angle and axis arrays are 0A030005's. Below it, 00759DF9 turns each quaternion into them with 00715000.
    if stream_version >= 0x0A03_0005 {
        if reader.u8(TRUNCATED)? != 0 {
            data.rotation_angles_54 = f32_values(&mut reader, count)?;
        }
        if reader.u8(TRUNCATED)? != 0 {
            data.rotation_axes = f32_vectors::<3>(&mut reader, count, None, None)?;
        }
    } else if !data.rotations.is_empty() {
        return Err(StreamError(
            "00759DF9: a legacy NiPSysData quaternion array",
        ));
    }

    // 00759DE0 / 0075F840: each 28-byte ParticleInfo holds a vector, three floats, then two 16-bit words. The modern
    // branch has no discarded legacy vector in front of these fields.
    data.info_5c.reserve(count);
    for _ in 0..count {
        let velocity_00 = f32_array::<3>(&mut reader)?;
        // 0075F859: below 0A030005 a second vector, discarded.
        if stream_version < 0x0A03_0005 {
            f32_array::<3>(&mut reader)?;
        }
        data.info_5c.push(ParticleInfo {
            velocity_00,
            age_0c: reader.f32(TRUNCATED)?,
            lifespan_10: reader.f32(TRUNCATED)?,
            last_update_time_14: reader.f32(TRUNCATED)?,
            generation_18: reader.u16(TRUNCATED)?,
            unresolved_flags_1a: reader.u16(TRUNCATED)?,
        });
    }
    if stream_version < 0x1400_0002 {
        // 00759DE0 initializes legacy rotation speeds to zero only when rotation angles were allocated; the stream
        // has no flag here.
        if !data.rotation_angles_54.is_empty() {
            data.rotation_speeds_60 = vec![0.0; count];
        }
    } else if reader.u8(TRUNCATED)? != 0 {
        data.rotation_speeds_60 = f32_values(&mut reader, count)?;
    }
    data.added_count = reader.u16(TRUNCATED)?;
    data.added_base = reader.u16(TRUNCATED)?;
    Ok(ParticleDataParseResult {
        data,
        bytes_consumed: reader.position(),
    })
}

#[cfg(test)]
mod tests {
    use super::super::stream_allocation_groups::read_stream_allocation_groups;
    use super::*;

    fn fixture() -> ParticleData {
        let mut data = ParticleData::default();
        for i in 0..8u16 {
            let value = f32::from(i);
            data.positions.push([value, value + 10.0, value + 20.0]);
            data.info_5c.push(ParticleInfo {
                velocity_00: [value, 0.0, 0.0],
                age_0c: value,
                lifespan_10: 10.0,
                last_update_time_14: value,
                generation_18: i,
                unresolved_flags_1a: 0,
            });
            data.colors.push([value, 1.0, 2.0, 3.0]);
            data.normals.push([value, 1.0, 2.0]);
            data.radii.push(value);
            data.sizes_4c.push(value);
            data.rotations.push([value, 1.0, 2.0, 3.0]);
            data.rotation_angles_54.push(value);
            data.rotation_axes.push([value, 1.0, 2.0]);
            data.rotation_speeds_60.push(value);
        }
        data
    }

    fn u16le(bytes: &mut Vec<u8>, value: u16) {
        bytes.extend(value.to_le_bytes());
    }

    fn u32le(bytes: &mut Vec<u8>, value: u32) {
        bytes.extend(value.to_le_bytes());
    }

    fn floats(bytes: &mut Vec<u8>, values: &[f32]) {
        for value in values {
            bytes.extend(value.to_bits().to_le_bytes());
        }
    }

    struct PayloadFixture {
        bytes: Vec<u8>,
        active_offset: usize,
        added_offset: usize,
    }

    fn make_payload(legacy: bool) -> PayloadFixture {
        let mut bytes = Vec::new();
        u32le(&mut bytes, 7); // allocation group
        u16le(&mut bytes, 2); // vertex capacity
        bytes.push(0x12);
        bytes.push(0x34);
        bytes.push(1); // positions
        floats(&mut bytes, &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        u16le(&mut bytes, 1); // one UV set
        bytes.push(1); // normals
        floats(&mut bytes, &[7.0, 8.0, 9.0, 10.0, 11.0, 12.0]);
        floats(&mut bytes, &[13.0, 14.0, 15.0, 16.0]); // bounds
        bytes.push(1); // colors
        floats(
            &mut bytes,
            &[17.0, 18.0, 19.0, 20.0, 21.0, 22.0, 23.0, 24.0],
        );
        floats(&mut bytes, &[0.25, 0.5, 0.75, 1.0]); // two UV pairs
        u16le(&mut bytes, 0x55AA);
        if !legacy {
            u32le(&mut bytes, 42); // deferred geometry link
        }
        bytes.push(0); // absent radii: default 1
        let active_offset = bytes.len();
        u16le(&mut bytes, 2);
        bytes.push(1); // sizes
        floats(&mut bytes, &[0.5, 1.5]);
        bytes.push(0); // rotations absent
        bytes.push(u8::from(legacy)); // legacy angles
        if legacy {
            floats(&mut bytes, &[0.25, 0.5]);
        }
        bytes.push(0); // axes absent
        for index in 0..2u16 {
            let offset = f32::from(index);
            floats(&mut bytes, &[1.0 + offset, 2.0 + offset, 3.0 + offset]);
            floats(&mut bytes, &[0.25 + offset, 4.0, 0.125]); // age, lifespan, last update
            u16le(&mut bytes, index + 3);
            u16le(&mut bytes, index + 9);
        }
        if !legacy {
            bytes.push(0); // modern rotation-speed flag
        }
        let added_offset = bytes.len();
        u16le(&mut bytes, 0);
        u16le(&mut bytes, 2);
        bytes.push(0xEE); // next record sentinel
        PayloadFixture {
            bytes,
            active_offset,
            added_offset,
        }
    }

    #[test]
    fn payload_shape_and_raw_counts() {
        const MODERN: u32 = 0x1400_0004;
        let source = make_payload(false);
        let parsed = parse_particle_data_payload(&source.bytes, MODERN, None).unwrap();
        let data = &parsed.data;
        assert_eq!(parsed.bytes_consumed + 1, source.bytes.len());
        assert_eq!(data.allocation_group_index, 7);
        assert_eq!(data.vertex_count, 2);
        assert_eq!(data.geometry_byte_30, 0x12);
        assert_eq!(data.geometry_byte_31, 0x34);
        assert_eq!(data.geometry_flags, 1);
        assert_eq!(data.geometry_extra_flags_2e, 0x55AA);
        assert_eq!(data.deferred_geometry_link, Some(42));
        assert_eq!(data.bounds[3], 16.0);
        assert_eq!(data.positions.len(), 2);
        assert_eq!(data.positions[1][2], 6.0);
        assert_eq!(data.normals[1][2], 12.0);
        assert_eq!(data.colors[1][3], 24.0);
        assert_eq!(data.texture_coordinates[1][1], 1.0);
        assert_eq!(data.radii, [1.0, 1.0]);
        assert_eq!(data.sizes_4c, [0.5, 1.5]);
        assert_eq!(data.active_count_48, 2);
        assert_eq!(data.added_count, 0);
        assert_eq!(data.added_base, 2);
        assert_eq!(data.info_5c.len(), 2);
        assert_eq!(data.info_5c[1].velocity_00[0], 2.0);
        assert_eq!(data.info_5c[1].age_0c, 1.25);
        assert_eq!(data.info_5c[1].generation_18, 4);
        assert_eq!(data.info_5c[1].unresolved_flags_1a, 10);

        let legacy = make_payload(true);
        let older = parse_particle_data_payload(&legacy.bytes, 0x0A03_0005, None).unwrap();
        assert_eq!(older.bytes_consumed + 1, legacy.bytes.len());
        assert_eq!(older.data.deferred_geometry_link, None);
        assert_eq!(older.data.rotation_angles_54, [0.25, 0.5]);
        assert_eq!(older.data.rotation_speeds_60, [0.0, 0.0]);

        let payload_size = source.bytes.len() - 1;
        for length in 0..payload_size {
            assert!(parse_particle_data_payload(&source.bytes[..length], MODERN, None).is_err());
        }
        assert!(parse_particle_data_payload(&source.bytes, 0x0A03_0004, None).is_err());
        let mut overactive = source.bytes.clone();
        overactive[source.active_offset] = 3;
        let raw_active = parse_particle_data_payload(&overactive, MODERN, None).unwrap();
        assert_eq!(raw_active.data.vertex_count, 2);
        assert_eq!(raw_active.data.active_count_48, 3);
        let mut overadded = source.bytes.clone();
        overadded[source.added_offset] = 1;
        let raw_added = parse_particle_data_payload(&overadded, MODERN, None).unwrap();
        assert_eq!(raw_added.data.added_count, 1);
        assert_eq!(raw_added.data.added_base, 2);
    }

    #[test]
    fn allocation_groups_take_exact_claims() {
        let mut header = Vec::new();
        u32le(&mut header, 7); // seven sized entries after the implicit null slot
        for _ in 1..7 {
            u32le(&mut header, 0);
        }
        u32le(&mut header, 192);
        header.push(0xAA);
        let mut groups = read_stream_allocation_groups(&header, 0x1400_0004).unwrap();
        assert_eq!(groups.bytes_consumed, 32);
        assert_eq!(groups.groups.len(), 8);
        assert!(groups.resolve(0).unwrap().is_none());
        let seven = groups.resolve(7).unwrap().unwrap();
        assert_eq!(seven.size(), 192);
        assert_eq!(seven.used_bytes(), 0);
        assert_eq!(seven.allocation_count(), 0);
        let skipped = read_stream_allocation_groups(&header, 0x0500_0005).unwrap();
        assert_eq!(skipped.bytes_consumed, 0);
        assert!(skipped.groups.is_empty());
        let mut zero = StreamAllocationGroup::new(0);
        assert_eq!(zero.allocate(0).unwrap().size, 0);
        assert_eq!(zero.used_bytes(), 0);
        assert_eq!(zero.allocation_count(), 1);
        assert!(zero.allocate(1).is_err());
        assert!(groups.resolve(8).is_err());

        let source = make_payload(false);
        let first =
            parse_particle_data_payload(&source.bytes, 0x1400_0004, Some(&mut groups)).unwrap();
        let second =
            parse_particle_data_payload(&source.bytes, 0x1400_0004, Some(&mut groups)).unwrap();
        assert_eq!(first.data.allocation_group, Some(7));
        assert_eq!(second.data.allocation_group, Some(7));
        let group = groups.resolve(7).unwrap().unwrap();
        assert_eq!(group.used_bytes(), 192);
        assert_eq!(group.allocation_count(), 8);
        for owner in [&first.data, &second.data] {
            assert!(owner.geometry_positions_present);
            assert!(owner.geometry_normals_present);
            assert!(owner.geometry_colors_present);
            assert_eq!(owner.geometry_group_arrays[0].size, 24);
            assert_eq!(owner.geometry_group_arrays[1].size, 24);
            assert_eq!(owner.geometry_group_arrays[2].size, 32);
            assert_eq!(owner.geometry_group_arrays[3].size, 16);
            assert_eq!(
                group.bytes(owner.geometry_group_arrays[0]),
                &source.bytes[9..33]
            );
        }
        assert_ne!(
            first.data.geometry_group_arrays[0].offset,
            second.data.geometry_group_arrays[0].offset
        );
        assert!(
            parse_particle_data_payload(&source.bytes, 0x1400_0004, Some(&mut groups)).is_err()
        );
        assert_eq!(groups.resolve(7).unwrap().unwrap().used_bytes(), 192);

        let mut short_header = Vec::new();
        u32le(&mut short_header, 1);
        u32le(&mut short_header, 95);
        let mut small = read_stream_allocation_groups(&short_header, 0x1400_0004).unwrap();
        let mut invalid_index = source.bytes.clone();
        invalid_index[0] = 1;
        assert!(
            parse_particle_data_payload(&invalid_index, 0x1400_0004, Some(&mut small)).is_err()
        );
        assert_eq!(small.resolve(1).unwrap().unwrap().used_bytes(), 80);
        assert!(parse_particle_data_payload(&source.bytes, 0x1400_0004, Some(&mut small)).is_err());
    }

    #[test]
    fn removal_and_compaction() {
        assert_eq!(ParticleData::default().geometry_extra_flags_2e, 0x8000);
        let mut data = fixture();
        data.active_count_48 = 6;
        data.remove(1).unwrap();
        assert_eq!(data.active_count_48, 5);
        assert_eq!(data.positions[1][0], 5.0);
        assert_eq!(data.info_5c[1].generation_18, 5);
        assert_eq!(data.colors[1][0], 5.0);
        assert_eq!(data.normals[1][0], 5.0);
        assert_eq!(data.radii[1], 5.0);
        assert_eq!(data.sizes_4c[1], 5.0);
        assert_eq!(data.rotations[1][0], 5.0);
        assert_eq!(data.rotation_angles_54[1], 5.0);
        assert_eq!(data.rotation_axes[1][0], 5.0);
        assert_eq!(data.rotation_speeds_60[1], 5.0);
        data.remove(4).unwrap();
        assert_eq!(data.active_count_48, 4);
        assert_eq!(data.positions.len(), 8);

        let mut data = fixture();
        data.active_count_48 = 2;
        data.added_base = 5;
        data.added_count = 2;
        data.finalize_added_particles().unwrap();
        assert_eq!(data.active_count_48, 4);
        assert_eq!(data.added_base, 4);
        assert_eq!(data.added_count, 0);
        assert_eq!(data.positions[2][0], 6.0);
        assert_eq!(data.positions[3][0], 5.0);

        let mut data = fixture();
        data.active_count_48 = 3;
        data.added_base = 5;
        data.added_count = 3;
        data.finalize_added_particles().unwrap();
        assert_eq!(data.active_count_48, 6);
        assert_eq!(data.positions[3][0], 7.0);
        assert_eq!(data.positions[4][0], 6.0);
        assert_eq!(data.positions[5][0], 5.0);

        let mut data = fixture();
        data.active_count_48 = 2;
        data.added_base = 5;
        data.finalize_added_particles().unwrap();
        assert_eq!(data.active_count_48, 2);
        assert_eq!(data.added_base, 2);
        data.added_count = 2;
        data.finalize_added_particles().unwrap();
        assert_eq!(data.active_count_48, 4);
        assert_eq!(data.added_base, 4);

        data.colors.clear();
        data.remove(0).unwrap();
        assert_eq!(data.active_count_48, 3);
        data.sizes_4c.truncate(1);
        assert!(data.remove(0).is_err());
        assert_eq!(data.active_count_48, 3);
        assert_eq!(data.positions[0][0], 3.0);

        let mut data = fixture();
        data.active_count_48 = 2;
        data.added_base = 65535;
        data.added_count = 1;
        assert!(data.finalize_added_particles().is_err());
        assert_eq!(data.active_count_48, 2);
        assert_eq!(data.added_base, 65535);
        assert_eq!(data.added_count, 1);
    }
}
