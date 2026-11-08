//! What the sky prelude of 00406360 Startup_InitializeGameData (00406390..004065FF, WinMain 0040EB2C; the prelude itself is in
//! `recomp_startup::initialize_game_data`) reaches: the Sky singleton (00542EA0 -> 00540FD0), its initialization on g_skyNode with g_fogProperty
//! (005411D0: Atmosphere, Stars, Sun, Moons Root, Precipitation, Clouds, the root properties and the HDR floats) and the shadow scene light
//! (007C5850 / 007D3400). Its outputs feed the Phase 09 TES constructor at 0040662D.
//!
//! The Sky and its parts are plain structs inside the `SkyGlobals` resource, since nothing but Sky owns them; the Ni objects they point at are
//! entities. Original offsets are in the field names.
//!
//! Main-thread x87 after 0040E865 runs at 24-bit precision: the sun quad arithmetic is plain `f32` in the original grouping.
//! Evidence: evidence/phase08-scope-2026-10-06, evidence/phase08-window-2026-10-06 (decompile-a..f, disasm-sun-init.txt), P08-RUN-001.

use bevy_ecs::prelude::*;
use recomp_core::reader::StreamError;

use crate::bs::scene_graphs::{
    AttachChildStatus, SceneGlobalStore, ShadowSceneLight, ShadowSceneNode, attach_scene_child,
    attach_scene_property, construct_shadow_scene_light, detach_scene_child, set_object_name,
};
use crate::bs::sky_shader::{
    PrepareGeometryContext, SKY_SHADER_SLOT_INDEX, prepare_object_geometry,
};
use crate::bs::stream::{BsStreamLoad, BsStreamLoadStatus};
use crate::gamebryo::av_object::NiAvParent;
use crate::gamebryo::mesh_geometry::{
    MeshGeometryData, MeshTriShapeData, MeshTriShapePayload, NiGeometry, construct_tri_shape,
    construct_tri_shape_data,
};
use crate::gamebryo::mesh_tri_strips::MeshTriStripsData;
use crate::gamebryo::object_net::{AlphaProperty, NiObjectNet};
use crate::gamebryo::properties::VertexColorProperty;
use crate::gamebryo::scene_derived::{
    BillboardNode, DirectionalLight, spawn_billboard_node, spawn_directional_light,
};
use crate::gamebryo::scene_node::{NiNodeChildren, SceneNodePayload, spawn_scene_node};
use crate::gamebryo::stream_graph::despawn_stream_graph;
use crate::gamebryo::stream_objects::NiClass;
use crate::tes::game_settings::GameSettingCollection;

/// IDA: vtables stored by the sky constructors.
pub const SKY_VTABLE: u32 = 0x00A5_6E14;
pub const SKY_OBJECT_VTABLE: u32 = 0x00A5_70C8;
pub const SKY_ATMOSPHERE_VTABLE: u32 = 0x00A5_6B40;
pub const SKY_STARS_VTABLE: u32 = 0x00A5_7144;
pub const SKY_SUN_VTABLE: u32 = 0x00A5_71DC;
pub const SKY_CLOUDS_VTABLE: u32 = 0x00A5_6BE0;
pub const SKY_PRECIPITATION_VTABLE: u32 = 0x00A5_6D7C;
pub const NI_PICK_RESULTS_VTABLE: u32 = 0x00AA_39D8;

/// IDA: 00B36658 fSecondsBetweenWindowUpdate, which 00541165 reads.
const SECONDS_BETWEEN_WINDOW_UPDATE: u32 = 0x00B3_6658;

/// IDA: 00B25AC4 g_niColorWhite (NiColor 1, 1, 1).
const NI_COLOR_WHITE: [f32; 3] = [1.0, 1.0, 1.0];
/// IDA: 00B25AE0 NiColorA white (1, 1, 1, 1).
const NI_COLOR_A_WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
/// IDA: 00B25AD0 NiColorA (0, 0, 0, 1).
const NI_COLOR_A_BLACK: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

/// IDA: 0053B3F0 Atmosphere (0x1C, vtable A56B40; 0053AE60): SkyObject +0x04 root ("Atmosphere Root"), +0x08 NiPointer<NiAVObject> mesh (child 0 of
/// Atmosphere.nif, "Atmosphere Mesh"), +0x0C NiPointer<BSFogProperty> (the 005411D0 fog argument, stored after a successful load), +0x10 "Atmosphere
/// Quad Node" and +0x14 "Atmosphere Quad" (built without g_rendererHDR), +0x18 byte 1. Completion: 100% of the constructor and init fields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkyAtmosphere {
    pub original_vtable: u32,
    pub root_04: Option<Entity>,
    pub mesh_08: Option<Entity>,
    pub fog_0c: Option<Entity>,
    pub quad_node_10: Option<Entity>,
    pub quad_14: Option<Entity>,
    pub byte_18: u8,
}

impl Default for SkyAtmosphere {
    // IDA: 0053AE60 Atmosphere constructor (0053B3D0 deleting destructor). Completion: 100%.
    fn default() -> Self {
        Self {
            original_vtable: SKY_ATMOSPHERE_VTABLE,
            root_04: None,
            mesh_08: None,
            fog_0c: None,
            quad_node_10: None,
            quad_14: None,
            byte_18: 1,
        }
    }
}

/// IDA: 005442A0 Stars (0x10, vtable A57144): SkyObject +0x04 root ("Stars Root"), +0x08 NiPointer (null), +0x0C float 0.0. Completion: 100%.
#[derive(Clone, Debug, PartialEq)]
pub struct SkyStars {
    pub original_vtable: u32,
    pub root_04: Option<Entity>,
    pub pointer_08: Option<Entity>,
    pub float_0c: f32,
}

impl Default for SkyStars {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            original_vtable: SKY_STARS_VTABLE,
            root_04: None,
            pointer_08: None,
            float_0c: 0.0,
        }
    }
}

/// IDA: 00959BC0 NiPick (0x30; FormHeap by 00544EA0, a raw owner at Sun +0x18): +0x00 pick type, +0x04 sort type, +0x08 intersect type, +0x0C
/// coordinate type, +0x10 front-only byte, +0x11 observe-app-cull byte, +0x14 NiPointer root, +0x18 NiTArray<NiPick::Record*> (vtable AA39D8, +0x1C data,
/// +0x20 capacity, +0x22 size, +0x24 count, +0x26 grow 1), +0x28 last record, +0x2C..+0x2F return flags. Constructor values; 00544EA0 then stores +0x00 = 1,
/// +0x08 = 1, +0x10 = 1, +0x11 = 1. Completion: 100%.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkySunPick {
    pub pick_type_00: u32,
    pub sort_type_04: u32,
    pub intersect_type_08: u32,
    pub coordinate_type_0c: u32,
    pub front_only_10: u8,
    pub observe_app_cull_11: u8,
    pub root_14: Option<Entity>,
    pub results_vtable_18: u32,
    pub results_data_1c: u32,
    pub results_capacity_20: u16,
    pub results_size_22: u16,
    pub results_count_24: u16,
    pub results_grow_26: u16,
    pub last_record_28: u32,
    pub return_flags_2c: [u8; 4],
}

