//! An actor's fade and alpha: a High process's fade state (+0x2BC) and amount (+0x2C0), which the ctor (`006290A9`) starts at
//! a fade-in (1, 0.0) and `00633250` steps each frame from `00603CA0`; `005EE1B0`, the alpha those give with the actor's
//! chameleon and invisibility; and the actor's vt+0x324 (`005E5FC0`, the player's `0066E950`), which marks a node whose
//! alpha is below 1.0 ("MAL", a FadeNodeMaxAlphaExtraData) and clears the mark at 1.0.
//!
//! Evidence: run-ar (`records-run-ar-fade.txt`): during New Game's move `00633250` runs twice for the player from `00605525`,
//! the first finishing the ctor's fade-in at once (`fPlayerTeleportFadeSeconds` 0.001); run-x's after_move holds state 0,
//! amount 1.0.

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;

use super::actor::TesObjectRefr;
use super::actor_process::{HighProcess, MiddleHighProcess, process_level};
use crate::tes::actor::process_of;
use crate::tes::player::the_player;

/// IDA: 00B367A8 fActorTeleportFadeSeconds.
const ACTOR_TELEPORT_FADE_SECONDS_B367A8: u32 = 0x00B3_67A8;
/// IDA: 00B367B0 fPlayerTeleportFadeSeconds.
const PLAYER_TELEPORT_FADE_SECONDS_B367B0: u32 = 0x00B3_67B0;
/// IDA: 00B38000 fMagicPlayerMinimumInvisibility.
const MAGIC_PLAYER_MINIMUM_INVISIBILITY_B38000: u32 = 0x00B3_8000;
/// IDA: the actor value codes `005EE1B0` reads: 46 chameleon, 47 invisibility.
const AV_CHAMELEON: u32 = 46;
const AV_INVISIBILITY: u32 = 47;
/// IDA: 00420FC0's extra type (0x51), the refraction property `005EE1B0` tests.
const EXTRA_REFRACTION: u8 = 0x51;

// IDA: N/A a float game setting's raw value.
fn setting_float(world: &World, address: u32) -> Result<f32, GameError> {
    world
        .resource::<crate::tes::game_settings::GameSettingCollection>()
        .float_at(address)
        .ok_or(GameError::unclassified(
            0x00633250,
            "00633250: a fade setting isn't registered",
        ))
}

// IDA: N/A the frame's seconds, g_timeInfo +0x0C (00B33E9C).
fn frame_time(world: &World) -> f32 {
    world
        .get_resource::<crate::tes::script_vm::ScriptVmGlobals>()
        .map_or(0.0, |globals| globals.frame_seconds_b33e9c)
}

/// IDA: 00633250 (thiscall process, actor) (`retn 4`), the fade's frame, from `00603CA0` (`00605525`) and
/// PlayerCharacter_UpdateFrame (`00671DA4`). State 0: an amount below 1.0 becomes 1.0 and `005EE1B0` runs; answers 0.
/// Otherwise the step is frame time / `fPlayerTeleportFadeSeconds` for the player, else `fActorTeleportFadeSeconds`: states
/// 1 and 3 add it, past 1.0 the amount is 1.0 and the state 0; state 2 subtracts it, below 0.0 the amount is 0.0, +0x2C4 null
/// and the state 0; states 4, 5 and 6 (the activation, 004E4690 and vt+0x8C fade-outs) are stops. Then `005EE1B0`, and the
/// answer is whether the state is 3 to 6 (`sub 3`, unsigned `<= 3`). Precision: 24-bit, each step a float. Completion: 100%
/// but states 4 to 6.
/// TODO: [ ] States 4 (TESObjectREFR_Activate on +0x2C4, 00628630), 5 (004E4690) and 6 (vt+0x8C(1)).
/// # Errors
/// Those states, a process without its High part, or a stop in `005EE1B0`.
pub fn actor_update_fade(
    world: &mut World,
    process: Entity,
    actor: Entity,
) -> Result<bool, GameError> {
    let high = world
        .get::<HighProcess>(process)
        .ok_or(GameError::unclassified(
            0x00633250,
            "00633250: a process with no High part",
        ))?;
    let (state, amount) = (high.field_2bc, high.float_2c0);
    if state == 0 {
        if amount < 1.0 {
            world
                .get_mut::<HighProcess>(process)
                .expect("tested")
                .float_2c0 = 1.0;
            actor_update_alpha(world, actor)?;
        }
        return Ok(false);
    }
    let divisor = if Some(actor) == the_player(world) {
        setting_float(world, PLAYER_TELEPORT_FADE_SECONDS_B367B0)?
    } else {
        setting_float(world, ACTOR_TELEPORT_FADE_SECONDS_B367A8)?
    };
    let step = frame_time(world) / divisor;
    {
        let mut high = world.get_mut::<HighProcess>(process).expect("tested");
        match state {
            1 | 3 => {
                let next = step + high.float_2c0;
                high.float_2c0 = next;
                if next > 1.0 {
                    high.float_2c0 = 1.0;
                    high.field_2bc = 0;
                }
            }
            2 => {
                let next = high.float_2c0 - step;
                high.float_2c0 = next;
                if next < 0.0 {
                    high.float_2c0 = 0.0;
                    high.field_2c4 = 0;
                    high.field_2bc = 0;
                }
            }
            4 => {
                return Err(GameError::unclassified(
                    0x00633250,
                    "00633250: the fade-out to an activation (state 4)",
                ));
            }
            5 => {
                return Err(GameError::unclassified(
                    0x00633250,
                    "00633250: the fade-out to 004E4690 (state 5)",
                ));
            }
            6 => {
                return Err(GameError::unclassified(
                    0x00633250,
                    "00633250: the fade-out to vt+0x8C (state 6)",
                ));
            }
            _ => {}
        }
    }
    actor_update_alpha(world, actor)?;
    let state = world.get::<HighProcess>(process).expect("tested").field_2bc;
    Ok(state.wrapping_sub(3) <= 3)
}

