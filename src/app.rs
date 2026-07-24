//! Top-level application: state, layout, and the glue between the UI,
//! the render worker, thumbnails, sidecar persistence, and settings.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::engine::histogram::Histogram;
use crate::engine::params::{CurveChannel, EditParams, Flag};
use crate::engine::tuning::Tuning;
use crate::engine::worker::{self, Cmd, Reply, Worker};
use crate::imgio::export::{BatchJob, BatchMsg};
use crate::imgio::metadata::ExifInfo;
use crate::imgio::{export, loader, presets, recent, sidecar, thumbs};
use crate::ui::masks::BrushSettings;
use crate::ui::{
    adjustments,
    crop::{self, AspectLock, CropAction},
    export_dialog::ExportDialog,
    filmstrip, histogram, info, masks, preview, settings, welcome,
};

/// How long after the last slider tweak before the sidecar is written and
/// an undo step is committed.
const SIDECAR_DEBOUNCE: Duration = Duration::from_millis(700);
/// How long transient status/error messages stay visible.
const TOAST_TTL: Duration = Duration::from_secs(5);
/// Cap on the undo history depth per photo.
const MAX_HISTORY: usize = 100;

struct BatchState {
    job: BatchJob,
    done: usize,
    current: String,
}

/// Which editing mode the right panel / preview is in.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Adjust,
    Crop,
    Mask,
}

pub struct App {
    worker: Worker,
    thumbs: thumbs::ThumbWorker,

    folder: Option<PathBuf>,
    files: Vec<PathBuf>,
    selected: Option<usize>,
    recent: Vec<PathBuf>,
    /// Cached rating/flag per file for filmstrip badges (culling).
    meta_cache: HashMap<PathBuf, (u8, Flag)>,

    params: EditParams,
    tuning: Tuning,
    copied_params: Option<EditParams>,
    confirm_paste_all: bool,
    sidecar_dirty: bool,
    last_edit: Instant,

    // Undo / redo (per photo).
    undo: Vec<EditParams>,
    redo: Vec<EditParams>,
    committed: EditParams,

    preview_tex: Option<egui::TextureHandle>,
    thumb_tex: HashMap<PathBuf, egui::TextureHandle>,
    full_size: Option<(usize, usize)>,
    oriented_dims: Option<(usize, usize)>,
    exif: ExifInfo,
    hist: Option<Histogram>,
    clip_shadows: bool,
    clip_highlights: bool,

    region_tex: Option<egui::TextureHandle>,
    region_rect: Option<[f32; 4]>,
    last_region_req: Option<preview::RegionRequest>,

    loading: bool,
    exporting: bool,
    batch: Option<BatchState>,
    mode: Mode,
    before_view: bool,
    eyedropper: bool,
    aspect: AspectLock,
    active_band: usize,
    curve_channel: CurveChannel,
    selected_mask: Option<usize>,
    brush: BrushSettings,

