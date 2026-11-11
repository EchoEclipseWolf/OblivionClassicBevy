//! Phase 05 row 05-D: 007BA0F0 BSShaderManager initialization, called from 00406950 at 00406A83, with the shader
//! package and the accumulator singleton. 00B42F50 is the `TextureManager` of `bs::render_targets`. The effect textures,
//! slots and image-space list are injected steps, as in the C++ port, and `crate::tes::winmain::winmain_scene` binds them.

use std::sync::Arc;

use bevy_ecs::prelude::{Resource, World};
use oblivion_core::crt::stricmp;
use oblivion_core::error::GameError;

use crate::bs::file::GameFile;
use crate::bs::render_targets::{TextureManager, TextureManagerStatus, construct_texture_manager};
use crate::gamebryo::render_targets::RenderMemoryStatistics;
use crate::gamebryo::renderer::{RendererGlobals, RendererState, shader_package};
#[cfg(test)]
use crate::tes::winmain::load_test_sphere;

// IDA: one entry of a shader package body: name[256], u32 size at +0x100,
// data at +0x104 (007DAD69 advances by size + 0x104).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShaderPackageEntry {
    pub name: Vec<u8>,
    pub size: u32,
    pub data_offset: usize,
}

// IDA: 007DB050 ShaderPackage constructor (0x1C bytes): +0x00 file name (a
// FormHeap copy of "shaderpackage%03i.sdp" when the index is above 0), +0x04
// body, +0x08 NiTStringPointerMap<ShaderBufferEntry*> (37 buckets, key copy
// byte +0x18 = 1). 007DACA0 loads, 007DAC70 looks up, 007DB010 destroys.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShaderPackage {
    file_name: Option<String>,
    body: Option<Vec<u8>>,
    // Buckets in chain order (InsertOrAssign links new nodes at the head).
    buckets: Vec<Vec<ShaderPackageEntry>>,
}

impl ShaderPackage {
    pub const BUCKET_COUNT: usize = 37;

    // IDA: 007DB050(index). Completion: 100% fields.
    #[must_use]
    pub fn new(index: i32) -> Self {
        Self {
            // 007DB0D2..007DB13C: index > 0 -> sprintf("shaderpackage%03i.sdp").
            file_name: (index > 0).then(|| format!("shaderpackage{index:03}.sdp")),
            body: None,
            buckets: vec![Vec::new(); Self::BUCKET_COUNT],
        }
    }

    // IDA: 007DAD56..007DAD7D insert each entry, advancing by size + 0x104.
    fn insert_entries(&mut self, count: u32) {
        let bytes = self.body.as_deref().unwrap_or_default();
        let mut parsed = Vec::new();
        let mut offset = 0usize;
        for _ in 0..count {
            if offset + 0x104 > bytes.len() {
                break; // host: the original reads past the body
            }
            let field = &bytes[offset..offset + 0x100];
            let name = &field[..field.iter().position(|&b| b == 0).unwrap_or(0x100)];
            let length = u32::from_le_bytes([
                bytes[offset + 0x100],
                bytes[offset + 0x101],
                bytes[offset + 0x102],
                bytes[offset + 0x103],
            ]);
            parsed.push(ShaderPackageEntry {
                name: name.to_vec(),
                size: length,
                data_offset: offset + 0x104,
            });
            offset += length as usize + 0x104;
        }
        for entry in parsed {
            // 007DAED0 NiTStringPointerMap_CalculateBucket, modulo the bucket count.
            let bucket = &mut self.buckets
                [oblivion_core::hash::ni_string_hash(&entry.name) as usize % Self::BUCKET_COUNT];
            if let Some(found) = bucket
                .iter_mut()
                .find(|item| stricmp(&item.name, &entry.name) == 0)
            {
                // 00412D9C: the copy-key map keeps the first key.
                found.size = entry.size;
                found.data_offset = entry.data_offset;
            } else {
                // 00412D83..00412D8E: link at the bucket head.
                bucket.insert(0, entry);
            }
        }
    }

