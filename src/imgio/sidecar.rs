//! Non-destructive edit persistence: each photo's slider values live in a
//! JSON sidecar next to the original (photo.jpg -> photo.jpg.edits.json).

use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::engine::params::EditParams;

pub fn sidecar_path(image_path: &Path) -> PathBuf {
    let mut name = image_path.file_name().unwrap_or_default().to_os_string();
    name.push(".edits.json");
    image_path.with_file_name(name)
}

/// Load saved edits for a photo, if any. Unreadable/corrupt sidecars are
/// treated as absent rather than erroring.
pub fn load(image_path: &Path) -> Option<EditParams> {
    let text = std::fs::read_to_string(sidecar_path(image_path)).ok()?;
    serde_json::from_str(&text).ok()
}

/// Persist edits. Identity params delete the sidecar instead of writing it,
/// so untouched photos leave no clutter behind.
pub fn save(image_path: &Path, params: &EditParams) -> Result<()> {
    let path = sidecar_path(image_path);
    if params.is_identity() {
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        return Ok(());
    }
    let json = serde_json::to_string_pretty(params)?;
    std::fs::write(&path, json)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidecar_path_appends_suffix() {
        let p = sidecar_path(Path::new(r"C:\photos\img_001.CR3"));
        assert_eq!(p, PathBuf::from(r"C:\photos\img_001.CR3.edits.json"));
    }

    #[test]
    fn save_load_round_trip() {
        let dir = std::env::temp_dir().join("photo-editor-test-sidecar");
        std::fs::create_dir_all(&dir).unwrap();
        let img = dir.join("fake.jpg");

        let mut p = EditParams::default();
        p.exposure = 0.7;
        p.hsl[5].hue = 22.0;
        save(&img, &p).unwrap();
        let loaded = load(&img).unwrap();
        assert!(loaded == p);

        // Identity save removes the sidecar.
        save(&img, &EditParams::default()).unwrap();
        assert!(load(&img).is_none());
        std::fs::remove_dir_all(&dir).ok();
    }
}
