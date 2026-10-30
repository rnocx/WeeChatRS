//! The sign-in helper is a separate invocation of this executable, with its own
//! native GUI event loop. Its stdout is a private pipe to the parent process.
use super::login::{ImportState, ImportedSession};
use serde::{Deserialize, Serialize};
use std::sync::mpsc;

const ORIGIN: &str = "https://injected-irc.com";
const MAX_REPLY: usize = 32 * 1024;
const SCRIPT: &str = include_str!("web_login.js");

#[derive(Deserialize, Serialize)]
struct SessionReply {
    token: String,
}
fn official_url(url: &str) -> bool {
    url::Url::parse(url)
        .map(|u| {
            u.scheme() == "https"
                && u.host_str() == Some("injected-irc.com")
                && u.port_or_known_default() == Some(443)
                && u.username().is_empty()
                && u.password().is_none()
        })
        .unwrap_or(false)
}
fn navigation_allowed(url: &str) -> bool {
    official_url(url)
        || matches!(url, "about:blank" | "about:srcdoc")
        || url::Url::parse(url)
            .map(|u| {
                u.scheme() == "https"
                    && u.host_str() == Some("challenges.cloudflare.com")
                    && u.port_or_known_default() == Some(443)
            })
            .unwrap_or(false)
}
fn parse_reply(origin: &str, body: &str) -> Option<SessionReply> {
    if !official_url(origin) || body.len() > MAX_REPLY {
        return None;
    }
    let reply: SessionReply = serde_json::from_str(body).ok()?;
    if reply.token.is_empty()
        || reply.token.len() > 16384
        || reply.token.chars().any(char::is_whitespace)
    {
        return None;
    }
    Some(reply)
}

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

async fn run_child() -> Result<ImportedSession, String> {
    use tokio::io::AsyncReadExt;
    let executable = std::env::current_exe()
        .map_err(|_| "Could not locate WeeChatRS for sign-in.".to_string())?;
    let mut command = tokio::process::Command::new(executable);
    command
        .arg("--injected-login")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    #[cfg(target_os = "windows")]
    command.creation_flags(0x08000000); // CREATE_NO_WINDOW: only the native webview is visible.
    let mut child=command.spawn().map_err(|_|"Could not open the Injected sign-in window. Check that the system webview runtime is installed.".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Could not open sign-in session pipe.".to_string())?;
    let mut bytes = Vec::new();
    let flow = async {
        stdout
            .take((MAX_REPLY + 1) as u64)
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| "Could not read the sign-in result.".to_string())?;
        if bytes.len() > MAX_REPLY {
            return Err("Invalid sign-in response.".to_string());
        }
        let status = child
            .wait()
            .await
            .map_err(|_| "Sign-in window closed unexpectedly.".to_string())?;
        if bytes.is_empty() {
            return Err(if status.success() {"Sign-in cancelled."} else {"The sign-in window could not start. Check your system webview installation, or use session import."}.into());
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

/// Called before initializing eframe, logging, persistence or desktop integration.
/// Never invoked from the UI process's existing event loop.
pub fn run() -> Result<(), String> {
    use tao::{
        event::{Event, WindowEvent},
        event_loop::{ControlFlow, EventLoopBuilder},
        window::WindowBuilder,
    };
    enum LoginEvent {
        Session(SessionReply),
        Open(String),
    }
    let event_loop = EventLoopBuilder::<LoginEvent>::with_user_event().build();
    let window = WindowBuilder::new()
        .with_title("Sign in to Injected — injected-irc.com")
        .with_inner_size(tao::dpi::LogicalSize::new(520.0, 760.0))
        .with_min_inner_size(tao::dpi::LogicalSize::new(420.0, 580.0))
        .build(&event_loop)
        .map_err(|_| "Could not create sign-in window.".to_string())?;
    let ipc_proxy = event_loop.create_proxy();
    let link_proxy = event_loop.create_proxy();
    let builder = wry::WebViewBuilder::new()
        .with_url(format!("{ORIGIN}/#login"))
        .with_incognito(true)
        .with_background_color((10, 16, 24, 255))
        .with_initialization_script(SCRIPT)
        .with_navigation_handler(|url| navigation_allowed(&url))
        .with_new_window_req_handler(move |url, _| {
            if official_url(&url) {
                let _ = link_proxy.send_event(LoginEvent::Open(url));
            }
            wry::NewWindowResponse::Deny
        })
        .with_ipc_handler(move |request| {
            if let Some(reply) = parse_reply(&request.uri().to_string(), request.body()) {
                let _ = ipc_proxy.send_event(LoginEvent::Session(reply));
            }
        });
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    let webview = builder
        .build(&window)
        .map_err(|_| "Could not initialize system webview.".to_string())?;
    #[cfg(target_os = "linux")]
    let webview = {
        use tao::platform::unix::WindowExtUnix;
        use wry::WebViewBuilderExtUnix;
        let container = window
            .default_vbox()
            .ok_or_else(|| "Could not create sign-in container.".to_string())?;
        builder
            .build_gtk(container)
            .map_err(|_| "Could not initialize WebKitGTK.".to_string())?
    };
    let mut returned = false;
    event_loop.run(move |event, _, control| {
        // Keep both alive until the helper exits.
        let _ = (&webview, &window);
        *control = ControlFlow::Wait;
        match event {
            Event::UserEvent(LoginEvent::Session(reply)) if !returned => {
                use std::io::Write;
                returned = true;
                // This pipe is read only by the parent; never put credentials in
                // argv, URLs, debug logs, files, or an egui text label.
                if let Ok(data) = serde_json::to_vec(&reply) {
                    let _ = std::io::stdout().write_all(&data);
                    let _ = std::io::stdout().flush();
                }
                *control = ControlFlow::Exit;
            }
            Event::UserEvent(LoginEvent::Open(url)) => {
                let _ = webbrowser::open(&url);
            }
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => *control = ControlFlow::Exit,
            _ => {}
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bridge_accepts_only_the_official_https_origin() {
        assert!(parse_reply(
            "https://injected-irc.com/#login",
            r#"{"token":"test-token"}"#
        )
        .is_some());
        for origin in [
            "https://injected-irc.com.evil.test",
            "http://injected-irc.com",
            "https://challenges.cloudflare.com",
            "https://injected-irc.com:444",
            "file:///login",
        ] {
            assert!(parse_reply(origin, r#"{"token":"test-token"}"#).is_none());
        }
        assert!(parse_reply(ORIGIN, r#"{"token":""}"#).is_none());
        assert!(parse_reply(ORIGIN, r#"{"token":"bad\ntoken"}"#).is_none());
        assert!(parse_reply(ORIGIN, &"x".repeat(MAX_REPLY + 1)).is_none());
        assert!(navigation_allowed(
            "https://challenges.cloudflare.com/turnstile/v0/api.js"
        ));
        assert!(navigation_allowed("about:srcdoc"));
        assert!(!navigation_allowed("https://example.org"));
    }
}
