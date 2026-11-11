//! The WinMain driver: `0040DF80` from its first instruction through `0040F08C`, one exclusive system per block, in an explicit chain, on the app's `World`. `PhaseOutcome` replaces `main.cpp`'s nesting. Nothing else touches game state while a block runs. `oblivion_host` schedules the chain once per frame, and a block that's long (`LoadSelectedFiles::step`, `ModelDrain::step`) returns when its budget is spent, so Bevy can draw a frame. Game-visible order doesn't change; only frame boundaries do. A test runs the chain straight through with no budget.
//!
//! IDA: 0040DF80 WinMain. Read in full, instruction by instruction, from 0040DF80 to 0040F0E0. Completion: every block up to 0040F0DF with each callee's own completion at its port.
//!
//! TODO: [ ] 0040F28D, the return to the menu, and 0040F488, shutdown, are later milestones. 0040F0E0..0040F275 is the host's frame: `winmain_screens` and `winmain_menu_media`.

pub mod game_frame;
pub mod initialize_game_data;
pub mod initialize_scene_graphs;
pub mod path_setup;
pub mod plugin_selection;
pub mod winmain_host;
pub mod winmain_initial_area;
pub mod winmain_interface;
pub mod winmain_memory;
pub mod winmain_menu_media;
pub mod winmain_movie;
pub mod winmain_place_player;
pub mod winmain_player_scene;
pub mod winmain_renderer;
pub mod winmain_scene;
pub mod winmain_screens;
pub mod winmain_settings;
pub mod winmain_shader;
pub mod winmain_starting_spell;
pub mod winmain_window;
pub mod winmain_world_setup;

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};
use std::thread::ThreadId;
use std::time::{Duration, Instant};

use crate::bs::archive::ArchiveAccess as _;
use crate::bs::archive::ArchiveManager;
use crate::bs::character_controller::CharacterControllerGlobals;
use crate::bs::effect_textures::load_shader_texture;
use crate::bs::file_finder::FileFinder;
use crate::bs::model_loader::{ModelBuildServices, ModelDrain, ModelDrainStep};
use crate::bs::scene_graphs::{
    CameraStaticAliases, SceneGraphGlobals, SceneGraphSettings, scene_graph_settings_from,
};
use crate::bs::shader::{BsShaderGlobals, BsShaderSlot, get_bs_shader_slot};
use crate::bs::sky_shader::PrepareGeometryContext;
use crate::bs::stream::{BsStreamLoad, BsStreamServices, load_bs_stream_from_path};
use crate::bs::task_manager::TaskGlobals;
use crate::gamebryo::device_caps::{DeviceCapsFixture, PinnedRendererPlatform};
use crate::gamebryo::dx9_system::WindowHandle;
use crate::gamebryo::particle_controller_schedule::ParticleControllerScheduleContext;
use crate::gamebryo::registration::{
    NiD3DShaderCreatorRegistry, NiFactoryRegistry, SingletonTail, StartupCallbackScheduler,
    register_known_crt_callbacks, set_device_image_converter_default,
};
use crate::gamebryo::renderer::{
    RendererCallbackPairs, RendererCreation, RendererCreationInputs, RendererUiGlobals,
    RendererUiState, pixel_format_words, renderer_settings_from,
};
use crate::gamebryo::scene_update::{SceneControllers, TileTextureLoader};
use crate::gamebryo::stream_postload::StreamPostLoadRuntime;
use crate::gamebryo::texture_loading::create_file_renderer_data;
use crate::tes::actor_process::{HighProcess, ProcessInputs, construct_actor_process_manager};
use crate::tes::ai_process::{AiProcessGlobals, AiRealArms};
use crate::tes::bink_library::{BinkLibrary, ScriptedBink};
use crate::tes::blend_settings::{
    BlendSettingsState, blend_settings_version_file, initialize_blend_settings,
};
use crate::tes::character_controller::{ControllerInputs, actor_character_controller};
use crate::tes::data_handler::{
    DataHandler, FormsById, build_data_handler_file_list, construct_data_handler,
};
use crate::tes::data_handler_world::propagate_runtime_field_to_masters;
use crate::tes::data_load::{DataLoadGlobals, DataLoadServices, LoadSelectedFiles};
use crate::tes::default_forms::DefaultFormStatics;
use crate::tes::facegen::{
    FaceGenInputs, FaceGenLastError, FaceGenSingleton, ensure_facegen_initialized,
    renderer_facegen_upload,
};
use crate::tes::form::FormIdentity;
use crate::tes::form_arms::editor_id_of;
use crate::tes::forms_references::save_load_active;
use crate::tes::game_settings::GameSettingCollection;
use crate::tes::hud_main_menu::HudSettings;
use crate::tes::interface_frame::menu_begin_fade_in;
use crate::tes::interface_manager::{
    InterfaceManager, InterfaceMovieReads, InterfaceSettings, interface_refresh_map_menu,
    interface_render_if_no_movie, interface_settings_from,
};
use crate::tes::interface_player_scene::{PlayerSceneSettings, interface_initialize_player_scene};
use crate::tes::inventory::InventoryGlobals;
use crate::tes::loading_menu::LoadingMenuClosing;
use crate::tes::loading_menu::{
    loading_menu_close_begin, loading_menu_close_end, loading_menu_complete_step,
    loading_menu_set_section_progress_if_ready, loading_menu_update_display,
};
use crate::tes::magic_selection::{queued_magic_item_finish, queued_magic_item_run};
use crate::tes::menus::{main_menu_begin_intro, main_menu_create};
use crate::tes::movie_player::{
    HostMoviePlatform, MovieDevice, MovieOsGlobals, MoviePlatform, MoviePlayer,
    MoviePlayerServices, MovieQueueEntry, QueuedMovieDevice, SplashLoader,
    movie_player_poll_wait_for_thread, movie_player_shutdown, movie_settings_from,
};
use crate::tes::os::renderer_init::{initialize_renderer, initialize_renderer_ui_state};
use crate::tes::os_globals::{
    HostInstance, HostWindow, OsGlobalsData, OsGlobalsPlatform, SoundSourceGlobal,
    construct_os_globals, lock_shared,
};
use crate::tes::os_sound_music::{MusicDevice, MusicSettings, ScriptedMusicDevice};
use crate::tes::pathing::PathingSingletons;
use crate::tes::paths::GamePaths;
use crate::tes::player::{
    PLAYER_FORM_ID, PlayerGlobals, PlayerInputs, particle_update_fov_scale, player_setting_inputs,
};
use crate::tes::settings::{SettingCollections, archive_settings_from, register_static_settings};
use crate::tes::sky::{SkyGlobals, SkyInitServices};
use crate::tes::tes::{
    GameThreadLocals, TesConstructionInputs, TesGlobals, TesPreloadServices, tes_setting_inputs,
};
use crate::tes::tes_grid::{TesGridAnswers, TesGridServices};
use crate::tes::tile::{InterfaceCx, InterfaceHost, TileEngine, with_interface_cx};
use crate::tes::tile_values::{TRAIT_VISIBLE, tile_numeric_trait, tile_set_numeric_trait};
use crate::tes::water::WaterServices;
use crate::tes::window::{SharedTimeInfo, WindowProcedureState};
use bevy_ecs::prelude::*;
use oblivion_core::crt::MsvcRand;
use oblivion_core::error::GameError;
use oblivion_core::logging::{Level, emit};

use crate::bs::shader_manager::{
    ShaderManagerGlobals, TestSphereInputs, TestSphereStatus, shader_accumulator_singleton,
};
use crate::gamebryo::scene_state::property_defaults;
use crate::tes::main_menu::main_menu_begin_backdrop_movie;
use crate::tes::winmain::initialize_game_data::{
    PluginBlockInputs, ScriptsSoundInputs, SkyPreludeInputs, SkyPreludeResult, TesBlockInputs,
    begin_load_selected_files, run_player_base, run_player_init, run_player_window,
    run_plugin_block, run_scripts_sound_and_clamp, run_sky_prelude, run_tes_block, run_tes_preload,
};
use crate::tes::winmain::initialize_scene_graphs::publish_camera_aliases;
use crate::tes::winmain::path_setup::{
    disc_check_passed, host_path_roots, original_path_buffer_overflow, plan_startup_paths,
    use_my_games_from_bootstrap,
};
use crate::tes::winmain::winmain_host::{
    DriverInterfaceHost, DriverLoadHost, DriverSourceTextures, DriverTileTextures,
    DriverWaterTextures, HostClock, HostInputFrame, HostOsPlatform, HostPathingPlatform,
    HostWindowPlatform, InterfaceRenderRequests, MainThreadCrt, ScreenSeams, SharedFiles,
    texture_services, working_directory_of,
};
use crate::tes::winmain::winmain_initial_area::{
    InitialAreaPath, RealInitialAreaArms, run_initial_area,
};
use crate::tes::winmain::winmain_interface::run_winmain_interface_window;
use crate::tes::winmain::winmain_memory::StartupMemoryState;
use crate::tes::winmain::winmain_menu_media::{
    MainMenuMedia, MainMenuMediaStage, main_menu_title_and_logo,
};
use crate::tes::winmain::winmain_movie::run_winmain_movie_window;
use crate::tes::winmain::winmain_place_player::{RealPlacePlayerArms, run_place_player};
use crate::tes::winmain::winmain_player_scene::run_player_scene;
use crate::tes::winmain::winmain_renderer::register_winmain_renderer_callbacks;
use crate::tes::winmain::winmain_scene::{SceneWindow, SceneWindowServices, run_scene_window};
use crate::tes::winmain::winmain_screens::{ScreenDirector, ScreenStage};
use crate::tes::winmain::winmain_settings::load_winmain_settings;
use crate::tes::winmain::winmain_starting_spell::{
    save_player_initial_state, select_starting_spell,
};
use crate::tes::winmain::winmain_window::{
    ORIGINAL_WINDOW_CLASS, activate_existing_window, create_original_main_window,
    replay_startup_activation,
};
use crate::tes::winmain::winmain_world_setup::{
    WorldspaceInputs, run_actor_locations_prelude, run_tracked_actors, run_world_setup_finish,
    run_world_setup_worldspace,
};

// ---- what a run is given ----

/// IDA: N/A where the settings come from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SettingsSource {
    /// `--ini PATH`: that INI, with the other per-user files beside it (`fixtures/config` for a test), read-only: nothing is created or copied. 0040DFCE's bUseMyGamesDirectory arm, on the given folder.
    Pinned(PathBuf),
    /// The user's Documents folder, per `path_setup`: Documents and LocalAppData come from `host_path_roots`. Read-only: 0040E043, 0040E0B8, 0040E0F3's directories and 0040E221's copy are not made (user, 2026-11-05: read-only), a recorded substitution.
    UserDocuments,
}

/// IDA: N/A what a driver run is given.
#[derive(Clone, Debug)]
pub struct WinMainConfig {
    /// `OBLIVION_DATA_DIR`, the folder whose parent is the original's process directory.
    pub data_directory: PathBuf,
    pub settings: SettingsSource,
    /// `--seed`: `_time64` answers this, and GetTickCount answers `SEEDED_TICK`. `None` is real time.
    pub seed: Option<u32>,
    /// How long a resumable block may run in one frame. `None` runs it straight through.
    pub frame_budget: Option<Duration>,
}

/// IDA: N/A the seams `oblivion_host` and later agents fill. Each default works with nothing behind it.
pub struct WinMainSeams {
    /// binkw32 imports 00A28330..00A28350: scripted, one frame per movie, until the movie agent supplies the decoder.
    pub bink: Arc<Mutex<dyn BinkLibrary>>,
    /// IDA: N/A a size FormHeap refuses, for the failure-path tests.
    pub failed_allocations: Vec<u32>,
    /// IDA: N/A each of the movie thread's device calls, on the main thread, at the frame's end.
    pub movie_device: Box<dyn FnMut(MovieQueueEntry)>,
    /// IDA: N/A the first screens' calls to sound, movies and later phases.
    pub screens: Rc<RefCell<ScreenSeams>>,
    /// IDA: N/A D3DXCreateTextureFromFileA's decode for 00410840. `None` fails it as a missing file does.
    pub splash_loader: Option<SplashLoader>,
    /// IDA: N/A the DirectShow graph of OSSoundGlobals' music: scripted, with no sound, until the host supplies a device.
    pub music: Arc<Mutex<dyn MusicDevice>>,
}

impl Default for WinMainSeams {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            bink: Arc::new(Mutex::new(ScriptedBink::default())),
            failed_allocations: Vec::new(),
            movie_device: Box::new(|_| {}),
            screens: Rc::new(RefCell::new(ScreenSeams::default())),
            splash_loader: None,
            music: Arc::new(Mutex::new(ScriptedMusicDevice::default())),
        }
    }
}

/// IDA: N/A the selected paths, for the host to read the INI before the window exists.
#[must_use]
pub fn selected_paths(source: &SettingsSource, data_directory: &Path) -> GamePaths {
    let working = working_directory_of(data_directory);
    match source {
        SettingsSource::Pinned(ini) => {
            let folder = ini
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
            GamePaths {
                use_my_games: true,
                bootstrap_ini: working.join("Oblivion.ini"),
                settings_directory: folder.clone(),
                local_data_directory: folder.clone(),
                selected_ini: ini.clone(),
                saves_directory: folder.join("Saves"),
            }
        }
        SettingsSource::UserDocuments => {
            // 0040DFB2..0040DFCE.
            let bootstrap = working.join("Oblivion.ini");
            let use_my_games = use_my_games_from_bootstrap(&bootstrap);
            plan_startup_paths(&host_path_roots(working), use_my_games)
        }
    }
}

// IDA: 0053AC85 GetPrivateProfileIntA("DEFAULT", "VERSION", 0, "BlendSettings.ini"): a bare name, so the Windows directory, with UAC's VirtualStore copy winning (`blend_settings_version_file`). A pinned run's is in its folder, as `fixtures/config` holds it.
fn blend_version_file(source: &SettingsSource, paths: &GamePaths) -> PathBuf {
    match source {
        SettingsSource::Pinned(_) => paths.settings_directory.join("BlendSettings-version.ini"),
        SettingsSource::UserDocuments => {
            // `local_data_directory` is LocalAppData\Oblivion; its parent is LocalAppData. drive_c is six levels above it.
            let local = paths
                .local_data_directory
                .parent()
                .map_or_else(PathBuf::new, Path::to_path_buf);
            let windows = std::env::var_os("WINEPREFIX")
                .map(PathBuf::from)
                .or_else(|| {
                    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".wine-oblivion"))
                })
                .map_or_else(PathBuf::new, |prefix| prefix.join("drive_c/windows"));
            blend_settings_version_file(&windows, &local)
        }
    }
}

// ---- the chain ----

/// IDA: N/A WinMain's blocks, in original order. A variant is one exclusive system of the chain.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Block {
    /// The cinit before 0098787C reaches WinMain.
    StaticConstructors,
    /// Phases 01, 02: 0040DF94..0040E3B7.
    PathSetup,
    /// Phase 03: 0040E3BF..0040E6B8.
    MemoryAndFactories,
    /// Phase 03: 0040E6BE..0040E75E.
    SettingsAndArchives,
    /// Phase 04: 0040E763..0040E893.
    WindowAndRenderer,
    /// Phase 05: 0040E898..0040EAAC.
    SceneGraphs,
    /// Phases 05, 06: 0040EAB1..0040EB11.
    ActivationAndMovie,
    /// Phase 07: 0040EB16..0040EB18.
    Interface,
    /// Phase 08: 00406390..004065FF.
    Sky,
    /// Phase 09: 00406604..00406687.
    Tes,
    /// Phase 10: 00406687..00406773.
    Plugins,
    /// Phase 11: 00406774..004067B9.
    Player,
    /// Phase 12: 004067BA..004067C7. Resumable.
    Load,
    /// Phase 13: 004067CC..00406944, the end of 00406360.
    GameDataTail,
    /// Phase 13e: 00433BC0 as a host drain on the main thread. Resumable.
    ModelDrain,
    /// Phase 14: 0040EB31..0040EBB8.
    WorldSetup,
    /// Phase 15: 0040EBB9..0040EE79.
    InitialArea,
    /// Phase 16: 0040EE7A..0040EF23.
    PlacePlayer,
    /// Phase 17: 0040EF24..0040EFF2. The no-cell jump enters at 0040EF54.
    PlayerScene,
    /// Phase 18: 0040EFF3..0040F068.
    StartingSpell,
    /// Phase 19: 0040F069..0040F08C. The fill of the LoadingMenu's bar yields to the frame.
    CloseLoadingMenu,
    /// Phase 20: 0040F08D..0040F091.
    MainMenu,
    /// Phase 21: 0040F092..0040F0DF. The wait yields to the frame.
    IntroSequence,
    /// 0040F0E0 is next: Phase 22, the host's frames.
    Finished,
}

/// IDA: N/A how a block ended. It replaces `main.cpp`'s nesting (brace depth 14, 7 nested `try`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PhaseOutcome {
    /// The next block runs, now if the budget allows.
    Done,
    /// The block has more to do. It returns so Bevy can draw a frame, and runs again next frame.
    Yield,
    /// An unported arm: the rest of the blocks don't run, as `main.cpp`'s nested `try`s skip them, and it's reported. The app stays up.
    HostStop(String),
    /// WinMain itself exits here (0040E3AC's existing window, or 004052F0's ExitProcess), or a required host step failed. The process exits with this code.
    Exit(u8),
}

