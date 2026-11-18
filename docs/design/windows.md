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
- Windows draws a menu bar light whatever the color scheme.
  When the scheme is dark, the host draws the bar on the undocumented `WM_UAHDRAWMENU` and `WM_UAHDRAWMENUITEM` messages, in `muda`'s colors.
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
- Not yet done: a popup with the keys of its owner, such as a combo box's list, doesn't tell UI Automation that it has the focus.

## System Settings

- Color scheme and accent color, as upstream's winit backend reports them on Windows.
  - The color scheme is dark when the app mode is dark and no high contrast theme is on.
  - The accent color is the DWM colorization color.
  - The host reports both to the context when it creates a window, and again on `WM_SETTINGCHANGE` and `WM_DWMCOLORIZATIONCOLORCHANGED`.
  - The title bar follows the color scheme (`DWMWA_USE_IMMERSIVE_DARK_MODE`).
- `SLINT_SCALE_FACTOR` sets every window's scale factor instead of its monitor's, as in upstream's winit backend.
  Logical sizes convert at that factor, and `WM_DPICHANGED` no longer changes it.
  The window frame keeps the monitor's DPI.

## Not Yet Decided

- System tray: upstream's core has a `Shell_NotifyIconW` backend (`items/system_tray/windows.rs`), kept in the fork.
- Drag and drop between applications (OLE).
