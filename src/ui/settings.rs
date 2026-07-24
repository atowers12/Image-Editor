//! Settings window: the user-tunable processing constants (Tuning).
//! Returns true when anything changed so the app can re-render and persist.

use crate::engine::tuning::Tuning;

pub fn show(ctx: &egui::Context, open: &mut bool, tuning: &mut Tuning) -> bool {
    if !*open {
        return false;
    }
    let mut changed = false;
    egui::Window::new("Processing Settings")
        .open(open)
        .collapsible(false)
        .resizable(false)
        .default_width(360.0)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.label(
                egui::RichText::new(
                    "How strongly each effect responds at full slider. Applies to all photos; saved automatically.",
                )
                .small()
                .weak(),
            );
            ui.add_space(6.0);

            egui::Grid::new("tuning_grid")
                .num_columns(2)
                .min_col_width(110.0)
                .spacing([8.0, 6.0])
                .show(ui, |ui| {
                    changed |= row(ui, "Tone range strength", &mut tuning.tone_range_strength, 0.10..=0.60, 2);
                    changed |= row(ui, "Texture strength", &mut tuning.texture_strength, 0.10..=1.50, 2);
                    changed |= row_per_mille(ui, "Texture radius", &mut tuning.texture_radius, 0.5..=5.0);
                    changed |= row(ui, "Clarity strength", &mut tuning.clarity_strength, 0.10..=1.50, 2);
                    changed |= row_per_mille(ui, "Clarity radius", &mut tuning.clarity_radius, 3.0..=30.0);
                    changed |= row(ui, "Dehaze strength", &mut tuning.dehaze_strength, 0.10..=0.60, 2);
                    changed |= row(ui, "Dehaze saturation", &mut tuning.dehaze_sat, 0.0..=0.50, 2);
                    changed |= row(ui, "Vignette strength", &mut tuning.vignette_strength, 0.20..=1.50, 2);
                    changed |= row(ui, "Vignette midpoint", &mut tuning.vignette_midpoint, 0.0..=0.80, 2);
                    changed |= row(ui, "Vignette feather", &mut tuning.vignette_feather, 0.05..=1.00, 2);

                    ui.label("Preview size (px)");
                    let resp = ui.add(
                        egui::Slider::new(&mut tuning.preview_edge, 1000..=3200).step_by(100.0),
                    );
                    if resp.changed() {
                        changed = true;
                    }
                    resp.on_hover_text("Long edge of the interactive preview. Applies when a photo is reopened.");
                    ui.end_row();
                });

            ui.add_space(8.0);
            if ui.button("Restore defaults").clicked() {
                *tuning = Tuning::default();
                changed = true;
            }
        });
    changed
}

fn row(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    decimals: usize,
) -> bool {
    ui.label(label);
    let changed = ui
        .add(egui::Slider::new(value, range).fixed_decimals(decimals))
        .changed();
    ui.end_row();
    changed
}

/// Radii are stored as a fraction of the image's long edge; display them
/// per-mille so the numbers are readable (e.g. 1.5‰ instead of 0.0015).
fn row_per_mille(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f32,
    range_pm: std::ops::RangeInclusive<f32>,
) -> bool {
    ui.label(label);
    let mut pm = *value * 1000.0;
    let changed = ui
        .add(
            egui::Slider::new(&mut pm, range_pm)
                .fixed_decimals(1)
                .suffix("‰"),
        )
        .changed();
    if changed {
        *value = pm / 1000.0;
    }
    ui.end_row();
    changed
}
