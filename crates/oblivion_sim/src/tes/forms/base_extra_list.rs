//! Phase 12c: BaseExtraList, the ExtraDataList at TESObjectREFR +0x44 and TESObjectCELL +0x28, and its chunk loader 004259E0. In the C++, base_extra_list.
//!
//! An entry is a `BaseExtraEntry`, and the list's +0x04 chain is the `Vec`'s order. The host keeps no 00B33800 lock and no TLS cache of 0041DF90 (TLS +0x08, +0x10..): neither holds game state. Evidence: objdump of each address named here; the presence bytes and the head's null-ness in P12-RUN-001's CELL and REFR samples. No row reads behind the head.

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;
use oblivion_core::x87;

use super::form::FormIdentity;
use super::form_arms::FormLoadCx;
use super::form_components::load_full_name;
use crate::bs::string::BsStringT;
use crate::tes::actor::TesObjectRefr;
use crate::tes::ai_cell_moves::cell_list;
use crate::tes::data_handler::DataHandlerLists;
use crate::tes::forms_references::parent_cell;
use crate::tes::forms_script::create_script_event_list;
use crate::tes::forms_world::{TesObjectCell, cell_set_flag_40};
use crate::tes::inventory::run_with_temporary;
use crate::tes::tes_file::{map_file_form_id, read_dword};

// IDA: the chunks 004259E0 compares.
const XMRC: u32 = 0x4352_4D58;
const XLOC: u32 = 0x434F_4C58;
const XGLB: u32 = 0x424C_4758;
const XSED: u32 = 0x4445_5358;
const XUSE: u32 = 0x4553_5558;
const XLOD: u32 = 0x444F_4C58;
const XRGD: u32 = 0x4447_5258;
const XCHG: u32 = 0x4748_4358;
const XRNK: u32 = 0x4B4E_5258;
const XTRG: u32 = 0x4752_5458;
const XPCI: u32 = 0x4943_5058;
const XMRK: u32 = 0x4B52_4D58;
const XTEL: u32 = 0x4C45_5458;
const XEDL: u32 = 0x4C44_4558;
const XCLM: u32 = 0x4D4C_4358;
const XSOL: u32 = 0x4C4F_5358;
const XPSL: u32 = 0x4C53_5058;
const XCCM: u32 = 0x4D43_4358;
const XRTM: u32 = 0x4D54_5258;
const XTIM: u32 = 0x4D49_5458;
const XLCM: u32 = 0x4D43_4C58;
const XPSN: u32 = 0x4E53_5058;
const XOWN: u32 = 0x4E57_4F58;
const XESP: u32 = 0x5053_4558;
const XCLR: u32 = 0x524C_4358;
const XHRS: u32 = 0x5352_4858;
const XCMT: u32 = 0x544D_4358;
const XACT: u32 = 0x5443_4158;
const XHLT: u32 = 0x544C_4858;
const XCNT: u32 = 0x544E_4358;
const XCWT: u32 = 0x5457_4358;
const XCLW: u32 = 0x574C_4358;
const FNAM: u32 = 0x4D41_4E46;
const FULL: u32 = 0x4C4C_5546;
const TNAM: u32 = 0x4D41_4E54;

/// IDA: the extra types this file names.
/// IDA: 0041DAC0 ExtraHavok: +0x0C the bhkWorld, +0x10 the CellMopp.
pub const EXTRA_HAVOK: u8 = 0x02;
pub const EXTRA_WATER_HEIGHT: u8 = 0x04;
pub const EXTRA_WATER: u8 = 0x05;
pub const EXTRA_REGION_LIST: u8 = 0x08;
pub const EXTRA_EDITOR_ID: u8 = 0x0A;
pub const EXTRA_CELL_MUSIC_TYPE: u8 = 0x0B;
pub const EXTRA_CELL_CLIMATE: u8 = 0x0C;
pub const EXTRA_TYPE_0D: u8 = 0x0D;
pub const EXTRA_PERSISTENT_CELL: u8 = 0x11;
pub const EXTRA_ACTION: u8 = 0x13;
pub const EXTRA_LOD: u8 = 0x18;
pub const EXTRA_RAG_DOLL: u8 = 0x19;
pub const EXTRA_PACKAGE_START: u8 = 0x1E;
pub const EXTRA_LEVEL_CREATURE_MODIFIER: u8 = 0x24;
pub const EXTRA_OWNER: u8 = 0x27;
pub const EXTRA_GLOBAL: u8 = 0x28;
pub const EXTRA_RANK: u8 = 0x29;
pub const EXTRA_COUNT: u8 = 0x2A;
/// IDA: 00485E00's type.
pub const EXTRA_CONTAINER_CHANGES: u8 = 0x1A;
/// IDA: 00429FF0 ExtraWorn; 0041DF10 tests it with 0x1C, ExtraWornLeft.
pub const EXTRA_WORN: u8 = 0x1B;
/// IDA: 00429FD0 ExtraGhost.
pub const EXTRA_GHOST: u8 = 0x25;
pub const EXTRA_WORN_LEFT: u8 = 0x1C;
/// IDA: 0041FE90's type.
pub const EXTRA_LEVELED_ITEM: u8 = 0x36;
pub const EXTRA_HEALTH: u8 = 0x2B;
pub const EXTRA_USES: u8 = 0x2C;
pub const EXTRA_TIME_LEFT: u8 = 0x2D;
pub const EXTRA_CHARGE: u8 = 0x2E;
pub const EXTRA_SOUL: u8 = 0x2F;
pub const EXTRA_LIGHT: u8 = 0x30;
pub const EXTRA_LOCK: u8 = 0x31;
pub const EXTRA_TELEPORT: u8 = 0x32;
pub const EXTRA_MAP_MARKER: u8 = 0x33;
pub const EXTRA_LEVELED_SEED: u8 = 0x38;
pub const EXTRA_ENABLE_STATE_PARENT: u8 = 0x3F;
pub const EXTRA_RANDOM_TELEPORT_MARKER: u8 = 0x43;
pub const EXTRA_MERCHANT_CONTAINER: u8 = 0x44;
pub const EXTRA_PERSUASION: u8 = 0x48;
pub const EXTRA_TRESPASS: u8 = 0x4D;
pub const EXTRA_HORSE: u8 = 0x58;
/// IDA: 0x47 ExtraCannotWear (0042A050), the equip lock: 0041F370 ExtraDataList_SetEquipLocked adds or removes it,
/// 0041DF40 ExtraDataList_IsEquipLocked tests its presence bit (+0x10 bit 7); the auto-equip searches keep a worn stack with it.
pub const EXTRA_CANNOT_WEAR: u8 = 0x47;
/// IDA: 0x50 ExtraBoundArmor (0042A010 ExtraBoundArmor_Constructor), presence byte +0x12 bit 0.
pub const EXTRA_BOUND_ARMOR: u8 = 0x50;
/// IDA: 0042A3B0, 00429B20.
pub const EXTRA_RUN_ONCE_PACKS: u8 = 0x21;
pub const EXTRA_REFERENCE_POINTER: u8 = 0x22;
pub const EXTRA_FOLLOWER: u8 = 0x23;

/// IDA: 0042B3F0 ExtraMapMarker's 0x10 block: +0x00 TESFullName vtable A322A0, +0x04 its BSStringT, +0x0C flags, +0x0E the marker type.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MapMarkerData {
    pub full_name_vtable_00: u32,
    pub name_04: BsStringT,
    pub flags_0c: u8,
    pub marker_type_0e: u16,
}

/// IDA: 00497210 the 8-byte ragdoll block: +0x00 u8 count, +0x04 the bytes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RagDollData {
    pub count_00: u8,
    pub data_04: Option<Vec<u8>>,
}

/// IDA: 004A62D0's 0x10 region list: vtable A4034C, +0x04 the BSSimpleList, +0x0C its argument (0 here).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExtraRegionList {
    pub original_vtable: u32,
    pub regions_04: oblivion_core::pointer_list::PointerList<Entity>,
    pub byte_0c: u8,
}

