//! The scene graph core 00406950 Startup_InitializeSceneGraphs builds: the SceneGraph class (004114E0) with its NiCamera
//! and NiCullingProcess, the ShadowSceneNode with its ShadowSceneLights, BSClearZNode, and the property and child helpers.
//!
//! Every x87 sequence runs after CreateDevice (0040E865) under the 24-bit precision control word, so each arithmetic
//! result is a float and these ports use `f32` arithmetic in the original operand grouping. SceneGraph, ShadowSceneNode
//! and BSClearZNode share every NiNode update slot, so each is a `NiClass::Node` entity plus a component with its fields.
//! Evidence: evidence/phase05-window-2026-10-06/disasm-05C*.txt, P05-RUN-001.

use bevy_ecs::prelude::*;
use recomp_core::reader::StreamError;
use recomp_core::x87::float_copy;

use crate::gamebryo::av_object::{NiAvParent, NiAvProperties};
use crate::gamebryo::object_net::NiObjectNet;
use crate::gamebryo::scene_node::{
    NiAvWorld, NiEffectAffectedNodes, NiNodeChildren, NiNodeEffects, SceneNodePayload,
    spawn_scene_node,
};
use crate::gamebryo::scene_state::property_type;
use crate::gamebryo::scene_update::{
    SceneUpdateResult, SceneUpdateRuntime, SceneUpdateStatus, update_scene_object,
};
use crate::gamebryo::stream_objects::NiClass;
use crate::gamebryo::transform::NiTransform;
use crate::tes::settings::SettingCollection;

/// IDA: vtables stored by the 05-C constructors.
pub const NI_CAMERA_VTABLE: u32 = 0x00A7_E4C4;
pub const NI_CULLING_PROCESS_VTABLE: u32 = 0x00A7_E610;
pub const SCENE_GRAPH_VTABLE: u32 = 0x00A3_1BD4;
pub const SHADOW_SCENE_NODE_VTABLE: u32 = 0x00A9_04B4;
pub const SHADOW_SCENE_LIGHT_VTABLE: u32 = 0x00A9_12B4;
pub const BS_CLEAR_Z_NODE_VTABLE: u32 = 0x00A3_F894;
/// IDA: 00406B66 stores SceneNode_ClassifyReferenceFormType into 00B42EB4.
pub const REFERENCE_FORM_TYPE_CLASSIFIER_ADDRESS: u32 = 0x004E_0CF0;

/// IDA: dbl_A31C78 0.01745329238474369 is the float 0x3C8EFA35 widened, so the x87 product under 24-bit precision equals a
/// float multiply.
const DEGREES_TO_RADIANS: f32 = f32::from_bits(0x3C8E_FA35);

/// IDA: NiFrustum (0x1C bytes): +0x00 left, +0x04 right, +0x08 top, +0x0C bottom, +0x10 near, +0x14 far, +0x18 ortho byte
/// (0070DE60).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CameraFrustum {
    pub left: f32,
    pub right: f32,
    pub top: f32,
    pub bottom: f32,
    pub near_plane: f32,
    pub far_plane: f32,
    pub orthographic: bool,
}

/// IDA: NiRect<float> viewport port (NiCamera +0x110): left, right, top, bottom.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CameraPort {
    pub left: f32,
    pub right: f32,
    pub top: f32,
    pub bottom: f32,
}

/// IDA: NiPlane (0x10 bytes): normal then constant; 00716DB0 sets the normal from g_niPoint3Zero and the constant to 0.0.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CullingPlane {
    pub normal: [f32; 3],
    pub constant: f32,
}

/// IDA: 0070D590 NiCamera (0x124 bytes) over the NiAVObject base (00708450). The name, +0x1C parent, +0x20 bound, +0x64
/// world transform and +0x98 list (no writer reaches it) are `NiObjectNet`, `NiAvParent`, `NiAvWorld` and `NiAvProperties`.
/// The +0x60 / +0x64 / +0x68 update slots are the leaf 00707580 / 007075B0, +0x74 is 0070D430 (0070C120 then 0070CC90) and
/// +0x78 is 0070CC70.
/// TODO: [ ] Bind the culling and render consumers of the camera (later phases).
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct SceneCamera {
    pub original_vtable: u32,
    /// IDA: 00708450: 0 & ~0x10 | 0x0E.
    pub av_flags_18: u16,
    pub local_transform_30: NiTransform,
    /// A row-major 4x4 (0070CC90).
    pub world_to_clip_ac: [f32; 16],
    pub frustum_ec: CameraFrustum,
    pub min_near_plane_108: f32,
    pub max_far_near_ratio_10c: f32,
    /// Left, right, top, bottom.
    pub viewport_110: CameraPort,
    pub lod_adjust_120: f32,
}

