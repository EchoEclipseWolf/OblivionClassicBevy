//! IDA: N/A the host's end of the movie thread's device calls: each `MovieQueueEntry` arrives on the main thread, and what the original would have put on the back buffer becomes one `Image`. The screens draw it. It doesn't draw anything itself, and no wgpu call stands behind a device call: this is the recorded substitution for the D3D9 device of 0040F6A0..0040FC8D.
//!
//! The picture is built the way 0040FA20 draws: every tile at the place its quad says, at the texture's own size. So the image has the tile grid's size, not the movie's: `Map loop.bik` at 1280x720 has a third row of 256-high tiles with 208 rows of picture (the rest is the zeros of 0040F724) and comes out 1280x768, and at the renderer's 1.5 that row runs past the window's bottom, as it does in the original. The screens must clip to the window, and draw over a black clear (`MOVIE_CLEAR_COLOR`).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use recomp_game::tes::movie_player::{
    DecodedSplash, MovieDeviceCall, MovieQueueEntry, MovieTexture,
};
use recomp_game::tes::os_globals::lock_shared;
use recomp_startup::winmain_host::InterfaceRenderRequests;

/// IDA: N/A the latest picture the movie code drew, for the screens: the Bevy `Image`, its size and where on the renderer's `iSize W` x `iSize H` screen it goes. `visible` is false until a movie has drawn, and again after the game's next Present once it has closed, as a Present replaces what the screen shows: nothing should be drawn then.
#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub struct LatestMovieFrame {
    /// `Bgra8Unorm`, 255 alpha. Replaced in place, so the handle stays the same.
    pub image: Handle<Image>,
    pub visible: bool,
    /// The image's size in texels.
    pub size: UVec2,
    /// The first quad's top-left in the renderer's pixels: (+0x18, +0x1C).
    pub screen_position: Vec2,
    /// `size` scaled by the movie's +0x14, in the renderer's pixels.
    pub screen_size: Vec2,
    /// Counts each new picture, so a screen or a test can see it change.
    pub generation: u64,
    /// IDA: N/A Present has run: the movie thread's own frames, not a menu-owned one's.
    pub presented: u64,
}

/// IDA: N/A the built picture before it's in `Assets<Image>`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ComposedMovieFrame {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
    pub screen_position: [f32; 2],
    pub screen_size: [f32; 2],
    pub generation: u64,
}

#[derive(Default)]
struct DrawnQuad {
    texture: u32,
    x: f32,
    y: f32,
    right: f32,
    bottom: f32,
}

/// IDA: N/A the textures the thread has made and the scene it's drawing, keyed by the host value `MovieTexture`. Plain data, so a test feeds it entries with no `App`. Completion: 100% of the calls 0040F6A0..0040FC8D and 00410840 make.
#[derive(Default)]
pub struct MovieFrameCompositor {
    textures: HashMap<u32, (u32, u32, Vec<u8>)>,
    bound: Option<u32>,
    quads: Vec<DrawnQuad>,
    /// A tile's bytes changed since the last picture.
    dirty: bool,
    pub frame: Option<ComposedMovieFrame>,
    pub presents: u64,
    pub calls: u64,
    /// `QueuedMovieDevice::texture`'s counter: this counts the same creations.
    created: u32,
    generation: u64,
    /// The thread's own BeginScene is open, so its EndScene will compose.
    scene_open: bool,
    /// Tiles drawn with no scene of the movie's own: 00410757's arm inside 0057F170.
    drawn_in_scene: bool,
    /// `frame` as its last texture went: still what's on the screen, until the next Present.
    closed: Option<ComposedMovieFrame>,
    /// The game's Presents as of the last publish.
    game_presents: u64,
}

