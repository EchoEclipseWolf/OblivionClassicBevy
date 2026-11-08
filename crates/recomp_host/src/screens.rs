//! IDA: N/A the real drawing of 0057F170: `recomp_game::tes::interface_draw`'s list on the window, in Bevy. Blending is D3D9's: `CompositingSpace::Srgb` keeps the target in gamma space, so an alpha blend works on the same numbers it does in the original. The screens' sequence itself is `recomp_startup::winmain_screens`. Input comes from Bevy's window, a recorded substitution for DirectInput's buffers on the windowed path. Device calls in the original (the accumulator, render targets, the shader's states) have no counterpart here.

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
use recomp_game::gamebryo::pixel_data::PixelData;
use recomp_game::tes::interface_draw::{
    InterfaceQuad, QuadTexture, interface_draw_list, interface_view,
};
use recomp_game::tes::interface_frame::{
    InterfaceInputFrame, KEY_DOWN, KEY_ENTER, KEY_LEFT, KEY_RIGHT, KEY_UP,
};
use recomp_game::tes::interface_manager::interface_render_if_no_movie;
use recomp_game::tes::movie_player::{MOVIE_STATE_MENU_OWNED, MoviePlayer};
use recomp_game::tes::os_globals::lock_shared;
use recomp_game::tes::tile::TileEngine;
use recomp_startup::winmain::{
    WinMainProgress, driver_files, in_scene_flag, quit_requested, set_in_scene,
    take_render_requests, with_driver_interface,
};
use recomp_startup::winmain_screens::{ScreenDirector, ScreenStage, run_screens_frame};

use crate::dds_image::decode_dds;
use crate::movie_frames::LatestMovieFrame;
use crate::raw_input::RawInputFrame;

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
}

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
    /// A menu `0057E150` built (1003 Stats, 1002 Inventory, 1022 Magic, 1023 Map), by its ID: its tiles' quads alone over
    /// the prison view from the player.
    Menu(u32),
}

