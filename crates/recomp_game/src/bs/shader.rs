//! Phase 05 row 05-D-4 (shared core, part 3): the BSShader object chain (NiShader 007385B0 -> NiD3DShaderInterface
//! 00779070 -> NiD3DShader / NiD3DDefaultShader 0076D040 -> BSShader 00802540 -> BSImageSpaceShader 008026C0), the
//! common virtuals the slot factories reach, the pass array SetSize 0076CCA0, the 8-byte slot pair 007AED00 and the
//! 007B4290 dispatch over 00B42EC0[28]. The class-specific parts live in `bs/shader_classes_*.rs`.

use std::any::Any;
use std::collections::BTreeMap;
use std::ops::Deref;
use std::sync::Arc;

use bevy_ecs::prelude::{Entity, Resource, World};
use recomp_core::reader::StreamError;

use crate::bs::shader_classes::bs_shader_factory;
use crate::bs::shader_classes_geometry::create_debug_slot_with;
use crate::bs::shader_programs::{ShaderProgramObject, ShaderProgramServices};
use crate::gamebryo::renderer::RendererState;
use crate::gamebryo::shader_runtime::{
    ConstantMapId, PassId, ShaderConstantMap, ShaderConstantMapEntry, ShaderDeclaration,
    ShaderRuntimePools, add_pass_reference, release_pass_reference,
};

/// IDA: N/A a NiPointer<NiD3DVertexShader or NiD3DPixelShader>.
pub type ProgramSlot = Option<Arc<ShaderProgramObject>>;
/// IDA: N/A a NiPointer<NiD3DPass>.
pub type PassSlot = Option<PassId>;

// IDA: the ShadowLightShader globals (007C8F50, 007CD090, 007CEFD0, 007D1170
// and its builders, 007CC280): the programs 00B45290[129] (00B4528C[1..129])
// and 00B45088[129], the 419 passes 00B455A0.., the per-pass tables
// 00B43490, 00B43B20, 00B441B0, 00B44840 (419 dwords each) and the map-entry
// NiPointer globals keyed by their original address.
#[derive(Clone, Debug)]
pub struct ShadowLightGlobals {
    pub vertex_b45290: Vec<ProgramSlot>,
    pub pixel_b45088: Vec<ProgramSlot>,
    pub passes_b455a0: Vec<PassSlot>,
    pub tables: [Vec<u32>; 4],
    /// Lookups by address only, so the `BTreeMap`'s order is never observed.
    pub entries: BTreeMap<u32, Option<Arc<ShaderConstantMapEntry>>>,
}

impl Default for ShadowLightGlobals {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            vertex_b45290: vec![None; 129],
            pixel_b45088: vec![None; 129],
            passes_b455a0: vec![None; 419],
            tables: [vec![0; 419], vec![0; 419], vec![0; 419], vec![0; 419]],
            entries: BTreeMap::new(),
        }
    }
}

// IDA: the pass globals of the ShadowLight subclasses: SkinShader 00B47718
// (20), SpeedTreeBranchShader 00B47790 (28), ParallaxShader 00B47620 (60).
#[derive(Clone, Debug)]
pub struct SkinGlobals {
    pub passes_b47718: Vec<PassSlot>,
}
/// IDA: 00B47790 (28); see `SkinGlobals`.
#[derive(Clone, Debug)]
pub struct BranchGlobals {
    pub passes_b47790: Vec<PassSlot>,
}
/// IDA: 00B47620 (60); see `SkinGlobals`.
#[derive(Clone, Debug)]
pub struct ParallaxGlobals {
    pub passes_b47620: Vec<PassSlot>,
}

// IDA: the Lighting30Shader globals (007FAC40, 007FB250, 007FD560, 007FDDE0,
// 007FEC30/0085E660, 007FCC80).
#[derive(Clone, Debug)]
pub struct Lighting30Globals {
    pub vertex_b47288: Vec<ProgramSlot>,
    pub pixel_b46ed8: Vec<ProgramSlot>,
    pub pixel_ll_b46c20: Vec<ProgramSlot>,
    pub passes_b473d0: Vec<PassSlot>,    // 00B473D0..00B474A4
    pub flags_b46988: Vec<u32>,          // 0085E660 per pass
    pub second_b46a60: Vec<u32>,         // 0085E660 per pass
    pub vectors_b46cc8: Vec<[f32; 4]>,   // 007FB250 (0, 0, 0, 1)
    pub constants_b46f78: Vec<[f32; 4]>, // src__24 block
    pub fields_b46964: [u32; 8],         // 007FAC40 = 0
    pub byte_b46984: u8,
    pub bytes_b46930: Vec<u8>,  // 007FAC40 memset
    pub max_lights_b2dcfc: i32, // .data 14; 8 below path 7
    pub entry_b46b98: Option<Arc<ShaderConstantMapEntry>>, // WorldViewProjTranspose
    pub entry_b46b9c: Option<Arc<ShaderConstantMapEntry>>, // SkinWorldViewProjTranspose
    pub entry_b46ba4: Option<Arc<ShaderConstantMapEntry>>, // WorldViewTranspose
    pub entry_b46ba8: Option<Arc<ShaderConstantMapEntry>>, // SkinWorldViewTranspose
    pub entry_b46ba0: Option<Arc<ShaderConstantMapEntry>>, // BoneMatrix3
    pub entry_b46c14: Option<Arc<ShaderConstantMapEntry>>, // decal fade
    pub entry_b46c18: Option<Arc<ShaderConstantMapEntry>>, // decal proj
}

