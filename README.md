# slint-wuc

Slint on Windows.UI.Composition (WUC): a fork of [Slint](https://slint.dev) 1.19 whose runtime
shows the user interface with Windows.UI.Composition on Win32, instead of upstream's backends and
renderers.

This is a derivative of Slint, not Slint itself. Report problems with it here, not to the Slint
project.

- Applications keep upstream's `.slint` files and Rust code. The `.slint` compiler is upstream's,
  unmodified: `slint-build` or the `slint!` macro, version 1.19.0. Only the runtime crate,
  `slint`, comes from this repository.
- Windows 11 (build 22621, version 22H2, or later) and Rust 1.95 or later.
- Rust only: there are no C++, JavaScript or Python bindings, no `slint-interpreter` and no live
  preview.

## Using it in an application

Use the fork's `slint` in place of upstream's, in either of two ways. Both fetch the same crate.

**As a git dependency**, when the application uses only the fork:

```toml
[dependencies]
slint = { git = "https://github.com/matarillo/slint-wuc", tag = "v1.19.0-wuc.1" }

[build-dependencies]
slint-build = "=1.19.0"
```

To go back to upstream, write `slint = "1.19"` again.

**As a patch**, to keep the dependency written for upstream and switch between the two:

```toml
[dependencies]
slint = "1.19"

[build-dependencies]
slint-build = "=1.19.0"

[patch.crates-io]
slint = { git = "https://github.com/matarillo/slint-wuc", tag = "v1.19.0-wuc.1" }
```

To go back to upstream, remove `[patch.crates-io]`.

- A patch works only in the top-level `Cargo.toml` of the application or workspace.
- With an existing `Cargo.lock` that names an older `slint`, adding the patch alone keeps the
  locked one. Changing `slint-build` to `"=1.19.0"` at the same time makes Cargo resolve again,
  and it picks the fork; `slint = "1.18"` may stay. `cargo tree -i slint` shows which one it is.
- When the version in `[dependencies]` doesn't accept 1.19.0 (`"1.20"`, say), Cargo only warns
  that the patch was not used, and builds with upstream's `slint`. Check that
  `cargo tree -i slint` shows the fork.

In both ways:

- Write `slint-build` (or `slint-macros`, if you depend on it directly) as `"=1.19.0"`. The code
  it generates names the compiler's version, and the runtime matches 1.19.0 only. `"1.19"`
  picks the newest 1.x, and the build fails once upstream releases 1.20.
- The tags name the upstream version the fork matches (`v1.19.0-wuc.N`). Pick a newer tag to
  update, or `rev = "<commit>"` for a commit between tags.
- To try a local clone, use `path = "<clone>/crates/slint"` in place of `git = ...`.

## Compared with upstream

- Of upstream 1.19.0's Rust examples and demos, 22 build, start and draw unchanged. The others
  use an API the fork removed (OpenGL textures, wgpu, `BackendSelector`, a platform of their
  own), need libraries that weren't installed (FFmpeg), or don't build on upstream either.
  The list is in `docs/design/upstream-examples.md`.
- Upstream's features that choose a backend, a renderer or a platform option (`backend-winit`,
  `renderer-skia`, `renderer-software`, ...) are accepted and do nothing. Upstream's features
  that bring an API of their own (`unstable-*`, `live-preview`, `system-testing`) are absent, so
  Cargo reports them.
- There is no backend or renderer selection, and an application can't install its own platform.
  `slint::platform` keeps only the types that the `Window` API uses.
- `Window::window_handle()` gives the HWND, with the `raw-window-handle-06` feature as upstream.
  Through it, an application sets what Slint has no API for, such as the Mica backdrop:

  ```rust
  use raw_window_handle::{HasWindowHandle, RawWindowHandle};
  use windows::Win32::Foundation::HWND;
  use windows::Win32::Graphics::Dwm::*;

  app.show()?; // The HWND exists once the window is shown.
  let handle = app.window().window_handle();
  if let RawWindowHandle::Win32(win32) = handle.window_handle()?.as_raw() {
      let backdrop = DWMSBT_MAINWINDOW;
      unsafe {
          DwmSetWindowAttribute(
              HWND(win32.hwnd.get() as _),
              DWMWA_SYSTEMBACKDROP_TYPE,
              (&backdrop as *const DWM_SYSTEMBACKDROP_TYPE).cast(),
              std::mem::size_of_val(&backdrop) as u32,
          )?;
      }
  }
  app.run()?;
  ```

  Mica shows through where the window's `background` is transparent. In a window with a
  `MenuBar`, the native menu bar keeps the client area white; with `SLINT_NO_MUDA` set, Slint
  draws the menu bar and Mica shows through it too.

  Upstream's winit backend creates the window only after an iteration of the event loop, so
  code that also runs on upstream sets the backdrop from a timer. `examples/backdrop` does, and
  switches between Mica, Mica Alt and Acrylic: `cargo run -p example-backdrop`. The same source
  builds on upstream as `upstream-backdrop` in `tools/upstream-gallery`.
