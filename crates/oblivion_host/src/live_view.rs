//! IDA: N/A host tooling: the live view, for looking at the port's scene while tuning it. A registry of named camera
//! placements (`LiveViews`), filled by whoever builds a scene (`world_view` adds the player's camera, the overview and one per
//! actor); PageDown / PageUp step through them, F9 toggles a fly camera (WASD, Q/E down/up, Shift faster, right mouse to
//! look), and F12 writes a screenshot to `screenshots/`. The camera it moves carries `LiveViewCamera`. The fly speed and look
//! sensitivity are themselves a tunable (`LiveViewTuning`).

use std::path::PathBuf;

use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use serde::{Deserialize, Serialize};

use crate::tuning::{Tunable, TuningAppExt};

/// IDA: N/A where one named view puts the camera: eye, forward and up in Bevy's axes, computed when the view is chosen.
pub type ViewPlacement = Box<dyn Fn(&mut World) -> Option<(Vec3, Vec3, Vec3)> + Send + Sync>;

/// IDA: N/A one named view.
pub struct LiveView {
    pub name: String,
    pub place: ViewPlacement,
}

/// IDA: N/A the views, in the order PageDown steps through them, and the one shown.
#[derive(Resource, Default)]
pub struct LiveViews {
    pub views: Vec<LiveView>,
    pub current: usize,
    /// The live controls run: set once a live scene is up (captures leave it off).
    pub active: bool,
    /// The fly camera is on.
    pub flying: bool,
}

impl LiveViews {
    /// IDA: N/A adds a view; a view with the same name is replaced, so a scene rebuild can re-register its views.
    pub fn add(&mut self, name: impl Into<String>, place: ViewPlacement) {
        let name = name.into();
        if let Some(view) = self.views.iter_mut().find(|view| view.name == name) {
            view.place = place;
        } else {
            self.views.push(LiveView { name, place });
        }
    }
}

/// IDA: N/A the camera the live view moves.
#[derive(Component)]
pub struct LiveViewCamera;

/// IDA: N/A the live view's own settings.
#[derive(Resource, Reflect, Serialize, Deserialize, Clone, Debug)]
#[reflect(Resource)]
#[serde(default)]
pub struct LiveViewTuning {
    /// Fly speed in game units per second.
    pub fly_speed: f32,
    /// The multiplier Shift applies.
    pub fast_multiplier: f32,
    /// Radians per pixel of mouse movement.
    pub look_sensitivity: f32,
}

impl Default for LiveViewTuning {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            fly_speed: 300.0,
            fast_multiplier: 4.0,
            look_sensitivity: 0.003,
        }
    }
}

impl Tunable for LiveViewTuning {
    const KEY: &'static str = "live_view";
    const LABEL: &'static str = "Live view camera";
    const GROUP: &'static str = "Tools";
}

/// IDA: N/A the plugin.
pub struct LiveViewPlugin;

impl Plugin for LiveViewPlugin {
    /// IDA: N/A Rust's `Plugin`.
    fn build(&self, app: &mut App) {
        app.init_resource::<LiveViews>();
        app.register_tunable::<LiveViewTuning>();
        app.add_systems(Update, (switch_views, fly_camera, take_screenshot));
    }
}

/// IDA: N/A puts the live camera at view `index` (wrapping) and reports its name.
pub fn show_view(world: &mut World, index: usize) {
    let count = world.resource::<LiveViews>().views.len();
    if count == 0 {
        return;
    }
    let index = index % count;
    world.resource_mut::<LiveViews>().current = index;
    // The placement runs with the registry taken out, so it may read anything else in the world.
    let views = std::mem::take(&mut world.resource_mut::<LiveViews>().views);
    let placed = (views[index].place)(world);
    let name = views[index].name.clone();
    world.resource_mut::<LiveViews>().views = views;
    let Some((eye, forward, up)) = placed else {
        println!("[WARNING] live view: {name} has no placement");
        return;
    };
    let mut cameras = world.query_filtered::<&mut Transform, With<LiveViewCamera>>();
    for mut transform in cameras.iter_mut(world) {
        *transform = Transform::from_translation(eye).looking_to(forward, up);
    }
    println!("[INFO] live view: {name}");
}

