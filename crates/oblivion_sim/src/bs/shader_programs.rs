//! Phase 05 row 05-D-4a (shared core, part 2): the BSShader program loaders 008014E0 (vertex) and 00801C90 (pixel)
//! with their path helpers 00801030 and 00801210, and the program objects they create. No GPU object exists here:
//! `Create*Shader` is a trace entry, and the program keeps its parsed D3D9 bytecode as data.

use std::sync::Arc;

use crate::bs::file::GameFile;
use crate::bs::shader_manager::ShaderPackage;
use crate::gamebryo::renderer::{RendererGlobals, pixel_shader_target};

const S_OK: u32 = 0;

/// IDA: N/A the parsed form of a D3D9 shader token stream: the version token and the dwords, retained as data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShaderBytecode {
    /// The first token: `0xFFFE` (vertex) or `0xFFFF` (pixel) in the high word, then major and minor bytes.
    pub version_token: u32,
    pub dwords: Vec<u32>,
    /// The last token is D3DSIO_END (`0x0000FFFF`).
    pub ends_with_end_token: bool,
}

impl ShaderBytecode {
    /// IDA: N/A host parse of the bytes CreateVertexShader/CreatePixelShader would take. None when the bytes are
    /// not a whole number of dwords or hold none.
    #[must_use]
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        if bytes.is_empty() || !bytes.len().is_multiple_of(4) {
            return None;
        }
        let dwords: Vec<u32> = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|word| u32::from_le_bytes(*word))
            .collect();
        Some(Self {
            version_token: dwords[0],
            ends_with_end_token: dwords.last() == Some(&0x0000_FFFF),
            dwords,
        })
    }

    /// IDA: N/A the version token's stage word says vertex.
    #[must_use]
    pub const fn is_vertex(&self) -> bool {
        self.version_token >> 16 == 0xFFFE
    }

    /// IDA: N/A (major, minor) of the version token.
    #[must_use]
    pub const fn version(&self) -> (u8, u8) {
        ((self.version_token >> 8) as u8, self.version_token as u8)
    }
}

// IDA: 007601E0 the 100-byte NiD3DPass (vtable A882E4 slot +0 the host
// owner): +4 byte, +0x10, +0x14 stage end (highest set index + 1), +0x18 set
// stages, +0x1C stages with a texture, +0x20 NiTArray<NiPointer<
// NiD3DTextureStage>> (capacity g_niD3DMaxTextureBlendStages, grow 1),
// +0x30 render-state group (lazily acquired by the callers), +0x34 vertex
// constant map, +0x38/+0x3C/+0x40 vertex program strings, +0x44 pixel shader,
// +0x48 pixel constant map, +0x4C/+0x50/+0x54 pixel program strings, +0x58
// vertex shader, +0x5C byte, +0x5D byte (set by the pool acquire), +0x60
// reference count.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShaderProgramObject {
    pub original_vtable: u32,
    pub vertex: bool,
    /// +0x1C was non-null. A handle stays a host value and is never compared with evidence.
    pub has_device: bool,
    /// +0x20 was non-null.
    pub has_renderer: bool,
    pub field_28: u8,
    pub field_2c: u32,
    pub field_34: u32,
    /// Vertex +0x30 / pixel +0x28: the seam's handle, a host value.
    pub handle: Option<u32>,
    /// Host: the program name looked up.
    pub package_name: String,
    /// Host: the bytes the device call took, parsed. None when they do not parse.
    pub bytecode: Option<ShaderBytecode>,
}

// IDA: one D3DXMACRO {Name, Definition}; a null Name ends the list. The
// BSShader descriptor records are 19 dwords: the HLSL path, then 9 macro
// pairs (the last pair stays null).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ShaderMacro {
    pub name: String,
    /// None is a null pointer.
    pub definition: Option<String>,
}

/// IDA: N/A a descriptor record; see `ShaderMacro`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ShaderProgramDescriptor {
    pub hlsl_path: String,
    pub macros: Vec<ShaderMacro>,
}

/// IDA: N/A one shader creation, in call order. `site` is the loader's address; `bytes` the size passed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShaderDeviceCall {
    pub site: u32,
    pub vertex: bool,
    pub program_name: String,
    pub bytes: usize,
}

