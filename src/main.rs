use winit::{
    application::ApplicationHandler,
    event::{ElementState, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{KeyCode, ModifiersState, PhysicalKey},
    window::WindowId,
};
use std::array;
use std::cell::Cell;
use std::collections::VecDeque;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::Arc;
use std::time::{Instant, SystemTime};
use pollster::block_on;
use serde::{Deserialize, Serialize};
use wgpu::util::DeviceExt;

mod audio;

const LOG_BIN_UNIFORM_ROWS: usize = audio::LOG_BAND_COUNT / 4;
const GRAPH_HISTORY_LENGTH: usize = 128;
const GRAPH_FEATURE_COUNT: usize = 12;
const GRAPH_NORMALIZATION_EPSILON: f32 = 1e-4;
const GRAPH_SIGNAL_EPSILON: f32 = 0.01;
const UI_CONFIG_FILE: &str = "ui_settings.toml";

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct StandardUniforms {
    resolution: [f32; 2],
    _padding: [f32; 2],  // WGPU 16-byte alignment
    time: f32,
    audio_bass: f32,
    audio_mid: f32,
    audio_treble: f32,
    audio_loudness: f32,
    audio_peak: f32,
    audio_beat: f32,
    audio_centroid: f32,
    audio_sub_bass: f32,
    audio_presence: f32,
    audio_air: f32,
    analysis_nyquist_hz: f32,
    status_color: [f32; 4],
    loading_progress: f32,
    loading_active: f32,
    show_help: f32,
    active_tuning_focus: f32,
    active_tuning_value: f32,
    active_tuning_step: f32,
    active_tuning_is_hz: f32,
    active_axis_focus: f32,
    axis_x_source: f32,
    axis_y_source: f32,
    axis_z_source: f32,
    axis_size_source: f32,
    split_sub_bass_max_hz: f32,
    split_bass_max_hz: f32,
    split_mid_max_hz: f32,
    split_presence_min_hz: f32,
    split_presence_max_hz: f32,
    split_air_min_hz: f32,
    _graph_view_padding: [f32; 2],
    graph_view_center: [f32; 3],
    graph_view_extent: f32,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct LogBinsUniforms {
    bins: [[f32; 4]; LOG_BIN_UNIFORM_ROWS],
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct GraphHistoryUniforms {
    points: [[f32; 4]; GRAPH_HISTORY_LENGTH],
}

#[derive(Copy, Clone, Debug)]
struct GraphViewState {
    center: [f32; 3],
    extent: f32,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum GraphFramingMode {
    Locality,
    Fixed,
}

impl GraphFramingMode {
    fn toggle(self) -> Self {
        match self {
            Self::Locality => Self::Fixed,
            Self::Fixed => Self::Locality,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Locality => "LOC",
            Self::Fixed => "FIX",
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum BeatResponseMode {
    Smooth,
    Balanced,
    Tight,
}

struct ShaderHotReload {
    path: PathBuf,
    last_seen_modified: Option<SystemTime>,
}

struct ShaderOverlayState {
    color: [f32; 3],
    intensity: f32,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum GraphAxisSource {
    SubBass,
    Bass,
    Mid,
    Treble,
    Presence,
    Air,
    Loudness,
    Peak,
    Beat,
    Centroid,
    LogFrequency,
    BinIndex,
}

impl GraphAxisSource {
    fn next(self) -> Self {
        match self {
            Self::SubBass => Self::Bass,
            Self::Bass => Self::Mid,
            Self::Mid => Self::Treble,
            Self::Treble => Self::Presence,
            Self::Presence => Self::Air,
            Self::Air => Self::Loudness,
            Self::Loudness => Self::Peak,
            Self::Peak => Self::Beat,
            Self::Beat => Self::Centroid,
            Self::Centroid => Self::LogFrequency,
            Self::LogFrequency => Self::BinIndex,
            Self::BinIndex => Self::SubBass,
        }
    }

    fn previous(self) -> Self {
        match self {
            Self::SubBass => Self::BinIndex,
            Self::Bass => Self::SubBass,
            Self::Mid => Self::Bass,
            Self::Treble => Self::Mid,
            Self::Presence => Self::Treble,
            Self::Air => Self::Presence,
            Self::Loudness => Self::Air,
            Self::Peak => Self::Loudness,
            Self::Beat => Self::Peak,
            Self::Centroid => Self::Beat,
            Self::LogFrequency => Self::Centroid,
            Self::BinIndex => Self::LogFrequency,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::SubBass => "SUB BASS",
            Self::Bass => "BASS",
            Self::Mid => "MID",
            Self::Treble => "TREBLE",
            Self::Presence => "PRESENCE",
            Self::Air => "AIR",
            Self::Loudness => "LOUD",
            Self::Peak => "PEAK",
            Self::Beat => "BEAT",
            Self::Centroid => "CENT",
            Self::LogFrequency => "LOG HZ",
            Self::BinIndex => "BIN IDX",
        }
    }

    fn index(self) -> u32 {
        match self {
            Self::SubBass => 0,
            Self::Bass => 1,
            Self::Mid => 2,
            Self::Treble => 3,
            Self::Presence => 4,
            Self::Air => 5,
            Self::Loudness => 6,
            Self::Peak => 7,
            Self::Beat => 8,
            Self::Centroid => 9,
            Self::LogFrequency => 10,
            Self::BinIndex => 11,
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum AxisSelectionFocus {
    X,
    Y,
    Z,
    Size,
}

impl AxisSelectionFocus {
    fn index(self) -> u32 {
        match self {
            Self::X => 0,
            Self::Y => 1,
            Self::Z => 2,
            Self::Size => 3,
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum GraphNormalizationMode {
    Absolute,
    Normalized,
}

impl GraphNormalizationMode {
    fn toggle(self) -> Self {
        match self {
            Self::Absolute => Self::Normalized,
            Self::Normalized => Self::Absolute,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Absolute => "ABS",
            Self::Normalized => "NORM",
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(default)]
struct UiSettings {
    axis_x_source: GraphAxisSource,
    axis_y_source: GraphAxisSource,
    axis_z_source: GraphAxisSource,
    axis_size_source: GraphAxisSource,
    graph_normalization_mode: GraphNormalizationMode,
    graph_framing_mode: GraphFramingMode,
    graph_fixed_center: [f32; 3],
    graph_fixed_extent: f32,
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            axis_x_source: GraphAxisSource::Bass,
            axis_y_source: GraphAxisSource::Beat,
            axis_z_source: GraphAxisSource::Centroid,
            axis_size_source: GraphAxisSource::Peak,
            graph_normalization_mode: GraphNormalizationMode::Absolute,
            graph_framing_mode: GraphFramingMode::Locality,
            graph_fixed_center: [0.5, 0.5, 0.5],
            graph_fixed_extent: 0.5,
        }
    }
}

fn ui_settings_path() -> Option<PathBuf> {
    dirs::config_dir().map(|path| path.join(audio::ANALYSIS_CONFIG_DIR).join(UI_CONFIG_FILE))
}

fn load_ui_settings() -> UiSettings {
    let Some(path) = ui_settings_path() else {
        return UiSettings::default();
    };

    fs::read_to_string(&path)
        .ok()
        .and_then(|contents| toml::from_str(&contents).ok())
        .unwrap_or_default()
}

fn save_ui_settings(settings: &UiSettings) {
    let Some(path) = ui_settings_path() else {
        return;
    };
    let Some(parent) = path.parent() else {
        return;
    };

    if let Err(error) = fs::create_dir_all(parent) {
        eprintln!("Failed to create UI settings directory {}: {}", parent.display(), error);
        return;
    }

    let serialized = match toml::to_string_pretty(settings) {
        Ok(serialized) => serialized,
        Err(error) => {
            eprintln!("Failed to serialize UI settings: {}", error);
            return;
        }
    };

    if let Err(error) = fs::write(&path, serialized) {
        eprintln!("Failed to write UI settings to {}: {}", path.display(), error);
    }
}

#[derive(Copy, Clone, Debug)]
enum TuningFocus {
    RmsThreshold,
    PeakThreshold,
    SubBassMax,
    BassMax,
    MidMax,
    PresenceMin,
    PresenceMax,
    AirMin,
}

impl TuningFocus {
    fn next(self) -> Self {
        match self {
            Self::RmsThreshold => Self::PeakThreshold,
            Self::PeakThreshold => Self::SubBassMax,
            Self::SubBassMax => Self::BassMax,
            Self::BassMax => Self::MidMax,
            Self::MidMax => Self::PresenceMin,
            Self::PresenceMin => Self::PresenceMax,
            Self::PresenceMax => Self::AirMin,
            Self::AirMin => Self::RmsThreshold,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::RmsThreshold => "RMS Gate",
            Self::PeakThreshold => "Peak Gate",
            Self::SubBassMax => "Sub Bass Max",
            Self::BassMax => "Bass Max",
            Self::MidMax => "Mid Max",
            Self::PresenceMin => "Presence Min",
            Self::PresenceMax => "Presence Max",
            Self::AirMin => "Air Min",
        }
    }

    fn index(self) -> u32 {
        match self {
            Self::RmsThreshold => 0,
            Self::PeakThreshold => 1,
            Self::SubBassMax => 2,
            Self::BassMax => 3,
            Self::MidMax => 4,
            Self::PresenceMin => 5,
            Self::PresenceMax => 6,
            Self::AirMin => 7,
        }
    }

    fn is_frequency(self) -> bool {
        !matches!(self, Self::RmsThreshold | Self::PeakThreshold)
    }

    fn base_step(self) -> f32 {
        match self {
            Self::RmsThreshold => 0.002,
            Self::PeakThreshold => 0.010,
            Self::SubBassMax => 5.0,
            Self::BassMax => 20.0,
            Self::MidMax => 100.0,
            Self::PresenceMin => 100.0,
            Self::PresenceMax => 100.0,
            Self::AirMin => 200.0,
        }
    }
}

impl BeatResponseMode {
    fn next(self) -> Self {
        match self {
            Self::Smooth => Self::Balanced,
            Self::Balanced => Self::Tight,
            Self::Tight => Self::Smooth,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Smooth => "Smooth",
            Self::Balanced => "Balanced",
            Self::Tight => "Tight",
        }
    }

    fn thresholds(self) -> (f32, f32, f32, f32, f32) {
        match self {
            Self::Smooth => (1.18, 1.30, 0.48, 0.95, 0.95),
            Self::Balanced => (1.35, 1.55, 0.35, 1.25, 1.25),
            Self::Tight => (1.55, 1.80, 0.24, 1.45, 1.45),
        }
    }

    fn envelope(self) -> (f32, f32) {
        match self {
            Self::Smooth => (0.16, 0.095),
            Self::Balanced => (0.22, 0.080),
            Self::Tight => (0.28, 0.060),
        }
    }
}

struct App {
    window: Option<Arc<winit::window::Window>>,
    surface: Option<wgpu::Surface<'static>>,
    device: Option<wgpu::Device>,
    queue: Option<wgpu::Queue>,
    config: Option<wgpu::SurfaceConfiguration>,
    audio_loader: Option<Receiver<audio::AudioLoadUpdate>>,
    audio_analyzer: Option<audio::AudioAnalyzer>,
    analysis_controller: Option<audio::AnalysisController>,
    transport_controller: Option<audio::TransportController>,
    render_pipeline: Option<wgpu::RenderPipeline>,
    uniform_buffer: Option<wgpu::Buffer>,
    log_bins_buffer: Option<wgpu::Buffer>,
    graph_points_buffer: Option<wgpu::Buffer>,
    bind_group_layout: Option<wgpu::BindGroupLayout>,
    bind_group: Option<wgpu::BindGroup>,
    start_time: Option<Instant>,
    last_title_refresh: Cell<Option<Instant>>,
    current_sub_bass: f32,
    current_bass: f32,
    current_mid: f32,
    current_treble: f32,
    current_presence: f32,
    current_air: f32,
    current_loudness: f32,
    current_peak: f32,
    current_beat: f32,
    current_centroid: f32,
    loudness_history: VecDeque<f32>,
    feature_history: VecDeque<[f32; GRAPH_FEATURE_COUNT]>,
    current_log_bins: [f32; audio::LOG_BAND_COUNT],
    analysis_nyquist_hz: f32,
    audio_source: audio::AudioSource,
    audio_loading_progress: f32,
    audio_loading_stage: String,
    audio_loading_error: Option<String>,
    beat_response_mode: BeatResponseMode,
    tuning_focus: TuningFocus,
    axis_focus: AxisSelectionFocus,
    axis_x_source: GraphAxisSource,
    axis_y_source: GraphAxisSource,
    axis_z_source: GraphAxisSource,
    axis_size_source: GraphAxisSource,
    graph_normalization_mode: GraphNormalizationMode,
    graph_framing_mode: GraphFramingMode,
    graph_view_state: GraphViewState,
    graph_fixed_view: GraphViewState,
    modifiers: ModifiersState,
    show_shortcuts: bool,
    shader_hot_reload: ShaderHotReload,
    shader_overlay: ShaderOverlayState,
}

fn shader_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("shader.wgsl")
}

fn read_shader_source(path: &Path) -> Result<(String, SystemTime), String> {
    let metadata = fs::metadata(path)
        .map_err(|error| format!("Failed to read shader metadata for {}: {}", path.display(), error))?;
    let modified = metadata
        .modified()
        .map_err(|error| format!("Failed to read shader modified time for {}: {}", path.display(), error))?;
    let source = fs::read_to_string(path)
        .map_err(|error| format!("Failed to read shader source from {}: {}", path.display(), error))?;
    Ok((source, modified))
}

fn create_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("Uniform Bind Group Layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
    })
}

fn build_log_bins_uniforms(log_bins: &[f32; audio::LOG_BAND_COUNT]) -> LogBinsUniforms {
    let mut bins = [[0.0; 4]; LOG_BIN_UNIFORM_ROWS];
    for (index, value) in log_bins.iter().enumerate() {
        bins[index / 4][index % 4] = *value;
    }
    LogBinsUniforms { bins }
}

fn feature_value_for_axis(sample: &[f32; GRAPH_FEATURE_COUNT], axis: GraphAxisSource) -> f32 {
    match axis {
        GraphAxisSource::SubBass => sample[0],
        GraphAxisSource::Bass => sample[1],
        GraphAxisSource::Mid => sample[2],
        GraphAxisSource::Treble => sample[3],
        GraphAxisSource::Presence => sample[4],
        GraphAxisSource::Air => sample[5],
        GraphAxisSource::Loudness => sample[6],
        GraphAxisSource::Peak => sample[7],
        GraphAxisSource::Beat => sample[8],
        GraphAxisSource::Centroid => sample[9],
        GraphAxisSource::LogFrequency => sample[10],
        GraphAxisSource::BinIndex => sample[11],
    }
}

fn build_graph_history_uniforms(
    history: &VecDeque<[f32; GRAPH_FEATURE_COUNT]>,
    axis_x: GraphAxisSource,
    axis_y: GraphAxisSource,
    axis_z: GraphAxisSource,
    size_source: GraphAxisSource,
    normalization_mode: GraphNormalizationMode,
) -> GraphHistoryUniforms {
    let mut points = [[0.0, 0.0, 0.0, -1.0]; GRAPH_HISTORY_LENGTH];

    for (index, sample) in history.iter().enumerate().take(GRAPH_HISTORY_LENGTH) {
        points[index] = [
            feature_value_for_axis(sample, axis_x),
            feature_value_for_axis(sample, axis_y),
            feature_value_for_axis(sample, axis_z),
            feature_value_for_axis(sample, size_source),
        ];
    }

    if normalization_mode == GraphNormalizationMode::Normalized {
        normalize_graph_history_points(&mut points);
    }

    GraphHistoryUniforms { points }
}

fn normalize_graph_history_points(points: &mut [[f32; 4]; GRAPH_HISTORY_LENGTH]) {
    let mut mins = [f32::INFINITY; 3];
    let mut maxs = [f32::NEG_INFINITY; 3];
    let mut has_points = false;

    for point in points.iter() {
        if point[3] < 0.0 {
            continue;
        }
        has_points = true;
        for axis in 0..3 {
            mins[axis] = mins[axis].min(point[axis]);
            maxs[axis] = maxs[axis].max(point[axis]);
        }
    }

    if !has_points {
        return;
    }

    for point in points.iter_mut() {
        if point[3] < 0.0 {
            continue;
        }
        for axis in 0..3 {
            let range = maxs[axis] - mins[axis];
            point[axis] = if range <= GRAPH_NORMALIZATION_EPSILON {
                0.5
            } else {
                ((point[axis] - mins[axis]) / range).clamp(0.0, 1.0)
            };
        }
    }
}

fn build_graph_view_state(points: &[[f32; 4]; GRAPH_HISTORY_LENGTH]) -> GraphViewState {
    let mut mins = [f32::INFINITY; 3];
    let mut maxs = [f32::NEG_INFINITY; 3];
    let mut has_points = false;

    for point in points.iter() {
        if point[3] < 0.0 {
            continue;
        }
        has_points = true;
        for axis in 0..3 {
            mins[axis] = mins[axis].min(point[axis]);
            maxs[axis] = maxs[axis].max(point[axis]);
        }
    }

    if !has_points {
        return GraphViewState {
            center: [0.5, 0.5, 0.5],
            extent: 0.5,
        };
    }

    let mut center = [0.5; 3];
    let mut max_half_range: f32 = 0.0;
    for axis in 0..3 {
        center[axis] = (mins[axis] + maxs[axis]) * 0.5;
        max_half_range = max_half_range.max((maxs[axis] - mins[axis]) * 0.5);
    }

    GraphViewState {
        center,
        extent: max_half_range.max(GRAPH_NORMALIZATION_EPSILON) * 1.15,
    }
}

fn should_capture_graph_sample(
    sample: &[f32; GRAPH_FEATURE_COUNT],
    axis_x: GraphAxisSource,
    axis_y: GraphAxisSource,
    axis_z: GraphAxisSource,
    size_source: GraphAxisSource,
) -> bool {
    [axis_x, axis_y, axis_z, size_source]
        .into_iter()
        .any(|axis| feature_value_for_axis(sample, axis) > GRAPH_SIGNAL_EPSILON)
}

fn smooth_graph_view_towards(current: &mut GraphViewState, target: GraphViewState) {
    for axis in 0..3 {
        envelope_follow(&mut current.center[axis], target.center[axis], 0.10, 0.08);
    }
    envelope_follow(&mut current.extent, target.extent, 0.08, 0.06);
}

fn graph_view_control_multiplier(modifiers: ModifiersState) -> f32 {
    if modifiers.shift_key() {
        2.5
    } else if modifiers.control_key() {
        0.5
    } else {
        1.0
    }
}

fn build_feature_snapshot(
    sub_bass: f32,
    bass: f32,
    mid: f32,
    treble: f32,
    presence: f32,
    air: f32,
    loudness: f32,
    peak: f32,
    beat: f32,
    centroid: f32,
    log_frequency: f32,
    bin_index: f32,
) -> [f32; GRAPH_FEATURE_COUNT] {
    [
        sub_bass,
        bass,
        mid,
        treble,
        presence,
        air,
        loudness,
        peak,
        beat,
        centroid,
        log_frequency,
        bin_index,
    ]
}

fn create_render_pipeline(
    device: &wgpu::Device,
    config: &wgpu::SurfaceConfiguration,
    bind_group_layout: &wgpu::BindGroupLayout,
    shader_source: &str,
) -> Result<wgpu::RenderPipeline, String> {
    device.push_error_scope(wgpu::ErrorFilter::Validation);

    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Track Shader"),
        source: wgpu::ShaderSource::Wgsl(shader_source.into()),
    });

    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Render Pipeline Layout"),
        bind_group_layouts: &[bind_group_layout],
        push_constant_ranges: &[],
    });

    let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Render Pipeline"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: config.format,
                blend: Some(wgpu::BlendState::REPLACE),
                write_mask: wgpu::ColorWrites::ALL,
            })],
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
        multisample: wgpu::MultisampleState {
            count: 1,
            mask: !0,
            alpha_to_coverage_enabled: false,
        },
        multiview: None,
        cache: None,
    });

    if let Some(error) = block_on(device.pop_error_scope()) {
        return Err(format!("Shader compilation failed: {}", error));
    }

    Ok(render_pipeline)
}

fn compress_audio(energy: f32, gain: f32) -> f32 {
    1.0 - (-energy.max(0.0) * gain).exp()
}

fn normalized_log_frequency(centroid: f32, nyquist_hz: f32) -> f32 {
    let nyquist = nyquist_hz.max(audio::MIN_ANALYSIS_FREQUENCY_HZ * 2.0);
    let clamped_centroid = centroid.clamp(0.0, 1.0);
    let frequency_hz = (clamped_centroid * nyquist).clamp(audio::MIN_ANALYSIS_FREQUENCY_HZ, nyquist);
    let ratio = (nyquist / audio::MIN_ANALYSIS_FREQUENCY_HZ).max(1.0 + f32::EPSILON);
    ((frequency_hz / audio::MIN_ANALYSIS_FREQUENCY_HZ).ln() / ratio.ln()).clamp(0.0, 1.0)
}

fn normalized_peak_log_bin(log_bins: &[f32; audio::LOG_BAND_COUNT]) -> f32 {
    let mut max_index = 0usize;
    let mut max_value = f32::NEG_INFINITY;

    for (index, value) in log_bins.iter().enumerate() {
        if *value > max_value {
            max_index = index;
            max_value = *value;
        }
    }

    if audio::LOG_BAND_COUNT <= 1 {
        0.0
    } else {
        max_index as f32 / (audio::LOG_BAND_COUNT - 1) as f32
    }
}

fn detect_beat(
    loudness: f32,
    peak: f32,
    history: &VecDeque<f32>,
    mode: BeatResponseMode,
) -> f32 {
    if history.len() < 10 {
        return 0.0;
    }

    let mean = history.iter().sum::<f32>() / history.len() as f32;
    let variance = history
        .iter()
        .map(|value| {
            let delta = value - mean;
            delta * delta
        })
        .sum::<f32>()
        / history.len() as f32;
    let std_dev = variance.sqrt();
    let (energy_sigma, transient_ratio, transient_scale, emphasis, gain) = mode.thresholds();
    let energy_spike = ((loudness - (mean + std_dev * energy_sigma)) / (mean + std_dev + 1e-4)).max(0.0);
    let transient_spike = ((peak - loudness * transient_ratio) / transient_scale).max(0.0);
    compress_audio(energy_spike * emphasis + transient_spike * 0.75, gain)
}

fn source_label(source: &audio::AudioSource) -> String {
    match source {
        audio::AudioSource::InputDevice => "Mic".to_string(),
        audio::AudioSource::File(path) => path
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| format!("File: {}", name))
            .unwrap_or_else(|| "File".to_string()),
    }
}

fn detect_audio_source() -> audio::AudioSource {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut args = env::args().skip(1);
    if let Some(arg) = args.next() {
        if arg == "--mic" {
            return audio::AudioSource::InputDevice;
        }

        let path = PathBuf::from(&arg);
        if path.is_absolute() {
            return audio::AudioSource::File(path);
        }

        return audio::AudioSource::File(manifest_dir.join(path));
    }

    let bundled_mp3 = fs::read_dir(&manifest_dir)
        .ok()
        .and_then(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .find(|path| {
                    path.extension()
                        .and_then(|ext| ext.to_str())
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("mp3"))
                })
        });

    if let Some(path) = bundled_mp3 {
        println!("Defaulting to bundled MP3 input: {}", path.display());
        return audio::AudioSource::File(path);
    }

    audio::AudioSource::InputDevice
}

