//! User-configurable processing constants ("engine tuning"). These control
//! how strong each effect is at full slider, blur radii for the local-contrast
//! effects, vignette shape, and preview resolution. Persisted globally (not
//! per photo) in %APPDATA%\photo-editor\settings.json.

use std::path::PathBuf;

use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Tuning {
    /// Highlights/shadows strength at full slider (whites/blacks use 5/6 of it).
    pub tone_range_strength: f32,
    /// Texture effect strength at full slider.
    pub texture_strength: f32,
    /// Texture blur radius as a fraction of the image's long edge.
    pub texture_radius: f32,
    /// Clarity effect strength at full slider.
    pub clarity_strength: f32,
    /// Clarity blur radius as a fraction of the image's long edge.
    pub clarity_radius: f32,
    /// Dehaze veil (black point shift) at full slider.
    pub dehaze_strength: f32,
    /// Saturation compensation paired with dehaze.
    pub dehaze_sat: f32,
    /// Vignette darkening/brightening at full slider.
    pub vignette_strength: f32,
    /// Where the vignette starts, 0 (center) .. 1 (corners).
    pub vignette_midpoint: f32,
    /// How soft the vignette transition is.
    pub vignette_feather: f32,
    /// Long edge of the interactive preview, in pixels. Larger = sharper
    /// fit-to-window view but slower slider response. Applied when a photo
    /// is (re)loaded.
    pub preview_edge: u32,
}

impl Default for Tuning {
    fn default() -> Self {
        Self {
            tone_range_strength: 0.30,
            texture_strength: 0.55,
            texture_radius: 0.0015,
            clarity_strength: 0.45,
            clarity_radius: 0.010,
            dehaze_strength: 0.30,
            dehaze_sat: 0.20,
            vignette_strength: 0.85,
            vignette_midpoint: 0.35,
            vignette_feather: 0.65,
            preview_edge: 1600,
        }
    }
}

impl Tuning {
    pub fn settings_path() -> Option<PathBuf> {
        std::env::var_os("APPDATA")
            .map(|appdata| PathBuf::from(appdata).join("photo-editor").join("settings.json"))
    }

    /// Load saved settings; any missing/corrupt file falls back to defaults.
    pub fn load() -> Tuning {
        Self::settings_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<()> {
        let Some(path) = Self::settings_path() else {
            anyhow::bail!("APPDATA not set; cannot persist settings");
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_round_trip_and_missing_fields() {
        let t = Tuning::default();
        let json = serde_json::to_string(&t).unwrap();
        let back: Tuning = serde_json::from_str(&json).unwrap();
        assert!(t == back);

        // Partial file (older version) fills in defaults.
        let partial: Tuning = serde_json::from_str(r#"{"dehaze_strength": 0.5}"#).unwrap();
        assert_eq!(partial.dehaze_strength, 0.5);
        assert_eq!(partial.preview_edge, 1600);
    }
}
