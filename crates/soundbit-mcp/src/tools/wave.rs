//! Direct continuous Hz wave, Aloboi analog drift, and XY Oscilloscope Lissajous synthesis tool.

use std::f64::consts::TAU;

use rmcp::handler::server::wrapper::Parameters;
use rmcp::{tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;
use soundbit_app::export_stereo_samples_wav;

use super::{ToolResult, attempt, refuse, reply};
use crate::SoundBitServer;

/// One continuous wave segment with fine-grained Hz, stereo XY ratio, and analog drift controls.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub(crate) struct WaveSegmentInput {
    /// Segment duration in milliseconds (10 to 60000).
    duration_ms: u32,
    /// Starting frequency in hertz (16.0 to 20000.0), e.g. 110.0 or 432.5.
    start_hz: f64,
    /// Ending frequency in hertz (16.0 to 20000.0). Defaults to `start_hz` (steady pitch); set different for continuous Hz sweeps/glides.
    end_hz: Option<f64>,
    /// Waveform style: `"sine"`, `"triangle"`, `"sawtooth"`, `"pulse"`, `"aloboi_warm"` (warm analog drift + sub + chorus), or `"oscilloscope_xy"` (phase-locked XY Lissajous stereo).
    #[serde(default = "default_waveform")]
    waveform: String,
    /// Right-channel harmonic frequency multiplier relative to Left channel (e.g. 1.0 for circle, 1.5 for 3:2 Lissajous fifth, 1.3333 for 4:3 fourth).
    #[serde(default = "one_f64")]
    xy_ratio: f64,
    /// Fine Hz offset added to the right channel (e.g. 0.5 Hz for slow 3D oscilloscope rotation or binaural beat).
    #[serde(default)]
    detune_hz: f64,
    /// Stereo phase offset in degrees (0 to 360; 90 produces circular XY quadrature on an oscilloscope).
    #[serde(default = "default_phase_deg")]
    phase_deg: f64,
    /// Aloboi-style analog tape wow/flutter depth in cents (0.0 to 60.0).
    #[serde(default = "default_drift_cents")]
    analog_drift_cents: f64,
    /// Segment amplitude in `[0.0, 1.0]`. Defaults to 0.75.
    #[serde(default = "default_gain")]
    gain: f32,
}

fn default_waveform() -> String {
    "aloboi_warm".to_owned()
}
const fn one_f64() -> f64 {
    1.0
}
const fn default_phase_deg() -> f64 {
    90.0
}
const fn default_drift_cents() -> f64 {
    14.0
}
const fn default_gain() -> f32 {
    0.75
}

/// Parameters for `wave_synthesize_hz`.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct WaveSynthesizeInput {
    /// Output WAV path relative to the workspace, e.g. `renders/aloboi_wave.wav`.
    path: String,
    /// Ordered list of continuous wave segments to synthesize seamlessly (phase-continuous across segments).
    segments: Vec<WaveSegmentInput>,
    /// Sample rate in hertz (8000 to 192000). Defaults to 48000.
    sample_rate: Option<u32>,
}

