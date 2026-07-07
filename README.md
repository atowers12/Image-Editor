# Photo Editor

A Lightroom-style photo editor written in Rust, using [egui/eframe](https://github.com/emilk/egui)
for the interface. Non-destructive by design: your original files are never modified.

## Features

- **Browse**: open a folder (or a single file) and flip through photos in a filmstrip
  with background-generated thumbnails
- **Formats**: JPEG, PNG, TIFF, WebP, BMP, and camera RAW — CR2, CR3, NEF, NRW, ARW,
  DNG, RAF, ORF, RW2, PEF, SRW, ERF, KDC, DCR, 3FR, IIQ (decoded with
  [rawler](https://crates.io/crates/rawler): demosaic, camera white balance, and
  color calibration)
- **Adjustments**:
  - *Light*: exposure (±5 EV), contrast, highlights, shadows, whites, blacks
  - *Color*: temperature, tint, vibrance (with skin-tone protection), saturation
  - *Color Mixer*: hue / saturation / luminance per color, across 8 bands
    (red, orange, yellow, green, aqua, blue, purple, magenta)
  - *Effects*: texture, clarity, dehaze, vignette
- **Non-destructive editing**: slider values are saved to a small JSON sidecar next to
  the original (`photo.jpg` → `photo.jpg.edits.json`). Reopening a photo restores its
  edits; resetting everything removes the sidecar. Sidecar writes are debounced and
  also flushed on photo switch and app close.
- **Export**: bakes the edits into a new full-resolution file — JPEG (adjustable
  quality), PNG, or TIFF

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
| Zoom | Mouse wheel (anchored at the cursor), or pinch |
| Pan | Drag the image while zoomed |
| Fit ⇄ 100% | Double-click the image |
| Export | **💾 Export…**, choose format/quality, pick a destination |

## How it works

### Code layout

```
src/
  main.rs             entry point, window setup
  app.rs              application state, layout, worker glue, sidecar autosave
  engine/
    params.rs         EditParams — every slider value; serde for sidecars
    pipeline.rs       SourceImage (linear-RGB f32) + the render pipeline
    blur.rs           fast approximate Gaussian (3× box blur, transposed passes)
    worker.rs         background render thread with stale-request coalescing
    ops/
      tone.rs         contrast curve, highlight/shadow/white/black range masks
      color.rs        white balance gains, vibrance, saturation, RGB↔HSL
      hsl.rs          the 8-band color mixer
      detail.rs       texture, clarity, dehaze
      vignette.rs     radial falloff
  imgio/
    loader.rs         decode (image crate / rawler) → linear-RGB f32
    sidecar.rs        photo.ext.edits.json load/save
    export.rs         full-res render + JPEG/PNG/TIFF encode
    thumbs.rs         filmstrip thumbnails (uses embedded RAW previews)
  ui/
    adjustments.rs    grouped slider panel
    filmstrip.rs      thumbnail strip
    preview.rs        zoom/pan photo view
    export_dialog.rs  export settings window
```

### Processing model

- Every photo is decoded to **linear RGB f32**. RAW files go through rawler's develop
  pipeline (demosaic → camera white balance → color calibration → sRGB); standard
  formats are sRGB-decoded to linear.
- The pipeline stage order mirrors Lightroom: white balance and exposure in linear
  light → sRGB gamma encode → tone ranges → contrast → texture/clarity (unsharp
  masks against blurred luminance at two radii) → color mixer → dehaze → vibrance /
  saturation → vignette.
- **Interactivity**: the UI thread never touches pixels. A worker thread owns the
  decoded image plus a ~1600 px preview copy; slider changes send parameters over a
  channel, stale requests are dropped, and only the newest state is rendered
  (rayon-parallel), so sliders stay real-time even for large RAW files.
- **Export** runs the identical pipeline once at full resolution, so what you see is
  what you get.

### Sidecar format

A pretty-printed JSON serialization of `EditParams` — safe to inspect, diff, or
delete. Deleting a sidecar simply reverts the photo to its unedited state. Unknown
or missing fields are tolerated, so sidecars stay compatible across app versions.
