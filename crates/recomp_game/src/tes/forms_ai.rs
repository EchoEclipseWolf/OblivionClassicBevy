//! Phase 12b: the AI forms, as in the C++'s forms_ai: TESIdleForm with its tree (the children arrays and the IdleAnims roots), TESPackage with its location, target and schedule blocks, TESCombatStyle with its CSAD block, and the TESCondition list loader. A condition's form parameters stay raw here: 0056AE20, which resolves them and reads the condition function tables, only runs from InitItem, in 12d.
//!
//! Evidence: objdump of each address named here; P12-RUN-001's SAMPLE and DATA rows.

use bevy_ecs::prelude::*;
use recomp_core::crt;
use recomp_core::reader::StreamError;
use recomp_core::x87;

use super::data_handler::{DataHandler, DataHandlerGlobals, FormsById, construct_tes_form};
use super::data_load::{
    create_form_by_type, initialize_form_from_current_record, load_form_from_file,
};
use super::form::{FormIdentity, OriginalVtable};
use super::form_arms::{
    DataLoadHost, FormClass, FormLoadCx, default_editor_id, reset_14_components,
    reset_18_components,
};
use super::form_components::{ComponentSlot, FormRef, TesModel};
use super::forms_misc::{load_model_arm, set_initialized};
use super::forms_script::{last_mod_reference, latin1, map_file_form_id};
use super::game_settings::GameSettingCollection;
use super::tes_file::{BsStringT, FORM_TYPE_TABLE};

/// IDA: the form types of this family.
pub const FORM_TYPE_IDLE: u8 = 0x3C;
pub const FORM_TYPE_PACKAGE: u8 = 0x3D;
pub const FORM_TYPE_COMBAT_STYLE: u8 = 0x3E;

/// IDA: the vtables, from the constructors named below.
pub const TES_IDLE_FORM_VTABLE: u32 = 0x00A5_3774;
pub const TES_IDLE_MODEL_VTABLE: u32 = 0x00A5_3754;
pub const TES_PACKAGE_VTABLE: u32 = 0x00A6_743C;
pub const TES_COMBAT_STYLE_VTABLE: u32 = 0x00A4_07F4;
pub const NI_FORM_ARRAY_VTABLE: u32 = 0x00A5_3854;

// IDA: the chunks this family's LoadForms dispatch on.
const EDID: u32 = 0x4449_4445;
const DATA: u32 = 0x4154_4144;
const ANAM: u32 = 0x4D41_4E41;
const MODL: u32 = 0x4C44_4F4D;
const MODB: u32 = 0x4244_4F4D;
const MODT: u32 = 0x5444_4F4D;
const CTDA: u32 = 0x4144_5443;
const CTDT: u32 = 0x5444_5443;
const PKDT: u32 = 0x5444_4B50;
const PLDT: u32 = 0x5444_4C50;
const PSDT: u32 = 0x5444_5350;
const PTDT: u32 = 0x5444_5450;
const CSTD: u32 = 0x4454_5343;
const CSAD: u32 = 0x4441_5343;

// ---- TESCondition ----

/// IDA: a TESConditionItem (0x18), with 0056A970's bytes: +0x14 is the chunk-was-CTDT byte, written over what the file gave.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TesConditionItem {
    pub data_00: [u8; 0x18],
    /// IDA: +0x04 after 0056AE20 stores a TESGlobal there.
    pub comparison_resolved: Option<Option<Entity>>,
    /// IDA: +0x0C and +0x10 after 0056B2E0 stores a form there.
    pub parameters_resolved: [Option<Option<Entity>>; 2],
}

impl TesConditionItem {
    /// IDA: +0x08 the function index.
    #[must_use]
    pub fn function_08(&self) -> u16 {
        u16::from_le_bytes([self.data_00[8], self.data_00[9]])
    }

    /// IDA: +0x0C + 4 * index, the word of a parameter as loaded.
    #[must_use]
    pub fn parameter(&self, index: usize) -> u32 {
        let at = 0x0C + 4 * index;
        u32::from_le_bytes(self.data_00[at..at + 4].try_into().expect("four bytes"))
    }
}

/// IDA: TESCondition: a BSSimpleList of items. 0056A7B0 appends; 0056A750 frees.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TesCondition {
    pub items: Vec<TesConditionItem>,
}

/// IDA: 0056A7B0 TESCondition_LoadChunk(file): a CTDT or CTDA chunk becomes a FormHeap(0x18) item: 0056AE00 zeroes it and runs 0056A9C0(0x104C), which at this argument (byte 0056AACB = 4, arm 0056AA65) only stores the function index 0x4C at +0x08; 0056A970 reads the chunk with size 0x18, so a short chunk keeps that default and a long one truncates; +0x14 = the chunk was CTDT; 0067B1E0 appends. Completion: 100%.
pub fn load_condition(
    condition: &mut TesCondition,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<(), StreamError> {
    let chunk = cx.file.chunk_type()?;
    if chunk != CTDT && chunk != CTDA {
        return Ok(());
    }
    let mut data = [0_u8; 0x18];
    data[8] = 0x4C;
    cx.file.read_chunk(&mut data, 0x18)?;
    data[0x14] = u8::from(chunk == CTDT);
    condition.items.push(TesConditionItem {
        data_00: data,
        comparison_resolved: None,
        parameters_resolved: [None; 2],
    });
    Ok(())
}

// ---- NiFormArray ----

/// IDA: NiTPrimitiveArray<TESIdleForm*> (0x18): the vtable, +0x04 data (`capacity_08` entries), +0x08 capacity, +0x0C size, +0x10 count (non-null), +0x14 grow by.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NiFormArray {
    pub original_vtable: u32,
    pub data_04: Vec<Option<Entity>>,
    pub capacity_08: u32,
    pub size_0c: u32,
    pub count_10: u32,
    pub grow_14: u32,
}

impl Default for NiFormArray {
    // IDA: 005207C8 and 00521344: everything 0, +0x14 = 1, the vtable.
    fn default() -> Self {
        Self {
            original_vtable: NI_FORM_ARRAY_VTABLE,
            data_04: Vec::new(),
            capacity_08: 0,
            size_0c: 0,
            count_10: 0,
            grow_14: 1,
        }
    }
}

impl NiFormArray {
    /// IDA: 00494E90 the index of an item within the size, -1 when absent. Completion: 100%.
    #[must_use]
    pub fn find(&self, item: Option<Entity>) -> i32 {
        if item.is_none() || self.size_0c == 0 {
            return -1;
        }
        for index in 0..self.size_0c {
            if self.data_04[index as usize] == item {
                return index as i32;
            }
        }
        -1
    }

    /// IDA: 00494ED0 the item at an index, null at or past the size, unsigned. Completion: 100%.
    #[must_use]
    pub fn at(&self, index: u32) -> Option<Entity> {
        if index < self.size_0c {
            self.data_04[index as usize]
        } else {
            None
        }
    }

    /// IDA: 005A56F0 Compact: nothing when count = size. Otherwise the non-null items move down; size = capacity = count, in a new block of that size, or none. Completion: 100%.
    pub fn compact(&mut self) {
        if self.count_10 == self.size_0c {
            return;
        }
        if self.count_10 != 0 {
            let mut write = 0;
            for index in 0..self.size_0c as usize {
                if self.data_04[index].is_some() {
                    self.data_04[write] = self.data_04[index];
                    write += 1;
                }
            }
        }
        self.size_0c = self.count_10;
        self.capacity_08 = self.count_10;
        self.data_04.truncate(self.count_10 as usize);
    }

