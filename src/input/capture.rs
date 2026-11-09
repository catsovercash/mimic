//! Physical input capture via Linux evdev.
//!
//! Mimic reads every accessible keyboard and relative pointer device directly.
//! It does not grab the devices, so normal desktop input continues to work.
//! Events are grouped into kernel input frames and assigned timestamps from a
//! monotonic clock. Paused time is excluded from the timeline.

use std::collections::HashSet;
use std::fs;
use std::io;
use std::os::fd::AsFd;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::thread;
use std::time::{Duration, Instant};

use evdev::{Device, EventType, InputEvent, KeyCode, RelativeAxisCode, SynchronizationCode};
use nix::poll::{poll, PollFd, PollFlags, PollTimeout};

use crate::cursor;
use crate::error::InputError;
use crate::model::{
    is_keyboard_key, is_mouse_button, Action, PointerPoint, RecordedEvent, Recording,
};

const POLL_TIMEOUT_MS: u16 = 20;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);
const RESCAN_INTERVAL: Duration = Duration::from_secs(2);
const CHECKPOINT_INTERVAL: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureCommand {
    Pause,
    Resume,
    Stop,
}

#[derive(Debug, Clone)]
pub enum CaptureUpdate {
    Started {
        devices: Vec<String>,
    },
    Progress {
        elapsed_us: u64,
        event_count: usize,
    },
    Paused {
        elapsed_us: u64,
        event_count: usize,
    },
    Resumed,
    Warning(String),
    /// Periodic snapshot used for crash recovery while recording continues.
    Checkpoint(Recording),
    Stopped(Recording),
    Failed(String),
}

#[derive(Debug, Clone, Copy)]
struct DeviceKind {
    keyboard: bool,
    pointer: bool,
}

impl DeviceKind {
    fn relevant(self, code: u16) -> bool {
        (self.pointer && is_mouse_button(code)) || (self.keyboard && is_keyboard_key(code))
    }
}

/// Starts a capture worker. The callback is deliberately a closure so the
/// controller can wrap updates into its own message enum without this module
/// depending on the GUI or controller.
pub fn spawn<F>(send: F, commands: Receiver<CaptureCommand>) -> thread::JoinHandle<()>
where
    F: Fn(CaptureUpdate) + Send + 'static,
{
    thread::Builder::new()
        .name("mimic-capture".into())
        .spawn(move || run(send, commands))
        .expect("failed to spawn capture thread")
}

fn run(send: impl Fn(CaptureUpdate), commands: Receiver<CaptureCommand>) {
    let mut devices = Vec::new();
    let report = scan_devices(&mut devices);
    let names = devices
        .iter()
        .map(|device| device.name.clone())
        .collect::<Vec<_>>();

    if devices.is_empty() {
        let error = if report.denied_relevant > 0 {
            InputError::NoCapturePermission
        } else {
            InputError::NoDevices
        };
        send(CaptureUpdate::Failed(error.to_string()));
        return;
    }

    // Snapshot the absolute cursor position before the first frame. Playback
    // uses it to return the pointer to the same screen coordinate before
    // replaying the relative movement. Failure is non-fatal; the recording will
    // simply replay relative to the pointer's current position.
    let pointer_start = cursor::current_position()
        .ok()
        .map(|(x, y)| PointerPoint { x, y });

    send(CaptureUpdate::Started {
        devices: names.clone(),
    });
    if report.denied_relevant > 0 {
        send(CaptureUpdate::Warning(format!(
            "{} input device(s) could not be opened. The matching devices will not be recorded. \
             Run the udev setup script once and log out and back in.",
            report.denied_relevant
        )));
    }

    let mut recording = Recording::empty();
    recording.pointer_start = pointer_start;
    let mut recorder = Recorder {
        recording,
        timeline: Timeline::start(),
        paused: false,
        send,
        commands,
        devices,
        last_progress: Instant::now(),
        last_checkpoint: Instant::now(),
        next_rescan: Instant::now() + RESCAN_INTERVAL,
        warned_no_devices: false,
    };

    recorder.loop_until_stopped();
}

