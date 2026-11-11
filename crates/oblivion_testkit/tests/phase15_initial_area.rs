//! Phase 15: WinMain 0040EBB9..0040EE79 on the real game, a fresh one per INI path, against P15-RUN-001..009. Each path runs its real arms end to end where the C++ test shared one game with recording hooks: `fixtures/rows/phase15.rows` and `phase15d.rows`, every line read. The recording arms (`RecordingArms`) keep the block's selection logic testable alone.
//!
//! The INI settings are set in the collection in memory, never in a file.

#[path = "game_data_window.rs"]
#[allow(
    dead_code,
    unused_imports,
    reason = "this test binary uses only part of the window"
)]
mod game_data_window;

use bevy_ecs::prelude::*;
use game_data_window::common::act_rows::RUN_004_ROLL_START;
use game_data_window::common::rows::{Deferred, RowBook, hex_bytes};
use game_data_window::{GameWindow, evidence_lines, run_00406360_seeded};
use oblivion_core::crt::MsvcRand;
use oblivion_core::error::GameError;
use oblivion_core::fixtures;
use oblivion_sim::tes::actor::TesObjectRefr;
use oblivion_sim::tes::actor_process::RandomSource;
use oblivion_sim::tes::ai_process::AiRealArms;
use oblivion_sim::tes::data_handler::{DataHandler, DataHandlerLists, FormsById};
use oblivion_sim::tes::data_handler_world::{
    create_exterior_cell, find_cell_by_editor_id, find_worldspace_grid_in_plugins,
    invalid_coordinate_warning, lookup_or_create_exterior_cell, propagate_runtime_field_to_masters,
};
use oblivion_sim::tes::data_load::DataLoadServices;
use oblivion_sim::tes::form::FormIdentity;
use oblivion_sim::tes::forms_world::{
    TesObjectCell, TesObjectLand, TesWorldSpace, WorldFormGlobals, cell_is_interior,
    find_master_exterior_cell, grid_key, land_height_range, world_lookup_cell,
};
use oblivion_sim::tes::os_globals::OsGlobalsData;
use oblivion_sim::tes::tes::{GameThreadLocals, Tes, TesGlobals};
use oblivion_sim::tes::tes_grid::{
    TesGridAnswers, TesGridBoundaries, TesGridGlobals, TesGridServices,
};
use oblivion_sim::tes::winmain::winmain_initial_area::{
    InitialAreaArms, InitialAreaPath, InitialAreaResult, RealInitialAreaArms, grid_origin_center,
    phase_15_line, run_initial_area,
};
use oblivion_sim::tes::winmain::winmain_world_setup::{
    WorldSetupSeams, WorldspaceInputs, run_world_setup,
};

const NO_DEFERRED: &[Deferred] = &[];
const TAMRIEL: u32 = 0x3C;

// ---- the shared game ----

struct StreamRandom<'a>(&'a mut MsvcRand);

impl RandomSource for StreamRandom<'_> {
    fn time64(&mut self) -> i64 {
        panic!("0047DFA8: the first-use seed, past Phase 11's draw");
    }
    fn srand(&mut self, seed: u32) {
        self.0.srand(seed);
    }
    fn rand(&mut self) -> i32 {
        self.0.rand()
    }
}

// IDA: N/A all of 00406360 and the Phase 14 block, leaving the world at 0040EBB9, and Rect.
fn game_at_window() -> (GameWindow, [f32; 3]) {
    let mut state = run_00406360_seeded(Some(RUN_004_ROLL_START));
    let rect = {
        let GameWindow {
            world,
            collections,
            harness,
            host,
        } = &mut state;
        world.insert_resource(OsGlobalsData::default());
        let lod_land = collections.ini.find_address(0x00B0_2D70).unwrap().raw as u8;
        let mut rand = host.rand;
        let mut warnings = Vec::new();
        let mut services = DataLoadServices {
            working_directory: &harness.root,
            warning: &mut |text, _| warnings.push(text),
            host,
        };
        let mut random = StreamRandom(&mut rand);
        let mut menu_mode = |_: &World| false;
        let outputs = run_world_setup(
            world,
            WorldspaceInputs {
                ini: &collections.ini,
                display_lod_land_b43077: lod_land,
                services: &mut services,
                level2_log: &mut |_| {},
                refresh_map_menu: &mut |_| Ok(false),
            },
            WorldSetupSeams {
                log: &mut |_| {},
                arms: AiRealArms {
                    random: &mut random,
                    menu_mode: &mut menu_mode,
                },
            },
        )
        .unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        outputs.worldspace.default_position
    };
    (state, rect)
}

fn set_starting(state: &mut GameWindow, cell: &str, x: &str, y: &str, space: &str) {
    for (address, text) in [
        (0x00B0_2CE0, cell),
        (0x00B0_2CF8, x),
        (0x00B0_2D00, y),
        (0x00B0_2CF0, space),
    ] {
        state
            .collections
            .ini
            .find_address_mut(address)
            .unwrap()
            .text = Some(text.as_bytes().to_vec());
    }
}

fn tes(world: &World) -> &Tes {
    world.resource::<TesGlobals>().tes.as_deref().unwrap()
}

fn form_id(world: &World, form: Entity) -> u32 {
    world.get::<FormIdentity>(form).unwrap().form_id_0c
}

fn tamriel(world: &World) -> Entity {
    let space = world
        .resource::<DataHandlerLists>()
        .world_spaces_0c
        .items()
        .next()
        .unwrap();
    assert_eq!(form_id(world, space), TAMRIEL);
    space
}

fn rect_hex(rect: &[f32; 3]) -> String {
    rect.iter()
        .flat_map(|value| value.to_bits().to_le_bytes())
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn floats_hex(values: &[f32]) -> String {
    values
        .iter()
        .flat_map(|value| value.to_bits().to_le_bytes())
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn word(raw: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(raw[at..at + 4].try_into().unwrap())
}

fn field<'a>(parts: &[&'a str], key: &str) -> &'a str {
    let prefix = format!("{key}=");
    parts
        .iter()
        .find_map(|part| part.strip_prefix(prefix.as_str()))
        .unwrap_or_else(|| panic!("no {key} in {parts:?}"))
}

fn hex_field(parts: &[&str], key: &str) -> u32 {
    u32::from_str_radix(field(parts, key), 16).unwrap()
}

fn record_field<'a>(line: &'a str, key: &str) -> &'a str {
    let parts: Vec<&'a str> = line.split(' ').collect();
    field(&parts, key)
}

// ---- rows ----

/// IDA: N/A the counts the tail rows hold past the host: FF-prefixed references, the boundary's extra types, and actor moves.
#[derive(Debug, Default)]
struct TailTally {
    dynamic_references: usize,
    extra_types: std::collections::BTreeMap<usize, usize>,
    moved_actors: usize,
    altered_cells: usize,
    cell_extra_types: std::collections::BTreeMap<usize, usize>,
}

