Histogram + clipping warnings — the biggest gap right now, especially with Levels; you're setting black/white points blind. RGB histogram in the right panel plus J/O-style shadow/highlight clipping overlays.
Copy/paste edits & batch export — apply one photo's look to a selection, export a whole folder. Turns it from an editor into a workflow tool.
Presets — save named looks (it's just an EditParams JSON, so this is cheap to build).
Tone curve — a proper interactive curve (levels is the 20% version; a curve with per-channel control replaces both).
Local adjustments — linear/radial gradients and a brush with the existing sliders masked to a region. This is the feature that makes Lightroom Lightroom; big but the pipeline architecture supports it.
Sharpening & noise reduction — output sharpening is nearly free (the unsharp machinery exists); luminance/color NR matters a lot for high-ISO RAW.
White-balance eyedropper — click a neutral gray to set temp/tint.
EXIF panel + star ratings/flags — camera/lens/ISO info and basic culling in the filmstrip.
Undo/redo history — currently the sidecar holds only the latest state.
GPU pipeline (wgpu) — if you ever want 100% instant full-res rendering with no region tricks, the pipeline stages would move into compute shaders. Big lift, huge payoff.