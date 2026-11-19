# Visual Tree Sync Engine

Based on spikes [S1](spike-s1-hosting.md), [S2](spike-s2-primitives.md), and [S3](spike-s3-verification.md), and settled in Phase 2.

## Goal

Keep one `Windows.UI.Composition` visual per item in sync with the item tree, touching only the visuals whose properties changed.
Rendering itself is the compositor's job; the runtime never draws a frame.

## Change Detection

Upstream's `PartialRenderer::compute_dirty_regions` walks the **whole** item tree on every redraw, comparing each item's geometry with a cached one and asking a per-item `PropertyTracker` whether rendering properties changed.
Clean trackers register as dependencies of the window's redraw tracker, so any later change requests a redraw; structural changes (repeaters, conditionals, popups) are caught because the walk reads the repeater models.

The sync pass (`WindowInner::sync_visuals`, in place of `draw_contents`) keeps that scheme, and updates visuals where upstream would repaint.

- The root visual's scale is applied inside the window tracker, so a scale factor change requests a pass.
- An item's content is updated again when its size changed, even with a clean tracker: the size is an argument of `update_visual`, so its tracker can't see it change.
- Nodes of live items that a pass doesn't visit are detached but kept: a conditional or a replaced component may come back.
- The dispatcher queue commits implicitly when the handler returns (S1), so a pass needs no explicit commit.
- Why not a dirty queue: the pass walks all items whenever anything changed, like upstream. Switch to per-node dirty handlers only if profiling shows the walk above about 1 ms for realistic trees; Phase 7's measurements didn't.
- Item structs never hold COM objects: the node handle lives in `CachedRenderingData` (index and generation), so the generated code's layout and `FieldOffsets` stay unchanged (`compat-contract.md`).

## Items

- `Transform` builds the matrix itself: Composition's `Scale` and `RotationAngle` apply scale before rotation, and Slint rotates first.
- `Layer`'s `cache-rendering-hint` has no equivalent; Composition retains content anyway.
- `Flickable` scrolls through the `Empty` viewport the compiler puts between it and its content, so scrolling changes only that visual's `Offset`.
- `Opacity` is a container's `Opacity`, which is group opacity (S2).

### Text

Text items keep upstream's layout: `textlayout/sharedparley` shapes and positions the glyphs, as for upstream's renderers ([S4](spike-s4-text.md)). Only the drawing is new.

- `update_visual` records parley's drawing as a display list and replays it into a drawing surface with Direct2D, so nothing touches Direct2D while parley walks the layout.
- The surface covers what's drawn, not the item: an elided text keeps its last line even when it overflows the item, and alignment can put lines above it.
- Color glyphs: COLRv1 paint trees (Segoe UI Emoji on Windows 11) need `IDWriteFactory8` and `ID2D1DeviceContext7::DrawPaintGlyphRun`. As in upstream's renderers, a translucent text color is the opacity of a color glyph as a whole, and a bitmap glyph stays upright in faux italic.
- The content tracker sees what parley reads, including the scale factor. The cursor's blinking is a property too: it redraws the text input, which costs about 25 µs (S4).
- Devices: one D3D11, D2D and `CompositionGraphicsDevice` per UI thread. When the device is lost or replaced, every text and shadow node redraws.
  They are never released: the main thread's thread-locals are destroyed while the process exits, after the other threads have ended, and releasing the devices then can block forever.
- `TextInput` updates the input method from `update_visual`, as upstream's `render` does, except for a read-only input, which disabled the input method when it got the focus.
- Upstream's text drawing clips to rectangles only, so the recorder ignores the radius of a clip.
- A text larger than 16384 pixels is cut off (virtual surfaces would lift that).

### Antialiasing

Upstream's Skia renderer antialiases rounded rectangles, clips with a radius, and everything under a rotation, and draws axis-aligned rectangles without.
The compositor antialiases only in `CompositionBorderMode::Soft`; the default `Hard` snaps to pixels.

- A rotating `Transform`, a clip with a radius, a rounded rectangle's shapes, and a `Path` with `anti-alias` get `Soft`; everything else inherits.
- Soft mode also antialiases the edges of the visual's own bounds. Where a shape's edge lies on them, both apply and the edge comes out lighter than upstream's.
  So a node's shapes are in a floor `ShapeVisual` that reaches 1 logical pixel past the item on each side. Being below the children, its border mode doesn't pass on to them.
- An arc of radius 0 ends a Direct2D figure where Composition reads it, so a square corner of a per-corner rounded rectangle has no arc.
- Gradients interpolate straight colors in the compositor and premultiplied colors in upstream's renderers. Between stops of different alpha, seven intermediate stops follow the premultiplied blend, and a transparent stop takes its neighbor's color on each side.

### Conic Gradients

Neither Composition nor Direct2D has a conic gradient brush, so it's computed on the CPU, interpolated premultiplied as Skia's sweep gradient.

- Where the item lies at a fractional pixel, the compositor samples the bitmap between its pixels, which blurs abrupt changes such as the seam at 12 o'clock.
  So the bitmap has up to four pixels per device pixel, within 2^20 pixels, and is drawn at the window's scale factor. Only a conic gradient reads the scale factor, so that other brushes don't redraw when it changes.
- A sprite shape rejects a surface brush ("Unsupported source brush type"), so a rounded rectangle or a path with a conic fill or stroke shows each part as a sprite clipped to the part's outline.
  The compositor antialiases these clips about a tenth lighter than Skia antialiases a path.
- The gradient covers the item only: a stroke that reaches outside the item has no gradient there.