    /// IDA: 00452910 SetCapacity: nothing when equal. Below the size, the items past it are cleared with the count following, and the size drops. A new block, the items copied, the rest null. Completion: 100%.
    pub fn set_capacity(&mut self, capacity: u32) {
        if capacity == self.capacity_08 {
            return;
        }
        if capacity < self.size_0c {
            for index in capacity..self.size_0c {
                if self.data_04[index as usize].take().is_some() {
                    self.count_10 = self.count_10.wrapping_sub(1);
                }
            }
            self.size_0c = capacity;
        }
        self.capacity_08 = capacity;
        self.data_04.resize(capacity as usize, None);
    }

    /// IDA: 00446C50 SetAt: at or past the size, the size becomes index + 1; the count follows the slot's null-ness. Completion: 100%.
    pub fn set_at(&mut self, index: u32, item: Option<Entity>) {
        if index >= self.size_0c {
            self.size_0c = index.wrapping_add(1);
            if item.is_some() {
                self.count_10 = self.count_10.wrapping_add(1);
            }
        } else {
            let filled = self.data_04[index as usize].is_some();
            if item.is_some() && !filled {
                self.count_10 = self.count_10.wrapping_add(1);
            } else if item.is_none() && filled {
                self.count_10 = self.count_10.wrapping_sub(1);
            }
        }
        self.data_04[index as usize] = item;
    }

    /// IDA: 0052F3C0 InsertAt(index, item): 005A56F0, then past the size SetAt (growing to +0x14 + index first when that's at or past the capacity); otherwise growing to +0x14 + size + 1 when full, shifting up, size + 1, count = size. Completion: 100%.
    pub fn insert(&mut self, index: u32, item: Entity) {
        self.compact();
        if index > self.size_0c {
            if index >= self.capacity_08 {
                self.set_capacity(self.grow_14.wrapping_add(index));
            }
            self.set_at(index, Some(item));
            return;
        }
        let needed = self.size_0c + 1;
        if self.capacity_08 < needed {
            self.set_capacity(self.grow_14.wrapping_add(needed));
        }
        self.data_04
            .copy_within(index as usize..self.size_0c as usize, index as usize + 1);
        self.size_0c += 1;
        self.count_10 = self.size_0c;
        self.data_04[index as usize] = Some(item);
    }

    /// IDA: 005304C0 RemoveAt(index): within the size, the slot nulled with the count following, and the size drops when it was the last. Completion: 100%.
    pub fn remove_at(&mut self, index: u32) {
        if index >= self.size_0c {
            return;
        }
        if self.data_04[index as usize].take().is_some() {
            self.count_10 = self.count_10.wrapping_sub(1);
        }
        if index == self.size_0c - 1 {
            self.size_0c = index;
        }
    }
}

// ---- the IdleAnims roots ----

/// IDA: 00521340 IDLE_ANIM_ROOT (0x24): the NiFormArray of top-level idles, +0x18 BSStringT key, +0x20 the KF file list 00435830 found, or null.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IdleAnimRoot {
    pub idles: NiFormArray,
    pub path_18: BsStringT,
    pub kf_files_20: Option<Vec<String>>,
}

/// IDA: the entries of the 00B362C0 map, whose head is `DataHandlerGlobals::idle_anim_roots_b362c0` (count +0x0C). Host substitution: lookups by _stricmp over this insertion-ordered `Vec`, where the original hashes into 37 buckets. No key can match twice, so a lookup can't tell.
#[derive(Resource, Clone, Debug, Default, PartialEq, Eq)]
pub struct IdleAnimRoots {
    pub roots: Vec<IdleAnimRoot>,
    /// IDA: the map's 37 buckets (hash 004A80A0, +0x04 = 37): indexes into `roots`, each chain head first. 00412D30 puts a new key at the head.
    pub buckets_08: Vec<Vec<usize>>,
}

impl IdleAnimRoots {
    /// IDA: 00452600's walk: the buckets in index order, each chain from its head. Completion: 100%.
    pub fn walk(&self) -> impl Iterator<Item = &IdleAnimRoot> + '_ {
        self.buckets_08
            .iter()
            .flatten()
            .map(|&index| &self.roots[index])
    }
}

/// IDA: 00521370 the root key of a model path (the buffer is 0x100 bytes on the stack, which a longer path overruns in the original): empty for a null or empty path; up to and including the first `strstr` "IdleAnims" (case-sensitive: 009840E0), or else the text to the last '\' plus "IdleAnims", or else empty. Completion: 100%.
#[must_use]
pub fn idle_root_key(path: &[u8]) -> BsStringT {
    const NAME: &[u8] = b"IdleAnims";
    let mut out = BsStringT::default();
    if path.is_empty() {
        return out;
    }
    if let Some(at) = path.windows(NAME.len()).position(|part| part == NAME) {
        out.set(Some(&path[..at + NAME.len()]), 0);
        return out;
    }
    if let Some(at) = path.iter().rposition(|&byte| byte == b'\\') {
        out.set(Some(&path[..=at]), 0);
        // 00412FA0 the append.
        let mut joined = out.data_00.clone().unwrap_or_default();
        joined.extend_from_slice(NAME);
        out.set(Some(&joined), 0);
    }
    out
}

// IDA: 004A80A0 vt+4, as `EditorIdMap::hash`, modulo the map's 37.
fn idle_root_bucket(key: &[u8]) -> usize {
    (super::data_handler::case_insensitive_hash(key) % 37) as usize
}

// IDA: 0055E000 on the map: the root of a key, _stricmp, in its bucket's chain.
fn lookup_root(roots: &IdleAnimRoots, key: &[u8]) -> Option<usize> {
    let bucket = idle_root_bucket(key);
    roots
        .buckets_08
        .get(bucket)?
        .iter()
        .copied()
        .find(|&index| {
            roots.roots[index]
                .path_18
                .data_00
                .as_deref()
                .is_some_and(|name| crt::stricmp(name, key) == 0)
        })
}

// IDA: 00435830(key, 1) (ecx = 00B33A1C, unused). 00521190 first: the key's last component replaced by "IdleAnims" (a key with no '\' writes to address 1), and that root's +0x20 list, if it has one, comes back copied into a new list, each copy prepended, so reversed. Otherwise "Data\Meshes\" + key, its last component replaced by "\*.KF", through 00431970(pattern, key, 1, 0) (`DataLoadHost::find_matching_files`), which answers null for an empty list. Completion: 100%.
fn find_idle_animations(
    world: &World,
    key: &[u8],
    host: &mut dyn DataLoadHost,
) -> Result<Option<Vec<String>>, StreamError> {
    let at = key
        .iter()
        .rposition(|&byte| byte == b'\\')
        .ok_or(StreamError(
            "005211E2: 00521190 on a key with no '\\' faults",
        ))?;
    let mut sibling = key[..=at].to_vec();
    sibling.extend_from_slice(b"IdleAnims");
    if let Some(roots) = world.get_resource::<IdleAnimRoots>()
        && let Some(index) = lookup_root(roots, &sibling)
        && let Some(files) = &roots.roots[index].kf_files_20
    {
        let mut copy = files.clone();
        copy.reverse();
        return Ok(Some(copy));
    }
    let key_text = latin1(key);
    let mut pattern = format!("Data\\Meshes\\{key_text}");
    let slash = pattern.rfind('\\').expect("the prefix has one");
    pattern.truncate(slash);
    pattern.push_str("\\*.KF");
    let found = host.find_matching_files(&pattern, &key_text, 1);
    Ok(if found.is_empty() { None } else { Some(found) })
}

