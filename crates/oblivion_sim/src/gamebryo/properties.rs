//! The NiProperty classes startup reaches. Their name, extras and controller are `NiObjectNet`.

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;
use oblivion_core::reader::ByteReader;

use super::object_net::{ObjectNetPending, parse_object_net_payload, resolve_object_net_links};
use super::stream_objects::StreamObjectTable;

/// IDA: the version dividing modern flags from the legacy raw fields.
const MODERN_FLAGS_VERSION: u32 = 0x1401_0002;
const TRUNCATED_U16: &str = "truncated property flags";
const TRUNCATED_U32: &str = "truncated property u32";

/// IDA: the raw shift and OR, then a 16-bit store: overflow bits spill and per-write truncation applies.
const fn merge_flags(flags: u16, keep_mask: u16, raw: u32, shift: u32) -> u16 {
    ((flags as u32 & keep_mask as u32) | raw.wrapping_shl(shift)) as u16
}

// IDA: N/A a helper, with no address of its own.
fn body<'a>(bytes: &'a [u8], net: &ObjectNetPending) -> ByteReader<'a> {
    ByteReader::new(&bytes[net.bytes_consumed..])
}

/// IDA: 00700AC0 NiProperty_LoadBinary: 006FFCE0, then below 0A000102 a u16 stored to stream+302 for the subclass that
/// reads it. The returned NiObjectNET counts those two bytes, so `body` starts after them.
/// TODO: [ ] stream+302's readers (the < 0A000102 flag branches of the flag properties).
pub fn parse_property_net(
    bytes: &[u8],
    version: u32,
) -> Result<(ObjectNetPending, u16), GameError> {
    let mut net = parse_object_net_payload(bytes, version)?;
    let mut legacy_flags_302 = 0;
    if version < 0x0A00_0102 {
        legacy_flags_302 = body(bytes, &net).u16(TRUNCATED_U16)?;
        net.bytes_consumed += 2;
    }
    Ok((net, legacy_flags_302))
}

/// IDA: 00706CB0 NiZBufferProperty: +0x18 flags, cold 000F.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct ZBufferProperty {
    pub flags_18: u16,
}

impl Default for ZBufferProperty {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self { flags_18: 0x000F }
    }
}

/// IDA: 00706530 NiVertexColorProperty: +0x18 flags, cold 0008.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct VertexColorProperty {
    pub flags_18: u16,
}

impl Default for VertexColorProperty {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self { flags_18: 0x0008 }
    }
}

/// IDA: 00718F80 NiStencilProperty: +0x18 flags, +0x1C reference, +0x20 mask.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct StencilProperty {
    pub flags_18: u16,
    pub reference_1c: u32,
    pub mask_20: u32,
}

impl Default for StencilProperty {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            flags_18: 0x4180,
            reference_1c: 0,
            mask_20: 0xFFFF_FFFF,
        }
    }
}

/// A property's parsed fields, waiting for the link pass.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PropertyPending<T> {
    pub net: ObjectNetPending,
    pub property: T,
    pub bytes_consumed: usize,
}

/// IDA: 00706E10 after 006FFCE0: u16 flags and, below 14010002, a u32 function merged into bits 2..5 with
/// raw overflow. Completion: 100% of the modern base.
pub fn parse_z_buffer_property_payload(
    bytes: &[u8],
    version: u32,
) -> Result<PropertyPending<ZBufferProperty>, GameError> {
    let net = parse_object_net_payload(bytes, version)?;
    let mut reader = body(bytes, &net);
    let mut flags_18 = reader.u16(TRUNCATED_U16)?;
    if version < MODERN_FLAGS_VERSION {
        flags_18 = merge_flags(flags_18, 0xFFC3, reader.u32(TRUNCATED_U32)?, 2);
    }
    let bytes_consumed = net.bytes_consumed + reader.position();
    Ok(PropertyPending {
        net,
        property: ZBufferProperty { flags_18 },
        bytes_consumed,
    })
}

/// IDA: 00706690: u16 flags and, below 14010002, u32 source and lighting modes merged into bits 4 and 3.
pub fn parse_vertex_color_property_payload(
    bytes: &[u8],
    version: u32,
) -> Result<PropertyPending<VertexColorProperty>, GameError> {
    let net = parse_object_net_payload(bytes, version)?;
    let mut reader = body(bytes, &net);
    let mut flags_18 = reader.u16(TRUNCATED_U16)?;
    if version < MODERN_FLAGS_VERSION {
        flags_18 = merge_flags(flags_18, 0xFFCF, reader.u32(TRUNCATED_U32)?, 4);
        flags_18 = merge_flags(flags_18, 0xFFF7, reader.u32(TRUNCATED_U32)?, 3);
    }
    let bytes_consumed = net.bytes_consumed + reader.position();
    Ok(PropertyPending {
        net,
        property: VertexColorProperty { flags_18 },
        bytes_consumed,
    })
}