struct Recorder<F: Fn(CaptureUpdate)> {
    recording: Recording,
    timeline: Timeline,
    paused: bool,
    send: F,
    commands: Receiver<CaptureCommand>,
    devices: Vec<CaptureDevice>,
    last_progress: Instant,
    last_checkpoint: Instant,
    next_rescan: Instant,
    warned_no_devices: bool,
}

impl<F: Fn(CaptureUpdate)> Recorder<F> {
    fn loop_until_stopped(&mut self) {
        loop {
            if self.handle_commands() {
                return;
            }

            let now = Instant::now();
            if now >= self.next_rescan {
                self.next_rescan = now + RESCAN_INTERVAL;
                self.rescan();
            }

            self.poll_devices();
            if self.paused {
                continue;
            }

            if self.last_progress.elapsed() >= PROGRESS_INTERVAL {
                self.last_progress = Instant::now();
                let elapsed_us = self.timeline.elapsed_us();
                (self.send)(CaptureUpdate::Progress {
                    elapsed_us,
                    event_count: self.recording.events.len(),
                });
            }
            if self.last_checkpoint.elapsed() >= CHECKPOINT_INTERVAL
                && !self.recording.events.is_empty()
            {
                self.send_checkpoint();
            }
        }
    }

    /// Returns true if the worker should exit.
    fn handle_commands(&mut self) -> bool {
        while let Ok(command) = self.commands.try_recv() {
            match command {
                CaptureCommand::Pause => {
                    if !self.paused {
                        self.flush_frames();
                        self.send_checkpoint();
                        self.timeline.pause();
                        self.paused = true;
                        (self.send)(CaptureUpdate::Paused {
                            elapsed_us: self.timeline.elapsed_us(),
                            event_count: self.recording.events.len(),
                        });
                    }
                }
                CaptureCommand::Resume => {
                    if self.paused {
                        // Discard input that arrived while paused, then rebuild
                        // the held-key set from the kernel. This prevents a key
                        // pressed during a pause from producing an unmatched
                        // release in the recording.
                        self.drain_paused_input();
                        for device in &mut self.devices {
                            device.refresh_held_state();
                        }
                        self.timeline.resume();
                        self.paused = false;
                        self.last_progress = Instant::now();
                        (self.send)(CaptureUpdate::Resumed);
                    }
                }
                CaptureCommand::Stop => {
                    return self.finish();
                }
            }
        }
        false
    }

    fn send_checkpoint(&mut self) {
        if self.recording.events.is_empty() {
            return;
        }
        let mut snapshot = self.recording.clone();
        snapshot.finish(self.timeline.elapsed_us());
        self.last_checkpoint = Instant::now();
        (self.send)(CaptureUpdate::Checkpoint(snapshot));
    }

    fn finish(&mut self) -> bool {
        if !self.paused {
            self.flush_frames();
        }
        self.timeline.pause();
        self.recording.events.sort_by_key(|event| event.at_us);
        let duration = self.timeline.elapsed_us();
        self.recording.finish(duration);
        (self.send)(CaptureUpdate::Stopped(self.recording.clone()));
        true
    }

