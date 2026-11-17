// SPDX-License-Identifier: MIT OR Apache-2.0

//! Keyboard messages sent to the hidden HWND of a window, what reaches its focus scope, and the
//! clipboard.
//!
//! Like the mouse test, this sends messages with `SendMessageW`, which runs the window procedure
//! right away, and never shows the window. The platform belongs to the thread that first uses
//! it, so one test runs every scenario.
//!
//! Two things in the thread decide what a key message means, and the test sets both and puts
//! them back at the end:
//!
//! - The keyboard layout. The host asks `ToUnicodeEx` for the text of a key in the layout of the
//!   thread (`GetKeyboardLayout(0)`). The test loads the US layout and activates it with
//!   `ActivateKeyboardLayout` and no `KLF_SETFORPROCESS`, which changes the layout of this
//!   thread only: the user's input language, and the layout of the window they are typing in,
//!   stay as they are. Without the US layout, the test says so and passes without checking.
//! - The keyboard state. `ToUnicodeEx` reads it with `GetKeyboardState`, and a message sent with
//!   `SendMessageW` doesn't change it. The test writes the state of the thread with
//!   `SetKeyboardState`: all keys up to begin with, then the modifiers a scenario needs. The
//!   thread doesn't take keyboard messages out of a queue for the real keyboard, which would
//!   change the state back, and the window is never in the foreground.
//!
//! The dead key scenario uses the real message path instead: it posts the key messages, and
//! `TranslateMessage` makes the `WM_CHAR` messages from them, as the event loop does.

use i_slint_core::host::Win32WindowAdapter;
use i_slint_core::input::key_codes;
use i_slint_core::platform::Clipboard;
use slint::{CloseRequestResponse, ComponentHandle};
use std::cell::Cell;
use std::rc::Rc;
use windows::Win32::Foundation::*;
use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::w;

slint::include_modules!();

/// A key: its virtual key, scan code and extended flag, as the messages for it carry them.
#[derive(Clone, Copy)]
struct Key {
    vk: u16,
    scan: u32,
    extended: bool,
}

const fn vkey(vk: VIRTUAL_KEY, scan: u32, extended: bool) -> Key {
    Key { vk: vk.0, scan, extended }
}

/// A letter or digit key of the US layout; the virtual key is the character.
const fn letter(c: u8, scan: u32) -> Key {
    Key { vk: c as u16, scan, extended: false }
}

const A: Key = letter(b'A', 0x1e);
const C: Key = letter(b'C', 0x2e);
const E: Key = letter(b'E', 0x12);
const X: Key = letter(b'X', 0x2d);
const SHIFT: Key = vkey(VK_SHIFT, 0x2a, false);
const SHIFT_RIGHT: Key = vkey(VK_SHIFT, 0x36, false);
const CONTROL: Key = vkey(VK_CONTROL, 0x1d, false);
const CONTROL_RIGHT: Key = vkey(VK_CONTROL, 0x1d, true);
const ALT: Key = vkey(VK_MENU, 0x38, false);
const ALT_RIGHT: Key = vkey(VK_MENU, 0x38, true);
const F4: Key = vkey(VK_F4, 0x3e, false);
const UP: Key = vkey(VK_UP, 0x48, true);
/// The key next to the right Shift on a US keyboard: ' and ", a dead key in the US-International
/// layout.
const QUOTE: Key = vkey(VK_OEM_7, 0x28, false);

/// The keys that Slint gives a code of its own, with the code.
const NAMED_KEYS: [(Key, char); 19] = [
    (vkey(VK_BACK, 0x0e, false), key_codes::Backspace),
    (vkey(VK_TAB, 0x0f, false), key_codes::Tab),
    (vkey(VK_RETURN, 0x1c, false), key_codes::Return),
    (vkey(VK_ESCAPE, 0x01, false), key_codes::Escape),
    (vkey(VK_DELETE, 0x53, true), key_codes::Delete),
    (UP, key_codes::UpArrow),
    (vkey(VK_DOWN, 0x50, true), key_codes::DownArrow),
    (vkey(VK_LEFT, 0x4b, true), key_codes::LeftArrow),
    (vkey(VK_RIGHT, 0x4d, true), key_codes::RightArrow),
    (vkey(VK_INSERT, 0x52, true), key_codes::Insert),
    (vkey(VK_HOME, 0x47, true), key_codes::Home),
    (vkey(VK_END, 0x4f, true), key_codes::End),
    (vkey(VK_PRIOR, 0x49, true), key_codes::PageUp),
    (vkey(VK_NEXT, 0x51, true), key_codes::PageDown),
    (vkey(VK_F1, 0x3b, false), key_codes::F1),
    (vkey(VK_F5, 0x3f, false), key_codes::F5),
    (vkey(VK_F12, 0x58, false), key_codes::F12),
    (vkey(VK_F24, 0x76, false), key_codes::F24),
    (vkey(VK_APPS, 0x5d, true), key_codes::Menu),
];

