//! Phase 12b, Family B: synthetic plugins for the arms P12-RUN-001 and P12-RUN-003 didn't take. Every expected value here comes from reading the named function's disassembly, not from a run. Host tests earn no validation bonus.

mod common;

use std::path::PathBuf;

use bevy_ecs::prelude::*;
use common::load::Loader;
use recomp_game::tes::actor::TesObjectRefr;
use recomp_game::tes::data_handler::{DataHandlerLists, FormsById, TesTexture};
use recomp_game::tes::form::{FormIdentity, ModReferences, OriginalVtable};
use recomp_game::tes::form_arms::form_class;
use recomp_game::tes::form_components::{
    ComponentSlot, FormRef, TES_ATTRIBUTES_VTABLE, TesActorBaseData, TesAiForm, TesAnimation,
    TesAttackDamageForm, TesAttributes, TesContainer, TesFullName, TesHealthForm, TesLeveledList,
    TesModel, TesModelList, TesReactionForm, TesScriptableForm, TesSpellList,
};
use recomp_game::tes::forms_actors::{
    ActorBaseValues, CreatureSoundsSlot, TesCreature, TesNpc, TesRace,
};
use recomp_game::tes::forms_magic::{
    EffectItemList, EffectSetting, EffectSettingCollection, MagicFormData, SCRIPT_EFFECT_CODE,
};
use recomp_game::tes::forms_script::Script;
use recomp_game::tes::player::PlayerGlobals;

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

fn record(kind: &[u8; 4], id: u32, payload: &[u8]) -> Vec<u8> {
    cat(&[
        kind,
        &u32_bytes(payload.len() as u32),
        &u32_bytes(0),
        &u32_bytes(id),
        &u32_bytes(0),
        payload,
    ])
}

fn group(label: &[u8; 4], contents: &[u8]) -> Vec<u8> {
    cat(&[
        b"GRUP",
        &u32_bytes(contents.len() as u32 + 20),
        label,
        &u32_bytes(0),
        &u32_bytes(0),
        contents,
    ])
}

fn tes4(master: bool, masters: &[&str]) -> Vec<u8> {
    let mut payload = chunk(
        b"HEDR",
        &cat(&[&f32_bytes(0.8), &u32_bytes(100), &u32_bytes(0x800)]),
    );
    for name in masters {
        payload.extend(chunk(b"MAST", &text(name)));
        payload.extend(chunk(b"DATA", &0_u64.to_le_bytes()));
    }
    cat(&[
        b"TES4",
        &u32_bytes(payload.len() as u32),
        &u32_bytes(u32::from(master)),
        &u32_bytes(0),
        &u32_bytes(0),
        &payload,
    ])
}

