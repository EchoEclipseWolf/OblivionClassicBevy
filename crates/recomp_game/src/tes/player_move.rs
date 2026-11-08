//! `00512540`'s player arm and `0066EAF0` PlayerCharacter_MoveToCell, as a script's MoveTo reaches them: a new game's command 64, into the Imperial Prison (`1FBB9`, an interior). The cell load needs the driver's load services, which `CellLoadHost` lends.
//!
//! Evidence: `evidence/gameplay-run002-2026-11-07/` (run-x's snapshots after command 64, run-w's coverage up to the IO drain).

use std::rc::Rc;

use bevy_ecs::prelude::*;
use recomp_core::reader::StreamError;
use recomp_core::x87;

use super::actor::{MobileObject, TesObjectRefr};
use super::ai_package::{
    actor_is_dead, cell_world, parent_cell, reference_position, reference_world,
};
use super::character_controller::actor_set_position;
use super::forms_world::cell_is_interior;
use super::move_to::release_followers;
use super::player::PlayerCharacter;
use super::tes_grid::TesGridServices;

/// IDA: N/A what the move reaches that only the host holds: the grid's services, whose `drain` is 00434020 with the host's timed pass.
pub struct CellLoadServices<'g, 'a, 'b> {
    pub grid: &'g mut TesGridServices<'a, 'b>,
}

