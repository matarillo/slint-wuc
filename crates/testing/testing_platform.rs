// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

use i_slint_core::host;
use i_slint_core::platform::{Platform, PlatformError};
use i_slint_core::window::{WindowAdapter, WindowInner};
use std::cell::RefCell;
use std::rc::{Rc, Weak};
use std::time::Duration;

std::thread_local! {
    /// Live windows targeted by [`mock_elapsed_time`].
    static ALL_TESTING_WINDOWS: RefCell<Vec<Weak<dyn WindowAdapter>>> =
        const { RefCell::new(Vec::new()) };

    /// The messages of `debug()` calls since the last [`TestingWindow::take_debug_log`].
    static DEBUG_LOGS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// The windows that the platform created and that are still alive.
fn live_windows() -> Vec<Rc<dyn WindowAdapter>> {
    ALL_TESTING_WINDOWS.with(|list| {
        let mut list = list.borrow_mut();
        list.retain(|w| w.upgrade().is_some());
        list.iter().filter_map(|w| w.upgrade()).collect()
    })
}

/// Advance the mocked time by the given number of milliseconds, updating
/// animations, firing timers, running change handlers, and instantiating
/// pending repeaters and conditionals on every live testing window.
/// Then run the pending sync pass of each of them.
pub fn mock_elapsed_time(time_in_ms: u64) {
    let tick = i_slint_core::animations::CURRENT_ANIMATION_DRIVER.with(|driver| {
        let mut tick = driver.current_tick();
        tick += Duration::from_millis(time_in_ms);
        driver.update_animations(tick);
        tick
    });
    i_slint_core::timers::TimerList::maybe_activate_timers(tick);
    i_slint_core::properties::ChangeTracker::run_change_handlers();
    for adapter in live_windows() {
        let window = adapter.window();
        WindowInner::from_pub(window).ensure_tree_instantiated();
        if let Some(host_window) = host::Win32WindowAdapter::from_window(window) {
            host_window.sync_now();
        }
    }
}

/// Return the current mocked time in milliseconds.
pub fn get_mocked_time() -> u64 {
    i_slint_core::animations::CURRENT_ANIMATION_DRIVER
        .with(|driver| driver.current_tick())
        .as_millis()
}

/// A platform for tests: the headless host, with a clock that the test advances.
///
/// Windows come from the host and have no HWND.
/// The platform has no event loop, so `slint::invoke_from_event_loop` and similar functions
/// don't work with it.
pub struct TestingPlatform {
    host: host::Platform,
    mock_time: bool,
}

impl TestingPlatform {
    /// Creates a platform on the calling thread.
    ///
    /// With `mock_time`, the animation clock only moves with [`mock_elapsed_time()`].
    pub fn new(mock_time: bool) -> Result<Self, PlatformError> {
        Ok(Self { host: host::Platform::new_headless()?, mock_time })
    }
}

impl Platform for TestingPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        let adapter = self.host.create_window_adapter()?;
        ALL_TESTING_WINDOWS.with(|list| {
            let mut list = list.borrow_mut();
            list.retain(|w| w.upgrade().is_some());
            list.push(Rc::downgrade(&adapter));
        });
        Ok(adapter)
    }

    fn run_event_loop(&self) -> Result<(), PlatformError> {
        Err(PlatformError::NoEventLoopProvider)
    }

    fn duration_since_start(&self) -> Duration {
        if self.mock_time {
            // `mock_elapsed_time` updates the animation tick directly.
            i_slint_core::animations::current_tick().into()
        } else {
            self.host.duration_since_start()
        }
    }

    fn click_interval(&self) -> Duration {
        self.host.click_interval()
    }

    fn cursor_flash_cycle(&self) -> Duration {
        self.host.cursor_flash_cycle()
    }

    fn debug_log(&self, arguments: core::fmt::Arguments) {
        DEBUG_LOGS.with(|logs| logs.borrow_mut().push(arguments.to_string()));
        i_slint_core::debug_log::default_log_message(arguments);
    }
}

/// What tests can reach of a window created by the [`TestingPlatform`], through
/// [`access_testing_window`].
///
/// The window itself belongs to the host, so this only exposes what the platform records.
#[cfg(feature = "internal")]
pub struct TestingWindow(());

#[cfg(feature = "internal")]
impl TestingWindow {
    /// Drain and return all debug_log messages captured since the last call.
    pub fn take_debug_log(&self) -> Vec<String> {
        DEBUG_LOGS.with(|logs| logs.borrow_mut().drain(..).collect())
    }
}

/// Runs `callback` with the testing view of `window`.
///
/// Panics if the window wasn't created by the [`TestingPlatform`].
#[cfg(feature = "internal")]
pub fn access_testing_window<R>(
    window: &i_slint_core::api::Window,
    callback: impl FnOnce(&TestingWindow) -> R,
) -> R {
    assert!(
        live_windows().iter().any(|adapter| core::ptr::eq(adapter.window(), window)),
        "access_testing_window called without testing platform/adapter"
    );
    callback(&TestingWindow(()))
}
