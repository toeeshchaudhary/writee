// Anti-aliased ink with per-vertex color. World-space triangle strip.
//
// signed_offset / half_width drive the SDF AA (see prior notes); color is an
// unorm8x4 RGBA that the vertex shader interpolates and the fragment shader
// modulates by the SDF alpha.

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

struct VsIn {
    @location(0) world: vec2<f32>,
    @location(1) signed_offset: f32,
    @location(2) half_width: f32,
    @location(3) color: vec4<f32>,
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) signed_off: f32,
    @location(1) half_w: f32,
    @location(2) color: vec4<f32>,
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
    o.signed_off = in.signed_offset;
    o.half_w = in.half_width;
    o.color = in.color;
    return o;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let dist_from_center = abs(in.signed_off);
    let edge = in.half_w - dist_from_center;
    let aa = fwidth(edge);
    let alpha = smoothstep(-aa, aa, edge);
    if (alpha <= 0.001) {
        discard;
    }
    return vec4<f32>(in.color.rgb, in.color.a * alpha);
}
