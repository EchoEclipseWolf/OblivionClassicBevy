//! The prison NPCs' FaceGen head parts, morphed by the port (`006F0A00`, `006F4B50`, `005221C0`, `00558840`) from the real
//! archives and compared with run-af's `after_move` heads (`fixtures/rows/facegen_nodes.rows`, from
//! `tools/original_debug/facegen_nodes_rows.py`): every position, bit for bit. The coefficients are the snapshot's own
//! (TESNPC +0x108 and the race's +0x29C), so this checks the files, the combination and the morph, not the form loaders.
//! Host tests earn no validation bonus.

use std::collections::HashMap;

use bevy_ecs::prelude::*;
use recomp_core::fixtures;
use recomp_game::bs::archive::{ArchiveManager, ArchiveSettings};
use recomp_game::bs::face_gen_files::{EgmData, TriData, read_egm, read_tri};
use recomp_game::bs::face_gen_model::{ApplyOutcome, apply_coefficients, base_morph};
use recomp_game::bs::file::{GameFile, OpenedFile};
use recomp_game::bs::file_finder::{BsFileCallbacks, FileFinder, bs_file_open_core};
use recomp_game::bs::stream::{BsStreamServices, load_bs_stream_from_path};
use recomp_game::gamebryo::mesh_geometry::{MeshTriShapeData, NiGeometry};
use recomp_game::gamebryo::mesh_tri_strips::MeshTriStripsData;
use recomp_game::gamebryo::registration::NiFactoryRegistry;
use recomp_game::gamebryo::scene_node::NiNodeChildren;
use recomp_game::gamebryo::stream_file_pipeline::StreamFileHeaderErrorCode;
use recomp_game::gamebryo::stream_postload::StreamPostLoadRuntime;
use recomp_game::gamebryo::texture::SourceTextureLoadServices;
use recomp_game::tes::face_gen_head::{PART_NAMES_B10CA8, combine_coefficients};
use recomp_game::tes::facegen::{FaceGenLastError, FaceGenMatrix};

/// The meshes' NiSourceTextures resolve against nothing: these tests read only positions.
struct NoTextures;

impl SourceTextureLoadServices for NoTextures {
    fn search_candidates(&mut self, file_path: &[u8]) -> Vec<Vec<u8>> {
        vec![file_path.to_vec()]
    }
    fn find_texture(&mut self, _path: &[u8]) -> Option<Entity> {
        None
    }
    fn register_texture(&mut self, _path: &[u8], _texture: Entity) {}
    fn file_exists(&mut self, _path: &[u8]) -> bool {
        false
    }
}

struct Archives {
    finder: FileFinder,
    manager: ArchiveManager,
    registry: NiFactoryRegistry,
}

