# Visual Tree Sync Engine

Based on spikes [S1](spike-s1-hosting.md), [S2](spike-s2-primitives.md), and [S3](spike-s3-verification.md), and settled in Phase 2.
The code is `crates/core/visual_tree.rs` and `crates/core/host.rs`.

## Goal

Keep one `Windows.UI.Composition` visual per item in sync with the item tree, touching only the visuals whose properties changed.
Rendering itself is the compositor's job; the runtime never draws a frame.

## How Upstream Detects Changes

Upstream's `PartialRenderer::compute_dirty_regions` walks the **whole** item tree on every redraw.
Each item has a `PropertyTracker` in a cache entry that `CachedRenderingData` points to.
The walk compares each item's geometry with the cached one and asks its tracker whether rendering properties changed.
Every tracker that isn't dirty registers as a dependency of the window's `redraw_tracker` (`WindowInner::draw_contents`), so any later change dirties the window and requests a redraw.
Structural changes (repeaters, conditionals, popups) are caught because the walk itself reads the repeater models.

## Design

### Sync Pass

`WindowInner::sync_visuals` replaces `draw_contents`.
The host runs it on the UI thread when the window's tracker is dirty, at most once per frame tick or message batch.

1. `ensure_tree_instantiated()` outside any tracking, as upstream does.
2. Evaluate the walk inside the window tracker (`evaluate_as_dependency_root`), so the walk's reads register as dependencies.
   The root visual's scale is applied here too, from `WindowInner::scale_factor()`, so a scale change requests a pass.
