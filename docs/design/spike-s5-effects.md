# Spike S5: Effects, Images, Shadows and Backdrops

Code: `spikes/s5-effects`.
`s5-effects.exe <out-dir>` runs E1–E9 and saves the captures as PNGs; `s5-effects.exe <out-dir> e7` runs only E7.
Pixels are read back with the S3 capture helper; E8 captures the window itself.
Environment as in [S1](spike-s1-hosting.md).

## Results

### E1: Effects Without Win2D

`spikes/s5-effects/src/effect.rs` implements `IGraphicsEffect`, `IGraphicsEffectSource` and `IGraphicsEffectD2D1Interop` in Rust (`#[implement]`).
The object gives the compositor a Direct2D effect CLSID, its properties in the order of the effect's property indices, and its sources.
Sources are other such effects or a `CompositionEffectSourceParameter`, which the brush fills by name.

- `CreateEffectFactory` accepts it, and the brush draws a Gaussian blur of a surface brush (stripe contrast 255 → 7 at σ 4).
- Named properties work: `GetNamedPropertyMapping` maps `BlurAmount` to index 0 (`GRAPHICS_EFFECT_PROPERTY_MAPPING_DIRECT`).
  `brush.Properties().InsertScalar("Blur.BlurAmount", v)` changes the blur without a new factory.
- Cost: the first factory takes 1.3 ms; one for the same graph again takes 0.02 ms, so the compositor caches compiled graphs.
  `CreateBrush` takes 2 µs.

### E2: Images

| Upload of a premultiplied BGRA image | `CreateBitmap` + `DrawBitmap` | `CopyFromMemory` into the surface's bitmap |
|---|---|---|
| 256 × 256 | 0.16 ms | 0.07 ms |
| 1024 × 1024 | 1.7 ms | 0.7 ms |
| 2048 × 2048 | 2.6 ms | 2.4 ms |
| 4096 × 4096 | 16 ms | 9.7 ms |

- `CopyFromMemory` into the target bitmap of `BeginDraw`, at the offset `BeginDraw` returns, gives the exact pixels.
- Placement: a `SpriteVisual` with the image's surface brush, `Stretch = None`, alignment ratios 0 and `SnapToPixels`.
  `TransformMatrix = translate(−clip origin) × scale(1 / scale factor)` shows a source clip of (16,16)–(48,48) at scale 1.5 pixel for pixel.
  The sprite's bounds cut off the rest of the image.
- With `Stretch = None`, the alignment ratios default to 0.5 and center the surface before the transform.
- `NearestNeighbor` interpolation magnifies 16 × 16 to 64 × 64 pixel for pixel.

### E3: Tiling

A `Border` effect (`D2D1_BORDER_EDGE_MODE_WRAP`) over the transformed surface brush repeats it.
The tiles have the transform's scale and start at the transform's offset, pixel for pixel at scale 1.5.
Wrapping only x and clamping y, with the sprite one tile high, tiles one axis.
The effect wraps the whole surface: a source clip with tiling needs the clipped part in a surface of its own.

### E4: Colorize

A `Composite` effect in `SOURCE_IN` mode draws its source (input 1) where its destination (input 0) is opaque.
With the image as destination and the colorize brush as source:

- A `CompositionColorBrush` gives the color with the image's alpha; transparent pixels stay transparent.
- A `CompositionLinearGradientBrush` works as an effect source; its relative mapping spans the sprite.
- The destination can be the tiling effect of E3 or a `CompositionNineGridBrush`.

### E5: Nine-Slice

`CompositionNineGridBrush` with the image's surface brush as source and `Insets = 16` keeps 16-pixel edges on a 240 × 120 sprite.
`InsetScales = 1 / scale factor` keeps them at one image pixel per physical pixel at scale 1.5.
The source surface brush must have `Stretch = Fill`.
Its default is `Uniform`, which centers a square in the grid.

### E6: Box Shadow

