// Copyright © Kentaro Inomata
// SPDX-License-Identifier: MIT-0

//! Shows `ui/drop.slint`, which takes drags from other programs, and prints each drop.

slint::include_modules!();

use slint::ComponentHandle;

fn main() {
    let app = DropProbe::new().unwrap();
    app.on_describe(|data| {
        let mut lines = Vec::new();
        if let Ok(paths) = data.file_paths() {
            lines.extend(paths.map(|p| format!("file: {}", p.display())));
        }
        if let Ok(text) = data.plain_text() {
            lines.push(format!("text: {text:?}"));
        }
        if lines.is_empty() {
            lines.push("(no files and no text)".into());
        }
        lines.join("\n").into()
    });
    app.on_report(|text| println!("{text}\n"));
    app.run().unwrap();
}
