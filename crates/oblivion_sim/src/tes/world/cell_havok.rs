//! Build 3 (goal 1): a cell's Havok world and the references entering it. `004D4A20` builds an interior's bhkWorld (through
//! the physics seam, `bs::physics_seam`) and its CellMopp; `004CB7F0` the references' centroid; `004D4970` steps it;
//! `004D4D00` re-centres a world's mopp bodies; TESObjectREFR's vt+0x148 (`004E1FD0`) puts a loaded reference's collision
//! into it, as `004DC100` calls it. The hk objects are the seam's (a recorded host substitution, the plan's Havok section).

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;

use crate::bs::character_controller::HAVOK_SCALE;
use crate::bs::havok::{HavokFrameGlobals, HavokGlobals};
use crate::bs::havok_world::{
    BHK_WORLD_VTABLE, BhkWorld, CELL_MOPP_VTABLE, CellMopp, HavokWorldCounters, WorldListener,
    bhk_world_update,
};
use crate::bs::physics_seam::WorldDesc;
use crate::tes::base_extra_list::{EXTRA_HAVOK, ExtraData, cell_mopp, set_extra_havok};
use crate::tes::forms_world::{TesObjectCell, cell_is_interior};
use crate::tes::reference_3d::reference_add_to_havok_world;

/// IDA: 00B3B800/00B3B804, the embedded list head and next link. BSDoorHavokController_Update
/// prepends the reference when the Open and Close sequences have both stopped.
#[derive(Resource, Default)]
pub struct DoorHavokAddQueue(pub std::collections::VecDeque<Entity>);

// IDA: 0060DEA0 BSDoorHavokController_DrainHavokAddQueue.
// Completion: dispatch vt+0x148 on the head, then remove the current head. A stopped virtual leaves it queued.
pub fn drain_door_havok_add_queue(world: &mut World) -> Result<(), GameError> {
    while let Some(reference) = world
        .get_resource::<DoorHavokAddQueue>()
        .and_then(|queue| queue.0.front().copied())
    {
        if world.get::<crate::tes::actor::Actor>(reference).is_some() {
            crate::tes::character_controller::actor_create_character_controller(
                world,
                reference,
                Default::default(),
            )?;
        } else {
            reference_add_to_havok_world(world, reference)?;
        }
        world.resource_mut::<DoorHavokAddQueue>().0.pop_front();
    }
    Ok(())
}