    // IDA: 007DACA0(name): null name -> false; BSFile_OpenAsBSFile(
    // "\Data\Shaders\%s", 0, 0); the 12-byte header {version, count, size}
    // read with component size 1 must return 12 bytes and version 100; then
    // FormHeap(size) into +0x04 and one read of `size` bytes (a short read
    // frees it and stores null); the result is true once the body read
    // completes, and each of `count` entries is inserted (InsertOrAssign:
    // an equal key keeps its first copied key and takes the new value). The
    // file is deleted on every path. 100%; reading `count` entries past the
    // body is not bounds-checked in the original (the host stops instead).
    pub fn load(&mut self, open: &mut dyn FnMut(&str) -> Option<Box<dyn GameFile>>) -> bool {
        // 007DACC3: a null name returns false.
        let Some(file_name) = &self.file_name else {
            return false;
        };
        // 007DACD1..007DACE7 "\Data\Shaders\%s" through 00431690(path, 0, 0).
        let Some(mut file) = open(&format!("\\Data\\Shaders\\{file_name}")) else {
            return false;
        };
        let mut loaded = false;
        // 007DACF9..007DAD23: 12-byte header, component size 1, version 100.
        let mut header = [0u8; 12];
        let header_bytes = file.read_components(&mut header, Some(&[1]));
        let word = |index: usize| {
            u32::from_le_bytes([
                header[index],
                header[index + 1],
                header[index + 2],
                header[index + 3],
            ])
        };
        if header_bytes == 12 && word(0) == 100 {
            let count = word(4);
            let size = word(8);
            // 007DAD25..007DAD46 FormHeap(size) into +0x04 and one read.
            let mut body = vec![0; size as usize];
            let read = file.read_components(&mut body, Some(&[1]));
            if read == size as usize {
                self.body = Some(body);
                loaded = true; // 007DAD4F
                self.insert_entries(count);
            }
            // 007DAD81..007DAD8D: a short read leaves +0x04 null.
        }
        // 007DAD90..007DAD98: the file is deleted on every path.
        drop(file);
        loaded
    }

    // IDA: 007DAC70 lookup(key): 007DAED0 raw signed-byte hash (h = 33h + c)
    // modulo 37, _stricmp within the bucket; null when absent. Completion: 100%.
    #[must_use]
    pub fn find(&self, name: &str) -> Option<&ShaderPackageEntry> {
        let key = name.as_bytes();
        self.buckets[oblivion_core::hash::ni_string_hash(key) as usize % Self::BUCKET_COUNT]
            .iter()
            .find(|entry| stricmp(&entry.name, key) == 0)
    }

    /// IDA: N/A
    #[must_use]
    pub fn file_name(&self) -> Option<&str> {
        self.file_name.as_deref()
    }

    /// IDA: N/A
    #[must_use]
    pub const fn has_body(&self) -> bool {
        self.body.is_some()
    }

    /// IDA: N/A an entry's bytes, clipped to the body.
    #[must_use]
    pub fn data(&self, entry: &ShaderPackageEntry) -> &[u8] {
        let bytes = self.body.as_deref().unwrap_or_default();
        if entry.data_offset > bytes.len() {
            return &[];
        }
        let available = bytes.len() - entry.data_offset;
        &bytes[entry.data_offset..entry.data_offset + (entry.size as usize).min(available)]
    }

    /// IDA: N/A map +0x0C (+0x14 of the object).
    #[must_use]
    pub fn entry_count(&self) -> usize {
        self.buckets.iter().map(Vec::len).sum()
    }

    #[cfg(test)]
    pub(crate) fn from_entries_for_test(entries: &[(&str, &[u8])]) -> Self {
        let mut package = Self::new(1);
        package.body = Some(package_body(entries));
        package.insert_entries(entries.len() as u32);
        package
    }
}

/// IDA: N/A test support: a package body, as `write_package` builds one.
#[cfg(test)]
#[must_use]
pub fn package_body(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut body = Vec::new();
    for (name, data) in entries {
        let mut key = [0u8; 0x100];
        key[..name.len()].copy_from_slice(name.as_bytes());
        body.extend_from_slice(&key);
        body.extend_from_slice(&(data.len() as u32).to_le_bytes());
        body.extend_from_slice(data);
    }
    body
}

