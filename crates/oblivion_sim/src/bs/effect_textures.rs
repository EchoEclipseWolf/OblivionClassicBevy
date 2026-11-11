//! 007B8990, the BSShaderManager effect textures created inside 007BA0F0 (007BA208): the procedural pixel generators, each traced
//! from the x87 sequence (the six cube-normalization faces, the flat 16x16 and 4x4 textures and the 128x128 radial falloff), the
//! NiScreenPolygon and the slot stores.
//!
//! x87 precision: 24-bit at every site in this file, because 007B8990 runs after CreateDevice (control word 007F). Every arithmetic
//! result is a float, so plain `f32` arithmetic in the original operand order is right. The byte conversions are fistp with the
//! rounding control set to chop (`or 0C00h`).

use bevy_ecs::prelude::*;
use oblivion_core::crt::c_prefix;
use oblivion_core::x87;

use crate::gamebryo::pixel_data::{
    PixelData, PixelFormat, construct_pixel_data, d3d_pixel_format_static, spawn_pixel_data,
};
use crate::gamebryo::renderer::original_rgb24_pixel_format;
use crate::gamebryo::scene_state::{ScenePropertyDefaults, construct_scene_property_state};
use crate::gamebryo::texture::SourceTexture;
use crate::gamebryo::texture_loading::{
    TextureLoadServices, create_cube_from_files, create_cube_from_pixel_data,
    create_texture_from_file, create_texture_from_pixel_data,
};

/// IDA: 00B256D0 g_faceGenTextureFormatPrefs {pixel layout 6, alpha 3, mip 2}.
const EFFECT_PREFS: [u32; 3] = [6, 3, 2];

// IDA: src__9 00B265E0, the NiPixelFormat static 007B8990 uses for A8R8G8B8: B, G, R, A bytes.
// Completion: 100%.
#[must_use]
pub fn effect_bgra32_format() -> PixelFormat {
    d3d_pixel_format_static(0x00B2_65E0).expect("a listed static")
}

// IDA: pixelFormat_ 00B25E48 (RGB24).
// Completion: 100%.
#[must_use]
pub const fn effect_rgb24_format() -> PixelFormat {
    original_rgb24_pixel_format()
}

// IDA: g_defaultDescriptorSource68 00B25E00 (RGBA32: R, G, B, A bytes).
// Completion: 100%.
#[must_use]
pub fn effect_rgba32_format() -> PixelFormat {
    d3d_pixel_format_static(0x00B2_5E00).expect("a listed static")
}

// IDA: T(v) at 007B8AE6..007B8B02 and its five siblings: fadd 1 (fld1), fmul 0.5 (dbl_A2FAA0), fmul 255 (dbl_A3DDD8), fistp under
// RC = chop; the low byte of the integer is stored.
// Completion: 100%.
fn normal_byte(value: f32) -> u8 {
    let shifted = value + 1.0;
    let half = shifted * 0.5;
    let scaled = half * 255.0;
    x87::ftol(scaled) as u8
}

// IDA: 007B8A17..007B952B the six 128x128 A8R8G8B8 cube-normalization faces (D3D face order +X, -X, +Y, -Y, +Z, -Z), each followed by
// +0x68 += 1. For row r and column c each face builds three axis values from 64, r and c, len = sqrt((a*a + b*b) + 4096) (exact
// integer sum, __CIsqrt), inv = 1 / len, and stores byte 0..2 = T(inv * axis) for B, G, R and byte 3 = 0. A face past 5 is the C++
// port's `invalid_argument`; the original has six inline loops.
// Completion: 100% of the traced operand order.
#[must_use]
pub fn generate_cube_normalization_face(face: u32) -> Option<PixelData> {
    if face > 5 {
        return None;
    }
    let mut data = construct_pixel_data(128, 128, &effect_bgra32_format(), 1, 1);
    let texels = data.level_mut(0, 0);
    for row in 0..128u32 {
        let r = row as f32;
        for column in 0..128u32 {
            let c = column as f32;
            let minus_row = 64.0 - r;
            let row_minus = r - 64.0;
            let minus_column = 64.0 - c;
            let column_minus = c - 64.0;
            // Per face the (R, G, B) axis values and the two non-constant terms.
            let (red, green, blue, first, second): (f32, f32, f32, f32, f32) = match face {
                // 007B8A17: (64, 64 - r, 64 - c)
                0 => (64.0, minus_row, minus_column, minus_row, minus_column),
                // 007B8BFF: (-64, 64 - r, c - 64)
                1 => (-64.0, minus_row, column_minus, minus_row, column_minus),
                // 007B8DDC: (c - 64, 64, r - 64)
                2 => (column_minus, 64.0, row_minus, column_minus, row_minus),
                // 007B8FC9: (c - 64, -64, 64 - r)
                3 => (column_minus, -64.0, minus_row, column_minus, minus_row),
                // 007B91A6: (c - 64, 64 - r, 64)
                4 => (column_minus, minus_row, 64.0, column_minus, minus_row),
                // 007B938F: (64 - c, 64 - r, -64)
                _ => (minus_column, minus_row, -64.0, minus_column, minus_row),
            };
            // The squared terms and 4096 are exact integers in float.
            let sum = first * first + second * second + 4096.0;
            let length = sum.sqrt(); // __CIsqrt, stored as float
            let inverse = 1.0 / length;
            let at = (row as usize * 128 + column as usize) * 4;
            texels[at + 3] = 0; // [esi+2]
            texels[at + 2] = normal_byte(inverse * red); // [esi+1]
            texels[at + 1] = normal_byte(inverse * green); // [esi]
            texels[at] = normal_byte(inverse * blue); // [esi-1]
        }
    }
    data.revision_68 = data.revision_68.wrapping_add(1); // +0x68 += 1
    Some(data)
}

