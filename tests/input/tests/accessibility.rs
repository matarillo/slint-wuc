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
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
};
use windows::Win32::UI::Accessibility::*;
use windows::Win32::UI::Input::KeyboardAndMouse::{SetFocus, VK_DOWN, VK_RETURN};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{Interface, Ref, implement};

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

/// What a client heard.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Heard {
    Focus,
    Selected,
}

/// An event that a client got, with what it reads of the element it's about.
#[derive(Clone, Debug)]
struct Event {
    heard: Heard,
    name: String,
    has_keyboard_focus: bool,
    control_type: UIA_CONTROLTYPE_ID,
    /// The control type of the container that a selected item names, if it names one.
    container: Option<UIA_CONTROLTYPE_ID>,
}

impl Event {
    fn of(heard: Heard, element: &IUIAutomationElement) -> Self {
        let name = unsafe { element.CurrentName() }.map(|n| n.to_string()).unwrap_or_default();
        let has_keyboard_focus =
            unsafe { element.CurrentHasKeyboardFocus() }.is_ok_and(|focus| focus.as_bool());
        let control_type = unsafe { element.CurrentControlType() }.unwrap_or_default();
        let container = unsafe {
            element
                .GetCurrentPatternAs::<IUIAutomationSelectionItemPattern>(
                    UIA_SelectionItemPatternId,
                )
                .and_then(|item| item.CurrentSelectionContainer())
                .and_then(|container| container.CurrentControlType())
        }
        .ok();
        Self { heard, name, has_keyboard_focus, control_type, container }
    }
}

#[implement(IUIAutomationFocusChangedEventHandler, IUIAutomationEventHandler)]
struct Handler {
    events: Mutex<mpsc::Sender<Event>>,
}

impl IUIAutomationFocusChangedEventHandler_Impl for Handler_Impl {
    fn HandleFocusChangedEvent(
        &self,
        sender: Ref<IUIAutomationElement>,
    ) -> windows::core::Result<()> {
        let _ = self.events.lock().unwrap().send(Event::of(Heard::Focus, sender.ok()?));
        Ok(())
    }
}

impl IUIAutomationEventHandler_Impl for Handler_Impl {
    fn HandleAutomationEvent(
        &self,
        sender: Ref<IUIAutomationElement>,
        _: UIA_EVENT_ID,
    ) -> windows::core::Result<()> {
        let _ = self.events.lock().unwrap().send(Event::of(Heard::Selected, sender.ok()?));
        Ok(())
    }
}

/// A client that listens for the focus events and the selection events of the whole desktop,
/// as a screen reader does, on a thread of its own.
struct Events {
    events: mpsc::Receiver<Event>,
    stop: mpsc::Sender<()>,
    thread: JoinHandle<()>,
    seen: Vec<Event>,
}

