//! Stroke tessellation: from a noisy sample stream to a triangle strip ready
//! for the GPU.
//!
//! Pipeline:
//!   1. One-Euro filter on (x, y, pressure) to kill jitter.
//!   2. Catmull-Rom resample with a fixed step count per segment.
//!   3. Variable-width perpendicular extrusion → triangle strip vertices.
//!
//! Width law: `width_base * pressure^0.7 * (1 + 0.6 * tilt_magnitude)` when
//! tilt is enabled. The exponent feels like a fountain pen at 0.7; raise toward
//! 1.0 for a ballpoint feel. Tilt magnitude is `(tilt_x^2 + tilt_y^2).sqrt()`
//! pre-clamped to [0, 1] (drivers report tilt in radians or normalised units).

use crate::arrow::Arrow;
use crate::stroke::InkPoint;
use crate::one_euro::{OneEuro, OneEuroParams};
use glam::Vec2;

/// One vertex emitted by [`tessellate`]. The list is a single triangle strip;
/// each consecutive pair of vertices represents one (left, right) cross-section
/// of the stroke.
///
/// `signed_offset` carries the side of the strip (+half_width on one edge,
/// -half_width on the other); `half_width` is the per-vertex half-width. Both
/// varyings interpolate, letting the fragment shader compute a per-pixel edge
/// SDF for anti-aliasing.
///
/// `color` is RGBA u8, uploaded as `Unorm8x4` so the shader gets a vec4<f32>
/// in [0,1]. Lets the editor mix pen + highlighter + selection-marquee in a
/// single draw call.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct InkVertex {
    pub pos: Vec2,
    pub signed_offset: f32,
    pub half_width: f32,
    pub color: [u8; 4],
}

/// Default near-black ink color.
pub const COLOR_INK: [u8; 4] = [18, 18, 18, 255];
/// Semi-transparent yellow highlighter.
pub const COLOR_HIGHLIGHT: [u8; 4] = [255, 220, 60, 110];
/// Bright orange selection / marquee cue.
pub const COLOR_SELECTION: [u8; 4] = [255, 110, 30, 235];
/// Light gray live-marquee preview while dragging.
pub const COLOR_MARQUEE: [u8; 4] = [120, 120, 180, 160];
/// Subtle gray-blue for connection links.
pub const COLOR_LINK: [u8; 4] = [60, 70, 90, 220];

const CR_SEGMENT_STEPS: usize = 8;
const PRESSURE_GAMMA: f32 = 0.7;
const MIN_HALF_WIDTH: f32 = 0.35;

/// Smooth + resample + tessellate. Returns a triangle strip of vertices.
/// Empty if fewer than two input points.
pub fn tessellate(points: &[InkPoint], width_base: f32, color: [u8; 4]) -> Vec<InkVertex> {
    tessellate_opts(points, width_base, color, true, false)
}

/// Like [`tessellate`] but lets the caller disable pressure-modulated width
/// and opt in to tilt-modulated width.
///
/// When `pressure_sensitive` is false, every sample uses pressure=1.0 so the
/// stroke is uniformly `width_base` wide. When `tilt_sensitive` is true and
/// the samples carry non-zero tilt, the stroke widens with tilt magnitude
/// (chisel-nib feel).
pub fn tessellate_opts(
    points: &[InkPoint],
    width_base: f32,
    color: [u8; 4],
    pressure_sensitive: bool,
    tilt_sensitive: bool,
) -> Vec<InkVertex> {
    if points.len() < 2 {
        return Vec::new();
    }
    let smoothed = smooth(points);
    let resampled = resample_catmull_rom(&smoothed);
    extrude(&resampled, width_base, color, pressure_sensitive, tilt_sensitive)
}

#[derive(Debug, Clone, Copy)]
struct SamplePt {
    pos: Vec2,
    pressure: f32,
    tilt_mag: f32,
}

fn smooth(points: &[InkPoint]) -> Vec<SamplePt> {
    // Slightly higher mincutoff than the paper default keeps slow, deliberate
    // marks from being over-smoothed away. beta keeps fast strokes responsive.
    let pos_params = OneEuroParams { mincutoff: 2.5, beta: 0.015, dcutoff: 1.0 };
    let mut fx = OneEuro::new(pos_params);
    let mut fy = OneEuro::new(pos_params);
    let mut fp = OneEuro::new(OneEuroParams {
        mincutoff: 3.0,
        beta: 0.01,
        dcutoff: 1.0,
    });
    let mut ftilt = OneEuro::new(OneEuroParams {
        mincutoff: 2.0,
        beta: 0.008,
        dcutoff: 1.0,
    });
    points
        .iter()
        .map(|p| {
            let t_s = p.t_ms as f32 / 1000.0;
            let raw_tilt = (p.tilt_x * p.tilt_x + p.tilt_y * p.tilt_y).sqrt().clamp(0.0, 1.0);
            SamplePt {
                pos: Vec2::new(fx.filter(t_s, p.x), fy.filter(t_s, p.y)),
                pressure: fp.filter(t_s, p.pressure).clamp(0.0, 1.0),
                tilt_mag: ftilt.filter(t_s, raw_tilt).clamp(0.0, 1.0),
            }
        })
        .collect()
}

