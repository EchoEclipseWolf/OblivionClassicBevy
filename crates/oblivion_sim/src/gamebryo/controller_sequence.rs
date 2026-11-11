//! NiControllerSequence (006C7840) and NiControllerManager (006C5BB0): cold state, modern LoadBinary and LinkObject, AddSequence,
//! the 006C4260 PostLoad StoreTargets pass with Bethesda's 006C8220 multi-target conversion, and the runtime Activate, Deactivate,
//! blend, morph and Update family. Controller chains, sequence slots and the active list are explicit ordered lists.

use super::transform_animation::QuatTransform;
use bevy_ecs::prelude::*;
use oblivion_core::crt::{stricmp, strnicmp};
use oblivion_core::error::GameError;
use oblivion_core::hash::ni_string_hash;
use oblivion_core::reader::ByteReader;
use oblivion_core::x87::float_fmod;

use super::av_object::NiAvParent;
use super::av_object_palette::{DefaultAvObjectPalette, rebuild_from_scene};
use super::blend_interpolator::{
    BlendInterpolator, ParticleBlendBoolInterpolator, ParticleBlendCold,
    ParticleBlendFloatInterpolator, ParticleBlendTransformInterpolator, manager_blend,
};
use super::controller_time::{
    controller_next, is_time_controller, set_controller_target, set_next_controller,
};
use super::extra_data::{TextKey, TextKeyExtraData};
use super::object_net::{NiObjectNet, controller_head, get_controller, remove_controller};
use super::particle_controller::{
    ParticleEmitterController, ParticleTimeControllerPayload, controller_time_mut,
    parse_particle_time_controller_payload,
};
use super::particle_modifier_value_controller::{
    ParticleEmitterFloatController, ParticleGravityStrengthController,
    ParticleModifierActiveController,
};
use super::particle_system::ParticleSystem;
use super::scene_node::set_selective_update_flags;
use super::stream_objects::{NULL_REFERENCE, NiClass, StreamObjectTable, read_owned_c_string};
use super::string_palette::StringPalette;
use super::transform_controller::{
    MultiTargetTransformController, NodeController, NodeControllerUpdateStatus,
    TransformController, VisController, add_interpolator_target, spawn_node_controller,
};

const NEGATIVE_INFINITY: f32 = -f32::MAX;
const TRUNCATED: &str = "truncated controller sequence payload";

/// IDA: 006C8820 modern InterpArrayItem read: immediate interpolator (+0), immediate controller (+4), then a u8 priority (+0x0D)
/// only when stream +0x04 (the 00712C00 auxiliary u32) is nonzero.
/// Completion: 100% of the >= 0A01006F field order; the older extra reference and u16 are excluded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ControllerSequenceItemPayload {
    pub interpolator_index: u32,
    pub controller_index: u32,
    pub priority: Option<u8>,
}

/// IDA: 006C8A60 modern IDTag read (>= 0A010071): immediate NiStringPalette (+0), then five u32 values truncated to u16 at +4 / +6 /
/// +8 / +0A / +0C.
/// Completion: 100% of the field order; the < 0A010071 owned-string path is excluded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ControllerSequenceIdTagPayload {
    pub palette_index: u32,
    pub raw_offsets: [u32; 5],
}

/// IDA: 006C8E40 NiControllerSequence::LoadBinary modern branch (>= 0A010068).
/// Completion: 100% of the bounded >= 0A010071 field order, including the [0A010071, 0A030001) discarded phase u32; < 0A010071
/// palette creation (006C5D80) is excluded.
/// TODO: [ ] Original NiStream short-read callbacks and FormHeap failures.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ControllerSequencePayload {
    pub name: Option<Vec<u8>>,
    pub array_size: u32,
    pub array_grow_by: u32,
    pub items: Vec<ControllerSequenceItemPayload>,
    pub id_tags: Vec<ControllerSequenceIdTagPayload>,
    pub weight_raw: u32,
    pub text_keys_index: u32,
    pub cycle_type: u32,
    pub frequency_raw: u32,
    pub begin_key_time_raw: u32,
    pub end_key_time_raw: u32,
    pub manager_index: u32,
    pub accum_root_name: Option<Vec<u8>>,
    pub string_palette_index: u32,
    pub bytes_consumed: usize,
}

// IDA: 006C8E40 modern field order; `stream_auxiliary_04` is NiStream +0x04. 007008A0 NiObject::LoadBinary reads nothing. Each element
// reads at least two item references and a six-u32 IDTag.
// Completion: 100% of the supported widths.
// TODO: [ ] Original short-read callbacks.
pub fn parse_controller_sequence_payload(
    bytes: &[u8],
    stream_version: u32,
    stream_auxiliary_04: u32,
) -> Result<ControllerSequencePayload, GameError> {
    if stream_version < 0x0A01_0071 {
        return Err(GameError::unclassified(
            0,
            "legacy NiControllerSequence payload unsupported",
        ));
    }
    let mut reader = ByteReader::new(bytes);
    let name = read_owned_c_string(
        &mut reader,
        TRUNCATED,
        "truncated controller sequence string",
    )?;
    let array_size = reader.u32(TRUNCATED)?;
    let array_grow_by = reader.u32(TRUNCATED)?;
    if array_size as usize > reader.remaining() / 32 {
        return Err(GameError::stream("truncated NiControllerSequence arrays"));
    }
    let mut items = Vec::with_capacity(array_size as usize);
    let mut id_tags = Vec::with_capacity(array_size as usize);
    for _ in 0..array_size {
        // 006C8820, then 006C8A60, per element.
        let interpolator_index = reader.u32(TRUNCATED)?;
        let controller_index = reader.u32(TRUNCATED)?;
        let priority = if stream_auxiliary_04 != 0 {
            Some(reader.u8(TRUNCATED)?)
        } else {
            None
        };
        items.push(ControllerSequenceItemPayload {
            interpolator_index,
            controller_index,
            priority,
        });
        let palette_index = reader.u32(TRUNCATED)?;
        let mut raw_offsets = [0u32; 5];
        for offset in &mut raw_offsets {
            *offset = reader.u32(TRUNCATED)?;
        }
        id_tags.push(ControllerSequenceIdTagPayload {
            palette_index,
            raw_offsets,
        });
    }
    let weight_raw = reader.u32(TRUNCATED)?;
    let text_keys_index = reader.u32(TRUNCATED)?; // 00712A20 deferred
    let cycle_type = reader.u32(TRUNCATED)?;
    let frequency_raw = reader.u32(TRUNCATED)?;
    if stream_version < 0x0A03_0001 {
        let _discarded_phase = reader.u32(TRUNCATED)?;
    }
    let begin_key_time_raw = reader.u32(TRUNCATED)?;
    let end_key_time_raw = reader.u32(TRUNCATED)?;
    let manager_index = reader.u32(TRUNCATED)?; // 00712A20 deferred
    let accum_root_name = read_owned_c_string(
        &mut reader,
        TRUNCATED,
        "truncated controller sequence string",
    )?;
    let string_palette_index = reader.u32(TRUNCATED)?; // 00712A90 immediate
    Ok(ControllerSequencePayload {
        name,
        array_size,
        array_grow_by,
        items,
        id_tags,
        weight_raw,
        text_keys_index,
        cycle_type,
        frequency_raw,
        begin_key_time_raw,
        end_key_time_raw,
        manager_index,
        accum_root_name,
        string_palette_index,
        bytes_consumed: reader.position(),
    })
}

/// IDA: InterpArrayItem +8 raw NiBlendInterpolator*: either an element of a NiMultiTargetTransformController +3C array (index*0x30) or a
/// controller's own +3C blend. `None` is the cold null.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SequenceBlendRef {
    #[default]
    None,
    MultiTargetSlot {
        controller: Entity,
        index: usize,
    },
    Transform(Entity),
    Bool(Entity),
    Float(Entity),
    Point3(Entity),
}

/// IDA: 006C6370 IDTag cold state: palette NiPointer +0 null, five u16 palette offsets +4..+0C = 0xFFFF (null string).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SequenceIdTag {
    pub palette_00: Option<Entity>,
    pub av_object_name_04: u16,
    pub property_type_06: u16,
    pub controller_type_08: u16,
    pub controller_id_0a: u16,
    pub interpolator_id_0c: u16,
}

impl Default for SequenceIdTag {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            palette_00: None,
            av_object_name_04: 0xFFFF,
            property_type_06: 0xFFFF,
            controller_type_08: 0xFFFF,
            controller_id_0a: 0xFFFF,
            interpolator_id_0c: 0xFFFF,
        }
    }
}

/// IDA: 006C62E0 InterpArrayItem cold state: +0 / +4 / +8 null, +0x0C byte A79EFC (0xFF), +0x0D priority 0xFF.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SequenceInterpItem {
    pub interpolator_00: Option<Entity>,
    pub controller_04: Option<Entity>,
    pub blend_08: SequenceBlendRef,
    pub byte_0c: u8,
    pub priority_0d: u8,
}

impl Default for SequenceInterpItem {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            interpolator_00: None,
            controller_04: None,
            blend_08: SequenceBlendRef::None,
            byte_0c: 0xFF,
            priority_0d: 0xFF,
        }
    }
}

/// IDA: 748530 debug messages emitted by 006C9590 (no control-flow effect).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreTargetsMessage {
    /// 006C96A3 interpolator +0x8C false.
    UnresolvedDependencies,
    /// 006C9845 AVObject not found (name filter).
    MissingTargetObject,
    /// 006C99C4 no controller in chain or item.
    MissingTargetController,
    /// 006C9AEB index 0xFFFF or no blend created.
    MissingInterpolator,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreTargetsStatus {
    /// The original 006C9590 always returns 1.
    Stored,
    // Host stops: the original dereferences a null pointer at these.
    UnsupportedMissingManager,
    UnsupportedMissingPalette,
    UnsupportedPaletteString,
    UnsupportedTargetClass,
    UnsupportedControllerClass,
    /// 006C3B70 accumulate path: NiBlendAccumTransformInterpolator (6C37D0).
    UnsupportedAccumulateBlend,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoreTargetsResult {
    pub status: StoreTargetsStatus,
    pub messages: Vec<(StoreTargetsMessage, usize)>,
}

/// IDA: NiControllerSequence +0x44 AnimState, a u32 field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum SequenceState {
    Inactive = 0,
    Animating = 1,
    EaseIn = 2,
    EaseOut = 3,
    TransSource = 4,
    TransDest = 5,
    MorphSource = 6,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SequenceUpdateStatus {
    Updated,
    Inactive,
    // Host stops: the original dereferences a missing partner, manager or morph key here.
    UnsupportedMissingPartner,
    UnsupportedMissingManager,
    UnsupportedMissingMorphKey,
}

/// IDA: 006C7840 NiControllerSequence factory: FormHeap 0x68, 006C6550 over the NiObject base, vtable A79A34 (RTTI B3CB24
/// "NiControllerSequence"). +0x20, +0x64 and each tag palette are strong in the original; +0x40, +0x58 and +0x60 raw.
/// Completion: 100% of the cold state, modern LoadBinary and LinkObject, StoreTargets for the bound controller and interpolator
/// classes, the runtime family and the modern 006CACE0 PostLoad no-op.
/// TODO: [ ] Text-key callbacks and the legacy (< 0A010068) conversion in 006CACE0.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct ControllerSequence {
    pub name_08: Option<Vec<u8>>,
    pub array_size_0c: u32,
    pub array_grow_by_10: u32,
    pub items_14: Vec<SequenceInterpItem>,
    pub id_tags_18: Vec<SequenceIdTag>,
    pub weight_1c: f32,
    pub text_keys_20: Option<Entity>,
    pub cycle_type_24: u32,
    pub frequency_28: f32,
    pub begin_key_time_2c: f32,
    pub end_key_time_30: f32,
    pub last_time_34: f32,
    pub weighted_last_time_38: f32,
    pub last_scaled_time_3c: f32,
    pub manager_40: Option<Entity>,
    pub state_44: u32,
    pub offset_48: f32,
    pub start_time_4c: f32,
    pub end_time_50: f32,
    pub dest_frame_54: f32,
    pub partner_58: Option<Entity>,
    pub accum_root_name_5c: Option<Vec<u8>>,
    pub accum_root_60: Option<Entity>,
    pub string_palette_64: Option<Entity>,
    pub links_resolved: bool,
}

impl Default for ControllerSequence {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            name_08: None,
            array_size_0c: 0,
            array_grow_by_10: 0,
            items_14: Vec::new(),
            id_tags_18: Vec::new(),
            weight_1c: 1.0,
            text_keys_20: None,
            cycle_type_24: 0,
            frequency_28: 1.0,
            begin_key_time_2c: f32::MAX,
            end_key_time_30: NEGATIVE_INFINITY,
            last_time_34: NEGATIVE_INFINITY,
            weighted_last_time_38: NEGATIVE_INFINITY,
            last_scaled_time_3c: NEGATIVE_INFINITY,
            manager_40: None,
            state_44: 0,
            offset_48: NEGATIVE_INFINITY,
            start_time_4c: NEGATIVE_INFINITY,
            end_time_50: NEGATIVE_INFINITY,
            dest_frame_54: NEGATIVE_INFINITY,
            partner_58: None,
            accum_root_name_5c: None,
            accum_root_60: None,
            string_palette_64: None,
            links_resolved: false,
        }
    }
}

impl ControllerSequence {
    // IDA: 006C8E40 scalar publication: name, sizes, cold arrays (006C62E0 / 006C6370 per element), u16 tag offsets, weight, cycle,
    // frequency, key times and accum root name. Immediate references are bound separately.
    // Completion: 100% of the modern store order for scalar fields.
    pub fn apply_loaded_payload(&mut self, payload: &ControllerSequencePayload) {
        self.name_08.clone_from(&payload.name);
        self.array_size_0c = payload.array_size;
        self.array_grow_by_10 = payload.array_grow_by;
        self.items_14 = vec![SequenceInterpItem::default(); payload.array_size as usize];
        self.id_tags_18 = vec![SequenceIdTag::default(); payload.array_size as usize];
        for (index, item) in payload.items.iter().enumerate() {
            if let Some(priority) = item.priority {
                self.items_14[index].priority_0d = priority;
            }
            let raw = &payload.id_tags[index].raw_offsets;
            let tag = &mut self.id_tags_18[index];
            tag.av_object_name_04 = raw[0] as u16;
            tag.property_type_06 = raw[1] as u16;
            tag.controller_type_08 = raw[2] as u16;
            tag.controller_id_0a = raw[3] as u16;
            tag.interpolator_id_0c = raw[4] as u16;
        }
        self.weight_1c = f32::from_bits(payload.weight_raw);
        self.cycle_type_24 = payload.cycle_type;
        self.frequency_28 = f32::from_bits(payload.frequency_raw);
        self.begin_key_time_2c = f32::from_bits(payload.begin_key_time_raw);
        self.end_key_time_30 = f32::from_bits(payload.end_key_time_raw);
        self.accum_root_name_5c.clone_from(&payload.accum_root_name);
    }

    // IDA: 006C7AD0 modern link: +20 text keys (refcounted) then +40 manager (raw), both 007124A0.
    // Completion: 100% of the >= 0A010068 order.
    pub const fn link_modern(&mut self, keys: Option<Entity>, owner: Option<Entity>) {
        self.text_keys_20 = keys;
        self.manager_40 = owner;
        self.links_resolved = true;
    }

    // IDA: 006C5FC0 ComputeScaledTime(time, store): +38 accumulates frequency * (time - +34); loop cycles (+24 == 0) wrap into
    // [begin, end] with fmod, other cycles clamp; store writes +38, +34, +3C.
    // Completion: 100%
    pub fn compute_scaled_time(&mut self, time: f32, store: bool) -> f32 {
        let mut weighted = self.weighted_last_time_38;
        let delta = if self.last_time_34 == NEGATIVE_INFINITY {
            weighted = 0.0;
            time
        } else {
            time - self.last_time_34
        };
        let scaled = self.frequency_28 * delta + weighted;
        let mut result = scaled;
        if self.cycle_type_24 == 0 {
            let span = self.end_key_time_30 - self.begin_key_time_2c;
            if span == 0.0 {
                result = self.begin_key_time_2c;
            } else {
                let from_begin = scaled - self.begin_key_time_2c;
                let mut wrapped = self.end_key_time_30;
                if span != from_begin {
                    wrapped = float_fmod(from_begin, span) + self.begin_key_time_2c;
                }
                result = if self.begin_key_time_2c > wrapped {
                    span + wrapped
                } else {
                    wrapped
                };
            }
        }
        let mut clamped = self.end_key_time_30;
        if self.end_key_time_30 >= result {
            clamped = if self.begin_key_time_2c <= result {
                result
            } else {
                self.begin_key_time_2c
            };
        }
        if store {
            self.weighted_last_time_38 = scaled;
            self.last_time_34 = time;
            self.last_scaled_time_3c = clamped;
        }
        clamped
    }
}

// IDA: N/A a helper, with no address of its own.
fn sequence(world: &World, entity: Entity) -> &ControllerSequence {
    world
        .get::<ControllerSequence>(entity)
        .expect("a NiControllerSequence entity")
}

// IDA: N/A a helper, with no address of its own.
fn sequence_mut(world: &mut World, entity: Entity) -> Mut<'_, ControllerSequence> {
    world
        .get_mut::<ControllerSequence>(entity)
        .expect("a NiControllerSequence entity")
}

// IDA: N/A C-string view of a host string (an embedded NUL ends it).
// Completion: 100%
fn c_string(text: &[u8]) -> &[u8] {
    text.split(|&byte| byte == 0).next().unwrap_or(text)
}

// IDA: IDTag palette +8 buffer + u16 offset (0xFFFF -> null) as used by 006C63A0 / 006C63C0 / 006C63E0 and the inline 006C9590 /
// 006C8220 reads. Outer `None`: the original would read a null palette, an empty buffer or past the capacity; the host reports
// unsupported instead.
// Completion: 100%
fn tag_string(world: &World, tag: &SequenceIdTag, offset: u16) -> Option<Option<Vec<u8>>> {
    if offset == 0xFFFF {
        return Some(None);
    }
    let bytes = world.get::<StringPalette>(tag.palette_00?)?.bytes();
    let rest = bytes.get(usize::from(offset)..)?;
    let end = rest.iter().position(|&byte| byte == 0)?;
    Some(Some(rest[..end].to_vec()))
}

/// IDA: 006C66B0 NiControllerSequence_GetItemName(index, &out): an item with no interpolator (+0x14's +0x00) answers null; otherwise the tag's AVObject name (+0x04), with a property type (+0x06) "%s\nPROP\n%s" of the two. Completion: 100%.
/// # Errors
/// A tag string the palette can't give (a null palette, an empty buffer, or a read past it).
pub(crate) fn interp_item_name(
    world: &World,
    this: Entity,
    index: usize,
) -> Result<Option<Vec<u8>>, GameError> {
    let state = sequence(world, this);
    if state
        .items_14
        .get(index)
        .is_none_or(|item| item.interpolator_00.is_none())
    {
        return Ok(None);
    }
    let tag = state.id_tags_18[index];
    let name = tag_string(world, &tag, tag.av_object_name_04).ok_or(GameError::unclassified(
        0x006C66D5,
        "006C66D5: an item name the palette can't give",
    ))?;
    let property = tag_string(world, &tag, tag.property_type_06).ok_or(GameError::unclassified(
        0x006C66F0,
        "006C66F0: a property type the palette can't give",
    ))?;
    let Some(property) = property else {
        // 006C6753: strcpy_s of a null name is the CRT's invalid-parameter path.
        return name.map(Some).ok_or(GameError::unclassified(
            0x006C676C,
            "006C676C: strcpy_s of a null item name",
        ));
    };
    let mut out = name.unwrap_or_else(|| b"(null)".to_vec());
    out.extend_from_slice(b"\nPROP\n");
    out.extend_from_slice(&property);
    Ok(Some(out))
}

// IDA: pointer-only uses (+8 buffer + offset) dereference the palette whenever the offset is not 0xFFFF; the string itself is not
// read.
// Completion: 100%
fn tag_palette_reachable(world: &World, tag: &SequenceIdTag, offset: u16) -> bool {
    offset == 0xFFFF
        || tag
            .palette_00
            .is_some_and(|palette| world.get::<StringPalette>(palette).is_some())
}

// IDA: 006C9878 CtlrID pointer nullness. Outer `None`: null palette, or an empty buffer whose original pointer identity is unknown.
// Completion: 100%
fn tag_pointer_nonnull(world: &World, tag: &SequenceIdTag, offset: u16) -> Option<bool> {
    if offset == 0xFFFF {
        return Some(false);
    }
    let palette = world.get::<StringPalette>(tag.palette_00?)?;
    if palette.bytes().is_empty() {
        return None;
    }
    Some(true)
}

