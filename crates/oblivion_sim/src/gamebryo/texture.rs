//! NiSourceTexture and NiTexturingProperty, and the two NiFloatInterpController subclasses that animate them,
//! NiFlipController and NiTextureTransformController.

use bevy_ecs::prelude::*;
use oblivion_core::crt::c_prefix;
use oblivion_core::error::GameError;
use oblivion_core::reader::ByteReader;
use oblivion_core::x87::ftol64_low;

use super::animation::update_float_interpolator;
use super::blend_interpolator::{ParticleBlendSampleStatus, sample_particle_blend_float};
use super::object_net::{
    NiObjectNet, ObjectNetPending, parse_object_net_payload, resolve_object_net_links,
};
use super::particle_controller::{ParticleTimeControllerCold, ParticleTimeControllerPayload};
use super::stream_objects::{NULL_REFERENCE, NiClass, StreamObjectTable, read_owned_c_string};
use super::texture_loading::TextureRendererData;
use super::transform_controller::{
    ControllerLinks, ControllerTimeMap, NodeControllerUpdateStatus, SingleInterpControllerPayload,
    begin_controller_update, parse_single_interp_controller_payload,
};

/// IDA: 0071B090 and friends: 0x104-byte path buffers (MAX_PATH).
pub const TEXTURE_PATH_BUFFER: usize = 0x104;
/// IDA: 006D19DB f64 A3B150, the flip index bias (0.01 as f32 widened). 100% bits.
const FLIP_INDEX_BIAS: f64 = f64::from_bits(0x3F84_7AE1_4000_0000);
const TRUNCATED: &str = "truncated texture payload";

/// IDA: 007023E0 NiSourceTexture::LoadBinary, modern (>= 0A000104) branch after 006FFCE0 NiObjectNET: u8 external
/// flag, 00713620 owned filename (+34), 00712BC0 link count 1, one deferred NiPixelData link, u32 pixel layout
/// (+18), u32 mip mapped (+20), u32 alpha format (+1C), u8 static (+40), then (>= 0A010067) u8 direct-to-renderer
/// (+41).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceTexturePayload {
    pub net: ObjectNetPending,
    pub external_raw: u8,
    pub filename: Option<Vec<u8>>,
    pub pixel_data_index: u32,
    pub pixel_layout: u32,
    pub mip_mapped: u32,
    pub alpha_format: u32,
    pub static_raw: u8,
    pub direct_render_raw: Option<u8>,
    pub bytes_consumed: usize,
}

// IDA: 007023E0, read order and widths. From 0A000104: the external byte, the filename and the pixel-data reference.
// Below it: the external byte; if set, the filename and no reference (PushLinkIDCount 0); if clear, no filename and a
// u8 that says whether the reference follows.
// Completion: 100% of the >= 0A000100 reads.
// TODO: [ ] The < 05000001 format remaps, and releasing an earlier pixelData on the old external arm.
pub fn parse_source_texture_payload(
    bytes: &[u8],
    version: u32,
) -> Result<SourceTexturePayload, GameError> {
    if version < 0x0A00_0100 {
        return Err(GameError::unclassified(
            0,
            "legacy NiSourceTexture payload unsupported",
        ));
    }
    let net = parse_object_net_payload(bytes, version)?;
    let mut reader = ByteReader::new(&bytes[net.bytes_consumed..]);
    let external_raw = reader.u8(TRUNCATED)?;
    let (filename, pixel_data_index) = if version >= 0x0A00_0104 {
        (
            read_owned_c_string(&mut reader, TRUNCATED, "truncated texture owned string")?,
            reader.u32(TRUNCATED)?,
        )
    } else if external_raw != 0 {
        (
            read_owned_c_string(&mut reader, TRUNCATED, "truncated texture owned string")?,
            NULL_REFERENCE,
        )
    } else if reader.u8(TRUNCATED)? != 0 {
        (None, reader.u32(TRUNCATED)?)
    } else {
        (None, NULL_REFERENCE)
    };
    let pixel_layout = reader.u32(TRUNCATED)?;
    let mip_mapped = reader.u32(TRUNCATED)?;
    let alpha_format = reader.u32(TRUNCATED)?;
    // 007026E1..007026FA remap pixel layout 4/5 only below 05000001.
    let static_raw = reader.u8(TRUNCATED)?;
    let direct_render_raw = if version >= 0x0A01_0067 {
        Some(reader.u8(TRUNCATED)?)
    } else {
        None
    };
    let bytes_consumed = net.bytes_consumed + reader.position();
    Ok(SourceTexturePayload {
        net,
        external_raw,
        filename,
        pixel_data_index,
        pixel_layout,
        mip_mapped,
        alpha_format,
        static_raw,
        direct_render_raw,
        bytes_consumed,
    })
}

/// IDA: stream-owned services consulted inside 007023E0. The search path is the 0040E63D BSSearchPath_Create
/// product (stream +1E8; 007478F0 SetFilePath, +4 Reset, +8 GetNextSearchPath); the palette is the WinMain
/// 0040E6B8 BSTexturePalette alias 00B3FAC8 (stream +254; +4 GetTexture, +8 SetTexture); `file_exists` is
/// g_bsFileExistsCallback (00748160) with mode 0.
pub trait SourceTextureLoadServices {
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    /// Candidate paths after SetFilePath(platform filename) + Reset, in order.
    fn search_candidates(&mut self, file_path: &[u8]) -> Vec<Vec<u8>>;
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn find_texture(&mut self, path: &[u8]) -> Option<Entity>;
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn register_texture(&mut self, path: &[u8], texture: Entity);
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn file_exists(&mut self, path: &[u8]) -> bool;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceTextureLoadStatus {
    Loaded,
    /// Missing host services, or a path the original copies with strcpy_s/strncpy_s past 0x104 bytes
    /// (invalid-parameter handler).
    Unsupported,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceTextureLoadResult {
    pub status: SourceTextureLoadStatus,
    /// 00712980 stream +0x24: the shared texture that replaces this object in the stream table; this duplicate is
    /// then released by 00702765.
    pub replacement: Option<Entity>,
    /// IDA: N/A 007023E0 registered this object in the palette, so the palette now holds it too.
    pub registered: bool,
}

// IDA: N/A a helper, with no address of its own.
const fn unsupported_load() -> SourceTextureLoadResult {
    SourceTextureLoadResult {
        status: SourceTextureLoadStatus::Unsupported,
        replacement: None,
        registered: false,
    }
}

// IDA: N/A a helper, with no address of its own.
const fn fits_path_buffer(path: &[u8]) -> bool {
    path.len() < TEXTURE_PATH_BUFFER
}

/// IDA: 007021F0 NiSourceTexture factory: FormHeap 0x48, 00701CD0 constructor: NiObjectNET base (`NiObjectNet`),
/// NiTexture vtable A7D874, format prefs +18 = 6 (pixel layout), +1C = 3 (alpha), +20 = 2 (mip), renderer data +24
/// null, 00701B00 global texture-list insertion (+28/+2C), then +30..+44 cold and vtable A7D8CC. 100% of the cold
/// state, load (with host services), link and PostLoad.
/// TODO: [ ] Bind the 00701B00 global list (B3F700/B3F704 under B3F780).
#[derive(Component, Clone, Debug, PartialEq)]
pub struct SourceTexture {
    /// IDA: vtable A7D8CC, or A7F644 for the NiSourceCubeMap of 00720A30/00720F80 with +0x48 = 0.
    pub original_vtable: u32,
    pub pixel_layout_18: u32,
    pub alpha_format_1c: u32,
    pub mip_mapped_20: u32,
    /// Presence of +0x24, the flag the stream path uses; `renderer_24` holds the object when one was built.
    pub renderer_data_24: bool,
    pub unknown_30: u8,
    pub filename_34: Option<Vec<u8>>,
    pub platform_filename_38: Option<Vec<u8>>,
    /// Presence of +0x3C; `app_pixel_data_3c` holds the NiPixelData when one was bound.
    pub pixel_data_3c: bool,
    pub static_40: bool,
    pub direct_to_renderer_41: bool,
    pub unknown_44: u32,
    pub cube_48: u32,
    pub links_resolved: bool,
    pub app_pixel_data_3c: Option<Entity>,
    /// IDA: +0x24 NiDX9SourceTextureData / NiDX9SourceCubeMapData, without the device objects.
    pub renderer_24: Option<TextureRendererData>,
}

impl Default for SourceTexture {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            original_vtable: 0x00A7_D8CC,
            pixel_layout_18: 6,
            alpha_format_1c: 3,
            mip_mapped_20: 2,
            renderer_data_24: false,
            unknown_30: 0,
            filename_34: None,
            platform_filename_38: None,
            pixel_data_3c: false,
            static_40: true,
            direct_to_renderer_41: true,
            unknown_44: 0,
            cube_48: 0,
            links_resolved: false,
            app_pixel_data_3c: None,
            renderer_24: None,
        }
    }
}

impl SourceTexture {
    // IDA: 007023E0 after the field reads: platform filename 0071B090, then the external search-path/palette
    // resolution or the internal palette lookup.
    // Completion: 100% of the branch order and store points.
    // TODO: [ ] The C++ tests each service for null; here `None` is all four missing at once, coarser but
    // the same two arms.
    pub fn load(
        &mut self,
        this: Entity,
        payload: &SourceTexturePayload,
        services: Option<&mut dyn SourceTextureLoadServices>,
    ) -> SourceTextureLoadResult {
        // 00702454: +34 receives the owned filename pointer.
        self.filename_34.clone_from(&payload.filename);
        // 0070250E: 0071B090 copies the C string into a fresh 0x104 buffer (00B3FD24 platform directory is never
        // set) through strncpy_s.
        if let Some(filename) = &self.filename_34 {
            let name = c_prefix(filename);
            if !fits_path_buffer(name) {
                return unsupported_load();
            }
            self.platform_filename_38 = Some(name.to_vec());
        }
        let mut replacement = None;
        let mut registered = false;
        if payload.external_raw != 0 {
            let Some(services) = services else {
                return unsupported_load();
            };
            let file_path = self.platform_filename_38.clone().unwrap_or_default();
            let mut done = false;
            for candidate in services.search_candidates(&file_path) {
                if !fits_path_buffer(&candidate) {
                    return unsupported_load();
                }
                if let Some(found) = services.find_texture(&candidate) {
                    self.platform_filename_38 = Some(candidate);
                    replacement = Some(found);
                    done = true;
                    break;
                }
            }
            if !done {
                for candidate in services.search_candidates(&file_path) {
                    if !fits_path_buffer(&candidate) {
                        return unsupported_load();
                    }
                    if services.file_exists(&candidate) {
                        self.platform_filename_38 = Some(candidate.clone());
                        services.register_texture(&candidate, this);
                        registered = true;
                        break;
                    }
                }
            }
        } else if let Some(platform) = self.platform_filename_38.clone() {
            let Some(services) = services else {
                return unsupported_load();
            };
            match services.find_texture(&platform) {
                Some(found) => replacement = Some(found),
                None => {
                    services.register_texture(&platform, this);
                    registered = true;
                }
            }
        }
        // 00702665..0070275E: format prefs, static and direct flags land on this object even when the stream slot
        // was replaced.
        self.pixel_layout_18 = payload.pixel_layout;
        self.mip_mapped_20 = payload.mip_mapped;
        self.alpha_format_1c = payload.alpha_format;
        self.static_40 = payload.static_raw != 0;
        if let Some(direct) = payload.direct_render_raw {
            self.direct_to_renderer_41 = direct != 0;
        }
        SourceTextureLoadResult {
            status: SourceTextureLoadStatus::Loaded,
            replacement,
            registered,
        }
    }

