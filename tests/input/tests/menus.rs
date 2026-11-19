// Copyright © Kentaro Inomata
// SPDX-License-Identifier: MIT-0

//! The native menus of a window: its Win32 menu bar, built from a `MenuBar`, and the request
//! to open a `ContextMenuArea`'s menu.
//!
//! The window is never shown, and nothing pumps its messages: a context menu would wait for
//! the user, so the tests only check that one was asked for.
//!
//! The platform belongs to the thread that first uses it, so one test runs every scenario.

use i_slint_core::host::Win32WindowAdapter;
use slint::ComponentHandle;
use windows::Win32::Foundation::*;
use windows::Win32::UI::WindowsAndMessaging::*;

slint::include_modules!();

/// `WM_APP_CONTEXT_MENU` in the host.
const WM_APP_CONTEXT_MENU: u32 = WM_APP + 6;

struct Fixture {
    ui: MenuTest,
    hwnd: HWND,
}

impl Fixture {
    fn new() -> Self {
        let ui = MenuTest::new().unwrap();
        let hwnd = Win32WindowAdapter::from_window(ui.window())
            .unwrap()
            .create_hwnd_for_testing()
            .unwrap();
        Self { ui, hwnd }
    }

    fn bar(&self) -> HMENU {
        unsafe { GetMenu(self.hwnd) }
    }

    fn file_menu(&self) -> HMENU {
        unsafe { GetSubMenu(self.bar(), 0) }
    }

    fn send(&self, msg: u32, wp: usize, lp: isize) -> LRESULT {
        unsafe { SendMessageW(self.hwnd, msg, Some(WPARAM(wp)), Some(LPARAM(lp))) }
    }

    /// Clicks the entry at `position` in `menu`, as the menu reports it.
    fn choose(&self, menu: HMENU, position: i32) {
        let id = unsafe { GetMenuItemID(menu, position) };
        self.send(WM_COMMAND, id as usize, 0);
    }
}

/// The text of the entry at `position`, the shortcut after a tab.
fn text(menu: HMENU, position: u32) -> String {
    let mut buffer = [0u16; 64];
    let len = unsafe { GetMenuStringW(menu, position, Some(&mut buffer), MF_BYPOSITION) };
    String::from_utf16_lossy(&buffer[..len as usize])
}

fn state(menu: HMENU, position: u32) -> MENU_ITEM_FLAGS {
    MENU_ITEM_FLAGS(unsafe { GetMenuState(menu, position, MF_BYPOSITION) })
}

/// Runs the timer that rebuilds a menu bar after a change.
fn run_timers() {
    i_slint_core::platform::update_timers_and_animations();
}

fn the_menu_bar_shows_the_entries() {
    let f = Fixture::new();
    let bar = f.bar();
    assert!(!bar.is_invalid());
    assert_eq!(unsafe { GetMenuItemCount(Some(bar)) }, 1);
    assert_eq!(text(bar, 0), "File");

    let file = f.file_menu();
    assert_eq!(unsafe { GetMenuItemCount(Some(file)) }, 5);
    assert_eq!(text(file, 0), "Open\tCtrl+O");
    assert!(state(file, 1).contains(MF_SEPARATOR));
    assert!(state(file, 2).contains(MF_CHECKED));
    assert!(state(file, 3).contains(MF_GRAYED));
    assert_eq!(text(file, 4), "Recent");
    assert!(!unsafe { GetSubMenu(file, 4) }.is_invalid());

    // The frame has room for the menu bar above the client area of the window's size.
    let mut client = RECT::default();
    unsafe { GetClientRect(f.hwnd, &mut client).unwrap() };
    assert_eq!(client.bottom as u32, f.ui.window().size().height);
}

fn a_command_activates_its_entry() {
    let f = Fixture::new();
    f.choose(f.file_menu(), 0);
    let recent = unsafe { GetSubMenu(f.file_menu(), 4) };
    f.choose(recent, 0);
    assert_eq!(f.ui.get_activated(), "open one ");
}

fn the_menu_bar_follows_changes() {
    let f = Fixture::new();
    f.ui.set_show_edit(true);
    f.ui.set_checked(false);
    run_timers();
    assert_eq!(unsafe { GetMenuItemCount(Some(f.bar())) }, 2);
    assert_eq!(text(f.bar(), 1), "Edit");
    assert!(!state(f.file_menu(), 2).contains(MF_CHECKED));
}

fn an_open_menu_keeps_its_entries_until_it_closes() {
    let f = Fixture::new();
    f.send(WM_ENTERMENULOOP, 0, 0);
    let file = f.file_menu();
    f.ui.set_show_edit(true);
    run_timers();
    assert_eq!(unsafe { GetMenuItemCount(Some(f.bar())) }, 1);
    assert_eq!(f.file_menu(), file);

    f.send(WM_EXITMENULOOP, 0, 0);
    assert_eq!(unsafe { GetMenuItemCount(Some(f.bar())) }, 2);
}

fn a_right_click_asks_for_a_native_context_menu() {
    let f = Fixture::new();
    // Slint shows no popup of its own: the window opens a Win32 menu once the click is handled.
    f.send(WM_RBUTTONDOWN, 0x0002, (20 << 16) | 20);
    f.send(WM_RBUTTONUP, 0, (20 << 16) | 20);
    let inner = i_slint_core::window::WindowInner::from_pub(f.ui.window());
    assert!(inner.active_popups().is_empty());
    let mut msg = MSG::default();
    let posted = unsafe {
        PeekMessageW(&mut msg, Some(f.hwnd), WM_APP_CONTEXT_MENU, WM_APP_CONTEXT_MENU, PM_REMOVE)
    };
    assert!(posted.as_bool());
}

#[test]
fn native_menus() {
    the_menu_bar_shows_the_entries();
    a_command_activates_its_entry();
    the_menu_bar_follows_changes();
    an_open_menu_keeps_its_entries_until_it_closes();
    a_right_click_asks_for_a_native_context_menu();
}
