//! Phase 12 (D1): the BaseFormComponent sub-objects of the TESForm classes, each once with its chunk handler, and the two passes over 00466E80's table. A shared component is a Bevy component on the form's entity, with offsets relative to the sub-object in its field names; the class records
//! where it sits. Original vtables are values, since the class's secondary vtable replaces the component's own. A class with two of a base has them in its own component, because 00466E80's `__RTDynamicCast` gives null for an ambiguous base.
//!
//! Evidence: evidence/phase12-window-2026-10-09/components-12b.txt and objdump of the addresses named here. `TesDescription` and `TesTexture` are in `data_handler`, from Phase 09.

use bevy_ecs::prelude::*;
use recomp_core::reader::StreamError;
use recomp_core::x87;

use super::data_handler::{
    DataHandlerGlobals, DataHandlerLists, TesDescription, TesTexture, reset_description,
};
use super::form::FormIdentity;
use super::form_arms::FormLoadCx;
use super::tes_file::BsStringT;

/// IDA: 00A322A0 ... the vtable a component's own constructor stores.
pub const TES_FULL_NAME_VTABLE: u32 = 0x00A3_22A0;
pub const TES_MODEL_VTABLE: u32 = 0x00A3_C24C;
pub const TES_ICON_VTABLE: u32 = 0x00A3_20A4;
pub const TES_SCRIPTABLE_FORM_VTABLE: u32 = 0x00A3_C490;
pub const TES_WEIGHT_FORM_VTABLE: u32 = 0x00A3_C6A0;
pub const TES_VALUE_FORM_VTABLE: u32 = 0x00A3_C680;
pub const TES_ENCHANTABLE_FORM_VTABLE: u32 = 0x00A3_B950;
pub const TES_ATTACK_DAMAGE_FORM_VTABLE: u32 = 0x00A3_B770;
pub const TES_LEVELED_LIST_VTABLE: u32 = 0x00A3_C078;
pub const TES_HEALTH_FORM_VTABLE: u32 = 0x00A3_C02C;
pub const TES_BIPED_MODEL_FORM_VTABLE: u32 = 0x00A3_B7A8;
pub const TES_ATTRIBUTES_VTABLE: u32 = 0x00A3_B788;
pub const TES_SPELL_LIST_VTABLE: u32 = 0x00A3_C540;
pub const TES_REACTION_FORM_VTABLE: u32 = 0x00A3_C41C;
pub const TES_PRODUCE_FORM_VTABLE: u32 = 0x00A3_C2A0;
pub const TES_QUALITY_FORM_VTABLE: u32 = 0x00A3_C35C;
pub const TES_RACE_FORM_VTABLE: u32 = 0x00A3_C370;
pub const TES_MODEL_LIST_VTABLE: u32 = 0x00A3_C28C;
pub const TES_CONTAINER_VTABLE: u32 = 0x00A3_B878;
pub const TES_USES_FORM_VTABLE: u32 = 0x00A3_C66C;
pub const TES_SOUND_FILE_VTABLE: u32 = 0x00A5_2168;
pub const TES_BOUND_OBJECT_VTABLE: u32 = 0x00A4_385C;
pub const TES_BOUND_ANIM_OBJECT_VTABLE: u32 = 0x00A4_3984;
pub const TES_BOUND_TREE_OBJECT_VTABLE: u32 = 0x00A4_3AAC;
pub const TES_ACTOR_BASE_DATA_VTABLE: u32 = 0x00A3_B654;
pub const TES_AI_FORM_VTABLE: u32 = 0x00A3_B718;
pub const TES_ANIMATION_VTABLE: u32 = 0x00A3_B75C;

/// IDA: a 32-bit slot holding the raw form ID from a record until InitItem stores the resolved TESForm*. Completion: 100%.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormRef {
    Raw(u32),
    Resolved(Option<Entity>),
}

impl Default for FormRef {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self::Raw(0)
    }
}

impl FormRef {
    /// IDA: N/A the word of a slot that still holds its raw ID, 0 once resolved: a printer's view before InitItem.
    #[must_use]
    pub const fn raw_word(&self) -> u32 {
        match self {
            Self::Raw(id) => *id,
            Self::Resolved(_) => 0,
        }
    }

    /// IDA: N/A the slot's null test (`*slot == 0`).
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        matches!(self, Self::Raw(0) | Self::Resolved(None))
    }
}

/// IDA: 00466E80's table, slot by slot, as 26 `__RTDynamicCast`s read from the exe (0: the form, whose vt+0 and vt+4 are nullsub_2 in every factory class). A class's list holds the slots it fills.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ComponentSlot {
    AiForm = 1,
    Animation = 2,
    ActorBaseData = 3,
    AttackDamage = 4,
    Attributes = 5,
    BipedModel = 6,
    Container = 7,
    Description = 8,
    Enchantable = 9,
    FullName = 10,
    Health = 11,
    Texture = 12,
    LeveledList = 13,
    Model = 14,
    Produce = 15,
    Quality = 16,
    RaceForm = 17,
    Scriptable = 18,
    SpellList = 19,
    Uses = 20,
    Value = 21,
    Weight = 22,
    SoundFile = 23,
    Reaction = 24,
    ModelList = 25,
}

// IDA: the form types 0046B090 / 0046B170 pass over (bytes 0046B13C, 0046B21C): SCPT, REFR, ACHR, ACRE, PGRD, LAND, PACK. Completion: 100%.
const fn skips_component_pass(form_type: u8) -> bool {
    matches!(form_type, 0x0D | 0x31 | 0x32 | 0x33 | 0x34 | 0x36 | 0x3D)
}

