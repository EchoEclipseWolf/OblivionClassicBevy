//! bhkCharacterController and the shapes, phantom and listener it owns.

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;
use oblivion_core::x87;

/// IDA: the vtables the controller path stores.
pub const NI_OBJECT_BASE_VTABLE: u32 = 0x00A9_6EB4;

pub const BHK_CHARACTER_PROXY_VTABLE: u32 = 0x00A9_667C;

pub const BHK_CHARACTER_CONTROLLER_VTABLE: u32 = 0x00A9_681C;

pub const BHK_CHARACTER_CONTROLLER_CONTEXT_VTABLE: u32 = 0x00A9_6810;

pub const BHK_CHARACTER_CONTROLLER_LISTENER_VTABLE: u32 = 0x00A9_67F4;

pub const HK_CHARACTER_CONTEXT_VTABLE: u32 = 0x00A9_6248;

pub const BHK_CHARACTER_LISTENER_VTABLE: u32 = 0x00A9_6590;

pub const HK_CD_POINT_COLLECTOR_VTABLE: u32 = 0x00A9_9DD8;

pub const BHK_WORLD_OBJECT_VTABLE: u32 = 0x00A5_59CC;

pub const BHK_PHANTOM_VTABLE: u32 = 0x00A5_5B9C;

pub const BHK_SHAPE_PHANTOM_VTABLE: u32 = 0x00A5_5C2C;

pub const BHK_SIMPLE_SHAPE_PHANTOM_VTABLE: u32 = 0x00A5_5CBC;

pub const BHK_SHAPE_VTABLE: u32 = 0x00A5_591C;

pub const BHK_SPHERE_REP_SHAPE_VTABLE: u32 = 0x00A5_5A5C;

pub const BHK_CONVEX_SHAPE_VTABLE: u32 = 0x00A5_5AFC;

pub const BHK_SPHERE_SHAPE_VTABLE: u32 = 0x00A5_5D4C;

/// IDA: ??_7bhkAvoidBox@@6B@ / ??_7bhkAabbPhantom@@6B@: the +0x368 object 00895190 creates (00890A70), and ??_7bhkCapsuleShape@@6B@ (008B6A40, the +0x374 shape).
pub const BHK_AVOID_BOX_VTABLE: u32 = 0x00A9_627C;

pub const BHK_AABB_PHANTOM_VTABLE: u32 = 0x00A7_5E64;

pub const BHK_CAPSULE_SHAPE_VTABLE: u32 = 0x00A6_5794;

/// IDA: 0065B847 / 00532110 / 005358F0 Havok units per game unit (dbl_A39088, float 0.1428767).
pub const HAVOK_SCALE: f32 = 0.142_876_729_369_163_5_f64 as f32;

/// IDA: the bhk / NiObject class instance counters the wrapper constructors increment (host records; 00B3FD64 is the NiRefObject live count NiObject_Constructor increments).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BhkClassCounters {
    pub ni_ref_objects_b3fd64: u32,
    pub character_proxies_ba8020: u32,
    pub world_objects_ba7d34: u32,
    pub phantoms_ba7f5c: u32,
    pub shape_phantoms_ba7f68: u32,
    pub simple_shape_phantoms_ba7f74: u32,
    pub shapes_ba7d70: u32,
    pub sphere_rep_shapes_ba7f44: u32,
    pub convex_shapes_ba7f50: u32,
    pub sphere_shapes_ba7f80: u32,
    pub aabb_phantoms_ba802c: u32,
}

/// IDA: 008B6A40 bhkCapsuleShape_Constructor(pointA, pointB, radius) (FormHeap 0x14): 0089D3D0, the bhkShape vtable, +0x0C = +0x10 = 0, bhkSphereRepShape / bhkConvexShape / bhkCapsuleShape with their counters, then
/// 008B6980 (the hkCapsuleShape, substituted) with the two points and the radius times 0.1428767 (24-bit products; kept). Also the 00893230 shared shape 00BA7A64 (the hkConvexVerticesShape hull 008D2770 builds is
/// substituted; only its identity is kept).
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct BhkShapeRecord {
    pub original_vtable: u32,
    pub reference_count_04: u32,
    pub havok_shape_08: bool,
    pub field_0c: u32,
    pub field_10: u32,
    pub havok_point_a: [f32; 3],
    pub havok_point_b: [f32; 3],
    pub havok_radius: f32,
}

/// IDA: 00532090 bhkSphereShape_Constructor(radius, scaleToHavok) (FormHeap 0x14): 0089D3D0 (NiObject base, +0x08 = 0), the bhkShape vtable, +0x0C = +0x10 = 0, then the bhkSphereRepShape / bhkConvexShape / bhkSphereShape
/// vtables with their counters, and vt+0x70 with {0, radius * 0.1428767 when scaled} creating the hkSphereShape (substituted: the radius is kept). Completion: 100% of the Bethesda-side stores.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct BhkSphereShapeRecord {
    pub original_vtable: u32,
    pub reference_count_04: u32,
    /// +0x08 hkSphereShape (substituted).
    pub havok_shape_08: bool,
    pub field_0c: u32,
    pub field_10: u32,
    /// The vt+0x70 argument (Havok units).
    pub havok_radius: f32,
}

/// IDA: 00531FC0 bhkSimpleShapePhantom_Constructor(cinfo) (FormHeap 0x14): 0089D3D0, the bhkWorldObject vtable, +0x0C = 0, the bhkPhantom / bhkShapePhantom vtables with +0x10 = 0, the bhkSimpleShapePhantom vtable,
/// 008AF1A0(cinfo) creating the hkSimpleShapePhantom (substituted: the cinfo's collision filter info and shape are kept) and +0x10 = 0. Also the +0x368 bhkAvoidBox (vtable 00A9627C, 00890A70 -> 0088E6F0 inside
/// 00895190): +0x20..+0x3F the AABB, +0x40..+0x5F the 00BA7A40 vectors (00895190 then stores +0x40 = {0, height, 0, 0}), +0x60 = +0x64 = 0.0, +0x68 the enabled flag 0088D560 stores, +0x69 = 0.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct BhkWorldObjectRecord {
    pub original_vtable: u32,
    pub reference_count_04: u32,
    /// +0x08 hk object (substituted).
    pub havok_object_08: bool,
    pub field_0c: u32,
    pub byte_10: u8,
    /// The hk object's +0x30 collision filter info (hkCollidable +0x14 +0x1C) Bethesda supplies through the cinfo and 0089F4D0 rewrites: (system group << 16) | (low word).
    pub collision_filter_info: u32,
    /// The cinfo's shape (phantoms).
    pub shape: Option<Entity>,
    pub aabb_20: [f32; 8],
    pub vectors_40: [f32; 8],
    pub update_distance_threshold_60: f32,
    pub float_64: f32,
    pub enabled_68: u8,
    pub byte_69: u8,
    /// IDA: N/A properties set on the hk object (008910F0 -> 008BC750: the keys).
    pub property_keys: Vec<u32>,
    /// IDA: N/A the hk object's property keyed by `rtti_1` (00B3FA80): the NiAVObject `0089F650` binds, `0089F6B0` reads.
    pub property_3d: Option<Entity>,
}

