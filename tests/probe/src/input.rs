// Copyright © Kentaro Inomata
// SPDX-License-Identifier: MIT-0

//! Shows `ui/input.slint`, the manual check of Phase 3; the file's header says what to try.

use slint::{Color, ModelRc, VecModel};
use std::rc::Rc;

slint::include_modules!();

fn main() {
    let input = Input::new().unwrap();

    let clicks = Rc::new(VecModel::<Color>::default());
    input.set_clicks(ModelRc::from(clicks.clone()));
    input.on_click(move |color| clicks.push(color));

    let keys = Rc::new(VecModel::<Color>::default());
    input.set_keys(ModelRc::from(keys.clone()));
    input.on_key(move |color| keys.push(color));

    input.run().unwrap();
}
