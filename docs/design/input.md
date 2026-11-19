# Input (Phase 3)

The host turns Win32 messages into the events `WindowInner` already handles.
Hit testing, focus, click counting, gestures and the input items stay in the upstream core, unchanged.
Upstream's winit backend (`internal/backends/winit/winitwindowadapter.rs`) is the reference for what each event carries.

## Pointer

- Why not `EnableMouseInPointer`: it's process-wide and can't be undone, which matters to a host application that embeds Slint. The mouse stays on mouse messages; touch and pen come through `WM_POINTER*`.
- Touch and pen messages don't go to `DefWindowProc`, or Windows also promotes them to mouse messages.
  - Touch finger ids are handed out per pointer id while it's down and reused once lifted, as winit's `touch_finger_ids`.
  - The pen is the mouse: the left button, or the right one while the barrel button is held.
  - A cancelled contact ends the touch as `Cancelled`, which the core treats as a release for a single finger, as upstream does, so it can still click.
  - Tests can't make Windows report a pointer without acting on the real screen, so `pointer_message_for_testing` takes what `GetPointerInfo` would return.
- `WM_CAPTURECHANGED` while a button is down (another window took the mouse) dispatches `Exit`: the press ends without a release, so nothing counts it as a click.
- `WM_MOUSELEAVE` dispatches `Exit` unless a button is down, as winit does; then the release of the last button dispatches it.
- No `CS_DBLCLKS`: the core counts clicks itself, with the platform's `click_interval`, so a second click must be a plain button down.
- Wheel: 60 logical pixels per line and winit's signs, so scrolling matches upstream. A precision touchpad sends smaller deltas through the same messages, and that's fine.
- A cursor change while the pointer is over the client area applies right away, without waiting for `WM_SETCURSOR`. Image cursors show the arrow.
- `WindowMoveArea` asks for `start_window_move` while the window dispatches a mouse move, so the system's move loop starts from a posted message, once that dispatch is over.
  From there, if the left button is still down, ends the press with `Exit`, releases the capture and sends `WM_NCLBUTTONDOWN` with `HTCAPTION`: the move loop returns on the button up.
  Tests stop at the posted message, since the loop waits for the real button; the move itself is a manual check.

## Keyboard

- `WM_CHAR` only produces text that no key-down produced: `VK_PACKET` input from `SendInput`, and Alt codes.
- Key text follows winit's Windows keyboard handling.
  - The right Alt is `AltGr` on a layout that has one: some key gives another character with Ctrl+Alt than without, which is how winit finds out.
  - Caps Lock has a key code, but upstream's winit backend sends nothing for it, and neither does this host. Keys that Slint has no code for (Num Lock, media keys) give no event.
  - `ToUnicodeEx` is called with the flag that leaves the kernel's dead-key state alone (`0x4`, Windows 10 1607 and later), or the call uses up or sets a dead key.
- Dead keys, and the order of messages: `ToUnicodeEx` alone doesn't compose them.
  The event loop calls `TranslateMessage` before the window procedure sees the key down, and by then the dead key is pending (`ToUnicodeEx` of the dead key itself gives its character twice) or used up (the key after it gives `e`, not `é`).
  So a key down first takes the `WM_CHAR`, `WM_SYSCHAR` and `WM_DEADCHAR` that `TranslateMessage` queued for it out of the queue (they come first, as hardware input waits behind posted messages), and their text is the event's text.
  A `WM_DEADCHAR` makes the key a dead key: no event, and no release for it.
  `ToUnicodeEx` gives the text when there are no such messages, as for a message sent with `SendMessage`.
- Posting a key down and its key up together (`PostMessage`) puts the `WM_CHAR` behind the key up, where the key down can't find it, and the text arrives a second time. A real keyboard, `SendInput` and a pump that posts one message at a time don't.
- When Ctrl turns the text into a control character (Ctrl+C gives U+0003), the text is the one without modifiers, so shortcuts see `c`.
- The system sends a Ctrl before an AltGr, as AltGr is Ctrl+Alt: that Ctrl, found by looking at the next key message, isn't a key.
- The digits of an Alt code aren't keys either; `WM_CHAR` delivers the character when Alt is released.
- Every `WM_SYSKEYDOWN` also goes to `DefWindowProc`, as in winit, which closes the window on Alt+F4 and opens the system menu on Alt+Space.
  The key down of Alt+F4 isn't dispatched to Slint at all, so no `FocusScope` can keep the window from closing.
  Neither `WM_SYSKEYUP` nor the release of Alt goes there: with no menu bar, `DefWindowProc` would start the system's menu mode on Alt, which takes the keyboard from the window.