/// IDA: 0046B090 TESForm_ResetComponents (0046C2E0 jumps here): types 0x0D, 0x31..0x34, 0x36 and 0x3D return; 0x30 runs 005A6A20 on +0x18, its TESFullName; the rest run each slot's vt+0, in slot order (00466D80). `slots` is the class's filled slots, ascending. Completion: 100%.
pub fn reset_form_components(
    world: &mut World,
    form: Entity,
    slots: &[ComponentSlot],
) -> Result<(), StreamError> {
    let form_type = form_identity(world, form).form_type_04;
    if skips_component_pass(form_type) {
        return Ok(());
    }
    if form_type == 0x30 {
        if let Some(mut name) = world.get_mut::<TesFullName>(form) {
            name.name_04 = BsStringT::default();
        }
        return Ok(());
    }
    for &slot in slots {
        init_slot(world, form, slot)?;
    }
    Ok(())
}

/// IDA: 0046B170 (0046C2F0 jumps here): the same type switch, with 0x30 running nullsub_2, then each slot's vt+4 (00466DA0). Completion: 100% of the ported components.
pub fn destroy_form_components(
    world: &mut World,
    form: Entity,
    slots: &[ComponentSlot],
) -> Result<(), StreamError> {
    let form_type = form_identity(world, form).form_type_04;
    if skips_component_pass(form_type) || form_type == 0x30 {
        return Ok(());
    }
    for &slot in slots {
        destroy_slot(world, form, slot)?;
    }
    Ok(())
}

// IDA: N/A a form's head, +0x04..+0x0F.
fn form_identity(world: &World, form: Entity) -> FormIdentity {
    *world
        .get::<FormIdentity>(form)
        .expect("a form has an identity")
}

// IDA: N/A a component a slot list names must be there.
fn component_mut<'w, T: Component<Mutability = bevy_ecs::component::Mutable>>(
    world: &'w mut World,
    form: Entity,
) -> Mut<'w, T> {
    world
        .get_mut::<T>(form)
        .expect("a class's slot list names a component it has")
}

// IDA: each component's vt+0, from 00466D80. Completion: 100% of the ported components.
fn init_slot(world: &mut World, form: Entity, slot: ComponentSlot) -> Result<(), StreamError> {
    match slot {
        ComponentSlot::Description => {
            // 0046A360.
            world.resource_scope(|world, mut globals: Mut<DataHandlerGlobals>| {
                reset_description(
                    &mut component_mut::<TesDescription>(world, form),
                    &mut globals,
                );
            });
        }
        // 005A6A20.
        ComponentSlot::FullName => {
            component_mut::<TesFullName>(world, form).name_04 = BsStringT::default()
        }
        ComponentSlot::Texture => {
            component_mut::<TesTexture>(world, form).path_04 = BsStringT::default()
        }
        ComponentSlot::SoundFile => {
            component_mut::<TesSoundFile>(world, form).path_04 = BsStringT::default()
        }
        ComponentSlot::Model => component_mut::<TesModel>(world, form).init(),
        ComponentSlot::Reaction => component_mut::<TesReactionForm>(world, form).group_type_0c = 6,
        ComponentSlot::Scriptable => {
            let mut script = component_mut::<TesScriptableForm>(world, form);
            script.script_04 = FormRef::Raw(0);
            script.resolved_08 = 0;
        }
        ComponentSlot::Value => component_mut::<TesValueForm>(world, form).value_04 = 0,
        ComponentSlot::Weight => component_mut::<TesWeightForm>(world, form).weight_04 = 0.0,
        ComponentSlot::Quality => component_mut::<TesQualityForm>(world, form).quality_04 = 0.0,
        ComponentSlot::Health => component_mut::<TesHealthForm>(world, form).health_04 = 0,
        ComponentSlot::AttackDamage => {
            component_mut::<TesAttackDamageForm>(world, form).damage_04 = 0
        }
        ComponentSlot::Enchantable => {
            let mut enchantable = component_mut::<TesEnchantableForm>(world, form);
            enchantable.enchantment_04 = FormRef::Raw(0);
            enchantable.charge_08 = 0;
            enchantable.cast_type_0c = 4;
        }
        ComponentSlot::BipedModel => component_mut::<TesBipedModelForm>(world, form).init(),
        ComponentSlot::Container => component_mut::<TesContainer>(world, form).flags_04 = 0,
        ComponentSlot::Produce => {
            let mut produce = component_mut::<TesProduceForm>(world, form);
            produce.ingredient_04 = FormRef::Raw(0);
            produce.chances_08 = [0; 4];
        }
        ComponentSlot::LeveledList => {
            let mut list = component_mut::<TesLeveledList>(world, form);
            list.chance_none_0c = 0;
            list.flags_0d = 0;
        }
        ComponentSlot::Uses => component_mut::<TesUsesForm>(world, form).uses_04 = 0,
        ComponentSlot::AiForm => component_mut::<TesAiForm>(world, form).init(),
        ComponentSlot::ActorBaseData => component_mut::<TesActorBaseData>(world, form).init(),
        ComponentSlot::Attributes => component_mut::<TesAttributes>(world, form).init(),
        ComponentSlot::RaceForm => {
            component_mut::<TesRaceForm>(world, form).race_04 = FormRef::Raw(0)
        }
        // nullsub_2 (0060D0A0), read from A3B75C, A3C540 and A3C28C.
        ComponentSlot::Animation | ComponentSlot::SpellList | ComponentSlot::ModelList => {}
    }
    Ok(())
}