impl Default for SkySunPick {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            pick_type_00: 0,
            sort_type_04: 0,
            intersect_type_08: 1,
            coordinate_type_0c: 1,
            front_only_10: 1,
            observe_app_cull_11: 0,
            root_14: None,
            results_vtable_18: NI_PICK_RESULTS_VTABLE,
            results_data_1c: 0,
            results_capacity_20: 0,
            results_size_22: 0,
            results_count_24: 0,
            results_grow_26: 1,
            last_record_28: 0,
            return_flags_2c: [0; 4],
        }
    }
}

/// IDA: 00544B50 Sun (0x28, vtable A571DC): SkyObject +0x04 root ("Sun Root"), +0x08 "Sun Billboard Node", +0x0C "Sun Glare Billboard Node", +0x10 "Sun
/// Geometry", +0x14 "Sun Glare Geometry" (NiTriShape), +0x18 the NiPick (raw), +0x1C "Sun Directional Light", +0x20 float 0.0, +0x24 byte 0 (the +0x25..+0x27
/// bytes stay uninitialized). Completion: 100%.
#[derive(Clone, Debug, PartialEq)]
pub struct SkySun {
    pub original_vtable: u32,
    pub root_04: Option<Entity>,
    pub billboard_08: Option<Entity>,
    pub glare_billboard_0c: Option<Entity>,
    pub geometry_10: Option<Entity>,
    pub glare_geometry_14: Option<Entity>,
    pub pick_18: Option<SkySunPick>,
    pub light_1c: Option<Entity>,
    pub float_20: f32,
    pub byte_24: u8,
}

impl Default for SkySun {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            original_vtable: SKY_SUN_VTABLE,
            root_04: None,
            billboard_08: None,
            glare_billboard_0c: None,
            geometry_10: None,
            glare_geometry_14: None,
            pick_18: None,
            light_1c: None,
            float_20: 0.0,
            byte_24: 0,
        }
    }
}

/// IDA: 0053BC80 Clouds (0x18, vtable A56BE0): SkyObject +0x04 root ("Cloud Root"), +0x08 / +0x0C the two geometry layers (children 0 and 1 of Clouds.nif's
/// root NiNode), +0x10 / +0x14 two NiPointers (null; 0053BBC0 releases them). The constructor's 0053B6E0 / 0053BBC0 find null layers (no 00802890 call).
/// Completion: 100%.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkyClouds {
    pub original_vtable: u32,
    pub root_04: Option<Entity>,
    pub layers_08: [Option<Entity>; 2],
    pub pointers_10: [Option<Entity>; 2],
}

impl Default for SkyClouds {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            original_vtable: SKY_CLOUDS_VTABLE,
            root_04: None,
            layers_08: [None; 2],
            pointers_10: [None; 2],
        }
    }
}

/// IDA: 0053D520 Precipitation (0x18, vtable A56D7C; not a SkyObject): +0x04 and +0x08 NiPointers (null), +0x0C the raw g_weatherNode copy (0053D8F0),
/// +0x10 / +0x14 = 0. Completion: 100%.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkyPrecipitation {
    pub original_vtable: u32,
    pub pointer_04: Option<Entity>,
    pub pointer_08: Option<Entity>,
    pub weather_0c: Option<Entity>,
    pub unknown_10: u32,
    pub unknown_14: u32,
}

impl Default for SkyPrecipitation {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            original_vtable: SKY_PRECIPITATION_VTABLE,
            pointer_04: None,
            pointer_08: None,
            weather_0c: None,
            unknown_10: 0,
            unknown_14: 0,
        }
    }
}

/// IDA: 0053D260 Moon (0x7C, vtable A56CAC, over SkyObject): +0x04 root, +0x08..+0x14 NiPointers, +0x18 eight BSStringT phase textures (the slot at +0x38 stays empty), +0x58 fade start, +0x5C fade end, +0x60 shadow early fade, +0x64 speed, +0x68 Z offset, +0x6C size, +0x70 0, +0x74 0.0, +0x78 FLT_MAX. Completion: 100% of the constructor.
/// TODO: [ ] 0053C830 vt+0x10's quads, nodes and properties under Moons Root, with the sky renderer, M6.
#[derive(Clone, Debug, PartialEq)]
pub struct Moon {
    pub original_vtable: u32,
    pub textures: [String; 8],
    pub fade_start_58: f32,
    pub fade_end_5c: f32,
    pub shadow_early_fade_60: f32,
    pub speed_64: f32,
    pub z_offset_68: f32,
    pub size_6c: i32,
    pub byte_70: u32,
    pub float_74: f32,
    pub float_78: f32,
}

const MOON_VTABLE: u32 = 0x00A5_6CAC;

// IDA: 0053D260 and its call at 005433C2.. or 00543453..: the six settings as they stand at the call. 0044CD49 runs before any plugin, so the first call takes the exe's defaults whatever the GMSTs say later: fMasserAngleFadeStart stays 55 here and is 60 by load_end.
fn construct_moon(settings: &GameSettingCollection, name: &str) -> Moon {
    let value = |suffix: &str| {
        settings
            .find(&format!(
                "{}{name}{suffix}",
                if suffix == "Size" { "i" } else { "f" }
            ))
            .map_or(0, |setting| setting.raw)
    };
    let phase = |phase: &str| format!("\\Textures\\Sky\\{name}_{phase}.dds");
    Moon {
        original_vtable: MOON_VTABLE,
        textures: [
            phase("full"),
            phase("three_wan"),
            phase("half_wan"),
            phase("one_wan"),
            String::new(),
            phase("one_wax"),
            phase("half_wax"),
            phase("three_wax"),
        ],
        fade_start_58: f32::from_bits(value("AngleFadeStart")),
        fade_end_5c: f32::from_bits(value("AngleFadeEnd")),
        shadow_early_fade_60: f32::from_bits(value("AngleShadowEarlyFade")),
        speed_64: f32::from_bits(value("Speed")),
        z_offset_68: f32::from_bits(value("ZOffset")),
        size_6c: value("Size") as i32,
        byte_70: 0,
        float_74: 0.0,
        float_78: f32::MAX,
    }
}

/// IDA: 00540FD0 Sky (0x104, vtable A56E14; three-part Sky / Sky_mbrs / Sky_vtbl in the IDB): +0x04 NiPointer root, +0x08 NiPointer Moons Root, +0x0C first
/// climate, +0x10 first weather, +0x14 / +0x18 weathers, +0x1C weather override, +0x20 atmosphere, +0x24 stars, +0x28 sun, +0x2C clouds, +0x30 Masser, +0x34
/// Secunda, +0x38 precipitation (raw owners), +0x3C ten NiColor (zeroed, then white), +0xB4 NiColor (white), +0xC0 0.0, +0xC4 1.0, +0xD0 10.0, +0xD4 0.0, +0xD8 1.0,
/// +0xDC 4, +0xE0 the 8-byte active weather sound list {0, 0}, +0xE4 0.0, +0xEC 0, +0xF0 = *(00B36658) (10.0), +0xF4 0.0, +0xFC 32, +0x100 byte 0. +0xC8..+0xCF,
/// +0xE8, +0xF8 and +0x101..+0x103 stay uninitialized FormHeap bytes (host zero). Completion: 100% of the constructor values.
#[derive(Clone, Debug, PartialEq)]
pub struct Sky {
    pub original_vtable: u32,
    pub root_04: Option<Entity>,
    pub moons_root_08: Option<Entity>,
    pub first_climate_0c: u32,
    pub first_weather_10: u32,
    pub weather_14: u32,
    pub weather_18: u32,
    pub weather_override_1c: u32,
    pub atmosphere_20: Option<SkyAtmosphere>,
    pub stars_24: Option<SkyStars>,
    pub sun_28: Option<SkySun>,
    pub clouds_2c: Option<SkyClouds>,
    pub masser_30: Option<Moon>,
    pub secunda_34: Option<Moon>,
    pub precipitation_38: Option<SkyPrecipitation>,
    pub colors_3c: [[f32; 3]; 10],
    pub color_b4: [f32; 3],
    pub float_c0: f32,
    pub float_c4: f32,
    pub unset_c8: [u32; 2],
    pub float_d0: f32,
    pub float_d4: f32,
    pub float_d8: f32,
    pub dword_dc: u32,
    pub active_weather_sounds_e0: [u32; 2],
    pub float_e4: f32,
    pub unset_e8: u32,
    pub dword_ec: u32,
    pub float_f0: f32,
    pub float_f4: f32,
    pub unset_f8: u32,
    pub dword_fc: u32,
    pub byte_100: u8,
}

