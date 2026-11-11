//! IDA: N/A P14-RUN-004's `ACT` rows against a host actor, for `phase14_ai` and `phase14_world_setup`. The 0x1A extra is compared on the rolls' own terms: a world seeded with `RUN_004_ROLL_START` holds it exactly where the run did.

use std::collections::BTreeMap;

use bevy_ecs::prelude::*;
use oblivion_sim::tes::actor::{MobileObject, TesObjectRefr};
use oblivion_sim::tes::actor_process::{BaseProcess, LowProcess};
use oblivion_sim::tes::base_extra_list::ExtraData;
use oblivion_sim::tes::form::{FormIdentity, OriginalVtable};

/// IDA: N/A a start of 12d's rolls that lands on RUN-004's first 00675310 draw and agrees with its ACT rows. `phase14_world_setup` says what it does and doesn't show.
pub const RUN_004_ROLL_START: u32 = 0x02BE_F03D;

pub fn le_hex(value: u32) -> String {
    format!(
        "{:02x}{:02x}{:02x}{:02x}",
        value & 0xFF,
        (value >> 8) & 0xFF,
        (value >> 16) & 0xFF,
        value >> 24
    )
}

pub fn triple(values: [f32; 3]) -> String {
    values.iter().map(|value| le_hex(value.to_bits())).collect()
}

pub fn field<'a>(row: &[&'a str], key: &str) -> &'a str {
    let prefix = format!("{key}=");
    row.iter()
        .find_map(|part| part.strip_prefix(prefix.as_str()))
        .unwrap_or_else(|| panic!("no {key} in {:?}", &row[..row.len().min(4)]))
}

pub fn hex(text: &str) -> u32 {
    u32::from_str_radix(text, 16).unwrap()
}

pub fn id_of(world: &World, form: Option<Entity>) -> u32 {
    form.map_or(0, |form| {
        world.get::<FormIdentity>(form).expect("a form").form_id_0c
    })
}

#[derive(Default)]
struct Extras {
    types: Vec<String>,
    details: BTreeMap<String, String>,
}

fn run_extras(text: &str) -> Extras {
    let mut out = Extras::default();
    if text == "-" {
        return out;
    }
    for item in text.split(',') {
        let kind = item[..2].to_owned();
        let slash = item.find('/');
        let payload = &item[3..43.min(item.len())];
        let detail = match kind.as_str() {
            "1E" => format!("{}@{}", &item[slash.unwrap() + 2..], &payload[8..]),
            "21" | "23" => slash.map_or(String::new(), |at| item[at + 1..].to_owned()),
            "11" | "58" | "27" | "22" => {
                slash.map_or(String::new(), |at| item[at + 2..].to_owned())
            }
            _ => String::new(),
        };
        if !detail.is_empty() {
            let slot = out.details.entry(kind.clone()).or_default();
            if !slot.is_empty() {
                slot.push('|');
            }
            slot.push_str(&detail);
        }
        out.types.push(kind);
    }
    out
}

fn host_extras(world: &World, reference: Entity) -> Extras {
    let mut out = Extras::default();
    for entry in &world
        .get::<TesObjectRefr>(reference)
        .unwrap()
        .extra_data_44
        .entries
    {
        let kind = format!("{:02X}", entry.type_04);
        let form_text = |form: Option<Entity>| format!("{:X}", id_of(world, form));
        let detail = match (&entry.data, entry.type_04) {
            (
                ExtraData::PackageStart {
                    location_10,
                    time_1c,
                    ..
                },
                _,
            ) => format!(
                "{}@{}{}{}{}",
                form_text(entry.resolved_0c.flatten()),
                le_hex(location_10[0]),
                le_hex(location_10[1]),
                le_hex(location_10[2]),
                le_hex(*time_1c)
            ),
            (ExtraData::Cell(cell), _) => form_text(Some(*cell)),
            (_, 0x58 | 0x27 | 0x22) => form_text(entry.resolved_0c.flatten()),
            (ExtraData::RunOncePacks(items), _) => items
                .iter()
                .map(|(package, day)| format!("{}:{day:X}", form_text(Some(*package))))
                .collect::<Vec<_>>()
                .join("+"),
            (ExtraData::Followers(items), _) => items
                .iter()
                .map(|follower| form_text(Some(*follower)))
                .collect::<Vec<_>>()
                .join("+"),
            _ => String::new(),
        };
        if !detail.is_empty() {
            let slot = out.details.entry(kind.clone()).or_default();
            if !slot.is_empty() {
                slot.push('|');
            }
            slot.push_str(&detail);
        }
        out.types.push(kind);
    }
    out
}

/// IDA: N/A what a comparison set aside: nothing since the script VM.
#[derive(Default)]
pub struct Tally;

