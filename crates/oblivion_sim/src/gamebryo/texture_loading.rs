//! The device-independent half of the NiSourceTexture creation paths the 007B8990 effect textures reach:
//! NiSourceTexture_CreateFromPixelData (00701FC0), NiSourceTexture_CreateFromFile (00701E00) and its renderer data
//! (00701C00 -> renderer vt+0x104 00763560 -> 00761270), the NiSourceCubeMap from six pixel data objects
//! (00720A30/007205A0) or six file names (00720F80/00720B40 with the base NiSearchPath walk), the direct load's checks
//! and counters (00760DA0, 007609C0), NiDevImageConverter_ReadImageFile (0071E690), the NiDDSReader (007363F0 header,
//! 00737330 ReadFile) and the BSShaderManager texture loader (007B8200).
//!
//! Everything from Direct3D 9 down is the device boundary, `TextureRendererBoundary`. No Direct3D is called. `RendererCreation` answers it
//! as data, D3DX's answers included (`direct_image_info`, `direct_texture_levels`), and Bevy's renderer will sit behind it as a recorded host
//! substitution.

use bevy_ecs::prelude::*;
use oblivion_core::crt::{c_prefix, stricmp};

use super::object_net::NiObjectNet;
use super::pixel_data::{
    PixelData, PixelFormat, construct_pixel_data, d3d_pixel_format_static, pixel_format_from_d3d,
    spawn_pixel_data,
};
use super::stream_objects::NiClass;
use super::texture::{SourceTexture, TEXTURE_PATH_BUFFER};
use crate::bs::file::GameFile;
use crate::gamebryo::file::FileCore;

const FOUR_CC_DXT1: u32 = 0x3154_5844;
const FOUR_CC_DXT3: u32 = 0x3354_5844;
const FOUR_CC_DXT5: u32 = 0x3554_5844;
const FOUR_CC_DXT2: u32 = 0x3254_5844;
const FOUR_CC_DXT4: u32 = 0x3454_5844;
const D3DFMT_A8P8: u32 = 40;
const D3DFMT_P8: u32 = 41;
const SEEK_CUR: i32 = 1;

/// IDA: the texture memory counters: 00B42054/00B42058 g_niDX9SourceTexture Bytes/PaddingBytes (00760803/00760824,
/// 007611A6/007611C7) and the cube counters 00B4283C/00B42840 (0077449A/007744BB). 100% host counters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextureMemoryStatistics {
    pub texture_bytes: u32,
    pub texture_padding: u32,
    pub cube_bytes: u32,
    pub cube_padding: u32,
}

/// IDA: NiDX9SourceTextureData (0x7C, vtable A883C4; 00760BF0) and NiDX9SourceCubeMapData (0x80, vtable A89F80; 00774550)
/// without their device objects: +0x0C the chosen table NiPixelFormat, +0x54/+0x58 size, +0x5C levels, +0x60 texture
/// bytes, +0x64 replacement, +0x65 mipmap, +0x68 pixel bytes, +0x74 (2D) / +0x7C (cube) source revision. `device_texture`
/// stands for +0x50 being set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextureRendererData {
    pub original_vtable: u32,
    pub cube: bool,
    /// 00760DA0 D3DX path.
    pub direct_load: bool,
    pub table_case: Option<usize>,
    pub pixel_format_0c: PixelFormat,
    pub width_54: u32,
    pub height_58: u32,
    pub levels_5c: u32,
    pub texture_bytes_60: u32,
    pub replacement_64: bool,
    pub mipmap_65: bool,
    pub pixel_bytes_68: u32,
    pub source_revision: u32,
    /// Raw D3DFORMAT; 0 is D3DFMT_UNKNOWN.
    pub d3d_format: u32,
    pub device_texture: bool,
}

impl Default for TextureRendererData {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            original_vtable: 0x00A8_83C4,
            cube: false,
            direct_load: false,
            table_case: None,
            pixel_format_0c: PixelFormat::default(),
            width_54: 0,
            height_58: 0,
            levels_5c: 0,
            texture_bytes_60: 0,
            replacement_64: false,
            mipmap_65: false,
            pixel_bytes_68: 0,
            source_revision: 0,
            d3d_format: 0,
            device_texture: false,
        }
    }
}

/// What renderer vt+0x104's upload answered for one-level pixel data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceTextureUpload {
    pub table_case: Option<usize>,
    /// The chosen row of the renderer's pixel format table 0.
    pub table_format: PixelFormat,
    pub d3d_format: u32,
    pub device_texture: bool,
}

/// IDA: `RendererTextureStatus` in the C++ port.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RendererTextureStatus {
    Created,
    /// 00774580: no cube format in renderer +0x7A4.
    NoCubeFormats,
    /// 00701400 error-replacement textures (8000000x codes): carried.
    ReplacementTexture,
    /// 0071B8D0 mipmap generation: carried (the effect textures do not ask).
    MipmapGeneration,
    /// A 0071EA60 conversion pair that is not ported.
    UnsupportedConversion,
    /// A failed device call (the original logs through nullsub_2).
    DeviceFailure,
}

/// The D3DX image kinds 00760DA0 branches on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirectResourceType {
    Texture,
    CubeTexture,
    VolumeTexture,
    Other,
}

/// IDA: D3DXIMAGE_INFO as 00760EEE reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DirectImageInfo {
    pub width: u32,
    pub height: u32,
    pub depth: u32,
    pub format: u32,
    pub resource_type: DirectResourceType,
    pub image_file_format: u32,
}

/// One GetLevelDesc answer 007609C0 reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DirectLevelDesc {
    pub format: u32,
    pub width: u32,
    pub height: u32,
    pub depth: u32,
}

/// The device boundary: every call the C++ makes into `renderer.cpp`, D3DX or the device. A `None` renderer in
/// `TextureLoadServices` is the original's null 00B3F928.
pub trait TextureRendererBoundary {
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    /// `renderer +0x280` is non-null.
    fn has_device(&self) -> bool;
    /// IDA: 00763560 -> 00761270's one-level upload (`create_renderer_source_texture` or, without a device,
    /// `prepare_renderer_source_texture`).
    fn upload_source_texture(
        &mut self,
        format: &PixelFormat,
        width: u32,
        height: u32,
        texels: &[u8],
        prefs: [u32; 3],
    ) -> SourceTextureUpload;
    /// IDA: 007635C0 -> 00774550 `create_renderer_cube_texture`, which fills `out` and the cube counters.
    fn create_cube_texture(
        &mut self,
        data: &PixelData,
        prefs: [u32; 3],
        out: &mut TextureRendererData,
        statistics: &mut TextureMemoryStatistics,
    ) -> RendererTextureStatus;
    /// IDA: 00760EF5 D3DXGetImageInfoFromFileInMemory. `None`: the DLL is missing or the call failed.
    fn image_info(&mut self, bytes: &[u8]) -> Option<DirectImageInfo>;
    /// IDA: 007610D3 D3DXCreate(Cube|Volume)TextureFromFileInMemory(device, data, original size), then GetLevelCount and
    /// GetLevelDesc per level (`None` for a failed one). `None`: the creation failed.
    fn create_direct_texture(
        &mut self,
        resource_type: DirectResourceType,
        bytes: &[u8],
        original_size: u32,
    ) -> Option<Vec<Option<DirectLevelDesc>>>;
}

pub type OpenFileHook<'a> = Box<dyn FnMut(&[u8]) -> Option<Box<dyn GameFile>> + 'a>;
pub type FileExistsHook<'a> = Box<dyn FnMut(&[u8]) -> bool + 'a>;
pub type PaletteFindHook<'a> = Box<dyn FnMut(&[u8]) -> Option<Entity> + 'a>;
pub type PaletteAddHook<'a> = Box<dyn FnMut(&[u8], Option<Entity>) + 'a>;
pub type WarningHook<'a> = Box<dyn FnMut(&[u8]) + 'a>;

/// IDA: N/A host services the texture paths use: g_bsFileOpenCallback (j_psub_748110, 0x8000 buffer),
/// g_bsFileExistsCallback (j_psub_748170, mode 0), the BSTexturePalette (00B42EBC) find/add, the renderer, 00B256CC
/// (create renderer data on create), 00B3F958 (drop static app pixel data), 00B3F709 (direct-to-renderer hint),
/// 00B4205C/00B42060 (mip skip and minimum, from the renderer globals) and the warning callback 00B42E8C. A field left
/// `None` is the C++ port's null `std::function`.
pub struct TextureLoadServices<'a> {
    pub open_file: Option<OpenFileHook<'a>>,
    pub file_exists: Option<FileExistsHook<'a>>,
    pub palette_find: Option<PaletteFindHook<'a>>,
    pub palette_add: Option<PaletteAddHook<'a>>,
    pub warning: Option<WarningHook<'a>>,
    pub renderer: Option<&'a mut dyn TextureRendererBoundary>,
    pub mip_limits: Option<(u32, u32)>,
    pub create_renderer_data_on_create: bool,
    pub drop_static_app_data: bool,
    pub direct_to_renderer_hint: bool,
    pub statistics: Option<&'a mut TextureMemoryStatistics>,
}

impl Default for TextureLoadServices<'_> {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            open_file: None,
            file_exists: None,
            palette_find: None,
            palette_add: None,
            warning: None,
            renderer: None,
            mip_limits: None,
            create_renderer_data_on_create: true,
            drop_static_app_data: true,
            direct_to_renderer_hint: true,
            statistics: None,
        }
    }
}

