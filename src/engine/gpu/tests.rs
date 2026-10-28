//! GPU ↔ CPU parity. Each test renders one source through
//! `pipeline::render_rgb` and through the GPU and compares the 8-bit
//! results. On a machine with no GPU at all (not even a software adapter)
//! they print a note and pass.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use super::*;
use crate::engine::params::{Dab, LocalAdjust, MaskComponent, MaskKind, MaskOp};
use crate::engine::pipeline::tests::{every_spatial_stage, noisy_image};
use crate::engine::pipeline::{mask_coverage, render_rgb, rgb_to_rgba};

/// Run `f` with the shared renderer. Opening a device and compiling every
/// pipeline is the slow part, so all tests share one, taking turns. Only a
/// machine with no adapter at all skips; any other failure to open — a
/// shader that doesn't compile, say — fails every test.
fn with_gpu(f: impl FnOnce(&mut Renderer)) {
    static GPU: OnceLock<Mutex<Option<Renderer>>> = OnceLock::new();
    let cell = GPU.get_or_init(|| {
        Mutex::new(match Renderer::new_allowing_software() {
            Ok(r) => {
                eprintln!("GPU tests running on {}", r.label());
                Some(r)
            }
            Err(e) if e.is::<NoAdapter>() => {
                eprintln!("no GPU adapter, skipping GPU tests: {e:#}");
                None
            }
            Err(e) => panic!("GPU renderer failed to open: {e:#}"),
        })
    });
    let mut guard = cell.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(r) = guard.as_mut() {
        f(r);
    }
}

/// A fresh source key, so tests sharing the renderer never see each other's
/// uploads.
fn key() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// The test image, rounded through f16 the way resident GPU sources are,
/// so both paths start from identical pixels.
fn source(w: usize, h: usize) -> SourceImage {
    let mut img = noisy_image(w, h);
    for v in &mut img.data {
        *v = half::f16::from_f32(*v).to_f32();
    }
    img
}

/// Compare two byte buffers: no value may be more than `tol` apart, and on
/// average they must agree almost exactly. Rounding differences between
/// GPU and CPU math flip the odd value by a step; a real discrepancy, even a
/// small one, shifts many values and shows up in the mean.
fn assert_close(cpu: &[u8], gpu: &[u8], tol: u32, what: &str) {
    assert_eq!(cpu.len(), gpu.len(), "{what}: length");
    let (mut max, mut sum, mut worst) = (0u32, 0u64, 0usize);
    for (i, (a, b)) in cpu.iter().zip(gpu).enumerate() {
        let d = (*a as i32 - *b as i32).unsigned_abs();
        if d > max {
            max = d;
            worst = i;
        }
        sum += d as u64;
    }
    let mean = sum as f64 / cpu.len().max(1) as f64;
    assert!(
        max <= tol && mean < 0.02,
        "{what}: max diff {max} at byte {worst} (cpu {} gpu {}), mean {mean:.4}",
        cpu[worst],
        gpu[worst]
    );
}

/// Render the whole of `src` both ways, preview-style, and compare.
fn check(p: &EditParams, t: &Tuning, what: &str) {
    let src = source(96, 64);
    let cpu = rgb_to_rgba(&render_rgb(&src, p, t, RenderCtx::full(96, 64)));
    with_gpu(|r| {
        assert!(r.set_source(Slot::Preview, key(), &src).unwrap());
        let ctx = RenderCtx::full(96, 64);
        let out = r.render(Slot::Preview, (96, 64), [0.0, 0.0, 1.0, 1.0], ctx, p, t, None);
        let out = out.unwrap();
        assert!(out.coverage.is_none());
        assert_close(&cpu, &out.rgba, 2, what);
    });
}

fn params(f: impl FnOnce(&mut EditParams)) -> EditParams {
    let mut p = EditParams::default();
    f(&mut p);
    p
}

#[test]
fn identity_matches() {
    check(&EditParams::default(), &Tuning::default(), "identity");
}

