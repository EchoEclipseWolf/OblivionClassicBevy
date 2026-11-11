// IDA: N/A the world's lighting (Track R3, the user's choice 2026-11-08: Bevy's lights with the game's falloff). The lights are
// Bevy entities (point lights clustered by Bevy, directional lights, the camera's AmbientLight and DistanceFog), but each
// carries the game's own numbers as its "linear" colour (`world_view::raw`) times a tuning scale as its intensity, and this
// module shades the way the SM3 programs do (shaderpackage019's SM3000.pso, read from its bytecode; run-ap's constants):
//   colour = base x (AmbientColor + sum over lights of max(N.L, 0) x (1 - saturate(d / r)^2) x LightColor)
//   then fog: lerp to FogColor by 1 - saturate((far - d) / (far - near))
// in the numbers the textures hold (gamma space, as D3D9 multiplied them). The result is returned linear: the world camera
// renders into an `Rgba8UnormSrgb` image (`world_view::spawn_world_composite`), whose store encodes it back to the game's
// numbers on D3D9's 8-bit gamma grid. Bevy's exposure (the tuning's stops) scales the light.

// This text has no imports of its own: `world_view` pastes it into each world shader at its `// @world_lighting` line, after
// that shader's imports, which must include `mesh_view_bindings as view_bindings` and `clustered_forward as clustering`.

fn to_stored(linear: vec3<f32>) -> vec3<f32> {
    let low = linear * 12.92;
    let high = 1.055 * pow(max(linear, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(high, low, linear <= vec3<f32>(0.0031308));
}

fn to_linear(stored: vec3<f32>) -> vec3<f32> {
    let low = stored / 12.92;
    let high = pow((max(stored, vec3<f32>(0.0)) + 0.055) / 1.055, vec3<f32>(2.4));
    return select(high, low, stored <= vec3<f32>(0.04045));
}

// The light sum in the game's numbers: the ambient plus each light's N.L times the window (1 - (d/r)^2).
fn game_light(world_position: vec4<f32>, normal: vec3<f32>, frag_coord: vec4<f32>, is_orthographic: bool) -> vec3<f32> {
    var light = view_bindings::lights.ambient_color.rgb;
    let view_z = dot(vec4<f32>(
        view_bindings::view.view_from_world[0].z,
        view_bindings::view.view_from_world[1].z,
        view_bindings::view.view_from_world[2].z,
        view_bindings::view.view_from_world[3].z
    ), world_position);
    let cluster_index = clustering::view_fragment_cluster_index(frag_coord.xy, view_z, is_orthographic);
    let ranges = clustering::unpack_clusterable_object_index_ranges(cluster_index);
    for (var i: u32 = ranges.first_point_light_index_offset; i < ranges.first_spot_light_index_offset; i = i + 1u) {
        let id = clustering::get_clusterable_object_id(i);
        let data = view_bindings::clustered_lights.data[id];
        let to_light = data.position_radius.xyz - world_position.xyz;
        let d = length(to_light);
        // Bevy keeps 1 / range^2; the range is the game's radius (times the tuning's range scale).
        let r = inverseSqrt(max(data.color_inverse_square_range.w, 1e-12));
        let t = clamp(d / r, 0.0, 1.0);
        let n_dot_l = max(dot(normal, to_light / max(d, 1e-6)), 0.0);
        light += data.color_inverse_square_range.rgb * n_dot_l * (1.0 - t * t);
    }
    for (var i: u32 = 0u; i < view_bindings::lights.n_directional_lights; i = i + 1u) {
        let directional = view_bindings::lights.directional_lights[i];
        light += directional.color.rgb * max(dot(normal, directional.direction_to_light), 0.0);
    }
    return light * view_bindings::view.exposure;
}

// The fog in the game's numbers: Bevy's linear DistanceFog carries the start and end (be.x, be.y) and the colour.
fn game_fog(stored: vec3<f32>, world_position: vec4<f32>) -> vec3<f32> {
    let fog = view_bindings::fog;
    if fog.mode != 1u {
        return stored;
    }
    let d = length(world_position.xyz - view_bindings::view.world_position.xyz);
    let span = max(fog.be.y - fog.be.x, 1e-6);
    let amount = 1.0 - clamp((fog.be.y - d) / span, 0.0, 1.0);
    return mix(stored, fog.base_color.rgb, amount);
}

// The whole shade: a base colour (linear, as StandardMaterial gives it) lit and fogged in the game's numbers, returned
// linear for the world image's sRGB store.
fn shade(base_linear: vec3<f32>, world_position: vec4<f32>, normal: vec3<f32>, frag_coord: vec4<f32>, is_orthographic: bool) -> vec3<f32> {
    let base = to_stored(base_linear);
    let lit = clamp(base * game_light(world_position, normal, frag_coord, is_orthographic), vec3<f32>(0.0), vec3<f32>(1.0));
    return to_linear(clamp(game_fog(lit, world_position), vec3<f32>(0.0), vec3<f32>(1.0)));
}
