//! Export settings modal: scope (current photo / whole folder), format,
//! and JPEG quality; the app follows up with a native save/folder dialog.

use crate::imgio::export::ExportFormat;

pub struct ExportDialog {
    pub open: bool,
    pub format: ExportFormat,
    pub jpeg_quality: u8,
    pub batch: bool,
}

impl Default for ExportDialog {
    fn default() -> Self {
        Self {
            open: false,
            format: ExportFormat::Jpeg,
            jpeg_quality: 90,
            batch: false,
        }
    }
}

pub struct ExportRequest {
    pub format: ExportFormat,
    pub jpeg_quality: u8,
    pub batch: bool,
}

impl ExportDialog {
    /// Returns Some when the user confirms; the caller then picks the
    /// destination with a native dialog and hands off to the worker.
    /// `folder_count` is how many photos a batch would cover.
    pub fn show(&mut self, ctx: &egui::Context, folder_count: usize) -> Option<ExportRequest> {
        if !self.open {
            return None;
        }
        let mut result = None;
        let mut open = self.open;
        egui::Window::new("Export")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.radio_value(&mut self.batch, false, "Current photo");
                ui.radio_value(
                    &mut self.batch,
                    true,
                    format!("All {folder_count} photos in folder (each with its own edits)"),
                );
                ui.add_space(6.0);
                egui::ComboBox::from_label("Format")
                    .selected_text(self.format.label())
                    .show_ui(ui, |ui| {
                        for f in ExportFormat::ALL {
                            ui.selectable_value(&mut self.format, f, f.label());
                        }
                    });
                if self.format == ExportFormat::Jpeg {
                    ui.add(egui::Slider::new(&mut self.jpeg_quality, 10..=100).text("Quality"));
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let label = if self.batch {
                        "Choose folder…"
                    } else {
                        "Choose destination…"
                    };
                    if ui.button(label).clicked() {
                        result = Some(ExportRequest {
                            format: self.format,
                            jpeg_quality: self.jpeg_quality,
                            batch: self.batch,
                        });
                    }
                    if ui.button("Cancel").clicked() {
                        self.open = false;
                    }
                });
            });
        if !open || result.is_some() {
            self.open = false;
        }
        result
    }
}
