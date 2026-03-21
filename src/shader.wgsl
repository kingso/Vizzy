const LOG_BAND_COUNT: u32 = 24u;
const LOG_BIN_UNIFORM_ROWS: u32 = 6u;
const GRAPH_HISTORY_LENGTH: u32 = 128u;
const MIN_ANALYSIS_FREQUENCY_HZ: f32 = 20.0;

struct Uniforms {
    resolution: vec2<f32>,
    padding: vec2<f32>,
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
    status_color: vec4<f32>,
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
    graph_view_padding: vec2<f32>,
    graph_view_center: vec3<f32>,
    graph_view_extent: f32,
}

struct LogBins {
    bins: array<vec4<f32>, LOG_BIN_UNIFORM_ROWS>,
}

struct GraphHistory {
    points: array<vec4<f32>, GRAPH_HISTORY_LENGTH>,
}

@group(0) @binding(0) var<uniform> uniforms: Uniforms;
@group(0) @binding(1) var<uniform> log_bins: LogBins;
@group(0) @binding(2) var<uniform> graph_history: GraphHistory;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
}

fn background(uv: vec2<f32>) -> vec3<f32> {
    let radial = exp(-length(uv * vec2<f32>(0.88, 1.10)) * 1.25);
    let grid = 0.5 + 0.5 * cos((uv.x + uv.y) * 18.0 + uniforms.time * 0.4);
    let base = mix(vec3<f32>(0.04, 0.05, 0.07), vec3<f32>(0.09, 0.11, 0.14), radial);
    return base + vec3<f32>(0.015, 0.020, 0.028) * grid * 0.18;
}

fn rounded_box(uv: vec2<f32>, center: vec2<f32>, half_size: vec2<f32>, softness: f32) -> f32 {
    let delta = abs(uv - center) - half_size;
    let outside = length(max(delta, vec2<f32>(0.0)));
    let inside = min(max(delta.x, delta.y), 0.0);
    return 1.0 - smoothstep(0.0, softness, outside + inside);
}

fn progress_bar(uv: vec2<f32>) -> vec3<f32> {
    if uniforms.loading_active < 0.5 {
        return vec3<f32>(0.0);
    }

    let frame = rounded_box(uv, vec2<f32>(0.0, 0.77), vec2<f32>(0.56, 0.06), 0.012);
    let lane = rounded_box(uv, vec2<f32>(0.0, 0.77), vec2<f32>(0.52, 0.03), 0.010);
    let fill_width = 0.52 * clamp(uniforms.loading_progress, 0.0, 1.0);
    let fill_center_x = -0.52 + fill_width;
    let fill = rounded_box(uv, vec2<f32>(fill_center_x, 0.77), vec2<f32>(fill_width, 0.03), 0.010);
    let pulse = 0.72 + 0.28 * sin(uniforms.time * 7.5);

    var color = vec3<f32>(0.0);
    color += vec3<f32>(0.12, 0.15, 0.18) * frame;
    color += vec3<f32>(0.06, 0.08, 0.10) * lane;
    color += vec3<f32>(0.26, 0.76, 0.96) * fill * pulse;
    return color;
}

fn log_bin_value(index: u32) -> f32 {
    let row = log_bins.bins[index / 4u];
    return row[index % 4u];
}

fn log_bin_center_frequency(index: u32) -> f32 {
    let nyquist = max(uniforms.analysis_nyquist_hz, MIN_ANALYSIS_FREQUENCY_HZ * 2.0);
    let ratio = max(nyquist / MIN_ANALYSIS_FREQUENCY_HZ, 1.0 + 1e-4);
    let start_t = f32(index) / f32(LOG_BAND_COUNT);
    let end_t = f32(index + 1u) / f32(LOG_BAND_COUNT);
    let start_frequency = MIN_ANALYSIS_FREQUENCY_HZ * pow(ratio, start_t);
    let end_frequency = MIN_ANALYSIS_FREQUENCY_HZ * pow(ratio, end_t);
    return sqrt(start_frequency * end_frequency);
}

fn frequency_to_column_x(frequency_hz: f32) -> f32 {
    let nyquist = max(uniforms.analysis_nyquist_hz, MIN_ANALYSIS_FREQUENCY_HZ * 2.0);
    let clamped = clamp(frequency_hz, MIN_ANALYSIS_FREQUENCY_HZ, nyquist);
    let ratio = max(nyquist / MIN_ANALYSIS_FREQUENCY_HZ, 1.0 + 1e-4);
    let t = log(clamped / MIN_ANALYSIS_FREQUENCY_HZ) / log(ratio);
    return -0.73 + t * 0.064 * f32(LOG_BAND_COUNT - 1u);
}

fn separator_line(uv: vec2<f32>, frequency_hz: f32) -> vec3<f32> {
    let x = frequency_to_column_x(frequency_hz);
    let panel_gate = rounded_box(uv, vec2<f32>(0.0, -0.79), vec2<f32>(0.80, 0.16), 0.016);
    let line = exp(-abs(uv.x - x) * 220.0) * exp(-abs(uv.y + 0.79) * 11.0) * panel_gate;
    return vec3<f32>(0.94, 0.95, 1.00) * line * 0.18;
}

