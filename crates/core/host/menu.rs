// SPDX-License-Identifier: MIT OR Apache-2.0

//! Native menus, as upstream's winit backend shows them on Windows through `muda`: a `MenuBar`
//! is the window's Win32 menu bar, and a `ContextMenu` opens a Win32 popup menu.

use super::Win32WindowAdapter;
use crate::SharedVector;
use crate::api::LogicalPosition;
use crate::graphics::Image;
use crate::items::MenuEntry;
use crate::menus::MenuVTable;
use crate::properties::{PropertyDirtyHandler, PropertyTracker};
use alloc::rc::Weak;
use alloc::vec::Vec;
use core::cell::{Cell, RefCell};
use core::pin::Pin;
use vtable::{VRc, VWeak};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, ClientToScreen, CreateDIBSection, DIB_RGB_COLORS,
    DeleteObject, HBITMAP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateMenu, CreatePopupMenu, DestroyMenu, DrawMenuBar, GetMenuItemCount, HMENU,
    MENUITEMINFOW, MF_CHECKED, MF_GRAYED, MF_POPUP, MF_SEPARATOR, MF_STRING, MF_UNCHECKED,
    MIIM_BITMAP, PostMessageW, SetForegroundWindow, SetMenu, SetMenuItemInfoW, TPM_LEFTALIGN,
    TPM_RETURNCMD, TrackPopupMenuEx, WM_COMMAND, WM_DESTROY, WM_ENTERMENULOOP, WM_EXITMENULOOP,
};
use windows::core::{HSTRING, PCWSTR};

/// Opens the context menu that [`Win32WindowAdapter::show_native_popup_menu`] asked for.
pub(super) const WM_APP_CONTEXT_MENU: u32 = super::WM_APP + 6;

/// Commands start at 1, since 0 means no command, and stay below the system commands
/// (`SC_*`, from `0xF000`).
const FIRST_COMMAND: usize = 1;
const LAST_COMMAND: usize = 0xEFFF;
/// As upstream, which limits what it hands to `muda`.
const MAX_DEPTH: usize = 15;
/// The size `muda` draws menu icons at.
const ICON_SIZE: usize = 16;

/// Whether `SLINT_NO_MUDA` is set, in which case Slint draws the menu bar and the context
/// menus, as upstream does.
pub(super) fn disabled() -> bool {
    std::env::var_os("SLINT_NO_MUDA").is_some()
}

/// A Win32 menu built from a Slint menu, and the entries its commands stand for. Dropping it
/// destroys the menu.
struct NativeMenu {
    hmenu: HMENU,
    entries: Vec<MenuEntry>,
    bitmaps: Vec<HBITMAP>,
}

impl NativeMenu {
    /// The entries of `menu` in `hmenu`, a menu bar or a popup menu.
    fn build(menu: &VRc<MenuVTable>, hmenu: HMENU) -> Self {
        let mut this = Self { hmenu, entries: Vec::new(), bitmaps: Vec::new() };
        this.append_children(VRc::borrow(menu), hmenu, None, 0);
        this
    }

    fn append_children(
        &mut self,
        menu: vtable::VRef<'_, MenuVTable>,
        parent: HMENU,
        entry: Option<&MenuEntry>,
        depth: usize,
    ) {
        let mut children = SharedVector::default();
        menu.sub_menu(entry, &mut children);
        if depth > MAX_DEPTH {
            if !children.is_empty() {
                let entry = MenuEntry {
                    title: "<Error: Menu Depth limit reached>".into(),
                    enabled: false,
                    ..Default::default()
                };
                self.append(menu, parent, &entry, depth);
            }
            return;
        }
        for child in children.iter() {
            self.append(menu, parent, child, depth);
        }
    }

    fn append(
        &mut self,
        menu: vtable::VRef<'_, MenuVTable>,
        parent: HMENU,
        entry: &MenuEntry,
        depth: usize,
    ) {
        if entry.is_separator {
            let _ = unsafe { AppendMenuW(parent, MF_SEPARATOR, 0, PCWSTR::null()) };
            return;
        }
        let title = title(entry);
        let mut flags = MF_STRING;
        if !entry.enabled {
            flags |= MF_GRAYED;
        }
        if entry.has_sub_menu {
            let Ok(sub_menu) = (unsafe { CreatePopupMenu() }) else { return };
            self.append_children(menu, sub_menu, Some(entry), depth + 1);
            let _ = unsafe { AppendMenuW(parent, flags | MF_POPUP, sub_menu.0 as usize, &title) };
            return;
        }
        let command = FIRST_COMMAND + self.entries.len();
        if command > LAST_COMMAND {
            return;
        }
        self.entries.push(entry.clone());
        if entry.checkable {
            flags |= if entry.checked { MF_CHECKED } else { MF_UNCHECKED };
        }
        let _ = unsafe { AppendMenuW(parent, flags, command, &title) };
        // As in `muda`, a checkable entry shows its check mark instead of its icon.
        if !entry.checkable
            && let Some(bitmap) = icon_bitmap(&entry.icon)
        {
            self.bitmaps.push(bitmap);
            let info = MENUITEMINFOW {
                cbSize: size_of::<MENUITEMINFOW>() as u32,
                fMask: MIIM_BITMAP,
                hbmpItem: bitmap,
                ..Default::default()
            };
            let _ = unsafe { SetMenuItemInfoW(parent, command as u32, false, &info) };
        }
    }