/// IDA: the stack cinfo of 0065B750 (00890B00 + 00890C00, then the 0065B750 stores), as passed to 00897230. +0x2C and +0x6C stay stack bytes (unset).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BhkCharacterControllerCinfo {
    /// +0x00..+0x08 (position * 0.1428767).
    pub position_00: [f32; 3],
    /// +0x0C (00BA7A40 vector, 0).
    pub field_0c: u32,
    /// +0x10..+0x1C (00BA7A40).
    pub vector_10: [u32; 4],
    pub float_20: f32,
    pub float_24: f32,
    pub float_28: f32,
    pub float_30: f32,
    pub float_34: f32,
    pub float_38: f32,
    pub float_3c: f32,
    pub float_40: f32,
    pub float_44: f32,
    /// +0x48 (00897040 stores the hk phantom).
    pub phantom_48: u32,
    pub float_4c: f32,
    pub float_50: f32,
    /// +0x54 (4).
    pub field_54: u32,
    pub float_58: f32,
    /// +0x5C (FLT_MAX).
    pub float_5c: f32,
    pub float_60: f32,
    /// +0x64 (pi / 2).
    pub float_64: f32,
    pub float_68: f32,
    /// +0x70 (actor +0x3C).
    pub loaded_3d_70: u32,
    /// +0x74 ((counter << 16) | 0x14).
    pub collision_filter_74: u32,
    pub field_78: u32,
    /// +0x7C (vt+0x1F4).
    pub jump_scale_7c: f32,
    pub float_80: f32,
    /// +0x84 (1).
    pub byte_84: u8,
    /// +0x85 (1 when 0065B8BF runs).
    pub no_base_bound_85: u8,
    pub initial_state_88: u32,
    pub field_8c: u32,
    /// +0x90 the character state manager (00890560).
    pub state_manager_90: bool,
    /// +0x94 (4.4291787).
    pub float_94: f32,
    pub float_98: f32,
    /// +0x9C (1).
    pub field_9c: u32,
    pub field_a0: u32,
}

/// IDA: 00897230 bhkCharacterController_Constructor(cinfo) in the 16-aligned 0x3E0 block (the byte before the object holds the alignment). Bethesda-side fields by offset. +0x08 (the hkCharacterProxy) and
/// +0x10..+0x1DF (the 008CEA50 point collector) are SDK.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct BhkCharacterController {
    /// The byte at -1 (16 - (block & 0xF)).
    pub alignment_m1: u8,
    pub original_vtable: u32,
    /// +0x004 NiRefObject count.
    pub reference_count_04: u32,
    pub field_0c: u32,
    /// +0x010: the 008CEA50 point collector header (substituted SDK object; host record).
    pub collector_vtable_10: u32,
    /// +0x1E0 hkCharacterContext (0088D310): vtable, u16 reference count +0x1E6, the state manager +0x1E8 (role), +0x1EC = cinfo +0x88.
    pub context_vtable_1e0: u32,
    pub context_reference_count_1e6: u16,
    pub context_state_manager_1e8: bool,
    pub context_state_1ec: u32,
    /// +0x1F0 bhkCharacterListener (00890130).
    pub listener_vtable_1f0: u32,
    pub flags_1f4: u32,
    /// +0x200..+0x20C (the 00897294 vector).
    pub listener_up_200: [f32; 4],
    /// +0x210 cos(slope).
    pub listener_cos_210: f32,
    /// +0x214 (0x1F).
    pub listener_field_214: u32,
    pub listener_field_218: u32,
    pub listener_zero_220: [u32; 4],
    /// +0x240 (A96588).
    pub listener_float_240: f32,
    /// +0x244 (A96588).
    pub listener_float_244: f32,
    /// +0x248 (the 00890130 argument, then 00897040).
    pub listener_float_248: f32,
    pub listener_float_24c: f32,
    pub listener_byte_250: u8,
    pub listener_power_attack_251: u8,
    pub power_attack_movement_suppression_254: u32,
    pub field_2a0: u32,
    /// +0x2B0..+0x2CC.
    pub floats_2b0: [f32; 8],
    pub zero_2f0: [u32; 4],
    /// +0x300..+0x314.
    pub floats_300: [f32; 6],
    /// +0x318: 0065A360, the parent cell's water height (004CACE0) times the Havok scale.
    pub water_height_318: f32,
    /// +0x320..+0x33C.
    pub floats_320: [f32; 8],
    /// IDA: N/A host records of the SDK calls on the +0x08 hkCharacterProxy (a D3 substitution): 00452A10 to 00891560 SetPosition with the Havok-scaled vector, and 008AC0B0(00BA7A40).
    pub host_proxy_position: [f32; 3],
    pub host_proxy_set_position_calls: u32,
    pub host_world_mark_calls: u32,
    /// +0x340..+0x34C: the proxy's offset from the reference's position, Havok units (`00891560` adds it, `00891440`
    /// subtracts it).
    pub floats_340: [f32; 4],
    /// IDA: N/A the world the hkCharacterProxy's phantom is in (`008AC0C0`'s +0x08 +0x2B0, the bhkWorld): the seam's record.
    pub host_world: Option<Entity>,
    /// IDA: N/A the seam's character for the +0x08 hkCharacterProxy, made when it first enters a world (`00895060`).
    pub host_physics: Option<crate::bs::physics_seam::PhysicsCharacterId>,
    /// IDA: N/A the hkCharacterProxy's keep distance, the cinfo's +0x4C `00897230` passes to it (SDK-held).
    pub host_keep_distance: f32,
    /// +0x364 bhkSimpleShapePhantom.
    pub phantom_364: Option<Entity>,
    /// +0x368 bhkAvoidBox (00895190).
    pub avoid_box_368: Option<Entity>,
    /// +0x36C (2, then 00894940(1)).
    pub shape_index_36c: u32,
    pub field_370: u32,
    /// +0x374/+0x378 NiPointer<bhkShape>[2]: the capsule (008905E0) and the shared 00BA7A64 shape.
    pub shapes_374: [Option<Entity>; 2],
    pub base_capsule_radius_3a0: f32,
    pub float_3a4: f32,
    pub float_3a8: f32,
    pub field_3ac: u32,
    /// +0x3B0 (cinfo +0xA0, then 1000).
    pub field_3b0: u32,
    pub field_3b4: u32,
    pub field_3b8: u32,
    /// +0x3BC (008A7560(0xF0, 0x14), substituted).
    pub havok_array_3bc: bool,
    pub field_3c0: u32,
    /// +0x3C4 (5).
    pub field_3c4: u32,
}

/// IDA: N/A one 0065A310 call on a controller: the argument and the +0x368 +0x68 byte around it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ControllerEnableCall {
    pub argument: bool,
    pub byte_before: u8,
    pub byte_after: u8,
}

