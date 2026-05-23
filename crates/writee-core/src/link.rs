//! Smart connectors. A [`Link`] joins two anchor points on two objects; the
//! renderer resolves their world positions every frame, so moving either
//! endpoint object drags the link with it.

use crate::geom::Aabb;
use glam::Vec2;
use serde::{Deserialize, Serialize};

/// One of nine canonical anchor points on an object's bbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Anchor {
    Center,
    N,
    S,
    E,
    W,
    NE,
    NW,
    SE,
    SW,
}

impl Anchor {
    pub fn all() -> [Anchor; 9] {
        [
            Anchor::Center,
            Anchor::N, Anchor::S, Anchor::E, Anchor::W,
            Anchor::NE, Anchor::NW, Anchor::SE, Anchor::SW,
        ]
    }

    pub fn position(self, bbox: Aabb) -> Vec2 {
        let mid_x = (bbox.min.x + bbox.max.x) * 0.5;
        let mid_y = (bbox.min.y + bbox.max.y) * 0.5;
        match self {
            Anchor::Center => Vec2::new(mid_x, mid_y),
            Anchor::N => Vec2::new(mid_x, bbox.min.y),
            Anchor::S => Vec2::new(mid_x, bbox.max.y),
            Anchor::E => Vec2::new(bbox.max.x, mid_y),
            Anchor::W => Vec2::new(bbox.min.x, mid_y),
            Anchor::NE => Vec2::new(bbox.max.x, bbox.min.y),
            Anchor::NW => Vec2::new(bbox.min.x, bbox.min.y),
            Anchor::SE => Vec2::new(bbox.max.x, bbox.max.y),
            Anchor::SW => Vec2::new(bbox.min.x, bbox.max.y),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct LinkEnd {
    /// Target object id within the current document.
    pub object_id: u64,
    pub anchor: Anchor,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Link {
    pub from: LinkEnd,
    pub to: LinkEnd,
    pub width: f32,
    pub color: [u8; 4],
}

impl Link {
    pub fn new(from: LinkEnd, to: LinkEnd) -> Self {
        Self {
            from,
            to,
            width: 1.6,
            color: [40, 40, 50, 220],
        }
    }
}
