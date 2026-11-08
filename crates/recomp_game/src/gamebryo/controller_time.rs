//! 007155A0, the NiTimeController time mapping and its one shared cache.

use bevy_ecs::prelude::*;
use recomp_core::x87::float_fmod;

const INVALID_TIME: f32 = -f32::MAX;
const INVALID_LOW: f32 = f32::MAX;

/// IDA: exactly the NiTimeController fields 007155A0 reads and writes (raw floats +0x08..+0x28 and the flags). Cold
/// values are the C++ port's `ParticleTimeControllerCold`, which is owned by the particle family; a controller holds or
/// converts into this.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ControllerTimeFields {
    pub flags: u16,
    pub frequency: f32,
    pub phase: f32,
    pub low_key_time: f32,
    pub high_key_time: f32,
    pub start_time: f32,
    pub last_time: f32,
    pub time_accumulator: f32,
}

impl Default for ControllerTimeFields {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            flags: 12,
            frequency: 1.0,
            phase: 0.0,
            low_key_time: INVALID_LOW,
            high_key_time: INVALID_TIME,
            start_time: INVALID_TIME,
            last_time: INVALID_TIME,
            time_accumulator: 0.0,
        }
    }
}

/// IDA: B27130 is disk-initialized cycle 3; B3FC88..94 are zero-filled BSS. There is one cache across all controllers.
/// Host substitution: the C++ port's `std::mutex` is replaced by the exclusive `&mut` borrow of this resource.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct ParticleControllerTimeCache {
    pub cycle: u32,
    pub output: f32,
    pub input: f32,
    pub low_key_time: f32,
    pub high_key_time: f32,
}

impl Default for ParticleControllerTimeCache {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            cycle: 3,
            output: 0.0,
            input: 0.0,
            low_key_time: 0.0,
            high_key_time: 0.0,
        }
    }
}

// IDA: 007155A0 float __thiscall (NiTimeController*, float currentTime), vtable +0x64 -> CRT fmod; raw float fields
// +0x08..+0x28
// Completion: 100% of the recovered state, cache, wrap, reverse and clamp order with f32 stores, in the supported host
// arithmetic domain, including the original shared-cache key. Precision: 24-bit per step on the main thread after
// CreateDevice, each fmul/fadd/fsub rounded to a float as it is computed.
// TODO: [ ] Match original x87 FP exception state.
// TODO: [ ] 007155A0 guards its cache with the B3FCA0 critical section: a later phase's worker thread that evaluates a
// controller runs at its own control word (no worker is recorded; 53-bit if it keeps the default).
pub fn evaluate_particle_controller_time(
    controller: &mut ControllerTimeFields,
    current_time: f32,
    cache: &mut ParticleControllerTimeCache,
) -> f32 {
    if controller.start_time == INVALID_TIME {
        controller.start_time = current_time;
    }

    let mut delta = current_time;
    if controller.last_time == INVALID_TIME {
        controller.time_accumulator = 0.0;
        if controller.flags & 1 != 0 {
            delta = 0.0;
        }
    } else {
        delta = current_time - controller.last_time;
    }
    let cycle = u32::from((controller.flags >> 1) & 3);
    controller.time_accumulator += controller.frequency * delta;
    let mut value = controller.time_accumulator + controller.phase;
    controller.last_time = current_time;

    if controller.high_key_time == cache.high_key_time
        && controller.low_key_time == cache.low_key_time
        && value == cache.input
        && cycle == cache.cycle
    {
        return cache.output;
    }

    cache.cycle = cycle;
    cache.high_key_time = controller.high_key_time;
    cache.low_key_time = controller.low_key_time;
    cache.input = value;

    if controller.high_key_time != INVALID_TIME && controller.low_key_time != INVALID_LOW {
        if cycle == 1 {
            let width = controller.high_key_time - controller.low_key_time;
            if width != 0.0 {
                let doubled_width = width + width;
                let mut folded = float_fmod(value, doubled_width);
                if folded < 0.0 {
                    folded += doubled_width;
                }
                // The original's reverse folding uses the input directly, without first subtracting the low key time.
                // That asymmetry is kept.
                let mut reflected = folded;
                if width < reflected {
                    reflected = doubled_width - reflected;
                }
                value = reflected + controller.low_key_time;
            } else {
                value = controller.low_key_time;
            }
        } else if cycle == 0 {
            let width = controller.high_key_time - controller.low_key_time;
            if width == 0.0 {
                value = controller.low_key_time;
            } else {
                let offset = value - controller.low_key_time;
                let wrapped = float_fmod(offset, width);
                value = wrapped + controller.low_key_time;
                if controller.low_key_time > value {
                    value += width;
                }
            }
        }
    }
    if controller.high_key_time < value {
        value = controller.high_key_time;
    } else if controller.low_key_time > value {
        value = controller.low_key_time;
    }
    if controller.flags & 0x10 != 0 {
        value = controller.high_key_time - (value - controller.low_key_time);
    }
    // The original cache key omits playback bit 0x10, so sharing the cache deliberately retains the result of the
    // preceding matching controller.
    cache.output = value;
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 007155F4..0071560E at 24-bit: the frequency product rounds before the accumulator is added. Expected bits from exact
    /// per-step rounding; one rounding over the exact sum (the old `f64` form) gives 0x44B98AD7.
    #[test]
    fn accumulator_rounds_each_step() {
        let mut fields = ControllerTimeFields {
            frequency: f32::from_bits(0x3FB2_8F5C),
            last_time: f32::from_bits(0x3FD9_DB23),
            time_accumulator: f32::from_bits(0x44B7_9571),
            ..ControllerTimeFields::default()
        };
        evaluate_particle_controller_time(
            &mut fields,
            f32::from_bits(0x414E_F1AA),
            &mut ParticleControllerTimeCache::default(),
        );
        assert_eq!(fields.time_accumulator.to_bits(), 0x44B9_8AD6);
    }

    /// Ported from `particle_controller_tests`' `verify_time_cache_and_reverse_low_key`.
    #[test]
    fn time_cache_and_reverse_low_key() {
        let mut cache = ParticleControllerTimeCache::default();
        let mut first = ControllerTimeFields {
            flags: 8,
            frequency: 0.0,
            phase: 3.0,
            low_key_time: 0.0,
            high_key_time: 10.0,
            ..ControllerTimeFields::default()
        };
        assert_eq!(
            evaluate_particle_controller_time(&mut first, 1.0, &mut cache),
            3.0
        );
        let mut reverse = first;
        reverse.flags = 0x18; // playback reverse is omitted from the shared cache key
        assert_eq!(
            evaluate_particle_controller_time(&mut reverse, 1.0, &mut cache),
            3.0
        );
        let mut fresh = ParticleControllerTimeCache::default();
        let mut independent = reverse;
        assert_eq!(
            evaluate_particle_controller_time(&mut independent, 1.0, &mut fresh),
            7.0
        );
        let mut reverse_cache = ParticleControllerTimeCache::default();
        let mut pingpong = ControllerTimeFields {
            flags: 0x0A, // cycle 1
            frequency: 0.0,
            phase: 7.0,
            low_key_time: 2.0,
            high_key_time: 6.0,
            ..ControllerTimeFields::default()
        };
        assert_eq!(
            evaluate_particle_controller_time(&mut pingpong, 0.0, &mut reverse_cache),
            3.0
        );
    }
}