/// IDA: 00719110: modern flags, reference and mask; or a legacy enabled byte and seven raw u32 fields.
/// `initial_flags` carries the bits legacy masks leave alone; 4180 is the cold factory's.
pub fn parse_stencil_property_payload(
    bytes: &[u8],
    version: u32,
    initial_flags: u16,
) -> Result<PropertyPending<StencilProperty>, GameError> {
    let net = parse_object_net_payload(bytes, version)?;
    let mut reader = body(bytes, &net);
    let property = if version >= MODERN_FLAGS_VERSION {
        StencilProperty {
            flags_18: reader.u16(TRUNCATED_U16)?,
            reference_1c: reader.u32(TRUNCATED_U32)?,
            mask_20: reader.u32(TRUNCATED_U32)?,
        }
    } else {
        let enabled = reader.u8("truncated property byte")?;
        let mut flags = if enabled != 0 {
            initial_flags | 1
        } else {
            initial_flags & 0xFFFE
        };
        flags = merge_flags(flags, 0x0FFF, reader.u32(TRUNCATED_U32)?, 12);
        let reference_1c = reader.u32(TRUNCATED_U32)?;
        let mask_20 = reader.u32(TRUNCATED_U32)?;
        flags = merge_flags(flags, 0xFFF1, reader.u32(TRUNCATED_U32)?, 1);
        flags = merge_flags(flags, 0xFF8F, reader.u32(TRUNCATED_U32)?, 4);
        flags = merge_flags(flags, 0xFC7F, reader.u32(TRUNCATED_U32)?, 7);
        flags = merge_flags(flags, 0xF3FF, reader.u32(TRUNCATED_U32)?, 10);
        StencilProperty {
            flags_18: flags,
            reference_1c,
            mask_20,
        }
    };
    let bytes_consumed = net.bytes_consumed + reader.position();
    Ok(PropertyPending {
        net,
        property,
        bytes_consumed,
    })
}

/// IDA: 00B25AF4 the shared material ID counter, image value 1, InterlockedIncrement.
#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq)]
pub struct MaterialIdCounter(pub u32);

impl Default for MaterialIdCounter {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self(1)
    }
}

/// IDA: 007099A0's scalars, as raw values: +0x1C ambient, diffuse, specular, emissive, shine, alpha.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaterialPropertyValues {
    pub ambient: [f32; 3],
    pub diffuse: [f32; 3],
    pub specular: [f32; 3],
    pub emissive: [f32; 3],
    pub shine: f32,
    pub alpha: f32,
}

impl Default for MaterialPropertyValues {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            ambient: [0.5; 3],
            diffuse: [0.5; 3],
            specular: [0.0; 3],
            emissive: [0.0; 3],
            shine: 4.0,
            alpha: 1.0,
        }
    }
}

/// IDA: 007095A0 NiMaterialProperty: +0x18 interlocked ID, the values, and raw words +0x54 (cold 1) and +0x58.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct MaterialProperty {
    pub material_id_18: u32,
    pub values: MaterialPropertyValues,
    pub raw_state_54: u32,
    pub raw_state_58: u32,
}

/// IDA: 007095A0: every material construction takes the next ID.
pub fn new_material_property(world: &mut World) -> MaterialProperty {
    let mut counter = world.get_resource_or_insert_with(MaterialIdCounter::default);
    counter.0 = counter.0.wrapping_add(1);
    MaterialProperty {
        material_id_18: counter.0,
        values: MaterialPropertyValues::default(),
        raw_state_54: 1,
        raw_state_58: 0,
    }
}

/// IDA: 007099A0: 00700AC0, four RGB triples, then shine and alpha, as raw bits; no gate of its own. Completion: 100%
/// of the >= 0A000100 form.
pub fn parse_material_property_payload(
    bytes: &[u8],
    version: u32,
) -> Result<PropertyPending<MaterialPropertyValues>, GameError> {
    let (net, _) = parse_property_net(bytes, version)?;
    let mut reader = body(bytes, &net);
    let mut values = MaterialPropertyValues::default();
    for color in [
        &mut values.ambient,
        &mut values.diffuse,
        &mut values.specular,
        &mut values.emissive,
    ] {
        for component in color {
            *component = reader.f32(TRUNCATED_U32)?;
        }
    }
    values.shine = reader.f32(TRUNCATED_U32)?;
    values.alpha = reader.f32(TRUNCATED_U32)?;
    let bytes_consumed = net.bytes_consumed + reader.position();
    Ok(PropertyPending {
        net,
        property: values,
        bytes_consumed,
    })
}

