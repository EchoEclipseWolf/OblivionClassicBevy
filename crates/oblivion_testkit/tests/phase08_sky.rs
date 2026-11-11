//! Phase 08: the sky prelude 00406390..004065FF on the shipped NIFs, read from the BSAs through the ported archives and stream, against the P08-RUN-001 dump
//! (HDR, precipitation on, g_lodRoot present) and P08-RUN-002's arrays: the Sky and its parts, the NIF trees, every property state slot, the SkyShaderProperty pass
//! kinds, the sun light, the shadow scene light and the LOD branch. Then the failure arms (missing NIFs, no weather node, precipitation off, HDR off, no g_lodRoot),
//! which only the C++ port has exercised. Host tests earn no validation bonus.
//!
//! `phase08.rows` is the imported `.inc`, and the manifest pins it, so the values the C++ test held by hand are in `phase08_run002.rows`.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;
use oblivion_core::fixtures;
use oblivion_sim::bs::archive::{ArchiveManager, ArchiveSettings};
use oblivion_sim::bs::file::{GameFile, OpenedFile};
use oblivion_sim::bs::file_finder::{
    BsFileCallbacks, FileFinder, bs_file_exists, bs_file_open_core,
};
use oblivion_sim::bs::scene_graphs::{SceneGraphGlobals, ShadowSceneNode};
use oblivion_sim::bs::shader::BsShaderSlot;
use oblivion_sim::bs::shader_classes_geometry::SkyShader;
use oblivion_sim::bs::sky_shader::{
    NiGeometryShader, PrepareGeometryContext, SKY_SHADER_SLOT_INDEX, SKY_SHADER_VTABLE,
};
use oblivion_sim::bs::stream::{BsStreamLoad, BsStreamServices, load_bs_stream_from_path};
use oblivion_sim::gamebryo::av_object::NiAvProperties;
use oblivion_sim::gamebryo::av_object::find_property_of_type;
use oblivion_sim::gamebryo::mesh_geometry::{
    MeshGeometryData, MeshTriShapeData, MeshTriShapePayload, NiGeometry, geometry_data_base,
};
use oblivion_sim::gamebryo::mesh_tri_strips::MeshTriStripsData;
use oblivion_sim::gamebryo::object_net::{AlphaProperty, NiObjectNet};
use oblivion_sim::gamebryo::properties::{
    MaterialProperty, SkyShaderProperty, VertexColorProperty, WireframeProperty, ZBufferProperty,
};
use oblivion_sim::gamebryo::registration::{
    NiFactoryRegistry, NiProcessDefaults, StartupCallbackScheduler, register_known_crt_callbacks,
};
use oblivion_sim::gamebryo::scene_derived::{BillboardNode, DirectionalLight};
use oblivion_sim::gamebryo::scene_node::{
    NiAvWorld, NiEffectAffectedNodes, NiNodeChildren, NiNodeEffects, SceneNodePayload,
};
use oblivion_sim::gamebryo::scene_state::{ScenePropertyDefaults, ScenePropertyState};
use oblivion_sim::gamebryo::scene_update::NoSceneControllers;
use oblivion_sim::gamebryo::search_path::bs_search_path_candidates;
use oblivion_sim::gamebryo::stream_objects::NiClass;
use oblivion_sim::gamebryo::stream_postload::StreamPostLoadRuntime;
use oblivion_sim::gamebryo::texture::{SourceTextureLoadServices, TexturingProperty};
use oblivion_sim::gamebryo::transform::NiTransform;
use oblivion_sim::tes::game_settings::GameSettingCollection;
use oblivion_sim::tes::settings::{SettingCollections, register_static_settings};
use oblivion_sim::tes::sky::{SkyGlobals, SkyInitServices};
use oblivion_sim::tes::winmain::initialize_game_data::{
    SkyPreludeInputs, SkyPreludeResult, run_sky_prelude,
};
use oblivion_sim::tes::winmain::initialize_scene_graphs::{
    SceneGraphInputs, initialize_scene_graphs,
};

fn hex8(value: u32) -> String {
    format!("{value:08X}")
}

fn hex4(value: u16) -> String {
    format!("{value:04X}")
}

fn bits(values: &[f32]) -> String {
    values
        .iter()
        .map(|value| hex8(value.to_bits()))
        .collect::<Vec<_>>()
        .join(",")
}

fn transform_bits(transform: &NiTransform) -> String {
    format!(
        "{},{},{}",
        bits(&transform.rotation),
        bits(&transform.translation),
        bits(&[transform.scale])
    )
}

fn name_of(world: &World, object: Entity) -> String {
    world
        .get::<NiObjectNet>(object)
        .and_then(|net| net.name.as_deref())
        .map_or("<null>".to_owned(), |name| {
            String::from_utf8_lossy(name).into_owned()
        })
}

fn le_f32(values: impl IntoIterator<Item = f32>) -> Vec<u8> {
    values
        .into_iter()
        .flat_map(|value| value.to_bits().to_le_bytes())
        .collect()
}

fn hex_string(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn fnv1a(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0x811C_9DC5, |hash, &byte| {
        (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193)
    })
}

/// IDA: N/A what stays the same across prelude runs: the files, the factories and the pinned INI.
struct Harness {
    root: PathBuf,
    callbacks: BsFileCallbacks,
    finder: FileFinder,
    archives: RefCell<ArchiveManager>,
    registry: NiFactoryRegistry,
    collections: SettingCollections,
}

