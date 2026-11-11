//! IDA: N/A the real drawing of 0057F170: `oblivion_sim::tes::interface_draw`'s list on the window, in Bevy. Blending is D3D9's: `CompositingSpace::Srgb` keeps the target in gamma space, so an alpha blend works on the same numbers it does in the original. The screens' sequence itself is `oblivion_sim::tes::winmain::winmain_screens`. Input comes from Bevy's window, a recorded substitution for DirectInput's buffers on the windowed path. Device calls in the original (the accumulator, render targets, the shader's states) have no counterpart here.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::Ordering;

use bevy::asset::RenderAssetUsages;
use bevy::asset::{AssetPath, embedded_asset, embedded_path};
use bevy::camera::{CompositingSpace, ScalingMode};
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::AsBindGroup;
use bevy::render::render_resource::{Extent3d, TextureDataOrder, TextureDimension, TextureFormat};
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured, save_to_disk};
use bevy::shader::ShaderRef;
use bevy::sprite_render::{AlphaMode2d, Material2d, Material2dPlugin, MeshMaterial2d};
use oblivion_sim::gamebryo::pixel_data::PixelData;
use oblivion_sim::tes::interface_draw::{
    InterfaceQuad, QuadTexture, interface_draw_list, interface_view,
};
use oblivion_sim::tes::interface_frame::{InterfaceInputFrame, KEY_DOWN, KEY_ENTER, KEY_RIGHT};
use oblivion_sim::tes::interface_manager::interface_render_if_no_movie;
use oblivion_sim::tes::movie_player::{MOVIE_STATE_MENU_OWNED, MoviePlayer};
use oblivion_sim::tes::os_globals::lock_shared;
use oblivion_sim::tes::tile::TileEngine;
use oblivion_sim::tes::winmain::game_frame::{FrameSet, GameFrame};
use oblivion_sim::tes::winmain::winmain_screens::{ScreenDirector, ScreenStage, run_screens_frame};
use oblivion_sim::tes::winmain::{
    WinMainProgress, driver_files, in_scene_flag, quit_requested, set_in_scene,
    take_render_requests, with_driver_interface,
};

use crate::dds_image::decode_dds;
use crate::movie_frames::LatestMovieFrame;

/// IDA: N/A D3D9's fixed function: the texture stage modulates in the numbers the file holds, and the filter works on them too. Bevy's `ColorMaterial` samples an sRGB texture, so it filters in linear light; this one samples them raw and writes them raw into `CompositingSpace::Srgb`'s gamma target. `texture: None` draws the vertex colour alone, on the white fallback.
#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct GammaMaterial {
    #[texture(0)]
    #[sampler(1)]
    pub texture: Option<Handle<Image>>,
}

impl Material2d for GammaMaterial {
    /// IDA: N/A `GammaMaterial`'s side of `Material2d`.
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("gamma_material.wgsl")).with_source("embedded"),
        )
    }

    /// IDA: N/A `GammaMaterial`'s side of `Material2d`.
    fn alpha_mode(&self) -> AlphaMode2d {
        AlphaMode2d::Blend
    }
}

/// IDA: N/A D3D9 puts a pixel's center at its whole coordinates, and wgpu at +0.5. So what the original's renderer draws at x lands here at x + 0.5, on both axes, with the same texels under each pixel.
const D3D9_PIXEL_CENTER: f32 = 0.5;

/// IDA: N/A a capture command: what the C++ host's `--menu-capture` and its siblings asked for.
#[derive(Clone, Debug, PartialEq)]
pub struct CapturePlan {
    pub path: PathBuf,
    pub script: CaptureScript,
    /// `--capture-frame`: the menu movie holds on this frame of its own, counted from 1, as the C++ flag's `BinkGoto` names it. A running movie can't seek, so the capture waits for the frame and holds it through 00B33424. `None` takes whatever is up.
    pub movie_frame: Option<u32>,
    /// `--new-game-view`: the script's steps run as for a capture, then instead of a screenshot and an exit the live view
    /// (`live_view`) takes over the window.
    pub live: bool,
    /// `--race-click` and `--race-set`: the race menu's tiles clicked or set, in order, before a capture's view is built.
    pub clicks: Vec<CaptureClick>,
    /// `--live-click`, `--live-drag`, `--live-shot`, `--live-resize` and `--live-exit`: steps a live view runs once it's up, one every
    /// `LIVE_STEP_FRAMES` updates, so the live player's rebuilds and the camera turn can be pictured without a mouse.
    pub live_script: Vec<LiveStep>,
}

/// IDA: N/A one scripted live step.
#[derive(Clone, Debug, PartialEq)]
pub enum LiveStep {
    /// A click, as `--race-click`'s.
    Click(CaptureClick),
    /// The race menu's camera turn: a press on the player (id 2, the tile at +0x2C) and eight held updates with the cursor
    /// moved this far in the interface's units (`005C2790`, `005C3BE0`).
    Drag(f32),
    /// A screenshot of the window to `screenshots/NAME.png`.
    Shot(String),
    /// The window resized to W x H physical pixels, as a player's resize would (`game_screen::follow_window` follows it).
    Resize(u32, u32),
    /// The app exits.
    Exit,
}

/// IDA: N/A updates between live steps.
const LIVE_STEP_FRAMES: u32 = 20;

/// IDA: N/A one `--race-click`: the tile named by its pane, child and arrow (each a tile name, spaces as in the XML; arrow
/// `-` for the child itself, as Face's Random) clicked `times` times through 0057F9F0 at one frame's end, as run-aq's
/// Appcall did; pane `Message` clicks the shown message box's button whose ID is the child (5 for a race box's sYes);
/// pane `Name` clicks the name tile and types the child's bytes through the menu's vt+0x30; or one
/// `--race-set`: the pane's child slider
/// set to `value` as the game's own slider store does (run-at's).
#[derive(Clone, Debug, PartialEq)]
pub struct CaptureClick {
    pub pane: String,
    pub child: String,
    pub arrow: String,
    pub times: u32,
    pub value: Option<f32>,
}

/// IDA: N/A how many interface frames run after a capture's clicks before its view is built.
const CLICK_SETTLE_FRAMES: u32 = 10;

/// IDA: N/A which screen, as the C++ flags named them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureScript {
    /// `--loading-capture`, this many of the bar's steps in.
    Loading(i32),
    /// `--menu-capture`.
    Menu,
    /// `--intro-capture`: while the intro thread still runs, with the LoadingMenu built behind it.
    Intro,
    /// `--menu-capture-selected`: Down once, so a button has focus.
    MenuSelected,
    /// `--message-capture`, with the button clicked by its menu ID.
    Message { button: i32, focused: bool },
    /// `--load-capture`.
    Load { confirm: bool },
    /// `--new-game-capture`: New Game clicked once the menu settles, then the view at the scripts' stop (the intro movie
    /// after the player's MoveTo).
    NewGame(NewGameView),
}

/// IDA: N/A what a New Game capture shows at the stop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NewGameView {
    /// The prison's loaded references (`world_view`), with no interface over them, from the player.
    Prison,
    /// The same from above and behind the drawn geometry.
    PrisonOverview,
    /// The same from close in front of the reference with this form ID.
    Actor(u32),
    /// A menu `0057E150` built (1003 Stats, 1002 Inventory, 1022 Magic, 1023 Map), by its ID: its tiles' quads alone over
    /// the prison view from the player.
    Menu(u32),
}

/// IDA: N/A a New Game capture's view at the stop: the screens' draw keeps only this menu's quads, or draws nothing.
#[derive(Resource, Clone, Copy, Debug)]
enum NewGameShot {
    OnlyMenu(oblivion_sim::tes::tile::MenuId),
    NoScreens,
}

