// SPDX-License-Identifier: MIT OR Apache-2.0

//! The scenarios: steps that each end with a screenshot.

use crate::app::{App, Result, keys, label, type_name};
use std::time::Duration;

pub type Step = (&'static str, fn(&mut App) -> Result);

pub struct Scenario {
    pub name: &'static str,
    pub steps: &'static [Step],
}

/// Longer than a tooltip's delay.
const TOOLTIP_WAIT: Duration = Duration::from_millis(1500);

pub const ALL: &[Scenario] = &[
    Scenario {
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
    },
    Scenario {
        // The menu bar that Slint draws: a menu, its keys, and a submenu.
        name: "menu-bar",
        steps: &[
            ("open-file", |app| app.click(type_name("MenuBarItem"))),
            ("down", |app| app.key(keys::DOWN)),
            ("down-again", |app| app.key(keys::DOWN)),
            ("open-recent", |app| app.key(keys::RIGHT)),
            ("down-in-submenu", |app| app.key(keys::DOWN)),
            ("close-submenu", |app| app.key(keys::LEFT)),
            ("escape", |app| app.key(keys::ESCAPE)),
        ],
    },
    Scenario {
        // The tooltips on the Controls page: plain text, then custom content.
        name: "tooltip",
        steps: &[
            ("controls-page", |app| app.click(label("Controls"))),
            ("hover", |app| app.hover(label("Hover me"), TOOLTIP_WAIT)),
            ("hover-custom", |app| app.hover(label("Custom Tooltip"), TOOLTIP_WAIT)),
            ("leave", |app| app.hover(label("Controls"), TOOLTIP_WAIT)),
        ],
    },
    Scenario {
        // Typing into a line edit on the Input page, then its context menu.
        name: "line-edit",
        steps: &[
            ("input-page", |app| app.click(label("Input"))),
            ("focus", |app| app.click(type_name("LineEdit"))),
            ("type", |app| app.type_text(" typed")),
            ("select-all", |app| app.key_with(keys::CONTROL, "a")),
            ("context-menu", |app| app.right_click(type_name("LineEdit"))),
            ("down", |app| app.key(keys::DOWN)),
            ("escape", |app| app.key(keys::ESCAPE)),
        ],
    },
];
