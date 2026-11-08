//! NiStringPalette's payload. A value component: the bytes' length is the original's capacity, not a
//! C string, and embedded NUL bytes are kept.

use bevy_ecs::prelude::*;
use recomp_core::reader::{ByteReader, StreamError};

/// IDA: 006D7AD0 cold 0x14-byte object; fields +8 / +C / +10.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct StringPalette {
    bytes: Vec<u8>,
    used_length: u32,
}

impl StringPalette {
    /// IDA: N/A a validated owner for the fields 006D7CF0 loads.
    pub fn new(capacity_bytes: Vec<u8>, used_length: u32) -> Result<Self, StreamError> {
        if used_length as usize > capacity_bytes.len() {
            return Err(StreamError("string palette used length exceeds capacity"));
        }
        Ok(Self {
            bytes: capacity_bytes,
            used_length,
        })
    }

    /// IDA: N/A an accessor.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    /// IDA: N/A an accessor.
    #[must_use]
    pub const fn capacity(&self) -> u32 {
        self.bytes.len() as u32
    }
    /// IDA: N/A an accessor.
    #[must_use]
    pub const fn used_length(&self) -> u32 {
        self.used_length
    }
}

/// IDA: 006D7CF0 after the NiObject base: u32 capacity, the bytes, u32 used. Returns the palette and
/// the bytes consumed. Completion: 100% of the payload.
/// TODO: [ ] Bind the NiObject prefix, stream version and endianness.
pub fn decode_string_palette_payload(bytes: &[u8]) -> Result<(StringPalette, usize), StreamError> {
    let mut reader = ByteReader::new(bytes);
    let capacity = reader.u32("truncated string palette length")? as usize;
    let buffer = reader
        .take(capacity, "truncated string palette buffer")?
        .to_vec();
    let used = reader.u32("truncated string palette buffer")?;
    Ok((StringPalette::new(buffer, used)?, reader.position()))
}

/// IDA: 006D7C50 writes used, the first used bytes, then used, compacting the capacity. Completion: 100%
/// of the payload.
#[must_use]
pub fn encode_string_palette_payload(palette: &StringPalette) -> Vec<u8> {
    let used = palette.used_length;
    let mut bytes = Vec::with_capacity(used as usize + 8);
    bytes.extend(used.to_le_bytes());
    bytes.extend(&palette.bytes[..used as usize]);
    bytes.extend(used.to_le_bytes());
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ported from `gamebryo_string_palette`, except the factory check, which comes with the factory.
    #[test]
    fn empty_palette_round_trips() {
        let empty = StringPalette::default();
        assert_eq!((empty.capacity(), empty.used_length()), (0, 0));
        let encoded = encode_string_palette_payload(&empty);
        assert_eq!(encoded, [0; 8]);
        assert_eq!(decode_string_palette_payload(&encoded).unwrap(), (empty, 8));
    }

    #[test]
    fn loads_keep_capacity_and_writes_compact() {
        let buffer = b"A\0BXY".to_vec();
        let source = StringPalette::new(buffer.clone(), 3).unwrap();
        let wire = [5, 0, 0, 0, b'A', 0, b'B', b'X', b'Y', 3, 0, 0, 0, 0xEE];
        let (loaded, consumed) = decode_string_palette_payload(&wire).unwrap();
        assert_eq!(consumed, 13);
        assert_eq!(loaded.capacity(), 5);
        assert_eq!(loaded.used_length(), 3);
        assert_eq!(loaded.bytes(), buffer);
        assert_eq!(loaded, source);
        let compact_wire = encode_string_palette_payload(&source);
        assert_eq!(compact_wire, [3, 0, 0, 0, b'A', 0, b'B', 3, 0, 0, 0]);
        let (compact, _) = decode_string_palette_payload(&compact_wire).unwrap();
        assert_eq!((compact.capacity(), compact.used_length()), (3, 3));
        assert_ne!(
            compact, source,
            "compact capacity is not the original capacity"
        );
    }

    /// IDA: 006D7B50 compares capacity, used and all capacity bytes.
    #[test]
    fn equality_covers_the_whole_capacity() {
        let baseline = StringPalette::new(b"A\0Z".to_vec(), 2).unwrap();
        assert_eq!(baseline, StringPalette::new(b"A\0Z".to_vec(), 2).unwrap());
        assert_ne!(baseline, StringPalette::new(b"A\0Y".to_vec(), 2).unwrap());
        assert_ne!(baseline, StringPalette::new(b"A\0Z".to_vec(), 1).unwrap());
        assert_ne!(baseline, StringPalette::new(b"A\0".to_vec(), 2).unwrap());
    }

    #[test]
    fn malformed_payloads_are_rejected() {
        assert!(decode_string_palette_payload(&[1, 0]).is_err());
        assert!(decode_string_palette_payload(&[3, 0, 0, 0, b'A', b'B']).is_err());
        assert!(decode_string_palette_payload(&[1, 0, 0, 0, b'A', 2, 0, 0, 0]).is_err());
        assert!(StringPalette::new(b"A".to_vec(), 2).is_err());
    }
}