// IDA: N/A a cell and its +0x48 list against the rows of `tag`. The tail's rows hold what 004D5BD0's counted reference loop adds: FF-prefixed references, the cell's flag 2 with them, actor moves, and extra types. Answers the references compared.
fn compare_cell(
    world: &World,
    book: &RowBook,
    cell: Entity,
    tag: &str,
    tally: &mut TailTally,
) -> usize {
    let tail = tag.starts_with("end");
    let row = book.take_one("CELL", tag);
    let what = format!("{tag} {:X}", form_id(world, cell));
    let identity = world.get::<FormIdentity>(cell).unwrap();
    let data = world.get::<TesObjectCell>(cell).unwrap();
    assert_eq!(identity.form_id_0c, hex_field(&row, "id"), "{what}");
    assert_eq!(
        u32::from(identity.form_type_04),
        hex_field(&row, "type"),
        "{what}"
    );
    let all_refs = book.take("REF", tag);
    let refs: Vec<&Vec<&str>> = all_refs
        .iter()
        .filter(|parts| !tail || hex_field(parts, "id") < 0xFF00_0000)
        .collect();
    let dynamic = refs.len() != all_refs.len();
    tally.dynamic_references += all_refs.len() - refs.len();
    for parts in all_refs
        .iter()
        .filter(|parts| tail && hex_field(parts, "id") >= 0xFF00_0000)
    {
        fixtures::ledger_relaxed(
            if tag == "end_interior" {
                "phase15d"
            } else {
                "phase15"
            },
            &parts[..3].join(" "),
            1,
            "a dynamic reference: 004D5BD0's actor boundary makes it, and the host's are held as a subset (docs/open-items.md, Phase 15)",
        );
    }
    let run_flags = hex_field(&row, "flags");
    if identity.flags_08 != run_flags {
        assert!(
            tail && dynamic && identity.flags_08 | 2 == run_flags,
            "{what} form flags {:X} run {run_flags:X}",
            identity.flags_08
        );
        tally.altered_cells += 1;
    }
    assert_eq!(
        u32::from(data.flags_24),
        hex_field(&row, "b24"),
        "{what} +0x24"
    );
    assert_eq!(
        u32::from(data.field_25),
        hex_field(&row, "b25"),
        "{what} +0x25"
    );
    assert_eq!(
        u32::from(data.process_level_26),
        hex_field(&row, "b26"),
        "{what} +0x26"
    );
    let world_id = data
        .world_50
        .filter(|_| data.flags_24 & 1 == 0)
        .map_or(0, |space| form_id(world, space));
    assert_eq!(world_id, hex_field(&row, "ws"), "{what} world");
    assert_eq!(
        data.land_40.map_or(0, |land| form_id(world, land)),
        hex_field(&row, "landid"),
        "{what} land"
    );
    assert_eq!(
        data.path_grid_44.map_or(0, |grid| form_id(world, grid)),
        hex_field(&row, "pgrdid"),
        "{what} path grid"
    );
    // The +0x3C block. An interior's 0x28 bytes, an exterior's 8.
    let block: Vec<u8> = data
        .data_3c
        .unwrap()
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect();
    let wanted = hex_bytes(field(&row, "data"));
    assert_eq!(&block[..wanted.len()], &wanted[..], "{what} +0x3C");
    let raw = hex_bytes(field(&row, "raw"));
    assert_eq!(word(&raw, 0), 0x00A4_69D4, "{what} vtable");
    assert_eq!(word(&raw, 0x18), 0x00A4_69C0, "{what} +0x18");
    assert_eq!(
        word(&raw, 0x28),
        data.extra_28.original_vtable,
        "{what} +0x28"
    );
    if tail {
        // 004D63A0's Havok world (type 2) and 004D5BD0's boundary add extras the host doesn't.
        for (byte, (&host, &run)) in data
            .extra_28
            .presence_08
            .iter()
            .zip(&raw[0x30..0x3C])
            .enumerate()
        {
            assert_eq!(
                host & !run,
                0,
                "{what}: a host extra the run lacks, byte {byte}"
            );
            for bit in 0..8 {
                if run & !host & (1 << bit) != 0 {
                    *tally.cell_extra_types.entry(byte * 8 + bit).or_default() += 1;
                }
            }
        }
    } else {
        assert_eq!(
            word(&raw, 0x2C) != 0,
            !data.extra_28.entries.is_empty(),
            "{what} +0x2C"
        );
        assert_eq!(
            &raw[0x30..0x3C],
            &data.extra_28.presence_08,
            "{what} +0x30 presence"
        );
    }
    // +0x54 the node: the host keeps 1 for a tree it counts, the run a pointer; +0x48's head is read by `refs`.
    if raw.len() >= 0x58 {
        assert_eq!(
            word(&raw, 0x54) != 0,
            data.scene_node_present_54 != 0,
            "{what} +0x54"
        );
    }
    let objects: Vec<Entity> = data.objects_48.items().collect();
    assert_eq!(objects.len(), refs.len(), "{what} reference count");
    for (index, (&reference, row)) in objects.iter().zip(&refs).enumerate() {
        let id = form_id(world, reference);
        let what = format!("{what} ref {index} {id:X}");
        assert_eq!(id, hex_field(row, "id"), "{what}");
        let identity = world.get::<FormIdentity>(reference).unwrap();
        assert_eq!(
            u32::from(identity.form_type_04),
            hex_field(row, "type"),
            "{what}"
        );
        assert_eq!(identity.flags_08, hex_field(row, "flags"), "{what} flags");
        let refr = world.get::<TesObjectRefr>(reference).unwrap();
        let base = refr.base_form_1c.unwrap();
        assert_eq!(form_id(world, base), hex_field(row, "base"), "{what} base");
        assert_eq!(
            u32::from(world.get::<FormIdentity>(base).unwrap().form_type_04),
            hex_field(row, "btype"),
            "{what} base type"
        );
        if tail && matches!(identity.form_type_04, 0x32 | 0x33) {
            for key in ["pos", "rot"] {
                fixtures::ledger_relaxed(
                    if tag == "end_interior" {
                        "phase15d"
                    } else {
                        "phase15"
                    },
                    &format!("{} {} {} #{key}", row[0], row[1], row[2]),
                    1,
                    "the tail's actors: 004D5BD0's actor arm places them behind a counted boundary, so a different place is tallied, not failed (docs/open-items.md, Phase 15)",
                );
            }
        }
        let same_place = floats_hex(&refr.position_2c) == field(row, "pos")
            && floats_hex(&refr.rotation_20) == field(row, "rot");
        if !same_place {
            // 004D5BD0's actor arm places actors.
            assert!(
                tail && matches!(identity.form_type_04, 0x32 | 0x33),
                "{what}: pos {} rot {}, run {} {}",
                floats_hex(&refr.position_2c),
                floats_hex(&refr.rotation_20),
                field(row, "pos"),
                field(row, "rot")
            );
            tally.moved_actors += 1;
        }
        assert_eq!(
            refr.scale_38.to_bits(),
            hex_field(row, "scale"),
            "{what} scale"
        );
        assert_eq!(
            form_id(world, refr.parent_cell_40.unwrap()),
            hex_field(row, "parent"),
            "{what} parent"
        );
        let extra = hex_bytes(field(row, "extra"));
        assert_eq!(
            word(&extra, 0),
            refr.extra_data_44.original_vtable,
            "{what}"
        );
        if tail {
            for (byte, (&host, &run)) in refr
                .extra_data_44
                .presence_08
                .iter()
                .zip(&extra[8..20])
                .enumerate()
            {
                assert_eq!(
                    host & !run,
                    0,
                    "{what}: a host extra the run lacks, byte {byte}"
                );
                for bit in 0..8 {
                    if run & !host & (1 << bit) != 0 {
                        *tally.extra_types.entry(byte * 8 + bit).or_default() += 1;
                    }
                }
            }
        } else {
            assert_eq!(
                word(&extra, 4) != 0,
                !refr.extra_data_44.entries.is_empty(),
                "{what} extra head"
            );
            assert_eq!(
                &extra[8..20],
                &refr.extra_data_44.presence_08,
                "{what} extra presence"
            );
        }
    }
    let counts = book.take_one("REFS", tag);
    assert_eq!(
        counts[2],
        format!("count={}", all_refs.len()),
        "{what} the REFS row"
    );
    if let Some(land) = data.land_40 {
        compare_land(world, book, land, tag);
    } else {
        // An interior's rows have no LAND row.
        assert!(book.take("LAND", tag).is_empty(), "{what}");
    }
    objects.len()
}

// IDA: N/A a LAND row: the form flags, the DATA word with 004C7090's bit 3, the cell and the block by null-ness.
fn compare_land(world: &World, book: &RowBook, land: Entity, tag: &str) {
    let row = book.take_one("LAND", tag);
    let identity = world.get::<FormIdentity>(land).unwrap();
    let data = world.get::<TesObjectLand>(land).unwrap();
    assert_eq!(identity.form_id_0c, hex_field(&row, "id"), "LAND {tag}");
    assert_eq!(identity.flags_08, hex_field(&row, "flags"), "LAND {tag}");
    let raw = hex_bytes(field(&row, "raw"));
    assert_eq!(word(&raw, 0), 0x00A4_6064, "LAND {tag}");
    assert_eq!(word(&raw, 0x18), data.child_cell_vtable_18, "LAND {tag}");
    assert_eq!(word(&raw, 0x1C), data.flags_1c, "LAND {tag} +0x1C");
    assert_eq!(word(&raw, 0x20) != 0, data.cell_20.is_some(), "LAND {tag}");
    assert_eq!(
        word(&raw, 0x24) != 0,
        data.data_24.is_some(),
        "LAND {tag} +0x24"
    );
}

