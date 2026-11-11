//! NiBlendAccumTransformInterpolator's Update (006CF780): the accum root's motion, relative to where its sequence started, and
//! the NiQuatTransform inverse and product it composes with.

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;

use super::blend_interpolator::{
    ParticleBlendSampleStatus, ParticleBlendTransformInterpolator, sample_transform_source,
};
use super::controller_sequence::BlendAccumTransformState;
use super::stream_objects::NiClass;
use super::transform_animation::{
    INVALID_TRANSFORM_COMPONENT, QuatTransform, multiply_quaternions, normalize_quaternion,
    quat_transform_invalid, set_quat_transform_rotate, set_quat_transform_scale,
    set_quat_transform_translate,
};
use super::transform_controller::quaternion_to_rotation_matrix;

const NEGATIVE_MAX: f32 = -f32::MAX;

/// IDA: 006CB4D0 NiQuatTransform_Invert(this, out): out's scale 1 / scale (an invalid scale stays invalid, a non-finite inverse
/// leaves out's), the rotation's conjugate through SetRotate and out's rotation as the matrix (an invalid rotation stays invalid,
/// the identity then), and the translate -(matrix * translate) times out's scale (1.0 with an invalid scale) through
/// SetTranslate. `out` is the caller's seed. Completion: 100%.
pub fn invert_quat_transform(value: &QuatTransform, out: &mut QuatTransform) {
    let mut scale = 1.0;
    if value.scale == NEGATIVE_MAX {
        out.scale = NEGATIVE_MAX;
    } else {
        let inverse = 1.0 / value.scale;
        set_quat_transform_scale(out, inverse);
        scale = out.scale;
    }
    let mut matrix = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
    if value.rotate[1] == NEGATIVE_MAX {
        out.rotate[1] = NEGATIVE_MAX;
    } else {
        let conjugate = [
            value.rotate[0],
            -value.rotate[1],
            -value.rotate[2],
            -value.rotate[3],
        ];
        set_quat_transform_rotate(out, &conjugate);
        matrix = quaternion_to_rotation_matrix(&out.rotate);
    }
    if value.translate[0] == NEGATIVE_MAX {
        out.translate[0] = NEGATIVE_MAX;
    } else {
        let negated = [
            -value.translate[0],
            -value.translate[1],
            -value.translate[2],
        ];
        let moved = super::transform::rotate_vector(&matrix, &negated);
        set_quat_transform_translate(out, &[moved[0] * scale, moved[1] * scale, scale * moved[2]]);
    }
}

/// IDA: 006CB640 NiQuatTransform_Multiply(this, out, rhs) = this * rhs from the invalid NiQuatTransform: the scales' product
/// (either invalid: invalid), the rotations' product (00714CF0) normalized (00715340) and this rotation as the matrix (either
/// invalid: invalid rotation, the identity), the translate this + scale * (matrix * rhs) (either invalid: invalid). Completion:
/// 100%.
#[must_use]
pub fn multiply_quat_transforms(lhs: &QuatTransform, rhs: &QuatTransform) -> QuatTransform {
    let mut out = QuatTransform::default();
    let mut scale = 1.0;
    if lhs.scale == NEGATIVE_MAX || rhs.scale == NEGATIVE_MAX {
        out.scale = NEGATIVE_MAX;
    } else {
        set_quat_transform_scale(&mut out, rhs.scale * lhs.scale);
        scale = out.scale;
    }
    let mut matrix = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
    if lhs.rotate[1] == NEGATIVE_MAX || rhs.rotate[1] == NEGATIVE_MAX {
        out.rotate[1] = NEGATIVE_MAX;
    } else {
        let product = normalize_quaternion(&multiply_quaternions(&lhs.rotate, &rhs.rotate));
        set_quat_transform_rotate(&mut out, &product);
        matrix = quaternion_to_rotation_matrix(&lhs.rotate);
    }
    if lhs.translate[0] == NEGATIVE_MAX || rhs.translate[0] == NEGATIVE_MAX {
        out.translate[0] = NEGATIVE_MAX;
    } else {
        let moved = super::transform::rotate_vector(&matrix, &rhs.translate);
        set_quat_transform_translate(
            &mut out,
            &[
                lhs.translate[0] + moved[0] * scale,
                lhs.translate[1] + moved[1] * scale,
                lhs.translate[2] + scale * moved[2],
            ],
        );
    }
    out
}