/// IDA: 0073D670 NiSpecularProperty (0x1C, flags +0x18 cold 0); vt+0x4C (0073D5D0) answers 12, so 007077D0 never merges it.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SpecularProperty {
    pub flags_18: u16,
}

/// IDA: 007414A0 NiSpecularProperty_LoadBinary: NiProperty_LoadBinary (NiObjectNET), then the u16 flags at 0A000102 and
/// above. Completion: 100% at the modern floor.
pub fn parse_specular_property_payload(
    bytes: &[u8],
    version: u32,
) -> Result<PropertyPending<SpecularProperty>, GameError> {
    let net = parse_object_net_payload(bytes, version)?;
    let mut reader = body(bytes, &net);
    let flags_18 = reader.u16(TRUNCATED_U16)?;
    let bytes_consumed = net.bytes_consumed + reader.position();
    Ok(PropertyPending {
        net,
        property: SpecularProperty { flags_18 },
        bytes_consumed,
    })
}

/// IDA: 00700050's links on a parsed property. A rejected link leaves `target` unchanged.
pub fn resolve_property_links<T: Component + Clone>(
    world: &mut World,
    target: Entity,
    pending: &PropertyPending<T>,
    table: &StreamObjectTable,
) -> Result<(), GameError> {
    let net = resolve_object_net_links(world, &pending.net, table)?;
    world
        .entity_mut(target)
        .insert((net, pending.property.clone()));
    Ok(())
}

/// IDA: 00741340 -> 00700050, then the parsed scalars on the existing material.
pub fn resolve_material_property_links(
    world: &mut World,
    target: Entity,
    pending: &PropertyPending<MaterialPropertyValues>,
    table: &StreamObjectTable,
) -> Result<(), GameError> {
    let net = resolve_object_net_links(world, &pending.net, table)?;
    let mut entity = world.entity_mut(target);
    entity.insert(net);
    entity
        .get_mut::<MaterialProperty>()
        .expect("the target is a material")
        .values = pending.property;
    Ok(())
}

/// IDA: 00406C35 NiWireframeProperty, built inline: vtable A2FD5C, +0x18 flags 0. No stream factory reaches it.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WireframeProperty {
    pub flags_18: u16,
}

impl WireframeProperty {
    pub const ORIGINAL_VTABLE: u32 = 0x00A2_FD5C;
}

/// IDA: 007C82A0 BSFogProperty (0x34): +0x18 flags, +0x1C depth, +0x20 color, +0x2C start, +0x30 end.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct FogProperty {
    pub flags_18: u16,
    pub depth_1c: f32,
    pub color_20: [f32; 3],
    pub start_distance_2c: f32,
    pub end_distance_30: f32,
}

impl FogProperty {
    pub const ORIGINAL_VTABLE: u32 = 0x00A9_0584;

    /// IDA: 007C8270(amount): +0x2C = (1 - amount * 0.83) * 163840, each x87 step rounded to float, then +0x30 =
    /// 163840. Completion: 100% of the operation and store order.
    pub fn set_start_fraction(&mut self, amount: f32) {
        let scaled = amount * 0.83;
        let remaining = 1.0 - scaled;
        self.start_distance_2c = remaining * 163_840.0;
        self.end_distance_30 = 163_840.0;
    }
}

impl Default for FogProperty {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            flags_18: 0,
            depth_1c: 1.0,
            color_20: [0.0; 3],
            start_distance_2c: 27_852.803,
            end_distance_30: 163_840.0,
        }
    }
}

/// IDA: 007C5190 SkyShaderProperty (0x8C), constructor store values. Render-pass lists keep only their counts until
/// the first frame's port. Shader property ID 11.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct SkyShaderProperty {
    pub flags_18: u16,
    pub pass_info_1c: u32,
    pub alpha_20: f32,
    pub last_pass_state_24: u32,
    pub render_pass_counts: [u32; 4],
    pub classifier_68: i32,
    pub color_6c: [f32; 4],
    pub blend_texture_7c: Option<Entity>,
    pub blend_value_80: f32,
    pub layer_84: u16,
    pub pass_kind_88: u32,
}

