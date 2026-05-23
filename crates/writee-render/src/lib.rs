//! wgpu-based renderer for the writee canvas.

use std::sync::Arc;
use wgpu::util::DeviceExt;
use winit::window::Window;

mod egui_pass;
mod grid;
mod image;
mod ink;
mod text;
mod viewport;

pub use egui_pass::EguiPass;
pub use image::ImageQuad;
pub use text::TextInstance;
pub use viewport::Viewport;
pub use writee_core::InkVertex;

pub struct Renderer {
    pub surface: wgpu::Surface<'static>,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub config: wgpu::SurfaceConfiguration,
    pub format: wgpu::TextureFormat,

    viewport_buf: wgpu::Buffer,
    viewport_bg: wgpu::BindGroup,

    grid: grid::GridPipeline,
    ink: ink::InkPipeline,
    image: image::ImagePipeline,
    text: text::TextLayer,

    window: Arc<Window>,
}

impl Renderer {
    pub async fn new(window: Arc<Window>) -> Result<Self, RendererError> {
        let size = window.inner_size();
        let (w, h) = (size.width.max(1), size.height.max(1));

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..Default::default()
        });

        let surface = instance
            .create_surface(window.clone())
            .map_err(|e| RendererError::Surface(e.to_string()))?;

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::LowPower,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .ok_or(RendererError::NoAdapter)?;

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("writee-device"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::downlevel_defaults()
                        .using_resolution(adapter.limits()),
                    memory_hints: wgpu::MemoryHints::Performance,
                },
                None,
            )
            .await
            .map_err(|e| RendererError::Device(e.to_string()))?;

        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(caps.formats[0]);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: w,
            height: h,
            present_mode: caps
                .present_modes
                .iter()
                .copied()
                .find(|m| *m == wgpu::PresentMode::Mailbox)
                .unwrap_or(wgpu::PresentMode::Fifo),
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        let viewport_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("viewport-bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let viewport_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("viewport-uniform"),
            contents: bytemuck::cast_slice(&[0f32; 16]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let viewport_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("viewport-bg"),
            layout: &viewport_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: viewport_buf.as_entire_binding(),
            }],
        });

        let grid = grid::GridPipeline::new(&device, format, &viewport_bgl);
        let ink = ink::InkPipeline::new(&device, format, &viewport_bgl);
        let image = image::ImagePipeline::new(&device, format, &viewport_bgl);
        let text = text::TextLayer::new(&device, &queue, format);

        Ok(Self {
            surface, device, queue, config, format,
            viewport_buf, viewport_bg,
            grid, ink, image, text,
            window,
        })
    }

    pub fn window(&self) -> &Window {
        &self.window
    }

    pub fn surface_size(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }

    pub fn resize(&mut self, w: u32, h: u32) {
        if w == 0 || h == 0 {
            return;
        }
        self.config.width = w;
        self.config.height = h;
        self.surface.configure(&self.device, &self.config);
    }

    pub fn upload_ink_geometry(&mut self, verts: &[InkVertex]) {
        self.ink.upload(&self.device, &self.queue, verts);
    }

    /// Make sure a texture (keyed by `id`) is uploaded. No-op if cached.
    pub fn upload_image_rgba(&mut self, id: u64, w: u32, h: u32, rgba: &[u8]) {
        self.image.upload_rgba(&self.device, &self.queue, id, w, h, rgba);
    }

    pub fn image_cached(&self, id: u64) -> bool {
        self.image.has_texture(id)
    }

    pub fn set_image_quads(&mut self, quads: &[ImageQuad]) {
        self.image.set_quads(&self.device, &self.queue, quads);
    }

    /// Render one frame: canvas (grid + ink + text) plus an optional egui
    /// chrome pass on top.
    pub fn render(
        &mut self,
        viewport: &Viewport,
        text_instances: &[TextInstance],
        theme: &writee_core::ColorTheme,
        egui: Option<EguiFrame<'_>>,
    ) -> Result<(), wgpu::SurfaceError> {
        let bg = rgba_to_f32(theme.canvas_bg);
        let dot = rgba_to_f32(theme.grid_dot);
        self.queue.write_buffer(
            &self.viewport_buf,
            0,
            bytemuck::cast_slice(&viewport.to_uniform(bg, dot)),
        );

        if let Err(e) = self.text.prepare(&self.device, &self.queue, viewport, text_instances) {
            log::warn!("text prepare failed: {e:?}");
        }

        let frame = self.surface.get_current_texture()?;
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("writee-encoder") });

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("writee-canvas-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: bg[0] as f64,
                            g: bg[1] as f64,
                            b: bg[2] as f64,
                            a: bg[3] as f64,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
            });
            self.grid.draw(&mut pass, &self.viewport_bg);
            // Images render below ink so the user can ink on top of them.
            self.image.draw(&mut pass, &self.viewport_bg);
            self.ink.draw(&mut pass, &self.viewport_bg);
            if let Err(e) = self.text.render(&mut pass) {
                log::warn!("text render failed: {e:?}");
            }
        }

        if let Some(frame_egui) = egui {
            frame_egui.pass.prepare_and_render(
                &self.device,
                &self.queue,
                &mut encoder,
                &view,
                frame_egui.ctx,
                frame_egui.output,
                frame_egui.screen,
            );
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        frame.present();
        self.text.trim_atlas();
        Ok(())
    }
}

fn rgba_to_f32(c: [u8; 4]) -> [f32; 4] {
    [
        c[0] as f32 / 255.0,
        c[1] as f32 / 255.0,
        c[2] as f32 / 255.0,
        c[3] as f32 / 255.0,
    ]
}

/// Bundle passed into [`Renderer::render`] when egui chrome is enabled.
pub struct EguiFrame<'a> {
    pub pass: &'a mut EguiPass,
    pub ctx: &'a egui::Context,
    pub output: egui::FullOutput,
    pub screen: egui_wgpu::ScreenDescriptor,
}

#[derive(Debug, thiserror::Error)]
pub enum RendererError {
    #[error("no compatible wgpu adapter found")]
    NoAdapter,
    #[error("surface creation failed: {0}")]
    Surface(String),
    #[error("device creation failed: {0}")]
    Device(String),
}
