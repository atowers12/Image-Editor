//! Local adjustments (masking) panel. Manages the list of masks and edits
//! the selected mask's local adjustments. The mask geometry is drawn and
//! dragged in the preview (see preview.rs).

use crate::engine::params::{EditParams, LocalAdjust, Mask, MaskKind};

/// Brush tool settings, shared with the preview while painting.
#[derive(Clone, Copy)]
pub struct BrushSettings {
    pub radius: f32,
    pub hardness: f32,
    pub erase: bool,
}

impl Default for BrushSettings {
    fn default() -> Self {
        Self {
            radius: 0.08,
            hardness: 0.5,
            erase: false,
        }
    }
}

pub enum MaskAction {
    None,
    Changed,
    Done,
}

pub fn panel(
    ui: &mut egui::Ui,
    params: &mut EditParams,
    selected: &mut Option<usize>,
    brush: &mut BrushSettings,
) -> MaskAction {
    let mut action = MaskAction::None;

    ui.heading("Masks");
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        if ui.button("+ Linear").clicked() {
            add_mask(
                params,
                selected,
                MaskKind::Linear {
                    p0: [0.5, 0.8],
                    p1: [0.5, 0.2],
                },
                "Linear",
            );
            action = MaskAction::Changed;
        }
        if ui.button("+ Radial").clicked() {
            add_mask(
                params,
                selected,
                MaskKind::Radial {
                    center: [0.5, 0.5],
                    radius: [0.3, 0.3],
                    feather: 0.5,
                },
                "Radial",
            );
            action = MaskAction::Changed;
        }
        if ui.button("+ Brush").clicked() {
            add_mask(params, selected, MaskKind::Brush { dabs: Vec::new() }, "Brush");
            action = MaskAction::Changed;
        }
    });

    ui.add_space(6.0);
    // Mask list.
    let mut delete: Option<usize> = None;
    for i in 0..params.masks.len() {
        ui.horizontal(|ui| {
            let m = &mut params.masks[i];
            if ui.checkbox(&mut m.enabled, "").changed() {
                action = MaskAction::Changed;
            }
            let is_sel = *selected == Some(i);
            let label = format!("{} — {}", m.name, m.kind.type_name());
            if ui.selectable_label(is_sel, label).clicked() {
                *selected = Some(i);
            }
            if ui.small_button("🗑").on_hover_text("Delete mask").clicked() {
                delete = Some(i);
            }
        });
    }
    if let Some(i) = delete {
        params.masks.remove(i);
        *selected = None;
        action = MaskAction::Changed;
    }

    ui.add_space(8.0);
    ui.separator();

    // Selected mask editor.
    if let Some(idx) = *selected {
        if idx < params.masks.len() {
            if selected_editor(ui, &mut params.masks[idx], brush) {
                action = MaskAction::Changed;
            }
        }
    } else {
        ui.label(
            egui::RichText::new("Select or add a mask, then drag its shape in the preview.")
                .small()
                .weak(),
        );
    }

    ui.add_space(10.0);
    ui.separator();
    if ui.button(egui::RichText::new("✔ Done").strong()).clicked() {
        action = MaskAction::Done;
    }

    action
}

fn add_mask(
    params: &mut EditParams,
    selected: &mut Option<usize>,
    kind: MaskKind,
    name: &str,
) {
    let n = params.masks.len() + 1;
    let mut adjust = LocalAdjust::default();
    adjust.exposure = 25.0; // a visible starting effect so the mask does something
    params.masks.push(Mask {
        name: format!("{name} {n}"),
        kind,
        adjust,
        enabled: true,
        inverted: false,
    });
    *selected = Some(params.masks.len() - 1);
}

fn selected_editor(ui: &mut egui::Ui, mask: &mut Mask, brush: &mut BrushSettings) -> bool {
    let mut changed = false;

    ui.horizontal(|ui| {
        ui.label("Name:");
        if ui.text_edit_singleline(&mut mask.name).changed() {
            // Renaming isn't a render change, but harmless to mark.
        }
        if ui.checkbox(&mut mask.inverted, "Invert").changed() {
            changed = true;
        }
    });

    // Brush tools when editing a brush mask.
    if let MaskKind::Brush { dabs } = &mut mask.kind {
        ui.add_space(4.0);
        ui.label(egui::RichText::new("Brush").strong());
        egui::Grid::new("brush_grid").num_columns(2).show(ui, |ui| {
            ui.label("Size");
            ui.add(egui::Slider::new(&mut brush.radius, 0.01..=0.4).fixed_decimals(2));
            ui.end_row();
            ui.label("Softness");
            ui.add(egui::Slider::new(&mut brush.hardness, 0.0..=0.99).fixed_decimals(2));
            ui.end_row();
        });
        ui.horizontal(|ui| {
            ui.selectable_value(&mut brush.erase, false, "Paint");
            ui.selectable_value(&mut brush.erase, true, "Erase");
            if ui.button("Clear strokes").clicked() {
                dabs.clear();
                changed = true;
            }
        });
        ui.label(
            egui::RichText::new("Paint over the image in the preview to build the mask.")
                .small()
                .weak(),
        );
    } else if let MaskKind::Radial { feather, .. } = &mut mask.kind {
        ui.horizontal(|ui| {
            ui.label("Feather");
            if ui
                .add(egui::Slider::new(feather, 0.0..=1.0).fixed_decimals(2))
                .changed()
            {
                changed = true;
            }
        });
    }

    ui.add_space(6.0);
    ui.label(egui::RichText::new("Local adjustments").strong());
    let a = &mut mask.adjust;
    egui::Grid::new("local_adjust_grid")
        .num_columns(2)
        .spacing([6.0, 5.0])
        .show(ui, |ui| {
            changed |= row(ui, "Exposure", &mut a.exposure);
            changed |= row(ui, "Contrast", &mut a.contrast);
            changed |= row(ui, "Highlights", &mut a.highlights);
            changed |= row(ui, "Shadows", &mut a.shadows);
            changed |= row(ui, "Whites", &mut a.whites);
            changed |= row(ui, "Blacks", &mut a.blacks);
            changed |= row(ui, "Temp", &mut a.temp);
            changed |= row(ui, "Tint", &mut a.tint);
            changed |= row(ui, "Saturation", &mut a.saturation);
            changed |= row(ui, "Clarity", &mut a.clarity);
            changed |= row(ui, "Sharpness", &mut a.sharpness);
        });
    changed
}

fn row(ui: &mut egui::Ui, label: &str, value: &mut f32) -> bool {
    ui.label(label);
    let resp = ui.add(egui::Slider::new(value, -100.0..=100.0).fixed_decimals(0));
    let mut changed = resp.changed();
    if resp.double_clicked() {
        *value = 0.0;
        changed = true;
    }
    ui.end_row();
    changed
}
