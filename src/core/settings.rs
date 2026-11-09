//! Persistent user preferences. Settings are saved atomically so an
//! interrupted write cannot leave an unreadable configuration file.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::paths;

pub const MIN_SPEED: f32 = 0.1;
pub const MAX_SPEED: f32 = 5.0;
pub const MAX_LOOP_COUNT: u32 = 9_999;
pub const MAX_LOOP_DELAY_MS: u64 = 60_000;

/// Application settings. `loop_count == 0` means infinite looping.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub playback_speed: f32,
    pub loop_enabled: bool,
    pub loop_count: u32,
    pub loop_delay_ms: u64,
    pub last_directory: PathBuf,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            playback_speed: 1.0,
            loop_enabled: false,
            loop_count: 0,
            loop_delay_ms: 500,
            last_directory: paths::default_directory(),
        }
    }
}

impl Settings {
    /// Clamp values loaded from disk or modified by the UI into supported
    /// ranges. This keeps the playback scheduler predictable.
    pub fn sanitized(mut self) -> Self {
        if !self.playback_speed.is_finite() {
            self.playback_speed = 1.0;
        }
        self.playback_speed = self.playback_speed.clamp(MIN_SPEED, MAX_SPEED);
        self.loop_count = self.loop_count.min(MAX_LOOP_COUNT);
        self.loop_delay_ms = self.loop_delay_ms.min(MAX_LOOP_DELAY_MS);
        if self.last_directory.as_os_str().is_empty() {
            self.last_directory = paths::default_directory();
        }
        self
    }

    /// Number of loop passes to play. `None` means loop forever.
    pub fn loop_passes(&self) -> Option<u32> {
        if !self.loop_enabled {
            Some(1)
        } else if self.loop_count == 0 {
            None
        } else {
            Some(self.loop_count.max(1))
        }
    }

    pub fn loop_delay(&self) -> std::time::Duration {
        std::time::Duration::from_millis(self.loop_delay_ms)
    }

    pub fn speed(&self) -> f64 {
        f64::from(self.playback_speed.clamp(MIN_SPEED, MAX_SPEED))
    }

    /// Loads settings. A missing file is not an error. On parse failure the
    /// defaults are returned together with a warning for the UI.
    pub fn load_from(path: &Path) -> (Self, Option<String>) {
        match fs::read(path) {
            Ok(bytes) => match serde_json::from_slice::<Self>(&bytes) {
                Ok(settings) => (settings.sanitized(), None),
                Err(error) => (
                    Self::default(),
                    Some(format!(
                        "Could not read settings ({}). Using defaults.",
                        error
                    )),
                ),
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (Self::default(), None),
            Err(error) => (
                Self::default(),
                Some(format!(
                    "Could not access {}. Using defaults. ({error})",
                    path.display()
                )),
            ),
        }
    }

    pub fn save_to(&self, path: &Path) -> Result<(), String> {
        let sanitized = self.clone().sanitized();
        crate::storage::write_json_atomic(path, &sanitized)
            .map_err(|error| format!("Could not save settings: {error}"))
    }
}

#[cfg(test)]
mod tests;