/// IDA: the controller-side globals.
#[derive(Resource, Clone, Debug)]
pub struct CharacterControllerGlobals {
    /// 00B2EB3C (image 10).
    pub collision_group_b2eb3c: u32,
    /// IDA: 00BA7A54 the character state manager (built once by 00890C00; substituted: the states are recorded by their state indices in registration order).
    pub state_manager_ba7a54: bool,
    pub registered_states: Vec<u32>,
    pub state_manager_reference_count: u16,
    /// 00B333B8 (read by 0065B958).
    pub player_flag_b333b8: u8,
    pub counters: BhkClassCounters,
    /// IDA: N/A the cinfo the last 0065B750 creation passed to 00897230: as passed, and as 00890F70 releases it (after 00897040 / 00895190 wrote +0x48 / +0x50).
    pub last_cinfo: Option<BhkCharacterControllerCinfo>,
    pub last_cinfo_released: Option<BhkCharacterControllerCinfo>,
    /// IDA: 00BA7A64 the shared shape 00895190 builds once (00893230) for every controller.
    pub shared_shape_ba7a64: Option<Entity>,
    /// IDA: N/A trace of the 0065A310 calls, in order.
    pub enable_calls: Vec<ControllerEnableCall>,
    /// IDA: N/A trace of the 0089F4D0 calls, in order: the filter info before and after.
    pub layer_calls: Vec<(u32, u32)>,
}

impl Default for CharacterControllerGlobals {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            collision_group_b2eb3c: 10,
            state_manager_ba7a54: false,
            registered_states: Vec::new(),
            state_manager_reference_count: 0,
            player_flag_b333b8: 0,
            counters: BhkClassCounters::default(),
            last_cinfo: None,
            last_cinfo_released: None,
            shared_shape_ba7a64: None,
            enable_calls: Vec::new(),
            layer_calls: Vec::new(),
        }
    }
}

// IDA: N/A the globals of the code below.
pub(crate) fn globals(world: &mut World) -> Mut<'_, CharacterControllerGlobals> {
    world.resource_mut::<CharacterControllerGlobals>()
}

// IDA: 00532090 bhkSphereShape_Constructor(radius, scaleToHavok). Completion: 100% of the Bethesda-side stores.
pub(crate) fn construct_sphere_shape(world: &mut World, radius: f32, scale: bool) -> Entity {
    let mut globals = globals(world);
    let counters = &mut globals.counters;
    // 0089D3D0 bhkRefObject_Constructor: NiObject_Constructor (NiRefObject vtable, count 0, ++00B3FD64, NiObject vtable), then vtable A96EB4 and +0x08 = 0.
    counters.ni_ref_objects_b3fd64 += 1;
    let mut shape = BhkSphereShapeRecord {
        original_vtable: NI_OBJECT_BASE_VTABLE,
        reference_count_04: 0,
        havok_shape_08: false,
        ..BhkSphereShapeRecord::default()
    };
    // 005320BF..005320F6 the bhkShape vtable, +0x0C = +0x10 = 0, the three derived vtables and their counters.
    shape.original_vtable = BHK_SHAPE_VTABLE;
    shape.field_0c = 0;
    shape.field_10 = 0;
    counters.shapes_ba7d70 += 1;
    shape.original_vtable = BHK_SPHERE_REP_SHAPE_VTABLE;
    counters.sphere_rep_shapes_ba7f44 += 1;
    shape.original_vtable = BHK_CONVEX_SHAPE_VTABLE;
    counters.convex_shapes_ba7f50 += 1;
    shape.original_vtable = BHK_SPHERE_SHAPE_VTABLE;
    counters.sphere_shapes_ba7f80 += 1;
    // 005320FC..00532132 vt+0x70({0, radius * 0.1428767 when scaled}): the hkSphereShape (substituted; the 24-bit product is kept).
    shape.havok_radius = if scale { radius * HAVOK_SCALE } else { radius };
    shape.havok_shape_08 = true;
    world.spawn(shape).id()
}

// IDA: 00531FC0 bhkSimpleShapePhantom_Constructor(cinfo {filter info, hk shape, ...}). Completion: 100% of the Bethesda-side stores; 008AF1A0 (the hkSimpleShapePhantom) is substituted.
pub(crate) fn construct_simple_shape_phantom(
    world: &mut World,
    filter_info: u32,
    shape: Option<Entity>,
) -> Entity {
    let mut globals = globals(world);
    let counters = &mut globals.counters;
    // 0089D3D0.
    counters.ni_ref_objects_b3fd64 += 1;
    let mut phantom = BhkWorldObjectRecord {
        original_vtable: NI_OBJECT_BASE_VTABLE,
        reference_count_04: 0,
        havok_object_08: false,
        ..BhkWorldObjectRecord::default()
    };
    // 00531FF1..00532020 bhkWorldObject vtable, +0x0C = 0, bhkPhantom / bhkShapePhantom vtables with +0x10 = 0 and their counters.
    phantom.original_vtable = BHK_WORLD_OBJECT_VTABLE;
    phantom.field_0c = 0;
    counters.world_objects_ba7d34 += 1;
    phantom.original_vtable = BHK_PHANTOM_VTABLE;
    counters.phantoms_ba7f5c += 1;
    phantom.byte_10 = 0;
    phantom.original_vtable = BHK_SHAPE_PHANTOM_VTABLE;
    counters.shape_phantoms_ba7f68 += 1;
    phantom.byte_10 = 0;
    // 0053202E..0053203F the bhkSimpleShapePhantom vtable, 008AF1A0(cinfo), the counter and +0x10 = 0.
    phantom.original_vtable = BHK_SIMPLE_SHAPE_PHANTOM_VTABLE;
    phantom.havok_object_08 = true;
    phantom.collision_filter_info = filter_info;
    phantom.shape = shape;
    counters.simple_shape_phantoms_ba7f74 += 1;
    phantom.byte_10 = 0;
    world.spawn(phantom).id()
}

// IDA: N/A a NiPointer<world object> store (InterlockedDecrement / InterlockedIncrement in the original): the record keeps the +0x04 count.
pub(crate) fn assign_pointer(world: &mut World, slot: &mut Option<Entity>, value: Entity) {
    if *slot == Some(value) {
        return;
    }
    if let Some(old) = *slot {
        world
            .get_mut::<BhkWorldObjectRecord>(old)
            .expect("a NiPointer target")
            .reference_count_04 -= 1;
    }
    *slot = Some(value);
    world
        .get_mut::<BhkWorldObjectRecord>(value)
        .expect("a NiPointer target")
        .reference_count_04 += 1;
}

// IDA: N/A a NiPointer<bhkShape> store.
fn assign_shape(world: &mut World, slot: &mut Option<Entity>, value: Entity) {
    if *slot == Some(value) {
        return;
    }
    if let Some(old) = *slot {
        world
            .get_mut::<BhkShapeRecord>(old)
            .expect("a NiPointer target")
            .reference_count_04 -= 1;
    }
    *slot = Some(value);
    world
        .get_mut::<BhkShapeRecord>(value)
        .expect("a NiPointer target")
        .reference_count_04 += 1;
}

// IDA: the (layer << 16) | low packing of the phantom cinfos (00532250: a stack record with the identity transform, +0x08 = 2, +0x14 = 0x80000000 and the filter info / shape the caller fills; only the filter info and
// the shape reach the substituted SDK object) and of 0089F4D0.
pub(crate) const fn collision_filter(high: u32, low: u32) -> u32 {
    (high << 16) | low
}

