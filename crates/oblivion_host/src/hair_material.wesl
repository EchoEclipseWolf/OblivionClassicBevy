// IDA: N/A the hair's draw (Build 5): pass 303's pixel program, SM3003 (shaderpackage019, the HAIR + SPECULAR variant of
// SM3Lighting.p.hlsl, read from its bytecode with `tools/original_debug/d3d9_disasm.py`). 00862B40 Lighting30Shader_GetPassForType's case 303 picks program 3
// over SM3002 (program 2) when 007FF4A0's flag is set: g_rendererShaderFlags & 8 (bDoSpecularPass) and the property's +0x1C
// bit 0, which 007DA220 BSShaderPPLightingProperty_SetupGeometry sets (| 0x40001) for a NiMaterialProperty named "hair" on a geometry with no controller; the race
// menu's hair (Style05.nif's material "Hair") takes it. Its terms, in the numbers the textures hold:
//   n = normalize(2 (NormalMap.rgb - 0.5));  A = normalize(0.5 n + (0, 0, 1));  S = normalize(A - (0.05, 0, 0))
//   per light, in tangent space (T, B, N from the TANGENT, BINORMAL and NORMAL streams): L, V = eye - position, H = norm(L + V)
//     diffuse += colour x max(n.L x window, 0)                              (window = 1 - saturate(d / r)^2; 1 for light 0)
//     spec    += colour x window x (k x Aniso(S.L, S.H).a + 0.7 x Aniso(A.L, A.H).a) x max(L.z, 0)
//       k = 2t for the directional light 0, 0.3t + 0.2 for the point lights, t = 0.5 + green x (HairTint - 0.5)
//   colour = (diffuse + ambient) x 2t x lerp(BaseMap, LayerMap, LayerMap.a) + NormalMap.a x green x spec
//   then the fog; alpha = BaseMap.a x MatAlpha (1.0 here).
// The vertex green comes in the second UV set (`uv_b.x`), not as the vertex colour, which StandardMaterial would multiply in.
// The binormal is rebuilt as tangent.w x (normal x tangent) (Bevy's convention) where D3D9 interpolates 007B7710's own +0x10
// stream: the same vector at each vertex for the generated tangent space, not between them. The lights are Bevy's, read from
// its clustered and view buffers as `world_lighting.wesl` reads them; the ambient and the fog are that file's.

import bevy_pbr::render::{
    pbr_fragment::pbr_input_from_standard_material,
    forward_io::{VertexOutput, FragmentOutput},
    mesh_view_bindings as view_bindings,
    clustered_forward as clustering,
};

// @world_lighting

