//! Recorded input model and the on-disk macro format.
//!
//! Mimic stores the evdev event codes directly. Keyboard translation happens in
//! the compositor, so a recording is independent of the active keyboard layout.
//! Pointer movement is stored as relative deltas, with the absolute cursor
//! position at the start of the recording. Playback first returns the pointer to
//! that start position, so relative movement and clicks line up with what was
//! recorded instead of depending on where the pointer happened to be.

use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::ValidationError;

/// Identifies a Mimic recording. This is deliberately more specific than the
/// file extension so that unrelated JSON files cannot be loaded by accident.
pub const FORMAT_NAME: &str = "mimic-recording";
/// The file schema version understood by this build.
pub const FORMAT_VERSION: u32 = 1;
/// Private extensions should use `<name>.mimic`.
pub const FILE_EXTENSION: &str = "mimic";

/// Linux `KEY_MAX`.
pub const KEY_MAX: u16 = 0x2ff;
/// Linux `BTN_LEFT`.
pub const BTN_LEFT: u16 = 0x110;
/// Linux `BTN_TASK` (last standard mouse button in the contiguous mouse range).
pub const BTN_TASK: u16 = 0x117;

/// An arbitrarily large recording is still bounded so a malicious or corrupt
/// file cannot exhaust memory before validation runs.
pub const MAX_EVENTS: usize = 5_000_000;
/// Seven days of macro time is far beyond normal use and catches timestamp
/// arithmetic mistakes.
pub const MAX_DURATION_US: u64 = 7 * 24 * 60 * 60 * 1_000_000;
/// A hard limit for files accepted by the UI.
pub const MAX_FILE_BYTES: u64 = 512 * 1024 * 1024;

/// Returns true for the standard Linux mouse button range. This covers left,
/// right, middle, side, extra, forward, back, and task buttons.
pub fn is_mouse_button(code: u16) -> bool {
    (BTN_LEFT..=BTN_TASK).contains(&code)
}

/// Returns true for values that can be emitted as a keyboard key.
///
/// Values below `BTN_MISC` (0x100) are ordinary keyboard keys. The `BTN_*`
/// range above that is reserved for pointer/tablet/gamepad buttons. A handful
/// of remote-control and special keys live above `KEY_OK` (0x160), so those
/// are accepted as well.
pub fn is_keyboard_key(code: u16) -> bool {
    (1..0x100).contains(&code) || (0x160..=KEY_MAX).contains(&code)
}

/// Human-readable mouse button name used in status text.
pub fn mouse_button_name(code: u16) -> &'static str {
    match code {
        0x110 => "Left",
        0x111 => "Right",
        0x112 => "Middle",
        0x113 => "Side",
        0x114 => "Extra",
        0x115 => "Forward",
        0x116 => "Back",
        0x117 => "Task",
        _ => "Button",
    }
}

/// A single action captured from an input device.
///
/// The enum is flattened into each event in JSON:
/// `{"at_us": 1234, "type": "key", "code": 30, "pressed": true}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Action {
    /// A keyboard key. `code` is a Linux evdev `KEY_*` value.
    Key { code: u16, pressed: bool },
    /// A mouse button. `code` is a Linux evdev `BTN_*` value.
    Button { code: u16, pressed: bool },
    /// Relative pointer movement accumulated over one kernel input frame.
    Move { dx: i32, dy: i32 },
    /// Mouse wheel movement. Positive `dy` scrolls up, positive `dx` scrolls
    /// right, matching evdev's `REL_WHEEL`/`REL_HWHEEL` convention.
    Scroll { dx: i32, dy: i32 },
}

impl Action {
    /// Short description for diagnostics and the event list.
    pub fn describe(&self) -> String {
        match self {
            Self::Key { code, pressed } => {
                format!("key {} {}", code, if *pressed { "down" } else { "up" })
            }
            Self::Button { code, pressed } => format!(
                "{} mouse {}",
                mouse_button_name(*code),
                if *pressed { "down" } else { "up" }
            ),
            Self::Move { dx, dy } => format!("move {dx:+} {dy:+}"),
            Self::Scroll { dx, dy } => format!("scroll {dx:+} {dy:+}"),
        }
    }
}

/// One timestamped action. Timestamps are relative to the start of the
/// recording and are stored in microseconds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordedEvent {
    pub at_us: u64,
    #[serde(flatten)]
    pub action: Action,
}

impl RecordedEvent {
    pub fn new(at_us: u64, action: Action) -> Self {
        Self { at_us, action }
    }
}

/// Absolute pointer position in KWin's logical workspace coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PointerPoint {
    pub x: i32,
    pub y: i32,
}