impl Harness {
    fn new() -> Self {
        let data = fixtures::data_dir();
        assert!(
            data.join("Oblivion - Meshes.bsa").exists(),
            "this test needs the game data: set OBLIVION_DATA_DIR ({data:?})"
        );
        let root = data.parent().unwrap().canonicalize().unwrap();
        let mut callbacks = BsFileCallbacks::default();
        let mut finder = FileFinder::new(&root, &mut callbacks);
        finder.add("Data");
        let settings = ArchiveSettings {
            invalidate_older_files: false,
            retain_directory_strings: 2,
            ..ArchiveSettings::default()
        };
        // As in `phase05_window`, what 0040E6AD left: the archive list from the settings, which also fills the type table 0xFFFF reads.
        let scratch = std::env::temp_dir().join(format!(
            "recomp_phase08_{}",
            std::thread::current()
                .name()
                .unwrap_or("x")
                .replace("::", "_")
        ));
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).unwrap();
        std::fs::write(scratch.join("Plugins.txt"), "Oblivion.esm\n").unwrap();
        let mut archives = ArchiveManager::new(&root, settings);
        archives.initialize_from_settings(&finder, &scratch);
        let _ = std::fs::remove_dir_all(&scratch);
        let mut registry = NiFactoryRegistry::default();
        registry.register_known_crt_factories();
        registry.register_winmain_bs_factories();
        let mut collections = SettingCollections::default();
        register_static_settings(&mut collections);
        collections
            .ini
            .set_path(fixtures::root().join("config/Oblivion.ini"));
        assert!(collections.ini.open_ini());
        collections.ini.read_all();
        collections.ini.close();
        Self {
            root,
            callbacks,
            finder,
            archives: RefCell::new(archives),
            registry,
            collections,
        }
    }

    fn setting_raw(&self, address: u32) -> u32 {
        self.collections.ini.find_address(address).unwrap().raw
    }
}

/// IDA: N/A 007023E0's services: no BSTexturePalette, so lookups miss and registrations drop. Existence goes through the ported archives.
struct NoPalette<'a>(&'a Harness);

impl SourceTextureLoadServices for NoPalette<'_> {
    fn search_candidates(&mut self, file_path: &[u8]) -> Vec<Vec<u8>> {
        bs_search_path_candidates(&String::from_utf8_lossy(file_path))
            .into_iter()
            .map(String::into_bytes)
            .collect()
    }
    fn find_texture(&mut self, _path: &[u8]) -> Option<Entity> {
        None
    }
    fn register_texture(&mut self, _path: &[u8], _texture: Entity) {}
    fn file_exists(&mut self, path: &[u8]) -> bool {
        let path: String = path.iter().map(|&byte| char::from(byte)).collect();
        bs_file_exists(
            &self.0.callbacks,
            &self.0.root,
            Some(&self.0.finder),
            Some(&mut self.0.archives.borrow_mut()),
            &path,
            0,
        )
    }
}

fn boxed(file: OpenedFile) -> Box<dyn GameFile> {
    match file {
        OpenedFile::Ni(file) => Box::new(file),
        OpenedFile::Loose(file) => Box::new(file),
        OpenedFile::Packed(file) => Box::new(file),
        OpenedFile::Compressed(file) => Box::new(file),
    }
}

/// IDA: N/A 006F9980 over the harness's files, or a null open slot.
fn load_stream(
    harness: &Harness,
    meshes: bool,
    world: &mut World,
    path: &str,
) -> Result<BsStreamLoad, GameError> {
    let mut open = |path: &str, mode: i32, size: u32| {
        if !meshes {
            return None;
        }
        bs_file_open_core(
            &harness.finder,
            Some(&mut harness.archives.borrow_mut()),
            path,
            mode,
            size,
            0xFFFF,
        )
        .map(boxed)
    };
    let mut rand = || 0;
    let mut textures = NoPalette(harness);
    load_bs_stream_from_path(
        world,
        path,
        &mut BsStreamServices {
            open: &mut open,
            registry: &harness.registry,
            draw_rand: Some(&mut rand),
            draw_on_io_thread: false,
            source_textures: Some(&mut textures),
            post_load: StreamPostLoadRuntime::default(),
        },
    )
}

#[derive(Clone, Copy)]
struct Options {
    hdr: bool,
    precipitation: bool,
    weather: bool,
    lod_root: bool,
    meshes: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            hdr: true,
            precipitation: true,
            weather: true,
            lod_root: true,
            meshes: true,
        }
    }
}

/// IDA: N/A one prelude run over freshly built Phase 05 scene graphs.
struct PreludeRun {
    options: Options,
    world: World,
    scenes: SceneGraphGlobals,
    defaults: ScenePropertyDefaults,
    outcome: Result<SkyPreludeResult, GameError>,
    warnings: Vec<&'static str>,
    shader_warnings: Vec<&'static str>,
    sky_shader_trace: Vec<Entity>,
    log: Vec<&'static str>,
}