/// IDA: N/A a temp working directory with `Data\` plugins.
struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("recomp_phase12b_{}_{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("Data")).unwrap();
        Self { root }
    }

    fn write(&self, name: &str, bytes: &[u8]) {
        std::fs::write(self.root.join("Data").join(name), bytes).unwrap();
    }

    // IDA: N/A one master holding these records, with `pre_load` standing in for what a load finds in place.
    fn run_records(&self, records: &[u8], pre_load: fn(&mut World)) -> Loader {
        let mut esm = tes4(true, &[]);
        esm.extend_from_slice(records);
        self.write("Base.esm", &esm);
        let mut loader = Loader::new(&self.root);
        loader.load_all();
        loader.host.pre_load = pre_load;
        assert_eq!(loader.run(), Ok(1));
        loader
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn form_at(loader: &Loader, id: u32) -> Entity {
    loader
        .world
        .resource::<FormsById>()
        .lookup(id)
        .unwrap_or_else(|| panic!("no form {id:08X}"))
}

// IDA: N/A a form another family's class would build, with only its identity.
fn stand_in(world: &mut World, form_type: u8, id: u32) {
    let form = world
        .spawn((
            FormIdentity {
                form_type_04: form_type,
                flags_08: 0,
                form_id_0c: id,
            },
            OriginalVtable(0),
            ModReferences::default(),
        ))
        .id();
    world.resource_mut::<FormsById>().set_at(id, form);
}

fn warned(loader: &Loader, want: &str) -> bool {
    loader.warnings.iter().any(|line| line == want)
}

fn raw_ids(list: &[FormRef]) -> Vec<u32> {
    list.iter()
        .map(|slot| match slot {
            FormRef::Raw(id) => *id,
            FormRef::Resolved(_) => panic!("raw until InitItem"),
        })
        .collect()
}

/// 00417420's table, as the map holds it: pushes read from the exe at 00417FF0 for the row with a push past its count.
#[test]
fn the_built_in_effect_settings() {
    let fixture = Fixture::new("built_in");
    let loader = fixture.run_records(&[], |_| {});
    let world = &loader.world;
    let collection = world.resource::<EffectSettingCollection>();
    assert_eq!(collection.count_0c, 161);
    assert_eq!(collection.built_in.len(), 161);
    // 'ABHE' at 00417FF0: school 5, associated 8, flags 0x1000025, resist 0x40, four counters, and an unread fifth push.
    let form = collection.lookup(0x4548_4241).unwrap();
    let setting = world.get::<EffectSetting>(form).unwrap();
    assert_eq!(setting.code_98, 0x4548_4241);
    assert_eq!(setting.data_58.school_64, 5);
    assert_eq!(setting.data_58.associated_60, 8);
    assert_eq!(setting.data_58.flags_58, 0x0100_0025);
    assert_eq!(setting.data_58.resist_68, 0x40);
    assert_eq!(setting.data_58.counter_count_6c, 4);
    assert_eq!(
        setting.counters_9c.as_deref(),
        Some(&[0x4C50_5344, 0x4548_4241, 0x4548_5244, 0x4548_4744][..])
    );
    // 00416430's defaults past the row; before cinit 201 and 202 the two floats are 0.
    assert_eq!(setting.data_58.projectile_speed_74, 1.0);
    assert_eq!(setting.data_58.enchant_factor_90, 0.0);
    assert_eq!(setting.data_58.barter_factor_94, 0.0);
    let identity = world.get::<FormIdentity>(form).unwrap();
    assert_eq!((identity.form_type_04, identity.form_id_0c), (0x0C, 0));
    assert_eq!(world.get::<ModReferences>(form).unwrap().0, [0_u8; 0]);
    assert!(collection.lookup(SCRIPT_EFFECT_CODE).is_some());
    assert!(
        world
            .get::<TesFullName>(form)
            .unwrap()
            .name_04
            .data_00
            .is_some()
    );
}

// IDA: 0044E367: an MGEF record's first chunk names the code.
fn mgef(id: u32, code: &[u8], rest: &[u8]) -> Vec<u8> {
    let mut name = code.to_vec();
    name.push(0);
    record(b"MGEF", id, &cat(&[&chunk(b"EDID", &name), rest]))
}

/// 00416100's DATA arm: which bits the record gives, and 0x1000000 keeping the setting's +0x60.
#[test]
fn a_built_in_mgef_merges_its_flags() {
    let fixture = Fixture::new("mgef");
    let fixture_probe = Fixture::new("mgef_probe");
    let probe = fixture_probe.run_records(&[], |_| {});
    let flags_rehe = {
        let form = probe
            .world
            .resource::<EffectSettingCollection>()
            .lookup(0x4548_4552)
            .unwrap();
        probe.world.get::<EffectSetting>(form).unwrap().data_58
    };
    drop(probe);
    let mut data = vec![0_u8; 0x40];
    // Bits the masks take from the record: 0x400 and 0x800000; a bit they keep from the setting: 0x1. Bit 21 is always cleared.
    data[..4].copy_from_slice(&(0x0020_0400_u32 | 0x0080_0000 | 0x1).to_le_bytes());
    data[4..8].copy_from_slice(&2.5_f32.to_le_bytes());
    data[8..0x0C].copy_from_slice(&0x77_u32.to_le_bytes());
    data[0x14..0x16].copy_from_slice(&2_u16.to_le_bytes());
    data[0x1C..0x20].copy_from_slice(&9.0_f32.to_le_bytes());
    let rest = cat(&[
        &chunk(b"DATA", &data),
        &chunk(b"ESCE", &cat(&[&u32_bytes(0x1111), &u32_bytes(0x2222)])),
    ]);
    let loader = fixture.run_records(&group(b"MGEF", &mgef(0x900, b"REHE", &rest)), |_| {});
    let world = &loader.world;
    let form = world
        .resource::<EffectSettingCollection>()
        .lookup(0x4548_4552)
        .unwrap();
    assert_eq!(
        form_at(&loader, 0x900),
        form,
        "0046C300 on the built-in form"
    );
    let setting = world.get::<EffectSetting>(form).unwrap();
    let old = flags_rehe.flags_58;
    let mut want = old & !(0x400 | 0x800 | 0x1000 | 0x2000 | 0x0800_0000 | 0x07E0_0000);
    want |= 0x400 | 0x0080_0000;
    if old & 0x0100_0000 != 0 {
        want |= 0x0100_0000;
        assert_eq!(setting.data_58.associated_60, flags_rehe.associated_60);
    } else {
        assert_eq!(setting.data_58.associated_60, 0x77);
    }
    assert_eq!(setting.data_58.flags_58, want);
    assert_eq!(setting.data_58.base_cost_5c, 2.5);
    assert_eq!(setting.data_58.projectile_speed_74, 9.0);
    assert_eq!(setting.data_58.counter_count_6c, 2);
    assert_eq!(setting.counters_9c.as_deref(), Some(&[0x1111, 0x2222][..]));
    assert_eq!(
        world
            .resource::<DataHandlerLists>()
            .list_8b8
            .items()
            .count(),
        0
    );
}

/// 0044E3B1: a code no setting has gets a new form with no LoadForm, on +0x8B8, with +0x90 and +0x94 from the two GameSettings. The C++ stores 0.0.
#[test]
fn an_unknown_mgef_is_built_from_the_game_settings() {
    let fixture = Fixture::new("mgef_new");
    let loader = fixture.run_records(
        &group(
            b"MGEF",
            &cat(&[
                &mgef(0x900, b"ZZZZ", &chunk(b"FULL", &text("Ignored"))),
                &mgef(0x901, b"ZZ", &[]),
                // A first chunk that isn't EDID: nothing at all.
                &record(b"MGEF", 0x902, &chunk(b"FULL", &text("Ignored"))),
            ]),
        ),
        |_| {},
    );
    let world = &loader.world;
    let list: Vec<Entity> = world
        .resource::<DataHandlerLists>()
        .list_8b8
        .items()
        .collect();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0], form_at(&loader, 0x901), "00446CB0 prepends");
    assert_eq!(list[1], form_at(&loader, 0x900));
    let setting = world.get::<EffectSetting>(list[1]).unwrap();
    assert_eq!(setting.code_98, 0xFFFF_FFFF, "no LoadForm");
    assert_eq!(
        setting.data_58.enchant_factor_90, 1.0,
        "00B33704's image value"
    );
    assert_eq!(
        setting.data_58.barter_factor_94, 1.0,
        "00B3370C's image value"
    );
    assert_eq!(
        world.get::<TesFullName>(list[1]).unwrap().name_04.data_00,
        None
    );
    assert!(world.resource::<FormsById>().lookup(0x902).is_none());
}

// IDA: an EFID and its EFIT for a code.
fn efid(code: u32, data: [u32; 5]) -> Vec<u8> {
    let mut efit = u32_bytes(code);
    for word in data {
        efit.extend(u32_bytes(word));
    }
    cat(&[&chunk(b"EFID", &u32_bytes(code)), &chunk(b"EFIT", &efit)])
}