/// A complete, versioned recording.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recording {
    pub format: String,
    pub version: u32,
    pub created_unix_ms: u64,
    /// Active recording duration. Time spent paused is excluded.
    pub duration_us: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Cursor position when recording began. Older files do not have this
    /// field and still replay relative to the current pointer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pointer_start: Option<PointerPoint>,
    pub events: Vec<RecordedEvent>,
}

impl Recording {
    /// Creates an empty recording with format metadata filled in.
    pub fn empty() -> Self {
        Self {
            format: FORMAT_NAME.to_owned(),
            version: FORMAT_VERSION,
            created_unix_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            duration_us: 0,
            name: None,
            pointer_start: None,
            events: Vec::new(),
        }
    }

    pub fn event_count(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Duration of the actual event stream. This is what playback needs; the
    /// trailing idle time at the end of a recording is not replayed.
    pub fn event_span_us(&self) -> u64 {
        self.events.last().map_or(0, |event| event.at_us)
    }

    /// Effective loop duration. A macro containing no events still has a
    /// meaningful user-visible duration.
    pub fn playback_span_us(&self) -> u64 {
        if self.events.is_empty() {
            self.duration_us
        } else {
            self.event_span_us()
        }
    }

    /// Sets the final active duration; timestamps must already be normalized.
    pub fn finish(&mut self, duration_us: u64) {
        self.duration_us = duration_us.max(self.event_span_us());
    }

    /// Validate all fields before the recording is loaded, replayed, or saved.
    /// Validation is intentionally strict at the file boundary.
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.format != FORMAT_NAME {
            return Err(ValidationError::BadFormat(self.format.clone()));
        }
        if self.version != FORMAT_VERSION {
            return Err(ValidationError::UnsupportedVersion(self.version));
        }
        if self.events.len() > MAX_EVENTS {
            return Err(ValidationError::TooManyEvents(self.events.len()));
        }
        if self.duration_us > MAX_DURATION_US {
            return Err(ValidationError::DurationTooLong(self.duration_us));
        }

        let mut previous_at = 0u64;
        for (index, event) in self.events.iter().enumerate() {
            if index > 0 && event.at_us < previous_at {
                return Err(ValidationError::NonMonotonic {
                    index,
                    previous: previous_at,
                    found: event.at_us,
                });
            }
            previous_at = event.at_us;

            if event.at_us > MAX_DURATION_US {
                return Err(ValidationError::TimestampTooLarge {
                    index,
                    value: event.at_us,
                });
            }
            match event.action {
                Action::Key { code, .. } => {
                    if !is_keyboard_key(code) {
                        return Err(ValidationError::BadKeyCode { index, code });
                    }
                }
                Action::Button { code, .. } => {
                    if !is_mouse_button(code) {
                        return Err(ValidationError::BadButtonCode { index, code });
                    }
                }
                Action::Move { dx, dy } => {
                    if dx == 0 && dy == 0 {
                        return Err(ValidationError::EmptyMove { index });
                    }
                    if dx.unsigned_abs() > 100_000 || dy.unsigned_abs() > 100_000 {
                        return Err(ValidationError::MovementOutOfRange { index, dx, dy });
                    }
                }
                Action::Scroll { dx, dy } => {
                    if dx == 0 && dy == 0 {
                        return Err(ValidationError::EmptyScroll { index });
                    }
                    if dx.unsigned_abs() > 1_000_000 || dy.unsigned_abs() > 1_000_000 {
                        return Err(ValidationError::ScrollOutOfRange { index, dx, dy });
                    }
                }
            }
        }

        if self
            .events
            .last()
            .is_some_and(|last| last.at_us > self.duration_us)
        {
            return Err(ValidationError::DurationShorterThanEvents {
                duration: self.duration_us,
                last: self.event_span_us(),
            });
        }

        Ok(())
    }

    /// Best-effort name for window titles and save dialogs.
    pub fn display_name(&self) -> String {
        self.name
            .clone()
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| "Untitled recording".to_owned())
    }
}

/// Formats a duration in microseconds as `MM:SS.mmm` (hours are shown when
/// needed). Used consistently by the timer and help text.
pub fn format_duration(us: u64) -> String {
    let total_ms = us / 1_000;
    let millis = total_ms % 1_000;
    let total_secs = total_ms / 1_000;
    let secs = total_secs % 60;
    let total_minutes = total_secs / 60;
    let minutes = total_minutes % 60;
    let hours = total_minutes / 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{secs:02}.{millis:03}")
    } else {
        format!("{minutes:02}:{secs:02}.{millis:03}")
    }
}

#[cfg(test)]
mod tests;
