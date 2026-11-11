//! NiParticleSystem: a NiGeometry in the original, with its modifier list and name map.
//!
//! The shared NiObjectNET and NiAVObject parts are `NiObjectNet`, `NiAvProperties`, `NiAvParent` and `NiAvWorld` on the same entity.
//! The geometry prefix is parsed here, not through the NiNode payload, because 00709EE0 clears flag 0x40 and reads child and effect
//! lists, and 007234B0 does not. The NiGeometry +0xAC, +0xB0 and +0xBC property state, effect state and shader wait for the scene
//! state family.

#![allow(
    clippy::type_complexity,
    reason = "the C++ port's injected std::function services, which stand for 007155A0 and the original target virtuals"
)]

use bevy_ecs::prelude::*;
use oblivion_core::crt::{c_prefix, stricmp};
use oblivion_core::error::GameError;
use oblivion_core::hash::ni_string_hash;
use oblivion_core::reader::ByteReader;

use super::super::bs::nif_objects::BsParentVelocityModifier;
use super::av_object::{AvObjectLinkStatus, NiAvParent, NiAvProperties, plan_av_object_links};
use super::color_animation::ColorKeyData;
use super::object_net::{
    NiObjectNet, ObjectNetPending, parse_object_net_payload, resolve_object_net_links,
};
use super::particle_color::{ParticleColorKeys, ParticleColorModifier, ParticleColorSample};
use super::particle_data::ParticleData;
use super::particle_drag::ParticleDragModifier;
use super::particle_dynamics::{
    ParticleAgeDeathModifier, ParticleBoundUpdateModifier, ParticleGrowFadeModifier,
    ParticlePositionModifier,
};
use super::particle_emitter::{ParticleBoxEmitter, ParticleCylinderEmitter, ParticleSphereEmitter};
use super::particle_gravity::ParticleGravityModifier;
use super::particle_mesh_emitter::ParticleMeshEmitter;
use super::particle_rotation::ParticleRotationModifier;
use super::particle_spawn::{ParticleSpawnModifier, ParticleSpawnServices};
use super::scene_node::NiAvWorld;
use super::stream_objects::{
    NULL_REFERENCE, NiClass, StreamObjectTable, check_link_index, read_owned_c_string,
};
use super::transform::NiTransform;

/// IDA: N/A the error of this file's `stream_objects::check_link_index` calls (the host's guard before 007124A0 / 00752CB0's reads).
const PARTICLE_SYSTEM_INDEX_OUT_OF_RANGE: GameError =
    GameError::unclassified(0, "particle system link index out of range");

const NEGATIVE_MAX: f32 = -f32::MAX;
const MODIFIER_BUCKETS: u32 = 37;
const TRUNCATED_U32: &str = "truncated particle system modifier list";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParticleSystemTail {
    pub in_world_space: bool,
    pub modifier_indices: Vec<u32>,
    pub bytes_consumed: usize,
}

impl Default for ParticleSystemTail {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            in_world_space: true,
            modifier_indices: Vec::new(),
            bytes_consumed: 0,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ParticleSystemShaderPayload {
    pub name: Option<Vec<u8>>,
    pub shader_field_c: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParticleSystemPayload {
    pub net: ObjectNetPending,
    pub raw_av_flags: u16,
    pub effective_av_flags: u16,
    pub local_transform: NiTransform,
    pub property_indices: Vec<u32>,
    pub collision_index: u32,
    pub geometry_data_index: u32,
    pub skin_index: u32,
    pub shader_flag_raw: u8,
    pub shader_payload: Option<ParticleSystemShaderPayload>,
    pub tail: ParticleSystemTail,
    /// IDA: NiStream +0xD8 file version for the 007081B0 link gates.
    pub stream_version: u32,
    pub bytes_consumed: usize,
}

impl Default for ParticleSystemPayload {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            net: ObjectNetPending::default(),
            raw_av_flags: 0,
            effective_av_flags: 0,
            local_transform: NiTransform::default(),
            property_indices: Vec::new(),
            collision_index: NULL_REFERENCE,
            geometry_data_index: NULL_REFERENCE,
            skin_index: NULL_REFERENCE,
            shader_flag_raw: 0,
            shader_payload: None,
            tail: ParticleSystemTail::default(),
            stream_version: 0x1400_0004,
            bytes_consumed: 0,
        }
    }
}

// IDA: 00749B70 after 007178F0's NiGeometry base: one in-world-space byte, then 00712AE0 reads a u32 count and count u32 deferred
// modifier indices
// Completion: 100% of the bounded modern tail byte order
// TODO: [ ] The old-version NiObject prefix.
pub fn parse_particle_system_tail(
    bytes: &[u8],
    stream_version: u32,
) -> Result<ParticleSystemTail, GameError> {
    if stream_version < 0x0A01_0072 {
        return Err(GameError::unclassified(
            0,
            "legacy particle system base payload unsupported",
        ));
    }
    let mut reader = ByteReader::new(bytes);
    let in_world_space = reader.u8("truncated particle system world-space byte")? != 0;
    let count = reader.u32(TRUNCATED_U32)?;
    if count as usize > reader.remaining() / 4 {
        return Err(GameError::stream(
            "truncated particle system modifier references",
        ));
    }
    let mut modifier_indices = Vec::with_capacity(count as usize);
    for _ in 0..count {
        modifier_indices.push(reader.u32(TRUNCATED_U32)?);
    }
    Ok(ParticleSystemTail {
        in_world_space,
        modifier_indices,
        bytes_consumed: reader.position(),
    })
}

// IDA: 006FFCE0 NiObjectNET name, extra and controller; 00707F00 AV flags, translation, matrix, scale, property and collision; 007234B0
// geometry refs and optional shader; 00749B70 world flag and modifier references
// Completion: 100% of the modern >= 0A010072 field widths, order and raw link retention
// TODO: [ ] Resolve every inherited owner and NiStream callback side effect. Reconcile the 007234B0 prefix with the mesh geometry port.
pub fn parse_particle_system_payload(
    bytes: &[u8],
    stream_version: u32,
) -> Result<ParticleSystemPayload, GameError> {
    const TRUNCATED_FLAGS: &str = "truncated particle system AV flags";
    let net = parse_object_net_payload(bytes, stream_version)?;
    let mut reader = ByteReader::new(bytes);
    reader.take(net.bytes_consumed, TRUNCATED_U32)?;
    let raw_av_flags = reader.u16(TRUNCATED_FLAGS)?;
    let mut effective_av_flags = raw_av_flags;
    if stream_version < 0x1400_0004 {
        effective_av_flags &= !0x40;
    }
    effective_av_flags = (effective_av_flags & 0xFFE1) | 0x000E;
    let mut local_transform = NiTransform::default();
    for component in &mut local_transform.translation {
        *component = reader.f32(TRUNCATED_U32)?;
    }
    for component in &mut local_transform.rotation {
        *component = reader.f32(TRUNCATED_U32)?;
    }
    local_transform.scale = reader.f32(TRUNCATED_U32)?;
    let property_indices = super::av_object::read_av_reference_list(
        &mut reader,
        TRUNCATED_U32,
        GameError::stream("truncated particle system deferred list"),
    )?;
    let collision_index = reader.u32(TRUNCATED_U32)?;
    let geometry_data_index = reader.u32(TRUNCATED_U32)?;
    let skin_index = reader.u32(TRUNCATED_U32)?;
    let shader_flag_raw = reader.u8("truncated particle system shader flag")?;
    let mut shader_payload = None;
    if shader_flag_raw != 0 {
        // IDA: 00713620 reads the shader name at 007386EE using a signed length.
        let name = read_owned_c_string(
            &mut reader,
            TRUNCATED_U32,
            "truncated particle system shader name",
        )?;
        shader_payload = Some(ParticleSystemShaderPayload {
            name,
            shader_field_c: reader.u32(TRUNCATED_U32)?,
        });
    }
    let at = reader.position();
    let tail = parse_particle_system_tail(&bytes[at..], stream_version)?;
    let bytes_consumed = at + tail.bytes_consumed;
    Ok(ParticleSystemPayload {
        net,
        raw_av_flags,
        effective_av_flags,
        local_transform,
        property_indices,
        collision_index,
        geometry_data_index,
        skin_index,
        shader_flag_raw,
        shader_payload,
        tail,
        stream_version,
        bytes_consumed,
    })
}