/// IDA: N/A the Sky globals: 00B365C4 (Sky_GetSingleton storage, raw owner), 00B333B0 (the prelude's copy, kept as whether it was published, since the host
/// has one Sky) and the HDR floats 00B2C73C / 00B2C740 (image value 1.0, stored again by 005411D0 under g_rendererHDR). `store_trace` records the global stores
/// in order (address, width).
#[derive(Resource, Clone, Debug, PartialEq)]
pub struct SkyGlobals {
    /// 00B365C4.
    pub singleton_b365c4: Option<Box<Sky>>,
    /// 00B333B0.
    pub published_b333b0: bool,
    pub hdr_b2c73c: f32,
    pub hdr_b2c740: f32,
    pub store_trace: Vec<SceneGlobalStore>,
}

impl Default for SkyGlobals {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            singleton_b365c4: None,
            published_b333b0: false,
            hdr_b2c73c: 1.0,
            hdr_b2c740: 1.0,
            store_trace: Vec::new(),
        }
    }
}

impl SkyGlobals {
    /// IDA: N/A trace one global store (address, width).
    pub fn trace(&mut self, address: u32, width: u8) {
        self.store_trace.push(SceneGlobalStore { address, width });
    }
}

/// IDA: N/A the services and settings 005411D0 and its part Init slots read.
pub struct SkyInitServices<'a> {
    /// 00B43070.
    pub renderer_hdr: bool,
    /// 00B11DE4 bPrecipitation:Weather.
    pub precipitation: bool,
    /// 00B11E2C fSunBaseSize:Weather.
    pub sun_base_size: f32,
    /// 00B11E34 fSunGlareSize:Weather.
    pub sun_glare_size: f32,
    /// 00B333CC (the non-HDR quad translate).
    pub world_scene: Option<Entity>,
    /// 00B333DC (0053D8F0).
    pub weather: Option<Entity>,
    /// 006F9980 BSStream_LoadFromPath with a fresh BSStream.
    pub load_stream:
        &'a mut (dyn FnMut(&mut World, &str) -> Result<BsStreamLoad, StreamError> + 'a),
    /// 004A7A60 DebugMessage_Warning(text), in call order.
    pub warnings: Vec<&'static str>,
}

// IDA: N/A AttachChild must attach (the original never fails for nonnull).
fn require_attached(status: AttachChildStatus, site: &'static str) -> Result<(), StreamError> {
    if status == AttachChildStatus::Attached {
        Ok(())
    } else {
        Err(StreamError(site))
    }
}

// IDA: N/A the +0x18 flags word of a bound class the sky code touches (NiNode family, NiTriShape, NiTriStrips).
fn or_av_flags(world: &mut World, object: Entity, bits: u16) -> Result<(), StreamError> {
    if let Some(mut node) = world.get_mut::<SceneNodePayload>(object) {
        node.effective_av_flags |= bits;
        return Ok(());
    }
    if let Some(mut mesh) = world.get_mut::<MeshTriShapePayload>(object) {
        mesh.effective_av_flags |= bits;
        return Ok(());
    }
    Err(StreamError(
        "NiAVObject +0x18 of an unbound sky child class",
    ))
}

// IDA: N/A the 006FF420 SetName target of a bound class.
fn name_object(world: &mut World, object: Entity, name: &[u8]) -> Result<(), StreamError> {
    let mut net = world.get_mut::<NiObjectNet>(object).ok_or(StreamError(
        "NiObjectNET +0x08 of an unbound sky child class",
    ))?;
    set_object_name(&mut net.name, Some(name));
    Ok(())
}

// IDA: 00405790 NiNode child slot `index` (below +0xB6, else null). Completion: 100%.
fn node_child(world: &World, node: Entity, index: usize) -> Option<Entity> {
    world
        .get::<NiNodeChildren>(node)
        .and_then(|children| children.slots_ac.get(index).copied().flatten())
}

// IDA: N/A the IsKindOf(NiNode) of a loaded root.
fn is_node(world: &World, object: Entity) -> bool {
    world
        .get::<NiClass>(object)
        .is_some_and(|class| class.is_node())
}

// IDA: N/A the +0xB4 geometry data of a NiTriShape or NiTriStrips slot; null for other classes (the cloud and vertex-colour code dereference it).
fn child_geometry_data(world: &World, child: Entity) -> Option<Entity> {
    matches!(
        world.get::<NiClass>(child),
        Some(NiClass::TriShape | NiClass::TriStrips)
    )
    .then(|| {
        world
            .get::<NiGeometry>(child)
            .and_then(|links| links.data_b4)
    })
    .flatten()
}

// IDA: N/A runs `edit` on the NiGeometryData base of a data entity, whichever subclass holds it.
fn with_geometry_data<R>(
    world: &mut World,
    data: Entity,
    edit: impl FnOnce(&mut MeshGeometryData) -> R,
) -> Option<R> {
    if let Some(mut owner) = world.get_mut::<MeshTriShapeData>(data) {
        return Some(edit(&mut owner.base));
    }
    world
        .get_mut::<MeshTriStripsData>(data)
        .map(|mut owner| edit(&mut owner.base))
}

// IDA: 00543D30 SkyObject vtable +0x08 Init(parent): FormHeap(0xDC) NiNode(0) into +0x04 (NiPointer assign), +0x18 |= 2 then |= 0x20, then the parent's
// vtable +0x84 AttachChild(+0x04, 1). Completion: 100% of the order.
fn initialize_sky_object(world: &mut World, parent: Option<Entity>) -> Result<Entity, StreamError> {
    let node = spawn_scene_node(world, SceneNodePayload::default());
    or_av_flags(world, node, 2)?;
    or_av_flags(world, node, 0x20)?;
    let parent = parent.ok_or(StreamError("00543DE6: null parent for AttachChild"))?;
    require_attached(
        attach_scene_child(world, parent, Some(node), true),
        "00543DE6",
    )?;
    Ok(node)
}

