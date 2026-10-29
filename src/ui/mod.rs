pub mod app;
pub mod ansi;
pub mod theme;
pub mod keybinds;
pub mod fset;
pub(crate) mod wallpaper;
mod sysinfo;

mod settings;
mod input;
mod emoji;
pub(crate) mod secure_storage;
pub(crate) mod fonts;
pub(crate) mod icons;
pub(crate) mod quick_switcher;
#[cfg(target_os = "macos")]
mod vibrancy;
mod url_safety;
pub(crate) mod notify;
mod np;
pub(crate) mod fileshare;
pub(crate) mod ssh_tunnel;
pub(crate) mod server_search;
