//! Phase 11 (D3): the player's Havok character controller and the two collision helpers that PlayerCharacter_ctor 0066A740 creates. 0065B750 Actor_CreateCharacterController runs when the process exists, its
//! level is 0 or 1 and the +0x40 getter (006ECC80, identical-code folded with TESClass_GetSpecialization: here TESObjectREFR +0x40 parentCell) is null or a cell whose +0x26 byte is 5 or 6 (P11-RUN-001: High
//! process, null cell: created). It fills a cinfo (00890C00 over 00890B00, with the 00B2EB3C collision-group counter and the once-built character state manager 00BA7A54), allocates a 16-aligned 0x3E0 block for
//! bhkCharacterController 00897230, stores it into the process (vt+0x190 00634910, +0x118), sets property 1000 = actor (008910F0), +0x334 = vt+0xEC, the +0x1F4 flag bits, and frees the cinfo (00890F70); then
//! vt+0x178 (0065B5C0). The window then sets +0x3B0 = 1000, enables the proxy object (0065A310 -> 008927E0 -> 0088D560), puts the +0x364 phantom and the +0x368 proxy object in system group 9 (0089F4D0) and builds the
//! camera collision helper (00532BC0(8.0, 9)) and the collision query helper (005358F0(2.0, 9) in a 16-aligned 0x1C0 block).
//!
//! bhk objects are NiObjects, so each is an entity with its record as a component and its own +0x04 count; NiPointer stores keep that count. Recorded substitution (Phase 09 D2): the Havok SDK objects underneath
//! (the hkCharacterProxy, the hkCharacterContext / state manager states, the shapes, the hkSimpleShapePhantoms, the point collectors and their world membership and properties) are not created. The records keep
//! the values Bethesda code passes to them (filter infos, radii, layers, property keys), and the queries the window makes are answered as the original world-less objects do (P11-RUN-001: no world). No physics
//! engine runs. Evidence: evidence/phase11-window-2026-10-06/disasm-reached.txt, disasm-bhk.txt, decompile-895190.c; P11-RUN-001.

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;

use super::actor::{MobileObject, TesObjectRefr};
use super::actor_process::{MiddleHighProcess, process_level};
use crate::bs::character_controller::{
    BhkCharacterController, BhkWorldObjectRecord, CharacterControllerGlobals, ControllerEnableCall,
    HAVOK_SCALE, assign_pointer, bind_phantom_3d, collision_filter, construct_character_controller,
    construct_controller_cinfo, construct_simple_shape_phantom, construct_sphere_shape,
    controller_phantom_filter, controller_set_position, controller_set_world, globals,
    next_collision_group,
};
use crate::tes::forms_references::set_reference_position;

pub const HK_ALL_CD_POINT_COLLECTOR_VTABLE: u32 = 0x00A5_5DEC;

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
) -> Result<(), GameError> {
    set_reference_position(world, actor, position)?;
    let Some(controller) = actor_character_controller(world, actor) else {
        return Ok(());
    };
    controller_set_position(world, controller, position);
    let mut body = world
        .get_mut::<BhkCharacterController>(controller)
        .expect("the process's controller");
    // 0065A419 +0x08: Phase 11's 00895190 always made the proxy, so `proxy_08` isn't a host field.
    body.host_world_mark_calls += 1;
    body.floats_320[0] = 0.0;
    if (body.flags_1f4 >> 7) & 1 != 0 {
        body.flags_1f4 &= 0xFFFF_FF7F;
    }
    Ok(())
}

