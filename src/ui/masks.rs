//! Local adjustments (masking) panel. Manages the list of masks, the shapes
//! composed inside the selected mask, its range refinement, and the local
//! adjustments it applies. Shape geometry is drawn and dragged in the preview
//! (see preview.rs).

use crate::engine::params::{
    EditParams, LocalAdjust, Mask, MaskComponent, MaskKind, MaskOp, RangeMask,
};

/// Brush tool settings, shared with the preview while painting.
#[derive(Clone, Copy)]
pub struct BrushSettings {
    pub radius: f32,
    pub hardness: f32,
    pub erase: bool,
    /// Restrict new strokes to pixels matching the color under the cursor.
    pub auto_mask: bool,
}

impl Default for BrushSettings {
    fn default() -> Self {
        Self {
            radius: 0.08,
            hardness: 0.5,
            erase: false,
            auto_mask: false,
        }
    }
}

pub enum MaskAction {
    None,
    Changed,
    /// Only the view changed (the coverage overlay), not the edit itself —
    /// re-render, but don't touch the sidecar or the undo history.
    ViewChanged,
    /// The user asked to sample a target color for the range mask.
    PickColor,
    Done,
}

pub fn panel(
    ui: &mut egui::Ui,
    params: &mut EditParams,
    selected: &mut Option<usize>,
    sel_comp: &mut usize,
    brush: &mut BrushSettings,
    show_overlay: &mut bool,
    picking_color: bool,
) -> MaskAction {
    let mut action = MaskAction::None;

    ui.heading("Masks");
    ui.add_space(4.0);
    if ui
        .checkbox(show_overlay, "Show mask coverage")
        .on_hover_text("Wash the selected mask's selection in red over the photo")
        .changed()
    {
        action = MaskAction::ViewChanged;
    }
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        for (label, kind) in new_shape_buttons() {
            if ui.button(format!("+ {label}")).clicked() {
                add_mask(params, selected, sel_comp, kind, label);
                action = MaskAction::Changed;
            }
        }
    });
    if ui
        .button("+ Range only")
        .on_hover_text("A mask with no shape, selected purely by color or brightness")
        .clicked()
    {
        add_range_mask(params, selected, sel_comp);
        action = MaskAction::Changed;
    }

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
            let label = format!("{} — {}", m.name, m.summary());
            if ui.selectable_label(is_sel, label).clicked() && !is_sel {
                *selected = Some(i);
                *sel_comp = 0;
                // The coverage wash follows the selection, so it has to be
                // re-rendered for the mask now being edited.
                action = MaskAction::ViewChanged;
            }
            if ui.small_button("🗑").on_hover_text("Delete mask").clicked() {
                delete = Some(i);
            }
        });
    }
    if let Some(i) = delete {
        params.masks.remove(i);
        *selected = None;
        *sel_comp = 0;
        action = MaskAction::Changed;
    }

    ui.add_space(8.0);
    ui.separator();

    // Selected mask editor.
    if let Some(idx) = *selected {
        if idx < params.masks.len() {
            match selected_editor(ui, &mut params.masks[idx], sel_comp, brush, picking_color) {
                MaskAction::None => {}
                other => action = other,
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

/// The shapes offered when adding a mask or a component, each with the
/// geometry it starts out with.
fn new_shape_buttons() -> [(&'static str, MaskKind); 3] {
    [
        (
            "Linear",
            MaskKind::Linear {
                p0: [0.5, 0.8],
                p1: [0.5, 0.2],
            },
        ),
        ("Radial", MaskKind::default_radial()),
        ("Brush", MaskKind::Brush { dabs: Vec::new() }),
    ]
}

/// A new mask starts with a visible effect so it does something immediately.
fn starting_adjust() -> LocalAdjust {
    LocalAdjust {
        exposure: 25.0,
        ..LocalAdjust::default()
    }
}

fn add_mask(
    params: &mut EditParams,
    selected: &mut Option<usize>,
    sel_comp: &mut usize,
    kind: MaskKind,
    name: &str,
) {
    let n = params.masks.len() + 1;
    params.masks.push(Mask {
        name: format!("{name} {n}"),
        components: vec![MaskComponent::new(kind, MaskOp::Add)],
        adjust: starting_adjust(),
        ..Mask::default()
    });
    *selected = Some(params.masks.len() - 1);
    *sel_comp = 0;
}

fn add_range_mask(params: &mut EditParams, selected: &mut Option<usize>, sel_comp: &mut usize) {
    let n = params.masks.len() + 1;
    let mut mask = Mask {
        name: format!("Range {n}"),
        components: Vec::new(),
        adjust: starting_adjust(),
        ..Mask::default()
    };
    mask.range.color_enabled = true;
    params.masks.push(mask);
    *selected = Some(params.masks.len() - 1);
    *sel_comp = 0;
}

fn selected_editor(
    ui: &mut egui::Ui,
    mask: &mut Mask,
    sel_comp: &mut usize,
    brush: &mut BrushSettings,
    picking_color: bool,
) -> MaskAction {
    let mut action = MaskAction::None;
    let mut changed = false;

    ui.horizontal(|ui| {
        ui.label("Name:");
        ui.text_edit_singleline(&mut mask.name);
        if ui
            .checkbox(&mut mask.inverted, "Invert")
            .on_hover_text("Flip the whole composed mask, shapes and all")
            .changed()
        {
            changed = true;
        }
    });

    changed |= shape_list(ui, mask, sel_comp);
    *sel_comp = (*sel_comp).min(mask.components.len().saturating_sub(1));
    if let Some(c) = mask.components.get_mut(*sel_comp) {
        changed |= shape_controls(ui, &mut c.kind, brush);
    }

    ui.add_space(6.0);
    egui::CollapsingHeader::new("Range")
        .default_open(mask.range.is_active())
        .show(ui, |ui| {
            let (r_changed, pick) = range_controls(ui, &mut mask.range, picking_color);
            changed |= r_changed;
            if pick {
                action = MaskAction::PickColor;
            }
        });

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
            changed |= row(ui, "Vibrance", &mut a.vibrance);
            changed |= row(ui, "Saturation", &mut a.saturation);
            changed |= row(ui, "Texture", &mut a.texture);
            changed |= row(ui, "Clarity", &mut a.clarity);
            changed |= row(ui, "Dehaze", &mut a.dehaze);
            changed |= row(ui, "Sharpness", &mut a.sharpness);
            changed |= positive_row(ui, "Noise", &mut a.noise);
        });

    if changed {
        if let MaskAction::None = action {
            action = MaskAction::Changed;
        }
    }
    action
}

/// The list of shapes composed into this mask, with each one's operator.
fn shape_list(ui: &mut egui::Ui, mask: &mut Mask, sel_comp: &mut usize) -> bool {
    let mut changed = false;

    ui.add_space(6.0);
    ui.label(egui::RichText::new("Shapes").strong());
    if mask.components.is_empty() {
        ui.label(
            egui::RichText::new("No shapes — the whole photo, narrowed by Range below.")
                .small()
                .weak(),
        );
    }

    let mut delete: Option<usize> = None;
    for i in 0..mask.components.len() {
        ui.horizontal(|ui| {
            // The first shape has nothing above it to combine with, so its
            // operator is meaningless.
            let first = i == 0;
            ui.add_enabled_ui(!first, |ui| {
                let op = &mut mask.components[i].op;
                egui::ComboBox::from_id_salt(("mask_op", i))
                    .width(84.0)
                    .selected_text(if first { "Base" } else { op.label() })
                    .show_ui(ui, |ui| {
                        for choice in [MaskOp::Add, MaskOp::Subtract, MaskOp::Intersect] {
                            if ui.selectable_value(op, choice, choice.label()).changed() {
                                changed = true;
                            }
                        }
                    });
            });
            let c = &mut mask.components[i];
            let is_sel = *sel_comp == i;
            if ui
                .selectable_label(is_sel, c.kind.type_name())
                .on_hover_text("Edit this shape in the preview")
                .clicked()
            {
                *sel_comp = i;
            }
            if ui
                .checkbox(&mut c.inverted, "Inv")
                .on_hover_text("Invert just this shape")
                .changed()
            {
                changed = true;
            }
            if ui.small_button("🗑").on_hover_text("Remove shape").clicked() {
                delete = Some(i);
            }
        });
    }
    if let Some(i) = delete {
        mask.components.remove(i);
        *sel_comp = (*sel_comp).min(mask.components.len().saturating_sub(1));
        changed = true;
    }

    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Add:").small().weak());
        for (label, kind) in new_shape_buttons() {
            if ui.small_button(label).clicked() {
                // New shapes add to the selection; switch the row's dropdown
                // to Subtract or Intersect to carve instead.
                mask.components.push(MaskComponent::new(kind, MaskOp::Add));
                *sel_comp = mask.components.len() - 1;
                changed = true;
            }
        }
    });

    changed
}

