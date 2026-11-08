//! The wrapped keyed-bool continuation of NiPSysEmitterCtlr::Update.
//! x87: every float operation here is a 24-bit site (main thread after CreateDevice, control word 007F, P04-RUN-002): plain
//! `f32` in the instructions' grouping. Qword constants hold exact values, so `(f64::from(x) op C) as f32` is one rounding.

use bevy_ecs::prelude::*;
use recomp_core::reader::StreamError;

use super::animation::{INVALID_FLOAT_INTERPOLATOR_VALUE, update_float_interpolator};
use super::bool_animation::{BoolKey, BoolKeyData, BoolKeyType};
use super::particle_controller::{ParticleEmitterController, rate_state};

const NEGATIVE_MAX: f32 = -f32::MAX;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ParticleWrappedBoolStatus {
    #[default]
    NotWrapped,
    InitializedFirstSample,
    WrappedQuiet,
    WrappedEmitted,
    UnsupportedMissingBoolSample,
    UnsupportedBoolTrack,
    UnsupportedNonfiniteTime,
    UnsupportedInconsistentRawState,
    UnsupportedKeyedRateAfterStores,
    UnsupportedMissingRateAfterStores,
    UnsupportedEmissionMathAfterStores,
}

/// IDA: 007509D7-00750A3D stages six f32 EmitRateWindow arguments after rate sampling. Completion: 100% of the typed carrier
/// for that call order.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ParticleWrappedWindow {
    pub emission_time: f32,
    pub upper_time: f32,
    pub lower_time: f32,
    pub interval_begin: f32,
    pub interval_end: f32,
    pub rate: f32,
}

/// IDA: 0075074E-00750755 stores the current raw sample into +0x54 and the mapped sample time into +0x50 before wrapped key
/// scanning. Completion: 85% of the typed view of reached branch state, including raw bool bytes.
/// TODO: [ ] The original NiBoolKey stride and type owner and the x87 live stack.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ParticleWrappedBoolResult {
    pub status: ParticleWrappedBoolStatus,
    pub prior_raw: u8,
    pub current_raw: u8,
    pub prior_time: f32,
    pub current_time: f32,
    pub active_begin_index: Option<usize>,
    pub active_end_index: Option<usize>,
}

// IDA: 007507AA-007507DF scans keys backward from count-1 until it finds the latest zero raw byte whose time is at or before the
// prior mapped time; ECX remains one past that off-key index
// Completion: 100% of the bounded host index order for finite type-5 keys
// TODO: [ ] Malformed key-count and stride pointer behaviour.
fn begin_after_prior_active(keys: &[BoolKey], prior_time: f32) -> usize {
    for one_past in (1..=keys.len()).rev() {
        let key = &keys[one_past - 1];
        if key.time <= prior_time && key.value == 0 {
            return one_past;
        }
    }
    0
}

// IDA: 007507E6-00750825 seeks the first raw-nonzero on-key strictly after the prior time. A wrapped pass does not cap the tail at
// current time; 00750836-0075085F scans the head only when that pass is wrapped
// Completion: 100% of the finite ordered type-5 scan branches and exact strict inequalities
// TODO: [ ] Raw key-stride semantics for other NiBoolKey types.
fn begin_after_prior_inactive(
    keys: &[BoolKey],
    prior_time: f32,
    current_time: f32,
) -> Option<usize> {
    let wrapped = current_time < prior_time;
    for (index, key) in keys.iter().enumerate() {
        if key.time > prior_time && (wrapped || key.time <= current_time) && key.value != 0 {
            return Some(index);
        }
    }
    if !wrapped {
        return None;
    }
    keys.iter()
        .position(|key| key.time < current_time && key.value != 0)
}

// IDA: 00750867-00750891 finds the next zero raw byte, or uses the final key when the on run reaches the key-count boundary
// Completion: 100% of the bounded type-5 index and byte order
fn end_of_active_run(keys: &[BoolKey], begin: usize) -> usize {
    let mut end = begin;
    while end < keys.len() && keys[end].value != 0 {
        end += 1;
    }
    if end == keys.len() {
        end -= 1;
    }
    end
}

/// IDA: 007506EB first keyed-bool sample, 00750779-0075088D wrapped scan, 007509C0 rate sample, 00750A3D rate-window and Emit, max
/// 20 passes. Completion: 80% of the exact first sample, wrapped scans and finite constant-rate windows.
/// TODO: [ ] Remaining keyed-rate and blend branches, once an owner is verified, and extended x87 precision.
pub struct ParticleControllerWrappedBool;

