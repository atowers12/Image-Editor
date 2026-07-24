//! The central photo view: fit-to-window by default, mouse-wheel zoom
//! around the cursor, drag to pan, double-click to toggle fit/100%.
//!
//! Two extras layered on top:
//! - Region-of-interest sharpening: when zoomed past the preview's native
//!   resolution, we ask the app to render the visible area from the
//!   full-res source and draw that texture over the soft preview.
//! - Crop overlay: while the crop tool is open, an interactive rect with
//!   corner/edge handles edits `EditParams::crop` in place.

pub struct PreviewState {
    pub fit: bool,
    pub zoom: f32,
    pub offset: egui::Vec2,
    crop_drag: Option<Handle>,
}

impl Default for PreviewState {
    fn default() -> Self {
        Self {
            fit: true,
            zoom: 1.0,
            offset: egui::Vec2::ZERO,
            crop_drag: None,
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Handle {
    N,
    S,
    E,
    W,
    NE,
    NW,
    SE,
    SW,
    Move,
}

/// Ask the app to render this part of the full-res image.
#[derive(Clone, Copy, PartialEq)]
pub struct RegionRequest {
    pub norm_rect: [f32; 4],
    pub target: (usize, usize),
}

impl RegionRequest {
    /// Close enough that re-rendering wouldn't change anything visible.
    pub fn roughly_eq(&self, other: &RegionRequest) -> bool {
        self.norm_rect
            .iter()
            .zip(other.norm_rect.iter())
            .all(|(a, b)| (a - b).abs() < 0.002)
            && self.target.0.abs_diff(other.target.0) < 32
            && self.target.1.abs_diff(other.target.1) < 32
    }
}

pub struct CropOverlay<'a> {
    pub crop: &'a mut [f32; 4],
    /// Locked pixel aspect ratio (w/h), if any.
    pub aspect: Option<f32>,
    /// Oriented, uncropped full-image dimensions.
    pub dims: (usize, usize),
}

/// Editing handle for the selected local-adjustment mask, drawn over the
/// preview. Radial/linear masks expose drag handles; brush masks paint.
pub struct MaskEditor<'a> {
    pub kind: &'a mut crate::engine::params::MaskKind,
    pub brush: crate::ui::masks::BrushSettings,
}

#[derive(Default)]
pub struct PreviewOutput {
    pub region_request: Option<RegionRequest>,
    pub crop_changed: bool,
    pub mask_changed: bool,
    /// Normalized image point the eyedropper sampled this frame.
    pub eyedrop_point: Option<[f32; 2]>,
}

const MIN_CROP: f32 = 0.03;

pub fn show(
    ui: &mut egui::Ui,
    state: &mut PreviewState,
    tex: Option<&egui::TextureHandle>,
    loading: bool,
    full_dims: Option<(usize, usize)>,
    region: Option<(&egui::TextureHandle, [f32; 4])>,
    crop: Option<CropOverlay>,
    mask_edit: Option<MaskEditor>,
    eyedropper: bool,
) -> PreviewOutput {
    let mut out = PreviewOutput::default();
    let rect = ui.available_rect_before_wrap();
    let resp = ui.allocate_rect(rect, egui::Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, egui::Color32::from_gray(16));

    let Some(tex) = tex else {
        if !loading {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Open a folder or photo to start editing",
                egui::FontId::proportional(16.0),
                egui::Color32::from_gray(120),
            );
        }
        if loading {
            let spinner_rect = egui::Rect::from_center_size(rect.center(), egui::vec2(36.0, 36.0));
            ui.put(spinner_rect, egui::Spinner::new().size(36.0));
        }
        return out;
    };

    let cropping = crop.is_some();
    let masking = mask_edit.is_some();
    // Both overlay modes fit the whole image and disable pan/zoom-drag.
    let overlay_mode = cropping || masking;
    let img_size = tex.size_vec2();
    let margin = if cropping { 24.0 } else { 0.0 };
    let fit_scale = ((rect.width() - margin) / img_size.x)
        .min((rect.height() - margin) / img_size.y)
        .min(4.0);
    let mut scale = if state.fit || overlay_mode {
        fit_scale
    } else {
        state.zoom
    };