// IDA: 006C8220 raw byte test buffer[offset + k] (no terminator check).
// Completion: 100%
fn tag_byte(world: &World, tag: &SequenceIdTag, offset: u16, k: usize) -> Option<u8> {
    world
        .get::<StringPalette>(tag.palette_00?)?
        .bytes()
        .get(usize::from(offset) + k)
        .copied()
}

// IDA: 006C824D's test for NiTransformController: [2] == 'T' and [7] == 'f' on the CtlrType bytes.
// Completion: 100%
fn is_transform_tag(world: &World, tag: &SequenceIdTag) -> Option<bool> {
    if tag.controller_type_08 == 0xFFFF {
        return Some(false);
    }
    if world
        .get::<StringPalette>(tag.palette_00?)?
        .bytes()
        .is_empty()
    {
        return None;
    }
    if tag_byte(world, tag, tag.controller_type_08, 2)? != b'T' {
        return Some(false);
    }
    Some(tag_byte(world, tag, tag.controller_type_08, 7)? == b'f')
}

// IDA: N/A a helper, with no address of its own.
fn class_of(world: &World, entity: Entity) -> Option<NiClass> {
    world.get::<NiClass>(entity).copied()
}

// IDA: vtable +0x04 GetRTTI name: B3CA58 "NiTransformController", B3D80C "NiVisController", B3CD7C "NiMultiTargetTransformController",
// B3CAC0 "NiControllerManager", 0074FB10 "NiPSysEmitterCtlr", 007566F0 "NiPSysModifierActiveCtlr", 00757140 "NiPSysGravityStrengthCtlr".
// Completion: 100% of the exact registered names.
fn rtti_name(world: &World, controller: Entity) -> &'static [u8] {
    match class_of(world, controller) {
        Some(NiClass::TransformController) => b"NiTransformController",
        Some(NiClass::VisController) => b"NiVisController",
        Some(NiClass::MaterialColorController) => b"NiMaterialColorController",
        Some(NiClass::MultiTargetTransformController) => b"NiMultiTargetTransformController",
        Some(NiClass::ControllerManager) => b"NiControllerManager",
        Some(NiClass::ParticleEmitterController) => b"NiPSysEmitterCtlr",
        Some(NiClass::ParticleModifierActiveController) => b"NiPSysModifierActiveCtlr",
        Some(NiClass::ParticleGravityStrengthController) => b"NiPSysGravityStrengthCtlr",
        // 00757C20 and 00757F10 return the concrete speed and life span RTTI records.
        Some(NiClass::ParticleEmitterSpeedController) => b"NiPSysEmitterSpeedCtlr",
        Some(NiClass::ParticleEmitterLifeSpanController) => b"NiPSysEmitterLifeSpanCtlr",
        _ => b"",
    }
}

// IDA: a property's RTTI name, as vt+4's +0 gives it. `None` for a class the host has no name for.
fn property_rtti_name(world: &World, property: Entity) -> Option<&'static [u8]> {
    Some(match class_of(world, property)? {
        NiClass::AlphaProperty => b"NiAlphaProperty",
        NiClass::MaterialProperty => b"NiMaterialProperty",
        NiClass::StencilProperty => b"NiStencilProperty",
        NiClass::TexturingProperty => b"NiTexturingProperty",
        NiClass::VertexColorProperty => b"NiVertexColorProperty",
        NiClass::ZBufferProperty => b"NiZBufferProperty",
        NiClass::SpecularProperty => b"NiSpecularProperty",
        _ => return None,
    })
}

// IDA: vtable +0x8C GetCtlrID: 006ECC80 (+0x40, the modifier name) on the NiPSysModifierCtlr classes; 006DEB00 on
// NiMaterialColorController, by +0x40 & 7: "AMB", "DIFF", "SPEC", "SELF_ILLUM", else null; 006F7070 (null) on the others.
// Completion: 100% of the bound classes.
fn ctlr_id(world: &World, controller: Entity) -> Option<&[u8]> {
    let name = match class_of(world, controller)? {
        NiClass::ParticleEmitterController => world
            .get::<ParticleEmitterController>(controller)?
            .modifier_name_40
            .as_deref(),
        NiClass::ParticleModifierActiveController => world
            .get::<ParticleModifierActiveController>(controller)?
            .base
            .modifier_name_40
            .as_deref(),
        NiClass::ParticleGravityStrengthController => world
            .get::<ParticleGravityStrengthController>(controller)?
            .base
            .modifier_name_40
            .as_deref(),
        NiClass::ParticleEmitterSpeedController | NiClass::ParticleEmitterLifeSpanController => {
            world
                .get::<ParticleEmitterFloatController>(controller)?
                .base
                .modifier_name_40
                .as_deref()
        }
        NiClass::MaterialColorController => {
            let held = world.get::<super::material_controllers::MaterialController>(controller)?;
            let super::material_controllers::MaterialControllerKind::Color { target_color_40 } =
                held.kind
            else {
                return None;
            };
            return match target_color_40 & 7 {
                0 => Some(b"AMB".as_slice()),
                1 => Some(b"DIFF".as_slice()),
                2 => Some(b"SPEC".as_slice()),
                3 => Some(b"SELF_ILLUM".as_slice()),
                _ => None,
            };
        }
        _ => None,
    };
    name.map(c_string)
}

// IDA: vtable +0x58: 0075E5C0 on the NiPSysModifierCtlr classes clears +0x44, runs 00715CE0, then with a target looks the +0x40 name up in its
// +0xD4 map for +0x44. 00715CE0 itself on the others.
// Completion: 100% of the bound classes with one of those two at +0x58.
// TODO: [ ] The other +0x58 owners run 00715CE0 here: NiControllerManager's is the shared no-op 0068F970; 006D0760
// NiGeomMorpherController_SetTarget (a target that isn't geometry takes SetTarget(null); with +0x50 morph data, vt+0xA8 and
// +0x5B = 1); 006D6C00 NiTextureTransformController_SetTarget (a new target clears +0x44 first).
fn set_target_virtual(
    world: &mut World,
    controller: Entity,
    target: Option<Entity>,
) -> Result<(), GameError> {
    if let Some(mut owner) = world.get_mut::<ParticleEmitterFloatController>(controller) {
        owner.base.clear_cache_before_set_target();
        set_controller_target(world, controller, target)?;
        if let Some(system) = target.and_then(|e| world.get::<ParticleSystem>(e)) {
            let name = world
                .get::<ParticleEmitterFloatController>(controller)
                .expect("an emitter float controller")
                .base
                .modifier_name_40
                .as_deref();
            let cached = name.and_then(|name| system.find_modifier(name));
            let mut owner = world
                .get_mut::<ParticleEmitterFloatController>(controller)
                .expect("an emitter float controller");
            owner.base.cached_modifier_44 = cached;
            owner.base.cached_modifier_linked = cached.is_some();
        }
        return Ok(());
    }
    set_controller_target(world, controller, target)?;
    let found = |world: &World, name: Option<&[u8]>| {
        target
            .and_then(|target| world.get::<ParticleSystem>(target))
            .zip(name)
            .and_then(|(system, name)| system.find_modifier(name))
    };
    match class_of(world, controller) {
        Some(NiClass::ParticleEmitterController) => {
            let name = world
                .get::<ParticleEmitterController>(controller)
                .expect("a NiPSysEmitterCtlr entity")
                .modifier_name_40
                .clone();
            let cached = found(world, name.as_deref());
            let mut owner = world
                .get_mut::<ParticleEmitterController>(controller)
                .expect("a NiPSysEmitterCtlr entity");
            owner.cached_modifier_44 = cached;
            owner.cached_modifier_linked = cached.is_some();
        }
        Some(NiClass::ParticleModifierActiveController) => {
            let name = world
                .get::<ParticleModifierActiveController>(controller)
                .expect("a NiPSysModifierActiveCtlr entity")
                .base
                .modifier_name_40
                .clone();
            let cached = found(world, name.as_deref());
            let mut owner = world
                .get_mut::<ParticleModifierActiveController>(controller)
                .expect("a NiPSysModifierActiveCtlr entity");
            owner.base.cached_modifier_44 = cached;
            owner.base.cached_modifier_linked = cached.is_some();
        }
        Some(NiClass::ParticleGravityStrengthController) => {
            let name = world
                .get::<ParticleGravityStrengthController>(controller)
                .expect("a NiPSysGravityStrengthCtlr entity")
                .base
                .modifier_name_40
                .clone();
            let cached = found(world, name.as_deref());
            let mut owner = world
                .get_mut::<ParticleGravityStrengthController>(controller)
                .expect("a NiPSysGravityStrengthCtlr entity");
            owner.base.cached_modifier_44 = cached;
            owner.base.cached_modifier_linked = cached.is_some();
        }
        _ => {}
    }
    Ok(())
}

// IDA: MT +0x40 target search (006C8467 / 006C69B2): first index whose raw target equals obj, else 0xFFFF (A7A160).
// Completion: 100%
fn find_target_slot(multi: &MultiTargetTransformController, target: Entity) -> u16 {
    for index in 0..multi.interpolator_count_44 {
        let Some(slot) = multi.targets_40.get(usize::from(index)) else {
            break;
        };
        if *slot == Some(target) {
            return index;
        }
    }
    0xFFFF
}

// IDA: N/A a helper, with no address of its own.
fn multi_target_mut(world: &mut World, entity: Entity) -> Mut<'_, MultiTargetTransformController> {
    world
        .get_mut::<MultiTargetTransformController>(entity)
        .expect("a NiMultiTargetTransformController entity")
}

// IDA: N/A a palette's object as 006C9590 takes it, a NiAVObject, with no class test of its own.
// Completion: 100% of the bound classes.
fn is_store_target(world: &World, entity: Entity) -> bool {
    class_of(world, entity).is_some_and(NiClass::is_av_object)
        && world.get::<NiObjectNet>(entity).is_some()
}

/// IDA: 006C3730's 0x68-byte item of NiBlendAccumTransformInterpolator +0x50 (one per blend slot): +0x00 the last time (-FLT_MAX by 006C3500), +0x04 and +0x24 two NiQuatTransforms (00B24260's translate, 00B3CBA4's rotate, A79E10's scale; 006C3500 then marks each translate.x, rotate.x and scale -FLT_MAX), +0x44 a rotation (the identity).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AccumTransformItem {
    pub last_time_00: f32,
    /// +0x04 the source's value at +0x00 (006CEE20).
    pub value_04: QuatTransform,
    /// +0x24 the source's change over one loop (its range end times its start's inverse), made when the time goes back.
    pub cycle_delta_24: QuatTransform,
    pub rotation_44: [f32; 9],
}

/// IDA: 006C37D0 NiBlendAccumTransformInterpolator(managerControlled, threshold, arraySize) (0x58, vtable A79584) past its NiBlendInterpolator base (`ParticleBlendTransformInterpolator` on the same entity): +0x30 the value (the invalid NiQuatTransform), +0x50 the items (`AccumTransformItem`, one per +0x0D slot), +0x54 a byte 0. Completion: 100% of the cold state; its vt+0x4C Update (006CF780) is `blend_accum::update_blend_accum`'s.
/// TODO: [ ] The items' growth with the blend array, and 006CF780's several-item arm.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct BlendAccumTransformState {
    pub value_30: QuatTransform,
    pub items_50: Vec<AccumTransformItem>,
    /// +0x54 set: the next single-item Update rebases +0x30 on the item's value (006CF630).
    pub reset_54: u8,
}

impl BlendAccumTransformState {
    // IDA: 006C37D0's stores past the base.
    fn new(capacity: u8) -> Self {
        let item = AccumTransformItem {
            last_time_00: -f32::MAX,
            value_04: QuatTransform::default(),
            cycle_delta_24: QuatTransform::default(),
            rotation_44: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
        };
        Self {
            value_30: QuatTransform::default(),
            items_50: vec![item; usize::from(capacity)],
            reset_54: 0,
        }
    }
}

// IDA: 0074F910's and the +0x98 classes' FormHeap_Allocate(0x34) NiBlendFloatInterpolator (006D2480), as `manager_blend` with -FLT_MAX at +0x30.
// Completion: 100%
fn create_float_blend(world: &mut World) -> Entity {
    world
        .spawn((
            NiClass::BlendFloatInterpolator,
            manager_blend::<ParticleBlendFloatInterpolator>(),
        ))
        .id()
}

// IDA: 006EC530 and 0074F910's and 0075F320's NiBlendBoolInterpolator.
// Completion: 100%
fn create_bool_blend(world: &mut World) -> Entity {
    world
        .spawn((
            NiClass::BlendBoolInterpolator,
            manager_blend::<ParticleBlendBoolInterpolator>(),
        ))
        .id()
}

// IDA: 00560920's NiDynamicCast(NiBlendInterpolator) on +0x80's result, for a particle controller's two kinds, else the `create` stored through +0x84.
// Completion: 100% of the bound classes.
fn particle_blend(
    world: &mut World,
    held: Option<Entity>,
    float: bool,
    set: impl FnOnce(&mut World, Entity),
) -> SequenceBlendRef {
    match held.and_then(|held| class_of(world, held).map(|class| (held, class))) {
        Some((held, NiClass::BlendFloatInterpolator)) => SequenceBlendRef::Float(held),
        Some((held, NiClass::BlendBoolInterpolator)) => SequenceBlendRef::Bool(held),
        _ if float => {
            let created = create_float_blend(world);
            set(world, created);
            SequenceBlendRef::Float(created)
        }
        _ => {
            let created = create_bool_blend(world);
            set(world, created);
            SequenceBlendRef::Bool(created)
        }
    }
}

// IDA: 006C8220 Bethesda NiTransformController -> NiMultiTargetTransform conversion. Returns its AL result (true only when it created
// one).
// Completion: 100% of the verified store order.
fn bind_multi_target_transforms(
    world: &mut World,
    this: Entity,
    root: Option<Entity>,
    palette: Entity,
) -> Result<bool, StoreTargetsStatus> {
    use StoreTargetsStatus as Status;
    let owner = sequence(world, this)
        .manager_40
        .filter(|&owner| world.get::<ControllerManager>(owner).is_some());
    let (Some(owner), Some(root)) = (owner, root) else {
        return Err(Status::UnsupportedMissingManager);
    };
    let array_size = sequence(world, this).array_size_0c as usize;
    // 006C824D: thiscall GetController(B3CD7C) on the root argument.
    let mut multi = get_controller(world, root, NiClass::MultiTargetTransformController);
    let mut created = false;
    if multi.is_none() {
        if array_size == 0 {
            return Ok(false);
        }
        let mut count = 0u16;
        for index in 0..array_size {
            let tag = sequence(world, this).id_tags_18[index];
            let matched = is_transform_tag(world, &tag).ok_or(Status::UnsupportedPaletteString)?;
            if matched {
                count = count.wrapping_add(1);
            }
        }
        if count == 0 {
            return Ok(false);
        }
        // 006C82D1: FormHeap 0x48, 006C5EE0(count + 10).
        let mut state = MultiTargetTransformController::default();
        state.set_interpolator_count(count.wrapping_add(10));
        let new_multi = world
            .spawn((
                NiClass::MultiTargetTransformController,
                NodeController::default(),
                state,
            ))
            .id();
        // Vtable +0x58, 00715CE0 on this class, whose +0x60 is false.
        set_controller_target(world, new_multi, Some(root))
            .map_err(|_| Status::UnsupportedTargetClass)?;
        world
            .get_mut::<NodeController>(new_multi)
            .expect("just spawned")
            .time
            .fields
            .flags |= 0x0020;
        created = true;
        let manager_target = world
            .get::<NodeController>(owner)
            .ok_or(Status::UnsupportedMissingManager)?
            .time
            .target_30;
        if Some(root) == manager_target {
            remove_controller(world, root, new_multi);
            // +0x34 is read after the remove, which may have changed it.
            let manager_next = controller_next(world, owner);
            set_next_controller(world, new_multi, manager_next);
            set_next_controller(world, owner, Some(new_multi));
        }
        multi = Some(new_multi);
    }
    let multi = multi.expect("found or created above");
    for index in 0..array_size {
        let tag = sequence(world, this).id_tags_18[index];
        let matched = is_transform_tag(world, &tag).ok_or(Status::UnsupportedPaletteString)?;
        if !matched {
            continue;
        }
        let object_name = tag_string(world, &tag, tag.av_object_name_04)
            .flatten()
            .ok_or(Status::UnsupportedPaletteString)?;
        let palette_state = world
            .get::<DefaultAvObjectPalette>(palette)
            .ok_or(Status::UnsupportedMissingPalette)?;
        let Some(object) = palette_state.get_av_object(&object_name) else {
            continue;
        };
        let cumulative = world
            .get::<ControllerManager>(owner)
            .expect("checked above")
            .cumulative_6c;
        if sequence(world, this).accum_root_60 == Some(object) && cumulative {
            continue;
        }
        if !is_store_target(world, object) {
            return Err(Status::UnsupportedTargetClass);
        }
        let existing = get_controller(world, object, NiClass::TransformController);
        let multi_state = world
            .get::<MultiTargetTransformController>(multi)
            .expect("a NiMultiTargetTransformController entity");
        let mut slot = find_target_slot(multi_state, object);
        if slot != 0xFFFF {
            // 006C84A7 rewrites the same pointer.
            multi_target_mut(world, multi).targets_40[usize::from(slot)] = Some(object);
        } else {
            slot = add_interpolator_target(&mut multi_target_mut(world, multi), object);
            if slot == 0xFFFF {
                continue;
            }
        }
        let name = world
            .get::<NiObjectNet>(object)
            .and_then(|net| net.name.clone())
            .ok_or(Status::UnsupportedTargetClass)?;
        if stricmp(c_string(&name), b"Bip01") != 0
            && let Some(existing) = existing
        {
            remove_controller(world, object, existing);
        }
        {
            let mut state = sequence_mut(world, this);
            state.items_14[index].controller_04 = Some(multi);
            state.items_14[index].blend_08 = SequenceBlendRef::MultiTargetSlot {
                controller: multi,
                index: usize::from(slot),
            };
        }
        multi_target_mut(world, multi).blend_interpolators_3c[usize::from(slot)]
            .base
            .flags_0c |= 1;
    }
    Ok(created)
}

// IDA: 006C6910: re-point items at the Bip01 parent's multi-target slots.
// Completion: 100%
fn rebind_parent_multi_target(
    world: &mut World,
    this: Entity,
    parent: Entity,
    palette: Entity,
) -> StoreTargetsStatus {
    let Some(multi) = get_controller(world, parent, NiClass::MultiTargetTransformController) else {
        return StoreTargetsStatus::Stored;
    };
    let array_size = sequence(world, this).array_size_0c as usize;
    for index in 0..array_size {
        let tag = sequence(world, this).id_tags_18[index];
        let Some(Some(object_name)) = tag_string(world, &tag, tag.av_object_name_04) else {
            return StoreTargetsStatus::UnsupportedPaletteString;
        };
        let Some(palette_state) = world.get::<DefaultAvObjectPalette>(palette) else {
            return StoreTargetsStatus::UnsupportedMissingPalette;
        };
        let Some(object) = palette_state.get_av_object(&object_name) else {
            continue;
        };
        let slot = find_target_slot(
            world
                .get::<MultiTargetTransformController>(multi)
                .expect("a NiMultiTargetTransformController entity"),
            object,
        );
        if slot != 0xFFFF {
            sequence_mut(world, this).items_14[index].blend_08 =
                SequenceBlendRef::MultiTargetSlot {
                    controller: multi,
                    index: usize::from(slot),
                };
        }
    }
    StoreTargetsStatus::Stored
}

