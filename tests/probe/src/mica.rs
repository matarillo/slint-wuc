// Copyright © Kentaro Inomata
// SPDX-License-Identifier: MIT-0

//! Shows `ui/mica.slint` with the Mica system backdrop. `--menu` shows the window with a
//! `MenuBar`; `--no-mica` leaves the backdrop unset, for comparison.

slint::include_modules!();

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use slint::ComponentHandle;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Dwm::{
    DWM_SYSTEMBACKDROP_TYPE, DWMSBT_MAINWINDOW, DWMWA_SYSTEMBACKDROP_TYPE, DwmSetWindowAttribute,
};

/// Sets the backdrop as an application would with upstream Slint: through the HWND that
/// `Window::window_handle()` gives once `show()` has created the window.
fn set_mica(window: &slint::Window) {
    let handle = window.window_handle();
    let RawWindowHandle::Win32(win32) = handle.window_handle().unwrap().as_raw() else {
        panic!("not a Win32 window");
    };
    let hwnd = HWND(win32.hwnd.get() as _);
    let result = unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_SYSTEMBACKDROP_TYPE,
            (&DWMSBT_MAINWINDOW as *const DWM_SYSTEMBACKDROP_TYPE).cast(),
            std::mem::size_of_val(&DWMSBT_MAINWINDOW) as u32,
        )
    };
    println!("DWMWA_SYSTEMBACKDROP_TYPE: {result:?}");
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mica = !args.iter().any(|a| a == "--no-mica");
    if args.iter().any(|a| a == "--menu") {
        let app = MicaMenu::new().unwrap();
        app.show().unwrap();
        if mica {
            set_mica(app.window());
        }
        app.run().unwrap();
    } else {
        let app = MicaPlain::new().unwrap();
        app.show().unwrap();
        if mica {
            set_mica(app.window());
        }
        app.run().unwrap();
    }
}
