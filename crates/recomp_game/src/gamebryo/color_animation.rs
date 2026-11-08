//! NiColorData. Host-owned storage; this is not the original 20-byte key ABI.

use bevy_ecs::prelude::*;
use recomp_core::reader::{ByteReader, StreamError};

const MODERN_NI_OBJECT_VERSION: u32 = 0x0A01_0072;
const COLOR_KEY_BYTES: usize = 20;
const TRUNCATED: &str = "truncated NiColorData payload";

/// The serialized u32 key type, kept raw.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ColorKeyType(pub u32);

impl ColorKeyType {
    pub const NONE: Self = Self(0);
    pub const LINEAR: Self = Self(1);
    pub const STEP: Self = Self(5);
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ColorKey {
    pub time: f32,
    pub rgba: [f32; 4],
}

// IDA: 006E4790 dispatch entries 1 and 5
// Completion: 100% of the supported payload type gate
// TODO: [ ] Recover entries 0, 2, 3 and 4 before accepting their payloads.
fn supported(key_type: ColorKeyType) -> bool {
    key_type == ColorKeyType::LINEAR || key_type == ColorKeyType::STEP
}

/// IDA: 006E4520 NiColorData factory. Cold: no keys, type 0.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct ColorKeyData {
    key_type: ColorKeyType,
    keys: Vec<ColorKey>,
}

impl ColorKeyData {
    // IDA: 006E4520 / 006E4640 a populated key owner
    // Completion: 100% of the supported type check and raw key retention
    // TODO: [ ] Recover original storage allocation and key evaluation callbacks.
    pub fn new(key_type: ColorKeyType, keys: Vec<ColorKey>) -> Result<Self, StreamError> {
        if !supported(key_type) || keys.is_empty() {
            return Err(StreamError("unsupported or empty NiColorData keys"));
        }
        Ok(Self { key_type, keys })
    }

    /// IDA: N/A an accessor.
    #[must_use]
    pub const fn key_type(&self) -> ColorKeyType {
        self.key_type
    }

    /// IDA: N/A an accessor.
    #[must_use]
    pub fn keys(&self) -> &[ColorKey] {
        &self.keys
    }

