// Copyright © Kentaro Inomata
// SPDX-License-Identifier: MIT-0

//! The frame and state of a top-level window, from the properties of its `Window` item, as
//! upstream's winit backend applies them on Windows: `no-frame`, `always-on-top`,
//! `full-screen`, `maximized`, `minimized`, whether the user can resize the window, and `icon`.
//! Also the frame of a window whose content extends into the title bar (`title_bar.rs`).

use super::Win32WindowAdapter;
use super::bitmap::{self, Alpha};
use crate::graphics::Image;
use crate::items::WindowItem;
use crate::window::{LayoutConstraints, WindowInner};
use core::cell::{Cell, RefCell};
use core::pin::Pin;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::DwmDefWindowProc;
use windows::Win32::Graphics::Gdi::{
    CreateBitmap, DeleteObject, GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO,
    MonitorFromWindow, ScreenToClient,
};
use windows::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
use windows::Win32::UI::WindowsAndMessaging::*;

#[derive(Clone, Copy, PartialEq)]
struct State {
    no_frame: bool,
    resizable: bool,
    always_on_top: bool,
    full_screen: bool,
    maximized: bool,
    minimized: bool,
}

impl Default for State {
    fn default() -> Self {
        Self {
            no_frame: false,
            resizable: true,
            always_on_top: false,
            full_screen: false,
            maximized: false,
            minimized: false,
        }
    }
}

impl State {
    fn style(self) -> WINDOW_STYLE {
        let mut style = if self.no_frame {
            WS_POPUP | WS_THICKFRAME | WS_SYSMENU | WS_MINIMIZEBOX | WS_MAXIMIZEBOX
        } else {
            WS_OVERLAPPEDWINDOW
        };
        if !self.resizable {
            style &= !(WS_THICKFRAME | WS_MAXIMIZEBOX);
        }
        style
    }

    /// Whether the client area covers the whole window: a frameless window keeps
    /// `WS_THICKFRAME` for its shadow and for resizing, but no border.
    fn client_covers_window(self) -> bool {
        self.no_frame && !self.full_screen
    }
}

#[derive(Default)]
pub(super) struct Frame {
    /// What the `Window` item asks for.
    wanted: Cell<State>,
    /// What the window shows, once it exists.
    shown: Cell<Option<State>>,
    /// `resize-border-width` in logical pixels.
    resize_border: Cell<f32>,
    /// The style and placement that full screen replaced.
    windowed: Cell<Option<(u32, WINDOWPLACEMENT)>>,
    icon: RefCell<Option<Icon>>,
}

impl Frame {
    /// Takes what the `Window` item asks for.
    pub(super) fn update(&self, item: Pin<&WindowItem>, constraints: LayoutConstraints) {
        let resizable = match constraints.min.zip(constraints.max) {
            Some((min, max)) => min.width < max.width || min.height < max.height,
            None => true,
        };
        self.wanted.set(State {
            no_frame: item.no_frame(),
            resizable,
            always_on_top: item.always_on_top(),
            full_screen: item.full_screen(),
            maximized: item.maximized(),
            minimized: item.minimized(),
        });
        self.resize_border.set(item.resize_border_width().get());
    }

    /// The styles to create the window with.
    pub(super) fn styles(&self) -> (WINDOW_STYLE, WINDOW_EX_STYLE) {
        let wanted = self.wanted.get();
        let ex_style = if wanted.always_on_top { WS_EX_TOPMOST } else { WINDOW_EX_STYLE(0) };
        (wanted.style(), ex_style)
    }

    pub(super) fn created(&self, hwnd: HWND) {
        let wanted = self.wanted.get();
        self.shown.set(Some(State {
            full_screen: false,
            maximized: false,
            minimized: false,
            ..wanted
        }));
        // `CreateWindowExW` sizes the client before the window procedure finds the adapter.
        if wanted.client_covers_window() {
            set_style(hwnd, wanted.style());
        }
    }

    /// Takes the state that the window is shown with, from `show_command`.
    pub(super) fn showing(&self) {
        let wanted = self.wanted.get();
        if let Some(shown) = self.shown.get() {
            self.shown.set(Some(State {
                minimized: wanted.minimized,
                maximized: wanted.maximized,
                ..shown
            }));
        }
    }

    /// Whether `frame_size` adds nothing to the client size.
    pub(super) fn client_covers_window(&self) -> bool {
        self.shown.get().unwrap_or_else(|| self.wanted.get()).client_covers_window()
    }

