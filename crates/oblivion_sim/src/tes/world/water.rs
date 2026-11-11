//! Phase 09 D6: 0043F560 TES_InitializeWater, called at 00406644 right after the TES constructor: the TESWaterSurfaceRecord (TES +0x54; 0049CD10 builds WaterRoot 00B35230 under the TES scene
//! root and the WaterShader statics), the "Interior Water Node" record (TES +0x58) and one "Water Node" record per GridCellArray slot, each a NiNode with its alpha/material/z-buffer/stencil
//! properties and a grid of NiTriShapes over one shared NiTriShapeData (0049E610 -> 0049B710, 0049DD00, 0049D2A0), then, with bUseWaterShader, g_shaderImageSpaceEffects and render path 2+,
//! BSShaderManager_PrepareObjectGeometry(WaterRoot, 17, 0, 1) attaches a WaterShaderProperty to every water geometry (`bs::sky_shader`). Without the water shader each shape gets a
//! NiTexturingProperty whose base texture and NiFlipController frames come from TES::CreateTextureImage (00442890). State only: nothing is drawn (Phase 15+).
//!
//! x87: the main thread is 24-bit after 0040E865, so plain `f32` with the operands in the original order.
//! Evidence: evidence/phase09-window-2026-10-06/disasm-09e.txt (0043F560 and its callees), disasm-09g.txt, disasm-09h.txt, disasm-09l.txt, disasm-09m.txt; P09-RUN-001 (the W/A/P/D records at
//! 00406649).

use std::thread::ThreadId;

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;
use oblivion_core::x87;

use super::tes::Tes;
use crate::bs::scene_graphs::{AttachChildStatus, attach_scene_child, detach_scene_child};
use crate::bs::shader::BsShaderGlobals;
use crate::bs::sky_shader::{
    PrepareGeometryContext, WATER_SHADER_SLOT_INDEX, prepare_object_geometry,
};
use crate::bs::task_manager::TaskGlobals;
use crate::gamebryo::av_object::NiAvParent;
use crate::gamebryo::controller_time::set_controller_target;
use crate::gamebryo::mesh_geometry::{
    MeshTriShapePayload, construct_tri_shape, construct_tri_shape_data,
};
use crate::gamebryo::object_net::{AlphaProperty, NiObjectNet};
use crate::gamebryo::properties::{StencilProperty, ZBufferProperty, new_material_property};
use crate::gamebryo::scene_derived::attach_affected_node;
use crate::gamebryo::scene_node::{SceneNodePayload, spawn_scene_node};
use crate::gamebryo::scene_state::{attach_object_property, refresh_effect_state};
use crate::gamebryo::scene_update::{SceneUpdateRuntime, SceneUpdateStatus, update_scene_object};
use crate::gamebryo::stream_objects::NiClass;
use crate::gamebryo::texture::{
    FlipController, TextureController, TexturingMap, TexturingProperty, attach_flip_texture,
    spawn_texture_controller,
};
use crate::tes::forms_references::parent_cell;
use crate::tes::forms_world::{TesObjectCell, cell_grid_x, cell_grid_y};
use crate::tes::player::the_player;
use crate::tes::settings::{SettingCollection, SettingInfo};
use crate::tes::tes_grid::{cell_water_height, grid_globals};

/// IDA: NiTPointerList<WadingWaterData*> vtable stored by 0049CD10.
pub const WADING_WATER_LIST_VTABLE: u32 = 0x00A3_F3BC;
/// IDA: flt_A2FF44, the size 0043F66F passes for each grid record.
pub const WATER_GRID_NODE_SIZE: f32 = 2048.0;

// IDA: the INI statics 0043F560 and its callees read.
const GRIDS_TO_LOAD: u32 = 0x00B0_6A2C;
const SURFACE_TILE_SIZE: u32 = 0x00B0_7040;
const TILE_TEXTURE_DIVISOR: u32 = 0x00B0_7048;
const USE_WATER_SHADER: u32 = 0x00B0_7050;
const USE_WATER_HI_RES: u32 = 0x00B0_7058;
const SURFACE_FRAME_COUNT: u32 = 0x00B0_7088;
const USE_WATER_DEPTH: u32 = 0x00B0_70A0;
const NUM_DEPTH_GRIDS: u32 = 0x00B0_70E0;
const SURFACE_TEXTURE: u32 = 0x00B0_70F0;
// IDA: the two statics 0049DD00 / 0049D2A0 register (00444060 / 00404850).
const SURFACE_TEXTURE_SIZE: u32 = 0x00B3_5264;
const WATER_ALPHA: u32 = 0x00B3_524C;

/// IDA: 0049CD10 TESWaterSurfaceRecord_Constructor (0x4C, FormHeap by 0043F587; TES +0x54): +0x00..+0x14 six NiPointers (null), +0x18 0.0, +0x20 and +0x24 = 0, byte +0x28 = 0, byte +0x29 = 1,
/// +0x2C 0.0, +0x30 NiTPointerList<WadingWaterData*> (vtable A3F3BC, +0x34 head, +0x38 tail, +0x3C count = 0), +0x40 = 0, +0x44 0.0, +0x48 NiPointer (null). +0x1C and +0x2A/+0x2B stay
/// uninitialized (host zero). Completion: 100% of the constructor stores.
/// TODO: [ ] Name the fields with their WaterShader/wading consumers (Phase 15+, M5).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TesWaterSurfaceRecord {
    pub pointers_00: [Option<Entity>; 6],
    pub float_18: f32,
    pub unset_1c: u32,
    pub dword_20: u32,
    pub dword_24: u32,
    pub byte_28: u8,
    pub byte_29: u8,
    pub float_2c: f32,
    pub wading_list_vtable_30: u32,
    pub wading_head_34: u32,
    pub wading_tail_38: u32,
    pub wading_count_3c: u32,
    pub dword_40: u32,
    pub float_44: f32,
    pub pointer_48: Option<Entity>,
}

