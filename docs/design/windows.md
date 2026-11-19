# Windows, Popups, Menus and Accessibility (Phase 6)

The core already models popups, menus, tooltips and accessibility.
The host connects them to Win32 and UI Automation.
On Windows, upstream's default backend is winit.
It shows menus natively through `muda`, draws every other popup inside its window, and exposes accessibility through AccessKit.
Qt gives each popup a native window instead.

## Popups

`PopupWindow`, Slint-drawn menus and tooltips each get an HWND of their own, as in upstream's Qt backend.
A popup then isn't cut off at the edge of its window, which a combo box near the bottom of a window needs.

- The generated code asks `WindowInner::create_child_window_adapter` for the popup's adapter.
  The host returns a `Win32WindowAdapter` with a parent.
  The core then treats the popup as `PopupWindowLocation::TopLevel`.
- The HWND is `WS_POPUP`, owned by the parent's HWND, so it stays above its owner and hides with it.
  Its extended style is `WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_NOREDIRECTIONBITMAP`: no activation, no taskbar button.
  A tooltip answers `WM_NCHITTEST` with `HTTRANSPARENT`, so the mouse goes through it to its owner.
- It has a composition target of its own.
  Where the popup draws nothing, it's transparent, so rounded corners and shadows that the style draws show as drawn.
- The window is larger than the popup by a margin: room for what its items draw past its edges, such as the drop shadow of Fluent's menus.
  Drawn inside its parent, upstream shows that too.
  The host takes the margin from the items' bounding rectangles, again when they change.
  The margin answers `WM_NCHITTEST` with `HTTRANSPARENT`, so a click there is outside the popup.
- The core gives the position in the parent's logical client coordinates.
  The host maps it to the screen with the parent's scale factor, then moves the popup into the work area of the parent's monitor.
  Windows menus do the same.
- The popup takes its parent's scale factor when created.
  After that, it follows `WM_DPICHANGED` like any window.
- Activation: `WM_MOUSEACTIVATE` returns `MA_NOACTIVATE`, and the popup is shown with `SW_SHOWNOACTIVATE`.
  The owner stays active and keeps the keyboard focus.
  The core moves the focus item to the popup's window, so the host hands key messages to the topmost popup that isn't a tooltip.
- Dismissal:
  - The core closes popups on clicks inside the application's windows, and on Escape.
  - The host also closes them when the application deactivates (`WM_ACTIVATEAPP`), on a press in the owner's non-client area, and when the owner moves or resizes.
    A popup with `close-policy: no-auto-close` stays, and moves with its owner.
- `Window::take_snapshot` draws the popup windows, margins included, over the window's own pixels where they are on the screen.
  Upstream's renderers draw popups inside the window, so its snapshots show them too.
- Tests: a detached window creates popup windows only after `use_native_popup(true)`, as upstream's testing backend does.
  Those popup windows are detached too.
  Otherwise popups are drawn inside the window, which upstream's cases expect by default.

## Menus

As upstream's winit backend does on Windows with `muda`:

- A `MenuBar` is the window's Win32 menu bar, built from the `MenuVTable` entries.
  - A property tracker rebuilds the whole menu when what it shows changes.
    While a menu is open, the rebuild waits for `WM_EXITMENULOOP`.
  - The window is created with its menu, and the frame keeps room for it, so the client area keeps its size.
  - `WM_COMMAND` activates the entry.
    The core matches the shortcuts itself, as upstream does, so the menu only shows them after a tab.
- A `ContextMenu` opens a Win32 popup menu with `TrackPopupMenuEx`.
  The core asks for it while it handles the right click.
  The menu runs a message loop of its own, so the host opens it from a posted message, once the click is handled.
