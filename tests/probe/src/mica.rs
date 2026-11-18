// SPDX-License-Identifier: MIT OR Apache-2.0

//! Shows `ui/mica.slint` with the Mica system backdrop. `--menu` shows the window with a
//! `MenuBar`; `--no-mica` leaves the backdrop unset, for comparison.

slint::include_modules!();

use i_slint_core::host::Win32WindowAdapter;
use slint::ComponentHandle;
use windows::Win32::Graphics::Dwm::{
    DWM_SYSTEMBACKDROP_TYPE, DWMSBT_MAINWINDOW, DWMWA_SYSTEMBACKDROP_TYPE, DwmSetWindowAttribute,
};

/// Sets the backdrop as an application would, through the window's HWND.
fn set_mica(window: &slint::Window) {
    let host = Win32WindowAdapter::from_window(window).unwrap();
    let hwnd = host.create_hwnd_for_testing().unwrap();
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
        if mica {
            set_mica(app.window());
        }
        app.run().unwrap();
    } else {
        let app = MicaPlain::new().unwrap();
        if mica {
            set_mica(app.window());
        }
        app.run().unwrap();
    }
}
