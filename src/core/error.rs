//! Errors shown to the user. Messages are deliberately actionable: when a
//! permission or portal error occurs, the UI can tell the user how to fix it.

use std::io;
use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum ValidationError {
    #[error("this is not a Mimic recording (format is '{0}')")]
    BadFormat(String),
    #[error("unsupported recording version {0}; this build supports version 1")]
    UnsupportedVersion(u32),
    #[error("recording contains too many events ({0})")]
    TooManyEvents(usize),
    #[error("recording duration is too long ({0} µs)")]
    DurationTooLong(u64),
    #[error("event {index} goes backwards in time (previous {previous} µs, found {found} µs)")]
    NonMonotonic {
        index: usize,
        previous: u64,
        found: u64,
    },
    #[error("event {index} has an invalid timestamp ({value} µs)")]
    TimestampTooLarge { index: usize, value: u64 },
    #[error("event {index} has an invalid key code ({code})")]
    BadKeyCode { index: usize, code: u16 },
    #[error("event {index} has an invalid mouse button code ({code})")]
    BadButtonCode { index: usize, code: u16 },
    #[error("event {index} contains an empty pointer movement")]
    EmptyMove { index: usize },
    #[error("event {index} contains unreasonable pointer movement ({dx}, {dy})")]
    MovementOutOfRange { index: usize, dx: i32, dy: i32 },
    #[error("event {index} contains an empty scroll event")]
    EmptyScroll { index: usize },
    #[error("event {index} contains unreasonable scroll movement ({dx}, {dy})")]
    ScrollOutOfRange { index: usize, dx: i32, dy: i32 },
    #[error("recording duration ({duration} µs) is shorter than its final event ({last} µs)")]
    DurationShorterThanEvents { duration: u64, last: u64 },
}

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("could not access '{path}': {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("the file is too large to be a Mimic recording ({size} bytes)")]
    TooLarge { size: u64 },
    #[error("malformed recording: {0}")]
    Malformed(#[from] serde_json::Error),
    #[error("{0}")]
    Invalid(#[from] ValidationError),
}

impl StorageError {
    pub fn io(path: impl Into<PathBuf>, source: io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}

/// Error raised by the evdev/uinput backend.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum InputError {
    #[error(
        "Mimic cannot read input devices. Grant access with the provided udev rule, then log out and back in."
    )]
    NoCapturePermission,
    #[error("no usable keyboard or mouse was found")]
    NoDevices,
    #[error("input device error: {0}")]
    Device(String),
    #[error(
        "Mimic cannot create a virtual input device. Grant access to /dev/uinput with the provided udev rule."
    )]
    NoInjectionPermission,
    #[error("could not create the virtual input device: {0}")]
    Injection(String),
    #[error("recording contains no events to play")]
    EmptyRecording,
}