/// IDA: 00749990's list row: the modifier, its +0x0C order and +0x08 name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParticleSystemModifierAttachment {
    pub owner: Entity,
    pub original_order: u32,
    pub original_name: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct NameMapEntry {
    key: Vec<u8>,
    modifier_index: usize,
}

/// IDA: 00749EE0 creates the 0xF0-byte NiParticleSystem, with its NiGeometry base at 00741FA0 / 00722810. Host storage is separate
/// from the original ABI. Completion: 100% of the observed cold fields, typed data association and ordered list subset.
/// TODO: [ ] Full NiGeometry base state, original refcounts and the map allocator.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct ParticleSystem {
    /// IDA: 00749EE0 +0xC0 cold flag; 00749310 and 00749550 read it.
    pub in_world_space_c0: bool,
    /// IDA: 00749EE0 +0xE4.
    /// TODO: [ ] Identify all writers and consumers before assigning a stronger meaning.
    pub byte_e4: bool,
    /// IDA: 00749EE0 +0xE8.
    pub system_update_time_e8: f32,
    /// IDA: 00749EE0 +0xEC; 00754F0C stores 1 on a strict mapped-time rewind.
    pub reset_requested_ec: bool,
    /// IDA: 006FFCE0's inherited records, kept as raw modern load state.
    pub net: ObjectNetPending,
    pub raw_av_flags: u16,
    /// IDA: +0x18 the live flags; 006CF9D0 tests this word and writes the local transform of any NiAVObject target.
    pub effective_av_flags_18: u16,
    /// IDA: +0x30 / +0x54 / +0x60 the local NiTransform.
    pub local_transform_30: NiTransform,
    pub property_indices: Vec<u32>,
    pub collision_index: u32,
    pub geometry_data_index: u32,
    pub skin_index: u32,
    /// IDA: 00723520's raw one-byte optional shader marker.
    pub shader_flag_raw: u8,
    pub shader_payload: Option<ParticleSystemShaderPayload>,
    /// IDA: 00722B30's NiGeometry +0xAC NiPropertyState.
    pub property_state_ac: Option<Entity>,
    /// IDA: +0xB0 the NiDynamicEffectState, which 00742060 (vt+0x70) installs.
    pub effect_state_b0: Option<Entity>,
    /// IDA: 00723350 NiGeometry +0xB4: a verified NiPSysData.
    pub data_b4: Option<Entity>,
    /// IDA: 00749990's +0xC4 list, head first.
    pub modifiers_c4: Vec<ParticleSystemModifierAttachment>,
    /// IDA: 00712AE0 / 00749BC0's count and sequential deferred references.
    pub pending_modifier_indices: Vec<u32>,
    modifier_map_d4: Vec<Vec<NameMapEntry>>,
}

impl Default for ParticleSystem {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            in_world_space_c0: true,
            byte_e4: true,
            system_update_time_e8: NEGATIVE_MAX,
            reset_requested_ec: false,
            net: ObjectNetPending::default(),
            raw_av_flags: 0,
            effective_av_flags_18: 0,
            local_transform_30: NiTransform::default(),
            property_indices: Vec::new(),
            collision_index: NULL_REFERENCE,
            geometry_data_index: NULL_REFERENCE,
            skin_index: NULL_REFERENCE,
            shader_flag_raw: 0,
            shader_payload: None,
            property_state_ac: None,
            effect_state_b0: None,
            data_b4: None,
            modifiers_c4: Vec::new(),
            pending_modifier_indices: Vec::new(),
            modifier_map_d4: vec![Vec::new(); MODIFIER_BUCKETS as usize],
        }
    }
}

