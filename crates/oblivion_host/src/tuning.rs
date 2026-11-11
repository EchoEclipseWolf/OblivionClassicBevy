//! IDA: N/A host tooling: live tuning. Any settings resource can be made tunable with one call,
//! `app.register_tunable::<T>()`; the module then loads it from the tuning file at startup (captures included), reloads it
//! when the file changes on disk, shows it in an in-game inspector panel (F1) with Save, Reload and Reset, and writes it back.
//! The module knows nothing about what a tunable controls: the owner reads the resource (`Res<T>`, `is_changed`) and applies
//! it, as `world_view`'s lighting does. The file is RON, one entry per tunable keyed by `Tunable::KEY`; entries for tunables a
//! build doesn't register are kept when it saves, so older and newer builds can share one file.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use bevy::prelude::*;
use bevy::reflect::GetTypeRegistration;
use bevy_egui::{EguiContext, EguiPlugin, EguiPrimaryContextPass, PrimaryEguiContext, egui};
use serde::Serialize;
use serde::de::DeserializeOwned;

/// IDA: N/A a group of settings the panel and the file know by a stable key. Implement it on a resource that also derives
/// `Reflect` (for the inspector), `Serialize`/`Deserialize` (for the file) and `Default` (the Reset value).
pub trait Tunable:
    Resource<Mutability = bevy::ecs::component::Mutable>
    + Reflect
    + GetTypeRegistration
    + Serialize
    + DeserializeOwned
    + Default
    + Clone
{
    /// The tuning file's key: stable across builds, so renaming the type doesn't lose saved values.
    const KEY: &'static str;
    /// The panel's heading.
    const LABEL: &'static str;
    /// The panel groups tunables under this heading ("Rendering", "Audio", ...).
    const GROUP: &'static str = "General";
}

/// IDA: N/A what the module knows about one registered tunable, as plain function pointers so it holds no generic state.
struct TunableEntry {
    key: &'static str,
    label: &'static str,
    group: &'static str,
    load: fn(&mut World, &ron::Value) -> Result<(), String>,
    save: fn(&World) -> Result<ron::Value, String>,
    reset: fn(&mut World),
    inspect: fn(&mut World, &mut egui::Ui),
}

/// IDA: N/A the registered tunables, in registration order.
#[derive(Resource, Default)]
struct TuningRegistry {
    entries: Vec<TunableEntry>,
}

/// IDA: N/A the tuning file and the panel's state.
#[derive(Resource)]
pub struct TuningState {
    /// The tuning file (`--tuning PATH`, else `config/tuning.ron`).
    pub path: PathBuf,
    /// The panel is shown (F1 toggles it).
    pub panel_open: bool,
    /// The file's last modification the module loaded or wrote, so an outside edit is noticed and its own save isn't.
    modified: Option<SystemTime>,
    /// Entries read from the file for keys no tunable here registers: written back unchanged on save.
    unknown: BTreeMap<String, ron::Value>,
    /// The last load or save's outcome, shown in the panel.
    status: String,
    check: Timer,
}

/// IDA: N/A the plugin: egui and the inspector, the registry, the file and the panel.
pub struct TuningPlugin {
    /// The tuning file; `None` takes `config/tuning.ron` in the working directory.
    pub path: Option<PathBuf>,
    /// The panel starts open (`--tuning-panel`).
    pub panel_open: bool,
}

impl Plugin for TuningPlugin {
    /// IDA: N/A Rust's `Plugin`.
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<EguiPlugin>() {
            app.add_plugins(EguiPlugin::default());
        }
        // egui draws on a camera of its own: the host's cameras keep D3D9's gamma-space target (`CompositingSpace::Srgb`,
        // `Rgba8Unorm`), which bevy_egui's pipeline (built for `Rgba8UnormSrgb`) can't draw into.
        app.world_mut()
            .resource_mut::<bevy_egui::EguiGlobalSettings>()
            .auto_create_primary_context = false;
        app.add_systems(Startup, spawn_overlay_camera);
        if !app.is_plugin_added::<bevy_inspector_egui::DefaultInspectorConfigPlugin>() {
            app.add_plugins(bevy_inspector_egui::DefaultInspectorConfigPlugin);
        }
        app.init_resource::<TuningRegistry>();
        app.insert_resource(TuningState {
            path: self
                .path
                .clone()
                .unwrap_or_else(|| PathBuf::from("config/tuning.ron")),
            panel_open: self.panel_open,
            modified: None,
            unknown: BTreeMap::new(),
            status: String::new(),
            check: Timer::new(Duration::from_millis(500), TimerMode::Repeating),
        });
        app.add_systems(PreStartup, load_tuning_file);
        app.add_systems(Update, (toggle_panel, reload_changed_file));
        app.add_systems(EguiPrimaryContextPass, tuning_panel);
    }
}

