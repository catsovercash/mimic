//! Mimic — a compact TinyTask-style input recorder for KDE Plasma Wayland.
//!
//! The crate is split into four layers:
//!
//! - [`core`]: recording model, validation, storage, settings, and errors
//! - [`input`]: evdev capture and uinput playback
//! - [`platform`]: KWin and XDG desktop portal integration
//! - [`ui`]: egui application, controller, and theme
//!
//! Recording and playback live below the GUI so both can be tested and reused
//! without starting a window. The backend reads physical evdev devices and
//! emits through a uinput virtual device; it does not use X11.

pub mod core;
pub mod input;
pub mod platform;
pub mod ui;

// Keep the original flat module paths available for callers and for the
// binary's imports. Internally, prefer the canonical paths above.
pub use crate::core::{error, model, paths, settings, storage};
pub use crate::input::{capture, playback};
pub use crate::platform::{cursor, hotkeys};
pub use crate::ui::{app, controller, theme};
