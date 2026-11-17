//! Runs a probe: shows the component, or with `--check`, applies changes step by step from a
//! timer, prints the visual tree after each, and quits.
//!
//! Each step gets 200ms, so that animations of up to about 100ms finish within it.

use slint::ComponentHandle;
use std::rc::Rc;
use std::time::Duration;

pub type Step<C> = (&'static str, fn(&C));

pub fn run<C: ComponentHandle + 'static>(component: C, steps: Vec<Step<C>>) {
    if !std::env::args().any(|a| a == "--check") {
        component.run().unwrap();
        return;
    }
    let weak = component.as_weak();
    let step = std::cell::Cell::new(0);
    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, Duration::from_millis(200), move || {
        let component = weak.upgrade().unwrap();
        let i = step.get();
        if i > 0 {
            let host = host(component.window());
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
        (steps[i].1)(&component);
        step.set(i + 1);
    });
    component.run().unwrap();
}

fn host(window: &slint::Window) -> Rc<i_slint_core::host::Win32WindowAdapter> {
    i_slint_core::host::Win32WindowAdapter::from_window(window)
        .expect("the window is created by this runtime's host")
}