/// IDA: N/A a capture's progress.
#[derive(Resource)]
struct CaptureState {
    plan: CapturePlan,
    frames_settled: u32,
    step: u32,
    taken: bool,
    ticks: u32,
    /// Updates since the movie held, so its picture has reached the screen.
    held_for: u32,
    /// Updates since the live view came up, for `live_script`.
    live_ticks: u32,
}

/// IDA: N/A how long a capture may wait, in updates.
const CAPTURE_FRAME_LIMIT: u32 = 2400;

/// IDA: N/A the textures the draw has made, by the file's path or the font's entity, and each's two sampler forms; and the pool of draw entities.
#[derive(Resource, Default)]
struct ScreenCache {
    materials: HashMap<(TextureKey, bool), Handle<GammaMaterial>>,
    failed: HashSet<TextureKey>,
    entities: Vec<Entity>,
    last_list: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum TextureKey {
    None,
    File(Vec<u8>),
    Font(Entity),
    Preview(usize),
    Movie,
}

/// IDA: N/A marks the camera.
#[derive(Component)]
struct ScreenCamera;

/// IDA: N/A marks a batch's mesh.
#[derive(Component)]
struct ScreenBatch;

/// IDA: N/A the host's plugin for the screens.
pub struct ScreensPlugin {
    pub capture: Option<CapturePlan>,
}

impl Plugin for ScreensPlugin {
    /// IDA: N/A Rust's `Plugin`.
    fn build(&self, app: &mut App) {
        crate::world_view::plugin(app);
        app.add_plugins(crate::game_screen::GameScreenPlugin);
        embedded_asset!(app, "gamma_material.wgsl");
        app.add_plugins(Material2dPlugin::<GammaMaterial>::default());
        app.add_plugins(bevy::pbr::MaterialPlugin::<crate::world_view::WorldMaterial>::default());
        app.add_plugins(bevy::pbr::MaterialPlugin::<crate::world_view::HairMaterial>::default());
        app.add_plugins(bevy::pbr::MaterialPlugin::<
            crate::world_view::FaceGenMaterial,
        >::default());
        app.init_resource::<ScreenCache>();
        // The capture's script drives the game, so its state lives with the game (restructure Stage 8a).
        let mut sim = app
            .world_mut()
            .remove_non_send::<crate::winmain_plugin::SimWorld>()
            .expect("WinMainPlugin builds first");
        if let Some(plan) = &self.capture {
            if matches!(
                plan.script,
                CaptureScript::Loading(_) | CaptureScript::Intro
            ) {
                sim.0
                    .world_mut()
                    .insert_resource(crate::winmain_plugin::HoldChainAfter(
                        oblivion_sim::tes::winmain::Block::Interface,
                    ));
            }
            sim.0.world_mut().insert_resource(CaptureState {
                plan: plan.clone(),
                frames_settled: 0,
                step: 0,
                taken: false,
                ticks: 0,
                held_for: 0,
                live_ticks: 0,
            });
        }
        sim.0
            .add_systems(GameFrame, run_screens.in_set(FrameSet::Interface));
        app.insert_non_send(sim);
        app.add_systems(Startup, spawn_camera);
        app.add_systems(
            Update,
            (draw_screens, run_capture, raise_world_view)
                .chain()
                .after(crate::winmain_plugin::run_winmain_frame)
                .before(crate::audio::apply_host_audio),
        );
        // Measured on the Stage 6 host: the optional exit system changes which
        // side of audio application and movie publication the live script runs on.
        let live = run_live_script.after(run_capture);
        if app.world().contains_resource::<crate::ExitAfter>() {
            app.add_systems(Update, live.before(crate::audio::apply_host_audio));
        } else {
            app.add_systems(Update, live.after(crate::movie_frames::publish_movie_frame));
        }
    }
}

// IDA: N/A a helper, with no address of its own.
fn spawn_camera(mut commands: Commands) {
    commands.spawn((
        Camera2d,
        ScreenCamera,
        CompositingSpace::Srgb,
        Msaa::Off,
        Camera {
            clear_color: ClearColorConfig::Custom(Color::BLACK),
            ..default()
        },
    ));
}

/// IDA: N/A `RawInputFrame` as the frame 005821F0 reads: the window's own cursor for 0057F490, as the windowed path does, and each key's press in the order 0057F7A0 would answer. A recorded substitution for 00403C90's buffered events.
fn read_input(world: &mut World) -> InterfaceInputFrame {
    let mut frame = InterfaceInputFrame::default();
    // The host's copy of the window's input (`HostRawInput`, written before the step); the pointer as 0..1 of the window
    // (`GameScreen`: any size or scale factor).
    let Some(input) = world
        .get_resource::<crate::winmain_plugin::HostRawInput>()
        .cloned()
    else {
        return frame;
    };
    let screen = input.screen;
    let Some(raw) = input.raw.as_ref() else {
        return frame;
    };
    if let (Some(screen), Some(position)) = (screen, raw.cursor) {
        frame.cursor = screen.pointer_fraction(position);
    }
    frame.left_pressed = raw.button_just_pressed(0);
    frame.left_held = raw.mouse_buttons[0] & 0x80 != 0;
    frame.left_released = raw.previous_mouse_buttons[0] & 0x80 != 0 && !frame.left_held;
    frame.wheel = raw.wheel;
    frame.shift = raw.held(0x2A) || raw.held(0x36);
    // 004035A8: DIK_ESCAPE, just pressed.
    frame.escape = raw.just_pressed(0x01);
    // Each key's release and press through 0057F7A0, in place of 00403C90's buffered events: the releases, then the
    // modifiers' presses (so a shift pressed with a letter in one frame shifts it), then the other presses in scan-code
    // order. A press that translates to 0 (a modifier, Escape, '|') isn't a key.
    let released: Vec<u8> = (0..=255u8)
        .filter(|&dik| raw.previous_keyboard[dik as usize] & 0x80 != 0 && !raw.held(dik))
        .collect();
    const MODIFIERS: [u8; 6] = [0x2A, 0x36, 0x1D, 0x9D, 0x38, 0xB8];
    let mut pressed: Vec<u8> = MODIFIERS
        .iter()
        .copied()
        .filter(|&dik| raw.just_pressed(dik))
        .collect();
    pressed.extend((0..=255u8).filter(|&dik| raw.just_pressed(dik) && !MODIFIERS.contains(&dik)));
    let layout = world
        .get_resource::<oblivion_sim::tes::settings::SettingCollections>()
        .and_then(|settings| settings.ini.find_address(0x00B0_2C44))
        .map_or(0, |setting| setting.raw);
    let ticks = oblivion_sim::tes::window::time_info_ticks_10(world).unwrap_or(0);
    if let Some(mut manager) =
        world.get_resource_mut::<oblivion_sim::tes::interface_manager::InterfaceManager>()
    {
        for dik in released {
            oblivion_sim::tes::interface_keys::translate_key_event(
                &mut manager,
                false,
                u32::from(dik),
                layout,
                ticks,
            );
        }
        for dik in pressed {
            let key = oblivion_sim::tes::interface_keys::translate_key_event(
                &mut manager,
                true,
                u32::from(dik),
                layout,
                ticks,
            );
            if key != 0 {
                frame.keys.push(key);
            }
        }
    }
    frame
}

/// IDA: N/A the script's frame, for a capture: no pointer, and the keys it needs. Each step waits for a settled menu. A click first walks focus with Right to the button's ID, however many that takes: whether Continue shows depends on the saves.
fn scripted_input(world: &mut World) -> InterfaceInputFrame {
    let mut frame = InterfaceInputFrame::default();
    let (script, step, settled_frames) = {
        let Some(state) = world.get_resource::<CaptureState>() else {
            return frame;
        };
        (state.plan.script, state.step, state.frames_settled)
    };
    if settled_frames < 30 || matches!(script, CaptureScript::NewGame(_)) {
        return frame;
    }
    let button = match script {
        CaptureScript::Message { button, .. } => button,
        CaptureScript::Load { .. } => 4,
        _ => 0,
    };
    let mut advance = true;
    match (script, step) {
        (CaptureScript::MenuSelected, 0) => frame.keys.push(KEY_DOWN),
        (CaptureScript::Message { .. } | CaptureScript::Load { .. }, 0) => {
            frame.keys.push(KEY_DOWN)
        }
        (CaptureScript::Message { .. } | CaptureScript::Load { .. }, 1) => {
            let focus = with_driver_interface(world, |cx| {
                let alt = cx
                    .world
                    .resource::<oblivion_sim::tes::interface_manager::InterfaceManager>()
                    .alt_active_tile_088;
                alt.map(|tile| {
                    oblivion_sim::tes::tile_values::tile_numeric_trait(
                        cx,
                        tile,
                        oblivion_sim::tes::tile_values::TRAIT_ID,
                    ) as i32
                })
            })
            .flatten();
            if focus == Some(button) {
                frame.keys.push(KEY_ENTER);
            } else {
                frame.keys.push(KEY_RIGHT);
                advance = false;
            }
        }
        (CaptureScript::Message { focused: true, .. }, 2) => frame.keys.push(KEY_DOWN),
        (CaptureScript::Load { confirm: true }, 2) => frame.keys.push(KEY_ENTER),
        _ => {}
    }
    let mut state = world.resource_mut::<CaptureState>();
    state.frames_settled = 0;
    if advance {
        state.step += 1;
    }
    frame
}

/// IDA: N/A a frame of `run_screens_frame`, with a stop reported once and the app kept up, as a chain's stop is.
fn run_screens(world: &mut World) {
    // A capture drives the screens by its script; a live view (`--new-game-view`) takes the window's input.
    let input = if world
        .get_resource::<CaptureState>()
        .is_some_and(|state| !state.plan.live)
    {
        scripted_input(world)
    } else {
        read_input(world)
    };
    if world.get_resource::<CaptureState>().is_some_and(|state| {
        matches!(
            state.plan.script,
            CaptureScript::Loading(_) | CaptureScript::Intro
        )
    }) {
        return;
    }
    if let Err(error) = run_screens_frame(world, &input) {
        report_screen_stop(world, &error);
        // Reported once: the app stays up with no more interface frames.
        world.resource_mut::<ScreenDirector>().stage = ScreenStage::NoMenu;
    }
    if quit_requested(world) {
        world.resource_mut::<WinMainProgress>().exit_code = Some(0);
    }
}

/// IDA: N/A a stop in the screens, reported as a chain's is, with the app kept up.
pub fn report_screen_stop(world: &mut World, error: &oblivion_core::error::GameError) {
    let text = format!("[ERROR] the screens stopped: {error:?}");
    println!("{text}");
    world.resource_mut::<WinMainProgress>().stops.push(
        oblivion_sim::tes::winmain::HostStopReport {
            block: oblivion_sim::tes::winmain::Block::Finished,
            text,
        },
    );
}

type QuadCorners = [[f32; 2]; 4];

// IDA: N/A CPU clipping of an axis-aligned quad to the scissor, as the device's scissor test clips it; UVs follow. `None` when nothing's left.
fn clip_quad(quad: &InterfaceQuad) -> Option<(QuadCorners, QuadCorners)> {
    let Some(scissor) = quad.scissor else {
        return Some((quad.positions, quad.uvs));
    };
    let [p0, _, _, p3] = quad.positions;
    let [u0, _, _, u3] = quad.uvs;
    let (left, right) = (p0[0].min(p3[0]), p0[0].max(p3[0]));
    let (top, bottom) = (p0[1].min(p3[1]), p0[1].max(p3[1]));
    let (cleft, cright) = (left.max(scissor[0]), right.min(scissor[2]));
    let (ctop, cbottom) = (top.max(scissor[1]), bottom.min(scissor[3]));
    if cleft >= cright || ctop >= cbottom || left >= right || top >= bottom {
        return None;
    }
    let lerp = |from: f32, to: f32, at: f32| from + (to - from) * at;
    let (ul, ur) = (
        lerp(u0[0], u3[0], (cleft - left) / (right - left)),
        lerp(u0[0], u3[0], (cright - left) / (right - left)),
    );
    let (vt, vb) = (
        lerp(u0[1], u3[1], (ctop - top) / (bottom - top)),
        lerp(u0[1], u3[1], (cbottom - top) / (bottom - top)),
    );
    Some((
        [
            [cleft, ctop],
            [cleft, cbottom],
            [cright, ctop],
            [cright, cbottom],
        ],
        [[ul, vt], [ul, vb], [ur, vt], [ur, vb]],
    ))
}

// IDA: N/A an RGBA image with its mips and sampler.
fn make_image(width: u32, height: u32, levels: Vec<Vec<u8>>, wrap: bool) -> Image {
    let mip_level_count = levels.len() as u32;
    let first = levels[0].clone();
    let all: Vec<u8> = levels.into_iter().flatten().collect();
    let mut image = Image::new(
        Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        first,
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.data = Some(all);
    image.data_order = TextureDataOrder::MipMajor;
    image.texture_descriptor.mip_level_count = mip_level_count;
    let address = if wrap {
        ImageAddressMode::Repeat
    } else {
        ImageAddressMode::ClampToEdge
    };
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: address,
        address_mode_v: address,
        address_mode_w: address,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        ..default()
    });
    image
}

