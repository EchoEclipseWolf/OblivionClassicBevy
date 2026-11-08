//! Phase 11 (D3): the player's Havok character controller and the two collision helpers that PlayerCharacter_ctor 0066A740 creates. 0065B750 Actor_CreateCharacterController runs when the process exists, its
//! level is 0 or 1 and the +0x40 getter (006ECC80, identical-code folded with TESClass_GetSpecialization: here TESObjectREFR +0x40 parentCell) is null or a cell whose +0x26 byte is 5 or 6 (P11-RUN-001: High
//! process, null cell: created). It fills a cinfo (00890C00 over 00890B00, with the 00B2EB3C collision-group counter and the once-built character state manager 00BA7A54), allocates a 16-aligned 0x3E0 block for
//! bhkCharacterController 00897230, stores it into the process (vt+0x190 00634910, +0x118), sets property 1000 = actor (008910F0), +0x334 = vt+0xEC, the +0x1F4 flag bits, and frees the cinfo (00890F70); then
//! vt+0x178 (0065B5C0). The window then sets +0x3B0 = 1000, enables the proxy object (0065A310 -> 008927E0 -> 0088D560), puts the +0x364 phantom and the +0x368 proxy object on layer 9 (0089F4D0) and builds the
//! camera collision helper (00532BC0(8.0, 9)) and the collision query helper (005358F0(2.0, 9) in a 16-aligned 0x1C0 block).
//!
//! bhk objects are NiObjects, so each is an entity with its record as a component and its own +0x04 count; NiPointer stores keep that count. Recorded substitution (Phase 09 D2): the Havok SDK objects underneath
//! (the hkCharacterProxy, the hkCharacterContext / state manager states, the shapes, the hkSimpleShapePhantoms, the point collectors and their world membership and properties) are not created. The records keep
//! the values Bethesda code passes to them (filter infos, radii, layers, property keys), and the queries the window makes are answered as the original world-less objects do (P11-RUN-001: no world). No physics
//! engine runs. Evidence: evidence/phase11-window-2026-10-06/disasm-reached.txt, disasm-bhk.txt, decompile-895190.c; P11-RUN-001.

use bevy_ecs::prelude::*;
use recomp_core::reader::StreamError;

use super::actor::{MobileObject, TesObjectRefr};
use super::actor_process::{MiddleHighProcess, process_level};
use super::forms_references::set_reference_position;

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
pub const HK_ALL_CD_POINT_COLLECTOR_VTABLE: u32 = 0x00A5_5DEC;
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
    /// The hk object's +0x30 collision filter info (hkCollidable +0x14 +0x1C) Bethesda supplies through the cinfo and 0089F4D0 rewrites: (layer << 16) | (low word).
    pub collision_filter_info: u32,
    /// The cinfo's shape (phantoms).
    pub shape: Option<Entity>,
    pub aabb_20: [f32; 8],
    pub vectors_40: [f32; 8],
    pub float_60: f32,
    pub float_64: f32,
    pub enabled_68: u8,
    pub byte_69: u8,
    /// IDA: N/A properties set on the hk object (008910F0 -> 008BC750: the keys).
    pub property_keys: Vec<u32>,
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
    pub field_88: u32,
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
    pub context_field_1ec: u32,
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
    pub listener_byte_251: u8,
    pub listener_field_254: u32,
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
    /// +0x340..+0x34C.
    pub floats_340: [f32; 4],
    /// +0x364 bhkSimpleShapePhantom.
    pub phantom_364: Option<Entity>,
    /// +0x368 bhkAvoidBox (00895190).
    pub proxy_object_368: Option<Entity>,
    /// +0x36C (2, then 00894940(1)).
    pub shape_index_36c: u32,
    pub field_370: u32,
    /// +0x374/+0x378 NiPointer<bhkShape>[2]: the capsule (008905E0) and the shared 00BA7A64 shape.
    pub shapes_374: [Option<Entity>; 2],
    pub float_3a0: f32,
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

