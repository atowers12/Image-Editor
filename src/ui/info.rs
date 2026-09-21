//! Photo info: star rating + pick/reject flag, the floating cull bar over the
//! preview, and a read-only EXIF panel.

use crate::engine::params::{EditParams, Flag};
use crate::imgio::metadata::ExifInfo;

/// What the cull bar wants the app to do.
#[derive(Default)]
pub struct CullAction {
    /// Rating or flag changed — persist it.
    pub changed: bool,
    /// Step this many photos through the folder (-1 back, +1 forward).
    pub step: isize,
}

/// Rating, flagging and prev/next in one compact strip, floated over the
/// bottom of the preview so culling never needs the right panel. Dims when
/// the pointer is elsewhere so it doesn't sit on the photo demanding
/// attention.
pub fn cull_bar(
    ui: &mut egui::Ui,
    params: &mut EditParams,
    auto_advance: &mut bool,
    at_start: bool,
    at_end: bool,
) -> CullAction {
    let mut out = CullAction::default();
    ui.horizontal(|ui| {
        if ui
            .add_enabled(!at_start, egui::Button::new("‹").frame(false))
            .on_hover_text("Previous photo (←)")
            .clicked()
        {
            out.step = -1;
        }

        // Stars. Clicking the star that is already the rating clears back to
        // it minus one, matching the panel's control.
        for star in 1..=5u8 {
            let filled = params.rating >= star;
            let color = if filled {
                egui::Color32::from_rgb(240, 200, 90)
            } else {
                ui.visuals().weak_text_color()
            };
            let txt = if filled { "★" } else { "☆" };
            if ui
                .add(
                    egui::Button::new(egui::RichText::new(txt).size(17.0).color(color))
                        .frame(false),
                )
                .on_hover_text(format!("{star} star{} ({star})", if star == 1 { "" } else { "s" }))
                .clicked()
            {
                params.rating = if params.rating == star { star - 1 } else { star };
                out.changed = true;
            }
        }

        ui.separator();
        let pick = params.flag == Flag::Pick;
        let reject = params.flag == Flag::Reject;
        if ui
            .add(egui::SelectableLabel::new(pick, "⚑"))
            .on_hover_text("Pick (P)")
            .clicked()
        {
            params.flag = if pick { Flag::None } else { Flag::Pick };
            out.changed = true;
        }
        if ui
            .add(egui::SelectableLabel::new(reject, "⚐"))
            .on_hover_text("Reject (X)")
            .clicked()
        {
            params.flag = if reject { Flag::None } else { Flag::Reject };
            out.changed = true;
        }
        if ui
            .add_enabled(
                params.flag != Flag::None,
                egui::Button::new("⊘").frame(false),
            )
            .on_hover_text("Clear flag (U)")
            .clicked()
        {
            params.flag = Flag::None;
            out.changed = true;
        }

        ui.separator();
        ui.toggle_value(auto_advance, "⏭")
            .on_hover_text("Auto advance: move to the next photo after every rating or flag (hold shift for one-off)");

        if ui
            .add_enabled(!at_end, egui::Button::new("›").frame(false))
            .on_hover_text("Next photo (→)")
            .clicked()
        {
            out.step = 1;
        }
    });
    out
}

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
                .add(
                    egui::Button::new(egui::RichText::new(txt).size(18.0).color(color))
                        .frame(false),
                )
                .clicked()
            {
                // Clicking the current rating clears it.
                params.rating = if params.rating == star {
                    star - 1
                } else {
                    star
                };
                changed = true;
            }
        }
        ui.separator();
        // Flags.
        let pick = params.flag == Flag::Pick;
        let reject = params.flag == Flag::Reject;
        if ui.add(egui::SelectableLabel::new(pick, "⚑ Pick")).clicked() {
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