// IDA: the 00B2EB3C counter increment of 00890C45 / 005323FC: (n + 1) & 0xFFFF, 0 -> 10.
pub(crate) fn next_collision_group(globals: &mut CharacterControllerGlobals) -> u32 {
    let mut value = globals.collision_group_b2eb3c.wrapping_add(1) & 0xFFFF;
    if value == 0 {
        value = 10;
    }
    globals.collision_group_b2eb3c = value;
    value
}

// IDA: 00890B00 + 00890C00(1) bhkCharacterControllerCinfo_Constructor. Completion: 100% of the store order.
#[expect(
    clippy::field_reassign_with_default,
    reason = "00890B00's stores stay one statement each, in original store order"
)]
pub(crate) fn construct_controller_cinfo(
    globals: &mut CharacterControllerGlobals,
) -> BhkCharacterControllerCinfo {
    let mut cinfo = BhkCharacterControllerCinfo::default();
    // 00890B00.
    cinfo.position_00 = [0.0; 3];
    cinfo.field_0c = 0;
    cinfo.vector_10 = [0; 4];
    cinfo.float_20 = 1.0;
    cinfo.float_24 = 0.0;
    // flt_A2FAAC.
    cinfo.float_28 = 0.1;
    cinfo.float_30 = 0.0;
    cinfo.float_38 = 0.0;
    cinfo.float_3c = 0.0;
    cinfo.float_34 = 1.0;
    cinfo.phantom_48 = 0;
    cinfo.field_54 = 4;
    cinfo.float_40 = 0.0;
    cinfo.float_44 = 0.0;
    // flt_A43328.
    cinfo.float_4c = 0.05;
    cinfo.float_50 = 1.0;
    // flt_A31C80.
    cinfo.float_58 = 10.0;
    // flt_A965AC FLT_MAX.
    cinfo.float_5c = f32::from_bits(0x7F7F_FFFF);
    cinfo.float_60 = 0.0;
    // flt_A3F3E0 pi / 2.
    cinfo.float_64 = f32::from_bits(0x3FC9_0FDB);
    cinfo.float_68 = 1.0;
    // 00890C30..00890C4D +0x74 = 0, +0x8C = 0, +0x84 = 1, +0x70 = 0, the group counter.
    cinfo.collision_filter_74 = 0;
    cinfo.field_8c = 0;
    cinfo.byte_84 = 1;
    cinfo.loaded_3d_70 = 0;
    let group = next_collision_group(globals);
    // 00890C67..00890CFA in store order.
    cinfo.jump_scale_7c = 0.0;
    cinfo.float_80 = 1.0;
    cinfo.collision_filter_74 = collision_filter(group, 0x14);
    cinfo.field_78 = 0;
    // flt_A967D0.
    cinfo.float_94 = f32::from_bits(0x408D_BBD5);
    cinfo.initial_state_88 = 0;
    cinfo.state_manager_90 = false;
    cinfo.field_9c = 1;
    cinfo.field_a0 = 0;
    cinfo.float_98 = 1.0;
    cinfo.no_base_bound_85 = 0;
    // 00890CB8..00890CD1 +0x00..+0x1F = the 00BA7A40 vector (zero, P11-RUN-001).
    cinfo.position_00 = [0.0; 3];
    cinfo.field_0c = 0;
    cinfo.float_50 = 0.0;
    cinfo.float_24 = 0.0;
    cinfo.vector_10 = [0; 4];
    cinfo.float_40 = 0.0;
    cinfo.phantom_48 = 0;
    cinfo.float_44 = 0.0;
    cinfo.field_54 = 4;
    cinfo.float_20 = 1.0;
    // flt_A3D65C.
    cinfo.float_4c = 0.5;
    cinfo.float_28 = 1.0;
    // flt_A2FE7C.
    cinfo.float_58 = 100.0;
    // 00890CFD the argument (1): 00890D03 build the state manager once (00BA7A54): its state objects (Havok allocator, substituted) registered as 0, 5, 1, 2, 4 (flying) and 6.
    if !globals.state_manager_ba7a54 {
        globals.state_manager_ba7a54 = true;
        globals.registered_states = vec![0, 5, 1, 2, 4, 6];
        // 008BA092 the manager's constructor: +0x06 = 1. The C++ starts at 0.
        globals.state_manager_reference_count = 1;
    }
    // 00890F45 00890560: +0x90 = the state manager with a reference.
    cinfo.state_manager_90 = true;
    globals.state_manager_reference_count += 1;
    cinfo
}

// IDA: 008B6A40 bhkCapsuleShape_Constructor (via 008905E0: FormHeap(0x14)). Completion: 100% of the Bethesda-side stores; 008B6980 (the hkCapsuleShape) is substituted with its inputs kept.
fn construct_capsule_shape(world: &mut World, a: [f32; 3], b: [f32; 3], radius: f32) -> Entity {
    let mut globals = globals(world);
    let counters = &mut globals.counters;
    counters.ni_ref_objects_b3fd64 += 1;
    let mut shape = BhkShapeRecord {
        original_vtable: NI_OBJECT_BASE_VTABLE,
        reference_count_04: 0,
        havok_shape_08: false,
        ..BhkShapeRecord::default()
    };
    shape.original_vtable = BHK_SHAPE_VTABLE;
    shape.field_0c = 0;
    shape.field_10 = 0;
    counters.shapes_ba7d70 += 1;
    shape.original_vtable = BHK_SPHERE_REP_SHAPE_VTABLE;
    counters.sphere_rep_shapes_ba7f44 += 1;
    shape.original_vtable = BHK_CONVEX_SHAPE_VTABLE;
    counters.convex_shapes_ba7f50 += 1;
    shape.original_vtable = BHK_CAPSULE_SHAPE_VTABLE;
    for index in 0..3 {
        shape.havok_point_a[index] = a[index] * HAVOK_SCALE;
        shape.havok_point_b[index] = b[index] * HAVOK_SCALE;
    }
    shape.havok_radius = HAVOK_SCALE * radius;
    shape.havok_shape_08 = true;
    world.spawn(shape).id()
}

// IDA: 00890A70 bhkAvoidBox wrapper stores over 0089D3D0, then 0088E6F0: +0x64 = 0.0, +0x40..+0x5F = 00BA7A40 (zero), +0x60 = 0.0, +0x68 = +0x69 = 0, the AABB copy, the hkAabbPhantom (substituted) with the filter.
// Completion: 100% of the Bethesda-side stores.
fn construct_avoid_box(world: &mut World, aabb_20: [f32; 8], filter: u32) -> Entity {
    let mut globals = globals(world);
    let counters = &mut globals.counters;
    counters.ni_ref_objects_b3fd64 += 1;
    let mut object = BhkWorldObjectRecord {
        original_vtable: NI_OBJECT_BASE_VTABLE,
        reference_count_04: 0,
        havok_object_08: false,
        ..BhkWorldObjectRecord::default()
    };
    object.original_vtable = BHK_WORLD_OBJECT_VTABLE;
    object.field_0c = 0;
    counters.world_objects_ba7d34 += 1;
    object.original_vtable = BHK_PHANTOM_VTABLE;
    counters.phantoms_ba7f5c += 1;
    object.byte_10 = 0;
    object.original_vtable = BHK_AABB_PHANTOM_VTABLE;
    counters.aabb_phantoms_ba802c += 1;
    object.byte_10 = 0;
    object.original_vtable = BHK_AVOID_BOX_VTABLE;
    object.float_64 = 0.0;
    object.vectors_40 = [0.0; 8];
    object.update_distance_threshold_60 = 0.0;
    object.enabled_68 = 0;
    object.byte_69 = 0;
    object.aabb_20 = aabb_20;
    object.havok_object_08 = true;
    object.collision_filter_info = filter;
    world.spawn(object).id()
}

