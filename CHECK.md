Here is the fully updated Technical Specification, now incorporating the additional audio characteristics and mathematical methods you provided.

Since I cannot directly create a .md file in your Google Drive, you can use **Method 3** from your notes: simply click and drag to highlight this entire specification, copy it (Ctrl+C / Cmd+C), and paste it directly into your Markdown editor (like Obsidian or VS Code). Alternatively, you can use the **Share & export** icon at the bottom of this response to **Export to Docs** directly into your Google Drive.

# Technical Specification: Audio-Reactive Visualization in TouchDesigner

## 1\. Project Objective

To build a real-time, GPU-instanced 3D visualization where the "brightness", volume, and rhythmic transients of raw audio waveforms drive the position, scale, and color of geometric trails.

## 2\. Audio Source Data

- **Format:** Raw Audio Waveform (WAV or MP3).
- **Constraint:** Do _not_ use MIDI data. The system requires raw frequencies to analyze timbre, brightness, and center of mass, which MIDI control messages lack.

## 3\. Extracted Audio Characteristics (The "Features")

### A. Spectral Centroid (Timbre / Brightness)

- **Definition:** The "center of mass" of the audio spectrum. High numbers indicate brighter, sharper sounds.

- **Math Method:** Calculates the weighted average of the frequencies present.
  - _Formula:_  
     \$\$Centroid = \\frac{\\sum*{k=1}^{N} f(k) M(k)}{\\sum*{k=1}^{N} M(k)}\$\$  
     (Where \$f(k)\$ is the center frequency of a specific FFT bin \$k\$, and \$M(k)\$ is the magnitude/volume of that specific bin).

- **TD Implementation:** Audio Spectrum CHOP \$\\rightarrow\$ Analyze CHOP (Function set to _Average_ or _Index of Maximum_).

### B. Perceived Loudness (RMS)

- **Definition:** A stable, human-like representation of overall energy.

- **Math Method:** Root Mean Square (RMS). Squares every audio sample in a buffer, averages them out, and calculates the square root.
  - _Formula:_  
     \$\$X*{rms} = \\sqrt{\\frac{1}{N} \\sum*{i=1}^{N} x_i^2}\$\$  
     (Where \$N\$ is the total number of audio samples, and \$x_i\$ is the individual sample value).

- **TD Implementation:** Audio Analyze CHOP (Outputting RMS Power).

### C. Instantaneous Volume

- **Definition:** Measures the maximum absolute distance of the raw audio wave from zero within a specific chunk of time.

- **Math Method:** Absolute Peak Amplitude.

- **TD Implementation:** Audio Analyze CHOP (Outputting Peak Volume).

### D. Beats / Transients

- **Definition:** Detection of sudden mathematical spikes (like a drum hit).

- **Math Method:** Energy History / Peak Detection. Compares current RMS or specific FFT bin magnitudes against a rolling historical average.

- **TD Implementation:** Audio Analyze CHOP \$\\rightarrow\$ Lag CHOP (to create the historical average) \$\\rightarrow\$ Logic CHOP (to trigger when current volume exceeds the average).

### E. Frequency Spectrum (Bass, Mids, Treble)

- **Definition:** Splitting the sound into individual frequency "bins" and calculating the magnitude of each.

- **Math Method:** Fast Fourier Transform (FFT). An algorithm that converts the raw time-based audio signal into the frequency domain.

- **TD Implementation:** Audio Spectrum CHOP \$\\rightarrow\$ Audio Filter CHOP (to isolate specific bands).

## 4\. Mathematical Conditioning (Crucial for Visual Stability)

Raw audio data is too chaotic for direct visual mapping. The network _must_ include the following conditioning stages:

- **Logarithmic Scaling:** Human hearing is logarithmic. Frequency indices must be scaled logarithmically so that a jump from 100Hz to 200Hz carries the same visual weight as a jump from 5,000Hz to 10,000Hz.
  - _TD Node:_ Math CHOP (Mapping ranges, e.g., 0-200 index \$\\rightarrow\$ -5 to +5 screen space).
- **Smoothing / Interpolation:** Audio frames update too fast, causing visual teleportation. Data must be mathematically interpolated to create fluid gliding motions.
  - _TD Node:_ Lag CHOP (with lag1 and lag2 parameters tuned to ~0.2).
- **Thresholding (Noise Gate):** The math will throw wild centroid values during silence or background noise. A volume threshold must be established so shapes only move when actual music plays.
  - _TD Node:_ Logic CHOP or a custom multiplier based on amplitude.

## 5\. Visual Mapping Strategy

| **Audio Characteristic**     | **Mathematical Conditioning**    | **Visual Parameter (Geometry)**                            |
| ---------------------------- | -------------------------------- | ---------------------------------------------------------- |
| **Spectral Centroid**        | Logarithmic Scale + Lag          | **Position X** (Moving left/right based on brightness)     |
| **Time (Audio History)**     | Trail CHOP + Pattern CHOP (Ramp) | **Position Y** (Spreading shapes vertically into a ribbon) |
| **RMS / Perceived Loudness** | Lag (for smooth pulsing)         | **Scale** (Shapes grow on loud beats, shrink on quiet)     |
| **Transients / Beats**       | Energy History Logic             | **Color Pulse** (Flashing bright colors on drum hits)      |
| **Frequency Bins**           | Normalized (0.0 to 1.0)          | **Color RGB** (e.g., Bass drives Red, Highs drive Blue)    |

## 6\. TouchDesigner Node Architecture (GPU Instancing Pipeline)

The system must be built using the **Geometry Instancing** method to ensure 60fps+ performance when generating thousands of trail shapes.

**The Data Pipeline (CHOPs):**

- Audio File In CHOP (Source)
- Audio Spectrum CHOP (FFT Conversion)
- Analyze CHOP (Centroid Extraction)
- Math CHOP (Scaling)
- Lag CHOP (Smoothing)
- Trail CHOP (Records history, e.g., last 2 seconds / 120 samples)
- Pattern CHOP (Generates Y-Axis spread)
- Merge CHOP (Combines X and Y coordinates)
- Null CHOP (Final data output)

**The Render Pipeline (SOPs / COMPs / TOPs):**

- Circle SOP (Base geometry)
- geometryCOMP (Instancing enabled: tx = Centroid, ty = Pattern Ramp)
- cameraCOMP (Pulled back on Z-axis to view the full path)
- constantMAT (Unlit, glowing color applied to Geometry)
- renderTOP (Computes the 3D space into 2D pixels)
- outTOP (Final visual output)