#[test]
fn light_matches() {
    let p = params(|p| {
        p.exposure = 0.7;
        p.contrast = 35.0;
        p.highlights = -60.0;
        p.shadows = 45.0;
        p.whites = 20.0;
        p.blacks = -25.0;
        p.temp = 30.0;
        p.tint = -15.0;
    });
    check(&p, &Tuning::default(), "light");
    let flat = params(|p| {
        p.contrast = -60.0;
        p.highlights = 80.0;
    });
    check(&flat, &Tuning::default(), "negative contrast");
}

#[test]
fn levels_and_curve_match() {
    let p = params(|p| {
        p.lv_in_black = 0.08;
        p.lv_in_white = 0.9;
        p.lv_gamma = 1.3;
        p.lv_out_black = 0.05;
        p.lv_out_white = 0.95;
        p.curve.master = vec![[0.0, 0.0], [0.3, 0.22], [0.7, 0.8], [1.0, 1.0]];
        p.curve.r = vec![[0.0, 0.05], [1.0, 1.0]];
        p.curve.b = vec![[0.0, 0.0], [0.5, 0.4], [1.0, 0.95]];
    });
    check(&p, &Tuning::default(), "levels + curve");
}

#[test]
fn color_matches() {
    let p = params(|p| {
        p.vibrance = 50.0;
        p.saturation = -20.0;
        p.dehaze = 40.0;
    });
    check(&p, &Tuning::default(), "vibrance / saturation / dehaze");
    let hazy = params(|p| {
        p.dehaze = -50.0;
        p.vibrance = -40.0;
    });
    check(&hazy, &Tuning::default(), "negative dehaze / vibrance");
}

#[test]
fn color_mixer_matches() {
    let p = params(|p| {
        p.hsl[0].hue = 40.0;
        p.hsl[2].sat = -70.0;
        p.hsl[3].lum = 50.0;
        p.hsl[5].hue = -60.0;
        p.hsl[5].sat = 30.0;
        p.hsl[7].lum = -40.0;
    });
    check(&p, &Tuning::default(), "color mixer");
}

#[test]
fn noise_reduction_matches() {
    let p = params(|p| {
        p.luminance_nr = 70.0;
        p.color_nr = 90.0;
    });
    check(&p, &Tuning::default(), "noise reduction");
}

#[test]
fn detail_matches() {
    let p = params(|p| {
        p.texture = 60.0;
        p.clarity = -40.0;
        p.sharpen = 80.0;
        p.sharpen_radius = 2.5;
    });
    let t = Tuning {
        clarity_radius: 0.03,
        ..Tuning::default()
    };
    check(&p, &t, "texture / clarity / sharpen");
}

#[test]
fn vignette_matches() {
    let p = params(|p| p.vignette = -70.0);
    check(&p, &Tuning::default(), "vignette");
    let p = params(|p| p.vignette = 50.0);
    check(&p, &Tuning::default(), "positive vignette");
}

fn mask(components: Vec<MaskComponent>, adjust: LocalAdjust) -> Mask {
    Mask {
        name: "m".into(),
        components,
        adjust,
        ..Mask::default()
    }
}

fn radial(center: [f32; 2], r: f32) -> MaskKind {
    MaskKind::Radial {
        center,
        radius: [r, r * 1.3],
        feather: 0.6,
    }
}

fn brighten() -> LocalAdjust {
    LocalAdjust {
        exposure: 50.0,
        contrast: 20.0,
        ..LocalAdjust::default()
    }
}

#[test]
fn composed_masks_match() {
    let mut composed = mask(
        vec![
            MaskComponent::new(radial([0.4, 0.5], 0.35), MaskOp::Add),
            MaskComponent::new(radial([0.45, 0.5], 0.12), MaskOp::Subtract),
            MaskComponent {
                kind: MaskKind::Linear {
                    p0: [0.2, 0.3],
                    p1: [0.8, 0.6],
                },
                op: MaskOp::Intersect,
                inverted: true,
            },
            MaskComponent::new(radial([0.85, 0.2], 0.2), MaskOp::Add),
        ],
        brighten(),
    );
    let mut p = params(|p| p.masks.push(composed.clone()));
    check(&p, &Tuning::default(), "composed mask");
    composed.inverted = true;
    p.masks[0] = composed;
    check(&p, &Tuning::default(), "inverted composed mask");
}