/// IDA: what an entry holds from its +0x0C on, by class. A byte or word class's other bytes are FormHeap bytes in the original.
#[derive(Clone, Debug, PartialEq)]
pub enum ExtraData {
    /// +0x0C a dword (a raw form ID slot, a rank).
    Dword(u32),
    /// +0x0C a byte.
    Byte(u8),
    /// +0x0C a word.
    Word(u16),
    /// +0x0C a float, as stored.
    Float(u32),
    /// 0x0A: +0x0C a BSStringT.
    EditorId(BsStringT),
    /// 0x08: +0x0C the region list.
    Regions(Box<ExtraRegionList>),
    /// 0x11: +0x0C the persistent cell.
    Cell(Entity),
    /// 0x13: +0x0C the flags (1), +0x10 the activator.
    Action { flags_0c: u8, ref_10: u32 },
    /// 0x18: +0x0C..+0x14.
    Lod([u32; 3]),
    /// 0x19: +0x0C the block.
    RagDoll(Box<RagDollData>),
    /// 0x1E: +0x0C the target, +0x10..+0x18 the location, +0x1C the time.
    PackageStart {
        target_0c: u32,
        location_10: [u32; 3],
        time_1c: u32,
    },
    /// 0x31: +0x0C's 0x0C block, as bytes: +0x00 level, +0x04 key, +0x08 flags. The block's three padding bytes after the level and after the flags stay what a 12-byte chunk read over them.
    Lock(Box<[u8; 12]>),
    /// 0x32: +0x0C's 0x1C block (0042B590).
    Teleport(Box<[u32; 7]>),
    /// 0x33: +0x0C's block.
    MapMarker(Box<MapMarkerData>),
    /// 0x3F: +0x0C the parent, +0x10 the flags.
    EnableStateParent { parent_0c: u32, flags_10: u8 },
    /// 0x12: 00429F20: +0x0C the script, +0x10 the event list.
    Script {
        script_0c: Entity,
        events_10: Option<super::forms_script::ScriptEventList>,
    },
    /// 0x14: 0042AC40: +0x0C the position, +0x18 the rotation, as stored.
    StartingPosition {
        position_0c: [u32; 3],
        rotation_18: [u32; 3],
    },
    /// 0x1A: 00429D50: +0x0C the 00485D50 block.
    ContainerChanges(Box<crate::tes::inventory::ContainerChanges>),
    /// 0x36: 0042A2B0: +0x0C the base container's index, +0x10 a byte.
    LeveledItem { index_0c: u32, byte_10: u8 },
    /// 0x21: 0042A3B0: +0x0C the list of FormHeap(8) {package, byte day}, in list order.
    RunOncePacks(Vec<(Entity, u8)>),
    /// 0x23: 00429B20: +0x0C the list of followers, in list order.
    Followers(Vec<Entity>),
    /// 0x40: 0042A600: +0x0C the child list, in list order.
    EnableStateChildren(Vec<Entity>),
    /// 0x30: 00429970 ExtraLight (vtable A357AC): +0x0C the FormHeap(8) ExtraLightData {+0x00 the light, +0x04 1.0}.
    LightData { light_00: Entity, fade_04: f32 },
    /// 0x4A: 0042A720 ExtraLastFinishedSequence (vtable A359EC): +0x0C the FormHeap copy of a sequence name.
    LastFinishedSequence(Vec<u8>),
    /// An entry of 0x0C with nothing past +0x08: 0x1B ExtraWorn (00429FF0).
    Marker,
    /// 0x02: 0041DAC0 ExtraHavok (0x14, vtable A34DD4): +0x0C the bhkWorld and +0x10 the CellMopp, both NiPointers.
    Havok {
        world_0c: Option<Entity>,
        cell_mopp_10: Option<Entity>,
    },
}

/// IDA: BaseExtraList's entry (vtable, +0x04 u8 type, +0x08 next, the data from +0x0C).
#[derive(Clone, Debug, PartialEq)]
pub struct BaseExtraEntry {
    pub original_vtable: u32,
    pub type_04: u8,
    pub data: ExtraData,
    /// IDA: the form 00426270 stores over the raw word: `Some` once its arm has run. The raw word stays in `data` for a printer.
    pub resolved_0c: Option<Option<Entity>>,
}

// IDA: the vtable each constructor stores (0041D920, 0041D940, 0041D960, 0041D980, 0041D9A0, 00424510, 00429A80, 00429C10, 00429CA0, 00429D30, 00429E00..00429F00, 0042A160, 0042A2E0, 0042A300, 0042A4C0, 0042A5E0, 0042A660..0042A6C0, 0042A700, 0042A820, 0042AE70).
const fn entry_vtable(extra_type: u8) -> u32 {
    match extra_type {
        EXTRA_HAVOK => 0x00A3_4DD4,
        EXTRA_WATER_HEIGHT => 0x00A3_4D68,
        EXTRA_WATER => 0x00A3_4D98,
        EXTRA_LIGHT => 0x00A3_57AC,
        0x4A => 0x00A3_59EC,
        EXTRA_TYPE_0D => 0x00A3_4DA4,
        EXTRA_REGION_LIST => 0x00A3_4D74,
        EXTRA_EDITOR_ID => 0x00A3_5140,
        EXTRA_CELL_MUSIC_TYPE => 0x00A3_4D80,
        EXTRA_CELL_CLIMATE => 0x00A3_4D8C,
        EXTRA_PERSISTENT_CELL => 0x00A3_5944,
        EXTRA_ACTION => 0x00A3_57F4,
        EXTRA_LOD => 0x00A3_5974,
        EXTRA_RAG_DOLL => 0x00A3_5950,
        EXTRA_PACKAGE_START => 0x00A3_5A7C,
        EXTRA_LEVEL_CREATURE_MODIFIER => 0x00A3_59C8,
        EXTRA_OWNER => 0x00A3_5818,
        EXTRA_GLOBAL => 0x00A3_5824,
        EXTRA_RANK => 0x00A3_5830,
        EXTRA_COUNT => 0x00A3_583C,
        EXTRA_HEALTH => 0x00A3_5848,
        EXTRA_USES => 0x00A3_5854,
        EXTRA_TIME_LEFT => 0x00A3_5860,
        EXTRA_CHARGE => 0x00A3_586C,
        EXTRA_SOUL => 0x00A3_5878,
        EXTRA_LOCK => 0x00A3_57B8,
        EXTRA_TELEPORT => 0x00A3_57DC,
        EXTRA_MAP_MARKER => 0x00A3_57E8,
        EXTRA_LEVELED_SEED => 0x00A3_58FC,
        EXTRA_ENABLE_STATE_PARENT => 0x00A3_598C,
        EXTRA_RANDOM_TELEPORT_MARKER => 0x00A3_59A4,
        EXTRA_MERCHANT_CONTAINER => 0x00A3_59B0,
        EXTRA_PERSUASION => 0x00A3_59E0,
        EXTRA_TRESPASS => 0x00A3_59F8,
        EXTRA_HORSE => 0x00A3_59BC,
        EXTRA_RUN_ONCE_PACKS => 0x00A3_5968,
        EXTRA_FOLLOWER => 0x00A3_57C4,
        0x12 => 0x00A3_5884,
        0x14 => 0x00A3_5A70,
        EXTRA_CONTAINER_CHANGES => 0x00A3_5800,
        EXTRA_WORN => 0x00A3_58A8,
        EXTRA_GHOST => 0x00A3_589C,
        EXTRA_LEVELED_ITEM => 0x00A3_5938,
        0x40 => 0x00A3_5998,
        0x4C => 0x00A3_4DBC,
        _ => 0,
    }
}

// IDA: N/A a new entry, as its class's constructor leaves it.
const fn new_entry(extra_type: u8, data: ExtraData) -> BaseExtraEntry {
    BaseExtraEntry {
        original_vtable: entry_vtable(extra_type),
        type_04: extra_type,
        data,
        resolved_0c: None,
    }
}

/// IDA: N/A a new entry from outside this file, as its class's constructor leaves it.
#[must_use]
pub const fn new_extra_entry(extra_type: u8, data: ExtraData) -> BaseExtraEntry {
    new_entry(extra_type, data)
}

// IDA: the presence bit of a type: `type >> 3` below 12, `1 << (type & 7)`. Types past 0x5F have no bit.
const fn presence_bit(extra_type: u8) -> Option<(usize, u8)> {
    let index = (extra_type >> 3) as usize;
    if index < 12 {
        Some((index, 1 << (extra_type & 7)))
    } else {
        None
    }
}

/// IDA: 00422EE0 ExtraDataList_Constructor (0x14): vtable A350F8, +0x04 the entries, +0x08..+0x13 the presence bits. Completion: 100%.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ExtraDataList {
    pub original_vtable: u32,
    /// +0x04 and each entry's +0x08, in link order.
    pub entries: Vec<BaseExtraEntry>,
    pub presence_08: [u8; 12],
}

impl ExtraDataList {
    /// IDA: 00422EE0 ExtraDataList_Constructor: an empty list with ExtraDataList's vtable (00A350F8). Completion: 100%.
    #[must_use]
    pub fn constructed() -> Self {
        Self {
            original_vtable: 0x00A3_50F8,
            ..Self::default()
        }
    }

    /// IDA: 0041DF10 ExtraDataList_IsWorn(list, 0): 0x1B or 0x1C is there. Completion: 100%.
    #[must_use]
    pub fn is_worn(&self) -> bool {
        self.present(EXTRA_WORN) || self.present(EXTRA_WORN_LEFT)
    }

