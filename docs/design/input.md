# Input (Phase 3)

The host turns Win32 messages into the events `WindowInner` already handles.
Hit testing, focus, click counting, gestures and the input items stay in the upstream core (`input.rs`, `items/input_items.rs`), unchanged.
Upstream's winit backend (`internal/backends/winit/winitwindowadapter.rs`) is the reference for what each event carries.

## Pointer

- The mouse comes through `WM_MOUSEMOVE`, `WM_[LRMX]BUTTON{DOWN,UP,DBLCLK}`, `WM_MOUSEWHEEL`, `WM_MOUSEHWHEEL` and `WM_MOUSELEAVE`.
  Don't use `EnableMouseInPointer`: it's process-wide and can't be undone, which matters to a host application that embeds Slint.
- Touch and pen come through `WM_POINTERDOWN`, `WM_POINTERUPDATE`, `WM_POINTERUP` and `WM_POINTERCAPTURECHANGED`.
  The handler doesn't pass them to `DefWindowProc`, so Windows doesn't also promote them to mouse messages.
  - Touch (`PT_TOUCH`) becomes `InternalEvent::Touch`, with finger ids handed out per pointer id while it's down,
    reused once lifted, as winit's `touch_finger_ids` does.
  - Pen (`PT_PEN`) becomes mouse events with the left button, or the right one while the barrel button is held.
  - A cancelled contact (`POINTER_FLAG_CANCELED`, or `WM_POINTERCAPTURECHANGED`) ends the touch as `Cancelled`, and the pen's press as `Exit`.
    The core treats a cancelled single finger as a release, as upstream does, so it can still click.
  - Updates of a pointer that came down on another window are ignored.
  - The code is `crates/core/host/pointer.rs`.
    Tests can't make Windows report a pointer without acting on the real screen,
    so `Win32WindowAdapter::pointer_message_for_testing` takes what `GetPointerInfo` would return.
- Positions are client pixels divided by the window's scale factor: logical coordinates.
- Presses and releases carry `click_count: 0`; the core counts clicks with the platform's `click_interval`.
- On a press, `SetCapture`; on the release of the last button, `ReleaseCapture`.
  `WM_CAPTURECHANGED` while a button is down (another window took the mouse) dispatches `Exit`:
  the press ends without a release, so nothing counts it as a click.
- The first move after entering arms `TrackMouseEvent(TME_LEAVE)`.
  `WM_MOUSELEAVE` dispatches `BackendMouseEvent::Exit` unless a button is down, as winit does;
  then the release of the last button dispatches it.
- The window class has no `CS_DBLCLKS`, so a second click is a plain button down.
- Wheel: `delta / WHEEL_DELTA` lines, 60 logical pixels per line, as winit converts line deltas.
  Use winit's Windows signs for both axes.
  A precision touchpad sends smaller deltas through the same messages, and that's fine.
- Cursor: `WM_SETCURSOR` with `HTCLIENT` sets the cursor that `WindowAdapterInternal::set_mouse_cursor` last asked for,
  mapped to `IDC_*`; other hit-test codes go to `DefWindowProc`.
  A change while the pointer is over the client area applies right away.
  Image cursors show the arrow until Phase 5.
- `WindowMoveArea` asks for `start_window_move` while the window dispatches a mouse move.
  The host posts itself a message, and from there, if the left button is still down, ends the press with `Exit`,
  releases the capture and sends `WM_NCLBUTTONDOWN` with `HTCAPTION`: the system's move loop, which returns on the button up.
  Tests stop at the posted message, since the loop waits for the real button; the move itself is a manual check.
- The code is `crates/core/host/input.rs`.

## Keyboard

- `WM_KEYDOWN`, `WM_SYSKEYDOWN`, `WM_KEYUP` and `WM_SYSKEYUP` produce `InternalKeyEvent`s.
  `WM_CHAR` only produces text that no key-down produced: `VK_PACKET` input from `SendInput`, and Alt codes.
  The code is `crates/core/host/keyboard.rs`.
