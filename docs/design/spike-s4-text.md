# Spike S4: Text

Code: `spikes/s4-text`.
`s4-text.exe <out-dir>` runs T1–T6 and saves the captures as PNGs; `s4-text.exe <out-dir> t1` runs only T1; `s4-text.exe <out-dir> ime` opens the window of T7.
Pixels are read back with the S3 capture helper.
Environment as in [S1](spike-s1-hosting.md).

## Results

### T1: Sharpness at Fractional Scales and Positions

DirectWrite lays out "Slint テキスト Agyj 0123" (Segoe UI, 14 DIP).
Direct2D draws it at `96 × scale` DPI into a `CompositionDrawingSurface` of the layout's size in physical pixels.
A `SpriteVisual` of the same size in DIP shows it through a `CompositionSurfaceBrush` (`Stretch = Fill`), under a parent scaled by the scale factor, as in the runtime.
The captures count pixels with partial and with full ink, and compare the inked pixels with the text at a whole-pixel position.

| Scale | Position (px) | Free sampling | `SnapToPixels` |
|---|---|---|---|
| 1 | 10.3, 10.5 | blurred: 974 partial, 0 full | identical to whole pixels (416 / 142) |
| 1.25 | 12.875, 13.125 | blurred: ~1370 partial, ≤5 full | identical (616 / 224) |
| 1.5 | 15.45, 15.75 | blurred: ~1890 partial, 2 full | identical to every other capture (927 / 342) |
| 1.75 | 18.025, 18.375 | blurred | identical (1080 / 580) |
| 2 | 20.6 | blurred | identical (801 / 944) |

- Without snapping, a surface at a fractional physical position is resampled, and the text loses all its full-ink pixels.
- `CompositionSurfaceBrush.SnapToPixels` puts the surface on whole pixels.
  The pixels are then the same as at a whole-pixel position, at every scale.
  Nearest-neighbor interpolation on top of it changes nothing.
- At 1.5 the very first capture of the run differs slightly (915 / 305) from all later ones, even at the same position and with the same sampling; it looks the same to the eye.
  It doesn't depend on the position or the sampling, so it isn't a sampling effect.
  It may be a glyph cache warming up.

### T2: Antialiasing

| Mode | Target | Result |
|---|---|---|
| Grayscale | transparent | 927 partial, 342 full, no color fringes |
| ClearType | transparent | **nothing is drawn** |
| ClearType | opaque white | color fringes (max \|R−B\| = 190) |
| Default | transparent | the same pixels as grayscale |

### T3: Glyphs

- Color emoji draw in color with `D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT`, and in monochrome without it.
- Japanese and Arabic text in a "Segoe UI" format fall back to system fonts by themselves (non-empty ink, plausible widths).

### T4: Cost

1,000 text items at scale 1.5, one surface each, in a window:

| Step | Time |
|---|---|
| Create 1,000 layouts | 1.4 ms |
| Create 1,000 surfaces and draw them | 45 ms |
| Create and insert the visuals | 4.5 ms |
| Commit | 22 ms |
| Redraw every surface | 12.6 ms |
| `Resize` and redraw every surface | 11.5 ms |
| New surface and draw, every item | 11.4 ms |
| Redraw one surface | 0.025 ms median, 0.029 ms p95 |

Memory grew by 21 MB: about the surfaces' pixels (four bytes each).

### T5: Device Replacement

- After `ICompositionGraphicsDeviceInterop::SetRenderingDevice` with a new D2D device, `RenderingDeviceReplaced` fires.
  Every surface is then empty: the capture shows no ink.
- Drawing the surface again with the new device restores it.

### T6: Measurement

- `IDWriteTextLayout::GetMetrics` and `GetLineMetrics` give the size, line count, baseline and line height in DIP.
  Segoe UI at 14 DIP: ascent 15.11, descent 3.51, line height 18.62.
- `HitTestTextPosition` gives the caret rectangle (x, y, height) for a UTF-16 index.
- `HitTestPoint` gives a UTF-16 index and whether the trailing side was hit.
  The right half of an emoji gives the index after its surrogate pair, never one inside it.
- Slint's interface speaks UTF-8 byte offsets, so the runtime converts in both directions.
- `IDWriteFont::GetMetrics` gives ascent, descent, cap height and x-height in design units, for `font_metrics`.

### T7: Input Method (Manual)