impl PreludeRun {
    fn new(harness: &Harness, options: Options) -> Self {
        let mut world = World::new();
        let mut known = StartupCallbackScheduler::default();
        register_known_crt_callbacks(&mut known).unwrap();
        known.run(&mut world).unwrap();
        world.insert_resource(GameSettingCollection::default());
        let defaults = world
            .resource::<NiProcessDefaults>()
            .scene_property_defaults();
        let mut scenes = SceneGraphGlobals::default();
        let mut runtime = NoSceneControllers;
        initialize_scene_graphs(
            &mut world,
            &mut scenes,
            &mut SceneGraphInputs {
                ini: &harness.collections.ini,
                renderer_width: 1920,
                renderer_height: 1080,
                render_path: 5,
                store_shader_flags: Box::new(|_| {}),
                renderer_present: true,
                image_space_effects: 1,
                lod_far_mode: false,
                tes_present: false,
                initialize_shader_manager: None,
                create_render_targets: None,
                load_test_sphere: None,
                property_defaults: Some(defaults),
                fallback_base_texture: None,
            },
            &mut runtime,
        )
        .unwrap();
        if !options.weather {
            scenes.weather = None;
        }
        if !options.lod_root {
            scenes.lod_root = None;
        }
        // The SkyShader slot (index 10) the Phase 05 manager created.
        let mut shader = SkyShader::default();
        shader.base.original_vtable = SKY_SHADER_VTABLE;
        shader.base.class_name = "SkyShader";
        let slot = Arc::new(BsShaderSlot {
            declaration: None,
            shader: Some(Arc::new(shader)),
        });
        let mut get_shader =
            |index: u32| (index == SKY_SHADER_SLOT_INDEX).then(|| Arc::clone(&slot));
        let mut prepare = PrepareGeometryContext {
            render_path: 5,
            hdr: options.hdr,
            reference_classifier: scenes.reference_classifier,
            property_defaults: defaults,
            get_shader: &mut get_shader,
            warnings: Vec::new(),
            sky_shader_trace: Vec::new(),
            water_shader_trace: Vec::new(),
            lava_b47844: 0,
            shaders_30_b42ea5: false,
            load_normal_map: None,
            unported_shader_setup: None,
            classify_trace: Vec::new(),
        };
        let mut load =
            |world: &mut World, path: &str| load_stream(harness, options.meshes, world, path);
        let mut init = SkyInitServices {
            renderer_hdr: options.hdr,
            precipitation: options.precipitation && harness.setting_raw(0x00B1_1DE4) & 0xFF != 0,
            sun_base_size: f32::from_bits(harness.setting_raw(0x00B1_1E2C)),
            sun_glare_size: f32::from_bits(harness.setting_raw(0x00B1_1E34)),
            world_scene: scenes.world,
            weather: scenes.weather,
            load_stream: &mut load,
            warnings: Vec::new(),
        };
        let mut log = Vec::new();
        let mut log_sink = |text: &'static str| log.push(text);
        let outcome = run_sky_prelude(
            &mut world,
            SkyPreludeInputs {
                scenes: &scenes,
                init: &mut init,
                prepare: &mut prepare,
                property_defaults: defaults,
                update: &mut runtime,
                log: &mut log_sink,
            },
        );
        let warnings = init.warnings;
        let shader_warnings = prepare.warnings;
        let sky_shader_trace = prepare.sky_shader_trace;
        Self {
            options,
            world,
            scenes,
            defaults,
            outcome,
            warnings,
            shader_warnings,
            sky_shader_trace,
            log,
        }
    }

    fn globals(&self) -> &SkyGlobals {
        self.world.resource::<SkyGlobals>()
    }
}

/// IDA: N/A prints the host state in the gen_expected.py line format. Names come from the walk over the explicit child and property slots, so no query order is seen.
struct Printer<'r> {
    run: &'r PreludeRun,
    names: HashMap<Entity, String>,
    default_properties: HashSet<Entity>,
    printed: HashSet<Entity>,
    sun_data_id: Option<u16>,
}

impl<'r> Printer<'r> {
    fn new(run: &'r PreludeRun) -> Self {
        let d = &run.defaults;
        let default_properties = [
            d.alpha,
            d.material,
            d.renderer_specific,
            d.shade,
            d.stencil,
            d.texturing,
            d.vertex_color,
            d.wireframe,
            d.zbuffer,
        ]
        .into_iter()
        .flatten()
        .collect();
        let mut printer = Self {
            run,
            names: HashMap::new(),
            default_properties,
            printed: HashSet::new(),
            sun_data_id: None,
        };
        let scenes = &run.scenes;
        printer.names.insert(scenes.fog.unwrap(), "fog".to_owned());
        if let Some(lod) = scenes.lod_root {
            printer.names.insert(lod, "LODRoot".to_owned());
        }
        if let Some(objects) = scenes.object_lod_root {
            printer.names.insert(objects, "ObjectLODRoot".to_owned());
        }
        // Paths in the dump traversal order: node, its properties, children.
        printer.assign(scenes.sky, "sky");
        printer.assign(scenes.weather, "weather");
        printer
    }

