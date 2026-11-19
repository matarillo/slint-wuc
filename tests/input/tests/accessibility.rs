// Copyright © Kentaro Inomata
// SPDX-License-Identifier: MIT-0

//! What a UI Automation client finds in a window: the items' names, roles and states, the
//! actions it can invoke, and where it hears that the focus moves.
//!
//! The client runs on a thread of its own, as a screen reader runs in a process of its own.
//! The test thread owns the window and handles its messages meanwhile. The window is never
//! shown, but a popup that it opens is, without being activated.
//!
//! The platform belongs to the thread that first uses it, so one test runs every scenario.

use i_slint_core::host::Win32WindowAdapter;
use slint::ComponentHandle;
use std::sync::{Mutex, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
};
use windows::Win32::UI::Accessibility::*;
use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{Ref, implement};

slint::include_modules!();

struct Fixture {
    ui: AccessibilityTest,
    hwnd: HWND,
}

impl Fixture {
    fn new() -> Self {
        let ui = AccessibilityTest::new().unwrap();
        let hwnd = Win32WindowAdapter::from_window(ui.window())
            .unwrap()
            .create_hwnd_for_testing()
            .unwrap();
        Self { ui, hwnd }
    }

    /// Runs `client` on a thread of its own with the UI Automation element of the window,
    /// while this thread handles the window's messages.
    fn with_client<T: Send + 'static>(
        &self,
        client: impl FnOnce(&IUIAutomation, &IUIAutomationElement) -> T + Send + 'static,
    ) -> T {
        let hwnd = self.hwnd.0 as usize;
        let (sender, receiver) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok().unwrap() };
            let automation: IUIAutomation =
                unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) }.unwrap();
            let window = unsafe { automation.ElementFromHandle(HWND(hwnd as *mut _)) }.unwrap();
            let _ = sender.send(client(&automation, &window));
        });
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            pump_messages();
            let finished = thread.is_finished();
            if let Ok(result) = receiver.try_recv() {
                thread.join().unwrap();
                return result;
            }
            if finished {
                // It panicked: pass the panic on.
                thread.join().unwrap();
                panic!("the client returned nothing");
            }
            assert!(Instant::now() < deadline, "the client didn't finish");
        }
    }
}