// IDA: one of the three 0x14-byte records at +0xC8 that 007AC240 clears:
// +0x00 dword, +0x04 byte, +0x08 float 0.0, +0x0C and +0x10 dwords.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ShaderAccumulatorRecord {
    pub dword_00: u32,
    pub byte_04: u8,
    pub float_08: f32,
    pub dword_0c: u32,
    pub dword_10: u32,
}

// IDA: 007ABE30 BSShaderAccumulator constructor (0x226C bytes, vtable
// A8CC5C) over NiAlphaAccumulator 0071A8E0 (+0x34 = 1, +0x35 = 0),
// NiBackToFrontAccumulator 00733710 (+0x0C NiTPointerList<NiGeometry*>,
// +0x1C..+0x2C zero) and NiAccumulator 00733810 (+0x08 = 0). The lists at
// +0x0C, +0x3C, +0x4C (GeometryGroup), +0x64 (ImmediateGeometryGroup),
// +0x7C/+0x90/+0xA4 (BSTPersistentList<RenderPass*>), the 419 render-pass
// lists at +0x104 (0x14 bytes each, 007A9AC0), +0x21C0 (RenderPass*),
// +0x21D0 (ShadowVolumeRPList*), +0x21EC/+0x2200/+0x2214
// (BSTPersistentList<NiGeometry*>), +0x222C (ReferenceVolume*) and
// +0x2244/+0x2254 (NiGeometry*) all start empty. Field names follow offsets;
// their roles stay unresolved until the render passes are ported. +0x30 is
// never written. Completion: 100% cold state.
#[derive(Clone, Debug, PartialEq)]
pub struct ShaderAccumulator {
    pub original_vtable: u32,
    pub dword_08: u32,
    pub dwords_1c: [u32; 5],
    pub byte_34: u8,
    pub byte_35: u8,
    pub dword_5c: u32,
    pub dword_60: u32,
    pub dword_74: u32,
    pub dword_78: u32,
    pub dword_bc: u32,
    pub byte_c0: u8,
    pub float_c4: f32,
    pub records_c8: [ShaderAccumulatorRecord; 3],
    pub render_pass_list_counts: Vec<u32>,
    pub bytes_21e0: [u8; 4],
    pub word_21e4: u16,
    pub word_21e6: u16,
    pub pointers_21e8: Vec<u32>, // FormHeap(0xC8), zeroed
    pub dword_2228: u32,
    pub dword_223c: u32,
    pub byte_2240: u8,
    // +0x2264: FormHeap(4) holding FormHeap(4 * g_niD3DMaxTextureBlendStages)
    // (007F5B50 leaves the array unset; the host zero-fills it).
    pub blend_stage_slots_2264: Vec<u32>,
    pub byte_2268: u8,
}

impl ShaderAccumulator {
    pub const RENDER_PASS_LISTS: usize = 0x1A3;

    fn new(max_texture_blend_stages: u32) -> Self {
        Self {
            original_vtable: 0x00A8_CC5C,
            dword_08: 0,
            dwords_1c: [0; 5],
            byte_34: 1,
            byte_35: 0,
            dword_5c: 0,
            dword_60: 0,
            dword_74: 0,
            dword_78: 0,
            dword_bc: 0,
            byte_c0: 0,
            float_c4: 1.0,
            records_c8: [ShaderAccumulatorRecord::default(); 3],
            render_pass_list_counts: vec![0; Self::RENDER_PASS_LISTS],
            bytes_21e0: [1, 0, 0, 1],
            word_21e4: 0,
            word_21e6: 50,
            pointers_21e8: vec![0; 50],
            dword_2228: 0,
            dword_223c: 0,
            byte_2240: 0,
            // 007AC266..007AC286 007F5B50: FormHeap(4 * g_niD3DMaxTextureBlendStages).
            blend_stage_slots_2264: vec![0; max_texture_blend_stages as usize],
            byte_2268: 0,
        }
    }
}

