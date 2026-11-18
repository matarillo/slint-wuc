// SPDX-License-Identifier: MIT OR Apache-2.0

//! The scenarios: steps that each end with a screenshot.

use crate::app::{App, Result, keys, label, type_name};

pub type Step = (&'static str, fn(&mut App) -> Result);

pub struct Scenario {
    pub name: &'static str,
    pub steps: &'static [Step],
}

pub const ALL: &[Scenario] = &[Scenario {
    // The combo box on the Selection page: its list, then the keys in the list.
    name: "combo-box",
    steps: &[
        ("selection-page", |app| app.click(label("Selection"))),
        ("open", |app| app.click(type_name("ComboBox"))),
        ("down", |app| app.key(keys::DOWN)),
        ("down-again", |app| app.key(keys::DOWN)),
        ("up", |app| app.key(keys::UP)),
        ("escape", |app| app.key(keys::ESCAPE)),
    ],
}];
