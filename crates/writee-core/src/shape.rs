//! Geometric primitives: rectangles, ellipses, and lines. Each carries a
//! `fill` flag (outline or solid) and an RGBA color so they can render
//! through the same ink pipeline as everything else.

use crate::geom::Aabb;
use glam::Vec2;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShapeKind {
    Rectangle,
    Ellipse,
    Line,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Shape {
    pub kind: ShapeKind,
    /// For Rectangle/Ellipse: the diagonal corners.
    /// For Line: the two endpoints.
    pub a: Vec2,
    pub b: Vec2,
    pub stroke_width: f32,
    /// `true` = filled, `false` = outline-only. Line ignores this (always
    /// stroked).
    pub filled: bool,
    /// RGBA color stored as the same packed `[u8; 4]` the InkVertex uses.
    pub color: [u8; 4],
}

impl Shape {
    pub fn new_rect(a: Vec2, b: Vec2, color: [u8; 4], stroke_width: f32, filled: bool) -> Self {
        Self { kind: ShapeKind::Rectangle, a, b, stroke_width, filled, color }
    }
    pub fn new_ellipse(a: Vec2, b: Vec2, color: [u8; 4], stroke_width: f32, filled: bool) -> Self {
        Self { kind: ShapeKind::Ellipse, a, b, stroke_width, filled, color }
    }
    pub fn new_line(a: Vec2, b: Vec2, color: [u8; 4], stroke_width: f32) -> Self {
        Self { kind: ShapeKind::Line, a, b, stroke_width, filled: false, color }
    }

    pub fn bbox(&self) -> Aabb {
        let min = self.a.min(self.b);
        let max = self.a.max(self.b);
        let pad = if self.filled { 0.0 } else { self.stroke_width };
        Aabb {
            min: min - Vec2::splat(pad),
            max: max + Vec2::splat(pad),
        }
    }

    pub fn center(&self) -> Vec2 {
        (self.a + self.b) * 0.5
    }
}
