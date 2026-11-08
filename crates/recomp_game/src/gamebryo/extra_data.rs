//! The NiExtraData classes. Their name is `NiExtraDataName`, shared with NiObjectNET's link pass.

use bevy_ecs::prelude::*;
use recomp_core::reader::{ByteReader, StreamError};

use super::object_net::NiExtraDataName;

// IDA: N/A no address of its own; its comments cite 00713620.
fn read_string(reader: &mut ByteReader<'_>) -> Result<Option<Vec<u8>>, StreamError> {
    // IDA: 00713620: a nonpositive signed length is null.
    let length = reader.u32("truncated string length")? as i32;
    if length <= 0 {
        return Ok(None);
    }
    Ok(Some(
        reader
            .take(length as usize, "truncated string contents")?
            .to_vec(),
    ))
}

// IDA: N/A a helper, with no address of its own.
fn c_prefix(text: &[u8]) -> &[u8] {
    text.split(|&byte| byte == 0).next().unwrap_or(text)
}

/// IDA: 00713720: a 32-bit count and exactly the C-string prefix's bytes.
fn append_string(output: &mut Vec<u8>, value: Option<&[u8]>) {
    let prefix = value.map_or(&b""[..], c_prefix);
    output.extend((prefix.len() as u32).to_le_bytes());
    output.extend(prefix);
}

/// IDA: 00721440 NiExtraData_SetName: a nonempty C-string prefix, or null. Completion: 100%.
#[must_use]
pub fn extra_name_value(name: Option<&[u8]>) -> NiExtraDataName {
    NiExtraDataName(
        name.map(c_prefix)
            .filter(|prefix| !prefix.is_empty())
            .map(<[u8]>::to_vec),
    )
}

/// IDA: 00716B20 NiStringExtraData.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct StringExtraData {
    pub value: Option<Vec<u8>>,
}

impl StringExtraData {
    /// IDA: 00716BF0 IsEqual: nulls match only nulls; otherwise `strcmp` of the values, ignoring the name.
    #[must_use]
    pub fn equals_value(&self, other: &Self) -> bool {
        match (&self.value, &other.value) {
            (None, None) => true,
            (Some(left), Some(right)) => c_prefix(left) == c_prefix(right),
            _ => false,
        }
    }
}

/// IDA: 00716BA0 / 00721610 / 00713620: the modern name and value. Returns (name, data, consumed).
pub fn decode_string_extra_data_payload(
    bytes: &[u8],
    version: u32,
) -> Result<(NiExtraDataName, StringExtraData, usize), StreamError> {
    if version < 0x0500_000B {
        return Err(StreamError("legacy string extra-data payload unsupported"));
    }
    let mut reader = ByteReader::new(bytes);
    let name = read_string(&mut reader)?;
    let value = read_string(&mut reader)?;
    Ok((
        NiExtraDataName(name),
        StringExtraData { value },
        reader.position(),
    ))
}

/// IDA: 00716BD0 / 006FE000 / 00713720: the name and value prefixes.
#[must_use]
pub fn encode_string_extra_data_payload(name: &NiExtraDataName, data: &StringExtraData) -> Vec<u8> {
    let mut output = Vec::new();
    append_string(&mut output, name.0.as_deref());
    append_string(&mut output, data.value.as_deref());
    output
}

/// IDA: 00727F20 NiBinaryExtraData. Equality (00727FA0) is size and raw bytes, ignoring the name.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct BinaryExtraData {
    pub bytes: Vec<u8>,
}

/// IDA: 00728110 / 00721610: the modern name, size and bytes.
pub fn decode_binary_extra_data_payload(
    bytes: &[u8],
    version: u32,
) -> Result<(NiExtraDataName, BinaryExtraData, usize), StreamError> {
    if version < 0x0500_000B {
        return Err(StreamError("legacy binary extra-data payload unsupported"));
    }
    let mut reader = ByteReader::new(bytes);
    let name = read_string(&mut reader)?;
    let size = reader.u32("truncated binary size")? as usize;
    let contents = reader.take(size, "truncated binary contents")?.to_vec();
    Ok((
        NiExtraDataName(name),
        BinaryExtraData { bytes: contents },
        reader.position(),
    ))
}

