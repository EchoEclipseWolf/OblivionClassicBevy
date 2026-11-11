//! DataHandler's form indexes, which carry the original's order (Bevy storage never does), and Phase 09 (D4): the DataHandler TES_Constructor builds (FormHeap(0xCE0)
//! 00446D80 -> 00B33A98) with its 21 TESSkill forms registered as form IDs 0x3D..0x51 in g_formsByID (00B0613C), the region list, the idle-animation root map (00B362C0)
//! and the plugin file list 0044A2B0 (`tes_file`).
//!
//! The first TESForm of the process runs the form-type table validation (0046C486..0046C550, 00B33C10); the CRT static forms carried from Phase 00 ran it before WinMain
//! (P09-RUN-001: 00B33C10 = 1 at the window), so `DataHandlerGlobals` starts with it done, and a test can clear it.
//! Evidence: evidence/phase09-window-2026-10-06/disasm-09d.txt, disasm-09f.txt (0052EAF0, 0046A400), disasm-09j/k.txt (0046B090 components), P09-RUN-001.

use bevy_ecs::prelude::*;
use oblivion_core::pointer_list::PointerList;

use super::tes_file::{
    FormTypeLookupCache, TesFileId, TesFileServices, TesFileStore, build_tes_file_list,
};
use crate::bs::string::BsStringT;
use crate::tes::data_load::DataLoadServices;
use crate::tes::default_forms::DefaultFormStatics;
use crate::tes::form::{FormIdentity, set_form_id, set_form_id_release};
use crate::tes::form_components::{
    FormRef, TesActorBaseData, TesFullName, TesObjectLinks, TesReaction, TesReactionForm,
    append_tes_object,
};
use crate::tes::form_hierarchy::RttiClass;
use crate::tes::forms_actors::{FORM_TYPE_NPC, FORM_TYPE_RACE, construct_npc, construct_race};
use crate::tes::forms_dialogue::create_default_topics;
use crate::tes::forms_environment::{
    FORM_TYPE_WEATHER, TesClimate, TesWaterForm, TesWeather, WeatherEntry, construct_climate,
    construct_water, construct_weather, create_default_effect_shaders, make_default_weather,
};
use crate::tes::forms_init::{auto_calc, cast, casts_to, init_item_of, lookup};
use crate::tes::forms_init_world::cell_run_reference_scripts;
use crate::tes::forms_items::{
    FOOTSTEP_SOUNDS, MAGIC_SOUNDS, construct_default, default_clothing, default_object,
    default_sound,
};
use crate::tes::forms_magic::create_default_spells;
use crate::tes::forms_misc::{
    TesFaction, TesFactionRank, TesSkill, clear_altered, construct_skill, set_initialized,
};
use crate::tes::forms_world::{TesWorldSpace, construct_world};
use crate::tes::global::{EditorId, TESGlobal, construct_global};
use crate::tes::sky::sky_set_climate_default;
use crate::tes::tes::GameThreadLocals;
use crate::tes::tes_file::close_tes_file;
use crate::tes::tes_grid::TesGridServices;
use oblivion_core::error::GameError;

/// IDA: 00B0613C g_formsByID hash size.
pub const FORMS_BY_ID_BUCKETS: usize = 0x2008D;

/// IDA: 00B0613C g_formsByID: 00452570 SetAt: key % size; a found key takes the new value, a new
/// entry goes to the bucket head. `Entity` values never leave this index.
/// Completion: 90%.
/// TODO: [ ] 00446B40's ID release on a form-ID change (M3, with `allocate_form_id`).
#[derive(Resource)]
pub struct FormsById {
    buckets: Vec<Vec<(u32, Entity)>>,
    count: u32,
}

impl Default for FormsById {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            buckets: vec![Vec::new(); FORMS_BY_ID_BUCKETS],
            count: 0,
        }
    }
}

impl FormsById {
    /// IDA: 00452570.
    pub fn set_at(&mut self, key: u32, value: Entity) {
        let chain = &mut self.buckets[key as usize % FORMS_BY_ID_BUCKETS];
        if let Some(entry) = chain.iter_mut().find(|entry| entry.0 == key) {
            entry.1 = value;
        } else {
            chain.insert(0, (key, value));
            self.count += 1;
        }
    }

    /// IDA: the g_formsByID lookup.
    #[must_use]
    pub fn lookup(&self, key: u32) -> Option<Entity> {
        self.buckets[key as usize % FORMS_BY_ID_BUCKETS]
            .iter()
            .find(|entry| entry.0 == key)
            .map(|entry| entry.1)
    }

    /// IDA: the map's RemoveAt.
    pub fn remove(&mut self, key: u32) {
        let chain = &mut self.buckets[key as usize % FORMS_BY_ID_BUCKETS];
        if let Some(index) = chain.iter().position(|entry| entry.0 == key) {
            chain.remove(index);
            self.count -= 1;
        }
    }

    /// IDA: +0x0C the entry count.
    #[must_use]
    pub const fn count(&self) -> u32 {
        self.count
    }

    /// IDA: N/A the chain of one bucket, head first.
    #[must_use]
    pub fn bucket(&self, index: usize) -> &[(u32, Entity)] {
        &self.buckets[index]
    }

    /// IDA: N/A every entry, bucket order then chain order, as P12's walk read the original.
    pub fn entries(&self) -> impl Iterator<Item = (u32, Entity)> + '_ {
        self.buckets.iter().flatten().copied()
    }
}

/// IDA: 00448D40 DataHandler_AllocateFormID (__thiscall on the DataHandler): remember +0x8C0; advance it ((v & 0xFFFFFF) == 0xFFFFFF ? (v & 0xFF000000) + 0x800 : v + 1) past every key present in g_formsByID (vt+4
/// HashKey, vt+8 KeysEqual walk) or in the g_saveLoadGame +0x00 map (00453450); return the remembered value unless one of the two maps holds it, in which case the current counter becomes the remembered value and
/// the loop repeats. `changes` is the 00453450 map, `TesSaveLoadGame::changes_00`, read through a null g_saveLoadGame as no key since the callers' tests build no TES; the original faults there. `save_load_form_ids` adds keys for a test. Completion: 100%.
pub fn allocate_form_id(
    next_form_id_8c0: &mut u32,
    forms: &FormsById,
    save_load_form_ids: &[u32],
    changes: Option<&crate::tes::save_load_game::ChangeChains>,
) -> u32 {
    let taken = |key: u32| {
        forms.lookup(key).is_some()
            || save_load_form_ids.contains(&key)
            || changes.is_some_and(|map| map.get(key).is_some())
    };
    loop {
        // 00448D50 ebp = +0x8C0 (the value handed out).
        let remembered = *next_form_id_8c0;
        // 00448D60..00448DE2 advance past keys present in either map.
        loop {
            let value = *next_form_id_8c0;
            *next_form_id_8c0 = if value & 0x00FF_FFFF == 0x00FF_FFFF {
                (value & 0xFF00_0000).wrapping_add(0x800)
            } else {
                value.wrapping_add(1)
            };
            if !taken(*next_form_id_8c0) {
                break;
            }
        }
        // 00448DE8..00448E0E the remembered value must itself be free.
        if !taken(remembered) {
            return remembered;
        }
    }
}