    // IDA: 007203F0 PostLoad: only when 00B256CC (set by 00498340 during renderer init) is set, and under critical
    // section 00B3FC00, vtable +5C 00701C00: no renderer (00B3F928) fails; a null +24 calls renderer +104; then
    // 00B3F958 && +24 && static calls +58 007021B0 (release pixel data).
    // Completion: 100% of the branches; the renderer call is a host service.
    // TODO: [ ] The D3D9 NiDX9SourceTextureData creation is the device boundary. The critical section is a host
    // substitution: exclusive access to the component.
    pub fn post_load(
        &mut self,
        precache_enabled: bool,
        release_static_pixel_data: bool,
        create_renderer_data: Option<&mut dyn FnMut(&mut Self) -> bool>,
    ) {
        if !precache_enabled {
            return;
        }
        let Some(create_renderer_data) = create_renderer_data else {
            return; // 00701C0D: no renderer
        };
        if !self.renderer_data_24 {
            if !create_renderer_data(self) {
                return;
            }
            self.renderer_data_24 = true;
        }
        if release_static_pixel_data && self.renderer_data_24 && self.static_40 {
            self.pixel_data_3c = false; // 007021B0
        }
    }
}

// IDA: 00702260: 00700050 NiObjectNET link, then the pixel data only when the pushed link count is nonzero (always 1
// on the modern branch). `Ok(false)` where the C++ link returns false.
// Completion: 100% of the supported null pixel-data and plain NET subset.
// TODO: [ ] Port the NiPixelData stream link and the shared-texture re-link side effects.
pub fn link_source_texture(
    world: &mut World,
    target: Entity,
    payload: &SourceTexturePayload,
    table: &StreamObjectTable,
) -> Result<bool, GameError> {
    // NiPixelData's stream link is not ported; a non-null link has no host owner.
    if payload.pixel_data_index != NULL_REFERENCE {
        return Ok(false);
    }
    if payload.net.controller_index != NULL_REFERENCE {
        return Ok(false);
    }
    let resolved = world
        .get::<SourceTexture>(target)
        .ok_or(GameError::unclassified(
            0,
            "source texture link target missing",
        ))?
        .links_resolved;
    if !resolved {
        let net = resolve_object_net_links(world, &payload.net, table)?;
        world.entity_mut(target).insert(net);
    } else if !payload.net.extra_indices.is_empty() {
        // A shared replacement is re-linked with this stream's links; appended extra data is not modelled by the host.
        return Ok(false);
    }
    let mut texture = world
        .get_mut::<SourceTexture>(target)
        .expect("checked above");
    texture.pixel_data_3c = false; // 00702284 NiRefObjectPtr_Assign(null)
    texture.links_resolved = true;
    Ok(true)
}

/// IDA: 00703A30 NiTextureTransform (0x48, no vtable): translate +0 (00B3FC80 zero), rotate +8 = 0, scale +0C = (1, 1),
/// center +14 = (0.5, 0.5), dirty byte +1C = 1, matrix +20..+40 uninitialized, method +44 = 0. 100% of the cold state,
/// the 0072FF90 load, 0072FF40 explicit construction and 0072FDF0.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextureTransform {
    pub translate_00: [f32; 2],
    pub rotate_08: f32,
    pub scale_0c: [f32; 2],
    pub center_14: [f32; 2],
    pub dirty_1c: bool,
    pub matrix_20: Option<[f32; 9]>,
    pub method_44: u32,
}

impl Default for TextureTransform {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            translate_00: [0.0; 2],
            rotate_08: 0.0,
            scale_0c: [1.0; 2],
            center_14: [0.5; 2],
            dirty_1c: true,
            matrix_20: None,
            method_44: 0,
        }
    }
}

impl TextureTransform {
    // IDA: 0072FDF0 UpdateMatrix: f32 CRT cos/sin of rotate; methods 0 and 1 write the nine entries in their verified
    // operand order; every method clears the dirty byte. Precision: 24-bit per step on the main thread after CreateDevice.
    // The `as f32` on the trig results is the fstp m32 at 0072FDFE/0072FE12; __CIcos/__CIsin do not round at the control
    // word's precision (00986000 takes the SSE2 double path when it is enabled).
    // Completion: 95%; CRT trig bits remain host values.
    // TODO: [ ] Compare __CIcos/__CIsin bits at runtime.
    pub fn update_matrix(&mut self) {
        let cosine = f64::from(self.rotate_08).cos() as f32;
        let sine = f64::from(self.rotate_08).sin() as f32;
        let c = cosine;
        let s = sine;
        if self.method_44 == 0 || self.method_44 == 1 {
            let mut m = self.matrix_20.unwrap_or([0.0; 9]);
            let sx = self.scale_0c[0];
            let sy = self.scale_0c[1];
            let dy = self.translate_00[1] - self.center_14[1];
            m[0] = sx * c;
            m[6] = 0.0;
            m[7] = 0.0;
            m[8] = 1.0;
            if self.method_44 == 0 {
                m[3] = sx * s;
                let sy_c = sy * c;
                let negative_s = -s;
                m[1] = negative_s * sy;
                m[4] = sy_c;
                let dx = self.translate_00[0] - self.center_14[0];
                m[5] = (c * dy + s * dx) + self.center_14[1];
                m[2] = (c * dx + negative_s * dy) + self.center_14[0];
            } else {
                m[3] = -s * sy;
                let sy_c = sy * c;
                m[1] = sx * s;
                m[4] = sy_c;
                let offset = -self.center_14[0] - self.translate_00[0];
                m[5] = (c * dy - s * offset) * sy + self.center_14[1];
                m[2] = (c * offset + s * dy) * sx + self.center_14[0];
            }
            self.matrix_20 = Some(m);
        }
        self.dirty_1c = false;
    }

    // IDA: 006D6A40: when not already dirty, equal values (no NaN) keep dirty cleared; otherwise dirty is set.
    // Completion: 100%.
    pub fn set_translate(&mut self, value: [f32; 2]) {
        self.dirty_1c = self.dirty_1c
            || !(self.translate_00[0] == value[0] && self.translate_00[1] == value[1]);
        self.translate_00 = value;
    }

    // IDA: 006D6A90, the same rule. Completion: 100%.
    pub fn set_rotate(&mut self, value: f32) {
        self.dirty_1c = self.dirty_1c || self.rotate_08 != value;
        self.rotate_08 = value;
    }

    // IDA: 006D6AD0, the same rule. Completion: 100%.
    pub fn set_scale(&mut self, value: [f32; 2]) {
        self.dirty_1c =
            self.dirty_1c || !(self.scale_0c[0] == value[0] && self.scale_0c[1] == value[1]);
        self.scale_0c = value;
    }
}

/// IDA: NiTexturingProperty::Map (0x10, vtable A7DBB4), BumpMap (0x28, A7DBC4), ShaderMap (0x14, A7DBD4).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TexturingMapKind {
    #[default]
    Map,
    Bump,
    Shader,
}

/// IDA: Map: flags +4 cold 0x3100 (clamp << 12 | filter << 8 | uv set), texture +8, transform +0C. BumpMap adds luma
/// scale +10 = 1, offset +14 = 0 and matrix +18..+24 = (0.5, 0, 0, 0.5); ShaderMap adds ID +10 = 0. A map is not a
/// stream object, so it stays inside `TexturingProperty`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TexturingMap {
    pub kind: TexturingMapKind,
    pub flags_04: u16,
    pub texture_08: Option<Entity>,
    pub transform_0c: Option<TextureTransform>,
    pub luma_scale_10: f32,
    pub luma_offset_14: f32,
    pub bump_matrix_18: [f32; 4],
    pub shader_id_10: u32,
}

impl Default for TexturingMap {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            kind: TexturingMapKind::Map,
            flags_04: 0x3100,
            texture_08: None,
            transform_0c: None,
            luma_scale_10: 1.0,
            luma_offset_14: 0.0,
            bump_matrix_18: [0.5, 0.0, 0.0, 0.5],
            shader_id_10: 0,
        }
    }
}

