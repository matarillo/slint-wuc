# Upstream's Examples on the Fork

Upstream 1.19.0's Rust examples and demos, built and started on the fork by `tools/upstream-examples.ps1`, on one machine on 2026-11-20 (125 %).
Each is copied out of `reference/slint-1.19.0` with its sources unchanged, and only its `Cargo.toml` is rewritten to what an application outside upstream's repository writes, with `[patch.crates-io]` to the fork.
A run builds a target, starts each of its binaries, captures the first window after 2.5 s and closes it.
Nothing is clicked, so it shows that an application starts and draws its first screen, not that every page works.

## Results

| Result | Examples and demos |
|---|---|
| Builds, starts and draws as upstream's code expects | 7guis (booker, cells, circledraw, crud), carousel, dnd-kanban, ffmpeg (below), gallery, imagefilter, maps, memory, native-gestures, plotter, runtime_key_bindings, slide_puzzle, speedometer, system-tray, todo, todo-mvc, virtual_keyboard; demos energy-monitor, food-processor, home-automation, printerdemo, usecases, weather-demo |
| Uses an API the fork removed | opengl_texture (`BorrowedOpenGLTextureBuilder`, `BackendSelector`), opengl_underlay (`BackendSelector`), gstreamer-player (`BackendSelector` with OpenGL ES), wgpu_texture and bevy's three (`unstable-wgpu-30`; bevy-hosts-slint a platform of its own and `SoftwareRenderer`), servo (`unstable-wgpu-30`, `unstable-winit-030`, `slint` from upstream's git), launcher (`unstable-libinput-09`, for Linux KMS), analogclock (`mcu-board-support`, which depends on upstream's internal crates) |
| Doesn't build on upstream either | async-io: its `serde` lacks the `derive` feature, which upstream's workspace supplies; built alone against upstream's crates.io release, it fails the same way |

Out of scope: the C++ examples, the microcontroller ones (`mcu-*`, `printerdemo_mcu`, `uefi-demo`, `safe-ui`), and the `.slint`-only examples, which upstream runs with `slint-viewer`.

No example showed a fork bug. Checked against upstream where a screen looked wrong:

- slide_puzzle's reset button is the text `↻`, which the default font lacks; upstream's build shows the same missing-glyph box, since the fork keeps upstream's font fallback.
- home-automation starts on its closed doors, a dark screen with a handle, as its `.slint` says.
- ffmpeg plays a video from a fixed URL, which now answers 403, so upstream's build stops there as well. Pointed at a local 30 fps test video instead (a copy of the example with that one string changed), it plays on the fork without dropping a frame: frame 169 at 5.633 s. Building it needs the FFmpeg 9.1 libraries (`FFMPEG_DIR`) and libclang (`LIBCLANG_PATH`).
- The gallery's About page draws `AboutSlint` (logo, "#MadeWithSlint", version, URL), which Royalty-free 2.0 asks for.

## What the Rewrite Does

- `slint` and `slint-build` paths into upstream's `api/` become crates.io requirements, `workspace = true` dependencies are inlined from the examples' workspace, and `[dev-dependencies]` go (some name upstream's internal crates).
- An optional `mcu-board-support` and the features naming it go: Cargo resolves optional path dependencies too, and that crate depends on upstream's `internal/`.
- `examples/`, `demos/`, `logo/` and `internal/common/sharedfontique` are mirrored side by side, so relative paths between them work (food-processor imports a font from the last).

## Why Not

- Building every example on upstream as well, to compare screens pixel by pixel: it would double the disk use (upstream's winit, FemtoVG and Skia) for a check that only matters where a screen looks wrong. Such a screen gets an upstream build of its own.
- Clicking through each example: the first screen exercises building, startup, layout and drawing, which is what the fork changed; interaction is covered by the fork's input tests and by slokk-slint ([app-compat](app-compat.md)).
