# Symbol inventory for the i-slint-core fork

This document lists every symbol of `i-slint-core` that the unmodified upstream compiler output and the interpreter touch.
It was produced by a read-only survey of Slint 1.19.0 in `reference/slint-1.19.0`.
All paths and line numbers are relative to that directory unless a path starts with `docs/` or `crates/`.
The data comes from grep over the source and from small throwaway scripts; nothing was built or run.
Where a fact is ambiguous the text says so instead of guessing.

## Reading guide

- `sp` is the alias `slint::private_unstable_api::re_exports`, defined in `api/rs/slint/private_unstable_api.rs:181-249`.
- "Core" means the crate `i_slint_core` in `internal/core`.
- "Generator" means `internal/compiler/generator/rust.rs` unless another file is named.
- A table row "resolves to" the real definition after following the re-exports of `private_unstable_api.rs` and `api/rs/slint/lib.rs`.
- The "Use" column says whether a symbol is used as a type, a fn call, a trait method call, a struct field access, a macro or a trait impl.
- Line numbers inside `internal/core` refer to the 1.19.0 files and will move once the fork edits them.
- A bare file name such as `items.rs`, `window.rs` or `lib.rs`, or a path such as `items/text.rs`, is a file of `internal/core`, except in the places named next.
- `rust.rs`, `cpp.rs`, `rust_live_preview.rs` and paths starting with `generator/` are in `internal/compiler/generator`; `passes/` and `llr/` are in `internal/compiler`.
- In sections 3 and 5, `cbindgen.rs` is `api/cpp/cbindgen.rs` and the `lib.rs` of a `slint` item is `api/rs/slint/lib.rs`; in section 4 and in table 6b a bare file name is in `internal/interpreter`.
- A path that starts with `api/`, `internal/`, `helper_crates/` or `docs/` is relative to the Slint root (or to the fork root for `docs/`).

## 1. Rust generated code

### 1.1 How the generated code gets its names

- Every generated module starts with `use slint::private_unstable_api::re_exports as sp;` (`generator/rust.rs:392`).
- It then imports the traits `RepeatedItemTree`, `ModelExt`, `Model` and `ItemTree` anonymously through `sp` (`generator/rust.rs:393`).
- Each item tree also runs `use slint::private_unstable_api::re_exports::*;` before `ItemTreeVTable_static!(static VT for self::Inner);` (`generator/rust.rs:2933-2934`).
- That glob is required: the macro expands to `ItemTreeVTable::new::<T>()` and `unsafe impl vtable::HasStaticVTable<ItemTreeVTable>`, so `ItemTreeVTable` and `vtable` must be visible by bare name (`helper_crates/vtable/macro/macro.rs:743-753`).
- `re_exports` is a flat namespace built from explicit `pub use` lines and from six globs.
- The globs are `i_slint_core::graphics::*`, `i_slint_core::items::{Transform, *}`, `i_slint_core::layout::*`, `i_slint_core::model::*`, `i_slint_core::date_time::*` and `vtable::{self, *}` (`private_unstable_api.rs:205,207,220,221,227,245`).
- A seventh glob, `i_slint_backend_selector::native_widgets::*` (`private_unstable_api.rs:194`), carries the Qt-only `Native*` items and is empty without Qt.
- `vtable::*` itself re-exports `const_field_offset::*` and `vtable_macro::*` (`helper_crates/vtable/src/lib.rs:63-71`), which is where `AllowPin`, `compose_field_offsets`, `VRc`, `VRcMapped`, `VWeak`, `VWeakMapped` and `VOffset` come from.
- `sp::Transform` is the item `items::Transform` because of the explicit `{Transform, *}` import; `graphics::Transform` is a type alias with the same name.
- `sp::ApproxEq` is `euclid::approxeq::ApproxEq` (explicit import at `private_unstable_api.rs:192`), not the core trait of the same name in `graphics/border_radius.rs:247`.
- `sp::ItemRc` is not in an explicit import: it resolves only because `items.rs:33` has `pub use crate::item_tree::{ItemRc, ItemTreeVTable}` and `items::*` is globbed.
- The compiler feature `renderer-software` adds embedded textures and bitmap fonts to the output (`sp::StaticTextures`, `sp::BitmapFont`); it is forwarded by the `renderer-software` feature of `slint-build` only (`api/rs/build/Cargo.toml:32`) and is absent from `slint-macros` (`api/rs/macros/Cargo.toml:25`).
- The compiler feature `bundle-translations` adds `sp::TranslationsBundled`; `slint-build` always enables it (`api/rs/build/Cargo.toml:48`) but the output appears only when the `.slint` build asks for bundled translations.

### 1.2 `rust_live_preview.rs`

- `generator/rust_live_preview.rs` is compiled whenever the compiler feature `rust` is on (`generator.rs:31-32`); it has no feature gate of its own.
- It is selected at compiler run time, not at compile time: `generate()` jumps to it when the environment variable `SLINT_LIVE_PREVIEW` is set (`generator/rust.rs:275-277`).
- Its output needs `sp::live_preview`, which exists only when the `slint` crate is built with the `live-preview` feature (`private_unstable_api.rs:247-248`).
- Without that feature a build with `SLINT_LIVE_PREVIEW=1` fails to compile, so the file is relevant only for live-preview users.
- It emits no `sp::` name that `rust.rs` does not already emit, apart from `sp::live_preview::{Compiler, LiveReloadingComponent, ComponentInstance, DefaultTranslationContext, Value}` and `sp::Err`.
- It also calls `sp::WindowInner::from_pub(slint::ComponentHandle::window(..)).window_adapter()` and stores `sp::Rc<dyn sp::WindowAdapter>` (`rust_live_preview.rs:206,212,220,232`).
- It emits `slint::run_event_loop()` (`rust_live_preview.rs:248`), which is not in `rust.rs`.
- Everything below describes `rust.rs`; `rust_live_preview.rs` is covered only by the notes above.

### 1.3 Non-`sp::` paths emitted by the generator

| Path as emitted | Resolves to | Use |
|---|---|---|
| `slint::PlatformError` | `i_slint_core::api::PlatformError` (via `pub use i_slint_core::api::*`, `api/rs/slint/lib.rs:226`) | type in 13 `Result<_, PlatformError>` signatures |
| `slint::Window` | `i_slint_core::api::Window` (`api.rs:452`) | type of `window()` return and `new_with_existing_window(window: &slint::Window)` |
| `slint::ComponentHandle` | `api::ComponentHandle` trait (`api.rs:1089`) | `impl slint::ComponentHandle for Public { as_weak, clone_strong, global, run, show, hide, window }` |
| `slint::StrongHandle` | `api::StrongHandle` trait (`api.rs:1072`) | `impl` with `type WeakInner` and `upgrade_from_weak_inner` |
| `slint::Global<'a, Root>` | `api::Global` trait (`api.rs:943`) | `impl` with `type StaticSelf`, `get`, `as_weak` |
| `slint::Weak`, `slint::Weak::new` | `api::weak_handle::Weak` (`api.rs:1269`) | return type and constructor of `as_weak()` |
| `slint::Brush` | `graphics::Brush` (via `api::*`) | type; variants `SolidColor`, `LinearGradient`, `RadialGradient`, `ConicGradient` |
| `slint::DataTransfer::default` | `data_transfer::DataTransfer` (via `api::*`) | fn call |
| `slint::ComponentFactory::default` | `component_factory::ComponentFactory` (deprecated re-export, `api/rs/slint/lib.rs:227-229`) | fn call |
| `slint::LogicalPosition`, `slint::LogicalSize` | `api::LogicalPosition`, `api::LogicalSize` | types of builtin struct values (`rust.rs:5466`) |
| `slint::language::<Name>` | `i_slint_core::items::<Name>` through the macro in `api/rs/slint/lib.rs:507-519` | public builtin structs: `KeyboardModifiers`, `PointerEvent`, `PointerScrollEvent`, `KeyEvent`, `DropEvent`, `StandardListViewItem`, `TableColumn`, `InputMethodHints` |
| `slint::VersionCheck_1_19_0` | struct defined in `api/rs/slint/lib.rs:548` | `const _THE_SAME_VERSION_MUST_BE_USED_FOR_THE_COMPILER_AND_THE_RUNTIME: slint::VersionCheck_1_19_0 = slint::VersionCheck_1_19_0;` (`rust.rs:419`) |
| `slint::private_unstable_api::ensure_context` | fn in `private_unstable_api.rs:108` | fn call (`rust.rs:526,624`) |
| `slint::private_unstable_api::new_item_tree` | fn in `private_unstable_api.rs:114` | fn call (`rust.rs:2844`) |
| `slint::private_unstable_api::no_dynamic_children` | fn in `private_unstable_api.rs:127` | fn item passed as `visit_dynamic` (`rust.rs:2758`) |
| `slint::private_unstable_api::get_item_ref` | fn in `private_unstable_api.rs:139` | fn call (`rust.rs:2856`) |
| `slint::private_unstable_api::translate` | fn in `private_unstable_api.rs:155` | fn call (`rust.rs:5413`) |
| `slint::private_unstable_api::debug` | fn in `private_unstable_api.rs:99` | fn call (`rust.rs:5202`) |
| `slint::private_unstable_api::build_array_chunk` | fn in `private_unstable_api.rs:176` | fn call (`rust.rs:4464`) |
| `slint::private_unstable_api::{set_property_binding, set_animated_property_binding, set_property_state_binding, set_callback_handler}` | fns in `private_unstable_api.rs:37-84` | fn calls for global components; chosen by `helper()` at `rust.rs:1031-1037` |
| `slint::private_unstable_api::init_translations` | fn in `private_unstable_api.rs:168` | called only from the `slint::init_translations!` macro, not by the generator |

- The helper functions in `private_unstable_api.rs` are not part of `i_slint_core`, but they call into it: `SlintContext::dispatch_log_message`, `debug_log::{LogMessage, LogMessageSource}`, `SlintContext::translate`, `Property::{set_binding, set_animated_binding}`, `Callback::set_handler`, `register_item_tree`, `ItemTreeNode`, `VOffset::apply_pin`.
- `ensure_context` calls `i_slint_backend_selector::with_global_context`, so the fork needs its own `slint` facade crate with this function.
- The generator also emits `pub use slint::{ComponentHandle as _, Global as _, ModelExt as _};` (`rust.rs:380-381`).
- `slint::Brush`, `slint::Image` and similar type names are the public API aliases of core types; they must stay identical to the `sp::` ones because values flow between them without conversion.

### 1.4 `sp::` symbols emitted by name

The `Use` column states how the generated code uses the symbol.
Rows are grouped by the core module they resolve to.

#### Rust standard library and external crates

| `sp::` path | Resolves to | Use |
|---|---|---|
| `Rc`, `Rc::new`, `Rc::clone`, `Rc::downgrade`, `Rc::pin` | `alloc::rc::Rc` | type and fn calls |
| `Weak` | `alloc::rc::Weak` | type of weak global references |
| `Box`, `Box::new` | `alloc::boxed::Box` | boxed closures for `show_popup` |
| `Vec`, `Vec::with_capacity` | `alloc::vec::Vec` | layout cell buffers |
| `vec!` | `alloc::vec` macro | array literals |
| `Option`, `Some`, `None`, `Option::Some`, `Option::None` | `core::option` | type and variants |
| `Result`, `Ok`, `Err` | `core::result` | type and variants |
| `OnceCell` | `once_cell::unsync::OnceCell` | field type (`self_weak`, `globals`, `window_adapter`) |
| `ApproxEq::<f64>::approx_eq` | `euclid::approxeq::ApproxEq` | trait method call for `==` on numbers |
| `euclid::rect` | `i_slint_core::graphics::euclid` (re-export of the `euclid` crate) | fn call in `item_geometry` |
| `FieldOffsets` | derive macro from `const_field_offset` | `#[derive(sp::FieldOffsets, Default)]` (`rust.rs:2210,2467`) |
| `const_field_offset` | the crate `const_field_offset`, via `private_unstable_api.rs:186` | `#[const_field_offset(sp::const_field_offset)]` (`rust.rs:2211,2468`) |
| `PinnedDrop` | `const_field_offset::PinnedDrop` | `impl sp::PinnedDrop for Inner` (`rust.rs:2725`) |
| `compose_field_offsets` | `const_field_offset::compose_field_offsets` (const fn) | fn call in constant property tables (`rust.rs:1880,6102`) |
| `VRc`, `VRcMapped`, `VWeak`, `VWeakMapped`, `VOffset`, `AllowPin`, `vtable` | `vtable` crate (`helper_crates/vtable/src/{lib,vrc}.rs`) | types, and the macro `sp::vtable::new_vref!` (`rust.rs:2727`) |

- The `vtable` and `const-field-offset` crates are not part of the fork's core; the plan keeps them as crates.io versions 0.5.1 and 0.2.1.
- `VRc` functions used: `new`, `into_dyn`, `map`, `downgrade`, `clone`, `as_pin_ref`, `borrow`; `VRcMapped` functions used: `origin`, `downgrade`, `map`; `VWeak::upgrade`.

#### Crate root of `i_slint_core` (`lib.rs`)

| `sp::` path | Resolves to | Use |
|---|---|---|
| `SharedString`, `SharedString::from`, `::default` | `string::SharedString` (re-exported at `lib.rs:76`) | type and fn calls |
| `SharedVector`, `SharedVector::default`, `::from_slice` | `sharedvector::SharedVector` (`lib.rs:79`) | type and fn calls |
| `Color`, `Color::from_argb_encoded` | `graphics::Color` (`lib.rs:91`) | type and fn call |
| `Coord` | `lib.rs:113-117` type alias (`f32`, or `i32` with `slint_int_coord`) | type |
| `SlintContext` | `context::SlintContext` (re-exported at `lib.rs:109`; struct at `context.rs:187`) | type; methods in 1.5 |
| `ItemTreeVTable_static` | `#[macro_export]` macro created by `#[vtable]` on `item_tree::ItemTreeVTable` (`item_tree.rs:46-48`) | macro call with the glob import |
| `macos_bring_all_windows_to_front` | `lib.rs:218-227` fn | fn call; no-op off macOS |

#### `animations`, `properties`, `callbacks`, `timers`

| `sp::` path | Resolves to | Use |
|---|---|---|
| `animation_tick` | `animations::animation_tick() -> u64` (`animations.rs:341`) | fn call for `animation-tick()` |
| `EasingCurve` and variants `Linear`, `CubicBezier([f32;4])`, `Spring(f32)`, `EaseInElastic`, `EaseOutElastic`, `EaseInOutElastic`, `EaseInBounce`, `EaseOutBounce`, `EaseInOutBounce` | `animations::EasingCurve` (`animations.rs:152`) | enum value; variant names come from the compiler's own enum (`expression_tree.rs`) through `format_ident!("{e:?}")` |
| `Property<T>` | `properties::Property` (`properties.rs:902`) | field type; see 1.5 for methods |
| `ChangeTracker`, `change_tracker_init_erased` | `properties::ChangeTracker`, `properties/erased_bindings.rs:116` | field type; fn call; `.init(..)` on the field for globals (`rust.rs:2397`) |
| `ErasedPropertyOffset`, `ErasedPropertyOffset::new`, `set_constant_properties` | `properties.rs:1148,1163` | const array of offsets and fn call |
| `set_property_binding_erased`, `set_animated_property_binding_erased`, `set_property_state_binding_erased`, `set_callback_handler_erased` | `properties/erased_bindings.rs:26,48,95,153` | fn calls; names are built at run time as `format_ident!("{name}_erased")` (`rust.rs:1031-1033`), so grep does not find them |
| `Callback<Args, Ret>` | `callbacks::Callback` (`callbacks.rs:21`) | field type; methods `call`, `set_handler` |
| `Timer`, `TimerMode::Repeated` | `timers::Timer`, `timers::TimerMode` (`timers.rs:62,31`) | field type and variant |

