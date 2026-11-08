//! Phase 07: the FontManager the InterfaceManager constructor creates (005802B0 -> 00576A30 FontManager_GetSingleton),
//! its five FontInfo objects
//! (00576420 -> 005757A0 FontInfo_ConstructAndLoad -> 005744E0 FontInfo_Load) and the font file and texture reads
//! through the Phase 03 archives
//! (00431130). Main-thread code after CreateDevice: every x87 result rounds to `f32`. Drawing the glyphs is M6's; this
//! owns the original load state.
//! Evidence: P07-RUN-001 (the F records at 005821B4).

use bevy_ecs::prelude::*;

use super::tile::{InterfaceHost, lift_text};
use crate::bs::effect_textures::effect_rgba32_format;
use crate::gamebryo::pixel_data::{construct_pixel_data, spawn_pixel_data};

/// IDA: 005744E0: the font data block is FormHeap(0x3928): +0x00 float line height, +0x04 texture count (max 8), +0x08
/// eight 0x24-byte texture entries
/// (+0x0C name, char[32]), +0x128 256 glyphs of 0x38 bytes.
pub const FONT_DATA_SIZE: usize = 0x3928;
pub const FONT_GLYPH_BASE: usize = 0x128;
pub const FONT_GLYPH_STRIDE: usize = 0x38;
pub const FONT_MAX_TEXTURES: usize = 8;

/// IDA: FontInfo (0x3C bytes; xOBSE GameAPI.h FontManager::FontInfo). Offsets in the names. +0x34 is written by neither
/// 005757A0 nor 005744E0 (P07-RUN-001
/// read FormHeap residue there), so it has no field. Completion: 100% of the fields 005757A0 and 005744E0 write.
/// TODO: [ ] +0x0C's slots are the NiPixelData entities, where the original holds NiTexturingProperty from 00704800. M6
/// builds those (host substitution).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FontInfo {
    /// +0x00: 005744E0 `add word ptr [esi], 1` on success or when already loaded or without a path (xOBSE
    /// bLoadedSuccessfully, a byte there).
    pub load_count_00: u16,
    /// +0x04: FormHeap copy of the path; null for a null or empty source.
    pub path_04: Option<Vec<u8>>,
    /// +0x08: a dword store of the ID, only when the source path is non-null.
    pub id_08: u32,
    /// +0x0C: eight slots (eh vector ctor).
    pub textures_0c: [Option<Entity>; FONT_MAX_TEXTURES],
    /// +0x2C: max over the glyphs of (lineHeight - g+0x34) + g+0x28.
    pub top_extent_2c: f32,
    /// +0x30: min over the glyphs of g+0x34 - g+0x28.
    pub bottom_extent_30: f32,
    /// +0x38: the font data block (empty when the .fnt did not open).
    pub font_data_38: Vec<u8>,
}

/// IDA: FontManager (0x18 bytes). A resource, present when 00B3A6BC is non-null. +0x00 five FontInfo* indexed by ID -
/// 1, +0x14 byte 0. Completion: 100%.
#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub struct FontManager {
    pub font_infos_00: [Option<FontInfo>; 5],
    pub unknown_14: u8,
}

/// IDA: N/A little-endian view of the 0x3928 block's floats. Beyond the block reads 0. Completion: 100%.
#[must_use]
pub fn font_data_float(data: &[u8], offset: usize) -> f32 {
    data.get(offset..offset + 4).map_or(0.0, |bytes| {
        f32::from_le_bytes(bytes.try_into().expect("four bytes"))
    })
}

/// IDA: N/A the +0x04 texture count. Completion: 100%.
#[must_use]
pub fn font_data_texture_count(data: &[u8]) -> u32 {
    data.get(4..8).map_or(0, |bytes| {
        u32::from_le_bytes(bytes.try_into().expect("four bytes"))
    })
}