/// IDA: N/A a save's screenshot as an image. Host: a size the device can't hold, from a hostile `.ess`, is no image, since wgpu stops the app on one. 8192 is wgpu's default limit; the game writes 256 by 192.
#[must_use]
fn preview_image(
    preview: &oblivion_sim::tes::interface_scene::SavePreview,
    wrap: bool,
) -> Option<Image> {
    if !(1..=8192).contains(&preview.width) || !(1..=8192).contains(&preview.height) {
        return None;
    }
    let mut rgba = Vec::with_capacity(preview.rgb.len() / 3 * 4);
    for texel in preview.rgb.as_chunks::<3>().0 {
        rgba.extend_from_slice(texel);
        rgba.push(255);
    }
    Some(make_image(preview.width, preview.height, vec![rgba], wrap))
}

// IDA: N/A the material for a quad, made at first use. A file that doesn't decode is reported once and drawn as a plain quad, which is the exe's NiTexturingProperty with no base map.
fn material_for(
    host: &mut World,
    sim: &mut World,
    texture: &QuadTexture,
    wrap: bool,
) -> (Handle<GammaMaterial>, bool) {
    let key = match texture {
        QuadTexture::None => TextureKey::None,
        QuadTexture::File(path) => TextureKey::File(path.clone()),
        QuadTexture::Font(entity) => TextureKey::Font(*entity),
        QuadTexture::Preview(preview) => {
            TextureKey::Preview(std::sync::Arc::as_ptr(preview) as usize)
        }
    };
    if let Some(handle) = host
        .resource::<ScreenCache>()
        .materials
        .get(&(key.clone(), wrap))
    {
        return (handle.clone(), key == TextureKey::None);
    }
    let image = match texture {
        QuadTexture::None => None,
        QuadTexture::File(path) => {
            let files = driver_files(sim);
            let text: String = path.iter().map(|&byte| char::from(byte)).collect();
            let bytes = files.and_then(|files| {
                let mut file = files.open_core(&text, 0, 0x2800, 0xFFFF)?;
                let mut bytes = vec![0; file.size() as usize];
                let _ = file.read(&mut bytes);
                Some(bytes)
            });
            match bytes.as_deref().and_then(decode_dds) {
                Some(decoded) => Some(make_image(
                    decoded.width,
                    decoded.height,
                    decoded.levels,
                    wrap,
                )),
                None => {
                    let mut cache = host.resource_mut::<ScreenCache>();
                    if cache.failed.insert(key.clone()) {
                        println!("[WARNING] screens: can't draw texture {text}");
                    }
                    None
                }
            }
        }
        QuadTexture::Font(entity) => sim.get::<PixelData>(*entity).map(|pixels| {
            make_image(
                pixels.widths_54[0],
                pixels.heights_58[0],
                vec![pixels.level(0, 0).to_vec()],
                wrap,
            )
        }),
        QuadTexture::Preview(preview) => preview_image(preview, wrap),
    };
    let image = image.map(|image| host.resource_mut::<Assets<Image>>().add(image));
    let material = GammaMaterial { texture: image };
    let plain = material.texture.is_none();
    let handle = host.resource_mut::<Assets<GammaMaterial>>().add(material);
    host.resource_mut::<ScreenCache>()
        .materials
        .insert((key, wrap), handle.clone());
    (handle, plain)
}