#### `item_tree`, `items`, `input`, `cursor`, `accessibility`

| `sp::` path | Resolves to | Use |
|---|---|---|
| `ItemTree` | trait generated by `#[vtable]` on `item_tree::ItemTreeVTable` (`item_tree.rs:46`) | `impl sp::ItemTree for Inner { .. }` (`rust.rs:2172`); 20 trait methods, see 1.6 |
| `ItemTreeVTable` | `item_tree::ItemTreeVTable` (`item_tree.rs:48`) | type argument of `VRc`, `VRcMapped`, `VWeak`, `VWeakMapped` (14 uses) |
| `ItemTreeRc`, `ItemTreeWeak` | `item_tree.rs:202,204` aliases of `VRc/VWeak<ItemTreeVTable, Dyn>` | types |
| `ItemTreeNode`, `ItemTreeNode::Item`, `::DynamicTree` | `item_tree::ItemTreeNode` (`item_tree.rs:1284`) | enum values in a `const` array; fields `is_accessible`, `children_count`, `children_index`, `parent_index`, `item_array_index`, `index` |
| `ItemWeak`, `ItemRc` | `item_tree::ItemWeak` (`:1209`), `item_tree::ItemRc` (`:335`, reached through `items::*`) | types; `ItemRc::new(ItemTreeRc, u32)`, `ItemRc::new_root(ItemTreeRc)`, `.downgrade()` |
| `IndexRange`, `IndexRange::from` | `item_tree::IndexRange` (`:26`) | type and fn call |
| `TraversalOrder`, `VisitChildrenResult`, `ItemVisitorRefMut` | `item_tree.rs:1230,1245,1438` | types in `visit_children_item` and `visit_dynamic_children` signatures |
| `visit_item_tree`, `visit_item_tree_z_sorted` | `item_tree.rs:1546,1603` | fn calls with a closure for dynamic children |
| `register_item_tree` | `item_tree.rs:215` | fn call `(&ItemTreeRc, Option<WindowAdapterRc>)` (`rust.rs:2834`) |
| `unregister_item_tree` | `item_tree.rs:235` | fn call `(Pin<&Base>, ItemTreeRef, &[VOffset<..>], &WindowAdapterRc)` (`rust.rs:2729`) |
| `Item` | trait generated by `#[vtable]` on `items::ItemVTable` (`items.rs:127-130`) | one call: `sp::Item::layout_info(item, orientation, constraint, &window_adapter, &item_rc)` (`rust.rs:5147`) |
| `ItemVTable`, `ItemRef` | `items::ItemVTable` (`items.rs:130`), `items::ItemRef` (`:226`) | types in `item_array()` and `get_item_ref` |
| `<NativeItem>` | one struct per `item!` in `builtin_elements.rs`, in `items.rs` and `items/*.rs` | field types and `FIELD_OFFSETS`; see section 2 |
| `WindowItem::resolved_default_font_size` | `items.rs:1422` associated fn | fn call returning `LogicalLength` (`rust.rs:5191`) |
| `FocusReason::Programmatic` | `items::FocusReason` | enum value for `set_focus_item` |
| `PopupClosePolicy::CloseOnClickOutside` | `items::PopupClosePolicy` | enum value for menus (`rust.rs:5015`) |
| `KeyboardModifiers::default` | `items::KeyboardModifiers` (builtin struct) | fn call in `make_keys` expansion |
| `Keys`, `make_keys` | `input::Keys` (`input.rs:612`), `input::make_keys` (`:665`) | type and fn call for key literals |
| `MouseCursorInner`, `::BuiltIn`, `::CustomMouseCursor` | `cursor::MouseCursorInner` (`cursor.rs:12`) | enum values |
| `AccessibleRole`, `AccessibleRole::default` | `items::AccessibleRole` | return type |
| `AccessibilityAction`, `AccessibleStringProperty`, `SupportedAccessibilityAction` | `accessibility.rs:52,19,69` | types and variants in the accessibility methods of the `ItemTree` impl |
| `Orientation`, `Orientation::{Horizontal, Vertical}` | `items::Orientation` (also `layout::Orientation`) | type and variants |
| `CrossAxisAlignment` | `items::CrossAxisAlignment` | type of `cross_axis_self_alignment_for_repeated` |
| `Menu`, `Menu::sub_menu`, `Menu::activate` | trait generated by `#[vtable]` on `menus::MenuVTable` | trait method calls on `&*VRc<MenuFromItemTree>` |
| `MenuFromItemTree::new`, `::new_with_condition_and_visible` | `menus.rs:48,61` | fn calls |
| `DataTransfer` | `data_transfer::DataTransfer` | type of a builtin value |

#### `layout` and `lengths`

| `sp::` path | Resolves to | Use |
|---|---|---|
| `LayoutInfo` | `layout::LayoutInfo` (`layout.rs:24`) | struct return type and struct literal |
| `LayoutItemInfo`, `LayoutItemInfo::default` | `layout.rs:1182` | struct literal with `..Default::default()` (12 uses) |
| `FlexboxLayoutItemInfo` | `layout.rs:1215` | return type |
| `GridLayoutInputData` | `layout.rs:481` | slice element type (4 uses) |
| `solve_box_layout`, `box_layout_info_ortho` | `layout.rs:1233,1427` | fn calls with measure cells |
| `solve_flexbox_layout_with_measure`, `flexbox_layout_info_cross_axis_with_measure` | `layout.rs:2010,2312` | fn calls taking `Some(&mut measure)` |
| `LogicalLength`, `LogicalLength::new` | `lengths.rs:13` | type and fn call |
| `LogicalRect` | `lengths.rs:14` | return type of `item_geometry` |

- More `layout` functions are emitted by dynamic name through `ExtraBuiltinFunctionCall` (`rust.rs:4279`): `grid_layout_info`, `box_layout_info`, `box_layout_info_ortho`, `organize_dialog_button_layout`, `organize_grid_layout`, `solve_grid_layout`, `solve_box_layout`, `solve_box_layout_ortho`, `solve_flexbox_layout`, `flexbox_layout_info_cross_axis`, `flexbox_layout_info_main_axis`, `flexbox_layout_unwrapped_main` (`llr/lower_layout_expression.rs`).

#### `model`

| `sp::` path | Resolves to | Use |
|---|---|---|
| `ModelRc`, `ModelRc::new` | `model::ModelRc` (`model.rs:880`) | type and fn call |
| `VecModel`, `VecModel::from_slice` | `model::VecModel` (`model.rs:507`) | type; `From<Vec<T>>` |
| `SharedVectorModel::from` | `model.rs:658` | fn call |
| `ModelError::out_of_bounds` | `model.rs:31` | fn call |
| `report_model_error` | `model.rs:392` | fn call `(&SlintContext, &str, None, Result<(), ModelError>)` |
| `model_any`, `model_all`, `model_find_index` | `model.rs:407,413,423` | fn calls |
| `Repeater<C>`, `Conditional<C>` | `model/repeater.rs:807,1242` | field types |
| `RepeatedItemTree` | trait in `model/repeater.rs:45` | `impl sp::RepeatedItemTree for Inner` (`rust.rs:3348`); items `update`, `init`, `z_order`, `listview_layout`, `layout_item_info`, `layout_item_info_at_cross_width`, `flexbox_layout_item_info`, `flexbox_layout_item_info_at_cross_width`, `grid_layout_input_data` |
| `Model`, `ModelExt` | traits in `model.rs` | in scope for `row_data_tracked`, `set_row_data`, `push_row`, `insert_row`, `remove_row`, `row_count`, `model_tracker().track_row_count_changes()` |

#### `graphics`, `styled_text`, `date_time`, `debug_info`, `slice`, `translations`

| `sp::` path | Resolves to | Use |
|---|---|---|
| `Image`, `Image::default`, `::from`, `::load_from_path` | `graphics/image.rs:1002` | type and fn calls; `set_nine_slice_edges` is called on the value (`rust.rs:4415`) |
| `load_image_from_embedded_data` | `graphics/image.rs:1501` | fn call with `Slice<'static, u8>` and a format `Slice<u8>` |
| `load_as_html_image` | `graphics/image.rs:1489` | wasm32 only |
| `ImageInner::StaticTextures`, `StaticTextures`, `StaticTexture`, `TexturePixelFormat::<fmt>`, `IntSize::new` | `graphics/image.rs:581,387-390`, `graphics.rs:29` | only with the compiler feature `renderer-software` |
| `BitmapFont`, `BitmapGlyph`, `BitmapGlyphs` | `graphics/bitmapfont.rs:26,18,22` | only with the compiler feature `renderer-software` |
| `RgbaColor<u8>`, `HsvaColor`, `OklchColor` | `graphics/color.rs:22,449,548` | type of `Color::to-*()` structs |
| `GradientStop`, `LinearGradientBrush::new`, `RadialGradientBrush::{new_circle, new_ellipse}`, `ConicGradientBrush::new` | `graphics/brush.rs` | fn calls; also `.with_radius`, `.with_radii`, `.with_center` |
| `PathData::{Elements, Events, Commands}`, `PathElement::Close` | `graphics/path.rs:350,125` | enum values; needs the `path` core feature |
| `StyledText`, `parse_markdown` | `styled_text.rs:9,230` | type and fn call |
| `parse_date`, `date_now` | `date_time.rs:58,65` | fn calls |
| `debug_info::{format_bool, format_brush, format_color, format_float, format_integer}` | `debug_info.rs` | fn calls in `element_testable_property_value` |
| `Slice`, `Slice::from_slice`, `Slice::as_slice` | `slice.rs:38` | type and fn calls (22 uses of `from_slice`) |
| `TranslationsBundled` | `translations.rs:318` | struct literal; only with compiler feature `bundle-translations` |
| `builtin_functions::<runtime name>`, `builtin_functions::ItemArg` | `builtin_functions.rs` | 79 fns declared by `i_slint_common::for_each_builtin_function!`; see 1.7 |

#### `window`

| `sp::` path | Resolves to | Use |
|---|---|---|
| `WindowAdapter` | `window::WindowAdapter` trait (`window.rs:86`) | `dyn sp::WindowAdapter` in `Rc<dyn ..>` (4 uses) |
| `WindowAdapterRc` | `window.rs:2805` alias of `Rc<dyn WindowAdapter>` | field type of `SharedGlobals.window_adapter` |
| `WindowInner`, `WindowInner::from_pub` | `window::WindowInner` (`window.rs:684`), `from_pub` at `:2769` | 21 uses; the entry point to every window operation, see 1.5 |
| `WindowKind::{Popup, ToolTip, Menu}` | `window.rs:51` | enum values for `show_popup` |
| `SlintContext` | see crate root | type of `SharedGlobals.context` and `new_with_context(ctx)` |

### 1.5 Methods called on the important values

#### On `Rc<dyn WindowAdapter>` (`WindowAdapter`)

| Method | Seen at | Note |
|---|---|---|
| `window() -> &Window` | about 25 sites, for example `rust.rs:501,572,576,580,683,734,799,807,4799,4899,5012` | the only plain `WindowAdapter` method besides `renderer()` that the generated code needs |
| `renderer() -> &dyn Renderer` then `register_font_from_path(&Path)` | `rust.rs:5160` | returns `Result<(), Box<dyn Error>>`; the code calls `.unwrap()` |
| `renderer()` then `register_font_from_memory(&'static [u8])` | `rust.rs:5170` | same |
| `renderer()` then `register_bitmap_font(&'static BitmapFont)` | `rust.rs:5180` | returns `()` |
| `internal(InternalToken)` | not called by generated code | called by core on its behalf, see section 6 |

#### On `WindowInner` (reached with `WindowInner::from_pub(&Window)`)

| Method | Signature | Seen at |
|---|---|---|
| `from_pub` | `(&Window) -> &WindowInner`, a field projection of `pub struct Window(pub(crate) WindowInner)` | 21 sites |
| `window_adapter` | `(&self) -> Rc<dyn WindowAdapter>` | `rust.rs:732` and in `rust_live_preview.rs` |
| `context` | `(&self) -> &SlintContext` | `rust.rs:527,566` |
| `set_component` | `(&self, &ItemTreeRc)` | `rust.rs:734,807` |
| `ensure_tree_instantiated` | `(&self)` | `rust.rs:501` |
| `set_const_scale_factor` | `(&self, f32)` | `rust.rs:683` |
| `set_focus_item` | `(&self, &ItemRc, bool, FocusReason)` | `rust.rs:4799,4810` |
| `show_popup` | `(&self, &ItemTreeRc, Box<dyn Fn() -> LogicalPosition>, PopupClosePolicy, &ItemRc, WindowKind, Box<dyn Fn(bool)>) -> NonZeroU32` | `rust.rs:4914,5012` |
| `close_popup` | `(&self, NonZeroU32)` | `rust.rs:4905,4962,5000` |
| `show_native_popup_menu` | `(&self, VRc<MenuVTable>, LogicalPosition, &ItemRc) -> bool` | `rust.rs:5073` |
| `create_child_window_adapter` | `(&self, WindowKind) -> Option<Rc<dyn WindowAdapter>>` | `rust.rs:4748` |
| `supports_native_menu_bar` | `(&self) -> bool` | `rust.rs:5263,5314` |
| `setup_menubar`, `setup_menubar_shortcuts` | `(&self, VRc<MenuVTable>)` | `rust.rs:5316,5346` |
| `text_input_focused`, `set_text_input_focused` | `(&self) -> bool`, `(&self, bool)` | `rust.rs:5405,5409` |

#### On `slint::Window` (`api::Window`)

| Method | Seen at |
|---|---|
| `show() -> Result<(), PlatformError>` | `rust.rs:572` |
| `hide() -> Result<(), PlatformError>` | `rust.rs:576` |
| `scale_factor() -> f32` | `rust.rs:799` |

#### On `SlintContext`

| Method | Seen at |
|---|---|
| `create_window_adapter() -> Result<Rc<dyn WindowAdapter>, PlatformError>` | `rust.rs:805` |
| `run_event_loop() -> Result<(), PlatformError>` | `rust.rs:566` |
| `start_timer(&Timer, TimerMode, Duration, impl FnMut())` | `rust.rs:2006` |
| `format_number(f64) -> SharedString` | `rust.rs:4128` |
| `translate_from_bundle(&[Option<S>], &impl FormatArgs)` | `rust.rs:4726` |
| `translate_from_bundle_with_plural(..)` | `rust.rs:4718` |
| `set_static_bundled_languages(&'static [TranslationsBundled])` | `rust.rs:510` |
| `color_scheme(Option<&ItemTreeRc>) -> ColorScheme` | `rust.rs:5258` |
| `clone()` | `rust.rs:527,2910` |
| passed as `&SlintContext` to the runtime builtins, to `report_model_error`, to `private_unstable_api::{debug, translate}` | `rust.rs:4770-4774`, `5218,5231,5247`, `5202`, `5413` |
| `dispatch_log_message`, `translate` | called by the helper fns in `private_unstable_api.rs:99-166` |

- The builtin bodies in `builtin_functions.rs` also call `format_number_fixed`, `format_number_precision`, `parse_number`, `accent_color`, `motion_preference`, `locale_decimal_separator` on the context; they are reached through the generated `sp::builtin_functions::<name>(&ctx, ..)` calls.

#### On `ItemRc`, `ItemTreeVTable` and `VRc<ItemTreeVTable>`

