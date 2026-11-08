//! IDA: N/A a DataHandler over a working directory's `Data\`, with a default TES and the GMST store, ready for 0044F3D0. Like the C++ `Loader`: the warning, log and progress recorders, and a form arm filter. The stand-ins are harness substitutions, and only what a run recorded is checked against one.

use std::path::Path;

use bevy_ecs::prelude::*;
use recomp_core::crt::MsvcRand;
use recomp_core::reader::StreamError;
use recomp_game::bs::archive::ArchiveManager;
use recomp_game::bs::file_finder::{FileFinder, bs_file_open_core};
use recomp_game::bs::scene_graphs::{SceneGraphSettings, construct_scene_graph};
use recomp_game::gamebryo::scene_update::NoSceneControllers;
use recomp_game::tes::actor_process::{
    CrtRandomSource, ProcessInputs, construct_actor_process_manager,
};
use recomp_game::tes::character_controller::ControllerInputs;
use recomp_game::tes::data_handler::{
    DataHandler, build_data_handler_file_list, construct_data_handler,
};
use recomp_game::tes::data_load::{
    DataLoadGlobals, DataLoadServices, LoadSelectedFiles, load_selected_files,
};
use recomp_game::tes::default_forms::create_default_forms;
use recomp_game::tes::facegen::{FaceGenMatrix, FaceGenSingleton, FaceGenState};
use recomp_game::tes::form_arms::{AbsentArm, DataLoadHost};
use recomp_game::tes::form_components::TextureFileEntry;
use recomp_game::tes::forms_magic::construct_effect_setting_collection;
use recomp_game::tes::game_settings::GameSettingCollection;
use recomp_game::tes::player::{PlayerGlobals, PlayerInputs, PlayerSettingInputs};
use recomp_game::tes::tes::{GameThreadLocals, Tes, TesGlobals, TesSaveLoadGame};
use recomp_game::tes::tes_file::TesFileData;
use recomp_startup::initialize_game_data::run_player_window;

/// IDA: N/A the test host. `pre_load` stands in for 0044AC20 and the tail is skipped, unless `real_pre_load` and `real_tail` say otherwise; absent classes count and skip.
pub struct TestHost {
    pub logs: Vec<String>,
    pub progress: Vec<(i32, f32)>,
    pub pre_load_calls: u32,
    pub post_load_calls: u32,
    pub pre_load: fn(&mut World),
    /// 0044AC20 itself, after `pre_load`.
    pub real_pre_load: bool,
    /// 0044F780..0044FA38 itself.
    pub real_tail: bool,
    /// A form type this filter refuses behaves as absent.
    pub arm_filter: fn(u32) -> bool,
    /// 0042DB10's archives; `None` is the trait default, no archive.
    pub archives: Option<ArchiveManager>,
    /// What 00431690 opens through, `None` for no file.
    pub finder: Option<FileFinder>,
    /// 0047DF80's CRT. No row sees a roll: the seed is a harness value.
    pub rand: MsvcRand,
}

impl Default for TestHost {
    fn default() -> Self {
        Self {
            logs: Vec::new(),
            progress: Vec::new(),
            pre_load_calls: 0,
            post_load_calls: 0,
            pre_load: |_| {},
            real_pre_load: false,
            real_tail: false,
            arm_filter: |_| true,
            archives: None,
            finder: None,
            rand: MsvcRand::default(),
        }
    }
}