// IDA: 00895190 bhkCharacterController_CreateShapes. The creature uses its scaled BBX
// (006FBA90 BSBound_GetFromObject, 004707B0 NiPoint3_Scale) or its node sphere.
// Completion: the vertical capsule, hull ownership and avoid box. The SDK hull is a host substitution.
// TODO: [ ] The owner-shape and horizontal branches.
fn create_controller_shapes(
    world: &mut World,
    c: &mut BhkCharacterController,
    cinfo: &mut BhkCharacterControllerCinfo,
    node: Option<Entity>,
) -> Result<(), GameError> {
    // 008951DB +0x370 = 2.
    c.field_370 = 2;
    // 008951ED cinfo +0x8C (an owner shape) and 0089526A cinfo +0x70 with +0x85 clear.
    if cinfo.field_8c != 0 {
        return Err(GameError::unclassified(
            0x0089520D,
            "0089520D: a cinfo owner shape is not reached",
        ));
    }
    let scale = if cinfo.no_base_bound_85 != 0 {
        1.0
    } else {
        cinfo.float_98
    };
    let (center, [width, depth, height]) =
        if let Some(node) = node.filter(|_| cinfo.no_base_bound_85 == 0) {
            let bound = crate::gamebryo::object_net::find_extra_data(world, node, b"BBX")?
                .and_then(|extra| world.get::<crate::gamebryo::extra_data::BsBound>(extra));
            if let Some(bound) = bound {
                (
                    bound.center_0c.map(|bits| f32::from_bits(bits) * scale),
                    bound.extents_18.map(|bits| f32::from_bits(bits) * scale),
                )
            } else {
                let radius = world
                    .get::<crate::gamebryo::scene_node::NiAvWorld>(node)
                    .ok_or(GameError::unclassified(
                        0x00895362,
                        "controller 3D has no world sphere",
                    ))?
                    .bound[3];
                let side = 25.0 * scale;
                let extents = if c.flags_1f4 & 1 != 0 {
                    [side, radius, side]
                } else {
                    [side, side, radius]
                };
                ([0.0; 3], extents)
            }
        } else {
            ([0.0; 3], [23.0 * scale, 17.5 * scale, scale * 64.0])
        };
    // 0089532D.. +0x1F4 bit 0 when the height is not the largest extent (not reached here).
    if !(height >= width && depth <= height) {
        c.flags_1f4 |= 1;
    }
    if c.flags_1f4 & 1 != 0 {
        return Err(GameError::unclassified(
            0x00895465,
            "00895465: the bit-0 (horizontal) controller shape is not reached",
        ));
    }
    // 00895B2B.. the vertical capsule: the extent 2 * height, the radius (width + depth) / 2, the top centre height - radius, the bottom -(top).
    let extent = height + height; // v106
    let radius = (width + depth) * 0.5; // v100
    let top = [0.0, 0.0, height - radius + 0.0];
    let bottom = [0.0, 0.0, -top[2]];
    let scaled_reach = radius * 0.75 * HAVOK_SCALE;
    if scaled_reach > c.listener_float_248 {
        c.listener_float_248 = scaled_reach;
    }
    // 00895CDC.. +0x340 / +0x344 = the bounding center's x / y times the Havok scale.
    c.floats_340[0] = center[0] * HAVOK_SCALE;
    c.floats_340[1] = center[1] * HAVOK_SCALE;
    if c.shapes_374[0].is_some() {
        return Err(GameError::unclassified(
            0,
            "00895D..: an existing controller shape is not reached",
        ));
    }
    // +0x314 = cinfo +0x4C * 2 + (top + radius) * scale; +0x348 += it.
    let step = cinfo.float_4c * 2.0 + (top[2] + radius) * HAVOK_SCALE;
    c.floats_300[5] = step;
    c.floats_340[2] += step;
    // cinfo +0x50 = radius * scale while it equals 0.
    if cinfo.float_50 == 0.0 {
        cinfo.float_50 = radius * HAVOK_SCALE;
    }
    let havok_radius = cinfo.float_50; // v122
    let avoid_height = 2.0 * havok_radius + 0.5 * havok_radius; // v123
    let mut avoid_bottom = bottom[2] * HAVOK_SCALE - havok_radius; // v124
    let mut avoid_top = HAVOK_SCALE * top[2]; // v111
    // 008905E0(top, bottom, radius) -> +0x374 (the capsule).
    let capsule = construct_capsule_shape(world, top, bottom, radius);
    assign_shape(world, &mut c.shapes_374[0], capsule);
    // 00893230 bhkCharacterController_CreateConvexHullShape. Only the default hull has a global owner.
    let share_hull = cinfo.no_base_bound_85 != 0;
    let shared = globals(world).shared_shape_ba7a64.filter(|_| share_hull);
    let hull = if let Some(shape) = shared {
        shape
    } else {
        let shape = world
            .spawn(BhkShapeRecord {
                havok_shape_08: true,
                havok_radius: radius * HAVOK_SCALE,
                reference_count_04: u32::from(share_hull),
                ..BhkShapeRecord::default()
            })
            .id();
        if share_hull {
            globals(world).shared_shape_ba7a64 = Some(shape);
        }
        shape
    };
    assign_shape(world, &mut c.shapes_374[1], hull);
    // LABEL_95..: +0x3A0 = +0x3A8 = radius * scale, +0x3A4 = extent * scale, +0x33C = scale * (height + height).
    let scaled_radius = radius * HAVOK_SCALE;
    c.base_capsule_radius_3a0 = scaled_radius;
    c.float_3a8 = scaled_radius;
    c.float_3a4 = extent * HAVOK_SCALE;
    c.floats_320[7] = HAVOK_SCALE * (height + height);
    // 00894940(cinfo +0x8C ? 0 : cinfo +0x9C): a different index below 2 with a shape there becomes +0x36C (the proxy's shape update is substituted).
    let index = if cinfo.field_8c != 0 {
        0
    } else {
        cinfo.field_9c
    };
    if c.shape_index_36c != index && index < 2 && c.shapes_374[index as usize].is_some() {
        if c.flags_1f4 & 0x8000 != 0 {
            return Err(GameError::unclassified(
                0x00894950,
                "00894950: a world-held controller shape swap is not reached",
            ));
        }
        c.shape_index_36c = index;
    }
    // cinfo +0x84: the bhkAvoidBox on {(-2r, -2r, bottom), (2r, 2r, top)}.
    if cinfo.byte_84 != 0 {
        let filter = (cinfo.collision_filter_74 & 0xFFFF_FFC0) | 0x15;
        avoid_bottom += c.listener_float_248;
        avoid_top += havok_radius;
        if avoid_top <= avoid_bottom {
            avoid_top += c.listener_float_248;
        }
        let low = -havok_radius * 2.0;
        let high = 2.0 * havok_radius;
        // j_MemoryHeap_Allocate(0x80, 1), 16-aligned; 00890A70.
        let object = construct_avoid_box(
            world,
            [low, low, avoid_bottom, 0.0, high, high, avoid_top, 0.0],
            filter,
        );
        assign_pointer(world, &mut c.avoid_box_368, object);
        // +0x368 +0x40 = {0, v123, 0, 0}; 008A5090 releases the cinfo arrays.
        let mut object = world
            .get_mut::<BhkWorldObjectRecord>(object)
            .expect("just spawned");
        object.vectors_40[0] = 0.0;
        object.vectors_40[1] = avoid_height;
        object.vectors_40[2] = 0.0;
        object.vectors_40[3] = 0.0;
    }
    Ok(())
}

