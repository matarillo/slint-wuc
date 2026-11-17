// SPDX-License-Identifier: MIT OR Apache-2.0

//! The input method, through IMM32 as winit (and so upstream on Windows) uses it (S4).
//!
//! The rest of the host only calls [`window_created`], [`request`], [`handle`] and
//! [`deactivating`], so that a TSF implementation can replace this module (for reconversion,
//! for example).
//!
//! Compositions behave as TSF handles them, so that moving to TSF changes nothing for
//! applications: a focus move commits the composition (the text input inserts its preedit text
//! when it loses the focus), and deactivating the window keeps it.
//!
//! The text input draws the composition itself, inline: the system's composition window is
//! hidden, and the candidate list is placed below the caret.

use super::Win32WindowAdapter;
use crate::SharedString;
use crate::input::{InternalKeyEvent, KeyEvent, KeyEventType};
use crate::items::InputType;
use crate::platform::WindowEvent;
use crate::window::{InputMethodProperties, InputMethodRequest};
use alloc::vec::Vec;
use core::cell::Cell;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::UI::Input::Ime::{
    ATTR_TARGET_CONVERTED, ATTR_TARGET_NOTCONVERTED, CANDIDATEFORM, CFS_EXCLUDE, CFS_POINT,
    COMPOSITIONFORM, CPS_CANCEL, GCS_COMPATTR, GCS_COMPSTR, GCS_CURSORPOS, GCS_RESULTSTR, HIMC,
    IACE_DEFAULT, IME_COMPOSITION_STRING, ISC_SHOWUICOMPOSITIONWINDOW, ImmAssociateContextEx,
    ImmGetCompositionStringW, ImmGetContext, ImmNotifyIME, ImmReleaseContext,
    ImmSetCandidateWindow, ImmSetCompositionWindow, NI_COMPOSITIONSTR,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DefWindowProcW, WM_IME_COMPOSITION, WM_IME_ENDCOMPOSITION, WM_IME_SETCONTEXT,
    WM_IME_STARTCOMPOSITION,
};

/// A window's input method state.
#[derive(Default)]
pub(super) struct Ime {
    /// Whether a text input that takes text has the focus.
    enabled: Cell<bool>,
    /// Whether the text input shows a preedit text that the input method gave.
    composing: Cell<bool>,
    /// The caret, in physical client pixels, for placing the candidate list.
    caret: Cell<RECT>,
    /// Set while Slint hears that the window is no longer active.
    deactivating: Cell<bool>,
}

/// Runs `dispatch`, which tells Slint that the window is no longer active.
///
/// The focused text input then ends the text input focus but keeps its preedit text, and the
/// input method keeps the composition: the user continues it when the window is active again.
pub(super) fn deactivating(adapter: &Win32WindowAdapter, dispatch: impl FnOnce()) {
    let was = adapter.ime.deactivating.replace(true);
    dispatch();
    adapter.ime.deactivating.set(was);
}

/// A new window takes no input method text until a text input asks for it, as in winit.
pub(super) fn window_created(hwnd: HWND) {
    associate(hwnd, false);
}

/// Handles the text input's [`InputMethodRequest`].
pub(super) fn request(adapter: &Win32WindowAdapter, hwnd: HWND, request: &InputMethodRequest) {
    let ime = &adapter.ime;
    let props = match request {
        // Password fields take no input method text, as Windows' own password edit controls.
        InputMethodRequest::Enable(props) if props.input_type == InputType::Password => {
            disable(ime, hwnd);
            return;
        }
        InputMethodRequest::Enable(props) => {
            if !ime.enabled.replace(true) {
                associate(hwnd, true);
            }
            props
        }
        InputMethodRequest::Update(props) if ime.enabled.get() => props,
        // The window comes back to the same text input, which enables it again.
        InputMethodRequest::Disable if ime.deactivating.get() => return,
        InputMethodRequest::Disable => {
            disable(ime, hwnd);
            return;
        }
        _ => return,
    };
    ime.caret.set(caret_rect(props, adapter.window.scale_factor()));
    place_windows(hwnd, ime.caret.get());
}

