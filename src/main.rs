// main.rs
/// ClipSync-RS: Cross-platform clipboard sync with peer-to-peer discovery
use eframe::egui;

mod app;
mod network;
mod types;
mod clipboard;

use app::ClipSyncApp;

fn main() -> Result<(), eframe::Error> {
    env_logger::init();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([560.0, 640.0])
            .with_title("ClipSync-RS")
            .with_resizable(false)
            .with_min_inner_size([560.0, 640.0]),
        ..Default::default()
    };

    eframe::run_native(
        "ClipSync-RS",
        options,
        Box::new(|cc| Ok(Box::new(ClipSyncApp::new(cc)))),
    )
}
