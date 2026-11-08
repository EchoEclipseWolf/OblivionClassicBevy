//! DataHandler's form indexes, which carry the original's order (Bevy storage never does), and Phase 09 (D4): the DataHandler TES_Constructor builds (FormHeap(0xCE0)
//! 00446D80 -> 00B33A98) with its 21 TESSkill forms registered as form IDs 0x3D..0x51 in g_formsByID (00B0613C), the region list, the idle-animation root map (00B362C0)
//! and the plugin file list 0044A2B0 (`tes_file`).
//!
//! The first TESForm of the process runs the form-type table validation (0046C486..0046C550, 00B33C10); the CRT static forms carried from Phase 00 ran it before WinMain
//! (P09-RUN-001: 00B33C10 = 1 at the window), so `DataHandlerGlobals` starts with it done, and a test can clear it.
//! Evidence: evidence/phase09-window-2026-10-06/disasm-09d.txt, disasm-09f.txt (0052EAF0, 0046A400), disasm-09j/k.txt (0046B090 components), P09-RUN-001.

use bevy_ecs::prelude::*;
use recomp_core::pointer_list::PointerList;

use super::form::{FORM_FLAG_PARTIAL, FormIdentity, ModReferences, OriginalVtable};
use super::tes::GameThreadLocals;
use super::tes_file::{
    BsStringT, FORM_TYPE_TABLE, FormTypeLookupCache, TesFileId, TesFileServices, TesFileStore,
    build_tes_file_list, record_fourcc,
};
use recomp_core::reader::StreamError;

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

/// IDA: 0046C2F0 TESForm_SetFormID(key, 1), without the 00446B40 hand-back: the C++ startup callers pass release = 1, and the paths differ only for an old ID with high byte 0xFF, which none of these callers has. The
/// other callers use `set_form_id_release`. Completion: 100% of its callers' arms.
pub fn set_form_id(world: &mut World, form: Entity, key: u32) {
    let identity = *world
        .get::<FormIdentity>(form)
        .expect("a form has an identity");
    let old = identity.form_id_0c;
    if key == old {
        return;
    }
    // 0046C30F..0046C318 forms with flag 0x4000 keep their map entries.
    if identity.flags_08 & FORM_FLAG_PARTIAL == 0 {
        let mut forms = world.resource_mut::<FormsById>();
        if old != 0 {
            forms.remove(old);
        }
        if key != 0 {
            forms.set_at(key, form);
        }
    }
    world
        .get_mut::<FormIdentity>(form)
        .expect("checked above")
        .form_id_0c = key;
}

/// IDA: 0046C300 TESForm_SetFormID(key, release): an equal key returns; without flag 0x4000 the old key leaves the map, a set `release` hands a nonzero old ID back (00446B40) and a nonzero key is inserted; then
/// +0x0C = key. Completion: 100%.
pub fn set_form_id_release(
    world: &mut World,
    form: Entity,
    key: u32,
    release: bool,
) -> Result<(), StreamError> {
    let identity = *world
        .get::<FormIdentity>(form)
        .expect("a form has an identity");
    let old = identity.form_id_0c;
    if key == old {
        return Ok(());
    }
    // 0046C30F..0046C318 forms with flag 0x4000 keep their map entries.
    if identity.flags_08 & FORM_FLAG_PARTIAL == 0 {
        let mut forms = world.resource_mut::<FormsById>();
        if old != 0 {
            forms.remove(old);
        }
        // 0046C329..0046C33E 00446B40(g_dataHandler, old) for a nonzero old ID when `release` is set.
        if release && old != 0 {
            release_form_id(world.get_resource_mut::<DataHandler>().as_deref_mut(), old)?;
        }
        if key != 0 {
            world.resource_mut::<FormsById>().set_at(key, form);
        }
    }
    world
        .get_mut::<FormIdentity>(form)
        .expect("checked above")
        .form_id_0c = key;
    Ok(())
}

