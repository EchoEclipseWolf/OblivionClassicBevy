//! The NiMaterialProperty controllers and their Point3 interpolators, as the stream gives them: NiAlphaController (006D23D0),
//! NiMaterialColorController (006DEF40), NiPoint3Interpolator (006DA740) and NiBlendPoint3Interpolator (006EA8C0).
//! Load and link only; their Updates (vtable +0x54, +0x5C) aren't ported, so a scene update that reaches one stops.

use bevy_ecs::prelude::*;
use recomp_core::reader::{ByteReader, StreamError};

use super::particle_blend_interpolator::{ParticleBlendCold, ParticleBlendPayload};
use super::particle_controller::ParticleTimeControllerCold;
use super::stream_objects::{NULL_REFERENCE, NiClass, StreamObjectTable};
use super::transform_controller::{
    SingleInterpControllerPayload, parse_single_interp_controller_payload,
};

const TRUNCATED: &str = "truncated material controller payload";

/// IDA: which NiMaterialProperty controller a component is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaterialControllerKind {
    /// NiAlphaController (vtable A7A3EC): a NiFloatInterpController.
    Alpha,
    /// NiMaterialColorController (vtable A7B024): a NiPoint3InterpController with the +0x40 target color word.
    Color { target_color_40: u16 },
}

/// IDA: the two controllers' state: the NiTimeController fields, +0x3C the interpolator NiSingleInterpController_LoadBinary
/// (006CE320) reads as an immediate reference, and the kind.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct MaterialController {
    pub kind: MaterialControllerKind,
    pub time: ParticleTimeControllerCold,
    pub interpolator_3c: Option<Entity>,
    pub links_resolved: bool,
}

impl MaterialController {
    /// IDA: N/A a cold controller of a kind.
    #[must_use]
    pub fn new(kind: MaterialControllerKind) -> Self {
        Self {
            kind,
            time: ParticleTimeControllerCold::default(),
            interpolator_3c: None,
            links_resolved: false,
        }
    }
}

/// IDA: what the controllers' LinkObject still needs.
#[derive(Clone, Debug, PartialEq)]
pub struct MaterialControllerPayload {
    pub base: SingleInterpControllerPayload,
    pub bytes_consumed: usize,
}

/// IDA: 006D2250 (NiAlphaController) and 006DEDD0 (NiMaterialColorController) LoadBinary: 006CE320 (00715F40's time fields,
/// the interpolator reference), then the color controller's u16 target color at 0A000102 and above; below 0A010068 each reads a
/// legacy data reference, outside the floor. `interpolator_float` says which sources the interpolator may be: the alpha
/// controller's float ones, the color controller's Point3 ones. The original assigns +0x3C with no type check; another class
/// is a host stop. Completion: 100% at 0A01006D and above.
pub fn load_material_controller(
    world: &mut World,
    entity: Entity,
    bytes: &[u8],
    version: u32,
    table: &StreamObjectTable,
) -> Result<Option<MaterialControllerPayload>, StreamError> {
    let base = parse_single_interp_controller_payload(bytes, version)?;
    let mut reader = ByteReader::new(&bytes[base.bytes_consumed..]);
    let alpha = world.get::<NiClass>(entity) == Some(&NiClass::AlphaController);
    let target_color = if alpha {
        None
    } else {
        Some(reader.u16(TRUNCATED)?)
    };
    let mut interpolator = None;
    if base.interpolator_index != NULL_REFERENCE {
        let other = table
            .objects
            .get(base.interpolator_index as usize)
            .ok_or(StreamError("material controller interpolator out of range"))?
            .entity;
        let class = world.get::<NiClass>(other).copied();
        let accepted = if alpha {
            class.is_some_and(NiClass::is_blend_float_source)
        } else {
            matches!(
                class,
                Some(NiClass::Point3Interpolator | NiClass::BlendPoint3Interpolator)
            )
        };
        if !accepted {
            return Ok(None);
        }
        interpolator = Some(other);
    }
    let mut controller = world
        .get_mut::<MaterialController>(entity)
        .ok_or(StreamError("material controller load target missing"))?;
    controller.time.apply_loaded_payload(&base.time);
    controller.interpolator_3c = interpolator;
    if let Some(target_color_40) = target_color {
        controller.kind = MaterialControllerKind::Color { target_color_40 };
    }
    let bytes_consumed = base.bytes_consumed + reader.position();
    Ok(Some(MaterialControllerPayload {
        base,
        bytes_consumed,
    }))
}

/// IDA: 006E3250 / 006DED20 LinkObject at 0A010068 and above: 00715E70, the next controller (strong) first, then the raw
/// target. The original has no type checks; the host takes a next that is a material controller and a NiMaterialProperty
/// target, and stops on anything else (`Ok(false)`). Completion: 100% of the modern link.
pub fn link_material_controller(
    world: &mut World,
    entity: Entity,
    payload: &MaterialControllerPayload,
    table: &StreamObjectTable,
) -> Result<bool, StreamError> {
    let resolve = |index: u32| -> Result<Option<Entity>, StreamError> {
        if index == NULL_REFERENCE {
            return Ok(None);
        }
        Ok(Some(
            table
                .objects
                .get(index as usize)
                .ok_or(StreamError("material controller link out of range"))?
                .entity,
        ))
    };
    let next = resolve(payload.base.time.next_index)?;
    let target = resolve(payload.base.time.target_index)?;
    if next.is_some_and(|next| {
        !matches!(
            world.get::<NiClass>(next),
            Some(NiClass::AlphaController | NiClass::MaterialColorController)
        )
    }) || target
        .is_some_and(|target| world.get::<NiClass>(target) != Some(&NiClass::MaterialProperty))
    {
        return Ok(false);
    }
    let mut controller = world
        .get_mut::<MaterialController>(entity)
        .ok_or(StreamError("material controller link target missing"))?;
    controller.time.next_34 = next;
    controller.time.next_linked = next.is_some();
    controller.time.target_30 = target;
    controller.time.target_linked = target.is_some();
    controller.links_resolved = true;
    Ok(true)
}