/// Handles the input method's window messages.
pub(super) fn handle(
    adapter: &Win32WindowAdapter,
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
) -> Option<LRESULT> {
    let ime = &adapter.ime;
    match msg {
        WM_IME_SETCONTEXT => {
            // The text input draws the composition.
            let lp = LPARAM(lp.0 & !(ISC_SHOWUICOMPOSITIONWINDOW as isize));
            Some(unsafe { DefWindowProcW(hwnd, msg, wp, lp) })
        }
        WM_IME_STARTCOMPOSITION if ime.enabled.get() => {
            place_windows(hwnd, ime.caret.get());
            Some(LRESULT(0))
        }
        WM_IME_COMPOSITION if ime.enabled.get() => {
            composition(adapter, hwnd, lp.0 as u32);
            // Not passed on: `DefWindowProc` would send the result again as `WM_IME_CHAR`.
            Some(LRESULT(0))
        }
        WM_IME_ENDCOMPOSITION if ime.enabled.get() => {
            // A composition that ends without a result (Escape, or the last character deleted)
            // leaves no preedit text behind.
            if ime.composing.replace(false) {
                dispatch(adapter, KeyEventType::UpdateComposition, "".into(), "".into(), None);
            }
            Some(LRESULT(0))
        }
        _ => None,
    }
}

/// `WM_IME_COMPOSITION`: the result, the composition and its cursor, as `flags` says.
fn composition(adapter: &Win32WindowAdapter, hwnd: HWND, flags: u32) {
    let ime = &adapter.ime;
    let himc = unsafe { ImmGetContext(hwnd) };
    if himc.is_invalid() {
        return;
    }
    let has = |kind: IME_COMPOSITION_STRING| flags & kind.0 != 0;
    let result = has(GCS_RESULTSTR).then(|| utf16_string(himc, GCS_RESULTSTR));
    let preedit = has(GCS_COMPSTR).then(|| {
        let text = utf16_string(himc, GCS_COMPSTR);
        let selection = selection(himc, &text, has(GCS_COMPATTR), has(GCS_CURSORPOS));
        (text, selection)
    });
    unsafe {
        let _ = ImmReleaseContext(hwnd, himc);
    }

    // One event can commit the result and show the next composition: the text input inserts
    // `text` and then shows the preedit text.
    let (preedit_text, selection) = match preedit {
        Some((text, selection)) => (String::from_utf16_lossy(&text), selection),
        None => (String::new(), None),
    };
    ime.composing.set(!preedit_text.is_empty());
    match result {
        Some(result) => dispatch(
            adapter,
            KeyEventType::CommitComposition,
            String::from_utf16_lossy(&result).into(),
            preedit_text.into(),
            selection,
        ),
        // A message with neither (the composition was emptied) clears the preedit text.
        None => dispatch(
            adapter,
            KeyEventType::UpdateComposition,
            "".into(),
            preedit_text.into(),
            selection,
        ),
    }
}

/// The part of the composition to show as selected, in UTF-8 bytes: the clause being
/// converted, or else an empty range at the cursor, as winit computes it.
fn selection(
    himc: HIMC,
    text: &[u16],
    has_attributes: bool,
    has_cursor: bool,
) -> Option<core::ops::Range<i32>> {
    let to_utf8 = |units: usize| -> i32 {
        String::from_utf16_lossy(&text[..units.min(text.len())]).len() as i32
    };
    if has_attributes {
        let attributes = composition_bytes(himc, GCS_COMPATTR);
        let target = |a: &u8| matches!(*a as u32, ATTR_TARGET_CONVERTED | ATTR_TARGET_NOTCONVERTED);
        if let Some(start) = attributes.iter().position(target) {
            let end = attributes[start..]
                .iter()
                .position(|a| !target(a))
                .map_or(attributes.len(), |n| start + n);
            return Some(to_utf8(start)..to_utf8(end));
        }
    }
    has_cursor.then(|| {
        let cursor = unsafe { ImmGetCompositionStringW(himc, GCS_CURSORPOS, None, 0) }.max(0);
        let cursor = to_utf8(cursor as usize);
        cursor..cursor
    })
}

