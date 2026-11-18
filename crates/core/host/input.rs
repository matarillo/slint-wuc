// SPDX-License-Identifier: MIT OR Apache-2.0

//! Mouse input: Win32 mouse messages become the pointer events of the window
//! (`docs/design/input.md`).

use super::{WM_APP_WINDOW_MOVE, Win32WindowAdapter};
use crate::input::{BackendMouseEvent, TouchPhase};
use crate::items::{BuiltInMouseCursor, PointerEventButton};
use crate::lengths::LogicalPoint;
use crate::platform::WindowEvent;
use core::cell::Cell;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    ReleaseCapture, SetCapture, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::PCWSTR;

/// winuser.h's value; the windows crate only has it with the common controls.
const WM_MOUSELEAVE: u32 = 0x02A3;

/// Logical pixels per wheel line, as upstream's winit backend converts line deltas.
const PIXELS_PER_LINE: f32 = 60.0;

/// What the window knows about the mouse between messages.
pub(super) struct Mouse {
    /// The last position in logical pixels, relative to the client area.
    position: Cell<LogicalPoint>,
    /// One bit per button that is down; see [`button_bit`].
    buttons: Cell<u8>,
    /// Whether `TrackMouseEvent` will send WM_MOUSELEAVE: the pointer is over the client area.
    tracking_leave: Cell<bool>,
    /// The system cursor that WM_SETCURSOR shows over the client area; `None` hides it.
    cursor: Cell<Option<PCWSTR>>,
}

impl Mouse {
    pub(super) fn new() -> Self {
        Self {
            position: Default::default(),
            buttons: Cell::new(0),
            tracking_leave: Cell::new(false),
            cursor: Cell::new(Some(IDC_ARROW)),
        }
    }

    /// Shows `cursor` over the client area from now on.
    pub(super) fn set_cursor(&self, cursor: BuiltInMouseCursor) {
        let id = cursor_id(cursor);
        if self.cursor.replace(id) != id && self.tracking_leave.get() {
            show_cursor(id);
        }
    }

    /// Forgets the pointer, for a window that is hidden.
    pub(super) fn reset(&self) {
        self.buttons.set(0);
        self.tracking_leave.set(false);
    }
}

