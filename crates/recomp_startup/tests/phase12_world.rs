//! Phase 12c: synthetic plugins for the world and reference arms P12-RUN-001's raw samples can't see: what sits behind a pointer, and the arms no shipped record takes. Every expected value here comes from reading the named function's disassembly, not from a run. Host tests earn no validation bonus.

mod common;

use std::path::PathBuf;

use bevy_ecs::prelude::*;
use common::load::{Loader, install_player};
use recomp_game::tes::actor::{CHARACTER_VTABLES, CREATURE_VTABLES, MobileObject, TesObjectRefr};
use recomp_game::tes::actor_process::{ActorListId, ActorProcessGlobals};
use recomp_game::tes::base_extra_list::{
    EXTRA_ACTION, EXTRA_COUNT, EXTRA_ENABLE_STATE_PARENT, EXTRA_LOCK, EXTRA_LOD, EXTRA_MAP_MARKER,
    EXTRA_OWNER, EXTRA_PERSISTENT_CELL, EXTRA_RAG_DOLL, EXTRA_REGION_LIST, EXTRA_TELEPORT,
    ExtraData,
};
use recomp_game::tes::data_handler::{DataHandler, DataHandlerLists, FormsById};
use recomp_game::tes::data_load::{DataLoadGlobals, toft_offset_arm};
use recomp_game::tes::form::{FormIdentity, ModReferences, OriginalVtable};
use recomp_game::tes::form_arms::{check_classes_against_the_map, editor_id_of, form_class};
use recomp_game::tes::form_components::FormRef;
use recomp_game::tes::form_components::TesFullName;
use recomp_game::tes::forms_dialogue::TesTopicInfo;
use recomp_game::tes::forms_world::{
    BOOL_MAP_VTABLE, CELL_MAP_VTABLE, TesObjectCell, TesObjectLand, TesPathGrid, TesRoad,
    TesWorldSpace, WorldFormGlobals,
};
use recomp_game::tes::player::{PlayerCharacter, PlayerGlobals};

fn u16_bytes(value: u16) -> Vec<u8> {
    value.to_le_bytes().to_vec()
}

fn u32_bytes(value: u32) -> Vec<u8> {
    value.to_le_bytes().to_vec()
}

fn f32_bytes(value: f32) -> Vec<u8> {
    value.to_le_bytes().to_vec()
}

fn cat(parts: &[&[u8]]) -> Vec<u8> {
    parts.concat()
}

fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    cat(&[kind, &u16_bytes(data.len() as u16), data])
}

fn text(value: &str) -> Vec<u8> {
    let mut bytes = value.as_bytes().to_vec();
    bytes.push(0);
    bytes
}

fn record(kind: &[u8; 4], flags: u32, id: u32, payload: &[u8]) -> Vec<u8> {
    cat(&[
        kind,
        &u32_bytes(payload.len() as u32),
        &u32_bytes(flags),
        &u32_bytes(id),
        &u32_bytes(0),
        payload,
    ])
}

fn tes4(master: bool, version: f32, masters: &[&str]) -> Vec<u8> {
    let mut payload = chunk(
        b"HEDR",
        &cat(&[&f32_bytes(version), &u32_bytes(100), &u32_bytes(0x800)]),
    );
    for name in masters {
        payload.extend(chunk(b"MAST", &text(name)));
        payload.extend(chunk(b"DATA", &0_u64.to_le_bytes()));
    }
    record(b"TES4", u32::from(master), 0, &payload)
}