fn resample_catmull_rom(pts: &[SamplePt]) -> Vec<SamplePt> {
    if pts.len() < 2 {
        return pts.to_vec();
    }
    let mut out = Vec::with_capacity(pts.len() * CR_SEGMENT_STEPS);
    let n = pts.len();
    for i in 0..n - 1 {
        let p0 = pts[i.saturating_sub(1)];
        let p1 = pts[i];
        let p2 = pts[i + 1];
        let p3 = pts[(i + 2).min(n - 1)];
        for s in 0..CR_SEGMENT_STEPS {
            let t = s as f32 / CR_SEGMENT_STEPS as f32;
            out.push(SamplePt {
                pos: catmull_rom_vec2(p0.pos, p1.pos, p2.pos, p3.pos, t),
                pressure: catmull_rom_f32(p0.pressure, p1.pressure, p2.pressure, p3.pressure, t),
                tilt_mag: catmull_rom_f32(p0.tilt_mag, p1.tilt_mag, p2.tilt_mag, p3.tilt_mag, t),
            });
        }
    }
    out.push(pts[n - 1]);
    out
}

fn catmull_rom_vec2(p0: Vec2, p1: Vec2, p2: Vec2, p3: Vec2, t: f32) -> Vec2 {
    Vec2::new(
        catmull_rom_f32(p0.x, p1.x, p2.x, p3.x, t),
        catmull_rom_f32(p0.y, p1.y, p2.y, p3.y, t),
    )
}

fn catmull_rom_f32(p0: f32, p1: f32, p2: f32, p3: f32, t: f32) -> f32 {
    let t2 = t * t;
    let t3 = t2 * t;
    0.5 * ((2.0 * p1)
        + (-p0 + p2) * t
        + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t2
        + (-p0 + 3.0 * p1 - 3.0 * p2 + p3) * t3)
}