/// IDA: N/A a New Game capture's view at the stop: the screens' draw keeps only this menu's quads, or draws nothing.
#[derive(Resource, Clone, Copy, Debug)]
enum NewGameShot {
    OnlyMenu(recomp_game::tes::tile::MenuId),
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
        embedded_asset!(app, "gamma_material.wgsl");
        app.add_plugins(Material2dPlugin::<GammaMaterial>::default());
        app.init_resource::<ScreenCache>();
        if let Some(plan) = &self.capture {
            if matches!(
                plan.script,
                CaptureScript::Loading(_) | CaptureScript::Intro
            ) {
                app.insert_resource(crate::winmain_plugin::HoldChainAfter(
                    recomp_startup::winmain::Block::Interface,
                ));
            }
            app.insert_resource(CaptureState {
                plan: plan.clone(),
                frames_settled: 0,
                step: 0,
                taken: false,
                ticks: 0,
                held_for: 0,
            });
        }
        app.add_systems(Startup, spawn_camera);
        app.add_systems(
            Update,
            (run_screens, draw_screens, run_capture)
                .chain()
                .after(crate::winmain_plugin::run_winmain_frame),
        );
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
    let size = {
        let mut windows = world.query::<&Window>();
        windows.iter(world).next().map(|window| {
            (
                window.physical_width() as f32,
                window.physical_height() as f32,
            )
        })
    };
    let Some(raw) = world.get_resource::<RawInputFrame>() else {
        return frame;
    };
    if let (Some((width, height)), Some(position)) = (size, raw.cursor)
        && width > 0.0
        && height > 0.0
    {
        frame.cursor = Some([position.x / width, position.y / height]);
    }
    frame.left_pressed = raw.button_just_pressed(0);
    frame.left_held = raw.mouse_buttons[0] & 0x80 != 0;
    frame.left_released = raw.previous_mouse_buttons[0] & 0x80 != 0 && !frame.left_held;
    frame.wheel = raw.wheel;
    frame.shift = raw.held(0x2A) || raw.held(0x36);
    // 004035A8: DIK_ESCAPE, just pressed.
    frame.escape = raw.just_pressed(0x01);
    // DIK_UP, DOWN, LEFT, RIGHT, RETURN.
    for (dik, value) in [
        (0xC8, KEY_UP),
        (0xD0, KEY_DOWN),
        (0xCB, KEY_LEFT),
        (0xCD, KEY_RIGHT),
        (0x1C, KEY_ENTER),
    ] {
        if raw.just_pressed(dik) {
            frame.keys.push(value);
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
                    .resource::<recomp_game::tes::interface_manager::InterfaceManager>()
                    .alt_active_tile_088;
                alt.map(|tile| {
                    recomp_game::tes::tile_values::tile_numeric_trait(
                        cx,
                        tile,
                        recomp_game::tes::tile_values::TRAIT_ID,
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
    let input = if world.contains_resource::<CaptureState>() {
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
pub fn report_screen_stop(world: &mut World, error: &recomp_core::reader::StreamError) {
    let text = format!("[ERROR] the screens stopped: {error:?}");
    println!("{text}");
    world
        .resource_mut::<WinMainProgress>()
        .stops
        .push(recomp_startup::winmain::HostStopReport {
            block: recomp_startup::winmain::Block::Finished,
            text,
        });
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
    preview: &recomp_game::tes::interface_scene::SavePreview,
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
    world: &mut World,
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
    if let Some(handle) = world
        .resource::<ScreenCache>()
        .materials
        .get(&(key.clone(), wrap))
    {
        return (handle.clone(), key == TextureKey::None);
    }
    let image = match texture {
        QuadTexture::None => None,
        QuadTexture::File(path) => {
            let files = driver_files(world);
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
                    let mut cache = world.resource_mut::<ScreenCache>();
                    if cache.failed.insert(key.clone()) {
                        println!("[WARNING] screens: can't draw texture {text}");
                    }
                    None
                }
            }
        }
        QuadTexture::Font(entity) => world.get::<PixelData>(*entity).map(|pixels| {
            make_image(
                pixels.widths_54[0],
                pixels.heights_58[0],
                vec![pixels.level(0, 0).to_vec()],
                wrap,
            )
        }),
        QuadTexture::Preview(preview) => preview_image(preview, wrap),
    };
    let image = image.map(|image| world.resource_mut::<Assets<Image>>().add(image));
    let material = GammaMaterial { texture: image };
    let plain = material.texture.is_none();
    let handle = world.resource_mut::<Assets<GammaMaterial>>().add(material);
    world
        .resource_mut::<ScreenCache>()
        .materials
        .insert((key, wrap), handle.clone());
    (handle, plain)
}

/// IDA: 0057F170's render, on a request or a menu's frame: the list, then one mesh per run of a texture, in order, each a step nearer the camera. g_dx9Renderer +0x200 stays set around it, so 00410390 waits as it does in the original.
#[allow(clippy::too_many_lines)]
fn draw_screens(world: &mut World) {
    let requests = take_render_requests(world);
    let menu_running = world
        .get_resource::<ScreenDirector>()
        .is_some_and(|director| director.stage != ScreenStage::Loading);
    // 00579260 gates every tile draw on !IsRunning, so before the menu the tiles change only on a request. While a movie thread runs, its picture alone is up. Otherwise the last list stays, as a presented frame does.
    let shot = world.get_resource::<NewGameShot>().copied();
    if matches!(shot, Some(NewGameShot::NoScreens)) {
        let old = std::mem::take(&mut world.resource_mut::<ScreenCache>().entities);
        for entity in old {
            world.despawn(entity);
        }
        return;
    }
    let draw_tiles = !requests.is_empty() || menu_running || shot.is_some();
    if !requests.is_empty() && !menu_running && world.resource::<ScreenCache>().last_list.is_none()
    {
        println!("[INFO] LoadingMenu: first frame drawn from a request");
    }
    // Nothing to draw before 0040EB18, or when the chain stopped short of it.
    if !world.contains_resource::<TileEngine>()
        || !world.contains_resource::<recomp_game::tes::interface_manager::InterfaceManager>()
    {
        return;
    }
    let flag = in_scene_flag(world);
    if let Some(flag) = &flag {
        set_in_scene(flag, true);
    }
    let Some((quads, view, movie_running)) = with_driver_interface(world, |cx| {
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
    if world.contains_resource::<CaptureState>() {
        let cursor = world
            .get_resource::<recomp_game::tes::interface_manager::InterfaceManager>()
            .and_then(|manager| manager.cursor_01c);
        quads.retain(|quad| Some(quad.tile) != cursor);
    }
    if let Some(NewGameShot::OnlyMenu(menu)) = shot {
        let engine = world.resource::<TileEngine>();
        quads.retain(|quad| engine.containing_menu(quad.tile) == Some(menu));
    }
    if !draw_tiles && !movie_running {
        if let Some(flag) = &flag {
            set_in_scene(flag, false);
        }
        return;
    }
    // The camera's scale: renderer pixels over the window, whatever its real size.
    let mut cameras = world.query_filtered::<&mut Projection, With<ScreenCamera>>();
    for mut projection in cameras.iter_mut(world) {
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
    let flush = |world: &mut World,
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
        let (material, _) = material_for(world, texture, *wrap);
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
                world,
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
        world,
        &mut batches,
        &current,
        &mut positions,
        &mut uvs,
        &mut colors,
        &mut indices,
    );
    // Under everything, the movie's picture: 00410810 runs before 0070E0A0, at its own place and size. The window clips what runs past it.
    let movie = world
        .get_resource::<LatestMovieFrame>()
        .cloned()
        .filter(|_| shot.is_none());
    let mut layers: Vec<(Handle<GammaMaterial>, Mesh)> = Vec::new();
    if let Some(movie) = movie.filter(|movie| movie.visible) {
        let existing = world
            .resource::<ScreenCache>()
            .materials
            .get(&(TextureKey::Movie, false))
            .cloned();
        let material = existing.unwrap_or_else(|| {
            let handle = world
                .resource_mut::<Assets<GammaMaterial>>()
                .add(GammaMaterial {
                    texture: Some(movie.image.clone()),
                });
            world
                .resource_mut::<ScreenCache>()
                .materials
                .insert((TextureKey::Movie, false), handle.clone());
            handle
        });
        // A new movie's size makes a new image.
        {
            let mut materials = world.resource_mut::<Assets<GammaMaterial>>();
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
    let old = std::mem::take(&mut world.resource_mut::<ScreenCache>().entities);
    let mut entities = Vec::with_capacity(layers.len());
    for (index, (material, mesh)) in layers.into_iter().enumerate() {
        let handle = world.resource_mut::<Assets<Mesh>>().add(mesh);
        let z = index as f32 * 0.01;
        let entity = old.get(index).copied();
        let entity = if let Some(entity) = entity {
            world.entity_mut(entity).insert((
                Mesh2d(handle),
                MeshMaterial2d(material),
                Transform::from_xyz(0.0, 0.0, z),
            ));
            entity
        } else {
            world
                .spawn((
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
        world.despawn(entity);
    }
    let mut cache = world.resource_mut::<ScreenCache>();
    cache.entities = entities;
    if draw_tiles {
        cache.last_list = Some(quads.len());
    }
    if let Some(flag) = &flag {
        set_in_scene(flag, false);
    }
}

/// IDA: N/A a capture: the script's end state held long enough to draw, a screenshot, and the exit.
fn run_capture(world: &mut World) {
    if !world.contains_resource::<CaptureState>() {
        return;
    }
    // A capture that can't finish reports where it stood and exits, rather than running on.
    let deadline = {
        let mut state = world.resource_mut::<CaptureState>();
        state.ticks += 1;
        state.ticks == CAPTURE_FRAME_LIMIT
    };
    if deadline {
        let state = world.resource::<CaptureState>();
        println!(
            "[ERROR] capture gave up: step {}, settled {}, movie {:?}, director {:?}, media {:?}",
            state.step,
            state.frames_settled,
            world.get_resource::<LatestMovieFrame>().map(|movie| (
                movie.visible,
                movie.generation,
                movie.presented
            )),
            world.get_resource::<ScreenDirector>(),
            world.get_resource::<recomp_startup::winmain_menu_media::MainMenuMedia>(),
        );
        world.write_message(AppExit::from_code(3));
        return;
    }
    let (script, step, taken, path) = {
        let state = world.resource::<CaptureState>();
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
        CaptureScript::NewGame(view) => new_game_capture_ready(world, view),
        CaptureScript::Intro => {
            let built = world.resource::<WinMainProgress>().next
                > recomp_startup::winmain::Block::Interface;
            let running = with_driver_interface(world, |cx| cx.host.movie_player_is_running())
                .unwrap_or(false);
            built && running
        }
        CaptureScript::Loading(tick) => {
            // The C++ `--loading-capture`: section 3 at 100, then this many 101 ms steps of 005AD980's state.
            let held = world.resource::<WinMainProgress>().next
                > recomp_startup::winmain::Block::Interface;
            if !held {
                return;
            }
            let running = with_driver_interface(world, |cx| cx.host.movie_player_is_running())
                .unwrap_or(false);
            if running {
                return;
            }
            let frames = {
                let mut state = world.resource_mut::<CaptureState>();
                state.frames_settled += 1;
                state.frames_settled
            };
            if frames == 1 {
                let _ = with_driver_interface(world, |cx| {
                    loading_capture_steps(cx, tick)?;
                    interface_render_if_no_movie(cx, 0)
                });
            }
            frames >= 30
        }
        _ => {
            let menu_visible = world
                .get_resource::<ScreenDirector>()
                .is_some_and(|director| director.stage == ScreenStage::Menu)
                && settled(world);
            if menu_visible {
                world.resource_mut::<CaptureState>().frames_settled += 1;
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
            let movie_ready = hold_menu_movie_frame(world);
            step >= needed && movie_ready && world.resource::<CaptureState>().frames_settled >= 30
        }
    };
    if !ready {
        return;
    }
    world.resource_mut::<CaptureState>().taken = true;
    println!(
        "capturing {} movie frame {:?}, picture {:?}",
        path.display(),
        menu_movie_frame(world),
        world.get_resource::<LatestMovieFrame>().map(|movie| (
            movie.visible,
            movie.generation,
            movie.presented,
            movie.size,
            movie.screen_position,
            movie.screen_size
        ))
    );
    world
        .spawn(Screenshot::primary_window())
        .observe(save_to_disk(path))
        .observe(
            |_: On<ScreenshotCaptured>, mut exits: MessageWriter<AppExit>| {
                exits.write(AppExit::Success);
            },
        );
}

// IDA: N/A a New Game capture's steps: New Game (00589420's button, `main_menu_new_game`) once the menu has settled 30
// updates, the scripts' run to their stop, then the view, held 30 updates so its textures reach the screen.
fn new_game_capture_ready(world: &mut World, view: NewGameView) -> bool {
    use recomp_startup::winmain_menu_media::MainMenuMedia;
    let step = world.resource::<CaptureState>().step;
    match step {
        0 => {
            let menu_visible = world
                .get_resource::<ScreenDirector>()
                .is_some_and(|director| director.stage == ScreenStage::Menu)
                && settled(world);
            if !menu_visible {
                return false;
            }
            let frames = {
                let mut state = world.resource_mut::<CaptureState>();
                state.frames_settled += 1;
                state.frames_settled
            };
            if frames >= 30 {
                match with_driver_interface(world, recomp_game::tes::menus::main_menu_new_game) {
                    Some(Ok(_)) => {}
                    Some(Err(error)) => report_screen_stop(world, &error),
                    None => println!("[ERROR] capture: no interface for New Game"),
                }
                let mut state = world.resource_mut::<CaptureState>();
                state.step = 1;
                state.frames_settled = 0;
                // The commands run to the stop at the scripts' pace: let the capture wait for them.
                state.ticks = 0;
            }
            false
        }
        1 => {
            if !world
                .get_resource::<MainMenuMedia>()
                .is_some_and(|media| media.scripts_stopped)
            {
                // The move's load takes far longer than a menu capture's wait.
                let mut state = world.resource_mut::<CaptureState>();
                state.ticks = state.ticks.min(CAPTURE_FRAME_LIMIT / 2);
                return false;
            }
            match view {
                NewGameView::Menu(id) => {
                    let menu = with_driver_interface(world, |cx| {
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
                            let _ = with_driver_interface(world, |cx| {
                                let tile = cx.engine.menu(menu).tile_04?;
                                Some((|| {
                                    recomp_game::tes::tile_values::tile_set_numeric_trait(
                                        cx,
                                        tile,
                                        recomp_game::tes::tile_values::TRAIT_VISIBLE,
                                        2.0,
                                    )?;
                                    recomp_game::tes::tile_values::tile_update_scene_graph(
                                        cx, tile, false,
                                    )?;
                                    recomp_game::tes::interface_frame::interface_apply_fade(
                                        cx, tile, 1.0,
                                    )
                                })())
                            });
                            world.insert_resource(NewGameShot::OnlyMenu(menu));
                            // Over the world, as an opened menu draws: the cell view's camera clears and draws first,
                            // the interface's camera draws on it without a clear.
                            if let Some(report) = crate::world_view::spawn_cell_view(world, false) {
                                println!("[INFO] world view: {report:?}");
                                let mut cameras =
                                    world.query_filtered::<&mut Camera, With<ScreenCamera>>();
                                for mut camera in cameras.iter_mut(world) {
                                    camera.clear_color = ClearColorConfig::None;
                                }
                            }
                        }
                        None => println!("[ERROR] capture: no menu {id} at the stop"),
                    }
                }
                NewGameView::Prison | NewGameView::PrisonOverview => {
                    world.insert_resource(NewGameShot::NoScreens);
                    let mut cameras = world.query_filtered::<&mut Camera, With<ScreenCamera>>();
                    for mut camera in cameras.iter_mut(world) {
                        camera.is_active = false;
                    }
                    match crate::world_view::spawn_cell_view(
                        world,
                        view == NewGameView::PrisonOverview,
                    ) {
                        Some(report) => {
                            println!("[INFO] world view: {report:?}");
                            world.insert_resource(report);
                        }
                        None => println!("[ERROR] capture: no player cell at the stop"),
                    }
                }
            }
            let mut state = world.resource_mut::<CaptureState>();
            state.step = 2;
            false
        }
        _ => {
            let mut state = world.resource_mut::<CaptureState>();
            state.frames_settled += 1;
            state.frames_settled >= 30
        }
    }
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
    cx: &mut recomp_game::tes::tile::InterfaceCx,
    tick: i32,
) -> Result<(), recomp_core::reader::StreamError> {
    use recomp_game::tes::tile::MenuClass;
    use recomp_game::tes::tile_values::{TRAIT_USER3, tile_set_numeric_trait};
    recomp_game::tes::loading_menu::loading_menu_set_section_progress(cx, 3, 100.0)?;
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
        .get_resource::<recomp_game::tes::interface_manager::InterfaceManager>()
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
    use recomp_game::tes::interface_scene::SavePreview;

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
}