impl App {
    fn new() -> Self {
        let ui_settings = load_ui_settings();
        Self {
            window: None,
            surface: None,
            device: None,
            queue: None,
            config: None,
            audio_loader: None,
            audio_analyzer: None,
            analysis_controller: None,
            transport_controller: None,
            render_pipeline: None,
            uniform_buffer: None,
            log_bins_buffer: None,
            graph_points_buffer: None,
            bind_group_layout: None,
            bind_group: None,
            start_time: None,
            last_title_refresh: Cell::new(None),
            current_sub_bass: 0.0,
            current_bass: 0.0,
            current_mid: 0.0,
            current_treble: 0.0,
            current_presence: 0.0,
            current_air: 0.0,
            current_loudness: 0.0,
            current_peak: 0.0,
            current_beat: 0.0,
            current_centroid: 0.0,
            loudness_history: VecDeque::with_capacity(48),
            feature_history: VecDeque::with_capacity(GRAPH_HISTORY_LENGTH),
            current_log_bins: [0.0; audio::LOG_BAND_COUNT],
            analysis_nyquist_hz: 22_050.0,
            audio_source: detect_audio_source(),
            audio_loading_progress: 0.0,
            audio_loading_stage: "Waiting to load audio".to_string(),
            audio_loading_error: None,
            beat_response_mode: BeatResponseMode::Balanced,
            tuning_focus: TuningFocus::RmsThreshold,
            axis_focus: AxisSelectionFocus::X,
            axis_x_source: ui_settings.axis_x_source,
            axis_y_source: ui_settings.axis_y_source,
            axis_z_source: ui_settings.axis_z_source,
            axis_size_source: ui_settings.axis_size_source,
            graph_normalization_mode: ui_settings.graph_normalization_mode,
            graph_framing_mode: ui_settings.graph_framing_mode,
            graph_view_state: GraphViewState {
                center: ui_settings.graph_fixed_center,
                extent: ui_settings.graph_fixed_extent,
            },
            graph_fixed_view: GraphViewState {
                center: ui_settings.graph_fixed_center,
                extent: ui_settings.graph_fixed_extent,
            },
            modifiers: ModifiersState::empty(),
            show_shortcuts: false,
            shader_hot_reload: ShaderHotReload {
                path: shader_path(),
                last_seen_modified: None,
            },
            shader_overlay: ShaderOverlayState {
                color: [0.18, 0.82, 0.42],
                intensity: 0.0,
            },
        }
    }

