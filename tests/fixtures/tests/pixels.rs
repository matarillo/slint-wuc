// SPDX-License-Identifier: MIT OR Apache-2.0

//! Pixel tests: what the compositor draws for a window's root visual.
//!
//! The window is detached, so nothing is shown on the desktop, but capturing needs a desktop
//! session with DWM and a hardware D3D11 device (see `docs/design/spike-s3-verification.md`).
//! The sequence is the one the spike settled on: run the sync pass, wait for the compositor to
//! commit, then capture.

use i_slint_core::api::PhysicalSize;
use i_slint_core::{Brush, Color};
use slint_fork_fixtures::capture::{Capturer, Image, wait_commit};
use slint_fork_fixtures::nested_rects::Rect;
use slint_fork_fixtures::{Headless, NestedRects, ResizeFollower};
use vtable::VRc;
use windows_numerics::Vector2;

/// What the compositor draws for the window, `width` x `height` pixels at scale factor 1.
fn capture(headless: &Headless, capturer: &Capturer, width: u32, height: u32) -> Image {
    headless.sync();
    // The root visual has no size of its own: a window target would give it the client area.
    let root = headless.root_visual();
    root.SetSize(Vector2 { X: width as f32, Y: height as f32 }).unwrap();
    wait_commit(&headless.compositor()).unwrap();
    let image = capturer.capture(&root).unwrap();
    assert_eq!((image.width, image.height), (width, height));
    image
}

/// `[r, g, b, a]` of `color`.
fn rgba(color: Color) -> [u8; 4] {
    [color.red(), color.green(), color.blue(), color.alpha()]
}

#[track_caller]
fn assert_pixel(image: &Image, x: u32, y: u32, expected: [u8; 4]) {
    let actual = image.pixel(x, y);
    assert!(
        actual.iter().zip(expected).all(|(a, b)| a.abs_diff(b) <= 2),
        "pixel ({x},{y}) is {actual:?}, expected {expected:?}"
    );
}

#[test]
fn the_compositor_draws_nested_rectangles() {
    let headless = Headless::new(PhysicalSize::new(400, 300));
    let tree = NestedRects::new(headless.adapter());
    headless.show(&VRc::into_dyn(tree.clone()));
    let capturer = Capturer::new().expect("a hardware D3D11 device");

    let image = capture(&headless, &capturer, 400, 300);
    let color = |r: Rect| rgba(r.initial().1);
    // The window's own background, in the corner and between the two halves.
    assert_pixel(&image, 2, 2, rgba(slint_fork_fixtures::nested_rects::WINDOW_BACKGROUND));
    assert_pixel(&image, 200, 150, rgba(slint_fork_fixtures::nested_rects::WINDOW_BACKGROUND));
    // Where a rectangle shows through between its own children, and each child.
    assert_pixel(&image, 15, 150, color(Rect::Left));
    assert_pixel(&image, 100, 145, color(Rect::Left));
    assert_pixel(&image, 100, 80, color(Rect::LeftTop));
    assert_pixel(&image, 100, 220, color(Rect::LeftBottom));
    assert_pixel(&image, 215, 150, color(Rect::Right));
    assert_pixel(&image, 300, 150, color(Rect::RightInner));
    // Edges: the first pixel inside and the first one outside a nested rectangle (LeftTop is
    // at 20,20 in the window, 160x120).
    assert_pixel(&image, 20, 20, color(Rect::LeftTop));
    assert_pixel(&image, 179, 139, color(Rect::LeftTop));
    assert_pixel(&image, 19, 20, color(Rect::Left));
    assert_pixel(&image, 180, 20, color(Rect::Left));
    assert_pixel(&image, 20, 140, color(Rect::Left));

    // A changed color shows after the next pass.
    tree.as_pin_ref()
        .set_background(Rect::LeftTop, Brush::SolidColor(Color::from_rgb_u8(255, 0, 0)));
    let image = capture(&headless, &capturer, 400, 300);
    assert_pixel(&image, 100, 80, [255, 0, 0, 255]);
    assert_pixel(&image, 100, 220, color(Rect::LeftBottom));

    // A rectangle that turns transparent shows what's behind it, and its children stay.
    tree.as_pin_ref()
        .set_background(Rect::Left, Brush::SolidColor(Color::from_argb_u8(0, 0, 0, 0)));
    let image = capture(&headless, &capturer, 400, 300);
    assert_pixel(&image, 15, 150, rgba(slint_fork_fixtures::nested_rects::WINDOW_BACKGROUND));
    assert_pixel(&image, 100, 80, [255, 0, 0, 255]);

    // A moved rectangle takes its children along.
    tree.as_pin_ref().set_geometry(Rect::Right, slint_fork_fixtures::rect(210., 150., 180., 140.));
    let image = capture(&headless, &capturer, 400, 300);
    assert_pixel(&image, 215, 50, rgba(slint_fork_fixtures::nested_rects::WINDOW_BACKGROUND));
    assert_pixel(&image, 215, 200, color(Rect::Right));
    // RightInner is at 20,20 inside Right, so at 230,170 in the window.
    assert_pixel(&image, 300, 200, color(Rect::RightInner));
    assert_pixel(&image, 300, 100, rgba(slint_fork_fixtures::nested_rects::WINDOW_BACKGROUND));
}

#[test]
fn the_compositor_draws_a_resized_follower() {
    let headless = Headless::new(PhysicalSize::new(400, 300));
    let tree = ResizeFollower::new(headless.adapter());
    headless.show(&VRc::into_dyn(tree.clone()));
    let capturer = Capturer::new().expect("a hardware D3D11 device");
    let window = rgba(slint_fork_fixtures::resize_follower::WINDOW_BACKGROUND);
    let follower = rgba(slint_fork_fixtures::resize_follower::FOLLOWER_COLOR);

    let image = capture(&headless, &capturer, 400, 300);
    assert_pixel(&image, 50, 50, window);
    assert_pixel(&image, 200, 150, follower);
    assert_pixel(&image, 99, 150, window);
    assert_pixel(&image, 100, 150, follower);
    assert_pixel(&image, 299, 150, follower);
    assert_pixel(&image, 300, 150, window);

    // The window gets wider and lower; the follower is still its middle half.
    headless.window().set_size(PhysicalSize::new(600, 200));
    let image = capture(&headless, &capturer, 600, 200);
    assert_pixel(&image, 100, 100, window);
    assert_pixel(&image, 300, 100, follower);
    assert_pixel(&image, 149, 100, window);
    assert_pixel(&image, 150, 100, follower);
    assert_pixel(&image, 449, 100, follower);
    assert_pixel(&image, 450, 100, window);
    assert_pixel(&image, 300, 49, window);
    assert_pixel(&image, 300, 50, follower);
    assert_pixel(&image, 300, 149, follower);
    assert_pixel(&image, 300, 150, window);
    // The fixed corner square on top.
    assert_pixel(&image, 10, 10, rgba(slint_fork_fixtures::resize_follower::CORNER_COLOR));
}