// IDA: N/A TES against a raw dump's words: the numbers exactly, and the pointers by null-ness.
fn check_tes_raw(world: &World, raw: &[u8], what: &str) {
    let data = tes(world);
    assert_eq!(word(raw, 0), data.vtable_00, "{what} +0x00");
    for (index, &value) in data.exterior_coordinates_20.iter().enumerate() {
        assert_eq!(
            word(raw, 0x20 + index * 4),
            value as u32,
            "{what} +0x{:X}",
            0x20 + index * 4
        );
    }
    assert_eq!(
        word(raw, 0x30) != 0,
        data.current_exterior_cell_30.is_some(),
        "{what} +0x30"
    );
    assert_eq!(
        word(raw, 0x34) != 0,
        data.current_interior_cell_34.is_some(),
        "{what} +0x34"
    );
    assert_eq!(
        word(raw, 0x48),
        data.last_exterior_coordinates_48[0] as u32,
        "{what} +0x48"
    );
    assert_eq!(
        word(raw, 0x4C),
        data.last_exterior_coordinates_48[1] as u32,
        "{what} +0x4C"
    );
    assert_eq!(raw[0x51], data.cell_test_active_51, "{what} +0x51");
    assert_eq!(raw[0x52], data.byte_52, "{what} +0x52");
    assert_eq!(raw[0x53], data.cell_borders_enabled_53, "{what} +0x53");
    assert_eq!(
        word(raw, 0x6C),
        data.grid_center_margin_x_6c.to_bits(),
        "{what} +0x6C"
    );
    assert_eq!(
        word(raw, 0x70),
        data.grid_center_margin_y_70.to_bits(),
        "{what} +0x70"
    );
    assert_eq!(
        word(raw, 0x74) != 0,
        data.current_world_space_74.is_some(),
        "{what} +0x74"
    );
    assert_eq!(
        word(raw, 0x78) != 0,
        data.exterior_cell_list_78.is_some(),
        "{what} +0x78"
    );
    assert_eq!(
        word(raw, 0x7C) != 0,
        !data.exterior_cell_list_next_7c.is_empty(),
        "{what} +0x7C"
    );
}

// IDA: N/A GridCellArray against a raw dump's words.
fn check_grid_raw(world: &World, raw: &[u8], what: &str) {
    let grid = tes(world).grid_cells_08.as_ref().unwrap();
    assert_eq!(word(raw, 0), grid.original_vtable, "{what}");
    assert_eq!(word(raw, 4), grid.world_x_04 as u32, "{what} +0x04");
    assert_eq!(word(raw, 8), grid.world_y_08 as u32, "{what} +0x08");
    assert_eq!(word(raw, 0xC), grid.size_0c, "{what} +0x0C");
    assert_eq!(word(raw, 0x14), grid.float_14.to_bits(), "{what} +0x14");
    assert_eq!(word(raw, 0x18), grid.float_18.to_bits(), "{what} +0x18");
    assert_eq!(word(raw, 0x1C), grid.float_1c.to_bits(), "{what} +0x1C");
    assert_eq!(raw[0x20], grid.byte_20, "{what} +0x20");
    if raw.len() >= 0x28 {
        assert_eq!(
            word(raw, 0x24) != 0,
            grid.pointer_24.is_some(),
            "{what} +0x24"
        );
    }
}

fn slot_ids(world: &World) -> Vec<u32> {
    tes(world)
        .grid_cells_08
        .as_ref()
        .unwrap()
        .grid_10
        .iter()
        .map(|slot| slot.cell_00.map_or(0, |cell| form_id(world, cell)))
        .collect()
}

fn hex_list(text: &str) -> Vec<u32> {
    text.split(',')
        .map(|id| u32::from_str_radix(id, 16).unwrap())
        .collect()
}

fn buffer_ids(buffer: &[Option<Entity>], world: &World) -> Vec<u32> {
    buffer
        .iter()
        .map(|entry| entry.map_or(0, |cell| form_id(world, cell)))
        .collect()
}

// IDA: N/A a run's end records, `BUF end`, `WATER end`, the grid and the buffers, on the host's state.
fn check_grid_end_records(world: &World, records: &[String]) {
    let mut water = 0;
    for line in records {
        let parts: Vec<&str> = line.split(' ').collect();
        match (parts.get(1), parts.get(2), parts.get(3)) {
            (Some(&"BUF"), Some(&"end"), Some(&"ext")) => {
                assert_eq!(
                    buffer_ids(&tes(world).exterior_cell_buffer_3c, world),
                    hex_list(field(&parts, "ids")),
                    "BUF ext"
                );
            }
            (Some(&"BUF"), Some(&"end"), Some(&"int")) => {
                assert_eq!(
                    buffer_ids(&tes(world).interior_cell_buffer_38, world),
                    hex_list(field(&parts, "ids")),
                    "BUF int"
                );
            }
            (Some(&"GRID"), Some(&"end"), _) => {
                check_grid_raw(world, &hex_bytes(field(&parts, "raw")), line);
            }
            (Some(&"WATER"), Some(&"end"), _) => {
                let index: usize = field(&parts, "idx").parse().unwrap();
                let slot = &tes(world).grid_cells_08.as_ref().unwrap().grid_10[index];
                let cell = slot.cell_00.unwrap();
                assert_eq!(form_id(world, cell), hex_field(&parts, "cell"), "{line}");
                let record = slot.water_04.as_ref().unwrap();
                assert_eq!(u32::from(record.byte_00), hex_field(&parts, "b0"), "{line}");
                assert_eq!(
                    record.cell_14.map_or(0, |entry| form_id(world, entry)),
                    hex_field(&parts, "cell14"),
                    "{line}"
                );
                let flags = world
                    .get::<oblivion_sim::gamebryo::scene_node::SceneNodePayload>(
                        record.node_04.unwrap(),
                    )
                    .unwrap()
                    .effective_av_flags;
                // `nodeflags` is the dword: the flags are its low word, and the high word is the next field's.
                assert_eq!(
                    u32::from(flags),
                    hex_field(&parts, "nodeflags") & 0xFFFF,
                    "{line}"
                );
                water += 1;
            }
            _ => {}
        }
    }
    assert_eq!(water, 25);
}

// ---- recording arms ----

type MasterHook<'h> = &'h mut dyn FnMut(&World, Option<Entity>);
type TailHook<'h> = &'h mut dyn FnMut(&World);

/// IDA: N/A a recorder over the real arms: `forward` says which group runs for real. Each call appends a line in the C++ test's text.
struct RecordingArms<'h, 'a, 'b, 'c> {
    real: RealInitialAreaArms<'a, 'b, 'c>,
    /// 004478B0 and 004F1630.
    forward_loads: bool,
    /// 004455E0, 00445A10, 00444EC0, 00482310.
    forward_transitions: bool,
    calls: Vec<String>,
    warnings: Vec<String>,
    at_master: Option<MasterHook<'h>>,
    at_tail: Option<TailHook<'h>>,
}

impl InitialAreaArms for RecordingArms<'_, '_, '_, '_> {
    fn find_worldspace_grid_in_plugins(
        &mut self,
        world: &mut World,
        name: &[u8],
    ) -> Result<(Option<Entity>, i32, i32), GameError> {
        self.calls.push(format!(
            "004478B0 {}",
            name.iter()
                .map(|&byte| char::from(byte))
                .collect::<String>()
        ));
        if self.forward_loads {
            return self.real.find_worldspace_grid_in_plugins(world, name);
        }
        Ok((None, 0, 0))
    }

    fn find_master_exterior_cell(
        &mut self,
        world: &mut World,
        space: Entity,
        x: i32,
        y: i32,
    ) -> Result<Option<Entity>, GameError> {
        self.calls
            .push(format!("004F1630 {:X} {x} {y}", form_id(world, space)));
        if !self.forward_loads {
            return Ok(None);
        }
        let cell = self.real.find_master_exterior_cell(world, space, x, y)?;
        if let Some(hook) = &mut self.at_master {
            hook(world, cell);
        }
        Ok(cell)
    }

    fn lookup_or_create_exterior_cell(
        &mut self,
        world: &mut World,
        x: i32,
        y: i32,
        space: Option<Entity>,
        create: bool,
    ) -> Result<Option<Entity>, GameError> {
        self.calls
            .push(format!("00447740 {x} {y} {}", u8::from(create)));
        self.real
            .lookup_or_create_exterior_cell(world, x, y, space, create)
    }

    fn set_current_worldspace(
        &mut self,
        world: &mut World,
        space: Option<Entity>,
    ) -> Result<(), GameError> {
        self.calls.push("004431F0".to_owned());
        self.real.set_current_worldspace(world, space)
    }

    fn enter_interior_cell(
        &mut self,
        world: &mut World,
        cell: Entity,
        rect: &mut [f32; 3],
    ) -> Result<(), GameError> {
        self.calls.push(format!(
            "004455E0 {:X} {}",
            form_id(world, cell),
            rect_hex(rect)
        ));
        if self.forward_transitions {
            return self.real.enter_interior_cell(world, cell, rect);
        }
        Ok(())
    }

    fn exit_interior_cell(
        &mut self,
        world: &mut World,
        rect: &mut [f32; 3],
    ) -> Result<(), GameError> {
        self.calls.push(format!("00445A10 {}", rect_hex(rect)));
        if self.forward_transitions {
            return self.real.exit_interior_cell(world, rect);
        }
        Ok(())
    }

