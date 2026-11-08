//! IDA: N/A what the WinMain driver stands on: the one file finder, archive manager and palette, the one per-thread CRT, the one OS platform and thread numbering, and the host's answers to the Win32 and pathing seams. Nothing here touches wgpu, winit, an audio device or FFmpeg. `recomp_host` fills the render, movie-frame and input seams.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::ThreadId;
use std::time::Instant;

use bevy_ecs::prelude::*;
use recomp_core::crt::{MsvcRand, RandomGlobals};
use recomp_core::logging::{Level, emit};
use recomp_game::bs::archive::{ArchiveAccess, ArchiveManager};
use recomp_game::bs::file::{GameFile, OpenedFile};
use recomp_game::bs::file_finder::{
    BsFileCallbacks, FileFinder, bs_file_exists, bs_file_open_as_bs_file, bs_file_open_core,
};
use recomp_game::bs::texture_palette::BsTexturePalette;
use recomp_game::gamebryo::renderer::RendererCreation;
use recomp_game::gamebryo::search_path::bs_search_path_candidates;
use recomp_game::gamebryo::texture::{SourceTexture, SourceTextureLoadServices};
use recomp_game::gamebryo::texture_loading::{
    TextureLoadServices, TextureMemoryStatistics, create_texture_from_file,
};
use recomp_game::tes::actor_process::{ActorProcessGlobals, RandomSource};
use recomp_game::tes::form_arms::{AbsentArm, DataLoadHost};
use recomp_game::tes::form_components::TextureFileEntry;
use recomp_game::tes::menus::GameStartRequest;
use recomp_game::tes::movie_player::{
    MOVIE_STATE_MENU_OWNED, MoviePlayer, host_thread_id, movie_player_is_running,
    present_current_movie,
};
use recomp_game::tes::os_globals::{
    CreatedInterface, DataFormat, DeviceInstance, DeviceObjectInstance, DeviceProperty, DiDevCaps,
    DiJoyState, DiMouseState2, DsCaps, E_FAIL, GUID_SYS_KEYBOARD, GUID_SYS_MOUSE, Guid,
    HostInstance, HostWindow, InputDevice, OsGlobalsPlatform, S_OK, SharedOsInput,
};
use recomp_game::tes::pathing::{PathingHandle, PathingPlatform};
use recomp_game::tes::save_files::{SaveFile, rebuild_save_file_list};
use recomp_game::tes::tile::InterfaceHost;
use recomp_game::tes::water::WaterTextureServices;

use crate::winmain_window::{CreateWindowCall, Rect, WindowClass, WindowPlatform};

// ---- the one file finder, archive manager and palette ----

/// IDA: 00B33A04 g_FileFinder, 00B338E0 g_ArchiveList and 00B35300 the BSTexturePalette, each one object in the original. Every seam reaches them through `SharedFiles` a call at a time, so that a stream load and its own texture lookups can run together, as they do through the original's globals. A resource can't serve here, since a seam has no `World`.
#[derive(Debug)]
pub struct GameFiles {
    /// The original's process directory, the parent of `Data\`.
    pub root: PathBuf,
    pub callbacks: BsFileCallbacks,
    pub finder: Option<FileFinder>,
    pub archives: Option<ArchiveManager>,
    pub palette: Option<BsTexturePalette>,
}

/// IDA: N/A a handle on `GameFiles`, for the main thread's seams. The lock is the host's: the original has none, and only the main thread calls these while the IO drain stays on it.
#[derive(Clone, Debug)]
pub struct SharedFiles(Arc<Mutex<GameFiles>>);

// IDA: N/A a path's bytes as the Latin-1 text the file code takes.
fn path_text(path: &[u8]) -> String {
    path.iter().map(|&byte| char::from(byte)).collect()
}

impl SharedFiles {
    /// IDA: N/A a helper, with no address of its own.
    #[must_use]
    pub fn new(root: PathBuf) -> Self {
        Self(Arc::new(Mutex::new(GameFiles {
            root,
            callbacks: BsFileCallbacks::default(),
            finder: None,
            archives: None,
            palette: None,
        })))
    }

    /// IDA: N/A an accessor.
    pub fn lock(&self) -> MutexGuard<'_, GameFiles> {
        recomp_game::tes::os_globals::lock_shared(&self.0)
    }

    /// IDA: 00430EF0 BSFile_OpenCore(path, mode, size, mask) through the finder and the archives.
    #[must_use]
    pub fn open_core(&self, path: &str, mode: i32, size: u32, mask: u32) -> Option<OpenedFile> {
        let mut files = self.lock();
        let files = &mut *files;
        let finder = files.finder.as_ref()?;
        bs_file_open_core(finder, files.archives.as_mut(), path, mode, size, mask)
    }

    /// IDA: 00431130 BSFile_OpenAsBSFile(path, mode, size, mask).
    #[must_use]
    pub fn open_as_bs_file(
        &self,
        path: &str,
        mode: i32,
        size: u32,
        mask: u32,
    ) -> Option<Box<dyn GameFile>> {
        let mut files = self.lock();
        let files = &mut *files;
        bs_file_open_as_bs_file(
            files.finder.as_ref(),
            files.archives.as_mut(),
            path,
            mode,
            size,
            mask,
        )
        .map(OpenedFile::into_game_file)
    }

    /// IDA: j_psub_748170 BSFile_Exists(path, 0).
    #[must_use]
    pub fn exists(&self, path: &[u8]) -> bool {
        let mut files = self.lock();
        let files = &mut *files;
        bs_file_exists(
            &files.callbacks,
            &files.root,
            files.finder.as_ref(),
            files.archives.as_mut(),
            &path_text(path),
            0,
        )
    }

    /// IDA: 00431020 FileFinder_FindFile(path, 0, flags, mask): the status, and the resolved path.
    #[must_use]
    pub fn find_file(&self, path: &str, flags: u32, mask: i32) -> Option<(i32, Vec<u8>)> {
        let mut files = self.lock();
        let files = &mut *files;
        let finder = files.finder.as_ref()?;
        let found = finder.find_file(path, flags, mask, files.archives.as_mut());
        Some((found.status, found.resolved.into_bytes()))
    }

    /// IDA: 00B35300 vt+4.
    #[must_use]
    pub fn palette_find(&self, path: &[u8]) -> Option<Entity> {
        let mut files = self.lock();
        let files = &mut *files;
        files.palette.as_ref()?.find(path, files.archives.as_mut())
    }

    /// IDA: 00B35300 vt+8.
    pub fn palette_add(&self, path: &[u8], texture: Option<Entity>) {
        let mut files = self.lock();
        let files = &mut *files;
        if let Some(palette) = files.palette.as_mut() {
            palette.add(path, texture, files.archives.as_mut());
        }
    }

    /// IDA: N/A whether 00B35300 is non-null.
    #[must_use]
    pub fn has_palette(&self) -> bool {
        self.lock().palette.is_some()
    }

    /// IDA: N/A whether 00B33A04 is non-null.
    #[must_use]
    pub fn has_finder(&self) -> bool {
        self.lock().finder.is_some()
    }

    /// IDA: N/A how many archives 00B338E0 holds.
    #[must_use]
    pub fn archive_count(&self) -> usize {
        self.lock()
            .archives
            .as_ref()
            .map_or(0, |archives| archives.archives().len())
    }

    /// IDA: N/A the renderer-free half of a texture service set: the file and palette hooks.
    #[must_use]
    pub fn texture_hooks(&self) -> TextureHooks<'_> {
        let (open, exists, find, add) = (self.clone(), self.clone(), self.clone(), self.clone());
        TextureHooks {
            // j_psub_748110(path, 0, 0x8000).
            open_file: Box::new(move |path| {
                open.open_core(&path_text(path), 0, 0x8000, 0xFFFF)
                    .map(OpenedFile::into_game_file)
            }),
            file_exists: Box::new(move |path| exists.exists(path)),
            palette_find: Box::new(move |path| find.palette_find(path)),
            palette_add: Box::new(move |path, texture| add.palette_add(path, texture)),
            _lifetime: std::marker::PhantomData,
        }
    }
}