/// IDA: 0057F170's render, on a request or a menu's frame: the list, then one mesh per run of a texture, in order, each a step nearer the camera. g_dx9Renderer +0x200 stays set around it, so 00410390 waits as it does in the original.
#[allow(clippy::too_many_lines)]
fn draw_screens(world: &mut World) {
    crate::winmain_plugin::with_sim(world, draw_screens_in);
}

// IDA: N/A `draw_screens` with both worlds: the list and textures from the game's, the batches into the host's.
fn draw_screens_in(host: &mut World, sim: &mut World) {
    let requests = take_render_requests(sim);
    let menu_running = sim
        .get_resource::<ScreenDirector>()
        .is_some_and(|director| director.stage != ScreenStage::Loading);
    // 00579260 gates every tile draw on !IsRunning, so before the menu the tiles change only on a request. While a movie thread runs, its picture alone is up. Otherwise the last list stays, as a presented frame does.
    let shot = host.get_resource::<NewGameShot>().copied();
    if matches!(shot, Some(NewGameShot::NoScreens)) {
        let old = std::mem::take(&mut host.resource_mut::<ScreenCache>().entities);
        for entity in old {
            host.despawn(entity);
        }
        return;
    }
    let draw_tiles = !requests.is_empty() || menu_running || shot.is_some();
    if !requests.is_empty() && !menu_running && host.resource::<ScreenCache>().last_list.is_none() {
        println!("[INFO] LoadingMenu: first frame drawn from a request");
    }
    // Nothing to draw before 0040EB18, or when the chain stopped short of it.
    if !sim.contains_resource::<TileEngine>()
        || !sim.contains_resource::<oblivion_sim::tes::interface_manager::InterfaceManager>()
    {
        return;
    }
    let flag = in_scene_flag(sim);
    if let Some(flag) = &flag {
        set_in_scene(flag, true);
    }
    let Some((quads, view, movie_running)) = with_driver_interface(sim, |cx| {
        let view = interface_view(cx);
        let quads = if draw_tiles {
            interface_draw_list(cx.world, cx.engine, &view)
        } else {
            Vec::new()
        };
        (quads, view, cx.host.movie_player_is_running())
    }) else {
        return;
    };
    // The C++ captures have no cursor: the window never had a pointer over it. Only for a capture.
    let mut quads = quads;
    if sim.contains_resource::<CaptureState>() {
        let cursor = sim
            .get_resource::<oblivion_sim::tes::interface_manager::InterfaceManager>()
            .and_then(|manager| manager.cursor_01c);
        quads.retain(|quad| Some(quad.tile) != cursor);
    }
    // A capture of one menu draws that menu alone; the live view draws every menu up, so a box the menu opens (Random's,
    // Reset's, Done's) shows over it.
    let live = sim
        .get_resource::<CaptureState>()
        .is_some_and(|state| state.plan.live);
    if let Some(NewGameShot::OnlyMenu(menu)) = shot
        && !live
    {
        let engine = sim.resource::<TileEngine>();
        quads.retain(|quad| engine.containing_menu(quad.tile) == Some(menu));
    }
    if !draw_tiles && !movie_running {
        if let Some(flag) = &flag {
            set_in_scene(flag, false);
        }
        return;
    }
    // The camera's scale: renderer pixels over the window, whatever its real size.
    let mut cameras = host.query_filtered::<&mut Projection, With<ScreenCamera>>();
    for mut projection in cameras.iter_mut(host) {
        if let Projection::Orthographic(orthographic) = &mut *projection {
            orthographic.scaling_mode = ScalingMode::Fixed {
                width: view.renderer_width,
                height: view.renderer_height,
            };
        }
    }
    let mut batches: Vec<(Handle<GammaMaterial>, Mesh)> = Vec::new();
    let mut current: Option<(QuadTexture, bool)> = None;
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut uvs: Vec<[f32; 2]> = Vec::new();
    let mut colors: Vec<[f32; 4]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let flush = |host: &mut World,
                 sim: &mut World,
                 batches: &mut Vec<(Handle<GammaMaterial>, Mesh)>,
                 current: &Option<(QuadTexture, bool)>,
                 positions: &mut Vec<[f32; 3]>,
                 uvs: &mut Vec<[f32; 2]>,
                 colors: &mut Vec<[f32; 4]>,
                 indices: &mut Vec<u32>| {
        let Some((texture, wrap)) = current else {
            return;
        };
        if positions.is_empty() {
            return;
        }
        let (material, _) = material_for(host, sim, texture, *wrap);
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD,
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, std::mem::take(positions));
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, std::mem::take(uvs));
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, std::mem::take(colors));
        mesh.insert_indices(Indices::U32(std::mem::take(indices)));
        batches.push((material, mesh));
    };
    for quad in &quads {
        if quad.color[3] <= 0.0 {
            continue;
        }
        let Some((corners, quad_uvs)) = clip_quad(quad) else {
            continue;
        };
        let wrap = quad.clamp == 3;
        let same = current
            .as_ref()
            .is_some_and(|(texture, was)| *texture == quad.texture && *was == wrap);
        if !same {
            flush(
                host,
                sim,
                &mut batches,
                &current,
                &mut positions,
                &mut uvs,
                &mut colors,
                &mut indices,
            );
            current = Some((quad.texture.clone(), wrap));
        }
        let color = quad.color;
        let base = positions.len() as u32;
        for (corner, uv) in corners.iter().zip(quad_uvs) {
            positions.push([
                corner[0] + D3D9_PIXEL_CENTER - view.renderer_width * 0.5,
                view.renderer_height * 0.5 - (corner[1] + D3D9_PIXEL_CENTER),
                0.0,
            ]);
            uvs.push(uv);
            colors.push(color);
        }
        // 00573F10 and 00591600's triangles: 0 1 2, 2 1 3.
        indices.extend([base, base + 1, base + 2, base + 2, base + 1, base + 3]);
    }
    flush(
        host,
        sim,
        &mut batches,
        &current,
        &mut positions,
        &mut uvs,
        &mut colors,
        &mut indices,
    );
    // Under everything, the movie's picture: 00410810 runs before 0070E0A0, at its own place and size. The window clips what runs past it.
    let movie = host
        .get_resource::<LatestMovieFrame>()
        .cloned()
        .filter(|_| shot.is_none());
    let mut layers: Vec<(Handle<GammaMaterial>, Mesh)> = Vec::new();
    if let Some(movie) = movie.filter(|movie| movie.visible) {
        let existing = host
            .resource::<ScreenCache>()
            .materials
            .get(&(TextureKey::Movie, false))
            .cloned();
        let material = existing.unwrap_or_else(|| {
            let handle = host
                .resource_mut::<Assets<GammaMaterial>>()
                .add(GammaMaterial {
                    texture: Some(movie.image.clone()),
                });
            host.resource_mut::<ScreenCache>()
                .materials
                .insert((TextureKey::Movie, false), handle.clone());
            handle
        });
        // A new movie's size makes a new image.
        {
            let mut materials = host.resource_mut::<Assets<GammaMaterial>>();
            let stale = materials
                .get(&material)
                .is_some_and(|known| known.texture.as_ref() != Some(&movie.image));
            if stale && let Some(mut known) = materials.get_mut(&material) {
                known.texture = Some(movie.image.clone());
            }
        }
        let (w, h) = (view.renderer_width * 0.5, view.renderer_height * 0.5);
        let (left, top) = (
            movie.screen_position.x + D3D9_PIXEL_CENTER - w,
            h - (movie.screen_position.y + D3D9_PIXEL_CENTER),
        );
        let (right, bottom) = (left + movie.screen_size.x, top - movie.screen_size.y);
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD,
        );
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_POSITION,
            vec![
                [left, top, 0.0],
                [left, bottom, 0.0],
                [right, top, 0.0],
                [right, bottom, 0.0],
            ],
        );
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_UV_0,
            vec![[0.0, 0.0], [0.0, 1.0], [1.0, 0.0], [1.0, 1.0]],
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[1.0f32; 4]; 4]);
        mesh.insert_indices(Indices::U32(vec![0, 1, 2, 2, 1, 3]));
        layers.push((material, mesh));
    }
    layers.extend(batches);
    // The pool: an entity a batch, moved up in z so the order is the list's.
    let old = std::mem::take(&mut host.resource_mut::<ScreenCache>().entities);
    let mut entities = Vec::with_capacity(layers.len());
    for (index, (material, mesh)) in layers.into_iter().enumerate() {
        let handle = host.resource_mut::<Assets<Mesh>>().add(mesh);
        let z = index as f32 * 0.01;
        let entity = old.get(index).copied();
        let entity = if let Some(entity) = entity {
            host.entity_mut(entity).insert((
                Mesh2d(handle),
                MeshMaterial2d(material),
                Transform::from_xyz(0.0, 0.0, z),
            ));
            entity
        } else {
            host.spawn((
                ScreenBatch,
                Mesh2d(handle),
                MeshMaterial2d(material),
                Transform::from_xyz(0.0, 0.0, z),
            ))
            .id()
        };
        entities.push(entity);
    }
    let kept = entities.len();
    for &entity in old.iter().skip(kept) {
        host.despawn(entity);
    }
    let mut cache = host.resource_mut::<ScreenCache>();
    cache.entities = entities;
    if draw_tiles {
        cache.last_list = Some(quads.len());
    }
    if let Some(flag) = &flag {
        set_in_scene(flag, false);
    }
}

