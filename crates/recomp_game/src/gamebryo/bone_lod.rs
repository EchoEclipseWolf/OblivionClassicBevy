//! A skeleton's node controllers: NiBSBoneLODController (006FD6E0), the bone LOD groups, whose Update (vt+0x54) does nothing, and
//! bhkBlendController (008AA920), a bone's rigid-body blend.

use bevy_ecs::prelude::*;
use recomp_core::reader::{ByteReader, StreamError};

use super::particle_controller::{
    ParticleTimeControllerPayload, parse_particle_time_controller_payload,
};
use super::scene_node::SceneNodePayload;
use super::stream_objects::{NULL_REFERENCE, StreamObjectTable};
use super::transform_controller::{NodeController, SingleInterpControllerPayload};

/// IDA: 006FD530 NiBSBoneLODController's constructor (0x54, vtable A7D3D4) over 00715F60 NiTimeController: +0x3C the LOD (-1),
/// +0x40 the LOD count (0), +0x44 a NiTArray<NiTSet<NiNode*>*> (vtable at +0x44, grow-by 1), one node set per group.
#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct BoneLodController {
    pub lod_3c: i32,
    pub lod_count_40: u32,
    /// Each group's nodes, in stream order; a null reference stays null.
    pub groups_44: Vec<Vec<Option<Entity>>>,
}

impl Default for BoneLodController {
    /// IDA: N/A 006FD530's stores.
    fn default() -> Self {
        Self {
            lod_3c: -1,
            lod_count_40: 0,
            groups_44: Vec::new(),
        }
    }
}

/// IDA: what 006FDC30 reads past NiTimeController_LoadBinary: the +0x3C and +0x40 words, a group count, and per group a node
/// count and that many deferred references (NiStream_ReadDeferredReference).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BoneLodPayload {
    pub time: ParticleTimeControllerPayload,
    pub lod_3c: u32,
    pub lod_count_40: u32,
    pub groups: Vec<Vec<u32>>,
    pub bytes_consumed: usize,
}

/// IDA: 006FDC30 NiBSBoneLODController_LoadBinary. Completion: 100% at the modern floor.
/// # Errors
/// A truncated payload, or the time controller prefix's errors.
pub fn parse_bone_lod_payload(bytes: &[u8], version: u32) -> Result<BoneLodPayload, StreamError> {
    const TRUNCATED: &str = "truncated NiBSBoneLODController";
    let time = parse_particle_time_controller_payload(bytes, version)?;
    let mut reader = ByteReader::new(&bytes[time.bytes_consumed..]);
    let lod_3c = reader.u32(TRUNCATED)?;
    let lod_count_40 = reader.u32(TRUNCATED)?;
    let count = reader.u32(TRUNCATED)?;
    let mut groups = Vec::new();
    for _ in 0..count {
        let nodes = reader.u32(TRUNCATED)?;
        let mut group = Vec::new();
        for _ in 0..nodes {
            group.push(reader.u32(TRUNCATED)?);
        }
        groups.push(group);
    }
    let bytes_consumed = time.bytes_consumed + reader.position();
    Ok(BoneLodPayload {
        time,
        lod_3c,
        lod_count_40,
        groups,
        bytes_consumed,
    })
}

/// IDA: 006FDC30's stores on the controller: the NiTimeController prefix (00715F40), +0x3C, +0x40; the groups wait for the link.
/// # Errors
/// A controller entity without its components.
pub fn load_bone_lod_controller(
    world: &mut World,
    controller: Entity,
    payload: &BoneLodPayload,
) -> Result<(), StreamError> {
    world
        .get_mut::<NodeController>(controller)
        .ok_or(StreamError("NiBSBoneLODController load target missing"))?
        .apply_loaded_prefix(&SingleInterpControllerPayload {
            time: payload.time,
            interpolator_index: NULL_REFERENCE,
            bytes_consumed: payload.time.bytes_consumed,
        });
    let mut owner = world
        .get_mut::<BoneLodController>(controller)
        .ok_or(StreamError("NiBSBoneLODController load target missing"))?;
    owner.lod_3c = payload.lod_3c as i32;
    owner.lod_count_40 = payload.lod_count_40;
    Ok(())
}

/// IDA: 006FD750 NiBSBoneLODController_LinkObject, after NiTimeController_LinkObject (the caller's 00715E70): each group's
/// references resolved in order (NiStream_ResolveNextDeferredReference). Completion: 100%.
/// # Errors
/// A reference outside the table.
pub fn link_bone_lod_groups(
    world: &mut World,
    controller: Entity,
    payload: &BoneLodPayload,
    table: &StreamObjectTable,
) -> Result<(), StreamError> {
    let mut groups = Vec::with_capacity(payload.groups.len());
    for group in &payload.groups {
        let mut nodes = Vec::with_capacity(group.len());
        for &index in group {
            if index == NULL_REFERENCE {
                nodes.push(None);
                continue;
            }
            let entry = table.objects.get(index as usize).ok_or(StreamError(
                "NiBSBoneLODController node reference out of range",
            ))?;
            nodes.push(Some(entry.entity));
        }
        groups.push(nodes);
    }
    world
        .get_mut::<BoneLodController>(controller)
        .ok_or(StreamError("NiBSBoneLODController link target missing"))?
        .groups_44 = groups;
    Ok(())
}