// IDA: 006C9590 StoreTargets(root). Palette = manager +7C. Linked host nodes have no property slots, so 006C9702's RTTI scan never
// matches.
// Completion: 100% of the verified branch order for the bound controller and interpolator classes, on any NiAVObject without a property type.
// TODO: [ ] Non-node NiAVObject property lists.
pub fn store_targets(world: &mut World, this: Entity, root: Option<Entity>) -> StoreTargetsResult {
    use StoreTargetsStatus as Status;
    let mut messages = Vec::new();
    macro_rules! fail {
        ($status:expr) => {
            return StoreTargetsResult {
                status: $status,
                messages,
            }
        };
    }
    let Some(owner) = sequence(world, this)
        .manager_40
        .filter(|&owner| world.get::<ControllerManager>(owner).is_some())
    else {
        fail!(Status::UnsupportedMissingManager);
    };
    let Some(palette) = world
        .get::<ControllerManager>(owner)
        .expect("checked above")
        .object_palette_7c
        .filter(|&palette| world.get::<DefaultAvObjectPalette>(palette).is_some())
    else {
        fail!(Status::UnsupportedMissingPalette);
    };
    let get_object = |world: &World, name: &[u8]| {
        world
            .get::<DefaultAvObjectPalette>(palette)
            .expect("checked above")
            .get_av_object(name)
    };
    if let Some(name) = sequence(world, this).accum_root_name_5c.clone() {
        let found = get_object(world, &name);
        sequence_mut(world, this).accum_root_60 = found;
    }
    let mut any_bound = match bind_multi_target_transforms(world, this, root, palette) {
        Ok(created) => created,
        Err(status) => fail!(status),
    };
    let mut bip02_seen = false;
    let array_size = sequence(world, this).array_size_0c as usize;
    for index in 0..array_size {
        let item = sequence(world, this).items_14[index];
        let tag = sequence(world, this).id_tags_18[index];
        // Interpolator +0x8C ResolveDependencies is 006FDFF0 (true) for every bound class, so 006C96A3 is never reached here.
        if item.interpolator_00.is_none() || item.blend_08 != SequenceBlendRef::None {
            continue;
        }
        let Some(Some(object_name)) = tag_string(world, &tag, tag.av_object_name_04) else {
            fail!(Status::UnsupportedPaletteString);
        };
        let Some(object) = get_object(world, &object_name) else {
            // 006C976F: Bip01 with a Bip02 sets the flag; the listed prefixes ("Bow" is Source_0 A451F8) suppress the message.
            if bip02_seen {
                continue;
            }
            let text = &object_name;
            if stricmp(text, b"Bip01") == 0 && get_object(world, b"Bip02").is_some() {
                bip02_seen = true;
            } else if strnicmp(b"Bow", text, 3) != 0
                && strnicmp(b"Arrow", text, 5) != 0
                && strnicmp(b"Bip01 Tail", text, 10) != 0
                && strnicmp(b"Bridle", text, 6) != 0
            {
                // 006C9822 / 006C982A compute the message arguments.
                if !tag_palette_reachable(world, &tag, tag.property_type_06)
                    || !tag_palette_reachable(world, &tag, tag.controller_type_08)
                {
                    fail!(Status::UnsupportedPaletteString);
                }
                messages.push((StoreTargetsMessage::MissingTargetObject, index));
            }
            continue;
        };
        if !is_store_target(world, object) {
            fail!(Status::UnsupportedTargetClass);
        }
        // 006C96EE: a nonnull property type walks the +0x9C list for the first property whose RTTI name (vt+4's +0) strcmp-equals
        // it, which becomes the target; with none the target stays the NiAVObject.
        let object = match tag_pointer_nonnull(world, &tag, tag.property_type_06) {
            None => fail!(Status::UnsupportedPaletteString),
            Some(false) => object,
            Some(true) => {
                let Some(Some(property_type)) = tag_string(world, &tag, tag.property_type_06)
                else {
                    fail!(Status::UnsupportedPaletteString);
                };
                let properties = world
                    .get::<super::av_object::NiAvProperties>(object)
                    .map(|list| list.0.clone())
                    .unwrap_or_default();
                let mut matched = None;
                for property in properties.into_iter().flatten() {
                    let Some(name) = property_rtti_name(world, property) else {
                        fail!(Status::UnsupportedTargetClass);
                    };
                    if name == property_type.as_slice() {
                        matched = Some(property);
                        break;
                    }
                }
                matched.unwrap_or(object)
            }
        };
        // 006C9852: chain scan by exact RTTI name, then GetCtlrID (+0x8C): both null, or both nonnull and strcmp equal.
        let mut found = None;
        let mut current = controller_head(world, object);
        while let Some(entry) = current.filter(|_| is_time_controller(world, current)) {
            let Some(Some(controller_type)) = tag_string(world, &tag, tag.controller_type_08)
            else {
                fail!(Status::UnsupportedPaletteString);
            };
            if rtti_name(world, entry) == controller_type.as_slice() {
                if class_of(world, entry) == Some(NiClass::ControllerManager) {
                    fail!(Status::UnsupportedControllerClass);
                }
                let Some(controller_id) = tag_string(world, &tag, tag.controller_id_0a) else {
                    fail!(Status::UnsupportedPaletteString);
                };
                let matched = match (ctlr_id(world, entry), controller_id.as_deref()) {
                    (None, None) => true,
                    (Some(own), Some(id)) => own == id,
                    _ => false,
                };
                if matched {
                    found = Some(entry);
                    break;
                }
            }
            current = controller_next(world, entry);
        }
        let found = if let Some(found) = found {
            found
        } else {
            // 006C98F7: the item's own controller is retargeted.
            let Some(controller) = item.controller_04 else {
                messages.push((StoreTargetsMessage::MissingTargetController, index));
                continue;
            };
            if set_target_virtual(world, controller, Some(object)).is_err() {
                fail!(Status::UnsupportedTargetClass);
            }
            any_bound = true;
            let manager_target = world
                .get::<NodeController>(owner)
                .expect("a NiControllerManager has time state")
                .time
                .target_30;
            if Some(object) == manager_target {
                remove_controller(world, object, controller);
                // +0x34 is read after the remove, which may have changed it.
                let manager_next = controller_next(world, owner);
                set_next_controller(world, controller, manager_next);
                set_next_controller(world, owner, Some(controller));
            }
            controller
        };
        // 006C9940: GetInterpolatorIndex(+0x7C) with the InterpID pointer, GetInterpolator(+0x80), 00560920 NiBlendInterpolator cast,
        // then CreateBlendInterpolator(+0x98, idx, 1, accumulate, 0.0, 2) and SetInterpolator(+0x84) when the cast fails.
        if !tag_palette_reachable(world, &tag, tag.interpolator_id_0c) {
            fail!(Status::UnsupportedPaletteString);
        }
        let cumulative = world
            .get::<ControllerManager>(owner)
            .expect("checked above")
            .cumulative_6c;
        let accumulate = sequence(world, this).accum_root_60 == Some(object) && cumulative;
        let blend = match class_of(world, found) {
            Some(NiClass::TransformController) => {
                // 006CE1A0 index 0, 006CE200 +3C, 006C3B70 create, 006CE210 set.
                let held = world
                    .get::<TransformController>(found)
                    .expect("a NiTransformController entity")
                    .interpolator_3c
                    .filter(|&held| {
                        matches!(
                            class_of(world, held),
                            Some(
                                NiClass::BlendTransformInterpolator
                                    | NiClass::BlendAccumTransformInterpolator
                            )
                        )
                    });
                let existing = if let Some(held) = held {
                    held
                } else if accumulate {
                    // 006C3B70 accumulate -> FormHeap(0x58) 006C37D0 NiBlendAccumTransformInterpolator.
                    let created = world
                        .spawn((
                            NiClass::BlendAccumTransformInterpolator,
                            manager_blend::<ParticleBlendTransformInterpolator>(),
                            BlendAccumTransformState::new(2),
                        ))
                        .id();
                    world
                        .get_mut::<TransformController>(found)
                        .expect("a NiTransformController entity")
                        .interpolator_3c = Some(created);
                    created
                } else {
                    let created = world
                        .spawn((
                            NiClass::BlendTransformInterpolator,
                            manager_blend::<ParticleBlendTransformInterpolator>(),
                        ))
                        .id();
                    world
                        .get_mut::<TransformController>(found)
                        .expect("a NiTransformController entity")
                        .interpolator_3c = Some(created);
                    created
                };
                SequenceBlendRef::Transform(existing)
            }
            Some(NiClass::VisController) => {
                // 006CE1A0 / 006CE200, 006EC530 NiBlendBoolInterpolator, 006CE210.
                let held = world
                    .get::<VisController>(found)
                    .expect("a NiVisController entity")
                    .interpolator_3c
                    .filter(|&held| class_of(world, held) == Some(NiClass::BlendBoolInterpolator));
                let existing = if let Some(held) = held {
                    held
                } else {
                    let created = world
                        .spawn((
                            NiClass::BlendBoolInterpolator,
                            manager_blend::<ParticleBlendBoolInterpolator>(),
                        ))
                        .id();
                    world
                        .get_mut::<VisController>(found)
                        .expect("a NiVisController entity")
                        .interpolator_3c = Some(created);
                    created
                };
                SequenceBlendRef::Bool(existing)
            }
            Some(NiClass::MaterialColorController) => {
                // 006CE1A0 / 006CE200, 006E2620 (FormHeap 0x40, 006EA850: the NiBlendInterpolator base, +0x30 the invalid point,
                // +0x3C 0), 006CE210.
                use super::material_controllers::{
                    BlendPoint3Interpolator, INVALID_POINT_BITS, MaterialController,
                };
                let held = world
                    .get::<MaterialController>(found)
                    .expect("a NiMaterialColorController entity")
                    .interpolator_3c
                    .filter(|&held| {
                        class_of(world, held) == Some(NiClass::BlendPoint3Interpolator)
                    });
                let existing = if let Some(held) = held {
                    held
                } else {
                    let mut blend = manager_blend::<BlendPoint3Interpolator>();
                    blend.value_bits_30 = [INVALID_POINT_BITS; 3];
                    blend.valid_3c = false;
                    let created = world.spawn((NiClass::BlendPoint3Interpolator, blend)).id();
                    world
                        .get_mut::<MaterialController>(found)
                        .expect("a NiMaterialColorController entity")
                        .interpolator_3c = Some(created);
                    created
                };
                SequenceBlendRef::Point3(existing)
            }
            Some(NiClass::MultiTargetTransformController) => {
                // 006CFB80: null ID -> 0xFFFF; else the first nonnull target whose +8 name strcmp-equals the ID. 006CFC00 returns
                // &blend[idx].
                let mut slot = 0xFFFFu16;
                if tag.interpolator_id_0c != 0xFFFF {
                    let Some(Some(interpolator_id)) =
                        tag_string(world, &tag, tag.interpolator_id_0c)
                    else {
                        fail!(Status::UnsupportedPaletteString);
                    };
                    let multi = world
                        .get::<MultiTargetTransformController>(found)
                        .expect("a NiMultiTargetTransformController entity");
                    for candidate in 0..multi.interpolator_count_44 {
                        let Some(target) = multi.targets_40.get(usize::from(candidate)) else {
                            break;
                        };
                        let Some(target) = *target else {
                            continue;
                        };
                        if !is_store_target(world, target) {
                            fail!(Status::UnsupportedTargetClass);
                        }
                        let Some(name) = world
                            .get::<NiObjectNet>(target)
                            .and_then(|net| net.name.as_deref())
                        else {
                            fail!(Status::UnsupportedTargetClass);
                        };
                        if c_string(name) == interpolator_id.as_slice() {
                            slot = candidate;
                            break;
                        }
                    }
                }
                if slot == 0xFFFF {
                    messages.push((StoreTargetsMessage::MissingInterpolator, index));
                    continue;
                }
                SequenceBlendRef::MultiTargetSlot {
                    controller: found,
                    index: usize::from(slot),
                }
            }
            // 0074F870: stricmp against B283E0 "BirthRate" and B283E4 "EmitterActive", else 0xFFFF. 0074FCF0 and 0074FD20 read and store +3C or
            // +48, and 0074F910 makes a float blend for 0 and a bool blend for 1.
            Some(NiClass::ParticleEmitterController) => {
                let Some(Some(interpolator_id)) = tag_string(world, &tag, tag.interpolator_id_0c)
                else {
                    fail!(Status::UnsupportedPaletteString);
                };
                let owner = world
                    .get::<ParticleEmitterController>(found)
                    .expect("a NiPSysEmitterCtlr entity");
                if stricmp(&interpolator_id, b"BirthRate") == 0 {
                    let held = owner.birth_rate_interpolator_3c;
                    particle_blend(world, held, true, |world, created| {
                        world
                            .get_mut::<ParticleEmitterController>(found)
                            .expect("a NiPSysEmitterCtlr entity")
                            .birth_rate_interpolator_3c = Some(created);
                    })
                } else if stricmp(&interpolator_id, b"EmitterActive") == 0 {
                    let held = owner.active_interpolator_48;
                    particle_blend(world, held, false, |world, created| {
                        world
                            .get_mut::<ParticleEmitterController>(found)
                            .expect("a NiPSysEmitterCtlr entity")
                            .active_interpolator_48 = Some(created);
                    })
                } else {
                    messages.push((StoreTargetsMessage::MissingInterpolator, index));
                    continue;
                }
            }
            // 006CE1A0 index 0, 006CE200 +3C, 0075F320, 006CE210.
            Some(NiClass::ParticleModifierActiveController) => {
                let held = world
                    .get::<ParticleModifierActiveController>(found)
                    .expect("a NiPSysModifierActiveCtlr entity")
                    .interpolator_3c;
                particle_blend(world, held, false, |world, created| {
                    world
                        .get_mut::<ParticleModifierActiveController>(found)
                        .expect("a NiPSysModifierActiveCtlr entity")
                        .interpolator_3c = Some(created);
                })
            }
            // 006CE1A0 index 0, 006CE200 +3C, 0075F570, 006CE210.
            Some(NiClass::ParticleGravityStrengthController) => {
                let held = world
                    .get::<ParticleGravityStrengthController>(found)
                    .expect("a NiPSysGravityStrengthCtlr entity")
                    .interpolator_3c;
                particle_blend(world, held, true, |world, created| {
                    world
                        .get_mut::<ParticleGravityStrengthController>(found)
                        .expect("a NiPSysGravityStrengthCtlr entity")
                        .interpolator_3c = Some(created);
                })
            }
            Some(
                NiClass::ParticleEmitterSpeedController
                | NiClass::ParticleEmitterLifeSpanController,
            ) => {
                let held = world
                    .get::<ParticleEmitterFloatController>(found)
                    .expect("an emitter float controller")
                    .interpolator_3c;
                particle_blend(world, held, true, |world, created| {
                    world
                        .get_mut::<ParticleEmitterFloatController>(found)
                        .expect("an emitter float controller")
                        .interpolator_3c = Some(created);
                })
            }
            _ => fail!(Status::UnsupportedControllerClass),
        };
        controller_time_mut(world, found)
            .expect("a controller")
            .fields
            .flags |= 0x0020;
        let mut state = sequence_mut(world, this);
        state.items_14[index].controller_04 = Some(found);
        state.items_14[index].blend_08 = blend;
    }
    // 006C9B35: manager +0x30 vtable +0x5C SetSelectiveUpdateFlags(&b, 1, &v).
    if any_bound {
        let Some(target) = world
            .get::<NodeController>(owner)
            .expect("a NiControllerManager has time state")
            .time
            .target_30
        else {
            fail!(Status::UnsupportedMissingManager);
        };
        let mut selective = false;
        let mut rigid = true;
        // Each stop of the selective-update walk is a class with no host arm.
        if set_selective_update_flags(world, target, &mut selective, true, &mut rigid).is_err() {
            fail!(Status::UnsupportedTargetClass);
        }
    }
    let mut status = Status::Stored;
    // 006C9B54: Bip02 (seen or present) -> Bip01 +0x1C parent -> 006C6910.
    if (bip02_seen || get_object(world, b"Bip02").is_some())
        && let Some(bip01) = get_object(world, b"Bip01")
    {
        if !is_store_target(world, bip01) {
            fail!(Status::UnsupportedTargetClass);
        }
        if let Some(parent) = world.get::<NiAvParent>(bip01).and_then(|parent| parent.0) {
            status = rebind_parent_multi_target(world, this, parent, palette);
        }
    }
    StoreTargetsResult { status, messages }
}

/// IDA: 006C4E50 NiControllerManager::LoadBinary: 00715F40 NiTimeController fields, u8 cumulative (+6C = byte != 0), 00712AE0 deferred
/// sequence list, then (>= 0A010068) the immediate +7C NiDefaultAVObjectPalette.
/// Completion: 100% of the bounded modern field order.
#[derive(Clone, Debug, PartialEq)]
pub struct ControllerManagerPayload {
    pub time: ParticleTimeControllerPayload,
    pub cumulative_raw: u8,
    pub sequence_indices: Vec<u32>,
    pub palette_index: u32,
    pub bytes_consumed: usize,
}

// IDA: 006C4E50 field order.
// Completion: 100% of the supported widths.
// TODO: [ ] Original NiStream short-read callbacks.
pub fn parse_controller_manager_payload(
    bytes: &[u8],
    stream_version: u32,
) -> Result<ControllerManagerPayload, GameError> {
    if stream_version < 0x0A01_0068 {
        return Err(GameError::unclassified(
            0,
            "legacy NiControllerManager payload unsupported",
        ));
    }
    let time = parse_particle_time_controller_payload(bytes, stream_version)?;
    let mut reader = ByteReader::new(&bytes[time.bytes_consumed..]);
    let cumulative_raw = reader.u8(TRUNCATED)?;
    let count = reader.u32(TRUNCATED)?; // 00712AE0 count then link IDs
    if count as usize > reader.remaining() / 4 {
        return Err(GameError::stream(
            "truncated NiControllerManager sequence list",
        ));
    }
    let mut sequence_indices = Vec::with_capacity(count as usize);
    for _ in 0..count {
        sequence_indices.push(reader.u32(TRUNCATED)?);
    }
    let palette_index = reader.u32(TRUNCATED)?;
    Ok(ControllerManagerPayload {
        time,
        cumulative_raw,
        sequence_indices,
        palette_index,
        bytes_consumed: time.bytes_consumed + reader.position(),
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControllerManagerAddStatus {
    Added,
    /// 006C578D seq +40 already set.
    AlreadyManaged,
    // Host stops: the original dereferences null (seq, target +8 or the map key).
    UnsupportedNullSequence,
    UnsupportedMissingTarget,
    UnsupportedNullName,
    UnsupportedStoreTargets,
}

/// One +0x58 map entry. The key is a host copy of the sequence's +08 name at insertion; the original keeps the pointer (copy flag 0).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ControllerManagerMapEntry {
    pub key: Vec<u8>,
    pub value: Entity,
}

/// IDA: 006C5BB0 NiControllerManager factory: FormHeap 0x80, 006C5520 over 00715990 NiTimeController, vtable A79804 (RTTI B3CAC0).
/// Sequence NiTArray +3C (data +40, capacity +44, size +46, count +48, grow +4A = 10), active array +4C / +50 / +54, NiTStringPointerMap
/// +58 (37 buckets, copy flag +68 = 0), cumulative +6C, +70 / +74 / +78 zero, object palette +7C. Its time state is `NodeController`.
/// Completion: 100% of the cold state, load, link (AddSequence without StoreTargets) and the 006C4260 PostLoad.
/// TODO: [ ] The +70..+78 fields.
#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct ControllerManager {
    /// The vector's length is the +44 capacity.
    pub sequence_slots_40: Vec<Option<Entity>>,
    pub sequence_size_46: u16,
    pub sequence_count_48: u16,
    pub sequence_grow_by_4a: u16,
    pub active_sequences_4c: Vec<Entity>,
    pub sequence_map_58: [Vec<ControllerManagerMapEntry>; ControllerManager::BUCKET_COUNT],
    pub cumulative_6c: bool,
    pub object_palette_7c: Option<Entity>,
}

impl Default for ControllerManager {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            sequence_slots_40: Vec::new(),
            sequence_size_46: 0,
            sequence_count_48: 0,
            sequence_grow_by_4a: 10,
            active_sequences_4c: Vec::new(),
            sequence_map_58: std::array::from_fn(|_| Vec::new()),
            cumulative_6c: false,
            object_palette_7c: None,
        }
    }
}

impl ControllerManager {
    pub const BUCKET_COUNT: usize = 37;

    // IDA: 007DAED0 NiTStringPointerMap_CalculateBucket: h = 33*h + signed byte, modulo 37.
    // Completion: 100% of the raw-byte hash without case folding.
    fn map_bucket(key: &[u8]) -> usize {
        ni_string_hash(key) as usize % Self::BUCKET_COUNT
    }

    // IDA: 006C4510 NiTArray::SetSize.
    // Completion: 100% of the slot release and copy order.
    pub fn set_sequence_capacity(&mut self, capacity: u16) {
        if usize::from(capacity) == self.sequence_slots_40.len() {
            return;
        }
        if capacity < self.sequence_size_46 {
            for index in usize::from(capacity)..usize::from(self.sequence_size_46) {
                if self.sequence_slots_40[index].take().is_some() {
                    self.sequence_count_48 = self.sequence_count_48.wrapping_sub(1);
                }
            }
            self.sequence_size_46 = capacity;
        }
        let mut resized = vec![None; usize::from(capacity)];
        resized[..usize::from(self.sequence_size_46)]
            .copy_from_slice(&self.sequence_slots_40[..usize::from(self.sequence_size_46)]);
        self.sequence_slots_40 = resized;
    }