    fn update_exterior_grid_from_position(
        &mut self,
        world: &mut World,
        rect: &[f32; 3],
        reload: bool,
    ) -> Result<(), GameError> {
        self.calls
            .push(format!("00444EC0 {} {}", rect_hex(rect), u8::from(reload)));
        if let Some(hook) = &mut self.at_tail {
            hook(world);
        }
        if self.forward_transitions {
            return self
                .real
                .update_exterior_grid_from_position(world, rect, reload);
        }
        Ok(())
    }

    fn initialize_loaded_cells(&mut self, world: &mut World) -> Result<(), GameError> {
        self.calls.push("00482310".to_owned());
        if self.forward_transitions {
            return self.real.initialize_loaded_cells(world);
        }
        Ok(())
    }

    fn warning(&mut self, world: &World, text: String) {
        self.warnings.push(text.clone());
        self.real.warning(world, text);
    }
}

/// IDA: N/A one run of the block with the recorder's switches, and what it left.
struct BlockRun {
    result: InitialAreaResult,
    rect: [f32; 3],
    calls: Vec<String>,
    warnings: Vec<String>,
    load_warnings: Vec<String>,
}

fn run_block<'h>(
    state: &mut GameWindow,
    rect: [f32; 3],
    forward_loads: bool,
    forward_transitions: bool,
    answers: TesGridAnswers,
    at_master: Option<MasterHook<'h>>,
    at_tail: Option<TailHook<'h>>,
) -> Result<BlockRun, GameError> {
    let GameWindow {
        world,
        collections,
        harness,
        host,
    } = state;
    let lod_land = collections.ini.find_address(0x00B0_2D70).unwrap().raw as u8;
    let mut load_warnings = Vec::new();
    let mut warning = |text: String, _: bool| load_warnings.push(text);
    let mut load = DataLoadServices {
        working_directory: &harness.root,
        warning: &mut warning,
        host,
    };
    let mut level2 = |_: String| {};
    let mut refresh = |_: &mut World| Ok(false);
    let mut grid = TesGridServices {
        load: &mut load,
        ini: &collections.ini,
        answers,
        display_lod_land_b43077: lod_land,
        level2_log: &mut level2,
        refresh_map_menu: &mut refresh,
        interface: None,
        drain: None,
        reference_loop: false,
        game_sounds: None,
        archives: None,
        models: None,
    };
    let mut rect = rect;
    let mut arms = RecordingArms {
        real: RealInitialAreaArms { grid: &mut grid },
        forward_loads,
        forward_transitions,
        calls: Vec::new(),
        warnings: Vec::new(),
        at_master,
        at_tail,
    };
    let result = run_initial_area(world, &collections.ini, &mut arms, &mut rect)?;
    let (calls, warnings) = (arms.calls, arms.warnings);
    Ok(BlockRun {
        result,
        rect,
        calls,
        warnings,
        load_warnings,
    })
}

// ---- the paths ----

// G4: every counted boundary of each path, by address and count. Build 3 ported 004D4A20 (`cell_havok`), so it is no longer a counted boundary. A count that goes missing fails here, since some of them own end-state fields (TES +0x30, P15-RUN-009). `docs/boundary-audit.md` sets these against the C++ log, with a reason per difference.
const ENTER_BOUNDARIES: &[(u32, u32)] = &[
    (0x0040_D160, 1),
    (0x0040_D4D0, 1),
    (0x0043_4020, 1),
    (0x0043_8060, 1),
    (0x0043_E000, 1),
    (0x0043_FC20, 2),
    (0x0044_0190, 1),
    (0x0044_0AF0, 1),
    (0x0044_1610, 1),
    (0x0049_A200, 1),
    (0x0049_B5F0, 1),
    (0x004C_AFC0, 1),
    (0x004C_B590, 1),
    (0x004C_B670, 2),
    (0x004D_4D00, 1),
    (0x004D_5BD0, 1),
    (0x004D_63A0, 1),
    (0x004E_7610, 1),
    (0x0057_32D0, 1),
    (0x005A_89A0, 1),
    (0x005A_DB40, 1),
    (0x0067_5F40, 1),
    (0x0067_5FC0, 1),
    (0x0067_7360, 1),
    (0x0067_8750, 1),
    (0x0070_7370, 4),
    (0x0070_7610, 2),
    (0x0070_7980, 1),
    (0x007A_9CF0, 1),
    (0x007A_B6F0, 1),
    (0x007B_2130, 1),
    (0x007C_4D90, 1),
    (0x007C_7E50, 1),
    (0x0088_9E10, 1),
    // Build 3: the interior's world now exists at 004D63A0 (its own 004D58B0 made it through 004D4A20, no longer counted), so 004D63A0's 0088B680 counts too.
    (0x0088_B680, 2),
    (0x0088_BD60, 1),
];
const EXIT_BOUNDARIES: &[(u32, u32)] = &[
    (0x0040_D160, 1),
    (0x0043_4020, 1),
    (0x0043_8060, 25),
    (0x0043_E000, 1),
    (0x0043_FC20, 2),
    (0x0044_02F0, 1),
    (0x0044_0AF0, 1),
    (0x0044_1610, 1),
    (0x0048_3D60, 1),
    (0x0049_9E40, 1),
    (0x0049_A200, 2),
    (0x004B_D380, 3025),
    (0x004C_5640, 25),
    (0x004C_5BA0, 25),
    (0x004C_AFC0, 25),
    (0x004C_B590, 25),
    (0x004C_B670, 50),
    (0x004C_CCC0, 1),
    (0x004D_4D00, 50),
    (0x004D_5BD0, 25),
    (0x004D_63A0, 25),
    (0x004E_7610, 22),
    (0x004E_A6E0, 1),
    (0x004E_BDA0, 1),
    (0x004E_F7E0, 1),
    (0x0053_7D40, 1),
    (0x0054_3BB0, 1),
    (0x0057_32D0, 1),
    (0x0057_B950, 25),
    (0x005A_89A0, 1),
    (0x0067_5F40, 1),
    (0x0067_5FC0, 1),
    (0x0067_7360, 1),
    (0x0067_8750, 1),
    (0x0070_7370, 76),
    (0x0070_7610, 76),
    (0x0070_7980, 51),
    (0x007A_B6F0, 1),
    (0x007B_2130, 1),
    (0x007B_3A40, 3025),
    (0x007C_7EB0, 1),
    (0x0088_9E00, 3),
    (0x0088_B680, 26),
    (0x0088_D1D0, 3),
    (0x00B4_2D78, 1),
];
const TAIL_BOUNDARIES: &[(u32, u32)] = &[
    (0x0043_4020, 1),
    (0x0043_8060, 25),
    (0x0043_FC20, 2),
    (0x0044_02F0, 1),
    (0x0044_0AF0, 1),
    (0x0044_1610, 1),
    (0x0048_3D60, 1),
    (0x0049_9E40, 1),
    (0x0049_A200, 2),
    (0x004B_D380, 6050),
    (0x004C_5640, 25),
    (0x004C_5BA0, 25),
    (0x004C_AFC0, 25),
    (0x004C_B590, 25),
    (0x004C_B670, 25),
    (0x004C_CCC0, 1),
    (0x004D_4D00, 50),
    (0x004D_5BD0, 25),
    (0x004D_63A0, 25),
    (0x004E_7610, 22),
    (0x004E_BDA0, 1),
    (0x0053_7D40, 1),
    (0x0057_32D0, 1),
    (0x0057_B950, 25),
    (0x0067_5F40, 1),
    (0x0067_5FC0, 1),
    (0x0067_8750, 1),
    (0x0070_7370, 76),
    (0x0070_7610, 75),
    (0x0070_7980, 50),
    (0x007B_3A40, 6050),
    (0x0088_9E00, 2),
    (0x0088_B680, 25),
    (0x0088_D1D0, 2),
];

fn assert_boundaries(run: &str, boundaries: &TesGridBoundaries, expected: &[(u32, u32)]) {
    let got: Vec<(u32, u32)> = boundaries.calls.iter().map(|(a, n)| (*a, *n)).collect();
    assert_eq!(got, expected, "{run}: the counted boundaries");
}

