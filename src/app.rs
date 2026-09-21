//! Top-level application: state, layout, and the glue between the UI,
//! the render worker, thumbnails, sidecar persistence, and settings.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::engine::histogram::Histogram;
use crate::engine::ops;
use crate::engine::params::{self, CurveChannel, EditParams, Flag};
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
/// How opaque the mask-coverage wash is where a mask selects fully.
const COVERAGE_ALPHA: u8 = 130;
/// Cap on the undo history depth per photo.
const MAX_HISTORY: usize = 100;
/// How far the cull bar floats above the bottom of the preview.
const CULL_BAR_INSET: f32 = 14.0;
/// How close the pointer must get to the bottom of the preview for the cull
/// bar to come up to full opacity.
const CULL_BAR_REACH: f32 = 130.0;
/// Minimum gap between photos while an arrow key is held down.
const NAV_REPEAT_INTERVAL: Duration = Duration::from_millis(110);

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

/// What the next eyedropper click is for. Both pickers arm the same crosshair
/// over the preview; only where the sampled color lands differs.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Pick {
    None,
    /// Set global temp/tint from a pixel that should be neutral gray.
    WhiteBalance,
    /// Aim the selected mask's color range at a pixel's hue.
    MaskColor,
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
    /// The preview's pixels as well as its texture, so the brush can read the
    /// color it is painting over for auto-masking.
    preview_rgba: Option<(usize, usize, Vec<u8>)>,
    /// Red wash showing what the mask being edited selects, when enabled.
    coverage_tex: Option<egui::TextureHandle>,
    logo_tex: Option<egui::TextureHandle>,
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
    pick: Pick,
    aspect: AspectLock,
    /// Keep the crop rectangle inside the photo's real pixels. A tool
    /// preference, not part of the edit, so it sticks across photos.
    constrain_crop: bool,
    /// Move to the next photo after every rating or flag, not just the ones
    /// pressed with shift. Lightroom binds this to Caps Lock.
    auto_advance: bool,
    /// Set when the keyboard changed the selection, so the filmstrip scrolls
    /// the newly selected thumbnail into view for one frame.
    scroll_filmstrip: bool,
    /// When the selection last moved, to pace a held arrow key.
    last_nav: Instant,
    active_band: usize,
    curve_channel: CurveChannel,
    selected_mask: Option<usize>,
    /// Which shape inside the selected mask the preview edits.
    selected_component: usize,
    brush: BrushSettings,
    show_mask_overlay: bool,

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
        let logo_tex = crate::branding::icon_data(256).map(|icon| {
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [icon.width as usize, icon.height as usize],
                &icon.rgba,
            );
            cc.egui_ctx
                .load_texture("app-logo", image, egui::TextureOptions::LINEAR)
        });
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
            preview_rgba: None,
            coverage_tex: None,
            logo_tex,
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
            pick: Pick::None,
            aspect: AspectLock::Free,
            constrain_crop: true,
            auto_advance: false,
            scroll_filmstrip: false,
            last_nav: Instant::now(),
            active_band: 0,
            curve_channel: CurveChannel::Master,
            selected_mask: None,
            selected_component: 0,
            brush: BrushSettings::default(),
            show_mask_overlay: false,
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
        // The coverage wash is only meaningful while a mask is being built,
        // and only for the mask being worked on.
        let overlay = (self.mode == Mode::Mask && self.show_mask_overlay && !self.before_view)
            .then_some(self.selected_mask)
            .flatten();
        let _ = self.worker.tx.send(Cmd::Render {
            params: self.effective_params(),
            tuning: self.tuning,
            include_crop: self.mode != Mode::Crop,
            clip: self.clip_flags(),
            overlay,
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
                        self.migrate_crop_space(full_width, full_height);
                    }
                }
                Reply::Neutral { temp, tint } => {
                    self.params.temp = temp;
                    self.params.tint = tint;
                    self.pick = Pick::None;
                    self.before_view = false;
                    self.params_edited();
                }
                Reply::Preview {
                    width,
                    height,
                    rgba,
                    full_size,
                    histogram,
                    coverage,
                } => {
                    self.set_coverage_tex(ctx, width, height, coverage);
                    let img = egui::ColorImage::from_rgba_unmultiplied([width, height], &rgba);
                    match &mut self.preview_tex {
                        Some(tex) => tex.set(img, egui::TextureOptions::LINEAR),
                        None => {
                            self.preview_tex =
                                Some(ctx.load_texture("preview", img, egui::TextureOptions::LINEAR))
                        }
                    }
                    self.preview_rgba = Some((width, height, rgba));
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

    /// Turn a mask's coverage map into a translucent red wash the preview can
    /// draw over the photo, so you can see exactly what the mask selects.
    fn set_coverage_tex(
        &mut self,
        ctx: &egui::Context,
        width: usize,
        height: usize,
        coverage: Option<Vec<u8>>,
    ) {
        let Some(cov) = coverage else {
            self.coverage_tex = None;
            return;
        };
        let mut rgba = vec![0u8; width * height * 4];
        for (px, &c) in rgba.chunks_mut(4).zip(cov.iter()) {
            px[0] = 255;
            px[1] = 60;
            px[2] = 60;
            px[3] = (c as u32 * COVERAGE_ALPHA as u32 / 255) as u8;
        }
        let img = egui::ColorImage::from_rgba_unmultiplied([width, height], &rgba);
        match &mut self.coverage_tex {
            Some(tex) => tex.set(img, egui::TextureOptions::LINEAR),
            None => {
                self.coverage_tex =
                    Some(ctx.load_texture("coverage", img, egui::TextureOptions::LINEAR))
            }
        }
    }

    /// Point the selected mask's color range at the pixel the eyedropper hit,
    /// read straight out of the preview the user clicked on.
    fn aim_mask_color(&mut self, point: [f32; 2]) {
        self.pick = Pick::None;
        let Some(idx) = self.selected_mask.filter(|&i| i < self.params.masks.len()) else {
            return;
        };
        let Some((w, h, rgba)) = &self.preview_rgba else {
            return;
        };
        if *w == 0 || *h == 0 {
            return;
        }
        let x = ((point[0] * *w as f32) as usize).min(w - 1);
        let y = ((point[1] * *h as f32) as usize).min(h - 1);
        let i = (y * w + x) * 4;
        let Some(px) = rgba.get(i..i + 3) else {
            return;
        };
        let (hue, sat, _) = ops::color::rgb_to_hsl(
            px[0] as f32 / 255.0,
            px[1] as f32 / 255.0,
            px[2] as f32 / 255.0,
        );
        masks::aim_color_range(&mut self.params.masks[idx].range, hue, sat);
        self.params_edited();
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

    /// Bring a sidecar written before straightening grew the canvas up to
    /// date. Straightening used to rotate inside the source size, so a saved
    /// crop was a fraction of that; it is now a fraction of the rotated
    /// bounding box. Same pixels, new denominator — remap once, on load,
    /// when the photo's true dimensions first arrive.
    fn migrate_crop_space(&mut self, full_width: usize, full_height: usize) {
        if self.params.crop_space >= params::CROP_SPACE_GROWN {
            return;
        }
        self.params.crop_space = params::CROP_SPACE_GROWN;
        let remap = self.params.angle != 0.0 && self.params.has_crop();
        if remap {
            let src = ops::geometry::source_dims(full_width, full_height, &self.params);
            self.params.crop = ops::geometry::migrate_crop_to_grown_canvas(
                self.params.crop,
                self.params.angle,
                src,
            );
        }
        // Sync either way so the version stamp alone never registers as an
        // edit — that would push an undo step and rewrite every old sidecar
        // just for opening the photo. Only a real remap is worth persisting.
        self.committed = self.params.clone();
        if remap {
            self.sidecar_dirty = true;
            self.request_render();
        }
    }

    /// The frame the straighten angle rotates — full dimensions after the 90°
    /// orientation step. This is what the crop constraint tests against.
    /// `None` until the photo's real dimensions arrive from the worker, so
    /// the constraint is never computed against a guessed frame.
    fn crop_source_dims(&self) -> Option<(usize, usize)> {
        self.full_size
            .map(|(w, h)| ops::geometry::source_dims(w, h, &self.params))
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
        self.pick = Pick::None;
        self.selected_mask = None;
        self.selected_component = 0;
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

    /// Culling keystrokes, following Lightroom's defaults: arrows move through
    /// the folder, `0`-`5` rate, `[` / `]` nudge the rating, `P` / `X` / `U`
    /// flag, and holding shift advances to the next photo afterwards.
    fn keyboard(&mut self, ctx: &egui::Context) {
        if ctx.wants_keyboard_input() {
            return; // don't steal keys while typing in a text field
        }
        let mut undo = false;
        let mut redo = false;
        let mut nav = 0isize;
        let mut nav_held = false;
        let mut rating = None;
        let mut bump = 0i8;
        let mut flag = None;
        let mut advance = false;

        ctx.input(|i| {
            for ev in &i.events {
                let egui::Event::Key {
                    key,
                    physical_key,
                    pressed: true,
                    repeat,
                    modifiers,
                } = ev
                else {
                    continue;
                };
                // Match the logical key *or* the physical one. Shift+1 reports
                // logically as "!", so the digit only survives as the physical
                // key; letters keep working on non-US layouts via the logical.
                let is = |want: egui::Key| *key == want || *physical_key == Some(want);

                if modifiers.command {
                    if is(egui::Key::Z) && !modifiers.shift {
                        undo = true;
                    }
                    if is(egui::Key::Y) || (is(egui::Key::Z) && modifiers.shift) {
                        redo = true;
                    }
                    continue; // Ctrl-combos are never culling keys.
                }

                // Held arrows should keep moving; a held rating key should not
                // machine-gun through the folder on auto-advance.
                if is(egui::Key::ArrowUp) || is(egui::Key::ArrowLeft) {
                    nav = -1;
                    nav_held |= *repeat;
                } else if is(egui::Key::ArrowDown) || is(egui::Key::ArrowRight) {
                    nav = 1;
                    nav_held |= *repeat;
                }
                if *repeat {
                    continue;
                }

                for (k, n) in [
                    (egui::Key::Num0, 0u8),
                    (egui::Key::Num1, 1),
                    (egui::Key::Num2, 2),
                    (egui::Key::Num3, 3),
                    (egui::Key::Num4, 4),
                    (egui::Key::Num5, 5),
                ] {
                    if is(k) {
                        rating = Some(n);
                        advance |= modifiers.shift;
                    }
                }
                if is(egui::Key::OpenBracket) {
                    bump = -1;
                } else if is(egui::Key::CloseBracket) {
                    bump = 1;
                }
                for (k, f) in [
                    (egui::Key::P, Flag::Pick),
                    (egui::Key::X, Flag::Reject),
                    (egui::Key::U, Flag::None),
                ] {
                    if is(k) {
                        flag = Some(f);
                        advance |= modifiers.shift;
                    }
                }
            }
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
        if nav != 0 {
            // A held arrow walks the folder at a steady pace rather than the
            // OS repeat rate — every step is a fresh decode, and on RAW files
            // an unthrottled sweep buries the worker.
            if !nav_held || self.last_nav.elapsed() >= NAV_REPEAT_INTERVAL {
                self.last_nav = Instant::now();
                self.select_relative(nav);
            }
            return; // The keys below belong to the photo we just left.
        }

        let mut rated = false;
        if let Some(n) = rating {
            self.set_rating(n);
            rated = true;
        }
        if bump != 0 {
            let next = (self.params.rating as i8 + bump).clamp(0, 5) as u8;
            self.set_rating(next);
        }
        if let Some(f) = flag {
            // Pick and reject toggle, so the key that set a flag also clears
            // it; `U` always means "no flag".
            self.params.flag = if f != Flag::None && self.params.flag == f {
                Flag::None
            } else {
                f
            };
            self.params_edited();
            rated = true;
        }
        if rated && (advance || self.auto_advance) {
            self.select_relative(1);
        }
    }

    fn set_rating(&mut self, stars: u8) {
        if self.params.rating != stars {
            self.params.rating = stars;
            self.params_edited();
        }
    }

    /// Step through the folder for culling. Stops at either end rather than
    /// wrapping, so holding an arrow doesn't loop back around.
    fn select_relative(&mut self, delta: isize) {
        let Some(current) = self.selected else { return };
        if self.files.is_empty() {
            return;
        }
        let last = self.files.len() as isize - 1;
        let next = (current as isize + delta).clamp(0, last) as usize;
        if next != current {
            self.select_photo(next);
            // Keep the keyboard-driven selection in view.
            self.scroll_filmstrip = true;
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
            self.mode_toggle(ui, has_photo, Mode::Mask, "◎ Mask");

            // Undo / redo.
            if ui
                .add_enabled(!self.undo.is_empty(), egui::Button::new("↩"))
                .on_hover_text("Undo (Ctrl+Z)")
                .clicked()
            {
                self.undo();
            }
            if ui
                .add_enabled(!self.redo.is_empty(), egui::Button::new("↪"))
                .on_hover_text("Redo (Ctrl+Y)")
                .clicked()
            {
                self.redo();
            }

            let mut before = self.before_view;
            if ui
                .add_enabled(
                    has_photo && self.mode == Mode::Adjust,
                    egui::SelectableLabel::new(before, "◑ Before"),
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
                .add_enabled(has_photo, egui::Button::new("🗐 Copy"))
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
                .add_enabled(
                    can_paste && self.files.len() > 1,
                    egui::Button::new("📋 All"),
                )
                .clicked()
            {
                self.confirm_paste_all = true;
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .button("⚙")
                    .on_hover_text("Processing settings")
                    .clicked()
                {
                    self.settings_open = !self.settings_open;
                }
                if let Some(b) = &self.batch {
                    let frac = (b.done as f32 + 0.5) / b.job.total.max(1) as f32;
                    if ui
                        .button("✖")
                        .on_hover_text("Cancel batch export")
                        .clicked()
                    {
                        b.job
                            .cancel
                            .store(true, std::sync::atomic::Ordering::Relaxed);
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
            self.pick = Pick::None;
            if self.mode == Mode::Mask && self.selected_mask.is_none() {
                self.selected_mask = (!self.params.masks.is_empty()).then_some(0);
                self.selected_component = 0;
            }
            // Open the crop tool on the whole photo — arriving zoomed in
            // would leave the frame's handles off screen. It pans and zooms
            // freely from there.
            if self.mode == Mode::Crop {
                self.preview_state.fit = true;
                self.preview_state.offset = egui::Vec2::ZERO;
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
                    if ui
                        .add_enabled(has_photo, egui::Button::new(name).frame(false))
                        .clicked()
                    {
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
                    .add_enabled(
                        has_photo && !self.new_preset_name.trim().is_empty(),
                        egui::Button::new("Save"),
                    )
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
                let source_dims = self.crop_source_dims();
                match crop::panel(
                    ui,
                    &mut self.params,
                    &mut self.aspect,
                    &mut self.constrain_crop,
                    dims,
                    source_dims,
                ) {
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
                &mut self.selected_component,
                &mut self.brush,
                &mut self.show_mask_overlay,
                self.pick == Pick::MaskColor,
            ) {
                masks::MaskAction::Changed => self.params_edited(),
                masks::MaskAction::ViewChanged => self.request_render(),
                masks::MaskAction::PickColor => {
                    self.pick = if self.pick == Pick::MaskColor {
                        Pick::None
                    } else {
                        Pick::MaskColor
                    };
                }
                masks::MaskAction::Done => {
                    self.mode = Mode::Adjust;
                    self.pick = Pick::None;
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
                    self.pick == Pick::WhiteBalance,
                );
                if out.eyedropper_toggled {
                    self.pick = if self.pick == Pick::WhiteBalance {
                        Pick::None
                    } else {
                        Pick::WhiteBalance
                    };
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
        // Captured before the preview claims the space, so the cull bar can
        // be placed over the bottom of the photo.
        let preview_rect = ui.available_rect_before_wrap();
        let dims = self.oriented_dims.unwrap_or((1, 1));
        let source_dims = self.crop_source_dims();

        // Build at most one editing overlay, borrowing the relevant params.
        let crop_overlay = if self.mode == Mode::Crop {
            Some(preview::CropOverlay {
                crop: &mut self.params.crop,
                aspect: self.aspect.ratio(dims),
                dims,
                angle: self.params.angle,
                source_dims: source_dims.unwrap_or(dims),
                // Never constrain against a frame we had to guess.
                constrain: self.constrain_crop && source_dims.is_some(),
            })
        } else {
            None
        };
        let mask_editor = if self.mode == Mode::Mask {
            let brush = self.brush;
            let selected_component = self.selected_component;
            let pixels = self
                .preview_rgba
                .as_ref()
                .map(|(w, h, rgba)| preview::PreviewPixels {
                    width: *w,
                    height: *h,
                    rgba,
                });
            self.selected_mask
                .filter(|&i| i < self.params.masks.len())
                .map(|i| preview::MaskEditor {
                    components: &mut self.params.masks[i].components,
                    selected: selected_component,
                    brush,
                    preview: pixels,
                })
        } else {
            None
        };

        // The wash only makes sense over the mask it was rendered for.
        let coverage = (self.mode == Mode::Mask)
            .then_some(self.coverage_tex.as_ref())
            .flatten();

        let out = preview::show(
            ui,
            &mut self.preview_state,
            self.preview_tex.as_ref(),
            self.loading,
            self.oriented_dims,
            region,
            crop_overlay,
            mask_editor,
            coverage,
            self.pick != Pick::None,
        );

        if out.crop_changed {
            self.crop_rect_edited();
        }
        if out.mask_changed {
            self.params_edited();
        }
        if let Some(point) = out.eyedrop_point {
            match self.pick {
                // White balance solves against the undeveloped source, so it
                // has to go through the worker, which owns those pixels.
                Pick::WhiteBalance => {
                    let _ = self.worker.tx.send(Cmd::SampleNeutral {
                        params: self.params.clone(),
                        norm_point: point,
                    });
                }
                // A range mask matches the developed pixels, which are
                // exactly what the preview is already showing.
                Pick::MaskColor => self.aim_mask_color(point),
                Pick::None => {}
            }
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

        self.cull_bar(ui, preview_rect);
    }

    /// The floating rating/flag/navigation strip over the bottom of the photo.
    /// Adjust mode only — in Crop and Mask it would sit on top of the very
    /// handles you are trying to drag.
    fn cull_bar(&mut self, ui: &mut egui::Ui, preview_rect: egui::Rect) {
        if self.mode != Mode::Adjust || self.current_path().is_none() {
            return;
        }
        let anchor = egui::pos2(preview_rect.center().x, preview_rect.max.y - CULL_BAR_INSET);
        // Fade out while the pointer is up in the photo, so the bar isn't
        // permanently competing with the image for attention.
        let near = ui.ctx().pointer_latest_pos().is_some_and(|p| {
            p.y > preview_rect.max.y - CULL_BAR_REACH && preview_rect.x_range().contains(p.x)
        });
        let opacity = if near { 1.0 } else { 0.35 };

        let (at_start, at_end) = match self.selected {
            Some(i) => (i == 0, i + 1 >= self.files.len()),
            None => (true, true),
        };
        let mut action = info::CullAction::default();
        egui::Area::new(egui::Id::new("cull_bar"))
            .order(egui::Order::Foreground)
            .fixed_pos(anchor)
            .pivot(egui::Align2::CENTER_BOTTOM)
            .constrain_to(preview_rect)
            .show(ui.ctx(), |ui| {
                ui.set_opacity(opacity);
                egui::Frame::popup(ui.style())
                    .corner_radius(8.0)
                    .show(ui, |ui| {
                        action = info::cull_bar(
                            ui,
                            &mut self.params,
                            &mut self.auto_advance,
                            at_start,
                            at_end,
                        );
                    });
            });

        if action.changed {
            self.params_edited();
            if self.auto_advance {
                action.step = 1;
            }
        }
        if action.step != 0 {
            self.select_relative(action.step);
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
                match welcome::show(ui, &self.recent, self.logo_tex.as_ref()) {
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
                    let scroll = std::mem::take(&mut self.scroll_filmstrip);
                    if let Some(i) = filmstrip::show(
                        ui,
                        &self.files,
                        self.selected,
                        &self.thumb_tex,
                        &self.meta_cache,
                        scroll,
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
            b.job
                .cancel
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
        self.commit_and_save();
    }
}
