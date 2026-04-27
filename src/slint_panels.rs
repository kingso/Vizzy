use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, PlatformError, PointerEventButton, WindowAdapter, WindowEvent};
use slint::{Color, ComponentHandle, LogicalPosition, ModelRc, PhysicalSize, SharedString, VecModel};
use wgpu::util::DeviceExt;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent as WinitWindowEvent};

const OVERLAY_BASE_WIDTH: u32 = 640;
const OVERLAY_BASE_HEIGHT: u32 = 720;
const OVERLAY_MIN_WIDTH: u32 = 420;
const OVERLAY_MIN_HEIGHT: u32 = 560;
const OVERLAY_TEXTURE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

pub const CHART_SAMPLE_COUNT: usize = 32;

const OVERLAY_SHADER: &str = r#"
struct VertexIn {
    @location(0) position: vec2<f32>,
    @location(1) tex_coords: vec2<f32>,
};

struct VertexOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) tex_coords: vec2<f32>,
};

@vertex
fn vs_main(vertex: VertexIn) -> VertexOut {
    var out: VertexOut;
    out.clip_position = vec4(vertex.position, 0.0, 1.0);
    out.tex_coords = vertex.tex_coords;
    return out;
}

@group(0) @binding(0)
var overlay_texture: texture_2d<f32>;

@group(0) @binding(1)
var overlay_sampler: sampler;

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    return textureSample(overlay_texture, overlay_sampler, in.tex_coords);
}
"#;

thread_local! {
    static WINDOW_SLOT: RefCell<Option<Rc<MinimalSoftwareWindow>>> = RefCell::new(None);
}

static PLATFORM_INSTALLED: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Debug)]
pub struct DiagnosticsChartSnapshot {
    pub tag: &'static str,
    pub title: String,
    pub value_label: String,
    pub accent: [u8; 3],
    pub samples: Vec<f32>,
}

#[derive(Clone, Debug)]
pub struct DiagnosticsTuningRowSnapshot {
    pub label: &'static str,
    pub value_label: String,
    pub step_label: String,
    pub is_active: bool,
}

#[derive(Clone, Debug)]
pub struct DiagnosticsArcSnapshot {
    pub label: &'static str,
    pub value_label: String,
    pub accent: [u8; 3],
    pub value: f32,
    pub peak: f32,
}

#[derive(Clone, Debug)]
pub struct DiagnosticsSpectrumBinSnapshot {
    pub value: f32,
    pub accent: [u8; 3],
}