impl TexturingMap {
    /// IDA: 00405710 Map::SetTexture refcount swap. Completion: 100% as an assignment.
    pub const fn set_texture(&mut self, value: Option<Entity>) {
        self.texture_08 = value;
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TexturingMapPayload {
    pub texture_index: u32,
    pub flags: u16,
    pub transform: Option<TextureTransform>,
    pub luma_scale: f32,
    pub luma_offset: f32,
    pub bump_matrix: [f32; 4],
    pub shader_id: u32,
}

/// IDA: 00704AD0 NiTexturingProperty::LoadBinary after 00700AC0 NiProperty: < 14010002 u32 apply mode (flags (f & FFF1)
/// | apply << 1), else u16 flags & F00F; u32 map count; per map a presence byte and Map/BumpMap (index 5)
/// 007052F0/007055F0; >= 05000011 u32 shader-map count and per entry a byte and ShaderMap 00705790.
#[derive(Clone, Debug, PartialEq)]
pub struct TexturingPropertyPayload {
    pub net: ObjectNetPending,
    pub flags: u16,
    pub map_count: u32,
    pub maps: Vec<Option<TexturingMapPayload>>,
    pub shader_map_count: Option<u32>,
    pub shader_maps: Vec<Option<TexturingMapPayload>>,
    pub bytes_consumed: usize,
}

/// IDA: 0072FF90 NiTextureTransform::LoadBinary on a fresh 00703A30 object: 00714BA0 translate (+0), 00714BA0 scale (+0C),
/// f32 rotate (+8), u32 method (+44), 00714BA0 center (+14), then dirty = 1. Completion: 100% of the read order.
fn read_texture_transform(reader: &mut ByteReader<'_>) -> Result<TextureTransform, GameError> {
    let translate_00 = [reader.f32(TRUNCATED)?, reader.f32(TRUNCATED)?];
    let scale_0c = [reader.f32(TRUNCATED)?, reader.f32(TRUNCATED)?];
    let rotate_08 = reader.f32(TRUNCATED)?;
    let method_44 = reader.u32(TRUNCATED)?;
    let center_14 = [reader.f32(TRUNCATED)?, reader.f32(TRUNCATED)?];
    Ok(TextureTransform {
        translate_00,
        rotate_08,
        scale_0c,
        center_14,
        dirty_1c: true,
        matrix_20: None,
        method_44,
    })
}

/// IDA: 007052F0 Map::LoadBinary (+ 007055F0 BumpMap, 00705790 ShaderMap) on a fresh map whose flags are 0x3100.
/// Completion: 100% of the version gates and the 16-bit flag packing of the u32 clamp/filter/uv fields.
fn read_texturing_map(
    reader: &mut ByteReader<'_>,
    kind: TexturingMapKind,
    version: u32,
) -> Result<TexturingMapPayload, GameError> {
    let texture_index = reader.u32(TRUNCATED)?;
    let flags = if version < 0x1401_0002 {
        let clamp = reader.u32(TRUNCATED)?;
        let mut flags = 0x3100u16 & 0xCFFF;
        flags |= clamp.wrapping_shl(12) as u16;
        let filter = reader.u32(TRUNCATED)?;
        flags = ((u32::from(flags) & 0xF0FF) | ((filter & 0xFF) << 8)) as u16;
        let uv = reader.u32(TRUNCATED)?;
        ((u32::from(flags) & 0xFF00) | (uv & 0xFFFF)) as u16
    } else {
        reader.u16(TRUNCATED)?
    };
    if version < 0x0A03_0004 {
        reader.u16(TRUNCATED)?; // PS2 L
        reader.u16(TRUNCATED)?; // PS2 K
    }
    if version < 0x0401_0010 {
        reader.u16(TRUNCATED)?;
    }
    let mut transform = None;
    // 00705461 clears +0C, then only the exact byte value 1 loads one.
    if version >= 0x0A00_010A && reader.u8(TRUNCATED)? == 1 {
        transform = Some(read_texture_transform(reader)?);
    }
    let mut map = TexturingMapPayload {
        texture_index,
        flags,
        transform,
        luma_scale: 0.0,
        luma_offset: 0.0,
        bump_matrix: [0.0; 4],
        shader_id: 0,
    };
    match kind {
        TexturingMapKind::Bump => {
            map.luma_scale = reader.f32(TRUNCATED)?;
            map.luma_offset = reader.f32(TRUNCATED)?;
            for entry in &mut map.bump_matrix {
                *entry = reader.f32(TRUNCATED)?;
            }
        }
        TexturingMapKind::Shader => map.shader_id = reader.u32(TRUNCATED)?,
        TexturingMapKind::Map => {}
    }
    Ok(map)
}

// IDA: 00704AD0. `current_flags` is the owner's current value (cold 0x0004). 00700AC0 NiProperty reads a u16 after
// NiObjectNET only below 0A000102.
// Completion: 100% of the >= 0A000100 branch.
// TODO: [ ] Port the pre-0303000D index shift and pre-04010000 u32 presence.
pub fn parse_texturing_property_payload(
    bytes: &[u8],
    version: u32,
    current_flags: u16,
) -> Result<TexturingPropertyPayload, GameError> {
    if version < 0x0A00_0100 {
        return Err(GameError::unclassified(
            0,
            "legacy NiTexturingProperty payload unsupported",
        ));
    }
    let (net, _) = super::properties::parse_property_net(bytes, version)?;
    let mut reader = ByteReader::new(&bytes[net.bytes_consumed..]);
    let flags = if version < 0x1401_0002 {
        let apply = reader.u32(TRUNCATED)?;
        (current_flags & 0xFFF1) | apply.wrapping_mul(2) as u16
    } else {
        reader.u16(TRUNCATED)? & 0xF00F
    };
    let map_count = reader.u32(TRUNCATED)?;
    if map_count as usize > bytes.len() {
        return Err(GameError::unclassified(
            0,
            "NiTexturingProperty map count exceeds payload",
        ));
    }
    let mut maps = Vec::with_capacity(map_count as usize);
    for index in 0..map_count {
        if reader.u8(TRUNCATED)? == 0 {
            maps.push(None);
            continue;
        }
        let kind = if index == 5 {
            TexturingMapKind::Bump
        } else {
            TexturingMapKind::Map
        };
        maps.push(Some(read_texturing_map(&mut reader, kind, version)?));
    }
    let mut shader_map_count = None;
    let mut shader_maps = Vec::new();
    if version >= 0x0500_0011 {
        let count = reader.u32(TRUNCATED)?;
        if count as usize > bytes.len() {
            return Err(GameError::unclassified(
                0,
                "NiTexturingProperty shader map count exceeds payload",
            ));
        }
        shader_map_count = Some(count);
        shader_maps.reserve(count as usize);
        for _ in 0..count {
            if reader.u8(TRUNCATED)? == 0 {
                shader_maps.push(None);
                continue;
            }
            shader_maps.push(Some(read_texturing_map(
                &mut reader,
                TexturingMapKind::Shader,
                version,
            )?));
        }
    }
    let bytes_consumed = net.bytes_consumed + reader.position();
    Ok(TexturingPropertyPayload {
        net,
        flags,
        map_count,
        maps,
        shader_map_count,
        shader_maps,
        bytes_consumed,
    })
}

/// IDA: N/A host construction of a published Map/BumpMap/ShaderMap from its decoded payload (texture bound later by the
/// link pass).
fn make_map(payload: &TexturingMapPayload, kind: TexturingMapKind) -> TexturingMap {
    let mut map = TexturingMap {
        kind,
        flags_04: payload.flags,
        transform_0c: payload.transform,
        ..TexturingMap::default()
    };
    match kind {
        TexturingMapKind::Bump => {
            map.luma_scale_10 = payload.luma_scale;
            map.luma_offset_14 = payload.luma_offset;
            map.bump_matrix_18 = payload.bump_matrix;
        }
        TexturingMapKind::Shader => map.shader_id_10 = payload.shader_id,
        TexturingMapKind::Map => {}
    }
    map
}

/// IDA: 00704A60 NiTexturingProperty factory: FormHeap 0x30, 007043B0: NiObjectNET base (`NiObjectNet`), flags +18,
/// NiTArray<Map*> +1C (capacity 7, grow 1) filled with seven nulls, shader-map array +2C null, final flags 0x0004. Link
/// 00703EC0; PostLoad +3C 00700160. 100% of the cold state, load and link.
/// TODO: [ ] Bind the renderer-side texture effects.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct TexturingProperty {
    pub flags_18: u16,
    pub maps_1c: Vec<Option<TexturingMap>>,
    pub shader_maps_2c: Option<Vec<Option<TexturingMap>>>,
    pub links_resolved: bool,
}

impl Default for TexturingProperty {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            flags_18: 0x0004,
            maps_1c: vec![None; 7],
            shader_maps_2c: None,
            links_resolved: false,
        }
    }
}

impl TexturingProperty {
    // IDA: 00704AD0 publication: 004E4A10 SetSize(count) then SetAt per index; new Map objects are created per present
    // entry; the trailing scan sets bit 0 when any map at index >= 1 exists.
    // Completion: 100% of the verified order.
    pub fn apply_loaded_payload(&mut self, payload: &TexturingPropertyPayload) {
        self.flags_18 = payload.flags;
        // 004E4A10 SetSize(count) drops slots beyond count (not deleted) and the per-index SetAt leaves exactly count
        // slots.
        self.maps_1c = vec![None; payload.map_count as usize];
        for (index, entry) in payload.maps.iter().enumerate() {
            if let Some(entry) = entry {
                let kind = if index == 5 {
                    TexturingMapKind::Bump
                } else {
                    TexturingMapKind::Map
                };
                self.maps_1c[index] = Some(make_map(entry, kind));
            }
        }
        if let Some(count) = payload.shader_map_count.filter(|&count| count != 0) {
            let mut shaders = vec![None; count as usize];
            for (index, entry) in payload.shader_maps.iter().enumerate() {
                if let Some(entry) = entry {
                    shaders[index] = Some(make_map(entry, TexturingMapKind::Shader));
                }
            }
            self.shader_maps_2c = Some(shaders);
        }
        let multitexture = self.maps_1c.iter().skip(1).any(Option::is_some);
        self.flags_18 = if multitexture {
            self.flags_18 | 0x0001
        } else {
            self.flags_18 & 0xFFFE
        };
    }
}

// IDA: 00703EC0 after 006D7DF0 (NiObjectNET) link: flags &= F00F; for each present map the deferred texture is swapped
// in, and present maps at index >= 6 increment the decal count (bits 4..11); then shader maps. `Ok(false)` where the C++
// link returns false; the property keeps its state on any failure.
// Completion: 100% of the order for NiSourceTexture targets.
pub fn link_texturing_property(
    world: &mut World,
    target: Entity,
    payload: &TexturingPropertyPayload,
    table: &StreamObjectTable,
    controller_head: Option<Entity>,
) -> Result<bool, GameError> {
    let mut property = world
        .get::<TexturingProperty>(target)
        .ok_or(GameError::unclassified(
            0,
            "texturing property link target missing",
        ))?
        .clone();
    if property.links_resolved {
        return Ok(false);
    }
    let mut net_pending = payload.net.clone();
    net_pending.controller_index = NULL_REFERENCE;
    let mut net: NiObjectNet = resolve_object_net_links(world, &net_pending, table)?;
    net.controller_0c = controller_head;
    property.flags_18 &= 0xF00F;
    for (index, entry) in payload.maps.iter().enumerate() {
        let Some(entry) = entry else {
            continue;
        };
        // 007124A0's deferred NiTexture link for the bound NiSourceTexture class; the outer `None` is a non-null class the
        // host does not support.
        let Some(texture) = super::mesh_geometry::typed_reference(
            world,
            table,
            entry.texture_index,
            NiClass::SourceTexture,
            "texture link index out of range",
        )?
        else {
            return Ok(false);
        };
        let Some(Some(map)) = property.maps_1c.get_mut(index) else {
            return Ok(false);
        };
        map.set_texture(texture);
        if index >= 6 {
            let decals = ((u32::from(property.flags_18) >> 4) & 0xFF) + 1;
            property.flags_18 = ((decals << 4) | (u32::from(property.flags_18) & 0xF00F)) as u16;
        }
    }
    for (index, entry) in payload.shader_maps.iter().enumerate() {
        let Some(entry) = entry else {
            continue;
        };
        let Some(texture) = super::mesh_geometry::typed_reference(
            world,
            table,
            entry.texture_index,
            NiClass::SourceTexture,
            "texture link index out of range",
        )?
        else {
            return Ok(false);
        };
        let Some(Some(map)) = property
            .shader_maps_2c
            .as_mut()
            .and_then(|maps| maps.get_mut(index))
        else {
            return Ok(false);
        };
        map.set_texture(texture);
    }
    property.links_resolved = true;
    world.entity_mut(target).insert((net, property));
    Ok(true)
}