    /// Whether the window has a title bar: it isn't `no-frame` and isn't full screen.
    pub(super) fn has_caption(&self) -> bool {
        let state = self.shown.get().unwrap_or_else(|| self.wanted.get());
        !state.no_frame && !state.full_screen
    }

    /// Whether the window's size follows its state rather than the size it was given.
    pub(super) fn sized_by_state(&self) -> bool {
        let wanted = self.wanted.get();
        wanted.full_screen || wanted.maximized || wanted.minimized
    }

    /// How to show the window the first time.
    pub(super) fn show_command(&self) -> SHOW_WINDOW_CMD {
        let wanted = self.wanted.get();
        if wanted.minimized {
            SW_SHOWMINIMIZED
        } else if wanted.maximized {
            SW_SHOWMAXIMIZED
        } else {
            SW_SHOW
        }
    }

    /// Brings the window to what the `Window` item asks for. The state of a hidden window
    /// waits until it's shown.
    pub(super) fn apply(&self, adapter: &Win32WindowAdapter, hwnd: HWND, visible: bool) {
        let Some(mut shown) = self.shown.get() else { return };
        let wanted = self.wanted.get();
        if (wanted.no_frame, wanted.resizable) != (shown.no_frame, shown.resizable) {
            shown.no_frame = wanted.no_frame;
            shown.resizable = wanted.resizable;
            self.shown.set(Some(shown));
            if !shown.full_screen {
                adapter.change_frame(|| set_style(hwnd, wanted.style()));
            }
        }
        if wanted.always_on_top != shown.always_on_top {
            shown.always_on_top = wanted.always_on_top;
            self.shown.set(Some(shown));
            let after = if wanted.always_on_top { HWND_TOPMOST } else { HWND_NOTOPMOST };
            unsafe {
                let _ = SetWindowPos(
                    hwnd,
                    Some(after),
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                );
            }
        }
        if !visible {
            return;
        }
        if wanted.full_screen != shown.full_screen {
            shown.full_screen = wanted.full_screen;
            self.shown.set(Some(shown));
            if wanted.full_screen {
                self.enter_full_screen(hwnd);
            } else {
                self.leave_full_screen(hwnd, shown);
            }
        }
        // Full screen keeps the other states for when it ends.
        if shown.full_screen {
            return;
        }
        if wanted.minimized != shown.minimized || wanted.maximized != shown.maximized {
            shown.minimized = wanted.minimized;
            shown.maximized = wanted.maximized;
            self.shown.set(Some(shown));
            let command = if wanted.minimized {
                SW_MINIMIZE
            } else if wanted.maximized {
                SW_MAXIMIZE
            } else {
                SW_RESTORE
            };
            unsafe {
                let _ = ShowWindow(hwnd, command);
            }
        }
    }