/// IDA: N/A a live view's `live_script`: once the view is up, step k runs at update (k + 1) x `LIVE_STEP_FRAMES`.
fn run_live_script(world: &mut World) {
    crate::winmain_plugin::with_sim(world, run_live_script_in);
}

// IDA: N/A `run_live_script` with both worlds: the script's state and clicks in the game's, the shots and window in the host's.
fn run_live_script_in(host: &mut World, sim: &mut World) {
    let Some(state) = sim.get_resource::<CaptureState>() else {
        return;
    };
    if !state.plan.live || !state.taken || state.plan.live_script.is_empty() {
        return;
    }
    let ticks = {
        let mut state = sim.resource_mut::<CaptureState>();
        state.live_ticks += 1;
        state.live_ticks
    };
    if !ticks.is_multiple_of(LIVE_STEP_FRAMES) {
        return;
    }
    let index = (ticks / LIVE_STEP_FRAMES) as usize - 1;
    let Some(step) = sim
        .resource::<CaptureState>()
        .plan
        .live_script
        .get(index)
        .cloned()
    else {
        return;
    };
    match step {
        LiveStep::Click(click) => match apply_click(sim, &click) {
            Some(Ok(())) => println!("[INFO] live script: applied {click:?}"),
            Some(Err(error)) => report_screen_stop(sim, &error),
            None => println!("[ERROR] live script: no tile for {click:?}"),
        },
        LiveStep::Drag(dx) => {
            let done = with_driver_interface(sim, |cx| {
                let menu = cx
                    .engine
                    .tile_menu_by_id(1036)
                    .and_then(|tile| cx.engine.containing_menu(tile))?;
                let grab = match &cx.engine.menu(menu).class {
                    oblivion_sim::tes::tile::MenuClass::RaceSex(race_sex) => {
                        race_sex.player_grab_2c
                    }
                    _ => None,
                }?;
                let start = cx
                    .world
                    .resource::<oblivion_sim::tes::interface_manager::InterfaceManager>()
                    .cursor_state_020[0];
                let set_cursor = |cx: &mut oblivion_sim::tes::tile::InterfaceCx, x: f32| {
                    cx.world
                        .resource_mut::<oblivion_sim::tes::interface_manager::InterfaceManager>()
                        .cursor_state_020[0] = x;
                };
                Some((|| {
                    oblivion_sim::tes::menu_slots::menu_handle_press(cx, menu, 2, grab)?;
                    for step in 1..=8 {
                        set_cursor(cx, start + dx * step as f32 / 8.0);
                        oblivion_sim::tes::menu_slots::menu_handle_held(cx, menu, 2, grab)?;
                    }
                    set_cursor(cx, start);
                    Ok(())
                })())
            })
            .flatten();
            match done {
                Some(Ok(())) => println!("[INFO] live script: dragged the player {dx}"),
                Some(Err(error)) => report_screen_stop(sim, &error),
                None => println!("[ERROR] live script: no race menu to drag"),
            }
        }
        LiveStep::Shot(name) => {
            let _ = std::fs::create_dir_all("screenshots");
            let path = std::path::PathBuf::from(format!("screenshots/{name}.png"));
            println!("[INFO] live script: screenshot {}", path.display());
            host.spawn(bevy::render::view::screenshot::Screenshot::primary_window())
                .observe(bevy::render::view::screenshot::save_to_disk(path));
        }
        LiveStep::Resize(width, height) => {
            let mut windows = host.query::<&mut Window>();
            for mut window in windows.iter_mut(host) {
                window.resolution.set_physical_resolution(width, height);
            }
            println!("[INFO] live script: window resized to {width} x {height}");
        }
        LiveStep::Exit => {
            host.write_message(AppExit::Success);
        }
    }
}

/// IDA: N/A a capture: the script's end state held long enough to draw, a screenshot, and the exit.
fn run_capture(world: &mut World) {
    crate::winmain_plugin::with_sim(world, run_capture_in);
}

