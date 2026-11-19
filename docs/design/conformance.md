# Item Conformance

Generated code and the interpreter address items by the names the compiler knows.
A runtime item that drifts from them breaks only the build of a user's generated code, so `tests/conformance` compares them instead:
the crates.io `i-slint-compiler` 1.19.0, unmodified, on one side, and `i_slint_core::rtti` on the other.

- It checks both directions: every property of a native class is in the item, and every property and callback of the item is known to the compiler, so that the runtime doesn't grow fields the compiler never sets.
- It doesn't check types: generated code names each field with its type, so the probe samples (`tests/probe`) catch a type mismatch at build time, for the properties they use.
- The `Native*` classes of the Qt style are left out (`compat-contract.md`).