/// IDA: N/A a host stop a run reported.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostStopReport {
    pub block: Block,
    pub text: String,
}

/// IDA: N/A how far the driver is. A plain resource, so that the host can read it.
#[derive(Resource, Debug)]
pub struct WinMainProgress {
    pub next: Block,
    pub stops: Vec<HostStopReport>,
    /// The process should exit with this.
    pub exit_code: Option<u8>,
    /// Lines the run printed, in order, for a test that runs in process.
    pub lines: Vec<String>,
    pub started: Instant,
    /// How long each block ran in all, in order.
    pub block_times: Vec<(Block, Duration)>,
    /// When the current frame's budget ends.
    deadline: Option<Instant>,
    budget: Option<Duration>,
    ran_this_frame: bool,
    script_hosts_installed: bool,
}

impl WinMainProgress {
    /// IDA: N/A the driver has reached the end of what's ported, or stopped.
    #[must_use]
    pub fn finished(&self) -> bool {
        self.next == Block::Finished
    }

    // IDA: N/A a helper, with no address of its own.
    fn over_budget(&self) -> bool {
        self.deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
    }

    // IDA: N/A an accessor.
    // A frame's first block runs whatever the clock says, so that even a zero budget moves on.
    fn turn_is_over(&self) -> bool {
        self.ran_this_frame && self.over_budget()
    }
}

/// IDA: N/A the movie thread's device calls, for the movie agent: drained on the main thread, each frame.
#[derive(Resource)]
pub struct MovieDeviceQueue(pub Mutex<Receiver<MovieQueueEntry>>);

/// IDA: N/A 00B39B84, 00B39E00 and the other WinMain-owned statics that later code doesn't reach through a seam yet, held so that they outlive their block.
#[derive(Resource, Default)]
pub struct WinMainStatics {
    pub camera_aliases: CameraStaticAliases,
    pub renderer_callbacks: RendererCallbackPairs,
    /// 00B3F709.
    pub load_direct_hint: bool,
    pub ui_globals: RendererUiGlobals,
    /// 00B3A6B0.
    pub ui_state: RendererUiState,
    pub blend: BlendSettingsState,
    /// 00B35B84's last error.
    pub facegen_error: FaceGenLastError,
    /// Shader manager statics 007B7250 leaves, before Phase 05 writes them.
    pub shader_manager_statics: ShaderManagerStatics,
}

/// IDA: 00A10EB0 -> 007B7250 BSShaderManager's static constructor: the presets a reader sees if it runs before the store. Completion: the bytes 007B7306..007B7375's readable stores, as the exe leaves them.
///
/// TODO: [ ] A reader of B42E90 (007BD3D0, 007C9230, 007C9F30, 7DCC0B) takes this when its phase is ported.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShaderManagerStatics {
    pub b42f3f: u8,
    pub b42e90: u32,
    pub b42ea7: u8,
    pub b42f40: u32,
    pub b42d70: u32,
    pub b42e95: u8,
    pub b42e87: u8,
}

impl Default for ShaderManagerStatics {
    /// IDA: N/A no address of its own; its comments cite 007B72FF.
    fn default() -> Self {
        // IDA: 007B72FF, 007B7306, 007B7316, 007B7323, 007B732D, 007B7343, 007B736E.
        Self {
            b42f3f: 1,
            b42e90: 0xFFFF_FFFF,
            b42ea7: 1,
            b42f40: 0x2F,
            b42d70: 0x007B_4150,
            b42e95: 1,
            b42e87: 1,
        }
    }
}

impl WinMainLocals {
    /// IDA: N/A the stack NiPoint3 of 0040EB37..0040EB54, for G2's comparison of the driver's end with the rows.
    #[must_use]
    pub const fn rect(&self) -> [f32; 3] {
        self.rect
    }
}

/// IDA: N/A the installed process configuration and resolved game paths.
#[derive(Resource)]
struct GameProcess {
    config: WinMainConfig,
    paths: GamePaths,
    root: PathBuf,
}

/// IDA: N/A the I/O thread's independent CRT (98BFE5), lent to a model drain.
#[derive(Resource, Default)]
struct IoThreadRand(MsvcRand);

/// IDA: N/A the OS platform shared with the movie worker.
#[derive(Resource)]
struct OsPlatformHandle(Arc<Mutex<HostOsPlatform>>);

/// IDA: N/A the device and screen seams supplied by the host on the main thread.
struct PlatformSeams(WinMainSeams);

/// IDA: N/A the time storage shared during startup, before the first idle update publishes TimeInfoHandle.
#[derive(Resource, Default)]
struct StartupTime(SharedTimeInfo);

/// IDA: N/A the renderer's in-scene flag, also read by the movie worker.
#[derive(Resource, Default)]
struct InSceneFlag(Arc<AtomicBool>);

/// IDA: N/A the quit flag shared by the main-thread interface and host.
#[derive(Default)]
struct QuitFlag(Rc<Cell<bool>>);

/// IDA: N/A WinMain's stack locals and state lent between blocks on the main thread.
pub struct WinMainLocals {
    window: HostWindow,
    collections: SettingCollections,
    registry: NiFactoryRegistry,
    memory: StartupMemoryState,
    creation: Option<RendererCreation>,
    scene: Option<SceneWindow>,
    particles: ParticleControllerScheduleContext<'static>,
    prelude: SkyPreludeResult,
    load: Option<LoadSelectedFiles>,
    load_sections: VecDeque<(i32, f32)>,
    drain: Option<ModelDrain>,
    /// Phase 19, between frames.
    closing_menu: Option<LoadingMenuClosing>,
    /// `DriverInterfaceHost::seeded_fill_ticks`.
    seeded_fill_ticks: Rc<Cell<u32>>,
    setups_not_run: u32,
    disable_warning_messages_b06b18: u8,
    /// The stack NiPoint3 of 0040EB37..0040EB54, Phase 15's Rect.
    rect: [f32; 3],
    display_lod_land_b43077: u8,
    player: Option<Entity>,
    base: Option<Entity>,
    /// 00B333C4 +0x40 as 0040F069 reads it: Phase 16's cell, kept for the menu's branch.
    player_cell: Option<Entity>,
}

/// IDA: N/A sets up the driver on a world: the resources the chain reads. The app's window and `World` exist already, as they do for the original's process entry.
pub fn install_winmain(world: &mut World, config: WinMainConfig, seams: WinMainSeams) {
    let paths = selected_paths(&config.settings, &config.data_directory);
    let root = working_directory_of(&config.data_directory);
    let clock = HostClock::new(config.seed);
    let crt = MainThreadCrt::new(clock.clone());
    let budget = config.frame_budget;
    let files = SharedFiles::new(root.clone());
    let os_platform = Arc::new(Mutex::new(HostOsPlatform {
        failed_allocations: seams.failed_allocations.clone(),
        ..HostOsPlatform::default()
    }));
    world.insert_resource(GameProcess {
        config,
        paths,
        root,
    });
    world.insert_resource(clock);
    world.insert_resource(files);
    world.insert_resource(crt);
    world.insert_resource(IoThreadRand::default());
    world.insert_resource(OsPlatformHandle(os_platform));
    world.insert_non_send(PlatformSeams(seams));
    world.insert_resource(StartupTime::default());
    world.insert_resource(InterfaceRenderRequests::default());
    world.insert_resource(InSceneFlag::default());
    world.insert_non_send(QuitFlag::default());
    world.insert_non_send(WinMainLocals {
        window: HostWindow::default(),
        collections: SettingCollections::default(),
        registry: NiFactoryRegistry::default(),
        memory: StartupMemoryState::default(),
        creation: None,
        scene: None,
        particles: ParticleControllerScheduleContext::default(),
        prelude: SkyPreludeResult::default(),
        load: None,
        load_sections: VecDeque::new(),
        drain: None,
        closing_menu: None,
        seeded_fill_ticks: Rc::default(),
        setups_not_run: 0,
        disable_warning_messages_b06b18: 0,
        rect: [0.0; 3],
        display_lod_land_b43077: 0,
        player: None,
        base: None,
        player_cell: None,
    });
    world.insert_resource(WinMainProgress {
        next: Block::StaticConstructors,
        stops: Vec::new(),
        exit_code: None,
        lines: Vec::new(),
        started: Instant::now(),
        block_times: Vec::new(),
        deadline: None,
        budget,
        ran_this_frame: false,
        script_hosts_installed: false,
    });
}

/// IDA: N/A the chain, each block an exclusive system. Each runs only at its turn, and only while the frame's budget lasts, so the chain can run once per frame for as long as it takes.
#[must_use]
pub fn winmain_schedule() -> Schedule {
    use bevy_ecs::schedule::IntoScheduleConfigs;
    let mut schedule = Schedule::default();
    schedule.set_executor(bevy_ecs::schedule::SingleThreadedExecutor::new());
    schedule.add_systems(
        (
            start_frame,
            (
                |world: &mut World| run_block(world, Block::StaticConstructors),
                |world: &mut World| run_block(world, Block::PathSetup),
                |world: &mut World| run_block(world, Block::MemoryAndFactories),
                |world: &mut World| run_block(world, Block::SettingsAndArchives),
                |world: &mut World| run_block(world, Block::WindowAndRenderer),
                |world: &mut World| run_block(world, Block::SceneGraphs),
                |world: &mut World| run_block(world, Block::ActivationAndMovie),
                |world: &mut World| run_block(world, Block::Interface),
                |world: &mut World| run_block(world, Block::Sky),
            )
                .chain(),
            (
                |world: &mut World| run_block(world, Block::Tes),
                |world: &mut World| run_block(world, Block::Plugins),
                |world: &mut World| run_block(world, Block::Player),
                |world: &mut World| run_block(world, Block::Load),
                |world: &mut World| run_block(world, Block::GameDataTail),
                |world: &mut World| run_block(world, Block::ModelDrain),
                |world: &mut World| run_block(world, Block::WorldSetup),
                |world: &mut World| run_block(world, Block::InitialArea),
                |world: &mut World| run_block(world, Block::PlacePlayer),
                |world: &mut World| run_block(world, Block::PlayerScene),
                |world: &mut World| run_block(world, Block::StartingSpell),
                |world: &mut World| run_block(world, Block::CloseLoadingMenu),
                |world: &mut World| run_block(world, Block::MainMenu),
                |world: &mut World| run_block(world, Block::IntroSequence),
            )
                .chain(),
            end_frame,
        )
            .chain(),
    );
    schedule
}

/// IDA: N/A the frame's budget starts.
fn start_frame(world: &mut World) {
    let mut progress = world.resource_mut::<WinMainProgress>();
    progress.deadline = progress.budget.map(|budget| Instant::now() + budget);
    progress.ran_this_frame = false;
}

/// IDA: N/A what the frame's end does for the seams: the movie thread's queue, on the main thread.
fn end_frame(world: &mut World) {
    drain_movie_device_queue(world);
}

/// IDA: N/A the movie thread's queued device calls into `WinMainSeams::movie_device`, on the main thread. The host's frame runs it past the driver's end too.
pub fn drain_movie_device_queue(world: &mut World) {
    if world.get_non_send::<WinMainLocals>().is_none() {
        return;
    }
    let Some(mut seams) = world.remove_non_send::<PlatformSeams>() else {
        return;
    };
    if let Some(queue) = world.get_resource::<MovieDeviceQueue>() {
        let receiver = lock_shared(&queue.0);
        while let Ok(entry) = receiver.try_recv() {
            (seams.0.movie_device)(entry);
        }
    }
    world.insert_non_send(seams);
}

/// IDA: N/A the chain straight through, as a test does.
pub fn run_winmain_straight_through(world: &mut World) {
    let mut schedule = winmain_schedule();
    while !world.resource::<WinMainProgress>().finished() {
        schedule.run(world);
        if world.resource::<WinMainProgress>().exit_code.is_some() {
            break;
        }
    }
}

// IDA: N/A one block at its turn.
fn run_block(world: &mut World, block: Block) {
    {
        let progress = world.resource::<WinMainProgress>();
        if progress.next != block || progress.turn_is_over() || progress.exit_code.is_some() {
            return;
        }
    }
    let mut context = world
        .remove_non_send::<WinMainLocals>()
        .expect("install_winmain ran");
    let started = Instant::now();
    let outcome = match block {
        Block::StaticConstructors => static_constructors(world, &mut context),
        Block::PathSetup => path_setup(&world.resource::<GameProcess>().paths),
        Block::MemoryAndFactories => memory_and_factories(world, &mut context),
        Block::SettingsAndArchives => settings_and_archives(world, &mut context),
        Block::WindowAndRenderer => window_and_renderer(world, &mut context),
        Block::SceneGraphs => scene_graphs(world, &mut context),
        Block::ActivationAndMovie => activation_and_movie(world, &mut context),
        Block::Interface => interface(world, &mut context),
        Block::Sky => sky(world, &mut context),
        Block::Tes => tes(world, &mut context),
        Block::Plugins => plugins(world, &mut context),
        Block::Player => player(world, &mut context),
        Block::Load => load(world, &mut context),
        Block::GameDataTail => game_data_tail(world, &mut context),
        Block::ModelDrain => model_drain(world, &mut context),
        Block::WorldSetup => world_setup(world, &mut context),
        Block::InitialArea => initial_area(world, &mut context),
        Block::PlacePlayer => place_player(world, &mut context),
        Block::PlayerScene => player_scene(world, &mut context),
        Block::StartingSpell => starting_spell(world, &mut context),
        Block::CloseLoadingMenu => close_loading_menu(world, &mut context),
        Block::MainMenu => create_main_menu(world, &mut context),
        Block::IntroSequence => intro_sequence(world, &mut context),
        Block::Finished => Ok(PhaseOutcome::Done),
    };
    let outcome = outcome.unwrap_or_else(|error| PhaseOutcome::HostStop(error.message.to_owned()));
    let elapsed = started.elapsed();
    world.insert_non_send(context);
    let mut progress = world.resource_mut::<WinMainProgress>();
    progress.ran_this_frame = true;
    match progress.block_times.last_mut() {
        Some((last, time)) if *last == block => *time += elapsed,
        _ => progress.block_times.push((block, elapsed)),
    }
    match outcome {
        PhaseOutcome::Done => progress.next = successor(block),
        PhaseOutcome::Yield => {}
        PhaseOutcome::HostStop(text) => {
            // `main.cpp`'s `catch`es: the rest of WinMain's blocks are nested inside the one that stopped.
            let line = format!("{} host stop: {text}", phase_name(block));
            print_line(&mut progress, Level::Warning, &line);
            progress.stops.push(HostStopReport { block, text });
            progress.next = Block::Finished;
            if block == Block::Load {
                // main.cpp's Phase-12 catch: the TLS +0x184 loader byte is cleared (host substitution), so later TESForm constructions keep their arm.
                world.resource_mut::<GameThreadLocals>().byte_184 = 0;
            }
        }
        PhaseOutcome::Exit(code) => {
            progress.exit_code = Some(code);
            progress.next = Block::Finished;
        }
    }
}

// IDA: N/A a helper, with no address of its own.
const fn successor(block: Block) -> Block {
    match block {
        Block::StaticConstructors => Block::PathSetup,
        Block::PathSetup => Block::MemoryAndFactories,
        Block::MemoryAndFactories => Block::SettingsAndArchives,
        Block::SettingsAndArchives => Block::WindowAndRenderer,
        Block::WindowAndRenderer => Block::SceneGraphs,
        Block::SceneGraphs => Block::ActivationAndMovie,
        Block::ActivationAndMovie => Block::Interface,
        Block::Interface => Block::Sky,
        Block::Sky => Block::Tes,
        Block::Tes => Block::Plugins,
        Block::Plugins => Block::Player,
        Block::Player => Block::Load,
        Block::Load => Block::GameDataTail,
        Block::GameDataTail => Block::ModelDrain,
        Block::ModelDrain => Block::WorldSetup,
        Block::WorldSetup => Block::InitialArea,
        Block::InitialArea => Block::PlacePlayer,
        Block::PlacePlayer => Block::PlayerScene,
        Block::PlayerScene => Block::StartingSpell,
        Block::StartingSpell => Block::CloseLoadingMenu,
        Block::CloseLoadingMenu => Block::MainMenu,
        Block::MainMenu => Block::IntroSequence,
        Block::IntroSequence | Block::Finished => Block::Finished,
    }
}

// IDA: N/A a helper, with no address of its own.
const fn phase_name(block: Block) -> &'static str {
    match block {
        Block::StaticConstructors | Block::PathSetup => "Phase-01",
        Block::MemoryAndFactories | Block::SettingsAndArchives => "Phase-03",
        Block::WindowAndRenderer => "Phase-04",
        Block::SceneGraphs => "Phase-05",
        Block::ActivationAndMovie => "Phase-06",
        Block::Interface => "Phase-07",
        Block::Sky => "Phase-08",
        Block::Tes => "Phase-09",
        Block::Plugins => "Phase-10",
        Block::Player => "Phase-11",
        Block::Load => "Phase-12",
        Block::GameDataTail | Block::ModelDrain => "Phase-13",
        Block::WorldSetup => "Phase-14",
        Block::InitialArea => "Phase-15",
        Block::PlacePlayer => "Phase-16",
        Block::PlayerScene => "Phase-17",
        Block::StartingSpell => "Phase-18",
        Block::CloseLoadingMenu => "Phase-19",
        Block::MainMenu => "Phase-20",
        Block::IntroSequence | Block::Finished => "Phase-21",
    }
}