/// IDA: N/A the four hooks of `TextureLoadServices`, built together.
pub struct TextureHooks<'a> {
    pub open_file: recomp_game::gamebryo::texture_loading::OpenFileHook<'a>,
    pub file_exists: recomp_game::gamebryo::texture_loading::FileExistsHook<'a>,
    pub palette_find: recomp_game::gamebryo::texture_loading::PaletteFindHook<'a>,
    pub palette_add: recomp_game::gamebryo::texture_loading::PaletteAddHook<'a>,
    _lifetime: std::marker::PhantomData<&'a ()>,
}

impl ArchiveAccess for SharedFiles {
    /// IDA: N/A `SharedFiles`'s side of `ArchiveAccess`.
    fn with_archives(&mut self, body: &mut dyn FnMut(Option<&mut ArchiveManager>)) {
        let mut files = self.lock();
        body(files.archives.as_mut());
    }
}

/// IDA: N/A `TextureLoadServices` over the real files and the renderer, a call at a time, as 00701E00, 007B8200 and 00701C00 read the globals. The gates are the values the renderer and WinMain stored: 00B256CC, 00B3F958 (`drop_static_app_data`) and 00B3F709 (`direct_to_renderer_hint`).
pub fn texture_services<'a>(
    files: &'a SharedFiles,
    creation: &'a mut RendererCreation,
    drop_static_app_data: bool,
    direct_to_renderer_hint: bool,
    statistics: &'a mut TextureMemoryStatistics,
) -> TextureLoadServices<'a> {
    let g = &creation.globals;
    let (create_on_create, mip_limits) = (
        g.texture_create_renderer_data,
        (g.mip_skip as u32, g.mip_minimum as u32),
    );
    let hooks = files.texture_hooks();
    TextureLoadServices {
        open_file: Some(hooks.open_file),
        file_exists: Some(hooks.file_exists),
        palette_find: Some(hooks.palette_find),
        palette_add: Some(hooks.palette_add),
        // 00B42E8C 00405150(text, null).
        warning: Some(Box::new(|text| {
            emit(Level::Warning, &path_text(text));
        })),
        renderer: Some(creation),
        mip_limits: Some(mip_limits),
        create_renderer_data_on_create: create_on_create,
        drop_static_app_data,
        direct_to_renderer_hint,
        statistics: Some(statistics),
    }
}

/// IDA: N/A 007023E0's services on the real palette and archives, as the C++ `make_source_texture_services` gives them, a call at a time.
pub struct DriverSourceTextures {
    pub files: SharedFiles,
}

impl SourceTextureLoadServices for DriverSourceTextures {
    /// IDA: N/A `DriverSourceTextures`'s side of `SourceTextureLoadServices`.
    fn search_candidates(&mut self, file_path: &[u8]) -> Vec<Vec<u8>> {
        bs_search_path_candidates(&path_text(file_path))
            .into_iter()
            .map(|path| path.chars().map(|c| c as u8).collect())
            .collect()
    }
    /// IDA: N/A `DriverSourceTextures`'s side of `SourceTextureLoadServices`.
    fn find_texture(&mut self, path: &[u8]) -> Option<Entity> {
        self.files.palette_find(path)
    }
    /// IDA: N/A `DriverSourceTextures`'s side of `SourceTextureLoadServices`.
    fn register_texture(&mut self, path: &[u8], texture: Entity) {
        self.files.palette_add(path, Some(texture));
    }
    /// IDA: N/A `DriverSourceTextures`'s side of `SourceTextureLoadServices`.
    fn file_exists(&mut self, path: &[u8]) -> bool {
        self.files.exists(path)
    }
}

// ---- the per-thread CRT ----

/// IDA: N/A the clock the driver reads: GetTickCount and `_time64`. `--seed` fixes both, for determinism in tests, at the plan's fixture values.
#[derive(Clone, Copy, Debug)]
pub struct HostClock {
    start: Instant,
    fixed_tick: Option<u32>,
    fixed_time64: Option<i64>,
}

/// IDA: N/A the plan's fixed GetTickCount for a seeded run ("The oracle"): 0x1000.
pub const SEEDED_TICK: u32 = 0x1000;

impl HostClock {
    /// IDA: N/A a helper, with no address of its own.
    #[must_use]
    pub fn new(seed: Option<u32>) -> Self {
        Self {
            start: Instant::now(),
            fixed_tick: seed.map(|_| SEEDED_TICK),
            fixed_time64: seed.map(i64::from),
        }
    }

    /// IDA: GetTickCount.
    #[must_use]
    pub fn tick(&self) -> u32 {
        self.fixed_tick
            .unwrap_or_else(|| self.start.elapsed().as_millis() as u32)
    }

    /// IDA: _time64(0).
    #[must_use]
    pub fn time64(&self) -> i64 {
        self.fixed_time64.unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_secs() as i64)
        })
    }
}

/// IDA: the main thread's MSVC CRT per-thread data `_holdrand` (+0x14 of `_tiddata`) and 00B069C3, 0047DF80's first-use byte. `DataLoadHost::random_next`, the tile engine's 0058C47E, `ActorProcessGlobals.random` and `_rand` itself (the stream link's) are this one CRT in the original, so they're one here. The IO thread's CRT stays separate: its holdrand starts at 1 (98BFE5).
#[derive(Clone, Debug)]
pub struct MainThreadCrt(Arc<Mutex<CrtState>>);

#[derive(Debug)]
struct CrtState {
    rand: MsvcRand,
    first_use_b069c3: RandomGlobals,
    clock: HostClock,
    draws: u64,
}

impl MainThreadCrt {
    /// IDA: N/A a helper, with no address of its own.
    #[must_use]
    pub fn new(clock: HostClock) -> Self {
        Self(Arc::new(Mutex::new(CrtState {
            rand: MsvcRand::default(),
            first_use_b069c3: RandomGlobals::default(),
            clock,
            draws: 0,
        })))
    }