    fn world(&self) -> &'r World {
        &self.run.world
    }

    fn assign(&mut self, object: Option<Entity>, path: &str) {
        let Some(object) = object else {
            return;
        };
        self.names.entry(object).or_insert_with(|| path.to_owned());
        let world = self.world();
        if let Some(list) = world.get::<NiAvProperties>(object) {
            for (index, property) in list.0.iter().enumerate() {
                if let Some(property) = property {
                    self.names
                        .entry(*property)
                        .or_insert_with(|| format!("{path}.p{index}"));
                }
            }
        }
        if let Some(children) = world.get::<NiNodeChildren>(object) {
            for (index, child) in children.slots_ac.iter().enumerate() {
                self.assign(*child, &format!("{path}.{index}"));
            }
        }
    }

    fn sym(&self, identity: Option<Entity>) -> String {
        identity.map_or("null".to_owned(), |entity| {
            self.names
                .get(&entity)
                .cloned()
                .unwrap_or_else(|| "?".to_owned())
        })
    }

    fn slot_symbol(&self, slot: Option<Entity>) -> String {
        if slot.is_some_and(|slot| self.default_properties.contains(&slot)) {
            return "default".to_owned();
        }
        self.sym(slot)
    }

    fn lines(&mut self) -> Vec<String> {
        let run = self.run;
        let globals = run.globals();
        let sky = globals.singleton_b365c4.as_deref().unwrap();
        let result = run.outcome.as_ref().unwrap();
        let flag = |value: bool| u8::from(value).to_string();
        let mut out = Vec::new();
        out.push(format!(
            "S hdr={}/{} precip={} classifier={:X} lod_arm={} land_lod={} scene_root={}",
            bits(&[globals.hdr_b2c73c]),
            bits(&[globals.hdr_b2c740]),
            u8::from(run.options.precipitation),
            run.scenes.reference_classifier,
            if result.lod_branch {
                "taken"
            } else {
                "skipped"
            },
            result
                .land_lod
                .map_or("null".to_owned(), |node| name_of(self.world(), node)),
            self.sym(result.scene_root),
        ));
        let white = sky.colors_3c == [[1.0; 3]; 10] && sky.color_b4 == [1.0; 3];
        out.push(format!(
            "K sky singleton={} root={} moons={} climate={},{},{},{},{} parts={},{},{},{},{},{},{} colors={} c0={} c4={} d0={} d4={} d8={} dc={} sounds=list e4={} ec={} f0={} f4={} fc={} b100={}",
            if globals.published_b333b0 { "published" } else { "differs" },
            self.sym(sky.root_04),
            self.sym(sky.moons_root_08),
            sky.first_climate_0c,
            sky.first_weather_10,
            sky.weather_14,
            sky.weather_18,
            sky.weather_override_1c,
            flag(sky.atmosphere_20.is_some()),
            flag(sky.stars_24.is_some()),
            flag(sky.sun_28.is_some()),
            flag(sky.clouds_2c.is_some()),
            flag(sky.masser_30.is_some()),
            flag(sky.secunda_34.is_some()),
            flag(sky.precipitation_38.is_some()),
            if white { "white" } else { "other" },
            bits(&[sky.float_c0]),
            bits(&[sky.float_c4]),
            bits(&[sky.float_d0]),
            bits(&[sky.float_d4]),
            bits(&[sky.float_d8]),
            sky.dword_dc,
            bits(&[sky.float_e4]),
            sky.dword_ec,
            bits(&[sky.float_f0]),
            bits(&[sky.float_f4]),
            sky.dword_fc,
            sky.byte_100,
        ));
        let a = sky.atmosphere_20.as_ref().unwrap();
        out.push(format!(
            "K atmosphere vt={} root={} mesh={} fog={} quad_node={} quad={} b18={}",
            hex8(a.original_vtable),
            self.sym(a.root_04),
            self.sym(a.mesh_08),
            self.sym(a.fog_0c),
            self.sym(a.quad_node_10),
            self.sym(a.quad_14),
            a.byte_18,
        ));
        let stars = sky.stars_24.as_ref().unwrap();
        out.push(format!(
            "K stars vt={} root={} p08={} f0c={}",
            hex8(stars.original_vtable),
            self.sym(stars.root_04),
            self.sym(stars.pointer_08),
            bits(&[stars.float_0c]),
        ));
        let sun = sky.sun_28.as_ref().unwrap();
        out.push(format!(
            "K sun vt={} root={} billboard={} glare_billboard={} geometry={} glare_geometry={} pick={} light={} f20={} b24={}",
            hex8(sun.original_vtable),
            self.sym(sun.root_04),
            self.sym(sun.billboard_08),
            self.sym(sun.glare_billboard_0c),
            self.sym(sun.geometry_10),
            self.sym(sun.glare_geometry_14),
            if sun.pick_18.is_some() { "set" } else { "null" },
            self.sym(sun.light_1c),
            bits(&[sun.float_20]),
            sun.byte_24,
        ));
        let pick = sun.pick_18.as_ref().unwrap();
        out.push(format!(
            "K pick {},{},{},{},{},{} root={} vt={} data={} cap={} size={} count={} grow={} last={} ret={},{},{},{}",
            pick.pick_type_00,
            pick.sort_type_04,
            pick.intersect_type_08,
            pick.coordinate_type_0c,
            pick.front_only_10,
            pick.observe_app_cull_11,
            self.sym(pick.root_14),
            hex8(pick.results_vtable_18),
            pick.results_data_1c,
            pick.results_capacity_20,
            pick.results_size_22,
            pick.results_count_24,
            pick.results_grow_26,
            pick.last_record_28,
            pick.return_flags_2c[0],
            pick.return_flags_2c[1],
            pick.return_flags_2c[2],
            pick.return_flags_2c[3],
        ));
        let clouds = sky.clouds_2c.as_ref().unwrap();
        out.push(format!(
            "K clouds vt={} root={} layer0={} layer1={} p10={} p14={}",
            hex8(clouds.original_vtable),
            self.sym(clouds.root_04),
            self.sym(clouds.layers_08[0]),
            self.sym(clouds.layers_08[1]),
            self.sym(clouds.pointers_10[0]),
            self.sym(clouds.pointers_10[1]),
        ));
        if let Some(p) = &sky.precipitation_38 {
            out.push(format!(
                "K precipitation vt={} p04={} p08={} weather={} p10={} p14={}",
                hex8(p.original_vtable),
                self.sym(p.pointer_04),
                self.sym(p.pointer_08),
                self.sym(p.weather_0c),
                p.unknown_10,
                p.unknown_14,
            ));
        }
        out.push(self.shadow_line());
        self.walk(run.scenes.sky, "sky", &mut out);
        self.walk(run.scenes.weather, "weather", &mut out);
        out
    }

    fn shadow_line(&self) -> String {
        let node = self
            .world()
            .get::<ShadowSceneNode>(self.run.scenes.shadow_slots[0].unwrap())
            .unwrap();
        let l = node.light_118.as_ref().unwrap();
        let planes_zero = l
            .planes_150
            .iter()
            .all(|plane| plane.normal == [0.0; 3] && plane.constant == 0.0);
        let zero_1b4 = l.zero_1b4.iter().all(|&value| value == 0);
        format!(
            "H light118 vt={} matrix={} d0={} dc={} e0={} list_e4=00A91294/{} f4={} f5={} f8={} fc={} light={} 104={} 108={} 114={} 118={} 11c={} 120={} 124={} 128={} 12c={} 130={} list_134=00A912A4/{} 144={} 148={} 14c={} planes={} 1b0={} 1b4={} 214={}",
            hex8(l.original_vtable),
            bits(&l.matrix_10),
            bits(&[l.last_contribution_d0, l.unknown_d4[0], l.unknown_d4[1]]),
            bits(&[l.unknown_dc]),
            bits(&[l.unknown_e0]),
            l.geometry_list_e4.len(),
            l.byte_f4,
            l.byte_f5,
            l.pointer_f8,
            l.is_point_light_fc,
            self.sym(l.light_100),
            l.byte_104,
            bits(&l.point_108),
            l.pointer_114,
            l.word_118,
            l.pointer_11c,
            l.byte_120,
            bits(&[l.float_124]),
            bits(&[l.float_128]),
            l.byte_12c,
            l.pointer_130,
            l.object_list_count_134,
            u8::from(l.geometry_refresh_fence_144.is_some()),
            u8::from(l.geometry_fence_marker_148.is_some()),
            l.pointer_14c,
            if planes_zero { "zero" } else { "set" },
            l.active_planes_1b0,
            if zero_1b4 { "zero" } else { "set" },
            l.byte_214,
        )
    }

    fn property_lines(&mut self, object: Entity, path: &str, out: &mut Vec<String>) {
        let world = self.world();
        let Some(list) = world.get::<NiAvProperties>(object) else {
            return;
        };
        for (index, property) in list.0.iter().enumerate() {
            let property = property.unwrap();
            let tag = format!("{path}.p{index}");
            if !self.printed.insert(property) {
                out.push(format!("P {tag} ref={}", self.sym(Some(property))));
                continue;
            }
            let name = name_of(world, property);
            let body = if let Some(alpha) = world.get::<AlphaProperty>(property) {
                format!(
                    "NiAlphaProperty name={name} flags={} ref={}",
                    hex4(alpha.flags_18),
                    alpha.alpha_test_ref_1a
                )
            } else if let Some(vc) = world.get::<VertexColorProperty>(property) {
                format!(
                    "NiVertexColorProperty name={name} flags={}",
                    hex4(vc.flags_18)
                )
            } else if let Some(z) = world.get::<ZBufferProperty>(property) {
                format!("NiZBufferProperty name={name} flags={}", hex4(z.flags_18))
            } else if let Some(wire) = world.get::<WireframeProperty>(property) {
                format!(
                    "NiWireframeProperty name=<null> flags={}",
                    hex4(wire.flags_18)
                )
            } else if let Some(texturing) = world.get::<TexturingProperty>(property) {
                format!(
                    "NiTexturingProperty name={name} flags={}",
                    hex4(texturing.flags_18)
                )
            } else if let Some(material) = world.get::<MaterialProperty>(property) {
                let v = &material.values;
                format!(
                    "NiMaterialProperty name={name} colors={},{},{},{} shine={} alpha={} 54={} 58={}",
                    bits(&v.ambient),
                    bits(&v.diffuse),
                    bits(&v.specular),
                    bits(&v.emissive),
                    bits(&[v.shine]),
                    bits(&[v.alpha]),
                    material.raw_state_54,
                    material.raw_state_58,
                )
            } else if let Some(s) = world.get::<SkyShaderProperty>(property) {
                format!(
                    "SkyShaderProperty name=<null> flags={} pass_info={} alpha={} last={} lists={},{},{},{} classifier={} color={} blend={} value={} layer={} kind={}",
                    hex4(s.flags_18),
                    s.pass_info_1c,
                    bits(&[s.alpha_20]),
                    s.last_pass_state_24,
                    s.render_pass_counts[0],
                    s.render_pass_counts[1],
                    s.render_pass_counts[2],
                    s.render_pass_counts[3],
                    s.classifier_68,
                    bits(&s.color_6c),
                    self.sym(s.blend_texture_7c),
                    bits(&[s.blend_value_80]),
                    s.layer_84,
                    s.pass_kind_88,
                )
            } else {
                "?".to_owned()
            };
            out.push(format!("P {tag} {body}"));
        }
    }

    fn geometry_lines(&mut self, object: Entity, path: &str, out: &mut Vec<String>) {
        let world = self.world();
        let links = world.get::<NiGeometry>(object).unwrap();
        let state = world
            .get::<ScenePropertyState>(links.property_state_ac.unwrap())
            .unwrap();
        let mut q = format!("Q {path}");
        for slot in state.slots {
            q.push(' ');
            q.push_str(&self.slot_symbol(slot));
        }
        out.push(q);
        let data = links.data_b4.unwrap();
        let d: &MeshGeometryData = geometry_data_base(world, data).unwrap();
        let (class, triangles) = if let Some(shape) = world.get::<MeshTriShapeData>(data) {
            ("NiTriShapeData", shape.triangle_count_40)
        } else {
            (
                "NiTriStripsData",
                world
                    .get::<MeshTriStripsData>(data)
                    .unwrap()
                    .triangle_count_40,
            )
        };
        let mut id = d.data_id_0a.to_string();
        if class == "NiTriShapeData" && d.data_id_0a != 0 {
            let first = *self.sun_data_id.get_or_insert(d.data_id_0a);
            id = format!("+{}", d.data_id_0a - first);
        }
        let array = |empty: bool, letter: char| if empty { '-' } else { letter };
        let arrays: String = [
            array(d.positions_1c.is_empty(), 'p'),
            array(d.normals_20.is_empty(), 'n'),
            array(d.colors_24.is_empty(), 'c'),
            array(d.texture_coordinates_28.is_empty(), 't'),
        ]
        .iter()
        .collect();
        out.push(format!(
            "G {path} {class} count={} id={id} bound={} arrays={arrays} format={} dirty={} keep={} compress={} triangles={triangles}",
            d.vertex_count_08,
            bits(&d.bounds_0c),
            hex4(d.geometry_flags_2c),
            hex4(d.dirty_flags_2e),
            d.keep_flags_30,
            d.compress_flags_31,
        ));
    }

    fn walk(&mut self, object: Option<Entity>, path: &str, out: &mut Vec<String>) {
        let Some(object) = object else {
            return;
        };
        let world = self.world();
        let av = world.get::<NiAvWorld>(object).unwrap();
        let props = world
            .get::<NiAvProperties>(object)
            .map_or(0, |list| list.0.len());
        let name = name_of(world, object);
        match *world.get::<NiClass>(object).unwrap() {
            class @ (NiClass::Node | NiClass::BillboardNode) => {
                let payload = world.get::<SceneNodePayload>(object).unwrap();
                let children = &world.get::<NiNodeChildren>(object).unwrap().slots_ac;
                let effects = &world.get::<NiNodeEffects>(object).unwrap().0;
                let mut line = format!(
                    "A {path} {} name={name} flags={} bound={} local={} world={} children={} effects={}",
                    if class == NiClass::BillboardNode {
                        "NiBillboardNode"
                    } else {
                        "NiNode"
                    },
                    hex4(payload.effective_av_flags),
                    bits(&av.bound),
                    transform_bits(&payload.local_transform),
                    transform_bits(&av.transform),
                    children.len(),
                    effects.len(),
                );
                if let Some(billboard) = world.get::<BillboardNode>(object) {
                    line.push_str(&format!(
                        " mode={} e0={}",
                        hex4(billboard.effective_mode_flags_dc),
                        bits(&[billboard.unknown_e0])
                    ));
                }
                line.push_str(&format!(" props={props}"));
                out.push(line);
                self.property_lines(object, path, out);
                for (index, effect) in effects.iter().enumerate() {
                    out.push(format!(
                        "E {path} effect{index}={}",
                        self.sym(Some(*effect))
                    ));
                }
                for (index, child) in children.iter().enumerate() {
                    self.walk(*child, &format!("{path}.{index}"), out);
                }
            }
            class @ (NiClass::TriShape | NiClass::TriStrips) => {
                let payload = world.get::<MeshTriShapePayload>(object).unwrap();
                let links = world.get::<NiGeometry>(object).unwrap();
                let set = |present: bool| if present { "set" } else { "null" };
                out.push(format!(
                    "A {path} {} name={name} flags={} bound={} local={} world={} state={} estate={} shader={} props={props}",
                    if class == NiClass::TriShape { "NiTriShape" } else { "NiTriStrips" },
                    hex4(payload.effective_av_flags),
                    bits(&av.bound),
                    transform_bits(&payload.local_transform),
                    transform_bits(&av.transform),
                    set(links.property_state_ac.is_some()),
                    set(links.effect_state_b0.is_some()),
                    set(world.get::<NiGeometryShader>(object).is_some()),
                ));
                self.property_lines(object, path, out);
                self.geometry_lines(object, path, out);
            }
            NiClass::DirectionalLight => {
                let l = world.get::<DirectionalLight>(object).unwrap();
                let affected = &world.get::<NiEffectAffectedNodes>(object).unwrap().0;
                out.push(format!(
                    "A {path} NiDirectionalLight name={name} flags={} bound={} local={} world={} enable={} b4={} b8={} affected={} cc={} dimmer={} ambient={} diffuse={} specular={} 104={} direction={} props={props}",
                    hex4(l.payload.av.effective_av_flags),
                    bits(&av.bound),
                    transform_bits(&l.payload.av.local_transform),
                    transform_bits(&av.transform),
                    u8::from(l.enabled),
                    l.unknown_b4,
                    l.unknown_b8,
                    affected.len(),
                    l.stream_node_list_cc.len(),
                    bits(&[l.payload.dimmer]),
                    bits(&l.payload.ambient),
                    bits(&l.payload.diffuse),
                    bits(&l.payload.specular),
                    l.unknown_104,
                    bits(&l.direction_108),
                ));
                for (index, node) in affected.iter().enumerate() {
                    out.push(format!(
                        "E {path} affected{index}={}",
                        self.sym(Some(*node))
                    ));
                }
            }
            _ => out.push(format!("A {path} ?")),
        }
    }
}

