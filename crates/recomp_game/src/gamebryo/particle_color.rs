//! NiPSysColorModifier.

use bevy_ecs::prelude::*;
use recomp_core::reader::{ByteReader, StreamError};

use super::color_animation::ColorKeyData;
use super::particle_dynamics::{ParticleModifierBase, read_modifier_name};
use super::stream_objects::{NULL_REFERENCE, StreamObjectTable};

const TRUNCATED_U32: &str = "truncated particle modifier payload";

/// IDA: N/A what this modifier reads from NiColorData (006E4520): the first and last key times, and the 006BE040 /
/// 006BECE0 / 006C1FC0 sequence sample. `ColorKeyData` implements it. Completion: 100% of the boundary.
pub trait ParticleColorKeys {
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    /// The stored first and last key times, or `None` for no keys.
    fn first_and_last_times(&self) -> Option<(f32, f32)>;
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    /// The sequence sample; `None` is outside the supported sample domain.
    fn sample(&self, time: f32, cached_left: &mut usize) -> Option<[f32; 4]>;
}

impl ParticleColorKeys for ColorKeyData {
    // IDA: 0075A3F0 reads the first and last key times of NiColorData
    // Completion: 100%; empty data gives no bounds
    fn first_and_last_times(&self) -> Option<(f32, f32)> {
        let keys = self.keys();
        Some((keys.first()?.time, keys.last()?.time))
    }

    // IDA: 006BE040 / 006BECE0 / 006C1FC0 through `ColorKeyData::sample`
    // Completion: 100% of the adapter; the cached left index stays in range of the u32 the sampler keeps
    fn sample(&self, time: f32, cached_left: &mut usize) -> Option<[f32; 4]> {
        let mut left = *cached_left as u32;
        let color = ColorKeyData::sample(self, time, &mut left);
        *cached_left = left as usize;
        color
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ParticleColorSample {
    pub age: f32,
    pub lifespan: f32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParticleColorPending {
    pub name: Option<Vec<u8>>,
    pub order: u32,
    pub target_index: u32,
    pub active: bool,
    pub data_index: u32,
    pub bytes_consumed: usize,
}

impl Default for ParticleColorPending {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            name: None,
            order: 0,
            target_index: NULL_REFERENCE,
            active: true,
            data_index: NULL_REFERENCE,
            bytes_consumed: 0,
        }
    }
}

/// IDA: 0075A360 creates the 0x24-byte NiPSysColorModifier via 00752BF0.
/// TODO: [ ] The target system owner and modifier scheduling.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct ParticleColorModifier {
    pub base: ParticleModifierBase,
    /// IDA: 0075A3F0 +0x18 the retained NiColorData.
    pub data_18: Option<Entity>,
    pub min_time_1c: f32,
    pub max_time_20: f32,
}

impl ParticleColorModifier {
    /// IDA: 0075A3F0 releases and replaces NiColorData, then caches its first and last times; empty data gives zero
    /// bounds. Completion: 100% of the observed bounds.
    /// TODO: [ ] Validate refcount timing on already-populated modifiers, and malformed key bound reads.
    pub fn set_data(&mut self, data: Option<(Entity, &dyn ParticleColorKeys)>) {
        self.data_18 = data.map(|held| held.0);
        self.min_time_1c = 0.0;
        self.max_time_20 = 0.0;
        if let Some((first, last)) = data.and_then(|held| held.1.first_and_last_times()) {
            self.min_time_1c = first;
            self.max_time_20 = last;
        }
    }

