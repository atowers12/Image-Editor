use serde::{Deserialize, Serialize};

/// Names of the 8 HSL color mixer bands, Lightroom-style.
pub const HSL_BAND_NAMES: [&str; 8] = [
    "Red", "Orange", "Yellow", "Green", "Aqua", "Blue", "Purple", "Magenta",
];

/// Center hue (degrees) of each HSL band.
pub const HSL_BAND_HUES: [f32; 8] = [0.0, 30.0, 60.0, 120.0, 180.0, 240.0, 280.0, 320.0];

/// Identity crop rectangle (x, y, w, h — normalized to the image).
pub const CROP_FULL: [f32; 4] = [0.0, 0.0, 1.0, 1.0];

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

/// A single tone-curve control point (x, y both 0..1).
pub type CurvePoint = [f32; 2];

/// The identity curve: input maps to output unchanged.
pub fn identity_curve() -> Vec<CurvePoint> {
    vec![[0.0, 0.0], [1.0, 1.0]]
}

fn curve_is_identity(pts: &[CurvePoint]) -> bool {
    pts.len() == 2 && pts[0] == [0.0, 0.0] && pts[1] == [1.0, 1.0]
}

/// Point tone curve: a master curve applied to all channels plus optional
/// per-channel R/G/B curves. Each is a sorted list of control points.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ToneCurve {
    pub master: Vec<CurvePoint>,
    pub r: Vec<CurvePoint>,
    pub g: Vec<CurvePoint>,
    pub b: Vec<CurvePoint>,
}

impl Default for ToneCurve {
    fn default() -> Self {
        Self {
            master: identity_curve(),
            r: identity_curve(),
            g: identity_curve(),
            b: identity_curve(),
        }
    }
}

impl ToneCurve {
    pub fn is_identity(&self) -> bool {
        curve_is_identity(&self.master)
            && curve_is_identity(&self.r)
            && curve_is_identity(&self.g)
            && curve_is_identity(&self.b)
    }
}

/// Which tone-curve channel is being edited in the UI.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum CurveChannel {
    #[default]
    Master,
    Red,
    Green,
    Blue,
}

/// Adjustments available inside a local mask. A subset of the global set,
/// each -100..=100 (exposure in EV-ish stops scaled the same as global).
#[derive(Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LocalAdjust {
    pub exposure: f32,
    pub contrast: f32,
    pub highlights: f32,
    pub shadows: f32,
    pub whites: f32,
    pub blacks: f32,
    pub temp: f32,
    pub tint: f32,
    pub saturation: f32,
    pub clarity: f32,
    pub sharpness: f32,
}

impl LocalAdjust {
    pub fn is_zero(&self) -> bool {
        *self == Self::default()
    }
}

/// One brush stamp (dab). Position/radius normalized to the image.
#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Dab {
    pub p: [f32; 2],
    pub radius: f32,
    pub hardness: f32,
    pub erase: bool,
}

/// The geometry of a mask.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub enum MaskKind {
    /// Linear gradient: full effect past `p1`, zero before `p0`, ramped between.
    Linear { p0: [f32; 2], p1: [f32; 2] },
    /// Radial (elliptical) gradient centered at `center` with normalized
    /// half-extents `radius`; `feather` softens the edge (0..1).
    Radial {
        center: [f32; 2],
        radius: [f32; 2],
        feather: f32,
    },
    /// Freehand brush: a set of dabs rasterized into a coverage mask.
    Brush { dabs: Vec<Dab> },
}

impl MaskKind {
    pub fn type_name(&self) -> &'static str {
        match self {
            MaskKind::Linear { .. } => "Linear",
            MaskKind::Radial { .. } => "Radial",
            MaskKind::Brush { .. } => "Brush",
        }
    }
}

/// A local adjustment mask: geometry + the adjustments it applies.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Mask {
    pub name: String,
    pub kind: MaskKind,
    pub adjust: LocalAdjust,
    pub enabled: bool,
    pub inverted: bool,
}

impl Default for Mask {
    fn default() -> Self {
        Self {
            name: "Mask".into(),
            kind: MaskKind::Radial {
                center: [0.5, 0.5],
                radius: [0.3, 0.3],
                feather: 0.5,
            },
            adjust: LocalAdjust::default(),
            enabled: true,
            inverted: false,
        }
    }
}

