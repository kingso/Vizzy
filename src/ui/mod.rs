use std::{borrow::Cow, collections::HashMap, ops::Range};

use egui::{
    Align2, Color32, Grid, ProgressBar, Rgba, RichText,
    epaint::{
        ClippedPrimitive, ImageData, Primitive, TextureId,
        textures::{TextureFilter, TextureOptions, TextureWrapMode},
    },
};
use egui_winit::State as EguiWinitState;
use wgpu::util::DeviceExt;
use winit::{event::WindowEvent, window::Window};

const UI_SHADER: &str = r#"
struct ScreenUniform {
    size_in_points: vec2<f32>,
    _padding: vec2<f32>,
};

@group(0) @binding(0)
var<uniform> screen: ScreenUniform;

@group(1) @binding(0)
var ui_texture: texture_2d<f32>;

@group(1) @binding(1)
var ui_sampler: sampler;

struct VertexInput {
    @location(0) pos: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
};

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var output: VertexOutput;
    let ndc = vec2<f32>(
        (input.pos.x / screen.size_in_points.x) * 2.0 - 1.0,
        1.0 - (input.pos.y / screen.size_in_points.y) * 2.0,
    );
    output.clip_position = vec4<f32>(ndc, 0.0, 1.0);
    output.uv = input.uv;
    output.color = input.color;
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let texel = textureSample(ui_texture, ui_sampler, input.uv);
    return texel * input.color;
}
"#;

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct ScreenUniform {
    size_in_points: [f32; 2],
    _padding: [f32; 2],
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuVertex {
    pos: [f32; 2],
    uv: [f32; 2],
    color: [f32; 4],
}

impl GpuVertex {
    fn from_egui(vertex: &egui::epaint::Vertex) -> Self {
        let [r, g, b, a] = vertex.color.to_array();
        let color = Rgba::from_srgba_premultiplied(r, g, b, a).to_array();
        Self {
            pos: [vertex.pos.x, vertex.pos.y],
            uv: [vertex.uv.x, vertex.uv.y],
            color,
        }
    }
}

#[derive(Copy, Clone, Debug)]
struct ScreenDescriptor {
    size_in_pixels: [u32; 2],
    pixels_per_point: f32,
}

impl ScreenDescriptor {
    fn size_in_points(self) -> [f32; 2] {
        [
            self.size_in_pixels[0] as f32 / self.pixels_per_point.max(1e-4),
            self.size_in_pixels[1] as f32 / self.pixels_per_point.max(1e-4),
        ]
    }
}

#[derive(Debug)]
struct PreparedMesh {
    clip_rect: egui::Rect,
    texture_id: TextureId,
    vertex_range: Range<u64>,
    index_range: Range<u64>,
    index_count: u32,
}

struct UiTexture {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    bind_group: wgpu::BindGroup,
    options: TextureOptions,
}

#[derive(Clone, Debug)]
pub struct MetricSnapshot {
    pub label: &'static str,
    pub value: f32,
    pub peak: f32,
}

#[derive(Clone, Debug)]
pub struct DebugUiSnapshot {
    pub status_line: String,
    pub source_label: String,
    pub is_diagnostics_view: bool,
    pub view_mode_label: &'static str,
    pub transport_label: &'static str,
    pub beat_label: &'static str,
    pub reactivity_label: &'static str,
    pub graph_preset_label: &'static str,
    pub graph_normalization_label: &'static str,
    pub graph_framing_label: &'static str,
    pub graph_history_points: usize,
    pub axis_labels: [(&'static str, &'static str); 4],
    pub tuning_focus_label: &'static str,
    pub tuning_value_label: String,
    pub tuning_step_label: String,
    pub metrics: Vec<MetricSnapshot>,
}

#[derive(Copy, Clone, Debug, Default)]
pub struct DebugUiEventResponse {
    pub consumed: bool,
    pub repaint: bool,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum DebugUiAction {
    ToggleViewMode,
}

pub struct PreparedDebugUi {
    meshes: Vec<PreparedMesh>,
    screen_descriptor: ScreenDescriptor,
    free_texture_ids: Vec<egui::TextureId>,
    pub actions: Vec<DebugUiAction>,
}

pub struct DebugUi {
    context: egui::Context,
    state: EguiWinitState,
    pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    uniform_bind_group: wgpu::BindGroup,
    texture_bind_group_layout: wgpu::BindGroupLayout,
    vertex_buffer: wgpu::Buffer,
    vertex_buffer_capacity: usize,
    index_buffer: wgpu::Buffer,
    index_buffer_capacity: usize,
    textures: HashMap<TextureId, UiTexture>,
    samplers: HashMap<TextureOptions, wgpu::Sampler>,
    visible: bool,
}

impl DebugUi {
    pub fn new(window: &Window, device: &wgpu::Device, output_format: wgpu::TextureFormat) -> Self {
        let context = egui::Context::default();
        install_visuals(&context);

        let state = EguiWinitState::new(
            context.clone(),
            egui::ViewportId::ROOT,
            window,
            Some(window.scale_factor() as f32),
            None,
            Some(device.limits().max_texture_dimension_2d as usize),
        );

        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Debug UI Uniform Buffer"),
            contents: bytemuck::cast_slice(&[ScreenUniform {
                size_in_points: [1.0, 1.0],
                _padding: [0.0; 2],
            }]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let uniform_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Debug UI Uniform Layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Debug UI Uniform Bind Group"),
            layout: &uniform_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        let texture_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Debug UI Texture Layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
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

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Debug UI Shader"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(UI_SHADER)),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Debug UI Pipeline Layout"),
            bind_group_layouts: &[&uniform_bind_group_layout, &texture_bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Debug UI Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<GpuVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x2,
                        1 => Float32x2,
                        2 => Float32x4,
                    ],
                }],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: output_format,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let vertex_buffer_capacity = std::mem::size_of::<GpuVertex>();
        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Debug UI Vertex Buffer"),
            size: vertex_buffer_capacity as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let index_buffer_capacity = std::mem::size_of::<u32>();
        let index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Debug UI Index Buffer"),
            size: index_buffer_capacity as u64,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            context,
            state,
            pipeline,
            uniform_buffer,
            uniform_bind_group,
            texture_bind_group_layout,
            vertex_buffer,
            vertex_buffer_capacity,
            index_buffer,
            index_buffer_capacity,
            textures: HashMap::new(),
            samplers: HashMap::new(),
            visible: true,
        }
    }

    pub fn toggle(&mut self) {
        self.visible = !self.visible;
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    pub fn on_window_event(&mut self, window: &Window, event: &WindowEvent) -> DebugUiEventResponse {
        if !self.visible {
            return DebugUiEventResponse::default();
        }

        let response = self.state.on_window_event(window, event);
        DebugUiEventResponse {
            consumed: response.consumed,
            repaint: response.repaint,
        }
    }

    pub fn prepare_frame(
        &mut self,
        window: &Window,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface_size: [u32; 2],
        snapshot: &DebugUiSnapshot,
    ) -> Option<PreparedDebugUi> {
        if !self.visible {
            return None;
        }

        let raw_input = self.state.take_egui_input(window);
        self.context.begin_pass(raw_input);
        let mut actions = Vec::new();
        render_debug_window(&self.context, snapshot, &mut actions);
        let full_output = self.context.end_pass();

        let egui::FullOutput {
            platform_output,
            textures_delta,
            shapes,
            ..
        } = full_output;

        self.state.handle_platform_output(window, platform_output);

        let pixels_per_point = window.scale_factor() as f32;
        let paint_jobs = self.context.tessellate(shapes, pixels_per_point);

        for (texture_id, image_delta) in &textures_delta.set {
            self.update_texture(device, queue, *texture_id, image_delta);
        }

        let screen_descriptor = ScreenDescriptor {
            size_in_pixels: surface_size,
            pixels_per_point,
        };

        queue.write_buffer(
            &self.uniform_buffer,
            0,
            bytemuck::cast_slice(&[ScreenUniform {
                size_in_points: screen_descriptor.size_in_points(),
                _padding: [0.0; 2],
            }]),
        );

        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        let mut meshes = Vec::new();

        for ClippedPrimitive {
            clip_rect,
            primitive,
        } in paint_jobs
        {
            let Primitive::Mesh(mesh) = primitive else {
                continue;
            };

            let vertex_start = vertices.len();
            vertices.extend(mesh.vertices.iter().map(GpuVertex::from_egui));
            let vertex_end = vertices.len();

            let index_start = indices.len();
            indices.extend(mesh.indices.iter().copied());
            let index_end = indices.len();

            meshes.push(PreparedMesh {
                clip_rect,
                texture_id: mesh.texture_id,
                vertex_range: (vertex_start * std::mem::size_of::<GpuVertex>()) as u64
                    ..(vertex_end * std::mem::size_of::<GpuVertex>()) as u64,
                index_range: (index_start * std::mem::size_of::<u32>()) as u64
                    ..(index_end * std::mem::size_of::<u32>()) as u64,
                index_count: mesh.indices.len() as u32,
            });
        }

        self.ensure_vertex_capacity(device, vertices.len().max(1));
        self.ensure_index_capacity(device, indices.len().max(1));

        if !vertices.is_empty() {
            queue.write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&vertices));
        }

        if !indices.is_empty() {
            queue.write_buffer(&self.index_buffer, 0, bytemuck::cast_slice(&indices));
        }

        Some(PreparedDebugUi {
            meshes,
            screen_descriptor,
            free_texture_ids: textures_delta.free,
            actions,
        })
    }

    pub fn render(&self, render_pass: &mut wgpu::RenderPass<'_>, frame: &PreparedDebugUi) {
        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_bind_group(0, &self.uniform_bind_group, &[]);

        for mesh in &frame.meshes {
            let Some((x, y, width, height)) = clip_rect_to_scissor(
                mesh.clip_rect,
                frame.screen_descriptor.pixels_per_point,
                frame.screen_descriptor.size_in_pixels,
            ) else {
                continue;
            };

            let Some(texture) = self.textures.get(&mesh.texture_id) else {
                continue;
            };

            render_pass.set_scissor_rect(x, y, width, height);
            render_pass.set_bind_group(1, &texture.bind_group, &[]);
            render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(mesh.vertex_range.clone()));
            render_pass.set_index_buffer(
                self.index_buffer.slice(mesh.index_range.clone()),
                wgpu::IndexFormat::Uint32,
            );
            render_pass.draw_indexed(0..mesh.index_count, 0, 0..1);
        }
    }

    pub fn finish_frame(&mut self, frame: PreparedDebugUi) {
        for texture_id in frame.free_texture_ids {
            self.textures.remove(&texture_id);
        }
    }

    fn ensure_vertex_capacity(&mut self, device: &wgpu::Device, vertices: usize) {
        let required = vertices * std::mem::size_of::<GpuVertex>();
        if required <= self.vertex_buffer_capacity {
            return;
        }

        self.vertex_buffer_capacity = required.next_power_of_two();
        self.vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Debug UI Vertex Buffer"),
            size: self.vertex_buffer_capacity as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
    }

    fn ensure_index_capacity(&mut self, device: &wgpu::Device, indices: usize) {
        let required = indices * std::mem::size_of::<u32>();
        if required <= self.index_buffer_capacity {
            return;
        }

        self.index_buffer_capacity = required.next_power_of_two();
        self.index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Debug UI Index Buffer"),
            size: self.index_buffer_capacity as u64,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
    }

    fn update_texture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture_id: TextureId,
        image_delta: &egui::epaint::ImageDelta,
    ) {
        let ImageData::Color(image) = &image_delta.image;
        let bytes = color_image_bytes(image);
        let width = image.size[0] as u32;
        let height = image.size[1] as u32;

        let layout = wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * 4),
            rows_per_image: Some(height),
        };

        if image_delta.pos.is_none() || !self.textures.contains_key(&texture_id) {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Debug UI Texture"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
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
                &bytes,
                layout,
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );

            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let sampler = self.sampler(device, image_delta.options).clone();
            let bind_group = create_texture_bind_group(
                device,
                &self.texture_bind_group_layout,
                &view,
                &sampler,
            );

            self.textures.insert(
                texture_id,
                UiTexture {
                    texture,
                    view,
                    bind_group,
                    options: image_delta.options,
                },
            );
            return;
        }

        let needs_sampler_refresh = self
            .textures
            .get(&texture_id)
            .is_some_and(|texture| texture.options != image_delta.options);
        let replacement_sampler = needs_sampler_refresh
            .then(|| self.sampler(device, image_delta.options).clone());

        let Some(texture) = self.textures.get_mut(&texture_id) else {
            return;
        };

        let Some([x, y]) = image_delta.pos else {
            return;
        };

        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture.texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: x as u32,
                    y: y as u32,
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            &bytes,
            layout,
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );

        if let Some(sampler) = replacement_sampler {
            texture.bind_group = create_texture_bind_group(
                device,
                &self.texture_bind_group_layout,
                &texture.view,
                &sampler,
            );
            texture.options = image_delta.options;
        }
    }

    fn sampler(&mut self, device: &wgpu::Device, options: TextureOptions) -> &wgpu::Sampler {
        self.samplers.entry(options).or_insert_with(|| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("Debug UI Sampler"),
                address_mode_u: address_mode(options.wrap_mode),
                address_mode_v: address_mode(options.wrap_mode),
                address_mode_w: address_mode(options.wrap_mode),
                mag_filter: filter_mode(options.magnification),
                min_filter: filter_mode(options.minification),
                mipmap_filter: options
                    .mipmap_mode
                    .map_or(wgpu::FilterMode::Nearest, filter_mode),
                ..Default::default()
            })
        })
    }
}

