//! Timestamp-scheduled playback through a uinput virtual input device.
//!
//! The scheduler is monotonic and does not busy-wait. It checks its command
//! channel at least every 10 ms, so Pause and Stop remain responsive even in
//! an infinite loop. Held keys and buttons are explicitly released on pause,
//! stop, completion, and errors.

use std::collections::HashSet;
use std::io;
use std::ops::Range;
use std::sync::mpsc::{Receiver, RecvTimeoutError, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use evdev::uinput::VirtualDevice;
use evdev::{
    AttributeSet, BusType, EventType, InputEvent, InputId, KeyCode, PropType, RelativeAxisCode,
};

use crate::cursor;
use crate::error::InputError;
use crate::model::{Action, PointerPoint, Recording};

const COMMAND_POLL: Duration = Duration::from_millis(10);
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);
const DEVICE_SETTLE_DELAY: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, PartialEq)]
pub struct PlaybackOptions {
    pub speed: f64,
    pub passes: Option<u32>,
    pub loop_delay: Duration,
}

impl PlaybackOptions {
    pub fn new(speed: f64, passes: Option<u32>, loop_delay: Duration) -> Self {
        Self {
            speed: speed.max(0.01),
            passes,
            loop_delay,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackCommand {
    Pause,
    Resume,
    Stop,
}

#[derive(Debug, Clone)]
pub enum PlaybackUpdate {
    Started {
        passes: Option<u32>,
    },
    Progress {
        position_us: u64,
        pass: u32,
        passes: Option<u32>,
    },
    Paused {
        position_us: u64,
    },
    Resumed,
    Finished {
        cancelled: bool,
        completed_passes: u32,
    },
    Warning(String),
    Failed(String),
}

pub fn spawn<F>(
    recording: Recording,
    options: PlaybackOptions,
    send: F,
    commands: Receiver<PlaybackCommand>,
) -> thread::JoinHandle<()>
where
    F: Fn(PlaybackUpdate) + Send + 'static,
{
    thread::Builder::new()
        .name("mimic-playback".into())
        .spawn(move || run(recording, options, send, commands))
        .expect("failed to spawn playback thread")
}

fn run<F: Fn(PlaybackUpdate)>(
    recording: Recording,
    options: PlaybackOptions,
    send: F,
    commands: Receiver<PlaybackCommand>,
) {
    if recording.events.is_empty() {
        (send)(PlaybackUpdate::Failed(
            InputError::EmptyRecording.to_string(),
        ));
        return;
    }

    let mut device = match create_virtual_device(&recording) {
        Ok(device) => device,
        Err(error) => {
            (send)(PlaybackUpdate::Failed(error));
            return;
        }
    };

    // Relative input must not be accelerated twice. KWin defaults a new
    // virtual pointer to adaptive acceleration, which makes raw recorded
    // deltas replay faster than the physical pointer. Set the virtual device
    // to flat/zero acceleration so movement is 1:1.
    if recording_uses_pointer(&recording) {
        if let Err(error) = configure_virtual_pointer() {
            (send)(PlaybackUpdate::Warning(format!(
                "Pointer acceleration could not be set to flat: {error}. \
                 Mouse playback may be less accurate; set KDE pointer acceleration to Flat."
            )));
        }
    }

    // A recording stores where the cursor was when recording began. Query the
    // current position and move back to that exact start before replaying the
    // relative deltas. This removes the dependency on where the user happened
    // to leave the pointer before pressing Play.
    let pointer_anchor = if recording_uses_pointer(&recording) {
        match recording.pointer_start {
            Some(target) => match cursor::current_position() {
                Ok(current) => {
                    eprintln!(
                        "DBG anchor target={target:?} current={current:?} delta={:?}",
                        pointer_anchor_delta(target, current)
                    );
                    pointer_anchor_delta(target, current)
                }
                Err(error) => {
                    (send)(PlaybackUpdate::Warning(format!(
                        "The saved pointer start could not be restored: {error}. \
                         Replaying relative to the current pointer position."
                    )));
                    None
                }
            },
            None => None,
        }
    } else {
        None
    };

    (send)(PlaybackUpdate::Started {
        passes: options.passes,
    });

    // The compositor sees uinput devices asynchronously. A short settle delay
    // prevents the first few events from racing device discovery.
    thread::sleep(DEVICE_SETTLE_DELAY);

    if let Some((dx, dy)) = pointer_anchor {
        let mut events = Vec::with_capacity(2);
        if dx != 0 {
            events.push(InputEvent::new(
                EventType::RELATIVE.0,
                RelativeAxisCode::REL_X.0,
                dx,
            ));
        }
        if dy != 0 {
            events.push(InputEvent::new(
                EventType::RELATIVE.0,
                RelativeAxisCode::REL_Y.0,
                dy,
            ));
        }
        if let Err(error) = device.emit(&events) {
            (send)(PlaybackUpdate::Failed(format!(
                "Could not move the pointer to the saved start position: {error}"
            )));
            return;
        }
        eprintln!(
            "DBG anchor emitted; cursor now {:?}",
            cursor::current_position()
        );
    }

    let groups = group_events(&recording);
    let mut player = Player {
        device,
        commands,
        send,
        held: HashSet::new(),
        physical: HashSet::new(),
        last_progress: Instant::now(),
        total_passes: options.passes,
        pass_net: (0, 0),
    };

    let result = player.play(&recording, &groups, &options);
    player.release_all();

    match result {
        Ok(PlayOutcome {
            cancelled,
            completed_passes,
        }) => {
            (player.send)(PlaybackUpdate::Finished {
                cancelled,
                completed_passes,
            });
        }
        Err(error) => {
            (player.send)(PlaybackUpdate::Failed(error));
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PlayOutcome {
    cancelled: bool,
    completed_passes: u32,
}

struct Player<F: Fn(PlaybackUpdate)> {
    device: VirtualDevice,
    commands: Receiver<PlaybackCommand>,
    send: F,
    /// Logical state of the macro. It survives a pause.
    held: HashSet<u16>,
    /// Keys/buttons currently down on the virtual device.
    physical: HashSet<u16>,
    last_progress: Instant,
    total_passes: Option<u32>,
    /// Net relative pointer movement in the current pass, used to put the
    /// pointer back at the playback start before another loop pass.
    pass_net: (i64, i64),
}

impl<F: Fn(PlaybackUpdate)> Player<F> {
    fn play(
        &mut self,
        recording: &Recording,
        groups: &[EventGroup],
        options: &PlaybackOptions,
    ) -> Result<PlayOutcome, String> {
        let mut completed_passes = 0u32;
        loop {
            let mut anchor = Instant::now();
            let pass = completed_passes + 1;
            self.pass_net = (0, 0);
            for group in groups {
                let deadline = anchor + scale_micros(group.at_us, options.speed);
                let position = group.previous_at_us;
                match self.wait_until(&mut anchor, deadline, position, pass)? {
                    WaitOutcome::Cancelled => {
                        return Ok(PlayOutcome {
                            cancelled: true,
                            completed_passes,
                        });
                    }
                    WaitOutcome::Elapsed => {}
                }

                self.emit_group(&recording.events[group.range.clone()])?;
                self.maybe_progress(group.at_us, pass);
            }

            self.release_all();
            completed_passes = completed_passes.saturating_add(1);

            if self
                .total_passes
                .is_some_and(|total| completed_passes >= total)
            {
                break;
            }

            // Relative motion is cumulative. Return the pointer to the
            // position it occupied when this playback started before the next
            // pass, otherwise each loop starts at the previous pass's final
            // offset and the iterations visibly drift.
            self.reset_pointer_to_playback_start()?;

            if !options.loop_delay.is_zero() {
                let mut delay_anchor = Instant::now();
                let deadline = delay_anchor + options.loop_delay;
                match self.wait_until(
                    &mut delay_anchor,
                    deadline,
                    recording.playback_span_us(),
                    completed_passes,
                )? {
                    WaitOutcome::Cancelled => {
                        return Ok(PlayOutcome {
                            cancelled: true,
                            completed_passes,
                        });
                    }
                    WaitOutcome::Elapsed => {}
                }
            } else {
                let mut immediate_anchor = Instant::now();
                match self.wait_until(
                    &mut immediate_anchor,
                    Instant::now(),
                    recording.playback_span_us(),
                    completed_passes,
                )? {
                    WaitOutcome::Cancelled => {
                        return Ok(PlayOutcome {
                            cancelled: true,
                            completed_passes,
                        });
                    }
                    WaitOutcome::Elapsed => {}
                }
            }

            (self.send)(PlaybackUpdate::Progress {
                position_us: 0,
                pass: completed_passes + 1,
                passes: self.total_passes,
            });
        }

        Ok(PlayOutcome {
            cancelled: false,
            completed_passes,
        })
    }

    fn emit_group(&mut self, events: &[crate::model::RecordedEvent]) -> Result<(), String> {
        let mut input_events = Vec::with_capacity(events.len() * 2);
        for event in events {
            match event.action {
                Action::Key { code, pressed } => {
                    input_events.push(InputEvent::new(EventType::KEY.0, code, i32::from(pressed)));
                    update_held(&mut self.held, code, pressed);
                }
                Action::Button { code, pressed } => {
                    input_events.push(InputEvent::new(EventType::KEY.0, code, i32::from(pressed)));
                    update_held(&mut self.held, code, pressed);
                }
                Action::Move { dx, dy } => {
                    self.pass_net.0 += i64::from(dx);
                    self.pass_net.1 += i64::from(dy);
                    if dx != 0 {
                        input_events.push(InputEvent::new(
                            EventType::RELATIVE.0,
                            RelativeAxisCode::REL_X.0,
                            dx,
                        ));
                    }
                    if dy != 0 {
                        input_events.push(InputEvent::new(
                            EventType::RELATIVE.0,
                            RelativeAxisCode::REL_Y.0,
                            dy,
                        ));
                    }
                }
                Action::Scroll { dx, dy } => {
                    if dx != 0 {
                        input_events.push(InputEvent::new(
                            EventType::RELATIVE.0,
                            RelativeAxisCode::REL_HWHEEL.0,
                            dx,
                        ));
                    }
                    if dy != 0 {
                        input_events.push(InputEvent::new(
                            EventType::RELATIVE.0,
                            RelativeAxisCode::REL_WHEEL.0,
                            dy,
                        ));
                    }
                }
            }
        }

        if input_events.is_empty() {
            return Ok(());
        }
        self.device
            .emit(&input_events)
            .map_err(|error| format!("Could not send input event: {error}"))?;
        // Update the physical set only after a successful write.
        for event in events {
            match event.action {
                Action::Key { code, pressed } | Action::Button { code, pressed } => {
                    if pressed {
                        self.physical.insert(code);
                    } else {
                        self.physical.remove(&code);
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn wait_until(
        &mut self,
        anchor: &mut Instant,
        mut deadline: Instant,
        position_us: u64,
        pass: u32,
    ) -> Result<WaitOutcome, String> {
        loop {
            match self.commands.try_recv() {
                Ok(PlaybackCommand::Stop) => return Ok(WaitOutcome::Cancelled),
                Ok(PlaybackCommand::Pause) => match self.pause_at(position_us)? {
                    PauseOutcome::Stop => return Ok(WaitOutcome::Cancelled),
                    PauseOutcome::Resumed { paused_for } => {
                        *anchor += paused_for;
                        deadline += paused_for;
                    }
                },
                Ok(PlaybackCommand::Resume) => {}
                Err(TryRecvError::Disconnected) => return Ok(WaitOutcome::Cancelled),
                Err(TryRecvError::Empty) => {}
            }

            let now = Instant::now();
            if now >= deadline {
                return Ok(WaitOutcome::Elapsed);
            }

            let timeout = (deadline - now).min(COMMAND_POLL);
            match self.commands.recv_timeout(timeout) {
                Ok(PlaybackCommand::Stop) => return Ok(WaitOutcome::Cancelled),
                Ok(PlaybackCommand::Pause) => match self.pause_at(position_us)? {
                    PauseOutcome::Stop => return Ok(WaitOutcome::Cancelled),
                    PauseOutcome::Resumed { paused_for } => {
                        *anchor += paused_for;
                        deadline += paused_for;
                    }
                },
                Ok(PlaybackCommand::Resume) => {}
                Err(RecvTimeoutError::Timeout) => {
                    self.maybe_progress(position_us, pass);
                }
                Err(RecvTimeoutError::Disconnected) => return Ok(WaitOutcome::Cancelled),
            }
        }
    }

    fn pause_at(&mut self, position_us: u64) -> Result<PauseOutcome, String> {
        self.release_physical();
        let started = Instant::now();
        (self.send)(PlaybackUpdate::Paused { position_us });

        loop {
            match self.commands.recv() {
                Ok(PlaybackCommand::Resume) => {
                    let paused_for = started.elapsed();
                    self.restore_held()?;
                    (self.send)(PlaybackUpdate::Resumed);
                    return Ok(PauseOutcome::Resumed { paused_for });
                }
                Ok(PlaybackCommand::Stop) => return Ok(PauseOutcome::Stop),
                Ok(PlaybackCommand::Pause) => {}
                Err(_) => return Ok(PauseOutcome::Stop),
            }
        }
    }

    fn reset_pointer_to_playback_start(&mut self) -> Result<(), String> {
        let (dx, dy) = self.pass_net;
        self.pass_net = (0, 0);
        if dx == 0 && dy == 0 {
            return Ok(());
        }

        let dx = (-dx).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
        let dy = (-dy).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
        let mut events = Vec::with_capacity(2);
        if dx != 0 {
            events.push(InputEvent::new(
                EventType::RELATIVE.0,
                RelativeAxisCode::REL_X.0,
                dx,
            ));
        }
        if dy != 0 {
            events.push(InputEvent::new(
                EventType::RELATIVE.0,
                RelativeAxisCode::REL_Y.0,
                dy,
            ));
        }
        self.device
            .emit(&events)
            .map_err(|error| format!("Could not reset the pointer for the next loop: {error}"))
    }

    fn release_physical(&mut self) {
        if self.physical.is_empty() {
            return;
        }
        let mut codes = self.physical.drain().collect::<Vec<_>>();
        codes.sort_unstable();
        let events = codes
            .into_iter()
            .map(|code| InputEvent::new(EventType::KEY.0, code, 0))
            .collect::<Vec<_>>();
        let _ = self.device.emit(&events);
    }

    fn restore_held(&mut self) -> Result<(), String> {
        let mut codes = self.held.iter().copied().collect::<Vec<_>>();
        codes.sort_unstable();
        if codes.is_empty() {
            return Ok(());
        }
        let events = codes
            .iter()
            .map(|code| InputEvent::new(EventType::KEY.0, *code, 1))
            .collect::<Vec<_>>();
        self.device
            .emit(&events)
            .map_err(|error| format!("Could not restore held inputs: {error}"))?;
        for code in codes {
            self.physical.insert(code);
        }
        Ok(())
    }

    fn release_all(&mut self) {
        self.release_physical();
        self.held.clear();
    }

    fn maybe_progress(&mut self, position_us: u64, pass: u32) {
        if self.last_progress.elapsed() >= PROGRESS_INTERVAL {
            self.last_progress = Instant::now();
            (self.send)(PlaybackUpdate::Progress {
                position_us,
                pass,
                passes: self.total_passes,
            });
        }
    }
}

enum PauseOutcome {
    Resumed { paused_for: Duration },
    Stop,
}

enum WaitOutcome {
    Elapsed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EventGroup {
    at_us: u64,
    previous_at_us: u64,
    range: Range<usize>,
}

fn group_events(recording: &Recording) -> Vec<EventGroup> {
    let mut groups = Vec::new();
    let mut index = 0;
    let mut previous_at_us = 0;
    while index < recording.events.len() {
        let at_us = recording.events[index].at_us;
        let start = index;
        while index < recording.events.len() && recording.events[index].at_us == at_us {
            index += 1;
        }
        groups.push(EventGroup {
            at_us,
            previous_at_us,
            range: start..index,
        });
        previous_at_us = at_us;
    }
    groups
}

fn scale_micros(micros: u64, speed: f64) -> Duration {
    let scaled = (micros as f64 / speed.max(0.01)).round().max(0.0);
    Duration::from_micros(scaled.min(u64::MAX as f64) as u64)
}

fn update_held(held: &mut HashSet<u16>, code: u16, pressed: bool) {
    if pressed {
        held.insert(code);
    } else {
        held.remove(&code);
    }
}

/// KWin exposes per-device pointer settings over D-Bus. A new virtual pointer
/// gets adaptive acceleration by default; switch it to flat so recorded raw
/// deltas are replayed at a 1:1 ratio. The device is discovered through the
/// InputDeviceManager because the libinput sysname is assigned asynchronously.
fn configure_virtual_pointer() -> Result<(), String> {
    let desktop_is_kde = std::env::var("XDG_CURRENT_DESKTOP")
        .map(|desktop| desktop.to_ascii_lowercase().contains("kde"))
        .unwrap_or(false);
    if !desktop_is_kde {
        return Ok(());
    }

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("Could not start the input configuration runtime: {error}"))?;

    runtime.block_on(async move {
        let connection = ashpd::zbus::Connection::session()
            .await
            .map_err(|error| format!("Could not connect to KWin: {error}"))?;
        let manager = ashpd::zbus::Proxy::new(
            &connection,
            "org.kde.KWin",
            "/org/kde/KWin/InputDevice",
            "org.kde.KWin.InputDeviceManager",
        )
        .await
        .map_err(|error| format!("Could not access KWin input devices: {error}"))?;

        let mut last_error = String::new();
        for _ in 0..50 {
            match manager.call::<_, _, Vec<String>>("ListPointers", &()).await {
                Ok(pointers) => {
                    for sysname in pointers {
                        let path = format!("/org/kde/KWin/InputDevice/{sysname}");
                        let Ok(proxy) = ashpd::zbus::Proxy::new(
                            &connection,
                            "org.kde.KWin",
                            path.as_str(),
                            "org.kde.KWin.InputDevice",
                        )
                        .await
                        else {
                            continue;
                        };
                        let is_virtual = proxy
                            .get_property::<bool>("isVirtual")
                            .await
                            .unwrap_or(false);
                        if !is_virtual {
                            continue;
                        }
                        let name = proxy
                            .get_property::<String>("name")
                            .await
                            .unwrap_or_default();
                        if name != "Mimic Virtual Input" {
                            continue;
                        }
                        let result = async {
                            proxy
                                .set_property("pointerAccelerationProfileFlat", true)
                                .await?;
                            proxy
                                .set_property("pointerAccelerationProfileAdaptive", false)
                                .await?;
                            proxy.set_property("pointerAcceleration", 0.0f64).await?;
                            Ok::<(), ashpd::zbus::Error>(())
                        }
                        .await;
                        match result {
                            Ok(()) => return Ok(()),
                            Err(error) => last_error = error.to_string(),
                        }
                    }
                }
                Err(error) => last_error = error.to_string(),
            }
            tokio::time::sleep(Duration::from_millis(40)).await;
        }
        Err(format!(
            "KWin did not expose the virtual pointer in time: {last_error}"
        ))
    })
}

fn pointer_anchor_delta(target: PointerPoint, current: (i32, i32)) -> Option<(i32, i32)> {
    let dx = i64::from(target.x) - i64::from(current.0);
    let dy = i64::from(target.y) - i64::from(current.1);
    if dx == 0 && dy == 0 {
        return None;
    }
    Some((
        dx.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
        dy.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
    ))
}

fn recording_uses_pointer(recording: &Recording) -> bool {
    recording.events.iter().any(|event| {
        matches!(
            event.action,
            Action::Move { .. } | Action::Scroll { .. } | Action::Button { .. }
        )
    })
}

fn create_virtual_device(recording: &Recording) -> Result<VirtualDevice, String> {
    let mut keys = AttributeSet::<KeyCode>::new();
    let mut needs_pointer_motion = false;
    let mut needs_wheel = false;
    let mut needs_pointer = false;
    for event in &recording.events {
        match event.action {
            Action::Key { code, .. } => {
                keys.insert(KeyCode::new(code));
            }
            Action::Button { code, .. } => {
                keys.insert(KeyCode::new(code));
                needs_pointer = true;
            }
            Action::Move { .. } => {
                needs_pointer_motion = true;
                needs_pointer = true;
            }
            Action::Scroll { .. } => {
                needs_wheel = true;
                needs_pointer = true;
            }
        }
    }

    // A virtual pointer without any buttons is not reliably classified as a
    // pointer by libinput/KWin (movement-only macros would then be dropped).
    if needs_pointer {
        keys.insert(KeyCode::BTN_LEFT);
    }

    let mut axes = AttributeSet::<RelativeAxisCode>::new();
    if needs_pointer_motion {
        axes.insert(RelativeAxisCode::REL_X);
        axes.insert(RelativeAxisCode::REL_Y);
    }
    if needs_wheel {
        axes.insert(RelativeAxisCode::REL_WHEEL);
        axes.insert(RelativeAxisCode::REL_HWHEEL);
    }

    let input_id = InputId::new(BusType::BUS_VIRTUAL, 0x4d49, 0x4d43, 1);
    let mut builder = VirtualDevice::builder()
        .map_err(|error| injection_error(&error))?
        .name("Mimic Virtual Input")
        .input_id(input_id);

    if keys.iter().next().is_some() {
        builder = builder
            .with_keys(&keys)
            .map_err(|error| injection_error(&error))?;
    }
    if axes.iter().next().is_some() {
        builder = builder
            .with_relative_axes(&axes)
            .map_err(|error| injection_error(&error))?;
    }
    if needs_pointer {
        // Without INPUT_PROP_POINTER, libinput/KWin classifies a combined
        // uinput keyboard/pointer as a keyboard. Motion may still arrive, but
        // button events are then not delivered as pointer clicks.
        let mut properties = AttributeSet::<PropType>::new();
        properties.insert(PropType::POINTER);
        builder = builder
            .with_properties(&properties)
            .map_err(|error| injection_error(&error))?;
    }

    builder.build().map_err(|error| injection_error(&error))
}

fn injection_error(error: &io::Error) -> String {
    if error.kind() == io::ErrorKind::PermissionDenied {
        InputError::NoInjectionPermission.to_string()
    } else {
        InputError::Injection(error.to_string()).to_string()
    }
}

/// Builds a batch of input events for tests without writing to a device.
#[cfg(test)]
fn events_for_actions(actions: &[Action]) -> Vec<InputEvent> {
    let mut events = Vec::new();
    for action in actions {
        match *action {
            Action::Key { code, pressed } | Action::Button { code, pressed } => {
                events.push(InputEvent::new(EventType::KEY.0, code, i32::from(pressed)));
            }
            Action::Move { dx, dy } => {
                events.push(InputEvent::new(
                    EventType::RELATIVE.0,
                    RelativeAxisCode::REL_X.0,
                    dx,
                ));
                events.push(InputEvent::new(
                    EventType::RELATIVE.0,
                    RelativeAxisCode::REL_Y.0,
                    dy,
                ));
            }
            Action::Scroll { dx, dy } => {
                events.push(InputEvent::new(
                    EventType::RELATIVE.0,
                    RelativeAxisCode::REL_HWHEEL.0,
                    dx,
                ));
                events.push(InputEvent::new(
                    EventType::RELATIVE.0,
                    RelativeAxisCode::REL_WHEEL.0,
                    dy,
                ));
            }
        }
    }
    events
}

#[cfg(test)]
mod tests;
