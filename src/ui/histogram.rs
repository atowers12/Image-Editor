//! RGB histogram panel with Lightroom-style clipping toggles in the top
//! corners: left = shadow clipping (blue overlay), right = highlight
//! clipping (red overlay). Returns true when a toggle changed so the app
//! can re-render the preview with the overlay baked in.

use crate::engine::histogram::{Histogram, BINS};

pub fn show(
    ui: &mut egui::Ui,
    hist: Option<&Histogram>,
    clip_shadows: &mut bool,
    clip_highlights: &mut bool,
) -> bool {
    let mut changed = false;
    let width = ui.available_width();
    let height = 92.0;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, egui::Color32::from_gray(24));

    if let Some(h) = hist {
        let max = h.display_max() as f32;
        let plot = rect.shrink(6.0);
        let cols = plot.width().max(1.0) as usize;
        let channels: [(&[u32], egui::Color32); 3] = [
            (
                &h.r,
                egui::Color32::from_rgba_unmultiplied(235, 80, 80, 130),
            ),
            (
                &h.g,
                egui::Color32::from_rgba_unmultiplied(90, 210, 90, 130),
            ),
            (
                &h.b,
                egui::Color32::from_rgba_unmultiplied(90, 130, 245, 130),
            ),
        ];
        for (bins, color) in channels {
            for cx in 0..cols {
                // Average the bins covered by this pixel column.
                let b0 = cx * BINS / cols;
                let b1 = ((cx + 1) * BINS / cols).max(b0 + 1).min(BINS);
                let v: u32 = bins[b0..b1].iter().copied().max().unwrap_or(0);
                if v == 0 {
                    continue;
                }
                let frac = (v as f32 / max).min(1.0);
                let x = plot.min.x + cx as f32;
                let y_top = plot.max.y - frac * plot.height();
                painter.line_segment(
                    [egui::pos2(x, plot.max.y), egui::pos2(x, y_top)],
                    egui::Stroke::new(1.0, color),
                );
            }
        }
    } else {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "—",
            egui::FontId::proportional(14.0),
            egui::Color32::from_gray(90),
        );
    }

    // Clipping toggles in the top corners.
    changed |= clip_toggle(
        ui,
        rect,
        egui::Align2::LEFT_TOP,
        clip_shadows,
        egui::Color32::from_rgb(70, 120, 235),
        "Show shadow clipping (blue overlay)",
    );
    changed |= clip_toggle(
        ui,
        rect,
        egui::Align2::RIGHT_TOP,
        clip_highlights,
        egui::Color32::from_rgb(235, 60, 60),
        "Show highlight clipping (red overlay)",
    );
    changed
}

fn clip_toggle(
    ui: &mut egui::Ui,
    hist_rect: egui::Rect,
    corner: egui::Align2,
    on: &mut bool,
    color: egui::Color32,
    tip: &str,
) -> bool {
    let size = egui::vec2(14.0, 14.0);
    let pos = match corner {
        egui::Align2::LEFT_TOP => hist_rect.min + egui::vec2(4.0, 4.0),
        _ => egui::pos2(hist_rect.max.x - size.x - 4.0, hist_rect.min.y + 4.0),
    };
    let rect = egui::Rect::from_min_size(pos, size);
    let resp = ui
        .interact(rect, ui.id().with((tip, "clip")), egui::Sense::click())
        .on_hover_text(tip);
    let painter = ui.painter();
    let fill = if *on {
        color
    } else {
        egui::Color32::from_gray(70)
    };
    painter.circle_filled(rect.center(), 4.5, fill);
    if resp.hovered() {
        painter.circle_stroke(
            rect.center(),
            6.0,
            egui::Stroke::new(1.0, egui::Color32::from_gray(160)),
        );
    }
    if resp.clicked() {
        *on = !*on;
        return true;
    }
    false
}
