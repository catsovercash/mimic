//! Global shortcuts through the XDG Desktop Portal.
//!
//! Wayland deliberately does not allow applications to read the keyboard
//! globally. KDE Plasma 6 implements `org.freedesktop.portal.GlobalShortcuts`,
//! which registers a compositor-level shortcut and delivers activation signals
//! only. That is the supported mechanism used here; Mimic never installs a
//! keyboard hook or assumes unrestricted access.
//!
//! The portal owns the actual key combinations. Users can change them in the
//! portal prompt or through the Configure button, and the bindings persist
//! across runs. The emergency-stop shortcut lives in the compositor, so it
//! still fires while Mimic is inside an infinite playback loop.

use std::thread;

use ashpd::desktop::global_shortcuts::{
    BindShortcutsOptions, ConfigureShortcutsOptions, GlobalShortcuts, ListShortcutsOptions,
    NewShortcut, Shortcut,
};
use ashpd::desktop::CreateSessionOptions;
use ashpd::register_host_app_with_connection;
use ashpd::{AppID, Error as PortalError};
use futures_util::StreamExt;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

use crate::paths;

pub const ACTION_RECORD_TOGGLE: &str = "record-toggle";
pub const ACTION_RECORD_PAUSE: &str = "record-pause";
pub const ACTION_PLAYBACK_TOGGLE: &str = "playback-toggle";
pub const ACTION_EMERGENCY_STOP: &str = "emergency-stop";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyAction {
    ToggleRecording,
    ToggleRecordingPause,
    TogglePlayback,
    EmergencyStop,
}

impl HotkeyAction {
    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            ACTION_RECORD_TOGGLE => Some(Self::ToggleRecording),
            ACTION_RECORD_PAUSE => Some(Self::ToggleRecordingPause),
            ACTION_PLAYBACK_TOGGLE => Some(Self::TogglePlayback),
            ACTION_EMERGENCY_STOP => Some(Self::EmergencyStop),
            _ => None,
        }
    }

    pub fn defaults(self) -> ShortcutDefinition {
        match self {
            Self::ToggleRecording => ShortcutDefinition {
                id: ACTION_RECORD_TOGGLE,
                label: "Start or stop recording",
                preferred_trigger: "CTRL+ALT+SHIFT+r",
            },
            Self::ToggleRecordingPause => ShortcutDefinition {
                id: ACTION_RECORD_PAUSE,
                label: "Pause or resume recording",
                preferred_trigger: "CTRL+ALT+SHIFT+p",
            },
            Self::TogglePlayback => ShortcutDefinition {
                id: ACTION_PLAYBACK_TOGGLE,
                label: "Start or stop playback",
                preferred_trigger: "CTRL+ALT+SHIFT+g",
            },
            Self::EmergencyStop => ShortcutDefinition {
                id: ACTION_EMERGENCY_STOP,
                label: "Emergency stop",
                preferred_trigger: "CTRL+ALT+SHIFT+x",
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShortcutDefinition {
    pub id: &'static str,
    pub label: &'static str,
    pub preferred_trigger: &'static str,
}

pub fn default_shortcut_definitions() -> [ShortcutDefinition; 4] {
    [
        HotkeyAction::ToggleRecording.defaults(),
        HotkeyAction::ToggleRecordingPause.defaults(),
        HotkeyAction::TogglePlayback.defaults(),
        HotkeyAction::EmergencyStop.defaults(),
    ]
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShortcutInfo {
    pub id: String,
    pub label: String,
    pub trigger: String,
}

/// State exposed to the GUI. The portal, not Mimic, owns the key bindings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HotkeyUiState {
    Starting,
    AwaitingApproval,
    Ready(Vec<ShortcutInfo>),
    Unavailable(String),
}

#[derive(Debug, Clone)]
pub enum HotkeyUpdate {
    Starting,
    AwaitingApproval,
    Ready { shortcuts: Vec<ShortcutInfo> },
    ConfigureFailed(String),
    Activated(HotkeyAction),
    Unavailable(String),
}

#[derive(Debug)]
pub enum HotkeyCommand {
    Configure,
    Shutdown,
}

/// Handle to a running portal shortcut service. Dropping the handle asks the
/// service to close its portal session; the application process is about to
/// exit at that point, so a blocked portal dialog cannot delay shutdown.
pub struct HotkeyService {
    commands: UnboundedSender<HotkeyCommand>,
}

impl HotkeyService {
    pub fn configure(&self) {
        let _ = self.commands.send(HotkeyCommand::Configure);
    }

    pub fn shutdown(&self) {
        let _ = self.commands.send(HotkeyCommand::Shutdown);
    }
}

impl Drop for HotkeyService {
    fn drop(&mut self) {
        let _ = self.commands.send(HotkeyCommand::Shutdown);
    }
}

pub fn spawn<F>(send: F) -> HotkeyService
where
    F: Fn(HotkeyUpdate) + Send + 'static,
{
    let (commands, command_rx) = mpsc::unbounded_channel();
    thread::Builder::new()
        .name("mimic-hotkeys".into())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(error) => {
                    (send)(HotkeyUpdate::Unavailable(format!(
                        "Could not start the global shortcut service: {error}"
                    )));
                    return;
                }
            };
            runtime.block_on(run(send, command_rx));
        })
        .expect("failed to spawn hotkey thread");

    HotkeyService { commands }
}

