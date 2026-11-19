# Windows, Popups, Menus and Accessibility (Phase 6)

On Windows, upstream's default backend is winit.
It shows menus natively through `muda`, draws every other popup inside its window, and exposes accessibility through AccessKit.
Qt gives each popup a native window instead.
The host follows winit where an application would see a difference, and Qt where a native window serves better.

## Popups

`PopupWindow`, Slint-drawn menus and tooltips each get an HWND of their own, as on Qt.
A popup drawn inside its window is cut off at the window's edge, which a combo box near the bottom of a window can't afford.

- The popup's window is larger than the popup by a margin, for what its items draw past their edges, such as the drop shadow of Fluent's menus: drawn inside its parent, upstream shows that too.
  The margin is transparent to the mouse, so a click there counts as outside the popup.
- Activation: the popup never activates, so the owner keeps the keyboard focus.
  The core moves the focus item into the popup anyway, so the host forwards key messages to the topmost popup that isn't a tooltip.
- Dismissal: clicks outside the application never reach the core, which closes popups only on clicks inside the application's windows and on Escape.
  So the host also closes them when the application deactivates (`WM_ACTIVATEAPP`), on a press in the owner's non-client area, and when the owner moves or resizes.
- `Window::take_snapshot` draws the popup windows over the window's pixels, because upstream's renderers draw popups inside the window and its snapshots show them.
- A detached window (tests) creates popup windows only after `use_native_popup(true)`, as upstream's testing backend does, because upstream's cases expect popups inside the window.

## Menus

- `MenuBar` and `ContextMenu` are Win32 menus, as upstream's winit backend shows them through `muda`: native menus keep F10, Alt+letter and the menus' UI Automation roles.
  - While a menu is open, a rebuild of the bar waits for `WM_EXITMENULOOP`.
  - The core matches the shortcuts itself, as upstream does, so the menus show them as text only.
  - `TrackPopupMenuEx` runs a message loop of its own, and the core asks for the context menu while it handles the right click, so the host opens the menu from a posted message once the click is handled.
- GDI draws the menu bar into the window's redirection surface, which `WS_EX_NOREDIRECTIONBITMAP` leaves out.
  So only a top-level window that has a `MenuBar` when it's created keeps the surface, with its composition target over it.
  - The fork never paints the surface, so it shows white wherever the visuals are transparent: such a window can't show through or show Mica in its client area.
    Upstream's winit backend has the same limit: it turns transparency off for a window with a native menu bar (#8793).
  - Every other window has no surface, so it shows through, or shows Mica when the application sets it. Until 2026-11-19 every top-level window had one, and no window showed through, not even a `no-frame` window's rounded corners.
  - The extended style can't change once the window exists: a `MenuBar` that comes later is upstream's Slint-drawn one (`supports_native_menu_bar()` is false from then on).
  - Why not keep the surface and make it transparent: `DwmEnableBlurBehindWindow` with an empty region, as winit does, didn't, nor did a black class brush on top of that (2026-11-19). Why it didn't wasn't found.
- Windows draws a menu bar light whatever the color scheme, so when it's dark, the host draws the bar on the undocumented `WM_UAHDRAWMENU` and `WM_UAHDRAWMENUITEM`, in `muda`'s colors.
- Below the bar, the default procedure draws a one pixel line, in `COLOR_3DFACE`, until the window's DPI changes; from then on it doesn't, and the line shows the surface's black.
  Upstream's winit shows the same (2026-11-19, the owner, 125% and 200% monitors). The host draws the line itself after the default procedure on `WM_NCPAINT` and `WM_NCACTIVATE`.
