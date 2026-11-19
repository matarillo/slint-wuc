// Copyright © Kentaro Inomata
// SPDX-License-Identifier: MIT-0

//! Drops from other programs: an OLE drop target turns what Explorer and other programs drag
//! over a window into the core's drag events, which reach the `DropArea`s as upstream's Qt
//! backend delivers Qt's drags (`docs/design/input.md`).

use super::Win32WindowAdapter;
use crate::data_transfer::DataTransfer;
use crate::input::BackendDragEvent;
use crate::items::{AllowedDragActions, DragAction, DropEvent};
use alloc::rc::Weak;
use alloc::vec::Vec;
use core::cell::{OnceCell, RefCell};
use std::os::windows::ffi::OsStringExt;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::System::Com::{
    DVASPECT_CONTENT, FORMATETC, IDataObject, STGMEDIUM, TYMED_HGLOBAL,
};
use windows::Win32::System::Ole::{
    CF_HDROP, CF_UNICODETEXT, CLIPBOARD_FORMAT, DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_LINK,
    DROPEFFECT_MOVE, DROPEFFECT_NONE, IDropTarget, IDropTarget_Impl, OleInitialize,
    RegisterDragDrop, ReleaseStgMedium, RevokeDragDrop,
};
use windows::Win32::System::SystemServices::{MK_CONTROL, MK_SHIFT, MODIFIERKEYS_FLAGS};
use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};
use windows_core::{Ref, implement};

/// The Alt key in the key state of a drag. OLE gives it the bit that mouse messages give
/// `MK_XBUTTON1`, so the windows crate has no name for it.
const MK_ALT: MODIFIERKEYS_FLAGS = MODIFIERKEYS_FLAGS(0x20);

std::thread_local! {
    /// Whether OLE is initialized on this thread, which drop targets need.
    static OLE: OnceCell<bool> = const { OnceCell::new() };
}

/// Initializes OLE on this thread the first time a window asks. It is never uninitialized, as
/// the Windows Runtime isn't.
fn ole_initialized() -> bool {
    OLE.with(|ole| {
        *ole.get_or_init(|| {
            // The Windows Runtime made the thread single-threaded already, which OLE wants;
            // it fails on a thread that the application made multithreaded.
            match unsafe { OleInitialize(None) } {
                Ok(()) => true,
                Err(e) => {
                    crate::debug_log!(
                        "cannot initialize OLE, windows take no drops: {}",
                        e.message()
                    );
                    false
                }
            }
        })
    })
}

/// Makes `hwnd` take drops for `adapter`, its top-level window. The target stays registered
/// until [`revoke`].
pub(super) fn register(adapter: &Win32WindowAdapter, hwnd: HWND) -> Option<IDropTarget> {
    if !ole_initialized() {
        return None;
    }
    let target: IDropTarget =
        DropTarget { adapter: adapter.self_weak.clone(), data: Default::default() }.into();
    match unsafe { RegisterDragDrop(hwnd, &target) } {
        Ok(()) => Some(target),
        Err(e) => {
            crate::debug_log!("the window takes no drops: {}", e.message());
            None
        }
    }
}

/// Releases the drop target of `hwnd`, which is being destroyed; nothing if it has none.
pub(super) fn revoke(hwnd: HWND) {
    let _ = unsafe { RevokeDragDrop(hwnd) };
}

#[implement(IDropTarget)]
struct DropTarget {
    adapter: Weak<Win32WindowAdapter>,
    /// What the drag over the window carries, read when it enters.
    data: RefCell<Option<DataTransfer>>,
}

