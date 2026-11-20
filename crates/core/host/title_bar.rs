// Copyright © Kentaro Inomata
// SPDX-License-Identifier: MIT-0

//! Content extended into the title bar, as WinUI 3's `ExtendsContentIntoTitleBar`: the
//! client area of a framed top-level window covers the title bar, and DWM keeps drawing the
//! caption buttons over it (`docs/design/windows.md`).
//!
//! The API is unstable; the facade's `slint::wuc` builds on the functions here.

use super::{Win32WindowAdapter, frame};
use crate::api::Window;
use crate::graphics::Brush;
use crate::lengths::{LogicalPoint, LogicalRect, LogicalSize};
use crate::properties::Property;
use alloc::boxed::Box;
use core::cell::{Cell, RefCell};
use core::pin::Pin;
use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DWMWA_CAPTION_BUTTON_BOUNDS, DWMWA_CAPTION_COLOR, DWMWA_COLOR_DEFAULT,
    DwmExtendFrameIntoClientArea, DwmGetWindowAttribute, DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::UI::Controls::MARGINS;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::*;

/// Measures the title bar again once the change that asked for it is over, and reports it.
pub(super) const WM_APP_TITLE_BAR: u32 = super::WM_APP + 8;
/// Opens the system menu at the screen point in `lParam`.
pub(super) const WM_APP_SYSTEM_MENU: u32 = super::WM_APP + 9;
/// Carries out the system command in `wParam`, `SC_MAXIMIZE` or `SC_RESTORE`, once the press
/// that asked for it is over.
pub(super) const WM_APP_TOGGLE_MAXIMIZED: u32 = super::WM_APP + 10;

/// The title bar of a window whose content extends into it, in logical pixels of the
/// content's coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TitleBar {
    /// From the top of the content to the bottom of the caption buttons.
    pub height: f32,
    /// Where DWM draws the minimize, maximize and close buttons over the content.
    pub buttons: LogicalRect,
}

/// Lets the content of `window` cover its title bar, or stops it. Only a framed top-level
/// window that isn't full screen and has no native menu bar extends; for other windows, the
/// setting waits until the window is one. Before `show()`, it applies when the window is
/// created; afterwards, the window keeps its client size.
pub fn set_extends_content_into_title_bar(window: &Window, extend: bool) {
    let Some(adapter) = Win32WindowAdapter::from_window(window) else { return };
    if adapter.popup.is_some() || adapter.title_bar.wanted.replace(extend) == extend {
        return;
    }
    if let Some(hwnd) = adapter.hwnd.get() {
        let was = adapter.title_bar.applied.get();
        let zoomed = unsafe { IsZoomed(hwnd).as_bool() };
        adapter.change_frame(|| frame::frame_changed(hwnd));
        let now = adapter.extends_into_title_bar();
        if zoomed && was != now {
            keep_restored_client_size(hwnd, now);
        }
    }
    adapter.update_title_bar();
}

/// What [`set_extends_content_into_title_bar`] last set.
pub fn extends_content_into_title_bar(window: &Window) -> bool {
    Win32WindowAdapter::from_window(window).is_some_and(|a| a.title_bar.wanted.get())
}

/// The title bar that the content of `window` extends into: `None` while it doesn't, or the
/// window has no HWND yet.
pub fn title_bar(window: &Window) -> Option<TitleBar> {
    Win32WindowAdapter::from_window(window)?.title_bar.current.get()
}

/// Calls `callback` whenever [`title_bar`] changes: when the window starts or stops
/// extending into it, and when its DPI, its maximized state or the system's metrics change
/// it. The callback comes from a posted message, after the change is over. It replaces any
/// earlier one.
pub fn on_title_bar_changed(window: &Window, callback: impl FnMut(Option<TitleBar>) + 'static) {
    if let Some(adapter) = Win32WindowAdapter::from_window(window) {
        adapter.title_bar.callback.replace(Some(Box::new(callback)));
    }
}

