// SPDX-License-Identifier: MIT OR Apache-2.0

//! Keyboard input: Win32 key messages become the key events of the window
//! (`docs/design/input.md`).
//!
//! A key down message produces the key event; the character messages that follow it don't.
//! The text of the event comes from, in this order:
//!
//! - [`named_key`]: keys without a character map to Slint's key codes.
//! - The character messages that `TranslateMessage` queued for the key down, which are what the
//!   system composed from the key and the dead key before it.
//! - `ToUnicodeEx` with the thread's keyboard state, for the messages that no `TranslateMessage`
//!   saw (`SendMessage`), and for the text without Ctrl and Alt.

use super::Win32WindowAdapter;
use crate::SharedString;
use crate::input::{InternalKeyEvent, KeyEvent, KeyEventType, key_codes};
use crate::items::KeyboardModifiers;
use crate::platform::WindowEvent;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::{Cell, RefCell};
use std::collections::HashMap;
use windows::Win32::Foundation::*;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, GetKeyboardLayout, GetKeyboardState, HKL, MAPVK_VK_TO_VSC, MapVirtualKeyExW,
    ToUnicodeEx, VIRTUAL_KEY, VK_APPS, VK_BACK, VK_CONTROL, VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE,
    VK_F1, VK_F4, VK_F24, VK_HOME, VK_INSERT, VK_LCONTROL, VK_LEFT, VK_LMENU, VK_LSHIFT, VK_LWIN,
    VK_MENU, VK_NEXT, VK_NUMPAD0, VK_NUMPAD9, VK_PACKET, VK_PAUSE, VK_PRIOR, VK_PROCESSKEY,
    VK_RCONTROL, VK_RETURN, VK_RIGHT, VK_RMENU, VK_RSHIFT, VK_RWIN, VK_SCROLL, VK_SHIFT,
    VK_SNAPSHOT, VK_SPACE, VK_TAB, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::*;

/// The flag of `ToUnicodeEx` that leaves the kernel's dead-key state alone (Windows 10 1607 and
/// later): asking for the text of a key never makes a dead key pending or uses it up.
const KEEP_DEAD_KEY_STATE: u32 = 0x4;

/// What the keyboard handler remembers between messages.
///
/// It belongs to the thread, as the keyboard state and the dead key of the system do, and not to
/// a window.
#[derive(Default)]
struct State {
    /// The first half of a surrogate pair that `WM_CHAR` is delivering.
    high_surrogate: Cell<u16>,
    /// The virtual key of the dead key that is down: its key up has no text to report.
    dead_key: Cell<u16>,
    /// Whether the layout has an AltGr key, by layout handle.
    alt_graph: RefCell<HashMap<isize, bool>>,
}

std::thread_local! {
    static STATE: State = State::default();
}

/// Handles the keyboard messages; `None` for any other message.
pub(super) fn handle(
    adapter: &Win32WindowAdapter,
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
) -> Option<LRESULT> {
    match msg {
        WM_KEYDOWN | WM_SYSKEYDOWN | WM_KEYUP | WM_SYSKEYUP => {
            Some(STATE.with(|state| key_message(state, adapter, hwnd, msg, wp, lp)))
        }
        WM_CHAR | WM_SYSCHAR => Some(STATE.with(|state| char_message(state, adapter, wp))),
        WM_ACTIVATE => {
            let active = (wp.0 & 0xffff) as u32 != WA_INACTIVE;
            if active {
                // Keys released while another window had the keyboard never reported it.
                adapter.inner().release_modifiers_not_held(held_modifiers());
            } else {
                STATE.with(|state| {
                    state.high_surrogate.set(0);
                    state.dead_key.set(0);
                });
            }
            let dispatch =
                || adapter.window.dispatch_event(WindowEvent::WindowActiveChanged(active));
            if active {
                dispatch()
            } else {
                super::ime::deactivating(adapter, dispatch)
            }
            // DefWindowProc gives the keyboard focus to a window that is activated.
            Some(unsafe { DefWindowProcW(hwnd, msg, wp, lp) })
        }
        _ => None,
    }
}