// IDA: the BSShaderManager globals 007BA0F0 writes.
#[derive(Resource, Clone, Debug)]
pub struct ShaderManagerGlobals {
    pub initialized: bool,                           // 00B42F31 once guard
    pub texture_manager: Option<TextureManager>,     // 00B42F50
    pub package: Option<Arc<ShaderPackage>>,         // 00B430B8
    pub accumulator: Option<Arc<ShaderAccumulator>>, // 00B430FC
    pub floats_b4616c: [f32; 4],                     // 00B4616C..00B46178
    pub depth_discard_flag: u32,                     // 00B294EC (.data initial 1)
    pub memory: RenderMemoryStatistics,              // 00B3FAB8/00B3FABC
    pub scene_byte_b42cdd: u8,                       // 00B42CDD (007AC071 clears it)
    pub texture_manager_status: TextureManagerStatus,
}

impl Default for ShaderManagerGlobals {
    fn default() -> Self {
        Self {
            initialized: false,
            texture_manager: None,
            package: None,
            accumulator: None,
            floats_b4616c: [0.0; 4],
            depth_discard_flag: 1,
            memory: crate::gamebryo::render_targets::RenderMemoryStatistics::default(),
            scene_byte_b42cdd: 0,
            texture_manager_status: TextureManagerStatus::Created,
        }
    }
}

// IDA: 007B7650 BSShaderAccumulator_GetSingleton: when 00B430FC is null,
// FormHeap(0x226C) 007ABE30 and a refcounted store; returns 00B430FC.
// `max_texture_blend_stages` is g_niD3DMaxTextureBlendStages (00B28CB0).
// Completion: 100%.
pub fn shader_accumulator_singleton(
    globals: &mut ShaderManagerGlobals,
    max_texture_blend_stages: u32,
) -> Arc<ShaderAccumulator> {
    // 007B7673: an existing 00B430FC is returned.
    if let Some(accumulator) = &globals.accumulator {
        return Arc::clone(accumulator);
    }
    // 007B767C..007B76E8 FormHeap(0x226C) 007ABE30, refcounted store.
    let accumulator = Arc::new(ShaderAccumulator::new(max_texture_blend_stages));
    // 007AC071: byte 00B42CDD = 0.
    globals.scene_byte_b42cdd = 0;
    globals.accumulator = Some(Arc::clone(&accumulator));
    accumulator
}

/// IDA: 004814ED vt+4 FindFile, by path.
pub type FindFileService<'a> = Box<dyn FnMut(&str) -> bool + 'a>;
/// IDA: 00431690, by the package load's shape.
pub type PackageOpen<'a> = Box<dyn FnMut(&str) -> Option<Box<dyn GameFile>> + 'a>;
/// IDA: 007C1FF0 with the renderer objects it reaches, through `construct_texture_manager`. A closure, because those objects
/// belong to `RendererCreation`, which the later steps also need; its arguments are 00B294EC and 00B3FAB8.
pub type TextureManagerStep<'a> = Box<
    dyn FnMut(
            &mut World,
            &mut u32,
            &mut RenderMemoryStatistics,
        ) -> Result<(TextureManager, TextureManagerStatus), GameError>
        + 'a,
>;
/// IDA: N/A a step of 007BA0F0. It sees the globals as the original's callee would, through the global names.
pub type ManagerStep<'a> =
    Box<dyn FnMut(&mut World, &ShaderManagerGlobals) -> Result<(), GameError> + 'a>;
/// IDA: N/A 007B4290's step, by slot.
pub type ManagerSlotStep<'a> =
    Box<dyn FnMut(&mut World, &ShaderManagerGlobals, i32) -> Result<(), GameError> + 'a>;

// IDA: N/A 007C1FF0 on a renderer with no device, as the C++ fixtures have it. Its device calls go nowhere, since there are none on
// this branch unless a multisample configuration asks, which then fails for want of a device.
// Completion: 100%.
pub fn texture_manager_without_device(state: &RendererState) -> TextureManagerStep<'_> {
    Box::new(move |world, discard, memory| {
        let mut calls = Vec::new();
        let mut services = crate::gamebryo::render_targets::TextureManagerServices::without_device(
            Some(state),
            &mut calls,
        );
        construct_texture_manager(world, &mut services, discard, memory)
    })
}

