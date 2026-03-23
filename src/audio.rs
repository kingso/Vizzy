use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use ringbuf::traits::{Producer, Split};
use ringbuf::{HeapRb, SharedRb, storage::Heap};
use rustfft::{num_complex::Complex, FftPlanner};
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

pub const FFT_SIZE: usize = 2048;
pub const HOP_SIZE: usize = FFT_SIZE / 4;
pub const LOG_BAND_COUNT: usize = 24;

// Analysis tuning constants. Adjust these first when retuning the diagnostics view.
pub const MIN_ANALYSIS_FREQUENCY_HZ: f32 = 20.0;
pub const SUB_BASS_MAX_FREQUENCY_HZ: f32 = 60.0;
pub const BASS_MAX_FREQUENCY_HZ: f32 = 250.0;
pub const MID_MAX_FREQUENCY_HZ: f32 = 4_000.0;
pub const PRESENCE_MIN_FREQUENCY_HZ: f32 = 4_000.0;
pub const PRESENCE_MAX_FREQUENCY_HZ: f32 = 6_000.0;
pub const AIR_MIN_FREQUENCY_HZ: f32 = 10_000.0;

pub const NOISE_GATE_RMS_THRESHOLD: f32 = 0.018;
pub const NOISE_GATE_RMS_KNEE: f32 = 0.030;
pub const NOISE_GATE_PEAK_THRESHOLD: f32 = 0.080;
pub const NOISE_GATE_PEAK_KNEE: f32 = 0.120;
pub const ANALYSIS_CONFIG_DIR: &str = "Vizzy";
pub const ANALYSIS_CONFIG_FILE: &str = "analysis_settings.toml";

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct AnalysisSettings {
    pub sub_bass_max_frequency_hz: f32,
    pub bass_max_frequency_hz: f32,
    pub mid_max_frequency_hz: f32,
    pub presence_min_frequency_hz: f32,
    pub presence_max_frequency_hz: f32,
    pub air_min_frequency_hz: f32,
    pub noise_gate_rms_threshold: f32,
    pub noise_gate_rms_knee: f32,
    pub noise_gate_peak_threshold: f32,
    pub noise_gate_peak_knee: f32,
}

impl Default for AnalysisSettings {
    fn default() -> Self {
        Self {
            sub_bass_max_frequency_hz: SUB_BASS_MAX_FREQUENCY_HZ,
            bass_max_frequency_hz: BASS_MAX_FREQUENCY_HZ,
            mid_max_frequency_hz: MID_MAX_FREQUENCY_HZ,
            presence_min_frequency_hz: PRESENCE_MIN_FREQUENCY_HZ,
            presence_max_frequency_hz: PRESENCE_MAX_FREQUENCY_HZ,
            air_min_frequency_hz: AIR_MIN_FREQUENCY_HZ,
            noise_gate_rms_threshold: NOISE_GATE_RMS_THRESHOLD,
            noise_gate_rms_knee: NOISE_GATE_RMS_KNEE,
            noise_gate_peak_threshold: NOISE_GATE_PEAK_THRESHOLD,
            noise_gate_peak_knee: NOISE_GATE_PEAK_KNEE,
        }
    }
}

#[derive(Clone)]
pub struct AnalysisController {
    shared: Arc<Mutex<AnalysisSettings>>,
}

impl AnalysisController {
    pub fn new() -> Self {
        Self {
            shared: Arc::new(Mutex::new(load_analysis_settings().unwrap_or_default())),
        }
    }

    pub fn snapshot(&self) -> AnalysisSettings {
        self.shared.lock().map(|settings| *settings).unwrap_or_default()
    }

    pub fn update(&self, updater: impl FnOnce(&mut AnalysisSettings)) {
        let snapshot = if let Ok(mut settings) = self.shared.lock() {
            updater(&mut settings);
            Some(*settings)
        } else {
            None
        };

        if let Some(settings) = snapshot {
            save_analysis_settings(&settings);
        }
    }