// IDA: each component's vt+4, from 00466DA0: nullsub_2 for the ones with no arm here. Completion: 100% of the ported components.
fn destroy_slot(world: &mut World, form: Entity, slot: ComponentSlot) -> Result<(), StreamError> {
    match slot {
        ComponentSlot::Model => component_mut::<TesModel>(world, form).destroy(),
        ComponentSlot::Reaction => component_mut::<TesReactionForm>(world, form)
            .reactions_04
            .clear(),
        ComponentSlot::BipedModel => component_mut::<TesBipedModelForm>(world, form).destroy(),
        ComponentSlot::Container => component_mut::<TesContainer>(world, form)
            .entries_08
            .clear(),
        ComponentSlot::LeveledList => component_mut::<TesLeveledList>(world, form)
            .entries_04
            .clear(),
        ComponentSlot::Description
        | ComponentSlot::FullName
        | ComponentSlot::Texture
        | ComponentSlot::SoundFile
        | ComponentSlot::Scriptable
        | ComponentSlot::Value
        | ComponentSlot::Weight
        | ComponentSlot::Quality
        | ComponentSlot::Health
        | ComponentSlot::AttackDamage
        | ComponentSlot::Enchantable
        | ComponentSlot::Produce
        | ComponentSlot::Uses
        | ComponentSlot::Attributes
        | ComponentSlot::RaceForm => {}
        // 00468170 -> 004526E0.
        ComponentSlot::AiForm => component_mut::<TesAiForm>(world, form).packages_10.clear(),
        // 00468740.
        ComponentSlot::Animation => component_mut::<TesAnimation>(world, form).names_04.clear(),
        // 00468020 -> 00467370.
        ComponentSlot::ActorBaseData => component_mut::<TesActorBaseData>(world, form)
            .factions_18
            .clear(),
        // 0046FC00 -> 0046F2E0.
        ComponentSlot::SpellList => component_mut::<TesSpellList>(world, form).destroy(),
        ComponentSlot::ModelList => component_mut::<TesModelList>(world, form).destroy(),
    }
    Ok(())
}

/// IDA: TESFullName (0x0C, built inline: vtable A322A0, +0x04 = 0, +0x08 = +0x0A = 0). vt+0 005A6A20 clears the BSStringT.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct TesFullName {
    pub original_vtable: u32,
    pub name_04: BsStringT,
}

/// IDA: an entry of a TESModel's +0x14 array: the archive file entry 0042DB10 found. Host substitution for `BSFileEntry*`: the two hashes it was found by. `None` is a null slot.
pub type TextureFileEntry = Option<(u64, u64)>;

/// IDA: 0046D7E0 TESModel_Constructor (0x18): +0x04 BSStringT path, +0x0C float bound radius 0.0, +0x10 byte texture count, +0x14 the FormHeap array of archive file entries (MODT).
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct TesModel {
    pub original_vtable: u32,
    pub path_04: BsStringT,
    pub bound_radius_0c: f32,
    pub texture_count_10: u8,
    pub texture_entries_14: Option<Vec<TextureFileEntry>>,
}

impl TesModel {
    /// IDA: 0046D4A0 vt+0: frees +0x04, +0x04 = +0x0A = +0x08 = 0, +0x0C = 0.0. Completion: 100%.
    pub fn init(&mut self) {
        self.path_04 = BsStringT::default();
        self.bound_radius_0c = 0.0;
    }

    /// IDA: 0046D4D0 vt+4: a nonzero +0x14 is freed and cleared; +0x10 = 0. Completion: 100%.
    pub fn destroy(&mut self) {
        self.texture_entries_14 = None;
        self.texture_count_10 = 0;
    }
}

/// IDA: an 8-byte {TESForm* faction, s32 modifier} block of a TESReactionForm.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TesReaction {
    pub faction: FormRef,
    pub modifier: i32,
}

/// IDA: 0046E5E0 TESReactionForm_Constructor (0x10): +0x04/+0x08 the BSSimpleList of blocks (XNAM), +0x0C byte group type 6. vt+0 0046E5D0: +0x0C = 6. vt+4 0046E600 frees the blocks and nodes.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct TesReactionForm {
    pub original_vtable: u32,
    pub reactions_04: Vec<TesReaction>,
    pub group_type_0c: u8,
}

/// IDA: TESSoundFile (0x0C, inline in 00517F40: vtable A52168, +0x04 BSStringT). vt+0 005A6A20.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct TesSoundFile {
    pub original_vtable: u32,
    pub path_04: BsStringT,
}

/// IDA: 0046EF60 TESScriptableForm_Constructor (0x0C): +0x04 the script (SCRI's raw ID until 0046F020), +0x08 byte resolved. vt+0 0046EF50 clears both.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TesScriptableForm {
    pub original_vtable: u32,
    pub script_04: FormRef,
    pub resolved_08: u8,
}

/// IDA: TESUsesForm (8): +0x04 byte. vt+0 00470320 clears it; vt+4 is nullsub_2.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TesUsesForm {
    pub original_vtable: u32,
    pub uses_04: u8,
}

/// IDA: 00470470 TESValueForm (8): +0x04 s32. vt+0 0046C9C0 clears it.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TesValueForm {
    pub original_vtable: u32,
    pub value_04: i32,
}

/// IDA: 004705A0 TESWeightForm (8): +0x04 float. vt+0 00470590 = 0.0.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct TesWeightForm {
    pub original_vtable: u32,
    pub weight_04: f32,
}

/// IDA: 0046E300 TESQualityForm (8): +0x04 float. vt+0 00470590 = 0.0.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct TesQualityForm {
    pub original_vtable: u32,
    pub quality_04: f32,
}

/// IDA: 0046CA50 TESHealthForm (8): +0x04 u32. vt+0 0046C9C0 clears it.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TesHealthForm {
    pub original_vtable: u32,
    pub health_04: u32,
}

/// IDA: 00468A60 TESAttackDamageForm (8): +0x04 u16. vt+0 004689B0 clears it.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TesAttackDamageForm {
    pub original_vtable: u32,
    pub damage_04: u16,
}

/// IDA: 0046A790 TESEnchantableForm (0x10): +0x04 enchantment (ENAM's raw ID), +0x08 u16 charge (ANAM), +0x0C u32 cast type 4. vt+0 0046A770: +0x04 = 0, +0x08 = 0, +0x0C = 4.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TesEnchantableForm {
    pub original_vtable: u32,
    pub enchantment_04: FormRef,
    pub charge_08: u16,
    pub cast_type_0c: u32,
}