fn create_texture_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    view: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Debug UI Texture Bind Group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}

fn color_image_bytes(image: &egui::ColorImage) -> Vec<u8> {
    image
        .pixels
        .iter()
        .flat_map(|color| color.to_array())
        .collect()
}

fn address_mode(wrap_mode: TextureWrapMode) -> wgpu::AddressMode {
    match wrap_mode {
        TextureWrapMode::ClampToEdge => wgpu::AddressMode::ClampToEdge,
        TextureWrapMode::Repeat => wgpu::AddressMode::Repeat,
        TextureWrapMode::MirroredRepeat => wgpu::AddressMode::MirrorRepeat,
    }
}

fn filter_mode(filter: TextureFilter) -> wgpu::FilterMode {
    match filter {
        TextureFilter::Nearest => wgpu::FilterMode::Nearest,
        TextureFilter::Linear => wgpu::FilterMode::Linear,
    }
}

fn clip_rect_to_scissor(
    clip_rect: egui::Rect,
    pixels_per_point: f32,
    size_in_pixels: [u32; 2],
) -> Option<(u32, u32, u32, u32)> {
    let width = size_in_pixels[0] as f32;
    let height = size_in_pixels[1] as f32;

    let min_x = (clip_rect.min.x * pixels_per_point).floor().clamp(0.0, width) as u32;
    let min_y = (clip_rect.min.y * pixels_per_point).floor().clamp(0.0, height) as u32;
    let max_x = (clip_rect.max.x * pixels_per_point).ceil().clamp(min_x as f32, width) as u32;
    let max_y = (clip_rect.max.y * pixels_per_point).ceil().clamp(min_y as f32, height) as u32;

    let clip_width = max_x.saturating_sub(min_x);
    let clip_height = max_y.saturating_sub(min_y);
    if clip_width == 0 || clip_height == 0 {
        return None;
    }

    Some((min_x, min_y, clip_width, clip_height))
}