// IDA: 0065B750 Actor_CreateCharacterController. Completion: 100% of the reached arms; unreached arms (a living-dead actor's queries, a world change) stop.
// TODO: [ ] Sleep-state collision resize and world teardown.
pub fn actor_create_character_controller(
    world: &mut World,
    actor: Entity,
    inputs: ControllerInputs,
) -> Result<(), GameError> {
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
    // 0065B7BD..0065B7E0 006ECC80 (+0x40 parentCell): null, or a cell with +0x26 == 6 / 5; otherwise nothing.
    if let Some(cell) = reference.parent_cell_40 {
        let level = world
            .get::<crate::tes::forms_world::TesObjectCell>(cell)
            .expect("a cell")
            .process_level_26;
        if level != 6 && level != 5 {
            return Ok(());
        }
    }
    // 0065B7E6..0065B828 the Actor cast; dead = 005E0DC0 Actor_GetDeathState (+0xB0) == 2 or 005F0310(-1).
    let dead = world
        .get::<super::actor::Actor>(actor)
        .is_some_and(|data| data.dead_state_b0 == 2)
        || actor_is_ragdolled(world, actor)?;
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
        cinfo.jump_scale_7c = if reference.base_form_1c.is_some() {
            super::mobile_move::actor_get_jump_scale(world, actor)?
        } else {
            inputs.jump_scale
        };
        if let Some(creature) = reference
            .base_form_1c
            .and_then(|base| world.get::<crate::tes::forms_actors::TesCreature>(base))
        {
            cinfo.float_98 = creature.scale_114 * reference.scale_38;
        } else {
            cinfo.no_base_bound_85 = 1;
        }
        globals(world).last_cinfo = Some(cinfo);
        let node = reference_3d(world, actor);
        let controller =
            construct_character_controller(world, &mut cinfo, inputs.controller_alignment, node)?;
        // 0065B901..0065B91E the NiPointer argument takes a reference, 0065B924..0065B92F process vt+0x190 00634910: +0x118 = controller (a reference), then the argument's reference is released.
        world
            .get_mut::<MiddleHighProcess>(process)
            .ok_or(GameError::unclassified(
                0x0065B924,
                "0065B924: vt+0x190 on a low process",
            ))?
            .char_controller_118 = Some(controller);
        let mut record = world
            .get_mut::<BhkCharacterController>(controller)
            .expect("just spawned");
        record.reference_count_04 += 1;
        // 0065B931..0065B939 008910F0(1000, actor): with a +0x364 phantom whose hk object lacks property 1000 (00890A10), 008BC750 adds it (substituted: the key is recorded).
        let phantom = record.phantom_364;
        if let Some(phantom) = phantom {
            world
                .get_mut::<BhkWorldObjectRecord>(phantom)
                .expect("a NiPointer target")
                .property_keys
                .push(1000);
        }
        // 0065B93E..0065B952 +0x334 = vt+0xEC, after storing the phantom property.
        let scale = if reference.base_form_1c.is_some() {
            super::actor_animation::actor_scale(world, actor)?
        } else {
            inputs.scale
        };
        world
            .get_mut::<BhkCharacterController>(controller)
            .expect("just spawned")
            .floats_320[5] = scale;
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
        // 0065B977..0065B9A9 process vt+0x36C (0064B090: +0x11D) nonzero: vt+0x1EC(0.1) and flag 0x800 (not reached: 0).
        if world
            .get::<MiddleHighProcess>(process)
            .expect("checked above")
            .sleep_state_11d
            != 0
        {
            return Err(GameError::unclassified(
                0x0065B988,
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
    let controller =
        actor_character_controller(world, actor).expect("made above or held by the process");
    // 0065B9C7..0065B9E5 0089F6B0: the +0x364 phantom's rtti_1-keyed property (the NiAVObject 0089F650 bound) against the
    // actor's +0x3C.
    let node = reference_3d(world, actor);
    let phantom = world
        .get::<BhkCharacterController>(controller)
        .expect("+0x118 is a controller")
        .phantom_364;
    let bound = phantom.and_then(|phantom| {
        world
            .get::<BhkWorldObjectRecord>(phantom)
            .and_then(|record| record.property_3d)
    });
    if bound != node {
        // 0065B9F4 0089F650(+0x3C, rtti_1): the old property removed (008BC7B0), the new added (008BC750).
        if let Some(phantom) = phantom {
            bind_phantom_3d(world, phantom, node);
        }
        // 0065B9FB..0065BA36 the cell's world: its +0x1A set runs the controller's vt+0x88(0). Only vt+0x98 (0088D2B0) writes
        // +0x1A, so it stays 0 here.
        if let Some(cell) = world
            .get::<TesObjectRefr>(actor)
            .expect("a reference")
            .parent_cell_40
            && let Some(bhk) = crate::tes::cell_havok::cell_bhk_world(world, cell)
            && world
                .get::<crate::bs::havok_world::BhkWorld>(bhk)
                .is_some_and(|data| data.byte_1a != 0)
        {
            return Err(GameError::unclassified(
                0x0065BA36,
                "0065BA36: the controller's vt+0x88(0)",
            ));
        }
        // 0065BA4A 0088D070(+0x3C, 6, 1, 0) unless dead.
        if !dead {
            crate::bs::havok_world::set_motion_type(world, node, 6, true, false)?;
        }
    }
    // 0065BA5E vt+0x178(0): 0065B5C0.
    actor_sync_char_controller_world(world, actor)
}

/// IDA: `005F0310` Actor_IsRagdolled(flags) with -1: no +0x3C answers false; flag 0x40 asks +0xB0 (1, 2 or 6 true; 4 walks the
/// active effects for a ReanimateEffect, a stop); then the process's vt+0x2E4 (`0064B080`, MiddleHigh +0x11C, the knocked
/// state; a Low process 0) in 1..5. Completion: 100% but the reanimate walk.
/// # Errors
/// A death state of 4.
pub fn actor_is_ragdolled(world: &World, actor: Entity) -> Result<bool, GameError> {
    if reference_3d(world, actor).is_none() {
        return Ok(false);
    }
    let state = world
        .get::<super::actor::Actor>(actor)
        .map_or(0, |data| data.dead_state_b0);
    if matches!(state, 1 | 2 | 6) {
        return Ok(true);
    }
    if state == 4 {
        return Err(GameError::unclassified(
            0x005F0348,
            "005F0348: 005F0310's ReanimateEffect walk",
        ));
    }
    let knocked = world
        .get::<MobileObject>(actor)
        .and_then(|mobile| mobile.process_58)
        .and_then(|process| world.get::<MiddleHighProcess>(process))
        .map_or(0, |middle| middle.knocked_state_11c as i8);
    Ok((1..=5).contains(&knocked))
}

// IDA: N/A the actor's +0x3C as an entity (the loaded 3D's node).
fn reference_3d(world: &World, actor: Entity) -> Option<Entity> {
    world
        .get::<crate::tes::reference_3d::Reference3D>(actor)
        .map(|loaded| loaded.0)
}

/// IDA: `0065B5C0` Actor_SyncCharControllerWorld (vt+0x178): with a controller (`0065A2C0`), dead = an actor whose +0xB0 is
/// 2; the parent cell's world (an interior's ExtraHavok, an exterior's 00B35C24; none without a cell) against the proxy's
/// (`008AC0C0`'s +0x08 +0x2B0): a change sets +0x2A0 = 0, +0x318 = the cell's water height (`004CACE0`, 0.0 with no cell)
/// times 0.1428767 (`fmul` the qword on the stored float), unless dead `00895060`(world), then `00452A10`(+0x2C). Then the
/// phantom's rtti_1 property against +0x3C (`0089F650` on a change). A change with an old world runs `0088CD50`(+0x3C, 1, 0)
/// (a stop: the old world's removal); with a new world a change adds +0x3C's collision with the phantom's system group
/// (`0057E270` >> 16) through world vt+0x90 (`0088CC80`(+0x3C, 1, 0, group, 0)); no change runs `0088CDC0`(+0x3C, 1, 0).
/// Completion: 100% but the old world's removal and an exterior world.
/// # Errors
/// A stop above, or under `0088CC80`.
pub fn actor_sync_char_controller_world(world: &mut World, actor: Entity) -> Result<(), GameError> {
    let Some(controller) = actor_character_controller(world, actor) else {
        return Ok(());
    };
    let dead = world
        .get::<super::actor::Actor>(actor)
        .is_some_and(|data| data.dead_state_b0 == 2);
    let cell = world
        .get::<TesObjectRefr>(actor)
        .expect("a reference")
        .parent_cell_40;
    let bhk = match cell {
        Some(cell) if crate::tes::forms_world::cell_is_interior(world, cell) => {
            crate::tes::cell_havok::cell_bhk_world(world, cell)
        }
        Some(_) => {
            return Err(GameError::unclassified(
                0x0065B62A,
                "0065B62A: an exterior world for the controller (00B35C24 has no bhkWorld record)",
            ));
        }
        None => None,
    };
    let current = world
        .get::<BhkCharacterController>(controller)
        .expect("+0x118 is a controller")
        .host_world;
    if bhk != current {
        let water = cell.map_or(0.0, |cell| {
            crate::tes::tes_grid::cell_water_height(world, cell)
        });
        let water_bits = (f64::from(water) * 0.142_876_729_369_163_5) as f32;
        {
            let mut record = world
                .get_mut::<BhkCharacterController>(controller)
                .expect("a controller");
            record.field_2a0 = 0;
            record.water_height_318 = water_bits;
        }
        if !dead {
            controller_set_world(world, controller, bhk);
        }
        let position = world
            .get::<TesObjectRefr>(actor)
            .expect("a reference")
            .position_2c;
        controller_set_position(world, controller, position);
    }
    let node = reference_3d(world, actor);
    let phantom = world
        .get::<BhkCharacterController>(controller)
        .expect("a controller")
        .phantom_364;
    let bound = phantom.and_then(|phantom| {
        world
            .get::<BhkWorldObjectRecord>(phantom)
            .and_then(|record| record.property_3d)
    });
    if bound != node
        && let Some(phantom) = phantom
    {
        bind_phantom_3d(world, phantom, node);
    }
    let mut changed = false;
    if current != bhk {
        if current.is_some() {
            return Err(GameError::unclassified(
                0x0065B6F5,
                "0065B6F5: 0088CD50, the old world's removal",
            ));
        }
        changed = true;
    }
    if let Some(bhk) = bhk {
        if changed {
            let group = controller_phantom_filter(world, controller) >> 16;
            crate::bs::havok_world::bhk_world_add_objects(
                world, bhk, node, true, false, group, false,
            )?;
        } else {
            crate::bs::havok_world::mark_collision_for_sync(world, node, true, false)?;
        }
    }
    Ok(())
}

/// IDA: N/A the seam's move for the controller's vt+0x80 integrate (under `0065AF30` at `0065B44B`): the caller ports
/// `0065AF30` and the controller's state logic around it (goal 2); this moves the proxy through its world.
/// # Errors
/// No physics backend.
pub fn controller_move(
    world: &mut World,
    controller: Entity,
    movement: &crate::bs::physics_seam::CharacterMove,
) -> Result<crate::bs::physics_seam::CharacterMoveResult, GameError> {
    let physics = world
        .get::<BhkCharacterController>(controller)
        .expect("a controller")
        .host_physics;
    crate::bs::physics_seam::seam_move_character(world, physics, movement)
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
        .avoid_box_368
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

// IDA: 0089F4D0 bhkWorldObject_SetSystemGroup(group): the hk object's filter info = (group << 16) | its low word, then vt+0x80 (the world update, substituted). Completion: 100%.
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

/// IDA: 00531F10 CameraCollisionHelper_GetWorld: phantom +0x00's vt+0x58 (its hk object's world) and that world's +0x2B0
/// (the bhkWorld); no phantom or no world answers null. Completion: 100%.
#[must_use]
pub fn camera_collision_helper_world(world: &World, helper: Entity) -> Option<Entity> {
    let phantom = world.get::<CameraCollisionHelper>(helper)?.phantom_00;
    world
        .get::<crate::bs::havok_world::BhkInWorld>(phantom)
        .map(|held| held.world)
}

/// IDA: 00531E90 CameraCollisionHelper_SetWorld(world): each phantom (+0x00, then +0x04) takes vt+0x5C(world), and with a
/// world 0088BB60 (the visual debugger's colour, hkColor::rgbFromChars(255, 46, 46, 25)), counted. **Host substitution:**
/// the phantoms join the bhkWorld's membership without a body in the seam; the helper's casts go through the seam's ray
/// (`camera_collision_helper_cast`), and nothing else queries them. Completion: 100% of the Bethesda side.
pub fn camera_collision_helper_set_world(
    world: &mut World,
    helper: Entity,
    bhk_world: Option<Entity>,
) -> CameraCollisionWorldCalls {
    let Some(&CameraCollisionHelper {
        phantom_00,
        phantom_04,
        ..
    }) = world.get::<CameraCollisionHelper>(helper)
    else {
        return CameraCollisionWorldCalls::default();
    };
    let mut calls = CameraCollisionWorldCalls::default();
    for phantom in [phantom_00, phantom_04] {
        match bhk_world {
            Some(target) => {
                world
                    .entity_mut(phantom)
                    .insert(crate::bs::havok_world::BhkInWorld {
                        world: target,
                        physics: None,
                    });
                calls.debug_colours_88bb60 += 1;
            }
            None => {
                world
                    .entity_mut(phantom)
                    .remove::<crate::bs::havok_world::BhkInWorld>();
            }
        }
    }
    calls
}

/// IDA: N/A the counted calls of 00531E90.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CameraCollisionWorldCalls {
    pub debug_colours_88bb60: u32,
}

/// IDA: 005326B0 CameraCollisionHelper_LinearCast(this, from, to, out_point): with phantom +0x00, an hkAllCdPointCollector
/// (early out FLT_MAX, inline storage of 8) and the cast input (to x 0.1428767293691635, maxExtraPenetration and
/// startPointTolerance 1.19209e-7); from x 0.1428767293691635 likewise; the hk object's vt+0x0C setPositionAndLinearCast
/// (from, input, collector, 0) between the world's lock calls; with hits, 008AF890 sorts them and the first whose
/// collidable's layer (filter & 0x3F) isn't 0x14 (or one whose owner isn't a mobile object with a controller flagged
/// 0x800) is copied (00532210) and its contact position written to `to` in game units (0043F3E0, x 6.999040126800537).
/// Answers whether the collector held any hit. **Host substitution:** the 8.0 sphere's linear cast is the seam's ray from
/// `from` to `to` (`cast_ray`, the phantom's system group left out), its hit point the contact position; the group
/// filter's layer pairs aren't applied. A layer 0x14 hit (an actor's body) is a stop. Completion: 90% (the substitution).
/// # Errors
/// A layer 0x14 hit.
pub fn camera_collision_helper_cast(
    world: &mut World,
    helper: Entity,
    from: [f32; 3],
    to: &mut [f32; 3],
) -> Result<bool, GameError> {
    let Some(phantom) = world
        .get::<CameraCollisionHelper>(helper)
        .map(|helper| helper.phantom_00)
    else {
        return Ok(false);
    };
    let Some(record) = world.get::<BhkWorldObjectRecord>(phantom) else {
        return Ok(false);
    };
    if !record.havok_object_08 {
        return Ok(false);
    }
    let group = record.collision_filter_info >> 16;
    let scale = |value: f32| (f64::from(value) * 0.142_876_729_369_163_5) as f32;
    let start = from.map(scale);
    let end = to.map(scale);
    let Some(physics) = world
        .get::<crate::bs::havok_world::BhkInWorld>(phantom)
        .and_then(|held| world.get::<crate::bs::havok_world::BhkWorld>(held.world))
        .and_then(|bhk| bhk.physics)
    else {
        return Ok(false);
    };
    let delta = [end[0] - start[0], end[1] - start[1], end[2] - start[2]];
    let length = (delta[0] * delta[0] + delta[1] * delta[1] + delta[2] * delta[2]).sqrt();
    if length <= 0.0 {
        return Ok(false);
    }
    let direction = delta.map(|value| value / length);
    let hits = crate::bs::physics_seam::seam_cast_ray(
        world,
        Some(physics),
        start,
        direction,
        length,
        group,
        8,
    );
    let Some(&(distance, body)) = hits.first() else {
        return Ok(false);
    };
    let mut bodies = world.query::<(Entity, &crate::bs::havok_world::BhkInWorld)>();
    let entity = bodies
        .iter(world)
        .find(|(_, held)| held.physics == Some(body))
        .map(|(entity, _)| entity);
    if entity.is_some_and(|entity| {
        crate::bs::havok_world::body_filter_info(world, entity) & 0x3F == 0x14
    }) {
        return Err(GameError::unclassified(
            0x005326F7,
            "005326F7: a layer 0x14 hit's owner and controller test",
        ));
    }
    let point = [0, 1, 2].map(|axis| start[axis] + direction[axis] * distance);
    *to = crate::bs::havok_world::to_game_units(point);
    Ok(true)
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