// IDA: 007B95BA..007B962D 16x16 RGB24 (0x80, 0x80, 0xFF) -> 00B430DC; 007B969D..007B9711 16x16 RGBA32 (0x80, 0x80, 0xFF, 0x40) -> 00B430F0;
// 007B9781..007B97ED 16x16 RGB24 black -> 00B430E0; 007B9979..007B9A41 4x4 RGBA32 (0xE6, 0xD4, 0xB8, 0x40) when 00B42D40, 00B42F3E and
// render path >= 2, else (0, 0, 0, 0x40) -> 00B4311C; 007B9AB1..007B9B21 4x4 RGBA32 (0, 0, 0, 0xFF) -> 00B43120. Each ends with +0x68 += 1.
// Completion: 100%.
#[must_use]
pub fn generate_flat_pixel_data(size: u32, format: &PixelFormat, texel: &[u8]) -> PixelData {
    let mut data = construct_pixel_data(size, size, format, 1, 1);
    for chunk in data.level_mut(0, 0).chunks_exact_mut(texel.len()) {
        chunk.copy_from_slice(texel);
    }
    data.revision_68 = data.revision_68.wrapping_add(1);
    data
}

// IDA: 007B9E1B..007B9FDF the 128x128 RGB24 radial falloff -> 00B43110 (and the 00B430D4 alias): u/v = |x - 63.5| / 63.5 computed as
// 1 - x/63.5 below the center and (x - 63.5)/63.5 above it, d = u*u + v*v clamped to [0, 1], 1 - d when 00404F00(0) (g_renderPath) is 1,
// then floor(d * 255) truncated into R, G and B.
// Completion: 100% of the traced operand order.
#[must_use]
pub fn generate_radial_falloff(render_path: i32) -> PixelData {
    let mut data = construct_pixel_data(128, 128, &effect_rgb24_format(), 1, 1);
    let texels = data.level_mut(0, 0);
    const CENTER: f32 = 63.5; // dbl_A8F550
    for row in 0..128u32 {
        let r = row as f32;
        for column in 0..128u32 {
            let c = column as f32;
            // 007B9EAE..007B9ECF.
            let u = if c < CENTER {
                1.0 - c / CENTER
            } else {
                (c - CENTER) / CENTER
            };
            // 007B9ED3..007B9EEA.
            let v = if r < CENTER {
                1.0 - r / CENTER
            } else {
                (r - CENTER) / CENTER
            };
            // 007B9EEE..007B9EFE: u*u + v*v stored as float.
            let distance = u * u + v * v;
            // 007B9F06..007B9F47: clamped to [0, 1].
            let mut value = if distance > 1.0 { 1.0 } else { distance };
            if value < 0.0 {
                value = 0.0;
            }
            // 007B9F49..007B9F62: 1 - value when 00404F00(0) is 1.
            if render_path == 1 {
                value = 1.0 - value;
            }
            // 007B9F64..007B9FB8: float(value * 255), floor, chop, three bytes.
            let scaled = value * 255.0;
            let byte = x87::ftol(f64::from(scaled).floor() as f32) as u8;
            let at = (row as usize * 128 + column as usize) * 3;
            texels[at] = byte;
            texels[at + 1] = byte;
            texels[at + 2] = byte;
        }
    }
    data.revision_68 = data.revision_68.wrapping_add(1);
    data
}