/// IDA: 00889810 bhk_AccumulateFrameTime(frameTime, a2), 0040D8C2 with g_timeInfo +0x0C and 00B333B8: 00B2E2E4 = the time;
/// t = time + 00BA7910 into 00B2E2E0, clamped at 166.66667 (A3D14C); t <= 0 (not NaN) leaves no substep and a zero substep
/// time. The cap is 3 - (a2 != 0). iUpdateType (00BA7918) 0: 00BA790C = 00B2E2E8 (the fixed step, min(fMaxTime, 1.0));
/// n = trunc(t / step) capped; n > 0: 00BA7910 = t - n x step, 00B2E2E0 = t - that, and a remainder of a step or more is
/// cut to a step; n = 0: half a step or more runs one substep of t, less carries t with 00B2E2E0 and 00BA790C 0. Type 10:
/// 00B2E2EC eases toward t by fChaseDeltaMult (00B2E2F0) and is the one substep, 00BA7910 = 0. Any other type: below
/// 0.008333334 (A95CA8) it carries t and steps nothing; else 00BA7910 = 0, n = trunc(t / step) capped, its substep t / n, or
/// one substep of t when n is 0. Precision: 24-bit (each x87 result rounds as an f32 operation does), `fistp` truncating
/// under the 0x0C00 control word. Completion: 100%.
pub fn bhk_accumulate_frame_time(world: &mut World, frame_time: f32, a2: bool) {
    let step = world
        .get_resource::<HavokGlobals>()
        .map_or(1.0 / 60.0, |globals| globals.max_time_b2e2e8);
    let (update_type, chase_mult) =
        world
            .get_resource::<super::tes::TesGlobals>()
            .map_or((0, 0.05), |globals| {
                (
                    globals.havok_copies.update_type_ba7918,
                    globals.havok_copies.chase_delta_mult_b2e2f0,
                )
            });
    let mut g = world.get_resource_or_init::<HavokFrameGlobals>();
    g.frame_time_b2e2e4 = frame_time;
    let mut t = frame_time + g.remainder_ba7910;
    g.stepped_b2e2e0 = t;
    // A3D14C, 166.66667.
    let limit = f32::from_bits(0x4326_AAAB);
    if t > limit {
        t = limit;
        g.stepped_b2e2e0 = limit;
    } else if t <= 0.0 {
        g.substeps_ba7914 = 0;
        g.substep_time_ba790c = 0.0;
        return;
    }
    let most = 3 - u32::from(a2);
    let truncate = |value: f32| -> u32 { value.trunc() as i64 as u32 };
    match update_type {
        0 => {
            g.substep_time_ba790c = step;
            let mut n = truncate(t / step);
            if n > most {
                n = most;
            }
            g.substeps_ba7914 = n;
            if n != 0 {
                let remainder = t - n as f32 * step;
                g.remainder_ba7910 = remainder;
                g.stepped_b2e2e0 = t - remainder;
                if remainder >= step || remainder.is_nan() {
                    g.remainder_ba7910 = step;
                }
            } else if step * 0.5 <= t || t.is_nan() {
                g.substep_time_ba790c = t;
                g.substeps_ba7914 = 1;
                g.remainder_ba7910 = 0.0;
            } else {
                g.remainder_ba7910 = t;
                g.stepped_b2e2e0 = 0.0;
                g.substep_time_ba790c = 0.0;
            }
        }
        10 => {
            g.chase_b2e2ec = (t - g.chase_b2e2ec) * chase_mult + g.chase_b2e2ec;
            g.substep_time_ba790c = g.chase_b2e2ec;
            g.substeps_ba7914 = 1;
            g.remainder_ba7910 = 0.0;
        }
        _ => {
            if f32::from_bits(0x3C08_8889) > t {
                g.remainder_ba7910 = t;
                g.substeps_ba7914 = 0;
                g.stepped_b2e2e0 = 0.0;
                g.substep_time_ba790c = 0.0;
                return;
            }
            g.remainder_ba7910 = 0.0;
            let mut n = truncate(t / step);
            if n > most {
                n = most;
            }
            g.substeps_ba7914 = n;
            if n != 0 {
                g.substep_time_ba790c = t / n as f32;
            } else {
                g.substep_time_ba790c = t;
                g.substeps_ba7914 = 1;
            }
        }
    }
}

/// IDA: `BaseExtraList_GetHavokWorld` on an interior (the type 2 extra's +0x0C); an exterior's 00B35C24 has no entity in the
/// port (the D2 world of `004D5600`), so it answers `None`. Completion: 100% of an interior.
#[must_use]
pub fn cell_bhk_world(world: &World, cell: Entity) -> Option<Entity> {
    let data = world.get::<TesObjectCell>(cell)?;
    if data.flags_24 & 1 == 0 {
        return None;
    }
    match data.extra_28.get(EXTRA_HAVOK).map(|entry| &entry.data) {
        Some(ExtraData::Havok { world_0c, .. }) => *world_0c,
        _ => None,
    }
}

/// IDA: `004CB7F0` TESObjectCELL_GetReferenceCentroid(out): zero, then under the region lock each nonnull +0x48 reference's
/// vt+0x174 position added in list order, a float at each store; with any, `1.0 / count` as a float and three products.
/// Completion: 100%.
/// # Errors
/// A stop in a position read.
pub fn cell_reference_centroid(world: &World, cell: Entity) -> Result<[f32; 3], GameError> {
    let mut sum = [0.0_f32; 3];
    let mut count = 0_i32;
    let references: Vec<Entity> = world
        .get::<TesObjectCell>(cell)
        .expect("a cell")
        .objects_48
        .items()
        .collect();
    for reference in references {
        let position = crate::tes::forms_references::reference_position(world, reference)?;
        count += 1;
        sum = [
            sum[0] + position[0],
            position[1] + sum[1],
            position[2] + sum[2],
        ];
    }
    if count > 0 {
        let inverse = 1.0_f32 / count as f32;
        sum = [sum[0] * inverse, sum[1] * inverse, inverse * sum[2]];
    }
    Ok(sum)
}

