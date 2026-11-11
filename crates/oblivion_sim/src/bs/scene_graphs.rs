//! The scene graph core 00406950 Startup_InitializeSceneGraphs builds: the SceneGraph class (004114E0) with its NiCamera
//! and NiCullingProcess, the ShadowSceneNode with its ShadowSceneLights, BSClearZNode, and the property and child helpers.
//!
//! Every x87 sequence runs after CreateDevice (0040E865) under the 24-bit precision control word, so each arithmetic
//! result is a float and these ports use `f32` arithmetic in the original operand grouping. SceneGraph, ShadowSceneNode
//! and BSClearZNode share every NiNode update slot, so each is a `NiClass::Node` entity plus a component with its fields.
//! Evidence: evidence/phase05-window-2026-10-06/disasm-05C*.txt, P05-RUN-001.

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;
use oblivion_core::x87::float_copy;

use crate::gamebryo::av_object::NiAvParent;
use crate::gamebryo::camera::{
    CameraFrustum, CameraPort, CullingPlane, CullingProcess, SceneCamera,
    construct_culling_process, construct_scene_camera, set_camera_view_frustum,
};
use crate::gamebryo::object_net::{NiObjectNet, set_object_name};
use crate::gamebryo::scene_node::{NiAvWorld, NiNodeChildren, SceneNodePayload, spawn_scene_node};
use crate::gamebryo::scene_update::{
    SceneUpdateResult, SceneUpdateRuntime, SceneUpdateStatus, update_scene_object,
};
use crate::gamebryo::stream_objects::NiClass;
use crate::tes::settings::SettingCollection;

/// IDA: vtables stored by the 05-C constructors.
pub const SCENE_GRAPH_VTABLE: u32 = 0x00A3_1BD4;
pub const SHADOW_SCENE_NODE_VTABLE: u32 = 0x00A9_04B4;
pub const SHADOW_SCENE_LIGHT_VTABLE: u32 = 0x00A9_12B4;
pub const BS_CLEAR_Z_NODE_VTABLE: u32 = 0x00A3_F894;
/// IDA: 00406B66 stores SceneNode_ClassifyReferenceFormType into 00B42EB4.
pub const REFERENCE_FORM_TYPE_CLASSIFIER_ADDRESS: u32 = 0x004E_0CF0;

/// IDA: dbl_A31C78 0.01745329238474369 is the float 0x3C8EFA35 widened, so the x87 product under 24-bit precision equals a
/// float multiply.
const DEGREES_TO_RADIANS: f32 = f32::from_bits(0x3C8E_FA35);

/// IDA: 004114E0 SceneGraph (0xF0 bytes) over NiNode, on a `NiClass::Node` entity. Completion: 100% of the fields of the
/// reached constructor path.
/// TODO: [ ] Bind the culling and render consumers (later phases).
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct SceneGraph {
    pub original_vtable: u32,
    /// +0xDC NiPointer<NiCamera>.
    pub camera_dc: Option<Entity>,
    pub unknown_e0: u32,
    /// +0xE4 raw owner (0x90 allocation).
    pub culling_e4: Option<CullingProcess>,
    /// +0xE8 useExtraCameraMode byte.
    pub menu_mode_e8: bool,
    /// +0xEC: only 00411160 writes it; an existing-camera construction leaves the FormHeap bytes unset (host 0.0; that
    /// branch is not reached).
    pub camera_fov_ec: f32,
}

/// IDA: 007D6060 ShadowSceneLight constructor (0x220 bytes, NiRefObject base; the NiRefObject constructor increments
/// 00B3FD64, the global object count the host does not model). Fields by offset; meanings stay unresolved.
/// Completion: 100% of the final cold values.
/// TODO: [ ] Name the fields when the shadow passes are ported.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct ShadowSceneLight {
    pub original_vtable: u32,
    pub matrix_10: [f32; 16],
    pub last_contribution_d0: f32,
    /// The other two floats are initialized with +0xD0. Their consumers are unresolved.
    pub unknown_d4: [f32; 2],
    pub unknown_dc: f32,
    pub unknown_e0: f32,
    /// NiTPointerList<NiPointer<NiTriBasedGeom>>.
    pub geometry_list_e4: Vec<Entity>,
    pub byte_f4: u8,
    /// 1 at 007D614D, 0 at 007D627E.
    pub byte_f5: u8,
    pub pointer_f8: u32,
    /// IDA: +0xFC IsKindOf(NiPointLight) of the +0x100 light (007D3400).
    pub is_point_light_fc: u8,
    /// IDA: +0x100 NiPointer<NiLight> (007D3400, Phase 08 D5: the sun light).
    pub light_100: Option<Entity>,
    pub byte_104: u8,
    /// g_niPoint3Zero.
    pub point_108: [f32; 3],
    pub pointer_114: u32,
    pub word_118: u16,
    pub pointer_11c: u32,
    pub byte_120: u8,
    /// flt_A430CC.
    pub float_124: f32,
    pub float_128: f32,
    pub byte_12c: u8,
    pub pointer_130: u32,
    /// NiTPointerList<NiPointer<NiAVObject>>.
    pub object_list_count_134: u32,
    /// Index of the temporary fence node in geometry_list_e4 during a light update.
    pub geometry_refresh_fence_144: Option<usize>,
    /// The named NiTriShape reused as the fence on each refresh.
    pub geometry_fence_marker_148: Option<Entity>,
    pub pointer_14c: u32,
    pub planes_150: [CullingPlane; 6],
    pub active_planes_1b0: u32,
    /// memset 0x60.
    pub zero_1b4: [u8; 0x60],
    pub byte_214: u8,
}

