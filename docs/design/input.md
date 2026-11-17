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
- The code is `crates/core/host/input.rs`.

## Keyboard

- `WM_KEYDOWN`, `WM_SYSKEYDOWN`, `WM_KEYUP` and `WM_SYSKEYUP` produce `InternalKeyEvent`s.
  `WM_CHAR` only produces text that no key-down produced: `VK_PACKET` input from `SendInput`, and Alt codes.
- Key text, as in winit's Windows keyboard handling:
  - Keys without a character map to Slint's key codes (`i_slint_common::key_codes`) through a virtual-key table.
  - Character keys get their text from `ToUnicodeEx` with the current keyboard state.
    `text_without_modifiers` comes from the same call without Ctrl and Alt.
    The flag that leaves the kernel's dead-key state alone (`0x4`, Windows 10 1607 and later) keeps dead keys composing.
  - When Ctrl turns the text into a control character (Ctrl+C gives U+0003), the text is `text_without_modifiers`, so shortcuts see `c`.
- Auto-repeat (bit 30 of `lParam`) sets `KeyEvent::repeat`.
- `WM_ACTIVATE` dispatches `WindowActiveChanged`; on activation, `release_modifiers_not_held` with the modifiers `GetKeyState` reports.
- Input methods are Phase 4, with text input.

## Clipboard

`Platform::set_clipboard_text` and `clipboard_text` use `CF_UNICODETEXT` through the message window.
Only `Clipboard::DefaultClipboard` exists on Windows.

## Testing

- `crates/testing` gets upstream's injection functions (`send_mouse_click`, `send_keyboard_char`, `send_keyboard_string_sequence`, and so on).
  They dispatch through `Window::dispatch_event`, as upstream's do, so upstream's `tests/cases/input` and `focus` run unchanged.
- The Win32 translation is tested separately in `tests/input`: a component from the upstream compiler gets a hidden HWND
  (`Win32WindowAdapter::create_hwnd_for_testing`), synthesized messages go to it through `SendMessageW`,
  and the test checks what reaches a `TouchArea` through the component's properties.
  The platform belongs to the first thread that uses it, so one test runs all scenarios in turn.
  Keyboard tests that depend on the keyboard layout pin it with `LoadKeyboardLayoutW("00000409")` or skip with a reason.