    fn entry(&self, command: usize) -> Option<&MenuEntry> {
        self.entries.get(command.checked_sub(FIRST_COMMAND)?)
    }
}

impl Drop for NativeMenu {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyMenu(self.hmenu);
            for bitmap in self.bitmaps.drain(..) {
                let _ = DeleteObject(bitmap.into());
            }
        }
    }
}

/// The title, and the shortcut after a tab, which Win32 menus align in a column of its own.
fn title(entry: &MenuEntry) -> HSTRING {
    if entry.shortcut == Default::default() {
        HSTRING::from(entry.title.as_str())
    } else {
        HSTRING::from(alloc::format!("{}\t{}", entry.title, entry.shortcut))
    }
}

/// `icon` as a 32-bit bitmap with premultiplied alpha, which menus draw with transparency.
fn icon_bitmap(icon: &Image) -> Option<HBITMAP> {
    let pixels = icon.to_rgba8_premultiplied()?;
    let (width, height) = (pixels.width() as usize, pixels.height() as usize);
    if width == 0 || height == 0 {
        return None;
    }
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: ICON_SIZE as i32,
            // Top-down.
            biHeight: -(ICON_SIZE as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits = core::ptr::null_mut();
    let bitmap =
        unsafe { CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0) }.ok()?;
    if bits.is_null() {
        unsafe {
            let _ = DeleteObject(bitmap.into());
        }
        return None;
    }
    // SAFETY: the section holds ICON_SIZE² pixels of 4 bytes.
    let target =
        unsafe { core::slice::from_raw_parts_mut(bits as *mut u8, ICON_SIZE * ICON_SIZE * 4) };
    // Each target pixel averages the source pixels it covers.
    let source = pixels.as_slice();
    for ty in 0..ICON_SIZE {
        let (y0, y1) = span(ty, height);
        for tx in 0..ICON_SIZE {
            let (x0, x1) = span(tx, width);
            let mut sum = [0u32; 4];
            for y in y0..y1 {
                for p in &source[y * width + x0..y * width + x1] {
                    sum[0] += p.b as u32;
                    sum[1] += p.g as u32;
                    sum[2] += p.r as u32;
                    sum[3] += p.a as u32;
                }
            }
            let count = ((y1 - y0) * (x1 - x0)) as u32;
            let offset = (ty * ICON_SIZE + tx) * 4;
            for (target, sum) in target[offset..offset + 4].iter_mut().zip(sum) {
                *target = (sum / count) as u8;
            }
        }
    }
    Some(bitmap)
}

/// The source pixels that target pixel `i` of [`ICON_SIZE`] covers, at least one.
fn span(i: usize, source: usize) -> (usize, usize) {
    let start = i * source / ICON_SIZE;
    let end = ((i + 1) * source / ICON_SIZE).max(start + 1).min(source);
    (start.min(source - 1), end)
}

/// The menu bar of a window, rebuilt when what the menu shows changes.
pub(super) struct MenuBar {
    menu: RefCell<Option<VWeak<MenuVTable>>>,
    native: RefCell<Option<NativeMenu>>,
    tracker: Pin<alloc::boxed::Box<PropertyTracker<false, Rebuild>>>,
    /// Set while a menu is open; the menu bar is rebuilt once it closes.
    in_menu_loop: Cell<bool>,
    rebuild_pending: Cell<bool>,
}

struct Rebuild(Weak<Win32WindowAdapter>);

impl PropertyDirtyHandler for Rebuild {
    fn notify(self: Pin<&Self>) {
        let adapter = self.0.clone();
        crate::timers::Timer::single_shot(Default::default(), move || {
            if let Some(adapter) = adapter.upgrade() {
                adapter.menu_bar.rebuild(&adapter);
            }
        });
    }
}

impl MenuBar {
    pub(super) fn new(adapter: Weak<Win32WindowAdapter>) -> Self {
        Self {
            menu: Default::default(),
            native: Default::default(),
            tracker: alloc::boxed::Box::pin(PropertyTracker::new_with_dirty_handler(Rebuild(
                adapter,
            ))),
            in_menu_loop: Cell::new(false),
            rebuild_pending: Cell::new(false),
        }
    }

    pub(super) fn set(&self, adapter: &Win32WindowAdapter, menu: &VRc<MenuVTable>) {
        self.menu.replace(Some(VRc::downgrade(menu)));
        self.rebuild(adapter);
    }