// IDA: 0070D590 NiCamera constructor: NiAVObject base, NiFrustum_SetOrtho(0), viewport 0/0/0/0, frustum near 1.0 / far 2.0 /
// top 0.5 / bottom -0.5 / left -0.5 / right 0.5 / ortho 0, +0x108 0.1, +0x10C 10000.0, viewport top/right 1.0, +0x120 1.0,
// viewport bottom/left 0.0 (that store order), then 0070C120, 0070CC90 and the bound center = world translation.
// Completion: 100% of the values and order.
// TODO: [ ] FormHeap ownership.
pub fn construct_scene_camera(world: &mut World) -> Entity {
    let local_transform_30 = NiTransform::default();
    let mut camera = SceneCamera {
        original_vtable: NI_CAMERA_VTABLE,
        av_flags_18: 0x000E,
        local_transform_30,
        world_to_clip_ac: [0.0; 16],
        // 0070D5D6 NiFrustum_SetOrtho(+0xEC, 0), then 0070D5F7..0070D62C: +0x104, +0xFC, +0x100, +0xF4, +0xF8, +0xEC, +0xF0.
        frustum_ec: CameraFrustum {
            orthographic: false,
            near_plane: 1.0,
            far_plane: 2.0, // flt_A379B4
            top: 0.5,       // flt_A3D65C
            bottom: -0.5,   // flt_A45E4C
            left: -0.5,
            right: 0.5,
        },
        // 0070D632..0070D644: +0x108 = 0.1 (A2FAAC), +0x10C = 10000.0 (A5A04C).
        min_near_plane_108: 0.1,
        max_far_near_ratio_10c: 10000.0,
        // 0070D5DB..0070D5F1 zeros, then 0070D64A..0070D662: +0x118, +0x114, +0x120 = 1.0, then +0x11C, +0x110 = 0.
        viewport_110: CameraPort {
            left: 0.0,
            right: 1.0,
            top: 1.0,
            bottom: 0.0,
        },
        lod_adjust_120: 1.0,
    };
    // 0070D668 0070C120 with a null +0x1C parent copies local into world.
    let world_transform = local_transform_30;
    // 0070D66F 0070CC90.
    update_camera_world_to_clip(&mut camera, &world_transform);
    // 0070D674..0070D68C: bound center = world translation (radius stays 0).
    let translation = world_transform.translation;
    world
        .spawn((
            NiClass::Camera,
            camera,
            NiObjectNet::default(),
            NiAvParent::default(),
            NiAvProperties::default(),
            NiAvWorld {
                transform: world_transform,
                bound: [translation[0], translation[1], translation[2], 0.0],
                combined_bound: None,
            },
        ))
        .id()
}