    fn refresh_window_title(&self) {
        if let Some(window) = &self.window {
            let tuning_suffix = self.analysis_controller.as_ref().map(|controller| {
                let settings = controller.snapshot();
                format!(
                    " | Tune {} = {} | Step {}",
                    self.tuning_focus.label(),
                    tuning_focus_value(self.tuning_focus, &settings),
                    tuning_step_value(self.tuning_focus, self.modifiers),
                )
            }).unwrap_or_default();
            let graph_suffix = format!(
                " | Graph {} {} | X {} Y {} Z {} S {}",
                self.graph_normalization_mode.label(),
                self.graph_framing_mode.label(),
                self.axis_x_source.label(),
                self.axis_y_source.label(),
                self.axis_z_source.label(),
                self.axis_size_source.label(),
            );
            let transport_suffix = if self.transport_controller.as_ref().is_some_and(|controller| controller.is_paused()) {
                " | Paused"
            } else {
                ""
            };
            let title = if let Some(error) = &self.audio_loading_error {
                format!(
                    "Vizzy Diagnostics | Audio Error | Beat {} | {} | {}{}{}{}",
                    self.beat_response_mode.label(),
                    source_label(&self.audio_source),
                    error,
                    tuning_suffix,
                    transport_suffix,
                    graph_suffix,
                )
            } else if self.show_shortcuts {
                format!(
                    "Vizzy Shortcuts | H help | Space play/pause | Tab target | [ ] tune | X/Y/Z/S axis | Arrows map | N norm | F frame | PgUp/PgDn zoom | IJKL/UO pan | Shift coarse | Ctrl fine | 0 reset | 1 beat mode{}{}{}",
                    tuning_suffix,
                    transport_suffix,
                    graph_suffix,
                )
            } else if self.audio_analyzer.is_none() {
                format!(
                    "Vizzy Diagnostics | Loading {:.0}% | Beat {} | {} | {}{}{}{}",
                    self.audio_loading_progress * 100.0,
                    self.beat_response_mode.label(),
                    self.audio_loading_stage,
                    source_label(&self.audio_source),
                    tuning_suffix,
                    transport_suffix,
                    graph_suffix,
                )
            } else {
                format!(
                    "Vizzy Diagnostics | Beat {} | {} | SB {:.2} B {:.2} M {:.2} T {:.2} Pr {:.2} Air {:.2} L {:.2} P {:.2} Bt {:.2} C {:.2}{}{}{}",
                    self.beat_response_mode.label(),
                    source_label(&self.audio_source),
                    self.current_sub_bass,
                    self.current_bass,
                    self.current_mid,
                    self.current_treble,
                    self.current_presence,
                    self.current_centroid,
                    tuning_suffix,
                    transport_suffix,
                    self.current_loudness,
                    self.current_peak,
                    self.current_beat,
                    self.current_centroid,
                    graph_suffix,
                )
            };

            window.set_title(&title);
            self.last_title_refresh.set(Some(Instant::now()));
        }
    }

