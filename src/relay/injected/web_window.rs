//! The sign-in window: Injected's official page in the system webview, run as its own
//! process (`weechat-rs --injected-login`, or the `weechat-rs-login` helper on Linux).
//! Its stdout is a private pipe to the app.
use super::bridge::{official_url, parse_reply, SessionReply, ORIGIN};

const SCRIPT: &str = include_str!("web_login.js");

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
    fn navigation_stays_on_injected_and_its_captcha() {
        assert!(navigation_allowed(
            "https://challenges.cloudflare.com/turnstile/v0/api.js"
        ));
        assert!(navigation_allowed("about:srcdoc"));
        assert!(!navigation_allowed("https://example.org"));
    }
}
