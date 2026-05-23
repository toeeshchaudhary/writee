//! Thin wrapper around `egui-wgpu::Renderer`. The wgpu 22 RenderPass API is
//! lifetime-decoupled, so we run egui in its own short-lived render pass that
//! loads on top of the canvas pass — keeps lifetimes simple and lets the
//! canvas pass close cleanly.

use egui::FullOutput;
use egui_wgpu::{Renderer as EguiRenderer, ScreenDescriptor};

pub struct EguiPass {
    renderer: EguiRenderer,
}

impl EguiPass {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let renderer = EguiRenderer::new(device, format, None, 1, false);
        Self { renderer }
    }

    pub fn prepare_and_render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        ctx: &egui::Context,
        output: FullOutput,
        screen: ScreenDescriptor,
    ) {
        for (id, image) in &output.textures_delta.set {
            self.renderer.update_texture(device, queue, *id, image);
        }
        let primitives = ctx.tessellate(output.shapes, output.pixels_per_point);
        self.renderer
            .update_buffers(device, queue, encoder, &primitives, &screen);

        {
            let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("egui-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
            });
            let mut pass = pass.forget_lifetime();
            self.renderer.render(&mut pass, &primitives, &screen);
        }

        for id in &output.textures_delta.free {
            self.renderer.free_texture(id);
        }
    }
}