/// IDA: 0049D140 TESWaterNodeRecord_Constructor (0x38; TES +0x58 and each GridCellArray slot +0x04) then 0049AC60 Reset: +0x00 byte (1 after the reset, 0 after 00499FF0), +0x04 NiPointer<NiNode>
/// the water node, +0x08 the count-prefixed NiPointer<NiTriShape> array (+0x18 squared entries), +0x0C FormHeap copy of the texture base path, +0x10 NiPointer the base texture, +0x14 = 0, +0x18
/// grid count (0049E610), +0x1C NiPointer NiFlipController, +0x20 NiPointer (null), +0x24 NiPointer the NiAlphaProperty, +0x28/+0x2C FLT_MAX (flt_A32048), +0x30 0.0, byte +0x34 reset-done.
/// +0x01..+0x03 and +0x35..+0x37 stay uninitialized (host zero). Completion: 100% of the constructor/reset stores.
/// TODO: [ ] Name +0x14, +0x20, +0x28..+0x30 with their per-frame consumers (Phase 15+, M5).
#[derive(Clone, Debug, PartialEq)]
pub struct TesWaterNodeRecord {
    pub byte_00: u8,
    pub node_04: Option<Entity>,
    pub shapes_08: Vec<Option<Entity>>,
    pub shapes_allocated_08: bool,
    pub texture_path_0c: Option<Vec<u8>>,
    pub texture_10: Option<Entity>,
    /// 0049A000 (Phase 15, M5).
    pub cell_14: Option<Entity>,
    pub grid_count_18: u32,
    pub flip_controller_1c: Option<Entity>,
    pub pointer_20: Option<Entity>,
    pub alpha_24: Option<Entity>,
    pub float_28: f32,
    pub float_2c: f32,
    pub float_30: f32,
    pub reset_done_34: u8,
}

/// IDA: the water statics 0043F560 and its callees write (defaults are the .bss/.data values before the window, P09-RUN-001 record 8).
#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub struct WaterGlobals {
    /// NiPointer "WaterRoot".
    pub water_root_b35230: Option<Entity>,
    /// 0049CDE9 = 1.
    pub byte_b3521d: u8,
    /// 0049CF7A = 0.
    pub dword_b35220: u32,
    /// uNumDepthGrids << 5.
    pub depth_grids_b45dbc: u32,
    /// 00B45E14 ShallowColor, 00B45E24 DeepColor, 00B45E34 ReflectionColor (WaterShader constant map 007DB4B0).
    pub shallow_color_b45e14: [f32; 4],
    pub deep_color_b45e24: [f32; 4],
    pub reflection_color_b45e34: [f32; 4],
    /// Tile (vertex constant c8).
    pub tile_b45ea4: f32,
    /// 0049DD3B once guard (bit 0) of the uSurfaceTextureSize:Water static.
    pub surface_texture_guard_b3526c: u32,
    /// 0049D2DD/0049D51A once guards: bit 0 the static normal, bit 1 the fAlpha:Water static.
    pub tri_shape_guard_b35260: u32,
    pub normal_b35254: [f32; 3],
    /// IDA: N/A the values of the two statics when 004040E0 rejects them as duplicates (the original keeps them in the static objects).
    pub surface_texture_size_b35264: u32,
    pub alpha_b3524c: f32,
}

/// IDA: N/A what TES::CreateTextureImage (00442890) calls. The real ones are 00B35300 g_texturePaletteRef (`BsTexturePalette`), 00B33A04 g_FileFinder and 00701E00: the driver binds them when
/// bUseWaterShader is off or Phase 13 reaches the decals.
pub trait WaterTextureServices {
    /// IDA: see the words here, which name the addresses this relates to.
    /// 00B35300 vt+4 (find).
    fn palette_find(&mut self, world: &mut World, path: &[u8]) -> Option<Entity>;
    /// IDA: see the words here, which name the addresses this relates to.
    /// 00B35300 vt+8 (add).
    fn palette_add(&mut self, world: &mut World, path: &[u8], texture: Entity);
    /// IDA: see the words here, which name the addresses this relates to.
    /// 00B33A04 vt+4 FindFile(path, out, flags, -1) (00431020): the status and the resolved path. `None` when the singleton is null.
    fn find_file(&mut self, path: &[u8], flags: u32) -> Option<(u32, Vec<u8>)>;
    /// IDA: see the words here, which name the addresses this relates to.
    /// 00701E00 NiSourceTexture_CreateFromFile(path, 00B256D0 {6, 3, 2}, 1).
    fn create_from_file(&mut self, world: &mut World, path: &[u8]) -> Option<Entity>;
    /// IDA: see the words here, which name the addresses this relates to.
    /// 00B33A10 g_IOManager +0x30, the thread 0044295A compares with.
    fn io_thread_id(&self) -> ThreadId;
}

/// IDA: N/A the settings, globals and services 0043F560 reads. The INI and 00707370's runtime are arguments, as the TES block's other steps share them.
pub struct WaterServices<'a, 'b> {
    /// 00B45DB9, 00B45FD0.
    pub shader: Option<&'a mut BsShaderGlobals>,
    /// 00B42F3E g_shaderImageSpaceEffects.
    pub image_space_effects: u8,
    /// 00B42F48 g_renderPath.
    pub render_path: i32,
    /// 007B8940.
    pub prepare: Option<&'a mut PrepareGeometryContext<'b>>,
    /// 00442890's targets; `None` is a null g_texturePaletteRef.
    pub textures: Option<&'a mut dyn WaterTextureServices>,
    /// 004A7A60 DebugMessage_Warning(text).
    pub warning: &'a mut dyn FnMut(String),
}

// IDA: N/A a registered INI static (the original reads the static object).
fn ini_static(ini: &SettingCollection, address: u32) -> Result<&SettingInfo, GameError> {
    ini.find_address(address).ok_or(GameError::unclassified(
        0x0043F560,
        "0043F560: missing INI static",
    ))
}