    /// Builds the menu bar, and returns it unless it has no entries.
    pub(super) fn build(&self) -> Option<HMENU> {
        let menu = self.menu.borrow().as_ref().and_then(VWeak::upgrade);
        let native = menu.and_then(|menu| {
            self.tracker.as_ref().evaluate(|| {
                if !VRc::borrow(&menu).visible() {
                    return None;
                }
                let hmenu = unsafe { CreateMenu() }.ok()?;
                let native = NativeMenu::build(&menu, hmenu);
                // Without entries, Windows would still show an empty bar.
                (unsafe { GetMenuItemCount(Some(hmenu)) } > 0).then_some(native)
            })
        });
        let hmenu = native.as_ref().map(|n| n.hmenu);
        // The old menu goes once the window has the new one.
        let _old = self.native.replace(native);
        hmenu
    }

    fn rebuild(&self, adapter: &Win32WindowAdapter) {
        let Some(hwnd) = adapter.hwnd.get() else { return };
        if self.in_menu_loop.get() {
            self.rebuild_pending.set(true);
            return;
        }
        let had_menu = self.native.borrow().is_some();
        let old = self.native.take();
        let hmenu = self.build();
        unsafe {
            let _ = SetMenu(hwnd, hmenu);
            let _ = DrawMenuBar(hwnd);
        }
        drop(old);
        if had_menu != hmenu.is_some() {
            adapter.menu_bar_changed();
        }
    }

    /// Handles the messages of the menu bar: its commands, and when it opens and closes.
    pub(super) fn handle(
        &self,
        adapter: &Win32WindowAdapter,
        hwnd: HWND,
        msg: u32,
        wp: WPARAM,
        lp: LPARAM,
    ) -> Option<LRESULT> {
        match msg {
            // A command from a menu has 0 in the high word and no control.
            WM_COMMAND if wp.0 >> 16 == 0 && lp.0 == 0 => {
                let command = wp.0 & 0xffff;
                let menu = self.menu.borrow().as_ref().and_then(VWeak::upgrade)?;
                let entry = self.native.borrow().as_ref()?.entry(command)?.clone();
                VRc::borrow(&menu).activate(&entry);
                Some(LRESULT(0))
            }
            WM_ENTERMENULOOP => {
                self.in_menu_loop.set(true);
                None
            }
            WM_EXITMENULOOP => {
                self.in_menu_loop.set(false);
                if self.rebuild_pending.take() {
                    self.rebuild(adapter);
                }
                None
            }
            // `DestroyWindow` destroys the window's menu.
            WM_DESTROY => {
                if let Some(native) = self.native.take() {
                    unsafe {
                        let _ = SetMenu(hwnd, None);
                    }
                    drop(native);
                }
                None
            }
            _ => None,
        }
    }
}

/// A context menu that waits for the current input to be handled, since a popup menu runs a
/// message loop of its own until it closes.
pub(super) struct PendingContextMenu {
    menu: VRc<MenuVTable>,
    position: LogicalPosition,
}

/// Asks `adapter`'s window to open `menu` at `position`, in its logical coordinates.
pub(super) fn request_context_menu(
    adapter: &Win32WindowAdapter,
    menu: VRc<MenuVTable>,
    position: LogicalPosition,
) -> bool {
    let Some(hwnd) = adapter.hwnd.get() else { return false };
    adapter.context_menu.replace(Some(PendingContextMenu { menu, position }));
    unsafe { PostMessageW(Some(hwnd), WM_APP_CONTEXT_MENU, WPARAM(0), LPARAM(0)) }.is_ok()
}

/// Opens the context menu that `adapter`'s window asked for, and activates the chosen entry.
pub(super) fn open_context_menu(adapter: &Win32WindowAdapter, hwnd: HWND) {
    let Some(PendingContextMenu { menu, position }) = adapter.context_menu.take() else { return };
    let Ok(hmenu) = (unsafe { CreatePopupMenu() }) else { return };
    let native = NativeMenu::build(&menu, hmenu);
    let scale = adapter.window.scale_factor();
    let margin = adapter.margin();
    let mut point = POINT {
        x: (position.x * scale).round() as i32 + margin.left,
        y: (position.y * scale).round() as i32 + margin.top,
    };
    // A popup window doesn't take activation, so its top-level window owns the menu.
    let owner = adapter.top_level_hwnd().unwrap_or(hwnd);
    let command = unsafe {
        let _ = ClientToScreen(hwnd, &mut point);
        // Without it, the menu doesn't close on a click elsewhere (KB 135788).
        let _ = SetForegroundWindow(owner);
        TrackPopupMenuEx(hmenu, (TPM_LEFTALIGN | TPM_RETURNCMD).0, point.x, point.y, owner, None)
    };
    if let Some(entry) = native.entry(command.0 as usize).cloned() {
        VRc::borrow(&menu).activate(&entry);
    }
}