/// 004154B0 and 00413130: an item's data, an unknown code, a scripted effect, and the chunk after SCIT that isn't FULL, which the caller's advance passes over.
#[test]
fn the_effect_chunks_of_a_spell() {
    let fixture = Fixture::new("effects");
    let enit: Vec<u8> = (0..0x10).collect();
    let spit: Vec<u8> = (0x20..0x30).collect();
    let seff = efid(SCRIPT_EFFECT_CODE, [1, 2, 3, 0, 7]);
    let scit = chunk(
        b"SCIT",
        &cat(&[
            &u32_bytes(0x0123),
            &u32_bytes(5),
            &u32_bytes(0x4548_4552),
            &u32_bytes(0x0000_0101),
        ]),
    );
    let records = cat(&[
        &group(
            b"ENCH",
            &cat(&[
                &record(
                    b"ENCH",
                    0x900,
                    &cat(&[
                        &chunk(b"FULL", &text("Ench")),
                        &efid(0x4548_4552, [4, 5, 6, 0, 9]),
                        &chunk(b"ENIT", &enit),
                    ]),
                ),
                // No effects: the warning names the +0x1C name.
                &record(b"ENCH", 0x901, &chunk(b"FULL", &text("Empty"))),
                // A code no setting has.
                &record(
                    b"ENCH",
                    0x902,
                    &cat(&[
                        &chunk(b"EFID", &u32_bytes(0x5A5A_5A5A)),
                        &chunk(b"ENIT", &enit),
                    ]),
                ),
            ]),
        ),
        &group(
            b"SPEL",
            &cat(&[
                &record(
                    b"SPEL",
                    0x910,
                    &cat(&[
                        &seff,
                        &scit,
                        &chunk(b"FULL", &text("Script name")),
                        &chunk(b"SPIT", &spit),
                    ]),
                ),
                // A chunk other than FULL after SCIT stays current, and the advance passes it: this SPIT is never read.
                &record(
                    b"SPEL",
                    0x911,
                    &cat(&[&seff, &scit, &chunk(b"SPIT", &spit)]),
                ),
                // No SCIT: the item is dropped with the warning, by the EFID's name.
                &record(b"SPEL", 0x912, &cat(&[&seff, &chunk(b"SPIT", &spit)])),
            ]),
        ),
    ]);
    let loader = fixture.run_records(&records, |_| {});
    let world = &loader.world;
    let ench = form_at(&loader, 0x900);
    let list = world.get::<EffectItemList>(ench).unwrap();
    assert_eq!(list.items_04.len(), 1);
    // 004131B7: REHE's flags have 0x1180000 and bit 24, so +0x14 is the setting's +0x60, 8, not the record's 9.
    assert_eq!(list.items_04[0].data_00, [0x4548_4552, 4, 5, 6, 0, 8]);
    assert_eq!(list.items_04[0].cost_20, -1.0);
    assert!(list.items_04[0].script_18.is_none());
    let data = world.get::<MagicFormData>(ench).unwrap();
    assert_eq!(data.bytes().to_vec(), enit);
    assert!(
        warned(
            &loader,
            "[disabled] Magic Item (00000901) Empty has no effects defined."
        ),
        "{:?}",
        loader.warnings
    );
    assert!(warned(
        &loader,
        &format!(
            "[disabled] Unable to find EffectSetting {} in spell '' (00000902).",
            0x5A5A_5A5A
        )
    ));
    assert!(warned(
        &loader,
        "[disabled] Magic Item (00000902)  has no effects defined."
    ));

    let spell = form_at(&loader, 0x910);
    let list = world.get::<EffectItemList>(spell).unwrap();
    let script = list.items_04[0].script_18.as_ref().unwrap();
    assert_eq!(script.school_04, 5);
    // 0046BB20 in the master itself: top byte 0 is the file's own index, 0, and IDs under 0x800 stay.
    assert_eq!(script.script_00, 0x0123);
    assert_eq!(script.visual_10, 0x4548_4552);
    assert_eq!(script.hostile_14, 1, "the fourth dword's low byte");
    assert_eq!(script.name_08.data_00.as_deref(), Some(&b"Script name"[..]));
    assert_eq!(list.hostile_count_0c, 1, "00413470 by +0x14");
    assert_eq!(
        world.get::<MagicFormData>(spell).unwrap().bytes().to_vec(),
        spit
    );

    let passed = form_at(&loader, 0x911);
    assert_eq!(
        world.get::<MagicFormData>(passed).unwrap().value_38,
        0xFFFF_FFFF,
        "the SPIT after SCIT was passed over: 0041D1B0's default"
    );
    let dropped = form_at(&loader, 0x912);
    assert!(
        world
            .get::<EffectItemList>(dropped)
            .unwrap()
            .items_04
            .is_empty()
    );
    let name = {
        let form = world
            .resource::<EffectSettingCollection>()
            .lookup(SCRIPT_EFFECT_CODE)
            .unwrap();
        String::from_utf8(
            world
                .get::<TesFullName>(form)
                .unwrap()
                .name_04
                .data_00
                .clone()
                .unwrap(),
        )
        .unwrap()
    };
    assert!(warned(
        &loader,
        &format!("[disabled] Unable to load EffectItem '{name}' in spell '' (00000912).")
    ));
}

/// 00418F10 and 0041D1B0 on a reload: the items go, the hostile count stays (00414C70 doesn't touch +0x0C).
#[test]
fn a_reload_frees_the_items_and_keeps_the_hostile_count() {
    let fixture = Fixture::new("reload");
    let seff = efid(SCRIPT_EFFECT_CODE, [1, 2, 3, 0, 7]);
    let scit = chunk(
        b"SCIT",
        &cat(&[&u32_bytes(0), &u32_bytes(5), &u32_bytes(0), &u32_bytes(1)]),
    );
    let mut esm = tes4(true, &[]);
    esm.extend(group(
        b"SPEL",
        &record(
            b"SPEL",
            0x910,
            &cat(&[&seff, &scit, &chunk(b"FULL", &text("x"))]),
        ),
    ));
    fixture.write("Base.esm", &esm);
    let mut esp = tes4(false, &["Base.esm"]);
    esp.extend(group(
        b"SPEL",
        &record(b"SPEL", 0x910, &chunk(b"FULL", &text("Reloaded"))),
    ));
    fixture.write("Plugin.esp", &esp);
    let mut loader = Loader::new(&fixture.root);
    loader.load_all();
    assert_eq!(loader.run(), Ok(1));
    let world = &loader.world;
    let form = form_at(&loader, 0x910);
    let list = world.get::<EffectItemList>(form).unwrap();
    assert!(list.items_04.is_empty());
    assert_eq!(list.hostile_count_0c, 1);
    assert_eq!(
        world
            .get::<TesFullName>(form)
            .unwrap()
            .name_04
            .data_00
            .as_deref(),
        Some(&b"Reloaded"[..])
    );
}

/// 005193E0 with 0046F350: a prepend, no repeat, no zero. BirthSign has no record test.
#[test]
fn a_birth_sign_prepends_its_spells() {
    let fixture = Fixture::new("bsgn");
    let spells = cat(&[
        &chunk(b"SPLO", &u32_bytes(0x111)),
        &chunk(b"SPLO", &u32_bytes(0x222)),
        &chunk(b"SPLO", &u32_bytes(0x111)),
        &chunk(b"SPLO", &u32_bytes(0)),
    ]);
    let loader = fixture.run_records(
        &group(
            b"BSGN",
            &record(
                b"BSGN",
                0x900,
                &cat(&[
                    &chunk(b"FULL", &text("Sign")),
                    &chunk(b"ICON", &text("a.dds")),
                    &spells,
                ]),
            ),
        ),
        |_| {},
    );
    let world = &loader.world;
    let form = form_at(&loader, 0x900);
    assert_eq!(
        raw_ids(&world.get::<TesSpellList>(form).unwrap().spells_04),
        [0x222, 0x111]
    );
    assert_eq!(
        world.get::<FormIdentity>(form).unwrap().flags_08 & 8,
        0,
        "0046AB80(0) at the end"
    );
    assert_eq!(
        world
            .get::<TesTexture>(form)
            .unwrap()
            .path_04
            .data_00
            .as_deref(),
        Some(&b"a.dds"[..])
    );
    assert_eq!(
        world
            .resource::<DataHandlerLists>()
            .birth_signs_8c
            .items()
            .next(),
        Some(form)
    );
}