// IDA: N/A `main.cpp`'s logging: `[INFO] Phase-NN: ...` goes to stdout, where a test reads it, with the run's record.
fn print_line(progress: &mut WinMainProgress, level: Level, line: &str) {
    println!("[{}] {line}", level.name());
    progress.lines.push(line.to_owned());
}

// IDA: N/A `print_line` from a block.
fn phase_line(world: &mut World, line: String) {
    let mut progress = world.resource_mut::<WinMainProgress>();
    print_line(&mut progress, Level::Info, &line);
}

// IDA: 00404EE0 Startup_LogInfo (D4: the host log).
fn startup_log(text: &str) {
    emit(Level::Info, text);
}

// IDA: N/A `operator<<(float)`'s default format, `%g` with 6 significant digits, as `main.cpp` prints its floats.
fn cpp_float(value: f32) -> String {
    let text = oblivion_core::crt::gcvt(f64::from(value), 6);
    text.strip_suffix('.').map_or(text.clone(), str::to_owned)
}

// IDA: N/A a setting's raw word.
fn ini_raw(context: &WinMainLocals, address: u32) -> u32 {
    context
        .collections
        .ini
        .find_address(address)
        .map_or(0, |setting| setting.raw)
}

// ---- 0040DF80 and before ----

// IDA: the cinit that reach WinMain with game state: 004040E0's setting collections (00B07D28, 00B07BF0), 009FE5F0 g_actorProcessManager (00B3BD00, 0067AF60), the 00B33780.. GameSetting statics, 00B069C3 = 1 (the image), 00A10EB0 -> 007B7250 BSShaderManager's presets. 00B33508's EffectSettingCollection (cinit slot 151) stays at `begin_load_selected_files`, a recorded timing substitution: it needs the TLS arm, and nothing reads it earlier. TLS: the template's 0 at +0x184, +0x185, +0x186. Completion: 100% of what a later block reads.
fn static_constructors(
    world: &mut World,
    context: &mut WinMainLocals,
) -> Result<PhaseOutcome, GameError> {
    register_static_settings(&mut context.collections);
    world.insert_resource(GameSettingCollection::default());
    world.init_resource::<GameThreadLocals>();
    construct_actor_process_manager(world);
    world.init_resource::<WinMainStatics>();
    world.init_resource::<InventoryGlobals>();
    // 00A10EB0 -> 007B7250: its tail calls 007B70E0 with the float at 00A46B24, 0x42960000: 75.0.
    world.init_resource::<PlayerGlobals>();
    particle_update_fov_scale(
        f32::from_bits(0x4296_0000),
        &mut world.resource_mut::<PlayerGlobals>(),
    );
    emit(
        Level::Info,
        &format!(
            "Starting OblivionRecomp; data directory {}",
            world
                .resource::<GameProcess>()
                .config
                .data_directory
                .display()
        ),
    );
    Ok(PhaseOutcome::Done)
}

// IDA: 0040DF94..0040E3B7. 0040DFB2..0040E186 the paths (`selected_paths`, at install); 0040E18F..0040E1AB DeleteFileA on 00494480, 004944F0 and 00494560's three logs: not made, read-only; 0040E1B6 _access and 0040E221 CopyFileA: not made, read-only; 0040E227 00404940, whose result 0040E22E's jmp discards; 0040E2CB's test likewise; 0040E39B FindWindowA, 0040E3A6 SetForegroundWindow and the exit. 0040E3B7 0047F670: GetComputerNameA into 00B34190 and two logs ("%s" with the class name, "Computer Name: %s"), whose only readers are 0050A5FB and 00668591, a save's name: not modelled. Completion: 100% of what a later block reads.
fn path_setup(paths: &GamePaths) -> Result<PhaseOutcome, GameError> {
    let overflow = original_path_buffer_overflow(paths);
    if overflow.local_app_data || overflow.my_games || overflow.selected_ini {
        emit(
            Level::Warning,
            "A startup path would overflow the original's char[0x104]; the host keeps it whole",
        );
    }
    emit(
        Level::Info,
        &format!(
            "Selected Oblivion settings {}; saves {}",
            paths.selected_ini.display(),
            paths.saves_directory.display()
        ),
    );
    let _ = disc_check_passed();
    let mut windows = HostWindowPlatform::default();
    if activate_existing_window(ORIGINAL_WINDOW_CLASS, &mut windows).found {
        return Ok(PhaseOutcome::Exit(0));
    }
    Ok(PhaseOutcome::Done)
}

// IDA: 0040E3BF..0040E6B8 in order: 27 pools; 0040E620 the out-of-memory callback; 0040E62A 00747BA0 the callbacks; 0040E62F 00B3F958 = 1; 0040E636 006F98F0 the BS factories; 0040E63D 00B40334 BSSearchPath_Create; the listener; the FileFinder with "Data"; the palette. Completion: 100% of the order.
fn memory_and_factories(
    world: &mut World,
    context: &mut WinMainLocals,
) -> Result<PhaseOutcome, GameError> {
    let files = world.resource::<SharedFiles>().clone();
    context.memory.register_fixed_pools();
    context.memory.install_out_of_memory_policy();
    let mut known = StartupCallbackScheduler::default();
    register_known_crt_callbacks(&mut known)
        .map_err(|_| GameError::unclassified(0x00747BA0, "00747BA0"))?;
    known
        .run(world)
        .map_err(|_| GameError::unclassified(0x00747BA0, "00747BA0"))?;
    context.memory.initializers_dispatched();
    context.registry.register_known_crt_factories();
    context.registry.register_winmain_bs_factories();
    let _ = context.memory.allocate_listener();
    {
        let mut files = files.lock();
        let files = &mut *files;
        files.finder = context
            .memory
            .allocate_file_finder(&files.root, &mut files.callbacks);
        // 0040E673..0040E687: the finder's vt+8 with 00A31014 "Data".
        if let Some(finder) = files.finder.as_mut() {
            finder.add("Data");
        }
    }
    if context.memory.allocate_texture_palette(world) {
        files.lock().palette =
            world.remove_resource::<crate::bs::texture_palette::BsTexturePalette>();
    }
    Ok(PhaseOutcome::Done)
}

// IDA: 0040E6BE..0040E757 the registry and INI; 0040E759 0053AC60 BlendSettings; 0040E75E 0042F610 the archives, its result ignored. Completion: 100% of the order.
fn settings_and_archives(
    world: &mut World,
    context: &mut WinMainLocals,
) -> Result<PhaseOutcome, GameError> {
    let files = world.resource::<SharedFiles>().clone();
    let load = load_winmain_settings(
        &mut context.collections,
        &world.resource::<GameProcess>().paths.selected_ini,
    );
    emit(
        Level::Debug,
        &format!(
            "Setting collections: registry not opened, INI {}",
            if load.ini_opened { "read" } else { "not read" }
        ),
    );
    let process = world.resource::<GameProcess>();
    let version_file = blend_version_file(&process.config.settings, &process.paths);
    let settings_directory = process.paths.settings_directory.clone();
    {
        let mut statics = world.resource_mut::<WinMainStatics>();
        let blend = initialize_blend_settings(
            &mut context.collections,
            &settings_directory,
            &version_file,
            &mut statics.blend,
        );
        emit(
            Level::Debug,
            &format!("BlendSettings gate {}", blend.version_gate_passed),
        );
    }
    let mut files = files.lock();
    let files = &mut *files;
    let mut archives =
        ArchiveManager::new(&files.root, archive_settings_from(&context.collections.ini));
    let all_opened = files.finder.as_ref().is_some_and(|finder: &FileFinder| {
        archives.initialize_from_settings(
            finder,
            &world.resource::<GameProcess>().paths.local_data_directory,
        )
    });
    emit(
        Level::Info,
        &format!(
            "Archive manager: {} archives opened{}",
            archives.archives().len(),
            if all_opened {
                ""
            } else {
                " (some listed archives failed)"
            }
        ),
    );
    files.archives = Some(archives);
    Ok(PhaseOutcome::Done)
}

// IDA: 0040E763..0040E893 in order: the class and window; 0040E812..0040E831 FormHeap(0x28) 00404A00 -> 00B33398; 0040E836..0040E852 FormHeap(8) 00572DC0 -> 00B3A6B0; "Initializing Renderer..."; 0040E865 004052F0 -> 00498340; 0040E876 00B3F709 = 1; 0040E87D, 0040E88E the two callback pairs; 0040E893 00553550 FaceGen. A failed 00498340 shows its box and calls ExitProcess(0) inside 004052F0: the host prints and exits 0. Completion: 100% of the order.
fn window_and_renderer(
    world: &mut World,
    context: &mut WinMainLocals,
) -> Result<PhaseOutcome, GameError> {
    let time = world.resource::<StartupTime>().0.clone();
    let os_platform = world.resource::<OsPlatformHandle>().0.clone();
    let files = world.resource::<SharedFiles>().clone();
    const INSTANCE: HostInstance = HostInstance(0x0040_0000);
    let mut windows = HostWindowPlatform::default();
    let main = create_original_main_window(INSTANCE, &mut windows);
    context.window = main.window.unwrap_or_default();
    world.insert_resource(WindowProcedureState {
        time,
        ..WindowProcedureState::default()
    });
    let os = {
        let mut platform = lock_shared(&os_platform);
        platform.active_window = context.window;
        construct_os_globals(
            context.window,
            INSTANCE,
            &mut context.collections,
            &world.resource::<GameProcess>().paths.settings_directory,
            &mut *platform,
        )
    }
    .map_err(|error| {
        emit(Level::Error, &error.0);
        GameError::unclassified(0x00404A00, "00404A00")
    })?;
    if let Some(os) = os {
        world.insert_resource(os);
    }
    {
        let mut statics = world.resource_mut::<WinMainStatics>();
        let statics = &mut *statics;
        statics.ui_state = initialize_renderer_ui_state(&mut statics.ui_globals);
    }
    startup_log("Initializing Renderer...");
    let mut platform = PinnedRendererPlatform::new(DeviceCapsFixture::pinned());
    let inputs = RendererCreationInputs {
        main_window: Some(WindowHandle(context.window.0)),
        instance: INSTANCE.0,
        window_class: ORIGINAL_WINDOW_CLASS.to_owned(),
        my_games_root: world
            .resource::<GameProcess>()
            .paths
            .settings_directory
            .clone(),
        // g_dx9AdapterFormats, from 0040E62A's callbacks (006F98F0's registry).
        adapter_formats: world
            .get_resource::<NiD3DShaderCreatorRegistry>()
            .map_or_else(Vec::new, |registry| registry.adapter_formats().to_vec()),
        settings: renderer_settings_from(&context.collections.ini),
    };
    let mut creation = initialize_renderer(&inputs, &mut platform);
    if !creation.succeeded {
        emit(Level::Error, &creation.failure_message);
        return Ok(PhaseOutcome::Exit(0));
    }
    // 00768C10 -> 00764630 -> 007013A0 inside 0076A1C0: the default 2D pixel format replaces 00B3F718.
    if let Some(format) = creation.state.default_formats_854[0]
        .and_then(|index| creation.state.pixel_format_tables[0][index].as_ref())
    {
        let words = pixel_format_words(format);
        world
            .get_resource_or_init::<SingletonTail>()
            .set_default_pixel_format(&words);
    }
    // 00498614..00498647 inside 00498340: 0071B290 makes the NiDevImageConverter the default.
    if creation.device_image_converter_default {
        set_device_image_converter_default(world);
    }
    // 00B34FA8 g_rendererDeviceWindow and 00B350D8 g_dx9Renderer, as 004060F0 reads them.
    {
        let mut procedure = world.resource_mut::<WindowProcedureState>();
        procedure.device_window = creation.child_window.map(|window| HostWindow(window.0));
        procedure.renderer_present = true;
    }
    emit(
        Level::Info,
        &format!(
            "Renderer created: {}x{}, render path {}",
            creation.globals.width, creation.globals.height, creation.globals.render_path
        ),
    );
    {
        let mut statics = world.resource_mut::<WinMainStatics>();
        let statics = &mut *statics;
        register_winmain_renderer_callbacks(
            &mut statics.renderer_callbacks,
            &mut statics.load_direct_hint,
        );
    }
    // 0040E893 00553550.
    let mut singleton = FaceGenSingleton::default();
    {
        let mut statics = world.resource_mut::<WinMainStatics>();
        let mut files = files.lock();
        let files = &mut *files;
        if let Some(finder) = files.finder.as_ref() {
            let (mipmap_by_default, converter) = (
                creation.globals.texture_mipmap_by_default,
                creation.device_image_converter_default,
            );
            let mut create = |request: &crate::tes::facegen::FaceGenTextureRequest<'_>| {
                renderer_facegen_upload(&mut creation, request)
            };
            ensure_facegen_initialized(
                &mut singleton,
                &mut statics.facegen_error,
                FaceGenInputs {
                    finder,
                    archives: files.archives.as_mut(),
                    create_texture: Some(&mut create),
                    texture_prefs: [6, 3, 2],
                    mipmap_by_default,
                    device_image_converter: converter,
                    object_load_callbacks_enabled: context.memory.object_load_callbacks_enabled(),
                },
            );
        }
    }
    world.insert_resource(singleton);
    context.creation = Some(creation);
    Ok(PhaseOutcome::Done)
}

// IDA: 0040E898..0040EAAC: the shader window and 00406950, with its steps bound in `run_scene_window`. 0040EAB1..0040EAC1's flush runs in the next block's `replay_startup_activation`, so `os_input` is `None` here: the original has one 00403160. Completion: 100% of the order.
fn scene_graphs(world: &mut World, context: &mut WinMainLocals) -> Result<PhaseOutcome, GameError> {
    let os_platform = world.resource::<OsPlatformHandle>().0.clone();
    let files = world.resource::<SharedFiles>().clone();
    // 0040E898..0040E89D.
    startup_log("Initializing Shader System...");
    let creation = context
        .creation
        .as_mut()
        .ok_or(GameError::unclassified(0x0040E898, "0040E898: no renderer"))?;
    let hooks = files.texture_hooks();
    let package = files.clone();
    let program = files.clone();
    let time = files.clone();
    let load_direct_hint = world.resource::<WinMainStatics>().load_direct_hint;
    let mut runtime = SceneControllers {
        particles: &mut context.particles,
        tile_textures: None,
    };
    let mut platform = lock_shared(&os_platform);
    let window = run_scene_window(
        world,
        SceneWindowServices {
            ini: &context.collections.ini,
            creation,
            load_direct_hint,
            drop_static_app_data: context.memory.object_load_callbacks_enabled(),
            texture_palette: files.has_palette(),
            file_finder: files.has_finder(),
            open_texture_file: hooks.open_file,
            texture_file_exists: hooks.file_exists,
            palette_find: Some(hooks.palette_find),
            palette_add: Some(hooks.palette_add),
            // 00431690.
            open_package: Box::new(move |path| package.open_as_bs_file(path, 0, 0, 0xFFFF)),
            // 00431130(path, 0, 10240, 0x20).
            open_program: Some(Box::new(move |path| {
                program.open_as_bs_file(path, 0, 10240, 0x20)
            })),
            // 004315F0(path, &time, 0x20): the packed archive's file, or the loose path. No shipped program is outside the package, so no time is compared (P05's rows).
            program_file_time: Some(Box::new(move |path| {
                let files = time.lock();
                let target = files.root.join(path.replace('\\', "/"));
                let modified = std::fs::metadata(target)
                    .and_then(|meta| meta.modified())
                    .ok()?;
                let since = modified.duration_since(std::time::UNIX_EPOCH).ok()?;
                // FILETIME: 100 ns ticks from 1601.
                Some(since.as_secs() * 10_000_000 + 116_444_736_000_000_000)
            })),
            shader_cache_directory: String::new(),
            os_input: None,
            os_platform: &mut *platform,
            runtime: &mut runtime,
        },
    )?;
    drop(platform);
    for warning in &window.warnings {
        emit(
            Level::Warning,
            &warning
                .iter()
                .map(|&byte| char::from(byte))
                .collect::<String>(),
        );
    }
    // IDA: N/A `main.cpp`'s debug lines for the P05-RUN-002 comparison.
    let debug = |text: String| emit(Level::Debug, &text);
    debug(format!(
        "Phase-05 effect textures: {} slots stored; texture bytes {}, cube bytes {}",
        window.effect_textures.store_trace.len(),
        window.texture_statistics.texture_bytes,
        window.texture_statistics.cube_bytes
    ));
    debug(format!(
        "Phase-05 shader slots: {} created",
        window.shader_slots.creation_order.len()
    ));
    debug(format!(
        "Phase-05 image-space list: {}",
        window.image_space_list.is_some()
    ));
    debug(format!(
        "Phase-05 render targets: bytes {}/{}",
        window.render_targets.texture_bytes_b42064, window.render_targets.texture_padding_b42068
    ));
    debug(format!(
        "Phase-05 test sphere: guard {:?}",
        window.test_sphere
    ));
    debug(format!(
        "Phase-05 shader manager: package {}, {} injected steps pending",
        if window.shader_manager.package.is_some() {
            "kept"
        } else {
            "released"
        },
        window.manager_result.pending_steps.len()
    ));
    debug(format!(
        "Phase-05 scene graphs: {} global stores, {} injected steps pending",
        window.scene_graphs.store_trace.len(),
        window.scene_result.pending_steps.len()
    ));
    world.insert_resource(window.scene_graphs.clone());
    context.scene = Some(window);
    Ok(PhaseOutcome::Done)
}