/// IDA: 00435830 IdleAnimRoot_FindKFFiles(key, 0) as an actor's animation setup reaches it (004E3A9A): 00521190's root, whose +0x20 list comes back copied, each copy prepended (so reversed), as `find_idle_animations`'s. Completion: 100% of the root arm; with no root, or a root with no list, the "Data\Meshes\...\*.KF" FileFinder arm (00431970) is a stop here.
/// # Errors
/// A key with no '\', or the FileFinder arm.
pub fn root_kf_files(world: &World, key: &[u8]) -> Result<Option<Vec<String>>, StreamError> {
    let at = key
        .iter()
        .rposition(|&byte| byte == b'\\')
        .ok_or(StreamError(
            "005211E2: 00521190 on a key with no '\\' faults",
        ))?;
    let mut sibling = key[..=at].to_vec();
    sibling.extend_from_slice(b"IdleAnims");
    if let Some(roots) = world.get_resource::<IdleAnimRoots>()
        && let Some(index) = lookup_root(roots, &sibling)
        && let Some(files) = &roots.roots[index].kf_files_20
    {
        let mut copy = files.clone();
        copy.reverse();
        return Ok(Some(copy));
    }
    Err(StreamError(
        "00435879: an actor's KF files through the FileFinder",
    ))
}

/// IDA: 005215C0 IdleAnimRootMap_FindOrCreate(key) (__thiscall on 00B362C0), which Phase 14 reaches too: a null or empty key answers null; a found root answers; otherwise FormHeap(0x24) 00521340, 004028D0(+0x18, key, 0), 00452910(1), +0x14 = 1, +0x20 = 00435830(key, 1), and the map insert. Answers the root's index in `IdleAnimRoots`. Completion: 100%, with the map's buckets as above.
pub fn find_or_create_idle_root(
    world: &mut World,
    key: &[u8],
    host: &mut dyn DataLoadHost,
) -> Result<Option<usize>, StreamError> {
    if key.is_empty() {
        return Ok(None);
    }
    if world
        .resource::<DataHandlerGlobals>()
        .idle_anim_roots_b362c0
        .is_none()
    {
        return Err(StreamError("00521607: 005215C0 on a null 00B362C0"));
    }
    if let Some(index) = lookup_root(world.get_resource_or_init::<IdleAnimRoots>().as_ref(), key) {
        return Ok(Some(index));
    }
    let mut root = IdleAnimRoot::default();
    root.path_18.set(Some(key), 0);
    root.idles.set_capacity(1);
    root.idles.grow_14 = 1;
    root.kf_files_20 = find_idle_animations(world, key, host)?;
    let mut roots = world.resource_mut::<IdleAnimRoots>();
    roots.roots.push(root);
    let index = roots.roots.len() - 1;
    if roots.buckets_08.is_empty() {
        roots.buckets_08 = vec![Vec::new(); 37];
    }
    roots.buckets_08[idle_root_bucket(key)].insert(0, index);
    let mut globals = world.resource_mut::<DataHandlerGlobals>();
    let map = globals
        .idle_anim_roots_b362c0
        .as_mut()
        .expect("tested above");
    map.count_0c = map.count_0c.wrapping_add(1);
    Ok(Some(index))
}

// IDA: 0046D3F0 on an idle: the TESModel cast, then vt+0x14 of A53754, the +0x04 text or `Default`; and 00521370.
fn idle_key(world: &World, form: Entity) -> BsStringT {
    let path = world
        .get::<TesModel>(form)
        .expect("an idle")
        .path_04
        .data_00
        .clone()
        .unwrap_or_default();
    idle_root_key(&path)
}

// ---- TESIdleForm ----

/// IDA: 00520290 TESIdleForm (0x48): TESModelAnim +0x18 (`TesModel` on the same entity), +0x30 TESCondition, +0x38 the animation group (ANAM), +0x3C the children (null until a child), +0x40 the parent, +0x44 the previous idle.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct TesIdleForm {
    pub conditions_30: TesCondition,
    pub group_38: u8,
    pub children_3c: Option<NiFormArray>,
    pub parent_40: Option<Entity>,
    pub previous_44: Option<Entity>,
}

// IDA: N/A a store to another idle's +0x44 through the `__RTDynamicCast` to TESIdleForm (b06514).
fn set_previous(world: &mut World, form: Option<Entity>, previous: Option<Entity>) {
    if let Some(mut idle) = form.and_then(|form| world.get_mut::<TesIdleForm>(form)) {
        idle.previous_44 = previous;
    }
}

// IDA: N/A g_formsByID then the cast to TESIdleForm.
fn lookup_idle(world: &World, id: u32) -> Option<Entity> {
    world
        .resource::<FormsById>()
        .lookup(id)
        .filter(|&form| world.get::<TesIdleForm>(form).is_some())
}

// IDA: 0046C700 vt+0x74, "%s Form '%s' (%08X)" with the table's name and the literal `Default` (empty) for the editor ID. Completion: 100%.
pub fn describe(world: &World, form: Entity) -> String {
    let identity = world.get::<FormIdentity>(form).expect("a form");
    let name = FORM_TYPE_TABLE
        .get(usize::from(identity.form_type_04))
        .map_or("", |(_, name)| *name);
    format!("{name} Form '' ({:08X})", identity.form_id_0c)
}

/// IDA: 00520780 TESIdleForm_AddChild(index, child) on the parent: a null +0x3C gets a new array; the index clamps to the size; the child's parent; its previous is the item before, or null at 0; the item at the index takes the child as previous; 0052F3C0. The null-child return can't arise here. Completion: 100%.
pub fn add_idle_child(world: &mut World, parent: Entity, index: u32, child: Entity) {
    let mut array = world
        .get_mut::<TesIdleForm>(parent)
        .expect("a parent idle")
        .children_3c
        .take()
        .unwrap_or_default();
    let slot = index.min(array.size_0c);
    let previous = if slot != 0 { array.at(slot - 1) } else { None };
    {
        let mut idle = world.get_mut::<TesIdleForm>(child).expect("an idle child");
        idle.parent_40 = Some(parent);
        idle.previous_44 = previous;
    }
    set_previous(world, array.at(slot), Some(child));
    array.insert(slot, child);
    world
        .get_mut::<TesIdleForm>(parent)
        .expect("a parent idle")
        .children_3c = Some(array);
}

// IDA: 005201E0 TESIdleForm vt+0x14: 0046C2E0, +0x38 = 4, +0x40 = +0x44 = 0. +0x3C stays. Completion: 100%.
fn reset_idle(world: &mut World, form: Entity) -> Result<(), StreamError> {
    reset_14_components(world, form)?;
    let mut idle = world.get_mut::<TesIdleForm>(form).expect("an idle");
    idle.group_38 = 4;
    idle.parent_40 = None;
    idle.previous_44 = None;
    Ok(())
}

// IDA: 005204F0 on the parent, with the child's index (-1 when 00494E90 misses): the item past it takes the item before it as previous; within the size, 005304C0's stores; 005A56F0. Completion: 100%.
fn unlink_from_parent(world: &mut World, parent: Entity, index: u32) {
    let mut array = world
        .get_mut::<TesIdleForm>(parent)
        .expect("a parent idle")
        .children_3c
        .take()
        .expect("the caller tested +0x3C");
    let next = array.at(index.wrapping_add(1));
    let previous = array.at(index.wrapping_sub(1));
    set_previous(world, next, previous);
    array.remove_at(index);
    array.compact();
    world
        .get_mut::<TesIdleForm>(parent)
        .expect("a parent idle")
        .children_3c = Some(array);
}