/// Handles the messages of this thread's windows, and runs the timers.
fn pump_messages() {
    let mut msg = MSG::default();
    unsafe {
        while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    i_slint_core::platform::update_timers_and_animations();
    unsafe { MsgWaitForMultipleObjects(None, false, 10, QS_ALLINPUT) };
}

/// An element that the client found, with what it reads of it.
#[derive(Clone, Debug)]
struct Found {
    name: String,
    control_type: UIA_CONTROLTYPE_ID,
}

/// The descendants of `window`, once the window has answered with its tree: UI Automation
/// first gets a placeholder, which the window replaces once it handles a posted message.
fn descendants(
    automation: &IUIAutomation,
    window: &IUIAutomationElement,
    until: impl Fn(&[Found]) -> bool,
) -> Vec<(Found, IUIAutomationElement)> {
    let condition = unsafe { automation.CreateTrueCondition() }.unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let all = unsafe { window.FindAll(TreeScope_Descendants, &condition) }.unwrap();
        let found = (0..unsafe { all.Length() }.unwrap())
            .map(|i| {
                let element = unsafe { all.GetElement(i) }.unwrap();
                let name = unsafe { element.CurrentName() }.unwrap().to_string();
                let control_type = unsafe { element.CurrentControlType() }.unwrap();
                (Found { name, control_type }, element)
            })
            .collect::<Vec<_>>();
        let infos = found.iter().map(|(found, _)| found.clone()).collect::<Vec<_>>();
        if until(&infos) || Instant::now() > deadline {
            return found;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn has(found: &[Found], name: &str) -> bool {
    found.iter().any(|f| f.name == name)
}

fn the_client_finds_the_items() {
    let f = Fixture::new();
    let found = f.with_client(|automation, window| {
        let name = unsafe { window.CurrentName() }.unwrap().to_string();
        let found = descendants(automation, window, |found| has(found, "Check"));
        let found = found.into_iter().map(|(found, _)| found).collect::<Vec<_>>();
        (name, found)
    });
    let (name, found) = found;
    assert_eq!(name, "Accessible");
    let control_type = |name: &str| {
        found.iter().find(|f| f.name == name).map(|f| f.control_type).unwrap_or_else(|| {
            panic!("no {name} in {found:?}");
        })
    };
    assert_eq!(control_type("Hello"), UIA_TextControlTypeId);
    assert_eq!(control_type("Press"), UIA_ButtonControlTypeId);
    assert_eq!(control_type("Check"), UIA_CheckBoxControlTypeId);
}

fn invoking_a_button_runs_its_action() {
    let f = Fixture::new();
    f.with_client(|automation, window| {
        let found = descendants(automation, window, |found| has(found, "Press"));
        let (_, button) = found.iter().find(|(found, _)| found.name == "Press").unwrap();
        let invoke: IUIAutomationInvokePattern =
            unsafe { button.GetCurrentPatternAs(UIA_InvokePatternId) }.unwrap();
        unsafe { invoke.Invoke() }.unwrap();
    });
    // The window runs the action once it handles the message that the request posted.
    let deadline = Instant::now() + Duration::from_secs(5);
    while f.ui.get_clicks() == 0 && Instant::now() < deadline {
        pump_messages();
    }
    assert_eq!(f.ui.get_clicks(), 1);
}

fn the_client_sees_changes() {
    let f = Fixture::new();
    let toggle_state = |f: &Fixture| {
        f.with_client(|automation, window| {
            let found = descendants(automation, window, |found| has(found, "Check"));
            let (_, check) = found.iter().find(|(found, _)| found.name == "Check").unwrap();
            let toggle: IUIAutomationTogglePattern =
                unsafe { check.GetCurrentPatternAs(UIA_TogglePatternId) }.unwrap();
            unsafe { toggle.CurrentToggleState() }.unwrap()
        })
    };
    assert_eq!(toggle_state(&f), ToggleState_Off);
    f.ui.set_checked(true);
    // The window updates the tree from a timer.
    pump_messages();
    assert_eq!(toggle_state(&f), ToggleState_On);
}

/// A focus event that a client got: the element that has the focus.
#[derive(Clone, Debug)]
struct FocusEvent {
    name: String,
    has_keyboard_focus: bool,
}

#[implement(IUIAutomationFocusChangedEventHandler)]
struct FocusHandler {
    events: Mutex<mpsc::Sender<FocusEvent>>,
}

impl IUIAutomationFocusChangedEventHandler_Impl for FocusHandler_Impl {
    fn HandleFocusChangedEvent(
        &self,
        sender: Ref<IUIAutomationElement>,
    ) -> windows::core::Result<()> {
        let element = sender.ok()?;
        let name = unsafe { element.CurrentName() }.map(|n| n.to_string()).unwrap_or_default();
        let has_keyboard_focus =
            unsafe { element.CurrentHasKeyboardFocus() }.is_ok_and(|focus| focus.as_bool());
        let _ = self.events.lock().unwrap().send(FocusEvent { name, has_keyboard_focus });
        Ok(())
    }
}

/// A client that listens for the focus events of the whole desktop, as a screen reader does,
/// on a thread of its own.
struct FocusEvents {
    events: mpsc::Receiver<FocusEvent>,
    stop: mpsc::Sender<()>,
    thread: JoinHandle<()>,
    seen: Vec<FocusEvent>,
}

impl FocusEvents {
    fn listen() -> Self {
        let (sender, events) = mpsc::channel();
        let (stop, stopped) = mpsc::channel::<()>();
        let (ready, listening) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok().unwrap() };
            let automation: IUIAutomation =
                unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) }.unwrap();
            let handler: IUIAutomationFocusChangedEventHandler =
                FocusHandler { events: Mutex::new(sender) }.into();
            unsafe { automation.AddFocusChangedEventHandler(None, &handler) }.unwrap();
            let _ = ready.send(());
            let _ = stopped.recv();
            unsafe { automation.RemoveAllEventHandlers() }.unwrap();
        });
        listening.recv_timeout(Duration::from_secs(10)).expect("the client listens");
        Self { events, stop, thread, seen: Vec::new() }
    }

    /// The first focus event on an element named `name` from now on, while this thread
    /// handles the window's messages.
    fn wait_for(&mut self, name: &str) -> FocusEvent {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            pump_messages();
            while let Ok(event) = self.events.try_recv() {
                self.seen.push(event.clone());
                if event.name == name {
                    return event;
                }
            }
            assert!(Instant::now() < deadline, "no focus event on {name}: {:?}", self.seen);
        }
    }

    fn stop(self) {
        let _ = self.stop.send(());
        while !self.thread.is_finished() {
            pump_messages();
        }
        self.thread.join().unwrap();
    }
}

/// A popup that gets the keys of its window never gets the keyboard focus of Windows, but
/// the client hears that the focus moves into it, and back when it closes.
fn the_focus_follows_a_popup_that_gets_the_keys() {
    let f = Fixture::new();
    let mut events = FocusEvents::listen();
    unsafe { SetFocus(Some(f.hwnd)) }.unwrap();
    f.ui.invoke_focus_field();
    events.wait_for("Field");

    f.ui.invoke_open_popup();
    let choice = events.wait_for("Choice");
    assert!(choice.has_keyboard_focus, "{choice:?}");

    f.ui.invoke_close_popup();
    let field = events.wait_for("Field");
    assert!(field.has_keyboard_focus, "{field:?}");
    events.stop();
}

#[test]
fn ui_automation() {
    the_client_finds_the_items();
    invoking_a_button_runs_its_action();
    the_client_sees_changes();
    the_focus_follows_a_popup_that_gets_the_keys();
}