impl IDropTarget_Impl for DropTarget_Impl {
    fn DragEnter(
        &self,
        data: Ref<IDataObject>,
        keys: MODIFIERKEYS_FLAGS,
        point: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows_core::Result<()> {
        *self.data.borrow_mut() = Some(data.as_ref().map(read).unwrap_or_default());
        self.drag(keys, point, effect, false)
    }

    fn DragOver(
        &self,
        keys: MODIFIERKEYS_FLAGS,
        point: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows_core::Result<()> {
        self.drag(keys, point, effect, false)
    }

    fn DragLeave(&self) -> windows_core::Result<()> {
        self.data.take();
        if let Some(adapter) = self.adapter.upgrade() {
            adapter.inner().process_drag_event(BackendDragEvent::Leave);
        }
        Ok(())
    }

    fn Drop(
        &self,
        _data: Ref<IDataObject>,
        keys: MODIFIERKEYS_FLAGS,
        point: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows_core::Result<()> {
        // The object is the one that entered, which `data` holds already.
        let result = self.drag(keys, point, effect, true);
        self.data.take();
        result
    }
}

impl DropTarget {
    /// Hands a drag at `point` on the screen to the window, and tells OLE in `effect` the
    /// action that the `DropArea` under it chose, from those that `effect` allows.
    fn drag(
        &self,
        keys: MODIFIERKEYS_FLAGS,
        point: &POINTL,
        effect: *mut DROPEFFECT,
        drop: bool,
    ) -> windows_core::Result<()> {
        if effect.is_null() {
            return Err(E_INVALIDARG.into());
        }
        // SAFETY: OLE passes the effects that the source allows, and takes back the chosen one.
        let effect = unsafe { &mut *effect };
        let allowed = allowed_actions(*effect);
        let chosen = self.dispatch(keys, point, allowed, drop).unwrap_or(DragAction::None);
        *effect = drop_effect(chosen);
        Ok(())
    }

    fn dispatch(
        &self,
        keys: MODIFIERKEYS_FLAGS,
        point: &POINTL,
        allowed: AllowedDragActions,
        drop: bool,
    ) -> Option<DragAction> {
        let adapter = self.adapter.upgrade()?;
        let hwnd = adapter.hwnd.get()?;
        let mut client = POINT { x: point.x, y: point.y };
        unsafe { ScreenToClient(hwnd, &mut client) }.ok().ok()?;
        let event = DropEvent {
            // Cloned, so that no borrow is held while the `DropArea`s run.
            data: self.data.borrow().clone().unwrap_or_default(),
            position: crate::lengths::logical_position_to_api(adapter.logical_point(client)),
            proposed_action: proposed_action(keys, allowed),
        };
        let event = if drop {
            BackendDragEvent::Drop { event, allowed }
        } else {
            BackendDragEvent::Move { event, allowed }
        };
        adapter.inner().process_drag_event(event)
    }
}

fn allowed_actions(effect: DROPEFFECT) -> AllowedDragActions {
    AllowedDragActions {
        copy: effect.0 & DROPEFFECT_COPY.0 != 0,
        move_: effect.0 & DROPEFFECT_MOVE.0 != 0,
        link: effect.0 & DROPEFFECT_LINK.0 != 0,
    }
}

fn drop_effect(action: DragAction) -> DROPEFFECT {
    match action {
        DragAction::None => DROPEFFECT_NONE,
        DragAction::Copy => DROPEFFECT_COPY,
        DragAction::Move => DROPEFFECT_MOVE,
        DragAction::Link => DROPEFFECT_LINK,
    }
}

/// The action that the keys ask for, as upstream's Qt backend proposes for a drag from another
/// program (`QPlatformDrag::defaultAction`): Ctrl+Shift links, Ctrl copies, Shift moves, Alt
/// links, and no key copies. An action that the source doesn't allow falls back to copy, move,
/// and link, in this order.
///
/// A drag inside the window proposes a move first; one from another program doesn't, since a
/// move that Explorer starts deletes the files once the target has them.
fn proposed_action(keys: MODIFIERKEYS_FLAGS, allowed: AllowedDragActions) -> DragAction {
    let is_allowed = |action| match action {
        DragAction::Copy => allowed.copy,
        DragAction::Move => allowed.move_,
        DragAction::Link => allowed.link,
        DragAction::None => false,
    };
    let requested = match (keys.contains(MK_CONTROL), keys.contains(MK_SHIFT)) {
        (true, true) => DragAction::Link,
        (true, false) => DragAction::Copy,
        (false, true) => DragAction::Move,
        (false, false) if keys.contains(MK_ALT) => DragAction::Link,
        (false, false) => DragAction::Copy,
    };
    [requested, DragAction::Copy, DragAction::Move, DragAction::Link]
        .into_iter()
        .find(|&action| is_allowed(action))
        .unwrap_or(DragAction::None)
}

/// The files and the text that `data` carries.
fn read(data: &IDataObject) -> DataTransfer {
    let mut transfer = DataTransfer::default();
    if let Some(paths) = global(data, CF_HDROP).and_then(|medium| file_paths(&medium))
        && !paths.is_empty()
    {
        transfer.set_file_paths(paths);
    }
    // SAFETY: the medium keeps the memory until it is dropped.
    if let Some(text) = global(data, CF_UNICODETEXT)
        .and_then(|medium| unsafe { super::text_from_global(medium.global()) })
        && !text.is_empty()
    {
        transfer.set_plain_text(text.into());
    }
    transfer
}

/// The paths of an `HDROP`.
fn file_paths(medium: &Medium) -> Option<Vec<std::path::PathBuf>> {
    let drop = HDROP(medium.global().0);
    let count = unsafe { DragQueryFileW(drop, u32::MAX, None) };
    let mut paths = Vec::with_capacity(count as usize);
    for index in 0..count {
        let length = unsafe { DragQueryFileW(drop, index, None) } as usize;
        let mut buffer = alloc::vec![0u16; length + 1];
        let copied = unsafe { DragQueryFileW(drop, index, Some(&mut buffer)) } as usize;
        if copied == 0 {
            return None;
        }
        paths.push(std::ffi::OsString::from_wide(&buffer[..copied]).into());
    }
    Some(paths)
}

/// What `data` gives in `format`, if it gives it in global memory.
fn global(data: &IDataObject, format: CLIPBOARD_FORMAT) -> Option<Medium> {
    let request = FORMATETC {
        cfFormat: format.0,
        ptd: core::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    };
    let medium = Medium(unsafe { data.GetData(&request) }.ok()?);
    (medium.0.tymed == TYMED_HGLOBAL.0 as u32).then_some(medium)
}

/// A storage medium that a data object gave, released when dropped.
struct Medium(STGMEDIUM);

impl Medium {
    fn global(&self) -> HGLOBAL {
        // SAFETY: `global` gives out only media in global memory.
        unsafe { self.0.u.hGlobal }
    }
}

impl Drop for Medium {
    fn drop(&mut self) {
        unsafe { ReleaseStgMedium(&mut self.0) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: AllowedDragActions = AllowedDragActions { copy: true, move_: true, link: true };

    #[test]
    fn keys_choose_the_proposed_action() {
        let keys = |bits| MODIFIERKEYS_FLAGS(bits);
        assert_eq!(proposed_action(keys(0), ALL), DragAction::Copy);
        assert_eq!(proposed_action(MK_CONTROL, ALL), DragAction::Copy);
        assert_eq!(proposed_action(MK_SHIFT, ALL), DragAction::Move);
        assert_eq!(proposed_action(keys(MK_CONTROL.0 | MK_SHIFT.0), ALL), DragAction::Link);
        assert_eq!(proposed_action(MK_ALT, ALL), DragAction::Link);
    }

    #[test]
    fn an_action_the_source_does_not_allow_falls_back() {
        let move_only = AllowedDragActions { copy: false, move_: true, link: false };
        let link_only = AllowedDragActions { copy: false, move_: false, link: true };
        assert_eq!(proposed_action(MODIFIERKEYS_FLAGS(0), move_only), DragAction::Move);
        assert_eq!(proposed_action(MK_CONTROL, link_only), DragAction::Link);
        assert_eq!(proposed_action(MK_SHIFT, AllowedDragActions::default()), DragAction::None);
    }

    #[test]
    fn effects_convert_both_ways() {
        let allowed = allowed_actions(DROPEFFECT(DROPEFFECT_COPY.0 | DROPEFFECT_LINK.0));
        assert_eq!(allowed, AllowedDragActions { copy: true, move_: false, link: true });
        assert_eq!(drop_effect(DragAction::Move), DROPEFFECT_MOVE);
        assert_eq!(drop_effect(DragAction::None), DROPEFFECT_NONE);
    }
}