/// IDA: 00446B40 DataHandler_ReleaseFormID(refID) (__thiscall, retn 4): a nonzero ID with high byte 0xFF that equals +0x8C0 - 1 and whose low 24 bits exceed 0x7FF becomes +0x8C0 again; returns the ID. A null
/// DataHandler faults only when the high byte test passes (host stop). Completion: 100%.
pub fn release_form_id(
    data_handler: Option<&mut DataHandler>,
    form_id: u32,
) -> Result<u32, GameError> {
    // 00446B44..00446B56 a nonzero ID with high byte 0xFF.
    if form_id == 0 || form_id & 0xFF00_0000 != 0xFF00_0000 {
        return Ok(form_id);
    }
    // 00446B58 [ecx+0x8C0]: a null DataHandler faults here.
    let Some(data_handler) = data_handler else {
        return Err(GameError::unclassified(
            0x00446B58,
            "00446B58: 00446B40 reads +0x8C0 of a null DataHandler",
        ));
    };
    if form_id == data_handler.next_form_id_8c0().wrapping_sub(1) && form_id & 0x00FF_FFFF > 0x7FF {
        *data_handler.next_form_id_8c0_mut() = form_id;
    }
    Ok(form_id)
}

/// IDA: DataHandler +0x00 TESObjectList (004B23A0, 0x10): +0x00 count, +0x04 first, +0x08 last. The objects' own links are `TesObjectLinks`. Completion: 100%.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TesObjectList {
    pub count_00: u32,
    pub first_04: Option<Entity>,
    pub last_08: Option<Entity>,
}

/// IDA: DataHandler's form lists, by original offset, each a BSSimpleList that 0044D950 prepends to (00446CB0). `regions_bc` is the list inside the +0xBC TESRegionList, at its +0x04. Completion: 100% of the heads; each fills with its family.
#[derive(Resource, Default)]
pub struct DataHandlerLists {
    pub objects_00: TesObjectList,
    pub packages_04: PointerList<Entity>,
    pub world_spaces_0c: PointerList<Entity>,
    pub climates_14: PointerList<Entity>,
    pub weathers_1c: PointerList<Entity>,
    pub enchantments_24: PointerList<Entity>,
    pub spells_2c: PointerList<Entity>,
    pub hairs_34: PointerList<Entity>,
    pub eyes_3c: PointerList<Entity>,
    pub races_44: PointerList<Entity>,
    pub land_textures_4c: PointerList<Entity>,
    pub classes_54: PointerList<Entity>,
    pub factions_5c: PointerList<Entity>,
    pub scripts_64: PointerList<Entity>,
    pub sounds_6c: PointerList<Entity>,
    pub globals_74: PointerList<Entity>,
    pub topics_7c: PointerList<Entity>,
    pub quests_84: PointerList<Entity>,
    pub birth_signs_8c: PointerList<Entity>,
    pub combat_styles_94: PointerList<Entity>,
    pub load_screens_9c: PointerList<Entity>,
    pub water_forms_a4: PointerList<Entity>,
    pub effect_shaders_ac: PointerList<Entity>,
    pub object_animations_b4: PointerList<Entity>,
    pub regions_bc: PointerList<Entity>,
    /// +0x8B8 (MGEF and the REFRs, 12b and 12c).
    pub list_8b8: PointerList<Entity>,
}

impl DataHandlerLists {
    /// IDA: N/A a list by its original offset, for a row printer. `None` where no list sits.
    #[must_use]
    pub const fn list_at(&self, offset: u32) -> Option<&PointerList<Entity>> {
        Some(match offset {
            0x04 => &self.packages_04,
            0x0C => &self.world_spaces_0c,
            0x14 => &self.climates_14,
            0x1C => &self.weathers_1c,
            0x24 => &self.enchantments_24,
            0x2C => &self.spells_2c,
            0x34 => &self.hairs_34,
            0x3C => &self.eyes_3c,
            0x44 => &self.races_44,
            0x4C => &self.land_textures_4c,
            0x54 => &self.classes_54,
            0x5C => &self.factions_5c,
            0x64 => &self.scripts_64,
            0x6C => &self.sounds_6c,
            0x74 => &self.globals_74,
            0x7C => &self.topics_7c,
            0x84 => &self.quests_84,
            0x8C => &self.birth_signs_8c,
            0x94 => &self.combat_styles_94,
            0x9C => &self.load_screens_9c,
            0xA4 => &self.water_forms_a4,
            0xAC => &self.effect_shaders_ac,
            0xB4 => &self.object_animations_b4,
            0xBC => &self.regions_bc,
            0x8B8 => &self.list_8b8,
            _ => return None,
        })
    }

    /// IDA: 0044D950's jump table (44DCA8 over 44DC44): the list of a form type, by the byte table read from the exe. 0x31 and types with no list go elsewhere in that function.
    #[must_use]
    pub const fn list_for_type(&mut self, form_type: u8) -> Option<&mut PointerList<Entity>> {
        Some(match form_type {
            0x04 => &mut self.globals_74,
            0x05 => &mut self.classes_54,
            0x06 => &mut self.factions_5c,
            0x07 => &mut self.hairs_34,
            0x08 => &mut self.eyes_3c,
            0x09 => &mut self.races_44,
            0x0A => &mut self.sounds_6c,
            0x0D => &mut self.scripts_64,
            0x0E => &mut self.land_textures_4c,
            0x0F => &mut self.enchantments_24,
            0x10 => &mut self.spells_2c,
            0x11 => &mut self.birth_signs_8c,
            0x2D => &mut self.weathers_1c,
            0x2E => &mut self.climates_14,
            0x2F => &mut self.regions_bc,
            0x35 => &mut self.world_spaces_0c,
            0x3B => &mut self.quests_84,
            0x3D => &mut self.packages_04,
            0x3E => &mut self.combat_styles_94,
            0x3F => &mut self.load_screens_9c,
            0x41 => &mut self.object_animations_b4,
            0x42 => &mut self.water_forms_a4,
            0x43 => &mut self.effect_shaders_ac,
            _ => return None,
        })
    }
}

/// IDA: 00B06164's hash size (00B06168, 0xFA1 in the image).
pub const EDITOR_ID_BUCKETS: u32 = 0xFA1;

/// IDA: one 0x0C NiTMapItem of 00B06164: +0x00 next (chain order is the `Vec`'s), +0x04 the key, +0x08 the form. The key is a copy where the original holds the form's own editor-ID pointer (+0x10 is 0, so 006C4F80 stores the pointer), and empty for a freed block's other contents, which compare unequal: a later SetEditorID doesn't reach it here, and that is a host
/// substitution no reader can see while nothing walks the map after one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorIdEntry {
    pub key: Vec<u8>,
    pub form: Entity,
}

/// IDA: 00B06164 g_formsByEditorID, BSTCaseInsensitiveStringMap<TESForm*> (cinit 009DBF70: 0xFA1 buckets cleared, +0x10 byte 00B06174 = 0, vtable A3BF1C). Hash, comparison and chain order are the original's. Completion: 100%.
#[derive(Resource)]
pub struct EditorIdMap {
    buckets: Vec<Vec<EditorIdEntry>>,
    count: u32,
    freed_cell_key: Option<FreedCellKey>,
}

/// IDA: what 0044DE09 and 0044DE12 free of a cell's name, for `EditorIdMap::insert_after_cell_override`. `freed_after` holds the sizes of the blocks freed after the key's: its own 0x14 entry, each later extra, the +0x3C block and FULL's string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FreedCellKey {
    pub form: Entity,
    pub key: Vec<u8>,
    pub freed_after: Vec<u32>,
}

