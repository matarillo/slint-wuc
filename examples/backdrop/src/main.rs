// Copyright © Kentaro Inomata
// SPDX-License-Identifier: MIT-0

//! Shows Windows 11's system backdrops (Mica, Mica Alt, Acrylic) behind Fluent widgets.
//!
//! Slint has no API for the backdrop, so the application sets it through the HWND that
//! `Window::window_handle()` gives, with `DwmSetWindowAttribute`. The window's background is
//! transparent, so that the backdrop shows where the widgets draw nothing. The same code builds
//! and runs on upstream Slint.

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use slint::ComponentHandle;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Dwm::{
    DWM_SYSTEMBACKDROP_TYPE, DWMSBT_MAINWINDOW, DWMSBT_NONE, DWMSBT_TABBEDWINDOW,
    DWMSBT_TRANSIENTWINDOW, DWMWA_SYSTEMBACKDROP_TYPE, DwmSetWindowAttribute,
};

slint::slint! {
    #[edition = "1.19"]
    import { Button, CheckBox, ComboBox, GroupBox, LineEdit, Palette, Slider, VerticalBox,
        HorizontalBox, Switch } from "std-widgets.slint";

    export component App inherits Window {
        in-out property <int> backdrop: 0;
        in property <string> status;
        callback backdrop-changed(int);

        title: "Backdrop";
        preferred-width: 560px;
        preferred-height: 420px;
        // Transparent where the widgets draw nothing, so the backdrop shows. Without a
        // backdrop, the window takes the style's opaque background again.
        background: root.backdrop == 3 ? Palette.background : transparent;

        ContextMenuArea {
            Menu {
                MenuItem { title: "Mica"; activated => { root.backdrop = 0; root.backdrop-changed(0); } }
                MenuItem { title: "Mica Alt"; activated => { root.backdrop = 1; root.backdrop-changed(1); } }
                MenuItem { title: "Acrylic"; activated => { root.backdrop = 2; root.backdrop-changed(2); } }
                MenuSeparator {}
                MenuItem { title: "None"; activated => { root.backdrop = 3; root.backdrop-changed(3); } }
            }

            VerticalBox {
                alignment: start;
                HorizontalBox {
                    padding: 0;
                    Text {
                        text: "System backdrop";
                        vertical-alignment: center;
                        font-size: 14px;
                    }
                    ComboBox {
                        model: ["Mica", "Mica Alt", "Acrylic", "None"];
                        current-index <=> root.backdrop;
                        selected => { root.backdrop-changed(self.current-index); }
                    }
                }

                GroupBox {
                    title: "Widgets over the backdrop";
                    VerticalBox {
                        alignment: start;
                        LineEdit { placeholder-text: "Type here"; }
                        HorizontalBox {
                            padding: 0;
                            CheckBox { text: "Check box"; checked: true; }
                            Switch { text: "Switch"; }
                        }
                        Slider { value: 40; }
                        HorizontalBox {
                            padding: 0;
                            Button { text: "Button"; }
                            Button { text: "Primary"; primary: true; }
                        }
                    }
                }

                Text {
                    text: "Right-click for the same choices in a context menu. Switch the app mode between light and dark in Settings to see the window follow.";
                    wrap: word-wrap;
                    color: Palette.foreground;
                }
                Text {
                    text: root.status;
                    color: Palette.foreground;
                    opacity: 0.7;
                }
            }
        }
    }
}

/// The backdrop for the `App.backdrop` index.
fn backdrop_type(index: i32) -> DWM_SYSTEMBACKDROP_TYPE {
    match index {
        0 => DWMSBT_MAINWINDOW,
        1 => DWMSBT_TABBEDWINDOW,
        2 => DWMSBT_TRANSIENTWINDOW,
        _ => DWMSBT_NONE,
    }
}

/// Sets the window's system backdrop through its HWND, which exists once the window is shown.
fn set_backdrop(window: &slint::Window, index: i32) -> Result<(), String> {
    let handle = window.window_handle();
    let handle = handle.window_handle().map_err(|e| format!("no window handle: {e}"))?;
    let RawWindowHandle::Win32(win32) = handle.as_raw() else {
        return Err("not a Win32 window".into());
    };
    let hwnd = HWND(win32.hwnd.get() as _);
    let value = backdrop_type(index);
    unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_SYSTEMBACKDROP_TYPE,
            (&value as *const DWM_SYSTEMBACKDROP_TYPE).cast(),
            size_of::<DWM_SYSTEMBACKDROP_TYPE>() as u32,
        )
    }
    .map_err(|e| format!("DwmSetWindowAttribute: {e}"))
}

/// Sets the backdrop and shows why it failed, if it did. Returns whether it was set.
fn apply(app: &App, index: i32) -> bool {
    let result = set_backdrop(app.window(), index);
    app.set_status(result.as_ref().err().cloned().unwrap_or_default().into());
    result.is_ok()
}

fn main() {
    let app = App::new().unwrap();
    // `--backdrop mica|mica-alt|acrylic|none` picks the first backdrop.
    let args: Vec<String> = std::env::args().collect();
    if let Some(name) = args.iter().position(|a| a == "--backdrop").and_then(|i| args.get(i + 1)) {
        let names = ["mica", "mica-alt", "acrylic", "none"];
        let index = names.iter().position(|n| n == name).expect("unknown backdrop");
        app.set_backdrop(index as i32);
    }
    let weak = app.as_weak();
    app.on_backdrop_changed(move |index| {
        if let Some(app) = weak.upgrade() {
            apply(&app, index);
        }
    });
    app.show().unwrap();
    // The HWND may exist only after an iteration of the event loop: this fork creates it in
    // `show()`, upstream's winit backend later. Try from the event loop until it's there.
    let timer = std::rc::Rc::new(slint::Timer::default());
    let (weak, weak_timer) = (app.as_weak(), std::rc::Rc::downgrade(&timer));
    let mut attempts = 0;
    timer.start(slint::TimerMode::Repeated, std::time::Duration::from_millis(16), move || {
        attempts += 1;
        let done = weak.upgrade().is_none_or(|app| apply(&app, app.get_backdrop()));
        if (done || attempts == 60)
            && let Some(timer) = weak_timer.upgrade()
        {
            timer.stop();
        }
    });
    app.run().unwrap();
}
