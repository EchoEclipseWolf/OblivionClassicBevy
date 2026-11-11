//! NiMorphData (006DDF90) and NiGeomMorpherController (006D1520), as the stream gives them. Load and link only: the
//! controller's Update (vtable +0x54) isn't ported, so a scene update that reaches one stops.

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;
use oblivion_core::reader::ByteReader;

use super::particle_controller::{ParticleTimeControllerCold, ParticleTimeControllerPayload};
use super::stream_objects::{NULL_REFERENCE, NiClass, StreamObjectTable};

const TRUNCATED: &str = "truncated morph payload";

/// IDA: one NiMorphData::MorphTarget (0x0C): +0x00 the vectors, +0x04 the name (006DE2F0).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MorphTarget {
    pub name: Option<Vec<u8>>,
    pub vector_bits: Vec<[u32; 3]>,
}

/// IDA: 006DDF90 NiMorphData: +0x08 the morph count, +0x0C the vertex count, +0x10 the targets, +0x14 relative targets.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct MorphData {
    pub vertex_count_0c: u32,
    pub targets_10: Vec<MorphTarget>,
    pub relative_targets_14: bool,
}

/// IDA: 006DE410 NiMorphData_LoadBinary at 0A010068 and above: 007008A0, the u32 morph and vertex counts, a byte (relative
/// targets when 1), then per morph 006DE2F0: NiStream_ReadOwnedCString's name (a nonpositive length is null), a dropped u32
/// when NiStream +0x04 is below 10, and the vertex count's 12-byte vectors. Completion: 100% at the floor.
pub fn parse_morph_data_payload(
    bytes: &[u8],
    version: u32,
    file_user_version: u32,
) -> Result<(MorphData, usize), GameError> {
    if version < 0x0A01_0068 {
        return Err(GameError::unclassified(
            0,
            "legacy NiMorphData payload unsupported",
        ));
    }
    let mut reader = ByteReader::new(bytes);
    let morphs = reader.u32(TRUNCATED)?;
    let vertex_count_0c = reader.u32(TRUNCATED)?;
    let relative_targets_14 = reader.u8(TRUNCATED)? == 1;
    let mut targets_10 = Vec::new();
    for _ in 0..morphs {
        let length = reader.u32(TRUNCATED)? as i32;
        let name = if length <= 0 {
            None
        } else {
            Some(reader.take(length as usize, TRUNCATED)?.to_vec())
        };
        if file_user_version < 10 {
            let _ = reader.u32(TRUNCATED)?;
        }
        let mut vector_bits = Vec::new();
        for _ in 0..vertex_count_0c {
            vector_bits.push([
                reader.u32(TRUNCATED)?,
                reader.u32(TRUNCATED)?,
                reader.u32(TRUNCATED)?,
            ]);
        }
        targets_10.push(MorphTarget { name, vector_bits });
    }
    Ok((
        MorphData {
            vertex_count_0c,
            targets_10,
            relative_targets_14,
        },
        reader.position(),
    ))
}

/// IDA: 006D1520 NiGeomMorpherController (0x5C): the NiInterpController, +0x3C the extra flags word, +0x40 the
/// NiTArray<float> of weights, +0x50 the NiMorphData, +0x54 the interpolator slots (006D10F0), +0x5A always-update.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct GeomMorpherController {
    pub time: ParticleTimeControllerCold,
    pub extra_flags_3c: u16,
    pub weights_40: Vec<f32>,
    pub data_50: Option<Entity>,
    pub interpolators_54: Vec<Option<Entity>>,
    pub always_update_5a: bool,
    pub links_resolved: bool,
}

/// IDA: what 006D15D0 still needs: the time links, the data and the interpolator list.
#[derive(Clone, Debug, PartialEq)]
pub struct GeomMorpherPayload {
    pub time: ParticleTimeControllerPayload,
    pub data_index: u32,
    pub interpolator_indices: Vec<u32>,
    pub bytes_consumed: usize,
}