// IDA: 00520620 TESIdleForm vt+0x18: 0046C2F0, 0056A750, then with DataHandler +0xCD4 clear: a parent with +0x3C runs 005204F0(00494E90(this)), or with no parent 00521730 finds the root (no create), and when 00494E90 finds this: the item past it takes this one's +0x44, 005304C0, 005A56F0. Completion: 100%.
fn destroy_idle(world: &mut World, form: Entity) -> Result<(), StreamError> {
    reset_18_components(world, form)?;
    let (parent, previous) = {
        let mut idle = world.get_mut::<TesIdleForm>(form).expect("an idle");
        idle.conditions_30.items.clear();
        (idle.parent_40, idle.previous_44)
    };
    if world.resource::<DataHandler>().bytes_cd0[4] != 0 {
        return Ok(());
    }
    if let Some(parent) = parent {
        let found = world
            .get::<TesIdleForm>(parent)
            .expect("a parent idle")
            .children_3c
            .as_ref()
            .map(|array| array.find(Some(form)));
        if let Some(found) = found {
            unlink_from_parent(world, parent, found as u32);
        }
        return Ok(());
    }
    let key = idle_key(world, form);
    let Some(key) = key.data_00.filter(|key| !key.is_empty()) else {
        return Ok(());
    };
    let roots = world.get_resource_or_init::<IdleAnimRoots>();
    let Some(root) = lookup_root(roots.as_ref(), &key) else {
        return Ok(());
    };
    let found = roots.roots[root].idles.find(Some(form));
    if found == -1 {
        return Ok(());
    }
    let index = found as u32;
    let next = roots.roots[root].idles.at(index + 1);
    set_previous(world, next, previous);
    let mut roots = world.resource_mut::<IdleAnimRoots>();
    let idles = &mut roots.roots[root].idles;
    idles.remove_at(index);
    idles.compact();
    Ok(())
}

// IDA: 00520290 TESIdleForm_Constructor: 0046C460, 0046D7E0 on +0x18, its A53734, the two vtables, 00434920 on +0x30, +0x04, +0x3C = 0, 0046C2E0, +0x38 = 4, +0x40 = +0x44 = 0. Completion: 100% of the store order.
fn construct_idle(world: &mut World) -> Result<Entity, StreamError> {
    let form = construct_tes_form(world)?;
    world.entity_mut(form).insert((
        OriginalVtable(TES_IDLE_FORM_VTABLE),
        TesModel {
            original_vtable: TES_IDLE_MODEL_VTABLE,
            ..TesModel::default()
        },
        TesIdleForm::default(),
    ));
    world
        .get_mut::<FormIdentity>(form)
        .expect("a form")
        .form_type_04 = FORM_TYPE_IDLE;
    reset_idle(world, form)?;
    Ok(form)
}

// IDA: 00520A08..00520C42 the DATA chunk: two IDs (8 bytes over zeros). Each nonzero one maps through 0046BB20 with the form's last file, looks up and casts, warning when that fails with the mapped ID and vt+0x74. +0x40 and +0x44 take the results. With a parent: the index is 0 with no previous and flag bit 5 clear, -1 with it set, or the previous's index in the parent's children + 1 (-1 stays -1, and so does a null +0x3C); 00520780. With no parent: 005216A0 the root (null: done). The index is 0 or the previous's index + 1 with -1 kept. An empty root with index 0 inserts straight away. Otherwise a nonzero index clamps to the size, or a zero one with bit 5 set becomes the size; the item there takes this as previous, and +0x44 becomes the item before it. 0052F3C0. Completion: 100%.
fn load_idle_data(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<(), StreamError> {
    let mut bytes = [0_u8; 8];
    cx.file.read_chunk(&mut bytes, 8)?;
    let parent_id = u32::from_le_bytes(bytes[..4].try_into().expect("four bytes"));
    let previous_id = u32::from_le_bytes(bytes[4..].try_into().expect("four bytes"));
    let mut parent = None;
    if parent_id != 0 {
        let file = last_mod_reference(world, form);
        let id = map_file_form_id(world, cx, parent_id, file)?;
        parent = lookup_idle(world, id);
        if parent.is_none() {
            let text = describe(world, form);
            cx.file
                .services
                .warn(format!("Could not find parent idle ({id:08X}) for {text}."));
        }
    }
    world
        .get_mut::<TesIdleForm>(form)
        .expect("an idle")
        .parent_40 = parent;
    let mut previous = None;
    if previous_id != 0 {
        let file = last_mod_reference(world, form);
        let id = map_file_form_id(world, cx, previous_id, file)?;
        previous = lookup_idle(world, id);
        if previous.is_none() {
            let text = describe(world, form);
            cx.file.services.warn(format!(
                "Could not find previous idle ({id:08X}) for {text}."
            ));
        }
    }
    world
        .get_mut::<TesIdleForm>(form)
        .expect("an idle")
        .previous_44 = previous;
    let flag_5 = world.get::<FormIdentity>(form).expect("a form").flags_08 & 0x20 != 0;
    if let Some(parent) = parent {
        let mut index = 0_u32;
        if previous.is_some() {
            let mut found = -1;
            if let Some(children) = &world
                .get::<TesIdleForm>(parent)
                .expect("a parent idle")
                .children_3c
            {
                found = children.find(previous);
            }
            if found != -1 {
                found += 1;
            }
            index = found as u32;
        } else if flag_5 {
            index = u32::MAX;
        }
        add_idle_child(world, parent, index, form);
        return Ok(());
    }
    let key = idle_key(world, form);
    let Some(root) = find_or_create_idle_root(
        world,
        key.data_00.as_deref().unwrap_or_default(),
        &mut *cx.host,
    )?
    else {
        return Ok(());
    };
    let (neighbours, index) = {
        let idles = &world.resource::<IdleAnimRoots>().roots[root].idles;
        let mut index = 0_u32;
        if previous.is_some() {
            let mut found = idles.find(previous);
            if found != -1 {
                found += 1;
            }
            index = found as u32;
        }
        let size = idles.size_0c;
        if size == 0 && index == 0 {
            (None, 0)
        } else {
            if index != 0 {
                index = index.min(size);
            } else if flag_5 {
                index = size;
            }
            (
                Some((idles.at(index), idles.at(index.wrapping_sub(1)))),
                index,
            )
        }
    };
    if let Some((next, before)) = neighbours {
        set_previous(world, next, Some(form));
        world
            .get_mut::<TesIdleForm>(form)
            .expect("an idle")
            .previous_44 = before;
    }
    world.resource_mut::<IdleAnimRoots>().roots[root]
        .idles
        .insert(index, form);
    Ok(())
}

// IDA: 00520970 TESIdleForm vt+0x1C LoadForm. The record must be IDLE (else 0, a full compare here); 00451210, with no 0046AB80. The chunks: ANAM 1 byte into +0x38; MODL, MODB and MODT 0046DB00; CTDA and CTDT 0056A7B0; EDID (alloca, size 0x200, vt+0xD8 006FDFF0 keeps nothing); DATA above. Returns 1. Completion: 100%.
fn load_idle_form(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<bool, StreamError> {
    if cx.file.record_form_type()? != u32::from(FORM_TYPE_IDLE) {
        return Ok(false);
    }
    initialize_form_from_current_record(world, form, cx)?;
    let mut chunk = cx.file.chunk_type()?;
    while chunk != 0 {
        match chunk {
            ANAM => {
                let mut idle = world.get_mut::<TesIdleForm>(form).expect("an idle");
                let mut byte = [idle.group_38, 0];
                cx.file.read_chunk(&mut byte, 1)?;
                idle.group_38 = byte[0];
            }
            MODL | MODB | MODT => load_model_arm(world, form, cx)?,
            CTDA | CTDT => load_condition(
                &mut world
                    .get_mut::<TesIdleForm>(form)
                    .expect("an idle")
                    .conditions_30,
                cx,
            )?,
            EDID => {
                cx.file.read_text(0x200)?;
            }
            DATA => load_idle_data(world, form, cx)?,
            _ => {}
        }
        if !cx.file.advance_chunk()? {
            break;
        }
        chunk = cx.file.chunk_type()?;
    }
    Ok(true)
}

// IDA: 0044E556 the IDLE arm of 0044DCF0: an existing form goes through 00447050 at 0044E291; otherwise 00447DF0 and 00447050. No list add. Returns 1. A null form faults in 00447050: a host stop. Completion: 100%.
fn idle_arm(
    world: &mut World,
    cx: &mut FormLoadCx<'_, '_>,
    existing: Option<Entity>,
    _first: bool,
) -> Result<u32, StreamError> {
    let form = match existing {
        Some(form) => form,
        None => create_form_by_type(world, u32::from(FORM_TYPE_IDLE), cx)?
            .ok_or(StreamError("0044E566: 00447050 on a null form faults"))?,
    };
    load_form_from_file(world, form, cx)?;
    Ok(1)
}

// ---- TESPackage ----

/// IDA: a package location (0x0C): +0x00 type (signed), +0x04 radius, +0x08 object (a raw ID until InitItem). +0x01..+0x03 are never stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PackageLocation {
    pub type_00: i8,
    pub radius_04: u32,
    pub object_08: FormRef,
}

impl Default for PackageLocation {
    // IDA: 005696B0: type 3, the rest 0.
    fn default() -> Self {
        Self {
            type_00: 3,
            radius_04: 0,
            object_08: FormRef::Raw(0),
        }
    }
}

impl PackageLocation {
    /// IDA: 00569750 PackageLocation_SetType: a new type is stored; type 1 clears +0x08 and +0x04, types 0 and 2..5 clear +0x08, and any other clears nothing (the 00569784 table). Completion: 100%.
    pub const fn set_type(&mut self, kind: i8) {
        if kind == self.type_00 {
            return;
        }
        self.type_00 = kind;
        match kind {
            1 => {
                self.object_08 = FormRef::Raw(0);
                self.radius_04 = 0;
            }
            0 | 2..=5 => self.object_08 = FormRef::Raw(0),
            _ => {}
        }
    }

    /// IDA: 005697A0 the radius: 0 for types -1 and 1. Completion: 100%.
    #[must_use]
    pub const fn radius(&self) -> u32 {
        if self.type_00 == -1 || self.type_00 == 1 {
            0
        } else {
            self.radius_04
        }
    }

    /// IDA: 005697C0: stored unless the type is -1 or 1. Completion: 100%.
    pub const fn set_radius(&mut self, radius: u32) {
        if self.type_00 != -1 && self.type_00 != 1 {
            self.radius_04 = radius;
        }
    }

    /// IDA: 00569840 PackageLocation_Assign(source, 0): a source of type -1 returns; 00569750 with its type; the 00569928 table: 0, 4 and 5 take the radius and the object (0 unless the types match, as they now do); 1 the object alone; 2 and 3 the radius alone. Any other type copies nothing, so the temporary's stack bytes (see `from_pldt`) are never read. Completion: 100%.
    pub const fn assign(&mut self, source: &Self) {
        if source.type_00 == -1 {
            return;
        }
        self.set_type(source.type_00);
        match self.type_00 {
            0 | 4 | 5 => {
                self.set_radius(source.radius());
                self.object_08 = source.object_08;
            }
            1 => self.object_08 = source.object_08,
            2 | 3 => self.set_radius(source.radius()),
            _ => {}
        }
    }

    /// IDA: 005696C0 the temporary from PLDT's {type, object, radius}: type, then +0x04 = the third word; the 0056970C table: 0, 4 and 5 +0x08 = the second word; 1 the same with +0x04 = 0; 2 and 3 +0x08 = 0. A type outside 0..5 leaves +0x08 as stack bytes, host zero: `assign` never reads them. The C++ stopped there. Completion: 100%.
    #[must_use]
    pub const fn from_pldt(words: [u32; 3]) -> Self {
        let type_00 = words[0] as u8 as i8;
        let mut location = Self {
            type_00,
            radius_04: words[2],
            object_08: FormRef::Raw(0),
        };
        match type_00 {
            1 => {
                location.object_08 = FormRef::Raw(words[1]);
                location.radius_04 = 0;
            }
            0 | 4 | 5 => location.object_08 = FormRef::Raw(words[1]),
            _ => {}
        }
        location
    }
}

/// IDA: a package target (0x0C): +0x00 type, +0x04 object or type value (raw until InitItem), +0x08 count. +0x01..+0x03 are never stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PackageTarget {
    pub type_00: u8,
    pub value_04: FormRef,
    pub count_08: u32,
}