/// IDA: 00448D40 DataHandler_AllocateFormID (__thiscall on the DataHandler): remember +0x8C0; advance it ((v & 0xFFFFFF) == 0xFFFFFF ? (v & 0xFF000000) + 0x800 : v + 1) past every key present in g_formsByID (vt+4
/// HashKey, vt+8 KeysEqual walk) or in the g_saveLoadGame +0x00 map (00453450); return the remembered value unless one of the two maps holds it, in which case the current counter becomes the remembered value and
/// the loop repeats. `changes` is the 00453450 map, `TesSaveLoadGame::changes_00`, read through a null g_saveLoadGame as no key since the callers' tests build no TES; the original faults there. `save_load_form_ids` adds keys for a test. Completion: 100%.
pub fn allocate_form_id(
    next_form_id_8c0: &mut u32,
    forms: &FormsById,
    save_load_form_ids: &[u32],
    changes: Option<&super::tes::ChangeChains>,
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
) -> Result<u32, StreamError> {
    // 00446B44..00446B56 a nonzero ID with high byte 0xFF.
    if form_id == 0 || form_id & 0xFF00_0000 != 0xFF00_0000 {
        return Ok(form_id);
    }
    // 00446B58 [ecx+0x8C0]: a null DataHandler faults here.
    let Some(data_handler) = data_handler else {
        return Err(StreamError(
            "00446B58: 00446B40 reads +0x8C0 of a null DataHandler",
        ));
    };
    if form_id == data_handler.next_form_id_8c0().wrapping_sub(1) && form_id & 0x00FF_FFFF > 0x7FF {
        *data_handler.next_form_id_8c0_mut() = form_id;
    }
    Ok(form_id)
}

/// IDA: 0046B6C0 with the file in hand, for the 0044F0C0 loop, where the store is out of `DataHandler` (`data_load::with_file_store`). Completion: 100% of the modelled files.
pub fn update_mod_references_from(
    world: &mut World,
    form: Entity,
    file: &super::tes_file::TesFileData,
) -> Result<(), StreamError> {
    if file.ghost_parent_04 != 0 {
        return Err(StreamError(
            "0046B6CE: ghost TESFiles are not modelled by the host",
        ));
    }
    world
        .get_mut::<ModReferences>(form)
        .expect("a form has a mod reference list")
        .update(file.mod_index_400, file.is_master());
    Ok(())
}

/// IDA: 0046B6C0 TESForm_UpdateModReferenceList(file) (__thiscall, retn 4): a file is first replaced by its root ghost parent (0044FB90, TESFile +0x04; no ghost file is modelled, so a nonzero +0x04 stops); a master
/// (0044FAA0: +0x3DC bit 0) clears the nodes (004526E0) and is prepended; another file is skipped when present, else appended. A null file removes the last non-empty entry. Completion: 100% of the modelled files.
/// TODO: [ ] Ghost parents (M4).
pub fn update_mod_reference_list(
    world: &mut World,
    form: Entity,
    file: Option<TesFileId>,
) -> Result<(), StreamError> {
    let Some(file) = file else {
        world
            .get_mut::<ModReferences>(form)
            .expect("a form has a mod reference list")
            .remove_last();
        return Ok(());
    };
    let data = world.resource::<DataHandler>();
    let file = data
        .file_store
        .get(file)
        .ok_or(StreamError("0046B6C0: a destroyed TESFile"))?;
    if file.ghost_parent_04 != 0 {
        return Err(StreamError(
            "0046B6CE: ghost TESFiles are not modelled by the host",
        ));
    }
    let (mod_index, is_master) = (file.mod_index_400, file.flags_3dc & 1 != 0);
    world
        .get_mut::<ModReferences>(form)
        .expect("a form has a mod reference list")
        .update(mod_index, is_master);
    Ok(())
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
        let lowered = recomp_core::crt::c_tolower(i32::from(byte as i8)) as i8;
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
            .find(|entry| recomp_core::crt::stricmp(key, &entry.key) == 0)
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
                entry.form == form && recomp_core::crt::stricmp(&freed.key, &entry.key) == 0
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
            .find(|entry| recomp_core::crt::stricmp(key, &entry.key) == 0)
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

/// IDA: 0052EB80 TESSkill (0x60), past the TESForm head (`FormIdentity`, `OriginalVtable`, `ModReferences`): TESDescription +0x18 (vtable A54EE0), TESTexture +0x20 (vtable
/// A54EC4), +0x2C skill (-1 from 0052EAF0, then the DataHandler's 12..32), +0x30/+0x34 = 0, +0x38/+0x3C = 1.0, four level TESDescriptions +0x40..+0x58.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct TesSkill {
    pub description_18: TesDescription,
    pub texture_20: TesTexture,
    pub skill_2c: i32,
    pub field_30: u32,
    /// +0x34 the specialization (00663CBA compares it with the class's +0x40).
    pub specialization_34: u32,
    pub float_38: f32,
    pub float_3c: f32,
    pub level_descriptions_40: [TesDescription; 4],
}

