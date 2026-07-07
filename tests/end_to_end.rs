//! End-to-end (headless) test of the import -> edit -> export path:
//! decode a real file from disk, run the full pipeline with non-trivial
//! params, encode to JPEG, and verify the result.

use photo_editor::engine::params::EditParams;
use photo_editor::imgio::{export, loader, sidecar};

#[test]
fn import_edit_export_round_trip() {
    let dir = std::env::temp_dir().join("photo-editor-e2e");
    std::fs::create_dir_all(&dir).unwrap();
    let src_path = dir.join("source.png");
    let out_path = dir.join("edited.jpg");

    // Build a 200x120 gradient "photo" on disk.
    let img = image::RgbImage::from_fn(200, 120, |x, y| {
        image::Rgb([
            (x * 255 / 200) as u8,
            (y * 255 / 120) as u8,
            ((x + y) % 255) as u8,
        ])
    });
    img.save(&src_path).unwrap();

    // Import the way the app does.
    let src = loader::load(&src_path).unwrap();
    assert_eq!(src.width, 200);
    assert_eq!(src.height, 120);

    // Edit: touch every category of adjustment.
    let mut params = EditParams::default();
    params.exposure = 0.5;
    params.contrast = 25.0;
    params.highlights = -30.0;
    params.shadows = 20.0;
    params.temp = 15.0;
    params.vibrance = 30.0;
    params.hsl[5].sat = -50.0; // mute blues
    params.texture = 20.0;
    params.clarity = 15.0;
    params.dehaze = 10.0;
    params.vignette = -35.0;

    // Sidecar persistence round-trip.
    sidecar::save(&src_path, &params).unwrap();
    let restored = sidecar::load(&src_path).unwrap();
    assert!(restored == params);

    // Export at full resolution.
    export::export(&src, &params, &out_path, export::ExportFormat::Jpeg, 90).unwrap();
    let exported = image::open(&out_path).unwrap();
    assert_eq!(exported.width(), 200);
    assert_eq!(exported.height(), 120);

    // The edit must actually change pixels vs. a straight identity export.
    let identity_out = dir.join("identity.jpg");
    export::export(
        &src,
        &EditParams::default(),
        &identity_out,
        export::ExportFormat::Jpeg,
        90,
    )
    .unwrap();
    let identity = image::open(&identity_out).unwrap().to_rgb8();
    let edited = exported.to_rgb8();
    let diff: u64 = identity
        .pixels()
        .zip(edited.pixels())
        .map(|(a, b)| {
            a.0.iter()
                .zip(b.0.iter())
                .map(|(x, y)| (*x as i64 - *y as i64).unsigned_abs())
                .sum::<u64>()
        })
        .sum();
    assert!(diff > 100_000, "edits didn't change the output (diff {diff})");

    std::fs::remove_dir_all(&dir).ok();
}
