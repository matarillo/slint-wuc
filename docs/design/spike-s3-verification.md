# Spike S3: Verifying What the Compositor Draws

Code: `spikes/s3-capture` (run `s3-capture.exe <out-dir>`), helpers in `spikes/common` (`Capturer`, `dump_tree`, `wait_commit`).
Environment as in [S1](spike-s1-hosting.md).

## Results

1. `GraphicsCaptureItem::CreateFromVisual` + `Direct3D11CaptureFramePool` + a staging texture reads back exact pixels of a visual subtree, in the visual's own coordinate space (one unit per pixel).
   Colors and edges matched exactly.
2. It also works for a tree that isn't attached to any `DesktopWindowTarget` or window.
3. After changing a brush and waiting for `RequestCommitAsync`, the next capture shows the new color.
4. The first capture takes 7–29 ms; repeated captures take about 14 ms each (20 in 407 ms).
5. Walking `ContainerVisual::Children` and reading the runtime class, offset, size, opacity, scale, and clip yields a stable text dump.
6. Captured pixels are premultiplied BGRA.

## Decisions

- **Tests need no visible window.**
  The `testing` module builds the item tree's visuals under a detached root with a fixed scale factor and reads them back through capture.
  A test window is only needed for input and HWND behavior.
- **Two kinds of checks:**
  - Tree dumps, the main check: deterministic text from walking the real visual tree (not the runtime's own node table), compared as snapshots.
    They're fast and don't depend on the GPU.
  - Pixel captures, for rendering details (borders, clips, gradients, text): compared against reference PNGs with a small per-channel tolerance.
- **Sequence for a pixel test:** run the sync pass, `wait_commit`, then capture.

## Open Items

- Capture needs a session with DWM.
  This machine has one; whether a CI runner's session does is untested.
  If not, CI runs dump tests only and pixel tests run locally.