// IDA: 007BD0D0 the atmosphere quad (without g_rendererHDR): positions (-1, -1, 0.999999), (1, -1, z), (1, 1, z), (-1, 1, z) with z = A8F8E4 (0x3F7FFFEF),
// triangles (0,1,2) (3,0,2), four NiColorA 00B25AD0 (0, 0, 0, 1), 007174B0(4, positions, null, colors, null, 0, 0, 2, triangles), then the data bound
// (+0x0C..+0x18) = (0, 0, 0, 1000.0). Completion: 100% of the values.
fn build_atmosphere_quad(world: &mut World) -> Entity {
    let z = f32::from_bits(0x3F7F_FFEF);
    let data = construct_tri_shape_data(
        world,
        4,
        vec![
            [-1.0, -1.0, z],
            [1.0, -1.0, z],
            [1.0, 1.0, z],
            [-1.0, 1.0, z],
        ],
        Vec::new(),
        vec![NI_COLOR_A_BLACK; 4],
        Vec::new(),
        0,
        0,
        2,
        vec![0, 1, 2, 3, 0, 2],
    );
    let quad = construct_tri_shape(world, Some(data));
    // 007BD2D4..007BD30D: data +0x0C / +0x10 / +0x14 = 0, +0x18 = 1000.0.
    world
        .get_mut::<MeshTriShapeData>(data)
        .expect("just constructed")
        .base
        .bounds_0c = [0.0, 0.0, 0.0, 1000.0];
    quad
}

// IDA: N/A a failed load releases everything the stream holds, since nothing took a reference. The host despawns its graph.
fn release_load(world: &mut World, load: &BsStreamLoad) {
    if let Some(stream) = &load.stream {
        despawn_stream_graph(world, &stream.graph);
    }
}

// IDA: 0053B3F0 Atmosphere vtable +0x10 Init(parent, fog). Completion: 100% of the order.
// TODO: [ ] NiStream's release of a successful load's roots: properties and objects are not counted, so the emptied NIF root stays as an unparented entity.
fn initialize_atmosphere(
    world: &mut World,
    atmosphere: &mut SkyAtmosphere,
    parent: Option<Entity>,
    fog: Option<Entity>,
    services: &mut SkyInitServices<'_>,
) -> Result<(), StreamError> {
    // 0053B43C 00543D30(parent); 0053B44F SetName(+4, "Atmosphere Root").
    let root = initialize_sky_object(world, parent)?;
    atmosphere.root_04 = Some(root);
    name_object(world, root, b"Atmosphere Root")?;
    if !services.renderer_hdr {
        // 0053B45E..0053B4A6 007BD0D0 -> +0x14 (copy assign), "Atmosphere Quad", +0x18 |= 2.
        let quad = build_atmosphere_quad(world);
        atmosphere.quad_14 = Some(quad);
        name_object(world, quad, b"Atmosphere Quad")?;
        or_av_flags(world, quad, 2)?;
        // 0053B4C9..0053B4FD: g_worldSceneGraph child 0 (size +0xB6 nonzero) local translate copied to the quad; a null child is dereferenced.
        let scene = services
            .world_scene
            .ok_or(StreamError("0053B4C9: null g_worldSceneGraph"))?;
        let translation = node_child(world, scene, 0)
            .and_then(|child| world.get::<SceneNodePayload>(child))
            .ok_or(StreamError("0053B4D9: world child 0 is not a bound NiNode"))?
            .local_transform
            .translation;
        world
            .get_mut::<MeshTriShapePayload>(quad)
            .expect("just constructed")
            .local_transform
            .translation = translation;
        // 0053B507..0053B53A FormHeap(0xDC) NiNode(0) -> +0x10, "Atmosphere Quad Node", +0x18 |= 2; +0x10 AttachChild(+0x14, 0); root AttachChild(+0x10, 0).
        let quad_node = spawn_scene_node(world, SceneNodePayload::default());
        atmosphere.quad_node_10 = Some(quad_node);
        name_object(world, quad_node, b"Atmosphere Quad Node")?;
        or_av_flags(world, quad_node, 2)?;
        require_attached(
            attach_scene_child(world, quad_node, Some(quad), false),
            "0053B56E",
        )?;
        require_attached(
            attach_scene_child(world, root, Some(quad_node), false),
            "0053B582",
        )?;
    }
    // 0053B58E..0053B5C0 NiStream/BSStream construction, the path "Meshes\\Sky\\Atmosphere.nif" (BSStringT_Format), 006F9980.
    let load = (services.load_stream)(world, "Meshes\\Sky\\Atmosphere.nif")?;
    // 0053B5C9..0053B612: loaded, one top-level object, nonnull; a NiNode (vtable +0x08) gives its child 0 (00405790), otherwise it must be a NiTriShape
    // (00405650 IsKindOf 00B3FCD4).
    let mut mesh = None;
    if load.status == BsStreamLoadStatus::Loaded
        && load.root_count() == 1
        && let Some(loaded) = load.root(0)
    {
        if is_node(world, loaded) {
            mesh = node_child(world, loaded, 0);
        } else if world.get::<NiClass>(loaded) == Some(&NiClass::TriShape) {
            mesh = Some(loaded);
        }
    }
    let Some(mesh) = mesh else {
        // 0053B61D DebugMessage_Warning; the stream is destroyed and the function returns without the +0x0C fog store.
        services.warnings.push("Cannot load the atmosphere model.");
        release_load(world, &load);
        return Ok(());
    };
    // 0053B63A..0053B676: +0x08 = mesh, "Atmosphere Mesh", +0x18 |= 2, root AttachChild(+0x08, 0) (detaching it from the NIF root first).
    atmosphere.mesh_08 = Some(mesh);
    name_object(world, mesh, b"Atmosphere Mesh")?;
    or_av_flags(world, mesh, 2)?;
    require_attached(
        attach_scene_child(world, root, Some(mesh), false),
        "0053B676",
    )?;
    // 0053B67E..0053B694: the stream is destroyed (its roots released), then +0x0C = fog (NiPointer assign).
    atmosphere.fog_0c = fog;
    Ok(())
}

// IDA: 005442A0 Stars constructor; 005443E0 vtable +0x08 Init(parent): 00543D30, then SetName(+4, "Stars Root"). Completion: 100%.
fn initialize_stars(
    world: &mut World,
    stars: &mut SkyStars,
    parent: Option<Entity>,
) -> Result<(), StreamError> {
    let root = initialize_sky_object(world, parent)?;
    stars.root_04 = Some(root);
    name_object(world, root, b"Stars Root")
}

// IDA: the sun quad (00544EE0..00544FBF for fSunBaseSize, 00545038..00545115 for fSunGlareSize): (-s, s, 0), (-s, -s, 0), (s, s, 0), (s, -s, 0); each -s is
// s * -1.0 (dbl_A3D360) rounded to float. Completion: 100%.
#[allow(
    clippy::neg_multiply,
    reason = "00544EE0's fmul of dbl_A3D360 (-1.0), kept as the original's operation"
)]
fn sun_quad(size: f32) -> Vec<[f32; 3]> {
    let negative = size * -1.0;
    vec![
        [negative, size, 0.0],
        [negative, negative, 0.0],
        [size, size, 0.0],
        [size, negative, 0.0],
    ]
}

// IDA: 007174B0 for one sun quad: zero normals, colors (1, 0, 0, 1), UVs (0,0) (0,1) (1,0) (1,1), triangles (0,1,2) (2,1,3), one texture set. Completion: 100%.
fn construct_sun_geometry(world: &mut World, size: f32) -> Entity {
    let data = construct_tri_shape_data(
        world,
        4,
        sun_quad(size),
        vec![[0.0; 3]; 4],
        vec![[1.0, 0.0, 0.0, 1.0]; 4],
        vec![[0.0, 0.0], [0.0, 1.0], [1.0, 0.0], [1.0, 1.0]],
        1,
        0,
        2,
        vec![0, 1, 2, 2, 1, 3],
    );
    construct_tri_shape(world, Some(data))
}