// IDA: 00749EE0's cold NiParticleSystem with its shared NiObjectNET and NiAVObject parts
// Completion: 100% of the observed cold fields
// TODO: [ ] Inherited NiAVObject cold state and CRT allocation.
pub fn spawn_particle_system(world: &mut World) -> Entity {
    world
        .spawn((
            NiClass::ParticleSystem,
            ParticleSystem::default(),
            NiObjectNet::default(),
            NiAvProperties::default(),
            NiAvParent::default(),
            NiAvWorld::default(),
        ))
        .id()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParticleSystemLinkStatus {
    Linked,
    UnsupportedOwner,
}

// IDA: N/A the original 37-bucket selection over 007DAED0's hash (`ni_string_hash`: signed raw C-string bytes,
// h = 33 * h + byte) of the name's C-string prefix.
// Completion: 100% of the modulo-2^32 hash
// TODO: [ ] Confirm non-ASCII signed-char and locale pairing on the original CRT.
fn name_bucket(name: &[u8]) -> usize {
    (ni_string_hash(c_prefix(name)) % MODIFIER_BUCKETS) as usize
}

struct ModifierView {
    name: Option<Vec<u8>>,
    order: u32,
}

// IDA: 00749990 reads name +0x08 and order +0x0C from the original NiPSysModifier base
// Completion: 100% of the typed metadata view; `None` is a modifier component the entity does not hold
// TODO: [ ] Bind original live modifier memory and refcount ABI.
fn modifier_view(world: &World, owner: Entity) -> Option<ModifierView> {
    // IDA: N/A a helper, with no address of its own.
    fn base(base: &super::particle_dynamics::ParticleModifierBase) -> ModifierView {
        ModifierView {
            name: base.name_08.clone(),
            order: base.order_0c,
        }
    }
    // IDA: N/A a helper, with no address of its own.
    fn pending(base: &super::particle_dynamics::ParticleModifierPending) -> ModifierView {
        ModifierView {
            name: base.name.clone(),
            order: base.order,
        }
    }
    match world.get::<NiClass>(owner)? {
        NiClass::ParticleColorModifier => world
            .get::<ParticleColorModifier>(owner)
            .map(|held| base(&held.base)),
        NiClass::ParticlePositionModifier => world
            .get::<ParticlePositionModifier>(owner)
            .map(|held| base(&held.base)),
        NiClass::ParticleGrowFadeModifier => world
            .get::<ParticleGrowFadeModifier>(owner)
            .map(|held| base(&held.base)),
        NiClass::ParticleAgeDeathModifier => world
            .get::<ParticleAgeDeathModifier>(owner)
            .map(|held| base(&held.base)),
        NiClass::ParticleBoundUpdateModifier => world
            .get::<ParticleBoundUpdateModifier>(owner)
            .map(|held| base(&held.base)),
        NiClass::ParticleDragModifier => world
            .get::<ParticleDragModifier>(owner)
            .map(|held| base(&held.base)),
        NiClass::ParticleSpawnModifier => world
            .get::<ParticleSpawnModifier>(owner)
            .map(|held| base(&held.base)),
        NiClass::ParticleRotationModifier => world
            .get::<ParticleRotationModifier>(owner)
            .map(|held| base(&held.base)),
        NiClass::ParticleGravityModifier => world
            .get::<ParticleGravityModifier>(owner)
            .map(|held| base(&held.base)),
        NiClass::BsParentVelocityModifier => world
            .get::<BsParentVelocityModifier>(owner)
            .map(|held| base(&held.base)),
        NiClass::ParticleBoxEmitter => world
            .get::<ParticleBoxEmitter>(owner)
            .map(|held| pending(&held.state.base)),
        NiClass::ParticleSphereEmitter => world
            .get::<ParticleSphereEmitter>(owner)
            .map(|held| pending(&held.state.base)),
        NiClass::ParticleCylinderEmitter => world
            .get::<ParticleCylinderEmitter>(owner)
            .map(|held| pending(&held.state.base)),
        NiClass::ParticleMeshEmitter => world
            .get::<ParticleMeshEmitter>(owner)
            .map(|held| pending(&held.state.base)),
        _ => None,
    }
}

// IDA: 00749990's later +0x14 read and 00756710's comparison byte
// Completion: 100% of the current typed owner and canonical BYTE view, without cloning the name
// TODO: [ ] Noncanonical active bytes.
fn modifier_active(world: &World, owner: Entity) -> Option<bool> {
    match world.get::<NiClass>(owner)? {
        NiClass::ParticleColorModifier => world
            .get::<ParticleColorModifier>(owner)
            .map(|held| held.base.active_14),
        NiClass::ParticlePositionModifier => world
            .get::<ParticlePositionModifier>(owner)
            .map(|held| held.base.active_14),
        NiClass::ParticleGrowFadeModifier => world
            .get::<ParticleGrowFadeModifier>(owner)
            .map(|held| held.base.active_14),
        NiClass::ParticleAgeDeathModifier => world
            .get::<ParticleAgeDeathModifier>(owner)
            .map(|held| held.base.active_14),
        NiClass::ParticleBoundUpdateModifier => world
            .get::<ParticleBoundUpdateModifier>(owner)
            .map(|held| held.base.active_14),
        NiClass::ParticleDragModifier => world
            .get::<ParticleDragModifier>(owner)
            .map(|held| held.base.active_14),
        NiClass::ParticleSpawnModifier => world
            .get::<ParticleSpawnModifier>(owner)
            .map(|held| held.base.active_14),
        NiClass::ParticleRotationModifier => world
            .get::<ParticleRotationModifier>(owner)
            .map(|held| held.base.active_14),
        NiClass::ParticleGravityModifier => world
            .get::<ParticleGravityModifier>(owner)
            .map(|held| held.base.active_14),
        NiClass::BsParentVelocityModifier => world
            .get::<BsParentVelocityModifier>(owner)
            .map(|held| held.base.active_14),
        NiClass::ParticleBoxEmitter => world
            .get::<ParticleBoxEmitter>(owner)
            .map(|held| held.state.base.active),
        NiClass::ParticleSphereEmitter => world
            .get::<ParticleSphereEmitter>(owner)
            .map(|held| held.state.base.active),
        NiClass::ParticleCylinderEmitter => world
            .get::<ParticleCylinderEmitter>(owner)
            .map(|held| held.state.base.active),
        NiClass::ParticleMeshEmitter => world
            .get::<ParticleMeshEmitter>(owner)
            .map(|held| held.state.base.active),
        _ => None,
    }
}

// IDA: 006FAB60 consists of an arg BYTE read, a BYTE +0x14 store and retn 4; all supported modifier virtual +0x58 slots select it
// Completion: 100% of the canonical byte effect, with no added reset, scheduling or notification
// TODO: [ ] Unrestricted raw ABI bytes.
fn store_modifier_active(world: &mut World, owner: Entity, value: bool) -> bool {
    macro_rules! store {
        ($component:ty, $($field:ident).+) => {
            world
                .get_mut::<$component>(owner)
                .map(|mut held| held.$($field).+ = value)
                .is_some()
        };
    }
    match world.get::<NiClass>(owner).copied() {
        Some(NiClass::ParticleColorModifier) => store!(ParticleColorModifier, base.active_14),
        Some(NiClass::ParticlePositionModifier) => store!(ParticlePositionModifier, base.active_14),
        Some(NiClass::ParticleGrowFadeModifier) => store!(ParticleGrowFadeModifier, base.active_14),
        Some(NiClass::ParticleAgeDeathModifier) => store!(ParticleAgeDeathModifier, base.active_14),
        Some(NiClass::ParticleBoundUpdateModifier) => {
            store!(ParticleBoundUpdateModifier, base.active_14)
        }
        Some(NiClass::ParticleDragModifier) => store!(ParticleDragModifier, base.active_14),
        Some(NiClass::ParticleSpawnModifier) => store!(ParticleSpawnModifier, base.active_14),
        Some(NiClass::ParticleRotationModifier) => store!(ParticleRotationModifier, base.active_14),
        Some(NiClass::ParticleGravityModifier) => store!(ParticleGravityModifier, base.active_14),
        Some(NiClass::BsParentVelocityModifier) => store!(BsParentVelocityModifier, base.active_14),
        Some(NiClass::ParticleBoxEmitter) => store!(ParticleBoxEmitter, state.base.active),
        Some(NiClass::ParticleSphereEmitter) => store!(ParticleSphereEmitter, state.base.active),
        Some(NiClass::ParticleCylinderEmitter) => {
            store!(ParticleCylinderEmitter, state.base.active)
        }
        Some(NiClass::ParticleMeshEmitter) => store!(ParticleMeshEmitter, state.base.active),
        _ => false,
    }
}

impl ParticleSystem {
    // IDA: 00749B70 stores the normalized +0xC0 flag and queues list refs
    // Completion: 100% of the publication on a fresh owner; the list stays deferred
    // TODO: [ ] Resolve this list only after the full NiGeometry base link pass.
    pub fn apply_loaded_tail(&mut self, tail: &ParticleSystemTail) -> Result<(), GameError> {
        if !self.modifiers_c4.is_empty() || !self.pending_modifier_indices.is_empty() {
            return Err(GameError::unclassified(
                0,
                "particle system tail already applied",
            ));
        }
        self.in_world_space_c0 = tail.in_world_space;
        self.pending_modifier_indices
            .clone_from(&tail.modifier_indices);
        Ok(())
    }

    // IDA: 006FFCE0 / 00707F00 / 007234B0 / 00749B70 combined modern load
    // Completion: 100% of the publication of parsed fields without silently resolving links
    // TODO: [ ] Reference-counted owners in exact LinkBinary order.
    pub fn apply_loaded_payload(
        &mut self,
        payload: &ParticleSystemPayload,
    ) -> Result<(), GameError> {
        if !self.modifiers_c4.is_empty() || !self.pending_modifier_indices.is_empty() {
            return Err(GameError::unclassified(
                0,
                "particle system payload already applied",
            ));
        }
        self.net = payload.net.clone();
        self.raw_av_flags = payload.raw_av_flags;
        self.effective_av_flags_18 = payload.effective_av_flags;
        self.local_transform_30 = payload.local_transform;
        self.property_indices.clone_from(&payload.property_indices);
        self.collision_index = payload.collision_index;
        self.geometry_data_index = payload.geometry_data_index;
        self.skin_index = payload.skin_index;
        self.shader_flag_raw = payload.shader_flag_raw;
        self.shader_payload.clone_from(&payload.shader_payload);
        self.in_world_space_c0 = payload.tail.in_world_space;
        self.pending_modifier_indices
            .clone_from(&payload.tail.modifier_indices);
        Ok(())
    }

    // IDA: 00749990 scans the list head to tail using unsigned `ja` for the first existing order greater than the new order, then
    // inserts before or appends. 007DAED0 / 00584D10 govern the separate 37-bucket name map
    // Completion: 100% of the list order, stable ties and map behaviour for typed named owners
    // TODO: [ ] FormHeap node allocation and Interlocked reference transitions.
    pub fn attach_modifier(&mut self, world: &World, modifier: Entity) -> Result<(), GameError> {
        let view = modifier_view(world, modifier)
            .ok_or(GameError::unclassified(0, "null particle modifier owner"))?;
        let name = view.name.ok_or(GameError::unclassified(
            0,
            "particle modifier has null map key",
        ))?;
        let index = self
            .modifiers_c4
            .iter()
            .position(|existing| existing.original_order > view.order)
            .unwrap_or(self.modifiers_c4.len());
        let bucket = name_bucket(&name);
        let key = c_prefix(&name).to_vec();
        self.modifiers_c4.insert(
            index,
            ParticleSystemModifierAttachment {
                owner: modifier,
                original_order: view.order,
                original_name: name,
            },
        );
        for entries in &mut self.modifier_map_d4 {
            for entry in entries {
                if entry.modifier_index >= index {
                    entry.modifier_index += 1;
                }
            }
        }
        let entries = &mut self.modifier_map_d4[bucket];
        if let Some(duplicate) = entries
            .iter_mut()
            .find(|entry| stricmp(&entry.key, &key) == 0)
        {
            duplicate.modifier_index = index;
        } else {
            entries.push(NameMapEntry {
                key,
                modifier_index: index,
            });
        }
        Ok(())
    }

    // IDA: 007DAED0 raw-byte bucket followed by 00584D10 _stricmp equality. Bytes after an embedded NUL do not enter the lookup
    // Completion: 100% of the lookup including the original same-bucket restriction
    // TODO: [ ] Non-ASCII CRT locale and map allocation-failure effects.
    #[must_use]
    pub fn find_modifier(&self, name: &[u8]) -> Option<Entity> {
        let key = c_prefix(name);
        self.modifier_map_d4[name_bucket(key)]
            .iter()
            .find(|entry| stricmp(&entry.key, key) == 0)
            .map(|entry| self.modifiers_c4[entry.modifier_index].owner)
    }

    // IDA: 00756710 compares the cached NiPSysModifier BYTE +0x14. The host searches retained identities for safe ownership; it does
    // not redo the name lookup
    // Completion: 100% of the canonical active byte view
    // TODO: [ ] The raw pointer lifetime.
    #[must_use]
    pub fn modifier_active_byte(&self, world: &World, identity: Entity) -> Option<u8> {
        self.modifiers_c4
            .iter()
            .find(|attachment| attachment.owner == identity)
            .and_then(|attachment| modifier_active(world, attachment.owner))
            .map(u8::from)
    }

    // IDA: 00754F0C mov byte ptr [NiParticleSystem+0xEC], 1
    // Completion: 100% of the single flag store after ResetOnLoop's strict rewind decision
    pub const fn request_reset(&mut self) {
        self.reset_requested_ec = true;
    }

    // IDA: 00749EE0 +0xD8 hash bucket count
    // Completion: 100% of the cold constant
    // TODO: [ ] The original map node allocator and capacity transitions.
    #[must_use]
    pub const fn modifier_bucket_count() -> usize {
        MODIFIER_BUCKETS as usize
    }
}

// IDA: 006FAB60 through a system's retained identities
// Completion: 100% of the canonical host byte effect
// TODO: [ ] Unrestricted raw ABI bytes and stale-cache lifetime.
pub fn set_modifier_active_byte(
    world: &mut World,
    system: Entity,
    identity: Entity,
    value: u8,
) -> Result<bool, GameError> {
    if value > 1 {
        return Err(GameError::unclassified(
            0,
            "noncanonical modifier active byte unsupported",
        ));
    }
    let retained = world.get::<ParticleSystem>(system).is_some_and(|state| {
        state
            .modifiers_c4
            .iter()
            .any(|attachment| attachment.owner == identity)
    });
    Ok(retained && store_modifier_active(world, identity, value != 0))
}

// IDA: 00749990 on a stream object's system
// Completion: 100%
pub fn attach_particle_modifier(
    world: &mut World,
    system: Entity,
    modifier: Entity,
) -> Result<(), GameError> {
    let mut state = world
        .get::<ParticleSystem>(system)
        .ok_or(GameError::unclassified(
            0,
            "particle system component missing",
        ))?
        .clone();
    state.attach_modifier(world, modifier)?;
    world.entity_mut(system).insert(state);
    Ok(())
}

// IDA: 00700050 inherited extras and controller, 007081B0 property and collision, 00723350 geometry data and skin, and 00749BC0's ordered
// modifier references. A rejected link publishes nothing on the system; invalid indices and wrong supported types fail
// Completion: 100% of the supported typed links, with an explicit unsupported owner status
// TODO: [ ] Concrete controller, property, collision, skin and shader adapters.
pub fn resolve_particle_system_links(
    world: &mut World,
    system: Entity,
    payload: &ParticleSystemPayload,
    table: &StreamObjectTable,
) -> Result<ParticleSystemLinkStatus, GameError> {
    let object_count = table.objects.len();
    for &index in &payload.net.extra_indices {
        check_link_index(index, object_count, PARTICLE_SYSTEM_INDEX_OUT_OF_RANGE)?;
        if index != NULL_REFERENCE
            && !world
                .get::<NiClass>(table.objects[index as usize].entity)
                .is_some_and(|class| class.is_extra_data())
        {
            return Err(GameError::unclassified(
                0,
                "particle system extra link has wrong type",
            ));
        }
    }
    check_link_index(
        payload.net.controller_index,
        object_count,
        PARTICLE_SYSTEM_INDEX_OUT_OF_RANGE,
    )?;
    for &index in &payload.property_indices {
        check_link_index(index, object_count, PARTICLE_SYSTEM_INDEX_OUT_OF_RANGE)?;
    }
    check_link_index(
        payload.collision_index,
        object_count,
        PARTICLE_SYSTEM_INDEX_OUT_OF_RANGE,
    )?;
    check_link_index(
        payload.geometry_data_index,
        object_count,
        PARTICLE_SYSTEM_INDEX_OUT_OF_RANGE,
    )?;
    check_link_index(
        payload.skin_index,
        object_count,
        PARTICLE_SYSTEM_INDEX_OUT_OF_RANGE,
    )?;

    let mut geometry_data = None;
    if payload.geometry_data_index != NULL_REFERENCE {
        let entity = table.objects[payload.geometry_data_index as usize].entity;
        if world.get::<NiClass>(entity) != Some(&NiClass::ParticleData) {
            return Err(GameError::unclassified(
                0,
                "particle system data link is not NiPSysData",
            ));
        }
        geometry_data = Some(entity);
    }
    let mut modifiers = Vec::with_capacity(payload.tail.modifier_indices.len());
    let mut unsupported_modifier = false;
    for &index in &payload.tail.modifier_indices {
        check_link_index(index, object_count, PARTICLE_SYSTEM_INDEX_OUT_OF_RANGE)?;
        if index == NULL_REFERENCE {
            return Err(GameError::unclassified(
                0,
                "null particle modifier list entry",
            ));
        }
        let entity = table.objects[index as usize].entity;
        match world.get::<NiClass>(entity) {
            None => unsupported_modifier = true,
            Some(class) if class.is_particle_modifier() => modifiers.push(entity),
            Some(_) => {
                return Err(GameError::unclassified(
                    0,
                    "particle modifier link has wrong object type",
                ));
            }
        }
    }

    if unsupported_modifier || payload.skin_index != NULL_REFERENCE || payload.shader_flag_raw != 0
    {
        return Ok(ParticleSystemLinkStatus::UnsupportedOwner);
    }
    // 007081B0's inherited properties (+0x98) and collision (+0xA8) are preflighted with the shared NiAVObject plan; collision stays a
    // later-phase blocker (bhk factories register in TES_Constructor).
    let plan = plan_av_object_links(
        world,
        &payload.net,
        &payload.property_indices,
        payload.collision_index,
        table,
        payload.stream_version,
    )?;
    if plan.status != AvObjectLinkStatus::Linked {
        return Ok(ParticleSystemLinkStatus::UnsupportedOwner);
    }

    let mut staged = world
        .get::<ParticleSystem>(system)
        .ok_or(GameError::unclassified(
            0,
            "particle system component missing",
        ))?
        .clone();
    if !staged.modifiers_c4.is_empty() {
        return Err(GameError::unclassified(
            0,
            "particle system modifiers already linked",
        ));
    }
    // 00700050 extras and controller head, then 007081B0's +0x98 property list.
    let net = resolve_object_net_links(world, &payload.net, table)?;
    staged.data_b4 = geometry_data;
    for modifier in modifiers {
        staged.attach_modifier(world, modifier)?;
    }
    world
        .entity_mut(system)
        .insert((staged, net, NiAvProperties(plan.properties)));
    Ok(ParticleSystemLinkStatus::Linked)
}

// IDA: 00749510's and 007498F0's +0xB4 read
// Completion: 100%
fn system_data(world: &World, system: Entity) -> Result<Entity, GameError> {
    world
        .get::<ParticleSystem>(system)
        .ok_or(GameError::unclassified(
            0,
            "particle system component missing",
        ))?
        .data_b4
        .filter(|&data| world.get::<ParticleData>(data).is_some())
        .ok_or(GameError::unclassified(
            0,
            "particle system data link unresolved",
        ))
}

// IDA: N/A runs `action` with a data entity's ParticleData detached, and restores it whatever the outcome
// Completion: 100%
fn with_data<R>(
    world: &mut World,
    data: Entity,
    action: impl FnOnce(&mut World, &mut ParticleData) -> R,
) -> R {
    let mut state = std::mem::take(
        &mut *world
            .get_mut::<ParticleData>(data)
            .expect("the link holds a NiPSysData"),
    );
    let result = action(world, &mut state);
    *world
        .get_mut::<ParticleData>(data)
        .expect("the link holds a NiPSysData") = state;
    result
}

/// The services of 007498F0: reset receives the data entity, dispatch the modifier, the data entity and the time.
pub type ParticleResetService<'a> = Option<&'a mut dyn FnMut(&mut World, Entity)>;
pub type ParticleUpdateDispatch<'a> =
    Option<&'a mut dyn FnMut(&mut World, Entity, Entity, f32) -> Result<(), GameError>>;

// IDA: 007498F0 calls data +0x5C, rewinds through 00749470 on a time regression or the flag, calls active modifiers' +0x4C in list order,
// sets data extra flags +0x2E bits 0..2 and stores +0xE8 current time
// Completion: 100% of the original schedule and flag ordering, with explicit host virtual services
// TODO: [ ] The reset and controller path and type-specific modifier dispatch.
pub fn update_particle_system(
    world: &mut World,
    system: Entity,
    current_time: f32,
    reset_services: ParticleResetService<'_>,
    mut dispatch: ParticleUpdateDispatch<'_>,
) -> Result<(), GameError> {
    let data = system_data(world, system)?;
    if !current_time.is_finite() {
        return Err(GameError::unclassified(
            0,
            "particle system time outside finite domain",
        ));
    }
    world
        .get_mut::<ParticleData>(data)
        .expect("checked by system_data")
        .finalize_added_particles()?;
    let state = world
        .get::<ParticleSystem>(system)
        .expect("checked by system_data");
    if state.reset_requested_ec || state.system_update_time_e8 > current_time {
        let Some(reset) = reset_services else {
            return Err(GameError::unclassified(
                0,
                "particle system reset services unresolved",
            ));
        };
        reset(world, data);
        let mut state = world
            .get_mut::<ParticleSystem>(system)
            .expect("checked by system_data");
        state.system_update_time_e8 = NEGATIVE_MAX;
        state.reset_requested_ec = false;
    }
    let mut state = world
        .get_mut::<ParticleSystem>(system)
        .expect("checked by system_data");
    if state.system_update_time_e8 == NEGATIVE_MAX {
        state.system_update_time_e8 = current_time;
    }
    let owners: Vec<Entity> = state.modifiers_c4.iter().map(|row| row.owner).collect();
    for owner in owners {
        if !modifier_active(world, owner)
            .ok_or(GameError::unclassified(0, "null particle modifier owner"))?
        {
            continue;
        }
        let Some(dispatch) = dispatch.as_mut() else {
            return Err(GameError::unclassified(
                0,
                "particle modifier dispatcher unresolved",
            ));
        };
        dispatch(world, owner, data, current_time)?;
    }
    world
        .get_mut::<ParticleData>(data)
        .expect("checked by system_data")
        .geometry_extra_flags_2e |= 7;
    world
        .get_mut::<ParticleSystem>(system)
        .expect("checked by system_data")
        .system_update_time_e8 = current_time;
    Ok(())
}

/// IDA: N/A the injected services of `update_particle_system_known`. The drag and gravity object transforms are not injected as in C++:
/// they are the +0x64 of the modifier's raw +0x18 object, read from its `NiAvWorld`.
#[derive(Default)]
pub struct ParticleSystemRuntime<'a> {
    pub reset_services: ParticleResetService<'a>,
    pub draw_rand: Option<&'a mut dyn FnMut() -> i32>,
    pub spawn_services: Option<&'a mut dyn ParticleSpawnServices>,
}