// IDA: N/A a float static.
fn ini_float(ini: &SettingCollection, address: u32) -> Result<f32, GameError> {
    Ok(f32::from_bits(ini_static(ini, address)?.raw))
}

// IDA: N/A the host AttachChild must attach (the original never fails for a nonnull child).
fn require_attached(status: AttachChildStatus, site: &'static str) -> Result<(), GameError> {
    if status == AttachChildStatus::Attached {
        Ok(())
    } else {
        Err(GameError::from_legacy_message(site))
    }
}

// IDA: 0049CA50 TESWaterNodeRecord_ClearShapes: each nonnull +0x08 entry with a +0x1C parent is detached (vtable +0x88) and released, then the array is freed and +0x08 = 0. Completion: 100%.
fn clear_water_shapes(world: &mut World, record: &mut TesWaterNodeRecord) {
    if !record.shapes_allocated_08 {
        return;
    }
    for shape in std::mem::take(&mut record.shapes_08).into_iter().flatten() {
        if let Some(parent) = world.get::<NiAvParent>(shape).and_then(|link| link.0) {
            detach_scene_child(world, parent, shape);
        }
    }
    record.shapes_allocated_08 = false;
}

// IDA: 0049B710 TESWaterNodeRecord_BuildNode(name): only while +0x04 is null. Completion: 100% of the order and flag values.
fn build_water_node(
    world: &mut World,
    record: &mut TesWaterNodeRecord,
    name: &[u8],
) -> Result<(), GameError> {
    if record.node_04.is_some() {
        return Ok(());
    }
    // 0049B747 FormHeap(0xDC) NiNode(0); 0049B77B NiPointer assign +0x04.
    let node = spawn_scene_node(world, SceneNodePayload::default());
    record.node_04 = Some(node);
    // 0049B780..0049B7CE NiAlphaProperty (0x1C: 006FFC70, vtable, +0x18 = 0xEC, +0x1A = 0), +0x18 = (+0x18 & 0xDCED) | 0xED, attached; 0049B7D7 NiPointer assign +0x24.
    let mut alpha = AlphaProperty::default();
    alpha.flags_18 = (alpha.flags_18 & 0xDCED) | 0x00ED;
    let alpha = world
        .spawn((NiClass::AlphaProperty, NiObjectNet::default(), alpha))
        .id();
    attach_object_property(world, node, Some(alpha))?;
    record.alpha_24 = Some(alpha);
    // 0049B7DC..0049B850 NiMaterialProperty (007095A0), +0x54 += 2, then +0x1C, +0x20, +0x28, +0x24, +0x2C, +0x30 = 1.0 (ambient/diffuse white).
    let mut material = new_material_property(world);
    material.raw_state_54 += 2;
    material.values.ambient[0] = 1.0;
    material.values.ambient[1] = 1.0;
    material.values.diffuse[0] = 1.0;
    material.values.ambient[2] = 1.0;
    material.values.diffuse[1] = 1.0;
    material.values.diffuse[2] = 1.0;
    let material = world
        .spawn((NiClass::MaterialProperty, NiObjectNet::default(), material))
        .id();
    attach_object_property(world, node, Some(material))?;
    // 0049B855..0049B8A2 NiZBufferProperty (+0x18 = 0x0F), then (+0x18 & 0xFFFD) | 1, attached.
    let mut zbuffer = ZBufferProperty::default();
    zbuffer.flags_18 = (zbuffer.flags_18 & 0xFFFD) | 1;
    let zbuffer = world
        .spawn((NiClass::ZBufferProperty, NiObjectNet::default(), zbuffer))
        .id();
    attach_object_property(world, node, Some(zbuffer))?;
    // 0049B8A7..0049B8F1 NiStencilProperty (+0x1C = 0, +0x20 = -1, +0x18 = 0x4180), +0x18 |= 0x0C00, attached.
    let mut stencil = StencilProperty::default();
    stencil.flags_18 |= 0x0C00;
    let stencil = world
        .spawn((NiClass::StencilProperty, NiObjectNet::default(), stencil))
        .id();
    attach_object_property(world, node, Some(stencil))?;
    // 0049B8F6..0049B908 SetName(name, or "Water Node" for a null name; both callers pass one).
    world
        .get_mut::<NiObjectNet>(node)
        .expect("just spawned")
        .name = Some(name.to_vec());
    Ok(())
}