| Use | Seen at |
|---|---|
| `ItemRc::new(origin_rc, index)`, `ItemRc::new_root(origin)`, `.downgrade()` | `rust.rs:2598,2612,3731` |
| `&ItemRc` passed to `set_focus_item`, `show_popup`, `show_native_popup_menu`, `listview_ensure_row_visible`, `Item::layout_info` | `rust.rs:4799,4914,5012,5133,5147` |
| `ItemRc` and `Pin<&NativeItem>` packed into `sp::builtin_functions::ItemArg { item, rc }` and converted with `.into()` | `rust.rs:4789` |
| `VRc::borrow(&menu).sub_menu(..)` and `.activate(..)` (trait `Menu`) | `rust.rs:5327-5342` |
| `ItemTree::window_adapter(true, &mut result)` is called by the core builtins, not by generated code | `internal/core/builtin_functions.rs:102-106` |
| `Pin<&Inner>` methods `ensure_instantiated`, `get_subtree_range`, `get_subtree`, `layout_info`, `item_geometry`, `accessible_role`, `accessible_string_property`, `accessibility_action`, `supported_accessibility_actions`, `visit_dynamic_children` on sub-components | `rust.rs:1750-1830` |

#### On properties, callbacks and repeaters

- `Property`: `get`, `set`, `set_animated_value(value, PropertyAnimation)`, `set_binding`, `mark_dirty`, `set_constant`, `link_two_way`, `link_two_way_with_map`, `link_two_way_to_model_data`.
- `Callback`: `call(&args)`, `set_handler`.
- `Timer`: `running()`, `interval()`, `stop()`, `restart()`.
- `Repeater` and `Conditional`: `set_model_binding`, `track_instance_changes`, `ensure_updated`, `visit`, `range`, `len`, `instance_at`, `for_each_instance_z`; `Repeater` only: `track_changes_listview`, `ensure_updated_listview`, `model_set_row_data`, `listview_ensure_row_visible`.
- Direct struct fields of native items: `ContextMenu::popup_id` (`Cell<Option<NonZeroU32>>`), `ContextMenu::sub_menu`, `ContextMenu::activated`, `SystemTrayIcon::visible`; the call `SystemTrayIcon::set_menu(&ItemRc, VRc<MenuVTable>)` (`rust.rs:5393`).

### 1.6 Trait contracts the generated code implements

- `impl sp::ItemTree for Inner` provides every slot of `ItemTreeVTable` except the two drop slots: `visit_children_item`, `get_item_ref`, `get_subtree_range`, `get_subtree`, `get_item_tree`, `parent_node`, `embed_component`, `subtree_index`, `layout_info`, `ensure_instantiated`, `item_geometry`, `accessible_role`, `accessible_string_property`, `accessibility_action`, `supported_accessibility_actions`, `item_element_infos`, `element_testable_properties`, `element_testable_property_value`, `window_adapter`, `slint_context`.
- The vtable struct is `item_tree.rs:46-189`; the C++ generator lists the same 20 slots in the same order, followed by `drop_in_place` and `dealloc` (`generator/cpp.rs:2203-2212`).
- The `window_adapter` slot has the signature `(Pin<&Self>, do_create: bool, &mut Option<Rc<dyn WindowAdapter>>)` and the `slint_context` slot has `(Pin<&Self>, &mut Option<SlintContext>)`.
- `impl sp::RepeatedItemTree for Inner` implements the items listed in the model table above.
- `impl sp::PinnedDrop for Inner` is emitted only when `needs_window_adapter()` is true, and then calls `sp::unregister_item_tree`.

### 1.7 Builtin functions reached as `sp::builtin_functions::<name>`

- The generator emits `sp::builtin_functions::<runtime name>(&ctx?, item?, args..)` for every `BuiltinFunction` that has a `runtime` name (`rust.rs:4767-4793`).
- The 79 runtime names are: `modulo`, `round`, `ceil`, `floor`, `abs`, `sqrt`, `cos`, `sin`, `tan`, `acos`, `asin`, `atan`, `atan2`, `log`, `ln`, `pow`, `exp`, `to_fixed`, `to_precision`, `to_string_unlocalized`, `scroll_to`, `ensure_visible`, `set_selection_offsets`, `text_input_select_all`, `text_input_clear_selection`, `text_input_cut`, `text_input_copy`, `text_input_paste`, `text_input_undo`, `text_input_redo`, `has_selection`, `styled_text_set_selection_offsets`, `styled_text_select_all`, `styled_text_clear_selection`, `styled_text_copy`, `styled_text_has_selection`, `item_font_metrics`, `string_to_float`, `string_is_float`, `string_is_empty`, `string_character_count`, `string_to_lowercase`, `string_to_uppercase`, `string_starts_with`, `string_ends_with`, `string_replace_all`, `keys_to_string`, `color_rgba`, `color_hsva`, `color_oklch`, `color_brighter`, `color_darker`, `color_transparentize`, `color_mix`, `color_with_alpha`, `rgb`, `hsv`, `oklch`, `accent_color`, `reduced_motion`, `use_24_hour_format`, `month_day_count`, `month_offset`, `format_date`, `valid_date`, `swipe_gesture_handler_cancel`, `context_menu_close`, `context_menu_is_open`, `window_close`, `window_hide`, `item_absolute_position`, `detect_operating_system`, `open_url`, `string_to_styled_text`, `color_to_styled_text`, `decimal_separator`, `default_window_title`, `path_point_at`, `path_angle_at`.
- `path_point_at` and `path_angle_at` exist only with the core feature `path`.
- The bodies live in `internal/common/builtin_functions.rs` and are compiled inside `i_slint_core::builtin_functions` by `i_slint_common::for_each_builtin_function!(builtin_functions)` (`internal/core/builtin_functions.rs:111`).
- Bodies that depend on window or platform concepts: `item_font_metrics` calls `crate::items::text_item_font_metrics(w, &element)`.
- `set_selection_offsets`, `text_input_select_all`, `text_input_cut`, `text_input_copy`, `text_input_paste`, `text_input_undo`, `text_input_redo`, `styled_text_copy`, `context_menu_close`, `context_menu_is_open`, `window_close` and `window_hide` call `with_window_adapter(..)`, which asks `ItemTree::window_adapter(true, ..)` (`internal/core/builtin_functions.rs:102-106`).
- `open_url` calls `crate::open_url(&url, ctx)`, which calls `ctx.platform().open_url(url)` (`internal/core/lib.rs:214-215`).
- `default_window_title` calls `crate::window::default_window_title()` (`window.rs:664`).
- `reduced_motion` and `accent_color` read the context.
- These functions are also exported with `#[unsafe(export_name = concat!("slint_builtin_", name))]` when the `ffi` feature is on (`builtin_functions.rs:56`), for the C++ generator.
- The builtin functions that have no runtime name are expanded inline by the generator and are covered by the rows above: `SetFocusItem`, `ClearFocusItem`, `ShowPopupWindow`, `ClosePopupWindow`, `ShowPopupMenu`, `ShowPopupMenuInternal`, `ListViewEnsureRowVisible`, `ImplicitLayoutInfo`, `RegisterCustomFontByPath`, `RegisterCustomFontByMemory`, `RegisterBitmapFont`, `GetWindowScaleFactor`, `GetWindowDefaultFontSize`, `AnimationTick`, `Debug`, `ImageSize`, `Array*`, `ColorScheme`, `SupportsNativeMenuBar`, `SetupMenuBar`, `SetupSystemTrayIcon`, `ParseDate`, `DateNow`, `TextInputFocused`, `SetTextInputFocused`, `Translate`, `UpdateTimers`, `RestartTimer`, `MacosBringAllWindowsToFront`, `ParseMarkdown`.

### 1.8 Names that are built at compiler run time

These are not found by grepping for `sp::`, so they are listed here.

| Emitted as | Chosen from | Resolves to |
|---|---|---|
| `sp::<ItemClass>` and `sp::<ItemClass>::FIELD_OFFSETS` | `ElementType::Builtin` class names, `item.ty.class_name` (`rust.rs:2215,3544,1880`) | the structs listed in section 2 |
| `sp::<Enum>` and `sp::<Enum>::<Variant>` | builtin enumerations without a source node (`rust.rs:188,3943`) | the 40 enums of `internal/common/enums.rs`, defined by the macro at `items.rs:2089-2111` |
| `sp::<Struct>` | `BuiltinStruct` names that are neither public nor `LogicalPosition/LogicalSize` (`rust.rs:5468`) | `FontMetrics`, `MenuEntry`, `Edges` (generated at `items.rs:2366`), `Point` and `Size` (aliases at `graphics.rs:25,27`), `StateInfo` (`properties.rs:1249`), `PropertyAnimation` (`items.rs:1234`), `GridLayoutData`, `GridLayoutInputData`, `BoxLayoutData`, `BoxLayoutOrthoData`, `FlexboxLayoutData`, `LayoutItemInfo`, `FlexboxLayoutItemInfo`, `FlexItemProps`, `Padding`, `LayoutInfo` (all in `layout.rs`), `PathMoveTo`, `PathLineTo`, `PathArcTo`, `PathCubicTo`, `PathQuadraticTo`, `PathClose` (`graphics/path.rs`) |
| `sp::RgbaColor<u8>`, `sp::HsvaColor`, `sp::OklchColor` | `ColorRgba`, `ColorHsva`, `ColorOklch` | `graphics/color.rs` |
| `sp::<Global>` | builtin global components (`rust.rs:3379-3386`) | only `NativeStyleMetrics` and `NativePalette`, which come from the Qt backend; `TextInputInterface` and `Platform` are lowered to builtin functions before the generator (`passes/lower_text_input_interface.rs`, `passes/lower_platform.rs`) |
| `sp::<fn>` | `ExtraBuiltinFunctionCall` names (`rust.rs:4279`) | the twelve `layout` functions listed above |
| `sp::<helper>_erased` | `helper()` closure (`rust.rs:1031`) | `properties::*_erased` |

### 1.9 Symbols re-exported but never emitted

- `String`, `FromIterator`, `OnceBox`, `PinWeak`, `format`, `current_tick`, `LogicalPosition`, `LogicalPoint`, `logical_position_to_api`, `InputMethodRequest`, `FocusEvent`, `InputEventResult`, `KeyEvent`, `KeyEventResult`, `MouseEvent`, `Key`, `ItemTreeRefPin`, `ensure_item_tree_instantiated`, `ItemVisitorVTable`, `MenuVTable` and `MenuVTable_static` are in `re_exports` but not emitted by `rust.rs`.
- `PinWeak`, `StateInfo`, `PropertyAnimation`, `set_state_binding` and the `*_erased` helpers are used by `private_unstable_api.rs` itself.
- The fork may drop the rest only if it also edits `re_exports`, because user code and other crates can name them.

## 2. Generated item struct field access

### 2.1 How the generator reaches the fields of native items

- Every generated component struct derives `sp::FieldOffsets` and `Default`, is `#[repr(C)]` and `#[pin]` (`#[pin_drop]` for roots that need a window adapter), see `generator/rust.rs:2210-2213`.
- Its native items are fields of type `sp::<ClassName>`, where `ClassName` is `item.ty.class_name` of the compiler's `NativeClass` (`rust.rs:2215`).
- Reading or writing a native property emits `(<path to item field> + sp::<ClassName>::FIELD_OFFSETS.<property>())` and then `.apply_pin(_self)` (`rust.rs:3544`).
- Constant-property tables use `sp::compose_field_offsets(<item field>, sp::<ClassName>::FIELD_OFFSETS.<property>())` (`rust.rs:1880`).
- `FIELD_OFFSETS.<field>()` is a `const fn` produced by the `FieldOffsets` derive of `helper_crates/const-field-offset`; it returns a `FieldOffset<Item, Property<T>>`-like value that `apply_pin` turns into `Pin<&Property<T>>`.
- The item array stores one `sp::VOffset::new(<offset of the item field>)` per item (`rust.rs:2672`) with the type `[sp::VOffset<Inner, sp::ItemVTable, sp::AllowPin>; N]` (`rust.rs:2925-2926`), so each native item must implement `HasStaticVTable<ItemVTable>`.
- A whole native item is referenced as `(<path> item_field).apply_pin(owner)` and paired with `sp::ItemRc::new(origin_rc, tree_index)` (`rust.rs:3727-3731`).
- A few members are accessed as plain struct fields instead of through `FIELD_OFFSETS`.
- `ContextMenu::popup_id` (`pub Cell<Option<NonZeroU32>>`), `ContextMenu::sub_menu` and `ContextMenu::activated` (`pub Callback<..>`) are used at `rust.rs:4999,5006,5086`.
- `SystemTrayIcon::visible` is set with `.visible.set(..)` at `rust.rs:597-609`, and `SystemTrayIcon::set_menu(&ItemRc, VRc<MenuVTable>)` is called at `rust.rs:5393`.
- The generator never names `cached_rendering_data` or any private field of an item.

### 2.2 Naming rule

- The Rust struct name is the first identifier of the `item!` declaration in `internal/compiler/builtin_elements.rs`, unchanged (for example `item! { BorderRectangle: BasicBorderRectangle { .. } }` gives `sp::BorderRectangle`).
- The element name used in `.slint` can differ: the compiler picks the smallest class in the parent chain that has every property the element uses, so a plain `Rectangle { }` becomes `sp::Empty`.
- A property named in kebab-case in `item!` becomes the same name with every `-` replaced by `_`; `ident()` does this (`rust.rs:44-52`).
- A name without a dash becomes a raw identifier (`r#text`), which is the plain name.
- The core struct must have a `pub` field with that exact snake_case name, of type `Property<T>` or `Callback<Args, Ret>`.
- The parent chain in `item!` is compile-time only: each core struct repeats every field of its ancestors, so `BorderRectangle` has its own `background`.
- A script check of all 32 core items against their `item!` declarations found no other gap than the three kinds below.
- Deprecated aliases written with `<=>` (`color <=> background`, `rotation-angle <=> transform-rotation`, `viewport-x <=> content-x`, `viewport-width <=> content-width`) have no field; the compiler resolves them.
- Members declared as `function` or as `out property .. { BuiltinFunction.X }` (`font-metrics`, `has-selection`, `commands` of `Path`) have no field; they lower to builtin functions.
- Core structs have extra private or internal fields that the compiler does not know (`bounce`, `carry_momentum`, `data`, `popup_id`, `undo_items`, `pressed`, ..); the generator cannot reach them.
- The 16 `Native*` items are `item!` declarations without a core struct; they come from `i_slint_backend_qt` and are out of scope.
- The interpreter uses the same names at run time: `#[derive(SlintElement)]` turns each `pub` field named `a_b` into the property or callback `a-b` (`internal/core-macros/lib.rs:131-133`).
- That derive recognizes a property or callback only by the literal syntax `Property<..>` or `Callback<..>` as the first path segment of the field type (`core-macros/lib.rs:136,155`), and plain fields only with `#[rtti_field]`.

### 2.3 Examples

| Slint property | `item!` item | Emitted access |
|---|---|---|
| `Rectangle.background` | `Rectangle` | `sp::Rectangle::FIELD_OFFSETS.background()` |
| `BorderRectangle.border-top-left-radius` | `BorderRectangle` | `sp::BorderRectangle::FIELD_OFFSETS.border_top_left_radius()` |
| `TouchArea.has-hover` | `TouchArea` | `sp::TouchArea::FIELD_OFFSETS.has_hover()` |
| `TouchArea.clicked` (callback) | `TouchArea` | `sp::TouchArea::FIELD_OFFSETS.clicked()`, a `Callback<VoidArg>` |
| `Text.font-size`, declared on `SimpleText`, used on `ComplexText` | `ComplexText` | `sp::ComplexText::FIELD_OFFSETS.font_size()` |
| `Flickable.content-x` | `Flickable` | `sp::Flickable::FIELD_OFFSETS.content_x()`; the deprecated `viewport-x` is rewritten to it |
| `Transform.transform-rotation` | `Transform` | `sp::Transform::FIELD_OFFSETS.transform_rotation()` |
| `Layer.cache-rendering-hint` | `Layer` | `sp::Layer::FIELD_OFFSETS.cache_rendering_hint()` |
| `Window.default-font-size` | `WindowItem` | `sp::WindowItem::FIELD_OFFSETS.default_font_size()` |
| `FocusScope.focus-gained` (callback) | `FocusScope` | `sp::FocusScope::FIELD_OFFSETS.focus_gained()` |

