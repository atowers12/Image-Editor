//! Top-level application: state, layout, and the glue between the UI,
//! the render worker, thumbnails, and sidecar persistence.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::engine::params::EditParams;
use crate::engine::worker::{self, Cmd, Reply, Worker};
use crate::imgio::{loader, sidecar, thumbs};
use crate::ui::{adjustments, export_dialog::ExportDialog, filmstrip, preview};

/// How long after the last slider tweak before the sidecar is written.
const SIDECAR_DEBOUNCE: Duration = Duration::from_millis(700);
/// How long transient status/error messages stay visible.
const TOAST_TTL: Duration = Duration::from_secs(5);

pub struct App {
    worker: Worker,
    thumbs: thumbs::ThumbWorker,

    folder: Option<PathBuf>,
    files: Vec<PathBuf>,
    selected: Option<usize>,

    params: EditParams,
    sidecar_dirty: bool,
    last_edit: Instant,

    preview_tex: Option<egui::TextureHandle>,
    thumb_tex: HashMap<PathBuf, egui::TextureHandle>,
    full_size: Option<(usize, usize)>,

    loading: bool,
    exporting: bool,
    export_dialog: ExportDialog,
    preview_state: preview::PreviewState,
    active_band: usize,

    status: Option<(String, Instant)>,
    error: Option<(String, Instant)>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        Self {
            worker: worker::spawn(cc.egui_ctx.clone()),
            thumbs: thumbs::spawn(cc.egui_ctx.clone()),
            folder: None,
            files: Vec::new(),
            selected: None,
            params: EditParams::default(),
            sidecar_dirty: false,
            last_edit: Instant::now(),
            preview_tex: None,
            thumb_tex: HashMap::new(),
            full_size: None,
            loading: false,
            exporting: false,
            export_dialog: ExportDialog::default(),
            preview_state: preview::PreviewState::default(),
            active_band: 0,
            status: None,
            error: None,
        }
    }

    fn current_path(&self) -> Option<&PathBuf> {
        self.selected.and_then(|i| self.files.get(i))
    }

    fn poll_workers(&mut self, ctx: &egui::Context) {
        while let Ok(reply) = self.worker.rx.try_recv() {
            match reply {
                Reply::Loaded {
                    path,
                    full_width,
                    full_height,
                } => {
                    // Ignore late replies for a photo we've already left.
                    if Some(&path) == self.current_path() {
                        self.full_size = Some((full_width, full_height));
                    }
                }
                Reply::Preview {
                    width,
                    height,
                    rgba,
                } => {
                    let img =
                        egui::ColorImage::from_rgba_unmultiplied([width, height], &rgba);
                    match &mut self.preview_tex {
                        Some(tex) => tex.set(img, egui::TextureOptions::LINEAR),
                        None => {
                            self.preview_tex = Some(ctx.load_texture(
                                "preview",
                                img,
                                egui::TextureOptions::LINEAR,
                            ))
                        }
                    }
                    self.loading = false;
                }
                Reply::ExportDone(path) => {
                    self.exporting = false;
                    self.status = Some((
                        format!("Exported {}", path.file_name().unwrap_or_default().to_string_lossy()),
                        Instant::now(),
                    ));
                }
                Reply::Error(e) => {
                    self.loading = false;
                    self.exporting = false;
                    self.error = Some((e, Instant::now()));
                }
            }
        }
        while let Ok(thumb) = self.thumbs.rx.try_recv() {
            let img = egui::ColorImage::from_rgba_unmultiplied(
                [thumb.width, thumb.height],
                &thumb.rgba,
            );
            let tex = ctx.load_texture(
                format!("thumb:{}", thumb.path.display()),
                img,
                egui::TextureOptions::LINEAR,
            );
            self.thumb_tex.insert(thumb.path, tex);
        }
    }

    fn save_sidecar_now(&mut self) {
        if !self.sidecar_dirty {
            return;
        }
        if let Some(path) = self.current_path().cloned() {
            if let Err(e) = sidecar::save(&path, &self.params) {
                self.error = Some((format!("couldn't save edits: {e:#}"), Instant::now()));
            }
        }
        self.sidecar_dirty = false;
    }

    fn params_edited(&mut self) {
        self.sidecar_dirty = true;
        self.last_edit = Instant::now();
        let _ = self.worker.tx.send(Cmd::Render(self.params));
    }

    fn select_photo(&mut self, index: usize) {
        if Some(index) == self.selected {
            return;
        }
        self.save_sidecar_now();
        let Some(path) = self.files.get(index).cloned() else {
            return;
        };
        self.selected = Some(index);
        self.params = sidecar::load(&path).unwrap_or_default();
        self.full_size = None;
        self.loading = true;
        self.preview_state = preview::PreviewState::default();
        let _ = self.worker.tx.send(Cmd::Load {
            path,
            params: self.params,
        });
    }

    fn open_folder_dialog(&mut self) {
        if let Some(dir) = rfd::FileDialog::new().pick_folder() {
            self.set_folder(dir, None);
        }
    }

    fn open_file_dialog(&mut self) {
        let all_exts: Vec<&str> = loader::STD_EXTS
            .iter()
            .chain(loader::RAW_EXTS.iter())
            .copied()
            .collect();
        let picked = rfd::FileDialog::new()
            .add_filter("Photos", &all_exts)
            .pick_file();
        if let Some(file) = picked {
            if let Some(parent) = file.parent().map(Path::to_path_buf) {
                self.set_folder(parent, Some(file));
            }
        }
    }

    /// Scan a folder for supported photos, kick off thumbnails, and select
    /// either `focus` (if given) or the first photo.
    fn set_folder(&mut self, dir: PathBuf, focus: Option<PathBuf>) {
        self.save_sidecar_now();
        let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .map(|e| e.path())
                    .filter(|p| p.is_file() && loader::is_supported(p))
                    .collect()
            })
            .unwrap_or_default();
        files.sort_by_key(|p| {
            p.file_name()
                .map(|n| n.to_string_lossy().to_lowercase())
                .unwrap_or_default()
        });

        for f in &files {
            if !self.thumb_tex.contains_key(f) {
                let _ = self.thumbs.tx.send(f.clone());
            }
        }

        self.folder = Some(dir);
        self.files = files;
        self.selected = None;
        self.preview_tex = None;
        self.full_size = None;

        let start = focus
            .and_then(|f| self.files.iter().position(|p| *p == f))
            .or(if self.files.is_empty() { None } else { Some(0) });
        if let Some(i) = start {
            self.select_photo(i);
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.button("📂 Open Folder…").clicked() {
                self.open_folder_dialog();
            }
            if ui.button("🖼 Open File…").clicked() {
                self.open_file_dialog();
            }
            ui.separator();

            let has_photo = self.current_path().is_some();
            if ui
                .add_enabled(has_photo && !self.exporting, egui::Button::new("💾 Export…"))
                .clicked()
            {
                self.export_dialog.open = true;
            }
            if ui
                .add_enabled(has_photo, egui::Button::new("↺ Reset All"))
                .clicked()
            {
                self.params = EditParams::default();
                self.params_edited();
            }
            ui.separator();

            if let Some(path) = self.current_path() {
                let mut label = path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                if let Some((w, h)) = self.full_size {
                    label.push_str(&format!("  ({w} × {h})"));
                }
                ui.label(egui::RichText::new(label).weak());
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.exporting {
                    ui.add(egui::Spinner::new());
                    ui.label("Exporting…");
                }
                if let Some((msg, at)) = &self.error {
                    if at.elapsed() < TOAST_TTL {
                        ui.colored_label(egui::Color32::from_rgb(240, 100, 100), msg);
                    } else {
                        self.error = None;
                    }
                } else if let Some((msg, at)) = &self.status {
                    if at.elapsed() < TOAST_TTL {
                        ui.colored_label(egui::Color32::from_rgb(120, 200, 120), msg);
                    } else {
                        self.status = None;
                    }
                }
            });
        });
    }

    fn handle_export(&mut self, ctx: &egui::Context) {
        let Some(request) = self.export_dialog.show(ctx) else {
            return;
        };
        let Some(src_path) = self.current_path().cloned() else {
            return;
        };
        let stem = src_path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let default_name = format!("{stem}_edited.{}", request.format.extension());
        let mut dialog = rfd::FileDialog::new()
            .set_file_name(&default_name)
            .add_filter(request.format.label(), &[request.format.extension()]);
        if let Some(dir) = &self.folder {
            dialog = dialog.set_directory(dir);
        }
        if let Some(dest) = dialog.save_file() {
            self.exporting = true;
            let _ = self.worker.tx.send(Cmd::Export {
                dest,
                params: self.params,
                format: request.format,
                jpeg_quality: request.jpeg_quality,
            });
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_workers(ctx);

        // Debounced sidecar autosave.
        if self.sidecar_dirty && self.last_edit.elapsed() > SIDECAR_DEBOUNCE {
            self.save_sidecar_now();
        }

        egui::TopBottomPanel::top("top_bar").show(ctx, |ui| {
            ui.add_space(4.0);
            self.top_bar(ui);
            ui.add_space(4.0);
        });

        egui::SidePanel::left("filmstrip")
            .resizable(true)
            .default_width(170.0)
            .width_range(110.0..=320.0)
            .show(ctx, |ui| {
                if self.files.is_empty() {
                    ui.add_space(12.0);
                    ui.vertical_centered(|ui| {
                        ui.label(egui::RichText::new("No photos").weak());
                    });
                } else if let Some(i) = filmstrip::show(
                    ui,
                    &self.files,
                    self.selected,
                    &self.thumb_tex,
                ) {
                    self.select_photo(i);
                }
            });

        egui::SidePanel::right("adjustments")
            .resizable(true)
            .default_width(300.0)
            .width_range(240.0..=420.0)
            .show(ctx, |ui| {
                ui.add_space(6.0);
                if self.current_path().is_some() {
                    if adjustments::show(ui, &mut self.params, &mut self.active_band) {
                        self.params_edited();
                    }
                } else {
                    ui.vertical_centered(|ui| {
                        ui.label(egui::RichText::new("Open a photo to edit").weak());
                    });
                }
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| {
                preview::show(
                    ui,
                    &mut self.preview_state,
                    self.preview_tex.as_ref(),
                    self.loading,
                );
            });

        self.handle_export(ctx);

        // Keep polling while a toast is fading or a debounced save is pending.
        if self.sidecar_dirty || self.status.is_some() || self.error.is_some() {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.save_sidecar_now();
    }
}