    // IDA: N/A an accessor.
    fn state(&self) -> MutexGuard<'_, CrtState> {
        recomp_game::tes::os_globals::lock_shared(&self.0)
    }

    /// IDA: 0047DF80 Random_Next(0): while 00B069C3 is set, srand(_time64(0)) and 00B069C3 = 0; then rand().
    #[must_use]
    pub fn random_next(&self) -> i32 {
        let mut state = self.state();
        let state = &mut *state;
        state.draws += 1;
        let mut source = CrtDraw {
            rand: &mut state.rand,
            clock: state.clock,
        };
        recomp_game::tes::actor_process::random_next(0, &mut state.first_use_b069c3, &mut source)
    }

    /// IDA: `_rand` itself, with no first-use test: the stream link's draw at 0074FF37 on the main thread.
    #[must_use]
    pub fn rand(&self) -> i32 {
        let mut state = self.state();
        state.draws += 1;
        state.rand.rand()
    }

    /// IDA: N/A how many draws this stream has given.
    #[must_use]
    pub fn draws(&self) -> u64 {
        self.state().draws
    }

    /// IDA: N/A the stream's state now, for a report or a test.
    #[must_use]
    pub fn holdrand(&self) -> u32 {
        self.state().rand.holdrand()
    }

    /// IDA: N/A 0047DF80 as the World's paths reach it (the player's 0067B3C4 draw and the 00675310 pass): 00B069C3 is one byte, whose ECS place is `ActorProcessGlobals.random`. It's copied in before the World path and back after, so that the seams with no World, which hold it between, see the same byte.
    pub fn with_world_flag<R>(
        &self,
        world: &mut World,
        body: impl FnOnce(&mut World, &mut Self) -> R,
    ) -> R {
        let flag = self.state().first_use_b069c3;
        world.resource_mut::<ActorProcessGlobals>().random = flag;
        let mut source = self.clone();
        let result = body(world, &mut source);
        self.state().first_use_b069c3 = world.resource::<ActorProcessGlobals>().random;
        result
    }
}

struct CrtDraw<'a> {
    rand: &'a mut MsvcRand,
    clock: HostClock,
}

impl RandomSource for CrtDraw<'_> {
    /// IDA: N/A `CrtDraw`'s side of `RandomSource`.
    fn time64(&mut self) -> i64 {
        self.clock.time64()
    }
    /// IDA: N/A `CrtDraw`'s side of `RandomSource`.
    fn srand(&mut self, seed: u32) {
        self.rand.srand(seed);
    }
    /// IDA: N/A `CrtDraw`'s side of `RandomSource`.
    fn rand(&mut self) -> i32 {
        self.rand.rand()
    }
}

impl RandomSource for MainThreadCrt {
    /// IDA: N/A `MainThreadCrt`'s side of `RandomSource`.
    fn time64(&mut self) -> i64 {
        self.state().clock.time64()
    }
    /// IDA: N/A `MainThreadCrt`'s side of `RandomSource`.
    fn srand(&mut self, seed: u32) {
        self.state().rand.srand(seed);
    }
    /// IDA: N/A `MainThreadCrt`'s side of `RandomSource`.
    fn rand(&mut self) -> i32 {
        let mut state = self.state();
        state.draws += 1;
        state.rand.rand()
    }
}

// ---- the one OS platform ----

/// IDA: N/A the input the host feeds: what `GetDeviceState` answers. Filled by `recomp_host`'s input system; nothing else writes it. The mouse's axes add up until a read takes them, as DirectInput's immediate state does; its buttons stay as they are.
#[derive(Clone, Debug)]
pub struct HostInputFrame {
    pub keyboard: [u8; 256],
    pub mouse: DiMouseState2,
    /// The window has the focus. A foreground device fails Acquire without it, which is 004046DE's zeroing branch.
    pub focused: bool,
}

impl Default for HostInputFrame {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            keyboard: [0; 256],
            mouse: DiMouseState2::default(),
            focused: true,
        }
    }
}

/// IDA: N/A the running host's `OsGlobalsPlatform`, shared by the main and movie threads: one thread numbering (`host_thread_id`), and no Win32, COM, DirectInput or DirectSound call. A device exists as a host value; a call that can fail answers as P04-RUN-001 recorded. Completion: 100% of the host role. Host substitutions: no joystick is ever enumerated; `GetActiveWindow` answers the main window.
#[derive(Debug)]
pub struct HostOsPlatform {
    pub active_window: HostWindow,
    pub input: Arc<Mutex<HostInputFrame>>,
    /// Sizes whose FormHeap request answers null, for the failure-path test.
    pub failed_allocations: Vec<u32>,
    pub capabilities: DiDevCaps,
    /// SetCooperativeLevel's value per device: keyboard, then mouse.
    pub cooperative_levels: [u32; 2],
}

impl Default for HostOsPlatform {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            active_window: HostWindow::default(),
            input: Arc::new(Mutex::new(HostInputFrame::default())),
            failed_allocations: Vec::new(),
            capabilities: DiDevCaps {
                size: recomp_game::tes::os_globals::DIDEVCAPS_SIZE,
                ..DiDevCaps::default()
            },
            cooperative_levels: [0; 2],
        }
    }
}

/// IDA: N/A DIERR_OTHERAPPHASPRIO: what Acquire answers a foreground device without the focus.
pub const DIERR_OTHERAPPHASPRIO: u32 = 0x8007_0005;

/// IDA: N/A the keyboard and mouse as host values.
const HOST_KEYBOARD: InputDevice = InputDevice(1);
const HOST_MOUSE: InputDevice = InputDevice(2);