### 2.4 The 32 items that the interpreter registers and the generator can name

- `ComponentContainer`, `Empty`, `ImageItem`, `ClippedImage`, `ComplexText`, `StyledTextItem`, `SimpleText`, `Rectangle`, `BasicBorderRectangle`, `BorderRectangle`, `TouchArea`, `TooltipArea`, `FocusScope`, `KeyBinding`, `SwipeGestureHandler`, `ScaleRotateGestureHandler`, `Path`, `Flickable`, `WindowItem`, `TextInput`, `Clip`, `BoxShadow`, `BackdropBlur`, `Transform`, `Opacity`, `Layer`, `DragArea`, `DropArea`, `WindowMoveArea`, `ContextMenu`, `MenuItem` and `SystemTrayIcon` (`internal/interpreter/item_registry.rs:43-76`).
- `MenuItem` lives in `menus.rs` and reaches `items::*` through `pub use crate::menus::MenuItem` (`items.rs:39`).
- The C++ list of items in `api/cpp/cbindgen.rs:481-513` has the same entries except `ComponentContainer`.
- `Path` exists only with the core feature `path`, which `std` and the interpreter turn on.

### 2.5 What each item struct must still provide

- `#[repr(C)]`, `#[derive(FieldOffsets, Default, SlintElement)]` and `#[pin]`.
- A field `cached_rendering_data: CachedRenderingData` in every item; the `ItemVTable` slot `cached_rendering_data_offset` points at it (`items.rs:138-142`).
- `CachedRenderingData` is `#[repr(C)]` with two `Cell<usize>` fields, `cache_index` and `cache_generation` (`partial_renderer.rs:44-51`).
- The C++ headers redefine it with a constexpr constructor (`api/cpp/cbindgen.rs:1003-1004`), so the layout is a C++ ABI contract as well.
- An `impl Item for <Struct>` with the 11 vtable methods: `init`, `deinit`, `layout_info`, `input_event_filter_before_children`, `input_event`, `focus_event`, `capture_key_event`, `key_event`, `render`, `bounding_rect`, `clips_children` (`items.rs:130-221`).
- Eight of those methods take `&WindowAdapterRc` (`deinit`, `layout_info`, `input_event_filter_before_children`, `input_event`, `focus_event`, `capture_key_event`, `key_event`, `bounding_rect`) and `render` takes `&mut ItemRendererRef`, so the `ItemVTable` signature itself depends on the seam.

## 3. C++ generated code

### 3.1 How the C++ side reaches `i-slint-core`

- The generated C++ never calls the core directly; it calls hand-written inline code in `api/cpp/include/slint.h` and `api/cpp/include/private/*.h`, which in turn call `extern "C"` functions of the core.
- The declarations of those C functions and the C++ layouts of the `#[repr(C)]` core types are produced by `cbindgen` at build time from the core sources, driven by `api/cpp/cbindgen.rs`.
- The output lands in `slint_internal.h`, `slint_string_internal.h`, `slint_sharedvector_internal.h`, `slint_properties_internal.h`, `slint_timer_internal.h`, `slint_image_internal.h`, `slint_color_internal.h`, `slint_pathdata_internal.h`, `slint_brush_internal.h`, `slint_data_transfer_internal.h`, `slint_events_internal.h`, `slint_keys_internal.h`, `slint_generated_public.h`, `slint_enums*.h`, `slint_builtin_structs*.h` and `slint_builtin_functions_internal.h`.
- The main header `slint_internal.h` is generated from `internal/core/lib.rs` plus `internal/common/bitmapfont.rs` (`cbindgen.rs:1068-1069`), so cbindgen follows the `mod` declarations through every module of the core, including `renderer`, `item_rendering`, `partial_renderer`, `platform` and `window`.
- The special-case headers are generated from these core files: `graphics.rs`, `graphics/color.rs`, `graphics/path.rs`, `graphics/brush.rs`, `graphics/image.rs`, `graphics/image/cache.rs`, `../common/pre_rendered.rs`, `data_transfer/ffi.rs`, `animations.rs`, `input.rs`, `items/drag_n_drop.rs`, `item_rendering.rs`, `window.rs` and `../renderers/software/lib.rs` (`cbindgen.rs:920-934`).
- The public header uses `graphics.rs`, `window.rs`, `api.rs`, `model.rs`, `graphics/image.rs` and `lengths.rs` (`cbindgen.rs:962-968`).
- The other headers use `string.rs`, `styled_text.rs`, `slice.rs`, `debug_info.rs`, `sharedvector.rs`, `properties.rs`, `properties/ffi.rs`, `callbacks.rs` and `timers.rs`.
- The fork can keep the C++ build only if it keeps these file names or updates `cbindgen.rs`; `item_rendering.rs` and the software renderer path in that list are the two entries that disappear in the planned layout.
- The types that cbindgen is told to export from the core include `Clipboard`, `ItemTreeVTable`, `Slice`, `WindowAdapterRcOpaque`, `PropertyAnimation`, `EasingCurve`, `MouseCursorInner`, `Rect`, `BitmapFont`, `StaticStorage`, `DataTransferOpaque`, `FlexboxLayoutItemInfo`, `PointerEvent`, `PointerScrollEvent`, `FocusReason`, `DragAction`, the text and image enums, and the 31 items of 3.4 (`cbindgen.rs:515-554`).
- The renames `Callback` to `private_api::CallbackHelper`, `ChangeTracker` to `private_api::ChangeTracker` and the `*Arg` aliases (`VoidArg`, `KeyEventArg`, `PointerEventArg`, `PointArg`, `MenuEntryModel`, ..) are part of the contract (`cbindgen.rs:400-425`).
- `CachedRenderingData` gets a hand-written constexpr constructor in C++ (`cbindgen.rs:1003-1004`), which fixes its two-`usize` layout.
- `ItemTreeVTable` and `ItemVTable` have their Rust field order as the C++ aggregate order, so slot order is an ABI contract.

### 3.2 Names that `generator/cpp.rs` emits

#### Under `slint::private_api::` (hand-written headers)

| Name | Defined in | Calls into the core |
|---|---|---|
| `Property`, `PropertyTracker`, `ChangeTracker`, `set_state_binding` | `private/slint_properties.h` | `properties/ffi.rs`: `slint_property_*`, `slint_change_tracker_*` |
| `Callback<Ret(Args..)>` (hand-written wrapper around the cbindgen type renamed to `private_api::CallbackHelper`) | `private/slint_callbacks.h` | `callbacks.rs`: `slint_callback_*` |
| `Repeater`, `Conditional`, `model_length`, `model_push`, `model_insert`, `model_remove`, `model_any`, `model_all`, `model_find_index`, `access_array_index` | `private/slint_models.h` | `model/repeater.rs`: `slint_repeater_*` |
| `ItemTreeVTable`, `ItemTreeRef`, `ItemTreeWeak`, `ItemWeak`, `ItemVisitorRefMut`, `TraversalOrder`, `IndexRange`, `ItemTreeNode`, `ItemArray`, `ItemArrayEntry`, `ItemRef`, `make_item_node`, `make_dyn_node`, `get_item_ref` | `private/slint_item_tree.h` | types only, from cbindgen |
| `register_item_tree` | `slint.h` | `item_tree.rs`: `slint_register_item_tree` |
| `WindowAdapterRc` | `private/slint_window.h`, forward-declared in `slint.h` | `window.rs`: `slint_windowrc_*`; `api/cpp/lib.rs`: `slint_windowrc_init`, `slint_register_font_*` |
| `optional_and_then`, `optional_or_default`, `optional_then`, `optional_transform` | `private/slint_window.h` | none |
| `layout_cache_access`, `layout_cache_grid_repeater_access`, `item_layout_info`, `solve_box_layout`, `box_layout_info_ortho`, `solve_flexbox_layout_with_measure`, `flexbox_layout_info_cross_axis_with_measure`, `convert_anonymous_rect`, `get_resolved_default_font_size` | `slint.h` | `layout.rs`: `slint_solve_*`, `slint_*_layout_info*`; `window.rs`: `slint_windowrc_resolved_default_font_size` |
| `translate`, `translate_from_bundle`, `translate_from_bundle_with_plural` | `slint.h` | `translations.rs`: `slint_translate*` |
| `debug`, `debug_info_format_*`, `parse_markdown`, `macos_bring_all_windows_to_front`, `saturating_float_to_int` | `slint.h` | `slint_debug` (api/cpp), `debug_info.rs`: `slint_debug_info_format_float`, `styled_text.rs`: `slint_styled_text_from_markdown` |
| `create_menu_wrapper`, `setup_menu_bar_from_menu_item_tree`, `setup_popup_menu_from_menu_item_tree` | `slint.h` | `menus.rs`: `slint_menus_create_wrapper`; `window.rs`: `slint_windowrc_setup_native_menu_bar`, `slint_windowrc_show_native_popup_menu`, `slint_windowrc_setup_menu_bar_shortcuts` |
| `make_slice`, `string_to_slice`, `make_keys` | `slint.h`, `private/slint_string.h`, `private/slint_keys.h` | `input.rs`: `slint_keys` |
| `call_builtin` | `private/slint_builtin_functions.h` | `builtin_functions.rs`: `slint_builtin_<name>` |
| `load_image_from_embedded_data`, `image_from_embedded_textures` | `private/slint_image.h` | `graphics/image.rs`: `slint_image_*` |
| `PathData`, `PathElement`, `PathLineTo`, `PathMoveTo`, `PathArcTo`, `PathCubicTo`, `PathQuadraticTo`, `PathClose` | `private/slint_pathdata.h` | `graphics/path.rs`: `slint_new_path_elements`, `slint_new_path_events` |
| `LinearGradientBrush`, `RadialGradientBrush`, `ConicGradientBrush`, `GradientStop` | `private/slint_brush.h`, `private/slint_color.h` | `graphics/brush.rs`, `graphics/color.rs` |
| `assert_main_thread`, `drop_in_place`, `dealloc` | `private/vtable.h`, `slint.h` | none |

#### Under `slint::cbindgen_private::` (cbindgen output or direct C functions)

- Types: `AccessibilityAction`, `AccessibleRole`, `AccessibleStringProperty`, `SupportedAccessibilityAction`, `BitmapFont`, `BitmapGlyph`, `BitmapGlyphs`, `CrossAxisAlignment`, `EasingCurve`, `FlexItemProps`, `FlexboxLayoutItemInfo`, `FocusReason`, `GridLayoutInputData`, `ItemRc`, `ItemTreeWeak`, `LayoutInfo`, `LayoutItemInfo`, `LogicalRect`, `Rect`, `MouseCursorInner`, `Option<T>`, `Orientation`, `PropertyAnimation`, `Slice<T>`, `TranslationsBundled`, `WindowAdapterRcOpaque`, `WindowKind::{Popup, ToolTip}`, `types::{StaticTexture, StaticTextures, TexturePixelFormat, RADIAL_GRADIENT_SHAPE_CIRCLE, RADIAL_GRADIENT_SHAPE_ELLIPSE}`.
- Items: `::slint::cbindgen_private::<ItemClass>` for each native item (`cpp.rs:2674`), and the global classes `::slint::cbindgen_private::<GlobalName>` for builtin globals (`cpp.rs:944`).
- Builtin structs: `::slint::cbindgen_private::<Name>` for non-public builtin structs and `::slint::language::<Name>` for public ones (`cpp.rs:527-536`); builtin enums are `::slint::cbindgen_private::<Enum>` (`cpp.rs:570`).
- Direct C functions: `slint_animation_tick`, `slint_builtin_<runtime name>`, `slint_context_color_scheme`, `slint_date_time_date_now`, `slint_date_time_parse_date`, `slint_ensure_backend`, `slint_system_tray_icon_set_menu`, `slint_translate_set_bundled_languages`, `slint_visit_item_tree`, `slint_visit_item_tree_z_sorted`, `slint_windowrc_ensure_tree_instantiated`.
- `slint_context_color_scheme` and `slint_ensure_backend` are defined in `api/cpp/lib.rs`, not in the core; the other names, counting the `slint_builtin_` family once, are core functions.

#### Under `slint::` (public C++ API)

- `Brush`, `Color`, `Color::from_argb_encoded`, `ComponentHandle`, `DataTransfer`, `HsvaColor`, `Image`, `Image::load_from_path`, `Keys`, `Model`, `OklchColor`, `RgbaColor`, `SharedString`, `SharedString::from_number`, `SharedVector`, `StyledText`, `Timer`, `TimerMode::Repeated`, `VectorModel`, `Window`, `run_event_loop`, and `slint::language::<public builtin struct or enum>`.
- The window-facing calls that cpp.rs spells on the C++ `WindowAdapterRc` object are `set_focus_item`, `show_popup<T>`, `show_popup_menu<T>`, `close_popup`, `register_font_from_path`, `register_font_from_data` and `register_bitmap_font` (`cpp.rs:5265,5276,5502,5579,5624,5636,5646`).
- Two lines are guarded by the compiler feature `renderer-software` (`cpp.rs:1150,1206`).
- The C++ `ItemTreeVTable` is filled in the order given in 1.6 (`cpp.rs:2203-2212`), with `window_adapter` implemented as `*reinterpret_cast<WindowAdapterRc*>(result) = ...->globals->window().window_handle()` (`cpp.rs:2155-2175`).

### 3.3 Core `extern "C"` functions, grouped by core module

- The core exports 179 hand-written C functions plus macro-generated ones.
- The list below comes from grepping `#[unsafe(no_mangle)]` in `internal/core`.
- A header entry means the name occurs in a file of `api/cpp/include`.
- A `generator/cpp.rs` entry means the generator emits the name itself.

- `callbacks.rs` (4): `slint_callback_init`, `slint_callback_call`, `slint_callback_set_handler`, `slint_callback_drop`.
  - The headers `slint_callbacks.h` declare or call some of these functions.
- `data_transfer/ffi.rs` (17): `slint_data_transfer_init_default`, `slint_data_transfer_drop`, `slint_data_transfer_clone`, `slint_data_transfer_eq`, `slint_data_transfer_set_plain_text`, `slint_data_transfer_set_image`, `slint_data_transfer_set_file_paths`, `slint_data_transfer_has_plain_text`, `slint_data_transfer_has_image`, `slint_data_transfer_has_file_paths`, `slint_data_transfer_is_empty`, `slint_data_transfer_plain_text`, `slint_data_transfer_image`, `slint_data_transfer_file_paths`, `slint_data_transfer_set_user_data`, `slint_data_transfer_user_data`, `slint_data_transfer_clear_user_data`.
  - The headers `slint_data_transfer.h` declare or call some of these functions.
- `date_time.rs` (2): `slint_date_time_date_now`, `slint_date_time_parse_date`.
  - `generator/cpp.rs` names `slint_date_time_date_now`, `slint_date_time_parse_date`.
- `debug_info.rs` (1): `slint_debug_info_format_float`.
  - The headers `slint.h` declare or call some of these functions.
- `graphics/brush.rs` (3): `slint_conic_gradient_normalize_stops`, `slint_conic_gradient_apply_rotation`, `slint_brush_compare_equal`.
  - The headers `slint_brush.h` declare or call some of these functions.
- `graphics/color.rs` (6): `slint_color_brighter`, `slint_color_darker`, `slint_color_transparentize`, `slint_color_with_alpha`, `slint_color_from_hsva`, `slint_color_from_oklch`.
  - The headers `slint_color.h` declare or call some of these functions.
- `graphics/image.rs` (11): `slint_image_load_from_path`, `slint_image_load_from_embedded_data`, `slint_image_load_from_data`, `slint_image_size`, `slint_image_path`, `slint_image_from_embedded_textures`, `slint_image_compare_equal`, `slint_image_set_nine_slice_edges`, `slint_image_to_rgb8`, `slint_image_to_rgba8`, `slint_image_to_rgba8_premultiplied`.
  - The headers `slint_image.h` declare or call some of these functions.