// IDA: 0049D2A0 TESWater_CreateTriShapeData(sizeX, sizeY, textureSize, repeat, normals, colors). The allocation failure branches (null data) are not reachable in the host. Completion: 100% of the
// values for the one call, with normals and colors.
#[allow(clippy::manual_clamp)] // 0049D5E2's two comparisons on the static, kept in the original order.
fn create_water_tri_shape_data(
    world: &mut World,
    size_x: f32,
    size_y: f32,
    repeat: u32,
    globals: &mut WaterGlobals,
    ini: &mut SettingCollection,
) -> Entity {
    // 0049D2E4..0049D3B6 FormHeap(0x30) positions: (x/2, y/2, 0), (-x/2, y/2, 0), (-x/2, -y/2, 0), (x/2, -y/2, 0) with x/2 = x * 0.5 (dbl_A2FAA0) rounded to float and -x/2 = (-x) * 0.5.
    let half_x = size_x * 0.5;
    let half_y = size_y * 0.5;
    let negative_x = -size_x * 0.5;
    let negative_y = -size_y * 0.5;
    let positions = vec![
        [half_x, half_y, 0.0],
        [negative_x, half_y, 0.0],
        [negative_x, negative_y, 0.0],
        [half_x, negative_y, 0.0],
    ];
    // 0049D3B9..0049D3D6 once (00B35260 bit 0): the static normal 00B35254 = (0, 0, 1).
    if globals.tri_shape_guard_b35260 & 1 == 0 {
        globals.tri_shape_guard_b35260 |= 1;
        globals.normal_b35254 = [0.0, 0.0, 1.0];
    }
    // 0049D3DA FormHeap(0x20) UVs; 0049D3FA FormHeap(0xC) triangles (0, 1, 2, 0, 2, 3).
    let triangles = vec![0, 1, 2, 0, 2, 3];
    // 0049D420..0049D4C5 with `normals`: FormHeap(0x30) four copies of the static normal.
    let normals = vec![globals.normal_b35254; 4];
    // 0049D4C8..0049D69D with `colors`: FormHeap(0x40) four NiColorA (0047EA50); once (00B35260 bit 1) fAlpha:Water (00B3524C, 0.5) is registered (00404850 -> 004040E0); the static is clamped to
    // [0, 1] (values above 1.0 -> 1.0, below 0.0 -> 0.0, NaN unchanged) and each colour is (1, 1, 1, fAlpha).
    if globals.tri_shape_guard_b35260 & 2 == 0 {
        globals.tri_shape_guard_b35260 |= 2;
        globals.alpha_b3524c = 0.5;
        ini.add(SettingInfo::new(
            WATER_ALPHA,
            "fAlpha:Water",
            0.5_f32.to_bits(),
            None,
        ));
    }
    if let Some(setting) = ini.find_address(WATER_ALPHA) {
        globals.alpha_b3524c = f32::from_bits(setting.raw);
    }
    if 1.0 < globals.alpha_b3524c {
        globals.alpha_b3524c = 1.0;
    } else if 0.0 > globals.alpha_b3524c {
        globals.alpha_b3524c = 0.0;
    }
    if let Some(setting) = ini.find_address_mut(WATER_ALPHA) {
        setting.raw = globals.alpha_b3524c.to_bits();
    }
    let colors = vec![[1.0, 1.0, 1.0, globals.alpha_b3524c]; 4];
    // 0049D6A1..0049D727 the UVs: v = 1.0 for repeat 1, else float(repeat): (v, v), (0, v), (0, 0), (v, 0).
    let v = if repeat == 1 {
        1.0
    } else {
        x87::u32_to_x87_float(repeat)
    };
    let uvs = vec![[v, v], [0.0, v], [0.0, 0.0], [v, 0.0]];
    // 0049D72A FormHeap(0x58) 0071FB40(4, positions, normals, colors, uvs, 1, 0, 2, triangles).
    construct_tri_shape_data(
        world, 4, positions, normals, colors, uvs, 1, 0, 2, triangles,
    )
}

// IDA: 0043F4D0 TES_UseWaterShader: bUseWaterShader && g_shaderImageSpaceEffects. Completion: 100%.
fn use_water_shader(ini: &SettingCollection, image_space_effects: u8) -> Result<bool, GameError> {
    Ok(ini_static(ini, USE_WATER_SHADER)?.as_bool() && image_space_effects != 0)
}

// IDA: 0049DE42 / 0049DF7A "%s%02d.DDS". Completion: 100%.
fn texture_frame_name(base: &[u8], frame: u32) -> Vec<u8> {
    let mut name = base.to_vec();
    name.extend_from_slice(format!("{frame:02}.DDS").as_bytes());
    name
}