/// IDA: 005EE1B0 (thiscall actor), the actor's alpha. With the image-space globals and a refraction extra (0x51), vt+0x9C
/// (a stop; the extra's absence takes the other arm whatever the globals). Otherwise: c = 1 - chameleon (AV 46, the float
/// getter) / 100, taken as 1.0 above 1.0 or unordered and 0.0 below 0.0; i = 0.0 with invisibility (AV 47) above 0, else 1.0;
/// a High process (level 0) gives its +0x154 and +0x2C0, and i = 1.0 when its +0x2BC (vt+0x47C) is 2; the alpha is
/// ((i x c) x +0x2C0) x +0x154. For the player: invisible and below `fMagicPlayerMinimumInvisibility`, that minimum; then times
/// 00B14E50. The mount 00B3BB00 fades takes 00B14E54. Then the actor's vt+0x324(alpha). Precision: 24-bit. Completion: 100%
/// but the refraction arm.
/// TODO: [ ] The refraction arm (vt+0x9C with the extra's +0x0C).
/// # Errors
/// The refraction arm, an actor value that stops, or a stop in vt+0x324.
pub fn actor_update_alpha(world: &mut World, actor: Entity) -> Result<(), GameError> {
    if world
        .get::<TesObjectRefr>(actor)
        .is_some_and(|refr| refr.extra_data_44.get(EXTRA_REFRACTION).is_some())
    {
        return Err(GameError::unclassified(
            0x005EE1B0,
            "005EE1B0: the refraction extra's arm (vt+0x9C)",
        ));
    }
    let chameleon = 1.0
        - crate::tes::ai_process::actor_get_actor_value_float(world, actor, AV_CHAMELEON)? / 100.0;
    let chameleon = if chameleon.is_nan() || chameleon > 1.0 {
        1.0
    } else if chameleon >= 0.0 {
        chameleon
    } else {
        0.0
    };
    let invisible = crate::tes::actor::actor_get_actor_value(world, actor, AV_INVISIBILITY)? > 0;
    let mut visibility: f32 = if invisible { 0.0 } else { 1.0 };
    let (mut amount, mut actor_alpha) = (1.0_f32, 1.0_f32);
    if let Some(process) = process_of(world, actor)
        && process_level(world, process) == 0
    {
        actor_alpha = world
            .get::<MiddleHighProcess>(process)
            .map_or(1.0, |middle| middle.actor_alpha_154);
        let high = world
            .get::<HighProcess>(process)
            .ok_or(GameError::unclassified(
                0x005EE1B0,
                "005EE1B0: a High process with no High part",
            ))?;
        amount = high.float_2c0;
        if high.field_2bc == 2 {
            visibility = 1.0;
        }
    }
    let mut alpha = visibility * chameleon * amount * actor_alpha;
    let globals = world
        .get_resource::<super::player_view::PlayerCameraFadeGlobals>()
        .copied()
        .unwrap_or_default();
    if Some(actor) == the_player(world) {
        let minimum = setting_float(world, MAGIC_PLAYER_MINIMUM_INVISIBILITY_B38000)?;
        if invisible && minimum > alpha {
            alpha = minimum;
        }
        alpha *= globals.player_fade_b14e50;
    }
    if Some(actor) == globals.fading_mount_b3bb00 {
        alpha = globals.mount_fade_b14e54;
    }
    actor_set_fade_alpha(world, actor, alpha)
}