- Windows also draws popup menus light (context menus, the bar's drop-downs, the tray's menu) unless the process opts in with uxtheme's undocumented `SetPreferredAppMode` and `FlushMenuThemes`.
  The mode is process-wide, so it follows the host's color scheme, and a high contrast theme gets the default mode.
  Upstream's winit and `muda` don't call it, so upstream's popup menus stay light in dark mode.
  Checked on build 26200 (Windows 11's dark menu, `#2C2C2C`), and by the owner with the system in dark mode (2026-11-19).
- `SLINT_NO_MUDA` turns native menus off, as upstream: the core draws them in popups. Mica and a menu bar in one window need it for now.

## Accessibility

The host ports upstream's `internal/backends/winit/accesskit.rs`, so the tree is upstream's, with `accesskit_windows` in place of `accesskit_winit`.

- The window creates its AccessKit adapter right after `CreateWindowExW`, before it handles any `WM_GETOBJECT`.
- The adapter is kept apart from the state that builds the tree, because raising the events of an update can send `WM_GETOBJECT` while that state is borrowed.
- UI Automation first gets a placeholder; the window builds the tree once it handles a posted message, as upstream builds it on a winit event.
- UI Automation asks for actions on threads of its own, so the requests go through a channel to the window's thread.
- Focus in popups: Windows never gives a popup the keyboard focus, so a popup that gets its owner's keys (a combo box's list) never got `WM_SETFOCUS`, and UI Automation saw no focus in it.
  The host tells each window's adapter that it's focused when the top-level window has the keyboard focus, no menu is open, it doesn't move or resize, and the keys go to that window.
  - When: the host works this out again for the top-level window and all its popups when the top-level window gains or loses the focus, when a menu loop or a move starts or ends, and once the core is done showing or closing a popup: it shows a popup before it adds it to the active popups, and moves the focus item after it hides one.
  - Before telling a window that it's focused, the host names the focus item in its tree, or the focus event lands on the window's root.
  - A UI Automation client hears the focus move into the popup and back (`tests/input`), even when no client asked for the popup's tree before it opened: AccessKit raises the event when it replaces the placeholder with the first tree. What asks for that tree isn't known; presumably UI Automation, once the popup is shown.
- Focus on the selected item: when the focus falls on the root of a popup's tree, and nothing names a descendant to delegate it to, it goes to the first selected list item below the root.
  - Why: Fluent's combo box keeps the focus on a `FocusScope` without a role, so it shows on the list's root, and the arrow keys only change the selection. Upstream, where the list is in the window's tree, Narrator reads the selection. The fork's popup raises the same selection events (`tests/input`), but Narrator reads nothing, presumably because they come from a window without the keyboard focus. Focus events from the popup it does read.
  - Why not a list box: making the popup's root a list box, so that the items have a selection container as upstream's do, didn't make Narrator read them (2026-11-20, by hand).
  - With the focus on the selected item, Narrator reads the selected item when the list opens, each item on the arrow keys, and the combo box when the list closes (2026-11-20, the gallery's combo box). NVDA and JAWS aren't checked.
- `SLINT_FORK_TRACE_A11Y` prints, with the time, when UI Automation asks a window for its tree and when AccessKit raises events.
- Not checked: what `IUIAutomation::GetFocusedElement` returns while a popup is open; the test's window isn't in the foreground, so the call returns another application's focus.

## Window Properties

The host applies the `Window` item's properties as upstream's winit backend does on Windows (`host/frame.rs`).

- `no-frame` keeps `WS_THICKFRAME`, as upstream does, for the shadow, resizing and the rounded corners that DWM draws; the owner saw the same border and corners on both (2026-11-19).
  The fork's window is `WS_POPUP` where upstream's is `WS_CAPTION`; whether that changes Win+arrow snapping or the minimize animation isn't checked.
- A minimized window keeps `maximized`, so that it comes back maximized.
- A frame change keeps the client size, and the `WM_SIZE` it sends doesn't count as the user's resize.
- A DPI change (`WM_GETDPISCALEDSIZE`, then `WM_DPICHANGED`) keeps the logical client size, whether the window moves to another monitor or the monitor's scale changes.
  The owner checked it on real monitors at 100%, 125%, 175% and 200% (2026-11-19, `SLINT_FORK_TRACE_DPI`): one message of each kind per change, and no extra `WM_SIZE`.
  The input method's windows and the popups came at the new monitor's scale, on the parent's monitor.
  Upstream switched the scale while the snap area at the monitors' edge was showing, and flickered; the fork didn't.
- Live resize: the content lags the frame by one or two frames (spike S1). The owner found nothing wrong in `probe-titlebar`, with and without a redirection surface (2026-11-19), so the host doesn't sync inside `WM_SIZE`.

## Title Bar

`host/title_bar.rs` extends a framed top-level window's content into its title bar, as WinUI 3's `ExtendsContentIntoTitleBar`: the client area covers the title bar, and DWM keeps drawing and hit-testing the caption buttons, so Snap Layouts stays on the maximize button.
The API is unstable (facade feature `unstable-wuc`).

- `no-frame`, full screen and popups don't extend, and neither does a window with a native `MenuBar`: GDI draws the bar in the non-client area, which the client area would cover.
  `supports_native_menu_bar()` is false once extension is asked for, but a component created before that already has the native bar.
- Moving the window is upstream's `WindowMoveArea`. While the window extends, a double click on it toggles maximized and a right click opens the system menu, from posted messages, as the context menus are.
  - Why not `HTCAPTION` from `WM_NCHITTEST` over the title bar: the host would have to guess which items take clicks, and a wrong guess makes a button unclickable.
  - The area counts the double click itself: it ignores the release and the moves, so the core no longer sees it under the mouse and counts every press as a first one.
- Maximized: the client area isn't inset at the top, since that stopped `DwmDefWindowProc` from hit-testing the buttons at all (spike). The client starts above the monitor by the frame and its padding (9 px at 120 dpi), and the content moves down by that margin, which every client-to-screen conversion (popups, IME, AccessKit, drops, menus, snapshots) goes through.
  The extended frame's top is then the title bar plus that margin (47 px at 120 dpi).
- Slint's visuals are drawn above DWM's buttons, so the root `Window` item's background leaves a hole where they are, and the caption color is the background's while it's an opaque solid color (DWM's default otherwise), which shows seamless. Other items over the buttons hide them; that's the application's to avoid, with `title_bar().buttons`.
- When: the host extends the frame again on activation (DWM forgot it in the spike), on `WM_SIZE`, DPI and setting changes, and reports the title bar from a posted message once the change is over; the callback runs only when the value changed.
- `DWMWA_CAPTION_BUTTON_BOUNDS` at 120 dpi: rows 0 to 37 of a restored window, against a 38 px title bar with its frame; a maximized window's start at its top, above the monitor, so 28 px show. `TitleBar::height` is the buttons' bottom in the content's coordinates.
- Checked 2026-11-20 at 120 dpi with `probe-caption`: button hit tests restored and maximized, the content filling the work area when maximized, the client size kept when switching before and after `show()`, Mica, a dark background, and a background changed while shown. Another DPI wasn't tried.