    /// IDA: 0041F370 ExtraDataList_SetEquipLocked(list, lock): a lock adds a new 0x47 unless its bit is set; an unlock removes
    /// it. Completion: 100%.
    pub fn set_equip_locked(&mut self, lock: bool) {
        let locked = self.present(EXTRA_CANNOT_WEAR);
        if lock {
            if !locked {
                self.add(new_extra_entry(EXTRA_CANNOT_WEAR, ExtraData::Marker));
            }
        } else if locked {
            self.remove_by_type(EXTRA_CANNOT_WEAR);
        }
    }

    /// IDA: 0041F2F0 ExtraDataList_EnsureBoundArmor(list): a new ExtraBoundArmor (0x50) unless its bit is set. Completion: 100%.
    pub fn ensure_bound_armor(&mut self) {
        if !self.present(EXTRA_BOUND_ARMOR) {
            self.add(new_extra_entry(EXTRA_BOUND_ARMOR, ExtraData::Marker));
        }
    }

    /// IDA: N/A +0x04 for a printer: 0 is null. A nonzero value only says an entry is there.
    #[must_use]
    pub const fn head_04(&self) -> u32 {
        !self.entries.is_empty() as u32
    }

    // IDA: N/A an accessor.
    pub(crate) fn present(&self, extra_type: u8) -> bool {
        presence_bit(extra_type).is_some_and(|(index, mask)| self.presence_08[index] & mask != 0)
    }

    /// IDA: 0041E210 BaseExtraList_GetByType: null without the presence bit, else the first entry of the type. Completion: 100%.
    #[must_use]
    pub fn get(&self, extra_type: u8) -> Option<&BaseExtraEntry> {
        if !self.present(extra_type) {
            return None;
        }
        self.entries
            .iter()
            .find(|entry| entry.type_04 == extra_type)
    }

    /// IDA: 0041E210, for a store through the result. Completion: 100%.
    pub fn get_mut(&mut self, extra_type: u8) -> Option<&mut BaseExtraEntry> {
        if !self.present(extra_type) {
            return None;
        }
        self.entries
            .iter_mut()
            .find(|entry| entry.type_04 == extra_type)
    }

    /// IDA: 0041E0A0 BaseExtraList_AddExtra: types 0x11, 0x12, 0x13, 0x1A and 0x32 go to the front (bytes 0041E138 over 0041E130), the rest to the back; then the presence bit. Completion: 100%.
    pub fn add(&mut self, entry: BaseExtraEntry) {
        let extra_type = entry.type_04;
        if matches!(extra_type, 0x11 | 0x12 | 0x13 | 0x1A | 0x32) {
            self.entries.insert(0, entry);
        } else {
            self.entries.push(entry);
        }
        if let Some((index, mask)) = presence_bit(extra_type) {
            self.presence_08[index] |= mask;
        }
    }

    /// IDA: 00422E20 BaseExtraList_RemoveExtra(entry, 1), for the entry `get` found of a type: 0041E2D0 finds the node before the first of the type, the entry is unlinked and deleted, the bit clears. Completion: 100%.
    pub fn remove_entry(&mut self, extra_type: u8) {
        if let Some(at) = self
            .entries
            .iter()
            .position(|entry| entry.type_04 == extra_type)
        {
            self.entries.remove(at);
        }
        if let Some((index, mask)) = presence_bit(extra_type) {
            self.presence_08[index] &= !mask;
        }
    }

    /// IDA: 0041E210 BaseExtraList_GetByType then 00422E20, the arm of the setters' empty value: nothing happens unless the bit is set and the entry exists. Completion: 100%.
    pub fn remove_if_found(&mut self, extra_type: u8) {
        if self.get(extra_type).is_some() {
            self.remove_entry(extra_type);
        }
    }

    /// IDA: 0041E160 BaseExtraList_RemoveByType(type): the first of the type goes if there is one; the bit clears either way, even when `get` would have found nothing. Completion: 100%.
    pub fn remove_by_type(&mut self, extra_type: u8) {
        self.remove_entry(extra_type);
    }

    /// IDA: 0041DFF0 BaseExtraList_RemoveAll(1): every entry deleted, +0x08..+0x13 cleared. Completion: 100%.
    pub fn remove_all(&mut self) {
        self.entries.clear();
        self.presence_08 = [0; 12];
    }

    /// IDA: 0041E450, the cell's vt+0x18: every entry goes but type 8, and with g_saveLoadGame +0x18 bit 2 types 2 and 3. It clears no presence bit, so a type it removes keeps its bit and `get` walks and finds nothing. The C++ clears them. Completion: 100%.
    pub fn remove_for_cell_reset(&mut self, keep_2_and_3: bool) {
        self.entries
            .retain(|entry| entry.type_04 == 8 || (keep_2_and_3 && matches!(entry.type_04, 2 | 3)));
    }

    /// IDA: 0041FF80 the type 0x11 entry's +0x0C. Completion: 100%.
    #[must_use]
    pub fn persistent_cell(&self) -> Option<Entity> {
        match self.get(EXTRA_PERSISTENT_CELL)?.data {
            ExtraData::Cell(cell) => Some(cell),
            _ => None,
        }
    }

    /// IDA: 004247B0 BaseExtraList_SetPersistentCell(cell): an entry takes the cell, or goes (00422E20) for a null one; without one, a nonnull cell adds one (0042A2E0). Completion: 100%.
    pub fn set_persistent_cell(&mut self, cell: Option<Entity>) {
        if self.get(EXTRA_PERSISTENT_CELL).is_some() {
            match cell {
                None => self.remove_entry(EXTRA_PERSISTENT_CELL),
                Some(cell) => {
                    self.get_mut(EXTRA_PERSISTENT_CELL)
                        .expect("checked above")
                        .data = ExtraData::Cell(cell);
                }
            }
        } else if let Some(cell) = cell {
            self.add(new_entry(EXTRA_PERSISTENT_CELL, ExtraData::Cell(cell)));
        }
    }

    /// IDA: 0041FA30 the type 0x0A entry's +0x0C char* (null with no entry, or a string with no text). Completion: 100%.
    #[must_use]
    pub fn editor_id(&self) -> Option<&[u8]> {
        match &self.get(EXTRA_EDITOR_ID)?.data {
            ExtraData::EditorId(text) => text.data_00.as_deref(),
            _ => None,
        }
    }

    /// IDA: 004245E0 BaseExtraList_SetEditorID(source): null removes; an entry takes the text (004028D0 minimum 0); otherwise 00424510 adds one. Completion: 100%.
    pub fn set_editor_id(&mut self, source: Option<&[u8]>) {
        let Some(source) = source else {
            if self.get(EXTRA_EDITOR_ID).is_some() {
                self.remove_entry(EXTRA_EDITOR_ID);
            }
            return;
        };
        if let Some(entry) = self.get_mut(EXTRA_EDITOR_ID) {
            if let ExtraData::EditorId(text) = &mut entry.data {
                text.set(Some(source), 0);
            }
            return;
        }
        let mut text = BsStringT::default();
        text.set(Some(source), 0);
        self.add(new_entry(EXTRA_EDITOR_ID, ExtraData::EditorId(text)));
    }

    /// IDA: 00423FF0 BaseExtraList_SetWaterHeight(value): `fucom` with 0: an equal value (+0 or -0) removes an entry; a NaN or anything else stores, through `fld`/`fstp`. Completion: 100%.
    pub fn set_water_height(&mut self, bits: u32) {
        let exists = self.get(EXTRA_WATER_HEIGHT).is_some();
        let value = x87::fld_fstp_copy(bits);
        if value == 0.0 {
            if exists {
                self.remove_entry(EXTRA_WATER_HEIGHT);
            }
            return;
        }
        if exists {
            self.get_mut(EXTRA_WATER_HEIGHT)
                .expect("checked above")
                .data = ExtraData::Float(value.to_bits());
        } else {
            self.add(new_entry(
                EXTRA_WATER_HEIGHT,
                ExtraData::Float(value.to_bits()),
            ));
        }
    }

    // IDA: 0041EB90 the type 0x13 entry, or a new one: 00429D30 stores flags 1, +0x10 = 0. Completion: 100%.
    fn action_entry(&mut self) -> &mut BaseExtraEntry {
        if self.get(EXTRA_ACTION).is_none() {
            self.add(new_entry(
                EXTRA_ACTION,
                ExtraData::Action {
                    flags_0c: 1,
                    ref_10: 0,
                },
            ));
        }
        self.get_mut(EXTRA_ACTION).expect("just added")
    }