// IDA: N/A `run_capture` with both worlds: the script's state in the game's, the movie picture, views and screenshot in the host's.
fn run_capture_in(host: &mut World, sim: &mut World) {
    if !sim.contains_resource::<CaptureState>() {
        return;
    }
    // A live view that's up runs on: the deadline is for captures that never get there.
    {
        let state = sim.resource::<CaptureState>();
        if state.plan.live && state.taken {
            return;
        }
    }
    // A capture that can't finish reports where it stood and exits, rather than running on.
    let deadline = {
        let mut state = sim.resource_mut::<CaptureState>();
        state.ticks += 1;
        state.ticks == CAPTURE_FRAME_LIMIT
    };
    if deadline {
        let state = sim.resource::<CaptureState>();
        println!(
            "[ERROR] capture gave up: step {}, settled {}, movie {:?}, director {:?}, media {:?}",
            state.step,
            state.frames_settled,
            host.get_resource::<LatestMovieFrame>().map(|movie| (
                movie.visible,
                movie.generation,
                movie.presented
            )),
            sim.get_resource::<ScreenDirector>(),
            sim.get_resource::<oblivion_sim::tes::winmain::winmain_menu_media::MainMenuMedia>(),
        );
        host.write_message(AppExit::from_code(3));
        return;
    }
    let (script, step, taken, path) = {
        let state = sim.resource::<CaptureState>();
        (
            state.plan.script,
            state.step,
            state.taken,
            state.plan.path.clone(),
        )
    };
    if taken {
        return;
    }
    let ready = match script {
        CaptureScript::NewGame(view) => new_game_capture_ready(host, sim, view),
        CaptureScript::Intro => {
            let built = sim.resource::<WinMainProgress>().next
                > oblivion_sim::tes::winmain::Block::Interface;
            let running =
                with_driver_interface(sim, |cx| cx.host.movie_player_is_running()).unwrap_or(false);
            built && running
        }
        CaptureScript::Loading(tick) => {
            // The C++ `--loading-capture`: section 3 at 100, then this many 101 ms steps of 005AD980's state.
            let held = sim.resource::<WinMainProgress>().next
                > oblivion_sim::tes::winmain::Block::Interface;
            if !held {
                return;
            }
            let running =
                with_driver_interface(sim, |cx| cx.host.movie_player_is_running()).unwrap_or(false);
            if running {
                return;
            }
            let frames = {
                let mut state = sim.resource_mut::<CaptureState>();
                state.frames_settled += 1;
                state.frames_settled
            };
            if frames == 1 {
                let _ = with_driver_interface(sim, |cx| {
                    loading_capture_steps(cx, tick)?;
                    interface_render_if_no_movie(cx, 0)
                });
            }
            frames >= 30
        }
        _ => {
            let menu_visible = sim
                .get_resource::<ScreenDirector>()
                .is_some_and(|director| director.stage == ScreenStage::Menu)
                && settled(sim);
            if menu_visible {
                sim.resource_mut::<CaptureState>().frames_settled += 1;
            }
            let needed = match script {
                CaptureScript::Menu
                | CaptureScript::Loading(_)
                | CaptureScript::Intro
                | CaptureScript::NewGame(_) => 0,
                CaptureScript::MenuSelected => 1,
                CaptureScript::Message { focused: false, .. }
                | CaptureScript::Load { confirm: false } => 2,
                CaptureScript::Message { focused: true, .. }
                | CaptureScript::Load { confirm: true } => 3,
            };
            let movie_ready = hold_menu_movie_frame(sim);
            step >= needed && movie_ready && sim.resource::<CaptureState>().frames_settled >= 30
        }
    };
    if !ready {
        return;
    }
    sim.resource_mut::<CaptureState>().taken = true;
    if sim.resource::<CaptureState>().plan.live {
        if let Some(mut views) = host.get_resource_mut::<crate::live_view::LiveViews>() {
            views.active = true;
        }
        println!(
            "[INFO] live view up: F1 tuning panel, PageDown/PageUp views, F9 fly camera, F12 screenshot"
        );
        return;
    }
    println!(
        "capturing {} movie frame {:?}, picture {:?}",
        path.display(),
        menu_movie_frame(sim),
        host.get_resource::<LatestMovieFrame>().map(|movie| (
            movie.visible,
            movie.generation,
            movie.presented,
            movie.size,
            movie.screen_position,
            movie.screen_size
        ))
    );
    host.spawn(Screenshot::primary_window())
        .observe(save_to_disk(path))
        .observe(
            |_: On<ScreenshotCaptured>, mut exits: MessageWriter<AppExit>| {
                exits.write(AppExit::Success);
            },
        );
}

/// IDA: N/A `--press-new-game`: a plain run presses the main menu's New Game itself once the menu has been up 200 frames and 30 more updates
/// (`settled` counts those, `pressed` says it's done), for a check with no mouse. Nothing else of a capture runs.
/// `--press-new-game-shot PATH` also writes the window to `shot` 60 updates after the world view is up, and exits.
#[derive(Resource, Debug, Default)]
pub struct PressNewGame {
    pub settled: u32,
    pub pressed: bool,
    pub shot: Option<std::path::PathBuf>,
    pub view_frames: u32,
}

// IDA: N/A `PressNewGame`'s one press, by the capture's own step 0: 00589420's button through `main_menu_new_game`.
fn press_new_game(host: &mut World, sim: &mut World) {
    let Some(mut press) = host.get_resource_mut::<PressNewGame>() else {
        return;
    };
    if press.pressed {
        return;
    }
    let menu_visible = sim
        .get_resource::<ScreenDirector>()
        .is_some_and(|director| director.stage == ScreenStage::Menu && director.frames >= 200);
    // Not `settled`: with no menu movies in the INI the interface's fader list never empties.
    if !menu_visible {
        return;
    }
    press.settled += 1;
    if press.settled < 30 {
        return;
    }
    press.pressed = true;
    match with_driver_interface(sim, oblivion_sim::tes::menus::main_menu_new_game) {
        Some(Ok(_)) => {}
        Some(Err(error)) => report_screen_stop(sim, &error),
        None => println!("[ERROR] --press-new-game: no interface for New Game"),
    }
}

/// IDA: N/A a plain run (no capture, no `--new-game-view`): once New Game's scripts have moved the player into the cell,
/// the cell's view goes up behind the interface and the live mirror follows the game's frames, as the live view's does.
/// Host substitution for the original drawing its world scene graph every frame.
fn raise_world_view(world: &mut World) {
    if world.contains_resource::<crate::world_view::RenderMirror>() {
        let Some(mut press) = world.get_resource_mut::<PressNewGame>() else {
            return;
        };
        press.view_frames += 1;
        if press.view_frames == 60
            && let Some(path) = press.shot.take()
        {
            println!("[INFO] --press-new-game-shot: {}", path.display());
            world
                .spawn(Screenshot::primary_window())
                .observe(save_to_disk(path))
                .observe(
                    |_: On<ScreenshotCaptured>, mut exits: MessageWriter<AppExit>| {
                        exits.write(AppExit::Success);
                    },
                );
        }
        return;
    }
    crate::winmain_plugin::with_sim(world, |host, sim| {
        if sim.contains_resource::<CaptureState>() {
            return;
        }
        press_new_game(host, sim);
        // The player is in the cell once the move's intro block has opened the race menu (0057B7E0, menu 1036). In a
        // run on the real clock the scripts then wait in menu mode at command 47, so a capture's sign (command 64
        // returned, which a seeded run reaches) comes much later or never; either one raises the view.
        let race_menu = with_driver_interface(sim, |cx| cx.engine.tile_menu_by_id(1036).is_some())
            .unwrap_or(false);
        let moved = race_menu
            || sim
                .get_resource::<oblivion_sim::tes::script_vm::ScriptVmCounts>()
                .is_some_and(|counts| counts.commands.len() >= 64);
        if !moved {
            return;
        }
        let Some(report) = crate::world_view::spawn_cell_view(
            host,
            sim,
            crate::world_view::CellViewCamera::Player,
        ) else {
            return;
        };
        println!("[INFO] world view: {report:?}");
        // The cell view's camera clears and draws first; the interface's camera draws on it without a clear.
        let mut cameras = host.query_filtered::<&mut Camera, With<ScreenCamera>>();
        for mut camera in cameras.iter_mut(host) {
            camera.clear_color = ClearColorConfig::None;
        }
        if let Some(mut views) = host.get_resource_mut::<crate::live_view::LiveViews>() {
            views.active = true;
        }
    });
}