    fn enter_full_screen(&self, hwnd: HWND) {
        let mut placement =
            WINDOWPLACEMENT { length: size_of::<WINDOWPLACEMENT>() as u32, ..Default::default() };
        let style = unsafe { GetWindowLongW(hwnd, GWL_STYLE) } as u32;
        if unsafe { GetWindowPlacement(hwnd, &mut placement) }.is_ok() {
            self.windowed.set(Some((style, placement)));
        }
        let mut monitor =
            MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        unsafe {
            let _ =
                GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST), &mut monitor);
            let style = WINDOW_STYLE(style) & !(WS_OVERLAPPEDWINDOW | WS_MAXIMIZE) | WS_POPUP;
            SetWindowLongW(hwnd, GWL_STYLE, style.0 as i32);
            let rc = monitor.rcMonitor;
            let _ = SetWindowPos(
                hwnd,
                Some(HWND_TOP),
                rc.left,
                rc.top,
                rc.right - rc.left,
                rc.bottom - rc.top,
                SWP_FRAMECHANGED | SWP_NOOWNERZORDER,
            );
        }
    }

    fn leave_full_screen(&self, hwnd: HWND, shown: State) {
        let Some((style, placement)) = self.windowed.take() else { return };
        // The frame may have changed while the window was full screen.
        let keep = WINDOW_STYLE(style) & (WS_VISIBLE | WS_MINIMIZE | WS_MAXIMIZE);
        unsafe {
            SetWindowLongW(hwnd, GWL_STYLE, (shown.style() | keep).0 as i32);
            let _ = SetWindowPlacement(hwnd, &placement);
            let _ = SetWindowPos(
                hwnd,
                None,
                0,
                0,
                0,
                0,
                SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOOWNERZORDER,
            );
        }
    }

    /// Follows the window's state when the user changes it, and tells the `Window` item.
    fn sized(&self, adapter: &Win32WindowAdapter, kind: u32) {
        let Some(mut shown) = self.shown.get() else { return };
        if shown.full_screen {
            return;
        }
        match kind {
            SIZE_MINIMIZED => shown.minimized = true,
            SIZE_MAXIMIZED => {
                shown.minimized = false;
                shown.maximized = true;
            }
            SIZE_RESTORED => {
                shown.minimized = false;
                shown.maximized = false;
            }
            _ => return,
        }
        self.shown.set(Some(shown));
        let wanted = self.wanted.get();
        self.wanted.set(State { minimized: shown.minimized, maximized: shown.maximized, ..wanted });
        // A maximized window keeps that state while minimized, to come back maximized.
        let inner = WindowInner::from_pub(&adapter.window);
        if inner.is_minimized() != shown.minimized {
            inner.set_minimized(shown.minimized);
        }
        if !shown.minimized && inner.is_maximized() != shown.maximized {
            inner.set_maximized(shown.maximized);
        }
    }

    /// Handles the messages of a frameless window's border and of a window that extends into
    /// its title bar; follows `WM_SIZE`.
    pub(super) fn handle(
        &self,
        adapter: &Win32WindowAdapter,
        hwnd: HWND,
        msg: u32,
        wp: WPARAM,
        lp: LPARAM,
    ) -> Option<LRESULT> {
        if adapter.extends_into_title_bar() {
            // The caption buttons: hover, press and the hit test of Snap Layouts.
            let mut result = LRESULT(0);
            if unsafe { DwmDefWindowProc(hwnd, msg, wp, lp, &mut result) }.as_bool() {
                return Some(result);
            }
        }
        match msg {
            WM_SIZE => {
                self.sized(adapter, wp.0 as u32);
                None
            }
            WM_NCCALCSIZE if wp.0 != 0 && adapter.extends_into_title_bar() => {
                // The default frame on the left, right and bottom; the client area takes the
                // title bar. A maximized window isn't inset at the top: that stops DWM from
                // hit-testing the caption buttons, so the host moves the content down instead.
                // SAFETY: with wParam TRUE, lParam points to NCCALCSIZE_PARAMS.
                let top = unsafe { (*(lp.0 as *const NCCALCSIZE_PARAMS)).rgrc[0].top };
                let result = unsafe { DefWindowProcW(hwnd, msg, wp, lp) };
                unsafe { (*(lp.0 as *mut NCCALCSIZE_PARAMS)).rgrc[0].top = top };
                Some(result)
            }
            WM_NCHITTEST if adapter.extends_into_title_bar() => {
                let result = unsafe { DefWindowProcW(hwnd, msg, wp, lp) };
                if result.0 as u32 != HTCLIENT {
                    return Some(result);
                }
                // The top resize edge, inside the client area now.
                let shown = self.shown.get()?;
                let mut point = super::input::point_from_lparam(lp);
                unsafe {
                    let _ = ScreenToClient(hwnd, &mut point);
                }
                let dpi = unsafe { GetDpiForWindow(hwnd) };
                let zoomed = unsafe { IsZoomed(hwnd).as_bool() };
                if shown.resizable && !zoomed && point.y < frame_y(dpi) {
                    return Some(LRESULT(HTTOP as isize));
                }
                Some(result)
            }
            WM_NCCALCSIZE if wp.0 != 0 && self.client_covers_window() => {
                if unsafe { IsZoomed(hwnd).as_bool() } {
                    // A maximized window reaches past the monitor by its frame.
                    // SAFETY: with wParam TRUE, lParam points to NCCALCSIZE_PARAMS.
                    let params = unsafe { &mut *(lp.0 as *mut NCCALCSIZE_PARAMS) };
                    let dpi = unsafe { GetDpiForWindow(hwnd) };
                    let padding = unsafe { GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi) };
                    let x = unsafe { GetSystemMetricsForDpi(SM_CXFRAME, dpi) } + padding;
                    let y = unsafe { GetSystemMetricsForDpi(SM_CYFRAME, dpi) } + padding;
                    let rc = &mut params.rgrc[0];
                    *rc = RECT {
                        left: rc.left + x,
                        top: rc.top + y,
                        right: rc.right - x,
                        bottom: rc.bottom - y,
                    };
                }
                Some(LRESULT(0))
            }
            WM_NCHITTEST if self.client_covers_window() => self.hit_test(adapter, hwnd, lp),
            _ => None,
        }
    }

    /// The border of a frameless window that the user can resize, `resize-border-width` wide.
    fn hit_test(&self, adapter: &Win32WindowAdapter, hwnd: HWND, lp: LPARAM) -> Option<LRESULT> {
        let shown = self.shown.get()?;
        let border = (self.resize_border.get() * adapter.window.scale_factor()).round() as i32;
        if !shown.resizable || border <= 0 || unsafe { IsZoomed(hwnd).as_bool() } {
            return None;
        }
        let mut point =
            POINT { x: (lp.0 & 0xffff) as i16 as i32, y: ((lp.0 >> 16) & 0xffff) as i16 as i32 };
        let mut client = RECT::default();
        unsafe {
            let _ = ScreenToClient(hwnd, &mut point);
            GetClientRect(hwnd, &mut client).ok()?;
        }
        let left = point.x < border;
        let right = point.x >= client.right - border;
        let top = point.y < border;
        let bottom = point.y >= client.bottom - border;
        let hit = match (left, right, top, bottom) {
            (true, _, true, _) => HTTOPLEFT,
            (_, true, true, _) => HTTOPRIGHT,
            (true, _, _, true) => HTBOTTOMLEFT,
            (_, true, _, true) => HTBOTTOMRIGHT,
            (true, ..) => HTLEFT,
            (_, true, ..) => HTRIGHT,
            (_, _, true, _) => HTTOP,
            (.., true) => HTBOTTOM,
            _ => return None,
        };
        Some(LRESULT(hit as isize))
    }

    /// Shows `image` as the window's icon, unless it already does.
    pub(super) fn set_icon(&self, hwnd: HWND, image: &Image) {
        if self.icon.borrow().as_ref().is_some_and(|icon| icon.image == *image) {
            return;
        }
        let dpi = unsafe { GetDpiForWindow(hwnd) };
        let big = icon(image, unsafe { GetSystemMetricsForDpi(SM_CXICON, dpi) });
        let small = icon(image, unsafe { GetSystemMetricsForDpi(SM_CXSMICON, dpi) });
        unsafe {
            let handle = |icon: Option<HICON>| LPARAM(icon.map_or(0, |icon| icon.0 as isize));
            SendMessageW(hwnd, WM_SETICON, Some(WPARAM(ICON_BIG as usize)), Some(handle(big)));
            SendMessageW(hwnd, WM_SETICON, Some(WPARAM(ICON_SMALL as usize)), Some(handle(small)));
        }
        // The window no longer uses the icons it had.
        self.icon.replace(Some(Icon { image: image.clone(), big, small }));
    }
}

