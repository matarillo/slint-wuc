// Copyright © Kentaro Inomata
// SPDX-License-Identifier: MIT-0

//! Fluent widgets over Mica, with the content extended into the title bar, as WinUI 3's
//! `ExtendsContentIntoTitleBar` does.
//!
//! This uses the fork's `slint::wuc`, which upstream Slint doesn't have, so it builds only on the
//! fork. DWM keeps drawing the caption buttons. A `WindowMoveArea` moves the window, and the
//! search box on top of it stays clickable. The window's background is transparent, so that Mica
//! shows through the title bar.

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use slint::ComponentHandle;
use slint::wuc::{TitleBar, WucWindowExt};
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Dwm::{
    DWM_SYSTEMBACKDROP_TYPE, DWMSBT_MAINWINDOW, DWMWA_SYSTEMBACKDROP_TYPE, DwmSetWindowAttribute,
};

slint::slint! {
    #[edition = "1.19"]
    import { Button, CheckBox, GroupBox, LineEdit, Palette, Slider, Switch, VerticalBox,
        HorizontalBox } from "std-widgets.slint";

    export component App inherits Window {
        // From the title bar: zero until the window extends into it.
        in property <length> title-bar-height;
        in property <length> buttons-width;
        in property <string> info;

        title: "Title bar";
        preferred-width: 560px;
        preferred-height: 380px;
        // Transparent, so that Mica shows through the title bar and between the widgets.
        background: transparent;

        VerticalLayout {
            // The title bar: a move area, with the title and a search box on top of it.
            Rectangle {
                height: root.title-bar-height;
                WindowMoveArea { }
                HorizontalLayout {
                    padding-left: 16px;
                    // Leaves the caption buttons alone.
                    padding-right: root.buttons-width + 16px;
                    spacing: 12px;
                    // A text takes no clicks, so a press on it moves the window.
                    Text {
                        text: root.title;
                        vertical-alignment: center;
                        color: Palette.foreground;
                        horizontal-stretch: 1;
                    }
                    VerticalLayout {
                        alignment: center;
                        LineEdit {
                            width: 200px;
                            placeholder-text: "Search";
                        }
                    }
                }
            }

            VerticalBox {
                alignment: start;
                GroupBox {
                    title: "Widgets below the title bar";
                    VerticalBox {
                        alignment: start;
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
                    text: "Drag the title bar to move the window. A double click on it maximizes, and a right click opens the system menu. The search box takes clicks as any widget does.";
                    wrap: word-wrap;
                    color: Palette.foreground;
                }
                Text {
                    text: root.info;
                    color: Palette.foreground;
                    opacity: 0.7;
                }
            }
        }
    }
}

/// Sets Mica as the window's system backdrop, through its HWND.
fn set_mica(window: &slint::Window) -> Result<(), String> {
    let handle = window.window_handle();
    let handle = handle.window_handle().map_err(|e| format!("no window handle: {e}"))?;
    let RawWindowHandle::Win32(win32) = handle.as_raw() else {
        return Err("not a Win32 window".into());
    };
    let backdrop = DWMSBT_MAINWINDOW;
    unsafe {
        DwmSetWindowAttribute(
            HWND(win32.hwnd.get() as _),
            DWMWA_SYSTEMBACKDROP_TYPE,
            (&backdrop as *const DWM_SYSTEMBACKDROP_TYPE).cast(),
            size_of::<DWM_SYSTEMBACKDROP_TYPE>() as u32,
        )
    }
    .map_err(|e| format!("DwmSetWindowAttribute: {e}"))
}

fn describe(title_bar: Option<TitleBar>) -> String {
    match title_bar {
        Some(t) => format!(
            "Title bar: {} px high; caption buttons: {} x {} px at ({}, {})",
            t.height,
            t.buttons_size.width,
            t.buttons_size.height,
            t.buttons_position.x,
            t.buttons_position.y
        ),
        None => "The content doesn't extend into the title bar.".into(),
    }
}

fn main() {
    let app = App::new().unwrap();
    // The window reports the title bar once it is shown, and again when it changes: at another
    // DPI, maximized, or when the system's metrics change.
    let weak = app.as_weak();
    app.window().on_title_bar_changed(move |title_bar| {
        let Some(app) = weak.upgrade() else { return };
        app.set_title_bar_height(title_bar.map_or(0., |t| t.height));
        app.set_buttons_width(title_bar.map_or(0., |t| t.buttons_size.width));
        app.set_info(describe(title_bar).into());
    });
    app.window().set_extends_content_into_title_bar(true);
    app.show().unwrap();
    // This fork creates the HWND in `show()`, so Mica needs no retry.
    if let Err(e) = set_mica(app.window()) {
        eprintln!("Mica: {e}");
    }
    app.run().unwrap();
}