- `graphics/path.rs` (2): `slint_new_path_elements`, `slint_new_path_events`.
  - The headers `slint_pathdata.h` declare or call some of these functions.
- `input.rs` (4): `slint_keys`, `slint_keys_debug_string`, `slint_keys_from_parts`, `slint_keys_to_parts`.
  - The headers `slint_keys.h`, `slint.h` declare or call some of these functions.
  - No header and not `cpp.rs` names `slint_keys_debug_string`.
- `item_tree.rs` (4): `slint_register_item_tree`, `slint_unregister_item_tree`, `slint_visit_item_tree`, `slint_visit_item_tree_z_sorted`.
  - The headers `slint.h`, `slint_window.h` declare or call some of these functions.
  - `generator/cpp.rs` names `slint_visit_item_tree`, `slint_visit_item_tree_z_sorted`.
- `items/system_tray.rs` (1): `slint_system_tray_icon_set_menu`.
  - `generator/cpp.rs` names `slint_system_tray_icon_set_menu`.
- `layout.rs` (13): `slint_organize_grid_layout`, `slint_organize_dialog_button_layout`, `slint_solve_grid_layout`, `slint_grid_layout_info`, `slint_solve_box_layout`, `slint_solve_box_layout_ortho`, `slint_box_layout_info`, `slint_box_layout_info_ortho`, `slint_solve_flexbox_layout`, `slint_flexbox_layout_info_main_axis`, `slint_flexbox_layout_unwrapped_main`, `slint_flexbox_layout_info_cross_axis`, `slint_flexbox_layout_info_cross_axis_with_measure`.
  - The headers `slint.h` declare or call some of these functions.
- `menus.rs` (1): `slint_menus_create_wrapper`.
  - The headers `slint.h` declare or call some of these functions.
- `model/repeater.rs` (5): `slint_repeater_scroll_to_row_construct`, `slint_repeater_scroll_to_row_drop`, `slint_repeater_ensure_updated`, `slint_repeater_ensure_updated_listview`, `slint_repeater_listview_ensure_row_visible`.
  - The headers `slint_models.h` declare or call some of these functions.
- `once_box.rs` (1): `slint_once_box_drop`.
  - The headers `slint.h` declare or call some of these functions.
- `properties/ffi.rs` (32): `slint_property_init`, `slint_property_update`, `slint_property_register_as_dependency`, `slint_property_set_changed`, `slint_property_set_binding`, `slint_property_set_binding_internal`, `slint_property_delete_binding`, `slint_property_evaluate_binding`, `slint_property_intercept_set_binding`, `slint_property_is_dirty`, `slint_property_mark_dirty`, `slint_property_is_currently_tracking`, `slint_property_set_constant`, `slint_property_drop`, `slint_property_set_animated_value_int`, `slint_property_set_animated_value_float`, `slint_property_set_animated_value_color`, `slint_property_set_animated_value_brush`, `slint_property_set_animated_binding_int`, `slint_property_set_animated_binding_float`, `slint_property_set_animated_binding_color`, `slint_property_set_animated_binding_brush`, `slint_property_set_state_binding`, `slint_property_tracker_init`, `slint_property_tracker_evaluate`, `slint_property_tracker_evaluate_as_dependency_root`, `slint_property_tracker_is_dirty`, `slint_property_tracker_drop`, `slint_change_tracker_construct`, `slint_change_tracker_drop`, `slint_change_tracker_init`, `slint_animation_tick`.
  - The headers `slint_properties.h`, `slint_brush.h` declare or call some of these functions.
  - `generator/cpp.rs` names `slint_animation_tick`.
- `sharedvector.rs` (3): `slint_shared_vector_allocate`, `slint_shared_vector_free`, `slint_shared_vector_empty`.
  - The headers `slint_sharedvector.h` declare or call some of these functions.
- `string.rs` (6): `slint_shared_string_bytes`, `slint_shared_string_drop`, `slint_shared_string_clone`, `slint_shared_string_from_bytes`, `slint_shared_string_from_number`, `slint_shared_string_append`.
  - The headers `slint_string.h` declare or call some of these functions.
- `styled_text.rs` (6): `slint_styled_text_new`, `slint_styled_text_drop`, `slint_styled_text_eq`, `slint_styled_text_clone`, `slint_styled_text_from_plain_text`, `slint_styled_text_from_markdown`.
  - The headers `slint_string.h` declare or call some of these functions.
- `timers.rs` (7): `slint_timer_start`, `slint_timer_singleshot`, `slint_timer_destroy`, `slint_timer_stop`, `slint_timer_restart`, `slint_timer_running`, `slint_timer_interval`.
  - The headers `slint_timer.h` declare or call some of these functions.
- `translations.rs` (6): `slint_translate`, `slint_translations_mark_dirty`, `slint_translate_from_bundle`, `slint_translate_from_bundle_with_plural`, `slint_translate_set_bundled_languages`, `slint_translate_select_bundled_translation`.
  - The headers `slint.h` declare or call some of these functions.
  - `generator/cpp.rs` names `slint_translate_set_bundled_languages`.
- `window.rs` (44): `slint_windowrc_drop`, `slint_windowrc_clone`, `slint_windowrc_ensure_tree_instantiated`, `slint_windowrc_show`, `slint_windowrc_hide`, `slint_windowrc_is_visible`, `slint_windowrc_get_scale_factor`, `slint_windowrc_set_const_scale_factor`, `slint_windowrc_get_text_input_focused`, `slint_windowrc_set_text_input_focused`, `slint_windowrc_set_focus_item`, `slint_windowrc_set_component`, `slint_windowrc_show_popup`, `slint_windowrc_create_child_window_adapter`, `slint_windowrc_close_popup`, `slint_windowrc_set_rendering_notifier`, `slint_windowrc_on_close_requested`, `slint_windowrc_request_redraw`, `slint_windowrc_position`, `slint_windowrc_set_physical_position`, `slint_windowrc_set_logical_position`, `slint_windowrc_size`, `slint_windowrc_set_physical_size`, `slint_windowrc_set_logical_size`, `slint_windowrc_supports_native_menu_bar`, `slint_windowrc_setup_native_menu_bar`, `slint_windowrc_setup_menu_bar_shortcuts`, `slint_windowrc_show_native_popup_menu`, `slint_windowrc_resolved_default_font_size`, `slint_windowrc_dispatch_key_event`, `slint_windowrc_dispatch_pointer_event`, `slint_windowrc_dispatch_event`, `slint_windowrc_is_fullscreen`, `slint_windowrc_is_minimized`, `slint_windowrc_is_maximized`, `slint_windowrc_set_fullscreen`, `slint_windowrc_set_minimized`, `slint_windowrc_set_maximized`, `slint_windowrc_take_snapshot`, `slint_windowrc_hwnd_win32`, `slint_windowrc_hinstance_win32`, `slint_windowrc_wlsurface_wayland`, `slint_windowrc_wldisplay_wayland`, `slint_windowrc_nsview_appkit`.
  - The headers `slint_window.h`, `slint.h` declare or call some of these functions.
  - `generator/cpp.rs` names `slint_windowrc_ensure_tree_instantiated`.

- Macro-generated exports are not in the grep list above.
- `builtin_functions.rs:56` exports `slint_builtin_<runtime name>` for the 79 runtime builtins (77 without the `path` feature); only `color_brighter`, `color_darker`, `color_hsva`, `color_mix`, `color_oklch`, `color_transparentize`, `color_with_alpha`, `item_absolute_position`, `keys_to_string`, `string_to_lowercase` and `string_to_uppercase` are named in a header, and `cpp.rs:5229` builds the others by name.
- `items.rs:89-114` (`declare_item_vtable!`) exports one static `<Item>VTable` per item (`#[unsafe(no_mangle)] pub static`), and on Windows also a getter `slint_get_<Item>VTable()` (the `all(feature = "ffi", windows)` variant).
- No vtable static is exported for `ItemTreeVTable` or `MenuVTable`; `ItemTreeVTable_static!` and `MenuVTable_static!` are Rust macros that C++ does not use.
- Exports that live outside the core but are called by the same headers: `api/cpp/lib.rs` (`slint_windowrc_init`, `slint_ensure_backend`, `slint_run_event_loop`, `slint_post_event`, `slint_quit_event_loop`, `slint_register_font_from_path`, `slint_register_font_from_data`, `slint_register_bitmap_font`, `slint_context_color_scheme`, `slint_string_to_float`, `slint_debug`, `slint_select_backend`, `slint_parse_markdown`, `slint_translate_set_translator`, ..) and `api/cpp/platform.rs` (`slint_platform_register`, `slint_window_adapter_new`, `slint_windowrc_has_active_animations`, `slint_platform_update_timers_and_animations`, `slint_software_renderer_*`, `slint_new_raw_window_handle_*`, ..).
- The `lib.rs` functions `slint_register_font_from_path/data` and `slint_register_bitmap_font` call `window_adapter.renderer().register_*` (`api/cpp/lib.rs:169-205`), the same renderer seam as the Rust generator.

### 3.4 The 31 item statics the C++ side declares

- `Empty`, `Rectangle`, `BasicBorderRectangle`, `BorderRectangle`, `DragArea`, `DropArea`, `WindowMoveArea`, `ImageItem`, `ClippedImage`, `TouchArea`, `TooltipArea`, `FocusScope`, `KeyBinding`, `SwipeGestureHandler`, `ScaleRotateGestureHandler`, `Flickable`, `SimpleText`, `StyledTextItem`, `ComplexText`, `Path`, `WindowItem`, `TextInput`, `Clip`, `BoxShadow`, `BackdropBlur`, `Transform`, `Opacity`, `Layer`, `ContextMenu`, `MenuItem`, `SystemTrayIcon` (`api/cpp/cbindgen.rs:481-513`).
- Each is declared by `SLINT_DECL_ITEM(Name)` as `extern const cbindgen_private::ItemVTable <Name>VTable` and `slint_get_<Name>VTable()` (`cbindgen.rs:450-474`).
- The item's data fields reach C++ as `cbindgen_private::<Name>`, so each Rust item struct must stay `#[repr(C)]` with the same field order that cbindgen sees.

## 4. Interpreter

### 4.1 Scope and method

- The interpreter is `internal/interpreter` (`slint-interpreter`): 22 source files of about 17,000 lines (counting `tests.rs`) plus nine integration tests in `tests/`.
- It depends on `i-slint-core` with the features `rtti` and `path`, and on non-wasm targets also `image-decoders` and `svg` (`internal/interpreter/Cargo.toml:197,229`).
- It also depends on `i-slint-backend-selector` with the feature `rtti` (`Cargo.toml:199`), and on `i-slint-common`, `i-slint-core-macros`, `vtable` and the compiler.
- The counts in 4.3 come from a parse of every `use i_slint_core::..` tree and every qualified `i_slint_core::..` path, with comments stripped.
- A "(module itself)" entry is a use of the module path without an item, as in `use i_slint_core::rtti;`; a `*` entry is a glob import.
- Paths built by macros are counted once at the macro and not per expansion: `api.rs:363-394` expands `i_slint_core::items::$Name` for every builtin struct and enum through `for_each_builtin_structs!` and `for_each_enums!`.
- `eval.rs:2147` expands `i_slint_core::builtin_functions::$name(..)` for every one of the 79 runtime builtins through `for_each_builtin_function!`, and `item_registry.rs:43-76` names 32 item types through `use i_slint_core::items::*`.

### 4.2 Highlights for the fork

#### `rtti`

- The interpreter needs the core feature `rtti` and the derive `SlintElement` on every item struct.
- `item_registry.rs:14,43-76` registers 32 item types with `<T as BuiltinItem>::name()` and builds them with `T::default()`.
- `item_holder.rs:12-35` implements `ErasedItem` for every `T: BuiltinItem + HasStaticVTable<ItemVTable>`; it calls `BuiltinItem::properties::<Value>()` and `callbacks::<Value>()` and then `PropertyInfo::{get, set, set_binding, prepare_for_two_way_binding, link_two_way_with_map}` and `CallbackInfo::{call, set_handler}`.
- `BuiltinItem::fields` and `FieldInfo` are not called; `ErasedItem::set_field` returns `Err(())` (`item_holder.rs:85-90`).
- `globals.rs:13,39,87` requires `rtti::BuiltinGlobal` (`fn new() -> Pin<Rc<Self>>`) for the native globals that come from `i_slint_backend_selector::NativeGlobals`.
- `erased.rs:13`, `bindings.rs:20` and `item_holder.rs:12-15` use `rtti::{AnimatedBindingKind, TwoWayBindingMapping, CallbackInfo, PropertyInfo}`.
- `api.rs:173` implements `rtti::ValueType` for the interpreter's `Value`; `rtti.rs:29-71` generates that trait from `for_each_enums!`.

#### `renderer` and `RendererSealed` methods reached through `WindowAdapter::renderer()`

- `instance.rs:294` calls `adapter.renderer().free_graphics_resources(item_tree_ref, &mut items.iter().copied())` in `Drop for Instance`.
- `instance.rs:406` calls `adapter.renderer().set_window_adapter(&adapter)` right after the window adapter is created.
- `eval.rs:2384` calls `adapter.renderer().register_font_from_path(&Path)` for `RegisterCustomFontByPath`.
- `eval.rs:2580` calls `adapter.renderer().register_shared_bitmap_font(&font)`, only with the feature `internal-mcu-interpreter` (core feature `owned-pre-rendered-resources`).
- No file of the interpreter names the module `i_slint_core::renderer` or the trait `Renderer`; the dependency is the method `WindowAdapter::renderer()`.

#### `platform`

- No non-test line of the interpreter uses `i_slint_core::platform`.
- The tests do: `tests.rs:1157,1165` (`WindowEvent::PointerExited`, `EventLoopProxy`, `Platform`, `PlatformError`, `WindowAdapter`), `tests/window_adapter_creation_error.rs:11` (`Platform`, `PlatformError`, `WindowAdapter`, `set_platform`) and `tests/menu_native_popup.rs:7` (`Key`, `PointerEventButton`, `WindowAdapter`, `WindowEvent`).
- Indirect uses: `instance.rs:396` calls `SlintContext::create_window_adapter()`, which calls `Platform::create_window_adapter`; `api.rs:1546,2202,2209` call `i_slint_backend_selector::{with_global_context, with_platform}`; `instance.rs:295` and `eval.rs:2301` call `WindowAdapter::internal(InternalToken)` to reach `WindowAdapterInternal::{unregister_item_tree, supports_native_menu_bar}`.
- `api.rs:31` re-exports `i_slint_backend_selector::api::*` (the `BackendSelector`) as part of the interpreter's public API.
- `component.rs:563` calls `i_slint_common::get_native_style(i_slint_backend_selector::HAS_NATIVE_STYLE, ..)`.

#### `item_rendering`, `partial_renderer`, `ItemRenderer`, `CachedRenderingData`

- None of these names occurs in the interpreter.
- The interpreter still creates native items with `T::default()`, so it depends on `CachedRenderingData: Default` through the item structs.

#### `window::WindowAdapter*`

- `WindowAdapterRc` is stored in `Instance.window_adapter: OnceCell<WindowAdapterRc>` (`instance.rs:227`) and passed through `component.rs:96,123,339`, `eval.rs:2666`, `popup.rs:275`, `item_tree_vtable.rs:22,440` and `api.rs:1568` (`UseExistingWindow(WindowAdapterRc)`).
- `ffi.rs:13,574-586` casts `Rc<dyn WindowAdapter>` to `i_slint_core::window::ffi::WindowAdapterRcOpaque`, which asserts equal size (`ffi.rs:577-578`).
- `WindowInner` methods called: `from_pub`, `window_adapter`, `context`, `ensure_tree_instantiated`, `set_component`, `set_focus_item`, `show_popup`, `close_popup`, `active_popups`, `show_native_popup_menu`, `create_child_window_adapter`, `setup_menubar`, `setup_menubar_shortcuts`, `supports_native_menu_bar`, `text_input_focused`, `set_text_input_focused` (`eval.rs:2300-2334,2522-2541`, `popup.rs:117-433`, `instance.rs:299-305,438`, `api.rs:1522,1784`).
- `WindowInner::context().clone()` is read from a given adapter at `instance.rs:637,680` so that a live-preview component can reuse the old window.
- `window::context_for_root` is called once, at `instance.rs:708`, to find the context of a host item tree; the `slint_context` slot of the interpreter's `ItemTree` returns `globals.context.clone()` (`item_tree_vtable.rs:382-384`).