/// The frame's thickness at the top plus its padding: the top resize edge, and how far a
/// maximized window reaches above the monitor.
pub(super) fn frame_y(dpi: u32) -> i32 {
    let frame = unsafe { GetSystemMetricsForDpi(SM_CYFRAME, dpi) };
    frame + unsafe { GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi) }
}

/// The height of a title bar with the frame above it, in physical pixels: what the frame
/// takes at the top of a window that isn't maximized.
pub(super) fn caption_band(dpi: u32) -> i32 {
    let caption = unsafe { GetSystemMetricsForDpi(SM_CYCAPTION, dpi) };
    caption + frame_y(dpi)
}

/// Makes the window compute its client area again.
pub(super) fn frame_changed(hwnd: HWND) {
    unsafe {
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

fn set_style(hwnd: HWND, style: WINDOW_STYLE) {
    unsafe {
        let current = WINDOW_STYLE(GetWindowLongW(hwnd, GWL_STYLE) as u32);
        let keep = current & (WS_VISIBLE | WS_MINIMIZE | WS_MAXIMIZE);
        SetWindowLongW(hwnd, GWL_STYLE, (style | keep).0 as i32);
    }
    frame_changed(hwnd);
}

/// The icons that a window shows, destroyed once it no longer does.
struct Icon {
    image: Image,
    big: Option<HICON>,
    small: Option<HICON>,
}

impl Drop for Icon {
    fn drop(&mut self) {
        for icon in [self.big, self.small].into_iter().flatten() {
            unsafe {
                let _ = DestroyIcon(icon);
            }
        }
    }
}

/// `image` as an icon of `size`×`size` pixels.
fn icon(image: &Image, size: i32) -> Option<HICON> {
    let color = bitmap::scaled(image, size.max(1) as usize, Alpha::Straight)?;
    unsafe {
        // The alpha channel masks the icon; the mask bitmap is required anyway.
        let mask = CreateBitmap(size, size, 1, 1, None);
        let info =
            ICONINFO { fIcon: true.into(), hbmMask: mask, hbmColor: color, ..Default::default() };
        let icon = CreateIconIndirect(&info).ok();
        let _ = DeleteObject(mask.into());
        let _ = DeleteObject(color.into());
        icon
    }
}