impl SkyShaderProperty {
    pub const ORIGINAL_VTABLE: u32 = 0x00A9_03D4;
    pub const SHADER_PROPERTY_ID: i32 = 11;
}

impl Default for SkyShaderProperty {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            flags_18: 1,
            pass_info_1c: 0,
            alpha_20: 1.0,
            last_pass_state_24: 0,
            render_pass_counts: [0; 4],
            classifier_68: 0,
            color_6c: [1.0; 4],
            blend_texture_7c: None,
            blend_value_80: 0.0,
            layer_84: 0,
            pass_kind_88: 8,
        }
    }
}

/// IDA: 007EE440 BSShaderLightingProperty_Constructor. Simulation fields used by
/// geometry registration and actor detection. The host owns texture and shader resources.
#[derive(Component, Clone, Debug, Default)]
pub struct LightingProperty {
    /// High nibble of +0x1C, indexing the renderer's shadow scenes.
    pub scene_index_1c: u8,
    /// +0x1C bit 6, derived from the geometry alpha property during setup.
    pub alpha_blend_pass_1c: bool,
    pub alpha_20: f32,
    pub last_pass_state_24: u32,
    pub classifier_68: i32,
    /// +0x6C NiTPointerList. The shadow scene owns the lights.
    pub lights_6c: Vec<Entity>,
    /// +0x7C next node for 007ED160 / 007ED180.
    pub next_light_7c: Option<usize>,
    /// +0xBC slot 0, a BSShaderPPLightingProperty's diffuse NiSourceTexture: 007DA220 takes it from the property state's
    /// NiTexturingProperty base map (+0x20, its maps' first texture) before that property leaves the geometry.
    pub diffuse_map_bc: Option<Entity>,
}

impl LightingProperty {
    // IDA: 007ED160 BSShaderLightingProperty_GetFirstLight. Completion: head and cursor stores.
    pub fn first_light(&mut self) -> Option<Entity> {
        self.next_light_7c = (!self.lights_6c.is_empty()).then_some(0);
        self.next_light()
    }

    // IDA: 007ED180 BSShaderLightingProperty_GetNextLight. Completion: advance before returning the entry.
    pub fn next_light(&mut self) -> Option<Entity> {
        let i = self.next_light_7c?;
        self.next_light_7c = (i + 1 < self.lights_6c.len()).then_some(i + 1);
        self.lights_6c.get(i).copied()
    }
}

/// IDA: 0085BBE0 WaterShaderProperty (0x88), constructor store values. Shader property ID 12.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct WaterShaderProperty {
    pub flags_18: u16,
    pub pass_info_1c: u32,
    pub alpha_20: f32,
    pub last_pass_state_24: u32,
    pub render_pass_counts: [u32; 4],
    pub classifier_68: i32,
    pub dword_6c: u32,
    pub bytes_70: [u8; 3],
    pub dword_74: u32,
    pub dword_78: u32,
    pub float_7c: f32,
    pub float_80: f32,
    pub bytes_84: [u8; 2],
}

impl WaterShaderProperty {
    pub const ORIGINAL_VTABLE: u32 = 0x00A9_56EC;
    pub const SHADER_PROPERTY_ID: i32 = 12;
}

impl Default for WaterShaderProperty {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            flags_18: 1,
            pass_info_1c: 0,
            alpha_20: 1.0,
            last_pass_state_24: 0,
            render_pass_counts: [0; 4],
            classifier_68: 0,
            dword_6c: 0,
            bytes_70: [0; 3],
            dword_74: 0,
            dword_78: 0,
            float_7c: 1.0,
            float_80: 0.0,
            bytes_84: [0; 2],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::object_net::{NiExtraDataName, NiObjectNet};
    use super::super::stream_objects::NiClass;
    use super::*;

    const NEW: u32 = 0x1401_0002;
    const OLD: u32 = 0x1401_0001;

    fn make_net(name: &str, extras: &[u32]) -> Vec<u8> {
        let mut bytes = (name.len() as u32).to_le_bytes().to_vec();
        bytes.extend(name.as_bytes());
        bytes.extend((extras.len() as u32).to_le_bytes());
        for index in extras {
            bytes.extend(index.to_le_bytes());
        }
        bytes.extend(0xFFFF_FFFFu32.to_le_bytes());
        bytes
    }

    fn u16s(bytes: &mut Vec<u8>, value: u16) {
        bytes.extend(value.to_le_bytes());
    }

