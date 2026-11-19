// Copyright © Kentaro Inomata
// SPDX-License-Identifier: MIT-0

//! The facade's `slint::wuc`: a window extends its content into the title bar, reports it from
//! the event loop, and stops.
//!
//! The window is shown, since the title bar is measured over the HWND that `show()` creates,
//! and for a moment only: the event loop runs just long enough to deliver the report.
//!
//! The platform belongs to the thread that first uses it, so one test runs every scenario.

use slint::ComponentHandle;
use slint::wuc::{TitleBar, WucWindowExt};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

slint::include_modules!();

/// Runs the event loop until the posted messages are through.
fn run_event_loop_briefly() {
    slint::Timer::single_shot(Duration::from_millis(100), || {
        slint::quit_event_loop().unwrap();
    });
    slint::run_event_loop().unwrap();
}

#[test]
fn a_window_extends_its_content_into_the_title_bar() {
    let ui = TitleBarTest::new().unwrap();
    let window = ui.window();
    assert!(!window.extends_content_into_title_bar());
    assert_eq!(window.title_bar(), None);

    let reports: Rc<RefCell<Vec<Option<TitleBar>>>> = Rc::default();
    let reports_ = reports.clone();
    window.on_title_bar_changed(move |title_bar| reports_.borrow_mut().push(title_bar));

    // Before `show()`: asked for, and waiting for the window to exist.
    window.set_extends_content_into_title_bar(true);
    assert!(window.extends_content_into_title_bar());
    assert_eq!(window.title_bar(), None);

    ui.show().unwrap();
    let title_bar = window.title_bar().expect("the shown window extends");
    assert!(title_bar.height > 0.);
    let scale = window.scale_factor();
    let width = window.size().to_logical(scale).width;
    let buttons_right = title_bar.buttons_position.x + title_bar.buttons_size.width;
    assert!(title_bar.buttons_size.width > 0. && title_bar.buttons_size.height > 0.);
    assert!(title_bar.buttons_position.x >= 0. && buttons_right <= width + 0.01);
    let buttons_bottom = title_bar.buttons_position.y + title_bar.buttons_size.height;
    assert!(buttons_bottom <= title_bar.height + 0.01);
    ui.set_title_bar_height(title_bar.height);

    run_event_loop_briefly();
    assert_eq!(*reports.borrow(), [Some(title_bar)]);

    // Stopped: the content is below the title bar again.
    window.set_extends_content_into_title_bar(false);
    assert!(!window.extends_content_into_title_bar());
    assert_eq!(window.title_bar(), None);
    run_event_loop_briefly();
    assert_eq!(*reports.borrow(), [Some(title_bar), None]);

    ui.hide().unwrap();
}
