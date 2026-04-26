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

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1024.0, 768.0])
            .with_min_inner_size([400.0, 300.0])
            .with_transparent(true)
            .with_icon(icon),
        ..Default::default()
    };

    eframe::run_native(
        "WeeChatRS",
        native_options,
        Box::new(|cc| Box::new(WeeChatApp::new(cc))),
    )
}
