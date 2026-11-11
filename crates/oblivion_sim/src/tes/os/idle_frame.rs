//! The game-side pieces of `0040D800` OSGlobals_RunIdleFrame that the menu-mode frames after New Game's command 64 run:
//! the first menu frame's weapon-node resets, the frame counter, the LOD fade factors, the per-frame resets, the process
//! manager's `+0x58` walk, `0065E900` for the player and `0055FA50`'s menu-mode stores. The driver's frame
//! (`crate::tes::winmain::game_frame`) calls them in the exe's order.
//!
//! **Counted boundaries** (`IdleFrameBoundaries`): `00498F00` (its sleep request 00B350E8 has no writer the port runs),
//! `007C1F50` (the shadow scene's per-camera light list: the renderer's, host substitution D10), `0078D140` (SpeedTree's
//! camera), `0055E060` and `0078D3F0` (the tree manager's frame, outside menu mode), `00701AB0`/`00701AD0` (00B3F940 is null:
//! no Gamebryo profiler), `00407AE0` (the debug text, off) and `OSGlobals_RenderFrame` (`0040D4D0`: Bevy draws).

use crate::tes::actor_process::reset_weapon_node_scales;
use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;

/// IDA: N/A the idle frame's calls that are counted rather than made.
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct IdleFrameBoundaries {
    pub calls: std::collections::BTreeMap<u32, u32>,
}

/// IDA: N/A count a boundary call by its address.
pub fn count_idle_call(world: &mut World, address: u32) {
    *world
        .get_resource_or_init::<IdleFrameBoundaries>()
        .calls
        .entry(address)
        .or_default() += 1;
}

/// IDA: the globals `0040D800` writes each frame: 00B3341C (set outside menu mode, so the first menu frame resets the weapon
/// nodes), 00B02C54 the frame counter (image 1), 00B075E8..F0 the LOD fade factors (object, item, actor; image 1.0),
/// 00B333B8 (the Havok step's flag, cleared each frame), 00BA7A04, 00B3C089 and 00B3B77C (cleared each frame), and
/// 00B3BCCC + 0x28, the frame's phase marker (0, 50, 100..400 around the process manager's calls).
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct IdleFrameGlobals {
    pub weapon_reset_pending_b3341c: u8,
    pub frame_counter_b02c54: u32,
    pub lod_fade_object_b075e8: f32,
    pub lod_fade_item_b075ec: f32,
    pub lod_fade_actor_b075f0: f32,
    pub havok_step_flag_b333b8: u8,
    pub byte_ba7a04: u8,
    pub byte_b3c089: u8,
    pub byte_b3b77c: u8,
    pub phase_b3bcf4: u32,
    /// 00B2D860: 0055FA50's clamp of 00B12638 to [0, 1].
    pub canopy_b2d860: f32,
    /// g_canopyShadowScale, 0055FA50's copy of 00B12630 (image 0x200).
    pub canopy_shadow_scale: u32,
}

impl Default for IdleFrameGlobals {
    /// IDA: N/A the image's values.
    fn default() -> Self {
        Self {
            weapon_reset_pending_b3341c: 0,
            frame_counter_b02c54: 1,
            lod_fade_object_b075e8: 1.0,
            lod_fade_item_b075ec: 1.0,
            lod_fade_actor_b075f0: 1.0,
            havok_step_flag_b333b8: 0,
            byte_ba7a04: 0,
            byte_b3c089: 0,
            byte_b3b77c: 0,
            phase_b3bcf4: 0,
            canopy_b2d860: 1.0,
            canopy_shadow_scale: 0x200,
        }
    }
}

/// IDA: 0040D817..0040D893: in menu mode with 00B3341C set, the process manager's level-0 list (00673A50(0): +0x68), each
/// actor at process level 0 with a node (vt+0x154) has its process's `00651DD0`, then the player's; 00B3341C = 0. Outside
/// menu mode 00B3341C = 1. Completion: 100%.
pub fn idle_frame_weapon_node_resets(world: &mut World, menu_mode: bool) {
    if !menu_mode {
        world
            .get_resource_or_init::<IdleFrameGlobals>()
            .weapon_reset_pending_b3341c = 1;
        return;
    }
    if world
        .get_resource_or_init::<IdleFrameGlobals>()
        .weapon_reset_pending_b3341c
        != 0
    {
        let actors = world
            .get_resource::<crate::tes::actor_process::ActorProcessGlobals>()
            .map(|globals| globals.manager.high_68.list.items())
            .unwrap_or_default();
        for actor in actors {
            if crate::tes::actor::actor_process_level(world, actor) == 0
                && crate::tes::forms_init_world::reference_ni_node(world, actor).is_some()
                && let Some(process) = world
                    .get::<crate::tes::actor::MobileObject>(actor)
                    .and_then(|mobile| mobile.process_58)
            {
                reset_weapon_node_scales(world, process);
            }
        }
        if let Some(process) = crate::tes::player::the_player(world)
            .and_then(|player| world.get::<crate::tes::actor::MobileObject>(player))
            .and_then(|mobile| mobile.process_58)
        {
            reset_weapon_node_scales(world, process);
        }
    }
    world
        .get_resource_or_init::<IdleFrameGlobals>()
        .weapon_reset_pending_b3341c = 0;
}