fn dispatch(
    adapter: &Win32WindowAdapter,
    event_type: KeyEventType,
    text: SharedString,
    preedit_text: SharedString,
    preedit_selection: Option<core::ops::Range<i32>>,
) {
    let event = InternalKeyEvent {
        key_event: KeyEvent { text, ..Default::default() },
        event_type,
        preedit_text,
        preedit_selection,
        ..Default::default()
    };
    super::popup::key_window(adapter).window().dispatch_event(WindowEvent::internal(event));
}

fn disable(ime: &Ime, hwnd: HWND) {
    if !ime.enabled.replace(false) {
        return;
    }
    // The text input that loses the focus commits its preedit text itself; committing here would
    // put the text into the input that gets the focus next, or twice.
    if ime.composing.replace(false) {
        let himc = unsafe { ImmGetContext(hwnd) };
        if !himc.is_invalid() {
            unsafe {
                let _ = ImmNotifyIME(himc, NI_COMPOSITIONSTR, CPS_CANCEL, 0);
                let _ = ImmReleaseContext(hwnd, himc);
            }
        }
    }
    associate(hwnd, false);
}

/// Gives the window the thread's default input context, or none.
fn associate(hwnd: HWND, enabled: bool) {
    unsafe {
        let _ =
            ImmAssociateContextEx(hwnd, HIMC::default(), if enabled { IACE_DEFAULT } else { 0 });
    }
}

/// The caret rectangle of `props`, in physical client pixels.
fn caret_rect(props: &InputMethodProperties, scale: f32) -> RECT {
    let px = |v: f32| (v * scale).round() as i32;
    let (origin, size) = (props.cursor_rect_origin, props.cursor_rect_size);
    let left = px(origin.x);
    let top = px(origin.y);
    RECT {
        left,
        top,
        right: px(origin.x + size.width).max(left + 1),
        bottom: px(origin.y + size.height).max(top + 1),
    }
}

/// Puts the candidate list below the caret, never over it.
fn place_windows(hwnd: HWND, caret: RECT) {
    unsafe {
        let himc = ImmGetContext(hwnd);
        if himc.is_invalid() {
            return;
        }
        // Input methods that draw their own composition window put it here.
        let composition = COMPOSITIONFORM {
            dwStyle: CFS_POINT,
            ptCurrentPos: POINT { x: caret.left, y: caret.top },
            rcArea: RECT::default(),
        };
        let _ = ImmSetCompositionWindow(himc, &composition);
        let candidate = CANDIDATEFORM {
            dwIndex: 0,
            dwStyle: CFS_EXCLUDE,
            ptCurrentPos: POINT { x: caret.left, y: caret.bottom },
            rcArea: caret,
        };
        let _ = ImmSetCandidateWindow(himc, &candidate);
        let _ = ImmReleaseContext(hwnd, himc);
    }
}

fn composition_bytes(himc: HIMC, kind: IME_COMPOSITION_STRING) -> Vec<u8> {
    unsafe {
        let len = ImmGetCompositionStringW(himc, kind, None, 0);
        if len <= 0 {
            return Vec::new();
        }
        let mut buffer = alloc::vec![0u8; len as usize];
        let len =
            ImmGetCompositionStringW(himc, kind, Some(buffer.as_mut_ptr().cast()), len as u32);
        buffer.truncate(len.max(0) as usize);
        buffer
    }
}

fn utf16_string(himc: HIMC, kind: IME_COMPOSITION_STRING) -> Vec<u16> {
    composition_bytes(himc, kind)
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&pair| u16::from_le_bytes(pair))
        .collect()
}