    if !overlay_mode {
        // Zoom: mouse wheel / pinch / ctrl+wheel, anchored at the cursor.
        if resp.hovered() {
            let (scroll, pinch) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
            let factor = pinch * (1.0 + scroll * 0.0015);
            if (factor - 1.0).abs() > 1e-4 {
                let new_scale = (scale * factor).clamp(0.02_f32.min(fit_scale), 8.0);
                if let Some(ptr) = resp.hover_pos() {
                    let center = rect.center() + state.offset;
                    let v = ptr - center;
                    state.offset += v - v * (new_scale / scale);
                }
                scale = new_scale;
                state.zoom = new_scale;
                state.fit = false;
            }
        }

        if resp.dragged() && !state.fit {
            state.offset += resp.drag_delta();
        }

        if resp.double_clicked() {
            if state.fit {
                state.fit = false;
                state.zoom = 1.0; // 100% of the *preview* texture
                if let Some(ptr) = resp.hover_pos() {
                    let frac = (ptr - (rect.center() + state.offset)) / fit_scale;
                    state.offset = -frac;
                }
            } else {
                state.fit = true;
            }
        }

        if state.fit {
            state.offset = egui::Vec2::ZERO;
            scale = fit_scale;
            state.zoom = fit_scale;
        }
    } else {
        state.offset = egui::Vec2::ZERO;
        scale = fit_scale;
    }

    let size = img_size * scale;
    // Keep at least part of the image on screen.
    let max_x = (size.x + rect.width()) * 0.5 - 40.0;
    let max_y = (size.y + rect.height()) * 0.5 - 40.0;
    state.offset.x = state.offset.x.clamp(-max_x.max(0.0), max_x.max(0.0));
    state.offset.y = state.offset.y.clamp(-max_y.max(0.0), max_y.max(0.0));

    let img_rect = egui::Rect::from_center_size(rect.center() + state.offset, size);
    let uv_full = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    painter.image(tex.id(), img_rect, uv_full, egui::Color32::WHITE);

    // --- Region of interest: sharpen what's visible when zoomed in ---
    let displayed_soft = img_rect.width() > img_size.x * 1.05;
    if !overlay_mode && displayed_soft {
        // Draw the last rendered region (may lag slightly behind panning).
        if let Some((rtex, rrect)) = region {
            let sub = egui::Rect::from_min_size(
                img_rect.min
                    + egui::vec2(rrect[0] * img_rect.width(), rrect[1] * img_rect.height()),
                egui::vec2(rrect[2] * img_rect.width(), rrect[3] * img_rect.height()),
            );
            painter.image(rtex.id(), sub, uv_full, egui::Color32::WHITE);
        }
        // And request the currently-visible area.
        if let Some((fw, fh)) = full_dims {
            let visible = rect.intersect(img_rect);
            if visible.width() > 1.0 && visible.height() > 1.0 {
                // Pad so small pans don't immediately hit soft edges.
                let pad_x = visible.width() * 0.15;
                let pad_y = visible.height() * 0.15;
                let padded = visible.expand2(egui::vec2(pad_x, pad_y)).intersect(img_rect);
                let nx = (padded.min.x - img_rect.min.x) / img_rect.width();
                let ny = (padded.min.y - img_rect.min.y) / img_rect.height();
                let nw = padded.width() / img_rect.width();
                let nh = padded.height() / img_rect.height();
                // Target size in screen pixels; the worker caps it at 1:1
                // of the source so we never upsample in the pipeline.
                let target = (
                    padded.width().round() as usize,
                    padded.height().round() as usize,
                );
                let _ = (fw, fh);
                out.region_request = Some(RegionRequest {
                    norm_rect: [nx, ny, nw, nh],
                    target,
                });
            }
        }
    }

    // --- Crop overlay ---
    if let Some(c) = crop {
        out.crop_changed = crop_overlay(ui, state, &painter, &resp, img_rect, c);
    } else if let Some(m) = mask_edit {
        out.mask_changed = mask_overlay(ui, &painter, &resp, img_rect, m);
    } else {
        state.crop_drag = None;
        // Eyedropper: click samples a normalized image point.
        if eyedropper {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
            if resp.clicked() {
                if let Some(p) = resp.interact_pointer_pos() {
                    if img_rect.contains(p) {
                        out.eyedrop_point = Some([
                            ((p.x - img_rect.min.x) / img_rect.width()).clamp(0.0, 1.0),
                            ((p.y - img_rect.min.y) / img_rect.height()).clamp(0.0, 1.0),
                        ]);
                    }
                }
            }
        }
        // Zoom readout, bottom-left.
        let label = if state.fit {
            "Fit".to_string()
        } else if let Some((fw, _)) = full_dims {
            // Percent of the true full-resolution image.
            format!("{:.0}%", img_rect.width() / fw as f32 * 100.0)
        } else {
            format!("{:.0}%", scale * 100.0)
        };
        painter.text(
            rect.left_bottom() + egui::vec2(10.0, -10.0),
            egui::Align2::LEFT_BOTTOM,
            label,
            egui::FontId::proportional(12.0),
            egui::Color32::from_gray(160),
        );
    }