/// Against the original: every line of `phase08.rows` (P08-RUN-001, final stage 004065FA), in order.
fn check_against_run(run: &PreludeRun) {
    run.outcome.as_ref().expect("the prelude ran");
    assert!(
        run.warnings.is_empty(),
        "P08-RUN-001 logged no warning in the window: {:?}",
        run.warnings
    );
    assert!(run.shader_warnings.is_empty());
    assert_eq!(run.log, ["Initializing Sky...", "Initializing TES..."]);
    // Compared with the C++ only: two inside 00544EA0, four on the sky tree, one on weather.
    assert_eq!(run.sky_shader_trace.len(), 7, "007BDAC0 ran seven times");
    let host = Printer::new(run).lines();
    let expected = fixtures::rows("phase08");
    assert_eq!(expected.len(), 55);
    let mut mismatches = Vec::new();
    for index in 0..host.len().max(expected.len()) {
        let left = expected.get(index).map_or("<missing>", String::as_str);
        let right = host.get(index).map_or("<missing>", String::as_str);
        if left != right {
            mismatches.push(format!("line {index}\n  run : {left}\n  host: {right}"));
        }
    }
    assert!(
        mismatches.is_empty(),
        "{} lines differ from P08-RUN-001:\n{}",
        mismatches.len(),
        mismatches.join("\n")
    );
    // Compared with the C++ only: the global stores in order.
    let trace: Vec<u32> = run
        .globals()
        .store_trace
        .iter()
        .map(|s| s.address)
        .collect();
    assert_eq!(trace, [0x00B3_65C4, 0x00B3_33B0, 0x00B2_C73C, 0x00B2_C740]);
}

