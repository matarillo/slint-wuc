// SPDX-License-Identifier: MIT OR Apache-2.0

//! Mouse messages sent to the hidden HWND of a window, and what reaches its touch area.
//!
//! `SendMessageW` runs the window procedure right away, so each check sees the result of the
//! messages before it. The window is never shown, so the real mouse can't interfere.
//!
//! The platform belongs to the thread that first uses it, so one test runs every scenario.

use i_slint_core::host::Win32WindowAdapter;
use slint::ComponentHandle;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetCapture, ReleaseCapture};
use windows::Win32::UI::WindowsAndMessaging::*;

slint::include_modules!();

const WM_MOUSELEAVE: u32 = 0x02A3;
/// `wParam` of a mouse message while the left button is down.
const MK_LBUTTON: usize = 0x0001;

struct Fixture {
    ui: MouseTest,
    hwnd: HWND,
}

impl Fixture {
    fn new() -> Self {
        let ui = MouseTest::new().unwrap();
        let hwnd = Win32WindowAdapter::from_window(ui.window())
            .unwrap()
            .create_hwnd_for_testing()
            .unwrap();
        Self { ui, hwnd }
    }

    /// The client pixel of a logical position.
    fn physical(&self, x: f32, y: f32) -> POINT {
        let scale = self.ui.window().scale_factor();
        POINT { x: (x * scale).round() as i32, y: (y * scale).round() as i32 }
    }

    fn send(&self, msg: u32, wp: usize, lp: isize) -> LRESULT {
        unsafe { SendMessageW(self.hwnd, msg, Some(WPARAM(wp)), Some(LPARAM(lp))) }
    }

    /// Sends a mouse message at a logical position in the client area.
    fn mouse(&self, msg: u32, wp: usize, x: f32, y: f32) -> LRESULT {
        self.send(msg, wp, pack(self.physical(x, y)))
    }

    /// Sends a wheel message, whose position is on the screen.
    fn wheel(&self, msg: u32, delta: i16, x: f32, y: f32) {
        let mut point = self.physical(x, y);
        unsafe { ClientToScreen(self.hwnd, &mut point).unwrap() };
        self.send(msg, (delta as u16 as usize) << 16, pack(point));
    }
}

fn pack(point: POINT) -> isize {
    ((point.y as u16 as isize) << 16) | point.x as u16 as isize
}

fn a_click_reaches_the_touch_area() {
    let f = Fixture::new();
    f.mouse(WM_MOUSEMOVE, 0, 40., 30.);
    assert!(f.ui.get_hover());
    // Positions are logical, rounded to the client pixel they came from.
    assert!((f.ui.get_mouse_x() - 40.).abs() < 1.);
    assert!((f.ui.get_mouse_y() - 30.).abs() < 1.);

    f.mouse(WM_LBUTTONDOWN, MK_LBUTTON, 40., 30.);
    assert!(f.ui.get_pressed());
    assert_eq!(unsafe { GetCapture() }, f.hwnd);

    f.mouse(WM_LBUTTONUP, 0, 40., 30.);
    assert!(!f.ui.get_pressed());
    assert_eq!(f.ui.get_clicks(), 1);
    assert_eq!(unsafe { GetCapture() }, HWND::default());
}

fn a_release_outside_after_dragging_out_is_no_click() {
    let f = Fixture::new();
    f.mouse(WM_MOUSEMOVE, 0, 40., 30.);
    f.mouse(WM_LBUTTONDOWN, MK_LBUTTON, 40., 30.);
    // The window keeps the mouse while the button is down, so it sees the moves outside.
    f.mouse(WM_MOUSEMOVE, MK_LBUTTON, 150., 150.);
    f.mouse(WM_LBUTTONUP, 0, 150., 150.);
    assert_eq!(f.ui.get_clicks(), 0);
    assert!(!f.ui.get_pressed());
}

fn losing_the_capture_cancels_the_press() {
    let f = Fixture::new();
    f.mouse(WM_MOUSEMOVE, 0, 40., 30.);
    f.mouse(WM_LBUTTONDOWN, MK_LBUTTON, 40., 30.);
    assert!(f.ui.get_pressed());
    // As when another window takes the mouse: WM_CAPTURECHANGED, and no button up.
    unsafe { ReleaseCapture().unwrap() };
    assert!(!f.ui.get_pressed());
    assert!(!f.ui.get_hover());

    // The button up that may still come doesn't count as a click.
    f.mouse(WM_LBUTTONUP, 0, 40., 30.);
    assert_eq!(f.ui.get_clicks(), 0);
}

fn leaving_the_window_ends_the_hover() {
    let f = Fixture::new();
    f.mouse(WM_MOUSEMOVE, 0, 40., 30.);
    assert!(f.ui.get_hover());
    f.send(WM_MOUSELEAVE, 0, 0);
    assert!(!f.ui.get_hover());
}

fn leaving_while_a_button_is_down_waits_for_the_release() {
    let f = Fixture::new();
    f.mouse(WM_MOUSEMOVE, 0, 40., 30.);
    f.mouse(WM_LBUTTONDOWN, MK_LBUTTON, 40., 30.);
    f.send(WM_MOUSELEAVE, 0, 0);
    assert!(f.ui.get_pressed());
    f.mouse(WM_LBUTTONUP, 0, 40., 30.);
    assert_eq!(f.ui.get_clicks(), 1);
    assert!(!f.ui.get_hover());
}