// IDA: N/A a helper, with no address of its own.
fn clone_component<C: Component + Clone>(world: &World, owner: Entity) -> Result<C, GameError> {
    world
        .get::<C>(owner)
        .cloned()
        .ok_or(GameError::unclassified(
            0,
            "particle modifier component missing",
        ))
}

// IDA: N/A a helper, with no address of its own.
fn object_world(
    world: &World,
    object: Option<Entity>,
    message: &'static str,
) -> Result<NiTransform, GameError> {
    object
        .and_then(|object| world.get::<NiAvWorld>(object))
        .map(|state| state.transform)
        .ok_or(GameError::from_legacy_message(message))
}

// IDA: N/A a helper, with no address of its own.
fn system_world_transform(world: &World, system: Entity) -> Result<NiTransform, GameError> {
    object_world(
        world,
        Some(system),
        "particle system world transform unresolved",
    )
}

// IDA: 007498F0 calls each active modifier's vtable +0x4C. Verified targets: 0075A250 color, 007555F0 position, 00756CA0 grow and fade,
// 0075BC50 age, 0075ACE0 bound, 00758980 drag, 0060CF60 spawn no-op, 0074DEB0 rotation, 00750BC0 gravity, 006FF0A0 parent velocity
// Completion: 100% of the typed dispatch for these owners, with explicit rand, spawn and reset services at the unresolved boundaries
// TODO: [ ] The original target and back-reference ABI, and remaining emitters.
pub fn update_particle_system_known(
    world: &mut World,
    system: Entity,
    current_time: f32,
    runtime: &mut ParticleSystemRuntime<'_>,
) -> Result<(), GameError> {
    let reset: ParticleResetService<'_> = match runtime.reset_services.as_mut() {
        Some(reset) => Some(&mut **reset),
        None => None,
    };
    let draw_rand = &mut runtime.draw_rand;
    let spawn_services = &mut runtime.spawn_services;
    let mut dispatch = |world: &mut World, owner: Entity, data: Entity, time: f32| {
        dispatch_known_modifier(world, system, owner, data, time, draw_rand, spawn_services)
    };
    update_particle_system(world, system, current_time, reset, Some(&mut dispatch))
}

