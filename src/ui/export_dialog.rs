//! Export settings modal: format + JPEG quality, then a native save dialog.

use crate::imgio::export::ExportFormat;

pub struct ExportDialog {
    pub open: bool,
    pub format: ExportFormat,
    pub jpeg_quality: u8,
}

impl Default for ExportDialog {
    fn default() -> Self {
        Self {
            open: false,
            format: ExportFormat::Jpeg,
            jpeg_quality: 90,
        }
    }
}

pub struct ExportRequest {
    pub format: ExportFormat,
    pub jpeg_quality: u8,
}

impl ExportDialog {
    /// Returns Some when the user confirms; the caller then picks the
    /// destination with a native dialog and hands off to the worker.
    pub fn show(&mut self, ctx: &egui::Context) -> Option<ExportRequest> {
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
                egui::ComboBox::from_label("Format")
                    .selected_text(self.format.label())
                    .show_ui(ui, |ui| {
                        for f in ExportFormat::ALL {
                            ui.selectable_value(&mut self.format, f, f.label());
                        }
                    });
                if self.format == ExportFormat::Jpeg {
                    ui.add(
                        egui::Slider::new(&mut self.jpeg_quality, 10..=100).text("Quality"),
                    );
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Choose destination…").clicked() {
                        result = Some(ExportRequest {
                            format: self.format,
                            jpeg_quality: self.jpeg_quality,
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
