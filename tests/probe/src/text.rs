// Copyright © Kentaro Inomata
// SPDX-License-Identifier: MIT-0

//! Shows `ui/text.slint`; see `harness.rs` for `--check`.

slint::include_modules!();

mod harness;

fn main() {
    harness::run(
        Text::new().unwrap(),
        vec![
            ("initial", |_| {}),
            ("new text", |t| t.set_greeting("Changed".into())),
            ("new color", |t| t.set_ink(slint::Color::from_rgb_u8(200, 0, 0))),
        ],
    );
}