/// IDA: FormHeap's pool for a request: up to a multiple of 4, at least 8. Completion: 100% of the sizes these strings take.
#[must_use]
pub const fn form_heap_class(size: u32) -> u32 {
    let rounded = (size + 3) & !3;
    if rounded < 8 { 8 } else { rounded }
}

impl Default for EditorIdMap {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            buckets: vec![Vec::new(); EDITOR_ID_BUCKETS as usize],
            count: 0,
            freed_cell_key: None,
        }
    }
}

/// IDA: 004A80A0 vt+4, before its modulo: tolower each byte into a copy, then h = 33 * h + (signed char) c up to the NUL. Completion: 100%.
#[must_use]
pub fn case_insensitive_hash(key: &[u8]) -> u32 {
    let mut hash = 0_u32;
    for &byte in super::tes_file::c_bytes(key) {
        let lowered = oblivion_core::crt::c_tolower(i32::from(byte as i8)) as i8;
        hash = hash
            .wrapping_shl(5)
            .wrapping_add(hash)
            .wrapping_add(i32::from(lowered) as u32);
    }
    hash
}

impl EditorIdMap {
    /// IDA: N/A every entry, bucket by bucket, each chain from its head, for a report.
    pub fn walk(&self) -> impl Iterator<Item = &EditorIdEntry> + '_ {
        self.buckets.iter().flatten()
    }

    /// IDA: 004A80A0 vt+4: `case_insensitive_hash` % +0x04 (`div`, unsigned). Completion: 100%.
    #[must_use]
    pub fn hash(key: &[u8]) -> u32 {
        case_insensitive_hash(key) % EDITOR_ID_BUCKETS
    }

    /// IDA: 00412D30 NiTPointerMap_InsertOrAssign with 00584D10 (_stricmp == 0): a hit stores the key (+0x10 is 0) and the value; a miss goes to the bucket head and counts. Completion: 100%.
    pub fn insert_or_assign(&mut self, key: &[u8], form: Entity) {
        let chain = &mut self.buckets[Self::hash(key) as usize];
        if let Some(entry) = chain
            .iter_mut()
            .find(|entry| oblivion_core::crt::stricmp(key, &entry.key) == 0)
        {
            entry.key = key.to_vec();
            entry.form = form;
            return;
        }
        chain.insert(
            0,
            EditorIdEntry {
                key: key.to_vec(),
                form,
            },
        );
        self.count += 1;
    }

    /// IDA: N/A keeps a cell's name as 0044DE09 frees it, for the insert that follows the record's load.
    pub fn note_freed_cell_key(&mut self, freed: FreedCellKey) {
        self.freed_cell_key = Some(freed);
    }

    /// IDA: 0044F2D9 after a cell's override. The node keeps the freed key's pointer, and 00412D30 reads whatever that block holds by the time the record has loaded, so the text decides a hit. FormHeap's pools are LIFO: the blocks of the key's size freed after it go first, to the record's allocations in order: the new 0x14 entry, the name (004245E0), FULL. If the name lands there the node holds it, as it does if FULL does; anything else leaves the free list's words or other data, which compares unequal. `run-c1-log`, 2026-11-06, all 84 overrides: 31 grow, and `Nagastani` hits on the block FULL took. Host substitution: only this one record's blocks are modelled, so a block a later record reuses keeps the text given here, which no compare showed. Completion: 100% of the 84.
    pub fn insert_after_cell_override(&mut self, key: &[u8], full: &[u8], form: Entity) {
        if let Some(freed) = self
            .freed_cell_key
            .take()
            .filter(|freed| freed.form == form)
        {
            let size = form_heap_class(freed.key.len() as u32 + 1);
            let mut covering = freed
                .freed_after
                .iter()
                .filter(|&&freed_size| form_heap_class(freed_size) == size)
                .count();
            // The name's entry, the name and FULL, each with a block only when it has text.
            let pops: [(u32, Option<&[u8]>); 3] = [
                (0x14, None),
                (key.len() as u32 + 1, Some(key)),
                (full.len() as u32 + 1, Some(full)),
            ];
            let mut holder = Vec::new();
            for (pop_size, text) in pops {
                let has_block = match text {
                    Some(text) => !text.is_empty(),
                    None => !key.is_empty(),
                };
                if !has_block || form_heap_class(pop_size) != size {
                    continue;
                }
                if covering == 0 {
                    holder = text.unwrap_or_default().to_vec();
                    break;
                }
                covering -= 1;
            }
            let bucket = Self::hash(&freed.key) as usize;
            if let Some(entry) = self.buckets[bucket].iter_mut().find(|entry| {
                entry.form == form && oblivion_core::crt::stricmp(&freed.key, &entry.key) == 0
            }) {
                entry.key = holder;
            }
        }
        self.insert_or_assign(key, form);
    }

    /// IDA: 0055E000's walk over this map. Completion: 100%.
    #[must_use]
    pub fn lookup(&self, key: &[u8]) -> Option<Entity> {
        self.buckets[Self::hash(key) as usize]
            .iter()
            .find(|entry| oblivion_core::crt::stricmp(key, &entry.key) == 0)
            .map(|entry| entry.form)
    }

    /// IDA: +0x0C the entry count.
    #[must_use]
    pub const fn count(&self) -> u32 {
        self.count
    }

    /// IDA: N/A one bucket's chain, head first.
    #[must_use]
    pub fn bucket(&self, index: u32) -> &[EditorIdEntry] {
        &self.buckets[index as usize]
    }
}

/// IDA: N/A adds the resources above to a world.
pub fn insert_form_indexes(world: &mut World) {
    world.init_resource::<FormsById>();
    world.init_resource::<DataHandlerLists>();
    world.init_resource::<EditorIdMap>();
}

/// IDA: vtables the DataHandler constructor stores (P09-RUN-001 raw bytes).
pub const TES_FORM_VTABLE: u32 = 0x00A3_BE3C;
pub const TES_SKILL_VTABLE: u32 = 0x00A5_4EFC;
pub const TES_SKILL_DESCRIPTION_VTABLE: u32 = 0x00A5_4EE0;
pub const TES_SKILL_TEXTURE_VTABLE: u32 = 0x00A5_4EC4;
pub const TES_DESCRIPTION_VTABLE: u32 = 0x00A3_B938;
pub const TES_TEXTURE_VTABLE: u32 = 0x00A3_C650;
pub const NI_T_LARGE_ARRAY_CELL_VTABLE: u32 = 0x00A3_7AB4;
pub const TES_REGION_LIST_VTABLE: u32 = 0x00A4_034C;
pub const TES_REGION_DATA_MANAGER_VTABLE: u32 = 0x00A4_00A0;
pub const IDLE_ANIM_ROOT_MAP_BASE_VTABLE: u32 = 0x00A5_38B4;
pub const IDLE_ANIM_ROOT_MAP_VTABLE: u32 = 0x00A5_3920;
pub const FORMS_BY_ID_VTABLE: u32 = 0x00A3_BDD8;

/// IDA: TESDescription (8 bytes): vtable, +0x04 cached description offset.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TesDescription {
    pub original_vtable: u32,
    pub offset_04: u32,
}

/// IDA: TESTexture (0x0C): vtable, +0x04 BSStringT path.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct TesTexture {
    pub original_vtable: u32,
    pub path_04: BsStringT,
}

/// IDA: 00B33C04..00B33C0E the static TESDescription text cache {u32 owner, BSStringT text} that every TESDescription constructor (0046A400) and reset (0046A360) frees and
/// clears. A host record with the reset count.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TesDescriptionTextCache {
    pub owner_b33c04: u32,
    pub text_b33c08: BsStringT,
    pub resets: u32,
}

