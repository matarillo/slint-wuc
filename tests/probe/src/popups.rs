// Copyright © Kentaro Inomata
// SPDX-License-Identifier: MIT-0

//! Shows `ui/popups.slint`. With `--check`, opens the combo box with keys and prints the
//! visual tree of its popup window after each step.

slint::include_modules!();

use i_slint_core::host::Win32WindowAdapter;
use i_slint_core::window::{PopupWindowLocation, WindowInner};
use slint::ComponentHandle;
use slint::platform::{Key, WindowEvent};
use std::time::Duration;

/// Sends `key` where the host sends keys: to the topmost popup with a window of its own.
fn key(window: &slint::Window, key: Key) {
    let inner = WindowInner::from_pub(window);
    let popup = inner.active_popups().last().and_then(|p| match &p.location {
        PopupWindowLocation::TopLevel(adapter) => Some(adapter.clone()),
        PopupWindowLocation::ChildWindow(_) => None,
    });
    let window = popup.as_ref().map_or(window, |a| a.window());
    let text = slint::SharedString::from(key);
    window.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window.dispatch_event(WindowEvent::KeyReleased { text });
}

fn dump_popup(window: &slint::Window) -> String {
    let inner = WindowInner::from_pub(window);
    let popups = inner.active_popups();
    let Some(popup) = popups.last() else { return "no popup".into() };
    match &popup.location {
        PopupWindowLocation::TopLevel(adapter) => {
            let host = Win32WindowAdapter::from_window(adapter.window()).unwrap();
            format!("({} sync passes)\n{}", host.sync_passes(), host.dump_visuals())
        }
        PopupWindowLocation::ChildWindow(_) => "drawn inside the window".into(),
    }
}

type Step = (&'static str, fn(&Popups));

fn main() {
    let popups = Popups::new().unwrap();
    if !std::env::args().any(|a| a == "--check") {
        popups.run().unwrap();
        return;
    }
    let steps: Vec<Step> = vec![
        ("open", |p| {
            p.invoke_focus_combo();
            key(p.window(), Key::Return);
        }),
        ("down", |p| key(p.window(), Key::DownArrow)),
        ("down", |p| key(p.window(), Key::DownArrow)),
        ("up", |p| key(p.window(), Key::UpArrow)),
    ];
    let weak = popups.as_weak();
    let step = std::cell::Cell::new(0);
    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, Duration::from_millis(400), move || {
        let p = weak.upgrade().unwrap();
        let i = step.get();
        if i > 0 {
            println!(
                "--- {} (index {})\n{}",
                steps[i - 1].0,
                p.get_index(),
                dump_popup(p.window())
            );
        }
        if i == steps.len() {
            slint::quit_event_loop().unwrap();
            return;
        }
        (steps[i].1)(&p);
        step.set(i + 1);
    });
    popups.run().unwrap();
}