    // IDA: 006C4940 NiTArray::SetAt with the size and count bookkeeping.
    // Completion: 100%
    pub fn set_sequence_at(
        &mut self,
        index: u16,
        sequence: Option<Entity>,
    ) -> Result<(), GameError> {
        let at = usize::from(index);
        if at >= self.sequence_slots_40.len() {
            return Err(GameError::unclassified(
                0,
                "NiTArray::SetAt beyond capacity",
            ));
        }
        if index >= self.sequence_size_46 {
            self.sequence_size_46 = index.wrapping_add(1);
            if sequence.is_some() {
                self.sequence_count_48 = self.sequence_count_48.wrapping_add(1);
            }
        } else if sequence.is_none() {
            if self.sequence_slots_40[at].is_some() {
                self.sequence_count_48 = self.sequence_count_48.wrapping_sub(1);
            }
        } else if self.sequence_slots_40[at].is_none() {
            self.sequence_count_48 = self.sequence_count_48.wrapping_add(1);
        }
        self.sequence_slots_40[at] = sequence;
        Ok(())
    }

    // IDA: 006C5240 NiTArray::AddFirstEmpty (0xFFFFFFFF for null). 006C526A scans downward and keeps the lowest index of the topmost
    // null run.
    // Completion: 100%
    pub fn add_first_empty(&mut self, sequence: Option<Entity>) -> Result<u32, GameError> {
        if sequence.is_none() {
            return Ok(0xFFFF_FFFF);
        }
        if self.sequence_count_48 < self.sequence_size_46 {
            let mut found = self.sequence_size_46;
            for index in (0..self.sequence_size_46).rev() {
                if self.sequence_slots_40[usize::from(index)].is_none() {
                    found = index;
                } else if found != self.sequence_size_46 {
                    break;
                }
            }
            if found != self.sequence_size_46 {
                self.sequence_slots_40[usize::from(found)] = sequence;
                self.sequence_count_48 = self.sequence_count_48.wrapping_add(1);
                return Ok(u32::from(found));
            }
        }
        let index = self.sequence_size_46;
        if usize::from(index) >= self.sequence_slots_40.len() {
            self.set_sequence_capacity(index.wrapping_add(self.sequence_grow_by_4a));
        }
        self.set_sequence_at(index, sequence)?;
        Ok(u32::from(index))
    }

    // IDA: 0055E000 NiTPointerMap_Lookup on the +58 map (vtable A798A0: 007DAED0 hash, 00584D10 case-insensitive compare).
    // Completion: 100% of the bucket-scoped lookup.
    #[must_use]
    pub fn find_sequence(&self, key: &[u8]) -> Option<Entity> {
        let lookup = c_string(key);
        self.sequence_map_58[Self::map_bucket(lookup)]
            .iter()
            .find(|entry| stricmp(&entry.key, lookup) == 0)
            .map(|entry| entry.value)
    }

    // IDA: 006C4E50 publishes the NiTimeController fields, then +6C. The time fields go to the entity's `NodeController`.
    // Completion: 100% of the supported publication order.
    pub const fn apply_loaded_payload(&mut self, payload: &ControllerManagerPayload) {
        self.cumulative_6c = payload.cumulative_raw != 0;
    }
}

// IDA: 006C5760 AddSequence(seq, name, storeTargets): manager check, +40 store, accum root name from the target (0049F4D0 frees +5C,
// then copies a nonnull source), optional StoreTargets, SetName (00434930), AddFirstEmpty, then 00412D30 insert-or-assign keyed by the
// sequence +08 name; a null key would fault in the 007DAED0 hash.
// Completion: 100% of the verified order.
pub fn add_sequence(
    world: &mut World,
    manager: Entity,
    new_sequence: Option<Entity>,
    new_name: Option<&[u8]>,
    run_store_targets: bool,
) -> Result<ControllerManagerAddStatus, GameError> {
    use ControllerManagerAddStatus as Status;
    let Some(this) = new_sequence else {
        return Ok(Status::UnsupportedNullSequence);
    };
    if sequence(world, this).manager_40.is_some() {
        return Ok(Status::AlreadyManaged);
    }
    sequence_mut(world, this).manager_40 = Some(manager);
    let target = world
        .get::<NodeController>(manager)
        .ok_or(GameError::unclassified(
            0,
            "controller manager time state missing",
        ))?
        .time
        .target_30;
    if sequence(world, this).accum_root_name_5c.is_none() {
        let Some(target) = target else {
            return Ok(Status::UnsupportedMissingTarget);
        };
        let name = world
            .get::<NiObjectNet>(target)
            .and_then(|net| net.name.clone());
        sequence_mut(world, this).accum_root_name_5c = name;
    }
    if run_store_targets {
        let result = store_targets(world, this, target);
        if result.status != StoreTargetsStatus::Stored {
            return Ok(Status::UnsupportedStoreTargets);
        }
    }
    if let Some(name) = new_name {
        sequence_mut(world, this).name_08 = Some(name.to_vec());
    }
    let name = sequence(world, this).name_08.clone();
    let mut state = world
        .get_mut::<ControllerManager>(manager)
        .ok_or(GameError::unclassified(
            0,
            "controller manager state missing",
        ))?;
    state.add_first_empty(Some(this))?;
    let Some(name) = name else {
        return Ok(Status::UnsupportedNullName);
    };
    let key = c_string(&name);
    let bucket = &mut state.sequence_map_58[ControllerManager::map_bucket(key)];
    if let Some(entry) = bucket
        .iter_mut()
        .find(|entry| stricmp(&entry.key, key) == 0)
    {
        entry.value = this;
        return Ok(Status::Added);
    }
    bucket.insert(
        0,
        ControllerManagerMapEntry {
            key: key.to_vec(),
            value: this,
        },
    );
    Ok(Status::Added)
}

// IDA: 006C4260 PostLoad: 0068F970, then StoreTargets(target +30) for each nonnull slot below +46 (result ignored in the original).
// Completion: 100% of the loop order; false reports an unsupported StoreTargets.
pub fn controller_manager_post_load(world: &mut World, manager: Entity) -> bool {
    let root = world
        .get::<NodeController>(manager)
        .expect("a NiControllerManager has time state")
        .time
        .target_30;
    let size = world
        .get::<ControllerManager>(manager)
        .expect("a NiControllerManager entity")
        .sequence_size_46;
    for index in 0..usize::from(size) {
        let slot = world
            .get::<ControllerManager>(manager)
            .expect("a NiControllerManager entity")
            .sequence_slots_40[index];
        let Some(this) = slot else {
            continue;
        };
        if store_targets(world, this, root).status != StoreTargetsStatus::Stored {
            return false;
        }
    }
    true
}

// IDA: 006C8A60 / 006C8E40 immediate NiStringPalette references. `Ok(None)` for another class; a null reference is `Some(None)`.
// Completion: 100% of the exact typed identity.
fn string_palette_owner(
    world: &World,
    table: &StreamObjectTable,
    reference: u32,
) -> Result<Option<Option<Entity>>, GameError> {
    if reference == NULL_REFERENCE {
        return Ok(Some(None));
    }
    let entry = table
        .objects
        .get(reference as usize)
        .ok_or(GameError::unclassified(
            0,
            "controller sequence palette out of range",
        ))?;
    if class_of(world, entry.entity) != Some(NiClass::StringPalette) {
        return Ok(None);
    }
    Ok(Some(Some(entry.entity)))
}

// IDA: 006C8E40's load on a cold sequence: the scalars, then 006C8820 / 006C8A60 per element: interpolator, controller, then the IDTag
// palette; finally the +64 string palette (006C93C1). `Ok(false)` for an unsupported class; earlier stores stay, as in the C++ graph
// case this comes from.
// Completion: 100% of the order.
pub fn load_controller_sequence(
    world: &mut World,
    this: Entity,
    payload: &ControllerSequencePayload,
    table: &StreamObjectTable,
) -> Result<bool, GameError> {
    sequence_mut(world, this).apply_loaded_payload(payload);
    for (slot, item) in payload.items.iter().enumerate() {
        if item.interpolator_index != NULL_REFERENCE {
            let entity = table
                .objects
                .get(item.interpolator_index as usize)
                .ok_or(GameError::unclassified(
                    0,
                    "sequence interpolator out of range",
                ))?
                .entity;
            if !class_of(world, entity).is_some_and(NiClass::is_sequence_interpolator) {
                return Ok(false);
            }
            sequence_mut(world, this).items_14[slot].interpolator_00 = Some(entity);
        }
        if item.controller_index != NULL_REFERENCE {
            let entity = table
                .objects
                .get(item.controller_index as usize)
                .ok_or(GameError::unclassified(
                    0,
                    "sequence controller out of range",
                ))?
                .entity;
            if !class_of(world, entity).is_some_and(NiClass::is_sequence_controller) {
                return Ok(false);
            }
            sequence_mut(world, this).items_14[slot].controller_04 = Some(entity);
        }
        let Some(palette) =
            string_palette_owner(world, table, payload.id_tags[slot].palette_index)?
        else {
            return Ok(false);
        };
        sequence_mut(world, this).id_tags_18[slot].palette_00 = palette;
    }
    let Some(palette) = string_palette_owner(world, table, payload.string_palette_index)? else {
        return Ok(false);
    };
    sequence_mut(world, this).string_palette_64 = palette;
    Ok(true)
}

// IDA: 006C7AD0: 007124A0 text keys (+20), then manager (+40). `Ok(false)` for an unsupported class.
// Completion: 100% of the >= 0A010068 order.
pub fn link_controller_sequence(
    world: &mut World,
    this: Entity,
    payload: &ControllerSequencePayload,
    table: &StreamObjectTable,
) -> Result<bool, GameError> {
    let mut keys = None;
    if payload.text_keys_index != NULL_REFERENCE {
        let entity = table
            .objects
            .get(payload.text_keys_index as usize)
            .ok_or(GameError::unclassified(
                0,
                "sequence text keys out of range",
            ))?
            .entity;
        if class_of(world, entity) != Some(NiClass::TextKeyExtraData) {
            return Ok(false);
        }
        keys = Some(entity);
    }
    let mut manager = None;
    if payload.manager_index != NULL_REFERENCE {
        let entity = table
            .objects
            .get(payload.manager_index as usize)
            .ok_or(GameError::unclassified(0, "sequence manager out of range"))?
            .entity;
        if class_of(world, entity) != Some(NiClass::ControllerManager) {
            return Ok(false);
        }
        manager = Some(entity);
    }
    sequence_mut(world, this).link_modern(keys, manager);
    Ok(true)
}

// IDA: 006C4E50's load: the NiTimeController fields, +6C, then 006C4EA4's immediate +7C NiDefaultAVObjectPalette. `Ok(false)` for an
// unsupported class.
// Completion: 100% of the order.
pub fn load_controller_manager(
    world: &mut World,
    manager: Entity,
    payload: &ControllerManagerPayload,
    table: &StreamObjectTable,
) -> Result<bool, GameError> {
    world
        .get_mut::<NodeController>(manager)
        .ok_or(GameError::unclassified(
            0,
            "controller manager time state missing",
        ))?
        .time
        .apply_loaded_payload(&payload.time);
    world
        .get_mut::<ControllerManager>(manager)
        .ok_or(GameError::unclassified(
            0,
            "controller manager state missing",
        ))?
        .apply_loaded_payload(payload);
    if payload.palette_index != NULL_REFERENCE {
        let entity = table
            .objects
            .get(payload.palette_index as usize)
            .ok_or(GameError::unclassified(
                0,
                "controller manager palette out of range",
            ))?
            .entity;
        if class_of(world, entity) != Some(NiClass::DefaultAvObjectPalette) {
            return Ok(false);
        }
        world
            .get_mut::<ControllerManager>(manager)
            .expect("checked above")
            .object_palette_7c = Some(entity);
    }
    Ok(true)
}

// IDA: 006C5C20: 00715E70 base link, 007124D0 count, 006C4510 SetSize, then AddSequence(seq, null, false) per entry. A null entry faults
// in 006C5760 (seq +40 read), which the host reports as `Ok(false)`, as it does an entry of another class.
// Completion: 100% of the order.
pub fn link_controller_manager(
    world: &mut World,
    manager: Entity,
    payload: &ControllerManagerPayload,
    table: &StreamObjectTable,
    stream_version: u32,
) -> Result<bool, GameError> {
    use super::transform_controller::link_node_controller;
    if !link_node_controller(world, manager, &payload.time, table, stream_version)? {
        return Ok(false);
    }
    if payload.sequence_indices.len() > 0xFFFF {
        return Ok(false);
    }
    world
        .get_mut::<ControllerManager>(manager)
        .ok_or(GameError::unclassified(
            0,
            "controller manager state missing",
        ))?
        .set_sequence_capacity(payload.sequence_indices.len() as u16);
    for &reference in &payload.sequence_indices {
        if reference == NULL_REFERENCE {
            return Ok(false);
        }
        let entity = table
            .objects
            .get(reference as usize)
            .ok_or(GameError::unclassified(0, "manager sequence out of range"))?
            .entity;
        if class_of(world, entity) != Some(NiClass::ControllerSequence) {
            return Ok(false);
        }
        let added = add_sequence(world, manager, Some(entity), None, false)?;
        if added != ControllerManagerAddStatus::Added
            && added != ControllerManagerAddStatus::AlreadyManaged
        {
            return Ok(false);
        }
    }
    Ok(true)
}

// IDA: N/A the blend an InterpArrayItem +8 raw pointer resolves to, for the C++ `visit_blend`. A missing owner is a host no-op. `f` also takes
// the test for which sources that blend adds.
// Completion: 100%
fn visit_blend<R>(
    world: &mut World,
    reference: SequenceBlendRef,
    f: impl FnOnce(&mut dyn BlendInterpolator, fn(NiClass) -> bool) -> R,
) -> Option<R> {
    match reference {
        SequenceBlendRef::None => None,
        SequenceBlendRef::MultiTargetSlot { controller, index } => {
            let mut multi = world.get_mut::<MultiTargetTransformController>(controller)?;
            let blend = multi.blend_interpolators_3c.get_mut(index)?;
            Some(f(blend, NiClass::is_blend_transform_source))
        }
        SequenceBlendRef::Transform(blend) => {
            let mut blend = world.get_mut::<ParticleBlendTransformInterpolator>(blend)?;
            Some(f(&mut *blend, NiClass::is_blend_transform_source))
        }
        SequenceBlendRef::Bool(blend) => {
            let mut blend = world.get_mut::<ParticleBlendBoolInterpolator>(blend)?;
            Some(f(&mut *blend, NiClass::is_blend_bool_source))
        }
        SequenceBlendRef::Float(blend) => {
            let mut blend = world.get_mut::<ParticleBlendFloatInterpolator>(blend)?;
            Some(f(&mut *blend, NiClass::is_blend_float_source))
        }
        SequenceBlendRef::Point3(blend) => {
            let mut blend =
                world.get_mut::<super::material_controllers::BlendPoint3Interpolator>(blend)?;
            Some(f(&mut *blend, NiClass::is_blend_point3_source))
        }
    }
}

// IDA: 0073A5E0 NiTPrimitiveArray AddUnique on manager +0x4C (doubling growth).
// Completion: 100%
fn add_unique_active(world: &mut World, this: Entity) {
    let Some(owner) = sequence(world, this).manager_40 else {
        return;
    };
    let Some(mut manager) = world.get_mut::<ControllerManager>(owner) else {
        return;
    };
    if !manager.active_sequences_4c.contains(&this) {
        manager.active_sequences_4c.push(this);
    }
}

// IDA: N/A a helper, with no address of its own.
fn morph_prefix(text: Option<&Vec<u8>>) -> bool {
    text.is_some_and(|text| strnicmp(text, b"m:", 2) == 0)
}

// IDA: N/A a helper, with no address of its own.
fn same_key_text(left: Option<&Vec<u8>>, right: Option<&Vec<u8>>) -> bool {
    left.is_some() && left == right
}

// IDA: N/A a helper, with no address of its own.
fn same_key_text_ignore_case(left: Option<&Vec<u8>>, right: Option<&Vec<u8>>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => stricmp(left, right) == 0,
        _ => false,
    }
}

// IDA: N/A a helper, with no address of its own.
fn key_time(key: &TextKey) -> f32 {
    f32::from_bits(key.time_bits)
}

// IDA: N/A a helper, with no address of its own.
fn text_keys_of(world: &World, this: Entity) -> Option<&[TextKey]> {
    sequence(world, this)
        .text_keys_20
        .and_then(|keys| world.get::<TextKeyExtraData>(keys))
        .map(|keys| keys.keys.as_slice())
}

// IDA: 006C6DC0 SetInterpsWeightAndTime(weight, ease, time) on every item's blend slot (+0x0C index); the single active item takes the
// time in blend +0x20.
// Completion: 100%; an out-of-range slot index is a host no-op.
pub fn set_interps_weight_and_time(
    world: &mut World,
    this: Entity,
    new_weight: f32,
    ease: f32,
    time: f32,
) {
    for at in 0..sequence(world, this).items_14.len() {
        let item = sequence(world, this).items_14[at];
        visit_blend(world, item.blend_08, |blend, _| {
            blend.sync_capacity();
            let (base, _, _) = blend.slots();
            let index = item.byte_0c;
            if index >= base.capacity_0d {
                return;
            }
            let at = usize::from(index);
            let single = |base: &ParticleBlendCold| {
                base.active_count_0e == 1 && index == base.single_index_0f
            };
            let invalidate = |base: &mut ParticleBlendCold| {
                base.high_weight_sum_24 = NEGATIVE_INFINITY;
                base.second_weight_sum_28 = NEGATIVE_INFINITY;
                base.flags_0c |= 4;
                base.high_ease_2c = NEGATIVE_INFINITY;
            };
            if !single(base) {
                base.items_14[at].weight = new_weight;
                invalidate(base);
            }
            if !single(base) {
                base.items_14[at].ease = ease;
                invalidate(base);
            }
            if single(base) {
                base.single_time_20 = time;
            } else {
                base.items_14[at].runtime_time = time;
            }
        });
    }
}

// IDA: 006C6A50 AttachInterpolators(priority): per item with an interpolator and a blend, +0x0C = AddInterpInfo(interp, 0.0, item
// priority or the sequence priority when 0xFF, 1.0).
// Completion: 100% for blend sources whose class matches the blend type.
pub fn attach_interpolators(world: &mut World, this: Entity, priority: i8) {
    for at in 0..sequence(world, this).items_14.len() {
        let item = sequence(world, this).items_14[at];
        let Some(interpolator) = item.interpolator_00 else {
            continue;
        };
        let item_priority = if item.priority_0d == 0xFF {
            priority
        } else {
            item.priority_0d as i8
        };
        let class = class_of(world, interpolator);
        let added = visit_blend(world, item.blend_08, |blend, accepts| {
            let accepted = class.is_some_and(accepts);
            accepted.then(|| blend.add_interp_info(interpolator, 0.0, item_priority, 1.0))
        });
        if let Some(Some(index)) = added {
            sequence_mut(world, this).items_14[at].byte_0c = index;
        }
    }
}

// IDA: 006C6AC0 DetachInterpolators: RemoveInterpInfo(+0x0C) per item with a blend; the returned reference is released.
// Completion: 100%
pub fn detach_interpolators(world: &mut World, this: Entity) {
    for at in 0..sequence(world, this).items_14.len() {
        let item = sequence(world, this).items_14[at];
        visit_blend(world, item.blend_08, |blend, _| {
            blend.remove_interp_info(item.byte_0c);
        });
    }
}

// IDA: 006C6110: false when this sequence is in the candidate's +58 chain. The original walks the chain without a bound; activation
// keeps it acyclic, and the host stops after a generous step limit.
// Completion: 100%
#[must_use]
pub fn can_sync_to(world: &World, this: Entity, candidate: Entity) -> bool {
    let mut link = sequence(world, candidate).partner_58;
    let mut step = 0;
    while let Some(entry) = link {
        if step >= 4096 {
            break;
        }
        if entry == this {
            return false;
        }
        link = sequence(world, entry).partner_58;
        step += 1;
    }
    true
}

