// Terrain and patch textures share one screen-to-map lookup. Quantize in map
// space before adding the atlas offset, so independent UV interpolation cannot
// choose opposite sides of a texel boundary during camera motion or zoom.
struct ScreenUniform {
    screen_size: vec2<f32>,
    _pad: vec2<f32>,
};
@group(0) @binding(0) var<uniform> screen: ScreenUniform;
@group(1) @binding(0) var sampled_tex: texture_2d<f32>;

struct VsIn {
    @location(0) pos: vec2<f32>,
    @location(1) texture_offset: vec2<f32>,
    @location(2) camera: vec4<f32>,
};
struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) texture_offset: vec2<f32>,
    @location(1) @interpolate(flat) camera: vec4<f32>,
};
@vertex
fn vs_main(v: VsIn) -> VsOut {
    var out: VsOut;
    out.position = vec4<f32>(
        v.pos.x / screen.screen_size.x * 2.0 - 1.0,
        1.0 - v.pos.y / screen.screen_size.y * 2.0, 0.0, 1.0,
    );
    out.texture_offset = v.texture_offset;
    out.camera = v.camera;
    return out;
}
@fragment
fn fs_main(v: VsOut) -> @location(0) vec4<f32> {
    let map_pixel = floor(v.position.xy / v.camera.z + v.camera.xy);
    let texel = vec2<i32>(map_pixel + v.texture_offset);
    let dimensions = vec2<i32>(textureDimensions(sampled_tex));
    // Outside the sampled texture is not map content. Clamping the lookup there
    // would smear the border texel across every uncovered fragment, which is
    // what a viewport larger than the level at the widest zoom exposes. Bound
    // against the bound texture, not a level extent: the atlas bind group keeps
    // in-range decals byte-identical.
    if any(texel < vec2<i32>(0)) || any(texel >= dimensions) {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }
    return textureLoad(sampled_tex, texel, 0);
}