/// IDA: 004A62D0 TESRegionList(owns) (0x10): vtable A4034C, +0x04/+0x08 = 0, +0x0C = owns.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TesRegionList {
    pub original_vtable: u32,
    pub list_04: [u32; 2],
    pub owns_0c: u8,
}

/// IDA: 004A45F0 TESRegionDataManager (8): vtable A400A0, +0x04 = 0.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TesRegionDataManager {
    pub original_vtable: u32,
    pub field_04: u32,
}

/// IDA: 00521950 BSTCaseInsensitiveStringMap<IDLE_ANIM_ROOT*> (0x14): +0x04 hash size 37, NiTMapBase vtable A538B4, +0x0C count 0, +0x08 buckets (0x94 bytes memset 0), +0x10
/// byte 1, vtable A53920.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IdleAnimRootMap {
    pub original_vtable: u32,
    pub hash_size_04: u32,
    pub buckets_08: Vec<u32>,
    pub count_0c: u32,
    pub byte_10: u8,
}

/// IDA: NiTLargeArray<TESObjectCELL*> at DataHandler +0xC0 (0x18).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NiTLargeArrayCell {
    pub original_vtable: u32,
    /// +0x04, nonzero once the block is allocated: `cells` holds it.
    pub data_04: u32,
    pub capacity_08: u32,
    pub size_0c: u32,
    pub count_10: u32,
    pub grow_by_14: u32,
    /// IDA: +0x04's slots, capacity long.
    pub cells: Vec<Option<Entity>>,
}

/// IDA: 00446D80 DataHandler_Constructor (0xCE0; xOBSE GameData.h DataHandler), present as a resource once 00B33A98 is published. The form lists +0x04..+0xBB are empty here:
/// they are `DataHandlerLists`, which adds each with its form type.
#[derive(Resource)]
pub struct DataHandler {
    /// 004B23A0 (0x10, zero).
    pub bound_objects_00: [u32; 4],
    pub bound_objects_allocated: bool,
    pub region_list_bc: TesRegionList,
    pub cells_c0: NiTLargeArrayCell,
    /// +0xD8 the 21 TESSkills, in order.
    pub skills_d8: [Entity; 21],
    /// +0x8B8 list, +0x8C0 = 0x800 next form ID, +0x8C4 (0: `active_file_8c4` holds the file).
    pub unknown_8b8: [u32; 4],
    /// +0x8C4 the file being loaded.
    pub active_file_8c4: Option<TesFileId>,
    /// +0x8C8 BSSimpleList<TESFile*>, in list order.
    pub files_8c8: Vec<TesFileId>,
    pub loaded_mod_count_8d0: u32,
    pub mods_by_id_8d4: [Option<TesFileId>; 0xFF],
    /// +0xCD0..+0xCD6.
    pub bytes_cd0: [u8; 7],
    /// +0xCD7, set while 0044F0C0 runs. The constructor leaves it as FormHeap bytes (P12-RUN-001 has 08 there).
    pub loading_cd7: u8,
    pub region_data_manager_cd8: TesRegionDataManager,
    pub field_cdc: u32,
    /// IDA: N/A host owner of the TESFile objects the list references.
    pub file_store: TesFileStore,
}

impl DataHandler {
    /// IDA: N/A +0x8C0 the next form ID, in `unknown_8b8`.
    #[must_use]
    pub const fn next_form_id_8c0(&self) -> u32 {
        self.unknown_8b8[2]
    }

    /// IDA: N/A +0x8C0 the next form ID, in `unknown_8b8`.
    pub const fn next_form_id_8c0_mut(&mut self) -> &mut u32 {
        &mut self.unknown_8b8[2]
    }
}

/// IDA: the DataHandler-side globals Phase 09 writes.
#[derive(Resource, Clone, Debug)]
pub struct DataHandlerGlobals {
    /// 00B33C04.
    pub description_cache: TesDescriptionTextCache,
    /// 00B33C1C / 00B33C20.
    pub form_type_cache: FormTypeLookupCache,
    /// 00B35460.
    pub region_list_count_b35460: u32,
    /// 00B362C0.
    pub idle_anim_roots_b362c0: Option<IdleAnimRootMap>,
    /// IDA: 00B33C10 form-type table validated (see the module comment).
    pub form_table_validated_b33c10: bool,
    /// IDA: 00B055CC the TESFile BSFile buffer size (0x2800 in the image).
    pub tes_file_buffer_size_b055cc: u32,
    /// IDA: N/A the form-type validation warnings.
    pub warnings: Vec<String>,
    /// IDA: the keys of the 00B33B00 g_saveLoadGame +0x00 map 00453450 tests (empty while nothing is loaded; P11-RUN-001).
    pub save_load_form_ids: Vec<u32>,
}

impl Default for DataHandlerGlobals {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            description_cache: TesDescriptionTextCache::default(),
            form_type_cache: FormTypeLookupCache::default(),
            region_list_count_b35460: 0,
            idle_anim_roots_b362c0: None,
            form_table_validated_b33c10: true,
            tes_file_buffer_size_b055cc: 0x2800,
            warnings: Vec::new(),
            save_load_form_ids: Vec::new(),
        }
    }
}

// IDA: 00446D80 DataHandler_Constructor -> 00B33A98 (published at 00441B8A). Completion: 100% of the store order. +0xC0's data pointer stays 0 until a cell.
pub fn construct_data_handler(world: &mut World) -> Result<(), GameError> {
    insert_form_indexes(world);
    world.init_resource::<DataHandlerGlobals>();
    world.init_resource::<GameThreadLocals>();
    // 00446DAF..00446E60 +0x04..+0xB8 = 0; 00446E66..00446E8C NiTLargeArray<TESObjectCELL*> at +0xC0.
    let mut cells_c0 = NiTLargeArrayCell {
        original_vtable: NI_T_LARGE_ARRAY_CELL_VTABLE,
        grow_by_14: 1,
        ..NiTLargeArrayCell::default()
    };
    // 00446E92..00446EAB the 21 TESSkill constructors (eh vector iterator).
    let mut skills = [Entity::PLACEHOLDER; 21];
    for skill in &mut skills {
        *skill = construct_skill(world)?;
    }
    // 00446EC0..00446F17 the +0x8B8 block, +0x8C0 = 0x800, the file list head, +0x8D0, the +0xCD0 bytes (+0xCD5 = 1) and TLS +0x184 = 0.
    world.resource_mut::<GameThreadLocals>().byte_184 = 0;
    // 00446F1E..00446F2B memset(+0x8D4, 0, 0x3FC). 00446F30..00446F5B FormHeap(0x10) 004B23A0 -> +0x00.
    // 00446F5D..00446F84 FormHeap(0x10) 004A62D0(1) -> +0xBC: the first region list clears the eight static region pairs (00B35420, all null at startup) and counts in 00B35460.
    world
        .resource_mut::<DataHandlerGlobals>()
        .region_list_count_b35460 += 1;
    // 00446F90 +0xD4 = 100.
    cells_c0.grow_by_14 = 100;
    // 00446F9A..00446FD1 skill IDs 0x3D..0x51 (0046C300 while below 0x5A) and skill values 12..32.
    for (index, &skill) in skills.iter().enumerate() {
        let id = 0x3D + index as u32;
        if id < 0x5A {
            set_form_id(world, skill, id);
        }
        world
            .get_mut::<TesSkill>(skill)
            .expect("just spawned")
            .skill_2c = id as i32 - 0x31;
    }
    // 00446FF5..00447022 FormHeap(0x14) 00521950 -> 00B362C0.
    world
        .resource_mut::<DataHandlerGlobals>()
        .idle_anim_roots_b362c0 = Some(IdleAnimRootMap {
        original_vtable: IDLE_ANIM_ROOT_MAP_VTABLE,
        hash_size_04: 37,
        buckets_08: vec![0; 37],
        count_0c: 0,
        byte_10: 1,
    });
    // 00441B8A g_dataHandler = this.
    world.insert_resource(DataHandler {
        bound_objects_00: [0; 4],
        bound_objects_allocated: true,
        region_list_bc: TesRegionList {
            original_vtable: TES_REGION_LIST_VTABLE,
            list_04: [0; 2],
            owns_0c: 1,
        },
        cells_c0,
        skills_d8: skills,
        unknown_8b8: [0, 0, 0x800, 0],
        active_file_8c4: None,
        files_8c8: Vec::new(),
        loaded_mod_count_8d0: 0,
        mods_by_id_8d4: [None; 0xFF],
        // 00447027 +0xCD6 = 0.
        bytes_cd0: [0, 0, 0, 0, 0, 1, 0],
        loading_cd7: 0,
        // 00446FD3..00446FFC FormHeap(8) TESRegionDataManager -> +0xCD8.
        region_data_manager_cd8: TesRegionDataManager {
            original_vtable: TES_REGION_DATA_MANAGER_VTABLE,
            field_04: 0,
        },
        // 00446F8A +0xCDC = 0.
        field_cdc: 0,
        file_store: TesFileStore::default(),
    });
    Ok(())
}

