// Copyright © Kentaro Inomata
// SPDX-License-Identifier: MIT-0

//! The frame and state of a window, from the properties of its `Window` item, and what the
//! window reports back when the user changes its state.
//!
//! The window is never shown, so showing states such as maximized waits, and the tests send
//! the messages that Windows sends when the user changes them.
//!
//! The platform belongs to the thread that first uses it, so one test runs every scenario.

use i_slint_core::host::Win32WindowAdapter;
use i_slint_core::window::WindowInner;
use slint::ComponentHandle;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::UI::WindowsAndMessaging::*;

slint::include_modules!();

struct Fixture {
    ui: WindowTest,
    hwnd: HWND,
}

impl Fixture {
    fn new() -> Self {
        let ui = WindowTest::new().unwrap();
        let hwnd = Win32WindowAdapter::from_window(ui.window())
            .unwrap()
            .create_hwnd_for_testing()
            .unwrap();
        let f = Self { ui, hwnd };
        f.update();
        f
    }

    /// Hands the properties to the window, as the core does from a timer once they change.
    fn update(&self) {
        WindowInner::from_pub(self.ui.window()).update_window_properties();
    }

    fn style(&self) -> WINDOW_STYLE {
        WINDOW_STYLE(unsafe { GetWindowLongW(self.hwnd, GWL_STYLE) } as u32)
    }

    fn ex_style(&self) -> WINDOW_EX_STYLE {
        WINDOW_EX_STYLE(unsafe { GetWindowLongW(self.hwnd, GWL_EXSTYLE) } as u32)
    }

    fn send(&self, msg: u32, wp: usize, lp: isize) -> LRESULT {
        unsafe { SendMessageW(self.hwnd, msg, Some(WPARAM(wp)), Some(LPARAM(lp))) }
    }

    /// The hit test at a client pixel.
    fn hit_test(&self, x: i32, y: i32) -> u32 {
        let mut point = POINT { x, y };
        unsafe { ClientToScreen(self.hwnd, &mut point).unwrap() };
        let lp = ((point.y as u16 as isize) << 16) | point.x as u16 as isize;
        self.send(WM_NCHITTEST, 0, lp).0 as u32
    }
}

fn client_and_window_size(hwnd: HWND) -> ((i32, i32), (i32, i32)) {
    let (mut client, mut window) = (RECT::default(), RECT::default());
    unsafe {
        GetClientRect(hwnd, &mut client).unwrap();
        GetWindowRect(hwnd, &mut window).unwrap();
    }
    ((client.right, client.bottom), (window.right - window.left, window.bottom - window.top))
}

fn a_window_has_a_frame_by_default() {
    let f = Fixture::new();
    assert!(f.style().contains(WS_CAPTION | WS_THICKFRAME | WS_MAXIMIZEBOX));
    assert!(!f.ex_style().contains(WS_EX_TOPMOST));
    let (client, window) = client_and_window_size(f.hwnd);
    assert!(window.0 > client.0 && window.1 > client.1);
}

fn a_frameless_window_is_all_client_area() {
    let f = Fixture::new();
    let size = f.ui.window().size();
    f.ui.set_frameless(true);
    f.update();
    assert!(f.style().contains(WS_POPUP));
    assert!(!f.style().contains(WS_CAPTION));
    let (client, window) = client_and_window_size(f.hwnd);
    assert_eq!(client, window);
    // The window keeps the size of its contents.
    assert_eq!((client.0 as u32, client.1 as u32), (size.width, size.height));

    // Its border resizes it, `resize-border-width` wide.
    let border = (5. * f.ui.window().scale_factor()).round() as i32;
    assert_eq!(f.hit_test(1, client.1 / 2), HTLEFT);
    assert_eq!(f.hit_test(client.0 - 1, client.1 - 1), HTBOTTOMRIGHT);
    assert_eq!(f.hit_test(border + 1, border + 1), HTCLIENT);

    f.ui.set_frameless(false);
    f.update();
    assert!(f.style().contains(WS_CAPTION));
    let (client, _) = client_and_window_size(f.hwnd);
    assert_eq!((client.0 as u32, client.1 as u32), (size.width, size.height));
}

fn a_fixed_size_window_cant_be_resized_or_maximized() {
    let f = Fixture::new();
    f.ui.set_fixed_size(true);
    f.update();
    assert!(!f.style().contains(WS_THICKFRAME));
    assert!(!f.style().contains(WS_MAXIMIZEBOX));
    f.ui.set_fixed_size(false);
    f.update();
    assert!(f.style().contains(WS_THICKFRAME | WS_MAXIMIZEBOX));
}

fn always_on_top_makes_the_window_topmost() {
    // Windows doesn't always make a window of a background process topmost later, so the
    // window starts topmost.
    let ui = WindowTest::new().unwrap();
    ui.set_on_top(true);
    WindowInner::from_pub(ui.window()).update_window_properties();
    let hwnd =
        Win32WindowAdapter::from_window(ui.window()).unwrap().create_hwnd_for_testing().unwrap();
    let f = Fixture { ui, hwnd };
    assert!(f.ex_style().contains(WS_EX_TOPMOST));
    f.ui.set_on_top(false);
    f.update();
    assert!(!f.ex_style().contains(WS_EX_TOPMOST));
}

fn the_icon_shows_in_the_window() {
    let f = Fixture::new();
    let icon = |kind: u32| f.send(WM_GETICON, kind as usize, 0).0;
    assert_eq!(icon(ICON_BIG), 0);
    f.ui.set_with_icon(true);
    f.update();
    assert_ne!(icon(ICON_BIG), 0);
    assert_ne!(icon(ICON_SMALL), 0);
    f.ui.set_with_icon(false);
    f.update();
    assert_eq!(icon(ICON_BIG), 0);
}

fn the_item_follows_what_the_user_does_to_the_window() {
    let f = Fixture::new();
    // A hidden window waits until it's shown.
    f.ui.set_is_maximized(true);
    f.update();
    assert!(!unsafe { IsWindowVisible(f.hwnd) }.as_bool());
    assert!(!unsafe { IsZoomed(f.hwnd) }.as_bool());

    f.send(WM_SIZE, SIZE_RESTORED as usize, 0);
    assert!(!f.ui.get_is_maximized());
    f.send(WM_SIZE, SIZE_MAXIMIZED as usize, 0);
    assert!(f.ui.get_is_maximized());
    // Minimized, the window stays maximized, to come back maximized.
    f.send(WM_SIZE, SIZE_MINIMIZED as usize, 0);
    assert!(f.ui.get_is_minimized());
    assert!(f.ui.get_is_maximized());
    f.send(WM_SIZE, SIZE_MAXIMIZED as usize, 0);
    assert!(!f.ui.get_is_minimized());
    assert!(f.ui.get_is_maximized());
}

#[test]
fn window_properties() {
    a_window_has_a_frame_by_default();
    a_frameless_window_is_all_client_area();
    a_fixed_size_window_cant_be_resized_or_maximized();
    always_on_top_makes_the_window_topmost();
    the_icon_shows_in_the_window();
    the_item_follows_what_the_user_does_to_the_window();
}
