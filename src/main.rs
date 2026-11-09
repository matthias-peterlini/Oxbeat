#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod engine;
mod project;
mod ui;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Oxbeat")
            .with_inner_size([1200.0, 760.0])
            .with_min_inner_size([800.0, 500.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Oxbeat",
        options,
        Box::new(|cc| Ok(Box::new(ui::OxbeatApp::new(cc)))),
    )
}