/// IDA: 00532BC0 CameraCollisionHelper_Constructor(radius, layer) (FormHeap 8): +0x00 = +0x04 = 0, then 00532370: a bhkSphereShape(radius, 1) (FormHeap 0x14), two phantom cinfos (00532250), the collision-group counter,
/// phantom 0 with filter (layer << 16) | 0x18 -> +0x00 and phantom 1 with (counter << 16) | 0x18 -> +0x04 (NiPointer stores), each over the shape's hk shape.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct CameraCollisionHelper {
    pub phantom_00: Entity,
    pub phantom_04: Entity,
    /// The 00532090 object (not released here).
    pub shape: Entity,
}

/// IDA: 005358F0 CollisionQueryHelper_Constructor(radius, layer) on a 16-aligned 0x1C0 FormHeap block: hkAllCdPointCollector (vtable A55DEC, +0x04 = FLT_MAX, +0x10 = +0x20 inline storage, +0x14 = 0, +0x18 = 0x80000008),
/// +0x1A0 = 0, then 00535730: a bhkSphereShape(radius, 1), +0x1A8 = layer, a phantom with filter (layer << 16) | 0x1C -> +0x1A0 (NiPointer), +0x1A4 = 1.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct CollisionQueryHelper {
    pub alignment_m1: u8,
    pub original_vtable: u32,
    pub early_out_distance_04: f32,
    /// +0x10 points at +0x20 (role).
    pub inline_storage_10: bool,
    pub size_14: u32,
    pub capacity_18: u32,
    pub phantom_1a0: Entity,
    pub field_1a4: u8,
    pub layer_1a8: u32,
    pub shape: Entity,
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

/// IDA: N/A what the controller code reads outside itself.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ControllerInputs {
    /// IDA: 0065B85D vt+0x1F4 (005E1800 Actor_GetJumpScale: vt+0x284(26) / 100.0) on the actor; P11-RUN-001: 0.0 (no base form).
    pub jump_scale: f32,
    /// IDA: 0065B940 vt+0xEC (00611D00: 004D7260 scale, times the race height when a base form exists). P11-RUN-001: 1.0.
    pub scale: f32,
    /// IDA: N/A host seam for the j_MemoryHeap_Allocate addresses (the alignment bytes).
    pub controller_alignment: u8,
    pub query_helper_alignment: u8,
}

impl Default for ControllerInputs {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            jump_scale: 0.0,
            scale: 1.0,
            controller_alignment: 8,
            query_helper_alignment: 4,
        }
    }
}

// IDA: N/A the globals of the code below.
fn globals(world: &mut World) -> Mut<'_, CharacterControllerGlobals> {
    world.resource_mut::<CharacterControllerGlobals>()
}