    fn refresh_window_title_if_due(&self, interval: std::time::Duration) {
        if self
            .last_title_refresh
            .get()
            .is_none_or(|last_refresh| last_refresh.elapsed() >= interval)
        {
            self.refresh_window_title();
        }
    }

    fn signal_status(&mut self, color: [f32; 3]) {
        self.shader_overlay.color = color;
        self.shader_overlay.intensity = 1.0;
    }

    fn adjust_tuning(&mut self, delta: f32) {
        let Some(controller) = &self.analysis_controller else {
            return;
        };

        let focus = self.tuning_focus;
        let step = tuning_step_size(self.tuning_focus, self.modifiers) * delta;
        controller.update(|settings| match focus {
            TuningFocus::RmsThreshold => {
                settings.noise_gate_rms_threshold = (settings.noise_gate_rms_threshold + step).clamp(0.0, 0.30);
            }
            TuningFocus::PeakThreshold => {
                settings.noise_gate_peak_threshold = (settings.noise_gate_peak_threshold + step).clamp(0.0, 1.0);
            }
            TuningFocus::SubBassMax => {
                settings.sub_bass_max_frequency_hz = (settings.sub_bass_max_frequency_hz + step)
                    .clamp(audio::MIN_ANALYSIS_FREQUENCY_HZ + 10.0, settings.bass_max_frequency_hz - 20.0);
            }
            TuningFocus::BassMax => {
                settings.bass_max_frequency_hz = (settings.bass_max_frequency_hz + step)
                    .clamp(settings.sub_bass_max_frequency_hz + 20.0, settings.mid_max_frequency_hz - 100.0);
            }
            TuningFocus::MidMax => {
                settings.mid_max_frequency_hz = (settings.mid_max_frequency_hz + step)
                    .clamp(settings.bass_max_frequency_hz + 100.0, self.analysis_nyquist_hz - 500.0);
            }
            TuningFocus::PresenceMin => {
                settings.presence_min_frequency_hz = (settings.presence_min_frequency_hz + step)
                    .clamp(settings.mid_max_frequency_hz, settings.presence_max_frequency_hz - 100.0);
            }
            TuningFocus::PresenceMax => {
                settings.presence_max_frequency_hz = (settings.presence_max_frequency_hz + step)
                    .clamp(settings.presence_min_frequency_hz + 100.0, self.analysis_nyquist_hz - 100.0);
            }
            TuningFocus::AirMin => {
                settings.air_min_frequency_hz = (settings.air_min_frequency_hz + step)
                    .clamp(settings.presence_max_frequency_hz, self.analysis_nyquist_hz - 50.0);
            }
        });
        self.signal_status([0.94, 0.74, 0.18]);
        self.refresh_window_title();
    }

