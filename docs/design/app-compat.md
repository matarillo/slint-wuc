# A Real Application on the Fork

What it took to move slokk-slint (the owner's contacts app, written for upstream Slint 1.18) onto the fork, and what it draws differently, by `tools/app-compat.ps1`.
The app is copied out and only the copy's `Cargo.toml` changes.

## Setup

- Three builds of the same sources: as locked (upstream 1.18.1), upstream 1.19.0 (`slint = "1.19"`, `slint-build = "=1.19.0"`), and the fork (the same, plus `[patch.crates-io]`).
  The upstream and fork manifests differ only by the patch, so a difference between them is the fork's; 1.18.1 against 1.19.0 shows what changed inside upstream.
- Screens from the app's own `--screenshot` (`take_snapshot`), 17 scenes in `tools/app-compat/slokk.scenes`, `SLINT_SCALE_FACTOR=1`, upstream with its default renderer (FemtoVG).
  Predictions were written before the first shot.
- One machine on 2026-11-20, as in [costs](costs.md).

## Adoption

- The least change for an app with a lock: `slint-build = "=1.19.0"` and `[patch.crates-io]`.
  Cargo then re-resolves the lock by itself (68 packages) and picks the fork, even with `slint = "1.18"` left as it is; no `cargo update`.
- The patch alone changes nothing: the lock keeps slint 1.18.1, and Cargo only warns that the patch was not used.
- The fork's lock is smaller than upstream's: against the app's, 541 lines added and 3,365 removed (winit, OpenGL and the other backends go), where upstream 1.19.0 changes 217 and 219.
- The app builds with `x86_64-pc-windows-gnullvm` too, upstream and fork; the fork's screenshots are the same pixels as with MSVC.

## Screens

Every scene has the same layout: positions, sizes, line breaks and selection states match.
Between 0.3 % and 1.7 % of the pixels differ by more than 16 (of 255), 0.07 % to 0.9 % by more than 64; 1.18.1 against 1.19.0 stays under 0.25 % (welcome-2 aside).

| Difference | Where | Reason |
|---|---|---|
| Glyph edges, a little heavier on the fork | all text | DirectWrite grayscale against FemtoVG's rasterizer ([S4](spike-s4-text.md)) |
| Tiny rounded shapes are fainter | the dotted ring of the compact side menu's create button (2 px dots) | Composition's coverage of a soft-edged shape: below |
| The gooey edge of the splash and welcome art | one lobe, about 5 px | The shape follows wall-clock time, and the fork's first frames come later: below |
| Edges of the scaled illustrations, under a pixel | splash, welcome | Resampling of a downscaled PNG (inferred, not investigated) |
| The sign-in code | welcome-2 | Random per run |

Predicted and not found: a different weight for Quicksand's synthesized bold, and shadows that differ in their blur.

### Small Rounded Shapes

White rounded rectangles on black, the ink summed per shape (1 = one full pixel), at whole and fractional positions:

| Shape | Area | Upstream FemtoVG | Upstream software | Fork |
|---|---|---|---|---|
| 2 px, radius 1 | 3.14 | 2.92 | 3.33 | 2.00–2.50 |
| 4 px, radius 2 | 12.57 | 12.6–12.9 | 13.21 | 11.5–11.9 |
| 6 px, radius 3 | 28.27 | 27.7–28.4 | 28.77 | 27.5–27.9 |
| 10 px, radius 5 | 78.54 | 77.8–78.1 | 79.15 | 76.7–77.2 |
| 2 px, square | 4 | 4.0–4.5 | 4 | 4 |

- The loss is about a pixel of ink per shape, so it shows only on shapes a few pixels wide; the dotted ring keeps 65 % of upstream's ink.
- The fork can't change the compositor's coverage. Drawing such shapes into a surface instead would cost a surface per shape for a few pixels (Why Not).

### Time-Driven Animation in a Snapshot

The splash moves its edge from a `Timer` with the time since the last tick, clamped to 1.5 steps of 1/60 s.
The fork's ticks are as regular as upstream's (16.8 ms against 16.6 ms on average, at most 24 ms), but its first two come after 151 and 64 ms where upstream's come after 216 and 24 ms, and both lose the clamped time.
After the same delay the fork's simulation is a few steps behind, so the shot shows another moment. Each side gives the same shot from run to run.

## Owner's Checks

On the fork's release build, 2026-11-20, on the same machine:

- Japanese IME in the edit panel, and scrolling the contacts with the wheel: no problem.
- The splash to the main screen: smooth.
- The detail panel sliding in and out when a contact is selected: not smooth, as at a low frame rate. Upstream's is smooth.
- Resizing by the window's edge: the fork redraws the whole window while dragging, but not smoothly. Upstream redraws the side menu and leaves the main area until the drag ends, so a hidden panel shows meanwhile. Which is right would need Flokk (the Flutter original) to compare; not done.

### The Panel's Slide, Frame by Frame

Traced on the fork with temporary timers (1280×820, scale 1). The slide animates the content's width, so every row of the list is laid out again in each frame:

| Step, per frame | Time |
|---|---|
| The tick: animations, layout, change handlers | 2 ms |
| Accessibility: rebuilding the 612 nodes whose bounds moved, after the tick | 7 ms |
| The sync pass: text laid out and drawn again (57), box shadows redrawn (14), the walk over 3,082 items | 5–7 ms |
| Accessibility again, after the sync pass (the list's repeater moves its rows while visited) | 3 ms |

About 18 ms in all, more than a frame of 16.7 ms, so the panel moves every 17 to 23 ms, at times 30 ms, in uneven steps. DWM itself composes every frame, on both sides.

- The accessibility work runs because a UI Automation client on the machine keeps the tree active (PowerToys, the text input host and the like run there; which one isn't known). Without one, a frame would take the other steps' 8 ms or so (not measured without a client).
- Upstream's winit backend has the same accessibility code, so it may pay the same; not measured, as clicks posted to winit's window don't reach the app.
- Resizing was not traced; it lays out the same list at every size, and is likely the same cost.

The accessibility update now waits for the end of a frame, after its sync passes, so it runs once per frame (6–8 ms) and sees what the frame draws.
Timed per message around the event loop, three slides each:

| | Mean step | Steps over 20 ms |
|---|---|---|
| Before | 18.1–21.6 ms | 6–13 of 20–24 |
| Once per frame | 16.9–18.0 ms | 2–5 of 23–25 |
| And text left as drawn | 15.7–17.0 ms | 1–4 of 25–27 |

With the text left as drawn when only its row's width changes, the sync pass takes 3.6–5.9 ms instead of 5–7; 42 to 55 of the 57 texts keep their drawing; the others move with the width, as a centered text does.
A frame now takes 12–15 ms, of which the accessibility update is 5–7 ms.

Tried and dropped: updating accessibility at most every 100 ms while an animation runs. Each update then took 9–11 ms at once, so a frame was missed every 100 ms, with 2–6 steps over 20 ms per slide instead of 1–4; the sync pass and the tick also took 1–2 ms more (cause not looked into).

## Fixed

- `Window::set_position` before `show()` was dropped: the window has no HWND yet, and was created at `CW_USEDEFAULT`.
  It is now kept and applied once the window exists, still hidden, as a size set before `show()` is (`tests/input/tests/window.rs`).
- Between a frame's tick and its sync passes, the event loop moved the animation tick to the current time again, so the sync passes drew another moment than the tick and the animated properties were evaluated twice per frame.
  A frame now lasts until a message posted after its sync passes. No test: it shows only in the timing of a real event loop.

## Numbers

Release builds with the `screenshot` feature, measured as in [costs](costs.md) (medians of 7 runs, 125 %, the scale of the monitor).
Scenes and predictions were written before measuring:

- startup: a normal launch, which shows the splash and then the welcome page, both animated.
- dashboard: `--phase main --page-name dashboard`, nothing animating.
- splash: `--phase splash`; a 16 ms `Timer` rebuilds the edge `Path` and moves the clouds, so every frame draws.

| Scene | Side | Window shown | Answers | Private | GPU dedicated / shared | CPU |
|---|---|---|---|---|---|---|
| startup | fork | 82 ms | 201 ms | 126 MB | 39 / 78 MB | 13.0 % |
| startup | upstream | 18 ms | 149 ms | 211 MB | 17 / 55 MB | 10.0 % |
| startup | upstream, software | 20 ms | 66 ms | 37 MB | none | 20.8 % |
| dashboard | fork | 76 ms | 202 ms | 128 MB | 73 / 78 MB | 0.3 % |
| dashboard | upstream | 25 ms | 147 ms | 259 MB | 54 / 100 MB | 0.6 % |
| dashboard | upstream, software | 19 ms | 64 ms | 35 MB | none | 0.2 % |
| splash | fork | 76 ms | 199 ms | 127 MB | 48 / 74 MB | 21.7 % |
| splash | upstream | 19 ms | 148 ms | 199 MB | 17 / 53 MB | 9.5 % |
| splash | upstream, software | 20 ms | 66 ms | 35 MB | none | 20.6 % |

CPU is a percentage of one core, over 10 s after 3 s of settling. Release executables: 19.0 MB on the fork, 20.7 MB upstream.

Against the predictions:

- Startup: the window shows 60 ms after upstream's and answers 50 ms later; predicted 20 ms and the same time. The gallery's gap was 20 ms ([costs](costs.md)), so this app's start costs the fork more (cause not measured).
- Memory: 128 MB private on the fork, predicted 45–60 MB: under half of upstream's default, but more than three times the software renderer's 35 MB. The gallery's was 47 MB; this app decodes larger images (cause not measured).
- Idle dashboard: nearly no CPU on every side, as predicted.
- Splash: the fork takes 2.3 times the CPU of upstream's default and as much as the software renderer, as predicted (fork at least the default). The cost per frame of a rebuilt `Path` and of what moves with it isn't broken down.

## Why Not

- Drawing small rounded rectangles into a Direct2D surface to match upstream's coverage: a surface and its memory for a few pixels each, and a second code path for one look. Reconsider when an app's design depends on dots or rings of a few pixels.
- Moving the cursor away before each shot, so a hover can't show: that moves the owner's pointer. A shot that shows a hover is taken again.