impl MovieFrameCompositor {
    /// IDA: N/A one call, in the thread's order.
    pub fn apply(&mut self, entry: MovieQueueEntry) {
        self.calls += 1;
        let MovieQueueEntry {
            entry,
            tile_bytes,
            splash,
        } = entry;
        match entry.call {
            MovieDeviceCall::CreateTexture { width, height, .. } => {
                // `QueuedMovieDevice` numbers textures in order from 1.
                let id = self.next_texture_id();
                self.textures
                    .insert(id, (width, height, vec![0; (width * height * 4) as usize]));
            }
            MovieDeviceCall::CreateTextureFromFile { .. } => {
                if let (Some(splash), Some(bytes)) = (splash, tile_bytes) {
                    self.created = splash.texture.0;
                    self.textures
                        .insert(splash.texture.0, (splash.width, splash.height, bytes));
                    self.dirty = true;
                }
            }
            MovieDeviceCall::UnlockRect { texture } => {
                if let (Some(tile), Some(bytes)) = (self.textures.get_mut(&texture.0), tile_bytes)
                    && tile.2.len() == bytes.len()
                {
                    tile.2 = bytes;
                    self.dirty = true;
                }
            }
            MovieDeviceCall::ReleaseTexture { texture } => {
                self.textures.remove(&texture.0);
                if self.textures.is_empty() {
                    self.dirty = false;
                    // 00410115's Release takes nothing off the screen: 004107C6's Present put it there, and the next Present replaces it, the next movie's or the game's.
                    if let Some(frame) = self.frame.take() {
                        self.closed = Some(frame);
                    }
                }
            }
            MovieDeviceCall::SetTexture { texture, .. } => {
                self.bound = texture.map(|MovieTexture(id)| id);
            }
            MovieDeviceCall::BeginScene => {
                self.scene_open = true;
                self.quads.clear();
            }
            MovieDeviceCall::Clear { .. } => self.quads.clear(),
            MovieDeviceCall::DrawPrimitiveUp { vertices, .. } => {
                if let Some(texture) = self.bound {
                    self.drawn_in_scene = true;
                    self.quads.push(DrawnQuad {
                        texture,
                        x: vertices[0].x,
                        y: vertices[0].y,
                        right: vertices[3].x,
                        bottom: vertices[3].y,
                    });
                }
            }
            MovieDeviceCall::EndScene => {
                self.scene_open = false;
                self.drawn_in_scene = false;
                self.compose();
            }
            MovieDeviceCall::Present => {
                self.compose();
                self.presents += 1;
            }
            _ => {}
        }
    }

    // IDA: N/A the next number `QueuedMovieDevice::texture` hands out: it counts every texture ever made, including the released ones.
    /// IDA: N/A 004106C0 skips Begin and EndScene inside 0057F170's scene, so a menu-owned movie's tiles have no EndScene to compose them: the picture is ready when the frame's calls have been applied.
    pub fn compose_scene_drawn(&mut self) {
        if std::mem::take(&mut self.drawn_in_scene) {
            self.compose();
        }
    }

    // IDA: N/A a helper, with no address of its own.
    fn next_texture_id(&mut self) -> u32 {
        self.created += 1;
        self.created
    }