// IDA: N/A inputs and injected steps of 007BA0F0. An empty function records
// its step as not yet ported.
pub struct ShaderManagerInputs<'a> {
    pub construct_texture_manager: TextureManagerStep<'a>, // 007C1FF0
    pub renderer_globals: &'a RendererGlobals,             // render path, HDR, SM3, FP16 filtering
    pub max_texture_blend_stages: u32,                     // 00B28CB0
    pub renderer_reference: bool,                          // 00B43104 (00406A56)
    pub image_space_effects: u8,                           // 00B42F3E
    pub open_file: PackageOpen<'a>,                        // 00431690
    pub initialize_screen_element_palette: Option<ManagerStep<'a>>, // 007AB1D0(1)
    pub create_effect_textures: Option<ManagerStep<'a>>,   // 007B8990
    pub create_shader: Option<ManagerSlotStep<'a>>,        // 007B4290
    pub create_image_space_shaders: Option<ManagerStep<'a>>, // 00803A30
}

/// IDA: N/A
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ShaderManagerResult {
    pub ran: bool,
    pub pending_steps: Vec<u32>,
}

// IDA: 007BA0F0 in order: guard, 007C1FF0 -> 00B42F50, FormHeap(0x1C)
// 007DB050(Renderer_GetShaderPackageIndex) -> 00B430B8, 007DACA0, an empty
// body destroys the package, 007B7650 -> 00B430FC, 007AB1D0(1), 007B8990,
// the floats (0.5, 0.5, 0.5, 1.0), then with the renderer alias 007B4290(1..27),
// the package destruction and, with image-space effects, 00803A30 ->
// 00B42D7C; finally 00B42F31 = 1. Completion: 100% order of the ported parts.
// The injected steps are ported (05-D-3 effect textures, 05-D-4 slots,
// 05-D-5 image-space list); `crate::tes::winmain::winmain_scene` binds them.
pub fn initialize_shader_manager(
    world: &mut World,
    globals: &mut ShaderManagerGlobals,
    inputs: &mut ShaderManagerInputs<'_>,
) -> Result<ShaderManagerResult, GameError> {
    let mut result = ShaderManagerResult::default();
    // 007BA115 the once guard 00B42F31.
    if globals.initialized {
        return Ok(result);
    }
    result.ran = true;
    // 007BA122..007BA150 FormHeap(0x48) 007C1FF0 -> 00B42F50.
    let (manager, status) = (inputs.construct_texture_manager)(
        world,
        &mut globals.depth_discard_flag,
        &mut globals.memory,
    )?;
    globals.texture_manager = Some(manager);
    globals.texture_manager_status = status;
    // 007BA14A..007BA180 FormHeap(0x1C) 007DB050(007DAB80) -> 00B430B8.
    let mut package = ShaderPackage::new(shader_package(inputs.renderer_globals));
    // 007BA185..007BA18E 007DACA0(+0x00).
    let _ = package.load(&mut *inputs.open_file);
    // 007BA193..007BA1AF: an empty body destroys the package.
    globals.package = package.has_body().then(|| Arc::new(package));
    // 007BA1B9..007BA1F8 007B7650 -> 00B430FC (same object, no refcount change).
    let _ = shader_accumulator_singleton(globals, inputs.max_texture_blend_stages);
    // 007BA1FE 007AB1D0(1).
    if let Some(step) = inputs.initialize_screen_element_palette.as_mut() {
        step(world, globals)?;
    } else {
        result.pending_steps.push(0x007A_B1D0);
    }
    // 007BA208 007B8990 (05-D-3).
    if let Some(step) = inputs.create_effect_textures.as_mut() {
        step(world, globals)?;
    } else {
        result.pending_steps.push(0x007B_8990);
    }
    // 007BA20D..007BA24D: 0.5 (flt_A3D65C) x3, then 1.0, stored in order.
    globals.floats_b4616c = [0.5, 0.5, 0.5, 1.0];
    // 007BA213/007BA252: the renderer alias 00B43104 gates the rest.
    if inputs.renderer_reference {
        // 007BA254..007BA26F 007B4290(1..27) (05-D-4).
        if let Some(step) = inputs.create_shader.as_mut() {
            for slot in 1..28 {
                step(world, globals, slot)?;
            }
        } else {
            result.pending_steps.push(0x007B_4290);
        }
        // 007BA271..007BA28B: the package is destroyed after the slots.
        globals.package = None;
        // 007BA295..007BA2CC: image-space shader list 00803A30 -> 00B42D7C.
        if inputs.image_space_effects != 0 {
            if let Some(step) = inputs.create_image_space_shaders.as_mut() {
                step(world, globals)?;
            } else {
                result.pending_steps.push(0x0080_3A30);
            }
        }
    }
    // 007BA2D1 00B42F31 = 1.
    globals.initialized = true;
    Ok(result)
}