#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "audio DSP synthesis converts sample indices and bounded floats by design"
)]
fn synthesize_segments(
    segments: &[WaveSegmentInput],
    sample_rate: u32,
) -> Result<Vec<f32>, String> {
    if segments.is_empty() {
        return Err("provide at least one segment in `segments`".to_owned());
    }
    if !(8_000..=192_000).contains(&sample_rate) {
        return Err(format!(
            "sample_rate must be between 8000 and 192000 (got {sample_rate})"
        ));
    }
    let sr = f64::from(sample_rate);
    let mut out = Vec::new();
    let mut phase_l = 0.0_f64;
    let mut phase_r = 0.0_f64;
    let mut phase_sub = 0.0_f64;
    let mut lfo_phase = 0.0_f64;

    for (idx, seg) in segments.iter().enumerate() {
        if !(10..=60_000).contains(&seg.duration_ms) {
            return Err(format!(
                "segments[{idx}].duration_ms must be between 10 and 60000"
            ));
        }
        let end_hz = seg.end_hz.unwrap_or(seg.start_hz);
        if !(16.0..=20_000.0).contains(&seg.start_hz) || !(16.0..=20_000.0).contains(&end_hz) {
            return Err(format!(
                "segments[{idx}] frequencies must be between 16.0 and 20000.0 Hz"
            ));
        }
        let frames = (u64::from(seg.duration_ms) * u64::from(sample_rate) / 1_000).max(1) as usize;
        let fade_frames = ((sample_rate as usize) / 250).min(frames / 4).max(1);
        let phase_offset = (seg.phase_deg / 360.0).rem_euclid(1.0);
        let gain = seg.gain.clamp(0.0, 1.0);

        for frame in 0..frames {
            let progress = frame as f64 / frames as f64;
            let base_hz = seg.start_hz + (end_hz - seg.start_hz) * progress;
            let wow = (lfo_phase * TAU).sin() * (seg.analog_drift_cents / 1_200.0);
            let drifted_hz = base_hz * 2.0_f64.powf(wow);

            let hz_l = drifted_hz.clamp(16.0, 20_000.0);
            let hz_r = (drifted_hz * seg.xy_ratio.clamp(0.125, 8.0) + seg.detune_hz)
                .clamp(16.0, 20_000.0);

            let (raw_l, raw_r) = match seg.waveform.as_str() {
                "sine" => ((phase_l * TAU).sin(), ((phase_r + phase_offset) * TAU).sin()),
                "triangle" => (
                    triangle_wave(phase_l),
                    triangle_wave((phase_r + phase_offset).rem_euclid(1.0)),
                ),
                "sawtooth" => (
                    2.0 * phase_l - 1.0,
                    2.0 * (phase_r + phase_offset).rem_euclid(1.0) - 1.0,
                ),
                "pulse" => (
                    if phase_l < 0.25 { 0.8 } else { -0.8 },
                    if (phase_r + phase_offset).rem_euclid(1.0) < 0.25 {
                        0.8
                    } else {
                        -0.8
                    },
                ),
                "oscilloscope_xy" => {
                    let x = (phase_l * TAU).sin() + 0.22 * (phase_l * 3.0 * TAU).sin();
                    let y = ((phase_r + phase_offset) * TAU).sin()
                        + 0.22 * ((phase_r + phase_offset) * 2.0 * TAU).sin();
                    (x * 0.82, y * 0.82)
                }
                _ => {
                    // "aloboi_warm": warm analog saw + sine fundamental + sub-octave warmth
                    let sub = (phase_sub * TAU).sin() * 0.42;
                    let warm_l = (phase_l * TAU).sin() * 0.55
                        + (phase_l * 2.0 * TAU).sin() * 0.22
                        + (2.0 * phase_l - 1.0) * 0.25
                        + sub;
                    let r_ph = (phase_r + phase_offset).rem_euclid(1.0);
                    let warm_r = (r_ph * TAU).sin() * 0.55
                        + (r_ph * 2.0 * TAU).sin() * 0.22
                        + (2.0 * r_ph - 1.0) * 0.25
                        + sub;
                    (warm_l.tanh(), warm_r.tanh())
                }
            };

            let env = if frame < fade_frames {
                frame as f32 / fade_frames as f32
            } else if frame + fade_frames >= frames {
                (frames - frame) as f32 / fade_frames as f32
            } else {
                1.0
            };

            out.push((raw_l as f32 * gain * env).clamp(-0.98, 0.98));
            out.push((raw_r as f32 * gain * env).clamp(-0.98, 0.98));

            phase_l = (phase_l + hz_l / sr).rem_euclid(1.0);
            phase_r = (phase_r + hz_r / sr).rem_euclid(1.0);
            phase_sub = (phase_sub + (hz_l * 0.5) / sr).rem_euclid(1.0);
            lfo_phase = (lfo_phase + 4.2 / sr).rem_euclid(1.0);
        }
    }
    Ok(out)
}

fn triangle_wave(phase: f64) -> f64 {
    if phase < 0.5 {
        4.0 * phase - 1.0
    } else {
        3.0 - 4.0 * phase
    }
}

#[tool_router(router = wave_tools, vis = "pub(crate)")]
impl SoundBitServer {
    #[tool(
        description = "Synthesizes continuous frequency waves (exact Hz, fractional Hz beating, \
                       continuous Hz glides, Aloboi warm analog tape-drift waves, and XY \
                       Oscilloscope Lissajous stereo waves) directly to a 16-bit stereo WAV file \
                       in the workspace."
    )]
    fn wave_synthesize_hz(&self, Parameters(input): Parameters<WaveSynthesizeInput>) -> ToolResult {
        let path = attempt!(self.workspace().resolve(&input.path, "wav"));
        let sample_rate = input.sample_rate.unwrap_or(48_000);
        let samples = attempt!(synthesize_segments(&input.segments, sample_rate));
        match export_stereo_samples_wav(&path, &samples, sample_rate) {
            Ok(report) => reply(&json!({
                "exported": self.workspace().display(&path),
                "seconds": (report.duration_seconds * 100.0).round() / 100.0,
                "peak": (f64::from(report.peak) * 1_000.0).round() / 1_000.0,
                "rms": (f64::from(report.rms) * 1_000.0).round() / 1_000.0,
                "segments": input.segments.len(),
            })),
            Err(error) => refuse(error.to_string()),
        }
    }
}
