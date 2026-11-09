//! Application controller.
//!
//! The GUI is a view over a shared snapshot. All state transitions and input
//! workers run on this controller thread, which keeps the UI responsive and
//! prevents overlapping recording/playback sessions.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use eframe::egui;

use crate::capture::{self, CaptureCommand, CaptureUpdate};
use crate::error::StorageError;
use crate::hotkeys::{self, HotkeyAction, HotkeyService, HotkeyUiState, HotkeyUpdate};
use crate::model::{format_duration, Recording};
use crate::paths;
use crate::playback::{self, PlaybackCommand, PlaybackOptions, PlaybackUpdate};
use crate::settings::Settings;
use crate::storage;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Idle,
    Recording,
    Playing,
}

impl Mode {
    pub fn is_busy(self) -> bool {
        self != Self::Idle
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BannerLevel {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Banner {
    pub id: u64,
    pub level: BannerLevel,
    pub title: String,
    pub detail: Option<String>,
}

/// Everything the render thread needs. It is a strict clone: the GUI never
/// blocks on backend state.
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub mode: Mode,
    pub paused: bool,
    pub elapsed_us: u64,
    pub event_count: usize,
    pub macro_event_count: usize,
    pub macro_duration_us: u64,
    pub macro_name: String,
    pub macro_path: Option<PathBuf>,
    pub dirty: bool,
    pub recovered: bool,
    pub settings: Settings,
    pub devices: Vec<String>,
    pub playback_pass: u32,
    pub playback_passes: Option<u32>,
    pub hotkeys: HotkeyUiState,
    pub dialog_pending: bool,
    pub banner: Option<Banner>,
    pub notice: Option<String>,
}

#[derive(Debug, Clone)]
pub enum UiCommand {
    StartRecording,
    PauseResume,
    Stop,
    Play,
    ToggleLoop,
    ApplySettings(Settings),
    Save,
    Open,
    ConfigureShortcuts,
    RetryHotkeys,
    DismissBanner(u64),
    Shutdown,
}

#[derive(Debug)]
enum DialogOutcome {
    Save(SaveOutcome),
    Open(OpenOutcome),
}

#[derive(Debug)]
enum SaveOutcome {
    Cancelled,
    Saved(PathBuf),
    Failed(String),
}

#[derive(Debug)]
enum OpenOutcome {
    Cancelled,
    Loaded(PathBuf, Recording),
    Failed(String),
}

enum Message {
    Ui(UiCommand),
    Capture(CaptureUpdate),
    Playback(PlaybackUpdate),
    Hotkeys(HotkeyUpdate),
    Dialog(DialogOutcome),
}

#[derive(Clone)]
pub struct ControllerHandle {
    tx: Sender<Message>,
    snapshot: Arc<Mutex<Snapshot>>,
}

impl ControllerHandle {
    pub fn send(&self, command: UiCommand) {
        let _ = self.tx.send(Message::Ui(command));
    }

    pub fn snapshot(&self) -> Snapshot {
        self.snapshot
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub fn shutdown(&self) {
        self.send(UiCommand::Shutdown);
    }
}

impl Drop for ControllerHandle {
    fn drop(&mut self) {
        let _ = self.tx.send(Message::Ui(UiCommand::Shutdown));
    }
}

pub fn spawn(repaint: egui::Context, initial_file: Option<PathBuf>) -> ControllerHandle {
    let snapshot = Arc::new(Mutex::new(initial_snapshot()));
    let (tx, rx) = mpsc::channel();
    let handle = ControllerHandle {
        tx: tx.clone(),
        snapshot: snapshot.clone(),
    };

    thread::Builder::new()
        .name("mimic-controller".into())
        .spawn(move || {
            let mut controller = Controller::new(tx, snapshot, repaint, initial_file);
            controller.run(rx);
        })
        .expect("failed to spawn controller thread");

    handle
}

#[derive(Default)]
struct StartupState {
    recording: Option<Recording>,
    path: Option<PathBuf>,
    banner: Option<(BannerLevel, String, Option<String>)>,
    notice: Option<String>,
    recovered: bool,
}

struct Controller {
    tx: Sender<Message>,
    snapshot: Arc<Mutex<Snapshot>>,
    repaint: egui::Context,
    settings: Settings,
    recording: Option<Recording>,
    path: Option<PathBuf>,
    dirty: bool,
    recovered: bool,
    mode: Mode,
    paused: bool,
    elapsed_us: u64,
    live_event_count: usize,
    devices: Vec<String>,
    playback_pass: u32,
    playback_passes: Option<u32>,
    capture_commands: Option<Sender<CaptureCommand>>,
    capture_handle: Option<JoinHandle<()>>,
    playback_commands: Option<Sender<PlaybackCommand>>,
    playback_handle: Option<JoinHandle<()>>,
    hotkey_service: Option<HotkeyService>,
    hotkey_state: HotkeyUiState,
    dialog_pending: bool,
    banner: Option<Banner>,
    banner_serial: u64,
    notice: Option<String>,
    shutting_down: bool,
    recovery_warning_shown: bool,
}

impl Controller {
    fn new(
        tx: Sender<Message>,
        snapshot: Arc<Mutex<Snapshot>>,
        repaint: egui::Context,
        initial_file: Option<PathBuf>,
    ) -> Self {
        let (settings, settings_warning) = Settings::load_from(&paths::settings_file());
        let settings_warning = settings_warning
            .map(|warning| (BannerLevel::Warning, "Settings".to_owned(), Some(warning)));
        let StartupState {
            recording,
            path,
            banner: startup_banner,
            notice,
            recovered,
        } = Self::load_startup(initial_file.as_deref());
        let elapsed_us = recording
            .as_ref()
            .map_or(0, |recording| recording.duration_us);

        let mut controller = Self {
            tx,
            snapshot,
            repaint,
            settings,
            recording,
            path,
            dirty: recovered,
            recovered,
            mode: Mode::Idle,
            paused: false,
            elapsed_us,
            live_event_count: 0,
            devices: Vec::new(),
            playback_pass: 1,
            playback_passes: None,
            capture_commands: None,
            capture_handle: None,
            playback_commands: None,
            playback_handle: None,
            hotkey_service: None,
            hotkey_state: HotkeyUiState::Starting,
            dialog_pending: false,
            banner: None,
            banner_serial: 0,
            notice,
            shutting_down: false,
            recovery_warning_shown: false,
        };

        if let Some((level, title, detail)) = settings_warning {
            controller.set_banner(level, title, detail);
        }
        if let Some((level, title, detail)) = startup_banner {
            // Startup warnings should not replace an explicit settings warning.
            if controller.banner.is_none() {
                controller.set_banner(level, title, detail);
            }
        }
        controller.update_title();
        controller.start_hotkeys();
        controller.publish();
        controller
    }

    fn load_startup(initial_file: Option<&Path>) -> StartupState {
        if let Some(path) = initial_file {
            match storage::load_recording(path) {
                Ok(recording) => {
                    return StartupState {
                        recording: Some(recording),
                        path: Some(path.to_owned()),
                        banner: None,
                        notice: Some(format!("Opened {}", display_path(path))),
                        recovered: false,
                    };
                }
                Err(error) => {
                    return StartupState {
                        recording: None,
                        path: None,
                        banner: Some((
                            BannerLevel::Error,
                            "Could not open recording".into(),
                            Some(format!("{}: {error}", path.display())),
                        )),
                        notice: None,
                        recovered: false,
                    };
                }
            }
        }

        let recovery_path = paths::recovery_file();
        match storage::load_recording(&recovery_path) {
            Ok(recording) if !recording.is_empty() => StartupState {
                recording: Some(recording),
                path: None,
                banner: Some((
                    BannerLevel::Info,
                    "Recovered unsaved recording".into(),
                    Some(
                        "Mimic found a recent recording that was not explicitly saved. \
                         Use Save to keep it, or start a new recording when ready."
                            .into(),
                    ),
                )),
                notice: Some("Recovered the previous session's unsaved recording.".into()),
                recovered: true,
            },
            Ok(_) => {
                let _ = fs::remove_file(recovery_path);
                StartupState::default()
            }
            Err(StorageError::Io { .. }) => StartupState::default(),
            Err(error) => StartupState {
                recording: None,
                path: None,
                banner: Some((
                    BannerLevel::Warning,
                    "Recovery file could not be read".into(),
                    Some(error.to_string()),
                )),
                notice: None,
                recovered: false,
            },
        }
    }

    fn run(&mut self, messages: Receiver<Message>) {
        loop {
            match messages.recv_timeout(Duration::from_secs(1)) {
                Ok(Message::Ui(command)) => {
                    if self.handle_ui(command) {
                        break;
                    }
                }
                Ok(Message::Capture(update)) => self.handle_capture(update),
                Ok(Message::Playback(update)) => self.handle_playback(update),
                Ok(Message::Hotkeys(update)) => self.handle_hotkeys(update),
                Ok(Message::Dialog(outcome)) => self.handle_dialog(outcome),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        self.shutdown(&messages);
    }

    fn handle_ui(&mut self, command: UiCommand) -> bool {
        match command {
            UiCommand::StartRecording => self.start_recording(),
            UiCommand::PauseResume => self.toggle_pause(),
            UiCommand::Stop => self.stop_active(),
            UiCommand::Play => self.start_playback(),
            UiCommand::ToggleLoop => {
                if !self.mode.is_busy() && !self.dialog_pending {
                    let mut settings = self.settings.clone();
                    settings.loop_enabled = !settings.loop_enabled;
                    self.apply_settings(settings);
                }
            }
            UiCommand::ApplySettings(settings) => {
                if !self.mode.is_busy() && !self.dialog_pending {
                    self.apply_settings(settings);
                }
            }
            UiCommand::Save => self.start_save_dialog(),
            UiCommand::Open => self.start_open_dialog(),
            UiCommand::ConfigureShortcuts => {
                if let Some(service) = &self.hotkey_service {
                    service.configure();
                    self.notice = Some("Opening KDE shortcut settings…".into());
                    self.publish();
                }
            }
            UiCommand::RetryHotkeys => {
                if matches!(self.hotkey_state, HotkeyUiState::Unavailable(_)) {
                    if let Some(service) = self.hotkey_service.take() {
                        service.shutdown();
                    }
                    self.hotkey_state = HotkeyUiState::Starting;
                    self.start_hotkeys();
                    self.publish();
                }
            }
            UiCommand::DismissBanner(id) => {
                if self.banner.as_ref().is_some_and(|banner| banner.id == id) {
                    self.banner = None;
                    self.publish();
                }
            }
            UiCommand::Shutdown => {
                self.shutting_down = true;
                return true;
            }
        }
        false
    }

    fn start_recording(&mut self) {
        if self.mode.is_busy() || self.dialog_pending || self.shutting_down {
            return;
        }
        self.mode = Mode::Recording;
        self.paused = false;
        self.elapsed_us = 0;
        self.live_event_count = 0;
        self.devices.clear();
        self.notice = Some("Recording input…".into());
        self.banner = self.banner.take().filter(|b| b.level == BannerLevel::Info);

        let (command_tx, command_rx) = mpsc::channel();
        let sender = self.tx.clone();
        let handle = capture::spawn(
            move |update| {
                let _ = sender.send(Message::Capture(update));
            },
            command_rx,
        );
        self.capture_commands = Some(command_tx);
        self.capture_handle = Some(handle);
        self.publish();
    }

    fn toggle_pause(&mut self) {
        match self.mode {
            Mode::Recording => {
                if let Some(commands) = &self.capture_commands {
                    let _ = commands.send(if self.paused {
                        CaptureCommand::Resume
                    } else {
                        CaptureCommand::Pause
                    });
                }
            }
            Mode::Playing => {
                if let Some(commands) = &self.playback_commands {
                    let _ = commands.send(if self.paused {
                        PlaybackCommand::Resume
                    } else {
                        PlaybackCommand::Pause
                    });
                }
            }
            Mode::Idle => {}
        }
    }

    fn stop_active(&mut self) {
        match self.mode {
            Mode::Recording => {
                if let Some(commands) = &self.capture_commands {
                    let _ = commands.send(CaptureCommand::Stop);
                    self.notice = Some("Stopping recording…".into());
                    self.publish();
                }
            }
            Mode::Playing => {
                if let Some(commands) = &self.playback_commands {
                    let _ = commands.send(PlaybackCommand::Stop);
                }
            }
            Mode::Idle => {}
        }
    }

    fn start_playback(&mut self) {
        if self.mode.is_busy() || self.dialog_pending {
            return;
        }
        let Some(recording) = self
            .recording
            .clone()
            .filter(|recording| !recording.is_empty())
        else {
            self.notice = Some("Record something or open a macro before playing.".into());
            self.publish();
            return;
        };

        let options = PlaybackOptions::new(
            self.settings.speed(),
            self.settings.loop_passes(),
            self.settings.loop_delay(),
        );
        self.mode = Mode::Playing;
        self.paused = false;
        self.elapsed_us = 0;
        self.playback_pass = 1;
        self.playback_passes = options.passes;
        self.notice = Some("Playing…".into());

        let (command_tx, command_rx) = mpsc::channel();
        let sender = self.tx.clone();
        let handle = playback::spawn(
            recording,
            options,
            move |update| {
                let _ = sender.send(Message::Playback(update));
            },
            command_rx,
        );
        self.playback_commands = Some(command_tx);
        self.playback_handle = Some(handle);
        self.publish();
    }

    fn apply_settings(&mut self, settings: Settings) {
        self.settings = settings.sanitized();
        if let Err(error) = self.settings.save_to(&paths::settings_file()) {
            self.set_banner(BannerLevel::Warning, "Could not save settings", Some(error));
        } else {
            self.notice = Some("Settings saved.".into());
        }
        self.publish();
    }

    fn handle_capture(&mut self, update: CaptureUpdate) {
        match update {
            CaptureUpdate::Started { devices } => {
                self.devices = devices;
                self.notice = Some("Recording input…".into());
            }
            CaptureUpdate::Progress {
                elapsed_us,
                event_count,
            } => {
                self.elapsed_us = elapsed_us;
                self.live_event_count = event_count;
            }
            CaptureUpdate::Paused {
                elapsed_us,
                event_count,
            } => {
                self.paused = true;
                self.elapsed_us = elapsed_us;
                self.live_event_count = event_count;
            }
            CaptureUpdate::Resumed => {
                self.paused = false;
            }
            CaptureUpdate::Warning(message) => {
                self.set_banner(BannerLevel::Warning, "Input warning", Some(message));
            }
            CaptureUpdate::Checkpoint(snapshot) => {
                if let Err(error) = storage::save_recording(&paths::recovery_file(), &snapshot) {
                    if !self.recovery_warning_shown {
                        self.recovery_warning_shown = true;
                        self.set_banner(
                            BannerLevel::Warning,
                            "Could not update crash recovery",
                            Some(error.to_string()),
                        );
                    }
                }
                return;
            }
            CaptureUpdate::Stopped(recording) => {
                let count = recording.event_count();
                let duration = recording.duration_us;
                self.mode = Mode::Idle;
                self.paused = false;
                self.devices.clear();
                self.live_event_count = 0;
                self.elapsed_us = duration;
                let was_empty = recording.is_empty();
                self.recording = Some(recording);
                self.path = None;
                self.dirty = !was_empty;
                self.recovered = false;
                self.capture_commands = None;
                if let Some(handle) = self.capture_handle.take() {
                    let _ = handle.join();
                }
                if was_empty {
                    self.notice = Some("No input was recorded.".into());
                } else {
                    self.notice = Some(format!(
                        "Recorded {count} events in {}.",
                        format_duration(duration)
                    ));
                    if let Err(error) = self.write_recovery() {
                        self.set_banner(
                            BannerLevel::Warning,
                            "Recording could not be backed up",
                            Some(error),
                        );
                    }
                }
                self.update_title();
            }
            CaptureUpdate::Failed(error) => {
                self.mode = Mode::Idle;
                self.paused = false;
                self.devices.clear();
                self.live_event_count = 0;
                self.capture_commands = None;
                if let Some(handle) = self.capture_handle.take() {
                    let _ = handle.join();
                }
                self.elapsed_us = self
                    .recording
                    .as_ref()
                    .map_or(0, |recording| recording.duration_us);
                self.notice = Some("Recording could not start.".into());
                self.set_banner(BannerLevel::Error, "Could not start recording", Some(error));
            }
        }
        self.publish();
    }

    fn handle_playback(&mut self, update: PlaybackUpdate) {
        match update {
            PlaybackUpdate::Started { passes } => {
                self.playback_passes = passes;
                self.notice = Some("Playing…".into());
            }
            PlaybackUpdate::Progress {
                position_us,
                pass,
                passes,
            } => {
                self.elapsed_us = position_us;
                self.playback_pass = pass;
                self.playback_passes = passes;
            }
            PlaybackUpdate::Paused { position_us } => {
                self.paused = true;
                self.elapsed_us = position_us;
            }
            PlaybackUpdate::Resumed => {
                self.paused = false;
            }
            PlaybackUpdate::Warning(message) => {
                self.notice = Some("Pointer replay may not be 1:1.".into());
                self.set_banner(BannerLevel::Warning, "Mouse accuracy", Some(message));
            }
            PlaybackUpdate::Finished {
                cancelled,
                completed_passes,
            } => {
                self.mode = Mode::Idle;
                self.paused = false;
                self.playback_commands = None;
                self.playback_passes = None;
                self.playback_pass = completed_passes.max(1);
                if let Some(handle) = self.playback_handle.take() {
                    let _ = handle.join();
                }
                self.elapsed_us = self
                    .recording
                    .as_ref()
                    .map_or(0, |recording| recording.duration_us);
                self.notice = Some(if cancelled {
                    "Playback stopped.".to_owned()
                } else {
                    format!("Playback finished ({completed_passes} loop(s)).")
                });
            }
            PlaybackUpdate::Failed(error) => {
                self.mode = Mode::Idle;
                self.paused = false;
                self.playback_commands = None;
                self.playback_passes = None;
                if let Some(handle) = self.playback_handle.take() {
                    let _ = handle.join();
                }
                self.elapsed_us = self
                    .recording
                    .as_ref()
                    .map_or(0, |recording| recording.duration_us);
                self.notice = Some("Playback could not start.".into());
                self.set_banner(BannerLevel::Error, "Playback failed", Some(error));
            }
        }
        self.publish();
    }

    fn handle_hotkeys(&mut self, update: HotkeyUpdate) {
        match update {
            HotkeyUpdate::Starting => {
                self.hotkey_state = HotkeyUiState::Starting;
            }
            HotkeyUpdate::AwaitingApproval => {
                self.hotkey_state = HotkeyUiState::AwaitingApproval;
                self.notice = Some("Waiting for global shortcut approval in KDE…".into());
            }
            HotkeyUpdate::Ready { shortcuts } => {
                self.hotkey_state = HotkeyUiState::Ready(shortcuts);
                if self
                    .notice
                    .as_deref()
                    .is_some_and(|notice| notice.starts_with("Waiting for global shortcut"))
                {
                    self.notice = None;
                }
            }
            HotkeyUpdate::ConfigureFailed(error) => {
                self.set_banner(
                    BannerLevel::Warning,
                    "Global shortcut configuration",
                    Some(error),
                );
            }
            HotkeyUpdate::Unavailable(error) => {
                self.hotkey_state = HotkeyUiState::Unavailable(error.clone());
                self.set_banner(
                    BannerLevel::Warning,
                    "Global shortcuts unavailable",
                    Some(error),
                );
            }
            HotkeyUpdate::Activated(action) => {
                self.handle_hotkey_action(action);
            }
        }
        self.publish();
    }

    fn handle_hotkey_action(&mut self, action: HotkeyAction) {
        if self.dialog_pending {
            return;
        }
        match action {
            HotkeyAction::ToggleRecording => match self.mode {
                Mode::Idle => self.start_recording(),
                Mode::Recording => self.stop_active(),
                Mode::Playing => {}
            },
            HotkeyAction::ToggleRecordingPause => {
                if self.mode == Mode::Recording {
                    self.toggle_pause();
                }
            }
            HotkeyAction::TogglePlayback => match self.mode {
                Mode::Idle => self.start_playback(),
                Mode::Playing => self.stop_active(),
                Mode::Recording => {}
            },
            HotkeyAction::EmergencyStop => match self.mode {
                Mode::Recording => self.stop_active(),
                Mode::Playing => self.stop_active(),
                Mode::Idle => {}
            },
        }
    }

    fn start_hotkeys(&mut self) {
        let sender = self.tx.clone();
        let service = hotkeys::spawn(move |update| {
            let _ = sender.send(Message::Hotkeys(update));
        });
        self.hotkey_service = Some(service);
    }

    fn start_save_dialog(&mut self) {
        if self.mode.is_busy() || self.dialog_pending || self.shutting_down {
            return;
        }
        let Some(recording) = self.recording.clone() else {
            self.notice = Some("There is no recording to save.".into());
            self.publish();
            return;
        };
        if recording.is_empty() {
            self.notice = Some("There is no recording to save.".into());
            self.publish();
            return;
        }

        self.dialog_pending = true;
        self.publish();
        let sender = self.tx.clone();
        let directory = self.settings.last_directory.clone();
        let suggested_name = self
            .path
            .as_ref()
            .and_then(|path| path.file_name())
            .and_then(|name| name.to_str())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| "mimic-recording.mimic".to_owned());

        thread::Builder::new()
            .name("mimic-save-dialog".into())
            .spawn(move || {
                let outcome = save_dialog(recording, &directory, &suggested_name);
                let _ = sender.send(Message::Dialog(DialogOutcome::Save(outcome)));
            })
            .expect("failed to spawn save dialog thread");
    }

    fn start_open_dialog(&mut self) {
        if self.mode.is_busy() || self.dialog_pending || self.shutting_down {
            return;
        }
        self.dialog_pending = true;
        self.publish();
        let sender = self.tx.clone();
        let directory = self.settings.last_directory.clone();
        thread::Builder::new()
            .name("mimic-open-dialog".into())
            .spawn(move || {
                let outcome = open_dialog(&directory);
                let _ = sender.send(Message::Dialog(DialogOutcome::Open(outcome)));
            })
            .expect("failed to spawn open dialog thread");
    }

    fn handle_dialog(&mut self, outcome: DialogOutcome) {
        self.dialog_pending = false;
        match outcome {
            DialogOutcome::Save(SaveOutcome::Cancelled) => {
                self.notice = Some("Save cancelled.".into());
            }
            DialogOutcome::Save(SaveOutcome::Saved(path)) => {
                self.path = Some(path.clone());
                self.dirty = false;
                self.recovered = false;
                if let Some(parent) = path.parent() {
                    self.settings.last_directory = parent.to_owned();
                    let _ = self.settings.save_to(&paths::settings_file());
                }
                if let Some(recording) = &mut self.recording {
                    if recording.name.is_none() {
                        recording.name = path
                            .file_stem()
                            .and_then(|name| name.to_str())
                            .map(ToOwned::to_owned);
                    }
                }
                let _ = fs::remove_file(paths::recovery_file());
                self.notice = Some(format!("Saved {}", display_path(&path)));
                self.update_title();
            }
            DialogOutcome::Save(SaveOutcome::Failed(error)) => {
                self.set_banner(BannerLevel::Error, "Could not save recording", Some(error));
            }
            DialogOutcome::Open(OpenOutcome::Cancelled) => {
                self.notice = Some("Open cancelled.".into());
            }
            DialogOutcome::Open(OpenOutcome::Failed(error)) => {
                self.set_banner(BannerLevel::Error, "Could not open recording", Some(error));
            }
            DialogOutcome::Open(OpenOutcome::Loaded(path, recording)) => {
                self.elapsed_us = recording.duration_us;
                self.recording = Some(recording);
                self.path = Some(path.clone());
                self.dirty = false;
                self.recovered = false;
                self.mode = Mode::Idle;
                self.paused = false;
                if let Some(parent) = path.parent() {
                    self.settings.last_directory = parent.to_owned();
                    let _ = self.settings.save_to(&paths::settings_file());
                }
                let _ = fs::remove_file(paths::recovery_file());
                self.notice = Some(format!("Opened {}", display_path(&path)));
                self.update_title();
            }
        }
        self.publish();
    }

    fn write_recovery(&self) -> Result<(), String> {
        let Some(recording) = &self.recording else {
            return Ok(());
        };
        if recording.is_empty() {
            return Ok(());
        }
        storage::save_recording(&paths::recovery_file(), recording)
            .map_err(|error| error.to_string())
    }

    fn update_title(&self) {
        let title = match &self.path {
            Some(path) => format!("Mimic — {}", display_path(path)),
            None => "Mimic".to_owned(),
        };
        self.repaint
            .send_viewport_cmd(egui::ViewportCommand::Title(title));
    }

    fn publish(&mut self) {
        if self.shutting_down {
            return;
        }
        let recording = self.recording.as_ref();
        let (event_count, elapsed_us) = match self.mode {
            Mode::Recording => (self.live_event_count, self.elapsed_us),
            Mode::Playing => (recording.map_or(0, Recording::event_count), self.elapsed_us),
            Mode::Idle => (
                recording.map_or(0, Recording::event_count),
                recording.map_or(0, |recording| recording.duration_us),
            ),
        };
        let macro_name = if self.mode == Mode::Recording {
            "New recording".to_owned()
        } else {
            match (&self.path, recording) {
                (_, None) => "No recording".to_owned(),
                (Some(path), Some(_)) => display_path(path),
                (None, Some(recording)) => {
                    let name = recording.display_name();
                    if name == "Untitled recording" {
                        "Untitled recording".to_owned()
                    } else {
                        name
                    }
                }
            }
        };
        let snapshot = Snapshot {
            mode: self.mode,
            paused: self.paused,
            elapsed_us,
            event_count,
            macro_event_count: recording.map_or(0, Recording::event_count),
            macro_duration_us: recording.map_or(0, |recording| recording.duration_us),
            macro_name,
            macro_path: self.path.clone(),
            dirty: self.dirty,
            recovered: self.recovered,
            settings: self.settings.clone(),
            devices: self.devices.clone(),
            playback_pass: self.playback_pass,
            playback_passes: self.playback_passes,
            hotkeys: self.hotkey_state.clone(),
            dialog_pending: self.dialog_pending,
            banner: self.banner.clone(),
            notice: self.notice.clone(),
        };
        *self
            .snapshot
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = snapshot;
        self.repaint.request_repaint();
    }

    fn set_banner(&mut self, level: BannerLevel, title: impl Into<String>, detail: Option<String>) {
        self.banner_serial = self.banner_serial.wrapping_add(1);
        self.banner = Some(Banner {
            id: self.banner_serial,
            level,
            title: title.into(),
            detail,
        });
    }

    fn shutdown(&mut self, messages: &Receiver<Message>) {
        self.shutting_down = true;

        if self.mode == Mode::Recording {
            if let Some(commands) = &self.capture_commands {
                let _ = commands.send(CaptureCommand::Stop);
            }
            // Give the capture worker a moment to hand back the partial
            // recording so it can be written to crash recovery before exit.
            let deadline = std::time::Instant::now() + Duration::from_millis(750);
            while std::time::Instant::now() < deadline {
                let remaining = deadline.saturating_duration_since(std::time::Instant::now());
                match messages.recv_timeout(remaining.min(Duration::from_millis(50))) {
                    Ok(Message::Capture(CaptureUpdate::Stopped(recording))) => {
                        if !recording.is_empty() {
                            self.recording = Some(recording);
                            self.dirty = true;
                        }
                        break;
                    }
                    Ok(_) => {}
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
        }

        if self.mode == Mode::Playing {
            if let Some(commands) = &self.playback_commands {
                let _ = commands.send(PlaybackCommand::Stop);
            }
        }

        if let Some(handle) = self.capture_handle.take() {
            let _ = handle.join();
        }
        if let Some(handle) = self.playback_handle.take() {
            let _ = handle.join();
        }
        if let Some(service) = self.hotkey_service.take() {
            service.shutdown();
        }
        if self.dirty {
            let _ = self.write_recovery();
        }
    }
}

fn save_dialog(recording: Recording, directory: &Path, suggested_name: &str) -> SaveOutcome {
    let picked = rfd::FileDialog::new()
        .set_title("Save Mimic recording")
        .add_filter("Mimic recording", &["mimic"])
        .set_directory(directory)
        .set_file_name(suggested_name)
        .save_file();
    let Some(path) = picked else {
        return SaveOutcome::Cancelled;
    };
    let path = storage::ensure_extension(path);
    let mut recording = recording;
    if recording.name.is_none() {
        recording.name = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .map(ToOwned::to_owned);
    }
    match storage::save_recording(&path, &recording) {
        Ok(()) => SaveOutcome::Saved(path),
        Err(error) => SaveOutcome::Failed(error.to_string()),
    }
}

fn open_dialog(directory: &Path) -> OpenOutcome {
    let picked = rfd::FileDialog::new()
        .set_title("Open Mimic recording")
        .add_filter("Mimic recording", &["mimic"])
        .set_directory(directory)
        .pick_file();
    let Some(path) = picked else {
        return OpenOutcome::Cancelled;
    };
    match storage::load_recording(&path) {
        Ok(recording) => OpenOutcome::Loaded(path, recording),
        Err(error) => OpenOutcome::Failed(format!("{}: {error}", path.display())),
    }
}

fn display_path(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| path.display().to_string())
}

fn initial_snapshot() -> Snapshot {
    Snapshot {
        mode: Mode::Idle,
        paused: false,
        elapsed_us: 0,
        event_count: 0,
        macro_event_count: 0,
        macro_duration_us: 0,
        macro_name: "No recording".to_owned(),
        macro_path: None,
        dirty: false,
        recovered: false,
        settings: Settings::default(),
        devices: Vec::new(),
        playback_pass: 1,
        playback_passes: None,
        hotkeys: HotkeyUiState::Starting,
        dialog_pending: false,
        banner: None,
        notice: None,
    }
}
