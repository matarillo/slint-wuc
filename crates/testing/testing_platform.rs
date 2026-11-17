// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

use i_slint_core::SharedString;
use i_slint_core::host;
use i_slint_core::platform::{Clipboard, Platform, PlatformError};
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

    /// The last URL passed to `open_url`, which the platform records instead of opening.
    static OPEN_URL: RefCell<Option<SharedString>> = const { RefCell::new(None) };
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

/// Simulate a click at (`x`, `y`) and release after 50 ms of mock time.
#[cfg(feature = "internal")]
pub fn send_mouse_click(x: f32, y: f32, window_adapter: &i_slint_core::window::WindowAdapterRc) {
    use i_slint_core::api::LogicalPosition;
    use i_slint_core::items::PointerEventButton;
    use i_slint_core::platform::WindowEvent;

    let position = LogicalPosition::new(x, y);
    let button = PointerEventButton::Left;

    window_adapter.window().dispatch_event(WindowEvent::PointerMoved { position });
    window_adapter.window().dispatch_event(WindowEvent::PointerPressed { position, button });
    mock_elapsed_time(50);
    window_adapter.window().dispatch_event(WindowEvent::PointerReleased { position, button });
}

/// Dispatch a single key press or release event.
#[cfg(feature = "internal")]
pub fn send_keyboard_key_text(
    text: &i_slint_core::SharedString,
    pressed: bool,
    window_adapter: &i_slint_core::window::WindowAdapterRc,
) {
    use i_slint_core::platform::WindowEvent;
    window_adapter.window().dispatch_event(if pressed {
        WindowEvent::KeyPressed { text: text.clone() }
    } else {
        WindowEvent::KeyReleased { text: text.clone() }
    })
}

/// Simulate typing a string, with automatic Shift handling for uppercase letters.
#[cfg(feature = "internal")]
pub fn send_keyboard_string_sequence(
    sequence: &i_slint_core::SharedString,
    window_adapter: &i_slint_core::window::WindowAdapterRc,
) {
    use i_slint_core::input::key_codes::Key;
    use i_slint_core::platform::WindowEvent;

    for ch in sequence.chars() {
        if ch.is_ascii_uppercase() {
            window_adapter
                .window()
                .dispatch_event(WindowEvent::KeyPressed { text: Key::Shift.into() });
        }

        let text: i_slint_core::SharedString = ch.into();
        window_adapter.window().dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
        window_adapter.window().dispatch_event(WindowEvent::KeyReleased { text });

        if ch.is_ascii_uppercase() {
            window_adapter
                .window()
                .dispatch_event(WindowEvent::KeyReleased { text: Key::Shift.into() });
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
    /// The clipboards are in memory, as upstream's testing backend keeps them, so that a test
    /// neither touches the system clipboard nor shares it with the tests running in parallel.
    clipboard: RefCell<Option<String>>,
    selection_clipboard: RefCell<Option<String>>,
}

impl TestingPlatform {
    /// Creates a platform on the calling thread.
    ///
    /// With `mock_time`, the animation clock only moves with [`mock_elapsed_time()`].
    pub fn new(mock_time: bool) -> Result<Self, PlatformError> {
        Ok(Self {
            host: host::Platform::new_headless()?,
            mock_time,
            clipboard: Default::default(),
            selection_clipboard: Default::default(),
        })
    }

    fn clipboard_slot(&self, clipboard: Clipboard) -> Option<&RefCell<Option<String>>> {
        match clipboard {
            Clipboard::DefaultClipboard => Some(&self.clipboard),
            Clipboard::SelectionClipboard => Some(&self.selection_clipboard),
            _ => None,
        }
    }
}

impl Platform for TestingPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        let adapter = self.host.create_window_adapter()?;
        // Only the testing platform measures `FixedTestFont` with fixed metrics.
        host::Win32WindowAdapter::from_window(adapter.window())
            .expect("the host created the window")
            .enable_fixed_test_font();
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

    fn set_clipboard_text(&self, text: &str, clipboard: Clipboard) {
        if let Some(slot) = self.clipboard_slot(clipboard) {
            *slot.borrow_mut() = Some(text.into());
        }
    }

    fn clipboard_text(&self, clipboard: Clipboard) -> Option<String> {
        self.clipboard_slot(clipboard)?.borrow().clone()
    }

    fn open_url(&self, url: &str) -> Result<(), PlatformError> {
        OPEN_URL.with(|last| *last.borrow_mut() = Some(url.into()));
        Ok(())
    }

    fn debug_log(&self, arguments: core::fmt::Arguments) {
        DEBUG_LOGS.with(|logs| logs.borrow_mut().push(arguments.to_string()));
        i_slint_core::debug_log::default_log_message(arguments);
    }
}

/// What tests can reach of a window created by the [`TestingPlatform`], through
/// [`access_testing_window`]: what upstream's testing window records, which the host records
/// for its windows.
#[cfg(feature = "internal")]
pub struct TestingWindow<'a> {
    host: &'a host::Win32WindowAdapter,
    /// The input method requests since a test last took them.
    pub ime_requests: &'a RefCell<Vec<i_slint_core::window::InputMethodRequest>>,
}

#[cfg(feature = "internal")]
impl TestingWindow<'_> {
    /// Makes the popups of this window windows of their own instead of drawing them inside it.
    pub fn use_native_popup(&self, native: bool) {
        self.host.set_native_popup_for_testing(native);
    }

    /// The last URL that was passed to `open_url`.
    pub fn open_url(&self) -> Option<SharedString> {
        OPEN_URL.with(|last| last.borrow().clone())
    }

    /// Drain and return all debug_log messages captured since the last call.
    pub fn take_debug_log(&self) -> Vec<String> {
        DEBUG_LOGS.with(|logs| logs.borrow_mut().drain(..).collect())
    }

    pub fn ime_requests(&self) -> Vec<i_slint_core::window::InputMethodRequest> {
        self.ime_requests.borrow().clone()
    }

    pub fn clear_ime_requests(&self) {
        self.ime_requests.borrow_mut().clear();
    }

    /// The cursor the window last asked for.
    pub fn mouse_cursor(&self) -> i_slint_core::cursor::MouseCursorInner {
        self.host.mouse_cursor_for_testing()
    }

    /// Number of interactive window moves requested via `WindowMoveArea`.
    pub fn window_move_request_count(&self) -> usize {
        self.host.window_move_requests_for_testing()
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
    let host = host::Win32WindowAdapter::from_window(window).expect("the host created the window");
    callback(&TestingWindow { host: &host, ime_requests: host.ime_requests_for_testing() })
}