/// IDA: 006FD7B0 NiBSBoneLODController_PostLoad: the loaded +0x3C taken and reset to -1, then 006FD5D0 SetLOD with it. Completion:
/// 100%.
pub fn post_load_bone_lod(world: &mut World, controller: Entity) {
    let lod = {
        let mut owner = world
            .get_mut::<BoneLodController>(controller)
            .expect("a NiBSBoneLODController");
        let lod = owner.lod_3c;
        owner.lod_3c = -1;
        lod
    };
    set_bone_lod(world, controller, lod);
}

/// IDA: 006FD5D0 NiBSBoneLODController_SetLOD(lod): no count, the current LOD, or one outside [-1, count) answers false. Otherwise
/// each group from the first: from group `lod` on, each node's flags +0x18 take bit 1, the groups before it lose it; +0x3C =
/// lod. Completion: 100%.
pub fn set_bone_lod(world: &mut World, controller: Entity, lod: i32) -> bool {
    let owner = world
        .get::<BoneLodController>(controller)
        .expect("a NiBSBoneLODController")
        .clone();
    let count = owner.lod_count_40;
    if count == 0 || owner.lod_3c == lod || (lod != -1 && (lod <= -1 || lod as u32 >= count)) {
        return false;
    }
    let mut reached = false;
    for (index, group) in owner.groups_44.iter().enumerate() {
        if index as i32 == lod {
            reached = true;
        }
        for node in group.iter().flatten() {
            if let Some(mut payload) = world.get_mut::<SceneNodePayload>(*node) {
                if reached {
                    payload.effective_av_flags |= 2;
                } else {
                    payload.effective_av_flags &= !2;
                }
            }
        }
    }
    world
        .get_mut::<BoneLodController>(controller)
        .expect("a NiBSBoneLODController")
        .lod_3c = lod;
    true
}

/// IDA: 008AA920 bhkBlendController_Create (0x64): 008AA810 over 00715F60 NiTimeController (vtable A97AC4): +0x3C 0, +0x40 a
/// NiTArray of 12-byte keys (vtable A97ABC, data +0x44, grow-by +0x54 = 4), +0x58 and +0x5C -1.0, +0x60 0. Its PostLoad is a
/// no-op; its Update is `update_bhk_blend_controller` (008AAD60). Completion: the cold state, load, link and Update but its
/// clamp end.
#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct BhkBlendController {
    /// +0x3C the last key 008AA990 found (its search starts there).
    pub last_key_3c: u32,
    /// Each key's three floats as raw bits.
    pub keys_44: Vec<[u32; 3]>,
    /// +0x58 / +0x5C the collision object's two blend floats before the first Update (-1.0 until then; 008AA420 restores them).
    pub saved_blend_58: u32,
    pub saved_blend_5c: u32,
    pub field_60: u32,
}

impl Default for BhkBlendController {
    /// IDA: N/A 008AA810's stores.
    fn default() -> Self {
        Self {
            last_key_3c: 0,
            keys_44: Vec::new(),
            saved_blend_58: (-1.0_f32).to_bits(),
            saved_blend_5c: (-1.0_f32).to_bits(),
            field_60: 0,
        }
    }
}

/// IDA: what 008AB7B0 reads past NiTimeController_LoadBinary: the key count (008AA480 sizes the array).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BhkBlendPayload {
    pub time: ParticleTimeControllerPayload,
    pub key_count: u32,
    pub bytes_consumed: usize,
}

/// IDA: 008AB7B0 bhkBlendController_LoadBinary: the NiTimeController prefix, a u32 count; each key {-1.0, 0.0, 0.0} goes through
/// the stream's read with a 12-byte record and an item count of 0, then 008AA710 appends it, +0x3C = 0 and 008AABE0 runs. A
/// nonzero count is a stop here: what that read consumes isn't checked against a file. Completion: 100% of no keys.
/// # Errors
/// A truncated payload, keys, or the prefix's errors.
pub fn parse_bhk_blend_payload(bytes: &[u8], version: u32) -> Result<BhkBlendPayload, StreamError> {
    let time = parse_particle_time_controller_payload(bytes, version)?;
    let mut reader = ByteReader::new(&bytes[time.bytes_consumed..]);
    let key_count = reader.u32("truncated bhkBlendController")?;
    if key_count != 0 {
        return Err(StreamError("008AB7F0: a bhkBlendController with keys"));
    }
    let bytes_consumed = time.bytes_consumed + reader.position();
    Ok(BhkBlendPayload {
        time,
        key_count,
        bytes_consumed,
    })
}

/// IDA: 008AB7B0's prefix store (00715F40) on the controller; the next and target links are NiTimeController_LinkObject's.
/// # Errors
/// A controller entity without its components.
pub fn load_bhk_blend_controller(
    world: &mut World,
    controller: Entity,
    payload: &BhkBlendPayload,
) -> Result<(), StreamError> {
    world
        .get_mut::<NodeController>(controller)
        .ok_or(StreamError("bhkBlendController load target missing"))?
        .apply_loaded_prefix(&SingleInterpControllerPayload {
            time: payload.time,
            interpolator_index: NULL_REFERENCE,
            bytes_consumed: payload.time.bytes_consumed,
        });
    Ok(())
}

