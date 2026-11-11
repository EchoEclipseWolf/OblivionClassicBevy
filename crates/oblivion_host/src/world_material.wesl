// IDA: N/A the world's draw (Track R3): StandardMaterial's base map, alpha test and normal, shaded by `world_lighting.wesl`
// (Bevy's lights with the game's falloff, in the game's numbers), returned linear so the world image's sRGB store gives back
// the game's numbers (`world_lighting.wesl`).

import bevy_pbr::render::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::alpha_discard,
    forward_io::{VertexOutput, FragmentOutput},
    mesh_view_bindings as view_bindings,
    clustered_forward as clustering,
};

// @world_lighting

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    pbr_input.material.base_color = alpha_discard(
        pbr_input.material.flags,
        pbr_input.material.alpha_cutoff,
        pbr_input.material.base_color,
    );
    var out: FragmentOutput;
    let shaded = shade(pbr_input.material.base_color.rgb, pbr_input.world_position, pbr_input.N, pbr_input.frag_coord, pbr_input.is_orthographic);
    out.color = vec4<f32>(shaded, pbr_input.material.base_color.a);
    return out;
}