// IDA: 0044A2B0 on the published DataHandler with "Data\\" (the TESFile close is suppressed while g_dataHandler +0xCD4 is 0). Completion: 100%; `tes_file` has the passes.
pub fn build_data_handler_file_list(
    world: &mut World,
    directory: &str,
    working_directory: &std::path::Path,
    warning: &mut dyn FnMut(String),
) -> Result<(), GameError> {
    world.resource_scope(|world, mut data: Mut<DataHandler>| {
        let mut globals = world.resource_mut::<DataHandlerGlobals>();
        let buffer_size = globals.tes_file_buffer_size_b055cc;
        let close_suppressed = data.bytes_cd0[4] == 0;
        let data = &mut *data;
        let mut warning = |text: String, _disabled: bool| warning(text);
        build_tes_file_list(
            &mut data.files_8c8,
            &mut data.file_store,
            directory,
            buffer_size,
            &mut TesFileServices {
                working_directory,
                warning: &mut warning,
                disable_warning_messages_b06b18: 0,
                close_suppressed,
                form_type_cache: &mut globals.form_type_cache,
            },
        )
    })
}

// IDA: 00447C50 DataHandler_LookupFileByName(this, name) (__thiscall, retn 4; xOBSE `LookupModByName`), called at 0040671E, 00404BFA, 00406762 and 00508ABD. `data_handler` is 00B33A98 and `name` a C string
// up to its first NUL. A null name returns 0 before the list is read. 00447C59 esi = this + 0x8C8, and 00447C61 tests that against 0, which a null this passes, so a null DataHandler reads [0x8C8] and faults:
// the host stops. The walk stops at the first empty item, returns the first `_stricmp(name, item + 0x1C) == 0` ("C" locale, __locale_changed 0 in P10-RUN-001), and ends at a null next node.
// `files_8c8` has no empty items (0044A2B0 leaves none), so a list entry whose file is destroyed stands for that empty item. Completion: 100%.
pub fn lookup_file_by_name(
    data_handler: Option<&DataHandler>,
    name: Option<&[u8]>,
) -> Result<Option<TesFileId>, GameError> {
    // 00447C55 a null name returns 0.
    let Some(name) = name else {
        return Ok(None);
    };
    // 00447C65 reads [this + 0x8C8].
    let data = data_handler.ok_or(GameError::unclassified(
        0x00447C65,
        "00447C65: DataHandler_LookupFileByName reads [0x8C8] on a null DataHandler",
    ))?;
    for &id in &data.files_8c8 {
        // 00447C65 the first empty item ends the walk.
        let Some(file) = data.file_store.get(id) else {
            return Ok(None);
        };
        // 00447C70 _stricmp(name, item + 0x1C); +0x1C holds the name lifted from the disk, so compare the original's bytes.
        let item = oblivion_core::host_path::original_bytes(&file.name_1c);
        if oblivion_core::crt::stricmp(name, &item) == 0 {
            return Ok(Some(id));
        }
    }
    // 00447C7C..00447C83 a null next node returns 0.
    Ok(None)
}

/// IDA: 00518000's stores, in call order: form ID and static. Not in address order (the C++ keeps call order).
#[rustfmt::skip]
const FOOTSTEP_SOUND_ADDRESSES: [(u32, u32); 25] = [(0x212, 0x00B36218), (0x22B, 0x00B36220), (0x213, 0x00B3621C), (0x214, 0x00B36224), (0x215, 0x00B36228), (0x216, 0x00B3622C), (0x21F, 0x00B36230), (0x217, 0x00B36234), (0x218, 0x00B36238), (0x229, 0x00B36274), (0x228, 0x00B36278), (0x219, 0x00B3623C), (0x21A, 0x00B36240), (0x21B, 0x00B36244), (0x21C, 0x00B36248), (0x21D, 0x00B3624C), (0x21E, 0x00B36250), (0x220, 0x00B36254), (0x221, 0x00B36258), (0x222, 0x00B3625C), (0x223, 0x00B36260), (0x225, 0x00B36264), (0x226, 0x00B36268), (0x227, 0x00B3626C), (0x224, 0x00B36270)];

// IDA: the STAT arm: `default_object` over a TESObjectSTAT cast.
fn stat(world: &mut World, id: u32, model: &[u8]) -> Result<Option<Entity>, GameError> {
    default_object(world, 0x1C, RttiClass::TESObjectSTAT, id, Some(model))
}

// IDA: N/A a helper, with no address of its own.
fn misc(world: &mut World, id: u32) -> Result<Option<Entity>, GameError> {
    default_object(world, 0x1B, RttiClass::TESObjectMISC, id, None)
}

// IDA: N/A a helper, with no address of its own.
fn soul_gem(world: &mut World, id: u32) -> Result<Option<Entity>, GameError> {
    default_object(world, 0x26, RttiClass::TESSoulGem, id, None)
}

// IDA: a default whose lookup has no cast: any form at the ID stays.
fn uncast(
    world: &mut World,
    form_type: u8,
    id: u32,
    model: Option<&[u8]>,
) -> Result<Option<Entity>, GameError> {
    if let Some(found) = lookup(world, id) {
        return Ok(Some(found));
    }
    default_object(world, form_type, RttiClass::TESForm, id, model)
}