### Images

`ImageItem` and `ClippedImage` place the image as upstream's Skia renderer does (`draw_image_impl`), with the effects of [S5](spike-s5-effects.md).

- Upstream draws images from a whole-pixel origin, so the floor sprite snaps to pixels. A single part's fractional offset goes into the brush transform instead, so that only the item's origin snaps; `SnapToPixels` on the brush would also round the offsets inside a scaled part. Tiles start at the part's offset cut to whole pixels, as upstream's `as i32`.
- An SVG under magnifying transforms is rasterized larger by that magnification, rounded up to a power of two, up to 2048 pixels (`transformed_scalable_render_size`, #10903). The image reads the ancestors' transforms through its tracker, so a change of one updates it.
- Surfaces are shared per pixel data, raster size and part, in a 64 MiB cache. Items keep the surfaces they show either way, since composition surfaces have no weak references.
- A tiled part needs a surface of its own, since the `Border` effect wraps the whole surface.
- `Border` takes a source parameter only, so tiles are colorized after tiling: the same for a color, but a gradient spans the first tile instead of repeating.
- The compositor rounds a negative half-pixel origin up, where Skia rounds it away from zero: an image at x = -5.5 shows one pixel right of upstream's.
- `BackdropBlur`: a backdrop brush sees only what's behind its sprite, so the sprite reaches 3σ beyond the item and a clip cuts it back, as upstream's blur picks up content that far. The blur amount is in logical pixels, since the root visual carries the scale factor.

### Paths

`Path` shows what upstream's Skia renderer draws in `draw_path`.

- A stroke width of 0 is a hairline of one physical pixel, as in Skia. `miter` joins are `MiterOrBevel`, which bevels beyond the miter limit as Skia does. A dash pattern that Skia ignores (an odd count, a negative length, a sum of 0) is solid here too.
- Composition cuts a shape off at its visual's bounds, so the floor covers what the path draws outside the item: the geometry's bounds grown by the stroke's reach (the miter limit in half widths, or a square cap's corner). With `clip: true` it covers the item only.
  The compiler lowers `clip` on a `Rectangle` to a `Clip` element, but keeps it as a property of `Path`.
- Edge coverage differs from Skia's by a few levels, and rounded joins and caps by a pixel at their curves.

### Shadows

`BoxShadow` draws the texture of upstream's `render_drop_shadow_image` and `render_inset_shadow_image` with Direct2D's `Shadow` effect ([S5](spike-s5-effects.md)).

- Each node holds its own texture; upstream shares them in a cache across items.
- The `Shadow` effect's Gaussian differs from Skia's blur mask by a few levels near the shape's edge.
- A shadow wider or higher than 16384 pixels isn't drawn.

## Snapshots

`Window::take_snapshot()` moves the root visual into a container of the window's size and captures it ([S3](spike-s3-verification.md)); a hosted window's target shows the container meanwhile, so the window doesn't flash.
The pixels come back straight-alpha, as upstream's renderers return them. Capturing needs a desktop session with DWM.
The capture device is never released, like the drawing devices.

## Children Patching

- Never `InsertAtTop`: it's linear per call (S2). New visuals go in with `InsertAbove` the previous sibling.
- Reordering (`z` changes) removes and reinserts the moved visuals.

## Releasing Nodes

- Nodes go newest first: Composition releases objects oldest first in quadratic time.
  Removing 2000 text rows took 6.3 s oldest first and takes 37 ms newest first; 8000 rows take 149 ms.
  Releasing an old object while many newer ones live is slow as well, so anything that holds released visuals longer has to keep that order.
- Known limitation: when hundreds of text surfaces are drawn and dropped with no commit in between, the compositor's graphics device keeps the atlas pages they took.
  That takes two sync passes in one dispatcher callback, which only `sync_now` (tests, snapshots) does.
  Each page is a texture with two render target views and two shader resource views, a `DxgkCompositionObject` and a `Section` handle, and a few MB of GPU memory; 1000 rows keep about 1.5 pages per cycle.
  `Close()`, reusing the surfaces, `ID2D1Device::ClearResources` and `Trim` don't free them; showing a popup or hiding another window sometimes does.
  Holding the released nodes until a commit has completed avoids it.
- Rows created and removed in separate frames level off: 400 cycles of 1000 text rows left 4 objects. `probe-perf` measures both.

## Frames and Animations

- A property change requests one sync pass, coalesced until it runs.
- A pass that finds active animations asks the frame thread (S1) for ticks on the compositor clock; each tick moves the animation tick to now, and the changes request the next pass.
- While frames tick, only the frame tick moves the animation tick. The message loop still fires timers, but moving the animation tick on every message would request a pass per message and spin the loop.
- With no animation nothing runs, and the message loop keeps the animation tick current, so that a new animation starts from now.
- The root visual carries the scale factor; the compositor snaps axis-aligned edges (S2).

## Testing

- `tests/screenshots` renders upstream's screenshot cases with `take_snapshot`. A case gets a reference of this fork when it's close to upstream's (Skia's where there is one), or when a review found the same layout and only the difference `exclusions.rs` gives (glyph rasterization, mostly).
  `SLINT_SCREENSHOT_REPORT=1` writes each case's difference to upstream's reference, with images side by side.

## Open Questions

- Whether long lists that aren't virtualized and change their texts every frame keep atlas pages too (deferred from Phase 7, see [Releasing Nodes](#releasing-nodes)).
- `WindowInner::set_component` on a shown window keeps the old component alive until the window is hidden, as upstream does. Generated code only sets the component before showing.