// IDA: 0070CC90 NiCamera world-to-clip matrix (+0xAC..+0xE8): dot products of the world translation with the three rotation
// columns, the reciprocal frustum extents, then the perspective rows (0070CF00, ortho byte clear) or the orthographic rows
// (0070CDE8). Completion: 100% of the x87 operand grouping as float arithmetic under the 24-bit precision control word.
// TODO: [ ] Runtime-compare the orthographic rows (no startup camera sets the ortho byte; their divisions follow the
// decoded operands).
pub fn update_camera_world_to_clip(camera: &mut SceneCamera, world_transform: &NiTransform) {
    // World rotation entries in memory order (+0x64..+0x84): r[row*3+col]; translation +0x88..+0x90.
    let r = &world_transform.rotation;
    let [tx, ty, tz] = world_transform.translation;
    // 0070CCF6..0070CD28: var_40 = -((tx*m02 + ty*m12) + tz*m22).
    let right_dot = (tx * r[2] + ty * r[5]) + tz * r[8];
    let negative_right = -right_dot;
    // 0070CD2C..0070CD4C: var_3C = -((tx*m01 + ty*m11) + tz*m21).
    let up_dot = (tx * r[1] + ty * r[4]) + tz * r[7];
    let negative_up = -up_dot;
    // 0070CD50..0070CD7C: var_44 = -((tx*m00 + ty*m10) + tz*m20).
    let direction_dot = (tx * r[0] + ty * r[3]) + tz * r[6];
    let negative_direction = -direction_dot;
    // 0070CD80..0070CDDE: sums and reciprocal extents.
    let f = camera.frustum_ec;
    let width_sum = f.right + f.left;
    let height_sum = f.bottom + f.top;
    let inverse_width = 1.0 / (f.right - f.left);
    let inverse_height = 1.0 / (f.top - f.bottom);
    let inverse_depth = 1.0 / (f.far_plane - f.near_plane);
    let m = &mut camera.world_to_clip_ac;
    if !f.orthographic {
        // 0070CF00 perspective rows.
        let scale_x = inverse_width * 2.0;
        let offset_x = inverse_width * -width_sum;
        let scale_y = inverse_height * 2.0;
        let offset_y = inverse_height * -height_sum;
        let depth = f.far_plane * inverse_depth;
        m[0] = r[2] * scale_x + offset_x * r[0];
        m[1] = offset_x * r[3] + scale_x * r[5];
        m[2] = offset_x * r[6] + scale_x * r[8];
        m[3] = scale_x * negative_right + offset_x * negative_direction;
        m[4] = offset_y * r[0] + scale_y * r[1];
        m[5] = offset_y * r[3] + scale_y * r[4];
        m[6] = offset_y * r[6] + scale_y * r[7];
        m[7] = offset_y * negative_direction + scale_y * negative_up;
        m[8] = depth * r[0];
        m[9] = depth * r[3];
        m[10] = depth * r[6];
        let depth_offset = -f.near_plane * depth;
        m[11] = depth * negative_direction + depth_offset;
        m[12] = r[0];
        m[13] = r[3];
        m[14] = r[6];
        m[15] = negative_direction;
        return;
    }
    // 0070CDE8 orthographic rows (decoded operands: the x/y offsets divide by the reciprocals and the depth scale is
    // 1 / inverse_depth).
    let scale_x = inverse_width * 2.0;
    let scale_y = 2.0 * inverse_height;
    let depth = 1.0 / inverse_depth;
    m[0] = r[2] * scale_x;
    m[1] = r[5] * scale_x;
    m[2] = r[8] * scale_x;
    let offset_x = width_sum / inverse_width;
    m[3] = scale_x * negative_right + offset_x;
    m[4] = r[1] * scale_y;
    m[5] = r[4] * scale_y;
    m[6] = r[7] * scale_y;
    let offset_y = height_sum / inverse_height;
    m[7] = scale_y * negative_up + offset_y;
    m[8] = r[0] * depth;
    m[9] = r[3] * depth;
    m[10] = r[6] * depth;
    let depth_offset = -f.near_plane * depth;
    m[11] = depth * negative_direction + depth_offset;
    m[12] = 0.0;
    m[13] = 0.0;
    m[14] = 0.0;
    m[15] = 1.0;
}

// IDA: 0070C190 NiCamera::SetViewFrustum(frustum): near = frustum near (stored first), raised to far / +0x10C when that is
// greater, then to +0x108 when that is greater (both compares skip on equal or unordered), then left, right, top, bottom,
// far and the ortho byte. Completion: 100% of the order.
pub fn set_camera_view_frustum(camera: &mut SceneCamera, frustum: &CameraFrustum) {
    // 0070C194..0070C19F: fst +0xFC = near.
    camera.frustum_ec.near_plane = frustum.near_plane;
    // 0070C1A5..0070C1C1: far / +0x10C replaces near when greater.
    let far_limit = frustum.far_plane / camera.max_far_near_ratio_10c;
    if far_limit > camera.frustum_ec.near_plane {
        camera.frustum_ec.near_plane = far_limit;
    }
    // 0070C1CB..0070C1E6: +0x108 replaces near when greater.
    if camera.min_near_plane_108 > camera.frustum_ec.near_plane {
        camera.frustum_ec.near_plane = camera.min_near_plane_108;
    }
    // 0070C1EC..0070C21B.
    camera.frustum_ec.left = frustum.left;
    camera.frustum_ec.right = frustum.right;
    camera.frustum_ec.top = frustum.top;
    camera.frustum_ec.bottom = frustum.bottom;
    camera.frustum_ec.far_plane = frustum.far_plane;
    camera.frustum_ec.orthographic = frustum.orthographic;
}