impl TesSkill {
    /// IDA: N/A the 0x14 bytes 0046BDA0 copies its DATA into, +0x2C..+0x40.
    #[must_use]
    pub fn data_2c_bytes(&self) -> [u8; 0x14] {
        let mut bytes = [0_u8; 0x14];
        bytes[0x00..0x04].copy_from_slice(&self.skill_2c.to_le_bytes());
        bytes[0x04..0x08].copy_from_slice(&self.field_30.to_le_bytes());
        bytes[0x08..0x0C].copy_from_slice(&self.specialization_34.to_le_bytes());
        bytes[0x0C..0x10].copy_from_slice(&self.float_38.to_bits().to_le_bytes());
        bytes[0x10..0x14].copy_from_slice(&self.float_3c.to_bits().to_le_bytes());
        bytes
    }

    /// IDA: N/A the same bytes, back into the fields, with the floats as raw bits.
    pub fn set_data_2c_bytes(&mut self, bytes: &[u8; 0x14]) {
        let word =
            |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().expect("four bytes"));
        self.skill_2c = word(0x00) as i32;
        self.field_30 = word(0x04);
        self.specialization_34 = word(0x08);
        self.float_38 = f32::from_bits(word(0x0C));
        self.float_3c = f32::from_bits(word(0x10));
    }
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

/// IDA: 0046A400 TESDescription_Constructor / 0046A360 vt+0 Init: free the static text cache (00B33C08) and clear it, then +0x04 = 0. Completion: 100%.
pub fn reset_description(description: &mut TesDescription, globals: &mut DataHandlerGlobals) {
    let cache = &mut globals.description_cache;
    cache.text_b33c08 = BsStringT::default();
    cache.owner_b33c04 = 0;
    cache.resets += 1;
    description.offset_04 = 0;
}

// IDA: 0046C486..0046C550 the first-form validation of the form-type table: each entry's type byte must equal its index and its fourcc must be unique ("formEnumString[ %d ]..."
// warnings; "You must fix the problems in TESForm.cpp" when any), then 00B33C10 = 1. Completion: 100% of the checks (the fourccs are the constant table's names).
fn validate_form_type_table(globals: &mut DataHandlerGlobals) {
    let mut failed = false;
    for (index, (kind, name)) in FORM_TYPE_TABLE.iter().enumerate() {
        if usize::from(*kind) != index {
            globals.warnings.push(format!(
                "formEnumString[ {index} ].cFormID in TESForm.cpp does not match its index"
            ));
            failed = true;
        }
        for (other, (_, other_name)) in FORM_TYPE_TABLE.iter().enumerate() {
            if other != index && record_fourcc(name) == record_fourcc(other_name) {
                globals.warnings.push(format!(
                    "formEnumString[ {index} ] and formEnumString[ {other} ] share {name}"
                ));
                failed = true;
            }
        }
    }
    if failed {
        globals
            .warnings
            .push("You must fix the problems in TESForm.cpp".to_owned());
    }
    globals.form_table_validated_b33c10 = true;
}