/// `WM_KEYDOWN`, `WM_SYSKEYDOWN`, `WM_KEYUP` and `WM_SYSKEYUP`.
///
/// Plain key messages return 0. As in winit, every `WM_SYSKEYDOWN` also goes to `DefWindowProc`,
/// whatever Slint did with it, and Alt+F4 only goes there: it closes the window, and no
/// `FocusScope` can take it. Alt+Space opens the system menu. `WM_SYSKEYUP` never goes there:
/// with no menu bar, releasing Alt would start the system's menu mode, which takes the keyboard
/// from the window.
fn key_message(
    state: &State,
    adapter: &Win32WindowAdapter,
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
) -> LRESULT {
    let down = matches!(msg, WM_KEYDOWN | WM_SYSKEYDOWN);
    let vk = (wp.0 & 0xffff) as u16;
    let flags = lp.0 as u32;
    let scan = (flags >> 16) & 0xff;
    let extended = flags & (1 << 24) != 0;
    // Bit 30 is whether the key was down before this message; a key up has it set too.
    let repeat = down && flags & (1 << 30) != 0;

    // These are no keys: `SendInput` with `KEYEVENTF_UNICODE` and an input method send them, and
    // the text arrives as `WM_CHAR`.
    if vk == VK_PACKET.0 || vk == VK_PROCESSKEY.0 {
        return LRESULT(0);
    }

    // What `TranslateMessage` made of this key down, so that no `WM_CHAR` is left for
    // `char_message` to send a second time.
    let chars = if down { take_chars(hwnd) } else { Chars::default() };

    if !down && state.dead_key.get() == vk {
        state.dead_key.set(0);
        return LRESULT(0);
    }

    let keys = keyboard_state();
    // The digits of an Alt code (Alt+0233) are no keys either: the system collects them, and
    // `WM_CHAR` delivers the character when Alt is released.
    if (VK_NUMPAD0.0..=VK_NUMPAD9.0).contains(&vk)
        && is_down(&keys, VK_MENU)
        && !is_down(&keys, VK_CONTROL)
    {
        return LRESULT(0);
    }

    let hkl = unsafe { GetKeyboardLayout(0) };
    let alt_graph = state.has_alt_graph(hkl);
    // AltGr is Ctrl+Alt, and the system sends a Ctrl before the AltGr: it isn't a key.
    if alt_graph
        && (vk == VK_CONTROL.0 || vk == VK_LCONTROL.0)
        && !extended
        && next_key_is_alt_gr(hwnd)
    {
        return LRESULT(0);
    }

    let composed = String::from_utf16_lossy(&chars.text);
    // `ToUnicodeEx` doesn't see a dead key as one: `TranslateMessage` made it pending already.
    let translation = if chars.dead && composed.is_empty() {
        Translation::Dead
    } else {
        translate(vk, scan, extended, &keys, hkl, alt_graph, &composed)
    };
    let alt_f4 = msg == WM_SYSKEYDOWN && vk == VK_F4.0;
    // A dead key, and a key without text, dispatch nothing; the dead key shows when the key
    // after it composes the accent.
    if let Translation::Text { text, without_modifiers } = translation
        && !alt_f4
    {
        let event_type = if down { KeyEventType::KeyPressed } else { KeyEventType::KeyReleased };
        dispatch(adapter, event_type, text, without_modifiers, repeat);
    }
    if down && chars.dead {
        state.dead_key.set(vk);
    }

    if msg != WM_SYSKEYDOWN {
        return LRESULT(0);
    }
    let result = unsafe { DefWindowProcW(hwnd, msg, wp, lp) };
    // The Alt+Space that opens the system menu is the character message, which the key down
    // took out of the queue.
    for m in chars.system_space {
        unsafe { DefWindowProcW(hwnd, m.message, m.wParam, m.lParam) };
    }
    result
}

/// `WM_CHAR` and `WM_SYSCHAR` that no key down produced: the characters of `SendInput` with
/// `KEYEVENTF_UNICODE`, of input methods, and Alt codes.
///
/// The characters of a key down were taken out of the queue by [`key_message`], so there is no
/// text here to send a second time.
fn char_message(state: &State, adapter: &Win32WindowAdapter, wp: WPARAM) -> LRESULT {
    // Characters outside the basic plane arrive as two messages.
    let unit = (wp.0 & 0xffff) as u16;
    let character = match unit {
        0xd800..=0xdbff => {
            state.high_surrogate.set(unit);
            return LRESULT(0);
        }
        0xdc00..=0xdfff => {
            char::decode_utf16([state.high_surrogate.replace(0), unit]).next().and_then(Result::ok)
        }
        _ => char::from_u32(unit.into()),
    };
    let character = match character {
        // Return is '\n' to Slint.
        Some('\r') => Some('\n'),
        // Control characters are keys, which have a key down of their own.
        Some(c) if c != '\n' && c.is_control() => None,
        other => other,
    };
    if let Some(character) = character {
        let text = SharedString::from(character);
        dispatch(adapter, KeyEventType::KeyPressed, text.clone(), text, false);
    }
    LRESULT(0)
}