- Key text, as in winit's Windows keyboard handling:
  - Keys without a character map to Slint's key codes (`i_slint_common::key_codes`) through a virtual-key table:
    Backspace, Tab, Return, Escape, Delete, the arrows, Insert, Home, End, Page Up and Down, F1 to F24, Scroll Lock, Pause, Print Screen (`SysReq`),
    the menu key, and the modifiers with a code for each side (`Shift`/`ShiftR`, `Control`/`ControlR`, `Meta`/`MetaR`, `Alt`).
    The right Alt is `AltGr` on a layout that has one: some key gives another character with Ctrl+Alt than without, which is how winit finds out.
    Caps Lock has a code too, but upstream's winit backend sends nothing for it, and neither does this host.
    The keys that Slint has no code for (Num Lock, media and browser keys, and so on) have no character either, and give no event.
  - Character keys get their text from `ToUnicodeEx` with the current keyboard state, in the layout of the thread (`GetKeyboardLayout(0)`).
    `text_without_modifiers` comes from the same call without Ctrl and Alt.
    The flag that leaves the kernel's dead-key state alone (`0x4`, Windows 10 1607 and later) keeps the call from using up or setting a dead key.
  - `ToUnicodeEx` alone doesn't compose dead keys:
    the event loop calls `TranslateMessage` before the window procedure sees the key down,
    and by then the dead key is pending (`ToUnicodeEx` of the dead key itself gives its character twice) or used up (the key after it gives `e`, not `é`).
    So a key down first takes the `WM_CHAR`, `WM_SYSCHAR` and `WM_DEADCHAR` messages that `TranslateMessage` queued for it out of the queue
    (they come first in the queue, as hardware input waits behind posted messages), and their text is the text of the event.
    A `WM_DEADCHAR` makes the key a dead key: no event, and no release for it.
    `ToUnicodeEx` gives the text when there are no such messages, as for a message sent with `SendMessage`.
  - When Ctrl turns the text into a control character (Ctrl+C gives U+0003), the text is `text_without_modifiers`, so shortcuts see `c`.
    Ctrl+Alt on a layout without AltGr gives no character at all, and is the same.
  - The system sends a Ctrl before an AltGr, as AltGr is Ctrl+Alt: that Ctrl, found by looking at the next key message, isn't a key.
  - The digits of an Alt code (Alt+0233 on the numeric keypad) aren't keys either; `WM_CHAR` delivers the character when Alt is released.
- Auto-repeat (bit 30 of `lParam`) sets `KeyEvent::repeat`.
- As in winit, every `WM_SYSKEYDOWN` also goes to `DefWindowProc`, whatever Slint did with it, which closes the window on Alt+F4 and opens the system menu on Alt+Space.
  The key down of Alt+F4 isn't dispatched to Slint at all, so no `FocusScope` can keep the window from closing.
  Neither `WM_SYSKEYUP` nor the release of Alt does: with no menu bar, `DefWindowProc` would start the system's menu mode on Alt, which takes the keyboard from the window.
  The `WM_SYSCHAR` that Alt+Space made is out of the queue by then, and goes to `DefWindowProc` too.
- `WM_ACTIVATE` dispatches `WindowActiveChanged`; on activation, `release_modifiers_not_held` with the modifiers `GetKeyState` reports.
- Posting a key down and its key up to the queue together (`PostMessage`) puts the `WM_CHAR` behind the key up, where the key down can't find it,
  and the text arrives a second time as `WM_CHAR`. A real keyboard, `SendInput` and a pump that posts one message at a time don't.
- Input methods are Phase 4, with text input.
  Until then the text an input method commits arrives as `WM_CHAR`, and types like `SendInput` text.

## Clipboard

`Platform::set_clipboard_text` and `clipboard_text` use `CF_UNICODETEXT` through the message window, which owns the clipboard while it is open.
Only `Clipboard::DefaultClipboard` exists on Windows: setting the selection clipboard does nothing, and reading it gives `None`.
Windows programs exchange CRLF line breaks: text is written with them and read back with `\n`.
`OpenClipboard` fails while another program has the clipboard open, so it is tried again a few times, 5 ms apart.
A failure to write is logged with `debug_log`, as the trait has nowhere to report it.

## Testing

- `crates/testing` gets upstream's injection functions (`send_mouse_click`, `send_keyboard_char`, `send_keyboard_string_sequence`, and so on).
  They dispatch through `Window::dispatch_event`, as upstream's do, so upstream's `tests/cases/input` and `focus` run unchanged.
- The Win32 translation is tested separately in `tests/input`: a component from the upstream compiler gets a hidden HWND
  (`Win32WindowAdapter::create_hwnd_for_testing`), synthesized messages go to it through `SendMessageW`,
  and the test checks what reaches a `TouchArea` through the component's properties.
  The platform belongs to the first thread that uses it, so one test runs all scenarios in turn.
  Keyboard tests that depend on the keyboard layout pin it with `LoadKeyboardLayoutW("00000409")` or skip with a reason.
  The host asks for the layout of the thread, so the test activates the layout with `ActivateKeyboardLayout` and no `KLF_SETFORPROCESS`:
  that changes the layout of the test thread only, never the user's input language, and the test puts the old layout back.
  Dead keys use the US-International layout (`00020409`), and AltGr the German one (`00000407`); each scenario skips if its layout is missing.
- `ToUnicodeEx` reads the thread's keyboard state, which `SendMessageW` doesn't change: the test writes it with `SetKeyboardState`
  (every key up, then the modifiers a scenario needs) and restores it at the end.
  The thread takes no keyboard input from the real keyboard, and the window is never in the foreground, so nothing changes it back.
  `GetKeyState`, which the activation uses, reads the same table.
- A scenario that depends on `TranslateMessage` (dead keys, AltGr, text sent once) posts the key messages one at a time and pumps the queue
  with `TranslateMessage` and `DispatchMessageW`, as the event loop does.
- `DefWindowProc` turns Alt+F4 into `SC_CLOSE` only for the active window of the thread, shown or not.
  The Alt+F4 scenario makes the hidden window the active window of the test thread with `SetActiveWindow`, which leaves the foreground window alone.
- The clipboard test replaces the content of the user's clipboard and puts the text back at the end;
  a clipboard that held something other than text is emptied.
