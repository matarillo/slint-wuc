# Embedding Visuals of the Application

Roadmap item 4, a spike to falsify the one claim only a fork can make: that the UI being a Composition visual tree lets an application put its own GPU content, or another process's, inside the UI.
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

WebView2 in composition hosting mode (`SetRootVisualTarget` on a container of the window's compositor) shows its page in the item, clipped by the rounded rectangle and under Slint's translucent rectangle. Display only: no input reaches it.

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
- **Input to a WebView2** (L): pointer, keyboard and focus forwarding (`SendMouseInput`, `MoveFocus`), the bounds as the item scrolls, the cursor, and the page's accessibility tree under Slint's.
- **Live smoothness** on screen, by hand: scrolling and resizing over a playing swap chain (`probe-embed --web`). The probe can't capture the screen while the session is locked.
- **Tiling and a 9-slice of a visual** aren't supported.
