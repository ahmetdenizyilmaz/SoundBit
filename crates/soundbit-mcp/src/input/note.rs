//! Notes written by name.

use schemars::JsonSchema;
use serde::Deserialize;
use soundbit_app::command::NoteKey;
use soundbit_app::model::{Note, Pitch, Velocity};

/// A note to write into a pattern.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub(crate) struct NoteInput {
    /// Note name with octave ("C4" is middle C; also "F#3", "Bb5") or exact frequency string ("440Hz", "432.5Hz").
    pub(crate) note: String,
    /// First step of the note, counted from 0.
    pub(crate) start: u16,
    /// Duration in steps. Defaults to 1.
    #[serde(default = "one_step")]
    pub(crate) length: u16,
    /// MIDI velocity 1-127. Defaults to 100.
    pub(crate) velocity: Option<u8>,
    /// Exact target frequency in hertz (20.0-20000.0). Overrides the pitch's base 12-TET frequency for micro-Hz / Aloboi / Oscilloscope tuning.
    pub(crate) hz: Option<f64>,
    /// Fine frequency offset in hertz (-2000.0 to 2000.0) added to the note's frequency (for binaural beats, Aloboi drift, and Oscilloscope phase rotation).
    pub(crate) detune_hz: Option<f64>,
    /// Microtonal pitch offset in cents (-4800 to 4800; 100 cents = 1 semitone).
    pub(crate) detune_cents: Option<i16>,
    /// Continuous linear pitch glide in semitones (-48 to 48) over the duration of the note (positive glides up for zoom-in/inflation, negative glides down for zoom-out/shrinking).
    pub(crate) slide_semitones: Option<i8>,
}

fn one_step() -> u16 {
    1
}

/// Converts a frequency in Hz to `(closest Pitch, residual millihertz)`.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "clamped to valid MIDI pitch and i32 millihertz ranges"
)]
fn hz_to_pitch_and_milli(target_hz: f64) -> Result<(Pitch, i32), String> {
    if !target_hz.is_finite() || !(16.0..=20_000.0).contains(&target_hz) {
        return Err(format!(
            "hz must be between 16.0 and 20000.0 (got {target_hz})"
        ));
    }
    let midi_float = 69.0 + 12.0 * (target_hz / 440.0).log2();
    let midi_clamped = midi_float.round().clamp(0.0, 127.0) as u8;
    let pitch = Pitch::new(midi_clamped).map_err(|error| error.to_string())?;
    let base_hz = 440.0 * 2.0_f64.powf((f64::from(midi_clamped) - 69.0) / 12.0);
    let diff_milli = ((target_hz - base_hz) * 1_000.0)
        .round()
        .clamp(-2_000_000.0, 2_000_000.0) as i32;
    Ok((pitch, diff_milli))
}

fn parse_hz_suffix(text: &str) -> Option<f64> {
    let trimmed = text.trim();
    let number_part = trimmed
        .strip_suffix("Hz")
        .or_else(|| trimmed.strip_suffix("hz"))
        .or_else(|| trimmed.strip_suffix("HZ"))?;
    number_part.trim().parse::<f64>().ok()
}