/// IDA: 0040D8DD..0040D9A6: with 00B333B8 the Complex LOD fade settings (00B02D90, 98, A0); else with the TES's worldspace
/// (0043F270, +0x74) with a parent (004EF1A0, +0x7C) the City ones (00B02DA8..B8, a stop: no city is reached); else a player in an interior cell
/// the Interior ones (00B02DC0..D0); else 1.0 each; into 00B075E8, EC and F0. Completion: 100% but the city arm.
/// # Errors
/// The city arm.
pub fn idle_frame_lod_fade(world: &mut World) -> Result<(), GameError> {
    let setting = |world: &World, address: u32| {
        world
            .get_resource::<crate::tes::settings::SettingCollections>()
            .and_then(|settings| settings.ini.find_address(address))
            .map_or(1.0, |setting| f32::from_bits(setting.raw))
    };
    let flag = world
        .get_resource_or_init::<IdleFrameGlobals>()
        .havok_step_flag_b333b8;
    let interior = crate::tes::player::the_player(world)
        .and_then(|player| crate::tes::forms_references::parent_cell(world, player))
        .is_some_and(|cell| crate::tes::forms_world::cell_is_interior(world, cell));
    // 0043F270 the TES's +0x74, then 004EF1A0 its +0x7C (the parent worldspace, WNAM).
    let worldspace = world
        .get_resource::<crate::tes::tes::TesGlobals>()
        .and_then(|globals| globals.tes.as_ref())
        .and_then(|tes| tes.current_world_space_74)
        .and_then(|space| world.get::<crate::tes::forms_world::TesWorldSpace>(space))
        .is_some_and(|space| {
            !matches!(
                space.parent_7c,
                crate::tes::form_components::FormRef::Resolved(None)
                    | crate::tes::form_components::FormRef::Raw(0)
            )
        });
    let factors = if flag != 0 {
        [0x00B0_2D90, 0x00B0_2D98, 0x00B0_2DA0].map(|address| setting(world, address))
    } else if worldspace {
        return Err(GameError::unclassified(
            0x0040D924,
            "0040D924: the City LOD fade factors (004EF1A0)",
        ));
    } else if interior {
        [0x00B0_2DC0, 0x00B0_2DC8, 0x00B0_2DD0].map(|address| setting(world, address))
    } else {
        [1.0; 3]
    };
    let mut globals = world.resource_mut::<IdleFrameGlobals>();
    globals.lod_fade_object_b075e8 = factors[0];
    globals.lod_fade_item_b075ec = factors[1];
    globals.lod_fade_actor_b075f0 = factors[2];
    Ok(())
}

/// IDA: 0040DBB9..0040DBCB: 00BA7A04, 00B3C089, 00B3B77C and 00B333B8 = 0. Completion: 100%.
pub fn idle_frame_resets(world: &mut World) {
    let mut globals = world.get_resource_or_init::<IdleFrameGlobals>();
    globals.byte_ba7a04 = 0;
    globals.byte_b3c089 = 0;
    globals.byte_b3b77c = 0;
    globals.havok_step_flag_b333b8 = 0;
}

/// IDA: 0055FA50(camera, menuMode), 0040DD66: outside menu mode 00B39F18 += the frame time and `0078D3F0` (counted, a stop
/// here: that path is the game's); a camera passes its world translate (+0x88) and its negated first column (+0x64, +0x70,
/// +0x7C) to `0078D140` (SpeedTree's camera, counted); outside menu mode the tree manager's fog colours and `0055E060`
/// (unreached here); g_canopyShadowScale = 00B12630; 00B2D860 = 00B12638 clamped to [0, 1]. Completion: 100% of the
/// menu-mode arm.
/// # Errors
/// The non-menu arm.
pub fn camera_tree_frame(world: &mut World, menu_mode: bool) -> Result<(), GameError> {
    if !menu_mode {
        return Err(GameError::unclassified(
            0x0055FA5C,
            "0055FA5C: 0055FA50 outside menu mode",
        ));
    }
    count_idle_call(world, 0x0078_D140);
    let mut globals = world.get_resource_or_init::<IdleFrameGlobals>();
    // 0055FBCB..0055FC14: 00B12630 and 00B12638 are image values (0x200, 1.0) with no writer the port runs.
    globals.canopy_shadow_scale = 0x200;
    let canopy = 1.0_f32;
    globals.canopy_b2d860 = if canopy < 0.0 {
        0.0
    } else if canopy <= 1.0 {
        canopy
    } else {
        1.0
    };
    Ok(())
}