// IDA: 007D6060: the final constructor values. Completion: 100% of the cold values.
#[must_use]
pub fn construct_shadow_scene_light() -> ShadowSceneLight {
    ShadowSceneLight {
        original_vtable: SHADOW_SCENE_LIGHT_VTABLE,
        matrix_10: [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ],
        last_contribution_d0: 0.0,
        unknown_d4: [0.0; 2],
        unknown_dc: 1.0,
        unknown_e0: 0.0,
        geometry_list_e4: Vec::new(),
        byte_f4: 0,
        byte_f5: 0,
        pointer_f8: 0,
        is_point_light_fc: 0,
        light_100: None,
        byte_104: 0,
        point_108: [0.0; 3],
        pointer_114: 0,
        word_118: 0,
        pointer_11c: 0,
        byte_120: 0,
        float_124: 90.0,
        float_128: 1.0,
        byte_12c: 0,
        pointer_130: 0,
        object_list_count_134: 0,
        geometry_refresh_fence_144: None,
        geometry_fence_marker_148: None,
        pointer_14c: 0,
        planes_150: [CullingPlane::default(); 6],
        active_planes_1b0: 63,
        zero_1b4: [0; 0x60],
        byte_214: 0,
    }
}

impl ShadowSceneLight {
    /// IDA: 007D31B0 ShadowSceneLight_CalculateLightAtPoint.
    /// Completion: the exclusion, point attenuation, maximum diffuse channel and +D0 stores.
    pub fn calculate_light_at_point(
        world: &mut World,
        shadow: Entity,
        point: [f32; 3],
        exclude: Option<Entity>,
    ) -> Result<f32, GameError> {
        use crate::gamebryo::scene_derived::{DirectionalLight, PointLight};
        let light = world
            .get::<ShadowSceneLight>(shadow)
            .expect("a shadow-scene light");
        if light.light_100 == exclude || light.byte_f4 == 1 {
            return Ok(0.0);
        }
        let source = light.light_100.ok_or(GameError::unclassified(
            0x007D31DA,
            "shadow light has no NiLight",
        ))?;
        let (diffuse, radius) = if let Some(light) = world.get::<PointLight>(source) {
            (light.diffuse_ec, light.specular_f8[0])
        } else if let Some(light) = world.get::<DirectionalLight>(source) {
            (light.payload.diffuse, light.payload.specular[0])
        } else {
            return Err(GameError::unclassified(
                0x007D31DA,
                "shadow light has no bound NiLight data",
            ));
        };
        let position = world
            .get::<NiAvWorld>(source)
            .ok_or(GameError::unclassified(
                0x007D31E0,
                "NiLight has no world transform",
            ))?
            .transform
            .translation;
        let mut light = world.get_mut::<ShadowSceneLight>(shadow).unwrap();
        light.last_contribution_d0 = 1.0;
        if light.is_point_light_fc != 0 {
            let [x, y, z] = std::array::from_fn(|i| point[i] - position[i]);
            let distance = ((x * x + y * y) + z * z).sqrt();
            let ratio = distance / radius;
            // An unordered comparison takes the upper-limit arm at 007D32B9.
            let attenuation = if ratio.is_nan() {
                1.0
            } else {
                ratio.clamp(0.0, 1.0)
            };
            light.last_contribution_d0 = 1.0 - attenuation * attenuation;
        }
        let [red, green, blue] = diffuse;
        let channel = if blue >= green { blue } else { green };
        let brightest = if channel >= red { channel } else { red };
        light.last_contribution_d0 *= brightest;
        Ok(light.last_contribution_d0)
    }
}

/// IDA: 007C7340 ShadowSceneNode (0x130 bytes) over NiNode, on a `NiClass::Node` entity: +0xDC null, two empty
/// NiTPointerList<NiPointer<ShadowSceneLight>> (+0xE4, +0xF4), +0x110 / +0x114 / +0x120 / +0x124 null, name "shadow scene
/// node", +0x118 = 0, +0xE0 = 0xFFFF, +0x128 = 0, +0x12C = 0, the two lights at +0x110 / +0x114, then +0x108 / +0x10C = 0.
/// Completion: 100% of the cold fields.
/// TODO: [ ] Name the fields with 05-D / 05-E.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct ShadowSceneNode {
    pub original_vtable: u32,
    pub pointer_dc: u32,
    pub word_e0: u16,
    pub light_list_e4: Vec<Entity>,
    pub light_list_count_f4: u32,
    pub unknown_108: u32,
    pub unknown_10c: u32,
    pub light_110: Option<Box<ShadowSceneLight>>,
    pub light_114: Option<Box<ShadowSceneLight>>,
    /// IDA: +0x118 raw ShadowSceneLight owner (0 from the constructor; Phase 08 D5: 007C5850 deletes the old one and stores
    /// a new 0x220 light).
    pub light_118: Option<Box<ShadowSceneLight>>,
    /// +0x11C: the constructor leaves it unset; 00406A8B stores 0.
    pub byte_11c: u8,
    pub pointer_120: u32,
    pub pointer_124: u32,
    pub unknown_128: u32,
    pub byte_12c: u8,
}

// IDA: 007C7340's stores: 007C73C8..007C73D4 SetName before the light allocations, then 007C7445..007C7528 two 0x220
// ShadowSceneLight owners (+0x110, +0x114). Completion: 100% of the cold fields.
pub fn construct_shadow_scene_node(world: &mut World) -> Entity {
    let node = spawn_scene_node(world, SceneNodePayload::default());
    set_object_name(
        &mut world
            .get_mut::<NiObjectNet>(node)
            .expect("just spawned")
            .name,
        Some(b"shadow scene node"),
    );
    world.entity_mut(node).insert(ShadowSceneNode {
        original_vtable: SHADOW_SCENE_NODE_VTABLE,
        pointer_dc: 0,
        word_e0: 0xFFFF,
        light_list_e4: Vec::new(),
        light_list_count_f4: 0,
        unknown_108: 0,
        unknown_10c: 0,
        light_110: Some(Box::new(construct_shadow_scene_light())),
        light_114: Some(Box::new(construct_shadow_scene_light())),
        light_118: None,
        byte_11c: 0,
        pointer_120: 0,
        pointer_124: 0,
        unknown_128: 0,
        byte_12c: 0,
    });
    node
}

/// IDA: 0049FFE0 BSClearZNode (0xE8 bytes) over NiNode, on a `NiClass::Node` entity: +0xE4 = 100.0 (A2FE7C), +0xDD = 0, +0xE0
/// = 283840.0 (A3F888), +0xDC = 0. Completion: 100% of the cold fields.
/// TODO: [ ] Name the bytes and floats with the render pass that reads them.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct ClearZNode {
    pub original_vtable: u32,
    pub byte_dc: u8,
    pub byte_dd: u8,
    pub float_e0: f32,
    pub float_e4: f32,
}

