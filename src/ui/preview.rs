//! The central photo view: fit-to-window by default, mouse-wheel zoom
//! around the cursor, drag to pan, double-click to toggle fit/100%.

pub struct PreviewState {
    pub fit: bool,
    pub zoom: f32,
    pub offset: egui::Vec2,
}

impl Default for PreviewState {
    fn default() -> Self {
        Self {
            fit: true,
            zoom: 1.0,
            offset: egui::Vec2::ZERO,
        }
    }
}

pub fn show(
    ui: &mut egui::Ui,
    state: &mut PreviewState,
    tex: Option<&egui::TextureHandle>,
    loading: bool,
) {
    let rect = ui.available_rect_before_wrap();
    let resp = ui.allocate_rect(rect, egui::Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, egui::Color32::from_gray(16));

    if let Some(tex) = tex {
        let img_size = tex.size_vec2();
        let fit_scale = (rect.width() / img_size.x)
            .min(rect.height() / img_size.y)
            .min(4.0);
        let mut scale = if state.fit { fit_scale } else { state.zoom };

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
                state.zoom = 1.0; // 100%
                if let Some(ptr) = resp.hover_pos() {
                    // Center the clicked point.
                    let frac = (ptr - (rect.center() + state.offset)) / fit_scale;
                    state.offset = -frac * 1.0;
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

        let size = img_size * scale;
        // Keep at least part of the image on screen.
        let max_x = (size.x + rect.width()) * 0.5 - 40.0;
        let max_y = (size.y + rect.height()) * 0.5 - 40.0;
        state.offset.x = state.offset.x.clamp(-max_x, max_x);
        state.offset.y = state.offset.y.clamp(-max_y, max_y);

        let img_rect = egui::Rect::from_center_size(rect.center() + state.offset, size);
        painter.image(
            tex.id(),
            img_rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );

        // Zoom readout, bottom-left.
        let label = if state.fit {
            "Fit".to_string()
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
    } else if !loading {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "Open a folder or photo to start editing",
            egui::FontId::proportional(16.0),
            egui::Color32::from_gray(120),
        );
    }

    if loading {
        let spinner_rect =
            egui::Rect::from_center_size(rect.center(), egui::vec2(36.0, 36.0));
        ui.put(spinner_rect, egui::Spinner::new().size(36.0));
    }
}
