//! Reads the absolute compositor cursor position.
//!
//! KWin does not expose `workspace.cursorPos` on its normal D-Bus API, but its
//! scripting interface does. A short-lived KWin script reports the position to
//! a private, uniquely named D-Bus service owned by this process. The script
//! and service exist only for the duration of one query.
//!
//! This runs entirely as the logged-in user. It is used to record the pointer's
//! starting screen coordinate and to put the pointer back at that coordinate
//! before a replay; it does not grant any additional input privileges.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use tokio::sync::oneshot;

static QUERY_GATE: Mutex<()> = Mutex::new(());
static NEXT_QUERY: AtomicU64 = AtomicU64::new(1);

struct CursorReport {
    token: String,
    sender: Mutex<Option<oneshot::Sender<(i32, i32)>>>,
}

#[zbus::interface(name = "io.github.mimic.CursorProbe")]
impl CursorReport {
    #[zbus(name = "Report")]
    fn report(&self, x: i32, y: i32, token: &str) {
        if token != self.token {
            return;
        }
        if let Ok(mut sender) = self.sender.lock() {
            if let Some(sender) = sender.take() {
                let _ = sender.send((x, y));
            }
        }
    }
}

/// Removes the temporary KWin script even when the caller returns early.
struct ScriptFile(PathBuf);

impl Drop for ScriptFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// Returns the current cursor position in KWin's logical workspace coordinates.
///
/// The query is intentionally best-effort: the caller can still record or play
/// relatively if KWin scripting is unavailable. Queries are serialized because
/// they install a short-lived KWin script.
pub fn current_position() -> Result<(i32, i32), String> {
    let desktop_is_kde = std::env::var("XDG_CURRENT_DESKTOP")
        .map(|desktop| desktop.to_ascii_lowercase().contains("kde"))
        .unwrap_or(false);
    if !desktop_is_kde {
        return Err("cursor position queries require KDE Plasma".into());
    }

    // Serialize queries: KWin's `start` method starts all loaded scripts, and a
    // single private service name is used for each short-lived receiver.
    let _gate = QUERY_GATE
        .lock()
        .map_err(|_| "cursor query lock was poisoned".to_owned())?;

    let token = format!(
        "p{}q{}",
        std::process::id(),
        NEXT_QUERY.fetch_add(1, Ordering::Relaxed)
    );
    let service = format!("io.github.mimic.CursorProbe.{token}");
    let object_path = format!("/io/github/mimic/CursorProbe/{token}");
    let interface = "io.github.mimic.CursorProbe";
    let script_name = format!("mimic-cursor-{token}");
    let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let script_path = runtime_dir.join(format!("mimic-cursor-{token}.js"));
    let script = format!(
        "callDBus({service:?}, {object_path:?}, {interface:?}, \"Report\", \
         workspace.cursorPos.x, workspace.cursorPos.y, {token:?});\n"
    );
    fs::write(&script_path, script)
        .map_err(|error| format!("could not create the KWin cursor script: {error}"))?;
    let script_file = ScriptFile(script_path);
    let script_path = script_file.0.to_string_lossy().into_owned();

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("could not start the cursor query runtime: {error}"))?;

    runtime.block_on(async move {
        let connection = zbus::Connection::session()
            .await
            .map_err(|error| format!("could not connect to the session bus: {error}"))?;
        connection
            .request_name(service.as_str())
            .await
            .map_err(|error| format!("could not own the cursor probe service: {error}"))?;

        let (sender, receiver) = oneshot::channel();
        connection
            .object_server()
            .at(
                object_path.as_str(),
                CursorReport {
                    token: token.clone(),
                    sender: Mutex::new(Some(sender)),
                },
            )
            .await
            .map_err(|error| format!("could not register the cursor probe service: {error}"))?;

        let scripting = zbus::Proxy::new(
            &connection,
            "org.kde.KWin",
            "/Scripting",
            "org.kde.kwin.Scripting",
        )
        .await
        .map_err(|error| format!("could not access KWin scripting: {error}"))?;

        scripting
            .call::<_, _, i32>("loadScript", &(script_path.as_str(), script_name.as_str()))
            .await
            .map_err(|error| format!("could not load the cursor script: {error}"))?;
        if let Err(error) = scripting.call::<_, _, ()>("start", &()).await {
            let _ = scripting
                .call::<_, _, bool>("unloadScript", &(script_name.as_str(),))
                .await;
            return Err(format!("could not start the cursor script: {error}"));
        }

        let result = tokio::time::timeout(Duration::from_secs(2), receiver)
            .await
            .map_err(|_| "KWin did not report the cursor position in time".to_owned())?
            .map_err(|_| "the cursor probe was closed before reporting".to_owned());

        // Unload the script on every path, including timeouts. A failure to
        // unload is harmless because the script only performs one call.
        let _ = scripting
            .call::<_, _, bool>("unloadScript", &(script_name.as_str(),))
            .await;

        result
    })
}

#[cfg(test)]
mod tests;
