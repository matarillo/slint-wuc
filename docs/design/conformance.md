# Item Conformance

Generated code and the interpreter address items by the names the compiler knows.
The conformance test checks that this runtime's items match those names,
so that a mismatch fails a test instead of the build of a user's generated code.

## What It Compares

- The compiler side is the crates.io `i-slint-compiler` 1.19.0, unmodified:
  `TypeRegister::builtin()` and the `NativeClass` of each builtin element,
  following `NativeClass::parent`.
- The runtime side is `i_slint_core::rtti::BuiltinItem` with the `rtti` feature,
  for the items the upstream interpreter registers (`internal/interpreter/item_registry.rs`).
- For each native class, except the `Native*` classes of the Qt style:
  - this runtime has an item whose `BuiltinItem::name()` is the class name;
  - each property of the class (kebab-case, mapped to snake_case) is a property, a callback, or a field of that item;
  - each property and callback of the item is known to the compiler,
    so that the runtime doesn't grow fields the compiler never sets.
- Native globals (`BuiltinGlobal`) get the same check.

## What It Leaves to Other Tests

- Types: generated code names each field with its type, so the probe samples
  (`tests/probe`) catch a type mismatch at build time for the properties they use.
- Behavior: dump and pixel tests per item.

## Not Shown Yet

`tests/conformance` keeps the list of items whose `update_visual` doesn't show them yet,
with the phase that adds them.
The list is checked against the `TODO(phase N)` comments in `crates/core/items.rs`,
so that it can't drift.
