//! Vertical filmstrip of the current folder's photos. Returns the index of
//! a newly clicked photo, if any.

use std::collections::HashMap;
use std::path::PathBuf;

pub fn show(
    ui: &mut egui::Ui,
    files: &[PathBuf],
    selected: Option<usize>,
    thumbs: &HashMap<PathBuf, egui::TextureHandle>,
) -> Option<usize> {
    let mut clicked = None;
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (i, path) in files.iter().enumerate() {
                let is_selected = selected == Some(i);
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();

                ui.vertical_centered_justified(|ui| {
                    let target_w = (ui.available_width() - 12.0).max(60.0);
                    let resp = match thumbs.get(path) {
                        Some(tex) => {
                            let tex_size = tex.size_vec2();
                            let scale = (target_w / tex_size.x).min(1.5);
                            let size = tex_size * scale;
                            let image = egui::Image::new(egui::load::SizedTexture::new(
                                tex.id(),
                                size,
                            ));
                            ui.add(egui::ImageButton::new(image).selected(is_selected))
                        }
                        None => {
                            // Thumb not ready yet: gray placeholder box.
                            let (rect, resp) = ui.allocate_exact_size(
                                egui::vec2(target_w, target_w * 0.66),
                                egui::Sense::click(),
                            );
                            ui.painter().rect_filled(
                                rect,
                                4.0,
                                ui.visuals().extreme_bg_color,
                            );
                            ui.painter().text(
                                rect.center(),
                                egui::Align2::CENTER_CENTER,
                                "…",
                                egui::FontId::proportional(18.0),
                                ui.visuals().weak_text_color(),
                            );
                            if is_selected {
                                ui.painter().rect_stroke(
                                    rect,
                                    4.0,
                                    egui::Stroke::new(2.0, ui.visuals().selection.bg_fill),
                                    egui::StrokeKind::Outside,
                                );
                            }
                            resp
                        }
                    };
                    if resp.clicked() {
                        clicked = Some(i);
                    }
                    resp.on_hover_text(&name);
                    let label_text = egui::RichText::new(name).small();
                    let label_text = if is_selected {
                        label_text.strong()
                    } else {
                        label_text.weak()
                    };
                    ui.add(egui::Label::new(label_text).truncate());
                    ui.add_space(6.0);
                });
            }
        });
    clicked
}
