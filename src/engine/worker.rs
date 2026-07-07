//! The render worker thread. It owns the decoded photo (full-res + a
//! downscaled preview) so the UI thread never touches pixels. Commands
//! arrive over a channel; stale preview-render requests are coalesced so
//! only the newest slider state is rendered.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};

use crate::engine::params::EditParams;
use crate::engine::pipeline::{self, SourceImage};
use crate::imgio::export::ExportFormat;
use crate::imgio::loader;

/// Preview long edge — full slider interactivity at this size.
const PREVIEW_EDGE: usize = 1600;

pub enum Cmd {
    /// Decode a photo and render it with the given (sidecar-restored) params.
    Load { path: PathBuf, params: EditParams },
    /// Re-render the current preview with new params.
    Render(EditParams),
    /// Full-resolution render + encode of the currently loaded photo.
    Export {
        dest: PathBuf,
        params: EditParams,
        format: ExportFormat,
        jpeg_quality: u8,
    },
}

pub enum Reply {
    Loaded {
        path: PathBuf,
        full_width: usize,
        full_height: usize,
    },
    Preview {
        width: usize,
        height: usize,
        rgba: Vec<u8>,
    },
    ExportDone(PathBuf),
    Error(String),
}

pub struct Worker {
    pub tx: Sender<Cmd>,
    pub rx: Receiver<Reply>,
}

pub fn spawn(ctx: egui::Context) -> Worker {
    let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<Cmd>();
    let (reply_tx, reply_rx) = std::sync::mpsc::channel::<Reply>();
    std::thread::Builder::new()
        .name("render-worker".into())
        .spawn(move || run(cmd_rx, reply_tx, ctx))
        .expect("failed to spawn render worker");
    Worker {
        tx: cmd_tx,
        rx: reply_rx,
    }
}

fn run(rx: Receiver<Cmd>, tx: Sender<Reply>, ctx: egui::Context) {
    let mut full: Option<SourceImage> = None;
    let mut preview: Option<SourceImage> = None;

    'outer: while let Ok(first) = rx.recv() {
        // Drain whatever else is queued, then drop renders that are
        // already superseded by a later render or load.
        let mut queue: VecDeque<Cmd> = VecDeque::new();
        queue.push_back(first);
        while let Ok(next) = rx.try_recv() {
            queue.push_back(next);
        }
        while let Some(cmd) = queue.pop_front() {
            let superseded = matches!(cmd, Cmd::Render(_))
                && queue
                    .iter()
                    .any(|m| matches!(m, Cmd::Render(_) | Cmd::Load { .. }));
            if superseded {
                continue;
            }

            let send = |reply: Reply| {
                let ok = tx.send(reply).is_ok();
                ctx.request_repaint();
                ok
            };

            match cmd {
                Cmd::Load { path, params } => match loader::load(&path) {
                    Ok(src) => {
                        let pv = src.downscale(PREVIEW_EDGE);
                        let alive = send(Reply::Loaded {
                            path,
                            full_width: src.width,
                            full_height: src.height,
                        });
                        if !alive {
                            break 'outer;
                        }
                        let rgba = pipeline::render_rgba(&pv, &params);
                        let (w, h) = (pv.width, pv.height);
                        full = Some(src);
                        preview = Some(pv);
                        if !send(Reply::Preview {
                            width: w,
                            height: h,
                            rgba,
                        }) {
                            break 'outer;
                        }
                    }
                    Err(e) => {
                        if !send(Reply::Error(format!("{e:#}"))) {
                            break 'outer;
                        }
                    }
                },
                Cmd::Render(params) => {
                    if let Some(pv) = &preview {
                        let rgba = pipeline::render_rgba(pv, &params);
                        if !send(Reply::Preview {
                            width: pv.width,
                            height: pv.height,
                            rgba,
                        }) {
                            break 'outer;
                        }
                    }
                }
                Cmd::Export {
                    dest,
                    params,
                    format,
                    jpeg_quality,
                } => {
                    let reply = match &full {
                        Some(src) => {
                            match crate::imgio::export::export(
                                src,
                                &params,
                                &dest,
                                format,
                                jpeg_quality,
                            ) {
                                Ok(()) => Reply::ExportDone(dest),
                                Err(e) => Reply::Error(format!("export failed: {e:#}")),
                            }
                        }
                        None => Reply::Error("no photo loaded to export".into()),
                    };
                    if !send(reply) {
                        break 'outer;
                    }
                }
            }
        }
    }
}