/// Culling flag: reject / none / pick.
#[derive(Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Flag {
    Reject,
    #[default]
    None,
    Pick,
}

/// All per-photo edit values. `Default` is the identity (no edit).
/// Sliders are -100..=100 except exposure (EV), levels, and geometry.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EditParams {
    // Light
    pub exposure: f32,
    pub contrast: f32,
    pub highlights: f32,
    pub shadows: f32,
    pub whites: f32,
    pub blacks: f32,
    // Levels (input black/white, gamma, output black/white)
    pub lv_in_black: f32,
    pub lv_in_white: f32,
    pub lv_gamma: f32,
    pub lv_out_black: f32,
    pub lv_out_white: f32,
    // Tone curve
    pub curve: ToneCurve,
    // Color
    pub temp: f32,
    pub tint: f32,
    pub vibrance: f32,
    pub saturation: f32,
    // Color mixer
    pub hsl: [HslBand; 8],
    // Detail
    pub texture: f32,
    pub clarity: f32,
    pub dehaze: f32,
    pub sharpen: f32,
    pub sharpen_radius: f32,
    pub luminance_nr: f32,
    pub color_nr: f32,
    // Effects
    pub vignette: f32,
    // Local adjustments
    pub masks: Vec<Mask>,
    // Geometry
    pub rotate90: u8, // quarter turns clockwise, 0..=3
    pub flip_h: bool,
    pub flip_v: bool,
    pub angle: f32, // straighten, degrees, -45..=45
    pub crop: [f32; 4], // x, y, w, h normalized to the oriented image
    // Metadata (not a pixel edit, but persisted alongside)
    pub rating: u8, // 0..=5
    pub flag: Flag,
}

impl Default for EditParams {
    fn default() -> Self {
        Self {
            exposure: 0.0,
            contrast: 0.0,
            highlights: 0.0,
            shadows: 0.0,
            whites: 0.0,
            blacks: 0.0,
            lv_in_black: 0.0,
            lv_in_white: 1.0,
            lv_gamma: 1.0,
            lv_out_black: 0.0,
            lv_out_white: 1.0,
            curve: ToneCurve::default(),
            temp: 0.0,
            tint: 0.0,
            vibrance: 0.0,
            saturation: 0.0,
            hsl: [HslBand::default(); 8],
            texture: 0.0,
            clarity: 0.0,
            dehaze: 0.0,
            sharpen: 0.0,
            sharpen_radius: 1.0,
            luminance_nr: 0.0,
            color_nr: 0.0,
            vignette: 0.0,
            masks: Vec::new(),
            rotate90: 0,
            flip_h: false,
            flip_v: false,
            angle: 0.0,
            crop: CROP_FULL,
            rating: 0,
            flag: Flag::None,
        }
    }
}

impl EditParams {
    /// True when every value is at its default (no editing applied).
    pub fn is_identity(&self) -> bool {
        *self == Self::default()
    }

    pub fn any_hsl(&self) -> bool {
        self.hsl.iter().any(|b| !b.is_zero())
    }

    pub fn has_levels(&self) -> bool {
        self.lv_in_black != 0.0
            || self.lv_in_white != 1.0
            || self.lv_gamma != 1.0
            || self.lv_out_black != 0.0
            || self.lv_out_white != 1.0
    }

    pub fn has_curve(&self) -> bool {
        !self.curve.is_identity()
    }

    /// Active masks that actually do something.
    pub fn active_masks(&self) -> impl Iterator<Item = &Mask> {
        self.masks
            .iter()
            .filter(|m| m.enabled && !m.adjust.is_zero())
    }

    pub fn has_masks(&self) -> bool {
        self.active_masks().next().is_some()
    }

    pub fn has_crop(&self) -> bool {
        self.crop != CROP_FULL
    }

    /// Any geometry work to do (orientation, straighten, and optionally crop)?
    pub fn has_geometry(&self, include_crop: bool) -> bool {
        self.rotate90 % 4 != 0
            || self.flip_h
            || self.flip_v
            || self.angle != 0.0
            || (include_crop && self.has_crop())
    }

    /// A hashable fingerprint of the geometry fields, used by the worker to
    /// know when cached geometry-applied images are stale.
    pub fn geo_signature(&self, include_crop: bool) -> (u8, bool, bool, u32, [u32; 4]) {
        let crop = if include_crop { self.crop } else { CROP_FULL };
        (
            self.rotate90 % 4,
            self.flip_h,
            self.flip_v,
            self.angle.to_bits(),
            crop.map(f32::to_bits),
        )
    }