3. Visit items back to front.
   Generated code visits in `z` order (`visit_item_tree_z_sorted`) when an element sets `z`.
   For each item:
   - Look up or create its node through `cached_rendering_data` (index + generation into the window's node table).
   - **Geometry**: read `item_rc.geometry()`; if it differs from the node's cached geometry, set `Offset`/`Size` on the visual.
   - **Content**: if the node's content tracker is dirty (or new), or the size changed, evaluate `ItemVTable::update_visual` inside it.
     The size is an argument of `update_visual`, so its tracker can't see it change.
     Otherwise register the clean tracker as a dependency of the window tracker.
   - The node keeps what `update_visual` returned, so a clean node skips its children as its last update said.
   - **Children**: compare the visited child sequence with the node's previous one and patch the `VisualCollection` (see below).
4. Nodes of live items not visited in this pass are detached but kept: a conditional or a replaced component may come back.
   `unregister_item_tree` releases nodes when their items are destroyed, and the pass drops any node whose item is gone.
5. The dispatcher queue commits implicitly when the handler returns (S1).

The pass walks all items whenever anything changed, like upstream.
If profiling shows the walk above about 1 ms for realistic trees, switch to per-node dirty handlers that enqueue nodes, plus a structure tracker per node with dynamic children.

### Node Table

- Per window, a slab of nodes: `{ visual, kind, item, geometry, content, result, children, parent }`.
- `CachedRenderingData { cache_index, cache_generation }` stays in every item struct (it's part of the generated-code contract) and holds the node handle.
  A generation mismatch means "no node yet".
- Item structs never hold COM objects, so the generated code's layout and the `FieldOffsets` stay unchanged.
- `unregister_item_tree` releases the nodes of the items it lists through `Renderer::free_graphics_resources`.

### Visuals per Item

| Item | Visual | Set by `update_visual` |
| --- | --- | --- |
| `Empty`, `Layer` | `ContainerVisual` | nothing |
| `Rectangle`, `WindowItem` | `SpriteVisual`, or a container when transparent | color brush, reused across color changes; gradient brushes with absolute mapping |
| `BasicBorderRectangle`, `BorderRectangle` | `ContainerVisual` with a floor `ShapeVisual`; a sprite without border and radius | a fill shape and a stroke shape laid out by upstream's `BorderRectLayout`; see [Antialiasing](#antialiasing) |
| `Opacity` | `ContainerVisual` | `Opacity`, which is group opacity on a container (S2) |
| `Clip` | `ContainerVisual` | `InsetClip`, or `CompositionGeometricClip` with a rounded rectangle or path |
| `Transform` | `ContainerVisual` | `TransformMatrix` |
| `Flickable` | `ContainerVisual` | `InsetClip` to the item, as upstream's `render` clips |
| Text items | `ContainerVisual` with a content sprite and a surface brush | the text drawn into a drawing surface; see [Text](#text) |
| `ImageItem`, `ClippedImage` | `ContainerVisual` with a floor sprite, and a child sprite per part of a nine-slice image | surface brushes, through effects for tiling and colorize; see [Images](#images) |
| `BackdropBlur` | `ContainerVisual` with a floor sprite | a blur effect over a backdrop brush; see [Images](#images) |

- `Transform` builds the matrix itself: Composition's `Scale` and `RotationAngle` apply scale before rotation, and Slint rotates first.
- `Layer`'s `cache-rendering-hint` has no equivalent; Composition retains content anyway.
- `Flickable` scrolls through its viewport, the `Empty` child the compiler puts between it and its content:
  scrolling changes the viewport's `x` and `y`, so only that visual's `Offset` changes.
- When an item needs a different visual type (a radius becomes non-zero), the node creates the new visual,
  copies offset, size, opacity, clip and transform, moves the children over, and replaces it in the parent's collection.

### Text

Text items (`SimpleText`, `ComplexText`, `StyledTextItem`, `TextInput`) keep upstream's layout: `textlayout/sharedparley` shapes and positions the glyphs, as it does for upstream's renderers ([S4](spike-s4-text.md)).
Only the drawing is new.

- **Recording.** `update_visual` calls upstream's `sharedparley::draw_text` or `draw_text_input` with a recording `GlyphRenderer`.
  It keeps the glyph runs, rectangles (selection, cursor, decorations) and clips as a display list, in physical pixels relative to the item.
  Nothing touches Direct2D while parley walks the layout.
- **Bounds.** The display list's bounds decide the surface: the union of the drawn glyphs and rectangles, padded for ink that overhangs the advances, cut by the clips in effect.
  They aren't the item's rectangle: an elided text keeps its last line even when it overflows the item, and alignment can put lines above it.
- **Surface.** The node stays a container and gets a content sprite: the first of its children, below the items' visuals, which children patching inserts above it.
  The sprite shows a `CompositionDrawingSurface` of the bounds' size, rounded up to whole pixels, through a `CompositionSurfaceBrush` with `SnapToPixels`, and is offset to the bounds' origin in logical pixels.
  A list with nothing to draw removes the sprite.
- **Replay.** The display list is drawn into the surface with a Direct2D device context at the window's scale factor, grayscale antialiasing.
  Glyph runs go through DirectWrite font faces made from parley's font data, without copying it: the faces are cached per font blob, face index, simulation and variation settings.
  Color glyphs (emoji) go through `TranslateColorGlyphRun`: COLRv1 paint trees (Segoe UI Emoji on Windows 11) need `IDWriteFactory8` and `ID2D1DeviceContext7::DrawPaintGlyphRun`; COLRv0 layers, bitmaps and SVG glyphs have their own calls.
  Strokes draw the glyph outlines; faux italic skews the run around its baseline, faux bold is DirectWrite's bold simulation.
  The code is in `visual_tree/text.rs` (recording and replay) and `visual_tree/graphics.rs` (devices and faces).
- **Changes.** The node's content tracker sees what parley reads, including the window's scale factor, so a change of text, style, size or scale factor redraws the surface.
  The cursor's blinking is a property too: it redraws the text input, which costs about 25 µs (S4).
- **Device.** One D3D11 device, its D2D device, and one `CompositionGraphicsDevice` per UI thread, shared by its windows.
  When the device is lost or replaced, every text node is marked dirty and redraws.
  The devices are never released: the main thread's thread-locals are destroyed while the process exits, after the other threads have ended, and releasing the devices then can block forever.
- **Cache.** The window's renderer keeps upstream's `TextLayoutCache`, which measurement and drawing share; destroyed components leave it as in upstream's renderers.
- **Input method.** `TextInput`'s `update_visual` sends the input method an update when the focused input's text changed, as upstream's `render` does, except for a read-only input, which disabled the input method when it got the focus.
- **Not yet.** Conic gradients draw in their first color (Phase 5 brings them with the other surface drawing); clips with a radius clip to their rectangle; a text larger than 16384 pixels is cut off (virtual surfaces would lift that).

### Antialiasing

Upstream's Skia renderer antialiases rounded rectangles, clips with a radius, and everything under a rotation, and draws axis-aligned rectangles without.
The compositor antialiases edges, clips and shapes only in `CompositionBorderMode::Soft`; the default `Hard` snaps them to pixels.

- A `Transform` that rotates, a clip with a radius, and a rounded rectangle's shapes get `Soft`.
  Everything else inherits, so it's `Hard` unless a rotation is above it.
- Soft mode also antialiases the edges of the visual's own bounds.
  Where a shape's edge lies on them, both apply and the edge comes out lighter than upstream's.
  So a node's shapes are in a floor `ShapeVisual` that reaches 1 logical pixel past the item on each side, with the shapes offset by that margin.
  Being below the children, the floor's border mode doesn't pass on to them.
- An arc of radius 0 ends a Direct2D figure where Composition reads it, so a square corner of a per-corner rounded rectangle has no arc.

Gradients interpolate straight colors in the compositor and premultiplied colors in upstream's renderers (Skia's `InPremul::Yes`).
Between stops of different alpha, `shapes.rs` adds seven intermediate stops that follow the premultiplied blend, and gives a transparent stop the color of its neighbor on each side.

### Images

`ImageItem` and `ClippedImage` place the image as upstream's Skia renderer does (`draw_image_impl`), with the effects of [S5](spike-s5-effects.md).
The code is in `visual_tree/image.rs`.

- **Parts.** `fit()`, or `fit9slice()` for a nine-slice image, gives the parts to draw, in physical pixels.
  One part shows in the node's floor sprite; the nine parts of a nine-slice image show in children of it.
- **Placement.** Each part's sprite has a `CompositionSurfaceBrush` with `Stretch::None`, alignment ratios 0, and a transform from raster pixels to the sprite: the part's clip moved to the origin, scaled by `source_to_target`.
  Upstream draws images from a whole-pixel origin (`pixel_align_origin`), so the floor sprite has `IsPixelSnappingEnabled`.
  A single part's fractional offset goes into the brush transform and an inset clip, so that only the item's origin snaps.
  The brush itself doesn't snap: `SnapToPixels` would also round the fractional offsets inside a scaled part.
  Tiles start at the part's offset cut to whole pixels, as upstream's `as i32` does.
- **Raster.** A bitmap shows at its own size.
  An SVG is rasterized at `scalable_render_size`, which keeps its aspect ratio as `tiny_skia::IntSize::scale_to` does; the brush stretches it over the requested size, as upstream's renderers do.
- **Surfaces.** One `CompositionDrawingSurface` per image's pixel data, raster size, and part, shared by the items that show it.
  The pixels go in with `CopyFromMemory` into the surface's own bitmap.
  A tiled part needs a surface of its own, since the `Border` effect wraps the whole surface.
  The cache keeps up to 64 MiB of surfaces, least recently used first out; items keep the surfaces they show either way, since composition surfaces have no weak references.
  A replaced device re-uploads the pixels of a surface when an item shows it next.
- **Effects.** Tiling is a `Border` effect with wrap on the tiled axes; colorize is a `Composite` effect with `SOURCE_IN`, the colorize brush as source.
  `Border` takes a source parameter only, so the tiles are colorized after tiling.
  That's the same for a color; a gradient spans the first tile instead of repeating with the tiles.
  Effect factories are cached per graph (`visual_tree/effects.rs`).
- **Rendering.** `ImageRendering::Pixelated` sets `NearestNeighbor` on the surface brush.
- **Known differences.** The compositor rounds a negative half-pixel origin up, where Skia rounds it away from zero: an image at x = -5.5 shows one pixel right of upstream's.

`BackdropBlur` shows a `GaussianBlur` effect over a `CompositionBackdropBrush` in its floor sprite.
The blur amount is in logical pixels, since the root visual carries the scale factor.
A backdrop brush sees only what's behind its sprite, so the sprite reaches 3σ beyond the item and an inset or rounded clip cuts it back to the item, as upstream's blur picks up content that far.

### Snapshots

`Window::take_snapshot()` syncs the window, moves its root visual into a container of the window's physical size, commits, and captures that container with `GraphicsCaptureItem::CreateFromVisual` ([S3](spike-s3-verification.md)).
A hosted window's target shows the container meanwhile, so the window doesn't flash.
The capture goes through a D3D11 device of its own, hardware or WARP, kept per thread and never released, like the drawing devices.
The pixels come back as straight-alpha RGBA, as upstream's renderers return them.
Capturing needs a desktop session with DWM.

### Children Patching

- Keep each node's child list from the previous pass.
- In the common case (same list), do nothing.
- Otherwise insert new visuals with `InsertAbove(new, previous_sibling)`, or `InsertAtBottom` for the first child, and remove dropped ones.
  Never use `InsertAtTop`: it's linear per call (S2).
- Reordering (`z` changes) removes and reinserts the moved visuals.

### Coordinates

- Item geometry is relative to the parent item, in logical pixels.
  Visual offsets use it directly.
- The window's root visual carries `Scale = scale_factor`; the compositor snaps axis-aligned edges (S2).
- Popups shown inside the main window (`PopupWindowLocation::ChildWindow`) get their own subtree above the main tree, offset by the popup location.

### Frames and Animations

- A property change dirties the window tracker; `request_redraw` posts one sync message, coalesced until the pass runs.
- When a pass finds active animations, it asks the frame thread (S1) for ticks on the compositor clock.
  Each tick moves the animation tick to now, and the resulting changes request the next pass.
- While frames tick, only the frame tick moves the animation tick.
  The message loop still fires timers, but moving the animation tick on every message would request a pass per message and spin the loop.
- With no animation, nothing runs, and the message loop keeps the animation tick current so that a new animation starts from now.

### Testing Hooks

- Detached windows have no HWND: a root visual that hangs nowhere, with the scale factor tests dispatch (S3).
  Their sync passes run when a test calls `sync_now`.
- Dumps walk the real visual tree; pixel tests capture the root after `wait_commit` (`tests/fixtures`).
- `tests/probe` runs code from the upstream compiler and dumps the tree of a real window after each step.

## Open Questions

- Whether to share one brush per color across the window (S2 measured the memory difference).
- How images and paths, which also draw into surfaces, follow the text design; settled with S5.
- `WindowInner::set_component` on a shown window keeps the old component alive until the window is hidden, as upstream does.
  Generated code only sets the component before showing.