// One ACT row against the host actor: the first difference, if any. Words of `proc` the original left as heap bytes aren't compared: +0x20's top three, +0x84's top three, +0x88, and the +0x78 and +0x7C entry pointers, which are only pointers. Pointers compare by null-ness.
pub fn compare_actor(
    world: &World,
    row: &[&str],
    actor: Entity,
    _tally: &mut Tally,
) -> Option<String> {
    if field(row, "n") == "0" {
        oblivion_core::fixtures::ledger_relaxed(
            "phase14",
            &format!("ACT {} #proc", row[1]),
            1,
            "words of `proc` the original left as heap bytes: +0x20's top three, +0x84's top three, +0x88, the +0x78 and +0x7C entry pointers",
        );
    }
    let identity = world.get::<FormIdentity>(actor).unwrap();
    if hex(field(row, "id")) != identity.form_id_0c {
        return Some(format!("id host {:X}", identity.form_id_0c));
    }
    if hex(field(row, "flags")) != identity.flags_08 {
        return Some(format!(
            "flags host {:X} run {}",
            identity.flags_08,
            field(row, "flags")
        ));
    }
    let reference = world.get::<TesObjectRefr>(actor).unwrap();
    if hex(field(row, "base")) != id_of(world, reference.base_form_1c) {
        return Some(format!("base run {}", field(row, "base")));
    }
    if field(row, "pos") != triple(reference.position_2c) {
        return Some(format!(
            "pos host {} run {}",
            triple(reference.position_2c),
            field(row, "pos")
        ));
    }
    if field(row, "rot") != triple(reference.rotation_20) {
        return Some(format!(
            "rot host {} run {}",
            triple(reference.rotation_20),
            field(row, "rot")
        ));
    }
    if hex(field(row, "cell")) != id_of(world, reference.parent_cell_40) {
        return Some(format!(
            "cell host {:X} run {}",
            id_of(world, reference.parent_cell_40),
            field(row, "cell")
        ));
    }
    let process = world.get::<MobileObject>(actor).unwrap().process_58?;
    let base = world.get::<BaseProcess>(process).unwrap();
    let low = world.get::<LowProcess>(process).unwrap();
    if hex(field(row, "pkg")) != id_of(world, base.editor_package_08) {
        return Some(format!(
            "pkg host {:X} run {}",
            id_of(world, base.editor_package_08),
            field(row, "pkg")
        ));
    }
    let words: Vec<u32> = field(row, "proc").split(',').map(hex).collect();
    let word_1c = u32::from_le_bytes([low.byte_1c, low.byte_1d, low.byte_1e, low.byte_1f]);
    let checks: [(usize, u32, u32); 21] = [
        (
            0x00,
            words[0],
            world.get::<OriginalVtable>(process).unwrap().0,
        ),
        (0x04, words[1], base.edit_pack_procedure_type_04),
        (
            0x08,
            u32::from(words[2] != 0),
            u32::from(base.editor_package_08.is_some()),
        ),
        (0x0C, words[3], low.field_0c.to_bits()),
        (0x10, words[4], low.field_10.to_bits()),
        (0x14, words[5], low.cur_hour_14.to_bits()),
        (0x18, words[6], low.cur_packed_date_18 as u32),
        (0x1C, words[7], word_1c),
        (0x20, words[8] & 0xFF, u32::from(low.byte_20)),
        (0x24, words[9], low.used_item_24),
        (0x28, words[10], low.actor_update_delay_seconds_28.to_bits()),
        (
            0x2C,
            u32::from(words[11] != 0),
            u32::from(low.field_2c.is_some()),
        ),
        (
            0x30,
            u32::from(words[12] != 0),
            u32::from(low.field_30.is_some()),
        ),
        (0x34, words[13], low.pathing_34),
        (0x38, words[14], low.field_38),
        (0x3C, words[15], low.list_3c[0]),
        (0x40, words[16], low.list_3c[1]),
        (0x44, words[17], low.block_44),
        (0x48, words[18], low.block_48),
        (0x84, words[33] & 0xFF, u32::from(low.byte_84)),
        (0x8C, words[35], low.float_8c.to_bits()),
    ];
    for (offset, run, host) in checks {
        if run != host {
            return Some(format!("proc+{offset:X} host {host:X} run {run:X}"));
        }
    }
    let want = run_extras(field(row, "ext"));
    let have = host_extras(world, actor);
    if want.types != have.types {
        return Some(format!(
            "extra-types host {:?} run {:?}",
            have.types, want.types
        ));
    }
    for (kind, detail) in &want.details {
        if have.details.get(kind) != Some(detail) {
            return Some(format!(
                "extra-{kind} host {:?} run {detail}",
                have.details.get(kind)
            ));
        }
    }
    None
}