// IDA: 0049DD00 TESWaterNodeRecord_BuildGeometry(node, size). Completion: 100% of the order.
#[allow(clippy::too_many_lines)] // One original function; the store order reads top to bottom.
fn build_water_geometry(
    world: &mut World,
    record: &mut TesWaterNodeRecord,
    node: Entity,
    size: f32,
    globals: &mut WaterGlobals,
    ini: &mut SettingCollection,
    services: &mut WaterServices<'_, '_>,
) -> Result<(), GameError> {
    // 0049DD3B once (00B3526C bit 0): 00444060 registers uSurfaceTextureSize:Water (00B35264, 128) with 004040E0.
    if globals.surface_texture_guard_b3526c & 1 == 0 {
        globals.surface_texture_guard_b3526c |= 1;
        globals.surface_texture_size_b35264 = 128;
        ini.add(SettingInfo::new(
            SURFACE_TEXTURE_SIZE,
            "uSurfaceTextureSize:Water",
            128,
            None,
        ));
    }
    // 0049DD91 var_149 = 0 (frames missing); 0049DD96 0049CA50.
    let mut frames_missing = false;
    clear_water_shapes(world, record);
    // 0049DD9B..0049DE06 a count-prefixed NiPointer array of +0x18 squared entries -> +0x08 (none for a zero count: the function returns).
    let count = record.grid_count_18.wrapping_mul(record.grid_count_18);
    if count != 0 {
        record.shapes_08 = vec![None; count as usize];
        record.shapes_allocated_08 = true;
    }
    if !record.shapes_allocated_08 {
        return Ok(());
    }
    // 0049DE1A..0049DE2F var_140 = fSurfaceTileSize, var_128 = tile - size.
    let mut step = ini_float(ini, SURFACE_TILE_SIZE)?;
    let start = step - size;
    let shader_on = use_water_shader(ini, services.image_space_effects)?;
    if !shader_on {
        let base = record.texture_path_0c.clone().unwrap_or_default();
        // 0049DE42 "%s00.DDS" from +0x0C; 0049DE71 FindFile(buffer, buffer, 0, -1) rewrites the buffer (the resolved path, "" on failure).
        let mut buffer = texture_frame_name(&base, 0);
        if let Some(textures) = services.textures.as_deref_mut()
            && let Some((_, resolved)) = textures.find_file(&buffer, 0)
        {
            buffer = resolved;
        }
        // 0049DE73 +0x10 null: TES::CreateTextureImage(buffer, 1, 0).
        if record.texture_10.is_none() {
            record.texture_10 = create_tes_texture_image(world, &buffer, true, false, services)?;
        }
        // 0049DEDB +0x1C null: FormHeap(0x5C) NiFlipController (006D20C0).
        if record.flip_controller_1c.is_none() {
            let mut flip = FlipController::default();
            // 0049DF2A 006D1BC0(+0x10, 0); then frames 1.. below the uSurfaceFrameCount static, compacted into the next index.
            attach_flip_texture(&mut flip, record.texture_10, 0);
            let mut found = 1;
            let mut frame = 1;
            let mut limit = ini_static(ini, SURFACE_FRAME_COUNT)?.raw;
            while frame < limit {
                let name = texture_frame_name(&base, frame);
                let texture = create_tes_texture_image(world, &name, true, false, services)?;
                if texture.is_some() {
                    attach_flip_texture(&mut flip, texture, found);
                    found += 1;
                }
                frame += 1;
                limit = ini_static(ini, SURFACE_FRAME_COUNT)?.raw;
            }
            // 0049DFD3..0049DFF5: the static = frames found; var_149 = the old value > found; flags (& 0xFFF7) | 1, then & 0xFFF9.
            ini.find_address_mut(SURFACE_FRAME_COUNT)
                .expect("just read")
                .raw = found;
            frames_missing = limit > found;
            let controller = spawn_texture_controller(world, NiClass::FlipController, flip);
            let mut state = world
                .get_mut::<TextureController>(controller)
                .expect("just spawned");
            let flags = &mut state.time.fields.flags;
            *flags = (*flags & 0xFFF7) | 1;
            *flags &= 0xFFF9;
            record.flip_controller_1c = Some(controller);
        }
    }
    // 0049DFFB..0049E07F: repeat = chop(int(tile) / (textureSize * fTileTextureDivisor)) (at least 1, unsigned), the quad size tile + tile.
    let tile_int = x87::ftol(step);
    let texture_size = ini
        .find_address(SURFACE_TEXTURE_SIZE)
        .map_or(globals.surface_texture_size_b35264, |setting| setting.raw);
    let divisor = x87::u32_to_x87_float(texture_size) * ini_float(ini, TILE_TEXTURE_DIVISOR)?;
    let mut repeat = x87::ftol64_low(tile_int as f32 / divisor);
    if repeat < 1 {
        repeat = 1;
    }
    let quad = step + step;
    let data = create_water_tri_shape_data(world, quad, quad, repeat, globals, ini);
    // 0049E084..0049E0A9: y = start, z = 0.0, the step = tile + tile.
    let mut y = start;
    let z = 0.0;
    let mut shape_index = 0;
    step += step;
    for _ in 0..record.grid_count_18 {
        let mut x = start;
        for _ in 0..record.grid_count_18 {
            // 0049E0CD FormHeap(0xC0) 00717570 NiTriShape(data); 0049E112 NiPointer store at +0x08[index]; 0049E14F local translate.
            let shape = construct_tri_shape(world, Some(data));
            record.shapes_08[shape_index] = Some(shape);
            world
                .get_mut::<MeshTriShapePayload>(shape)
                .expect("just spawned")
                .local_transform
                .translation = [x, y, z];
            // 0049E164 without the water shader: a NiTexturingProperty (007043B0) with the +0x10 base texture (004057B0), base filter mode 2 (00405900), flags (& 0xFFF5) | 4, the flip controller
            // retargeted to it (vtable +0x58) with flags |= 8, attached.
            if !use_water_shader(ini, services.image_space_effects)? {
                let mut texturing = TexturingProperty::default();
                let map = texturing.maps_1c[0].get_or_insert_with(TexturingMap::default);
                map.texture_08 = record.texture_10;
                map.flags_04 = (map.flags_04 & 0xF0FF) | (2 << 8);
                texturing.flags_18 = (texturing.flags_18 & 0xFFF5) | 4;
                let property = world
                    .spawn((
                        NiClass::TexturingProperty,
                        NiObjectNet::default(),
                        texturing,
                    ))
                    .id();
                let flip = record.flip_controller_1c.ok_or(GameError::unclassified(
                    0x0049E1C7,
                    "0049E1C7: null +0x1C NiFlipController",
                ))?;
                // Vtable +0x58, 00715CE0 on NiFlipController, whose +0x60 (0069D990) is false.
                set_controller_target(world, flip, Some(property))?;
                world
                    .get_mut::<TextureController>(flip)
                    .expect("a flip controller")
                    .time
                    .fields
                    .flags |= 8;
                attach_object_property(world, shape, Some(property))?;
            }
            // 0049E1F1 node vtable +0x84 AttachChild(shape, 1).
            require_attached(
                attach_scene_child(world, node, Some(shape), true),
                "0049E1F1",
            )?;
            x += step;
            shape_index += 1;
        }
        y += step;
    }
    // 0049E238 the frames warning.
    if frames_missing {
        (services.warning)("Water art assets don't match .ini settings.".to_owned());
    }
    Ok(())
}