- Entries map to Win32 menu items as in `muda`: a check mark for a checkable entry, otherwise its icon as a 16×16 bitmap, and `MF_GRAYED` when disabled.
- GDI draws the menu bar into the window's redirection surface, which `WS_EX_NOREDIRECTIONBITMAP` leaves out.
  So only a top-level window that has a `MenuBar` when it's created keeps the surface, unless `SLINT_NO_MUDA` is set, and its composition target is topmost, over that surface.
  - The fork never paints the surface, so it shows white wherever the visuals are transparent: such a window can't show through or show Mica in its client area.
    Upstream's winit backend has the same limit: it turns transparency off for a window with a native menu bar (#8793).
  - Every other window has no surface, so what the visuals leave transparent shows what's behind it, or the system backdrop (Mica) when the application sets one.
  - The extended style can't change once the window exists. A window created without a surface reports `supports_native_menu_bar()` false from then on, so a `MenuBar` that comes later is upstream's Slint-drawn one.
  - Keeping the surface and calling `DwmEnableBlurBehindWindow` with an empty region, as winit does, didn't make it transparent, nor did a black class brush on top of that (2026-11-19). Why wasn't found.
- Windows draws a menu bar light whatever the color scheme.
  When the scheme is dark, the host draws the bar on the undocumented `WM_UAHDRAWMENU` and `WM_UAHDRAWMENUITEM` messages, in `muda`'s colors.
- Below the bar, the default procedure draws a one pixel line, in `COLOR_3DFACE` (measured on build 26200).
  Once the window's DPI changes, it stops drawing it, and the line shows the redirection surface's black; upstream's winit shows the same (2026-11-19, the owner, 125% and 200% monitors).
  The host draws the line after the default procedure on `WM_NCPAINT` and `WM_NCACTIVATE`, in `COLOR_3DFACE`, or in the dark bar's color.
- Windows also draws popup menus light (context menus, the bar's drop-downs, the tray's menu), unless the process opts in.
  When the scheme is dark, the host calls uxtheme's undocumented `SetPreferredAppMode` (ordinal 135) with `ForceDark`, then `FlushMenuThemes` (ordinal 136); when it's light, `Default` (`host/theme.rs`).
  The mode is process-wide and follows the host's color scheme, so a high contrast theme gets the default mode.
  Upstream's winit and `muda` don't call it, so upstream's popup menus stay light in dark mode.
  Checked on build 26200 by opening a popup menu in a process forced to dark: Windows 11's dark menu (`#2C2C2C`), with the check mark and the disabled entry drawn for it.
  With the system in dark mode, the owner saw `examples/backdrop`'s context menu dark (2026-11-19).
- `SLINT_NO_MUDA` turns both off, as upstream does, and the core draws the menus in popups instead.
- A detached window has no native menus, as upstream's testing backend.

## Accessibility

AccessKit, as upstream's winit backend, with `accesskit_windows` instead of `accesskit_winit`.

- Upstream's `internal/backends/winit/accesskit.rs` builds the AccessKit tree from the item tree.
  The host ports it (`host/accesskit.rs`), replacing winit's events with messages posted to the window.
- The window creates its `accesskit_windows::Adapter` with the HWND, before it handles any `WM_GETOBJECT`.
  The window procedure hands `WM_GETOBJECT` to it.
  It follows the keyboard focus on `WM_SETFOCUS` and `WM_KILLFOCUS`, and while a menu is open or the window moves, as `accesskit_windows`'s subclassing adapter does.
- That adapter is kept apart from the state that builds the tree.
  Raising the events of an update can send `WM_GETOBJECT` while the state is borrowed.
- UI Automation first gets a placeholder.
  The window builds the tree once it handles a posted message, as upstream builds it on a winit event.
- UI Automation asks for actions on threads of its own.
  The requests go through a channel, and a posted message has the window run them.
- Each popup window exposes its own tree.
  The tree's bounds take the popup's margin into account.
