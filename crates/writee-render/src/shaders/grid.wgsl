// Procedural dot-grid background.
//
// World-space coordinates are derived in the vertex shader from a full-screen
// triangle by inverting the screen<->world transform encoded in the Viewport
// uniform. The fragment shader stamps a circular dot at every integer multiple
// of `spacing` world units. Anti-aliased via fwidth.

struct Viewport {
    // World-space coord at the screen origin (top-left in pixel space).
    offset: vec2<f32>,
    // Pixels per world unit. zoom=1 means 1 world unit == 1 px.
    zoom: f32,
    _pad0: f32,
    // Physical screen size in pixels.
    screen: vec2<f32>,
    _pad1: vec2<f32>,
    // Theme-driven colours.
    bg_color: vec4<f32>,
    dot_color: vec4<f32>,
};

@group(0) @binding(0) var<uniform> vp: Viewport;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) idx: u32) -> VsOut {
    // Full-screen triangle. uv covers [0,2] x [0,2] so the on-screen
    // portion is the [0,1] x [0,1] sub-quad.
    let x = f32((idx << 1u) & 2u);
    let y = f32(idx & 2u);
    let uv = vec2<f32>(x, y);
    let clip = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
    let pixel = vec2<f32>(uv.x * vp.screen.x, uv.y * vp.screen.y);
    let world = vp.offset + pixel / vp.zoom;

    var out: VsOut;
    out.clip = clip;
    out.world = world;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let bg = vp.bg_color.rgb;
    let dot_color = vp.dot_color.rgb;

    // World-space grid spacing. 24 world units between dots.
    let spacing: f32 = 24.0;

    // Distance to the nearest grid intersection, in world units.
    let g = (fract(in.world / spacing + vec2<f32>(0.5)) - vec2<f32>(0.5)) * spacing;
    let d_world = length(g);

    // Constant *screen-space* dot radius (~1.2 px regardless of zoom).
    let r_world = 1.2 / vp.zoom;

    let aa = fwidth(d_world);
    let alpha = 1.0 - smoothstep(r_world - aa, r_world + aa, d_world);

    let col = mix(bg, dot_color, alpha);
    return vec4<f32>(col, 1.0);
}