/// IDA: 004315F0 and 00431130 and D3DXCompileShader, by the shape the loaders need.
pub type FileTimeService = Box<dyn FnMut(&str) -> Option<u64> + Send + Sync>;
/// IDA: 00431130(path, 0, 10240, 0x20): BSFile_OpenCore cast to BSFile.
pub type OpenFileService = Box<dyn FnMut(&str) -> Option<Box<dyn GameFile>> + Send + Sync>;
/// IDA: D3DXCompileShader(source, macros (+ "HDR"), ShaderIncludes, "main", profile): host substitution; a later
/// milestone decides whether HLSL compiles at all. Returns the success flag, the object bytes and the error text.
pub type CompileService =
    Box<dyn FnMut(&[u8], &[ShaderMacro], &str) -> (bool, Vec<u8>, String) + Send + Sync>;
/// IDA: the device's CreateVertexShader/CreatePixelShader result, by call.
pub type DeviceResultService = Box<dyn FnMut(&ShaderDeviceCall) -> u32 + Send + Sync>;

/// IDA: N/A the services and globals the loaders read.
pub struct ShaderProgramServices {
    pub package: Option<Arc<ShaderPackage>>, // 00B430B8 g_shaderPackage
    pub compile_fallback: bool,              // 00B2DD4C (1 in the executable)
    pub use_hard_drive_cache: bool,          // 00B430AE
    pub hdr: bool,                           // 00B43070
    pub cache_directory: String,             // 00B42D80 (mutable: 00800FB0)
    pub shader_directory: String,            // 00B42F68
    pub render_path: i32,                    // 00B42F48
    pub ps2x_path: i32,                      // 00B42D74
    pub has_device: bool,                    // renderer +0x280
    pub has_renderer: bool,                  // 00B43104
    // 004315F0(path, &time, 0x20): FindPackedArchive then FindFirstFileA on the archive or loose path.
    pub file_time: Option<FileTimeService>,
    pub open_file: Option<OpenFileService>,
    pub compile: Option<CompileService>,
    pub device_result: Option<DeviceResultService>,
    /// 00B42E8C(text, null), in call order.
    pub warnings: Vec<String>,
    /// The device calls, in call order.
    pub device_calls: Vec<ShaderDeviceCall>,
    next_handle: u32,
}

impl Default for ShaderProgramServices {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            package: None,
            compile_fallback: true,
            use_hard_drive_cache: false,
            hdr: false,
            cache_directory: String::new(),
            shader_directory: String::new(),
            render_path: 0,
            ps2x_path: 0,
            has_device: false,
            has_renderer: false,
            file_time: None,
            open_file: None,
            compile: None,
            device_result: None,
            warnings: Vec::new(),
            device_calls: Vec::new(),
            next_handle: 0,
        }
    }
}

// IDA: the macro text loops of 008014E0/00801C90: name, "=definition" when
// the definition is non-null and non-empty, then ", ". `stop_at_null`
// selects the loops that break at the first null name (pixel 00801F35 and
// vertex 008018DA); the others skip null names (vertex 00801756 and pixel
// 008020C7).
fn macro_text(macros: &[ShaderMacro], stop_at_null: bool) -> String {
    let mut text = String::new();
    for index in 0..8 {
        let Some(item) = macros.get(index).filter(|item| !item.name.is_empty()) else {
            if stop_at_null {
                break;
            }
            continue;
        };
        text += &item.name;
        if let Some(definition) = item
            .definition
            .as_ref()
            .filter(|definition| !definition.is_empty())
        {
            text.push('=');
            text += definition;
        }
        text += ", ";
    }
    text
}

