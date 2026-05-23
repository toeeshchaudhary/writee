// Textured-quad pipeline for ImageBlock.
//
// Each draw call uses a unique sampled texture bound to group 1.
// Vertices carry world-space position + UV; viewport transform matches the
// ink pipeline so quads pan/zoom with the canvas.

struct Viewport {
    offset: vec2<f32>,
    zoom: f32,
    _pad0: f32,
    screen: vec2<f32>,
    _pad1: vec2<f32>,
    bg_color: vec4<f32>,
    dot_color: vec4<f32>,
};

@group(0) @binding(0) var<uniform> vp: Viewport;
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var samp: sampler;

struct VsIn {
    @location(0) world: vec2<f32>,
    @location(1) uv: vec2<f32>,
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    let pixel = (in.world - vp.offset) * vp.zoom;
    let ndc = vec2<f32>(
        pixel.x / vp.screen.x * 2.0 - 1.0,
        1.0 - pixel.y / vp.screen.y * 2.0,
    );
    var o: VsOut;
    o.clip = vec4<f32>(ndc, 0.0, 1.0);
    o.uv = in.uv;
    return o;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return textureSample(tex, samp, in.uv);
}