/// The character messages of a key down.
#[derive(Default)]
struct Chars {
    /// The UTF-16 text, with all modifiers and dead keys applied.
    text: Vec<u16>,
    /// The key is a dead key.
    dead: bool,
    /// The `WM_SYSCHAR` messages, which the system menu needs when Slint doesn't use the key.
    system_space: Vec<MSG>,
}

/// Takes the character messages that `TranslateMessage` queued for the key down out of the queue.
///
/// They are first in the queue: the next key message in the queue is the next key. A message that
/// came with `SendMessage` has none.
fn take_chars(hwnd: HWND) -> Chars {
    let mut chars = Chars::default();
    loop {
        let mut next = MSG::default();
        let found =
            unsafe { PeekMessageW(&mut next, Some(hwnd), WM_KEYFIRST, WM_KEYLAST, PM_NOREMOVE) };
        if !found.as_bool() {
            break;
        }
        let message = next.message;
        if !matches!(message, WM_CHAR | WM_SYSCHAR | WM_DEADCHAR | WM_SYSDEADCHAR) {
            break;
        }
        // The one that was first in the key messages.
        if !unsafe { PeekMessageW(&mut next, Some(hwnd), message, message, PM_REMOVE) }.as_bool() {
            break;
        }
        match message {
            WM_DEADCHAR | WM_SYSDEADCHAR => chars.dead = true,
            _ => {
                chars.text.push((next.wParam.0 & 0xffff) as u16);
                if message == WM_SYSCHAR && next.wParam.0 == usize::from(b' ') {
                    chars.system_space.push(next);
                }
            }
        }
    }
    chars
}

/// Dispatches a key event.
fn dispatch(
    adapter: &Win32WindowAdapter,
    event_type: KeyEventType,
    text: SharedString,
    text_without_modifiers: SharedString,
    repeat: bool,
) {
    let event = InternalKeyEvent {
        key_event: KeyEvent { text, repeat, ..Default::default() },
        event_type,
        text_without_modifiers,
        ..Default::default()
    };
    adapter.window.dispatch_event(WindowEvent::internal(event));
}

/// What a key message means to Slint.
#[derive(Debug, PartialEq)]
enum Translation {
    Text {
        /// What the key gives, as `KeyEvent::text`.
        text: SharedString,
        /// What the key gives without Ctrl and Alt.
        without_modifiers: SharedString,
    },
    /// A dead key.
    Dead,
    /// A key without text: no character, no key code.
    Nothing,
}

/// The text of a key, in the keyboard state `keys`.
///
/// This is what winit's Windows keyboard code reports: a key code for the keys without a
/// character, the text of the key as the layout gives it (with Shift and Caps Lock, and AltGr)
/// for the others. Ctrl turns text into control characters, and a layout without AltGr gives
/// nothing for Ctrl+Alt: then the text is the key without Ctrl and Alt, so that shortcuts see
/// the letter they are made of.
///
/// `composed` is the text of the character messages that the key down got, if any.
fn translate(
    vk: u16,
    scan: u32,
    extended: bool,
    keys: &[u8; 256],
    hkl: HKL,
    alt_graph: bool,
    composed: &str,
) -> Translation {
    if let Some(code) = named_key(vk, scan, extended, alt_graph) {
        let text = SharedString::from(code);
        return Translation::Text { without_modifiers: text.clone(), text };
    }

    let all = if composed.is_empty() {
        to_unicode(vk, scan, keys, hkl)
    } else {
        Characters::Text(composed.into())
    };
    let without = if is_down(keys, VK_CONTROL) || is_down(keys, VK_MENU) {
        let mut keys = *keys;
        for key in [VK_CONTROL, VK_LCONTROL, VK_RCONTROL, VK_MENU, VK_LMENU, VK_RMENU] {
            keys[key.0 as usize] &= !0x80;
        }
        to_unicode(vk, scan, &keys, hkl)
    } else {
        all.clone()
    };
    let without = match without {
        Characters::Text(text) if printable(&text) => Some(text),
        // Space has no character while a dead key waits for the key after it.
        _ if vk == VK_SPACE.0 => Some(String::from(' ')),
        _ => None,
    };

    let text = match all {
        Characters::Dead => return Translation::Dead,
        Characters::Text(text) if printable(&text) => Some(text),
        // Ctrl+C gives U+0003, and Ctrl+Alt+C gives nothing without AltGr.
        _ => without.clone(),
    };
    match text {
        Some(text) => Translation::Text {
            text: text.into(),
            without_modifiers: without.unwrap_or_default().into(),
        },
        None => Translation::Nothing,
    }
}