// IDA: 0040EAB1..0040EB11 in order: esi = g_osGlobals; with an input, 00403160 and 004046A0; 0047D0F0; +2 = 1; 0040EADB 00410310; 0040EAEA 00410E40(00B03084, 0, -1); the camera aliases; 0067FCF0; 00683030. Completion: 100% of the order.
fn activation_and_movie(
    world: &mut World,
    context: &mut WinMainLocals,
) -> Result<PhaseOutcome, GameError> {
    let time = world.resource::<StartupTime>().0.clone();
    let clock = world.resource::<HostClock>().clone();
    let os_platform = world.resource::<OsPlatformHandle>().0.clone();
    let creation = context
        .creation
        .as_ref()
        .ok_or(GameError::unclassified(0x0040EAB1, "0040EAB1: no renderer"))?;
    // `InterfaceMovieReads`: the clone the thread holds, before Phase 07 publishes through it.
    let reads = InterfaceMovieReads::default();
    world.insert_resource(reads.clone());
    let replay = {
        let mut os = world
            .get_resource_mut::<OsGlobalsData>()
            .ok_or(GameError::unclassified(
                0x0040EAB7,
                "0040EAB7: a null g_osGlobals",
            ))?;
        let mut platform = lock_shared(&os_platform);
        replay_startup_activation(&mut os, &time, clock.tick(), &mut *platform)
    };
    emit(
        Level::Debug,
        &format!(
            "Phase-05 shader branch: {}; input flush {}; Phase-06 activation: time count {}, window active {}",
            if creation.globals.hdr { "HDR" } else { "blur" },
            if replay.input { "ran" } else { "skipped" },
            replay.time_count,
            replay.window_active
        ),
    );
    let (sender, receiver) = channel();
    world.insert_resource(MovieDeviceQueue(Mutex::new(receiver)));
    let device: Arc<Mutex<dyn MovieDevice>> = Arc::new(Mutex::new(
        QueuedMovieDevice::new(sender)
            .with_splash_loader(world.non_send::<PlatformSeams>().0.splash_loader.clone()),
    ));
    let mut platform = HostMoviePlatform::default();
    platform.in_scene = Arc::clone(&world.resource::<InSceneFlag>().0);
    let platform: Arc<Mutex<dyn MoviePlatform>> = Arc::new(Mutex::new(platform));
    let os_platform: Arc<Mutex<dyn OsGlobalsPlatform + Send>> = os_platform.clone();
    let os_globals = world.resource::<OsGlobalsData>();
    let player = MoviePlayer::new(MoviePlayerServices {
        bink: Arc::clone(&world.non_send::<PlatformSeams>().0.bink),
        device: Some(device),
        platform,
        os_platform,
        renderer_width: creation.globals.width as i32,
        renderer_height: creation.globals.height as i32,
        os_globals: Some(MovieOsGlobals::from(os_globals)),
        time: time.clone(),
        interface: reads,
        video_directory: world
            .resource::<GameProcess>()
            .config
            .data_directory
            .join("Video"),
        settings: movie_settings_from(&context.collections.ini),
    });
    let scenes = world.resource::<SceneGraphGlobals>().clone();
    let mut aliases = CameraStaticAliases::default();
    let mut pathing = PathingSingletons::default();
    let mut pathing_platform = HostPathingPlatform::default();
    let result = run_winmain_movie_window(
        &player,
        || publish_camera_aliases(world, &scenes, &mut aliases),
        &mut pathing,
        &mut pathing_platform,
    );
    let thread = lock_shared(&player.globals.host_thread_owner).is_some();
    let intro =
        String::from_utf8_lossy(&player.services.settings.intro_sequence_b03084).into_owned();
    phase_line(
        world,
        format!(
            "Phase-06: Bink sound {}, intro thread {} ({intro}), camera aliases {}, pathing map {}, PathBuilder {}",
            result.bink_sound,
            if thread { "started" } else { "absent" },
            if aliases.face_gen_node_camera.is_some() {
                "set"
            } else {
                "empty"
            },
            if pathing.world_node_map_b3be00.is_some() {
                "ready"
            } else {
                "null"
            },
            if pathing.path_builder_b3bf80.is_some() {
                "ready"
            } else {
                "null"
            },
        ),
    );
    world.resource_mut::<WinMainStatics>().camera_aliases = aliases;
    world.insert_resource(pathing);
    world.insert_resource(player);
    Ok(PhaseOutcome::Done)
}

// IDA: N/A `InterfaceSettings` for this moment, each present flag from its owner: 00B333C4, 00B33A98, g_osGlobals +0x24, 00B333A0.
fn interface_settings(world: &World, context: &WinMainLocals) -> InterfaceSettings {
    let creation = context.creation.as_ref();
    let (width, height) = creation.map_or((0, 0), |creation| {
        (creation.globals.width, creation.globals.height)
    });
    let os = world.get_resource::<OsGlobalsData>();
    // 0057E7C0: OSInputGlobals +0 & 8.
    let input_flags = os
        .and_then(|os| os.input_20.as_ref())
        .map_or(0, |input| input.lock().flags_0000 as u8);
    let tes_present = world
        .get_resource::<TesGlobals>()
        .is_some_and(|globals| globals.tes.is_some());
    let scene =
        scene_graph_settings_from(&context.collections.ini, width, height, false, tes_present);
    let mut settings = interface_settings_from(
        &context.collections.ini,
        width as i32,
        height as i32,
        input_flags,
        scene,
    );
    settings.player_present = world
        .get_resource::<PlayerGlobals>()
        .is_some_and(|globals| globals.the_player_b333c4.is_some());
    settings.data_handler_present = world.contains_resource::<DataHandler>();
    settings.sound_present = os.is_some_and(|os| os.sound_24.is_some());
    settings
}

// IDA: N/A the interface's host for this moment.
fn interface_host(world: &World, context: &WinMainLocals) -> DriverInterfaceHost {
    let process = world.resource::<GameProcess>();
    let os = world.get_resource::<OsGlobalsData>();
    DriverInterfaceHost {
        files: world.resource::<SharedFiles>().clone(),
        crt: world.resource::<MainThreadCrt>().clone(),
        clock: world.resource::<HostClock>().clone(),
        movie: world.get_resource::<MoviePlayer>().cloned(),
        input: os.and_then(|os| os.input_20.clone()),
        sound: os.and_then(|os| {
            os.sound_24
                .as_ref()
                .map(|sound| sound.game_sounds_paused_a4.clone())
        }),
        sound_enabled: ini_raw(context, 0x00B1_6178) as u8 != 0,
        renders: world.resource::<InterfaceRenderRequests>().clone(),
        in_scene: Arc::clone(&world.resource::<InSceneFlag>().0),
        loading_updates: 0,
        screens: world.non_send::<PlatformSeams>().0.screens.clone(),
        saves_directory: process.paths.saves_directory.clone(),
        quit: world.non_send::<QuitFlag>().0.clone(),
        seeded_fill_ticks: (process.config.seed.is_some() && context.closing_menu.is_some())
            .then(|| context.seeded_fill_ticks.clone()),
        seeded_close_ticks: process
            .config
            .seed
            .is_some()
            .then(|| context.seeded_fill_ticks.clone()),
        fill_active: false,
        menu_media: Some(menu_media_of(world, context)),
    }
}

// IDA: N/A 00B3F958 and 00B3F709, as `load_host`'s other readers take them.
fn tile_texture_gates(world: &World, context: &WinMainLocals) -> (bool, bool) {
    (
        context.memory.object_load_callbacks_enabled(),
        world.resource::<WinMainStatics>().load_direct_hint,
    )
}

// IDA: N/A 00591360's targets for a frame, from the context's parts: a frame before the renderer, or a test with no scene, keeps the seam.
fn tile_textures<'a>(
    files: &'a SharedFiles,
    creation: Option<&'a mut RendererCreation>,
    scene: Option<&'a mut SceneWindow>,
    gates: (bool, bool),
) -> Option<DriverTileTextures<'a>> {
    Some(DriverTileTextures {
        files,
        creation: creation?,
        statistics: &mut scene?.texture_statistics,
        gates,
    })
}

// IDA: 0040EB16..0040EB18 00578CC0 -> 00582160(1, 1). Completion: 100% of the call.
fn interface(world: &mut World, context: &mut WinMainLocals) -> Result<PhaseOutcome, GameError> {
    let files = world.resource::<SharedFiles>().clone();
    let settings = interface_settings(world, context);
    let mut host = interface_host(world, context);
    let gates = tile_texture_gates(world, context);
    let mut textures = tile_textures(
        &files,
        context.creation.as_mut(),
        context.scene.as_mut(),
        gates,
    );
    let mut runtime = SceneControllers {
        particles: &mut context.particles,
        tile_textures: textures
            .as_mut()
            .map(|textures| textures as &mut dyn TileTextureLoader),
    };
    run_winmain_interface_window(world, &settings, &mut host, &mut runtime)?;
    // 00578E00's calls from 0058E92E, which the seam holds: it has no World.
    for _ in 0..host.loading_updates {
        with_interface_cx(world, &settings, &mut host, &mut runtime, |cx| {
            loading_menu_update_display(cx, false)
        })?;
    }
    let manager = world.get_resource::<InterfaceManager>();
    let camera = manager.map_or(0.0, |manager| manager.camera_distance_074);
    let published = manager.is_some();
    let menu = world
        .get_resource::<TileEngine>()
        .and_then(|engine| engine.tile_menu_by_id(0x3EF))
        .is_some();
    phase_line(
        world,
        format!(
            "Phase-07: InterfaceManager {}, camera {}, LoadingMenu {}, view {}",
            if published { "published" } else { "absent" },
            cpp_float(camera),
            if menu { "created" } else { "absent" },
            // The view is the screens agent's: host-owned, and not adopted yet.
            "pending"
        ),
    );
    Ok(PhaseOutcome::Done)
}

// IDA: N/A 007B4290 over the real Phase 05 slots: each slot was made at 007BA254.., so this finds it; a gated one comes back null.
struct RealShaders<'a> {
    window: &'a mut SceneWindow,
}

impl RealShaders<'_> {
    // IDA: N/A a helper, with no address of its own.
    fn get(&mut self, index: u32) -> Option<Arc<BsShaderSlot>> {
        get_bs_shader_slot(
            &mut self.window.shader_slots,
            index,
            &mut self.window.shader_context,
        )
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "each is a global 006F9980 and its callees read in the original"
)]
// IDA: 006F9980 through the 00B27E4C open slot over the real files, on the main thread: `_rand` is that thread's (the stream link's 0074FF37).
fn load_main_thread_stream(
    world: &mut World,
    path: &str,
    files: &SharedFiles,
    crt: &MainThreadCrt,
    registry: &NiFactoryRegistry,
    creation: &std::cell::RefCell<&mut RendererCreation>,
    statistics: &std::cell::RefCell<&mut crate::gamebryo::texture_loading::TextureMemoryStatistics>,
    gates: (bool, bool),
) -> Result<BsStreamLoad, GameError> {
    let open_files = files.clone();
    let mut open = move |path: &str, mode: i32, size: u32| {
        open_files
            .open_core(path, mode, size, 0xFFFF)
            .map(crate::bs::file::OpenedFile::into_game_file)
    };
    let crt = crt.clone();
    let mut rand = move || crt.rand();
    let mut textures = DriverSourceTextures {
        files: files.clone(),
    };
    let (precache, _) = {
        let creation = creation.borrow();
        (creation.globals.texture_create_renderer_data, ())
    };
    let hook_files = files.clone();
    let mut create = |world: &mut World, texture: &mut crate::gamebryo::texture::SourceTexture| {
        let mut creation = creation.borrow_mut();
        let mut statistics = statistics.borrow_mut();
        let mut services = texture_services(
            &hook_files,
            &mut creation,
            gates.0,
            gates.1,
            &mut statistics,
        );
        create_file_renderer_data(world, texture, &mut services)
    };
    load_bs_stream_from_path(
        world,
        path,
        &mut BsStreamServices {
            open: &mut open,
            registry,
            draw_rand: Some(&mut rand),
            draw_on_io_thread: false,
            source_textures: Some(&mut textures),
            post_load: StreamPostLoadRuntime {
                // 007203F0: 00B256CC (00498340) and 00B3F958 (0040E62F, 00441D1B).
                precache_source_textures: precache,
                release_static_texture_pixel_data: gates.0,
                create_source_texture_renderer_data: Some(&mut create),
                ..StreamPostLoadRuntime::default()
            },
        },
    )
}

// IDA: 00406390..004065FF, 0040EB1D..0040EB2C's arguments: g_worldSceneGraph, g_osGlobals. Completion: 100% of the block; `run_sky_prelude`.
fn sky(world: &mut World, context: &mut WinMainLocals) -> Result<PhaseOutcome, GameError> {
    let crt = world.resource::<MainThreadCrt>().clone();
    let files = world.resource::<SharedFiles>().clone();
    let defaults = property_defaults(world);
    let scenes = world.resource::<SceneGraphGlobals>().clone();
    let raw = |address: u32| ini_raw(context, address);
    let (precipitation, sun_base_size, sun_glare_size) = (
        raw(0x00B1_1DE4) & 0xFF != 0,
        f32::from_bits(raw(0x00B1_1E2C)),
        f32::from_bits(raw(0x00B1_1E34)),
    );
    let gates = (
        context.memory.object_load_callbacks_enabled(),
        world.resource::<WinMainStatics>().load_direct_hint,
    );
    let WinMainLocals {
        creation,
        scene,
        registry,
        particles,
        ..
    } = context;
    let creation = creation
        .as_mut()
        .ok_or(GameError::unclassified(0x00406390, "00406390: no renderer"))?;
    let window = scene.as_mut().ok_or(GameError::unclassified(
        0x00406390,
        "00406390: no scene window",
    ))?;
    let (hdr, render_path, shaders_30) = (
        creation.globals.hdr,
        creation.globals.render_path,
        creation.globals.shaders_30,
    );
    let mut statistics_owned = std::mem::take(&mut window.texture_statistics);
    let creation_cell = std::cell::RefCell::new(creation);
    let statistics = std::cell::RefCell::new(&mut statistics_owned);
    let reference_classifier = window.scene_graphs.reference_classifier;
    let mut shaders = RealShaders { window };
    let mut get_shader = |index: u32| shaders.get(index);
    let mut prepare = PrepareGeometryContext {
        render_path,
        hdr,
        reference_classifier,
        property_defaults: defaults,
        get_shader: &mut get_shader,
        warnings: Vec::new(),
        sky_shader_trace: Vec::new(),
        water_shader_trace: Vec::new(),
        lava_b47844: 0,
        shaders_30_b42ea5: shaders_30,
        load_normal_map: None,
        unported_shader_setup: None,
        classify_trace: Vec::new(),
    };
    let mut load = |world: &mut World, path: &str| {
        load_main_thread_stream(
            world,
            path,
            &files,
            &crt,
            registry,
            &creation_cell,
            &statistics,
            gates,
        )
    };
    let mut init = SkyInitServices {
        renderer_hdr: hdr,
        precipitation,
        sun_base_size,
        sun_glare_size,
        world_scene: scenes.world,
        weather: scenes.weather,
        load_stream: &mut load,
        warnings: Vec::new(),
    };
    let mut runtime = SceneControllers {
        particles,
        tile_textures: None,
    };
    let result = run_sky_prelude(
        world,
        SkyPreludeInputs {
            scenes: &scenes,
            init: &mut init,
            prepare: &mut prepare,
            property_defaults: defaults,
            update: &mut runtime,
            log: &mut |text| startup_log(text),
        },
    );
    let sky_warnings = std::mem::take(&mut init.warnings);
    let sky_geometries = prepare.sky_shader_trace.len();
    drop(init);
    drop(prepare);
    context_statistics(context, statistics_owned);
    for warning in sky_warnings {
        emit(Level::Warning, warning);
    }
    let prelude = result?;
    context.prelude = prelude;
    // IDA: N/A `main.cpp`'s line: the process counters P08-RUN-002 read.
    let line = phase_08_line(world, &prelude, sky_geometries);
    phase_line(world, line);
    Ok(PhaseOutcome::Done)
}

// IDA: N/A hands the counters back to the scene window.
fn context_statistics(
    context: &mut WinMainLocals,
    statistics: crate::gamebryo::texture_loading::TextureMemoryStatistics,
) {
    if let Some(window) = context.scene.as_mut() {
        window.texture_statistics = statistics;
    }
}