/// IDA: 00B24FC8 g_niPoint3InvalidPoint, -FLT_MAX in each component.
pub const INVALID_POINT_BITS: u32 = 0xFF7F_FFFF;

/// IDA: 006DA740 NiPoint3Interpolator: +0x0C the value (cold the invalid point), +0x18 the NiPosData.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct Point3Interpolator {
    pub value_bits_0c: [u32; 3],
    pub data_18: Option<Entity>,
}

impl Default for Point3Interpolator {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            value_bits_0c: [INVALID_POINT_BITS; 3],
            data_18: None,
        }
    }
}

/// IDA: 006DA7E0 NiPoint3Interpolator_LoadBinary: NiKeyBasedInterpolator_LoadBinary (007008A0, nothing at the floor),
/// NiVector3_LoadBinary into +0x0C, then the NiPosData through NiStream_ReadObjectReference into +0x18. The original assigns
/// with no type check; another class is a host stop. Completion: 100% at the floor.
pub fn load_point3_interpolator(
    world: &mut World,
    entity: Entity,
    bytes: &[u8],
    version: u32,
    table: &StreamObjectTable,
) -> Result<Option<usize>, StreamError> {
    if version < 0x0A01_0072 {
        return Err(StreamError(
            "legacy NiPoint3Interpolator payload unsupported",
        ));
    }
    let mut reader = ByteReader::new(bytes);
    let value_bits_0c = [
        reader.u32(TRUNCATED)?,
        reader.u32(TRUNCATED)?,
        reader.u32(TRUNCATED)?,
    ];
    let index = reader.u32(TRUNCATED)?;
    let mut data_18 = None;
    if index != NULL_REFERENCE {
        let other = table
            .objects
            .get(index as usize)
            .ok_or(StreamError("Point3 interpolator data out of range"))?
            .entity;
        if world.get::<NiClass>(other) != Some(&NiClass::PositionKeyData) {
            return Ok(None);
        }
        data_18 = Some(other);
    }
    *world
        .get_mut::<Point3Interpolator>(entity)
        .ok_or(StreamError("Point3 interpolator load target missing"))? = Point3Interpolator {
        value_bits_0c,
        data_18,
    };
    Ok(Some(reader.position()))
}

/// IDA: 006EA8C0 NiBlendPoint3Interpolator: the NiBlendInterpolator base, +0x30 the value, +0x3C the valid flag.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct BlendPoint3Interpolator {
    pub base: ParticleBlendCold,
    pub value_bits_30: [u32; 3],
    pub valid_3c: bool,
    pub item_sources: Vec<Option<Entity>>,
    pub single_source_18: Option<Entity>,
}

/// IDA: 006EAD00 NiBlendPoint3Interpolator_LoadBinary: NiBlendInterpolator_LoadBinary (006CDC10, the items' references
/// immediate), NiVector3_LoadBinary into +0x30, and +0x3C = 1 when the value isn't g_niPoint3InvalidPoint (exact float
/// compares). Items and the single source take Point3 interpolators; another class is a host stop. Completion: 100% at the
/// floor.
pub fn load_blend_point3_interpolator(
    world: &mut World,
    entity: Entity,
    payload: &ParticleBlendPayload<[u32; 3]>,
    table: &StreamObjectTable,
    invalid_point: [f32; 3],
) -> Result<bool, StreamError> {
    let resolve = |index: u32| -> Result<Option<Option<Entity>>, StreamError> {
        if index == NULL_REFERENCE {
            return Ok(Some(None));
        }
        let other = table
            .objects
            .get(index as usize)
            .ok_or(StreamError("Point3 blend item out of range"))?
            .entity;
        Ok(matches!(
            world.get::<NiClass>(other),
            Some(NiClass::Point3Interpolator | NiClass::BlendPoint3Interpolator)
        )
        .then_some(Some(other)))
    };
    let mut sources = Vec::new();
    for item in &payload.items {
        let Some(source) = resolve(item.interpolator_index)? else {
            return Ok(false);
        };
        sources.push(source);
    }
    let Some(single) = resolve(payload.single_interpolator_index)? else {
        return Ok(false);
    };
    let value = payload.value.map(f32::from_bits);
    let valid = value[0] != invalid_point[0]
        || value[1] != invalid_point[1]
        || value[2] != invalid_point[2];
    let mut owner = world
        .get_mut::<BlendPoint3Interpolator>(entity)
        .ok_or(StreamError("Point3 blend load target missing"))?;
    owner.base.apply_loaded_prefix(payload);
    owner.value_bits_30 = payload.value;
    if valid {
        owner.valid_3c = true;
    }
    if owner.base.flags_0c & 1 == 0 {
        owner.item_sources = sources;
        owner.single_source_18 = single;
    }
    Ok(true)
}