/// IDA: 006D0D30 NiGeomMorpherController_LoadBinary at 10.1.0.106 and the modern floor: NiInterpController_LoadBinary (00715F40's time
/// fields), the u16 at +0x3C (0A000102 and above), the data reference, the always-update byte (04000002 and above), the
/// interpolator reference list (0A010068 and above), and with NiStream +0x04 at 10 or more a u32 count of words that 004CA040
/// and 004CA210 put into +0x40 as floats, where the link's 006D10F0 replaces them. Completion: 100% at the floor.
pub fn load_geom_morpher_controller(
    world: &mut World,
    entity: Entity,
    bytes: &[u8],
    version: u32,
    file_user_version: u32,
) -> Result<GeomMorpherPayload, GameError> {
    if version < 0x0A01_006D && version != 0x0A01_006A {
        return Err(GameError::unclassified(
            0,
            "legacy NiGeomMorpherController payload unsupported",
        ));
    }
    let time = super::transform_controller::parse_interp_controller_prefix(bytes, version)?;
    let mut reader = ByteReader::new(&bytes[time.bytes_consumed..]);
    let extra_flags_3c = reader.u16(TRUNCATED)?;
    let data_index = reader.u32(TRUNCATED)?;
    let always_update_5a = reader.u8(TRUNCATED)? != 0;
    let count = reader.u32(TRUNCATED)?;
    let mut interpolator_indices = Vec::new();
    for _ in 0..count {
        interpolator_indices.push(reader.u32(TRUNCATED)?);
    }
    let mut weights_40 = Vec::new();
    if file_user_version >= 10 {
        let words = reader.u32(TRUNCATED)?;
        for _ in 0..words {
            weights_40.push(f32::from_bits(reader.u32(TRUNCATED)?));
        }
    }
    let mut controller =
        world
            .get_mut::<GeomMorpherController>(entity)
            .ok_or(GameError::unclassified(
                0,
                "geom morpher load target missing",
            ))?;
    controller.time.apply_loaded_payload(&time);
    controller.extra_flags_3c = extra_flags_3c;
    controller.always_update_5a = always_update_5a;
    controller.weights_40 = weights_40;
    Ok(GeomMorpherPayload {
        time,
        data_index,
        interpolator_indices,
        bytes_consumed: time.bytes_consumed + reader.position(),
    })
}

/// IDA: 006D15D0 NiGeomMorpherController_LinkObject at 0A010068 and above: 00715E70's next and target, the NiMorphData into
/// +0x50, 006D10F0 (the +0x54 slots for its morph count, +0x40 zeroed to that count), then the list's count resizes +0x40
/// again (004CA040, zeros) and each interpolator goes in through vt+0x84 SetInterpolator(i). The original checks no classes;
/// the host takes a geometry target, NiMorphData and float interpolators, and stops on anything else (`Ok(false)`).
/// Completion: 100% of the modern link; vt+0x84's own bounds are taken as the slot count.
pub fn link_geom_morpher_controller(
    world: &mut World,
    entity: Entity,
    payload: &GeomMorpherPayload,
    table: &StreamObjectTable,
) -> Result<bool, GameError> {
    let resolve = |index: u32| -> Result<Option<Entity>, GameError> {
        if index == NULL_REFERENCE {
            return Ok(None);
        }
        Ok(Some(
            table
                .objects
                .get(index as usize)
                .ok_or(GameError::unclassified(0, "geom morpher link out of range"))?
                .entity,
        ))
    };
    let next = resolve(payload.time.next_index)?;
    let target = resolve(payload.time.target_index)?;
    let data = resolve(payload.data_index)?;
    if next.is_some_and(|next| {
        !world
            .get::<NiClass>(next)
            .is_some_and(|class| class.is_controller_head())
    }) || target.is_some_and(|target| {
        !matches!(
            world.get::<NiClass>(target),
            Some(NiClass::TriShape | NiClass::TriStrips)
        )
    }) || data.is_some_and(|data| world.get::<NiClass>(data) != Some(&NiClass::MorphData))
    {
        return Ok(false);
    }
    let morphs = data
        .and_then(|data| world.get::<MorphData>(data))
        .map_or(0, |data| data.targets_10.len() as u16);
    let mut interpolators = Vec::new();
    for &index in &payload.interpolator_indices {
        let interpolator = resolve(index)?;
        if interpolator.is_some_and(|interpolator| {
            !world
                .get::<NiClass>(interpolator)
                .is_some_and(|class| class.is_blend_float_source())
        }) {
            return Ok(false);
        }
        interpolators.push(interpolator);
    }
    let mut controller =
        world
            .get_mut::<GeomMorpherController>(entity)
            .ok_or(GameError::unclassified(
                0,
                "geom morpher link target missing",
            ))?;
    controller.time.next_34 = next;
    controller.time.next_linked = next.is_some();
    controller.time.target_30 = target;
    controller.time.target_linked = target.is_some();
    controller.data_50 = data;
    controller.interpolators_54 = vec![None; usize::from(morphs)];
    // 006D1699 skips the second resize when the deferred list is empty.
    let weights = if interpolators.is_empty() {
        usize::from(morphs)
    } else {
        interpolators.len()
    };
    controller.weights_40 = vec![0.0; weights];
    for (slot, interpolator) in interpolators.into_iter().enumerate() {
        if let Some(entry) = controller.interpolators_54.get_mut(slot) {
            *entry = interpolator;
        }
    }
    controller.links_resolved = true;
    Ok(true)
}