    pub fn reset(&self) {
        let snapshot = if let Ok(mut settings) = self.shared.lock() {
            *settings = AnalysisSettings::default();
            Some(*settings)
        } else {
            None
        };

        if let Some(settings) = snapshot {
            save_analysis_settings(&settings);
        }
    }
}

fn analysis_settings_path() -> Option<PathBuf> {
    dirs::config_dir().map(|path| path.join(ANALYSIS_CONFIG_DIR).join(ANALYSIS_CONFIG_FILE))
}

fn load_analysis_settings() -> Option<AnalysisSettings> {
    let path = analysis_settings_path()?;
    let contents = fs::read_to_string(&path).ok()?;
    toml::from_str(&contents).ok()
}

fn save_analysis_settings(settings: &AnalysisSettings) {
    let Some(path) = analysis_settings_path() else {
        return;
    };

    let Some(parent) = path.parent() else {
        return;
    };

    if let Err(error) = fs::create_dir_all(parent) {
        eprintln!("Failed to create analysis settings directory {}: {}", parent.display(), error);
        return;
    }

    let serialized = match toml::to_string_pretty(settings) {
        Ok(serialized) => serialized,
        Err(error) => {
            eprintln!("Failed to serialize analysis settings: {}", error);
            return;
        }
    };

    if let Err(error) = fs::write(&path, serialized) {
        eprintln!("Failed to write analysis settings to {}: {}", path.display(), error);
    }
}

#[derive(Clone)]
pub struct TransportController {
    paused: Arc<AtomicBool>,
}