impl Default for ClearZNode {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            original_vtable: BS_CLEAR_Z_NODE_VTABLE,
            byte_dc: 0,
            byte_dd: 0,
            float_e0: 283_840.0,
            float_e4: 100.0,
        }
    }
}

// IDA: 0049FFE0's cold BSClearZNode. Completion: 100% of the cold fields.
pub fn spawn_clear_z_node(world: &mut World) -> Entity {
    let node = spawn_scene_node(world, SceneNodePayload::default());
    world.entity_mut(node).insert(ClearZNode::default());
    node
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttachChildStatus {
    Attached,
    /// 0070AF00 returns at once for a null child.
    NullChild,
    /// 007074B0 with a nonnull old parent whose entity has gone.
    UnsupportedReparent,
}

// IDA: 0070B120 NiNode::DetachChild (vtable +0x88): the first slot below the size holding the child; child +0x1C = 0, then
// 006D7F60 RemoveAt (null the slot, decrement the used count, shrink the size only for the last slot). Returns the removed
// child (null when absent). Completion: 100%.
pub fn detach_scene_child(world: &mut World, parent: Entity, child: Entity) -> Option<Entity> {
    // 0070B14F..0070B1BC: the first slot below the size holding the child.
    let index = world
        .get::<NiNodeChildren>(parent)
        .expect("a NiNode has child slots")
        .slots_ac
        .iter()
        .position(|&slot| slot == Some(child))?;
    // 0070B1E6 child +0x1C = 0, then 006D7F60 RemoveAt(index).
    world.entity_mut(child).insert(NiAvParent(None));
    let mut children = world
        .get_mut::<NiNodeChildren>(parent)
        .expect("checked above");
    let removed = children.slots_ac[index].take();
    if index == children.slots_ac.len() - 1 {
        children.slots_ac.pop();
    }
    removed
}

// IDA: 0070AF00 NiNode::AttachChild(child, firstAvailable), vtable +0x84 of NiNode / SceneGraph / ShadowSceneNode /
// BSClearZNode: 007074B0 SetParent, then 004B3680 (first null slot below the size, else append) or an append at the size
// (+0xB6) after 00523B10 growth; a current parent detaches the child first (0070B120). Completion: 100% of the slot rule;
// the host vector does not model the NiTArray capacity and grow-by fields.
pub fn attach_scene_child(
    world: &mut World,
    parent: Entity,
    child: Option<Entity>,
    first_available: bool,
) -> AttachChildStatus {
    // 0070AF2A: a null child returns at once.
    let Some(child) = child else {
        return AttachChildStatus::NullChild;
    };
    // 0070AF41 007074B0 SetParent: a current parent detaches the child first through its vtable +0x88 (0070B120).
    if let Some(old_parent) = world.get::<NiAvParent>(child).and_then(|link| link.0) {
        if world.get::<NiNodeChildren>(old_parent).is_none() {
            return AttachChildStatus::UnsupportedReparent;
        }
        detach_scene_child(world, old_parent, child);
    }
    world.entity_mut(child).insert(NiAvParent(Some(parent)));
    let mut children = world
        .get_mut::<NiNodeChildren>(parent)
        .expect("a NiNode has child slots");
    if first_available {
        // 0070AF67 004B3680: the first null slot below the size.
        if let Some(slot) = children.slots_ac.iter_mut().find(|slot| slot.is_none()) {
            *slot = Some(child);
            return AttachChildStatus::Attached;
        }
    }
    // 004B36C8 / 0070AF70: append at the size after growth.
    children.slots_ac.push(Some(child));
    AttachChildStatus::Attached
}

/// IDA: N/A a view of the values the scene graph functions read: 00B03134 fNearDistance, 00B0313C fDefaultFOV, 00B0311C /
/// 00B03124 / 00B0312C fNoLODFarDistanceMin / Max / Pct (03-E INI collection), g_rendererWidth / Height 00B06C4C / 00B06C50
/// (Phase 04), 00B3608F (read by 004E9F40) and the TES singleton 00B333A0. Completion: 100% of the sources.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SceneGraphSettings {
    pub near_distance: f32,
    pub default_fov: f32,
    pub no_lod_far_min: f32,
    pub no_lod_far_max: f32,
    pub no_lod_far_pct: f32,
    pub renderer_width: i32,
    pub renderer_height: i32,
    pub lod_far_mode: bool,
    pub tes_present: bool,
}

impl Default for SceneGraphSettings {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            near_distance: 10.0,
            default_fov: 75.0,
            no_lod_far_min: 1700.0,
            no_lod_far_max: 10240.0,
            no_lod_far_pct: 1.0,
            renderer_width: 640,
            renderer_height: 480,
            lod_far_mode: false,
            tes_present: false,
        }
    }
}

// IDA: N/A gathers `SceneGraphSettings` from the 03-E INI collection and the Phase 04 renderer globals. The renderer is a
// host boundary here, so its width and height arrive as plain values. Completion: 100% of the addresses; raw float bits are
// kept, through fld/fstp's copy.
// TODO: [ ] Read width and height from the renderer's service resource when it exists.
#[must_use]
pub fn scene_graph_settings_from(
    ini: &SettingCollection,
    renderer_width: u32,
    renderer_height: u32,
    lod_far_mode: bool,
    tes_present: bool,
) -> SceneGraphSettings {
    let raw = |address| ini.find_address(address).map_or(0, |setting| setting.raw);
    SceneGraphSettings {
        near_distance: float_copy(raw(0x00B0_3134)),
        default_fov: float_copy(raw(0x00B0_313C)),
        no_lod_far_min: float_copy(raw(0x00B0_311C)),
        no_lod_far_max: float_copy(raw(0x00B0_3124)),
        no_lod_far_pct: float_copy(raw(0x00B0_312C)),
        renderer_width: renderer_width as i32,
        renderer_height: renderer_height as i32,
        lod_far_mode,
        tes_present,
    }
}