struct Fixture {
    ui: KeyboardTest,
    hwnd: HWND,
}

impl Fixture {
    fn new() -> Self {
        let ui = KeyboardTest::new().unwrap();
        let hwnd = Win32WindowAdapter::from_window(ui.window())
            .unwrap()
            .create_hwnd_for_testing()
            .unwrap();
        Self { ui, hwnd }
    }

    /// Makes the window the active window of this thread, which is not the window the user types
    /// in: the foreground window stays what it was.
    fn activate(&self) {
        // The error is that there was no active window before.
        let _ = unsafe { SetActiveWindow(self.hwnd) };
    }

    fn send(&self, msg: u32, wp: usize, lp: isize) -> LRESULT {
        unsafe { SendMessageW(self.hwnd, msg, Some(WPARAM(wp)), Some(LPARAM(lp))) }
    }

    /// Sends a key message; `repeat` is the previous state of the key.
    fn key(&self, msg: u32, key: Key, repeat: bool) -> LRESULT {
        self.send(msg, key.vk as usize, key_lparam(msg, key, repeat))
    }

    fn down(&self, key: Key) -> LRESULT {
        self.key(WM_KEYDOWN, key, false)
    }

    fn up(&self, key: Key) -> LRESULT {
        self.key(WM_KEYUP, key, false)
    }

    /// Posts a key message to the queue, for `pump` to take out.
    fn post(&self, msg: u32, key: Key) {
        unsafe {
            PostMessageW(
                Some(self.hwnd),
                msg,
                WPARAM(key.vk as usize),
                LPARAM(key_lparam(msg, key, false)),
            )
            .unwrap()
        };
    }

    /// Presses and releases a key through the queue, as the event loop gets it: the key down is
    /// translated into `WM_CHAR` before the window procedure sees it, and the `WM_CHAR` is next
    /// in the queue.
    ///
    /// A real keyboard puts its messages in the input queue, and the thread takes the posted
    /// `WM_CHAR` before the next of them. `PostMessageW` has no such queue, so this posts one
    /// message at a time.
    fn type_key(&self, key: Key) {
        for msg in [WM_KEYDOWN, WM_KEYUP] {
            self.post(msg, key);
            pump();
        }
    }

    /// What the focus scope got so far: "+" and the text of every press, "-" and the text of every
    /// release.
    fn log(&self) -> String {
        self.ui.get_log().to_string()
    }

    fn clear_log(&self) {
        self.ui.set_log("".into());
    }
}

/// The `lParam` of a key message: repeat count 1, scan code, extended flag, context code of the
/// system keys, and the previous state and transition of a key up or a repeat.
fn key_lparam(msg: u32, key: Key, repeat: bool) -> isize {
    let up = matches!(msg, WM_KEYUP | WM_SYSKEYUP);
    let system = matches!(msg, WM_SYSKEYDOWN | WM_SYSKEYUP);
    let mut lp = 1 | (key.scan << 16);
    if key.extended {
        lp |= 1 << 24;
    }
    if system {
        lp |= 1 << 29;
    }
    if repeat || up {
        lp |= 1 << 30;
    }
    if up {
        lp |= 1 << 31;
    }
    lp as isize
}