impl TransportController {
    fn new() -> Self {
        Self {
            paused: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn toggle_paused(&self) -> bool {
        let next = !self.paused.load(Ordering::Relaxed);
        self.paused.store(next, Ordering::Relaxed);
        next
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }
}

#[derive(Clone)]
pub enum AudioSource {
    InputDevice,
    File(PathBuf),
}

/// Pre-compute a Hann window to reduce spectral leakage in FFT output.
fn hann_window(size: usize) -> Vec<f32> {
    (0..size)
        .map(|i| {
            let x = std::f32::consts::PI * i as f32 / (size - 1) as f32;
            x.sin().powi(2)
        })
        .collect()
}

#[derive(Clone)]
pub struct AudioFrame {
    pub log_bins: [f32; LOG_BAND_COUNT],
    pub sub_bass_energy: f32,
    pub bass_energy: f32,
    pub mid_energy: f32,
    pub treble_energy: f32,
    pub presence_energy: f32,
    pub air_energy: f32,
    pub peak_amplitude: f32,
    pub rms: f32,
    pub spectral_centroid: f32,
}

struct SpectralFeatures {
    log_bins: [f32; LOG_BAND_COUNT],
    sub_bass_energy: f32,
    bass_energy: f32,
    mid_energy: f32,
    treble_energy: f32,
    presence_energy: f32,
    air_energy: f32,
}

type FftItem = AudioFrame;

pub type FftProducer = ringbuf::wrap::CachingProd<Arc<SharedRb<Heap<FftItem>>>>;
pub type FftConsumer = ringbuf::wrap::CachingCons<Arc<SharedRb<Heap<FftItem>>>>;

pub struct PreparedFileAudio {
    samples: Arc<[f32]>,
    channels: u16,
    sample_rate: u32,
    pub path: PathBuf,
}

pub enum PreparedAudioSource {
    InputDevice,
    File(PreparedFileAudio),
}

pub enum AudioLoadUpdate {
    Progress { progress: f32, stage: String },
    Ready(PreparedAudioSource),
    Error(String),
}

struct DecodedAudio {
    samples: Vec<f32>,
    channels: u16,
    sample_rate: u32,
}

struct PlaybackState {
    samples: Arc<[f32]>,
    source_channels: usize,
    source_sample_rate: u32,
    playhead_frames: f64,
}

pub struct AudioAnalyzer {
    _input_stream: Option<cpal::Stream>,
    _file_playback: Option<cpal::Stream>,
    pub analysis_controller: AnalysisController,
    pub transport_controller: Option<TransportController>,
    pub sample_rate: u32,
    pub rx: FftConsumer,
}

impl AudioAnalyzer {
    pub fn try_from_prepared(prepared: PreparedAudioSource) -> Result<Self, String> {
        match prepared {
            PreparedAudioSource::InputDevice => Self::try_from_input_device(),
            PreparedAudioSource::File(file) => Self::try_from_prepared_file(file),
        }
    }

    fn try_from_input_device() -> Result<Self, String> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or_else(|| "Failed to get default input device".to_string())?;
        
        let device_name = device
            .description()
            .map(|description| description.to_string())
            .unwrap_or_else(|_| "Unknown".into());
        println!("Using audio input device: {}", device_name);

        let config = device
            .default_input_config()
            .map_err(|error| format!("Failed to get default input format: {}", error))?;
        
        let (mut tx, rx) = HeapRb::<FftItem>::new(16).split();

        let stream_config = config.config();
        let channels = stream_config.channels as usize;
        let sample_rate_hz = stream_config.sample_rate;
        let sample_rate = sample_rate_hz as f32;
        let analysis_controller = AnalysisController::new();
        let analysis_controller_f32 = analysis_controller.clone();
        let analysis_controller_i16 = analysis_controller.clone();

        let mut planner = FftPlanner::new();
        let fft = planner.plan_fft_forward(FFT_SIZE);
        let window = hann_window(FFT_SIZE);
        let mut sample_buffer = Vec::with_capacity(FFT_SIZE);
        let mut complex_buffer = vec![Complex::new(0.0, 0.0); FFT_SIZE];

        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => device.build_input_stream(
                &stream_config,
                move |data: &[f32], _: &_| {
                    let settings = analysis_controller_f32.snapshot();
                    process_audio_f32(
                        data,
                        channels,
                        sample_rate,
                        &mut sample_buffer,
                        &window,
                        &fft,
                        &mut complex_buffer,
                        &settings,
                        &mut tx,
                    );
                },
                err_fn,
                None,
            ),
            cpal::SampleFormat::I16 => device.build_input_stream(
                &stream_config,
                move |data: &[i16], _: &_| {
                    let settings = analysis_controller_i16.snapshot();
                    process_audio_i16(
                        data,
                        channels,
                        sample_rate,
                        &mut sample_buffer,
                        &window,
                        &fft,
                        &mut complex_buffer,
                        &settings,
                        &mut tx,
                    );
                },
                err_fn,
                None,
            ),
            _ => return Err("Unsupported input sample format".to_string()),
        }
        .map_err(|error| format!("Failed to build input stream: {}", error))?;

        stream
            .play()
            .map_err(|error| format!("Failed to play input stream: {}", error))?;

        Ok(Self {
            _input_stream: Some(stream),
            _file_playback: None,
            analysis_controller,
            transport_controller: None,
            sample_rate: sample_rate_hz,
            rx,
        })
    }

    fn try_from_prepared_file(file: PreparedFileAudio) -> Result<Self, String> {
        println!("Analyzing audio file: {}", file.path.display());

        let analysis_controller = AnalysisController::new();
        let transport_controller = TransportController::new();
        let playback_position = Arc::new(AtomicU64::new(0));
        let playback = start_file_playback(
            file.samples.clone(),
            file.channels as usize,
            file.sample_rate,
            transport_controller.clone(),
            playback_position.clone(),
        )?;

        let (mut tx, rx) = HeapRb::<FftItem>::new(64).split();
        let channels = file.channels as usize;
        let sample_rate = file.sample_rate;
        let shared_samples = file.samples.clone();
        let analysis_controller_for_thread = analysis_controller.clone();
        let transport_controller_for_thread = transport_controller.clone();
        let playback_position_for_thread = playback_position.clone();

        thread::spawn(move || {
            loop {
                if let Err(error) = analyze_audio_samples(
                    &shared_samples,
                    channels,
                    sample_rate,
                    &analysis_controller_for_thread,
                    Some(&transport_controller_for_thread),
                    &mut tx,
                    &playback_position_for_thread,
                ) {
                    eprintln!("Audio file analysis failed: {}", error);
                    break;
                }
            }
        });

        Ok(Self {
            _input_stream: None,
            _file_playback: Some(playback),
            analysis_controller,
            transport_controller: Some(transport_controller),
            sample_rate,
            rx,
        })
    }
}