fn metric_row(uv: vec2<f32>, y: f32, value: f32, color: vec3<f32>) -> vec3<f32> {
    let frame = rounded_box(uv, vec2<f32>(0.06, y), vec2<f32>(0.66, 0.055), 0.012);
    let lane = rounded_box(uv, vec2<f32>(0.08, y), vec2<f32>(0.60, 0.026), 0.010);
    let clamped_value = clamp(value, 0.0, 1.0);
    let fill_width = 0.60 * clamped_value;
    let fill_center_x = -0.52 + fill_width;
    let fill = rounded_box(uv, vec2<f32>(fill_center_x, y), vec2<f32>(fill_width, 0.026), 0.010);
    let marker = rounded_box(uv, vec2<f32>(-0.71, y), vec2<f32>(0.030, 0.030), 0.010);
    let glow = exp(-abs(uv.y - y) * 36.0) * exp(-abs(uv.x - (fill_center_x + fill_width * 0.5)) * 8.0) * clamped_value;

    var row = vec3<f32>(0.0);
    row += vec3<f32>(0.11, 0.13, 0.16) * frame;
    row += vec3<f32>(0.05, 0.06, 0.08) * lane;
    row += color * fill;
    row += color * marker * 0.95;
    row += color * glow * 0.10;
    return row;
}

fn status_badge(uv: vec2<f32>) -> vec3<f32> {
    let alpha = uniforms.status_color.w;
    if alpha <= 0.001 {
        return vec3<f32>(0.0);
    }

    let badge = rounded_box(uv, vec2<f32>(-0.78, -0.82), vec2<f32>(0.14, 0.05), 0.012);
    let pulse = 0.75 + 0.25 * sin(uniforms.time * 10.0);
    let color = uniforms.status_color.xyz;

    var indicator = vec3<f32>(0.08, 0.09, 0.11) * badge;
    indicator += color * badge * (0.25 + alpha * 0.55);
    indicator += color * exp(-length(uv - vec2<f32>(-0.88, -0.82)) * 70.0) * pulse * alpha;
    return indicator;
}

fn ascii_digit(value: u32) -> u32 {
    return 48u + min(value, 9u);
}

fn decimal_divisor(power: u32) -> u32 {
    switch power {
        case 4u: { return 10000u; }
        case 3u: { return 1000u; }
        case 2u: { return 100u; }
        case 1u: { return 10u; }
        default: { return 1u; }
    }
}

