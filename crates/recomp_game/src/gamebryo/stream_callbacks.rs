//! The root callback tail of 00714390.

use bevy_ecs::prelude::*;
use recomp_core::reader::StreamError;

use super::stream_roots::StreamRoots;

/// IDA: 006D43E0, the one callback registered at startup.
pub const LEGACY_CONTROLLER_STREAM_CALLBACK: u32 = 0x006D_43E0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StreamRootCallbackStatus {
    #[default]
    Complete,
    Cancelled,
    UnsupportedCallback,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StreamRootCallbackResult {
    pub status: StreamRootCallbackStatus,
    pub root_index: Option<usize>,
    pub callback_slot: Option<usize>,
    pub original_callback: Option<u32>,
}

/// IDA: 00714390's raw NiStream +0x260 state: 2 cancels, 3 pauses, and 4 is stored before 00748810 waits. A worker
/// waiting here never holds `World`; the wait is the caller's. It receives the state because another thread can
/// store it during the original wait, and the loop reads it afterwards.
#[derive(Default)]
pub struct StreamRootCallbackControl<'a> {
    pub load_state: u32,
    pub wait_for_resume: Option<&'a mut dyn FnMut(&mut u32)>,
}

/// `current_callbacks` mirrors the global reload after every non-null callback. `invoke` supplies already-converted
/// callbacks; returning false exposes a missing body instead of skipping its side effects.
#[derive(Default)]
pub struct StreamRootCallbackRuntime<'a> {
    pub current_callbacks: Option<CurrentCallbacks<'a>>,
    pub invoke: Option<InvokeCallback<'a>>,
}

pub type CurrentCallbacks<'a> = &'a mut (dyn FnMut(&World) -> Vec<u32> + 'a);
pub type InvokeCallback<'a> = &'a mut (dyn FnMut(&mut World, u32, Entity, u32) -> bool + 'a);

/// IDA: 006D43E0 __cdecl (NiStream*, NiObject*) returns at version >= 0A010068, before any RTTI. Completion: 100% of
/// the cutoff and identity branch; unknown or legacy work is never credited.
/// TODO: [ ] 006D42D0's legacy controller conversion.
#[must_use]
pub const fn invoke_supported_stream_callback(original_callback: u32, stream_version: u32) -> bool {
    original_callback == LEGACY_CONTROLLER_STREAM_CALLBACK && stream_version >= 0x0A01_0068
}

/// IDA: 00714390 0071451B..007145AB after the PostLoad pass: root-outer, slot-inner, null slots skipped, the global
/// reloaded after each call, and state 2 checked only before the wait and at the end. Completion: 100% of the
/// supported loop and state order.
/// TODO: [ ] Original wait and event returns, mutable root array and reference counts.
pub fn dispatch_stream_root_callbacks(
    world: &mut World,
    roots: &StreamRoots,
    stream_version: u32,
    control: &mut StreamRootCallbackControl<'_>,
    runtime: &mut StreamRootCallbackRuntime<'_>,
) -> Result<StreamRootCallbackResult, StreamError> {
    if control.load_state == 2 {
        return Ok(StreamRootCallbackResult {
            status: StreamRootCallbackStatus::Cancelled,
            ..Default::default()
        });
    }
    if control.load_state == 3 {
        control.load_state = 4;
        let wait = control
            .wait_for_resume
            .as_mut()
            .ok_or(StreamError("missing stream pause wait service"))?;
        wait(&mut control.load_state);
    }
    // The original does not check state 2 again after the wait, or between calls.
    let current = runtime
        .current_callbacks
        .as_mut()
        .ok_or(StreamError("missing current stream callback registry"))?;
    let mut slots = current(world);
    if slots.iter().any(|&value| value != 0) {
        for (root_index, root) in roots.owners.iter().enumerate() {
            let Some(root) = *root else { continue };
            let mut slot = 0;
            while slot < slots.len() {
                let callback = slots[slot];
                if callback != 0 {
                    let supported = match runtime.invoke.as_mut() {
                        Some(invoke) => invoke(world, callback, root, stream_version),
                        None => invoke_supported_stream_callback(callback, stream_version),
                    };
                    if !supported {
                        return Ok(StreamRootCallbackResult {
                            status: StreamRootCallbackStatus::UnsupportedCallback,
                            root_index: Some(root_index),
                            callback_slot: Some(slot),
                            original_callback: Some(callback),
                        });
                    }
                    slots = current(world);
                }
                slot += 1;
            }
        }
    }
    Ok(StreamRootCallbackResult {
        status: if control.load_state == 2 {
            StreamRootCallbackStatus::Cancelled
        } else {
            StreamRootCallbackStatus::Complete
        },
        ..Default::default()
    })
}