fn each_button_reaches_the_touch_area() {
    let f = Fixture::new();
    f.mouse(WM_MOUSEMOVE, 0, 40., 30.);
    for (down, up, wp) in [
        (WM_LBUTTONDOWN, WM_LBUTTONUP, 0),
        (WM_RBUTTONDOWN, WM_RBUTTONUP, 0),
        (WM_MBUTTONDOWN, WM_MBUTTONUP, 0),
        (WM_XBUTTONDOWN, WM_XBUTTONUP, (XBUTTON1 as usize) << 16),
        (WM_XBUTTONDOWN, WM_XBUTTONUP, (XBUTTON2 as usize) << 16),
    ] {
        let result = f.mouse(down, wp, 40., 30.);
        // The X button messages return TRUE.
        assert_eq!(result, LRESULT((down == WM_XBUTTONDOWN) as isize));
        f.mouse(up, wp, 40., 30.);
    }
    assert_eq!(f.ui.get_buttons(), "left right middle back forward ");
}

fn a_wheel_notch_scrolls_sixty_pixels() {
    let f = Fixture::new();
    f.mouse(WM_MOUSEMOVE, 0, 40., 30.);
    // Up and away from the user.
    f.wheel(WM_MOUSEWHEEL, WHEEL_DELTA as i16, 40., 30.);
    assert_eq!((f.ui.get_scroll_x(), f.ui.get_scroll_y()), (0., 60.));
    // Tilted to the right: winit's sign, inverted.
    f.wheel(WM_MOUSEHWHEEL, WHEEL_DELTA as i16, 40., 30.);
    assert_eq!((f.ui.get_scroll_x(), f.ui.get_scroll_y()), (-60., 60.));
    // A precision touchpad sends fractions of a notch.
    f.wheel(WM_MOUSEWHEEL, -30, 40., 30.);
    assert_eq!((f.ui.get_scroll_x(), f.ui.get_scroll_y()), (-60., 45.));
    // Outside the touch area, the wheel doesn't reach it.
    f.wheel(WM_MOUSEWHEEL, WHEEL_DELTA as i16, 150., 150.);
    assert_eq!(f.ui.get_scroll_y(), 45.);
}

fn the_cursor_follows_the_touch_area() {
    let f = Fixture::new();
    let system = |id| unsafe { LoadCursorW(None, id).unwrap() };
    let set_cursor = || f.send(WM_SETCURSOR, f.hwnd.0 as usize, HTCLIENT as isize);

    f.mouse(WM_MOUSEMOVE, 0, 40., 30.);
    assert_eq!(set_cursor(), LRESULT(1));
    assert_eq!(unsafe { GetCursor() }, system(IDC_HAND));

    f.mouse(WM_MOUSEMOVE, 0, 150., 150.);
    assert_eq!(unsafe { GetCursor() }, system(IDC_ARROW));
    set_cursor();
    assert_eq!(unsafe { GetCursor() }, system(IDC_ARROW));
}

/// The system's move loop would wait for the real mouse button, so this only checks that the
/// drag asks for it, after the dispatch of the move that started it.
fn dragging_a_window_move_area_starts_moving_the_window() {
    const WM_APP_WINDOW_MOVE: u32 = WM_APP + 5;
    let f = Fixture::new();
    let pending = || {
        let mut msg = MSG::default();
        unsafe {
            PeekMessageW(&mut msg, Some(f.hwnd), WM_APP_WINDOW_MOVE, WM_APP_WINDOW_MOVE, PM_REMOVE)
        }
        .as_bool()
    };
    f.mouse(WM_MOUSEMOVE, 0, 150., 150.);
    f.mouse(WM_LBUTTONDOWN, MK_LBUTTON, 150., 150.);
    f.mouse(WM_MOUSEMOVE, MK_LBUTTON, 151., 150.);
    assert!(!pending(), "a move within the drag threshold");
    f.mouse(WM_MOUSEMOVE, MK_LBUTTON, 170., 150.);
    assert!(pending());
    f.mouse(WM_LBUTTONUP, 0, 170., 150.);

    // Over the touch area, a drag is no window move.
    f.mouse(WM_MOUSEMOVE, 0, 40., 30.);
    f.mouse(WM_LBUTTONDOWN, MK_LBUTTON, 40., 30.);
    f.mouse(WM_MOUSEMOVE, MK_LBUTTON, 80., 30.);
    f.mouse(WM_LBUTTONUP, 0, 80., 30.);
    assert!(!pending());
}

#[test]
fn mouse_messages() {
    a_click_reaches_the_touch_area();
    a_release_outside_after_dragging_out_is_no_click();
    losing_the_capture_cancels_the_press();
    leaving_the_window_ends_the_hover();
    leaving_while_a_button_is_down_waits_for_the_release();
    each_button_reaches_the_touch_area();
    a_wheel_notch_scrolls_sixty_pixels();
    the_cursor_follows_the_touch_area();
    dragging_a_window_move_area_starts_moving_the_window();
}
