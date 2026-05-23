//! Glyphon-backed text rendering layer.
//!
//! TextBoxes carry their `font_size` in *world units*. To get crisp glyphs at
//! any zoom we re-rasterise glyphs at the effective screen-space size each
//! frame. Glyphon caches glyphs by size internally; the atlas is trimmed each
//! frame to bound memory.

use crate::viewport::Viewport;
use glam::Vec2;
use glyphon::{
    Attrs, Buffer, Cache, Color, Family, FontSystem, Metrics, Resolution, Shaping, SwashCache,
    TextArea, TextAtlas, TextBounds, TextRenderer, Viewport as GlyphonViewport,
};
use writee_core::TextBox;

/// Single text instance the layer should draw this frame. Owned so the
/// renderer doesn't need to keep an open borrow on the document.
#[derive(Debug, Clone)]
pub struct TextInstance {
    pub world_pos: Vec2,
    pub font_size_world: f32,
    pub content: String,
    /// Optional family name. `None` ⇒ sans-serif fallback.
    pub font_name: Option<String>,
    pub color: [u8; 4],
}

pub struct TextLayer {
    font_system: FontSystem,
    swash_cache: SwashCache,
    atlas: TextAtlas,
    renderer: TextRenderer,
    glyphon_viewport: GlyphonViewport,
    // Held to keep its GPU resources alive; the atlas and viewport reference it.
    #[allow(dead_code)]
    cache: Cache,
    buffers: Vec<Buffer>,
}

impl TextLayer {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
    ) -> Self {
        let font_system = FontSystem::new();
        let swash_cache = SwashCache::new();
        let cache = Cache::new(device);
        let glyphon_viewport = GlyphonViewport::new(device, &cache);
        let mut atlas = TextAtlas::new(device, queue, &cache, format);
        let renderer = TextRenderer::new(
            &mut atlas,
            device,
            wgpu::MultisampleState::default(),
            None,
        );
        Self {
            font_system,
            swash_cache,
            atlas,
            renderer,
            glyphon_viewport,
            cache,
            buffers: Vec::new(),
        }
    }

    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        viewport: &Viewport,
        instances: &[TextInstance],
    ) -> Result<(), glyphon::PrepareError> {
        self.glyphon_viewport.update(
            queue,
            Resolution {
                width: viewport.screen.0,
                height: viewport.screen.1,
            },
        );

        // Rebuild buffers from scratch every frame; cheap for the dozens-of-boxes
        // workloads we expect in v1. Upgrade to a per-textbox cache in M2-polish
        // if profiles say it matters.
        self.buffers.clear();
        for inst in instances {
            let size_px = (inst.font_size_world * viewport.zoom).max(1.0);
            let metrics = Metrics::new(size_px, size_px * 1.25);
            let mut buffer = Buffer::new(&mut self.font_system, metrics);
            buffer.set_size(
                &mut self.font_system,
                Some(viewport.screen.0 as f32 * 2.0),
                Some(viewport.screen.1 as f32 * 2.0),
            );
            let family = inst
                .font_name
                .as_deref()
                .map(Family::Name)
                .unwrap_or(Family::SansSerif);
            buffer.set_text(
                &mut self.font_system,
                &inst.content,
                Attrs::new().family(family),
                Shaping::Advanced,
            );
            buffer.shape_until_scroll(&mut self.font_system, false);
            self.buffers.push(buffer);
        }

        let areas: Vec<TextArea> = instances
            .iter()
            .zip(self.buffers.iter())
            .map(|(inst, buffer)| {
                let pixel = (inst.world_pos - viewport.offset) * viewport.zoom;
                TextArea {
                    buffer,
                    left: pixel.x,
                    top: pixel.y,
                    scale: 1.0,
                    bounds: TextBounds {
                        left: 0,
                        top: 0,
                        right: viewport.screen.0 as i32,
                        bottom: viewport.screen.1 as i32,
                    },
                    default_color: Color::rgba(
                        inst.color[0],
                        inst.color[1],
                        inst.color[2],
                        inst.color[3],
                    ),
                    custom_glyphs: &[],
                }
            })
            .collect();

        self.renderer.prepare(
            device,
            queue,
            &mut self.font_system,
            &mut self.atlas,
            &self.glyphon_viewport,
            areas,
            &mut self.swash_cache,
        )
    }

    pub fn render<'rp>(&'rp self, pass: &mut wgpu::RenderPass<'rp>) -> Result<(), glyphon::RenderError> {
        self.renderer.render(&self.atlas, &self.glyphon_viewport, pass)
    }

    pub fn trim_atlas(&mut self) {
        self.atlas.trim();
    }
}

impl TextInstance {
    pub fn from_textbox(tb: &TextBox) -> Self {
        Self {
            world_pos: tb.origin,
            font_size_world: tb.font_size,
            content: tb.content.clone(),
            font_name: tb.font_name.clone(),
            color: [18, 18, 18, 255],
        }
    }
}
