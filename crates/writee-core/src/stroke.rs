use crate::geom::Aabb;
use glam::Vec2;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct InkPoint {
    pub x: f32,
    pub y: f32,
    pub pressure: f32,
    pub tilt_x: f32,
    pub tilt_y: f32,
    pub t_ms: u32,
}

impl InkPoint {
    pub fn pos(&self) -> Vec2 {
        Vec2::new(self.x, self.y)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Stroke {
    pub points: Vec<InkPoint>,
    /// 0 = ink, 1 = highlighter. Legacy index for files saved before
    /// `color_rgba` existed; ignored when `color_rgba` is `Some`.
    pub color: u8,
    pub width_base: f32,
    /// Per-stroke RGBA override. When `Some`, this is the authoritative
    /// color the renderer uses (replacing the legacy `color` index).
    /// `None` for old files; serde default keeps them loadable.
    #[serde(default)]
    pub color_rgba: Option<[u8; 4]>,
}

impl Stroke {
    pub fn new(width_base: f32) -> Self {
        Self { points: Vec::new(), color: 0, width_base, color_rgba: None }
    }

    pub fn with_color(width_base: f32, color: u8) -> Self {
        Self { points: Vec::new(), color, width_base, color_rgba: None }
    }

    pub fn with_rgba(width_base: f32, color_idx: u8, color_rgba: [u8; 4]) -> Self {
        Self {
            points: Vec::new(),
            color: color_idx,
            width_base,
            color_rgba: Some(color_rgba),
        }
    }

    /// Effective color for rendering. Prefers the explicit RGBA when present.
    pub fn effective_color(&self) -> [u8; 4] {
        if let Some(c) = self.color_rgba {
            return c;
        }
        match self.color {
            1 => crate::tessellate::COLOR_HIGHLIGHT,
            _ => crate::tessellate::COLOR_INK,
        }
    }

    pub fn push(&mut self, p: InkPoint) {
        self.points.push(p);
    }

    pub fn bbox(&self) -> Aabb {
        let mut bb = Aabb::EMPTY;
        for p in &self.points {
            bb.expand_radius(p.pos(), self.width_base * p.pressure.max(0.1));
        }
        bb
    }
}