impl DataLoadHost for TestHost {
    fn debug_log(&mut self, text: String) {
        self.logs.push(text);
    }
    fn section_progress(&mut self, section: i32, percent: f32) {
        self.progress.push((section, percent));
    }
    fn run_pre_load_pass(&mut self, world: &mut World) -> bool {
        self.pre_load_calls += 1;
        (self.pre_load)(world);
        self.real_pre_load
    }
    fn run_post_load_passes(&mut self, _world: &mut World) -> bool {
        self.post_load_calls += 1;
        self.real_tail
    }
    fn absent_arm(&mut self, _form_type: u32) -> AbsentArm {
        AbsentArm::CountAndSkip
    }
    fn arm_enabled(&mut self, form_type: u32) -> bool {
        (self.arm_filter)(form_type)
    }
    fn find_texture_entry(&mut self, folder_hash: u64, file_hash: u64) -> TextureFileEntry {
        self.archives
            .as_mut()?
            .file_record_by_hashes(1, folder_hash, file_hash)?;
        Some((folder_hash, file_hash))
    }
    fn find_file(&mut self, path: &str, flags: u32, mask: i32) -> i32 {
        self.finder.as_ref().map_or(0, |finder| {
            finder
                .find_file(path, flags, mask, self.archives.as_mut())
                .status
        })
    }
    fn apply_archive_cache_settings(&mut self) {
        if let Some(archives) = self.archives.as_mut() {
            archives.apply_cache_settings();
        }
    }
    fn find_matching_files(&mut self, pattern: &str, source: &str, mask: u32) -> Vec<String> {
        self.archives.as_mut().map_or_else(Vec::new, |archives| {
            archives.find_matching_files(pattern, source, mask)
        })
    }
    fn random_next(&mut self) -> Option<i32> {
        Some(self.rand.rand())
    }
    fn read_data_file(&mut self, path: &str) -> Option<Vec<u8>> {
        // IDA: 00431690 -> 00430EF0(path, 0, 0x800, 0xFFFF).
        let finder = self.finder.as_ref()?;
        let mut file = bs_file_open_core(finder, self.archives.as_mut(), path, 0, 0x800, 0xFFFF)?;
        let mut bytes = vec![0; file.size() as usize];
        file.read(&mut bytes);
        Some(bytes)
    }
    fn warnings_log_path(&self) -> String {
        "<MyGames>Warnings.txt".to_owned()
    }
}

/// IDA: N/A the loader context of one test.
pub struct Loader {
    pub world: World,
    pub working_directory: std::path::PathBuf,
    pub warnings: Vec<String>,
    pub host: TestHost,
}

impl Loader {
    /// IDA: N/A 00446D80 and 0044A2B0 over `working_directory`, with g_disableWarningMessages 1 as in P12-RUN-001.
    pub fn new(working_directory: &Path) -> Self {
        let mut world = World::new();
        world.init_resource::<GameThreadLocals>();
        construct_data_handler(&mut world).unwrap();
        let mut warnings = Vec::new();
        build_data_handler_file_list(&mut world, "Data\\", working_directory, &mut |text| {
            warnings.push(text)
        })
        .unwrap();
        world.insert_resource(TesGlobals {
            tes: Some(Box::new(Tes::default())),
            // 0046BB20 reads +0x18, which is clear in P11-RUN-001.
            save_load_game: Some(TesSaveLoadGame::default()),
            ..TesGlobals::default()
        });
        world.init_resource::<GameSettingCollection>();
        // 00B3BD00, which the ACHR and ACRE constructors add to.
        construct_actor_process_manager(&mut world);
        // The pre-WinMain 00B33508, and 00B39B80 with the coefficient sizes of the shipped si.ctl (50, 30, 50, none), a harness stand-in for Phase 04's, whose floats a load zeroes.
        construct_effect_setting_collection(&mut world).unwrap();
        let mut facegen = Box::<FaceGenState>::default();
        for (matrix, rows) in facegen.coefficients.iter_mut().zip([50_u32, 30, 50]) {
            *matrix = FaceGenMatrix {
                rows,
                cols: 1,
                data: vec![0.0; rows as usize],
            };
        }
        world.insert_resource(FaceGenSingleton(Some(facegen)));
        world.insert_resource(DataLoadGlobals {
            disable_warning_messages_b06b18: 1,
            ..DataLoadGlobals::default()
        });
        Self {
            world,
            working_directory: working_directory.to_owned(),
            warnings,
            host: TestHost::default(),
        }
    }

    /// IDA: N/A every listed file Loaded, as the run's Plugins.txt leaves them.
    pub fn load_all(&mut self) {
        let mut data = self.world.resource_mut::<DataHandler>();
        let list = data.files_8c8.clone();
        for id in list {
            data.file_store.get_mut(id).unwrap().set_loaded(true);
        }
    }

    pub fn file(&self, name: &str) -> &TesFileData {
        let data = self.world.resource::<DataHandler>();
        data.files_8c8
            .iter()
            .map(|&id| data.file_store.get(id).unwrap())
            .find(|file| file.name_1c == name)
            .unwrap_or_else(|| panic!("no file {name}"))
    }

    /// IDA: N/A 0044F3D0(0, 0) straight through.
    pub fn run(&mut self) -> Result<u32, StreamError> {
        let warnings = &mut self.warnings;
        let mut warning = |text: String, disabled: bool| {
            warnings.push(if disabled {
                format!("[disabled] {text}")
            } else {
                text
            })
        };
        load_selected_files(
            &mut self.world,
            false,
            false,
            &mut DataLoadServices {
                working_directory: &self.working_directory,
                warning: &mut warning,
                host: &mut self.host,
            },
        )
    }

