// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

//! Testing platform for the Slint runtime on Windows.UI.Composition.
//!
//! The platform wraps the headless host of `i-slint-core`: windows have no HWND, and every
//! [`mock_elapsed_time()`] runs the sync pass that brings the window's visuals up to date.
//! The API follows the upstream crate of the same name, as far as input and text allow.

mod search_api;
pub use search_api::*;
#[cfg(feature = "internal")]
mod internal_tests;
#[cfg(feature = "internal")]
pub use internal_tests::*;
pub mod testing_platform;
pub use testing_platform::{TestingPlatform, get_mocked_time};
#[cfg(feature = "internal")]
pub use testing_platform::{TestingWindow, access_testing_window, mock_elapsed_time};

/// Initialize the testing platform without support for event loop.
/// This means that each test thread can use its own platform, but global functions that needs
/// an event loop such as `slint::invoke_from_event_loop` won't work.
/// Must be called before any call that would otherwise initialize the Win32 host.
/// Calling it when the host is already initialized will panic.
///
/// Note that for animations and timers, the changes in the system time will be disregarded.
/// Instead, use [`mock_elapsed_time()`] to advance the simulate (mock) time Slint uses.
pub fn init_no_event_loop() {
    let platform = TestingPlatform::new(true).expect("cannot set up the headless host");
    i_slint_core::platform::set_platform(Box::new(platform)).expect("platform already initialized");
}

/// Advance the simulated mock time by the specified duration. Use in combination with
/// [`init_no_event_loop()`].
#[cfg(not(feature = "internal"))]
pub fn mock_elapsed_time(duration: std::time::Duration) {
    testing_platform::mock_elapsed_time(duration.as_millis() as _);
}

/// Replace the font collection with embedded NotoSans fonts for deterministic test results.
/// Must be called after initializing the testing platform (e.g. after [`init_no_event_loop()`]).
///
/// The runtime has no text support yet, so this does nothing until text arrives in phase 4.
#[cfg(feature = "internal")]
pub fn configure_test_fonts() {}