/// IDA: N/A a temp working directory with `Data\` plugins.
struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("recomp_phase12c_{}_{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("Data")).unwrap();
        Self { root }
    }

    fn write(&self, name: &str, bytes: &[u8]) {
        std::fs::write(self.root.join("Data").join(name), bytes).unwrap();
    }

    // IDA: N/A a loader over the plugins written so far, with the harness's player and two bound objects.
    fn loader(&self) -> Loader {
        let mut loader = Loader::new(&self.root);
        loader.load_all();
        loader.host.pre_load = |world| {
            install_player(world);
            bound_object(world, 0x1C, 0x700);
            bound_object(world, 0x1B, 0x701);
        };
        loader
    }

    fn run(&self, records: &[u8]) -> Loader {
        let mut esm = tes4(true, 0.8, &[]);
        esm.extend_from_slice(records);
        self.write("Base.esm", &esm);
        let mut loader = self.loader();
        assert_eq!(loader.run(), Ok(1));
        loader
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

// IDA: N/A a STAT or MISC from its class, for a NAME to find.
fn bound_object(world: &mut World, form_type: u32, id: u32) {
    let form = (form_class(form_type).unwrap().construct)(world).unwrap();
    recomp_game::tes::data_handler::set_form_id(world, form, id);
}

fn form_at(loader: &Loader, id: u32) -> Entity {
    loader
        .world
        .resource::<FormsById>()
        .lookup(id)
        .unwrap_or_else(|| panic!("no form {id:08X}"))
}

fn warned(loader: &Loader, want: &str) -> bool {
    loader.warnings.iter().any(|line| line.contains(want))
}

fn world_record(id: u32) -> Vec<u8> {
    record(b"WRLD", 0, id, &chunk(b"DATA", &[0]))
}

fn exterior(id: u32, flags: u32, x: i32, y: i32) -> Vec<u8> {
    record(
        b"CELL",
        flags,
        id,
        &cat(&[
            &chunk(b"DATA", &[0]),
            &chunk(b"XCLC", &cat(&[&x.to_le_bytes(), &y.to_le_bytes()])),
        ]),
    )
}

fn interior(id: u32, flags: u32) -> Vec<u8> {
    record(b"CELL", flags, id, &chunk(b"DATA", &[1]))
}

fn name_data(base: u32, position: [f32; 3], rotation: [f32; 3]) -> Vec<u8> {
    let mut data = Vec::new();
    for value in position.iter().chain(&rotation) {
        data.extend(f32_bytes(*value));
    }
    cat(&[&chunk(b"NAME", &u32_bytes(base)), &chunk(b"DATA", &data)])
}

// 004F1E70: every chunk, and 0x2BD buckets for a non-reserved ID with DATA clearing +0x5C bit 0.
#[test]
fn a_world_space_loads_its_chunks() {
    let fixture = Fixture::new("world");
    let map: Vec<u8> = (0..0x10).collect();
    let loader = fixture.run(&record(
        b"WRLD",
        0,
        0x900,
        &cat(&[
            &chunk(b"EDID", &text("Sancta")),
            &chunk(b"FULL", &text("A Name")),
            &chunk(b"WNAM", &u32_bytes(0x11)),
            &chunk(b"CNAM", &u32_bytes(0x22)),
            &chunk(b"NAM2", &u32_bytes(0x33)),
            &chunk(b"SNAM", &u32_bytes(0x44)),
            &chunk(b"ICON", &text("map.dds")),
            &chunk(b"MNAM", &map),
            &chunk(b"DATA", &[0x40]),
            &chunk(b"NAM0", &cat(&[&f32_bytes(-8.0), &f32_bytes(4.0)])),
            &chunk(b"NAM9", &cat(&[&f32_bytes(16.0), &f32_bytes(32.0)])),
            &chunk(b"NAM0", &cat(&[&f32_bytes(-1.0), &f32_bytes(-9.0)])),
            &chunk(b"OFST", &cat(&[&u32_bytes(5), &u32_bytes(6)])),
        ]),
    ));
    assert!(loader.warnings.is_empty(), "{:?}", loader.warnings);
    let form = form_at(&loader, 0x900);
    let world = &loader.world;
    assert_eq!(editor_id_of(world, form), b"Sancta");
    assert_eq!(
        world
            .get::<TesFullName>(form)
            .unwrap()
            .name_04
            .data_00
            .as_deref(),
        Some(&b"A Name"[..])
    );
    let space = world.get::<TesWorldSpace>(form).unwrap();
    assert_eq!(space.parent_7c, FormRef::Raw(0x11));
    assert_eq!(space.climate_58, FormRef::Raw(0x22));
    assert_eq!(space.water_80, FormRef::Raw(0x33));
    assert_eq!(space.sound_94, 0x44);
    assert_eq!(space.map_84.to_vec(), map);
    assert_eq!(space.flags_5c, 0x40);
    assert_eq!(space.bounds_98, [-8.0, -9.0, 16.0, 32.0], "the min and max");
    // The master words are the last chunk's, raw.
    assert_eq!(
        space.master_bounds_ac,
        [
            (-1.0_f32).to_bits(),
            (-9.0_f32).to_bits(),
            16.0_f32.to_bits(),
            32.0_f32.to_bits()
        ]
    );
    assert_eq!(space.offsets_a8.as_deref(), Some(&[5, 6][..]));
    assert_ne!(space.record_offset_bc, 0, "a master's record offset");
    assert_eq!(space.cells_30.original_vtable, CELL_MAP_VTABLE);
    assert_eq!(space.cells_30.buckets_04, 0x2BD);
    assert_eq!(space.references_64.buckets_04, 0x25);
    assert_eq!(space.map_c8.original_vtable, BOOL_MAP_VTABLE);
    assert_eq!(space.field_dc, 7);
    assert_eq!(space.world_48, Some(form));
    assert_eq!(
        world.resource::<DataLoadGlobals>().current_world_b33aa0,
        Some(form)
    );
    assert_eq!(world.resource::<WorldFormGlobals>().lod_map_count_b360a0, 1);
}

// 004F21D8: +0x5C bit 0 keeps the 37 buckets; a reserved ID's reset leaves it clear and takes 7001.
#[test]
fn the_cell_map_rebuild() {
    let fixture = Fixture::new("cellmap");
    let loader = fixture.run(&cat(&[
        &record(b"WRLD", 0, 0x901, &chunk(b"DATA", &[1])),
        &record(b"WRLD", 0, 0x902, &[]),
        &record(b"WRLD", 0, 0x3C, &chunk(b"DATA", &[0])),
    ]));
    let buckets = |id| {
        loader
            .world
            .get::<TesWorldSpace>(form_at(&loader, id))
            .unwrap()
            .cells_30
            .buckets_04
    };
    assert_eq!(buckets(0x901), 0x25);
    assert_eq!(buckets(0x902), 0x25, "the constructor's reset set bit 0");
    assert_eq!(buckets(0x3C), 0x1B59);
    // 0067B1E0 appends.
    let order: Vec<u32> = loader
        .world
        .resource::<DataHandlerLists>()
        .world_spaces_0c
        .items()
        .map(|form| loader.world.get::<FormIdentity>(form).unwrap().form_id_0c)
        .collect();
    assert_eq!(order, [0x901, 0x902, 0x3C]);
}

// 004CD600 and 0044DEF0: an exterior goes in the map under movsx x << 16 | movzx y, an interior in +0xC0.
#[test]
fn cells_join_the_world_or_the_interior_array() {
    let fixture = Fixture::new("cells");
    let loader = fixture.run(&cat(&[
        &world_record(0x900),
        &exterior(0x901, 0, 3, -2),
        &exterior(0x902, 0x400, 9, 9),
        &record(
            b"CELL",
            0,
            0x903,
            &cat(&[
                &chunk(b"EDID", &text("Inner")),
                &chunk(b"FULL", &text("Inside")),
                &chunk(b"XCLL", &[0xAA; 0x28]),
                &chunk(b"DATA", &[0xC1]),
                &chunk(b"XCLL", &[0xBB; 0x28]),
                &chunk(b"XCLC", &[0xCC; 8]),
                &chunk(b"XOWN", &u32_bytes(0x77)),
                &chunk(b"XQQQ", &u32_bytes(0x88)),
            ]),
        ),
        &interior(0x904, 0),
    ]));
    assert!(loader.warnings.is_empty(), "{:?}", loader.warnings);
    let world = &loader.world;
    let space_form = form_at(&loader, 0x900);
    let space = world.get::<TesWorldSpace>(space_form).unwrap();
    let plain = form_at(&loader, 0x901);
    assert_eq!(
        space.cells_30.get(0x0003_FFFE),
        Some(&plain),
        "the key 0044 computes"
    );
    assert_eq!(space.cells_30.count_0c, 1);
    let persistent = form_at(&loader, 0x902);
    assert_eq!(space.persistent_cell_34, Some(persistent));
    assert_eq!(
        world.get::<TesObjectCell>(persistent).unwrap().world_50,
        Some(space_form)
    );
    let cell = world.get::<TesObjectCell>(plain).unwrap();
    assert_eq!(cell.flags_24, 2, "an exterior takes bit 1");
    assert_eq!(cell.data_3c.unwrap()[0], 3);
    assert_eq!(cell.data_3c.unwrap()[1] as i32, -2);
    assert_eq!(
        world.get::<FormIdentity>(plain).unwrap().flags_08 & 1,
        1,
        "0046A9A0(1)"
    );
    let inner = form_at(&loader, 0x903);
    let cell = world.get::<TesObjectCell>(inner).unwrap();
    assert_eq!(cell.flags_24, 0x81, "0xBF drops bit 6 with +0xCD1 clear");
    assert_eq!(cell.world_50, None);
    let block = cell.data_3c.unwrap();
    assert_eq!(block[0], 0xBBBB_BBBB, "XCLL before DATA has no block");
    assert_eq!(
        block[9], 0xBBBB_BBBB,
        "a later XCLL overwrites 004C9D30's word"
    );
    let plain_inner = form_at(&loader, 0x904);
    let block = world
        .get::<TesObjectCell>(plain_inner)
        .unwrap()
        .data_3c
        .unwrap();
    assert_eq!(block[7], 1.0_f32.to_bits(), "004CA752");
    assert_ne!(block[9], 0, "004C9D30: a master's record offset");
    assert_eq!(block[0], 0);
    let inner = form_at(&loader, 0x903);
    assert_eq!(editor_id_of(world, inner), b"Inner");
    assert_eq!(
        cell.extra_28.get(EXTRA_OWNER).unwrap().data,
        ExtraData::Dword(0x77)
    );
    assert_eq!(cell.extra_28.entries.len(), 2, "XQQQ isn't among the 32");
    let array = &world.resource::<DataHandler>().cells_c0;
    assert_eq!(array.size_0c, 2);
    assert_eq!(array.count_10, 2);
    assert_eq!(array.capacity_08, 100, "the grow-by of 00446F90");
    assert_eq!(array.cells[0], Some(inner));
    assert_eq!(array.cells[1], Some(plain_inner));
    // 004CCBC0: an exterior with no name answers 00B35C0C's GMST, which 0044F2A9 then maps.
    assert_eq!(editor_id_of(world, plain), b"Wilderness");
    assert_eq!(editor_id_of(world, persistent), b"Wilderness");
    let last = world
        .resource::<recomp_game::tes::data_handler::EditorIdMap>()
        .lookup(b"wilderness");
    assert_eq!(last, Some(persistent), "the last one in keeps the name");
}

// 004EFF64: a taken coordinate warns; the cell's deletion is a host stop.
#[test]
fn a_taken_coordinate_warns() {
    let fixture = Fixture::new("taken");
    let mut esm = tes4(true, 0.8, &[]);
    esm.extend(world_record(0x900));
    esm.extend(exterior(0x901, 0, 1, 1));
    esm.extend(exterior(0x902, 0, 1, 1));
    fixture.write("Base.esm", &esm);
    let mut loader = fixture.loader();
    let error = loader.run().unwrap_err();
    assert!(error.0.contains("0044DFED"), "{error:?}");
    assert!(warned(
        &loader,
        "Cell (00000901) Wilderness already exists at coord (1, 1 )."
    ));
    assert!(warned(
        &loader,
        "Error adding cell (00000902) to world space (00000900). Cell will be destroyed."
    ));
}

// 004D9D00 and 004259E0: a REFR's chunks, in a non-persistent interior cell.
#[test]
fn a_reference_loads_its_chunks_and_extras() {
    let fixture = Fixture::new("refr");
    let lock = [
        7_u8, 0xEE, 0xEE, 0xEE, 0x44, 0x33, 0x22, 0x11, 2, 0xEE, 0xEE, 0xEE,
    ];
    let teleport: Vec<u8> = (0..0x1C).collect();
    let loader = fixture.run(&cat(&[
        &interior(0x901, 0),
        &record(
            b"REFR",
            0,
            0x920,
            &cat(&[
                &chunk(b"EDID", &text("Ignored")),
                &name_data(0x700, [1.0, 2.0, 3.0], [4.0, 5.0, 6.0]),
                &chunk(b"XSCL", &f32_bytes(2.5)),
                &chunk(b"XOWN", &u32_bytes(0x111)),
                &chunk(b"XCNT", &u32_bytes(1)),
                &chunk(b"XLOD", &cat(&[&f32_bytes(1.0), &f32_bytes(2.0)])),
                &chunk(b"XLOC", &lock),
                &chunk(b"XTEL", &teleport),
                &chunk(b"XMRK", &[]),
                &chunk(b"FNAM", &[3]),
                &chunk(b"FULL", &text("Marker")),
                &chunk(b"TNAM", &u16_bytes(0x0102)),
                &chunk(b"XESP", &cat(&[&u32_bytes(0x222), &u32_bytes(0xFFFF_FF09)])),
                &chunk(b"ONAM", &[]),
                &chunk(b"XCLR", &[]),
            ]),
        ),
    ]));
    assert!(loader.warnings.is_empty(), "{:?}", loader.warnings);
    let world = &loader.world;
    let form = form_at(&loader, 0x920);
    let cell = form_at(&loader, 0x901);
    let reference = world.get::<TesObjectRefr>(form).unwrap();
    assert_eq!(reference.base_form_1c, Some(form_at(&loader, 0x700)));
    assert_eq!(reference.position_2c, [1.0, 2.0, 3.0], "the first three");
    assert_eq!(reference.rotation_20, [4.0, 5.0, 6.0]);
    assert_eq!(reference.scale_38, 2.5);
    assert_eq!(reference.parent_cell_40, Some(cell));
    assert_eq!(
        world
            .get::<TesObjectCell>(cell)
            .unwrap()
            .objects_48
            .items()
            .collect::<Vec<_>>(),
        [form]
    );
    assert!(
        editor_id_of(world, form).is_empty(),
        "004D6DF0 answers \"\""
    );
    let list = &reference.extra_data_44;
    assert_eq!(list.get(EXTRA_OWNER).unwrap().data, ExtraData::Dword(0x111));
    assert!(list.get(EXTRA_COUNT).is_none(), "a count of 1 adds nothing");
    assert_eq!(
        list.get(EXTRA_LOD).unwrap().data,
        ExtraData::Lod([1.0_f32.to_bits(), 2.0_f32.to_bits(), 0x3F78_51EC]),
        "0.97 under a short chunk"
    );
    let mut stored = lock;
    stored[8] |= 1;
    assert_eq!(
        list.get(EXTRA_LOCK).unwrap().data,
        ExtraData::Lock(Box::new(stored)),
        "00425B0D ORs +8 with 1"
    );
    let ExtraData::Teleport(block) = &list.get(EXTRA_TELEPORT).unwrap().data else {
        panic!("a teleport block");
    };
    assert_eq!(block[0], u32::from_le_bytes([0, 1, 2, 3]));
    let ExtraData::MapMarker(marker) = &list.get(EXTRA_MAP_MARKER).unwrap().data else {
        panic!("a marker block");
    };
    assert_eq!(marker.flags_0c, 3);
    assert_eq!(marker.name_04.data_00.as_deref(), Some(&b"Marker"[..]));
    assert_eq!(marker.marker_type_0e, 0x0102);
    assert_eq!(
        list.get(EXTRA_ENABLE_STATE_PARENT).unwrap().data,
        ExtraData::EnableStateParent {
            parent_0c: 0x222,
            flags_10: 9
        },
        "00420380 stores the low byte"
    );
    assert_eq!(
        list.get(EXTRA_ACTION).unwrap().data,
        ExtraData::Action {
            flags_0c: 9,
            ref_10: 0
        },
        "ONAM ORs 8 into the absent 1"
    );
    assert!(
        list.get(EXTRA_REGION_LIST).is_some(),
        "an empty XCLR still stores a list"
    );
    // 0041E0A0: 0x32 and 0x13 go in front, in reverse order of arrival.
    assert_eq!(list.entries[0].type_04, EXTRA_ACTION);
    assert_eq!(list.entries[1].type_04, EXTRA_TELEPORT);
}

// 00425C4E 00497470; 00425A89's other sizes; 00425FEF.
#[test]
fn the_extra_failure_arms() {
    let fixture = Fixture::new("extras");
    let loader = fixture.run(&cat(&[
        &interior(0x901, 0),
        &record(
            b"REFR",
            0,
            0x920,
            &cat(&[
                &name_data(0x700, [0.0; 3], [0.0; 3]),
                &chunk(b"XLOC", &[0; 5]),
                &chunk(b"XCLR", &[0; 5]),
                &chunk(b"XRGD", &[0x5A; 0x39]),
                &chunk(
                    b"XLOC",
                    &[3, 0, 0, 0, 1, 0, 0, 0, 0xFF, 0, 0, 0, 2, 0, 0, 0],
                ),
            ]),
        ),
        &record(
            b"REFR",
            0,
            0x921,
            &cat(&[
                &name_data(0x700, [0.0; 3], [0.0; 3]),
                &chunk(b"XRGD", &[0x5A; 0x38]),
            ]),
        ),
    ]));
    assert!(warned(
        &loader,
        "Unrecognized format for lock data in file 'Base.esm'."
    ));
    assert!(warned(
        &loader,
        "Invalid Extra Data - Region List in file \"Base.esm\"."
    ));
    assert!(warned(&loader, "Failed to load RagDoll Data."));
    let world = &loader.world;
    let list = &world
        .get::<TesObjectRefr>(form_at(&loader, 0x920))
        .unwrap()
        .extra_data_44;
    assert!(
        list.get(EXTRA_RAG_DOLL).is_none(),
        "0x39 % 2 fails, and the entry goes"
    );
    let mut expected = [0_u8; 12];
    expected[0] = 3;
    expected[4] = 1;
    expected[8] = 2 | 1;
    assert_eq!(
        list.get(EXTRA_LOCK).unwrap().data,
        ExtraData::Lock(Box::new(expected)),
        "a 0x10 lock takes bytes +0, +4 and +0xC"
    );
    let list = &world
        .get::<TesObjectRefr>(form_at(&loader, 0x921))
        .unwrap()
        .extra_data_44;
    let ExtraData::RagDoll(block) = &list.get(EXTRA_RAG_DOLL).unwrap().data else {
        panic!("a ragdoll block");
    };
    assert_eq!(block.count_00, 2);
    assert_eq!(block.data_04.as_ref().unwrap().len(), 0x38);
}

// 004F0120, 004F00C0 and 004EFE40, through 004D35D0's persistent arm.
#[test]
fn a_persistent_reference_joins_its_world() {
    let fixture = Fixture::new("persistent");
    let loader = fixture.run(&cat(&[
        &world_record(0x900),
        &exterior(0x901, 0x400, 0, 0),
        // STAT 0x1C is keyed. fistp rounds to nearest even: 4096.5 is 4096, 4097.5 is 4098.
        &record(
            b"REFR",
            0x400,
            0x920,
            &name_data(0x700, [4096.5, -1.0, 0.0], [0.0; 3]),
        ),
        // MISC 0x1B isn't.
        &record(
            b"REFR",
            0x400,
            0x921,
            &name_data(0x701, [1.0, 1.0, 1.0], [0.0; 3]),
        ),
    ]));
    assert!(loader.warnings.is_empty(), "{:?}", loader.warnings);
    let world = &loader.world;
    let space = world.get::<TesWorldSpace>(form_at(&loader, 0x900)).unwrap();
    let keyed = form_at(&loader, 0x920);
    let key = (1_u32 << 16) | 0xFFFF;
    assert_eq!(
        space
            .references_64
            .get(key)
            .unwrap()
            .items()
            .collect::<Vec<_>>(),
        [keyed]
    );
    assert_eq!(space.references_64.count_0c, 1);
    assert_eq!(
        space.unsorted_74.items().collect::<Vec<_>>(),
        [form_at(&loader, 0x921)]
    );
    let cell = form_at(&loader, 0x901);
    let reference = world.get::<TesObjectRefr>(keyed).unwrap();
    assert_eq!(reference.parent_cell_40, None, "the extra holds the cell");
    assert_eq!(reference.extra_data_44.persistent_cell(), Some(cell));
    assert_eq!(
        reference.extra_data_44.entries[0].type_04,
        EXTRA_PERSISTENT_CELL
    );
    assert_eq!(
        world
            .get::<TesObjectCell>(cell)
            .unwrap()
            .objects_48
            .items()
            .next(),
        Some(form_at(&loader, 0x921)),
        "00446CB0 prepends"
    );
}

// 0044E05D: a failed load, in 0044DCF0's REFR arm.
#[test]
fn a_reference_with_no_base_goes_to_8b8() {
    let fixture = Fixture::new("nobase");
    let loader = fixture.run(&cat(&[
        &interior(0x901, 0),
        &record(
            b"REFR",
            0,
            0x920,
            &cat(&[
                &name_data(0x7FF, [1.0, 1.0, 1.0], [2.0; 3]),
                &chunk(b"QQQQ", &[]),
            ]),
        ),
        &record(b"REFR", 0x20, 0x921, &[]),
    ]));
    assert!(warned(
        &loader,
        "Missing object (000007FF) for reference (00000920)"
    ));
    assert!(warned(
        &loader,
        "TESObjectREFR::Load abnormally terminated."
    ));
    let world = &loader.world;
    let form = form_at(&loader, 0x920);
    let reference = world.get::<TesObjectRefr>(form).unwrap();
    assert_eq!(reference.base_form_1c, None);
    assert_eq!(reference.position_2c, [0.0; 3], "a missing base skips DATA");
    let cell = form_at(&loader, 0x901);
    assert_eq!(reference.parent_cell_40, Some(cell), "vt+0x194");
    assert_eq!(
        world
            .get::<TesObjectCell>(cell)
            .unwrap()
            .objects_48
            .items()
            .count(),
        0,
        "not inserted"
    );
    let order: Vec<u32> = world
        .resource::<DataHandlerLists>()
        .list_8b8
        .items()
        .map(|form| world.get::<FormIdentity>(form).unwrap().form_id_0c)
        .collect();
    assert_eq!(order, [0x921, 0x920]);
}

// 0044E001: no current cell returns 0, which 0044F0C0 counts.
#[test]
fn a_reference_with_no_cell_fails_the_file() {
    let fixture = Fixture::new("nocell");
    let loader = fixture.run(&record(
        b"REFR",
        0,
        0x920,
        &name_data(0x700, [0.0; 3], [0.0; 3]),
    ));
    assert_eq!(loader.world.resource::<DataLoadGlobals>().file_results, [0]);
    assert!(warned(&loader, "DataHandler: unrecognized form"));
    assert!(loader.world.resource::<FormsById>().lookup(0x920).is_none());
}

// 0044E194: below a file version of 0.2, a persistent reference warns and loses flag 0x400.
#[test]
fn an_old_file_loses_its_persistent_flag() {
    let fixture = Fixture::new("old");
    let mut esm = tes4(true, 0.1, &[]);
    esm.extend(interior(0x901, 0));
    esm.extend(record(
        b"REFR",
        0x400,
        0x920,
        &name_data(0x700, [0.0; 3], [0.0; 3]),
    ));
    fixture.write("Base.esm", &esm);
    let mut loader = fixture.loader();
    assert_eq!(loader.run(), Ok(1));
    assert!(warned(
        &loader,
        "ref (00000920) to base object  (00000700) in cell  (0, 0) (00000901) is persistent in the non-persistent file Base.esm."
    ));
    let flags = loader
        .world
        .get::<FormIdentity>(form_at(&loader, 0x920))
        .unwrap()
        .flags_08;
    assert_eq!(flags & 0x400, 0);
}

// 004DB260's arms, 005E1880's add, and 004DA040's removal of a disabled actor.
#[test]
fn actor_references_take_a_process() {
    let fixture = Fixture::new("actors");
    let loader = fixture.run(&cat(&[
        &interior(0x901, 0),
        &record(b"ACHR", 0, 0x920, &name_data(0x700, [1.0; 3], [0.0; 3])),
        &record(b"ACRE", 0, 0x921, &name_data(0x700, [1.0; 3], [0.0; 3])),
        &record(b"ACHR", 0x800, 0x922, &name_data(0x700, [1.0; 3], [0.0; 3])),
    ]));
    assert!(loader.warnings.is_empty(), "{:?}", loader.warnings);
    let world = &loader.world;
    let character = form_at(&loader, 0x920);
    let creature = form_at(&loader, 0x921);
    let disabled = form_at(&loader, 0x922);
    assert_eq!(
        world.get::<OriginalVtable>(character).unwrap().0,
        CHARACTER_VTABLES[0]
    );
    assert_eq!(
        world
            .get::<TesObjectRefr>(creature)
            .unwrap()
            .child_cell_vtable_18,
        CREATURE_VTABLES[1]
    );
    assert_eq!(
        world.get::<FormIdentity>(creature).unwrap().form_type_04,
        0x33
    );
    assert_eq!(
        world.get::<FormIdentity>(character).unwrap().flags_08 & 0x0020_0000,
        0,
        "00451210 replaces 005E191F's flag with the record's, as the run's samples show"
    );
    assert!(
        world
            .get::<MobileObject>(character)
            .unwrap()
            .process_58
            .is_some()
    );
    assert_eq!(
        world.get::<MobileObject>(disabled).unwrap().process_58,
        None
    );
    let manager = &world.resource::<ActorProcessGlobals>().manager;
    assert_eq!(
        manager.list(ActorListId::Low18).list.items(),
        vec![creature, character],
        "00673A90 prepends, and the disabled one left"
    );
    assert_eq!(
        manager.queued_58.items().count(),
        0,
        "006748B0 found nothing"
    );
    assert_eq!(
        world
            .get::<TesObjectRefr>(character)
            .unwrap()
            .parent_cell_40,
        Some(form_at(&loader, 0x901))
    );
}

// 0065AA60's 006748B0: a sleeping player queues every actor at its DATA chunk, with flag 0x20000.
#[test]
fn a_sleeping_player_queues_an_actor() {
    let fixture = Fixture::new("sleeping");
    let mut esm = tes4(true, 0.8, &[]);
    esm.extend(interior(0x901, 0));
    esm.extend(record(
        b"ACHR",
        0,
        0x920,
        &name_data(0x700, [1.0; 3], [0.0; 3]),
    ));
    fixture.write("Base.esm", &esm);
    let mut loader = fixture.loader();
    loader.host.pre_load = |world| {
        install_player(world);
        bound_object(world, 0x1C, 0x700);
        let player = world.resource::<PlayerGlobals>().the_player_b333c4.unwrap();
        world
            .get_mut::<PlayerCharacter>(player)
            .unwrap()
            .pc_sleep_hours_590 = 1;
    };
    assert_eq!(loader.run(), Ok(1));
    let form = form_at(&loader, 0x920);
    let world = &loader.world;
    assert_eq!(
        world
            .resource::<ActorProcessGlobals>()
            .manager
            .queued_58
            .items()
            .collect::<Vec<_>>(),
        [form]
    );
    assert_eq!(
        world.get::<FormIdentity>(form).unwrap().flags_08 & 0x0002_0000,
        0x0002_0000
    );
}

// 004D9B50 and 004D6BF0: a second file's record resets the first's: only the persistent cell stays. 004CECD0 takes it from its old cell when the cell changes, and the reference goes in the new one.
#[test]
fn an_override_resets_and_moves_a_reference() {
    let fixture = Fixture::new("override");
    let mut esm = tes4(true, 0.8, &[]);
    esm.extend(interior(0x902, 0x400));
    esm.extend(interior(0x903, 0x400));
    esm.extend(interior(0x901, 0x400));
    esm.extend(record(
        b"REFR",
        0x400,
        0x920,
        &cat(&[
            &name_data(0x700, [1.0; 3], [0.0; 3]),
            &chunk(b"XSCL", &f32_bytes(3.0)),
            &chunk(b"XOWN", &u32_bytes(0x111)),
        ]),
    ));
    fixture.write("Base.esm", &esm);
    let mut plugin = tes4(false, 0.8, &["Base.esm"]);
    plugin.extend(record(b"CELL", 0x400, 0x902, &chunk(b"DATA", &[1])));
    plugin.extend(record(
        b"REFR",
        0x400,
        0x920,
        &name_data(0x700, [9.0; 3], [0.0; 3]),
    ));
    fixture.write("Plug.esp", &plugin);
    let mut loader = fixture.loader();
    assert_eq!(loader.run(), Ok(1));
    assert!(loader.warnings.is_empty(), "{:?}", loader.warnings);
    let world = &loader.world;
    let form = form_at(&loader, 0x920);
    let reference = world.get::<TesObjectRefr>(form).unwrap();
    assert_eq!(reference.position_2c, [9.0; 3]);
    assert_eq!(reference.scale_38, 1.0, "004D6BF0");
    assert!(
        reference.extra_data_44.get(EXTRA_OWNER).is_none(),
        "0041DFF0"
    );
    assert_eq!(
        reference.extra_data_44.persistent_cell(),
        Some(form_at(&loader, 0x902))
    );
    let cell = |id| {
        world
            .get::<TesObjectCell>(form_at(&loader, id))
            .unwrap()
            .objects_48
            .items()
            .count()
    };
    assert_eq!(cell(0x901), 0, "004CECD0");
    assert_eq!(cell(0x902), 1);
    assert_eq!(cell(0x903), 0);
    assert_eq!(
        world.get::<ModReferences>(form).unwrap().0,
        [0, 1],
        "both files"
    );
}

// 004CED63: a non-persistent reference leaving a non-persistent cell reaches the cell's vt+0x90(1), which needs the 00B06150 array: a host stop, which no shipped record reaches.
#[test]
fn moving_a_non_persistent_reference_stops() {
    let fixture = Fixture::new("move");
    let mut esm = tes4(true, 0.8, &[]);
    esm.extend(interior(0x902, 0));
    esm.extend(interior(0x901, 0));
    esm.extend(record(
        b"REFR",
        0,
        0x920,
        &name_data(0x700, [1.0; 3], [0.0; 3]),
    ));
    fixture.write("Base.esm", &esm);
    let mut plugin = tes4(false, 0.8, &["Base.esm"]);
    plugin.extend(record(b"CELL", 0, 0x902, &chunk(b"DATA", &[1])));
    plugin.extend(record(
        b"REFR",
        0,
        0x920,
        &name_data(0x700, [9.0; 3], [0.0; 3]),
    ));
    fixture.write("Plug.esp", &plugin);
    let mut loader = fixture.loader();
    assert!(loader.run().unwrap_err().0.contains("004CED63"));
}

// 0041E450, through a cell's own override: the presence bits stay.
#[test]
fn an_overridden_cell_keeps_stale_presence_bits() {
    let fixture = Fixture::new("cellreset");
    let mut esm = tes4(true, 0.8, &[]);
    esm.extend(record(
        b"CELL",
        0,
        0x901,
        &cat(&[&chunk(b"DATA", &[1]), &chunk(b"XOWN", &u32_bytes(5))]),
    ));
    fixture.write("Base.esm", &esm);
    let mut plugin = tes4(false, 0.8, &["Base.esm"]);
    plugin.extend(record(b"CELL", 0, 0x901, &chunk(b"DATA", &[1])));
    fixture.write("Plug.esp", &plugin);
    let mut loader = fixture.loader();
    assert_eq!(loader.run(), Ok(1));
    let world = &loader.world;
    let cell = world.get::<TesObjectCell>(form_at(&loader, 0x901)).unwrap();
    assert!(cell.extra_28.entries.is_empty());
    assert_eq!(cell.extra_28.presence_08[4], 0x80, "0x27 keeps its bit");
    assert!(cell.data_3c.is_some(), "the new DATA's block");
}

// 0044E237 and 004C7090.
#[test]
fn land_goes_on_an_exterior_cell() {
    let fixture = Fixture::new("land");
    let loader = fixture.run(&cat(&[
        &record(
            b"WRLD",
            0,
            0x900,
            &cat(&[&chunk(b"WNAM", &u32_bytes(0x1)), &chunk(b"DATA", &[1])]),
        ),
        &exterior(0x901, 0, 0, 0),
        &record(
            b"LAND",
            0,
            0x930,
            &cat(&[
                &chunk(b"DATA", &u32_bytes(0x0000_040F)),
                &chunk(b"VHGT", &[0xFF; 8]),
            ]),
        ),
        &interior(0x902, 0),
        &record(b"LAND", 0, 0x931, &chunk(b"DATA", &u32_bytes(0x10))),
    ]));
    assert!(loader.warnings.is_empty(), "{:?}", loader.warnings);
    let world = &loader.world;
    let land = form_at(&loader, 0x930);
    let data = world.get::<TesObjectLand>(land).unwrap();
    assert_eq!(
        data.flags_1c, 0x407,
        "bit 3 clears and the world's WNAM set bit 10"
    );
    assert_eq!(data.cell_20, Some(form_at(&loader, 0x901)));
    assert_eq!(
        world
            .get::<TesObjectCell>(data.cell_20.unwrap())
            .unwrap()
            .land_40,
        Some(land)
    );
    let inner = form_at(&loader, 0x931);
    assert_eq!(world.get::<TesObjectLand>(inner).unwrap().flags_1c, 0x10);
    assert_eq!(
        world
            .get::<TesObjectCell>(form_at(&loader, 0x902))
            .unwrap()
            .land_40,
        None,
        "004C9AE0 ignores an interior"
    );
    let globals = world.resource::<WorldFormGlobals>();
    assert_eq!(globals.land_count_b35be0, 2);
    assert_eq!(globals.land_statics_built_b35bdc, 1);
}

// 004C70E1: a reused land with a new ID warns, with the arm's cell +0x40.
#[test]
fn a_second_land_on_a_cell_warns() {
    let fixture = Fixture::new("land2");
    let loader = fixture.run(&cat(&[
        &world_record(0x900),
        &exterior(0x901, 0, 0, 0),
        &record(b"LAND", 0, 0x930, &[]),
        &record(b"LAND", 0, 0x931, &[]),
    ]));
    assert!(warned(
        &loader,
        "Potentially duplicate Land (00000931) encountered in file Base.esm."
    ));
    assert!(loader.world.resource::<FormsById>().lookup(0x930).is_none());
}

// 0044E2A5 and 004E74E0.
#[test]
fn a_path_grid_goes_on_its_cell() {
    let fixture = Fixture::new("pgrd");
    let loader = fixture.run(&cat(&[
        &record(b"PGRD", 0, 0x940, &[]),
        &interior(0x901, 0),
        &record(
            b"PGRD",
            0,
            0x941,
            &cat(&[
                &chunk(b"DATA", &u16_bytes(5)),
                &chunk(b"PGRP", &[0; 16]),
                &chunk(b"EDID", &text("Ignored")),
            ]),
        ),
        &interior(0x902, 0),
        &record(b"PGRD", 0x20, 0x942, &chunk(b"DATA", &u16_bytes(5))),
    ]));
    assert!(warned(&loader, "PathGrid not associated with cell."));
    assert_eq!(loader.world.resource::<DataLoadGlobals>().file_results, [0]);
    let world = &loader.world;
    let grid = form_at(&loader, 0x941);
    let data = world.get::<TesPathGrid>(grid).unwrap();
    assert_eq!(data.point_count_30, 5);
    assert_eq!(data.cell_20, Some(form_at(&loader, 0x901)));
    assert_eq!(
        world
            .get::<TesObjectCell>(data.cell_20.unwrap())
            .unwrap()
            .path_grid_44,
        Some(grid)
    );
    assert_eq!(
        world
            .get::<TesPathGrid>(form_at(&loader, 0x942))
            .unwrap()
            .point_count_30,
        0,
        "flag 0x20"
    );
    assert_eq!(
        world.resource::<WorldFormGlobals>().path_grid_count_b35f80,
        2
    );
}

// 0044DE83, 004E9CA0, 004E9060, 0047D810 and 004BEFE0.
#[test]
fn a_road_builds_its_points() {
    let fixture = Fixture::new("road");
    let point = |x: f32, y: f32, links: u8| {
        cat(&[
            &f32_bytes(x),
            &f32_bytes(y),
            &f32_bytes(0.0),
            &[links, 0, 0, 0],
        ])
    };
    let link = |x: f32, y: f32| cat(&[&f32_bytes(x), &f32_bytes(y), &f32_bytes(0.0)]);
    let loader = fixture.run(&cat(&[
        &world_record(0x900),
        &record(
            b"ROAD",
            0,
            0x950,
            &cat(&[
                &chunk(b"EDID", &text("Ignored")),
                &chunk(
                    b"PGRP",
                    &cat(&[
                        &point(0.0, 0.0, 1),
                        // Within 2.0 of the first, so it isn't mapped.
                        &point(1.0, 1.0, 0),
                        &point(5000.0, 0.0, 1),
                    ]),
                ),
                &chunk(b"PGRR", &cat(&[&link(5000.0, 0.0), &link(0.0, 0.0)])),
            ]),
        ),
    ]));
    assert!(loader.warnings.is_empty(), "{:?}", loader.warnings);
    let world = &loader.world;
    let form = form_at(&loader, 0x950);
    let road = world.get::<TesRoad>(form).unwrap();
    assert_eq!(road.points.len(), 3);
    assert_eq!(road.points_18, 2);
    assert_eq!(road.map_1c.count_0c, 2, "keys 0 and 1 << 16");
    assert_eq!(
        road.points[0].connections_20.items().collect::<Vec<_>>(),
        [2]
    );
    assert_eq!(
        road.points[2].connections_20.items().collect::<Vec<_>>(),
        [0]
    );
    assert_eq!(road.points[1].connections_20.items().count(), 0);
    let space = form_at(&loader, 0x900);
    assert_eq!(road.world_2c, Some(space));
    assert_eq!(
        world.get::<TesWorldSpace>(space).unwrap().road_54,
        Some(form)
    );
}

// 0044F1C2, 004EF030 and 004C9D20, by the first current form of the three.
#[test]
fn the_toft_offset_arms() {
    let fixture = Fixture::new("toft");
    // minX 0, minY 0, maxX 4096, maxY 4096: two by two words, an index width * y + x.
    let mut world_chunks = chunk(b"NAM0", &cat(&[&f32_bytes(0.0), &f32_bytes(0.0)]));
    world_chunks.extend(chunk(
        b"NAM9",
        &cat(&[&f32_bytes(4096.0), &f32_bytes(4096.0)]),
    ));
    world_chunks.extend(chunk(
        b"OFST",
        &cat(&[
            &u32_bytes(10),
            &u32_bytes(0),
            &u32_bytes(30),
            &u32_bytes(40),
        ]),
    ));
    let loader = fixture.run(&cat(&[
        &interior(0x901, 0),
        &record(b"TOFT", 0, 0x77, &[]),
        &record(b"WRLD", 0, 0x900, &world_chunks),
    ]));
    let mut world = loader.world;
    let inner = world.resource::<FormsById>().lookup(0x901).unwrap();
    assert_eq!(
        world.get::<TesObjectCell>(inner).unwrap().toft_50,
        0x77,
        "the cell was current then"
    );
    let space = world.resource::<FormsById>().lookup(0x900).unwrap();
    let mut load = world.resource_mut::<DataLoadGlobals>();
    load.current_world_b33aa0 = Some(space);
    load.current_cell_b33a9c = Some(inner);
    toft_offset_arm(&mut world, 0x105, 0x100).unwrap();
    assert_eq!(
        world
            .get::<TesWorldSpace>(space)
            .unwrap()
            .offsets_a8
            .as_deref(),
        Some(&[15, 0, 35, 45][..]),
        "a zero word stays"
    );
    assert_eq!(world.get::<TesObjectCell>(inner).unwrap().toft_50, 0x77);
    // A current INFO comes first: its +0x34, the slot INFO's own load and reset write, takes the ID whole.
    let info = world.spawn(TesTopicInfo::default()).id();
    world.resource_mut::<DataLoadGlobals>().current_info_b33aa4 = Some(info);
    toft_offset_arm(&mut world, 0x999, 0x100).unwrap();
    assert_eq!(
        world.get::<TesTopicInfo>(info).unwrap().record_offset_34,
        0x999
    );
    assert_eq!(
        world
            .get::<TesWorldSpace>(space)
            .unwrap()
            .offsets_a8
            .as_deref(),
        Some(&[15, 0, 35, 45][..]),
        "the world's arm didn't run"
    );
}

#[test]
fn the_classes_agree_with_the_map() {
    check_classes_against_the_map().unwrap();
}