// IDA: N/A a New Game capture's steps: New Game (00589420's button, `main_menu_new_game`) once the menu has settled 30
// updates, the scripts' run to their stop, then the view, held 30 updates so its textures reach the screen.
fn new_game_capture_ready(host: &mut World, sim: &mut World, view: NewGameView) -> bool {
    use oblivion_sim::tes::winmain::winmain_menu_media::MainMenuMedia;
    let step = sim.resource::<CaptureState>().step;
    match step {
        0 => {
            let menu_visible = sim
                .get_resource::<ScreenDirector>()
                .is_some_and(|director| director.stage == ScreenStage::Menu)
                && settled(sim);
            if !menu_visible {
                return false;
            }
            let frames = {
                let mut state = sim.resource_mut::<CaptureState>();
                state.frames_settled += 1;
                state.frames_settled
            };
            if frames >= 30 {
                match with_driver_interface(sim, oblivion_sim::tes::menus::main_menu_new_game) {
                    Some(Ok(_)) => {}
                    Some(Err(error)) => report_screen_stop(sim, &error),
                    None => println!("[ERROR] capture: no interface for New Game"),
                }
                let mut state = sim.resource_mut::<CaptureState>();
                state.step = 1;
                state.frames_settled = 0;
                // The commands run to the stop at the scripts' pace: let the capture wait for them.
                state.ticks = 0;
            }
            false
        }
        1 => {
            // Build 4: the scripts no longer stop past the move (0057C140's Big Four arm), so command 64's return counts too.
            let moved = sim
                .get_resource::<oblivion_sim::tes::script_vm::ScriptVmCounts>()
                .is_some_and(|counts| counts.commands.len() >= 64);
            if !moved
                && !sim
                    .get_resource::<MainMenuMedia>()
                    .is_some_and(|media| media.scripts_stopped)
            {
                // The move's load takes far longer than a menu capture's wait.
                let mut state = sim.resource_mut::<CaptureState>();
                state.ticks = state.ticks.min(CAPTURE_FRAME_LIMIT / 2);
                return false;
            }
            if click_capture_tiles(sim) {
                return false;
            }
            match view {
                NewGameView::Menu(id) => {
                    let menu = with_driver_interface(sim, |cx| {
                        cx.engine
                            .tile_menu_by_id(id)
                            .and_then(|tile| cx.engine.containing_menu(tile))
                    })
                    .flatten();
                    match menu {
                        Some(menu) => {
                            // Host, for the picture: 0057E150 leaves the menu hidden (visible 1) at fade 0 until it's
                            // opened, so the capture shows it as an opened one stands: visible 2, the scene graph updated
                            // (the Map's 00579F80 had culled it), and the end of 00584390's fade-in, 0057EA20(tile, 1.0).
                            let _ = with_driver_interface(sim, |cx| {
                                let tile = cx.engine.menu(menu).tile_04?;
                                Some((|| {
                                    oblivion_sim::tes::tile_values::tile_set_numeric_trait(
                                        cx,
                                        tile,
                                        oblivion_sim::tes::tile_values::TRAIT_VISIBLE,
                                        2.0,
                                    )?;
                                    oblivion_sim::tes::tile_values::tile_update_scene_graph(
                                        cx, tile, false,
                                    )?;
                                    oblivion_sim::tes::interface_frame::interface_apply_fade(
                                        cx, tile, 1.0,
                                    )
                                })())
                            });
                            host.insert_resource(NewGameShot::OnlyMenu(menu));
                            // Over the world, as an opened menu draws: the cell view's camera clears and draws first,
                            // the interface's camera draws on it without a clear.
                            if let Some(report) = crate::world_view::spawn_cell_view(
                                host,
                                sim,
                                crate::world_view::CellViewCamera::Player,
                            ) {
                                println!("[INFO] world view: {report:?}");
                                let mut cameras =
                                    host.query_filtered::<&mut Camera, With<ScreenCamera>>();
                                for mut camera in cameras.iter_mut(host) {
                                    camera.clear_color = ClearColorConfig::None;
                                }
                            }
                        }
                        None => println!("[ERROR] capture: no menu {id} at the stop"),
                    }
                }
                NewGameView::Prison | NewGameView::PrisonOverview | NewGameView::Actor(_) => {
                    host.insert_resource(NewGameShot::NoScreens);
                    let mut cameras = host.query_filtered::<&mut Camera, With<ScreenCamera>>();
                    for mut camera in cameras.iter_mut(host) {
                        camera.is_active = false;
                    }
                    let camera = match view {
                        NewGameView::PrisonOverview => crate::world_view::CellViewCamera::Overview,
                        NewGameView::Actor(id) => crate::world_view::CellViewCamera::Actor(id),
                        _ => crate::world_view::CellViewCamera::Player,
                    };
                    match crate::world_view::spawn_cell_view(host, sim, camera) {
                        Some(report) => {
                            println!("[INFO] world view: {report:?}");
                            host.insert_resource(report);
                        }
                        None => println!("[ERROR] capture: no player cell at the stop"),
                    }
                }
            }
            let mut state = sim.resource_mut::<CaptureState>();
            state.step = 2;
            false
        }
        _ => {
            let mut state = sim.resource_mut::<CaptureState>();
            state.frames_settled += 1;
            state.frames_settled >= 30
        }
    }
}

// IDA: N/A one `--race-click`, `--race-set` or `--live-click` on the race menu (or its message box and name field), as
// `click_capture_tiles` applies them: `None` when the tile isn't there.
fn apply_click(
    world: &mut World,
    click: &CaptureClick,
) -> Option<Result<(), oblivion_core::error::GameError>> {
    use oblivion_sim::tes::interface_frame::{TRAIT_CLICKED, interface_dispatch_pointer_event};
    with_driver_interface(world, |cx| {
        if click.pane == "Name" {
            // The name tile (id 10) clicked, then the child's bytes as keys through the menu's vt+0x30 (005C3DD0),
            // as 005821F0's key route delivers them.
            let menu = cx
                .engine
                .tile_menu_by_id(1036)
                .and_then(|tile| cx.engine.containing_menu(tile))?;
            let name = match &cx.engine.menu(menu).class {
                oblivion_sim::tes::tile::MenuClass::RaceSex(race_sex) => race_sex.name_30,
                _ => None,
            }?;
            if let Err(error) = interface_dispatch_pointer_event(cx, Some(name), TRAIT_CLICKED) {
                return Some(Err(error));
            }
            for &byte in click.child.as_bytes() {
                if let Err(error) =
                    oblivion_sim::tes::menu_slots::menu_handle_keyboard(cx, menu, u32::from(byte))
                {
                    return Some(Err(error));
                }
            }
            return Some(Ok(()));
        }
        if click.pane == "Message" {
            // A message box's button by its ID (MessageMenu +0x28.., IDs 1..13), as run-bg's YES5.
            let menu = cx
                .engine
                .tile_menu_by_id(1001)
                .and_then(|tile| cx.engine.containing_menu(tile))?;
            let id: usize = click.child.parse().ok()?;
            let button = match &cx.engine.menu(menu).class {
                oblivion_sim::tes::tile::MenuClass::Message(message) => {
                    *message.tiles_28.get(id.checked_sub(1)?)?
                }
                _ => None,
            }?;
            return Some(interface_dispatch_pointer_event(
                cx,
                Some(button),
                TRAIT_CLICKED,
            ));
        }
        let menu = cx
            .engine
            .tile_menu_by_id(1036)
            .and_then(|tile| cx.engine.containing_menu(tile))?;
        let toggle = oblivion_sim::tes::race_sex_menu::pane_child(
            cx,
            menu,
            click.pane.as_bytes(),
            click.child.as_bytes(),
        )
        .ok()
        .flatten()?;
        if let Some(value) = click.value {
            return Some(oblivion_sim::tes::race_sex_menu::race_sex_menu_set_slider(
                cx, toggle, value,
            ));
        }
        let arrow = if click.arrow == "-" {
            toggle
        } else {
            oblivion_sim::tes::tile_values::tile_child_named(cx, toggle, click.arrow.as_bytes())?
        };
        for _ in 0..click.times {
            if let Err(error) = interface_dispatch_pointer_event(cx, Some(arrow), TRAIT_CLICKED) {
                return Some(Err(error));
            }
        }
        Some(Ok(()))
    })
    .flatten()
}