/// What a top-level window keeps about its title bar.
pub(super) struct State {
    /// What [`set_extends_content_into_title_bar`] asks for.
    wanted: Cell<bool>,
    /// Whether the window extended into the title bar when it was last updated.
    applied: Cell<bool>,
    /// The room above the content, in physical pixels: a maximized window's client area
    /// starts above the monitor by the frame.
    pub(super) top_margin: Cell<i32>,
    /// The caption color that the `Window` item's background asks for: its color when it's
    /// an opaque solid color.
    wanted_caption_color: Cell<u32>,
    /// The caption color the window was given, once the host gave it one.
    caption_color: Cell<Option<u32>>,
    /// Where the root `Window` item's background leaves a hole for the caption buttons.
    hole: Pin<Box<Property<Option<LogicalRect>>>>,
    current: Cell<Option<TitleBar>>,
    reported: Cell<Option<TitleBar>>,
    callback: RefCell<Option<Box<dyn FnMut(Option<TitleBar>)>>>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            wanted: Cell::new(false),
            applied: Cell::new(false),
            top_margin: Cell::new(0),
            wanted_caption_color: Cell::new(DWMWA_COLOR_DEFAULT),
            caption_color: Cell::new(None),
            hole: Box::pin(Property::new(None)),
            current: Cell::new(None),
            reported: Cell::new(None),
            callback: RefCell::new(None),
        }
    }
}

impl State {
    pub(super) fn wanted(&self) -> bool {
        self.wanted.get()
    }

    /// The hole in the root `Window` item's background, read in its binding.
    pub(super) fn hole(&self) -> Option<LogicalRect> {
        self.hole.as_ref().get()
    }

    /// Takes the caption color from the `Window` item's background.
    pub(super) fn set_background(&self, background: &Brush) {
        let color = match background {
            Brush::SolidColor(c) if c.alpha() == 255 => {
                u32::from_le_bytes([c.red(), c.green(), c.blue(), 0])
            }
            _ => DWMWA_COLOR_DEFAULT,
        };
        self.wanted_caption_color.set(color);
    }
}

impl Win32WindowAdapter {
    /// Whether the client area covers the title bar now.
    pub(super) fn extends_into_title_bar(&self) -> bool {
        self.popup.is_none()
            && self.title_bar.wanted.get()
            && self.frame.has_caption()
            && self.hwnd.get().is_none_or(|hwnd| unsafe { GetMenu(hwnd) }.is_invalid())
    }

    /// Brings the frame's extension, the margin, the caption color and the title bar up to
    /// date after something they depend on changed, and reports the title bar once the
    /// change is over.
    pub(super) fn update_title_bar(&self) {
        let Some(hwnd) = self.hwnd.get() else { return };
        // Most windows never extend: nothing to update, nothing to report.
        if self.popup.is_some() || !(self.title_bar.wanted.get() || self.title_bar.applied.get())
        {
            return;
        }
        let extends = self.extends_into_title_bar();
        let dpi = unsafe { GetDpiForWindow(hwnd) };
        let zoomed = unsafe { IsZoomed(hwnd).as_bool() };
        let top_margin = if extends && zoomed { frame::frame_y(dpi) } else { 0 };
        if self.title_bar.top_margin.replace(top_margin) != top_margin {
            self.offset_root();
        }
        if extends || self.title_bar.applied.get() {
            // DWM hit-tests and draws the caption buttons only inside the extended frame.
            let top = if extends { frame::caption_band(dpi) + top_margin } else { 0 };
            let margins = MARGINS { cyTopHeight: top, ..Default::default() };
            unsafe {
                let _ = DwmExtendFrameIntoClientArea(hwnd, &margins);
            }
        }
        self.title_bar.applied.set(extends);
        self.update_caption_color(hwnd);
        self.measure_title_bar(hwnd);
        unsafe {
            let _ = PostMessageW(Some(hwnd), WM_APP_TITLE_BAR, WPARAM(0), LPARAM(0));
        }
    }