// P15-RUN-001, the user's INI: two strlen calls and 0040EE7A. Nothing changes.
#[test]
fn run_001_the_default_path_changes_nothing() {
    let records = evidence_lines("phase15-run001-2026-10-10");
    let (mut state, rect) = game_at_window();
    let rows = |tag: &str, kind: &str| -> Vec<String> {
        records
            .iter()
            .filter(|line| {
                line.starts_with(&format!("{kind} ")) && line.contains(&format!(" {tag} "))
            })
            .cloned()
            .collect()
    };
    for (tag, marker) in [("before", "40EBB9"), ("after", "40EE7A")] {
        let tes_row = &rows(tag, "T")[0];
        assert!(tes_row.contains(marker), "{tes_row}");
        assert_eq!(record_field(tes_row, "rect"), rect_hex(&rect));
        check_tes_raw(
            &state.world,
            &hex_bytes(record_field(tes_row, "raw")),
            tes_row,
        );
        let grid_row = &rows(tag, "G")[0];
        check_grid_raw(
            &state.world,
            &hex_bytes(record_field(grid_row, "raw")),
            grid_row,
        );
        assert!(slot_ids(&state.world).iter().all(|&id| id == 0));
        for file_row in rows(tag, "F") {
            let parts: Vec<&str> = file_row.split(' ').collect();
            let data = state.world.resource::<DataHandler>();
            let index: usize = field(&parts, "idx").parse().unwrap();
            let file = data.file_store.get(data.files_8c8[index]).unwrap();
            assert_eq!(
                file.buffer_size_228,
                hex_field(&parts, "f228"),
                "{file_row}"
            );
            assert_eq!(file.flags_3dc, hex_field(&parts, "flags3dc"), "{file_row}");
            assert!(file.name_1c.ends_with(field(&parts, "name")), "{file_row}");
        }
        for setting_row in rows(tag, "S") {
            assert_eq!(record_field(&setting_row, "text"), "", "{setting_row}");
        }
        let locals = rows(tag, "TL");
        let tls = state.world.resource::<GameThreadLocals>();
        // RUN-001's TLS read ran at teb=0: its zeros are the dump's, not the thread's, whose b185 the suspended record has.
        assert_eq!(record_field(&locals[0], "teb"), "0");
        assert_eq!(tls.track_changes_185, 1);
        assert_eq!(tls.byte_184, 0);
        assert_eq!(tls.byte_186, 0);
        if tag == "before" {
            let run = run_block(
                &mut state,
                rect,
                false,
                false,
                TesGridAnswers::default(),
                None,
                None,
            )
            .unwrap();
            assert_eq!(run.result.path, InitialAreaPath::NoSettings);
            assert!(!run.result.tail);
            assert!(run.calls.is_empty(), "{:?}", run.calls);
            assert!(run.warnings.is_empty());
            assert_eq!(run.rect, rect);
            let line = phase_15_line(&run.result, 0, &run.rect);
            assert_eq!(
                line,
                "Phase-15: path no settings, cell 0, Rect {2048, 2048, 128}, tail skipped"
            );
            let baseline =
                std::fs::read_to_string(fixtures::root().join("cpp-baseline/LastTest-x64.log"))
                    .unwrap();
            assert!(baseline.contains(&line), "the C++ prints the same line");
        }
    }
    let tes_data = tes(&state.world);
    assert_eq!(
        form_id(&state.world, tes_data.current_world_space_74.unwrap()),
        TAMRIEL
    );
}

// 0044569x asks 0040FDA0 at the site, on the world's MoviePlayer: `answers` holds nothing about it. A thread alive there means no 0040D4D0.
#[test]
fn the_movie_question_is_asked_at_the_interior_site() {
    use oblivion_sim::tes::movie_player::{
        MoviePlayer, MoviePlayerServices, MovieSettings, MovieThreadHandle, ScriptedMovieDevice,
        ScriptedMoviePlatform,
    };
    use oblivion_sim::tes::os_globals::{ScriptedOsPlatform, lock_shared};
    use std::sync::{Arc, Mutex};
    let (mut state, rect) = game_at_window();
    set_starting(&mut state, "XPAichan01", "", "", "");
    let player = MoviePlayer::new(MoviePlayerServices {
        bink: Arc::new(Mutex::new(
            oblivion_sim::tes::bink_library::ScriptedBink::default(),
        )),
        device: Some(Arc::new(Mutex::new(ScriptedMovieDevice::default()))),
        platform: Arc::new(Mutex::new(ScriptedMoviePlatform::default())),
        os_platform: Arc::new(Mutex::new(ScriptedOsPlatform::with_devices(true, false))),
        renderer_width: 1920,
        renderer_height: 1080,
        os_globals: None,
        time: oblivion_sim::tes::window::SharedTimeInfo::default(),
        interface: oblivion_sim::tes::interface_manager::InterfaceMovieReads::default(),
        video_directory: std::env::temp_dir(),
        settings: MovieSettings {
            sound_enabled_b16178: false,
            preload_intro_sequence_b030b4: false,
            preload_size_limit_b030bc: 0,
            intro_sequence_priority_b030c4: 0,
            static_screen_wait_time_b030ac: 0.0,
            intro_sequence_b03084: Vec::new(),
        },
    });
    let thread = MovieThreadHandle::spawn_suspended("held", || {}).unwrap();
    *lock_shared(&player.globals.thread_b33434) = Some(thread.clone());
    state.world.insert_resource(player);
    run_block(
        &mut state,
        rect,
        true,
        true,
        TesGridAnswers::default(),
        None,
        None,
    )
    .unwrap();
    assert_eq!(
        state.world.resource::<TesGridBoundaries>().at(0x0040_D4D0),
        0
    );
    thread.resume();
    thread.wait();
}

// P15-RUN-002, P15-RUN-008: XPAichan01 -> 004476B0 -> interior 00010360 -> 004455E0, then the tail, whose 00444EC0 gate is closed by +0x34.
#[test]
fn run_002_an_interior_cell_with_every_phase15d_row() {
    let book = RowBook::load("phase15d");
    let (mut state, rect) = game_at_window();
    set_starting(&mut state, "XPAichan01", "", "", "");
    let run = run_block(
        &mut state,
        rect,
        true,
        true,
        TesGridAnswers::default(),
        None,
        None,
    )
    .unwrap();
    let world = &state.world;
    assert_eq!(run.result.path, InitialAreaPath::EditorIdCell);
    assert!(run.result.tail);
    assert!(run.warnings.is_empty());
    assert!(run.load_warnings.is_empty(), "{:?}", run.load_warnings);
    assert_eq!(run.rect, rect, "RUN-002's T after: rect unchanged");
    assert_eq!(run.calls[0], format!("004455E0 10360 {}", rect_hex(&rect)));
    assert_eq!(run.calls[1], format!("00444EC0 {} 1", rect_hex(&rect)));
    assert_eq!(run.calls[2], "00482310");
    let cell = run.result.cell.unwrap();
    assert_eq!(form_id(world, cell), 0x10360);
    let mut tally = TailTally::default();
    let references = compare_cell(world, &book, cell, "end_interior", &mut tally);
    assert_eq!(references + tally.dynamic_references, 643);
    let data = tes(world);
    assert_eq!(data.current_interior_cell_34, Some(cell));
    assert_eq!(data.current_exterior_cell_30, None);
    let row = book.take_one("TESF", "end_interior");
    check_tes_raw(world, &hex_bytes(field(&row, "raw")), "RUN-008 TES");
    let row = book.take_one("WATER58", "end_interior");
    fixtures::ledger_relaxed(
        "phase15d",
        "WATER58 end_interior #raw",
        1,
        "a raw dump with heap words: `b0`, `cell14` and `nodeflags` are the values read out of it",
    );
    let record = data.water_node_58.as_ref().unwrap();
    assert_eq!(u32::from(record.byte_00), hex_field(&row, "b0"));
    assert_eq!(
        form_id(world, record.cell_14.unwrap()),
        hex_field(&row, "cell14")
    );
    let flags = world
        .get::<oblivion_sim::gamebryo::scene_node::SceneNodePayload>(record.node_04.unwrap())
        .unwrap()
        .effective_av_flags;
    assert_eq!(u32::from(flags), hex_field(&row, "nodeflags") & 0xFFFF);
    book.assert_accounted(NO_DEFERRED);
    // RUN-002's own records: T and G after, and the cell after.
    let records = evidence_lines("phase15-run002-2026-10-10");
    let after = records
        .iter()
        .find(|line| line.starts_with("T ") && line.contains(" after "))
        .unwrap();
    assert_eq!(record_field(after, "rect"), rect_hex(&rect));
    check_tes_raw(world, &hex_bytes(record_field(after, "raw")), after);
    let grid = records
        .iter()
        .find(|line| line.starts_with("G ") && line.contains(" after "))
        .unwrap();
    check_grid_raw(world, &hex_bytes(record_field(grid, "raw")), grid);
    assert!(
        slot_ids(world).iter().all(|&id| id == 0),
        "RUN-002: no grid cell"
    );
    let cell_row = records
        .iter()
        .find(|line| line.contains(" after_interior "))
        .unwrap();
    assert_eq!(
        u32::from(world.get::<TesObjectCell>(cell).unwrap().flags_24),
        hex_field(&cell_row.split(' ').collect::<Vec<_>>(), "b24"),
        "RUN-002 +0x24"
    );
    assert_eq!(data.interior_cell_buffer_38, vec![Some(cell), None, None]);
    let grid = data.grid_cells_08.as_ref().unwrap();
    for slot in &grid.grid_10 {
        assert_eq!(
            slot.water_04.as_ref().unwrap().byte_00,
            0,
            "00441D50(1) hides the records"
        );
    }
    let boundaries = world.resource::<TesGridBoundaries>();
    assert_eq!(
        boundaries.at(0x0044_0AF0),
        1,
        "the 643 references' one 00440AF0"
    );
    assert_eq!(boundaries.at(0x0043_E000), 1);
    let globals = world.resource::<TesGridGlobals>();
    assert_eq!(globals.transition_b43384, 0);
    assert_eq!(globals.interior_b43074, 1);
    println!(
        "RUN-002: {references} references, boundaries {:X?}",
        boundaries.calls
    );
    assert_boundaries("RUN-002", boundaries, ENTER_BOUNDARIES);
}