/// IDA: N/A registering a tunable on the app.
pub trait TuningAppExt {
    /// Makes `T` tunable: initialises it to `Default` (the tuning file overrides it at startup), registers it for
    /// reflection and adds it to the panel and the file.
    fn register_tunable<T: Tunable>(&mut self) -> &mut Self;
}

impl TuningAppExt for App {
    /// IDA: N/A `TuningAppExt`'s implementation.
    fn register_tunable<T: Tunable>(&mut self) -> &mut Self {
        self.register_type::<T>();
        self.init_resource::<T>();
        self.init_resource::<TuningRegistry>();
        self.world_mut()
            .resource_mut::<TuningRegistry>()
            .entries
            .push(TunableEntry {
                key: T::KEY,
                label: T::LABEL,
                group: T::GROUP,
                load: |world, value| {
                    let parsed: T = value
                        .clone()
                        .into_rust()
                        .map_err(|error| error.to_string())?;
                    world.insert_resource(parsed);
                    Ok(())
                },
                save: |world| {
                    let value = world.resource::<T>();
                    let text = ron::to_string(value).map_err(|error| error.to_string())?;
                    ron::from_str(&text).map_err(|error| error.to_string())
                },
                reset: |world| world.insert_resource(T::default()),
                inspect: |world, ui| {
                    bevy_inspector_egui::bevy_inspector::ui_for_resource::<T>(world, ui);
                },
            });
        self
    }
}

/// IDA: N/A the render layer only the tooling overlay camera draws, so it renders nothing of the game's.
pub const OVERLAY_LAYER: usize = 31;

// IDA: N/A the overlay camera: after every game camera, cleared to transparent and alpha-blended onto the window, drawing only
// egui (and whatever else is put on `OVERLAY_LAYER`).
fn spawn_overlay_camera(mut commands: Commands) {
    commands.spawn((
        Camera2d,
        Camera {
            order: 1_000,
            clear_color: ClearColorConfig::Custom(Color::NONE),
            output_mode: bevy::camera::CameraOutputMode::Write {
                blend_state: Some(bevy::render::render_resource::BlendState::ALPHA_BLENDING),
                clear_color: ClearColorConfig::None,
            },
            ..default()
        },
        // The host's cameras draw without multisampling; cameras on one window share their depth texture.
        Msaa::Off,
        bevy::camera::visibility::RenderLayers::layer(OVERLAY_LAYER),
        PrimaryEguiContext,
        Name::new("tooling overlay camera"),
    ));
}

// IDA: N/A reads the tuning file into the registered tunables; a missing file leaves the defaults.
fn load(world: &mut World) -> String {
    let path = world.resource::<TuningState>().path.clone();
    let modified = std::fs::metadata(&path)
        .and_then(|meta| meta.modified())
        .ok();
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(_) => {
            world.resource_mut::<TuningState>().modified = modified;
            return format!("no tuning file at {}: defaults", path.display());
        }
    };
    let map: BTreeMap<String, ron::Value> = match ron::from_str(&text) {
        Ok(map) => map,
        Err(error) => return format!("{}: {error}", path.display()),
    };
    let mut errors = Vec::new();
    let mut unknown = map.clone();
    world.resource_scope(|world, registry: Mut<TuningRegistry>| {
        for entry in &registry.entries {
            if let Some(value) = map.get(entry.key) {
                unknown.remove(entry.key);
                if let Err(error) = (entry.load)(world, value) {
                    errors.push(format!("{}: {error}", entry.key));
                }
            }
        }
    });
    let mut state = world.resource_mut::<TuningState>();
    state.unknown = unknown;
    state.modified = modified;
    if errors.is_empty() {
        format!("loaded {}", path.display())
    } else {
        format!(
            "loaded {} with errors: {}",
            path.display(),
            errors.join("; ")
        )
    }
}