    fn poll_devices(&mut self) {
        if self.devices.is_empty() {
            thread::sleep(Duration::from_millis(POLL_TIMEOUT_MS as u64));
            if !self.warned_no_devices {
                self.warned_no_devices = true;
                (self.send)(CaptureUpdate::Warning(
                    "All input devices were disconnected. Recording continues and will resume \
                     when a device is reconnected."
                        .to_owned(),
                ));
            }
            return;
        }

        let mut ready = Vec::new();
        {
            let mut poll_fds = self
                .devices
                .iter()
                .map(|device| PollFd::new(device.device.as_fd(), PollFlags::POLLIN))
                .collect::<Vec<_>>();
            match poll(&mut poll_fds, PollTimeout::from(POLL_TIMEOUT_MS)) {
                Ok(0) => return,
                Ok(_) => {
                    for (index, fd) in poll_fds.iter().enumerate() {
                        if fd
                            .revents()
                            .is_some_and(|flags| flags.intersects(PollFlags::POLLIN))
                        {
                            ready.push(index);
                        }
                    }
                }
                Err(nix::errno::Errno::EINTR) => return,
                Err(_) => {
                    // If poll itself fails, fall back to a short sleep rather
                    // than spinning. Individual device failures are handled
                    // below.
                    thread::sleep(Duration::from_millis(POLL_TIMEOUT_MS as u64));
                    return;
                }
            }
        }

        let mut remove = Vec::new();
        {
            let mut poll_fds = self
                .devices
                .iter()
                .map(|device| PollFd::new(device.device.as_fd(), PollFlags::POLLIN))
                .collect::<Vec<_>>();
            // A second non-blocking poll is cheap and lets us notice hung-up
            // devices without holding a borrow across event processing.
            let _ = poll(&mut poll_fds, PollTimeout::ZERO);
            for (index, fd) in poll_fds.iter().enumerate() {
                if fd.revents().is_some_and(|flags| {
                    flags.intersects(PollFlags::POLLERR | PollFlags::POLLHUP | PollFlags::POLLNVAL)
                }) {
                    remove.push(index);
                }
            }
        }

        for index in ready {
            if remove.contains(&index) {
                continue;
            }
            match self.devices[index].read_available(
                self.paused,
                &mut self.recording,
                &self.timeline,
            ) {
                Ok(()) => {}
                Err(error) => {
                    (self.send)(CaptureUpdate::Warning(format!(
                        "Input device '{}' stopped responding: {error}",
                        self.devices[index].name
                    )));
                    remove.push(index);
                }
            }
        }

        for index in remove.into_iter().rev() {
            self.devices.remove(index);
        }
        if self.devices.is_empty() {
            self.warned_no_devices = false;
        }
    }

    fn drain_paused_input(&mut self) {
        for device in &mut self.devices {
            let _ = device.drain_events();
            device.clear_frame();
        }
    }

    fn flush_frames(&mut self) {
        let t_us = self.timeline.elapsed_us();
        for device in &mut self.devices {
            device.flush_frame(&mut self.recording, t_us);
        }
    }

    fn rescan(&mut self) {
        let before = self.devices.len();
        let report = scan_devices(&mut self.devices);
        if report.denied_relevant > 0 && before != self.devices.len() {
            (self.send)(CaptureUpdate::Warning(format!(
                "{} additional input device(s) could not be opened.",
                report.denied_relevant
            )));
        }
        if self.devices.len() > before {
            self.warned_no_devices = false;
        }
    }
}

struct Timeline {
    started: Instant,
    paused_at: Option<Instant>,
    paused_total: Duration,
}

impl Timeline {
    fn start() -> Self {
        Self {
            started: Instant::now(),
            paused_at: None,
            paused_total: Duration::ZERO,
        }
    }

    fn pause(&mut self) {
        if self.paused_at.is_none() {
            self.paused_at = Some(Instant::now());
        }
    }

    fn resume(&mut self) {
        if let Some(paused_at) = self.paused_at.take() {
            self.paused_total += paused_at.elapsed();
        }
    }

    fn elapsed_us(&self) -> u64 {
        let now = self.paused_at.unwrap_or_else(Instant::now);
        now.saturating_duration_since(self.started)
            .saturating_sub(self.paused_total)
            .as_micros()
            .try_into()
            .unwrap_or(u64::MAX)
    }
}

struct CaptureDevice {
    path: PathBuf,
    name: String,
    device: Device,
    kind: DeviceKind,
    frame: FrameBuffer,
    down: HashSet<u16>,
    ignored_until_release: HashSet<u16>,
}

impl CaptureDevice {
    fn read_available(
        &mut self,
        paused: bool,
        recording: &mut Recording,
        timeline: &Timeline,
    ) -> io::Result<()> {
        loop {
            let events = match self.device.fetch_events() {
                Ok(events) => events.collect::<Vec<_>>(),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(error) => return Err(error),
            };
            if events.is_empty() {
                return Ok(());
            }
            if paused {
                continue;
            }
            for event in events {
                self.handle_event(event, recording, timeline);
            }
        }
    }