    fn u32s(bytes: &mut Vec<u8>, values: &[u32]) {
        for value in values {
            bytes.extend(value.to_le_bytes());
        }
    }

    /// Ported from `gamebryo_properties`.
    #[test]
    fn cold_values() {
        assert_eq!(ZBufferProperty::default().flags_18, 0x000F);
        assert_eq!(VertexColorProperty::default().flags_18, 0x0008);
        let stencil = StencilProperty::default();
        assert_eq!(
            (stencil.flags_18, stencil.reference_1c, stencil.mask_20),
            (0x4180, 0, 0xFFFF_FFFF)
        );
    }

    #[test]
    fn z_buffer_and_vertex_color_versions() {
        let mut z_new = make_net("", &[]);
        u16s(&mut z_new, 0xA55A);
        let length = z_new.len();
        z_new.push(0xAB);
        let direct = parse_z_buffer_property_payload(&z_new, NEW).unwrap();
        assert_eq!(
            (direct.property.flags_18, direct.bytes_consumed),
            (0xA55A, length)
        );
        for short in 0..length {
            assert!(parse_z_buffer_property_payload(&z_new[..short], NEW).is_err());
        }
        let mut z_old = make_net("", &[]);
        u16s(&mut z_old, 0x0003);
        u32s(&mut z_old, &[0xFFFF_FFFF]);
        let legacy = parse_z_buffer_property_payload(&z_old, OLD).unwrap();
        assert_eq!(
            legacy.property.flags_18, 0xFFFF,
            "high bits spill beyond the field"
        );
        for short in 0..z_old.len() {
            assert!(parse_z_buffer_property_payload(&z_old[..short], OLD).is_err());
        }

        let mut vertex_new = make_net("", &[]);
        u16s(&mut vertex_new, 0xABCD);
        assert_eq!(
            parse_vertex_color_property_payload(&vertex_new, NEW)
                .unwrap()
                .property
                .flags_18,
            0xABCD
        );
        let mut vertex_old = make_net("", &[]);
        u16s(&mut vertex_old, 0);
        u32s(&mut vertex_old, &[0xFFFF_FFFF, 0xFFFF_FFFF]);
        assert_eq!(
            parse_vertex_color_property_payload(&vertex_old, OLD)
                .unwrap()
                .property
                .flags_18,
            0xFFF8
        );
        for short in 0..vertex_old.len() {
            assert!(parse_vertex_color_property_payload(&vertex_old[..short], OLD).is_err());
        }
    }

    #[test]
    fn stencil_modern_and_legacy() {
        let mut modern = make_net("", &[]);
        u16s(&mut modern, 0xABCD);
        u32s(&mut modern, &[0x1234_5678, 0x89AB_CDEF]);
        let parsed = parse_stencil_property_payload(&modern, NEW, 0x4180).unwrap();
        assert_eq!(
            parsed.property,
            StencilProperty {
                flags_18: 0xABCD,
                reference_1c: 0x1234_5678,
                mask_20: 0x89AB_CDEF
            }
        );
        assert_eq!(parsed.bytes_consumed, modern.len());
        for short in 0..modern.len() {
            assert!(parse_stencil_property_payload(&modern[..short], NEW, 0x4180).is_err());
        }
        let mut legacy = make_net("", &[]);
        legacy.push(0x80);
        u32s(&mut legacy, &[0, 0x1234_5678, 0x89AB_CDEF, 0, 0, 0, 0]);
        let parsed = parse_stencil_property_payload(&legacy, OLD, 0x4180).unwrap();
        assert_eq!(
            parsed.property.flags_18, 0x0001,
            "the enabled byte is not a u32"
        );
        assert_eq!(parsed.property.reference_1c, 0x1234_5678);
        assert_eq!(parsed.bytes_consumed, legacy.len());
        for short in 0..legacy.len() {
            assert!(parse_stencil_property_payload(&legacy[..short], OLD, 0x4180).is_err());
        }
        let mut spill = make_net("", &[]);
        spill.push(0);
        u32s(&mut spill, &[0, 0, 0, 0xFFFF_FFFF, 0, 0, 0]);
        assert_eq!(
            parse_stencil_property_payload(&spill, OLD, 0x4180)
                .unwrap()
                .property
                .flags_18,
            0xF00E
        );
        assert!(parse_stencil_property_payload(&modern, 0x0A01_0071, 0x4180).is_err());
    }

