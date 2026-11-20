# Costs Against Upstream

What the fork costs an application against upstream Slint 1.19.0, measured from outside the process by `tools/compare-numbers.ps1`.
The README's "Costs" section quotes the gallery rows.

## Setup

- The same app, release builds of each: upstream's gallery (its first page keeps an indeterminate `ProgressIndicator` and `Spinner` turning, so it draws every frame while idle) and upstream's custom-titlebar (static).
- Upstream runs with its default, winit and FemtoVG on OpenGL, and with the software renderer (`SLINT_BACKEND=winit-software`).
  Skia wasn't built, though it is the closest match on Windows: GPU drawing with partial updates.
- One machine on 2026-11-20: Ryzen 7 8745H, Radeon 780M, 2560x1440 at 60 Hz, 125 %. Medians of 7 runs after a warm-up.
- "Answers" is when the visible window answers a `WM_NULL`, so its thread is pumping messages.
  It isn't when the first frame reaches the screen: a sent message is handled before `WM_PAINT`, so upstream may answer before it first draws.
- Not counted: memory that `dwm.exe` holds, where the fork's visuals and every window's surface live.

## Results

| App | Side | Window shown | Answers | Working set | Private | GPU dedicated / shared | Threads | CPU while idle |
|---|---|---|---|---|---|---|---|---|
| gallery | fork | 56 ms | 115 ms | 59 MB | 47 MB | 12 / 3 MB | 28 | 3.0 % |
| gallery | upstream | 36 ms | 116 ms | 123 MB | 116 MB | 20 / 18 MB | 25 | 8.7 % |
| gallery | upstream, software | 36 ms | 42 ms | 28 MB | 7 MB | none | 10 | 2.8 % |
| custom-titlebar | fork | 56 ms | 102 ms | 52 MB | 39 MB | 12 / 3 MB | 27 | 0 % |
| custom-titlebar | upstream | 37 ms | 122 ms | 102 MB | 109 MB | 15 / 13 MB | 25 | 0 % |
| custom-titlebar | upstream, software | 35 ms | 40 ms | 24 MB | 5 MB | none | 10 | 0 % |

CPU is a percentage of one core. Release executables: gallery 24.3 MB on the fork, 22.2 MB upstream; custom-titlebar 17.4 MB and 15.4 MB.

## Reading

- The fork isn't lighter. Against the software renderer it answers 60-70 ms later and holds 35-40 MB more private memory; against the default it answers at the same time and shows its window 20 ms later.
- The fork's cost is starting and holding Direct3D 11, Direct2D, DirectWrite and Composition (inferred).
- The default holds more memory than the fork mostly for its OpenGL driver, which varies with the GPU, not for Slint's design (inferred).
- Only CPU while animating favors the fork over the default, and it matches the software renderer. FemtoVG redraws the whole window every frame; the fork and the software renderer redraw what changed (inferred).
- Not measured: time to the first frame on screen, other GPUs and drivers, large windows and live resizing, where the software renderer's cost grows with the pixels.

## Why Not

- A heavier animation demo chosen after these results: it would pick a scene the fork wins. The comparison for delegating animations to Composition declares its scenes, metrics and predictions first, counts frames on both sides, and includes `dwm.exe`.