/// IDA: 00738DC0 NiScreenPolygon(count, vertices, texCoords, colors) (0x1C, vtable A80F4C): NiObject base, +0x08 property state = null,
/// +0x0C u16 count, +0x10 count * 12 vertex bytes copied, +0x14 count * 8 texture coordinates or null, +0x18 count * 16 colors
/// (ArrayConstructorIterator with 0047EA50, then copied) or null, then a new NiPropertyState (007319E0) swapped into +0x08. An empty
/// `Vec` is a null array. Completion: 100% of the fields; the renderer-side screen polygon draw is a later consumer.
/// TODO: [ ] Port the NiScreenPolygon draw path (vt slots) with its consumer.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct ScreenPolygon {
    pub original_vtable: u32,
    pub property_state_08: Option<Entity>,
    pub vertex_count_0c: u16,
    pub vertices_10: Vec<[f32; 3]>,
    pub texture_coords_14: Vec<[f32; 2]>,
    pub colors_18: Vec<[f32; 4]>,
}

// IDA: 007A98E0 the full-screen polygon: x87 stores of -1.0 (flt_A30634), 0.1 (flt_A2FAAC) and 1.0 build (-1, -1, 0.1), (1, -1, 0.1), (1, 1,
// 0.1), (-1, 1, 0.1), then 00738DC0(4, vertices, null, null). 00738DFC..00738E39 +0x08 null, +0x0C count, +0x10 the vertex copy;
// 00738E78/00738ED9 null texture coordinates and colors; 00738EE0..00738F3B a new NiPropertyState swapped into +0x08.
// Completion: 100%.
pub fn create_fullscreen_polygon(world: &mut World, defaults: &ScenePropertyDefaults) -> Entity {
    let a = -1.0f32;
    let b = f32::from_bits(0x3DCC_CCCD);
    let state = construct_scene_property_state(world, defaults);
    world
        .spawn(ScreenPolygon {
            original_vtable: 0x00A8_0F4C,
            property_state_08: Some(state),
            vertex_count_0c: 4,
            vertices_10: vec![[a, a, b], [1.0, a, b], [1.0, 1.0, b], [a, 1.0, b]],
            texture_coords_14: Vec::new(),
            colors_18: Vec::new(),
        })
        .id()
}

/// IDA: the BSShaderManager effect texture globals 007B8990 writes (NiPointer slots: equal pointers skip the store; otherwise the old
/// reference is released, the slot stored and the new one referenced). A replaced entity stays spawned, since references are not
/// modelled, as with `NiProcessDefaults`.
#[derive(Resource, Clone, Debug, Default, PartialEq, Eq)]
pub struct EffectTextureGlobals {
    /// 00B430D4 (= 00B43110).
    pub radial_alias: Option<Entity>,
    /// 00B430DC.
    pub flat_normal: Option<Entity>,
    /// 00B430E0.
    pub black: Option<Entity>,
    /// 00B430E4.
    pub window_environment_02: Option<Entity>,
    /// 00B430E8.
    pub terrain_noise: Option<Entity>,
    /// 00B430EC.
    pub normalization_cube: Option<Entity>,
    /// 00B430F0.
    pub flat_normal_alpha: Option<Entity>,
    /// 00B430F4.
    pub reflection_cube: Option<Entity>,
    /// 00B430F8.
    pub aniso: Option<Entity>,
    /// 00B43100.
    pub window_environment_01: Option<Entity>,
    /// 00B43110.
    pub radial: Option<Entity>,
    /// 00B43114.
    pub eye_reflection_cube: Option<Entity>,
    /// 00B43118, a `ScreenPolygon`.
    pub screen_polygon: Option<Entity>,
    /// 00B4311C.
    pub local_map_tint: Option<Entity>,
    /// 00B43120.
    pub opaque_black: Option<Entity>,
    /// 00B43128.
    pub blood_decal: Option<Entity>,
    /// Host record of the slot stores, in order (addresses).
    pub store_trace: Vec<u32>,
}

impl EffectTextureGlobals {
    // IDA: 0076C910's argument in the pass tables: the three globals they name. Any other address is the C++ port's null.
    // Completion: 100%.
    #[must_use]
    pub const fn stage_texture(&self, address: u32) -> Option<Entity> {
        match address {
            0x00B4_30D4 => self.radial_alias,
            0x00B4_30EC => self.normalization_cube,
            0x00B4_3110 => self.radial,
            _ => None,
        }
    }
}

/// IDA: the inputs 007B8990 reads besides the texture services (g_renderer/00B43104 and the callbacks).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EffectTextureInputs {
    /// 00B42D40.
    pub local_map_shader: bool,
    /// 00B42F3E.
    pub image_space_effects: bool,
    /// 00B42F48 (also 00404F00(0)).
    pub render_path: i32,
    /// 007319E0 defaults; `None` skips the polygon, the C++ port's null registry.
    pub property_defaults: Option<ScenePropertyDefaults>,
}

// IDA: the NiPointer slot assignment idiom of 007B8990: equal pointers skip the store; otherwise release old, store, reference new.
// Completion: 100%.
fn assign_slot(
    trace: &mut Vec<u32>,
    slot: &mut Option<Entity>,
    address: u32,
    value: Option<Entity>,
) {
    if *slot == value {
        return;
    }
    *slot = value;
    trace.push(address);
}