// IDA: 006C6ED0 VerifyMatchingMorphKeys: both key sets must contain the other's "m:" keys (strcmp) and at least one each.
// Completion: 100%
#[must_use]
pub fn verify_matching_morph_keys(world: &World, this: Entity, other: Entity) -> bool {
    let (Some(mine), Some(theirs)) = (text_keys_of(world, this), text_keys_of(world, other)) else {
        return false;
    };
    if theirs.is_empty() {
        return false;
    }
    let mut found_theirs = false;
    for key in theirs {
        if !morph_prefix(key.text.as_ref()) {
            continue;
        }
        found_theirs = true;
        if mine.is_empty() {
            return false;
        }
        if !mine
            .iter()
            .any(|own| same_key_text(key.text.as_ref(), own.text.as_ref()))
        {
            return false;
        }
    }
    if !found_theirs || mine.is_empty() {
        return false;
    }
    let mut found_mine = false;
    for own in mine {
        if !morph_prefix(own.text.as_ref()) {
            continue;
        }
        found_mine = true;
        if !theirs
            .iter()
            .any(|key| same_key_text(own.text.as_ref(), key.text.as_ref()))
        {
            return false;
        }
    }
    found_mine
}

// IDA: 006C6B30 FindCorrespondingMorphFrame(other, time): the two "m:" keys of `other` around its scaled time, wrapping at the ends;
// the matching keys in this sequence by `_stricmp`; the same ratio between them, wrapped into this sequence's key range.
// Completion: 100%
#[expect(
    clippy::assign_op_pattern,
    reason = "006C6B30's operand order, span then key time"
)]
pub fn find_corresponding_morph_frame(
    world: &mut World,
    this: Entity,
    other: Entity,
    time: f32,
) -> Option<f32> {
    let scaled = sequence_mut(world, other).compute_scaled_time(time, false);
    let (other_begin, other_end) = {
        let state = sequence(world, other);
        (state.begin_key_time_2c, state.end_key_time_30)
    };
    let mut before: Option<&TextKey> = None;
    let mut after: Option<&TextKey> = None;
    let mut first: Option<&TextKey> = None;
    let mut last: Option<&TextKey> = None;
    if let Some(keys) = text_keys_of(world, other) {
        for key in keys {
            if !morph_prefix(key.text.as_ref()) {
                continue;
            }
            last = Some(key);
            if first.is_none() {
                first = Some(key);
            }
            if scaled > key_time(key) {
                before = Some(key);
            } else if after.is_none() {
                after = Some(key);
            }
        }
    }
    let mut wrapped = false;
    let before_time;
    let ratio;
    if let Some(after_key) = after {
        if let Some(before_key) = before {
            before_time = key_time(before_key);
        } else {
            let last_key = last?;
            before = Some(last_key);
            before_time = other_begin - other_end + key_time(last_key);
        }
        ratio = (scaled - before_time) / (key_time(after_key) - before_time);
    } else {
        let before_key = before?;
        let first_key = first?;
        after = Some(first_key);
        wrapped = true;
        before_time = key_time(before_key);
        let after_time = other_end - other_begin + key_time(first_key);
        ratio = (scaled - before_time) / (after_time - before_time);
    }
    let own = sequence(world, this);
    let (begin, end) = (own.begin_key_time_2c, own.end_key_time_30);
    let mut own_before: Option<usize> = None;
    let mut own_after: Option<usize> = None;
    let mut before_done = before.is_none();
    let mut after_done = after.is_none();
    let own_keys = text_keys_of(world, this);
    if let Some(keys) = own_keys {
        for (index, key) in keys.iter().enumerate() {
            if before_done && after_done {
                break;
            }
            if !before_done
                && same_key_text_ignore_case(
                    key.text.as_ref(),
                    before.and_then(|key| key.text.as_ref()),
                )
            {
                own_before = Some(index);
                before_done = true;
            }
            if !after_done
                && same_key_text_ignore_case(
                    key.text.as_ref(),
                    after.and_then(|key| key.text.as_ref()),
                )
            {
                own_after = Some(index);
                after_done = true;
            }
        }
    }
    let own_time =
        |index: usize| key_time(&own_keys.expect("an index came from these keys")[index]);
    let mut low = 0.0f32;
    let mut high = 0.0f32;
    let span = end - begin;
    if own_before == own_after {
        let index = own_before?;
        low = own_time(index);
        high = own_time(index);
        if wrapped {
            high = span + high;
        } else {
            low -= span;
        }
    } else {
        if let Some(index) = own_after {
            high = own_time(index);
        }
        if let Some(index) = own_before {
            low = own_time(index);
        }
        if high < low {
            high = span + high;
        }
    }
    let result = (high - low) * ratio + low;
    if begin > result {
        return Some(result + span);
    }
    if end < result {
        return Some(result - span);
    }
    Some(result)
}

// IDA: 006C9BA0 Activate(priority, startOver, weight, easeIn, timeSync, transition): fails unless inactive; a time-sync sequence must pass
// the 006C6110 chain check and 006C6ED0 morph-key match and becomes +58; 006C6A50 attaches the interpolators; weight +1C; easeIn <= 0 ->
// animating, else trans-dest (transition) or ease-in with start -FLT_MAX and end = easeIn; a newly active sequence is added to manager
// +4C (0073A5E0, unique); startOver resets +48.
// Completion: 100%
#[expect(
    clippy::too_many_arguments,
    reason = "006C9BA0's six arguments, on the world and this"
)]
pub fn activate_sequence(
    world: &mut World,
    this: Entity,
    priority: i8,
    start_over: bool,
    new_weight: f32,
    ease_in_time: f32,
    time_sync: Option<Entity>,
    transition: bool,
) -> bool {
    if sequence(world, this).state_44 != 0 {
        return false;
    }
    sequence_mut(world, this).partner_58 = None;
    if let Some(sync) = time_sync {
        let sync_partner = sequence(world, sync).partner_58;
        if sync_partner.is_some_and(|partner| partner == this || !can_sync_to(world, this, partner))
            || !verify_matching_morph_keys(world, this, sync)
        {
            return false;
        }
        sequence_mut(world, this).partner_58 = Some(sync);
    }
    attach_interpolators(world, this, priority);
    sequence_mut(world, this).weight_1c = new_weight;
    let was_inactive = sequence(world, this).state_44 == 0;
    if ease_in_time <= 0.0 {
        sequence_mut(world, this).state_44 = SequenceState::Animating as u32;
        if was_inactive {
            add_unique_active(world, this);
        }
    } else {
        sequence_mut(world, this).state_44 = if transition {
            SequenceState::TransDest as u32
        } else {
            SequenceState::EaseIn as u32
        };
        if was_inactive {
            add_unique_active(world, this);
        }
        let mut state = sequence_mut(world, this);
        state.start_time_4c = NEGATIVE_INFINITY;
        state.end_time_50 = ease_in_time;
    }
    if start_over {
        sequence_mut(world, this).offset_48 = NEGATIVE_INFINITY;
    }
    true
}

// IDA: 006C9D30 StartBlend(dest, duration, destFrame, priority, sourceWeight, destWeight, timeSync): this->Deactivate(0, true); a duration
// <= 0 becomes 0.0001; this re-attaches and animates with the source weight; dest->Activate(priority, false, destWeight, duration,
// timeSync, true) must succeed; then this->Deactivate(duration, true), this +54 = +3C / frequency and dest +54 = destFrame / dest frequency.
// Completion: 100%
#[expect(
    clippy::too_many_arguments,
    reason = "006C9D30's seven arguments, on the world and this"
)]
pub fn start_blend(
    world: &mut World,
    this: Entity,
    destination: Option<Entity>,
    mut duration: f32,
    destination_frame: f32,
    priority: i8,
    source_weight: f32,
    destination_weight: f32,
    time_sync: Option<Entity>,
) -> bool {
    deactivate_sequence(world, this, 0.0, true);
    if duration <= 0.0 {
        #[expect(
            clippy::excessive_precision,
            reason = "006C9D30's literal as the C++ port writes it"
        )]
        {
            duration = 0.000_099_999_997;
        }
    }
    let Some(destination) = destination.filter(|_| sequence(world, this).state_44 == 0) else {
        return false;
    };
    sequence_mut(world, this).partner_58 = None;
    attach_interpolators(world, this, priority);
    let was_inactive = sequence(world, this).state_44 == 0;
    {
        let mut state = sequence_mut(world, this);
        state.weight_1c = source_weight;
        state.state_44 = SequenceState::Animating as u32;
    }
    if was_inactive {
        add_unique_active(world, this);
    }
    if !activate_sequence(
        world,
        destination,
        priority,
        false,
        destination_weight,
        duration,
        time_sync,
        true,
    ) {
        return false;
    }
    deactivate_sequence(world, this, duration, true);
    {
        let mut state = sequence_mut(world, this);
        state.dest_frame_54 = state.last_scaled_time_3c / state.frequency_28;
    }
    let mut state = sequence_mut(world, destination);
    state.dest_frame_54 = destination_frame / state.frequency_28;
    true
}

// IDA: 006C9E00 StartMorph(dest, duration, priority, sourceWeight, destWeight): this->Deactivate(0, true); this->Activate(priority, false,
// sourceWeight, duration, dest, true) must succeed with dest inactive; dest attaches and becomes animating (duration <= 0) or trans-dest
// with end = duration, its +48 resets, and this enters morph-source (6).
// Completion: 100%
pub fn start_morph(
    world: &mut World,
    this: Entity,
    destination: Option<Entity>,
    duration: f32,
    priority: i8,
    source_weight: f32,
    destination_weight: f32,
) -> bool {
    deactivate_sequence(world, this, 0.0, true);
    let Some(destination) = destination else {
        return false;
    };
    if !activate_sequence(
        world,
        this,
        priority,
        false,
        source_weight,
        duration,
        Some(destination),
        true,
    ) || sequence(world, destination).state_44 != 0
    {
        return false;
    }
    sequence_mut(world, destination).partner_58 = None;
    attach_interpolators(world, destination, priority);
    sequence_mut(world, destination).weight_1c = destination_weight;
    let destination_was_inactive = sequence(world, destination).state_44 == 0;
    if duration <= 0.0 {
        sequence_mut(world, destination).state_44 = SequenceState::Animating as u32;
        if destination_was_inactive {
            add_unique_active(world, destination);
        }
    } else {
        sequence_mut(world, destination).state_44 = SequenceState::TransDest as u32;
        if destination_was_inactive {
            add_unique_active(world, destination);
        }
        let mut state = sequence_mut(world, destination);
        state.start_time_4c = NEGATIVE_INFINITY;
        state.end_time_50 = duration;
    }
    sequence_mut(world, destination).offset_48 = NEGATIVE_INFINITY;
    let was_inactive = sequence(world, this).state_44 == 0;
    sequence_mut(world, this).state_44 = SequenceState::MorphSource as u32;
    if was_inactive {
        add_unique_active(world, this);
    }
    true
}

// IDA: 006C9CB0 Deactivate(easeOut, transition): fails when inactive; easeOut <= 0 folds the last weighted time into +48 (when +34 is set),
// clears the state, partner and dest frame and detaches (006C6AC0); otherwise state 4 (transition) or 3 with start -FLT_MAX, end easeOut.
// Completion: 100%
#[expect(
    clippy::assign_op_pattern,
    reason = "006C9CB0's operand order, +48 last"
)]
pub fn deactivate_sequence(
    world: &mut World,
    this: Entity,
    ease_out_time: f32,
    transition: bool,
) -> bool {
    if sequence(world, this).state_44 == 0 {
        return false;
    }
    if ease_out_time <= 0.0 {
        {
            let mut state = sequence_mut(world, this);
            if state.last_time_34 != NEGATIVE_INFINITY {
                state.offset_48 = state.weighted_last_time_38 / state.frequency_28
                    - state.last_time_34
                    + state.offset_48;
            }
            state.state_44 = SequenceState::Inactive as u32;
            state.partner_58 = None;
            state.dest_frame_54 = NEGATIVE_INFINITY;
        }
        detach_interpolators(world, this);
        return true;
    }
    let mut state = sequence_mut(world, this);
    state.state_44 = if transition {
        SequenceState::TransSource as u32
    } else {
        SequenceState::EaseOut as u32
    };
    state.start_time_4c = NEGATIVE_INFINITY;
    state.end_time_50 = ease_out_time;
    true
}

// IDA: N/A the raw +0x4C clamp the 006CA950 branches share.
// Completion: 100%
fn clamp_to_start(world: &World, this: Entity, now: &mut f32) {
    let start = sequence(world, this).start_time_4c;
    if start > *now {
        *now = start;
    }
}

// IDA: the 006CA950 trans-source arm, also reached from morph-source: at the end time Deactivate(0, true) and no interpolator pass;
// otherwise the spinner falls across the window.
// Completion: 100%
fn trans_source_step(
    world: &mut World,
    this: Entity,
    now: &mut f32,
    transition_spinner: &mut f32,
    apply: &mut bool,
) {
    if sequence(world, this).end_time_50 <= *now {
        deactivate_sequence(world, this, 0.0, true);
        *apply = false;
    } else {
        clamp_to_start(world, this, now);
        let state = sequence(world, this);
        *transition_spinner =
            (state.end_time_50 - *now) / (state.end_time_50 - state.start_time_4c);
    }
}

// IDA: 006CA950 Update(time, updateInterpolators). Inactive returns. A -FLT_MAX +48 becomes -time. A -FLT_MAX +4C anchors start = time and
// end += time. Then per state: animating stores +3C to +4C; ease-in, ease-out, trans-dest and trans-source advance their spinners or finish;
// morph-source aligns the partner's +48 through 006C6B30 and turns into trans-source. Finally the update time is +54, or the partner's
// corresponding morph frame, or now + +48, for 006C5FC0 and 006C6DC0.
// Completion: 100% of the order; x87 intermediates use host floats.
pub fn update_sequence(
    world: &mut World,
    this: Entity,
    time: f32,
    update_interpolators: bool,
) -> SequenceUpdateStatus {
    use SequenceUpdateStatus as Status;
    if sequence(world, this).state_44 == 0 {
        return Status::Inactive;
    }
    let mut now = time;
    {
        let mut state = sequence_mut(world, this);
        if state.offset_48 == NEGATIVE_INFINITY {
            state.offset_48 = -now;
        }
        if state.start_time_4c == NEGATIVE_INFINITY {
            state.start_time_4c = time;
            state.end_time_50 += now;
        }
    }
    let mut ease = 1.0f32;
    let mut transition_spinner = 1.0f32;
    let mut apply = true;
    const ANIMATING: u32 = SequenceState::Animating as u32;
    const EASE_IN: u32 = SequenceState::EaseIn as u32;
    const EASE_OUT: u32 = SequenceState::EaseOut as u32;
    const TRANS_SOURCE: u32 = SequenceState::TransSource as u32;
    const TRANS_DEST: u32 = SequenceState::TransDest as u32;
    const MORPH_SOURCE: u32 = SequenceState::MorphSource as u32;
    match sequence(world, this).state_44 {
        ANIMATING => {
            let mut state = sequence_mut(world, this);
            state.start_time_4c = state.last_scaled_time_3c;
        }
        EASE_IN => {
            if sequence(world, this).end_time_50 <= now {
                sequence_mut(world, this).state_44 = SequenceState::Animating as u32;
            } else {
                clamp_to_start(world, this, &mut now);
                let state = sequence(world, this);
                ease = (now - state.start_time_4c) / (state.end_time_50 - state.start_time_4c);
            }
        }
        EASE_OUT => {
            if sequence(world, this).end_time_50 <= now {
                deactivate_sequence(world, this, 0.0, false);
                return Status::Updated;
            }
            clamp_to_start(world, this, &mut now);
            let state = sequence(world, this);
            ease = (state.end_time_50 - now) / (state.end_time_50 - state.start_time_4c);
        }
        MORPH_SOURCE => {
            let morph_time = now + sequence(world, this).offset_48;
            let Some(source) = sequence(world, this).partner_58 else {
                return Status::UnsupportedMissingPartner;
            };
            let Some(frame) = find_corresponding_morph_frame(world, source, this, morph_time)
            else {
                return Status::UnsupportedMissingMorphKey;
            };
            {
                let mut partner = sequence_mut(world, source);
                let partner_time = frame / partner.frequency_28;
                partner.offset_48 = partner_time - time;
            }
            let was_inactive = sequence(world, this).state_44 == 0;
            sequence_mut(world, this).state_44 = SequenceState::TransSource as u32;
            if was_inactive {
                add_unique_active(world, this);
            }
            now = time;
            trans_source_step(world, this, &mut now, &mut transition_spinner, &mut apply);
        }
        TRANS_SOURCE => {
            trans_source_step(world, this, &mut now, &mut transition_spinner, &mut apply);
        }
        TRANS_DEST => {
            if sequence(world, this).end_time_50 <= now {
                let mut state = sequence_mut(world, this);
                if state.dest_frame_54 != NEGATIVE_INFINITY {
                    state.offset_48 = state.dest_frame_54 - now;
                    state.dest_frame_54 = NEGATIVE_INFINITY;
                }
                state.state_44 = SequenceState::Animating as u32;
            } else {
                clamp_to_start(world, this, &mut now);
                let state = sequence(world, this);
                transition_spinner =
                    (now - state.start_time_4c) / (state.end_time_50 - state.start_time_4c);
            }
        }
        _ => {}
    }
    if !apply || !update_interpolators {
        return Status::Updated;
    }
    let mut update_time = sequence(world, this).dest_frame_54;
    if update_time == NEGATIVE_INFINITY {
        if let Some(source) = sequence(world, this).partner_58 {
            let partner = sequence(world, source);
            if partner.offset_48 + now != partner.last_time_34 {
                update_sequence(world, source, now, false);
            }
            let partner_time = now + sequence(world, source).offset_48;
            let Some(frame) = find_corresponding_morph_frame(world, this, source, partner_time)
            else {
                return Status::UnsupportedMissingMorphKey;
            };
            update_time = frame / sequence(world, this).frequency_28;
        } else {
            update_time = now + sequence(world, this).offset_48;
        }
    }
    let scaled = sequence_mut(world, this).compute_scaled_time(update_time, true);
    let weight = sequence(world, this).weight_1c;
    set_interps_weight_and_time(world, this, weight * transition_spinner, ease, scaled);
    Status::Updated
}

// IDA: 006C4200 NiControllerManager vtable +0x54 Update: only with NiTimeController flag 0x08 (u16 +0x08), for each active sequence (+4C,
// count +54): 006CA950(seq, time, true), then a sequence whose +0x44 state is 0 is swap-removed with the last active entry (count
// decremented, i revisited).
// Completion: 100% of the loop shape; an expired or unsupported sequence stops the host update.
pub fn update_controller_manager(
    world: &mut World,
    manager: Entity,
    time: f32,
) -> NodeControllerUpdateStatus {
    use NodeControllerUpdateStatus as Status;
    let flags = world
        .get::<NodeController>(manager)
        .expect("a NiControllerManager has time state")
        .time
        .fields
        .flags;
    if flags & 0x0008 == 0 {
        return Status::Skipped;
    }
    let mut index = 0;
    loop {
        let active = &world
            .get::<ControllerManager>(manager)
            .expect("a NiControllerManager entity")
            .active_sequences_4c;
        let Some(&this) = active.get(index) else {
            break;
        };
        if world.get::<ControllerSequence>(this).is_none() {
            return Status::UnsupportedSequenceUpdate;
        }
        let status = update_sequence(world, this, time, true);
        if status != SequenceUpdateStatus::Updated && status != SequenceUpdateStatus::Inactive {
            return Status::UnsupportedSequenceUpdate;
        }
        if sequence(world, this).state_44 == 0 {
            let mut state = world
                .get_mut::<ControllerManager>(manager)
                .expect("a NiControllerManager entity");
            let last = *state.active_sequences_4c.last().expect("holds this entry");
            state.active_sequences_4c[index] = last;
            state.active_sequences_4c.pop();
            continue;
        }
        index += 1;
    }
    Status::Applied
}

