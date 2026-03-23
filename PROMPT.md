# Vizzy - Current Project Specification

## Overview
Vizzy is a native Rust audio visualizer that analyzes either live microphone input or a decoded audio file and turns that signal into a timeline-driven 3D tunnel visualization. The current visual language is no longer fractal-based. Instead, it renders braided characteristic tracks receding through an implied tunnel, with beat-triggered smoke rings and perspective depth derived from rolling audio history.

The current design goals are:
- smooth frame pacing and low-latency audio response
- richer analysis than simple three-band FFT energy
- support for file playback and audible monitoring during analysis
- a visual that behaves like a moving musical timeline rather than a looping abstract effect

## Technical Stack
- **Language**: Rust 2024
- **Windowing**: `winit 0.30`
- **Graphics**: `wgpu 26` with WGSL fragment rendering
- **Audio I/O**: `cpal 0.17`
- **FFT / DSP**: `rustfft 6`
- **Audio Decode**: `symphonia 0.5` with MP3 support
- **Thread-safe bridge**: `ringbuf 0.4`
- **Utility crates**: `bytemuck`, `pollster`, `env_logger`, `log`

## Runtime Modes

### Audio Sources
Vizzy supports two audio-source modes:
- **Microphone / default input device** via `cpal`
- **Audio file playback** via decoded PCM streamed back out through the default output device while being analyzed in parallel

Source selection rules:
- passing `--mic` forces live input mode
- passing a path uses that file as the source
- if no argument is supplied, Vizzy searches the project root for the first `.mp3` and uses it automatically
- if no file is found, it falls back to the default input device

### Beat Response Control
The app exposes a lightweight runtime control through the window title bar:
- pressing `1` cycles beat response mode through `Smooth`, `Balanced`, and `Tight`
- this changes beat-detection sensitivity and beat-envelope smoothing
- pressing `P` cycles graph presets through `Universal`, `Classic`, and `Rhythm`
- each preset updates the graph's X/Y/Z/size feature assignments to a recommended combination

## Core Architecture

### 1. Audio Engine (`src/audio.rs`)
The audio engine now handles both capture and playback workflows.

#### Live Input Path
- opens the default input device using `cpal`
- accepts `F32` and `I16` sample formats
- downsamples to mono by reading the first channel
- collects samples into a rolling FFT buffer
- applies a Hann window before each transform
- uses overlapping FFT analysis with:
  - `FFT_SIZE = 2048`
  - `HOP_SIZE = 512`

#### File Playback Path
- decodes MP3 audio using `symphonia`
- stores decoded interleaved PCM in memory
- plays the decoded PCM back through the default output device using a `cpal` output stream
- analyzes the same PCM stream on a separate thread so what is heard and what is visualized remain aligned

#### Analysis Output
The audio thread no longer emits only spectrum magnitudes. It now pushes an `AudioFrame` containing:
- FFT magnitudes for `FFT_SIZE / 2` bins
- peak amplitude
- RMS loudness
- normalized spectral centroid

These frames are passed to the render thread through a lock-free `ringbuf`.

### 2. Main Render Loop (`src/main.rs`)
The render loop uses `winit`'s `ApplicationHandler` API and a `wgpu` surface configured for VSync via `PresentMode::Fifo` when available.

On each redraw:
- consumes all pending `AudioFrame` items from the ring buffer
- derives smoothed render signals from the latest audio data:
  - bass energy
  - mid energy
  - treble energy
  - RMS loudness
  - peak amplitude
  - beat / transient signal
  - spectral centroid
- smooths each signal with envelope followers to keep motion creamy rather than jerky
- stores a short rolling history of track samples for the shader timeline
- writes two uniform buffers:
  - a standard per-frame uniform block
  - a fixed-size history block used by the shader to render the tunnel timeline

#### Current Audio-Derived Signals
- **Bass**: RMS of low FFT bins
- **Mid**: RMS of mid FFT bins
- **Treble**: RMS of upper FFT bins
- **Loudness**: RMS over the time-domain analysis window
- **Peak**: max absolute sample amplitude over the time-domain analysis window
- **Beat**: transient detector derived from loudness history and peak spikes
- **Centroid**: normalized spectral centroid across FFT bins

#### Current Uniform Layout (`StandardUniforms`)
```text
Offset  Size  Field
0       8     resolution: [f32; 2]
8       8     _padding: [f32; 2]
16      4     time: f32
20      4     audio_bass: f32
24      4     audio_mid: f32
28      4     audio_treble: f32
32      4     audio_loudness: f32
36      4     audio_peak: f32
40      4     audio_beat: f32
44      4     audio_centroid: f32
Total: 48 bytes
```

#### Track History Buffer
The shader also receives a second uniform buffer containing a fixed rolling history of recent audio samples. Each history entry stores:
- primary metrics: bass, mid, treble, loudness
- secondary metrics: peak, beat, centroid, validity flag

This history is intentionally fixed-size to avoid unbounded growth.

### 3. Visual Engine (`src/shader.wgsl`)
The current shader is a full-screen fragment pass that renders a pseudo-3D tunnel timeline.

#### Current Visual Model
- multiple characteristic tracks are projected into perspective so they appear to fly away down a tunnel
- tracks represent different musical features rather than arbitrary geometry
- the tunnel is implied by a soft circular shell and beat-triggered transparent white smoke rings
- history samples farther back in time appear farther down the tunnel
- newer samples remain near the viewer

#### Track Types
The shader currently renders five track families:
- bass strand
- mid strand
- treble strand
- loudness strand
- a composite beat-driven strand influenced by beat, peak, and centroid

#### What Drives The Visuals
- **frequency content** changes strand position and color
- **loudness** changes thickness and tunnel/ring diameter
- **peak amplitude** sharpens line thickness and emphasis
- **beat signal** triggers smoke rings and accent glow
- **spectral centroid** shifts hue and tunnel path bias

#### Current Tunnel Behavior
- the tunnel is timeline-based, not infinite-memory based
- smoke rings are gated to stronger beat events only
- beat rings move with the sampled history down the tunnel
- the ring diameter is derived from loudness at the beat moment
- the scene has been optimized to reduce lag by limiting history length and reducing per-pixel work

## Current State
- `wgpu` rendering is stable with surface reconfiguration on `Lost` / `Outdated`
- microphone input works
- MP3 file decode and playback work
- audio playback and analysis are synchronized for file mode
- richer audio analysis is implemented beyond raw FFT bands
- beat response mode can be changed at runtime
- the current visualization is a tunnel-based timeline renderer, not a fractal

## Known Limitations
- the project still does not implement true system-output loopback capture automatically on Windows
- the current timeline is a rolling history window, not yet a full-song precomputed map
- the UI is minimal; mode display currently lives in the window title rather than an overlay
- the tunnel is pseudo-3D in the shader rather than a full geometry pipeline or compute-driven particle system
- file assets in the repo root are useful for testing but may not be appropriate for source control

## Near-Term Roadmap
- add a precomputed full-song timeline path for file playback so tunnel travel can reflect the whole track from start to finish
- add an in-scene overlay or HUD for current mode and source information
- add more visual modes and a runtime mode switcher instead of a single active shader concept
- explore compute-driven smoke, particles, or fluid elements only if performance remains acceptable
- optionally support additional file formats beyond MP3