// IDA: 00410EE0 SceneGraph_GetFarPlaneDistance: 20480.0 (A31BAC) when +0xE8 is set; otherwise, with TES absent, 283840.0
// (A31BA8) when 004E9F40 (00B3608F) is set, else Min + (Max - Min) * ((Pct - 0) / (1 - 0)). Completion: 100% for the startup
// state (no TES); an error when TES is present.
// TODO: [ ] Port the TES branches (00410F06 / 00410F64, 004C9A60).
pub fn scene_graph_far_plane_distance(
    menu_mode: bool,
    settings: &SceneGraphSettings,
) -> Result<f32, GameError> {
    // 00410EE1..00410EF7: menu mode returns 20480.0 (flt_A31BAC).
    if menu_mode {
        return Ok(20480.0);
    }
    if settings.tes_present {
        return Err(GameError::unclassified(
            0x00410EE0,
            "00410EE0: the TES branches are not ported",
        ));
    }
    // 00410F06 -> 00410F52: 004E9F40 set and TES null return 283840.0 (A31BA8).
    if settings.lod_far_mode {
        return Ok(283_840.0);
    }
    // 00410FB1: far = fNoLODFarDistanceMax; 00410FBA..00410FC8: (Pct - 0) / (1 - 0).
    let maximum = settings.no_lod_far_max;
    let numerator = settings.no_lod_far_pct - 0.0;
    let denominator = 1.0 - 0.0;
    let fraction = numerator / denominator;
    // 00410FCA..00410FDB: Min + (Max - Min) * fraction.
    let span = maximum - settings.no_lod_far_min;
    let scaled = span * fraction;
    Ok(settings.no_lod_far_min + scaled)
}

const UPDATED: SceneUpdateResult = SceneUpdateResult::UPDATED;

// IDA: 00411160 SceneGraph_SetCameraFOV(fov, forceUpdate): unless forced, return when +0xEC equals fov; +0xEC = fov, half
// angle = fov * (float)pi/180 * 0.5, a local NiFrustum (ortho 0) with near fNearDistance and far 00410EE0, camera +0x10C =
// far / near, aspect = H / W; menu mode: +-tan, +-tan*aspect; otherwise each scaled by 0.75 / aspect (dbl_A31C70); then
// 0070C190, NiAVObject_Update(camera, 0, 1) and camera +0x120 = fov / fDefaultFOV. Completion: 100% of the order. Host
// substitution: `f64::tan` for __CItan.
pub fn scene_graph_set_camera_fov(
    world: &mut World,
    graph: Entity,
    fov_degrees: f32,
    force: bool,
    settings: &SceneGraphSettings,
    runtime: &mut dyn SceneUpdateRuntime,
) -> Result<SceneUpdateResult, GameError> {
    let mut state = world
        .get_mut::<SceneGraph>(graph)
        .expect("the graph is a SceneGraph");
    // 00411167..00411180: unless forced, an equal +0xEC returns (unordered compares continue).
    if !force && state.camera_fov_ec == fov_degrees {
        return Ok(UPDATED);
    }
    let Some(camera) = state.camera_dc else {
        return Err(GameError::unclassified(
            0x00411160,
            "00411160: null SceneGraph camera",
        ));
    };
    // 00411186 fst +0xEC; 0041118E / 00411198 half angle.
    state.camera_fov_ec = fov_degrees;
    let menu_mode = state.menu_mode_e8;
    let half_angle = fov_degrees * DEGREES_TO_RADIANS * 0.5;
    // 004111A2 NiFrustum_SetOrtho(local, 0).
    // 004111A9..004111BE: far (var_8) and fNearDistance (var_C).
    let mut local = CameraFrustum {
        far_plane: scene_graph_far_plane_distance(menu_mode, settings)?,
        near_plane: settings.near_distance,
        ..CameraFrustum::default()
    };
    let mut owner = world
        .get_mut::<SceneCamera>(camera)
        .expect("+0xDC is a NiCamera");
    // 004111C2..004111D2: camera +0x10C = far / near.
    owner.max_far_near_ratio_10c = local.far_plane / local.near_plane;
    // 004111D8 / 004111E5: fild g_rendererHeight / fidiv g_rendererWidth.
    let aspect = settings.renderer_height as f32 / settings.renderer_width as f32;
    if menu_mode {
        // 004111ED..00411228, then 00411285.
        let tangent = f64::from(half_angle).tan() as f32; // __CItan
        let negative = -tangent;
        local.left = negative;
        local.right = tangent;
        local.bottom = negative * aspect;
        local.top = aspect * tangent;
    } else {
        // 0041122A..00411283: v = 0.75 / aspect (dbl_A31C70).
        let correction = 0.75 / aspect;
        let tangent = f64::from(half_angle).tan() as f32; // __CItan
        let negative = -tangent;
        local.left = negative * correction;
        local.right = tangent * correction;
        local.bottom = (negative * aspect) * correction;
        local.top = (aspect * tangent) * correction;
    }
    // 00411294 0070C190, then 004112A7 NiAVObject_Update(camera, 0.0, 1).
    set_camera_view_frustum(&mut owner, &local);
    let result = update_scene_object(world, camera, 0.0, true, runtime)?;
    if result.status != SceneUpdateStatus::Updated {
        return Ok(result);
    }
    // 004112AC..004112C5: camera +0x120 = fov / fDefaultFOV.
    world
        .get_mut::<SceneCamera>(camera)
        .expect("checked above")
        .lod_adjust_120 = fov_degrees / settings.default_fov;
    Ok(UPDATED)
}

// IDA: 00411100: when the camera's +0x1C parent is not child slot 0, child 0 AttachChild(camera, 1) (vtable +0x84) and
// NiAVObject_Update(camera, 0, 1). Child 0 is the "...Root CameraNode" of the constructor. Completion: 100% of the order.
pub fn scene_graph_attach_camera(
    world: &mut World,
    graph: Entity,
    runtime: &mut dyn SceneUpdateRuntime,
) -> Result<SceneUpdateResult, GameError> {
    let Some(camera) = world
        .get::<SceneGraph>(graph)
        .expect("the graph is a SceneGraph")
        .camera_dc
    else {
        return Err(GameError::unclassified(
            0x00411100,
            "00411100: null SceneGraph camera",
        ));
    };
    // 00411103..00411127: child slot 0 (null when the size is 0) against the camera +0x1C parent.
    let first = world
        .get::<NiNodeChildren>(graph)
        .expect("a NiNode has child slots")
        .slots_ac
        .first()
        .copied()
        .flatten();
    let parent = world.get::<NiAvParent>(camera).and_then(|link| link.0);
    if parent == first {
        return Ok(UPDATED);
    }
    // 0041113A..00411145: child 0 vtable +0x84 AttachChild(camera, 1).
    let Some(node) = first.filter(|&first| {
        world
            .get::<NiClass>(first)
            .is_some_and(|class| class.is_node())
    }) else {
        return Err(GameError::unclassified(
            0x00411100,
            "00411100: child slot 0 is not a NiNode",
        ));
    };
    require_attached(attach_scene_child(world, node, Some(camera), true))?;
    // 00411147..00411155 NiAVObject_Update(camera, 0.0, 1).
    update_scene_object(world, camera, 0.0, true, runtime)
}