// IDA: 006D5DB0 NiTransformInterpolator vt+0x80 GetActiveTimeRange(begin, end): begin = FLT_MAX, end = -FLT_MAX; with key
// data (+0x2C): the position keys (+0x0A count, +0x24, stride +0x1D), then the rotation keys (+0x08; for an Euler key (type
// 4, +0x10) its three axes' float keys, else the keys at +0x20, stride +0x1C), then the scale keys (+0x0C, +0x28, stride
// +0x1E): each nonempty array's first key time lowers begin and its last raises end. No data, or no key array before an
// empty scale block, answers 0.0 and 0.0. Completion: 100%.
fn transform_active_time_range(world: &World, source: Entity) -> (f32, f32) {
    use super::transform_animation::{TransformInterpolator, TransformKeyData};
    let mut begin = f32::MAX;
    let mut end = -f32::MAX;
    let mut any = false;
    let take = |first: f32, last: f32, begin: &mut f32, end: &mut f32| {
        if *begin > first {
            *begin = first;
        }
        if *end < last {
            *end = last;
        }
    };
    let Some(data) = world
        .get::<TransformInterpolator>(source)
        .and_then(|interpolator| interpolator.data_2c)
        .and_then(|data| world.get::<TransformKeyData>(data))
    else {
        return (0.0, 0.0);
    };
    let count = usize::from(data.position_count_0a);
    if count != 0 {
        let keys = data.position.keys();
        take(keys[0].time, keys[count - 1].time, &mut begin, &mut end);
        any = true;
    }
    let count = usize::from(data.rotation_count_08);
    if count != 0 {
        if data.rotation_type_10 == 4 {
            if let Some(euler) = data.euler_keys.first() {
                for axis in 0..3 {
                    let axis_count = euler.axis_counts[axis] as usize;
                    if axis_count != 0 {
                        let keys = euler.axes[axis].keys();
                        take(
                            keys[0].time,
                            keys[axis_count - 1].time,
                            &mut begin,
                            &mut end,
                        );
                        any = true;
                    }
                }
            }
        } else {
            take(
                data.rotation_keys[0].time,
                data.rotation_keys[count - 1].time,
                &mut begin,
                &mut end,
            );
            any = true;
        }
    }
    let count = usize::from(data.scale_count_0c);
    if count != 0 {
        let keys = data.scale.keys();
        take(keys[0].time, keys[count - 1].time, &mut begin, &mut end);
    } else if !any {
        return (0.0, 0.0);
    }
    (begin, end)
}

// IDA: a source's vt+0x80 GetActiveTimeRange: NiBSplineInterpolator's 006ED090 answers +0x0C and +0x10 (0 and 0 when the
// caller's two outs already hold FLT_MAX and -FLT_MAX: 006CEE20 passes uninitialized locals, taken as not);
// NiTransformInterpolator's 006D5DB0 (`transform_active_time_range`). Other classes are a stop.
fn active_time_range(world: &World, source: Entity) -> Result<(f32, f32), GameError> {
    match world.get::<NiClass>(source) {
        Some(NiClass::TransformInterpolator) => Ok(transform_active_time_range(world, source)),
        Some(NiClass::BSplineCompTransformInterpolator) => {
            let base = world
                .get::<super::bspline::BSplineCompTransformInterpolator>(source)
                .expect("a B-spline interpolator")
                .base;
            Ok((base.start_time_0c, base.end_time_10))
        }
        _ => Err(GameError::unclassified(
            0x006CEE6E,
            "006CEE6E: a source's vt+0x80 time range",
        )),
    }
}

// IDA: N/A a source's vt+0x4C Update as `sample_transform_source` answers it.
fn source_update(
    world: &mut World,
    source: Entity,
    time: f32,
    out: &mut QuatTransform,
) -> Result<bool, GameError> {
    match sample_transform_source(world, source, time, out) {
        ParticleBlendSampleStatus::Sampled => Ok(true),
        ParticleBlendSampleStatus::Failed => Ok(false),
        _ => Err(GameError::unclassified(
            0x006CEEF5,
            "006CEEF5: an accum source's Update not ported",
        )),
    }
}

/// IDA: 006CEE20 NiBlendAccumTransformInterpolator_UpdateSingleItem(time, owner) on the +0x0F item: a time different from the
/// accum item's +0x00 (both ordered) that went back (the sequence looped): with no cycle delta at +0x24, the source's value at
/// its range's end times the inverse of its value at the start; a valid +0x30 takes +0x30 * delta. Then the source's Update
/// into the item's +0x04. +0x00 takes the time either way. Completion: 100%.
fn update_single_item(
    world: &mut World,
    blend: &ParticleBlendTransformInterpolator,
    state: &mut BlendAccumTransformState,
    time: f32,
) -> Result<(), GameError> {
    let index = usize::from(blend.base.single_index_0f);
    let source =
        blend
            .item_sources
            .get(index)
            .copied()
            .flatten()
            .ok_or(GameError::unclassified(
                0x006CEE36,
                "006CEE36: the single item's null interpolator faults",
            ))?;
    let last = state
        .items_50
        .get(index)
        .ok_or(GameError::unclassified(
            0x006CEE36,
            "006CEE36: no accum item",
        ))?
        .last_time_00;
    if time != last && !time.is_nan() && !last.is_nan() {
        if last > time {
            if quat_transform_invalid(&state.items_50[index].cycle_delta_24) {
                let (begin, end) = active_time_range(world, source)?;
                let mut at_begin = QuatTransform::default();
                source_update(world, source, begin, &mut at_begin)?;
                let mut at_end = QuatTransform::default();
                source_update(world, source, end, &mut at_end)?;
                let mut inverse = QuatTransform::default();
                invert_quat_transform(&at_begin, &mut inverse);
                state.items_50[index].cycle_delta_24 = multiply_quat_transforms(&at_end, &inverse);
            }
            if !quat_transform_invalid(&state.value_30) {
                state.value_30 = multiply_quat_transforms(
                    &state.value_30,
                    &state.items_50[index].cycle_delta_24,
                );
            }
        }
        let mut value = state.items_50[index].value_04;
        source_update(world, source, time, &mut value)?;
        state.items_50[index].value_04 = value;
    }
    state.items_50[index].last_time_00 = time;
    Ok(())
}

