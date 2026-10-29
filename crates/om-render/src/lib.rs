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
use om_core::geom::quad_aspect;
use om_core::shape::{MAX_OUTLINE_POINTS, outline_uv};
use om_core::{BlendMode, Generator, Mapping, Mask, Palette, Project, SourceId, SourceKind, Surface};

pub use om_media::{VIDEO_EXTENSIONS, VideoInfo};
use textures::TextureCache;

/// Format of the output texture (required by egui for native textures).
pub const OUTPUT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Pixel size of one tile in the thumbnail atlas.
pub const THUMB_SIZE: (u32, u32) = (192, 120);
const THUMB_COLUMNS: u32 = 6;

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
    pal: [f32; 4],
    stops: [[f32; 4]; MAX_STOPS],
    shape: [f32; 4],
    points: [[f32; 4]; MAX_OUTLINE_POINTS / 2],
}

const MAX_STOPS: usize = 6;

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
    /// Live previews of generators (shader library), one tile each.
    thumbnails: Option<OutputTarget>,
    thumbnail_generation: u64,
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
            thumbnails: None,
            thumbnail_generation: 0,
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

    /// Pixel size of a loaded image or video source.
    pub fn source_size(&self, id: SourceId) -> Option<(u32, u32)> {
        self.textures.size(id)
    }

    pub fn video_info(&self, id: SourceId) -> Option<&VideoInfo> {
        self.textures.video_info(id)
    }

    /// `waker` is called from decoder threads when a new video frame is
    /// ready, so the UI can repaint even while paused.
    pub fn set_frame_waker(&mut self, waker: impl Fn() + Send + Sync + 'static) {
        self.textures.set_waker(std::sync::Arc::new(waker));
    }

    /// Renders the project at `time` seconds and submits the work.
    pub fn render(&mut self, project: &Project, time: f64) {
        let (w, h) = (project.canvas.width.max(1), project.canvas.height.max(1));
        if (w, h) != (self.output.width, self.output.height) {
            self.output = create_output(&self.device, w, h);
            self.output_generation += 1;
        }
        self.textures.sync(&self.device, &self.queue, project, time);
        let view = self.output.view.clone();
        self.draw(project, time, &view);
    }

    /// Renders one [`THUMB_SIZE`] tile per generator into the thumbnail
    /// atlas, row by row. See [`Renderer::thumbnail_uv`].
    pub fn render_thumbnails(&mut self, generators: &[Generator], time: f64) {
        let rows = (generators.len() as u32).div_ceil(THUMB_COLUMNS).max(1);
        let (w, h) = (THUMB_SIZE.0 * THUMB_COLUMNS, THUMB_SIZE.1 * rows);
        if self.thumbnails.as_ref().is_none_or(|t| (t.width, t.height) != (w, h)) {
            self.thumbnails = Some(create_output(&self.device, w, h));
            self.thumbnail_generation += 1;
        }
        let mut project = Project::empty();
        project.canvas.width = w;
        project.canvas.height = h;
        let tile = Vec2::new(THUMB_SIZE.0 as f32, THUMB_SIZE.1 as f32);
        for (i, g) in generators.iter().enumerate() {
            let source = project.add_source("", SourceKind::Generator(g.clone()));
            let id = project.new_surface_id();
            let cell = Vec2::new((i as u32 % THUMB_COLUMNS) as f32, (i as u32 / THUMB_COLUMNS) as f32);
            let mut s = Surface::rect(id, "", cell * tile, tile);
            s.source = Some(source);
            project.surfaces.push(s);
        }
        let view = self.thumbnails.as_ref().map(|t| t.view.clone()).expect("created above");
        self.draw(&project, time, &view);
    }

    pub fn thumbnails(&self) -> Option<&OutputTarget> {
        self.thumbnails.as_ref()
    }

    /// Increments whenever the thumbnail atlas is re-created.
    pub fn thumbnail_generation(&self) -> u64 {
        self.thumbnail_generation
    }

    /// Texture coordinates (min, max) of tile `index` in the thumbnail atlas.
    pub fn thumbnail_uv(&self, index: usize) -> ([f32; 2], [f32; 2]) {
        let Some(t) = &self.thumbnails else { return ([0.0; 2], [0.0; 2]) };
        let (col, row) = (index as u32 % THUMB_COLUMNS, index as u32 / THUMB_COLUMNS);
        let (tw, th) = (THUMB_SIZE.0 as f32 / t.width as f32, THUMB_SIZE.1 as f32 / t.height as f32);
        ([col as f32 * tw, row as f32 * th], [(col + 1) as f32 * tw, (row + 1) as f32 * th])
    }

    fn draw(&mut self, project: &Project, time: f64, target: &wgpu::TextureView) {
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
                    view: target,
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
                let blend = if s.cutout { BlendMode::Normal } else { s.blend };
                pass.set_pipeline(&self.pipelines[&blend]);
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

    // A cutout paints the canvas background over everything below it, using
    // the same coverage (mask, feather, opacity) as a normal surface.
    let bg = project.canvas.background;
    let mut palette = (Palette::Duotone, 0.5);
    let (kind, color_a, color_b, scale, speed, angle) = match s
        .source
        .and_then(|id| project.source(id))
        .map(|src| &src.kind)
    {
        _ if s.cutout => (1.0, [bg[0], bg[1], bg[2], 1.0], [0.0; 4], 1.0, 0.0, 0.0),
        None => (1.0, [0.0; 4], [0.0; 4], 1.0, 0.0, 0.0),
        Some(SourceKind::Solid { color }) => (1.0, *color, *color, 1.0, 0.0, 0.0),
        Some(SourceKind::Image { .. } | SourceKind::Video { .. }) => (0.0, [1.0; 4], [1.0; 4], 1.0, 0.0, 0.0),
        Some(SourceKind::Generator(g)) => {
            palette = (g.palette, g.detail);
            (g.pattern.shader_index() as f32, g.color_a, g.color_b, g.scale, g.speed, g.angle.to_radians())
        }
    };
    let stop_list = palette.0.stops(color_a, color_b);
    let mut stops = [[0.0; 4]; MAX_STOPS];
    for (dst, src) in stops.iter_mut().zip(&stop_list) {
        *dst = *src;
    }

    let mask = match s.mask {
        Mask::Quad => 0.0,
        Mask::Ellipse => 1.0,
        Mask::Triangle => 2.0,
        _ => 3.0,
    };
    let mut points = [[0.0; 4]; MAX_OUTLINE_POINTS / 2];
    let mut point_count = 0;
    if s.mask.is_polygon() {
        let outline = outline_uv(s.mask, &s.path);
        point_count = outline.len().min(MAX_OUTLINE_POINTS);
        for (i, p) in outline.iter().take(point_count).enumerate() {
            points[i / 2][(i % 2) * 2] = p.x;
            points[i / 2][(i % 2) * 2 + 1] = p.y;
        }
    }
    let mapping = match s.mapping {
        Mapping::Surface => 0.0,
        Mapping::Canvas => 1.0,
    };
    let aspect = match s.mapping {
        Mapping::Surface => quad_aspect(c),
        Mapping::Canvas => canvas.x / canvas.y.max(1.0),
    };

    SurfaceUniforms {
        corners0: [c[0].x, c[0].y, c[1].x, c[1].y],
        corners1: [c[2].x, c[2].y, c[3].x, c[3].y],
        inv_h: mat3_cols(frame.canvas_to_surface),
        uv_m: mat3_cols(frame.uv_matrix),
        canvas: [canvas.x, canvas.y, frame.local_time, mapping],
        tint: if s.cutout { [1.0; 4] } else { s.tint },
        color_a,
        color_b,
        gen_params: [kind, scale, speed, angle],
        misc: [frame.opacity, if s.cutout { 0.0 } else { frame.hue_shift }, s.feather.clamp(0.0, 0.5), mask],
        pal: [
            stop_list.len().min(MAX_STOPS) as f32,
            if palette.0.is_cyclic() { 1.0 } else { 0.0 },
            palette.1.clamp(0.0, 1.0),
            aspect,
        ],
        stops,
        shape: [point_count as f32, 0.0, 0.0, 0.0],
        points,
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
    fn cutout_paints_the_background() {
        let mut p = om_core::Project::demo();
        p.canvas.background = [0.1, 0.2, 0.3, 1.0];
        p.surfaces[0].cutout = true;
        p.surfaces[0].tint = [0.5; 4];
        p.surfaces[0].animation.hue_speed = 1.0;
        let u = super::surface_uniforms(&p, &p.surfaces[0], p.canvas.size(), 0.5);
        assert_eq!(u.gen_params[0], 1.0, "drawn as a solid color");
        assert_eq!(u.color_a, [0.1, 0.2, 0.3, 1.0]);
        assert_eq!(u.tint, [1.0; 4]);
        assert_eq!(u.misc[1], 0.0, "no hue shift");
    }

    #[test]
    fn polygon_shapes_send_their_outline() {
        let mut p = om_core::Project::demo();
        let u = super::surface_uniforms(&p, &p.surfaces[0], p.canvas.size(), 0.0);
        assert_eq!((u.misc[3], u.shape[0]), (0.0, 0.0), "quad is analytic");

        let s = &mut p.surfaces[0];
        s.mask = om_core::Mask::Star;
        let u = super::surface_uniforms(&p, &p.surfaces[0], p.canvas.size(), 0.0);
        assert_eq!((u.misc[3], u.shape[0]), (3.0, 10.0));
        // First star tip points up from the top-center.
        assert!((u.points[0][0] - 0.5).abs() < 1e-4 && u.points[0][1].abs() < 1e-4);
    }

    #[test]
    fn generator_palette_reaches_the_shader() {
        let mut p = om_core::Project::demo();
        let id = p.surfaces[0].source.unwrap();
        if let Some(om_core::SourceKind::Generator(g)) = p.source_mut(id).map(|s| &mut s.kind) {
            g.palette = om_core::Palette::Fire;
            g.detail = 0.25;
        }
        let u = super::surface_uniforms(&p, &p.surfaces[0], p.canvas.size(), 0.0);
        assert_eq!(u.pal[0], 5.0);
        assert_eq!(u.pal[2], 0.25);
        assert_eq!(u.stops[0], [0.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn uniforms_match_shader_layout() {
        // 86 vec4s, see `SurfaceUniforms` in surface.wgsl.
        assert_eq!(size_of::<super::SurfaceUniforms>(), 86 * 16);
    }
}