// IDA: 005E5FC0 Actor_SetFadeAlpha, shared node processing with 0066E950.
// Completion: update, sorted insertion (006FF570) and removal of the named MAL extra.
fn set_node_max_alpha(world: &mut World, node: Entity, alpha: f32) -> Result<(), GameError> {
    use crate::bs::fade_node::FadeNodeMaxAlphaExtraData;
    use crate::gamebryo::object_net::{NiExtraDataName, NiObjectNet};
    use crate::gamebryo::stream_objects::NiClass;
    let found = crate::gamebryo::object_net::find_extra_data(world, node, b"MAL")?;
    if alpha < 1.0 {
        if let Some(extra) = found {
            if let Some(mut value) = world.get_mut::<FadeNodeMaxAlphaExtraData>(extra) {
                value.0 = alpha;
            }
            // A different class with this name wins the later duplicate-name insertion.
            return Ok(());
        }
        let index = world
            .get::<NiObjectNet>(node)
            .unwrap()
            .extra_data
            .partition_point(|&e| {
                world
                    .get::<NiExtraDataName>(e)
                    .and_then(|n| n.0.as_deref())
                    .is_some_and(|n| n < b"MAL".as_slice())
            });
        let extra = world
            .spawn((
                NiClass::FadeNodeMaxAlphaExtraData,
                NiExtraDataName(Some(b"MAL".to_vec())),
                FadeNodeMaxAlphaExtraData(alpha),
            ))
            .id();
        world
            .get_mut::<NiObjectNet>(node)
            .unwrap()
            .extra_data
            .insert(index, extra);
    } else if let Some(extra) = found {
        world
            .get_mut::<NiObjectNet>(node)
            .unwrap()
            .extra_data
            .retain(|&e| e != extra);
        if world.get::<FadeNodeMaxAlphaExtraData>(extra).is_some() {
            world.despawn(extra);
        }
    }
    Ok(())
}

/// IDA: the actor's vt+0x324(alpha): `005E5FC0` for an Actor, Character or Creature on its vt+0x154 node (`00612220`
/// Character_GetNiNode: +0x3C, null for a non-player whose node has flag bit 0); the player's `0066E950` on +0x3C directly,
/// then its first-person node (+0x5D0) as a NiNode. On the node: below 1.0 the "MAL" FadeNodeMaxAlphaExtraData takes the alpha
/// (one is added when there's none); at 1.0 or more "MAL" is removed. On the player's first-person node: below 1.0 a
/// NiAlphaProperty with bit 0 set is attached when it has none; at 1.0 or more its alpha property is removed; then
/// NiAVObject_RefreshPropertyState and `004A2A90`(node, alpha). Completion: both alpha ranges and the first-person property dispatch.
/// # Errors
/// A stop in `004A2A90` or the property refresh.
pub fn actor_set_fade_alpha(world: &mut World, actor: Entity, alpha: f32) -> Result<(), GameError> {
    let player = Some(actor) == the_player(world);
    let node = if player {
        world
            .get::<crate::tes::reference_3d::Reference3D>(actor)
            .map(|n| n.0)
    } else {
        crate::tes::forms_init_world::reference_ni_node(world, actor)
    };
    if let Some(node) = node {
        set_node_max_alpha(world, node, alpha)?;
    }
    if !player {
        return Ok(());
    }
    let first_person = world
        .get::<super::player::PlayerCharacter>(actor)
        .and_then(|body| body.first_person_node_5d0)
        .filter(|&node| {
            world
                .get::<crate::gamebryo::scene_node::NiNodeChildren>(node)
                .is_some()
        });
    let Some(first_person) = first_person else {
        return Ok(());
    };
    if alpha < 1.0 {
        if crate::gamebryo::av_object::find_property_of_type(world, first_person, 0)?.is_some() {
            return crate::tes::faders::node_set_children_material_alpha(
                world,
                first_person,
                alpha,
            );
        }
        use crate::gamebryo::{
            object_net::{AlphaProperty, NiObjectNet},
            stream_objects::NiClass,
        };
        let mut alpha_property = AlphaProperty::default();
        alpha_property.flags_18 |= 1;
        let property = world
            .spawn((
                NiClass::AlphaProperty,
                NiObjectNet::default(),
                alpha_property,
            ))
            .id();
        crate::gamebryo::scene_state::attach_object_property(world, first_person, Some(property))?;
    } else {
        crate::gamebryo::av_object::detach_property_of_type(world, first_person, 0)?;
    }
    let defaults = world
        .get_resource::<crate::gamebryo::registration::NiProcessDefaults>()
        .map(crate::gamebryo::registration::NiProcessDefaults::scene_property_defaults)
        .unwrap_or_default();
    crate::gamebryo::scene_state::refresh_property_state(world, first_person, &defaults)?;
    crate::tes::faders::node_set_children_material_alpha(world, first_person, alpha)
}