/// Controls belonging to the selected shape: feather for a radial, the brush
/// tools for a brush. Linear gradients are aimed in the preview only.
fn shape_controls(ui: &mut egui::Ui, kind: &mut MaskKind, brush: &mut BrushSettings) -> bool {
    let mut changed = false;
    match kind {
        MaskKind::Brush { dabs } => {
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
            ui.checkbox(&mut brush.auto_mask, "Auto mask").on_hover_text(
                "Each dab remembers the color under the cursor and only paints \
                 similar pixels, so a stroke stops at edges",
            );
            ui.label(
                egui::RichText::new("Paint over the image in the preview to build the mask.")
                    .small()
                    .weak(),
            );
        }
        MaskKind::Radial { feather, .. } => {
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
        MaskKind::Linear { .. } => {
            ui.label(
                egui::RichText::new("Drag the line's ends in the preview to aim the gradient.")
                    .small()
                    .weak(),
            );
        }
    }
    changed
}

/// Luminance and color range controls. Returns (changed, pick-color clicked).
fn range_controls(ui: &mut egui::Ui, r: &mut RangeMask, picking: bool) -> (bool, bool) {
    let mut changed = false;
    let mut pick = false;

    ui.label(
        egui::RichText::new("Narrow the mask to pixels matching a brightness or a color.")
            .small()
            .weak(),
    );

    changed |= ui.checkbox(&mut r.lum_enabled, "Luminance range").changed();
    if r.lum_enabled {
        egui::Grid::new("range_lum_grid")
            .num_columns(2)
            .spacing([6.0, 5.0])
            .show(ui, |ui| {
                ui.label("From");
                changed |= ui
                    .add(egui::Slider::new(&mut r.lum_lo, 0.0..=1.0).fixed_decimals(2))
                    .changed();
                ui.end_row();
                ui.label("To");
                changed |= ui
                    .add(egui::Slider::new(&mut r.lum_hi, 0.0..=1.0).fixed_decimals(2))
                    .changed();
                ui.end_row();
                ui.label("Feather");
                changed |= ui
                    .add(egui::Slider::new(&mut r.lum_feather, 0.01..=0.5).fixed_decimals(2))
                    .changed();
                ui.end_row();
            });
        // Keep the band ordered however the user drags its two ends.
        if r.lum_lo > r.lum_hi {
            std::mem::swap(&mut r.lum_lo, &mut r.lum_hi);
        }
    }

    ui.add_space(4.0);
    changed |= ui.checkbox(&mut r.color_enabled, "Color range").changed();
    if r.color_enabled {
        ui.horizontal(|ui| {
            if ui
                .add(egui::SelectableLabel::new(picking, "💧 Pick color"))
                .on_hover_text("Click a color in the photo to target it")
                .clicked()
            {
                pick = true;
            }
            let (rr, gg, bb) = crate::engine::ops::color::hsl_to_rgb(r.hue, 0.9, 0.5);
            let swatch =
                egui::Color32::from_rgb((rr * 255.0) as u8, (gg * 255.0) as u8, (bb * 255.0) as u8);
            let (rect, _) = ui.allocate_exact_size(egui::vec2(22.0, 16.0), egui::Sense::hover());
            ui.painter().rect_filled(rect, 2.0, swatch);
        });
        egui::Grid::new("range_color_grid")
            .num_columns(2)
            .spacing([6.0, 5.0])
            .show(ui, |ui| {
                ui.label("Hue");
                changed |= ui
                    .add(egui::Slider::new(&mut r.hue, 0.0..=360.0).fixed_decimals(0))
                    .changed();
                ui.end_row();
                ui.label("Width");
                changed |= ui
                    .add(egui::Slider::new(&mut r.hue_width, 1.0..=180.0).fixed_decimals(0))
                    .changed();
                ui.end_row();
                ui.label("Feather");
                changed |= ui
                    .add(egui::Slider::new(&mut r.hue_feather, 1.0..=90.0).fixed_decimals(0))
                    .changed();
                ui.end_row();
                ui.label("Min sat");
                changed |= ui
                    .add(egui::Slider::new(&mut r.sat_min, 0.0..=1.0).fixed_decimals(2))
                    .changed();
                ui.end_row();
            });
    }

    (changed, pick)
}

fn row(ui: &mut egui::Ui, label: &str, value: &mut f32) -> bool {
    slider_row(ui, label, value, -100.0..=100.0)
}

/// A slider that only goes one way — noise reduction has no negative side.
fn positive_row(ui: &mut egui::Ui, label: &str, value: &mut f32) -> bool {
    slider_row(ui, label, value, 0.0..=100.0)
}

fn slider_row(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
) -> bool {
    ui.label(label);
    let resp = ui.add(egui::Slider::new(value, range).fixed_decimals(0));
    let mut changed = resp.changed();
    if resp.double_clicked() {
        *value = 0.0;
        changed = true;
    }
    ui.end_row();
    changed
}

/// Point a mask's color range at a sampled pixel: target its hue, and relax
/// the saturation gate if the sample is too muted to pass it.
pub fn aim_color_range(r: &mut RangeMask, hue: f32, sat: f32) {
    r.color_enabled = true;
    r.hue = hue.rem_euclid(360.0);
    if sat < r.sat_min + 0.08 {
        r.sat_min = (sat - 0.08).max(0.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lay the panel out headlessly and hand back the state it left behind.
    /// Catches the index bookkeeping going wrong (a selected shape that no
    /// longer exists, say) without needing a window.
    fn lay_out(params: &mut EditParams, selected: &mut Option<usize>, sel_comp: &mut usize) {
        let ctx = egui::Context::default();
        let mut brush = BrushSettings::default();
        let mut show_overlay = true;
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                panel(
                    ui,
                    params,
                    selected,
                    sel_comp,
                    &mut brush,
                    &mut show_overlay,
                    false,
                );
            });
        });
    }

    #[test]
    fn panel_lays_out_with_no_masks() {
        let mut params = EditParams::default();
        let (mut selected, mut sel_comp) = (None, 0);
        lay_out(&mut params, &mut selected, &mut sel_comp);
        assert!(params.masks.is_empty());
    }

    #[test]
    fn panel_clamps_a_stale_shape_selection() {
        // A selected-shape index left over from a mask with more shapes must
        // not index off the end of the one now selected.
        let mut params = EditParams::default();
        params.masks.push(Mask::default()); // one component
        let (mut selected, mut sel_comp) = (Some(0), 7);
        lay_out(&mut params, &mut selected, &mut sel_comp);
        assert_eq!(sel_comp, 0);
    }

    #[test]
    fn panel_handles_a_mask_with_no_shapes() {
        let mut params = EditParams::default();
        let mut m = Mask::default();
        m.components.clear();
        m.range.color_enabled = true;
        params.masks.push(m);
        let (mut selected, mut sel_comp) = (Some(0), 0);
        lay_out(&mut params, &mut selected, &mut sel_comp);
        assert_eq!(sel_comp, 0);
        assert!(params.masks[0].components.is_empty());
    }

    #[test]
    fn panel_lays_out_every_shape_and_range_combination() {
        let mut params = EditParams::default();
        let mut m = Mask::default();
        m.components.push(MaskComponent::new(
            MaskKind::Brush { dabs: Vec::new() },
            MaskOp::Subtract,
        ));
        m.components.push(MaskComponent::new(
            MaskKind::Linear {
                p0: [0.0, 0.0],
                p1: [1.0, 1.0],
            },
            MaskOp::Intersect,
        ));
        m.range.lum_enabled = true;
        m.range.color_enabled = true;
        params.masks.push(m);
        // Select each shape in turn; every branch of the editor must lay out.
        for i in 0..3 {
            let (mut selected, mut sel_comp) = (Some(0), i);
            lay_out(&mut params, &mut selected, &mut sel_comp);
            assert_eq!(sel_comp, i);
        }
    }

    #[test]
    fn out_of_order_luminance_band_is_reordered() {
        let mut params = EditParams::default();
        let mut m = Mask::default();
        m.range.lum_enabled = true;
        m.range.lum_lo = 0.8;
        m.range.lum_hi = 0.2;
        params.masks.push(m);
        let (mut selected, mut sel_comp) = (Some(0), 0);
        lay_out(&mut params, &mut selected, &mut sel_comp);
        let r = params.masks[0].range;
        assert!(r.lum_lo <= r.lum_hi, "{} .. {}", r.lum_lo, r.lum_hi);
    }

    #[test]
    fn aiming_at_a_vivid_color_keeps_the_saturation_gate() {
        let mut r = RangeMask::default();
        let before = r.sat_min;
        aim_color_range(&mut r, 210.0, 0.8);
        assert!(r.color_enabled);
        assert_eq!(r.hue, 210.0);
        assert_eq!(r.sat_min, before);
    }

    #[test]
    fn aiming_at_a_muted_color_relaxes_the_gate() {
        let mut r = RangeMask::default(); // sat_min 0.10
        aim_color_range(&mut r, 30.0, 0.05);
        assert!(r.sat_min < 0.05, "gate {} must admit the sample", r.sat_min);
        assert!(r.sat_min >= 0.0);
    }

    #[test]
    fn hue_wraps_into_range() {
        let mut r = RangeMask::default();
        aim_color_range(&mut r, -30.0, 0.5);
        assert!((r.hue - 330.0).abs() < 1e-4);
    }
}