    // IDA: 00423DA0's body, past its argument: no entry: 1 adds nothing, otherwise a new one takes the byte. An entry: 1 with a null +0x10 removes it, otherwise it takes the byte. Completion: 100%.
    fn set_action_flags(&mut self, flags: u32) {
        match self.get(EXTRA_ACTION).map(|entry| &entry.data) {
            None if flags == 1 => return,
            Some(ExtraData::Action { ref_10: 0, .. }) if flags == 1 => {
                self.remove_entry(EXTRA_ACTION);
                return;
            }
            _ => {}
        }
        if let ExtraData::Action { flags_0c, .. } = &mut self.action_entry().data {
            *flags_0c = flags as u8;
        }
    }

    /// IDA: 00423E50: the type 0x13 byte (1 with no entry) with `flags` cleared, then 00423DA0's arms. Completion: 100%.
    pub fn clear_action_flags(&mut self, flags: u32) {
        let current = match self.get(EXTRA_ACTION).map(|entry| &entry.data) {
            Some(ExtraData::Action { flags_0c, .. }) => u32::from(*flags_0c),
            _ => 1,
        };
        self.set_action_flags(!flags & current);
    }

    /// IDA: 00423DF0 the ONAM store: the type 0x13 byte (1 with no entry) ORed with `flags`, then 00423DA0's arms. Completion: 100%.
    pub fn or_action_flags(&mut self, flags: u32) {
        let current = match self.get(EXTRA_ACTION).map(|entry| &entry.data) {
            Some(ExtraData::Action { flags_0c, .. }) => u32::from(*flags_0c),
            _ => 1,
        };
        self.set_action_flags(current | flags);
    }

    // IDA: the shape of 004204E0, 004205C0, 00420780, 00420860, 004206A0, 00420D40 (and 00420280 for its +0x0C): zero goes to 0041E160; an entry takes the dword; otherwise a new entry does. Completion: 100%.
    fn set_dword_or_remove_by_type(&mut self, extra_type: u8, value: u32) {
        if value == 0 {
            self.remove_by_type(extra_type);
            return;
        }
        self.store_dword(extra_type, value);
    }

    // IDA: the shape of 00423660, 00423720, 00424380: zero removes an entry through 00422E20, and changes nothing without one; an entry takes the dword; otherwise a new entry does. Completion: 100%.
    fn set_dword_or_remove_entry(&mut self, extra_type: u8, value: u32) {
        if value == 0 {
            if self.get(extra_type).is_some() {
                self.remove_entry(extra_type);
            }
            return;
        }
        self.store_dword(extra_type, value);
    }

    // IDA: N/A an entry's +0x0C dword, or a new entry holding it.
    fn store_dword(&mut self, extra_type: u8, value: u32) {
        if let Some(entry) = self.get_mut(extra_type) {
            entry.data = ExtraData::Dword(value);
        } else {
            self.add(new_entry(extra_type, ExtraData::Dword(value)));
        }
    }

    // IDA: N/A an entry's +0x0C byte, or a new entry holding it: 0041ED50, 0041EF30, 00424380's byte form 004242C0 past its zero test, 00423BD0 past its 0xFF test.
    fn store_byte(&mut self, extra_type: u8, value: u8) {
        if let Some(entry) = self.get_mut(extra_type) {
            entry.data = ExtraData::Byte(value);
        } else {
            self.add(new_entry(extra_type, ExtraData::Byte(value)));
        }
    }

    // IDA: N/A an entry's +0x0C float, or a new entry holding it: 0041ECB0, 0041EDF0, 0041EE90. The value comes in through `fld`/`fstp`.
    fn store_float(&mut self, extra_type: u8, bits: u32) {
        let stored = x87::fld_fstp_copy(bits).to_bits();
        if let Some(entry) = self.get_mut(extra_type) {
            entry.data = ExtraData::Float(stored);
        } else {
            self.add(new_entry(extra_type, ExtraData::Float(stored)));
        }
    }
}

/// IDA: the chunks 004CD600 (CELL, past its own XCLC and XCLL) and 004D9D00 (REFR, ACHR, ACRE) hand to 004259E0, the same 32 in both, read from each function's compare tree. Completion: 100%.
#[must_use]
pub const fn is_extra_chunk(chunk: u32) -> bool {
    matches!(
        chunk,
        XMRC | XLOC
            | XGLB
            | XSED
            | XUSE
            | XLOD
            | XRGD
            | XCHG
            | XRNK
            | XTRG
            | XPCI
            | XMRK
            | XEDL
            | XTEL
            | XSOL
            | XPSL
            | XCCM
            | XRTM
            | XTIM
            | XLCM
            | XPSN
            | XOWN
            | XESP
            | XCLM
            | XCLR
            | XHRS
            | XCMT
            | XACT
            | XHLT
            | XCNT
            | XCWT
            | XCLW
    )
}