// IDA: 00780D80 NiD3DVertexShader (A8B41C) / 00780B90 NiD3DPixelShader (A8B3CC), then the device call.
fn create_program_object(
    services: &mut ShaderProgramServices,
    vertex: bool,
    bytes: &[u8],
    program_name: &str,
    cache_path: &str,
    site: u32,
) -> Option<Arc<ShaderProgramObject>> {
    let mut program = ShaderProgramObject {
        original_vtable: if vertex { 0x00A8_B41C } else { 0x00A8_B3CC },
        vertex,
        has_device: services.has_device,
        has_renderer: services.has_renderer,
        field_28: 0,
        field_2c: 0,
        field_34: 0,
        handle: None,
        package_name: program_name.to_owned(),
        bytecode: ShaderBytecode::parse(bytes),
    };
    if !services.has_device {
        return Some(Arc::new(program)); // host fixtures: no device, the object without a handle
    }
    let call = ShaderDeviceCall {
        site,
        vertex,
        program_name: program_name.to_owned(),
        bytes: bytes.len(),
    };
    let result = services
        .device_result
        .as_mut()
        .map_or(S_OK, |result| result(&call));
    services.device_calls.push(call);
    if (result as i32) < 0 {
        services.warnings.push(format!(
            "{}{cache_path}",
            if vertex {
                "SHADER ERROR : failed to create vertex shader : "
            } else {
                "SHADER ERROR : failed to create pixel shader : "
            }
        ));
        return None;
    }
    services.next_handle += 1;
    program.handle = Some(services.next_handle);
    Some(Arc::new(program))
}

// IDA: N/A a helper, with no address of its own.
fn file_time(services: &mut ShaderProgramServices, path: &str) -> Option<u64> {
    services.file_time.as_mut().and_then(|time| time(path))
}

// IDA: N/A a helper, with no address of its own.
fn read_whole(file: &mut dyn GameFile) -> Vec<u8> {
    let mut bytes = vec![0; file.size() as usize];
    let _ = file.read_components(&mut bytes, Some(&[1]));
    bytes
}

// IDA: the shared body of 008014E0 and 00801C90.
fn load_program(
    services: &mut ShaderProgramServices,
    vertex: bool,
    source_path: &str,
    caller_macros: &[ShaderMacro],
    profile: &str,
    program_name: &str,
) -> Option<Arc<ShaderProgramObject>> {
    let site = if vertex { 0x0080_14E0 } else { 0x0080_1C90 };
    // 00801569/00801D15 00801210.
    let mut cache_path = shader_cache_path(
        &services.cache_directory,
        &services.shader_directory,
        services.hdr,
        program_name,
    );
    let mut use_cache = true;
    let package = services.package.clone();
    let entry = package
        .as_ref()
        .and_then(|package| package.find(program_name));
    // 00801595..0080167C the fallback decision.
    if services.compile_fallback && entry.is_none() {
        let mut cache_time = file_time(services, &cache_path);
        let source_time = file_time(services, source_path);
        if cache_time.is_none() && services.use_hard_drive_cache {
            if services.cache_directory.is_empty() {
                services.cache_directory = "cache:\\".to_owned(); // 00800FB0
                cache_path = shader_cache_path(
                    &services.cache_directory,
                    &services.shader_directory,
                    services.hdr,
                    program_name,
                );
            }
            cache_time = file_time(services, &cache_path);
        }
        if source_time.is_none() {
            services.warnings.push(format!(
                "SHADER ERROR : error opening HLSL source file : {source_path}"
            ));
        }
        if cache_time.is_none() {
            use_cache = false;
        }
        if source_time.unwrap_or(0) > cache_time.unwrap_or(0) {
            use_cache = false;
        }
    }
    let owned;
    let bytes: &[u8] = if let (Some(entry), Some(package)) = (entry, package.as_ref()) {
        package.data(entry) // 00801696
    } else if use_cache {
        // 00801AFD..00801B47 the cache file.
        let Some(mut file) = services
            .open_file
            .as_mut()
            .and_then(|open| open(&cache_path))
        else {
            services.warnings.push(format!(
                "SHADER ERROR : error opening cache file : {cache_path}"
            ));
            return None;
        };
        owned = if file.good() {
            read_whole(file.as_mut())
        } else {
            Vec::new()
        };
        &owned
    } else {
        // 008016B1..00801ACA / 00801E62..0080227E the compile.
        let mut macros = caller_macros.to_vec();
        if services.hdr {
            macros.push(ShaderMacro {
                name: "HDR".to_owned(),
                definition: Some(String::new()),
            });
        }
        let mut compile_profile = profile.to_owned();
        if !vertex && compile_profile.starts_with("ps_2") {
            // 007B4780 Renderer_GetPixelShaderTarget(1) (Phase 04 port).
            let globals = RendererGlobals {
                render_path: services.render_path,
                ps2x_path: services.ps2x_path,
                ..RendererGlobals::default()
            };
            if let Some(target) = pixel_shader_target(&globals, true) {
                target.clone_into(&mut compile_profile);
            }
        }
        let file = services
            .open_file
            .as_mut()
            .and_then(|open| open(source_path));
        let Some(mut file) = file.filter(|file| file.good()) else {
            services.warnings.push(format!(
                "SHADER ERROR : error opening hlsl file : {source_path}"
            ));
            return None;
        };
        let source = read_whole(file.as_mut());
        drop(file);
        services.warnings.push(format!(
            "compiling : {source_path}\n {compile_profile} {}",
            macro_text(&macros, !vertex)
        ));
        let (compiled, object, errors) = services
            .compile
            .as_mut()
            .map_or((false, Vec::new(), String::new()), |compile| {
                compile(&source, &macros, &compile_profile)
            });
        if !errors.is_empty() && object.is_empty() {
            services.warnings.push(format!(
                "{source_path}\n {}\n {errors}",
                macro_text(&macros, vertex)
            ));
        }
        if !compiled {
            return None;
        }
        // The cache write (fopen(cache, "wb")) is a host no-op: Data stays
        // read-only. The compiled bytes are used directly.
        owned = object;
        &owned
    };
    if bytes.is_empty() {
        return None;
    }
    create_program_object(services, vertex, bytes, program_name, &cache_path, site)
}