struct HairParams {
    // HairTint (c2).
    tint: vec4<f32>,
    // The NiAlphaProperty's test: x on, y the function (bits 10-12), z the reference (0..255).
    alpha_test: vec4<f32>,
    // x the normal map, y the layer map, z the aniso map were found (1.0) or not (0.0).
    maps: vec4<f32>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(100) var<uniform> hair: HairParams;
@group(constants::MATERIAL_BIND_GROUP) @binding(101) var hair_normal_map: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(102) var hair_normal_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(103) var hair_layer_map: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(104) var hair_layer_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(105) var hair_aniso_map: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(106) var hair_aniso_sampler: sampler;

// The two aniso lookups' alphas: (Aniso(A.L, A.H).a, Aniso(S.L, S.H).a). The coordinates are computed, so the level is 0 (D3D9
// takes a mip from their screen gradients); the sampler wraps, as the stage defaults do.
fn aniso(a: vec3<f32>, s: vec3<f32>, l: vec3<f32>, h: vec3<f32>) -> vec2<f32> {
    if hair.maps.z < 0.5 {
        return vec2<f32>(0.0);
    }
    let first = textureSampleLevel(hair_aniso_map, hair_aniso_sampler, vec2<f32>(dot(a, l), dot(a, h)), 0.0).a;
    let second = textureSampleLevel(hair_aniso_map, hair_aniso_sampler, vec2<f32>(dot(s, l), dot(s, h)), 0.0).a;
    return vec2<f32>(first, second);
}

// D3D9's alpha test on the 8-bit output alpha: NiAlphaProperty's functions ALWAYS, LESS, EQUAL, LESSEQUAL, GREATER, NOTEQUAL,
// GREATEREQUAL, NEVER (0..7), the alpha on the left.
fn alpha_passes(alpha: f32) -> bool {
    if hair.alpha_test.x < 0.5 {
        return true;
    }
    let a = floor(clamp(alpha, 0.0, 1.0) * 255.0 + 0.5);
    let r = hair.alpha_test.z;
    switch u32(hair.alpha_test.y) {
        case 0u: { return true; }
        case 1u: { return a < r; }
        case 2u: { return a == r; }
        case 3u: { return a <= r; }
        case 4u: { return a > r; }
        case 5u: { return a != r; }
        case 6u: { return a >= r; }
        default: { return false; }
    }
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    let pbr_input = pbr_input_from_standard_material(in, is_front);
    let uv = in.uv;
    let green = in.uv_b.x;
    let base = vec4<f32>(to_stored(pbr_input.material.base_color.rgb), pbr_input.material.base_color.a);
    let normal_texel = textureSample(hair_normal_map, hair_normal_sampler, uv);
    let layer_texel = textureSample(hair_layer_map, hair_layer_sampler, uv);
    if !alpha_passes(base.a) {
        discard;
    }

    // The tangent frame (v2, v3, v4, each normalised as the program does).
    let geometric = normalize(in.world_normal);
    // The view always gives the hair a TANGENT stream (+X with none made), so VERTEX_TANGENTS is defined.
    let tangent = normalize(in.world_tangent.xyz);
    let binormal = normalize(in.world_tangent.w * cross(geometric, tangent));
    let to_tangent = transpose(mat3x3<f32>(tangent, binormal, geometric));

    // A flat normal with no specular mask when the normal map is missing.
    let has_normal = hair.maps.x > 0.5;
    let n = select(vec3<f32>(0.0, 0.0, 1.0), normalize(2.0 * (normal_texel.rgb - vec3<f32>(0.5))), has_normal);
    let mask = select(0.0, normal_texel.a, has_normal);
    let a = normalize(0.5 * n + vec3<f32>(0.0, 0.0, 1.0));
    let s = normalize(a - vec3<f32>(0.05, 0.0, 0.0));

    let world_position = pbr_input.world_position;
    let v = normalize(to_tangent * (view_bindings::view.world_position.xyz - world_position.xyz));
    let t = vec3<f32>(0.5) + green * (hair.tint.rgb - vec3<f32>(0.5));
    let exposure = view_bindings::view.exposure;

    var diffuse = vec3<f32>(0.0);
    var spec = vec3<f32>(0.0);
    // Light 0, the directional light (c9/c10): L in tangent space, no window, the spec weight 2t.
    for (var i: u32 = 0u; i < view_bindings::lights.n_directional_lights; i = i + 1u) {
        let directional = view_bindings::lights.directional_lights[i];
        let colour = directional.color.rgb * exposure;
        let l = to_tangent * directional.direction_to_light;
        let h = normalize(l + v);
        let lobes = aniso(a, s, l, h);
        diffuse += max(dot(n, l), 0.0) * colour;
        spec += colour * (2.0 * t * lobes.y + vec3<f32>(0.7 * lobes.x)) * max(dot(geometric, directional.direction_to_light), 0.0);
    }
    // The point lights (c(9+2i) colour, c(10+2i) position and radius).
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
        let window = 1.0 - ratio * ratio;
        let l = normalize(to_tangent * to_light);
        let h = normalize(v + l);
        let lobes = aniso(a, s, l, h);
        diffuse += colour * max(dot(n, l) * window, 0.0);
        spec += window * colour * ((0.3 * t + vec3<f32>(0.2)) * lobes.y + vec3<f32>(0.7 * lobes.x)) * max(l.z, 0.0);
    }

    // lrp r1, layer.a, layer, base: an unbound layer stage samples 0, which keeps the base map.
    let layer = select(vec4<f32>(0.0), layer_texel, hair.maps.y > 0.5);
    let coloured = 2.0 * t * mix(base.rgb, layer.rgb, layer.a);
    let ambient = view_bindings::lights.ambient_color.rgb * exposure;
    let lit = (diffuse + ambient) * coloured + mask * green * spec;
    let stored = clamp(game_fog(lit, world_position), vec3<f32>(0.0), vec3<f32>(1.0));
    var out: FragmentOutput;
    out.color = vec4<f32>(to_linear(stored), base.a);
    return out;
}