// IDA: N/A `Phase-08:`.
fn phase_08_line(world: &World, prelude: &SkyPreludeResult, sky_geometries: usize) -> String {
    let globals = world.get_resource::<SkyGlobals>();
    let sky = globals.and_then(|globals| globals.singleton_b365c4.as_deref());
    let sun = sky.and_then(|sky| sky.sun_28.as_ref());
    let light_uid = sun
        .and_then(|sun| sun.light_1c)
        .and_then(|light| world.get::<crate::gamebryo::scene_derived::DirectionalLight>(light))
        .map_or(0, |light| light.uid_b0);
    let data_id = sun
        .and_then(|sun| sun.geometry_10)
        .and_then(|geometry| world.get::<crate::gamebryo::mesh_geometry::NiGeometry>(geometry))
        .and_then(|geometry| geometry.data_b4)
        .and_then(|data| world.get::<crate::gamebryo::mesh_geometry::MeshTriShapeData>(data))
        .map_or(0, |data| data.base.data_id_0a);
    let material = sky
        .and_then(|sky| sky.clouds_2c.as_ref())
        .and_then(|clouds| clouds.layers_08[0])
        .and_then(|layer| {
            crate::gamebryo::av_object::find_property_of_type(world, layer, 2)
                .ok()
                .flatten()
        })
        .and_then(|material| world.get::<crate::gamebryo::properties::MaterialProperty>(material))
        .map_or(0, |material| material.material_id_18);
    format!(
        "Phase-08: Sky {}, SkyShader geometries {sky_geometries}, g_lodRoot arm {}, land LOD {}, sun light UID {light_uid}, sun data ID {data_id}, cloud material ID {material}",
        if sky.is_some() {
            "initialized"
        } else {
            "absent"
        },
        if prelude.lod_branch {
            "taken"
        } else {
            "skipped"
        },
        if prelude.land_lod.is_some() {
            "set"
        } else {
            "null"
        },
    )
}

// IDA: 00406604..00406687. Completion: 100% of the block; `run_tes_block`.
fn tes(world: &mut World, context: &mut WinMainLocals) -> Result<PhaseOutcome, GameError> {
    let files = world.resource::<SharedFiles>().clone();
    let defaults = property_defaults(world);
    let scenes = world.resource::<SceneGraphGlobals>().clone();
    let working = world.resource::<GameProcess>().root.clone();
    let (havok_settings, debris_max_velocity, debris_min_extent, step_speed) = tes_setting_inputs(
        world.resource::<GameSettingCollection>(),
        &context.collections.ini,
        working.as_os_str().as_encoded_bytes().to_vec(),
    )?;
    let gates = (
        context.memory.object_load_callbacks_enabled(),
        world.resource::<WinMainStatics>().load_direct_hint,
    );
    let WinMainLocals {
        creation,
        scene,
        collections,
        registry,
        memory,
        prelude,
        particles,
        ..
    } = context;
    let creation = creation
        .as_mut()
        .ok_or(GameError::unclassified(0x00406604, "00406604: no renderer"))?;
    let window = scene.as_mut().ok_or(GameError::unclassified(
        0x00406604,
        "00406604: no scene window",
    ))?;
    let (hdr, render_path, shaders_30) = (
        creation.globals.hdr,
        creation.globals.render_path,
        creation.globals.shaders_30,
    );
    let image_space_effects = window.shader_system.image_space_effects;
    let reference_classifier = window.scene_graphs.reference_classifier;
    let mut shader_globals: BsShaderGlobals = std::mem::take(&mut window.shader_context.globals);
    // 0044295A compares with g_IOManager +0x30, the IO thread's ID, which doesn't exist until 0040662D, and the main thread is never it: it takes the pause arm. A finished thread's ID stands for "not this one", the only thing the compare asks here.
    let io_thread = std::thread::spawn(|| std::thread::current().id())
        .join()
        .map_err(|_| GameError::unclassified(0x00442956, "00442956: no thread ID"))?;
    let mut runtime = SceneControllers {
        particles,
        tile_textures: None,
    };
    let mut warnings = Vec::new();
    let mut warning = |text: String| warnings.push(text);
    let water_shaders = {
        let mut shaders = RealShaders { window };
        let mut get_shader = |index: u32| shaders.get(index);
        let mut prepare = PrepareGeometryContext {
            render_path,
            hdr,
            reference_classifier,
            property_defaults: defaults,
            get_shader: &mut get_shader,
            warnings: Vec::new(),
            sky_shader_trace: Vec::new(),
            water_shader_trace: Vec::new(),
            lava_b47844: 0,
            shaders_30_b42ea5: shaders_30,
            load_normal_map: None,
            unported_shader_setup: None,
            classify_trace: Vec::new(),
        };
        let mut list_warnings = Vec::new();
        let mut construct = |world: &mut World| construct_data_handler(world);
        let mut build = |world: &mut World, path: &str| {
            build_data_handler_file_list(world, path, &working, &mut |text| {
                list_warnings.push(text);
            })
        };
        let mut gate = |_: &mut World| memory.enable_object_load_callbacks();
        let files_for_textures = files.clone();
        let mut statistics = crate::gamebryo::texture_loading::TextureMemoryStatistics::default();
        let mut water_textures = DriverWaterTextures {
            files: files_for_textures,
            creation,
            drop_static_app_data: gates.0,
            direct_to_renderer_hint: gates.1,
            statistics: &mut statistics,
            io_thread,
        };
        let result = run_tes_block(
            world,
            prelude,
            TesBlockInputs {
                ini: &mut collections.ini,
                construction: TesConstructionInputs {
                    registry,
                    havok_settings,
                    data_path: "Data\\",
                    scene_root: None,
                    land_lod: None,
                    debris_max_velocity,
                    debris_min_extent,
                    step_speed,
                    construct_data_handler: &mut construct,
                    build_file_list: &mut build,
                    enable_object_load_callbacks: &mut gate,
                    fail_tes_allocation: false,
                },
                water: WaterServices {
                    shader: Some(&mut shader_globals),
                    image_space_effects,
                    render_path,
                    prepare: Some(&mut prepare),
                    textures: Some(&mut water_textures),
                    warning: &mut warning,
                },
                world_scene: scenes.world,
                property_defaults: defaults,
                update: &mut runtime,
                log: &mut |text| startup_log(text),
            },
        );
        for text in list_warnings {
            emit(Level::Warning, &text);
        }
        result?;
        prepare.water_shader_trace.len()
    };
    let window = context.scene.as_mut().expect("just used");
    window.shader_context.globals = shader_globals;
    for text in warnings {
        emit(Level::Warning, &text);
    }
    let plugins = world.resource::<DataHandler>().files_8c8.len();
    let tes = world
        .get_resource::<TesGlobals>()
        .is_some_and(|globals| globals.tes.is_some());
    let trees = world
        .get_resource::<crate::bs::tree_manager::TreeManagerGlobals>()
        .is_some_and(|globals| globals.tree_manager_b39e04.is_some());
    phase_line(
        world,
        format!(
            "Phase-09: TES {}, plugins {plugins}, water geometries prepared {water_shaders}, TreeManager {}",
            if tes { "published" } else { "absent" },
            if trees { "published" } else { "absent" },
        ),
    );
    Ok(PhaseOutcome::Done)
}

// IDA: 00406687..00406773. Completion: 100% of the block.
fn plugins(world: &mut World, context: &mut WinMainLocals) -> Result<PhaseOutcome, GameError> {
    let paths = world.resource::<GameProcess>().paths.clone();
    let selection = run_plugin_block(
        world,
        PluginBlockInputs {
            ini: &context.collections.ini,
            paths: &paths,
            warning: &mut |text| emit(Level::Warning, &text),
        },
    )?;
    let data = world.resource::<DataHandler>();
    let total = data.files_8c8.len();
    let loaded = data
        .files_8c8
        .iter()
        .filter(|&&id| data.file_store.get(id).is_some_and(|file| file.is_loaded()))
        .count();
    phase_line(
        world,
        format!(
            "Phase-10: plugins loaded {loaded} of {total}, Plugins.txt {} ({} of {} lines matched), test files {}, fallback {}",
            if selection.list.opened {
                "read"
            } else {
                "absent"
            },
            selection.list.matched,
            selection.list.lines,
            if selection.test_file_selected {
                "selected"
            } else {
                "none"
            },
            if selection.fallback_taken {
                "taken"
            } else {
                "not taken"
            },
        ),
    );
    Ok(PhaseOutcome::Done)
}

// IDA: 00406774..004067B9. 0066AD96's GetTickCount, 0066AEED's _time64 and 0067B3C4's 0047DF80 go through the one clock and CRT. LowProcess +0x88 is FormHeap garbage in the original: 0 here, a recorded substitution, and P11-RUN-001's value stays in its test. Completion: 100% of the block.
fn player(world: &mut World, context: &mut WinMainLocals) -> Result<PhaseOutcome, GameError> {
    let clock = world.resource::<HostClock>().clone();
    let crt = world.resource::<MainThreadCrt>().clone();
    let settings = player_setting_inputs(
        world.resource::<GameSettingCollection>(),
        &context.collections.ini,
    )?;
    let creation = context
        .creation
        .as_ref()
        .ok_or(GameError::unclassified(0x00406774, "00406774: no renderer"))?;
    let scene_settings: SceneGraphSettings = scene_graph_settings_from(
        &context.collections.ini,
        creation.globals.width,
        creation.globals.height,
        false,
        world
            .get_resource::<TesGlobals>()
            .is_some_and(|globals| globals.tes.is_some()),
    );
    let scenes = world.resource::<SceneGraphGlobals>().clone();
    // 0067B411 0045A500 on the main thread.
    let save_load_active = save_load_active(world)?;
    let idle = settings.idle_chatter_comment_timer;
    let time_clock = clock.clone();
    let mut get_tick_count = move || clock.tick();
    let mut time64 = move || time_clock.time64();
    let mut runtime = SceneControllers {
        particles: &mut context.particles,
        tile_textures: None,
    };
    let window = crt.clone().with_world_flag(world, |world, random| {
        run_player_window(
            world,
            &mut PlayerInputs {
                process: ProcessInputs {
                    random,
                    idle_chatter_comment_timer: idle,
                    low_process_unset_88: 0,
                    save_load_active,
                },
                controller: ControllerInputs::default(),
                settings,
                world_scene_graph: scenes.world,
                scene_settings: &scene_settings,
                scene_runtime: &mut runtime,
                get_tick_count: &mut get_tick_count,
                time64: &mut time64,
            },
            false,
            &mut |text| startup_log(text),
        )
    })?;
    context.player = window.player;
    let player = window.player.ok_or(GameError::unclassified(
        0x004067AB,
        "004067AB: a null g_thePlayer",
    ))?;
    let high = world
        .get::<HighProcess>(player_process(world, player)?)
        .is_some();
    let controller = actor_character_controller(world, player).is_some();
    let group = world
        .resource::<CharacterControllerGlobals>()
        .collision_group_b2eb3c;
    let fov = world
        .get::<crate::tes::player::PlayerCharacter>(player)
        .map_or(0.0, |body| body.camera_fov_598);
    let scale = world.resource::<PlayerGlobals>().particle_fov_scale_b2d80c;
    let id = world
        .get::<FormIdentity>(player)
        .map_or(0, |form| form.form_id_0c);
    phase_line(
        world,
        format!(
            "Phase-11: player 0x{}, process {}, controller {}, collision group {group}, camera FOV {}, particle FOV scale {}",
            if id == PLAYER_FORM_ID { "14" } else { "?" },
            if high { "High" } else { "other" },
            if controller { "created" } else { "absent" },
            cpp_float(fov),
            cpp_float(scale),
        ),
    );
    Ok(PhaseOutcome::Done)
}

// IDA: N/A the actor's +0x58.
fn player_process(world: &World, player: Entity) -> Result<Entity, GameError> {
    world
        .get::<crate::tes::actor::MobileObject>(player)
        .and_then(|mobile| mobile.process_58)
        .ok_or(GameError::unclassified(
            0,
            "Phase-11: a player with no process",
        ))
}

// IDA: N/A `DriverLoadHost` for a block.
fn load_host(world: &World, context: &mut WinMainLocals) -> DriverLoadHost {
    let crt = world.resource::<MainThreadCrt>().clone();
    let files = world.resource::<SharedFiles>().clone();
    DriverLoadHost {
        files,
        crt,
        texture_gates: (
            ini_raw(context, 0x00B0_6310) as u8 != 0,
            ini_raw(context, 0x00B0_4428) as u8 != 0,
        ),
        // 00494480's text is 00B3F280 + "Warnings.txt".
        warnings_path: world
            .resource::<GameProcess>()
            .paths
            .settings_directory
            .join("Warnings.txt")
            .to_string_lossy()
            .into_owned(),
        sections: std::mem::take(&mut context.load_sections),
    }
}

// IDA: WinMain 0040EF24..0040EFF2: `winmain_player_scene`'s half, then 0040EFEE 00578CD0. 00B135C8.. and 00B37BC0, 00B37BC8 are read here since the interface's seam has neither collection.
fn player_scene(world: &mut World, context: &mut WinMainLocals) -> Result<PhaseOutcome, GameError> {
    let time = world.resource::<StartupTime>().0.clone();
    let files = world.resource::<SharedFiles>().clone();
    let settings = interface_settings(world, context);
    let mut interface = interface_host(world, context);
    let gates = tile_texture_gates(world, context);
    let mut host = load_host(world, context);
    let float = |context: &WinMainLocals, address: u32| f32::from_bits(ini_raw(context, address));
    let game = world.resource::<GameSettingCollection>();
    let scene = PlayerSceneSettings {
        ambient: [
            float(context, 0x00B1_35C8),
            float(context, 0x00B1_35D0),
            float(context, 0x00B1_35D8),
        ],
        diffuse: [
            float(context, 0x00B1_35E0),
            float(context, 0x00B1_35E8),
            float(context, 0x00B1_35F0),
        ],
        hud: HudSettings {
            map_marker_reveal_b37bc0: game.raw_at(0x00B3_7BC0).unwrap_or(0) as i32,
            map_marker_visible_b37bc8: game.raw_at(0x00B3_7BC8).unwrap_or(0) as i32,
        },
    };
    let rect = context.rect;
    let cell = context.player_cell;
    let frame_seconds = time.lock().frame_seconds_0c;
    let mut textures = tile_textures(
        &files,
        context.creation.as_mut(),
        context.scene.as_mut(),
        gates,
    );
    let mut runtime = SceneControllers {
        particles: &mut context.particles,
        tile_textures: textures
            .as_mut()
            .map(|textures| textures as &mut dyn TileTextureLoader),
    };
    run_player_scene(world, rect, cell, &mut runtime)?;
    with_interface_cx(world, &settings, &mut interface, &mut runtime, |cx| {
        interface_initialize_player_scene(cx, &scene, frame_seconds)
    })?;
    context.load_sections = std::mem::take(&mut host.sections);
    startup_log("Phase-17: 00578CD0 returned, 0040EFF3");
    Ok(PhaseOutcome::Done)
}

// IDA: 0040EFF3..0040F068: the starting spell, 006632A0, g_osGlobals +3 and 0042BA50. The IO thread's load of the spell's art is, as elsewhere, a pass on the main thread (a recorded substitution), here at the block's end.
fn starting_spell(
    world: &mut World,
    context: &mut WinMainLocals,
) -> Result<PhaseOutcome, GameError> {
    let mut files = world.resource::<SharedFiles>().clone();
    let settings = interface_settings(world, context);
    let mut host = interface_host(world, context);
    let gates = tile_texture_gates(world, context);
    let mut textures = tile_textures(
        &files,
        context.creation.as_mut(),
        context.scene.as_mut(),
        gates,
    );
    let mut runtime = SceneControllers {
        particles: &mut context.particles,
        tile_textures: textures
            .as_mut()
            .map(|textures| textures as &mut dyn TileTextureLoader),
    };
    let selected = with_interface_cx(world, &settings, &mut host, &mut runtime, |cx| {
        select_starting_spell(cx)
    })?;
    if let Some(request) = &selected {
        let mut archives = files.clone();
        queued_magic_item_run(world, request, Some(&mut archives))?;
    }
    save_player_initial_state(world)?;
    // 0040F05A: written again only at 0040F488, with no reader found.
    world
        .get_resource_mut::<OsGlobalsData>()
        .ok_or(GameError::unclassified(
            0x0040F05A,
            "0040F05A: a null g_osGlobals faults",
        ))?
        .unk_03 = 1;
    files.with_archives(&mut |manager| {
        if let Some(manager) = manager {
            manager.set_buffer_limit_128k();
        }
    });
    if let Some(request) = &selected {
        let failed = with_model_services(world, context, |world, services| {
            let mut drain = ModelDrain::new();
            drain.run_to_end(world, services)?;
            Ok::<u32, GameError>(drain.record.load_failed)
        })??;
        if failed != 0 {
            // The original's 6 IO draws come from this model, so a failed load leaves the IO thread's rand short of them.
            emit(
                Level::Warning,
                "Phase-18: the starting spell's art model didn't load",
            );
        }
        queued_magic_item_finish(world, request)?;
    }
    startup_log("Phase-18: the starting spell and the initial state, 0040F069");
    Ok(PhaseOutcome::Done)
}

/// IDA: `&HUDSubtitleMenu;`.
const HUD_SUBTITLE_MENU_ID: u32 = 1010;