// P15-RUN-003, P15-RUN-009: OldBridgeExterior -> 004476B0 misses -> 004478B0 Tamriel (4, 7) -> 004431F0, Rect, 00445A10 -> 00444FB0 -> 00444EC0(&Rect, 0), 00447740(4, 7, Tamriel, 0), no tail.
#[test]
fn run_003_the_plugin_grid_path_through_the_exit() {
    let (mut state, rect) = game_at_window();
    set_starting(&mut state, "OldBridgeExterior", "", "", "");
    let run = run_block(
        &mut state,
        rect,
        true,
        true,
        TesGridAnswers::default(),
        None,
        None,
    )
    .unwrap();
    let world = &state.world;
    assert_eq!(run.result.path, InitialAreaPath::PluginWorldspace);
    assert_eq!((run.result.x, run.result.y), (4, 7));
    assert!(!run.result.tail);
    assert!(run.warnings.is_empty());
    assert!(run.load_warnings.is_empty(), "{:?}", run.load_warnings);
    assert_eq!(
        run.calls,
        [
            "004478B0 OldBridgeExterior".to_owned(),
            "004431F0".to_owned(),
            format!("00445A10 {}", rect_hex(&[18432.0, 30720.0, 0.0])),
            "00447740 4 7 0".to_owned(),
        ]
    );
    assert_eq!(run.result.space, Some(tamriel(world)));
    let records = evidence_lines("phase15-run003-2026-10-10");
    let after = records
        .iter()
        .find(|line| line.starts_with("T ") && line.contains(" after "))
        .unwrap();
    assert_eq!(record_field(after, "rect"), rect_hex(&run.rect));
    // +0x30 ends at the actor-process boundary's last 00440590 (RUN-009's HW records); compare by what the host owns.
    check_tes_raw(world, &hex_bytes(record_field(after, "raw")), after);
    let grid = records
        .iter()
        .find(|line| line.starts_with("G ") && line.contains(" after "))
        .unwrap();
    check_grid_raw(world, &hex_bytes(record_field(grid, "raw")), grid);
    // RUN-003's C records: every cell 004F1630 made, with its flags and grid.
    let mut created = 0;
    for line in records
        .iter()
        .filter(|line| line.starts_with("C ") && line.contains(" created "))
    {
        let parts: Vec<&str> = line.split(' ').collect();
        let cell = world
            .resource::<FormsById>()
            .lookup(hex_field(&parts, "id"))
            .unwrap();
        let data = world.get::<TesObjectCell>(cell).unwrap();
        assert_eq!(u32::from(data.flags_24), hex_field(&parts, "b24"), "{line}");
        let host_flags = world.get::<FormIdentity>(cell).unwrap().flags_08;
        assert_eq!(host_flags | 2, hex_field(&parts, "flags") | 2, "{line}");
        let block = data.data_3c.unwrap();
        let grid = hex_bytes(field(&parts, "xy"));
        assert_eq!(word(&grid, 0), block[0], "{line}");
        assert_eq!(word(&grid, 4), block[1], "{line}");
        created += 1;
    }
    assert_eq!(created, 25);
    // RUN-009: the same path, its end records.
    let run_009 = evidence_lines("phase15-run009-2026-10-10");
    check_grid_end_records(world, &run_009);
    let tes_end = run_009
        .iter()
        .find(|line| line.contains("TESF end"))
        .unwrap();
    check_tes_raw(world, &hex_bytes(record_field(tes_end, "raw")), tes_end);
    let data = tes(world);
    assert_eq!(data.last_exterior_coordinates_48, [0x7FFF_FFFF; 2]);
    let globals = world.resource::<TesGridGlobals>();
    assert_eq!(globals.exterior_done_b3522c, 1, "RUN-009 b3522c");
    assert_eq!(globals.transition_b43384, 0);
    assert_eq!(globals.interior_b43074, 0);
    assert_eq!(
        globals.centre_b4312c.map(f32::to_bits),
        [0x4690_0000, 0x46F0_0000]
    );
    assert_eq!(globals.centre_copy_b43134, globals.centre_b4312c);
    let boundaries = world.resource::<TesGridBoundaries>();
    // 004430F0(1) skips every slot, so 00444FB0's own is the only 00440AF0. The C++ has two.
    assert_eq!(boundaries.at(0x0044_0AF0), 1);
    assert_eq!(boundaries.at(0x0043_E000), 1);
    assert_eq!(world.resource::<TesGridBoundaries>().reference_counts, [0]);
    println!("RUN-003: boundaries {:X?}", boundaries.calls);
    assert_boundaries("RUN-003", boundaries, EXIT_BOUNDARIES);
}