/// Whether a key press inserts the text: not when it is made of control characters.
fn printable(text: &str) -> bool {
    !text.is_empty() && !text.chars().all(char::is_control)
}

/// The result of `ToUnicodeEx`.
#[derive(Clone, Debug)]
enum Characters {
    Text(String),
    Dead,
    Nothing,
}

fn to_unicode(vk: u16, scan: u32, keys: &[u8; 256], hkl: HKL) -> Characters {
    let mut buffer = [0u16; 8];
    let count =
        unsafe { ToUnicodeEx(vk.into(), scan, keys, &mut buffer, KEEP_DEAD_KEY_STATE, Some(hkl)) };
    match count {
        ..0 => Characters::Dead,
        0 => Characters::Nothing,
        count => Characters::Text(String::from_utf16_lossy(&buffer[..count as usize])),
    }
}

/// The Slint key code of the keys that have none of the characters `ToUnicodeEx` gives:
/// modifiers, arrows, function keys and the like.
///
/// Left and right modifiers have a code each (Alt has one, and AltGr is the right Alt of a
/// layout that has it). The keys that Slint has no code for, such as Num Lock or the media
/// keys, are left to `ToUnicodeEx`, which gives them no text.
fn named_key(vk: u16, scan: u32, extended: bool, alt_graph: bool) -> Option<char> {
    use key_codes as k;
    if (VK_F1.0..=VK_F24.0).contains(&vk) {
        return char::from_u32(k::F1 as u32 + u32::from(vk - VK_F1.0));
    }
    Some(match VIRTUAL_KEY(vk) {
        // The messages say VK_SHIFT, VK_CONTROL and VK_MENU, and the side is in the scan code
        // and the extended flag.
        VK_SHIFT if scan == 0x36 => k::ShiftR,
        VK_SHIFT | VK_LSHIFT => k::Shift,
        VK_RSHIFT => k::ShiftR,
        VK_CONTROL if extended => k::ControlR,
        VK_CONTROL | VK_LCONTROL => k::Control,
        VK_RCONTROL => k::ControlR,
        VK_MENU if extended && alt_graph => k::AltGr,
        VK_MENU | VK_LMENU => k::Alt,
        VK_RMENU if alt_graph => k::AltGr,
        VK_RMENU => k::Alt,
        VK_LWIN => k::Meta,
        VK_RWIN => k::MetaR,
        VK_BACK => k::Backspace,
        VK_TAB => k::Tab,
        VK_RETURN => k::Return,
        VK_ESCAPE => k::Escape,
        VK_DELETE => k::Delete,
        VK_UP => k::UpArrow,
        VK_DOWN => k::DownArrow,
        VK_LEFT => k::LeftArrow,
        VK_RIGHT => k::RightArrow,
        VK_INSERT => k::Insert,
        VK_HOME => k::Home,
        VK_END => k::End,
        VK_PRIOR => k::PageUp,
        VK_NEXT => k::PageDown,
        VK_SCROLL => k::ScrollLock,
        VK_PAUSE => k::Pause,
        VK_SNAPSHOT => k::SysReq,
        VK_APPS => k::Menu,
        _ => return None,
    })
}