/// IDA: 0044AC20 DataHandler_CreateDefaultForms, with TLS +0x184 set by 0044F3F9. Completion: 100% of the order and the forms; 00543200's graphics are a host boundary, counted.
/// TODO: [ ] 0044CD49 00543200's sky graphics (`sky_set_climate_default`).
pub fn create_default_forms(world: &mut World) -> Result<(), GameError> {
    debug_assert!(world.resource::<GameThreadLocals>().byte_184 != 0);
    // 0044AC5D..0044ACD5.
    let surviving = std::mem::take(world.get_resource_or_init::<DefaultFormStatics>().as_mut());
    {
        let mut statics = world.resource_mut::<DefaultFormStatics>();
        statics.loot_bag_b33aa8 = surviving.loot_bag_b33aa8;
        statics.stolen_goods_b33aac = surviving.stolen_goods_b33aac;
        statics.jail_shirt_b33ab0 = surviving.jail_shirt_b33ab0;
        statics.jail_pants_b33ab4 = surviving.jail_pants_b33ab4;
        statics.jail_shoes_b33ab8 = surviving.jail_shoes_b33ab8;
    }
    let eyes = default_eyes(world)?;
    world
        .resource_mut::<DefaultFormStatics>()
        .eyes_reanimate_b362bc = Some(eyes);
    let race = create_default_vampire_race(world)?;
    world
        .resource_mut::<DefaultFormStatics>()
        .vampire_race_b36308 = Some(race);
    let water = create_default_water(world)?;
    world
        .resource_mut::<DefaultFormStatics>()
        .default_water_b360ac = Some(water);
    let form = stat(world, 0x12, b"Marker_Horse.nif")?;
    world
        .resource_mut::<DefaultFormStatics>()
        .horse_marker_b35ed4 = form;
    let form = misc(world, 0x194)?;
    world
        .resource_mut::<DefaultFormStatics>()
        .varla_stone_b35edc = form;
    let form = misc(world, 0x191)?;
    world
        .resource_mut::<DefaultFormStatics>()
        .welkynd_stone_b35ed8 = form;
    let form = soul_gem(world, 0x192)?;
    world
        .resource_mut::<DefaultFormStatics>()
        .black_soul_gem_b35ee0 = form;
    let form = soul_gem(world, 0x193)?;
    world
        .resource_mut::<DefaultFormStatics>()
        .azura_stone_b35ee4 = form;
    let form = misc(world, 0x0A)?;
    world.resource_mut::<DefaultFormStatics>().lockpick_b35ec8 = form;
    let form = misc(world, 0x0B)?;
    world
        .resource_mut::<DefaultFormStatics>()
        .skeleton_key_b35ecc = form;
    let form = misc(world, 0x0C)?;
    world
        .resource_mut::<DefaultFormStatics>()
        .repair_hammer_b35ed0 = form;
    let form = stat(world, 0x01, b"MarkerTeleport.nif")?;
    world
        .resource_mut::<DefaultFormStatics>()
        .door_marker_b35ea4 = form;
    create_default_world_space(world)?;
    let form = stat(world, 0x3B, b"MarkerX.nif")?;
    world.resource_mut::<DefaultFormStatics>().x_marker_b35eac = form;
    let form = stat(world, 0x34, b"MarkerXHeading.nif")?;
    world
        .resource_mut::<DefaultFormStatics>()
        .x_marker_heading_b35eb0 = form;
    let form = stat(world, 0x10, b"Marker_Map.NIF")?;
    world.resource_mut::<DefaultFormStatics>().map_marker_b35ea8 = form;
    let form = stat(world, 0x02, b"Marker_Travel.nif")?;
    world
        .resource_mut::<DefaultFormStatics>()
        .travel_marker_b35eb4 = form;
    let form = stat(world, 0x03, b"Marker_North.nif")?;
    world
        .resource_mut::<DefaultFormStatics>()
        .north_marker_b35eb8 = form;
    let form = default_object(
        world,
        0x18,
        RttiClass::TESObjectDOOR,
        0x04,
        Some(b"Marker_Prison.nif"),
    )?;
    world
        .resource_mut::<DefaultFormStatics>()
        .prison_marker_b35ebc = form;
    let form = stat(world, 0x06, b"Marker_Temple.nif")?;
    world
        .resource_mut::<DefaultFormStatics>()
        .temple_marker_b35ec0 = form;
    let form = stat(world, 0x05, b"Marker_Divine.nif")?;
    world
        .resource_mut::<DefaultFormStatics>()
        .divine_marker_b35ec4 = form;
    let form = default_creature_faction(world)?;
    world
        .resource_mut::<DefaultFormStatics>()
        .creature_faction_b36298 = Some(form);
    // 0x0F Gold001, 0x0E LootBag, 0x11 StolenGoods, with no cast. 0044BE74..0044BEA9: the bag's empty path takes the sack, which a new one always has.
    uncast(world, 0x1B, 0x0F, None)?;
    let form = uncast(world, 0x17, 0x0E, Some(b"Clutter\\Sack01.NIF"))?;
    world.resource_mut::<DefaultFormStatics>().loot_bag_b33aa8 = form;
    let form = uncast(world, 0x17, 0x11, None)?;
    world
        .resource_mut::<DefaultFormStatics>()
        .stolen_goods_b33aac = form;
    let form = default_clothing(world, 0x17, "Shirt", &[(2, true), (3, false)])?;
    world.resource_mut::<DefaultFormStatics>().jail_shirt_b33ab0 = form;
    let form = default_clothing(world, 0x15, "Pants", &[(2, false), (3, true)])?;
    world.resource_mut::<DefaultFormStatics>().jail_pants_b33ab4 = form;
    let form = default_clothing(world, 0x16, "Shoes", &[(2, false), (3, false), (5, true)])?;
    world.resource_mut::<DefaultFormStatics>().jail_shoes_b33ab8 = form;
    // 0044C2B5..0044C350 the FlameNode statics of 00B067C0 (0x54 bytes of IDs), 0044C4D0..0044C59C the FurnitureMarkers 100..119 (0x50 bytes): no model, no cast.
    for id in [
        0x1F, 0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x1E, 0x28, 0x29, 0x2A, 0x2B, 0x2C,
        0x2D, 0x2E, 0x2F, 0x30, 0x31, 0x32,
    ] {
        uncast(world, 0x1C, id, None)?;
    }
    for id in 100..120 {
        uncast(world, 0x1C, id, None)?;
    }
    // 0044C5A2 0052FE90.
    create_default_topics(world)?;
    // 0044C5A7..0044CAD0 the six time globals.
    create_default_globals(world)?;
    // 0044CAD0..0044CB94 the player's NPC_.
    create_default_player_npc(world)?;
    // 0044CB94.. 0x15E, then 0x15F.
    create_default_weather(world)?;
    create_default_climate(world)?;
    // 0044CD42 Sky_GetSingleton, 0044CD49 00543200(sky, 0, 1).
    sky_set_climate_default(world)?;
    world.resource_mut::<DefaultFormStatics>().sky_climate_calls += 1;
    // 0044CD4E 0041BB80: the 18 sounds, the two spells, the three shaders.
    let mut sounds = Vec::with_capacity(MAGIC_SOUNDS.len());
    for (id, name) in MAGIC_SOUNDS {
        sounds.push(Some(default_sound(world, id, name, false)?));
    }
    let spells = create_default_spells(world)?;
    let shaders = create_default_effect_shaders(world)?;
    {
        let mut statics = world.resource_mut::<DefaultFormStatics>();
        statics.magic_sounds_b33560 = sounds;
        statics.default_spells_b335a8 = [Some(spells[0]), Some(spells[1])];
        statics.effect_shaders_b335b0 = shaders.into_iter().map(Some).collect();
    }
    // 0044CD53 00518000.
    let mut sounds = [None; 25];
    for (at, (id, name)) in FOOTSTEP_SOUNDS.into_iter().enumerate() {
        let (store_id, address) = FOOTSTEP_SOUND_ADDRESSES[at];
        debug_assert_eq!(store_id, id);
        sounds[((address - 0x00B3_6218) / 4) as usize] =
            Some(default_sound(world, id, name, true)?);
    }
    world
        .resource_mut::<DefaultFormStatics>()
        .footstep_sounds_b36218 = sounds;
    Ok(())
}