// IDA: N/A PageDown / PageUp step through the views.
fn switch_views(world: &mut World) {
    if !world.resource::<LiveViews>().active {
        return;
    }
    let (next, previous) = {
        let keys = world.resource::<ButtonInput<KeyCode>>();
        (
            keys.just_pressed(KeyCode::PageDown),
            keys.just_pressed(KeyCode::PageUp),
        )
    };
    if !next && !previous {
        return;
    }
    let (current, count) = {
        let views = world.resource::<LiveViews>();
        (views.current, views.views.len())
    };
    if count == 0 {
        return;
    }
    let index = if next {
        current + 1
    } else {
        current + count - 1
    };
    show_view(world, index);
}

// IDA: N/A F9 toggles flying; WASD moves along the view, Q/E down and up, Shift faster, the right mouse button looks.
fn fly_camera(
    keys: Res<ButtonInput<KeyCode>>,
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    time: Res<Time>,
    tuning: Res<LiveViewTuning>,
    mut views: ResMut<LiveViews>,
    mut cameras: Query<&mut Transform, With<LiveViewCamera>>,
) {
    if !views.active {
        return;
    }
    if keys.just_pressed(KeyCode::F9) {
        views.flying = !views.flying;
        println!(
            "[INFO] live view: fly camera {}",
            if views.flying { "on" } else { "off" }
        );
    }
    if !views.flying {
        return;
    }
    let speed = tuning.fly_speed
        * if keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight) {
            tuning.fast_multiplier
        } else {
            1.0
        };
    for mut transform in &mut cameras {
        if buttons.pressed(MouseButton::Right) && motion.delta != Vec2::ZERO {
            let (yaw, pitch, _) = transform.rotation.to_euler(EulerRot::YXZ);
            let yaw = yaw - motion.delta.x * tuning.look_sensitivity;
            let pitch = (pitch - motion.delta.y * tuning.look_sensitivity).clamp(-1.54, 1.54);
            transform.rotation = Quat::from_euler(EulerRot::YXZ, yaw, pitch, 0.0);
        }
        let mut direction = Vec3::ZERO;
        let forward = transform.forward().as_vec3();
        let right = transform.right().as_vec3();
        for (key, step) in [
            (KeyCode::KeyW, forward),
            (KeyCode::KeyS, -forward),
            (KeyCode::KeyD, right),
            (KeyCode::KeyA, -right),
            (KeyCode::KeyE, Vec3::Y),
            (KeyCode::KeyQ, -Vec3::Y),
        ] {
            if keys.pressed(key) {
                direction += step;
            }
        }
        if direction != Vec3::ZERO {
            transform.translation += direction.normalize() * speed * time.delta_secs();
        }
    }
}

// IDA: N/A F12 writes the window to `screenshots/live-<view>-<n>.png`.
fn take_screenshot(world: &mut World) {
    if !world.resource::<LiveViews>().active
        || !world
            .resource::<ButtonInput<KeyCode>>()
            .just_pressed(KeyCode::F12)
    {
        return;
    }
    let name = {
        let views = world.resource::<LiveViews>();
        views
            .views
            .get(views.current)
            .map_or_else(|| "view".to_owned(), |view| view.name.clone())
    };
    let folder = PathBuf::from("screenshots");
    let _ = std::fs::create_dir_all(&folder);
    let safe: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let mut n = 0;
    let path = loop {
        let path = folder.join(format!("live-{safe}-{n}.png"));
        if !path.exists() {
            break path;
        }
        n += 1;
    };
    println!("[INFO] live view: screenshot {}", path.display());
    world
        .spawn(Screenshot::primary_window())
        .observe(save_to_disk(path));
}
