// Copyright © Kentaro Inomata
// SPDX-License-Identifier: MIT-0

//! What happens to a composition when the focus leaves its text input: as in TSF, a focus move
//! commits it into the input it was typed in, and deactivating the window keeps it.
//!
//! The composition events are dispatched as the host's input method module dispatches them.

use i_slint_core::input::{InternalKeyEvent, KeyEventType};
use i_slint_core::platform::WindowEvent;
use slint::ComponentHandle;

slint::include_modules!();

fn compose(ui: &ImeTest, preedit: &str) {
    let event = InternalKeyEvent {
        event_type: KeyEventType::UpdateComposition,
        preedit_text: preedit.into(),
        preedit_selection: Some(preedit.len() as i32..preedit.len() as i32),
        ..Default::default()
    };
    ui.window().dispatch_event(WindowEvent::internal(event));
}

fn commit(ui: &ImeTest, text: &str) {
    let mut event =
        InternalKeyEvent { event_type: KeyEventType::CommitComposition, ..Default::default() };
    event.key_event.text = text.into();
    ui.window().dispatch_event(WindowEvent::internal(event));
}

/// One test: the platform belongs to the thread that first uses it.
#[test]
fn compositions_and_focus() {
    let ui = ImeTest::new().unwrap();
    ui.window().dispatch_event(WindowEvent::WindowActiveChanged(true));

    // A focus move commits the composition into the input it was typed in.
    ui.invoke_focus_first();
    compose(&ui, "にほんご");
    assert_eq!(ui.get_first_text(), "", "a composition isn't text yet");
    ui.invoke_focus_second();
    assert_eq!(ui.get_first_text(), "にほんご");
    assert_eq!(ui.get_second_text(), "");

    // Deactivating the window keeps the composition; the input method continues it after.
    compose(&ui, "かな");
    ui.window().dispatch_event(WindowEvent::WindowActiveChanged(false));
    assert_eq!(ui.get_second_text(), "", "deactivating commits nothing");
    ui.window().dispatch_event(WindowEvent::WindowActiveChanged(true));
    commit(&ui, "仮名");
    assert_eq!(ui.get_second_text(), "仮名", "the composition was kept, not committed twice");

    // A composition that the input method commits is not committed again by a focus move.
    ui.invoke_focus_first();
    assert_eq!(ui.get_first_text(), "にほんご");
    assert_eq!(ui.get_second_text(), "仮名");
}