/// IDA: 004690D0 TESBipedModelForm (0x80): +0x04 u16 slot mask, +0x06 byte flags, +0x07 unset, +0x08 / +0x20 the male / female biped TESModels, +0x38 / +0x50 the male / female world TESModels, +0x68 / +0x74 the male / female TESIcons.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct TesBipedModelForm {
    pub original_vtable: u32,
    pub slot_mask_04: u16,
    pub flags_06: u8,
    pub unset_07: u8,
    pub biped_models_08: [TesModel; 2],
    pub world_models_38: [TesModel; 2],
    pub icons_68: [TesTexture; 2],
}

impl TesBipedModelForm {
    /// IDA: 00468CC0 vt+0: +0x04 = 0 (u16), +0x06 = 0, then each part's vt+0: +0x08, +0x38, +0x68, +0x20, +0x50, +0x74. Completion: 100%.
    pub fn init(&mut self) {
        self.slot_mask_04 = 0;
        self.flags_06 = 0;
        self.biped_models_08[0].init();
        self.world_models_38[0].init();
        self.icons_68[0].path_04 = BsStringT::default();
        self.biped_models_08[1].init();
        self.world_models_38[1].init();
        self.icons_68[1].path_04 = BsStringT::default();
    }

    /// IDA: 00468D10 vt+4: each part's vt+4, in the same order; a TESIcon's is nullsub_2. Completion: 100%.
    pub fn destroy(&mut self) {
        self.biped_models_08[0].destroy();
        self.world_models_38[0].destroy();
        self.biped_models_08[1].destroy();
        self.world_models_38[1].destroy();
    }
}

/// IDA: an 8-byte {s32 count, TESForm* item} entry of a TESContainer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TesContainerEntry {
    pub count: i32,
    pub item: FormRef,
}

/// IDA: 00469690 TESContainer (0x10): +0x04 byte flags, +0x08/+0x0C the BSSimpleList of entries (CNTO). vt+0 00470320: +0x04 = 0. vt+4 004696B0 frees the entries and nodes.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct TesContainer {
    pub original_vtable: u32,
    pub flags_04: u8,
    pub entries_08: Vec<TesContainerEntry>,
}

impl TesContainer {
    /// IDA: 00469750 TESContainer_AddEntry({item, count}): nothing with flags bit 0 set; the first entry with the same item takes the new count; otherwise a new {count, item} block is prepended (00446CB0). Completion: 100%.
    pub fn add_entry(&mut self, item: u32, count: i32) {
        if self.flags_04 & 1 != 0 {
            return;
        }
        if let Some(entry) = self
            .entries_08
            .iter_mut()
            .find(|entry| entry.item == FormRef::Raw(item))
        {
            entry.count = count;
            return;
        }
        self.entries_08.insert(
            0,
            TesContainerEntry {
                count,
                item: FormRef::Raw(item),
            },
        );
    }
}

/// IDA: 0046E0B0 TESProduceForm (0x0C): +0x04 ingredient (PFIG's raw ID), +0x08 the four season chances (PFPC). vt+0 0046E0A0 clears both dwords.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TesProduceForm {
    pub original_vtable: u32,
    pub ingredient_04: FormRef,
    pub chances_08: [u8; 4],
}

/// IDA: a 0x0C entry of a TESLeveledList: +0x00 u16 level, +0x04 form (a raw ID), +0x08 s16 count.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TesLeveledEntry {
    pub level_00: u16,
    pub form_04: FormRef,
    pub count_08: i16,
}

/// IDA: 0046CB80 TESLeveledList (0x10): +0x04/+0x08 the BSSimpleList of entries sorted by level, +0x0C byte chance none, +0x0D byte flags. vt+0 0046CAD0: both bytes 0. vt+4 0046CBA0 frees the entries and nodes.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct TesLeveledList {
    pub original_vtable: u32,
    pub entries_04: Vec<TesLeveledEntry>,
    pub chance_none_0c: u8,
    pub flags_0d: u8,
}

impl TesLeveledList {
    /// IDA: 0046CBF0 TESLeveledList_AddEntry(level, count, form): the walk stops at the first entry whose level is greater (unsigned); 00446CB0 at that node keeps it after the new entry, otherwise 0067B1E0 appends. No null node exists here, since only vt+4 removes. Completion: 100%.
    pub fn add_entry(&mut self, level: u16, count: i16, form: u32) {
        let entry = TesLeveledEntry {
            level_00: level,
            form_04: FormRef::Raw(form),
            count_08: count,
        };
        match self
            .entries_04
            .iter()
            .position(|entry| entry.level_00 > level)
        {
            Some(index) => self.entries_04.insert(index, entry),
            None => self.entries_04.push(entry),
        }
    }
}

/// IDA: 004B2FF0 TESBoundObject / 004B31D0 TESBoundAnimObject / 004B31F0 TESBoundTreeObject constructors: TESForm_Constructor, +0x1C = +0x20 = +0x18 = 0, then the vtable. +0x18 is the list, +0x1C previous, +0x20 next. Presence of this component is the `TESObject` RTTI cast of 0044DBE8.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TesObjectLinks {
    pub in_list_18: bool,
    pub previous_1c: Option<Entity>,
    pub next_20: Option<Entity>,
}