// IDA: the +0xDC store of 00545450.. and 005455B4..: (+0xDC & 0xFFFA) | 2. Completion: 100%.
fn set_sun_billboard_mode(world: &mut World, billboard: Entity) {
    let mut node = world
        .get_mut::<BillboardNode>(billboard)
        .expect("just spawned");
    node.effective_mode_flags_dc = (node.effective_mode_flags_dc & 0xFFFA) | 2;
}

// IDA: 00544EA0 Sun vtable +0x08 Init(parent). Completion: 100% of the order (disasm-sun-init.txt).
fn initialize_sun(
    world: &mut World,
    sun: &mut SkySun,
    parent: Option<Entity>,
    services: &SkyInitServices<'_>,
    prepare: &mut PrepareGeometryContext<'_>,
) -> Result<(), StreamError> {
    let root = initialize_sky_object(world, parent)?;
    sun.root_04 = Some(root);
    name_object(world, root, b"Sun Root")?;
    // 005453BC..00545448 007174B0 -> +0x10 "Sun Geometry", +0x18 |= 2.
    let geometry = construct_sun_geometry(world, services.sun_base_size);
    sun.geometry_10 = Some(geometry);
    name_object(world, geometry, b"Sun Geometry")?;
    or_av_flags(world, geometry, 2)?;
    // 00545450..00545518 NiBillboardNode (NiNode(0), +0xE0 = 0.0, vtable, +0xDC = 9) -> +0x08, "Sun Billboard Node", +0x18 |= 2, +0xDC = (+0xDC & 0xFFFA) | 2,
    // then AttachChild(+0x10, 0).
    let billboard = spawn_billboard_node(world);
    sun.billboard_08 = Some(billboard);
    name_object(world, billboard, b"Sun Billboard Node")?;
    or_av_flags(world, billboard, 2)?;
    set_sun_billboard_mode(world, billboard);
    require_attached(
        attach_scene_child(world, billboard, Some(geometry), false),
        "00545518",
    )?;
    // 0054551A..005455B0 007174B0 -> +0x14 "Sun Glare Geometry", +0x18 |= 2.
    let glare_geometry = construct_sun_geometry(world, services.sun_glare_size);
    sun.glare_geometry_14 = Some(glare_geometry);
    name_object(world, glare_geometry, b"Sun Glare Geometry")?;
    or_av_flags(world, glare_geometry, 2)?;
    // 005455B4..00545690 the glare billboard -> +0x0C, "Sun Glare Billboard Node", +0x18 |= 2, +0xDC op, local scale +0x60 = fabs(+0x20) (float round trips),
    // then AttachChild(+0x14, 0).
    let glare_billboard = spawn_billboard_node(world);
    sun.glare_billboard_0c = Some(glare_billboard);
    name_object(world, glare_billboard, b"Sun Glare Billboard Node")?;
    or_av_flags(world, glare_billboard, 2)?;
    set_sun_billboard_mode(world, glare_billboard);
    world
        .get_mut::<SceneNodePayload>(glare_billboard)
        .expect("just spawned")
        .local_transform
        .scale = sun.float_20.abs();
    require_attached(
        attach_scene_child(world, glare_billboard, Some(glare_geometry), false),
        "00545690",
    )?;
    // 005456A3 root AttachChild(+0x08, 0).
    require_attached(
        attach_scene_child(world, root, Some(billboard), false),
        "005456A3",
    )?;
    // 005456A5..005457AA FormHeap(0x114) 00719760 -> +0x1C, "Sun Directional Light", dimmer 1.0, +0xB8 += 1, ambient white, +0xB8 += 1, diffuse white, +0xB8 += 1,
    // root AttachChild(+0x1C, 0).
    let light = spawn_directional_light(world);
    sun.light_1c = Some(light);
    name_object(world, light, b"Sun Directional Light")?;
    {
        let mut state = world
            .get_mut::<DirectionalLight>(light)
            .expect("just spawned");
        state.payload.dimmer = 1.0;
        state.unknown_b8 = state.unknown_b8.wrapping_add(1);
        state.payload.ambient = NI_COLOR_WHITE;
        state.unknown_b8 = state.unknown_b8.wrapping_add(1);
        state.payload.diffuse = NI_COLOR_WHITE;
        state.unknown_b8 = state.unknown_b8.wrapping_add(1);
    }
    require_attached(
        attach_scene_child(world, root, Some(light), false),
        "005457AA",
    )?;
    // 005457AC..005457E9 FormHeap(0x30) 00959BC0 -> +0x18 (raw), then +0x00 = 1, +0x08 = 1, +0x10 = 1, +0x11 = 1.
    sun.pick_18 = Some(SkySunPick {
        pick_type_00: 1,
        intersect_type_08: 1,
        front_only_10: 1,
        observe_app_cull_11: 1,
        ..SkySunPick::default()
    });
    // 005457FB 007B8940(+0x0C, 10, 0, 1); 00545809 007B8940(+0x08, 10, 0, 1).
    prepare_object_geometry(
        world,
        Some(glare_billboard),
        SKY_SHADER_SLOT_INDEX,
        0,
        1,
        prepare,
    )?;
    prepare_object_geometry(world, Some(billboard), SKY_SHADER_SLOT_INDEX, 0, 1, prepare)?;
    Ok(())
}

// IDA: 0053BDB0 Clouds vtable +0x08 Init(parent): 00543D30, "Cloud Root", the Clouds.nif load through 006F9980 ("Cannot load the clouds." unless it is one
// top-level NiNode), children 0 and 1 as the layers ("Missing expected geometry layer in Clouds.nif"; each data +0x2E &= 0x0FFF, +0x18 |= 2, root AttachChild(slot,
// 1)), then 00541790(root, two NiColor (1, 0, 0), 0). Completion: 100% of the order.
// TODO: [ ] NiStream's release of a successful load's root, as for the atmosphere.
fn initialize_clouds(
    world: &mut World,
    clouds: &mut SkyClouds,
    parent: Option<Entity>,
    services: &mut SkyInitServices<'_>,
) -> Result<(), StreamError> {
    let root = initialize_sky_object(world, parent)?;
    clouds.root_04 = Some(root);
    name_object(world, root, b"Cloud Root")?;
    let load = (services.load_stream)(world, "Meshes\\Sky\\Clouds.nif")?;
    // 0053BE4D..0053BE90: loaded, one top-level object, nonnull, IsKindOf NiNode (RTTI walk); otherwise "Cannot load the clouds.".
    let mut loaded = None;
    if load.status == BsStreamLoadStatus::Loaded && load.root_count() == 1 {
        loaded = load.root(0).filter(|&object| is_node(world, object));
    }
    let Some(loaded) = loaded else {
        services.warnings.push("Cannot load the clouds.");
        release_load(world, &load);
        return Ok(());
    };
    // 0053BEEF..0053BF05: a null root warns (unreachable after the test).
    // 0053BF14..0053BFF6: slots 0 and 1 (n = (n + 1) % 3 while n < 2).
    let mut layer = 0;
    while layer < 2 {
        let Some(child) = node_child(world, loaded, layer) else {
            // 0053BFFC "Missing expected geometry layer in Clouds.nif".
            clouds.layers_08[layer] = None;
            services
                .warnings
                .push("Missing expected geometry layer in Clouds.nif");
            return Ok(());
        };
        clouds.layers_08[layer] = Some(child);
        // 0053BF42 data +0x2E &= 0x0FFF; 0053BF50 +0x18 |= 2; root(+4) AttachChild(slot, 1).
        let data = child_geometry_data(world, child)
            .ok_or(StreamError("0053BF42: cloud layer without NiGeometryData"))?;
        with_geometry_data(world, data, |base| base.dirty_flags_2e &= 0x0FFF)
            .ok_or(StreamError("0053BF42: cloud layer without NiGeometryData"))?;
        or_av_flags(world, child, 2)?;
        require_attached(
            attach_scene_child(world, root, Some(child), true),
            "0053BF6B",
        )?;
        layer = (layer + 1) % 3;
    }
    // 0053BF7A..0053BFE6: two NiColor (1, 0, 0), 00541790(root, colors, 0).
    set_geometry_vertex_colors(world, root, &[[1.0, 0.0, 0.0], [1.0, 0.0, 0.0]], false)
}