pub fn spawn_audio_preload(source: AudioSource) -> Receiver<AudioLoadUpdate> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || match source {
        AudioSource::InputDevice => {
            send_progress(&tx, 0.20, "Preparing microphone input");
            send_progress(&tx, 1.00, "Microphone ready");
            let _ = tx.send(AudioLoadUpdate::Ready(PreparedAudioSource::InputDevice));
        }
        AudioSource::File(path) => {
            let display_path = path.display().to_string();
            send_progress(&tx, 0.08, format!("Opening {}", display_path));
            send_progress(&tx, 0.28, "Decoding audio file");

            match decode_audio_file(&path) {
                Ok(decoded) => {
                    send_progress(&tx, 0.78, "Preparing playback and analysis");
                    let prepared = PreparedAudioSource::File(PreparedFileAudio {
                        samples: Arc::from(decoded.samples),
                        channels: decoded.channels,
                        sample_rate: decoded.sample_rate,
                        path,
                    });
                    send_progress(&tx, 1.00, "Audio ready");
                    let _ = tx.send(AudioLoadUpdate::Ready(prepared));
                }
                Err(error) => {
                    let _ = tx.send(AudioLoadUpdate::Error(format!(
                        "Failed to decode audio file {}: {}",
                        display_path,
                        error
                    )));
                }
            }
        }
    });
    rx
}

fn send_progress(tx: &Sender<AudioLoadUpdate>, progress: f32, stage: impl Into<String>) {
    let _ = tx.send(AudioLoadUpdate::Progress {
        progress: progress.clamp(0.0, 1.0),
        stage: stage.into(),
    });
}

fn process_audio_f32(
    input_data: &[f32],
    channels: usize,
    sample_rate: f32,
    sample_buffer: &mut Vec<f32>,
    window: &[f32],
    fft: &Arc<dyn rustfft::Fft<f32>>,
    complex_buffer: &mut [Complex<f32>],
    settings: &AnalysisSettings,
    tx: &mut FftProducer,
) {
    mixdown_samples_f32(input_data, channels, sample_buffer);

    process_fft_frames(sample_buffer, sample_rate, window, fft, complex_buffer, settings, tx);
}

fn process_audio_i16(
    input_data: &[i16],
    channels: usize,
    sample_rate: f32,
    sample_buffer: &mut Vec<f32>,
    window: &[f32],
    fft: &Arc<dyn rustfft::Fft<f32>>,
    complex_buffer: &mut [Complex<f32>],
    settings: &AnalysisSettings,
    tx: &mut FftProducer,
) {
    mixdown_samples_i16(input_data, channels, sample_buffer);

    process_fft_frames(sample_buffer, sample_rate, window, fft, complex_buffer, settings, tx);
}

fn mixdown_samples_f32(input_data: &[f32], channels: usize, sample_buffer: &mut Vec<f32>) {
    for chunk in input_data.chunks(channels.max(1)) {
        if chunk.is_empty() {
            continue;
        }

        let mixed = chunk.iter().copied().sum::<f32>() / chunk.len() as f32;
        sample_buffer.push(mixed);
    }
}

fn mixdown_samples_i16(input_data: &[i16], channels: usize, sample_buffer: &mut Vec<f32>) {
    for chunk in input_data.chunks(channels.max(1)) {
        if chunk.is_empty() {
            continue;
        }

        let sum = chunk.iter().map(|sample| *sample as f32 / i16::MAX as f32).sum::<f32>();
        sample_buffer.push(sum / chunk.len() as f32);
    }
}