/// A controller write the original would perform on memory the host cannot address.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextureControllerWrite {
    Applied,
    /// The C++ `NodeControllerUpdateStatus::unsupported_interpolator_range`.
    UnsupportedRange,
}

/// NiFlipController's own stream fields, the bytes after the single-interp prefix.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FlipControllerFields {
    pub affected_map_54: u32,
    pub texture_indices: Vec<u32>,
    pub bytes_consumed: usize,
}

// IDA: 006D1CD0 modern (>= 0A010068) order after 006CE320: u32 affected map (+54), then 00712AE0 deferred reference list
// (u32 count pushed through the link-count queue, then count link IDs).
// Completion: 100% of these widths.
// TODO: [ ] The legacy float-data conversion.
pub fn parse_flip_controller_fields(bytes: &[u8]) -> Result<FlipControllerFields, GameError> {
    let mut reader = ByteReader::new(bytes);
    let affected_map_54 = reader.u32(TRUNCATED)?;
    let count = reader.u32(TRUNCATED)?;
    if count as usize > reader.remaining() / 4 {
        return Err(GameError::stream("truncated NiFlipController texture list"));
    }
    let mut texture_indices = Vec::with_capacity(count as usize);
    for _ in 0..count {
        texture_indices.push(reader.u32(TRUNCATED)?);
    }
    Ok(FlipControllerFields {
        affected_map_54,
        texture_indices,
        bytes_consumed: reader.position(),
    })
}

/// IDA: 006D19D3..006D1A04: fistp under a forced truncate control word stores the low dword of the 64-bit result of
/// value + 0.01; then the u32 clamp to size - 1. Precision: 24-bit on the main thread after CreateDevice, so 006D19DB's
/// `fadd qword` rounds the exact sum to a float before the fistp. Completion: 100% of the index arithmetic.
#[must_use]
pub fn flip_controller_index(value: f32, size: u32) -> u32 {
    let index = ftol64_low((f64::from(value) + FLIP_INDEX_BIAS) as f32);
    if index < size {
        index
    } else {
        size.wrapping_sub(1)
    }
}

// IDA: 006D1960's tail: Map::SetTexture on map +54, or shader map +54 - 0x400, with texture `index`.
// Completion: 100% of the branches.
pub fn flip_controller_write(
    property: &mut TexturingProperty,
    affected_map: u32,
    textures: &[Option<Entity>],
    index: u32,
) -> TextureControllerWrite {
    let map = if let Some(Some(map)) = property.maps_1c.get_mut(affected_map as usize) {
        map
    } else {
        let shader_index = affected_map.wrapping_sub(0x400) as usize;
        let Some(maps) = &mut property.shader_maps_2c else {
            return TextureControllerWrite::Applied;
        };
        let Some(Some(map)) = maps.get_mut(shader_index) else {
            return TextureControllerWrite::Applied;
        };
        map
    };
    let Some(&texture) = textures.get(index as usize) else {
        return TextureControllerWrite::UnsupportedRange;
    };
    map.set_texture(texture);
    TextureControllerWrite::Applied
}

/// NiTextureTransformController's own stream fields, the bytes after the single-interp prefix.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextureTransformControllerFields {
    pub shader_map_raw_48: u8,
    pub map_index_4c: u32,
    pub operation_50: u32,
    pub bytes_consumed: usize,
}

// IDA: 006D6FE0 modern order after 006CE320: u8 shader-map flag (+48), u32 map index (+4C), u32 operation (+50).
// Completion: 100% of these widths.
// TODO: [ ] The < 0A010068 deferred float-data reference.
pub fn parse_texture_transform_controller_fields(
    bytes: &[u8],
) -> Result<TextureTransformControllerFields, GameError> {
    let mut reader = ByteReader::new(bytes);
    let shader_map_raw_48 = reader.u8(TRUNCATED)?;
    let map_index_4c = reader.u32(TRUNCATED)?;
    let operation_50 = reader.u32(TRUNCATED)?;
    Ok(TextureTransformControllerFields {
        shader_map_raw_48,
        map_index_4c,
        operation_50,
        bytes_consumed: reader.position(),
    })
}

// IDA: 006D6C80 resolving the +44 map from the target: shader maps bounds-checked; regular maps read without a check in the
// original, which the host stops at. `Ok(None)` is a null map.
// Completion: 100%.
pub fn texture_transform_controller_map(
    property: &mut TexturingProperty,
    shader_map: bool,
    map_index: u32,
) -> Result<Option<&mut TexturingMap>, TextureControllerWrite> {
    if shader_map {
        return Ok(property
            .shader_maps_2c
            .as_mut()
            .and_then(|maps| maps.get_mut(map_index as usize))
            .and_then(Option::as_mut));
    }
    match property.maps_1c.get_mut(map_index as usize) {
        Some(map) => Ok(map.as_mut()),
        None => Err(TextureControllerWrite::UnsupportedRange),
    }
}

// IDA: 006D6CE0's tail: a missing transform is created by 0072FF40 (defaults, then 0072FDF0); operation 0/1/2/3/4 sets
// translate u/v, rotate, scale u/v, through the dirty setters; others do nothing.
// Completion: 100%.
pub fn texture_transform_controller_write(map: &mut TexturingMap, operation: u32, value: f32) {
    let transform = map.transform_0c.get_or_insert_with(|| {
        // 006D6DDF 0072FF40(translate 0, rotate 0, scale 1, center 0.5, 0).
        let mut created = TextureTransform::default();
        created.update_matrix();
        created
    });
    match operation {
        0 => transform.set_translate([value, transform.translate_00[1]]),
        1 => transform.set_translate([transform.translate_00[0], value]),
        2 => transform.set_rotate(value),
        3 => transform.set_scale([value, transform.scale_0c[1]]),
        4 => transform.set_scale([transform.scale_0c[0], value]),
        _ => {}
    }
}

/// IDA: shared NiSingleInterpController state for controllers whose target is a NiTexturingProperty: 00715990 time cold, +3C
/// interpolator (a NiFloatInterpolator or NiBlendFloatInterpolator through vtable +0x5C), next and target from the plain 00715E70
/// link (vtable +20 -> 006CE1C0).
/// Completion: 100% of the host values.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct TextureController {
    pub time: ParticleTimeControllerCold,
    pub pending_interpolator_index: u32,
    pub interpolator_3c: Option<Entity>,
    pub links_resolved: bool,
}

impl Default for TextureController {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            time: ParticleTimeControllerCold::default(),
            pending_interpolator_index: NULL_REFERENCE,
            interpolator_3c: None,
            links_resolved: false,
        }
    }
}

impl TextureController {
    // IDA: 00715F40 scalar and deferred-index publication before 006CE320.
    // Completion: 100% of the supported host field order.
    pub const fn apply_loaded_prefix(&mut self, payload: &SingleInterpControllerPayload) {
        self.time.apply_loaded_payload(&payload.time);
        self.pending_interpolator_index = payload.interpolator_index;
    }

    // IDA: 00715E70: next (strong) first, then raw target.
    // Completion: 100% of the modern >= 0A010068 association.
    // TODO: [ ] Original intrusive refcount swap timing.
    pub fn link_modern(
        &mut self,
        next: Option<Entity>,
        target: Option<Entity>,
        stream_version: u32,
    ) -> Result<(), GameError> {
        if stream_version < 0x0A01_0068 {
            return Err(GameError::unclassified(
                0,
                "legacy texture controller link unsupported",
            ));
        }
        if next.is_some() != (self.time.pending_next_index != NULL_REFERENCE) {
            return Err(GameError::unclassified(
                0,
                "texture controller next index/owner mismatch",
            ));
        }
        if target.is_some() != (self.time.pending_target_index != NULL_REFERENCE) {
            return Err(GameError::unclassified(
                0,
                "texture controller target index/owner mismatch",
            ));
        }
        self.time.next_34 = next;
        self.time.next_linked = next.is_some();
        self.time.target_30 = target;
        self.time.target_linked = target.is_some();
        self.links_resolved = true;
        Ok(())
    }
}

// IDA: N/A a cold NiFlipController or NiTextureTransformController entity.
// Completion: 100%
pub fn spawn_texture_controller(world: &mut World, class: NiClass, state: impl Bundle) -> Entity {
    world
        .spawn((class, TextureController::default(), state))
        .id()
}

/// IDA: 006D21D0 NiFlipController factory: FormHeap 0x5C, 006D20C0: 006EC180 NiFloatInterpController base, vtable A7A33C, texture
/// NiTArray +40 (00523DE0 capacity 1, grow 1), current index +50, affected map +54, ID string +58.
/// Completion: 100% of the cold state, load, link and the 006D1960 Update.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct FlipController {
    pub textures_40: Vec<Option<Entity>>,
    pub current_index_50: u32,
    pub affected_map_54: u32,
}

/// A NiTexturingProperty map slot. Host substitution for the +44 cached Map*: the maps are values inside the property's component, so
/// the cache names the property and slot. It follows a later map replaced in that slot, where the pointer would not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextureMapSlot {
    pub property: Entity,
    pub shader_map: bool,
    pub map_index: u32,
}

/// IDA: 006D7320 NiTextureTransformController factory: FormHeap 0x58, 006D7120 (null target, null map, operation 0): +40 = 0, map +44 =
/// null, shader flag +48 = 0, map index +4C = 0, operation +50 = 0, ID string +54 = null.
/// Completion: 100% of the cold state, load, link and the 006D6CE0 Update.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextureTransformController {
    pub map_44: Option<TextureMapSlot>,
    pub shader_map_48: bool,
    pub map_index_4c: u32,
    pub operation_50: u32,
}

/// IDA: 006D1CD0's whole modern payload.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FlipControllerPayload {
    pub base: SingleInterpControllerPayload,
    pub affected_map_54: u32,
    pub texture_indices: Vec<u32>,
    pub bytes_consumed: usize,
}

// IDA: 006CE320, then 006D1CD0's own fields.
// Completion: 100% of the supported widths.
pub fn parse_flip_controller_payload(
    bytes: &[u8],
    stream_version: u32,
) -> Result<FlipControllerPayload, GameError> {
    let base = parse_single_interp_controller_payload(bytes, stream_version)?;
    let fields = parse_flip_controller_fields(&bytes[base.bytes_consumed..])?;
    Ok(FlipControllerPayload {
        base,
        affected_map_54: fields.affected_map_54,
        texture_indices: fields.texture_indices,
        bytes_consumed: base.bytes_consumed + fields.bytes_consumed,
    })
}

/// IDA: 006D6FE0's whole modern payload.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TextureTransformControllerPayload {
    pub base: SingleInterpControllerPayload,
    pub shader_map_raw_48: u8,
    pub map_index_4c: u32,
    pub operation_50: u32,
    pub bytes_consumed: usize,
}

