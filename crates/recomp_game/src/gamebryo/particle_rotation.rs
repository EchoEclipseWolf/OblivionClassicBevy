//! NiPSysRotationModifier.
//! x87: every float operation here is a 24-bit site (main thread after CreateDevice, control word 007F, P04-RUN-002): plain
//! `f32` in the instructions' grouping. Qword constants hold exact values, so `(f64::from(x) op C) as f32` is one rounding.

use bevy_ecs::prelude::*;
use recomp_core::reader::{ByteReader, StreamError};

use super::particle_data::ParticleData;
use super::particle_dynamics::{ParticleModifierBase, read_modifier_name};
use super::stream_objects::{NULL_REFERENCE, StreamObjectTable};

const TRUNCATED: &str = "truncated particle rotation payload";

/// IDA: 00A09B90 stores float(atan(1)) then float(*4) into B3F9A4; 00A09BD0 adds that stored float to itself into
/// B3F9A0. Completion: 100% of the recovered post-initializer bits.
/// TODO: [ ] Compare the target CRT's __CIatan rounding on a live initialized process.
const ENGINE_PI: f32 = f32::from_bits(0x4049_0FDB);
const ENGINE_TWO_PI: f32 = f32::from_bits(0x40C9_0FDB);

/// IDA: an injected `rand`. `None` stands for the C++ empty `std::function`.
pub type RotationRand<'a> = Option<&'a mut dyn FnMut() -> i32>;

/// IDA: 0074E1C0 NiPSysRotationModifier factory and the 00752BF0 base constructor: cold fields in host storage, not the
/// 0x38-byte ABI.
/// TODO: [ ] Bind the NiPSysModifier target, refcount and allocator.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct ParticleRotationModifier {
    pub base: ParticleModifierBase,
    pub initial_speed: f32,
    pub speed_variation: f32,
    pub initial_angle: f32,
    pub angle_variation: f32,
    pub initial_axis: [f32; 3],
    pub random_initial_axis: bool,
    pub random_speed_sign: bool,
}

impl Default for ParticleRotationModifier {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            base: ParticleModifierBase::default(),
            initial_speed: 0.0,
            speed_variation: 0.0,
            initial_angle: 0.0,
            angle_variation: 0.0,
            initial_axis: [1.0, 0.0, 0.0],
            random_initial_axis: true,
            random_speed_sign: false,
        }
    }
}

/// IDA: 0074DFA0 calls CRT rand and divides its integer return by 32767.0. Completion: 100% of the bounded return for
/// the CRT's RAND_MAX domain.
/// TODO: [ ] Connect the caller to the verified CRT rand state.
fn next_rand(draw_rand: &mut RotationRand<'_>) -> Result<i32, StreamError> {
    let draw = draw_rand
        .as_deref_mut()
        .ok_or(StreamError("particle rotation random source missing"))?;
    let result = draw();
    if !(0..=32767).contains(&result) {
        return Err(StreamError(
            "particle rotation random result outside CRT domain",
        ));
    }
    Ok(result)
}

impl ParticleRotationModifier {
    /// IDA: 0074E2B0 post-load writes the initial speed to the active rotation speeds only for stream version below
    /// 14000002 and a nonnull +0x60 array. Completion: 100% of the existing array's active prefix.
    /// TODO: [ ] Bind the stream post-load order; null target and malformed count behaviour.
    pub fn apply_legacy_rotation_speeds(
        &self,
        data: &mut ParticleData,
        stream_version: u32,
    ) -> Result<(), StreamError> {
        if stream_version >= 0x1400_0002 || data.rotation_speeds_60.is_empty() {
            return Ok(());
        }
        let active = usize::from(data.active_count_48);
        if active > data.rotation_speeds_60.len() {
            return Err(StreamError("active count exceeds rotation speed array"));
        }
        for speed in &mut data.rotation_speeds_60[..active] {
            *speed = self.initial_speed;
        }
        Ok(())
    }