#[derive(Clone, Debug)]
pub struct DiagnosticsSnapshot {
    pub headline: String,
    pub subline: String,
    pub graph_line: String,
    pub toggle_label: String,
    pub help_label: String,
    pub show_help: bool,
    pub charts: [DiagnosticsChartSnapshot; 4],
    pub left_arcs: Vec<DiagnosticsArcSnapshot>,
    pub right_arcs: Vec<DiagnosticsArcSnapshot>,
    pub spectrum_bins: Vec<DiagnosticsSpectrumBinSnapshot>,
    pub spectrum_markers: Vec<f32>,
    pub history_label: String,
    pub tuning_rows: Vec<DiagnosticsTuningRowSnapshot>,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum SlintDiagnosticsAction {
    ToggleViewMode,
    ToggleHelp,
    FocusTuning(usize),
    NudgeTuning { index: usize, direction: i32 },
    NudgeHistory(i32),
    ShowAllHistory,
    ResetTuning,
}

#[derive(Default)]
pub struct SlintDiagnosticsEventResponse {
    pub consumed: bool,
    pub repaint: bool,
    pub actions: Vec<SlintDiagnosticsAction>,
}

slint::slint! {
    export struct UiChartPoint {
        value: float,
    }

    export struct UiChartCard {
        tag: string,
        title: string,
        value_label: string,
        accent: color,
        samples: [UiChartPoint],
    }

    export struct UiTuningRow {
        label: string,
        value_label: string,
        step_label: string,
        is_active: bool,
    }

    export struct UiArcGauge {
        label: string,
        value_label: string,
        accent: color,
        value: float,
        peak: float,
    }

    export struct UiSpectrumBin {
        value: float,
        accent: color,
    }

    export struct UiSpectrumMarker {
        position: float,
    }

    component SmallActionButton inherits Rectangle {
        in property <string> label;
        in property <color> fill;
        callback activated;

        border-radius: 11px;
        background: touch.pressed ? #173447 : fill;

        Text {
            x: 0px;
            y: 0px;
            width: parent.width;
            height: parent.height;
            text: root.label;
            color: #f2f0e9;
            font-size: 12px;
            horizontal-alignment: center;
            vertical-alignment: center;
        }

        touch := TouchArea {
            clicked => {
                root.activated();
            }
        }
    }

    component ChartBars inherits Rectangle {
        in property <[UiChartPoint]> samples;
        in property <color> accent;

        clip: true;

        for sample[index] in root.samples : Rectangle {
            x: 4px + index * ((parent.width - 8px) / 32);
            width: ((parent.width - 8px) / 32) - 1px;
            height: 8px + sample.value * (parent.height - 14px);
            y: parent.height - self.height - 4px;
            background: root.accent;
            opacity: 0.22 + sample.value * 0.78;
            border-radius: 1px;
        }
    }

    component ChartTile inherits Rectangle {
        in property <UiChartCard> data;

        background: rgb(11, 25, 33);
        border-radius: 16px;
        border-width: 1px;
        border-color: #142733;

        Rectangle {
            x: 10px;
            y: 10px;
            width: 24px;
            height: 18px;
            background: data.accent;
            border-radius: 9px;

            Text {
                x: 0px;
                y: 0px;
                width: parent.width;
                height: parent.height;
                text: data.tag;
                color: #081116;
                font-size: 10px;
                horizontal-alignment: center;
                vertical-alignment: center;
            }
        }

        Text {
            x: parent.width - 60px;
            y: 12px;
            width: 50px;
            text: data.value_label;
            color: data.accent;
            font-size: 10px;
            horizontal-alignment: right;
        }

        Text {
            x: 40px;
            y: 12px;
            width: parent.width - 110px;
            text: data.title;
            color: #f3eadb;
            font-size: 11px;
        }

        Rectangle {
            x: 10px;
            y: 32px;
            width: parent.width - 20px;
            height: parent.height - 42px;
            background: #081116;
            border-radius: 10px;
            border-width: 1px;
            border-color: #10202b;

            ChartBars {
                x: 4px;
                y: 0px;
                width: parent.width - 8px;
                height: parent.height;
                samples: data.samples;
                accent: data.accent;
            }
        }
    }

    component ArcGaugeMiniChip inherits Rectangle {
        in property <UiArcGauge> gauge;

        background: #0c1620;
        border-radius: 10px;
        border-width: 1px;
        border-color: #173447;

        Text {
            x: 6px;
            y: 3px;
            text: gauge.label;
            color: #f3eadb;
            font-size: 8px;
        }

        Rectangle {
            x: 6px;
            y: parent.height - 8px;
            width: parent.width - 12px;
            height: 4px;
            background: #173447;
            border-radius: 2px;
        }

        Rectangle {
            x: 6px;
            y: parent.height - 8px;
            width: gauge.value * (parent.width - 12px);
            height: 4px;
            background: gauge.accent;
            border-radius: 2px;
        }

        Rectangle {
            x: 6px + gauge.peak * (parent.width - 14px);
            y: parent.height - 10px;
            width: 2px;
            height: 8px;
            background: #f3eadb;
            opacity: 0.82;
            border-radius: 1px;
        }
    }

    component SpectrumTile inherits Rectangle {
        in property <[UiSpectrumBin]> bins;
        in property <[UiSpectrumMarker]> markers;

        background: #0a1720;
        border-radius: 18px;
        border-width: 1px;
        border-color: #142733;

        Text {
            x: 14px;
            y: 12px;
            text: "Spectrum";
            color: #f3eadb;
            font-size: 16px;
        }

        Text {
            x: 108px;
            y: 14px;
            text: "24 log bins with live split markers";
            color: #7e96a1;
            font-size: 10px;
        }

        Rectangle {
            x: 12px;
            y: 36px;
            width: parent.width - 24px;
            height: parent.height - 48px;
            background: #081116;
            border-radius: 12px;
            border-width: 1px;
            border-color: #10202b;

            for marker in root.markers : Rectangle {
                x: 10px + marker.position * (parent.width - 20px);
                y: 6px;
                width: 1px;
                height: parent.height - 12px;
                background: #d8d0bf;
                opacity: 0.2;
            }

            for bin[index] in root.bins : Rectangle {
                x: 10px + index * ((parent.width - 20px) / 24);
                width: ((parent.width - 20px) / 24) - 4px;
                height: 8px + bin.value * (parent.height - 14px);
                y: parent.height - self.height - 4px;
                background: bin.accent;
                opacity: 0.24 + bin.value * 0.76;
                border-radius: 2px;
            }
        }
    }

    component HelpRow inherits Rectangle {
        in property <string> key_label;
        in property <string> description;

        Text {
            x: 0px;
            y: 1px;
            width: 92px;
            text: root.key_label;
            color: #88d7cf;
            font-size: 11px;
        }

        Text {
            x: 96px;
            y: 1px;
            width: parent.width - 96px;
            text: root.description;
            color: #cad5d1;
            font-size: 11px;
        }
    }

    export component DiagnosticsSurface inherits Window {
        in property <string> headline;
        in property <string> subline;
        in property <string> graph_line;
        in property <string> toggle_label;
        in property <string> help_label;
        in property <bool> show_help;
        in property <UiChartCard> chart0;
        in property <UiChartCard> chart1;
        in property <UiChartCard> chart2;
        in property <UiChartCard> chart3;
        in property <[UiArcGauge]> left_arcs;
        in property <[UiArcGauge]> right_arcs;
        in property <[UiSpectrumBin]> spectrum_bins;
        in property <[UiSpectrumMarker]> spectrum_markers;
        in property <string> history_label;
        in property <[UiTuningRow]> tuning_rows;

        callback toggle_view();
    callback toggle_help();
        callback focus_tuning(index: int);
        callback nudge_tuning(index: int, direction: int);
        callback nudge_history(direction: int);
        callback show_all_history();
        callback reset_tuning();

        property <length> gutter: 16px;
        property <length> section_gap: 12px;
        property <length> inner_width: root.width - root.gutter * 2;
        property <length> tuning_height: 218px;
        property <length> bottom_gap: root.section_gap;
        property <length> tuning_y: root.height - root.tuning_height - root.bottom_gap;
        property <length> spectrum_y: 86px;
        property <length> spectrum_height: 240px;
        property <length> chart_y: root.spectrum_y + root.spectrum_height + root.section_gap;
        property <length> chart_width: (root.inner_width - root.section_gap) / 2;
        property <length> chart_height: (root.tuning_y - root.chart_y - root.section_gap - root.section_gap) / 2;

        background: #08131a;

        Rectangle {
            x: 0px;
            y: 0px;
            width: root.width;
            height: root.height;
            background: #08131a;
            border-radius: 24px;
            border-width: 1px;
            border-color: #122634;
        }

        Rectangle {
            x: root.width - 120px;
            y: 20px;
            width: 78px;
            height: 78px;
            border-radius: 39px;
            background: #11242f;
            opacity: 0.48;
        }

        Rectangle {
            x: 24px;
            y: root.height - 96px;
            width: 92px;
            height: 92px;
            border-radius: 46px;
            background: #10222c;
            opacity: 0.34;
        }

        Text {
            x: 18px;
            y: 16px;
            text: root.headline;
            color: #f3eadb;
            font-size: 24px;
        }

        Text {
            x: 18px;
            y: 44px;
            text: root.subline;
            color: #90a6ae;
            font-size: 11px;
        }

        Text {
            x: 18px;
            y: 60px;
            text: root.graph_line;
            color: #708893;
            font-size: 11px;
        }

        SmallActionButton {
            x: root.width - 180px;
            y: 18px;
            width: 62px;
            height: 28px;
            label: root.help_label;
            fill: #21495b;
            activated => {
                root.toggle_help();
            }
        }

        SmallActionButton {
            x: root.width - 110px;
            y: 18px;
            width: 92px;
            height: 28px;
            label: root.toggle_label;
            fill: #21495b;
            activated => {
                root.toggle_view();
            }
        }

        SpectrumTile {
            x: root.gutter;
            y: root.spectrum_y;
            width: root.inner_width;
            height: root.spectrum_height;
            bins: root.spectrum_bins;
            markers: root.spectrum_markers;
        }

        ChartTile { x: root.gutter; y: root.chart_y; width: root.chart_width; height: root.chart_height; data: root.chart0; }
        ChartTile { x: root.gutter + root.chart_width + root.section_gap; y: root.chart_y; width: root.chart_width; height: root.chart_height; data: root.chart1; }
        ChartTile { x: root.gutter; y: root.chart_y + root.chart_height + root.section_gap; width: root.chart_width; height: root.chart_height; data: root.chart2; }
        ChartTile { x: root.gutter + root.chart_width + root.section_gap; y: root.chart_y + root.chart_height + root.section_gap; width: root.chart_width; height: root.chart_height; data: root.chart3; }

        Rectangle {
            x: root.gutter;
            y: root.tuning_y;
            width: root.inner_width;
            height: root.tuning_height;
            background: #0a1720;
            border-radius: 18px;
            border-width: 1px;
            border-color: #142733;

            Text {
                x: 14px;
                y: 12px;
                text: "Tuning";
                color: #f3eadb;
                font-size: 18px;
            }

            SmallActionButton {
                x: parent.width - 76px;
                y: 12px;
                width: 62px;
                height: 22px;
                label: "Reset";
                fill: #6c3c2b;
                activated => {
                    root.reset_tuning();
                }
            }

            Text {
                x: 14px;
                y: 38px;
                text: "Graph range";
                color: #7e96a1;
                font-size: 10px;
            }

            Text {
                x: 94px;
                y: 36px;
                width: 128px;
                text: root.history_label;
                color: #88d7cf;
                font-size: 10px;
            }

            SmallActionButton {
                x: parent.width - 106px;
                y: 32px;
                width: 22px;
                height: 20px;
                label: "-";
                fill: #243f54;
                activated => {
                    root.nudge_history(-1);
                }
            }

            SmallActionButton {
                x: parent.width - 78px;
                y: 32px;
                width: 22px;
                height: 20px;
                label: "+";
                fill: #1c5d55;
                activated => {
                    root.nudge_history(1);
                }
            }

            SmallActionButton {
                x: parent.width - 50px;
                y: 32px;
                width: 36px;
                height: 20px;
                label: "All";
                fill: #21535c;
                activated => {
                    root.show_all_history();
                }
            }

            for gauge[index] in root.left_arcs : ArcGaugeMiniChip {
                x: 12px + index * ((parent.width - 24px) / 10);
                y: 60px;
                width: ((parent.width - 24px) / 10) - 4px;
                height: 26px;
                gauge: gauge;
            }

            for gauge[index] in root.right_arcs : ArcGaugeMiniChip {
                x: 12px + (5 + index) * ((parent.width - 24px) / 10);
                y: 60px;
                width: ((parent.width - 24px) / 10) - 4px;
                height: 26px;
                gauge: gauge;
            }

            for row[index] in root.tuning_rows : Rectangle {
                x: 12px;
                y: 92px + index * 16px;
                width: parent.width - 24px;
                height: 14px;
                background: row.is_active ? #163247 : #0c1620;
                border-radius: 9px;
                border-width: row.is_active ? 1px : 0px;
                border-color: row.is_active ? #5ca89f : #0c1620;

                Text {
                    x: 10px;
                    y: 2px;
                    text: row.label;
                    color: #f3eadb;
                    font-size: 9px;
                }

                Text {
                    x: 116px;
                    y: 2px;
                    width: 78px;
                    text: row.value_label;
                    color: row.is_active ? #88d7cf : #cad5d1;
                    font-size: 9px;
                    horizontal-alignment: right;
                }

                Text {
                    x: 204px;
                    y: 2px;
                    width: parent.width - 270px;
                    text: row.step_label;
                    color: #718692;
                    font-size: 8px;
                    horizontal-alignment: right;
                }

                TouchArea {
                    x: 0px;
                    y: 0px;
                    width: parent.width - 64px;
                    height: parent.height;
                    clicked => {
                        root.focus_tuning(index);
                    }
                }

                SmallActionButton {
                    x: parent.width - 54px;
                    y: 0px;
                    width: 20px;
                    height: 14px;
                    label: "-";
                    fill: #243f54;
                    activated => {
                        root.nudge_tuning(index, -1);
                    }
                }

                SmallActionButton {
                    x: parent.width - 28px;
                    y: 0px;
                    width: 20px;
                    height: 14px;
                    label: "+";
                    fill: #1c5d55;
                    activated => {
                        root.nudge_tuning(index, 1);
                    }
                }
            }
        }

        Rectangle {
            visible: root.show_help;
            x: 12px;
            y: 78px;
            width: root.width - 24px;
            height: root.height - 92px;
            background: #09141b;
            border-radius: 22px;
            border-width: 1px;
            border-color: #173447;

            Rectangle {
                x: 0px;
                y: 0px;
                width: parent.width;
                height: parent.height;
                background: #09141b;
                opacity: 0.98;
                border-radius: 22px;
            }

            Text {
                x: 20px;
                y: 18px;
                text: "Shortcuts";
                color: #f3eadb;
                font-size: 24px;
            }

            Text {
                x: 20px;
                y: 48px;
                width: parent.width - 120px;
                text: "Matches the current keyboard bindings and retained UI controls.";
                color: #90a6ae;
                font-size: 11px;
            }

            SmallActionButton {
                x: parent.width - 74px;
                y: 18px;
                width: 54px;
                height: 24px;
                label: "Close";
                fill: #6c3c2b;
                activated => {
                    root.toggle_help();
                }
            }

            HelpRow { x: 20px; y: 88px; width: parent.width / 2 - 34px; height: 18px; key_label: "`"; description: "Toggle retained deck"; }
            HelpRow { x: 20px; y: 112px; width: parent.width / 2 - 34px; height: 18px; key_label: "H"; description: "Open or close help"; }
            HelpRow { x: 20px; y: 136px; width: parent.width / 2 - 34px; height: 18px; key_label: "V"; description: "Switch hero and diagnostics"; }
            HelpRow { x: 20px; y: 160px; width: parent.width / 2 - 34px; height: 18px; key_label: "Space"; description: "Play or pause"; }
            HelpRow { x: 20px; y: 184px; width: parent.width / 2 - 34px; height: 18px; key_label: "P"; description: "Next graph preset"; }
            HelpRow { x: 20px; y: 208px; width: parent.width / 2 - 34px; height: 18px; key_label: "1"; description: "Cycle beat response"; }
            HelpRow { x: 20px; y: 232px; width: parent.width / 2 - 34px; height: 18px; key_label: "R"; description: "Cycle reactivity"; }
            HelpRow { x: 20px; y: 256px; width: parent.width / 2 - 34px; height: 18px; key_label: "X Y Z S"; description: "Pick graph axis focus"; }

            HelpRow { x: parent.width / 2 + 6px; y: 88px; width: parent.width / 2 - 26px; height: 18px; key_label: "Arrows"; description: "Remap selected axis"; }
            HelpRow { x: parent.width / 2 + 6px; y: 112px; width: parent.width / 2 - 26px; height: 18px; key_label: "N"; description: "Toggle normalization"; }
            HelpRow { x: parent.width / 2 + 6px; y: 136px; width: parent.width / 2 - 26px; height: 18px; key_label: "F"; description: "Toggle framing"; }
            HelpRow { x: parent.width / 2 + 6px; y: 160px; width: parent.width / 2 - 26px; height: 18px; key_label: "PgUp PgDn"; description: "Zoom fixed framing"; }
            HelpRow { x: parent.width / 2 + 6px; y: 184px; width: parent.width / 2 - 26px; height: 18px; key_label: "J L I K U O"; description: "Pan fixed framing"; }
            HelpRow { x: parent.width / 2 + 6px; y: 208px; width: parent.width / 2 - 26px; height: 18px; key_label: "Tab"; description: "Advance tuning focus"; }
            HelpRow { x: parent.width / 2 + 6px; y: 232px; width: parent.width / 2 - 26px; height: 18px; key_label: "[ ]  -  ="; description: "Adjust tuning value"; }
            HelpRow { x: parent.width / 2 + 6px; y: 256px; width: parent.width / 2 - 26px; height: 18px; key_label: "0 / 9"; description: "Reset tuning or view"; }
        }
    }
}

struct OffscreenSlintPlatform {
    start: Instant,
}

impl Platform for OffscreenSlintPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        WINDOW_SLOT.with(|slot| {
            *slot.borrow_mut() = Some(window.clone());
        });
        Ok(window)
    }

    fn duration_since_start(&self) -> Duration {
        self.start.elapsed()
    }
}