/// IDA: 004B2B80 TESObjectList_Append(object) on DataHandler +0x00: previous = last; with a last, next = last's next, whose back link moves to the object, and last's next becomes the object; otherwise next = 0. A null next makes the object the last, a null previous the first; +0x00 count + 1;
/// the object's list = this. Completion: 100%.
pub fn append_tes_object(world: &mut World, object: Entity) {
    let last = world.resource::<DataHandlerLists>().objects_00.last_08;
    let next = last.and_then(|last| {
        world
            .get::<TesObjectLinks>(last)
            .expect("the list holds TESObjects")
            .next_20
    });
    {
        let mut links = world
            .get_mut::<TesObjectLinks>(object)
            .expect("the caller checked the TESObject");
        links.previous_1c = last;
        links.next_20 = next;
    }
    if let Some(last) = last {
        if let Some(next) = next {
            world
                .get_mut::<TesObjectLinks>(next)
                .expect("the list holds TESObjects")
                .previous_1c = Some(object);
        }
        world
            .get_mut::<TesObjectLinks>(last)
            .expect("the list holds TESObjects")
            .next_20 = Some(object);
    }
    let mut lists = world.resource_mut::<DataHandlerLists>();
    if next.is_none() {
        lists.objects_00.last_08 = Some(object);
    }
    if last.is_none() {
        lists.objects_00.first_04 = Some(object);
    }
    lists.objects_00.count_00 = lists.objects_00.count_00.wrapping_add(1);
    world
        .get_mut::<TesObjectLinks>(object)
        .expect("the caller checked the TESObject")
        .in_list_18 = true;
}

/// IDA: 00467320 TESActorBaseData's faction block (8, FormHeap): +0x00 faction (raw until InitItem), +0x04 s8 rank, +0x05..+0x07 unset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TesFaction {
    pub faction_00: FormRef,
    pub rank_04: i8,
}

/// IDA: 00467320 TESActorBaseData (0x20): +0x04 flags, +0x08 u16 magicka 50, +0x0A u16 fatigue 50, +0x0C u16 barter gold, +0x0E u16 level 1, +0x10 / +0x12 u16 calc min / max, +0x14 the death item (INAM, raw), +0x18/+0x1C the BSSimpleList of faction blocks in list order. vt+0 00467240; vt+4 00468020 -> 00467370.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct TesActorBaseData {
    pub original_vtable: u32,
    pub flags_04: u32,
    pub magicka_08: u16,
    pub fatigue_0a: u16,
    pub barter_0c: u16,
    pub level_0e: u16,
    pub min_10: u16,
    pub max_12: u16,
    pub death_item_14: FormRef,
    pub factions_18: Vec<TesFaction>,
}

impl TesActorBaseData {
    /// IDA: 00467240 vt+0, the same stores as 00467320's. Completion: 100%.
    pub fn init(&mut self) {
        self.flags_04 = 0;
        self.magicka_08 = 0x32;
        self.fatigue_0a = 0x32;
        self.barter_0c = 0;
        self.level_0e = 1;
        self.min_10 = 0;
        self.max_12 = 0;
        self.death_item_14 = FormRef::Raw(0);
    }

    /// IDA: N/A the 0x10 bytes at +0x04, in their order, which ACBS reads over.
    #[must_use]
    pub fn acbs_bytes(&self) -> [u8; 0x10] {
        let mut bytes = [0_u8; 0x10];
        bytes[..4].copy_from_slice(&self.flags_04.to_le_bytes());
        for (index, value) in [
            self.magicka_08,
            self.fatigue_0a,
            self.barter_0c,
            self.level_0e,
            self.min_10,
            self.max_12,
        ]
        .iter()
        .enumerate()
        {
            bytes[4 + 2 * index..6 + 2 * index].copy_from_slice(&value.to_le_bytes());
        }
        bytes
    }

    /// IDA: N/A the same bytes back into the fields.
    pub fn set_acbs_bytes(&mut self, bytes: &[u8; 0x10]) {
        let word = |at: usize| u16::from_le_bytes([bytes[at], bytes[at + 1]]);
        self.flags_04 = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        self.magicka_08 = word(4);
        self.fatigue_0a = word(6);
        self.barter_0c = word(8);
        self.level_0e = word(10);
        self.min_10 = word(12);
        self.max_12 = word(14);
    }

    /// IDA: 004675E0 TESActorBaseData_SetFactionRank(faction, rank): a null faction returns; vt+0x50(0x40) is the caller's mark; the first block with this faction takes the rank, or with rank 0xFF leaves the list (the block isn't freed); otherwise a new block is prepended (00446CB0), even with rank 0xFF. Completion: 100%.
    pub fn set_faction_rank(&mut self, faction: u32, rank: i8) {
        if let Some(index) = self
            .factions_18
            .iter()
            .position(|entry| entry.faction_00 == FormRef::Raw(faction))
        {
            if rank == -1 {
                self.factions_18.remove(index);
            } else {
                self.factions_18[index].rank_04 = rank;
            }
            return;
        }
        self.factions_18.insert(
            0,
            TesFaction {
                faction_00: FormRef::Raw(faction),
                rank_04: rank,
            },
        );
    }
}

/// IDA: 004684A0 TESAIForm (0x18): +0x04 the four AI bytes (aggression, confidence, energy, responsibility), +0x08 services, +0x0C / +0x0D trainer skill / level, +0x0E..+0x0F (cleared by vt+0, nothing else stores them), +0x10/+0x14 the BSSimpleList of packages (raw IDs). vt+0 00468160; vt+4 00468170.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct TesAiForm {
    pub original_vtable: u32,
    pub ai_04: [u8; 4],
    pub services_08: u32,
    pub trainer_skill_0c: u8,
    pub trainer_level_0d: u8,
    pub cleared_0e: u16,
    pub packages_10: Vec<FormRef>,
}

impl TesAiForm {
    /// IDA: 00468160 vt+0: the dwords +0x04, +0x08 and +0x0C = 0. Completion: 100%.
    pub fn init(&mut self) {
        self.ai_04 = [0; 4];
        self.services_08 = 0;
        self.trainer_skill_0c = 0;
        self.trainer_level_0d = 0;
        self.cleared_0e = 0;
    }

    /// IDA: 00468280 TESAIForm_SetData(AIDT's 12 bytes): the four AI bytes, each with vt+0x10(0x100), which is the caller's mark; +0x08; +0x0C; +0x0D. Completion: 100%.
    pub fn set_data(&mut self, data: &[u8; 12]) {
        self.ai_04.copy_from_slice(&data[..4]);
        self.services_08 = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
        self.trainer_skill_0c = data[8];
        self.trainer_level_0d = data[9];
    }
}