/// IDA: 008AA990 bhkBlendController's key value at a scaled time: no keys answers false, one its two floats; otherwise from
/// +0x3C (back to 0 when that key is past the time) forward to the last key before the time, the next key's span (a zero span
/// answers the found key's floats), each float NiFloat_LinearInterpolate (006D3690: ratio * end + (1 - ratio) * start).
/// Completion: 100%; the four-at-a-time search finds the same key as the plain one.
fn blend_key_value(controller: &mut BhkBlendController, time: f32) -> Option<(f32, f32)> {
    let key = |index: usize, word: usize| f32::from_bits(controller.keys_44[index][word]);
    let count = controller.keys_44.len();
    match count {
        0 => return None,
        1 => return Some((key(0, 1), key(0, 2))),
        _ => {}
    }
    let last = count - 1;
    let mut found = controller.last_key_3c as usize;
    let mut found_time = key(found, 0);
    if found_time > time {
        found = 0;
        found_time = key(0, 0);
    }
    let mut next = found + 1;
    let mut next_time = found_time;
    while next <= last {
        next_time = key(next, 0);
        if next_time >= time {
            break;
        }
        found += 1;
        found_time = next_time;
        next += 1;
    }
    controller.last_key_3c = found as u32;
    let span = next_time - found_time;
    let (start_a, start_b) = (key(found, 1), key(found, 2));
    if span == 0.0 {
        return Some((start_a, start_b));
    }
    let ratio = (time - found_time) / span;
    let lerp = |start: f32, end: f32| ratio * end + (1.0 - ratio) * start;
    Some((lerp(start_a, key(next, 1)), lerp(start_b, key(next, 2))))
}

/// IDA: 008AAD60 bhkBlendController_Update(time): with a target, flag 8 (active) and keys, 00497420 finds the target's +0xA8
/// collision object as a bhkBlendCollisionObject (none: +0x20 = time); then vt+0x64 (007155A0) scales the time, 008AA990's
/// floats go to the object's +0x14 / +0x18 (the first time saving the old pair at +0x58 / +0x5C), and at the high key time with
/// the clamp cycle (+0x08 & 6 == 4) 008AA7F0, 008AA3E0, 008AA420 and vt+0x50 Stop run, a stop here. Completion: 100% but the
/// clamp end.
/// # Errors
/// The clamp end.
pub fn update_bhk_blend_controller(
    world: &mut World,
    controller: Entity,
    time: f32,
) -> Result<(), StreamError> {
    let mut state = world
        .get::<NodeController>(controller)
        .ok_or(StreamError(
            "008AAD60: a bhkBlendController with no time state",
        ))?
        .time
        .clone();
    let has_keys = world
        .get::<BhkBlendController>(controller)
        .is_some_and(|blend| !blend.keys_44.is_empty());
    let Some(target) = state.target_30 else {
        return Ok(());
    };
    if state.fields.flags & 8 == 0 || !has_keys {
        return Ok(());
    }
    let object = world
        .get::<super::av_object::NiAvCollision>(target)
        .map(|collision| collision.0)
        .filter(|&object| {
            world
                .get::<crate::bs::havok_stream::BhkBlendCollisionObject>(object)
                .is_some()
        });
    let Some(object) = object else {
        state.fields.last_time = time;
        world
            .get_mut::<NodeController>(controller)
            .expect("read above")
            .time = state;
        return Ok(());
    };
    let mut cache =
        *world.get_resource_or_init::<super::controller_time::ParticleControllerTimeCache>();
    let scaled = super::controller_time::evaluate_particle_controller_time(
        &mut state.fields,
        time,
        &mut cache,
    );
    *world.resource_mut::<super::controller_time::ParticleControllerTimeCache>() = cache;
    let mut blend = world
        .get::<BhkBlendController>(controller)
        .expect("checked")
        .clone();
    if let Some((first, second)) = blend_key_value(&mut blend, scaled) {
        let mut collision = world
            .get_mut::<crate::bs::havok_stream::BhkBlendCollisionObject>(object)
            .expect("filtered");
        if f32::from_bits(blend.saved_blend_58) < 0.0 {
            blend.saved_blend_58 = collision.float_14;
            blend.saved_blend_5c = collision.float_18;
        }
        collision.float_14 = first.to_bits();
        collision.float_18 = second.to_bits();
    }
    let flags = state.fields.flags;
    let at_end = state.fields.high_key_time == scaled && flags & 6 == 4;
    *world
        .get_mut::<BhkBlendController>(controller)
        .expect("checked") = blend;
    world
        .get_mut::<NodeController>(controller)
        .expect("read above")
        .time = state;
    if at_end {
        return Err(StreamError(
            "008AADEB: a clamped bhkBlendController's end (008AA7F0, vt+0x50)",
        ));
    }
    Ok(())
}