fn extrude(
    pts: &[SamplePt],
    width_base: f32,
    color: [u8; 4],
    pressure_sensitive: bool,
    tilt_sensitive: bool,
) -> Vec<InkVertex> {
    if pts.len() < 2 {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(pts.len() * 2);
    let n = pts.len();
    const AA_PAD: f32 = 0.6;
    const TILT_GAIN: f32 = 0.6;
    for i in 0..n {
        let prev = pts[i.saturating_sub(1)].pos;
        let next = pts[(i + 1).min(n - 1)].pos;
        let tangent = (next - prev).normalize_or_zero();
        let normal = Vec2::new(-tangent.y, tangent.x);

        let pressure = if pressure_sensitive { pts[i].pressure } else { 1.0 };
        let tilt_boost = if tilt_sensitive { 1.0 + pts[i].tilt_mag * TILT_GAIN } else { 1.0 };
        let half = (width_base * pressure.powf(PRESSURE_GAMMA) * tilt_boost * 0.5).max(MIN_HALF_WIDTH);
        let extruded = half + AA_PAD;
        out.push(InkVertex {
            pos: pts[i].pos + normal * extruded,
            signed_offset: extruded,
            half_width: half,
            color,
        });
        out.push(InkVertex {
            pos: pts[i].pos - normal * extruded,
            signed_offset: -extruded,
            half_width: half,
            color,
        });
    }
    out
}

/// Tessellate an arrow with a chosen color.
pub fn tessellate_arrow(a: &Arrow, color: [u8; 4]) -> Vec<InkVertex> {
    let dir = (a.end - a.start).normalize_or_zero();
    if dir.length_squared() < 1e-6 {
        return Vec::new();
    }
    let normal = Vec2::new(-dir.y, dir.x);
    let half = a.width.max(0.5);
    let head_size = a.head_size.max(2.0);
    let body_end = a.end - dir * head_size * 0.85;
    let aa_pad = 0.6;
    let extruded = half + aa_pad;

    let v = |pos: Vec2, signed_offset: f32, half_width: f32| InkVertex {
        pos,
        signed_offset,
        half_width,
        color,
    };
    let body_l_s = v(a.start + normal * extruded, extruded, half);
    let body_r_s = v(a.start - normal * extruded, -extruded, half);
    let body_l_e = v(body_end + normal * extruded, extruded, half);
    let body_r_e = v(body_end - normal * extruded, -extruded, half);

    let solid = |pos| v(pos, 0.0, 1000.0);
    let head_half = head_size * 0.55;
    let tip = solid(a.end);
    let base_l = solid(body_end + normal * head_half);
    let base_r = solid(body_end - normal * head_half);

    vec![
        body_l_s, body_r_s, body_l_e, body_r_e,
        body_r_e, tip,
        tip, base_l, base_r,
    ]
}

/// Filled rectangle (two triangles). All four vertices carry the same color
/// and SDF sentinel values so the fragment shader paints solid.
pub fn tessellate_rect(min: Vec2, max: Vec2, color: [u8; 4]) -> Vec<InkVertex> {
    let solid = |pos| InkVertex {
        pos,
        signed_offset: 0.0,
        half_width: 1000.0,
        color,
    };
    // Triangle strip: TL, BL, TR, BR.
    vec![
        solid(min),
        solid(Vec2::new(min.x, max.y)),
        solid(Vec2::new(max.x, min.y)),
        solid(max),
    ]
}

/// Filled ellipse. Triangle fan via degenerate-bridged strip segments.
pub fn tessellate_ellipse(min: Vec2, max: Vec2, color: [u8; 4], filled: bool) -> Vec<InkVertex> {
    if filled {
        tessellate_ellipse_filled(min, max, color)
    } else {
        tessellate_ellipse_outline(min, max, color, 1.0)
    }
}

const ELLIPSE_SEGMENTS: usize = 48;

fn tessellate_ellipse_filled(min: Vec2, max: Vec2, color: [u8; 4]) -> Vec<InkVertex> {
    let center = (min + max) * 0.5;
    let radius = (max - min) * 0.5;
    let solid = |pos| InkVertex {
        pos,
        signed_offset: 0.0,
        half_width: 1000.0,
        color,
    };
    let mut out: Vec<InkVertex> = Vec::with_capacity(ELLIPSE_SEGMENTS * 2 + 4);
    // Pairs of (perimeter, center) build a fan as a strip.
    for i in 0..=ELLIPSE_SEGMENTS {
        let t = (i as f32 / ELLIPSE_SEGMENTS as f32) * std::f32::consts::TAU;
        let p = center + Vec2::new(t.cos() * radius.x, t.sin() * radius.y);
        out.push(solid(p));
        out.push(solid(center));
    }
    out
}

fn tessellate_ellipse_outline(min: Vec2, max: Vec2, color: [u8; 4], half_width: f32) -> Vec<InkVertex> {
    let center = (min + max) * 0.5;
    let radius = (max - min) * 0.5;
    let mut points: Vec<Vec2> = Vec::with_capacity(ELLIPSE_SEGMENTS + 1);
    for i in 0..=ELLIPSE_SEGMENTS {
        let t = (i as f32 / ELLIPSE_SEGMENTS as f32) * std::f32::consts::TAU;
        points.push(center + Vec2::new(t.cos() * radius.x, t.sin() * radius.y));
    }
    let mut out: Vec<InkVertex> = Vec::new();
    for w in points.windows(2) {
        let seg = tessellate_segment_strip(w[0], w[1], half_width, color);
        if !out.is_empty() && !seg.is_empty() {
            let last = *out.last().unwrap();
            let first = seg[0];
            out.push(last);
            out.push(first);
        }
        out.extend(seg);
    }
    out
}

/// Single-segment line as a strip ready to be appended/bridged. Used by
/// rectangle outlines, ellipse outlines, links, etc.
pub fn tessellate_segment_strip(a: Vec2, b: Vec2, half_width: f32, color: [u8; 4]) -> Vec<InkVertex> {
    let dir = (b - a).normalize_or_zero();
    if dir.length_squared() < 1e-6 {
        return Vec::new();
    }
    let normal = Vec2::new(-dir.y, dir.x);
    let aa_pad = 0.5;
    let extruded = half_width + aa_pad;
    vec![
        InkVertex { pos: a + normal * extruded, signed_offset: extruded, half_width, color },
        InkVertex { pos: a - normal * extruded, signed_offset: -extruded, half_width, color },
        InkVertex { pos: b + normal * extruded, signed_offset: extruded, half_width, color },
        InkVertex { pos: b - normal * extruded, signed_offset: -extruded, half_width, color },
    ]
}

/// Standalone variable-width line segment (the Shape::Line tool).
pub fn tessellate_line(a: Vec2, b: Vec2, stroke_width: f32, color: [u8; 4]) -> Vec<InkVertex> {
    tessellate_segment_strip(a, b, stroke_width * 0.5, color)
}

/// Tessellate a rectangular outline. Used for selection/marquee cues.
pub fn tessellate_rect_outline(
    min: Vec2,
    max: Vec2,
    half_width: f32,
    color: [u8; 4],
) -> Vec<InkVertex> {
    let corners = [
        min,
        Vec2::new(max.x, min.y),
        max,
        Vec2::new(min.x, max.y),
    ];
    let mut out: Vec<InkVertex> = Vec::new();
    for i in 0..4 {
        let a = corners[i];
        let b = corners[(i + 1) % 4];
        let strip = tessellate_segment_strip(a, b, half_width, color);
        if !out.is_empty() {
            let last = *out.last().unwrap();
            let first = strip[0];
            out.push(last);
            out.push(first);
        }
        out.extend(strip);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f32, y: f32, pressure: f32, t_ms: u32) -> InkPoint {
        InkPoint { x, y, pressure, tilt_x: 0.0, tilt_y: 0.0, t_ms }
    }

    #[test]
    fn empty_below_two_points() {
        assert!(tessellate(&[], 4.0, COLOR_INK).is_empty());
        assert!(tessellate(&[p(0.0, 0.0, 1.0, 0)], 4.0, COLOR_INK).is_empty());
    }

    #[test]
    fn straight_line_widens_with_pressure() {
        let pts = vec![
            p(0.0, 0.0, 0.2, 0),
            p(100.0, 0.0, 1.0, 100),
        ];
        let v = tessellate(&pts, 8.0, COLOR_INK);
        assert!(v.len() >= 4);
        let first_hw = v[0].half_width;
        let last_hw = v[v.len() - 1].half_width;
        assert!(last_hw > first_hw, "{last_hw} should exceed {first_hw}");
    }

    #[test]
    fn pressure_insensitive_yields_uniform_width() {
        let pts = vec![
            p(0.0, 0.0, 0.2, 0),
            p(100.0, 0.0, 1.0, 100),
        ];
        let v = tessellate_opts(&pts, 8.0, COLOR_INK, false, false);
        let first_hw = v[0].half_width;
        let last_hw = v[v.len() - 1].half_width;
        assert!((first_hw - last_hw).abs() < 0.01, "{first_hw} vs {last_hw}");
    }

    #[test]
    fn tilt_widens_stroke_when_enabled() {
        let tilted = vec![
            InkPoint { x: 0.0, y: 0.0, pressure: 0.5, tilt_x: 0.9, tilt_y: 0.0, t_ms: 0 },
            InkPoint { x: 50.0, y: 0.0, pressure: 0.5, tilt_x: 0.9, tilt_y: 0.0, t_ms: 50 },
            InkPoint { x: 100.0, y: 0.0, pressure: 0.5, tilt_x: 0.9, tilt_y: 0.0, t_ms: 100 },
        ];
        let flat = vec![
            InkPoint { x: 0.0, y: 0.0, pressure: 0.5, tilt_x: 0.0, tilt_y: 0.0, t_ms: 0 },
            InkPoint { x: 50.0, y: 0.0, pressure: 0.5, tilt_x: 0.0, tilt_y: 0.0, t_ms: 50 },
            InkPoint { x: 100.0, y: 0.0, pressure: 0.5, tilt_x: 0.0, tilt_y: 0.0, t_ms: 100 },
        ];
        let v_tilt_on = tessellate_opts(&tilted, 8.0, COLOR_INK, true, true);
        let v_tilt_off = tessellate_opts(&flat, 8.0, COLOR_INK, true, true);
        // Tilt-enabled stroke with tilt should be wider than tilt-enabled
        // stroke with no tilt at the steady-state midpoint.
        let mid = v_tilt_on.len() / 2;
        let mid_flat = v_tilt_off.len() / 2;
        assert!(
            v_tilt_on[mid].half_width > v_tilt_off[mid_flat].half_width,
            "tilted {} should exceed flat {}",
            v_tilt_on[mid].half_width,
            v_tilt_off[mid_flat].half_width,
        );
    }

    #[test]
    fn vertex_count_is_even() {
        let pts: Vec<_> = (0..10)
            .map(|i| p(i as f32 * 10.0, 0.0, 0.5, i as u32 * 8))
            .collect();
        let v = tessellate(&pts, 6.0, COLOR_INK);
        assert_eq!(v.len() % 2, 0);
    }
}