- The fork's own feature `unstable-wuc` adds `slint::wuc`, which upstream doesn't have, so code
  that uses it doesn't build there. It extends a window's content into its title bar, as WinUI 3's
  `ExtendsContentIntoTitleBar` does: DWM keeps drawing the caption buttons, a `WindowMoveArea` is
  the drag region, and the application lays out the rest. The API may change in any release.
  `examples/titlebar` shows it over Mica: `cargo run -p example-titlebar`.
- Popup menus (context menus, the menu bar's drop-downs, the tray's menu) are dark when the color
  scheme is dark. Upstream's stay light.
- A `DropArea` takes files and text that other programs drag onto the window, such as files from
  Explorer, as upstream's Qt backend does. Upstream's winit backend takes none. Without a
  modifier key, the proposed action is copy, also as on Qt. Dragging out of the window to other
  programs isn't supported, as on upstream's winit backend.

## Costs

The fork isn't faster or lighter than upstream. Upstream's gallery, built on each, on one
machine (Ryzen 7 8745H, Radeon 780M, 125 %), medians of 7 runs (`tools/compare-numbers.ps1`; the
setup and what isn't measured are in `docs/design/costs.md`):

|                           | Fork   | Upstream, FemtoVG (default) | Upstream, software |
|---------------------------|--------|-----------------------------|--------------------|
| Window shown              | 56 ms  | 36 ms                       | 36 ms              |
| Answers messages          | 115 ms | 116 ms                      | 42 ms              |
| Private memory            | 47 MB  | 116 MB                      | 7 MB               |
| CPU while a spinner turns | 3.0 %  | 8.7 %                       | 2.8 %              |

Starting Direct3D, Direct2D, DirectWrite and Composition costs about 60 ms and 40 MB over the
software renderer. Upstream's default uses more memory here mostly for its OpenGL driver, which
varies with the GPU. What the fork gives for that cost is the Windows integration above.

A real application costs more than the gallery. In a contacts app with large images
(`docs/design/app-compat.md`), the fork's window shows 60 ms after upstream's, it holds 128 MB
of private memory (upstream 199–259 MB, software 35 MB), and an animation that rebuilds a `Path`
every frame takes 22 % of a core (upstream 10 %, software 21 %).

## Developing the fork

The tests, the probes and the tools that compare the fork with upstream read upstream's sources
from `reference/slint-1.19.0`, which isn't tracked. Get them first:

```sh
git clone --depth 1 --branch v1.19.0 https://github.com/slint-ui/slint reference/slint-1.19.0
```

Without them, these fail:

- the workspace build, in the build scripts of `tests/probe` (upstream's examples),
  `tests/driver` (upstream's `tests/cases`) and `tests/screenshots` (upstream's reference images).
  `cargo build -p slint` doesn't need them.
- the tests of `tools/cpp-headers`, and `tools/upstream-gallery`
- `tools/upstream-examples.ps1`, which builds and launches upstream's Rust examples and demos,
  unmodified, on the fork, and captures their windows

Build and test from PowerShell, where cargo finds the MSVC linker.

The plan and the design notes are in `docs/`, mostly in Japanese.

## License

Files from upstream keep SixtyFPS's copyright notices and upstream's licenses. The fork's own
changes are Copyright © Kentaro Inomata, under MIT-0, which asks for nothing more. So an
application uses the fork on upstream's terms:

- under the GPLv3, or
- under the Royalty-free License 2.0, for desktop, mobile and web applications that show they use
  Slint (the `AboutSlint` widget or the Slint badge). That this covers a modified version
  distributed by a third party is our reading, not confirmed by SixtyFPS.

Whether upstream's commercial license covers the fork is unknown. See [LICENSE.md](LICENSE.md)
for the details.