/// IDA: `004D4A20` TESObjectCELL_CreateInteriorHavokWorld, from `004D58B0`. An interior whose ExtraHavok has no world:
/// `0088A4F0`'s bhkWorldCinfo (gravity (0, 0, -73.575, 0)), `008A9460(3000.0)` the broadphase, 00B2E2D4 = iNumHavokThreads
/// (00B097D8), a 16-aligned 0x90 FormHeap bhkWorld (`0088AEB0`: 00BA7900 counted), its vt+0x58; `00536E10`'s TESTrapListener
/// through `004CD320` (+0x60 and +0x24), with the cell's water flag a TESWaterListener (`00537E40` of `004CACE0`'s height)
/// and a TESArrowTargetListener (`005360F0`) through `004CD2D0` (+0x60); `004CB7F0`'s centroid into `0088D260` (times the
/// Havok scale, the broadphase centre); `0088B680(bHavokDebug)` (0: +0x14 released and cleared); 00B2E2E8 = fMaxTime
/// (00B097C0) when below 1.0 (`fcom`, NaN takes 1.0), else 1.0; `004240C0` stores it in the ExtraHavok; vt+0x58; vt+0x80
/// (`0088C440`, the step); world +0x19 = (00B33A34 == 0). Then any cell whose ExtraHavok has no CellMopp (`0041F950`) gets
/// one (`00533380`, `0041F890`). The hkpWorld is the seam's world. Completion: 100% of the Bethesda-side stores.
/// # Errors
/// A stop in the centroid's reads or the step.
pub fn create_interior_havok_world(world: &mut World, cell: Entity) -> Result<(), GameError> {
    let interior = cell_is_interior(world, cell);
    if interior && cell_bhk_world(world, cell).is_none() {
        let ini = |world: &World, address: u32| {
            world
                .get_resource::<crate::tes::settings::SettingCollections>()
                .and_then(|collections| {
                    collections
                        .ini
                        .find_address(address)
                        .map(|setting| setting.raw)
                })
        };
        let threads = ini(world, 0x00B0_97D8).unwrap_or(1);
        let max_time = f32::from_bits(ini(world, 0x00B0_97C0).unwrap_or(0x3C88_8889));
        let debug = ini(world, 0x00B0_97B8).unwrap_or(0) & 0xFF;
        // 004D4AEA 00B2E2D4 = iNumHavokThreads.
        world
            .get_resource_or_init::<HavokGlobals>()
            .num_havok_threads_b2e2d4 = threads;
        // 004D4B18 0088AEB0.
        world
            .get_resource_or_init::<HavokWorldCounters>()
            .worlds_ba7900 += 1;
        let mut record = BhkWorld {
            original_vtable: BHK_WORLD_VTABLE,
            reference_count_04: 0,
            visual_debugger_14: false,
            byte_18: 0,
            byte_19: 0,
            byte_1a: 0,
            listener_24: None,
            listeners_60: Vec::new(),
            desc: WorldDesc {
                gravity: [0.0, 0.0, -73.575],
                broadphase_size: 3000.0,
                broadphase_center: [0.0; 3],
                max_substep: 0.0,
            },
            physics: None,
        };
        // 004D4B2A vt+0x58 (the hk world's getter): nothing to keep.
        // 004D4B33..004D4B62 the TESTrapListener through 004CD320: +0x60 appends it, +0x24 holds it.
        record.listeners_60.push(WorldListener::Trap);
        record.listener_24 = Some(record.listeners_60.len() - 1);
        // 004D4B67 the cell's +0x24 bit 1 (water).
        let flags = world.get::<TesObjectCell>(cell).expect("a cell").flags_24;
        if (flags >> 1) & 1 != 0 {
            let height = super::tes_grid::cell_water_height(world, cell);
            record.listeners_60.push(WorldListener::Water {
                height_18: height * HAVOK_SCALE,
            });
        }
        // 004D4BD1 the TESArrowTargetListener through 004CD2D0.
        record.listeners_60.push(WorldListener::ArrowTarget);
        // 004D4C05 004CB7F0, 004D4C18 0088D260: the centre times the Havok scale.
        let centroid = cell_reference_centroid(world, cell)?;
        record.desc.broadphase_center = [
            centroid[0] * HAVOK_SCALE,
            centroid[1] * HAVOK_SCALE,
            HAVOK_SCALE * centroid[2],
        ];
        // 004D4C27 0088B680(bHavokDebug): with 1 a visual debugger is a stop (the hkVisualDebugger isn't ported).
        if debug != 0 {
            return Err(GameError::unclassified(
                0x0088B680,
                "0088B680: bHavokDebug's visual debugger",
            ));
        }
        record.visual_debugger_14 = false;
        // 004D4C2C..004D4C44 00B2E2E8.
        let substep = if max_time < 1.0 { max_time } else { 1.0 };
        world.get_resource_or_init::<HavokGlobals>().max_time_b2e2e8 = substep;
        record.desc.max_substep = substep;
        record.physics = crate::bs::physics_seam::seam_create_world(world, &record.desc);
        let bhk = world.spawn(record).id();
        // 004D4C4D 004240C0: the ExtraHavok's +0x0C (NiPointer).
        set_extra_havok(world, cell, Some(bhk), None);
        world
            .get_mut::<BhkWorld>(bhk)
            .expect("spawned")
            .reference_count_04 += 1;
        // 004D4C61 vt+0x80 0088C440.
        bhk_world_update(world, bhk)?;
        // 004D4C6B..004D4C7F +0x19 = (00B33A34 == 0): 00B33A34 is BSS no reached code writes.
        world.get_mut::<BhkWorld>(bhk).expect("spawned").byte_19 = 1;
    }
    // 004D4C8A..004D4CCE the CellMopp.
    if cell_mopp(world, cell).is_none() {
        world
            .get_resource_or_init::<HavokWorldCounters>()
            .cell_mopps_b36588 += 1;
        let mopp = world
            .spawn(CellMopp {
                original_vtable: CELL_MOPP_VTABLE,
                bodies_08: Vec::new(),
                field_2c: 1,
            })
            .id();
        set_extra_havok(world, cell, None, Some(mopp));
    }
    Ok(())
}