fn install_visuals(context: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.override_text_color = Some(Color32::from_rgb(228, 236, 246));
    visuals.window_fill = Color32::from_rgba_premultiplied(8, 12, 18, 236);
    visuals.panel_fill = Color32::from_rgba_premultiplied(8, 12, 18, 236);
    visuals.hyperlink_color = Color32::from_rgb(118, 214, 255);
    visuals.selection.bg_fill = Color32::from_rgb(18, 88, 126);
    context.set_visuals(visuals);
}

fn render_debug_window(
    context: &egui::Context,
    snapshot: &DebugUiSnapshot,
    actions: &mut Vec<DebugUiAction>,
) {
    egui::Window::new("Vizzy Retained UI")
        .anchor(Align2::RIGHT_TOP, egui::vec2(-16.0, 16.0))
        .default_width(340.0)
        .min_width(300.0)
        .resizable(true)
        .show(context, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Retained UI Spike").strong());
                ui.separator();
                ui.small("press ` to hide");
            });

            ui.add_space(4.0);
            ui.label(
                RichText::new(&snapshot.status_line)
                    .color(Color32::from_rgb(124, 219, 255))
                    .strong(),
            );
            ui.small(&snapshot.source_label);

            ui.add_space(6.0);
            if ui
                .button(if snapshot.is_diagnostics_view {
                    "Switch To Hero View"
                } else {
                    "Switch To Diagnostics Panels"
                })
                .clicked()
            {
                actions.push(DebugUiAction::ToggleViewMode);
            }

            ui.separator();

            Grid::new("vizzy_debug_summary")
                .num_columns(2)
                .spacing(egui::vec2(12.0, 6.0))
                .striped(true)
                .show(ui, |ui| {
                    summary_row(ui, "View", snapshot.view_mode_label);
                    summary_row(ui, "Transport", snapshot.transport_label);
                    summary_row(ui, "Beat", snapshot.beat_label);
                    summary_row(ui, "Reactivity", snapshot.reactivity_label);
                    summary_row(ui, "Preset", snapshot.graph_preset_label);
                    summary_row(
                        ui,
                        "Graph",
                        format!(
                            "{} / {} / {} pts",
                            snapshot.graph_normalization_label,
                            snapshot.graph_framing_label,
                            snapshot.graph_history_points,
                        ),
                    );
                    summary_row(
                        ui,
                        "Axes",
                        format!(
                            "{} {} | {} {} | {} {} | {} {}",
                            snapshot.axis_labels[0].0,
                            snapshot.axis_labels[0].1,
                            snapshot.axis_labels[1].0,
                            snapshot.axis_labels[1].1,
                            snapshot.axis_labels[2].0,
                            snapshot.axis_labels[2].1,
                            snapshot.axis_labels[3].0,
                            snapshot.axis_labels[3].1,
                        ),
                    );
                    summary_row(ui, "Tuning", snapshot.tuning_focus_label);
                    summary_row(ui, "Value", &snapshot.tuning_value_label);
                    summary_row(ui, "Step", &snapshot.tuning_step_label);
                });

            ui.separator();
            ui.label(RichText::new("Signal Snapshot").strong());

            for metric in &snapshot.metrics {
                ui.horizontal(|ui| {
                    ui.monospace(format!("{:>4}", metric.label));
                    ui.add_sized(
                        [180.0, 18.0],
                        ProgressBar::new(metric.value.clamp(0.0, 1.0))
                            .text(format!("{:.2}", metric.value)),
                    );
                    ui.small(format!("pk {:.2}", metric.peak.clamp(0.0, 1.0)));
                });
            }
        });
}

fn summary_row(ui: &mut egui::Ui, label: &str, value: impl AsRef<str>) {
    ui.small(RichText::new(label).strong());
    ui.monospace(value.as_ref());
    ui.end_row();
}