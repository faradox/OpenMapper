//! GPU textures for image sources, plus a 1x1 white fallback used by
//! solid and generator sources (the shader always samples *something*).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use om_core::{Project, SourceId, SourceKind};

struct Entry {
    path: PathBuf,
    /// `None` when loading failed; the error is kept in `errors`.
    bind_group: Option<wgpu::BindGroup>,
    size: (u32, u32),
}

pub struct TextureCache {
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    fallback: wgpu::BindGroup,
    entries: HashMap<SourceId, Entry>,
    errors: HashMap<SourceId, String>,
}

impl TextureCache {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("om source texture"),
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
        // Repeat addressing so scrolling/zooming textures tile seamlessly.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("om source sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let fallback = upload(device, queue, &layout, &sampler, 1, 1, &[255; 4], "om white");
        Self { layout, sampler, fallback, entries: HashMap::new(), errors: HashMap::new() }
    }

    pub fn layout(&self) -> &wgpu::BindGroupLayout {
        &self.layout
    }

    pub fn error(&self, id: SourceId) -> Option<&str> {
        self.errors.get(&id).map(String::as_str)
    }

    pub fn size(&self, id: SourceId) -> Option<(u32, u32)> {
        self.entries.get(&id).filter(|e| e.bind_group.is_some()).map(|e| e.size)
    }

    /// Loads new or changed image sources and drops removed ones.
    pub fn sync(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, project: &Project) {
        self.entries.retain(|id, _| project.source(*id).is_some());
        self.errors.retain(|id, _| project.source(*id).is_some());

        for src in &project.sources {
            let SourceKind::Image { path } = &src.kind else {
                self.entries.remove(&src.id);
                self.errors.remove(&src.id);
                continue;
            };
            if self.entries.get(&src.id).is_some_and(|e| &e.path == path) {
                continue;
            }
            let max = device.limits().max_texture_dimension_2d;
            let entry = match load_rgba(path, max) {
                Ok(img) => {
                    self.errors.remove(&src.id);
                    let (w, h) = img.dimensions();
                    let bg = upload(device, queue, &self.layout, &self.sampler, w, h, &img, "om image");
                    Entry { path: path.clone(), bind_group: Some(bg), size: (w, h) }
                }
                Err(e) => {
                    log::warn!("failed to load image {}: {e:#}", path.display());
                    self.errors.insert(src.id, format!("{e:#}"));
                    Entry { path: path.clone(), bind_group: None, size: (0, 0) }
                }
            };
            self.entries.insert(src.id, entry);
        }
    }

    pub fn bind_group_for(&self, source: Option<SourceId>, project: &Project) -> &wgpu::BindGroup {
        source
            .filter(|id| matches!(project.source(*id).map(|s| &s.kind), Some(SourceKind::Image { .. })))
            .and_then(|id| self.entries.get(&id))
            .and_then(|e| e.bind_group.as_ref())
            .unwrap_or(&self.fallback)
    }
}

fn load_rgba(path: &Path, max_dim: u32) -> anyhow::Result<image::RgbaImage> {
    let img = image::ImageReader::open(path)?.with_guessed_format()?.decode()?;
    let img = if img.width() > max_dim || img.height() > max_dim {
        img.resize(max_dim, max_dim, image::imageops::FilterType::Triangle)
    } else {
        img
    };
    Ok(img.to_rgba8())
}

#[allow(clippy::too_many_arguments)]
fn upload(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    width: u32,
    height: u32,
    rgba: &[u8],
    label: &str,
) -> wgpu::BindGroup {
    let size = wgpu::Extent3d { width, height, depth_or_array_layers: 1 };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        rgba,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4 * width),
            rows_per_image: Some(height),
        },
        size,
    );
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(label),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
        ],
    })
}