/// IDA: the controller-manager part of `00475B60` (ActorAnimData's destructor) on its +0x98 manager: each active sequence
/// (+0x4C, count +0x54) through NiControllerSequence_Deactivate(0.0, false); `006C4BD0`: each sequence slot (+0x40, count
/// +0x46) through `006CAC60` (deactivated when its +0x44 state is set, then each interp item's +0x04 controller released and
/// its +0x08 blend link cleared) and its +0x40 manager cleared, the slots emptied (`00739670`) and the +0x58 map emptied
/// (NiTMapBase_RemoveAll); then the manager's target (+0x30, a NiObjectNET) loses it (`006FFE90` NiObjectNET_RemoveController).
/// Completion: 100%; the release of the manager's last reference has no host form.
pub fn release_actor_manager(world: &mut World, manager: Entity) {
    let active = world
        .get::<ControllerManager>(manager)
        .map(|state| state.active_sequences_4c.clone())
        .unwrap_or_default();
    for sequence in active {
        deactivate_sequence(world, sequence, 0.0, false);
    }
    let slots = world
        .get::<ControllerManager>(manager)
        .map(|state| state.sequence_slots_40.clone())
        .unwrap_or_default();
    for sequence in slots.into_iter().flatten() {
        if self::sequence(world, sequence).state_44 != 0 {
            deactivate_sequence(world, sequence, 0.0, false);
        }
        let mut state = sequence_mut(world, sequence);
        for item in &mut state.items_14 {
            item.controller_04 = None;
            item.blend_08 = SequenceBlendRef::None;
        }
        state.manager_40 = None;
    }
    if let Some(mut state) = world.get_mut::<ControllerManager>(manager) {
        state.sequence_slots_40.fill(None);
        state.sequence_size_46 = 0;
        state.sequence_count_48 = 0;
        for bucket in &mut state.sequence_map_58 {
            bucket.clear();
        }
    }
    let target = controller_time_mut(world, manager).and_then(|time| time.target_30);
    if let Some(target) = target {
        remove_controller(world, target, manager);
    }
}

// IDA: 00470B20(sequence, 0, 0, 1.0, 0.0, 0), then +0x48 = -FLT_MAX.
pub(crate) fn activate_from_start(world: &mut World, sequence: Entity) {
    crate::gamebryo::controller_sequence::activate_sequence(
        world, sequence, 0, false, 1.0, 0.0, None, false,
    );
    world
        .get_mut::<crate::gamebryo::controller_sequence::ControllerSequence>(sequence)
        .expect("a NiControllerSequence")
        .offset_48 = f32::MIN;
}

// IDA: 004715C0(0.0): NiControllerSequence_Deactivate(0.0, 0) on each of the manager's +0x4C active sequences, in order.
pub(crate) fn deactivate_all(world: &mut World, manager: Entity) {
    let active = world
        .get::<crate::gamebryo::controller_sequence::ControllerManager>(manager)
        .expect("a NiControllerManager")
        .active_sequences_4c
        .clone();
    for sequence in active {
        crate::gamebryo::controller_sequence::deactivate_sequence(world, sequence, 0.0, false);
    }
}

// IDA: 00471600(manager): +0x6C (cumulative) clear answers null; otherwise the first nonnull of the +0x40 slots below +0x46, its +0x60 accum root.
pub(crate) fn manager_accum_root(world: &World, manager: Entity) -> Option<Entity> {
    let state = world.get::<ControllerManager>(manager)?;
    if !state.cumulative_6c {
        return None;
    }
    let size = usize::from(state.sequence_size_46);
    let first = state
        .sequence_slots_40
        .iter()
        .take(size)
        .copied()
        .flatten()
        .next()?;
    world.get::<ControllerSequence>(first)?.accum_root_60
}

/// IDA: 006C5610 NiControllerManager(target, cumulative): 00715990's time state, the sequence array (grow 10), the 37-bucket map, +0x6C the flag, +0x70..+0x7C zero, `NiTimeController_SetTarget(target)` and a new NiDefaultAVObjectPalette over the target (006C5360: the scene stored, 00716690's rebuild) in +0x7C. Completion: 100%; SetTarget's chain insertion is `set_controller_target`'s.
/// # Errors
/// As `set_controller_target`.
pub fn new_controller_manager(
    world: &mut World,
    target: Entity,
    cumulative: bool,
) -> Result<Entity, GameError> {
    let manager = spawn_node_controller(
        world,
        NiClass::ControllerManager,
        ControllerManager {
            cumulative_6c: cumulative,
            ..ControllerManager::default()
        },
    );
    set_controller_target(world, manager, Some(target))?;
    let mut palette = DefaultAvObjectPalette::default();
    palette.set_scene(Some(target));
    let palette = world.spawn((NiClass::DefaultAvObjectPalette, palette)).id();
    rebuild_from_scene(world, palette);
    world
        .get_mut::<ControllerManager>(manager)
        .expect("just spawned")
        .object_palette_7c = Some(palette);
    Ok(manager)
}

#[cfg(test)]
mod tests {
    use super::super::av_object_palette::{
        apply_av_object_palette_payload, parse_av_object_palette_payload,
    };
    use super::super::bool_animation::BoolInterpolator;
    use super::super::camera::SceneCamera;
    use super::super::scene_derived::DirectionalLight;
    use super::super::scene_node::{
        NiNodeChildren, SceneNodePayload, parse_scene_node_payload, spawn_scene_node,
    };
    use super::super::string_palette::decode_string_palette_payload;
    use super::super::transform_animation::{TransformInterpolator, decode_quat_transform};
    use super::super::transform_controller::{publish_node_controller_head, spawn_node_controller};
    use super::*;

    const OBLIVION_VERSION: u32 = 0x1400_0004;
    const NULL_OFFSET: u16 = 0xFFFF;

    fn u32_le(output: &mut Vec<u8>, value: u32) {
        output.extend(value.to_le_bytes());
    }

    fn f32_le(output: &mut Vec<u8>, value: f32) {
        output.extend(value.to_le_bytes());
    }

    fn owned(output: &mut Vec<u8>, text: &str) {
        u32_le(output, text.len() as u32);
        output.extend(text.as_bytes());
    }

    // Offsets into `standard_strings`' palette.
    const BONE: usize = 0;
    const TRANSFORM_TYPE: usize = 1;
    const VIS_TYPE: usize = 2;
    const GHOST: usize = 3;
    const ARROW: usize = 4;

    fn make_strings(world: &mut World, texts: &[&str]) -> (Entity, Vec<u16>) {
        let mut bytes = Vec::new();
        let mut offsets = Vec::new();
        for text in texts {
            offsets.push(bytes.len() as u16);
            bytes.extend(text.as_bytes());
            bytes.push(0);
        }
        let used = bytes.len() as u32;
        let palette = world
            .spawn((
                NiClass::StringPalette,
                StringPalette::new(bytes, used).unwrap(),
            ))
            .id();
        (palette, offsets)
    }

    fn named_node(world: &mut World, name: &str) -> Entity {
        let node = spawn_scene_node(world, SceneNodePayload::default());
        world.get_mut::<NiObjectNet>(node).unwrap().name = Some(name.as_bytes().to_vec());
        node
    }

    fn spawn_manager(world: &mut World) -> Entity {
        spawn_node_controller(
            world,
            NiClass::ControllerManager,
            ControllerManager::default(),
        )
    }

    fn spawn_sequence(world: &mut World) -> Entity {
        world
            .spawn((NiClass::ControllerSequence, ControllerSequence::default()))
            .id()
    }

    /// Ported from `verify_manager_update` in the C++ scene graph tests: 006C4200 skips without flag 0x08, and an inactive
    /// entry is swap-removed.
    #[test]
    fn manager_update() {
        use super::super::transform_controller::NodeController;
        let mut world = World::new();
        let manager = spawn_manager(&mut world);
        world
            .get_mut::<NodeController>(manager)
            .unwrap()
            .time
            .fields
            .flags = 0x0000;
        assert_eq!(
            update_controller_manager(&mut world, manager, 1.0),
            NodeControllerUpdateStatus::Skipped
        );
        world
            .get_mut::<NodeController>(manager)
            .unwrap()
            .time
            .fields
            .flags = 0x0008;
        assert_eq!(
            update_controller_manager(&mut world, manager, 1.0),
            NodeControllerUpdateStatus::Applied
        );
        let sequence = spawn_sequence(&mut world);
        manager_mut(&mut world, manager)
            .active_sequences_4c
            .push(sequence);
        assert_eq!(
            update_controller_manager(&mut world, manager, 1.0),
            NodeControllerUpdateStatus::Applied
        );
        assert!(
            manager_state(&world, manager)
                .active_sequences_4c
                .is_empty()
        );
    }

    fn spawn_transform_interpolator(world: &mut World) -> Entity {
        world
            .spawn((
                NiClass::TransformInterpolator,
                TransformInterpolator::default(),
            ))
            .id()
    }

    fn base(world: &World, controller: Entity) -> &NodeController {
        world.get::<NodeController>(controller).unwrap()
    }