/// IDA: 00728180 / 006FE000 / 00713720.
#[must_use]
pub fn encode_binary_extra_data_payload(name: &NiExtraDataName, data: &BinaryExtraData) -> Vec<u8> {
    let mut output = Vec::new();
    append_string(&mut output, name.0.as_deref());
    output.extend((data.bytes.len() as u32).to_le_bytes());
    output.extend(&data.bytes);
    output
}

/// IDA: one 006FB3B0 BSFurnitureMarker entry (0x10): +0x00 the offset (NiVector3_LoadBinary), +0x0C the heading word, +0x0E a
/// byte; a third byte is read and dropped.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FurnitureMarkerEntry {
    pub offset_bits: [u32; 3],
    pub heading_0c: u16,
    pub byte_0e: u8,
}

/// IDA: 006FBA20 BSBound_Create (0x24, 006FB8B0: NiExtraData then vtable A7D2D4, both vectors zero): +0x0C the center,
/// +0x18 the half extents, as raw f32 bits. Its clone is shared (vt+0x18 NiObject_CreateCloneShared) and its PostLoad a no-op.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BsBound {
    pub center_0c: [u32; 3],
    pub extents_18: [u32; 3],
}

/// IDA: 006FB910 BSBound_LoadBinary: NiExtraData_LoadBinary (the name), then NiVector3_LoadBinary into +0x0C and +0x18.
/// Completion: 100% at the modern floor.
pub fn decode_bound_payload(
    bytes: &[u8],
    version: u32,
) -> Result<(NiExtraDataName, BsBound, usize), StreamError> {
    const TRUNCATED: &str = "truncated BSBound";
    if version < 0x0A01_0072 {
        return Err(StreamError("legacy BSBound payload unsupported"));
    }
    let mut reader = ByteReader::new(bytes);
    let name = read_string(&mut reader)?;
    let vector = |reader: &mut ByteReader<'_>| -> Result<[u32; 3], StreamError> {
        Ok([
            reader.u32(TRUNCATED)?,
            reader.u32(TRUNCATED)?,
            reader.u32(TRUNCATED)?,
        ])
    };
    let center_0c = vector(&mut reader)?;
    let extents_18 = vector(&mut reader)?;
    Ok((
        NiExtraDataName(name),
        BsBound {
            center_0c,
            extents_18,
        },
        reader.position(),
    ))
}

/// IDA: 006FB340 BSFurnitureMarker (NiExtraData, "FRN"), the entries at +0x0C.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct BsFurnitureMarker {
    pub entries: Vec<FurnitureMarkerEntry>,
}

/// IDA: 006FB4E0 BSFurnitureMarker_LoadBinary: NiExtraData_LoadBinary (the name), a u32 count, then each 006FB3B0 entry. A
/// negative heading word is an old file's radians times 1000: `fild`, `fdiv` 1000.0 to a float, then WrappedAngle16_Set
/// (006FAEE0) wraps it into [0, 00B3F9A0 two pi) and stores it times 1000.0, chopped. That runs on the IO thread, at 53-bit
/// precision. Completion: 100% at the modern floor.
pub fn decode_furniture_marker_payload(
    bytes: &[u8],
    version: u32,
    two_pi: f32,
) -> Result<(NiExtraDataName, BsFurnitureMarker, usize), StreamError> {
    const TRUNCATED: &str = "truncated BSFurnitureMarker";
    if version < 0x0A01_0072 {
        return Err(StreamError("legacy BSFurnitureMarker payload unsupported"));
    }
    let mut reader = ByteReader::new(bytes);
    let name = read_string(&mut reader)?;
    let count = reader.u32(TRUNCATED)?;
    let mut marker = BsFurnitureMarker::default();
    for _ in 0..count {
        let offset_bits = [
            reader.u32(TRUNCATED)?,
            reader.u32(TRUNCATED)?,
            reader.u32(TRUNCATED)?,
        ];
        let mut heading_0c = reader.u16(TRUNCATED)?;
        let byte_0e = reader.u8(TRUNCATED)?;
        let _ = reader.u8(TRUNCATED)?;
        if (heading_0c as i16) < 0 {
            let angle = (f64::from(heading_0c as i16) / 1000.0) as f32;
            heading_0c = wrapped_angle_word_53(angle, two_pi);
        }
        marker.entries.push(FurnitureMarkerEntry {
            offset_bits,
            heading_0c,
            byte_0e,
        });
    }
    Ok((NiExtraDataName(name), marker, reader.position()))
}

