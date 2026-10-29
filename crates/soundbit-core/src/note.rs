//! Notes placed on a pattern's step grid.

use std::num::NonZeroU16;

use serde::{Deserialize, Serialize};

use crate::ranged::ranged;
use crate::{ModelError, Pitch};

ranged! {
    /// How hard a note is played, as a MIDI velocity.
    Velocity(u8): 1..=127, default 100, "velocity"
}

/// A note in a pattern. Time is measured in steps from the start of the pattern.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Note {
    /// Pitch to play.
    pub pitch: Pitch,
    /// First step of the note, counted from 0.
    pub start: u16,
    /// Duration in steps.
    pub length: NonZeroU16,
    /// Velocity.
    #[serde(default)]
    pub velocity: Velocity,
    /// Microtonal / fine pitch offset in cents (100 cents = 1 semitone).
    #[serde(default, skip_serializing_if = "is_zero_i16")]
    pub detune_cents: i16,
    /// Fine frequency offset in millihertz (1000 = +1.000 Hz) for Aloboi / Oscilloscope beating and exact Hz tuning.
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub detune_milli_hz: i32,
    /// Continuous linear pitch glide in semitones across the note's duration (e.g. +12 for zoom-in/inflation, -12 for zoom-out/shrink).
    #[serde(default, skip_serializing_if = "is_zero_i8")]
    pub slide_semitones: i8,
}

const fn is_zero_i16(val: &i16) -> bool {
    *val == 0
}

const fn is_zero_i32(val: &i32) -> bool {
    *val == 0
}

const fn is_zero_i8(val: &i8) -> bool {
    *val == 0
}

impl Note {
    /// Creates a note with the default velocity.
    ///
    /// # Errors
    ///
    /// Returns [`ModelError::ZeroLengthNote`] if `length` is 0.
    pub fn new(pitch: Pitch, start: u16, length: u16) -> Result<Self, ModelError> {
        let length = NonZeroU16::new(length).ok_or(ModelError::ZeroLengthNote)?;
        Ok(Self {
            pitch,
            start,
            length,
            velocity: Velocity::default(),
            detune_cents: 0,
            detune_milli_hz: 0,
            slide_semitones: 0,
        })
    }

    /// Returns the note with a different velocity.
    #[must_use]
    pub const fn with_velocity(self, velocity: Velocity) -> Self {
        Self { velocity, ..self }
    }

    /// Returns the note with a microtonal pitch offset in cents.
    #[must_use]
    pub const fn with_detune_cents(self, detune_cents: i16) -> Self {
        Self {
            detune_cents,
            ..self
        }
    }

    /// Returns the note with a fine frequency offset in millihertz (1000 = +1.0 Hz).
    #[must_use]
    pub const fn with_detune_milli_hz(self, detune_milli_hz: i32) -> Self {
        Self {
            detune_milli_hz,
            ..self
        }
    }

    /// Returns the note with a continuous pitch slide in semitones over its duration.
    #[must_use]
    pub const fn with_slide_semitones(self, slide_semitones: i8) -> Self {
        Self {
            slide_semitones,
            ..self
        }
    }

    /// The step right after the note ends.
    pub fn end(&self) -> u32 {
        u32::from(self.start) + u32::from(self.length.get())
    }

    /// Whether the two notes sound at the same time at any step.
    pub fn overlaps_in_time(&self, other: &Self) -> bool {
        u32::from(self.start) < other.end() && u32::from(other.start) < self.end()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(start: u16, length: u16) -> Note {
        Note::new(Pitch::default(), start, length).expect("non-zero length")
    }

    #[test]
    fn rejects_zero_length() {
        assert_eq!(
            Note::new(Pitch::default(), 0, 0),
            Err(ModelError::ZeroLengthNote)
        );
    }

    #[test]
    fn adjacent_notes_do_not_overlap() {
        assert!(!note(0, 4).overlaps_in_time(&note(4, 4)));
        assert!(note(0, 5).overlaps_in_time(&note(4, 4)));
        assert!(note(2, 1).overlaps_in_time(&note(0, 8)));
    }
}
