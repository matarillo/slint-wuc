// Copyright © Kentaro Inomata
// SPDX-License-Identifier: MIT-0

//! Drags from other programs, as OLE hands them to the drop target of a hidden HWND, and what
//! reaches its `DropArea`.
//!
//! The test calls the drop target as OLE does, with a data object of its own, so no mouse
//! moves and no other program takes part.
//!
//! The platform belongs to the thread that first uses it, so one test runs every scenario.

use core::mem::ManuallyDrop;
use i_slint_core::host::Win32WindowAdapter;
use slint::ComponentHandle;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::System::Com::*;
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::System::Ole::*;
use windows::Win32::System::SystemServices::{MK_SHIFT, MODIFIERKEYS_FLAGS};
use windows::Win32::UI::Shell::DROPFILES;
use windows_core::{BOOL, HRESULT, Ref, implement};

slint::include_modules!();

const NO_KEYS: MODIFIERKEYS_FLAGS = MODIFIERKEYS_FLAGS(0);
const ANY_EFFECT: DROPEFFECT =
    DROPEFFECT(DROPEFFECT_COPY.0 | DROPEFFECT_MOVE.0 | DROPEFFECT_LINK.0);

/// A data object that gives files as `CF_HDROP` and text as `CF_UNICODETEXT`, as Explorer
/// and editors do.
#[implement(IDataObject)]
struct Data {
    files: Vec<&'static str>,
    text: Option<&'static str>,
}

impl Data {
    fn files(files: &[&'static str]) -> IDataObject {
        Self { files: files.to_vec(), text: None }.into()
    }

    fn text(text: &'static str) -> IDataObject {
        Self { files: Vec::new(), text: Some(text) }.into()
    }

    /// The bytes of `format`, if the object gives it.
    fn bytes(&self, format: CLIPBOARD_FORMAT) -> Option<Vec<u8>> {
        let wide = |text: &str| text.encode_utf16().chain([0]).collect::<Vec<u16>>();
        let units = if format == CF_HDROP && !self.files.is_empty() {
            // The paths follow the header, each with its NUL, and an empty one ends them.
            let mut units: Vec<u16> = self.files.iter().flat_map(|f| wide(f)).collect();
            units.push(0);
            let header = DROPFILES {
                pFiles: size_of::<DROPFILES>() as u32,
                pt: POINT::default(),
                fNC: BOOL(0),
                fWide: BOOL(1),
            };
            // SAFETY: DROPFILES is plain data.
            let header = unsafe {
                core::slice::from_raw_parts(
                    (&header as *const DROPFILES).cast::<u8>(),
                    size_of::<DROPFILES>(),
                )
            };
            let mut bytes = header.to_vec();
            bytes.extend(units.iter().flat_map(|u| u.to_le_bytes()));
            return Some(bytes);
        } else if format == CF_UNICODETEXT {
            wide(self.text?)
        } else {
            return None;
        };
        Some(units.iter().flat_map(|u| u.to_le_bytes()).collect())
    }
}

impl IDataObject_Impl for Data_Impl {
    fn GetData(&self, request: *const FORMATETC) -> windows_core::Result<STGMEDIUM> {
        let request = unsafe { &*request };
        if request.tymed & TYMED_HGLOBAL.0 as u32 == 0 {
            return Err(DV_E_TYMED.into());
        }
        let bytes = self.bytes(CLIPBOARD_FORMAT(request.cfFormat)).ok_or(DV_E_FORMATETC)?;
        unsafe {
            let memory = GlobalAlloc(GMEM_MOVEABLE, bytes.len())?;
            let destination = GlobalLock(memory).cast::<u8>();
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), destination, bytes.len());
            let _ = GlobalUnlock(memory);
            // The receiver frees the memory with `ReleaseStgMedium`.
            Ok(STGMEDIUM {
                tymed: TYMED_HGLOBAL.0 as u32,
                u: STGMEDIUM_0 { hGlobal: memory },
                pUnkForRelease: ManuallyDrop::new(None),
            })
        }
    }

    fn GetDataHere(&self, _: *const FORMATETC, _: *mut STGMEDIUM) -> windows_core::Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn QueryGetData(&self, request: *const FORMATETC) -> HRESULT {
        match self.bytes(CLIPBOARD_FORMAT(unsafe { &*request }.cfFormat)) {
            Some(_) => S_OK,
            None => DV_E_FORMATETC,
        }
    }

    fn GetCanonicalFormatEtc(&self, _: *const FORMATETC, _: *mut FORMATETC) -> HRESULT {
        E_NOTIMPL
    }

    fn SetData(
        &self,
        _: *const FORMATETC,
        _: *const STGMEDIUM,
        _: BOOL,
    ) -> windows_core::Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn EnumFormatEtc(&self, _: u32) -> windows_core::Result<IEnumFORMATETC> {
        Err(E_NOTIMPL.into())
    }

    fn DAdvise(
        &self,
        _: *const FORMATETC,
        _: u32,
        _: Ref<IAdviseSink>,
    ) -> windows_core::Result<u32> {
        Err(OLE_E_ADVISENOTSUPPORTED.into())
    }

    fn DUnadvise(&self, _: u32) -> windows_core::Result<()> {
        Err(OLE_E_ADVISENOTSUPPORTED.into())
    }

    fn EnumDAdvise(&self) -> windows_core::Result<IEnumSTATDATA> {
        Err(OLE_E_ADVISENOTSUPPORTED.into())
    }
}