impl Default for PackageTarget {
    // IDA: 00569E40: type 2, the rest 0.
    fn default() -> Self {
        Self {
            type_00: 2,
            value_04: FormRef::Raw(0),
            count_08: 0,
        }
    }
}

impl PackageTarget {
    /// IDA: 00569EF0 over the 0056A0A0 temporary, which comes down to this: a new type is stored and types 0..=2 clear +0x04; types 0..=2 then take PTDT's second word; +0x08 takes the third. A type over 2 keeps +0x04, so the temporary's stack bytes are never read. The C++ stopped there. Completion: 100%.
    pub const fn assign_ptdt(&mut self, words: [u32; 3]) {
        let kind = words[0] as u8;
        if kind != self.type_00 {
            self.type_00 = kind;
            if kind <= 2 {
                self.value_04 = FormRef::Raw(0);
            }
        }
        if kind <= 2 {
            self.value_04 = FormRef::Raw(words[1]);
        }
        self.count_08 = words[2];
    }
}

/// IDA: 00569D60 a package schedule (8): month, day, date, time bytes, +0x04 duration: FF FF 00 FF, 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PackageSchedule {
    pub data_00: [u8; 4],
    pub duration_04: u32,
}

impl Default for PackageSchedule {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            data_00: [0xFF, 0xFF, 0x00, 0xFF],
            duration_04: 0,
        }
    }
}

/// IDA: 00568E10 TESPackage (0x3C): +0x18 (-1), +0x1C flags, +0x20 type (signed; +0x21..+0x23 are never stored), +0x24 the location, +0x28 the target, +0x2C the schedule, +0x34 TESCondition.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct TesPackage {
    pub field_18: u32,
    pub flags_1c: u32,
    pub type_20: i8,
    pub location_24: Option<PackageLocation>,
    pub target_28: Option<PackageTarget>,
    pub schedule_2c: PackageSchedule,
    pub conditions_34: TesCondition,
}

impl TesPackage {
    /// IDA: 005661E0 TESPackage_SetType(type): the same type returns. Types 3..=6 free the target; 7 frees the location. The 00566368 byte table over 0..=9 (unsigned, so a negative type takes no arm): 0, 1, 7, 8, 9 make a target; 2 a target and a location; 5, 6 a location; 3, 4 nothing. The type is stored. Completion: 100%.
    pub fn set_type(&mut self, kind: i32) {
        if kind == i32::from(self.type_20) {
            return;
        }
        if (3..=6).contains(&kind) {
            self.target_28 = None;
        } else if kind == 7 {
            self.location_24 = None;
        }
        match kind {
            0 | 1 | 7 | 8 | 9 => {
                self.target_28.get_or_insert_with(PackageTarget::default);
            }
            2 => {
                self.target_28.get_or_insert_with(PackageTarget::default);
                self.location_24
                    .get_or_insert_with(PackageLocation::default);
            }
            5 | 6 => {
                self.location_24
                    .get_or_insert_with(PackageLocation::default);
            }
            _ => {}
        }
        self.type_20 = kind as i8;
    }