// IDA: N/A the globals 004814C0 reads after the accumulator.
pub struct TestSphereInputs<'a> {
    pub max_texture_blend_stages: u32,          // 00B28CB0 for 007B7650
    pub model_loader: bool,                     // 00B33A1C (TES_Constructor/TES_Destructor)
    pub file_finder: bool,                      // 00B33A04 g_FileFinder
    pub find_file: Option<FindFileService<'a>>, // vt+4(path, 0, 0, -1)
}

/// IDA: N/A how 004814C0 finished.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TestSphereStatus {
    NoAccumulator,
    AccumulatorBusy, // +0x2228 non-zero
    NoModelLoader,
    NoFileFinder,
    NotFound,
    NifBranchCarried, // 00439EB0 load, 00405790 walk, refresh (Phase 09+)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bs::file::{BsFile, NiFileMode};
    use std::cell::{Cell, RefCell};
    use std::path::{Path, PathBuf};

    struct TempRoot(PathBuf);
    impl TempRoot {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!("{name}-{}", std::process::id()));
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

    /// IDA: N/A test support: `write_package` of `phase05_tests.cpp`.
    fn write_package_file(path: &Path, version: u32, body: &[u8], count: u32, declared_extra: u32) {
        let mut bytes = Vec::with_capacity(body.len() + 12);
        bytes.extend_from_slice(&version.to_le_bytes());
        bytes.extend_from_slice(&count.to_le_bytes());
        bytes.extend_from_slice(&(body.len() as u32 + declared_extra).to_le_bytes());
        bytes.extend_from_slice(body);
        std::fs::write(path, bytes).expect("the test's temporary file");
    }

    fn write_package(root: &Path, file: &str, version: u32, entries: &[(&str, &[u8])], extra: u32) {
        write_package_file(
            &root.join(file),
            version,
            &package_body(entries),
            entries.len() as u32,
            extra,
        );
    }

    fn open_at(root: &Path, name: &str) -> Option<Box<dyn GameFile>> {
        Some(Box::new(BsFile::new(root, name, NiFileMode::ReadOnly, 0)))
    }

    /// `verify_shader_package`, the fixture half. The real package is `tests/shader_package.rs`.
    #[test]
    fn verify_shader_package() {
        let temp = TempRoot::new("oblivion-recomp-phase05-package");
        let root = temp.0.as_path();
        write_package(
            root,
            "good.sdp",
            100,
            &[
                ("A.vso", b"first"),
                ("b.pso", b"second"),
                ("A.vso", b"third!"),
                ("a.vso", b"lower"),
            ],
            0,
        );
        let mut package = ShaderPackage::new(13);
        assert_eq!(
            package.file_name(),
            Some("shaderpackage013.sdp"),
            "007DB050 name"
        );
        // "A.vso" twice merges; "a.vso" hashes to another bucket (the raw-byte
        // hash is case-sensitive; _stricmp only runs inside one bucket).
        assert!(package.load(&mut |_| open_at(root, "good.sdp")));
        assert!(package.has_body());
        assert_eq!(
            package.entry_count(),
            3,
            "007DACA0 must load and merge equal keys"
        );
        let merged = package.find("A.vso").unwrap();
        assert_eq!(merged.name, b"A.vso");
        assert_eq!(merged.size, 6);
        assert_eq!(
            package.data(merged),
            b"third!",
            "InsertOrAssign keeps the first key and takes the new value"
        );
        assert_eq!(package.find("a.vso").unwrap().size, 5);
        assert!(package.find("b.pso").is_some());
        assert!(package.find("c.pso").is_none());

        write_package(root, "version.sdp", 99, &[("x", b"y")], 0);
        let mut wrong = ShaderPackage::new(1);
        assert!(
            !wrong.load(&mut |_| open_at(root, "version.sdp")),
            "a version other than 100 must fail"
        );
        assert!(!wrong.has_body());
        write_package(root, "short.sdp", 100, &[("x", b"y")], 16);
        let mut short_read = ShaderPackage::new(1);
        assert!(
            !short_read.load(&mut |_| open_at(root, "short.sdp")),
            "a short body read must free the body"
        );
        assert!(!short_read.has_body());
        let mut unnamed = ShaderPackage::new(0);
        assert!(unnamed.file_name().is_none());
        assert!(
            !unnamed.load(&mut |_| open_at(root, "good.sdp")),
            "index 0 has no name and does not load"
        );
    }

    /// `verify_test_sphere`: 004814C0's guards.
    #[test]
    fn verify_test_sphere() {
        let mut globals = ShaderManagerGlobals::default();
        let searched = RefCell::new(Vec::new());
        let mut inputs = TestSphereInputs {
            max_texture_blend_stages: 8,
            model_loader: false,
            // Startup: 00B33A1C is null, so neither FindFile nor 00439EB0 runs; the
            // accumulator singleton is still created (007B7650).
            file_finder: true,
            find_file: Some(Box::new(|path| {
                searched.borrow_mut().push(path.to_owned());
                true
            })),
        };
        assert_eq!(
            load_test_sphere(&mut globals, &mut inputs),
            TestSphereStatus::NoModelLoader
        );
        assert!(globals.accumulator.is_some());
        assert!(searched.borrow().is_empty());
        inputs.model_loader = true;
        inputs.file_finder = false;
        assert_eq!(
            load_test_sphere(&mut globals, &mut inputs),
            TestSphereStatus::NoFileFinder
        );
        inputs.file_finder = true;
        Arc::get_mut(globals.accumulator.as_mut().unwrap())
            .unwrap()
            .dword_2228 = 1;
        assert_eq!(
            load_test_sphere(&mut globals, &mut inputs),
            TestSphereStatus::AccumulatorBusy
        );
        assert!(searched.borrow().is_empty());
        Arc::get_mut(globals.accumulator.as_mut().unwrap())
            .unwrap()
            .dword_2228 = 0;
        assert_eq!(
            load_test_sphere(&mut globals, &mut inputs),
            TestSphereStatus::NifBranchCarried
        );
        assert_eq!(*searched.borrow(), ["Meshes\\TestSphere.NIF"]);
        inputs.find_file = Some(Box::new(|_| false));
        assert_eq!(
            load_test_sphere(&mut globals, &mut inputs),
            TestSphereStatus::NotFound
        );
    }

    /// `verify_shader_manager`: 007B7650 / 007ABE30 and the 007BA0F0 order.
    #[test]
    fn verify_shader_manager() {
        let mut globals = ShaderManagerGlobals::default();
        let accumulator = shader_accumulator_singleton(&mut globals, 8);
        assert!(Arc::ptr_eq(
            &accumulator,
            &shader_accumulator_singleton(&mut globals, 4)
        ));
        assert_eq!(accumulator.original_vtable, 0x00A8_CC5C);
        assert_eq!(accumulator.byte_34, 1);
        assert_eq!(accumulator.float_c4, 1.0);
        assert_eq!(accumulator.word_21e6, 50);
        assert_eq!(accumulator.bytes_21e0, [1, 0, 0, 1]);
        assert_eq!(accumulator.blend_stage_slots_2264.len(), 8);

        let temp = TempRoot::new("oblivion-recomp-phase05-manager");
        let root = temp.0.as_path();
        write_package(root, "pkg.sdp", 100, &[("SLS1000.vso", b"code")], 0);
        let state = RendererState::default();
        let renderer = RendererGlobals {
            render_path: 5,
            shaders_30: true,
            hdr: true,
            fp16_filtering: true,
            ..RendererGlobals::default()
        };
        let expected_slots: Vec<i32> = (1..=27).collect();
        let opened = RefCell::new(String::new());
        let palette_calls = Cell::new(0);
        let slots = RefCell::new(Vec::new());
        let alive = Cell::new(false);
        let mut manager = ShaderManagerGlobals::default();
        let mut world = World::new();
        let mut inputs = ShaderManagerInputs {
            construct_texture_manager: texture_manager_without_device(&state),
            renderer_globals: &renderer,
            max_texture_blend_stages: 8,
            renderer_reference: true,
            image_space_effects: 1,
            open_file: Box::new(|path| {
                *opened.borrow_mut() = path.to_owned();
                open_at(root, "pkg.sdp")
            }),
            initialize_screen_element_palette: Some(Box::new(|_, _| {
                palette_calls.set(palette_calls.get() + 1);
                Ok(())
            })),
            create_effect_textures: None,
            create_shader: Some(Box::new(|_, globals, slot| {
                slots.borrow_mut().push(slot);
                alive.set(
                    globals
                        .package
                        .as_ref()
                        .is_some_and(|package| package.find("SLS1000.vso").is_some()),
                );
                Ok(())
            })),
            create_image_space_shaders: None,
        };
        let result = initialize_shader_manager(&mut world, &mut manager, &mut inputs).unwrap();
        assert!(result.ran);
        assert!(manager.initialized);
        // 007BA0F0 must open the 007DAB80 package (render path 5, SM3, HDR, FP16 = 13).
        assert_eq!(*opened.borrow(), "\\Data\\Shaders\\shaderpackage013.sdp");
        assert_eq!(*slots.borrow(), expected_slots);
        assert!(alive.get());
        assert!(manager.package.is_none());
        assert_eq!(palette_calls.get(), 1);
        assert!(manager.texture_manager.is_some());
        assert!(manager.accumulator.is_some());
        assert_eq!(manager.floats_b4616c, [0.5, 0.5, 0.5, 1.0]);
        assert_eq!(result.pending_steps, [0x007B_8990, 0x0080_3A30]);
        let again = initialize_shader_manager(&mut world, &mut manager, &mut inputs).unwrap();
        assert!(!again.ran, "00B42F31 must guard a second call");
        assert_eq!(palette_calls.get(), 1);

        // Without the renderer alias the slots and the package release are skipped.
        let mut no_renderer = ShaderManagerGlobals::default();
        inputs.renderer_reference = false;
        inputs.create_shader = None;
        let skipped = initialize_shader_manager(&mut world, &mut no_renderer, &mut inputs).unwrap();
        assert!(skipped.ran);
        assert!(no_renderer.package.is_some());
        assert_eq!(skipped.pending_steps, [0x007B_8990]);

        // 05-D-6: a missing package file or a wrong version leaves 00B430B8 null
        // (007BA193..007BA1AF); the slots still run and load from the cache/source.
        for version in [0u32, 99] {
            if version != 0 {
                write_package(root, "bad.sdp", version, &[("SLS1000.vso", b"code")], 0);
            }
            let mut missing = ShaderManagerGlobals::default();
            let seen = Cell::new(false);
            let local = RefCell::new(Vec::new());
            {
                let mut step_inputs = ShaderManagerInputs {
                    construct_texture_manager: texture_manager_without_device(&state),
                    renderer_globals: &renderer,
                    max_texture_blend_stages: 8,
                    renderer_reference: true,
                    image_space_effects: 1,
                    open_file: Box::new(move |_| {
                        (version != 0).then(|| open_at(root, "bad.sdp")).flatten()
                    }),
                    initialize_screen_element_palette: None,
                    create_effect_textures: None,
                    create_shader: Some(Box::new(|_, globals, slot| {
                        local.borrow_mut().push(slot);
                        seen.set(seen.get() || globals.package.is_some());
                        Ok(())
                    })),
                    create_image_space_shaders: None,
                };
                let failed =
                    initialize_shader_manager(&mut world, &mut missing, &mut step_inputs).unwrap();
                assert!(failed.ran);
            }
            assert!(missing.initialized);
            assert!(missing.package.is_none());
            assert!(!seen.get());
            assert_eq!(*local.borrow(), expected_slots);
            assert!(missing.texture_manager.is_some());
            assert!(missing.accumulator.is_some());
        }
    }
}