struct Fixture {
    ui: DropTest,
    hwnd: HWND,
    target: IDropTarget,
}

impl Fixture {
    fn new() -> Self {
        let ui = DropTest::new().unwrap();
        ui.on_describe(|data| {
            let mut parts = Vec::new();
            if let Ok(paths) = data.file_paths() {
                let paths: Vec<_> = paths.map(|p| p.display().to_string()).collect();
                parts.push(format!("files: {}", paths.join("|")));
            }
            if let Ok(text) = data.plain_text() {
                parts.push(format!("text: {text}"));
            }
            parts.join("; ").into()
        });
        let adapter = Win32WindowAdapter::from_window(ui.window()).unwrap();
        let hwnd = adapter.create_hwnd_for_testing().unwrap();
        // OLE took the window's target, so OLE works on the thread that the Windows Runtime
        // initialized.
        let target = adapter.drop_target_for_testing().expect("the window takes drops");
        Self { ui, hwnd, target }
    }

    /// The screen point of a logical position in the client area.
    fn screen(&self, x: f32, y: f32) -> POINTL {
        let scale = self.ui.window().scale_factor();
        let mut point = POINT { x: (x * scale).round() as i32, y: (y * scale).round() as i32 };
        unsafe { ClientToScreen(self.hwnd, &mut point).unwrap() };
        POINTL { x: point.x, y: point.y }
    }

    /// The effect that the window answers when a drag that allows `allowed` enters at a
    /// logical position.
    fn enter(
        &self,
        data: &IDataObject,
        keys: MODIFIERKEYS_FLAGS,
        allowed: DROPEFFECT,
        x: f32,
        y: f32,
    ) -> DROPEFFECT {
        let mut effect = allowed;
        unsafe { self.target.DragEnter(data, keys, self.screen(x, y), &mut effect) }.unwrap();
        effect
    }

    fn over(&self, keys: MODIFIERKEYS_FLAGS, allowed: DROPEFFECT, x: f32, y: f32) -> DROPEFFECT {
        let mut effect = allowed;
        unsafe { self.target.DragOver(keys, self.screen(x, y), &mut effect) }.unwrap();
        effect
    }

    fn drop(
        &self,
        data: &IDataObject,
        keys: MODIFIERKEYS_FLAGS,
        allowed: DROPEFFECT,
        x: f32,
        y: f32,
    ) -> DROPEFFECT {
        let mut effect = allowed;
        unsafe { self.target.Drop(data, keys, self.screen(x, y), &mut effect) }.unwrap();
        effect
    }

    fn leave(&self) {
        unsafe { self.target.DragLeave() }.unwrap();
    }
}