impl Default for Lighting30Globals {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            vertex_b47288: vec![None; 32],
            pixel_b46ed8: vec![None; 39],
            pixel_ll_b46c20: vec![None; 39],
            passes_b473d0: vec![None; 54],
            flags_b46988: vec![0; 54],
            second_b46a60: vec![0; 54],
            vectors_b46cc8: vec![[0.0; 4]; 33],
            constants_b46f78: vec![[0.0; 4]; 49],
            fields_b46964: [0; 8],
            byte_b46984: 0,
            bytes_b46930: vec![0; 0x31],
            max_lights_b2dcfc: 14,
            entry_b46b98: None,
            entry_b46b9c: None,
            entry_b46ba4: None,
            entry_b46ba8: None,
            entry_b46ba0: None,
            entry_b46c14: None,
            entry_b46c18: None,
        }
    }
}

// IDA: the shader-class globals the slot constructors write.
#[derive(Clone, Debug)]
pub struct BsShaderGlobals {
    pub tallgrass_b460d0: [f32; 4], // 00B460D0..00B460DC (007E8D61..007E8D89)
    pub hdr_b43328_cleared: bool,   // 00B43328 NiPointer released by 007C0330
    // 007E0ED0 WaterShaderHeightMap: 00B45FE0 the instance (the slot's index here, a host value), 00B45FC4 5.0,
    // 00B45FC0 90.0, 00B45FD8 0.5, 00B45FD4 1.0; 00B45FD0 (written only by
    // 00499EF0 and the 0043F560 TES_InitializeWater path, both after the
    // shaders, so .bss zero here) picks 00B45FC8/00B45FCC = 256/8 or 128/7;
    // 00B45FD1 = 1; 00B45FE4..00B45FF0 = 1 / 00B45FC8.
    pub water_height_map_b45fe0: Option<u32>,
    pub water_b45fc0: f32,
    pub water_b45fc4: f32,
    pub water_hi_res_b45fd0: bool,
    pub water_resolution_b45fc8: i32,
    pub water_passes_b45fcc: i32,
    pub water_b45fd1: u8,
    pub water_b45fd4: f32,
    pub water_b45fd8: f32,
    pub water_resolution_b45fe4: [f32; 4],
    // 007DE240 WaterShaderDisplacement: 00B45F54 the instance, byte 00B3FF00 =
    // 0, 00B45F44 BlendAmount 0.0, 00B45F58 RainVars (0.1, 0.6, 0.985, 0.01),
    // 00B45F68 WadingVars (0.4, 0.6, 0.985, 0.05).
    pub water_displacement_b45f54: Option<u32>,
    pub water_b3ff00: u8,
    pub water_blend_b45f44: f32,
    pub water_rain_b45f58: [f32; 4],
    pub water_wading_b45f68: [f32; 4],
    // 007DC9A0 WaterShader: 00B45DCC the instance, 00B45E64 Scroll, 00B45E68,
    // 00B45E54, 00B45E58 = 0.0, bytes 00B45DBB/00B45DC0 = 0, 00B45EC4
    // NormalOffset and 00B45EE4 DepthOffset = (0, 0, 0, 0), byte 00B45DB9 = 0
    // up to render path 2 (.bss zero otherwise).
    pub water_shader_b45dcc: Option<u32>,
    pub water_scroll_b45e64: f32,
    pub water_b45e68: f32,
    pub water_b45e54: f32,
    pub water_b45e58: f32,
    pub water_b45dbb: u8,
    pub water_b45dc0: u8,
    pub water_b45db9: u8,
    pub water_normal_offset_b45ec4: [f32; 4],
    pub water_depth_offset_b45ee4: [f32; 4],
    // 007BA440 SkyShader: the seven vectors 00B43168 (eyepos), 00B43178,
    // 00B43188, 00B43198 (Color0..Color2), 00B431A8, 00B431B8, 00B431C8, each
    // src__25 (0, 0, 0, 1).
    pub sky_vectors_b43168: [[f32; 4]; 7],
    // 00806210 GeometryDecalShader: the declarations 00B47604 (2 entries, the
    // slot's) and 00B47608 (4 entries, skinned).
    pub decal_declaration_b47604: Option<Arc<ShaderDeclaration>>,
    pub decal_skin_declaration_b47608: Option<Arc<ShaderDeclaration>>,
    // 007F1200 SpeedTreeLeafShader: 00B46758/00B46768 = (1, 1, 1, 1),
    // 00B46778/00B46788 = 0.
    pub leaf_vectors_b46758: [[f32; 4]; 4],
    // 007E2D90/007E3160 particle count 00B4600C and 007F3890/007F3C10 bolt
    // count 00B468FC (set when 0: 40/120 and 75/235 by render path).
    pub particle_count_b4600c: i32,
    pub bolt_count_b468fc: i32,
    // PrecipitationShader's static programs and pass: 00B466E0[4] (vertex),
    // 00B46708[2] (pixel), 00B46704 (the pool pass).
    pub precipitation_vertex_b466e0: [ProgramSlot; 4],
    pub precipitation_pixel_b46708: [ProgramSlot; 2],
    pub precipitation_pass_b46704: PassSlot,
    pub lighting30: Lighting30Globals,
    pub shadow_light: ShadowLightGlobals,
    pub skin: SkinGlobals,
    pub branch: BranchGlobals,
    pub parallax: ParallaxGlobals,
}