/// IDA: 00447300 DataHandler_RefreshMarkedNPCActorValues (WinMain 0040EBAA): +0x00's object list from 004B2460's first, each next through 004B2360 read after the call; `__RTDynamicCast` to TESNPC, and +0x28 bit 7 runs 005222D0(0). After 0040EB8B its vt+0x40 calls record, which is P14-RUN-001's +1,679 change entries. Answers the visited NPCs in order. Completion: 100%.
/// # Errors
/// A stop in 005222D0.
pub fn refresh_pc_level_npcs(world: &mut World) -> Result<Vec<Entity>, GameError> {
    let mut visited = Vec::new();
    let mut object = world.resource::<DataHandlerLists>().objects_00.first_04;
    while let Some(form) = object {
        if casts_to(world, form, RttiClass::TESNPC)
            && world
                .get::<TesActorBaseData>(form)
                .expect("an NPC")
                .flags_04
                & 0x80
                != 0
        {
            auto_calc(world, form)?;
            visited.push(form);
        }
        object = world
            .get::<TesObjectLinks>(form)
            .expect("the list holds TESObjects")
            .next_20;
    }
    Ok(visited)
}

/// IDA: 00447D80 on g_dataHandler: +0x64's vt+0x6C until a null item, then 00447CA0: +0xCC read once and tested signed, each nonzero +0xC4 slot's 004CB8C0(cell, 1, 1); +0x0C's worldspaces, a null item skipped: 004EF1E0's +0x34, nonzero, the same. Completion: 100%.
/// # Errors
/// A stop below.
pub fn init_scripts_and_cells(
    world: &mut World,
    services: &mut DataLoadServices<'_>,
) -> Result<(), GameError> {
    let scripts: Vec<Entity> = world
        .resource::<DataHandlerLists>()
        .scripts_64
        .items()
        .collect();
    for script in scripts {
        init_item_of(world, script, services)?;
    }
    let array = &world.resource::<DataHandler>().cells_c0;
    let cells: Vec<Option<Entity>> = array
        .cells
        .iter()
        .take(array.size_0c.min(i32::MAX as u32) as usize)
        .copied()
        .collect();
    for cell in cells.into_iter().flatten() {
        cell_run_reference_scripts(world, cell, true, true)?;
    }
    let worlds: Vec<Entity> = world
        .resource::<DataHandlerLists>()
        .world_spaces_0c
        .items()
        .collect();
    for form in worlds {
        let cell = world
            .get::<TesWorldSpace>(form)
            .expect("a worldspace")
            .persistent_cell_34;
        if let Some(cell) = cell {
            cell_run_reference_scripts(world, cell, true, true)?;
        }
    }
    Ok(())
}

// IDA: 00447130: 004507D0 on each +0x8C8 file until a null item. Completion: 100%.
pub(crate) fn close_all_files(
    world: &mut World,
    services: &mut TesGridServices<'_, '_>,
) -> Result<(), GameError> {
    let list = world.resource::<DataHandler>().files_8c8.clone();
    crate::tes::data_load::with_files(
        world,
        services.load,
        |_world, files, file_services, _host| {
            for id in list {
                close_tes_file(files.get_mut(id)?, file_services)?;
            }
            Ok(())
        },
    )
}

/// IDA: 0044CB0E..0044CB92, in 0044AC20: g_formsByID 7 through the TESNPC cast; else FormHeap(0x200) 00527220, 0046C300(7, 1), vt+0xD8 006FDFF0, 004B2B80 on +0x00, vt+0x90(0). Completion: 100%, called by a test until 12d has 0044AC20.
pub fn create_default_player_npc(world: &mut World) -> Result<Entity, GameError> {
    if let Some(form) = world.resource::<FormsById>().lookup(7).filter(|&form| {
        world
            .get::<FormIdentity>(form)
            .expect("a form")
            .form_type_04
            == FORM_TYPE_NPC
    }) {
        return Ok(form);
    }
    let form = construct_npc(world)?;
    set_form_id_release(world, form, 7, true)?;
    append_tes_object(world, form);
    clear_altered(world, form)?;
    Ok(form)
}

/// IDA: 0044ADA8..0044AE45, in 0044AC20, the VampireRace: g_formsByID 0x19 through the TESRace cast into 00B36308; else FormHeap(0x318) 0052D100, 0046C300(0x19, 1), vt+0xD8 006FDFF0, the +0x44 prepend, vt+0x90(0). Completion: 100%, called by a test until 12d has 0044AC20.
pub fn create_default_vampire_race(world: &mut World) -> Result<Entity, GameError> {
    if let Some(form) = world.resource::<FormsById>().lookup(0x19).filter(|&form| {
        world
            .get::<FormIdentity>(form)
            .expect("a form")
            .form_type_04
            == FORM_TYPE_RACE
    }) {
        return Ok(form);
    }
    let form = construct_race(world)?;
    set_form_id_release(world, form, 0x19, true)?;
    world
        .resource_mut::<DataHandlerLists>()
        .races_44
        .prepend_non_null(form);
    clear_altered(world, form)?;
    Ok(form)
}

/// IDA: 0044AE4C the DefaultWater arm of 0044AC20: an ID 0x18 that casts to TESWaterForm is kept (and answered); otherwise FormHeap(0xAC) 004EDE40, 0046C300, vt+0xD8 006FDFF0 (the name goes nowhere), 004EDDA0 "Water\water00.dds" on +0x24, the +0xA4 prepend, then vt+0x90(0). Completion: 100% of the arm, with its global for 12d.
pub fn create_default_water(world: &mut World) -> Result<Entity, GameError> {
    if let Some(form) = world
        .resource::<FormsById>()
        .lookup(0x18)
        .filter(|&form| world.get::<TesWaterForm>(form).is_some())
    {
        return Ok(form);
    }
    let form = construct_water(world)?;
    set_form_id(world, form, 0x18);
    world
        .get_mut::<TesTexture>(form)
        .expect("just built")
        .path_04
        .set(Some(b"Water\\water00.dds"), 0);
    world
        .resource_mut::<DataHandlerLists>()
        .water_forms_a4
        .prepend_non_null(form);
    clear_altered(world, form)?;
    Ok(form)
}

/// IDA: 0044CB9C the DefaultWeather arm of 0044AC20, whose lookup has no cast: an ID 0x15E that is present stays; otherwise FormHeap(0x148) 004EE7C0, 0046C300, vt+0xD8 "DefaultWeather" (goes nowhere), 004EE8C0, the inlined +0x1C prepend. Completion: 100% of the arm.
pub fn create_default_weather(world: &mut World) -> Result<(), GameError> {
    if world.resource::<FormsById>().lookup(0x15E).is_some() {
        return Ok(());
    }
    let form = construct_weather(world)?;
    set_form_id(world, form, 0x15E);
    make_default_weather(&mut world.get_mut::<TesWeather>(form).expect("just built"));
    world
        .resource_mut::<DataHandlerLists>()
        .weathers_1c
        .prepend_non_null(form);
    Ok(())
}