impl NoteInput {
    #[allow(
        clippy::cast_possible_truncation,
        reason = "detune_hz is validated and clamped before converting to millihertz"
    )]
    fn to_note(&self) -> Result<Note, String> {
        let (pitch, mut milli_hz) = if let Some(explicit_hz) = self.hz {
            let (inferred_pitch, residual_milli) = hz_to_pitch_and_milli(explicit_hz)?;
            let chosen_pitch = if self.note.trim().is_empty() || parse_hz_suffix(&self.note).is_some()
            {
                inferred_pitch
            } else {
                self.note.parse().map_err(|error| format!("{error}"))?
            };
            let base_hz = 440.0 * 2.0_f64.powf((f64::from(chosen_pitch.get()) - 69.0) / 12.0);
            let diff = ((explicit_hz - base_hz) * 1_000.0)
                .round()
                .clamp(-2_000_000.0, 2_000_000.0) as i32;
            let _ = residual_milli;
            (chosen_pitch, diff)
        } else if let Some(parsed_hz) = parse_hz_suffix(&self.note) {
            hz_to_pitch_and_milli(parsed_hz)?
        } else {
            let pitch: Pitch = self.note.parse().map_err(|error| format!("{error}"))?;
            (pitch, 0)
        };

        if let Some(detune_hz) = self.detune_hz {
            if !detune_hz.is_finite() || !(-2_000.0..=2_000.0).contains(&detune_hz) {
                return Err(format!(
                    "detune_hz must be between -2000.0 and 2000.0 (got {detune_hz})"
                ));
            }
            milli_hz = milli_hz.saturating_add((detune_hz * 1_000.0).round() as i32);
        }

        let mut note =
            Note::new(pitch, self.start, self.length).map_err(|error| error.to_string())?;
        if let Some(velocity) = self.velocity {
            note = note.with_velocity(Velocity::new(velocity).map_err(|error| error.to_string())?);
        }
        if let Some(cents) = self.detune_cents {
            if !(-4_800..=4_800).contains(&cents) {
                return Err(format!(
                    "detune_cents must be between -4800 and 4800 (got {cents})"
                ));
            }
            note = note.with_detune_cents(cents);
        }
        if milli_hz != 0 {
            note = note.with_detune_milli_hz(milli_hz);
        }
        if let Some(slide) = self.slide_semitones {
            if !(-48..=48).contains(&slide) {
                return Err(format!(
                    "slide_semitones must be between -48 and 48 (got {slide})"
                ));
            }
            note = note.with_slide_semitones(slide);
        }
        Ok(note)
    }
}

/// Converts notes, naming the position of the first invalid one.
pub(crate) fn notes(inputs: &[NoteInput]) -> Result<Vec<Note>, String> {
    inputs
        .iter()
        .enumerate()
        .map(|(index, input)| {
            input
                .to_note()
                .map_err(|error| format!("notes[{index}]: {error}"))
        })
        .collect()
}

/// Identifies an existing note.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub(crate) struct NoteRef {
    /// Note name with octave, e.g. "C4".
    pub(crate) note: String,
    /// Start step of the note.
    pub(crate) start: u16,
}

impl NoteRef {
    pub(crate) fn to_key(&self) -> Result<NoteKey, String> {
        let pitch = self
            .note
            .parse()
            .map_err(|error: soundbit_app::model::ModelError| error.to_string())?;
        Ok(NoteKey {
            start: self.start,
            pitch,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(note: &str, start: u16, length: u16, velocity: Option<u8>) -> NoteInput {
        NoteInput {
            note: note.to_owned(),
            start,
            length,
            velocity,
            hz: None,
            detune_hz: None,
            detune_cents: None,
            slide_semitones: None,
        }
    }

    #[test]
    fn converts_valid_notes() {
        let converted = notes(&[input("A4", 2, 3, Some(90))]).expect("valid");
        assert_eq!(converted[0].pitch.get(), 69);
        assert_eq!((converted[0].start, converted[0].length.get()), (2, 3));
        assert_eq!(converted[0].velocity.get(), 90);
    }

    #[test]
    fn errors_name_the_note() {
        let error = notes(&[input("C4", 0, 1, None), input("H2", 0, 1, None)]).expect_err("bad");
        assert!(
            error.starts_with("notes[1]: invalid note name `H2`"),
            "{error}"
        );
        let error = notes(&[input("C4", 0, 0, None)]).expect_err("zero length");
        assert!(error.contains("at least 1 step"), "{error}");
    }

    #[test]
    fn length_defaults_to_one_step() {
        let note: NoteInput = serde_json::from_str(r#"{"note": "C4", "start": 0}"#).expect("valid");
        assert_eq!(note.length, 1);
    }
}
