//! Textured-quad rendering for `ImageBlock`.
//!
//! Each image is uploaded to its own `wgpu::Texture` keyed by a content
//! hash so identical bytes share GPU memory. A single shared sampler.
//! Per-frame the App calls `set_quads` with the world-space rectangles +
//! texture handles, and the pipeline draws one indexed quad per image.

use std::collections::HashMap;

use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ImageVertex {
    world: [f32; 2],
    uv: [f32; 2],
}

/// Per-image draw request. `texture_id` indexes into ImagePipeline's cache.
#[derive(Debug, Clone)]
pub struct ImageQuad {
    pub texture_id: u64,
    pub world_min: [f32; 2],
    pub world_max: [f32; 2],
}

pub struct ImagePipeline {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    /// Cached textures keyed by `texture_id`.
    textures: HashMap<u64, CachedTexture>,
    /// Per-frame vertex + index buffer holding every visible quad.
    vbuf: Option<wgpu::Buffer>,
    ibuf: Option<wgpu::Buffer>,
    /// (texture_id, index_offset, index_count) per quad in draw order.
    draw_list: Vec<(u64, u32, u32)>,
}

struct CachedTexture {
    bind_group: wgpu::BindGroup,
}

impl ImagePipeline {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        viewport_bgl: &wgpu::BindGroupLayout,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("image.wgsl"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/image.wgsl").into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("image-tex-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("image-pipeline-layout"),
            bind_group_layouts: &[viewport_bgl, &bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("image-pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<ImageVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 0,
                            shader_location: 0,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 8,
                            shader_location: 1,
                        },
                    ],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("image-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        Self {
            pipeline,
            bind_group_layout,
            sampler,
            textures: HashMap::new(),
            vbuf: None,
            ibuf: None,
            draw_list: Vec::new(),
        }
    }

    pub fn has_texture(&self, id: u64) -> bool {
        self.textures.contains_key(&id)
    }

    /// Upload pre-decoded RGBA8 pixels for `id`. Idempotent.
    pub fn upload_rgba(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        id: u64,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) {
        if self.textures.contains_key(&id) {
            return;
        }
        let size = wgpu::Extent3d { width, height, depth_or_array_layers: 1 };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("image-texture"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            rgba,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(4 * width),
                rows_per_image: Some(height),
            },
            size,
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("image-bg"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            ],
        });
        self.textures.insert(id, CachedTexture { bind_group });
    }

    /// Replace the per-frame draw list. Re-uploads vbuf/ibuf only when the
    /// quad set changed; trivial to make per-frame even for many quads.
    pub fn set_quads(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, quads: &[ImageQuad]) {
        let mut verts: Vec<ImageVertex> = Vec::with_capacity(quads.len() * 4);
        let mut idx: Vec<u32> = Vec::with_capacity(quads.len() * 6);
        self.draw_list.clear();
        for q in quads {
            if !self.textures.contains_key(&q.texture_id) {
                continue;
            }
            let base = verts.len() as u32;
            verts.push(ImageVertex {
                world: [q.world_min[0], q.world_min[1]],
                uv: [0.0, 0.0],
            });
            verts.push(ImageVertex {
                world: [q.world_max[0], q.world_min[1]],
                uv: [1.0, 0.0],
            });
            verts.push(ImageVertex {
                world: [q.world_min[0], q.world_max[1]],
                uv: [0.0, 1.0],
            });
            verts.push(ImageVertex {
                world: [q.world_max[0], q.world_max[1]],
                uv: [1.0, 1.0],
            });
            let idx_start = idx.len() as u32;
            idx.extend_from_slice(&[base, base + 1, base + 2, base + 1, base + 3, base + 2]);
            self.draw_list.push((q.texture_id, idx_start, 6));
        }

        if verts.is_empty() {
            self.vbuf = None;
            self.ibuf = None;
            return;
        }

        let needed_v = (verts.len() * std::mem::size_of::<ImageVertex>()) as u64;
        let need_new_vbuf = match &self.vbuf {
            Some(b) => b.size() < needed_v,
            None => true,
        };
        if need_new_vbuf {
            self.vbuf = Some(device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("image-vbuf"),
                contents: bytemuck::cast_slice(&verts),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            }));
        } else if let Some(b) = &self.vbuf {
            queue.write_buffer(b, 0, bytemuck::cast_slice(&verts));
        }

        let needed_i = (idx.len() * std::mem::size_of::<u32>()) as u64;
        let need_new_ibuf = match &self.ibuf {
            Some(b) => b.size() < needed_i,
            None => true,
        };
        if need_new_ibuf {
            self.ibuf = Some(device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("image-ibuf"),
                contents: bytemuck::cast_slice(&idx),
                usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            }));
        } else if let Some(b) = &self.ibuf {
            queue.write_buffer(b, 0, bytemuck::cast_slice(&idx));
        }
    }

    pub fn draw<'rp>(&'rp self, pass: &mut wgpu::RenderPass<'rp>, viewport_bg: &'rp wgpu::BindGroup) {
        let (Some(vbuf), Some(ibuf)) = (&self.vbuf, &self.ibuf) else { return };
        if self.draw_list.is_empty() {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, viewport_bg, &[]);
        pass.set_vertex_buffer(0, vbuf.slice(..));
        pass.set_index_buffer(ibuf.slice(..), wgpu::IndexFormat::Uint32);
        for (tex_id, idx_start, idx_count) in &self.draw_list {
            let Some(t) = self.textures.get(tex_id) else { continue };
            pass.set_bind_group(1, &t.bind_group, &[]);
            pass.draw_indexed(*idx_start..*idx_start + *idx_count, 0, 0..1);
        }
    }
}
