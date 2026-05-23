use writee_core::InkVertex;

/// Dynamic per-frame ink pipeline. Owns a growable vertex buffer that the app
/// re-uploads each frame for the live "wet" stroke. Committed strokes will be
/// rasterized into the tile cache in M2; for M1 everything goes through this
/// dynamic path.
pub struct InkPipeline {
    pipeline: wgpu::RenderPipeline,
    vbuf: wgpu::Buffer,
    vbuf_capacity: u64,
    count: u32,
}

impl InkPipeline {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        viewport_bgl: &wgpu::BindGroupLayout,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ink.wgsl"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/ink.wgsl").into()),
        });

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ink-pll"),
            bind_group_layouts: &[viewport_bgl],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ink-pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<InkVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 0,
                            shader_location: 0,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32,
                            offset: 8,
                            shader_location: 1,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32,
                            offset: 12,
                            shader_location: 2,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Unorm8x4,
                            offset: 16,
                            shader_location: 3,
                        },
                    ],
                }],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let initial_capacity = 4096u64 * std::mem::size_of::<InkVertex>() as u64;
        let vbuf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ink-vbuf"),
            size: initial_capacity,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self { pipeline, vbuf, vbuf_capacity: initial_capacity, count: 0 }
    }

    pub fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, verts: &[InkVertex]) {
        self.count = verts.len() as u32;
        if verts.is_empty() {
            return;
        }
        let bytes = bytemuck::cast_slice(verts);
        let needed = bytes.len() as u64;
        if needed > self.vbuf_capacity {
            let new_cap = needed.saturating_mul(2).next_power_of_two();
            self.vbuf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("ink-vbuf"),
                size: new_cap,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.vbuf_capacity = new_cap;
        }
        queue.write_buffer(&self.vbuf, 0, bytes);
    }

    pub fn draw<'rp>(&'rp self, rpass: &mut wgpu::RenderPass<'rp>, viewport_bg: &'rp wgpu::BindGroup) {
        if self.count < 3 {
            return;
        }
        rpass.set_pipeline(&self.pipeline);
        rpass.set_bind_group(0, viewport_bg, &[]);
        rpass.set_vertex_buffer(0, self.vbuf.slice(..));
        rpass.draw(0..self.count, 0..1);
    }
}
