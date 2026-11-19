// Copyright © Kentaro Inomata
// SPDX-License-Identifier: MIT-0

//! Specific to this runtime, Slint on Windows.UI.Composition.
//!
//! *Note*: This module is behind a feature flag and may change in any release, including with
//!         breaking changes.
//!         It doesn't exist in upstream Slint, so code that uses it doesn't build against it.
//!
//! Use the traits and types in this module for what Slint's own API doesn't cover.
//! For now, that is extending a window's content into its title bar, as WinUI 3's
//! `ExtendsContentIntoTitleBar` does.
//!
//! `Cargo.toml`:
//! ```toml
//! slint = { git = "https://github.com/matarillo/slint-wuc", features = ["unstable-wuc"] }
//! ```
//!
//! `main.rs`:
//! ```rust,no_run
//! use slint::ComponentHandle;
//! use slint::wuc::WucWindowExt;
//!
//! slint::slint! {
//!     #[edition = "1.19"]
//!     export component App inherits Window {
//!         in property <length> title-bar-height;
//!         in property <length> buttons-width;
//!         VerticalLayout {
//!             // Moves the window, and maximizes it on a double click.
//!             WindowMoveArea {
//!                 height: root.title-bar-height;
//!                 Text {
//!                     text: "My application";
//!                     x: 12px;
//!                     // Leaves the caption buttons alone, on the right.
//!                     width: parent.width - root.buttons-width - 12px;
//!                 }
//!             }
//!             Text { text: "The content below the title bar."; }
//!         }
//!     }
//! }
//!
//! fn main() -> Result<(), slint::PlatformError> {
//!     let app = App::new()?;
//!     let weak = app.as_weak();
//!     app.window().on_title_bar_changed(move |title_bar| {
//!         if let Some(app) = weak.upgrade() {
//!             app.set_title_bar_height(title_bar.map_or(0., |t| t.height));
//!             app.set_buttons_width(title_bar.map_or(0., |t| t.buttons_size.width));
//!         }
//!     });
//!     app.window().set_extends_content_into_title_bar(true);
//!     app.run()
//! }
//! ```
//!
//! # Extending the content into the title bar
//!
//! The window keeps its frame, but its content covers the title bar.
//! DWM keeps drawing the minimize, maximize and close buttons over the content, so Snap Layouts
//! stays on the maximize button.
//! Slint draws everything else, and the application lays out the title bar:
//!
//! - [`WucWindowExt::title_bar()`] gives the height of the title bar and where the buttons are.
//! - A `WindowMoveArea` is the drag region.
//!   While the window extends into the title bar, a double click on it maximizes or restores the
//!   window, and a right click opens the system menu.
//! - The `Window` item's background leaves a hole under the buttons.
//!   While it is an opaque solid color, the buttons take that color, so they don't stand out.
//!   Otherwise, such as with `background: transparent` for Mica, DWM draws them as it does in a
//!   window with a title bar.
//! - Other items drawn over the buttons hide them.
//!   Keep them out of the rectangle that `buttons_position` and `buttons_size` give.
//!
//! The content doesn't extend into the title bar of a window with `no-frame`, of a full-screen
//! window, or of a window with a native `MenuBar`, which Windows draws in the area that the
//! content would cover.
//! For a menu bar, set the `SLINT_NO_MUDA` environment variable, so that Slint draws the menu
//! bar itself.
//! A window that doesn't extend keeps its title bar, and [`WucWindowExt::title_bar()`] gives
//! `None`.

use crate::{LogicalPosition, LogicalSize, Window};
use i_slint_core::host::title_bar;

/// The title bar of a window whose content extends into it, in logical pixels of the content's
/// coordinates.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TitleBar {
    /// From the top of the content to the bottom of the caption buttons.
    pub height: f32,
    /// The top left corner of the caption buttons, which DWM draws over the content.
    pub buttons_position: LogicalPosition,
    /// The size of the caption buttons together.
    pub buttons_size: LogicalSize,
}

impl TitleBar {
    // Not a `From` impl, which would put the core's type in the public API.
    fn from_core(title_bar: title_bar::TitleBar) -> Self {
        let buttons = title_bar.buttons;
        Self {
            height: title_bar.height,
            buttons_position: LogicalPosition::new(buttons.origin.x, buttons.origin.y),
            buttons_size: LogicalSize::new(buttons.size.width, buttons.size.height),
        }
    }
}

/// Extends [`Window`] with what is specific to this runtime.
pub trait WucWindowExt {
    /// Lets the content of the window cover its title bar, or stops it.
    ///
    /// Only a framed top-level window that isn't full screen and has no native `MenuBar`
    /// extends; for other windows, the setting waits until the window is one.
    /// Call it before `show()` to have the window created that way.
    /// Called afterwards, it keeps the window's client size.
    fn set_extends_content_into_title_bar(&self, extend: bool);

    /// What [`set_extends_content_into_title_bar()`](Self::set_extends_content_into_title_bar)
    /// last set, `false` by default.
    ///
    /// It doesn't say whether the window extends now.
    /// [`title_bar()`](Self::title_bar) does.
    fn extends_content_into_title_bar(&self) -> bool;

    /// The title bar that the content of the window extends into.
    ///
    /// It is `None` while the content doesn't extend, and until the window is shown.
    fn title_bar(&self) -> Option<TitleBar>;

    /// Calls `callback` whenever [`title_bar()`](Self::title_bar) changes.
    ///
    /// That is when the window starts or stops extending into the title bar, and when its DPI,
    /// its maximized state or the system's metrics change it.
    /// The callback runs from the event loop, after the change is over.
    /// It replaces any earlier callback.
    fn on_title_bar_changed(&self, callback: impl FnMut(Option<TitleBar>) + 'static);
}

impl WucWindowExt for Window {
    fn set_extends_content_into_title_bar(&self, extend: bool) {
        title_bar::set_extends_content_into_title_bar(self, extend);
    }

    fn extends_content_into_title_bar(&self) -> bool {
        title_bar::extends_content_into_title_bar(self)
    }

    fn title_bar(&self) -> Option<TitleBar> {
        title_bar::title_bar(self).map(TitleBar::from_core)
    }

    fn on_title_bar_changed(&self, mut callback: impl FnMut(Option<TitleBar>) + 'static) {
        title_bar::on_title_bar_changed(self, move |title_bar| {
            callback(title_bar.map(TitleBar::from_core))
        });
    }
}