// IDA: 00442890 TES::CreateTextureImage(out, path, quiet, archiveOnly): the palette vt+4 lookup returns a hit; otherwise FindFile(path, null, 0 or 6 with archiveOnly, -1) failing (or a null
// g_FileFinder) with `quiet` returns null; then 0044295A..00442976 a thread other than g_IOManager +0x30 pauses the workers (00432860); 00701E00(path, 00B256D0, 1); 00432890 resumes them;
// a texture is added (vt+8), a failure without `quiet` warns "TES::CreateTextureImage unable to create image for "%s"." Completion: 100% of the order.
pub fn create_tes_texture_image(
    world: &mut World,
    path: &[u8],
    quiet: bool,
    archive_only: bool,
    services: &mut WaterServices<'_, '_>,
) -> Result<Option<Entity>, GameError> {
    // 004428C1 g_texturePaletteRef vt+4(path, 0): a hit is returned.
    let textures = services
        .textures
        .as_deref_mut()
        .ok_or(GameError::unclassified(
            0x004428CD,
            "004428CD: null g_texturePaletteRef",
        ))?;
    if let Some(texture) = textures.palette_find(world, path) {
        return Ok(Some(texture));
    }
    // 0044292E FindFile(path, null, archiveOnly ? 6 : 0, -1); a null g_FileFinder or status 0 with `quiet` returns null.
    let found = textures
        .find_file(path, if archive_only { 6 } else { 0 })
        .is_some_and(|(status, _)| status != 0);
    if !found && quiet {
        return Ok(None);
    }
    // 0044295A..00442976 only another thread than g_IOManager +0x30 pauses the workers.
    let paused = std::thread::current().id() != textures.io_thread_id();
    if paused {
        world
            .get_resource_mut::<TaskGlobals>()
            .ok_or(GameError::unclassified(
                0x00432860,
                "00432860: a null g_IOManager",
            ))?
            .pause_workers()?;
    }
    let texture = textures.create_from_file(world, path);
    if paused {
        world
            .get_resource_mut::<TaskGlobals>()
            .ok_or(GameError::unclassified(
                0x00432890,
                "00432890: a null g_IOManager",
            ))?
            .resume_workers()?;
    }
    // 004429A5 palette vt+8(path, texture); 004429C2 the warning.
    if let Some(texture) = texture {
        textures.palette_add(world, path, texture);
    } else if !quiet {
        let text = String::from_utf8_lossy(path).into_owned();
        (services.warning)(format!(
            "TES::CreateTextureImage unable to create image for \"{text}\"."
        ));
    }
    Ok(texture)
}

// IDA: 0049CD10 TESWaterSurfaceRecord_Constructor. Completion: 100% of the store order (the WaterRoot branch included).
fn construct_water_surface_record(
    world: &mut World,
    scene_root: Option<Entity>,
    globals: &mut WaterGlobals,
    ini: &SettingCollection,
    services: &mut WaterServices<'_, '_>,
) -> Result<TesWaterSurfaceRecord, GameError> {
    // 0049CD3F..0049CD6B: +0x00..+0x14, +0x3C, +0x34, +0x38 = 0, the +0x30 list vtable, +0x48 = 0, +0x18 = 0.0; 0049CD99 +0x40 = 0 (the two NiPointer null assigns at 0049CD69/0049CDB9 have no old
    // owner).
    let mut record = TesWaterSurfaceRecord {
        pointers_00: [None; 6],
        float_18: 0.0,
        unset_1c: 0,
        dword_20: 0,
        dword_24: 0,
        byte_28: 0,
        byte_29: 1,
        float_2c: 0.0,
        wading_list_vtable_30: WADING_WATER_LIST_VTABLE,
        wading_head_34: 0,
        wading_tail_38: 0,
        wading_count_3c: 0,
        dword_40: 0,
        float_44: 0.0,
        pointer_48: None,
    };
    // 0049CD9C..0049CDB3 00B45DB9 = bUseWaterDepth (byte); nonzero -> 00B45DBC = uNumDepthGrids << 5.
    let depth = ini_static(ini, USE_WATER_DEPTH)?.raw as u8;
    if let Some(shader) = services.shader.as_deref_mut() {
        shader.water_b45db9 = depth;
    }
    if depth != 0 {
        globals.depth_grids_b45dbc = ini_static(ini, NUM_DEPTH_GRIDS)?.raw << 5;
    }
    // 0049CDE9 00B3521D = 1; 0049CDE3 a null WaterRoot (00B35230) gets FormHeap(0xDC) NiNode(0) (NiPointer assign), "WaterRoot", and the TES +0x0C scene root's vtable +0x84 AttachChild(root, 0).
    globals.byte_b3521d = 1;
    if globals.water_root_b35230.is_none() {
        let root = spawn_scene_node(world, SceneNodePayload::default());
        globals.water_root_b35230 = Some(root);
        world
            .get_mut::<NiObjectNet>(root)
            .expect("just spawned")
            .name = Some(b"WaterRoot".to_vec());
        let scene_root = scene_root.ok_or(GameError::unclassified(
            0x0049CE8F,
            "0049CE8F: null TES +0x0C scene root",
        ))?;
        require_attached(
            attach_scene_child(world, scene_root, Some(root), false),
            "0049CE8F",
        )?;
    }
    // 0049CE93..0049CF57 ShallowColor (0, 0.5, 0.5, 1), DeepColor (0, 0, 0.1, 1), ReflectionColor (1, 1, 1, 1) in address order.
    globals.shallow_color_b45e14 = [0.0, 0.5, 0.5, 1.0];
    globals.deep_color_b45e24 = [0.0, 0.0, f32::from_bits(0x3DCC_CCCD), 1.0];
    globals.reflection_color_b45e34 = [1.0, 1.0, 1.0, 1.0];
    // 0049CF24..0049CF74: Tile = float(ftol(fSurfaceTileSize / (fTileTextureDivisor * 128.0))); 00B45FD0 = bUseWaterHiRes (stored first).
    let tile = ini_float(ini, SURFACE_TILE_SIZE)? / (ini_float(ini, TILE_TEXTURE_DIVISOR)? * 128.0);
    if let Some(shader) = services.shader.as_deref_mut() {
        shader.water_hi_res_b45fd0 = ini_static(ini, USE_WATER_HI_RES)?.as_bool();
    }
    globals.tile_b45ea4 = x87::ftol(tile) as f32;
    // 0049CF7A..0049CF8F 00B35220 = 0, +0x20 = 0, +0x2C = 0.0, +0x24 = 0, +0x44 = 0.0, +0x28 = 0, +0x29 = 1.
    globals.dword_b35220 = 0;
    record.dword_20 = 0;
    record.float_2c = 0.0;
    record.dword_24 = 0;
    record.float_44 = 0.0;
    record.byte_28 = 0;
    record.byte_29 = 1;
    Ok(record)
}