// IDA: N/A CRT `_splitpath`: drive, directory, file name and extension. The `_s` size limits of the C++ port's
// buffers do not apply to a Rust string (recorded difference for parts of 260 characters and more).
fn split_path(path: &str) -> (&str, &str, &str, &str) {
    let (drive, rest) = if path.as_bytes().get(1) == Some(&b':') {
        path.split_at(2)
    } else {
        ("", path)
    };
    let slash = rest.rfind(['\\', '/']).map_or(0, |index| index + 1);
    let (directory, file) = rest.split_at(slash);
    let dot = file.rfind('.').unwrap_or(file.len());
    let (name, extension) = file.split_at(dot);
    (drive, directory, name, extension)
}

// IDA: 00801030(path, buffer): splits the descriptor path and 00B42F68
// (g_shaderDirectory), appends the path's directory to the shader directory's
// and _makepath(shader drive, joined directory, name, extension). Completion: 100%.
#[must_use]
pub fn shader_source_path(shader_directory: &str, descriptor_path: &str) -> String {
    // 00801055..008010E9.
    let (_, directory, file_name, extension) = split_path(descriptor_path);
    let (shader_drive, shader_directory_part, _, _) = split_path(shader_directory);
    // `_makepath`: the joined directory takes a separator when it lacks one; the extension its dot already has.
    let mut joined = format!("{shader_drive}{shader_directory_part}{directory}");
    if !joined.is_empty() && !joined.ends_with(['\\', '/']) {
        joined.push('\\');
    }
    format!("{joined}{file_name}{extension}")
}

// IDA: 00801210(buffer, sourcePath, programName): the cache directory 00B42D80
// (or 00B42F68 when empty), "HDR." when 00B43070, then the program name.
// Completion: 100%.
#[must_use]
pub fn shader_cache_path(
    cache_directory: &str,
    shader_directory: &str,
    hdr: bool,
    program_name: &str,
) -> String {
    // 0080123B..008012AF.
    let mut path = if cache_directory.is_empty() {
        shader_directory
    } else {
        cache_directory
    }
    .to_owned();
    if hdr {
        path += "HDR.";
    }
    path += program_name;
    path
}