// IDA: N/A writes every registered tunable (and the file's unknown entries) back to the tuning file.
fn save(world: &mut World) -> String {
    let mut map = world.resource::<TuningState>().unknown.clone();
    let mut errors = Vec::new();
    world.resource_scope(|world, registry: Mut<TuningRegistry>| {
        for entry in &registry.entries {
            match (entry.save)(world) {
                Ok(value) => {
                    map.insert(entry.key.to_owned(), value);
                }
                Err(error) => errors.push(format!("{}: {error}", entry.key)),
            }
        }
    });
    let path = world.resource::<TuningState>().path.clone();
    let text = match ron::ser::to_string_pretty(&map, ron::ser::PrettyConfig::default()) {
        Ok(text) => text,
        Err(error) => return format!("can't write the tuning file: {error}"),
    };
    if let Some(folder) = path
        .parent()
        .filter(|folder| !folder.as_os_str().is_empty())
    {
        let _ = std::fs::create_dir_all(folder);
    }
    if let Err(error) = std::fs::write(&path, text) {
        return format!("can't write {}: {error}", path.display());
    }
    world.resource_mut::<TuningState>().modified = std::fs::metadata(&path)
        .and_then(|meta| meta.modified())
        .ok();
    if errors.is_empty() {
        format!("saved {}", path.display())
    } else {
        format!("saved {} without: {}", path.display(), errors.join("; "))
    }
}

// IDA: N/A startup: the file's values replace the defaults before anything reads them.
fn load_tuning_file(world: &mut World) {
    let status = load(world);
    println!("[INFO] tuning: {status}");
    world.resource_mut::<TuningState>().status = status;
}

// IDA: N/A an outside edit of the file (by hand, or another instance's Save) is picked up within half a second.
fn reload_changed_file(world: &mut World) {
    let due = {
        let delta = world.resource::<Time>().delta();
        let mut state = world.resource_mut::<TuningState>();
        state.check.tick(delta).just_finished()
    };
    if !due {
        return;
    }
    let (path, known) = {
        let state = world.resource::<TuningState>();
        (state.path.clone(), state.modified)
    };
    let modified = std::fs::metadata(&path)
        .and_then(|meta| meta.modified())
        .ok();
    if modified.is_some() && modified != known {
        let status = load(world);
        println!("[INFO] tuning: {status}");
        world.resource_mut::<TuningState>().status = status;
    }
}

// IDA: N/A F1 shows or hides the panel.
fn toggle_panel(keys: Res<ButtonInput<KeyCode>>, mut state: ResMut<TuningState>) {
    if keys.just_pressed(KeyCode::F1) {
        state.panel_open = !state.panel_open;
    }
}

// IDA: N/A the panel: each group's tunables under collapsing headings, each with its inspector and a Reset; Save and Reload.
fn tuning_panel(world: &mut World) {
    if !world.resource::<TuningState>().panel_open {
        return;
    }
    let Ok(context) = world
        .query_filtered::<&mut EguiContext, With<PrimaryEguiContext>>()
        .single(world)
    else {
        return;
    };
    let mut context = context.clone();
    let mut action = None;
    egui::Window::new("Tuning (F1)")
        .default_width(360.0)
        .show(context.get_mut(), |ui| {
            ui.horizontal(|ui| {
                if ui.button("Save").clicked() {
                    action = Some("save");
                }
                if ui.button("Reload").clicked() {
                    action = Some("reload");
                }
            });
            {
                let state = world.resource::<TuningState>();
                ui.label(format!("{}", state.path.display()));
                if !state.status.is_empty() {
                    ui.small(state.status.clone());
                }
            }
            ui.separator();
            egui::ScrollArea::vertical().show(ui, |ui| {
                world.resource_scope(|world, registry: Mut<TuningRegistry>| {
                    let mut groups: Vec<&str> = Vec::new();
                    for entry in &registry.entries {
                        if !groups.contains(&entry.group) {
                            groups.push(entry.group);
                        }
                    }
                    for group in groups {
                        ui.heading(group);
                        for entry in registry.entries.iter().filter(|entry| entry.group == group) {
                            egui::CollapsingHeader::new(entry.label)
                                .id_salt(entry.key)
                                .default_open(true)
                                .show(ui, |ui| {
                                    (entry.inspect)(world, ui);
                                    if ui.small_button("Reset to defaults").clicked() {
                                        (entry.reset)(world);
                                    }
                                });
                        }
                    }
                });
            });
        });
    let status = match action {
        Some("save") => Some(save(world)),
        Some("reload") => Some(load(world)),
        _ => None,
    };
    if let Some(status) = status {
        println!("[INFO] tuning: {status}");
        world.resource_mut::<TuningState>().status = status;
    }
}
