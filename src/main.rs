// Hide the console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() -> eframe::Result<()> {
    let icon = photo_editor::branding::icon_data(256).unwrap_or_default();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([1000.0, 620.0])
            .with_title("Photo Editor")
            .with_icon(icon),
        ..Default::default()
    };
    eframe::run_native(
        "Photo Editor",
        options,
        Box::new(|cc| Ok(Box::new(photo_editor::app::App::new(cc)))),
    )
}
