// SPDX-License-Identifier: MIT OR Apache-2.0

//! Touch and pen input: `WM_POINTER*` messages (`docs/design/input.md`).
//!
//! The mouse doesn't come through here: without `EnableMouseInPointer`, Windows keeps sending
//! the mouse messages that `input.rs` handles. Touch and pen messages aren't passed to
//! `DefWindowProc`, so Windows doesn't also turn them into mouse messages.

use super::Win32WindowAdapter;
use crate::input::{BackendMouseEvent, TouchPhase};
use crate::items::PointerEventButton;
use crate::lengths::LogicalPoint;
use crate::platform::{InternalEvent, WindowEvent};
use alloc::vec::Vec;
use core::cell::{Cell, RefCell};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::UI::Input::Pointer::{
    GetPointerInfo, GetPointerPenInfo, GetPointerType, POINTER_FLAG_CANCELED, POINTER_INFO,
    POINTER_PEN_INFO,
};
use windows::Win32::UI::WindowsAndMessaging::*;

/// Which kind of pointer a message comes from.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerKind {
    Touch,
    Pen,
}

/// What a `WM_POINTER*` message says about its pointer, from `GetPointerInfo`.
#[doc(hidden)]
#[derive(Clone, Copy, Debug)]
pub struct PointerMessage {
    /// `WM_POINTERDOWN`, `WM_POINTERUPDATE`, `WM_POINTERUP`, `WM_POINTERCAPTURECHANGED` or
    /// `WM_POINTERLEAVE`.
    pub msg: u32,
    pub kind: PointerKind,
    pub id: u32,
    /// In screen pixels.
    pub position: POINT,
    /// The system cancelled the contact, for example for a palm or an edge gesture.
    pub canceled: bool,
    /// The pen's barrel button is held.
    pub barrel: bool,
}

/// The touch and pen pointers the window knows about.
#[derive(Default)]
pub(super) struct Pointers {
    /// For each finger down: its pointer id, the finger id the core sees, and the last position.
    fingers: RefCell<Vec<(u32, i32, LogicalPoint)>>,
    /// The button the pen pressed while it's in contact.
    pen_button: Cell<Option<PointerEventButton>>,
}

impl Pointers {
    /// Forgets the pointers, for a window that is hidden.
    pub(super) fn reset(&self) {
        self.fingers.borrow_mut().clear();
        self.pen_button.set(None);
    }
}

/// Handles the touch and pen messages; `None` for any other message, the mouse's included.
pub(super) fn handle(
    adapter: &Win32WindowAdapter,
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
) -> Option<LRESULT> {
    if !matches!(
        msg,
        WM_POINTERDOWN
            | WM_POINTERUPDATE
            | WM_POINTERUP
            | WM_POINTERCAPTURECHANGED
            | WM_POINTERLEAVE
    ) {
        return None;
    }
    let id = (wp.0 & 0xffff) as u32;
    let message = query(msg, id)?;
    dispatch(adapter, hwnd, message);
    Some(LRESULT(0))
}

/// Reads the pointer's state; `None` for the mouse and pointers that are gone.
fn query(msg: u32, id: u32) -> Option<PointerMessage> {
    let mut kind = POINTER_INPUT_TYPE::default();
    unsafe { GetPointerType(id, &mut kind) }.ok()?;
    let (kind, info, barrel) = match kind {
        PT_TOUCH => {
            let mut info = POINTER_INFO::default();
            unsafe { GetPointerInfo(id, &mut info) }.ok()?;
            (PointerKind::Touch, info, false)
        }
        PT_PEN => {
            let mut pen = POINTER_PEN_INFO::default();
            unsafe { GetPointerPenInfo(id, &mut pen) }.ok()?;
            (PointerKind::Pen, pen.pointerInfo, pen.penFlags & PEN_FLAG_BARREL != 0)
        }
        _ => return None,
    };
    Some(PointerMessage {
        msg,
        kind,
        id,
        position: info.ptPixelLocation,
        canceled: info.pointerFlags.contains(POINTER_FLAG_CANCELED),
        barrel,
    })
}

