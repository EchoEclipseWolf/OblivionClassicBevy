//! TESSaveLoadGame: the change records and the save thread's flag.

use crate::tes::form::FormIdentity;
use crate::tes::forms_world::TesObjectCell;
use crate::tes::hud_main_menu::interface_active_top_menu_id;
use crate::tes::pause_menu::{MENU_ID_PAUSE, interface_first_stacked_menu_mode};
use crate::tes::player::{PlayerCharacter, the_player};
use crate::tes::tes::{
    CHANGES_MAP_BUCKETS, CHANGES_MAP_VTABLE, ChangeRecord, EXTERIOR_CELL_NEW_REFERENCES_MAP_VTABLE,
    GameThreadLocals, INTERIOR_CELL_NEW_REFERENCES_MAP_VTABLE, NUMERIC_ID_BUFFER_MAP_VTABLE,
    NiTLargeArrayU32, NiTPointerMapRecord, TesGlobals, load_form_change,
};
use crate::tes::tile::InterfaceCx;
use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;

// IDA: 00464910 TESSaveLoadGame_ReadCellChanges(cell): 0045A500 was tested by the caller and again here. 004637C0 on the cell, whose al is the answer, which 004CDA22 drops. An interior looks its ID up in +0x08, an exterior its world's in +0x0C (a null world faults); a miss answers. A hit walks the list with 00463D70, a host stop: the maps are empty with no save. Completion: 100% of the arms with no save.
// TODO: [ ] 00464977, 004649E8 the two maps' hits, with save loading.
pub(crate) fn load_cell_change(world: &mut World, cell: Entity) -> Result<(), GameError> {
    load_form_change(world, cell)?;
    let game = world
        .resource::<TesGlobals>()
        .save_load_game
        .as_ref()
        .ok_or(GameError::unclassified(
            0x00464919,
            "00464919: a null g_saveLoadGame",
        ))?;
    let data = world.get::<TesObjectCell>(cell).expect("a cell");
    let found = if data.flags_24 & 1 != 0 {
        game.interior_references_08.count_0c != 0
    } else {
        if data.world_50.is_none() {
            return Err(GameError::unclassified(
                0x004649CF,
                "004649CF: 004C9CF0 gives a null world",
            ));
        }
        game.exterior_references_0c.count_0c != 0
    };
    if found {
        return Err(GameError::unclassified(
            0x00464971,
            "00464971: a saved cell's new references (00463D70)",
        ));
    }
    Ok(())
}

/// IDA: MenuID_Save, MenuID_Load, MenuID_Console.
const MENU_ID_SAVE: u32 = 1039;

const MENU_ID_CONSOLE: u32 = 3;

/// IDA: 00452330 TESSaveLoadGame_CanSaveNow (a thiscall on g_saveLoadGame that reads none of it): the player's death state
/// (005E0DC0, +0xB0) neither 2 nor 1, 0065D140 (+0x71D or +0x71C set) false, and both 00578FE0's top menu and
/// 00579060's first one Save, none, Pause or the console. Completion: 100%.
/// # Errors
/// No player: 00452336 reads null's +0xB0.
pub fn save_load_can_save_now(cx: &InterfaceCx) -> Result<bool, GameError> {
    let player = the_player(cx.world).ok_or(GameError::unclassified(
        0x00452336,
        "00452336: no player, and the original reads its +0xB0",
    ))?;
    let state = cx
        .world
        .get::<crate::tes::actor::Actor>(player)
        .ok_or(GameError::unclassified(
            0x00452336,
            "00452336: the player has no Actor part to read +0xB0 from",
        ))?
        .dead_state_b0;
    if state == 2 || state == 1 {
        return Ok(false);
    }
    let data = cx
        .world
        .get::<PlayerCharacter>(player)
        .ok_or(GameError::unclassified(
            0x0045235B,
            "0045235B: the player has no PlayerCharacter part to read +0x71C from",
        ))?;
    if data.sit_sleep_exit_pending_71d != 0 || data.sit_sleep_entry_pending_71c != 0 {
        return Ok(false);
    }
    let allowed = |id: u32| matches!(id, MENU_ID_SAVE | 0 | MENU_ID_PAUSE | MENU_ID_CONSOLE);
    Ok(allowed(interface_active_top_menu_id(cx)) && allowed(interface_first_stacked_menu_mode(cx)))
}

