# Gaps for Future Bindings

What the fork lacks for upstream's interpreter, C++, Node and Python bindings, as of Phase 7.
The fork has `crates/common`, `crates/core`, `crates/core-macros`, `crates/slint` and `crates/testing`.
Upstream's bindings also need `slint-interpreter`, `i-slint-backend-selector`, `i-slint-backend-testing`, the renderers and `i-slint-live-preview`, which the fork doesn't have.

## What Already Carries Over

- `i-slint-core` builds with `rtti` and `ffi`, and the conformance test checks every item's `item!` declaration against the compiler's native classes.
- `tools/cpp-headers` generates the C++ headers of the core C ABI with upstream's cbindgen setup.
  15 of its 18 headers match upstream's byte for byte.
  The others differ only in `update_visual`, which replaces `render` in the item vtable, and in the wasm, wgpu and Android variants that the fork removed.
  Its test compiles them with MSVC together with upstream's handwritten headers that don't need the missing headers.
- `i_slint_core::context::{with_global_context, with_existing_context}` are as upstream's.
- `crates/testing` has `init_no_event_loop`, `mock_elapsed_time`, `set_system_accent_color` and the test fonts.
- The Slint compiler is upstream's, unchanged, for Rust and C++ code generation.

## Interpreter

All three language bindings use `slint-interpreter`, so it comes first.

| It uses | From | In the fork |
| --- | --- | --- |
| `i-slint-core` with `rtti` and `path` | core | both features exist |
| `with_global_context`, `with_platform` | `i-slint-backend-selector` | `with_global_context` is in `i_slint_core::context`; `with_platform` needs a counterpart |
| `NativeWidgets`, `NativeGlobals`, `HAS_NATIVE_STYLE` | `i-slint-backend-selector` | no native style (Qt) in the fork: empty lists and `false` |
| `api::BackendSelector` | `i-slint-backend-selector` | one backend only; to drop or to accept only the Win32 host |
| `init_testing_backends`, `init_no_event_loop` | backend selector, `i-slint-backend-testing` | `init_no_event_loop` is in `crates/testing` |
| `TestingBackend`, `TestingWindow`, `access_testing_window` | `i-slint-backend-testing` (interpreter tests only) | missing, see [Testing Backend](#testing-backend) |
| `i-slint-renderer-software` | optional, for `internal-embed-textures` (MCU) | out of scope |

- Every `i_slint_core::` module path that the interpreter and the bindings import exists in the fork.
  Nobody has checked the symbols inside those modules yet.
- First step: build upstream's `slint-interpreter` against the fork with a `[patch]` and a small replacement for `i-slint-backend-selector`. The errors list the remaining gaps.
- Upstream's cases that need the interpreter: `elements/component_container*.slint` (7) and `text/componentcontainer_font_size_propagation.slint` (`tests/driver/exclusions.rs`).

## Testing Backend

Upstream's `i-slint-backend-testing` gives each test a `SlintContext` of its own through `TestingBackend`, a `slint::platform::Platform`.
The fork keeps `Platform` out of its public API (`compat-contract.md`), and `crates/testing` installs a testing platform globally instead.

- Cases that wait for it: `popups/popupwindow_own_context.slint`, `elements/timer_own_context.slint`, `elements/window_shared_adapter_context.slint`, `elements/systemtray_context.slint` (records `open_url`) and `expr/debug_context.slint` (collects `debug_logs`).
- Node uses `testing_backend`, `configure_test_fonts`, `get_mocked_time`, `init_integration_test_with_mock_time` and `mock_elapsed_time`.
  Python uses `configure_test_fonts`.
- Adding it means exposing a testing `Platform` from `crates/testing` without making `Platform` public in `crates/slint`.

## C++

Upstream's `api/cpp` is a Rust static library with C functions behind the handwritten headers, plus CMake packaging (Corrosion).
The core functions that cbindgen exports are in the fork. These `api/cpp` functions aren't:

- Windows and the event loop: `slint_windowrc_init`, `slint_ensure_backend`, `slint_select_backend`, `slint_post_event`, `slint_quit_event_loop`, `slint_windowrc_has_active_animations`.
  They map onto the Win32 host.
- Fonts: `slint_register_font_from_path`, `slint_register_font_from_data`, `slint_register_bitmap_font`.
- Other: `slint_context_color_scheme`, `slint_string_to_float`, `slint_string_to_usize`, `slint_debug`, `slint_parse_markdown`, `slint_translate_set_translator`, and the platform-specific `slint_set_xdg_app_id` and `slint_macos_bring_all_windows_to_front`.
- `slint-platform.h` (`slint_platform_register`, `slint_window_adapter_new`, the window property getters, the software and Skia renderers, raw window handles): it lets C++ code provide a platform, which the fork doesn't offer. Out of scope, like `slint::platform::Platform` in Rust.
- `slint-interpreter.h` needs the interpreter, and `slint-testing.h` needs the testing backend.

## Node

Upstream's `api/node` is a napi-rs module over the interpreter.

- It merges Slint's event loop into libuv by calling `Platform::process_events` with a timeout (`uv_event_loop/windows.rs`).
  The Win32 host doesn't implement `process_events`, so it returns `NoEventLoopProvider` there.
  The host's message loop needs a variant that runs until a timeout or until no messages are pending.
- It also uses `invoke_from_event_loop` and `run_event_loop`, which the fork has, and the testing functions above.
- `i-slint-live-preview` is optional.

## Python

Upstream's `api/python/slint` is a PyO3 module over the interpreter.

- It uses `run_event_loop` and `invoke_from_event_loop`, which the fork has, and `configure_test_fonts`.
- Its event loop integration hasn't been checked against the host beyond these names.

## Suggested Order

1. The interpreter against the fork, with a replacement for the backend selector.
2. The testing backend. It unlocks the interpreter's own tests and the 5 excluded cases.
3. `process_events` in the host, for Node.
4. The C++ static library functions and CMake packaging. Python and Node packaging follow upstream's.