// IDA: 0046C460 TESForm_Constructor on a new entity (the head components, which the derived constructors then rewrite): 0046C470 vtable A3BE3C; 0046C47A/0046C47D +0x10 = +0x14 = 0; 0046C469..0046C550 the
// once-per-process validation; 0046C557..0046C562 +0x04 = 0, +0x08 = 8, +0x0C = 0; 0046C569..0046C5FD with g_dataHandler set and the TLS +0x184 byte clear, a new form ID from 00448D40, TESForm_UpdateModReferenceList
// (+0x8C4) and an ID at or below 0x7FF moving to 0x800; 0046C604..0046C612 a nonzero ID is registered in g_formsByID. Completion: 100% of the store order.
pub fn construct_tes_form(world: &mut World) -> Result<Entity, StreamError> {
    let validated = world
        .resource::<DataHandlerGlobals>()
        .form_table_validated_b33c10;
    if !validated {
        validate_form_type_table(&mut world.resource_mut::<DataHandlerGlobals>());
    }
    let form = world
        .spawn((
            FormIdentity {
                form_type_04: 0,
                flags_08: 8,
                form_id_0c: 0,
            },
            OriginalVtable(TES_FORM_VTABLE),
            ModReferences::default(),
        ))
        .id();
    if world.contains_resource::<DataHandler>()
        && world.resource::<GameThreadLocals>().byte_184 == 0
    {
        // 0046C58F 00448D40 on the DataHandler (ecx = g_dataHandler).
        let id = world.resource_scope(|world, mut data: Mut<DataHandler>| {
            allocate_form_id(
                data.next_form_id_8c0_mut(),
                world.resource::<FormsById>(),
                &world.resource::<DataHandlerGlobals>().save_load_form_ids,
                world
                    .get_resource::<super::tes::TesGlobals>()
                    .and_then(|globals| globals.save_load_game.as_ref())
                    .map(|game| &game.changes_00),
            )
        });
        set_identity_id(world, form, id);
        // 0046C597..0046C5AA TESForm_UpdateModReferenceList(+0x8C4) when set.
        if let Some(file) = world.resource::<DataHandler>().active_file_8c4 {
            update_mod_reference_list(world, form, Some(file))?;
        }
        // 0046C5AF..0046C5FD an ID at or below 0x7FF (the 0x800 compare that follows can never match) moves to 0x800: without flag 0x4000 the old key leaves the map, 00446B40 gives it back and 0x800 is inserted.
        let identity = *world.get::<FormIdentity>(form).expect("just spawned");
        if identity.form_id_0c <= 0x7FF {
            if identity.flags_08 & FORM_FLAG_PARTIAL == 0 {
                if identity.form_id_0c != 0 {
                    world
                        .resource_mut::<FormsById>()
                        .remove(identity.form_id_0c);
                    release_form_id(
                        Some(&mut world.resource_mut::<DataHandler>()),
                        identity.form_id_0c,
                    )?;
                }
                world.resource_mut::<FormsById>().set_at(0x800, form);
            }
            set_identity_id(world, form, 0x800);
        }
    }
    // 0046C604..0046C612 a nonzero form ID is registered.
    let id = world
        .get::<FormIdentity>(form)
        .expect("just spawned")
        .form_id_0c;
    if id != 0 {
        world.resource_mut::<FormsById>().set_at(id, form);
    }
    Ok(form)
}

// IDA: N/A a store to +0x0C.
fn set_identity_id(world: &mut World, form: Entity, id: u32) {
    world
        .get_mut::<FormIdentity>(form)
        .expect("just spawned")
        .form_id_0c = id;
}

/// IDA: 0052EAF0 TESSkill_Init, but for the +0x10 list, which the caller clears (`ModReferences`): +0x2C = -1, +0x34 = +0x30 = 0, +0x38 = +0x3C = 1.0, the four level descriptions' vt+0, vt+0x90(0). 0046C640 with 0 clears flag 2, after taking the form out of the 00B06150 array;
/// that array stays empty while `load_form_from_file` stops on an Active file, the only way in at startup. Completion: 100% of the store order.
pub fn init_skill(
    skill: &mut TesSkill,
    identity: &mut FormIdentity,
    globals: &mut DataHandlerGlobals,
) {
    skill.skill_2c = -1;
    skill.specialization_34 = 0;
    skill.field_30 = 0;
    skill.float_38 = 1.0;
    skill.float_3c = 1.0;
    for description in &mut skill.level_descriptions_40 {
        reset_description(description, globals);
    }
    identity.flags_08 &= !2;
}