// IDA: 0049D140 TESWaterNodeRecord_Constructor (+0x04, +0x10, +0x1C..+0x24 = 0, byte +0x34 = 0) then 0049D185 0049AC60 Reset. Completion: 100%.
#[must_use]
pub fn construct_water_node_record() -> TesWaterNodeRecord {
    let mut record = TesWaterNodeRecord {
        byte_00: 0,
        node_04: None,
        shapes_08: Vec::new(),
        shapes_allocated_08: false,
        texture_path_0c: None,
        texture_10: None,
        cell_14: None,
        grid_count_18: 0,
        flip_controller_1c: None,
        pointer_20: None,
        alpha_24: None,
        float_28: 0.0,
        float_2c: 0.0,
        float_30: 0.0,
        reset_done_34: 0,
    };
    reset_water_node_record(&mut record);
    record
}

// IDA: 0049AC60 TESWaterNodeRecord_Reset: once (byte +0x34): +0x34 = 1, +0x0C = +0x08 = 0, byte +0x00 = 1, +0x14 = 0, +0x10 released; always +0x1C and +0x20 released, +0x28 = +0x2C = FLT_MAX, +0x30
// = 0.0. Completion: 100%.
pub fn reset_water_node_record(record: &mut TesWaterNodeRecord) {
    if record.reset_done_34 == 0 {
        record.reset_done_34 = 1;
        record.texture_path_0c = None;
        record.shapes_08.clear();
        record.shapes_allocated_08 = false;
        record.byte_00 = 1;
        record.cell_14 = None;
        record.texture_10 = None;
    }
    record.flip_controller_1c = None;
    record.pointer_20 = None;
    record.float_28 = f32::MAX;
    record.float_2c = f32::MAX;
    record.float_30 = 0.0;
}

// IDA: 0049E610 TESWaterNodeRecord_Initialize(size, name). Completion: 100% of the order of the reached branches.
#[allow(clippy::too_many_arguments)] // The original's two arguments, plus the state its globals are here.
fn initialize_water_node_record(
    world: &mut World,
    record: &mut TesWaterNodeRecord,
    sun_light: Option<Entity>,
    size: f32,
    name: &[u8],
    globals: &mut WaterGlobals,
    ini: &mut SettingCollection,
    services: &mut WaterServices<'_, '_>,
    update: &mut dyn SceneUpdateRuntime,
) -> Result<(), GameError> {
    // 0049E624..0049E665 +0x18 = chop(size / fSurfaceTileSize), 0 -> 1. `fistp qword`, low dword (the C++ truncates to 32 bits): a tile size of 0 gives inf, the 64-bit indefinite's low dword 0, so one tile.
    let grid = x87::ftol64_low(size / ini_float(ini, SURFACE_TILE_SIZE)?);
    record.grid_count_18 = grid;
    if grid == 0 {
        record.grid_count_18 = 1;
    }
    // 0049E66C..0049E67D fTileTextureDivisor <= 0.0 -> 1.0 (the static).
    if ini_float(ini, TILE_TEXTURE_DIVISOR)? <= 0.0 {
        ini.find_address_mut(TILE_TEXTURE_DIVISOR)
            .expect("just read")
            .raw = 1.0_f32.to_bits();
    }
    // 0049E683..0049E6D3 "%s\\water\\%s" ("Textures", sSurfaceTexture) -> FormHeap copy at +0x0C.
    let surface = ini_static(ini, SURFACE_TEXTURE)?;
    let mut path = b"Textures\\water\\".to_vec();
    path.extend_from_slice(surface.text.as_deref().unwrap_or_default());
    record.texture_path_0c = Some(path);
    // 0049E6D8 0049B710(name); 0049E6EE 0049DD00(+0x04, size).
    build_water_node(world, record, name)?;
    let node = record.node_04.expect("just built");
    build_water_geometry(world, record, node, size, globals, ini, services)?;
    // 0049E707 WaterRoot vtable +0x84 AttachChild(+0x04, 1).
    let root = globals.water_root_b35230.ok_or(GameError::unclassified(
        0x0049E6F9,
        "0049E6F9: null WaterRoot",
    ))?;
    require_attached(
        attach_scene_child(world, root, Some(node), true),
        "0049E707",
    )?;
    // 0049E709..0049E71A the TES +0x18 sun light gains the node (00708E40).
    if let Some(light) = sun_light {
        attach_affected_node(world, light, node);
    }
    // 0049E722 00707980; 0049E732 00707370(0.0, 0).
    refresh_effect_state(world, node)?;
    let updated = update_scene_object(world, node, 0.0, false, update)?;
    if updated.status != SceneUpdateStatus::Updated {
        return Err(GameError::unclassified(
            0x0049E732,
            "0049E732: the water node update stopped",
        ));
    }
    Ok(())
}

// IDA: 00499FF0 TESWaterNodeRecord_Hide: byte +0x00 = 0, node +0x18 |= 1. Completion: 100%.
pub(crate) fn hide_water_node_record(
    world: &mut World,
    record: &mut TesWaterNodeRecord,
) -> Result<(), GameError> {
    record.byte_00 = 0;
    let node = record.node_04.ok_or(GameError::unclassified(
        0x00499FF3,
        "00499FF3: null water node",
    ))?;
    world
        .get_mut::<SceneNodePayload>(node)
        .ok_or(GameError::unclassified(
            0x00499FF3,
            "00499FF3: the water node has no payload",
        ))?
        .effective_av_flags |= 1;
    Ok(())
}