/// Handles the mouse messages; `None` for any other message.
pub(super) fn handle(
    adapter: &Win32WindowAdapter,
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
) -> Option<LRESULT> {
    let mouse = &adapter.mouse;
    let dispatch = |event: BackendMouseEvent| {
        adapter.window.dispatch_event(WindowEvent::internal(event));
    };
    match msg {
        WM_MOUSEMOVE => {
            if !mouse.tracking_leave.replace(true) {
                let mut track = TRACKMOUSEEVENT {
                    cbSize: core::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                unsafe {
                    let _ = TrackMouseEvent(&mut track);
                }
            }
            let position = adapter.logical_point(point_from_lparam(lp));
            mouse.position.set(position);
            dispatch(BackendMouseEvent::Moved { position, touch_finger_id: 0 });
            Some(LRESULT(0))
        }
        WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_XBUTTONDOWN => {
            let button = button(msg, wp)?;
            let position = adapter.logical_point(point_from_lparam(lp));
            mouse.position.set(position);
            let buttons = mouse.buttons.get();
            if buttons == 0 {
                // Keep getting the mouse while a button is down, also outside the window.
                unsafe { SetCapture(hwnd) };
            }
            mouse.buttons.set(buttons | button_bit(button));
            dispatch(BackendMouseEvent::Pressed {
                position,
                button,
                click_count: 0,
                touch_finger_id: 0,
            });
            Some(handled(msg))
        }
        WM_LBUTTONUP | WM_RBUTTONUP | WM_MBUTTONUP | WM_XBUTTONUP => {
            let button = button(msg, wp)?;
            let position = adapter.logical_point(point_from_lparam(lp));
            mouse.position.set(position);
            let buttons = mouse.buttons.get() & !button_bit(button);
            mouse.buttons.set(buttons);
            if buttons == 0 {
                // WM_CAPTURECHANGED comes now, and finds no button down.
                unsafe {
                    let _ = ReleaseCapture();
                }
            }
            dispatch(BackendMouseEvent::Released {
                position,
                button,
                click_count: 0,
                touch_finger_id: 0,
            });
            // A WM_MOUSELEAVE while the button was down didn't report the exit.
            if buttons == 0 && !mouse.tracking_leave.get() {
                dispatch(BackendMouseEvent::Exit);
            }
            Some(handled(msg))
        }
        WM_MOUSEWHEEL | WM_MOUSEHWHEEL => {
            let lines = ((wp.0 >> 16) & 0xffff) as u16 as i16 as f32 / WHEEL_DELTA as f32;
            // winit's signs: the horizontal wheel is inverted.
            let (delta_x, delta_y) = if msg == WM_MOUSEWHEEL {
                (0.0, lines * PIXELS_PER_LINE)
            } else {
                (-lines * PIXELS_PER_LINE, 0.0)
            };
            // The wheel messages carry screen coordinates.
            let mut point = point_from_lparam(lp);
            unsafe {
                let _ = ScreenToClient(hwnd, &mut point);
            }
            let position = adapter.logical_point(point);
            dispatch(BackendMouseEvent::Wheel {
                position,
                delta_x,
                delta_y,
                phase: TouchPhase::Moved,
            });
            Some(LRESULT(0))
        }
        WM_MOUSELEAVE => {
            mouse.tracking_leave.set(false);
            // While a button is down, the window keeps the mouse; the release reports the exit.
            if mouse.buttons.get() == 0 {
                dispatch(BackendMouseEvent::Exit);
            }
            Some(LRESULT(0))
        }
        WM_CAPTURECHANGED => {
            // Another window took the mouse while a button was down: the press ends
            // without a release, so nothing counts it as a click.
            if lp.0 != hwnd.0 as isize && mouse.buttons.replace(0) != 0 {
                dispatch(BackendMouseEvent::Exit);
            }
            Some(LRESULT(0))
        }
        WM_APP_WINDOW_MOVE => {
            // Only while the left button that started the drag is still down.
            if mouse.buttons.get() & button_bit(PointerEventButton::Left) != 0 {
                // The system's move loop takes the button up, so the press ends here.
                mouse.buttons.set(0);
                dispatch(BackendMouseEvent::Exit);
                unsafe {
                    let _ = ReleaseCapture();
                    // Returns when the button is released.
                    SendMessageW(hwnd, WM_NCLBUTTONDOWN, Some(WPARAM(HTCAPTION as usize)), None);
                }
            }
            Some(LRESULT(0))
        }
        WM_SETCURSOR if (lp.0 & 0xffff) as u32 == HTCLIENT => {
            show_cursor(mouse.cursor.get());
            Some(LRESULT(1))
        }
        _ => None,
    }
}

/// Lets the user move the window by dragging it, as a `WindowMoveArea` asks while the left
/// button is down.
///
/// The request comes while the window dispatches a mouse move, so the system's move loop
/// starts from a posted message instead, once that dispatch is over.
pub(super) fn start_window_move(hwnd: HWND) {
    unsafe {
        let _ = PostMessageW(Some(hwnd), WM_APP_WINDOW_MOVE, WPARAM(0), LPARAM(0));
    }
}

/// The button of a button message.
fn button(msg: u32, wp: WPARAM) -> Option<PointerEventButton> {
    Some(match msg {
        WM_LBUTTONDOWN | WM_LBUTTONUP => PointerEventButton::Left,
        WM_RBUTTONDOWN | WM_RBUTTONUP => PointerEventButton::Right,
        WM_MBUTTONDOWN | WM_MBUTTONUP => PointerEventButton::Middle,
        _ => match ((wp.0 >> 16) & 0xffff) as u16 {
            XBUTTON1 => PointerEventButton::Back,
            XBUTTON2 => PointerEventButton::Forward,
            _ => return None,
        },
    })
}

fn button_bit(button: PointerEventButton) -> u8 {
    match button {
        PointerEventButton::Left => 1,
        PointerEventButton::Right => 2,
        PointerEventButton::Middle => 4,
        PointerEventButton::Back => 8,
        PointerEventButton::Forward => 16,
        _ => 32,
    }
}

/// What a button message returns: the X button messages return TRUE.
fn handled(msg: u32) -> LRESULT {
    LRESULT((msg == WM_XBUTTONDOWN || msg == WM_XBUTTONUP) as isize)
}

/// The signed coordinates that mouse messages pack into `lParam`.
pub(super) fn point_from_lparam(lp: LPARAM) -> POINT {
    POINT {
        x: (lp.0 & 0xffff) as u16 as i16 as i32,
        y: ((lp.0 >> 16) & 0xffff) as u16 as i16 as i32,
    }
}

fn show_cursor(id: Option<PCWSTR>) {
    unsafe {
        let cursor = id.and_then(|id| LoadCursorW(None, id).ok());
        SetCursor(cursor);
    }
}

/// The system cursor for a Slint cursor; `None` hides the cursor.
/// Windows has no cursor for alias and copy, and one for each axis of resizing.
fn cursor_id(cursor: BuiltInMouseCursor) -> Option<PCWSTR> {
    use BuiltInMouseCursor as C;
    Some(match cursor {
        C::None => return None,
        C::Default | C::Alias | C::Copy => IDC_ARROW,
        C::Help => IDC_HELP,
        C::Pointer => IDC_HAND,
        C::Progress => IDC_APPSTARTING,
        C::Wait => IDC_WAIT,
        C::Crosshair => IDC_CROSS,
        C::Text => IDC_IBEAM,
        C::Move | C::Grab | C::Grabbing => IDC_SIZEALL,
        C::NoDrop | C::NotAllowed => IDC_NO,
        C::ColResize | C::EResize | C::WResize | C::EwResize => IDC_SIZEWE,
        C::RowResize | C::NResize | C::SResize | C::NsResize => IDC_SIZENS,
        C::NeResize | C::SwResize | C::NeswResize => IDC_SIZENESW,
        C::NwResize | C::SeResize | C::NwseResize => IDC_SIZENWSE,
    })
}
