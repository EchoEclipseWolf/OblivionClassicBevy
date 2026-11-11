//! BSXFlags and BSParentVelocityModifier, the two Bethesda NIF objects reached before first idle (P03-RUN-005).
//! x87: every float operation here is a 24-bit site (main thread after CreateDevice, control word 007F, P04-RUN-002): plain
//! `f32` in the instructions' grouping. Qword constants hold exact values, so `(f64::from(x) op C) as f32` is one rounding.

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;
use oblivion_core::reader::ByteReader;

use crate::gamebryo::object_net::NiExtraDataName;
use crate::gamebryo::particle_data::ParticleData;
use crate::gamebryo::particle_dynamics::{
    MODIFIER_BASE_TEXTS, ParticleModifierBase, ParticleModifierPending,
    parse_particle_modifier_payload,
};
use crate::gamebryo::stream_objects::{NULL_REFERENCE, StreamObjectTable};

const TRUNCATED_U32: &str = "truncated BS object u32";

/// IDA: B3F9A8 NiPoint3 zero (BSS, no static writer; live 0,0,0).
const ZERO_POINT: [f32; 3] = [0.0; 3];

/// IDA: 006FA900 factory: FormHeap 0x10, 006FA820 constructor (NiExtraData base, +0x0C = 0, vtable A7D14C, SetName "BSX"), then the
/// factory itself calls NiExtraData_SetName(null), so a fresh factory object has no name. RTTI B3F484 names the class "BSXFlags" (read
/// live in P03-RUN-005), so an unnamed object linked into ObjectNET gets a bare "ED%03d" name (006FF8A0 finds no "ExtraData" substring).
/// The name is the entity's `NiExtraDataName`. Completion: 100% of the cold state, modern load and the no-op link.
/// TODO: [ ] The BSX flag consumers (bAnimated, bHavok, bRagDoll, bComplex, bFlame per 006FA9A0) in the later TES and scene phases.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BsxFlagsExtraData {
    /// IDA: NiIntegerExtraData +0x0C.
    pub flags_0c: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BsxFlagsPayload {
    pub name: Option<Vec<u8>>,
    pub flags: u32,
    pub bytes_consumed: usize,
}

// IDA: 006FA790 -> 00730A50: 00721610 NiExtraData::LoadBinary (>= 0500000B reads the 00713620 owned name; older streams read a deferred
// next-extra reference and an optional raw block), then one u32 into +0x0C
// Completion: 100% of the modern >= 0500000B bounded order
// TODO: [ ] The < 0500000B legacy branch.
pub fn parse_bsx_flags_payload(
    bytes: &[u8],
    stream_version: u32,
) -> Result<BsxFlagsPayload, GameError> {
    if stream_version < 0x0500_000B {
        return Err(GameError::unclassified(
            0,
            "legacy BSXFlags payload unsupported",
        ));
    }
    let mut reader = ByteReader::new(bytes);
    let length = reader.u32(TRUNCATED_U32)? as i32;
    let name = if length <= 0 {
        None
    } else {
        Some(
            reader
                .take(length as usize, "truncated BS object name")?
                .to_vec(),
        )
    };
    let flags = reader.u32(TRUNCATED_U32)?;
    Ok(BsxFlagsPayload {
        name,
        flags,
        bytes_consumed: reader.position(),
    })
}

// IDA: 00730A50 publication order: name (00721610), then +0x0C. The 006FAEC0 -> 00721550 link reads nothing at >= 0500000B, so nothing is
// deferred
// Completion: 100% of the host field order
pub fn load_bsx_flags(
    world: &mut World,
    entity: Entity,
    pending: &BsxFlagsPayload,
) -> Result<(), GameError> {
    let mut extra = world
        .get_entity_mut(entity)
        .map_err(|_| GameError::unclassified(0, "BSXFlags component missing"))?;
    extra.insert((
        NiExtraDataName(pending.name.clone()),
        BsxFlagsExtraData {
            flags_0c: pending.flags,
        },
    ));
    Ok(())
}

/// IDA: 006FF000 BSParentVelocityModifier factory: FormHeap 0x3C, 006FEEE0 over 00752BF0 NiPSysModifier (name null, order 0, target null,
/// active 1), vtable A7D53C, then order +0x0C = 3000, +0x24 last parent position and +0x30 velocity both copied from B3F9A8, +0x20 damping
/// = 0 and +0x18 delta = 0. The +0x1C last time is not initialized by the constructor; Update writes it before its first read.
/// Completion: 100% of the cold state, modern load, Update (+0x4C 006FF0A0), particle init (+0x50 006FEF50) and ResetParent (+0x54
/// 006FEFE0).
/// TODO: [ ] Validate x87 results against an original runtime sample.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct BsParentVelocityModifier {
    pub base: ParticleModifierBase,
    pub last_delta_time_18: f32,
    /// IDA: +0x1C, uninitialized until Update.
    pub last_time_1c: Option<f32>,
    pub damping_20: f32,
    pub last_parent_position_24: [f32; 3],
    pub velocity_30: [f32; 3],
}