    if loading {
        let spinner_rect = egui::Rect::from_center_size(rect.center(), egui::vec2(36.0, 36.0));
        ui.put(spinner_rect, egui::Spinner::new().size(36.0));
    }

    out
}

/// Draw and edit the crop rectangle. Returns true if the crop changed.
fn crop_overlay(
    ui: &egui::Ui,
    state: &mut PreviewState,
    painter: &egui::Painter,
    resp: &egui::Response,
    img_rect: egui::Rect,
    c: CropOverlay,
) -> bool {
    let crop_rect = egui::Rect::from_min_size(
        img_rect.min + egui::vec2(c.crop[0] * img_rect.width(), c.crop[1] * img_rect.height()),
        egui::vec2(c.crop[2] * img_rect.width(), c.crop[3] * img_rect.height()),
    );

    // Darken everything outside the crop.
    let shade = egui::Color32::from_black_alpha(140);
    let full = img_rect;
    painter.rect_filled(
        egui::Rect::from_min_max(full.min, egui::pos2(full.max.x, crop_rect.min.y)),
        0.0,
        shade,
    );
    painter.rect_filled(
        egui::Rect::from_min_max(egui::pos2(full.min.x, crop_rect.max.y), full.max),
        0.0,
        shade,
    );
    painter.rect_filled(
        egui::Rect::from_min_max(
            egui::pos2(full.min.x, crop_rect.min.y),
            egui::pos2(crop_rect.min.x, crop_rect.max.y),
        ),
        0.0,
        shade,
    );
    painter.rect_filled(
        egui::Rect::from_min_max(
            egui::pos2(crop_rect.max.x, crop_rect.min.y),
            egui::pos2(full.max.x, crop_rect.max.y),
        ),
        0.0,
        shade,
    );

    // Border, rule-of-thirds grid, and handles.
    let stroke = egui::Stroke::new(1.5, egui::Color32::from_gray(230));
    painter.rect_stroke(crop_rect, 0.0, stroke, egui::StrokeKind::Middle);
    let thin = egui::Stroke::new(0.7, egui::Color32::from_white_alpha(90));
    for i in 1..3 {
        let fx = crop_rect.min.x + crop_rect.width() * i as f32 / 3.0;
        let fy = crop_rect.min.y + crop_rect.height() * i as f32 / 3.0;
        painter.line_segment(
            [egui::pos2(fx, crop_rect.min.y), egui::pos2(fx, crop_rect.max.y)],
            thin,
        );
        painter.line_segment(
            [egui::pos2(crop_rect.min.x, fy), egui::pos2(crop_rect.max.x, fy)],
            thin,
        );
    }
    for (hx, hy, _) in handle_points(&crop_rect) {
        painter.rect_filled(
            egui::Rect::from_center_size(egui::pos2(hx, hy), egui::vec2(8.0, 8.0)),
            1.0,
            egui::Color32::from_gray(240),
        );
    }

    // Cursor feedback.
    if let Some(ptr) = resp.hover_pos() {
        if let Some(h) = state.crop_drag.or_else(|| hit_handle(&crop_rect, ptr)) {
            ui.ctx().set_cursor_icon(match h {
                Handle::N | Handle::S => egui::CursorIcon::ResizeVertical,
                Handle::E | Handle::W => egui::CursorIcon::ResizeHorizontal,
                Handle::NE | Handle::SW => egui::CursorIcon::ResizeNeSw,
                Handle::NW | Handle::SE => egui::CursorIcon::ResizeNwSe,
                Handle::Move => egui::CursorIcon::Grab,
            });
        }
    }

    // Interaction.
    let mut changed = false;
    if resp.drag_started() {
        state.crop_drag = resp
            .interact_pointer_pos()
            .and_then(|p| hit_handle(&crop_rect, p));
    }
    if resp.dragged() {
        if let Some(handle) = state.crop_drag {
            let d = resp.drag_delta();
            let dn = egui::vec2(d.x / img_rect.width(), d.y / img_rect.height());
            if dn != egui::Vec2::ZERO {
                // Normalized aspect: w_norm / h_norm for a locked pixel ratio.
                let aspect_norm = c
                    .aspect
                    .map(|a| a * c.dims.1.max(1) as f32 / c.dims.0.max(1) as f32);
                apply_crop_drag(c.crop, handle, dn, aspect_norm);
                changed = true;
            }
        }
    }
    if resp.drag_stopped() {
        state.crop_drag = None;
    }
    changed
}

