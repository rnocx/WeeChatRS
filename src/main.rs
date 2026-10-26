#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod relay;
mod ui;

use ui::app::WeeChatApp;

fn load_icon() -> egui::IconData {
    let bytes = include_bytes!("../assets/icon.png");
    let img = image::load_from_memory(bytes)
        .expect("invalid icon")
        .into_rgba8();
    let (w, h) = img.dimensions();
    egui::IconData { rgba: img.into_raw(), width: w, height: h }
}

#[tokio::main]
async fn main() -> eframe::Result<()> {
    env_logger::init();

    let icon = load_icon();

    let viewport = egui::ViewportBuilder::default()
        .with_inner_size([1024.0, 768.0])
        .with_min_inner_size([400.0, 300.0])
        .with_transparent(true)
        // Linux: Wayland app_id / X11 WM_CLASS, matched against weechatrs.desktop.
        .with_app_id("weechatrs")
        .with_icon(icon);

    // On macOS a transparent window also makes the native titlebar see-through.
    // Extend content under it instead and let the app paint an opaque bar there.
    #[cfg(target_os = "macos")]
    let viewport = viewport
        .with_fullsize_content_view(true)
        .with_titlebar_shown(false)
        .with_title_shown(false);

    let native_options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "WeeChatRS",
        native_options,
        Box::new(|cc| Ok(Box::new(WeeChatApp::new(cc)))),
    )
}