    /// IDA: 0074DFA0 on-new-particle callback: 0074DFC7..0074E098 axis, 0074E09B..0074E14F angle and speed, in the
    /// original's conditional draw order. Completion: 100% of the branch and array semantics for valid data and
    /// CRT-domain draws.
    /// TODO: [ ] Compare trigonometric x87 rounding and the real CRT rand sequence.
    pub fn initialize_particle(
        &self,
        data: &mut ParticleData,
        index: u16,
        mut draw_rand: RotationRand<'_>,
    ) -> Result<(), StreamError> {
        let slot = usize::from(index);
        if (!data.rotation_axes.is_empty() && slot >= data.rotation_axes.len())
            || (!data.rotation_angles_54.is_empty()
                && (slot >= data.rotation_angles_54.len() || slot >= data.rotation_speeds_60.len()))
        {
            return Err(StreamError(
                "particle rotation index outside enabled arrays",
            ));
        }

        if !data.rotation_axes.is_empty() {
            let axis = &mut data.rotation_axes[slot];
            if self.random_initial_axis {
                let polar_ratio = (f64::from(next_rand(&mut draw_rand)?) / 32767.0) as f32;
                let polar = ENGINE_PI * polar_ratio;
                let z = f64::from(polar).cos() as f32;
                let radial_square = 1.0 - z * z;
                let radial = radial_square.sqrt() as f32;
                let azimuth_ratio = (f64::from(next_rand(&mut draw_rand)?) / 32767.0) as f32;
                let azimuth = ENGINE_TWO_PI * azimuth_ratio;
                let cosine = f64::from(azimuth).cos() as f32;
                axis[0] = cosine * radial;
                let sine = f64::from(azimuth).sin() as f32;
                axis[1] = sine * radial;
                axis[2] = z;
            } else {
                *axis = self.initial_axis;
            }
        }
        if !data.rotation_angles_54.is_empty() {
            let angle_draw = next_rand(&mut draw_rand)?;
            let angle_fraction = (f64::from(angle_draw) * 2.0 / 32767.0) as f32 - 1.0;
            data.rotation_angles_54[slot] =
                self.angle_variation * angle_fraction + self.initial_angle;
            let speed_draw = next_rand(&mut draw_rand)?;
            let speed_fraction = (f64::from(speed_draw) * 2.0 / 32767.0) as f32 - 1.0;
            let mut speed = self.speed_variation * speed_fraction + self.initial_speed;
            if self.random_speed_sign {
                let sign_sample = (f64::from(next_rand(&mut draw_rand)?) / 32767.0) as f32;
                if sign_sample <= 0.5 {
                    speed = -speed;
                }
            }
            data.rotation_speeds_60[slot] = speed;
        }
        Ok(())
    }

