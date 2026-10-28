//! OpenMapper GPU compositor.
//!
//! [`Renderer`] draws a [`Project`] at a point in time into an offscreen
//! `Rgba8Unorm` texture the size of the canvas. The app displays that one
//! texture both in the editor preview and full-screen on the projector, so
//! what you see while editing is exactly what gets projected.
//!
//! Colors are kept in display (sRGB-encoded) space end to end, matching how
//! designers pick colors and how egui displays user textures.

mod textures;

use std::collections::HashMap;
use std::num::NonZeroU64;

use bytemuck::{Pod, Zeroable};
use glam::{Mat3, Vec2};
use om_core::anim::evaluate;
use om_core::{BlendMode, Mapping, Mask, Project, SourceId, SourceKind, Surface};

use textures::TextureCache;

/// Format of the output texture (required by egui for native textures).
pub const OUTPUT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SurfaceUniforms {
    corners0: [f32; 4],
    corners1: [f32; 4],
    inv_h: [[f32; 4]; 3],
    uv_m: [[f32; 4]; 3],
    canvas: [f32; 4],
    tint: [f32; 4],
    color_a: [f32; 4],
    color_b: [f32; 4],
    gen_params: [f32; 4],
    misc: [f32; 4],
}

fn mat3_cols(m: Mat3) -> [[f32; 4]; 3] {
    [m.x_axis.extend(0.0).into(), m.y_axis.extend(0.0).into(), m.z_axis.extend(0.0).into()]
}

/// The offscreen render target that receives the composed canvas.
pub struct OutputTarget {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub width: u32,
    pub height: u32,
}

pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipelines: HashMap<BlendMode, wgpu::RenderPipeline>,
    uniform_layout: wgpu::BindGroupLayout,
    uniform_buffer: wgpu::Buffer,
    uniform_bind_group: wgpu::BindGroup,
    uniform_capacity: usize,
    uniform_stride: u64,
    textures: TextureCache,
    output: OutputTarget,
    output_generation: u64,
}

impl Renderer {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("om surface shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("surface.wgsl").into()),
        });

        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("om surface uniforms"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: NonZeroU64::new(size_of::<SurfaceUniforms>() as u64),
                },
                count: None,
            }],
        });

        let textures = TextureCache::new(device, queue);

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("om surface layout"),
            bind_group_layouts: &[Some(&uniform_layout), Some(textures.layout())],
            immediate_size: 0,
        });

        let pipelines = BlendMode::ALL
            .into_iter()
            .map(|mode| (mode, create_pipeline(device, &layout, &shader, mode)))
            .collect();

        let align = device.limits().min_uniform_buffer_offset_alignment as u64;
        let uniform_stride = (size_of::<SurfaceUniforms>() as u64).div_ceil(align) * align;
        let (uniform_buffer, uniform_bind_group) =
            create_uniforms(device, &uniform_layout, uniform_stride, 16);

        let output = create_output(device, 1920, 1080);

        Self {
            device: device.clone(),
            queue: queue.clone(),
            pipelines,
            uniform_layout,
            uniform_buffer,
            uniform_bind_group,
            uniform_capacity: 16,
            uniform_stride,
            textures,
            output,
            output_generation: 0,
        }
    }

    pub fn output(&self) -> &OutputTarget {
        &self.output
    }

    /// Increments whenever the output texture is re-created (canvas resize),
    /// so callers know to re-register it with their UI toolkit.
    pub fn output_generation(&self) -> u64 {
        self.output_generation
    }

    /// Error message for a source that failed to load, if any.
    pub fn source_error(&self, id: SourceId) -> Option<&str> {
        self.textures.error(id)
    }

    /// Pixel size of a loaded image source.
    pub fn source_size(&self, id: SourceId) -> Option<(u32, u32)> {
        self.textures.size(id)
    }

    /// Renders the project at `time` seconds and submits the work.
    pub fn render(&mut self, project: &Project, time: f64) {
        let (w, h) = (project.canvas.width.max(1), project.canvas.height.max(1));
        if (w, h) != (self.output.width, self.output.height) {
            self.output = create_output(&self.device, w, h);
            self.output_generation += 1;
        }

        self.textures.sync(&self.device, &self.queue, project);

        let visible: Vec<&Surface> = project.surfaces.iter().filter(|s| s.visible).collect();
        self.ensure_uniform_capacity(visible.len());

        let canvas = project.canvas.size();
        let mut data = vec![0u8; self.uniform_stride as usize * visible.len().max(1)];
        for (i, s) in visible.iter().enumerate() {
            let u = surface_uniforms(project, s, canvas, time as f32);
            let off = i * self.uniform_stride as usize;
            data[off..off + size_of::<SurfaceUniforms>()].copy_from_slice(bytemuck::bytes_of(&u));
        }
        self.queue.write_buffer(&self.uniform_buffer, 0, &data);

        let bg = project.canvas.background;
        let mut encoder =
            self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("om render") });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("om canvas"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.output.view,
                    depth_slice: None,
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
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });

            for (i, s) in visible.iter().enumerate() {
                pass.set_pipeline(&self.pipelines[&s.blend]);
                pass.set_bind_group(0, &self.uniform_bind_group, &[(i as u64 * self.uniform_stride) as u32]);
                pass.set_bind_group(1, self.textures.bind_group_for(s.source, project), &[]);
                pass.draw(0..6, 0..1);
            }
        }
        self.queue.submit([encoder.finish()]);
    }

    fn ensure_uniform_capacity(&mut self, count: usize) {
        if count <= self.uniform_capacity {
            return;
        }
        let cap = count.next_power_of_two();
        let (buf, bg) = create_uniforms(&self.device, &self.uniform_layout, self.uniform_stride, cap);
        self.uniform_buffer = buf;
        self.uniform_bind_group = bg;
        self.uniform_capacity = cap;
    }
}

