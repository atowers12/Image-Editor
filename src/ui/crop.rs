//! Crop tool side panel: orientation buttons, straighten slider, aspect
//! lock, the constrain-to-image toggle, reset. The interactive crop
//! rectangle itself lives in the preview (see preview.rs); this panel edits
//! the rest of the geometry.

use crate::engine::ops::geometry;
use crate::engine::params::EditParams;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum AspectLock {
    #[default]
    Free,
    Original,
    Square,
    R3x2,
    R2x3,
    R4x3,
    R3x4,
    R16x9,
    R9x16,
}

impl AspectLock {
    pub const ALL: [AspectLock; 9] = [
        AspectLock::Free,
        AspectLock::Original,
        AspectLock::Square,
        AspectLock::R3x2,
        AspectLock::R2x3,
        AspectLock::R4x3,
        AspectLock::R3x4,
        AspectLock::R16x9,
        AspectLock::R9x16,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            AspectLock::Free => "Free",
            AspectLock::Original => "Original",
            AspectLock::Square => "1 : 1",
            AspectLock::R3x2 => "3 : 2",
            AspectLock::R2x3 => "2 : 3",
            AspectLock::R4x3 => "4 : 3",
            AspectLock::R3x4 => "3 : 4",
            AspectLock::R16x9 => "16 : 9",
            AspectLock::R9x16 => "9 : 16",
        }
    }

    /// Width/height ratio in *pixels*, given the oriented uncropped dims.
    pub fn ratio(&self, dims: (usize, usize)) -> Option<f32> {
        match self {
            AspectLock::Free => None,
            AspectLock::Original => Some(dims.0 as f32 / dims.1.max(1) as f32),
            AspectLock::Square => Some(1.0),
            AspectLock::R3x2 => Some(3.0 / 2.0),
            AspectLock::R2x3 => Some(2.0 / 3.0),
            AspectLock::R4x3 => Some(4.0 / 3.0),
            AspectLock::R3x4 => Some(3.0 / 4.0),
            AspectLock::R16x9 => Some(16.0 / 9.0),
            AspectLock::R9x16 => Some(9.0 / 16.0),
        }
    }
}

pub enum CropAction {
    None,
    /// Geometry changed — preview must re-render.
    Changed,
    /// User is done cropping; leave crop mode.
    Done,
}

/// Conform a crop rect to an aspect ratio (in pixel terms), shrinking one
/// dimension around its center.
pub fn conform_to_aspect(crop: &mut [f32; 4], ratio: f32, dims: (usize, usize)) {
    let (iw, ih) = (dims.0 as f32, dims.1.max(1) as f32);
    let w_px = crop[2] * iw;
    let h_px = crop[3] * ih;
    if w_px <= 0.0 || h_px <= 0.0 {
        return;
    }
    let current = w_px / h_px;
    if (current - ratio).abs() < 1e-3 {
        return;
    }
    if current > ratio {
        // Too wide: shrink width around center.
        let new_w = (h_px * ratio) / iw;
        crop[0] += (crop[2] - new_w) * 0.5;
        crop[2] = new_w;
    } else {
        let new_h = (w_px / ratio) / ih;
        crop[1] += (crop[3] - new_h) * 0.5;
        crop[3] = new_h;
    }
    clamp_crop(crop);
}

/// Smallest crop, as a fraction of each side. Shared with the preview's
/// drag handling so both paths agree on what "too small" means.
pub const MIN_CROP: f32 = 0.03;

pub fn clamp_crop(crop: &mut [f32; 4]) {
    crop[2] = crop[2].clamp(MIN_CROP, 1.0);
    crop[3] = crop[3].clamp(MIN_CROP, 1.0);
    crop[0] = crop[0].clamp(0.0, 1.0 - crop[2]);
    crop[1] = crop[1].clamp(0.0, 1.0 - crop[3]);
}

/// Pull the crop back inside the photo after something moved the frame it
/// sits in (a new angle, a new aspect, the toggle being switched on).
/// A no-op when the crop is free to roam, or before the photo's real
/// dimensions are known. Returns whether it moved.
fn refit(params: &mut EditParams, constrain: bool, source: Option<(usize, usize)>) -> bool {
    match source {
        Some(src) if constrain => geometry::fit_crop_in_frame(&mut params.crop, params.angle, src),
        _ => false,
    }
}

