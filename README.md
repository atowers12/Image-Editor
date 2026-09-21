# Photo Editor

<p align="center">
  <img src="LOGO%202.jpg" alt="Photo Editor icon" width="160" height="160">
</p>

A light, portable, photo editor written in Rust, using [egui/eframe](https://github.com/emilk/egui)
for the interface. This project is made to be extended, full source code is available here. If you have any additions you would like to see - fork the repo and add them! Non-destructive by design: your original files are never modified.

## Features

- **Browse**: open a folder (or a single file) and flip through photos in a filmstrip
  with background-generated thumbnails
- **Formats**: JPEG, PNG, TIFF, WebP, BMP, and camera RAW — CR2, CR3, NEF, NRW, ARW,
  DNG, RAF, ORF, RW2, PEF, SRW, ERF, KDC, DCR, 3FR, IIQ (decoded with
  [rawler](https://crates.io/crates/rawler): demosaic, camera white balance, and
  color calibration)
- **Adjustments**:
  - *Light*: exposure (±5 EV), contrast, highlights, shadows, whites, blacks
  - *Tone Curve*: interactive master + per-channel (R/G/B) curves with a
    monotone spline — drag to add/move points, right-click to remove
  - *Levels*: input black/white points, midtone gamma, output black/white
  - *Color*: temperature, tint, vibrance (with skin-tone protection), saturation,
    and a white-balance eyedropper (click a neutral gray to set temp/tint)
  - *Color Mixer*: hue / saturation / luminance per color, across 8 bands
    (red, orange, yellow, green, aqua, blue, purple, magenta)
  - *Detail*: texture, clarity, sharpening (amount + radius), luminance noise
    reduction (bilateral), color noise reduction
  - *Effects*: dehaze, vignette
- **Local adjustments (masking)**: a mask is any number of shapes — linear gradient,
  radial gradient, freehand brush — composed with **add / subtract / intersect**, so
  you can brighten a sky with a gradient and carve the mountains back out of it.
  Drag a shape (or paint) directly in the preview; shapes and whole masks can be
  inverted, and masks stack.
  - *Range masks* narrow a mask by pixel content — a luminance band, a target hue
    (with a color picker), or both. A mask needs no shape at all if a range defines
    it, which is how you reach "every green pixel in the frame".
  - *Auto mask* makes a brush stroke remember the color under the cursor and paint
    only matching pixels, so it stops at edges instead of spilling over them.
  - *Show mask coverage* washes exactly what the mask selects in red over the photo.
  - Each mask carries exposure, contrast, highlights, shadows, whites, blacks, temp,
    tint, vibrance, saturation, texture, clarity, dehaze, sharpness, and noise
    reduction.
- **Presets**: save the current look as a named preset and apply it to any photo
  (stored as JSON in `%APPDATA%\photo-editor\presets`)
- **Ratings & flags**: 0–5 stars and pick/reject flags (keys `0`–`5`, `P`, `X`),
  shown as badges in the filmstrip for culling
- **EXIF panel**: camera, lens, focal length, aperture, shutter, ISO, date
- **Culling**: Lightroom's keys — arrows to move through the folder, `0`–`5` to rate,
  `[` / `]` to nudge, `P` / `X` / `U` to flag, and `Shift` to apply-and-advance (or
  latch it with auto advance). A cull bar floats over the photo with the same
  controls, and ratings show as filmstrip badges
- **Undo / redo**: full per-photo history (`Ctrl+Z` / `Ctrl+Y`)
- **Crop & Rotate** (non-destructive): interactive crop with aspect-ratio locks
  (original, 1:1, 3:2, 4:3, 16:9, …), 90° rotation, horizontal/vertical flip,
  and a ±45° straighten slider with rule-of-thirds grid. Straightening grows the
  canvas to the rotated bounding box, so it never clips the corners off your photo.
  The view zooms and pans while you crop; **Constrain to image** keeps the crop
  frame on real pixels so a straightened photo never exports black corners (the
  boundary is drawn, so you can see where it stops); and **Fill frame** grows the
  crop to the largest one of its shape that fits
- **Full-resolution zoom**: zooming past the preview's resolution renders the
  visible area from the original pixels in the background, so you always end up
  pixel-sharp — panning stays fluid because the soft preview shows instantly
  underneath while the sharp tile catches up
- **Before/After** toggle (geometry kept, pixel edits removed)
- **Histogram & clipping warnings**: live RGB histogram above the adjustment panel;
  the corner dots toggle shadow (blue) and highlight (red) clipping overlays on the
  preview — indispensable when setting Levels
- **Copy / Paste edits**: copy one photo's settings, paste onto another, or apply
  to every photo in the folder at once (with confirmation)
- **Batch export**: export the whole folder in one go, each photo with its own
  edits, with a progress bar and cancel — runs on its own thread so editing stays
  responsive
- **Welcome screen**: open-folder/open-file shortcuts and recent folders on launch
- **Configurable processing** (⚙): the strength and radius of dehaze, texture,
  clarity, tone ranges, and the vignette's shape (midpoint/feather/strength),
  plus preview resolution — saved globally to `%APPDATA%\photo-editor\settings.json`
- **Non-destructive editing**: slider values are saved to a small JSON sidecar next to
  the original (`photo.jpg` → `photo.jpg.edits.json`). Reopening a photo restores its
  edits; resetting everything removes the sidecar. Sidecar writes are debounced and
  also flushed on photo switch and app close.
- **Export**: bakes the edits into a new full-resolution file — JPEG (adjustable
  quality), PNG, or TIFF

See [TODO.md](TODO.md) for what's shipped from the roadmap and what's still open.

## Building and running

Requires Rust (via [rustup](https://rustup.rs)) and, on Windows, the MSVC build tools.

```
cargo run --release
```

Debug builds also work (`cargo run`) — the dev profile is configured with
optimizations because pixel processing is far too slow without them.

## Usage

| Action | How |
|---|---|
| Open photos | **📂 Open Folder…** or **🖼 Open File…** in the top bar |
| Switch photo | Click a thumbnail in the left filmstrip |
| Adjust | Drag sliders in the right panel — the preview updates live |
| Reset one slider | Double-click it (or right-click → Reset) |
| Reset everything | **↺ Reset All** in the top bar |
| Crop / rotate | **✂ Crop**, then drag the frame's corners/edges; rotate, flip, straighten, and lock aspect in the panel; **✔ Done** to apply |
| Move the crop frame | Drag inside it |
| Pan / zoom while cropping | Drag *outside* the frame, or middle-drag anywhere; wheel or pinch to zoom; double-click outside the frame to fit |
| Keep the crop on real pixels | **Constrain to image** in the crop panel (on by default) — the frame stops at the photo's straightened edge, drawn as an amber outline, and an out-of-bounds crop is pulled back in, keeping its aspect ratio. Untick to place it freely over the black corners |
| Use as much photo as possible | **⛶ Fill frame** — grows the crop to the largest one of its current shape (or locked aspect) that fits the straightened photo, centred |
| Compare | **◑ Before** toggles the unedited view |
| Zoom | Mouse wheel (anchored at the cursor), or pinch — the zoom readout shows % of true full resolution |
| Pan | Drag the image while zoomed |
| Fit ⇄ 100% | Double-click the image |
| Clipping warnings | Click the dots in the histogram corners: left = shadows (blue), right = highlights (red) |
| Tone curve | Drag on the curve to add/move points; right-click a point to remove it; switch RGB/R/G/B above |
| White balance picker | **💧 WB picker** in Color, then click a neutral gray in the photo |
| Local adjustments | **◎ Mask**, add a Linear/Radial/Brush mask, drag its shape (or paint) in the preview, set its sliders |
| Compose a mask | In the mask's **Shapes** list, **Add** another shape and set its dropdown to Add, Subtract or Intersect; click a row to edit that shape in the preview |
| See what a mask selects | **Show mask coverage** at the top of the mask panel |
| Mask by color / brightness | Open **Range** on the mask; **💧 Pick color** targets the hue you click |
| Brush around edges | Tick **Auto mask** before painting |
| Rate / flag | The cull bar over the photo, the stars/flags in the right panel, or the [keyboard shortcuts](#keyboard-shortcuts) |
| Cull a folder | `Shift`+rating/flag applies it and moves on; or turn on ⏭ auto advance and use plain keys. `←` `→` step through photos |
| Presets | **🎨 Presets** — apply a saved look or save the current one |
| Undo / redo | The ↩ ↪ buttons, or `Ctrl+Z` / `Ctrl+Y` |
| Copy edits | **🗐 Copy**, then **📋 Paste** on another photo, or **📋 All** for the whole folder |
| Export | **💾 Export…** — current photo or all photos, format/quality, destination |
| Tune the engine | **⚙** — effect strengths, blur radii, vignette shape, preview size |

## Keyboard shortcuts

Lightroom's culling keys, so muscle memory carries over.

| Key | Does |
|---|---|
| `←` `→` *or* `↑` `↓` | Previous / next photo (hold to walk the folder) |
| `0` – `5` | Set the star rating (`0` clears it) |
| `[` `]` | Nudge the rating down / up one star |
| `P` | Toggle the **pick** flag |
| `X` | Toggle the **reject** flag |
| `U` | Clear the flag |
| `Shift` + any of the above ratings/flags | Apply it, then jump to the next photo |
| `Ctrl+Z` | Undo |
| `Ctrl+Y` *or* `Ctrl+Shift+Z` | Redo |

### Culling

The intended loop is: look, press a rating or flag with `Shift` held, land on the
next photo, repeat. If you'd rather not hold `Shift` for a whole pass, turn on
**auto advance** (the ⏭ button on the cull bar) and every plain rating or flag
advances too. Lightroom binds that to Caps Lock; egui can't read Caps Lock reliably,
so it's a button here.

The **cull bar** floats over the bottom of the photo in Adjust mode — stars,
pick/reject/clear, auto-advance, and prev/next. It dims while your pointer is up in
the image and comes back to full strength as you approach it. It's hidden in Crop and
Mask mode, where it would sit on top of the handles you're dragging.

Arrowing onto a photo scrolls the filmstrip to keep it in view, and holding an arrow
paces itself rather than running at the key-repeat rate, so a fast sweep through a
folder of RAWs doesn't queue up a decode for every file it passes.

### Notes

`P` and `X` toggle: pressing the key that's already set clears the flag, so `X` on a
rejected photo un-rejects it rather than doing nothing. `U` always clears. Setting a
rating or a flag is an ordinary edit — it lands in the undo history and the sidecar
like any slider, and the filmstrip badge updates with it.

`Shift+1` reports as `!` rather than `1` on most layouts, so the digit keys are
matched on the physical key as well as the logical one. Letters are matched both
ways too, so `P` / `X` / `U` stay on the printed letter on non-US layouts.

Three things suppress every shortcut above:

- **No photo open.** They're ignored on the welcome screen.
- **A text field has focus** — in practice the **🎨 Presets** name box, the only one
  in the app. Keys go to the field instead, so typing "Portrait 2x" as a preset name
  won't re-rate the photo underneath. Click away from the field to get them back.
- **A slider or other widget has keyboard focus**, so the arrow keys nudge that
  widget instead of changing photo. Click the image to hand focus back.

`Ctrl` is `⌘` on macOS. Everything else in the app is mouse-driven; see the
[Usage](#usage) table above, which covers the preview's drag/wheel/double-click
gestures.

## How it works

### Code layout

```
src/
  main.rs             entry point, window setup
  app.rs              application state, layout, worker glue, sidecar autosave
  engine/
    params.rs         EditParams — every slider + geometry value; serde for sidecars
    tuning.rs         Tuning — user-configurable processing constants (persisted)
    histogram.rs      RGB histogram + clipping overlay marking
    pipeline.rs       SourceImage (linear-RGB f32), RenderCtx, region sampling,
                      the render pipeline
    blur.rs           fast approximate Gaussian (3× box blur, transposed passes)
    worker.rs         background render thread: geometry caches, preview/region/
                      export commands, stale-request coalescing
    ops/
      tone.rs         contrast curve, levels, highlight/shadow/white/black masks
      curve.rs        tone-curve monotone spline → 256-entry LUTs
      color.rs        white balance gains + eyedropper solve, vibrance, sat, RGB↔HSL
      hsl.rs          the 8-band color mixer
      detail.rs       texture, clarity, dehaze
      sharpen.rs      unsharp sharpening, bilateral luma NR, chroma NR
      mask.rs         shape coverage, add/subtract/intersect folding, range
                      masks, auto-masked brush dabs
      local.rs        per-pixel local adjustments blended by mask weight
      vignette.rs     radial falloff (normalized coords, region-safe)
      geometry.rs     90° orientation, flips, straighten (into the rotated
                      bounding box), crop, and the frame tests the crop
                      constraint, fill-frame and sidecar migration use
  imgio/
    loader.rs         decode (image crate / rawler) → linear-RGB f32
    sidecar.rs        photo.ext.edits.json load/save
    export.rs         full-res render + JPEG/PNG/TIFF encode; batch export thread
    metadata.rs       best-effort EXIF extraction (kamadak-exif)
    presets.rs        named preset (look) save/load/list
    recent.rs         recent-folders persistence for the welcome screen
    thumbs.rs         filmstrip thumbnails (uses embedded RAW previews)
  ui/
    adjustments.rs    grouped slider panel (Light/Curve/Levels/Color/Mixer/Detail/Effects)
    curve.rs          interactive tone-curve widget
    crop.rs           crop tool panel: rotate/flip/straighten/aspect/constrain
    masks.rs          mask list, shape composition, range mask, local sliders
    filmstrip.rs      thumbnail strip with rating/flag badges
    histogram.rs      histogram plot + clipping toggles
    info.rs           star rating / flag controls, the floating cull bar,
                      and the EXIF panel
    preview.rs        zoom/pan view (including while cropping), full-res region,
                      crop + mask overlays, eyedropper
    settings.rs       processing settings window (Tuning)
    welcome.rs        start screen (open buttons, recent folders)
    export_dialog.rs  export settings window
```

### Processing model

- Every photo is decoded to **linear RGB f32**. RAW files go through rawler's develop
  pipeline (demosaic → camera white balance → color calibration → sRGB); standard
  formats are sRGB-decoded to linear.
- The pipeline stage order mirrors Lightroom: geometry (orientation → straighten →
  crop) → white balance and exposure in linear light → sRGB gamma encode → tone
  ranges → contrast → levels → tone curve → noise reduction → texture/clarity/sharpen
  (unsharp masks against blurred luminance) → color mixer → dehaze → vibrance /
  saturation → local masked adjustments → vignette.
- **Local adjustments** run in the same position-aware pass as the vignette. Linear
  and radial shapes are evaluated analytically per pixel (so they cost nothing to
  store and are resolution-independent), while brush shapes are rasterized from their
  dabs into the current render buffer. A mask's shapes are folded together in order —
  the first sets the base coverage, then each later one adds (`w + c − wc`), subtracts
  (`w(1 − c)`) or intersects (`wc`) — then the whole thing is inverted if asked, and
  finally multiplied by the range mask's verdict on that pixel's luminance and hue.
  The result is a 0..1 weight, and the local adjustment is blended in by it.
  Auto-masked brush dabs store the reference color captured when they were *painted*
  rather than sampling at render time, so a stroke resolves identically in the
  preview, in a zoomed region, and in the export.
- **Straighten grows the canvas.** Rotating a rectangle inside its own dimensions
  pushes its four corners outside them, so straightening into a same-sized canvas
  would clip real pixels off every edge *as well as* adding black corners. Instead
  `rotate_angle` renders into the rotated bounding box (`straightened_dims`), which
  keeps every source pixel; the only dead area is the four corner wedges. The crop
  rectangle is normalized against that grown canvas.
- **The crop constraint**: **Constrain to image** keeps the crop off those wedges.
  `geometry::rect_in_frame` rotates the rectangle's corners back by the straighten
  angle — the same inverse map the rotation itself uses, so the UI and the pixels
  agree by construction — and asks whether they land inside the pre-rotation frame.
  `fit_crop_in_frame` shrinks an out-of-bounds rectangle toward the canvas centre
  until it does, and `fill_frame` grows one the other way to the largest that still
  fits; both move along a path that holds the width:height ratio, so an aspect lock
  survives either. With the toggle off the rectangle is bounded only by the canvas.
  The overlay draws the boundary itself, so the frame visibly stops at something.
- **Interactivity**: the UI thread never touches pixels. A worker thread owns the
  decoded image plus a preview-sized copy; slider changes send parameters over a
  channel, stale requests are dropped, and only the newest state is rendered
  (rayon-parallel), so sliders stay real-time even for large RAW files.
- **Full-res zoom**: when the view outgrows the preview, the app requests just the
  visible rectangle. The worker bilinear-samples that region from the (cached,
  geometry-applied) full-resolution image at exactly the on-screen pixel density —
  cost scales with the viewport, not the photo — then runs the same pipeline on it.
  Position-dependent ops stay correct because the render carries a `RenderCtx`
  mapping the region back into full-image coordinates (vignette) and an equivalent
  radius scale (texture/clarity).
- **Export** runs the identical pipeline once at full resolution, so what you see is
  what you get.

### Sidecar format

A pretty-printed JSON serialization of `EditParams` — safe to inspect, diff, or
delete. Deleting a sidecar simply reverts the photo to its unedited state. Unknown
or missing fields are tolerated, so sidecars stay compatible across app versions.

Where a field's shape has genuinely changed, the old one is still read and migrated
on load:

- Masks used to hold a single `kind` before they could compose several shapes, and
  such a mask now loads with that shape as its first component.
- `crop` used to be a fraction of the *un-grown* straighten canvas, back when
  straightening rotated inside the source size. It is now a fraction of the rotated
  bounding box, which is larger and shares the same centre. Sidecars written before
  the change have no `crop_space` key, read as `0`, and are re-normalized once when
  the photo's dimensions arrive — same pixels, new denominator. Only photos that had
  both a straighten angle and a crop are touched, and only once; the rewritten
  sidecar carries `"crop_space": 1`.