/// IDA: 00468BB0 TESAttributes (0x0C): the eight attribute bytes, 0x32. vt+0 00468AC0; vt+4 nullsub_2.
#[derive(Clone, Copy, Component, Debug, Default, PartialEq, Eq)]
pub struct TesAttributes {
    pub original_vtable: u32,
    pub values_04: [u8; 8],
}

impl TesAttributes {
    /// IDA: 00468BB0, under the class's vtable.
    #[must_use]
    pub const fn new(original_vtable: u32) -> Self {
        Self {
            original_vtable,
            values_04: [0x32; 8],
        }
    }

    /// IDA: 00468AC0 vt+0. Completion: 100%.
    pub const fn init(&mut self) {
        self.values_04 = [0x32; 8];
    }
}

/// IDA: 00468720 TESAnimation (0x0C): +0x04/+0x08 the KFFZ names, FormHeap copies in list order. vt+0 nullsub_2; vt+4 00468740 frees each.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct TesAnimation {
    pub original_vtable: u32,
    pub names_04: Vec<Vec<u8>>,
}

/// IDA: an 8-byte block of a TESModelList's +0x10 array: +0x00 u8 count, +0x04 the FormHeap entries.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TesModelTextures {
    pub count_00: u8,
    pub entries_04: Vec<TextureFileEntry>,
}

/// IDA: 0046DBC0 TESModelList (0x14): +0x04/+0x08 the NIFZ names, +0x0C NIFT count, +0x10 the per-model texture lists. vt+0 nullsub_2; vt+4 0046DF80.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct TesModelList {
    pub original_vtable: u32,
    pub names_04: Vec<Vec<u8>>,
    pub count_0c: u32,
    pub textures_10: Option<Vec<TesModelTextures>>,
}

impl TesModelList {
    /// IDA: 0046DF80 vt+4: frees each name, then 0046DE10: frees each list and the array when +0x10 is set; +0x0C = 0. Completion: 100%.
    pub fn destroy(&mut self) {
        self.names_04.clear();
        self.textures_10 = None;
        self.count_0c = 0;
    }
}

/// IDA: 0046E450 TESRaceForm (8): +0x04 the race (RNAM, raw). vt+0 0046C9C0 clears it; vt+4 nullsub_2.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TesRaceForm {
    pub original_vtable: u32,
    pub race_04: FormRef,
}

/// IDA: 0046F2C0 TESSpellList (0x14): +0x04/+0x08 the spells, +0x0C/+0x10 the leveled spells, raw form IDs while loading, in list order. vt+0 nullsub_2; vt+4 0046FC00 -> 0046F2E0 frees both lists' nodes.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct TesSpellList {
    pub original_vtable: u32,
    pub spells_04: Vec<FormRef>,
    pub leveled_0c: Vec<FormRef>,
}

impl TesSpellList {
    /// IDA: 0046F2E0. Completion: 100%.
    pub fn destroy(&mut self) {
        self.spells_04.clear();
        self.leveled_0c.clear();
    }

    /// IDA: 0046F420 TESSpellList_AddSpell(form) with TLS +0x184 set: 0046F350 prepends a nonzero ID not on +0x04 (00446CB0). Its 0065F6A0 arm needs TLS +0x184 clear. With the byte clear the argument is a form pointer, and the two casts of 0046F44D are a host stop. Completion: 100% of the arm a load takes.
    /// TODO: [ ] 0046F44D the casts, 0046F3D0 and 0065F6A0 (no load reaches them; the caller's world answers `byte_184`).
    pub fn add_spell_while_loading(
        &mut self,
        id: u32,
        loading_184: bool,
    ) -> Result<(), StreamError> {
        if !loading_184 {
            return Err(StreamError(
                "0046F44D: an add outside a load casts a form pointer",
            ));
        }
        if id == 0 || self.spells_04.contains(&FormRef::Raw(id)) {
            return Ok(());
        }
        self.spells_04.insert(0, FormRef::Raw(id));
        Ok(())
    }
}

// IDA: the chunk fourccs the component loaders compare.
const FULL: u32 = 0x4C4C_5546;
const MODB: u32 = 0x4244_4F4D;
const MODL: u32 = 0x4C44_4F4D;
const MODT: u32 = 0x5444_4F4D;
const MOD2: u32 = 0x3244_4F4D;
const MOD3: u32 = 0x3344_4F4D;
const MOD4: u32 = 0x3444_4F4D;
const MO2B: u32 = 0x4232_4F4D;
const MO3B: u32 = 0x4233_4F4D;
const MO4B: u32 = 0x4234_4F4D;
const MO2T: u32 = 0x5432_4F4D;
const MO3T: u32 = 0x5433_4F4D;
const MO4T: u32 = 0x5434_4F4D;
const ICON: u32 = 0x4E4F_4349;
const ICO2: u32 = 0x324F_4349;
const BMDT: u32 = 0x5444_4D42;