## Window Handle

`Window::window_handle()` gives the HWND (facade feature `raw-window-handle-06`, as upstream), so applications set what Slint has no API for, such as Mica, without a fork-only API.

- When: the HWND exists once `show()` has created it; before that the handle reports `Unavailable`, as winit's does before its window exists. The handle asks the window each time, so one taken before `show()` works after it.
- It keeps the window alive, as upstream's keeps the winit window, so the HWND stays valid while someone holds it.
- On upstream, the handle reports `NotSupported` until the winit window exists, after an iteration of the event loop, so `examples/backdrop`, which builds on both, sets the backdrop from a timer that retries.
  Captured with `tools/backdrop-capture.ps1` (2026-11-19, build 26200): Mica the same on both (`#EFF4F9` at the center of an active window).

## System Settings

- The color scheme is dark when the app mode is dark and no high contrast theme is on, and the accent color is the DWM colorization color, as upstream's winit reports them.
  The host reports them again on `WM_SETTINGCHANGE` and `WM_DWMCOLORIZATIONCOLORCHANGED`.
- `SLINT_SCALE_FACTOR` overrides every window's scale factor, as upstream's winit; the window frame keeps the monitor's DPI.

## System Tray

The fork keeps upstream's `Shell_NotifyIconW` backend (`items/system_tray/windows.rs`) unchanged; its message-only window gets its messages from the host's message loop.

## Drag and Drop

- The host doesn't start native drags, as upstream's winit backend: `start_drag` returns `false`, and the core drags inside the window.
- A `DropArea` takes drags from other programs, as on upstream's Qt backend; upstream's winit backend takes none.
  - OLE is initialized the first time a window registers its drop target, on the UI thread, and never uninitialized, as the Windows Runtime isn't. `RoInitialize(RO_INIT_SINGLETHREADED)` has made the thread single-threaded by then, which OLE accepts (`tests/input` checks that the target registers). On a thread the application made multithreaded, OLE fails, and the windows take no drops.
  - The target is revoked on `WM_DESTROY`, before the adapter, which may be going away already, would be looked up.
  - When: the data is read once, as the drag enters, and the same `DataTransfer` goes with every move and the drop.
  - Why copy without a modifier key, where a drag inside the window proposes a move: when the target answers `DROPEFFECT_MOVE`, Explorer deletes the files after the drop (an unoptimized move, in the Shell's documentation; not tried). Qt proposes copy for drags from other programs too (`QPlatformDrag::defaultAction`).
  - During the drag, the cursor is the source's (`IDropSource::GiveFeedback`), from the effect that the window answers.
  - An elevated application presumably takes no drops from Explorer, as Windows blocks drags from a lower integrity level (UIPI); not tried.
  - `tests/input/tests/drop.rs` calls the target as OLE does, because a real drag needs the mouse. The owner checked real drags from Explorer and an editor with `probe-drop` (2026-11-20).