#### `textlayout` and fonts

- `eval.rs:2369-2370` builds a `fontique::Blob` through `i_slint_core::textlayout::sharedparley::fontique::Blob::new` and calls `SlintContext::font_context().borrow_mut().register_fonts(blob)`.
- That is the only use of `textlayout` and of `font_context()` outside tests (`tests.rs:1316`).
- The interpreter also implements `ItemTree for Instance` (`item_tree_vtable.rs:56`) and `RepeatedItemTree for Instance` (`instance.rs:1153`) by hand, so the whole `ItemTreeVTable` and `RepeatedItemTree` surface of section 1.6 is a contract for it too, with `ItemTreeVTable_static!` at `item_tree_vtable.rs:25`.

#### Duplicated core logic

- `Drop for Instance` (`instance.rs:262-310`) re-implements `item_tree::unregister_item_tree`: deinit each item, free graphics resources, notify `WindowAdapterInternal`, close popups of removed parents.
- Any change of the core function must be mirrored there.

### 4.3 Every `i_slint_core` path by core module

- Modules are ordered by number of occurrences.
- "Non-test" counts occurrences outside `tests.rs`, `tests/` and `#[cfg(test)]` items.

The interpreter and its tests contain 587 occurrences of `i_slint_core` paths in total.
454 of them are in non-test code and 133 are in test code (`tests.rs`, `tests/*.rs` and inline test modules).

#### `items`

81 non-test occurrences and 13 test occurrences.

Non-test paths: `ItemRc::new` (10), `Orientation` (9), `PropertyAnimation` (8), `Orientation::Vertical` (7), `(module itself)` (5), `ItemVTable` (5), `Orientation::Horizontal` (5), `ContextMenu` (3), `ItemRc` (3), `ItemRef` (3), `MenuEntry` (3), `*` (2), `ComponentContainer` (2), `ItemRef::downcast_pin` (2), `AccessibleRole` (1), `AngleInterpolation` (1), `BasicBorderRectangle` (1), `BorderRectangle` (1), `CrossAxisAlignment` (1), `CrossAxisAlignment::Auto` (1), `DialogButtonRole` (1), `PathEvent` (1), `PopupClosePolicy` (1), `PopupClosePolicy::CloseOnClickOutside` (1), `ScrollBehavior` (1), `SystemTrayIcon` (1), `Transform` (1), `WindowItem::resolved_default_font_size` (1).
Test-only paths: `ItemRc` (4), `Clip` (2), `Opacity` (2), `Transform` (2), `BoxShadow` (1), `ColorScheme::Dark` (1), `Layer` (1).

#### `(crate root)`

40 non-test occurrences and 34 test occurrences.

Non-test paths: `InternalToken` (20), `Color` (3), `SharedString` (3), `SharedVector` (3), `Brush::SolidColor` (2), `SharedVector::default` (2), `Brush` (1), `Callback` (1), `Coord` (1), `ItemTreeVTable_static` (1), `Property` (1), `SlintContext` (1), `macos_bring_all_windows_to_front` (1).
Test-only paths: `InternalToken` (26), `Color::from_rgb_u8` (3), `Brush::SolidColor` (2), `Color::from_argb_u8` (1), `Property` (1), `SharedString` (1).

#### `model`

61 non-test occurrences and 12 test occurrences.

Non-test paths: `Model` (9), `ModelRc` (8), `RepeatedItemTree` (7), `ModelRc::new` (5), `VecModel::from` (4), `report_model_error` (3), `ListViewProperties` (2), `ModelError` (2), `ModelError::out_of_bounds` (2), `ModelExt` (2), `ModelRc::default` (2), `ModelRc::from` (2), `ModelTracker` (2), `SharedVectorModel` (2), `VecModel` (2), `Conditional` (1), `ModelNotify` (1), `Repeater` (1), `SharedVectorModel::from` (1), `model_all` (1), `model_any` (1), `model_find_index` (1).
Test-only paths: `ModelRc` (3), `VecModel` (3), `*` (2), `Model` (1), `ModelNotify` (1), `ModelTracker` (1), `VecModel::from` (1).

#### `graphics`

36 non-test occurrences and 28 test occurrences.

Non-test paths: `GradientStop` (4), `Brush` (3), `Image` (3), `PathElement` (3), `PathData` (2), `ConicGradientBrush` (1), `HsvaColor` (1), `Image::load_from_path` (1), `ImageInner` (1), `LinearGradientBrush` (1), `LinearGradientBrush::new` (1), `LoadImageError` (1), `OklchColor` (1), `PathArcTo` (1), `PathCubicTo` (1), `PathLineTo` (1), `PathMoveTo` (1), `PathQuadraticTo` (1), `Point` (1), `RadialGradientBrush` (1), `RgbaColorU8` (1), `euclid` (1), `euclid::approxeq::ApproxEq` (1), `load_as_html_image` (1), `load_image_from_data_uri` (1), `load_image_from_embedded_data` (1).
Test-only paths: `GradientStop` (14), `euclid` (4), `LinearGradientBrush::new` (2), `RadialGradientBrush::new_circle` (2), `ApproxEq` (1), `IntRect` (1), `IntSize` (1), `RadialGradientBrush::new_ellipse` (1), `Rgba8Pixel` (1), `SharedPixelBuffer::new` (1).

#### `layout`

48 non-test occurrences and 0 test occurrences.

Non-test paths: `LayoutItemInfo` (8), `LayoutInfo` (6), `FlexItemProps` (4), `FlexboxLayoutItemInfo` (4), `LayoutItemInfo::default` (2), `box_layout_info_ortho` (2), `solve_box_layout` (2), `BoxLayoutData` (1), `BoxLayoutOrthoData` (1), `FlexboxLayoutData` (1), `GridLayoutData` (1), `GridLayoutInputData` (1), `LayoutInfo::default` (1), `Orientation` (1), `Padding` (1), `box_layout_info` (1), `flexbox_layout_info_cross_axis` (1), `flexbox_layout_info_cross_axis_with_measure` (1), `flexbox_layout_info_main_axis` (1), `flexbox_layout_unwrapped_main` (1), `grid_layout_info` (1), `organize_dialog_button_layout` (1), `organize_grid_layout` (1), `solve_box_layout_ortho` (1), `solve_flexbox_layout` (1), `solve_flexbox_layout_with_measure` (1), `solve_grid_layout` (1).

#### `window`

38 non-test occurrences and 8 test occurrences.

Non-test paths: `WindowInner::from_pub` (17), `WindowAdapterRc` (11), `WindowKind::Menu` (2), `ffi::WindowAdapterRcOpaque` (2), `WindowAdapter` (1), `WindowInner` (1), `WindowKind` (1), `WindowKind::Popup` (1), `WindowKind::ToolTip` (1), `context_for_root` (1).
Test-only paths: `WindowInner::from_pub` (5), `WindowInner` (2), `PopupWindowLocation` (1).

#### `item_tree`

33 non-test occurrences and 3 test occurrences.

Non-test paths: `TraversalOrder` (4), `VisitChildrenResult` (4), `ItemTree` (3), `ItemVisitorVTable` (3), `ItemTreeNode` (2), `ItemTreeRc` (2), `ItemTreeVTable` (2), `ParentItemTraversalMode::StopAtPopups` (2), `IndexRange` (1), `ItemRc` (1), `ItemTreeWeak` (1), `ItemVisitor` (1), `ItemVisitorRefMut` (1), `ItemWeak` (1), `VisitChildrenResult::CONTINUE` (1), `ensure_item_tree_instantiated` (1), `register_item_tree` (1), `visit_item_tree` (1), `visit_item_tree_z_sorted` (1).
Test-only paths: `ItemTreeRc::ptr_eq` (2), `ParentItemTraversalMode` (1).

#### `lengths`

27 non-test occurrences and 1 test occurrences.

Non-test paths: `LogicalLength` (15), `LogicalPoint` (3), `ItemTransform` (2), `LogicalLength::new` (2), `LogicalRect` (2), `LogicalPoint::new` (1), `LogicalSize::new` (1), `LogicalVector` (1).
Test-only paths: `LogicalRect` (1).

#### `api`

8 non-test occurrences and 6 test occurrences.

Non-test paths: `LogicalPosition` (3), `PlatformError` (2), `*` (1), `LogicalSize` (1), `PlatformError::Other` (1).
Test-only paths: `EventLoopError` (2), `Window` (2), `ComponentHandle` (1), `LogicalPosition` (1).

#### `platform`

0 non-test occurrences and 13 test occurrences.

Test-only paths: `WindowAdapter` (3), `Platform` (2), `PlatformError` (2), `EventLoopProxy` (1), `Key` (1), `PointerEventButton` (1), `WindowEvent` (1), `WindowEvent::PointerExited` (1), `set_platform` (1).

#### `debug_log`

6 non-test occurrences and 6 test occurrences.

Non-test paths: `(module itself)` (3), `LogMessageLocation` (2), `*` (1).
Test-only paths: `LogMessage` (4), `LogMessageHandler` (2).

#### `rtti`

11 non-test occurrences and 0 test occurrences.

Non-test paths: `AnimatedBindingKind` (3), `BuiltinItem` (2), `TwoWayBindingMapping` (2), `(module itself)` (1), `CallbackInfo` (1), `PropertyInfo` (1), `ValueType` (1).

#### `animations`

9 non-test occurrences and 0 test occurrences.

Non-test paths: `Instant` (5), `EasingCurve` (3), `animation_tick` (1).

#### `menus`

9 non-test occurrences and 0 test occurrences.

Non-test paths: `MenuFromItemTree` (2), `MenuFromItemTree::new_with_condition_and_visible` (2), `MenuVTable` (2), `Menu::activate` (1), `Menu::sub_menu` (1), `MenuFromItemTree::new` (1).

#### `properties`

7 non-test occurrences and 0 test occurrences.

Non-test paths: `ChangeTracker` (2), `InterpolatedPropertyValue` (1), `Property` (1), `Property::new` (1), `StateInfo` (1), `set_state_binding` (1).

#### `accessibility`

3 non-test occurrences and 4 test occurrences.

Non-test paths: `AccessibilityAction` (1), `AccessibleStringProperty` (1), `SupportedAccessibilityAction` (1).
Test-only paths: `AccessibleStringProperty` (1), `accessible_descendents` (1), `find_exposed_text_input` (1), `find_text_input_with_rc` (1).

#### `input`

6 non-test occurrences and 0 test occurrences.

Non-test paths: `FocusReason::Programmatic` (2), `Keys` (2), `KeyboardModifiers::default` (1), `make_keys` (1).

#### `slice`

5 non-test occurrences and 1 test occurrences.

Non-test paths: `Slice` (3), `Slice::from_slice` (2).
Test-only paths: `Slice::from_slice` (1).

#### `cursor`

5 non-test occurrences and 0 test occurrences.

Non-test paths: `MouseCursorInner` (5).

#### `data_transfer`

5 non-test occurrences and 0 test occurrences.

Non-test paths: `DataTransfer` (2), `ffi::PathValueType` (1), `ffi::path_from_units` (1), `ffi::path_to_units` (1).

#### `styled_text`

5 non-test occurrences and 0 test occurrences.

Non-test paths: `StyledText` (4), `parse_markdown` (1).

#### `context`

0 non-test occurrences and 4 test occurrences.

Test-only paths: `SlintContext` (1), `set_window_event_hook` (1), `set_window_shown_hook` (1), `with_existing_context` (1).

#### `date_time`

2 non-test occurrences and 0 test occurrences.

Non-test paths: `date_now` (1), `parse_date` (1).

#### `component_factory`

2 non-test occurrences and 0 test occurrences.

Non-test paths: `ComponentFactory` (1), `FactoryContext` (1).

#### `timers`

2 non-test occurrences and 0 test occurrences.

Non-test paths: `Timer` (1), `TimerMode::Repeated` (1).

#### `builtin_functions`

2 non-test occurrences and 0 test occurrences.

Non-test paths: `(module itself)` (1), `ItemArg::new` (1).

#### `translations`

1 non-test occurrences and 0 test occurrences.

Non-test paths: `FormatArgs` (1).

#### `textlayout`

1 non-test occurrences and 0 test occurrences.

Non-test paths: `sharedparley::fontique::Blob::new` (1).

#### `debug_info`

1 non-test occurrences and 0 test occurrences.

Non-test paths: `(module itself)` (1).


## 5. Public Rust API

- The public crate is `api/rs/slint` (`slint`); it is a facade over `i_slint_core`, `i_slint_backend_selector` and the renderers.
- "Kept" means the fork can leave the item as it is or with a mechanical change; "incompatible" means the item depends on a removed module and cannot keep its upstream form.
- The status column follows the plan: `slint::platform`, the software, FemtoVG and Skia renderers, `set_rendering_notifier`, `GraphicsAPI`, wgpu, `BackendSelector`, MCU and `Native*` elements are incompatible.

### 5.1 Top level of `api/rs/slint/lib.rs`

| Item | Defined in | Status |
|---|---|---|
| `slint!` (`lib.rs:223`) | `slint_macros` | kept; it only calls the compiler and emits the code of section 1 |
| everything in `i_slint_backend_selector::api::*`, that is `BackendSelector` (`lib.rs:225`) | `internal/backends/selector/api.rs:33-471` | incompatible: it selects Qt, winit, LinuxKMS and renderers |
| everything in `i_slint_core::api::*` (`lib.rs:226`) | `internal/core/api.rs` | see 5.2 |
| `ComponentFactory` (hidden and deprecated, `lib.rs:227-229`) | `component_factory` | kept; the generator still emits it |
| `BorrowedOpenGLTextureBuilder`, `BorrowedOpenGLTextureOrigin` (`lib.rs:230`) | `graphics` | incompatible: OpenGL texture import |
| `Keys`, `KeysParseError` (`lib.rs:231`) | `input` | kept |
| `StandardListViewItem`, `TableColumn` (deprecated, `lib.rs:232-237`) | `items` | kept |
| `FilterModel`, `MapModel`, `Model`, `ModelError`, `ModelExt`, `ModelNotify`, `ModelPeer`, `ModelRc`, `ModelTracker`, `ReverseModel`, `SortModel`, `VecModel` (`lib.rs:238-241`) | `model` | kept |
| `StyledText`, `StyledTextFromMarkdownError` (`lib.rs:242-244`) | `styled_text` | kept |
| `Timer`, `TimerMode` (`lib.rs:245`) | `timers` | kept |
| `SelectBundledTranslationError`, `select_bundled_translation` (`lib.rs:246`) | `translations` | kept |
| `private_unstable_api` (`lib.rs:248`) | `api/rs/slint/private_unstable_api.rs` | kept in name; needs a replacement for `ensure_context` and the `native_widgets` glob |
| `run_event_loop`, `run_event_loop_until_quit` (`lib.rs:261,272`) | selector `with_platform` then `Platform::run_event_loop` and `set_event_loop_quit_on_last_window_closed` | kept in signature; the implementation changes |
| `spawn_local` (`lib.rs:362`) | `SlintContext::spawn_local` | kept; it needs an event-loop proxy |
| `include_modules!` (`lib.rs:376`), `init_translations!` (`lib.rs:419`), `update_all_translations` (`lib.rs:429`) | | kept |
| `mod platform` (`lib.rs:440-501`) | `i_slint_core::platform` plus renderer modules | incompatible, see 5.3 |
| `mod language` (`lib.rs:507-533`) | `i_slint_core::items` through macros over `for_each_builtin_structs!` and `for_each_enums!` | kept |
| `mod android` (`lib.rs:543`) | `android.rs` | incompatible |
| `mod wgpu_29`, `mod wgpu_30` (`lib.rs:556-671`) | `graphics::wgpu_29`, `graphics::wgpu_30` | incompatible |
| `mod winit_030` (`lib.rs:674-745`) | `i_slint_backend_winit` | incompatible |
| `mod fontique_012` (`lib.rs:747-834`) | `SlintContext::font_context` and `font_collection::supply` | incompatible: it exposes the parley font collection |
| `VersionCheck_1_19_0` (`lib.rs:548`) | `lib.rs` | kept; the generator emits it |