fn glyph_row(ch: u32, row: u32) -> u32 {
    switch ch {
        case 32u: { return 0u; }
        case 46u: {
            switch row {
                case 6u: { return 4u; }
                default: { return 0u; }
            }
        }
        case 48u: {
            switch row {
                case 0u: { return 14u; }
                case 1u: { return 17u; }
                case 2u: { return 19u; }
                case 3u: { return 21u; }
                case 4u: { return 25u; }
                case 5u: { return 17u; }
                case 6u: { return 14u; }
                default: { return 0u; }
            }
        }
        case 49u: {
            switch row {
                case 0u: { return 4u; }
                case 1u: { return 12u; }
                case 2u: { return 4u; }
                case 3u: { return 4u; }
                case 4u: { return 4u; }
                case 5u: { return 4u; }
                case 6u: { return 14u; }
                default: { return 0u; }
            }
        }
        case 50u: {
            switch row {
                case 0u: { return 14u; }
                case 1u: { return 17u; }
                case 2u: { return 1u; }
                case 3u: { return 2u; }
                case 4u: { return 4u; }
                case 5u: { return 8u; }
                case 6u: { return 31u; }
                default: { return 0u; }
            }
        }
        case 51u: {
            switch row {
                case 0u: { return 30u; }
                case 1u: { return 1u; }
                case 2u: { return 1u; }
                case 3u: { return 14u; }
                case 4u: { return 1u; }
                case 5u: { return 1u; }
                case 6u: { return 30u; }
                default: { return 0u; }
            }
        }
        case 52u: {
            switch row {
                case 0u: { return 2u; }
                case 1u: { return 6u; }
                case 2u: { return 10u; }
                case 3u: { return 18u; }
                case 4u: { return 31u; }
                case 5u: { return 2u; }
                case 6u: { return 2u; }
                default: { return 0u; }
            }
        }
        case 53u: {
            switch row {
                case 0u: { return 31u; }
                case 1u: { return 16u; }
                case 2u: { return 16u; }
                case 3u: { return 30u; }
                case 4u: { return 1u; }
                case 5u: { return 1u; }
                case 6u: { return 30u; }
                default: { return 0u; }
            }
        }
        case 54u: {
            switch row {
                case 0u: { return 14u; }
                case 1u: { return 16u; }
                case 2u: { return 16u; }
                case 3u: { return 30u; }
                case 4u: { return 17u; }
                case 5u: { return 17u; }
                case 6u: { return 14u; }
                default: { return 0u; }
            }
        }
        case 55u: {
            switch row {
                case 0u: { return 31u; }
                case 1u: { return 1u; }
                case 2u: { return 2u; }
                case 3u: { return 4u; }
                case 4u: { return 8u; }
                case 5u: { return 8u; }
                case 6u: { return 8u; }
                default: { return 0u; }
            }
        }
        case 56u: {
            switch row {
                case 0u: { return 14u; }
                case 1u: { return 17u; }
                case 2u: { return 17u; }
                case 3u: { return 14u; }
                case 4u: { return 17u; }
                case 5u: { return 17u; }
                case 6u: { return 14u; }
                default: { return 0u; }
            }
        }
        case 57u: {
            switch row {
                case 0u: { return 14u; }
                case 1u: { return 17u; }
                case 2u: { return 17u; }
                case 3u: { return 15u; }
                case 4u: { return 1u; }
                case 5u: { return 1u; }
                case 6u: { return 14u; }
                default: { return 0u; }
            }
        }
        case 65u: {
            switch row {
                case 0u: { return 14u; }
                case 1u: { return 17u; }
                case 2u: { return 17u; }
                case 3u: { return 31u; }
                case 4u: { return 17u; }
                case 5u: { return 17u; }
                case 6u: { return 17u; }
                default: { return 0u; }
            }
        }
        case 66u: {
            switch row {
                case 0u: { return 30u; }
                case 1u: { return 17u; }
                case 2u: { return 17u; }
                case 3u: { return 30u; }
                case 4u: { return 17u; }
                case 5u: { return 17u; }
                case 6u: { return 30u; }
                default: { return 0u; }
            }
        }
        case 67u: {
            switch row {
                case 0u: { return 14u; }
                case 1u: { return 17u; }
                case 2u: { return 16u; }
                case 3u: { return 16u; }
                case 4u: { return 16u; }
                case 5u: { return 17u; }
                case 6u: { return 14u; }
                default: { return 0u; }
            }
        }
        case 68u: {
            switch row {
                case 0u: { return 30u; }
                case 1u: { return 17u; }
                case 2u: { return 17u; }
                case 3u: { return 17u; }
                case 4u: { return 17u; }
                case 5u: { return 17u; }
                case 6u: { return 30u; }
                default: { return 0u; }
            }
        }
        case 69u: {
            switch row {
                case 0u: { return 31u; }
                case 1u: { return 16u; }
                case 2u: { return 16u; }
                case 3u: { return 30u; }
                case 4u: { return 16u; }
                case 5u: { return 16u; }
                case 6u: { return 31u; }
                default: { return 0u; }
            }
        }
        case 71u: {
            switch row {
                case 0u: { return 14u; }
                case 1u: { return 17u; }
                case 2u: { return 16u; }
                case 3u: { return 23u; }
                case 4u: { return 17u; }
                case 5u: { return 17u; }
                case 6u: { return 15u; }
                default: { return 0u; }
            }
        }
        case 72u: {
            switch row {
                case 0u: { return 17u; }
                case 1u: { return 17u; }
                case 2u: { return 17u; }
                case 3u: { return 31u; }
                case 4u: { return 17u; }
                case 5u: { return 17u; }
                case 6u: { return 17u; }
                default: { return 0u; }
            }
        }
        case 73u: {
            switch row {
                case 0u: { return 14u; }
                case 1u: { return 4u; }
                case 2u: { return 4u; }
                case 3u: { return 4u; }
                case 4u: { return 4u; }
                case 5u: { return 4u; }
                case 6u: { return 14u; }
                default: { return 0u; }
            }
        }
        case 75u: {
            switch row {
                case 0u: { return 17u; }
                case 1u: { return 18u; }
                case 2u: { return 20u; }
                case 3u: { return 24u; }
                case 4u: { return 20u; }
                case 5u: { return 18u; }
                case 6u: { return 17u; }
                default: { return 0u; }
            }
        }
        case 76u: {
            switch row {
                case 0u: { return 16u; }
                case 1u: { return 16u; }
                case 2u: { return 16u; }
                case 3u: { return 16u; }
                case 4u: { return 16u; }
                case 5u: { return 16u; }
                case 6u: { return 31u; }
                default: { return 0u; }
            }
        }
        case 77u: {
            switch row {
                case 0u: { return 17u; }
                case 1u: { return 27u; }
                case 2u: { return 21u; }
                case 3u: { return 21u; }
                case 4u: { return 17u; }
                case 5u: { return 17u; }
                case 6u: { return 17u; }
                default: { return 0u; }
            }
        }
        case 78u: {
            switch row {
                case 0u: { return 17u; }
                case 1u: { return 25u; }
                case 2u: { return 21u; }
                case 3u: { return 19u; }
                case 4u: { return 17u; }
                case 5u: { return 17u; }
                case 6u: { return 17u; }
                default: { return 0u; }
            }
        }
        case 79u: {
            switch row {
                case 0u: { return 14u; }
                case 1u: { return 17u; }
                case 2u: { return 17u; }
                case 3u: { return 17u; }
                case 4u: { return 17u; }
                case 5u: { return 17u; }
                case 6u: { return 14u; }
                default: { return 0u; }
            }
        }
        case 80u: {
            switch row {
                case 0u: { return 30u; }
                case 1u: { return 17u; }
                case 2u: { return 17u; }
                case 3u: { return 30u; }
                case 4u: { return 16u; }
                case 5u: { return 16u; }
                case 6u: { return 16u; }
                default: { return 0u; }
            }
        }
        case 82u: {
            switch row {
                case 0u: { return 30u; }
                case 1u: { return 17u; }
                case 2u: { return 17u; }
                case 3u: { return 30u; }
                case 4u: { return 20u; }
                case 5u: { return 18u; }
                case 6u: { return 17u; }
                default: { return 0u; }
            }
        }
        case 83u: {
            switch row {
                case 0u: { return 15u; }
                case 1u: { return 16u; }
                case 2u: { return 16u; }
                case 3u: { return 14u; }
                case 4u: { return 1u; }
                case 5u: { return 1u; }
                case 6u: { return 30u; }
                default: { return 0u; }
            }
        }
        case 84u: {
            switch row {
                case 0u: { return 31u; }
                case 1u: { return 4u; }
                case 2u: { return 4u; }
                case 3u: { return 4u; }
                case 4u: { return 4u; }
                case 5u: { return 4u; }
                case 6u: { return 4u; }
                default: { return 0u; }
            }
        }
        case 85u: {
            switch row {
                case 0u: { return 17u; }
                case 1u: { return 17u; }
                case 2u: { return 17u; }
                case 3u: { return 17u; }
                case 4u: { return 17u; }
                case 5u: { return 17u; }
                case 6u: { return 14u; }
                default: { return 0u; }
            }
        }
        case 88u: {
            switch row {
                case 0u: { return 17u; }
                case 1u: { return 17u; }
                case 2u: { return 10u; }
                case 3u: { return 4u; }
                case 4u: { return 10u; }
                case 5u: { return 17u; }
                case 6u: { return 17u; }
                default: { return 0u; }
            }
        }
        case 89u: {
            switch row {
                case 0u: { return 17u; }
                case 1u: { return 17u; }
                case 2u: { return 10u; }
                case 3u: { return 4u; }
                case 4u: { return 4u; }
                case 5u: { return 4u; }
                case 6u: { return 4u; }
                default: { return 0u; }
            }
        }
        case 90u: {
            switch row {
                case 0u: { return 31u; }
                case 1u: { return 1u; }
                case 2u: { return 2u; }
                case 3u: { return 4u; }
                case 4u: { return 8u; }
                case 5u: { return 16u; }
                case 6u: { return 31u; }
                default: { return 0u; }
            }
        }
        default: { return 0u; }
    }
}