/// IDA: 0044CC6F the DefaultClimate arm of 0044AC20: an ID 0x15F that is present stays; otherwise FormHeap(0x58) 004BEC50, 0046C300, vt+0xD8 "DefaultClimate" (goes nowhere), 004BEE70, the inlined +0x14 prepend. 004BEE70: the sun and glare paths, vt+0x18 "Sky\Stars.nif" on the model, the six TNAM bytes, the list emptied, and {0x15E as TESWeather, 100} prepended, or a warning (to `DataHandlerGlobals::warnings`, since this runs outside a file). Completion: 100% of the arm.
pub fn create_default_climate(world: &mut World) -> Result<(), GameError> {
    if world.resource::<FormsById>().lookup(0x15F).is_some() {
        return Ok(());
    }
    let form = construct_climate(world)?;
    set_form_id(world, form, 0x15F);
    let weather = world
        .resource::<FormsById>()
        .lookup(0x15E)
        .filter(|&found| {
            world
                .get::<FormIdentity>(found)
                .is_some_and(|identity| identity.form_type_04 == FORM_TYPE_WEATHER)
        });
    {
        let mut climate = world.get_mut::<TesClimate>(form).expect("just built");
        climate.textures_38[0].path_04.set(Some(b"Sky\\Sun.dds"), 0);
        climate.textures_38[1]
            .path_04
            .set(Some(b"Sky\\SunGlare.dds"), 0);
        climate.model_18.path_04.set(Some(b"Sky\\Stars.nif"), 0);
        climate.timing_50 = [0x24, 0x2A, 0x6C, 0x72, 0, 0xC3];
        climate.weathers_30.clear();
        if let Some(weather) = weather {
            climate.weathers_30.insert(
                0,
                WeatherEntry {
                    weather,
                    chance: 100,
                },
            );
        }
    }
    if weather.is_none() {
        world.resource_mut::<DataHandlerGlobals>().warnings.push(
            "Unable to add default weather to default climate.  ( TESClimate::MakeDefault() )"
                .to_owned(),
        );
    }
    world
        .resource_mut::<DataHandlerLists>()
        .climates_14
        .prepend_non_null(form);
    Ok(())
}

/// IDA: 0044AD11..0044AD93, the 0x1A eyeReanimate arm: through the TESEyes cast; FormHeap(0x34), 0051EFD0, vt+0xD8, 004028D0(+0x1C), +0x3C prepend, vt+0x90(0). Row-backed by P12-RUN-001. Completion: 100%.
pub fn default_eyes(world: &mut World) -> Result<Entity, GameError> {
    if let Some(found) = cast(
        world,
        world.resource::<FormsById>().lookup(0x1A),
        RttiClass::TESEyes,
    ) {
        return Ok(found);
    }
    let form = construct_default(world, 0x08)?;
    set_form_id(world, form, 0x1A);
    world
        .get_mut::<TesFullName>(form)
        .expect("eyes")
        .name_04
        .set(Some(b"Reanimate Eyes"), 0);
    world
        .resource_mut::<DataHandlerLists>()
        .eyes_3c
        .prepend_non_null(form);
    clear_altered(world, form)?;
    Ok(form)
}

/// IDA: 0044BC85..0044BD5E, the 0x13 CreatureFaction arm: through the TESFaction cast; FormHeap(0x44), 0051F820, +0x5C prepend, 0046E900(self, 0x64) on +0x24, 0051F760 an empty rank, 0046AB80(1), vt+0x90(0). Row-backed by P12-RUN-001. Completion: 100%.
pub fn default_creature_faction(world: &mut World) -> Result<Entity, GameError> {
    if let Some(found) = cast(
        world,
        world.resource::<FormsById>().lookup(0x13),
        RttiClass::TESFaction,
    ) {
        return Ok(found);
    }
    let form = construct_default(world, 0x06)?;
    set_form_id(world, form, 0x13);
    world
        .resource_mut::<DataHandlerLists>()
        .factions_5c
        .prepend_non_null(form);
    world
        .get_mut::<TesReactionForm>(form)
        .expect("a faction")
        .reactions_04
        .push(TesReaction {
            faction: FormRef::Resolved(Some(form)),
            modifier: 100,
        });
    world
        .get_mut::<TesFaction>(form)
        .expect("a faction")
        .ranks_3c
        .push(TesFactionRank {
            insignia_10: TesTexture {
                original_vtable: TES_TEXTURE_VTABLE,
                path_04: BsStringT::default(),
            },
            ..TesFactionRank::default()
        });
    set_initialized(world, form, true);
    clear_altered(world, form)?;
    Ok(form)
}

/// IDA: 0044B57E..0044B642 in 0044AC20: g_formsByID 0x3C through the TESWorldSpace cast. None: 004F2A10, 0046C300(0x3C, 1), vt+0xD8("Tamriel"), vt+0x90(0), a prepend to +0x0C, and "Tamriel" in 00B06164. One: to the head of +0x0C unless it's there. Completion: 100%, called by a test until 12d has 0044AC20.
pub fn create_default_world_space(world: &mut World) -> Result<Entity, GameError> {
    const NAME: &[u8] = b"Tamriel";
    if let Some(form) = world
        .resource::<FormsById>()
        .lookup(0x3C)
        .filter(|&form| world.get::<TesWorldSpace>(form).is_some())
    {
        let mut lists = world.resource_mut::<DataHandlerLists>();
        if lists.world_spaces_0c.items().next() != Some(form) {
            lists.world_spaces_0c.remove_first_matching(form);
            lists.world_spaces_0c.prepend_non_null(form);
        }
        return Ok(form);
    }
    let form = construct_world(world)?;
    set_form_id_release(world, form, 0x3C, true)?;
    world
        .get_mut::<TesWorldSpace>(form)
        .expect("just built")
        .editor_c0
        .set(Some(NAME), 0);
    clear_altered(world, form)?;
    world
        .resource_mut::<DataHandlerLists>()
        .world_spaces_0c
        .prepend_non_null(form);
    world
        .resource_mut::<EditorIdMap>()
        .insert_or_assign(NAME, form);
    Ok(form)
}

/// IDA: 0044C5A7..0044CAD0, the globals arm of 0044AC20: in order GameYear .. TimeScale (the globals 00402860
/// TimeGlobals_Init names). Each runs SetFormID, vt+0xD8, the +0x24 value, the +0x74 prepend and the editor-ID map. An ID
/// already present is skipped.
/// Completion: 100% of the globals; the function's other form types arrive with them.
pub fn create_default_globals(world: &mut World) -> Result<(), GameError> {
    const DEFAULTS: [(u32, &[u8], f32); 6] = [
        (0x35, b"GameYear", 427.0),
        (0x36, b"GameMonth", 7.0),
        (0x37, b"GameDay", 17.0),
        (0x38, b"GameHour", 12.0),
        (0x39, b"GameDaysPassed", 1.0),
        (0x3A, b"TimeScale", 30.0),
    ];
    for (id, name, value) in DEFAULTS {
        if world.resource::<FormsById>().lookup(id).is_some() {
            continue;
        }
        let form = construct_global(world)?;
        set_form_id(world, form, id);
        world
            .get_mut::<EditorId>(form)
            .expect("just built")
            .0
            .set(Some(name), 0);
        world
            .get_mut::<TESGlobal>(form)
            .expect("just built")
            .value_bits_24 = value.to_bits();
        world
            .resource_mut::<DataHandlerLists>()
            .globals_74
            .prepend_non_null(form);
        world
            .resource_mut::<EditorIdMap>()
            .insert_or_assign(name, form);
    }
    Ok(())
}
