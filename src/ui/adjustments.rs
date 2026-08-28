//! The right-hand adjustment panel: Lightroom-style grouped sliders.
//! Returns true when any value changed this frame. Double-click a slider
//! (or use its right-click menu) to reset it to its default.

use crate::engine::ops::color::hsl_to_rgb;
use crate::engine::params::{CurveChannel, EditParams, HSL_BAND_HUES, HSL_BAND_NAMES};
use crate::ui::curve;

pub struct AdjustOutput {
    pub changed: bool,
    /// User toggled the white-balance eyedropper this frame.
    pub eyedropper_toggled: bool,
}

pub fn show(
    ui: &mut egui::Ui,
    params: &mut EditParams,
    active_band: &mut usize,
    curve_channel: &mut CurveChannel,
    eyedropper_active: bool,
) -> AdjustOutput {
    let mut changed = false;
    let mut eyedropper_toggled = false;
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            section(ui, "Light", |ui| {
                grid(ui, "light_grid", |ui| {
                    slider_row(
                        ui,
                        &mut changed,
                        "Exposure",
                        &mut params.exposure,
                        -5.0..=5.0,
                        2,
                        0.0,
                    );
                    slider_row(
                        ui,
                        &mut changed,
                        "Contrast",
                        &mut params.contrast,
                        -100.0..=100.0,
                        0,
                        0.0,
                    );
                    slider_row(
                        ui,
                        &mut changed,
                        "Highlights",
                        &mut params.highlights,
                        -100.0..=100.0,
                        0,
                        0.0,
                    );
                    slider_row(
                        ui,
                        &mut changed,
                        "Shadows",
                        &mut params.shadows,
                        -100.0..=100.0,
                        0,
                        0.0,
                    );
                    slider_row(
                        ui,
                        &mut changed,
                        "Whites",
                        &mut params.whites,
                        -100.0..=100.0,
                        0,
                        0.0,
                    );
                    slider_row(
                        ui,
                        &mut changed,
                        "Blacks",
                        &mut params.blacks,
                        -100.0..=100.0,
                        0,
                        0.0,
                    );
                });
            });

            section(ui, "Tone Curve", |ui| {
                if curve::show(ui, params, curve_channel) {
                    changed = true;
                }
            });

            section(ui, "Levels", |ui| {
                grid(ui, "levels_grid", |ui| {
                    slider_row(
                        ui,
                        &mut changed,
                        "In Black",
                        &mut params.lv_in_black,
                        0.0..=0.45,
                        2,
                        0.0,
                    );
                    slider_row(
                        ui,
                        &mut changed,
                        "In White",
                        &mut params.lv_in_white,
                        0.55..=1.0,
                        2,
                        1.0,
                    );
                    gamma_row(ui, &mut changed, &mut params.lv_gamma);
                    slider_row(
                        ui,
                        &mut changed,
                        "Out Black",
                        &mut params.lv_out_black,
                        0.0..=0.45,
                        2,
                        0.0,
                    );
                    slider_row(
                        ui,
                        &mut changed,
                        "Out White",
                        &mut params.lv_out_white,
                        0.55..=1.0,
                        2,
                        1.0,
                    );
                });
            });

            section(ui, "Color", |ui| {
                ui.horizontal(|ui| {
                    if ui
                        .add(egui::SelectableLabel::new(
                            eyedropper_active,
                            "💧 WB picker",
                        ))
                        .on_hover_text("Click a neutral gray in the photo to set white balance")
                        .clicked()
                    {
                        eyedropper_toggled = true;
                    }
                });
                grid(ui, "color_grid", |ui| {
                    slider_row(
                        ui,
                        &mut changed,
                        "Temp",
                        &mut params.temp,
                        -100.0..=100.0,
                        0,
                        0.0,
                    );
                    slider_row(
                        ui,
                        &mut changed,
                        "Tint",
                        &mut params.tint,
                        -100.0..=100.0,
                        0,
                        0.0,
                    );
                    slider_row(
                        ui,
                        &mut changed,
                        "Vibrance",
                        &mut params.vibrance,
                        -100.0..=100.0,
                        0,
                        0.0,
                    );
                    slider_row(
                        ui,
                        &mut changed,
                        "Saturation",
                        &mut params.saturation,
                        -100.0..=100.0,
                        0,
                        0.0,
                    );
                });
            });

            section(ui, "Color Mixer", |ui| {
                band_picker(ui, active_band);
                ui.add_space(4.0);
                let band = &mut params.hsl[*active_band];
                grid(ui, "mixer_grid", |ui| {
                    slider_row(
                        ui,
                        &mut changed,
                        "Hue",
                        &mut band.hue,
                        -100.0..=100.0,
                        0,
                        0.0,
                    );
                    slider_row(
                        ui,
                        &mut changed,
                        "Saturation",
                        &mut band.sat,
                        -100.0..=100.0,
                        0,
                        0.0,
                    );
                    slider_row(
                        ui,
                        &mut changed,
                        "Luminance",
                        &mut band.lum,
                        -100.0..=100.0,
                        0,
                        0.0,
                    );
                });
            });

            section(ui, "Detail", |ui| {
                grid(ui, "detail_grid", |ui| {
                    slider_row(
                        ui,
                        &mut changed,
                        "Texture",
                        &mut params.texture,
                        -100.0..=100.0,
                        0,
                        0.0,
                    );
                    slider_row(
                        ui,
                        &mut changed,
                        "Clarity",
                        &mut params.clarity,
                        -100.0..=100.0,
                        0,
                        0.0,
                    );
                    slider_row(
                        ui,
                        &mut changed,
                        "Sharpen",
                        &mut params.sharpen,
                        0.0..=100.0,
                        0,
                        0.0,
                    );
                    slider_row(
                        ui,
                        &mut changed,
                        "  Radius",
                        &mut params.sharpen_radius,
                        0.5..=3.0,
                        1,
                        1.0,
                    );
                    slider_row(
                        ui,
                        &mut changed,
                        "Luminance NR",
                        &mut params.luminance_nr,
                        0.0..=100.0,
                        0,
                        0.0,
                    );
                    slider_row(
                        ui,
                        &mut changed,
                        "Color NR",
                        &mut params.color_nr,
                        0.0..=100.0,
                        0,
                        0.0,
                    );
                });
            });

            section(ui, "Effects", |ui| {
                grid(ui, "effects_grid", |ui| {
                    slider_row(
                        ui,
                        &mut changed,
                        "Dehaze",
                        &mut params.dehaze,
                        -100.0..=100.0,
                        0,
                        0.0,
                    );
                    slider_row(
                        ui,
                        &mut changed,
                        "Vignette",
                        &mut params.vignette,
                        -100.0..=100.0,
                        0,
                        0.0,
                    );
                });
            });
        });
    AdjustOutput {
        changed,
        eyedropper_toggled,
    }
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
    reset: f32,
) {
    ui.label(label);
    let slider_width = (ui.available_width() - 60.0).max(80.0);
    ui.spacing_mut().slider_width = slider_width;
    let resp = ui.add(egui::Slider::new(value, range).fixed_decimals(decimals));
    handle_reset(&resp, value, reset, changed);
    ui.end_row();
}