### 5.2 `i_slint_core::api::*`

| Item | Status |
|---|---|
| `LogicalPosition`, `PhysicalPosition`, `LogicalSize`, `PhysicalSize`, `WindowPosition`, `WindowSize` | kept |
| `Window` | kept in part; see the list below |
| `CloseRequestResponse` | kept |
| `Global`, `StrongHandle`, `ComponentHandle`, `Weak` (and the `weak_handle` module) | kept; the generated code implements them |
| `invoke_from_event_loop`, `quit_event_loop`, `EventLoopError` | kept; they call `platform::with_event_loop_proxy` (`api.rs:1303,1320`) |
| `PlatformError` | kept; the variant `SetPlatformError(platform::SetPlatformError)` ties it to the `platform` module (`api.rs:1377,1411`) |
| `set_xdg_app_id` | not meaningful on Windows |
| `GraphicsAPI`, `RenderingState`, `RenderingNotifier`, `SetRenderingNotifierError` | incompatible: renderer hooks, with wgpu variants (`api.rs:266-374`) |
| `WindowHandle` (feature `raw-window-handle-06`) | incompatible in its upstream form; kept since 2026-11-19, when the host started to give the HWND (`docs/recommendations-after-phase7.md`) |
| `Brush`, `Color`, `Gray8Pixel`, `Image`, `LoadImageError`, `OklchColor`, `Rgb8Pixel`, `Rgb565Pixel`, `Rgba8Pixel`, `RgbaColor`, `SharedPixelBuffer` (`api.rs:19-22`) | kept |
| `DataTransfer`, `Keys`, `SharedVector`, `SharedString`, `ToSharedString`, `format!` (`api.rs:16,23-25`) | kept |
| `future::*`, that is `JoinHandle` (`api.rs:18`) | kept; it needs `EventLoopProxy` (`future.rs:36`) |

- `Window` methods that stay: `show`, `hide`, `on_close_requested`, `request_redraw`, `scale_factor`, `position`, `set_position`, `size`, `is_fullscreen`, `set_fullscreen`, `is_maximized`, `set_maximized`, `is_minimized`, `set_minimized`, `has_active_animations`, `is_visible`.
- `Window::set_size` stays and only calls `WindowAdapter::set_size`; the `renderer().resize(..)` call is in the `WindowEvent::Resized` arm of `dispatch_event_with_result` (`api.rs:754`).
- `Window::set_rendering_notifier` (`api.rs:521-525`) and `Window::take_snapshot` (`api.rs:850-851`) call `renderer()` and are incompatible.
- `Window::dispatch_event`, `try_dispatch_event` and `dispatch_event_with_result` (`api.rs:631-786`) take `platform::WindowEvent` and return `platform::WindowEventDispatchResult`; they are the input path for the host and cannot keep the `platform` module path.
- `Window::window_handle` (`api.rs:825`) needs `raw-window-handle-06`. Since 2026-11-19 the facade has the feature and the host gives the HWND.
- `Window::set_virtual_keyboard` and `virtual_keyboard` (`api.rs:604,614`) stay in shape.

### 5.3 `slint::platform`

- `slint::platform` re-exports all of `i_slint_core::platform::*` and wraps `set_platform` (`lib.rs:440-451`).
- The re-exported items are the trait `Platform` (`platform.rs:28`, 14 methods: `create_window_adapter`, `run_event_loop`, `process_events`, `bind_context`, `set_event_loop_quit_on_last_window_closed`, `new_event_loop_proxy`, `duration_since_start`, `click_interval`, `cursor_flash_cycle`, `set_clipboard_text`, `clipboard_text`, `debug_log`, `open_url`, `long_press_interval`).
- It also re-exports `Clipboard` (`:173`), `EventLoopProxy` (`:189`), `SetPlatformError` (`:235`), `set_platform` (`:255`), `update_timers_and_animations` (`:285`), `duration_until_next_timer_update` (`:307`), `WindowEventDispatchResult` (`:323`), `WindowEvent` (`:362`), `InternalEventBox` (`:495`), `InternalEvent` (`:573`), `PointerEventButton`, `Key`, `PlatformError`, `Renderer`, `LayoutConstraints`, `WindowAdapter` and `WindowProperties`.
- The renderer sub-modules are `platform::femtovg_renderer`, `platform::skia_renderer` and `platform::software_renderer` (`lib.rs:455-501`).
- All of it is incompatible with the plan, but `WindowEvent`, `WindowEventDispatchResult`, `Clipboard`, `EventLoopProxy` and `SetPlatformError` are referenced by kept core modules; section 6 lists where.

## 6. Uses of the to-be-removed modules

- The modules are `renderer`, `item_rendering`, `partial_renderer`, `platform`, `textlayout` (including `textlayout/*`), and `font_collection`.
- Hits inside comments and doc comments are not listed; hits inside doctests and `#[cfg(test)]` code are marked as test code.
- A `.renderer()` hit means a call on `Rc<dyn WindowAdapter>`; it reaches the sealed trait `RendererSealed` without naming the module.
- A `.platform()` hit means a call on `SlintContext::platform()`, which returns `&dyn Platform`.
- `textlayout::sharedparley` and `font_collection` matter only with the `shared-parley` core feature, which is on by default; section 6c.7a lists what that feature gates.

### 6a. References from the generators

#### `internal/compiler/generator/rust.rs`

- `renderer`: lines 5160, 5170 and 5180 call `window_adapter_ref()?.renderer().register_font_from_path(..)`, `.register_font_from_memory(..)` and `.register_bitmap_font(..)` for `RegisterCustomFontByPath`, `RegisterCustomFontByMemory` and `RegisterBitmapFont`.
- `renderer`, indirect: line 5147 passes `&Rc<dyn WindowAdapter>` to `sp::Item::layout_info`, which text items answer with `window_adapter.renderer().text_size(..)`, `char_size`, `text_content_widths` and `font_metrics` (`items/text.rs`, see 6c.1).
- `renderer`, indirect: the builtin `item_font_metrics` and `ImplicitLayoutInfo` reach the same renderer methods.
- `item_rendering` and `partial_renderer`: no reference by name; every item struct that the generator instantiates embeds `CachedRenderingData` and the generated component derives `Default` over those structs.
- `platform`: no reference by name; `slint::PlatformError` is `api::PlatformError`, not a `platform` item.
- `platform`, indirect: `SlintContext::create_window_adapter` (`rust.rs:805`) calls `Platform::create_window_adapter`, and `SlintContext::run_event_loop` (`rust.rs:566`) calls `Platform::run_event_loop`.
- `platform`, indirect: `WindowInner::create_child_window_adapter` (`rust.rs:4748`), `show_native_popup_menu` (`:5073`), `supports_native_menu_bar` (`:5263,5314`), `setup_menubar` (`:5316`) and `set_text_input_focused` (`:5409`) go through `WindowAdapterInternal`.
- `textlayout`: no reference by name; `sp::WindowItem::resolved_default_font_size` (`rust.rs:5191`) ends in `textlayout::DEFAULT_FONT_SIZE` (`items.rs:1427`).
- `font_collection`: none.

#### `internal/compiler/generator/rust_live_preview.rs`

- No reference to any of the six modules; it uses `sp::WindowInner::from_pub(..).window_adapter()` and `sp::WindowAdapter` only.

#### `internal/compiler/generator/cpp.rs` and the C++ headers it relies on

- `cpp.rs:1150,1206` are guarded by the compiler feature `renderer-software` (embedded textures and bitmap fonts).
- `cpp.rs:5624,5636,5646` emit `window.register_font_from_path(..)`, `register_font_from_data(..)` and `register_bitmap_font(..)`, which call `api/cpp/lib.rs:169-205`, which call `window_adapter.renderer().register_*`.
- `cpp.rs:2155-2175` implements the `window_adapter` slot with `window().window_handle()`.
- `cpp.rs:2241` calls `slint_ensure_backend`, which creates a `Platform` through the backend selector; `cpp.rs:5309` calls `slint_context_color_scheme`.
- The C++ headers reach `platform` through `slint-platform.h` and `api/cpp/platform.rs` (`slint_platform_register`, `slint_window_adapter_new`, `Clipboard`), which are outside the six generators.

#### Other generators

- `generator/python.rs`, `generator/typescript.rs` and `generator/slint_sc.rs` do not reference `i_slint_core`; they are out of scope.

### 6b. References from the interpreter

| Module | References |
|---|---|
| `renderer` | `instance.rs:294` (`free_graphics_resources`), `instance.rs:406` (`set_window_adapter`), `eval.rs:2384` (`register_font_from_path`), `eval.rs:2580` (`register_shared_bitmap_font`, feature `internal-mcu-interpreter`) |
| `item_rendering` | none |
| `partial_renderer` | none |
| `platform` | none outside tests; test files `tests.rs:1157,1165`, `tests/window_adapter_creation_error.rs:11`, `tests/menu_native_popup.rs:7`; indirect through `SlintContext::create_window_adapter` (`instance.rs:396`), `WindowAdapter::internal(InternalToken)` (`instance.rs:295`, `eval.rs:2301`) and `i_slint_backend_selector::{with_platform, with_global_context}` (`api.rs:1546,2202,2209`) |
| `textlayout` | `eval.rs:2369` (`textlayout::sharedparley::fontique::Blob`) and `eval.rs:2370` (`SlintContext::font_context()`); test `tests.rs:1316` |
| `font_collection` | none |

### 6c. References from the kept modules of `internal/core`

- The scan covers every file of `internal/core` except `renderer.rs`, `item_rendering.rs`, `partial_renderer.rs`, `platform.rs`, `font_collection.rs`, `textlayout.rs` and `textlayout/`.
- `layout.rs`, `input.rs` (apart from three `.renderer()` and one `.platform()` hit), `graphics/image.rs` and its submodules (apart from the `font_context()` hit in `graphics/image/svg.rs`), `model*`, `properties*`, `callbacks.rs`, `animations.rs` (apart from one `.platform()` hit), `lengths.rs`, `accessibility.rs`, `item_focus.rs`, `translations.rs` and `sharedvector.rs` have no other hit.
- Each line below gives the file, then the symbol and the line numbers where it occurs.
- `lib.rs` hits are the `pub mod` declarations.
- `window.rs:28` imports `crate::renderer::Renderer` and `window.rs:144` declares `fn renderer(&self) -> &dyn Renderer` in the trait `WindowAdapter`, which is the seam itself.
- `window.rs:1975,1988,2066` take `&mut dyn ItemRenderer` in `WindowInner::draw_contents`, in its `post_render` closure and in `render_image_overlay` (drag and cursor image overlays).

#### 6c.1 `renderer`

7 kept files reference it, with 37 non-test line hits.

- `api.rs`: `.renderer()` at 525, 754, 851.
- `input.rs`: `.renderer()` at 1638, 1735, 2014.
- `item_tree.rs`: `.renderer()` at 251, 537, 566, 1178.
  - Test code only: `Renderer` at 1815, 1820, 1832, 1846, 3477; `crate::renderer` at 3477; `RendererSealed` at 3477.
- `items.rs`: `.renderer()` at 1749.
- `items/text.rs`: `.renderer()` at 234, 358, 583, 963, 977, 998, 1013, 1023, 1195, 1205, 1933, 2593, 2621, 2783.
- `window.rs`: `crate::renderer` at 28; `Renderer` at 28, 144; `.renderer()` at 828, 1939, 1943, 1980, 2059, 2139, 2441, 2567, 3167.
- `lib.rs`: `module declaration` at 61.

#### 6c.2 `item_rendering`

16 kept files reference it, with 161 non-test line hits.

- `graphics/boxshadow.rs`: `item_rendering` at 21, 117; `BorderRectLayout` at 21, 117.
- `graphics/boxshadowcache.rs`: `item_rendering` at 67; `ItemCache` at 67.
- `graphics/rendering_metrics_collector.rs`: `item_rendering` at 180; `ItemRenderer` at 180.
- `item_tree.rs`: test code only, `item_rendering` at 3581; `RenderString` at 3581.
- `items.rs`: `item_rendering` at 31, 75; `CachedRenderingData` at 31, 141, 233, 325, 339, 437, 454, 561, 582, 695, 733, 847, 865, 984, 1000, 1091, 1109, 1205, 1291, 1544, 1561, 1764, 1791, 1918, 1936, 2041, 2126, 2293; `RenderRectangle` at 31, 428, 1388; `RenderBorderRectangle` at 31, 543, 671, 1806; `ItemRenderer` at 75; `ItemRendererRef` at 75, 207, 300, 406, 521, 649, 813, 932, 1067, 1176, 1361, 1696, 1885, 2015, 2219.
- `items/component_container.rs`: `item_rendering` at 17; `CachedRenderingData` at 17, 44, 292; `RenderRectangle` at 17, 274; `ItemRendererRef` at 252.
- `items/drag_n_drop.rs`: `item_rendering` at 16; `ItemRenderer` at 16, 224, 398; `CachedRenderingData` at 16, 122, 249, 298, 423.
- `items/flickable.rs`: `ItemRendererRef` at 10, 264; `item_rendering` at 25; `CachedRenderingData` at 25, 132, 290.
- `items/image.rs`: `item_rendering` at 18, 19; `ItemRenderer` at 18, 133, 305; `CachedRenderingData` at 19, 46, 192, 217, 369; `RenderImage` at 19, 155, 327.
- `items/input_items.rs`: `ItemRendererRef` at 5, 255, 396, 776, 984, 1212; `item_rendering` at 14, 499; `CachedRenderingData` at 14, 55, 280, 287, 299, 421, 432, 546, 801, 827, 1007, 1064, 1235; `ItemRenderer` at 499.
- `items/path.rs`: `ItemRendererRef` at 12, 131; `item_rendering` at 19; `CachedRenderingData` at 19, 62, 225.
- `items/system_tray.rs`: `item_rendering` at 18, 422; `CachedRenderingData` at 18, 174, 444; `ItemRenderer` at 422.
- `items/text.rs`: `item_rendering` at 22, 567, 568; `ItemRenderer` at 23, 140, 484, 877, 1649; `CachedRenderingData` at 23, 66, 165, 263, 509, 803, 902, 1141, 1684; `RenderString` at 23, 182, 526, 919, 970, 997, 1701, 1934; `RenderText` at 23, 199, 540, 929, 987; `HasFont` at 23, 169, 513, 906, 1688; `PlainOrStyledText` at 23, 183, 184, 527, 528, 920, 921, 974, 1702, 1706; `TextForWidth` at 24, 1006, 1202; `resolve_line_height_factor` at 24, 187, 1709; `TextSelection` at 567, 568.
- `menus.rs`: `item_rendering` at 9, 311; `CachedRenderingData` at 9, 236, 335; `ItemRenderer` at 311.
- `window.rs`: `item_rendering` at 1975, 1988, 2066; `ItemRenderer` at 1975, 1988, 2066.
- `lib.rs`: `module declaration` at 50.

#### 6c.3 `partial_renderer`

