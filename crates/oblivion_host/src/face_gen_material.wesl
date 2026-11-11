// IDA: N/A the FaceGen face's and ears' draw (Build 5 goal 6): SkinShader's passes, as run-bi caught them for FaceGenFace
// (105, 236, 266, 294; the ears 100, 233, 261, 291) and shaderpackage019's bytecode gives them:
//   105 SKIN2013.vso / SKIN2006.pso (opaque, Z LESSEQUAL): AmbientColor c1 plus, for the directional light (c2) and the first
//       two point lights (c3, c4), (max(N.L, 0) + 0.5 x (1 - max(N.E, 0))^3) x colour x attenuation, N the normal map's
//       (2 x (texel - 0.5), normalised; the property's +0xC0[0], the age map's `_n`), L and E in tangent space; the attenuation
//       is the radial falloff map's 1 - (d/r)^2 (00B43110, 128 x 128 of min(fx^2 + fy^2, 1), sampled twice), saturated.
//   236 SKIN2017.vso / SKIN2008.pso (ONE/ONE, Z EQUAL): the same for the next two point lights, no ambient.
//   266 SLS1S007.vso / SLS1005.pso (DESTCOLOR/ZERO): the target times 2 x FaceGenMap1 x (base + 2 x (FaceGenMap0 - 0.5)),
//       doubled: SM3006's FACEGENBLEND product, here multiplied into the lit target. The 8-bit target clamps the light sum and
//       the blend's source to [0, 1] first.
//   294 SLS2036.vso / SLS2041.pso (ONE/ONE, Z EQUAL), once per point light: pow(saturate(N.H), Toggles c7.z) x the normal
//       map's alpha, taken x max(N.L + 0.5, 0) where N.L <= 0.2, x colour x attenuation, saturated; H = normalize(E + L).
// All in the numbers the textures hold, as D3D9 worked; returned linear for the world image's sRGB store.
// Host substitutions: one pass where the original blends four through an 8-bit target (no quantisation between them); the
// per-pixel world-space frame where the programs normalise per-vertex tangent-space vectors and interpolate them; Bevy's
// clustered lights where the pass holds the accumulator's chosen ones; the analytic falloff for the falloff map's texels; no
// fog (the skin programs output none).

import bevy_pbr::render::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::alpha_discard,
    forward_io::{VertexOutput, FragmentOutput},
    mesh_view_bindings as view_bindings,
    clustered_forward as clustering,
};

// @world_lighting

struct FaceGenParams {
    // x: 1.0 when FaceGenMap1 is a texture, else 0.0 for g_faceGen's grey; y: that grey's number (0x40 / 255).
    map1: vec4<f32>,
}

