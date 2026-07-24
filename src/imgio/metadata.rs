//! Best-effort EXIF extraction for the info panel. Standard formats and
//! most TIFF-based RAWs (CR2, NEF, ARW, DNG…) are read directly with
//! kamadak-exif; anything unreadable simply yields an empty record.

use std::path::Path;

use exif::{In, Tag};

#[derive(Clone, Default)]
pub struct ExifInfo {
    pub camera: Option<String>,
    pub lens: Option<String>,
    pub focal_length: Option<String>,
    pub aperture: Option<String>,
    pub shutter: Option<String>,
    pub iso: Option<String>,
    pub date: Option<String>,
    pub dimensions: Option<String>,
}

impl ExifInfo {
    pub fn is_empty(&self) -> bool {
        self.camera.is_none()
            && self.lens.is_none()
            && self.focal_length.is_none()
            && self.aperture.is_none()
            && self.shutter.is_none()
            && self.iso.is_none()
            && self.date.is_none()
    }

    /// Rows of (label, value) for display, skipping missing fields.
    pub fn rows(&self) -> Vec<(&'static str, String)> {
        let mut rows = Vec::new();
        let mut push = |label, v: &Option<String>| {
            if let Some(val) = v {
                rows.push((label, val.clone()));
            }
        };
        push("Camera", &self.camera);
        push("Lens", &self.lens);
        push("Focal length", &self.focal_length);
        push("Aperture", &self.aperture);
        push("Shutter", &self.shutter);
        push("ISO", &self.iso);
        push("Dimensions", &self.dimensions);
        push("Date", &self.date);
        rows
    }
}

pub fn read(path: &Path) -> ExifInfo {
    let mut info = ExifInfo::default();
    let Ok(file) = std::fs::File::open(path) else {
        return info;
    };
    let mut reader = std::io::BufReader::new(&file);
    let Ok(exif) = exif::Reader::new().read_from_container(&mut reader) else {
        return info;
    };

    let get = |tag: Tag| {
        exif.get_field(tag, In::PRIMARY)
            .map(|f| f.display_value().with_unit(&exif).to_string())
    };
    let clean = |s: Option<String>| s.map(|v| v.trim_matches('"').trim().to_string());

    let make = clean(get(Tag::Make));
    let model = clean(get(Tag::Model));
    info.camera = match (make, model) {
        (Some(mk), Some(md)) if md.starts_with(&mk) => Some(md),
        (Some(mk), Some(md)) => Some(format!("{mk} {md}")),
        (None, Some(md)) => Some(md),
        (Some(mk), None) => Some(mk),
        _ => None,
    };
    info.lens = clean(get(Tag::LensModel));
    info.focal_length = clean(get(Tag::FocalLength));
    info.aperture = clean(get(Tag::FNumber)).map(|f| {
        if f.starts_with('f') {
            f
        } else {
            format!("f/{f}")
        }
    });
    info.shutter = clean(get(Tag::ExposureTime)).map(|s| format!("{s} s"));
    info.iso = clean(get(Tag::PhotographicSensitivity)).map(|s| format!("ISO {s}"));
    info.date = clean(get(Tag::DateTimeOriginal));
    info
}