    /// Gives the caption buttons the background's color while the window extends into the
    /// title bar, so that they don't stand out from the content; DWM's default otherwise.
    pub(super) fn update_caption_color(&self, hwnd: HWND) {
        let state = &self.title_bar;
        let color = if state.applied.get() {
            state.wanted_caption_color.get()
        } else {
            DWMWA_COLOR_DEFAULT
        };
        let current = state.caption_color.get();
        if current == Some(color) || (current.is_none() && color == DWMWA_COLOR_DEFAULT) {
            return;
        }
        state.caption_color.set(Some(color));
        unsafe {
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_CAPTION_COLOR,
                (&color as *const u32).cast(),
                size_of::<u32>() as u32,
            );
        }
    }

    /// Measures the title bar and the hole in the background. A minimized window keeps
    /// what it had.
    fn measure_title_bar(&self, hwnd: HWND) {
        if unsafe { IsIconic(hwnd).as_bool() } {
            return;
        }
        let state = &self.title_bar;
        let measured = if state.applied.get() { self.measure(hwnd) } else { None };
        state.current.set(measured.map(|(title_bar, _)| title_bar));
        state.hole.as_ref().set(measured.map(|(_, hole)| hole));
    }

    /// The title bar, and the hole in the background: from the caption buttons' left edge to
    /// the right edge of the content, and from its top to the buttons' bottom.
    fn measure(&self, hwnd: HWND) -> Option<(TitleBar, LogicalRect)> {
        let mut buttons = RECT::default();
        let mut window = RECT::default();
        let mut origin = POINT::default();
        unsafe {
            DwmGetWindowAttribute(
                hwnd,
                DWMWA_CAPTION_BUTTON_BOUNDS,
                (&mut buttons as *mut RECT).cast(),
                size_of::<RECT>() as u32,
            )
            .ok()?;
            GetWindowRect(hwnd, &mut window).ok()?;
            ClientToScreen(hwnd, &mut origin).ok().ok()?;
        }
        // From window coordinates to the content's physical pixels.
        let margin = self.margin();
        let dx = window.left - origin.x - margin.left;
        let dy = window.top - origin.y - margin.top;
        // The client area's, since `WM_SIZE` measures before the content is resized.
        let width = super::client_size(hwnd).width as i32 - margin.left - margin.right;
        // DWM's bounds reach a pixel past the client area on the right (120 dpi).
        let (left, top) = (buttons.left + dx, buttons.top + dy);
        let (right, bottom) = ((buttons.right + dx).min(width), buttons.bottom + dy);
        let scale = self.window.scale_factor();
        let logical = |v: i32| v as f32 / scale;
        let buttons = LogicalRect::new(
            LogicalPoint::new(logical(left), logical(top)),
            LogicalSize::new(logical(right - left), logical(bottom - top)),
        );
        let hole = LogicalRect::new(
            LogicalPoint::new(logical(left), 0.),
            LogicalSize::new(logical(width - left), logical(bottom)),
        );
        Some((TitleBar { height: logical(bottom.max(0)), buttons }, hole))
    }

    /// Measures the title bar again and calls the callback if it changed.
    pub(super) fn report_title_bar(&self) {
        let Some(hwnd) = self.hwnd.get() else { return };
        self.measure_title_bar(hwnd);
        let current = self.title_bar.current.get();
        if self.title_bar.reported.replace(current) == current {
            return;
        }
        // Taken out while it runs, so that it can set another one.
        let Some(mut callback) = self.title_bar.callback.take() else { return };
        callback(current);
        let mut slot = self.title_bar.callback.borrow_mut();
        if slot.is_none() {
            *slot = Some(callback);
        }
    }

    /// Toggles maximized as a double click on the title bar does, unless the window can't
    /// be maximized.
    pub(super) fn toggle_maximized(&self) {
        let Some(hwnd) = self.hwnd.get() else { return };
        let style = WINDOW_STYLE(unsafe { GetWindowLongW(hwnd, GWL_STYLE) } as u32);
        if !style.contains(WS_MAXIMIZEBOX) {
            return;
        }
        let command = if unsafe { IsZoomed(hwnd).as_bool() } { SC_RESTORE } else { SC_MAXIMIZE };
        // Posted: the core is dispatching the press.
        unsafe {
            let _ = PostMessageW(
                Some(hwnd),
                WM_APP_TOGGLE_MAXIMIZED,
                WPARAM(command as usize),
                LPARAM(0),
            );
        }
    }

    /// Asks for the system menu at `position`, in logical window coordinates.
    pub(super) fn request_system_menu(&self, position: LogicalPoint) {
        let Some(hwnd) = self.hwnd.get() else { return };
        let scale = self.window.scale_factor();
        let margin = self.margin();
        let mut point = POINT {
            x: (position.x * scale).round() as i32 + margin.left,
            y: (position.y * scale).round() as i32 + margin.top,
        };
        unsafe {
            let _ = ClientToScreen(hwnd, &mut point);
            let lp = ((point.y as u16 as isize) << 16) | point.x as u16 as isize;
            // `TrackPopupMenuEx` runs a message loop of its own, and the core is dispatching
            // the release.
            let _ = PostMessageW(Some(hwnd), WM_APP_SYSTEM_MENU, WPARAM(0), LPARAM(lp));
        }
    }
}