// IDA: N/A no address of its own; its comments cite 0060CF60.
fn dispatch_known_modifier(
    world: &mut World,
    system: Entity,
    owner: Entity,
    data: Entity,
    time: f32,
    draw_rand: &mut Option<&mut dyn FnMut() -> i32>,
    spawn_services: &mut Option<&mut dyn ParticleSpawnServices>,
) -> Result<(), GameError> {
    match world.get::<NiClass>(owner).copied() {
        Some(NiClass::ParticleColorModifier) => {
            let state = world.get::<ParticleData>(data).expect("checked by caller");
            if state.colors.is_empty() {
                return Ok(());
            }
            let count = usize::from(state.active_count_48);
            if state.colors.len() < count || state.info_5c.len() < count {
                return Err(GameError::unclassified(
                    0,
                    "particle color arrays too short",
                ));
            }
            let samples: Vec<ParticleColorSample> = state.info_5c[..count]
                .iter()
                .map(|info| ParticleColorSample {
                    age: info.age_0c,
                    lifespan: info.lifespan_10,
                })
                .collect();
            let mut colors = state.colors[..count].to_vec();
            let modifier =
                world
                    .get::<ParticleColorModifier>(owner)
                    .ok_or(GameError::unclassified(
                        0,
                        "particle modifier component missing",
                    ))?;
            let keys = modifier
                .data_18
                .and_then(|keys| world.get::<ColorKeyData>(keys))
                .map(|keys| keys as &dyn ParticleColorKeys);
            modifier.update_colors(&samples, &mut colors, keys)?;
            world
                .get_mut::<ParticleData>(data)
                .expect("checked by caller")
                .colors[..count]
                .copy_from_slice(&colors);
            Ok(())
        }
        Some(NiClass::ParticlePositionModifier) => {
            let modifier: ParticlePositionModifier = clone_component(world, owner)?;
            with_data(world, data, |_, state| {
                modifier.update_positions(time, state)
            })
        }
        Some(NiClass::ParticleGrowFadeModifier) => {
            let modifier: ParticleGrowFadeModifier = clone_component(world, owner)?;
            with_data(world, data, |_, state| modifier.update_sizes(state))
        }
        Some(NiClass::ParticleAgeDeathModifier) => {
            let modifier: ParticleAgeDeathModifier = clone_component(world, owner)?;
            if modifier.spawn_on_death_18 && modifier.spawn_linked {
                let Some(services) = spawn_services.as_mut() else {
                    return Err(GameError::unclassified(
                        0,
                        "particle spawn services unresolved",
                    ));
                };
                with_data(world, data, |world, state| {
                    let spawn = modifier
                        .spawn_1c
                        .and_then(|spawn| world.get::<ParticleSpawnModifier>(spawn));
                    modifier.update_ages_with_spawn(time, state, spawn, &mut **services)
                })
            } else {
                with_data(world, data, |_, state| modifier.update_ages(time, state))
            }
        }
        Some(NiClass::ParticleBoundUpdateModifier) => {
            let mut modifier: ParticleBoundUpdateModifier = clone_component(world, owner)?;
            let result = with_data(world, data, |_, state| modifier.update_bound(time, state));
            *world
                .get_mut::<ParticleBoundUpdateModifier>(owner)
                .expect("cloned above") = modifier;
            result
        }
        Some(NiClass::ParticleDragModifier) => {
            let modifier: ParticleDragModifier = clone_component(world, owner)?;
            let active_count = world
                .get::<ParticleData>(data)
                .expect("checked by caller")
                .active_count_48;
            if !modifier.drag_object_linked
                || modifier.drag_coefficient_28.partial_cmp(&0.0)
                    != Some(std::cmp::Ordering::Greater)
                || active_count == 0
            {
                return Ok(());
            }
            let drag_world = object_world(
                world,
                modifier.drag_object_18.0,
                "particle drag object transform unresolved",
            )?;
            let system_world = system_world_transform(world, system)?;
            with_data(world, data, |_, state| {
                modifier.update_velocities(time, state, &drag_world, &system_world)
            })
        }
        // All four +0x4C slots are the same 0060CF60 nullsub_1. Their +0x5C emission and spawn dispatch runs separately.
        Some(
            NiClass::ParticleSpawnModifier
            | NiClass::ParticleBoxEmitter
            | NiClass::ParticleSphereEmitter
            | NiClass::ParticleCylinderEmitter
            | NiClass::ParticleMeshEmitter,
        ) => Ok(()),
        Some(NiClass::ParticleRotationModifier) => {
            let modifier: ParticleRotationModifier = clone_component(world, owner)?;
            with_data(world, data, |_, state| {
                modifier.update_particles(state, time)
            })
        }
        Some(NiClass::ParticleGravityModifier) => {
            let modifier: ParticleGravityModifier = clone_component(world, owner)?;
            let active_count = world
                .get::<ParticleData>(data)
                .expect("checked by caller")
                .active_count_48;
            if modifier.gravity_object_index == NULL_REFERENCE || active_count == 0 {
                return Ok(());
            }
            let gravity_world = object_world(
                world,
                modifier.gravity_object_18.0,
                "particle gravity object transform unresolved",
            )?;
            let system_world = system_world_transform(world, system)?;
            let rand: super::particle_gravity::GravityRand<'_> = match draw_rand.as_mut() {
                Some(rand) => Some(&mut **rand),
                None => None,
            };
            with_data(world, data, |_, state| {
                modifier.update_particles(state, time, &gravity_world, &system_world, rand)
            })
        }
        Some(NiClass::BsParentVelocityModifier) => {
            // IDA: A7D53C +0x4C 006FF0A0 reads target +0x10 (this system) +0x1C parent, then the parent's +0x88 world translation. A
            // stored but expired host parent stops.
            let parent = world.get::<NiAvParent>(system).and_then(|parent| parent.0);
            let parent_translate = match parent {
                Some(parent) => Some(
                    world
                        .get::<NiAvWorld>(parent)
                        .ok_or(GameError::unclassified(
                            0,
                            "particle system parent owner expired",
                        ))?
                        .transform
                        .translation,
                ),
                None => None,
            };
            let mut modifier: BsParentVelocityModifier = clone_component(world, owner)?;
            let result = with_data(world, data, |_, state| {
                modifier.update(time, state, parent_translate)
            });
            *world
                .get_mut::<BsParentVelocityModifier>(owner)
                .expect("cloned above") = modifier;
            result
        }
        _ => Ok(()),
    }
}