// IDA: 00890130 bhkCharacterListener_Constructor(up, slopeRadians, height) on controller +0x1F0. x87 at 24-bit precision: cos / sin of the float angle rounded to float (host double CRT, substitution for __CIcos /
// __CIsin), the quotient and product rounded at each step. Completion: 100%.
fn construct_listener(c: &mut BhkCharacterController, up: [f32; 4], slope: f32, height: f32) {
    c.listener_vtable_1f0 = BHK_CHARACTER_LISTENER_VTABLE;
    c.listener_up_200 = up;
    let cosine = f64::from(slope).cos() as f32;
    c.listener_cos_210 = cosine;
    c.flags_1f4 = 0;
    c.listener_float_248 = height;
    c.listener_field_214 = 0x1F;
    c.listener_field_218 = 0;
    // flt_A96588 FLT_MIN.
    c.listener_float_244 = f32::from_bits(0x0080_0000);
    c.listener_zero_220 = [0; 4];
    c.listener_float_240 = f32::from_bits(0x0080_0000);
    c.listener_byte_250 = 0;
    c.power_attack_movement_suppression_254 = 0;
    c.listener_power_attack_251 = 0;
    let sine = f64::from(slope).sin() as f32;
    let quotient = height / sine;
    c.listener_float_24c = quotient * cosine;
}

// IDA: 00897230 bhkCharacterController_Constructor(cinfo). Completion: 100% of the Bethesda-side stores; 008CEA50, 008A7560, 00895190's SDK objects, 008B9E50 and 008ACD60 are the recorded substitution.
pub(crate) fn construct_character_controller(
    world: &mut World,
    cinfo: &mut BhkCharacterControllerCinfo,
    alignment: u8,
    node: Option<Entity>,
) -> Result<Entity, GameError> {
    let mut c = BhkCharacterController {
        alignment_m1: alignment,
        // 00897270 00890240 bhkCharacterProxy base: 0089D3D0, +0x0C = 0, the bhkCharacterProxy vtable, 008CEA50(+0x10, 0) (point collector, substituted) and ++00BA8020.
        original_vtable: NI_OBJECT_BASE_VTABLE,
        reference_count_04: 0,
        field_0c: 0,
        ..BhkCharacterController::default()
    };
    {
        let mut globals = globals(world);
        globals.counters.ni_ref_objects_b3fd64 += 1;
        c.original_vtable = BHK_CHARACTER_PROXY_VTABLE;
        c.collector_vtable_10 = HK_CD_POINT_COLLECTOR_VTABLE;
        globals.counters.character_proxies_ba8020 += 1;
        // 0089728F 0088D310(+0x1E0, cinfo +0x90, cinfo +0x88): hkCharacterContext (u16 count 1, the state manager with a reference, +0x1EC).
        c.context_reference_count_1e6 = 1;
        c.context_vtable_1e0 = HK_CHARACTER_CONTEXT_VTABLE;
        c.context_state_manager_1e8 = cinfo.state_manager_90;
        c.context_state_1ec = cinfo.initial_state_88;
        if cinfo.state_manager_90 {
            globals.state_manager_reference_count += 1;
        }
    }
    // 00897294..008972EA 00890130(+0x1F0, {0, 0, 1, 0}, 00B2E77C * pi / 180 (dbl_A968B0, one 24-bit product), cinfo +0x98 * +0x94).
    let height = cinfo.float_98 * cinfo.float_94;
    let slope = (f64::from(f32::from_bits(0x423C_0000)) * 0.017_453_292_519_943_295) as f32;
    construct_listener(&mut c, [0.0, 0.0, 1.0, 0.0], slope, height);
    // 008972EF..00897329 the three vtables, +0x364 = +0x368 = 0, +0x374 NiPointer[2] = 0.
    c.original_vtable = BHK_CHARACTER_CONTROLLER_VTABLE;
    c.context_vtable_1e0 = BHK_CHARACTER_CONTROLLER_CONTEXT_VTABLE;
    c.listener_vtable_1f0 = BHK_CHARACTER_CONTROLLER_LISTENER_VTABLE;
    c.phantom_364 = None;
    c.avoid_box_368 = None;
    c.shapes_374 = [None; 2];
    // 00897334..00897365 +0x3BC = 008A7560(TLS +0x19C or 00BA7D9C, 0xF0, 0x14) (Havok array).
    c.havok_array_3bc = true;
    // 0089736B..00897460 in store order.
    c.field_3c4 = 5;
    c.base_capsule_radius_3a0 = 2.0;
    c.field_3ac = 0;
    c.float_3a8 = 2.0;
    c.flags_1f4 = 0;
    c.float_3a4 = 10.0;
    c.floats_340 = [0.0; 4];
    c.zero_2f0 = [0; 4];
    c.floats_320[0] = 0.0; // +0x320
    c.floats_320[1] = 0.0; // +0x324
    c.floats_300[0] = 0.0; // +0x300
    c.floats_320[3] = 0.0; // +0x32C
    c.floats_320[4] = 1.0; // +0x330
    c.floats_300[3] = 0.0; // +0x30C
    c.floats_300[1] = 0.0; // +0x304
    c.floats_300[2] = 0.0; // +0x308
    c.floats_300[4] = 0.0; // +0x310
    c.floats_300[5] = 0.0; // +0x314
    c.floats_320[2] = 1.0; // +0x328
    c.floats_2b0[0] = 0.0; // +0x2B0
    c.floats_2b0[1] = 0.0; // +0x2B4
    c.floats_2b0[3] = 0.0; // +0x2BC
    c.floats_2b0[2] = 1.0; // +0x2B8
    c.floats_2b0[4] = 1.0; // +0x2C0
    c.floats_2b0[5] = 0.0; // +0x2C4
    c.floats_2b0[6] = 0.0; // +0x2C8
    c.floats_2b0[7] = 0.0; // +0x2CC
    c.field_2a0 = 0;
    c.shape_index_36c = 2;
    c.field_370 = 2;
    c.field_3b0 = 0;
    c.field_3b4 = 0;
    c.field_3b8 = 0;
    c.field_3c0 = 0;
    // 00897466 00897040(cinfo).
    // 0089707B..00897090 +0x248 = cinfo +0x98 * +0x94 (stored float).
    c.listener_float_248 = cinfo.float_98 * cinfo.float_94;
    c.host_keep_distance = cinfo.float_4c;
    // 00897096 00895190(cinfo): the shapes, the dimensions and the +0x368 bhkAvoidBox.
    create_controller_shapes(world, &mut c, cinfo, node)?;
    // 0089709B..008970F2 the phantom cinfo (00532250): filter = cinfo +0x74, the shape of 00890BA0 (the +0x36C-indexed +0x374 slot; SDK-held), then FormHeap(0x14) 00531FC0.
    let phantom = construct_simple_shape_phantom(world, cinfo.collision_filter_74, None);
    // 008970F4..00897105 NiRefObjectPtr_Assign(+0x364, phantom).
    assign_pointer(world, &mut c.phantom_364, phantom);
    // 008971B5 8B9E50 -> vt+0x4C 0089D730 -> 0089D400 -> vt+0x50 008B9BE0: 008B9C1B InterlockedIncrement on the bhk wrapper's +0x04, whatever the SDK does around it. The C++ misses this reference; P11-RUN-001 shows 2.
    if let Some(phantom) = c.phantom_364 {
        world
            .get_mut::<BhkWorldObjectRecord>(phantom)
            .ok_or(GameError::unclassified(
                0x00897105,
                "00897105: the phantom has no wrapper record",
            ))?
            .reference_count_04 += 1;
    }
    // 0089710A cinfo +0x78 (null here): the world add arm (not reached).
    if cinfo.field_78 != 0 {
        return Err(GameError::unclassified(
            0x00897111,
            "00897111: adding the controller phantom to a world is not reached",
        ));
    }
    // 00897175..008971A0 +0x2A0, +0x310, +0x328, +0x3B0 from the cinfo.
    c.field_2a0 = cinfo.initial_state_88;
    c.floats_300[4] = cinfo.jump_scale_7c;
    c.floats_320[2] = cinfo.float_80;
    c.field_3b0 = cinfo.field_a0;
    // 008971A6..008971B2 cinfo +0x48 = the phantom's hk object (role).
    cinfo.phantom_48 = 1;
    // 008971B5 008B9E50(cinfo) and 008971CA 008ACD60(listener): SDK (substituted).
    Ok(world.spawn(c).id())
}