/// IDA: 00748760 PathParts_ParsePath: _splitpath_s into Drive[3], Dir[0x100], Filename[0x100], Ext[0x100]; +0x303 sub
/// directory = "".
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct PathParts {
    drive: Vec<u8>,
    dir: Vec<u8>,
    filename: Vec<u8>,
    ext: Vec<u8>,
    sub_dir: Vec<u8>,
}

// IDA: 00748760 PathParts_ParsePath.
fn parse_path(full_path: &[u8]) -> PathParts {
    let path = c_prefix(full_path);
    let mut parts = PathParts::default();
    let mut start = 0;
    if path.len() >= 2 && path[1] == b':' {
        parts.drive = path[..2].to_vec();
        start = 2;
    }
    let body = &path[start..];
    let name_start = body
        .iter()
        .rposition(|&byte| byte == b'/' || byte == b'\\')
        .map_or(0, |slash| slash + 1);
    parts.dir = body[..name_start].to_vec();
    let name = &body[name_start..];
    match name.iter().rposition(|&byte| byte == b'.') {
        Some(dot) => {
            parts.filename = name[..dot].to_vec();
            parts.ext = name[dot..].to_vec();
        }
        None => parts.filename = name.to_vec(),
    }
    // _splitpath_s: a component past its buffer is ERANGE, with every output cleared.
    if parts.dir.len() >= 0x100 || parts.filename.len() >= 0x100 || parts.ext.len() >= 0x100 {
        return PathParts::default();
    }
    parts
}

/// IDA: _makepath_s into the callers' 0x104 buffers; an overrun clears the buffer.
fn make_path(drive: &[u8], dir: &[u8], filename: &[u8], ext: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    if let Some(&letter) = drive.first() {
        out.extend([letter, b':']);
    }
    out.extend(dir);
    if dir
        .last()
        .is_some_and(|&last| last != b'\\' && last != b'/')
    {
        out.push(b'\\');
    }
    out.extend(filename);
    if let Some(&first) = ext.first() {
        if first != b'.' {
            out.push(b'.');
        }
        out.extend(ext);
    }
    if out.len() >= TEXTURE_PATH_BUFFER {
        out.clear();
    }
    out
}

/// IDA: 00748670 PathParts_BuildPath: with a sub directory, Dir + ('\' when it ends with neither '\' nor '/') + sub
/// directory, then _makepath. Host: bounded by the callers' 0x104 buffers (the unbounded original would overrun them;
/// startup paths stay short).
fn build_path(parts: &PathParts) -> Vec<u8> {
    if parts.sub_dir.is_empty() {
        return make_path(&parts.drive, &parts.dir, &parts.filename, &parts.ext);
    }
    let mut destination = parts.dir.clone();
    if destination
        .last()
        .is_some_and(|&last| last != b'\\' && last != b'/')
    {
        destination.push(b'\\');
    }
    destination.extend(&parts.sub_dir);
    make_path(&parts.drive, &destination, &parts.filename, &parts.ext)
}

/// IDA: 007479F0 NiSearchPath_GetNextSearchPath for a fresh base search path (+0x10C reference path empty, the 00B40230
/// default path never written): index 0 the path itself, 1 drive/dir cleared, 2 the reference path with the file's
/// directory as sub directory, 3 the reference path alone, 4 the default path when nonempty, then false.
/// Completion: 100% for that fresh state.
fn base_search_candidates(file_path: &[u8]) -> Vec<Vec<u8>> {
    let source = parse_path(file_path);
    let mut candidates = vec![build_path(&source)];
    let mut parts = source.clone();
    parts.drive.clear();
    parts.dir.clear();
    candidates.push(build_path(&parts));
    parts.sub_dir.clone_from(&source.dir);
    candidates.push(build_path(&parts));
    parts.sub_dir.clear();
    candidates.push(build_path(&parts));
    // Index 4: 00B40230 has no writer, so the walk ends.
    candidates
}

// IDA: the page padding the texture counters add (to the next 4 KB boundary).
// Completion: 100%.
#[must_use]
pub const fn page_padding(bytes: u32) -> u32 {
    let page = bytes & 0xFFFF_F000;
    if page == bytes {
        0
    } else {
        page.wrapping_sub(bytes).wrapping_add(0x1000)
    }
}

/// IDA: the NiDDSReader header fields 007363F0 leaves in the reader (+0x100 height, +0x104 width, +0x108 levels, +0x10C
/// byte, +0x110 file format) and its outputs (format, mipmapped byte, faces). 100% host view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DdsHeader {
    pub width: u32,
    pub height: u32,
    pub levels: u32,
    pub mipmapped: bool,
    pub faces: u32,
    pub file_format: PixelFormat,
    pub output_format: PixelFormat,
    /// The RGB branch's five dwords: bit count, then the red, green, blue and alpha masks. Alpha is 0 unless the pixel format's flags have 1 (0073673F).
    pub rgb_masks: Option<[u32; 5]>,
}

impl Default for DdsHeader {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            width: 0,
            height: 0,
            levels: 1,
            mipmapped: false,
            faces: 1,
            file_format: PixelFormat::default(),
            output_format: PixelFormat::default(),
            rgb_masks: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DdsReadStatus {
    Read,
    /// 007363F0 returned false (magic, size, flags, volume, unknown format, caps or incomplete cube faces).
    Rejected,
    /// The 16/24/32-bit RGB branch read its header. Its pixels (00736950/00736A20/00736EB0 and 0070F360) are carried: the direct path takes such a file without them.
    UnsupportedRgbConversion,
}

/// IDA: 006ED6A0: file vt+4 Read(dst, 4, &4, 1) for a 32-bit field. A short read leaves `value`'s bytes.
fn read_u32(file: &mut dyn GameFile, value: &mut u32) {
    let mut bytes = value.to_le_bytes();
    let _ = file.read_components(&mut bytes, Some(&[4]));
    *value = u32::from_le_bytes(bytes);
}

// IDA: 007363F0 NiDDSReader::ReadHeader: "DDS " magic (a byte-swapped magic turns endian swapping on), dwSize 124, (flags &
// 0x1007) == 0x1007, height, width, pitch, depth (DDSD_DEPTH with a depth is rejected), mip count, 0x2C skipped, pixel format
// size/flags/FourCC: DXT1/DXT3/DXT5 and the float formats 111..116 by FourCC (statics copied raw; the 0x14 masks skipped), else
// the RGB branch; caps1 needs COMPLEX or TEXTURE, caps2 VOLUME is rejected, CUBEMAP needs all six face bits (faces 6); 0x0C
// skipped; levels = mip count when COMPLEX | MIPMAP and the count is not 1, else 1.
// Completion: 100% of the FourCC branch.
// TODO: [ ] The RGB branch reports `UnsupportedRgbConversion`.
pub fn read_dds_header(file: &mut dyn GameFile, header: &mut DdsHeader) -> DdsReadStatus {
    // 0073641E..0073647F magic; a byte-swapped magic enables swapping.
    let mut magic = 0;
    read_u32(file, &mut magic);
    if magic != 0x2053_4444 {
        if magic.swap_bytes() != 0x2053_4444 {
            return DdsReadStatus::Rejected;
        }
        file.set_endian_swap(true);
    }
    // 00736481..0073649B dwSize 124.
    let mut size = 0;
    read_u32(file, &mut size);
    if size != 124 {
        return DdsReadStatus::Rejected;
    }
    // 007364E8..00736510 (flags & 0x1007) == 0x1007.
    let mut flags = 0;
    read_u32(file, &mut flags);
    if flags & 0x1007 != 0x1007 {
        return DdsReadStatus::Rejected;
    }
    // 00736535..00736596 height, width, pitch, depth.
    read_u32(file, &mut header.height);
    read_u32(file, &mut header.width);
    let mut pitch_or_depth = 0;
    read_u32(file, &mut pitch_or_depth);
    read_u32(file, &mut pitch_or_depth);
    if flags & 0x0080_0000 != 0 && pitch_or_depth != 0 {
        return DdsReadStatus::Rejected;
    }
    // 007365CF..007365FA mip count, 0x2C reserved bytes.
    let mut mip_count = 0;
    read_u32(file, &mut mip_count);
    file.seek(0x2C, SEEK_CUR);
    // 007365FC..00736641 pixel format size/flags/FourCC.
    let mut pf_size = 0;
    let mut pf_flags = 0;
    let mut four_cc = 0;
    read_u32(file, &mut pf_size);
    read_u32(file, &mut pf_flags);
    read_u32(file, &mut four_cc);
    // 0073664A +0x10C = 0.
    if pf_flags & 4 != 0 {
        // 00736657..007366C1 the FourCC statics (DXT2/DXT4 and others reject).
        let source = match four_cc {
            FOUR_CC_DXT1 => 0x00B2_5FB0, // src__10
            FOUR_CC_DXT3 => 0x00B2_5FF8, // src__11
            FOUR_CC_DXT5 => 0x00B2_6040, // src__12
            111 => 0x00B2_61F0,          // src__18
            112 => 0x00B2_6160,          // src__19
            113 => 0x00B2_60D0,          // src__20
            114 => 0x00B2_6118,          // src__21
            115 => 0x00B2_61A8,          // src__22
            116 => 0x00B2_6088,          // src__23
            _ => return DdsReadStatus::Rejected,
        };
        // 007366C6..007366E3 rep movsd into +0x110, then into the output.
        header.file_format = d3d_pixel_format_static(source).expect("a listed static");
        header.output_format = header.file_format;
        file.seek(0x14, SEEK_CUR);
    } else if pf_flags & 0x40 != 0 {
        // 00736705..0073673C bit count, R, G, B, A; 0073673F alpha only with flag 1; 00736769 007362C0 takes 16, 24 and 32 bits.
        let mut masks = [0u32; 5];
        for field in &mut masks {
            read_u32(file, field);
        }
        if pf_flags & 1 == 0 {
            masks[4] = 0;
        }
        if !matches!(masks[0], 16 | 24 | 32) {
            return DdsReadStatus::Rejected;
        }
        header.rgb_masks = Some(masks);
    } else {
        return DdsReadStatus::Rejected;
    }
    // 007367DC..00736811 caps1: COMPLEX, or TEXTURE/MIPMAP (0x401000).
    let mut caps1 = 0;
    read_u32(file, &mut caps1);
    if caps1 & 8 == 0 && caps1 & 0x0040_1000 == 0 {
        return DdsReadStatus::Rejected;
    }
    // 00736811..00736875 caps2: VOLUME rejected, CUBEMAP needs all six faces.
    let mut caps2 = 0;
    read_u32(file, &mut caps2);
    if caps2 & 0x0020_0000 != 0 {
        return DdsReadStatus::Rejected;
    }
    if caps2 & 0x200 != 0 && caps2 & 0xFC00 != 0xFC00 {
        return DdsReadStatus::Rejected;
    }
    header.faces = if caps2 & 0x200 != 0 { 6 } else { 1 };
    // 00736877..00736888 caps3, caps4, reserved2.
    file.seek(0x0C, SEEK_CUR);
    // 007368A8..007368F6 mipmapped levels.
    if caps1 & 8 != 0 && caps1 & 0x0040_0000 != 0 && mip_count != 1 {
        header.mipmapped = true;
        header.levels = mip_count;
    } else {
        header.mipmapped = false;
        header.levels = 1;
    }
    if header.rgb_masks.is_some() {
        return DdsReadStatus::UnsupportedRgbConversion;
    }
    DdsReadStatus::Read
}

/// IDA: N/A bytes already in memory as a read-only `GameFile`, for `read_dds_header`.
struct MemoryFile<'a> {
    core: FileCore,
    bytes: &'a [u8],
    position: usize,
}