// IDA: 006CE320, then 006D6FE0's own fields.
// Completion: 100% of the supported widths.
pub fn parse_texture_transform_controller_payload(
    bytes: &[u8],
    stream_version: u32,
) -> Result<TextureTransformControllerPayload, GameError> {
    let base = parse_single_interp_controller_payload(bytes, stream_version)?;
    let fields = parse_texture_transform_controller_fields(&bytes[base.bytes_consumed..])?;
    Ok(TextureTransformControllerPayload {
        base,
        shader_map_raw_48: fields.shader_map_raw_48,
        map_index_4c: fields.map_index_4c,
        operation_50: fields.operation_50,
        bytes_consumed: base.bytes_consumed + fields.bytes_consumed,
    })
}

// IDA: 006CE320's prefix and immediate +0x3C float interpolator on a cold texture controller. `Ok(false)` for another class. From the
// C++ stream graph's 006D21D0 and 006D7320 cases.
// Completion: 100% of the order: inherited fields, then the interpolator.
fn load_texture_controller_prefix(
    world: &mut World,
    controller: Entity,
    payload: &SingleInterpControllerPayload,
    table: &StreamObjectTable,
) -> Result<bool, GameError> {
    world
        .get_mut::<TextureController>(controller)
        .ok_or(GameError::unclassified(
            0,
            "texture controller load target missing",
        ))?
        .apply_loaded_prefix(payload);
    if payload.interpolator_index == NULL_REFERENCE {
        return Ok(true);
    }
    let entity = table
        .objects
        .get(payload.interpolator_index as usize)
        .ok_or(GameError::unclassified(
            0,
            "texture controller interpolator out of range",
        ))?
        .entity;
    if !world
        .get::<NiClass>(entity)
        .is_some_and(|class| class.is_blend_float_source())
    {
        return Ok(false);
    }
    world
        .get_mut::<TextureController>(controller)
        .expect("checked above")
        .interpolator_3c = Some(entity);
    Ok(true)
}

// IDA: the 006D1CD0 load on a cold NiFlipController.
// Completion: 100% of the modern order.
pub fn load_flip_controller(
    world: &mut World,
    controller: Entity,
    payload: &FlipControllerPayload,
    table: &StreamObjectTable,
) -> Result<bool, GameError> {
    if !load_texture_controller_prefix(world, controller, &payload.base, table)? {
        return Ok(false);
    }
    world
        .get_mut::<FlipController>(controller)
        .ok_or(GameError::unclassified(
            0,
            "NiFlipController load target missing",
        ))?
        .affected_map_54 = payload.affected_map_54;
    Ok(true)
}

// IDA: the 006D6FE0 load on a cold NiTextureTransformController.
// Completion: 100% of the modern order.
pub fn load_texture_transform_controller(
    world: &mut World,
    controller: Entity,
    payload: &TextureTransformControllerPayload,
    table: &StreamObjectTable,
) -> Result<bool, GameError> {
    if !load_texture_controller_prefix(world, controller, &payload.base, table)? {
        return Ok(false);
    }
    let mut state = world
        .get_mut::<TextureTransformController>(controller)
        .ok_or(GameError::unclassified(
            0,
            "NiTextureTransformController load target missing",
        ))?;
    state.shader_map_48 = payload.shader_map_raw_48 != 0;
    state.map_index_4c = payload.map_index_4c;
    state.operation_50 = payload.operation_50;
    Ok(true)
}

// IDA: 00715E70 modern link for the texture controllers: next first, then the raw NiTexturingProperty target. `Ok(None)` for an
// unsupported class.
// Completion: 100% of the supported owners.
fn resolve_texture_controller_links(
    world: &World,
    time: &ParticleTimeControllerPayload,
    table: &StreamObjectTable,
) -> Result<Option<ControllerLinks>, GameError> {
    let mut next = None;
    if time.next_index != NULL_REFERENCE {
        let entity = table
            .objects
            .get(time.next_index as usize)
            .ok_or(GameError::unclassified(
                0,
                "texture controller next out of range",
            ))?
            .entity;
        if !world
            .get::<NiClass>(entity)
            .is_some_and(|class| class.is_texture_controller())
        {
            return Ok(None);
        }
        next = Some(entity);
    }
    let mut target = None;
    if time.target_index != NULL_REFERENCE {
        let entity = table
            .objects
            .get(time.target_index as usize)
            .ok_or(GameError::unclassified(
                0,
                "texture controller target out of range",
            ))?
            .entity;
        if world.get::<NiClass>(entity) != Some(&NiClass::TexturingProperty) {
            return Ok(None);
        }
        target = Some(entity);
    }
    Ok(Some((next, target)))
}

// IDA: 00700050's +0x0C head for a NiTexturingProperty, for `link_texturing_property`. `Ok(None)` for another class; a null reference is
// `Some(None)`.
// Completion: 100% of the bound classes.
pub fn resolve_texture_controller_head(
    world: &World,
    controller_index: u32,
    table: &StreamObjectTable,
) -> Result<Option<Option<Entity>>, GameError> {
    if controller_index == NULL_REFERENCE {
        return Ok(Some(None));
    }
    let entity = table
        .objects
        .get(controller_index as usize)
        .ok_or(GameError::unclassified(
            0,
            "texturing controller out of range",
        ))?
        .entity;
    if !world
        .get::<NiClass>(entity)
        .is_some_and(|class| class.is_texture_controller())
    {
        return Ok(None);
    }
    Ok(Some(Some(entity)))
}

// IDA: 006CE1C0 -> 00715E70 on a NiTextureTransformController, which has no derived links. `Ok(false)` for an unsupported class.
// Completion: 100%
pub fn link_texture_transform_controller(
    world: &mut World,
    controller: Entity,
    time: &ParticleTimeControllerPayload,
    table: &StreamObjectTable,
    stream_version: u32,
) -> Result<bool, GameError> {
    let Some((next, target)) = resolve_texture_controller_links(world, time, table)? else {
        return Ok(false);
    };
    world
        .get_mut::<TextureController>(controller)
        .ok_or(GameError::unclassified(
            0,
            "texture controller link target missing",
        ))?
        .link_modern(next, target, stream_version)?;
    Ok(true)
}

// IDA: 006D1F60 after the base link: only when the target (+30) is nonnull, 00523B10 SetSize(count) and each resolved texture SetAt.
// With a null target the pushed 00712AE0 count is never consumed, desynchronising the stream's link queue: `false`, a host stop.
// Completion: 100%
pub fn link_flip_textures(
    world: &mut World,
    controller: Entity,
    resolved: Vec<Option<Entity>>,
) -> bool {
    let has_target = world
        .get::<TextureController>(controller)
        .is_some_and(|base| base.time.target_30.is_some());
    if !has_target {
        return false;
    }
    let Some(mut flip) = world.get_mut::<FlipController>(controller) else {
        return false;
    };
    flip.textures_40 = resolved;
    true
}

// IDA: the 00715E70 base link, then 006D1F60's 00712AE0 list, resolved before anything is published. `Ok(false)` for an unsupported
// class or a null target.
// Completion: 100% of the order for NiSourceTexture entries.
pub fn link_flip_controller(
    world: &mut World,
    controller: Entity,
    payload: &FlipControllerPayload,
    table: &StreamObjectTable,
    stream_version: u32,
) -> Result<bool, GameError> {
    let Some((next, target)) = resolve_texture_controller_links(world, &payload.base.time, table)?
    else {
        return Ok(false);
    };
    let mut base = world
        .get::<TextureController>(controller)
        .ok_or(GameError::unclassified(
            0,
            "texture controller link target missing",
        ))?
        .clone();
    base.link_modern(next, target, stream_version)?;
    let mut textures = Vec::with_capacity(payload.texture_indices.len());
    for &reference in &payload.texture_indices {
        if reference == NULL_REFERENCE {
            textures.push(None);
            continue;
        }
        let entity = table
            .objects
            .get(reference as usize)
            .ok_or(GameError::unclassified(
                0,
                "flip texture index out of range",
            ))?
            .entity;
        if world.get::<NiClass>(entity) != Some(&NiClass::SourceTexture) {
            return Ok(false);
        }
        textures.push(Some(entity));
    }
    if target.is_none() {
        return Ok(false);
    }
    *world
        .get_mut::<TextureController>(controller)
        .expect("checked above") = base;
    Ok(link_flip_textures(world, controller, textures))
}

/// The sampled value and the controller's +0x30 target.
type SampledFloat = (f32, Option<Entity>);

/// The C++ `sample_float`'s three outcomes.
enum FloatSample {
    Value(f32),
    Failed,
    Unsupported,
}

// IDA: the +0x5C float Update of 006D1960 and 006D6CE0 on a NiFloatInterpolator or NiBlendFloatInterpolator.
// Completion: 100%
fn sample_float(world: &mut World, interpolator: Entity, time: f32) -> FloatSample {
    let mut value = 0.0f32;
    match world.get::<NiClass>(interpolator).copied() {
        Some(NiClass::BlendFloatInterpolator) => {
            match sample_particle_blend_float(world, interpolator, time, &mut value) {
                ParticleBlendSampleStatus::Sampled => FloatSample::Value(value),
                ParticleBlendSampleStatus::Failed => FloatSample::Failed,
                _ => FloatSample::Unsupported,
            }
        }
        Some(NiClass::FloatInterpolator) => {
            if update_float_interpolator(world, interpolator, time, &mut value) {
                FloatSample::Value(value)
            } else {
                FloatSample::Failed
            }
        }
        _ => FloatSample::Unsupported,
    }
}

// IDA: the head 006D1960 and 006D6CE0 share, up to their interpolator's float: `Ok(Err(status))` when Update returns here.
// Completion: 100% of the verified branches.
fn begin_texture_controller(
    world: &mut World,
    controller: Entity,
    current_time: f32,
    map: ControllerTimeMap<'_>,
) -> Result<Result<SampledFloat, NodeControllerUpdateStatus>, GameError> {
    use NodeControllerUpdateStatus as Status;
    let mut base = world
        .get::<TextureController>(controller)
        .ok_or(GameError::unclassified(
            0,
            "texture controller update target missing",
        ))?
        .clone();
    let run = begin_controller_update(world, &mut base.time, current_time, map);
    world
        .get_mut::<TextureController>(controller)
        .expect("checked above")
        .time = base.time.clone();
    if !run? {
        return Ok(Err(Status::Skipped));
    }
    let Some(interpolator) = base.interpolator_3c else {
        return Ok(Err(Status::NoInterpolator));
    };
    Ok(
        match sample_float(world, interpolator, base.time.last_value_28) {
            FloatSample::Value(value) => Ok((value, base.time.target_30)),
            FloatSample::Failed => Err(Status::InterpolatorFailed),
            FloatSample::Unsupported => Err(Status::UnsupportedInterpolatorRange),
        },
    )
}

