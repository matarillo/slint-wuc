# Spike S2: Composition Primitives

Code: `spikes/s2-primitives` (run `s2-primitives.exe <out-dir>`, `… perf`, `… insert`).
Pixels are read back with the S3 capture helper; the PNGs land in `<out-dir>`.
Environment as in [S1](spike-s1-hosting.md).

## Results

| Test | Question | Result |
|---|---|---|
| T1 | Is `Visual.Opacity` on a container group opacity? | Yes. Two overlapping opaque children under a 0.5 container give `[127,127,255]` at the overlap, the group result (per-visual would be `[127,63,191]`). |
| T2 | `ShapeVisual` + `CompositionRoundedRectangleGeometry` with fill and stroke | Works. With the geometry inset by `border/2` and the radius reduced by `border/2`, the stroke covers exactly pixels 0…3 of a 4 px border, like Slint's inner border. Corners outside the radius stay transparent. |
| T2 | Children of a `ShapeVisual` | Drawn above the shapes and not clipped to the parent's bounds. |
| T3 | Per-corner radii | A D2D path geometry wrapped in a Rust `#[implement(IGeometrySource2D, IGeometrySource2DInterop)]` object becomes a `CompositionPath`. Radii 0/20/4/30 render correctly. Building path and shape takes 0.5 ms. |
| T4 | Rounded clip for children | `CompositionGeometricClip` with a rounded-rectangle geometry clips children at the corners and at the bounds. |
| T5 | Gradients | `CompositionLinearGradientBrush` and `CompositionRadialGradientBrush` render as expected. There is no conic gradient brush. |
| T6 | Fractional scale | With the root at `Scale = 1.25`, a sprite at x = 10 DIP (12.5 px) gets a hard edge at pixel 12. The compositor snaps axis-aligned sprite edges to pixels; nothing blurs. At 1.5 the edges land on whole pixels anyway. |

### Visual Count (T7, T8)

Creating a `SpriteVisual` with offset, size, and brush costs about 0.6 µs, linearly up to 20,000.
Insertion cost depends on the method:

| n | `InsertAtTop` | `InsertAtBottom` (reverse order) | `InsertAbove(new, previous)` | 100 per container |
|---|---|---|---|---|
| 1,000 | 1.8 ms | 0.1 ms | 0.1 ms | 0.2 ms |
| 10,000 | 217.8 ms | 1.5 ms | 1.5 ms | 1.9 ms |
| 20,000 | 2,004 ms | 3.8 ms | 4.5 ms | 6.1 ms |

`InsertAtTop` walks the whole sibling list, so filling one parent with it is quadratic (50,000 children took 16 s).

With a window attached (T7, sprites in one container):

| n | First commit | Update every offset | Commit after update | In-process memory |
|---|---|---|---|---|
| 1,000 | 17–25 ms | 0.2 ms | 18–22 ms | +0.3–0.5 MB |
| 10,000 | 27–52 ms | 2–4 ms | 20–30 ms | +3.6–6.5 MB |
| 50,000 | ~100 ms | ~10 ms | 80–170 ms | +20–31 MB |

The lower memory figure is for one shared brush, the higher for one brush per visual.
DWM's own memory couldn't be read (`OpenProcess` on dwm.exe is denied to this user).

## Decisions

- **One item, one visual** holds up to about 10,000 live visuals per window, with full updates within two frames.
  Beyond that, rely on Slint's virtualized `ListView`; don't optimize the mapping until a real tree needs it.
- **Never use `InsertAtTop`** in the runtime.
  The sync engine inserts with `InsertAbove(new, previous_sibling)`, or `InsertAtBottom` for the first child, and keeps a handle to each node's previous sibling for that.
- **Visual types**:
  - `Empty`, `Transform`, `Layer` → `ContainerVisual`.
    `Layer`'s caching hint has no effect on a retained tree.
  - `Rectangle` without border and radius → `SpriteVisual` with a color or gradient brush.
  - `BasicBorderRectangle` / `BorderRectangle` → `ShapeVisual` with one `CompositionSpriteShape`: a rounded-rectangle geometry for a uniform radius, a D2D path geometry for per-corner radii.
    Switch the node's visual type when border or radius change between zero and non-zero; children move with it.
  - `Clip` → `Visual.Clip` with a rounded-rectangle or path geometry (`CompositionGeometricClip`), or an inset clip without radius.
  - `Opacity` → `Visual.Opacity` on the item's container. No `LayerVisual` needed.
- **Brushes**: map Slint linear and radial gradients to the native brushes.
  Conic gradients have no native brush; draw them into a surface (decide in Phase 5 together with the other surface work).
  Share one `CompositionColorBrush` per color to save memory.
- **Coordinates**: everything below the root is in logical pixels (DIP); the root carries `Scale = dpi / 96`.
  The runtime doesn't snap sprites and shapes itself.
  Surface-backed content (text, images) is checked separately in S4.

## Open Items

- With a semi-transparent border, a single fill-and-stroke shape blends the inner half of the stroke over the fill.
  Phase 2 must compare this with upstream's renderers and, if they differ, use a fill geometry inset by the full border width.
- Measure how many `ShapeVisual`s with path geometries a window can hold; T7 covered only `SpriteVisual`s.