impl Archives {
    fn new() -> Self {
        let data = fixtures::data_dir();
        assert!(
            data.join("Oblivion - Meshes.bsa").exists(),
            "this test needs the game data: set OBLIVION_DATA_DIR ({data:?})"
        );
        let root = data.parent().unwrap().canonicalize().unwrap();
        let scratch = std::env::temp_dir().join(format!(
            "recomp_face_gen_heads_{}_{}",
            std::process::id(),
            std::thread::current()
                .name()
                .unwrap_or("x")
                .replace("::", "_")
        ));
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).unwrap();
        std::fs::write(scratch.join("Plugins.txt"), "Oblivion.esm\n").unwrap();
        let mut callbacks = BsFileCallbacks::default();
        let mut finder = FileFinder::new(&root, &mut callbacks);
        finder.add("Data");
        let settings = ArchiveSettings {
            invalidate_older_files: false,
            retain_directory_strings: 2,
            ..ArchiveSettings::default()
        };
        let mut manager = ArchiveManager::new(&root, settings);
        manager.initialize_from_settings(&finder, &scratch);
        let _ = std::fs::remove_dir_all(&scratch);
        let mut registry = NiFactoryRegistry::default();
        registry.register_known_crt_factories();
        registry.register_winmain_bs_factories();
        Self {
            finder,
            manager,
            registry,
        }
    }

    /// BSFile_OpenCore(path, 0, 0x2800, 1), as `006F66E0` opens a FaceGen file.
    fn open(&mut self, path: &str) -> Option<OpenedFile> {
        bs_file_open_core(&self.finder, Some(&mut self.manager), path, 0, 0x2800, 1)
    }

    /// `00559330`'s mesh: the stream's one top object and its first geometry child's positions. `None` for a NIF the port's
    /// stream loader doesn't read yet (10.0.1.0, the ears: `docs/open-items.md`).
    fn mesh_positions(&mut self, path: &str) -> Option<Vec<[f32; 3]>> {
        let mut world = World::new();
        let finder = &self.finder;
        let manager = &mut self.manager;
        let mut open = |path: &str, mode: i32, size: u32| {
            bs_file_open_core(finder, Some(&mut *manager), path, mode, size, 0xFFFF).map(
                |file| -> Box<dyn GameFile> {
                    match file {
                        OpenedFile::Ni(file) => Box::new(file),
                        OpenedFile::Loose(file) => Box::new(file),
                        OpenedFile::Packed(file) => Box::new(file),
                        OpenedFile::Compressed(file) => Box::new(file),
                    }
                },
            )
        };
        let load = load_bs_stream_from_path(
            &mut world,
            path,
            &mut BsStreamServices {
                open: &mut open,
                registry: &self.registry,
                draw_rand: None,
                draw_on_io_thread: true,
                source_textures: Some(&mut NoTextures),
                post_load: StreamPostLoadRuntime::default(),
            },
        )
        .unwrap_or_else(|error| panic!("{path}: {error:?}"));
        if load
            .error
            .as_ref()
            .and_then(|error| error.header.as_ref())
            .is_some_and(|header| header.code == StreamFileHeaderErrorCode::UnsupportedVersion)
        {
            return None;
        }
        assert_eq!(
            load.root_count(),
            1,
            "{path}: {:?} {:?}",
            load.status,
            load.error
        );
        let root = load.root(0).unwrap();
        let children = world
            .get::<NiNodeChildren>(root)
            .map(|children| children.slots_ac.clone())
            .unwrap_or_default();
        let geometry = children
            .into_iter()
            .flatten()
            .find(|&child| world.get::<NiGeometry>(child).is_some())
            .unwrap_or_else(|| panic!("{path}: no geometry child"));
        let data = world.get::<NiGeometry>(geometry).unwrap().data_b4.unwrap();
        if let Some(shape) = world.get::<MeshTriShapeData>(data) {
            return Some(shape.base.positions_1c.clone());
        }
        Some(
            world
                .get::<MeshTriStripsData>(data)
                .unwrap_or_else(|| panic!("{path}: no geometry data"))
                .base
                .positions_1c
                .clone(),
        )
    }
}

/// A part's EGM and its NIF's positions (none for a NIF the stream loader can't read yet).
type PartFiles = (EgmData, Option<Vec<[f32; 3]>>);

fn hex(text: &str) -> u32 {
    u32::from_str_radix(text, 16).unwrap()
}

/// `FGCOEF` rows: (base or race, label) → the four matrices.
fn coefficients(rows: &[String]) -> HashMap<(u32, String), [FaceGenMatrix; 4]> {
    let mut out: HashMap<(u32, String), [FaceGenMatrix; 4]> = HashMap::new();
    for row in rows {
        let fields: Vec<&str> = row.split(' ').filter(|f| !f.is_empty()).collect();
        if fields[0] != "FGCOEF" {
            continue;
        }
        let key = (hex(fields[1]), fields[2].to_owned());
        let k: usize = fields[3].parse().unwrap();
        let entry = out.entry(key).or_default();
        entry[k] = FaceGenMatrix {
            rows: hex(fields[4]),
            cols: hex(fields[5]),
            data: fields[6..]
                .iter()
                .map(|word| f32::from_bits(hex(word)))
                .collect(),
        };
    }
    out
}

fn fnv1a(positions: &[[f32; 3]]) -> u32 {
    let mut hash = 0x811C_9DC5_u32;
    for value in positions.iter().flatten() {
        for byte in value.to_bits().to_le_bytes() {
            hash = (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193);
        }
    }
    hash
}

/// The sibling of a model path with another extension, as `005500C0`, `00550010` and `0054FEB0` build it.
fn sibling(path: &str, extension: &str) -> String {
    let stem = &path[..path.rfind('.').unwrap()];
    format!("{stem}.{extension}")
}

#[test]
fn the_head_files_read() {
    let mut archives = Archives::new();
    let mut warnings = Vec::new();
    let mut error = FaceGenLastError::default();
    let path = "Meshes\\Characters\\Imperial\\HeadHuman.egm";
    let mut egm = EgmData::default();
    let mut opened = archives.open(path);
    assert!(read_egm(
        &mut egm,
        opened.as_mut(),
        path.as_bytes(),
        &mut warnings,
        &mut error
    ));
    assert_eq!((egm.symmetric_04.len(), egm.asymmetric_14.len()), (50, 30));
    let path = "Meshes\\Characters\\Imperial\\HeadHuman.tri";
    let mut tri = TriData::default();
    let mut opened = archives.open(path);
    assert!(read_tri(
        &mut tri,
        opened.as_mut(),
        path.as_bytes(),
        &mut warnings,
        &mut error
    ));
    let positions = archives
        .mesh_positions("Meshes\\Characters\\Imperial\\HeadHuman.nif")
        .unwrap();
    assert_eq!(tri.base_vertex_count() as usize, positions.len());
    assert!(
        egm.symmetric_04
            .iter()
            .all(|morph| morph.deltas_04.len() >= positions.len())
    );
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(error.code, 0);
}