/// 004B04A0: LVLD's bit 7 and low seven bits, LVLF's byte, and LVLO's word at +0x08 defaulting to 1 under a short chunk.
#[test]
fn a_leveled_spell_list() {
    let fixture = Fixture::new("lvsp");
    let entry = |level: u16, form: u32, count: u16| {
        chunk(
            b"LVLO",
            &cat(&[
                &u16_bytes(level),
                &[0xAA, 0xAA],
                &u32_bytes(form),
                &u16_bytes(count),
                &[0, 0],
            ]),
        )
    };
    let loader = fixture.run_records(
        &group(
            b"LVSP",
            &cat(&[
                &record(
                    b"LVSP",
                    0x900,
                    &cat(&[
                        &chunk(b"LVLD", &[0x85]),
                        &entry(5, 0x111, 2),
                        &entry(1, 0x222, 3),
                        // 8 bytes: the zeroed locals' +0x08 stays 1.
                        &chunk(b"LVLO", &cat(&[&u16_bytes(9), &[0, 0], &u32_bytes(0x333)])),
                    ]),
                ),
                &record(
                    b"LVSP",
                    0x901,
                    &cat(&[&chunk(b"LVLF", &[0x06]), &chunk(b"LVLD", &[0x05])]),
                ),
            ]),
        ),
        |_| {},
    );
    let world = &loader.world;
    let list = world
        .get::<TesLeveledList>(form_at(&loader, 0x900))
        .unwrap();
    assert_eq!(list.chance_none_0c, 5);
    assert_eq!(list.flags_0d, 1);
    let order: Vec<(u16, i16)> = list
        .entries_04
        .iter()
        .map(|e| (e.level_00, e.count_08))
        .collect();
    assert_eq!(order, [(1, 3), (5, 2), (9, 1)]);
    let list = world
        .get::<TesLeveledList>(form_at(&loader, 0x901))
        .unwrap();
    assert_eq!(list.flags_0d, 6, "bit 0 cleared by 0046CB10(0)");
    assert_eq!(list.chance_none_0c, 5);
    assert_eq!(
        world
            .get::<FormIdentity>(form_at(&loader, 0x900))
            .unwrap()
            .flags_08
            & 8,
        0
    );
}

/// 0052B840 and 0052D100, by the stores in the disassembly.
#[test]
fn a_race_constructor_stores_its_defaults() {
    let fixture = Fixture::new("race_new");
    let mut loader = fixture.run_records(&[], |_| {});
    let class = form_class(0x09).unwrap();
    let form = (class.construct)(&mut loader.world).unwrap();
    let world = &loader.world;
    let race = world.get::<TesRace>(form).unwrap();
    let mut data = [0_u8; 0x24];
    for pair in 0..7 {
        data[2 * pair] = 0xFF;
    }
    data[0x10..0x14].copy_from_slice(&1.0_f32.to_le_bytes());
    data[0x14..0x18].copy_from_slice(&1.0_f32.to_le_bytes());
    data[0x18..0x1C].copy_from_slice(&1.0_f32.to_le_bytes());
    data[0x1C..0x20].copy_from_slice(&1.0_f32.to_le_bytes());
    assert_eq!(race.data_50, data);
    assert_eq!(
        race.face_models_e0[7].path_04.data_00.as_deref(),
        Some(&b"Characters\\Imperial\\EyeLeftHuman.nif"[..])
    );
    assert_eq!(
        race.face_textures_1b8[6].path_04.data_00.as_deref(),
        Some(&b"Characters\\Imperial\\TongueHuman.dds"[..])
    );
    assert_eq!(race.face_textures_1b8[7].path_04.data_00, None);
    assert_eq!(
        race.body_textures_224[3].path_04.data_00.as_deref(),
        Some(&b"Characters\\Imperial\\Male\\HandMale.dds"[..])
    );
    assert_eq!(race.body_textures_224[4].path_04.data_00, None);
    assert_eq!(
        race.body_textures_224[8].path_04.data_00.as_deref(),
        Some(&b"Characters\\Imperial\\Female\\HandFemale.dds"[..])
    );
    assert_eq!(
        race.attributes_74,
        TesAttributes::new(TES_ATTRIBUTES_VTABLE)
    );
    let rows: Vec<(u32, u32)> = race.facegen_29c.iter().map(|m| (m.rows, m.cols)).collect();
    assert_eq!(rows, [(50, 1), (30, 1), (50, 1), (0, 0)]);
    assert_eq!(race.undo_308.grow_0e, 1);
    assert_eq!(world.get::<TesReactionForm>(form).unwrap().group_type_0c, 6);
    assert_eq!(world.get::<FormIdentity>(form).unwrap().form_type_04, 9);
}

