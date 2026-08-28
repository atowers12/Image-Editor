//! Welcome screen shown when nothing is open yet: big open buttons plus
//! recently used folders.

use std::path::PathBuf;

pub enum WelcomeAction {
    OpenFolder,
    OpenFile,
    OpenRecent(PathBuf),
}

pub fn show(
    ui: &mut egui::Ui,
    recent: &[PathBuf],
    logo: Option<&egui::TextureHandle>,
) -> Option<WelcomeAction> {
    let mut action = None;
    ui.vertical_centered(|ui| {
        let top_pad = (ui.available_height() * 0.24).max(20.0);
        ui.add_space(top_pad);
        if let Some(logo) = logo {
            ui.add(egui::Image::new(egui::load::SizedTexture::new(
                logo.id(),
                egui::vec2(112.0, 112.0),
            )));
            ui.add_space(10.0);
        }
        ui.label(
            egui::RichText::new("Photo Editor")
                .size(30.0)
                .strong()
                .color(ui.visuals().strong_text_color()),
        );
        ui.label(
            egui::RichText::new("Non-destructive RAW & JPEG editing")
                .size(14.0)
                .weak(),
        );
        ui.add_space(24.0);

        let big = egui::vec2(240.0, 40.0);
        if ui
            .add_sized(big, egui::Button::new("📂  Open Folder…"))
            .clicked()
        {
            action = Some(WelcomeAction::OpenFolder);
        }
        ui.add_space(8.0);
        if ui
            .add_sized(big, egui::Button::new("🖼  Open File…"))
            .clicked()
        {
            action = Some(WelcomeAction::OpenFile);
        }

        if !recent.is_empty() {
            ui.add_space(28.0);
            ui.label(egui::RichText::new("Recent folders").small().weak());
            ui.add_space(4.0);
            for dir in recent {
                let name = dir
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| dir.display().to_string());
                let label = egui::RichText::new(format!("📁 {name}")).size(14.0);
                let resp = ui
                    .add(egui::Button::new(label).frame(false))
                    .on_hover_text(dir.display().to_string());
                if resp.clicked() {
                    action = Some(WelcomeAction::OpenRecent(dir.clone()));
                }
            }
        }
    });
    action
}