fn handle_points(r: &egui::Rect) -> [(f32, f32, Handle); 8] {
    let (cx, cy) = (r.center().x, r.center().y);
    [
        (r.min.x, r.min.y, Handle::NW),
        (cx, r.min.y, Handle::N),
        (r.max.x, r.min.y, Handle::NE),
        (r.max.x, cy, Handle::E),
        (r.max.x, r.max.y, Handle::SE),
        (cx, r.max.y, Handle::S),
        (r.min.x, r.max.y, Handle::SW),
        (r.min.x, cy, Handle::W),
    ]
}

fn hit_handle(crop_rect: &egui::Rect, p: egui::Pos2) -> Option<Handle> {
    const GRAB: f32 = 14.0;
    // Corners and edge midpoints first (they overlap the interior).
    let mut best: Option<(f32, Handle)> = None;
    for (hx, hy, h) in handle_points(crop_rect) {
        let d = p.distance(egui::pos2(hx, hy));
        if d < GRAB && best.map(|(bd, _)| d < bd).unwrap_or(true) {
            best = Some((d, h));
        }
    }
    if let Some((_, h)) = best {
        return Some(h);
    }
    // Edges (anywhere along them, not just midpoints).
    let near = |a: f32, b: f32| (a - b).abs() < 8.0;
    let inside_x = p.x > crop_rect.min.x - 8.0 && p.x < crop_rect.max.x + 8.0;
    let inside_y = p.y > crop_rect.min.y - 8.0 && p.y < crop_rect.max.y + 8.0;
    if inside_x && near(p.y, crop_rect.min.y) {
        return Some(Handle::N);
    }
    if inside_x && near(p.y, crop_rect.max.y) {
        return Some(Handle::S);
    }
    if inside_y && near(p.x, crop_rect.min.x) {
        return Some(Handle::W);
    }
    if inside_y && near(p.x, crop_rect.max.x) {
        return Some(Handle::E);
    }
    if crop_rect.contains(p) {
        return Some(Handle::Move);
    }
    None
}