- On activation, the host releases the modifiers that `GetKeyState` no longer reports: their key ups went to another window.

## Input Method

IMM32, as winit uses it on Windows ([S4](spike-s4-text.md)).
The rest of the host only calls `ime.rs`'s `window_created`, `request`, `handle` and `deactivating`, so TSF can replace it (for reconversion, which upstream lacks too).

- A new window has no input context, as in winit: typing goes to `WM_CHAR` until a text input asks for the input method.
- A password input is treated as `Disable`, as Windows' own password edit controls take no input method text (winit ignores the purpose on Windows).
- The candidate list is placed with `CFS_EXCLUDE`, so it opens below the caret and never covers it.
- `WM_IME_SETCONTEXT` clears `ISC_SHOWUICOMPOSITIONWINDOW`: the text input draws the composition inline, as upstream's renderers do with the preedit text.
- `WM_IME_COMPOSITION` doesn't go to `DefWindowProc`, which would send the result again as `WM_IME_CHAR`.
- A composition that the focus leaves behaves as TSF handles it, since the host may move to TSF and applications shouldn't see a difference then.
  Windows has no single convention: a classic edit control doesn't complete the composition when it loses the focus (`EIMES_COMPLETECOMPSTRKILLFOCUS` is off by default), while TSF commits it when the focus moves and keeps it when the window is deactivated (Mozilla bug 1053053), and browsers mostly commit it.
  Upstream's text input drops it in both cases; the one thing every source agrees on is that it must never move into the next focused input.
  - **Focus moves** (a click, Tab, `focus()`, a popup): the text input inserts its preedit text when it loses the focus, and `Disable` cancels the input method's composition (`CPS_CANCEL`).
    By the time `Disable` arrives the focus has moved, so a commit from the input method would put the text into the next focused input.
  - **The window is deactivated**: the text input keeps its preedit text, and the host ignores the `Disable` that comes while it dispatches the deactivation, so the input method keeps the composition.
    Activating the window again gives the focus back to the same text input, and the user continues the composition.
    An input method that ends the composition on deactivation by itself sends the result or the end message, which reach the same text input, as the focus item stays.
    The Microsoft Japanese IME does that through IMM32: it commits the composition when the window is deactivated (checked by hand), as WordPad does. Through TSF it would keep it; nothing in Slint changes then.

## Clipboard

- Windows programs exchange CRLF line breaks: text is written with them and read back with `\n`.
- `OpenClipboard` fails while another program has the clipboard open, so it's tried again a few times, 5 ms apart.
- Only the default clipboard exists on Windows: the selection clipboard is X11's.

## Testing

- `crates/testing`'s injection functions dispatch through `Window::dispatch_event`, as upstream's do, so upstream's `tests/cases/input` and `focus` run unchanged. The Win32 translation is tested separately in `tests/input`, with messages sent to a hidden HWND.
- The platform belongs to the first thread that uses it, so each test file runs all its scenarios in one test.
- Keyboard layouts: the host asks for the layout of the thread, so the tests activate a layout with `ActivateKeyboardLayout` and no `KLF_SETFORPROCESS`. That changes the test thread only, never the user's input language. Scenarios skip when their layout (US, US-International, German) is missing.
- `ToUnicodeEx` and `GetKeyState` read the thread's keyboard state, which `SendMessageW` doesn't change: the tests write it with `SetKeyboardState` and restore it. The window is never in the foreground, so the real keyboard doesn't change it back.
- Scenarios that depend on `TranslateMessage` (dead keys, AltGr, text sent once) post the key messages one at a time and pump the queue as the event loop does.
- `DefWindowProc` turns Alt+F4 into `SC_CLOSE` only for the active window of the thread, so the scenario makes the hidden window active with `SetActiveWindow`, which leaves the foreground window alone.
- The clipboard test replaces the user's clipboard and puts the text back; a clipboard that held something other than text is emptied.