// IDA: 006FAEE0 WrappedAngle16_Set at 53-bit precision: the period added or taken off by float stores, an unordered compare
// leaving each loop, then `fmul` 1000.0 and `fistp` with chop, the low word kept.
fn wrapped_angle_word_53(angle: f32, period: f32) -> u16 {
    let mut value = angle;
    while value < 0.0 {
        value = (f64::from(value) + f64::from(period)) as f32;
    }
    while value >= period {
        value = (f64::from(value) - f64::from(period)) as f32;
    }
    (f64::from(value) * 1000.0).trunc() as i32 as u16
}

/// A text key's time is kept as raw bits, which may be a NaN payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextKey {
    pub time_bits: u32,
    pub text: Option<Vec<u8>>,
}

/// IDA: 006D7450 NiTextKeyExtraData.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct TextKeyExtraData {
    pub keys: Vec<TextKey>,
}

/// IDA: 006D7780 / 006EC720 / 00713620: the modern name and keys. The bool is whether a load replaces existing
/// keys: a zero count leaves them.
pub fn decode_text_key_extra_data_payload(
    bytes: &[u8],
    version: u32,
) -> Result<(NiExtraDataName, TextKeyExtraData, usize, bool), StreamError> {
    if version < 0x0A01_0072 {
        return Err(StreamError("legacy text-key payload unsupported"));
    }
    let mut reader = ByteReader::new(bytes);
    let name = read_string(&mut reader)?;
    let count = reader.u32("truncated binary size")?;
    if count as usize > reader.remaining() / 8 {
        return Err(StreamError("truncated text-key array"));
    }
    let mut keys = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let time_bits = reader.u32("truncated binary size")?;
        let text = read_string(&mut reader)?;
        keys.push(TextKey { time_bits, text });
    }
    Ok((
        NiExtraDataName(name),
        TextKeyExtraData { keys },
        reader.position(),
        count != 0,
    ))
}

/// IDA: 006D7780's publication on an existing object: the name always; keys only for a nonzero count.
pub fn load_text_key_extra_data_payload(
    world: &mut World,
    target: Entity,
    bytes: &[u8],
    version: u32,
) -> Result<usize, StreamError> {
    let (name, data, consumed, replaces_keys) = decode_text_key_extra_data_payload(bytes, version)?;
    let mut entity = world.entity_mut(target);
    entity.insert(name);
    if replaces_keys {
        entity.insert(data);
    }
    Ok(consumed)
}

