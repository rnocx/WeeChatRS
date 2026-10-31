//! The Injected sign-in window for Linux builds without one: shipped next to
//! `weechat-rs`, which stays free of WebKitGTK and starts this when asked to sign in.
#[path = "../relay/injected/bridge.rs"]
mod bridge;
#[path = "../relay/injected/web_window.rs"]
mod web_window;

fn main() {
    if web_window::run().is_err() {
        std::process::exit(1);
    }
}
