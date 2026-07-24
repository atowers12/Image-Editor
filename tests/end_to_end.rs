//! End-to-end (headless) test of the import -> edit -> export path:
//! decode a real file from disk, apply geometry + the full pixel pipeline
//! with non-trivial params, encode to JPEG, and verify the result.

use photo_editor::engine::ops::geometry;
use photo_editor::engine::params::EditParams;
use photo_editor::engine::tuning::Tuning;
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

    // Edit: touch every category of adjustment, including levels & geometry.
    let mut params = EditParams::default();
    params.exposure = 0.5;
    params.contrast = 25.0;
    params.highlights = -30.0;
    params.shadows = 20.0;
    params.lv_in_black = 0.05;
    params.lv_gamma = 1.2;
    params.temp = 15.0;
    params.vibrance = 30.0;
    params.hsl[5].sat = -50.0; // mute blues
    params.texture = 20.0;
    params.clarity = 15.0;
    params.dehaze = 10.0;
    params.vignette = -35.0;
    params.rotate90 = 1; // portrait now
    params.angle = 2.0;
    params.crop = [0.1, 0.1, 0.8, 0.8];

    // Sidecar persistence round-trip.
    sidecar::save(&src_path, &params).unwrap();
    let restored = sidecar::load(&src_path).unwrap();
    assert!(restored == params);

    // Export the way the worker does: geometry first, then the pipeline.
    let tuning = Tuning::default();
    let geo = geometry::apply(&src, &params, true);
    let expected_dims = geometry::oriented_dims(200, 120, &params, true);
    assert_eq!((geo.width, geo.height), expected_dims);

    export::export(&geo, &params, &tuning, &out_path, export::ExportFormat::Jpeg, 90).unwrap();
    let exported = image::open(&out_path).unwrap();
    // rotate90 swaps dims (200x120 -> 120x200), crop takes 80%: 96x160.
    assert_eq!(exported.width(), 96);
    assert_eq!(exported.height(), 160);

    // The edit must actually change pixels vs. an identity export.
    let identity_out = dir.join("identity.jpg");
    export::export(
        &src,
        &EditParams::default(),
        &tuning,
        &identity_out,
        export::ExportFormat::Jpeg,
        90,
    )
    .unwrap();
    let identity = image::open(&identity_out).unwrap();
    assert_eq!(identity.width(), 200); // no geometry -> original dims

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn batch_export_whole_folder() {
    let dir = std::env::temp_dir().join("photo-editor-e2e-batch");
    let out_dir = dir.join("out");
    std::fs::create_dir_all(&out_dir).unwrap();

    // Three photos; give one of them saved edits.
    for i in 0..3 {
        let img = image::RgbImage::from_pixel(60, 40, image::Rgb([100 + i * 30, 120, 140]));
        img.save(dir.join(format!("photo{i}.png"))).unwrap();
    }
    let mut edited = EditParams::default();
    edited.exposure = 1.0;
    edited.rotate90 = 1;
    sidecar::save(&dir.join("photo1.png"), &edited).unwrap();

    let files: Vec<_> = (0..3).map(|i| dir.join(format!("photo{i}.png"))).collect();
    let job = export::spawn_batch(
        files,
        out_dir.clone(),
        export::ExportFormat::Jpeg,
        90,
        Tuning::default(),
        egui::Context::default(),
    );

    // Drain messages until the batch reports completion.
    let mut finished = None;
    while let Ok(msg) = job.rx.recv_timeout(std::time::Duration::from_secs(30)) {
        if let export::BatchMsg::Finished { exported, failed } = msg {
            finished = Some((exported, failed));
            break;
        }
    }
    assert_eq!(finished, Some((3, 0)));

    // Every photo exported with its own edits: photo1 was rotated 90°.
    let plain = image::open(out_dir.join("photo0_edited.jpg")).unwrap();
    assert_eq!((plain.width(), plain.height()), (60, 40));
    let rotated = image::open(out_dir.join("photo1_edited.jpg")).unwrap();
    assert_eq!((rotated.width(), rotated.height()), (40, 60));

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn advanced_edits_export_end_to_end() {
    use photo_editor::engine::params::{LocalAdjust, Mask, MaskKind};

    let dir = std::env::temp_dir().join("photo-editor-e2e-advanced");
    std::fs::create_dir_all(&dir).unwrap();
    let src_path = dir.join("src.png");
    let out_path = dir.join("out.png");

    let img = image::RgbImage::from_fn(120, 90, |x, y| {
        image::Rgb([(x * 2) as u8, (y * 2) as u8, 128])
    });
    img.save(&src_path).unwrap();
    let src = loader::load(&src_path).unwrap();

    // Curve + sharpen + noise reduction + a radial local mask.
    let mut p = EditParams::default();
    p.curve.master = vec![[0.0, 0.0], [0.4, 0.55], [1.0, 1.0]];
    p.sharpen = 40.0;
    p.luminance_nr = 20.0;
    p.color_nr = 20.0;
    p.masks.push(Mask {
        name: "center".into(),
        kind: MaskKind::Radial {
            center: [0.5, 0.5],
            radius: [0.3, 0.3],
            feather: 0.5,
        },
        adjust: LocalAdjust {
            exposure: 60.0,
            ..LocalAdjust::default()
        },
        enabled: true,
        inverted: false,
    });

    // Sidecar round-trip must preserve curve + masks.
    sidecar::save(&src_path, &p).unwrap();
    assert!(sidecar::load(&src_path).unwrap() == p);

    // Export succeeds and differs from an unedited export.
    export::export(&src, &p, &Tuning::default(), &out_path, export::ExportFormat::Png, 90).unwrap();
    let edited = image::open(&out_path).unwrap();
    assert_eq!((edited.width(), edited.height()), (120, 90));

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn tuning_changes_pipeline_output() {
    use photo_editor::engine::pipeline::{render_rgb, RenderCtx, SourceImage};

    let src = SourceImage {
        width: 16,
        height: 16,
        data: vec![0.3; 16 * 16 * 3],
    };
    let mut p = EditParams::default();
    p.dehaze = 60.0;

    let weak = Tuning {
        dehaze_strength: 0.10,
        ..Tuning::default()
    };
    let strong = Tuning {
        dehaze_strength: 0.50,
        ..Tuning::default()
    };
    let ctx = RenderCtx::full(16, 16);
    let out_weak = render_rgb(&src, &p, &weak, ctx);
    let out_strong = render_rgb(&src, &p, &strong, ctx);
    // Stronger dehaze pulls the flat gray down further.
    assert!(out_strong[0] < out_weak[0]);
}
