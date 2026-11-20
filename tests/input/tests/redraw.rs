// Copyright © Kentaro Inomata
// SPDX-License-Identifier: MIT-0

//! Text that is left as drawn when its item only changes size: after the rows of
//! `ui/redraw.slint` change width a few times, the window shows what a new window of the last
//! width shows. A row that keeps a stale drawing (a centered or elided text, a gradient that
//! spans the row) shows up as a difference.

use slint::ComponentHandle;

slint::include_modules!();

fn snapshot(ui: &RedrawTest) -> slint::SharedPixelBuffer<slint::Rgba8Pixel> {
    ui.window().take_snapshot().unwrap()
}

#[test]
fn text_resized_in_steps_looks_as_if_drawn_at_its_last_size() {
    let ui = RedrawTest::new().unwrap();
    ui.window().set_size(slint::PhysicalSize::new(320, 160));
    for width in [300., 120., 301.5, 180., 250.] {
        ui.set_row_width(width);
        snapshot(&ui);
    }
    let resized = snapshot(&ui);

    let fresh = RedrawTest::new().unwrap();
    fresh.window().set_size(slint::PhysicalSize::new(320, 160));
    fresh.set_row_width(250.);
    let expected = snapshot(&fresh);

    assert_eq!((resized.width(), resized.height()), (expected.width(), expected.height()));
    let differing = resized
        .as_slice()
        .iter()
        .zip(expected.as_slice())
        .enumerate()
        .filter(|(_, (a, b))| a != b)
        .map(|(i, _)| (i as u32 % resized.width(), i as u32 / resized.width()))
        .collect::<Vec<_>>();
    assert!(
        differing.is_empty(),
        "{} pixels differ, the first at {:?}",
        differing.len(),
        differing.first()
    );
}
