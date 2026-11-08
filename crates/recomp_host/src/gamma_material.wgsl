// IDA: N/A D3D9's modulate in the numbers the textures hold. See `GammaMaterial` in screens.rs. Group 2 is bevy_sprite_render's MATERIAL_2D_BIND_GROUP_INDEX.
@group(2) @binding(0) var color_texture: texture_2d<f32>;
@group(2) @binding(1) var color_sampler: sampler;

// The mesh pipeline's output: uv at 2, the vertex colour at 4.
struct Input {
    @location(2) uv: vec2<f32>,
    @location(4) color: vec4<f32>,
}

@fragment
fn fragment(in: Input) -> @location(0) vec4<f32> {
    return in.color * textureSample(color_texture, color_sampler, in.uv);
}
