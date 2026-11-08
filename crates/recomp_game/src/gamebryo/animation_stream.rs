//! The key-array payloads of NiFloatData and NiBoolData.

use recomp_core::reader::{ByteReader, StreamError};

use super::animation::{FloatKey, FloatKeyData, FloatKeyType};
use super::bool_animation::{BoolKey, BoolKeyData, BoolKeyType};

const TRUNCATED: &str = "truncated float key payload";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DecodedFloatKeys {
    pub data: FloatKeyData,
    pub bytes_consumed: usize,
}

// IDA: 006E3690 count and type dispatch after the NiObject base read, then 006C2590 / 006BB960 / 006C1AD0
// Completion: 100% of the bounded little-endian payload for supported finite host keys; later bytes stay unconsumed
// TODO: [ ] Bind the versioned NiObject prefix, object graph and NiStream error policy; legacy stream conversion.
pub fn decode_float_key_payload(bytes: &[u8]) -> Result<DecodedFloatKeys, StreamError> {
    let mut reader = ByteReader::new(bytes);
    let count = reader.u32(TRUNCATED)?;
    if count == 0 {
        return Ok(DecodedFloatKeys {
            data: FloatKeyData::default(),
            bytes_consumed: reader.position(),
        });
    }
    let raw_type = reader.u32(TRUNCATED)?;
    let stride: usize = match raw_type {
        1 | 5 => 8,
        2 => 16,
        // Serialized T/C/B; the two derived tangents are not stored.
        3 => 20,
        _ => return Err(StreamError("unsupported float key payload type")),
    };
    if count as usize > reader.remaining() / stride {
        return Err(StreamError("truncated float key array"));
    }
    let mut keys = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let mut key = FloatKey {
            time: reader.f32(TRUNCATED)?,
            value: reader.f32(TRUNCATED)?,
            ..FloatKey::default()
        };
        if raw_type == 2 {
            key.in_tangent = reader.f32(TRUNCATED)?;
            key.out_tangent = reader.f32(TRUNCATED)?;
        } else if raw_type == 3 {
            key.tension = reader.f32(TRUNCATED)?;
            key.continuity = reader.f32(TRUNCATED)?;
            key.bias = reader.f32(TRUNCATED)?;
        }
        keys.push(key);
    }
    let data = if raw_type == 3 {
        FloatKeyData::with_generated_tbc(keys)?
    } else {
        FloatKeyData::new(FloatKeyType(raw_type as u8), keys)?
    };
    Ok(DecodedFloatKeys {
        data,
        bytes_consumed: reader.position(),
    })
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DecodedBoolKeys {
    pub data: BoolKeyData,
    pub bytes_consumed: usize,
}