fn glyph_sample(ch: u32, rel: vec2<f32>) -> f32 {
    if rel.x < 0.0 || rel.x >= 1.0 || rel.y < 0.0 || rel.y >= 1.0 {
        return 0.0;
    }

    let cell_x = min(u32(floor(rel.x * 5.0)), 4u);
    let cell_y = min(u32(floor(rel.y * 7.0)), 6u);
    let row_bits = glyph_row(ch, cell_y);
    let bit = (row_bits >> (4u - cell_x)) & 1u;
    return f32(bit);
}

fn draw_char(uv: vec2<f32>, top_left: vec2<f32>, char_size: vec2<f32>, ch: u32, tint: vec3<f32>) -> vec3<f32> {
    let rel = vec2<f32>(
        (uv.x - top_left.x) / char_size.x,
        (uv.y - top_left.y) / char_size.y,
    );
    let mask = glyph_sample(ch, rel);
    return tint * mask;
}

fn tuning_label_char(focus: u32, index: u32) -> u32 {
    switch focus {
        case 0u: {
            switch index {
                case 0u: { return 82u; }
                case 1u: { return 77u; }
                case 2u: { return 83u; }
                case 3u: { return 32u; }
                case 4u: { return 71u; }
                case 5u: { return 65u; }
                case 6u: { return 84u; }
                case 7u: { return 69u; }
                default: { return 0u; }
            }
        }
        case 1u: {
            switch index {
                case 0u: { return 80u; }
                case 1u: { return 69u; }
                case 2u: { return 65u; }
                case 3u: { return 75u; }
                case 4u: { return 32u; }
                case 5u: { return 71u; }
                case 6u: { return 65u; }
                case 7u: { return 84u; }
                case 8u: { return 69u; }
                default: { return 0u; }
            }
        }
        case 2u: {
            switch index {
                case 0u: { return 83u; }
                case 1u: { return 85u; }
                case 2u: { return 66u; }
                case 3u: { return 32u; }
                case 4u: { return 66u; }
                case 5u: { return 65u; }
                case 6u: { return 83u; }
                case 7u: { return 83u; }
                case 8u: { return 32u; }
                case 9u: { return 77u; }
                case 10u: { return 65u; }
                case 11u: { return 88u; }
                default: { return 0u; }
            }
        }
        case 3u: {
            switch index {
                case 0u: { return 66u; }
                case 1u: { return 65u; }
                case 2u: { return 83u; }
                case 3u: { return 83u; }
                case 4u: { return 32u; }
                case 5u: { return 77u; }
                case 6u: { return 65u; }
                case 7u: { return 88u; }
                default: { return 0u; }
            }
        }
        case 4u: {
            switch index {
                case 0u: { return 77u; }
                case 1u: { return 73u; }
                case 2u: { return 68u; }
                case 3u: { return 32u; }
                case 4u: { return 77u; }
                case 5u: { return 65u; }
                case 6u: { return 88u; }
                default: { return 0u; }
            }
        }
        case 5u: {
            switch index {
                case 0u: { return 80u; }
                case 1u: { return 82u; }
                case 2u: { return 69u; }
                case 3u: { return 83u; }
                case 4u: { return 32u; }
                case 5u: { return 77u; }
                case 6u: { return 73u; }
                case 7u: { return 78u; }
                default: { return 0u; }
            }
        }
        case 6u: {
            switch index {
                case 0u: { return 80u; }
                case 1u: { return 82u; }
                case 2u: { return 69u; }
                case 3u: { return 83u; }
                case 4u: { return 32u; }
                case 5u: { return 77u; }
                case 6u: { return 65u; }
                case 7u: { return 88u; }
                default: { return 0u; }
            }
        }
        default: {
            switch index {
                case 0u: { return 65u; }
                case 1u: { return 73u; }
                case 2u: { return 82u; }
                case 3u: { return 32u; }
                case 4u: { return 77u; }
                case 5u: { return 73u; }
                case 6u: { return 78u; }
                default: { return 0u; }
            }
        }
    }
}

fn tuning_label_length(focus: u32) -> u32 {
    switch focus {
        case 0u: { return 8u; }
        case 1u: { return 9u; }
        case 2u: { return 12u; }
        case 3u: { return 8u; }
        case 4u: { return 7u; }
        case 5u: { return 8u; }
        case 6u: { return 8u; }
        default: { return 7u; }
    }
}

fn formatted_value_length(value: f32, is_hz: bool) -> u32 {
    if !is_hz {
        return 5u;
    }

    let integer_value = u32(round(clamp(value, 0.0, 99999.0)));
    var digits = 1u;
    if integer_value >= 10000u {
        digits = 5u;
    } else if integer_value >= 1000u {
        digits = 4u;
    } else if integer_value >= 100u {
        digits = 3u;
    } else if integer_value >= 10u {
        digits = 2u;
    }
    return digits + 2u;
}

