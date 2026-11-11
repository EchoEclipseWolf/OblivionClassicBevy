//! Defer semantic Quit until real GPU readback and asynchronous pipeline work
//! have drained. No wall-clock sleep and no focus or window activation request. Exiting with pipelines still compiling
//! crashes the process in the NVIDIA driver (status 139). Adapted from CreeperWorld3Bevy.
//! IDA: N/A host only.
use bevy::{
    prelude::*,
    render::{
        Render, RenderApp, RenderSystems,
        render_resource::{CachedPipelineState, PipelineCache},
        view::screenshot::{Screenshot, ScreenshotCaptured},
    },
};
use std::sync::{Arc, Mutex};
#[derive(Debug, Clone, Default)]
pub struct RenderExitSnapshot {
    pub render_frames: u64,
    pub pipeline_count: usize,
    pub pending_pipelines: usize,
    pub pipeline_errors: usize,
    pub gpu_readback_received: bool,
    pub requested: bool,
    pub ready: bool,
    pub idle_render_frames: u32,
}
#[derive(Default)]
struct Shared {
    observation: RenderExitSnapshot,
    code: Option<AppExit>,
    requested_after: u64,
    render_backend: bool,
}
#[derive(Resource, Clone, Default)]
pub struct RenderExitReadiness(Arc<Mutex<Shared>>);
impl RenderExitReadiness {
    /// IDA: N/A a helper, with no address of its own.
    pub fn snapshot(&self) -> RenderExitSnapshot {
        self.0
            .lock()
            .expect("render exit readiness lock")
            .observation
            .clone()
    }
    // IDA: N/A a helper, with no address of its own.
    fn readback(&self) {
        self.0
            .lock()
            .expect("render exit readiness lock")
            .observation
            .gpu_readback_received = true;
    }
}
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RenderExitFinalize;
pub struct RenderExitPlugin;
impl Plugin for RenderExitPlugin {
    /// IDA: N/A Rust's `Plugin`.
    fn build(&self, app: &mut App) {
        let readiness = RenderExitReadiness::default();
        if let Some(render) = app.get_sub_app_mut(RenderApp) {
            readiness
                .0
                .lock()
                .expect("render exit readiness lock")
                .render_backend = true;
            render
                .insert_resource(readiness.clone())
                .add_systems(Render, report_pipelines.after(RenderSystems::PostCleanup));
        }
        app.insert_resource(readiness)
            .add_systems(Last, finish_exit.in_set(RenderExitFinalize));
    }
}
/// IDA: N/A a helper, with no address of its own.
/// Success means a Quit request was queued. Snapshot.ready and actual process
/// termination are separate observations. Repeated requests do not start more
/// readbacks; an error exit supersedes success.
pub fn request_exit(world: &mut World, code: AppExit) -> Result<(), String> {
    let readiness = world
        .get_resource::<RenderExitReadiness>()
        .ok_or("render exit guard unavailable")?
        .clone();
    let needs_readback = {
        let mut shared = readiness
            .0
            .lock()
            .map_err(|_| "render exit readiness lock poisoned")?;
        let first = shared.code.is_none();
        if first || matches!(code, AppExit::Error(_)) {
            shared.code = Some(code);
        }
        if first {
            shared.requested_after = shared.observation.render_frames;
            shared.observation.requested = true;
            shared.observation.ready = false;
            shared.observation.gpu_readback_received = false;
            shared.observation.idle_render_frames = 0;
        }
        first && shared.render_backend
    };
    if needs_readback {
        world.spawn(Screenshot::primary_window()).observe(
            |_: On<ScreenshotCaptured>, ready: Res<RenderExitReadiness>| {
                ready.readback();
            },
        );
    }
    Ok(())
}
// IDA: N/A a helper, with no address of its own.
fn report_pipelines(cache: Res<PipelineCache>, ready: Res<RenderExitReadiness>) {
    let mut shared = ready.0.lock().expect("render exit readiness lock");
    let mut count = 0;
    let mut pending = 0;
    let mut errors = 0;
    for pipeline in cache.pipelines() {
        count += 1;
        match &pipeline.state {
            CachedPipelineState::Queued | CachedPipelineState::Creating(_) => pending += 1,
            CachedPipelineState::Err(_) => errors += 1,
            CachedPipelineState::Ok(_) => {}
        }
    }
    // Includes queued cache work not yet materialized in the public pipeline
    // slice; this also keeps compilation retry states visible to the frontend.
    pending = pending.max(cache.waiting_pipelines().count());
    let requested_after = shared.requested_after;
    let observation = &mut shared.observation;
    observation.render_frames += 1;
    observation.pipeline_count = count;
    observation.pending_pipelines = pending;
    observation.pipeline_errors = errors;
    if observation.requested
        && observation.render_frames > requested_after
        && observation.gpu_readback_received
        && pending == 0
    {
        observation.idle_render_frames = observation.idle_render_frames.saturating_add(1);
    } else {
        observation.idle_render_frames = 0;
    }
    observation.ready = observation.requested && observation.idle_render_frames >= 2;
}
// IDA: N/A a helper, with no address of its own.
fn finish_exit(world: &mut World) {
    let Some(ready) = world.get_resource::<RenderExitReadiness>().cloned() else {
        return;
    };
    let code = {
        let mut shared = ready.0.lock().expect("render exit readiness lock");
        if shared.render_backend && !shared.observation.ready {
            None
        } else {
            shared.code.take()
        }
    };
    if let Some(code) = code {
        world.write_message(code);
    }
}