    // IDA: 006BE040 search and 006BECE0 / 006C1FC0 color callbacks
    // Completion: 100% of the valid host domain
    // TODO: [ ] Compare x87 intermediates and malformed-key behaviour against the runtime.
    pub fn sample(&self, time: f32, cached_left: &mut u32) -> Option<[f32; 4]> {
        let keys = &self.keys;
        if keys.is_empty() {
            return None;
        }
        if keys.len() == 1 || time == -f32::MAX {
            return Some(keys[0].rgba);
        }
        if !supported(self.key_type) || !time.is_finite() {
            return None;
        }
        for (index, key) in keys.iter().enumerate() {
            if !key.time.is_finite() || (index != 0 && key.time <= keys[index - 1].time) {
                return None;
            }
            if key.rgba.iter().any(|component| !component.is_finite()) {
                return None;
            }
        }
        if time < keys[0].time || time > keys[keys.len() - 1].time {
            return None;
        }
        let mut left = if (*cached_left as usize) < keys.len() - 1 {
            *cached_left as usize
        } else {
            0
        };
        if keys[left].time > time {
            left = 0;
        }
        while left + 1 < keys.len() - 1 && keys[left + 1].time < time {
            left += 1;
        }
        let first = &keys[left];
        let second = &keys[left + 1];
        let ratio = (time - first.time) / (second.time - first.time);
        let mut result = [0.0; 4];
        if self.key_type == ColorKeyType::STEP {
            result = if ratio >= 1.0 {
                second.rgba
            } else {
                first.rgba
            };
        } else {
            let first_weight = 1.0 - ratio;
            for (index, component) in result.iter_mut().enumerate() {
                let first_part = first.rgba[index] * first_weight;
                let second_part = second.rgba[index] * ratio;
                *component = second_part + first_part;
            }
        }
        *cached_left = left as u32;
        Some(result)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ColorKeyPayload {
    pub data: ColorKeyData,
    pub bytes_consumed: usize,
}

// IDA: 006E4790 / 006C2270 / 006BD510 / 00715420
// Completion: 100% of the modern bounded raw payload
// TODO: [ ] Compare NiStream callback failures and endian handling in the original runtime.
pub fn decode_color_key_payload(
    bytes: &[u8],
    version: u32,
) -> Result<ColorKeyPayload, StreamError> {
    if version < MODERN_NI_OBJECT_VERSION {
        return Err(StreamError("unsupported NiColorData stream version"));
    }
    let mut reader = ByteReader::new(bytes);
    let count = reader.u32(TRUNCATED)?;
    if count == 0 {
        return Ok(ColorKeyPayload {
            data: ColorKeyData::default(),
            bytes_consumed: reader.position(),
        });
    }
    let key_type = ColorKeyType(reader.u32(TRUNCATED)?);
    if !supported(key_type) {
        return Err(StreamError("unsupported NiColorData key type"));
    }
    if count as usize > reader.remaining() / COLOR_KEY_BYTES {
        return Err(StreamError("truncated NiColorData keys"));
    }
    let mut keys = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let mut key = ColorKey {
            time: reader.f32(TRUNCATED)?,
            rgba: [0.0; 4],
        };
        for component in &mut key.rgba {
            *component = reader.f32(TRUNCATED)?;
        }
        keys.push(key);
    }
    Ok(ColorKeyPayload {
        data: ColorKeyData::new(key_type, keys)?,
        bytes_consumed: reader.position(),
    })
}

// IDA: 006E4790 count-zero branch and 006E4640 positive replacement
// Completion: 100% of the host commit policy
// TODO: [ ] Compare original heap failure and destructor or refcount side effects.
pub fn load_color_key_payload(
    target: &mut ColorKeyData,
    bytes: &[u8],
    version: u32,
) -> Result<usize, StreamError> {
    let decoded = decode_color_key_payload(bytes, version)?;
    if !decoded.data.keys().is_empty() {
        *target = decoded.data;
    }
    Ok(decoded.bytes_consumed)
}

// IDA: 006E4820 / 006BEFA0
// Completion: 100% of the raw count, type, time and RGBA payload for types 1 and 5
// TODO: [ ] Compare original NiStream writer failure and endian behaviour.
pub fn encode_color_key_payload(data: &ColorKeyData) -> Result<Vec<u8>, StreamError> {
    let count =
        u32::try_from(data.keys().len()).map_err(|_| StreamError("too many NiColorData keys"))?;
    let mut output = count.to_le_bytes().to_vec();
    if data.keys().is_empty() {
        return Ok(output);
    }
    if !supported(data.key_type()) {
        return Err(StreamError("unsupported NiColorData key type"));
    }
    output.extend(data.key_type().0.to_le_bytes());
    for key in data.keys() {
        output.extend(key.time.to_bits().to_le_bytes());
        for component in &key.rgba {
            output.extend(component.to_bits().to_le_bytes());
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODERN_VERSION: u32 = 0x1400_0004;

    fn word(output: &mut Vec<u8>, value: u32) {
        output.extend(value.to_le_bytes());
    }

    fn make_keys(key_type: ColorKeyType) -> Vec<u8> {
        let mut output = Vec::new();
        word(&mut output, 2);
        word(&mut output, key_type.0);
        for value in [
            0x7FC1_2345,
            0x8000_0000,
            0x3F80_0000,
            0x7F80_0000,
            0xFF80_0000,
            0xBF80_0000,
            0x7FA1_2345,
            0x0000_0000,
            0x4120_0000,
            0x3E80_0000,
        ] {
            word(&mut output, value);
        }
        output
    }

    /// Ported from `color_animation_tests`: IDA 006E4520 cold and the 006E4790 / 006E4820 storage mirror.
    #[test]
    fn cold_and_roundtrip() {
        let cold = ColorKeyData::default();
        assert_eq!(cold.key_type(), ColorKeyType::NONE);
        assert!(cold.keys().is_empty());
        assert_eq!(encode_color_key_payload(&cold).unwrap(), vec![0; 4]);
        for key_type in [ColorKeyType::LINEAR, ColorKeyType::STEP] {
            let bytes = make_keys(key_type);
            let decoded = decode_color_key_payload(&bytes, MODERN_VERSION).unwrap();
            assert_eq!(decoded.bytes_consumed, bytes.len());
            assert_eq!(decoded.data.key_type(), key_type);
            assert_eq!(decoded.data.keys().len(), 2);
            assert_eq!(decoded.data.keys()[0].time.to_bits(), 0x7FC1_2345);
            assert_eq!(decoded.data.keys()[0].rgba[0].to_bits(), 0x8000_0000);
            assert_eq!(decoded.data.keys()[1].time.to_bits(), 0xBF80_0000);
            assert_eq!(encode_color_key_payload(&decoded.data).unwrap(), bytes);
        }
    }

    /// IDA: 006E4790's count-zero branch keeps the existing owner.
    #[test]
    fn reload_and_consumption() {
        let bytes = make_keys(ColorKeyType::STEP);
        let mut data = decode_color_key_payload(&bytes, MODERN_VERSION)
            .unwrap()
            .data;
        let key_address = data.keys().as_ptr();
        let mut zero = vec![0; 4];
        word(&mut zero, 0x1234_5678);
        assert_eq!(
            load_color_key_payload(&mut data, &zero, MODERN_VERSION),
            Ok(4)
        );
        assert_eq!(data.keys().as_ptr(), key_address);
        assert_eq!(data.key_type(), ColorKeyType::STEP);
        assert_eq!(data.keys().len(), 2);
        let mut replacement = make_keys(ColorKeyType::LINEAR);
        replacement.push(0xA5);
        assert_eq!(
            load_color_key_payload(&mut data, &replacement, MODERN_VERSION),
            Ok(bytes.len())
        );
        assert_eq!(data.key_type(), ColorKeyType::LINEAR);
        assert_eq!(data.keys().len(), 2);
        let replacement_address = data.keys().as_ptr();
        assert!(load_color_key_payload(&mut data, &replacement[..12], MODERN_VERSION).is_err());
        assert_eq!(data.keys().as_ptr(), replacement_address);
        assert_eq!(data.key_type(), ColorKeyType::LINEAR);
    }

    /// IDA: 006E4790's dispatch and 006C2270's count allocation.
    #[test]
    fn rejections() {
        let bytes = make_keys(ColorKeyType::LINEAR);
        for length in 0..bytes.len() {
            assert!(decode_color_key_payload(&bytes[..length], MODERN_VERSION).is_err());
        }
        for key_type in [0u32, 2, 3, 4, 6, 0xFFFF_FFFF] {
            let mut unknown = bytes.clone();
            unknown[4..8].copy_from_slice(&key_type.to_le_bytes());
            assert!(decode_color_key_payload(&unknown, MODERN_VERSION).is_err());
        }
        let mut huge = Vec::new();
        word(&mut huge, 0xFFFF_FFFF);
        word(&mut huge, 1);
        assert!(decode_color_key_payload(&huge, MODERN_VERSION).is_err());
        assert!(decode_color_key_payload(&bytes, 0x0A01_0071).is_err());
    }

    /// IDA: 006BE040 / 006BECE0 / 006C1FC0's sample and cache branches.
    #[test]
    fn sampling() {
        let cold = ColorKeyData::default();
        let mut cached = 9;
        assert_eq!(cold.sample(0.0, &mut cached), None);
        assert_eq!(cached, 9);
        let nan = f32::from_bits(0x7FC1_2345);
        let single = ColorKeyData::new(
            ColorKeyType::LINEAR,
            vec![ColorKey {
                time: nan,
                rgba: [nan, -0.0, 1.0, 2.0],
            }],
        )
        .unwrap();
        let value = single.sample(8.0, &mut cached).unwrap();
        assert_eq!(value[0].to_bits(), 0x7FC1_2345);
        assert_eq!(cached, 9);
        let keys: Vec<ColorKey> = (0..7)
            .map(|index| ColorKey {
                time: index as f32,
                rgba: [
                    (index * 10) as f32,
                    (index * 20) as f32,
                    (index * 30) as f32,
                    (index * 40) as f32,
                ],
            })
            .collect();
        let linear = ColorKeyData::new(ColorKeyType::LINEAR, keys.clone()).unwrap();
        cached = 0;
        let value = linear.sample(4.5, &mut cached).unwrap();
        assert_eq!(value[0], 45.0);
        assert_eq!(value[3], 180.0);
        assert_eq!(cached, 4);
        assert_eq!(linear.sample(1.5, &mut cached).unwrap()[0], 15.0);
        assert_eq!(cached, 1);
        assert_eq!(linear.sample(3.0, &mut cached).unwrap()[0], 30.0);
        assert_eq!(cached, 2);
        assert_eq!(linear.sample(6.0, &mut cached).unwrap()[0], 60.0);
        assert_eq!(cached, 5);
        cached = 100;
        assert_eq!(linear.sample(0.5, &mut cached).unwrap()[0], 5.0);
        assert_eq!(cached, 0);
        cached = 3;
        assert_eq!(linear.sample(-f32::MAX, &mut cached).unwrap()[0], 0.0);
        assert_eq!(cached, 3);
        let step = ColorKeyData::new(ColorKeyType::STEP, keys.clone()).unwrap();
        cached = 0;
        assert_eq!(step.sample(2.99, &mut cached).unwrap()[0], 20.0);
        assert_eq!(cached, 2);
        assert_eq!(step.sample(3.0, &mut cached).unwrap()[0], 30.0);
        cached = 3;
        assert_eq!(linear.sample(-1.0, &mut cached), None);
        assert_eq!(cached, 3);
        assert_eq!(linear.sample(nan, &mut cached), None);
        assert_eq!(cached, 3);
        let mut unsorted = keys.clone();
        unsorted[3].time = unsorted[2].time;
        let bad_times = ColorKeyData::new(ColorKeyType::LINEAR, unsorted).unwrap();
        assert_eq!(bad_times.sample(2.5, &mut cached), None);
        assert_eq!(cached, 3);
        let mut nonfinite = keys;
        nonfinite[4].rgba[1] = nan;
        let bad_rgba = ColorKeyData::new(ColorKeyType::LINEAR, nonfinite).unwrap();
        assert_eq!(bad_rgba.sample(2.5, &mut cached), None);
        assert_eq!(cached, 3);
    }
}