/// Gamma needs a logarithmic slider centered on 1.0.
fn gamma_row(ui: &mut egui::Ui, changed: &mut bool, value: &mut f32) {
    ui.label("Gamma");
    let slider_width = (ui.available_width() - 60.0).max(80.0);
    ui.spacing_mut().slider_width = slider_width;
    let resp = ui.add(
        egui::Slider::new(value, 0.2..=4.0)
            .logarithmic(true)
            .fixed_decimals(2),
    );
    handle_reset(&resp, value, 1.0, changed);
    ui.end_row();
}

fn handle_reset(resp: &egui::Response, value: &mut f32, reset: f32, changed: &mut bool) {
    if resp.double_clicked() {
        *value = reset;
        *changed = true;
    } else if resp.changed() {
        *changed = true;
    }
    resp.context_menu(|ui| {
        if ui.button("Reset").clicked() {
            *value = reset;
            *changed = true;
            ui.close_menu();
        }
    });
}

/// Row of 8 color chips to pick which mixer band the H/S/L sliders target.
fn band_picker(ui: &mut egui::Ui, active_band: &mut usize) {
    ui.horizontal_wrapped(|ui| {
        for i in 0..8 {
            let (r, g, b) = hsl_to_rgb(HSL_BAND_HUES[i], 0.75, 0.5);
            let color =
                egui::Color32::from_rgb((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8);
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
