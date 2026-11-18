// SPDX-License-Identifier: MIT OR Apache-2.0

//! Renders upstream screenshot cases headless with `Window::take_snapshot()`, saves them to the
//! directory given as argument, and compares them with upstream's Skia references.

slint::include_modules!();

use slint::ComponentHandle;

fn main() {
    let out = std::path::PathBuf::from(std::env::args().nth(1).expect("an output directory"));
    std::fs::create_dir_all(&out).unwrap();
    i_slint_backend_testing::init_no_event_loop();
    i_slint_backend_testing::configure_test_fonts();
    let references = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../reference/slint-1.19.0/tests/screenshots/references/skia");
    check(&out, &references, "image/images", (64, 64), &Images::new().unwrap());
    check(&out, &references, "image/border-image", (64, 64), &BorderImage::new().unwrap());
    check(
        &out,
        &references,
        "image/image-repeat-colorize",
        (64, 64),
        &ImageRepeatColorize::new().unwrap(),
    );
    check(&out, &references, "image/images-alignment", (64, 64), &ImagesAlignment::new().unwrap());
    check(&out, &references, "basic/backdrop-blur", (128, 112), &BackdropBlur::new().unwrap());
}

fn check(
    out: &std::path::Path,
    references: &std::path::Path,
    case: &str,
    (w, h): (u32, u32),
    component: &impl ComponentHandle,
) {
    component.window().set_size(slint::PhysicalSize::new(w, h));
    component.show().unwrap();
    let shot = match component.window().take_snapshot() {
        Ok(shot) => shot,
        Err(e) => {
            println!("{case}: no snapshot: {e}");
            return;
        }
    };
    let name = case.replace('/', "-");
    if std::env::args().any(|a| a == "--dump") {
        let host = i_slint_core::host::Win32WindowAdapter::from_window(component.window()).unwrap();
        println!("{case}:\n{}", host.dump_visuals());
    }
    image::save_buffer(
        out.join(format!("{name}.png")),
        shot.as_bytes(),
        shot.width(),
        shot.height(),
        image::ColorType::Rgba8,
    )
    .unwrap();
    let reference = image::open(references.join(format!("{case}.png"))).unwrap().into_rgba8();
    if (reference.width(), reference.height()) != (shot.width(), shot.height()) {
        println!(
            "{case}: size {}x{}, reference {}x{}",
            shot.width(),
            shot.height(),
            reference.width(),
            reference.height()
        );
        return;
    }
    let (mut differ, mut far, mut max) = (0, 0, 0f32);
    for (a, b) in reference.pixels().zip(shot.as_slice()) {
        let d = ((a[0] as f32 - b.r as f32).powi(2)
            + (a[1] as f32 - b.g as f32).powi(2)
            + (a[2] as f32 - b.b as f32).powi(2)
            + (a[3] as f32 - b.a as f32).powi(2))
        .sqrt();
        differ += (d > 4.) as u32;
        far += (d > 64.) as u32;
        max = max.max(d);
    }
    // Reference, snapshot, and difference side by side, magnified, for looking at.
    let zoom = 4;
    let mut side = image::RgbaImage::new(w * 3 * zoom, h * zoom);
    for (i, (a, b)) in reference.pixels().zip(shot.as_slice()).enumerate() {
        let (x, y) = (i as u32 % w, i as u32 / w);
        let b = image::Rgba([b.r, b.g, b.b, b.a]);
        let d = image::Rgba([
            a[0].abs_diff(b[0]).saturating_mul(4),
            a[1].abs_diff(b[1]).saturating_mul(4),
            a[2].abs_diff(b[2]).saturating_mul(4),
            255,
        ]);
        for (k, p) in [*a, b, d].into_iter().enumerate() {
            for dy in 0..zoom {
                for dx in 0..zoom {
                    side.put_pixel((k as u32 * w + x) * zoom + dx, y * zoom + dy, p);
                }
            }
        }
    }
    side.save(out.join(format!("{name}-cmp.png"))).unwrap();
    let n = w * h;
    println!(
        "{case}: {:.1}% of pixels differ by more than 4, {:.1}% by more than 64, max {max:.0}",
        differ as f32 * 100. / n as f32,
        far as f32 * 100. / n as f32
    );
    component.hide().unwrap();
}