impl GameFile for MemoryFile<'_> {
    /// IDA: N/A `MemoryFile`'s side of `GameFile`.
    fn core(&self) -> &FileCore {
        &self.core
    }
    /// IDA: N/A `MemoryFile`'s side of `GameFile`.
    fn core_mut(&mut self) -> &mut FileCore {
        &mut self.core
    }
    /// IDA: N/A `MemoryFile`'s side of `GameFile`.
    fn proc_read(&mut self, buffer: &mut [u8]) -> usize {
        let rest = self.bytes.get(self.position..).unwrap_or(&[]);
        let count = buffer.len().min(rest.len());
        buffer[..count].copy_from_slice(&rest[..count]);
        self.position += count;
        count
    }
    /// IDA: N/A `MemoryFile`'s side of `GameFile`.
    fn proc_write(&mut self, _source: &[u8]) -> usize {
        0
    }
    /// IDA: N/A `MemoryFile`'s side of `GameFile`.
    fn seek(&mut self, offset: i32, _origin: i32) {
        self.position = self.position.saturating_add_signed(offset as isize); // Only SEEK_CUR is used here.
    }
    /// IDA: N/A `MemoryFile`'s side of `GameFile`.
    fn size(&mut self) -> u32 {
        self.bytes.len() as u32
    }
}

/// IDA: N/A what a DDS file in memory says, through the NiDDSReader's header port: everything here is read from the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DirectFileFacts {
    pub width: u32,
    pub height: u32,
    /// The file's own mip count.
    pub levels: u32,
    /// The ddspf FourCC, which is the D3DFORMAT (DXTn by character code, 111..116 by number).
    pub format: u32,
    pub resource_type: DirectResourceType,
}

// IDA: N/A the DDS facts behind the D3DX stand-ins, with the header reader's limits. `None` is a host stop, not a D3DX answer: D3DX
// also reads files the NiDDSReader refuses (DXT2, DXT4, RGB masks, volumes, .bmp, .tga), and no startup file reaches those.
// Completion: 100% of the FourCC branch.
// TODO: [ ] The other D3DX inputs, with the first caller that reaches one.
#[must_use]
pub fn direct_file_facts(bytes: &[u8]) -> Option<DirectFileFacts> {
    let mut file = MemoryFile {
        core: crate::gamebryo::file::FileCore::default(),
        bytes,
        position: 0,
    };
    let mut header = DdsHeader::default();
    let status = read_dds_header(&mut file, &mut header);
    if status == DdsReadStatus::Rejected || file.core().swap_active() {
        return None;
    }
    let format = match header.rgb_masks {
        Some(masks) => rgb_d3d_format(masks)?,
        None => u32::from_le_bytes(bytes.get(0x54..0x58)?.try_into().ok()?),
    };
    Some(DirectFileFacts {
        width: header.width,
        height: header.height,
        levels: header.levels,
        format,
        resource_type: if header.faces == 6 {
            DirectResourceType::CubeTexture
        } else {
            DirectResourceType::Texture
        },
    })
}

// IDA: N/A D3DX's format for a file's RGB masks: the D3DFORMAT with those masks. This is why `direct_image_info` answered None for cursor.dds and the ribbon DDS: they're A8R8G8B8 with flags 0x41 and no FourCC, the branch 007363F0 sends through its own conversion. No original run has read D3DX's answer for one (no data).
// Completion: 100% of the masks the menu files hold; the others are a host stop.
#[must_use]
fn rgb_d3d_format(masks: [u32; 5]) -> Option<u32> {
    match masks {
        [32, 0x00FF_0000, 0x0000_FF00, 0x0000_00FF, 0xFF00_0000] => Some(21),
        [32, 0x00FF_0000, 0x0000_FF00, 0x0000_00FF, 0] => Some(22),
        [24, 0x00FF_0000, 0x0000_FF00, 0x0000_00FF, 0] => Some(20),
        [16, 0xF800, 0x07E0, 0x001F, 0] => Some(23),
        [16, 0x7C00, 0x03E0, 0x001F, 0x8000] => Some(25),
        [16, 0x0F00, 0x00F0, 0x000F, 0xF000] => Some(26),
        _ => None,
    }
}

// IDA: N/A D3DXGetImageInfoFromFileInMemory's D3DXIMAGE_INFO for a DDS file, as data: width, height and format are the file's; depth 1
// and D3DXIFF_DDS (4) are what D3DX reports for any non-volume DDS. The DLL is not part of the executable, so its answer is a host
// substitution. Compared with P05-RUN-001 and P05-RUN-002 through the five file-backed effect slots.
// Completion: 100% of the DDS FourCC inputs.
#[must_use]
pub fn direct_image_info(bytes: &[u8]) -> Option<DirectImageInfo> {
    let facts = direct_file_facts(bytes)?;
    Some(DirectImageInfo {
        width: facts.width,
        height: facts.height,
        depth: 1,
        format: facts.format,
        resource_type: facts.resource_type,
        image_file_format: 4,
    })
}

// IDA: N/A the texture D3DXCreateTextureFromFileInMemory makes from a DDS file: the D3DX_DEFAULT size, level and format arguments take
// the file's own for a power-of-two size, and a chain shorter than full is generated down to 1, as D3DX_DEFAULT asks (a no-data answer: the menu's logo and bars hold one level, and no run has read their level counts). Each level halves down to 1. `None`
// is a host stop for a size that is not a power of two (D3DX rounds). The D3D9 runtime's sizes for DXT levels below 4 are not recorded in either run; they feed only +0x68,
// which is 0 for a DXTn format (`bits_per_pixel_01` is 0).
// Completion: 100% of the reached inputs.
// TODO: [ ] The D3DX rounding arm, with the first file that reaches it.
#[must_use]
pub fn direct_texture_levels(facts: &DirectFileFacts) -> Option<Vec<DirectLevelDesc>> {
    let power_of_two = |value: u32| value != 0 && value & (value - 1) == 0;
    if !power_of_two(facts.width) || !power_of_two(facts.height) {
        return None;
    }
    let full_chain = 32 - facts.width.max(facts.height).leading_zeros();
    Some(
        (0..full_chain)
            .map(|level| DirectLevelDesc {
                format: facts.format,
                width: (facts.width >> level).max(1),
                height: (facts.height >> level).max(1),
                depth: 1,
            })
            .collect(),
    )
}

// IDA: 00737330 NiDDSReader::ReadFile(stream, optionalDest): 007363F0, a NiPixelData(width, height, output format, levels, faces)
// (the optional destination is reused only when it matches; startup passes null), then for equal file/output formats each
// face's levels read straight into the pixel data (component size 1, the read count unchecked).
// Completion: 100% of the FourCC branch.
pub fn read_dds_file(file: &mut dyn GameFile) -> (DdsReadStatus, Option<PixelData>) {
    let mut header = DdsHeader::default();
    let status = read_dds_header(file, &mut header);
    if status != DdsReadStatus::Read {
        return (status, None);
    }
    // 0073741D..00737476 a new NiPixelData.
    let mut data = construct_pixel_data(
        header.width,
        header.height,
        &header.output_format,
        header.levels,
        header.faces,
    );
    // 007374A9..00737549 equal formats: per face (slot = switch index + 1), each level read straight into the pixel data; the
    // read count is not checked.
    for face in 0..header.faces {
        for level in 0..data.levels_60 {
            let _ = file.read_components(data.level_mut(face, level), Some(&[1]));
        }
    }
    (status, Some(data))
}

