use serde::{Deserialize, Serialize};

/// Names of the 8 HSL color mixer bands, Lightroom-style.
pub const HSL_BAND_NAMES: [&str; 8] = [
    "Red", "Orange", "Yellow", "Green", "Aqua", "Blue", "Purple", "Magenta",
];

/// Center hue (degrees) of each HSL band.
pub const HSL_BAND_HUES: [f32; 8] = [0.0, 30.0, 60.0, 120.0, 180.0, 240.0, 280.0, 320.0];

/// Per-band hue/saturation/luminance adjustments, each -100..=100.
#[derive(Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct HslBand {
    pub hue: f32,
    pub sat: f32,
    pub lum: f32,
}

impl HslBand {
    pub fn is_zero(&self) -> bool {
        self.hue == 0.0 && self.sat == 0.0 && self.lum == 0.0
    }
}

/// All slider values. Everything defaults to 0.0 (identity).
/// Sliders are -100..=100 except exposure (-5..=5 EV).
#[derive(Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct EditParams {
    // Light
    pub exposure: f32,
    pub contrast: f32,
    pub highlights: f32,
    pub shadows: f32,
    pub whites: f32,
    pub blacks: f32,
    // Color
    pub temp: f32,
    pub tint: f32,
    pub vibrance: f32,
    pub saturation: f32,
    // Color mixer
    pub hsl: [HslBand; 8],
    // Effects
    pub texture: f32,
    pub clarity: f32,
    pub dehaze: f32,
    pub vignette: f32,
}

impl EditParams {
    /// True when every slider is at its default (no editing applied).
    pub fn is_identity(&self) -> bool {
        *self == Self::default()
    }

    pub fn any_hsl(&self) -> bool {
        self.hsl.iter().any(|b| !b.is_zero())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_identity() {
        assert!(EditParams::default().is_identity());
        let mut p = EditParams::default();
        p.exposure = 1.0;
        assert!(!p.is_identity());
    }

    #[test]
    fn serde_round_trip() {
        let mut p = EditParams::default();
        p.exposure = 1.25;
        p.hsl[3].sat = -40.0;
        p.vignette = -30.0;
        let json = serde_json::to_string(&p).unwrap();
        let back: EditParams = serde_json::from_str(&json).unwrap();
        assert!(p == back);
    }

    #[test]
    fn serde_tolerates_missing_fields() {
        // Older sidecars with fewer fields must still load.
        let p: EditParams = serde_json::from_str(r#"{"exposure": 0.5}"#).unwrap();
        assert_eq!(p.exposure, 0.5);
        assert_eq!(p.contrast, 0.0);
    }
}