impl OsGlobalsPlatform for HostOsPlatform {
    /// IDA: N/A `HostOsPlatform`'s side of `OsGlobalsPlatform`.
    fn allocate(&mut self, _site: u32, bytes: u32) -> bool {
        !self.failed_allocations.contains(&bytes)
    }
    /// IDA: N/A `HostOsPlatform`'s side of `OsGlobalsPlatform`.
    fn get_current_thread_id(&mut self, _site: u32) -> u32 {
        host_thread_id()
    }
    /// IDA: N/A `HostOsPlatform`'s side of `OsGlobalsPlatform`.
    fn get_current_thread(&mut self, _site: u32) -> u32 {
        // GetCurrentThread's pseudo-handle.
        0xFFFF_FFFE
    }
    /// IDA: N/A no address of its own; its comments cite 00404A62.
    fn duplicate_handle(
        &mut self,
        _site: u32,
        _source_process: u32,
        _source_handle: u32,
        _target_process: u32,
        _access: u32,
        _inherit: u32,
        _options: u32,
    ) -> (u32, u32) {
        // 00404A62's null process handles make the call fail; +0x14 stays 0 (P06-RUN-001 mainHandle=0).
        (0, 0)
    }
    /// IDA: N/A `HostOsPlatform`'s side of `OsGlobalsPlatform`.
    fn name_thread(&mut self, _site: u32, _thread_id: u32, _name: &'static str) {}
    /// IDA: N/A `HostOsPlatform`'s side of `OsGlobalsPlatform`.
    fn get_active_window(&mut self, _site: u32) -> HostWindow {
        self.active_window
    }
    /// IDA: N/A `HostOsPlatform`'s side of `OsGlobalsPlatform`.
    fn direct_input8_create(
        &mut self,
        _site: u32,
        _instance: HostInstance,
        _version: u32,
    ) -> CreatedInterface<()> {
        CreatedInterface {
            result: S_OK,
            object: Some(()),
        }
    }
    /// IDA: N/A `HostOsPlatform`'s side of `OsGlobalsPlatform`.
    fn enum_devices(&mut self, _site: u32, _dev_type: u32, _flags: u32) -> Vec<DeviceInstance> {
        Vec::new()
    }
    /// IDA: N/A `HostOsPlatform`'s side of `OsGlobalsPlatform`.
    fn create_device(&mut self, _site: u32, guid: Guid) -> CreatedInterface<InputDevice> {
        let device = if guid == GUID_SYS_KEYBOARD {
            Some(HOST_KEYBOARD)
        } else if guid == GUID_SYS_MOUSE {
            Some(HOST_MOUSE)
        } else {
            None
        };
        CreatedInterface {
            result: if device.is_some() { S_OK } else { E_FAIL },
            object: device,
        }
    }
    /// IDA: N/A `HostOsPlatform`'s side of `OsGlobalsPlatform`.
    fn set_cooperative_level(
        &mut self,
        _site: u32,
        device: InputDevice,
        _window: HostWindow,
        level: u32,
    ) -> u32 {
        if device == HOST_KEYBOARD {
            self.cooperative_levels[0] = level;
        } else if device == HOST_MOUSE {
            self.cooperative_levels[1] = level;
        }
        S_OK
    }
    /// IDA: N/A `HostOsPlatform`'s side of `OsGlobalsPlatform`.
    fn set_data_format(&mut self, _site: u32, _device: InputDevice, _format: DataFormat) -> u32 {
        S_OK
    }
    /// IDA: N/A `HostOsPlatform`'s side of `OsGlobalsPlatform`.
    fn set_property(&mut self, _site: u32, _device: InputDevice, _property: DeviceProperty) -> u32 {
        S_OK
    }
    /// IDA: N/A `HostOsPlatform`'s side of `OsGlobalsPlatform`.
    fn get_capabilities(&mut self, _site: u32, _device: InputDevice, caps: &mut DiDevCaps) -> u32 {
        *caps = self.capabilities;
        S_OK
    }
    /// IDA: N/A `HostOsPlatform`'s side of `OsGlobalsPlatform`.
    fn enum_objects(
        &mut self,
        _site: u32,
        _device: InputDevice,
        _flags: u32,
    ) -> Vec<DeviceObjectInstance> {
        Vec::new()
    }
    /// IDA: N/A `HostOsPlatform`'s side of `OsGlobalsPlatform`.
    fn acquire(&mut self, _site: u32, device: InputDevice) -> u32 {
        let level = if device == HOST_KEYBOARD {
            self.cooperative_levels[0]
        } else {
            self.cooperative_levels[1]
        };
        let focused = recomp_game::tes::os_globals::lock_shared(&self.input).focused;
        if !focused && level & recomp_game::tes::os_globals::DISCL_FOREGROUND != 0 {
            return DIERR_OTHERAPPHASPRIO;
        }
        S_OK
    }
    /// IDA: N/A `HostOsPlatform`'s side of `OsGlobalsPlatform`.
    fn poll(&mut self, _site: u32, _device: InputDevice) -> u32 {
        S_OK
    }
    /// IDA: N/A `HostOsPlatform`'s side of `OsGlobalsPlatform`.
    fn get_keyboard_state(
        &mut self,
        _site: u32,
        _device: InputDevice,
        state: &mut [u8; 256],
    ) -> u32 {
        state.copy_from_slice(&recomp_game::tes::os_globals::lock_shared(&self.input).keyboard);
        S_OK
    }
    /// IDA: N/A `HostOsPlatform`'s side of `OsGlobalsPlatform`.
    fn get_mouse_state(
        &mut self,
        _site: u32,
        _device: InputDevice,
        state: &mut DiMouseState2,
    ) -> u32 {
        let mut input = recomp_game::tes::os_globals::lock_shared(&self.input);
        *state = input.mouse;
        input.mouse.x = 0;
        input.mouse.y = 0;
        input.mouse.z = 0;
        S_OK
    }
    /// IDA: N/A `HostOsPlatform`'s side of `OsGlobalsPlatform`.
    fn get_joystick_state(
        &mut self,
        _site: u32,
        _device: InputDevice,
        _state: &mut DiJoyState,
    ) -> u32 {
        S_OK
    }
    /// IDA: N/A no address of its own; its comments cite 00403160.
    fn get_device_data(
        &mut self,
        _site: u32,
        _device: InputDevice,
        _element_size: u32,
        _count: u32,
        _flags: u32,
    ) -> u32 {
        // 00403160 asks with a null buffer; a device with nothing buffered answers DI_OK.
        S_OK
    }
    /// IDA: N/A `HostOsPlatform`'s side of `OsGlobalsPlatform`.
    fn swap_mouse_button(&mut self, _site: u32, _swap: u32) -> u32 {
        0
    }
    /// IDA: N/A `HostOsPlatform`'s side of `OsGlobalsPlatform`.
    fn get_double_click_time(&mut self, _site: u32) -> u32 {
        // The Windows default P04-RUN-001 read.
        0x1F4
    }
    /// IDA: N/A `HostOsPlatform`'s side of `OsGlobalsPlatform`.
    fn direct_sound_create8(&mut self, _site: u32) -> CreatedInterface<()> {
        CreatedInterface {
            result: S_OK,
            object: Some(()),
        }
    }
    /// IDA: N/A `HostOsPlatform`'s side of `OsGlobalsPlatform`.
    fn sound_set_cooperative_level(&mut self, _site: u32, _window: HostWindow, _level: u32) -> u32 {
        S_OK
    }
    /// IDA: N/A `HostOsPlatform`'s side of `OsGlobalsPlatform`.
    fn sound_get_caps(&mut self, _site: u32, _caps: &mut DsCaps) -> u32 {
        S_OK
    }
    /// IDA: N/A `HostOsPlatform`'s side of `OsGlobalsPlatform`.
    fn create_sound_buffer(&mut self, _site: u32, _size: u32, _flags: u32) -> CreatedInterface<()> {
        CreatedInterface {
            result: S_OK,
            object: Some(()),
        }
    }
    /// IDA: N/A `HostOsPlatform`'s side of `OsGlobalsPlatform`.
    fn query_listener(&mut self, _site: u32, _iid: Guid) -> CreatedInterface<()> {
        CreatedInterface {
            result: S_OK,
            object: Some(()),
        }
    }
    /// IDA: N/A `HostOsPlatform`'s side of `OsGlobalsPlatform`.
    fn listener_set_position(&mut self, _site: u32, _x: f32, _y: f32, _z: f32, _apply: u32) -> u32 {
        S_OK
    }
    /// IDA: N/A `HostOsPlatform`'s side of `OsGlobalsPlatform`.
    fn listener_set_rolloff_factor(&mut self, _site: u32, _rolloff: f32, _apply: u32) -> u32 {
        S_OK
    }
    /// IDA: N/A `HostOsPlatform`'s side of `OsGlobalsPlatform`.
    fn co_initialize(&mut self, _site: u32) -> u32 {
        S_OK
    }
}

// ---- the window and pathing seams ----

/// IDA: N/A the Win32 window seam of 0040E766..0040E80C for the Bevy host: the window is `recomp_host`'s, so the call list is kept as a record and the handle is a host value. `AdjustWindowRect` answers P04-RUN-001's rect for {0, 0, 320, 240} with style 00CA0000. `FindWindow` finds nothing: Linux has no window class to search, a recorded substitution for 0040E39B's second-instance branch.
#[derive(Debug, Default)]
pub struct HostWindowPlatform {
    next_window: u32,
}

impl WindowPlatform for HostWindowPlatform {
    /// IDA: N/A `HostWindowPlatform`'s side of `WindowPlatform`.
    fn load_icon(&mut self, _instance: HostInstance, _resource: u32) -> u32 {
        1
    }
    /// IDA: N/A `HostWindowPlatform`'s side of `WindowPlatform`.
    fn get_stock_object(&mut self, _object: u32) -> u32 {
        1
    }
    /// IDA: N/A `HostWindowPlatform`'s side of `WindowPlatform`.
    fn register_class(&mut self, _class: &WindowClass) -> u16 {
        1
    }
    /// IDA: N/A `HostWindowPlatform`'s side of `WindowPlatform`.
    fn adjust_window_rect(&mut self, rect: Rect, _style: u32, _menu: u32) -> Rect {
        // P04-RUN-001 `S 3 40E7E7 RECT`: 3 on the sides and bottom, 26 on top.
        Rect {
            left: rect.left - 3,
            top: rect.top - 26,
            right: rect.right + 3,
            bottom: rect.bottom + 3,
        }
    }
    /// IDA: N/A `HostWindowPlatform`'s side of `WindowPlatform`.
    fn create_window_ex(&mut self, _call: &CreateWindowCall) -> Option<HostWindow> {
        self.next_window += 1;
        Some(HostWindow(self.next_window))
    }
    /// IDA: N/A `HostWindowPlatform`'s side of `WindowPlatform`.
    fn find_window(&mut self, _class_name: &str) -> Option<HostWindow> {
        None
    }
    /// IDA: N/A `HostWindowPlatform`'s side of `WindowPlatform`.
    fn set_foreground_window(&mut self, _window: HostWindow) -> u32 {
        1
    }
    /// IDA: N/A `HostWindowPlatform`'s side of `WindowPlatform`.
    fn get_client_rect(&mut self, _window: HostWindow) -> Rect {
        Rect::default()
    }
}

/// IDA: N/A 0067FB70 and 00682EF0's semaphores and critical sections as host values: the pathing threads aren't ported, so nothing waits on them.
#[derive(Debug, Default)]
pub struct HostPathingPlatform {
    next_handle: u32,
}

impl PathingPlatform for HostPathingPlatform {
    /// IDA: N/A `HostPathingPlatform`'s side of `PathingPlatform`.
    fn allocate(&mut self, _bytes: u32) -> bool {
        true
    }
    /// IDA: N/A `HostPathingPlatform`'s side of `PathingPlatform`.
    fn create_semaphore(&mut self, _initial: i32, _maximum: i32) -> PathingHandle {
        self.next_handle += 1;
        PathingHandle(self.next_handle)
    }
    /// IDA: N/A `HostPathingPlatform`'s side of `PathingPlatform`.
    fn initialize_critical_section(&mut self) -> PathingHandle {
        self.next_handle += 1;
        PathingHandle(self.next_handle)
    }
}

// ---- the game's hosts ----

/// IDA: N/A `DataLoadHost` for a real run: the one files and CRT, the INI's gates, and 0057B950's calls held for the driver, since the seam has no `World`: a held call runs at the next step boundary, in order, and nothing in the load reads what it writes.
pub struct DriverLoadHost {
    pub files: SharedFiles,
    pub crt: MainThreadCrt,
    /// 00B06310 and 00B04428 g_UseArchives, read where 0046D6BB reads them.
    pub texture_gates: (bool, bool),
    /// 00494480's text: 00B3F280 + "Warnings.txt".
    pub warnings_path: String,
    /// 0057B950's calls not yet applied.
    pub sections: VecDeque<(i32, f32)>,
}

impl DataLoadHost for DriverLoadHost {
    /// IDA: N/A `DriverLoadHost`'s side of `DataLoadHost`.
    fn debug_log(&mut self, text: String) {
        emit(Level::Info, &text);
    }
    /// IDA: N/A `DriverLoadHost`'s side of `DataLoadHost`.
    fn section_progress(&mut self, section: i32, percent: f32) {
        self.sections.push_back((section, percent));
    }
    /// IDA: N/A `DriverLoadHost`'s side of `DataLoadHost`.
    fn random_next(&mut self) -> Option<i32> {
        Some(self.crt.random_next())
    }
    /// IDA: N/A no address of its own; its comments cite 00431690.
    fn read_data_file(&mut self, path: &str) -> Option<Vec<u8>> {
        // IDA: 00431690 -> 00430EF0(path, 0, 0x800, 0xFFFF).
        let mut file = self.files.open_core(path, 0, 0x800, 0xFFFF)?;
        let mut bytes = vec![0; file.size() as usize];
        file.read(&mut bytes);
        Some(bytes)
    }
    /// IDA: N/A `DriverLoadHost`'s side of `DataLoadHost`.
    fn absent_arm(&mut self, _form_type: u32) -> AbsentArm {
        // Every record type the shipped files hold has its class. A new one stops rather than guesses.
        AbsentArm::Stop
    }
    /// IDA: N/A `DriverLoadHost`'s side of `DataLoadHost`.
    fn find_texture_entry(&mut self, folder_hash: u64, file_hash: u64) -> TextureFileEntry {
        let mut files = self.files.lock();
        files
            .archives
            .as_mut()?
            .file_record_by_hashes(1, folder_hash, file_hash)?;
        Some((folder_hash, file_hash))
    }
    /// IDA: N/A `DriverLoadHost`'s side of `DataLoadHost`.
    fn find_file(&mut self, path: &str, flags: u32, mask: i32) -> i32 {
        self.files
            .find_file(path, flags, mask)
            .map_or(0, |(status, _)| status)
    }
    /// IDA: N/A `DriverLoadHost`'s side of `DataLoadHost`.
    fn apply_archive_cache_settings(&mut self) {
        if let Some(archives) = self.files.lock().archives.as_mut() {
            archives.apply_cache_settings();
        }
    }
    /// IDA: N/A `DriverLoadHost`'s side of `DataLoadHost`.
    fn find_matching_files(&mut self, pattern: &str, source: &str, mask: u32) -> Vec<String> {
        self.files
            .lock()
            .archives
            .as_mut()
            .map_or_else(Vec::new, |archives| {
                archives.find_matching_files(pattern, source, mask)
            })
    }
    /// IDA: N/A `DriverLoadHost`'s side of `DataLoadHost`.
    fn texture_warning_gates(&self) -> (bool, bool) {
        self.texture_gates
    }
    /// IDA: N/A `DriverLoadHost`'s side of `DataLoadHost`.
    fn warnings_log_path(&self) -> String {
        self.warnings_path.clone()
    }
}

/// IDA: N/A what `render_interface` tells the host, so that the screens agent can draw from a Bevy system with the World in hand: the render happens at the next frame, a recorded substitution for 0057F170's place inside a block.
#[derive(Clone, Debug, Default)]
pub struct InterfaceRenderRequests(Arc<Mutex<Vec<u32>>>, Arc<AtomicU64>);

impl InterfaceRenderRequests {
    /// IDA: N/A a helper, with no address of its own.
    pub fn push(&self, argument: u32) {
        recomp_game::tes::os_globals::lock_shared(&self.0).push(argument);
        self.1.fetch_add(1, Ordering::AcqRel);
    }
    /// IDA: N/A how many requests there have been: each is a 0057F170, whose scene ends in the game's Present.
    #[must_use]
    pub fn count(&self) -> u64 {
        self.1.load(Ordering::Acquire)
    }
    /// IDA: N/A the requests since the last call, in order.
    #[must_use]
    pub fn take(&self) -> Vec<u32> {
        std::mem::take(&mut *recomp_game::tes::os_globals::lock_shared(&self.0))
    }
}

/// IDA: N/A `InterfaceHost` for a real run. A texture's size comes from its DDS header, a recorded substitution for 00591360's device texture (`create_texture_from_file` needs the World, which this seam doesn't have).
pub struct DriverInterfaceHost {
    pub files: SharedFiles,
    pub crt: MainThreadCrt,
    pub clock: HostClock,
    pub movie: Option<MoviePlayer>,
    pub input: Option<SharedOsInput>,
    pub sound: Option<recomp_game::tes::os_globals::GameSoundPause>,
    /// 00B16178.
    pub sound_enabled: bool,
    /// 0057F170, for `recomp_host`.
    pub renders: InterfaceRenderRequests,
    /// g_dx9Renderer +0x200, set while the interface renders: 00410390 waits on it.
    pub in_scene: Arc<AtomicBool>,
    /// 00578E00's calls, run by the driver, which holds the World.
    pub loading_updates: u32,
    /// `WinMainSeams::screens`.
    pub screens: Rc<RefCell<ScreenSeams>>,
    /// 00B3F280 + "Saves\\".
    pub saves_directory: PathBuf,
    /// g_osGlobals->quitGame.
    pub quit: Rc<Cell<bool>>,
    /// IDA: N/A a recorded host substitution. A seeded run's GetTickCount doesn't move, and 005ADB40 loops on it, so inside that loop each read is 11 ms on. `NullInterfaceHost`'s step, past +0x68's wait of 10.
    pub seeded_fill_ticks: Option<Rc<Cell<u32>>>,
    /// What 005B5F20's media reads.
    pub menu_media: Option<crate::winmain::MenuMediaContext>,
}

/// IDA: N/A what the first screens ask of the sound host and of later phases: no sound until it's filled, and `start_game` a host stop by return.
pub struct ScreenSeams {
    /// 0057DE50.
    pub play_ui_sound: Box<dyn FnMut(u32)>,
    /// 005AE190, 005B5D60 and 005B5DF0's Yes.
    pub start_game: Box<dyn FnMut(GameStartRequest)>,
}

impl Default for ScreenSeams {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            play_ui_sound: Box::new(|_| {}),
            start_game: Box::new(|request| {
                emit(
                    Level::Warning,
                    &format!("{request:?}: a game start is a later phase's"),
                );
            }),
        }
    }
}