#[derive(Copy, Clone)]
struct OverlayRect {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct OverlayVertex {
    position: [f32; 2],
    tex_coords: [f32; 2],
}

impl OverlayVertex {
    const ATTRIBUTES: [wgpu::VertexAttribute; 2] =
        wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2];

    fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBUTES,
        }
    }
}

pub struct SlintDiagnostics {
    component: DiagnosticsSurface,
    window: Rc<MinimalSoftwareWindow>,
    texture: wgpu::Texture,
    texture_view: wgpu::TextureView,
    texture_size: [u32; 2],
    sampler: wgpu::Sampler,
    bind_group_layout: wgpu::BindGroupLayout,
    bind_group: wgpu::BindGroup,
    pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
    pixel_buffer: Vec<slint::Rgb8Pixel>,
    rgba_buffer: Vec<u8>,
    current_rect: Option<OverlayRect>,
    pointer_inside: bool,
    last_local_pointer: Option<[f32; 2]>,
    action_queue: Rc<RefCell<Vec<SlintDiagnosticsAction>>>,
}

impl SlintDiagnostics {
    pub fn new(device: &wgpu::Device, surface_format: wgpu::TextureFormat) -> Result<Self, String> {
        ensure_platform()?;

        let component = DiagnosticsSurface::new()
            .map_err(|error| format!("Failed to construct Slint diagnostics surface: {}", error))?;
        let window = take_window_adapter()?;
        window.set_size(PhysicalSize::new(OVERLAY_BASE_WIDTH, OVERLAY_BASE_HEIGHT));
        component
            .show()
            .map_err(|error| format!("Failed to show Slint diagnostics surface: {}", error))?;

        let action_queue = Rc::new(RefCell::new(Vec::new()));
        {
            let action_queue = Rc::clone(&action_queue);
            component.on_toggle_view(move || {
                action_queue.borrow_mut().push(SlintDiagnosticsAction::ToggleViewMode);
            });
        }
        {
            let action_queue = Rc::clone(&action_queue);
            component.on_toggle_help(move || {
                action_queue.borrow_mut().push(SlintDiagnosticsAction::ToggleHelp);
            });
        }
        {
            let action_queue = Rc::clone(&action_queue);
            component.on_focus_tuning(move |index| {
                if index >= 0 {
                    action_queue
                        .borrow_mut()
                        .push(SlintDiagnosticsAction::FocusTuning(index as usize));
                }
            });
        }
        {
            let action_queue = Rc::clone(&action_queue);
            component.on_nudge_tuning(move |index, direction| {
                if index >= 0 && direction != 0 {
                    action_queue.borrow_mut().push(SlintDiagnosticsAction::NudgeTuning {
                        index: index as usize,
                        direction,
                    });
                }
            });
        }
        {
            let action_queue = Rc::clone(&action_queue);
            component.on_nudge_history(move |direction| {
                if direction != 0 {
                    action_queue
                        .borrow_mut()
                        .push(SlintDiagnosticsAction::NudgeHistory(direction));
                }
            });
        }
        {
            let action_queue = Rc::clone(&action_queue);
            component.on_show_all_history(move || {
                action_queue
                    .borrow_mut()
                    .push(SlintDiagnosticsAction::ShowAllHistory);
            });
        }
        {
            let action_queue = Rc::clone(&action_queue);
            component.on_reset_tuning(move || {
                action_queue.borrow_mut().push(SlintDiagnosticsAction::ResetTuning);
            });
        }

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Slint Overlay Bind Group Layout"),
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

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Slint Overlay Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        let (texture, texture_view) = create_overlay_texture(device, OVERLAY_BASE_WIDTH, OVERLAY_BASE_HEIGHT);
        let bind_group = create_overlay_bind_group(
            device,
            &bind_group_layout,
            &texture_view,
            &sampler,
        );

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Slint Overlay Shader"),
            source: wgpu::ShaderSource::Wgsl(OVERLAY_SHADER.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Slint Overlay Pipeline Layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Slint Overlay Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[OverlayVertex::layout()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
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
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Slint Overlay Vertex Buffer"),
            contents: bytemuck::cast_slice(&[
                OverlayVertex {
                    position: [0.0, 0.0],
                    tex_coords: [0.0, 0.0],
                };
                6
            ]),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });

        let pixel_count = (OVERLAY_BASE_WIDTH * OVERLAY_BASE_HEIGHT) as usize;

        Ok(Self {
            component,
            window,
            texture,
            texture_view,
            texture_size: [OVERLAY_BASE_WIDTH, OVERLAY_BASE_HEIGHT],
            sampler,
            bind_group_layout,
            bind_group,
            pipeline,
            vertex_buffer,
            pixel_buffer: vec![slint::Rgb8Pixel::default(); pixel_count],
            rgba_buffer: vec![0; pixel_count * 4],
            current_rect: None,
            pointer_inside: false,
            last_local_pointer: None,
            action_queue,
        })
    }