    // IDA: N/A the scene's quads into one image, unless nothing changed.
    fn compose(&mut self) {
        if !self.dirty || self.quads.is_empty() {
            return;
        }
        let origin = (self.quads[0].x, self.quads[0].y);
        let Some(first) = self.textures.get(&self.quads[0].texture) else {
            return;
        };
        // 0040F760's quad is `size * scale` across, so the scale is that over the texture's size.
        let scale = (
            (self.quads[0].right - self.quads[0].x) / first.0 as f32,
            (self.quads[0].bottom - self.quads[0].y) / first.1 as f32,
        );
        let place = |x: f32, from: f32, by: f32| ((x - from) / by).round() as u32;
        let (mut width, mut height) = (0, 0);
        for quad in &self.quads {
            let Some(tile) = self.textures.get(&quad.texture) else {
                continue;
            };
            width = width.max(place(quad.x, origin.0, scale.0) + tile.0);
            height = height.max(place(quad.y, origin.1, scale.1) + tile.1);
        }
        let mut bgra = vec![0; (width * height * 4) as usize];
        for quad in &self.quads {
            let Some(tile) = self.textures.get(&quad.texture) else {
                continue;
            };
            let (left, top) = (
                place(quad.x, origin.0, scale.0),
                place(quad.y, origin.1, scale.1),
            );
            for row in 0..tile.1 {
                let to = (((top + row) * width + left) * 4) as usize;
                let from = (row * tile.0 * 4) as usize;
                let bytes = (tile.0 * 4) as usize;
                bgra[to..to + bytes].copy_from_slice(&tile.2[from..from + bytes]);
            }
        }
        // D3DFMT_X8R8G8B8 has no alpha, and BINKSURFACE32 leaves that byte undefined.
        for pixel in bgra.as_chunks_mut::<4>().0 {
            pixel[3] = 255;
        }
        self.generation += 1;
        self.closed = None;
        let generation = self.generation;
        self.frame = Some(ComposedMovieFrame {
            width,
            height,
            bgra,
            screen_position: [origin.0, origin.1],
            screen_size: [width as f32 * scale.0, height as f32 * scale.1],
            generation,
        });
        self.dirty = false;
    }
}

/// IDA: N/A the compositor the seam's closure and the system share. Non-send data isn't needed: it's plain bytes.
#[derive(Resource, Clone, Default)]
pub struct SharedMovieFrames(pub Arc<Mutex<MovieFrameCompositor>>);

/// IDA: N/A `--dump-movie-frames`: writes a movie's first frame, and every 30th, as `.ppm` files in this folder.
#[derive(Resource, Clone, Debug)]
pub struct DumpMovieFrames {
    pub folder: PathBuf,
    pub written: u32,
}

/// IDA: N/A writes a BGRA image as a binary PPM (`P6`), which `ffmpeg` and most viewers read, and which adds no dependency.
///
/// # Errors
/// The file can't be written.
pub fn write_ppm(
    path: &std::path::Path,
    width: u32,
    height: u32,
    bgra: &[u8],
) -> std::io::Result<()> {
    let mut bytes = format!("P6\n{width} {height}\n255\n").into_bytes();
    bytes.reserve(bgra.len());
    for pixel in bgra.as_chunks::<4>().0 {
        bytes.extend_from_slice(&[pixel[2], pixel[1], pixel[0]]);
    }
    std::fs::write(path, bytes)
}

/// IDA: N/A the recorded substitution for D3DXCreateTextureFromFileA at 004108CD: the DDS's own header for the size 00410933 reads back, and FFmpeg for the pixels. `None` for anything else, as a failed load. Completion: 100% of the host role; DDS formats FFmpeg can't decode fail the load.
#[must_use]
pub fn load_splash_with_ffmpeg(path: &std::path::Path) -> Option<DecodedSplash> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut header = [0u8; 0x14];
    std::io::Read::read_exact(&mut file, &mut header).ok()?;
    if &header[..4] != b"DDS " {
        return None;
    }
    let word = |offset: usize| {
        u32::from_le_bytes([
            header[offset],
            header[offset + 1],
            header[offset + 2],
            header[offset + 3],
        ])
    };
    let (height, width) = (word(0x0C), word(0x10));
    let output = std::process::Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-i"])
        .arg(path)
        .args([
            "-frames:v",
            "1",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "bgra",
            "pipe:1",
        ])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    (output.status.success() && output.stdout.len() == (width * height * 4) as usize).then_some(
        DecodedSplash {
            width,
            height,
            bgra: output.stdout,
        },
    )
}

/// IDA: N/A the game's Presents, for a closed movie's picture: `recomp_startup::winmain::render_requests`.
#[derive(Resource, Clone, Default)]
pub struct GamePresents(pub InterfaceRenderRequests);