// IDA: 0071E690 NiDevImageConverter_ReadImageFile(path, optionalDest): path copied into 0x104 bytes, the extension parsed, the
// first reader whose CanReadFile(ext) accepts it (NIF 00733920 and TGA 00734AC0 decline .dds, DDS 007362A0 accepts; SGI
// 00735920 and BMP 00737730 follow) reads the opened file (0x8000 buffer); a file that is not good returns null.
// Completion: 100% for .dds.
// TODO: [ ] The other readers return null (carried).
pub fn read_image_file(path: &[u8], services: &mut TextureLoadServices<'_>) -> Option<PixelData> {
    // 0071E6BC..0071E6EB the 0x104 copy and the extension.
    let name = c_prefix(path);
    if name.len() >= TEXTURE_PATH_BUFFER {
        return None;
    }
    let open_file = services.open_file.as_mut()?;
    // Reader order NIF, TGA, DDS, SGI, BMP: only NiDDSReader (007362A0 CanReadFile _stricmp(ext, ".dds")) is ported.
    if stricmp(&parse_path(name).ext, b".dds") != 0 {
        return None;
    }
    // 0071E711..0071E754 open (0x8000), good check, ReadFile, delete.
    let mut file = open_file(name)?;
    if !file.good() {
        return None;
    }
    let (status, data) = read_dds_file(file.as_mut());
    if status == DdsReadStatus::Read {
        data
    } else {
        None
    }
}

// IDA: 00760F6A..007610C1 the DDS FourCC mip skip (00B4205C/00B42060), for a DDS file (`image_file_format` 4) whose ddspf flags
// (+0x50) carry FOURCC: unsigned max(levels - skip, minimum), or min(levels, minimum) when levels <= skip; per dropped level
// ((h/4)*(w/4)*16)/div. Rewrites `bytes`, the header with the new height/width/linear size/levels then the kept data, and
// `info`'s size. Host: the buffer keeps the original size (zero tail) because 007610C3 reloads the original size for D3DX.
// Returns false where the original would copy out of bounds on a malformed file.
// Completion: 100% of the arithmetic.
pub fn dds_trim_mip_levels(
    bytes: &mut Vec<u8>,
    info: &mut DirectImageInfo,
    mip_skip: u32,
    mip_minimum: u32,
) -> bool {
    let size = bytes.len();
    if info.image_file_format != 4 || size < 0x80 || bytes[0x50] & 4 == 0 {
        return true;
    }
    let dword =
        |at: usize| u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
    let four_cc = dword(0x54);
    let divisor = if four_cc == FOUR_CC_DXT1 {
        2
    } else if (FOUR_CC_DXT2..=FOUR_CC_DXT5).contains(&four_cc) {
        1
    } else {
        return true;
    };
    // 00760FC9..00760FEA.
    let levels = dword(0x1C);
    let mut keep = levels.wrapping_sub(mip_skip).max(mip_minimum);
    if levels <= mip_skip {
        keep = levels.min(mip_minimum);
    }
    if keep >= levels {
        return true;
    }
    // 00761000..0076105F.
    let mut used = size as u32;
    let mut offset = 0x80usize;
    for _ in 0..levels - keep {
        let blocks_w = (info.width >> 2).max(1);
        let blocks_h = (info.height >> 2).max(1);
        let level_bytes = blocks_h.wrapping_mul(blocks_w).wrapping_mul(16) / divisor;
        offset = offset.wrapping_add(level_bytes as usize);
        used = used.wrapping_sub(level_bytes);
        info.width = (info.width >> 1).max(1);
        info.height = (info.height >> 1).max(1);
    }
    // Host stop: the checks the 0076106E..007610C1 copy does not have.
    let kept = (used as usize).wrapping_sub(0x80);
    if used as usize > size || used < 0x80 || offset.checked_add(kept).is_none_or(|end| end > size)
    {
        return false;
    }
    // 0076106E..007610C1 a `used`-byte copy.
    let mut trimmed = vec![0; size];
    trimmed[..0x80].copy_from_slice(&bytes[..0x80]);
    trimmed[0x10..0x14].copy_from_slice(&info.width.to_le_bytes());
    trimmed[0x0C..0x10].copy_from_slice(&info.height.to_le_bytes());
    trimmed[0x1C..0x20].copy_from_slice(&keep.to_le_bytes());
    let linear = info.width.wrapping_mul(info.height);
    trimmed[0x14..0x18].copy_from_slice(&linear.to_le_bytes());
    trimmed[0x80..0x80 + kept].copy_from_slice(&bytes[offset..offset + kept]);
    *bytes = trimmed;
    true
}

// IDA: 007609C0 vt+0x24 InitializeFromD3DTexture: levels, mipmap = levels != 1, pixel format (0076C3B0; P8/A8P8 fail on level 0),
// pixel bytes per level, the cube's six times. The level answers come over the device boundary.
// Completion: 100%.
pub fn initialize_from_level_descs(
    data: &mut TextureRendererData,
    resource_type: DirectResourceType,
    levels: &[Option<DirectLevelDesc>],
) -> bool {
    // 007609CF..007609E3.
    data.levels_5c = levels.len() as u32;
    data.mipmap_65 = data.levels_5c != 1;
    data.pixel_bytes_68 = 0;
    if resource_type != DirectResourceType::Other {
        for (level, desc) in levels.iter().enumerate() {
            let Some(desc) = desc else {
                return false;
            };
            if level == 0 {
                data.height_58 = desc.height;
                data.width_54 = desc.width;
                // 00760A25..00760A30: P8 (41) / A8P8 (40) fail.
                if desc.format == D3DFMT_A8P8 || desc.format == D3DFMT_P8 {
                    return false;
                }
                data.pixel_format_0c = pixel_format_from_d3d(desc.format);
                data.d3d_format = desc.format;
            }
            let bits = u32::from(data.pixel_format_0c.bits_per_pixel_01);
            data.pixel_bytes_68 = data.pixel_bytes_68.wrapping_add(
                bits.wrapping_mul(desc.height)
                    .wrapping_mul(desc.width)
                    .wrapping_mul(desc.depth)
                    >> 3,
            );
        }
        if resource_type == DirectResourceType::CubeTexture {
            data.pixel_bytes_68 = data.pixel_bytes_68.wrapping_mul(6); // 00760B21..00760B2F
            data.cube = true;
        }
    }
    data.device_texture = true;
    true
}

/// IDA: 00760EFE..00760F38 layout 4 (bump) accepts only the bump formats.
const fn direct_format_accepted(pixel_layout: u32, format: u32) -> bool {
    pixel_layout != 4
        || matches!(
            format,
            0x3C | 0x3F | 0x40 | 0x6E | 0x75 | 0x3D | 0x3E | 0x43
        )
}

// IDA: 00760DA0 the direct-to-renderer load and 007609C0: .bmp/.tga/.dds only; the whole file read; the D3DX image info; layout 4
// (bump) only accepts the bump formats; a DDS FourCC texture may drop its top levels; the D3DX creation; the renderer data takes
// levels, mipmap, size, pixel format, pixel bytes, then +0x54/+0x58 from the image info and the 00B42054/00B42058 counters (DXTn
// count 1 byte per pixel).
// Completion: 100% of the branches above the device boundary.
pub fn load_texture_direct(
    path: &[u8],
    prefs: [u32; 3],
    services: &mut TextureLoadServices<'_>,
    out: &mut TextureRendererData,
) -> bool {
    // 00760DB4..00760DEF renderer and its +0x280 device.
    if !services.renderer.as_deref().is_some_and(|r| r.has_device()) {
        return false;
    }
    let name = c_prefix(path);
    if name.len() >= TEXTURE_PATH_BUFFER {
        return false;
    }
    // 00760E14..00760E71 .bmp / .tga / .dds.
    let ext = parse_path(name).ext;
    if stricmp(&ext, b".bmp") != 0 && stricmp(&ext, b".tga") != 0 && stricmp(&ext, b".dds") != 0 {
        return false;
    }
    // 00760E77..00760EEC open (0x8000), good, size, FormHeap(size), read, delete.
    let Some(open_file) = services.open_file.as_mut() else {
        return false;
    };
    let Some(mut file) = open_file(name) else {
        return false;
    };
    if !file.good() {
        return false;
    }
    let size = file.size();
    if size == 0 {
        return false;
    }
    let mut bytes = vec![0; size as usize];
    let _ = file.read_components(&mut bytes, Some(&[1]));
    drop(file);
    let renderer = services.renderer.as_deref_mut().expect("checked above");
    // 00760EEE D3DXGetImageInfoFromFileInMemory.
    let Some(mut info) = renderer.image_info(&bytes) else {
        return false;
    };
    if !direct_format_accepted(prefs[0], info.format) {
        return false;
    }
    // 00760F4A..00760F60 the image size kept for +0x54/+0x58; the byte counters below use the (possibly trimmed) info size.
    let image_width = info.width;
    let image_height = info.height;
    if info.resource_type == DirectResourceType::Other {
        return false; // D3DERR_INVALIDCALL stays in `result`.
    }
    if info.resource_type == DirectResourceType::Texture
        && let Some((mip_skip, mip_minimum)) = services.mip_limits
        && !dds_trim_mip_levels(&mut bytes, &mut info, mip_skip, mip_minimum)
    {
        return false;
    }
    // 007610C3..007610D3 the creation, with the original size.
    let Some(levels) = renderer.create_direct_texture(info.resource_type, &bytes, size) else {
        return false;
    };
    // 00761149..00761173 vt+0x24, then +0x54/+0x58 from the saved image size (stored before the result is tested); a failure
    // releases the texture.
    out.direct_load = true;
    let initialized = initialize_from_level_descs(out, info.resource_type, &levels);
    out.width_54 = image_width;
    out.height_58 = image_height;
    if !initialized {
        return false;
    }
    // 00761175..007611C7 bytes: DXT1..DXT5 count one byte per pixel.
    let bytes_per_pixel = if matches!(
        info.format,
        FOUR_CC_DXT1 | FOUR_CC_DXT2 | FOUR_CC_DXT3 | FOUR_CC_DXT4 | FOUR_CC_DXT5
    ) {
        1
    } else {
        u32::from(pixel_format_from_d3d(info.format).bits_per_pixel_01) >> 3
    };
    out.texture_bytes_60 = info
        .depth
        .wrapping_mul(bytes_per_pixel)
        .wrapping_mul(info.height)
        .wrapping_mul(info.width);
    count_texture_bytes(services, out.texture_bytes_60);
    true
}