fn formatted_value_char(value: f32, is_hz: bool, index: u32) -> u32 {
    if !is_hz {
        let scaled = u32(round(clamp(value, 0.0, 9.999) * 1000.0));
        let integer_part = scaled / 1000u;
        let decimals = scaled % 1000u;
        switch index {
            case 0u: { return ascii_digit(integer_part); }
            case 1u: { return 46u; }
            case 2u: { return ascii_digit(decimals / 100u); }
            case 3u: { return ascii_digit((decimals / 10u) % 10u); }
            case 4u: { return ascii_digit(decimals % 10u); }
            default: { return 0u; }
        }
    }

    let integer_value = u32(round(clamp(value, 0.0, 99999.0)));
    let digits = formatted_value_length(value, true) - 2u;
    if index < digits {
        let power = digits - 1u - index;
        let divisor = decimal_divisor(power);
        return ascii_digit((integer_value / divisor) % 10u);
    }

    if index == digits {
        return 72u;
    }

    if index == digits + 1u {
        return 90u;
    }

    return 0u;
}

fn draw_text_line(
    uv: vec2<f32>,
    origin: vec2<f32>,
    char_size: vec2<f32>,
    max_chars: u32,
    line_type: u32,
    focus: u32,
    value: f32,
    is_hz: bool,
    tint: vec3<f32>,
) -> vec3<f32> {
    var color = vec3<f32>(0.0);
    let spacing = char_size.x * 0.18;
    for (var index: u32 = 0u; index < max_chars; index = index + 1u) {
        var ch = 0u;
        if line_type == 0u {
            if index < tuning_label_length(focus) {
                ch = tuning_label_char(focus, index);
            }
        } else if line_type == 1u {
            if index < formatted_value_length(value, is_hz) {
                ch = formatted_value_char(value, is_hz, index);
            }
        } else {
            switch index {
                case 0u: { ch = 83u; }
                case 1u: { ch = 84u; }
                case 2u: { ch = 69u; }
                case 3u: { ch = 80u; }
                case 4u: { ch = 32u; }
                default: {
                    let value_index = index - 5u;
                    if value_index < formatted_value_length(value, is_hz) {
                        ch = formatted_value_char(value, is_hz, value_index);
                    }
                }
            }
        }

        if ch != 0u {
            let top_left = origin + vec2<f32>(f32(index) * (char_size.x + spacing), 0.0);
            color += draw_char(uv, top_left, char_size, ch, tint);
        }
    }

    return color;
}

fn tuning_hud(uv: vec2<f32>) -> vec3<f32> {
    let panel = rounded_box(uv, vec2<f32>(-0.28, 0.82), vec2<f32>(0.48, 0.12), 0.016);
    let focus = u32(round(uniforms.active_tuning_focus));
    let is_hz = uniforms.active_tuning_is_hz > 0.5;
    var accent = vec3<f32>(0.96, 0.72, 0.24);
    if is_hz {
        accent = vec3<f32>(0.26, 0.74, 0.98);
    }

    var color = vec3<f32>(0.07, 0.08, 0.10) * panel;
    color += accent * rounded_box(uv, vec2<f32>(-0.70, 0.82), vec2<f32>(0.03, 0.10), 0.010) * 0.85;
    color += draw_text_line(uv, vec2<f32>(-0.63, 0.73), vec2<f32>(0.030, 0.050), 12u, 0u, focus, 0.0, false, vec3<f32>(0.86, 0.90, 0.95));
    color += draw_text_line(uv, vec2<f32>(-0.63, 0.80), vec2<f32>(0.036, 0.058), 8u, 1u, focus, uniforms.active_tuning_value, is_hz, accent);
    color += draw_text_line(uv, vec2<f32>(-0.63, 0.88), vec2<f32>(0.026, 0.042), 12u, 2u, focus, uniforms.active_tuning_step, is_hz, vec3<f32>(0.72, 0.78, 0.86));
    return color;
}

fn axis_source_length(source: u32) -> u32 {
    switch source {
        case 0u: { return 8u; }
        case 1u: { return 4u; }
        case 2u: { return 3u; }
        case 3u: { return 6u; }
        case 4u: { return 8u; }
        case 5u: { return 3u; }
        case 6u: { return 4u; }
        case 7u: { return 4u; }
        case 8u: { return 4u; }
        case 9u: { return 4u; }
        case 10u: { return 6u; }
        default: { return 7u; }
    }
}