/// 0052D800's section state: MNAM and FNAM pick the sex, NAM0 and NAM1 the face section; INDX resets a face part; ICON and MODL go by those; VNAM by its size; HNAM refuses a size that isn't a multiple of 4, keeps one of each ID, and warns for a form it can't find.
#[test]
fn the_race_chunks() {
    let fixture = Fixture::new("race");
    let payload = cat(&[
        &chunk(b"FNAM", &[]),
        &chunk(b"INDX", &u32_bytes(2)),
        &chunk(b"ICON", &text("female2.dds")),
        &chunk(b"MODL", &text("female.nif")),
        &chunk(b"MNAM", &[]),
        &chunk(b"MODL", &text("male.nif")),
        &chunk(b"NAM0", &[]),
        &chunk(b"INDX", &u32_bytes(1)),
        &chunk(b"MODL", &text("face1.nif")),
        &chunk(b"ICON", &text("face1.dds")),
        &chunk(b"INDX", &u32_bytes(0)),
        &chunk(b"NAM1", &[]),
        &chunk(b"INDX", &u32_bytes(3)),
        &chunk(b"ICON", &text("male3.dds")),
        &chunk(b"VNAM", &cat(&[&u32_bytes(0x71), &u32_bytes(0x72)])),
        &chunk(b"DNAM", &cat(&[&u32_bytes(0x81), &u32_bytes(0x82)])),
        &chunk(b"CNAM", &[4]),
        &chunk(b"PNAM", &f32_bytes(0.5)),
        &chunk(b"UNAM", &f32_bytes(0.25)),
        &chunk(b"SNAM", &[0x34]),
        &chunk(b"XNAM", &cat(&[&u32_bytes(0x91), &u32_bytes(0xFFFF_FFFE)])),
        &chunk(
            b"ATTR",
            &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16],
        ),
        &chunk(b"HNAM", &[0, 0, 0]),
        &chunk(
            b"HNAM",
            &cat(&[
                &u32_bytes(0x700),
                &u32_bytes(0x700),
                &u32_bytes(0x701),
                &u32_bytes(0x702),
            ]),
        ),
        &chunk(b"ENAM", &u32_bytes(0x800)),
        &chunk(b"FGGA", &cat(&[&f32_bytes(1.5), &f32_bytes(2.5)])),
        &chunk(b"SPLO", &u32_bytes(0x111)),
    ]);
    let vnam_4 = cat(&[
        &chunk(b"VNAM", &cat(&[&u32_bytes(0x71), &u32_bytes(0x72)])),
        &chunk(b"VNAM", &u32_bytes(5)),
        &chunk(b"SNAM", &[0x34, 0x12]),
    ]);
    let loader = fixture.run_records(
        &group(
            b"RACE",
            &cat(&[
                &record(b"RACE", 0x900, &payload),
                &record(b"RACE", 0x901, &vnam_4),
            ]),
        ),
        |world| {
            stand_in(world, 0x07, 0x700);
            stand_in(world, 0x07, 0x701);
            stand_in(world, 0x08, 0x702);
            stand_in(world, 0x08, 0x800);
        },
    );
    let world = &loader.world;
    let form = form_at(&loader, 0x900);
    let race = world.get::<TesRace>(form).unwrap();
    let path = |model: &TesModel| model.path_04.data_00.clone();
    assert_eq!(
        race.body_textures_224[7].path_04.data_00.as_deref(),
        Some(&b"female2.dds"[..])
    );
    assert_eq!(
        path(&race.body_models_b0[1]).as_deref(),
        Some(&b"female.nif"[..])
    );
    assert_eq!(
        path(&race.body_models_b0[0]).as_deref(),
        Some(&b"male.nif"[..])
    );
    assert_eq!(
        path(&race.face_models_e0[1]).as_deref(),
        Some(&b"face1.nif"[..])
    );
    assert_eq!(
        race.face_textures_1b8[1].path_04.data_00.as_deref(),
        Some(&b"face1.dds"[..])
    );
    // INDX 0 in the face section ran the two vt+0s.
    assert_eq!(path(&race.face_models_e0[0]), None);
    assert_eq!(race.face_textures_1b8[0].path_04.data_00, None);
    // NAM1 closed the section, and the sex is male again: 5 * 0 + 3.
    assert_eq!(
        race.body_textures_224[3].path_04.data_00.as_deref(),
        Some(&b"male3.dds"[..])
    );
    assert_eq!(race.voices_300, [FormRef::Raw(0x71), FormRef::Raw(0x72)]);
    assert_eq!(
        race.default_hairs_94,
        [FormRef::Raw(0x81), FormRef::Raw(0x82)]
    );
    assert_eq!(race.hair_color_9c, 4);
    assert_eq!(race.facegen_main_a0, 0.5);
    assert_eq!(race.facegen_face_a4, 0.25);
    assert_eq!(race.snam_2fc, 0, "under 2 bytes does nothing");
    assert_eq!(race.attributes_74.values_04, [1, 2, 3, 4, 5, 6, 7, 8]);
    assert_eq!(
        race.attributes_80.values_04,
        [9, 10, 11, 12, 13, 14, 15, 16]
    );
    assert_eq!(race.hairs_8c.len(), 2, "one of each ID");
    assert_eq!(race.eyes_a8.len(), 1);
    assert_eq!(race.facegen_29c[1].rows, 2);
    assert_eq!(race.facegen_29c[1].data, [1.5, 2.5]);
    assert!(
        warned(
            &loader,
            "[disabled] Could not find hair (00000702) for race '' (00000900)."
        ),
        "{:?}",
        loader.warnings
    );
    let reaction = world.get::<TesReactionForm>(form).unwrap();
    assert_eq!(reaction.reactions_04[0].modifier, -2);
    let other = world.get::<TesRace>(form_at(&loader, 0x901)).unwrap();
    assert_eq!(other.voices_300, [FormRef::Raw(0); 2], "size 4 clears both");
    assert_eq!(other.snam_2fc, 0x1234);
}

/// 00527220 and 005255A0, by the stores in the disassembly.
#[test]
fn an_npc_constructor_stores_its_defaults() {
    let fixture = Fixture::new("npc_new");
    let mut loader = fixture.run_records(&[], |_| {});
    let class = form_class(0x23).unwrap();
    let form = (class.construct)(&mut loader.world).unwrap();
    let world = &loader.world;
    let npc = world.get::<TesNpc>(form).unwrap();
    assert_eq!(npc.skills_ec, [5; 21]);
    assert_eq!(npc.fnam_1e0, 0xFF);
    assert_eq!(npc.hair_color_1e8, 0x0019_324B);
    let rows: Vec<u32> = npc.facegen_108.iter().map(|matrix| matrix.rows).collect();
    assert_eq!(rows, [50, 30, 50, 0]);
    assert!(npc.facegen_168.iter().all(|matrix| matrix.rows == 0));
    assert_eq!(npc.undo_1f0.grow_0e, 1);
    assert_eq!(world.get::<TesHealthForm>(form).unwrap().health_04, 0x32);
    assert_eq!(
        world.get::<TesAiForm>(form).unwrap().ai_04,
        [5, 0x32, 0x32, 0x32]
    );
    let base = world.get::<TesActorBaseData>(form).unwrap();
    assert_eq!(
        (base.magicka_08, base.fatigue_0a, base.level_0e),
        (0x32, 0x32, 1)
    );
    assert_eq!(
        world.get::<TesAttributes>(form).unwrap().values_04,
        [0x32; 8]
    );
    let values = &world.get::<ActorBaseValues>(form).unwrap().0;
    assert_eq!(values.magicka_08.unwrap().code_00, 9);
    assert_eq!(values.fatigue_0c.unwrap().code_00, 10);
    assert_eq!(world.get::<FormIdentity>(form).unwrap().form_type_04, 0x23);
    assert!(class.slots.contains(&ComponentSlot::RaceForm));
}

