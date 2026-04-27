const LOG_BAND_COUNT: u32 = 24u;
const LOG_BIN_UNIFORM_ROWS: u32 = 6u;
const GRAPH_HISTORY_LENGTH: u32 = 128u;
const MIN_GRAPH_HISTORY_POINTS: u32 = 16u;
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
    reactivity_mode: f32,
    peak_sub_bass: f32,
    peak_bass: f32,
    peak_mid: f32,
    peak_treble: f32,
    peak_presence: f32,
    peak_air: f32,
    peak_loudness: f32,
    peak_peak: f32,
    peak_beat: f32,
    peak_centroid: f32,
    graph_history_count: f32,
    graph_offset: vec2<f32>,
    spectrum_offset: vec2<f32>,
    left_arc_offset: vec2<f32>,
    right_arc_offset: vec2<f32>,
    chart_x_offset: vec2<f32>,
    chart_y_offset: vec2<f32>,
    chart_z_offset: vec2<f32>,
    chart_s_offset: vec2<f32>,
    tuning_offset: vec2<f32>,
    axis_offset: vec2<f32>,
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

fn hash21(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3<f32>(p.x, p.y, p.x) * 0.1031);
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

fn hash31(p: vec3<f32>) -> f32 {
    var p3 = fract(p * 0.1031);
    p3 += dot(p3, p3.zyx + 31.32);
    return fract((p3.x + p3.y) * p3.z);
}

fn reactivity_scale() -> f32 {
    return clamp(uniforms.reactivity_mode * 0.5, 0.0, 1.0);
}

fn background(uv: vec2<f32>) -> vec3<f32> {
    let aspect = uniforms.resolution.x / uniforms.resolution.y;
    let stage_uv = vec2<f32>(uv.x / max(aspect, 1e-4), uv.y);
    let base = mix(vec3<f32>(0.014, 0.014, 0.016), vec3<f32>(0.030, 0.030, 0.034), smoothstep(-0.18, 0.95, uv.y));
    let overhead = exp(-length(vec2<f32>(stage_uv.x * 2.2, (uv.y + 0.98) * 2.9)) * 4.6);
    let floor_shape = vec2<f32>(stage_uv.x * 2.8, (uv.y - 0.72) * 4.2);
    let floor_glow = exp(-dot(floor_shape, floor_shape) * 1.8);
    let vignette = smoothstep(0.45, 1.15, length(vec2<f32>(stage_uv.x * 1.2, uv.y * 0.95)));

    var color = base;
    color += vec3<f32>(0.070, 0.070, 0.076) * overhead * 0.55;
    color += vec3<f32>(0.155, 0.156, 0.162) * floor_glow * 0.34;
    color *= 1.0 - vignette * 0.58;
    color += vec3<f32>(0.020, 0.020, 0.024) * floor_glow * 0.28;
    return color;
}

fn rounded_box(uv: vec2<f32>, center: vec2<f32>, half_size: vec2<f32>, softness: f32) -> f32 {
    let delta = abs(uv - center) - half_size;
    let outside = length(max(delta, vec2<f32>(0.0)));
    let inside = min(max(delta.x, delta.y), 0.0);
    return step(outside + inside, 0.0);
}

fn holographic_panel(uv: vec2<f32>, center: vec2<f32>, half_size: vec2<f32>, tint: vec3<f32>) -> vec3<f32> {
    let outer = rounded_box(uv, center, half_size, 0.016);
    let inner = rounded_box(uv, center, half_size - vec2<f32>(0.012, 0.012), 0.012);
    let border = max(outer - inner, 0.0);

    let edge_dist = length((uv - center) / half_size);
    let aberration = step(0.90, edge_dist) * 0.08;
    let r_shift = rounded_box(uv + vec2<f32>(0.003, 0.0) * aberration, center, half_size - vec2<f32>(0.012, 0.012), 0.012);
    let b_shift = rounded_box(uv - vec2<f32>(0.003, 0.0) * aberration, center, half_size - vec2<f32>(0.012, 0.012), 0.012);

    let sweep = 0.62 + 0.18 * sin((uv.y + center.x * 0.35) * 18.0 - uniforms.time * 0.65);

    var color = tint * border * (0.26 + sweep * 0.24);
    color.r += tint.r * max(outer - r_shift, 0.0) * aberration * 0.18;
    color.b += tint.b * max(outer - b_shift, 0.0) * aberration * 0.18;
    return color;
}

fn hud_corner(uv: vec2<f32>, corner: vec2<f32>, size: f32, tint: vec3<f32>) -> vec3<f32> {
    let rel = (uv - corner) * sign(corner);
    if rel.x < -0.01 || rel.y < -0.01 || rel.x > size * 1.5 || rel.y > size * 1.5 {
        return vec3<f32>(0.0);
    }

    let arm_thickness = 0.004;
    let arm_length = size;

    // horizontal arm
    let h_arm = step(0.0, rel.x) * step(rel.x, arm_length) * step(abs(rel.y), arm_thickness);
    // vertical arm
    let v_arm = step(0.0, rel.y) * step(rel.y, arm_length) * step(abs(rel.x), arm_thickness);

    var color = tint * (h_arm + v_arm) * 0.7;

    // tick marks along arms
    let h_ticks = step(0.0, rel.x) * step(rel.x, arm_length) *
        step(fract(rel.x / (arm_length * 0.2) + 0.5), 0.15) *
        step(abs(rel.y - arm_thickness * 3.0), arm_thickness * 0.8);
    let v_ticks = step(0.0, rel.y) * step(rel.y, arm_length) *
        step(fract(rel.y / (arm_length * 0.2) + 0.5), 0.15) *
        step(abs(rel.x - arm_thickness * 3.0), arm_thickness * 0.8);
    color += tint * (h_ticks + v_ticks) * 0.35;

    return color;
}

fn hud_corners(uv: vec2<f32>) -> vec3<f32> {
    let tint = vec3<f32>(0.20, 0.21, 0.24);
    let inset = 0.06;
    let arm_size = 0.14;
    let aspect = uniforms.resolution.x / uniforms.resolution.y;
    let ex = aspect - inset;
    let ey = 1.0 - inset;
    var color = vec3<f32>(0.0);
    color += hud_corner(uv, vec2<f32>(-ex, ey), arm_size, tint);
    color += hud_corner(uv, vec2<f32>(ex, ey), arm_size, tint);
    color += hud_corner(uv, vec2<f32>(-ex, -ey), arm_size, tint);
    color += hud_corner(uv, vec2<f32>(ex, -ey), arm_size, tint);
    return color;
}

const PI: f32 = 3.14159265;