    /// IDA: 0075A250 divides age by lifespan, clamps first to min then max, and samples each particle with a fresh zero
    /// cached-left index. Returns false with no writes when no data is bound; the active flag is the caller's. `keys`
    /// is the component of `data_18`, which the caller reads from the world. Completion: 100% of the finite
    /// positive-lifespan domain with preflighted writes.
    /// TODO: [ ] Compare particle runtime output and x87 rounding.
    pub fn update_colors(
        &self,
        particles: &[ParticleColorSample],
        output: &mut [[f32; 4]],
        keys: Option<&dyn ParticleColorKeys>,
    ) -> Result<bool, StreamError> {
        if particles.len() != output.len() {
            return Err(StreamError("particle and color output counts differ"));
        }
        if self.data_18.is_none() {
            return Ok(false);
        }
        let keys = keys.ok_or(StreamError("particle color keys not supplied"))?;
        let mut prepared = Vec::with_capacity(particles.len());
        for particle in particles {
            if !particle.age.is_finite()
                || !particle.lifespan.is_finite()
                || particle.lifespan <= 0.0
            {
                return Err(StreamError(
                    "particle age/lifespan outside supported domain",
                ));
            }
            let mut time = particle.age / particle.lifespan;
            if !time.is_finite() {
                return Err(StreamError("particle time ratio outside supported domain"));
            }
            if self.min_time_1c >= time {
                time = self.min_time_1c;
            }
            if self.max_time_20 <= time {
                time = self.max_time_20;
            }
            let mut cached_left = 0;
            let color = keys.sample(time, &mut cached_left).ok_or(StreamError(
                "particle color keys outside supported sample domain",
            ))?;
            prepared.push(color);
        }
        output.copy_from_slice(&prepared);
        Ok(true)
    }
}

/// IDA: 00752DC0 reads the base name, order, deferred target and one-byte active; 0075A220 queues one more color-data
/// link. Completion: 100% of the modern >= 0A010072 bounded payload and deferred indices.
/// TODO: [ ] The older NiObject prefix and NiStream error codes.
pub fn parse_particle_color_payload(
    bytes: &[u8],
    version: u32,
) -> Result<ParticleColorPending, StreamError> {
    if version < 0x0A01_0072 {
        return Err(StreamError("legacy particle modifier payload unsupported"));
    }
    let mut reader = ByteReader::new(bytes);
    let name = read_modifier_name(
        &mut reader,
        TRUNCATED_U32,
        "truncated particle modifier name",
    )?;
    let order = reader.u32(TRUNCATED_U32)?;
    let target_index = reader.u32(TRUNCATED_U32)?;
    let active = reader.u8("truncated particle active byte")? != 0;
    let data_index = reader.u32(TRUNCATED_U32)?;
    Ok(ParticleColorPending {
        name,
        order,
        target_index,
        active,
        data_index,
        bytes_consumed: reader.position(),
    })
}

/// IDA: 00752CB0 consumes the target link without retaining it; 0075A470 resolves the color link and calls 0075A3F0.
/// Completion: 100% of the target and data bounds and the typed owner's publication.
/// TODO: [ ] Bind the particle-system owner and callback ordering.
pub fn resolve_particle_color_links(
    target: &mut ParticleColorModifier,
    pending: &ParticleColorPending,
    data: Option<(Entity, &dyn ParticleColorKeys)>,
    object_count: usize,
) -> Result<(), StreamError> {
    if pending.target_index != NULL_REFERENCE && pending.target_index as usize >= object_count {
        return Err(StreamError("particle target link index out of range"));
    }
    if pending.data_index != NULL_REFERENCE && pending.data_index as usize >= object_count {
        return Err(StreamError("particle color data link index out of range"));
    }
    if (pending.data_index == NULL_REFERENCE) != data.is_none() {
        return Err(StreamError("particle color data owner/link mismatch"));
    }
    target.base = ParticleModifierBase {
        name_08: pending.name.clone(),
        order_0c: pending.order,
        active_14: pending.active,
    };
    target.set_data(data);
    Ok(())
}