// P15-RUN-004, P15-RUN-006, P15-RUN-007: X 4, Y 7: 004F1630(Tamriel, 4, 7) -> exterior -> 004431F0, Rect, the tail. Every phase15.rows line.
#[test]
fn run_004_the_coordinates_with_every_phase15_row() {
    let book = RowBook::load("phase15");
    let (mut state, rect) = game_at_window();
    set_starting(&mut state, "", "4", "7", "");
    let mut at_master = |world: &World, cell: Option<Entity>| {
        let mut tally = TailTally::default();
        let cell = cell.unwrap();
        let references = compare_cell(world, &book, cell, "master", &mut tally);
        assert_eq!(references, 52);
        assert_eq!(tally.dynamic_references, 0);
    };
    let mut at_tail = |world: &World| {
        let row = book.take_one("TESF", "tail_start");
        check_tes_raw(world, &hex_bytes(field(&row, "raw")), "tail_start");
        let row = book.take_one("GRID", "tail_start");
        check_grid_raw(world, &hex_bytes(field(&row, "raw")), "tail_start");
        let slots = slot_ids(world);
        for (at, row) in book.take("SLOT", "tail_start").iter().enumerate() {
            let index: usize = field(row, "idx").parse().unwrap();
            assert_eq!(index, at, "{row:?}");
            assert_eq!(slots[index], hex_field(row, "id"), "{row:?}");
        }
    };
    let started = std::time::Instant::now();
    let run = run_block(
        &mut state,
        rect,
        true,
        true,
        TesGridAnswers::default(),
        Some(&mut at_master),
        Some(&mut at_tail),
    )
    .unwrap();
    println!(
        "0040EBB9..0040EE79, RUN-004's path: {:.2?}",
        started.elapsed()
    );
    let world = &state.world;
    assert_eq!(run.result.path, InitialAreaPath::CoordinatesCell);
    assert!(run.result.tail);
    assert!(run.warnings.is_empty());
    assert!(run.load_warnings.is_empty(), "{:?}", run.load_warnings);
    assert_eq!(run.rect.map(f32::to_bits), [0x4690_0000, 0x46F0_0000, 0]);
    assert_eq!(run.result.space, Some(tamriel(world)));
    // ---- the end: every slot, row by row ----
    let row = book.take_one("TESF", "end");
    check_tes_raw(world, &hex_bytes(field(&row, "raw")), "end");
    let raw = hex_bytes(field(&row, "raw"));
    let row = book.take_one("GRID", "end");
    check_grid_raw(world, &hex_bytes(field(&row, "raw")), "end");
    let slots = slot_ids(world);
    let mut tally = TailTally::default();
    let mut references = 0;
    let mut slot_of_cell_30 = None;
    for row in book.take("SLOT", "end") {
        let index: usize = field(&row, "idx").parse().unwrap();
        assert_eq!(slots[index], hex_field(&row, "id"), "{row:?}");
        let cell = world.resource::<FormsById>().lookup(slots[index]).unwrap();
        let tag = format!("end_slot{index}");
        // The row's LAND +0x20 is the cell's own pointer, so TES +0x30 can be named by it.
        let land = book.take_one("LAND", &tag);
        if word(&hex_bytes(field(&land, "raw")), 0x20) == word(&raw, 0x30) {
            slot_of_cell_30 = Some(slots[index]);
        }
        references += compare_cell(world, &book, cell, &tag, &mut tally);
    }
    let data = tes(world);
    assert_eq!(
        slot_of_cell_30,
        data.current_exterior_cell_30
            .map(|cell| form_id(world, cell)),
        "TES +0x30 by the run's pointer"
    );
    assert_eq!(slot_of_cell_30, Some(0x5FC9));
    assert_eq!(data.exterior_coordinates_20, [4, 7, 4, 7]);
    assert_eq!(data.current_interior_cell_34, None);
    let distant = data.grid_distant_04.as_ref().unwrap();
    assert_eq!((distant.world_x_04, distant.world_y_08), (4, 7));
    book.assert_accounted(NO_DEFERRED);
    println!(
        "RUN-006: {references} references compared, past the host: {tally:?}, of the rows' 1,666"
    );
    // ---- RUN-007 ----
    let records = evidence_lines("phase15-run007-2026-10-10");
    check_grid_end_records(world, &records);
    let count_line = records
        .iter()
        .find(|line| line.contains("REFCOUNT"))
        .unwrap();
    let parts: Vec<&str> = count_line.split(' ').collect();
    let boundaries = world.resource::<TesGridBoundaries>();
    assert_eq!(
        boundaries.reference_counts,
        [hex_field(&parts, "v15")],
        "{count_line}"
    );
    assert_eq!(hex_field(&parts, "v15"), 1609);
    assert_eq!(boundaries.at(0x0044_0AF0), 1);
    assert_eq!(boundaries.at(0x0048_3D60), 1, "lod_b3608f=1");
    assert_eq!(boundaries.at(0x0057_B950), 25, "HIDDEN v21_al=1");
    assert_eq!(boundaries.at(0x004C_5640), 25, "00482310");
    let globals = world.resource::<TesGridGlobals>();
    let tail_end = records
        .iter()
        .rfind(|line| line.contains(" GLOB "))
        .unwrap();
    let parts: Vec<&str> = tail_end.split(' ').collect();
    assert_eq!(
        u32::from(globals.loading_b33a69),
        hex_field(&parts, "b33a69"),
        "{tail_end}"
    );
    assert_eq!(
        u32::from(globals.scripts_b33a58),
        hex_field(&parts, "b33a58"),
        "{tail_end}"
    );
    assert_eq!(
        u32::from(globals.lod_update_b2cbc0),
        hex_field(&parts, "b2cbc0"),
        "{tail_end}"
    );
    assert_eq!(
        u32::from(globals.water_player_b3521d),
        hex_field(&parts, "b3521d"),
        "{tail_end}"
    );
    let flags = world
        .resource::<TesGlobals>()
        .save_load_game
        .as_ref()
        .unwrap()
        .fields_14[1];
    assert_eq!(flags, hex_field(&parts, "slg18"), "{tail_end}");
    // The percents go by 4 from 4, each at 24 bits. No record holds them.
    let percents = &boundaries.percents;
    assert_eq!(percents.len(), 25);
    assert_eq!(percents[0].to_bits(), 4.0_f32.to_bits());
    assert_eq!(percents[24].to_bits(), 100.0_f32.to_bits());
    // RUN-004's own records.
    let records = evidence_lines("phase15-run004-2026-10-10");
    let after = records
        .iter()
        .find(|line| line.starts_with("T ") && line.contains(" after "))
        .unwrap();
    assert_eq!(record_field(after, "rect"), rect_hex(&run.rect));
    check_tes_raw(world, &hex_bytes(record_field(after, "raw")), after);
    // ---- the LAND data, which no row holds past +0x1C ----
    let cell = run.result.cell.unwrap();
    let land = world.get::<TesObjectCell>(cell).unwrap().land_40.unwrap();
    let data = world
        .get::<TesObjectLand>(land)
        .unwrap()
        .data_24
        .as_ref()
        .unwrap();
    assert_eq!((data.grid_x_98, data.grid_y_9c), (4, 7));
    assert!(data.min_height_18 <= data.max_height_1c);
    assert!(
        data.vertices_04[0]
            .iter()
            .all(|vertex| vertex[2].fract() == 0.0)
    );
    assert!(world.resource::<WorldFormGlobals>().land_textures_queued > 0);
    println!(
        "LAND (4, 7): heights {}..{}, {} textures queued; boundaries {:X?}",
        data.min_height_18,
        data.max_height_1c,
        world.resource::<WorldFormGlobals>().land_textures_queued,
        boundaries.calls
    );
    assert_boundaries("RUN-004", boundaries, TAIL_BOUNDARIES);
}

// P15-RUN-004b: sStartingWorld Tamriel matches at the first item.
#[test]
fn run_004b_a_world_that_matches_and_one_that_doesnt() {
    let (mut state, rect) = game_at_window();
    let records = evidence_lines("phase15-run004b-2026-10-10");
    assert!(
        records
            .iter()
            .any(|line| line.contains("sStartingWorld") && line.contains("text=Tamriel"))
    );
    for name in ["Tamriel", "tamriel", "NoSuchWorld", ""] {
        set_starting(&mut state, "", "4", "7", name);
        let run = run_block(
            &mut state,
            rect,
            false,
            false,
            TesGridAnswers::default(),
            None,
            None,
        )
        .unwrap();
        assert_eq!(run.result.space, Some(tamriel(&state.world)), "{name}");
        assert_eq!(run.calls[0], "004F1630 3C 4 7");
    }
}

// P15-RUN-005: an unknown editor ID -> 004478B0 misses -> the warning -> the tail at the default point, around (0, 0).
#[test]
fn run_005_a_missing_cell_warns_and_takes_the_tail() {
    let (mut state, rect) = game_at_window();
    set_starting(&mut state, "NoSuchStartCellP15", "", "", "");
    let run = run_block(
        &mut state,
        rect,
        true,
        true,
        TesGridAnswers::default(),
        None,
        None,
    )
    .unwrap();
    let world = &state.world;
    assert_eq!(run.result.path, InitialAreaPath::NotFound);
    assert!(run.result.tail);
    assert_eq!(
        run.warnings,
        ["Could not find starting cell 'NoSuchStartCellP15'."]
    );
    assert_eq!(run.load_warnings, run.warnings);
    assert_eq!(run.rect, rect, "RUN-005: the default point stays");
    let records = evidence_lines("phase15-run005-2026-10-10");
    let after = records
        .iter()
        .find(|line| line.starts_with("T ") && line.contains(" after "))
        .unwrap();
    assert_eq!(record_field(after, "rect"), rect_hex(&run.rect));
    check_tes_raw(world, &hex_bytes(record_field(after, "raw")), after);
    let grid = records
        .iter()
        .find(|line| line.starts_with("G ") && line.contains(" after "))
        .unwrap();
    check_grid_raw(world, &hex_bytes(record_field(grid, "raw")), grid);
    let mut created = 0;
    for line in records
        .iter()
        .filter(|line| line.starts_with("C ") && line.contains(" created "))
    {
        let parts: Vec<&str> = line.split(' ').collect();
        let cell = world
            .resource::<FormsById>()
            .lookup(hex_field(&parts, "id"))
            .unwrap();
        let data = world.get::<TesObjectCell>(cell).unwrap();
        assert_eq!(u32::from(data.flags_24), hex_field(&parts, "b24"), "{line}");
        let block = data.data_3c.unwrap();
        let grid = hex_bytes(field(&parts, "xy"));
        assert_eq!(
            (word(&grid, 0), word(&grid, 4)),
            (block[0], block[1]),
            "{line}"
        );
        created += 1;
    }
    assert_eq!(created, 25);
    assert_eq!(tes(world).exterior_coordinates_20, [0; 4]);
}

// ---- 15a alone: the coordinates' warnings no run reached, and the exits ----