/// IDA: `0089F650`(phantom, node, rtti_1): the hk object's rtti_1 property removed (`008BC7B0`) when present, then a nonnull
/// node added (`008BC750`). Completion: 100% of the record.
pub(crate) fn bind_phantom_3d(world: &mut World, phantom: Entity, node: Option<Entity>) {
    let mut record = world
        .get_mut::<BhkWorldObjectRecord>(phantom)
        .expect("a bhkWorldObject");
    const RTTI_1: u32 = 0x00B3_FA80;
    record.property_keys.retain(|&key| key != RTTI_1);
    record.property_3d = None;
    if let Some(node) = node {
        record.property_keys.push(RTTI_1);
        record.property_3d = Some(node);
    }
}

/// IDA: `0057E270` bhkCharacterController_GetPhantomFilter(out): the +0x364 phantom's hk filter (+0x08 +0x14 +0x1C), 0 with no
/// phantom or hk object. Completion: 100%.
#[must_use]
pub fn controller_phantom_filter(world: &World, controller: Entity) -> u32 {
    world
        .get::<BhkCharacterController>(controller)
        .and_then(|c| c.phantom_364)
        .and_then(|phantom| world.get::<BhkWorldObjectRecord>(phantom))
        .filter(|record| record.havok_object_08)
        .map_or(0, |record| record.collision_filter_info)
}

/// IDA: `00895060` bhkCharacterController_SetWorld(world): the proxy's world (`008AC0C0` +0x08 +0x2B0) unchanged returns. An old
/// world removes the proxy (`008B9F60`) and the phantom's and +0x368's vt+0x60 (none here: the port's proxies start outside
/// a world). With a new world and a +0x364 phantom: +0x1F4 bits 0 and 0x100000 clear runs `00894BD0`(0.0014287672),
/// `008912A0`(0.0014287672) and `00894BD0`(0.0); the phantom's vt+0x5C(world) and `0088BB60`'s debug colour; +0x368's
/// vt+0x5C(world). The hk objects entering the world are the seam's character. Completion: 100% of the Bethesda-side stores.
pub(crate) fn controller_set_world(world: &mut World, controller: Entity, bhk: Option<Entity>) {
    let mut c = world
        .get::<BhkCharacterController>(controller)
        .expect("a controller")
        .clone();
    if c.host_world == bhk {
        return;
    }
    let physics_world = bhk.and_then(|bhk| {
        world
            .get::<crate::bs::havok_world::BhkWorld>(bhk)
            .and_then(|data| data.physics)
    });
    if bhk.is_some() && c.phantom_364.is_some() {
        if c.flags_1f4 & 1 == 0 && c.flags_1f4 & 0x0010_0000 == 0 {
            controller_set_radius(&mut c, 0.001_428_767_2);
            capsule_set_radius(world, &c, 0.001_428_767_2);
            controller_set_radius(&mut c, 0.0);
        }
        if c.host_physics.is_none() {
            let desc = crate::bs::physics_seam::CharacterDesc {
                shape: controller_capsule(world, &c),
                position: c.host_proxy_position,
                filter_info: controller_phantom_filter(world, controller),
                keep_distance: c.host_keep_distance,
            };
            c.host_physics = crate::bs::physics_seam::seam_create_character(world, &desc);
        }
    }
    c.host_world = bhk;
    crate::bs::physics_seam::seam_set_character_world(world, c.host_physics, physics_world);
    *world
        .get_mut::<BhkCharacterController>(controller)
        .expect("a controller") = c;
}

/// IDA: `00894BD0` bhkCharacterController_SetTargetRadius(radius): with +0x1F4 bit 0 clear, the radius capped at half +0x3A4,
/// a value at or below 0.0 taking +0x3A0; one other than +0x3A8: +0x370 at 2 takes +0x36C and `00894940(0)` (+0x36C = 0, the
/// capsule; a world-held swap with +0x1F4 bit 0x8000 isn't reached), +0x3AC = 2, +0x3A8 = it. Completion: 100%.
fn controller_set_radius(c: &mut BhkCharacterController, radius: f32) {
    if c.flags_1f4 & 1 != 0 {
        return;
    }
    let half = c.float_3a4 * 0.5;
    let mut radius = if radius > half { half } else { radius };
    if radius <= 0.0 {
        radius = c.base_capsule_radius_3a0;
    }
    if c.float_3a8 != radius {
        if c.field_370 == 2 {
            c.field_370 = c.shape_index_36c;
            if c.shape_index_36c != 0 && c.shapes_374[0].is_some() {
                c.shape_index_36c = 0;
            }
        }
        c.field_3ac = 2;
        c.float_3a8 = radius;
    }
}