/// Turns a pointer message into the window's events.
pub(super) fn dispatch(adapter: &Win32WindowAdapter, hwnd: HWND, message: PointerMessage) {
    let mut point = message.position;
    unsafe {
        let _ = ScreenToClient(hwnd, &mut point);
    }
    let position = adapter.logical_point(point);
    match message.kind {
        PointerKind::Touch => touch(adapter, message, position),
        PointerKind::Pen => pen(adapter, message, position),
    }
}

/// Touch becomes touch events, with finger ids handed out per pointer while it's down and
/// reused once lifted, as upstream's winit backend does.
fn touch(adapter: &Win32WindowAdapter, message: PointerMessage, position: LogicalPoint) {
    let pointers = &adapter.pointers;
    let send = |id: i32, position: LogicalPoint, phase: TouchPhase| {
        adapter.window.dispatch_event(WindowEvent::internal(InternalEvent::Touch {
            id,
            position,
            phase,
            event_time: None,
            history: Default::default(),
        }));
    };
    let finger = |pointer: u32| {
        pointers.fingers.borrow().iter().find(|(p, ..)| *p == pointer).map(|&(_, finger, _)| finger)
    };
    let lift = |pointer: u32| {
        let mut fingers = pointers.fingers.borrow_mut();
        let index = fingers.iter().position(|(p, ..)| *p == pointer)?;
        Some(fingers.remove(index))
    };
    match message.msg {
        WM_POINTERDOWN => {
            let id = {
                let mut fingers = pointers.fingers.borrow_mut();
                let id = (0..).find(|id| fingers.iter().all(|&(_, f, _)| f != *id)).unwrap();
                fingers.push((message.id, id, position));
                id
            };
            send(id, position, TouchPhase::Started);
        }
        WM_POINTERUPDATE => {
            let Some(id) = finger(message.id) else { return };
            if message.canceled {
                lift(message.id);
                send(id, position, TouchPhase::Cancelled);
            } else {
                for entry in pointers.fingers.borrow_mut().iter_mut() {
                    if entry.0 == message.id {
                        entry.2 = position;
                    }
                }
                send(id, position, TouchPhase::Moved);
            }
        }
        WM_POINTERUP => {
            if let Some((_, id, _)) = lift(message.id) {
                let phase =
                    if message.canceled { TouchPhase::Cancelled } else { TouchPhase::Ended };
                send(id, position, phase);
            }
        }
        // Another window took the pointer: where it is now doesn't matter to this window.
        WM_POINTERCAPTURECHANGED => {
            if let Some((_, id, last)) = lift(message.id) {
                send(id, last, TouchPhase::Cancelled);
            }
        }
        _ => {}
    }
}

/// The pen becomes the mouse: the left button, or the right one while the barrel button is held.
fn pen(adapter: &Win32WindowAdapter, message: PointerMessage, position: LogicalPoint) {
    let pointers = &adapter.pointers;
    let send = |event: BackendMouseEvent| {
        adapter.window.dispatch_event(WindowEvent::internal(event));
    };
    match message.msg {
        WM_POINTERDOWN => {
            let button =
                if message.barrel { PointerEventButton::Right } else { PointerEventButton::Left };
            pointers.pen_button.set(Some(button));
            send(BackendMouseEvent::Moved { position, touch_finger_id: 0 });
            send(BackendMouseEvent::Pressed {
                position,
                button,
                click_count: 0,
                touch_finger_id: 0,
            });
        }
        // Hovering above the screen, or moving in contact.
        WM_POINTERUPDATE => {
            if message.canceled && pointers.pen_button.take().is_some() {
                send(BackendMouseEvent::Exit);
            } else {
                send(BackendMouseEvent::Moved { position, touch_finger_id: 0 });
            }
        }
        WM_POINTERUP => {
            if let Some(button) = pointers.pen_button.take() {
                send(if message.canceled {
                    BackendMouseEvent::Exit
                } else {
                    BackendMouseEvent::Released {
                        position,
                        button,
                        click_count: 0,
                        touch_finger_id: 0,
                    }
                });
            }
        }
        WM_POINTERCAPTURECHANGED => {
            if pointers.pen_button.take().is_some() {
                send(BackendMouseEvent::Exit);
            }
        }
        // Out of range, or off the window.
        WM_POINTERLEAVE if pointers.pen_button.get().is_none() => {
            send(BackendMouseEvent::Exit);
        }
        _ => {}
    }
}
