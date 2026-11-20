# Embedding Visuals of the Application

Roadmap item 4, a spike to falsify the one claim only a fork can make: that the UI being a Composition visual tree lets an application put its own GPU content, or another process's, inside the UI. Roadmap item 6 forwarded the input of such an item to a WebView2 ([Input](#input)).
Run on one machine on 2026-11-20 (125 %, and `SLINT_SCALE_FACTOR=2`) with `probe-embed` (`tests/probe/src/embed.rs`); none of it is public API.

## Result

It holds. An application's visual shown in an `Image` item takes part in the item tree like any image, and its content updates cost Slint nothing:

| Question | Answer |
|---|---|
| 1. Slint content over it, a clip with rounded corners, group opacity, `Flickable` scrolling | All apply: a translucent rectangle and a text above it, a 40 px radius clip at 60 % opacity, `image-fit: contain` in a taller item, and a scroll of 150 px, in `take_snapshot` at both scales |
| 2. Sync passes while its content changes every frame | None: 180 presents in 3 s from the application's thread, 0 sync passes. With a WebView2 animating as well, also 0 |
| 3. The application's own D3D device | Works: a swap chain of a separate D3D11 device (`CreateSwapChainForComposition`, wrapped by `ICompositorInterop`), presented from a thread of its own |
| 4. `take_snapshot`, DPI, device loss | `take_snapshot` includes the swap chain's frame and the WebView2's page. At scale 2 the 640×360 swap chain lands one to one. A change of scale lays the item out again (the image reads the scale factor), but the resolution of the content is the application's. Device loss wasn't tried; see below |
| 5. Without `windows` types in the API | Yes: the application gets the compositor and gives the visual as raw COM pointers, so its `windows` crate needn't match the fork's |