fn axis_source_char(source: u32, index: u32) -> u32 {
    switch source {
        case 0u: {
            switch index {
                case 0u: { return 83u; }
                case 1u: { return 85u; }
                case 2u: { return 66u; }
                case 3u: { return 32u; }
                case 4u: { return 66u; }
                case 5u: { return 65u; }
                case 6u: { return 83u; }
                case 7u: { return 83u; }
                default: { return 0u; }
            }
        }
        case 1u: {
            switch index {
                case 0u: { return 66u; }
                case 1u: { return 65u; }
                case 2u: { return 83u; }
                case 3u: { return 83u; }
                default: { return 0u; }
            }
        }
        case 2u: {
            switch index {
                case 0u: { return 77u; }
                case 1u: { return 73u; }
                case 2u: { return 68u; }
                default: { return 0u; }
            }
        }
        case 3u: {
            switch index {
                case 0u: { return 84u; }
                case 1u: { return 82u; }
                case 2u: { return 69u; }
                case 3u: { return 66u; }
                case 4u: { return 76u; }
                case 5u: { return 69u; }
                default: { return 0u; }
            }
        }
        case 4u: {
            switch index {
                case 0u: { return 80u; }
                case 1u: { return 82u; }
                case 2u: { return 69u; }
                case 3u: { return 83u; }
                case 4u: { return 69u; }
                case 5u: { return 78u; }
                case 6u: { return 67u; }
                case 7u: { return 69u; }
                default: { return 0u; }
            }
        }
        case 5u: {
            switch index {
                case 0u: { return 65u; }
                case 1u: { return 73u; }
                case 2u: { return 82u; }
                default: { return 0u; }
            }
        }
        case 6u: {
            switch index {
                case 0u: { return 76u; }
                case 1u: { return 79u; }
                case 2u: { return 85u; }
                case 3u: { return 68u; }
                default: { return 0u; }
            }
        }
        case 7u: {
            switch index {
                case 0u: { return 80u; }
                case 1u: { return 69u; }
                case 2u: { return 65u; }
                case 3u: { return 75u; }
                default: { return 0u; }
            }
        }
        case 8u: {
            switch index {
                case 0u: { return 66u; }
                case 1u: { return 69u; }
                case 2u: { return 65u; }
                case 3u: { return 84u; }
                default: { return 0u; }
            }
        }
        case 9u: {
            switch index {
                case 0u: { return 67u; }
                case 1u: { return 69u; }
                case 2u: { return 78u; }
                case 3u: { return 84u; }
                default: { return 0u; }
            }
        }
        case 10u: {
            switch index {
                case 0u: { return 76u; }
                case 1u: { return 79u; }
                case 2u: { return 71u; }
                case 3u: { return 32u; }
                case 4u: { return 72u; }
                case 5u: { return 90u; }
                default: { return 0u; }
            }
        }
        default: {
            switch index {
                case 0u: { return 66u; }
                case 1u: { return 73u; }
                case 2u: { return 78u; }
                case 3u: { return 32u; }
                case 4u: { return 73u; }
                case 5u: { return 68u; }
                case 6u: { return 88u; }
                default: { return 0u; }
            }
        }
    }
}

fn draw_axis_selector_line(
    uv: vec2<f32>,
    origin: vec2<f32>,
    axis_char: u32,
    source: u32,
    is_active: bool,
) -> vec3<f32> {
    var accent = vec3<f32>(0.68, 0.74, 0.84);
    if is_active {
        accent = vec3<f32>(0.98, 0.78, 0.28);
    }
    var color = draw_char(uv, origin, vec2<f32>(0.034, 0.052), axis_char, accent);
    color += draw_char(uv, origin + vec2<f32>(0.045, 0.0), vec2<f32>(0.028, 0.044), 45u, accent);

    let text_origin = origin + vec2<f32>(0.086, 0.0);
    let spacing = 0.028 * 0.18;
    for (var index: u32 = 0u; index < axis_source_length(source); index = index + 1u) {
        let ch = axis_source_char(source, index);
        if ch != 0u {
            color += draw_char(
                uv,
                text_origin + vec2<f32>(f32(index) * (0.028 + spacing), 0.0),
                vec2<f32>(0.028, 0.044),
                ch,
                vec3<f32>(0.86, 0.90, 0.95),
            );
        }
    }

    if is_active {
        color += accent * rounded_box(uv, origin + vec2<f32>(0.28, 0.026), vec2<f32>(0.28, 0.032), 0.010) * 0.22;
    }
    return color;
}

fn axis_selector_hud(uv: vec2<f32>) -> vec3<f32> {
    let panel = rounded_box(uv, vec2<f32>(0.30, 0.82), vec2<f32>(0.48, 0.17), 0.016);
    var color = vec3<f32>(0.07, 0.08, 0.10) * panel;
    let active_focus = u32(round(uniforms.active_axis_focus));
    color += draw_axis_selector_line(uv, vec2<f32>(-0.10, 0.69), 88u, u32(round(uniforms.axis_x_source)), active_focus == 0u);
    color += draw_axis_selector_line(uv, vec2<f32>(-0.10, 0.77), 89u, u32(round(uniforms.axis_y_source)), active_focus == 1u);
    color += draw_axis_selector_line(uv, vec2<f32>(-0.10, 0.85), 90u, u32(round(uniforms.axis_z_source)), active_focus == 2u);
    color += draw_axis_selector_line(uv, vec2<f32>(-0.10, 0.93), 83u, u32(round(uniforms.axis_size_source)), active_focus == 3u);
    return color;
}

fn graph_axis_labels(uv: vec2<f32>) -> vec3<f32> {
    let min_point = graph_view_min();
    let max_point = graph_view_max();
    let origin = vec3<f32>(min_point.x, min_point.y, min_point.z);
    let x_tip = vec3<f32>(max_point.x, min_point.y, min_point.z);
    let y_tip = vec3<f32>(min_point.x, max_point.y, min_point.z);
    let z_tip = vec3<f32>(min_point.x, min_point.y, max_point.z);

    let origin_projected = project_graph_point(origin);
    let x_projected = project_graph_point(x_tip);
    let y_projected = project_graph_point(y_tip);
    let z_projected = project_graph_point(z_tip);

    let x_direction = normalize(x_projected - origin_projected + vec2<f32>(1e-4, 0.0));
    let y_direction = normalize(y_projected - origin_projected + vec2<f32>(1e-4, 0.0));
    let z_direction = normalize(z_projected - origin_projected + vec2<f32>(1e-4, 0.0));

    let char_size = vec2<f32>(0.040, 0.062);
    let x_anchor = x_projected + x_direction * 0.035 + vec2<f32>(0.008, -0.020);
    let y_anchor = y_projected + y_direction * 0.035 + vec2<f32>(0.008, -0.020);
    let z_anchor = z_projected + z_direction * 0.035 + vec2<f32>(0.008, -0.020);

    var color = vec3<f32>(0.0);
    color += draw_char(uv, x_anchor, char_size, 88u, vec3<f32>(0.92, 0.66, 0.26));
    color += draw_char(uv, y_anchor, char_size, 89u, vec3<f32>(0.40, 0.86, 0.54));
    color += draw_char(uv, z_anchor, char_size, 90u, vec3<f32>(0.34, 0.72, 0.98));
    return color;
}