/// 00527E40: LNAM's clamps through `fcom`, DATA's 21 and 23-byte layouts, the unrecognized size, the one face warning, FaceGen's copy to +0x168, a short ACBS over the fields, SCRI's resolve, a null PKID, and form ID 7 as the player's base.
#[test]
fn the_npc_chunks() {
    let fixture = Fixture::new("npc");
    let skills: Vec<u8> = (1..=21).collect();
    let health = u32_bytes(77);
    let attributes = [8, 7, 6, 5, 4, 3, 2, 1];
    let npc = |id: u32, chunks: &[Vec<u8>]| record(b"NPC_", id, &chunks.concat());
    let lnam = |value: u32| vec![chunk(b"LNAM", &u32_bytes(value))];
    let mut wide = vec![0xEE];
    wide.extend_from_slice(&skills);
    wide.push(0xDD);
    let records = group(
        b"NPC_",
        &cat(&[
            &npc(
                0x900,
                &[
                    chunk(b"DATA", &cat(&[&skills, &health, &attributes])),
                    chunk(b"NAM0", &[]),
                    chunk(b"NAM1", &[]),
                    chunk(b"FGGS", &cat(&[&f32_bytes(1.0), &f32_bytes(2.0)])),
                    chunk(b"RNAM", &u32_bytes(0x19)),
                    chunk(b"CNAM", &u32_bytes(0x20)),
                    chunk(b"INAM", &u32_bytes(0x21)),
                    chunk(b"ZNAM", &u32_bytes(0x22)),
                    chunk(b"HCLR", &u32_bytes(0x0012_3456)),
                    chunk(b"FNAM", &[0x34, 0x12]),
                    chunk(b"ACBS", &[0x11, 0, 0, 0, 0x22, 0]),
                    chunk(b"PKID", &u32_bytes(0)),
                    chunk(b"PKID", &u32_bytes(0x31)),
                    chunk(b"PKID", &u32_bytes(0x32)),
                    chunk(b"CNTO", &cat(&[&u32_bytes(0x41), &u32_bytes(3)])),
                    chunk(b"SNAM", &cat(&[&u32_bytes(0x51), &[2, 0, 0, 0]])),
                    chunk(b"SNAM", &cat(&[&u32_bytes(0x52), &[3, 0, 0, 0]])),
                    chunk(b"SNAM", &cat(&[&u32_bytes(0x51), &[0xFF, 0, 0, 0]])),
                    chunk(b"KFFZ", &[b'a', 0, b'B', 0, b'A', 0, 0]),
                    chunk(b"SCRI", &u32_bytes(0x500)),
                    chunk(b"HNAM", &u32_bytes(0x700)),
                    chunk(b"ENAM", &u32_bytes(0x700)),
                ],
            ),
            &npc(
                0x901,
                &[
                    chunk(b"DATA", &cat(&[&wide, &health, &attributes])),
                    chunk(b"SCRI", &u32_bytes(0x501)),
                ],
            ),
            &npc(0x902, &[chunk(b"DATA", &[0; 4])]),
            &npc(0x903, &lnam(0xBF80_0000)),
            &npc(0x904, &lnam(0x4000_0000)),
            &npc(0x905, &lnam(0x3F00_0000)),
            &npc(0x906, &lnam(0x7FC0_0000)),
            &npc(0x907, &lnam(0x8000_0000)),
            &npc(0x908, &lnam(0x7F80_0001)),
            &npc(7, &[chunk(b"FULL", &text("Player"))]),
        ]),
    );
    let mut esm = tes4(true, &[]);
    esm.extend_from_slice(&records);
    fixture.write("Base.esm", &esm);
    let mut loader = Loader::new(&fixture.root);
    loader.load_all();
    loader.host.pre_load = |world| {
        stand_in(world, 0x07, 0x700);
        stand_in(world, 0x0D, 0x500);
        let script = world.resource::<FormsById>().lookup(0x500).unwrap();
        world.entity_mut(script).insert(Script::default());
    };
    let player = loader.world.spawn(TesObjectRefr::default()).id();
    loader.world.insert_resource(PlayerGlobals {
        the_player_b333c4: Some(player),
        ..PlayerGlobals::default()
    });
    assert_eq!(loader.run(), Ok(1));
    let world = &loader.world;
    let form = form_at(&loader, 0x900);
    let data = world.get::<TesNpc>(form).unwrap();
    assert_eq!(data.skills_ec.to_vec(), skills);
    assert_eq!(world.get::<TesHealthForm>(form).unwrap().health_04, 77);
    assert_eq!(
        world.get::<TesAttributes>(form).unwrap().values_04,
        attributes
    );
    assert_eq!(data.facegen_108[0].rows, 2);
    assert_eq!(data.facegen_108[0].data, [1.0, 2.0]);
    assert_eq!(data.facegen_168[0].data, [1.0, 2.0], "005528F0 to +0x168");
    assert_eq!(data.facegen_168[1].rows, 30);
    assert_eq!(data.class_104, FormRef::Raw(0x20));
    assert_eq!(data.combat_style_1e4, FormRef::Raw(0x22));
    assert_eq!(data.hair_color_1e8, 0x0012_3456);
    assert_eq!(data.fnam_1e0, 0x1234);
    assert!(data.hair_1c8.is_some());
    assert_eq!(data.eyes_1d0, None, "0x700 isn't a TESEyes");
    assert!(warned(
        &loader,
        "[disabled] Could not find eyes (00000700) on NPC '' (00000900)."
    ));
    assert_eq!(
        loader
            .warnings
            .iter()
            .filter(|line| line.contains("Found face texture"))
            .count(),
        1
    );
    let base = world.get::<TesActorBaseData>(form).unwrap();
    assert_eq!(base.flags_04, 0x11);
    assert_eq!(base.magicka_08, 0x22);
    assert_eq!(base.fatigue_0a, 0x32, "a short ACBS leaves the rest");
    assert_eq!(base.level_0e, 1);
    assert_eq!(base.death_item_14, FormRef::Raw(0x21));
    assert_eq!(
        base.factions_18.len(),
        1,
        "rank 0xFF took 0x51 off the list"
    );
    assert_eq!(base.factions_18[0].faction_00, FormRef::Raw(0x52));
    assert_eq!(
        raw_ids(&world.get::<TesAiForm>(form).unwrap().packages_10),
        [0x31, 0x32],
        "0067B1E0 ignores a null, and appends"
    );
    let container = world.get::<TesContainer>(form).unwrap();
    assert_eq!(container.entries_08[0].count, 3);
    assert_eq!(
        world.get::<TesAnimation>(form).unwrap().names_04,
        [b"a".to_vec(), b"B".to_vec()]
    );
    let script = world.get::<TesScriptableForm>(form).unwrap();
    assert_eq!(script.resolved_08, 1);
    assert!(matches!(script.script_04, FormRef::Resolved(Some(_))));
    assert_eq!(world.get::<FormIdentity>(form).unwrap().flags_08 & 8, 0);

    let wide_form = form_at(&loader, 0x901);
    assert_eq!(
        world.get::<TesNpc>(wide_form).unwrap().skills_ec.to_vec(),
        skills
    );
    let unresolved = world.get::<TesScriptableForm>(wide_form).unwrap();
    assert_eq!(unresolved.script_04, FormRef::Resolved(None));
    assert_eq!(unresolved.resolved_08, 1);
    assert!(warned(
        &loader,
        "[disabled] Unable to find script (00000501) on owner object (00000901)."
    ));
    assert!(warned(
        &loader,
        "[disabled] Unrecognized data format for NPC '' (00000902) in file 'Base.esm'."
    ));

    let hair_bits = |id: u32| {
        world
            .get::<TesNpc>(form_at(&loader, id))
            .unwrap()
            .hair_length_1cc
            .to_bits()
    };
    assert_eq!(hair_bits(0x903), 0, "-1.0 takes 0.0");
    assert_eq!(hair_bits(0x904), 0x3F80_0000, "2.0 takes 1.0");
    assert_eq!(hair_bits(0x905), 0x3F00_0000);
    assert_eq!(
        hair_bits(0x906),
        0x7FC0_0000,
        "a NaN stays, where the C++ takes 0.0"
    );
    assert_eq!(hair_bits(0x907), 0x8000_0000, "-0.0 isn't below 0.0");
    assert_eq!(hair_bits(0x908), 0x7FC0_0001, "fld quiets an sNaN");
    assert_eq!(
        world.get::<TesObjectRefr>(player).unwrap().base_form_1c,
        Some(form_at(&loader, 7)),
        "005284DA"
    );
}