// IDA: 00749510 reads NiParticleSystem's +0xC8 list head and +0xB4 data, then invokes every modifier's vtable +0x50 with the data and the
// new index, without testing +0x14 active
// Completion: 100% of the unconditional traversal and typed target association; the callback keeps the concrete virtual effects
// TODO: [ ] Bind all concrete modifier +0x50 implementations and returns.
pub fn initialize_added_particle(
    world: &mut World,
    system: Entity,
    index: u16,
    mut dispatch: Option<
        &mut (dyn FnMut(&mut World, Entity, Entity, u16) -> Result<(), GameError> + '_),
    >,
) -> Result<(), GameError> {
    let data = system_data(world, system)?;
    if index
        >= world
            .get::<ParticleData>(data)
            .expect("checked by system_data")
            .vertex_count
    {
        return Err(GameError::unclassified(
            0,
            "particle index outside data capacity",
        ));
    }
    let owners: Vec<Entity> = world
        .get::<ParticleSystem>(system)
        .expect("checked by system_data")
        .modifiers_c4
        .iter()
        .map(|row| row.owner)
        .collect();
    if !owners.is_empty() && dispatch.is_none() {
        return Err(GameError::unclassified(
            0,
            "particle initialization dispatcher unresolved",
        ));
    }
    for owner in owners {
        let dispatch = dispatch.as_mut().expect("checked above");
        dispatch(world, owner, data, index)?;
    }
    Ok(())
}

// IDA: 00749510 dispatches +0x50 in list order without testing +0x14 active. The vtable mapping for supported types is 0074DFA0 rotation,
// 006FEF50 parent velocity, and 0060CF60 nullsub_1 for color, position, grow and fade, age and death, bound, drag, spawn, gravity and
// the shape emitters
// Completion: 100% of the mapped supported behaviour
// TODO: [ ] Extend when another concrete NiPSysModifier owner is reached.
pub fn initialize_added_particle_known(
    world: &mut World,
    system: Entity,
    index: u16,
    mut draw_rand: Option<&mut (dyn FnMut() -> i32 + '_)>,
) -> Result<(), GameError> {
    let mut dispatch = |world: &mut World, owner: Entity, data: Entity, particle: u16| match world
        .get::<NiClass>(owner)
        .copied()
    {
        Some(NiClass::ParticleRotationModifier) => {
            let modifier: ParticleRotationModifier = clone_component(world, owner)?;
            let rand: Option<&mut (dyn FnMut() -> i32 + '_)> = match draw_rand.as_mut() {
                Some(rand) => Some(&mut **rand),
                None => None,
            };
            with_data(world, data, |_, state| {
                initialize_rotation(&modifier, state, particle, rand)
            })
        }
        Some(NiClass::BsParentVelocityModifier) => {
            let modifier: BsParentVelocityModifier = clone_component(world, owner)?;
            with_data(world, data, |_, state| {
                modifier.initialize_particle(state, particle)
            })
        }
        _ => Ok(()),
    };
    initialize_added_particle(world, system, index, Some(&mut dispatch))
}

// IDA: N/A adapts a shorter-lived rand borrow to `RotationRand`, whose trait object carries the borrow's lifetime
// Completion: 100%
fn initialize_rotation(
    modifier: &ParticleRotationModifier,
    state: &mut ParticleData,
    particle: u16,
    mut draw_rand: Option<&mut (dyn FnMut() -> i32 + '_)>,
) -> Result<(), GameError> {
    let mut forward;
    let rand: super::particle_rotation::RotationRand<'_> = match draw_rand.as_mut() {
        Some(rand) => {
            forward = || rand();
            Some(&mut forward)
        }
        None => None,
    };
    modifier.initialize_particle(state, particle, rand)
}

#[cfg(test)]
mod tests {
    use super::super::registration::{FactoryCreate, NiFactoryRegistry};
    use super::*;

    const MODERN: u32 = 0x1400_0004;
    const OLDER: u32 = 0x0A01_0072;

    fn u32_bytes(bytes: &mut Vec<u8>, value: u32) {
        bytes.extend(value.to_le_bytes());
    }

    fn name_bytes(bytes: &mut Vec<u8>, value: &str) {
        u32_bytes(bytes, value.len() as u32);
        bytes.extend(value.as_bytes());
    }

    /// Ported from `particle_system_tests`' `verify_tail_and_cold`.
    #[test]
    fn tail_and_cold() {
        let mut world = World::new();
        let mut cold = ParticleSystem::default();
        assert!(cold.in_world_space_c0);
        assert!(cold.byte_e4);
        assert_eq!(cold.system_update_time_e8, -f32::MAX);
        assert!(!cold.reset_requested_ec);
        assert_eq!(cold.data_b4, None);
        assert!(cold.modifiers_c4.is_empty());
        assert!(cold.pending_modifier_indices.is_empty());
        assert_eq!(ParticleSystem::modifier_bucket_count(), 37);

        let mut registry = NiFactoryRegistry::default();
        registry.insert_or_assign("NiParticleSystem", 0x0074_9FE0);
        let FactoryCreate::Created { entity, .. } = registry.create(&mut world, "NiParticleSystem")
        else {
            panic!("the cold factory did not create a typed owner");
        };
        assert!(world.get::<ParticleSystem>(entity).is_some());

        let mut bytes = vec![0u8];
        u32_bytes(&mut bytes, 2);
        u32_bytes(&mut bytes, 3);
        u32_bytes(&mut bytes, NULL_REFERENCE);
        let size = bytes.len();
        bytes.push(0xAA);
        let tail = parse_particle_system_tail(&bytes, OLDER).unwrap();
        assert_eq!(tail.bytes_consumed, size);
        assert!(!tail.in_world_space);
        assert_eq!(tail.modifier_indices, [3, NULL_REFERENCE]);
        for length in 0..size {
            assert!(parse_particle_system_tail(&bytes[..length], OLDER).is_err());
        }
        assert!(parse_particle_system_tail(&bytes, 0x0A01_0071).is_err());
        cold.apply_loaded_tail(&tail).unwrap();
        assert!(!cold.in_world_space_c0);
        assert_eq!(cold.pending_modifier_indices, tail.modifier_indices);
    }

    fn full_payload_bytes() -> (Vec<u8>, usize) {
        let mut bytes = Vec::new();
        name_bytes(&mut bytes, "System");
        u32_bytes(&mut bytes, 1);
        u32_bytes(&mut bytes, 9); // one ObjectNET extra link
        u32_bytes(&mut bytes, 8); // controller link
        bytes.extend(0xFFFFu16.to_le_bytes()); // raw AV flags
        for index in 0..13 {
            u32_bytes(&mut bytes, if index == 12 { 0x4000_0000 } else { 0 });
        }
        u32_bytes(&mut bytes, 2);
        u32_bytes(&mut bytes, 1);
        u32_bytes(&mut bytes, NULL_REFERENCE); // properties
        u32_bytes(&mut bytes, 3); // collision
        u32_bytes(&mut bytes, 4); // geometry data
        u32_bytes(&mut bytes, 5); // skin
        bytes.push(2); // nonzero shader flag
        name_bytes(&mut bytes, "Shader");
        u32_bytes(&mut bytes, 0x1122_3344);
        bytes.push(0); // local system
        u32_bytes(&mut bytes, 2);
        u32_bytes(&mut bytes, 6);
        u32_bytes(&mut bytes, 7); // modifiers
        let used = bytes.len();
        bytes.push(0xAA);
        (bytes, used)
    }

    fn rotation(world: &mut World, name: &str, order: u32, active: bool) -> Entity {
        let mut modifier = ParticleRotationModifier::default();
        modifier.base.name_08 = Some(name.as_bytes().to_vec());
        modifier.base.order_0c = order;
        modifier.base.active_14 = active;
        world
            .spawn((NiClass::ParticleRotationModifier, modifier))
            .id()
    }

    /// Ported from `particle_system_tests`' `verify_full_payload`.
    #[test]
    fn full_payload() {
        let (bytes, used) = full_payload_bytes();
        let modern = parse_particle_system_payload(&bytes, MODERN).unwrap();
        let older = parse_particle_system_payload(&bytes, OLDER).unwrap();
        assert_eq!(modern.bytes_consumed, used);
        assert_eq!(older.bytes_consumed, used);
        assert_eq!(modern.net.name.as_deref(), Some(&b"System"[..]));
        assert_eq!(modern.net.extra_indices, [9]);
        assert_eq!(modern.net.controller_index, 8);
        assert_eq!(modern.raw_av_flags, 0xFFFF);
        assert_eq!(modern.effective_av_flags, 0xFFEF);
        assert_eq!(older.effective_av_flags, 0xFFAF);
        assert_eq!(modern.local_transform.scale, 2.0);
        assert_eq!(modern.property_indices, [1, NULL_REFERENCE]);
        assert_eq!(modern.collision_index, 3);
        assert_eq!(modern.geometry_data_index, 4);
        assert_eq!(modern.skin_index, 5);
        assert_eq!(modern.shader_flag_raw, 2);
        let shader = modern.shader_payload.as_ref().unwrap();
        assert_eq!(shader.name.as_deref(), Some(&b"Shader"[..]));
        assert_eq!(shader.shader_field_c, 0x1122_3344);
        assert!(!modern.tail.in_world_space);
        assert_eq!(modern.tail.modifier_indices, [6, 7]);
        for length in 0..used {
            assert!(parse_particle_system_payload(&bytes[..length], MODERN).is_err());
        }

        let mut system = ParticleSystem::default();
        system.apply_loaded_payload(&modern).unwrap();
        assert_eq!(system.net.name.as_deref(), Some(&b"System"[..]));
        assert_eq!(system.raw_av_flags, 0xFFFF);
        assert_eq!(system.effective_av_flags_18, 0xFFEF);
        assert_eq!(system.local_transform_30.scale, 2.0);
        assert_eq!(system.property_indices, modern.property_indices);
        assert_eq!(system.collision_index, 3);
        assert_eq!(system.geometry_data_index, 4);
        assert_eq!(system.skin_index, 5);
        assert!(system.shader_payload.is_some());
        assert_eq!(
            system.pending_modifier_indices,
            modern.tail.modifier_indices
        );

        let mut supported = modern;
        supported.net.extra_indices.clear();
        supported.net.controller_index = NULL_REFERENCE;
        supported.property_indices.clear();
        supported.collision_index = NULL_REFERENCE;
        supported.geometry_data_index = 0;
        supported.skin_index = NULL_REFERENCE;
        supported.shader_flag_raw = 0;
        supported.shader_payload = None;
        supported.tail.modifier_indices = vec![2, 1]; // raw order differs from schedule order

        let mut world = World::new();
        let mut state = ParticleData {
            vertex_count: 1,
            active_count_48: 1,
            rotation_angles_54: vec![0.0],
            rotation_speeds_60: vec![1.0],
            ..ParticleData::default()
        };
        state.positions.resize(1, [0.0; 3]);
        state.info_5c.resize(1, Default::default());
        let data_owner = world.spawn((NiClass::ParticleData, state)).id();
        let later = rotation(&mut world, "Later", 2, true);
        let earlier = rotation(&mut world, "Earlier", 1, true);
        let mut table = StreamObjectTable::default();
        table.push("NiPSysData", 0x0075_9C50, data_owner);
        table.push("NiPSysRotationModifier", 0x0074_E1C0, later);
        table.push("NiPSysRotationModifier", 0x0074_E1C0, earlier);

        let linked = spawn_particle_system(&mut world);
        world
            .get_mut::<ParticleSystem>(linked)
            .unwrap()
            .apply_loaded_payload(&supported)
            .unwrap();
        assert_eq!(
            resolve_particle_system_links(&mut world, linked, &supported, &table).unwrap(),
            ParticleSystemLinkStatus::Linked
        );
        let state = world.get::<ParticleSystem>(linked).unwrap();
        assert_eq!(state.data_b4, Some(data_owner));
        assert_eq!(state.modifiers_c4.len(), 2);
        assert_eq!(state.modifiers_c4[0].original_name, b"Earlier");
        assert_eq!(state.modifiers_c4[1].original_name, b"Later");
        assert_eq!(state.pending_modifier_indices, [2, 1]);
        update_particle_system_known(
            &mut world,
            linked,
            1.0,
            &mut ParticleSystemRuntime::default(),
        )
        .unwrap();
        let data = world.get::<ParticleData>(data_owner).unwrap();
        assert_eq!(data.rotation_angles_54[0], 2.0);
        assert_eq!(
            world
                .get::<ParticleSystem>(linked)
                .unwrap()
                .system_update_time_e8,
            1.0
        );
        assert_eq!(data.geometry_extra_flags_2e & 7, 7);

        let unresolved = spawn_particle_system(&mut world);
        world
            .get_mut::<ParticleSystem>(unresolved)
            .unwrap()
            .apply_loaded_payload(&supported)
            .unwrap();
        let mut bad = supported.clone();
        bad.net.controller_index = 1;
        assert!(resolve_particle_system_links(&mut world, unresolved, &bad, &table).is_err());
        let state = world.get::<ParticleSystem>(unresolved).unwrap();
        assert_eq!(state.data_b4, None);
        assert!(state.modifiers_c4.is_empty());
        bad = supported.clone();
        bad.geometry_data_index = 1;
        assert!(resolve_particle_system_links(&mut world, unresolved, &bad, &table).is_err());
        assert_eq!(
            world.get::<ParticleSystem>(unresolved).unwrap().data_b4,
            None
        );
        bad = supported;
        bad.tail.modifier_indices = vec![3];
        assert!(resolve_particle_system_links(&mut world, unresolved, &bad, &table).is_err());
        assert!(
            world
                .get::<ParticleSystem>(unresolved)
                .unwrap()
                .modifiers_c4
                .is_empty()
        );
    }

    /// Ported from `particle_system_tests`' `verify_attachment_and_update`.
    #[test]
    fn attachment_and_update() {
        let mut world = World::new();
        let system = spawn_particle_system(&mut world);
        let late = rotation(&mut world, "Late", 0xFFFF_FFFF, true);
        let first = rotation(&mut world, "aaaaaaa", 1, true);
        let tied = rotation(&mut world, "Tie", 1, false);
        let duplicate = rotation(&mut world, "AaAaaaa", 2, true);
        for modifier in [late, first, tied, duplicate] {
            attach_particle_modifier(&mut world, system, modifier).unwrap();
        }
        let state = world.get::<ParticleSystem>(system).unwrap();
        assert_eq!(state.modifiers_c4.len(), 4);
        assert_eq!(state.modifiers_c4[0].original_name, b"aaaaaaa");
        assert_eq!(state.modifiers_c4[1].original_name, b"Tie");
        assert_eq!(state.modifiers_c4[2].original_name, b"AaAaaaa");
        assert_eq!(state.modifiers_c4[3].original_order, 0xFFFF_FFFF);
        assert_eq!(state.find_modifier(b"aaaaaaa"), Some(duplicate));
        assert_eq!(state.find_modifier(b"AAAAAAA"), None);

        let mut data_state = ParticleData {
            vertex_count: 2,
            active_count_48: 1,
            added_base: 1,
            added_count: 1,
            ..ParticleData::default()
        };
        data_state.positions.resize(2, [0.0; 3]);
        data_state.info_5c.resize(2, Default::default());
        let data = world.spawn((NiClass::ParticleData, data_state)).id();
        world.get_mut::<ParticleSystem>(system).unwrap().data_b4 = Some(data);

        let name_of = |world: &World, owner: Entity| -> String {
            String::from_utf8(
                world
                    .get::<ParticleRotationModifier>(owner)
                    .unwrap()
                    .base
                    .name_08
                    .clone()
                    .unwrap(),
            )
            .unwrap()
        };
        let mut initialized = Vec::new();
        let mut listener = |world: &mut World, owner: Entity, current: Entity, index: u16| {
            assert_eq!(current, data);
            assert_eq!(index, 1);
            initialized.push(name_of(world, owner));
            Ok(())
        };
        initialize_added_particle(&mut world, system, 1, Some(&mut listener)).unwrap();
        assert_eq!(initialized, ["aaaaaaa", "Tie", "AaAaaaa", "Late"]);
        initialize_added_particle_known(&mut world, system, 1, None).unwrap();

        let inactive_system = spawn_particle_system(&mut world);
        let inactive_data = world
            .spawn((
                NiClass::ParticleData,
                ParticleData {
                    vertex_count: 1,
                    rotation_angles_54: vec![0.0],
                    rotation_speeds_60: vec![0.0],
                    ..ParticleData::default()
                },
            ))
            .id();
        world
            .get_mut::<ParticleSystem>(inactive_system)
            .unwrap()
            .data_b4 = Some(inactive_data);
        let inactive = rotation(&mut world, "Inactive", 0, false);
        {
            let mut modifier = world.get_mut::<ParticleRotationModifier>(inactive).unwrap();
            modifier.random_initial_axis = false;
            modifier.initial_angle = 3.0;
            modifier.initial_speed = 4.0;
        }
        attach_particle_modifier(&mut world, inactive_system, inactive).unwrap();
        let mut draws = 0;
        let mut draw = || {
            draws += 1;
            0
        };
        initialize_added_particle_known(&mut world, inactive_system, 0, Some(&mut draw)).unwrap();
        assert_eq!(draws, 2);
        let state = world.get::<ParticleData>(inactive_data).unwrap();
        assert_eq!(state.rotation_angles_54[0], 3.0);
        assert_eq!(state.rotation_speeds_60[0], 4.0);

        let mut calls = Vec::new();
        let mut dispatch = |world: &mut World, owner: Entity, current: Entity, time: f32| {
            assert_eq!(current, data);
            assert_eq!(
                world.get::<ParticleData>(current).unwrap().active_count_48,
                2
            );
            assert_eq!(time, 3.0);
            calls.push(name_of(world, owner));
            Ok(())
        };
        update_particle_system(&mut world, system, 3.0, None, Some(&mut dispatch)).unwrap();
        let state = world.get::<ParticleData>(data).unwrap();
        assert_eq!(state.added_count, 0);
        assert_eq!(state.active_count_48, 2);
        assert_eq!(calls, ["aaaaaaa", "AaAaaaa", "Late"]);
        assert_eq!(state.geometry_extra_flags_2e & 7, 7);
        assert_eq!(
            world
                .get::<ParticleSystem>(system)
                .unwrap()
                .system_update_time_e8,
            3.0
        );

        let mut reset_called = false;
        let mut reset = |_: &mut World, current: Entity| {
            assert_eq!(current, data);
            reset_called = true;
        };
        let mut ignore = |_: &mut World, _: Entity, _: Entity, _: f32| Ok(());
        update_particle_system(&mut world, system, 2.0, Some(&mut reset), Some(&mut ignore))
            .unwrap();
        assert!(reset_called);
        assert_eq!(
            world
                .get::<ParticleSystem>(system)
                .unwrap()
                .system_update_time_e8,
            2.0
        );
    }

    /// IDA: 006FAB60 through the retained identities: a noncanonical byte and a stranger are refused. The C++ reach these only through
    /// the scheduler.
    #[test]
    fn modifier_active_bytes() {
        let mut world = World::new();
        let system = spawn_particle_system(&mut world);
        let modifier = rotation(&mut world, "Rot", 0, true);
        let stranger = rotation(&mut world, "Other", 0, true);
        attach_particle_modifier(&mut world, system, modifier).unwrap();
        let state = world.get::<ParticleSystem>(system).unwrap();
        assert_eq!(state.modifier_active_byte(&world, modifier), Some(1));
        assert_eq!(state.modifier_active_byte(&world, stranger), None);
        assert!(set_modifier_active_byte(&mut world, system, modifier, 2).is_err());
        assert!(!set_modifier_active_byte(&mut world, system, stranger, 0).unwrap());
        assert!(set_modifier_active_byte(&mut world, system, modifier, 0).unwrap());
        assert!(
            !world
                .get::<ParticleRotationModifier>(modifier)
                .unwrap()
                .base
                .active_14
        );
    }
}