fn log_bin_columns(uv: vec2<f32>) -> vec3<f32> {
    let panel = rounded_box(uv, vec2<f32>(0.0, -0.79), vec2<f32>(0.80, 0.16), 0.016);
    var color = vec3<f32>(0.08, 0.09, 0.11) * panel;

    let base_y = -0.92;
    let max_height = 0.26;
    for (var index: u32 = 0u; index < LOG_BAND_COUNT; index = index + 1u) {
        let x = -0.73 + f32(index) * 0.064;
        let value = clamp(log_bin_value(index), 0.0, 1.0);
        let height = 0.012 + value * max_height;
        let column = rounded_box(uv, vec2<f32>(x, base_y + height), vec2<f32>(0.020, height), 0.010);
        let glow = exp(-abs(uv.x - x) * 42.0) * exp(-abs(uv.y - (base_y + height * 2.0)) * 18.0) * value;
        let center_frequency = log_bin_center_frequency(index);
        var tint = vec3<f32>(0.72, 0.88, 1.00);
        if center_frequency < uniforms.split_sub_bass_max_hz {
            tint = vec3<f32>(1.00, 0.58, 0.20);
        } else if center_frequency < uniforms.split_bass_max_hz {
            tint = vec3<f32>(0.96, 0.38, 0.18);
        } else if center_frequency < uniforms.split_mid_max_hz {
            tint = vec3<f32>(0.22, 0.84, 0.46);
        } else if center_frequency < uniforms.split_presence_min_hz {
            tint = vec3<f32>(0.22, 0.60, 0.98);
        } else if center_frequency < uniforms.split_presence_max_hz {
            tint = vec3<f32>(0.98, 0.40, 0.72);
        } else if center_frequency < uniforms.split_air_min_hz {
            tint = vec3<f32>(0.52, 0.62, 0.95);
        } else {
            tint = vec3<f32>(0.72, 0.88, 1.00);
        }
        color += tint * column;
        color += tint * glow * 0.12;
    }

    color += separator_line(uv, uniforms.split_sub_bass_max_hz);
    color += separator_line(uv, uniforms.split_bass_max_hz);
    color += separator_line(uv, uniforms.split_mid_max_hz);
    color += separator_line(uv, uniforms.split_presence_min_hz);
    color += separator_line(uv, uniforms.split_presence_max_hz);
    color += separator_line(uv, uniforms.split_air_min_hz);

    return color;
}

fn rotate_y(point: vec3<f32>, angle: f32) -> vec3<f32> {
    let c = cos(angle);
    let s = sin(angle);
    return vec3<f32>(c * point.x + s * point.z, point.y, -s * point.x + c * point.z);
}

fn rotate_x(point: vec3<f32>, angle: f32) -> vec3<f32> {
    let c = cos(angle);
    let s = sin(angle);
    return vec3<f32>(point.x, c * point.y - s * point.z, s * point.y + c * point.z);
}

fn graph_view_min() -> vec3<f32> {
    return uniforms.graph_view_center - vec3<f32>(uniforms.graph_view_extent);
}

fn graph_view_max() -> vec3<f32> {
    return uniforms.graph_view_center + vec3<f32>(uniforms.graph_view_extent);
}

fn project_graph_point(point: vec3<f32>) -> vec2<f32> {
    let extent = max(uniforms.graph_view_extent, 1e-4);
    var centered = (point - uniforms.graph_view_center) / extent;
    centered *= vec3<f32>(1.25, 1.05, 1.25);
    let rotated_y = rotate_y(centered, uniforms.time * 0.18 + 0.75);
    let rotated = rotate_x(rotated_y, -0.55);
    let depth = 3.4 + rotated.z;
    return vec2<f32>(rotated.x / depth, rotated.y / depth) * 1.85;
}

fn distance_to_segment(point: vec2<f32>, start: vec2<f32>, end: vec2<f32>) -> f32 {
    let segment = end - start;
    let length_squared = max(dot(segment, segment), 1e-5);
    let t = clamp(dot(point - start, segment) / length_squared, 0.0, 1.0);
    return length(point - (start + segment * t));
}

fn graph_point(index: u32) -> vec4<f32> {
    return graph_history.points[index];
}

fn graph_edge(uv: vec2<f32>, start: vec3<f32>, end: vec3<f32>, tint: vec3<f32>) -> vec3<f32> {
    let a = project_graph_point(start);
    let b = project_graph_point(end);
    let line = exp(-distance_to_segment(uv, a, b) * 180.0);
    return tint * line * 0.12;
}

fn graph_wireframe(uv: vec2<f32>) -> vec3<f32> {
    let tint = vec3<f32>(0.24, 0.30, 0.38);
    let min_point = graph_view_min();
    let max_point = graph_view_max();
    let p000 = vec3<f32>(min_point.x, min_point.y, min_point.z);
    let p100 = vec3<f32>(max_point.x, min_point.y, min_point.z);
    let p010 = vec3<f32>(min_point.x, max_point.y, min_point.z);
    let p001 = vec3<f32>(min_point.x, min_point.y, max_point.z);
    let p110 = vec3<f32>(max_point.x, max_point.y, min_point.z);
    let p101 = vec3<f32>(max_point.x, min_point.y, max_point.z);
    let p011 = vec3<f32>(min_point.x, max_point.y, max_point.z);
    let p111 = vec3<f32>(max_point.x, max_point.y, max_point.z);
    var color = vec3<f32>(0.0);
    color += graph_edge(uv, p000, p100, tint);
    color += graph_edge(uv, p000, p010, tint);
    color += graph_edge(uv, p000, p001, tint);
    color += graph_edge(uv, p111, p011, tint);
    color += graph_edge(uv, p111, p101, tint);
    color += graph_edge(uv, p111, p110, tint);
    color += graph_edge(uv, p100, p110, tint);
    color += graph_edge(uv, p100, p101, tint);
    color += graph_edge(uv, p010, p110, tint);
    color += graph_edge(uv, p010, p011, tint);
    color += graph_edge(uv, p001, p101, tint);
    color += graph_edge(uv, p001, p011, tint);
    return color;
}

