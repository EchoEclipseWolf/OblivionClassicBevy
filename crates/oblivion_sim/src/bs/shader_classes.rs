//! Phase 05 row 05-D-4 (b..d): the case table of the BSShader slot classes 007B4290 creates. Each class keeps its
//! original layout as named members (offsets in the comments), its descriptor tables (`shader_tables`, generated from
//! the exact stack images) and its program and pass setup in the original call order.

use std::sync::Arc;

use crate::bs::shader::{BsShaderContext, BsShaderFactory, BsShaderSlot, BsShaderSlots};
use crate::bs::shader_classes_geometry::{
    create_bolt_slot, create_debug_slot, create_distant_lod_slot, create_geometry_decal_slot,
    create_particle_slot, create_precipitation_slot, create_sky_slot, create_speedtree_frond_slot,
    create_speedtree_leaf_slot, create_tallgrass_slot, create_water_shader_slot,
};
use crate::bs::shader_classes_image::{
    create_blur_p20_slot, create_blur_slot, create_copy_slot, create_gethit_slot, create_hdr_slot,
    create_map_slot, create_menu_bg_slot, create_nighteye_slot, create_refraction_slot,
    create_water_displacement_slot, create_water_height_map_slot,
};
use crate::bs::shader_classes_lighting::{
    create_hair_slot, create_lighting30_slot, create_parallax_slot, create_shadow_light_slot,
    create_skin_slot, create_speedtree_branch_slot,
};

// A factory that never reaches another slot.
macro_rules! plain {
    ($factory:path) => {
        |context: &mut BsShaderContext, _: &mut BsShaderSlots| -> Arc<BsShaderSlot> {
            $factory(context)
        }
    };
}

// IDA: the case table of 007B4290 (index -> factory; render-path variants
// handled in get_bs_shader_slot). Classes not yet ported return nullptr
// factories and are reported by the caller.
#[must_use]
pub fn bs_shader_factory(index: u32, blur_below_path_2: bool) -> Option<BsShaderFactory> {
    Some(match index {
        1 => plain!(create_shadow_light_slot),
        2 => plain!(create_tallgrass_slot),
        3 => plain!(create_distant_lod_slot),
        4 => plain!(create_speedtree_branch_slot),
        5 => plain!(create_speedtree_frond_slot),
        6 => plain!(create_speedtree_leaf_slot),
        7 => {
            if blur_below_path_2 {
                plain!(create_blur_slot)
            } else {
                plain!(create_blur_p20_slot)
            }
        }
        8 => plain!(create_hdr_slot),
        9 => plain!(create_debug_slot),
        10 => plain!(create_sky_slot),
        11 => plain!(create_gethit_slot),
        12 => plain!(create_copy_slot),
        13 => create_hair_slot,
        14 => create_skin_slot,
        15 => create_parallax_slot,
        16 => plain!(create_geometry_decal_slot),
        17 => plain!(create_water_shader_slot),
        18 => plain!(create_nighteye_slot),
        19 => plain!(create_water_height_map_slot),
        20 => plain!(create_water_displacement_slot),
        21 => plain!(create_refraction_slot),
        22 => plain!(create_particle_slot),
        23 => plain!(create_bolt_slot),
        24 => plain!(create_map_slot),
        25 => plain!(create_menu_bg_slot),
        26 => plain!(create_lighting30_slot),
        27 => plain!(create_precipitation_slot),
        _ => return None,
    })
}