fn band_rms(bins: &[f32]) -> f32 {
    if bins.is_empty() {
        return 0.0;
    }

    (bins.iter().map(|value| value * value).sum::<f32>() / bins.len() as f32).sqrt()
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let width = (edge1 - edge0).max(f32::EPSILON);
    let t = ((x - edge0) / width).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn noise_gate_gain(rms: f32, peak_amplitude: f32, settings: &AnalysisSettings) -> f32 {
    let rms_gate = smoothstep(
        settings.noise_gate_rms_threshold,
        settings.noise_gate_rms_threshold + settings.noise_gate_rms_knee,
        rms,
    );
    let peak_gate = smoothstep(
        settings.noise_gate_peak_threshold,
        settings.noise_gate_peak_threshold + settings.noise_gate_peak_knee,
        peak_amplitude,
    );

    rms_gate.max(peak_gate)
}

fn frequency_to_bin(frequency_hz: f32, sample_rate: f32) -> usize {
    ((frequency_hz / sample_rate) * FFT_SIZE as f32)
        .floor()
        .clamp(0.0, (FFT_SIZE / 2 - 1) as f32) as usize
}

fn aggregate_log_band_range(log_bins: &[f32], centers: &[f32], min_frequency_hz: f32, max_frequency_hz: f32) -> f32 {
    let mut sum = 0.0;
    let mut count = 0u32;

    for (bin, center_frequency) in log_bins.iter().zip(centers.iter()) {
        if *center_frequency >= min_frequency_hz && *center_frequency < max_frequency_hz {
            sum += bin * bin;
            count += 1;
        }
    }

    if count > 0 {
        (sum / count as f32).sqrt()
    } else {
        0.0
    }
}

fn extract_log_spaced_features(magnitudes: &[f32], sample_rate: f32, settings: &AnalysisSettings) -> SpectralFeatures {
    if magnitudes.is_empty() {
        return SpectralFeatures {
            log_bins: [0.0; LOG_BAND_COUNT],
            sub_bass_energy: 0.0,
            bass_energy: 0.0,
            mid_energy: 0.0,
            treble_energy: 0.0,
            presence_energy: 0.0,
            air_energy: 0.0,
        };
    }

    let nyquist = (sample_rate * 0.5).max(MIN_ANALYSIS_FREQUENCY_HZ * 2.0);
    let min_frequency = MIN_ANALYSIS_FREQUENCY_HZ.min(nyquist * 0.5);
    let frequency_ratio = (nyquist / min_frequency).max(1.0 + f32::EPSILON);
    let mut log_bins = [0.0; LOG_BAND_COUNT];
    let mut log_bin_centers = [0.0; LOG_BAND_COUNT];

    for band_index in 0..LOG_BAND_COUNT {
        let start_t = band_index as f32 / LOG_BAND_COUNT as f32;
        let end_t = (band_index + 1) as f32 / LOG_BAND_COUNT as f32;
        let start_frequency = min_frequency * frequency_ratio.powf(start_t);
        let end_frequency = min_frequency * frequency_ratio.powf(end_t);
        let start_bin = frequency_to_bin(start_frequency, sample_rate).min(magnitudes.len() - 1);
        let mut end_bin = frequency_to_bin(end_frequency, sample_rate).min(magnitudes.len());
        if end_bin <= start_bin {
            end_bin = (start_bin + 1).min(magnitudes.len());
        }

        log_bins[band_index] = band_rms(&magnitudes[start_bin..end_bin]);
        log_bin_centers[band_index] = (start_frequency * end_frequency).sqrt();
    }

    SpectralFeatures {
        sub_bass_energy: aggregate_log_band_range(&log_bins, &log_bin_centers, MIN_ANALYSIS_FREQUENCY_HZ, settings.sub_bass_max_frequency_hz),
        bass_energy: aggregate_log_band_range(&log_bins, &log_bin_centers, settings.sub_bass_max_frequency_hz, settings.bass_max_frequency_hz),
        mid_energy: aggregate_log_band_range(&log_bins, &log_bin_centers, settings.bass_max_frequency_hz, settings.mid_max_frequency_hz),
        treble_energy: aggregate_log_band_range(&log_bins, &log_bin_centers, settings.mid_max_frequency_hz, nyquist + 1.0),
        presence_energy: aggregate_log_band_range(&log_bins, &log_bin_centers, settings.presence_min_frequency_hz, settings.presence_max_frequency_hz),
        air_energy: aggregate_log_band_range(&log_bins, &log_bin_centers, settings.air_min_frequency_hz, nyquist + 1.0),
        log_bins,
    }
}

fn process_fft_frames(
    sample_buffer: &mut Vec<f32>,
    sample_rate: f32,
    window: &[f32],
    fft: &Arc<dyn rustfft::Fft<f32>>,
    complex_buffer: &mut [Complex<f32>],
    settings: &AnalysisSettings,
    tx: &mut FftProducer,
) {
    while sample_buffer.len() >= FFT_SIZE {
        let mono_frame = &sample_buffer[..FFT_SIZE];
        let peak_amplitude = mono_frame
            .iter()
            .map(|sample| sample.abs())
            .fold(0.0, f32::max);
        let rms = (mono_frame.iter().map(|sample| sample * sample).sum::<f32>() / FFT_SIZE as f32).sqrt();

        for ((complex, sample), weight) in complex_buffer
            .iter_mut()
            .zip(mono_frame.iter())
            .zip(window.iter())
        {
            *complex = Complex::new(*sample * *weight, 0.0);
        }

        fft.process(complex_buffer);

        let mut magnitudes = [0.0; FFT_SIZE / 2];
        let mut weighted_sum = 0.0;
        let mut magnitude_sum = 0.0;
        for (index, value) in magnitudes.iter_mut().enumerate() {
            let bin = complex_buffer[index];
            let magnitude = (bin.re * bin.re + bin.im * bin.im).sqrt() / FFT_SIZE as f32;
            *value = magnitude;

            let bin_frequency = index as f32 * sample_rate / FFT_SIZE as f32;
            weighted_sum += bin_frequency * magnitude;
            magnitude_sum += magnitude;
        }

        let nyquist = (sample_rate * 0.5).max(1.0);
        let spectral_centroid = if magnitude_sum > 1e-6 {
            (weighted_sum / magnitude_sum / nyquist).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let spectral_features = extract_log_spaced_features(&magnitudes, sample_rate, settings);
        let gate = noise_gate_gain(rms, peak_amplitude, settings);

        for magnitude in &mut magnitudes {
            *magnitude *= gate;
        }

        let mut gated_log_bins = spectral_features.log_bins;
        for bin in &mut gated_log_bins {
            *bin *= gate;
        }

        let _ = tx.try_push(AudioFrame {
            log_bins: gated_log_bins,
            sub_bass_energy: spectral_features.sub_bass_energy * gate,
            bass_energy: spectral_features.bass_energy * gate,
            mid_energy: spectral_features.mid_energy * gate,
            treble_energy: spectral_features.treble_energy * gate,
            presence_energy: spectral_features.presence_energy * gate,
            air_energy: spectral_features.air_energy * gate,
            peak_amplitude: peak_amplitude * gate,
            rms: rms * gate,
            spectral_centroid: spectral_centroid * gate,
        });
        sample_buffer.drain(..HOP_SIZE);
    }
}

fn decode_audio_file(path: &Path) -> Result<DecodedAudio, String> {
    let file = File::open(path).map_err(|error| error.to_string())?;
    let media_source = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();

    if let Some(extension) = path.extension().and_then(|ext| ext.to_str()) {
        hint.with_extension(extension);
    }

    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            media_source,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|error| error.to_string())?;

    let mut format = probed.format;
    let track = format
        .default_track()
        .ok_or_else(|| "No default audio track found in file".to_string())?;
    let sample_rate = track
        .codec_params
        .sample_rate
        .ok_or_else(|| "Audio file is missing a sample rate".to_string())?;
    let channel_count = track
        .codec_params
        .channels
        .map(|channels| channels.count() as u16)
        .unwrap_or(1);
    let track_id = track.id;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|error| error.to_string())?;
    let mut samples = Vec::new();

    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(SymphoniaError::IoError(_)) => break,
            Err(SymphoniaError::ResetRequired) => {
                return Err("Unsupported mid-stream format reset".to_string())
            }
            Err(error) => return Err(error.to_string()),
        };

        if packet.track_id() != track_id {
            continue;
        }

        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(SymphoniaError::IoError(_)) => break,
            Err(error) => return Err(error.to_string()),
        };

        let mut interleaved = SampleBuffer::<f32>::new(decoded.capacity() as u64, *decoded.spec());
        interleaved.copy_interleaved_ref(decoded);
        samples.extend_from_slice(interleaved.samples());
    }

    Ok(DecodedAudio {
        samples,
        channels: channel_count,
        sample_rate,
    })
}