impl Events {
    fn listen() -> Self {
        let (sender, events) = mpsc::channel();
        let (stop, stopped) = mpsc::channel::<()>();
        let (ready, listening) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok().unwrap() };
            let automation: IUIAutomation =
                unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) }.unwrap();
            let handler = Handler { events: Mutex::new(sender) };
            let focus: IUIAutomationFocusChangedEventHandler = handler.into();
            let selected: IUIAutomationEventHandler = focus.cast().unwrap();
            unsafe {
                automation.AddFocusChangedEventHandler(None, &focus).unwrap();
                let root = automation.GetRootElement().unwrap();
                automation
                    .AddAutomationEventHandler(
                        UIA_SelectionItem_ElementSelectedEventId,
                        &root,
                        TreeScope_Subtree,
                        None,
                        &selected,
                    )
                    .unwrap();
            }
            let _ = ready.send(());
            let _ = stopped.recv();
            unsafe { automation.RemoveAllEventHandlers() }.unwrap();
        });
        listening.recv_timeout(Duration::from_secs(10)).expect("the client listens");
        Self { events, stop, thread, seen: Vec::new() }
    }

    /// The first event of `heard` on an element named `name` from now on, while this thread
    /// handles the window's messages.
    fn wait_for(&mut self, heard: Heard, name: &str) -> Event {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            pump_messages();
            while let Ok(event) = self.events.try_recv() {
                self.seen.push(event.clone());
                if event.heard == heard && event.name == name {
                    return event;
                }
            }
            assert!(Instant::now() < deadline, "no {heard:?} on {name}: {:?}", self.seen);
        }
    }

    /// Handles the window's messages for `duration`, and keeps the events that came.
    fn settle(&mut self, duration: Duration) {
        let deadline = Instant::now() + duration;
        while Instant::now() < deadline {
            pump_messages();
            self.seen.extend(self.events.try_iter());
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
    let mut events = Events::listen();
    unsafe { SetFocus(Some(f.hwnd)) }.unwrap();
    f.ui.invoke_focus_field();
    events.wait_for(Heard::Focus, "Field");

    f.ui.invoke_open_popup();
    let choice = events.wait_for(Heard::Focus, "Choice");
    assert!(choice.has_keyboard_focus, "{choice:?}");

    f.ui.invoke_close_popup();
    let field = events.wait_for(Heard::Focus, "Field");
    assert!(field.has_keyboard_focus, "{field:?}");
    events.stop();
}

/// Sends a key press and release to `hwnd`: a virtual key and its scan code.
fn press(hwnd: HWND, vk: u16, scan: u32, extended: bool) {
    let lp = 1 | (scan << 16) | if extended { 1 << 24 } else { 0 };
    unsafe {
        SendMessageW(hwnd, WM_KEYDOWN, Some(WPARAM(vk as usize)), Some(LPARAM(lp as isize)));
        let up = lp | (1 << 30) | (1 << 31);
        SendMessageW(hwnd, WM_KEYUP, Some(WPARAM(vk as usize)), Some(LPARAM(up as isize)));
    }
}

/// The list of upstream's combo box keeps the focus on itself and selects its items with the
/// arrow keys: the client hears the selection, from the list's popup.
///
/// `SLINT_FORK_A11Y_POPUP` picks how the popup's tree stands in for the combo box, while the
/// owner compares them with Narrator: `list` gives the items a list as their container, and
/// `selected` moves the focus to the selected item; `both` does both. With `both`, this takes
/// some 16 seconds more, while the window raises the events of the down key. Probably the client
/// asks the item for its container meanwhile and waits for this thread, which a screen reader in
/// a process of its own doesn't.
fn the_combo_box_list_tells_its_selection() {
    let variant = std::env::var("SLINT_FORK_A11Y_POPUP").unwrap_or_default();
    let list_root = matches!(&*variant, "list" | "both");
    let focus_selected = matches!(&*variant, "selected" | "both");
    let ui = ComboBoxTest::new().unwrap();
    let hwnd =
        Win32WindowAdapter::from_window(ui.window()).unwrap().create_hwnd_for_testing().unwrap();
    let mut events = Events::listen();
    unsafe { SetFocus(Some(hwnd)) }.unwrap();
    ui.invoke_focus_box();
    // AccessKit raises the selection events of a tree that a client asked for: give the client
    // time to ask for the window's tree, and for the popup's once it opens.
    events.settle(Duration::from_millis(500));
    press(hwnd, VK_RETURN.0, 0x1c, false);
    if focus_selected {
        let one = events.wait_for(Heard::Focus, "One");
        assert!(one.has_keyboard_focus, "{one:?}");
    }
    events.settle(Duration::from_millis(500));
    if list_root && !focus_selected {
        let seen = &events.seen;
        let list =
            seen.iter().any(|e| e.heard == Heard::Focus && e.control_type == UIA_ListControlTypeId);
        assert!(list, "{seen:?}");
    }
    press(hwnd, VK_DOWN.0, 0x50, true);
    let two = events.wait_for(Heard::Selected, "Two");
    let expected = list_root.then_some(UIA_ListControlTypeId);
    assert_eq!(two.container, expected, "{two:?}");
    if focus_selected {
        // The focus moves before the selection is told.
        let seen = &events.seen;
        let two = seen.iter().find(|e| e.heard == Heard::Focus && e.name == "Two");
        assert!(two.is_some_and(|two| two.has_keyboard_focus), "{seen:?}");
    }
    assert_eq!(ui.get_current(), 1);
    events.settle(Duration::from_millis(200));
    events.stop();
}

#[test]
fn ui_automation() {
    the_client_finds_the_items();
    invoking_a_button_runs_its_action();
    the_client_sees_changes();
    the_focus_follows_a_popup_that_gets_the_keys();
    the_combo_box_list_tells_its_selection();
}