// IDA: 0052EB80 TESSkill_Constructor. Completion: 100% of the store order.
fn construct_skill(world: &mut World) -> Result<Entity, StreamError> {
    // 0052EBAA TESForm_Constructor.
    let form = construct_tes_form(world)?;
    let mut identity = *world.get::<FormIdentity>(form).expect("just spawned");
    let mut globals = world.resource_mut::<DataHandlerGlobals>();
    let mut skill = TesSkill::default();
    // 0052EBBC 0046A400 on +0x18; 0052EBC6 0046FFD0 TESTexture on +0x20: vtable A3C650, +0x04 = 0, +0x08 = +0x0A = 0, FormHeap_Free(0), then the three fields cleared again.
    skill.description_18.original_vtable = TES_DESCRIPTION_VTABLE;
    reset_description(&mut skill.description_18, &mut globals);
    skill.texture_20 = TesTexture {
        original_vtable: TES_TEXTURE_VTABLE,
        path_04: BsStringT::default(),
    };
    // 0052EBCB..0052EBDC the three TESSkill vtables.
    skill.description_18.original_vtable = TES_SKILL_DESCRIPTION_VTABLE;
    skill.texture_20.original_vtable = TES_SKILL_TEXTURE_VTABLE;
    // 0052EBE2..0052EBFD the four level descriptions.
    for description in &mut skill.level_descriptions_40 {
        description.original_vtable = TES_DESCRIPTION_VTABLE;
        reset_description(description, &mut globals);
    }
    // 0052EC01 +0x04 = 0x0B (SKIL).
    identity.form_type_04 = 0x0B;
    // 0052EC05 0052EAF0 TESSkill_Init.
    init_skill(&mut skill, &mut identity, &mut globals);
    // 0052EC0C 0046C2E0 -> 0046B090: type 0x0B takes the default arm: the 26-slot component table (00466E80) and each component's vt+0 Init (00466D80): the form (nullsub_2),
    // TESDescription +0x18 (0046A360) and TESTexture +0x20 (005A6A20: free +0x04, then +0x04 = 0, +0x0A = 0, +0x08 = 0).
    reset_description(&mut skill.description_18, &mut globals);
    skill.texture_20.path_04 = BsStringT::default();
    world
        .entity_mut(form)
        .insert((identity, OriginalVtable(TES_SKILL_VTABLE), skill));
    Ok(form)
}

// IDA: 00446D80 DataHandler_Constructor -> 00B33A98 (published at 00441B8A). Completion: 100% of the store order. +0xC0's data pointer stays 0 until a cell.
pub fn construct_data_handler(world: &mut World) -> Result<(), StreamError> {
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
) -> Result<(), StreamError> {
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
) -> Result<Option<TesFileId>, StreamError> {
    // 00447C55 a null name returns 0.
    let Some(name) = name else {
        return Ok(None);
    };
    // 00447C65 reads [this + 0x8C8].
    let data = data_handler.ok_or(StreamError(
        "00447C65: DataHandler_LookupFileByName reads [0x8C8] on a null DataHandler",
    ))?;
    for &id in &data.files_8c8 {
        // 00447C65 the first empty item ends the walk.
        let Some(file) = data.file_store.get(id) else {
            return Ok(None);
        };
        // 00447C70 _stricmp(name, item + 0x1C); +0x1C holds the name lifted from the disk, so compare the original's bytes.
        let item = recomp_core::host_path::original_bytes(&file.name_1c);
        if recomp_core::crt::stricmp(name, &item) == 0 {
            return Ok(Some(id));
        }
    }
    // 00447C7C..00447C83 a null next node returns 0.
    Ok(None)
}
