//! Background thumbnail generation for the filmstrip. A dedicated thread
//! works through requests FIFO and hands finished RGBA thumbs back to the UI.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};

use crate::imgio::loader;

pub const THUMB_EDGE: u32 = 192;

pub struct Thumb {
    pub path: PathBuf,
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

pub struct ThumbWorker {
    pub tx: Sender<PathBuf>,
    pub rx: Receiver<Thumb>,
}

pub fn spawn(ctx: egui::Context) -> ThumbWorker {
    let (req_tx, req_rx) = std::sync::mpsc::channel::<PathBuf>();
    let (out_tx, out_rx) = std::sync::mpsc::channel::<Thumb>();
    std::thread::Builder::new()
        .name("thumbnails".into())
        .spawn(move || {
            while let Ok(path) = req_rx.recv() {
                if let Some(thumb) = make_thumb(&path) {
                    if out_tx.send(thumb).is_err() {
                        break;
                    }
                    ctx.request_repaint();
                }
            }
        })
        .expect("failed to spawn thumbnail thread");
    ThumbWorker {
        tx: req_tx,
        rx: out_rx,
    }
}

fn make_thumb(path: &PathBuf) -> Option<Thumb> {
    let dyn_img = if loader::is_raw(path) {
        raw_preview(path)?
    } else {
        image::open(path).ok()?
    };
    let small = dyn_img.thumbnail(THUMB_EDGE, THUMB_EDGE);
    let rgba = small.to_rgba8();
    Some(Thumb {
        path: path.clone(),
        width: rgba.width() as usize,
        height: rgba.height() as usize,
        rgba: rgba.into_raw(),
    })
}

/// For RAW files, pull the embedded camera preview instead of a full
/// develop — orders of magnitude faster. Falls back to a full decode.
fn raw_preview(path: &PathBuf) -> Option<image::DynamicImage> {
    let source = rawler::rawsource::RawSource::new(path).ok()?;
    let decoder = rawler::get_decoder(&source).ok()?;
    let params = rawler::decoders::RawDecodeParams::default();
    if let Ok(Some(img)) = decoder.thumbnail_image(&source, &params) {
        return Some(img);
    }
    if let Ok(Some(img)) = decoder.preview_image(&source, &params) {
        return Some(img);
    }
    loader::decode_raw(path).ok()
}
