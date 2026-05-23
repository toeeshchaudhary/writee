use glam::Vec2;

/// Camera state for the canvas. Owned by the app; passed into the renderer
/// each frame.
#[derive(Debug, Clone, Copy)]
pub struct Viewport {
    /// World-space coordinate at the top-left pixel of the window.
    pub offset: Vec2,
    /// Pixels per world unit.
    pub zoom: f32,
    /// Physical surface size in pixels.
    pub screen: (u32, u32),
}

impl Viewport {
    pub fn new(screen: (u32, u32)) -> Self {
        Self { offset: Vec2::ZERO, zoom: 1.0, screen }
    }

    pub fn pan(&mut self, delta_px: Vec2) {
        // Drag the *canvas* by delta_px screen pixels.
        self.offset -= delta_px / self.zoom;
    }

    /// Zoom by `factor` (e.g. 1.1 to zoom in 10%) keeping `pivot_px` fixed on
    /// screen. The world point under the cursor stays under the cursor.
    pub fn zoom_about(&mut self, pivot_px: Vec2, factor: f32) {
        let world_before = self.offset + pivot_px / self.zoom;
        self.zoom = (self.zoom * factor).clamp(0.05, 50.0);
        let world_after = self.offset + pivot_px / self.zoom;
        self.offset += world_before - world_after;
    }

    /// GPU layout for the wgsl `Viewport` uniform. 64 bytes total, std140-safe.
    /// `bg_rgba` and `dot_rgba` colours feed the grid shader.
    pub fn to_uniform(&self, bg_rgba: [f32; 4], dot_rgba: [f32; 4]) -> [f32; 16] {
        [
            self.offset.x,
            self.offset.y,
            self.zoom,
            0.0,
            self.screen.0 as f32,
            self.screen.1 as f32,
            0.0,
            0.0,
            bg_rgba[0],
            bg_rgba[1],
            bg_rgba[2],
            bg_rgba[3],
            dot_rgba[0],
            dot_rgba[1],
            dot_rgba[2],
            dot_rgba[3],
        ]
    }
}