// IDA: N/A a capture's `--race-click`s and `--race-set`s on the race menu (1036), in order, each followed by
// CLICK_SETTLE_FRAMES updates while the interface frames run: a click goes through 0057F9F0 `times` times; a set stores the
// slider as the game does (005C7BA0's user3 store). Answers true while any is left or settling; false once done (or none).
fn click_capture_tiles(world: &mut World) -> bool {
    let clicks = world.resource::<CaptureState>().plan.clicks.clone();
    if clicks.is_empty() {
        return false;
    }
    let held = world.resource::<CaptureState>().held_for;
    let window = CLICK_SETTLE_FRAMES + 1;
    let index = (held / window) as usize;
    if index >= clicks.len() {
        return false;
    }
    if held.is_multiple_of(window) {
        let click = clicks[index].clone();
        let done = apply_click(world, &click);
        match done {
            Some(Ok(())) => println!("[INFO] capture: applied {click:?}"),
            Some(Err(error)) => report_screen_stop(world, &error),
            None => println!("[ERROR] capture: no tile for {click:?}"),
        }
    }
    world.resource_mut::<CaptureState>().held_for += 1;
    true
}

// IDA: N/A the menu-owned movie's +0x0C: how many of its frames 004106C0 has copied, so the number of the one up.
fn menu_movie_frame(world: &World) -> Option<u32> {
    let player = world.get_resource::<MoviePlayer>()?;
    let current = lock_shared(&player.globals.current_b33428).clone()?;
    (current.state() == MOVIE_STATE_MENU_OWNED).then(|| current.fields().frames_shown_0c)
}

// IDA: N/A the capture's wait for its movie frame: at the frame, 00B33424 keeps 004106C0 from copying another, and a few updates carry that picture to the screen. Host-only: the original's flag, set by nothing in the original's menu path.
fn hold_menu_movie_frame(world: &mut World) -> bool {
    let Some(wanted) = world.resource::<CaptureState>().plan.movie_frame else {
        return true;
    };
    if world.resource::<CaptureState>().held_for > 0 {
        let mut state = world.resource_mut::<CaptureState>();
        state.held_for += 1;
        return state.held_for > 3;
    }
    if menu_movie_frame(world) == Some(wanted) {
        let player = world.resource::<MoviePlayer>();
        player.globals.hold_frame_b33424.store(1, Ordering::Release);
        world.resource_mut::<CaptureState>().held_for = 1;
    }
    false
}

// IDA: N/A the steps of a loading capture: 005ADEC0(3, 100), and `tick` of 005AD980's decisions at 101 ms apart, whose user3 the ribbon reads.
fn loading_capture_steps(
    cx: &mut oblivion_sim::tes::tile::InterfaceCx,
    tick: i32,
) -> Result<(), oblivion_core::error::GameError> {
    use oblivion_sim::tes::tile::MenuClass;
    use oblivion_sim::tes::tile_values::{TRAIT_USER3, tile_set_numeric_trait};
    oblivion_sim::tes::loading_menu::loading_menu_set_section_progress(cx, 3, 100.0)?;
    let Some(tile) = cx.engine.tile_menu_by_id(1007) else {
        return Ok(());
    };
    let Some(menu) = cx.engine.containing_menu(tile) else {
        return Ok(());
    };
    let limits = cx.settings.loading_move_bar_ms;
    for _ in 0..tick {
        let MenuClass::Loading(loading) = &mut cx.engine.menu_mut(menu).class else {
            break;
        };
        let now = loading.last_step_68.wrapping_add(101);
        let shown = loading.update_display_state(false, now, limits).ribbon;
        if let Some(shown) = shown {
            tile_set_numeric_trait(cx, tile, TRAIT_USER3, shown)?;
        }
    }
    Ok(())
}

// IDA: N/A no timed transition is running.
fn settled(world: &World) -> bool {
    world
        .get_resource::<oblivion_sim::tes::interface_manager::InterfaceManager>()
        .is_some_and(|manager| {
            manager
                .block_130
                .as_ref()
                .is_some_and(|block| block.nodes.is_empty())
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use oblivion_sim::tes::interface_scene::SavePreview;

    // A hostile `.ess`: a screenshot wgpu would stop the app on is refused before it gets there.
    #[test]
    fn a_hostile_preview_is_no_image() {
        let preview = |width, height| SavePreview {
            width,
            height,
            rgb: vec![0; 3 * width as usize * height as usize],
        };
        assert!(preview_image(&preview(20_000, 1), false).is_none());
        assert!(preview_image(&preview(1, 20_000), false).is_none());
        assert!(preview_image(&preview(0, 4), false).is_none());
        assert!(preview_image(&preview(4, 2), false).is_some());
    }

    /// A released key is translated with this idle frame's tick. The message callback in
    /// the same screen update queues Quit for the next host frame, before its interface.
    #[test]
    fn the_interface_uses_this_frames_tick_and_quit_reaches_the_next_frame() {
        use crate::raw_input::RawInputFrame;
        use crate::winmain_plugin::{HostExitRequest, HostRawInput};
        use oblivion_core::fixtures;
        use oblivion_sim::tes::interface_manager::InterfaceManager;
        use oblivion_sim::tes::menus::MessageCallback;
        use oblivion_sim::tes::window::time_info_ticks_10;
        use oblivion_sim::tes::winmain::{
            SettingsSource, WinMainConfig, WinMainSeams, install_winmain,
            run_winmain_straight_through, set_driver_tick, winmain_shutdown,
        };

        let mut world = World::new();
        install_winmain(
            &mut world,
            WinMainConfig {
                data_directory: fixtures::data_dir(),
                settings: SettingsSource::Pinned(fixtures::root().join("config/Oblivion.ini")),
                seed: Some(1),
                frame_budget: None,
            },
            WinMainSeams::default(),
        );
        run_winmain_straight_through(&mut world);
        assert!(world.resource::<WinMainProgress>().stops.is_empty());
        // The simulation's world as the host steps it: the raw input copied in, the exit taken out at MovieExit.
        world.init_resource::<HostRawInput>();
        world.init_resource::<HostExitRequest>();
        let mut frame = crate::winmain_plugin::host_game_frame();
        frame.add_systems(run_screens.in_set(FrameSet::Interface));
        set_driver_tick(&world, Some(5000));
        frame.run(&mut world);
        let previous = time_info_ticks_10(&world).unwrap();
        let mut raw = RawInputFrame::default();
        raw.previous_keyboard[0xCB] = 0x80;
        world.resource_mut::<HostRawInput>().raw = Some(raw);
        {
            let mut manager = world.resource_mut::<InterfaceManager>();
            manager.message_callback_0b4 = Some(MessageCallback::MainMenuExit);
            manager.msg_box_button_pressed_0b0 = 1;
        }
        set_driver_tick(&world, Some(5035));
        frame.run(&mut world);
        let updated = time_info_ticks_10(&world).unwrap();
        assert_ne!(updated, previous);
        assert_eq!(
            world.resource::<InterfaceManager>().key_pressed_tick_120,
            updated
        );
        assert!(quit_requested(&world));
        assert_eq!(world.resource::<WinMainProgress>().exit_code, Some(0));
        assert_eq!(world.resource::<HostExitRequest>().0, None);

        set_driver_tick(&world, Some(5070));
        frame.run(&mut world);
        assert_eq!(world.resource::<HostExitRequest>().0, Some(0));
        assert_eq!(
            time_info_ticks_10(&world),
            Some(updated),
            "pending exit skips idle"
        );
        winmain_shutdown(&mut world);
    }
}
