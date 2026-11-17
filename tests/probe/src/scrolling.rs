// SPDX-License-Identifier: MIT OR Apache-2.0

//! Shows `ui/scrolling.slint`; see `harness.rs` for `--check`.

slint::include_modules!();

mod harness;

fn main() {
    harness::run(
        Scrolling::new().unwrap(),
        vec![("initial", |_| {}), ("scrolled", |s| s.set_offset(-150.))],
    );
}