/// IDA: N/A host helper. Completion: 100%.
fn store_float(data: &mut [u8], offset: usize, value: f32) {
    data[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

/// IDA: N/A host helper. Completion: 100%.
const fn glyph(index: usize, field: usize) -> usize {
    FONT_GLYPH_BASE + index * FONT_GLYPH_STRIDE + field
}

/// IDA: 005744E0 FontInfo_Load(this). x87 at 24-bit precision, so every intermediate is an `f32`. The original
/// overflows its 0x3928 allocation with a larger
/// file; the host grows the block (recorded). Completion: 100% of the control flow, reads, metric fold and fixups.
/// TODO: [ ] Compare the glyph fixup bytes with an original run (P07-RUN-002 FNV).
pub fn load_font_info(world: &mut World, info: &mut FontInfo, host: &mut dyn InterfaceHost) {
    // 0057451D..00574530: an already-loaded font or a null path only counts.
    let Some(path) = info.path_04.clone().filter(|_| info.load_count_00 == 0) else {
        info.load_count_00 += 1;
        return;
    };
    let path = lift_text(&path);
    // 00574536 00431130(path, 0, 0x2800, 0x80); +0x24 is the good byte.
    let mut file = host.open_font_file(&path);
    let Some(opened) = file.as_mut().and_then(|file| file.good().then_some(file)) else {
        // 00574A5F "Font file not found: %s. \n"; the file is deleted.
        host.warning(&format!("Font file not found: {path}. \n"));
        return;
    };
    // 0057455D FormHeap(0x3928) -> +0x38; vt+0x1C ComputeSize bytes read through 0042C8E0.
    let size = opened.size() as usize;
    info.font_data_38 = vec![0; FONT_DATA_SIZE.max(size)];
    let _ = opened.read(&mut info.font_data_38[..size]);
    // 00574582 vt+0 (1).
    drop(file);
    let data = &mut info.font_data_38;
    // 0057458C fldz: +0x2C = 0, +0x30 = 0, the running top = 0.
    info.top_extent_2c = 0.0;
    info.bottom_extent_30 = 0.0;
    let mut top = 0.0f32;
    let line = font_data_float(data, 0);
    for index in 0..256 {
        let height = font_data_float(data, glyph(index, 0x28));
        let bottom = font_data_float(data, glyph(index, 0x34));
        // (line - g+0x34) + g+0x28, each rounded to float; kept unless smaller.
        let above = line - bottom;
        let extent = above + height;
        info.top_extent_2c = if extent < info.top_extent_2c {
            info.top_extent_2c
        } else {
            extent
        };
        top = if height < top { top } else { height };
        // g+0x34 - g+0x28; the smaller value wins, an unordered one replaces.
        let below = bottom - height;
        info.bottom_extent_30 = if info.bottom_extent_30 < below {
            info.bottom_extent_30
        } else {
            below
        };
    }
    // 00574784..005747A1: swap the space glyph's +0x24 and +0x30.
    let space_24 = font_data_float(data, glyph(32, 0x24));
    let space_30 = font_data_float(data, glyph(32, 0x30));
    store_float(data, glyph(32, 0x24), space_30);
    store_float(data, glyph(32, 0x30), space_24);
    // 005747AA space +0x28 = top; 005747C2 +0x34 = +0x30 extent + top.
    store_float(data, glyph(32, 0x28), top);
    let bottom_line = info.bottom_extent_30 + top;
    store_float(data, glyph(32, 0x34), bottom_line);
    // 005747C8..0057480D glyph 127 takes glyph 124's +0x24..+0x34 in the order +0x24, +0x2C, +0x30, +0x28, +0x34.
    for field in [0x24, 0x2C, 0x30, 0x28, 0x34] {
        let value = font_data_float(data, glyph(124, field));
        store_float(data, glyph(127, field), value);
    }
    // 00574816..0057488C glyph 0: +0x24 = +0x30 = 0, +0x28 = top, +0x34 = extent + top, then +0x04..+0x20 = 0.
    store_float(data, glyph(0, 0x24), 0.0);
    store_float(data, glyph(0, 0x30), 0.0);
    store_float(data, glyph(0, 0x28), top);
    store_float(data, glyph(0, 0x34), bottom_line);
    for field in [0x04, 0x0C, 0x14, 0x1C, 0x08, 0x10, 0x18, 0x20] {
        store_float(data, glyph(0, field), 0.0);
    }
    // 00574892: more than eight textures warns and stops without counting.
    let count = font_data_texture_count(data) as i32;
    if count > FONT_MAX_TEXTURES as i32 {
        host.warning(&format!(
            "Too many font textures for {path}.\nMax textures is 8 yours has {count}.\n"
        ));
        return;
    }
    for slot in 0..count.max(0) as usize {
        // 005748C9 sprintf("%s\\%s.tex", "Data\\Fonts", +0x0C + 0x24 * i).
        let entry = 0x0C + 0x24 * slot;
        let name: Vec<u8> = info.font_data_38[entry..]
            .iter()
            .copied()
            .take_while(|&byte| byte != 0)
            .collect();
        let texture_path = format!("Data\\Fonts\\{}.tex", lift_text(&name));
        let mut texture_file = host.open_font_file(&texture_path);
        let Some(opened) = texture_file
            .as_mut()
            .and_then(|file| file.good().then_some(file))
        else {
            // 00574A3D "Font file not found: %S. \n": %S reads the narrow buffer as a wide string in the original
            // (garbled text); the host logs the name. No
            // count; the earlier slots stay.
            host.warning(&format!("Font file not found: {texture_path}. \n"));
            return;
        };
        // 0057492D read proc(&width, 8, {1}, 1).
        let mut extent = [0u8; 8];
        let _ = opened.read_components(&mut extent, Some(&[1]));
        let width = u32::from_le_bytes(extent[0..4].try_into().expect("four bytes"));
        let height = u32::from_le_bytes(extent[4..8].try_into().expect("four bytes"));
        // 0057495D NiPixelData(width, height, 00B25E00 RGBA32, 1, 1).
        let mut pixels = construct_pixel_data(width, height, &effect_rgba32_format(), 1, 1);
        // 00574999 read proc(pixels + offsets[0], 4 * width * height, {1}, 1).
        let bytes = (4u32.wrapping_mul(width).wrapping_mul(height)) as usize;
        let level = pixels.level_mut(0, 0);
        let length = bytes.min(level.len());
        let _ = opened.read_components(&mut level[..length], Some(&[1]));
        // 005749BB 00704800(pixels), 005749D5 SetBaseFilterMode(1), 005749E1 NiPointer assign into slot i, 005749EE the
        // file deleted.
        info.textures_0c[slot] = Some(spawn_pixel_data(world, pixels));
        drop(texture_file);
    }
    // 00574A11.
    info.load_count_00 += 1;
}

/// IDA: 005757A0 FontInfo_ConstructAndLoad(id, path, loadImmediately): slots null, +0x04 null, +0x00 word 0, +0x38
/// null, then the path copy (skipped for an empty
/// string), the +0x08 ID and the load, all only for a non-null path. Completion: 100% of the store order.
pub fn construct_font_info(
    world: &mut World,
    id: u32,
    path: Option<&[u8]>,
    load_immediately: bool,
    host: &mut dyn InterfaceHost,
) -> FontInfo {
    let mut info = FontInfo::default();
    if let Some(path) = path {
        if !path.is_empty() {
            info.path_04 = Some(path.to_vec());
        }
        info.id_08 = id;
        if load_immediately {
            load_font_info(world, &mut info, host);
        }
    }
    info
}

/// IDA: 00576420 FontManager_ctor: five FontInfo (0x3C) from the five g_fontPath* settings (sFontFile_1..5:Fonts), IDs
/// 1..5 in order, then +0x14 = 0. A null FormHeap
/// result stores null in the original; host allocation does not fail. Completion: 100%.
pub fn construct_font_manager(
    world: &mut World,
    paths: &[Option<Vec<u8>>; 5],
    host: &mut dyn InterfaceHost,
) -> FontManager {
    let mut manager = FontManager::default();
    for (index, path) in paths.iter().enumerate() {
        manager.font_infos_00[index] = Some(construct_font_info(
            world,
            index as u32 + 1,
            path.as_deref(),
            true,
            host,
        ));
    }
    manager.unknown_14 = 0;
    manager
}

/// IDA: 00576A30 FontManager_GetSingleton: 00B3A6BC, created on first use. Completion: 100%.
pub fn font_manager_get_singleton(
    world: &mut World,
    paths: &[Option<Vec<u8>>; 5],
    host: &mut dyn InterfaceHost,
) {
    if !world.contains_resource::<FontManager>() {
        let manager = construct_font_manager(world, paths, host);
        world.insert_resource(manager);
    }
}