/// IDA: 0075A470 on a stream object; `K` is the color key component, and any other class at the data index fails, as
/// the C++ graph's typed lookup does. Completion: 100%.
pub fn link_particle_color<K: Component + ParticleColorKeys + Clone>(
    world: &mut World,
    entity: Entity,
    pending: &ParticleColorPending,
    table: &StreamObjectTable,
) -> Result<(), StreamError> {
    let mut data = None;
    if pending.data_index != NULL_REFERENCE {
        let entry = table
            .objects
            .get(pending.data_index as usize)
            .ok_or(StreamError("particle color data index out of range"))?;
        let keys = world
            .get::<K>(entry.entity)
            .ok_or(StreamError("particle color link target is not NiColorData"))?;
        data = Some((entry.entity, keys.clone()));
    }
    let mut target = world
        .get_mut::<ParticleColorModifier>(entity)
        .ok_or(StreamError("particle modifier component missing"))?;
    resolve_particle_color_links(
        &mut target,
        pending,
        data.as_ref()
            .map(|held| (held.0, &held.1 as &dyn ParticleColorKeys)),
        table.objects.len(),
    )
}

#[cfg(test)]
mod tests {
    use super::super::color_animation::{ColorKey, ColorKeyType};
    use super::*;

    fn keys(key_type: ColorKeyType, keys: &[(f32, [f32; 4])]) -> ColorKeyData {
        ColorKeyData::new(
            key_type,
            keys.iter()
                .map(|&(time, rgba)| ColorKey { time, rgba })
                .collect(),
        )
        .unwrap()
    }

    fn name_bytes(bytes: &mut Vec<u8>, name: &[u8]) {
        bytes.extend((name.len() as u32).to_le_bytes());
        bytes.extend(name);
    }

    #[test]
    fn payload() {
        let cold = ParticleColorModifier::default();
        assert_eq!(cold.base.name_08, None);
        assert_eq!(cold.base.order_0c, 0);
        assert!(cold.base.active_14);
        assert_eq!(cold.data_18, None);
        assert_eq!(cold.min_time_1c, 0.0);
        assert_eq!(cold.max_time_20, 0.0);
        let mut bytes = Vec::new();
        name_bytes(&mut bytes, b"Tint");
        bytes.extend(17u32.to_le_bytes());
        bytes.extend(2u32.to_le_bytes());
        bytes.push(0x80);
        bytes.extend(3u32.to_le_bytes());
        let payload_size = bytes.len();
        bytes.push(0xAB);
        let pending = parse_particle_color_payload(&bytes, 0x1400_0004).unwrap();
        assert_eq!(pending.name.as_deref(), Some(&b"Tint"[..]));
        assert_eq!(pending.order, 17);
        assert_eq!(pending.target_index, 2);
        assert!(pending.active);
        assert_eq!(pending.data_index, 3);
        assert_eq!(pending.bytes_consumed, payload_size);
        for length in 0..payload_size {
            assert!(parse_particle_color_payload(&bytes[..length], 0x1400_0004).is_err());
        }
        assert!(parse_particle_color_payload(&bytes, 0x0A01_0071).is_err());
    }

