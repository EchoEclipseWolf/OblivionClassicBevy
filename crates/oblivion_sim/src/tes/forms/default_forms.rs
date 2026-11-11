//! Phase 12d: 0044AC20 DataHandler_CreateDefaultForms, called at 0044F6CB after the compile pass and before the record loop. Each default looks its ID up in g_formsByID, most through a cast, and when absent constructs the form (FormHeap and the class constructor), gives it its ID (0046C300(id, 1)), its name (vt+0xD8), the class defaults and its list; most then take vt+0x90(0). The record loop later loads the master over the forms it also defines.
//!
//! The function, with its order read from the exe 0044AC56..0044CD71, and its arms are `DataHandler`'s and live in `tes/data/data_handler.rs`; this file holds the statics it stores. P12-RUN-001's AC20 rows check the forms; no row can see the order between defaults. Evidence: objdump of 0044AC20, 0041BB80, 00518000.

use bevy_ecs::prelude::*;

/// IDA: the statics 0044AC20 stores, cleared at 0044AC5D..0044ACD5 where it says, then set to the form found or created.
#[derive(Resource, Debug, Default)]
pub struct DefaultFormStatics {
    pub door_marker_b35ea4: Option<Entity>,
    pub map_marker_b35ea8: Option<Entity>,
    pub x_marker_b35eac: Option<Entity>,
    pub x_marker_heading_b35eb0: Option<Entity>,
    pub travel_marker_b35eb4: Option<Entity>,
    pub north_marker_b35eb8: Option<Entity>,
    pub prison_marker_b35ebc: Option<Entity>,
    pub temple_marker_b35ec0: Option<Entity>,
    pub divine_marker_b35ec4: Option<Entity>,
    pub lockpick_b35ec8: Option<Entity>,
    pub skeleton_key_b35ecc: Option<Entity>,
    pub repair_hammer_b35ed0: Option<Entity>,
    pub horse_marker_b35ed4: Option<Entity>,
    pub welkynd_stone_b35ed8: Option<Entity>,
    pub varla_stone_b35edc: Option<Entity>,
    pub black_soul_gem_b35ee0: Option<Entity>,
    pub azura_stone_b35ee4: Option<Entity>,
    pub default_water_b360ac: Option<Entity>,
    pub creature_faction_b36298: Option<Entity>,
    pub eyes_reanimate_b362bc: Option<Entity>,
    pub vampire_race_b36308: Option<Entity>,
    /// 00B33AA8..00B33AB8: stored here too, but not among the clears.
    pub loot_bag_b33aa8: Option<Entity>,
    pub stolen_goods_b33aac: Option<Entity>,
    pub jail_shirt_b33ab0: Option<Entity>,
    pub jail_pants_b33ab4: Option<Entity>,
    pub jail_shoes_b33ab8: Option<Entity>,
    /// 00B33560..00B335A4, the 18 magic sounds of 0041BB80 in its order.
    pub magic_sounds_b33560: Vec<Option<Entity>>,
    /// 00B335A8 and 00B335AC, the two default spells.
    pub default_spells_b335a8: [Option<Entity>; 2],
    /// 00B335B0..00B335B8, the three effect shaders.
    pub effect_shaders_b335b0: Vec<Option<Entity>>,
    /// The 00518000 sounds, indexed by address from 00B36218. Call order differs (`FOOTSTEP_SOUND_ADDRESSES`), and 006B16E5 reads by address.
    pub footstep_sounds_b36218: [Option<Entity>; 25],
    /// IDA: N/A the calls of 00543200 whose graphics a host hasn't drawn.
    pub sky_climate_calls: u32,
}

/// IDA: 00402860(&g_timeGlobals 00B332E0) the six lookups, which 0044F903 makes after the topics sort.
#[derive(Resource, Debug, Default, PartialEq, Eq)]
pub struct TimeGlobals {
    pub globals_b332e0: [Option<Entity>; 6],
}