impl Default for BsShaderGlobals {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            tallgrass_b460d0: [0.0; 4],
            hdr_b43328_cleared: false,
            water_height_map_b45fe0: None,
            water_b45fc0: 0.0,
            water_b45fc4: 0.0,
            water_hi_res_b45fd0: false,
            water_resolution_b45fc8: 0,
            water_passes_b45fcc: 0,
            water_b45fd1: 0,
            water_b45fd4: 0.0,
            water_b45fd8: 0.0,
            water_resolution_b45fe4: [0.0; 4],
            water_displacement_b45f54: None,
            water_b3ff00: 0,
            water_blend_b45f44: 0.0,
            water_rain_b45f58: [0.0; 4],
            water_wading_b45f68: [0.0; 4],
            water_shader_b45dcc: None,
            water_scroll_b45e64: 0.0,
            water_b45e68: 0.0,
            water_b45e54: 0.0,
            water_b45e58: 0.0,
            water_b45dbb: 0,
            water_b45dc0: 0,
            water_b45db9: 0,
            water_normal_offset_b45ec4: [0.0; 4],
            water_depth_offset_b45ee4: [0.0; 4],
            sky_vectors_b43168: [[0.0; 4]; 7],
            decal_declaration_b47604: None,
            decal_skin_declaration_b47608: None,
            leaf_vectors_b46758: [[0.0; 4]; 4],
            particle_count_b4600c: 0,
            bolt_count_b468fc: 0,
            precipitation_vertex_b466e0: Default::default(),
            precipitation_pixel_b46708: Default::default(),
            precipitation_pass_b46704: None,
            lighting30: Lighting30Globals::default(),
            shadow_light: ShadowLightGlobals::default(),
            skin: SkinGlobals {
                passes_b47718: vec![None; 20],
            },
            branch: BranchGlobals {
                passes_b47790: vec![None; 28],
            },
            parallax: ParallaxGlobals {
                passes_b47620: vec![None; 60],
            },
        }
    }
}

/// IDA: 0076C910 stage textures: the NiSourceTexture behind an effect-texture global (00B430D4, 00B430EC, 00B43110), read when the
/// stage is built; None when absent. `EffectTextureGlobals::stage_texture` is the one for a real run.
pub type EffectTextureService = Box<dyn FnMut(u32) -> Option<Entity> + Send + Sync>;