#[test]
fn drops_from_other_programs() {
    let f = Fixture::new();

    files_are_copied_unless_the_keys_ask_otherwise(&f);
    text_comes_with_newlines(&f);
    a_drag_outside_the_area_is_refused(&f);
    the_proposal_falls_back_to_what_the_source_allows(&f);
    an_area_that_refuses_shows_no_effect(&f);
}

fn files_are_copied_unless_the_keys_ask_otherwise(f: &Fixture) {
    let data = Data::files(&[r"C:\Data\a.txt", r"C:\Data\b c.png"]);
    // Explorer allows every effect; without keys, the drag copies.
    assert_eq!(f.enter(&data, NO_KEYS, ANY_EFFECT, 50.0, 50.0), DROPEFFECT_COPY);
    assert!(f.ui.get_has_drag());
    assert_eq!(f.ui.get_proposed(), "copy");
    assert_eq!(f.ui.get_hovered(), r"files: C:\Data\a.txt|C:\Data\b c.png");

    assert_eq!(f.over(MK_SHIFT, ANY_EFFECT, 60.0, 40.0), DROPEFFECT_MOVE);
    assert_eq!(f.ui.get_proposed(), "move");

    // Multiples of 4 logical pixels are whole physical pixels at 125%, 150% and 175%.
    assert_eq!(f.drop(&data, NO_KEYS, ANY_EFFECT, 40.0, 32.0), DROPEFFECT_COPY);
    assert_eq!(f.ui.get_dropped(), r"files: C:\Data\a.txt|C:\Data\b c.png");
    assert_eq!((f.ui.get_drop_x(), f.ui.get_drop_y()), (40.0, 32.0));
    assert!(!f.ui.get_has_drag());
}

fn text_comes_with_newlines(f: &Fixture) {
    f.ui.set_dropped("".into());
    let data = Data::text("one\r\ntwo");
    assert_eq!(f.enter(&data, NO_KEYS, ANY_EFFECT, 10.0, 10.0), DROPEFFECT_COPY);
    assert_eq!(f.ui.get_hovered(), "text: one\ntwo");
    // The drag goes back to its program: nothing is dropped.
    f.leave();
    assert!(!f.ui.get_has_drag());
    assert_eq!(f.ui.get_dropped(), "");
}

fn a_drag_outside_the_area_is_refused(f: &Fixture) {
    let data = Data::files(&[r"C:\Data\a.txt"]);
    assert_eq!(f.enter(&data, NO_KEYS, ANY_EFFECT, 150.0, 150.0), DROPEFFECT_NONE);
    assert!(!f.ui.get_has_drag());
    assert_eq!(f.over(NO_KEYS, ANY_EFFECT, 20.0, 20.0), DROPEFFECT_COPY);
    assert!(f.ui.get_has_drag());
    assert_eq!(f.over(NO_KEYS, ANY_EFFECT, 150.0, 20.0), DROPEFFECT_NONE);
    assert!(!f.ui.get_has_drag());
    f.leave();
}

fn the_proposal_falls_back_to_what_the_source_allows(f: &Fixture) {
    let data = Data::files(&[r"C:\Data\a.txt"]);
    assert_eq!(f.enter(&data, NO_KEYS, DROPEFFECT_MOVE, 50.0, 50.0), DROPEFFECT_MOVE);
    assert_eq!(f.ui.get_proposed(), "move");
    assert_eq!(f.over(NO_KEYS, DROPEFFECT_LINK, 50.0, 50.0), DROPEFFECT_LINK);
    assert_eq!(f.ui.get_proposed(), "link");
    f.leave();
}

fn an_area_that_refuses_shows_no_effect(f: &Fixture) {
    f.ui.set_accept(false);
    let data = Data::files(&[r"C:\Data\a.txt"]);
    assert_eq!(f.enter(&data, NO_KEYS, ANY_EFFECT, 50.0, 50.0), DROPEFFECT_NONE);
    assert!(!f.ui.get_has_drag());
    f.leave();
    f.ui.set_accept(true);
}