    // IDA: the 00568730 stores, in order: +0x1C, +0x20, +0x24, +0x28, the schedule, +0x18, and a created ID (00446B80: at or above 0xFF000000) clears flag 0x800 of the +0x1C just zeroed.
    fn reset(&mut self, form_id: u32) {
        self.flags_1c = 0;
        self.type_20 = 0;
        self.location_24 = None;
        self.target_28 = None;
        self.schedule_2c = PackageSchedule::default();
        self.field_18 = 0xFFFF_FFFF;
        if form_id >= 0xFF00_0000 {
            self.flags_1c &= !0x800;
        }
    }
}

// IDA: 00568730 TESPackage vt+0x14, with no component pass. Completion: 100%.
fn reset_package(world: &mut World, form: Entity) -> Result<(), StreamError> {
    let id = world.get::<FormIdentity>(form).expect("a form").form_id_0c;
    world
        .get_mut::<TesPackage>(form)
        .expect("a package")
        .reset(id);
    Ok(())
}

// IDA: 00566190 TESPackage vt+0x18: the location, the target (freed, not nulled: only vt+0x14 follows), 0056A750, then a jump to 0046C2F0, which skips form type 0x3D. Completion: 100%.
fn destroy_package(world: &mut World, form: Entity) -> Result<(), StreamError> {
    {
        let mut package = world.get_mut::<TesPackage>(form).expect("a package");
        package.location_24 = None;
        package.target_28 = None;
        package.conditions_34.items.clear();
    }
    reset_18_components(world, form)
}

// IDA: 00568E10 TESPackage_Constructor: 0046C460, the vtable, 00569D60, 00434920, +0x04, 00568730. Completion: 100% of the store order.
fn construct_package(world: &mut World) -> Result<Entity, StreamError> {
    let form = construct_tes_form(world)?;
    world
        .entity_mut(form)
        .insert((OriginalVtable(TES_PACKAGE_VTABLE), TesPackage::default()));
    world
        .get_mut::<FormIdentity>(form)
        .expect("a form")
        .form_type_04 = FORM_TYPE_PACKAGE;
    reset_package(world, form)?;
    Ok(form)
}

// IDA: 3 dwords over zeros, read with size 0x0C.
fn read_three_words(cx: &mut FormLoadCx<'_, '_>) -> Result<[u32; 3], StreamError> {
    let mut bytes = [0_u8; 0x0C];
    cx.file.read_chunk(&mut bytes, 0x0C)?;
    let word = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().expect("four bytes"));
    Ok([word(0), word(4), word(8)])
}

// IDA: 00566530 TESPackage vt+0x1C LoadForm. The record must be PACK (the low byte); 00451210. Do-while over 0044FEA0, so with no 0 test on the type. PKDT: size 4, a word of flags and a signed byte of type; size 8, a dword and a signed byte at +4; any other warns "File '%s' contains package data of unrecognized type.", clears the flags, and a nonzero type makes a target if none (00569E40) and becomes 0 with no 005661E0. PLDT: 005696C0, 00565E00 (a location if none, then 00569840(temp, 0)). PSDT: 8 bytes over zeros, 00569D80 and 00565F80 copy them to +0x2C. PTDT: 0056A0A0, 00565EC0 (a target if none, then 00569EF0). CTDA and CTDT 0056A7B0. EDID. Then 0046AB80(0). Returns 1. Completion: 100%.
fn load_package_form(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<bool, StreamError> {
    if cx.file.record_form_type()? as u8 != FORM_TYPE_PACKAGE {
        return Ok(false);
    }
    initialize_form_from_current_record(world, form, cx)?;
    loop {
        match cx.file.chunk_type()? {
            PKDT => {
                let size = cx.file.chunk_size()?;
                let mut package = world.get_mut::<TesPackage>(form).expect("a package");
                if size == 4 {
                    let mut bytes = [0_u8; 4];
                    cx.file.read_chunk(&mut bytes, 4)?;
                    package.flags_1c = u32::from(u16::from_le_bytes([bytes[0], bytes[1]]));
                    package.set_type(i32::from(bytes[2] as i8));
                } else if size == 8 {
                    let mut bytes = [0_u8; 8];
                    cx.file.read_chunk(&mut bytes, 8)?;
                    package.flags_1c =
                        u32::from_le_bytes(bytes[..4].try_into().expect("four bytes"));
                    package.set_type(i32::from(bytes[4] as i8));
                } else {
                    let name = cx.file.file()?.name_1c.clone();
                    cx.file.services.warn(format!(
                        "File '{name}' contains package data of unrecognized type."
                    ));
                    let typed = package.type_20 != 0;
                    package.flags_1c = 0;
                    if typed {
                        package.target_28.get_or_insert_with(PackageTarget::default);
                        package.type_20 = 0;
                    }
                }
            }
            PLDT => {
                let temporary = PackageLocation::from_pldt(read_three_words(cx)?);
                world
                    .get_mut::<TesPackage>(form)
                    .expect("a package")
                    .location_24
                    .get_or_insert_with(PackageLocation::default)
                    .assign(&temporary);
            }
            PSDT => {
                let mut bytes = [0_u8; 8];
                cx.file.read_chunk(&mut bytes, 8)?;
                world
                    .get_mut::<TesPackage>(form)
                    .expect("a package")
                    .schedule_2c = PackageSchedule {
                    data_00: [bytes[0], bytes[1], bytes[2], bytes[3]],
                    duration_04: u32::from_le_bytes(bytes[4..].try_into().expect("four bytes")),
                };
            }
            PTDT => {
                let words = read_three_words(cx)?;
                world
                    .get_mut::<TesPackage>(form)
                    .expect("a package")
                    .target_28
                    .get_or_insert_with(PackageTarget::default)
                    .assign_ptdt(words);
            }
            CTDA | CTDT => load_condition(
                &mut world
                    .get_mut::<TesPackage>(form)
                    .expect("a package")
                    .conditions_34,
                cx,
            )?,
            EDID => {
                cx.file.read_text(0x200)?;
            }
            _ => {}
        }
        if !cx.file.advance_chunk()? {
            break;
        }
    }
    set_initialized(world, form, false);
    Ok(true)
}

// ---- TESCombatStyle ----

/// IDA: 004ABB40 TESCombatStyle (0x98): the 0x7C CSTD block +0x18..+0x93, and +0x94 the 0x54 CSAD block.
#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct TesCombatStyle {
    pub data_18: [u8; 0x7C],
    pub advanced_94: Option<[u8; 0x54]>,
    /// IDA: N/A a CSTD has memset the block: 004A9A00 alone leaves +0x1A, +0x1B, +0x3E, +0x3F, +0x4D..+0x4F, +0x5D..+0x5F, +0x6A, +0x6B and +0x89..+0x8B as FormHeap bytes.
    pub data_written: bool,
}

// IDA: N/A a static GameSetting's +0 at 00B35464 + offset, which CSTY reads directly.
fn setting(world: &World, offset: u32) -> Result<u32, StreamError> {
    world
        .get_resource::<GameSettingCollection>()
        .ok_or(StreamError("00B35464: no GameSettingCollection"))?
        .raw_at(0x00B3_5464 + offset)
        .ok_or(StreamError("00B35464: no static GameSetting here"))
}

// IDA: `movzx`, `mov byte`: a setting's low byte, at a form offset of the block.
fn store_setting_byte(
    world: &World,
    data: &mut [u8; 0x7C],
    at: usize,
    offset: u32,
) -> Result<(), StreamError> {
    data[at - 0x18] = setting(world, offset)? as u8;
    Ok(())
}