/// IDA: N/A the globals and services the 27 slots read, with the pools and maps that the originals keep in globals.
#[derive(Resource, Default)]
pub struct BsShaderContext {
    pub pools: ShaderRuntimePools,
    pub renderer: Option<Arc<RendererState>>, // 00B43104 g_shaderRendererRef (null = no slots)
    pub render_path: i32,                     // 00B42F48
    pub ps2x_path: i32,                       // 00B42D74
    pub hdr: bool,                            // 00B43070
    pub shaders_30: bool,                     // 00B42EA5
    pub fp16_filtering: bool,                 // 00B43071 g_rendererFP16Filtering
    pub anisotropic_min_filter: bool,         // 00B43072
    pub sun_brightness: f32,                  // 00B43154 (WinMain 0040EA26/0040EA79)
    pub sky_brightness: f32,                  // 00B43158 (WinMain 0040EA32/0040EA88)
    pub shader_model_class: i32,              // 00B42E88 g_rendererShaderModelClass
    pub specular_multiplier: f32,             // 00B2C2BC
    pub emissive_multiplier: f32,             // 00B2C2C0
    pub effect_texture: Option<EffectTextureService>,
    /// Host record: every effect-texture global 0076C910 was asked for, in order.
    pub effect_texture_requests: Vec<u32>,
    /// Counted boundaries: calls that need `&mut World` and are not run here, by address, in order.
    pub counted_boundaries: Vec<u32>,
    /// Every constant map; slots hold `ConstantMapId`s.
    pub constant_maps: Vec<ShaderConstantMap>,
    pub programs: ShaderProgramServices, // 008014E0/00801C90
    pub globals: BsShaderGlobals,
}

impl BsShaderContext {
    /// IDA: N/A
    #[must_use]
    pub fn constant_map(&self, id: ConstantMapId) -> &ShaderConstantMap {
        &self.constant_maps[id.0 as usize]
    }
}

// IDA: the BSShader base layout shared by every slot class:
// NiShader +0 vtable, +4 count, +8/+0xC = 0; NiD3DShaderInterface +0x10
// device (renderer +0x280, AddRef), +0x14 renderer, +0x18 renderer +0x8AC,
// +0x1C byte initialized, +0x1D byte; NiD3DShader +0x20/+0x21 bytes, +0x24
// declaration (NiPointer), +0x28..+0x3C = 0, +0x40 NiTArray<NiPointer<
// NiD3DPass>> (capacity 2, grow 2), +0x64..+0x6C = 0; BSShader +0x70 the
// current pass (NiPointer<NiD3DPass>), +0x78 byte 0; BSImageSpaceShader
// +0x7C NiPointer = 0, +0x80..+0x8C = 0.
#[derive(Clone, Debug)]
pub struct BsShaderObject {
    pub original_vtable: u32,
    pub class_name: &'static str, // RTTI name (host)
    pub field_08: u32,
    pub field_0c: u32,
    pub has_device: bool,                     // +0x10 was non-null
    pub renderer: Option<Arc<RendererState>>, // +0x14
    pub initialized: u8,                      // +0x1C
    pub field_1d: u8,
    pub field_20: u8,
    pub field_21: u8,
    pub declaration: Option<Arc<ShaderDeclaration>>, // +0x24
    pub pixel_constant_map: Option<ConstantMapId>,   // +0x2C NiD3DSCM_Pixel
    pub vertex_constant_map: Option<ConstantMapId>,  // +0x30 NiD3DSCM_Vertex
    // +0x40 NiTArray<NiPointer<NiD3DPass>>.
    pub passes: Vec<PassSlot>,
    pub pass_capacity: u16,
    pub pass_size: u16,
    pub pass_count: u16,
    pub pass_grow: u16,
    pub current_pass: PassSlot, // +0x70
    pub field_78: u8,
    pub image_space: bool,        // BSImageSpaceShader subclass
    pub image_space_7c: u32,      // +0x7C (NiPointer, null)
    pub image_space_80: [u32; 4], // +0x80..+0x8C
}

impl Default for BsShaderObject {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            original_vtable: 0,
            class_name: "",
            field_08: 0,
            field_0c: 0,
            has_device: false,
            renderer: None,
            initialized: 0,
            field_1d: 0,
            field_20: 0,
            field_21: 0,
            declaration: None,
            pixel_constant_map: None,
            vertex_constant_map: None,
            passes: Vec::new(),
            pass_capacity: 2,
            pass_size: 0,
            pass_count: 0,
            pass_grow: 2,
            current_pass: None,
            field_78: 0,
            image_space: false,
            image_space_7c: 0,
            image_space_80: [0; 4],
        }
    }
}

/// IDA: N/A a slot class as `dynamic_cast` and the base-class view see it. Host substitution for the original's
/// class chain: the shared base is reached through `Deref`.
pub trait BsShaderClass: Any + Send + Sync {
    /// IDA: N/A
    fn base(&self) -> &BsShaderObject;
    /// IDA: N/A
    fn base_mut(&mut self) -> &mut BsShaderObject;
    /// IDA: N/A the class itself, for the one `dynamic_cast` (007CC...: `copy_shadow_light_maps`).
    fn as_any(&self) -> &dyn Any;
}