// IDA: 0045BCC0 TESSaveLoadGame_GetChangeFlags(form, use_04): with `use_04` and a +0x04 map, its record's flags when it has one;
// otherwise the +0x00 map's, 0 for none.
pub(crate) fn change_flags(world: &World, reference: Entity, use_04: bool) -> u32 {
    let id = world
        .get::<FormIdentity>(reference)
        .expect("a form")
        .form_id_0c;
    let Some(save) = world
        .get_resource::<crate::tes::world::tes::TesGlobals>()
        .and_then(|globals| globals.save_load_game.as_ref())
    else {
        return 0;
    };
    if use_04
        && save.field_04 != 0
        && let Some(record) = save.changes_04.get(id)
    {
        return record.flags_00;
    }
    save.changes_00.get(id).map_or(0, |record| record.flags_00)
}

// IDA: 0045A860 / 0045A940 / 0045AA80 / 0045ABE0 the NiTMapBase map family: +0x04 = size, base vtable, +0x0C = 0, FormHeap(4 * size) memset 0 -> +0x08, derived
// vtable. Completion: 100%.
fn construct_pointer_map(size: u32, derived_vtable: u32) -> NiTPointerMapRecord {
    NiTPointerMapRecord {
        original_vtable: derived_vtable,
        hash_size_04: size,
        buckets_08: vec![0; size as usize],
        count_0c: 0,
    }
}

/// IDA: 0045B300 TESSaveLoadGame_Constructor (0x1C4). Fields named by offset; the bytes the constructor does not store stay uninitialized FormHeap bytes (host
/// zero). Completion: 100% of the stores.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TesSaveLoadGame {
    /// ChangesMap (0x13AF).
    pub changes_map_00: NiTPointerMapRecord,
    /// IDA: N/A the chains of +0x00's buckets, which `changes_map_00` only counts.
    pub changes_00: ChangeChains,
    /// +0x04 the save load's own map, null in every startup run (SAVELOAD map04=0). Nonzero means `changes_04` holds its chains.
    pub field_04: u32,
    /// IDA: N/A the chains of +0x04's map.
    pub changes_04: ChangeChains,
    pub interior_references_08: NiTPointerMapRecord,
    pub exterior_references_0c: NiTPointerMapRecord,
    pub exterior_references_10: NiTPointerMapRecord,
    /// +0x14..+0x40.
    pub fields_14: [u32; 12],
    pub field_4c: u32,
    pub field_50: u32,
    /// +0x54..+0x60.
    pub numeric_id_maps_54: [NiTPointerMapRecord; 4],
    pub field_64: u32,
    pub field_68: u32,
    pub field_6c: u32,
    pub byte_70: u8,
    pub byte_71: u8,
    pub array_74: NiTLargeArrayU32,
    pub array_78: NiTLargeArrayU32,
    pub byte_7c: u8,
    pub byte_7d: u8,
    /// +0x80..+0x90.
    pub fields_80: [u32; 5],
    /// +0x94..+0xA0.
    pub fields_94: [u32; 4],
    pub field_a4: u32,
    pub bytes_a8: [u8; 4],
    pub field_ac: u32,
    pub field_1c0: u32,
}

impl TesSaveLoadGame {
    /// IDA: 0045A500 on `this`: on the thread 00B33398 +0x10 names, +0x18 bit 0; on another, bit 18. Host substitution: `World` code all runs on the main thread (the 13e drain too, where the original's IO thread would read bit 18), so this reads bit 0. It therefore doesn't read g_osGlobals, whose null would fault. Completion: 100% of the main thread's arm.
    #[must_use]
    pub const fn active(&self) -> bool {
        self.fields_14[1] & 1 != 0
    }

    // IDA: 00452C20(map, form, flags) on +0x00. Completion: 100%.
    fn add_to_00(&mut self, id: u32, flags: u32) {
        if self.changes_00.add(id, flags) {
            self.changes_map_00.count_0c = self.changes_map_00.count_0c.wrapping_add(1);
        }
    }

    /// IDA: 0045B700's choice, after its test of flag 0x4000: active with a +0x04 map, 00452C20 there; every other way, on +0x00. Completion: 100%.
    pub fn record_form_change(&mut self, id: u32, flags: u32) {
        if self.active() && self.field_04 != 0 {
            self.changes_04.add(id, flags);
        } else {
            self.add_to_00(id, flags);
        }
    }