// IDA: `fld`, `fstp`: a setting as a float, at a form offset of the block.
fn store_setting_float(
    world: &World,
    data: &mut [u8; 0x7C],
    at: usize,
    offset: u32,
) -> Result<(), StreamError> {
    let bits = x87::fld_fstp_copy(setting(world, offset)?).to_bits();
    data[at - 0x18..at - 0x14].copy_from_slice(&bits.to_le_bytes());
    Ok(())
}

// IDA: N/A a float of the block, at a form offset.
fn block_float(data: &[u8; 0x7C], at: usize) -> f32 {
    f32::from_le_bytes(data[at - 0x18..at - 0x14].try_into().expect("four bytes"))
}

// IDA: `fldz`, `fcom [m32]`, `test ah,1`, `jne`: keeps the value when 0 < it or when it's unordered (C0 is set for a NaN). The C++ wrote `!(0.0F < v)` and replaced a NaN.
fn keeps_positive(value: f32) -> bool {
    0.0 < value || value.is_nan()
}

// IDA: 004A9A00 TESCombatStyle vt+0x14: each store from its setting in the exe's order, then a jump to 0046C2E0, which finds no slots. Offsets are from 00B35464. Completion: 100% of the store order.
fn reset_combat_style(world: &mut World, form: Entity) -> Result<(), StreamError> {
    let mut data = world
        .get::<TesCombatStyle>(form)
        .expect("a combat style")
        .data_18;
    {
        let world = &*world;
        let d = &mut data;
        store_setting_byte(world, d, 0x18, 0x12C)?;
        store_setting_byte(world, d, 0x69, 0x134)?;
        store_setting_byte(world, d, 0x19, 0x13C)?;
        for (at, offset) in [
            (0x1C, 0x144),
            (0x20, 0x14C),
            (0x24, 0x154),
            (0x28, 0x15C),
            (0x2C, 0x164),
            (0x30, 0x16C),
            (0x34, 0x174),
            (0x38, 0x17C),
        ] {
            store_setting_float(world, d, at, offset)?;
        }
        store_setting_byte(world, d, 0x3C, 0x184)?;
        store_setting_byte(world, d, 0x3D, 0x18C)?;
        for (at, offset) in [(0x40, 0x194), (0x44, 0x19C), (0x48, 0x1A4)] {
            store_setting_float(world, d, at, offset)?;
        }
        store_setting_byte(world, d, 0x4C, 0x1AC)?;
        store_setting_float(world, d, 0x50, 0x1B4)?;
        store_setting_float(world, d, 0x54, 0x1BC)?;
        for (at, offset) in [
            (0x58, 0x1C4),
            (0x59, 0x1CC),
            (0x5A, 0x1D4),
            (0x5B, 0x1DC),
            (0x5C, 0x1E4),
        ] {
            store_setting_byte(world, d, at, offset)?;
        }
        for (at, offset) in [
            (0x60, 0x1EC),
            (0x64, 0x1F4),
            (0x6C, 0x1FC),
            (0x70, 0x204),
            (0x74, 0x20C),
            (0x78, 0x214),
            (0x7C, 0x21C),
            (0x80, 0x224),
            (0x84, 0x22C),
        ] {
            store_setting_float(world, d, at, offset)?;
        }
        store_setting_byte(world, d, 0x88, 0x314)?;
        d[0x68 - 0x18] = 0;
        store_setting_float(world, d, 0x8C, 0x31C)?;
        d[0x90 - 0x18..0x94 - 0x18].fill(0);
    }
    {
        let mut style = world
            .get_mut::<TesCombatStyle>(form)
            .expect("a combat style");
        style.data_18 = data;
        style.advanced_94 = None;
    }
    reset_14_components(world, form)
}

// IDA: 004A96C0 TESCombatStyle vt+0x18: a nonzero +0x94 is freed and cleared; 0046C2F0. Completion: 100%.
fn destroy_combat_style(world: &mut World, form: Entity) -> Result<(), StreamError> {
    world
        .get_mut::<TesCombatStyle>(form)
        .expect("a combat style")
        .advanced_94 = None;
    reset_18_components(world, form)
}

// IDA: 004ABB40 TESCombatStyle_Constructor: 0046C460, the vtable, +0x04, 004A9A00. Completion: 100% of the store order.
fn construct_combat_style(world: &mut World) -> Result<Entity, StreamError> {
    let form = construct_tes_form(world)?;
    world.entity_mut(form).insert((
        OriginalVtable(TES_COMBAT_STYLE_VTABLE),
        TesCombatStyle {
            data_18: [0; 0x7C],
            advanced_94: None,
            data_written: false,
        },
    ));
    world
        .get_mut::<FormIdentity>(form)
        .expect("a form")
        .form_type_04 = FORM_TYPE_COMBAT_STYLE;
    reset_combat_style(world, form)?;
    Ok(form)
}