fn start_file_playback(
    samples: Arc<[f32]>,
    source_channels: usize,
    source_sample_rate: u32,
    transport_controller: TransportController,
    playback_position: Arc<AtomicU64>,
) -> Result<cpal::Stream, String> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| "Failed to get default output device".to_string())?;
    let output_config = device
        .default_output_config()
        .map_err(|error| error.to_string())?;
    let output_channels = output_config.channels() as usize;
    let output_sample_rate = output_config.sample_rate();

    let state = Arc::new(Mutex::new(PlaybackState {
        samples,
        source_channels,
        source_sample_rate,
        playhead_frames: 0.0,
    }));

    let stream = match output_config.sample_format() {
        cpal::SampleFormat::F32 => build_output_stream::<f32>(
            &device,
            &output_config.into(),
            output_channels,
            output_sample_rate,
            state,
            transport_controller,
            playback_position,
        ),
        cpal::SampleFormat::I16 => build_output_stream::<i16>(
            &device,
            &output_config.into(),
            output_channels,
            output_sample_rate,
            state,
            transport_controller,
            playback_position,
        ),
        cpal::SampleFormat::U16 => build_output_stream::<u16>(
            &device,
            &output_config.into(),
            output_channels,
            output_sample_rate,
            state,
            transport_controller,
            playback_position,
        ),
        _ => Err("Unsupported output sample format".to_string()),
    }?;

    stream.play().map_err(|error| error.to_string())?;
    Ok(stream)
}