    /// IDA: 0045B670's choice, after its gates: active needs +0x18 bit 3 and a +0x04 map, and adds there, or nothing happens; not active adds on +0x00. Completion: 100%.
    pub fn set_change_flags(&mut self, id: u32, flags: u32) {
        if self.active() {
            if self.fields_14[1] & 8 != 0 && self.field_04 != 0 {
                self.changes_04.add(id, flags);
            }
        } else {
            self.add_to_00(id, flags);
        }
    }

    /// IDA: 00452D60 ChangesMap_RemoveFlags(form, flags) on +0x00, as 004533D0 reaches it: +0x18 bit 12 answers untouched; a miss too; a record with no +0x04 block loses the flags; a record at zero then goes through 004524C0, whatever its block, whose release is `Err` (00452DC6). Completion: 100% of the arms but the block's release.
    /// # Errors
    /// A record with a block at zero flags.
    pub fn clear_change_flags(&mut self, id: u32, flags: u32) -> Result<(), GameError> {
        if self.fields_14[1] & 0x1000 != 0 {
            return Ok(());
        }
        let Some(record) = self.changes_00.get(id) else {
            return Ok(());
        };
        let left = if record.block_04 == 0 {
            let left = record.flags_00 & !flags;
            self.changes_00.set_flags(id, left);
            left
        } else {
            record.flags_00
        };
        if left != 0 {
            return Ok(());
        }
        if record.block_04 != 0 {
            return Err(GameError::unclassified(
                0x00452DC6,
                "00452DC6: 00401E40 releases a change record's block",
            ));
        }
        self.changes_00.remove(id);
        self.changes_map_00.count_0c = self.changes_map_00.count_0c.wrapping_sub(1);
        Ok(())
    }

    /// IDA: 00452DF0(this->+0x00, id, release): +0x18 bit 12 answers false untouched; a miss answers false; a record with a block stays unless `release`; otherwise 004524C0, the block to 00B02020's 00401E40, the free. Answers whether a record was found. `Err` is 00452E50, a block no host path makes. Completion: 100% of the arms but the block's release.
    /// # Errors
    /// A record with a +0x04 block being released.
    pub fn remove_change(&mut self, id: u32, release: bool) -> Result<bool, GameError> {
        if self.fields_14[1] & 0x1000 != 0 {
            return Ok(false);
        }
        let Some(record) = self.changes_00.get(id) else {
            return Ok(false);
        };
        if record.block_04 != 0 {
            if !release {
                return Ok(true);
            }
            return Err(GameError::unclassified(
                0x00452E50,
                "00452E50: 00401E40 releases a change record's block",
            ));
        }
        self.changes_00.remove(id);
        self.changes_map_00.count_0c = self.changes_map_00.count_0c.wrapping_sub(1);
        Ok(true)
    }
}

/// IDA: 0045B780 TESSaveLoadGame_RemoveChanges(form, release) on g_saveLoadGame: a temporary form returns, otherwise a tail call of 00452DF0 on +0x00. The caller tested g_saveLoadGame. Completion: 100%.
/// # Errors
/// As `remove_change`.
pub fn remove_form_changes(
    world: &mut World,
    form: Entity,
    release: bool,
) -> Result<(), GameError> {
    let identity = *world.get::<FormIdentity>(form).expect("a form");
    if identity.flags_08 & 0x4000 != 0 {
        return Ok(());
    }
    let mut globals = world.resource_mut::<TesGlobals>();
    let game = globals
        .save_load_game
        .as_mut()
        .ok_or(GameError::unclassified(
            0x0045B792,
            "0045B792: g_saveLoadGame is null",
        ))?;
    game.remove_change(identity.form_id_0c, release)?;
    Ok(())
}