    fn drain_events(&mut self) -> io::Result<()> {
        loop {
            let events = match self.device.fetch_events() {
                Ok(events) => events.collect::<Vec<_>>(),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(error) => return Err(error),
            };
            if events.is_empty() {
                return Ok(());
            }
        }
    }

    fn handle_event(&mut self, event: InputEvent, recording: &mut Recording, timeline: &Timeline) {
        match event.event_type() {
            EventType::SYNCHRONIZATION => {
                if event.code() == SynchronizationCode::SYN_REPORT.0 {
                    self.flush_frame(recording, timeline.elapsed_us());
                }
            }
            EventType::KEY => self.handle_key(event.code(), event.value()),
            EventType::RELATIVE => self.handle_relative(event.code(), event.value()),
            _ => {}
        }
    }

    fn handle_key(&mut self, code: u16, value: i32) {
        if value == 2 || !self.kind.relevant(code) {
            return;
        }
        if value != 0 && value != 1 {
            return;
        }
        let pressed = value == 1;

        if self.ignored_until_release.contains(&code) {
            if !pressed {
                self.ignored_until_release.remove(&code);
                self.down.remove(&code);
            }
            return;
        }

        let changed = if pressed {
            self.down.insert(code)
        } else {
            self.down.remove(&code)
        };
        if !changed {
            return;
        }

        let action = if is_mouse_button(code) {
            Action::Button { code, pressed }
        } else if is_keyboard_key(code) {
            Action::Key { code, pressed }
        } else {
            return;
        };
        self.frame.push_action(action);
    }

    fn handle_relative(&mut self, code: u16, value: i32) {
        match code {
            c if c == RelativeAxisCode::REL_X.0 => self.frame.add_motion(i64::from(value), 0),
            c if c == RelativeAxisCode::REL_Y.0 => self.frame.add_motion(0, i64::from(value)),
            c if c == RelativeAxisCode::REL_WHEEL.0 => {
                self.frame.add_scroll(0, i64::from(value));
            }
            c if c == RelativeAxisCode::REL_HWHEEL.0 => {
                self.frame.add_scroll(i64::from(value), 0);
            }
            _ => {}
        }
    }

    fn flush_frame(&mut self, recording: &mut Recording, at_us: u64) {
        for action in self.frame.take() {
            recording.events.push(RecordedEvent::new(at_us, action));
        }
    }

    fn clear_frame(&mut self) {
        self.frame.clear();
    }

    fn refresh_held_state(&mut self) {
        self.down.clear();
        self.ignored_until_release.clear();
        if let Ok(state) = self.device.get_key_state() {
            if let Some(keys) = self.device.supported_keys() {
                for key in keys.iter() {
                    if self.kind.relevant(key.0) && state.contains(key) {
                        self.down.insert(key.0);
                        self.ignored_until_release.insert(key.0);
                    }
                }
            }
        }
        self.frame.clear();
    }
}

#[derive(Default)]
struct FrameBuffer {
    parts: Vec<FramePart>,
    motion_index: Option<usize>,
    scroll_index: Option<usize>,
}

#[derive(Debug, Clone)]
enum FramePart {
    Action(Action),
    Motion { dx: i64, dy: i64 },
    Scroll { dx: i64, dy: i64 },
}

impl FrameBuffer {
    fn push_action(&mut self, action: Action) {
        self.parts.push(FramePart::Action(action));
    }

    fn add_motion(&mut self, dx: i64, dy: i64) {
        let index = match self.motion_index {
            Some(index) => index,
            None => {
                self.parts.push(FramePart::Motion { dx: 0, dy: 0 });
                let index = self.parts.len() - 1;
                self.motion_index = Some(index);
                index
            }
        };
        if let FramePart::Motion {
            dx: current_x,
            dy: current_y,
        } = &mut self.parts[index]
        {
            *current_x += dx;
            *current_y += dy;
        }
    }

