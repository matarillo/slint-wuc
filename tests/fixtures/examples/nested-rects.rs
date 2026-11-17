// SPDX-License-Identifier: MIT OR Apache-2.0

//! Opens a real window with `NestedRects` and changes the color of one rectangle every second.
//!
//! This is for the manual check of the Phase 1 gate: resize the window (the window background
//! follows, the rectangles keep their size), and drag it between monitors of different density
//! (the rectangles keep their logical size and stay sharp). Closing the window ends the event
//! loop.
//!
//! ```text
//! cargo run -p slint-fork-fixtures --example nested-rects
//! ```

use i_slint_core::api::LogicalSize;
use i_slint_core::host::Platform;
use i_slint_core::item_tree::ItemTreeRc;
use i_slint_core::timers::{Timer, TimerMode};
use i_slint_core::window::WindowInner;
use i_slint_core::{Brush, Color, SlintContext};
use slint_fork_fixtures::NestedRects;
use slint_fork_fixtures::nested_rects::Rect;
use std::rc::Rc;
use std::time::Duration;
use vtable::VRc;

/// The colors `RightInner` goes through, one per second.
const COLORS: [(u8, u8, u8); 4] =
    [(0x30, 0xa0, 0x30), (0xe0, 0x60, 0x20), (0x80, 0x30, 0xc0), (0xf0, 0xf0, 0xf0)];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // As the facade will: this also installs the process-wide event loop proxy that
    // `quit_event_loop` uses.
    i_slint_core::platform::set_platform(Box::new(Platform::new()?))?;
    let ctx = SlintContext::current().expect("set_platform created the context");
    let adapter = ctx.create_window_adapter()?;

    let tree = NestedRects::new(&adapter);
    let item_tree: ItemTreeRc = VRc::into_dyn(tree.clone());
    WindowInner::from_pub(adapter.window()).set_component(&item_tree);
    // The rectangles are laid out for 400x300 logical pixels.
    adapter.window().set_size(LogicalSize::new(400., 300.));
    adapter.window().show()?;

    let weak = VRc::downgrade(&tree);
    let window = Rc::downgrade(&adapter);
    let mut step = 0;
    let timer = Timer::default();
    timer.start(TimerMode::Repeated, Duration::from_secs(1), move || {
        let (Some(tree), Some(adapter)) = (weak.upgrade(), window.upgrade()) else { return };
        step = (step + 1) % COLORS.len();
        let (r, g, b) = COLORS[step];
        tree.as_pin_ref()
            .set_background(Rect::RightInner, Brush::SolidColor(Color::from_rgb_u8(r, g, b)));
        // What a manual check of resizing and DPI changes can compare against the screen.
        let window = adapter.window();
        let (size, scale) = (window.size(), window.scale_factor());
        println!(
            "RightInner is #{r:02x}{g:02x}{b:02x}; the window is {}x{} pixels at scale factor {scale}",
            size.width, size.height
        );
    });

    // Closing the window hides it, which releases the last keep-alive and quits the loop.
    ctx.run_event_loop()?;
    println!("the window was closed");
    Ok(())
}