impl InterfaceHost for DriverInterfaceHost {
    /// IDA: N/A `DriverInterfaceHost`'s side of `InterfaceHost`.
    fn open_font_file(&mut self, path: &str) -> Option<OpenedFile> {
        self.files.open_core(path, 0, 0x2800, 0x80)
    }
    /// IDA: N/A `DriverInterfaceHost`'s side of `InterfaceHost`.
    fn read_tile_file(&mut self, path: &str) -> Option<Vec<u8>> {
        let mut file = self.files.open_core(path, 0, 0x2800, 4)?;
        if !file.good() {
            return None;
        }
        let mut bytes = vec![0; file.size() as usize];
        let _ = file.read(&mut bytes);
        Some(bytes)
    }
    /// IDA: N/A `DriverInterfaceHost`'s side of `InterfaceHost`.
    fn file_exists(&mut self, path: &str) -> bool {
        self.files
            .find_file(path, 0, -1)
            .is_some_and(|(status, _)| status != 0)
    }
    /// IDA: N/A `DriverInterfaceHost`'s side of `InterfaceHost`.
    fn texture_size(&mut self, path: &str) -> Option<[u32; 2]> {
        let mut file = self.files.open_core(path, 0, 0x2800, 0xFFFF)?;
        let mut bytes = vec![0; file.size() as usize];
        let _ = file.read(&mut bytes);
        let word = |offset: usize| {
            bytes
                .get(offset..offset + 4)
                .map(|field| u32::from_le_bytes(field.try_into().expect("four bytes")))
        };
        bytes.starts_with(b"DDS ").then_some(())?;
        Some([word(0x10)?, word(0x0C)?])
    }
    /// IDA: N/A `DriverInterfaceHost`'s side of `InterfaceHost`.
    fn movie_player_is_running(&mut self) -> bool {
        self.movie.as_ref().is_some_and(movie_player_is_running)
    }
    /// IDA: N/A no address of its own; its comments cite 0057F3A5.
    fn render_interface(&mut self, argument: u32) {
        self.in_scene.store(true, Ordering::Release);
        // 0057F3A5..0057F3BC: a menu-owned movie (state 2) presents under the tiles, 00410810(0, 1). Its only caller on this path.
        if let Some(player) = self.movie.as_ref() {
            let current =
                recomp_game::tes::os_globals::lock_shared(&player.globals.current_b33428).clone();
            if current.is_some_and(|movie| movie.state() == MOVIE_STATE_MENU_OWNED) {
                present_current_movie(player, false, true);
            }
        }
        self.renders.push(argument);
        self.in_scene.store(false, Ordering::Release);
    }
    /// IDA: N/A no address of its own; its comments cite 00403190.
    fn mouse_axis(&mut self, axis: i32) -> i32 {
        let Some(input) = self.input.as_ref() else {
            return 0;
        };
        // 00403190: DIMOUSESTATE2 lX, lY, lZ (+0x1B20..).
        let input = input.lock();
        match axis {
            1 => input.mouse_state_1b20.x,
            2 => input.mouse_state_1b20.y,
            3 => input.mouse_state_1b20.z,
            _ => 0,
        }
    }
    /// IDA: N/A `DriverInterfaceHost`'s side of `InterfaceHost`.
    fn random_next(&mut self) -> i32 {
        self.crt.random_next()
    }
    /// IDA: N/A `DriverInterfaceHost`'s side of `InterfaceHost`.
    fn loading_update_display(&mut self) {
        self.loading_updates += 1;
    }
    /// IDA: N/A `DriverInterfaceHost`'s side of `InterfaceHost`.
    fn warning(&mut self, text: &str) {
        emit(Level::Warning, text);
    }
    /// IDA: N/A `DriverInterfaceHost`'s side of `InterfaceHost`.
    fn output_debug_string(&mut self, text: &str) {
        emit(Level::Debug, text);
    }
    /// IDA: N/A `DriverInterfaceHost`'s side of `InterfaceHost`.
    fn debug_log(&mut self, text: &str) {
        emit(Level::Debug, text);
    }
    /// IDA: N/A `DriverInterfaceHost`'s side of `InterfaceHost`.
    fn tick_count(&mut self) -> u32 {
        let tick = self.clock.tick();
        self.seeded_fill_ticks.as_ref().map_or(tick, |ticks| {
            ticks.set(ticks.get().wrapping_add(11));
            tick.wrapping_add(ticks.get())
        })
    }
    /// IDA: N/A `DriverInterfaceHost`'s side of `InterfaceHost`.
    fn pause_game_sounds(&mut self) {
        if let Some(sound) = self.sound.as_ref() {
            sound.pause(self.sound_enabled);
        }
    }
    /// IDA: N/A `DriverInterfaceHost`'s side of `InterfaceHost`: _strtol stops at the first byte that isn't hex.
    fn chargen_quest_id(&mut self) -> u32 {
        let Some(context) = self.menu_media.as_ref() else {
            return 0x0002_466E;
        };
        let digits: String = context
            .chargen_quest_b1436c
            .iter()
            .map(|&byte| char::from(byte))
            .take_while(char::is_ascii_hexdigit)
            .collect();
        u32::from_str_radix(&digits, 16).unwrap_or(0)
    }
    /// IDA: N/A `DriverInterfaceHost`'s side of `InterfaceHost`.
    fn new_game_music(&mut self, world: &mut World) {
        let Some(context) = self.menu_media.as_ref() else {
            return;
        };
        let has_cell = context.player_has_cell;
        let mut music = recomp_game::tes::os_globals::lock_shared(&context.music);
        let crt = self.crt.clone();
        let mut random = || crt.random_next();
        let mut os = world.resource_mut::<recomp_game::tes::os_globals::OsGlobalsData>();
        if let Some(sound) = os.sound_24.as_mut() {
            sound.main_menu_music_b3c0ec = 0;
            sound.load_random_music_track(
                0xFFFF,
                has_cell,
                &context.settings,
                &mut *music,
                &mut random,
            );
            sound.load_random_music_track(2, has_cell, &context.settings, &mut *music, &mut random);
            sound.start_music_playback(&context.settings, &mut *music);
        }
    }
    /// IDA: N/A `DriverInterfaceHost`'s side of `InterfaceHost`.
    fn main_menu_destructor_media(&mut self, world: &mut World) {
        let Some(context) = self.menu_media.as_ref() else {
            return;
        };
        let has_cell = context.player_has_cell;
        let paused_movie = self.movie.as_ref().is_some_and(|player| {
            recomp_game::tes::os_globals::lock_shared(&player.globals.current_b33428)
                .as_ref()
                .is_some_and(|movie| movie.state() != 0 && movie.state() != MOVIE_STATE_MENU_OWNED)
        });
        let mut music = recomp_game::tes::os_globals::lock_shared(&context.music);
        let crt = self.crt.clone();
        let mut random = || crt.random_next();
        let mut os = world.resource_mut::<recomp_game::tes::os_globals::OsGlobalsData>();
        let Some(sound) = os.sound_24.as_mut() else {
            return;
        };
        sound.main_menu_music_b3c0ec = 0;
        sound.resume_music_playback(&mut *music);
        if sound.load_random_music_track(
            0xFFFF,
            has_cell,
            &context.settings,
            &mut *music,
            &mut random,
        ) {
            sound.start_music_playback(&context.settings, &mut *music);
        }
        if paused_movie {
            if let Some(game_sounds) = self.sound.as_ref() {
                game_sounds.pause(self.sound_enabled);
            }
            sound.pause_music_playback(&mut *music);
        }
    }
    /// IDA: N/A `DriverInterfaceHost`'s side of `InterfaceHost`.
    fn main_menu_ctor_media(&mut self, world: &mut World) -> bool {
        self.menu_media
            .as_ref()
            .is_some_and(|context| crate::winmain_menu_media::main_menu_ctor_media(world, context))
    }
    /// IDA: N/A `DriverInterfaceHost`'s side of `InterfaceHost`.
    fn resume_game_sounds_after_loading(&mut self) {
        let Some(sound) = self.sound.as_ref() else {
            return;
        };
        let state = self.movie.as_ref().and_then(|player| {
            recomp_game::tes::os_globals::lock_shared(&player.globals.current_b33428)
                .as_ref()
                .map(|movie| movie.state())
        });
        if state.is_none_or(|state| state == 0 || state == 2) {
            sound.resume(self.sound_enabled);
        }
    }
    /// IDA: N/A `DriverInterfaceHost`'s side of `InterfaceHost`.
    fn play_ui_sound(&mut self, sound: u32) {
        (self.screens.borrow_mut().play_ui_sound)(sound);
    }
    /// IDA: N/A `DriverInterfaceHost`'s side of `InterfaceHost`.
    fn save_file_list(&mut self) -> Vec<SaveFile> {
        rebuild_save_file_list(&self.saves_directory)
    }
    /// IDA: N/A `DriverInterfaceHost`'s side of `InterfaceHost`.
    fn start_game(&mut self, request: GameStartRequest) {
        (self.screens.borrow_mut().start_game)(request);
    }
    /// IDA: N/A `DriverInterfaceHost`'s side of `InterfaceHost`.
    fn quit_game(&mut self) {
        self.quit.set(true);
    }
}