impl Default for BsParentVelocityModifier {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            base: ParticleModifierBase {
                order_0c: 3000,
                ..ParticleModifierBase::default()
            },
            last_delta_time_18: 0.0,
            last_time_1c: None,
            damping_20: 0.0,
            last_parent_position_24: ZERO_POINT,
            velocity_30: ZERO_POINT,
        }
    }
}

// IDA: N/A (site unresolved: Vec3_EqualsExact, a vector compare BSParentVelocityModifier's arms call, of the same shape as
// 0072A0A0 NiBound_Equals): x, y, z exact ==, the right operand first
// Completion: 100% of the verified order
fn equals_exact(left: &[f32; 3], right: &[f32; 3]) -> bool {
    right[0] == left[0] && right[1] == left[1] && right[2] == left[2]
}

impl BsParentVelocityModifier {
    // IDA: 006FF310 LoadBinary publication: 00752DC0 base (name, order, deferred target, active byte), then the damping f32 into +0x20
    // Completion: 100% of the host field order
    pub fn apply_loaded(&mut self, base: &ParticleModifierPending, damping: f32) {
        self.base = base.published();
        self.damping_20 = damping;
    }

    // IDA: 006FEFE0 vtable +0x54 ResetParent: +0x24 = B3F9A8 (zero)
    // Completion: 100% of the verified store
    pub const fn reset_parent(&mut self) {
        self.last_parent_position_24 = ZERO_POINT;
    }

    // IDA: 006FF0A0 vtable +0x4C Update(time, data). With a zero +0x48 active count it returns. A null target +0x1C parent calls +0x54 and
    // returns. When +0x24 equals B3F9A8 exactly: velocity = B3F9A8, +0x1C = time, +0x24 = parent world translate (+0x88). Then dt = time -
    // +0x1C (f32); unless dt == 0 (an unordered NaN takes the compute path), the new velocity is ((cur - last) * damping) * (1 / dt) per
    // component (f32 stores), the difference from the old velocity is added to every active particle's velocity when it is not exactly zero
    // (008AA390), and +0x30 takes the new velocity. Finally +0x18 = dt, +0x24 = cur, +0x1C = time. `parent_translate` is the parent's +0x88
    // world translation, or `None` for a null +0x1C
    // Completion: 100% of the verified store order
    // TODO: [ ] Compare results against an original run. Precision: 24-bit per step on the main thread after CreateDevice.
    pub fn update(
        &mut self,
        time: f32,
        data: &mut ParticleData,
        parent_translate: Option<[f32; 3]>,
    ) -> Result<(), GameError> {
        if data.active_count_48 == 0 {
            return Ok(());
        }
        let Some(current) = parent_translate else {
            self.reset_parent(); // vtable +0x54
            return Ok(());
        };
        if equals_exact(&self.last_parent_position_24, &ZERO_POINT) {
            self.velocity_30 = ZERO_POINT;
            self.last_time_1c = Some(time); // 006FF100 fst [+1C]
            self.last_parent_position_24 = current;
        }
        let count = usize::from(data.active_count_48); // 006FF138 reads +0x48 before the loop
        // The constructor and +0x54 leave +0x24 at B3F9A8, so +0x1C is always written above before this read; an unset host value is a logic
        // error.
        let last_time = self.last_time_1c.ok_or(GameError::unclassified(
            0,
            "BSParentVelocityModifier last time unset",
        ))?;
        let delta = time - last_time;
        if delta != 0.0 {
            let last = self.last_parent_position_24;
            let moved = [
                current[0] - last[0],
                current[1] - last[1],
                current[2] - last[2],
            ];
            let damping = self.damping_20;
            let scaled = [moved[0] * damping, moved[1] * damping, damping * moved[2]];
            let inverse = 1.0 / delta;
            let next = [
                scaled[0] * inverse,
                scaled[1] * inverse,
                inverse * scaled[2],
            ];
            let change = [
                next[0] - self.velocity_30[0],
                next[1] - self.velocity_30[1],
                next[2] - self.velocity_30[2],
            ];
            // 008AA390: any component != the zero point.
            if !equals_exact(&change, &ZERO_POINT) && count != 0 {
                if data.info_5c.len() < count {
                    return Err(GameError::unclassified(
                        0,
                        "particle info array shorter than active count",
                    ));
                }
                for info in &mut data.info_5c[..count] {
                    let velocity = &mut info.velocity_00;
                    velocity[0] += change[0];
                    velocity[1] += change[1];
                    velocity[2] += change[2];
                }
            }
            self.velocity_30 = next;
        }
        self.last_delta_time_18 = delta;
        self.last_parent_position_24 = current;
        self.last_time_1c = Some(time);
        Ok(())
    }