| | Direct2D bake | Composition `DropShadow` |
|---|---|---|
| Method | `Shadow` effect over a command list with the shape, σ = blur / 2 (upstream's `blur_sigma`), into a surface padded by the blur | `BlurRadius = blur`, masked by a rounded-rectangle surface |
| Falloff of a 16 px blur: to 16 % / 2 % of the alpha | 8 px / 16 px (σ = 8, as upstream) | 5 px / 11 px (σ ≈ `BlurRadius` / 3) |
| Spread, inset, a border without fill | the shape upstream's `BoxShadowOptions` describes | not expressible without extra masks |
| Cost | 0.15–0.25 ms per bake, 100 × 50 to 1200 × 800 at scale 1.5 (2 ms for the very first) | none on resize |

### E7: Backdrop Blur

A `SpriteVisual` whose effect brush is a Gaussian blur of `CreateBackdropBrush()` blurs what lies beneath it.
The test scene has 8 px stripes; the contrast is 255 outside the sprite and 4 inside it.

- It works in a window and in a visual that no window hosts (the capture of S3).
- A rounded `CompositionGeometricClip` clips it: the stripes stay sharp outside the radius.
- `BlurAmount` is in the visual's units: σ 2 under a scale of 2 matches σ 4 unscaled (difference 26 over 24 pixels, against 1140 for σ 2).
- Under a container with `Opacity = 0.5`, the blurred backdrop mixes half and half with the sharp one beneath (contrast 130).
- The brush reads only the backdrop inside the visual's bounds; `BorderMode = Hard` keeps the edges opaque.
  Skia reads around the bounds too, so the edges differ slightly from upstream.

### E8: System Backdrop

`DwmSetWindowAttribute(DWMWA_SYSTEMBACKDROP_TYPE, DWMSBT_MAINWINDOW)` shows Mica through a transparent composition root.
The client area is transparent without it, and Mica's color with it (the inactive color, since the spike doesn't activate its window).
`DwmExtendFrameIntoClientArea` isn't needed.

### E9: Conic Gradients

There is no conic gradient brush in Composition or Direct2D; the spike computes the pixels on the CPU.

- `CompositionSpriteShape.FillBrush` rejects a surface brush (`E_INVALIDARG`, "Unsupported source brush type").
- A `SpriteVisual` with the surface brush and a rounded geometric clip draws it correctly.

## Decisions

- **Effects through `IGraphicsEffectD2D1Interop`**, implemented in the runtime: Gaussian blur, border and composite.
  The fallback of baking with Direct2D isn't needed.
  A thread-local cache keeps one factory per effect graph; a property that changes often, such as `BlurAmount`, is a named property of the brush.
- **One surface per image**, shared by every item that shows it, filled with `CopyFromMemory`.
  The items' visuals place it with the brush transform from upstream's `fit()` (`graphics/image.rs`).
  The sprite takes the fitted rectangle, and the transform is `translate(−clip origin) × scale(source_to_target / scale factor)`.
  Every `image-fit` reduces to this, so the brush always has `Stretch = None` and alignment ratios 0.
  - Tiling wraps the surface brush in a `Border` effect, per axis.
    A source clip with tiling copies the clipped part into a surface of its own.
  - Nine-slice uses `CompositionNineGridBrush` with a `Fill` source.
  - Colorize wraps whatever draws the image in a `Composite` (`SOURCE_IN`) effect with the colorize brush as source.
  - `image-rendering: pixelated` uses `NearestNeighbor`.
  - SVGs are rasterized at upstream's `scalable_render_size`, into a surface of their own.
  - A device replacement uploads the pixels again.
    The image keeps its decoded pixels, so this needs no decoding.
- **Box shadows are baked** by Direct2D into a surface from upstream's `BoxShadowOptions`.
  They are redrawn when the options change, including the size.
  `DropShadow` can't express spread, inset or a shadow of a border.
- **`BackdropBlur` is a sprite with a blurred backdrop brush**, `BlurAmount` = `blur` in logical pixels (the root carries the scale factor), hard border mode.
  Its radii become a geometric clip, as for `Clip`.
- **Conic gradients are computed on the CPU** into a surface.
  A rectangle's fill shows it in a clipped sprite, and text uses it as a Direct2D bitmap brush.
  A conic border bakes the whole rectangle with Direct2D.

## Open Items

- A window background material (Mica) works, but upstream has no API for it.
  Adding one would be a fork extension outside generated-code compatibility; it waits for a request.
- Downscaling large images with linear interpolation aliases; mipmaps (`CompositionMipmapSurface`) or a CPU resize on upload are candidates if applications need it.