// IDA: N/A a helper, with no address of its own.
fn count_texture_bytes(services: &mut TextureLoadServices<'_>, bytes: u32) {
    if let Some(statistics) = services.statistics.as_deref_mut() {
        statistics.texture_bytes = statistics.texture_bytes.wrapping_add(bytes);
        statistics.texture_padding = statistics.texture_padding.wrapping_add(page_padding(bytes));
    }
}

// IDA: 00701C00 + renderer vt+0x104 00763560 + 00761270 for a texture that carries app pixel data: the one-level path.
// Completion: 100% of the accounting; the upload is the device boundary.
fn create_pixel_renderer_data(
    world: &World,
    texture: &mut SourceTexture,
    services: &mut TextureLoadServices<'_>,
) -> bool {
    let Some(data) = texture
        .app_pixel_data_3c
        .and_then(|entity| world.get::<PixelData>(entity))
    else {
        return false;
    };
    let Some(renderer) = services.renderer.as_deref_mut() else {
        return false;
    };
    if data.levels_60 != 1 || data.faces_6c != 1 {
        return false;
    }
    let has_device = renderer.has_device();
    let prefs = [
        texture.pixel_layout_18,
        texture.alpha_format_1c,
        texture.mip_mapped_20,
    ];
    let upload = renderer.upload_source_texture(
        &data.format_08,
        data.widths_54[0],
        data.heights_58[0],
        data.level(0, 0),
        prefs,
    );
    if upload.table_case.is_none() || upload.d3d_format == 0 {
        return false;
    }
    let mut result = TextureRendererData {
        table_case: upload.table_case,
        pixel_format_0c: upload.table_format,
        width_54: data.widths_54[0],
        height_58: data.heights_58[0],
        levels_5c: 1,
        // 00761584..00761595.
        pixel_bytes_68: data.offsets_5c[data.levels_60 as usize].wrapping_mul(data.faces_6c),
        source_revision: data.revision_68, // 007614DC..007614E3
        d3d_format: upload.d3d_format,
        device_texture: upload.device_texture,
        ..TextureRendererData::default()
    };
    result.texture_bytes_60 = (u32::from(result.pixel_format_0c.bits_per_pixel_01) >> 3)
        .wrapping_mul(result.width_54)
        .wrapping_mul(result.height_58);
    count_texture_bytes(services, result.texture_bytes_60);
    if has_device && !result.device_texture {
        return false;
    }
    texture.renderer_24 = Some(result);
    texture.renderer_data_24 = true;
    true
}

// IDA: 007205A0 the six-face merge: one 6-face NiPixelData with face 0's size/levels/format; face i takes source i's face 0 levels.
// Completion: 100%.
fn merge_cube_faces(faces: [&PixelData; 6]) -> PixelData {
    let first = faces[0];
    let mut merged = construct_pixel_data(
        first.widths_54[0],
        first.heights_58[0],
        &first.format_08,
        first.levels_60,
        6,
    );
    let stride = *merged.offsets_5c.last().expect("levels + 1 offsets") as usize;
    for (face, source) in faces.iter().enumerate() {
        for level in 0..source.levels_60 {
            let from = source.level(0, level);
            let start = stride * face
                + merged.offsets_5c[(level as usize).min(merged.levels_60 as usize)] as usize;
            let count = from.len().min(merged.pixels_50.len() - start);
            merged.pixels_50[start..start + count].copy_from_slice(&from[..count]);
        }
    }
    merged
}

// IDA: 007635C0 -> 00774550 for a cube that carries app pixel data.
// Completion: 100% above the device boundary.
fn create_cube_renderer_data(
    cube: &mut SourceTexture,
    data: &PixelData,
    services: &mut TextureLoadServices<'_>,
) -> bool {
    let Some(renderer) = services.renderer.as_deref_mut() else {
        return false;
    };
    let mut result = TextureRendererData::default();
    let mut scratch = TextureMemoryStatistics::default();
    let prefs = [
        cube.pixel_layout_18,
        cube.alpha_format_1c,
        cube.mip_mapped_20,
    ];
    let has_device = renderer.has_device();
    let statistics = services.statistics.as_deref_mut().unwrap_or(&mut scratch);
    let status = renderer.create_cube_texture(data, prefs, &mut result, statistics);
    if status != RendererTextureStatus::Created
        && !(status == RendererTextureStatus::DeviceFailure && !has_device)
    {
        return false;
    }
    cube.renderer_24 = Some(result);
    cube.renderer_data_24 = true;
    true
}

/// IDA: see the words here, which name the addresses this relates to.
/// 00701CD0 NiSourceTexture constructor (prefs +0x18..+0x20 overwritten).
fn new_source_texture(prefs: [u32; 3]) -> SourceTexture {
    SourceTexture {
        pixel_layout_18: prefs[0],
        alpha_format_1c: prefs[1],
        mip_mapped_20: prefs[2],
        ..SourceTexture::default()
    }
}

/// IDA: N/A spawns a finished texture; a C++ failure path deletes it, so nothing is spawned before the outcome is known.
fn spawn_source_texture(world: &mut World, texture: SourceTexture) -> Entity {
    world
        .spawn((NiClass::SourceTexture, NiObjectNet::default(), texture))
        .id()
}

// IDA: 00701FC0 NiSourceTexture_CreateFromPixelData(pixelData, prefs): a NiSourceTexture (00701CD0) with the prefs and the pixel
// data (00701FE4..00702064), the 00702064 00B256CC gate, 00701C00 (no renderer 00B3F928 fails; a null +0x24 calls renderer
// vt+0x104), then 00B3F958 && +0x24 && +0x40 static calls vt+0x58 007021B0, which releases +0x3C. A failure deletes the texture.
// Completion: 100% above the device boundary.
// TODO: [ ] 007021B0's reference drop: the caller's pixel data entity is not despawned here, because the caller may hold it. The
// C++ `shared_ptr` freed it once unreferenced.
pub fn create_texture_from_pixel_data(
    world: &mut World,
    pixel_data: Option<Entity>,
    prefs: [u32; 3],
    services: &mut TextureLoadServices<'_>,
) -> Option<Entity> {
    let mut texture = new_source_texture(prefs);
    texture.app_pixel_data_3c = pixel_data;
    texture.pixel_data_3c = pixel_data.is_some();
    if services.create_renderer_data_on_create {
        if !create_pixel_renderer_data(world, &mut texture, services) {
            return None;
        }
        if services.drop_static_app_data && texture.renderer_24.is_some() && texture.static_40 {
            texture.app_pixel_data_3c = None;
            texture.pixel_data_3c = false;
        }
    }
    Some(spawn_source_texture(world, texture))
}

// IDA: 00701C00 vt+0x5C on a file-named NiSourceTexture without renderer data (00701E00, and the stream PostLoad 007203F0 under
// 00B256CC): no renderer fails; renderer vt+0x104 00763560 -> 00761270 takes the 00760DA0 direct load of +0x38 when 00B3F709 and
// +0x41 allow it, else the image converter (0071E690) into the one-level path, releasing the converter's pixel data afterwards.
// Completion: 100% above the device boundary.
pub fn create_file_renderer_data(
    world: &mut World,
    texture: &mut SourceTexture,
    services: &mut TextureLoadServices<'_>,
) -> bool {
    if services.renderer.is_none() {
        return false; // 00701C0D: no renderer (00B3F928)
    }
    let name = texture.platform_filename_38.clone().unwrap_or_default();
    let prefs = [
        texture.pixel_layout_18,
        texture.alpha_format_1c,
        texture.mip_mapped_20,
    ];
    // 00761322..0076134B the direct load.
    let mut renderer = TextureRendererData::default();
    if services.direct_to_renderer_hint
        && texture.direct_to_renderer_41
        && load_texture_direct(&name, prefs, services, &mut renderer)
    {
        texture.renderer_24 = Some(renderer);
        texture.renderer_data_24 = true;
        return true;
    }
    // 00761357..00761367 the image converter, then the pixel-data path.
    let data = read_image_file(&name, services);
    texture.pixel_data_3c = data.is_some();
    texture.app_pixel_data_3c = data.map(|data| spawn_pixel_data(world, data));
    let Some(converted) = texture.app_pixel_data_3c else {
        return false;
    };
    if !create_pixel_renderer_data(world, texture, services) {
        return false;
    }
    // 007615BB..007615D5: the converter's pixel data is released (not +0x3C in the original).
    world.despawn(converted);
    texture.app_pixel_data_3c = None;
    texture.pixel_data_3c = false;
    true
}

