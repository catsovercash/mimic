//! KDE Plasma / Wayland integration.
//!
//! These modules talk to KWin and XDG desktop portals. They are kept separate
//! from the input backend and the GUI so platform changes stay contained.

pub mod cursor;
pub mod hotkeys;