// IDA: 00532090 bhkSphereShape_Constructor(radius, scaleToHavok). Completion: 100% of the Bethesda-side stores.
fn construct_sphere_shape(world: &mut World, radius: f32, scale: bool) -> Entity {
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
fn construct_simple_shape_phantom(
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
fn assign_pointer(world: &mut World, slot: &mut Option<Entity>, value: Entity) {
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
const fn collision_filter(high: u32, low: u32) -> u32 {
    (high << 16) | low
}

// IDA: the 00B2EB3C counter increment of 00890C45 / 005323FC: (n + 1) & 0xFFFF, 0 -> 10.
fn next_collision_group(globals: &mut CharacterControllerGlobals) -> u32 {
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
fn construct_controller_cinfo(
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
    cinfo.field_88 = 0;
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
    object.float_60 = 0.0;
    object.enabled_68 = 0;
    object.byte_69 = 0;
    object.aabb_20 = aabb_20;
    object.havok_object_08 = true;
    object.collision_filter_info = filter;
    world.spawn(object).id()
}

// IDA: 00895190 bhkCharacterController_CreateShapes(cinfo): the reached path of P11-RUN-001 (no cinfo +0x8C owner, no +0x70 3D or the +0x85 default flag, +0x1F4 bit 0 clear): the default dimensions 23 / 17.5 / 64 scaled by
// 1.0, the capsule over the two centres, the 00BA7A64 shared shape (00893230, built once), the radius / height stores, 00894940(cinfo +0x9C) and, with cinfo +0x84, the bhkAvoidBox (00890A70) on its AABB. x87 at 24-bit
// precision: float arithmetic in the original grouping. Completion: 100% of the reached Bethesda-side stores; the unreached arms stop.
// TODO: [ ] Sizing from a 3D bound, the base-form bound, the horizontal shape and a per-controller 00893230 shape, for NPC controllers (M5 or later).
fn create_controller_shapes(
    world: &mut World,
    c: &mut BhkCharacterController,
    cinfo: &mut BhkCharacterControllerCinfo,
) -> Result<(), StreamError> {
    // 008951DB +0x370 = 2.
    c.field_370 = 2;
    // 008951ED cinfo +0x8C (an owner shape) and 0089526A cinfo +0x70 with +0x85 clear.
    if cinfo.field_8c != 0 {
        return Err(StreamError("0089520D: a cinfo owner shape is not reached"));
    }
    if cinfo.loaded_3d_70 != 0 && cinfo.no_base_bound_85 == 0 {
        return Err(StreamError(
            "00895287: sizing the controller from a 3D bound is not reached",
        ));
    }
    // 008953E2..0089544B the scale (1.0 with +0x85, else cinfo +0x98) and the dimensions.
    let scale = if cinfo.no_base_bound_85 != 0 {
        1.0
    } else {
        cinfo.float_98
    };
    let width = 23.0 * scale; // v119
    let depth = 17.5 * scale; // v120
    let height = scale * 64.0; // v121
    // 0089532D.. +0x1F4 bit 0 when the height is not the largest extent (not reached here).
    if !(height >= width && depth <= height) {
        c.flags_1f4 |= 1;
    }
    if c.flags_1f4 & 1 != 0 {
        return Err(StreamError(
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
    // 00895CDC.. +0x340 / +0x344 = the zero point's x / y times the Havok scale.
    c.floats_340[0] = 0.0 * HAVOK_SCALE;
    c.floats_340[1] = 0.0 * HAVOK_SCALE;
    if c.shapes_374[0].is_some() {
        return Err(StreamError(
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
    // cinfo +0x85: the shared 00BA7A64 shape (00893230(top, bottom, radius, +0x248), once) -> +0x378 (NiRefObjectPtr_CopyAssign); otherwise a new one (not reached).
    if cinfo.no_base_bound_85 == 0 {
        return Err(StreamError(
            "00895C..: a per-controller 00893230 shape is not reached",
        ));
    }
    let shared = if let Some(shared) = globals(world).shared_shape_ba7a64 {
        shared
    } else {
        let shared = world
            .spawn(BhkShapeRecord {
                havok_shape_08: true,
                havok_radius: radius * HAVOK_SCALE,
                reference_count_04: 1,
                ..BhkShapeRecord::default()
            })
            .id();
        globals(world).shared_shape_ba7a64 = Some(shared);
        shared
    };
    assign_shape(world, &mut c.shapes_374[1], shared);
    // LABEL_95..: +0x3A0 = +0x3A8 = radius * scale, +0x3A4 = extent * scale, +0x33C = scale * (height + height).
    let scaled_radius = radius * HAVOK_SCALE;
    c.float_3a0 = scaled_radius;
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
            return Err(StreamError(
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
        assign_pointer(world, &mut c.proxy_object_368, object);
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
    c.listener_field_254 = 0;
    c.listener_byte_251 = 0;
    let sine = f64::from(slope).sin() as f32;
    let quotient = height / sine;
    c.listener_float_24c = quotient * cosine;
}

// IDA: 00897230 bhkCharacterController_Constructor(cinfo). Completion: 100% of the Bethesda-side stores; 008CEA50, 008A7560, 00895190's SDK objects, 008B9E50 and 008ACD60 are the recorded substitution.
fn construct_character_controller(
    world: &mut World,
    cinfo: &mut BhkCharacterControllerCinfo,
    alignment: u8,
) -> Result<Entity, StreamError> {
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
        c.context_field_1ec = cinfo.field_88;
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
    c.proxy_object_368 = None;
    c.shapes_374 = [None; 2];
    // 00897334..00897365 +0x3BC = 008A7560(TLS +0x19C or 00BA7D9C, 0xF0, 0x14) (Havok array).
    c.havok_array_3bc = true;
    // 0089736B..00897460 in store order.
    c.field_3c4 = 5;
    c.float_3a0 = 2.0;
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
    // 00897096 00895190(cinfo): the shapes, the dimensions and the +0x368 bhkAvoidBox.
    create_controller_shapes(world, &mut c, cinfo)?;
    // 0089709B..008970F2 the phantom cinfo (00532250): filter = cinfo +0x74, the shape of 00890BA0 (the +0x36C-indexed +0x374 slot; SDK-held), then FormHeap(0x14) 00531FC0.
    let phantom = construct_simple_shape_phantom(world, cinfo.collision_filter_74, None);
    // 008970F4..00897105 NiRefObjectPtr_Assign(+0x364, phantom).
    assign_pointer(world, &mut c.phantom_364, phantom);
    // 008971B5 8B9E50 -> vt+0x4C 0089D730 -> 0089D400 -> vt+0x50 008B9BE0: 008B9C1B InterlockedIncrement on the bhk wrapper's +0x04, whatever the SDK does around it. The C++ misses this reference; P11-RUN-001 shows 2.
    if let Some(phantom) = c.phantom_364 {
        world
            .get_mut::<BhkWorldObjectRecord>(phantom)
            .ok_or(StreamError("00897105: the phantom has no wrapper record"))?
            .reference_count_04 += 1;
    }
    // 0089710A cinfo +0x78 (null here): the world add arm (not reached).
    if cinfo.field_78 != 0 {
        return Err(StreamError(
            "00897111: adding the controller phantom to a world is not reached",
        ));
    }
    // 00897175..008971A0 +0x2A0, +0x310, +0x328, +0x3B0 from the cinfo.
    c.field_2a0 = cinfo.field_88;
    c.floats_300[4] = cinfo.jump_scale_7c;
    c.floats_320[2] = cinfo.float_80;
    c.field_3b0 = cinfo.field_a0;
    // 008971A6..008971B2 cinfo +0x48 = the phantom's hk object (role).
    cinfo.phantom_48 = 1;
    // 008971B5 008B9E50(cinfo) and 008971CA 008ACD60(listener): SDK (substituted).
    Ok(world.spawn(c).id())
}

// IDA: 0065A2C0 Actor_GetCharController: process vt+0x18C (006348E0: +0x118 with a reference) and the temporary NiPointer's release; null without a process. The Low / MiddleLow slot 0060D860 stores a null controller.
// Completion: 100%.
#[must_use]
pub fn actor_character_controller(world: &World, actor: Entity) -> Option<Entity> {
    let process = world.get::<MobileObject>(actor)?.process_58?;
    world.get::<MiddleHighProcess>(process)?.char_controller_118
}

/// IDA: 0065A3E0 Actor_SetPositionAndRefreshProcess (vt+0x1CC on Actor, Character and PlayerCharacter), in original order: 004D8A30 the reference position; 0065A2C0 the controller; with one, 00452A10 (`fld qword` 0.1428767293691635, whose f64 is exactly a float, then the three 24-bit products, x * C, y * C, C * z, into 00891560); 008AC0B0(00BA7A40) when +0x08 is nonnull; +0x320 = 0.0; bit 7 of +0x1F4 cleared when set. Completion: 100% of the Bethesda-side stores; the two SDK calls are host records, D3.
/// # Errors
/// A stop under vt+0x40 (0045B670's null globals).
pub fn actor_set_position(
    world: &mut World,
    actor: Entity,
    position: [f32; 3],
) -> Result<(), StreamError> {
    set_reference_position(world, actor, position)?;
    let Some(controller) = actor_character_controller(world, actor) else {
        return Ok(());
    };
    let mut body = world
        .get_mut::<BhkCharacterController>(controller)
        .expect("the process's controller");
    body.host_proxy_position = [
        position[0] * HAVOK_SCALE,
        position[1] * HAVOK_SCALE,
        HAVOK_SCALE * position[2],
    ];
    body.host_proxy_set_position_calls += 1;
    // 0065A419 +0x08: Phase 11's 00895190 always made the proxy, so `proxy_08` isn't a host field.
    body.host_world_mark_calls += 1;
    body.floats_320[0] = 0.0;
    if (body.flags_1f4 >> 7) & 1 != 0 {
        body.flags_1f4 &= 0xFFFF_FF7F;
    }
    Ok(())
}

// IDA: 0065B750 Actor_CreateCharacterController. Completion: 100% of the reached arms; unreached arms (a living-dead actor's queries, a world change) stop.
// TODO: [ ] The cell, 3D, base-form and world arms, for an actor in a cell (M5, Phase 16).
pub fn actor_create_character_controller(
    world: &mut World,
    actor: Entity,
    inputs: ControllerInputs,
) -> Result<(), StreamError> {
    // 0065B792..0065B7B7 a process with level 0 or 1.
    let Some(process) = world
        .get::<MobileObject>(actor)
        .and_then(|mobile| mobile.process_58)
    else {
        return Ok(());
    };
    let level = process_level(world, process);
    if level != 0 && level != 1 {
        return Ok(());
    }
    let reference = world
        .get::<TesObjectRefr>(actor)
        .expect("an actor is a reference")
        .clone();
    // 0065B7BD..0065B7E0 006ECC80 (+0x40 parentCell): null, or a cell with +0x26 == 6 / 5.
    if reference.parent_cell_40.is_some() {
        return Err(StreamError(
            "0065B7C8: a controller for an actor in a cell is not reached",
        ));
    }
    // 0065B7E6..0065B828 the Actor cast; dead = 005E0DC0 Actor_GetDeathState (+0xB0) == 2 or 005F0310(-1) (+0x3C 3D null: false).
    if reference.loaded_3d_3c != 0 {
        return Err(StreamError(
            "0065B813: 005F0310 with a loaded 3D is not reached",
        ));
    }
    // 0065B82A 0065A2C0.
    if actor_character_controller(world, actor).is_none() {
        // 0065B839 00890C00(1).
        let mut cinfo = construct_controller_cinfo(&mut globals(world));
        // 0065B844..0065B883 position * 0.1428767 (24-bit products), +0x70 = actor +0x3C, +0x78 = 0, +0x7C = vt+0x1F4 (jump scale).
        cinfo.position_00[0] = reference.position_2c[0] * HAVOK_SCALE;
        cinfo.loaded_3d_70 = reference.loaded_3d_3c;
        cinfo.position_00[1] = reference.position_2c[1] * HAVOK_SCALE;
        cinfo.field_78 = 0;
        cinfo.position_00[2] = HAVOK_SCALE * reference.position_2c[2];
        cinfo.jump_scale_7c = inputs.jump_scale;
        // 0065B889..0065B8BF the base form's bound (vt+0x170 GetBaseForm, null here): +0x85 = 1.
        if reference.base_form_1c.is_some() {
            return Err(StreamError(
                "0065B8A4: a base-form bound for the controller is not reached",
            ));
        }
        cinfo.no_base_bound_85 = 1;
        // 0065B8C7..0065B8FC j_MemoryHeap_Allocate(0x3E0, 1), 16-aligned, 00897230(cinfo).
        globals(world).last_cinfo = Some(cinfo);
        let controller =
            construct_character_controller(world, &mut cinfo, inputs.controller_alignment)?;
        // 0065B901..0065B91E the NiPointer argument takes a reference, 0065B924..0065B92F process vt+0x190 00634910: +0x118 = controller (a reference), then the argument's reference is released.
        world
            .get_mut::<MiddleHighProcess>(process)
            .ok_or(StreamError("0065B924: vt+0x190 on a low process"))?
            .char_controller_118 = Some(controller);
        let mut record = world
            .get_mut::<BhkCharacterController>(controller)
            .expect("just spawned");
        record.reference_count_04 += 1;
        // 0065B931..0065B939 008910F0(1000, actor): with a +0x364 phantom whose hk object lacks property 1000 (00890A10), 008BC750 adds it (substituted: the key is recorded).
        let phantom = record.phantom_364;
        // 0065B93E..0065B952 +0x334 = vt+0xEC (00611D00, stored float).
        record.floats_320[5] = inputs.scale;
        // 0065B958..0065B96D flag 0x100000 at +0x1F4 from 00B333B8.
        let flag = world
            .resource::<CharacterControllerGlobals>()
            .player_flag_b333b8;
        let mut record = world
            .get_mut::<BhkCharacterController>(controller)
            .expect("just spawned");
        if flag != 0 {
            record.flags_1f4 |= 0x0010_0000;
        } else {
            record.flags_1f4 &= 0xFFEF_FFFF;
        }
        if let Some(phantom) = phantom {
            world
                .get_mut::<BhkWorldObjectRecord>(phantom)
                .expect("a NiPointer target")
                .property_keys
                .push(1000);
        }
        // 0065B977..0065B9A9 process vt+0x36C (0064B090: +0x11D) nonzero: vt+0x1EC(0.1) and flag 0x800 (not reached: 0).
        if world
            .get::<MiddleHighProcess>(process)
            .expect("checked above")
            .byte_11d
            != 0
        {
            return Err(StreamError(
                "0065B988: the vt+0x36C controller arm is not reached",
            ));
        }
        // 0065B9B3..0065B9C2 00890F70: the cinfo releases its state manager and +0x8C.
        let mut globals = globals(world);
        globals.last_cinfo_released = Some(cinfo);
        if cinfo.state_manager_90 {
            globals.state_manager_reference_count -= 1;
        }
    }
    // 0065B9C7..0065B9E5 the +0x364 phantom's world (0089F6B0, substituted: no world) against the actor's +0x3C: equal (both null) skips the world change.
    // 0065BA52..0065BA5E this vt+0x178 (0065B5C0)(0): 0065A2C0, vt+0x190, 006ECC80 (null cell: no world), the controller's +0x08 world (substituted: none): equal, the +0x364 world (none) against +0x3C (null): equal; no
    // cell: return. Read-only on this path.
    Ok(())
}

// IDA: 0065A310 Actor_SetCharControllerEnabled(argument) (vt+0x1BC): with a controller, tail call 008927E0(argument == 0) on it: when the +0x368 object's +0x68 byte equals the requested state nothing happens; enabling
// runs its vt+0x60 and 0088D560(1) (+0x68 = 1; the hk entity's vt+0x3C, substituted); disabling 0088D560(0) and the world re-add (substituted). Completion: 100%.
pub fn actor_set_character_controller_enabled(world: &mut World, actor: Entity, enabled: bool) {
    // 0065A310 0065A2C0; none: return.
    let Some(controller) = actor_character_controller(world, actor) else {
        return;
    };
    // 008927E0 on the controller with (argument == 0).
    let Some(object) = world
        .get::<BhkCharacterController>(controller)
        .expect("+0x118 is a controller")
        .proxy_object_368
    else {
        return;
    };
    let mut record = world
        .get_mut::<BhkWorldObjectRecord>(object)
        .expect("a NiPointer target");
    let byte_before = record.enabled_68;
    let disable = !enabled;
    // 008927ED..008927FA nothing to do when (+0x68 == 0) equals the request.
    if disable != (record.enabled_68 == 0) {
        // 00892800..00892846 0088D560(0) and vt+0x5C with the world (substituted), or 0089284B..0089285A vt+0x60 (substituted) then 0088D560(1): +0x68 = 1; the hk entity (+0x08) vt+0x3C (substituted).
        record.enabled_68 = u8::from(!disable);
    }
    let byte_after = record.enabled_68;
    globals(world).enable_calls.push(ControllerEnableCall {
        argument: enabled,
        byte_before,
        byte_after,
    });
}

// IDA: 0089F4D0 bhkWorldObject_SetCollisionLayer(layer): the hk object's filter info = (layer << 16) | its low word, then vt+0x80 (the world update, substituted). Completion: 100%.
pub fn set_collision_layer(world: &mut World, object: Entity, layer: u32) {
    let mut record = world
        .get_mut::<BhkWorldObjectRecord>(object)
        .expect("a bhkWorldObject");
    // 0089F4D4..0089F503.
    let before = record.collision_filter_info;
    record.collision_filter_info = collision_filter(layer, before & 0xFFFF);
    let after = record.collision_filter_info;
    // 0089F506 vt+0x80 (world update, substituted).
    globals(world).layer_calls.push((before, after));
}

// IDA: 00532BC0 (FormHeap 8, then 00532370). Completion: 100% of the Bethesda-side stores.
pub fn construct_camera_collision_helper(world: &mut World, radius: f32, layer: u32) -> Entity {
    // 00532BE8..00532BF0 +0x00 = +0x04 = 0; 00532C05 00532370(radius, layer). 005323A8..005323D5 FormHeap(0x14) bhkSphereShape(radius, 1).
    let shape = construct_sphere_shape(world, radius, true);
    // 005323F9..00532434 the two cinfos: (layer << 16) | 0x18 and (counter << 16) | 0x18.
    let first = collision_filter(layer, 0x18);
    let group = next_collision_group(&mut globals(world));
    let second = collision_filter(group, 0x18);
    // 005324C6..00532527 phantom 0 -> +0x00 (NiPointer store).
    let phantom_0 = construct_simple_shape_phantom(world, first, Some(shape));
    let mut slot_0 = None;
    assign_pointer(world, &mut slot_0, phantom_0);
    // 0053259A..00532606 phantom 1 -> +0x04.
    let phantom_1 = construct_simple_shape_phantom(world, second, Some(shape));
    let mut slot_1 = None;
    assign_pointer(world, &mut slot_1, phantom_1);
    world
        .spawn(CameraCollisionHelper {
            phantom_00: phantom_0,
            phantom_04: phantom_1,
            shape,
        })
        .id()
}

// IDA: 005358F0 on the 16-aligned 0x1C0 block (0066AACB). Completion: 100% of the Bethesda-side stores.
pub fn construct_collision_query_helper(
    world: &mut World,
    radius: f32,
    layer: u32,
    alignment: u8,
) -> Entity {
    // 00535957 00535730(radius, layer): bhkSphereShape(radius, 1), +0x1A8 = layer, a phantom with (layer << 16) | 0x1C -> +0x1A0 (NiPointer), +0x1A4 byte = 1.
    let shape = construct_sphere_shape(world, radius, true);
    let phantom = construct_simple_shape_phantom(world, collision_filter(layer, 0x1C), Some(shape));
    let mut slot = None;
    assign_pointer(world, &mut slot, phantom);
    world
        .spawn(CollisionQueryHelper {
            alignment_m1: alignment,
            // 00535918..0053593D hkAllCdPointCollector: vtable, +0x10 = +0x20, +0x18 = 0x80000008, +0x04 = FLT_MAX, +0x14 = 0; +0x1A0 = 0.
            original_vtable: HK_ALL_CD_POINT_COLLECTOR_VTABLE,
            inline_storage_10: true,
            capacity_18: 0x8000_0008,
            early_out_distance_04: f32::from_bits(0x7F7F_FFFF),
            size_14: 0,
            phantom_1a0: phantom,
            field_1a4: 1,
            layer_1a8: layer,
            shape,
        })
        .id()
}
