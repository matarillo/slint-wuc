// Copyright © Kentaro Inomata
// SPDX-License-Identifier: MIT-0

//! Upstream's `slint!` macro, re-exported, builds a component that runs on this runtime.

slint::slint! {
    #[edition = "1.19"]
    import { Button } from "std-widgets.slint";
    export component App inherits Window {
        in-out property <int> counter: 0;
        callback bumped(int);
        VerticalLayout {
            Text { text: "count: " + root.counter; }
            Button { text: "+1"; clicked => { root.counter += 1; root.bumped(root.counter); } }
        }
    }
}

#[test]
fn the_macro_builds_a_component() {
    let app = App::new().unwrap();
    app.set_counter(41);
    assert_eq!(app.get_counter(), 41);

    let seen = std::rc::Rc::new(std::cell::Cell::new(0));
    let seen_ = seen.clone();
    app.on_bumped(move |n| seen_.set(n));
    app.invoke_bumped(42);
    assert_eq!(seen.get(), 42);
}