#[test]
fn range_masks_match() {
    let mut lum = mask(Vec::new(), brighten());
    lum.range.lum_enabled = true;
    lum.range.lum_lo = 0.3;
    lum.range.lum_hi = 0.6;
    let mut hue = mask(
        vec![MaskComponent::new(radial([0.5, 0.5], 0.4), MaskOp::Add)],
        LocalAdjust {
            saturation: 60.0,
            temp: -40.0,
            tint: 20.0,
            ..LocalAdjust::default()
        },
    );
    hue.range.color_enabled = true;
    hue.range.hue = 200.0;
    hue.range.hue_width = 40.0;
    let p = params(|p| {
        p.masks.push(lum);
        p.masks.push(hue);
    });
    check(&p, &Tuning::default(), "range masks");
}

fn stroke(auto: Option<[f32; 3]>) -> Vec<Dab> {
    (0..40)
        .map(|i| Dab {
            p: [0.15 + i as f32 * 0.018, 0.3 + (i as f32 * 0.3).sin() * 0.15],
            radius: 0.12,
            hardness: 0.4,
            erase: i % 9 == 8,
            auto,
        })
        .collect()
}

#[test]
fn brush_masks_match() {
    let p = params(|p| {
        p.masks.push(mask(
            vec![MaskComponent::new(MaskKind::Brush { dabs: stroke(None) }, MaskOp::Add)],
            brighten(),
        ));
        p.masks.push(mask(
            vec![
                MaskComponent::new(radial([0.5, 0.6], 0.5), MaskOp::Add),
                MaskComponent::new(
                    MaskKind::Brush {
                        dabs: stroke(Some([0.55, 0.5, 0.6])),
                    },
                    MaskOp::Subtract,
                ),
            ],
            LocalAdjust {
                exposure: -60.0,
                ..LocalAdjust::default()
            },
        ));
    });
    check(&p, &Tuning::default(), "brush masks");
}

#[test]
fn local_detail_matches() {
    let p = params(|p| {
        p.masks.push(mask(
            vec![MaskComponent::new(radial([0.5, 0.5], 0.45), MaskOp::Add)],
            LocalAdjust {
                texture: 70.0,
                clarity: 60.0,
                sharpness: 80.0,
                noise: 70.0,
                dehaze: 40.0,
                highlights: -40.0,
                shadows: 30.0,
                vibrance: -30.0,
                ..LocalAdjust::default()
            },
        ));
        // A second mask with only some of the neighbourhood buffers in use.
        p.masks.push(mask(
            vec![MaskComponent::new(radial([0.3, 0.3], 0.3), MaskOp::Add)],
            LocalAdjust {
                clarity: -50.0,
                ..LocalAdjust::default()
            },
        ));
    });
    let t = Tuning {
        clarity_radius: 0.03,
        ..Tuning::default()
    };
    check(&p, &t, "local detail");
}

#[test]
fn everything_at_once_matches() {
    let mut p = every_spatial_stage();
    p.contrast = 20.0;
    p.highlights = -30.0;
    p.curve.master = vec![[0.0, 0.0], [0.4, 0.5], [1.0, 1.0]];
    p.hsl[3].sat = 40.0;
    p.dehaze = 15.0;
    p.saturation = 10.0;
    check(&p, &Tuning::default(), "everything");
}

#[test]
fn region_matches_the_cpu_region() {
    // A zoomed-in region: bilinear-sampled from the full image at a scale
    // that isn't 1:1, with radii scaled to the full image.
    let full = source(160, 120);
    let p = every_spatial_stage();
    let t = Tuning::default();
    let rect = [0.3, 0.2, 0.45, 0.5];
    let (tw, th) = (90, 75);
    let ctx = RenderCtx {
        norm_rect: rect,
        radius_dim: 160.0 * 1.25,
    };
    let cpu = rgb_to_rgba(&render_rgb(&full.sample_region(rect, tw, th), &p, &t, ctx));
    with_gpu(|r| {
        assert!(r.set_source(Slot::Full, key(), &full).unwrap());
        let out = r.render(Slot::Full, (tw, th), rect, ctx, &p, &t, None).unwrap();
        assert_close(&cpu, &out.rgba, 2, "region");
    });
}