#[test]
fn the_selection_logic_with_recording_arms() {
    let (mut state, rect) = game_at_window();
    // Only X, or only Y: 0040EE7A with no tail.
    for (x, y) in [("4", ""), ("", "7")] {
        set_starting(&mut state, "", x, y, "");
        let run = run_block(
            &mut state,
            rect,
            false,
            false,
            TesGridAnswers::default(),
            None,
            None,
        )
        .unwrap();
        assert_eq!(run.result.path, InitialAreaPath::NoSettings);
        assert!(!run.result.tail);
        assert!(run.calls.is_empty());
    }
    // 004F1630 records and answers null; 00447740's create arm off with TLS +0x184: the world's warning text.
    state.world.resource_mut::<GameThreadLocals>().byte_184 = 1;
    for (name, text) in [
        (
            "",
            "Could not find starting cell (5, 6) in default Tamriel worldspace",
        ),
        (
            "Tamriel",
            "Could not find starting cell (5, 6) in worldspace 'Tamriel'.",
        ),
    ] {
        set_starting(&mut state, "", "5", "6", name);
        let run = run_block(
            &mut state,
            rect,
            false,
            false,
            TesGridAnswers::default(),
            None,
            None,
        )
        .unwrap();
        assert_eq!(run.result.path, InitialAreaPath::NotFound);
        assert_eq!(run.warnings, [text]);
        assert_eq!(run.rect, rect);
        assert_eq!(
            run.calls,
            [
                "004F1630 3C 5 6".to_owned(),
                "00447740 5 6 1".to_owned(),
                format!("00444EC0 {} 1", rect_hex(&rect)),
                "00482310".to_owned(),
            ]
        );
    }
    state.world.resource_mut::<GameThreadLocals>().byte_184 = 0;
    // `atol` takes the leading number.
    set_starting(&mut state, "", " -3x", "+12", "");
    let run = run_block(
        &mut state,
        rect,
        false,
        false,
        TesGridAnswers::default(),
        None,
        None,
    )
    .unwrap();
    assert_eq!((run.result.x, run.result.y), (-3, 12));
    // An interior with a recording 004455E0: RUN-002's selection alone.
    set_starting(&mut state, "xpaichan01", "", "", "");
    let run = run_block(
        &mut state,
        rect,
        false,
        false,
        TesGridAnswers::default(),
        None,
        None,
    )
    .unwrap();
    assert_eq!(run.result.path, InitialAreaPath::EditorIdCell, "_stricmp");
    assert_eq!(
        run.calls,
        [
            format!("004455E0 10360 {}", rect_hex(&rect)),
            format!("00444EC0 {} 1", rect_hex(&rect)),
            "00482310".to_owned(),
        ]
    );
    // The Rect arithmetic.
    assert_eq!(grid_origin_center(4).to_bits(), 0x4690_0000);
    assert_eq!(grid_origin_center(-1).to_bits(), (-2048.0_f32).to_bits());
    // Past 24 bits the sum rounds once.
    assert_eq!(grid_origin_center(0x7FFFF), 0x7FFF_F800_u32 as f32);
}

// 004476B0, 004EFE80, 00447740, 004471D0, 00447DB0 on the real load. RUN-002 finds 00010360; RUN-003's 004476B0 misses an exterior name, whose cell isn't loaded; RUN-004's 004EFE80 misses (4, 7) before the load. No run reaches 004471D0: RUN-004's 004F1630 finds its cell.
#[test]
fn the_small_callees_on_the_loaded_data() {
    let (mut state, _rect) = game_at_window();
    let GameWindow {
        world,
        harness,
        host,
        ..
    } = &mut state;
    let interior = find_cell_by_editor_id(world, b"XPAichan01")
        .unwrap()
        .unwrap();
    assert_eq!(form_id(world, interior), 0x10360);
    assert!(cell_is_interior(world, interior));
    assert_eq!(
        find_cell_by_editor_id(world, b"xpaichan01").unwrap(),
        Some(interior)
    );
    assert_eq!(
        find_cell_by_editor_id(world, b"OldBridgeExterior").unwrap(),
        None
    );
    let space = tamriel(world);
    assert_eq!(world_lookup_cell(world, space, 4, 7), (None, false));
    assert_eq!(world_lookup_cell(world, space, 0x8000, 0), (None, true));
    assert_eq!(grid_key(4, 7), 0x0004_0007);
    assert_eq!(grid_key(-1, -1), 0xFFFF_FFFF);
    let mut warnings = Vec::new();
    let mut warning = |text: String, _: bool| warnings.push(text);
    let mut load = DataLoadServices {
        working_directory: &harness.root,
        warning: &mut warning,
        host,
    };
    assert_eq!(
        lookup_or_create_exterior_cell(world, 0x8000, 0, Some(space), true, &mut load).unwrap(),
        None
    );
    assert_eq!(
        lookup_or_create_exterior_cell(world, 100, -100, Some(space), false, &mut load).unwrap(),
        None
    );
    let next = world.resource::<DataHandler>().next_form_id_8c0();
    let created = lookup_or_create_exterior_cell(world, 100, -100, None, true, &mut load)
        .unwrap()
        .unwrap();
    assert_eq!(form_id(world, created), next, "00448D40's dynamic ID");
    let data = world.get::<TesObjectCell>(created).unwrap();
    assert_eq!(data.flags_24, 2, "004C97E0(2)");
    let block = data.data_3c.unwrap();
    assert_eq!(block[0] as i32, 100);
    assert_eq!(block[1] as i32, -100);
    assert_eq!(data.world_50, Some(space));
    assert_eq!(
        world.get::<FormIdentity>(created).unwrap().form_type_04,
        0x30
    );
    assert_eq!(
        world
            .get::<TesWorldSpace>(space)
            .unwrap()
            .cells_30
            .get(grid_key(100, -100))
            .copied(),
        Some(created)
    );
    assert_eq!(
        lookup_or_create_exterior_cell(world, 100, -100, Some(space), true, &mut load).unwrap(),
        Some(created)
    );
    assert_eq!(
        create_exterior_cell(world, b"", 1, 1, None, &mut load).unwrap(),
        None,
        "a null world"
    );
    assert_eq!(warnings, [invalid_coordinate_warning()]);
    propagate_runtime_field_to_masters(world);
    let data = world.resource::<DataHandler>();
    let masters: Vec<u32> = data
        .files_8c8
        .iter()
        .map(|&id| data.file_store.get(id).unwrap())
        .filter(|file| file.is_master())
        .map(|file| file.buffer_size_228)
        .collect();
    assert_eq!(masters, [0x2800], "one master, Oblivion.esm");
}

// 004478B0 and 004F1630 alone: a miss, an empty name, a loaded cell answers at once, and the 4 LAND arms' data.
#[test]
fn the_plugin_re_read_and_the_master_load() {
    let (mut state, _rect) = game_at_window();
    let GameWindow {
        world,
        harness,
        host,
        ..
    } = &mut state;
    let mut warnings = Vec::new();
    let mut warning = |text: String, _: bool| warnings.push(text);
    let mut load = DataLoadServices {
        working_directory: &harness.root,
        warning: &mut warning,
        host,
    };
    let space = tamriel(world);
    assert_eq!(
        find_worldspace_grid_in_plugins(world, b"OldBridgeExterior", &mut load).unwrap(),
        (Some(space), 4, 7)
    );
    assert_eq!(
        find_worldspace_grid_in_plugins(world, b"oldbridgeexterior", &mut load).unwrap(),
        (Some(space), 4, 7),
        "_stricmp"
    );
    assert_eq!(
        find_worldspace_grid_in_plugins(world, b"NoSuchStartCellP15", &mut load).unwrap(),
        (None, 0, 0)
    );
    assert_eq!(
        find_worldspace_grid_in_plugins(world, b"", &mut load).unwrap(),
        (None, 0, 0)
    );
    let cell = find_master_exterior_cell(world, space, 4, 7, &mut load)
        .unwrap()
        .unwrap();
    assert_eq!(form_id(world, cell), 0x5FC9);
    let references = world
        .get::<TesObjectCell>(cell)
        .unwrap()
        .objects_48
        .items()
        .count();
    assert_eq!(references, 52);
    assert_eq!(
        find_master_exterior_cell(world, space, 4, 7, &mut load).unwrap(),
        Some(cell),
        "004F167B a loaded cell answers at once"
    );
    assert_eq!(
        world
            .get::<TesObjectCell>(cell)
            .unwrap()
            .objects_48
            .items()
            .count(),
        references
    );
    let tls = *world.resource::<GameThreadLocals>();
    assert_eq!((tls.byte_184, tls.byte_186), (0, 0), "004D150D..");
    let land = world.get::<TesObjectCell>(cell).unwrap().land_40.unwrap();
    assert_ne!(
        world.get::<TesObjectLand>(land).unwrap().flags_1c & 8,
        0,
        "004C7976"
    );
    let range = land_height_range(world, land);
    assert!(range[0] <= range[1]);
    assert!(warnings.is_empty(), "{warnings:?}");
}