The window takes text through IMM32, the way winit does:

- `WM_IME_SETCONTEXT` clears `ISC_SHOWUICOMPOSITIONWINDOW`, so the system draws no composition window.
- `WM_IME_COMPOSITION` reads `GCS_COMPSTR`, `GCS_CURSORPOS` and `GCS_RESULTSTR`.
  The window draws the composition inline, underlined, and the caret at the cursor position.
  It doesn't pass the message to `DefWindowProc`, so no `WM_IME_CHAR` follows.
- `ImmSetCandidateWindow` with `CFS_EXCLUDE` and the caret rectangle places the candidate list below the caret.

Result, with the Microsoft Japanese IME:

- The composition shows inline and underlined, with no window of the system's.
- The candidate list opens below the caret and doesn't cover the composition.
- Committed text stays, and Backspace deletes it.
- Moving between clauses moves the caret and the candidate list with it.

Reconverting text that was already committed wasn't tried.
It needs `WM_IME_REQUEST` with `IMR_RECONVERTSTRING`, which hands the input method the text around the caret.
winit doesn't answer it, so upstream can't reconvert on Windows either.

## Decisions

- **One surface per text item.**
  A text item is a `SpriteVisual` whose `CompositionSurfaceBrush` (`Stretch = Fill`, `SnapToPixels = true`) shows a `CompositionDrawingSurface`.
  The surface has the size of the item's text in physical pixels, rounded up.
  The visual has that size divided by the scale factor, in DIP.
  Redrawing is cheap enough (25 µs) to redraw on every change of text, color or size, with `Resize` when the size changes.
- **Draw at the window's scale.**
  The device context gets `SetDpi(96 × scale)` and a translation by the surface's atlas offset; layout and drawing stay in DIP.
  A scale factor change redraws every text surface.
- **Grayscale antialiasing**, always.
  Slint's text has no opaque background of its own, and ClearType draws nothing on a transparent target.
- **Color fonts on**, always (`D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT`).
- **One device per thread**: a D3D11 device (BGRA support), its D2D device, and one `CompositionGraphicsDevice` that every window of the thread shares.
  `RenderingDeviceReplaced` redraws every surface.
  A `BeginDraw` or `EndDraw` that fails with `DXGI_ERROR_DEVICE_REMOVED`, `DXGI_ERROR_DEVICE_RESET` or `D2DERR_RECREATE_TARGET` creates a new device and hands it to `SetRenderingDevice`, which leads to the same redraw.
- **Measurement stays with parley** (decided after the spike, superseding the next bullet).
  Upstream's text behavior (wrapping, elision, line limits, styled text, selection, the preedit underline) lives in `textlayout/sharedparley`, and upstream's test cases expect its numbers.
  Only drawing changes: parley's glyph runs are drawn with Direct2D into the surface of the first decision.
  DirectWrite's measurement below stays the fallback if parley turns out wrong for applications.
- **Measurement through `IDWriteTextLayout`** (superseded), with the system font collection and its fallback.
  The text item keeps its layout until its text, font or wrapping width changes.
  The renderer interface's calls map as follows:
  - `text_size` → `GetMetrics`.
  - `font_metrics` → `IDWriteFont::GetMetrics`, scaled to the font size.
  - `text_input_cursor_rect_for_byte_offset` → `HitTestTextPosition`.
  - `text_input_byte_offset_for_position` → `HitTestPoint`.
  - UTF-8 ↔ UTF-16 conversion at that boundary.
  Whether `text_size` includes trailing whitespace is left to the upstream text cases of Phase 4.
- **IMM32 for input methods**, as winit (and so upstream on Windows) uses; the manual check passed.
  The host keeps the input method behind one module, so TSF can replace it later for reconversion and context.
  Composition updates become Slint's composition key events (`UpdateComposition` with the preedit text and cursor, `CommitComposition` with the result).
  The `InputMethodRequest`s of the text input give the caret rectangle for `ImmSetCandidateWindow`.
  TSF stays out unless an input method needs it.

## Open Items

- Reconversion (`IMR_RECONVERTSTRING`), which upstream lacks; a candidate for after Phase 4, since the text input knows the text around the caret.
- The first-capture difference at scale 1.5 in T1.
- A real device loss (not a replacement) wasn't simulated.
  The recovery path above is the documented one; Phase 4 tests it by replacing the device.