#[test]
fn overlay_coverage_matches() {
    let src = source(96, 64);
    let ctx = RenderCtx::full(96, 64);
    let t = Tuning::default();
    let mut m = mask(
        vec![
            MaskComponent::new(radial([0.5, 0.5], 0.35), MaskOp::Add),
            MaskComponent::new(
                MaskKind::Brush {
                    dabs: stroke(Some([0.5, 0.5, 0.55])),
                },
                MaskOp::Add,
            ),
        ],
        brighten(),
    );
    m.range.lum_enabled = true;
    m.range.lum_lo = 0.2;
    let p = params(|p| {
        p.vibrance = 30.0;
        p.masks.push(m.clone());
    });
    let rgb = render_rgb(&src, &p, &t, ctx);
    let cpu_cov: Vec<u8> = mask_coverage(&m, &rgb, 96, 64, ctx)
        .iter()
        .map(|w| (w * 255.0 + 0.5) as u8)
        .collect();
    let empty = Mask {
        components: Vec::new(),
        ..m.clone()
    };
    let empty = Mask {
        range: Default::default(),
        ..empty
    };
    with_gpu(|r| {
        assert!(r.set_source(Slot::Preview, key(), &src).unwrap());
        let out = r
            .render(Slot::Preview, (96, 64), [0.0, 0.0, 1.0, 1.0], ctx, &p, &t, Some(&m))
            .unwrap();
        assert!(out.rgba.chunks(4).all(|px| px[3] == 255), "alpha restored");
        assert_close(&rgb_to_rgba(&rgb), &out.rgba, 2, "overlay pixels");
        assert_close(&cpu_cov, &out.coverage.unwrap(), 2, "overlay coverage");
        // A mask that selects nothing washes nothing.
        let out = r
            .render(Slot::Preview, (96, 64), [0.0, 0.0, 1.0, 1.0], ctx, &p, &t, Some(&empty))
            .unwrap();
        assert!(out.coverage.unwrap().iter().all(|&c| c == 0));
    });
}

#[test]
fn tiled_export_matches_the_full_cpu_render() {
    let src = noisy_image(200, 150); // export tiles upload full floats
    let mut p = every_spatial_stage();
    p.dehaze = 20.0;
    let t = Tuning {
        clarity_radius: 0.02,
        ..Tuning::default()
    };
    let cpu: Vec<u8> = render_rgb(&src, &p, &t, RenderCtx::full(200, 150))
        .iter()
        .map(|v| (v * 255.0 + 0.5) as u8)
        .collect();
    let halo = StageRadii::new(&p, &t, 200.0).halo();
    with_gpu(|r| {
        // Small tiles, so the image is cut many times over.
        let edge = 4 * halo;
        assert!(plan_tiles(200, 150, halo, edge).unwrap().len() >= 6);
        let gpu = r.render_tiled(&src, &p, &t, &[edge]).unwrap().unwrap();
        assert_close(&cpu, &gpu, 2, "tiled export");
        // And as one tile, through the public entry point.
        let whole = r.render_export(&src, &p, &t).unwrap().unwrap();
        assert_close(&cpu, &whole, 2, "single-tile export");
    });
}

#[test]
fn export_too_wide_to_tile_falls_back() {
    let src = noisy_image(300, 200);
    let p = params(|p| p.clarity = 50.0);
    let t = Tuning {
        clarity_radius: 0.03, // 9 px × 3 passes
        ..Tuning::default()
    };
    with_gpu(|r| {
        // An edge under 4× the halo can't be tiled efficiently.
        assert!(r.render_tiled(&src, &p, &t, &[64]).unwrap().is_none());
    });
}

