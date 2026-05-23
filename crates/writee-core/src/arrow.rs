use crate::geom::Aabb;
use glam::Vec2;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Arrow {
    pub start: Vec2,
    pub end: Vec2,
    /// Half-width of the shaft, world units.
    pub width: f32,
    /// Length of the arrowhead along the shaft, world units.
    pub head_size: f32,
}

impl Arrow {
    pub fn new(start: Vec2, end: Vec2) -> Self {
        Self { start, end, width: 1.8, head_size: 14.0 }
    }

    pub fn bbox(&self) -> Aabb {
        let mut bb = Aabb::from_point(self.start);
        bb.expand_radius(self.start, self.width + self.head_size);
        bb.expand_radius(self.end, self.width + self.head_size);
        bb
    }
}
