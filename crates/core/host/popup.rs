// Copyright © Kentaro Inomata
// SPDX-License-Identifier: MIT-0

//! Popup windows: `docs/design/windows.md`.

use super::Win32WindowAdapter;
use crate::api::{LogicalPosition, LogicalSize, PhysicalSize};
use crate::graphics::{Rgba8Pixel, SharedPixelBuffer};
use crate::item_tree::ItemRc;
use crate::items::PopupClosePolicy;
use crate::lengths::LogicalRect;
use crate::properties::PropertyTracker;
use crate::window::{PopupWindowLocation, WindowAdapter, WindowInner, WindowKind};
use alloc::boxed::Box;
use alloc::rc::{Rc, Weak};
use alloc::vec::Vec;
use core::cell::Cell;
use core::pin::Pin;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DWM_WINDOW_CORNER_PREFERENCE, DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE,
    DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND, DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{
    ClientToScreen, GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
    ScreenToClient,
};
use windows::Win32::UI::WindowsAndMessaging::*;

/// What makes a window a popup.
pub(super) struct Popup {
    pub(super) parent: Weak<Win32WindowAdapter>,
    pub(super) kind: WindowKind,
    /// The position in the parent's logical client coordinates, as the core last set it.
    pub(super) position: Cell<Option<LogicalPosition>>,
    /// The room around the popup, in physical pixels, for what its items draw past its edges,
    /// such as drop shadows: drawn inside its parent, upstream shows them.
    pub(super) margin: Cell<Margin>,
    margin_tracker: Pin<Box<PropertyTracker>>,
}

/// Room on each side of a rectangle, in physical pixels.
#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub(super) struct Margin {
    pub(super) left: i32,
    pub(super) top: i32,
    pub(super) right: i32,
    pub(super) bottom: i32,
}

impl Popup {
    pub(super) fn new(parent: Weak<Win32WindowAdapter>, kind: WindowKind) -> Self {
        Self {
            parent,
            kind,
            position: Cell::new(None),
            margin: Cell::new(Margin::default()),
            margin_tracker: Box::pin(PropertyTracker::default()),
        }
    }

    pub(super) const STYLE: WINDOW_STYLE = WS_POPUP;
    pub(super) const EX_STYLE: WINDOW_EX_STYLE =
        WINDOW_EX_STYLE(WS_EX_NOACTIVATE.0 | WS_EX_TOOLWINDOW.0 | WS_EX_NOREDIRECTIONBITMAP.0);

    /// The top-left corner of the window of a popup of `size` physical pixels, in screen
    /// pixels: the popup goes where the core asked for, moved into the work area of the
    /// parent's monitor, and its margin around it.
    pub(super) fn screen_position(&self, size: (i32, i32)) -> Option<POINT> {
        let parent = self.parent.upgrade()?;
        let parent_hwnd = parent.hwnd.get()?;
        let position = self.position.get()?.to_physical(parent.window.scale_factor());
        // A popup of a popup, such as a submenu, is placed in its parent's content, which
        // starts after the parent window's margin.
        let parent_margin = parent.margin();
        let mut point =
            POINT { x: position.x + parent_margin.left, y: position.y + parent_margin.top };
        unsafe {
            let _ = ClientToScreen(parent_hwnd, &mut point);
            let monitor = MonitorFromWindow(parent_hwnd, MONITOR_DEFAULTTONEAREST);
            let mut info =
                MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
            if GetMonitorInfoW(monitor, &mut info).as_bool() {
                point = keep_inside(point, size, info.rcWork);
            }
        }
        let margin = self.margin.get();
        Some(POINT { x: point.x - margin.left, y: point.y - margin.top })
    }

    /// Computes the margin again if what it depends on changed, and tells whether it did.
    pub(super) fn update_margin(
        &self,
        window: &WindowInner,
        size: LogicalSize,
        scale: f32,
    ) -> bool {
        if !self.margin_tracker.is_dirty() {
            return false;
        }
        let bounds = self.margin_tracker.as_ref().evaluate(|| drawn_bounds(window, size));
        let px = |v: f32| (v.max(0.) * scale).ceil() as i32;
        let margin = Margin {
            left: px(-bounds.min_x()),
            top: px(-bounds.min_y()),
            right: px(bounds.max_x() - size.width),
            bottom: px(bounds.max_y() - size.height),
        };
        self.margin.replace(margin) != margin
    }
}