/// IDA: 0070DF30 NiCullingProcess (0x90 bytes): +0x04 byte 0, frustum +0x10 all zero (0070DE70), six planes +0x2C (00716DB0),
/// +0x8C = 63, +0x08 = the argument, +0x0C = 0. Field names follow the layout; their meanings stay unresolved.
/// TODO: [ ] Bind the culling pass (render phases; a Direct3D-side boundary).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CullingProcess {
    pub original_vtable: u32,
    pub unknown_04: u8,
    pub argument_08: u32,
    pub unknown_0c: u32,
    pub frustum_10: CameraFrustum,
    pub planes_2c: [CullingPlane; 6],
    pub active_planes_8c: u32,
}

// IDA: 0070DF30's stores: 0070DF4C vtable, 0070DF56 +0x04 = 0, 0070DF61 frustum zeros, 0070DF72 planes, 0070DF83 +0x8C = 63,
// 0070DF8B +0x08 = argument, +0x0C = 0. Completion: 100% of the cold values.
#[must_use]
pub fn construct_culling_process(argument: u32) -> CullingProcess {
    CullingProcess {
        original_vtable: NI_CULLING_PROCESS_VTABLE,
        unknown_04: 0,
        argument_08: argument,
        unknown_0c: 0,
        frustum_10: CameraFrustum::default(),
        planes_2c: [CullingPlane::default(); 6],
        active_planes_8c: 63,
    }
}

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
#[derive(Clone, Debug, PartialEq)]
pub struct ShadowSceneLight {
    pub original_vtable: u32,
    pub matrix_10: [f32; 16],
    pub unknown_d0: [f32; 3],
    pub unknown_dc: f32,
    pub unknown_e0: f32,
    /// NiTPointerList<NiPointer<NiTriBasedGeom>>.
    pub geometry_list_count_e4: u32,
    pub byte_f4: u8,
    /// 1 at 007D614D, 0 at 007D627E.
    pub byte_f5: u8,
    pub pointer_f8: u32,
    /// IDA: +0xFC IsKindOf(NiPointLight) of the +0x100 light (007D3400).
    pub byte_fc: u8,
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
    pub unknown_144: u32,
    pub pointer_148: u32,
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
        unknown_d0: [0.0; 3],
        unknown_dc: 1.0,
        unknown_e0: 0.0,
        geometry_list_count_e4: 0,
        byte_f4: 0,
        byte_f5: 0,
        pointer_f8: 0,
        byte_fc: 0,
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
        unknown_144: 0,
        pointer_148: 0,
        pointer_14c: 0,
        planes_150: [CullingPlane::default(); 6],
        active_planes_1b0: 63,
        zero_1b4: [0; 0x60],
        byte_214: 0,
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
    pub light_list_count_e4: u32,
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
        light_list_count_e4: 0,
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

// IDA: 00708E40 NiDynamicEffect_AttachAffectedNode(effect, node): when the node is not yet in the effect's +0xBC list (+0xC0
// head), AddHead it (a raw pointer, no refcount) and make the mirrored call, which AddHeads the effect to the node's +0xBC
// effect list. Shared by the Phase 07 menu lights and the Phase 08 sun light. Completion: 100%.
pub fn attach_affected_node(world: &mut World, light: Entity, node: Entity) {
    // 00708E43..00708E60: scan the effect's +0xC0 head for the node.
    if world.get::<NiEffectAffectedNodes>(light).is_none() {
        world
            .entity_mut(light)
            .insert(NiEffectAffectedNodes::default());
    }
    let mut affected = world
        .get_mut::<NiEffectAffectedNodes>(light)
        .expect("just ensured");
    if affected.0.contains(&node) {
        return;
    }
    // 00708E68..00708EA1 AddHead (raw pointer, no refcount), then the mirror call 00708E40(node, effect) on the node's +0xBC
    // effect list.
    affected.0.insert(0, node);
    let mut effects = world
        .get_mut::<NiNodeEffects>(node)
        .expect("a NiNode has an effect list");
    if !effects.0.contains(&light) {
        effects.0.insert(0, light);
    }
}

// IDA: 006FF420 NiObjectNET::SetName: FormHeap_Free(+0x08), then a strcpy_s copy of the source, or null for a null source.
// Completion: 100% of the host string owner.
pub fn set_object_name(name: &mut Option<Vec<u8>>, source: Option<&[u8]>) {
    *name = source.map(<[u8]>::to_vec);
}

// IDA: 00405680 attaches a property when its vtable +0x4C type is below 10 (AddHead 00749800 on +0x98); see
// `property_type` for the reached types. A null property dereferences the vtable in the original; the host reports an
// error. Completion: 100% for the bound classes.
pub fn attach_scene_property(
    world: &mut World,
    node: Entity,
    property: Option<Entity>,
) -> Result<bool, StreamError> {
    // 004056A9..004056B5: vtable +0x4C type, `cmp eax, 0Ah; jge` skips.
    if property_type(world, property)? >= 10 {
        return Ok(false);
    }
    // 004056D8 NiTPointerList AddHead on +0x98 (the host list is head first).
    if world.get::<NiAvProperties>(node).is_none() {
        world.entity_mut(node).insert(NiAvProperties::default());
    }
    world
        .get_mut::<NiAvProperties>(node)
        .expect("just ensured")
        .0
        .insert(0, property);
    Ok(true)
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
) -> Result<f32, StreamError> {
    // 00410EE1..00410EF7: menu mode returns 20480.0 (flt_A31BAC).
    if menu_mode {
        return Ok(20480.0);
    }
    if settings.tes_present {
        return Err(StreamError("00410EE0: the TES branches are not ported"));
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
) -> Result<SceneUpdateResult, StreamError> {
    let mut state = world
        .get_mut::<SceneGraph>(graph)
        .expect("the graph is a SceneGraph");
    // 00411167..00411180: unless forced, an equal +0xEC returns (unordered compares continue).
    if !force && state.camera_fov_ec == fov_degrees {
        return Ok(UPDATED);
    }
    let Some(camera) = state.camera_dc else {
        return Err(StreamError("00411160: null SceneGraph camera"));
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
) -> Result<SceneUpdateResult, StreamError> {
    let Some(camera) = world
        .get::<SceneGraph>(graph)
        .expect("the graph is a SceneGraph")
        .camera_dc
    else {
        return Err(StreamError("00411100: null SceneGraph camera"));
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
        return Err(StreamError("00411100: child slot 0 is not a NiNode"));
    };
    require_attached(attach_scene_child(world, node, Some(camera), true))?;
    // 00411147..00411155 NiAVObject_Update(camera, 0.0, 1).
    update_scene_object(world, camera, 0.0, true, runtime)
}

// IDA: N/A AttachChild did not attach: a host stop.
fn require_attached(status: AttachChildStatus) -> Result<(), StreamError> {
    if status != AttachChildStatus::Attached {
        return Err(StreamError("AttachChild did not attach"));
    }
    Ok(())
}

// IDA: N/A a host update that reports an unsupported state stops the port.
fn require_updated(result: SceneUpdateResult) -> Result<(), StreamError> {
    if result.status != SceneUpdateStatus::Updated {
        return Err(StreamError("scene update reported an unsupported state"));
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
// TODO: [ ] 0040FEC0 -> DebugLog_Dispatch(2, ...) at 0041153A: `recomp_game` has no logger yet, so the log line is omitted.
pub fn construct_scene_graph(
    world: &mut World,
    name: &[u8],
    menu_mode: bool,
    existing_camera: Option<Entity>,
    settings: &SceneGraphSettings,
    runtime: &mut dyn SceneUpdateRuntime,
) -> Result<Entity, StreamError> {
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
    use crate::gamebryo::scene_derived::spawn_directional_light;
    use crate::gamebryo::scene_update::NoSceneControllers;

    fn camera_of(world: &World, graph: Entity) -> Entity {
        world.get::<SceneGraph>(graph).unwrap().camera_dc.unwrap()
    }

    fn name_of(world: &World, object: Entity) -> Option<&[u8]> {
        world.get::<NiObjectNet>(object).unwrap().name.as_deref()
    }

    /// Ported from `phase05_tests.cpp` `verify_scene_camera`: 0070D590's cold NiCamera and its 0070CC90 rows for the identity
    /// world (direction = column 0, up = column 1, right = column 2).
    #[test]
    fn scene_camera_cold_values_and_rows() {
        let mut world = World::new();
        let entity = construct_scene_camera(&mut world);
        let c = *world.get::<SceneCamera>(entity).unwrap();
        assert_eq!(c.original_vtable, 0x00A7_E4C4);
        assert_eq!(c.av_flags_18, 0x000E);
        assert_eq!(c.frustum_ec.left, -0.5);
        assert_eq!(c.frustum_ec.right, 0.5);
        assert_eq!(c.frustum_ec.top, 0.5);
        assert_eq!(c.frustum_ec.bottom, -0.5);
        assert_eq!(c.frustum_ec.near_plane, 1.0);
        assert_eq!(c.frustum_ec.far_plane, 2.0);
        assert!(!c.frustum_ec.orthographic);
        assert_eq!(c.min_near_plane_108, 0.1);
        assert_eq!(c.max_far_near_ratio_10c, 10000.0);
        assert_eq!(c.lod_adjust_120, 1.0);
        assert_eq!(c.viewport_110.left, 0.0);
        assert_eq!(c.viewport_110.right, 1.0);
        assert_eq!(c.viewport_110.top, 1.0);
        assert_eq!(c.viewport_110.bottom, 0.0);
        let expected: [f32; 16] = [
            0.0, 0.0, 2.0, 0.0, 0.0, 2.0, 0.0, 0.0, 2.0, 0.0, 0.0, -2.0, 1.0, 0.0, 0.0, -0.0,
        ];
        assert_eq!(c.world_to_clip_ac, expected);
        assert_eq!(c.world_to_clip_ac[15].to_bits(), 0x8000_0000);
        assert_eq!(world.get::<NiAvWorld>(entity).unwrap().bound, [0.0; 4]);

        // 0070C190: near = max(near, far / ratio, +0x108).
        let mut view = c;
        view.max_far_near_ratio_10c = 100.0;
        view.min_near_plane_108 = 0.5;
        let frustum = |near_plane, far_plane| CameraFrustum {
            left: -1.0,
            right: 1.0,
            top: 1.0,
            bottom: -1.0,
            near_plane,
            far_plane,
            orthographic: false,
        };
        set_camera_view_frustum(&mut view, &frustum(2.0, 1000.0));
        assert_eq!(view.frustum_ec.near_plane, 10.0);
        assert_eq!(view.frustum_ec.far_plane, 1000.0);
        set_camera_view_frustum(&mut view, &frustum(0.1, 10.0));
        assert_eq!(view.frustum_ec.near_plane, 0.5);
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
    fn affected_nodes_attach_both_ways_once() {
        let mut world = World::new();
        let light = spawn_directional_light(&mut world);
        let first = spawn_scene_node(&mut world, SceneNodePayload::default());
        let second = spawn_scene_node(&mut world, SceneNodePayload::default());
        attach_affected_node(&mut world, light, first);
        attach_affected_node(&mut world, light, second);
        attach_affected_node(&mut world, light, first);
        assert_eq!(
            world.get::<NiEffectAffectedNodes>(light).unwrap().0,
            [second, first],
            "AddHead, once"
        );
        assert_eq!(world.get::<NiNodeEffects>(first).unwrap().0, [light]);
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
    fn attach_scene_property_adds_at_the_head() {
        let mut world = World::new();
        let node = spawn_scene_node(&mut world, SceneNodePayload::default());
        let first = world.spawn(NiClass::AlphaProperty).id();
        let second = world.spawn(NiClass::ZBufferProperty).id();
        assert!(attach_scene_property(&mut world, node, Some(first)).unwrap());
        assert!(attach_scene_property(&mut world, node, Some(second)).unwrap());
        assert_eq!(
            world.get::<NiAvProperties>(node).unwrap().0,
            [Some(second), Some(first)]
        );
        assert!(attach_scene_property(&mut world, node, None).is_err());
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