- The facade's `accessibility` feature, on by default as upstream, turns this on.
- A popup that gets the keys of its owner, such as a combo box's list, has the focus for UI Automation, and its owner doesn't.
  Windows never gives the popup the keyboard focus, so the host tells each window's AccessKit adapter whether its window is focused: the top-level window has the keyboard focus, no menu is open, it doesn't move or resize, and the keys go to that window.
  The host works this out again for the top-level window and all its popups when the top-level window gains or loses the focus, when a menu loop or a move starts or ends, and once the core is done showing or closing a popup.
  Before telling a window that it's focused, the host names its focus item in its tree, so that the focus event lands on the item.
  - A UI Automation client hears the focus move into the popup and back (`tests/input`), even when no client asked for the popup's tree before it opened. AccessKit raises the event when it replaces the placeholder with the first tree. What asks for that tree isn't known; presumably UI Automation, once the popup is shown.
  - Narrator reads it (checked by hand on 2026-11-19 with the gallery's combo box): "region" when the list opens, and the combo box again when it closes.
    It reads nothing on the arrow keys in the list: Fluent's combo box keeps the focus on a `FocusScope` without a role, so it shows on the list's root, and the keys only change `current-index`. Not yet compared with upstream.
  - Not checked: what `IUIAutomation::GetFocusedElement` returns while the popup is open. The window in the test isn't in the foreground, so the call returns the focus of another application.

## Window Properties

The host applies the `Window` item's properties as upstream's winit backend does on Windows (`host/frame.rs`).

- `no-frame`: `WS_POPUP`, with `WS_THICKFRAME` kept for the shadow and for resizing.
  `WM_NCCALCSIZE` makes the whole window client area.
  `WM_NCHITTEST` makes a border of `resize-border-width` resize it.
- A window whose minimum and maximum sizes are equal loses `WS_THICKFRAME` and `WS_MAXIMIZEBOX`, as winit's `set_resizable(false)`.
- `always-on-top`: `HWND_TOPMOST`.
- `full-screen`: `WS_POPUP` over the monitor.
  The window's style and placement come back when full screen ends.
- `maximized` and `minimized` show the window that way.
  A hidden window takes them when it's shown.
  When the user changes them, `WM_SIZE` sets the properties, as winit's `window_state_event`.
  A minimized window stays maximized, to come back maximized.
- `icon`: big and small icons at the system metrics' sizes.
- A frame change keeps the client size, so the `WM_SIZE` it sends doesn't count as the user's.
- `Dialog` needs nothing more: the core orders its buttons for Windows.

## Window Handle

- With the facade's `raw-window-handle-06` feature, as upstream, `Window::window_handle()` gives the HWND and the module's `hinstance` (`Win32WindowHandle`) and a `WindowsDisplayHandle`.
- The HWND exists once `show()` has created it; before that, the handle reports `HandleError::Unavailable`, as winit's does before its window exists.
- The handle asks the window each time, so one taken before `show()` works after it.
- It keeps the window alive, as upstream's keeps the winit window, so the HWND stays valid while someone holds it.
- Applications set what upstream has no API for through it, such as the Mica backdrop (`tests/probe/src/mica.rs`, `examples/backdrop`).
- `examples/backdrop` builds unchanged on upstream (`upstream-backdrop` in `tools/upstream-gallery`).
  There, the handle reports `NotSupported` until the winit window exists, after an iteration of the event loop, so the example sets the backdrop from a timer that retries.
  Captured with `tools/backdrop-capture.ps1` (2026-11-19, build 26200): Mica showed the same on both, `#EFF4F9` at the center of an active window; on the fork, Mica Alt `#CDDDEC`, Acrylic the backdrop window blurred through (`#FF73FF` over magenta), and none the style's opaque background.

## System Settings

- Color scheme and accent color, as upstream's winit backend reports them on Windows.
  - The color scheme is dark when the app mode is dark and no high contrast theme is on.
  - The accent color is the DWM colorization color.
  - The host reports both to the context when it creates a window, and again on `WM_SETTINGCHANGE` and `WM_DWMCOLORIZATIONCOLORCHANGED`.
  - The title bar follows the color scheme (`DWMWA_USE_IMMERSIVE_DARK_MODE`).
- `SLINT_SCALE_FACTOR` sets every window's scale factor instead of its monitor's, as in upstream's winit backend.
  Logical sizes convert at that factor, and `WM_DPICHANGED` no longer changes it.
  The window frame keeps the monitor's DPI.

## System Tray

Upstream's core has a `Shell_NotifyIconW` backend (`items/system_tray/windows.rs`), which the fork keeps unchanged.
Its message-only window gets its messages from the host's message loop.
`probe-tray` runs upstream's `examples/system-tray`.

## Drag and Drop

As upstream's winit backend, the host doesn't start native drags: `start_drag` returns `false`, and the core drags inside the window.
Drag and drop between applications through OLE comes after Phase 7.