// IDA: 00701E00 NiSourceTexture_CreateFromFile(path, prefs, directToRenderer): +0x18..+0x20 prefs, +0x41 the flag, +0x34 the name
// copy, +0x38 the platform name (0071B090), then with 00B256CC vt+0x5C 00701C00. Null when it fails (00701EBF deletes the texture).
// Completion: 100% above the device boundary.
pub fn create_texture_from_file(
    world: &mut World,
    path: &[u8],
    prefs: [u32; 3],
    direct_to_renderer: bool,
    services: &mut TextureLoadServices<'_>,
) -> Option<Entity> {
    let mut texture = new_source_texture(prefs);
    // 00701E4F..00701EA6 +0x41, +0x34, +0x38 (0071B090: the same C string).
    texture.direct_to_renderer_41 = direct_to_renderer;
    let name = c_prefix(path).to_vec();
    texture.filename_34 = Some(name.clone());
    if name.len() >= TEXTURE_PATH_BUFFER {
        return None;
    }
    texture.platform_filename_38 = Some(name);
    // 00701EA9 00B256CC gate.
    if services.create_renderer_data_on_create
        && !create_file_renderer_data(world, &mut texture, services)
    {
        if let Some(converted) = texture.app_pixel_data_3c {
            world.despawn(converted);
        }
        return None;
    }
    Some(spawn_source_texture(world, texture))
}

// IDA: 00720A30 NiSourceCubeMap from six pixel data objects: 00720A52..00720AAA NiSourceCubeMap (0x4C): 00701CD0, vtable A7F644, +0x48
// = 0, prefs; 00720AC3 007205A0, then 00720AC8 +0x48 cleared again when set; 00720AD5..00720B09 with a renderer, vt+0x10C (007635C0); a
// failure deletes the cube. A null renderer returns the cube without renderer data. 00720A30 does not call 007021B0.
// Completion: 100% above the device boundary.
pub fn create_cube_from_pixel_data(
    world: &mut World,
    faces: [Option<Entity>; 6],
    prefs: [u32; 3],
    services: &mut TextureLoadServices<'_>,
) -> Option<Entity> {
    // Host guard: 007205A0 dereferences every face (a null face would fault).
    let mut sources = Vec::with_capacity(6);
    for face in faces {
        sources.push(world.get::<PixelData>(face?)?);
    }
    let merged = merge_cube_faces(sources.try_into().ok()?);
    finish_cube(world, new_cube(prefs), Some(merged), services)
}

// IDA: N/A a helper, with no address of its own.
fn new_cube(prefs: [u32; 3]) -> SourceTexture {
    let mut cube = new_source_texture(prefs);
    cube.original_vtable = 0x00A7_F644;
    cube.cube_48 = 0;
    cube
}

// IDA: N/A a helper, with no address of its own.
fn finish_cube(
    world: &mut World,
    mut cube: SourceTexture,
    merged: Option<PixelData>,
    services: &mut TextureLoadServices<'_>,
) -> Option<Entity> {
    cube.cube_48 = 0;
    if services.renderer.is_some() {
        let data = merged.as_ref()?;
        if !create_cube_renderer_data(&mut cube, data, services) {
            return None;
        }
    }
    if let Some(merged) = merged {
        cube.app_pixel_data_3c = Some(spawn_pixel_data(world, merged));
        cube.pixel_data_3c = true;
    }
    Some(spawn_source_texture(world, cube))
}

