// Copyright 2022-2022 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The dark menu bar, ported from muda 0.21.2. Windows draws a menu bar light whatever the
//! app's color scheme, so the window draws it on the undocumented `WM_UAH*` messages.

use alloc::vec::Vec;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateSolidBrush, DT_CENTER, DT_HIDEPREFIX, DT_SINGLELINE, DT_VCENTER, DeleteObject, DrawTextW,
    FillRect, GetWindowDC, HDC, MapWindowPoints, ReleaseDC, SetBkMode, SetTextColor, TRANSPARENT,
};
use windows::Win32::UI::Controls::{
    DRAWITEMSTRUCT, ODS_DISABLED, ODS_FLAGS, ODS_GRAYED, ODS_HOTLIGHT, ODS_NOACCEL, ODS_SELECTED,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClientRect, GetMenuBarInfo, GetMenuItemInfoW, GetWindowRect, HMENU, MENUBARINFO,
    MENUITEMINFOW, MIIM_STRING, OBJID_MENU, WM_NCACTIVATE, WM_NCPAINT,
};
use windows::core::PWSTR;

/// Asks to draw the background of the menu bar.
pub(super) const WM_UAHDRAWMENU: u32 = 0x0091;
/// Asks to draw one item of the menu bar.
pub(super) const WM_UAHDRAWMENUITEM: u32 = 0x0092;

const BACKGROUND: COLORREF = COLORREF(0x2b2b2b);
const HOT_BACKGROUND: COLORREF = COLORREF(0x414141);
const TEXT: COLORREF = COLORREF(0xffffff);
const DISABLED_TEXT: COLORREF = COLORREF(0x6d6d6d);

/// `UAHMENU`, which `WM_UAHDRAWMENU` passes.
#[repr(C)]
struct UahMenu {
    hmenu: HMENU,
    hdc: HDC,
    _flags: u32,
}

/// `UAHMENUITEM`: the position of the item, then the metrics (`UAHMENUITEMMETRICS` and
/// `UAHMENUPOPUPMETRICS`), which aren't read.
#[repr(C)]
struct UahMenuItem {
    position: u32,
    _metrics: [u32; 12],
    _popup_metrics: [u32; 5],
}

/// `UAHDRAWMENUITEM`, which `WM_UAHDRAWMENUITEM` passes.
#[repr(C)]
struct UahDrawMenuItem {
    item: DRAWITEMSTRUCT,
    menu: UahMenu,
    menu_item: UahMenuItem,
}

/// Draws what `msg` asks for, dark. `WM_NCACTIVATE` and `WM_NCPAINT` draw after the default
/// procedure, over the line it leaves below the menu bar.
pub(super) fn draw(hwnd: HWND, msg: u32, _wp: WPARAM, lp: LPARAM) {
    match msg {
        WM_NCACTIVATE | WM_NCPAINT => draw_line_below_bar(hwnd),
        WM_UAHDRAWMENU => {
            // SAFETY: the message passes a `UAHMENU`.
            let menu = unsafe { &*(lp.0 as *const UahMenu) };
            draw_background(hwnd, menu.hdc);
        }
        WM_UAHDRAWMENUITEM => {
            // SAFETY: the message passes a `UAHDRAWMENUITEM`.
            let item = unsafe { &*(lp.0 as *const UahDrawMenuItem) };
            draw_item(item);
        }
        _ => {}
    }
}

/// Fills the one pixel line between the menu bar and the client area.
fn draw_line_below_bar(hwnd: HWND) {
    let mut client = RECT::default();
    let mut window = RECT::default();
    if unsafe { GetClientRect(hwnd, &mut client) }.is_err()
        || unsafe { GetWindowRect(hwnd, &mut window) }.is_err()
    {
        return;
    }
    let mut corners =
        [POINT { x: client.left, y: client.top }, POINT { x: client.right, y: client.bottom }];
    unsafe { MapWindowPoints(Some(hwnd), None, &mut corners) };
    // The window's DC has its origin at the window's top left corner.
    let top = corners[0].y - window.top;
    let line = RECT {
        left: corners[0].x - window.left,
        top: top - 1,
        right: corners[1].x - window.left,
        bottom: top,
    };
    unsafe {
        let hdc = GetWindowDC(Some(hwnd));
        fill(hdc, &line, BACKGROUND);
        ReleaseDC(Some(hwnd), hdc);
    }
}

fn draw_background(hwnd: HWND, hdc: HDC) {
    let mut info = MENUBARINFO { cbSize: size_of::<MENUBARINFO>() as u32, ..Default::default() };
    let mut window = RECT::default();
    if unsafe { GetMenuBarInfo(hwnd, OBJID_MENU, 0, &mut info) }.is_err()
        || unsafe { GetWindowRect(hwnd, &mut window) }.is_err()
    {
        return;
    }
    // The bar is in screen coordinates, and the DC's origin is the window's top left corner.
    let bar = RECT {
        left: info.rcBar.left - window.left,
        top: info.rcBar.top - window.top - 1,
        right: info.rcBar.right - window.left,
        bottom: info.rcBar.bottom - window.top,
    };
    fill(hdc, &bar, BACKGROUND);
}

fn draw_item(item: &UahDrawMenuItem) {
    let Some(mut label) = label(item.menu.hmenu, item.menu_item.position) else { return };
    let state = item.item.itemState;
    let has = |flag: ODS_FLAGS| state.0 & flag.0 != 0;
    let disabled = has(ODS_GRAYED) || has(ODS_DISABLED);
    // Either the pointer is over the item, or its menu is open.
    let hot = !disabled && (has(ODS_HOTLIGHT) || has(ODS_SELECTED));
    let mut format = DT_CENTER | DT_SINGLELINE | DT_VCENTER;
    if has(ODS_NOACCEL) {
        format |= DT_HIDEPREFIX;
    }
    let hdc = item.menu.hdc;
    let mut rect = item.item.rcItem;
    fill(hdc, &rect, if hot { HOT_BACKGROUND } else { BACKGROUND });
    unsafe {
        SetBkMode(hdc, TRANSPARENT);
        SetTextColor(hdc, if disabled { DISABLED_TEXT } else { TEXT });
        DrawTextW(hdc, &mut label, &mut rect, format);
    }
}

/// The title of the item at `position` of the menu bar `menu`.
fn label(menu: HMENU, position: u32) -> Option<Vec<u16>> {
    let mut info = MENUITEMINFOW {
        cbSize: size_of::<MENUITEMINFOW>() as u32,
        fMask: MIIM_STRING,
        ..Default::default()
    };
    // Without a buffer, the call reports the length.
    unsafe { GetMenuItemInfoW(menu, position, true, &mut info) }.ok()?;
    // One more for the terminator.
    let mut label = alloc::vec![0u16; info.cch as usize + 1];
    info.cch += 1;
    info.dwTypeData = PWSTR(label.as_mut_ptr());
    unsafe { GetMenuItemInfoW(menu, position, true, &mut info) }.ok()?;
    label.truncate(info.cch as usize);
    Some(label)
}

fn fill(hdc: HDC, rect: &RECT, color: COLORREF) {
    unsafe {
        let brush = CreateSolidBrush(color);
        FillRect(hdc, rect, brush);
        let _ = DeleteObject(brush.into());
    }
}