/// 0051EB80 and 0051C7B0, by the stores in the disassembly.
#[test]
fn a_creature_constructor_stores_its_defaults() {
    let fixture = Fixture::new("crea_new");
    let mut loader = fixture.run_records(&[], |_| {});
    let class = form_class(0x24).unwrap();
    assert!(
        (class.construct)(&mut loader.world).is_err(),
        "0051ECDA: outside a load, the faction is a host stop"
    );
    loader
        .world
        .resource_mut::<recomp_game::tes::tes::GameThreadLocals>()
        .byte_184 = 1;
    let form = (class.construct)(&mut loader.world).unwrap();
    let world = &loader.world;
    let creature = world.get::<TesCreature>(form).unwrap();
    assert_eq!(creature.data_104, [0, 0x32, 0x32, 0x32]);
    assert_eq!(creature.soul_108, 3);
    assert_eq!(creature.reach_10a, 0x20);
    assert_eq!(creature.foot_weight_110, 3.0);
    assert_eq!(creature.scale_114, 1.0);
    assert_eq!(creature.turning_10c, 0.0);
    assert_eq!(world.get::<TesHealthForm>(form).unwrap().health_04, 0x32);
    assert_eq!(
        world.get::<TesAiForm>(form).unwrap().ai_04,
        [0x46, 0x32, 0x32, 0x32]
    );
    assert_eq!(
        world.get::<TesActorBaseData>(form).unwrap().flags_04,
        0x4240,
        "0x40, 0x200, 0x4000"
    );
    assert_eq!(
        world.get::<TesAttributes>(form).unwrap().values_04,
        [0x32; 8]
    );
    assert_eq!(world.get::<TesAttackDamageForm>(form).unwrap().damage_04, 0);
}

/// 0051DD00: the sound chunks in 00519880's chance order; a CSDI with no CSDC after it, whose next chunk is looked at again with no advance; a type past 9; CSCR both ways; the data chunks; and NIFZ with NIFT.
#[test]
fn the_creature_chunks() {
    let fixture = Fixture::new("crea");
    let sound =
        |id: u32, chance: u8| cat(&[&chunk(b"CSDI", &u32_bytes(id)), &chunk(b"CSDC", &[chance])]);
    let nift_entry = [0_u8; 0x18];
    let records = group(
        b"CREA",
        &cat(&[
            &record(
                b"CREA",
                0x900,
                &cat(&[
                    &chunk(b"CSDT", &u32_bytes(2)),
                    &sound(0x600, 50),
                    &sound(0x601, 10),
                    &sound(0x602, 50),
                    &sound(0x603, 30),
                    &chunk(b"CSDT", &u32_bytes(10)),
                    &sound(0x604, 1),
                    &chunk(b"CSDT", &u32_bytes(3)),
                    // No CSDC: the block is freed, and RNAM is examined with no advance.
                    &chunk(b"CSDI", &u32_bytes(0x605)),
                    &chunk(b"RNAM", &[0x30]),
                    &chunk(
                        b"DATA",
                        &cat(&[
                            &[9, 8, 7, 6],
                            &u16_bytes(4),
                            &u32_bytes(99),
                            &u16_bytes(5),
                            &[1, 2, 3, 4, 5, 6, 7, 8],
                        ]),
                    ),
                    &chunk(b"TNAM", &f32_bytes(2.0)),
                    &chunk(b"WNAM", &f32_bytes(4.0)),
                    &chunk(b"BNAM", &f32_bytes(8.0)),
                    &chunk(b"ZNAM", &u32_bytes(0x61)),
                    &chunk(b"INAM", &u32_bytes(0x62)),
                    &chunk(b"NAM0", &text("blood.nif")),
                    &chunk(b"NAM1", &text("decal.dds")),
                    &chunk(b"NIFZ", b"One.nif\0two.NIF\0ONE.NIF\0\0"),
                    &chunk(b"NIFT", &cat(&[&u32_bytes(2), &[1], &nift_entry, &[0]])),
                ]),
            ),
            &record(b"CREA", 0x901, &chunk(b"CSCR", &u32_bytes(0x70))),
            &record(
                b"CREA",
                0x902,
                &cat(&[
                    &sound(0x600, 5),
                    &chunk(b"CSCR", &u32_bytes(0)),
                    &chunk(b"FULL", &text("Kept")),
                ]),
            ),
            &record(
                b"CREA",
                0x903,
                &cat(&[&sound(0x600, 5), &chunk(b"CSCR", &u32_bytes(0x71))]),
            ),
        ]),
    );
    let loader = fixture.run_records(&records, |world| {
        for id in 0x600..=0x604 {
            stand_in(world, 0x0A, id);
        }
    });
    let world = &loader.world;
    let form = form_at(&loader, 0x900);
    let creature = world.get::<TesCreature>(form).unwrap();
    let CreatureSoundsSlot::Sounds(sounds) = &creature.sounds_100 else {
        panic!("the block");
    };
    let chances: Vec<u8> = sounds.lists[2]
        .as_ref()
        .unwrap()
        .iter()
        .map(|e| e.chance_04)
        .collect();
    assert_eq!(chances, [10, 30, 50, 50]);
    let ids: Vec<u32> = sounds.lists[2]
        .as_ref()
        .unwrap()
        .iter()
        .map(|entry| {
            world
                .get::<FormIdentity>(entry.sound_00.unwrap())
                .unwrap()
                .form_id_0c
        })
        .collect();
    assert_eq!(ids, [0x601, 0x603, 0x600, 0x602], "after equal chances");
    assert!(
        sounds
            .lists
            .iter()
            .enumerate()
            .all(|(index, list)| index == 2 || list.is_none()),
        "type 10 is dropped, and 3's CSDI had no CSDC"
    );
    assert_eq!(creature.reach_10a, 0x30);
    assert_eq!(creature.data_104, [9, 8, 7, 6]);
    assert_eq!(creature.soul_108, 4);
    assert_eq!(world.get::<TesHealthForm>(form).unwrap().health_04, 99);
    assert_eq!(world.get::<TesAttackDamageForm>(form).unwrap().damage_04, 5);
    assert_eq!(
        world.get::<TesAttributes>(form).unwrap().values_04,
        [1, 2, 3, 4, 5, 6, 7, 8]
    );
    assert_eq!(creature.turning_10c, 2.0);
    assert_eq!(creature.foot_weight_110, 4.0);
    assert_eq!(creature.scale_114, 8.0);
    assert_eq!(creature.combat_style_118, FormRef::Raw(0x61));
    assert_eq!(
        creature.blood_11c.path_04.data_00.as_deref(),
        Some(&b"blood.nif"[..])
    );
    assert_eq!(
        creature.decal_134.path_04.data_00.as_deref(),
        Some(&b"decal.dds"[..])
    );
    assert_eq!(
        world.get::<TesActorBaseData>(form).unwrap().flags_04 & 0x100,
        0x100,
        "0051CDE0"
    );
    let models = world.get::<TesModelList>(form).unwrap();
    assert_eq!(
        models.names_04,
        [b"One.nif".to_vec(), b"two.NIF".to_vec()],
        "_stricmp"
    );
    assert_eq!(models.count_0c, 2);
    let lists = models.textures_10.as_ref().unwrap();
    assert_eq!(lists.len(), 2);
    assert_eq!((lists[0].count_00, lists[0].entries_04.len()), (1, 1));
    assert_eq!((lists[1].count_00, lists[1].entries_04.len()), (0, 0));

    let with_id = world.get::<TesCreature>(form_at(&loader, 0x901)).unwrap();
    assert_eq!(
        with_id.sounds_100,
        CreatureSoundsSlot::Reference(FormRef::Raw(0x70))
    );
    let kept = world.get::<TesCreature>(form_at(&loader, 0x902)).unwrap();
    assert!(
        matches!(kept.sounds_100, CreatureSoundsSlot::Sounds(_)),
        "a zero CSCR with the bit set does nothing"
    );
    let replaced = form_at(&loader, 0x903);
    assert_eq!(
        world.get::<TesCreature>(replaced).unwrap().sounds_100,
        CreatureSoundsSlot::Reference(FormRef::Raw(0x71))
    );
    assert_eq!(
        world.get::<TesActorBaseData>(replaced).unwrap().flags_04 & 0x100,
        0
    );
}