    fn base_mut(world: &mut World, controller: Entity) -> Mut<'_, NodeController> {
        world.get_mut::<NodeController>(controller).unwrap()
    }

    fn manager_state(world: &World, manager: Entity) -> &ControllerManager {
        world.get::<ControllerManager>(manager).unwrap()
    }

    fn manager_mut(world: &mut World, manager: Entity) -> Mut<'_, ControllerManager> {
        world.get_mut::<ControllerManager>(manager).unwrap()
    }

    fn flags(world: &World, node: Entity) -> u16 {
        world
            .get::<SceneNodePayload>(node)
            .unwrap()
            .effective_av_flags
    }

    fn head(world: &World, node: Entity) -> Option<Entity> {
        world.get::<NiObjectNet>(node).unwrap().controller_0c
    }

    /// IDA: N/A host scene: root NiNode (manager target and controller head) with a "Bone" child, a palette mapping Root and Bone,
    /// and the manager.
    struct Scene {
        root: Entity,
        bone: Entity,
        manager: Entity,
        palette: Entity,
        strings: Entity,
        offsets: Vec<u16>,
    }

    impl Scene {
        fn new(world: &mut World) -> Self {
            let root = named_node(world, "Root");
            let bone = named_node(world, "Bone");
            let manager = spawn_manager(world);
            world
                .get_mut::<NiNodeChildren>(root)
                .unwrap()
                .slots_ac
                .push(Some(bone));
            world.get_mut::<NiAvParent>(bone).unwrap().0 = Some(root);
            base_mut(world, manager).time.target_30 = Some(root);
            world.get_mut::<NiObjectNet>(root).unwrap().controller_0c = Some(manager);
            let mut palette_state = DefaultAvObjectPalette::default();
            palette_state.set_av_object(b"Root", Some(root));
            palette_state.set_av_object(b"Bone", Some(bone));
            let palette = world
                .spawn((NiClass::DefaultAvObjectPalette, palette_state))
                .id();
            manager_mut(world, manager).object_palette_7c = Some(palette);
            let (strings, offsets) = make_strings(
                world,
                &[
                    "Bone",
                    "NiTransformController",
                    "NiVisController",
                    "Ghost",
                    "Arrow01",
                    "Root",
                    "Bip01",
                    "Bip02",
                ],
            );
            Self {
                root,
                bone,
                manager,
                palette,
                strings,
                offsets,
            }
        }

        fn tag(&self, object: usize, controller_type: usize) -> SequenceIdTag {
            SequenceIdTag {
                palette_00: Some(self.strings),
                av_object_name_04: self.offsets[object],
                controller_type_08: self.offsets[controller_type],
                ..SequenceIdTag::default()
            }
        }

        fn sequence(&self, world: &mut World, tags: Vec<SequenceIdTag>) -> Entity {
            let mut items = Vec::with_capacity(tags.len());
            for _ in &tags {
                items.push(SequenceInterpItem {
                    interpolator_00: Some(spawn_transform_interpolator(world)),
                    ..SequenceInterpItem::default()
                });
            }
            let this = spawn_sequence(world);
            let mut state = sequence_mut(world, this);
            state.name_08 = Some(b"Walk".to_vec());
            state.array_size_0c = tags.len() as u32;
            state.items_14 = items;
            state.id_tags_18 = tags;
            state.manager_40 = Some(self.manager);
            this
        }
    }

    fn multi_target_after_manager(world: &World, manager: Entity) -> Option<Entity> {
        base(world, manager)
            .time
            .next_34
            .filter(|&next| class_of(world, next) == Some(NiClass::MultiTargetTransformController))
    }

    fn slot_of(world: &World, this: Entity, index: usize) -> (Entity, usize) {
        match sequence(world, this).items_14[index].blend_08 {
            SequenceBlendRef::MultiTargetSlot { controller, index } => (controller, index),
            other => panic!("not a multi-target slot: {other:?}"),
        }
    }

    fn build_sequence_payload(priority: bool) -> Vec<u8> {
        let mut bytes = Vec::new();
        owned(&mut bytes, "Idle");
        u32_le(&mut bytes, 1);
        u32_le(&mut bytes, 0);
        u32_le(&mut bytes, 5);
        u32_le(&mut bytes, 6);
        if priority {
            bytes.push(0x7F);
        }
        u32_le(&mut bytes, 7);
        for offset in [0, 6, 0x1_0003, 0xFFFF_FFFF, 0xFFFF] {
            u32_le(&mut bytes, offset);
        }
        f32_le(&mut bytes, 0.5);
        u32_le(&mut bytes, 8);
        u32_le(&mut bytes, 2);
        f32_le(&mut bytes, 1.5);
        f32_le(&mut bytes, 0.25);
        f32_le(&mut bytes, 4.0);
        u32_le(&mut bytes, 9);
        owned(&mut bytes, "Root");
        u32_le(&mut bytes, 10);
        bytes
    }

    /// IDA: 006C8E40 modern LoadBinary and 006C6550 cold state; the priority byte follows NiStream +0x04 and the IDTag u32 offsets
    /// truncate to u16.
    #[test]
    fn sequence_payload() {
        let cold = ControllerSequence::default();
        assert_eq!(cold.weight_1c, 1.0);
        assert_eq!(cold.frequency_28, 1.0);
        assert_eq!(cold.begin_key_time_2c, f32::MAX);
        assert_eq!(cold.end_key_time_30, -f32::MAX);
        assert_eq!(cold.dest_frame_54, -f32::MAX);
        assert!(cold.manager_40.is_none());
        assert_eq!(cold.state_44, 0);
        assert!(cold.name_08.is_none());
        assert!(cold.items_14.is_empty());

        let with_priority = build_sequence_payload(true);
        let payload =
            parse_controller_sequence_payload(&with_priority, OBLIVION_VERSION, 11).unwrap();
        assert_eq!(payload.bytes_consumed, with_priority.len());
        assert_eq!(payload.name.as_deref(), Some(&b"Idle"[..]));
        assert_eq!(payload.array_size, 1);
        assert_eq!(payload.items[0].interpolator_index, 5);
        assert_eq!(payload.items[0].controller_index, 6);
        assert_eq!(payload.items[0].priority, Some(0x7F));
        assert_eq!(payload.id_tags[0].palette_index, 7);
        assert_eq!(payload.text_keys_index, 8);
        assert_eq!(payload.cycle_type, 2);
        assert_eq!(payload.manager_index, 9);
        assert_eq!(payload.accum_root_name.as_deref(), Some(&b"Root"[..]));
        assert_eq!(payload.string_palette_index, 10);

        let without_priority = build_sequence_payload(false);
        let no_priority =
            parse_controller_sequence_payload(&without_priority, OBLIVION_VERSION, 0).unwrap();
        assert_eq!(no_priority.bytes_consumed, without_priority.len());
        assert_eq!(no_priority.items[0].priority, None);

        let mut state = ControllerSequence::default();
        state.apply_loaded_payload(&payload);
        assert_eq!(state.name_08.as_deref(), Some(&b"Idle"[..]));
        assert_eq!(state.items_14.len(), 1);
        assert_eq!(state.items_14[0].priority_0d, 0x7F);
        assert_eq!(state.items_14[0].byte_0c, 0xFF);
        assert_eq!(state.id_tags_18[0].av_object_name_04, 0);
        assert_eq!(state.id_tags_18[0].property_type_06, 6);
        assert_eq!(state.id_tags_18[0].controller_type_08, 3, "u16 truncation");
        assert_eq!(state.id_tags_18[0].controller_id_0a, 0xFFFF);
        assert_eq!(state.id_tags_18[0].interpolator_id_0c, 0xFFFF);
        assert_eq!(state.weight_1c, 0.5);
        assert_eq!(state.cycle_type_24, 2);
        assert_eq!(state.frequency_28, 1.5);
        assert_eq!(state.begin_key_time_2c, 0.25);
        assert_eq!(state.end_key_time_30, 4.0);
        assert_eq!(state.accum_root_name_5c.as_deref(), Some(&b"Root"[..]));

        assert!(
            parse_controller_sequence_payload(&with_priority, 0x0A01_0070, 11).is_err(),
            "legacy"
        );
        assert!(
            parse_controller_sequence_payload(
                &with_priority[..with_priority.len() - 2],
                OBLIVION_VERSION,
                11
            )
            .is_err(),
            "truncated"
        );
    }

    /// IDA: 006C4E50 LoadBinary and 006C5520 cold state: field order and the u8 -> bool cumulative store.
    #[test]
    fn manager_payload() {
        let cold = ControllerManager::default();
        assert!(cold.sequence_slots_40.is_empty());
        assert_eq!(cold.sequence_size_46, 0);
        assert_eq!(cold.sequence_count_48, 0);
        assert_eq!(cold.sequence_grow_by_4a, 10);
        assert!(!cold.cumulative_6c);
        assert!(cold.object_palette_7c.is_none());
        assert!(cold.active_sequences_4c.is_empty());

        let mut bytes = Vec::new();
        u32_le(&mut bytes, NULL_REFERENCE);
        bytes.extend(0x0008u16.to_le_bytes());
        f32_le(&mut bytes, 1.0);
        f32_le(&mut bytes, 0.0);
        f32_le(&mut bytes, 0.0);
        f32_le(&mut bytes, 1.0);
        u32_le(&mut bytes, 0);
        bytes.push(2);
        u32_le(&mut bytes, 2);
        u32_le(&mut bytes, 2);
        u32_le(&mut bytes, 3);
        u32_le(&mut bytes, 4);
        let payload = parse_controller_manager_payload(&bytes, OBLIVION_VERSION).unwrap();
        assert_eq!(payload.bytes_consumed, bytes.len());
        assert_eq!(payload.time.flags, 0x0008);
        assert_eq!(payload.time.target_index, 0);
        assert_eq!(payload.cumulative_raw, 2);
        assert_eq!(payload.sequence_indices, [2, 3]);
        assert_eq!(payload.palette_index, 4);

        let mut world = World::new();
        let manager = spawn_manager(&mut world);
        let table = StreamObjectTable::default();
        let mut no_palette = payload.clone();
        no_palette.palette_index = NULL_REFERENCE;
        assert_eq!(
            load_controller_manager(&mut world, manager, &no_palette, &table),
            Ok(true)
        );
        assert!(manager_state(&world, manager).cumulative_6c);
        assert_eq!(base(&world, manager).time.fields.flags, 0x0008);
        assert_eq!(base(&world, manager).time.pending_target_index, 0);
    }

    /// IDA: 006C4510 SetSize, 006C5240 AddFirstEmpty (lowest index of the topmost null run) and 006C4940 SetAt count bookkeeping.
    #[test]
    fn sequence_array() {
        let mut world = World::new();
        let sequences: Vec<Entity> = (0..7).map(|_| world.spawn_empty().id()).collect();
        let mut manager = ControllerManager::default();
        manager.set_sequence_capacity(4);
        assert_eq!(manager.sequence_slots_40.len(), 4);
        assert_eq!(manager.add_first_empty(None), Ok(0xFFFF_FFFF));
        for (index, &entry) in sequences.iter().take(4).enumerate() {
            assert_eq!(manager.add_first_empty(Some(entry)), Ok(index as u32));
        }
        manager.set_sequence_at(1, None).unwrap();
        manager.set_sequence_at(3, None).unwrap();
        assert_eq!(manager.sequence_size_46, 4);
        assert_eq!(manager.sequence_count_48, 2);
        assert_eq!(
            manager.add_first_empty(Some(sequences[4])),
            Ok(3),
            "the topmost null run"
        );
        assert_eq!(manager.sequence_count_48, 3);
        assert_eq!(
            manager.add_first_empty(Some(sequences[5])),
            Ok(1),
            "the lower null slot"
        );
        assert_eq!(manager.sequence_count_48, 4);
        assert_eq!(manager.add_first_empty(Some(sequences[6])), Ok(4));
        assert_eq!(manager.sequence_slots_40.len(), 14, "the +4A grow-by");
        assert_eq!(manager.sequence_size_46, 5);
        assert_eq!(manager.sequence_count_48, 5);
        manager.set_sequence_capacity(2);
        assert_eq!(manager.sequence_slots_40.len(), 2);
        assert_eq!(manager.sequence_size_46, 2);
        assert_eq!(manager.sequence_count_48, 2, "shrinking releases the tail");
        assert_eq!(manager.sequence_slots_40[1], Some(sequences[5]));
    }

    /// IDA: 006C5760 AddSequence: refusal when +40 is set, +40 store, accum root name from the target name, SetName, AddFirstEmpty,
    /// then the case-insensitive +58 map insert or assign.
    #[test]
    fn add_sequence_order() {
        use ControllerManagerAddStatus as Status;
        let mut world = World::new();
        let manager = spawn_manager(&mut world);
        let target = named_node(&mut world, "Root");
        base_mut(&mut world, manager).time.target_30 = Some(target);
        manager_mut(&mut world, manager).set_sequence_capacity(1);
        let walk = spawn_sequence(&mut world);
        sequence_mut(&mut world, walk).name_08 = Some(b"Block".to_vec());
        assert_eq!(
            add_sequence(&mut world, manager, None, None, false),
            Ok(Status::UnsupportedNullSequence)
        );
        assert_eq!(
            add_sequence(&mut world, manager, Some(walk), None, false),
            Ok(Status::Added)
        );
        assert_eq!(sequence(&world, walk).manager_40, Some(manager));
        assert_eq!(
            sequence(&world, walk).accum_root_name_5c.as_deref(),
            Some(&b"Root"[..])
        );
        assert_eq!(
            manager_state(&world, manager).sequence_slots_40[0],
            Some(walk)
        );
        assert_eq!(
            manager_state(&world, manager).find_sequence(b"bloCK"),
            Some(walk),
            "a same-bucket case-insensitive lookup"
        );
        assert_eq!(manager_state(&world, manager).find_sequence(b"block"), None);
        assert_eq!(
            add_sequence(&mut world, manager, Some(walk), None, false),
            Ok(Status::AlreadyManaged)
        );

        let renamed = spawn_sequence(&mut world);
        {
            let mut state = sequence_mut(&mut world, renamed);
            state.name_08 = Some(b"Old".to_vec());
            state.accum_root_name_5c = Some(b"Pelvis".to_vec());
        }
        // "bloCK" shares "Block"'s bucket (1), so the map entry is reassigned.
        assert_eq!(
            add_sequence(&mut world, manager, Some(renamed), Some(b"bloCK"), false),
            Ok(Status::Added)
        );
        assert_eq!(
            sequence(&world, renamed).name_08.as_deref(),
            Some(&b"bloCK"[..])
        );
        assert_eq!(
            sequence(&world, renamed).accum_root_name_5c.as_deref(),
            Some(&b"Pelvis"[..])
        );
        let state = manager_state(&world, manager);
        assert_eq!(state.sequence_size_46, 2);
        assert_eq!(state.find_sequence(b"Block"), Some(renamed));
        assert_eq!(state.sequence_map_58[1].len(), 1);
        assert_eq!(state.sequence_map_58[1][0].key, b"Block");

        let unnamed = spawn_sequence(&mut world);
        assert_eq!(
            add_sequence(&mut world, manager, Some(unnamed), None, false),
            Ok(Status::UnsupportedNullName)
        );
        assert_eq!(
            manager_state(&world, manager).sequence_size_46,
            3,
            "the stop comes after AddFirstEmpty"
        );

        let orphan_manager = spawn_manager(&mut world);
        let orphan = spawn_sequence(&mut world);
        assert_eq!(
            add_sequence(&mut world, orphan_manager, Some(orphan), None, false),
            Ok(Status::UnsupportedMissingTarget)
        );
    }

    /// IDA: 006C9590 -> 006C8220: a new NiMultiTargetTransformController (count + 10) is targeted at the root, spliced after the
    /// manager, the object's own NiTransformController is removed (name is not Bip01), the item points at blend[slot] with flag 1, and
    /// the root receives 0070AA20 flags.
    #[test]
    fn store_targets_conversion() {
        let mut world = World::new();
        let scene = Scene::new(&mut world);
        let old_controller = spawn_node_controller(
            &mut world,
            NiClass::TransformController,
            TransformController::default(),
        );
        base_mut(&mut world, old_controller).time.target_30 = Some(scene.bone);
        world
            .get_mut::<NiObjectNet>(scene.bone)
            .unwrap()
            .controller_0c = Some(old_controller);
        let tag = scene.tag(BONE, TRANSFORM_TYPE);
        let this = scene.sequence(&mut world, vec![tag]);
        let result = store_targets(&mut world, this, Some(scene.root));
        assert_eq!(result.status, StoreTargetsStatus::Stored);
        assert!(result.messages.is_empty());

        let multi = multi_target_after_manager(&world, scene.manager).expect("spliced after");
        let state = world.get::<MultiTargetTransformController>(multi).unwrap();
        assert_eq!(state.interpolator_count_44, 11);
        assert_eq!(base(&world, multi).time.target_30, Some(scene.root));
        assert_ne!(base(&world, multi).time.fields.flags & 0x0020, 0);
        assert_eq!(head(&world, scene.root), Some(scene.manager));
        assert_eq!(base(&world, multi).time.next_34, None);
        assert_eq!(state.targets_40[0], Some(scene.bone));
        assert_ne!(state.blend_interpolators_3c[0].base.flags_0c & 1, 0);
        assert_eq!(slot_of(&world, this, 0), (multi, 0));
        assert_eq!(
            sequence(&world, this).items_14[0].controller_04,
            Some(multi)
        );
        assert_eq!(
            head(&world, scene.bone),
            None,
            "the old controller is removed"
        );
        assert_eq!(base(&world, old_controller).time.next_34, None);
        assert_eq!(flags(&world, scene.root), 0x0016);
        assert_eq!(flags(&world, scene.bone), 0x0016);
    }

    /// IDA: 006C8220 reuses an existing multi-target controller (no flag pass), and the 006C9590 generic path binds an NiVisController
    /// found in the chain, replacing its interpolator by a manager-controlled NiBlendBoolInterpolator.
    #[test]
    fn store_targets_existing_and_generic() {
        let mut world = World::new();
        let scene = Scene::new(&mut world);
        let mut multi_state = MultiTargetTransformController::default();
        multi_state.set_interpolator_count(2);
        let existing = spawn_node_controller(
            &mut world,
            NiClass::MultiTargetTransformController,
            multi_state,
        );
        base_mut(&mut world, existing).time.target_30 = Some(scene.root);
        base_mut(&mut world, scene.manager).time.next_34 = Some(existing);
        let source = world
            .spawn((NiClass::BoolInterpolator, BoolInterpolator::default()))
            .id();
        let vis = spawn_node_controller(
            &mut world,
            NiClass::VisController,
            VisController {
                interpolator_3c: Some(source),
            },
        );
        base_mut(&mut world, vis).time.target_30 = Some(scene.bone);
        world
            .get_mut::<NiObjectNet>(scene.bone)
            .unwrap()
            .controller_0c = Some(vis);
        let tags = vec![scene.tag(BONE, TRANSFORM_TYPE), scene.tag(BONE, VIS_TYPE)];
        let this = scene.sequence(&mut world, tags);
        let result = store_targets(&mut world, this, Some(scene.root));
        assert_eq!(result.status, StoreTargetsStatus::Stored);
        assert!(result.messages.is_empty());
        assert_eq!(
            multi_target_after_manager(&world, scene.manager),
            Some(existing)
        );
        assert_eq!(
            world
                .get::<MultiTargetTransformController>(existing)
                .unwrap()
                .interpolator_count_44,
            2
        );
        assert_eq!(slot_of(&world, this, 0).0, existing);
        let blend = world
            .get::<VisController>(vis)
            .unwrap()
            .interpolator_3c
            .unwrap();
        let state = world.get::<ParticleBlendBoolInterpolator>(blend).unwrap();
        assert_eq!(state.base.flags_0c, 1);
        assert_eq!(state.base.capacity_0d, 2);
        assert_eq!(state.base.items_14.len(), 2);
        assert_eq!(state.value_30, 2);
        assert_ne!(base(&world, vis).time.fields.flags & 0x0020, 0);
        let item = sequence(&world, this).items_14[1];
        assert_eq!(item.blend_08, SequenceBlendRef::Bool(blend));
        assert_eq!(item.controller_04, Some(vis));
        assert_eq!(
            flags(&world, scene.root),
            0x000E,
            "no new target binding, so no SetSelectiveUpdateFlags"
        );
    }

    /// IDA: 006C9590 on a NiParticleSystem's chain: 006C9852 matches the +0x8C modifier name, 0074F870 picks BirthRate or EmitterActive, 0074FCF0's
    /// held blend is kept, 0074F910, 0075F320 and 0075F570 make the missing ones, and 0075E5C0 retargets the item's own controller.
    #[test]
    fn store_targets_on_particle_controllers() {
        let mut world = World::new();
        let scene = Scene::new(&mut world);
        let system = world
            .spawn((
                NiClass::ParticleSystem,
                NiObjectNet {
                    name: Some(b"Sys".to_vec()),
                    ..NiObjectNet::default()
                },
            ))
            .id();
        let palette = scene.palette;
        world
            .get_mut::<DefaultAvObjectPalette>(palette)
            .unwrap()
            .set_av_object(b"Sys", Some(system));
        let held_rate = world
            .spawn((
                NiClass::BlendFloatInterpolator,
                ParticleBlendFloatInterpolator::default(),
            ))
            .id();
        let plain_bool = world
            .spawn((NiClass::BoolInterpolator, BoolInterpolator::default()))
            .id();
        let plain_float = world.spawn(NiClass::FloatInterpolator).id();
        let mut active = ParticleModifierActiveController::default();
        active.base.modifier_name_40 = Some(b"Mod".to_vec());
        active.interpolator_3c = Some(plain_bool);
        let active = world
            .spawn((NiClass::ParticleModifierActiveController, active))
            .id();
        let mut gravity = ParticleGravityStrengthController::default();
        gravity.base.modifier_name_40 = Some(b"Mod".to_vec());
        gravity.base.time.next_34 = Some(active);
        gravity.interpolator_3c = Some(plain_float);
        let gravity = world
            .spawn((NiClass::ParticleGravityStrengthController, gravity))
            .id();
        let mut emitter = ParticleEmitterController {
            modifier_name_40: Some(b"Mod".to_vec()),
            birth_rate_interpolator_3c: Some(held_rate),
            active_interpolator_48: Some(plain_bool),
            ..ParticleEmitterController::default()
        };
        emitter.time.next_34 = Some(gravity);
        let emitter = world
            .spawn((NiClass::ParticleEmitterController, emitter))
            .id();
        world.get_mut::<NiObjectNet>(system).unwrap().controller_0c = Some(emitter);
        let mut retargeted = ParticleGravityStrengthController::default();
        retargeted.base.modifier_name_40 = Some(b"Other".to_vec());
        let retargeted = world
            .spawn((NiClass::ParticleGravityStrengthController, retargeted))
            .id();
        let (strings, offsets) = make_strings(
            &mut world,
            &[
                "Sys",
                "NiPSysEmitterCtlr",
                "NiPSysModifierActiveCtlr",
                "NiPSysGravityStrengthCtlr",
                "Mod",
                "Other",
                "BirthRate",
                "EMITTERACTIVE",
                "Nope",
            ],
        );
        let tag = |controller_type: usize, id: usize, interpolator: Option<usize>| SequenceIdTag {
            palette_00: Some(strings),
            av_object_name_04: offsets[0],
            controller_type_08: offsets[controller_type],
            controller_id_0a: offsets[id],
            interpolator_id_0c: interpolator.map_or(0xFFFF, |at| offsets[at]),
            ..SequenceIdTag::default()
        };
        let tags = vec![
            tag(1, 4, Some(6)),
            tag(1, 4, Some(7)),
            tag(2, 4, None),
            tag(3, 4, None),
            tag(1, 4, Some(8)),
            tag(3, 5, None),
        ];
        let this = scene.sequence(&mut world, tags);
        sequence_mut(&mut world, this).items_14[5].controller_04 = Some(retargeted);
        let result = store_targets(&mut world, this, Some(scene.root));
        assert_eq!(result.status, StoreTargetsStatus::Stored);
        assert_eq!(
            result.messages,
            vec![(StoreTargetsMessage::MissingInterpolator, 4)]
        );
        let items = sequence(&world, this).items_14.clone();
        assert_eq!(items[0].blend_08, SequenceBlendRef::Float(held_rate));
        assert_eq!(items[0].controller_04, Some(emitter));
        let state = world.get::<ParticleEmitterController>(emitter).unwrap();
        assert_eq!(state.birth_rate_interpolator_3c, Some(held_rate));
        let created_active = state.active_interpolator_48.unwrap();
        assert_ne!(created_active, plain_bool);
        assert_ne!(state.time.fields.flags & 0x0020, 0);
        assert_eq!(items[1].blend_08, SequenceBlendRef::Bool(created_active));
        let state = world
            .get::<ParticleModifierActiveController>(active)
            .unwrap();
        assert_eq!(
            items[2].blend_08,
            SequenceBlendRef::Bool(state.interpolator_3c.unwrap())
        );
        assert_ne!(state.interpolator_3c, Some(plain_bool));
        let state = world
            .get::<ParticleGravityStrengthController>(gravity)
            .unwrap();
        let created_float = state.interpolator_3c.unwrap();
        assert_eq!(items[3].blend_08, SequenceBlendRef::Float(created_float));
        let blend = world
            .get::<ParticleBlendFloatInterpolator>(created_float)
            .unwrap();
        assert_eq!(blend.base.flags_0c, 1);
        assert_eq!(blend.base.capacity_0d, 2);
        assert_eq!(blend.value_30, -f32::MAX);
        assert_eq!(items[4].blend_08, SequenceBlendRef::None);
        // No chain entry has "Other", so the item's own controller goes to the head.
        assert_eq!(
            world.get::<NiObjectNet>(system).unwrap().controller_0c,
            Some(retargeted)
        );
        let state = world
            .get::<ParticleGravityStrengthController>(retargeted)
            .unwrap();
        assert_eq!(state.base.time.target_30, Some(system));
        assert_eq!(state.base.time.next_34, Some(emitter));
        assert!(matches!(items[5].blend_08, SequenceBlendRef::Float(_)));
    }

    /// IDA: 006C9590 fallback (item controller SetTarget + chain prepend), the silent prefixes, and the 748530 messages for a missing
    /// object or controller.
    #[test]
    fn store_targets_fallback_and_messages() {
        let mut world = World::new();
        let scene = Scene::new(&mut world);
        let source = world
            .spawn((NiClass::BoolInterpolator, BoolInterpolator::default()))
            .id();
        let fallback = spawn_node_controller(
            &mut world,
            NiClass::VisController,
            VisController {
                interpolator_3c: Some(source),
            },
        );
        let tags = vec![
            scene.tag(GHOST, VIS_TYPE),
            scene.tag(ARROW, VIS_TYPE),
            scene.tag(BONE, VIS_TYPE),
            scene.tag(BONE, GHOST),
        ];
        let this = scene.sequence(&mut world, tags);
        sequence_mut(&mut world, this).items_14[2].controller_04 = Some(fallback);
        let result = store_targets(&mut world, this, Some(scene.root));
        assert_eq!(result.status, StoreTargetsStatus::Stored);
        assert_eq!(
            result.messages,
            [
                (StoreTargetsMessage::MissingTargetObject, 0),
                (StoreTargetsMessage::MissingTargetController, 3)
            ]
        );
        assert_eq!(base(&world, fallback).time.target_30, Some(scene.bone));
        assert_eq!(head(&world, scene.bone), Some(fallback));
        assert!(matches!(
            sequence(&world, this).items_14[2].blend_08,
            SequenceBlendRef::Bool(_)
        ));
        assert_eq!(flags(&world, scene.root), 0x0016);
        assert_eq!(flags(&world, scene.bone), 0x0016);
    }

    /// IDA: 006C98F7's splice when the item's controller already sits right after the manager: +0x34 is read after 006FFE90's remove,
    /// so the controller does not end up as its own next.
    #[test]
    fn store_targets_splice_reads_next_after_the_remove() {
        let mut world = World::new();
        let scene = Scene::new(&mut world);
        let source = world
            .spawn((NiClass::BoolInterpolator, BoolInterpolator::default()))
            .id();
        let vis = spawn_node_controller(
            &mut world,
            NiClass::VisController,
            VisController {
                interpolator_3c: Some(source),
            },
        );
        base_mut(&mut world, vis).time.target_30 = Some(scene.root);
        base_mut(&mut world, scene.manager).time.next_34 = Some(vis);
        // A nonnull CtlrID makes the chain scan miss, which reaches the item's own controller.
        let tag = SequenceIdTag {
            controller_id_0a: scene.offsets[GHOST],
            ..scene.tag(5, VIS_TYPE)
        };
        let this = scene.sequence(&mut world, vec![tag]);
        sequence_mut(&mut world, this).items_14[0].controller_04 = Some(vis);
        assert_eq!(
            store_targets(&mut world, this, Some(scene.root)).status,
            StoreTargetsStatus::Stored
        );
        assert_eq!(base(&world, vis).time.next_34, None, "not its own next");
        assert_eq!(base(&world, scene.manager).time.next_34, Some(vis));
        assert_eq!(head(&world, scene.root), Some(scene.manager));
    }

    /// IDA: 0070AA20's dispatch to NiParticleSystem 00749360 and the leaf 00707D80 on NiDirectionalLight and NiCamera.
    #[test]
    fn selective_flags_reach_particle_and_leaf_children() {
        let mut world = World::new();
        let root = named_node(&mut world, "Root");
        let system = super::super::particle_system::spawn_particle_system(&mut world);
        world
            .get_mut::<ParticleSystem>(system)
            .unwrap()
            .effective_av_flags_18 = 0x0010;
        let light = super::super::scene_derived::spawn_directional_light(&mut world);
        let camera = crate::gamebryo::camera::construct_scene_camera(&mut world);
        world.get_mut::<NiNodeChildren>(root).unwrap().slots_ac =
            vec![Some(system), Some(light), None, Some(camera)];
        let (mut selective, mut rigid) = (false, true);
        assert!(
            set_selective_update_flags(&mut world, root, &mut selective, false, &mut rigid).is_ok()
        );
        assert!(selective, "the particle child forces it");
        assert!(!rigid, "the particle child clears it");
        assert_eq!(
            world
                .get::<ParticleSystem>(system)
                .unwrap()
                .effective_av_flags_18,
            0x000E,
            "0x02 | 0x0C set, 0x10 cleared"
        );
        assert_eq!(
            world
                .get::<DirectionalLight>(light)
                .unwrap()
                .payload
                .av
                .effective_av_flags,
            0x0010,
            "00707D80 from 0x000E: 0x02 and 0x04 clear, 0x08 clears, 0x10 sets"
        );
        assert_eq!(
            world.get::<SceneCamera>(camera).unwrap().av_flags_18,
            0x0010
        );
    }

    /// IDA: 006C9B54 Bip02 path: Bip01's +0x1C parent multi-target controller re-points matching items (006C6910) without changing their
    /// controller.
    #[test]
    fn store_targets_bip02_rebind() {
        let mut world = World::new();
        let scene = Scene::new(&mut world);
        let parent = named_node(&mut world, "Creature");
        let bip01 = named_node(&mut world, "Bip01");
        world.get_mut::<NiAvParent>(bip01).unwrap().0 = Some(parent);
        let bip02 = named_node(&mut world, "Bip02");
        {
            let mut palette = world
                .get_mut::<DefaultAvObjectPalette>(scene.palette)
                .unwrap();
            palette.set_av_object(b"Bip01", Some(bip01));
            palette.set_av_object(b"Bip02", Some(bip02));
        }
        let mut parent_state = MultiTargetTransformController::default();
        parent_state.set_interpolator_count(1);
        parent_state.bind_target(0, Some(scene.bone)).unwrap();
        let parent_multi = spawn_node_controller(
            &mut world,
            NiClass::MultiTargetTransformController,
            parent_state,
        );
        world.get_mut::<NiObjectNet>(parent).unwrap().controller_0c = Some(parent_multi);
        let tag = scene.tag(BONE, TRANSFORM_TYPE);
        let this = scene.sequence(&mut world, vec![tag]);
        assert_eq!(
            store_targets(&mut world, this, Some(scene.root)).status,
            StoreTargetsStatus::Stored
        );
        let root_multi = multi_target_after_manager(&world, scene.manager).unwrap();
        assert_eq!(slot_of(&world, this, 0), (parent_multi, 0));
        assert_eq!(
            sequence(&world, this).items_14[0].controller_04,
            Some(root_multi),
            "only the blend slot is repointed"
        );
    }

    /// IDA: 006C9590 faults on a null manager (+40), palette (+7C) or AV name; the host reports them.
    #[test]
    fn store_targets_unsupported() {
        let mut world = World::new();
        let scene = Scene::new(&mut world);
        let tag = scene.tag(BONE, VIS_TYPE);
        let orphan = scene.sequence(&mut world, vec![tag]);
        sequence_mut(&mut world, orphan).manager_40 = None;
        assert_eq!(
            store_targets(&mut world, orphan, Some(scene.root)).status,
            StoreTargetsStatus::UnsupportedMissingManager
        );
        let this = scene.sequence(&mut world, vec![tag]);
        sequence_mut(&mut world, this).id_tags_18[0].av_object_name_04 = NULL_OFFSET;
        assert_eq!(
            store_targets(&mut world, this, Some(scene.root)).status,
            StoreTargetsStatus::UnsupportedPaletteString
        );
        manager_mut(&mut world, scene.manager).object_palette_7c = None;
        let last = scene.sequence(&mut world, vec![tag]);
        assert_eq!(
            store_targets(&mut world, last, Some(scene.root)).status,
            StoreTargetsStatus::UnsupportedMissingPalette
        );
    }

    /// IDA: 006C4260 PostLoad: StoreTargets(+30) for each nonnull slot below +46; an unsupported StoreTargets reports false.
    #[test]
    fn manager_post_load() {
        let mut world = World::new();
        let scene = Scene::new(&mut world);
        let tag = scene.tag(BONE, TRANSFORM_TYPE);
        let this = scene.sequence(&mut world, vec![tag]);
        sequence_mut(&mut world, this).manager_40 = None;
        manager_mut(&mut world, scene.manager).set_sequence_capacity(2);
        assert_eq!(
            add_sequence(&mut world, scene.manager, Some(this), None, false),
            Ok(ControllerManagerAddStatus::Added)
        );
        assert!(controller_manager_post_load(&mut world, scene.manager));
        assert!(multi_target_after_manager(&world, scene.manager).is_some());
        manager_mut(&mut world, scene.manager).object_palette_7c = None;
        assert!(!controller_manager_post_load(&mut world, scene.manager));
    }

    /// The object bodies of `verify_stream_graph`: NiNode -> NiControllerManager -> NiControllerSequence with NiDefaultAVObjectPalette,
    /// NiStringPalette and NiTransformInterpolator: immediate and deferred references, AddSequence at link, and StoreTargets from the
    /// 006C4260 PostLoad. The class functions are driven by hand, in 00714390's order, because the stream graph is another family's.
    #[test]
    fn hand_driven_graph() {
        let mut world = World::new();
        let root = spawn_scene_node(&mut world, SceneNodePayload::default());
        let manager = spawn_manager(&mut world);
        let this = spawn_sequence(&mut world);
        let palette = world
            .spawn((
                NiClass::DefaultAvObjectPalette,
                DefaultAvObjectPalette::default(),
            ))
            .id();
        let strings_owner = world.spawn(NiClass::StringPalette).id();
        let interpolator = spawn_transform_interpolator(&mut world);
        let mut table = StreamObjectTable::default();
        for (name, entity) in [
            ("NiNode", root),
            ("NiControllerManager", manager),
            ("NiControllerSequence", this),
            ("NiDefaultAVObjectPalette", palette),
            ("NiStringPalette", strings_owner),
            ("NiTransformInterpolator", interpolator),
        ] {
            table.push(name, 0, entity);
        }

        // 0 NiNode "Root", controller head = manager (1).
        let mut bytes = Vec::new();
        owned(&mut bytes, "Root");
        u32_le(&mut bytes, 0);
        u32_le(&mut bytes, 1);
        bytes.extend(0x000Eu16.to_le_bytes());
        for value in [
            0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0,
        ] {
            f32_le(&mut bytes, value);
        }
        u32_le(&mut bytes, 0);
        u32_le(&mut bytes, NULL_REFERENCE);
        u32_le(&mut bytes, 0);
        u32_le(&mut bytes, 0);
        let node_payload = parse_scene_node_payload(&bytes, OBLIVION_VERSION).unwrap();
        assert_eq!(node_payload.bytes_consumed, bytes.len());
        let name = node_payload.net.name.clone();
        let controller_index = node_payload.net.controller_index;
        *world.get_mut::<SceneNodePayload>(root).unwrap() = node_payload;
        world.get_mut::<NiObjectNet>(root).unwrap().name = name;

        // 1 NiControllerManager targeting the root.
        let mut bytes = Vec::new();
        u32_le(&mut bytes, NULL_REFERENCE);
        bytes.extend(0x0008u16.to_le_bytes());
        for value in [1.0f32, 0.0, 0.0, 1.0] {
            f32_le(&mut bytes, value);
        }
        u32_le(&mut bytes, 0);
        bytes.push(0);
        u32_le(&mut bytes, 1);
        u32_le(&mut bytes, 2);
        u32_le(&mut bytes, 3);
        let manager_payload = parse_controller_manager_payload(&bytes, OBLIVION_VERSION).unwrap();
        assert_eq!(manager_payload.bytes_consumed, bytes.len());
        assert_eq!(
            load_controller_manager(&mut world, manager, &manager_payload, &table),
            Ok(true)
        );

        // 2 NiControllerSequence "Idle": one item on the root's transform.
        let mut bytes = Vec::new();
        owned(&mut bytes, "Idle");
        u32_le(&mut bytes, 1);
        u32_le(&mut bytes, 0);
        u32_le(&mut bytes, 5);
        u32_le(&mut bytes, NULL_REFERENCE);
        bytes.push(9);
        u32_le(&mut bytes, 4);
        for offset in [0, 0xFFFF, 5, 0xFFFF, 0xFFFF] {
            u32_le(&mut bytes, offset);
        }
        f32_le(&mut bytes, 1.0);
        u32_le(&mut bytes, NULL_REFERENCE);
        u32_le(&mut bytes, 0);
        f32_le(&mut bytes, 1.0);
        f32_le(&mut bytes, 0.0);
        f32_le(&mut bytes, 1.0);
        u32_le(&mut bytes, 1);
        u32_le(&mut bytes, 0);
        u32_le(&mut bytes, 4);
        let sequence_payload =
            parse_controller_sequence_payload(&bytes, OBLIVION_VERSION, 11).unwrap();
        assert_eq!(sequence_payload.bytes_consumed, bytes.len());
        assert_eq!(
            load_controller_sequence(&mut world, this, &sequence_payload, &table),
            Ok(true)
        );

        // 3 NiDefaultAVObjectPalette: scene root, "Root" -> 0.
        let mut bytes = Vec::new();
        u32_le(&mut bytes, 0);
        u32_le(&mut bytes, 1);
        owned(&mut bytes, "Root");
        u32_le(&mut bytes, 0);
        let palette_payload = parse_av_object_palette_payload(&bytes, OBLIVION_VERSION).unwrap();
        assert_eq!(palette_payload.bytes_consumed, bytes.len());
        apply_av_object_palette_payload(&mut world, palette, &palette_payload, &table).unwrap();

        // 4 NiStringPalette "Root\0NiTransformController\0".
        let strings = b"Root\0NiTransformController\0";
        let mut bytes = Vec::new();
        u32_le(&mut bytes, strings.len() as u32);
        bytes.extend(strings);
        u32_le(&mut bytes, strings.len() as u32);
        let (strings_state, consumed) = decode_string_palette_payload(&bytes).unwrap();
        assert_eq!(consumed, bytes.len());
        world.entity_mut(strings_owner).insert(strings_state);

        // 5 NiTransformInterpolator: static transform, no data.
        let mut bytes = Vec::new();
        for value in [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0] {
            f32_le(&mut bytes, value);
        }
        let decoded = decode_quat_transform(&bytes, OBLIVION_VERSION).unwrap();
        world
            .get_mut::<TransformInterpolator>(interpolator)
            .unwrap()
            .load_transform_and_data(&decoded.value, None);

        // The link pass.
        assert_eq!(
            publish_node_controller_head(&mut world, root, controller_index, &table),
            Ok(true)
        );
        assert_eq!(
            link_controller_manager(
                &mut world,
                manager,
                &manager_payload,
                &table,
                OBLIVION_VERSION
            ),
            Ok(true)
        );
        assert_eq!(
            link_controller_sequence(&mut world, this, &sequence_payload, &table),
            Ok(true)
        );
        assert_eq!(head(&world, root), Some(manager));
        assert_eq!(base(&world, manager).time.target_30, Some(root));
        let state = manager_state(&world, manager);
        assert_eq!(state.object_palette_7c, Some(palette));
        assert_eq!(state.sequence_size_46, 1);
        assert_eq!(state.sequence_slots_40[0], Some(this));
        assert_eq!(state.find_sequence(b"Idle"), Some(this));
        let state = sequence(&world, this);
        assert_eq!(state.manager_40, Some(manager));
        assert_eq!(state.accum_root_name_5c.as_deref(), Some(&b"Root"[..]));
        assert_eq!(state.items_14[0].priority_0d, 9);
        assert_eq!(state.items_14[0].interpolator_00, Some(interpolator));
        assert_eq!(state.id_tags_18[0].palette_00, Some(strings_owner));
        assert_eq!(state.string_palette_64, Some(strings_owner));

        assert!(controller_manager_post_load(&mut world, manager));
        let multi = multi_target_after_manager(&world, manager).unwrap();
        assert_eq!(base(&world, multi).time.target_30, Some(root));
        assert_eq!(slot_of(&world, this, 0).0, multi);
        assert_eq!(flags(&world, root), 0x0016);
    }

    /// IDA: 006C5FC0 ComputeScaledTime loop, clamp and store.
    #[test]
    fn sequence_scaled_time() {
        let range = ControllerSequence {
            begin_key_time_2c: 0.0,
            end_key_time_30: 2.0,
            ..ControllerSequence::default()
        };
        let mut looped = range.clone();
        assert_eq!(looped.compute_scaled_time(0.5, true), 0.5);
        assert_eq!(looped.last_time_34, 0.5);
        assert_eq!(looped.weighted_last_time_38, 0.5, "from zero weighted time");
        assert_eq!(looped.last_scaled_time_3c, 0.5);
        assert_eq!(looped.compute_scaled_time(2.5, true), 0.5, "fmod wrap");
        assert_eq!(looped.weighted_last_time_38, 2.5, "+38 stays unwrapped");
        let mut exact = range.clone();
        assert_eq!(
            exact.compute_scaled_time(2.0, false),
            2.0,
            "equal to the span"
        );
        assert!(exact.last_time_34 < -1e38, "nothing stored");
        let mut clamped = ControllerSequence {
            cycle_type_24: 2,
            frequency_28: 2.0,
            ..range
        };
        assert_eq!(clamped.compute_scaled_time(3.0, false), 2.0);
        assert_eq!(clamped.compute_scaled_time(-1.0, false), 0.0);
    }

    fn sequence_on(world: &mut World, manager: Entity, blend: Entity) -> Entity {
        let interpolator = spawn_transform_interpolator(world);
        let this = spawn_sequence(world);
        let mut state = sequence_mut(world, this);
        state.manager_40 = Some(manager);
        state.begin_key_time_2c = 0.0;
        state.end_key_time_30 = 10.0;
        state.items_14.push(SequenceInterpItem {
            interpolator_00: Some(interpolator),
            blend_08: SequenceBlendRef::Transform(blend),
            ..SequenceInterpItem::default()
        });
        this
    }

    fn spawn_transform_blend(world: &mut World) -> Entity {
        world
            .spawn((
                NiClass::BlendTransformInterpolator,
                ParticleBlendTransformInterpolator::default(),
            ))
            .id()
    }

    fn blend_base(world: &World, blend: Entity) -> &ParticleBlendCold {
        &world
            .get::<ParticleBlendTransformInterpolator>(blend)
            .unwrap()
            .base
    }

    /// IDA: 006C9BA0 Activate, 006CA950 Update, 006C9CB0 Deactivate, 006C4200.
    #[test]
    fn sequence_activation_update() {
        use SequenceUpdateStatus::Updated;
        let mut world = World::new();
        let manager = spawn_manager(&mut world);
        base_mut(&mut world, manager).time.fields.flags = 0x0008;
        let blend = spawn_transform_blend(&mut world);
        let first = sequence_on(&mut world, manager, blend);
        assert!(activate_sequence(
            &mut world, first, 0, false, 1.0, 0.0, None, false
        ));
        assert_eq!(
            sequence(&world, first).state_44,
            SequenceState::Animating as u32
        );
        assert_eq!(manager_state(&world, manager).active_sequences_4c.len(), 1);
        assert_eq!(sequence(&world, first).items_14[0].byte_0c, 0);
        assert_eq!(blend_base(&world, blend).active_count_0e, 1);
        assert!(
            !activate_sequence(&mut world, first, 0, false, 1.0, 0.0, None, false),
            "registers once"
        );
        assert_eq!(update_sequence(&mut world, first, 1.0, true), Updated);
        assert_eq!(sequence(&world, first).offset_48, -1.0);
        assert_eq!(sequence(&world, first).start_time_4c, -f32::MAX);
        assert_eq!(blend_base(&world, blend).single_time_20, 0.0);
        assert_eq!(
            update_controller_manager(&mut world, manager, 1.5),
            NodeControllerUpdateStatus::Applied
        );
        assert_eq!(blend_base(&world, blend).single_time_20, 0.5);

        let second = sequence_on(&mut world, manager, blend);
        assert!(activate_sequence(
            &mut world, second, 0, true, 0.5, 2.0, None, false
        ));
        assert_eq!(
            sequence(&world, second).state_44,
            SequenceState::EaseIn as u32
        );
        assert_eq!(sequence(&world, second).end_time_50, 2.0);
        assert_eq!(blend_base(&world, blend).active_count_0e, 2);
        assert_eq!(update_sequence(&mut world, second, 10.0, true), Updated);
        assert_eq!(sequence(&world, second).start_time_4c, 10.0);
        assert_eq!(sequence(&world, second).end_time_50, 12.0);
        let slot = usize::from(sequence(&world, second).items_14[0].byte_0c);
        let item = blend_base(&world, blend).items_14[slot];
        assert_eq!(item.weight, 0.5);
        assert_eq!(item.ease, 0.0, "spinner 0");
        assert_eq!(item.runtime_time, 0.0);
        assert_eq!(update_sequence(&mut world, second, 11.0, true), Updated);
        assert_eq!(blend_base(&world, blend).items_14[slot].ease, 0.5);
        assert_eq!(update_sequence(&mut world, second, 12.0, true), Updated);
        assert_eq!(
            sequence(&world, second).state_44,
            SequenceState::Animating as u32
        );

        assert!(deactivate_sequence(&mut world, second, 1.0, false));
        assert_eq!(
            sequence(&world, second).state_44,
            SequenceState::EaseOut as u32
        );
        assert_eq!(update_sequence(&mut world, second, 20.0, true), Updated);
        assert_eq!(sequence(&world, second).end_time_50, 21.0);
        assert_eq!(blend_base(&world, blend).items_14[slot].ease, 1.0);
        assert_eq!(update_sequence(&mut world, second, 21.0, true), Updated);
        assert_eq!(sequence(&world, second).state_44, 0);
        assert_eq!(
            blend_base(&world, blend).active_count_0e,
            1,
            "detaches at once"
        );
        assert_eq!(
            update_controller_manager(&mut world, manager, 22.0),
            NodeControllerUpdateStatus::Applied
        );
        assert_eq!(
            manager_state(&world, manager).active_sequences_4c.len(),
            1,
            "swap-removes inactive sequences"
        );
        assert!(deactivate_sequence(&mut world, first, 0.0, false));
        assert_eq!(sequence(&world, first).state_44, 0);
        assert_eq!(blend_base(&world, blend).active_count_0e, 0);
        assert!(sequence(&world, first).partner_58.is_none());
        assert!(sequence(&world, first).dest_frame_54 < -1e38);
    }

    fn text_keys(world: &mut World, keys: &[(f32, &str)]) -> Entity {
        world
            .spawn((
                NiClass::TextKeyExtraData,
                TextKeyExtraData {
                    keys: keys
                        .iter()
                        .map(|(time, text)| TextKey {
                            time_bits: time.to_bits(),
                            text: Some(text.as_bytes().to_vec()),
                        })
                        .collect(),
                },
            ))
            .id()
    }

    /// IDA: 006C6ED0 morph-key match and 006C6B30 corresponding frame.
    #[test]
    fn sequence_morph_sync() {
        let mut world = World::new();
        let source = spawn_sequence(&mut world);
        let target = spawn_sequence(&mut world);
        let source_keys = text_keys(&mut world, &[(0.0, "m:a"), (1.0, "m:b"), (1.5, "start")]);
        let target_keys = text_keys(&mut world, &[(0.0, "m:a"), (2.0, "m:b")]);
        {
            let mut state = sequence_mut(&mut world, source);
            state.begin_key_time_2c = 0.0;
            state.end_key_time_30 = 2.0;
            state.text_keys_20 = Some(source_keys);
        }
        {
            let mut state = sequence_mut(&mut world, target);
            state.begin_key_time_2c = 0.0;
            state.end_key_time_30 = 4.0;
            state.text_keys_20 = Some(target_keys);
        }
        assert!(verify_matching_morph_keys(&world, target, source));
        assert!(verify_matching_morph_keys(&world, source, target));
        assert_eq!(
            find_corresponding_morph_frame(&mut world, target, source, 0.5),
            Some(1.0),
            "between matching key intervals"
        );
        assert_eq!(
            find_corresponding_morph_frame(&mut world, target, source, 1.5),
            Some(3.0),
            "after the last key wraps to the first"
        );
        let other = spawn_sequence(&mut world);
        let other_keys = text_keys(&mut world, &[(0.0, "m:x")]);
        sequence_mut(&mut world, other).text_keys_20 = Some(other_keys);
        assert!(!verify_matching_morph_keys(&world, target, other));
        assert!(can_sync_to(&world, target, source), "an empty chain");
    }

    /// IDA: 006C9D30 StartBlend and 006C9E00 StartMorph.
    #[test]
    fn sequence_blend_morph() {
        let mut world = World::new();
        let manager = spawn_manager(&mut world);
        let blend = spawn_transform_blend(&mut world);
        let source = sequence_on(&mut world, manager, blend);
        let destination = sequence_on(&mut world, manager, blend);
        assert!(activate_sequence(
            &mut world, source, 0, false, 1.0, 0.0, None, false
        ));
        assert_eq!(
            update_sequence(&mut world, source, 1.0, true),
            SequenceUpdateStatus::Updated
        );
        assert!(start_blend(
            &mut world,
            source,
            Some(destination),
            1.0,
            0.5,
            0,
            1.0,
            0.75,
            None
        ));
        assert_eq!(
            sequence(&world, source).state_44,
            SequenceState::TransSource as u32
        );
        let state = sequence(&world, destination);
        assert_eq!(state.state_44, SequenceState::TransDest as u32);
        assert_eq!(state.weight_1c, 0.75);
        assert_eq!(state.dest_frame_54, 0.5);
        assert_eq!(state.end_time_50, 1.0);
        assert_eq!(sequence(&world, source).dest_frame_54, 0.0);
        assert_eq!(blend_base(&world, blend).active_count_0e, 2);

        let morph_manager = spawn_manager(&mut world);
        let morph_blend = spawn_transform_blend(&mut world);
        let morph_source = sequence_on(&mut world, morph_manager, morph_blend);
        let morph_target = sequence_on(&mut world, morph_manager, morph_blend);
        let keys = [(0.0, "m:a"), (5.0, "m:b")];
        let source_keys = text_keys(&mut world, &keys);
        let target_keys = text_keys(&mut world, &keys);
        sequence_mut(&mut world, morph_source).text_keys_20 = Some(source_keys);
        sequence_mut(&mut world, morph_target).text_keys_20 = Some(target_keys);
        assert!(start_morph(
            &mut world,
            morph_source,
            Some(morph_target),
            1.0,
            0,
            1.0,
            1.0
        ));
        assert_eq!(
            sequence(&world, morph_source).state_44,
            SequenceState::MorphSource as u32
        );
        assert_eq!(
            sequence(&world, morph_target).state_44,
            SequenceState::TransDest as u32
        );
        assert_eq!(
            sequence(&world, morph_source).partner_58,
            Some(morph_target)
        );
        assert_eq!(
            manager_state(&world, morph_manager)
                .active_sequences_4c
                .len(),
            2
        );
        assert_eq!(
            update_sequence(&mut world, morph_source, 2.0, true),
            SequenceUpdateStatus::Updated
        );
        assert_eq!(
            sequence(&world, morph_source).state_44,
            SequenceState::TransSource as u32
        );
    }
}