/// IDA: 00591360 TileImage_LoadTexture's middle with the renderer the frame holds: the palette's texture, else FindFile and 00701E00(path, {6, 3, 2}, 1), and the palette's add unless the path holds "oading\\load_" (_mbsstr), so a loading screen's slide isn't kept. The answer is 00780D10 and 0076DDF0's width and height. Completion: 100%. It replaces the DDS-header seam for the driver.
pub struct DriverTileTextures<'a> {
    pub files: &'a SharedFiles,
    pub creation: &'a mut RendererCreation,
    pub statistics: &'a mut TextureMemoryStatistics,
    pub gates: (bool, bool),
}

impl recomp_game::gamebryo::scene_update::TileTextureLoader for DriverTileTextures<'_> {
    /// IDA: N/A `DriverTileTextures`'s side of `TileTextureLoader`.
    fn load(&mut self, world: &mut World, path: &str) -> Option<[u32; 2]> {
        let bytes: Vec<u8> = path.chars().map(|c| c as u8).collect();
        let found = self.files.palette_find(&bytes);
        let texture = if let Some(texture) = found {
            texture
        } else {
            if self
                .files
                .find_file(path, 0, -1)
                .is_none_or(|(status, _)| status == 0)
            {
                return None;
            }
            let mut services = texture_services(
                self.files,
                self.creation,
                self.gates.0,
                self.gates.1,
                self.statistics,
            );
            let texture = create_texture_from_file(world, &bytes, [6, 3, 2], true, &mut services)?;
            drop(services);
            if !bytes.windows(12).any(|window| window == b"oading\\load_") {
                self.files.palette_add(&bytes, Some(texture));
            }
            texture
        };
        let source = world.get::<SourceTexture>(texture)?;
        let data = source.renderer_24.as_ref()?;
        Some([data.width_54, data.height_58])
    }
}

