mod relay;
mod ui;

use ui::app::WeeChatApp;

#[tokio::main]
async fn main() -> eframe::Result<()> {
    env_logger::init();

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1024.0, 768.0])
            .with_min_inner_size([400.0, 300.0])
            .with_transparent(true), // Essential for OS-level transparency
        ..Default::default()
    };

    eframe::run_native(
        "WeeChatRS",
        native_options,
        Box::new(|cc| Box::new(WeeChatApp::new(cc))),
    )
}