/// IDA: `004D4970` TESObjectCELL_UpdateHavokAndQuadrants(time): 00B35C04 = 0; an interior at process level 6 with a world
/// runs its vt+0x80 (`0088C440`); then for quadrants 0..3 the cell node's child `2 + q`'s child 3, when not culled, gets
/// `NiAVObject_UpdateSelected(time)` (counted: the port's quadrant subnodes hold nothing that updates). Completion: 100% of
/// the stores.
/// # Errors
/// A stop in the step.
pub fn cell_update_havok(world: &mut World, cell: Entity) -> Result<(), GameError> {
    let data = world.get::<TesObjectCell>(cell).expect("a cell");
    if data.flags_24 & 1 != 0
        && data.process_level_26 == 6
        && let Some(bhk) = cell_bhk_world(world, cell)
    {
        bhk_world_update(world, bhk)?;
    }
    Ok(())
}

/// IDA: `004D4D00` TESObjectCELL_CenterHavokWorld: the cell's world (an interior's ExtraHavok, an exterior's 00B35C24) and its
/// CellMopp: both nonnull run `00532F50`(world, (x * 4096 + 2048, y * 4096 + 2048, 0, 1)), which returns at once with an
/// empty mopp (+0x14 count 0). A mopp with bodies is a stop. Completion: 100% of an empty mopp.
/// # Errors
/// A mopp with bodies.
pub fn cell_center_havok_world(world: &World, cell: Entity) -> Result<(), GameError> {
    let Some(mopp) = cell_mopp(world, cell) else {
        return Ok(());
    };
    let has_world = if cell_is_interior(world, cell) {
        cell_bhk_world(world, cell).is_some()
    } else {
        world
            .get_resource::<HavokGlobals>()
            .is_some_and(|globals| globals.world_b35c24.is_some())
    };
    if has_world
        && !world
            .get::<CellMopp>(mopp)
            .expect("a CellMopp")
            .bodies_08
            .is_empty()
    {
        return Err(GameError::unclassified(
            0x00532F50,
            "00532F50: a CellMopp with bodies",
        ));
    }
    Ok(())
}

/// IDA: `005368B0`(node): a null node answers 0; its collision object (`0047FAC0`) with a body whose motion type (the hk
/// object's +0x50 vt+8, or 7 with none through `004B6D90`) is neither 6 nor 7 answers 1; otherwise each child in slot order,
/// the first 1 answering. The host reads the motion from the rigid body's cinfo +0xD0, which builds it (D2); a phantom has
/// none and answers 0 here. Completion: 100% of the classes the host links.
pub(crate) fn tree_has_moving_body(world: &World, node: Option<Entity>) -> bool {
    let Some(node) = node else {
        return false;
    };
    if let Some(collision) = crate::bs::havok_world::collision_object(world, node)
        && let Some(body) = world
            .get::<crate::bs::havok_stream::BhkSpCollisionObject>(collision)
            .and_then(|object| object.body_10)
        && let Some(rigid) = world.get::<crate::bs::havok_stream::BhkRigidBody>(body)
        && !matches!(rigid.cinfo_0c[0xD0], 6 | 7)
    {
        return true;
    }
    world
        .get::<crate::gamebryo::scene_node::NiNodeChildren>(node)
        .is_some_and(|children| {
            children
                .slots_ac
                .iter()
                .any(|&child| tree_has_moving_body(world, child))
        })
}