    fn toggle_play_pause(&mut self) {
        if let Some(controller) = &self.transport_controller {
            let paused = controller.toggle_paused();
            self.signal_status(if paused { [0.24, 0.72, 1.0] } else { [0.18, 0.82, 0.42] });
        } else {
            self.signal_status([0.96, 0.52, 0.18]);
        }
        self.refresh_window_title();
    }

    fn save_axis_settings(&self) {
        save_ui_settings(&UiSettings {
            axis_x_source: self.axis_x_source,
            axis_y_source: self.axis_y_source,
            axis_z_source: self.axis_z_source,
            axis_size_source: self.axis_size_source,
            graph_normalization_mode: self.graph_normalization_mode,
            graph_framing_mode: self.graph_framing_mode,
            graph_fixed_center: self.graph_fixed_view.center,
            graph_fixed_extent: self.graph_fixed_view.extent,
        });
    }

    fn cycle_axis_mapping(&mut self, next: bool) {
        let target = match self.axis_focus {
            AxisSelectionFocus::X => &mut self.axis_x_source,
            AxisSelectionFocus::Y => &mut self.axis_y_source,
            AxisSelectionFocus::Z => &mut self.axis_z_source,
            AxisSelectionFocus::Size => &mut self.axis_size_source,
        };
        *target = if next { target.next() } else { target.previous() };
        self.save_axis_settings();
        self.signal_status([0.38, 0.76, 1.0]);
        self.refresh_window_title();
    }

    fn set_axis_focus(&mut self, focus: AxisSelectionFocus) {
        self.axis_focus = focus;
        self.signal_status([0.38, 0.76, 1.0]);
        self.refresh_window_title();
    }

    fn toggle_graph_normalization(&mut self) {
        self.graph_normalization_mode = self.graph_normalization_mode.toggle();
        self.save_axis_settings();
        self.signal_status([0.76, 0.34, 0.98]);
        self.refresh_window_title();
    }

    fn toggle_graph_framing(&mut self) {
        self.graph_framing_mode = self.graph_framing_mode.toggle();
        if self.graph_framing_mode == GraphFramingMode::Fixed {
            self.graph_fixed_view = self.graph_view_state;
        }
        self.save_axis_settings();
        self.signal_status([0.34, 0.86, 0.98]);
        self.refresh_window_title();
    }

    fn adjust_fixed_graph_zoom(&mut self, delta: f32) {
        if self.graph_framing_mode != GraphFramingMode::Fixed {
            return;
        }

        let multiplier = graph_view_control_multiplier(self.modifiers);
        let factor: f32 = if delta > 0.0 { 0.88 } else { 1.0 / 0.88 };
        self.graph_fixed_view.extent = (self.graph_fixed_view.extent * factor.powf(multiplier))
            .clamp(0.05, 1.5);
        self.save_axis_settings();
        self.signal_status([0.34, 0.86, 0.98]);
        self.refresh_window_title();
    }

    fn pan_fixed_graph_view(&mut self, delta: [f32; 3]) {
        if self.graph_framing_mode != GraphFramingMode::Fixed {
            return;
        }

        let step = self.graph_fixed_view.extent * 0.18 * graph_view_control_multiplier(self.modifiers);
        for (axis, axis_delta) in delta.into_iter().enumerate() {
            self.graph_fixed_view.center[axis] = (self.graph_fixed_view.center[axis] + axis_delta * step)
                .clamp(0.0, 1.0);
        }
        self.save_axis_settings();
        self.signal_status([0.34, 0.86, 0.98]);
        self.refresh_window_title();
    }

    fn begin_audio_loading(&mut self) {
        self.audio_loading_progress = 0.0;
        self.audio_loading_stage = "Preparing audio".to_string();
        self.audio_loading_error = None;
        self.audio_loader = Some(audio::spawn_audio_preload(self.audio_source.clone()));
    }

    fn poll_audio_loading(&mut self) {
        let Some(receiver) = self.audio_loader.as_ref() else {
            return;
        };

        let mut clear_receiver = false;
        loop {
            match receiver.try_recv() {
                Ok(audio::AudioLoadUpdate::Progress { progress, stage }) => {
                    self.audio_loading_progress = progress;
                    self.audio_loading_stage = stage;
                }
                Ok(audio::AudioLoadUpdate::Ready(prepared)) => {
                    match audio::AudioAnalyzer::try_from_prepared(prepared) {
                        Ok(analyzer) => {
                            self.analysis_nyquist_hz = analyzer.sample_rate as f32 * 0.5;
                            self.analysis_controller = Some(analyzer.analysis_controller.clone());
                            self.transport_controller = analyzer.transport_controller.clone();
                            self.audio_analyzer = Some(analyzer);
                            self.audio_loading_progress = 1.0;
                            self.audio_loading_stage = "Audio ready".to_string();
                            self.audio_loading_error = None;
                            self.signal_status([0.18, 0.82, 0.42]);
                        }
                        Err(error) => {
                            self.audio_loading_error = Some(error);
                            self.signal_status([0.96, 0.22, 0.18]);
                        }
                    }
                    clear_receiver = true;
                    break;
                }
                Ok(audio::AudioLoadUpdate::Error(error)) => {
                    self.audio_loading_error = Some(error);
                    self.signal_status([0.96, 0.22, 0.18]);
                    clear_receiver = true;
                    break;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    if self.audio_analyzer.is_none() && self.audio_loading_error.is_none() {
                        self.audio_loading_error = Some("Audio loading worker disconnected".to_string());
                        self.signal_status([0.96, 0.22, 0.18]);
                    }
                    clear_receiver = true;
                    break;
                }
            }
        }

        if clear_receiver {
            self.audio_loader = None;
        }
    }

    fn reload_shader_if_needed(&mut self) {
        let (device, config, bind_group_layout) = match (
            self.device.as_ref(),
            self.config.as_ref(),
            self.bind_group_layout.as_ref(),
        ) {
            (Some(device), Some(config), Some(bind_group_layout)) => (device, config, bind_group_layout),
            _ => return,
        };

        let metadata = match fs::metadata(&self.shader_hot_reload.path) {
            Ok(metadata) => metadata,
            Err(error) => {
                eprintln!(
                    "Shader hot reload skipped, could not read {}: {}",
                    self.shader_hot_reload.path.display(),
                    error
                );
                return;
            }
        };

        let modified = match metadata.modified() {
            Ok(modified) => modified,
            Err(error) => {
                eprintln!(
                    "Shader hot reload skipped, could not read modified time for {}: {}",
                    self.shader_hot_reload.path.display(),
                    error
                );
                return;
            }
        };

        if self.shader_hot_reload.last_seen_modified == Some(modified) {
            return;
        }

        match read_shader_source(&self.shader_hot_reload.path)
            .and_then(|(source, modified)| {
                create_render_pipeline(device, config, bind_group_layout, &source)
                    .map(|pipeline| (pipeline, modified))
            }) {
            Ok((render_pipeline, modified)) => {
                self.render_pipeline = Some(render_pipeline);
                self.shader_hot_reload.last_seen_modified = Some(modified);
                self.shader_overlay.color = [0.18, 0.82, 0.42];
                self.shader_overlay.intensity = 1.0;
                println!("Reloaded shader from {}", self.shader_hot_reload.path.display());
            }
            Err(error) => {
                self.shader_hot_reload.last_seen_modified = Some(modified);
                self.shader_overlay.color = [0.96, 0.22, 0.18];
                self.shader_overlay.intensity = 1.0;
                eprintln!("{}", error);
            }
        }
    }
}