impl ParticleControllerWrappedBool {
    // IDA: N/A host entry for first-sample and quiet scans without Emit
    // Completion: 100% of the scoped prefix and the explicit no-service active-window boundary
    // TODO: [ ] Supply the rate-window service to convert active emission.
    pub fn continue_after_gate(
        world: &mut World,
        controller: &mut ParticleEmitterController,
        bool_keys: &BoolKeyData,
        mapped_time: f32,
    ) -> Result<ParticleWrappedBoolResult, StreamError> {
        Self::continue_after_gate_with_emit(
            world,
            controller,
            bool_keys,
            mapped_time,
            0.0,
            None,
            None,
        )
    }

    // IDA: 007506EB-00750A91 NiPSysEmitterCtlr keyed-bool continuation. Call after 00750260's target, modifier, gate and
    // interpolator checks and before its keyed branch samples BoolKeyData. `NotWrapped` leaves the controller unchanged so the
    // ordinary path runs
    // Completion: 80% of the exact first sample, ordered wrapped scans, constant-rate windows and the 20-pass cap for finite type-5
    // keys; an explicit keyed-rate and blend frontier
    // TODO: [ ] Original malformed keys.
    #[allow(
        clippy::type_complexity,
        reason = "the window service takes the world and the detached controller the C++ lambda captures"
    )]
    pub fn continue_after_gate_with_emit(
        world: &mut World,
        controller: &mut ParticleEmitterController,
        bool_keys: &BoolKeyData,
        mapped_time: f32,
        target_system_update_time: f32,
        mut emit_window: Option<
            &mut (
                     dyn FnMut(
                &mut World,
                &mut ParticleEmitterController,
                &ParticleWrappedWindow,
            ) -> Result<(), StreamError>
                         + '_
                 ),
        >,
        mut read_target_system_update_time: Option<&mut (dyn FnMut(&World) -> f32 + '_)>,
    ) -> Result<ParticleWrappedBoolResult, StreamError> {
        use ParticleWrappedBoolStatus as Status;
        let mut result = ParticleWrappedBoolResult {
            prior_time: controller.last_emit_time_50,
            prior_raw: controller.byte_54,
            current_time: mapped_time,
            ..ParticleWrappedBoolResult::default()
        };

        // Host domain check precedes original key-pointer arithmetic.
        if !mapped_time.is_finite()
            || (result.prior_time != NEGATIVE_MAX && !result.prior_time.is_finite())
        {
            result.status = Status::UnsupportedNonfiniteTime;
            return Ok(result);
        }
        // A nonwrapped caller continues through its existing ordinary path. A NaN is excluded above.
        if result.prior_time != NEGATIVE_MAX && mapped_time >= result.prior_time {
            result.status = Status::NotWrapped;
            return Ok(result);
        }
        if bool_keys.key_type() != BoolKeyType::STEP || bool_keys.keys().is_empty() {
            result.status = Status::UnsupportedBoolTrack;
            return Ok(result);
        }

        let mut sampled_index = 0;
        let Some(sampled) = bool_keys.sample(mapped_time, &mut sampled_index) else {
            result.status = Status::UnsupportedMissingBoolSample;
            return Ok(result);
        };
        result.current_raw = sampled;

        // 00750723-0075073D: the sentinel sample stores +0x50 before raw +0x54.
        if result.prior_time == NEGATIVE_MAX {
            controller.last_emit_time_50 = mapped_time;
            controller.byte_54 = sampled;
            result.status = Status::InitializedFirstSample;
            return Ok(result);
        }

        // 00750740-00750755: the ordinary sample publishes raw +0x54 before mapped time +0x50, then scans keys with the prior
        // values in locals.
        controller.byte_54 = sampled;
        controller.last_emit_time_50 = mapped_time;

        let keys = bool_keys.keys();
        let mut prior_time = result.prior_time;
        let mut prior_raw = result.prior_raw;
        let mut emitted = false;
        for _ in 0..20 {
            let wrapped = mapped_time < prior_time; // 00750795-0075079E
            let begin = if prior_raw != 0 {
                let index = begin_after_prior_active(keys, prior_time);
                if index >= keys.len() {
                    result.status = Status::UnsupportedInconsistentRawState;
                    return Ok(result);
                }
                index
            } else {
                let Some(index) = begin_after_prior_inactive(keys, prior_time, mapped_time) else {
                    result.status = if emitted {
                        Status::WrappedEmitted
                    } else {
                        Status::WrappedQuiet
                    };
                    return Ok(result);
                };
                index
            };
            let end = end_of_active_run(keys, begin);
            result.active_begin_index = Some(begin);
            result.active_end_index = Some(end);

            let begin_key_time = keys[begin].time;
            let end_key_time = keys[end].time;
            let mut window = ParticleWrappedWindow::default();
            let rate_sample_time = if wrapped {
                if end_key_time > prior_time {
                    // 007508C3-00750902: tail section from the prior time through the final key; the raw active run supplies the
                    // rate span.
                    window.upper_time = keys[keys.len() - 1].time;
                    window.lower_time = prior_time;
                    window.interval_begin = begin_key_time;
                    window.interval_end = end_key_time;
                } else {
                    // 00750907-00750948: head section from the first key through mapped time; bound the active interval before
                    // the rate sample.
                    window.upper_time = mapped_time;
                    window.lower_time = keys[0].time;
                    window.interval_begin = begin_key_time;
                    window.interval_end = if end_key_time < mapped_time {
                        end_key_time
                    } else {
                        mapped_time
                    };
                }
                window.interval_begin + (window.interval_end - window.interval_begin) * 0.5
            } else {
                // 0075094E-007509D5: a continuation pass after wrapping uses the ordinary window, sampling at the midpoint only if
                // the off key is strictly before the mapped time.
                window.upper_time = if end_key_time < mapped_time {
                    end_key_time
                } else {
                    mapped_time
                };
                window.lower_time = prior_time;
                window.interval_begin = begin_key_time;
                window.interval_end = end_key_time;
                if end_key_time < mapped_time {
                    begin_key_time + (end_key_time - begin_key_time) * 0.5
                } else {
                    mapped_time
                }
            };

            // Original rate-owner checks occur only after a real active window has been selected and its time bounds have been
            // calculated.
            let Some(rate_source) = controller.birth_rate_interpolator_3c else {
                result.status = Status::UnsupportedMissingRateAfterStores;
                return Ok(result);
            };
            let (rate_data, rate_cached) = rate_state(world, rate_source);
            if rate_cached == INVALID_FLOAT_INTERPOLATOR_VALUE {
                result.status = Status::UnsupportedMissingRateAfterStores;
                return Ok(result);
            }
            if rate_data.is_some() {
                result.status = Status::UnsupportedKeyedRateAfterStores;
                return Ok(result);
            }
            // 007509A1 / 007509D5: rate virtual +0x5C precedes the target +0xE8 read and the six-argument 00750040 call.
            let mut rate = 0.0;
            if !update_float_interpolator(world, rate_source, rate_sample_time, &mut rate) {
                result.status = Status::UnsupportedMissingRateAfterStores;
                return Ok(result);
            }
            window.rate = rate;
            // 007509DF reads target +0xE8 only after the rate virtual returns.
            let mut target_base = match read_target_system_update_time.as_mut() {
                Some(read) => read(world),
                None => target_system_update_time,
            };
            if wrapped {
                // 007509EB-007509F5 stores this sum to an f32 local first.
                target_base +=
                    controller.time.fields.high_key_time - controller.time.fields.low_key_time;
            }
            // 00750A26-00750A3D adds current then subtracts prior before an f32 argument store, each rounded at 24 bits.
            window.emission_time = target_base + mapped_time - prior_time;
            let Some(emit) = emit_window.as_mut() else {
                result.status = Status::UnsupportedEmissionMathAfterStores;
                return Ok(result);
            };
            emit(world, controller, &window)?;
            emitted = true;

            // 00750A42-00750A91 tests the raw off-key time after Emit, then resamples the bool track at the completed window's upper
            // bound.
            if end_key_time >= mapped_time {
                result.status = Status::WrappedEmitted;
                return Ok(result);
            }
            let mut next_index = 0;
            let Some(next_raw) = bool_keys.sample(window.upper_time, &mut next_index) else {
                result.status = Status::UnsupportedMissingBoolSample;
                return Ok(result);
            };
            prior_time = window.upper_time;
            prior_raw = next_raw;
        }
        result.status = if emitted {
            Status::WrappedEmitted
        } else {
            Status::WrappedQuiet
        };
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(values: &[(f32, u8)]) -> BoolKeyData {
        BoolKeyData::new(
            BoolKeyType::STEP,
            values
                .iter()
                .map(|&(time, value)| BoolKey { time, value })
                .collect(),
        )
        .unwrap()
    }

    /// Ported from `particle_controller_wrap_tests`' `verify_first_sample_and_nonwrapped_state`.
    #[test]
    fn first_sample_and_nonwrapped_state() {
        let mut world = World::new();
        let mut controller = ParticleEmitterController::default();
        let data = keys(&[(0.0, 0), (1.0, 7), (2.0, 0)]);
        let first = ParticleControllerWrappedBool::continue_after_gate(
            &mut world,
            &mut controller,
            &data,
            1.0,
        )
        .unwrap();
        assert_eq!(
            first.status,
            ParticleWrappedBoolStatus::InitializedFirstSample
        );
        assert_eq!(controller.last_emit_time_50, 1.0);
        assert_eq!(controller.byte_54, 7);
        assert_eq!(first.prior_time, -f32::MAX);
        assert_eq!(first.current_raw, 7);
        let ordinary = ParticleControllerWrappedBool::continue_after_gate(
            &mut world,
            &mut controller,
            &data,
            2.0,
        )
        .unwrap();
        assert_eq!(ordinary.status, ParticleWrappedBoolStatus::NotWrapped);
        assert_eq!(controller.last_emit_time_50, 1.0);
        assert_eq!(controller.byte_54, 7);
    }

    /// Ported from `particle_controller_wrap_tests`' `verify_wrapped_quiet_and_strict_boundaries`.
    #[test]
    fn wrapped_quiet_and_strict_boundaries() {
        let mut world = World::new();
        let all_off = keys(&[(0.0, 0), (1.0, 0), (2.0, 0)]);
        let mut quiet = ParticleEmitterController {
            last_emit_time_50: 2.0,
            byte_54: 0,
            ..ParticleEmitterController::default()
        };
        let result = ParticleControllerWrappedBool::continue_after_gate(
            &mut world, &mut quiet, &all_off, 0.5,
        )
        .unwrap();
        assert_eq!(result.status, ParticleWrappedBoolStatus::WrappedQuiet);
        assert_eq!(quiet.last_emit_time_50, 0.5);
        assert_eq!(quiet.byte_54, 0);
        assert_eq!(result.active_begin_index, None);
        assert_eq!(result.active_end_index, None);

        // The tail scan requires key.time > old time; the head scan requires key.time < new time.
        let equality = keys(&[(0.0, 0), (1.0, 1), (2.0, 0)]);
        let mut exact = ParticleEmitterController {
            last_emit_time_50: 1.0,
            byte_54: 0,
            ..ParticleEmitterController::default()
        };
        let boundary = ParticleControllerWrappedBool::continue_after_gate(
            &mut world, &mut exact, &equality, 1.0,
        )
        .unwrap();
        assert_eq!(boundary.status, ParticleWrappedBoolStatus::NotWrapped);
        assert_eq!(exact.last_emit_time_50, 1.0);
        assert_eq!(exact.byte_54, 0);
        exact.last_emit_time_50 = 2.0;
        let strict = ParticleControllerWrappedBool::continue_after_gate(
            &mut world, &mut exact, &equality, 1.0,
        )
        .unwrap();
        assert_eq!(strict.status, ParticleWrappedBoolStatus::WrappedQuiet);
        assert_eq!(strict.active_begin_index, None);
        assert_eq!(exact.last_emit_time_50, 1.0);
        assert_eq!(exact.byte_54, 1);
    }

    /// Ported from `particle_controller_wrap_tests`' `verify_reached_store_unsupported_branch`.
    #[test]
    fn reached_store_unsupported_branch() {
        let mut world = World::new();
        let mut controller = ParticleEmitterController {
            last_emit_time_50: 3.0,
            byte_54: 0,
            ..ParticleEmitterController::default()
        };
        let data = keys(&[(0.0, 0), (1.0, 1), (2.0, 0), (3.0, 0)]);
        let result = ParticleControllerWrappedBool::continue_after_gate(
            &mut world,
            &mut controller,
            &data,
            2.5,
        )
        .unwrap();
        assert_eq!(
            result.status,
            ParticleWrappedBoolStatus::UnsupportedMissingRateAfterStores
        );
        assert_eq!(result.active_begin_index, Some(1));
        assert_eq!(result.active_end_index, Some(2));
        assert_eq!(controller.last_emit_time_50, 2.5);
        assert_eq!(controller.byte_54, 0);
    }
}