/// Not a check — timings. `cargo test --release bench_gpu_vs_cpu --
/// --ignored --nocapture` prints CPU vs GPU for a heavy edit at preview,
/// zoomed-region and full-export sizes.
#[test]
#[ignore]
fn bench_gpu_vs_cpu() {
    use std::time::Instant;
    let mut p = every_spatial_stage();
    p.dehaze = 20.0;
    p.hsl[3].sat = 30.0;
    // A long auto-masked brush stroke: the CPU's slowest local stage.
    p.masks.push(mask(
        vec![MaskComponent::new(
            MaskKind::Brush {
                dabs: (0..600)
                    .map(|i| Dab {
                        p: [0.05 + i as f32 * 0.0015, 0.5 + (i as f32 * 0.05).sin() * 0.3],
                        radius: 0.04,
                        hardness: 0.5,
                        erase: false,
                        auto: Some([0.5, 0.5, 0.5]),
                    })
                    .collect(),
            },
            MaskOp::Add,
        )],
        brighten(),
    ));
    let t = Tuning::default();
    // The GPU gets a longer warm-up: it idles at low clocks while the CPU
    // runs, and while a slider is dragged it stays busy, so a cold first
    // render would understate it.
    let time = |warm: usize, n: usize, f: &mut dyn FnMut()| {
        for _ in 0..warm {
            f();
        }
        let start = Instant::now();
        for _ in 0..n {
            f();
        }
        start.elapsed().as_secs_f64() * 1000.0 / n as f64
    };
    let cpu = |f: &mut dyn FnMut()| time(1, 3, f);
    let gpu = |f: &mut dyn FnMut()| time(5, 10, f);
    let full = noisy_image(6000, 4000);
    let preview = full.downscale(1600);
    let (pw, ph) = (preview.width, preview.height);
    let rect = [0.4, 0.4, 2560.0 / 6000.0, 1440.0 / 4000.0];
    let region_ctx = RenderCtx {
        norm_rect: rect,
        radius_dim: 6000.0,
    };
    with_gpu(|r| {
        assert!(r.set_source(Slot::Preview, key(), &preview).unwrap());
        assert!(r.set_source(Slot::Full, key(), &full).unwrap());
        let whole = [0.0, 0.0, 1.0, 1.0];
        let rows = [
            (
                format!("preview {pw}×{ph}"),
                cpu(&mut || {
                    render_rgb(&preview, &p, &t, RenderCtx::full(pw, ph));
                }),
                gpu(&mut || {
                    r.render(Slot::Preview, (pw, ph), whole, RenderCtx::full(pw, ph), &p, &t, None)
                        .unwrap();
                }),
            ),
            (
                "region 2560×1440 at 1:1".to_string(),
                cpu(&mut || {
                    let src = full.sample_region(rect, 2560, 1440);
                    render_rgb(&src, &p, &t, region_ctx);
                }),
                gpu(&mut || {
                    r.render(Slot::Full, (2560, 1440), rect, region_ctx, &p, &t, None)
                        .unwrap();
                }),
            ),
            (
                "export 6000×4000".to_string(),
                cpu(&mut || {
                    render_rgb(&full, &p, &t, RenderCtx::full(6000, 4000));
                }),
                gpu(&mut || {
                    r.render_export(&full, &p, &t).unwrap().unwrap();
                }),
            ),
        ];
        eprintln!("\n{} — every spatial stage + 600-dab auto brush", r.label());
        for (what, cpu, gpu) in rows {
            eprintln!("  {what:<26} CPU {cpu:>8.1} ms   GPU {gpu:>7.1} ms   ×{:.1}", cpu / gpu);
        }
    });
}

#[test]
fn oversized_source_is_refused_not_fatal() {
    with_gpu(|r| {
        let max = r.gpu.limits.max_texture_dimension_2d as usize;
        let img = SourceImage {
            width: max + 1,
            height: 1,
            data: vec![0.5; (max + 1) * 3],
        };
        assert!(!r.set_source(Slot::Full, key(), &img).unwrap());
    });
}