fn tuning_focus_value(focus: TuningFocus, settings: &audio::AnalysisSettings) -> String {
    match focus {
        TuningFocus::RmsThreshold => format!("{:.3}", settings.noise_gate_rms_threshold),
        TuningFocus::PeakThreshold => format!("{:.3}", settings.noise_gate_peak_threshold),
        TuningFocus::SubBassMax => format!("{:.0}Hz", settings.sub_bass_max_frequency_hz),
        TuningFocus::BassMax => format!("{:.0}Hz", settings.bass_max_frequency_hz),
        TuningFocus::MidMax => format!("{:.0}Hz", settings.mid_max_frequency_hz),
        TuningFocus::PresenceMin => format!("{:.0}Hz", settings.presence_min_frequency_hz),
        TuningFocus::PresenceMax => format!("{:.0}Hz", settings.presence_max_frequency_hz),
        TuningFocus::AirMin => format!("{:.0}Hz", settings.air_min_frequency_hz),
    }
}

fn tuning_focus_numeric_value(focus: TuningFocus, settings: &audio::AnalysisSettings) -> f32 {
    match focus {
        TuningFocus::RmsThreshold => settings.noise_gate_rms_threshold,
        TuningFocus::PeakThreshold => settings.noise_gate_peak_threshold,
        TuningFocus::SubBassMax => settings.sub_bass_max_frequency_hz,
        TuningFocus::BassMax => settings.bass_max_frequency_hz,
        TuningFocus::MidMax => settings.mid_max_frequency_hz,
        TuningFocus::PresenceMin => settings.presence_min_frequency_hz,
        TuningFocus::PresenceMax => settings.presence_max_frequency_hz,
        TuningFocus::AirMin => settings.air_min_frequency_hz,
    }
}

fn tuning_step_size(focus: TuningFocus, modifiers: ModifiersState) -> f32 {
    let multiplier = if modifiers.shift_key() {
        5.0
    } else if modifiers.control_key() {
        0.25
    } else {
        1.0
    };
    focus.base_step() * multiplier
}