// IDA: 007287C0 NiGeometryData_CreateColors(fill): without colors, a zero-initialized array of the vertex count (0047EA50); fill stores 00B25AD0 (0, 0, 0, 1)
// into each. Completion: 100%.
fn create_geometry_colors(data: &mut MeshGeometryData, fill: bool) {
    if data.colors_24.is_empty() {
        data.colors_24 = vec![[0.0; 4]; usize::from(data.vertex_count_08)];
    }
    if fill {
        data.colors_24.fill(NI_COLOR_A_BLACK);
    }
}

// IDA: 00541790 Sky_SetGeometryVertexColors(node, colors, add): for each child slot i below +0xB6, geometry (vtable +0x10) gets colors created white when absent
// (007287C0(0), then 00B25AE0), then each vertex's rgb set (or added) from colors[i] and +0x2E |= 4; a NiNode child recurses with the same colors. Completion:
// 100% of the order.
pub fn set_geometry_vertex_colors(
    world: &mut World,
    node: Entity,
    colors: &[[f32; 3]],
    add: bool,
) -> Result<(), StreamError> {
    // 00541798: the slots below +0xB6; colors[i] follows the slot index.
    let children = world
        .get::<NiNodeChildren>(node)
        .expect("a NiNode has child slots")
        .slots_ac
        .clone();
    for (index, child) in children.into_iter().enumerate() {
        let Some(child) = child else {
            continue;
        };
        // 005417E1 vtable +0x10 (geometry) is zero: vtable +0x08 recursion.
        let Some(data) = child_geometry_data(world, child) else {
            if is_node(world, child) {
                set_geometry_vertex_colors(world, child, colors, add)?;
            }
            continue;
        };
        let tint = *colors
            .get(index)
            .ok_or(StreamError("00541790: more geometry slots than colors"))?;
        with_geometry_data(world, data, |data| {
            let count = usize::from(data.vertex_count_08);
            if data.colors_24.is_empty() {
                // 005417FB 007287C0(0); a zero count skips to the +0x2E store.
                create_geometry_colors(data, false);
                if count != 0 {
                    data.colors_24.fill(NI_COLOR_A_WHITE);
                }
            }
            if count != 0 {
                for color in data.colors_24.iter_mut().take(count) {
                    if add {
                        color[0] += tint[0];
                        #[allow(
                            clippy::assign_op_pattern,
                            reason = "00541790's operand order on this one component: tint, then color"
                        )]
                        {
                            color[1] = tint[1] + color[1];
                        }
                        color[2] += tint[2];
                    } else {
                        color[0] = tint[0];
                        color[1] = tint[1];
                        color[2] = tint[2];
                    }
                }
            }
            // 005418A7 data +0x2E |= 4.
            data.dirty_flags_2e |= 4;
        })
        .ok_or(StreamError("00541790: geometry without its data"))?;
    }
    Ok(())
}

// IDA: 00540FD0 Sky_Constructor: roots null, colors zeroed, parts null, +0xDC = 4, +0xD0 = 10.0, climate and weather null, +0xD4 = 0.0, colors white, +0xB4 white,
// +0xC0 = 0.0, +0xC4 = 1.0, +0xD8 = 1.0, +0xF4 = 0.0, the 8-byte list, +0xE4 = 0.0, +0xEC = 0, +0xF0 = *(00B36658), +0xFC = 32, +0x100 = 0. Completion: 100% of the
// store values.
pub fn construct_sky(settings: &GameSettingCollection) -> Result<Sky, StreamError> {
    // 00541165 fld 00B36658: the fSecondsBetweenWindowUpdate GMST value (the 10.0 default before the Phase 12 GMST load).
    let window_update = settings
        .raw_at(SECONDS_BETWEEN_WINDOW_UPDATE)
        .ok_or(StreamError(
            "00541165: no 00B36658 fSecondsBetweenWindowUpdate",
        ))?;
    Ok(Sky {
        original_vtable: SKY_VTABLE,
        root_04: None,
        moons_root_08: None,
        first_climate_0c: 0,
        first_weather_10: 0,
        weather_14: 0,
        weather_18: 0,
        weather_override_1c: 0,
        atmosphere_20: None,
        stars_24: None,
        sun_28: None,
        clouds_2c: None,
        masser_30: None,
        secunda_34: None,
        precipitation_38: None,
        colors_3c: [NI_COLOR_WHITE; 10],
        color_b4: NI_COLOR_WHITE,
        float_c0: 0.0,
        float_c4: 1.0,
        unset_c8: [0; 2],
        float_d0: 10.0,
        float_d4: 0.0,
        float_d8: 1.0,
        dword_dc: 4,
        active_weather_sounds_e0: [0; 2],
        float_e4: 0.0,
        unset_e8: 0,
        dword_ec: 0,
        float_f0: f32::from_bits(window_update),
        float_f4: 0.0,
        unset_f8: 0,
        dword_fc: 32,
        byte_100: 0,
    })
}

// IDA: 00542EA0 Sky_GetSingleton: 00B365C4, else FormHeap(0x104) 00540FD0 stored there. An allocation failure stores 0 and returns null in the original; the
// host's allocator aborts instead. Completion: 100%.
pub fn get_sky_singleton<'g>(
    globals: &'g mut SkyGlobals,
    settings: &GameSettingCollection,
) -> Result<&'g mut Sky, StreamError> {
    if globals.singleton_b365c4.is_none() {
        globals.singleton_b365c4 = Some(Box::new(construct_sky(settings)?));
        globals.trace(0x00B3_65C4, 4);
    }
    Ok(globals
        .singleton_b365c4
        .as_deref_mut()
        .expect("just constructed"))
}