    // IDA: 006FEF50 vtable +0x50 (data, u16 index): velocity[index] += +0x30 (x, y, z), then position[index] -= +0x30 * +0x18 (each product
    // stored f32 first)
    // Completion: 100% of the verified operand and store order
    pub fn initialize_particle(
        &self,
        data: &mut ParticleData,
        index: u16,
    ) -> Result<(), GameError> {
        let slot = usize::from(index);
        if slot >= data.info_5c.len() || slot >= data.positions.len() {
            return Err(GameError::unclassified(
                0,
                "particle index outside data arrays",
            ));
        }
        let velocity = &mut data.info_5c[slot].velocity_00;
        velocity[0] += self.velocity_30[0];
        velocity[1] += self.velocity_30[1];
        velocity[2] += self.velocity_30[2];
        let delta = self.last_delta_time_18;
        let offset = [
            self.velocity_30[0] * delta,
            self.velocity_30[1] * delta,
            delta * self.velocity_30[2],
        ];
        let position = &mut data.positions[slot];
        position[0] -= offset[0];
        position[1] -= offset[1];
        position[2] -= offset[2];
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct BsParentVelocityModifierPayload {
    pub base: ParticleModifierPending,
    pub damping: f32,
    pub bytes_consumed: usize,
}

// IDA: 006FF310: 00752DC0 NiPSysModifier base, then one f32
// Completion: 100% of the modern bounded field order
// TODO: [ ] Original NiStream short-read callbacks.
pub fn parse_bs_parent_velocity_payload(
    bytes: &[u8],
    stream_version: u32,
) -> Result<BsParentVelocityModifierPayload, GameError> {
    let base = parse_particle_modifier_payload(bytes, stream_version, &MODIFIER_BASE_TEXTS)?;
    let mut reader = ByteReader::new(bytes);
    reader.take(base.bytes_consumed, TRUNCATED_U32)?;
    let damping = reader.f32(TRUNCATED_U32)?;
    Ok(BsParentVelocityModifierPayload {
        base,
        damping,
        bytes_consumed: reader.position(),
    })
}

// IDA: 006FF310 publishes the base and damping during LoadBinary; the 00752CB0 base link consumes the target like the other modifiers
// Completion: 100% of the C++ graph arm
pub fn load_bs_parent_velocity(
    world: &mut World,
    entity: Entity,
    pending: &BsParentVelocityModifierPayload,
    table: &StreamObjectTable,
) -> Result<(), GameError> {
    let mut modifier =
        world
            .get_mut::<BsParentVelocityModifier>(entity)
            .ok_or(GameError::unclassified(
                0,
                "particle modifier component missing",
            ))?;
    modifier.apply_loaded(&pending.base, pending.damping);
    if pending.base.target_index != NULL_REFERENCE
        && pending.base.target_index as usize >= table.objects.len()
    {
        return Err(GameError::unclassified(
            0,
            "particle modifier target link out of range",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gamebryo::object_net::{
        ObjectNetPending, object_net_extras_supported, resolve_object_net_extra_links,
    };
    use crate::gamebryo::registration::{FactoryCreate, NiFactoryRegistry};

    /// Ported from `scene_graph_tests`' `verify_bsx_flags`.
    #[test]
    fn bsx_flags() {
        let mut bytes = Vec::new();
        bytes.extend(3u32.to_le_bytes());
        bytes.extend(b"BSX");
        bytes.extend(0x0000_000Bu32.to_le_bytes());
        let parsed = parse_bsx_flags_payload(&bytes, 0x1400_0004).unwrap();
        assert_eq!(parsed.bytes_consumed, bytes.len());
        assert_eq!(parsed.name.as_deref(), Some(&b"BSX"[..]));
        assert_eq!(parsed.flags, 0x0B);

        let mut world = World::new();
        let mut registry = NiFactoryRegistry::default();
        registry.register_winmain_bs_factories();
        let FactoryCreate::Created { entity, .. } = registry.create(&mut world, "BSXFlags") else {
            panic!("BSXFlags did not create");
        };
        assert_eq!(
            world.get::<NiExtraDataName>(entity),
            Some(&NiExtraDataName(None))
        );
        let mut table = StreamObjectTable::default();
        table.push("BSXFlags", 0x006F_A900, entity);
        let pending = ObjectNetPending {
            extra_indices: vec![0],
            ..ObjectNetPending::default()
        };
        assert!(object_net_extras_supported(&world, &pending, &table));
        let state = resolve_object_net_extra_links(&mut world, &pending, &table).unwrap();
        assert_eq!(state.extra_data.len(), 1);
        assert_eq!(
            world.get::<NiExtraDataName>(entity).unwrap().0.as_deref(),
            Some(&b"ED000"[..])
        );

        load_bsx_flags(&mut world, entity, &parsed).unwrap();
        assert_eq!(
            world.get::<BsxFlagsExtraData>(entity).unwrap().flags_0c,
            0x0B
        );
        assert_eq!(
            world.get::<NiExtraDataName>(entity).unwrap().0.as_deref(),
            Some(&b"BSX"[..])
        );
    }

    /// Ported from `scene_graph_tests`' `verify_parent_velocity_modifier`.
    #[test]
    fn parent_velocity_modifier() {
        let mut bytes = Vec::new();
        bytes.extend(0u32.to_le_bytes()); // null name
        bytes.extend(3000u32.to_le_bytes()); // order
        bytes.extend(NULL_REFERENCE.to_le_bytes()); // target
        bytes.push(1);
        bytes.extend(0.5f32.to_le_bytes());
        let parsed = parse_bs_parent_velocity_payload(&bytes, 0x1400_0004).unwrap();
        assert_eq!(parsed.bytes_consumed, bytes.len());
        assert_eq!(parsed.damping, 0.5);
        assert_eq!(parsed.base.order, 3000);

        let mut modifier = BsParentVelocityModifier::default();
        modifier.apply_loaded(&parsed.base, parsed.damping);
        let mut data = ParticleData {
            active_count_48: 1,
            ..ParticleData::default()
        };
        data.info_5c.resize(1, Default::default());
        data.positions.resize(1, [0.0; 3]);
        modifier
            .update(1.0, &mut data, Some([1.0, 0.0, 0.0]))
            .unwrap();
        assert_eq!(modifier.last_time_1c, Some(1.0));
        assert_eq!(modifier.last_delta_time_18, 0.0);
        assert_eq!(modifier.last_parent_position_24, [1.0, 0.0, 0.0]);
        assert_eq!(data.info_5c[0].velocity_00, [0.0, 0.0, 0.0]);
        modifier
            .update(2.0, &mut data, Some([3.0, 0.0, 0.0]))
            .unwrap();
        assert_eq!(modifier.velocity_30, [1.0, 0.0, 0.0]);
        assert_eq!(data.info_5c[0].velocity_00, [1.0, 0.0, 0.0]);
        assert_eq!(modifier.last_delta_time_18, 1.0);
        data.positions[0] = [5.0, 0.0, 0.0];
        modifier.initialize_particle(&mut data, 0).unwrap();
        assert_eq!(data.info_5c[0].velocity_00, [2.0, 0.0, 0.0]);
        assert_eq!(data.positions[0], [4.0, 0.0, 0.0]);

        let mut origin = BsParentVelocityModifier::default();
        origin.apply_loaded(&parsed.base, 1.0);
        origin.update(1.0, &mut data, Some([0.0; 3])).unwrap();
        origin.update(2.0, &mut data, Some([0.0; 3])).unwrap();
        assert_eq!(origin.last_delta_time_18, 0.0);
        assert_eq!(origin.last_time_1c, Some(2.0));
        origin.update(3.0, &mut data, None).unwrap();
        assert_eq!(origin.last_parent_position_24, [0.0, 0.0, 0.0]);
    }

    /// IDA: the C++ graph's target bounds check, and the cold order.
    #[test]
    fn load_checks_the_target_bound() {
        let mut world = World::new();
        let modifier = world.spawn(BsParentVelocityModifier::default()).id();
        assert_eq!(
            world
                .get::<BsParentVelocityModifier>(modifier)
                .unwrap()
                .base
                .order_0c,
            3000
        );
        let mut pending = BsParentVelocityModifierPayload {
            damping: 0.25,
            ..BsParentVelocityModifierPayload::default()
        };
        let table = StreamObjectTable::default();
        load_bs_parent_velocity(&mut world, modifier, &pending, &table).unwrap();
        pending.base.target_index = 0;
        assert!(load_bs_parent_velocity(&mut world, modifier, &pending, &table).is_err());
    }
}
