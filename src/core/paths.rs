//! XDG paths used by Mimic. Nothing is written outside the user's own
//! configuration/state directories unless the user explicitly chooses a save
//! location.

use std::env;
use std::path::PathBuf;

const APP_DIR: &str = "mimic";

fn home_dir() -> PathBuf {
    env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn xdg_dir(variable: &str, fallback: PathBuf) -> PathBuf {
    env::var_os(variable)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or(fallback)
}

pub fn config_dir() -> PathBuf {
    xdg_dir("XDG_CONFIG_HOME", home_dir().join(".config")).join(APP_DIR)
}

pub fn state_dir() -> PathBuf {
    xdg_dir("XDG_STATE_HOME", home_dir().join(".local/state")).join(APP_DIR)
}

pub fn settings_file() -> PathBuf {
    config_dir().join("settings.json")
}

/// An automatically written copy of the most recent unsaved recording. It is
/// only used for crash recovery; user-visible files are never overwritten.
pub fn recovery_file() -> PathBuf {
    state_dir().join("recovery.mimic")
}

pub fn default_directory() -> PathBuf {
    home_dir()
}

pub fn app_id() -> &'static str {
    "io.github.mimic.Mimic"
}