/// Takes every message out of the queue the way the event loop does: `TranslateMessage`, which
/// makes `WM_CHAR` from the key messages, and then `DispatchMessageW`.
fn pump() {
    let mut msg = MSG::default();
    while unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE) }.as_bool() {
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

/// The keyboard layout of the thread, until it is dropped.
struct Layout {
    previous: HKL,
}

impl Layout {
    /// Activates the layout of a locale (`"00000409"` for US) for this thread only.
    fn activate(locale: windows::core::PCWSTR) -> Option<Self> {
        unsafe {
            let hkl = LoadKeyboardLayoutW(locale, KLF_NOTELLSHELL).ok()?;
            let previous = GetKeyboardLayout(0);
            ActivateKeyboardLayout(hkl, ACTIVATE_KEYBOARD_LAYOUT_FLAGS(0)).ok()?;
            let this = Self { previous };
            (GetKeyboardLayout(0) == hkl).then_some(this)
        }
    }
}

impl Drop for Layout {
    fn drop(&mut self) {
        unsafe {
            let _ = ActivateKeyboardLayout(self.previous, ACTIVATE_KEYBOARD_LAYOUT_FLAGS(0));
        }
    }
}

/// The keyboard state of the thread, with every key up until [`Keys::hold`] says otherwise; the
/// state from before comes back when it is dropped.
struct Keys {
    saved: [u8; 256],
}

impl Keys {
    fn new() -> Self {
        let mut saved = [0u8; 256];
        unsafe { GetKeyboardState(&mut saved).unwrap() };
        let this = Self { saved };
        this.hold(&[]);
        this
    }

    /// Holds exactly these keys. A modifier holds its generic virtual key and its side's:
    /// `Shift` holds `VK_SHIFT` and `VK_LSHIFT`.
    fn hold(&self, keys: &[Modifier]) {
        let mut state = [0u8; 256];
        for key in keys {
            for vk in key.virtual_keys() {
                state[vk.0 as usize] = 0x80;
            }
            // AltGr is Ctrl+Alt.
            if matches!(key, Modifier::AltGr) {
                state[VK_CONTROL.0 as usize] = 0x80;
                state[VK_LCONTROL.0 as usize] = 0x80;
            }
        }
        unsafe { SetKeyboardState(&state).unwrap() };
    }

    /// The Caps Lock toggle is on, and nothing else.
    fn caps_lock(&self) {
        let mut state = [0u8; 256];
        state[VK_CAPITAL.0 as usize] = 0x01;
        unsafe { SetKeyboardState(&state).unwrap() };
    }
}

impl Drop for Keys {
    fn drop(&mut self) {
        unsafe {
            let _ = SetKeyboardState(&self.saved);
        }
    }
}

#[derive(Clone, Copy)]
enum Modifier {
    Shift,
    Control,
    Alt,
    /// The right Alt of a layout that has AltGr, which the system holds as Ctrl+Alt.
    AltGr,
}

impl Modifier {
    fn virtual_keys(self) -> [VIRTUAL_KEY; 2] {
        match self {
            Modifier::Shift => [VK_SHIFT, VK_LSHIFT],
            Modifier::Control => [VK_CONTROL, VK_LCONTROL],
            Modifier::Alt => [VK_MENU, VK_LMENU],
            Modifier::AltGr => [VK_MENU, VK_RMENU],
        }
    }
}

/// The text of a key code, as `KeyEvent::text` has it.
fn code(c: char) -> String {
    c.to_string()
}

fn a_letter_reaches_the_focus_scope() {
    let f = Fixture::new();
    assert!(f.ui.get_focused());

    assert_eq!(f.down(A), LRESULT(0));
    assert_eq!(f.ui.get_pressed(), "a");
    assert_eq!(f.ui.get_presses(), 1);
    assert!(!f.ui.get_repeat());
    let m = f.ui.get_modifiers();
    assert!(!m.shift && !m.control && !m.alt && !m.meta);

    f.up(A);
    assert_eq!(f.ui.get_released(), "a");
    assert_eq!(f.ui.get_releases(), 1);
    assert_eq!(f.log(), "+a-a");

    // Digits and punctuation come from the layout too.
    f.clear_log();
    for (key, text) in [
        (letter(b'1', 0x02), "1"),
        (vkey(VK_OEM_PERIOD, 0x34, false), "."),
        (vkey(VK_SPACE, 0x39, false), " "),
        (vkey(VK_NUMPAD7, 0x47, false), "7"),
    ] {
        f.down(key);
        f.up(key);
        assert_eq!(f.ui.get_pressed(), text);
        assert_eq!(f.ui.get_released(), text);
    }
}

fn shift_and_caps_lock_change_the_letter(keys: &Keys) {
    let f = Fixture::new();
    keys.hold(&[Modifier::Shift]);
    f.down(SHIFT);
    f.down(A);
    assert_eq!(f.ui.get_pressed(), "A");
    // The modifiers come from the key events that Slint got, not from the keyboard state.
    assert!(f.ui.get_modifiers().shift);
    f.down(letter(b'2', 0x03));
    assert_eq!(f.ui.get_pressed(), "@");
    f.up(A);
    assert_eq!(f.ui.get_released(), "A");
    assert!(f.ui.get_released_modifiers().shift);
    f.up(SHIFT);
    keys.hold(&[]);
    assert!(!f.ui.get_released_modifiers().shift);

    keys.caps_lock();
    f.down(A);
    assert_eq!(f.ui.get_pressed(), "A");
    assert!(!f.ui.get_modifiers().shift);
    f.up(A);
    keys.hold(&[]);
}

fn ctrl_c_gives_the_letter_to_shortcuts(keys: &Keys) {
    let f = Fixture::new();
    keys.hold(&[Modifier::Control]);
    f.down(CONTROL);
    // ToUnicodeEx gives U+0003 for this: the text is the letter, as without Ctrl.
    f.down(C);
    assert_eq!(f.ui.get_pressed(), "c");
    assert!(f.ui.get_modifiers().control);
    assert!(!f.ui.get_modifiers().shift);
    f.up(C);
    assert_eq!(f.ui.get_released(), "c");

    // Shift stays in the text.
    keys.hold(&[Modifier::Control, Modifier::Shift]);
    f.down(SHIFT);
    f.down(C);
    assert_eq!(f.ui.get_pressed(), "C");
    assert!(f.ui.get_modifiers().control && f.ui.get_modifiers().shift);
    f.up(C);
    f.up(SHIFT);

    // Without AltGr, Ctrl+Alt gives no character: the letter is the text, and both modifiers stay.
    keys.hold(&[Modifier::Control, Modifier::Alt]);
    f.key(WM_SYSKEYDOWN, ALT, false);
    f.down(X);
    assert_eq!(f.ui.get_pressed(), "x");
    let m = f.ui.get_modifiers();
    assert!(m.control && m.alt);
    f.up(X);
    f.key(WM_KEYUP, ALT, false);
    f.up(CONTROL);
    keys.hold(&[]);
    assert!(!f.ui.get_released_modifiers().control);
}

fn keys_without_a_character_have_key_codes() {
    let f = Fixture::new();
    for (key, expected) in NAMED_KEYS {
        f.down(key);
        f.up(key);
        assert_eq!(f.ui.get_pressed().as_str(), code(expected), "{:#x}", key.vk);
        assert_eq!(f.ui.get_released().as_str(), code(expected), "{:#x}", key.vk);
    }
    assert_eq!(f.ui.get_presses(), NAMED_KEYS.len() as i32);

    // Keys with no code and no character are no event. Caps Lock has a code, but upstream's
    // winit backend sends nothing for it.
    f.clear_log();
    for key in [
        vkey(VK_NUMLOCK, 0x45, true),
        vkey(VK_VOLUME_UP, 0x30, true),
        vkey(VK_CAPITAL, 0x3a, false),
    ] {
        f.down(key);
        f.up(key);
    }
    assert_eq!(f.log(), "");
}

fn modifier_keys_have_a_code_per_side() {
    let f = Fixture::new();
    for (key, expected) in [
        (SHIFT, key_codes::Shift),
        (SHIFT_RIGHT, key_codes::ShiftR),
        (vkey(VK_LSHIFT, 0x2a, false), key_codes::Shift),
        (vkey(VK_RSHIFT, 0x36, false), key_codes::ShiftR),
        (CONTROL, key_codes::Control),
        (CONTROL_RIGHT, key_codes::ControlR),
        (vkey(VK_LWIN, 0x5b, true), key_codes::Meta),
        (vkey(VK_RWIN, 0x5c, true), key_codes::MetaR),
        (ALT, key_codes::Alt),
        // The US layout has no AltGr.
        (ALT_RIGHT, key_codes::Alt),
    ] {
        let system = key.vk == VK_MENU.0;
        f.key(if system { WM_SYSKEYDOWN } else { WM_KEYDOWN }, key, false);
        assert_eq!(f.ui.get_pressed().as_str(), code(expected), "{:#x}", key.vk);
        // A modifier key is a key down, and then the letters carry the modifier.
        f.down(A);
        let m = f.ui.get_modifiers();
        match expected {
            key_codes::Shift | key_codes::ShiftR => assert!(m.shift),
            key_codes::Control | key_codes::ControlR => assert!(m.control),
            key_codes::Alt => assert!(m.alt),
            _ => assert!(m.meta),
        }
        f.up(A);
        f.up(key);
        assert_eq!(f.ui.get_released().as_str(), code(expected), "{:#x}", key.vk);
        let m = f.ui.get_released_modifiers();
        assert!(!m.shift && !m.control && !m.alt && !m.meta, "{:#x}", key.vk);
    }
}

fn a_held_key_repeats() {
    let f = Fixture::new();
    f.key(WM_KEYDOWN, A, false);
    assert!(!f.ui.get_repeat());
    // Bit 30: the key was down already.
    f.key(WM_KEYDOWN, A, true);
    assert!(f.ui.get_repeat());
    f.key(WM_KEYDOWN, A, true);
    assert!(f.ui.get_repeat());
    f.up(A);
    assert_eq!(f.log(), "+a+a+a-a");
    // The next press is a new one.
    f.down(A);
    assert!(!f.ui.get_repeat());
    f.up(A);
}

fn characters_that_no_key_produced_arrive_from_wm_char(keys: &Keys) {
    let f = Fixture::new();
    // SendInput with KEYEVENTF_UNICODE: VK_PACKET, and the character in WM_CHAR.
    let packet = vkey(VK_PACKET, 0, false);
    f.down(packet);
    assert_eq!(f.log(), "");
    f.send(WM_CHAR, 0xe9, 0);
    f.up(packet);
    assert_eq!(f.log(), "+é");

    // A character outside the basic plane comes as two units.
    f.send(WM_CHAR, 0xd83d, 0);
    assert_eq!(f.log(), "+é");
    f.send(WM_CHAR, 0xde00, 0);
    assert_eq!(f.log(), "+é+😀");

    // An Alt code: the system collects the digits, and gives WM_CHAR when Alt is released.
    f.clear_log();
    keys.hold(&[Modifier::Alt]);
    f.key(WM_SYSKEYDOWN, ALT, false);
    let digit = vkey(VK_NUMPAD0, 0x52, false);
    f.key(WM_SYSKEYDOWN, digit, false);
    f.key(WM_SYSKEYUP, digit, false);
    keys.hold(&[]);
    f.key(WM_KEYUP, ALT, false);
    f.send(WM_CHAR, 0xe9, 0);
    // Alt and its release, and the character; the digits are no keys.
    assert_eq!(f.log(), format!("+{0}-{0}+é", code(key_codes::Alt)));

    // Control characters are keys, and have their key down.
    f.clear_log();
    f.send(WM_CHAR, 0x03, 0);
    f.send(WM_CHAR, 0x08, 0);
    assert_eq!(f.log(), "");
    // Return is a new line.
    f.send(WM_CHAR, 0x0d, 0);
    assert_eq!(f.log(), "+\n");
}

fn characters_of_a_key_are_not_sent_twice() {
    let f = Fixture::new();
    // Through the queue, the key down is followed by the WM_CHAR that TranslateMessage made.
    f.type_key(A);
    f.type_key(vkey(VK_RETURN, 0x1c, false));
    assert_eq!(f.log(), format!("+a-a+{0}-{0}", code(key_codes::Return)));
}

fn dead_keys_compose_with_the_next_key() {
    let Some(_layout) = Layout::activate(w!("00020409")) else {
        eprintln!("skipped: the US-International layout is not installed");
        return;
    };
    let f = Fixture::new();
    // ' is dead: the press shows nothing, and the next key composes with it.
    f.type_key(QUOTE);
    f.type_key(E);
    assert_eq!(f.log(), "+é-e");

    // A key that doesn't compose gives the accent and the key.
    f.clear_log();
    f.type_key(QUOTE);
    f.type_key(X);
    assert_eq!(f.log(), "+'x-x");

    // And the key after that is a plain one again.
    f.clear_log();
    f.type_key(E);
    assert_eq!(f.log(), "+e-e");
}

fn altgr_gives_the_third_level_of_a_key(keys: &Keys) {
    let Some(_layout) = Layout::activate(w!("00000407")) else {
        eprintln!("skipped: the German layout is not installed");
        return;
    };
    let f = Fixture::new();

    // AltGr: the system sends a Ctrl before the right Alt, and the Ctrl is no key.
    keys.hold(&[Modifier::AltGr]);
    f.post(WM_KEYDOWN, CONTROL);
    f.post(WM_SYSKEYDOWN, ALT_RIGHT);
    pump();
    assert_eq!(f.log(), format!("+{}", code(key_codes::AltGr)));
    f.type_key(E);
    assert_eq!(f.log(), format!("+{}+\u{20ac}-\u{20ac}", code(key_codes::AltGr)));
    // AltGr is not Ctrl and Alt to Slint.
    let m = f.ui.get_modifiers();
    assert!(!m.control && !m.alt && !m.shift);
    f.post(WM_KEYUP, CONTROL);
    f.post(WM_SYSKEYUP, ALT_RIGHT);
    pump();
    keys.hold(&[]);
    assert_eq!(f.log(), format!("+{0}+\u{20ac}-\u{20ac}-{0}", code(key_codes::AltGr)));

    // Ctrl+Alt is AltGr to the layout, and the key without them tells Slint that it was: the
    // modifiers don't include them, so that the text is typed and no shortcut is meant.
    f.clear_log();
    keys.hold(&[Modifier::Control, Modifier::Alt]);
    f.post(WM_KEYDOWN, CONTROL);
    f.post(WM_SYSKEYDOWN, ALT);
    pump();
    f.type_key(E);
    assert_eq!(f.ui.get_pressed(), "\u{20ac}");
    let m = f.ui.get_modifiers();
    assert!(!m.control && !m.alt);
    f.post(WM_KEYUP, CONTROL);
    f.post(WM_SYSKEYUP, ALT);
    pump();
    keys.hold(&[]);
    let m = f.ui.get_released_modifiers();
    assert!(!m.control && !m.alt);
}

fn activation_follows_the_window_and_releases_modifiers(keys: &Keys) {
    let f = Fixture::new();
    let activate = |state: u32| f.send(WM_ACTIVATE, state as usize, 0);

    activate(WA_INACTIVE);
    assert!(!f.ui.get_focused());
    activate(WA_ACTIVE);
    assert!(f.ui.get_focused());
    activate(WA_INACTIVE);
    activate(WA_CLICKACTIVE);
    assert!(f.ui.get_focused());

    // The Shift that Slint saw go down is not held any more when the window is activated.
    f.down(SHIFT);
    f.down(A);
    assert!(f.ui.get_modifiers().shift);
    f.up(A);
    activate(WA_ACTIVE);
    f.down(A);
    assert!(!f.ui.get_modifiers().shift);
    f.up(A);

    // One that is still held stays.
    keys.hold(&[Modifier::Shift]);
    f.down(SHIFT);
    activate(WA_ACTIVE);
    f.down(A);
    assert!(f.ui.get_modifiers().shift);
    f.up(A);
    f.up(SHIFT);
    keys.hold(&[]);

    // Deactivating forgets all of them.
    f.down(CONTROL);
    activate(WA_INACTIVE);
    f.down(A);
    assert!(!f.ui.get_modifiers().control);
    f.up(A);
    activate(WA_ACTIVE);
}

fn alt_f4_closes_the_window() {
    let f = Fixture::new();
    // DefWindowProc turns Alt+F4 into a close request only for the active window of the thread.
    f.activate();
    let close_requests = Rc::new(Cell::new(0));
    f.ui.window().on_close_requested({
        let close_requests = close_requests.clone();
        move || {
            close_requests.set(close_requests.get() + 1);
            // The window stays: the test goes on with it.
            CloseRequestResponse::KeepWindowShown
        }
    });

    // The focus scope accepts every key, but as in winit, Alt+F4 never reaches it: it goes to
    // DefWindowProc, which turns it into WM_SYSCOMMAND(SC_CLOSE), and that into WM_CLOSE,
    // which asks the window to close.
    f.key(WM_SYSKEYDOWN, ALT, false);
    f.key(WM_SYSKEYDOWN, F4, false);
    pump();
    assert_eq!(close_requests.get(), 1);
    f.key(WM_SYSKEYUP, F4, false);
    f.key(WM_KEYUP, ALT, false);
    // The scope saw Alt, and F4's release, as winit reports them.
    assert_eq!(f.log(), format!("+{0}-{1}-{0}", code(key_codes::Alt), code(key_codes::F4)));
    assert_eq!(close_requests.get(), 1);

    // When the focus scope rejects the keys, it is the same.
    f.ui.set_accept_keys(false);
    f.key(WM_SYSKEYDOWN, ALT, false);
    f.key(WM_SYSKEYDOWN, F4, false);
    pump();
    assert_eq!(close_requests.get(), 2);
    f.key(WM_SYSKEYUP, F4, false);
    f.key(WM_KEYUP, ALT, false);

    // Keys without Alt go to the focus scope, and when it rejects them, nothing happens.
    f.down(F4);
    f.up(F4);
    pump();
    assert_eq!(close_requests.get(), 2);
}

/// Opens the clipboard with no owner, for the test to look at it and clear it.
fn clear_clipboard() {
    unsafe {
        OpenClipboard(None).unwrap();
        EmptyClipboard().unwrap();
        CloseClipboard().unwrap();
    }
}

fn the_clipboard_round_trips_text() {
    // The previous text, for the end: the test replaces the content of the user's clipboard.
    let (previous, result) = i_slint_core::with_existing_context(|ctx| {
        let platform = ctx.platform();
        let previous = platform.clipboard_text(Clipboard::DefaultClipboard);
        let text = "Clipboard test\nsecond line: é, 日本語, 😀";

        platform.set_clipboard_text(text, Clipboard::DefaultClipboard);
        let default = platform.clipboard_text(Clipboard::DefaultClipboard);
        // There is no selection clipboard on Windows: it takes nothing, and has nothing.
        platform.set_clipboard_text("not for the selection", Clipboard::SelectionClipboard);
        let selection = platform.clipboard_text(Clipboard::SelectionClipboard);
        let after_selection = platform.clipboard_text(Clipboard::DefaultClipboard);

        platform.set_clipboard_text("", Clipboard::DefaultClipboard);
        let empty = platform.clipboard_text(Clipboard::DefaultClipboard);

        // Windows programs write CRLF; the text that comes back has new lines.
        (previous, (text, default, selection, after_selection, empty))
    })
    .unwrap();
    let (text, default, selection, after_selection, empty) = result;

    // Restore before checking, so that a failure doesn't leave the test text behind.
    i_slint_core::with_existing_context(|ctx| match &previous {
        Some(previous) => ctx.platform().set_clipboard_text(previous, Clipboard::DefaultClipboard),
        None => clear_clipboard(),
    })
    .unwrap();

    assert_eq!(default.as_deref(), Some(text));
    assert_eq!(selection, None);
    assert_eq!(after_selection.as_deref(), Some(text));
    assert_eq!(empty.as_deref(), Some(""));
}

#[test]
fn keyboard_messages() {
    let Some(_layout) = Layout::activate(w!("00000409")) else {
        eprintln!("skipped: the US layout is not installed, and the texts depend on it");
        return;
    };
    let keys = Keys::new();

    a_letter_reaches_the_focus_scope();
    shift_and_caps_lock_change_the_letter(&keys);
    ctrl_c_gives_the_letter_to_shortcuts(&keys);
    keys_without_a_character_have_key_codes();
    modifier_keys_have_a_code_per_side();
    a_held_key_repeats();
    characters_that_no_key_produced_arrive_from_wm_char(&keys);
    characters_of_a_key_are_not_sent_twice();
    dead_keys_compose_with_the_next_key();
    altgr_gives_the_third_level_of_a_key(&keys);
    activation_follows_the_window_and_releases_modifiers(&keys);
    alt_f4_closes_the_window();
    the_clipboard_round_trips_text();
}
