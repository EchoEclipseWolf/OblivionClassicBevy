//! NiCamera with its NiFrustum, viewport and NiFrustumPlanes, and NiCullingProcess: the camera and culling objects
//! 004114E0 SceneGraph_Constructor builds.

use bevy_ecs::prelude::*;

use super::av_object::{NiAvParent, NiAvProperties};
use super::object_net::NiObjectNet;
use super::scene_node::NiAvWorld;
use super::stream_objects::NiClass;
use super::transform::NiTransform;

/// IDA: vtables stored by the 05-C constructors.
pub const NI_CAMERA_VTABLE: u32 = 0x00A7_E4C4;
pub const NI_CULLING_PROCESS_VTABLE: u32 = 0x00A7_E610;

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

impl CullingPlane {
    // IDA: 00716E00 NiPlane_SetFromNormalAndPoint.
    fn from_point(normal: [f32; 3], point: [f32; 3]) -> Self {
        Self {
            normal,
            constant: normal[1] * point[1] + normal[0] * point[0] + normal[2] * point[2],
        }
    }

    // IDA: 0047DA70 NiBound_WhichSide, the negative side (return value 2).
    pub(crate) fn excludes(&self, bound: [f32; 4]) -> bool {
        let [x, y, z, radius] = bound;
        let [nx, ny, nz] = self.normal;
        (ny * y + nx * x + nz * z) - self.constant <= -radius
    }
}

// IDA: 00717A40 NiFrustumPlanes::Set. Camera basis vectors are matrix columns.
// Completion: all six planes, including the orthographic branch.
pub(crate) fn camera_frustum_planes(
    f: &CameraFrustum,
    transform: &NiTransform,
) -> [CullingPlane; 6] {
    let column = |c| {
        [
            transform.rotation[c],
            transform.rotation[c + 3],
            transform.rotation[c + 6],
        ]
    };
    let forward = column(0);
    let up = column(1);
    let right = column(2);
    let eye = transform.translation;
    let parallel = |axis: [f32; 3], offset: f32, sign: f32| {
        let point = std::array::from_fn(|i| eye[i] + axis[i] * offset);
        CullingPlane::from_point(axis.map(|v| sign * v), point)
    };
    let side = |axis: [f32; 3], edge: f32, sign: f32| {
        if f.orthographic {
            return parallel(axis, edge, sign);
        }
        let inv_length = 1.0 / (edge * edge + 1.0).sqrt();
        let along_forward = -sign * edge * inv_length;
        let along_axis = sign * inv_length;
        CullingPlane::from_point(
            std::array::from_fn(|i| forward[i] * along_forward + axis[i] * along_axis),
            eye,
        )
    };
    [
        parallel(forward, f.near_plane, 1.0),
        parallel(forward, f.far_plane, -1.0),
        side(right, f.left, 1.0),
        side(right, f.right, -1.0),
        side(up, f.top, -1.0),
        side(up, f.bottom, 1.0),
    ]
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