impl State {
    /// Whether `hkl` has an AltGr key: some key gives another character with Ctrl+Alt than
    /// without. That is how winit finds out.
    fn has_alt_graph(&self, hkl: HKL) -> bool {
        *self.alt_graph.borrow_mut().entry(hkl.0 as isize).or_insert_with(|| {
            let none = [0u8; 256];
            let mut ctrl_alt = [0u8; 256];
            for key in [VK_CONTROL, VK_LCONTROL, VK_MENU, VK_LMENU] {
                ctrl_alt[key.0 as usize] = 0x80;
            }
            // The digits, the letters and the OEM keys; 0xE2 is the key next to the left Shift.
            (0x30..=0x39)
                .chain(0x41..=0x5a)
                .chain(0xba..=0xc0)
                .chain(0xdb..=0xdf)
                .chain([0xe2])
                .any(|vk| {
                    let scan = unsafe { MapVirtualKeyExW(vk, MAPVK_VK_TO_VSC, Some(hkl)) };
                    match (
                        to_unicode(vk as u16, scan, &none, hkl),
                        to_unicode(vk as u16, scan, &ctrl_alt, hkl),
                    ) {
                        (Characters::Text(a), Characters::Text(b)) => {
                            a != b && !b.chars().any(char::is_control)
                        }
                        _ => false,
                    }
                })
        })
    }
}

/// Whether the next key message is the right Alt: the Ctrl that comes before it is no key.
fn next_key_is_alt_gr(hwnd: HWND) -> bool {
    let mut next = MSG::default();
    let found =
        unsafe { PeekMessageW(&mut next, Some(hwnd), WM_KEYFIRST, WM_KEYLAST, PM_NOREMOVE) };
    // The scan code of the right Alt: 0x38 with the extended flag.
    found.as_bool()
        && matches!(next.message, WM_KEYDOWN | WM_SYSKEYDOWN | WM_KEYUP | WM_SYSKEYUP)
        && (next.lParam.0 as u32 >> 16) & 0x1ff == 0x138
}

/// The keyboard state as of the message that the thread is handling.
fn keyboard_state() -> [u8; 256] {
    let mut keys = [0u8; 256];
    let _ = unsafe { GetKeyboardState(&mut keys) };
    keys
}

fn is_down(keys: &[u8; 256], key: VIRTUAL_KEY) -> bool {
    keys[key.0 as usize] & 0x80 != 0
}

/// The modifiers that are down now.
fn held_modifiers() -> KeyboardModifiers {
    let down = |key: VIRTUAL_KEY| unsafe { GetKeyState(key.0.into()) } < 0;
    KeyboardModifiers {
        alt: down(VK_MENU),
        control: down(VK_CONTROL),
        meta: down(VK_LWIN) || down(VK_RWIN),
        shift: down(VK_SHIFT),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn function_keys_follow_each_other() {
        // `named_key` counts from F1.
        assert_eq!(key_codes::F2 as u32, key_codes::F1 as u32 + 1);
        assert_eq!(key_codes::F12 as u32, key_codes::F1 as u32 + 11);
        assert_eq!(key_codes::F24 as u32, key_codes::F1 as u32 + 23);
        assert_eq!(named_key(VK_F24.0, 0, false, false), Some(key_codes::F24));
    }

    #[test]
    fn modifiers_have_a_code_for_each_side() {
        assert_eq!(named_key(VK_SHIFT.0, 0x2a, false, false), Some(key_codes::Shift));
        assert_eq!(named_key(VK_SHIFT.0, 0x36, false, false), Some(key_codes::ShiftR));
        assert_eq!(named_key(VK_LSHIFT.0, 0, false, false), Some(key_codes::Shift));
        assert_eq!(named_key(VK_RSHIFT.0, 0, false, false), Some(key_codes::ShiftR));
        assert_eq!(named_key(VK_CONTROL.0, 0x1d, false, false), Some(key_codes::Control));
        assert_eq!(named_key(VK_CONTROL.0, 0x1d, true, false), Some(key_codes::ControlR));
        assert_eq!(named_key(VK_LWIN.0, 0, true, false), Some(key_codes::Meta));
        assert_eq!(named_key(VK_RWIN.0, 0, true, false), Some(key_codes::MetaR));
    }

    #[test]
    fn the_right_alt_is_altgr_only_with_a_layout_that_has_it() {
        assert_eq!(named_key(VK_MENU.0, 0x38, true, false), Some(key_codes::Alt));
        assert_eq!(named_key(VK_MENU.0, 0x38, true, true), Some(key_codes::AltGr));
        assert_eq!(named_key(VK_MENU.0, 0x38, false, true), Some(key_codes::Alt));
        assert_eq!(named_key(VK_RMENU.0, 0, false, true), Some(key_codes::AltGr));
    }

    #[test]
    fn character_keys_are_not_named() {
        assert_eq!(named_key(u16::from(b'A'), 0x1e, false, false), None);
        assert_eq!(named_key(VK_SPACE.0, 0x39, false, false), None);
    }
}