    /// IDA: 0074DEB0 uses ParticleInfo +0x14 last update time, +0x54 angle, +0x60 speed and +0x48 active count;
    /// 00A09B90 / 00A09BD0 initialize pi and tau. Completion: 100% of the finite update with the original's greater-than
    /// wrap and reset branches.
    /// TODO: [ ] Negative and nonfinite runtime cases and exact FPU status behaviour.
    pub fn update_particles(
        &self,
        data: &mut ParticleData,
        current_time: f32,
    ) -> Result<(), StreamError> {
        if data.rotation_angles_54.is_empty() || data.active_count_48 == 0 {
            return Ok(());
        }
        let active = usize::from(data.active_count_48);
        if active > data.rotation_angles_54.len()
            || active > data.rotation_speeds_60.len()
            || active > data.info_5c.len()
        {
            return Err(StreamError("active count exceeds particle rotation arrays"));
        }
        if !current_time.is_finite() {
            return Err(StreamError("particle rotation time must be finite"));
        }
        let reset_threshold = ENGINE_PI * 10.0;
        for index in 0..active {
            if !data.info_5c[index].last_update_time_14.is_finite()
                || !data.rotation_angles_54[index].is_finite()
                || !data.rotation_speeds_60[index].is_finite()
            {
                return Err(StreamError("particle rotation state must be finite"));
            }
            let elapsed = current_time - data.info_5c[index].last_update_time_14;
            let mut angle =
                elapsed * data.rotation_speeds_60[index] + data.rotation_angles_54[index];
            if !angle.is_finite() {
                return Err(StreamError("particle rotation result must be finite"));
            }
            data.rotation_angles_54[index] = angle;
            if angle > reset_threshold {
                data.rotation_angles_54[index] = 0.0;
            } else {
                while angle > ENGINE_TWO_PI {
                    angle -= ENGINE_TWO_PI;
                    data.rotation_angles_54[index] = angle;
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParticleRotationLoadResult {
    pub modifier: ParticleRotationModifier,
    pub target_index: u32,
    pub bytes_consumed: usize,
}

/// IDA: 00752DC0 NiPSysModifier::LoadBinary, 0074E390 rotation load, 00709430 NiVector3::LoadBinary. Completion: 100%
/// of the modern >= 0A010072 order and raw float bits; no graph link is silently resolved.
/// TODO: [ ] NiStream's callback and allocator short-read behaviour, and the pre-modern NiObject prefix.
pub fn parse_particle_rotation_payload(
    bytes: &[u8],
    stream_version: u32,
) -> Result<ParticleRotationLoadResult, StreamError> {
    const TRUNCATED_FLAG: &str = "truncated particle rotation flag";
    if stream_version < 0x0A01_0072 {
        return Err(StreamError("legacy particle modifier prefix unsupported"));
    }
    let mut reader = ByteReader::new(bytes);
    let mut modifier = ParticleRotationModifier::default();
    modifier.base.name_08 =
        read_modifier_name(&mut reader, TRUNCATED, "truncated particle rotation name")?;
    modifier.base.order_0c = reader.u32(TRUNCATED)?;
    let target_index = reader.u32(TRUNCATED)?;
    modifier.base.active_14 = reader.u8(TRUNCATED_FLAG)? != 0;
    modifier.initial_speed = reader.f32(TRUNCATED)?;
    if stream_version >= 0x1400_0002 {
        modifier.speed_variation = reader.f32(TRUNCATED)?;
        modifier.initial_angle = reader.f32(TRUNCATED)?;
        modifier.angle_variation = reader.f32(TRUNCATED)?;
        modifier.random_speed_sign = reader.u8(TRUNCATED_FLAG)? != 0;
    }
    modifier.random_initial_axis = reader.u8(TRUNCATED_FLAG)? != 0;
    for component in &mut modifier.initial_axis {
        *component = reader.f32(TRUNCATED)?;
    }
    Ok(ParticleRotationLoadResult {
        modifier,
        target_index,
        bytes_consumed: reader.position(),
    })
}

/// IDA: 00752CB0 consumes the deferred base target; actual target installation and 0074E2B0's post-load need the
/// particle-system owner. This is the C++ graph's inline arm. Completion: 100%.
pub fn link_particle_rotation(
    world: &mut World,
    entity: Entity,
    fields: &ParticleRotationLoadResult,
    table: &StreamObjectTable,
) -> Result<(), StreamError> {
    if fields.target_index != NULL_REFERENCE && fields.target_index as usize >= table.objects.len()
    {
        return Err(StreamError("particle rotation target out of range"));
    }
    let mut target = world
        .get_mut::<ParticleRotationModifier>(entity)
        .ok_or(StreamError("particle modifier component missing"))?;
    *target = fields.modifier.clone();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload() {
        let cold = ParticleRotationModifier::default();
        assert_eq!(cold.base.name_08, None);
        assert_eq!(cold.base.order_0c, 0);
        assert!(cold.base.active_14);
        assert_eq!(cold.initial_axis, [1.0, 0.0, 0.0]);
        assert!(cold.random_initial_axis);
        assert!(!cold.random_speed_sign);
        assert_eq!(cold.initial_speed, 0.0);
        assert_eq!(cold.speed_variation, 0.0);
        assert_eq!(cold.initial_angle, 0.0);
        assert_eq!(cold.angle_variation, 0.0);
        let mut b = Vec::new();
        b.extend(3u32.to_le_bytes());
        b.extend(b"Rot");
        b.extend(7u32.to_le_bytes());
        b.extend(2u32.to_le_bytes());
        b.push(0);
        for word in [0x8000_0000u32, 0x3F80_0000, 0x4000_0000, 0x4040_0000] {
            b.extend(word.to_le_bytes());
        }
        b.push(2); // random sign
        b.push(0); // fixed axis
        for word in [0x3F80_0000u32, 0x7FC1_2345, 0] {
            b.extend(word.to_le_bytes());
        }
        let used = b.len();
        b.push(0xAC);
        let modern = parse_particle_rotation_payload(&b, 0x1400_0002).unwrap();
        assert_eq!(modern.bytes_consumed, used);
        assert_eq!(modern.target_index, 2);
        assert_eq!(modern.modifier.base.name_08.as_deref(), Some(&b"Rot"[..]));
        assert_eq!(modern.modifier.base.order_0c, 7);
        assert!(!modern.modifier.base.active_14);
        assert!(modern.modifier.random_speed_sign);
        assert!(!modern.modifier.random_initial_axis);
        assert_eq!(modern.modifier.initial_speed.to_bits(), 0x8000_0000);
        assert_eq!(modern.modifier.initial_axis[1].to_bits(), 0x7FC1_2345);
        for length in 0..used {
            assert!(parse_particle_rotation_payload(&b[..length], 0x1400_0002).is_err());
        }
        let mut legacy = Vec::new();
        legacy.extend(0u32.to_le_bytes());
        legacy.extend(8u32.to_le_bytes());
        legacy.extend(u32::MAX.to_le_bytes());
        legacy.push(1);
        legacy.extend(0x4080_0000u32.to_le_bytes());
        legacy.push(1);
        legacy.extend(0x3F80_0000u32.to_le_bytes());
        legacy.extend(0u32.to_le_bytes());
        legacy.extend(0u32.to_le_bytes());
        let old = parse_particle_rotation_payload(&legacy, 0x0A01_0072).unwrap();
        assert_eq!(old.bytes_consumed, legacy.len());
        assert_eq!(old.modifier.base.name_08, None);
        assert_eq!(old.modifier.base.order_0c, 8);
        assert_eq!(old.modifier.initial_speed, 4.0);
        assert_eq!(old.modifier.speed_variation, 0.0);
        assert_eq!(old.modifier.initial_angle, 0.0);
        assert!(!old.modifier.random_speed_sign);
        assert!(old.modifier.random_initial_axis);
        assert!(parse_particle_rotation_payload(&legacy, 0x0A01_0071).is_err());
    }

    struct Draws {
        values: Vec<i32>,
        next: usize,
    }

    impl Draws {
        fn new(values: &[i32]) -> Self {
            Self {
                values: values.to_vec(),
                next: 0,
            }
        }

        fn draw(&mut self) -> i32 {
            assert!(
                self.next < self.values.len(),
                "rotation made an unexpected random draw"
            );
            self.next += 1;
            self.values[self.next - 1]
        }
    }

    #[test]
    fn initialization_draw_order() {
        let mut modifier = ParticleRotationModifier {
            initial_axis: [0.0, 1.0, 0.0],
            random_initial_axis: false,
            initial_angle: 2.0,
            angle_variation: 1.0,
            initial_speed: 4.0,
            speed_variation: 1.0,
            random_speed_sign: true,
            ..ParticleRotationModifier::default()
        };
        let mut data = ParticleData {
            rotation_axes: vec![[0.0; 3]],
            rotation_angles_54: vec![0.0],
            rotation_speeds_60: vec![0.0],
            ..ParticleData::default()
        };
        let mut fixed = Draws::new(&[0, 32767, 0]);
        modifier
            .initialize_particle(&mut data, 0, Some(&mut || fixed.draw()))
            .unwrap();
        assert_eq!(fixed.next, 3);
        assert_eq!(data.rotation_axes[0], modifier.initial_axis);
        assert_eq!(data.rotation_angles_54[0], 1.0);
        assert_eq!(data.rotation_speeds_60[0], -5.0);
        modifier.random_initial_axis = true;
        modifier.random_speed_sign = false;
        let mut random = Draws::new(&[0, 0, 32767, 0]);
        modifier
            .initialize_particle(&mut data, 0, Some(&mut || random.draw()))
            .unwrap();
        assert_eq!(random.next, 4);
        assert_eq!(data.rotation_axes[0], [0.0, 0.0, 1.0]);
        assert_eq!(data.rotation_angles_54[0], 3.0);
        assert_eq!(data.rotation_speeds_60[0], 3.0);
        data.rotation_angles_54.clear();
        data.rotation_speeds_60.clear();
        let mut axis_only = Draws::new(&[0, 0]);
        modifier
            .initialize_particle(&mut data, 0, Some(&mut || axis_only.draw()))
            .unwrap();
        assert_eq!(axis_only.next, 2);
        data.rotation_axes.clear();
        let mut none = Draws::new(&[]);
        modifier
            .initialize_particle(&mut data, 0, Some(&mut || none.draw()))
            .unwrap();
        assert_eq!(none.next, 0);
    }

    #[test]
    fn legacy_speeds_and_update() {
        let modifier = ParticleRotationModifier {
            initial_speed: 4.0,
            ..ParticleRotationModifier::default()
        };
        let mut data = ParticleData {
            active_count_48: 2,
            rotation_speeds_60: vec![1.0, 2.0, 3.0],
            ..ParticleData::default()
        };
        modifier
            .apply_legacy_rotation_speeds(&mut data, 0x0A01_0072)
            .unwrap();
        assert_eq!(data.rotation_speeds_60, [4.0, 4.0, 3.0]);
        data.rotation_speeds_60 = vec![1.0, 2.0, 3.0];
        modifier
            .apply_legacy_rotation_speeds(&mut data, 0x1400_0002)
            .unwrap();
        assert_eq!(data.rotation_speeds_60, [1.0, 2.0, 3.0]);
        data.rotation_angles_54 = vec![6.0, 31.0, 99.0];
        data.info_5c.resize(2, Default::default());
        data.rotation_speeds_60 = vec![1.0, 1.0, 0.0];
        modifier.update_particles(&mut data, 1.0).unwrap();
        assert!(data.rotation_angles_54[0] > 0.7167 && data.rotation_angles_54[0] < 0.7169);
        assert_eq!(data.rotation_angles_54[1], 0.0);
        assert_eq!(data.rotation_angles_54[2], 99.0);
        assert_eq!(data.info_5c[0].last_update_time_14, 0.0);
    }
}