// IDA: 004ABC32 the CSTD arm: memset(+0x18, 0, 0x7C); +0x6C = +0x70 = 1.0; +0x74..+0x84, +0x88 and +0x8C from their settings; 00450C20(size 0x7C); then +0x74, +0x78 and +0x8C go back to their settings unless `keeps_positive`, and a signed +0x88 of 0 or less goes back to its setting. Completion: 100%.
fn load_cstd(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<(), StreamError> {
    const ONE: [u8; 4] = 0x3F80_0000_u32.to_le_bytes();
    let mut data = [0_u8; 0x7C];
    data[0x6C - 0x18..0x70 - 0x18].copy_from_slice(&ONE);
    data[0x70 - 0x18..0x74 - 0x18].copy_from_slice(&ONE);
    for (at, offset) in [
        (0x74, 0x20C),
        (0x78, 0x214),
        (0x7C, 0x21C),
        (0x80, 0x224),
        (0x84, 0x22C),
    ] {
        store_setting_float(world, &mut data, at, offset)?;
    }
    store_setting_byte(world, &mut data, 0x88, 0x314)?;
    store_setting_float(world, &mut data, 0x8C, 0x31C)?;
    cx.file.read_chunk(&mut data, 0x7C)?;
    if !keeps_positive(block_float(&data, 0x74)) {
        store_setting_float(world, &mut data, 0x74, 0x20C)?;
    }
    if !keeps_positive(block_float(&data, 0x78)) {
        store_setting_float(world, &mut data, 0x78, 0x214)?;
    }
    if (data[0x88 - 0x18] as i8) <= 0 {
        store_setting_byte(world, &mut data, 0x88, 0x314)?;
    }
    if !keeps_positive(block_float(&data, 0x8C)) {
        store_setting_float(world, &mut data, 0x8C, 0x31C)?;
    }
    let mut style = world
        .get_mut::<TesCombatStyle>(form)
        .expect("a combat style");
    style.data_18 = data;
    style.data_written = true;
    Ok(())
}

// IDA: 004A9BF0 the CSAD defaults: 21 floats from their settings, in the exe's order, which swaps three pairs. Offsets are from 00B35464. Completion: 100%.
const CSAD_SETTINGS: [u32; 21] = [
    0x23C, 0x234, 0x244, 0x24C, 0x254, 0x25C, 0x264, 0x26C, 0x274, 0x27C, 0x28C, 0x284, 0x294,
    0x29C, 0x2AC, 0x2A4, 0x2B4, 0x2C4, 0x2BC, 0x2CC, 0x2D4,
];

// IDA: 004ABBC0 TESCombatStyle vt+0x1C LoadForm. The record must be CSTY (the low byte); vt+0x18 first; 00451210; 0046AB80(0). The chunks: CSTD above; EDID; CSAD +0x94 freed, FormHeap(0x54), 004A9BF0, 00450C20(size 0x54), +0x68 |= 1. Returns 1. Completion: 100%.
fn load_combat_style_form(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<bool, StreamError> {
    if cx.file.record_form_type()? as u8 != FORM_TYPE_COMBAT_STYLE {
        return Ok(false);
    }
    destroy_combat_style(world, form)?;
    initialize_form_from_current_record(world, form, cx)?;
    set_initialized(world, form, false);
    let mut chunk = cx.file.chunk_type()?;
    while chunk != 0 {
        match chunk {
            CSAD => {
                let mut block = [0_u8; 0x54];
                for (index, offset) in CSAD_SETTINGS.iter().enumerate() {
                    let bits = x87::fld_fstp_copy(setting(world, *offset)?).to_bits();
                    block[index * 4..index * 4 + 4].copy_from_slice(&bits.to_le_bytes());
                }
                cx.file.read_chunk(&mut block, 0x54)?;
                let mut style = world
                    .get_mut::<TesCombatStyle>(form)
                    .expect("a combat style");
                style.advanced_94 = Some(block);
                style.data_18[0x68 - 0x18] |= 1;
            }
            EDID => {
                cx.file.read_text(0x200)?;
            }
            CSTD => load_cstd(world, form, cx)?,
            _ => {}
        }
        if !cx.file.advance_chunk()? {
            break;
        }
        chunk = cx.file.chunk_type()?;
    }
    Ok(true)
}

/// IDA: this family's classes. A type absent here takes `AbsentArm`.
/// TODO: [ ] 12d: the InitItem bodies of TESIdleForm 00520490, TESPackage 005667F0 and TESCombatStyle 004A9B70, with 0056AE20 and the condition tables, which only they reach.
pub const CLASSES: &[FormClass] = &[
    FormClass {
        form_type: FORM_TYPE_IDLE,
        vtable: TES_IDLE_FORM_VTABLE,
        construct: construct_idle,
        load_form: load_idle_form,
        reset_14: reset_idle,
        reset_18: destroy_idle,
        editor_id: default_editor_id,
        init_item: super::forms_init::init_idle,
        slots: &[ComponentSlot::Model],
        special_arm: Some(idle_arm),
    },
    FormClass {
        form_type: FORM_TYPE_PACKAGE,
        vtable: TES_PACKAGE_VTABLE,
        construct: construct_package,
        load_form: load_package_form,
        reset_14: reset_package,
        reset_18: destroy_package,
        editor_id: default_editor_id,
        init_item: super::forms_init::init_package,
        slots: &[],
        special_arm: None,
    },
    FormClass {
        form_type: FORM_TYPE_COMBAT_STYLE,
        vtable: TES_COMBAT_STYLE_VTABLE,
        construct: construct_combat_style,
        load_form: load_combat_style_form,
        reset_14: reset_combat_style,
        reset_18: destroy_combat_style,
        editor_id: default_editor_id,
        init_item: super::forms_init::init_flag_only,
        slots: &[],
        special_arm: None,
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_root_key_follows_00521370() {
        let key = |path: &[u8]| idle_root_key(path).data_00.unwrap_or_default();
        assert_eq!(key(b""), b"");
        assert_eq!(key(b"noslash.kf"), b"");
        assert_eq!(
            key(b"Characters\\_Male\\IdleAnims\\Sit.kf"),
            b"Characters\\_Male\\IdleAnims"
        );
        // strstr, and a match inside a longer folder name stops there.
        assert_eq!(key(b"A\\SpecialIdleAnims\\x.kf"), b"A\\SpecialIdleAnims");
        assert_eq!(key(b"A\\B\\x.kf"), b"A\\B\\IdleAnims");
        // Case-sensitive: 009840E0 compares bytes.
        assert_eq!(key(b"A\\idleanims\\x.kf"), b"A\\idleanims\\IdleAnims");
    }

    #[test]
    fn location_assign_follows_the_tables() {
        let mut location = PackageLocation::default();
        location.assign(&PackageLocation::from_pldt([1, 0x1234, 77]));
        assert_eq!(
            location,
            PackageLocation {
                type_00: 1,
                radius_04: 0,
                object_08: FormRef::Raw(0x1234)
            }
        );
        location.assign(&PackageLocation::from_pldt([4, 0x55, 9]));
        assert_eq!(
            location,
            PackageLocation {
                type_00: 4,
                radius_04: 9,
                object_08: FormRef::Raw(0x55)
            }
        );
        // Type 2: the object cleared by 00569750, the radius copied.
        location.assign(&PackageLocation::from_pldt([2, 0x66, 3]));
        assert_eq!(
            location,
            PackageLocation {
                type_00: 2,
                radius_04: 3,
                object_08: FormRef::Raw(0)
            }
        );
        // -1 returns, and 6 stores the type alone: no stack bytes reach the form.
        location.assign(&PackageLocation::from_pldt([0xFF, 1, 1]));
        assert_eq!(location.type_00, 2);
        location.assign(&PackageLocation::from_pldt([6, 1, 1]));
        assert_eq!(
            location,
            PackageLocation {
                type_00: 6,
                radius_04: 3,
                object_08: FormRef::Raw(0)
            }
        );
    }

    #[test]
    fn target_assign_follows_00569ef0() {
        let mut target = PackageTarget::default();
        target.assign_ptdt([0, 0xAB, 5]);
        assert_eq!(
            (target.type_00, target.value_04, target.count_08),
            (0, FormRef::Raw(0xAB), 5)
        );
        target.assign_ptdt([3, 0xCD, 6]);
        assert_eq!(
            (target.type_00, target.value_04, target.count_08),
            (3, FormRef::Raw(0xAB), 6)
        );
    }

    #[test]
    fn set_type_follows_the_byte_table() {
        let mut package = TesPackage::default();
        package.set_type(2);
        assert!(package.target_28.is_some() && package.location_24.is_some());
        package.set_type(4);
        assert!(package.target_28.is_none() && package.location_24.is_some());
        package.set_type(7);
        assert!(package.target_28.is_some() && package.location_24.is_none());
        package.set_type(-3);
        assert_eq!(package.type_20, -3);
    }

    #[test]
    fn array_insert_matches_0052f3c0() {
        let mut world = World::new();
        let [a, b, c] = [(); 3].map(|()| world.spawn_empty().id());
        let mut array = NiFormArray::default();
        array.insert(0, a);
        assert_eq!(
            (array.capacity_08, array.size_0c, array.count_10),
            (2, 1, 1)
        );
        array.insert(0, b);
        assert_eq!(array.at(0), Some(b));
        assert_eq!(array.at(1), Some(a));
        array.insert(5, c);
        assert_eq!(array.size_0c, 6);
        assert_eq!(array.count_10, 3);
        assert_eq!(array.capacity_08, 6);
        assert_eq!(array.at(5), Some(c));
        assert_eq!(array.find(Some(c)), 5);
        assert_eq!(array.at(u32::MAX), None);
        array.compact();
        assert_eq!(
            (array.capacity_08, array.size_0c, array.count_10),
            (3, 3, 3)
        );
        assert_eq!(array.at(2), Some(c));
        array.remove_at(2);
        assert_eq!((array.size_0c, array.count_10), (2, 2));
    }

    #[test]
    fn nan_keeps_like_the_x87_compare() {
        assert!(keeps_positive(f32::NAN));
        assert!(keeps_positive(1.0));
        assert!(!keeps_positive(0.0));
        assert!(!keeps_positive(-1.0));
    }
}