/// The rectangle that the items of `window`, of `size`, draw into, in its logical coordinates.
fn drawn_bounds(window: &WindowInner, size: LogicalSize) -> LogicalRect {
    let mut bounds = LogicalRect::from_size(size.to_euclid());
    let Some(component) = window.try_component() else { return bounds };
    let adapter = window.window_adapter();
    let mut pending = Vec::from([ItemRc::new_root(component)]);
    while let Some(item) = pending.pop() {
        let mut child = item.first_child();
        while let Some(c) = child {
            let rect = c.bounding_rect(&c.geometry(), &adapter);
            if !rect.is_empty() {
                let origin = c.map_to_window(rect.origin);
                bounds = bounds.union(&LogicalRect::new(origin, rect.size));
            }
            // What a clipping item holds stays inside its bounds.
            if !c.borrow().as_ref().clips_children() {
                pending.push(c.clone());
            }
            child = c.next_sibling();
        }
    }
    bounds
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
pub(super) fn handle(
    adapter: &Win32WindowAdapter,
    popup: &Popup,
    hwnd: HWND,
    msg: u32,
    lp: LPARAM,
) -> Option<LRESULT> {
    match msg {
        WM_MOUSEACTIVATE => Some(LRESULT(MA_NOACTIVATE as isize)),
        // A tooltip and the margin let the mouse through to the window below, which is in this
        // thread: there, a click is outside the popup.
        WM_NCHITTEST
            if matches!(popup.kind, WindowKind::ToolTip) || !in_popup(adapter, popup, hwnd, lp) =>
        {
            Some(LRESULT(HTTRANSPARENT as isize))
        }
        _ => None,
    }
}

/// Whether the screen point in `lp` is on the popup rather than in its margin.
fn in_popup(adapter: &Win32WindowAdapter, popup: &Popup, hwnd: HWND, lp: LPARAM) -> bool {
    let mut point = super::input::point_from_lparam(lp);
    unsafe {
        let _ = ScreenToClient(hwnd, &mut point);
    }
    let margin = popup.margin.get();
    let size = adapter.size.get();
    (margin.left..margin.left + size.width as i32).contains(&point.x)
        && (margin.top..margin.top + size.height as i32).contains(&point.y)
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
pub(super) fn native_popups(adapter: &Win32WindowAdapter) -> Vec<Rc<Win32WindowAdapter>> {
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

/// Draws the popups of `adapter` that have windows of their own over `snapshot` of its client
/// area, where they are on the screen, margins included.
pub(super) fn draw_popups(
    adapter: &Win32WindowAdapter,
    snapshot: &mut SharedPixelBuffer<Rgba8Pixel>,
) {
    let Some(hwnd) = adapter.hwnd.get() else { return };
    let mut origin = POINT::default();
    unsafe {
        let _ = ClientToScreen(hwnd, &mut origin);
    }
    // A popup's own popups, such as submenus, are the popup window's and come after it.
    let mut popups = native_popups(adapter);
    let mut i = 0;
    while let Some(popup) = popups.get(i) {
        let nested = native_popups(popup);
        popups.splice(i + 1..i + 1, nested);
        i += 1;
    }
    for popup in popups {
        let (Some(popup_hwnd), Some(p)) = (popup.hwnd.get(), &popup.popup) else { continue };
        let mut rect = RECT::default();
        if unsafe { GetWindowRect(popup_hwnd, &mut rect) }.is_err() {
            continue;
        }
        let margin = p.margin.get();
        let size = popup.size.get();
        let full = PhysicalSize::new(
            size.width + (margin.left + margin.right) as u32,
            size.height + (margin.top + margin.bottom) as u32,
        );
        match popup.capture(full) {
            Ok(pixels) => blend(snapshot, &pixels, rect.left - origin.x, rect.top - origin.y),
            Err(e) => crate::debug_log!("cannot capture a popup: {e}"),
        }
    }
}

/// Draws `top` over `bottom` with its top-left corner at `(x, y)`; both have straight alpha.
fn blend(
    bottom: &mut SharedPixelBuffer<Rgba8Pixel>,
    top: &SharedPixelBuffer<Rgba8Pixel>,
    x: i32,
    y: i32,
) {
    let (width, height) = (bottom.width() as i32, bottom.height() as i32);
    let top_width = top.width() as i32;
    let bottom_pixels = bottom.make_mut_slice();
    for (i, s) in top.as_slice().iter().enumerate() {
        let (tx, ty) = (x + i as i32 % top_width, y + i as i32 / top_width);
        if s.a == 0 || !(0..width).contains(&tx) || !(0..height).contains(&ty) {
            continue;
        }
        let d = &mut bottom_pixels[(ty * width + tx) as usize];
        let (sa, da) = (s.a as u32, d.a as u32 * (255 - s.a as u32) / 255);
        let a = sa + da;
        let mix = |s: u8, d: u8| ((s as u32 * sa + d as u32 * da + a / 2) / a) as u8;
        *d = Rgba8Pixel { r: mix(s.r, d.r), g: mix(s.g, d.g), b: mix(s.b, d.b), a: a as u8 };
    }
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