    pub fn prepare_frame(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface_size: [u32; 2],
        snapshot: &DiagnosticsSnapshot,
    ) -> bool {
        let Some(rect) = overlay_rect(surface_size) else {
            self.current_rect = None;
            return false;
        };

        if self.texture_size != [rect.width, rect.height] {
            self.resize_texture(device, rect.width, rect.height);
        }

        self.window.set_size(PhysicalSize::new(rect.width, rect.height));
        self.apply_snapshot(snapshot);
        slint::platform::update_timers_and_animations();
        self.window.request_redraw();
        let redrawn = self.window.draw_if_needed(|renderer| {
            renderer.render(&mut self.pixel_buffer, rect.width as usize);
        });

        if redrawn {
            pack_pixels(&self.pixel_buffer, &mut self.rgba_buffer);
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &self.rgba_buffer,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(rect.width * 4),
                    rows_per_image: Some(rect.height),
                },
                wgpu::Extent3d {
                    width: rect.width,
                    height: rect.height,
                    depth_or_array_layers: 1,
                },
            );
        }

        update_overlay_vertices(&self.vertex_buffer, queue, rect, surface_size);
        self.current_rect = Some(rect);
        true
    }

    pub fn on_window_event(
        &mut self,
        event: &WinitWindowEvent,
        surface_size: [u32; 2],
        active: bool,
    ) -> SlintDiagnosticsEventResponse {
        let mut response = SlintDiagnosticsEventResponse::default();

        let rect = if active { overlay_rect(surface_size) } else { None };

        let Some(rect) = rect else {
            self.exit_pointer();
            response.actions = self.drain_actions();
            response.repaint = !response.actions.is_empty();
            return response;
        };

        match event {
            WinitWindowEvent::CursorMoved { position, .. } => {
                let local_x = position.x as f32 - rect.x as f32;
                let local_y = position.y as f32 - rect.y as f32;
                let inside = local_x >= 0.0
                    && local_x <= rect.width as f32
                    && local_y >= 0.0
                    && local_y <= rect.height as f32;

                if inside {
                    self.pointer_inside = true;
                    self.last_local_pointer = Some([local_x, local_y]);
                    self.dispatch(WindowEvent::PointerMoved {
                        position: LogicalPosition::new(local_x, local_y),
                    });
                    response.consumed = true;
                    response.repaint = true;
                } else {
                    if self.pointer_inside {
                        self.exit_pointer();
                        response.repaint = true;
                    }
                    self.last_local_pointer = None;
                }
            }
            WinitWindowEvent::MouseInput { state, button, .. } => {
                if self.pointer_inside {
                    if let (Some(position), Some(button)) =
                        (self.last_local_pointer, map_pointer_button(*button))
                    {
                        let event = match state {
                            ElementState::Pressed => Some(WindowEvent::PointerPressed {
                                position: LogicalPosition::new(position[0], position[1]),
                                button,
                            }),
                            ElementState::Released => Some(WindowEvent::PointerReleased {
                                position: LogicalPosition::new(position[0], position[1]),
                                button,
                            }),
                        };
                        if let Some(event) = event {
                            self.dispatch(event);
                            response.consumed = true;
                            response.repaint = true;
                        }
                    }
                }
            }
            WinitWindowEvent::MouseWheel { delta, .. } => {
                if self.pointer_inside {
                    if let Some(position) = self.last_local_pointer {
                        let (delta_x, delta_y) = match delta {
                            MouseScrollDelta::LineDelta(x, y) => (*x * 40.0, *y * 40.0),
                            MouseScrollDelta::PixelDelta(delta) => (delta.x as f32, delta.y as f32),
                        };
                        self.dispatch(WindowEvent::PointerScrolled {
                            position: LogicalPosition::new(position[0], position[1]),
                            delta_x,
                            delta_y,
                        });
                        response.consumed = true;
                        response.repaint = true;
                    }
                }
            }
            _ => {}
        }

        response.actions = self.drain_actions();
        response.repaint |= !response.actions.is_empty();
        response
    }

    pub fn render<'pass>(&'pass self, render_pass: &mut wgpu::RenderPass<'pass>) {
        if self.current_rect.is_none() {
            return;
        }

        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_bind_group(0, &self.bind_group, &[]);
        render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        render_pass.draw(0..6, 0..1);
    }

    fn apply_snapshot(&self, snapshot: &DiagnosticsSnapshot) {
        self.component
            .set_headline(snapshot.headline.clone().into());
        self.component
            .set_subline(snapshot.subline.clone().into());
        self.component
            .set_graph_line(snapshot.graph_line.clone().into());
        self.component
            .set_toggle_label(snapshot.toggle_label.clone().into());
        self.component
            .set_help_label(snapshot.help_label.clone().into());
        self.component
            .set_show_help(snapshot.show_help);
        self.component.set_chart0(chart_card(snapshot.charts[0].clone()));
        self.component.set_chart1(chart_card(snapshot.charts[1].clone()));
        self.component.set_chart2(chart_card(snapshot.charts[2].clone()));
        self.component.set_chart3(chart_card(snapshot.charts[3].clone()));
        self.component.set_left_arcs(model_from_items(
            snapshot.left_arcs.iter().cloned().map(arc_gauge).collect(),
        ));
        self.component.set_right_arcs(model_from_items(
            snapshot.right_arcs.iter().cloned().map(arc_gauge).collect(),
        ));
        self.component.set_spectrum_bins(model_from_items(
            snapshot
                .spectrum_bins
                .iter()
                .cloned()
                .map(spectrum_bin)
                .collect(),
        ));
        self.component.set_spectrum_markers(model_from_items(
            snapshot
                .spectrum_markers
                .iter()
                .copied()
                .map(spectrum_marker)
                .collect(),
        ));
        self.component
            .set_history_label(snapshot.history_label.clone().into());
        self.component.set_tuning_rows(model_from_items(
            snapshot
                .tuning_rows
                .iter()
                .cloned()
                .map(tuning_row)
                .collect(),
        ));
    }

    fn resize_texture(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        let (texture, texture_view) = create_overlay_texture(device, width, height);
        self.texture = texture;
        self.texture_view = texture_view;
        self.texture_size = [width, height];
        self.bind_group = create_overlay_bind_group(
            device,
            &self.bind_group_layout,
            &self.texture_view,
            &self.sampler,
        );
        let pixel_count = (width * height) as usize;
        self.pixel_buffer = vec![slint::Rgb8Pixel::default(); pixel_count];
        self.rgba_buffer = vec![0; pixel_count * 4];
    }

    fn dispatch(&self, event: WindowEvent) {
        let _ = self.window.try_dispatch_event(event);
    }

    fn exit_pointer(&mut self) {
        if self.pointer_inside {
            self.dispatch(WindowEvent::PointerExited);
        }
        self.pointer_inside = false;
        self.last_local_pointer = None;
    }

    fn drain_actions(&self) -> Vec<SlintDiagnosticsAction> {
        self.action_queue.borrow_mut().drain(..).collect()
    }
}