    /// IDA: N/A 0044F3D0(0, 0) up to its tail, for the rows of that moment. `finish` runs the rest.
    pub fn run_to_tail(&mut self) -> Result<LoadSelectedFiles, StreamError> {
        let warnings = &mut self.warnings;
        let mut warning = |text: String, disabled: bool| {
            warnings.push(if disabled {
                format!("[disabled] {text}")
            } else {
                text
            })
        };
        let mut services = DataLoadServices {
            working_directory: &self.working_directory,
            warning: &mut warning,
            host: &mut self.host,
        };
        let mut cursor = LoadSelectedFiles::new(false, false);
        while !cursor.at_tail() {
            if cursor.step(&mut self.world, &mut services)?.is_some() {
                break;
            }
        }
        Ok(cursor)
    }

    /// IDA: N/A the rest of a cursor's 0044F3D0.
    pub fn finish(&mut self, mut cursor: LoadSelectedFiles) -> Result<u32, StreamError> {
        let warnings = &mut self.warnings;
        let mut warning = |text: String, disabled: bool| {
            warnings.push(if disabled {
                format!("[disabled] {text}")
            } else {
                text
            })
        };
        cursor.run(
            &mut self.world,
            &mut DataLoadServices {
                working_directory: &self.working_directory,
                warning: &mut warning,
                host: &mut self.host,
            },
        )
    }

    /// IDA: N/A the same, a step at a time.
    pub fn run_stepped(&mut self) -> Result<(u32, u64), StreamError> {
        let warnings = &mut self.warnings;
        let mut warning = |text: String, disabled: bool| {
            warnings.push(if disabled {
                format!("[disabled] {text}")
            } else {
                text
            })
        };
        let mut services = DataLoadServices {
            working_directory: &self.working_directory,
            warning: &mut warning,
            host: &mut self.host,
        };
        let mut cursor = LoadSelectedFiles::new(false, false);
        let mut steps = 0;
        loop {
            steps += 1;
            if let Some(result) = cursor.step(&mut self.world, &mut services)? {
                return Ok((result, steps));
            }
        }
    }
}

/// IDA: 0044AC20 itself, with the player its caller's window has built first and TLS +0x184 set, as 0044F3F9 leaves it.
pub fn pre_load_default_globals(world: &mut World) {
    install_player(world);
    world.resource_mut::<GameThreadLocals>().byte_184 = 1;
    create_default_forms(world).unwrap();
}

/// IDA: 00406774..004067B9 the Phase 11 block, through `run_player_window`: 0066A740, g_thePlayer and 0046C300(0x14, 1). A world with a player keeps it. The GMST and INI values are host defaults and the tick, time and rand are harness values: `phase11_player.rs` checks those against P11-RUN-001, and P12-RUN-001's AC20 32 row shows only the head. TLS +0x184 is the window's own, clear.
pub fn install_player(world: &mut World) {
    if world
        .get_resource::<PlayerGlobals>()
        .is_some_and(|globals| globals.the_player_b333c4.is_some())
    {
        return;
    }
    let saved = world.resource::<GameThreadLocals>().byte_184;
    world.resource_mut::<GameThreadLocals>().byte_184 = 0;
    let settings = SceneGraphSettings {
        renderer_width: 1920,
        renderer_height: 1080,
        default_fov: 75.0,
        ..SceneGraphSettings::default()
    };
    let graph = construct_scene_graph(
        world,
        b"World",
        false,
        None,
        &settings,
        &mut NoSceneControllers,
    )
    .unwrap();
    let mut tick = || 0;
    let mut time = || 0;
    let mut runtime = NoSceneControllers;
    let mut random = CrtRandomSource {
        rand: MsvcRand::default(),
        time64: &mut || 0,
    };
    let mut inputs = PlayerInputs {
        process: ProcessInputs {
            random: &mut random,
            idle_chatter_comment_timer: 0,
            low_process_unset_88: 0,
            save_load_active: false,
        },
        controller: ControllerInputs::default(),
        settings: PlayerSettingInputs::default(),
        world_scene_graph: Some(graph),
        scene_settings: &settings,
        scene_runtime: &mut runtime,
        get_tick_count: &mut tick,
        time64: &mut time,
    };
    run_player_window(world, &mut inputs, false, &mut |_| {}).unwrap();
    world.resource_mut::<GameThreadLocals>().byte_184 = saved;
}