// IDA: N/A a generator's result through 00701FC0. The original's reference count frees pixel data nothing holds; here that is a
// failed creation, or 007021B0 having cleared +0x3C.
fn texture_from_generated(
    world: &mut World,
    data: PixelData,
    services: &mut TextureLoadServices<'_>,
) -> Option<Entity> {
    let pixels = spawn_pixel_data(world, data);
    let texture = create_texture_from_pixel_data(world, Some(pixels), EFFECT_PREFS, services);
    let held = texture
        .and_then(|entity| world.get::<SourceTexture>(entity))
        .is_some_and(|texture| texture.app_pixel_data_3c == Some(pixels));
    if !held {
        world.despawn(pixels);
    }
    texture
}

// IDA: 007B8990 BSShaderManager_CreateEffectTextures, in order: 00B430EC cube normalization (six faces, 00720A30 with 00B43104 and the prefs
// 00B256D0 {6, 3, 2}); 00B430DC, 00B430F0, 00B430E0 (00701FC0); 00B43100 and 00B430E4 (007B8200(WindowEnvironmentMap01/02.dds, 1, 1)); 00B4311C
// and 00B43120 (00701FC0); 00B43114 and 00B430F4 (00720F80 with one name six times: eyereflection.dds, reflection.dds); 00B430F8 aniso.dds and
// 00B43128 blooddecal.dds (00701E00(path, prefs, 1)); 00B430E8 terrainnoise.dds only while the slot is null; 00B43110 radial (00701FC0) and its
// 00B430D4 alias; 00B43118 007A98E0. The ReturnZero_SharedVirtual asserts after null slots (lines 0x97C..0xA05 of BSShaderManager.cpp) have no
// effect.
// Completion: 100% of the order.
pub fn create_effect_textures(
    world: &mut World,
    globals: &mut EffectTextureGlobals,
    inputs: &EffectTextureInputs,
    services: &mut TextureLoadServices<'_>,
) {
    let trace = &mut globals.store_trace;
    // 007B89D0..007B9566 the six cube-normalization faces, 00720A30. The cube holds a merged copy, so nothing holds the six afterwards.
    let faces: [Entity; 6] = std::array::from_fn(|face| {
        let data = generate_cube_normalization_face(face as u32).expect("0..=5");
        spawn_pixel_data(world, data)
    });
    let cube = create_cube_from_pixel_data(world, faces.map(Some), EFFECT_PREFS, services);
    for face in faces {
        world.despawn(face);
    }
    assign_slot(trace, &mut globals.normalization_cube, 0x00B4_30EC, cube);
    // 007B95BA..007B985D the three 16x16 textures.
    let rgb24 = effect_rgb24_format();
    let rgba32 = effect_rgba32_format();
    let data = generate_flat_pixel_data(16, &rgb24, &[0x80, 0x80, 0xFF]);
    let texture = texture_from_generated(world, data, services);
    assign_slot(trace, &mut globals.flat_normal, 0x00B4_30DC, texture);
    let data = generate_flat_pixel_data(16, &rgba32, &[0x80, 0x80, 0xFF, 0x40]);
    let texture = texture_from_generated(world, data, services);
    assign_slot(trace, &mut globals.flat_normal_alpha, 0x00B4_30F0, texture);
    let data = generate_flat_pixel_data(16, &rgb24, &[0, 0, 0]);
    let texture = texture_from_generated(world, data, services);
    assign_slot(trace, &mut globals.black, 0x00B4_30E0, texture);
    // 007B985D..007B9979 the two window environment maps (007B8200(.., 1, 1)).
    let texture = load_shader_texture(
        world,
        b"Data\\Textures\\Effects\\WindowEnvironmentMap01.dds",
        true,
        true,
        services,
    );
    assign_slot(
        trace,
        &mut globals.window_environment_01,
        0x00B4_3100,
        texture,
    );
    let texture = load_shader_texture(
        world,
        b"Data\\Textures\\Effects\\WindowEnvironmentMap02.dds",
        true,
        true,
        services,
    );
    assign_slot(
        trace,
        &mut globals.window_environment_02,
        0x00B4_30E4,
        texture,
    );
    // 007B9979..007B9A41 the 4x4 tint: 00B42D40 && 00B42F3E && render path >= 2.
    let tinted = inputs.local_map_shader && inputs.image_space_effects && inputs.render_path >= 2;
    let tint: [u8; 4] = if tinted {
        [0xE6, 0xD4, 0xB8, 0x40]
    } else {
        [0, 0, 0, 0x40]
    };
    let data = generate_flat_pixel_data(4, &rgba32, &tint);
    let texture = texture_from_generated(world, data, services);
    assign_slot(trace, &mut globals.local_map_tint, 0x00B4_311C, texture);
    // 007B9AB1..007B9B21 4x4 opaque black.
    let data = generate_flat_pixel_data(4, &rgba32, &[0, 0, 0, 0xFF]);
    let texture = texture_from_generated(world, data, services);
    assign_slot(trace, &mut globals.opaque_black, 0x00B4_3120, texture);
    // 007B9B91..007B9CB1 the cube maps: sprintf("%s<name>", "Data\\Textures\\Effects\\") passed six times to 00720F80 with 00B43104 and the
    // prefs.
    let name: &[u8] = b"Data\\Textures\\Effects\\eyereflection.dds";
    let texture = create_cube_from_files(world, [name; 6], EFFECT_PREFS, services);
    assign_slot(
        trace,
        &mut globals.eye_reflection_cube,
        0x00B4_3114,
        texture,
    );
    let name: &[u8] = b"Data\\Textures\\Effects\\reflection.dds";
    let texture = create_cube_from_files(world, [name; 6], EFFECT_PREFS, services);
    assign_slot(trace, &mut globals.reflection_cube, 0x00B4_30F4, texture);
    // 007B9CC5..007B9D9C aniso.dds and blooddecal.dds: 00701E00(path, prefs, 1).
    let texture = create_texture_from_file(
        world,
        b"Data\\Textures\\Effects\\aniso.dds",
        EFFECT_PREFS,
        true,
        services,
    );
    assign_slot(trace, &mut globals.aniso, 0x00B4_30F8, texture);
    let texture = create_texture_from_file(
        world,
        b"Data\\Textures\\Effects\\blooddecal.dds",
        EFFECT_PREFS,
        true,
        services,
    );
    assign_slot(trace, &mut globals.blood_decal, 0x00B4_3128, texture);
    // 007B9DA3..007B9E11 terrainnoise.dds only while 00B430E8 is null.
    if globals.terrain_noise.is_none() {
        let texture = create_texture_from_file(
            world,
            b"Data\\Textures\\Effects\\terrainnoise.dds",
            EFFECT_PREFS,
            true,
            services,
        );
        assign_slot(trace, &mut globals.terrain_noise, 0x00B4_30E8, texture);
    }
    // 007B9E1B..007BA078 the radial falloff and the 00B430D4 alias.
    let data = generate_radial_falloff(inputs.render_path);
    let texture = texture_from_generated(world, data, services);
    assign_slot(trace, &mut globals.radial, 0x00B4_3110, texture);
    assign_slot(
        trace,
        &mut globals.radial_alias,
        0x00B4_30D4,
        globals.radial,
    );
    // 007BA07A..007BA0B6 007A98E0.
    if let Some(defaults) = inputs.property_defaults.as_ref() {
        let polygon = create_fullscreen_polygon(world, defaults);
        assign_slot(
            trace,
            &mut globals.screen_polygon,
            0x00B4_3118,
            Some(polygon),
        );
    }
}

