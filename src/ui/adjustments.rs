//! The right-hand adjustment panel: Lightroom-style grouped sliders.
//! Returns true when any value changed this frame. Double-click a slider
//! (or use its right-click menu) to reset it to zero.

use crate::engine::ops::color::hsl_to_rgb;
use crate::engine::params::{EditParams, HSL_BAND_HUES, HSL_BAND_NAMES};

pub fn show(ui: &mut egui::Ui, params: &mut EditParams, active_band: &mut usize) -> bool {
    let mut changed = false;
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            section(ui, "Light", |ui| {
                grid(ui, "light_grid", |ui| {
                    slider_row(ui, &mut changed, "Exposure", &mut params.exposure, -5.0..=5.0, 2);
                    slider_row(ui, &mut changed, "Contrast", &mut params.contrast, -100.0..=100.0, 0);
                    slider_row(ui, &mut changed, "Highlights", &mut params.highlights, -100.0..=100.0, 0);
                    slider_row(ui, &mut changed, "Shadows", &mut params.shadows, -100.0..=100.0, 0);
                    slider_row(ui, &mut changed, "Whites", &mut params.whites, -100.0..=100.0, 0);
                    slider_row(ui, &mut changed, "Blacks", &mut params.blacks, -100.0..=100.0, 0);
                });
            });

            section(ui, "Color", |ui| {
                grid(ui, "color_grid", |ui| {
                    slider_row(ui, &mut changed, "Temp", &mut params.temp, -100.0..=100.0, 0);
                    slider_row(ui, &mut changed, "Tint", &mut params.tint, -100.0..=100.0, 0);
                    slider_row(ui, &mut changed, "Vibrance", &mut params.vibrance, -100.0..=100.0, 0);
                    slider_row(ui, &mut changed, "Saturation", &mut params.saturation, -100.0..=100.0, 0);
                });
            });

            section(ui, "Color Mixer", |ui| {
                band_picker(ui, active_band);
                ui.add_space(4.0);
                let band = &mut params.hsl[*active_band];
                grid(ui, "mixer_grid", |ui| {
                    slider_row(ui, &mut changed, "Hue", &mut band.hue, -100.0..=100.0, 0);
                    slider_row(ui, &mut changed, "Saturation", &mut band.sat, -100.0..=100.0, 0);
                    slider_row(ui, &mut changed, "Luminance", &mut band.lum, -100.0..=100.0, 0);
                });
            });

            section(ui, "Effects", |ui| {
                grid(ui, "effects_grid", |ui| {
                    slider_row(ui, &mut changed, "Texture", &mut params.texture, -100.0..=100.0, 0);
                    slider_row(ui, &mut changed, "Clarity", &mut params.clarity, -100.0..=100.0, 0);
                    slider_row(ui, &mut changed, "Dehaze", &mut params.dehaze, -100.0..=100.0, 0);
                    slider_row(ui, &mut changed, "Vignette", &mut params.vignette, -100.0..=100.0, 0);
                });
            });
        });
    changed
}

fn section(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    egui::CollapsingHeader::new(egui::RichText::new(title).strong())
        .default_open(true)
        .show(ui, add);
    ui.add_space(2.0);
}

fn grid(ui: &mut egui::Ui, id: &str, add: impl FnOnce(&mut egui::Ui)) {
    egui::Grid::new(id)
        .num_columns(2)
        .min_col_width(64.0)
        .spacing([6.0, 6.0])
        .show(ui, add);
}

fn slider_row(
    ui: &mut egui::Ui,
    changed: &mut bool,
    label: &str,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    decimals: usize,
) {
    ui.label(label);
    let slider_width = (ui.available_width() - 60.0).max(80.0);
    ui.spacing_mut().slider_width = slider_width;
    let resp = ui.add(egui::Slider::new(value, range).fixed_decimals(decimals));
    if resp.double_clicked() {
        *value = 0.0;
        *changed = true;
    } else if resp.changed() {
        *changed = true;
    }
    resp.context_menu(|ui| {
        if ui.button("Reset").clicked() {
            *value = 0.0;
            *changed = true;
            ui.close_menu();
        }
    });
    ui.end_row();
}

/// Row of 8 color chips to pick which mixer band the H/S/L sliders target.
fn band_picker(ui: &mut egui::Ui, active_band: &mut usize) {
    ui.horizontal_wrapped(|ui| {
        for i in 0..8 {
            let (r, g, b) = hsl_to_rgb(HSL_BAND_HUES[i], 0.75, 0.5);
            let color = egui::Color32::from_rgb(
                (r * 255.0) as u8,
                (g * 255.0) as u8,
                (b * 255.0) as u8,
            );
            let selected = *active_band == i;
            let size = egui::vec2(22.0, 22.0);
            let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
            let rounding = 4.0;
            ui.painter().rect_filled(rect.shrink(2.0), rounding, color);
            if selected {
                ui.painter().rect_stroke(
                    rect,
                    rounding,
                    egui::Stroke::new(2.0, ui.visuals().strong_text_color()),
                    egui::StrokeKind::Outside,
                );
            }
            if resp.clicked() {
                *active_band = i;
            }
            resp.on_hover_text(HSL_BAND_NAMES[i]);
        }
    });
    ui.label(
        egui::RichText::new(HSL_BAND_NAMES[*active_band])
            .small()
            .weak(),
    );
}