    fn add_scroll(&mut self, dx: i64, dy: i64) {
        let index = match self.scroll_index {
            Some(index) => index,
            None => {
                self.parts.push(FramePart::Scroll { dx: 0, dy: 0 });
                let index = self.parts.len() - 1;
                self.scroll_index = Some(index);
                index
            }
        };
        if let FramePart::Scroll {
            dx: current_x,
            dy: current_y,
        } = &mut self.parts[index]
        {
            *current_x += dx;
            *current_y += dy;
        }
    }

    fn take(&mut self) -> Vec<Action> {
        let parts = std::mem::take(&mut self.parts);
        self.motion_index = None;
        self.scroll_index = None;
        let mut actions = Vec::with_capacity(parts.len());
        for part in parts {
            match part {
                FramePart::Action(action) => actions.push(action),
                FramePart::Motion { dx, dy } => {
                    if dx != 0 || dy != 0 {
                        actions.push(Action::Move {
                            dx: clamp_motion(dx),
                            dy: clamp_motion(dy),
                        });
                    }
                }
                FramePart::Scroll { dx, dy } => {
                    if dx != 0 || dy != 0 {
                        actions.push(Action::Scroll {
                            dx: clamp_scroll(dx),
                            dy: clamp_scroll(dy),
                        });
                    }
                }
            }
        }
        actions
    }

    fn clear(&mut self) {
        self.parts.clear();
        self.motion_index = None;
        self.scroll_index = None;
    }

    #[cfg(test)]
    fn is_empty(&self) -> bool {
        self.parts.is_empty()
    }
}

fn clamp_motion(value: i64) -> i32 {
    value.clamp(-100_000, 100_000) as i32
}

fn clamp_scroll(value: i64) -> i32 {
    value.clamp(-1_000_000, 1_000_000) as i32
}

#[derive(Default)]
struct ScanReport {
    denied_relevant: usize,
}

/// Adds every accessible keyboard/pointer to `devices`. Devices that are not
/// currently openable are counted for diagnostics only when their sysfs
/// capabilities show that they are a keyboard or mouse.
fn scan_devices(devices: &mut Vec<CaptureDevice>) -> ScanReport {
    let mut report = ScanReport::default();
    let existing = devices
        .iter()
        .map(|device| device.path.clone())
        .collect::<HashSet<_>>();

    let mut paths = match fs::read_dir("/dev/input") {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| {
                        name.strip_prefix("event").is_some_and(|suffix| {
                            !suffix.is_empty() && suffix.chars().all(|c| c.is_ascii_digit())
                        })
                    })
            })
            .collect::<Vec<_>>(),
        Err(_) => {
            report.denied_relevant += 1;
            return report;
        }
    };
    paths.sort();

    for path in paths {
        if existing.contains(&path) {
            continue;
        }
        match open_capture_device(&path) {
            Ok(Some(device)) => devices.push(device),
            Ok(None) => {}
            Err(error) => {
                if error.kind() == io::ErrorKind::PermissionDenied && sysfs_looks_relevant(&path) {
                    report.denied_relevant += 1;
                }
            }
        }
    }

    report
}

fn open_capture_device(path: &Path) -> io::Result<Option<CaptureDevice>> {
    let device = Device::open(path)?;
    let name = device.name().unwrap_or("Unnamed input device").to_owned();
    let phys = device.physical_path().unwrap_or("");

    if name == "Mimic Virtual Input" || phys == "mimic/input0" {
        return Ok(None);
    }

    let Some(kind) = classify_device(&device) else {
        return Ok(None);
    };

    device.set_nonblocking(true)?;

    let mut capture_device = CaptureDevice {
        path: path.to_owned(),
        name,
        device,
        kind,
        frame: FrameBuffer::default(),
        down: HashSet::new(),
        ignored_until_release: HashSet::new(),
    };
    capture_device.refresh_held_state();
    Ok(Some(capture_device))
}