fn arc_gauge(uv: vec2<f32>, center: vec2<f32>, radius: f32, value: f32, peak: f32, tint: vec3<f32>, aspect: f32) -> vec3<f32> {
    // early-out: skip if pixel is far from the gauge
    let dx = (uv.x - center.x) * aspect;
    let dy = uv.y - center.y;
    let dist_sq = dx * dx + dy * dy;
    let outer = radius + 0.03;
    if dist_sq > outer * outer {
        return vec3<f32>(0.0);
    }

    let dist = sqrt(dist_sq);
    let angle = atan2(-dx, dy);

    let arc_start = -PI * 1.25;
    let arc_end = PI * 0.25;
    let arc_span = arc_end - arc_start;

    let clamped_value = clamp(value, 0.0, 1.0);
    let fill_angle = arc_start + clamped_value * arc_span;
    let on_arc = step(arc_start, angle) * step(angle, arc_end);
    let on_fill = step(arc_start, angle) * step(angle, fill_angle);

    // track ring
    let track_thickness = 0.006;
    let track = on_arc * step(abs(dist - radius), track_thickness) * 0.25;

    // fill ring
    let fill_thickness = 0.007;
    let fill = on_fill * step(abs(dist - radius), fill_thickness);

    // value indicator dot
    let indicator_corrected = vec2<f32>(-sin(fill_angle), cos(fill_angle)) * radius;
    let indicator_world = center + vec2<f32>(indicator_corrected.x / aspect, indicator_corrected.y);
    let indicator = step(length(vec2<f32>((uv.x - indicator_world.x) * aspect, uv.y - indicator_world.y)), 0.012);

    // peak max dot (white)
    let clamped_peak = clamp(peak, 0.0, 1.0);
    let peak_angle = arc_start + clamped_peak * arc_span;
    let peak_corrected = vec2<f32>(-sin(peak_angle), cos(peak_angle)) * radius;
    let peak_world = center + vec2<f32>(peak_corrected.x / aspect, peak_corrected.y);
    let peak_dot = step(length(vec2<f32>((uv.x - peak_world.x) * aspect, uv.y - peak_world.y)), 0.008);

    // tick marks — angle-based (no distance_to_segment)
    var tick_val = 0.0;
    let on_tick_ring = step(abs(dist - (radius + 0.017)), 0.006);
    if on_tick_ring > 0.0 {
        for (var i: u32 = 0u; i <= 10u; i = i + 1u) {
            let tick_angle = arc_start + f32(i) / 10.0 * arc_span;
            let angle_diff = abs(angle - tick_angle);
            tick_val += step(angle_diff, 0.04);
        }
    }
    let ticks = vec3<f32>(0.72, 0.78, 0.88) * min(tick_val, 1.0) * on_tick_ring * 0.6;

    var color = vec3<f32>(0.0);
    color += vec3<f32>(0.10, 0.14, 0.20) * track;
    color += tint * fill * 0.85;
    color += tint * indicator * 1.2;
    color += vec3<f32>(1.00, 1.00, 1.00) * peak_dot * 0.9;
    color += ticks;
    return color;
}

fn particles(uv: vec2<f32>) -> vec3<f32> {
    let rx = reactivity_scale();
    if rx < 0.01 {
        return vec3<f32>(0.0);
    }

    var color = vec3<f32>(0.0);
    let count = 40.0;
    let t = uniforms.time;

    for (var i: f32 = 0.0; i < count; i = i + 1.0) {
        let seed = vec2<f32>(i * 7.23, i * 13.71);
        let base_x = hash21(seed) * 3.6 - 1.8;
        let drift_speed = 0.04 + hash21(seed + vec2<f32>(1.0, 0.0)) * 0.08;
        let wobble = sin(t * (0.4 + hash21(seed + vec2<f32>(2.0, 0.0)) * 0.6) + i) * 0.04;
        let px = base_x + wobble;
        let py = fract(hash21(seed + vec2<f32>(3.0, 0.0)) + t * drift_speed) * 2.8 - 1.4;
        let p = vec2<f32>(px, py);

        let size = 0.003 + hash21(seed + vec2<f32>(4.0, 0.0)) * 0.005;
        let brightness = (0.3 + uniforms.audio_loudness * 0.7) * rx;
        let falloff = step(length(uv - p), size);

        // tint by frequency zone hash
        let zone = hash21(seed + vec2<f32>(5.0, 0.0));
        var ptint = vec3<f32>(0.22, 0.78, 0.96);
        if zone < 0.2 {
            ptint = vec3<f32>(1.00, 0.52, 0.20);
        } else if zone < 0.4 {
            ptint = vec3<f32>(0.20, 0.82, 0.44);
        } else if zone < 0.6 {
            ptint = vec3<f32>(0.86, 0.36, 1.00);
        } else if zone < 0.8 {
            ptint = vec3<f32>(0.98, 0.36, 0.70);
        }

        color += ptint * falloff * brightness * 0.15;
    }

    // burst on beat (aggressive)
    if rx > 0.8 {
        let burst_count = 12.0;
        for (var j: f32 = 0.0; j < burst_count; j = j + 1.0) {
            let ba = j / burst_count * PI * 2.0;
            let br = uniforms.audio_beat * 0.3;
            let bp = vec2<f32>(sin(ba), cos(ba)) * br;
            color += vec3<f32>(0.90, 0.60, 0.20) * step(length(uv - bp), 0.016) * uniforms.audio_beat * 0.08;
        }
    }

    return color;
}

fn progress_bar(uv: vec2<f32>) -> vec3<f32> {
    if uniforms.loading_active < 0.5 {
        return vec3<f32>(0.0);
    }

    let aspect = uniforms.resolution.x / uniforms.resolution.y;
    let frame_center = vec2<f32>(0.0, 0.92);
    let frame_half = vec2<f32>(aspect - 0.08, 0.045);
    let lane_half = frame_half - vec2<f32>(0.08, 0.016);
    let frame = rounded_box(uv, frame_center, frame_half, 0.018);
    let lane = rounded_box(uv, frame_center, lane_half, 0.012);
    let progress = clamp(uniforms.loading_progress, 0.0, 1.0);
    let fill_half_x = lane_half.x * progress;
    let fill_center_x = frame_center.x - lane_half.x + fill_half_x;
    let chrome = holographic_panel(uv, frame_center, frame_half, vec3<f32>(0.34, 0.36, 0.40));

    var color = vec3<f32>(0.0);
    color += vec3<f32>(0.028, 0.029, 0.032) * frame;
    color += vec3<f32>(0.016, 0.017, 0.020) * lane;
    if fill_half_x > 0.0005 {
        let fill = rounded_box(uv, vec2<f32>(fill_center_x, frame_center.y), vec2<f32>(fill_half_x, lane_half.y), 0.012);
        color += vec3<f32>(0.62, 0.66, 0.72) * fill;
    }
    color += chrome;
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
    let line = step(abs(uv.x - x), 0.003) * panel_gate;
    return vec3<f32>(0.94, 0.95, 1.00) * line * 0.18;
}

fn metric_row(uv: vec2<f32>, y: f32, value: f32, color: vec3<f32>) -> vec3<f32> {
    let frame = rounded_box(uv, vec2<f32>(0.06, y), vec2<f32>(0.68, 0.060), 0.016);
    let lane = rounded_box(uv, vec2<f32>(0.08, y), vec2<f32>(0.60, 0.026), 0.012);
    let clamped_value = clamp(value, 0.0, 1.0);
    let fill_width = 0.60 * clamped_value;
    let fill_center_x = -0.52 + fill_width;
    let fill = rounded_box(uv, vec2<f32>(fill_center_x, y), vec2<f32>(fill_width, 0.026), 0.010);
    let marker = rounded_box(uv, vec2<f32>(-0.72, y), vec2<f32>(0.034, 0.034), 0.010);
    let edge = max(frame - rounded_box(uv, vec2<f32>(0.06, y), vec2<f32>(0.66, 0.044), 0.012), 0.0);
    var row = vec3<f32>(0.0);
    row += vec3<f32>(0.05, 0.07, 0.10) * frame;
    row += vec3<f32>(0.02, 0.035, 0.055) * lane;
    row += color * fill;
    row += color * marker * 0.90;
    row += color * edge * 0.22;
    return row;
}