// IDA: 007B8200 BSShaderManager texture loader(out, path, usePalette, requireMipmaps): the palette lookup (vt+4); with usePalette 0 and
// a palette the lookup result is final; otherwise a missing texture whose file exists is created (00701E00, prefs {6, 3, 2}, direct 1)
// and added (vt+8); with both flags a renderer data reporting at most one level (vt+0x10 00625D30) is rejected with "TEXTURE ERROR :
// texture does not contain mipmaps & will not be used : %s" (00A8F4A0; the 00A8F448 NOTASOURCETEXTURE form needs a non-NiSourceTexture,
// which the host does not have) through the warning callback (text, 0) and null returned.
// Completion: 100%.
pub fn load_shader_texture(
    world: &mut World,
    path: &[u8],
    use_palette: bool,
    require_mipmaps: bool,
    services: &mut TextureLoadServices<'_>,
) -> Option<Entity> {
    let name = c_prefix(path);
    // 007B825A..007B8286 palette vt+4(path, 0) (a reference is taken).
    let has_palette = services.palette_find.is_some();
    let mut texture = services.palette_find.as_mut().and_then(|find| find(name));
    // 007B8292..007B829E: usePalette 0 with a palette keeps the lookup.
    if !use_palette && has_palette {
        return texture;
    }
    // 007B82A4..007B82E5 a missing texture whose file exists (mode 0): 00701E00(path, {6, 3, 2}, 1), then palette vt+8 even for a null
    // result.
    if texture.is_none()
        && services
            .file_exists
            .as_mut()
            .is_some_and(|exists| exists(name))
    {
        texture = create_texture_from_file(world, name, [6, 3, 2], true, services);
        if has_palette && let Some(add) = services.palette_add.as_mut() {
            add(name, texture);
        }
    }
    // 007B82E7..007B8391 both flags: renderer data vt+0x10 (00625D30, +0x5C levels) <= 1 is rejected. Every host texture is an
    // NiSourceTexture, so the cast succeeds and the "%s" form prints the platform name (the CRT prints "(null)" for a null name); the
    // callback gets (text, 0).
    if use_palette && require_mipmaps {
        let source = texture.and_then(|entity| world.get::<SourceTexture>(entity));
        if let Some(source) = source
            && source
                .renderer_24
                .as_ref()
                .is_some_and(|renderer| renderer.levels_5c <= 1)
        {
            let mut text =
                b"TEXTURE ERROR : texture does not contain mipmaps & will not be used : ".to_vec();
            text.extend(source.platform_filename_38.as_deref().unwrap_or(b"(null)"));
            text.truncate(0x1FF); // The 0x200 snprintf buffer.
            if let Some(warning) = services.warning.as_mut() {
                warning(&text);
            }
            return None;
        }
    }
    texture
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bs::render_targets::tests::{pinned_renderer, rows};
    use crate::gamebryo::pixel_data::pixel_data_fnv1a;

    /// Ported from `verify_pixel_data`'s three uses of the `effect_*_format` helpers. `pixel_data_layout` in `pixel_data.rs` holds the
    /// rest, with the static 00B26598 where the C++ test has `effect_rgb24_format`, 00B25E48.
    #[test]
    fn pixel_data_effect_formats() {
        let bgra = construct_pixel_data(128, 128, &effect_bgra32_format(), 1, 1);
        assert_eq!(bgra.bytes_per_pixel_64, 4);
        assert_eq!(bgra.levels_60, 1);
        assert_eq!(bgra.offsets_5c, [0, 65536]);
        assert_eq!(bgra.allocation_stride, 65536);
        assert_eq!(bgra.revision_68, 1);
        assert_eq!(bgra.original_vtable, 0x00A7_E624);
        let rgb = construct_pixel_data(16, 16, &effect_rgb24_format(), 1, 1);
        assert_eq!(rgb.bytes_per_pixel_64, 3);
        assert_eq!(rgb.offsets_5c[1], 768);
        assert_eq!(rgb.allocation_stride, 768);
        let cube = construct_pixel_data(4, 4, &effect_rgba32_format(), 1, 6);
        assert_eq!(cube.faces_6c, 6);
        assert_eq!(cube.pixels_50.len(), 6 * 64);
        assert_eq!(cube.level(5, 0).len(), 64);
    }

    /// Ported from `verify_effect_pixel_generators`. IDA: 007B8990 procedural generators (centre texels by hand).
    #[test]
    fn effect_pixel_generators() {
        // Face +X centre (r = c = 64): inv = 1/64 -> R = T(1) = 255, G = B = T(0) = 127.
        let positive_x = generate_cube_normalization_face(0).unwrap();
        let centre = (64 * 128 + 64) * 4;
        let bytes = positive_x.level(0, 0);
        assert_eq!(bytes[centre..centre + 4], [127, 127, 255, 0]);
        assert_eq!(positive_x.revision_68, 2);
        let negative_z = generate_cube_normalization_face(5).unwrap();
        let z_bytes = negative_z.level(0, 0);
        assert_eq!(z_bytes[centre..centre + 3], [0, 127, 127]);
        let mut previous = 0;
        for face in 0..6 {
            let hash = pixel_data_fnv1a(&generate_cube_normalization_face(face).unwrap());
            assert_ne!(hash, previous, "cube faces must differ");
            previous = hash;
            println!("cube face {face} FNV {hash:08X}");
        }
        assert!(generate_cube_normalization_face(6).is_none());

        let flat = generate_flat_pixel_data(16, &effect_rgb24_format(), &[0x80, 0x80, 0xFF]);
        let flat_bytes = flat.level(0, 0);
        assert_eq!(flat_bytes.len(), 768);
        assert_eq!(flat_bytes[0], 0x80);
        assert_eq!(flat_bytes[2], 0xFF);
        assert_eq!(flat_bytes[767], 0xFF);
        assert_eq!(flat.revision_68, 2);

        let radial = generate_radial_falloff(5);
        let radial_bytes = radial.level(0, 0);
        assert_eq!(radial_bytes[0], 255);
        assert_eq!(radial_bytes[(63 * 128 + 63) * 3], 0);
        assert_eq!(radial_bytes[(127 * 128 + 127) * 3], 255);
        let inverted = generate_radial_falloff(1);
        assert_eq!(inverted.level(0, 0)[0], 0);
        assert_eq!(
            inverted.level(0, 0)[(63 * 128 + 63) * 3],
            254,
            "render path 1 must invert the falloff"
        );
        println!("radial FNV {:08X}", pixel_data_fnv1a(&radial));
    }

    /// Not checked by the C++ test, which prints its FNVs: the normalization cube against P05-RUN-002's memory, face by face, through
    /// the generator and the 007205A0 merge.
    #[test]
    fn normalization_cube_matches_p05_run_002() {
        let row = rows("CUBE")
            .into_iter()
            .find(|row| row[2] == "B430EC")
            .expect("the 00B430EC row");
        let wanted: Vec<u32> = row[8]
            .split(',')
            .map(|text| u32::from_str_radix(text, 16).unwrap())
            .collect();
        assert_eq!(wanted.len(), 6);
        let mut world = World::new();
        let faces: [Entity; 6] = std::array::from_fn(|face| {
            spawn_pixel_data(
                &mut world,
                generate_cube_normalization_face(face as u32).unwrap(),
            )
        });
        let mut services = TextureLoadServices::default();
        let cube =
            create_cube_from_pixel_data(&mut world, faces.map(Some), EFFECT_PREFS, &mut services)
                .unwrap();
        let cube = world.get::<SourceTexture>(cube).unwrap();
        assert_eq!(cube.original_vtable, 0x00A7_F644);
        let merged = world
            .get::<PixelData>(cube.app_pixel_data_3c.unwrap())
            .unwrap();
        assert_eq!(merged.original_vtable, 0x00A7_E624);
        assert_eq!(merged.levels_60.to_string(), row[3]);
        assert_eq!(merged.faces_6c.to_string(), row[4]);
        assert_eq!(merged.level(0, 0).len().to_string(), row[5]);
        assert_eq!(merged.widths_54[0].to_string(), row[6]);
        let format = merged.format_08;
        let dump = format!(
            "{:02x}{:02x}0000{:08x}{:08x}{:08x}{:08x}",
            format.flags_00,
            format.bits_per_pixel_01,
            format.format_04.swap_bytes(),
            format.tiling_08.swap_bytes(),
            format.renderer_hint_0c.swap_bytes(),
            format.extra_10.swap_bytes()
        );
        assert_eq!(dump, row[7]);
        for (face, wanted) in wanted.iter().enumerate() {
            let mut hash = 0x811C_9DC5u32;
            for &byte in merged.level(face as u32, 0) {
                hash ^= u32::from(byte);
                hash = hash.wrapping_mul(0x0100_0193);
            }
            assert_eq!(hash, *wanted, "face {face}");
        }
    }

    /// Ported from `verify_effect_texture_order`. IDA: 007B8990 store order without a renderer (00B256CC clear so the pixel-data textures
    /// survive) and 007A98E0.
    #[test]
    fn effect_texture_order() {
        let mut world = World::new();
        let mut globals = EffectTextureGlobals::default();
        let mut services = TextureLoadServices {
            create_renderer_data_on_create: false,
            file_exists: Some(Box::new(|_| false)),
            ..TextureLoadServices::default()
        };
        let mut inputs = EffectTextureInputs {
            local_map_shader: true,
            image_space_effects: true,
            render_path: 2,
            property_defaults: Some(ScenePropertyDefaults::default()),
        };
        create_effect_textures(&mut world, &mut globals, &inputs, &mut services);
        // 00B43100/00B430E4 stay null (no file): equal pointers skip the store.
        assert_eq!(
            globals.store_trace,
            [
                0x00B4_30EC,
                0x00B4_30DC,
                0x00B4_30F0,
                0x00B4_30E0,
                0x00B4_311C,
                0x00B4_3120,
                0x00B4_3114,
                0x00B4_30F4,
                0x00B4_30F8,
                0x00B4_3128,
                0x00B4_30E8,
                0x00B4_3110,
                0x00B4_30D4,
                0x00B4_3118
            ],
            "007B8990 store order differs"
        );
        assert!(globals.window_environment_01.is_none());
        assert!(globals.window_environment_02.is_none());
        assert_eq!(globals.radial_alias, globals.radial);
        let source = |world: &World, slot: Option<Entity>| -> SourceTexture {
            world.get::<SourceTexture>(slot.unwrap()).unwrap().clone()
        };
        assert_eq!(
            source(&world, globals.aniso).filename_34.as_deref(),
            Some(&b"Data\\Textures\\Effects\\aniso.dds"[..])
        );
        let tint_pixels = |world: &World, globals: &EffectTextureGlobals| -> Vec<u8> {
            let tint = source(world, globals.local_map_tint);
            world
                .get::<PixelData>(tint.app_pixel_data_3c.unwrap())
                .unwrap()
                .level(0, 0)
                .to_vec()
        };
        assert_eq!(
            tint_pixels(&world, &globals)[..4],
            [0xE6, 0xD4, 0xB8, 0x40],
            "the local-map tint must be E6 D4 B8 40"
        );
        let polygon = world
            .get::<ScreenPolygon>(globals.screen_polygon.unwrap())
            .unwrap();
        assert_eq!(polygon.original_vtable, 0x00A8_0F4C);
        assert_eq!(polygon.vertex_count_0c, 4);
        assert_eq!(polygon.vertices_10[0][..2], [-1.0, -1.0]);
        assert_eq!(polygon.vertices_10[0][2].to_bits(), 0x3DCC_CCCD);
        assert_eq!(polygon.vertices_10[2][..2], [1.0, 1.0]);
        assert_eq!(polygon.vertices_10[3][2].to_bits(), 0x3DCC_CCCD);
        assert!(polygon.texture_coords_14.is_empty());
        assert!(polygon.colors_18.is_empty());
        assert!(polygon.property_state_08.is_some());
        // A second pass keeps terrainnoise (only created while null).
        let noise = globals.terrain_noise;
        inputs.local_map_shader = false;
        create_effect_textures(&mut world, &mut globals, &inputs, &mut services);
        assert_eq!(globals.terrain_noise, noise, "00B430E8 is created once");
        assert_eq!(
            tint_pixels(&world, &globals)[0],
            0,
            "the tint falls back to 0 0 0 40"
        );
    }

    /// Not in the C++ test: the slot paths against P05-RUN-001's names. File-backed slots need `file_exists` to answer yes so that
    /// 007B8200 creates them; cube faces have no file behind them here, so those slots have no name to compare (the run prints `-`).
    #[test]
    fn effect_texture_names_match_p05_run_001() {
        let mut world = World::new();
        let mut globals = EffectTextureGlobals::default();
        let mut services = TextureLoadServices {
            create_renderer_data_on_create: false,
            file_exists: Some(Box::new(|path| {
                path.ends_with(b"Map01.dds") || path.ends_with(b"Map02.dds")
            })),
            ..TextureLoadServices::default()
        };
        create_effect_textures(
            &mut world,
            &mut globals,
            &EffectTextureInputs::default(),
            &mut services,
        );
        let names = rows("NAME");
        assert_eq!(names.len(), 5);
        for row in names {
            let slot = match row[2].as_str() {
                "B430E4" => globals.window_environment_02,
                "B430E8" => globals.terrain_noise,
                "B430F8" => globals.aniso,
                "B43100" => globals.window_environment_01,
                "B43128" => globals.blood_decal,
                other => panic!("no slot {other}"),
            };
            let texture = world.get::<SourceTexture>(slot.unwrap()).unwrap();
            assert_eq!(
                texture.platform_filename_38.as_deref(),
                Some(row[3].as_bytes()),
                "{row:?}"
            );
        }
    }

    /// Not in the C++ test, which has no renderer in its effect checks: the pixel-data slots through the pinned renderer against
    /// P05-RUN-002's textures. The file-backed slots need the shipped files: `tests/effect_textures_real.rs` and the startup window test.
    #[test]
    fn effect_textures_match_p05_run_002() {
        let mut creation = pinned_renderer();
        let render_path = 5;
        let mut world = World::new();
        let mut globals = EffectTextureGlobals::default();
        {
            let mut services = TextureLoadServices {
                renderer: Some(&mut creation),
                file_exists: Some(Box::new(|_| false)),
                ..TextureLoadServices::default()
            };
            create_effect_textures(
                &mut world,
                &mut globals,
                &EffectTextureInputs {
                    local_map_shader: true,
                    image_space_effects: true,
                    render_path,
                    property_defaults: Some(ScenePropertyDefaults::default()),
                },
                &mut services,
            );
        }
        let mut checked = 0;
        for row in rows("TEX") {
            let slot = match row[2].as_str() {
                "B430D4" => globals.radial_alias,
                "B430DC" => globals.flat_normal,
                "B430E0" => globals.black,
                "B430EC" => globals.normalization_cube,
                "B430F0" => globals.flat_normal_alpha,
                "B43110" => globals.radial,
                "B4311C" => globals.local_map_tint,
                "B43120" => globals.opaque_black,
                _ => continue,
            };
            let texture = world.get::<SourceTexture>(slot.unwrap()).unwrap();
            let data = texture.renderer_24.as_ref().expect("renderer data");
            assert_eq!(format!("{:X}", data.original_vtable), row[3], "{row:?}");
            assert_eq!(data.width_54.to_string(), row[4], "{row:?}");
            assert_eq!(data.height_58.to_string(), row[5], "{row:?}");
            assert_eq!(data.levels_5c.to_string(), row[6], "{row:?}");
            assert_eq!(data.texture_bytes_60.to_string(), row[7], "{row:?}");
            checked += 1;
        }
        assert_eq!(checked, 8);
    }
}