/// IDA: 0046C7A0 TESFullName_LoadFULL(component, file) (__cdecl): a FULL chunk with data is read whole into +0x04 (alloca, 00450C20 size 0, 004028D0 minimum 0); an empty FULL frees +0x04 and clears the three fields; another chunk does nothing. A null component is the caller's `Option`. Completion: 100%.
pub fn load_full_name(
    name: &mut BsStringT,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<(), StreamError> {
    if cx.file.chunk_type()? != FULL {
        return Ok(());
    }
    load_text_or_clear(name, cx)
}

// IDA: 0046C7D9..0046C81C and 00470101..0047014E: both read a chunk with data whole and set it, and free and clear for an empty one.
fn load_text_or_clear(
    text: &mut BsStringT,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<(), StreamError> {
    if cx.file.chunk_size()? != 0 {
        let read = cx.file.read_text(0)?;
        text.set(Some(&read), 0);
    } else {
        *text = BsStringT::default();
    }
    Ok(())
}

/// IDA: 004700E0 TESTexture_LoadPath(texture, file) (__cdecl), which doesn't test the chunk type. Completion: 100%.
pub fn load_texture(
    texture: &mut TesTexture,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<(), StreamError> {
    load_text_or_clear(&mut texture.path_04, cx)
}

/// IDA: 0046A390 TESDescription_LoadOffset(description, file): +0x04 = file +0x25C. Completion: 100%.
pub fn load_description(
    description: &mut TesDescription,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<(), StreamError> {
    description.offset_04 = cx.file.record_offset()?;
    Ok(())
}

/// IDA: 0046DB00 TESModel_LoadChunk(model, file) (__cdecl). MODB reads 4 bytes (004510E0) into a local and `fld`/`fstp`s it to +0x0C. MODL reads the chunk whole and calls vt+0x18, 0046E080: 004028D0(+0x04, text, 0). MODT is 0046D940. `owner` is the form 0046D940's RTTI cross cast finds. Completion: 100%.
pub fn load_model(
    world: &World,
    model: &mut TesModel,
    owner: Option<Entity>,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<(), StreamError> {
    match cx.file.chunk_type()? {
        MODB => {
            model.bound_radius_0c = read_radius(cx)?;
        }
        MODL => {
            let text = cx.file.read_text(0)?;
            model.path_04.set(Some(&text), 0);
        }
        MODT => load_model_texture_hashes(world, model, owner, cx)?,
        _ => {}
    }
    Ok(())
}

// IDA: 0046DB98..0046DBA6. A chunk with nothing to read leaves the local's stack bytes in the original; the host reads zero there.
fn read_radius(cx: &mut FormLoadCx<'_, '_>) -> Result<f32, StreamError> {
    let (bits, _) = cx.file.read_dword(0)?;
    Ok(x87::fld_fstp_copy(bits))
}

/// IDA: 0046D940 -> 0046D750: the chunk must be a nonzero multiple of 0x18 bytes (`mul` by AAAAAAAB, then `shr` 4); it goes in a zeroed alloca, is read with that size, and 0046D610 runs with the model's vt+0x14 path (+0x04 or ""). Completion: 100%.
pub fn load_model_texture_hashes(
    world: &World,
    model: &mut TesModel,
    owner: Option<Entity>,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<(), StreamError> {
    let size = cx.file.chunk_size()?;
    let count = size / 0x18;
    if size % 0x18 != 0 || count == 0 {
        return Ok(());
    }
    let mut buffer = vec![0_u8; size as usize];
    cx.file.read_chunk(&mut buffer, size)?;
    let path = model.path_04.data_00.clone().unwrap_or_default();
    let (count_out, entries) = resolve_texture_hashes(world, &buffer, count, &path, owner, cx);
    model.texture_count_10 = count_out;
    model.texture_entries_14 = Some(entries);
    Ok(())
}

/// IDA: 0046D610 the texture hash list: the count (a byte store), FormHeap(4 * count), and for each 0x18-byte entry 0042DB10(1, the hash at +0x10, the hash at +0x00, 0) into the array. When 00B06310 and 00B04428 (g_UseArchives) are set and it's null, a warning: with a form and a nonempty-pointer path,
/// a3c1f8 (path, vt+0xD4, ID); with no form, a3c1a8 (path); a null path can't arise, since vt+0x14 answers "". Completion: 100%.
pub fn resolve_texture_hashes(
    world: &World,
    data: &[u8],
    count: u32,
    path: &[u8],
    owner: Option<Entity>,
    cx: &mut FormLoadCx<'_, '_>,
) -> (u8, Vec<TextureFileEntry>) {
    let mut entries = vec![None; count as usize];
    for (index, slot) in entries.iter_mut().enumerate() {
        let entry = &data[index * 0x18..index * 0x18 + 0x18];
        let file_hash = u64::from_le_bytes(entry[..8].try_into().expect("eight bytes"));
        let folder_hash = u64::from_le_bytes(entry[0x10..0x18].try_into().expect("eight bytes"));
        *slot = cx.host.find_texture_entry(folder_hash, file_hash);
        let (warn, use_archives) = cx.host.texture_warning_gates();
        if !warn || !use_archives || slot.is_some() {
            continue;
        }
        let path: String = path.iter().map(|&byte| char::from(byte)).collect();
        if let Some(form) = owner {
            let id = form_identity(world, form).form_id_0c;
            let name = super::form_arms::editor_id_of(world, form);
            let name: String = name.iter().map(|&byte| char::from(byte)).collect();
            cx.file.services.warn(format!(
                "Failed to find archive file entry for texture on model '{path}' for form {name} ({id:08X})."
            ));
        } else {
            cx.file.services.warn(format!(
                "Failed to find archive file entry for texture on model '{path}' on UNKNOWN form."
            ));
        }
    }
    (count as u8, entries)
}

/// IDA: the biped chunks ARMO's 004B4D90 and CLOT's 004B5DB0 share, inlined there (read for ARMO: MOD2 +0x38, MOD3 +0x20, MOD4 +0x50, ICO2 +0x74 of the biped form). Returns whether the chunk was one of them. Completion: 100% of the map; the two LoadForms are 12b's.
pub fn load_biped_chunk(
    world: &World,
    biped: &mut TesBipedModelForm,
    chunk: u32,
    owner: Option<Entity>,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<bool, StreamError> {
    match chunk {
        MODL | MOD2 | MOD3 | MOD4 => {
            let text = cx.file.read_text(0)?;
            biped_model(biped, chunk)?.path_04.set(Some(&text), 0);
        }
        MODB | MO2B | MO3B | MO4B => {
            let radius = read_radius(cx)?;
            biped_model(biped, chunk)?.bound_radius_0c = radius;
        }
        MODT | MO2T | MO3T | MO4T => {
            load_model_texture_hashes(world, biped_model(biped, chunk)?, owner, cx)?;
        }
        ICON => {
            let text = cx.file.read_text(0)?;
            biped.icons_68[0].path_04.set(Some(&text), 0);
        }
        ICO2 => {
            let text = cx.file.read_text(0)?;
            biped.icons_68[1].path_04.set(Some(&text), 0);
        }
        BMDT => {
            // 004B50E4 00450C20(+0x04 of the biped form, 4): a short chunk leaves the rest of the words.
            let mask = biped.slot_mask_04.to_le_bytes();
            let mut bytes = [mask[0], mask[1], biped.flags_06, biped.unset_07];
            cx.file.read_chunk(&mut bytes, 4)?;
            biped.slot_mask_04 = u16::from_le_bytes([bytes[0], bytes[1]]);
            biped.flags_06 = bytes[2];
            biped.unset_07 = bytes[3];
        }
        _ => return Ok(false),
    }
    Ok(true)
}

// IDA: N/A the TESModel a biped chunk names.
fn biped_model(biped: &mut TesBipedModelForm, chunk: u32) -> Result<&mut TesModel, StreamError> {
    match chunk {
        MODL | MODB | MODT => Ok(&mut biped.biped_models_08[0]),
        MOD2 | MO2B | MO2T => Ok(&mut biped.world_models_38[0]),
        MOD3 | MO3B | MO3T => Ok(&mut biped.biped_models_08[1]),
        MOD4 | MO4B | MO4T => Ok(&mut biped.world_models_38[1]),
        _ => Err(StreamError("004B4D90: not a biped model chunk")),
    }
}

/// IDA: the components 0046BDA0 fills after the copied DATA bytes, in cast order; `None` is a null cast. A null (or ambiguous) one reads nothing and moves nothing.
#[derive(Default)]
pub struct DataChunkTails<'a> {
    /// TESUsesForm +0x04.
    pub uses: Option<&'a mut u8>,
    /// TESValueForm +0x04, through 004703C0.
    pub value: Option<&'a mut i32>,
    /// IDA: 004703D6..004703F1 004703C0's tail: the form cast, then vt+0x40(8). `None` is a null cast. Every class with a value form reaches 0046ABF0 there (slots read from the exe).
    pub value_change: Option<&'a mut dyn FnMut(u32) -> Result<(), StreamError>>,
    /// TESHealthForm +0x04.
    pub health: Option<&'a mut u32>,
    /// TESWeightForm +0x04.
    pub weight: Option<&'a mut f32>,
    /// TESQualityForm +0x04.
    pub quality: Option<&'a mut f32>,
    /// TESAttackDamageForm +0x04, through 00468A50.
    pub attack_damage: Option<&'a mut u16>,
    /// TESAttributes +0x04, through 00468CA0.
    pub attributes: Option<&'a mut [u8; 8]>,
}

/// IDA: 0046BDA0 TESForm_LoadDataChunk(form, file, destination, size). The size words are u16: alloca(chunk), 00450C20(buffer, chunk), memcpy(destination, buffer, min(chunk, size)), then each tail, in cast order, while the cursor (starting at `size`) is below the chunk:
/// TESUsesForm a byte; TESValueForm a dword through 004703C0; TESHealthForm a dword; TESWeightForm a float with three `fld`/`fstp` copies; TESQualityForm a float, `fistp` under chop, the low byte, `fild`, stored (the C++ copies the bits); TESAttackDamageForm a word; TESAttributes 8 bytes with no bound test.
/// A bound past the alloca reads stack bytes in the original, and the host pads with zeros. Completion: 100%.
pub fn load_data_chunk(
    destination: &mut [u8],
    size: u16,
    tails: DataChunkTails<'_>,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<(), StreamError> {
    let chunk = cx.file.chunk_size()? as u16;
    let mut buffer = vec![0_u8; usize::from(chunk) + 16];
    cx.file.read_chunk(&mut buffer, u32::from(chunk))?;
    let count = usize::from(chunk.min(size));
    destination[..count].copy_from_slice(&buffer[..count]);
    let dword = |at: u16| {
        u32::from_le_bytes(
            buffer[usize::from(at)..usize::from(at) + 4]
                .try_into()
                .expect("four bytes"),
        )
    };
    let mut cursor = size;
    if let Some(uses) = tails.uses
        && cursor < chunk
    {
        *uses = buffer[usize::from(cursor)];
        cursor = cursor.wrapping_add(1);
    }
    if let Some(value) = tails.value
        && cursor < chunk
    {
        // 004703C0: the store, then its tail call.
        *value = dword(cursor) as i32;
        if let Some(change) = tails.value_change {
            change(8)?;
        }
        cursor = cursor.wrapping_add(4);
    }
    if let Some(health) = tails.health
        && cursor < chunk
    {
        *health = dword(cursor);
        cursor = cursor.wrapping_add(4);
    }
    if let Some(weight) = tails.weight
        && cursor < chunk
    {
        // 0046BF1F..0046BF2E three `fld`/`fstp` copies: only the first can change the bits.
        *weight = x87::fld_fstp_copy(dword(cursor));
        cursor = cursor.wrapping_add(4);
    }
    if let Some(quality) = tails.quality
        && cursor < chunk
    {
        // 0046BF40..0046BF74: control word | 0xC00, `fistp`, `movzx` of the low byte, `fild`, `fstp`.
        let truncated = x87::ftol(x87::fld_fstp_copy(dword(cursor)));
        *quality = f32::from(truncated as u8);
        cursor = cursor.wrapping_add(4);
    }
    if let Some(damage) = tails.attack_damage
        && cursor < chunk
    {
        let at = usize::from(cursor);
        *damage = u16::from_le_bytes([buffer[at], buffer[at + 1]]);
        cursor = cursor.wrapping_add(2);
    }
    if let Some(attributes) = tails.attributes {
        // 0046BFA2 no remaining-bytes test.
        let at = usize::from(cursor);
        attributes.copy_from_slice(&buffer[at..at + 8]);
    }
    Ok(())
}