/// IDA: N/A 00442890's real targets: the palette, the FileFinder, 00701E00 with 00B256D0 {6, 3, 2} and g_IOManager +0x30.
pub struct DriverWaterTextures<'a> {
    pub files: SharedFiles,
    pub creation: &'a mut RendererCreation,
    pub drop_static_app_data: bool,
    pub direct_to_renderer_hint: bool,
    pub statistics: &'a mut TextureMemoryStatistics,
    pub io_thread: ThreadId,
}

impl WaterTextureServices for DriverWaterTextures<'_> {
    /// IDA: N/A `DriverWaterTextures`'s side of `WaterTextureServices`.
    fn palette_find(&mut self, _world: &mut World, path: &[u8]) -> Option<Entity> {
        self.files.palette_find(path)
    }
    /// IDA: N/A `DriverWaterTextures`'s side of `WaterTextureServices`.
    fn palette_add(&mut self, _world: &mut World, path: &[u8], texture: Entity) {
        self.files.palette_add(path, Some(texture));
    }
    /// IDA: N/A `DriverWaterTextures`'s side of `WaterTextureServices`.
    fn find_file(&mut self, path: &[u8], flags: u32) -> Option<(u32, Vec<u8>)> {
        self.files
            .find_file(&path_text(path), flags, -1)
            .map(|(status, resolved)| (status as u32, resolved))
    }
    /// IDA: N/A `DriverWaterTextures`'s side of `WaterTextureServices`.
    fn create_from_file(&mut self, world: &mut World, path: &[u8]) -> Option<Entity> {
        let mut services = texture_services(
            &self.files,
            self.creation,
            self.drop_static_app_data,
            self.direct_to_renderer_hint,
            self.statistics,
        );
        create_texture_from_file(world, path, [6, 3, 2], true, &mut services)
    }
    /// IDA: N/A `DriverWaterTextures`'s side of `WaterTextureServices`.
    fn io_thread_id(&self) -> ThreadId {
        self.io_thread
    }
}