// IDA: 005411D0 Sky_Initialize(sky, root, fog). Each part constructor starts with 00543C10 SkyObject_Constructor (+0x04 root cleared). Completion: 100% of the order
// of the reached branches; the callees' host stops return errors.
// TODO: [ ] The old root's and old parts' releases: a second initialization is not reached at startup.
pub fn initialize_sky(
    world: &mut World,
    globals: &mut SkyGlobals,
    root: Option<Entity>,
    fog: Option<Entity>,
    services: &mut SkyInitServices<'_>,
    prepare: &mut PrepareGeometryContext<'_>,
) -> Result<(), StreamError> {
    let sky = globals
        .singleton_b365c4
        .as_deref_mut()
        .ok_or(StreamError("005411D0: no Sky"))?;
    // 005411DB..00541228: an old root is detached from its +0x1C parent (vtable +0x88) and released.
    if let Some(old) = sky.root_04.take()
        && let Some(parent) = world.get::<NiAvParent>(old).and_then(|link| link.0)
    {
        detach_scene_child(world, parent, old);
    }
    // 0054122E..00541288: the given root, or a "Sky Root" NiNode with +0x18 |= 2.
    let root = match root {
        Some(root) => root,
        None => {
            let root = spawn_scene_node(world, SceneNodePayload::default());
            name_object(world, root, b"Sky Root")?;
            or_av_flags(world, root, 2)?;
            root
        }
    };
    sky.root_04 = Some(root);
    // 00541290 root +0x54..+0x5C = g_niPoint3Zero.
    world
        .get_mut::<SceneNodePayload>(root)
        .ok_or(StreamError("00541290: the Sky root is not a NiNode"))?
        .local_transform
        .translation = [0.0; 3];
    // 0054129B..005412E5 Atmosphere (0x1C, 0053AE60) -> +0x20; vtable +0x10.
    let atmosphere = sky.atmosphere_20.insert(SkyAtmosphere::default());
    initialize_atmosphere(world, atmosphere, Some(root), fog, services)?;
    // 005412E7..00541326 Stars (0x10, 005442A0) -> +0x24; vtable +0x08.
    let stars = sky.stars_24.insert(SkyStars::default());
    initialize_stars(world, stars, Some(root))?;
    // 00541328..00541367 Sun (0x28, 00544B50) -> +0x28; vtable +0x08.
    let sun = sky.sun_28.insert(SkySun::default());
    initialize_sun(world, sun, Some(root), services, prepare)?;
    // 00541369..005413E1 FormHeap(0xDC) NiNode(0) -> +0x08 (NiPointer), root AttachChild(+0x08, 0), then SetName(+0x08, "Moons Root").
    let moons_root = spawn_scene_node(world, SceneNodePayload::default());
    sky.moons_root_08 = Some(moons_root);
    require_attached(
        attach_scene_child(world, root, Some(moons_root), false),
        "005413CE",
    )?;
    name_object(world, moons_root, b"Moons Root")?;
    // 005413E3..0054143E Precipitation (0x18, 0053D520) under bPrecipitation, then 0053D8F0; otherwise +0x38 = 0.
    if services.precipitation {
        // 0053D8F0: +0x0C = g_weatherNode (raw); null warns.
        sky.precipitation_38 = Some(SkyPrecipitation {
            weather_0c: services.weather,
            ..SkyPrecipitation::default()
        });
        if services.weather.is_none() {
            services
                .warnings
                .push("Can't find Weather Root Node.  Precipitation has no parent on scene graph.");
        }
    } else {
        sky.precipitation_38 = None;
    }
    // 00541440..0054147F Clouds (0x18, 0053BC80) -> +0x2C; vtable +0x08.
    let clouds = sky.clouds_2c.insert(SkyClouds::default());
    initialize_clouds(world, clouds, Some(root), services)?;
    // 00541481..00541523 NiAlphaProperty (0x1C; flags 0xEC, +0x1A = 0): flags = (flags & 0xFE01) | 0xEC, then |= 0x2001; 00405680 on the root.
    let mut alpha = AlphaProperty::default();
    alpha.flags_18 = (alpha.flags_18 & 0xFE01) | 0x00EC;
    alpha.flags_18 |= 0x2001;
    let alpha = world
        .spawn((NiClass::AlphaProperty, NiObjectNet::default(), alpha))
        .id();
    attach_scene_property(world, root, Some(alpha))?;
    // 00541525..005415AE NiVertexColorProperty (flags 8): (flags & 0xFFCF) | 0x20; 00405680 on the root.
    let vertex_color = VertexColorProperty {
        flags_18: (VertexColorProperty::default().flags_18 & 0xFFCF) | 0x0020,
    };
    let vertex_color = world
        .spawn((
            NiClass::VertexColorProperty,
            NiObjectNet::default(),
            vertex_color,
        ))
        .id();
    attach_scene_property(world, root, Some(vertex_color))?;
    // 005415B0..005415C9: g_rendererHDR -> 00B2C73C = 00B2C740 = 1.0.
    if services.renderer_hdr {
        globals.hdr_b2c73c = 1.0;
        globals.trace(0x00B2_C73C, 4);
        globals.hdr_b2c740 = 1.0;
        globals.trace(0x00B2_C740, 4);
    }
    Ok(())
}

// IDA: 007D3400 ShadowSceneLight_SetLight(light): +0x100 NiPointer assign (equal skips), +0xFC = IsKindOf(NiPointLight 00B3FD80); a point light with +0x104 set
// copies the light's world translate into +0x108..+0x110; otherwise +0xD4 = 1.0. Completion: 100% for the bound classes: only NiDirectionalLight is bound, so the
// IsKindOf is false for every host light.
// TODO: [ ] The point light arm (007D347C), with NiPointLight.
pub fn set_shadow_scene_light_light(light: &mut ShadowSceneLight, effect: Entity) {
    // 007D3404..007D3430.
    if light.light_100 != Some(effect) {
        light.light_100 = Some(effect);
    }
    // 007D3440..007D3470.
    light.byte_fc = 0;
    // 007D34A8 +0xD4 = 1.0.
    light.unknown_d0[1] = 1.0;
}

// IDA: 007C5850 ShadowSceneNode_SetSunLight(node, light): +0x118 is deleted (vtable +0, 1) and cleared, a new 0x220 ShadowSceneLight (007D6060) is stored raw at
// +0x118, then 007D3400(light) on it. Completion: 100% of the order.
pub fn set_shadow_scene_sun_light(node: &mut ShadowSceneNode, light: Entity) {
    // 007C5856..007C586E.
    node.light_118 = None;
    // 007C5870..007C5887 FormHeap(0x220) 007D6060 -> +0x118 (raw).
    let mut new_light = Box::new(construct_shadow_scene_light());
    // 007C588D 007D3400(light) on the new ShadowSceneLight.
    set_shadow_scene_light_light(&mut new_light, light);
    node.light_118 = Some(new_light);
}