fn tuning_step_value(focus: TuningFocus, modifiers: ModifiersState) -> String {
    let step = tuning_step_size(focus, modifiers);
    if focus.is_frequency() {
        format!("{:.0}Hz", step)
    } else {
        format!("{:.3}", step)
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none() {
            let window_attr = winit::window::WindowAttributes::default()
                .with_title("Vizzy Audio Visualizer")
                .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 720.0));
            
            let window = Arc::new(event_loop.create_window(window_attr).unwrap());
            self.window = Some(window.clone());

            // Initialize WGPU Context
            let instance = wgpu::Instance::default();
            let surface = instance.create_surface(window.clone()).unwrap();

            let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })).expect("Failed to find wgpu adapter");

            let (device, queue) = block_on(adapter.request_device(
                &wgpu::DeviceDescriptor {
                    label: None,
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                    memory_hints: Default::default(),
                    ..Default::default()
                },
            )).expect("Failed to create device");

            let size = window.inner_size();
            let surface_caps = surface.get_capabilities(&adapter);
            let surface_format = surface_caps.formats.iter()
                .find(|f| f.is_srgb())
                .copied()
                .unwrap_or(surface_caps.formats[0]);

            // Prefer Fifo (VSync) for smooth presentation, fall back to first available
            let present_mode = if surface_caps.present_modes.contains(&wgpu::PresentMode::Fifo) {
                wgpu::PresentMode::Fifo
            } else {
                surface_caps.present_modes[0]
            };

            let config = wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format: surface_format,
                width: size.width.max(1),
                height: size.height.max(1),
                present_mode,
                alpha_mode: surface_caps.alpha_modes[0],
                view_formats: vec![],
                desired_maximum_frame_latency: 2,
            };

            surface.configure(&device, &config);

            // Uniforms
            let uniforms = StandardUniforms {
                resolution: [config.width as f32, config.height as f32],
                _padding: [0.0; 2],
                time: 0.0,
                audio_bass: 0.0,
                audio_mid: 0.0,
                audio_treble: 0.0,
                audio_loudness: 0.0,
                audio_peak: 0.0,
                audio_beat: 0.0,
                audio_centroid: 0.0,
                audio_sub_bass: 0.0,
                audio_presence: 0.0,
                audio_air: 0.0,
                analysis_nyquist_hz: self.analysis_nyquist_hz,
                status_color: [0.0; 4],
                loading_progress: 0.0,
                loading_active: 1.0,
                show_help: 0.0,
                active_tuning_focus: self.tuning_focus.index() as f32,
                active_tuning_value: audio::NOISE_GATE_RMS_THRESHOLD,
                active_tuning_step: tuning_step_size(self.tuning_focus, self.modifiers),
                active_tuning_is_hz: 0.0,
                active_axis_focus: self.axis_focus.index() as f32,
                axis_x_source: self.axis_x_source.index() as f32,
                axis_y_source: self.axis_y_source.index() as f32,
                axis_z_source: self.axis_z_source.index() as f32,
                axis_size_source: self.axis_size_source.index() as f32,
                split_sub_bass_max_hz: audio::SUB_BASS_MAX_FREQUENCY_HZ,
                split_bass_max_hz: audio::BASS_MAX_FREQUENCY_HZ,
                split_mid_max_hz: audio::MID_MAX_FREQUENCY_HZ,
                split_presence_min_hz: audio::PRESENCE_MIN_FREQUENCY_HZ,
                split_presence_max_hz: audio::PRESENCE_MAX_FREQUENCY_HZ,
                split_air_min_hz: audio::AIR_MIN_FREQUENCY_HZ,
                _graph_view_padding: [0.0; 2],
                graph_view_center: self.graph_view_state.center,
                graph_view_extent: self.graph_view_state.extent,
            };

            debug_assert_eq!(std::mem::size_of::<StandardUniforms>(), 176);

            let uniform_buffer = device.create_buffer_init(
                &wgpu::util::BufferInitDescriptor {
                    label: Some("Uniform Buffer"),
                    contents: bytemuck::cast_slice(&[uniforms]),
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                }
            );

            let log_bins_buffer = device.create_buffer_init(
                &wgpu::util::BufferInitDescriptor {
                    label: Some("Log Bins Buffer"),
                    contents: bytemuck::cast_slice(&[build_log_bins_uniforms(&self.current_log_bins)]),
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                }
            );

            let initial_graph_history = build_graph_history_uniforms(
                &self.feature_history,
                self.axis_x_source,
                self.axis_y_source,
                self.axis_z_source,
                self.axis_size_source,
                self.graph_normalization_mode,
            );
            self.graph_view_state = build_graph_view_state(&initial_graph_history.points);

            let graph_points_buffer = device.create_buffer_init(
                &wgpu::util::BufferInitDescriptor {
                    label: Some("Graph Points Buffer"),
                    contents: bytemuck::cast_slice(&[initial_graph_history]),
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                }
            );

            let bind_group_layout = create_bind_group_layout(&device);

            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Uniform Bind Group"),
                layout: &bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: uniform_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: log_bins_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: graph_points_buffer.as_entire_binding(),
                    }
                ],
            });

            let (shader_source, shader_modified) = read_shader_source(&self.shader_hot_reload.path)
                .unwrap_or_else(|error| panic!("{}", error));
            let render_pipeline = create_render_pipeline(
                &device,
                &config,
                &bind_group_layout,
                &shader_source,
            )
            .unwrap_or_else(|error| panic!("{}", error));

            self.surface = Some(surface);
            self.device = Some(device);
            self.queue = Some(queue);
            self.config = Some(config);
            
            println!("Starting audio preload...");
            self.begin_audio_loading();
            self.render_pipeline = Some(render_pipeline);
            self.uniform_buffer = Some(uniform_buffer);
            self.log_bins_buffer = Some(log_bins_buffer);
            self.graph_points_buffer = Some(graph_points_buffer);
            self.bind_group_layout = Some(bind_group_layout);
            self.bind_group = Some(bind_group);
            self.start_time = Some(Instant::now());
            self.shader_hot_reload.last_seen_modified = Some(shader_modified);
            self.refresh_window_title();
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let window = match &self.window {
            Some(window) if window.id() == id => Arc::clone(window),
            _ => return,
        };

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::ModifiersChanged(modifiers) => {
                self.modifiers = modifiers.state();
                self.refresh_window_title_if_due(std::time::Duration::from_millis(250));
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if event.state == ElementState::Pressed && !event.repeat {
                    if let PhysicalKey::Code(code) = event.physical_key {
                        match code {
                            KeyCode::Digit1 => {
                                self.beat_response_mode = self.beat_response_mode.next();
                                self.refresh_window_title();
                            }
                            KeyCode::KeyH => {
                                self.show_shortcuts = !self.show_shortcuts;
                                self.signal_status([0.30, 0.70, 1.00]);
                                self.refresh_window_title();
                            }
                            KeyCode::Space => self.toggle_play_pause(),
                            KeyCode::KeyX => self.set_axis_focus(AxisSelectionFocus::X),
                            KeyCode::KeyY => self.set_axis_focus(AxisSelectionFocus::Y),
                            KeyCode::KeyZ => self.set_axis_focus(AxisSelectionFocus::Z),
                            KeyCode::KeyS => self.set_axis_focus(AxisSelectionFocus::Size),
                            KeyCode::KeyN => self.toggle_graph_normalization(),
                            KeyCode::KeyF => self.toggle_graph_framing(),
                            KeyCode::PageUp => self.adjust_fixed_graph_zoom(1.0),
                            KeyCode::PageDown => self.adjust_fixed_graph_zoom(-1.0),
                            KeyCode::KeyJ => self.pan_fixed_graph_view([-1.0, 0.0, 0.0]),
                            KeyCode::KeyL => self.pan_fixed_graph_view([1.0, 0.0, 0.0]),
                            KeyCode::KeyI => self.pan_fixed_graph_view([0.0, 1.0, 0.0]),
                            KeyCode::KeyK => self.pan_fixed_graph_view([0.0, -1.0, 0.0]),
                            KeyCode::KeyU => self.pan_fixed_graph_view([0.0, 0.0, -1.0]),
                            KeyCode::KeyO => self.pan_fixed_graph_view([0.0, 0.0, 1.0]),
                            KeyCode::Tab => {
                                self.tuning_focus = self.tuning_focus.next();
                                self.signal_status([0.94, 0.74, 0.18]);
                                self.refresh_window_title();
                            }
                            KeyCode::ArrowLeft | KeyCode::ArrowDown => self.cycle_axis_mapping(false),
                            KeyCode::ArrowRight | KeyCode::ArrowUp => self.cycle_axis_mapping(true),
                            KeyCode::BracketLeft | KeyCode::Minus => self.adjust_tuning(-1.0),
                            KeyCode::BracketRight | KeyCode::Equal => self.adjust_tuning(1.0),
                            KeyCode::Digit0 => {
                                if let Some(controller) = &self.analysis_controller {
                                    controller.reset();
                                    self.signal_status([0.94, 0.74, 0.18]);
                                    self.refresh_window_title();
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            WindowEvent::Resized(physical_size) => {
                if physical_size.width > 0 && physical_size.height > 0 {
                    if let (Some(config), Some(surface), Some(device)) = 
                        (&mut self.config, &self.surface, &self.device) {
                        config.width = physical_size.width;
                        config.height = physical_size.height;
                        surface.configure(device, config);
                        window.request_redraw();
                    }
                }
            }
            WindowEvent::RedrawRequested => {
                self.poll_audio_loading();
                if self.audio_loader.is_some()
                    && self.audio_loading_error.is_none()
                    && self.audio_loading_stage.contains("Decoding")
                {
                    self.audio_loading_progress = (self.audio_loading_progress + 0.0025).min(0.74);
                }
                self.reload_shader_if_needed();

                let surface = self.surface.as_ref().unwrap();
                let device = self.device.as_ref().unwrap();
                let queue = self.queue.as_ref().unwrap();
                let config = self.config.as_ref().unwrap();
                let start_time = self.start_time.as_ref().unwrap();

                // Consume latest audio FFT if available
                let mut target_bass = self.current_bass;
                let mut target_mid = self.current_mid;
                let mut target_treble = self.current_treble;
                let mut target_sub_bass = self.current_sub_bass;
                let mut target_presence = self.current_presence;
                let mut target_air = self.current_air;
                let mut target_loudness = self.current_loudness;
                let mut target_peak = self.current_peak;
                let mut target_beat = 0.0;
                let mut target_centroid = self.current_centroid;
                let mut target_log_bins = self.current_log_bins;
                let mut received_audio_update = false;

                if let Some(analyzer) = &mut self.audio_analyzer {
                    let mut sub_bass_energy = 0.0;
                    let mut b_energy = 0.0;
                    let mut m_energy = 0.0;
                    let mut t_energy = 0.0;
                    let mut presence_energy = 0.0;
                    let mut air_energy = 0.0;
                    let mut loudness = 0.0;
                    let mut peak = 0.0;
                    let mut centroid = 0.0;
                    let mut log_bins_sum = [0.0; audio::LOG_BAND_COUNT];
                    let mut count = 0;
                    
                    use ringbuf::traits::Consumer;
                    while let Some(frame) = analyzer.rx.try_pop() {
                        sub_bass_energy += frame.sub_bass_energy;
                        b_energy += frame.bass_energy;
                        m_energy += frame.mid_energy;
                        t_energy += frame.treble_energy;
                        presence_energy += frame.presence_energy;
                        air_energy += frame.air_energy;
                        loudness += frame.rms;
                        peak += frame.peak_amplitude;
                        centroid += frame.spectral_centroid;
                        for (sum, value) in log_bins_sum.iter_mut().zip(frame.log_bins.iter()) {
                            *sum += *value;
                        }
                        count += 1;
                    }
                    
                    if count > 0 {
                        received_audio_update = true;
                        let c = count as f32;
                        let average_loudness = loudness / c;
                        let average_peak = peak / c;
                        let average_centroid = centroid / c;

                        target_sub_bass = compress_audio(sub_bass_energy / c, 210.0);
                        target_bass = compress_audio(b_energy / c, 180.0);
                        target_mid = compress_audio(m_energy / c, 220.0);
                        target_treble = compress_audio(t_energy / c, 360.0);
                        target_presence = compress_audio(presence_energy / c, 320.0);
                        target_air = compress_audio(air_energy / c, 340.0);
                        target_loudness = compress_audio(average_loudness, 8.5);
                        target_peak = compress_audio(average_peak, 2.4);
                        target_centroid = average_centroid;
                        target_log_bins = array::from_fn(|index| compress_audio(log_bins_sum[index] / c, 280.0));
                        target_beat = detect_beat(
                            average_loudness,
                            average_peak,
                            &self.loudness_history,
                            self.beat_response_mode,
                        );

                        if self.loudness_history.len() == 48 {
                            self.loudness_history.pop_front();
                        }
                        self.loudness_history.push_back(average_loudness);
                    }
                }

                envelope_follow(&mut self.current_sub_bass, target_sub_bass, 0.16, 0.050);
                envelope_follow(&mut self.current_bass, target_bass, 0.14, 0.045);
                envelope_follow(&mut self.current_mid, target_mid, 0.12, 0.050);
                envelope_follow(&mut self.current_treble, target_treble, 0.16, 0.070);
                envelope_follow(&mut self.current_presence, target_presence, 0.16, 0.080);
                envelope_follow(&mut self.current_air, target_air, 0.18, 0.090);
                envelope_follow(&mut self.current_loudness, target_loudness, 0.12, 0.040);
                envelope_follow(&mut self.current_peak, target_peak, 0.18, 0.090);
                let (beat_attack, beat_release) = self.beat_response_mode.envelope();
                envelope_follow(&mut self.current_beat, target_beat, beat_attack, beat_release);
                envelope_follow(&mut self.current_centroid, target_centroid, 0.10, 0.050);
                for (current_bin, target_bin) in self.current_log_bins.iter_mut().zip(target_log_bins.iter()) {
                    envelope_follow(current_bin, *target_bin, 0.18, 0.080);
                }

                // Clamp audio values to sane range for shader stability
                let sub_bass_clamped = self.current_sub_bass.clamp(0.0, 1.0);
                let bass_clamped = self.current_bass.clamp(0.0, 1.0);
                let mid_clamped = self.current_mid.clamp(0.0, 1.0);
                let treble_clamped = self.current_treble.clamp(0.0, 1.0);
                let presence_clamped = self.current_presence.clamp(0.0, 1.0);
                let air_clamped = self.current_air.clamp(0.0, 1.0);
                let loudness_clamped = self.current_loudness.clamp(0.0, 1.0);
                let peak_clamped = self.current_peak.clamp(0.0, 1.0);
                let beat_clamped = self.current_beat.clamp(0.0, 1.0);
                let centroid_clamped = self.current_centroid.clamp(0.0, 1.0);
                let log_frequency_clamped = normalized_log_frequency(centroid_clamped, self.analysis_nyquist_hz);
                let bin_index_clamped = normalized_peak_log_bin(&self.current_log_bins);
                let graph_sample = build_feature_snapshot(
                    sub_bass_clamped,
                    bass_clamped,
                    mid_clamped,
                    treble_clamped,
                    presence_clamped,
                    air_clamped,
                    loudness_clamped,
                    peak_clamped,
                    beat_clamped,
                    centroid_clamped,
                    log_frequency_clamped,
                    bin_index_clamped,
                );
                if received_audio_update {
                    if should_capture_graph_sample(
                        &graph_sample,
                        self.axis_x_source,
                        self.axis_y_source,
                        self.axis_z_source,
                        self.axis_size_source,
                    ) {
                        if self.feature_history.len() == GRAPH_HISTORY_LENGTH {
                            self.feature_history.pop_front();
                        }
                        self.feature_history.push_back(graph_sample);
                    }
                }
                self.shader_overlay.intensity = (self.shader_overlay.intensity - 0.018).max(0.0);
                self.refresh_window_title_if_due(std::time::Duration::from_millis(250));

                let elapsed = start_time.elapsed().as_secs_f32();
                let settings = self.analysis_controller.as_ref().map(|controller| controller.snapshot()).unwrap_or_default();
                let active_tuning_step = tuning_step_size(self.tuning_focus, self.modifiers);
                let graph_history_uniforms = build_graph_history_uniforms(
                    &self.feature_history,
                    self.axis_x_source,
                    self.axis_y_source,
                    self.axis_z_source,
                    self.axis_size_source,
                    self.graph_normalization_mode,
                );
                let graph_view = build_graph_view_state(&graph_history_uniforms.points);
                let active_graph_view = if self.graph_framing_mode == GraphFramingMode::Locality {
                    smooth_graph_view_towards(&mut self.graph_view_state, graph_view);
                    self.graph_view_state
                } else {
                    smooth_graph_view_towards(&mut self.graph_view_state, self.graph_fixed_view);
                    self.graph_view_state
                };

                let uniforms = StandardUniforms {
                    resolution: [config.width as f32, config.height as f32],
                    _padding: [0.0; 2],
                    time: elapsed,
                    audio_bass: bass_clamped,
                    audio_mid: mid_clamped,
                    audio_treble: treble_clamped,
                    audio_loudness: loudness_clamped,
                    audio_peak: peak_clamped,
                    audio_beat: beat_clamped,
                    audio_centroid: centroid_clamped,
                    audio_sub_bass: sub_bass_clamped,
                    audio_presence: presence_clamped,
                    audio_air: air_clamped,
                    analysis_nyquist_hz: self.analysis_nyquist_hz,
                    status_color: [
                        self.shader_overlay.color[0],
                        self.shader_overlay.color[1],
                        self.shader_overlay.color[2],
                        self.shader_overlay.intensity,
                    ],
                    loading_progress: self.audio_loading_progress,
                    loading_active: if self.audio_analyzer.is_some() || self.audio_loading_error.is_some() {
                        0.0
                    } else {
                        1.0
                    },
                    show_help: if self.show_shortcuts { 1.0 } else { 0.0 },
                    active_tuning_focus: self.tuning_focus.index() as f32,
                    active_tuning_value: tuning_focus_numeric_value(self.tuning_focus, &settings),
                    active_tuning_step: active_tuning_step,
                    active_tuning_is_hz: if self.tuning_focus.is_frequency() { 1.0 } else { 0.0 },
                    active_axis_focus: self.axis_focus.index() as f32,
                    axis_x_source: self.axis_x_source.index() as f32,
                    axis_y_source: self.axis_y_source.index() as f32,
                    axis_z_source: self.axis_z_source.index() as f32,
                    axis_size_source: self.axis_size_source.index() as f32,
                    split_sub_bass_max_hz: settings.sub_bass_max_frequency_hz,
                    split_bass_max_hz: settings.bass_max_frequency_hz,
                    split_mid_max_hz: settings.mid_max_frequency_hz,
                    split_presence_min_hz: settings.presence_min_frequency_hz,
                    split_presence_max_hz: settings.presence_max_frequency_hz,
                    split_air_min_hz: settings.air_min_frequency_hz,
                    _graph_view_padding: [0.0; 2],
                    graph_view_center: active_graph_view.center,
                    graph_view_extent: active_graph_view.extent,
                };

                queue.write_buffer(
                    self.uniform_buffer.as_ref().unwrap(),
                    0,
                    bytemuck::cast_slice(&[uniforms])
                );
                queue.write_buffer(
                    self.log_bins_buffer.as_ref().unwrap(),
                    0,
                    bytemuck::cast_slice(&[build_log_bins_uniforms(&self.current_log_bins)])
                );
                queue.write_buffer(
                    self.graph_points_buffer.as_ref().unwrap(),
                    0,
                    bytemuck::cast_slice(&[graph_history_uniforms])
                );

                let output_frame = match surface.get_current_texture() {
                    Ok(frame) => frame,
                    Err(wgpu::SurfaceError::Outdated | wgpu::SurfaceError::Lost) => {
                        if let (Some(cfg), Some(surf), Some(dev)) =
                            (&self.config, &self.surface, &self.device) {
                            surf.configure(dev, cfg);
                        }
                        window.request_redraw();
                        return;
                    }
                    Err(e) => {
                        eprintln!("Dropped frame: {:?}", e);
                        window.request_redraw();
                        return;
                    }
                };
                
                let view = output_frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
                let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });

                {
                    let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: None,
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &view,
                            resolve_target: None,
                            depth_slice: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }),
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                    });

                    render_pass.set_pipeline(self.render_pipeline.as_ref().unwrap());
                    render_pass.set_bind_group(0, self.bind_group.as_ref().unwrap(), &[]);
                    render_pass.draw(0..3, 0..1);
                }

                queue.submit(std::iter::once(encoder.finish()));
                output_frame.present();
            }
            _ => (),
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}

/// Envelope follower: attack and release rates in the [0, 1] range.
fn envelope_follow(curr: &mut f32, target: f32, attack: f32, release: f32) {
    let rate = if target > *curr { attack } else { release };
    *curr += (target - *curr) * rate;
}

fn main() {
    env_logger::init();
    let event_loop = EventLoop::new().unwrap();
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut app = App::new();
    let _ = event_loop.run_app(&mut app);
}
