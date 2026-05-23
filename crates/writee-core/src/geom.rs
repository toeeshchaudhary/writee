use glam::Vec2;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Aabb {
    pub min: Vec2,
    pub max: Vec2,
}

impl Aabb {
    pub const EMPTY: Self = Self {
        min: Vec2::new(f32::INFINITY, f32::INFINITY),
        max: Vec2::new(f32::NEG_INFINITY, f32::NEG_INFINITY),
    };

    pub fn from_point(p: Vec2) -> Self {
        Self { min: p, max: p }
    }

    pub fn expand_point(&mut self, p: Vec2) {
        self.min = self.min.min(p);
        self.max = self.max.max(p);
    }

    pub fn expand_radius(&mut self, p: Vec2, r: f32) {
        self.min = self.min.min(p - Vec2::splat(r));
        self.max = self.max.max(p + Vec2::splat(r));
    }

    pub fn intersects(&self, other: &Aabb) -> bool {
        self.min.x <= other.max.x
            && self.max.x >= other.min.x
            && self.min.y <= other.max.y
            && self.max.y >= other.min.y
    }
}