fn graph_trace(uv: vec2<f32>) -> vec3<f32> {
    var color = vec3<f32>(0.0);
    for (var index: u32 = 1u; index < GRAPH_HISTORY_LENGTH; index = index + 1u) {
        let previous = graph_point(index - 1u);
        let current = graph_point(index);
        if previous.w < 0.0 || current.w < 0.0 {
            continue;
        }

        let age = f32(index) / f32(GRAPH_HISTORY_LENGTH - 1u);
        let size = clamp(current.w, 0.0, 1.0);
        let projected_previous = project_graph_point(previous.xyz);
        let projected_current = project_graph_point(current.xyz);
        let distance = distance_to_segment(uv, projected_previous, projected_current);
        let line = exp(-distance * 210.0);
        let dot_falloff = mix(260.0, 120.0, size);
        let tint = mix(vec3<f32>(0.20, 0.58, 0.98), vec3<f32>(0.98, 0.54, 0.22), age);
        color += tint * line * (0.32 + age * 0.48);
        color += tint * exp(-length(uv - projected_current) * dot_falloff) * (0.08 + size * 0.18);
    }
    return color;
}

fn graph_panel(uv: vec2<f32>) -> vec3<f32> {
    let panel = rounded_box(uv, vec2<f32>(0.0, 0.0), vec2<f32>(0.98, 0.98), 0.000);
    var color = vec3<f32>(0.06, 0.07, 0.09) * panel;
    color += graph_wireframe(uv);
    color += graph_trace(uv);
    color += graph_axis_labels(uv);
    return color * panel;
}

fn diagnostics_dashboard(uv: vec2<f32>) -> vec3<f32> {
    var color = background(uv);
    color += progress_bar(uv);
    color += tuning_hud(uv);
    color += axis_selector_hud(uv);
    if uniforms.show_help > 0.5 {
        color += vec3<f32>(0.22, 0.30, 0.40) * rounded_box(uv, vec2<f32>(0.0, 0.84), vec2<f32>(0.80, 0.05), 0.014);
    }
    color += metric_row(uv, 0.60, uniforms.audio_sub_bass, vec3<f32>(1.00, 0.52, 0.20));
    color += metric_row(uv, 0.48, uniforms.audio_bass, vec3<f32>(0.92, 0.38, 0.18));
    color += metric_row(uv, 0.36, uniforms.audio_mid, vec3<f32>(0.20, 0.82, 0.44));
    color += metric_row(uv, 0.24, uniforms.audio_treble, vec3<f32>(0.20, 0.58, 0.98));
    color += metric_row(uv, 0.12, uniforms.audio_presence, vec3<f32>(0.98, 0.36, 0.70));
    color += metric_row(uv, 0.00, uniforms.audio_air, vec3<f32>(0.66, 0.84, 1.00));
    color += metric_row(uv, -0.12, uniforms.audio_loudness, vec3<f32>(0.95, 0.78, 0.22));
    color += metric_row(uv, -0.24, uniforms.audio_peak, vec3<f32>(0.98, 0.48, 0.62));
    color += metric_row(uv, -0.36, uniforms.audio_beat, vec3<f32>(0.86, 0.36, 1.00));
    color += metric_row(uv, -0.48, uniforms.audio_centroid, vec3<f32>(0.24, 0.92, 0.94));
    color += log_bin_columns(uv);
    color += status_badge(uv);
    return color;
}

fn dashboard(uv: vec2<f32>) -> vec3<f32> {
    let left_center = vec2<f32>(-1.02, 0.0);
    let left_half_size = vec2<f32>(0.72, 0.98);
    let right_center = vec2<f32>(0.84, 0.0);
    let right_half_size = vec2<f32>(0.78, 0.78);

    let left_mask = rounded_box(uv, left_center, left_half_size, 0.026);
    let right_mask = rounded_box(uv, right_center, right_half_size, 0.026);
    let left_uv = (uv - left_center) / left_half_size;
    let right_uv = (uv - right_center) / right_half_size;

    var color = background(uv) * 0.55;
    color += diagnostics_dashboard(left_uv) * left_mask;
    color += graph_panel(right_uv) * right_mask;
    color += vec3<f32>(0.22, 0.26, 0.32) * exp(-abs(uv.x + 0.12) * 140.0) * exp(-abs(uv.y) * 0.9) * 0.30;
    color = pow(clamp(color, vec3<f32>(0.0), vec3<f32>(1.0)), vec3<f32>(0.96));
    return color;
}

@vertex
fn vs_main(@builtin(vertex_index) in_vertex_index: u32) -> VertexOutput {
    var out: VertexOutput;
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, 3.0),
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0)
    );
    out.position = vec4<f32>(positions[in_vertex_index], 0.0, 1.0);
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = (in.position.xy * 2.0 - uniforms.resolution) / uniforms.resolution.y;
    let color = dashboard(uv);
    return vec4<f32>(color, 1.0);
}
