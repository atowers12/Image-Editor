# TODO / Roadmap

Status of the original "future adds" list. Percentages are judgement calls about
how much of the *feature as a user would expect it* exists, not how much code is
written.

**Overall: 9 of 10 shipped — roughly 92% of the roadmap, with the GPU rewrite the
only untouched item.**

| # | Feature | Done | Status |
|---|---|---|---|
| 1 | [Histogram + clipping warnings](#1-histogram--clipping-warnings) | 100% | ✅ Shipped |
| 2 | [Copy/paste edits & batch export](#2-copypaste-edits--batch-export) | 100% | ✅ Shipped |
| 3 | [Presets](#3-presets) | 100% | ✅ Shipped |
| 4 | [Tone curve](#4-tone-curve) | 100% | ✅ Shipped |
| 5 | [Local adjustments](#5-local-adjustments) | 100% | ✅ Shipped |
| 6 | [Sharpening & noise reduction](#6-sharpening--noise-reduction) | 85% | ✅ Shipped, refinements open |
| 7 | [White-balance eyedropper](#7-white-balance-eyedropper) | 100% | ✅ Shipped |
| 8 | [EXIF panel + ratings/flags](#8-exif-panel--star-ratingsflags) | 85% | ✅ Shipped, culling workflow open |
| 9 | [Undo/redo history](#9-undoredo-history) | 90% | ✅ Shipped, session-only |
| 10 | [GPU pipeline (wgpu)](#10-gpu-pipeline-wgpu) | 0% | ⬜ Not started |

---

## 1. Histogram + clipping warnings

**100% — done.**

Live RGB histogram sits above the adjustment panel, computed in
[histogram.rs](src/engine/histogram.rs) and drawn by
[ui/histogram.rs](src/ui/histogram.rs). The corner dots toggle shadow (blue) and
highlight (red) clipping overlays on the preview, so Levels is no longer set
blind.

Nothing outstanding.

## 2. Copy/paste edits & batch export

**100% — done.**

Copy one photo's `EditParams`, paste onto another, or apply to every photo in the
folder (with a confirmation step). Batch export lives in
[imgio/export.rs](src/imgio/export.rs) and runs on its own thread with a progress
bar and cancel, so editing stays responsive. Covered end to end by
`batch_export_whole_folder` in [tests/end_to_end.rs](tests/end_to_end.rs).

Nothing outstanding.

## 3. Presets

**100% — done.**

Named looks saved as `EditParams` JSON under `%APPDATA%\photo-editor\presets`,
handled by [imgio/presets.rs](src/imgio/presets.rs).

Possible polish, none of it required:

- [ ] Partial presets (apply only Color, only Detail, …) — currently a preset is
      all-or-nothing.
- [ ] Preset groups/folders and reordering.

## 4. Tone curve

**100% — done.**

Interactive master + per-channel R/G/B curves with a monotone spline
([ops/curve.rs](src/engine/ops/curve.rs)) baked to 256-entry LUTs, and a
drag-to-edit widget in [ui/curve.rs](src/ui/curve.rs) — drag to add or move a
point, right-click to remove. Levels was kept alongside it rather than replaced.

Nothing outstanding.

## 5. Local adjustments

**100% — done.**

A mask is a list of shapes plus a content-based refinement, all folded into one
0..1 weight per pixel in [ops/mask.rs](src/engine/ops/mask.rs) and applied by
[ops/local.rs](src/engine/ops/local.rs).

- [x] **Shapes** — linear gradient, radial gradient, freehand brush. Linear and
      radial are evaluated analytically from normalized coordinates, so they
      cost nothing to store and are resolution-independent; brush dabs are
      stored in normalized image space and rasterized into whatever buffer is
      being rendered. All are dragged (or painted) directly in the preview.
- [x] **Mask composition** — a mask holds any number of shapes, folded in order
      with add (`w + c − wc`), subtract (`w(1 − c)`) or intersect (`wc`). The
      first shape sets the base. Shapes and whole masks invert independently.
      Each shape is outlined in the preview in a color keyed to its operator.
- [x] **Range masks** — a luminance band and a hue target (with feathering, a
      saturation gate, and a color picker that reads the pixel you click).
      A mask with *no* shapes is legal and selects the whole frame, so a range
      alone can drive it — that is how you reach "every green pixel".
- [x] **More local sliders** — the set is now exposure, contrast, highlights,
      shadows, whites, blacks, temp, tint, vibrance, saturation, texture,
      clarity, dehaze, sharpness, and noise reduction. Texture, clarity and
      sharpness each get their own blur radius; noise blends toward a
      full-strength bilateral denoise of the buffer.
- [x] **Brush auto-mask** — a dab captures the color under the cursor when it is
      painted and only covers pixels close to it (luma-weighted, so it stops at
      brightness edges as readily as hue ones). Storing the reference at paint
      time rather than sampling it at render time is what makes a stroke resolve
      identically in the preview, in a zoomed region, and in the export.
- [x] **Show mask coverage** — washes exactly what the mask selects in red over
      the photo. The coverage travels from the worker as its own alpha map
      rather than being baked into the pixels, so the preview the brush samples
      for auto-masking stays the real render.

Sidecars written before composition existed still load: a mask that stored a
single `kind` becomes a one-component mask (see `MaskRepr` in
[params.rs](src/engine/params.rs)).

Nothing outstanding. Further ideas, none of them required:

- [ ] Subject / sky detection to seed a mask automatically (needs a model).
- [ ] Reorder shapes within a mask by dragging.
- [ ] Duplicate a mask, or copy one between photos on its own.
- [ ] The coverage overlay re-rasterizes brush dabs that the render itself has
      already rasterized, so painting with it on costs roughly double. Worth
      threading the render's coverage buffers out if it ever feels slow.

## 6. Sharpening & noise reduction

**85% — shipped; missing the finesse controls.**

In [ops/sharpen.rs](src/engine/ops/sharpen.rs), all operating on the
gamma-encoded buffer:

- Sharpening: unsharp mask on luminance, amount + radius, reusing the existing
  blur machinery as predicted.
- Luminance NR: a small bilateral filter on luma — smooths noise while ignoring
  neighbours across a large luma jump, so edges survive.
- Color NR: blurs chroma while leaving luminance crisp, which is what kills
  high-ISO colour speckle without softening detail.

Outstanding:

- [ ] **Sharpening detail + masking sliders** — currently every pixel gets the
      same sharpening; a masking slider (sharpen edges, skip flat sky) is the
      single biggest quality win left here.
- [ ] **NR detail/contrast sliders** to trade smoothing against texture
      retention.
- [ ] Separate capture vs. output sharpening (output sharpening tuned to export
      size).

## 7. White-balance eyedropper

**100% — done.**

**💧 WB picker** in the Color group; click a neutral gray in the photo and the
solve in [ops/color.rs](src/engine/ops/color.rs) back-computes temp/tint. Picker
state is cleared on photo switch and reset.

Nothing outstanding.

## 8. EXIF panel + star ratings/flags

**85% — the display is done; the culling *workflow* is thin.**

EXIF is read best-effort with kamadak-exif in
[imgio/metadata.rs](src/imgio/metadata.rs) — camera, lens, focal length,
aperture, shutter, ISO, dimensions, date — and shown read-only in
[ui/info.rs](src/ui/info.rs), skipping fields that are missing. Ratings (0–5)
and pick/reject flags are bound to `0`–`5`, `P`, `X`, persisted in the sidecar,
cached per file in `meta_cache`, and drawn as badges in the filmstrip.

Outstanding:

- [ ] **Filter the filmstrip by rating/flag** ("show 3★ and up", "hide
      rejects"). The badges exist but you still scroll past everything — this is
      what actually makes culling fast.
- [ ] Sort by rating or capture date.
- [ ] Colour labels.
- [ ] A "delete/move rejects" action.

## 9. Undo/redo history

**90% — done, but session-scoped.**

Per-photo undo and redo stacks in [app.rs](src/app.rs), capped at
`MAX_HISTORY = 100` steps, driven by `Ctrl+Z` / `Ctrl+Y` and the ↶ ↷ buttons.
Steps are committed on the same 700 ms debounce that writes the sidecar, so a
slider drag collapses into one undo step rather than hundreds. A pending edit is
flushed into history before an undo so nothing is lost mid-drag.

Outstanding:

- [ ] **History does not survive a photo switch or app restart** — the stacks are
      cleared in `select_photo`, and the sidecar still holds only the latest
      state. Persisting history would mean a new sidecar shape (a step list plus
      a cursor).
- [ ] No visible history *panel* — you can't jump back N steps or see what each
      step changed, only step one at a time.

## 10. GPU pipeline (wgpu)

**0% — not started.** No `wgpu` dependency; every stage is CPU + rayon.

Still the biggest lift and the biggest payoff. The current design already hides
most of the latency — the worker thread owns the pixels, stale requests are
coalesced, and full-res zoom renders only the visible rectangle at screen pixel
density — so this buys *fully* instant full-res rendering with no region tricks,
not a fix for something broken.

Sketch, if it's ever picked up:

- [ ] Move `SourceImage` to a GPU texture; keep the CPU path for export and as a
      fallback.
- [ ] Port the pipeline stages to compute shaders in dependency order: white
      balance/exposure → tone → curve LUTs → detail → mixer → local → vignette.
      The LUT-based stages (curve, mixer) port almost mechanically.
- [ ] Keep `RenderCtx` semantics intact so position-dependent ops (vignette,
      masks) and radius scaling stay correct at any region/resolution.
- [ ] Blur is the interesting one — the 3× box approximation in
      [blur.rs](src/engine/blur.rs) maps well to a separable compute pass.
- [ ] Decide whether export renders on GPU (fast, needs exact-match validation
      against the CPU path) or stays CPU (safe, and export isn't latency-bound).

---

## Not on the original list

Ideas that surfaced while building the above. None are committed to.

- [ ] Lens corrections (distortion / vignetting / CA) from EXIF lens profiles.
- [ ] Perspective / upright transform.
- [ ] Spot healing and clone.
- [ ] Black & white conversion with per-channel mixing.
- [ ] Split toning / colour grading wheels.
- [ ] XMP sidecars for interop with Lightroom, instead of the bespoke JSON.
- [ ] Soft proofing and colour management (ICC output profiles).
