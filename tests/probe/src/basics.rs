// Copyright © Kentaro Inomata
// SPDX-License-Identifier: MIT-0

//! Shows `ui/basics.slint`; see `harness.rs` for `--check`.

slint::include_modules!();

mod harness;

fn main() {
    harness::run(
        App::new().unwrap(),
        vec![
            ("initial", |_| {}),
            ("toggled", |app| app.set_toggled(true)),
            ("count = 5", |app| app.set_count(5)),
            ("count = 1", |app| app.set_count(1)),
        ],
    );
}