// IDA: 0045B300 TESSaveLoadGame_Constructor. Completion: 100% of the store order.
pub(crate) fn construct_save_load_game(
    globals: &mut TesGlobals,
    locals: &mut GameThreadLocals,
) -> Result<TesSaveLoadGame, GameError> {
    // 0045B338..0045B3C0 the zero stores; +0x7D = 1.
    let mut game = TesSaveLoadGame {
        byte_7d: 1,
        ..TesSaveLoadGame::default()
    };
    // 0045B3C8 00B33B00 = this (before the TES constructor's own store).
    globals.trace(0x00B3_3B00, 4);
    // 0045B3CE TLS +0x185 = 0.
    locals.track_changes_185 = 0;
    // 0045B3D4..0045B529 the eight maps.
    game.changes_map_00 = construct_pointer_map(0x13AF, CHANGES_MAP_VTABLE);
    game.interior_references_08 =
        construct_pointer_map(37, INTERIOR_CELL_NEW_REFERENCES_MAP_VTABLE);
    game.exterior_references_0c =
        construct_pointer_map(37, EXTERIOR_CELL_NEW_REFERENCES_MAP_VTABLE);
    game.exterior_references_10 =
        construct_pointer_map(37, EXTERIOR_CELL_NEW_REFERENCES_MAP_VTABLE);
    for map in &mut game.numeric_id_maps_54 {
        *map = construct_pointer_map(37, NUMERIC_ID_BUFFER_MAP_VTABLE);
    }
    // 0045B529..0045B5AA NiTLargeArray<uint>(100, 50) -> +0x74; capacity nonzero so 00452910 is skipped; 00446C50(0, &zero).
    game.array_74 = NiTLargeArrayU32::new(100, 50);
    game.array_74.set_at(0, 0)?;
    // 0045B5AF..0045B629 NiTLargeArray<uint>(5, 1) -> +0x78; likewise.
    game.array_78 = NiTLargeArrayU32::new(5, 1);
    game.array_78.set_at(0, 0)?;
    // 0045B62E..0045B650 +0x94..+0xA0 = 0, +0x71 = +0x7C = 0x7D, +0x70 = 0.
    game.fields_94 = [0; 4];
    game.byte_71 = 0x7D;
    game.byte_7c = 0x7D;
    game.byte_70 = 0;
    Ok(game)
}

/// IDA: N/A a change map's buckets (vt+4 006A9060: key modulo +0x04), each chain from its head, as 00452570 links a new key and 00452600 walks. The +0x04 size of the one 0x13AF map, from 0045B300.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChangeChains {
    buckets: Vec<Vec<(u32, ChangeRecord)>>,
}

impl ChangeChains {
    // IDA: N/A a helper, with no address of its own.
    fn bucket(key: u32) -> usize {
        (key % CHANGES_MAP_BUCKETS) as usize
    }

    /// IDA: 0055E000 on the map. Completion: 100%.
    #[must_use]
    pub fn get(&self, key: u32) -> Option<ChangeRecord> {
        self.buckets
            .get(Self::bucket(key))?
            .iter()
            .find(|(entry, _)| *entry == key)
            .map(|(_, record)| *record)
    }

    /// IDA: 00452C20's body without the count: a found record, otherwise a new {0, 0} at the chain's head (00452570); then `flags` ORs in when +0x04 is null. Returns whether it added. Completion: 100%; FormHeap(8) failing stores a null value there and faults at 00452C78, which the host doesn't model.
    pub fn add(&mut self, key: u32, flags: u32) -> bool {
        if self.buckets.is_empty() {
            self.buckets
                .resize_with(CHANGES_MAP_BUCKETS as usize, Vec::new);
        }
        let chain = &mut self.buckets[Self::bucket(key)];
        let (added, record) = match chain.iter().position(|(entry, _)| *entry == key) {
            Some(at) => (false, &mut chain[at].1),
            None => {
                chain.insert(0, (key, ChangeRecord::default()));
                (true, &mut chain[0].1)
            }
        };
        if record.block_04 == 0 {
            record.flags_00 |= flags;
        }
        added
    }

    // IDA: N/A a store through 0055E000's value.
    pub(crate) fn set_flags(&mut self, key: u32, flags: u32) {
        if let Some(chain) = self.buckets.get_mut(Self::bucket(key))
            && let Some((_, record)) = chain.iter_mut().find(|(entry, _)| *entry == key)
        {
            record.flags_00 = flags;
        }
    }

    /// IDA: 004524C0 RemoveAt. Completion: 100%.
    pub fn remove(&mut self, key: u32) -> bool {
        let Some(chain) = self.buckets.get_mut(Self::bucket(key)) else {
            return false;
        };
        let Some(at) = chain.iter().position(|(entry, _)| *entry == key) else {
            return false;
        };
        chain.remove(at);
        true
    }

    /// IDA: 00452600's order: bucket by bucket, each chain from its head. A row printer walks this.
    pub fn walk(&self) -> impl Iterator<Item = (u32, ChangeRecord)> + '_ {
        self.buckets.iter().flatten().copied()
    }

    /// IDA: N/A the number of entries, which `NiTPointerMapRecord::count_0c` holds in the original.
    #[must_use]
    pub fn len(&self) -> usize {
        self.buckets.iter().map(Vec::len).sum()
    }

    /// IDA: N/A an accessor.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.buckets.iter().all(Vec::is_empty)
    }
}