// IDA: 0040F069..0040F08C: 006ECC80, the player's +0x40; nonzero jumps to 0040F0DB (Phase 21). 00447DB0(-1). 00440AF0(0, 0, 0): 00578E30, then 005A9010's HUDSubtitle and 006A9AA0, with no game sound yet. Each step of 005ADB40's loop draws in the original, so the bar's fill yields to the frame.
fn close_loading_menu(
    world: &mut World,
    context: &mut WinMainLocals,
) -> Result<PhaseOutcome, GameError> {
    if context.player_cell.is_some() {
        return Ok(PhaseOutcome::Done);
    }
    if context.closing_menu.is_none() {
        // A missing `Data` gets here with no handler, and exits 1 at the missing XML.
        if world.contains_resource::<DataHandler>() {
            propagate_runtime_field_to_masters(world);
        }
        if !world.contains_resource::<InterfaceManager>() {
            return Ok(PhaseOutcome::Done);
        }
        context.closing_menu = with_context_interface(world, context, loading_menu_close_begin)?;
    }
    if let Some(closing) = context.closing_menu {
        if closing.filling {
            let budget_active = world.resource::<WinMainProgress>().deadline.is_some();
            let full = with_context_interface(world, context, |cx| {
                loop {
                    if loading_menu_complete_step(cx, closing.menu)? {
                        return Ok::<bool, GameError>(true);
                    }
                    if budget_active && cx.world.resource::<WinMainProgress>().over_budget() {
                        return Ok(false);
                    }
                }
            })?;
            if !full {
                return Ok(PhaseOutcome::Yield);
            }
        }
        with_context_interface(world, context, |cx| {
            loading_menu_close_end(cx, closing.menu)
        })?;
        context.closing_menu = None;
    }
    with_context_interface(world, context, |cx| {
        if let Some(tile) = cx.engine.tile_menu_by_id(HUD_SUBTITLE_MENU_ID) {
            let menu = cx
                .engine
                .containing_menu(tile)
                .ok_or(GameError::unclassified(
                    0x005A9010,
                    "005A9010: a HUDSubtitle tile with no menu",
                ))?;
            tile_set_numeric_trait(cx, tile, TRAIT_VISIBLE, 2.0)?;
            let frame_time = cx.world.resource::<InterfaceManager>().frame_seconds_host;
            menu_begin_fade_in(cx, menu, frame_time);
        }
        Ok::<(), GameError>(())
    })?;
    startup_log("Phase-19: the LoadingMenu closed, 0040F08D");
    Ok(PhaseOutcome::Done)
}

// IDA: 0040F08D MainMenu_Create. 0040F070's branch leaves it out with a cell. No main_menu.xml: the original dereferences a null buffer; a host window with nothing to draw is no answer, so the host exits 1, as the C++ did.
fn create_main_menu(
    world: &mut World,
    context: &mut WinMainLocals,
) -> Result<PhaseOutcome, GameError> {
    let time = world.resource::<StartupTime>().0.clone();
    // No manager: the chain stopped short of 0040EB18 without saying so.
    if context.player_cell.is_some() || !world.contains_resource::<InterfaceManager>() {
        return Ok(PhaseOutcome::Done);
    }
    let frame_time = time.lock().frame_seconds_0c;
    let created = with_context_interface(world, context, |cx| main_menu_create(cx, frame_time))?;
    if created.is_none() {
        emit(
            Level::Error,
            "No MainMenu: Data\\Menus\\Options\\main_menu.xml is missing. Check the data directory above, or set OBLIVION_DATA_DIR.",
        );
        world.get_resource_or_init::<ScreenDirector>().stage = ScreenStage::NoMenu;
        return Ok(PhaseOutcome::Exit(1));
    }
    startup_log("Phase-20: the MainMenu created, 0040F092");
    Ok(PhaseOutcome::Done)
}

// IDA: 0040F092..0040F0D1, or 0040F0DB's wait alone with a cell. The wait yields to the frame, a recorded substitution: the movie thread's device calls are drained on the main thread. 0040F09A and 0040F09F, an empty cleared frame, aren't ported, since the host render owns its frames. 0040F0A5 and 0040F0D1 are ReturnTrue_SharedStub. 00B33430's store at 0040F0CA isn't kept: the exe has no other reference to that byte.
fn intro_sequence(
    world: &mut World,
    context: &mut WinMainLocals,
) -> Result<PhaseOutcome, GameError> {
    let player = world.get_resource::<MoviePlayer>().cloned();
    if let Some(player) = &player
        && !movie_player_poll_wait_for_thread(player)
    {
        return Ok(PhaseOutcome::Yield);
    }
    if !world.contains_resource::<InterfaceManager>() {
        return Ok(PhaseOutcome::Done);
    }
    if context.player_cell.is_some() {
        world.get_resource_or_init::<MainMenuMedia>().stage = MainMenuMediaStage::NoMenu;
        return Ok(PhaseOutcome::Done);
    }
    if let Some(input) = world.resource::<OsGlobalsData>().input_20.clone() {
        // D3: the host's mouse never takes the system's exclusively, so it's safe to read here. The real app wants its deltas.
        input.lock().acquire_exclusive_mouse = true;
    }
    let media = menu_media_of(world, context);
    let intro = player.is_some() && main_menu_title_and_logo(world, &media);
    let backdrop = with_context_interface(world, context, |cx| {
        main_menu_begin_intro(cx, || {
            player.as_ref().is_some_and(|player| {
                main_menu_begin_backdrop_movie(player, &media.main_menu_movie_b03094)
            })
        })
    })?;
    emit(
        Level::Info,
        &format!(
            "MainMenu media: {} {}, {} {}",
            media.main_menu_movie_intro_b0308c,
            if intro { "played" } else { "not played" },
            media.main_menu_movie_b03094,
            if backdrop { "playing" } else { "absent" },
        ),
    );
    let mut state = world.get_resource_or_init::<MainMenuMedia>();
    state.backdrop_movie_b3b408 = u8::from(backdrop);
    state.stage = MainMenuMediaStage::Running;
    startup_log("Phase-21: the intro sequence, 0040F0E0");
    Ok(PhaseOutcome::Done)
}

// IDA: N/A the held 0057B950 calls at a step boundary, each through `loading_menu_set_section_progress_if_ready` (00582160(0, 1), its +0x1C, then 005ADEC0(section, percent)). Then what the host's frame does: 005AD980 and 00579260's render.
fn pump_loading_menu(
    world: &mut World,
    context: &mut WinMainLocals,
    sections: &mut VecDeque<(i32, f32)>,
) -> Result<(), GameError> {
    let files = world.resource::<SharedFiles>().clone();
    let settings = interface_settings(world, context);
    let mut host = interface_host(world, context);
    let gates = tile_texture_gates(world, context);
    let mut textures = tile_textures(
        &files,
        context.creation.as_mut(),
        context.scene.as_mut(),
        gates,
    );
    let mut runtime = SceneControllers {
        particles: &mut context.particles,
        tile_textures: textures
            .as_mut()
            .map(|textures| textures as &mut dyn TileTextureLoader),
    };
    let first_draw_owed = world.resource::<InterfaceRenderRequests>().count() == 0;
    with_interface_cx(world, &settings, &mut host, &mut runtime, |cx| {
        let drained: Vec<_> = sections.drain(..).collect();
        for (section, percent) in drained {
            loading_menu_set_section_progress_if_ready(cx, section, percent)?;
        }
        loading_menu_update_display(cx, false)?;
        // Recorded host substitution: 005ADE80's draw, again. The original's device setup and tile loads put 005ADE80 past
        // the end of the intro's stub movies (run-c1-log: the thread ends, then the first draw), and the port reaches it
        // 20 ms in with the stubs still in FFmpeg, so a lost race would leave the screen black for the whole load.
        if first_draw_owed {
            interface_render_if_no_movie(cx, 0)?;
        }
        Ok(())
    })
}

// IDA: N/A `pump_loading_menu` with what a block's host left.
fn pump_pending_sections(world: &mut World, context: &mut WinMainLocals) -> Result<(), GameError> {
    let mut sections = std::mem::take(&mut context.load_sections);
    pump_loading_menu(world, context, &mut sections)
}

// IDA: 004067BA..004067C7 0044F3D0(0, 0), a step at a time. Its result is ignored at 004067CC. Completion: 100% of the block.
fn load(world: &mut World, context: &mut WinMainLocals) -> Result<PhaseOutcome, GameError> {
    let mut cursor = match context.load.take() {
        Some(cursor) => cursor,
        None => {
            let cursor = begin_load_selected_files(world, &context.collections.ini)?;
            context.disable_warning_messages_b06b18 = world
                .resource::<DataLoadGlobals>()
                .disable_warning_messages_b06b18;
            cursor
        }
    };
    let root = world.resource::<GameProcess>().root.clone();
    let mut host = load_host(world, context);
    let mut warning = |text: String, _: bool| emit(Level::Warning, &text);
    let mut services = DataLoadServices {
        working_directory: &root,
        warning: &mut warning,
        host: &mut host,
    };
    let budget_active = world.resource::<WinMainProgress>().deadline.is_some();
    let mut finished = false;
    loop {
        if cursor.step(world, &mut services)?.is_some() {
            finished = true;
            break;
        }
        if budget_active && world.resource::<WinMainProgress>().over_budget() {
            break;
        }
    }
    let mut sections = std::mem::take(&mut host.sections);
    pump_loading_menu(world, context, &mut sections)?;
    if !finished {
        context.load = Some(cursor);
        return Ok(PhaseOutcome::Yield);
    }
    let data = world.resource::<DataHandler>();
    let load = world.resource::<DataLoadGlobals>();
    let inventory = world.resource::<InventoryGlobals>();
    let line = format!(
        "Phase-12: files compiled {}, records {} of {}, forms {}, editor IDs {}, item scripts not executed {}, doors not registered {}, sky climate not applied {}",
        data.loaded_mod_count_8d0,
        load.record_count_b33a94,
        load.record_total_b33a90,
        world.resource::<FormsById>().count(),
        world
            .resource::<crate::tes::data_handler::EditorIdMap>()
            .count(),
        inventory.script_runs_not_executed,
        inventory.pathing_doors_not_registered,
        world
            .get_resource::<DefaultFormStatics>()
            .map_or(0, |statics| statics.sky_climate_calls),
    );
    phase_line(world, line);
    Ok(PhaseOutcome::Done)
}

// IDA: 004067CC..00406944: 00443550; 004067D7..00406861 the player base; 00406866..004068CE its InitItem and checks; 004068D1..00406944 the scripts, 006AF850(0) and the FPS clamp. Completion: 100% of the block.
fn game_data_tail(
    world: &mut World,
    context: &mut WinMainLocals,
) -> Result<PhaseOutcome, GameError> {
    let time = world.resource::<StartupTime>().0.clone();
    let files = world.resource::<SharedFiles>().clone();
    let gates = (
        context.memory.object_load_callbacks_enabled(),
        world.resource::<WinMainStatics>().load_direct_hint,
    );
    let io_thread: ThreadId = world
        .resource::<TaskGlobals>()
        .io_manager_b33a10
        .as_ref()
        .ok_or(GameError::unclassified(
            0x004067CC,
            "004067CC: a null g_IOManager",
        ))?
        .main_thread_id_30;
    {
        let WinMainLocals {
            creation, scene, ..
        } = context;
        let creation = creation
            .as_mut()
            .ok_or(GameError::unclassified(0x004067CC, "004067CC: no renderer"))?;
        let window = scene.as_mut().ok_or(GameError::unclassified(
            0x004067CC,
            "004067CC: no scene window",
        ))?;
        let mut statistics = std::mem::take(&mut window.texture_statistics);
        let mut textures = DriverWaterTextures {
            files: files.clone(),
            creation,
            drop_static_app_data: gates.0,
            direct_to_renderer_hint: gates.1,
            statistics: &mut statistics,
            io_thread,
        };
        let mut warning = |text: String| emit(Level::Warning, &text);
        let mut archives = files.clone();
        let result = run_tes_preload(
            world,
            &mut TesPreloadServices {
                archives: Some(&mut archives),
                textures: WaterServices {
                    shader: None,
                    image_space_effects: 0,
                    render_path: 0,
                    prepare: None,
                    textures: Some(&mut textures),
                    warning: &mut warning,
                },
            },
        );
        window.texture_statistics = statistics;
        result?;
    }
    context.base = run_player_base(world, &mut |text| startup_log(text))?;
    let root = world.resource::<GameProcess>().root.clone();
    let mut host = load_host(world, context);
    let mut warning = |text: String, _: bool| emit(Level::Warning, &text);
    let mut services = DataLoadServices {
        working_directory: &root,
        warning: &mut warning,
        host: &mut host,
    };
    let checks = run_player_init(world, context.base, &mut services)?;
    let object_runs = world.resource::<InventoryGlobals>().reference_script_runs;
    let os_globals = world.get_resource::<OsGlobalsData>().cloned();
    let clamped = {
        let mut time = time.lock();
        run_scripts_sound_and_clamp(
            world,
            ScriptsSoundInputs {
                services: &mut services,
                ini: &context.collections.ini,
                os_globals: os_globals.as_ref(),
                time: &mut time,
                log: &mut |text| startup_log(text),
            },
        )?
    };
    context.load_sections = std::mem::take(&mut host.sections);
    pump_pending_sections(world, context)?;
    let inventory = world.resource::<InventoryGlobals>();
    let queued = world
        .resource::<TaskGlobals>()
        .io_manager_b33a10
        .as_ref()
        .map_or(0, |io| io.tasks.queued().count());
    let decals = world
        .resource::<TesGlobals>()
        .tes
        .as_ref()
        .map_or(0, |tes| tes.blood_decals_94.iter().flatten().count());
    let line = format!(
        "Phase-13: models queued {queued}, blood decals {decals}, player base {}, fatigue {}, health {}, object scripts not executed {}, sound source {}, fps clamp {}",
        if context.base.is_some() {
            "NPC_ 0x7"
        } else {
            "null"
        },
        checks.fatigue,
        checks.health,
        inventory.reference_script_runs - object_runs,
        if world
            .get_resource::<SoundSourceGlobal>()
            .is_some_and(|source| source.0.is_some())
        {
            "set"
        } else {
            "null"
        },
        if clamped { "stored" } else { "unchanged" },
    );
    phase_line(world, line);
    Ok(PhaseOutcome::Done)
}