fn classify_device(device: &Device) -> Option<DeviceKind> {
    let keys = device.supported_keys()?;
    let rel = device.supported_relative_axes();

    let keyboard = [
        KeyCode::KEY_A,
        KeyCode::KEY_Z,
        KeyCode::KEY_ENTER,
        KeyCode::KEY_SPACE,
        KeyCode::KEY_ESC,
        KeyCode::KEY_TAB,
        KeyCode::KEY_BACKSPACE,
        KeyCode::KEY_LEFTSHIFT,
        KeyCode::KEY_LEFTCTRL,
        KeyCode::KEY_LEFTALT,
        KeyCode::KEY_LEFTMETA,
        KeyCode::KEY_F1,
        KeyCode::KEY_UP,
        KeyCode::KEY_VOLUMEUP,
        KeyCode::KEY_PLAYPAUSE,
    ]
    .iter()
    .any(|key| keys.contains(*key));

    let pointer = keys.iter().any(|key| is_mouse_button(key.0))
        && rel.is_some_and(|axes| {
            axes.contains(RelativeAxisCode::REL_X) || axes.contains(RelativeAxisCode::REL_Y)
        });

    if keyboard || pointer {
        Some(DeviceKind { keyboard, pointer })
    } else {
        None
    }
}

/// Best-effort classification without opening the device, used to distinguish
/// "no keyboard/mouse exists" from "permission denied" in the UI.
///
/// This deliberately checks capabilities rather than the device name. Some
/// composite devices expose extra `event*` nodes that are named like a mouse
/// but only contain absolute axes and no buttons. Warning about those is noise:
/// even with permission they could not be captured as a keyboard or pointer.
fn sysfs_looks_relevant(path: &Path) -> bool {
    let Some(node) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let base = Path::new("/sys/class/input")
        .join(node)
        .join("device/capabilities");
    let Ok(key_bits) = fs::read_to_string(base.join("key")) else {
        return false;
    };
    let rel_bits = fs::read_to_string(base.join("rel")).unwrap_or_default();
    sysfs_capabilities_look_relevant(&key_bits, &rel_bits)
}

/// Pure capability check for the denial diagnostic. Keep this aligned with
/// what `classify_device` can actually record: a real keyboard key, or a
/// pointer with mouse buttons and relative X/Y movement. Media-only endpoints
/// such as a microphone's volume/play buttons are intentionally not included
/// in this permission warning.
fn sysfs_capabilities_look_relevant(key_bits: &str, rel_bits: &str) -> bool {
    let key_words = key_bits.split_whitespace().collect::<Vec<_>>();
    let rel_words = rel_bits.split_whitespace().collect::<Vec<_>>();

    let core_keyboard_probe = [
        30u16, // KEY_A
        44,    // KEY_Z
        28,    // KEY_ENTER
        57,    // KEY_SPACE
        1,     // KEY_ESC
        15,    // KEY_TAB
        14,    // KEY_BACKSPACE
        42,    // KEY_LEFTSHIFT
        29,    // KEY_LEFTCTRL
        56,    // KEY_LEFTALT
        125,   // KEY_LEFTMETA
        59,    // KEY_F1
        103,   // KEY_UP
    ];
    let has_keyboard = core_keyboard_probe
        .iter()
        .any(|code| bitmap_has(&key_words, *code));
    let has_mouse_button = (0x110..=0x117).any(|code| bitmap_has(&key_words, code));
    let has_relative_xy = bitmap_has(&rel_words, 0) || bitmap_has(&rel_words, 1);

    has_keyboard || (has_mouse_button && has_relative_xy)
}

/// Sysfs capability files print bitmap words from most significant to least
/// significant, while the code index is measured from the least significant
/// word.
fn bitmap_has(words: &[&str], bit: u16) -> bool {
    let word_from_start = usize::from(bit / 64);
    if word_from_start >= words.len() {
        return false;
    }
    let index = words.len() - 1 - word_from_start;
    let bit_in_word = u32::from(bit % 64);
    u64::from_str_radix(words[index], 16)
        .map(|word| word & (1u64 << bit_in_word) != 0)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests;
