//! The two point costs: 005EC1F0 Actor_GetPathPointCost (the actor's own cost of standing on a point) and 0067EDE0
//! PathingLocation_GetPointCost (that, plus the exit penalties and the distance from a location). The x87 runs at 24
//! bits there, so each step is an f32 operation.

use oblivion_core::error::GameError;
use oblivion_core::x87;

use crate::grid::PathGridPoint;

/// IDA: N/A what 005EC1F0 and 0067EDE0 read from outside the grid, gathered by `oblivion_sim` before a query: the
/// reference's base type (a creature, 0x24, takes no preferred-point bonus), 005E1E90's answer, and the static
/// GameSettings fPathPreferredPointBonus (00B3A400), fPathInvalidMovementTypePenalty (00B3A408),
/// fPathWaterExitPenalty (00B3A420) and fPathSpaceExitPenalty (00B3A428). Each is a `Result` so that a missing one
/// stops only where the original reads it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ActorPathFacts {
    pub is_creature: bool,
    /// 005E1E90's answer for the reference.
    pub movement_type_invalid: Result<bool, GameError>,
    pub preferred_point_bonus: Result<f32, GameError>,
    pub invalid_movement_type_penalty: Result<f32, GameError>,
    pub water_exit_penalty: Result<f32, GameError>,
    pub space_exit_penalty: Result<f32, GameError>,
}

/// IDA: 005EC1F0 Actor_GetPathPointCost(reference, point): 1.0, less fPathPreferredPointBonus for a z-odd point when the
/// base isn't a creature. A point with bit 0 or 1 reads its bits 4 and 3; otherwise bit 3 (under water) runs the swim
/// test (005E06C0), a stop here, and 0067ED30 stores the answer in bit 4. With bit 4 the 00519C70 and 005EA640 arms, a
/// stop; without, 005E1E90 adds fPathInvalidMovementTypePenalty, and bit 3 the wading penalty, a stop. Completion: 100%
/// of a dry point for an NPC.
/// TODO: [ ] 005E06C0 for a point under water, the swimming arms (00519C70, 005EA640) and the wading penalty (005E3400).
/// # Errors
/// A stop, or a setting the facts lack.
pub fn actor_point_cost(
    points: &mut [PathGridPoint],
    point: usize,
    facts: &ActorPathFacts,
) -> Result<f32, GameError> {
    let data = points[point].clone();
    let mut cost = 1.0f32;
    if x87::fistp(data.position_14[2]) & 1 != 0 && !facts.is_creature {
        cost = 1.0 - facts.preferred_point_bonus?;
    }
    let flags = data.flags_10;
    let (swimming, wading);
    if flags & 3 != 0 {
        swimming = flags & 0x10 != 0;
        wading = flags & 8 != 0;
    } else {
        if flags & 8 != 0 {
            return Err(GameError::stop(
                0x005EC24F,
                "005EC24F: 005E06C0 for a point under water",
            ));
        }
        swimming = false;
        wading = false;
        points[point].flags_10 &= !0x10;
    }
    if swimming {
        return Err(GameError::stop(
            0x005EC33C,
            "005EC33C: 00519C70 for a swimming point",
        ));
    }
    if facts.movement_type_invalid? {
        cost += facts.invalid_movement_type_penalty?;
    }
    if wading {
        return Err(GameError::stop(
            0x005EC3A0,
            "005EC3A0: 005E3400 for a wading point",
        ));
    }
    Ok(cost)
}

/// IDA: 0067EDE0 PathingLocation_GetPointCost(location, point, reference): 005EC1F0's cost, plus fPathWaterExitPenalty
/// when the two flag bytes differ in bit 3 and fPathSpaceExitPenalty when they differ in bit 6; a cost above 0 adds
/// hkVector4::length3 of location minus point (24 bits, 00404C90), otherwise 0. The location is a 0067D820
/// PathingLocation or, in the A*, the popped point. Completion: 100%.
/// # Errors
/// 005EC1F0's, or a setting the facts lack.
pub fn location_point_cost(
    location_flags: u8,
    location_position: [f32; 3],
    points: &mut [PathGridPoint],
    point: usize,
    facts: &ActorPathFacts,
) -> Result<f32, GameError> {
    let mut cost = actor_point_cost(points, point, facts)?;
    let (flags, at) = (points[point].flags_10, points[point].position_14);
    if (flags >> 3 ^ location_flags >> 3) & 1 != 0 {
        cost += facts.water_exit_penalty?;
    }
    if (flags >> 6 ^ location_flags >> 6) & 1 != 0 {
        cost += facts.space_exit_penalty?;
    }
    if 0.0 >= cost {
        return Ok(0.0);
    }
    let delta = [
        location_position[0] - at[0],
        location_position[1] - at[1],
        location_position[2] - at[2],
    ];
    let length = ((delta[1] * delta[1] + delta[0] * delta[0]) + delta[2] * delta[2]).sqrt();
    Ok(length + cost)
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn npc_facts() -> ActorPathFacts {
        ActorPathFacts {
            is_creature: false,
            movement_type_invalid: Ok(false),
            preferred_point_bonus: Ok(0.25),
            invalid_movement_type_penalty: Ok(1000.0),
            water_exit_penalty: Ok(100.0),
            space_exit_penalty: Ok(50.0),
        }
    }

    #[test]
    fn a_dry_point_costs_one_less_the_bonus_when_z_is_odd() {
        let mut points = vec![
            PathGridPoint {
                position_14: [0.0, 0.0, -465.0],
                flags_10: 0x10,
                ..PathGridPoint::default()
            },
            PathGridPoint {
                position_14: [3.0, 4.0, -466.0],
                ..PathGridPoint::default()
            },
        ];
        let facts = npc_facts();
        assert_eq!(actor_point_cost(&mut points, 0, &facts), Ok(0.75));
        assert_eq!(
            points[0].flags_10, 0,
            "005EC1F0 clears bit 4 on a point not yet searched"
        );
        assert_eq!(actor_point_cost(&mut points, 1, &facts), Ok(1.0));
        // From (0, 0, -466): 1.0 + |(-3, -4, 0)|.
        assert_eq!(
            location_point_cost(0, [0.0, 0.0, -466.0], &mut points, 1, &facts),
            Ok(6.0)
        );
        // The subspace bit differs: the space exit penalty.
        assert_eq!(
            location_point_cost(0x40, [0.0, 0.0, -466.0], &mut points, 1, &facts),
            Ok(56.0)
        );
        let creature = ActorPathFacts {
            is_creature: true,
            ..facts
        };
        assert_eq!(actor_point_cost(&mut points, 0, &creature), Ok(1.0));
        points[1].flags_10 = 8;
        let error = actor_point_cost(&mut points, 1, &facts).unwrap_err();
        assert_eq!(error.kind, oblivion_core::error::GameErrorKind::Stop);
        assert_eq!(error.site, 0x005E_C24F);
    }
}