// IDA: N/A what a pass of 00433BC0 reads, over the real files, renderer and slots, with the IO thread's own CRT. `body` runs with it.
fn with_model_services<R>(
    world: &mut World,
    context: &mut WinMainLocals,
    body: impl FnOnce(&mut World, &mut ModelBuildServices<'_>) -> R,
) -> Result<R, GameError> {
    world.resource_scope(|world, mut random: Mut<IoThreadRand>| {
        with_model_rng(world, context, &mut random.0, body)
    })
}

// IDA: N/A the same synchronous model loader used by startup, at an actor's gameplay call site.
fn load_gameplay_model(
    world: &mut World,
    path: &[u8],
    a3: bool,
    lod: u32,
    a5: bool,
) -> Result<Option<Entity>, GameError> {
    let mut context = world.remove_non_send::<WinMainLocals>().ok_or_else(|| {
        GameError::stop(
            0x00439EB0,
            "a synchronous model load with no driver context",
        )
    })?;
    let result = with_model_services(world, &mut context, |world, services| {
        crate::bs::model_loader::load_model_now(world, path, a3, lod, a5, services)
    });
    world.insert_non_send(context);
    result?
}

// IDA: N/A the driver's model services at a gameplay call site that builds 3D.
fn gameplay_model_services(
    world: &mut World,
    body: &mut dyn FnMut(&mut World, &mut ModelBuildServices<'_>) -> Result<(), GameError>,
) -> Result<(), GameError> {
    let mut context = world.remove_non_send::<WinMainLocals>().ok_or_else(|| {
        GameError::stop(0x00479780, "gameplay model services with no driver context")
    })?;
    let result = with_model_services(world, &mut context, |world, services| body(world, services));
    world.insert_non_send(context);
    result?
}

// IDA: N/A the model service loan while the I/O CRT is checked out of the world.
fn with_model_rng<R>(
    world: &mut World,
    context: &mut WinMainLocals,
    io_rand: &mut MsvcRand,
    body: impl FnOnce(&mut World, &mut ModelBuildServices<'_>) -> R,
) -> Result<R, GameError> {
    let crt = world.resource::<MainThreadCrt>().clone();
    let files = world.resource::<SharedFiles>().clone();
    let defaults = property_defaults(world);
    let gates = (
        context.memory.object_load_callbacks_enabled(),
        world.resource::<WinMainStatics>().load_direct_hint,
    );
    let mut disable = context.disable_warning_messages_b06b18;
    let WinMainLocals {
        creation,
        scene,
        registry,
        setups_not_run,
        ..
    } = context;
    let creation = creation
        .as_mut()
        .ok_or(GameError::unclassified(0x00433BC0, "00433BC0: no renderer"))?;
    let window = scene.as_mut().ok_or(GameError::unclassified(
        0x00433BC0,
        "00433BC0: no scene window",
    ))?;
    let mut statistics = std::mem::take(&mut window.texture_statistics);
    let (hdr, render_path, shaders_30, precache) = (
        creation.globals.hdr,
        creation.globals.render_path,
        creation.globals.shaders_30,
        creation.globals.texture_create_renderer_data,
    );
    let reference_classifier = window.scene_graphs.reference_classifier;
    let cell = std::cell::RefCell::new((creation, &mut statistics));
    let mut shaders = RealShaders { window };
    let mut get_shader = |index: u32| shaders.get(index);
    let normal_files = files.clone();
    let mut load_normal = |world: &mut World, path: &[u8]| {
        let mut held = cell.borrow_mut();
        let (creation, statistics) = &mut *held;
        let mut services = texture_services(&normal_files, creation, gates.0, gates.1, statistics);
        // 007D1A1E 007B8200(path, 1, 1).
        load_shader_texture(world, path, true, true, &mut services).is_some()
    };
    let mut setup = |_vtable: u32, _geometry: Entity| {
        *setups_not_run += 1;
        true
    };
    let open_files = files.clone();
    let mut open = move |path: &str, mode: i32, size: u32| {
        open_files
            .open_core(path, mode, size, 0xFFFF)
            .map(crate::bs::file::OpenedFile::into_game_file)
    };
    let mut draw = || io_rand.rand();
    // The main thread's finish (0043B090 and what it reaches) draws from the main thread's CRT.
    let mut main_draw = || crt.random_next();
    let mut textures = DriverSourceTextures {
        files: files.clone(),
    };
    let create_files = files.clone();
    let mut create = |world: &mut World, texture: &mut crate::gamebryo::texture::SourceTexture| {
        let mut held = cell.borrow_mut();
        let (creation, statistics) = &mut *held;
        let mut services = texture_services(&create_files, creation, gates.0, gates.1, statistics);
        create_file_renderer_data(world, texture, &mut services)
    };
    let mut archives = files.clone();
    let mut warning = |text: Vec<u8>, _: bool| {
        emit(
            Level::Warning,
            &text
                .iter()
                .map(|&byte| char::from(byte))
                .collect::<String>(),
        );
    };
    let mut services = ModelBuildServices {
        archives: Some(&mut archives),
        stream: BsStreamServices {
            open: &mut open,
            registry,
            draw_rand: Some(&mut draw),
            draw_on_io_thread: true,
            source_textures: Some(&mut textures),
            post_load: StreamPostLoadRuntime {
                precache_source_textures: precache,
                release_static_texture_pixel_data: gates.0,
                create_source_texture_renderer_data: Some(&mut create),
                ..StreamPostLoadRuntime::default()
            },
        },
        prepare: PrepareGeometryContext {
            render_path,
            hdr,
            reference_classifier,
            property_defaults: defaults,
            get_shader: &mut get_shader,
            warnings: Vec::new(),
            sky_shader_trace: Vec::new(),
            water_shader_trace: Vec::new(),
            lava_b47844: 0,
            shaders_30_b42ea5: shaders_30,
            load_normal_map: Some(&mut load_normal),
            unported_shader_setup: Some(&mut setup),
            classify_trace: Vec::new(),
        },
        disable_warning_messages_b06b18: &mut disable,
        // 00B34390 has no writer in the exe.
        show_editor_markers_b34390: 0,
        warning: &mut warning,
        main_random_next: Some(&mut main_draw),
    };
    let result = body(world, &mut services);
    drop(services);
    let window = context.scene.as_mut().expect("just used");
    window.texture_statistics = statistics;
    context.disable_warning_messages_b06b18 = disable;
    Ok(result)
}

// IDA: 00433BC0 on the main thread, a recorded substitution: the IO thread's run loop as steps, and the tasks all present when the walk starts, where the original's thread starts on the first and races the queue. Its order differs from P13-RUN-001's in which task runs first; each task's own work doesn't. Completion: 100% of the walk.
fn model_drain(world: &mut World, context: &mut WinMainLocals) -> Result<PhaseOutcome, GameError> {
    let mut drain = context.drain.take().unwrap_or_default();
    let budget_active = world.resource::<WinMainProgress>().deadline.is_some();
    let ended = with_model_services(world, context, |world, services| {
        loop {
            let step = drain.step(world, services)?;
            if matches!(
                step,
                ModelDrainStep::Ended | ModelDrainStep::WaitingForToken
            ) {
                return Ok::<bool, GameError>(true);
            }
            if budget_active && world.resource::<WinMainProgress>().over_budget() {
                return Ok(false);
            }
        }
    })??;
    pump_pending_sections(world, context)?;
    if !ended {
        context.drain = Some(drain);
        return Ok(PhaseOutcome::Yield);
    }
    let line = drain.record.log_line(context.setups_not_run);
    phase_line(world, line);
    Ok(PhaseOutcome::Done)
}

// IDA: 0057A0D0 with a host for it: at 0040EB62 no map menu exists, so 0057A0FF's lookup answers null before any file is read.
fn refresh_map_menu(
    world: &mut World,
    settings: &InterfaceSettings,
    host: &mut dyn InterfaceHost,
) -> Result<bool, GameError> {
    let mut particles = ParticleControllerScheduleContext::default();
    let mut runtime = SceneControllers {
        particles: &mut particles,
        tile_textures: None,
    };
    with_interface_cx(
        world,
        settings,
        host,
        &mut runtime,
        interface_refresh_map_menu,
    )
}

// IDA: 0040EB31..0040EBB8: 0040EB32 0055F750(0), the stack point, 004431F0, 00662EE0, 0042C970, TLS +0x185, the log, 00675310, 00447300, the log. Completion: 100% of the block.
fn world_setup(world: &mut World, context: &mut WinMainLocals) -> Result<PhaseOutcome, GameError> {
    let crt = world.resource::<MainThreadCrt>().clone();
    let files = world.resource::<SharedFiles>().clone();
    let display_lod_land = ini_raw(context, 0x00B0_2D70) as u8;
    context.display_lod_land_b43077 = display_lod_land;
    let settings = interface_settings(world, context);
    let mut interface = interface_host(world, context);
    let root = world.resource::<GameProcess>().root.clone();
    let mut host = load_host(world, context);
    let mut warning = |text: String, _: bool| emit(Level::Warning, &text);
    let mut services = DataLoadServices {
        working_directory: &root,
        warning: &mut warning,
        host: &mut host,
    };
    let outputs = run_world_setup_worldspace(
        world,
        WorldspaceInputs {
            ini: &context.collections.ini,
            display_lod_land_b43077: display_lod_land,
            services: &mut services,
            level2_log: &mut |text| emit(Level::Info, &text),
            refresh_map_menu: &mut |world| refresh_map_menu(world, &settings, &mut interface),
        },
    )?;
    context.rect = outputs.default_position;
    run_actor_locations_prelude(world, &mut services, &mut |text| startup_log(text));
    {
        let mut menu_mode = |world: &World| crate::tes::interface_manager::is_menu_mode(world);
        crt.clone().with_world_flag(world, |world, random| {
            run_tracked_actors(
                world,
                &mut AiRealArms {
                    random,
                    menu_mode: &mut menu_mode,
                },
            )
        })?;
    }
    let autocalc = run_world_setup_finish(world, &mut |text| startup_log(text))?;
    context.load_sections = std::mem::take(&mut host.sections);
    let name = outputs.world_space.map_or_else(
        || "null".to_owned(),
        |space| {
            editor_id_of(world, space)
                .iter()
                .map(|&byte| char::from(byte))
                .collect()
        },
    );
    let idle_roots = world
        .get_resource::<crate::tes::forms_ai::IdleAnimRoots>()
        .map_or(0, |roots| roots.roots.len());
    let changes = world
        .resource::<TesGlobals>()
        .save_load_game
        .as_ref()
        .map_or(0, |game| game.changes_map_00.count_0c);
    pump_pending_sections(world, context)?;
    let ai = world.resource::<AiProcessGlobals>();
    let line = format!(
        "Phase-14: worldspace {name}, idle roots {idle_roots}, archives {}, change records {changes}, tracked actors {}, autocalc NPCs {}, scripts not executed {}",
        files.archive_count(),
        ai.actors_visited,
        autocalc.len(),
        ai.script_event_runs_not_executed,
    );
    phase_line(world, line);
    Ok(PhaseOutcome::Done)
}

// IDA: N/A `TesGridAnswers` from their real owners where one exists: 0057BAC0. 004F9FA0 reads the byte 00B09E25, whose image byte is 1, and its only writers (00500D5B, 0050E60D, 0050E668) are console and script commands: true is the exe's value. `cell_tasks_pending` (0043E000) and `game_time` (00B42D78) stay the seams' answers: their walks aren't ported.
fn grid_answers(world: &mut World, context: &WinMainLocals) -> TesGridAnswers {
    let settings = interface_settings(world, context);
    let mut host = interface_host(world, context);
    let mut particles = ParticleControllerScheduleContext::default();
    let mut runtime = SceneControllers {
        particles: &mut particles,
        tile_textures: None,
    };
    // IDA: 0057BAC0: no manager or cursor: false; 00589B70(0x3EF) null: false; else 0057BAFF `fcomp` trait 0xFA1 with 00A2F948 (1.0), and 0057BB07 `test ah, 0x44; jnp` returns 0 only on equal: true for anything else, NaN too. The menu's XML holds 2.0, so P15-RUN-007's true.
    let hidden = with_interface_cx(world, &settings, &mut host, &mut runtime, |cx| {
        let ready = cx
            .world
            .get_resource::<InterfaceManager>()
            .is_some_and(|manager| manager.cursor_01c.is_some());
        if !ready {
            return false;
        }
        cx.engine
            .tile_menu_by_id(0x3EF)
            .is_some_and(|tile| tile_numeric_trait(cx, tile, TRAIT_VISIBLE) != 1.0)
    });
    TesGridAnswers {
        loading_menu_hidden: hidden,
        scripts_ready: crate::tes::script_vm::scripts_enabled(world),
        ..TesGridAnswers::default()
    }
}

// IDA: 0040EBB9..0040EE79. Completion: 100% of the block.
fn initial_area(world: &mut World, context: &mut WinMainLocals) -> Result<PhaseOutcome, GameError> {
    let answers = grid_answers(world, context);
    let settings = interface_settings(world, context);
    let mut interface = interface_host(world, context);
    let root = world.resource::<GameProcess>().root.clone();
    let mut host = load_host(world, context);
    let mut warning = |text: String, _: bool| emit(Level::Warning, &text);
    let mut load = DataLoadServices {
        working_directory: &root,
        warning: &mut warning,
        host: &mut host,
    };
    let mut level2 = |text: String| emit(Level::Info, &text);
    let mut refresh = |world: &mut World| refresh_map_menu(world, &settings, &mut interface);
    let mut grid = TesGridServices {
        load: &mut load,
        ini: &context.collections.ini,
        answers,
        display_lod_land_b43077: context.display_lod_land_b43077,
        level2_log: &mut level2,
        refresh_map_menu: &mut refresh,
        interface: None,
        drain: None,
        reference_loop: false,
        game_sounds: None,
        archives: None,
        models: None,
    };
    let mut arms = RealInitialAreaArms { grid: &mut grid };
    let mut rect = context.rect;
    let initial = run_initial_area(world, &context.collections.ini, &mut arms, &mut rect)?;
    context.load_sections = std::mem::take(&mut host.sections);
    context.rect = rect;
    let cell_id = initial
        .cell
        .and_then(|cell| world.get::<FormIdentity>(cell))
        .map_or(0, |form| form.form_id_0c);
    let path = match initial.path {
        InitialAreaPath::NoSettings => "no settings",
        InitialAreaPath::EditorIdCell => "editor ID cell",
        InitialAreaPath::PluginWorldspace => "plugin worldspace",
        InitialAreaPath::CoordinatesCell => "coordinates cell",
        InitialAreaPath::NotFound => "not found",
    };
    phase_line(
        world,
        format!(
            "Phase-15: path {path}, cell {cell_id}, Rect {{{}, {}, {}}}, tail {}",
            cpp_float(rect[0]),
            cpp_float(rect[1]),
            cpp_float(rect[2]),
            if initial.tail { "run" } else { "skipped" },
        ),
    );
    if let Some(boundaries) = world.get_resource::<crate::tes::tes_grid::TesGridBoundaries>() {
        // The C++ `TesGridBoundaries::text`.
        let text = boundaries
            .calls
            .iter()
            .map(|(address, count)| format!("{address:08X} x{count}"))
            .collect::<Vec<_>>()
            .join(", ");
        if !text.is_empty() {
            phase_line(world, format!("Phase-15c/d: counted boundaries {text}"));
        }
    }
    Ok(PhaseOutcome::Done)
}

// IDA: 0040EE7A..0040EF23. Completion: 100% of the block.
fn place_player(world: &mut World, context: &mut WinMainLocals) -> Result<PhaseOutcome, GameError> {
    let answers = grid_answers(world, context);
    let settings = interface_settings(world, context);
    let mut interface = interface_host(world, context);
    let root = world.resource::<GameProcess>().root.clone();
    let mut host = load_host(world, context);
    let mut warning = |text: String, _: bool| emit(Level::Warning, &text);
    let mut load = DataLoadServices {
        working_directory: &root,
        warning: &mut warning,
        host: &mut host,
    };
    let mut level2 = |text: String| emit(Level::Info, &text);
    let mut refresh = |world: &mut World| refresh_map_menu(world, &settings, &mut interface);
    let collections = std::mem::take(&mut context.collections);
    let mut grid = TesGridServices {
        load: &mut load,
        ini: &collections.ini,
        answers,
        display_lod_land_b43077: context.display_lod_land_b43077,
        level2_log: &mut level2,
        refresh_map_menu: &mut refresh,
        interface: None,
        drain: None,
        reference_loop: false,
        game_sounds: None,
        archives: None,
        models: None,
    };
    let mut rect = context.rect;
    let placed = {
        // 00433590: the IO thread's walk, as a pass on the main thread.
        let mut run_timed = |world: &mut World| -> Result<(), GameError> {
            with_model_services(world, context, |world, services| {
                let mut drain = ModelDrain::new();
                drain.run_to_end(world, services).map(|_| ())
            })?
        };
        let mut sleep = |ms: u32| std::thread::sleep(Duration::from_millis(u64::from(ms)));
        let mut info = |text: &str| emit(Level::Info, text);
        let mut arms = RealPlacePlayerArms {
            grid: &mut grid,
            run_timed_tasks: &mut run_timed,
            sleep: &mut sleep,
            info: &mut info,
        };
        run_place_player(world, &collections.ini, &mut arms, &mut rect)
    };
    context.collections = collections;
    context.load_sections = std::mem::take(&mut host.sections);
    let placed = placed?;
    context.rect = rect;
    if let Some(drained) = placed.drain.as_ref() {
        phase_line(
            world,
            format!(
                "Phase-16c: 00434020 total {}, loops {}, host-gated {}",
                drained.total, drained.loops, drained.host_gated
            ),
        );
    }
    context.player_cell = placed.cell;
    let cell_id = placed
        .cell
        .and_then(|cell| world.get::<FormIdentity>(cell))
        .map_or(0, |form| form.form_id_0c);
    phase_line(
        world,
        format!(
            "Phase-16: cell {cell_id}, Rect {{{}, {}, {}}}, rotation z {}, {}",
            cpp_float(rect[0]),
            cpp_float(rect[1]),
            cpp_float(rect[2]),
            cpp_float(placed.rotation[2]),
            if placed.reached_0040ef54 {
                "0040EF54"
            } else {
                "0040EF24"
            },
        ),
    );
    pump_pending_sections(world, context)?;
    Ok(PhaseOutcome::Done)
}

// ---- for the host ----

/// IDA: N/A host accessors previously returned their absent-driver result while a block,
/// interface call or grid loan held the driver's locals. Internal services remain usable by the borrower.
fn published_driver_service<R: Resource>(world: &World) -> Option<&R> {
    world.get_non_send::<WinMainLocals>()?;
    world.get_resource::<R>()
}

/// IDA: N/A the input the host feeds `HostOsPlatform`, which `GetDeviceState` answers on both threads. `oblivion_host::raw_input` fills it each frame.
#[must_use]
pub fn os_input_frame(world: &World) -> Option<Arc<Mutex<HostInputFrame>>> {
    published_driver_service::<OsPlatformHandle>(world)
        .map(|platform| Arc::clone(&lock_shared(&platform.0).input))
}

/// IDA: N/A the one files object: the finder and archives, for a host read after the load, such as a UI sound.
#[must_use]
pub fn shared_files(world: &World) -> Option<SharedFiles> {
    published_driver_service::<SharedFiles>(world).cloned()
}

/// IDA: N/A what the frames after `Block::Finished` take from the driver: `winmain_menu_media`.
pub struct MenuMediaContext {
    pub music: Arc<Mutex<dyn MusicDevice>>,
    pub os_platform: Arc<Mutex<HostOsPlatform>>,
    pub time: SharedTimeInfo,
    pub clock: HostClock,
    pub settings: MusicSettings,
    /// 00B14374 sMainMenuMusic, its bytes.
    /// 00B1436C sCharGenQuest:General.
    pub chargen_quest_b1436c: Vec<u8>,
    pub main_menu_music_b14374: Vec<u8>,
    /// 00B0308C sMainMenuMovieIntro.
    pub main_menu_movie_intro_b0308c: String,
    /// 00B03094 sMainMenuMovie.
    pub main_menu_movie_b03094: String,
    pub player_has_cell: bool,
    /// 00B16178 bSoundEnabled.
    pub sound_enabled_b16178: bool,
    /// 00B06704 fGlobalTimeMultiplier:General (009DC302 registers it, 0040E733 loads it), 0047D247's `fmul`.
    pub time_multiplier_b06704: f32,
}

/// IDA: N/A a test's replay of an original run's GetTickCount through the driver's clock (`HostClock::set_scripted_tick`):
/// every later read answers `tick` until it's changed, `None` restoring the seeded or real clock.
pub fn set_driver_tick(world: &World, tick: Option<u32>) {
    if let Some(clock) = published_driver_service::<HostClock>(world) {
        clock.set_scripted_tick(tick);
    }
}

/// IDA: N/A a test's replay of an original run's `_time64` through the driver's clock (`HostClock::set_scripted_time64`).
pub fn set_driver_time64(world: &World, time: Option<u32>) {
    if let Some(clock) = published_driver_service::<HostClock>(world) {
        clock.set_scripted_time64(time);
    }
}

/// IDA: N/A the main thread's CRT, for a script's 0047DF80.
pub(crate) fn driver_crt(world: &World) -> MainThreadCrt {
    world.resource::<MainThreadCrt>().clone()
}

/// IDA: N/A a helper, with no address of its own.
pub(crate) fn menu_media_context(world: &World) -> Option<MenuMediaContext> {
    world
        .get_non_send::<WinMainLocals>()
        .map(|locals| menu_media_of(world, locals))
}

/// IDA: N/A `menu_media_context` from a holder of the context.
fn menu_media_of(world: &World, context: &WinMainLocals) -> MenuMediaContext {
    let time = world.resource::<StartupTime>().0.clone();
    let clock = world.resource::<HostClock>().clone();
    let os_platform = world.resource::<OsPlatformHandle>().0.clone();
    let text = |address| {
        context
            .collections
            .ini
            .find_address(address)
            .and_then(|setting| setting.text.clone())
            .unwrap_or_default()
    };
    let latin1 = |bytes: Vec<u8>| bytes.iter().map(|&byte| char::from(byte)).collect();
    MenuMediaContext {
        music: Arc::clone(&world.non_send::<PlatformSeams>().0.music),
        os_platform,
        time,
        clock,
        settings: MusicSettings {
            music_enabled_b16180: ini_raw(context, 0x00B1_6180) as u8 != 0,
            main_menu_music_volume_b16188: f32::from_bits(ini_raw(context, 0x00B1_6188)),
        },
        main_menu_music_b14374: text(0x00B1_4374),
        chargen_quest_b1436c: text(0x00B1_436C),
        main_menu_movie_intro_b0308c: latin1(text(0x00B0_308C)),
        main_menu_movie_b03094: latin1(text(0x00B0_3094)),
        player_has_cell: context.player_cell.is_some(),
        sound_enabled_b16178: ini_raw(context, 0x00B1_6178) as u8 != 0,
        time_multiplier_b06704: f32::from_bits(ini_raw(context, 0x00B0_6704)),
    }
}

/// IDA: N/A the handle on 0057F170's requests, whose count stands for the game's Presents: `oblivion_host::movie_frames` holds a closed movie's picture until the next.
#[must_use]
pub fn render_requests(world: &World) -> Option<InterfaceRenderRequests> {
    published_driver_service::<InterfaceRenderRequests>(world).cloned()
}

/// IDA: N/A the frame's work outside the chain: the media frames are `winmain_menu_media`. The host's exit runs `winmain_shutdown`.
pub fn winmain_shutdown(world: &mut World) {
    // IDA: N/A `MoviePlayerGuard`: stop and join the thread while what it uses exists. The original's own waiters are 0040F092 and 0040F0DB, `Block::IntroSequence`'s.
    if let Some(player) = world.get_resource::<MoviePlayer>() {
        movie_player_shutdown(player);
    }
}

/// IDA: N/A `SkyGlobals`' sibling, kept for the host: the requests since the last call.
#[must_use]
pub fn take_render_requests(world: &World) -> Vec<u32> {
    published_driver_service::<InterfaceRenderRequests>(world)
        .map_or_else(Vec::new, InterfaceRenderRequests::take)
}

/// IDA: N/A g_dx9Renderer +0x200, the flag `HostMoviePlatform` reads, for whoever renders.
#[must_use]
pub fn in_scene_flag(world: &World) -> Option<Arc<AtomicBool>> {
    published_driver_service::<InSceneFlag>(world).map(|flag| Arc::clone(&flag.0))
}

/// IDA: N/A the main thread's `_holdrand`, and how many draws gave it, for a report or a test.
#[must_use]
pub fn main_thread_crt_state(world: &World) -> Option<(u32, u64)> {
    published_driver_service::<MainThreadCrt>(world).map(|crt| (crt.holdrand(), crt.draws()))
}

/// IDA: N/A sets the main thread's `_holdrand` (`srand`), for a test replaying a capture's draws from a logged state.
pub fn set_main_thread_holdrand(world: &World, holdrand: u32) {
    if let Some(crt) = published_driver_service::<MainThreadCrt>(world) {
        crate::tes::actor_process::RandomSource::srand(&mut crt.clone(), holdrand);
    }
}

/// IDA: N/A g_timeInfo (00B33E90) +0x14, the clock's game-start ticks, for a test replaying a capture's +0x10.
#[must_use]
pub fn driver_game_start_ticks(world: &World) -> u32 {
    published_driver_service::<StartupTime>(world)
        .map_or(0, |time| time.0.lock().game_start_ticks_14)
}

/// IDA: N/A g_timeInfo (00B33E90) +0x0C and +0x10 as the driver's frames leave them, for a test.
#[must_use]
pub fn driver_time_info(world: &World) -> (f32, u32) {
    published_driver_service::<StartupTime>(world).map_or((0.0, 0), |time| {
        let time = time.0.lock();
        (time.frame_seconds_0c, time.paused_ticks_10)
    })
}

/// IDA: N/A 00B04360, for a test.
#[must_use]
pub fn archive_buffer_limit(world: &World) -> Option<u32> {
    published_driver_service::<SharedFiles>(world)?
        .lock()
        .archives
        .as_ref()
        .map(ArchiveManager::buffer_limit)
}

/// IDA: N/A the IO thread's `_holdrand`, for a report or a test.
#[must_use]
pub fn io_thread_holdrand(world: &World) -> Option<u32> {
    published_driver_service::<IoThreadRand>(world).map(|rand| rand.0.holdrand())
}

/// IDA: N/A a `body` that reaches interface code, as a block's would: the settings and host of a frame, with the driver's own context, and 00591360's real targets. For the screens, which run after `Block::Finished`.
pub fn with_driver_interface<R>(
    world: &mut World,
    body: impl FnOnce(&mut InterfaceCx) -> R,
) -> Option<R> {
    let mut context = world.remove_non_send::<WinMainLocals>()?;
    let result = with_context_interface(world, &mut context, body);
    world.insert_non_send(context);
    Some(result)
}

/// IDA: N/A a script command's cell load (0066EAF0's 004455E0) with the services the startup tail's blocks build: the driver's load host and the grid's INI and answers, as `place_player` makes them. No context, as before the driver is installed, is a stop.
/// # Errors
/// The body's, or no context.
pub fn with_grid_services(
    world: &mut World,
    body: &mut crate::tes::player_move::CellLoadBody<'_>,
) -> Result<(), GameError> {
    let mut context = world
        .remove_non_send::<WinMainLocals>()
        .ok_or(GameError::unclassified(
            0x0066EAF0,
            "0066EAF0: a cell load with no driver",
        ))?;
    let files = world.resource::<SharedFiles>().clone();
    let answers = grid_answers(world, &context);
    let settings = interface_settings(world, &context);
    let mut interface = interface_host(world, &context);
    let root = world.resource::<GameProcess>().root.clone();
    let mut host = load_host(world, &mut context);
    let mut warning = |text: String, _: bool| emit(Level::Warning, &text);
    let mut load = DataLoadServices {
        working_directory: &root,
        warning: &mut warning,
        host: &mut host,
    };
    let mut level2 = |text: String| emit(Level::Info, &text);
    let mut refresh = |world: &mut World| refresh_map_menu(world, &settings, &mut interface);
    let mut grid_archives = files.clone();
    let sound_settings = crate::tes::game_sounds::GameSoundSettings {
        sound_enabled_b16178: ini_raw(&context, 0x00B1_6178) as u8 != 0,
        invalidate_older_files: archive_settings_from(&context.collections.ini)
            .invalidate_older_files,
    };
    let collections = std::mem::take(&mut context.collections);
    let display_lod_land = context.display_lod_land_b43077;
    // The drain, the menu's progress and the interface each need the context in turn.
    let context = std::cell::RefCell::new(context);
    let result = {
        let mut lend_interface =
            |world: &mut World,
             body: &mut dyn FnMut(&mut InterfaceCx) -> Result<(), GameError>|
             -> Result<(), GameError> {
                with_context_interface(world, &mut context.borrow_mut(), |cx| body(cx))
            };
        let mut grid = TesGridServices {
            load: &mut load,
            ini: &collections.ini,
            answers,
            display_lod_land_b43077: display_lod_land,
            level2_log: &mut level2,
            refresh_map_menu: &mut refresh,
            interface: None,
            drain: None,
            reference_loop: false,
            game_sounds: None,
            archives: None,
            models: None,
        };
        // 00434020 with 00433590 as the host's pass, as `place_player` drains; 0057B950's progress reaches the menu at once.
        let mut drain = |world: &mut World, stage: i32| {
            let mut run_timed = |world: &mut World| -> Result<(), GameError> {
                with_model_services(world, &mut context.borrow_mut(), |world, services| {
                    let mut drain = ModelDrain::new();
                    drain.run_to_end(world, services).map(|_| ())
                })?
            };
            let mut sleep = |ms: u32| std::thread::sleep(Duration::from_millis(u64::from(ms)));
            // A stop in the progress call (0057B950 -> 005ADEC0 -> 0057E150's menus) is kept and returned after the drain.
            let mut stopped: Option<GameError> = None;
            let mut section = |world: &mut World, section: i32, percent: f32| {
                if stopped.is_some() {
                    return;
                }
                let result = with_context_interface(world, &mut context.borrow_mut(), |cx| {
                    crate::tes::loading_menu::loading_menu_set_section_progress_if_ready(
                        cx, section, percent,
                    )
                });
                if let Err(error) = result {
                    stopped = Some(error);
                }
            };
            let mut save_load = |_: i32, _: f32| {};
            let result = crate::bs::model_loader::io_drain_tasks_through_stage(
                world,
                stage,
                &mut crate::bs::model_loader::IoDrainServices {
                    run_timed_tasks: &mut run_timed,
                    section_progress: &mut section,
                    save_load_progress: &mut save_load,
                    sleep: &mut sleep,
                },
            );
            if let Some(error) = stopped {
                return Err(error);
            }
            let result = result?;
            // 00578CF0's 0057D940 calls queued inside the drain (`ReticleRequests`, a host substitution).
            with_context_interface(world, &mut context.borrow_mut(), |cx| {
                crate::tes::hud_reticle::run_reticle_requests(cx)
            })?;
            Ok(result)
        };
        // 0066F358's 00444A10: ModelLoader_LoadModelNow on the main thread, with the driver's model services.
        let mut lend_models = |world: &mut World,
                               body: &mut dyn FnMut(
            &mut World,
            &mut ModelBuildServices<'_>,
        ) -> Result<(), GameError>|
         -> Result<(), GameError> {
            with_model_services(world, &mut context.borrow_mut(), |world, services| {
                body(world, services)
            })?
        };
        grid.interface = Some(&mut lend_interface);
        grid.drain = Some(&mut drain);
        grid.models = Some(&mut lend_models);
        grid.reference_loop = true;
        grid.game_sounds = Some(sound_settings);
        grid.archives = Some(&mut grid_archives);
        let mut services = crate::tes::player_move::CellLoadServices { grid: &mut grid };
        body(world, &mut services)
    };
    let mut context = context.into_inner();
    context.collections = collections;
    world.insert_non_send(context);
    result
}

/// IDA: N/A `with_driver_interface` from a block, which holds the context.
fn with_context_interface<R>(
    world: &mut World,
    context: &mut WinMainLocals,
    body: impl FnOnce(&mut InterfaceCx) -> R,
) -> R {
    let files = world.resource::<SharedFiles>().clone();
    let settings = interface_settings(world, context);
    let mut host = interface_host(world, context);
    let gates = tile_texture_gates(world, context);
    let context = RefCell::new(context);
    let mut runtime = DriverInterfaceRuntime {
        context: &context,
        files: &files,
        gates,
    };
    with_interface_cx(world, &settings, &mut host, &mut runtime, body)
}

/// IDA: N/A the interface's runtime in the driver: the context held in a cell and borrowed per call, so the tile textures,
/// the particle controllers and the model services each take the renderer in turn. `005C2F20` rebuilds the player's head
/// inside the race menu's frame, between other interface calls, as the original does.
struct DriverInterfaceRuntime<'c, 'w> {
    context: &'c RefCell<&'w mut WinMainLocals>,
    files: &'c SharedFiles,
    gates: (bool, bool),
}

impl crate::gamebryo::scene_update::SceneUpdateRuntime for DriverInterfaceRuntime<'_, '_> {
    /// IDA: N/A `DriverInterfaceRuntime`'s side of `SceneUpdateRuntime`: 00591360 with the context's renderer.
    fn tile_texture_size(&mut self, world: &mut World, path: &str) -> Option<Option<[u32; 2]>> {
        let mut context = self.context.borrow_mut();
        let context = &mut **context;
        let mut textures = tile_textures(
            self.files,
            context.creation.as_mut(),
            context.scene.as_mut(),
            self.gates,
        );
        textures
            .as_mut()
            .map(|loader| (loader as &mut dyn TileTextureLoader).load(world, path))
    }

    /// IDA: N/A `DriverInterfaceRuntime`'s side of `SceneUpdateRuntime`.
    fn update_controller(
        &mut self,
        world: &mut World,
        controller: Entity,
        time: f32,
    ) -> Result<crate::gamebryo::scene_update::SceneControllerStep, GameError> {
        crate::gamebryo::scene_update::update_node_controller(world, controller, time)
    }

    /// IDA: N/A `DriverInterfaceRuntime`'s side of `SceneUpdateRuntime`.
    fn run_particle_controllers(
        &mut self,
        world: &mut World,
        head: Option<Entity>,
        system: Entity,
        time: f32,
    ) -> Result<
        crate::gamebryo::particle_controller_schedule::ParticleControllerScheduleResult,
        GameError,
    > {
        let mut context = self.context.borrow_mut();
        crate::gamebryo::particle_controller_schedule::ParticleControllerScheduler::run(
            world,
            head,
            system,
            time,
            &mut context.particles,
        )
    }

    /// IDA: N/A `DriverInterfaceRuntime`'s side of `SceneUpdateRuntime`: `with_model_services` on the context.
    fn with_model_services(
        &mut self,
        world: &mut World,
        body: &mut dyn FnMut(&mut World, &mut ModelBuildServices<'_>) -> Result<(), GameError>,
    ) -> Result<(), GameError> {
        let mut context = self.context.borrow_mut();
        with_model_services(world, &mut context, |world, services| body(world, services))?
    }
}

/// IDA: N/A 00B33E90's +0x0C, g_timeInfo.frameTime.
#[must_use]
pub fn driver_frame_seconds(world: &World) -> f32 {
    published_driver_service::<StartupTime>(world)
        .map_or(0.0, |time| time.0.lock().frame_seconds_0c)
}

/// IDA: N/A whether a click has set g_osGlobals->quitGame (005B59B0).
#[must_use]
pub fn quit_requested(world: &World) -> bool {
    world.get_non_send::<WinMainLocals>().is_some()
        && world
            .get_non_send::<QuitFlag>()
            .is_some_and(|quit| quit.0.get())
}

/// IDA: N/A the one files object, for the host's texture reads.
#[must_use]
pub fn driver_files(world: &World) -> Option<SharedFiles> {
    shared_files(world)
}

/// IDA: N/A sets the flag above, for a host's scene.
pub fn set_in_scene(flag: &AtomicBool, value: bool) {
    flag.store(value, Ordering::Release);
}

// IDA: 004814C0 (00406B53): 007B7650 the accumulator singleton, then only
// when it is non-null, its +0x2228 is 0, 00B33A1C (the ModelLoader) and
// g_FileFinder are non-null and FindFile("Meshes\\TestSphere.NIF", 0, 0,
// -1) succeeds: 00439EB0(00B33A1C, path, 0, 0, 1), the 00405790/vt+8 walk
// and NiAVObject_RefreshPropertyState. The ModelLoader is null throughout
// Phase 05 (P05-RUN-001: 00B33A1C = 0), so the load is unreachable at
// startup; the NIF branch is carried to Phase 09+. Completion: 100% of the guards.
// TODO: [ ] Port the 00439EB0 NIF load with the TES ModelLoader (Phase 09+).
pub fn load_test_sphere(
    globals: &mut ShaderManagerGlobals,
    inputs: &mut TestSphereInputs<'_>,
) -> TestSphereStatus {
    // 004814C0 007B7650; 004814C5 a null accumulator returns (never, as the singleton's request cannot fail here).
    let accumulator = shader_accumulator_singleton(globals, inputs.max_texture_blend_stages);
    // 004814CD cmp [eax+2228h], 0.
    if accumulator.dword_2228 != 0 {
        return TestSphereStatus::AccumulatorBusy;
    }
    // 004814DA cmp 00B33A1C, 0.
    if !inputs.model_loader {
        return TestSphereStatus::NoModelLoader;
    }
    // 004814E3 g_FileFinder.
    if !inputs.file_finder {
        return TestSphereStatus::NoFileFinder;
    }
    // 004814ED..004814FD vt+4 FindFile("Meshes\\TestSphere.NIF", 0, 0, -1).
    if !inputs
        .find_file
        .as_mut()
        .is_some_and(|find| find("Meshes\\TestSphere.NIF"))
    {
        return TestSphereStatus::NotFound;
    }
    // 00481503..0048155A: the NIF branch (carried).
    TestSphereStatus::NifBranchCarried
}
