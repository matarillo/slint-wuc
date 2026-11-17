// SPDX-License-Identifier: MIT OR Apache-2.0

//! Popup windows: `docs/design/windows.md`.

use super::Win32WindowAdapter;
use crate::api::LogicalPosition;
use crate::items::PopupClosePolicy;
use crate::window::{PopupWindowLocation, WindowAdapter, WindowInner, WindowKind};
use alloc::rc::{Rc, Weak};
use alloc::vec::Vec;
use core::cell::Cell;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DWM_WINDOW_CORNER_PREFERENCE, DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE,
    DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND, DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{
    ClientToScreen, GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
};
use windows::Win32::UI::WindowsAndMessaging::*;

/// What makes a window a popup.
pub(super) struct Popup {
    pub(super) parent: Weak<Win32WindowAdapter>,
    pub(super) kind: WindowKind,
    /// The position in the parent's logical client coordinates, as the core last set it.
    pub(super) position: Cell<Option<LogicalPosition>>,
}

impl Popup {
    pub(super) const STYLE: WINDOW_STYLE = WS_POPUP;
    pub(super) const EX_STYLE: WINDOW_EX_STYLE =
        WINDOW_EX_STYLE(WS_EX_NOACTIVATE.0 | WS_EX_TOOLWINDOW.0 | WS_EX_NOREDIRECTIONBITMAP.0);

    /// The top-left corner of a popup of `size` physical pixels, in screen pixels: the position
    /// the core asked for, moved into the work area of the parent's monitor.
    pub(super) fn screen_position(&self, size: (i32, i32)) -> Option<POINT> {
        let parent = self.parent.upgrade()?;
        let parent_hwnd = parent.hwnd.get()?;
        let position = self.position.get()?.to_physical(parent.window.scale_factor());
        let mut point = POINT { x: position.x, y: position.y };
        unsafe {
            let _ = ClientToScreen(parent_hwnd, &mut point);
            let monitor = MonitorFromWindow(parent_hwnd, MONITOR_DEFAULTTONEAREST);
            let mut info =
                MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
            if GetMonitorInfoW(monitor, &mut info).as_bool() {
                point = keep_inside(point, size, info.rcWork);
            }
        }
        Some(point)
    }
}

/// `point` moved so that a rectangle of `size` there lies inside `area`, or starts at its
/// top-left corner when it's larger.
fn keep_inside(point: POINT, (width, height): (i32, i32), area: RECT) -> POINT {
    POINT {
        x: point.x.min(area.right - width).max(area.left),
        y: point.y.min(area.bottom - height).max(area.top),
    }
}

/// Removes the corners and the border that Windows 11 may draw around a top-level window.
pub(super) fn created(hwnd: HWND) {
    let corners = DWMWCP_DONOTROUND;
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            (&corners as *const DWM_WINDOW_CORNER_PREFERENCE).cast(),
            size_of_val(&corners) as u32,
        );
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR,
            (&DWMWA_COLOR_NONE as *const u32).cast(),
            size_of::<u32>() as u32,
        );
    }
}

/// The messages of a popup's own window.
pub(super) fn handle(popup: &Popup, msg: u32, _wp: WPARAM, _lp: LPARAM) -> Option<LRESULT> {
    match msg {
        WM_MOUSEACTIVATE => Some(LRESULT(MA_NOACTIVATE as isize)),
        // A tooltip lets the mouse through to the window below it, which is in this thread.
        WM_NCHITTEST if matches!(popup.kind, WindowKind::ToolTip) => {
            Some(LRESULT(HTTRANSPARENT as isize))
        }
        _ => None,
    }
}

/// The messages of a window that may own popups: what dismisses them, besides the clicks and
/// keys the core handles.
pub(super) fn handle_owner(adapter: &Win32WindowAdapter, msg: u32, wp: WPARAM) {
    match msg {
        WM_ACTIVATEAPP if wp.0 == 0 => close_auto_closing(adapter),
        WM_NCLBUTTONDOWN | WM_NCRBUTTONDOWN | WM_NCMBUTTONDOWN | WM_SIZE => {
            close_auto_closing(adapter)
        }
        WM_MOVE => {
            close_auto_closing(adapter);
            for popup in native_popups(adapter) {
                popup.place_popup();
            }
        }
        _ => {}
    }
}

/// The adapters of the popups of `adapter` that have windows of their own.
fn native_popups(adapter: &Win32WindowAdapter) -> Vec<Rc<Win32WindowAdapter>> {
    adapter
        .inner()
        .active_popups()
        .iter()
        .filter_map(|p| match &p.location {
            PopupWindowLocation::TopLevel(wa) => Win32WindowAdapter::from_window(wa.window()),
            PopupWindowLocation::ChildWindow(_) => None,
        })
        .collect()
}

/// Closes the popups of `adapter` that close on a click outside them.
fn close_auto_closing(adapter: &Win32WindowAdapter) {
    let inner = adapter.inner();
    let ids: Vec<_> = inner
        .active_popups()
        .iter()
        .filter(|p| p.close_policy != PopupClosePolicy::NoAutoClose)
        .map(|p| p.popup_id)
        .collect();
    for id in ids {
        inner.close_popup(id);
    }
}

/// The window that gets the keys that arrive at `adapter`: the topmost popup with a window of
/// its own that isn't a tooltip, since the core moves the focus item there.
pub(super) fn key_window(adapter: &Win32WindowAdapter) -> Rc<dyn WindowAdapter> {
    let mut window =
        adapter.self_weak.upgrade().expect("the adapter is alive") as Rc<dyn WindowAdapter>;
    loop {
        let next = WindowInner::from_pub(window.window())
            .active_popups()
            .iter()
            .rev()
            .find(|p| !matches!(p.window_kind, WindowKind::ToolTip))
            .and_then(|p| match &p.location {
                PopupWindowLocation::TopLevel(wa) => Some(wa.clone()),
                PopupWindowLocation::ChildWindow(_) => None,
            });
        match next {
            Some(next) => window = next,
            None => return window,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_popup_moves_into_the_work_area() {
        let area = RECT { left: 0, top: 0, right: 100, bottom: 80 };
        let inside = |x, y| {
            let p = keep_inside(POINT { x, y }, (30, 20), area);
            (p.x, p.y)
        };
        assert_eq!(inside(10, 10), (10, 10));
        assert_eq!(inside(90, 70), (70, 60));
        assert_eq!(inside(-5, -5), (0, 0));
        // Larger than the area: from its top-left corner.
        let p = keep_inside(POINT { x: 50, y: 50 }, (200, 200), area);
        assert_eq!((p.x, p.y), (0, 0));
    }
}