impl Deref for dyn BsShaderClass {
    type Target = BsShaderObject;
    /// IDA: N/A Rust's `Deref`.
    fn deref(&self) -> &BsShaderObject {
        self.base()
    }
}

// IDA: 00802540 BSShader constructor tail (after 0076D040) and 008026C0
// BSImageSpaceShader. Fills the shared base fields. Completion: 100%.
pub fn construct_bs_shader_base(shader: &mut BsShaderObject, image_space: bool) {
    // 007385B0 NiShader (+8/+0xC), 00779070 NiD3DShaderInterface (+0x10..+0x1D),
    // 0076D040 NiD3DShader/NiD3DDefaultShader (+0x20..+0x3C, the +0x40 pass
    // array (capacity 2, grow 2) cleared by 0076C8C0, +0x64..+0x6C).
    shader.field_08 = 0;
    shader.field_0c = 0;
    shader.has_device = false;
    shader.renderer = None;
    shader.initialized = 0;
    shader.field_1d = 0;
    shader.field_20 = 0;
    shader.field_21 = 0;
    shader.declaration = None;
    shader.passes = vec![None; 2];
    shader.pass_capacity = 2;
    shader.pass_size = 0;
    shader.pass_count = 0;
    shader.pass_grow = 2;
    // 00802540 BSShader: +0x70 (null pass), +0x78 = 0.
    shader.current_pass = None;
    shader.field_78 = 0;
    // 008026C0 BSImageSpaceShader: +0x7C NiPointer cleared, +0x80..+0x8C = 0.
    shader.image_space = image_space;
    shader.image_space_7c = 0;
    shader.image_space_80 = [0; 4];
}

// IDA: 0077A400 vt+0x54 SetShaderDeclaration (+0x24 NiPointer assign). Completion: 100%.
pub fn set_shader_declaration(shader: &mut BsShaderObject, value: Option<Arc<ShaderDeclaration>>) {
    // 0077A400.
    let same = match (&shader.declaration, &value) {
        (Some(old), Some(new)) => Arc::ptr_eq(old, new),
        (None, None) => true,
        _ => false,
    };
    if !same {
        shader.declaration = value;
    }
}

// IDA: the NiPointer<NiD3DPass> slot assign (equal skips; old released;
// new referenced). Completion: 100%.
pub fn assign_pass(pools: &mut ShaderRuntimePools, slot: &mut PassSlot, value: PassSlot) {
    if *slot == value {
        return;
    }
    release_pass_reference(pools, *slot);
    *slot = value;
    add_pass_reference(pools, *slot);
}

// IDA: 0076CCA0 NiTArray<NiPointer<NiD3DPass>>::SetSize(capacity): a new
// capacity releases the entries past it, copies the rest into a new array
// (the size clamps to the capacity) and releases the old array's refs. Completion: 100%.
pub fn set_pass_array_capacity(
    pools: &mut ShaderRuntimePools,
    shader: &mut BsShaderObject,
    capacity: u16,
) {
    if capacity == shader.pass_capacity {
        return;
    }
    // 0076CCBF..0076CD0A entries past the new capacity.
    if capacity < shader.pass_size {
        for index in capacity as usize..shader.pass_size as usize {
            if shader.passes[index].is_some() {
                release_pass_reference(pools, shader.passes[index]);
                shader.passes[index] = None;
                shader.pass_count -= 1;
            }
        }
        shader.pass_size = capacity;
    }
    // 0076CD10..0076CE23 the new array (copies referenced, then the old array
    // released: the counts net to zero).
    shader.pass_capacity = capacity;
    shader.passes.resize(capacity as usize, None);
}

// IDA: 007796F0(shader, renderer): false for a null renderer; 007791A0 ->
// 007790F0 BindRenderer (+0x14 renderer, +0x10 device, +0x18 render state)
// and +0x1C = 1. Completion: 100%.
pub fn bind_shader_renderer(context: &BsShaderContext, shader: &mut BsShaderObject) -> bool {
    // 007796F0..00779709.
    let Some(renderer) = &context.renderer else {
        return false;
    };
    shader.renderer = Some(Arc::clone(renderer));
    shader.has_device = context.programs.has_device;
    shader.initialized = 1;
    true
}

// IDA: 00801380 vt+0x9C: a pool pass into +0x70 (the temporary released).
// Completion: 100%.
pub fn create_default_pass(pools: &mut ShaderRuntimePools, shader: &mut BsShaderObject) -> bool {
    // 008013B2..008013FD: the temporary NiPointer from the acquire is
    // assigned to +0x70 and then released.
    let temporary = pools.acquire_pass();
    assign_pass(pools, &mut shader.current_pass, Some(temporary));
    release_pass_reference(pools, Some(temporary));
    true
}