// IDA: 006E8A30 defaults to step below 0A010068; 006C1E10 / 006BDF60 time and normalized-byte key records
// Completion: 100% of the bounded little-endian payload after the versioned NiObject base prefix
// TODO: [ ] Bind whole-file version parsing, legacy conversion and graph ownership.
pub fn decode_bool_key_payload(bytes: &[u8], version: u32) -> Result<DecodedBoolKeys, StreamError> {
    let mut reader = ByteReader::new(bytes);
    let count = reader.u32(TRUNCATED)?;
    if count == 0 {
        return Ok(DecodedBoolKeys {
            data: BoolKeyData::default(),
            bytes_consumed: reader.position(),
        });
    }
    let key_type = if version >= 0x0A01_0068 {
        reader.u32(TRUNCATED)?
    } else {
        5
    };
    if key_type != 5 {
        return Err(StreamError("unsupported bool key payload type"));
    }
    if count as usize > reader.remaining() / 5 {
        return Err(StreamError("truncated bool key array"));
    }
    let mut keys = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let time = reader.f32(TRUNCATED)?;
        let value = u8::from(reader.u8(TRUNCATED)? != 0);
        keys.push(BoolKey { time, value });
    }
    Ok(DecodedBoolKeys {
        data: BoolKeyData::new(BoolKeyType::STEP, keys)?,
        bytes_consumed: reader.position(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ported from `animation_stream_tests`: IDA 006E3690's scalar payload contract.
    #[test]
    fn float_payloads() {
        let linear: [u8; 25] = [
            2, 0, 0, 0, 1, 0, 0, 0, // count, linear type
            0, 0, 0, 0, 0, 0, 0, 64, // time 0, value 2
            0, 0, 0, 64, 0, 0, 192, 64,   // time 2, value 6
            0xAB, // The next block must remain unconsumed.
        ];
        let decoded = decode_float_key_payload(&linear).unwrap();
        let mut index = 0;
        assert_eq!(decoded.bytes_consumed, 24);
        assert_eq!(decoded.data.sample(1.0, &mut index), Some(4.0));
        for length in 0..24 {
            assert!(decode_float_key_payload(&linear[..length]).is_err());
        }
        let mut unsupported = linear;
        unsupported[4] = 4;
        assert!(decode_float_key_payload(&unsupported).is_err());

        let mut step = linear;
        step[4] = 5;
        let stepped = decode_float_key_payload(&step).unwrap();
        assert_eq!(stepped.data.sample(1.0, &mut index), Some(2.0));
        assert_eq!(stepped.data.sample(2.0, &mut index), Some(6.0));

        let quadratic: [u8; 40] = [
            2, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 128, 64, 0, 0, 128,
            63, 0, 0, 128, 63, 0, 0, 0, 0, 0, 0, 0, 0,
        ];
        let curved = decode_float_key_payload(&quadratic).unwrap();
        assert_eq!(curved.bytes_consumed, 40);
        assert_eq!(curved.data.sample(0.5, &mut index), Some(1.0));

        let cold = decode_float_key_payload(&[0; 4]).unwrap();
        assert_eq!(cold.bytes_consumed, 4);
        assert!(cold.data.keys().is_empty());

        let tbc: [u8; 48] = [
            2, 0, 0, 0, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 64, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 64, 0, 0, 192, 64, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ];
        let derived = decode_float_key_payload(&tbc).unwrap();
        assert_eq!(derived.bytes_consumed, 48);
        assert!((derived.data.sample(1.0, &mut index).unwrap() - 4.0).abs() < 0.0001);
        assert_eq!(derived.data.keys()[0].tbc_out_tangent, 4.0);
        assert_eq!(derived.data.keys()[1].tbc_in_tangent, 4.0);
    }

    /// IDA: 006E8A30 / 006BDF60's version boundary and byte normalization.
    #[test]
    fn bool_payloads() {
        let modern: [u8; 19] = [
            2, 0, 0, 0, 5, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 128, 63, 0xFF, 0xAB,
        ];
        let data = decode_bool_key_payload(&modern, 0x0A01_0068).unwrap();
        let mut index = 0;
        assert_eq!(data.bytes_consumed, 18);
        assert_eq!(data.data.keys()[1].value, 1);
        assert_eq!(data.data.sample(0.5, &mut index), Some(0));
        assert_eq!(data.data.sample(1.0, &mut index), Some(1));
        let legacy: [u8; 14] = [2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 128, 63, 1];
        let old = decode_bool_key_payload(&legacy, 0x0A01_0067).unwrap();
        assert_eq!(old.bytes_consumed, 14);
        assert_eq!(old.data.keys().len(), 2);
        for length in 0..18 {
            assert!(decode_bool_key_payload(&modern[..length], 0x1400_0004).is_err());
        }
        let mut unsupported = modern;
        unsupported[4] = 1;
        assert!(decode_bool_key_payload(&unsupported, 0x1400_0004).is_err());
        assert_eq!(
            decode_bool_key_payload(&[0; 4], 0x1400_0004)
                .unwrap()
                .bytes_consumed,
            4
        );
    }
}