/// IDA: 006CF780 NiBlendAccumTransformInterpolator vt+0x4C Update(time, owner, out): one active item takes 006CF630 (the
/// manager flag's +0x20 single time; -FLT_MAX answers false; 006CEE20; with +0x54 set or +0x30 invalid, +0x30 becomes the
/// inverse of the item's value (006CB4D0 from the invalid seed), or +0x30 times it, and +0x54 clears; an invalid +0x30 answers
/// false; out = +0x30 * the item's value). No active item answers false. Either stores +0x08. Completion: 100% of those arms;
/// several active items (0045ED10's weights, 006CEF80 per item, 006CE840) are a stop.
/// # Errors
/// Several active items, or a stop in a source.
pub fn update_blend_accum(
    world: &mut World,
    interpolator: Entity,
    time: f32,
    out: &mut QuatTransform,
) -> Result<bool, GameError> {
    let blend = world
        .get::<ParticleBlendTransformInterpolator>(interpolator)
        .ok_or(GameError::unclassified(
            0x006CF780,
            "006CF780: not a blend interpolator",
        ))?
        .clone();
    let mut state = world
        .get::<BlendAccumTransformState>(interpolator)
        .ok_or(GameError::unclassified(
            0x006CF780,
            "006CF780: not a NiBlendAccumTransformInterpolator",
        ))?
        .clone();
    let result = match blend.base.active_count_0e {
        0 => false,
        1 => {
            let time = if blend.base.flags_0c & 1 != 0 {
                blend.base.single_time_20
            } else {
                time
            };
            if time == NEGATIVE_MAX {
                return Ok(false);
            }
            update_single_item(world, &blend, &mut state, time)?;
            let index = usize::from(blend.base.single_index_0f);
            if state.reset_54 != 0 || quat_transform_invalid(&state.value_30) {
                let mut inverse = QuatTransform {
                    translate: [INVALID_TRANSFORM_COMPONENT; 3],
                    rotate: [INVALID_TRANSFORM_COMPONENT; 4],
                    scale: INVALID_TRANSFORM_COMPONENT,
                };
                invert_quat_transform(&state.items_50[index].value_04, &mut inverse);
                state.value_30 = if quat_transform_invalid(&state.value_30) {
                    inverse
                } else {
                    multiply_quat_transforms(&state.value_30, &inverse)
                };
                state.reset_54 = 0;
            }
            if quat_transform_invalid(&state.value_30) {
                false
            } else {
                *out = multiply_quat_transforms(&state.value_30, &state.items_50[index].value_04);
                true
            }
        }
        _ => {
            return Err(GameError::unclassified(
                0x006CF7A8,
                "006CF7A8: several active accum items (006CEF80, 006CE840)",
            ));
        }
    };
    *world
        .get_mut::<BlendAccumTransformState>(interpolator)
        .expect("read above") = state;
    world
        .get_mut::<ParticleBlendTransformInterpolator>(interpolator)
        .expect("read above")
        .base
        .last_time_08 = time;
    Ok(result)
}

/// IDA: 00471640 (thiscall interpolator), a NiBlendAccumTransformInterpolator's value (+0x30) reset: with one active item
/// (+0x0E == 1) the invalid transform (00B24260's -FLT_MAX translate, 00B3CBA4's NaN rotation, 00A79E10's -FLT_MAX scale);
/// otherwise a zero translate, the identity rotation (00B27110) and scale 1.0 (stored when finite, which 1.0 is); then
/// +0x54 = 1. Completion: 100%.
/// # Errors
/// An interpolator with no blend base.
pub fn reset_blend_accum(world: &mut World, interpolator: Entity) -> Result<(), GameError> {
    let active = world
        .get::<crate::gamebryo::blend_interpolator::ParticleBlendTransformInterpolator>(
            interpolator,
        )
        .ok_or(GameError::unclassified(
            0x00471640,
            "00471640: an accum interpolator with no blend base",
        ))?
        .base
        .active_count_0e;
    let mut state = world
        .get_mut::<crate::gamebryo::controller_sequence::BlendAccumTransformState>(interpolator)
        .expect("tested by the caller");
    if active == 1 {
        state.value_30.translate = [f32::from_bits(0xFF7F_FFFF); 3];
        state.value_30.rotate = [f32::from_bits(0xFFFF_FFFF); 4];
        state.value_30.scale = f32::from_bits(0xFF7F_FFFF);
    } else {
        state.value_30.translate = [0.0; 3];
        state.value_30.rotate = [1.0, 0.0, 0.0, 0.0];
        state.value_30.scale = 1.0;
    }
    state.reset_54 = 1;
    Ok(())
}
