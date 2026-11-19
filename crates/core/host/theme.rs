// Copyright © Kentaro Inomata
// SPDX-License-Identifier: MIT-0

//! The system's color scheme and accent color, which the context hands to the styles, as
//! upstream's winit backend reports them on Windows.

use crate::graphics::Color;
use crate::items::ColorScheme;
use crate::window::WindowInner;
use core::cell::Cell;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DWMWA_USE_IMMERSIVE_DARK_MODE, DwmGetColorizationColor, DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{COLOR_HIGHLIGHT, GetSysColor};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
use windows::Win32::UI::WindowsAndMessaging::{
    DefWindowProcW, GetForegroundWindow, SPI_GETHIGHCONTRAST, SWP_FRAMECHANGED, SWP_NOACTIVATE,
    SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SetWindowPos,
    SystemParametersInfoW, WM_NCACTIVATE,
};
use windows::core::{BOOL, w};

/// Reports the system's color scheme and accent color to the window's context, and gives
/// the window's title bar the color scheme. The context ignores values that haven't changed.
/// Called when the window is created and when a system setting changes; `dark_title_bar`
/// keeps what the title bar has.
pub(super) fn apply(window: &crate::api::Window, hwnd: HWND, dark_title_bar: &Cell<Option<bool>>) {
    let scheme = color_scheme();
    let context = WindowInner::from_pub(window).context();
    context.set_color_scheme(scheme);
    context.set_accent_color(accent_color());

    let dark = scheme == ColorScheme::Dark;
    let previous = dark_title_bar.replace(Some(dark));
    if previous == Some(dark) {
        return;
    }
    let value = BOOL::from(dark);
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &value as *const BOOL as *const _,
            size_of::<BOOL>() as u32,
        );
    }
    if previous.is_some() {
        // A shown window keeps the old title bar until it's next activated: draw it again,
        // in the state it's in.
        unsafe {
            let active = GetForegroundWindow() == hwnd;
            DefWindowProcW(hwnd, WM_NCACTIVATE, WPARAM(active as usize), LPARAM(0));
            let _ = SetWindowPos(
                hwnd,
                None,
                0,
                0,
                0,
                0,
                SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }
}

/// Dark when the apps' mode is dark, unless a high contrast theme is on, as winit decides.
fn color_scheme() -> ColorScheme {
    if apps_use_dark_mode() && !high_contrast() { ColorScheme::Dark } else { ColorScheme::Light }
}

/// The "Choose your app mode" setting.
fn apps_use_dark_mode() -> bool {
    let mut light = 1u32;
    let mut size = size_of::<u32>() as u32;
    let result = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut light as *mut u32 as *mut _),
            Some(&mut size),
        )
    };
    result.is_ok() && light == 0
}

fn high_contrast() -> bool {
    let mut hc = HIGHCONTRASTW { cbSize: size_of::<HIGHCONTRASTW>() as u32, ..Default::default() };
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            hc.cbSize,
            Some(&mut hc as *mut HIGHCONTRASTW as *mut _),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    ok.is_ok() && hc.dwFlags.contains(HCF_HIGHCONTRASTON)
}

/// The DWM colorization color, or the selection highlight color without it.
fn accent_color() -> Color {
    let mut argb = 0u32;
    let mut opaque_blend = BOOL::default();
    if unsafe { DwmGetColorizationColor(&mut argb, &mut opaque_blend) }.is_ok() {
        let [b, g, r, a] = argb.to_le_bytes();
        return Color::from_argb_u8(a, r, g, b);
    }
    let [r, g, b, _] = unsafe { GetSysColor(COLOR_HIGHLIGHT) }.to_le_bytes();
    Color::from_argb_u8(255, r, g, b)
}