/// 00519A31 and 00517ED0: CSDF names the sound by the +0x6C list's +0x28 with _stricmp, where the C++ stops. Only a sound in that list is found, so with no list a null goes in, with no warning.
#[test]
fn a_csdf_with_no_matching_sound_gives_a_null_entry() {
    let fixture = Fixture::new("csdf");
    let loader = fixture.run_records(
        &group(
            b"CREA",
            &record(
                b"CREA",
                0x900,
                &cat(&[
                    &chunk(b"CSDT", &u32_bytes(1)),
                    &chunk(b"CSDF", &text("nothing.wav")),
                    &chunk(b"CSDC", &[7]),
                ]),
            ),
        ),
        |_| {},
    );
    let world = &loader.world;
    let creature = world.get::<TesCreature>(form_at(&loader, 0x900)).unwrap();
    let CreatureSoundsSlot::Sounds(sounds) = &creature.sounds_100 else {
        panic!("the block");
    };
    let entry = sounds.lists[1].as_ref().unwrap()[0];
    assert_eq!((entry.sound_00, entry.chance_04), (None, 7));
    assert!(
        loader
            .warnings
            .iter()
            .all(|line| !line.contains("CreatureSound"))
    );
}

/// 0044DE05: a reload of an NPC_ in a second file runs vt+0x18 and vt+0x14: the lists go, 0x200 clears, the health returns.
#[test]
fn a_reload_of_an_npc_resets_it() {
    let fixture = Fixture::new("npc_reload");
    let mut esm = tes4(true, &[]);
    esm.extend(group(
        b"NPC_",
        &record(
            b"NPC_",
            0x900,
            &cat(&[
                &chunk(b"PKID", &u32_bytes(0x31)),
                &chunk(b"SPLO", &u32_bytes(0x32)),
                &chunk(b"ACBS", &[0, 2, 0, 0, 1, 0, 2, 0, 3, 0, 4, 0, 5, 0, 6, 0]),
                &chunk(b"KFFZ", b"a\0\0"),
                &chunk(b"FGGS", &f32_bytes(3.0)),
            ]),
        ),
    ));
    fixture.write("Base.esm", &esm);
    let mut esp = tes4(false, &["Base.esm"]);
    esp.extend(group(
        b"NPC_",
        &record(b"NPC_", 0x900, &chunk(b"FULL", &text("Again"))),
    ));
    fixture.write("Plugin.esp", &esp);
    let mut loader = Loader::new(&fixture.root);
    loader.load_all();
    assert_eq!(loader.run(), Ok(1));
    let world = &loader.world;
    let form = form_at(&loader, 0x900);
    assert!(world.get::<TesAiForm>(form).unwrap().packages_10.is_empty());
    assert!(
        world
            .get::<TesSpellList>(form)
            .unwrap()
            .spells_04
            .is_empty()
    );
    assert!(world.get::<TesAnimation>(form).unwrap().names_04.is_empty());
    let base = world.get::<TesActorBaseData>(form).unwrap();
    assert_eq!(base.flags_04, 0);
    assert_eq!(base.level_0e, 1);
    assert_eq!(world.get::<TesHealthForm>(form).unwrap().health_04, 0x32);
    let npc = world.get::<TesNpc>(form).unwrap();
    assert_eq!(npc.facegen_108[0].rows, 50, "005528F0 from g_faceGen");
    assert!(npc.facegen_108[0].data.iter().all(|value| *value == 0.0));
    assert_eq!(npc.facegen_168[0].rows, 1, "+0x168 keeps its shape");
    assert_eq!(npc.facegen_168[0].data, [0.0], "and its floats zero");
    assert_eq!(
        world.get::<ModReferences>(form).unwrap().0,
        [0, 1],
        "the master and the plugin"
    );
}
