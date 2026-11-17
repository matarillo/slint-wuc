//! Shows `ui/basics.slint`.
//!
//! With `--check`, it changes properties from timers, prints the visual tree after each step,
//! and quits: a smoke test of generated code on this runtime.

slint::include_modules!();

use slint::ComponentHandle;
use std::time::Duration;

fn host(app: &App) -> std::rc::Rc<i_slint_core::host::Win32WindowAdapter> {
    i_slint_core::host::Win32WindowAdapter::from_window(app.window())
        .expect("the window is created by this runtime's host")
}

fn main() {
    let app = App::new().unwrap();
    if std::env::args().any(|a| a == "--check") {
        let weak = app.as_weak();
        type Step = (&'static str, fn(&App));
        let steps: Vec<Step> = vec![
            ("initial", |_| {}),
            ("toggled", |app| app.set_toggled(true)),
            ("count = 5", |app| app.set_count(5)),
            ("count = 1", |app| app.set_count(1)),
        ];
        let step = std::cell::Cell::new(0);
        let timer = slint::Timer::default();
        timer.start(slint::TimerMode::Repeated, Duration::from_millis(200), move || {
            let app = weak.upgrade().unwrap();
            let i = step.get();
            if i > 0 {
                // The previous step's change has been synced by now.
                let host = host(&app);
                println!(
                    "--- {} ({} sync passes so far)\n{}",
                    steps[i - 1].0,
                    host.sync_passes(),
                    host.dump_visuals()
                );
            }
            if i == steps.len() {
                slint::quit_event_loop().unwrap();
                return;
            }
            (steps[i].1)(&app);
            step.set(i + 1);
        });
        app.run().unwrap();
    } else {
        app.run().unwrap();
    }
}