fn status_badge(uv: vec2<f32>) -> vec3<f32> {
    let alpha = uniforms.status_color.w;
    if alpha <= 0.001 {
        return vec3<f32>(0.0);
    }

    let aspect = uniforms.resolution.x / uniforms.resolution.y;
    let badge_center = vec2<f32>(-aspect + 0.20, 0.92);
    let badge_half = vec2<f32>(0.14, 0.05);
    let badge = rounded_box(uv, badge_center, badge_half, 0.016);
    let color = uniforms.status_color.xyz;

    var indicator = vec3<f32>(0.05, 0.06, 0.08) * badge;
    indicator += color * badge * (0.24 + alpha * 0.45);
    indicator += holographic_panel(uv, badge_center, badge_half, color);
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
        case 70u: {
            switch row {
                case 0u: { return 31u; }
                case 1u: { return 16u; }
                case 2u: { return 16u; }
                case 3u: { return 30u; }
                case 4u: { return 16u; }
                case 5u: { return 16u; }
                case 6u: { return 16u; }
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
        case 74u: {
            switch row {
                case 0u: { return 7u; }
                case 1u: { return 2u; }
                case 2u: { return 2u; }
                case 3u: { return 2u; }
                case 4u: { return 2u; }
                case 5u: { return 18u; }
                case 6u: { return 12u; }
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

fn glyph_sample_filtered(ch: u32, rel: vec2<f32>) -> f32 {
    let dx = 0.42 / 5.0;
    let dy = 0.42 / 7.0;
    let coverage = glyph_sample(ch, rel) * 0.40
        + glyph_sample(ch, rel + vec2<f32>(-dx, -dy)) * 0.15
        + glyph_sample(ch, rel + vec2<f32>(dx, -dy)) * 0.15
        + glyph_sample(ch, rel + vec2<f32>(-dx, dy)) * 0.15
        + glyph_sample(ch, rel + vec2<f32>(dx, dy)) * 0.15;
    return smoothstep(0.18, 0.78, coverage);
}

fn draw_char(uv: vec2<f32>, top_left: vec2<f32>, char_size: vec2<f32>, ch: u32, tint: vec3<f32>) -> vec3<f32> {
    let rel = vec2<f32>(
        (uv.x - top_left.x) / char_size.x,
        (uv.y - top_left.y) / char_size.y,
    );
    let mask = glyph_sample_filtered(ch, rel);
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
    let panel_half = vec2<f32>(0.34, 0.125);
    let panel = rounded_box(uv, vec2<f32>(0.0, 0.0), panel_half, 0.012);
    let focus = u32(round(uniforms.active_tuning_focus));
    let is_hz = uniforms.active_tuning_is_hz > 0.5;
    let history_count = visible_graph_history_count();
    let history_t = f32(history_count - MIN_GRAPH_HISTORY_POINTS) / f32(GRAPH_HISTORY_LENGTH - MIN_GRAPH_HISTORY_POINTS);
    var accent = vec3<f32>(0.96, 0.72, 0.24);
    if is_hz {
        accent = vec3<f32>(0.26, 0.74, 0.98);
    }

    var color = vec3<f32>(0.04, 0.06, 0.09) * panel;
    color += holographic_panel(uv, vec2<f32>(0.0, 0.0), panel_half, accent);
    color += accent * rounded_box(uv, vec2<f32>(-0.29, -0.01), vec2<f32>(0.012, 0.096), 0.006) * 0.85;
    color += draw_text_line(uv, vec2<f32>(-0.24, -0.095), vec2<f32>(0.020, 0.034), 12u, 0u, focus, 0.0, false, vec3<f32>(0.86, 0.90, 0.95));
    color += draw_text_line(uv, vec2<f32>(-0.24, -0.045), vec2<f32>(0.022, 0.036), 8u, 1u, focus, uniforms.active_tuning_value, is_hz, accent);
    color += draw_text_line(uv, vec2<f32>(-0.24, 0.000), vec2<f32>(0.018, 0.030), 12u, 2u, focus, uniforms.active_tuning_step, is_hz, vec3<f32>(0.72, 0.78, 0.86));

    let slider_label_size = vec2<f32>(0.016, 0.028);
    let slider_label_gap = slider_label_size.x * 1.18;
    let slider_label_origin = vec2<f32>(-0.24, 0.040);
    color += draw_char(uv, slider_label_origin + vec2<f32>(slider_label_gap * 0.0, 0.0), slider_label_size, 72u, vec3<f32>(0.82, 0.86, 0.94));
    color += draw_char(uv, slider_label_origin + vec2<f32>(slider_label_gap * 1.0, 0.0), slider_label_size, 73u, vec3<f32>(0.82, 0.86, 0.94));
    color += draw_char(uv, slider_label_origin + vec2<f32>(slider_label_gap * 2.0, 0.0), slider_label_size, 83u, vec3<f32>(0.82, 0.86, 0.94));
    color += draw_char(uv, slider_label_origin + vec2<f32>(slider_label_gap * 3.0, 0.0), slider_label_size, 84u, vec3<f32>(0.82, 0.86, 0.94));

    let count_size = vec2<f32>(0.016, 0.028);
    let count_gap = count_size.x * 1.16;
    let count_origin = vec2<f32>(0.17, 0.040);
    var count_hundreds = 32u;
    if history_count >= 100u {
        count_hundreds = ascii_digit(history_count / 100u);
    }
    var count_tens = 32u;
    if history_count >= 10u {
        count_tens = ascii_digit((history_count / 10u) % 10u);
    }
    color += draw_char(uv, count_origin + vec2<f32>(count_gap * 0.0, 0.0), count_size, count_hundreds, accent);
    color += draw_char(uv, count_origin + vec2<f32>(count_gap * 1.0, 0.0), count_size, count_tens, accent);
    color += draw_char(uv, count_origin + vec2<f32>(count_gap * 2.0, 0.0), count_size, ascii_digit(history_count % 10u), accent);

    let slider_center = vec2<f32>(0.038, 0.052);
    let slider_half = vec2<f32>(0.230, 0.012);
    let slider_lane = rounded_box(uv, slider_center, slider_half, 0.006);
    let slider_fill_half = vec2<f32>(slider_half.x * history_t, slider_half.y);
    let slider_fill_center = vec2<f32>(slider_center.x - slider_half.x + slider_fill_half.x, slider_center.y);
    let slider_fill = rounded_box(uv, slider_fill_center, slider_fill_half, 0.006);
    let knob_x = mix(slider_center.x - slider_half.x, slider_center.x + slider_half.x, history_t);
    let knob = rounded_box(uv, vec2<f32>(knob_x, slider_center.y), vec2<f32>(0.014, 0.028), 0.006);
    color += vec3<f32>(0.06, 0.08, 0.12) * slider_lane;
    color += accent * slider_fill * 0.85;
    color += vec3<f32>(0.90, 0.94, 1.0) * knob * 0.85;
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
    var color = draw_char(uv, origin, vec2<f32>(0.020, 0.034), axis_char, accent);
    color += draw_char(uv, origin + vec2<f32>(0.026, 0.0), vec2<f32>(0.018, 0.030), 45u, accent);

    let text_origin = origin + vec2<f32>(0.052, 0.0);
    let spacing = 0.018 * 0.18;
    for (var index: u32 = 0u; index < axis_source_length(source); index = index + 1u) {
        let ch = axis_source_char(source, index);
        if ch != 0u {
            color += draw_char(
                uv,
                text_origin + vec2<f32>(f32(index) * (0.018 + spacing), 0.0),
                vec2<f32>(0.018, 0.030),
                ch,
                vec3<f32>(0.86, 0.90, 0.95),
            );
        }
    }

    if is_active {
        color += accent * rounded_box(uv, origin + vec2<f32>(0.16, 0.018), vec2<f32>(0.16, 0.020), 0.008) * 0.22;
    }
    return color;
}

fn axis_selector_hud(uv: vec2<f32>) -> vec3<f32> {
    let panel_half = vec2<f32>(0.32, 0.13);
    let panel = rounded_box(uv, vec2<f32>(0.0, 0.0), panel_half, 0.012);
    var color = vec3<f32>(0.04, 0.06, 0.09) * panel;
    color += holographic_panel(uv, vec2<f32>(0.0, 0.0), panel_half, vec3<f32>(0.22, 0.74, 0.98));
    let active_focus = u32(round(uniforms.active_axis_focus));
    color += draw_axis_selector_line(uv, vec2<f32>(-0.28, -0.096), 88u, u32(round(uniforms.axis_x_source)), active_focus == 0u);
    color += draw_axis_selector_line(uv, vec2<f32>(-0.28, -0.040), 89u, u32(round(uniforms.axis_y_source)), active_focus == 1u);
    color += draw_axis_selector_line(uv, vec2<f32>(-0.28, 0.016), 90u, u32(round(uniforms.axis_z_source)), active_focus == 2u);
    color += draw_axis_selector_line(uv, vec2<f32>(-0.28, 0.072), 83u, u32(round(uniforms.axis_size_source)), active_focus == 3u);
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
    var color = vec3<f32>(0.04, 0.055, 0.082) * panel;
    color += holographic_panel(uv, vec2<f32>(0.0, -0.79), vec2<f32>(0.80, 0.16), vec3<f32>(0.28, 0.76, 0.96));

    let base_y = -0.92;
    let max_height = 0.26;
    for (var index: u32 = 0u; index < LOG_BAND_COUNT; index = index + 1u) {
        let x = -0.73 + f32(index) * 0.064;
        let value = clamp(log_bin_value(index), 0.0, 1.0);
        let height = 0.012 + value * max_height;
        let column = rounded_box(uv, vec2<f32>(x, base_y + height), vec2<f32>(0.020, height), 0.010);
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

fn visible_graph_history_count() -> u32 {
    return min(max(u32(round(uniforms.graph_history_count)), MIN_GRAPH_HISTORY_POINTS), GRAPH_HISTORY_LENGTH);
}

fn visible_graph_history_start() -> u32 {
    return GRAPH_HISTORY_LENGTH - visible_graph_history_count();
}

fn visible_graph_history_t(index: u32) -> f32 {
    let count = visible_graph_history_count();
    let start = visible_graph_history_start();
    if count <= 1u {
        return 1.0;
    }
    return f32(index - start) / f32(count - 1u);
}

fn graph_edge(uv: vec2<f32>, start: vec3<f32>, end: vec3<f32>, tint: vec3<f32>) -> vec3<f32> {
    let a = project_graph_point(start);
    let b = project_graph_point(end);
    let line = step(distance_to_segment(uv, a, b), 0.0035);
    return tint * line * 0.08;
}

fn graph_wireframe(uv: vec2<f32>) -> vec3<f32> {
    let tint = vec3<f32>(0.28, 0.30, 0.34);
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
    let rx = reactivity_scale();
    var color = vec3<f32>(0.0);
    let start = visible_graph_history_start();
    for (var index: u32 = start + 1u; index < GRAPH_HISTORY_LENGTH; index = index + 1u) {
        let previous = graph_point(index - 1u);
        let current = graph_point(index);
        if previous.w < 0.0 || current.w < 0.0 {
            continue;
        }

        let age = visible_graph_history_t(index);
        let size = clamp(current.w, 0.0, 1.0);
        let projected_previous = project_graph_point(previous.xyz);
        let projected_current = project_graph_point(current.xyz);
        let distance = distance_to_segment(uv, projected_previous, projected_current);
        let line_width = 1.0 / (mix(560.0, 420.0, size) * mix(1.0, 0.82, age));
        let line = step(distance, line_width);
        let tint = mix(vec3<f32>(0.46, 0.47, 0.50), vec3<f32>(0.70, 0.72, 0.76), age * 0.55 + rx * 0.08);
        color += tint * line * (0.18 + age * 0.16);
        let dot_radius = 1.0 / mix(170.0, 95.0, size);
        color += vec3<f32>(0.82, 0.84, 0.88) * step(length(uv - projected_current), dot_radius) * (0.05 + size * 0.09);
    }
    return color;
}

fn graph_panel(uv: vec2<f32>) -> vec3<f32> {
    let panel = rounded_box(uv, vec2<f32>(0.0, 0.0), vec2<f32>(0.98, 0.98), 0.000);
    var color = vec3<f32>(0.022, 0.023, 0.028) * panel;
    color += holographic_panel(uv, vec2<f32>(0.0, 0.0), vec2<f32>(0.98, 0.98), vec3<f32>(0.34, 0.36, 0.40));
    let grid_pulse = 0.5 + 0.5 * sin(uniforms.time * 0.3);
    color += graph_wireframe(uv) * (0.52 + grid_pulse * 0.08);
    color += graph_trace(uv);
    color += graph_axis_labels(uv);
    return color * panel;
}

fn graph_history_component(point: vec4<f32>, component: u32) -> f32 {
    switch component {
        case 0u: { return point.x; }
        case 1u: { return point.y; }
        case 2u: { return point.z; }
        default: { return point.w; }
    }
}

fn history_source(component: u32) -> u32 {
    switch component {
        case 0u: { return u32(round(uniforms.axis_x_source)); }
        case 1u: { return u32(round(uniforms.axis_y_source)); }
        case 2u: { return u32(round(uniforms.axis_z_source)); }
        default: { return u32(round(uniforms.axis_size_source)); }
    }
}

fn layout_offset(offset: vec2<f32>) -> vec2<f32> {
    let aspect = uniforms.resolution.x / uniforms.resolution.y;
    return vec2<f32>(offset.x * aspect, offset.y);
}

fn history_chart_panel(uv: vec2<f32>, component: u32, label_char: u32, tint: vec3<f32>) -> vec3<f32> {
    let panel = rounded_box(uv, vec2<f32>(0.0, 0.0), vec2<f32>(0.98, 0.98), 0.000);
    if panel < 0.001 {
        return vec3<f32>(0.0);
    }

    var color = vec3<f32>(0.025, 0.038, 0.064) * panel;
    color += holographic_panel(uv, vec2<f32>(0.0, 0.0), vec2<f32>(0.98, 0.98), tint);

    let grid_a = step(abs(uv.y + 0.45), 0.010);
    let grid_b = step(abs(uv.y), 0.010);
    let grid_c = step(abs(uv.y - 0.45), 0.010);
    let guide_color = tint * 0.10 + vec3<f32>(0.04, 0.05, 0.07);
    color += guide_color * (grid_a + grid_b + grid_c);

    let title_y = -0.82;
    color += draw_char(uv, vec2<f32>(-0.88, title_y), vec2<f32>(0.060, 0.26), label_char, tint);
    color += draw_char(uv, vec2<f32>(-0.79, title_y + 0.01), vec2<f32>(0.040, 0.16), 45u, tint);
    let source = history_source(component);
    let source_size = vec2<f32>(0.035, 0.150);
    let source_gap = source_size.x * 1.30;
    let source_origin = vec2<f32>(-0.72, title_y + 0.01);
    for (var index: u32 = 0u; index < axis_source_length(source); index = index + 1u) {
        let ch = axis_source_char(source, index);
        if ch != 0u {
            color += draw_char(uv, source_origin + vec2<f32>(f32(index) * source_gap, 0.0), source_size, ch, vec3<f32>(0.86, 0.90, 0.95));
        }
    }

    let left_x = -0.88;
    let right_x = 0.90;
    let top_y = -0.46;
    let bottom_y = 0.42;
    let axis_y = 0.64;
    let start = visible_graph_history_start();
    let latest_index = GRAPH_HISTORY_LENGTH - 1u;
    var latest_dot = 0.0;

    let axis_line = step(abs(uv.y - axis_y), 0.012) * step(left_x, uv.x) * step(uv.x, right_x);
    color += (tint * 0.28 + vec3<f32>(0.12, 0.14, 0.18)) * axis_line;
    for (var tick: u32 = 0u; tick <= 4u; tick = tick + 1u) {
        let tick_t = f32(tick) / 4.0;
        let tick_x = mix(left_x, right_x, tick_t);
        let tick_mark = step(abs(uv.x - tick_x), 0.010) * step(abs(uv.y - axis_y), 0.050);
        color += tint * tick_mark * 0.24;
    }

    let time_sz = vec2<f32>(0.040, 0.170);
    let time_gap = time_sz.x * 1.30;
    let time_x = 0.08;
    let time_y = 0.74;
    color += draw_char(uv, vec2<f32>(time_x - time_gap * 2.0, time_y), time_sz, 84u, tint);
    color += draw_char(uv, vec2<f32>(time_x - time_gap * 1.0, time_y), time_sz, 73u, tint);
    color += draw_char(uv, vec2<f32>(time_x, time_y), time_sz, 77u, tint);
    color += draw_char(uv, vec2<f32>(time_x + time_gap * 1.0, time_y), time_sz, 69u, tint);

    for (var index: u32 = start + 1u; index < GRAPH_HISTORY_LENGTH; index = index + 1u) {
        let previous = graph_point(index - 1u);
        let current = graph_point(index);
        if previous.w < 0.0 || current.w < 0.0 {
            continue;
        }

        let prev_value = clamp(graph_history_component(previous, component), 0.0, 1.0);
        let curr_value = clamp(graph_history_component(current, component), 0.0, 1.0);
        let prev_t = visible_graph_history_t(index - 1u);
        let curr_t = visible_graph_history_t(index);
        let a = vec2<f32>(mix(left_x, right_x, prev_t), mix(bottom_y, top_y, prev_value));
        let b = vec2<f32>(mix(left_x, right_x, curr_t), mix(bottom_y, top_y, curr_value));
        let distance = distance_to_segment(uv, a, b);
        let age = curr_t;
        let line = step(distance, 0.016);
        color += tint * line * (0.20 + age * 0.55);

        if index == latest_index {
            latest_dot = step(length(uv - b), 0.040);
        }
    }

    color += tint * latest_dot * 0.95;
    return color * panel;
}

fn arc_label_char(label_id: u32, index: u32) -> u32 {
    // 0=SUB B, 1=BASS, 2=MID, 3=TREBLE, 4=PRES, 5=AIR, 6=LOUD, 7=PEAK, 8=BEAT, 9=CENT
    switch label_id {
        case 0u: {
            switch index { case 0u: { return 83u; } case 1u: { return 85u; } case 2u: { return 66u; } case 3u: { return 32u; } case 4u: { return 66u; } default: { return 0u; } }
        }
        case 1u: {
            switch index { case 0u: { return 66u; } case 1u: { return 65u; } case 2u: { return 83u; } case 3u: { return 83u; } default: { return 0u; } }
        }
        case 2u: {
            switch index { case 0u: { return 77u; } case 1u: { return 73u; } case 2u: { return 68u; } default: { return 0u; } }
        }
        case 3u: {
            switch index { case 0u: { return 84u; } case 1u: { return 82u; } case 2u: { return 69u; } case 3u: { return 66u; } default: { return 0u; } }
        }
        case 4u: {
            switch index { case 0u: { return 80u; } case 1u: { return 82u; } case 2u: { return 69u; } case 3u: { return 83u; } default: { return 0u; } }
        }
        case 5u: {
            switch index { case 0u: { return 65u; } case 1u: { return 73u; } case 2u: { return 82u; } default: { return 0u; } }
        }
        case 6u: {
            switch index { case 0u: { return 76u; } case 1u: { return 79u; } case 2u: { return 85u; } case 3u: { return 68u; } default: { return 0u; } }
        }
        case 7u: {
            switch index { case 0u: { return 80u; } case 1u: { return 69u; } case 2u: { return 65u; } case 3u: { return 75u; } default: { return 0u; } }
        }
        case 8u: {
            switch index { case 0u: { return 66u; } case 1u: { return 69u; } case 2u: { return 65u; } case 3u: { return 84u; } default: { return 0u; } }
        }
        case 9u: {
            switch index { case 0u: { return 67u; } case 1u: { return 69u; } case 2u: { return 78u; } case 3u: { return 84u; } default: { return 0u; } }
        }
        default: { return 0u; }
    }
}

fn arc_label_len(label_id: u32) -> u32 {
    switch label_id {
        case 0u: { return 5u; }
        case 2u: { return 3u; }
        case 5u: { return 3u; }
        default: { return 4u; }
    }
}

fn draw_arc_label(uv: vec2<f32>, center: vec2<f32>, label_id: u32, tint: vec3<f32>, panel_half: vec2<f32>) -> vec3<f32> {
    // Scale label size so it matches help panel text (0.020, 0.034) in screen UV
    let sz = vec2<f32>(0.020 / panel_half.x, 0.034 / panel_half.y);
    let g = sz.x * 1.16;
    let len = arc_label_len(label_id);
    // early-out: skip if pixel is outside the label bounding box
    let total_w = f32(len) * g;
    let start_x = center.x - total_w * 0.5;
    if uv.x < start_x - sz.x || uv.x > start_x + total_w + sz.x || abs(uv.y - center.y) > sz.y * 1.5 {
        return vec3<f32>(0.0);
    }
    var color = vec3<f32>(0.0);
    for (var i: u32 = 0u; i < len; i = i + 1u) {
        let ch = arc_label_char(label_id, i);
        if ch != 0u {
            color += draw_char(uv, vec2<f32>(start_x + f32(i) * g, center.y), sz, ch, tint);
        }
    }
    return color;
}

fn left_arc_gauges(uv: vec2<f32>, panel_half: vec2<f32>) -> vec3<f32> {
    let panel_asp = panel_half.x / panel_half.y;
    let r = 0.13;
    let spacing = 0.38;
    let x = 0.0;
    var color = vec3<f32>(0.0);
    color += arc_gauge(uv, vec2<f32>(x, 0.74), r, uniforms.audio_sub_bass, uniforms.peak_sub_bass, vec3<f32>(1.00, 0.52, 0.20), panel_asp);
    color += draw_arc_label(uv, vec2<f32>(x, 0.74 - r - 0.08), 0u, vec3<f32>(0.80, 0.50, 0.20), panel_half);
    color += arc_gauge(uv, vec2<f32>(x, 0.74 - spacing), r, uniforms.audio_bass, uniforms.peak_bass, vec3<f32>(0.92, 0.38, 0.18), panel_asp);
    color += draw_arc_label(uv, vec2<f32>(x, 0.74 - spacing - r - 0.08), 1u, vec3<f32>(0.72, 0.38, 0.18), panel_half);
    color += arc_gauge(uv, vec2<f32>(x, 0.74 - spacing * 2.0), r, uniforms.audio_mid, uniforms.peak_mid, vec3<f32>(0.20, 0.82, 0.44), panel_asp);
    color += draw_arc_label(uv, vec2<f32>(x, 0.74 - spacing * 2.0 - r - 0.08), 2u, vec3<f32>(0.20, 0.62, 0.34), panel_half);
    color += arc_gauge(uv, vec2<f32>(x, 0.74 - spacing * 3.0), r, uniforms.audio_treble, uniforms.peak_treble, vec3<f32>(0.20, 0.58, 0.98), panel_asp);
    color += draw_arc_label(uv, vec2<f32>(x, 0.74 - spacing * 3.0 - r - 0.08), 3u, vec3<f32>(0.20, 0.48, 0.78), panel_half);
    color += arc_gauge(uv, vec2<f32>(x, 0.74 - spacing * 4.0), r, uniforms.audio_presence, uniforms.peak_presence, vec3<f32>(0.98, 0.36, 0.70), panel_asp);
    color += draw_arc_label(uv, vec2<f32>(x, 0.74 - spacing * 4.0 - r - 0.08), 4u, vec3<f32>(0.78, 0.36, 0.56), panel_half);
    return color;
}

fn right_arc_gauges(uv: vec2<f32>, panel_half: vec2<f32>) -> vec3<f32> {
    let panel_asp = panel_half.x / panel_half.y;
    let r = 0.13;
    let spacing = 0.38;
    let x = 0.0;
    var color = vec3<f32>(0.0);
    color += arc_gauge(uv, vec2<f32>(x, 0.74), r, uniforms.audio_air, uniforms.peak_air, vec3<f32>(0.66, 0.84, 1.00), panel_asp);
    color += draw_arc_label(uv, vec2<f32>(x, 0.74 - r - 0.08), 5u, vec3<f32>(0.52, 0.64, 0.78), panel_half);
    color += arc_gauge(uv, vec2<f32>(x, 0.74 - spacing), r, uniforms.audio_loudness, uniforms.peak_loudness, vec3<f32>(0.95, 0.78, 0.22), panel_asp);
    color += draw_arc_label(uv, vec2<f32>(x, 0.74 - spacing - r - 0.08), 6u, vec3<f32>(0.72, 0.60, 0.20), panel_half);
    color += arc_gauge(uv, vec2<f32>(x, 0.74 - spacing * 2.0), r, uniforms.audio_peak, uniforms.peak_peak, vec3<f32>(0.98, 0.48, 0.62), panel_asp);
    color += draw_arc_label(uv, vec2<f32>(x, 0.74 - spacing * 2.0 - r - 0.08), 7u, vec3<f32>(0.78, 0.40, 0.50), panel_half);
    color += arc_gauge(uv, vec2<f32>(x, 0.74 - spacing * 3.0), r, uniforms.audio_beat, uniforms.peak_beat, vec3<f32>(0.86, 0.36, 1.00), panel_asp);
    color += draw_arc_label(uv, vec2<f32>(x, 0.74 - spacing * 3.0 - r - 0.08), 8u, vec3<f32>(0.66, 0.30, 0.78), panel_half);
    color += arc_gauge(uv, vec2<f32>(x, 0.74 - spacing * 4.0), r, uniforms.audio_centroid, uniforms.peak_centroid, vec3<f32>(0.24, 0.92, 0.94), panel_asp);
    color += draw_arc_label(uv, vec2<f32>(x, 0.74 - spacing * 4.0 - r - 0.08), 9u, vec3<f32>(0.24, 0.70, 0.72), panel_half);
    return color;
}

fn spectrum_strip(uv: vec2<f32>) -> vec3<f32> {
    let rx = reactivity_scale();
    let strip_center = vec2<f32>(0.0, 0.0);
    let strip_half = vec2<f32>(0.96, 0.86);
    let panel = rounded_box(uv, strip_center, strip_half, 0.016);
    var color = vec3<f32>(0.025, 0.040, 0.065) * panel;
    color += holographic_panel(uv, strip_center, strip_half, vec3<f32>(0.28, 0.76, 0.96));

    let base_y = -0.80;
    let max_height = 0.76;
    let bin_spacing = 0.078;
    let bin_width = 0.026;
    for (var index: u32 = 0u; index < LOG_BAND_COUNT; index = index + 1u) {
        let x = -0.88 + f32(index) * bin_spacing;
        let value = clamp(log_bin_value(index), 0.0, 1.0);
        let height = 0.015 + value * max_height;
        let column = rounded_box(uv, vec2<f32>(x, base_y + height), vec2<f32>(bin_width, height), 0.008);
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
    }

    // separator lines
    let sep_center = vec2<f32>(0.0, 0.0);
    let sep_gate = panel;
    color += vec3<f32>(0.94, 0.95, 1.00) * step(abs(uv.x - frequency_to_column_x_wide(uniforms.split_sub_bass_max_hz)), 0.003) * sep_gate * 0.15;
    color += vec3<f32>(0.94, 0.95, 1.00) * step(abs(uv.x - frequency_to_column_x_wide(uniforms.split_bass_max_hz)), 0.003) * sep_gate * 0.15;
    color += vec3<f32>(0.94, 0.95, 1.00) * step(abs(uv.x - frequency_to_column_x_wide(uniforms.split_mid_max_hz)), 0.003) * sep_gate * 0.15;
    color += vec3<f32>(0.94, 0.95, 1.00) * step(abs(uv.x - frequency_to_column_x_wide(uniforms.split_presence_min_hz)), 0.003) * sep_gate * 0.15;
    color += vec3<f32>(0.94, 0.95, 1.00) * step(abs(uv.x - frequency_to_column_x_wide(uniforms.split_presence_max_hz)), 0.003) * sep_gate * 0.15;
    color += vec3<f32>(0.94, 0.95, 1.00) * step(abs(uv.x - frequency_to_column_x_wide(uniforms.split_air_min_hz)), 0.003) * sep_gate * 0.15;

    return color * panel;
}

fn frequency_to_column_x_wide(frequency_hz: f32) -> f32 {
    let nyquist = max(uniforms.analysis_nyquist_hz, MIN_ANALYSIS_FREQUENCY_HZ * 2.0);
    let clamped = clamp(frequency_hz, MIN_ANALYSIS_FREQUENCY_HZ, nyquist);
    let ratio = max(nyquist / MIN_ANALYSIS_FREQUENCY_HZ, 1.0 + 1e-4);
    let t = log(clamped / MIN_ANALYSIS_FREQUENCY_HZ) / log(ratio);
    return -0.88 + t * 0.078 * f32(LOG_BAND_COUNT - 1u);
}

fn top_bar(uv: vec2<f32>) -> vec3<f32> {
    var color = vec3<f32>(0.0);
    // tuning HUD (left portion)
    let tuning_uv = uv - vec2<f32>(-0.52, 0.0);
    color += tuning_hud(tuning_uv);
    // axis selector (right portion)
    let axis_uv = uv - vec2<f32>(0.24, 0.0);
    color += axis_selector_hud(axis_uv);
    // progress bar (center)
    color += progress_bar(uv);
    // status badge
    color += status_badge(uv);
    return color;
}

fn help_panel(uv: vec2<f32>) -> vec3<f32> {
    let ctr = vec2<f32>(0.0, 0.0);
    let hlf = vec2<f32>(0.42, 0.52);
    let bg = rounded_box(uv, ctr, hlf, 0.024);
    if bg < 0.001 { return vec3<f32>(0.0); }

    var c = vec3<f32>(0.04, 0.06, 0.10) * bg;
    c += holographic_panel(uv, ctr, hlf, vec3<f32>(0.22, 0.66, 0.96));

    let sz = vec2<f32>(0.018, 0.031);
    let g = sz.x * 1.16;
    let dy = 0.052;
    let kc = vec3<f32>(0.98, 0.78, 0.28);
    let dc = vec3<f32>(0.82, 0.86, 0.94);
    let tc = vec3<f32>(0.40, 0.88, 0.98);
    let kx = -0.33;
    let dx = -0.16;

    // Title: SHORTCUTS
    let tsz = sz * 1.25;
    let tg = tsz.x * 1.16;
    let ty = -0.43;
    let tx = -0.14;
    c += draw_char(uv, vec2<f32>(tx + tg * 0.0, ty), tsz, 83u, tc);
    c += draw_char(uv, vec2<f32>(tx + tg * 1.0, ty), tsz, 72u, tc);
    c += draw_char(uv, vec2<f32>(tx + tg * 2.0, ty), tsz, 79u, tc);
    c += draw_char(uv, vec2<f32>(tx + tg * 3.0, ty), tsz, 82u, tc);
    c += draw_char(uv, vec2<f32>(tx + tg * 4.0, ty), tsz, 84u, tc);
    c += draw_char(uv, vec2<f32>(tx + tg * 5.0, ty), tsz, 67u, tc);
    c += draw_char(uv, vec2<f32>(tx + tg * 6.0, ty), tsz, 85u, tc);
    c += draw_char(uv, vec2<f32>(tx + tg * 7.0, ty), tsz, 84u, tc);
    c += draw_char(uv, vec2<f32>(tx + tg * 8.0, ty), tsz, 83u, tc);

    // H → HELP
    let y0 = -0.31;
    c += draw_char(uv, vec2<f32>(kx, y0), sz, 72u, kc);
    c += draw_char(uv, vec2<f32>(dx + g * 0.0, y0), sz, 72u, dc);
    c += draw_char(uv, vec2<f32>(dx + g * 1.0, y0), sz, 69u, dc);
    c += draw_char(uv, vec2<f32>(dx + g * 2.0, y0), sz, 76u, dc);
    c += draw_char(uv, vec2<f32>(dx + g * 3.0, y0), sz, 80u, dc);

    // R → RX MODE
    let y1 = y0 + dy;
    c += draw_char(uv, vec2<f32>(kx, y1), sz, 82u, kc);
    c += draw_char(uv, vec2<f32>(dx + g * 0.0, y1), sz, 82u, dc);
    c += draw_char(uv, vec2<f32>(dx + g * 1.0, y1), sz, 88u, dc);
    c += draw_char(uv, vec2<f32>(dx + g * 2.0, y1), sz, 32u, dc);
    c += draw_char(uv, vec2<f32>(dx + g * 3.0, y1), sz, 77u, dc);
    c += draw_char(uv, vec2<f32>(dx + g * 4.0, y1), sz, 79u, dc);
    c += draw_char(uv, vec2<f32>(dx + g * 5.0, y1), sz, 68u, dc);
    c += draw_char(uv, vec2<f32>(dx + g * 6.0, y1), sz, 69u, dc);

    // 1 → BEAT MODE
    let y2 = y0 + dy * 2.0;
    c += draw_char(uv, vec2<f32>(kx, y2), sz, 49u, kc);
    c += draw_char(uv, vec2<f32>(dx + g * 0.0, y2), sz, 66u, dc);
    c += draw_char(uv, vec2<f32>(dx + g * 1.0, y2), sz, 69u, dc);
    c += draw_char(uv, vec2<f32>(dx + g * 2.0, y2), sz, 65u, dc);
    c += draw_char(uv, vec2<f32>(dx + g * 3.0, y2), sz, 84u, dc);
    c += draw_char(uv, vec2<f32>(dx + g * 4.0, y2), sz, 32u, dc);
    c += draw_char(uv, vec2<f32>(dx + g * 5.0, y2), sz, 77u, dc);
    c += draw_char(uv, vec2<f32>(dx + g * 6.0, y2), sz, 79u, dc);
    c += draw_char(uv, vec2<f32>(dx + g * 7.0, y2), sz, 68u, dc);
    c += draw_char(uv, vec2<f32>(dx + g * 8.0, y2), sz, 69u, dc);

    // SP → PLAY
    let y3 = y0 + dy * 3.0;
    c += draw_char(uv, vec2<f32>(kx + g * 0.0, y3), sz, 83u, kc);
    c += draw_char(uv, vec2<f32>(kx + g * 1.0, y3), sz, 80u, kc);
    c += draw_char(uv, vec2<f32>(dx + g * 0.0, y3), sz, 80u, dc);
    c += draw_char(uv, vec2<f32>(dx + g * 1.0, y3), sz, 76u, dc);
    c += draw_char(uv, vec2<f32>(dx + g * 2.0, y3), sz, 65u, dc);
    c += draw_char(uv, vec2<f32>(dx + g * 3.0, y3), sz, 89u, dc);

    // XYZS → AXIS
    let y4 = y0 + dy * 4.0;
    c += draw_char(uv, vec2<f32>(kx + g * 0.0, y4), sz, 88u, kc);
    c += draw_char(uv, vec2<f32>(kx + g * 1.0, y4), sz, 89u, kc);
    c += draw_char(uv, vec2<f32>(kx + g * 2.0, y4), sz, 90u, kc);
    c += draw_char(uv, vec2<f32>(kx + g * 3.0, y4), sz, 83u, kc);
    c += draw_char(uv, vec2<f32>(dx + g * 0.0, y4), sz, 65u, dc);
    c += draw_char(uv, vec2<f32>(dx + g * 1.0, y4), sz, 88u, dc);
    c += draw_char(uv, vec2<f32>(dx + g * 2.0, y4), sz, 73u, dc);
    c += draw_char(uv, vec2<f32>(dx + g * 3.0, y4), sz, 83u, dc);

    // ARR → MAP
    let y5 = y0 + dy * 5.0;
    c += draw_char(uv, vec2<f32>(kx + g * 0.0, y5), sz, 65u, kc);
    c += draw_char(uv, vec2<f32>(kx + g * 1.0, y5), sz, 82u, kc);
    c += draw_char(uv, vec2<f32>(kx + g * 2.0, y5), sz, 82u, kc);
    c += draw_char(uv, vec2<f32>(dx + g * 0.0, y5), sz, 77u, dc);
    c += draw_char(uv, vec2<f32>(dx + g * 1.0, y5), sz, 65u, dc);
    c += draw_char(uv, vec2<f32>(dx + g * 2.0, y5), sz, 80u, dc);

    // N → NORM
    let y6 = y0 + dy * 6.0;
    c += draw_char(uv, vec2<f32>(kx, y6), sz, 78u, kc);
    c += draw_char(uv, vec2<f32>(dx + g * 0.0, y6), sz, 78u, dc);
    c += draw_char(uv, vec2<f32>(dx + g * 1.0, y6), sz, 79u, dc);
    c += draw_char(uv, vec2<f32>(dx + g * 2.0, y6), sz, 82u, dc);
    c += draw_char(uv, vec2<f32>(dx + g * 3.0, y6), sz, 77u, dc);

    // F → FRAME
    let y7 = y0 + dy * 7.0;
    c += draw_char(uv, vec2<f32>(kx, y7), sz, 70u, kc);
    c += draw_char(uv, vec2<f32>(dx + g * 0.0, y7), sz, 70u, dc);
    c += draw_char(uv, vec2<f32>(dx + g * 1.0, y7), sz, 82u, dc);
    c += draw_char(uv, vec2<f32>(dx + g * 2.0, y7), sz, 65u, dc);
    c += draw_char(uv, vec2<f32>(dx + g * 3.0, y7), sz, 77u, dc);
    c += draw_char(uv, vec2<f32>(dx + g * 4.0, y7), sz, 69u, dc);

    // PGUP → ZM IN
    let y8 = y0 + dy * 8.0;
    c += draw_char(uv, vec2<f32>(kx + g * 0.0, y8), sz, 80u, kc);
    c += draw_char(uv, vec2<f32>(kx + g * 1.0, y8), sz, 71u, kc);
    c += draw_char(uv, vec2<f32>(kx + g * 2.0, y8), sz, 85u, kc);
    c += draw_char(uv, vec2<f32>(kx + g * 3.0, y8), sz, 80u, kc);
    c += draw_char(uv, vec2<f32>(dx + g * 0.0, y8), sz, 90u, dc);
    c += draw_char(uv, vec2<f32>(dx + g * 1.0, y8), sz, 77u, dc);
    c += draw_char(uv, vec2<f32>(dx + g * 2.0, y8), sz, 32u, dc);
    c += draw_char(uv, vec2<f32>(dx + g * 3.0, y8), sz, 73u, dc);
    c += draw_char(uv, vec2<f32>(dx + g * 4.0, y8), sz, 78u, dc);

    // PGDN → ZM OUT
    let y9 = y0 + dy * 9.0;
    c += draw_char(uv, vec2<f32>(kx + g * 0.0, y9), sz, 80u, kc);
    c += draw_char(uv, vec2<f32>(kx + g * 1.0, y9), sz, 71u, kc);
    c += draw_char(uv, vec2<f32>(kx + g * 2.0, y9), sz, 68u, kc);
    c += draw_char(uv, vec2<f32>(kx + g * 3.0, y9), sz, 78u, kc);
    c += draw_char(uv, vec2<f32>(dx + g * 0.0, y9), sz, 90u, dc);
    c += draw_char(uv, vec2<f32>(dx + g * 1.0, y9), sz, 77u, dc);
    c += draw_char(uv, vec2<f32>(dx + g * 2.0, y9), sz, 32u, dc);
    c += draw_char(uv, vec2<f32>(dx + g * 3.0, y9), sz, 79u, dc);
    c += draw_char(uv, vec2<f32>(dx + g * 4.0, y9), sz, 85u, dc);
    c += draw_char(uv, vec2<f32>(dx + g * 5.0, y9), sz, 84u, dc);

    // JLIKUO → PAN
    let y10 = y0 + dy * 10.0;
    c += draw_char(uv, vec2<f32>(kx + g * 0.0, y10), sz, 74u, kc);
    c += draw_char(uv, vec2<f32>(kx + g * 1.0, y10), sz, 76u, kc);
    c += draw_char(uv, vec2<f32>(kx + g * 2.0, y10), sz, 73u, kc);
    c += draw_char(uv, vec2<f32>(kx + g * 3.0, y10), sz, 75u, kc);
    c += draw_char(uv, vec2<f32>(kx + g * 4.0, y10), sz, 85u, kc);
    c += draw_char(uv, vec2<f32>(kx + g * 5.0, y10), sz, 79u, kc);
    c += draw_char(uv, vec2<f32>(dx + g * 0.0, y10), sz, 80u, dc);
    c += draw_char(uv, vec2<f32>(dx + g * 1.0, y10), sz, 65u, dc);
    c += draw_char(uv, vec2<f32>(dx + g * 2.0, y10), sz, 78u, dc);

    // TAB → TUNE
    let y11 = y0 + dy * 11.0;
    c += draw_char(uv, vec2<f32>(kx + g * 0.0, y11), sz, 84u, kc);
    c += draw_char(uv, vec2<f32>(kx + g * 1.0, y11), sz, 65u, kc);
    c += draw_char(uv, vec2<f32>(kx + g * 2.0, y11), sz, 66u, kc);
    c += draw_char(uv, vec2<f32>(dx + g * 0.0, y11), sz, 84u, dc);
    c += draw_char(uv, vec2<f32>(dx + g * 1.0, y11), sz, 85u, dc);
    c += draw_char(uv, vec2<f32>(dx + g * 2.0, y11), sz, 78u, dc);
    c += draw_char(uv, vec2<f32>(dx + g * 3.0, y11), sz, 69u, dc);

    // DRAG → UI
    let y12 = y0 + dy * 12.0;
    c += draw_char(uv, vec2<f32>(kx + g * 0.0, y12), sz, 68u, kc);
    c += draw_char(uv, vec2<f32>(kx + g * 1.0, y12), sz, 82u, kc);
    c += draw_char(uv, vec2<f32>(kx + g * 2.0, y12), sz, 65u, kc);
    c += draw_char(uv, vec2<f32>(kx + g * 3.0, y12), sz, 71u, kc);
    c += draw_char(uv, vec2<f32>(dx + g * 0.0, y12), sz, 85u, dc);
    c += draw_char(uv, vec2<f32>(dx + g * 1.0, y12), sz, 73u, dc);

    // 0 → RESET
    let y13 = y0 + dy * 13.0;
    c += draw_char(uv, vec2<f32>(kx, y13), sz, 48u, kc);
    c += draw_char(uv, vec2<f32>(dx + g * 0.0, y13), sz, 82u, dc);
    c += draw_char(uv, vec2<f32>(dx + g * 1.0, y13), sz, 69u, dc);
    c += draw_char(uv, vec2<f32>(dx + g * 2.0, y13), sz, 83u, dc);
    c += draw_char(uv, vec2<f32>(dx + g * 3.0, y13), sz, 69u, dc);
    c += draw_char(uv, vec2<f32>(dx + g * 4.0, y13), sz, 84u, dc);

    // 9 → UI RESET
    let y14 = y0 + dy * 14.0;
    c += draw_char(uv, vec2<f32>(kx, y14), sz, 57u, kc);
    c += draw_char(uv, vec2<f32>(dx + g * 0.0, y14), sz, 85u, dc);
    c += draw_char(uv, vec2<f32>(dx + g * 1.0, y14), sz, 73u, dc);
    c += draw_char(uv, vec2<f32>(dx + g * 2.0, y14), sz, 32u, dc);
    c += draw_char(uv, vec2<f32>(dx + g * 3.0, y14), sz, 82u, dc);
    c += draw_char(uv, vec2<f32>(dx + g * 4.0, y14), sz, 69u, dc);
    c += draw_char(uv, vec2<f32>(dx + g * 5.0, y14), sz, 83u, dc);
    c += draw_char(uv, vec2<f32>(dx + g * 6.0, y14), sz, 69u, dc);
    c += draw_char(uv, vec2<f32>(dx + g * 7.0, y14), sz, 84u, dc);

    return c;
}

fn dashboard(uv: vec2<f32>) -> vec3<f32> {
    let aspect = uniforms.resolution.x / uniforms.resolution.y;
    let margin = 0.05;

    var color = background(uv);

    let graph_half = vec2<f32>(aspect * 0.5 - margin * 1.2, 0.84);
    let graph_center = vec2<f32>(aspect - margin - graph_half.x, 0.0) + layout_offset(uniforms.graph_offset);
    let graph_mask = rounded_box(uv, graph_center, graph_half, 0.020);
    let graph_uv = (uv - graph_center) / graph_half;
    color += graph_panel(graph_uv) * graph_mask;

    let left_min = -aspect + margin;
    let left_max = graph_center.x - graph_half.x - margin;
    let left_cx = (left_min + left_max) * 0.5;
    let left_hx = (left_max - left_min) * 0.5;

    let stack_hx = 0.34;
    let stack_center_x = left_max - stack_hx;
    let stack_gap = 0.035;
    let chart_half = vec2<f32>(stack_hx, 0.080);
    let chart_step = chart_half.y * 2.0 + stack_gap;
    let tuning_pos = vec2<f32>(stack_center_x, -0.78) + layout_offset(uniforms.tuning_offset);

    let arc_region_max = stack_center_x - stack_hx - margin * 0.8;
    let arc_gap = margin * 0.24;
    let arc_hx = min(max(((arc_region_max - left_min) - arc_gap) * 0.25, 0.10), 0.16);
    let arc_hy = 0.46;
    let arc_cy = -0.32;
    let arc_panel_half = vec2<f32>(arc_hx, arc_hy);

    let arc_center_base = (left_min + arc_region_max) * 0.5;
    let left_arc_cx = arc_center_base - (arc_hx + arc_gap * 0.5);
    let left_arc_center = vec2<f32>(left_arc_cx, arc_cy) + layout_offset(uniforms.left_arc_offset);
    let left_arc_mask = rounded_box(uv, left_arc_center, arc_panel_half, 0.018);
    let left_arc_uv = (uv - left_arc_center) / arc_panel_half;
    color += left_arc_gauges(left_arc_uv, arc_panel_half) * left_arc_mask;

    let right_arc_cx = arc_center_base + (arc_hx + arc_gap * 0.5);
    let right_arc_center = vec2<f32>(right_arc_cx, arc_cy) + layout_offset(uniforms.right_arc_offset);
    let right_arc_mask = rounded_box(uv, right_arc_center, arc_panel_half, 0.018);
    let right_arc_uv = (uv - right_arc_center) / arc_panel_half;
    color += right_arc_gauges(right_arc_uv, arc_panel_half) * right_arc_mask;

    color += tuning_hud(uv - tuning_pos);

    let chart_center_0 = vec2<f32>(stack_center_x, -0.46) + layout_offset(uniforms.chart_x_offset);
    let chart_center_1 = vec2<f32>(stack_center_x, -0.46 + chart_step) + layout_offset(uniforms.chart_y_offset);
    let chart_center_2 = vec2<f32>(stack_center_x, -0.46 + chart_step * 2.0) + layout_offset(uniforms.chart_z_offset);
    let chart_center_3 = vec2<f32>(stack_center_x, -0.46 + chart_step * 3.0) + layout_offset(uniforms.chart_s_offset);

    let chart_mask_0 = rounded_box(uv, chart_center_0, chart_half, 0.018);
    let chart_uv_0 = (uv - chart_center_0) / chart_half;
    color += history_chart_panel(chart_uv_0, 0u, 88u, vec3<f32>(0.92, 0.66, 0.26)) * chart_mask_0;

    let chart_mask_1 = rounded_box(uv, chart_center_1, chart_half, 0.018);
    let chart_uv_1 = (uv - chart_center_1) / chart_half;
    color += history_chart_panel(chart_uv_1, 1u, 89u, vec3<f32>(0.40, 0.86, 0.54)) * chart_mask_1;

    let chart_mask_2 = rounded_box(uv, chart_center_2, chart_half, 0.018);
    let chart_uv_2 = (uv - chart_center_2) / chart_half;
    color += history_chart_panel(chart_uv_2, 2u, 90u, vec3<f32>(0.34, 0.72, 0.98)) * chart_mask_2;

    let chart_mask_3 = rounded_box(uv, chart_center_3, chart_half, 0.018);
    let chart_uv_3 = (uv - chart_center_3) / chart_half;
    color += history_chart_panel(chart_uv_3, 3u, 83u, vec3<f32>(0.96, 0.42, 0.72)) * chart_mask_3;

    let spec_center = vec2<f32>(left_cx, 0.56) + layout_offset(uniforms.spectrum_offset);
    let spec_half = vec2<f32>(left_hx, 0.26);
    let spec_mask = rounded_box(uv, spec_center, spec_half, 0.018);
    let spec_uv = (uv - spec_center) / spec_half;
    color += spectrum_strip(spec_uv) * spec_mask;

    color += progress_bar(uv);
    color += status_badge(uv);
    color += hud_corners(uv);

    if uniforms.show_help > 0.5 {
        color = mix(color, color * 0.3, 0.5);
        color += help_panel(uv);
    }

    color = pow(clamp(color, vec3<f32>(0.0), vec3<f32>(1.0)), vec3<f32>(0.94));
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