/// IDA: N/A the latest picture into `Assets<Image>`, each frame. Completion: 100% of the host role. A request counted before a release in the same frame ends that picture at once, since the frame's order isn't kept; a game render can't run while a movie other than the menu's plays (00579282).
pub fn publish_movie_frame(
    shared: Res<SharedMovieFrames>,
    mut latest: ResMut<LatestMovieFrame>,
    images: Option<ResMut<Assets<Image>>>,
    dump: Option<ResMut<DumpMovieFrames>>,
    game_presents: Option<Res<GamePresents>>,
) {
    let mut compositor = lock_shared(&shared.0);
    compositor.compose_scene_drawn();
    latest.presented = compositor.presents;
    let presents = game_presents.map_or(0, |presents| presents.0.count());
    if presents != compositor.game_presents {
        compositor.game_presents = presents;
        compositor.closed = None;
    }
    let Some(frame) = compositor.frame.as_ref().or(compositor.closed.as_ref()) else {
        latest.visible = false;
        return;
    };
    if !latest.visible || latest.generation != frame.generation {
        show_picture(frame, &mut latest, images, dump);
    }
}

// IDA: N/A one new picture into the resource and its `Image`.
fn show_picture(
    frame: &ComposedMovieFrame,
    latest: &mut LatestMovieFrame,
    images: Option<ResMut<Assets<Image>>>,
    dump: Option<ResMut<DumpMovieFrames>>,
) {
    if let Some(mut dump) = dump
        && (latest.size != UVec2::new(frame.width, frame.height)
            || !latest.visible
            || frame.generation % 30 == 1)
    {
        let path = dump.folder.join(format!(
            "frame-{:05}-{}x{}.ppm",
            dump.written, frame.width, frame.height
        ));
        if let Err(error) = write_ppm(&path, frame.width, frame.height, &frame.bgra) {
            warn!("Couldn't write {}: {error}", path.display());
        }
        dump.written += 1;
    }
    let same_size = latest.size == UVec2::new(frame.width, frame.height);
    latest.visible = true;
    latest.generation = frame.generation;
    latest.size = UVec2::new(frame.width, frame.height);
    latest.screen_position = Vec2::from(frame.screen_position);
    latest.screen_size = Vec2::from(frame.screen_size);
    let Some(mut images) = images else {
        return;
    };
    if same_size && let Some(mut image) = images.get_mut(&latest.image) {
        image.data = Some(frame.bgra.clone());
        return;
    }
    let image = Image::new(
        Extent3d {
            width: frame.width,
            height: frame.height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        frame.bgra.clone(),
        TextureFormat::Bgra8Unorm,
        RenderAssetUsages::default(),
    );
    latest.image = images.add(image);
}

#[cfg(test)]
mod tests {
    use super::*;
    use recomp_game::tes::movie_player::{
        MovieDevice, MovieDeviceEntry, MovieX87, QueuedMovieDevice, ScriptedMoviePlatform,
        create_movie_tile_set, destroy_movie_tile_set, draw_movie_tiles,
    };
    use std::sync::mpsc::channel;

    // Everything a queue holds.
    fn drain(
        receiver: &std::sync::mpsc::Receiver<MovieQueueEntry>,
        into: &mut MovieFrameCompositor,
    ) {
        while let Ok(entry) = receiver.try_recv() {
            into.apply(entry);
        }
    }

    /// The real tile code over the real queue: a 720x480 movie, its tiles at 2.25 as in P06-RUN-001. Each tile's own bytes land at its place in the grid of 768x512.
    #[test]
    fn tiles_compose_at_their_places() {
        let (sender, receiver) = channel();
        let mut device = QueuedMovieDevice::new(sender);
        let mut platform = ScriptedMoviePlatform::default();
        let mut set =
            create_movie_tile_set(&mut device, &mut platform, 720, 480, 4, 256, 22).unwrap();
        for (index, tile) in set.tiles_40.iter_mut().enumerate() {
            tile.pixels.fill(index as u8 + 1);
            device.unlock_rect(0x0040_FD32, tile.texture, &tile.pixels);
        }
        device.call(0x0041_076C, MovieDeviceCall::BeginScene);
        draw_movie_tiles(&mut device, &set, 150.0, 0.0, 2.25, 2.25, MovieX87::Bits53);
        device.call(0x0041_07AD, MovieDeviceCall::EndScene);
        device.call(0x0041_07C6, MovieDeviceCall::Present);
        let mut compositor = MovieFrameCompositor::default();
        drain(&receiver, &mut compositor);
        let frame = compositor.frame.as_ref().unwrap();
        assert_eq!((frame.width, frame.height), (768, 512));
        assert_eq!(frame.screen_position, [150.0, 0.0]);
        assert_eq!(frame.screen_size, [768.0 * 2.25, 512.0 * 2.25]);
        let at = |x: u32, y: u32| frame.bgra[((y * 768 + x) * 4) as usize];
        assert_eq!(set.tile_count_20, 6);
        assert_eq!(at(0, 0), 1);
        assert_eq!(at(256, 0), 2);
        assert_eq!(at(512, 0), 3);
        assert_eq!(at(0, 256), 4);
        assert_eq!(at(767, 511), 6);
        assert_eq!(frame.bgra[3], 255, "X8R8G8B8's byte is opaque");
        assert_eq!(compositor.presents, 1);
    }

    /// Releasing the textures ends the picture.
    #[test]
    fn a_closed_movie_has_no_picture() {
        let mut compositor = MovieFrameCompositor::default();
        let entry = |call| MovieQueueEntry {
            entry: MovieDeviceEntry { site: 0, call },
            tile_bytes: None,
            splash: None,
        };
        compositor.apply(entry(MovieDeviceCall::CreateTexture {
            width: 4,
            height: 4,
            levels: 1,
            usage: 0,
            format: 22,
            pool: 0,
        }));
        compositor.frame = Some(ComposedMovieFrame::default());
        compositor.apply(entry(MovieDeviceCall::ReleaseTexture {
            texture: MovieTexture(1),
        }));
        assert!(compositor.frame.is_none());
    }

    /// The screens' contract: a picture becomes the resource's `Image`, the same handle while the size holds, with the new bytes; a closed movie isn't `visible`.
    #[test]
    fn the_resource_holds_the_latest_image() {
        let mut app = App::new();
        app.add_plugins(AssetPlugin::default());
        app.init_asset::<Image>();
        app.init_resource::<LatestMovieFrame>();
        app.init_resource::<SharedMovieFrames>();
        app.add_systems(Update, publish_movie_frame);
        let shared = app.world().resource::<SharedMovieFrames>().clone();
        let set = |value: u8, generation: u64| {
            lock_shared(&shared.0).frame = Some(ComposedMovieFrame {
                width: 2,
                height: 1,
                bgra: vec![value; 8],
                screen_position: [1.0, 2.0],
                screen_size: [4.0, 2.0],
                generation,
            });
        };
        app.update();
        assert!(!app.world().resource::<LatestMovieFrame>().visible);
        set(7, 1);
        app.update();
        let first = app.world().resource::<LatestMovieFrame>().clone();
        assert!(first.visible);
        assert_eq!(first.size, UVec2::new(2, 1));
        assert_eq!(first.screen_position, Vec2::new(1.0, 2.0));
        assert_eq!(first.screen_size, Vec2::new(4.0, 2.0));
        let bytes = |app: &App, handle: &Handle<Image>| {
            let images = app.world().resource::<Assets<Image>>();
            let image = images.get(handle).unwrap();
            assert_eq!(image.texture_descriptor.format, TextureFormat::Bgra8Unorm);
            image.data.clone().unwrap()
        };
        assert_eq!(bytes(&app, &first.image), vec![7; 8]);
        set(9, 2);
        app.update();
        let second = app.world().resource::<LatestMovieFrame>().clone();
        assert_eq!(second.image, first.image);
        assert_eq!(second.generation, 2);
        assert_eq!(bytes(&app, &second.image), vec![9; 8]);
        lock_shared(&shared.0).frame = None;
        app.update();
        assert!(!app.world().resource::<LatestMovieFrame>().visible);
    }

    // One movie's whole life in one queue drain: tiles, one frame, Present, and the release 004102C0 runs straight after.
    fn one_frame_movie(device: &mut QueuedMovieDevice, value: u8) {
        let mut platform = ScriptedMoviePlatform::default();
        let mut set = create_movie_tile_set(device, &mut platform, 16, 16, 4, 256, 22);
        let tiles = set.as_mut().unwrap();
        for tile in &mut tiles.tiles_40 {
            tile.pixels.fill(value);
            device.unlock_rect(0x0040_FD32, tile.texture, &tile.pixels);
        }
        device.call(
            0x0041_0755,
            MovieDeviceCall::Clear {
                count: 0,
                flags: 1,
                color: 0xFF00_0000,
                z_bits: 1.0f32.to_bits(),
                stencil: 0,
            },
        );
        device.call(0x0041_076C, MovieDeviceCall::BeginScene);
        draw_movie_tiles(device, tiles, 0.0, 0.0, 1.0, 1.0, MovieX87::Bits53);
        device.call(0x0041_07AD, MovieDeviceCall::EndScene);
        device.call(0x0041_07C6, MovieDeviceCall::Present);
        destroy_movie_tile_set(device, &mut set);
    }

    /// 004107C6 Present put the picture on the screen, and 00410115's Release doesn't take it off. A one-frame movie (all four of `sIntroSequence`'s) has its Present and its Release in one drain, and the picture still reaches the resource. P06-RUN-001's order.
    #[test]
    fn a_picture_released_in_the_drain_that_presented_it_is_still_published() {
        let mut app = App::new();
        app.add_plugins(AssetPlugin::default());
        app.init_asset::<Image>();
        app.init_resource::<LatestMovieFrame>();
        app.init_resource::<SharedMovieFrames>();
        app.init_resource::<GamePresents>();
        app.add_systems(Update, publish_movie_frame);
        let shared = app.world().resource::<SharedMovieFrames>().clone();
        let (sender, receiver) = channel();
        let mut device = QueuedMovieDevice::new(sender);
        one_frame_movie(&mut device, 0x40);
        drain(&receiver, &mut lock_shared(&shared.0));
        app.update();
        let latest = app.world().resource::<LatestMovieFrame>().clone();
        assert_eq!(latest.presented, 1);
        assert_eq!(
            latest.generation, 1,
            "the picture was composed and published"
        );
        assert!(latest.visible);
        let images = app.world().resource::<Assets<Image>>();
        let data = images.get(&latest.image).unwrap().data.clone().unwrap();
        assert_eq!(&data[..3], &[0x40; 3], "its own bytes");
        // Its textures are gone, and no Present has followed: it's still what's on the screen.
        app.update();
        app.update();
        assert!(app.world().resource::<LatestMovieFrame>().visible);
        // The game's next Present replaces it.
        app.world().resource::<GamePresents>().0.push(0);
        app.update();
        assert!(!app.world().resource::<LatestMovieFrame>().visible);
        // Two movies in one drain: the screen shows the later one.
        one_frame_movie(&mut device, 0x41);
        one_frame_movie(&mut device, 0x42);
        drain(&receiver, &mut lock_shared(&shared.0));
        app.update();
        let latest = app.world().resource::<LatestMovieFrame>().clone();
        assert_eq!((latest.presented, latest.visible), (3, true));
        let images = app.world().resource::<Assets<Image>>();
        let data = images.get(&latest.image).unwrap().data.clone().unwrap();
        assert_eq!(&data[..3], &[0x42; 3]);
    }

    #[test]
    fn ppm_has_rgb_rows() {
        let folder = std::env::temp_dir().join(format!("recomp_ppm_{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("frame.ppm");
        write_ppm(&path, 1, 1, &[1, 2, 3, 255]).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"P6\n1 1\n255\n\x03\x02\x01");
        std::fs::remove_dir_all(&folder).unwrap();
    }
}