/// IDA: N/A a body run with `CellLoadServices`.
pub type CellLoadBody<'f> =
    dyn FnMut(&mut World, &mut CellLoadServices<'_, '_, '_>) -> Result<(), StreamError> + 'f;

/// IDA: N/A `CellLoadHost`'s call: the body to run with the services.
pub type CellLoadCall = dyn Fn(&mut World, &mut CellLoadBody<'_>) -> Result<(), StreamError>;

/// IDA: N/A how a script command reaches the cell load's services: the host builds them for the call and runs the body with them.
#[derive(Clone)]
pub struct CellLoadHost(pub Rc<CellLoadCall>);

/// IDA: N/A the music device's half of 0066EAF0's interior arm and 00440AF0's, which the host holds: 006AB160(type) with no path, and when it loads, 006A8D50 ResumeMusic when `resume` holds and 006AB420 StartMusicPlayback when `start` does. Answers whether it loaded.
#[derive(Clone)]
pub struct CellMusicHost(pub Rc<CellMusicCall>);

/// IDA: N/A `CellMusicHost`'s call: the music type, and whether to start it.
pub type CellMusicCall = dyn Fn(&mut World, u16, bool, bool) -> bool;

/// IDA: 00B34420, which 004835D0 sets from the old world's +0x7C.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PlayerMoveGlobals {
    pub parent_world_b34420: u8,
    /// IDA: N/A 0066EAF0's local at [esp+0x12] (var_B4+2): a track loaded while a movie ran, resumed after it.
    pub music_pending: bool,
}

// IDA: 004CAD00 TESObjectCELL_ResolveMusicType(position null) for an interior: under the cell's region lock (00B35E10's first call zeroes 00B35E04..0C), the 0x0B extra's byte (0041F9C0, sign-extended), else 0, since an interior skips the regions and the world. Completion: 100% of the interior arm.
pub(crate) fn cell_music_type(world: &World, cell: Entity) -> u16 {
    let value = world
        .get::<super::forms_world::TesObjectCell>(cell)
        .and_then(|data| data.extra_28.get(0x0B))
        .map_or(0, |entry| match entry.data {
            super::base_extra_list::ExtraData::Byte(value) => i32::from(value as i8),
            _ => 0,
        });
    value as u16
}

// IDA: 004BE5A0 and 004BD980 on 00B35B90 and 00B35B8C: each cancels its map's queued loader tasks (00642D90's walk, 00432130). An empty map does nothing; a held task is a stop.
fn cancel_loader_tasks(world: &World) -> Result<(), StreamError> {
    let Some(tasks) = world.get_resource::<crate::bs::task_manager::TaskGlobals>() else {
        return Ok(());
    };
    for map in [
        &tasks.exterior_cell_tasks_b35b90,
        &tasks.distant_lod_tasks_b35b8c,
    ]
    .into_iter()
    .flatten()
    {
        if map.buckets_0c.iter().any(|&bucket| bucket != 0) {
            return Err(StreamError(
                "004BE48C: cancelling a queued cell or LOD loader task",
            ));
        }
    }
    Ok(())
}

/// IDA: 00512540(player, target, x, y, z): vt+0x198(0) (Actor_IsDead) returns; 00675D50 (`release_followers`); the target's parent cell and exterior world (004D6670) and its position (vt+0x174) plus the offsets, double-precision sums stored as floats; 004BE5A0 and 004BD980 on the two loader maps. A target with no cell is the exterior lookup or creation (0051263A), a stop. ConsoleManager_GetSingleton(1) makes the console's block, which the port doesn't hold, and its +0x31 is clear, so no toggle. Then 0066EAF0(position, rotation (0, 0, the target's Z), cell, 1), vt+0x178(0) (0065B5C0), +0x117 = 1 and 00665260. Completion: 100% of the arms an interior target reaches.
/// TODO: [ ] 00512753 vt+0x178's controller sync for the player (0065B5C0), and the console block.
/// # Errors
/// A stop above or in 0066EAF0.
pub fn move_player_to(
    world: &mut World,
    player: Entity,
    target: Entity,
    offset: [f32; 3],
) -> Result<(), StreamError> {
    if actor_is_dead(world, player, false) {
        return Ok(());
    }
    release_followers(world, player)?;
    let cell = parent_cell(world, target);
    let target_rotation = world
        .get::<TesObjectRefr>(target)
        .expect("a reference")
        .rotation_20;
    let base = reference_position(world, target)?;
    let position: [f32; 3] =
        std::array::from_fn(|at| x87::stored_float(f64::from(base[at]) + f64::from(offset[at])));
    let _space = reference_world(world, target);
    cancel_loader_tasks(world)?;
    let Some(cell) = cell else {
        return Err(StreamError(
            "0051263A: a MoveTo the player into an exterior",
        ));
    };
    let host = world
        .get_non_send::<CellLoadHost>()
        .cloned()
        .ok_or(StreamError("0066EAF0: a cell load with no host services"))?;
    (host.0)(world, &mut |world, services| {
        player_move_to_cell(
            world,
            player,
            position,
            [0.0, 0.0, target_rotation[2]],
            cell,
            true,
            services,
        )
    })?;
    Err(StreamError(
        "00512722: vt+0x178 on the player after 0066EAF0",
    ))
}

/// IDA: 0066EAF0 PlayerCharacter_MoveToCell(position, rotation, cell, reset_sky), its interior arm. The old world (004D6670) and the cell's (an interior's is null); 004835D0: 00B34420 = old world's +0x7C set. MoviePlayer_IsRunning. The parent cell's 004CECD0. vt+0x1CC 0065A3E0 (`actor_set_position`); 004D89A0 the rotation and vt+0x40(4); a horse (vt+0x380) is a stop; 004D8AA0 the parent cell, vt+0x40(4) when it changes, then the process's vt+0x500. An interior with OSGlobals +0x24: `cell_music_type` and the host's 006AB160; a load while a movie runs is kept for after it (`music_pending`), else ResumeMusic and StartMusicPlayback. Then 004455E0, 00560110, 0066ED16's sounds (counted), the equipped light, 00540380 with `reset_sky`, 004D8AA0(null), 004D35D0 and, with no node, 00434020(5). Completion: as far as the drain; the rest is a stop.
/// # Errors
/// A stop above, or in a callee.
pub fn player_move_to_cell(
    world: &mut World,
    player: Entity,
    position: [f32; 3],
    rotation: [f32; 3],
    cell: Entity,
    reset_sky: bool,
    services: &mut CellLoadServices<'_, '_, '_>,
) -> Result<(), StreamError> {
    let old_space = reference_world(world, player);
    let _new_space = cell_world(world, Some(cell));
    // 0066EB23 MoviePlayer_IsRunning, read before anything moves.
    let movie_running = world
        .get_resource::<super::movie_player::MoviePlayer>()
        .is_some_and(super::movie_player::movie_player_is_running);
    // 004835D0.
    if let Some(space) = old_space {
        let held = world
            .get::<super::forms_world::TesWorldSpace>(space)
            .is_some_and(|data| {
                !matches!(
                    data.parent_7c,
                    super::form_components::FormRef::Raw(0)
                        | super::form_components::FormRef::Resolved(None)
                )
            });
        world
            .get_resource_or_init::<PlayerMoveGlobals>()
            .parent_world_b34420 = u8::from(held);
    }
    if let Some(old) = parent_cell(world, player) {
        super::ai_cell_moves::remove_reference_runtime(world, old, player)?;
    }
    actor_set_position(world, player, position)?;
    world
        .get_mut::<TesObjectRefr>(player)
        .expect("the player")
        .rotation_20 = rotation;
    super::forms_items::set_change_flags(world, player, 4)?;
    if world
        .get::<PlayerCharacter>(player)
        .is_some_and(|data| data.last_ridden_horse_1e0 != 0)
    {
        return Err(StreamError("0066EB94: the player's horse"));
    }
    // 004D8AA0.
    if parent_cell(world, player) != Some(cell) {
        super::forms_items::set_change_flags(world, player, 4)?;
    }
    world
        .get_mut::<TesObjectRefr>(player)
        .expect("the player")
        .parent_cell_40 = Some(cell);
    // 004D8ADD vt+0x500, MiddleHighProcess 00651EF0: its +0x170 list's references move with the actor; null returns.
    if let Some(process) = world
        .get::<MobileObject>(player)
        .and_then(|mobile| mobile.process_58)
        && world
            .get::<super::actor_process::MiddleHighProcess>(process)
            .is_some_and(|middle| middle.field_170 != 0)
    {
        return Err(StreamError("00651F10: the player's linked references"));
    }
    if !cell_is_interior(world, cell) {
        return Err(StreamError("0066EC35: an exterior cell"));
    }
    let mut music_pending = false;
    if world
        .get_resource::<super::os_globals::OsGlobalsData>()
        .is_some_and(|os| os.sound_24.is_some())
    {
        let music_type = cell_music_type(world, cell);
        let host = world
            .get_non_send::<CellMusicHost>()
            .cloned()
            .ok_or(StreamError("0066EBF2: the cell's music with no host"))?;
        let loaded = (host.0)(world, music_type, !movie_running, !movie_running);
        music_pending = loaded && movie_running;
    }
    world
        .get_resource_or_init::<PlayerMoveGlobals>()
        .music_pending = music_pending;
    let mut rect = position;
    super::tes_grid::enter_interior_cell(world, Some(cell), &mut rect, services.grid)?;
    reset_tree_canopy(world)?;
    // 0066ED16: with OSGlobals +0x24, 006AC210 empties the game sounds' +0x304 map and 006AC3C0 stops those with flag 0x1000; the port plays no game sounds, so both are counted.
    if world
        .get_resource::<super::os_globals::OsGlobalsData>()
        .is_some_and(|os| os.sound_24.is_some())
    {
        count(world, 0x006A_C210);
        count(world, 0x006A_C3C0);
    }
    // 0066ED2D the process's vt+0xF0(1), 0064B270: the equipped light at +0xE8, held by 00484E80; with one, vt+0x348.
    if let Some(process) = world
        .get::<MobileObject>(player)
        .and_then(|mobile| mobile.process_58)
        && world
            .get::<super::actor_process::MiddleHighProcess>(process)
            .is_some_and(|middle| middle.light_e8.is_some())
    {
        return Err(StreamError(
            "0066ED40: the player's equipped light (vt+0x348)",
        ));
    }
    if reset_sky {
        sky_reset_weather(world);
    }
    // 0066ED63 004D8AA0(null): no flag for a null cell, +0x40 = null, then the process's vt+0x500 as above.
    world
        .get_mut::<TesObjectRefr>(player)
        .expect("the player")
        .parent_cell_40 = None;
    // 004D35D0 with the grid's archives: the player's 00438060 (QueuedPlayer, priority 00440C80's).
    {
        let mut load = super::reference_load::ReferenceLoadServices {
            archives: services.grid.archives.as_deref_mut(),
        };
        super::ai_cell_moves::insert_reference_queued(world, cell, player, Some(&mut load))?;
    }
    // 0066ED70 vt+0x154: with no node and no save active, the IO manager drains through stage 5.
    let node = world
        .get::<TesObjectRefr>(player)
        .expect("the player")
        .loaded_3d_3c;
    if node == 0 && !super::forms_references::save_load_active(world)? {
        let drain = services
            .grid
            .drain
            .as_mut()
            .ok_or(StreamError("0066ED70: a cell load with no drain"))?;
        drain(world, 5)?;
    }
    place_camera(world, player, position, rotation)?;
    place_listener(world, player, position);
    super::script_commands::stop_on_active_quest(world)?;
    // 0066F02E the horse's move (vt+0x380), none here; 0066F07F 007AA4D0 on the BSShaderAccumulator, the renderer's, counted.
    count(world, 0x007A_A4D0);
    if world
        .get::<PlayerCharacter>(player)
        .is_some_and(|data| data.first_person_node_5d0.is_some())
    {
        // 0066F09C..0066F234, the camera block with a first-person node (0065AF30's placement, 00603CA0, 0066B710 and the
        // first-person camera, run-ab's 6 draws): host deferral D16, Build 3's. Recorded, and 0066EAF0 goes on at 0066F234.
        world
            .get_resource_or_init::<CameraBlockDeferrals>()
            .moves
            .push(player);
    }
    // 0066F243 0055FA50(camera, 0.0): SpeedTree's wind time 00B39F18, camera, fog into the tree manager and canopy scale, the renderer's state, counted.
    count(world, 0x0055_FA50);
    let frame = world
        .get_resource::<super::script_vm::ScriptVmGlobals>()
        .map_or(0.0, |globals| globals.frame_seconds_b33e9c);
    tes_update(world, frame)?;
    Err(StreamError(
        "0066F260: the intro movie (+0x7F8) and the rest of 0066EAF0",
    ))
}

/// IDA: N/A host deferral D16: 0066EAF0's camera block with the player's first-person node (+0x5D0), 0066F09C..0066F234:
/// 0065AF30, 00603CA0, 0066B710 and the camera's placement, whose 6 main-thread draws run-ab records (Build 3).
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct CameraBlockDeferrals {
    pub moves: Vec<Entity>,
}

/// IDA: 00B33A30, which 004424D0 accumulates.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq)]
pub struct TesUpdateGlobals {
    pub time_b33a30: f32,
}

// IDA: 004424D0 TES_Update(time) on g_tes, with an interior: 006FACA0(0, 0) and 008984A0, the water and wind shader globals, counted; 00B33A30 += time, a float; 004D4970 on the interior, whose Havok world (a cell of level 6 with one) is a stop, the rest counted; no water node (+0x58) is TES_InitializeWater, a stop; 0057AB70 answers 0, since no DialogMenu is open, so the time stays; 00542F20 the Sky's update (`sky_update`); NiAVObject_Update on +0x14, counted; 00676E40, whose projectile list (+0x50) holding anything is a stop. An exterior (004823D0) is a stop. Completion: 100% of the state for an interior.
fn tes_update(world: &mut World, time: f32) -> Result<(), StreamError> {
    let (interior, water) = {
        let tes = super::tes_grid::tes(world)?;
        (tes.current_interior_cell_34, tes.water_node_58.is_some())
    };
    count(world, 0x006F_ACA0);
    count(world, 0x0089_84A0);
    {
        let mut globals = world.get_resource_or_init::<TesUpdateGlobals>();
        globals.time_b33a30 = x87::stored_float(f64::from(globals.time_b33a30) + f64::from(time));
    }
    let Some(cell) = interior else {
        return Err(StreamError(
            "00442563: 004823D0, the exterior grid's update",
        ));
    };
    if world
        .get::<super::forms_world::TesObjectCell>(cell)
        .is_some_and(|data| data.process_level_26 == 6)
        && super::tes_grid::cell_has_havok_world(world, cell)
    {
        return Err(StreamError("004D499B: the interior's Havok world step"));
    }
    count(world, 0x004D_4970);
    if !water {
        return Err(StreamError("00442574: TES_InitializeWater"));
    }
    let draw = world
        .get_non_send::<super::ai_conditions::ScriptRandom>()
        .map(|random| random.1.clone())
        .ok_or(StreamError("005422F0: the sky's draws with no host"))?;
    super::sky_update::sky_update(world, &mut || Ok(draw()))?;
    count(world, 0x0070_7370);
    if world
        .resource::<super::actor_process::ActorProcessGlobals>()
        .manager
        .fields_50[..2]
        != [0, 0]
    {
        return Err(StreamError("00676E54: the manager's projectiles"));
    }
    Ok(())
}

// IDA: 0065DA60 PlayerCharacter_GetZRotation: the reference's +0x28 (0065ABB0) plus +0x61C, a float; below 0.0 it gains 2pi, above 2pi it loses it, a float. Completion: 100%.
fn player_heading(world: &World, player: Entity) -> f32 {
    let two_pi = 6.283_185_5_f32;
    let base = world
        .get::<TesObjectRefr>(player)
        .expect("the player")
        .rotation_20[2];
    let offset = world
        .get::<PlayerCharacter>(player)
        .map_or(0.0, |data| data.z_rotation_offset_61c);
    let heading = x87::stored_float(f64::from(offset) + f64::from(base));
    if heading < 0.0 {
        x87::stored_float(f64::from(heading) + f64::from(two_pi))
    } else if two_pi < heading {
        x87::stored_float(f64::from(heading) - f64::from(two_pi))
    } else {
        heading
    }
}

// IDA: 0066EF95..0066F026 with OSGlobals +0x24: 006A8E10(position): +0x80, +0x84, +0x88 = z + 128.0 (00A3F428); then the heading's `_CIcos` and `_CIsin`, each a float, into 006A8E40(sin, cos, 0): +0x8C, +0x90, +0x94, then +0x98 = +0x9C = 0.0, +0xA0 = 1.0. Completion: 100%.
fn place_listener(world: &mut World, player: Entity, position: [f32; 3]) {
    let cos = f64::from(player_heading(world, player)).cos() as f32;
    let sin = f64::from(player_heading(world, player)).sin() as f32;
    let Some(mut os) = world.get_resource_mut::<super::os_globals::OsGlobalsData>() else {
        return;
    };
    let Some(sound) = os.sound_24.as_mut() else {
        return;
    };
    sound.listener_position_80 = [
        position[0],
        position[1],
        x87::stored_float(f64::from(position[2]) + 128.0),
    ];
    sound.listener_front_8c = [sin, cos, 0.0];
    sound.listener_top_98 = [0.0, 0.0, 1.0];
}

// IDA: 0070FDD0 NiMatrix33_MakeZRotation(angle): fsincos, each stored as a float, then cos, sin, 0, -sin, cos, 0, 0, 0, 1.
fn rotation_z(angle: f32) -> [f32; 9] {
    let (sin, cos) = f64::from(angle).sin_cos();
    let (sin, cos) = (sin as f32, cos as f32);
    [cos, sin, 0.0, -sin, cos, 0.0, 0.0, 0.0, 1.0]
}

// IDA: 0070FD30 NiMatrix33_MakeXRotation(angle): 1, 0, 0, 0, cos, sin, 0, -sin, cos, with `fchs` on the float sine.
fn rotation_x(angle: f32) -> [f32; 9] {
    let (sin, cos) = f64::from(angle).sin_cos();
    let (sin, cos) = (sin as f32, cos as f32);
    [1.0, 0.0, 0.0, 0.0, cos, sin, 0.0, -sin, cos]
}

// IDA: 0066ED9D..0066EF95. The Z rotation wraps: above pi (00A3D5B8) it loses 2pi (00A3D5B0) until it isn't, below -pi (00A491E0) it gains 2pi until it isn't, stored as a float each step. The view is identity times 0070FDD0(z) (007100A0); the pitch is x - y for z at or above 0.0, else y + x, a float, through 0070FD30. With a player node (vt+0x154, 006600F0) its +0x30 takes the pitch. The pitch times the view goes to the World SceneGraph's first child's +0x30 rotation and +0x54 the position, then NiAVObject_Update(0.0, 0) on it, and TES +0x0C's 00707610 and 00707980. Completion: 100% with no player node.
fn place_camera(
    world: &mut World,
    player: Entity,
    position: [f32; 3],
    rotation: [f32; 3],
) -> Result<(), StreamError> {
    use crate::gamebryo::path_interpolator::multiply;
    const IDENTITY: [f32; 9] = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
    let pi = std::f32::consts::PI;
    let two_pi = 6.283_185_5_f32;
    let mut z = rotation[2];
    while z > pi {
        z = x87::stored_float(f64::from(z) - f64::from(two_pi));
    }
    while z < -pi {
        z = x87::stored_float(f64::from(z) + f64::from(two_pi));
    }
    let view = multiply(&IDENTITY, &rotation_z(z));
    let pitch_angle = if z >= 0.0 {
        x87::stored_float(f64::from(rotation[0]) - f64::from(rotation[1]))
    } else {
        x87::stored_float(f64::from(rotation[1]) + f64::from(rotation[0]))
    };
    let pitch = rotation_x(pitch_angle);
    // 0066EEA2..0066EECC vt+0x154 (006600F0 PlayerCharacter_GetNiNode: +0x5D0 unless +0x588 is set or it's null, else
    // 00612220's +0x3C): a node takes the pitch as its +0x30 rotation.
    let node = world.get::<PlayerCharacter>(player).and_then(|data| {
        if data.view_flags_588[0] != 0 {
            None
        } else {
            data.first_person_node_5d0
        }
    });
    let node = node.or_else(|| {
        world
            .get::<super::reference_3d::Reference3D>(player)
            .map(|loaded| loaded.0)
            .filter(|&node| {
                world
                    .get::<crate::gamebryo::scene_node::SceneNodePayload>(node)
                    .is_some_and(|payload| payload.effective_av_flags & 1 == 0)
            })
    });
    if let Some(node) = node
        && let Some(mut payload) =
            world.get_mut::<crate::gamebryo::scene_node::SceneNodePayload>(node)
    {
        payload.local_transform.rotation = pitch;
    }
    let combined = multiply(&pitch, &view);
    let graph = world
        .get_resource::<crate::bs::scene_graphs::SceneGraphGlobals>()
        .and_then(|globals| globals.world)
        .ok_or(StreamError("0066EEF7: a null g_worldSceneGraph faults"))?;
    let child = world
        .get::<crate::gamebryo::scene_node::NiNodeChildren>(graph)
        .and_then(|children| children.slots_ac.first().copied().flatten())
        .ok_or(StreamError(
            "0066EF0D: a World SceneGraph with no first child faults",
        ))?;
    {
        let mut payload = world
            .get_mut::<crate::gamebryo::scene_node::SceneNodePayload>(child)
            .ok_or(StreamError("0066EF0D: a first child other than a NiNode"))?;
        payload.local_transform.rotation = combined;
        payload.local_transform.translation = position;
    }
    let mut runtime = crate::gamebryo::scene_update::NoSceneControllers;
    crate::gamebryo::scene_update::update_scene_object(world, child, 0.0, false, &mut runtime)?;
    let root = super::tes_grid::tes(world)?
        .scene_root_0c
        .ok_or(StreamError("0066EF82: a null TES +0x0C"))?;
    let defaults = world
        .resource::<crate::gamebryo::registration::NiProcessDefaults>()
        .scene_property_defaults();
    crate::gamebryo::scene_state::refresh_property_state(world, root, &defaults)?;
    crate::gamebryo::scene_state::refresh_effect_state(world, root)
}

/// IDA: N/A the counts of the move's calls the port doesn't make.
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct PlayerMoveBoundaries {
    pub calls: std::collections::BTreeMap<u32, u32>,
}

// IDA: N/A a helper, with no address of its own.
fn count(world: &mut World, address: u32) {
    *world
        .get_resource_or_init::<PlayerMoveBoundaries>()
        .calls
        .entry(address)
        .or_default() += 1;
}

// IDA: 00560110: the tree manager (0055F750(0) when null, a stop here); with its +0x04 set, that and 00B43108 are released and 00440420 reloads Data\Textures\Trees\CanopyShadow.dds, a stop. A null +0x04 returns. Completion: 100% of the null arm.
fn reset_tree_canopy(world: &World) -> Result<(), StreamError> {
    let globals = world.get_resource::<crate::bs::tree_manager::TreeManagerGlobals>();
    let Some(manager) = globals.and_then(|globals| globals.tree_manager_b39e04.as_ref()) else {
        return Err(StreamError("0056011B: 00560110 with no tree manager"));
    };
    if manager.pointer_04.is_some() {
        return Err(StreamError("00560158: reloading the canopy shadow"));
    }
    Ok(())
}

// IDA: 00540380 on the Sky: +0xF4 = 0.0; +0x1C, +0x14 and +0x10 = 0; +0xFC = (+0xFC & ~8) | 1; the clouds' 0053BBC0 and the precipitation's 0053D6C0, the sky renderer's, counted. Completion: 100% of the state.
fn sky_reset_weather(world: &mut World) {
    let (clouds, precipitation) = {
        let Some(mut globals) = world.get_resource_mut::<super::sky::SkyGlobals>() else {
            return;
        };
        let Some(sky) = globals.singleton_b365c4.as_mut() else {
            return;
        };
        sky.float_f4 = 0.0;
        sky.weather_override_1c = 0;
        sky.weather_14 = 0;
        sky.first_weather_10 = 0;
        sky.dword_fc = (sky.dword_fc & !8) | 1;
        (sky.clouds_2c.is_some(), sky.precipitation_38.is_some())
    };
    if clouds {
        count(world, 0x0053_BBC0);
    }
    if precipitation {
        count(world, 0x0053_D6C0);
    }
}