    /// Copy with all pixel-level edits zeroed but geometry (and metadata)
    /// kept — used for the Before/After view so framing doesn't jump.
    pub fn without_pixel_edits(&self) -> EditParams {
        EditParams {
            rotate90: self.rotate90,
            flip_h: self.flip_h,
            flip_v: self.flip_v,
            angle: self.angle,
            crop: self.crop,
            rating: self.rating,
            flag: self.flag,
            ..EditParams::default()
        }
    }

    /// Overwrite this photo's visual edits from another params set, keeping
    /// this photo's own rating and flag (used by Copy/Paste edits).
    pub fn apply_edits_from(&mut self, other: &EditParams) {
        let rating = self.rating;
        let flag = self.flag;
        *self = other.clone();
        self.rating = rating;
        self.flag = flag;
    }

    /// Reset only geometry (crop tool's Reset button).
    pub fn reset_geometry(&mut self) {
        self.rotate90 = 0;
        self.flip_h = false;
        self.flip_v = false;
        self.angle = 0.0;
        self.crop = CROP_FULL;
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
        let mut p = EditParams::default();
        p.lv_gamma = 1.2;
        assert!(!p.is_identity());
        assert!(p.has_levels());
    }

    #[test]
    fn curve_identity_detection() {
        let mut p = EditParams::default();
        assert!(!p.has_curve());
        p.curve.master.push([0.5, 0.6]);
        assert!(p.has_curve());
    }

    #[test]
    fn serde_round_trip() {
        let mut p = EditParams::default();
        p.exposure = 1.25;
        p.hsl[3].sat = -40.0;
        p.vignette = -30.0;
        p.rotate90 = 3;
        p.crop = [0.1, 0.2, 0.5, 0.6];
        p.curve.r.push([0.4, 0.5]);
        p.masks.push(Mask::default());
        p.rating = 4;
        p.flag = Flag::Pick;
        let json = serde_json::to_string(&p).unwrap();
        let back: EditParams = serde_json::from_str(&json).unwrap();
        assert!(p == back);
    }

    #[test]
    fn serde_tolerates_missing_fields() {
        // Older sidecars (fewer fields) must load with correct defaults,
        // including the non-zero ones.
        let p: EditParams = serde_json::from_str(r#"{"exposure": 0.5}"#).unwrap();
        assert_eq!(p.exposure, 0.5);
        assert_eq!(p.lv_gamma, 1.0);
        assert_eq!(p.lv_in_white, 1.0);
        assert_eq!(p.crop, CROP_FULL);
        assert_eq!(p.sharpen_radius, 1.0);
        assert!(p.curve.is_identity());
        assert!(p.masks.is_empty());
        assert_eq!(p.rating, 0);
        assert!(!p.has_levels());
    }

    #[test]
    fn without_pixel_edits_keeps_geometry_and_metadata() {
        let mut p = EditParams::default();
        p.exposure = 2.0;
        p.rotate90 = 1;
        p.crop = [0.1, 0.1, 0.8, 0.8];
        p.rating = 3;
        p.masks.push(Mask::default());
        let b = p.without_pixel_edits();
        assert_eq!(b.exposure, 0.0);
        assert_eq!(b.rotate90, 1);
        assert_eq!(b.crop, p.crop);
        assert_eq!(b.rating, 3);
        assert!(b.masks.is_empty());
    }

    #[test]
    fn apply_edits_from_keeps_rating() {
        let mut target = EditParams::default();
        target.rating = 5;
        let mut source = EditParams::default();
        source.exposure = 1.0;
        source.rating = 1;
        target.apply_edits_from(&source);
        assert_eq!(target.exposure, 1.0);
        assert_eq!(target.rating, 5); // target's own rating preserved
    }

    #[test]
    fn active_masks_filters_disabled_and_empty() {
        let mut p = EditParams::default();
        let mut m = Mask::default();
        m.adjust.exposure = 30.0;
        p.masks.push(m.clone());
        assert_eq!(p.active_masks().count(), 1);
        p.masks[0].enabled = false;
        assert_eq!(p.active_masks().count(), 0);
        p.masks[0].enabled = true;
        p.masks[0].adjust = LocalAdjust::default();
        assert_eq!(p.active_masks().count(), 0);
    }
}
