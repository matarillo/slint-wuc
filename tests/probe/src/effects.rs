// Copyright © Kentaro Inomata
// SPDX-License-Identifier: MIT-0

//! Shows `ui/effects.slint`; see `harness.rs` for `--check`.

slint::include_modules!();

mod harness;

fn main() {
    harness::run(
        Effects::new().unwrap(),
        vec![
            ("initial", |_| {}),
            ("unclipped", |e| e.set_clipped(false)),
            ("opaque", |e| e.set_fade(1.)),
            ("unturned", |e| e.set_turn(0.)),
        ],
    );
}