/// The crop tool's controls (shown in the right panel while cropping).
/// `oriented_dims`: uncropped straightened-canvas dims, for aspect math.
/// `source_dims`: the frame the straighten angle rotates — the boundary the
/// constraint tests against. `None` until the photo's real size is known.
/// `constrain`: keep the crop rectangle inside the photo's real pixels.
pub fn panel(
    ui: &mut egui::Ui,
    params: &mut EditParams,
    aspect: &mut AspectLock,
    constrain: &mut bool,
    oriented_dims: (usize, usize),
    source_dims: Option<(usize, usize)>,
) -> CropAction {
    let mut action = CropAction::None;
    // A quarter turn swaps the frame the straighten rotates.
    let turned = source_dims.map(|(w, h)| (h, w));

    ui.heading("Crop & Rotate");
    ui.add_space(6.0);

    ui.horizontal(|ui| {
        if ui
            .button("⟲ Rotate L")
            .on_hover_text("Rotate 90° counter-clockwise")
            .clicked()
        {
            params.rotate90 = (params.rotate90 + 3) % 4;
            params.crop = geometry::crop_rotated_ccw(params.crop);
            refit(params, *constrain, turned);
            action = CropAction::Changed;
        }
        if ui
            .button("⟳ Rotate R")
            .on_hover_text("Rotate 90° clockwise")
            .clicked()
        {
            params.rotate90 = (params.rotate90 + 1) % 4;
            params.crop = geometry::crop_rotated_cw(params.crop);
            refit(params, *constrain, turned);
            action = CropAction::Changed;
        }
    });
    ui.horizontal(|ui| {
        if ui.button("⬌ Flip H").clicked() {
            params.flip_h = !params.flip_h;
            params.crop = geometry::crop_flipped(params.crop, true);
            refit(params, *constrain, source_dims);
            action = CropAction::Changed;
        }
        if ui.button("⬍ Flip V").clicked() {
            params.flip_v = !params.flip_v;
            params.crop = geometry::crop_flipped(params.crop, false);
            refit(params, *constrain, source_dims);
            action = CropAction::Changed;
        }
    });

    ui.add_space(8.0);
    ui.label("Straighten");
    let resp = ui.add(
        egui::Slider::new(&mut params.angle, -45.0..=45.0)
            .fixed_decimals(1)
            .suffix("°"),
    );
    if resp.double_clicked() {
        params.angle = 0.0;
        action = CropAction::Changed;
    } else if resp.changed() {
        action = CropAction::Changed;
    }
    // A new angle re-shapes the canvas and its black corner wedges, which can
    // leave the crop sitting over them.
    if matches!(action, CropAction::Changed) {
        refit(params, *constrain, source_dims);
    }

    ui.add_space(8.0);
    let mut aspect_changed = false;
    egui::ComboBox::from_label("Aspect")
        .selected_text(aspect.label())
        .show_ui(ui, |ui| {
            for a in AspectLock::ALL {
                if ui.selectable_value(aspect, a, a.label()).changed() {
                    aspect_changed = true;
                }
            }
        });
    if aspect_changed {
        if let Some(ratio) = aspect.ratio(oriented_dims) {
            conform_to_aspect(&mut params.crop, ratio, oriented_dims);
            refit(params, *constrain, source_dims);
            action = CropAction::Changed;
        }
    }

    ui.add_space(8.0);
    if let Some(src) = source_dims {
        if ui
            .button("⛶ Fill frame")
            .on_hover_text(
                "Grow the crop to the largest one of this shape that fits the \
                 straightened photo, centred on it",
            )
            .clicked()
        {
            geometry::fill_frame(&mut params.crop, params.angle, src);
            action = CropAction::Changed;
        }
    }

    ui.add_space(6.0);
    let constrain_resp = ui.checkbox(constrain, "Constrain to image").on_hover_text(
        "Keep the crop frame on the photo's real pixels, so a straightened \
         image never leaves black corners. The limit is drawn in the preview. \
         Untick to place the frame anywhere on the canvas.",
    );
    if constrain_resp.changed() && refit(params, *constrain, source_dims) {
        action = CropAction::Changed;
    }

    ui.add_space(12.0);
    ui.separator();
    ui.horizontal(|ui| {
        if ui.button(egui::RichText::new("✔ Done").strong()).clicked() {
            action = CropAction::Done;
        }
        if ui.button("Reset").clicked() {
            params.reset_geometry();
            *aspect = AspectLock::Free;
            action = CropAction::Changed;
        }
    });
    ui.add_space(6.0);
    ui.label(
        egui::RichText::new(
            "Drag the corners or edges of the frame in the preview; drag inside to move it. \
             Drag outside the frame — or middle-drag anywhere — to pan, scroll to zoom, \
             double-click outside to fit.",
        )
        .small()
        .weak(),
    );

    action
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conform_shrinks_to_ratio() {
        // 200x100 image, full crop, want 1:1 → width shrinks to 100px (0.5 norm).
        let mut crop = [0.0, 0.0, 1.0, 1.0];
        conform_to_aspect(&mut crop, 1.0, (200, 100));
        assert!((crop[2] - 0.5).abs() < 1e-4);
        assert!((crop[3] - 1.0).abs() < 1e-4);
        assert!((crop[0] - 0.25).abs() < 1e-4);
    }

    #[test]
    fn clamp_keeps_rect_inside() {
        let mut crop = [0.9, -0.1, 0.5, 0.5];
        clamp_crop(&mut crop);
        assert!(crop[0] + crop[2] <= 1.0 + 1e-6);
        assert!(crop[1] >= 0.0);
    }

    #[test]
    fn refit_only_acts_when_constrained_and_sized() {
        let dims = (4000, 3000);
        let mut params = EditParams {
            angle: -28.0,
            crop: [0.0, 0.0, 1.0, 1.0],
            ..EditParams::default()
        };
        // Unconstrained the crop is left over the black corners.
        assert!(!refit(&mut params, false, Some(dims)));
        assert_eq!(params.crop, [0.0, 0.0, 1.0, 1.0]);
        // Nor is anything guessed before the real dimensions arrive.
        assert!(!refit(&mut params, true, None));
        assert_eq!(params.crop, [0.0, 0.0, 1.0, 1.0]);
        // Constrained, with dimensions, it is pulled inside.
        assert!(refit(&mut params, true, Some(dims)));
        assert!(geometry::rect_in_frame(params.crop, -28.0, dims));
    }
}