WebView2 in composition hosting mode (`SetRootVisualTarget` on a container of the window's compositor) shows its page in the item, clipped by the rounded rectangle and under Slint's translucent rectangle.
Scrolling and resizing over the playing swap chain looked smooth to the owner (2026-11-21, by hand).

So the fork's claim beyond the integrations is shown for display: zero-copy content, from any thread or process, composed under and over Slint's items by the compositor.
Upstream mixes content through an `Image` too, but each frame is a new image and a new Slint frame: `examples/ffmpeg` copies every video frame into a pixel buffer and sets it, `servo` imports a wgpu texture per frame. And on upstream, a WebView2 can only be a child window, which no Slint item can draw over.

## Why

- **An `Image`, not a new item:** upstream's code for foreign content is `Image` (`BackendStorage` and the texture imports), so placing, fitting and clipping follow image items, and the compiler and generated code stay upstream's. The source size given with the visual drives the layout and `image-fit` as a pixel size would.
- **The fork sizes and offsets the visual; the application owns everything else** (brush, scale, children): Composition visuals have no layout, and the item's place changes with the layout only on the UI thread. The visual is cut to the item by an inset clip on a container between the two.
- **WebView2 draws in pixels:** the root visual carries the scale factor, so the application scales its container by the inverse and sets the rasterization scale to Slint's. Without that the page is magnified and blurred.

## Why Not

- **A new `ImageInner` variant:** it would change a `#[repr(u8)]` enum that `tools/cpp-headers` exports, and isn't needed: `BackendStorage` with a vtable of its own is told apart by `VRc::downcast`.
- **Showing one visual in several items:** a visual has one parent, so the second item would take it from the first. The probe gives three sprites over one swap chain's surface brush instead.

## Not Settled

These would decide an API, not whether to go on:

- **Device loss:** the fork's devices are separate, and its device-lost path redraws only text and shadow nodes; the application's own loss is the application's to handle, and replacing the brush on its sprite needs no Slint call.
- **Scale changes while running:** the item follows, but nothing tells the application to reallocate its swap chain or to update the WebView2's rasterization scale. Upstream has no event for it either.
- **Tiling and a 9-slice of a visual** aren't supported.

## Input

Item 6, run on 2026-11-21 (125 %, and `SLINT_SCALE_FACTOR=2`) with `probe-embed --input-check` (messages posted to the probe's own window), and by the owner's hand for clicks, the cursor, typing with the Japanese IME, Tab, Alt+Tab and the popups.

The application can forward the input of the item to a WebView2 with upstream's API and the window's `HWND`; the fork needs no API for it:

| Input | How the application forwards it | Result |
|---|---|---|
| Buttons, moves, leave | A `TouchArea` over the image: `pointer-event` and `has-hover` to `SendMouseInput`, at the item's position times the scale | Clicks, double clicks, the context menu, and a selection dragged out of the item and released outside it |
| Slint over the page | A `TouchArea` of the item above | It takes the input; the page gets none there |
| Wheel | `scroll-event`, the host's 60 px per notch back to 120 | The page scrolls. The item takes every wheel event, so a `Flickable` around it doesn't scroll over it |
| Cursor | `CursorChanged` and `SystemCursorId` to the item's `mouse-cursor` | Arrow, hand, I-beam. A page's image cursor comes as 0, shown as the arrow |
| Keys and IME | Nothing: the WebView2's own child window (0×0, in the application's process) has the keyboard | Typing, and the IME's candidates at the caret. Slint doesn't see these keys |
| Focus | A `FocusScope` around the `TouchArea`: gained moves the WebView2's focus (`MoveFocus`, next or previous by the state of Shift), lost gives the window the keyboard back (`SetFocus`); `MoveFocusRequested` moves Slint's focus | Click and Tab in both directions between Slint's text fields and the page |
| Activation | A subclass of the window: `WM_SETFOCUS` while the item has Slint's focus moves the WebView2's focus | Typing goes on in the page after Alt+Tab |
| Popups | Bounds at the item's place in the window (`absolute-position`, `changed`); `NotifyParentWindowPositionChanged` on `WM_MOVE` in the subclass | A `select`'s list and the context menu open at the page, also scrolled and with the window moved |
| Closing popups | A click forwarded outside the bounds, from the subclass: a click or a wheel outside the item, on the frame, a deactivation, and the item moving, only while the item has Slint's focus | The popups close; the page's focus and selection stay |

So making the embedding an API is worth it, and the API stays as small as the spike's: the compositor and an image of a visual, as raw COM pointers. The forwarding (about 450 lines in the probe, with the WebView2's setup) is the application's, and the fork could ship it as an example.
What only the fork could add is listed under [Not Available to an Application](#not-available-to-an-application).

### Why

- **The bounds' offset is for the popups only:** it moves neither the content (it stays where the container is) nor the input (`SendMouseInput` takes points relative to the page whatever the offset). The popups and the IME's window are placed by it, so it must follow the item as it scrolls, and the window's position as it moves.
- **A click outside the bounds closes the popups:** nothing that happens outside the page reaches the WebView2, so its menus stayed open while the window moved or the content scrolled (owner, 2026-11-21). `SetFocus` on the window and a leave don't close them. The click also gives the WebView2 the keyboard, so the application sends it only while the page has the focus; before that, a click on a Slint text field left the keyboard in the page.
- **`WM_SETFOCUS`, not Slint's focus event of the activation:** `DefWindowProc` of `WM_ACTIVATE` gives the keyboard to the window after Slint gave the item its focus back, which took it from the WebView2 (owner, 2026-11-21).
- **No double click kinds:** the page counts the clicks itself; down, up, down, up made a `dblclick`.
- **The WebView2's popups don't block the mouse:** a layered, non-activating window of the browser process covers the bounds, but `WindowFromPoint` there finds Slint's window.

### Not Available to an Application

- **Accessibility** (L): the page isn't in the window's UI Automation tree, and the WebView2's child window shows an empty pane. The page's provider (`ICoreWebView2CompositionController2::AutomationProvider`) has to be a child of the item's node, and `accesskit_windows` has no place for a provider of another kind (`GetEmbeddedFragmentRoots` returns none, and navigation knows only AccessKit's nodes). It needs a patch of `accesskit_windows`, or a provider of the fork's own.
- **Dropping on the page** (M, not tried): the fork registers the window's `IDropTarget` (`host/drop.rs`); forwarding to `ICoreWebView2CompositionController3::DragEnter` and the rest needs a hook there.
- **Pen and touch** (not tried): Slint's pointer events carry no pointer data, so these arrive as the mouse, without pressure. `SendPointerInput` needs what `WM_POINTER` messages carry.

Not tried, and possible for an application: Slint's shortcuts while the page has the keyboard (`AcceleratorKeyPressed` and `Window::dispatch_event`), and a change of scale (see [Not Settled](#not-settled)).