// IDA: 006D1960 vtable +0x54: shared head, interpolator +5C float Update at +0x28, index = low dword of fistp-truncate(value + 0.01
// (A3B150)), clamped to size - 1 (u32), then Map::SetTexture on map +54 or shader map +54 - 0x400.
// Completion: 100% of the verified branches.
// TODO: [ ] Validate against original runtime results.
pub fn update_flip_controller(
    world: &mut World,
    controller: Entity,
    current_time: f32,
    map: ControllerTimeMap<'_>,
) -> Result<NodeControllerUpdateStatus, GameError> {
    use NodeControllerUpdateStatus as Status;
    let (value, target) = match begin_texture_controller(world, controller, current_time, map)? {
        Ok(sampled) => sampled,
        Err(status) => return Ok(status),
    };
    let mut flip = world
        .get_mut::<FlipController>(controller)
        .ok_or(GameError::unclassified(
            0,
            "NiFlipController update target missing",
        ))?;
    let index = flip_controller_index(value, flip.textures_40.len() as u32);
    flip.current_index_50 = index;
    let affected = flip.affected_map_54;
    let textures = flip.textures_40.clone();
    let Some(mut property) = target.and_then(|target| world.get_mut::<TexturingProperty>(target))
    else {
        return Ok(Status::Applied);
    };
    Ok(
        match flip_controller_write(&mut property, affected, &textures, index) {
            TextureControllerWrite::Applied => Status::Applied,
            TextureControllerWrite::UnsupportedRange => Status::UnsupportedInterpolatorRange,
        },
    )
}

// IDA: 006D6CE0 vtable +0x54: shared head, interpolator +5C; 006D6C80 caches +44 (shader maps bounds-checked, regular maps not); a missing
// transform is created by 0072FF40; operation 0 / 1 / 2 / 3 / 4 sets translate u / v, rotate, scale u / v.
// Completion: 100% of the verified branches.
// TODO: [ ] Validate against original runtime results.
pub fn update_texture_transform_controller(
    world: &mut World,
    controller: Entity,
    current_time: f32,
    map: ControllerTimeMap<'_>,
) -> Result<NodeControllerUpdateStatus, GameError> {
    use NodeControllerUpdateStatus as Status;
    let (value, target) = match begin_texture_controller(world, controller, current_time, map)? {
        Ok(sampled) => sampled,
        Err(status) => return Ok(status),
    };
    let state =
        *world
            .get::<TextureTransformController>(controller)
            .ok_or(GameError::unclassified(
                0,
                "NiTextureTransformController update target missing",
            ))?;
    // 006D6C80: the cached +44 map wins; otherwise resolve from the target.
    let slot = if let Some(slot) = state.map_44 {
        slot
    } else {
        let Some(target) = target else {
            return Ok(Status::Applied);
        };
        let Some(mut property) = world.get_mut::<TexturingProperty>(target) else {
            return Ok(Status::Applied);
        };
        match texture_transform_controller_map(
            &mut property,
            state.shader_map_48,
            state.map_index_4c,
        ) {
            Err(_) => return Ok(Status::UnsupportedInterpolatorRange),
            Ok(None) => return Ok(Status::Applied),
            Ok(Some(_)) => {}
        }
        let slot = TextureMapSlot {
            property: target,
            shader_map: state.shader_map_48,
            map_index: state.map_index_4c,
        };
        world
            .get_mut::<TextureTransformController>(controller)
            .expect("checked above")
            .map_44 = Some(slot);
        slot
    };
    let Some(mut property) = world.get_mut::<TexturingProperty>(slot.property) else {
        return Ok(Status::Applied);
    };
    if let Ok(Some(found)) =
        texture_transform_controller_map(&mut property, slot.shader_map, slot.map_index)
    {
        texture_transform_controller_write(found, state.operation_50, value);
    }
    Ok(Status::Applied)
}