/// The line of a run's records for one array, as bytes in hex.
fn record_hex(records: &str, tag: &str, field: &str) -> String {
    let wanted = format!(" {tag} {field}=");
    let line = records
        .lines()
        .find(|line| line.starts_with("G ") && line.contains(&wanted))
        .unwrap_or_else(|| panic!("no record for {tag} {field}"));
    let value = &line[line.find(&wanted).unwrap() + wanted.len()..];
    value.rsplit(' ').next().unwrap().to_owned()
}

fn data_of(world: &World, geometry: Entity) -> &MeshGeometryData {
    geometry_data_base(
        world,
        world.get::<NiGeometry>(geometry).unwrap().data_b4.unwrap(),
    )
    .unwrap()
}

/// Against the original: the sun quad (00544EE0..) and the cloud vertex colors (00541790), from P08-RUN-002 through `phase08_run002.rows`, and every row held
/// against the evidence it came from. `data_id` order is word_B27504: the sun data, then the glare data.
fn check_geometry_values(run: &PreludeRun) {
    let world = &run.world;
    let sky = run.globals().singleton_b365c4.as_deref().unwrap();
    let sun = sky.sun_28.as_ref().unwrap();
    let clouds = sky.clouds_2c.as_ref().unwrap();
    let sun_data = data_of(world, sun.geometry_10.unwrap());
    let glare_data = data_of(world, sun.glare_geometry_14.unwrap());
    assert_eq!(
        glare_data.data_id_0a,
        sun_data.data_id_0a + 1,
        "word_B27504 order"
    );
    let bytes =
        std::fs::read(fixtures::root().join("evidence/phase08-run002-2026-10-06.records.txt"))
            .unwrap();
    let records: String = bytes.iter().map(|&byte| char::from(byte)).collect();
    let triangles = |geometry: Entity| -> Vec<u8> {
        let data = world.get::<NiGeometry>(geometry).unwrap().data_b4.unwrap();
        world
            .get::<MeshTriShapeData>(data)
            .unwrap()
            .triangle_list_48
            .iter()
            .flat_map(|index| index.to_le_bytes())
            .collect()
    };
    let array = |data: &MeshGeometryData, field: &str| -> Vec<u8> {
        match field {
            "positions" => le_f32(data.positions_1c.iter().flatten().copied()),
            "normals" => le_f32(data.normals_20.iter().flatten().copied()),
            "colors" => le_f32(data.colors_24.iter().flatten().copied()),
            "uvs" => le_f32(data.texture_coordinates_28.iter().flatten().copied()),
            other => panic!("no array {other}"),
        }
    };
    let rows = fixtures::rows("phase08_run002");
    assert_eq!(rows.len(), 12);
    for row in &rows {
        let parts: Vec<&str> = row.split('|').collect();
        assert_eq!(parts[0], "P08-RUN-002", "{row}");
        match parts[1] {
            "G" => {
                let (geometry, data) = match parts[2] {
                    "sun" => (sun.geometry_10.unwrap(), sun_data),
                    "glare" => (sun.glare_geometry_14.unwrap(), glare_data),
                    other => panic!("no geometry {other}"),
                };
                assert_eq!(record_hex(&records, parts[2], parts[3]), parts[4], "{row}");
                let port = if parts[3] == "triangles" {
                    triangles(geometry)
                } else {
                    array(data, parts[3])
                };
                assert_eq!(hex_string(&port), parts[4], "{row}");
            }
            "FNV" => {
                let layer = match parts[2] {
                    "cloud0" => 0,
                    "cloud1" => 1,
                    other => panic!("no layer {other}"),
                };
                let port = array(data_of(world, clouds.layers_08[layer].unwrap()), "colors");
                assert_eq!(
                    hex_string(&port),
                    record_hex(&records, parts[2], "colors"),
                    "{row}: rgb (1, 0, 0) from 00541790, alpha from Clouds.nif"
                );
                assert_eq!(format!("{:08X}", fnv1a(&port)), parts[3], "{row}");
            }
            other => panic!("unknown row kind {other}"),
        }
    }
    // Against the original too, read from the evidence directly: the rest of each cloud layer's arrays, as Clouds.nif gave them.
    for (tag, layer) in [("cloud0", 0), ("cloud1", 1)] {
        let data = data_of(world, clouds.layers_08[layer].unwrap());
        for field in ["positions", "normals", "uvs"] {
            assert_eq!(
                hex_string(&array(data, field)),
                record_hex(&records, tag, field),
                "{tag} {field}"
            );
        }
    }
}