/// IDA: 006D7890 / 006EC760 / 00713720.
#[must_use]
pub fn encode_text_key_extra_data_payload(
    name: &NiExtraDataName,
    data: &TextKeyExtraData,
) -> Vec<u8> {
    let mut output = Vec::new();
    append_string(&mut output, name.0.as_deref());
    output.extend((data.keys.len() as u32).to_le_bytes());
    for key in &data.keys {
        output.extend(key.time_bits.to_le_bytes());
        append_string(&mut output, key.text.as_deref());
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ported from `gamebryo_extra_data`.
    #[test]
    fn string_equality_is_value_only() {
        let left = StringExtraData {
            value: Some(b"a\0hidden".to_vec()),
        };
        let right = StringExtraData {
            value: Some(b"a\0other".to_vec()),
        };
        assert!(left.equals_value(&right), "embedded suffixes do not count");
        assert!(!left.equals_value(&StringExtraData::default()));
        assert!(!StringExtraData::default().equals_value(&StringExtraData {
            value: Some(Vec::new())
        }));
    }

    #[test]
    fn string_payloads_decode() {
        let bytes = [
            3, 0, 0, 0, b'N', 0, b'X', 6, 0, 0, 0, b'V', 0, b'A', b'L', b'U', b'E', 0xAB,
        ];
        let (name, data, consumed) = decode_string_extra_data_payload(&bytes, 0x0500_000B).unwrap();
        assert_eq!(consumed, 17);
        assert_eq!(name.0.as_deref(), Some(&b"N\0X"[..]));
        assert_eq!(data.value.as_deref(), Some(&b"V\0ALUE"[..]));
        let (name, data, consumed) =
            decode_string_extra_data_payload(&[0; 8], 0x1400_0004).unwrap();
        assert!(name.0.is_none() && data.value.is_none());
        assert_eq!(consumed, 8);
        let negative = [0xFF, 0xFF, 0xFF, 0xFF, 0, 0, 0, 0];
        assert!(
            decode_string_extra_data_payload(&negative, 0x1400_0004)
                .unwrap()
                .0
                .0
                .is_none()
        );
        for length in 0..17 {
            assert!(
                decode_string_extra_data_payload(&bytes[..length], 0x1400_0004).is_err(),
                "{length}"
            );
        }
        assert!(decode_string_extra_data_payload(&bytes, 0x0500_000A).is_err());
        assert!(
            decode_string_extra_data_payload(&[0xFF, 0xFF, 0xFF, 0x7F, 0, 0, 0, 0], 0x1400_0004)
                .is_err()
        );
    }

    #[test]
    fn string_writer_stops_at_the_first_nul() {
        let name = NiExtraDataName(Some(b"N\0X".to_vec()));
        let data = StringExtraData {
            value: Some(b"V\0ALUE".to_vec()),
        };
        let written = encode_string_extra_data_payload(&name, &data);
        assert_eq!(written, [1, 0, 0, 0, b'N', 1, 0, 0, 0, b'V']);
        let (name, data, _) = decode_string_extra_data_payload(&written, 0x1400_0004).unwrap();
        assert_eq!(name.0.as_deref(), Some(&b"N"[..]));
        assert_eq!(data.value.as_deref(), Some(&b"V"[..]));
        let empty = encode_string_extra_data_payload(
            &NiExtraDataName(Some(Vec::new())),
            &StringExtraData {
                value: Some(Vec::new()),
            },
        );
        let (name, data, _) = decode_string_extra_data_payload(&empty, 0x1400_0004).unwrap();
        assert!(
            name.0.is_none() && data.value.is_none(),
            "empty strings reload as null"
        );
    }

    #[test]
    fn binary_payloads_round_trip() {
        let bytes = [
            3, 0, 0, 0, b'N', 0, b'X', 5, 0, 0, 0, 0, 0xFF, 0x41, 0, 0x80, 0xAB,
        ];
        let (name, data, consumed) = decode_binary_extra_data_payload(&bytes, 0x0500_000B).unwrap();
        assert_eq!(consumed, 16);
        assert_eq!(name.0.as_deref(), Some(&b"N\0X"[..]));
        assert_eq!(data.bytes, [0, 0xFF, 0x41, 0, 0x80]);
        for length in 0..16 {
            assert!(
                decode_binary_extra_data_payload(&bytes[..length], 0x1400_0004).is_err(),
                "{length}"
            );
        }
        assert!(decode_binary_extra_data_payload(&bytes, 0x0500_000A).is_err());
        assert!(
            decode_binary_extra_data_payload(&[0, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF], 0x1400_0004)
                .is_err()
        );
        let written = encode_binary_extra_data_payload(&name, &data);
        assert_eq!(
            written,
            [1, 0, 0, 0, b'N', 5, 0, 0, 0, 0, 0xFF, 0x41, 0, 0x80]
        );
        let (name, reloaded, _) = decode_binary_extra_data_payload(&written, 0x1400_0004).unwrap();
        assert_eq!(name.0.as_deref(), Some(&b"N"[..]));
        assert_eq!(reloaded, data);
        let empty =
            encode_binary_extra_data_payload(&NiExtraDataName(None), &BinaryExtraData::default());
        let (name, data, consumed) = decode_binary_extra_data_payload(&empty, 0x1400_0004).unwrap();
        assert!(name.0.is_none() && data.bytes.is_empty());
        assert_eq!(consumed, 8);
    }

    #[test]
    fn text_keys_keep_raw_times_and_order() {
        let bytes = [
            4, 0, 0, 0, b'N', b'a', b'm', b'e', 3, 0, 0, 0, 0x45, 0x23, 0xC1, 0x7F, 3, 0, 0, 0,
            b'A', 0, b'B', 0, 0, 0x80, 0x3F, 0xFF, 0xFF, 0xFF, 0xFF, 0, 0, 0x80, 0xBF, 0, 0, 0, 0,
            0xAB,
        ];
        let (name, data, consumed, replaces) =
            decode_text_key_extra_data_payload(&bytes, 0x1400_0004).unwrap();
        assert_eq!(consumed, 39);
        assert!(replaces);
        assert_eq!(name.0.as_deref(), Some(&b"Name"[..]));
        assert_eq!(data.keys.len(), 3);
        assert_eq!(
            data.keys[0].time_bits, 0x7FC1_2345,
            "the NaN payload survives"
        );
        assert_eq!(data.keys[0].text.as_deref(), Some(&b"A\0B"[..]));
        assert_eq!(f32::from_bits(data.keys[1].time_bits), 1.0);
        assert!(data.keys[1].text.is_none());
        assert_eq!(f32::from_bits(data.keys[2].time_bits), -1.0);
        for length in 0..39 {
            assert!(
                decode_text_key_extra_data_payload(&bytes[..length], 0x1400_0004).is_err(),
                "{length}"
            );
        }
        assert!(decode_text_key_extra_data_payload(&bytes, 0x0A01_0071).is_err());
        assert!(
            decode_text_key_extra_data_payload(&[0, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF], 0x1400_0004)
                .is_err()
        );
    }

    #[test]
    fn text_key_saves_and_zero_count_loads() {
        let name = NiExtraDataName(Some(b"N\0hidden".to_vec()));
        let data = TextKeyExtraData {
            keys: vec![TextKey {
                time_bits: 0x7FC1_2345,
                text: Some(b"A\0B".to_vec()),
            }],
        };
        let written = encode_text_key_extra_data_payload(&name, &data);
        assert_eq!(
            written,
            [
                1, 0, 0, 0, b'N', 1, 0, 0, 0, 0x45, 0x23, 0xC1, 0x7F, 1, 0, 0, 0, b'A'
            ]
        );

        let mut world = World::new();
        let retained = TextKeyExtraData {
            keys: vec![TextKey {
                time_bits: 42.0f32.to_bits(),
                text: Some(b"retained".to_vec()),
            }],
        };
        let existing = world
            .spawn((NiExtraDataName(Some(b"old".to_vec())), retained.clone()))
            .id();
        let zero = [3, 0, 0, 0, b'n', b'e', b'w', 0, 0, 0, 0];
        assert_eq!(
            load_text_key_extra_data_payload(&mut world, existing, &zero, 0x1400_0004).unwrap(),
            11
        );
        assert_eq!(
            world.get::<NiExtraDataName>(existing).unwrap().0.as_deref(),
            Some(&b"new"[..])
        );
        assert_eq!(
            world.get::<TextKeyExtraData>(existing).unwrap(),
            &retained,
            "zero keys keep the old ones"
        );
        let (_, cold, _, replaces) =
            decode_text_key_extra_data_payload(&zero, 0x1400_0004).unwrap();
        assert!(!replaces && cold.keys.is_empty());
    }

    #[test]
    fn the_name_setter_keeps_a_nonempty_prefix() {
        assert_eq!(
            extra_name_value(Some(b"new\0hidden")).0.as_deref(),
            Some(&b"new"[..])
        );
        assert!(extra_name_value(Some(b"")).0.is_none());
        assert!(extra_name_value(Some(b"\0hidden")).0.is_none());
        assert!(extra_name_value(None).0.is_none());
    }

    // 006FB3B0: a negative heading is radians times 1000 from an old file, wrapped into [0, two pi) and stored times 1000.
    #[test]
    fn a_furniture_marker_wraps_a_negative_heading() {
        let mut bytes = 3_u32.to_le_bytes().to_vec();
        bytes.extend_from_slice(b"FRN");
        bytes.extend_from_slice(&1_u32.to_le_bytes());
        bytes.extend_from_slice(&[0; 12]);
        bytes.extend_from_slice(&(-1000_i16).to_le_bytes());
        bytes.extend_from_slice(&[5, 9]);
        let (name, marker, consumed) =
            decode_furniture_marker_payload(&bytes, 0x1400_0004, f32::from_bits(0x40C9_0FDB))
                .unwrap();
        assert_eq!(consumed, bytes.len());
        assert_eq!(name.0.as_deref(), Some(b"FRN".as_slice()));
        assert_eq!(marker.entries[0].byte_0e, 5);
        // -1.0 + 6.2831855 = 5.2831855, times 1000 chopped.
        assert_eq!(marker.entries[0].heading_0c, 5283);
    }
}
