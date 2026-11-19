// Copyright © Kentaro Inomata
// SPDX-License-Identifier: MIT-0

//! Renders the screenshot cases of upstream Slint with `Window::take_snapshot()` and compares
//! them with the references in `references/`. The tests that `build.rs` generates call [`init`]
//! and [`check`].
//!
//! - `SLINT_CREATE_SCREENSHOTS=1` writes the reference of a case that is missing or differs. The
//!   test still fails, so that a stale or missing reference can't pass unnoticed.
//! - `SLINT_SCREENSHOT_REPORT=1` compares the snapshots with the references of upstream, appends
//!   a line per case to `target/screenshots/report.tsv`, and writes a side-by-side image
//!   `target/screenshots/<case>-cmp.png`. It never fails a test.
//!
//! The snapshot of a case that fails is saved as `target/screenshots/<case>.png`.

use image::{ColorType, Rgba, RgbaImage};
use slint::{ComponentHandle, PhysicalSize, Rgba8Pixel, SharedPixelBuffer};
use std::io::Write;
use std::path::{Path, PathBuf};

/// The smallest threshold that fails a case: a difference below it is noise of the compositor.
const MIN_THRESHOLD: f32 = 3.0;

/// How many pixels of the side-by-side image show a pixel of a snapshot.
const ZOOM: u32 = 4;

/// The references of upstream to compare with, in the order of preference. The text cases
/// have none for Skia, and Skia renders the cases that have a scale factor at scale factor 1.
const UPSTREAM_REFERENCES: &[&str] =
    &["skia", "software", "software_embed_assets", "vello_cpu", "femtovg"];

/// What `build.rs` knows about a case.
pub struct Case {
    /// The path below upstream's `tests/screenshots/cases`, without extension: `basic/border`.
    pub name: &'static str,
    /// The size of the window in physical pixels, the `SIZE=` marker of the case. Upstream's
    /// Skia driver renders a case at the size its window declares, as does this crate without it.
    pub size: Option<(u32, u32)>,
    /// The `BASE_THRESHOLD=` marker of the case, or 0.
    pub base_threshold: f32,
}

/// Starts the headless platform. Each test calls it first, on its own thread.
pub fn init() {
    i_slint_backend_testing::init_no_event_loop();
    i_slint_backend_testing::configure_test_fonts();
    // The references of upstream render the OS-dependent parts, like the text cursor, as on Linux.
    i_slint_core::OPERATING_SYSTEM_OVERRIDE
        .set(Some(i_slint_core::items::OperatingSystemType::Linux));
}

/// Renders `component` at the size of `case`, and panics when the snapshot differs from the
/// reference of the case.
pub fn check(case: &Case, component: &impl ComponentHandle) {
    if let Some((width, height)) = case.size {
        component.window().set_size(PhysicalSize::new(width, height));
    }
    component.show().unwrap();
    let snapshot = component
        .window()
        .take_snapshot()
        .unwrap_or_else(|e| panic!("{}: no snapshot: {e}", case.name));
    component.hide().unwrap();
    if let Some(size) = case.size {
        assert_eq!(
            (snapshot.width(), snapshot.height()),
            size,
            "{}: the snapshot doesn't have the size of the case",
            case.name
        );
    }

    if env_is_set("SLINT_SCREENSHOT_REPORT") {
        report(case, &snapshot);
    }

    let reference_path =
        case_path(&Path::new(env!("CARGO_MANIFEST_DIR")).join("references"), case, ".png");
    if let Err(message) = compare_with_reference(&reference_path, case, &snapshot) {
        save(&output_path(case, ".png"), &snapshot);
        if env_is_set("SLINT_CREATE_SCREENSHOTS") {
            save(&reference_path, &snapshot);
            eprintln!("{}: wrote the reference {}", case.name, reference_path.display());
        }
        panic!("{}: {message}", case.name);
    }
}

fn env_is_set(name: &str) -> bool {
    std::env::var(name).is_ok_and(|value| value == "1")
}

/// The file of `case` below `root`, with the separators of the platform.
fn case_path(root: &Path, case: &Case, suffix: &str) -> PathBuf {
    let mut path = root.to_path_buf();
    path.extend(case.name.split('/'));
    let mut file = path.into_os_string();
    file.push(suffix);
    file.into()
}

/// Where the files of a run go: below the target directory of the build.
fn output_path(case: &Case, suffix: &str) -> PathBuf {
    case_path(&Path::new(env!("SLINT_SCREENSHOTS_TARGET_DIR")).join("screenshots"), case, suffix)
}

fn save(path: &Path, snapshot: &SharedPixelBuffer<Rgba8Pixel>) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    image::save_buffer(
        path,
        snapshot.as_bytes(),
        snapshot.width(),
        snapshot.height(),
        ColorType::Rgba8,
    )
    .unwrap();
}

fn color_difference(a: &Rgba<u8>, b: &Rgba8Pixel) -> f32 {
    ((a[0] as f32 - b.r as f32).powi(2)
        + (a[1] as f32 - b.g as f32).powi(2)
        + (a[2] as f32 - b.b as f32).powi(2)
        + (a[3] as f32 - b.a as f32).powi(2))
    .sqrt()
}