struct SkinParams {
    // x: 1.0 when the normal map was found (else the flat (0x80, 0x80, 0xFF), alpha 1.0); y: the specular exponent.
    skin: vec4<f32>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(100) var<uniform> face_gen: FaceGenParams;
@group(constants::MATERIAL_BIND_GROUP) @binding(101) var face_gen_map0: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(102) var face_gen_map0_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(103) var face_gen_map1: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(104) var face_gen_map1_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(105) var<uniform> skin: SkinParams;
@group(constants::MATERIAL_BIND_GROUP) @binding(106) var skin_normal_map: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(107) var skin_normal_sampler: sampler;

// SKIN2006/2008's per-light term: (max(N.L, 0) + 0.5 x rim) x colour.
fn skin_term(n: vec3<f32>, l: vec3<f32>, rim: f32, colour: vec3<f32>) -> vec3<f32> {
    return (max(dot(n, l), 0.0) + 0.5 * rim) * colour;
}

// SLS2041's specular: pow(saturate(N.H), power) x gloss, softened by max(N.L + 0.5, 0) where N.L <= 0.2.
fn skin_specular(n: vec3<f32>, l: vec3<f32>, e: vec3<f32>, gloss: f32) -> f32 {
    let h = normalize(e + l);
    let s = pow(clamp(dot(n, h), 0.0, 1.0), skin.skin.y) * gloss;
    let n_dot_l = dot(n, l);
    return select(s, max(n_dot_l + 0.5, 0.0) * s, 0.2 - n_dot_l >= 0.0);
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    let uv = in.uv;
    let map0 = textureSample(face_gen_map0, face_gen_map0_sampler, uv).rgb;
    let sampled = to_stored(textureSample(face_gen_map1, face_gen_map1_sampler, uv).rgb);
    let map1 = select(vec3<f32>(face_gen.map1.y), sampled, face_gen.map1.x > 0.5);
    let base = to_stored(pbr_input.material.base_color.rgb);
    let normal_texel = textureSample(skin_normal_map, skin_normal_sampler, uv);
    pbr_input.material.base_color = alpha_discard(
        pbr_input.material.flags,
        pbr_input.material.alpha_cutoff,
        pbr_input.material.base_color,
    );

    // The tangent frame; the view gives every FaceGen part a TANGENT stream (+X with none made).
    let geometric = normalize(in.world_normal);
    let tangent = normalize(in.world_tangent.xyz);
    let binormal = normalize(in.world_tangent.w * cross(geometric, tangent));
    let to_tangent = transpose(mat3x3<f32>(tangent, binormal, geometric));
    let has_normal = skin.skin.x > 0.5;
    let n = select(vec3<f32>(0.0, 0.0, 1.0), normalize(2.0 * (normal_texel.rgb - vec3<f32>(0.5))), has_normal);
    let gloss = select(1.0, normal_texel.a, has_normal);

    let world_position = pbr_input.world_position;
    let e = normalize(to_tangent * (view_bindings::view.world_position.xyz - world_position.xyz));
    let facing = 1.0 - max(dot(n, e), 0.0);
    let rim = facing * facing * facing;
    let exposure = view_bindings::view.exposure;

    // Passes 105 and 236: AmbientColor (min(emissive + ambient, 1), 00848C40) and the lights.
    var light = min(view_bindings::lights.ambient_color.rgb, vec3<f32>(1.0)) * exposure;
    var spec = vec3<f32>(0.0);
    for (var i: u32 = 0u; i < view_bindings::lights.n_directional_lights; i = i + 1u) {
        let directional = view_bindings::lights.directional_lights[i];
        let l = normalize(to_tangent * directional.direction_to_light);
        light += skin_term(n, l, rim, directional.color.rgb * exposure);
    }
    let view_z = dot(vec4<f32>(
        view_bindings::view.view_from_world[0].z,
        view_bindings::view.view_from_world[1].z,
        view_bindings::view.view_from_world[2].z,
        view_bindings::view.view_from_world[3].z
    ), world_position);
    let cluster_index = clustering::view_fragment_cluster_index(pbr_input.frag_coord.xy, view_z, pbr_input.is_orthographic);
    let ranges = clustering::unpack_clusterable_object_index_ranges(cluster_index);
    for (var i: u32 = ranges.first_point_light_index_offset; i < ranges.first_spot_light_index_offset; i = i + 1u) {
        let id = clustering::get_clusterable_object_id(i);
        let data = view_bindings::clustered_lights.data[id];
        let colour = data.color_inverse_square_range.rgb * exposure;
        let to_light = data.position_radius.xyz - world_position.xyz;
        let d = length(to_light);
        let r = inverseSqrt(max(data.color_inverse_square_range.w, 1e-12));
        let ratio = clamp(d / r, 0.0, 1.0);
        let attenuation = clamp(1.0 - ratio * ratio, 0.0, 1.0);
        let l = normalize(to_tangent * to_light);
        light += skin_term(n, l, rim, colour) * attenuation;
        // Pass 294, one per point light, each saturated by the target.
        spec += clamp(skin_specular(n, l, e, gloss) * colour * attenuation, vec3<f32>(0.0), vec3<f32>(1.0));
    }
    let lit = clamp(light, vec3<f32>(0.0), vec3<f32>(1.0));

    // Pass 266: the texture product, clamped as the blend's source, times the lit target.
    let texture = clamp((base + 2.0 * (map0 - vec3<f32>(0.5))) * (2.0 * map1) * 2.0, vec3<f32>(0.0), vec3<f32>(1.0));
    let stored = clamp(lit * texture + spec, vec3<f32>(0.0), vec3<f32>(1.0));
    var out: FragmentOutput;
    out.color = vec4<f32>(to_linear(stored), pbr_input.material.base_color.a);
    return out;
}