5 kept files reference it, with 2 non-test line hits.

- `graphics/rendering_metrics_collector.rs`: `partial_renderer` at 35; `DirtyRegion` at 35.
- `item_tree.rs`: test code only, `partial_renderer` at 3502; `DirtyRegion` at 3502.
- `tests/font_query_bench.rs`: test code only, `RepaintBufferType` at 21, 33.
- `tests/image_pixel_formats.rs`: test code only, `RepaintBufferType` at 9, 78.
- `lib.rs`: `module declaration` at 58.

#### 6c.4 `platform`

19 kept files reference it, with 93 non-test line hits.

- `animations.rs`: `.platform()` at 241.
- `api.rs`: `crate::platform` at 11, 631, 644, 659, 670, 675, 685, 695, 706, 715, 724, 732, 740, 748, 752, 760, 770, 774, 775, 780, 783, 786, 1303, 1320, 1377; `WindowEvent` at 11, 631, 644, 659, 660, 670, 675, 685, 695, 706, 715, 721, 724, 732, 740, 748, 750, 752, 758, 760, 762, 765, 767, 770, 772, 774, 775, 778, 780, 783, 786; `EventLoopProxy` at 1303, 1320; `SetPlatformError` at 1377, 1411.
- `context.rs`: `crate::platform` at 11, 22, 62, 578, 580, 584, 599, 600, 618, 619; `Platform` at 11, 46, 69, 197, 276, 572; `EventLoopProxy` at 11, 312, 578, 599; `WindowEvent` at 11, 22; `.platform()` at 207, 291, 313, 326, 458; `update_timers_and_animations` at 368, 378; `SetPlatformError` at 579, 580, 584, 600.
  - Test code only: `Platform` at 662; `EventLoopProxy` at 672, 679; `update_timers_and_animations` at 707, 720, 727, 747, 757, 758.
- `cursor.rs`: `Platform` at 43.
- `future.rs`: `crate::platform` at 36; `EventLoopProxy` at 36.
- `input.rs`: `.platform()` at 1406.
- `item_tree.rs`: test code only, `crate::platform` at 1846, 3498.
- `items.rs`: `.platform()` at 1626.
- `items/system_tray.rs`: `crate::platform` at 78.
- `items/system_tray/appkit.rs`: `crate::platform` at 204.
- `items/system_tray/ksni.rs`: `crate::platform` at 277.
- `items/system_tray/windows.rs`: `crate::platform` at 111, 136, 151, 361, 452.
- `items/text.rs`: `crate::platform` at 28; `Clipboard` at 28, 446, 606, 612, 718, 1292, 1313, 2462, 2468, 2486, 2493; `.platform()` at 621, 2481, 2497.
- `lib.rs`: `.platform()` at 215; `module declaration` at 59.
- `testing.rs`: `crate::platform` at 11.
- `tests/font_collection.rs`: test code only, `crate::platform` at 10, 116, 120, 131, 154; `Platform` at 10, 20; `SetPlatformError` at 116, 120, 131, 154.
- `tests/font_query_bench.rs`: test code only, `crate::platform` at 20, 23, 118; `Platform` at 23, 38; `SetPlatformError` at 118.
- `tests/image_pixel_formats.rs`: test code only, `crate::platform` at 9, 10, 17, 53, 79; `SetPlatformError` at 79.
- `window.rs`: `.platform()` at 1562; `crate::platform` at 3354, 3376, 3384; `WindowEvent` at 3354, 3376, 3384.

#### 6c.5 `textlayout`

7 kept files reference it, with 19 non-test line hits.

- `context.rs`: `textlayout` at 175, 253, 300; `sharedparley` at 175, 253, 300; `.font_context()` at 175, 248, 249, 254, 298, 301; `swash_scale_context` at 177, 258, 306, 307.
- `graphics/image/svg.rs`: `.font_context()` at 114, 115.
- `items.rs`: `textlayout` at 1427.
- `items/text.rs`: `textlayout` at 365, 380; `sharedparley` at 365, 380.
- `tests/font_collection.rs`: test code only, `.font_context()` at 42, 66, 85, 89, 90, 92, 133, 157.
- `window.rs`: `.font_context()` at 1985.
- `lib.rs`: `module declaration` at 70.

#### 6c.6 `font_collection`

3 kept files reference it, with 3 non-test line hits.

- `context.rs`: `font_collection` at 232, 583.
- `tests/font_collection.rs`: test code only, `font_collection` at 114, 130, 151, 152, 169.
- `lib.rs`: `module declaration` at 45.

- `context.rs:583` calls `font_collection::prefetch()` while the global context is created; it starts a worker thread named `slint-font-collection` that enumerates system fonts, and `context.rs:232` later joins it through `take_or_create()` (`font_collection.rs`).
- The same module offers `font_collection::supply(..)`, which `slint::fontique_012::set_shared_collection` calls (`api/rs/slint/lib.rs:832`).

#### 6c.7 Other graphics modules with a renderer flavor

- `graphics.rs:52,54,57` declares `rendering_metrics_collector`, `boxshadow` and `boxshadowcache`; the plan removes the first and the last, and `items.rs:1903` calls `crate::graphics::boxshadow::drop_shadow_bounding_rect` from `BoxShadow::bounding_rect`.
- `graphics/rendering_metrics_collector.rs:35,180` uses `partial_renderer::DirtyRegion` and `item_rendering::ItemRenderer`.
- `graphics.rs:62-65,193-198,218-253`, `graphics/image.rs:74,507,541-560`, `graphics/image/cache.rs:30`, `api.rs:283-325` and `window.rs:3157-3160` carry `wgpu-29` and `wgpu-30` variants (`GraphicsAPI`, `ImageInner::WGPU*Texture`).
- `graphics/bitmapfont.rs` is re-exported by `graphics::*` as `sp::BitmapFont`, and the test-only `RendererSealed` impl at `item_tree.rs:3506` names `graphics::BitmapFont`.
- `graphics.rs:36-37,124-165` has `FontRequest::query_fontique`, behind `shared-fontique`; `FontRequest` itself is the argument of `RendererSealed::font_metrics` and `text_line_height`.
- `lib.rs:21,28` and `platform.rs:15` use `unsafe_single_threaded`, which is the no_std path the plan drops.

#### 6c.7a Cargo features that tie kept code to the text stack

- The core feature `shared-parley` is on by default and pulls `parley`, `skrifa`, `icu_normalizer` and `shared-fontique`.
- It gates `SlintContext::font_context` and `swash_scale_context` (`context.rs:174-177,231-258,297-307`), the link and selection code of `StyledTextItem` and `Text` (`items/text.rs:265-413,648,731,760`), the font-context `begin_frame` and text-layout-cache calls in `draw_contents` (`window.rs:1979-1985`), and SVG text font lookup (`graphics/image/svg.rs:103-130`).
- It also gates NFC normalization of key text with `icu_normalizer` in `input.rs:732` and `window.rs:1378`, so dropping the feature silently drops that normalization unless the dependency is kept.
- `shared-fontique` gates `FontRequest::query_fontique` (`graphics.rs:36-37,124-165`) and is implied by `std`.
- `RendererSealed::text_layout_cache`, `text_size`, `char_size` and `text_content_widths` have two bodies each, selected by `shared-parley` (`renderer.rs:45-135`).

#### 6c.8 The `Platform` trait methods that kept code calls

| Call | Site | `Platform` method |
|---|---|---|
| `ctx.platform().bind_context(..)` | `context.rs:207` | `bind_context` |
| `self.platform().create_window_adapter()` | `context.rs:291` | `create_window_adapter` |
| `self.platform().new_event_loop_proxy()` | `context.rs:313` | `new_event_loop_proxy` |
| `self.platform().run_event_loop()` | `context.rs:326` | `run_event_loop` |
| `self.platform().debug_log(..)` | `context.rs:458` | `debug_log` |
| `ctx.platform().duration_since_start()` | `animations.rs:241` | `duration_since_start` |
| `ctx.platform().open_url(url)` | `lib.rs:215` | `open_url` |
| `ctx.platform().long_press_interval(..)` | `items.rs:1626` | `long_press_interval` |
| `ctx.platform().cursor_flash_cycle()` | `window.rs:1562` | `cursor_flash_cycle` |
| `ctx.platform().click_interval()` | `input.rs:1406` | `click_interval` |
| `.platform()` for clipboard text | `items/text.rs:621,2481,2497` | `clipboard_text`, `set_clipboard_text` |
| `set_platform`, `with_event_loop_proxy` | `context.rs:578-584,599`, `api.rs:1303,1320` | free functions of `platform.rs` |
| `SlintContext::update_timers_and_animations`, `duration_until_next_timer_update` | `context.rs:368,378` | the free functions `platform::update_timers_and_animations` and `duration_until_next_timer_update` (`platform.rs:285,307`) call these methods, so the dependency points from `platform` to `context` |

#### 6c.9 The `RendererSealed` methods that kept code calls

| Method | Callers (all `.renderer()` sites of 6c.1) |
|---|---|
| `supports_transformations` | `input.rs:1638,1735,2014`, `items.rs:1749`, `item_tree.rs:537,566,1178` |
| `font_metrics` | `items/text.rs:234,583,963,2783` |
| `text_size`, `text_content_widths`, `char_size`, `text_line_height` | `items/text.rs:977,998,1013,1023,1195,1205,1933` |
| `text_input_cursor_rect_for_byte_offset`, `text_input_byte_offset_for_position` | `items/text.rs:2593,2621` |
| `text_layout_cache`, `text_has_parley_layout` | `items/text.rs:358-387`, `window.rs:1980` |
| `mark_dirty_region` | `window.rs:1939,1943,2059,2441,2567` |
| `set_window_adapter` | `window.rs:828`, `instance.rs:406` of the interpreter |
| `resize` | `window.rs:2139`, `api.rs:754` |
| `free_graphics_resources` | `item_tree.rs:251`, `instance.rs:294` of the interpreter |
| `set_rendering_notifier`, `take_snapshot` | `api.rs:525,851`, `window.rs:3167` |
| `register_font_from_path`, `register_font_from_memory`, `register_bitmap_font`, `register_shared_bitmap_font` | generated code, interpreter and `api/cpp/lib.rs`, not kept core |

#### 6c.10 `ItemRenderer` calls made by the `render` of the items

- The `render` of a native item calls an `ItemRenderer` method and, where the method caches, passes `&self.cached_rendering_data`: `draw_rectangle`, `draw_border_rectangle`, `draw_image`, `draw_text`, `draw_text_input`, `draw_path`, `draw_box_shadow`, `draw_backdrop_blur`, `draw_window_background`, `visit_clip`, `visit_opacity` and `visit_layer`.
- `Transform::render` (`items.rs:1181-1184`) calls `translate`, `scale` and `rotate`; `Path::render` (`items/path.rs:137-142`) calls `save_state`, `combine_clip` and `restore_state`; `Flickable::render` (`items/flickable.rs:268`) calls `combine_clip`; `TextInput::render` reads `backend.window()` (`items/text.rs:1654-1655`).
- There are 32 `fn render(` definitions in `items.rs`, `items/*.rs` and `menus.rs`; `window.rs:1971-2066` (`draw_contents`) is the only caller that holds a `&mut dyn ItemRenderer`.

## 7. Most surprising couplings

- The generated Rust code reaches the renderer: `rust.rs:5160,5170,5180` call `window_adapter.renderer().register_font_*` and `register_bitmap_font`, and the interpreter and the C++ API do the same, so `WindowAdapter::renderer()` cannot simply disappear.
- Text measurement is not in the renderer module but in the items: `Item::layout_info(.., &WindowAdapterRc, ..)` of every text item calls `window_adapter.renderer().text_size`, `char_size`, `font_metrics`, `text_content_widths`, which the generator invokes at `rust.rs:5147` and the builtin `item_font_metrics` invokes through `text_item_font_metrics`.
- `ItemVTable` mentions the seam in ten of its twelve members: eight methods take `&WindowAdapterRc` (`deinit`, `layout_info`, `input_event_filter_before_children`, `input_event`, `focus_event`, `capture_key_event`, `key_event`, `bounding_rect`), `render` takes `&mut ItemRendererRef`, and the slot `cached_rendering_data_offset` points at the renderer's cache handle, so editing any of them changes a vtable that the generated code, the interpreter and the C++ layout all fix.
- `CachedRenderingData` is an ABI type: two `Cell<usize>` fields redefined by hand in C++ (`api/cpp/cbindgen.rs:1003`), embedded in all 32 items, never named by the generators, and defined in `partial_renderer.rs`, a module the plan deletes.
- `register_item_tree` and `unregister_item_tree` are called by generated code (`rust.rs:2834,2729`) and by the interpreter's `Drop for Instance` (`instance.rs:262-310`), which duplicates core logic; `register_item_tree` calls `WindowAdapterInternal::register_item_tree`, and `unregister_item_tree` calls `renderer().free_graphics_resources`, `WindowAdapterInternal::unregister_item_tree` and `WindowInner::{active_popups, close_popup}`.
- `platform.rs` holds types that kept code cannot do without: `WindowEvent`, `WindowEventDispatchResult` and `InternalEvent` are the argument types of `Window::dispatch_event*` (`api.rs:631-786`), `Clipboard` is used by `items/text.rs`, and `EventLoopProxy` and `SetPlatformError` are used by `api.rs`, `future.rs` and `context.rs`.
- `SlintContext` is hard-wired to the `Platform` trait object, the parley `FontContext` (`textlayout::sharedparley`), the process-wide `font_collection` and a swash `ScaleContext` (`context.rs:11,175-177,232,248-258`); its public constructors are `SlintContext::new(Box<dyn Platform>)` (`context.rs:197`) and `new_sharing_platform(&self)` (`context.rs:223`).
- The platform leaks into non-window code: `animations.rs:241` takes the animation clock from `Platform::duration_since_start`, and `items.rs:1626`, `window.rs:1562` and `input.rs:1406` read long-press, cursor-blink and double-click intervals from it.
- SVG loading depends on the font engine: `graphics/image/svg.rs:103-126` resolves SVG text fonts through `SlintContext::font_context()`, so `graphics/image` is not free of the text stack under `shared-parley`.
- `shared-parley`, a default feature, also decides whether key text is NFC-normalized (`input.rs:732`, `window.rs:1378`), so removing the parley stack changes input behavior as a side effect.
- `private_unstable_api.rs` is not in the core and cannot be used unchanged: `ensure_context` calls `i_slint_backend_selector::with_global_context`, `re_exports` globs `i_slint_backend_selector::native_widgets::*`, and `live_preview` pulls in `i_slint_live_preview`; the fork needs its own `slint` facade.
- `re_exports` is a glob-built namespace, so `sp::Transform` and `sp::ApproxEq` depend on explicit imports that win over globs, and `sp::ItemRc` depends on one `pub use` inside `items.rs:33`.
- Six kinds of names are built at compiler run time, so a grep for `sp::` misses them: the item classes, the builtin enums, the builtin structs, the `*_erased` helpers, the extra layout functions and the builtin globals.
- `ItemTreeVTable_static!` is expanded after a glob import of the whole `re_exports`, so `vtable` and `ItemTreeVTable` must stay nameable from it.
- `cbindgen` follows the `mod` tree of `internal/core` from `lib.rs`, and `cbindgen.rs:920-934` names `item_rendering.rs` and `../renderers/software/lib.rs` explicitly, so the C++ headers fail to generate if those files vanish without a matching edit.
- `internal/common/builtin_functions.rs` is a macro table whose Rust bodies are compiled into `i_slint_core::builtin_functions`; 13 of its 79 bodies call `with_window_adapter`, and `open_url` calls `Platform::open_url`, so the "runtime function" list is partly a window and platform contract.
- The interpreter uses the core only through public paths plus `rtti`, which makes `#[derive(SlintElement)]` and the literal field type spelling `Property<..>` / `Callback<..>` part of the contract.