async fn run<F: Fn(HotkeyUpdate)>(send: F, mut commands: UnboundedReceiver<HotkeyCommand>) {
    (send)(HotkeyUpdate::Starting);

    let result = setup_and_listen(&send, &mut commands).await;
    if let Err(message) = result {
        (send)(HotkeyUpdate::Unavailable(message));
    }
}

async fn setup_and_listen<F: Fn(HotkeyUpdate)>(
    send: &F,
    commands: &mut UnboundedReceiver<HotkeyCommand>,
) -> Result<(), String> {
    let connection = ashpd::zbus::Connection::session()
        .await
        .map_err(|error| format!("Could not connect to the session bus: {error}"))?;

    // Registering the host application ID makes KDE persist shortcut bindings
    // for Mimic rather than for a transient D-Bus name. Failure is not fatal;
    // some portal versions work without the registry interface.
    let app_id = AppID::try_from(paths::app_id())
        .map_err(|error| format!("Invalid application ID: {error}"))?;
    let registration_note =
        match register_host_app_with_connection(connection.clone(), app_id).await {
            Ok(()) => None,
            Err(error) => Some(format!("Portal app registration unavailable ({error}).")),
        };

    let portal = GlobalShortcuts::with_connection(connection)
        .await
        .map_err(|error| portal_unavailable_message(&error))?;

    let session = portal
        .create_session(CreateSessionOptions::default())
        .await
        .map_err(|error| {
            format!(
                "Could not create a global shortcut session: {error}. KDE requires an \
                 application ID, so the Mimic desktop entry must be installed (run \
                 scripts/install.sh) and xdg-desktop-portal-kde must be running."
            )
        })?;

    let definitions = default_shortcut_definitions();

    // KDE activates the actions owned by this portal session when
    // BindShortcuts is called. Calling it again on a later session with the
    // same IDs is a no-op for the user (the shortcuts are treated as
    // "returning") but is required for the compositor to keep them live.
    // Bind once per session, always.
    let existing = list_shortcuts(&portal, &session).await.unwrap_or_default();
    let needs_prompt = !definitions
        .iter()
        .all(|definition| existing.iter().any(|shortcut| shortcut.id == definition.id));
    if needs_prompt {
        (send)(HotkeyUpdate::AwaitingApproval);
    }

    let requested = definitions
        .iter()
        .map(|definition| {
            NewShortcut::new(definition.id, definition.label)
                .preferred_trigger(Some(definition.preferred_trigger))
        })
        .collect::<Vec<_>>();
    let request = portal
        .bind_shortcuts(&session, &requested, None, BindShortcutsOptions::default())
        .await
        .map_err(|error| shortcut_error_message(&error))?;
    let response = request
        .response()
        .map_err(|error| shortcut_error_message(&error))?;
    let shortcuts = response.shortcuts().iter().map(to_info).collect::<Vec<_>>();

    let _ = registration_note;
    let info = merge_shortcut_info(shortcuts);
    (send)(HotkeyUpdate::Ready { shortcuts: info });

    // `Session` deliberately hides its object path, but it serializes as the
    // path string. Capture it so signals from another application's session
    // can never be mistaken for ours.
    let session_path = serde_json::to_value(&session)
        .ok()
        .and_then(|value| value.as_str().map(ToOwned::to_owned));

    let mut activated = portal
        .receive_activated()
        .await
        .map_err(|error| format!("Could not listen for global shortcuts: {error}"))?;
    let mut changed = portal
        .receive_shortcuts_changed()
        .await
        .map_err(|error| format!("Could not listen for shortcut changes: {error}"))?;
    let mut closed = session
        .receive_closed()
        .await
        .map_err(|error| format!("Could not monitor the shortcut session: {error}"))?;

    loop {
        tokio::select! {
            maybe_activation = activated.next() => {
                let Some(activation) = maybe_activation else {
                    return Err("The global shortcut session ended.".to_owned());
                };
                if session_path
                    .as_deref()
                    .is_some_and(|path| activation.session_handle().as_str() != path)
                {
                    continue;
                }
                if let Some(action) = HotkeyAction::from_id(activation.shortcut_id()) {
                    (send)(HotkeyUpdate::Activated(action));
                }
            }
            maybe_changed = changed.next() => {
                let Some(change) = maybe_changed else {
                    return Err("The global shortcut session ended.".to_owned());
                };
                if session_path
                    .as_deref()
                    .is_some_and(|path| change.session_handle().as_str() != path)
                {
                    continue;
                }
                let info = merge_shortcut_info(change.shortcuts().iter().map(to_info).collect());
                (send)(HotkeyUpdate::Ready { shortcuts: info });
            }
            maybe_closed = closed.next() => {
                let _ = maybe_closed;
                return Err("The global shortcut session was closed by the desktop.".to_owned());
            }
            command = commands.recv() => {
                match command {
                    Some(HotkeyCommand::Configure) => {
                        if let Err(error) = portal
                            .configure_shortcuts(
                                &session,
                                None,
                                ConfigureShortcutsOptions::default(),
                            )
                            .await
                        {
                            (send)(HotkeyUpdate::ConfigureFailed(format!(
                                "Could not open shortcut settings: {error}"
                            )));
                            continue;
                        }
                        match list_shortcuts(&portal, &session).await {
                            Ok(shortcuts) => {
                                (send)(HotkeyUpdate::Ready {
                                    shortcuts: merge_shortcut_info(shortcuts),
                                });
                            }
                            Err(error) => {
                                (send)(HotkeyUpdate::ConfigureFailed(error));
                            }
                        }
                    }
                    Some(HotkeyCommand::Shutdown) | None => {
                        let _ = session.close().await;
                        return Ok(());
                    }
                }
            }
        }
    }
}