// IDA: N/A AttachChild did not attach: a host stop.
fn require_attached(status: AttachChildStatus) -> Result<(), GameError> {
    if status != AttachChildStatus::Attached {
        return Err(GameError::unclassified(0, "AttachChild did not attach"));
    }
    Ok(())
}

// IDA: N/A a host update that reports an unsupported state stops the port.
fn require_updated(result: SceneUpdateResult) -> Result<(), GameError> {
    if result.status != SceneUpdateStatus::Updated {
        return Err(GameError::unclassified(
            0,
            "scene update reported an unsupported state",
        ));
    }
    Ok(())
}

// IDA: N/A sprintf(buffer, "%s<suffix>", base) through the static 00B34448 buffer; a host string.
fn prefixed_name(base: &[u8], suffix: &[u8]) -> Vec<u8> {
    [base, suffix].concat()
}

// IDA: 004114E0 SceneGraph_Constructor(name, useExtraCameraMode, camera): NiNode(0), vtable, +0xDC = 0, the 0040FEC0 log, +0xE8,
// the 0070DF30 culling process at +0xE4, +0xE0 = 0, names through sprintf into the static 00B34448 buffer ("%sRoot Node",
// "%sRoot CameraNode", "%sRoot Camera"), the camera node attached with firstAvailable 1, then either the given camera or a
// new NiCamera with viewport (0, 1, 1, 0), rotation {0,0,1, 1,0,0, 0,1,0}, +0x108 = 1.0, +0x10C = 00410EE0 / fNearDistance
// and 00411160(fDefaultFOV, 1); finally 00411100. Completion: 100% of the reached order; the static name buffer is a host
// string. An unsupported host update state is an error.
// TODO: [ ] 0040FEC0 -> DebugLog_Dispatch(2, ...) at 0041153A: `oblivion_sim` has no logger yet, so the log line is omitted.
pub fn construct_scene_graph(
    world: &mut World,
    name: &[u8],
    menu_mode: bool,
    existing_camera: Option<Entity>,
    settings: &SceneGraphSettings,
    runtime: &mut dyn SceneUpdateRuntime,
) -> Result<Entity, GameError> {
    // 00411510 NiNode(0), 0041151B vtable, 00411526 +0xDC = 0.
    let graph = spawn_scene_node(world, SceneNodePayload::default());
    // 00411558 +0xE8; 0041155E..00411589 culling process; 0041158F +0xE0 = 0.
    world.entity_mut(graph).insert(SceneGraph {
        original_vtable: SCENE_GRAPH_VTABLE,
        camera_dc: None,
        unknown_e0: 0,
        culling_e4: Some(construct_culling_process(0)),
        menu_mode_e8: menu_mode,
        camera_fov_ec: 0.0,
    });
    // 00411595..004115AE "%sRoot Node" through the 00B34448 buffer.
    set_object_name(
        &mut world
            .get_mut::<NiObjectNet>(graph)
            .expect("just spawned")
            .name,
        Some(&prefixed_name(name, b"Root Node")),
    );
    // 004115B3..00411601 the camera node, named, attached with firstAvailable 1.
    let camera_node = spawn_scene_node(world, SceneNodePayload::default());
    set_object_name(
        &mut world
            .get_mut::<NiObjectNet>(camera_node)
            .expect("just spawned")
            .name,
        Some(&prefixed_name(name, b"Root CameraNode")),
    );
    require_attached(attach_scene_child(world, graph, Some(camera_node), true))?;
    if let Some(camera) = existing_camera {
        // 0041160E NiRefObjectPtr_Assign(+0xDC, camera).
        world
            .get_mut::<SceneGraph>(graph)
            .expect("just inserted")
            .camera_dc = Some(camera);
    } else {
        // 0041161B..00411648 a new NiCamera assigned to +0xDC.
        let camera = construct_scene_camera(world);
        world
            .get_mut::<SceneGraph>(graph)
            .expect("just inserted")
            .camera_dc = Some(camera);
        // 0041164D..00411663 "%sRoot Camera".
        set_object_name(
            &mut world
                .get_mut::<NiObjectNet>(camera)
                .expect("just spawned")
                .name,
            Some(&prefixed_name(name, b"Root Camera")),
        );
        let far_plane = scene_graph_far_plane_distance(menu_mode, settings)?;
        let mut owner = world.get_mut::<SceneCamera>(camera).expect("just spawned");
        // 0041167A..004116B1 viewport +0x110..+0x11C = 0, 1, 1, 0.
        owner.viewport_110 = CameraPort {
            left: 0.0,
            right: 1.0,
            top: 1.0,
            bottom: 0.0,
        };
        // 004116C1..004116D6 rep movsd of the nine rotation entries.
        owner.local_transform_30.rotation = [0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        // 004116DC +0x108 = 1.0; 004116E4..004116F4 +0x10C = far / fNearDistance.
        owner.min_near_plane_108 = 1.0;
        owner.max_far_near_ratio_10c = far_plane / settings.near_distance;
        // 004116FA..00411703 00411160(fDefaultFOV, 1).
        require_updated(scene_graph_set_camera_fov(
            world,
            graph,
            settings.default_fov,
            true,
            settings,
            runtime,
        )?)?;
    }
    // 0041170A 00411100.
    require_updated(scene_graph_attach_camera(world, graph, runtime)?)?;
    Ok(graph)
}

/// IDA: one global store of 00406950, in order: destination VA and width.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SceneGlobalStore {
    pub address: u32,
    pub width: u8,
}