/// IDA: N/A the working directory for a data folder: the original's process directory, whose `Data\` it is.
#[must_use]
pub fn working_directory_of(data_directory: &Path) -> PathBuf {
    let absolute = std::path::absolute(data_directory).unwrap_or_else(|_| data_directory.into());
    absolute
        .parent()
        .map_or_else(|| absolute.clone(), Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use recomp_game::tes::actor_process::construct_actor_process_manager;

    fn fresh_world() -> World {
        let mut world = World::new();
        world.init_resource::<recomp_game::tes::tes::GameThreadLocals>();
        construct_actor_process_manager(&mut world);
        world
    }

    // IDA: 0047DF80, _srand and _rand by hand: the stream every seam must give.
    fn expected(seed: u32, draws: usize) -> Vec<i32> {
        let mut rand = MsvcRand::default();
        rand.srand(seed);
        (0..draws).map(|_| rand.rand()).collect()
    }

    // `DataLoadHost::random_next`, the tile engine's and `ActorProcessGlobals.random`'s path are one stream, seeded once.
    #[test]
    fn every_seam_draws_from_one_stream() {
        let clock = HostClock::new(Some(0x1234_5678));
        let crt = MainThreadCrt::new(clock);
        let files = SharedFiles::new(PathBuf::from("/nonexistent"));
        let mut load = DriverLoadHost {
            files: files.clone(),
            crt: crt.clone(),
            texture_gates: (false, false),
            warnings_path: String::new(),
            sections: VecDeque::new(),
        };
        let mut interface = DriverInterfaceHost {
            files,
            crt: crt.clone(),
            clock,
            movie: None,
            input: None,
            sound: None,
            sound_enabled: false,
            renders: InterfaceRenderRequests::default(),
            in_scene: Arc::new(AtomicBool::new(false)),
            loading_updates: 0,
            screens: Rc::new(RefCell::new(ScreenSeams::default())),
            saves_directory: PathBuf::new(),
            quit: Rc::new(Cell::new(false)),
            seeded_fill_ticks: None,
            menu_media: None,
        };
        let mut world = fresh_world();
        let want = expected(0x1234_5678, 5);
        // The tile engine's 0058C47E draws first: the first use seeds.
        assert_eq!(InterfaceHost::random_next(&mut interface), want[0]);
        assert_eq!(DataLoadHost::random_next(&mut load), Some(want[1]));
        // The World's path sees 00B069C3 clear and doesn't seed again.
        let from_world = crt.with_world_flag(&mut world, |world, source| {
            let mut globals = world.resource_mut::<ActorProcessGlobals>();
            recomp_game::tes::actor_process::random_next(0, &mut globals.random, source)
        });
        assert_eq!(from_world, want[2]);
        assert_eq!(
            world
                .resource::<ActorProcessGlobals>()
                .random
                .first_use_b069c3,
            0
        );
        assert_eq!(crt.rand(), want[3]);
        assert_eq!(crt.random_next(), want[4]);
        assert_eq!(crt.draws(), 5);
    }

    // The World's path can draw first too, and the seams see the flag it cleared.
    #[test]
    fn the_first_use_byte_follows_the_world_path() {
        let crt = MainThreadCrt::new(HostClock::new(Some(77)));
        let mut world = fresh_world();
        assert_eq!(
            world
                .resource::<ActorProcessGlobals>()
                .random
                .first_use_b069c3,
            1,
            "the image's byte"
        );
        let first = crt.with_world_flag(&mut world, |world, source| {
            let mut globals = world.resource_mut::<ActorProcessGlobals>();
            recomp_game::tes::actor_process::random_next(0, &mut globals.random, source)
        });
        let want = expected(77, 2);
        assert_eq!(first, want[0]);
        assert_eq!(crt.random_next(), want[1], "no second seed");
    }

    // Before the first use, `_rand` runs from the CRT's holdrand of 1, and doesn't clear the byte.
    #[test]
    fn plain_rand_does_not_seed() {
        let crt = MainThreadCrt::new(HostClock::new(Some(77)));
        let mut plain = MsvcRand::default();
        assert_eq!(crt.rand(), plain.rand());
        assert_eq!(crt.random_next(), expected(77, 1)[0]);
    }

    #[test]
    fn a_seeded_clock_is_fixed() {
        let clock = HostClock::new(Some(9));
        assert_eq!(clock.tick(), SEEDED_TICK);
        assert_eq!(clock.time64(), 9);
        assert!(HostClock::new(None).time64() > 1_000_000_000);
    }

    // One numbering: the main thread's OS seam answers what the movie thread's platform answers on that thread.
    #[test]
    fn the_os_platform_uses_the_movie_threads_numbering() {
        let mut platform = HostOsPlatform::default();
        assert_eq!(platform.get_current_thread_id(0), host_thread_id());
        assert_eq!(
            platform.duplicate_handle(0, 0, 0xFFFF_FFFE, 0, 0, 0, 0),
            (0, 0)
        );
        let other = std::thread::spawn(host_thread_id).join().unwrap();
        assert_ne!(other, host_thread_id());
    }

    // P04-RUN-001's `S 3 40E7E7 RECT` and `C 4 40E80C`: what AdjustWindowRect answered, and the size CreateWindowExA took from it.
    #[test]
    fn the_window_rect_is_the_one_the_run_recorded() {
        let rows = recomp_core::fixtures::rows("phase04");
        let rect = rows
            .iter()
            .find(|row| row.starts_with("P04-RUN-001 ") && row.contains(" 40E7E7 RECT "))
            .unwrap();
        let hex = rect.rsplit("rect=").next().unwrap();
        let word = |index: usize| {
            let bytes = u32::from_str_radix(&hex[index * 8..index * 8 + 8], 16).unwrap();
            bytes.swap_bytes() as i32
        };
        let create = rows
            .iter()
            .find(|row| row.starts_with("P04-RUN-001 ") && row.contains(" 40E80C "))
            .unwrap();
        let arguments: Vec<&str> = create.rsplit("a=").next().unwrap().split(',').collect();
        let mut windows = HostWindowPlatform::default();
        let main = crate::winmain_window::create_original_main_window(
            HostInstance(0x0040_0000),
            &mut windows,
        );
        assert_eq!(
            main.bootstrap,
            Rect {
                left: word(0),
                top: word(1),
                right: word(2),
                bottom: word(3),
            }
        );
        let size = (
            i32::from_str_radix(arguments[6], 16).unwrap(),
            i32::from_str_radix(arguments[7], 16).unwrap(),
        );
        assert_eq!(size, (0x146, 0x10D));
        assert_eq!(
            (
                main.bootstrap.right - main.bootstrap.left,
                main.bootstrap.bottom - main.bootstrap.top
            ),
            size
        );
    }

    #[test]
    fn the_os_platform_answers_devices_from_the_shared_input() {
        let mut platform = HostOsPlatform::default();
        assert_eq!(
            platform.create_device(0, GUID_SYS_KEYBOARD).object,
            Some(HOST_KEYBOARD)
        );
        let missing = platform.create_device(0, Guid::new(1, 2, 3, [0; 8]));
        assert_eq!((missing.result, missing.object), (E_FAIL, None));
        recomp_game::tes::os_globals::lock_shared(&platform.input).keyboard[3] = 0x80;
        let mut keys = [0; 256];
        assert_eq!(
            platform.get_keyboard_state(0, HOST_KEYBOARD, &mut keys),
            S_OK
        );
        assert_eq!(keys[3], 0x80);
    }
}