// IDA: 006D1BC0 NiFlipController_AttachTexture(texture, index): the +0x40 NiTArray grows to index + 1 (00523B10) and 005254D0 SetAt stores the refcounted texture. Completion: 100% of the host
// vector (capacity/count words not modelled).
// TODO: [ ] Record the +0x48..+0x4E NiTArray words if a consumer appears.
pub(crate) fn attach_flip_texture(flip: &mut FlipController, texture: Option<Entity>, index: u32) {
    if index as usize >= flip.textures_40.len() {
        flip.textures_40.resize(index as usize + 1, None);
    }
    flip.textures_40[index as usize] = texture;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 006D19DB at 24-bit: the `fadd qword` rounds to a float before the fistp. 4.99 (0x409FAE14) + 0.01 lies just below 5 and
    /// rounds to 5.0, so the index is 5; truncating the unrounded sum (the old form) gives 4.
    #[test]
    fn flip_index_rounds_before_truncating() {
        assert_eq!(flip_controller_index(f32::from_bits(0x409F_AE14), 9), 5);
    }

    const OBLIVION_VERSION: u32 = 0x1400_0004;

    fn u32_le(output: &mut Vec<u8>, value: u32) {
        output.extend(value.to_le_bytes());
    }

    fn f32_le(output: &mut Vec<u8>, value: f32) {
        output.extend(value.to_le_bytes());
    }

    fn owned(output: &mut Vec<u8>, text: &[u8]) {
        u32_le(output, text.len() as u32);
        output.extend(text);
    }

    /// IDA: 006FFCE0 modern NiObjectNET: name, no extra data, null controller.
    fn object_net(output: &mut Vec<u8>, name: &[u8]) {
        owned(output, name);
        u32_le(output, 0);
        u32_le(output, NULL_REFERENCE);
    }

    fn source_texture_bytes(external: u8, filename: &[u8], pixel_data: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        object_net(&mut bytes, b"texture");
        bytes.push(external);
        owned(&mut bytes, filename);
        u32_le(&mut bytes, pixel_data);
        u32_le(&mut bytes, 1); // pixel layout
        u32_le(&mut bytes, 0); // mip mapped
        u32_le(&mut bytes, 2); // alpha format
        bytes.push(0); // static
        bytes.push(0); // direct to renderer (>= 0A010067)
        bytes
    }

    /// IDA: 007023E0 modern LoadBinary read order and the 00701CD0 cold prefs.
    #[test]
    fn source_texture_payload() {
        let cold = SourceTexture::default();
        assert_eq!(cold.pixel_layout_18, 6);
        assert_eq!(cold.alpha_format_1c, 3);
        assert_eq!(cold.mip_mapped_20, 2);
        assert!(cold.static_40 && cold.direct_to_renderer_41 && !cold.renderer_data_24);
        let bytes = source_texture_bytes(1, b"tex.dds", NULL_REFERENCE);
        let payload = parse_source_texture_payload(&bytes, OBLIVION_VERSION).unwrap();
        assert_eq!(payload.bytes_consumed, bytes.len());
        assert_eq!(payload.net.name.as_deref(), Some(&b"texture"[..]));
        assert_eq!(payload.external_raw, 1);
        assert_eq!(payload.filename.as_deref(), Some(&b"tex.dds"[..]));
        assert_eq!(payload.pixel_data_index, NULL_REFERENCE);
        assert_eq!(payload.pixel_layout, 1);
        assert_eq!(payload.mip_mapped, 0);
        assert_eq!(payload.alpha_format, 2);
        assert_eq!(payload.static_raw, 0);
        assert_eq!(payload.direct_render_raw, Some(0));
        assert!(
            parse_source_texture_payload(&bytes[..bytes.len() - 1], OBLIVION_VERSION).is_err(),
            "missing direct-render byte (>= 0A010067)"
        );
    }

    #[derive(Default)]
    struct Fixture {
        registered: Vec<Vec<u8>>,
        shared: Option<Entity>,
    }

    impl SourceTextureLoadServices for Fixture {
        fn search_candidates(&mut self, file_path: &[u8]) -> Vec<Vec<u8>> {
            vec![[b"a\\", file_path].concat(), [b"b\\", file_path].concat()]
        }
        fn find_texture(&mut self, path: &[u8]) -> Option<Entity> {
            if path == b"b\\hit.dds" {
                self.shared
            } else {
                None
            }
        }
        fn register_texture(&mut self, path: &[u8], _texture: Entity) {
            self.registered.push(path.to_vec());
        }
        fn file_exists(&mut self, path: &[u8]) -> bool {
            path == b"b\\new.dds"
        }
    }

    /// IDA: 007023E0 texture sharing: internal names use the palette directly; external names search BSSearchPath
    /// candidates in the palette, then by file existence, registering the first existing candidate; a palette hit
    /// replaces the stream slot (00712980). Format prefs land on this object.
    #[test]
    fn source_texture_load() {
        let mut world = World::new();
        let this = world.spawn_empty().id();
        let mut services = Fixture::default();
        let parse = |external, name: &[u8]| {
            parse_source_texture_payload(
                &source_texture_bytes(external, name, NULL_REFERENCE),
                OBLIVION_VERSION,
            )
            .unwrap()
        };

        let mut internal = SourceTexture::default();
        let internal_payload = parse(0, b"int.dds");
        let result = internal.load(this, &internal_payload, Some(&mut services));
        assert_eq!(result.status, SourceTextureLoadStatus::Loaded);
        assert_eq!(result.replacement, None);
        assert_eq!(services.registered, [b"int.dds".to_vec()]);
        assert_eq!(
            internal.platform_filename_38.as_deref(),
            Some(&b"int.dds"[..])
        );
        assert_eq!(internal.pixel_layout_18, 1);
        assert_eq!(internal.alpha_format_1c, 2);
        assert!(!internal.static_40 && !internal.direct_to_renderer_41);

        let mut created = SourceTexture::default();
        let result = created.load(this, &parse(1, b"new.dds"), Some(&mut services));
        assert_eq!(result.status, SourceTextureLoadStatus::Loaded);
        assert_eq!(result.replacement, None);
        assert_eq!(services.registered.last().unwrap(), b"b\\new.dds");
        assert_eq!(
            created.platform_filename_38.as_deref(),
            Some(&b"b\\new.dds"[..])
        );

        let shared = world.spawn_empty().id();
        services.shared = Some(shared);
        let mut duplicate = SourceTexture::default();
        let result = duplicate.load(this, &parse(1, b"hit.dds"), Some(&mut services));
        assert_eq!(result.status, SourceTextureLoadStatus::Loaded);
        assert_eq!(result.replacement, Some(shared));
        assert_eq!(services.registered.len(), 2);
        assert_eq!(
            duplicate.platform_filename_38.as_deref(),
            Some(&b"b\\hit.dds"[..])
        );

        let mut orphan = SourceTexture::default();
        assert_eq!(
            orphan.load(this, &internal_payload, None).status,
            SourceTextureLoadStatus::Unsupported,
            "missing palette services"
        );
        let long_payload = parse(0, &[b'x'; 0x104]);
        assert_eq!(
            orphan.load(this, &long_payload, Some(&mut services)).status,
            SourceTextureLoadStatus::Unsupported,
            "a filename past the 0x104 path buffer"
        );
    }

    /// IDA: 00702260 link and 007203F0 PostLoad (B256CC precache gate, renderer data, B3F958 static pixel-data release).
    #[test]
    fn source_texture_link_and_post_load() {
        let mut world = World::new();
        let table = StreamObjectTable::default();
        let target = world.spawn(SourceTexture::default()).id();
        let with_pixels =
            parse_source_texture_payload(&source_texture_bytes(0, b"x.dds", 3), OBLIVION_VERSION)
                .unwrap();
        assert_eq!(
            link_source_texture(&mut world, target, &with_pixels, &table),
            Ok(false),
            "a NiPixelData link"
        );
        let plain = parse_source_texture_payload(
            &source_texture_bytes(0, b"x.dds", NULL_REFERENCE),
            OBLIVION_VERSION,
        )
        .unwrap();
        assert_eq!(
            link_source_texture(&mut world, target, &plain, &table),
            Ok(true)
        );
        let linked = world.get::<SourceTexture>(target).unwrap();
        assert!(linked.links_resolved && !linked.pixel_data_3c);
        assert_eq!(
            world.get::<NiObjectNet>(target).unwrap().name.as_deref(),
            Some(&b"texture"[..])
        );

        let mut texture = world.get::<SourceTexture>(target).unwrap().clone();
        let mut calls = 0;
        texture.pixel_data_3c = true;
        texture.static_40 = true;
        texture.post_load(
            false,
            true,
            Some(&mut |_: &mut SourceTexture| {
                calls += 1;
                true
            }),
        );
        assert_eq!(calls, 0, "disabled precache");
        assert!(!texture.renderer_data_24);
        texture.post_load(
            true,
            true,
            Some(&mut |_: &mut SourceTexture| {
                calls += 1;
                false
            }),
        );
        assert_eq!(calls, 1);
        assert!(
            !texture.renderer_data_24 && texture.pixel_data_3c,
            "failed renderer creation changes nothing"
        );
        texture.post_load(
            true,
            true,
            Some(&mut |_: &mut SourceTexture| {
                calls += 1;
                true
            }),
        );
        assert_eq!(calls, 2);
        assert!(
            texture.renderer_data_24 && !texture.pixel_data_3c,
            "static pixel data is released after renderer creation"
        );
    }

    /// IDA: 0072FDF0 UpdateMatrix (methods 0 and 1 only, dirty always cleared) and the 006D6A40/006D6A90/006D6AD0
    /// dirty-tracking setters. Exact values at rotation 0.
    #[test]
    fn texture_transform() {
        let mut transform = TextureTransform::default();
        assert_eq!(transform.scale_0c, [1.0, 1.0]);
        assert_eq!(transform.center_14, [0.5, 0.5]);
        assert!(transform.dirty_1c);
        assert!(transform.matrix_20.is_none());
        transform.translate_00 = [0.25, 0.5];
        transform.scale_0c = [2.0, 3.0];
        transform.update_matrix();
        assert!(!transform.dirty_1c);
        assert_eq!(
            transform.matrix_20.unwrap(),
            [2.0, -0.0, 0.25, 0.0, 3.0, 0.5, 0.0, 0.0, 1.0]
        );
        transform.method_44 = 1;
        transform.update_matrix();
        assert_eq!(
            transform.matrix_20.unwrap(),
            [2.0, 0.0, -1.0, -0.0, 3.0, 0.5, 0.0, 0.0, 1.0]
        );
        transform.method_44 = 2;
        transform.dirty_1c = true;
        transform.update_matrix();
        assert!(!transform.dirty_1c);
        assert_eq!(
            transform.matrix_20.unwrap()[2],
            -1.0,
            "an unsupported method leaves the matrix"
        );

        transform.set_translate([0.25, 0.5]);
        transform.set_rotate(0.0);
        transform.set_scale([2.0, 3.0]);
        assert!(!transform.dirty_1c, "equal setter values");
        transform.set_rotate(1.0);
        assert!(transform.dirty_1c, "a changed rotation");
    }

    fn texturing_property_bytes() -> Vec<u8> {
        let mut bytes = Vec::new();
        object_net(&mut bytes, b"texturing");
        u32_le(&mut bytes, 3); // apply mode (< 14010002)
        u32_le(&mut bytes, 7);
        // Map 0: texture 0, clamp 3, filter 2, uv 1, no transform.
        bytes.push(1);
        for value in [0, 3, 2, 1] {
            u32_le(&mut bytes, value);
        }
        bytes.push(0);
        bytes.extend([0; 4]); // maps 1..5 absent
        // Map 5: bump map with a transform and bump fields.
        bytes.push(1);
        for value in [NULL_REFERENCE, 0, 0, 0] {
            u32_le(&mut bytes, value);
        }
        bytes.push(1);
        for value in [0.1, 0.2, 3.0, 4.0, 0.5] {
            f32_le(&mut bytes, value);
        }
        u32_le(&mut bytes, 1); // method
        f32_le(&mut bytes, 0.5);
        f32_le(&mut bytes, 0.5);
        for value in [2.0, 0.25, 1.0, 0.0, 0.0, 1.0] {
            f32_le(&mut bytes, value);
        }
        // Map 6: decal, texture 0.
        bytes.push(1);
        for value in [0, 0, 0, 0] {
            u32_le(&mut bytes, value);
        }
        bytes.push(0);
        // One shader map with ID 42.
        u32_le(&mut bytes, 1);
        bytes.push(1);
        for value in [NULL_REFERENCE, 0, 0, 0] {
            u32_le(&mut bytes, value);
        }
        bytes.push(0);
        u32_le(&mut bytes, 42);
        bytes
    }

    /// IDA: 00704AD0 LoadBinary (apply mode, clamp/filter/uv flag packing, bump fields, shader maps, multitexture bit)
    /// and 00703EC0 link (textures, decal count, single link).
    #[test]
    fn texturing_property() {
        let mut property = TexturingProperty::default();
        assert_eq!(property.flags_18, 0x0004);
        assert_eq!(property.maps_1c.len(), 7);
        assert!(property.shader_maps_2c.is_none());
        let bytes = texturing_property_bytes();
        let payload =
            parse_texturing_property_payload(&bytes, OBLIVION_VERSION, property.flags_18).unwrap();
        assert_eq!(payload.maps.len(), 7);
        assert_eq!(payload.shader_maps.len(), 1);
        let map0 = payload.maps[0].unwrap();
        let map5 = payload.maps[5].unwrap();
        let shader0 = payload.shader_maps[0].unwrap();
        assert_eq!(payload.bytes_consumed, bytes.len());
        assert_eq!(payload.flags, 0x0006);
        assert_eq!(payload.map_count, 7);
        assert!(payload.maps[1].is_none());
        assert_eq!(map0.flags, 0x3201);
        let transform = map5.transform.unwrap();
        assert_eq!(transform.method_44, 1);
        assert_eq!(transform.rotate_08, 0.5);
        assert_eq!(map5.luma_scale, 2.0);
        assert_eq!(payload.shader_map_count, Some(1));
        assert_eq!(shader0.shader_id, 42);

        property.apply_loaded_payload(&payload);
        assert_eq!(property.flags_18, 0x0007);
        assert_eq!(property.maps_1c[0].unwrap().kind, TexturingMapKind::Map);
        let bump = property.maps_1c[5].unwrap();
        assert_eq!(bump.kind, TexturingMapKind::Bump);
        assert_eq!(bump.bump_matrix_18, [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(
            property.shader_maps_2c.as_ref().unwrap()[0]
                .unwrap()
                .shader_id_10,
            42
        );

        let mut world = World::new();
        let target = world.spawn(property).id();
        let texture = world.spawn(NiClass::SourceTexture).id();
        let mut table = StreamObjectTable::default();
        table.push("NiSourceTexture", 0x0070_21F0, texture);
        assert_eq!(
            link_texturing_property(&mut world, target, &payload, &table, None),
            Ok(true)
        );
        let linked = world.get::<TexturingProperty>(target).unwrap();
        assert_eq!(linked.maps_1c[0].unwrap().texture_08, Some(texture));
        assert_eq!(linked.maps_1c[5].unwrap().texture_08, None);
        assert_eq!(linked.maps_1c[6].unwrap().texture_08, Some(texture));
        assert_eq!(linked.flags_18, 0x0017);
        assert!(linked.links_resolved);
        assert_eq!(
            link_texturing_property(&mut world, target, &payload, &table, None),
            Ok(false),
            "a second link pass"
        );
    }

    /// IDA: 006D1CD0 load and 006D1960: index = low dword of trunc(value + 0.01), clamped to size - 1, shader maps
    /// addressed as map index - 0x400. The interpolator sample is the C++ test's scalar value.
    #[test]
    fn flip_controller() {
        let mut bytes = Vec::new();
        u32_le(&mut bytes, 0);
        u32_le(&mut bytes, 3);
        for index in [4, 5, NULL_REFERENCE] {
            u32_le(&mut bytes, index);
        }
        let fields = parse_flip_controller_fields(&bytes).unwrap();
        assert_eq!(fields.bytes_consumed, bytes.len());
        assert_eq!(fields.affected_map_54, 0);
        assert_eq!(fields.texture_indices, [4, 5, NULL_REFERENCE]);

        let mut world = World::new();
        let textures: Vec<Option<Entity>> =
            (0..3).map(|_| Some(world.spawn_empty().id())).collect();
        let mut property = TexturingProperty::default();
        property.maps_1c[0] = Some(TexturingMap::default());
        property.shader_maps_2c = Some(vec![Some(TexturingMap::default())]);

        let step = |property: &mut TexturingProperty, value, affected| {
            let index = flip_controller_index(value, 3);
            assert_eq!(
                flip_controller_write(property, affected, &textures, index),
                TextureControllerWrite::Applied
            );
            index
        };
        assert_eq!(step(&mut property, 1.995, 0), 2, "the index bias");
        assert_eq!(property.maps_1c[0].unwrap().texture_08, textures[2]);
        assert_eq!(step(&mut property, 1.98, 0), 1, "truncation");
        assert_eq!(property.maps_1c[0].unwrap().texture_08, textures[1]);
        assert_eq!(step(&mut property, 9.0, 0x400), 2, "the clamp");
        assert_eq!(
            property.shader_maps_2c.as_ref().unwrap()[0]
                .unwrap()
                .texture_08,
            textures[2],
            "shader-map addressing"
        );
    }

    /// IDA: 006D6FE0 load and 006D6CE0: transform creation through 0072FF40 and operations 0-4 through the dirty setters.
    #[test]
    fn texture_transform_controller() {
        let mut bytes = vec![0];
        u32_le(&mut bytes, 0);
        u32_le(&mut bytes, 2);
        let fields = parse_texture_transform_controller_fields(&bytes).unwrap();
        assert_eq!(fields.bytes_consumed, bytes.len());
        assert_eq!(fields.shader_map_raw_48, 0);
        assert_eq!(fields.map_index_4c, 0);
        assert_eq!(fields.operation_50, 2);

        let mut property = TexturingProperty::default();
        property.maps_1c[0] = Some(TexturingMap::default());
        let map = texture_transform_controller_map(&mut property, false, 0)
            .unwrap()
            .unwrap();
        texture_transform_controller_write(map, 0, 0.75);
        let created = map.transform_0c.unwrap();
        assert_eq!(created.translate_00[0], 0.75);
        assert!(created.dirty_1c);
        for operation in 1..=4 {
            texture_transform_controller_write(map, operation, 0.75);
            let transform = map.transform_0c.unwrap();
            let written = match operation {
                1 => transform.translate_00[1],
                2 => transform.rotate_08,
                3 => transform.scale_0c[0],
                _ => transform.scale_0c[1],
            };
            assert_eq!(written, 0.75, "operation {operation}");
        }
        assert_eq!(
            texture_transform_controller_map(&mut property, false, 9).unwrap_err(),
            TextureControllerWrite::UnsupportedRange,
            "an unchecked regular-map index stops"
        );
    }

    fn controller_prefix(output: &mut Vec<u8>, next: u32, interpolator: u32) {
        u32_le(output, next);
        output.extend(0x0008u16.to_le_bytes());
        for value in [1.0f32, 0.0, 0.0, 2.0] {
            f32_le(output, value);
        }
        u32_le(output, NULL_REFERENCE);
        u32_le(output, interpolator);
    }

    fn identity(_: &mut crate::gamebryo::controller_time::ControllerTimeFields, time: f32) -> f32 {
        time
    }

    fn float_source(world: &mut World, value: f32) -> Entity {
        let mut state = crate::gamebryo::animation::FloatInterpolator::default();
        state.load_scalar_and_data(value, None);
        world.spawn((NiClass::FloatInterpolator, state)).id()
    }

    fn set_float(world: &mut World, source: Entity, value: f32) {
        world
            .get_mut::<crate::gamebryo::animation::FloatInterpolator>(source)
            .unwrap()
            .load_scalar_and_data(value, None);
    }

    fn set_time_flags(world: &mut World, controller: Entity) {
        world
            .get_mut::<TextureController>(controller)
            .unwrap()
            .time
            .fields
            .flags = 0x0008;
    }

    /// Ported from `texture_tests`' `verify_flip_controller`. IDA: 006D1CD0 load, 006D1F60 texture link (needs a target) and 006D1960
    /// Update.
    #[test]
    fn flip_controller_stream_object() {
        let mut bytes = Vec::new();
        controller_prefix(&mut bytes, 1, 2);
        u32_le(&mut bytes, 0);
        u32_le(&mut bytes, 3);
        for index in [4, 5, NULL_REFERENCE] {
            u32_le(&mut bytes, index);
        }
        let payload = parse_flip_controller_payload(&bytes, OBLIVION_VERSION).unwrap();
        assert_eq!(payload.bytes_consumed, bytes.len());
        assert_eq!(payload.base.interpolator_index, 2);
        assert_eq!(payload.affected_map_54, 0);
        assert_eq!(payload.texture_indices, [4, 5, NULL_REFERENCE]);

        let mut world = World::new();
        let flip = spawn_texture_controller(
            &mut world,
            NiClass::FlipController,
            FlipController::default(),
        );
        assert!(
            !link_flip_textures(&mut world, flip, Vec::new()),
            "a texture list links only with a target"
        );
        let mut state = TexturingProperty::default();
        state.maps_1c[0] = Some(TexturingMap::default());
        state.shader_maps_2c = Some(vec![Some(TexturingMap::default())]);
        let property = world.spawn(state).id();
        world
            .get_mut::<TextureController>(flip)
            .unwrap()
            .time
            .target_30 = Some(property);
        let textures: Vec<Option<Entity>> =
            (0..3).map(|_| Some(world.spawn_empty().id())).collect();
        assert!(link_flip_textures(&mut world, flip, textures.clone()));

        let value = float_source(&mut world, 1.995);
        set_time_flags(&mut world, flip);
        world
            .get_mut::<TextureController>(flip)
            .unwrap()
            .interpolator_3c = Some(value);
        let mut evaluate = identity;
        let mut step = |world: &mut World, time| {
            update_flip_controller(world, flip, time, ControllerTimeMap::Custom(&mut evaluate))
        };
        assert_eq!(
            step(&mut world, 1.0),
            Ok(NodeControllerUpdateStatus::Applied)
        );
        assert_eq!(
            world.get::<FlipController>(flip).unwrap().current_index_50,
            2
        );
        let map_texture = |world: &World| {
            world.get::<TexturingProperty>(property).unwrap().maps_1c[0]
                .unwrap()
                .texture_08
        };
        assert_eq!(map_texture(&world), textures[2], "the index bias");
        set_float(&mut world, value, 1.98);
        assert_eq!(
            step(&mut world, 2.0),
            Ok(NodeControllerUpdateStatus::Applied)
        );
        assert_eq!(
            world.get::<FlipController>(flip).unwrap().current_index_50,
            1
        );
        assert_eq!(map_texture(&world), textures[1], "truncation");
        set_float(&mut world, value, 9.0);
        world
            .get_mut::<FlipController>(flip)
            .unwrap()
            .affected_map_54 = 0x400;
        assert_eq!(
            step(&mut world, 3.0),
            Ok(NodeControllerUpdateStatus::Applied)
        );
        assert_eq!(
            world.get::<FlipController>(flip).unwrap().current_index_50,
            2
        );
        assert_eq!(
            world
                .get::<TexturingProperty>(property)
                .unwrap()
                .shader_maps_2c
                .as_ref()
                .unwrap()[0]
                .unwrap()
                .texture_08,
            textures[2],
            "the clamp and shader-map addressing"
        );
    }

    /// Ported from `texture_tests`' `verify_texture_transform_controller`. IDA: 006D6FE0 load and 006D6CE0 Update: cached map, transform
    /// creation through 0072FF40, and operations 0-4 through the dirty setters.
    #[test]
    fn texture_transform_controller_stream_object() {
        let mut bytes = Vec::new();
        controller_prefix(&mut bytes, 1, 2);
        bytes.push(0);
        u32_le(&mut bytes, 0);
        u32_le(&mut bytes, 2);
        let payload = parse_texture_transform_controller_payload(&bytes, OBLIVION_VERSION).unwrap();
        assert_eq!(payload.bytes_consumed, bytes.len());
        assert_eq!(payload.shader_map_raw_48, 0);
        assert_eq!(payload.map_index_4c, 0);
        assert_eq!(payload.operation_50, 2);

        let mut world = World::new();
        let mut state = TexturingProperty::default();
        state.maps_1c[0] = Some(TexturingMap::default());
        let property = world.spawn(state).id();
        let value = float_source(&mut world, 0.75);
        let controller = spawn_texture_controller(
            &mut world,
            NiClass::TextureTransformController,
            TextureTransformController::default(),
        );
        set_time_flags(&mut world, controller);
        {
            let mut base = world.get_mut::<TextureController>(controller).unwrap();
            base.time.target_30 = Some(property);
            base.interpolator_3c = Some(value);
        }
        let mut evaluate = identity;
        assert_eq!(
            update_texture_transform_controller(
                &mut world,
                controller,
                1.0,
                ControllerTimeMap::Custom(&mut evaluate)
            ),
            Ok(NodeControllerUpdateStatus::Applied)
        );
        assert_eq!(
            world
                .get::<TextureTransformController>(controller)
                .unwrap()
                .map_44,
            Some(TextureMapSlot {
                property,
                shader_map: false,
                map_index: 0
            }),
            "the map is cached"
        );
        let transform = |world: &World| {
            world.get::<TexturingProperty>(property).unwrap().maps_1c[0]
                .unwrap()
                .transform_0c
                .unwrap()
        };
        assert_eq!(transform(&world).translate_00[0], 0.75);
        assert!(transform(&world).dirty_1c);
        for operation in 1..=4u32 {
            world
                .get_mut::<TextureTransformController>(controller)
                .unwrap()
                .operation_50 = operation;
            let _ = update_texture_transform_controller(
                &mut world,
                controller,
                1.0 + operation as f32,
                ControllerTimeMap::Custom(&mut evaluate),
            );
            let current = transform(&world);
            let written = match operation {
                1 => current.translate_00[1],
                2 => current.rotate_08,
                3 => current.scale_0c[0],
                _ => current.scale_0c[1],
            };
            assert_eq!(written, 0.75, "operation {operation}");
        }

        let out_of_range = spawn_texture_controller(
            &mut world,
            NiClass::TextureTransformController,
            TextureTransformController {
                map_index_4c: 9,
                ..TextureTransformController::default()
            },
        );
        set_time_flags(&mut world, out_of_range);
        {
            let mut base = world.get_mut::<TextureController>(out_of_range).unwrap();
            base.time.target_30 = Some(property);
            base.interpolator_3c = Some(value);
        }
        assert_eq!(
            update_texture_transform_controller(
                &mut world,
                out_of_range,
                1.0,
                ControllerTimeMap::Custom(&mut evaluate)
            ),
            Ok(NodeControllerUpdateStatus::UnsupportedInterpolatorRange),
            "an unchecked regular-map index stops"
        );
    }

    /// IDA: 00715E70 and 006D1F60 over a stream table: next first, the target's class, then the texture list.
    #[test]
    fn flip_controller_links_from_a_table() {
        let mut world = World::new();
        let property = world
            .spawn((NiClass::TexturingProperty, TexturingProperty::default()))
            .id();
        let texture = world.spawn(NiClass::SourceTexture).id();
        let flip = spawn_texture_controller(
            &mut world,
            NiClass::FlipController,
            FlipController::default(),
        );
        let next = spawn_texture_controller(
            &mut world,
            NiClass::TextureTransformController,
            TextureTransformController::default(),
        );
        let mut table = StreamObjectTable::default();
        for entity in [property, texture, flip, next] {
            table.push("Object", 0, entity);
        }
        let mut payload = FlipControllerPayload::default();
        payload.base.time.next_index = 3;
        payload.base.time.target_index = 0;
        payload.texture_indices = vec![1, NULL_REFERENCE];
        world
            .get_mut::<TextureController>(flip)
            .unwrap()
            .apply_loaded_prefix(&payload.base);
        assert_eq!(
            link_flip_controller(&mut world, flip, &payload, &table, OBLIVION_VERSION),
            Ok(true)
        );
        let base = world.get::<TextureController>(flip).unwrap();
        assert_eq!(base.time.next_34, Some(next));
        assert_eq!(base.time.target_30, Some(property));
        assert!(base.links_resolved);
        assert_eq!(
            world.get::<FlipController>(flip).unwrap().textures_40,
            [Some(texture), None]
        );
        assert_eq!(
            resolve_texture_controller_head(&world, 2, &table),
            Ok(Some(Some(flip)))
        );
        assert_eq!(resolve_texture_controller_head(&world, 0, &table), Ok(None));
    }
}