/// IDA: 00543200 Sky_SetClimate(sky, 0, 1) as 0044CD49 calls it, with 00542EA0's singleton: a zero climate takes 0x15F through the TESClimate cast, and a null returns. +0xFC |= 0x3F00, +0x0C the climate; 00540380: +0xF4 = 0.0, +0x1C = +0x14 = +0x10 = 0, +0xFC = (+0xFC & ~8) | 1. Then the moons, by the climate's +0x55: bit 7 Masser, bit 6 Secunda, built when set and null, deleted when clear. With no Sky built, as in a phase test, only the lookup runs. Completion: 100% of the state; the graphics are a recorded host boundary.
/// TODO: [ ] 00540391 0053BBC0 and 005403C6 0053D6C0 on the clouds and precipitation; 00543280..0054338F the stars (00544780) and the sun (00542D30); with the sky renderer, M6.
pub fn sky_set_climate_default(world: &mut World) -> Result<(), StreamError> {
    let climate = world
        .get_resource::<super::data_handler::FormsById>()
        .and_then(|forms| forms.lookup(0x15F))
        .filter(|&form| {
            super::forms_init::casts_to(world, form, super::form_hierarchy::RttiClass::TESClimate)
        });
    let Some(climate) = climate else {
        return Ok(());
    };
    let id = super::forms_init::form_id(world, climate);
    let moon_bits = world
        .get::<super::forms_environment::TesClimate>(climate)
        .map_or(0, |climate| climate.timing_50[5]);
    let moons = world
        .get_resource::<GameSettingCollection>()
        .map(|settings| {
            (
                construct_moon(settings, "Masser"),
                construct_moon(settings, "Secunda"),
            )
        });
    let Some(mut globals) = world.get_resource_mut::<SkyGlobals>() else {
        return Ok(());
    };
    let Some(sky) = globals.singleton_b365c4.as_mut() else {
        return Ok(());
    };
    sky.dword_fc |= 0x3F00;
    sky.first_climate_0c = id;
    sky.float_f4 = 0.0;
    sky.dword_fc = (sky.dword_fc & !8) | 1;
    sky.weather_override_1c = 0;
    sky.weather_14 = 0;
    sky.first_weather_10 = 0;
    // 005433A0..005434B3. A climate's change to a set moon keeps the old one.
    let (masser, secunda) = moons.map_or((None, None), |(masser, secunda)| {
        (Some(masser), Some(secunda))
    });
    if moon_bits & 0x80 == 0 {
        sky.masser_30 = None;
    } else if sky.masser_30.is_none() {
        sky.masser_30 = masser;
    }
    if moon_bits & 0x40 == 0 {
        sky.secunda_34 = None;
    } else if sky.secunda_34.is_none() {
        sky.secunda_34 = secunda;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gamebryo::mesh_geometry::spawn_mesh_geometry;

    #[test]
    fn sky_constructor_values() {
        let settings = GameSettingCollection::default();
        assert_eq!(
            settings
                .find("fSecondsBetweenWindowUpdate")
                .unwrap()
                .definition
                .address,
            SECONDS_BETWEEN_WINDOW_UPDATE
        );
        let sky = construct_sky(&settings).unwrap();
        assert_eq!(sky.original_vtable, 0x00A5_6E14);
        assert_eq!(sky.float_f0.to_bits(), 0x4120_0000);
        assert_eq!(sky.colors_3c, [[1.0; 3]; 10]);
        assert_eq!(sky.dword_dc, 4);
        assert_eq!(sky.dword_fc, 32);
        assert!(sky.atmosphere_20.is_none());
    }

    #[test]
    fn the_singleton_is_constructed_once() {
        let settings = GameSettingCollection::default();
        let mut globals = SkyGlobals::default();
        get_sky_singleton(&mut globals, &settings).unwrap().dword_dc = 7;
        assert_eq!(
            get_sky_singleton(&mut globals, &settings).unwrap().dword_dc,
            7
        );
        assert_eq!(
            globals.store_trace,
            [SceneGlobalStore {
                address: 0x00B3_65C4,
                width: 4
            }]
        );
    }

    #[test]
    fn sun_quad_order() {
        assert_eq!(
            sun_quad(250.0),
            [
                [-250.0, 250.0, 0.0],
                [-250.0, -250.0, 0.0],
                [250.0, 250.0, 0.0],
                [250.0, -250.0, 0.0]
            ]
        );
    }

    fn strips_with_colors(
        world: &mut World,
        colors: Vec<[f32; 4]>,
        count: u16,
    ) -> (Entity, Entity) {
        let data = world
            .spawn(MeshTriStripsData {
                base: MeshGeometryData {
                    vertex_count_08: count,
                    colors_24: colors,
                    ..MeshGeometryData::default()
                },
                ..MeshTriStripsData::default()
            })
            .id();
        let strips = spawn_mesh_geometry(world, NiClass::TriStrips, MeshTriShapePayload::default());
        world.get_mut::<NiGeometry>(strips).unwrap().data_b4 = Some(data);
        (strips, data)
    }

    /// 00541790: colors are created white when absent, rgb is set or added, alpha stays, a node child recurses, and a null slot still uses up its index.
    #[test]
    fn vertex_colors_follow_the_slot_index() {
        let mut world = World::new();
        let root = spawn_scene_node(&mut world, SceneNodePayload::default());
        let inner = spawn_scene_node(&mut world, SceneNodePayload::default());
        let (absent, absent_data) = strips_with_colors(&mut world, Vec::new(), 2);
        let (present, present_data) =
            strips_with_colors(&mut world, vec![[0.5, 0.5, 0.5, 0.25]], 1);
        let (nested, nested_data) = strips_with_colors(&mut world, Vec::new(), 1);
        world.get_mut::<NiNodeChildren>(root).unwrap().slots_ac =
            vec![Some(absent), None, Some(present), Some(inner)];
        world.get_mut::<NiNodeChildren>(inner).unwrap().slots_ac = vec![Some(nested)];
        let colors = [
            [1.0, 0.0, 0.0],
            [9.0, 9.0, 9.0],
            [0.0, 1.0, 0.0],
            [9.0, 9.0, 9.0],
        ];
        set_geometry_vertex_colors(&mut world, root, &colors, false).unwrap();
        let base = |world: &World, data| world.get::<MeshTriStripsData>(data).unwrap().base.clone();
        assert_eq!(
            base(&world, absent_data).colors_24,
            [[1.0, 0.0, 0.0, 1.0]; 2]
        );
        assert_eq!(base(&world, absent_data).dirty_flags_2e, 4);
        assert_eq!(
            base(&world, present_data).colors_24,
            [[0.0, 1.0, 0.0, 0.25]]
        );
        // The recursion restarts the index: the nested slot 0 takes colors[0].
        assert_eq!(base(&world, nested_data).colors_24, [[1.0, 0.0, 0.0, 1.0]]);
        // Add.
        set_geometry_vertex_colors(&mut world, root, &colors, true).unwrap();
        assert_eq!(
            base(&world, present_data).colors_24,
            [[0.0, 2.0, 0.0, 0.25]]
        );
        // More geometry slots than colors stops.
        assert!(set_geometry_vertex_colors(&mut world, root, &colors[..1], false).is_err());
    }

    #[test]
    fn create_colors_fill_is_opaque_black() {
        let mut data = MeshGeometryData {
            vertex_count_08: 2,
            ..MeshGeometryData::default()
        };
        create_geometry_colors(&mut data, false);
        assert_eq!(data.colors_24, [[0.0; 4]; 2]);
        create_geometry_colors(&mut data, true);
        assert_eq!(data.colors_24, [[0.0, 0.0, 0.0, 1.0]; 2]);
    }

    /// 007C5850 replaces +0x118 with a new light, and 007D3400 sets +0xD4 and +0x100.
    #[test]
    fn shadow_sun_light() {
        let mut world = World::new();
        let node = crate::bs::scene_graphs::construct_shadow_scene_node(&mut world);
        let first = spawn_directional_light(&mut world);
        let second = spawn_directional_light(&mut world);
        let mut state = world.get_mut::<ShadowSceneNode>(node).unwrap();
        set_shadow_scene_sun_light(&mut state, first);
        let light = state.light_118.as_ref().unwrap();
        assert_eq!(light.light_100, Some(first));
        assert_eq!(light.byte_fc, 0);
        assert_eq!(light.unknown_d0, [0.0, 1.0, 0.0]);
        assert_eq!(light.float_124, 90.0);
        set_shadow_scene_sun_light(&mut state, second);
        assert_eq!(state.light_118.as_ref().unwrap().light_100, Some(second));
    }
}