fn build_output_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    output_channels: usize,
    output_sample_rate: u32,
    state: Arc<Mutex<PlaybackState>>,
    transport_controller: TransportController,
    playback_position: Arc<AtomicU64>,
) -> Result<cpal::Stream, String>
where
    T: cpal::SizedSample + 'static,
{
    device
        .build_output_stream(
            config,
            move |data: &mut [T], _: &_| {
                fill_output_buffer(data, output_channels, output_sample_rate, &state, &transport_controller, &playback_position);
            },
            err_fn,
            None,
        )
        .map_err(|error| error.to_string())
}

fn fill_output_buffer<T>(
    data: &mut [T],
    output_channels: usize,
    output_sample_rate: u32,
    state: &Arc<Mutex<PlaybackState>>,
    transport_controller: &TransportController,
    playback_position: &Arc<AtomicU64>,
) where
    T: cpal::SizedSample + 'static,
{
    if transport_controller.is_paused() {
        for sample in data.iter_mut() {
            *sample = silence_sample();
        }
        return;
    }

    let Ok(mut state) = state.lock() else {
        for sample in data.iter_mut() {
            *sample = silence_sample();
        }
        return;
    };

    let source_channels = state.source_channels.max(1);
    let source_frame_count = state.samples.len() / source_channels;
    if source_frame_count == 0 {
        for sample in data.iter_mut() {
            *sample = silence_sample();
        }
        return;
    }

    let step = state.source_sample_rate as f64 / output_sample_rate as f64;

    for frame in data.chunks_mut(output_channels) {
        let frame_position = state.playhead_frames;
        let base_frame = frame_position.floor() as usize % source_frame_count;
        let next_frame = (base_frame + 1) % source_frame_count;
        let fraction = (frame_position - frame_position.floor()) as f32;

        for (channel, sample) in frame.iter_mut().enumerate() {
            let source_channel = channel.min(source_channels - 1);
            let current = state.samples[base_frame * source_channels + source_channel];
            let next = state.samples[next_frame * source_channels + source_channel];
            let mixed = current + (next - current) * fraction;
            *sample = convert_sample::<T>(mixed);
        }

        state.playhead_frames += step;
        if state.playhead_frames >= source_frame_count as f64 {
            state.playhead_frames -= source_frame_count as f64;
        }
    }

    // Publish current playback position for analysis thread sync
    playback_position.store(state.playhead_frames.to_bits(), Ordering::Relaxed);
}