// IDA: 008913C0 bhkCharacterController_GetCapsuleRadius. A non-capsule or absent shape
// uses the controller's base radius. A capsule with no SDK shape answers flt_B2EFC4.
pub(crate) fn controller_capsule_radius(world: &World, controller: Entity) -> f32 {
    let c = world
        .get::<BhkCharacterController>(controller)
        .expect("a character controller");
    let capsule = c.shapes_374[0]
        .and_then(|shape| world.get::<BhkShapeRecord>(shape))
        .filter(|shape| shape.original_vtable == BHK_CAPSULE_SHAPE_VTABLE);
    match capsule {
        Some(shape) if shape.havok_shape_08 => shape.havok_radius,
        Some(_) => 0.1,
        None => c.base_capsule_radius_3a0,
    }
}

/// IDA: `008912A0` bhkCharacterController_SetCapsuleRadius(radius): a bhkCapsuleShape at +0x374: its hk capsule's points move
/// out along z by (old radius - radius), top up and bottom down, and its radius becomes `radius`. Completion: 100% on the
/// recorded capsule (the hkCapsuleShape is the record's).
fn capsule_set_radius(world: &mut World, c: &BhkCharacterController, radius: f32) {
    let Some(shape) = c.shapes_374[0] else {
        return;
    };
    let Some(mut capsule) = world.get_mut::<BhkShapeRecord>(shape) else {
        return;
    };
    if capsule.original_vtable != BHK_CAPSULE_SHAPE_VTABLE {
        return;
    }
    let grow = capsule.havok_radius - radius;
    capsule.havok_point_a[2] += grow;
    capsule.havok_point_b[2] -= grow;
    capsule.havok_radius = radius;
}

/// IDA: N/A the seam's shape for the proxy: the +0x374 capsule's segment brought back to the controller's target radius
/// (+0x3A8), the shape the controller's state update grows it to; its ends stay where `008912A0` put them.
fn controller_capsule(
    world: &World,
    c: &BhkCharacterController,
) -> crate::bs::physics_seam::ShapeDesc {
    let record = c.shapes_374[0]
        .and_then(|shape| world.get::<BhkShapeRecord>(shape).copied())
        .unwrap_or_default();
    let shrink = c.float_3a8 - record.havok_radius;
    let mut point_a = record.havok_point_a;
    let mut point_b = record.havok_point_b;
    point_a[2] -= shrink;
    point_b[2] += shrink;
    crate::bs::physics_seam::ShapeDesc::Capsule {
        point_a,
        point_b,
        radius: c.float_3a8,
    }
}

/// IDA: `00452A10` bhkCharacterController_SetHavokPosition(position) then `00891560`: the position times 0.1428767 (three
/// 24-bit products), plus the +0x340 offset (`006848D0`'s identity through `0088FE00`), into the hkCharacterProxy (the seam's).
/// Completion: 100%.
pub fn controller_set_position(world: &mut World, controller: Entity, position: [f32; 3]) {
    let mut c = world
        .get_mut::<BhkCharacterController>(controller)
        .expect("a controller");
    let scaled = [
        position[0] * HAVOK_SCALE,
        position[1] * HAVOK_SCALE,
        HAVOK_SCALE * position[2],
    ];
    c.host_proxy_position = scaled;
    c.host_proxy_set_position_calls += 1;
    let proxy = [
        scaled[0] + c.floats_340[0],
        scaled[1] + c.floats_340[1],
        scaled[2] + c.floats_340[2],
    ];
    let physics = c.host_physics;
    crate::bs::physics_seam::seam_set_character_position(world, physics, proxy);
}

/// IDA: `005E1500` bhkCharacterController_GetGamePosition(out): `00891440` (the proxy's position less the +0x340 offset) and
/// `HavokVector_ToGameUnits` (`0043F3E0`: each times 6.99904, a float). Completion: 100%.
/// # Errors
/// No physics backend.
pub fn controller_game_position(world: &World, controller: Entity) -> Result<[f32; 3], GameError> {
    let c = world
        .get::<BhkCharacterController>(controller)
        .expect("a controller");
    let proxy = crate::bs::physics_seam::seam_character_position(world, c.host_physics)?;
    let havok = [
        proxy[0] - c.floats_340[0],
        proxy[1] - c.floats_340[1],
        proxy[2] - c.floats_340[2],
    ];
    Ok(crate::bs::havok_world::to_game_units(havok))
}

/// IDA: 00894C70 bhkCharacterController_UpdateCapsuleGrowth(time): +0x3AC != 2 returns. The +0x374 shape as a
/// bhkCapsuleShape (NiDynamicCast BA7FD8): its hk capsule's radius (+0x0C, or 00B2EFC4 0.1 with none) moves toward the
/// target +0x3A8 by time x 5.0 (100.0 x 5.0 with +0x1F4 bit 0x100000), capped at the target, the two ends' z shifted by the
/// opposite of the change (A down, B up) with their w keeping the old radius; then at the target +0x3A8, with +0x3A0
/// there too, +0x370 other than 2 is `00894940` SetShapeIndex (a stop) and +0x3AC = 0, else +0x3AC = 1. The hk capsule is
/// the shape record's (the seam's proxy keeps the shape it was made with). Completion: 100% of the stores.
/// # Errors
/// `00894940`'s shape change.
pub fn controller_update_capsule_growth(
    world: &mut World,
    controller: Entity,
    time: f32,
) -> Result<(), GameError> {
    use crate::bs::character_controller::BhkShapeRecord;
    let (state, shape, flags, target, current_3a0, index) = {
        let c = world
            .get::<BhkCharacterController>(controller)
            .expect("a controller");
        (
            c.field_3ac,
            c.shapes_374[0],
            c.flags_1f4,
            c.float_3a8,
            c.base_capsule_radius_3a0,
            c.field_370,
        )
    };
    if state != 2 {
        return Ok(());
    }
    let mut reached = target;
    if let Some(shape) = shape.filter(|&shape| world.get::<BhkShapeRecord>(shape).is_some()) {
        let time = if flags & 0x10_0000 != 0 { 100.0 } else { time };
        let radius = world
            .get::<BhkShapeRecord>(shape)
            .expect("a shape")
            .havok_radius;
        let mut step = x87::stored_float(f64::from(time) * 5.0);
        let grown = if target >= radius {
            let grown = x87::stored_float(f64::from(step) + f64::from(radius));
            (target > grown).then_some(grown)
        } else {
            let shrunk = x87::stored_float(f64::from(radius) - f64::from(step));
            if target < shrunk {
                step = -step;
                Some(shrunk)
            } else {
                None
            }
        };
        let value = grown.unwrap_or_else(|| {
            step = x87::stored_float(f64::from(target) - f64::from(radius));
            target
        });
        let mut record = world.get_mut::<BhkShapeRecord>(shape).expect("a shape");
        record.havok_point_a[2] =
            x87::stored_float(f64::from(record.havok_point_a[2]) - f64::from(step));
        record.havok_point_b[2] =
            x87::stored_float(f64::from(step) + f64::from(record.havok_point_b[2]));
        record.havok_radius = value;
        reached = value;
    }
    let mut c = world
        .get_mut::<BhkCharacterController>(controller)
        .expect("a controller");
    if target == reached {
        if current_3a0 == reached {
            if index != 2 {
                return Err(GameError::unclassified(
                    0x00894E4A,
                    "00894E4A: 00894940 SetShapeIndex",
                ));
            }
            c.field_3ac = 0;
        } else {
            c.field_3ac = 1;
        }
    }
    Ok(())
}