fn load(path: &Path) -> Result<RgbaImage, String> {
    image::open(path)
        .map(|image| image.into_rgba8())
        .map_err(|e| format!("can't load the reference {}: {e}", path.display()))
}

fn compare_with_reference(
    reference_path: &Path,
    case: &Case,
    snapshot: &SharedPixelBuffer<Rgba8Pixel>,
) -> Result<(), String> {
    let reference = load(reference_path)?;
    if (reference.width(), reference.height()) != (snapshot.width(), snapshot.height()) {
        return Err(format!(
            "the snapshot is {}x{}, the reference {}x{}",
            snapshot.width(),
            snapshot.height(),
            reference.width(),
            reference.height()
        ));
    }
    let (mut differing, mut max) = (0usize, 0f32);
    for (a, b) in reference.pixels().zip(snapshot.as_slice()) {
        let difference = color_difference(a, b);
        differing += (difference > 0.) as usize;
        max = max.max(difference);
    }
    let threshold = case.base_threshold.max(MIN_THRESHOLD);
    if max < threshold {
        return Ok(());
    }
    let pixels = reference.width() as usize * reference.height() as usize;
    Err(format!(
        "the maximum color difference is {max:.1}, the threshold {threshold}; {:.2}% of the pixels differ; the snapshot is {}",
        differing as f32 * 100. / pixels as f32,
        output_path(case, ".png").display()
    ))
}

/// Compares the snapshot with the reference of upstream, appends a line to the report, and
/// writes the side-by-side image. The columns are the case, the size, the maximum color
/// difference, the percentages of pixels that differ by more than 4, 16 and 64, and the
/// renderer of the reference.
fn report(case: &Case, snapshot: &SharedPixelBuffer<Rgba8Pixel>) {
    save(&output_path(case, ".png"), snapshot);
    let (width, height) = (snapshot.width(), snapshot.height());
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../reference/slint-1.19.0/tests/screenshots/references");
    let candidates: Vec<_> = UPSTREAM_REFERENCES
        .iter()
        .filter_map(|renderer| {
            let path = case_path(&root.join(renderer), case, ".png");
            path.exists().then_some((*renderer, path))
        })
        .collect();
    // A reference of another size can't be compared: prefer one of the size of the snapshot.
    let found = candidates
        .iter()
        .find(|(_, path)| image::image_dimensions(path).is_ok_and(|size| size == (width, height)))
        .or(candidates.first());
    let line = match found {
        None => format!("{}\t{width}x{height}\t-\t-\t-\t-\tno reference\n", case.name),
        Some((renderer, path)) => {
            let reference = load(path).unwrap();
            if (reference.width(), reference.height()) != (width, height) {
                format!(
                    "{}\t{width}x{height}\t-\t-\t-\t-\t{renderer} {}x{}\n",
                    case.name,
                    reference.width(),
                    reference.height()
                )
            } else {
                let mut max = 0f32;
                let mut over = [0usize; 3];
                for (a, b) in reference.pixels().zip(snapshot.as_slice()) {
                    let difference = color_difference(a, b);
                    max = max.max(difference);
                    for (count, limit) in over.iter_mut().zip([4., 16., 64.]) {
                        *count += (difference > limit) as usize;
                    }
                }
                side_by_side(&reference, snapshot).save(output_path(case, "-cmp.png")).unwrap();
                let percent = |count: usize| count as f32 * 100. / (width * height) as f32;
                format!(
                    "{}\t{width}x{height}\t{max:.1}\t{:.2}\t{:.2}\t{:.2}\t{renderer}\n",
                    case.name,
                    percent(over[0]),
                    percent(over[1]),
                    percent(over[2])
                )
            }
        }
    };
    let report_path =
        Path::new(env!("SLINT_SCREENSHOTS_TARGET_DIR")).join("screenshots/report.tsv");
    std::fs::create_dir_all(report_path.parent().unwrap()).unwrap();
    // One write per line, so that the tests of a binary don't interleave.
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(report_path)
        .unwrap()
        .write_all(line.as_bytes())
        .unwrap();
}

/// The reference, the snapshot, and four times their difference, magnified.
fn side_by_side(reference: &RgbaImage, snapshot: &SharedPixelBuffer<Rgba8Pixel>) -> RgbaImage {
    let (w, h) = (snapshot.width(), snapshot.height());
    let mut side = RgbaImage::new(w * 3 * ZOOM, h * ZOOM);
    for (i, (a, b)) in reference.pixels().zip(snapshot.as_slice()).enumerate() {
        let (x, y) = (i as u32 % w, i as u32 / w);
        let b = Rgba([b.r, b.g, b.b, b.a]);
        let difference = Rgba([
            a[0].abs_diff(b[0]).saturating_mul(4),
            a[1].abs_diff(b[1]).saturating_mul(4),
            a[2].abs_diff(b[2]).saturating_mul(4),
            255,
        ]);
        for (k, pixel) in [*a, b, difference].into_iter().enumerate() {
            for dy in 0..ZOOM {
                for dx in 0..ZOOM {
                    side.put_pixel((k as u32 * w + x) * ZOOM + dx, y * ZOOM + dy, pixel);
                }
            }
        }
    }
    side
}