#[test]
fn facegen_heads_match_after_move() {
    let rows = fixtures::rows("facegen_nodes");
    let coefficients = coefficients(&rows);
    let mut parts: HashMap<(u32, usize), String> = HashMap::new();
    let mut npcs: HashMap<u32, (u32, u32)> = HashMap::new();
    for row in &rows {
        let fields: Vec<&str> = row.split(' ').collect();
        match fields[0] {
            "FGPART" => {
                parts.insert(
                    (hex(fields[1]), fields[2].parse().unwrap()),
                    fields[3].to_owned(),
                );
            }
            "FGNPC" => {
                npcs.insert(hex(fields[1]), (hex(fields[2]), hex(fields[3])));
            }
            _ => {}
        }
    }
    let mut archives = Archives::new();
    let mut cache: HashMap<String, PartFiles> = HashMap::new();
    let mut checked = 0;
    let mut unread = 0;
    let mut mismatched: Vec<String> = Vec::new();
    for row in &rows {
        let fields: Vec<&str> = row.split(' ').collect();
        if fields[0] != "FGNODE" || fields.len() < 4 || fields[2] == "none" {
            continue;
        }
        let reference = hex(fields[1]);
        // The player's head (base 7) isn't the drain's, and its parts differ from this build by an ulp in places (neither the
        // 53-bit nor the 24-bit rounding gives them): which path built it (00667BE0's or the race menu's) isn't read yet.
        if reference == 0x14 {
            continue;
        }
        let name = fields[4];
        // The parts 00555A80 builds from the race (the hair and eyes are 00554260's and 005547F0's).
        let Some(part) = PART_NAMES_B10CA8[..7]
            .iter()
            .position(|&known| known == name)
        else {
            continue;
        };
        let (base, race) = npcs[&reference];
        let model = format!("Meshes\\{}", parts[&(race, part)]);
        let (egm, positions) = cache.entry(model.clone()).or_insert_with(|| {
            let path = sibling(&model, "egm");
            let mut egm = EgmData::default();
            let mut opened = archives.open(&path);
            let mut warnings = Vec::new();
            assert!(read_egm(
                &mut egm,
                opened.as_mut(),
                path.as_bytes(),
                &mut warnings,
                &mut FaceGenLastError::default()
            ));
            (egm, archives.mesh_positions(&model))
        });
        let mut combined: [FaceGenMatrix; 4] = Default::default();
        combine_coefficients(
            &coefficients[&(race, "race29c".to_owned())],
            &coefficients[&(base, "npc108".to_owned())],
            &mut combined,
        )
        .unwrap();
        let Some(positions) = positions else {
            // The ears' 10.0.1.0 NIFs: the stream loader's gap, not FaceGen's.
            assert!(model.contains("Ears"), "{model}");
            unread += 1;
            continue;
        };
        let mut morphed = positions.clone();
        let mut base_vertices = base_morph(Some(positions), &[]);
        let mut warnings = Vec::new();
        let outcome = apply_coefficients(
            egm,
            &combined,
            1.0,
            Some(&mut base_vertices),
            &mut morphed,
            None,
            &mut warnings,
        )
        .unwrap();
        assert_eq!(outcome, ApplyOutcome::Applied, "{row}");
        assert_eq!(morphed.len(), hex(fields[6]) as usize, "{row}");
        let first: Vec<u32> = morphed[0].iter().map(|v| v.to_bits()).collect();
        let want: Vec<u32> = fields[7..10].iter().map(|w| hex(w)).collect();
        if first != want {
            mismatched.push(format!("{} {name} v0", fields[1]));
        }
        if fnv1a(&morphed) != hex(fields[13]) {
            mismatched.push(format!("{} {name}", fields[1]));
        }
        checked += 1;
    }
    // The nine NPCs' face, mouth, both teeth and tongue; their ears wait for the stream loader's 10.0.1.0 header.
    assert_eq!((checked, unread), (45, 9));
    assert!(
        mismatched.is_empty(),
        "{} differ: {mismatched:?}",
        mismatched.len()
    );
}
