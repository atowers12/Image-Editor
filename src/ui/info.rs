//! Photo info: star rating + pick/reject flag, and a read-only EXIF panel.

use crate::engine::params::{EditParams, Flag};
use crate::imgio::metadata::ExifInfo;

/// Star rating (0..5) and flag controls. Returns true if either changed
/// (so the sidecar gets persisted).
pub fn ratings(ui: &mut egui::Ui, params: &mut EditParams) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        // Stars.
        for star in 1..=5u8 {
            let filled = params.rating >= star;
            let txt = if filled { "★" } else { "☆" };
            let color = if filled {
                egui::Color32::from_rgb(240, 200, 90)
            } else {
                ui.visuals().weak_text_color()
            };
            if ui
                .add(egui::Button::new(egui::RichText::new(txt).size(18.0).color(color)).frame(false))
                .clicked()
            {
                // Clicking the current rating clears it.
                params.rating = if params.rating == star { star - 1 } else { star };
                changed = true;
            }
        }
        ui.separator();
        // Flags.
        let pick = params.flag == Flag::Pick;
        let reject = params.flag == Flag::Reject;
        if ui
            .add(egui::SelectableLabel::new(pick, "⚑ Pick"))
            .clicked()
        {
            params.flag = if pick { Flag::None } else { Flag::Pick };
            changed = true;
        }
        if ui
            .add(egui::SelectableLabel::new(reject, "⚐ Reject"))
            .clicked()
        {
            params.flag = if reject { Flag::None } else { Flag::Reject };
            changed = true;
        }
    });
    changed
}

/// Read-only EXIF panel.
pub fn exif_panel(ui: &mut egui::Ui, exif: &ExifInfo) {
    let rows = exif.rows();
    if rows.is_empty() {
        ui.label(egui::RichText::new("No EXIF metadata").weak().small());
        return;
    }
    egui::Grid::new("exif_grid")
        .num_columns(2)
        .spacing([10.0, 3.0])
        .show(ui, |ui| {
            for (label, value) in rows {
                ui.label(egui::RichText::new(label).weak().small());
                ui.label(egui::RichText::new(value).small());
                ui.end_row();
            }
        });
}
