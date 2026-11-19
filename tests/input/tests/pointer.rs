// Copyright © Kentaro Inomata
// SPDX-License-Identifier: MIT-0

//! Touch and pen messages for the hidden HWND of a window, and what reaches its touch area.
//!
//! Windows only reports touch and pen pointers that exist, and making them exist would act on
//! the real screen. So the test hands the host what `GetPointerInfo` would return, through
//! `Win32WindowAdapter::pointer_message_for_testing`; the rest of the path is the real one.
//!
//! The platform belongs to the thread that first uses it, so one test runs every scenario.

use i_slint_core::host::Win32WindowAdapter;
use i_slint_core::host::pointer::{PointerKind, PointerMessage};
use slint::ComponentHandle;
use std::rc::Rc;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::UI::WindowsAndMessaging::*;

slint::include_modules!();

struct Fixture {
    ui: MouseTest,
    host: Rc<Win32WindowAdapter>,
    hwnd: HWND,
}

impl Fixture {
    fn new() -> Self {
        let ui = MouseTest::new().unwrap();
        let host = Win32WindowAdapter::from_window(ui.window()).unwrap();
        let hwnd = host.create_hwnd_for_testing().unwrap();
        Self { ui, host, hwnd }
    }

    /// Sends a message for pointer `id` at a logical position in the client area.
    fn pointer(&self, msg: u32, kind: PointerKind, id: u32, x: f32, y: f32) {
        self.pointer_with(msg, kind, id, x, y, false, false);
    }

    #[allow(clippy::too_many_arguments)]
    fn pointer_with(
        &self,
        msg: u32,
        kind: PointerKind,
        id: u32,
        x: f32,
        y: f32,
        canceled: bool,
        barrel: bool,
    ) {
        let scale = self.ui.window().scale_factor();
        let mut position = POINT { x: (x * scale).round() as i32, y: (y * scale).round() as i32 };
        unsafe { ClientToScreen(self.hwnd, &mut position).unwrap() };
        self.host.pointer_message_for_testing(PointerMessage {
            msg,
            kind,
            id,
            position,
            canceled,
            barrel,
        });
    }
}

fn a_tap_clicks() {
    let f = Fixture::new();
    f.pointer(WM_POINTERDOWN, PointerKind::Touch, 7, 40., 30.);
    assert!(f.ui.get_pressed());
    f.pointer(WM_POINTERUPDATE, PointerKind::Touch, 7, 42., 31.);
    f.pointer(WM_POINTERUP, PointerKind::Touch, 7, 42., 31.);
    assert!(!f.ui.get_pressed());
    assert_eq!(f.ui.get_clicks(), 1);
}

/// A cancelled touch ends the press. Whether it clicks is the core's decision: like
/// upstream, it treats a cancelled single finger as a release.
fn a_cancelled_touch_ends_the_press() {
    let f = Fixture::new();
    f.pointer(WM_POINTERDOWN, PointerKind::Touch, 7, 40., 30.);
    // The system cancels the contact, for example as a palm.
    f.pointer_with(WM_POINTERUP, PointerKind::Touch, 7, 40., 30., true, false);
    assert!(!f.ui.get_pressed());

    // Another window takes the pointer.
    f.pointer(WM_POINTERDOWN, PointerKind::Touch, 8, 40., 30.);
    assert!(f.ui.get_pressed());
    f.pointer(WM_POINTERCAPTURECHANGED, PointerKind::Touch, 8, 0., 0.);
    assert!(!f.ui.get_pressed());
    // Its lift no longer belongs to this window.
    let clicks = f.ui.get_clicks();
    f.pointer(WM_POINTERUP, PointerKind::Touch, 8, 40., 30.);
    assert_eq!(f.ui.get_clicks(), clicks);
}

fn updates_of_unknown_fingers_are_ignored() {
    let f = Fixture::new();
    // A finger that came down on another window.
    f.pointer(WM_POINTERUPDATE, PointerKind::Touch, 9, 40., 30.);
    f.pointer(WM_POINTERUP, PointerKind::Touch, 9, 40., 30.);
    assert!(!f.ui.get_pressed());
    assert_eq!(f.ui.get_clicks(), 0);
}

fn a_pen_tap_is_a_left_click() {
    let f = Fixture::new();
    // Hovering above the screen moves the pointer.
    f.pointer(WM_POINTERUPDATE, PointerKind::Pen, 3, 40., 30.);
    assert!(f.ui.get_hover());
    f.pointer(WM_POINTERDOWN, PointerKind::Pen, 3, 40., 30.);
    assert!(f.ui.get_pressed());
    f.pointer(WM_POINTERUP, PointerKind::Pen, 3, 40., 30.);
    assert_eq!(f.ui.get_clicks(), 1);
    assert_eq!(f.ui.get_buttons(), "left ");
    // Out of range.
    f.pointer(WM_POINTERLEAVE, PointerKind::Pen, 3, 40., 30.);
    assert!(!f.ui.get_hover());
}

fn the_barrel_button_makes_a_right_click() {
    let f = Fixture::new();
    f.pointer_with(WM_POINTERDOWN, PointerKind::Pen, 3, 40., 30., false, true);
    f.pointer_with(WM_POINTERUP, PointerKind::Pen, 3, 40., 30., false, true);
    assert_eq!(f.ui.get_buttons(), "right ");
}

fn a_pen_that_loses_the_window_doesnt_click() {
    let f = Fixture::new();
    f.pointer(WM_POINTERDOWN, PointerKind::Pen, 3, 40., 30.);
    f.pointer(WM_POINTERCAPTURECHANGED, PointerKind::Pen, 3, 0., 0.);
    assert!(!f.ui.get_pressed());
    f.pointer(WM_POINTERUP, PointerKind::Pen, 3, 40., 30.);
    assert_eq!(f.ui.get_clicks(), 0);
}

#[test]
fn touch_and_pen_messages() {
    a_tap_clicks();
    a_cancelled_touch_ends_the_press();
    updates_of_unknown_fingers_are_ignored();
    a_pen_tap_is_a_left_click();
    the_barrel_button_makes_a_right_click();
    a_pen_that_loses_the_window_doesnt_click();
}