// IDA: 008025F0 vt+0x84 (most classes): 007796F0(g_shaderRendererRef);
// on success vt+0x98, vt+0xA0, vt+0x9C, then 0076CCA0(passes, 1). The
// class supplies vt+0x98/+0xA0/+0x9C (00977C50 return true and 00801380
// for most). Completion: 100%.
pub fn initialize_bs_shader(context: &mut BsShaderContext, shader: &mut BsShaderObject) -> bool {
    // 008025F9..00802632. Every class's vt+0x98 and vt+0xA0 are 00977C50 (return true), and its vt+0x9C 00801380.
    if !bind_shader_renderer(context, shader) {
        return false;
    }
    let _ = create_default_pass(&mut context.pools, shader);
    set_pass_array_capacity(&mut context.pools, shader, 1);
    true
}

// IDA: 007AED00 the 8-byte slot pair {+0 declaration, +4 shader}, both
// NiPointers (cleared).
#[derive(Default)]
pub struct BsShaderSlot {
    pub declaration: Option<Arc<ShaderDeclaration>>,
    pub shader: Option<Arc<dyn BsShaderClass>>,
}

// IDA: 00B42EC0[28] (index 0 unused).
#[derive(Resource, Default)]
pub struct BsShaderSlots {
    pub slots: [Option<Arc<BsShaderSlot>>; 28],
    pub creation_order: Vec<u32>, // host record
}

// IDA: one slot factory (the 27 functions 007B4290 dispatches to).
pub type BsShaderFactory = fn(&mut BsShaderContext, &mut BsShaderSlots) -> Arc<BsShaderSlot>;

// IDA: 007B4290 BSShaderManager_GetShader(index): null past 27; an existing
// slot is returned; otherwise the case runs (4 creates slot 1 first; 7
// picks BlurShader 007B1070 below render path 2, else 007EAE80; 8 needs
// g_rendererHDR, 11/21/24 render path >= 2, 26 g_rendererShaders30; a
// failed gate stores null) and its result is stored. 100% of the dispatch;
// the factories are the class ports.
pub fn get_bs_shader_slot(
    slots: &mut BsShaderSlots,
    index: u32,
    context: &mut BsShaderContext,
) -> Option<Arc<BsShaderSlot>> {
    // 007B4295..007B42A7.
    if index > 27 {
        return None;
    }
    if let Some(slot) = &slots.slots[index as usize] {
        return Some(Arc::clone(slot));
    }
    // 007B42AD..007B4494 the gates.
    let gated_out = match index {
        4 => {
            let _ = get_bs_shader_slot(slots, 1, context); // 007B42EC
            false
        }
        8 => !context.hdr,
        11 | 21 | 24 => context.render_path < 2,
        26 => !context.shaders_30,
        _ => false,
    };
    if gated_out {
        slots.slots[index as usize] = None; // 007B4359
        return None;
    }
    // 007B42B9 default (index 0): the slot value, null.
    let factory = bs_shader_factory(index, context.render_path < 2)?;
    let slot = factory(context, slots);
    slots.slots[index as usize] = Some(Arc::clone(&slot));
    slots.creation_order.push(index);
    Some(slot)
}

// IDA: 007B4290 for a caller holding a `World`: slot 9's factory 007FA910 then runs 00707610 and 00707370 for real, in place. Every other
// slot needs no `World`, and the 007B42EC / 00803A30 nested requests never name slot 9.
// Completion: 100%.
pub fn get_bs_shader_slot_in_world(
    world: &mut World,
    slots: &mut BsShaderSlots,
    index: u32,
    context: &mut BsShaderContext,
) -> Result<Option<Arc<BsShaderSlot>>, StreamError> {
    if index != 9 || slots.slots[9].is_some() {
        return Ok(get_bs_shader_slot(slots, index, context));
    }
    let slot = create_debug_slot_with(context, Some(world))?;
    slots.slots[9] = Some(Arc::clone(&slot));
    slots.creation_order.push(9);
    Ok(Some(slot))
}

// IDA: 00803A30 BSImageSpaceShaderList (0x1C, FormHeap by 007BA2A7 into
// 00B42D7C): NiTPointerList<BSImageSpaceShader *> (vtable A93754; +4 head,
// +8 tail, +0xC count), +0x10/+0x14 NiPointers (released, null), +0x18 the
// CopyShader (a raw pointer). The list holds raw shader pointers in AddTail
// (00803790) order.
#[derive(Resource)]
pub struct ImageSpaceShaderList {
    pub original_vtable: u32,
    pub shaders: Vec<Arc<dyn BsShaderClass>>, // +4..+0xC
    pub field_10: u32,
    pub field_14: u32,
    pub copy_shader: Option<Arc<dyn BsShaderClass>>, // +0x18
}