/// Mutate the normalized crop rect for a drag of `dn` on `handle`,
/// optionally keeping a locked (normalized) aspect ratio.
fn apply_crop_drag(crop: &mut [f32; 4], handle: Handle, dn: egui::Vec2, aspect: Option<f32>) {
    let [x, y, w, h] = *crop;
    let right = x + w;
    let bottom = y + h;
    let (mut nx, mut ny, mut nw, mut nh) = (x, y, w, h);

    match handle {
        Handle::Move => {
            nx = (x + dn.x).clamp(0.0, 1.0 - w);
            ny = (y + dn.y).clamp(0.0, 1.0 - h);
        }
        Handle::E => nw = (w + dn.x).max(MIN_CROP),
        Handle::W => {
            nx = (x + dn.x).min(right - MIN_CROP);
            nw = right - nx;
        }
        Handle::S => nh = (h + dn.y).max(MIN_CROP),
        Handle::N => {
            ny = (y + dn.y).min(bottom - MIN_CROP);
            nh = bottom - ny;
        }
        Handle::SE => {
            nw = (w + dn.x).max(MIN_CROP);
            nh = (h + dn.y).max(MIN_CROP);
        }
        Handle::NE => {
            nw = (w + dn.x).max(MIN_CROP);
            ny = (y + dn.y).min(bottom - MIN_CROP);
            nh = bottom - ny;
        }
        Handle::SW => {
            nx = (x + dn.x).min(right - MIN_CROP);
            nw = right - nx;
            nh = (h + dn.y).max(MIN_CROP);
        }
        Handle::NW => {
            nx = (x + dn.x).min(right - MIN_CROP);
            nw = right - nx;
            ny = (y + dn.y).min(bottom - MIN_CROP);
            nh = bottom - ny;
        }
    }

    // Enforce a locked aspect: width leads for E/W/corners, height for N/S.
    if let Some(a) = aspect {
        if handle != Handle::Move {
            match handle {
                Handle::N | Handle::S => {
                    nw = (nh * a).max(MIN_CROP);
                    nx = (x + (w - nw) * 0.5).clamp(0.0, 1.0 - nw);
                }
                _ => {
                    nh = (nw / a).max(MIN_CROP);
                    match handle {
                        Handle::NE | Handle::NW => ny = bottom - nh,
                        Handle::E | Handle::W => {
                            ny = (y + (h - nh) * 0.5).clamp(0.0, 1.0 - nh)
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    // Clamp inside the frame.
    nw = nw.min(1.0);
    nh = nh.min(1.0);
    nx = nx.clamp(0.0, 1.0 - nw);
    ny = ny.clamp(0.0, 1.0 - nh);
    *crop = [nx, ny, nw, nh];
}

/// Draw and edit the selected mask over the image. Returns true if the
/// mask geometry (or brush strokes) changed.
fn mask_overlay(
    ui: &egui::Ui,
    painter: &egui::Painter,
    resp: &egui::Response,
    img_rect: egui::Rect,
    m: MaskEditor,
) -> bool {
    use crate::engine::params::{Dab, MaskKind};

    let to_screen = |p: [f32; 2]| {
        egui::pos2(
            img_rect.min.x + p[0] * img_rect.width(),
            img_rect.min.y + p[1] * img_rect.height(),
        )
    };
    let to_norm = |p: egui::Pos2| {
        [
            ((p.x - img_rect.min.x) / img_rect.width()).clamp(0.0, 1.0),
            ((p.y - img_rect.min.y) / img_rect.height()).clamp(0.0, 1.0),
        ]
    };
    let line = egui::Stroke::new(1.5, egui::Color32::from_rgb(120, 200, 255));
    let handle_fill = egui::Color32::from_gray(245);
    let mut changed = false;

    match m.kind {
        MaskKind::Radial {
            center,
            radius,
            feather,
        } => {
            let c = to_screen(*center);
            let rx = radius[0] * img_rect.width();
            let ry = radius[1] * img_rect.height();
            // Ellipse outline (polyline) + inner feather ring.
            draw_ellipse(painter, c, rx, ry, line);
            let inner = 1.0 - feather.clamp(0.0, 1.0);
            draw_ellipse(
                painter,
                c,
                rx * inner,
                ry * inner,
                egui::Stroke::new(0.7, egui::Color32::from_white_alpha(70)),
            );
            // Handles: center + right (rx) + bottom (ry).
            let h_right = egui::pos2(c.x + rx, c.y);
            let h_bottom = egui::pos2(c.x, c.y + ry);
            for h in [c, h_right, h_bottom] {
                painter.circle_filled(h, 5.0, handle_fill);
            }

            let drag_id = ui.id().with("radial_handle");
            let mut which: Option<u8> = ui.data(|d| d.get_temp(drag_id)).flatten();
            if resp.drag_started() {
                if let Some(p) = resp.interact_pointer_pos() {
                    which = if p.distance(h_right) < 12.0 {
                        Some(1)
                    } else if p.distance(h_bottom) < 12.0 {
                        Some(2)
                    } else {
                        Some(0) // move center
                    };
                }
            }
            if resp.dragged() {
                if let (Some(w), Some(p)) = (which, resp.interact_pointer_pos()) {
                    match w {
                        1 => {
                            radius[0] =
                                ((p.x - c.x).abs() / img_rect.width()).clamp(0.02, 1.0);
                        }
                        2 => {
                            radius[1] =
                                ((p.y - c.y).abs() / img_rect.height()).clamp(0.02, 1.0);
                        }
                        _ => *center = to_norm(p),
                    }
                    changed = true;
                }
            }
            if resp.drag_stopped() {
                which = None;
            }
            ui.data_mut(|d| d.insert_temp(drag_id, which));
        }
        MaskKind::Linear { p0, p1 } => {
            let a = to_screen(*p0);
            let b = to_screen(*p1);
            painter.line_segment([a, b], line);
            // Perpendicular guide lines through the endpoints.
            let dir = (b - a).normalized();
            let perp = egui::vec2(-dir.y, dir.x) * 40.0;
            painter.line_segment(
                [a - perp, a + perp],
                egui::Stroke::new(0.8, egui::Color32::from_white_alpha(90)),
            );
            painter.line_segment(
                [b - perp, b + perp],
                egui::Stroke::new(0.8, egui::Color32::from_white_alpha(90)),
            );
            painter.circle_filled(a, 5.0, handle_fill);
            painter.circle_filled(b, 5.0, handle_fill);

            let drag_id = ui.id().with("linear_handle");
            let mut which: Option<u8> = ui.data(|d| d.get_temp(drag_id)).flatten();
            if resp.drag_started() {
                if let Some(p) = resp.interact_pointer_pos() {
                    which = if p.distance(a) < 14.0 {
                        Some(0)
                    } else if p.distance(b) < 14.0 {
                        Some(1)
                    } else {
                        Some(2) // move both
                    };
                }
            }
            if resp.dragged() {
                if let (Some(w), Some(p)) = (which, resp.interact_pointer_pos()) {
                    match w {
                        0 => *p0 = to_norm(p),
                        1 => *p1 = to_norm(p),
                        _ => {
                            let d = resp.drag_delta();
                            let dn = [d.x / img_rect.width(), d.y / img_rect.height()];
                            p0[0] = (p0[0] + dn[0]).clamp(0.0, 1.0);
                            p0[1] = (p0[1] + dn[1]).clamp(0.0, 1.0);
                            p1[0] = (p1[0] + dn[0]).clamp(0.0, 1.0);
                            p1[1] = (p1[1] + dn[1]).clamp(0.0, 1.0);
                        }
                    }
                    changed = true;
                }
            }
            if resp.drag_stopped() {
                which = None;
            }
            ui.data_mut(|d| d.insert_temp(drag_id, which));
        }
        MaskKind::Brush { dabs } => {
            // Draw a cursor circle and paint dabs while dragging.
            if let Some(p) = resp.hover_pos() {
                let r = m.brush.radius * img_rect.width();
                let col = if m.brush.erase {
                    egui::Color32::from_rgb(255, 140, 140)
                } else {
                    egui::Color32::from_rgb(140, 220, 255)
                };
                painter.circle_stroke(p, r, egui::Stroke::new(1.0, col));
                ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
            }
            if resp.dragged() || resp.drag_started() {
                if let Some(p) = resp.interact_pointer_pos() {
                    if img_rect.contains(p) {
                        dabs.push(Dab {
                            p: to_norm(p),
                            radius: m.brush.radius,
                            hardness: m.brush.hardness,
                            erase: m.brush.erase,
                        });
                        changed = true;
                    }
                }
            }
        }
    }

    changed
}

/// Draw an axis-aligned ellipse as a closed polyline.
fn draw_ellipse(painter: &egui::Painter, center: egui::Pos2, rx: f32, ry: f32, stroke: egui::Stroke) {
    const N: usize = 48;
    let pts: Vec<egui::Pos2> = (0..=N)
        .map(|i| {
            let a = i as f32 / N as f32 * std::f32::consts::TAU;
            egui::pos2(center.x + rx * a.cos(), center.y + ry * a.sin())
        })
        .collect();
    painter.add(egui::Shape::line(pts, stroke));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drag_east_grows_width() {
        let mut c = [0.25, 0.25, 0.5, 0.5];
        apply_crop_drag(&mut c, Handle::E, egui::vec2(0.1, 0.0), None);
        assert!((c[2] - 0.6).abs() < 1e-5);
        assert_eq!(c[0], 0.25);
    }

    #[test]
    fn drag_nw_keeps_bottom_right_anchored() {
        let mut c = [0.2, 0.2, 0.6, 0.6];
        apply_crop_drag(&mut c, Handle::NW, egui::vec2(0.1, 0.1), None);
        assert!((c[0] - 0.3).abs() < 1e-5);
        assert!((c[0] + c[2] - 0.8).abs() < 1e-5); // right edge unchanged
        assert!((c[1] + c[3] - 0.8).abs() < 1e-5); // bottom edge unchanged
    }

    #[test]
    fn aspect_lock_follows_width() {
        let mut c = [0.0, 0.0, 0.5, 0.5];
        apply_crop_drag(&mut c, Handle::E, egui::vec2(0.2, 0.0), Some(1.0));
        assert!((c[2] - 0.7).abs() < 1e-5);
        assert!((c[3] - 0.7).abs() < 1e-5); // height followed (norm aspect 1)
    }

    #[test]
    fn move_clamps_to_frame() {
        let mut c = [0.5, 0.5, 0.4, 0.4];
        apply_crop_drag(&mut c, Handle::Move, egui::vec2(0.5, 0.5), None);
        assert!((c[0] - 0.6).abs() < 1e-5);
        assert!((c[1] - 0.6).abs() < 1e-5);
    }
}