fn surface_uniforms(project: &Project, s: &Surface, canvas: Vec2, time: f32) -> SurfaceUniforms {
    let frame = evaluate(s, canvas, time);
    let c = s.corners;

    let (kind, color_a, color_b, scale, speed, angle) = match s
        .source
        .and_then(|id| project.source(id))
        .map(|src| &src.kind)
    {
        None => (1.0, [0.0; 4], [0.0; 4], 1.0, 0.0, 0.0),
        Some(SourceKind::Solid { color }) => (1.0, *color, *color, 1.0, 0.0, 0.0),
        Some(SourceKind::Image { .. }) => (0.0, [1.0; 4], [1.0; 4], 1.0, 0.0, 0.0),
        Some(SourceKind::Generator(g)) => {
            (g.pattern.shader_index() as f32, g.color_a, g.color_b, g.scale, g.speed, g.angle.to_radians())
        }
    };

    let mask = match s.mask {
        Mask::Quad => 0.0,
        Mask::Ellipse => 1.0,
        Mask::Triangle => 2.0,
    };
    let mapping = match s.mapping {
        Mapping::Surface => 0.0,
        Mapping::Canvas => 1.0,
    };

    SurfaceUniforms {
        corners0: [c[0].x, c[0].y, c[1].x, c[1].y],
        corners1: [c[2].x, c[2].y, c[3].x, c[3].y],
        inv_h: mat3_cols(frame.canvas_to_surface),
        uv_m: mat3_cols(frame.uv_matrix),
        canvas: [canvas.x, canvas.y, frame.local_time, mapping],
        tint: s.tint,
        color_a,
        color_b,
        gen_params: [kind, scale, speed, angle],
        misc: [frame.opacity, frame.hue_shift, s.feather.clamp(0.0, 0.5), mask],
    }
}

fn create_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    mode: BlendMode,
) -> wgpu::RenderPipeline {
    use wgpu::{BlendComponent, BlendFactor as F, BlendOperation as Op};
    let comp = |src, dst| BlendComponent { src_factor: src, dst_factor: dst, operation: Op::Add };
    // The shader outputs premultiplied alpha.
    let blend = match mode {
        BlendMode::Normal => wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING,
        BlendMode::Add => {
            wgpu::BlendState { color: comp(F::One, F::One), alpha: comp(F::One, F::OneMinusSrcAlpha) }
        }
        BlendMode::Screen => {
            wgpu::BlendState { color: comp(F::One, F::OneMinusSrc), alpha: comp(F::One, F::OneMinusSrcAlpha) }
        }
        BlendMode::Multiply => wgpu::BlendState {
            color: comp(F::Dst, F::OneMinusSrcAlpha),
            alpha: comp(F::One, F::OneMinusSrcAlpha),
        },
    };

    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(&format!("om surface {mode:?}")),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: OUTPUT_FORMAT,
                blend: Some(blend),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn create_uniforms(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    stride: u64,
    capacity: usize,
) -> (wgpu::Buffer, wgpu::BindGroup) {
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("om surface uniforms"),
        size: stride * capacity as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("om surface uniforms"),
        layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                buffer: &buffer,
                offset: 0,
                size: NonZeroU64::new(size_of::<SurfaceUniforms>() as u64),
            }),
        }],
    });
    (buffer, bind_group)
}

fn create_output(device: &wgpu::Device, width: u32, height: u32) -> OutputTarget {
    let max = device.limits().max_texture_dimension_2d;
    let (width, height) = (width.min(max), height.min(max));
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("om output"),
        size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: OUTPUT_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    OutputTarget { texture, view, width, height }
}

#[cfg(test)]
mod tests {
    #[test]
    fn surface_shader_validates() {
        let module = naga::front::wgsl::parse_str(include_str!("surface.wgsl"))
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(include_str!("surface.wgsl"))));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::empty())
            .validate(&module)
            .unwrap_or_else(|e| panic!("{e:?}"));
    }

    #[test]
    fn uniforms_match_shader_layout() {
        // 14 vec4s, see `SurfaceUniforms` in surface.wgsl.
        assert_eq!(size_of::<super::SurfaceUniforms>(), 14 * 16);
    }
}