// IDA: 00803A30: Nighteye (007B4290(18)); HDR (8) under g_rendererHDR else
// Blur (7); Refraction (21) under g_rendererRefraction (00B42E84); Gethit
// (11); each appended when the slot and its shader exist; +0x18 = slot 12's
// shader. Completion: 100%.
pub fn create_image_space_shader_list(
    slots: &mut BsShaderSlots,
    context: &mut BsShaderContext,
    refraction: bool,
) -> ImageSpaceShaderList {
    let mut list = ImageSpaceShaderList {
        original_vtable: 0x00A9_3754,
        shaders: Vec::new(),
        // 00803A5D..00803ACA: an empty list; +0x10/+0x14 released (null).
        field_10: 0,
        field_14: 0,
        copy_shader: None,
    };
    let mut append = |index: u32, slots: &mut BsShaderSlots, context: &mut BsShaderContext| {
        if let Some(shader) =
            get_bs_shader_slot(slots, index, context).and_then(|slot| slot.shader.clone())
        {
            list.shaders.push(shader); // 00803790 AddTail
        }
    };
    append(18, slots, context);
    let hdr_or_blur = if context.hdr { 8 } else { 7 };
    append(hdr_or_blur, slots, context);
    if refraction {
        append(21, slots, context);
    }
    append(11, slots, context);
    if let Some(copy) = get_bs_shader_slot(slots, 12, context) {
        list.copy_shader.clone_from(&copy.shader); // 00803B66
    }
    list
}

#[cfg(test)]
mod tests {
    use super::*;

    fn class_names(list: &ImageSpaceShaderList) -> String {
        list.shaders
            .iter()
            .map(|shader| format!("{} ", shader.class_name))
            .collect()
    }