// IDA: 008014E0 (stdcall: sourcePath, macros, profile, programName, 0, 0) and
// 00801C90 (the same for pixel programs; a "ps_2" profile is replaced by
// Renderer_GetPixelShaderTarget(1) for the compile): the package entry
// first; with the fallback and no entry, the cache/source file times
// (00800FB0("cache:\\") when the hard-drive cache is on and the cache
// directory is empty), the "error opening HLSL source file" warning, then
// the cache file or the compile ("HDR" appended to the macros while
// compiling, the "compiling : ..." warning, errors as warnings; the cache
// write is a host no-op: the user's Data stays read-only); 00B430B4 (never
// written) is null; CreateVertexShader/CreatePixelShader on the device and
// a NiD3DVertexShader/NiD3DPixelShader holding the handle, else the
// "failed to create ..." warning and null. 100% for the package path; the
// compile path is a recorded host substitution.
pub fn load_vertex_program(
    services: &mut ShaderProgramServices,
    source_path: &str,
    macros: &[ShaderMacro],
    profile: &str,
    program_name: &str,
) -> Option<Arc<ShaderProgramObject>> {
    load_program(services, true, source_path, macros, profile, program_name)
}

// IDA: 00801C90; see `load_vertex_program`.
pub fn load_pixel_program(
    services: &mut ShaderProgramServices,
    source_path: &str,
    macros: &[ShaderMacro],
    profile: &str,
    program_name: &str,
) -> Option<Arc<ShaderProgramObject>> {
    load_program(services, false, source_path, macros, profile, program_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `verify_shader_runtime`'s path checks: 00801030 / 00801210.
    #[test]
    fn verify_shader_runtime_paths() {
        assert_eq!(
            shader_source_path("data\\shaders\\", "tallgrass\\1x\\v\\lowDetail.v.hlsl"),
            "data\\shaders\\tallgrass\\1x\\v\\lowDetail.v.hlsl"
        );
        assert_eq!(
            shader_cache_path("", "data\\shaders\\", true, "GRASS2020.vso"),
            "data\\shaders\\HDR.GRASS2020.vso"
        );
    }

    /// `verify_shader_program_loader`: 008014E0 package path and fallback warnings without a device.
    #[test]
    fn verify_shader_program_loader() {
        let mut services = ShaderProgramServices {
            shader_directory: "data\\shaders\\".to_owned(),
            use_hard_drive_cache: true,
            file_time: Some(Box::new(|_| None)),
            open_file: Some(Box::new(|_| None)),
            ..ShaderProgramServices::default()
        };
        // No package entry: the fallback warns, switches the cache directory
        // to "cache:\" and reports the missing source.
        let missing = load_vertex_program(
            &mut services,
            "data\\shaders\\a.v.hlsl",
            &[],
            "vs_2_0",
            "MISSING000.vso",
        );
        assert!(missing.is_none());
        assert_eq!(services.cache_directory, "cache:\\");
        assert_eq!(
            services.warnings,
            [
                "SHADER ERROR : error opening HLSL source file : data\\shaders\\a.v.hlsl",
                "SHADER ERROR : error opening hlsl file : data\\shaders\\a.v.hlsl"
            ]
        );
    }

    /// The device-call seam the C++ fixture could not reach: a trace entry, and the failure branch.
    #[test]
    fn device_call_is_a_trace_entry_and_failure_warns() {
        let package = ShaderPackage::from_entries_for_test(&[(
            "A.vso",
            &[0x00, 0x02, 0xFE, 0xFF, 0xFF, 0xFF, 0, 0],
        )]);
        let mut services = ShaderProgramServices {
            package: Some(Arc::new(package)),
            has_device: true,
            shader_directory: "data\\shaders\\".to_owned(),
            ..ShaderProgramServices::default()
        };
        let program = load_vertex_program(&mut services, "x", &[], "vs_2_0", "A.vso").unwrap();
        assert_eq!(services.device_calls[0].program_name, "A.vso");
        assert_eq!(services.device_calls[0].bytes, 8);
        assert_eq!(program.original_vtable, 0x00A8_B41C);
        let bytecode = program.bytecode.as_ref().unwrap();
        assert!(bytecode.is_vertex());
        assert_eq!(bytecode.version(), (2, 0));
        services.device_result = Some(Box::new(|_| 0x8876_086C));
        assert!(load_vertex_program(&mut services, "x", &[], "vs_2_0", "A.vso").is_none());
        assert_eq!(
            services.warnings,
            ["SHADER ERROR : failed to create vertex shader : data\\shaders\\A.vso"]
        );
    }
}