/// Compared with the C++ only: the failure arms of 0053B3F0, 0053BDB0, 005411D0 and 004065B5. No original run took them.
fn check_failure_arms(harness: &Harness) {
    {
        let run = PreludeRun::new(
            harness,
            Options {
                meshes: false,
                ..Options::default()
            },
        );
        run.outcome.as_ref().unwrap();
        assert_eq!(
            run.warnings,
            [
                "Cannot load the atmosphere model.",
                "Cannot load the clouds."
            ],
            "missing NIFs warn in 0053B61D and 0053BE9E order"
        );
        let sky = run.globals().singleton_b365c4.as_deref().unwrap();
        let a = sky.atmosphere_20.as_ref().unwrap();
        let c = sky.clouds_2c.as_ref().unwrap();
        assert!(a.root_04.is_some());
        assert_eq!(a.mesh_08, None);
        assert_eq!(
            a.fog_0c, None,
            "a failed atmosphere load skips the +0x0C fog store"
        );
        assert_eq!(c.layers_08, [None, None]);
    }
    {
        let run = PreludeRun::new(
            harness,
            Options {
                precipitation: false,
                ..Options::default()
            },
        );
        run.outcome.as_ref().unwrap();
        assert!(
            run.globals()
                .singleton_b365c4
                .as_deref()
                .unwrap()
                .precipitation_38
                .is_none(),
            "bPrecipitation 0 leaves +0x38 null"
        );
    }
    {
        // 004063D4 dereferences a null g_weatherNode after 005411D0.
        let run = PreludeRun::new(
            harness,
            Options {
                weather: false,
                ..Options::default()
            },
        );
        assert_eq!(
            run.outcome,
            Err(GameError::unclassified(
                0x004063D4,
                "004063D4: null g_weatherNode or sun"
            )),
            "a null g_weatherNode must stop at 004063D4"
        );
        assert_eq!(
            run.warnings,
            ["Can't find Weather Root Node.  Precipitation has no parent on scene graph."],
            "0053D8F0 warns for a null g_weatherNode"
        );
    }
    {
        let run = PreludeRun::new(
            harness,
            Options {
                lod_root: false,
                ..Options::default()
            },
        );
        let result = run.outcome.as_ref().unwrap();
        assert!(!result.lod_branch);
        assert_eq!(
            result.land_lod, None,
            "a null g_lodRoot skips 004065C1..004065F2 (esi = 0)"
        );
        let sky = run.globals().singleton_b365c4.as_deref().unwrap();
        let light = sky.sun_28.as_ref().unwrap().light_1c.unwrap();
        assert_eq!(
            run.world
                .get::<NiEffectAffectedNodes>(light)
                .unwrap()
                .0
                .len(),
            1
        );
    }
    {
        let run = PreludeRun::new(
            harness,
            Options {
                hdr: false,
                ..Options::default()
            },
        );
        run.outcome.as_ref().unwrap();
        let world = &run.world;
        let sky = run.globals().singleton_b365c4.as_deref().unwrap();
        let a = sky.atmosphere_20.as_ref().unwrap();
        let (quad, quad_node) = (a.quad_14.unwrap(), a.quad_node_10.unwrap());
        assert_eq!(
            world.get::<NiNodeChildren>(quad_node).unwrap().slots_ac,
            [Some(quad)]
        );
        assert_eq!(
            world
                .get::<NiNodeChildren>(a.root_04.unwrap())
                .unwrap()
                .slots_ac,
            [Some(quad_node), a.mesh_08],
            "without HDR the quad node precedes the mesh under Atmosphere Root"
        );
        assert_eq!(run.globals().hdr_b2c73c, 1.0);
        assert!(
            run.globals()
                .store_trace
                .iter()
                .all(|store| store.address != 0x00B2_C73C),
            "the image value stays, with no store"
        );
        let property = find_property_of_type(world, quad, 4).unwrap().unwrap();
        assert_eq!(
            world
                .get::<SkyShaderProperty>(property)
                .unwrap()
                .pass_kind_88,
            4,
            "the atmosphere quad's SkyShaderProperty pass kind is 4"
        );
        let data = data_of(world, quad);
        assert_eq!(
            data.bounds_0c,
            [0.0, 0.0, 0.0, 1000.0],
            "007BD0D0 quad bound"
        );
        assert_eq!(
            data.colors_24[0],
            [0.0, 0.0, 0.0, 1.0],
            "007BD0D0 quad colors"
        );
        assert_eq!(data.positions_1c[0][2].to_bits(), 0x3F7F_FFEF);
        let child = world
            .get::<NiNodeChildren>(run.scenes.world.unwrap())
            .unwrap()
            .slots_ac[0]
            .unwrap();
        assert_eq!(
            world
                .get::<MeshTriShapePayload>(quad)
                .unwrap()
                .local_transform
                .translation,
            world
                .get::<SceneNodePayload>(child)
                .unwrap()
                .local_transform
                .translation,
            "the quad takes the world child 0 translate"
        );
    }
}

#[test]
fn prelude_matches_the_runs_on_the_shipped_nifs() {
    let harness = Harness::new();
    // The INI is the pinned Windows one the runs used: 00B11E2C and 00B11E34 carry the handoff's values.
    assert_eq!(f32::from_bits(harness.setting_raw(0x00B1_1E2C)), 250.0);
    assert_eq!(f32::from_bits(harness.setting_raw(0x00B1_1E34)), 350.0);
    let run = PreludeRun::new(&harness, Options::default());
    check_against_run(&run);
    check_geometry_values(&run);
}

#[test]
fn prelude_failure_arms() {
    check_failure_arms(&Harness::new());
}