    /// `verify_bs_shader_slots`: the 27 slot classes on a host renderer without a device (every program load fails;
    /// the passes, stages, states, tables and maps remain).
    #[test]
    fn verify_bs_shader_slots() {
        let renderer = RendererState {
            max_streams_6dc: 16,
            ..RendererState::default()
        };
        let mut context = BsShaderContext {
            renderer: Some(Arc::new(renderer)),
            render_path: 7,
            ps2x_path: 0,
            hdr: true,
            shaders_30: true,
            shader_model_class: 8,
            effect_texture: Some(Box::new(|_| Some(Entity::PLACEHOLDER))),
            ..BsShaderContext::default()
        };
        context.pools.max_texture_blend_stages = 8;
        context.pools.max_sampler_stages = 16;
        context.programs.file_time = Some(Box::new(|_| None));
        context.programs.open_file = Some(Box::new(|_| None));
        let mut slots = BsShaderSlots::default();
        let names = [
            "",
            "ShadowLightShader",
            "TallGrassShader",
            "DistantLODShader",
            "SpeedTreeBranchShader",
            "SpeedTreeFrondShader",
            "SpeedTreeLeafShader",
            "BlurShader_P20",
            "HDRShader",
            "DebugShader",
            "SkyShader",
            "GethitShader",
            "CopyShader",
            "HairShader",
            "SkinShader",
            "ParallaxShader",
            "GeometryDecalShader",
            "WaterShader",
            "NighteyeShader",
            "WaterShaderHeightMap",
            "WaterShaderDisplacement",
            "RefractionShader",
            "ParticleShader",
            "BoltShader",
            "MapShader",
            "MenuBGShader",
            "Lighting30Shader",
            "PrecipitationShader",
        ];
        for index in 1..28u32 {
            let slot = get_bs_shader_slot(&mut slots, index, &mut context).expect("a slot");
            assert_eq!(
                slot.shader.as_ref().expect("a shader").class_name,
                names[index as usize],
                "007B4290 slot class"
            );
        }
        // Slot 4 creates slot 1 first; the order is the index order otherwise.
        assert_eq!(slots.creation_order.len(), 27);
        assert_eq!(slots.creation_order[0], 1);
        assert_eq!(slots.creation_order[3], 4);
        assert_eq!(slots.creation_order[26], 27);
        let globals = &context.globals;
        // The common builders release four ShadowLight passes: 415 of 419 remain, as P05-RUN-001 recorded.
        assert_eq!(
            globals.shadow_light.passes_b455a0.iter().flatten().count(),
            415
        );
        assert!(globals.skin.passes_b47718.iter().all(Option::is_some));
        assert!(globals.branch.passes_b47790.iter().all(Option::is_some));
        assert!(globals.parallax.passes_b47620.iter().all(Option::is_some));
        assert!(globals.lighting30.passes_b473d0.iter().all(Option::is_some));
        assert!(globals.precipitation_pass_b46704.is_some());
        // Lighting30 pass 0: 0085E570 seven stages; the first traced states.
        let l30 = context
            .pools
            .pass(globals.lighting30.passes_b473d0[0].unwrap());
        assert_eq!(l30.stage_count, 7);
        assert!(l30.render_states.is_some());
        assert_eq!(globals.lighting30.flags_b46988[0], 2066, "0085E660 pass 0");
        // HairShader's 0080C8B0 table stores and SkinShader's effect textures.
        assert_eq!(globals.shadow_light.tables[1][224], 0x2_0082);
        assert_eq!(globals.shadow_light.tables[2][225], 0x13C);
        assert!(context.effect_texture_requests.contains(&0x00B4_3110));
        // The map-entry globals (007CC280) and the constant maps shared by 00808DF0.
        assert!(globals.shadow_light.entries[&0x00B4_5018].is_some());
        assert!(globals.shadow_light.entries[&0x00B4_5038].is_none());
        let slot14 = slots.slots[14]
            .as_ref()
            .unwrap()
            .shader
            .as_ref()
            .unwrap()
            .vertex_constant_map;
        let slot1 = slots.slots[1]
            .as_ref()
            .unwrap()
            .shader
            .as_ref()
            .unwrap()
            .vertex_constant_map;
        assert!(slot14.is_some());
        assert_eq!(slot14, slot1, "The ShadowLight map globals");
        assert_eq!(globals.sky_vectors_b43168[0][3], 1.0);
        assert_eq!(globals.particle_count_b4600c, 120);
        assert_eq!(globals.bolt_count_b468fc, 235);
        assert_eq!(globals.water_resolution_b45fc8, 128);
        assert_eq!(globals.lighting30.max_lights_b2dcfc, 14);
        // 00803A30: Nighteye, HDR (g_rendererHDR) or Blur, Refraction (00B42E84), Gethit; +0x18 Copy.
        let full = create_image_space_shader_list(&mut slots, &mut context, true);
        context.hdr = false;
        let plain = create_image_space_shader_list(&mut slots, &mut context, false);
        assert_eq!(full.original_vtable, 0x00A9_3754);
        assert_eq!(
            class_names(&full),
            "NighteyeShader HDRShader RefractionShader GethitShader "
        );
        assert_eq!(
            class_names(&plain),
            "NighteyeShader BlurShader_P20 GethitShader "
        );
        let copy = slots.slots[12].as_ref().unwrap().shader.as_ref().unwrap();
        assert!(Arc::ptr_eq(full.copy_shader.as_ref().unwrap(), copy));
        assert!(Arc::ptr_eq(plain.copy_shader.as_ref().unwrap(), copy));
        assert_eq!((full.field_10, full.field_14), (0, 0));
        // 05-D-6: 007B4290 returns null for index 0 and > 27; a gated slot stores
        // null (007B4359) and is evaluated again on the next call.
        let mut gated = BsShaderSlots::default();
        assert!(get_bs_shader_slot(&mut gated, 0, &mut context).is_none());
        assert!(get_bs_shader_slot(&mut gated, 28, &mut context).is_none());
        assert!(get_bs_shader_slot(&mut gated, 8, &mut context).is_none());
        assert!(gated.slots[8].is_none());
        assert!(gated.creation_order.is_empty());
        context.hdr = true;
        let hdr = get_bs_shader_slot(&mut gated, 8, &mut context).expect("the gate is open");
        assert_eq!(hdr.shader.as_ref().unwrap().class_name, "HDRShader");
        assert!(Arc::ptr_eq(gated.slots[8].as_ref().unwrap(), &hdr));
        assert_eq!(gated.creation_order, [8]);
        // 007B4325..007B4486 below render path 2 and without SM3: slot 7 is
        // BlurShader, 11/21/24 store null, 26 stores null without 00B42EA5.
        let mut low = BsShaderSlots::default();
        context.render_path = 1;
        context.shaders_30 = false;
        let blur = get_bs_shader_slot(&mut low, 7, &mut context).expect("a slot");
        assert_eq!(blur.shader.as_ref().unwrap().class_name, "BlurShader");
        assert!(get_bs_shader_slot(&mut low, 11, &mut context).is_none());
        assert!(get_bs_shader_slot(&mut low, 21, &mut context).is_none());
        assert!(get_bs_shader_slot(&mut low, 24, &mut context).is_none());
        assert!(get_bs_shader_slot(&mut low, 26, &mut context).is_none());
        assert_eq!(low.creation_order, [7]);
    }
}