/// Opens the system menu at the screen point in `lp`, with its entries enabled as the
/// window's state allows, and carries out the chosen command.
pub(super) fn open_system_menu(hwnd: HWND, lp: LPARAM) {
    let point = super::input::point_from_lparam(lp);
    unsafe {
        let menu = GetSystemMenu(hwnd, false);
        if menu.is_invalid() {
            return;
        }
        let style = WINDOW_STYLE(GetWindowLongW(hwnd, GWL_STYLE) as u32);
        let zoomed = IsZoomed(hwnd).as_bool();
        let enable = |command: u32, enabled: bool| {
            let flags = if enabled { MF_ENABLED } else { MF_GRAYED };
            let _ = EnableMenuItem(menu, command, MF_BYCOMMAND | flags);
        };
        enable(SC_RESTORE, zoomed);
        enable(SC_MOVE, !zoomed);
        enable(SC_SIZE, !zoomed && style.contains(WS_THICKFRAME));
        enable(SC_MINIMIZE, style.contains(WS_MINIMIZEBOX));
        enable(SC_MAXIMIZE, !zoomed && style.contains(WS_MAXIMIZEBOX));
        enable(SC_CLOSE, true);
        let _ = SetMenuDefaultItem(menu, SC_CLOSE, 0);
        let flags = TPM_RETURNCMD | TPM_LEFTALIGN | TPM_RIGHTBUTTON;
        let command = TrackPopupMenuEx(menu, flags.0, point.x, point.y, hwnd, None);
        if command.0 != 0 {
            let _ = PostMessageW(Some(hwnd), WM_SYSCOMMAND, WPARAM(command.0 as usize), LPARAM(0));
        }
    }
}

/// Keeps the client size that a maximized window comes back to when the title bar starts or
/// stops being part of the client area: the restored window rectangle loses or gains it.
fn keep_restored_client_size(hwnd: HWND, extends: bool) {
    let mut placement =
        WINDOWPLACEMENT { length: size_of::<WINDOWPLACEMENT>() as u32, ..Default::default() };
    unsafe {
        if GetWindowPlacement(hwnd, &mut placement).is_err() {
            return;
        }
        let band = frame::caption_band(GetDpiForWindow(hwnd));
        placement.rcNormalPosition.bottom += if extends { -band } else { band };
        // Only the restored rectangle changes: the window stays maximized.
        placement.showCmd = SW_SHOWMAXIMIZED.0 as u32;
        let _ = SetWindowPlacement(hwnd, &placement);
    }
}
