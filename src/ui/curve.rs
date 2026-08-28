//! Interactive tone-curve widget. Drag control points to bend the curve;
//! click empty space to add a point; right-click or double-click a point to
//! remove it (the two endpoints stay). A channel selector switches between
//! the master curve and the per-channel R/G/B curves.

use crate::engine::ops::curve::build_lut;
use crate::engine::params::{CurveChannel, EditParams};

pub fn show(ui: &mut egui::Ui, params: &mut EditParams, channel: &mut CurveChannel) -> bool {
    let mut changed = false;

    ui.horizontal(|ui| {
        for (ch, label, color) in [
            (CurveChannel::Master, "RGB", egui::Color32::GRAY),
            (CurveChannel::Red, "R", egui::Color32::from_rgb(230, 90, 90)),
            (
                CurveChannel::Green,
                "G",
                egui::Color32::from_rgb(90, 210, 90),
            ),
            (
                CurveChannel::Blue,
                "B",
                egui::Color32::from_rgb(110, 150, 245),
            ),
        ] {
            let selected = *channel == ch;
            if ui
                .add(egui::SelectableLabel::new(
                    selected,
                    egui::RichText::new(label).color(color),
                ))
                .clicked()
            {
                *channel = ch;
            }
        }
        if ui
            .small_button("Reset")
            .on_hover_text("Reset this channel")
            .clicked()
        {
            *pts_mut(params, *channel) = crate::engine::params::identity_curve();
            changed = true;
        }
    });

    let size = egui::vec2(ui.available_width().min(280.0), 180.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, egui::Color32::from_gray(20));

    // Grid.
    let grid = egui::Stroke::new(0.5, egui::Color32::from_gray(45));
    for i in 1..4 {
        let fx = rect.min.x + rect.width() * i as f32 / 4.0;
        let fy = rect.min.y + rect.height() * i as f32 / 4.0;
        painter.line_segment(
            [egui::pos2(fx, rect.min.y), egui::pos2(fx, rect.max.y)],
            grid,
        );
        painter.line_segment(
            [egui::pos2(rect.min.x, fy), egui::pos2(rect.max.x, fy)],
            grid,
        );
    }

    // Map between curve space (0..1, y up) and screen space.
    let to_screen = |x: f32, y: f32| {
        egui::pos2(
            rect.min.x + x * rect.width(),
            rect.max.y - y * rect.height(),
        )
    };
    let to_curve = |p: egui::Pos2| {
        (
            ((p.x - rect.min.x) / rect.width()).clamp(0.0, 1.0),
            ((rect.max.y - p.y) / rect.height()).clamp(0.0, 1.0),
        )
    };

    let ch_color = match channel {
        CurveChannel::Master => egui::Color32::from_gray(220),
        CurveChannel::Red => egui::Color32::from_rgb(230, 90, 90),
        CurveChannel::Green => egui::Color32::from_rgb(90, 210, 90),
        CurveChannel::Blue => egui::Color32::from_rgb(110, 150, 245),
    };

    // Draw the smooth curve using the same LUT the engine uses.
    let pts = pts_mut(params, *channel);
    let lut = build_lut(pts);
    let mut poly = Vec::with_capacity(lut.len());
    for (i, v) in lut.iter().enumerate() {
        let x = i as f32 / (lut.len() - 1) as f32;
        poly.push(to_screen(x, *v));
    }
    painter.add(egui::Shape::line(poly, egui::Stroke::new(1.5, ch_color)));

    // Interaction state persisted across frames: which point is being dragged.
    let drag_id = ui.id().with("curve_drag");
    let mut dragging: Option<usize> = ui.data(|d| d.get_temp(drag_id)).flatten();

    // Hit-test points.
    let hit = |p: egui::Pos2, pts: &[[f32; 2]]| -> Option<usize> {
        pts.iter()
            .enumerate()
            .filter(|(_, q)| to_screen(q[0], q[1]).distance(p) < 10.0)
            .min_by(|a, b| {
                to_screen(a.1[0], a.1[1])
                    .distance(p)
                    .partial_cmp(&to_screen(b.1[0], b.1[1]).distance(p))
                    .unwrap()
            })
            .map(|(i, _)| i)
    };

    if resp.drag_started() {
        if let Some(p) = resp.interact_pointer_pos() {
            dragging = hit(p, pts);
            if dragging.is_none() {
                // Add a point where the user grabbed.
                let (cx, cy) = to_curve(p);
                pts.push([cx, cy]);
                pts.sort_by(|a, b| a[0].partial_cmp(&b[0]).unwrap());
                dragging = pts.iter().position(|q| q[0] == cx && q[1] == cy);
                changed = true;
            }
        }
    }

    if resp.dragged() {
        if let (Some(idx), Some(p)) = (dragging, resp.interact_pointer_pos()) {
            let (mut cx, cy) = to_curve(p);
            let last = pts.len() - 1;
            // Endpoints keep their x; interior points stay between neighbors.
            if idx == 0 {
                cx = 0.0;
            } else if idx == last {
                cx = 1.0;
            } else {
                let lo = pts[idx - 1][0] + 0.005;
                let hi = pts[idx + 1][0] - 0.005;
                cx = cx.clamp(lo, hi);
            }
            pts[idx] = [cx, cy];
            changed = true;
        }
    }

    if resp.drag_stopped() {
        dragging = None;
    }

    // Remove a point on right-click / double-click (never the endpoints).
    if resp.double_clicked() || resp.secondary_clicked() {
        if let Some(p) = resp.interact_pointer_pos() {
            if let Some(idx) = hit(p, pts) {
                if idx != 0 && idx != pts.len() - 1 {
                    pts.remove(idx);
                    changed = true;
                }
            }
        }
    }

    // Draw control points.
    for q in pts.iter() {
        let c = to_screen(q[0], q[1]);
        painter.circle_filled(c, 4.0, egui::Color32::WHITE);
        painter.circle_stroke(c, 4.0, egui::Stroke::new(1.0, ch_color));
    }

    ui.data_mut(|d| d.insert_temp(drag_id, dragging));
    changed
}

fn pts_mut(params: &mut EditParams, channel: CurveChannel) -> &mut Vec<[f32; 2]> {
    match channel {
        CurveChannel::Master => &mut params.curve.master,
        CurveChannel::Red => &mut params.curve.r,
        CurveChannel::Green => &mut params.curve.g,
        CurveChannel::Blue => &mut params.curve.b,
    }
}