    export_dialog: ExportDialog,
    settings_open: bool,
    preset_list: Vec<String>,
    new_preset_name: String,
    preview_state: preview::PreviewState,

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
            recent: recent::load(),
            meta_cache: HashMap::new(),
            params: EditParams::default(),
            tuning: Tuning::load(),
            copied_params: None,
            confirm_paste_all: false,
            sidecar_dirty: false,
            last_edit: Instant::now(),
            undo: Vec::new(),
            redo: Vec::new(),
            committed: EditParams::default(),
            preview_tex: None,
            thumb_tex: HashMap::new(),
            full_size: None,
            oriented_dims: None,
            exif: ExifInfo::default(),
            hist: None,
            clip_shadows: false,
            clip_highlights: false,
            region_tex: None,
            region_rect: None,
            last_region_req: None,
            loading: false,
            exporting: false,
            batch: None,
            mode: Mode::Adjust,
            before_view: false,
            eyedropper: false,
            aspect: AspectLock::Free,
            active_band: 0,
            curve_channel: CurveChannel::Master,
            selected_mask: None,
            brush: BrushSettings::default(),
            export_dialog: ExportDialog::default(),
            settings_open: false,
            preset_list: presets::list(),
            new_preset_name: String::new(),
            preview_state: preview::PreviewState::default(),
            status: None,
            error: None,
        }
    }

    fn current_path(&self) -> Option<&PathBuf> {
        self.selected.and_then(|i| self.files.get(i))
    }

    fn effective_params(&self) -> EditParams {
        if self.before_view {
            self.params.without_pixel_edits()
        } else {
            self.params.clone()
        }
    }

    fn clip_flags(&self) -> (bool, bool) {
        (self.clip_shadows, self.clip_highlights)
    }

    fn request_render(&mut self) {
        self.invalidate_region();
        let _ = self.worker.tx.send(Cmd::Render {
            params: self.effective_params(),
            tuning: self.tuning,
            include_crop: self.mode != Mode::Crop,
            clip: self.clip_flags(),
        });
    }

    fn poll_workers(&mut self, ctx: &egui::Context) {
        while let Ok(reply) = self.worker.rx.try_recv() {
            match reply {
                Reply::Loaded {
                    path,
                    full_width,
                    full_height,
                    exif,
                } => {
                    if Some(&path) == self.current_path() {
                        self.full_size = Some((full_width, full_height));
                        self.exif = exif;
                    }
                }
                Reply::Neutral { temp, tint } => {
                    self.params.temp = temp;
                    self.params.tint = tint;
                    self.eyedropper = false;
                    self.before_view = false;
                    self.params_edited();
                }
                Reply::Preview {
                    width,
                    height,
                    rgba,
                    full_size,
                    histogram,
                } => {
                    let img = egui::ColorImage::from_rgba_unmultiplied([width, height], &rgba);
                    match &mut self.preview_tex {
                        Some(tex) => tex.set(img, egui::TextureOptions::LINEAR),
                        None => {
                            self.preview_tex =
                                Some(ctx.load_texture("preview", img, egui::TextureOptions::LINEAR))
                        }
                    }
                    self.oriented_dims = Some(full_size);
                    self.hist = Some(histogram);
                    self.loading = false;
                }
                Reply::Region {
                    width,
                    height,
                    rgba,
                    norm_rect,
                } => {
                    let img = egui::ColorImage::from_rgba_unmultiplied([width, height], &rgba);
                    match &mut self.region_tex {
                        Some(tex) => tex.set(img, egui::TextureOptions::LINEAR),
                        None => {
                            self.region_tex =
                                Some(ctx.load_texture("region", img, egui::TextureOptions::LINEAR))
                        }
                    }
                    self.region_rect = Some(norm_rect);
                }
                Reply::ExportDone(path) => {
                    self.exporting = false;
                    self.status = Some((
                        format!(
                            "Exported {}",
                            path.file_name().unwrap_or_default().to_string_lossy()
                        ),
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
            let img =
                egui::ColorImage::from_rgba_unmultiplied([thumb.width, thumb.height], &thumb.rgba);
            let tex = ctx.load_texture(
                format!("thumb:{}", thumb.path.display()),
                img,
                egui::TextureOptions::LINEAR,
            );
            self.thumb_tex.insert(thumb.path, tex);
        }

        let mut batch_finished = None;
        if let Some(b) = &mut self.batch {
            while let Ok(msg) = b.job.rx.try_recv() {
                match msg {
                    BatchMsg::Progress { index, name } => {
                        b.done = index;
                        b.current = name;
                    }
                    BatchMsg::Failed { name, error } => {
                        self.error = Some((format!("{name}: {error}"), Instant::now()));
                    }
                    BatchMsg::Finished { exported, failed } => {
                        batch_finished = Some((exported, failed));
                    }
                }
            }
        }
        if let Some((exported, failed)) = batch_finished {
            self.batch = None;
            let msg = if failed == 0 {
                format!("Exported {exported} photos")
            } else {
                format!("Exported {exported} photos ({failed} failed)")
            };
            self.status = Some((msg, Instant::now()));
        }
    }

    /// Commit the current params as an undo step and persist the sidecar.
    fn commit_and_save(&mut self) {
        if self.params != self.committed {
            self.undo.push(self.committed.clone());
            if self.undo.len() > MAX_HISTORY {
                self.undo.remove(0);
            }
            self.redo.clear();
            self.committed = self.params.clone();
        }
        if self.sidecar_dirty {
            if let Some(path) = self.current_path().cloned() {
                if let Err(e) = sidecar::save(&path, &self.params) {
                    self.error = Some((format!("couldn't save edits: {e:#}"), Instant::now()));
                }
                if self.params.rating > 0 || self.params.flag != Flag::None {
                    self.meta_cache
                        .insert(path, (self.params.rating, self.params.flag));
                } else {
                    self.meta_cache.remove(&path);
                }
            }
            self.sidecar_dirty = false;
        }
    }

    fn params_edited(&mut self) {
        self.sidecar_dirty = true;
        self.last_edit = Instant::now();
        self.request_render();
    }

    fn undo(&mut self) {
        // Flush any pending edit into history first.
        if self.params != self.committed {
            self.undo.push(self.committed.clone());
            self.committed = self.params.clone();
        }
        if let Some(prev) = self.undo.pop() {
            self.redo.push(self.params.clone());
            self.params = prev.clone();
            self.committed = prev;
            self.sidecar_dirty = true;
            self.before_view = false;
            self.request_render();
        }
    }

    fn redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            self.undo.push(self.params.clone());
            self.params = next.clone();
            self.committed = next;
            self.sidecar_dirty = true;
            self.before_view = false;
            self.request_render();
        }
    }

    fn crop_rect_edited(&mut self) {
        self.sidecar_dirty = true;
        self.last_edit = Instant::now();
    }

    fn invalidate_region(&mut self) {
        self.last_region_req = None;
    }

    fn select_photo(&mut self, index: usize) {
        if Some(index) == self.selected {
            return;
        }
        self.commit_and_save();
        let Some(path) = self.files.get(index).cloned() else {
            return;
        };
        self.selected = Some(index);
        self.params = sidecar::load(&path).unwrap_or_default();
        self.committed = self.params.clone();
        self.undo.clear();
        self.redo.clear();
        self.full_size = None;
        self.oriented_dims = None;
        self.exif = ExifInfo::default();
        self.hist = None;
        self.loading = true;
        self.mode = Mode::Adjust;
        self.before_view = false;
        self.eyedropper = false;
        self.selected_mask = None;
        self.aspect = AspectLock::Free;
        self.preview_state = preview::PreviewState::default();
        self.region_tex = None;
        self.region_rect = None;
        self.last_region_req = None;
        let _ = self.worker.tx.send(Cmd::Load {
            path,
            params: self.params.clone(),
            tuning: self.tuning,
            clip: self.clip_flags(),
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

    fn set_folder(&mut self, dir: PathBuf, focus: Option<PathBuf>) {
        self.commit_and_save();
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

        if files.is_empty() {
            self.error = Some((
                format!("No supported photos in {}", dir.display()),
                Instant::now(),
            ));
            return;
        }

        self.meta_cache.clear();
        for f in &files {
            if !self.thumb_tex.contains_key(f) {
                let _ = self.thumbs.tx.send(f.clone());
            }
            if let Some(p) = sidecar::load(f) {
                if p.rating > 0 || p.flag != Flag::None {
                    self.meta_cache.insert(f.clone(), (p.rating, p.flag));
                }
            }
        }

        recent::remember(&mut self.recent, dir.clone());
        self.folder = Some(dir);
        self.files = files;
        self.selected = None;
        self.preview_tex = None;
        self.full_size = None;

        let start = focus
            .and_then(|f| self.files.iter().position(|p| *p == f))
            .unwrap_or(0);
        self.select_photo(start);
    }

    fn apply_preset(&mut self, name: &str) {
        match presets::load(name) {
            Ok(p) => {
                self.params.apply_edits_from(&p);
                self.before_view = false;
                self.params_edited();
                self.status = Some((format!("Applied preset “{name}”"), Instant::now()));
            }
            Err(e) => self.error = Some((format!("{e:#}"), Instant::now())),
        }
    }

    fn keyboard(&mut self, ctx: &egui::Context) {
        if ctx.wants_keyboard_input() {
            return; // don't steal keys while typing in a text field
        }
        let (undo, redo, digits, pick, reject) = ctx.input(|i| {
            let undo = i.modifiers.command && !i.modifiers.shift && i.key_pressed(egui::Key::Z);
            let redo = i.modifiers.command
                && (i.key_pressed(egui::Key::Y) || (i.modifiers.shift && i.key_pressed(egui::Key::Z)));
            let mut digit = None;
            for (k, n) in [
                (egui::Key::Num0, 0u8),
                (egui::Key::Num1, 1),
                (egui::Key::Num2, 2),
                (egui::Key::Num3, 3),
                (egui::Key::Num4, 4),
                (egui::Key::Num5, 5),
            ] {
                if i.key_pressed(k) {
                    digit = Some(n);
                }
            }
            (
                undo,
                redo,
                digit,
                i.key_pressed(egui::Key::P),
                i.key_pressed(egui::Key::X),
            )
        });
        if self.current_path().is_none() {
            return;
        }
        if undo {
            self.undo();
        }
        if redo {
            self.redo();
        }
        if let Some(n) = digits {
            self.params.rating = n;
            self.params_edited();
        }
        if pick {
            self.params.flag = if self.params.flag == Flag::Pick { Flag::None } else { Flag::Pick };
            self.params_edited();
        }
        if reject {
            self.params.flag = if self.params.flag == Flag::Reject { Flag::None } else { Flag::Reject };
            self.params_edited();
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
                .add_enabled(
                    has_photo && !self.exporting && self.batch.is_none(),
                    egui::Button::new("💾 Export…"),
                )
                .clicked()
            {
                self.export_dialog.open = true;
            }

            self.mode_toggle(ui, has_photo, Mode::Crop, "✂ Crop");
            self.mode_toggle(ui, has_photo, Mode::Mask, "✦ Mask");

            // Undo / redo.
            if ui
                .add_enabled(!self.undo.is_empty(), egui::Button::new("↶"))
                .on_hover_text("Undo (Ctrl+Z)")
                .clicked()
            {
                self.undo();
            }
            if ui
                .add_enabled(!self.redo.is_empty(), egui::Button::new("↷"))
                .on_hover_text("Redo (Ctrl+Y)")
                .clicked()
            {
                self.redo();
            }

            let mut before = self.before_view;
            if ui
                .add_enabled(
                    has_photo && self.mode == Mode::Adjust,
                    egui::SelectableLabel::new(before, "◧ Before"),
                )
                .clicked()
            {
                before = !before;
            }
            if before != self.before_view {
                self.before_view = before;
                self.request_render();
            }

            if ui
                .add_enabled(has_photo, egui::Button::new("↺ Reset"))
                .clicked()
            {
                // Keep geometry & metadata, reset the pixel edits.
                let keep = self.params.without_pixel_edits();
                self.params = keep;
                self.params_edited();
            }
            ui.separator();

            self.presets_menu(ui, has_photo);
            ui.separator();

            if ui
                .add_enabled(has_photo, egui::Button::new("⧉ Copy"))
                .on_hover_text("Copy this photo's edit settings")
                .clicked()
            {
                self.copied_params = Some(self.params.clone());
                self.status = Some(("Edits copied".into(), Instant::now()));
            }
            let can_paste = has_photo && self.copied_params.is_some();
            if ui
                .add_enabled(can_paste, egui::Button::new("📋 Paste"))
                .clicked()
            {
                if let Some(p) = self.copied_params.clone() {
                    self.params.apply_edits_from(&p);
                    self.before_view = false;
                    self.params_edited();
                }
            }
            if ui
                .add_enabled(can_paste && self.files.len() > 1, egui::Button::new("📋 All"))
                .clicked()
            {
                self.confirm_paste_all = true;
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("⚙").on_hover_text("Processing settings").clicked() {
                    self.settings_open = !self.settings_open;
                }
                if let Some(b) = &self.batch {
                    let frac = (b.done as f32 + 0.5) / b.job.total.max(1) as f32;
                    if ui.button("✖").on_hover_text("Cancel batch export").clicked() {
                        b.job.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                    ui.add(
                        egui::ProgressBar::new(frac)
                            .desired_width(150.0)
                            .text(format!("{}/{}", b.done + 1, b.job.total)),
                    );
                } else if self.exporting {
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

    fn mode_toggle(&mut self, ui: &mut egui::Ui, has_photo: bool, mode: Mode, label: &str) {
        let active = self.mode == mode;
        if ui
            .add_enabled(has_photo, egui::SelectableLabel::new(active, label))
            .clicked()
        {
            self.mode = if active { Mode::Adjust } else { mode };
            self.before_view = false;
            self.eyedropper = false;
            if self.mode == Mode::Mask && self.selected_mask.is_none() {
                self.selected_mask = self.params.masks.iter().position(|_| true);
            }
            self.request_render();
        }
    }

    fn presets_menu(&mut self, ui: &mut egui::Ui, has_photo: bool) {
        ui.menu_button("🎨 Presets", |ui| {
            ui.set_min_width(180.0);
            if self.preset_list.is_empty() {
                ui.label(egui::RichText::new("No presets yet").weak());
            }
            let mut to_apply = None;
            let mut to_delete = None;
            for name in &self.preset_list {
                ui.horizontal(|ui| {
                    if ui.add_enabled(has_photo, egui::Button::new(name).frame(false)).clicked() {
                        to_apply = Some(name.clone());
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("🗑").clicked() {
                            to_delete = Some(name.clone());
                        }
                    });
                });
            }
            ui.separator();
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.new_preset_name)
                        .hint_text("New preset name")
                        .desired_width(120.0),
                );
                if ui
                    .add_enabled(has_photo && !self.new_preset_name.trim().is_empty(), egui::Button::new("Save"))
                    .clicked()
                {
                    let name = self.new_preset_name.trim().to_string();
                    match presets::save(&name, &self.params) {
                        Ok(()) => {
                            self.status = Some((format!("Saved preset “{name}”"), Instant::now()));
                            self.new_preset_name.clear();
                            self.preset_list = presets::list();
                        }
                        Err(e) => self.error = Some((format!("{e:#}"), Instant::now())),
                    }
                }
            });
            if let Some(name) = to_apply {
                self.apply_preset(&name);
                ui.close_menu();
            }
            if let Some(name) = to_delete {
                let _ = presets::delete(&name);
                self.preset_list = presets::list();
            }
        });
    }

    fn paste_all_confirm(&mut self, ctx: &egui::Context) {
        if !self.confirm_paste_all {
            return;
        }
        let count = self.files.len();
        let mut open = true;
        let mut apply = false;
        egui::Window::new("Paste to all photos?")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(format!(
                    "This replaces the edit settings on all {count} photos in the folder.\nOriginal image files are never modified."
                ));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button(egui::RichText::new("Apply to all").strong()).clicked() {
                        apply = true;
                    }
                    if ui.button("Cancel").clicked() {
                        self.confirm_paste_all = false;
                    }
                });
            });
        if !open {
            self.confirm_paste_all = false;
        }
        if apply {
            self.confirm_paste_all = false;
            let Some(copied) = self.copied_params.clone() else {
                return;
            };
            let mut failed = 0;
            for path in &self.files {
                let mut p = sidecar::load(path).unwrap_or_default();
                p.apply_edits_from(&copied);
                if sidecar::save(path, &p).is_err() {
                    failed += 1;
                }
            }
            self.params.apply_edits_from(&copied);
            self.committed = self.params.clone();
            self.sidecar_dirty = false;
            self.before_view = false;
            self.request_render();
            let msg = if failed == 0 {
                format!("Edits applied to {count} photos")
            } else {
                format!("Edits applied ({failed} failed)")
            };
            self.status = Some((msg, Instant::now()));
        }
    }

    fn handle_export(&mut self, ctx: &egui::Context) {
        let Some(request) = self.export_dialog.show(ctx, self.files.len()) else {
            return;
        };
        if request.batch {
            self.commit_and_save();
            let mut dialog = rfd::FileDialog::new().set_title("Export folder");
            if let Some(dir) = &self.folder {
                dialog = dialog.set_directory(dir);
            }
            if let Some(dest_dir) = dialog.pick_folder() {
                let job = export::spawn_batch(
                    self.files.clone(),
                    dest_dir,
                    request.format,
                    request.jpeg_quality,
                    self.tuning,
                    ctx.clone(),
                );
                self.batch = Some(BatchState {
                    job,
                    done: 0,
                    current: String::new(),
                });
            }
            return;
        }

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
                params: self.params.clone(),
                tuning: self.tuning,
                format: request.format,
                jpeg_quality: request.jpeg_quality,
            });
        }
    }

    fn right_panel(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        if histogram::show(
            ui,
            self.hist.as_ref(),
            &mut self.clip_shadows,
            &mut self.clip_highlights,
        ) {
            self.request_render();
        }

        if self.current_path().is_none() {
            ui.add_space(10.0);
            ui.vertical_centered(|ui| {
                ui.label(egui::RichText::new("Open a photo to edit").weak());
            });
            return;
        }

        ui.add_space(4.0);
        if info::ratings(ui, &mut self.params) {
            self.params_edited();
        }
        ui.separator();

        match self.mode {
            Mode::Crop => {
                let dims = self.oriented_dims.unwrap_or((1, 1));
                match crop::panel(ui, &mut self.params, &mut self.aspect, dims) {
                    CropAction::Changed => self.params_edited(),
                    CropAction::Done => {
                        self.mode = Mode::Adjust;
                        self.params_edited();
                    }
                    CropAction::None => {}
                }
            }
            Mode::Mask => match masks::panel(
                ui,
                &mut self.params,
                &mut self.selected_mask,
                &mut self.brush,
            ) {
                masks::MaskAction::Changed => self.params_edited(),
                masks::MaskAction::Done => {
                    self.mode = Mode::Adjust;
                    self.request_render();
                }
                masks::MaskAction::None => {}
            },
            Mode::Adjust => {
                let out = adjustments::show(
                    ui,
                    &mut self.params,
                    &mut self.active_band,
                    &mut self.curve_channel,
                    self.eyedropper,
                );
                if out.eyedropper_toggled {
                    self.eyedropper = !self.eyedropper;
                }
                if out.changed {
                    self.before_view = false;
                    self.params_edited();
                }
                ui.add_space(8.0);
                egui::CollapsingHeader::new("Info")
                    .default_open(false)
                    .show(ui, |ui| info::exif_panel(ui, &self.exif));
            }
        }
    }

    fn center_panel(&mut self, ui: &mut egui::Ui) {
        let region = match (&self.region_tex, self.region_rect) {
            (Some(t), Some(r)) => Some((t, r)),
            _ => None,
        };
        let dims = self.oriented_dims.unwrap_or((1, 1));

        // Build at most one editing overlay, borrowing the relevant params.
        let crop_overlay = if self.mode == Mode::Crop {
            Some(preview::CropOverlay {
                crop: &mut self.params.crop,
                aspect: self.aspect.ratio(dims),
                dims,
            })
        } else {
            None
        };
        let mask_editor = if self.mode == Mode::Mask {
            self.selected_mask
                .filter(|&i| i < self.params.masks.len())
                .map(|i| preview::MaskEditor {
                    kind: &mut self.params.masks[i].kind,
                    brush: self.brush,
                })
        } else {
            None
        };

        let out = preview::show(
            ui,
            &mut self.preview_state,
            self.preview_tex.as_ref(),
            self.loading,
            self.oriented_dims,
            region,
            crop_overlay,
            mask_editor,
            self.eyedropper,
        );

        if out.crop_changed {
            self.crop_rect_edited();
        }
        if out.mask_changed {
            self.params_edited();
        }
        if let Some(point) = out.eyedrop_point {
            let _ = self.worker.tx.send(Cmd::SampleNeutral {
                params: self.params.clone(),
                norm_point: point,
            });
        }
        if let Some(req) = out.region_request {
            let is_new = self
                .last_region_req
                .map(|last| !req.roughly_eq(&last))
                .unwrap_or(true);
            if is_new && !self.loading {
                self.last_region_req = Some(req);
                let _ = self.worker.tx.send(Cmd::RenderRegion {
                    params: self.effective_params(),
                    tuning: self.tuning,
                    norm_rect: req.norm_rect,
                    target: req.target,
                    clip: self.clip_flags(),
                });
            }
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_workers(ctx);
        self.keyboard(ctx);

        // Debounced commit + autosave.
        if (self.sidecar_dirty || self.params != self.committed)
            && self.last_edit.elapsed() > SIDECAR_DEBOUNCE
        {
            self.commit_and_save();
        }

        egui::TopBottomPanel::top("top_bar").show(ctx, |ui| {
            ui.add_space(4.0);
            self.top_bar(ui);
            ui.add_space(4.0);
        });

        if self.files.is_empty() {
            egui::CentralPanel::default().show(ctx, |ui| {
                match welcome::show(ui, &self.recent) {
                    Some(welcome::WelcomeAction::OpenFolder) => self.open_folder_dialog(),
                    Some(welcome::WelcomeAction::OpenFile) => self.open_file_dialog(),
                    Some(welcome::WelcomeAction::OpenRecent(dir)) => self.set_folder(dir, None),
                    None => {}
                }
            });
        } else {
            egui::SidePanel::left("filmstrip")
                .resizable(true)
                .default_width(170.0)
                .width_range(110.0..=320.0)
                .show(ctx, |ui| {
                    if let Some(i) = filmstrip::show(
                        ui,
                        &self.files,
                        self.selected,
                        &self.thumb_tex,
                        &self.meta_cache,
                    ) {
                        self.select_photo(i);
                    }
                });

            egui::SidePanel::right("adjustments")
                .resizable(true)
                .default_width(310.0)
                .width_range(250.0..=440.0)
                .show(ctx, |ui| self.right_panel(ui));

            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ctx, |ui| self.center_panel(ui));
        }

        self.handle_export(ctx);
        self.paste_all_confirm(ctx);

        if settings::show(ctx, &mut self.settings_open, &mut self.tuning) {
            if let Err(e) = self.tuning.save() {
                self.error = Some((format!("couldn't save settings: {e:#}"), Instant::now()));
            }
            self.request_render();
        }

        if self.sidecar_dirty
            || self.params != self.committed
            || self.status.is_some()
            || self.error.is_some()
            || self.batch.is_some()
        {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Some(b) = &self.batch {
            b.job.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        self.commit_and_save();
    }
}
