//! Web sign-in from the app's side: start the sign-in window's process and take the
//! session it hands back. The window is this executable again with `--injected-login`,
//! or on Linux builds without it, the `weechat-rs-login` helper next to it, which alone
//! links WebKitGTK.
use super::bridge::{parse_reply, MAX_REPLY, ORIGIN};
use super::login::{ImportState, ImportedSession};
use std::sync::mpsc;

pub fn start(state: &mut ImportState, ctx: egui::Context) {
    // Dropping the prior state cancels and reaps any previous helper.
    *state = ImportState::default();
    let (tx, rx) = mpsc::channel();
    let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
    state.receiver = Some(rx);
    state.cancel = Some(cancel_tx);
    state.message = Some(
        "Complete sign-in in the Injected window, including any CAPTCHA or second-factor prompt."
            .into(),
    );
    tokio::spawn(async move {
        let result = tokio::select! {
            result=run_child()=>result,
            _=cancel_rx=>return,
        };
        let _ = tx.send(result);
        ctx.request_repaint();
    });
}

/// The command that opens the sign-in window.
#[cfg(feature = "injected-web-login")]
fn window_command() -> Result<tokio::process::Command, String> {
    let executable = std::env::current_exe()
        .map_err(|_| "Could not locate WeeChatRS for sign-in.".to_string())?;
    let mut command = tokio::process::Command::new(executable);
    command.arg("--injected-login");
    Ok(command)
}

/// The command that opens the sign-in window: the helper next to the executable,
/// as the Linux zip and install.sh place it.
#[cfg(not(feature = "injected-web-login"))]
fn window_command() -> Result<tokio::process::Command, String> {
    let executable = std::env::current_exe()
        .map_err(|_| "Could not locate WeeChatRS for sign-in.".to_string())?;
    let executable = executable.canonicalize().unwrap_or(executable);
    let helper = executable.with_file_name(HELPER);
    if !helper.is_file() {
        return Err(format!("The sign-in helper ({HELPER}) isn't next to WeeChatRS. It comes in the Linux zip, and install.sh installs both."));
    }
    Ok(tokio::process::Command::new(helper))
}

#[cfg(not(feature = "injected-web-login"))]
const HELPER: &str = "weechat-rs-login";

/// Why the window's process didn't start, from what it wrote to stderr: on Linux,
/// most likely WebKitGTK 4.1 isn't installed.
fn start_failure(stderr: &[u8]) -> String {
    let stderr = String::from_utf8_lossy(stderr);
    if stderr.contains("libwebkit2gtk-4.1") || stderr.contains("libjavascriptcoregtk-4.1") || stderr.contains("error while loading shared libraries") {
        "The sign-in window needs WebKitGTK 4.1. Install libwebkit2gtk-4.1-0 (Debian, Ubuntu), webkit2gtk4.1 (Fedora) or webkit2gtk-4.1 (Arch), or use session import.".into()
    } else {
        "The sign-in window could not start. Check your system webview installation, or use session import.".into()
    }
}

async fn run_child() -> Result<ImportedSession, String> {
    use tokio::io::AsyncReadExt;
    let mut command = window_command()?;
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    #[cfg(target_os = "windows")]
    command.creation_flags(0x08000000); // CREATE_NO_WINDOW: only the native webview is visible.
    let mut child=command.spawn().map_err(|_|"Could not open the Injected sign-in window. Check that the system webview runtime is installed.".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Could not open sign-in session pipe.".to_string())?;
    let mut stderr = child.stderr.take();
    let mut bytes = Vec::new();
    let mut errors = Vec::new();
    let flow = async {
        // stderr is only read for why the window didn't start; WebKit may write a lot
        // there, so all of it is read and the start kept.
        let read_errors = async {
            if let Some(stderr) = stderr.as_mut() {
                let mut chunk = [0u8; 4096];
                while let Ok(n @ 1..) = stderr.read(&mut chunk).await {
                    if errors.len() < 8192 { errors.extend_from_slice(&chunk[..n]); }
                }
            }
        };
        let mut reply = stdout.take((MAX_REPLY + 1) as u64);
        let (read, ()) = tokio::join!(reply.read_to_end(&mut bytes), read_errors);
        read.map_err(|_| "Could not read the sign-in result.".to_string())?;
        if bytes.len() > MAX_REPLY {
            return Err("Invalid sign-in response.".to_string());
        }
        let status = child
            .wait()
            .await
            .map_err(|_| "Sign-in window closed unexpectedly.".to_string())?;
        if bytes.is_empty() {
            return Err(if status.success() { "Sign-in cancelled.".into() } else { start_failure(&errors) });
        }
        if !status.success() {
            return Err("Sign-in window closed unexpectedly.".into());
        }
        let body =
            std::str::from_utf8(&bytes).map_err(|_| "Invalid sign-in response.".to_string())?;
        let reply =
            parse_reply(ORIGIN, body).ok_or_else(|| "Invalid sign-in response.".to_string())?;
        // The native IPC origin and JSON are checked above. The server remains
        // authoritative for the account name and token validity.
        super::login::verify_session(reply.token).await
    };
    tokio::time::timeout(std::time::Duration::from_secs(600), flow)
        .await
        .map_err(|_| "Sign-in timed out. Please try again.".to_string())?
}