    #[test]
    fn links_and_owner() {
        let mut world = World::new();
        let held = world.spawn_empty().id();
        let mut modifier = ParticleColorModifier::default();
        let data = keys(
            ColorKeyType::LINEAR,
            &[(0.25, [1.0, 0.0, 0.0, 1.0]), (0.75, [0.0, 0.0, 1.0, 0.0])],
        );
        let pending = ParticleColorPending {
            name: Some(b"Color".to_vec()),
            order: 9,
            active: false,
            target_index: 1,
            data_index: 2,
            bytes_consumed: 0,
        };
        resolve_particle_color_links(&mut modifier, &pending, Some((held, &data)), 3).unwrap();
        assert_eq!(modifier.base.name_08.as_deref(), Some(&b"Color"[..]));
        assert_eq!(modifier.base.order_0c, 9);
        assert!(!modifier.base.active_14);
        assert_eq!(modifier.data_18, Some(held));
        assert_eq!(modifier.min_time_1c, 0.25);
        assert_eq!(modifier.max_time_20, 0.75);
        let particle = [ParticleColorSample {
            age: 0.5,
            lifespan: 1.0,
        }];
        let mut color = [[0.0; 4]; 1];
        assert!(
            modifier
                .update_colors(&particle, &mut color, Some(&data))
                .unwrap()
        );
        assert_eq!(color[0], [0.5, 0.0, 0.5, 0.5]);

        let empty = ColorKeyData::default();
        modifier.set_data(Some((held, &empty)));
        assert_eq!(modifier.min_time_1c, 0.0);
        assert_eq!(modifier.max_time_20, 0.0);
        modifier.set_data(None);
        assert_eq!(modifier.data_18, None);
        assert_eq!(modifier.min_time_1c, 0.0);
        assert_eq!(modifier.max_time_20, 0.0);

        let mut invalid = ParticleColorPending {
            data_index: 1,
            target_index: 4,
            ..ParticleColorPending::default()
        };
        assert!(
            resolve_particle_color_links(&mut modifier, &invalid, Some((held, &empty)), 3).is_err()
        );
        assert_eq!(modifier.data_18, None);
        invalid.target_index = NULL_REFERENCE;
        invalid.data_index = NULL_REFERENCE;
        assert!(
            resolve_particle_color_links(&mut modifier, &invalid, Some((held, &empty)), 3).is_err()
        );
        assert_eq!(modifier.data_18, None);
    }

    fn sample(age: f32, lifespan: f32) -> ParticleColorSample {
        ParticleColorSample { age, lifespan }
    }

    #[test]
    fn updates() {
        let mut world = World::new();
        let held = world.spawn_empty().id();
        let mut modifier = ParticleColorModifier::default();
        let linear = keys(
            ColorKeyType::LINEAR,
            &[(0.0, [0.0, 0.0, 0.0, 1.0]), (1.0, [1.0, 0.5, 0.25, 0.0])],
        );
        modifier.set_data(Some((held, &linear)));
        let particles = [sample(-1.0, 2.0), sample(1.0, 2.0), sample(3.0, 2.0)];
        let mut colors = [[0.0; 4]; 3];
        assert!(
            modifier
                .update_colors(&particles, &mut colors, Some(&linear))
                .unwrap()
        );
        assert_eq!(colors[0], [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(colors[1], [0.5, 0.25, 0.125, 0.5]);
        assert_eq!(colors[2], [1.0, 0.5, 0.25, 0.0]);
        let step = keys(
            ColorKeyType::STEP,
            &[
                (0.0, [1.0, 0.0, 0.0, 1.0]),
                (0.5, [0.0, 1.0, 0.0, 1.0]),
                (1.0, [0.0, 0.0, 1.0, 1.0]),
            ],
        );
        modifier.set_data(Some((held, &step)));
        let step_particles = [sample(0.49, 1.0), sample(0.5, 1.0), sample(1.0, 1.0)];
        assert!(
            modifier
                .update_colors(&step_particles, &mut colors, Some(&step))
                .unwrap()
        );
        assert_eq!(colors[0], [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(colors[1], [0.0, 1.0, 0.0, 1.0]);
        assert_eq!(colors[2], [0.0, 0.0, 1.0, 1.0]);
        let before = colors;
        let invalid = [sample(0.2, 1.0), sample(0.3, 0.0), sample(0.4, 1.0)];
        assert!(
            modifier
                .update_colors(&invalid, &mut colors, Some(&step))
                .is_err()
        );
        assert_eq!(colors, before);
        assert!(
            modifier
                .update_colors(&particles, &mut colors[..2], Some(&step))
                .is_err()
        );
        assert_eq!(colors, before);
        modifier.set_data(None);
        assert!(
            !modifier
                .update_colors(&particles, &mut colors, None)
                .unwrap()
        );
        assert_eq!(colors, before);
    }
}