/// IDA: 004259E0 BaseExtraList_LoadChunk(file, form): one chunk into its class. XEDL and XCLM, which the LoadForms send here, have no arm. Completion: 100%.
pub fn load_extra_chunk(
    world: &World,
    list: &mut ExtraDataList,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<(), GameError> {
    match cx.file.chunk_type()? {
        // 00425A64 004206A0.
        XMRC => {
            let value = read_dword(cx)?;
            list.set_dword_or_remove_by_type(EXTRA_MERCHANT_CONTAINER, value);
        }
        XLOC => load_lock(list, cx)?,
        // 00425B5E 00423720.
        XGLB => {
            let value = read_dword(cx)?;
            list.set_dword_or_remove_entry(EXTRA_GLOBAL, value);
        }
        XSED => load_leveled_seed(list, cx)?,
        // 00425BF6 0041ED50: the low byte of the dword.
        XUSE => {
            let value = read_dword(cx)?;
            list.store_byte(EXTRA_USES, value as u8);
        }
        // 00425C1B: 0, 0, 00A34F5C = 0.97 under the read; 00420100 copies the three dwords.
        XLOD => {
            let mut bytes = [0_u8; 12];
            bytes[8..].copy_from_slice(&0x3F78_51EC_u32.to_le_bytes());
            cx.file.read_chunk(&mut bytes, 0x0C)?;
            let words = [
                u32::from_le_bytes(bytes[0..4].try_into().expect("four bytes")),
                u32::from_le_bytes(bytes[4..8].try_into().expect("four bytes")),
                u32::from_le_bytes(bytes[8..12].try_into().expect("four bytes")),
            ];
            if let Some(entry) = list.get_mut(EXTRA_LOD) {
                entry.data = ExtraData::Lod(words);
            } else {
                list.add(new_entry(EXTRA_LOD, ExtraData::Lod(words)));
            }
        }
        XRGD => load_rag_doll(list, cx)?,
        // 00425CF1 0041EE90, over 0.0.
        XCHG => {
            let value = read_dword(cx)?;
            list.store_float(EXTRA_CHARGE, value);
        }
        // 00425D3D 004237E0: -1 removes an entry; an entry or a new one takes anything else.
        XRNK => {
            let value = read_dword(cx)?;
            if value == 0xFFFF_FFFF {
                if list.get(EXTRA_RANK).is_some() {
                    list.remove_entry(EXTRA_RANK);
                }
            } else {
                list.store_dword(EXTRA_RANK, value);
            }
        }
        // 00425D62: the FULL that follows is skipped.
        XPCI => {
            cx.file.advance_chunk()?;
        }
        // 00425D6E 00420D40.
        XTRG => {
            let value = read_dword(cx)?;
            list.set_dword_or_remove_by_type(EXTRA_TRESPASS, value);
        }
        XMRK => load_map_marker(list, cx)?,
        XTEL => load_teleport(list, cx)?,
        // 00425E45: only a 1-byte chunk reads; 0041EF30 stores the byte.
        XSOL => {
            if cx.file.chunk_size()? == 1 {
                let mut value = [0_u8];
                cx.file.read_chunk(&mut value, 1)?;
                list.store_byte(EXTRA_SOUL, value[0]);
            }
        }
        XPSL => load_package_start(list, cx)?,
        // 00425EC8 00424380.
        XCCM => {
            let value = read_dword(cx)?;
            list.set_dword_or_remove_entry(EXTRA_CELL_CLIMATE, value);
        }
        // 00425F28 004205C0.
        XRTM => {
            let value = read_dword(cx)?;
            list.set_dword_or_remove_by_type(EXTRA_RANDOM_TELEPORT_MARKER, value);
        }
        // 00425F4D 0041EDF0, over 0.0.
        XTIM => {
            let value = read_dword(cx)?;
            list.store_float(EXTRA_TIME_LEFT, value);
        }
        // 00425F73 00420780.
        XLCM => {
            let value = read_dword(cx)?;
            list.set_dword_or_remove_by_type(EXTRA_LEVEL_CREATURE_MODIFIER, value);
        }
        // 00425F98 0041EFD0.
        XPSN => {
            let value = read_dword(cx)?;
            list.store_dword(EXTRA_PERSUASION, value);
        }
        // 004260FB 00423660.
        XOWN => {
            let value = read_dword(cx)?;
            list.set_dword_or_remove_entry(EXTRA_OWNER, value);
        }
        // 004260C6: 00420280 with the parent, then 00420380 with the flags, on a local cleared first.
        XESP => {
            let mut bytes = [0_u8; 8];
            cx.file.read_chunk(&mut bytes, 8)?;
            let parent = u32::from_le_bytes(bytes[0..4].try_into().expect("four bytes"));
            if parent == 0 {
                list.remove_by_type(EXTRA_ENABLE_STATE_PARENT);
            } else if let Some(entry) = list.get_mut(EXTRA_ENABLE_STATE_PARENT) {
                if let ExtraData::EnableStateParent { parent_0c, .. } = &mut entry.data {
                    *parent_0c = parent;
                }
            } else {
                list.add(new_entry(
                    EXTRA_ENABLE_STATE_PARENT,
                    ExtraData::EnableStateParent {
                        parent_0c: parent,
                        flags_10: 0,
                    },
                ));
            }
            if let Some(entry) = list.get_mut(EXTRA_ENABLE_STATE_PARENT)
                && let ExtraData::EnableStateParent { flags_10, .. } = &mut entry.data
            {
                *flags_10 = bytes[4];
            }
        }
        XCLR => load_region_list(world, list, cx)?,
        // 00426120 00420860.
        XHRS => {
            let value = read_dword(cx)?;
            list.set_dword_or_remove_by_type(EXTRA_HORSE, value);
        }
        // 0042616F: 1 byte, sign extended; 004242C0 tests the dword, stores the byte. A zero removes an entry.
        XCMT => {
            let mut value = [0_u8];
            cx.file.read_chunk(&mut value, 1)?;
            if value[0] == 0 {
                if list.get(EXTRA_CELL_MUSIC_TYPE).is_some() {
                    list.remove_entry(EXTRA_CELL_MUSIC_TYPE);
                }
            } else {
                list.store_byte(EXTRA_CELL_MUSIC_TYPE, value[0]);
            }
        }
        // 00426194: `fild` the dword, `fstp` as a float at 24 bits; 0041ECB0.
        XHLT => {
            let value = read_dword(cx)? as i32;
            list.store_float(EXTRA_HEALTH, (value as f32).to_bits());
        }
        // 004261BC 00423DA0.
        XACT => {
            let value = read_dword(cx)?;
            list.set_action_flags(value);
        }
        // 004261DE 004238A0: a low word of 0 or 1 removes an entry; otherwise the word.
        XCNT => {
            let value = read_dword(cx)? as u16;
            if value <= 1 {
                if list.get(EXTRA_COUNT).is_some() {
                    list.remove_entry(EXTRA_COUNT);
                }
            } else if let Some(entry) = list.get_mut(EXTRA_COUNT) {
                entry.data = ExtraData::Word(value);
            } else {
                list.add(new_entry(EXTRA_COUNT, ExtraData::Word(value)));
            }
        }
        // 00426231 004204E0.
        XCWT => {
            let value = read_dword(cx)?;
            list.set_dword_or_remove_by_type(EXTRA_WATER, value);
        }
        // 0042620E 00423FF0, over 0.0.
        XCLW => {
            let value = read_dword(cx)?;
            list.set_water_height(value);
        }
        _ => {}
    }
    Ok(())
}

// IDA: 00425A89: type 0x31, a new entry (00429A80) over a 0x0C block with +0x00, +0x04 and +0x08 cleared. A 0x0C chunk reads over the block; a 0x10 one reads a local, keeping its byte +0x00, dword +0x04 and byte +0x0C; each ends in `or [+8], 1`. Any other size warns with the file's name and removes the entry. Completion: 100%.
fn load_lock(list: &mut ExtraDataList, cx: &mut FormLoadCx<'_, '_>) -> Result<(), GameError> {
    if list.get(EXTRA_LOCK).is_none() {
        list.add(new_entry(EXTRA_LOCK, ExtraData::Lock(Box::new([0; 12]))));
    }
    let size = cx.file.chunk_size()?;
    let mut lock = match &list.get(EXTRA_LOCK).expect("just added").data {
        ExtraData::Lock(block) => **block,
        _ => [0; 12],
    };
    if size == 0x0C {
        cx.file.read_chunk(&mut lock, size)?;
    } else if size == 0x10 {
        let mut local = [0_u8; 0x10];
        cx.file.read_chunk(&mut local, size)?;
        lock[0] = local[0];
        lock[4..8].copy_from_slice(&local[4..8]);
        lock[8] = local[12];
    } else {
        let name = cx.file.file()?.name_1c.clone();
        cx.file.services.warn(format!(
            "Unrecognized format for lock data in file '{name}'."
        ));
        list.remove_entry(EXTRA_LOCK);
        return Ok(());
    }
    lock[8] |= 1;
    list.get_mut(EXTRA_LOCK).expect("just added").data = ExtraData::Lock(Box::new(lock));
    Ok(())
}

// IDA: 00425B83: a 4-byte chunk reads a dword and keeps its low byte, any other reads 1 byte (the C++ agrees). 00423BD0: 0xFF removes an entry; otherwise an entry or a new one (0042A160) takes the byte. Completion: 100%.
fn load_leveled_seed(
    list: &mut ExtraDataList,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<(), GameError> {
    let value = if cx.file.chunk_size()? == 4 {
        read_dword(cx)? as u8
    } else {
        let mut byte = [0_u8];
        cx.file.read_chunk(&mut byte, 1)?;
        byte[0]
    };
    if value == 0xFF {
        if list.get(EXTRA_LEVELED_SEED).is_some() {
            list.remove_entry(EXTRA_LEVELED_SEED);
        }
    } else {
        list.store_byte(EXTRA_LEVELED_SEED, value);
    }
    Ok(())
}

// IDA: 00425C4E: type 0x19, a new entry (0042A300) over a zero block (00497210). 00497470: count = +0x254 / 0x1C in +0x00, a byte; `div` of the size by that byte (zero faults); a remainder clears the byte and fails, which warns "Failed to load RagDoll Data." and removes the entry; otherwise count * 0x1C bytes for +0x04, into which the whole chunk reads. A second chunk's old block leaks in the original. The C++ keeps the count as a dword. Completion: 100%.
fn load_rag_doll(list: &mut ExtraDataList, cx: &mut FormLoadCx<'_, '_>) -> Result<(), GameError> {
    if list.get(EXTRA_RAG_DOLL).is_none() {
        list.add(new_entry(
            EXTRA_RAG_DOLL,
            ExtraData::RagDoll(Box::default()),
        ));
    }
    let size = cx.file.chunk_size()?;
    let count = (size / 0x1C) as u8;
    if count == 0 {
        return Err(GameError::unclassified(
            0x004974AB,
            "004974AB: a count byte of 0 divides by zero (an XRGD under 0x1C bytes)",
        ));
    }
    if size % u32::from(count) != 0 {
        cx.file
            .services
            .warn("Failed to load RagDoll Data.".to_owned());
        list.remove_entry(EXTRA_RAG_DOLL);
        return Ok(());
    }
    let mut bytes = vec![0_u8; size as usize];
    cx.file.read_chunk(&mut bytes, size)?;
    if let ExtraData::RagDoll(block) = &mut list.get_mut(EXTRA_RAG_DOLL).expect("just added").data {
        block.count_00 = count;
        block.data_04 = Some(bytes);
    }
    Ok(())
}

// IDA: 00425D93: 0041E6D0 the block, or a new one (0042B3F0) stored by 00423590. 0042B200: an advance (a failure returns); an FNAM reads 1 byte to +0x0C and advances (a failure returns); no FULL returns; 0046C7A0 on the block's TESFullName; an advance (a failure returns); a TNAM reads its word to +0x0E (004510F0). Completion: 100%.
fn load_map_marker(list: &mut ExtraDataList, cx: &mut FormLoadCx<'_, '_>) -> Result<(), GameError> {
    if list.get(EXTRA_MAP_MARKER).is_none() {
        list.add(new_entry(
            EXTRA_MAP_MARKER,
            ExtraData::MapMarker(Box::new(MapMarkerData {
                full_name_vtable_00: super::form_components::TES_FULL_NAME_VTABLE,
                ..MapMarkerData::default()
            })),
        ));
    }
    let ExtraData::MapMarker(marker) =
        &mut list.get_mut(EXTRA_MAP_MARKER).expect("just added").data
    else {
        return Ok(());
    };
    if !cx.file.advance_chunk()? {
        return Ok(());
    }
    if cx.file.chunk_type()? == FNAM {
        let mut flags = [marker.flags_0c];
        cx.file.read_chunk(&mut flags, 1)?;
        marker.flags_0c = flags[0];
        if !cx.file.advance_chunk()? {
            return Ok(());
        }
    }
    if cx.file.chunk_type()? != FULL {
        return Ok(());
    }
    load_full_name(&mut marker.name_04, cx)?;
    if !cx.file.advance_chunk()? {
        return Ok(());
    }
    if cx.file.chunk_type()? == TNAM {
        let mut word = marker.marker_type_0e.to_le_bytes();
        cx.file.read_chunk(&mut word, 2)?;
        marker.marker_type_0e = u16::from_le_bytes(word);
    }
    Ok(())
}

// IDA: 00425E77: 0041E6B0 the block, or a new one: 0042B590 stores +0x00 = 0 and six copies of 00A35BE0 (0x7F7FFFCE; the C++ has 0x7F7FFF4B), stored by 004234D0. 0042B670 reads 0x1C bytes into a cleared local and copies all seven dwords over the block. Completion: 100%.
fn load_teleport(list: &mut ExtraDataList, cx: &mut FormLoadCx<'_, '_>) -> Result<(), GameError> {
    const UNSET: u32 = 0x7F7F_FFCE;
    if list.get(EXTRA_TELEPORT).is_none() {
        list.add(new_entry(
            EXTRA_TELEPORT,
            ExtraData::Teleport(Box::new([0, UNSET, UNSET, UNSET, UNSET, UNSET, UNSET])),
        ));
    }
    let mut bytes = [0_u8; 0x1C];
    cx.file.read_chunk(&mut bytes, 0x1C)?;
    let mut words = [0_u32; 7];
    for (word, source) in words.iter_mut().zip(bytes.as_chunks::<4>().0) {
        *word = u32::from_le_bytes(*source);
    }
    if let ExtraData::Teleport(block) = &mut list.get_mut(EXTRA_TELEPORT).expect("just added").data
    {
        **block = words;
    }
    Ok(())
}

// IDA: 00425E01: five dwords cleared, 0x14 read. 0041F4C0(d0, 0, &d1, d4): an entry takes d0 (or the 0 for a null d0) and the three dwords, and keeps its +0x1C; a new one (0042AE70) takes those and d4 through `fld`/`fstp`. No shipped file holds an XPSL (the C++ stops here): C++ and disassembly only. Completion: 100%.
fn load_package_start(
    list: &mut ExtraDataList,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<(), GameError> {
    let mut bytes = [0_u8; 0x14];
    cx.file.read_chunk(&mut bytes, 0x14)?;
    let mut words = [0_u32; 5];
    for (word, source) in words.iter_mut().zip(bytes.as_chunks::<4>().0) {
        *word = u32::from_le_bytes(*source);
    }
    let location = [words[1], words[2], words[3]];
    if let Some(entry) = list.get_mut(EXTRA_PACKAGE_START) {
        if let ExtraData::PackageStart {
            target_0c,
            location_10,
            ..
        } = &mut entry.data
        {
            *target_0c = words[0];
            *location_10 = location;
        }
    } else {
        list.add(new_entry(
            EXTRA_PACKAGE_START,
            ExtraData::PackageStart {
                target_0c: words[0],
                location_10: location,
                time_1c: x87::fld_fstp_copy(words[4]).to_bits(),
            },
        ));
    }
    Ok(())
}

// IDA: 00425FDE: a size with low bits warns with the file's name; otherwise a new list (004A62D0(0)), the IDs read whole, each mapped (0046BB20) and looked up in DataHandler +0xBC's list by +0x0C (004A63E0), a found one prepended unless present (004A6350); 004241E0 stores the list: a nonnull list with an entry deletes the old one, with none adds one (0041D940). Completion: 100%.
/// IDA: 004C9B40(1)'s exterior arm: 0041F9B0's 0x08 extra's +0x0C; none is FormHeap(0x10) 004A62D0(0), vtable A4034C, added by 004241E0. Completion: 100%.
pub fn cell_region_list_or_new(list: &mut ExtraDataList) -> &ExtraRegionList {
    if !matches!(
        list.get(EXTRA_REGION_LIST).map(|entry| &entry.data),
        Some(ExtraData::Regions(_))
    ) {
        list.add(new_entry(
            EXTRA_REGION_LIST,
            ExtraData::Regions(Box::new(ExtraRegionList {
                original_vtable: 0x00A4_034C,
                ..ExtraRegionList::default()
            })),
        ));
    }
    match &list.get(EXTRA_REGION_LIST).expect("just added").data {
        ExtraData::Regions(regions) => regions,
        _ => unreachable!("tested above"),
    }
}

// IDA: the XCLR arm of 004259E0.
fn load_region_list(
    world: &World,
    list: &mut ExtraDataList,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<(), GameError> {
    let size = cx.file.chunk_size()?;
    if size & 3 != 0 {
        let name = cx.file.file()?.name_1c.clone();
        cx.file.services.warn(format!(
            "Invalid Extra Data - Region List in file \"{name}\"."
        ));
        return Ok(());
    }
    let mut regions = ExtraRegionList {
        original_vtable: 0x00A4_034C,
        ..ExtraRegionList::default()
    };
    let mut bytes = vec![0_u8; size as usize];
    cx.file.read_chunk(&mut bytes, size)?;
    let file = cx.file.id;
    let known = &world.resource::<DataHandlerLists>().regions_bc;
    for source in bytes.as_chunks::<4>().0 {
        let raw = u32::from_le_bytes(*source);
        let id = map_file_form_id(world, cx, raw, Some(file))?;
        let found = known.items().find(|&region| {
            world
                .get::<FormIdentity>(region)
                .expect("a form")
                .form_id_0c
                == id
        });
        if let Some(region) = found
            && !regions.regions_04.items().any(|item| item == region)
        {
            regions.regions_04.prepend_non_null(region);
        }
    }
    if let Some(entry) = list.get_mut(EXTRA_REGION_LIST) {
        entry.data = ExtraData::Regions(Box::new(regions));
    } else {
        list.add(new_entry(
            EXTRA_REGION_LIST,
            ExtraData::Regions(Box::new(regions)),
        ));
    }
    Ok(())
}

// IDA: 0041F830 BaseExtraList_TestActionFlags: the type 0x13 byte against the flag, with no entry: the flag against 1. Completion: 100%.
pub fn action_flag(extras: &ExtraDataList, flag: u8) -> bool {
    match extras.get(EXTRA_ACTION).map(|entry| &entry.data) {
        Some(ExtraData::Action { flags_0c, .. }) => flags_0c & flag != 0,
        _ => flag & 1 != 0,
    }
}

/// IDA: 004FBF90 ExtraDataList_SendScriptEvent(form, list, event): no list or no event list (0041E920: the 0x12 extra's
/// +0x10) answers 0; else 004FB580 on it: the first walk takes the entry with the form's ID, else the last null-form one
/// (a null entry ends the walk), and ORs the event in; the second walk ORs it into the first null-form entry too and
/// answers 1. An event list with no +0x08 (no block has been tested yet) takes nothing. The entry's ID is the form's ID
/// here (the exe's form pointer), 0 for none. Completion: 100%.
pub fn extra_list_script_event_or(list: &mut ExtraDataList, id: u32, event: u32) -> bool {
    let Some(entry) = list.get_mut(0x12) else {
        return false;
    };
    let ExtraData::Script {
        events_10: Some(events),
        ..
    } = &mut entry.data
    else {
        return false;
    };
    let Some(entries) = events.events_08.as_mut() else {
        return false;
    };
    let mut answered = false;
    let found = entries
        .iter()
        .position(|entry| entry.id_00 == id)
        .or_else(|| entries.iter().rposition(|entry| entry.id_00 == 0));
    if let Some(index) = found {
        entries[index].mask_04 |= event;
        answered = true;
    }
    if let Some(first) = entries.iter_mut().find(|entry| entry.id_00 == 0) {
        first.mask_04 |= event;
        return true;
    }
    answered
}

// IDA: 0041E7B0, 0041E7D0: the form of an ownership or global extra, resolved at InitItem.
pub(crate) fn extra_form(
    list: &ExtraDataList,
    extra_type: u8,
) -> Result<Option<Entity>, GameError> {
    let Some(entry) = list.get(extra_type) else {
        return Ok(None);
    };
    match (entry.resolved_0c, &entry.data) {
        (Some(form), _) => Ok(form),
        (None, ExtraData::Dword(0)) => Ok(None),
        _ => Err(GameError::unclassified(
            0x0041E7B0,
            "0041E7B0: an unresolved form extra",
        )),
    }
}

// IDA: 0041E860 BaseExtraList_GetStackCountOrOne: ExtraCount's signed word, else 1.
pub(crate) fn stack_count(list: &ExtraDataList) -> i32 {
    match list.get(EXTRA_COUNT).map(|entry| &entry.data) {
        Some(ExtraData::Word(count)) => i32::from(*count as i16),
        _ => 1,
    }
}

// IDA: 004238A0 ExtraCount(count) on a list: a count of 0 or 1 removes it; an entry takes it; otherwise a new one.
pub(crate) fn set_extra_count(list: &mut ExtraDataList, count: u16) {
    if count < 2 {
        if list.get(EXTRA_COUNT).is_some() {
            list.remove_entry(EXTRA_COUNT);
        }
        return;
    }
    if let Some(entry) = list.get_mut(EXTRA_COUNT) {
        entry.data = ExtraData::Word(count);
        return;
    }
    list.add(new_extra_entry(EXTRA_COUNT, ExtraData::Word(count)));
}

// IDA: 0041E4D0 ExtraDataList_CompareListForContainer(other): 1 when an extra of `other` differs: types 0x12 and 0x27 do always, 0x2A never; a type absent here does; two 0x36 entries never do. Another type's compare stops here: this slice makes no such stack.
pub(crate) fn lists_differ(this: &ExtraDataList, other: &ExtraDataList) -> Result<bool, GameError> {
    for entry in &other.entries {
        match entry.type_04 {
            0x12 | 0x27 => return Ok(true),
            EXTRA_COUNT => {}
            // 00429140 ExtraLeveledItem's vt+4 is 0041DF70, which compares only the type byte, so two of them never differ and a second pick of a form merges whatever its index (the C++ compares the payload).
            EXTRA_LEVELED_ITEM => {
                if this.get(EXTRA_LEVELED_ITEM).is_none() {
                    return Ok(true);
                }
            }
            _ => {
                if this.get(entry.type_04).is_none() {
                    return Ok(true);
                }
                return Err(GameError::unclassified(
                    0x0041E52F,
                    "0041E52F: comparing an extra other than 0x36",
                ));
            }
        }
    }
    Ok(false)
}

// IDA: 0041F090, 004FBDC0 and 0041F130 on a stack, then the run.
pub(crate) fn attach_and_run(
    world: &mut World,
    list: &mut ExtraDataList,
    script: Entity,
) -> Result<(), GameError> {
    let events = create_script_event_list(world, script);
    list.add(new_extra_entry(
        0x12,
        ExtraData::Script {
            script_0c: script,
            events_10: Some(events),
        },
    ));
    run_with_temporary(world, script, list)
}

// IDA: 0041FD70 ExtraDataList_IsExtraDefaultforContainer(strict): every extra of the list is 0x12, 0x22, 0x26, 0x27, 0x2A,
// 0x2D, 0x36, 0x37 or 0x55, and with `strict` also 0x1B; an empty list is.
pub(crate) fn default_for_container(list: &ExtraDataList, strict: bool) -> bool {
    list.entries.iter().all(|entry| {
        matches!(
            entry.type_04,
            0x12 | 0x22 | 0x26 | 0x27 | 0x2A | 0x2D | 0x36 | 0x37 | 0x55
        ) || (strict && entry.type_04 == 0x1B)
    })
}

// IDA: 0041FCB0: an ownership (0x27) among only the default extras above (without 0x1B); any other extra answers false.
pub(crate) fn owned_default(list: &ExtraDataList) -> bool {
    let mut owned = false;
    for entry in &list.entries {
        match entry.type_04 {
            0x12 | 0x22 | 0x26 | 0x2A | 0x2D | 0x36 | 0x37 | 0x55 => {}
            0x27 => owned = true,
            _ => return false,
        }
    }
    owned
}

// IDA: 0041DF10(left): the presence bit of 0x1C, or of 0x1B or 0x1C (byte +0x0B, 0x10 / 0x18).
pub(crate) fn worn(list: &ExtraDataList, left_only: bool) -> bool {
    list.present(EXTRA_WORN_LEFT) || (!left_only && list.present(EXTRA_WORN))
}

// IDA: 0041DEF0: the presence bit of 0x36 (byte +0x0E, 0x40).
pub(crate) fn leveled(list: &ExtraDataList) -> bool {
    list.present(EXTRA_LEVELED_ITEM)
}

// IDA: 0042B5F0 (teleport data, door, value): the owned cells of the linked door, and the door.
pub(crate) fn mark_teleport_cells(
    world: &mut World,
    linked: Option<Entity>,
    door: Entity,
    value: bool,
) -> Result<(), GameError> {
    if let Some(cell) = linked.and_then(|linked| parent_cell(world, linked))
        && extra_form(cell_list(world, cell), EXTRA_OWNER)?.is_some()
    {
        cell_set_flag_40(world, cell, value)?;
    }
    if let Some(cell) = parent_cell(world, door)
        && extra_form(cell_list(world, cell), EXTRA_OWNER)?.is_some()
    {
        cell_set_flag_40(world, cell, value)?;
    }
    Ok(())
}

/// IDA: 0041E6B0 the 0x32 extra's door, with 0x32 holding the linked reference.
#[must_use]
pub fn teleport_door(world: &World, reference: Entity) -> Option<Entity> {
    let entry = world
        .get::<TesObjectRefr>(reference)?
        .extra_data_44
        .get(EXTRA_TELEPORT)?;
    entry.resolved_0c.flatten()
}

// IDA: 0041E9B0's entry.
pub(crate) fn package_start_entry(world: &World, reference: Entity) -> Option<&BaseExtraEntry> {
    world
        .get::<TesObjectRefr>(reference)?
        .extra_data_44
        .get(EXTRA_PACKAGE_START)
}

// IDA: 004200C0: the 0x21 extra's list has the package: a null item ends the walk.
pub(crate) fn run_once_contains(world: &World, actor: Entity, package: Entity) -> bool {
    let list = &world
        .get::<TesObjectRefr>(actor)
        .expect("an actor")
        .extra_data_44;
    match list.get(EXTRA_RUN_ONCE_PACKS).map(|entry| &entry.data) {
        Some(ExtraData::RunOncePacks(items)) => items.iter().any(|item| item.0 == package),
        _ => false,
    }
}

// IDA: 0041F4C0 the 0x1E extra: a new 0x20 one (0042AE70: +0x0C the world or else the cell, the position, rotZ), or on an existing one +0x0C and the position alone.
pub(crate) fn set_package_start_location(
    world: &mut World,
    reference: Entity,
    space: Option<Entity>,
    cell: Option<Entity>,
    position: [f32; 3],
    rotation_z: f32,
) {
    let location = space.or(cell);
    let bits = position.map(f32::to_bits);
    let list = &mut world
        .get_mut::<TesObjectRefr>(reference)
        .expect("a reference")
        .into_inner()
        .extra_data_44;
    if let Some(entry) = list.get_mut(EXTRA_PACKAGE_START) {
        let time = match entry.data {
            ExtraData::PackageStart { time_1c, .. } => time_1c,
            _ => 0,
        };
        entry.data = ExtraData::PackageStart {
            target_0c: 0,
            location_10: bits,
            time_1c: time,
        };
        entry.resolved_0c = Some(location);
        return;
    }
    let mut entry = new_extra_entry(
        EXTRA_PACKAGE_START,
        ExtraData::PackageStart {
            target_0c: 0,
            location_10: bits,
            time_1c: rotation_z.to_bits(),
        },
    );
    entry.resolved_0c = Some(location);
    list.add(entry);
}

// IDA: 0041FFC0, 0042A440: the 0x21 extra when missing; an entry takes the day; otherwise prepended (00446CB0), then the package and the day byte.
pub(crate) fn run_once_add(world: &mut World, actor: Entity, package: Entity, day: u8) {
    let list = &mut world
        .get_mut::<TesObjectRefr>(actor)
        .expect("an actor")
        .into_inner()
        .extra_data_44;
    if list.get(EXTRA_RUN_ONCE_PACKS).is_none() {
        list.add(new_extra_entry(
            EXTRA_RUN_ONCE_PACKS,
            ExtraData::RunOncePacks(Vec::new()),
        ));
    }
    let entry = list.get_mut(EXTRA_RUN_ONCE_PACKS).expect("just added");
    if let ExtraData::RunOncePacks(items) = &mut entry.data {
        if let Some(item) = items.iter_mut().find(|item| item.0 == package) {
            item.1 = day;
            return;
        }
        items.insert(0, (package, day));
    }
}

// IDA: 00424C50 the 0x23 extra when missing (00429B20); a follower not listed on any node is prepended (the C++ stops looking at a null item).
pub(crate) fn follower_add(world: &mut World, list_owner: Entity, follower: Entity) {
    let list = &mut world
        .get_mut::<TesObjectRefr>(list_owner)
        .expect("a reference")
        .into_inner()
        .extra_data_44;
    if list.get(EXTRA_FOLLOWER).is_none() {
        list.add(new_extra_entry(
            EXTRA_FOLLOWER,
            ExtraData::Followers(Vec::new()),
        ));
    }
    if let ExtraData::Followers(items) = &mut list.get_mut(EXTRA_FOLLOWER).expect("just added").data
        && !items.contains(&follower)
    {
        items.insert(0, follower);
    }
}

// IDA: 00421A00 and 00421340's types that write nothing with flags 4: 0x1A, the player's only extra at 0040F050.
const EXTRA_WRITES_NOTHING: u8 = 0x1A;

// IDA: 00421340 and 00421A00 on TESObjectREFR +0x44, with flags 4: the u16 count and nothing for type 0x1A. Each other type is an arm no run reached.
pub(crate) fn extras_write_nothing(world: &World, player: Entity) -> Result<(), GameError> {
    let refr = world.get::<TesObjectRefr>(player).expect("a reference");
    for entry in &refr.extra_data_44.entries {
        if entry.type_04 != EXTRA_WRITES_NOTHING {
            return Err(GameError::unclassified(
                0x00421A00,
                "00421A00: an extra other than type 0x1A",
            ));
        }
    }
    Ok(())
}

// IDA: `0041F950` the type 2 extra's +0x10 (0 with none). Completion: 100%.
pub(crate) fn cell_mopp(world: &World, cell: Entity) -> Option<Entity> {
    match world
        .get::<TesObjectCell>(cell)?
        .extra_28
        .get(EXTRA_HAVOK)
        .map(|entry| &entry.data)
    {
        Some(ExtraData::Havok { cell_mopp_10, .. }) => *cell_mopp_10,
        _ => None,
    }
}

// IDA: `004240C0`(world) and `0041F890`(mopp): the type 2 extra's +0x0C / +0x10 NiPointer store, a new ExtraHavok (`0041DAC0`,
// FormHeap 0x14, both null) added (`0041E0A0`) when the cell has none. A null world removes the extra (00422E20): not reached.
// Completion: 100% of the nonnull stores.
pub(crate) fn set_extra_havok(
    world: &mut World,
    cell: Entity,
    new_world: Option<Entity>,
    new_mopp: Option<Entity>,
) {
    let mut data = world.get_mut::<TesObjectCell>(cell).expect("a cell");
    if data.extra_28.get(EXTRA_HAVOK).is_none() {
        data.extra_28.add(new_extra_entry(
            EXTRA_HAVOK,
            ExtraData::Havok {
                world_0c: None,
                cell_mopp_10: None,
            },
        ));
    }
    if let Some(ExtraData::Havok {
        world_0c,
        cell_mopp_10,
    }) = data
        .extra_28
        .get_mut(EXTRA_HAVOK)
        .map(|entry| &mut entry.data)
    {
        if new_world.is_some() {
            *world_0c = new_world;
        }
        if new_mopp.is_some() {
            *cell_mopp_10 = new_mopp;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dword_entry(extra_type: u8) -> BaseExtraEntry {
        new_entry(extra_type, ExtraData::Dword(1))
    }

    // 0041E0A0: bytes 0041E138 over 0041E130 give the front types.
    #[test]
    fn the_front_types_go_first_and_set_their_bit() {
        let mut list = ExtraDataList::default();
        list.add(dword_entry(EXTRA_OWNER));
        list.add(dword_entry(EXTRA_PERSISTENT_CELL));
        list.add(dword_entry(EXTRA_GLOBAL));
        list.add(dword_entry(EXTRA_TELEPORT));
        let order: Vec<u8> = list.entries.iter().map(|entry| entry.type_04).collect();
        assert_eq!(order, [0x32, 0x11, 0x27, 0x28]);
        assert_eq!(list.presence_08[2], 0x02, "0x11");
        assert_eq!(list.presence_08[4], 0x80, "0x27");
        assert_eq!(list.presence_08[5], 0x01, "0x28");
        assert_eq!(list.presence_08[6], 0x04, "0x32");
        assert_eq!(list.head_04(), 1);
    }

    // 0041E210 tests the bit first, so an entry whose bit is clear isn't found.
    #[test]
    fn get_needs_the_presence_bit() {
        let mut list = ExtraDataList::default();
        list.add(dword_entry(EXTRA_OWNER));
        list.presence_08[4] = 0;
        assert!(list.get(EXTRA_OWNER).is_none());
    }

    // 0041E160 clears the bit with or without an entry; 0041DFF0 clears all of them.
    #[test]
    fn removal_clears_the_bits() {
        let mut list = ExtraDataList::default();
        list.presence_08[4] = 0x80;
        list.remove_by_type(EXTRA_OWNER);
        assert_eq!(list.presence_08[4], 0);
        list.add(dword_entry(EXTRA_OWNER));
        list.remove_all();
        assert_eq!(list.presence_08, [0; 12]);
        assert_eq!(list.head_04(), 0);
    }

    // 0041E450 has no presence store, where the C++ clears each bit.
    #[test]
    fn the_cell_reset_leaves_stale_bits() {
        let mut list = ExtraDataList::default();
        list.add(dword_entry(EXTRA_OWNER));
        list.add(dword_entry(EXTRA_REGION_LIST));
        list.add(dword_entry(2));
        list.remove_for_cell_reset(false);
        assert_eq!(list.entries.len(), 1);
        assert_eq!(list.entries[0].type_04, EXTRA_REGION_LIST);
        assert_eq!(list.presence_08[4], 0x80, "0x27 keeps its bit");
        assert_eq!(list.presence_08[0], 0x04, "2 keeps its bit");
        assert!(list.get(EXTRA_OWNER).is_none(), "the walk finds nothing");
        let mut list = ExtraDataList::default();
        list.add(dword_entry(2));
        list.add(dword_entry(3));
        list.add(dword_entry(4));
        list.remove_for_cell_reset(true);
        let kept: Vec<u8> = list.entries.iter().map(|entry| entry.type_04).collect();
        assert_eq!(kept, [2, 3], "g_saveLoadGame +0x18 bit 2 keeps 2 and 3");
    }

    // 00423FF0's `fucom`: +0 and -0 remove, a NaN stores, quieted by `fld`.
    #[test]
    fn the_water_height_arms() {
        let mut list = ExtraDataList::default();
        list.set_water_height(1.5_f32.to_bits());
        assert_eq!(
            list.get(EXTRA_WATER_HEIGHT).unwrap().data,
            ExtraData::Float(1.5_f32.to_bits())
        );
        list.set_water_height(0x8000_0000);
        assert!(list.get(EXTRA_WATER_HEIGHT).is_none());
        list.set_water_height(0);
        assert!(list.entries.is_empty());
        list.set_water_height(0x7F80_0001);
        assert_eq!(
            list.get(EXTRA_WATER_HEIGHT).unwrap().data,
            ExtraData::Float(0x7FC0_0001),
            "an sNaN quiets"
        );
    }

    // 00423DA0 and 00423DF0: flags of 1 with no entry add nothing; an entry with a null +0x10 goes.
    #[test]
    fn the_action_flag_arms() {
        let mut list = ExtraDataList::default();
        list.set_action_flags(1);
        assert!(list.entries.is_empty());
        list.or_action_flags(8);
        assert_eq!(
            list.get(EXTRA_ACTION).unwrap().data,
            ExtraData::Action {
                flags_0c: 9,
                ref_10: 0
            },
            "the absent byte reads as 1"
        );
        list.or_action_flags(2);
        assert_eq!(
            list.get(EXTRA_ACTION).unwrap().data,
            ExtraData::Action {
                flags_0c: 11,
                ref_10: 0
            }
        );
        list.set_action_flags(1);
        assert!(list.get(EXTRA_ACTION).is_none());
        list.set_action_flags(5);
        if let ExtraData::Action { ref_10, .. } = &mut list.get_mut(EXTRA_ACTION).unwrap().data {
            *ref_10 = 7;
        }
        list.set_action_flags(1);
        assert_eq!(
            list.get(EXTRA_ACTION).unwrap().data,
            ExtraData::Action {
                flags_0c: 1,
                ref_10: 7
            },
            "a set +0x10 keeps the entry"
        );
    }

    #[test]
    fn the_persistent_cell_arms() {
        let mut world = World::new();
        let cell = world.spawn_empty().id();
        let other = world.spawn_empty().id();
        let mut list = ExtraDataList::default();
        list.set_persistent_cell(None);
        assert!(list.entries.is_empty(), "null with no entry adds nothing");
        list.set_persistent_cell(Some(cell));
        assert_eq!(list.persistent_cell(), Some(cell));
        list.set_persistent_cell(Some(other));
        assert_eq!(list.persistent_cell(), Some(other));
        assert_eq!(list.entries.len(), 1);
        list.set_persistent_cell(None);
        assert_eq!(list.persistent_cell(), None);
        assert_eq!(list.presence_08[2], 0);
    }
}