fn ensure_platform() -> Result<(), String> {
    if PLATFORM_INSTALLED.load(Ordering::Acquire) {
        return Ok(());
    }

    slint::platform::set_platform(Box::new(OffscreenSlintPlatform {
        start: Instant::now(),
    }))
    .map_err(|error| format!("Failed to install Slint off-screen platform: {}", error))?;
    PLATFORM_INSTALLED.store(true, Ordering::Release);
    Ok(())
}

fn take_window_adapter() -> Result<Rc<MinimalSoftwareWindow>, String> {
    WINDOW_SLOT
        .with(|slot| slot.borrow().as_ref().cloned())
        .ok_or_else(|| "Slint did not create the diagnostics window adapter".to_string())
}

fn overlay_rect(surface_size: [u32; 2]) -> Option<OverlayRect> {
    let width = surface_size[0] / 2;
    let height = surface_size[1];

    if width < OVERLAY_MIN_WIDTH || height < OVERLAY_MIN_HEIGHT {
        return None;
    }

    Some(OverlayRect {
        x: 0,
        y: 0,
        width,
        height,
    })
}

fn create_overlay_texture(
    device: &wgpu::Device,
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Slint Overlay Texture"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: OVERLAY_TEXTURE_FORMAT,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

fn create_overlay_bind_group(
    device: &wgpu::Device,
    bind_group_layout: &wgpu::BindGroupLayout,
    texture_view: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Slint Overlay Bind Group"),
        layout: bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(texture_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}

fn update_overlay_vertices(
    vertex_buffer: &wgpu::Buffer,
    queue: &wgpu::Queue,
    rect: OverlayRect,
    surface_size: [u32; 2],
) {
    let width = surface_size[0].max(1) as f32;
    let height = surface_size[1].max(1) as f32;

    let left = rect.x as f32 / width * 2.0 - 1.0;
    let right = (rect.x + rect.width) as f32 / width * 2.0 - 1.0;
    let top = 1.0 - rect.y as f32 / height * 2.0;
    let bottom = 1.0 - (rect.y + rect.height) as f32 / height * 2.0;

    let vertices = [
        OverlayVertex {
            position: [left, top],
            tex_coords: [0.0, 0.0],
        },
        OverlayVertex {
            position: [right, top],
            tex_coords: [1.0, 0.0],
        },
        OverlayVertex {
            position: [left, bottom],
            tex_coords: [0.0, 1.0],
        },
        OverlayVertex {
            position: [left, bottom],
            tex_coords: [0.0, 1.0],
        },
        OverlayVertex {
            position: [right, top],
            tex_coords: [1.0, 0.0],
        },
        OverlayVertex {
            position: [right, bottom],
            tex_coords: [1.0, 1.0],
        },
    ];

    queue.write_buffer(vertex_buffer, 0, bytemuck::cast_slice(&vertices));
}

fn pack_pixels(pixel_buffer: &[slint::Rgb8Pixel], rgba_buffer: &mut [u8]) {
    for (pixel, rgba) in pixel_buffer.iter().zip(rgba_buffer.chunks_exact_mut(4)) {
        rgba[0] = pixel.r;
        rgba[1] = pixel.g;
        rgba[2] = pixel.b;
        rgba[3] = 255;
    }
}

fn chart_card(snapshot: DiagnosticsChartSnapshot) -> UiChartCard {
    UiChartCard {
        tag: SharedString::from(snapshot.tag),
        title: SharedString::from(snapshot.title),
        value_label: SharedString::from(snapshot.value_label),
        accent: Color::from_rgb_u8(snapshot.accent[0], snapshot.accent[1], snapshot.accent[2]),
        samples: model_from_items(
            snapshot
                .samples
                .into_iter()
                .map(|value| UiChartPoint {
                    value: value.clamp(0.0, 1.0),
                })
                .collect(),
        ),
    }
}

fn tuning_row(snapshot: DiagnosticsTuningRowSnapshot) -> UiTuningRow {
    UiTuningRow {
        label: SharedString::from(snapshot.label),
        value_label: SharedString::from(snapshot.value_label),
        step_label: SharedString::from(snapshot.step_label),
        is_active: snapshot.is_active,
    }
}

fn arc_gauge(snapshot: DiagnosticsArcSnapshot) -> UiArcGauge {
    UiArcGauge {
        label: SharedString::from(snapshot.label),
        value_label: SharedString::from(snapshot.value_label),
        accent: Color::from_rgb_u8(snapshot.accent[0], snapshot.accent[1], snapshot.accent[2]),
        value: snapshot.value.clamp(0.0, 1.0),
        peak: snapshot.peak.clamp(0.0, 1.0),
    }
}

fn spectrum_bin(snapshot: DiagnosticsSpectrumBinSnapshot) -> UiSpectrumBin {
    UiSpectrumBin {
        value: snapshot.value.clamp(0.0, 1.0),
        accent: Color::from_rgb_u8(snapshot.accent[0], snapshot.accent[1], snapshot.accent[2]),
    }
}

fn spectrum_marker(position: f32) -> UiSpectrumMarker {
    UiSpectrumMarker {
        position: position.clamp(0.0, 1.0),
    }
}

fn model_from_items<T: Clone + 'static>(items: Vec<T>) -> ModelRc<T> {
    Rc::new(VecModel::from(items)).into()
}

fn map_pointer_button(button: MouseButton) -> Option<PointerEventButton> {
    match button {
        MouseButton::Left => Some(PointerEventButton::Left),
        MouseButton::Right => Some(PointerEventButton::Right),
        MouseButton::Middle => Some(PointerEventButton::Middle),
        MouseButton::Back | MouseButton::Forward | MouseButton::Other(_) => None,
    }
}