// IDA: 0043F560 TES_InitializeWater(tes). Completion: 100% of the order.
pub fn initialize_water(
    world: &mut World,
    tes: &mut Tes,
    ini: &mut SettingCollection,
    services: &mut WaterServices<'_, '_>,
    update: &mut dyn SceneUpdateRuntime,
) -> Result<(), GameError> {
    world.init_resource::<WaterGlobals>();
    world.resource_scope(|world, mut globals: Mut<WaterGlobals>| {
        let globals = &mut *globals;
        // 0043F587..0043F5B4 FormHeap(0x4C) 0049CD10 -> +0x54.
        tes.water_surface_54 = Some(construct_water_surface_record(
            world,
            tes.scene_root_0c,
            globals,
            ini,
            services,
        )?);
        // 0043F5AD..0043F613 a null +0x58 gets FormHeap(0x38) 0049D140, then 0049E610(float(uGridsToLoad << 11), "Interior Water Node") and 00499FF0.
        if tes.water_node_58.is_none() {
            let mut record = Box::new(construct_water_node_record());
            let extent = ini_static(ini, GRIDS_TO_LOAD)?.raw << 11;
            initialize_water_node_record(
                world,
                &mut record,
                tes.sun_light_18,
                x87::u32_to_x87_float(extent),
                b"Interior Water Node",
                globals,
                ini,
                services,
                update,
            )?;
            hide_water_node_record(world, &mut record)?;
            tes.water_node_58 = Some(record);
        }
        // 0043F618..0043F697 each GridCellArray slot (x outer, y inner, bounds re-read from uGridsToLoad): a null +0x04 gets a record (stored before 0049E610(2048.0, "Water Node")) and 00499FF0.
        let sun_light = tes.sun_light_18;
        let cells = tes.grid_cells_08.as_mut().ok_or(GameError::unclassified(
            0x0043F62A,
            "0043F62A: null TES +0x08 GridCellArray",
        ))?;
        let mut x = 0;
        while x < ini_static(ini, GRIDS_TO_LOAD)?.raw {
            let mut y = 0;
            while y < ini_static(ini, GRIDS_TO_LOAD)?.raw {
                let slot = cells.slot_at(x, y)?;
                if slot.water_04.is_none() {
                    let mut record = Box::new(construct_water_node_record());
                    initialize_water_node_record(
                        world,
                        &mut record,
                        sun_light,
                        WATER_GRID_NODE_SIZE,
                        b"Water Node",
                        globals,
                        ini,
                        services,
                        update,
                    )?;
                    hide_water_node_record(world, &mut record)?;
                    slot.water_04 = Some(record);
                }
                y += 1;
            }
            x += 1;
        }
        // 0043F699..0043F6C5 bUseWaterShader, g_shaderImageSpaceEffects and g_renderPath >= 2: 007B8940(WaterRoot (0049A140), 17, 0, 1).
        if ini_static(ini, USE_WATER_SHADER)?.as_bool()
            && services.image_space_effects != 0
            && services.render_path >= 2
        {
            let prepare = services
                .prepare
                .as_deref_mut()
                .ok_or(GameError::unclassified(
                    0x0043F6C0,
                    "0043F6C0: no PrepareGeometry context",
                ))?;
            prepare_object_geometry(
                world,
                globals.water_root_b35230,
                WATER_SHADER_SLOT_INDEX,
                0,
                1,
                prepare,
            )?;
        }
        Ok(())
    })
}

// IDA: 0049A000 TESWaterNodeRecord_SetCell(cell). A new nonnull cell with a +0x08 shape array: 004CAFC0 (its answer dropped), the node's +0x54..+0x5C = ((gx << 12) + 0x800, (gy << 12) + 0x800, the water height under +0x24 bit 1 else 0.0), 00707370(0.0, 1) counted. +0x14 = cell. +0x00 and the node's +0x18 bit 0 follow bit 1. The player's parent cell with water sets 00B3521D. Completion: 100%.
pub(crate) fn set_water_node_cell(
    world: &mut World,
    record: &mut TesWaterNodeRecord,
    cell: Option<Entity>,
) -> Result<(), GameError> {
    if cell != record.cell_14 {
        if let Some(cell) = cell
            && !record.shapes_08.is_empty()
        {
            crate::tes::tes_grid::count(world, 0x004C_AFC0);
            let x = cell_grid_x(world, cell)
                .wrapping_shl(12)
                .wrapping_add(0x800) as f32;
            let y = cell_grid_y(world, cell)
                .wrapping_shl(12)
                .wrapping_add(0x800) as f32;
            let z = if world.get::<TesObjectCell>(cell).expect("a cell").flags_24 & 2 != 0 {
                cell_water_height(world, cell)
            } else {
                0.0
            };
            let node = record.node_04.ok_or(GameError::unclassified(
                0x0049A078,
                "0049A078: a null water node",
            ))?;
            world
                .get_mut::<SceneNodePayload>(node)
                .ok_or(GameError::unclassified(
                    0x0049A078,
                    "0049A078: the water node has no payload",
                ))?
                .local_transform
                .translation = [x, y, z];
            crate::tes::tes_grid::count(world, 0x0070_7370);
        }
        record.cell_14 = cell;
    }
    let water = cell
        .is_some_and(|cell| world.get::<TesObjectCell>(cell).expect("a cell").flags_24 & 2 != 0);
    record.byte_00 = u8::from(water);
    let node = record.node_04.ok_or(GameError::unclassified(
        0x0049A0BB,
        "0049A0BB: a null water node",
    ))?;
    let mut payload = world
        .get_mut::<SceneNodePayload>(node)
        .ok_or(GameError::unclassified(
            0x0049A0BB,
            "0049A0BB: the water node has no payload",
        ))?;
    if water {
        payload.effective_av_flags &= 0xFFFE;
    } else {
        payload.effective_av_flags |= 1;
    }
    let player_here = the_player(world).and_then(|player| parent_cell(world, player)) == cell;
    if player_here && water {
        grid_globals(world).water_player_b3521d = 1;
    }
    Ok(())
}

// IDA: 00498F30 sets 00B0703C and 00B3521D. Completion: 100%.
pub(crate) fn water_surface_on(world: &mut World) {
    let mut globals = grid_globals(world);
    globals.water_surface_b0703c = 1;
    globals.water_player_b3521d = 1;
}

// IDA: 0049A8B0(1): 00B0703C = 0, then its releases, a counted boundary. Completion: 100% of the flag.
pub(crate) fn water_surface_off(world: &mut World) {
    grid_globals(world).water_surface_b0703c = 0;
    crate::tes::tes_grid::count(world, 0x0049_A8B0);
}