    #[test]
    fn links_publish_after_parsing() {
        let mut world = World::new();
        let extra = world
            .spawn((
                NiClass::TextKeyExtraData,
                NiExtraDataName(Some(b"extra".to_vec())),
            ))
            .id();
        let mut table = StreamObjectTable::default();
        table.push("NiTextKeyExtraData", 0x006D_7450, extra);
        let mut payload = make_net("P", &[0]);
        u16s(&mut payload, 0x000F);
        let pending = parse_z_buffer_property_payload(&payload, NEW).unwrap();
        assert_eq!(pending.net.extra_indices.len(), 1);
        let target = world.spawn(ZBufferProperty::default()).id();
        assert!(
            world.get::<NiObjectNet>(target).is_none(),
            "parsing publishes nothing"
        );
        resolve_property_links(&mut world, target, &pending, &table).unwrap();
        let net = world.get::<NiObjectNet>(target).unwrap();
        assert_eq!(net.name.as_deref(), Some(&b"P"[..]));
        assert_eq!(net.extra_data, [extra]);
    }

    #[test]
    fn material_ids_scalars_and_atomic_links() {
        let mut world = World::new();
        let first = new_material_property(&mut world);
        let second = new_material_property(&mut world);
        assert_eq!(
            (first.material_id_18, second.material_id_18),
            (2, 3),
            "the counter starts at 1"
        );
        assert_eq!(first.values, MaterialPropertyValues::default());
        assert_eq!((first.raw_state_54, first.raw_state_58), (1, 0));

        let raw: [u32; 14] = [
            0x7FC1_2345,
            0x8000_0000,
            0x3F00_0000,
            0x3F80_0000,
            0x4000_0000,
            0x4040_0000,
            0,
            0x7F80_0000,
            0xFF80_0000,
            0x3E80_0000,
            0x3F40_0000,
            0x3F80_0000,
            0x4080_0000,
            0x3F00_0000,
        ];
        let mut payload = make_net("Mat", &[0]);
        u32s(&mut payload, &raw);
        let parsed = parse_material_property_payload(&payload, 0x1400_0004).unwrap();
        assert_eq!(parsed.bytes_consumed, payload.len());
        assert_eq!(
            parsed.property.ambient[0].to_bits(),
            raw[0],
            "raw bits, even a NaN"
        );
        assert_eq!(parsed.property.ambient[1].to_bits(), raw[1]);
        assert_eq!(parsed.property.specular[1].to_bits(), raw[7]);
        assert_eq!(parsed.property.emissive[2].to_bits(), raw[11]);
        assert_eq!(parsed.property.shine.to_bits(), raw[12]);
        assert_eq!(parsed.property.alpha.to_bits(), raw[13]);
        for short in 0..payload.len() {
            assert!(parse_material_property_payload(&payload[..short], 0x1400_0004).is_err());
        }

        let extra = world
            .spawn((
                NiClass::TextKeyExtraData,
                NiExtraDataName(Some(b"extra".to_vec())),
            ))
            .id();
        let mut table = StreamObjectTable::default();
        table.push("NiTextKeyExtraData", 0x006D_7450, extra);
        let target = world.spawn(first).id();
        resolve_material_property_links(&mut world, target, &parsed, &table).unwrap();
        assert_eq!(
            world.get::<NiObjectNet>(target).unwrap().extra_data,
            [extra]
        );
        assert_eq!(
            world
                .get::<MaterialProperty>(target)
                .unwrap()
                .values
                .ambient[0]
                .to_bits(),
            raw[0]
        );

        let mut bad = parsed.clone();
        bad.net.extra_indices[0] = 2;
        bad.property.alpha = 9.0;
        assert!(resolve_material_property_links(&mut world, target, &bad, &table).is_err());
        assert_eq!(
            world.get::<NiObjectNet>(target).unwrap().name.as_deref(),
            Some(&b"Mat"[..])
        );
        assert_eq!(
            world
                .get::<MaterialProperty>(target)
                .unwrap()
                .values
                .alpha
                .to_bits(),
            raw[13],
            "unchanged"
        );
    }

    /// IDA: 007C8270's order, with each step rounded to float.
    #[test]
    fn fog_start_fraction() {
        let mut fog = FogProperty::default();
        assert_eq!(fog.start_distance_2c.to_bits(), 27_852.803f32.to_bits());
        fog.set_start_fraction(1.0);
        assert_eq!(fog.start_distance_2c, (1.0f32 - 1.0f32 * 0.83) * 163_840.0);
        assert_eq!(fog.end_distance_30, 163_840.0);
    }
}