/// IDA: 00406950 globals (refcounted NiPointer slots and raw pointers). `store_trace` records each global store in order (address,
/// width). A raw pointer is an `Option<Entity>` like the refcounted ones, since references are not modelled.
/// Completion: 100% of the globals.
#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub struct SceneGraphGlobals {
    /// 00B333CC.
    pub world: Option<Entity>,
    /// 00B333D0 (00405B80).
    pub menu: Option<Entity>,
    /// 00B333D4 (00405C30).
    pub menu_3d: Option<Entity>,
    /// 00B333D8.
    pub sky: Option<Entity>,
    /// 00B333DC.
    pub weather: Option<Entity>,
    /// 00B333E0.
    pub wireframe: Option<Entity>,
    /// 00B333E4.
    pub fog: Option<Entity>,
    /// 00B333A4 raw.
    pub magic_projectile_root: Option<Entity>,
    /// 00B333A8 raw (BSClearZNode).
    pub lod_root: Option<Entity>,
    /// 00B333AC raw.
    pub object_lod_root: Option<Entity>,
    /// 00B42D64 raw.
    pub lod_trees: Option<Entity>,
    /// Host view of the local (ebp).
    pub shadow_scene: Option<Entity>,
    /// 00B43104 NiPointer = g_dx9Renderer.
    pub renderer_reference: bool,
    /// 00B42F68 (byte copy of A30048).
    pub shader_directory: String,
    /// 00B42F54 (007B4270).
    pub shadow_slots: [Option<Entity>; 5],
    /// 00B42EA7.
    pub byte_b42ea7: u8,
    /// 00B42E85.
    pub byte_b42e85: u8,
    /// 00B2C678.
    pub shadow_flags: u32,
    /// 00B44EE4.
    pub float_b44ee4: f32,
    /// 00B42EB4.
    pub reference_classifier: u32,
    /// 00B33396.
    pub static_menu_background: u8,
    /// 00B333EC (05-E-3).
    pub fallback_quad: Option<Entity>,
    pub store_trace: Vec<SceneGlobalStore>,
}