async fn list_shortcuts(
    portal: &GlobalShortcuts,
    session: &ashpd::desktop::Session<GlobalShortcuts>,
) -> Result<Vec<ShortcutInfo>, String> {
    let request = portal
        .list_shortcuts(session, ListShortcutsOptions::default())
        .await
        .map_err(|error| shortcut_error_message(&error))?;
    let response = request
        .response()
        .map_err(|error| shortcut_error_message(&error))?;
    Ok(response.shortcuts().iter().map(to_info).collect())
}

fn to_info(shortcut: &Shortcut) -> ShortcutInfo {
    let definition = default_shortcut_definitions()
        .into_iter()
        .find(|definition| definition.id == shortcut.id());
    let trigger = shortcut.trigger_description();
    ShortcutInfo {
        id: shortcut.id().to_owned(),
        label: definition
            .map(|definition| definition.label.to_owned())
            .unwrap_or_else(|| shortcut.description().to_owned()),
        trigger: if trigger.is_empty() {
            "Not assigned".to_owned()
        } else {
            trigger.to_owned()
        },
    }
}

/// Keep a stable order in the UI and add friendly labels for shortcuts the
/// portal did not return (for example, if the user unassigned one).
fn merge_shortcut_info(mut actual: Vec<ShortcutInfo>) -> Vec<ShortcutInfo> {
    let definitions = default_shortcut_definitions();
    let mut result = Vec::with_capacity(definitions.len());
    for definition in definitions {
        if let Some(existing) = actual
            .iter()
            .position(|shortcut| shortcut.id == definition.id)
        {
            result.push(actual.remove(existing));
        } else {
            result.push(ShortcutInfo {
                id: definition.id.to_owned(),
                label: definition.label.to_owned(),
                trigger: "Not assigned".to_owned(),
            });
        }
    }
    result
}

fn portal_unavailable_message(error: &PortalError) -> String {
    format!(
        "Global shortcuts are unavailable. Install xdg-desktop-portal and \
         xdg-desktop-portal-kde, then try again. ({error})"
    )
}

fn shortcut_error_message(error: &PortalError) -> String {
    let text = error.to_string();
    if text.to_ascii_lowercase().contains("cancel") {
        "Global shortcut setup was cancelled. Use Settings → Configure shortcuts to try again."
            .to_owned()
    } else {
        format!("Global shortcut portal error: {error}")
    }
}

#[cfg(test)]
mod tests;