// IDA: 00720F80 NiSourceCubeMap from six file names: 00720FF0..00721057 NiSourceCubeMap (0x4C) with the prefs, then a base NiSearchPath
// (0072105A); 00720B40 per name: copy, 0071B090, SetFilePath, Reset, then GetNextSearchPath until g_bsFileExistsCallback(candidate, 0)
// accepts one; the default converter's ReadImageFile(candidate, 0) fills the face. The first face without data stops the walk before
// 007205A0. 00721088..007210D4 +0x48 cleared; with a renderer, a cube without pixel data (+0x3C) or a failed vt+0x10C is deleted and
// null returned.
// Completion: 100% above the device boundary.
pub fn create_cube_from_files(
    world: &mut World,
    names: [&[u8]; 6],
    prefs: [u32; 3],
    services: &mut TextureLoadServices<'_>,
) -> Option<Entity> {
    let mut faces = Vec::with_capacity(6);
    for name in names {
        let mut face = None;
        for candidate in base_search_candidates(c_prefix(name)) {
            if services
                .file_exists
                .as_mut()
                .is_some_and(|exists| exists(&candidate))
            {
                face = read_image_file(&candidate, services);
                break;
            }
        }
        match face {
            Some(face) => faces.push(face),
            None => break,
        }
    }
    let merged = (faces.len() == 6).then(|| {
        merge_cube_faces([
            &faces[0], &faces[1], &faces[2], &faces[3], &faces[4], &faces[5],
        ])
    });
    finish_cube(world, new_cube(prefs), merged, services)
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::path::{Path, PathBuf};

    use super::*;
    use crate::bs::effect_textures::load_shader_texture;
    use crate::bs::file::{BsFile, NiFileMode};

    struct TempRoot(PathBuf);

    impl TempRoot {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!("recomp_{name}_{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// IDA: N/A fixture: a DDS header (124 bytes after "DDS ") followed by data.
    #[derive(Clone, Copy)]
    struct DdsSpec {
        flags: u32,
        height: u32,
        width: u32,
        depth: u32,
        mip_count: u32,
        pf_flags: u32,
        four_cc: u32,
        caps1: u32,
        caps2: u32,
        size: u32,
        swapped: bool,
    }

    impl Default for DdsSpec {
        fn default() -> Self {
            Self {
                flags: 0x1007 | 0x2_0000,
                height: 4,
                width: 4,
                depth: 0,
                mip_count: 1,
                pf_flags: 4,
                four_cc: FOUR_CC_DXT1,
                caps1: 0x1000,
                caps2: 0,
                size: 124,
                swapped: false,
            }
        }
    }

    fn dds_bytes(spec: DdsSpec, data_bytes: usize) -> Vec<u8> {
        let mut out = Vec::new();
        let mut put = |value: u32| {
            let value = if spec.swapped {
                value.swap_bytes()
            } else {
                value
            };
            out.extend(value.to_le_bytes());
        };
        put(0x2053_4444);
        put(spec.size);
        put(spec.flags);
        put(spec.height);
        put(spec.width);
        put(0); // pitch
        put(spec.depth);
        put(spec.mip_count);
        for _ in 0..11 {
            put(0);
        }
        put(32);
        put(spec.pf_flags);
        put(spec.four_cc);
        for _ in 0..5 {
            put(0);
        }
        put(spec.caps1);
        put(spec.caps2);
        for _ in 0..3 {
            put(0);
        }
        for index in 0..data_bytes {
            out.push((index as u32).wrapping_mul(7).wrapping_add(1) as u8);
        }
        out
    }

    fn read_named(root: &Path, name: &str) -> (DdsReadStatus, Option<PixelData>) {
        let mut file = BsFile::new(root, name, NiFileMode::ReadOnly, 0);
        read_dds_file(&mut file)
    }

    /// Not in the C++ tests: the D3DX stand-ins over a hand-built file. The five real files are `tests/effect_textures_real.rs`.
    #[test]
    fn direct_answers_come_from_the_file() {
        let bytes = |width: u32, height: u32, mips: u32, four_cc: u32, caps2: u32| {
            let mut file = vec![0u8; 0x80];
            file[..4].copy_from_slice(b"DDS ");
            let mut put =
                |at: usize, value: u32| file[at..at + 4].copy_from_slice(&value.to_le_bytes());
            put(4, 124);
            put(8, 0x0002_1007);
            put(12, height);
            put(16, width);
            put(0x1C, mips);
            put(0x50, 4);
            put(0x54, four_cc);
            put(0x6C, 0x0040_1008);
            put(0x70, caps2);
            file
        };
        // A 16x8 DXT5 with its whole chain.
        let file = bytes(16, 8, 5, FOUR_CC_DXT5, 0);
        let info = direct_image_info(&file).unwrap();
        assert_eq!(
            info,
            DirectImageInfo {
                width: 16,
                height: 8,
                depth: 1,
                format: FOUR_CC_DXT5,
                resource_type: DirectResourceType::Texture,
                image_file_format: 4,
            }
        );
        let levels = direct_texture_levels(&direct_file_facts(&file).unwrap()).unwrap();
        let sizes: Vec<(u32, u32)> = levels
            .iter()
            .map(|level| (level.width, level.height))
            .collect();
        assert_eq!(sizes, [(16, 8), (8, 4), (4, 2), (2, 1), (1, 1)]);
        assert!(
            levels
                .iter()
                .all(|level| level.format == FOUR_CC_DXT5 && level.depth == 1)
        );
        // A cube says so.
        let cube = direct_image_info(&bytes(8, 8, 4, FOUR_CC_DXT1, 0xFE00)).unwrap();
        assert_eq!(cube.resource_type, DirectResourceType::CubeTexture);
        // D3DX_DEFAULT generates the rest of a short chain. Host stops: what D3DX would have to round, and what the header reader refuses.
        let short = direct_file_facts(&bytes(16, 16, 2, FOUR_CC_DXT1, 0)).unwrap();
        assert_eq!(direct_texture_levels(&short).unwrap().len(), 5);
        let odd = direct_file_facts(&bytes(15, 16, 5, FOUR_CC_DXT1, 0)).unwrap();
        assert_eq!(direct_texture_levels(&odd), None, "D3DX rounds the size");
        assert_eq!(direct_image_info(&bytes(16, 16, 5, FOUR_CC_DXT2, 0)), None);
        assert_eq!(direct_image_info(b"BM not a dds"), None);
    }

    /// Ported from `verify_dds_reader` in `tests/startup/phase05_tests.cpp`. IDA: 007363F0 / 00737330 NiDDSReader.
    #[test]
    fn dds_reader() {
        let temp = TempRoot::new("texture_dds");
        let root = &temp.0;
        let write = |name: &str, spec: DdsSpec, data: usize| {
            std::fs::write(root.join(name), dds_bytes(spec, data)).unwrap();
        };
        // DXT1 64x64 with seven levels (COMPLEX | TEXTURE | MIPMAP).
        let mut dxt1 = DdsSpec {
            width: 64,
            height: 64,
            mip_count: 7,
            caps1: 0x0040_1008,
            ..DdsSpec::default()
        };
        write("dxt1.dds", dxt1, 2048 + 512 + 128 + 32 + 8 + 8 + 8);
        let (status, mipped) = read_named(root, "dxt1.dds");
        let mipped = mipped.unwrap();
        assert_eq!(status, DdsReadStatus::Read);
        assert_eq!(mipped.levels_60, 7);
        assert_eq!(mipped.format_08.source_address, 0x00B2_5FB0);
        assert_eq!(mipped.format_08.format_04, 4);
        assert_eq!(mipped.format_08.renderer_hint_0c, 0xFFFF_FFFF);
        assert_eq!(mipped.widths_54[0], 64);
        assert_eq!(mipped.level(0, 0)[0], 1);
        assert_eq!(u32::from(mipped.level(0, 1)[0]), (2048 * 7 + 1) & 0xFF);
        // MIPMAP without COMPLEX keeps one level.
        dxt1.caps1 = 0x0040_1000;
        write("flat.dds", dxt1, 2048);
        let (status, single) = read_named(root, "flat.dds");
        assert_eq!(status, DdsReadStatus::Read);
        assert_eq!(single.unwrap().levels_60, 1);
        // A DXT5 cube map with all six faces.
        let cube = DdsSpec {
            four_cc: FOUR_CC_DXT5,
            caps1: 0x1008,
            caps2: 0xFE00,
            ..DdsSpec::default()
        };
        write("cube.dds", cube, 6 * 16);
        let (status, faces) = read_named(root, "cube.dds");
        let faces = faces.unwrap();
        assert_eq!(status, DdsReadStatus::Read);
        assert_eq!(faces.faces_6c, 6);
        assert_eq!(faces.format_08.source_address, 0x00B2_6040);
        assert_eq!(u32::from(faces.level(5, 0)[0]), (80 * 7 + 1) & 0xFF);
        // The float FourCC 116 maps to src__23.
        write(
            "float.dds",
            DdsSpec {
                four_cc: 116,
                ..DdsSpec::default()
            },
            256,
        );
        let (status, data) = read_named(root, "float.dds");
        assert_eq!(status, DdsReadStatus::Read);
        assert!(data.is_some());
        // A byte-swapped header turns endian swapping on.
        write(
            "swapped.dds",
            DdsSpec {
                swapped: true,
                ..dxt1
            },
            2048,
        );
        let (status, big) = read_named(root, "swapped.dds");
        assert_eq!(status, DdsReadStatus::Read);
        assert_eq!(big.unwrap().widths_54[0], 64);
        // Rejections.
        let rejects = |name: &str, spec: DdsSpec| {
            write(name, spec, 64);
            let (status, data) = read_named(root, name);
            data.is_none() && status == DdsReadStatus::Rejected
        };
        assert!(
            rejects(
                "partial.dds",
                DdsSpec {
                    caps2: 0xBE00,
                    ..cube
                }
            ),
            "no -X face"
        );
        assert!(rejects(
            "volume.dds",
            DdsSpec {
                caps2: 0x20_0000,
                ..dxt1
            }
        ));
        assert!(
            rejects(
                "dxt2.dds",
                DdsSpec {
                    four_cc: FOUR_CC_DXT2,
                    ..dxt1
                }
            ),
            "DXT2 is not in the 007366xx switch"
        );
        assert!(rejects("size.dds", DdsSpec { size: 123, ..dxt1 }));
        assert!(rejects(
            "flags.dds",
            DdsSpec {
                flags: 0x1003,
                ..dxt1
            }
        ));
        assert!(rejects("caps.dds", DdsSpec { caps1: 2, ..dxt1 }));
        assert!(rejects(
            "depth.dds",
            DdsSpec {
                flags: dxt1.flags | 0x80_0000,
                depth: 2,
                ..dxt1
            }
        ));
        write(
            "rgb.dds",
            DdsSpec {
                pf_flags: 0x40,
                ..dxt1
            },
            64,
        );
        // 00736769 007362C0 takes only 16, 24 and 32 bits, and this file's count is 0. A 32-bit one is `rgb_files_read_for_d3dx`.
        let (status, data) = read_named(root, "rgb.dds");
        assert!(data.is_none());
        assert_eq!(status, DdsReadStatus::Rejected);
    }

    /// 007366FD..: the A8R8G8B8 form of cursor.dds and the dialog bars: flags 0x41, no FourCC. `direct_image_info` answered None for them before this branch read the masks and caps.
    #[test]
    fn rgb_files_read_for_d3dx() {
        let mut bytes = vec![0u8; 128];
        bytes[..4].copy_from_slice(b"DDS ");
        for (at, value) in [
            (4usize, 124u32),
            (8, 0x000A_1007),
            (12, 64),
            (16, 64),
            (28, 7),
            (76, 32),
            (80, 0x41),
            (88, 32),
            (92, 0x00FF_0000),
            (96, 0x0000_FF00),
            (100, 0x0000_00FF),
            (104, 0xFF00_0000),
            (108, 0x0040_1008),
        ] {
            bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
        let facts = direct_file_facts(&bytes).unwrap();
        assert_eq!((facts.width, facts.height, facts.levels), (64, 64, 7));
        assert_eq!(facts.format, 21, "D3DFMT_A8R8G8B8");
        assert_eq!(direct_image_info(&bytes).unwrap().format, 21);
        // No flag 1: the alpha mask is dropped, and it's X8R8G8B8.
        bytes[80..84].copy_from_slice(&0x40u32.to_le_bytes());
        assert_eq!(direct_file_facts(&bytes).unwrap().format, 22);
    }

    /// The device boundary with no device: `RendererState no_device` in the C++ test.
    struct NoDevice;

    impl TextureRendererBoundary for NoDevice {
        fn has_device(&self) -> bool {
            false
        }
        fn upload_source_texture(
            &mut self,
            format: &PixelFormat,
            _width: u32,
            _height: u32,
            _texels: &[u8],
            _prefs: [u32; 3],
        ) -> SourceTextureUpload {
            SourceTextureUpload {
                table_case: None,
                table_format: *format,
                d3d_format: 0,
                device_texture: false,
            }
        }
        fn create_cube_texture(
            &mut self,
            _data: &PixelData,
            _prefs: [u32; 3],
            _out: &mut TextureRendererData,
            _statistics: &mut TextureMemoryStatistics,
        ) -> RendererTextureStatus {
            RendererTextureStatus::DeviceFailure
        }
        fn image_info(&mut self, _bytes: &[u8]) -> Option<DirectImageInfo> {
            None
        }
        fn create_direct_texture(
            &mut self,
            _resource_type: DirectResourceType,
            _bytes: &[u8],
            _original_size: u32,
        ) -> Option<Vec<Option<DirectLevelDesc>>> {
            None
        }
    }

    fn flat_pixel_data(size: u32, format: &PixelFormat, texel: [u8; 4]) -> PixelData {
        let mut data = construct_pixel_data(size, size, format, 1, 1);
        for chunk in data.level_mut(0, 0).as_chunks_mut::<4>().0 {
            *chunk = texel;
        }
        data.revision_68 += 1;
        data
    }

    /// Ported from `verify_texture_loading_paths` in `tests/startup/phase05_tests.cpp`: 0071E690, 00720A30,
    /// 00720F80/00720B40 (base NiSearchPath walk) and 007B8200 without a renderer or device.
    #[test]
    fn texture_loading_paths() {
        let temp = TempRoot::new("texture_load");
        let root = temp.0.clone();
        let face = DdsSpec {
            four_cc: FOUR_CC_DXT5,
            ..DdsSpec::default()
        };
        std::fs::write(root.join("face.dds"), dds_bytes(face, 16)).unwrap();
        let mut world = World::new();

        let open_root = root.clone();
        let open_file = move |path: &[u8]| -> Option<Box<dyn GameFile>> {
            let text = String::from_utf8(path.to_vec()).unwrap();
            let name = text.rsplit('\\').next().unwrap().to_owned();
            if !open_root.join(&name).exists() {
                return None;
            }
            Some(Box::new(BsFile::new(
                &open_root,
                &name,
                NiFileMode::ReadOnly,
                0,
            )))
        };
        let probed: RefCell<Vec<Vec<u8>>> = RefCell::new(Vec::new());
        let added: RefCell<Vec<(Vec<u8>, bool)>> = RefCell::new(Vec::new());
        let palette_texture: Cell<Option<Entity>> = Cell::new(None);
        let exists_calls = Cell::new(0);
        let warning: RefCell<Vec<u8>> = RefCell::new(Vec::new());
        let mut no_device = NoDevice;
        let mut services = TextureLoadServices {
            open_file: Some(Box::new(open_file)),
            file_exists: Some(Box::new(|path| {
                probed.borrow_mut().push(path.to_vec());
                path == b"face.dds"
            })),
            ..TextureLoadServices::default()
        };
        assert!(read_image_file(b"face.dds", &mut services).is_some());
        assert!(
            read_image_file(b"face.tga", &mut services).is_none(),
            "0071E690 reads .dds only (the other readers are carried)"
        );

        // 00720B40: candidates 0 (the path) then 1 (name only).
        let effects: &[u8] = b"Data\\Textures\\Effects\\face.dds";
        let names = [effects; 6];
        let cube = create_cube_from_files(&mut world, names, [6, 3, 2], &mut services).unwrap();
        let cube = world.get::<SourceTexture>(cube).unwrap();
        assert_eq!(cube.original_vtable, 0x00A7_F644);
        assert!(cube.renderer_24.is_none());
        let data = world
            .get::<PixelData>(cube.app_pixel_data_3c.unwrap())
            .unwrap();
        assert_eq!(data.faces_6c, 6);
        {
            let probed = probed.borrow();
            assert_eq!(probed.len(), 12);
            assert_eq!(probed[0], effects);
            assert_eq!(probed[1], b"face.dds");
        }
        services.file_exists = Some(Box::new(|_| false));
        let empty = create_cube_from_files(&mut world, names, [6, 3, 2], &mut services).unwrap();
        assert!(
            world
                .get::<SourceTexture>(empty)
                .unwrap()
                .app_pixel_data_3c
                .is_none(),
            "without a renderer a cube with a missing face is returned without data"
        );

        // 00720A30 + 007205A0: face i takes source i's level 0.
        let rgba32 = d3d_pixel_format_static(0x00B2_5E00).unwrap();
        let sources: [Entity; 6] = std::array::from_fn(|i| {
            spawn_pixel_data(&mut world, flat_pixel_data(4, &rgba32, [i as u8, 1, 2, 3]))
        });
        let merged =
            create_cube_from_pixel_data(&mut world, sources.map(Some), [6, 3, 2], &mut services)
                .unwrap();
        let merged = world.get::<SourceTexture>(merged).unwrap();
        assert_eq!(merged.cube_48, 0);
        let data = world
            .get::<PixelData>(merged.app_pixel_data_3c.unwrap())
            .unwrap();
        assert_eq!(data.faces_6c, 6);
        assert_eq!(data.level(3, 0)[0], 3);
        assert_eq!(data.revision_68, 1);

        // 00701FC0 without a renderer: the 00B256CC gate, then 00701C00 fails.
        services.create_renderer_data_on_create = false;
        let gated =
            create_texture_from_pixel_data(&mut world, Some(sources[0]), [6, 3, 2], &mut services)
                .unwrap();
        let gated = world.get::<SourceTexture>(gated).unwrap();
        assert_eq!(gated.app_pixel_data_3c, Some(sources[0]));
        assert!(gated.renderer_24.is_none());
        services.create_renderer_data_on_create = true;
        assert!(
            create_texture_from_pixel_data(&mut world, Some(sources[0]), [6, 3, 2], &mut services)
                .is_none(),
            "00701C00 fails without a renderer"
        );

        // 05-D-6: a missing effect file with renderer data on create: 00701EBF deletes the texture.
        services.renderer = Some(&mut no_device);
        assert!(
            create_texture_from_file(
                &mut world,
                b"Data\\Textures\\Effects\\missing.dds",
                [6, 3, 2],
                true,
                &mut services
            )
            .is_none(),
            "00701E00 deletes a texture whose file is missing"
        );
        services.renderer = None;

        // 007B8200 palette semantics.
        services.palette_find = Some(Box::new(|_| palette_texture.get()));
        services.palette_add = Some(Box::new(|path, texture| {
            added.borrow_mut().push((path.to_vec(), texture.is_some()));
        }));
        services.file_exists = Some(Box::new(|_| {
            exists_calls.set(exists_calls.get() + 1);
            true
        }));
        assert!(load_shader_texture(&mut world, b"x.dds", false, false, &mut services).is_none());
        assert_eq!(
            exists_calls.get(),
            0,
            "usePalette 0 with a palette keeps the lookup"
        );
        services.create_renderer_data_on_create = false;
        let loaded =
            load_shader_texture(&mut world, b"Data\\x.dds", true, true, &mut services).unwrap();
        assert_eq!(exists_calls.get(), 1);
        assert_eq!(added.borrow().as_slice(), [(b"Data\\x.dds".to_vec(), true)]);
        let loaded = world.get::<SourceTexture>(loaded).unwrap();
        assert_eq!(loaded.filename_34.as_deref(), Some(&b"Data\\x.dds"[..]));
        assert_eq!(loaded.pixel_layout_18, 6);
        assert_eq!(loaded.alpha_format_1c, 3);
        assert_eq!(loaded.mip_mapped_20, 2);
        services.create_renderer_data_on_create = true;
        assert!(
            load_shader_texture(&mut world, b"Data\\y.dds", true, true, &mut services).is_none()
        );
        assert_eq!(added.borrow().len(), 2);
        assert!(
            !added.borrow()[1].1,
            "a failed creation is still added (vt+8 with null)"
        );

        // The mipmap requirement: a palette texture whose renderer data has 1 level.
        let held = world
            .spawn(SourceTexture {
                platform_filename_38: Some(b"Data\\one.dds".to_vec()),
                renderer_24: Some(TextureRendererData {
                    levels_5c: 1,
                    ..TextureRendererData::default()
                }),
                ..SourceTexture::default()
            })
            .id();
        palette_texture.set(Some(held));
        services.warning = Some(Box::new(|text| *warning.borrow_mut() = text.to_vec()));
        assert!(load_shader_texture(&mut world, b"one.dds", true, true, &mut services).is_none());
        assert_eq!(
            warning.borrow().as_slice(),
            &b"TEXTURE ERROR : texture does not contain mipmaps & will not be used : Data\\one.dds"
                [..]
        );
        world
            .get_mut::<SourceTexture>(held)
            .unwrap()
            .renderer_24
            .as_mut()
            .unwrap()
            .levels_5c = 2;
        assert_eq!(
            load_shader_texture(&mut world, b"one.dds", true, true, &mut services),
            Some(held),
            "a mipmapped palette texture"
        );
        world
            .get_mut::<SourceTexture>(held)
            .unwrap()
            .renderer_24
            .as_mut()
            .unwrap()
            .levels_5c = 1;
        assert_eq!(
            load_shader_texture(&mut world, b"one.dds", true, false, &mut services),
            Some(held),
            "requireMipmaps 0 skips the check"
        );
    }

    /// New: the C++ tests reach no device-free assertion for the 00760FC9..0076105F arithmetic. Hand-computed: DXT1 16x16
    /// with 3 levels, skip 1, minimum 1 keeps 2; one dropped level of (4*4*16)/2 = 128 bytes.
    #[test]
    fn dds_mip_skip_arithmetic() {
        let mut bytes = dds_bytes(
            DdsSpec {
                width: 16,
                height: 16,
                mip_count: 3,
                ..DdsSpec::default()
            },
            128 + 32 + 8,
        );
        let total = bytes.len();
        let kept_first = bytes[0x80 + 128];
        let mut info = DirectImageInfo {
            width: 16,
            height: 16,
            depth: 1,
            format: FOUR_CC_DXT1,
            resource_type: DirectResourceType::Texture,
            image_file_format: 4,
        };
        assert!(dds_trim_mip_levels(&mut bytes, &mut info, 1, 1));
        assert_eq!((info.width, info.height), (8, 8));
        assert_eq!(bytes.len(), total, "the original size stays");
        let dword = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
        assert_eq!(dword(0x10), 8);
        assert_eq!(dword(0x0C), 8);
        assert_eq!(dword(0x1C), 2);
        assert_eq!(dword(0x14), 64);
        assert_eq!(bytes[0x80], kept_first);
        assert_eq!(*bytes.last().unwrap(), 0, "the zero tail");
        assert_eq!(page_padding(0x1000), 0);
        assert_eq!(page_padding(0x1001), 0xFFF);
    }

    /// New: 007609C0's accounting over level answers. 4x4 DXT1 has bits 0; A8R8G8B8 2x2 with two levels is 16 + 4.
    #[test]
    fn level_desc_accounting() {
        let level = |format, width, height| {
            Some(DirectLevelDesc {
                format,
                width,
                height,
                depth: 1,
            })
        };
        let mut data = TextureRendererData::default();
        let levels = [level(21, 2, 2), level(21, 1, 1)];
        assert!(initialize_from_level_descs(
            &mut data,
            DirectResourceType::Texture,
            &levels
        ));
        assert_eq!(data.levels_5c, 2);
        assert!(data.mipmap_65 && !data.cube && data.device_texture);
        assert_eq!(data.pixel_bytes_68, 20);
        let mut cube = TextureRendererData::default();
        assert!(initialize_from_level_descs(
            &mut cube,
            DirectResourceType::CubeTexture,
            &levels[..1]
        ));
        assert_eq!(cube.pixel_bytes_68, 96);
        assert!(cube.cube && !cube.mipmap_65);
        let mut paletted = TextureRendererData::default();
        assert!(!initialize_from_level_descs(
            &mut paletted,
            DirectResourceType::Texture,
            &[level(D3DFMT_P8, 1, 1)]
        ));
        assert!(direct_format_accepted(6, 21));
        assert!(!direct_format_accepted(4, 21));
        assert!(direct_format_accepted(4, 0x3C));
    }
}
