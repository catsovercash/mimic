//! Physical input capture and virtual input playback.
//!
//! `capture` reads evdev devices; `playback` schedules and injects events
//! through `/dev/uinput`. Both are independent of the GUI.

pub mod capture;
pub mod playback;