/// IDA: 00B39B84 (BSFaceGenNiNode camera static: 0055CBF0 / 0055CC60 read its world translate for a distance test) and 00B39E00
/// (BSTreeNode camera static: 00563EB0 / 00563F20 cull overrides through 0047F7B0): non-owning aliases of the World SceneGraph's
/// camera, published by WinMain 0040EAEF..0040EB06. Completion: 100%.
/// TODO: [ ] Scene and actor phases (carried): the four readers above.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CameraStaticAliases {
    /// 00B39B84.
    pub face_gen_node_camera: Option<Entity>,
    /// 00B39E00.
    pub tree_node_camera: Option<Entity>,
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::gamebryo::scene_update::NoSceneControllers;

    #[test]
    fn dreth_light_sample_matches_the_first_dungeon_torch_query() {
        use crate::gamebryo::scene_derived::PointLight;
        use crate::tes::light_3d::spawn_point_light;
        // living-light/living.tsv, 0001FC43, first High pass in the original.
        let probe = [0xC37A0000, 0xC2894EB1, 0xC209E364].map(f32::from_bits);
        let inputs = [
            (
                [0xC3CD8614, 0xC18A837C, 0x428E26C6],
                [0x3EFAFAFB, 0x3ED8D8D9, 0x3E9A9A9B],
                250.0,
            ),
            (
                [0xC1F1A1F9, 0xC281D679, 0x428E26C6],
                [0x3EAAAAAB, 0x3E5CDCDD, 0x3DB8B8B9],
                250.0,
            ),
            (
                [0x43208749, 0x434038A7, 0x43474A0E],
                [0x3F51D1D2, 0x3F19999A, 0x3EB4B4B5],
                500.0,
            ),
        ];
        let mut world = World::new();
        let mut sum = 0.0;
        for (position, color, radius) in inputs {
            let source = spawn_point_light(&mut world);
            world
                .get_mut::<NiAvWorld>(source)
                .unwrap()
                .transform
                .translation = position.map(f32::from_bits);
            let mut light = world.get_mut::<PointLight>(source).unwrap();
            light.diffuse_ec = color.map(f32::from_bits);
            light.specular_f8[0] = radius;
            let mut shadow = construct_shadow_scene_light();
            shadow.light_100 = Some(source);
            shadow.is_point_light_fc = 1;
            let shadow = world.spawn(shadow).id();
            let value = ShadowSceneLight::calculate_light_at_point(&mut world, shadow, probe, None)
                .unwrap();
            sum += value;
            assert_eq!(
                ShadowSceneLight::calculate_light_at_point(&mut world, shadow, probe, Some(source))
                    .unwrap(),
                0.0
            );
            assert_eq!(
                world
                    .get::<ShadowSceneLight>(shadow)
                    .unwrap()
                    .last_contribution_d0
                    .to_bits(),
                value.to_bits()
            );
        }
        assert_eq!((sum * 100.0).to_bits(), 0x419BEEB8);
    }

    fn camera_of(world: &World, graph: Entity) -> Entity {
        world.get::<SceneGraph>(graph).unwrap().camera_dc.unwrap()
    }

    fn name_of(world: &World, object: Entity) -> Option<&[u8]> {
        world.get::<NiObjectNet>(object).unwrap().name.as_deref()
    }

    fn fixture_settings() -> SceneGraphSettings {
        SceneGraphSettings {
            renderer_width: 1920,
            renderer_height: 1080,
            ..SceneGraphSettings::default()
        }
    }

    /// Ported from `phase05_tests.cpp` `verify_scene_graph_constructor`: 004114E0 / 00411160 / 00410EE0 / 00411100 for "World"
    /// and "Menu".
    #[test]
    fn scene_graph_constructor() {
        let mut world = World::new();
        let mut runtime = NoSceneControllers;
        let settings = fixture_settings();
        let graph =
            construct_scene_graph(&mut world, b"World", false, None, &settings, &mut runtime)
                .unwrap();
        let state = *world.get::<SceneGraph>(graph).unwrap();
        assert_eq!(state.original_vtable, 0x00A3_1BD4);
        assert_eq!(name_of(&world, graph), Some(&b"WorldRoot Node"[..]));
        assert_eq!(
            world.get::<NiNodeChildren>(graph).unwrap().slots_ac.len(),
            1
        );
        assert!(!state.menu_mode_e8);
        assert_eq!(state.unknown_e0, 0);
        let culling = state.culling_e4.unwrap();
        assert_eq!(culling.original_vtable, 0x00A7_E610);
        assert_eq!(culling.active_planes_8c, 63);
        assert_eq!(culling.argument_08, 0);

        let camera_node = world.get::<NiNodeChildren>(graph).unwrap().slots_ac[0].unwrap();
        assert_eq!(
            name_of(&world, camera_node),
            Some(&b"WorldRoot CameraNode"[..])
        );
        assert_eq!(world.get::<NiAvParent>(camera_node).unwrap().0, Some(graph));
        let camera_entity = camera_of(&world, graph);
        assert_eq!(
            world.get::<NiNodeChildren>(camera_node).unwrap().slots_ac,
            [Some(camera_entity)],
            "00411100's attachment"
        );
        assert_eq!(
            name_of(&world, camera_entity),
            Some(&b"WorldRoot Camera"[..])
        );
        assert_eq!(
            world.get::<NiAvParent>(camera_entity).unwrap().0,
            Some(camera_node)
        );

        // 00410EE0 with no TES: 1700 + (10240 - 1700) * 1 = 10240; ratio 1024.
        let camera = *world.get::<SceneCamera>(camera_entity).unwrap();
        assert_eq!(
            scene_graph_far_plane_distance(false, &settings),
            Ok(10240.0)
        );
        assert_eq!(camera.max_far_near_ratio_10c, 1024.0);
        assert_eq!(camera.min_near_plane_108, 1.0);
        assert_eq!(state.camera_fov_ec, 75.0);
        assert_eq!(camera.lod_adjust_120, 1.0);

        let half = 75.0f32 * f32::from_bits(0x3C8E_FA35) * 0.5;
        let tangent = f64::from(half).tan() as f32;
        let aspect = 1080.0f32 / 1920.0;
        let correction = 0.75 / aspect;
        assert_eq!(camera.frustum_ec.left, -tangent * correction);
        assert_eq!(camera.frustum_ec.right, tangent * correction);
        assert_eq!(camera.frustum_ec.bottom, (-tangent * aspect) * correction);
        assert_eq!(camera.frustum_ec.top, (aspect * tangent) * correction);
        assert_eq!(camera.frustum_ec.near_plane, 10.0);
        assert_eq!(camera.frustum_ec.far_plane, 10240.0);
        assert!(
            (camera.frustum_ec.right - 1.023_102_6).abs() < 1.0e-6,
            "the right extent is tan(37.5) * 4/3"
        );
        let rotation = [0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        assert_eq!(camera.local_transform_30.rotation, rotation);
        assert_eq!(
            world
                .get::<NiAvWorld>(camera_entity)
                .unwrap()
                .transform
                .rotation,
            rotation
        );
        // Row 3 of the rotated camera at the origin is column 0, (0, 1, 0).
        assert_eq!(camera.world_to_clip_ac[12], 0.0);
        assert_eq!(camera.world_to_clip_ac[13], 1.0);
        assert_eq!(camera.world_to_clip_ac[14], 0.0);

        let menu = construct_scene_graph(&mut world, b"Menu", true, None, &settings, &mut runtime)
            .unwrap();
        assert!(world.get::<SceneGraph>(menu).unwrap().menu_mode_e8);
        assert_eq!(scene_graph_far_plane_distance(true, &settings), Ok(20480.0));
        let menu_camera = *world.get::<SceneCamera>(camera_of(&world, menu)).unwrap();
        assert_eq!(menu_camera.max_far_near_ratio_10c, 2048.0);
        assert_eq!(menu_camera.frustum_ec.left, -tangent);
        assert_eq!(menu_camera.frustum_ec.right, tangent);
        assert_eq!(menu_camera.frustum_ec.bottom, -tangent * aspect);
        assert_eq!(menu_camera.frustum_ec.top, aspect * tangent);

        let lod = SceneGraphSettings {
            lod_far_mode: true,
            ..settings
        };
        assert_eq!(scene_graph_far_plane_distance(false, &lod), Ok(283_840.0));
        let tes = SceneGraphSettings {
            tes_present: true,
            ..settings
        };
        assert!(scene_graph_far_plane_distance(false, &tes).is_err());
        assert_eq!(
            scene_graph_far_plane_distance(true, &tes),
            Ok(20480.0),
            "menu mode returns before the TES branches"
        );
    }

    /// 00411160's early return, and an existing camera, which 004114E0 attaches without touching its frustum.
    #[test]
    fn fov_early_return_and_existing_camera() {
        let mut world = World::new();
        let mut runtime = NoSceneControllers;
        let settings = fixture_settings();
        let graph =
            construct_scene_graph(&mut world, b"World", false, None, &settings, &mut runtime)
                .unwrap();
        let camera = camera_of(&world, graph);
        let frustum = world.get::<SceneCamera>(camera).unwrap().frustum_ec;
        world.get_mut::<SceneCamera>(camera).unwrap().lod_adjust_120 = 9.0;
        let result =
            scene_graph_set_camera_fov(&mut world, graph, 75.0, false, &settings, &mut runtime)
                .unwrap();
        assert_eq!(result.status, SceneUpdateStatus::Updated);
        assert_eq!(
            world.get::<SceneCamera>(camera).unwrap().lod_adjust_120,
            9.0,
            "an equal +0xEC returns before any store"
        );
        scene_graph_set_camera_fov(&mut world, graph, 75.0, true, &settings, &mut runtime).unwrap();
        assert_eq!(
            world.get::<SceneCamera>(camera).unwrap().lod_adjust_120,
            1.0
        );
        scene_graph_set_camera_fov(&mut world, graph, 37.5, false, &settings, &mut runtime)
            .unwrap();
        let changed = world.get::<SceneCamera>(camera).unwrap();
        assert_eq!(changed.lod_adjust_120, 0.5);
        assert_ne!(changed.frustum_ec, frustum);

        let shared = construct_scene_camera(&mut world);
        let cold = *world.get::<SceneCamera>(shared).unwrap();
        let other = construct_scene_graph(
            &mut world,
            b"Other",
            false,
            Some(shared),
            &settings,
            &mut runtime,
        )
        .unwrap();
        assert_eq!(camera_of(&world, other), shared);
        assert_eq!(*world.get::<SceneCamera>(shared).unwrap(), cold);
        let node = world.get::<NiNodeChildren>(other).unwrap().slots_ac[0];
        assert_eq!(world.get::<NiAvParent>(shared).unwrap().0, node);

        // The camera's parent is now slot 0, so a repeat takes 00411103's early return.
        let before = world
            .get::<NiNodeChildren>(node.unwrap())
            .unwrap()
            .slots_ac
            .clone();
        scene_graph_attach_camera(&mut world, other, &mut runtime).unwrap();
        assert_eq!(
            world.get::<NiNodeChildren>(node.unwrap()).unwrap().slots_ac,
            before
        );

        // The world-to-clip rows follow a parent's translation through the camera's +0x74.
        world
            .get_mut::<SceneNodePayload>(node.unwrap())
            .unwrap()
            .local_transform
            .translation = [0.0, 0.0, 100.0];
        update_scene_object(&mut world, other, 0.0, true, &mut runtime).unwrap();
        assert_eq!(
            world.get::<NiAvWorld>(shared).unwrap().bound,
            [0.0, 0.0, 100.0, 0.0],
            "0070CC70 stores the center and leaves the radius"
        );
    }

    #[test]
    fn attach_and_detach_follow_the_slot_rules() {
        let mut world = World::new();
        let spawn = |world: &mut World| spawn_scene_node(world, SceneNodePayload::default());
        let parent = spawn(&mut world);
        let (a, b, c) = (spawn(&mut world), spawn(&mut world), spawn(&mut world));
        assert_eq!(
            attach_scene_child(&mut world, parent, None, true),
            AttachChildStatus::NullChild
        );
        for child in [a, b, c] {
            assert_eq!(
                attach_scene_child(&mut world, parent, Some(child), true),
                AttachChildStatus::Attached
            );
        }
        let slots = |world: &World| {
            world
                .get::<NiNodeChildren>(parent)
                .unwrap()
                .slots_ac
                .clone()
        };
        assert_eq!(slots(&world), [Some(a), Some(b), Some(c)]);

        // A middle slot nulls; only the last slot shrinks the size.
        assert_eq!(detach_scene_child(&mut world, parent, b), Some(b));
        assert_eq!(slots(&world), [Some(a), None, Some(c)]);
        assert_eq!(world.get::<NiAvParent>(b).unwrap().0, None);
        assert_eq!(detach_scene_child(&mut world, parent, b), None, "absent");

        // firstAvailable takes the null slot; without it the child appends.
        attach_scene_child(&mut world, parent, Some(b), false);
        assert_eq!(slots(&world), [Some(a), None, Some(c), Some(b)]);
        assert_eq!(detach_scene_child(&mut world, parent, b), Some(b));
        assert_eq!(slots(&world), [Some(a), None, Some(c)]);
        attach_scene_child(&mut world, parent, Some(b), true);
        assert_eq!(slots(&world), [Some(a), Some(b), Some(c)]);

        // A current parent detaches first.
        let other = spawn(&mut world);
        attach_scene_child(&mut world, other, Some(c), true);
        assert_eq!(slots(&world), [Some(a), Some(b)]);
        assert_eq!(world.get::<NiAvParent>(c).unwrap().0, Some(other));

        let gone = spawn(&mut world);
        let held = spawn(&mut world);
        world.entity_mut(held).insert(NiAvParent(Some(gone)));
        world.despawn(gone);
        assert_eq!(
            attach_scene_child(&mut world, parent, Some(held), true),
            AttachChildStatus::UnsupportedReparent
        );
    }

    #[test]
    fn cold_shadow_and_clear_z_nodes() {
        let mut world = World::new();
        let node = construct_shadow_scene_node(&mut world);
        let state = world.get::<ShadowSceneNode>(node).unwrap();
        assert_eq!(state.original_vtable, 0x00A9_04B4);
        assert_eq!(state.word_e0, 0xFFFF);
        assert_eq!(state.byte_11c, 0);
        let light = state.light_110.as_deref().unwrap();
        assert_eq!(light.original_vtable, 0x00A9_12B4);
        assert_eq!(light.float_124, 90.0);
        assert_eq!(light.active_planes_1b0, 63);
        assert_eq!(light.matrix_10[5], 1.0);
        assert!(state.light_114.is_some() && state.light_118.is_none());
        assert_eq!(world.get::<NiClass>(node), Some(&NiClass::Node));
        assert_eq!(name_of(&world, node), Some(&b"shadow scene node"[..]));

        let clear = spawn_clear_z_node(&mut world);
        let state = world.get::<ClearZNode>(clear).unwrap();
        assert_eq!(state.original_vtable, 0x00A3_F894);
        assert_eq!(state.float_e0, 283_840.0);
        assert_eq!(state.float_e4, 100.0);
    }

    #[test]
    fn set_name_replaces_or_clears() {
        let mut name = Some(b"old".to_vec());
        set_object_name(&mut name, Some(b"new"));
        assert_eq!(name.as_deref(), Some(&b"new"[..]));
        set_object_name(&mut name, None);
        assert_eq!(name, None);
    }

    #[test]
    fn settings_read_raw_bits() {
        let mut ini = SettingCollection::default();
        for (address, name, bits) in [
            (0x00B0_3134, "fNearDistance:Display", 0x4120_0000u32),
            (0x00B0_313C, "fDefaultFOV:Display", 0x7F80_0001),
        ] {
            assert!(ini.add(crate::tes::settings::SettingInfo::new(
                address, name, bits, None
            )));
        }
        let settings = scene_graph_settings_from(&ini, 1920, 1080, true, false);
        assert_eq!(settings.near_distance, 10.0);
        assert_eq!(
            settings.default_fov.to_bits(),
            0x7FC0_0001,
            "fld / fstp quiet a signaling NaN"
        );
        assert_eq!(settings.no_lod_far_min, 0.0, "an absent setting reads 0");
        assert_eq!(settings.renderer_width, 1920);
        assert!(settings.lod_far_mode && !settings.tes_present);
    }
}