fn convert_sample<T>(sample: f32) -> T
where
    T: cpal::SizedSample + 'static,
{
    let sample = sample.clamp(-1.0, 1.0);

    if std::any::TypeId::of::<T>() == std::any::TypeId::of::<f32>() {
        unsafe { std::mem::transmute_copy::<f32, T>(&sample) }
    } else if std::any::TypeId::of::<T>() == std::any::TypeId::of::<i16>() {
        let value = (sample * i16::MAX as f32) as i16;
        unsafe { std::mem::transmute_copy::<i16, T>(&value) }
    } else {
        let value = ((sample * 0.5 + 0.5) * u16::MAX as f32) as u16;
        unsafe { std::mem::transmute_copy::<u16, T>(&value) }
    }
}

fn silence_sample<T>() -> T
where
    T: cpal::SizedSample + 'static,
{
    convert_sample::<T>(0.0)
}

fn analyze_audio_samples(
    samples: &[f32],
    channels: usize,
    sample_rate: u32,
    analysis_controller: &AnalysisController,
    transport_controller: Option<&TransportController>,
    tx: &mut FftProducer,
    playback_position: &Arc<AtomicU64>,
) -> Result<(), String> {
    if channels == 0 || sample_rate == 0 {
        return Err("Decoded audio has invalid stream parameters".to_string());
    }

    let window = hann_window(FFT_SIZE);
    let mut planner = FftPlanner::new();
    let fft = planner.plan_fft_forward(FFT_SIZE);
    let mut sample_buffer = Vec::with_capacity(FFT_SIZE * 2);
    let mut complex_buffer = vec![Complex::new(0.0, 0.0); FFT_SIZE];
    let chunk_frames = 1024usize;
    let chunk_samples = chunk_frames * channels;
    let mut emitted_frames = 0u64;
    let total_frames = (samples.len() / channels.max(1)) as u64;

    for chunk in samples.chunks(chunk_samples.max(channels)) {
        while transport_controller.is_some_and(|controller| controller.is_paused()) {
            thread::sleep(Duration::from_millis(16));
        }

        let settings = analysis_controller.snapshot();
        process_audio_f32(
            chunk,
            channels,
            sample_rate as f32,
            &mut sample_buffer,
            &window,
            &fft,
            &mut complex_buffer,
            &settings,
            tx,
        );

        emitted_frames += (chunk.len() / channels) as u64;
        // Pace analysis against actual playback position (hardware audio clock)
        loop {
            let playback_bits = playback_position.load(Ordering::Relaxed);
            let playback_frames = f64::from_bits(playback_bits) as u64;
            // Allow analysis to be slightly ahead (1 chunk) but not more
            if emitted_frames <= playback_frames + chunk_frames as u64 {
                break;
            }
            // Handle wraparound: if playback wrapped past us, break
            if total_frames > 0 && playback_frames + total_frames / 2 < emitted_frames {
                break;
            }
            thread::sleep(Duration::from_millis(2));
        }
    }

    sample_buffer.clear();
    Ok(())
}

fn err_fn(err: cpal::StreamError) {
    eprintln!("An error occurred on the audio stream: {}", err);
}