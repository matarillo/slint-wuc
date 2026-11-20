// Copyright © Kentaro Inomata
// SPDX-License-Identifier: MIT-0

//! `--web`: a WebView2 in composition hosting mode, drawing into a container visual of the
//! window's compositor, shown in the `web` image (roadmap item 4). Roadmap item 6: the
//! application forwards the input of that item to it, with only what an application has:
//! the item's `TouchArea` and `FocusScope`, and the window's `HWND`.
//!
//! `--input-check <dir>` posts mouse and keyboard messages to the probe's own window (and keys
//! to the WebView2's window that has the focus), prints what the page and Slint got, and quits.

use super::Embed;
use i_slint_core::cursor::MouseCursorInner;
use i_slint_core::items::{BuiltInMouseCursor, PointerEvent, PointerEventButton, PointerEventKind};
use slint::ComponentHandle;
use std::cell::{Cell, RefCell};
use std::time::Duration;
use webview2_com::Microsoft::Web::WebView2::Win32::*;
use webview2_com::{
    CreateCoreWebView2CompositionControllerCompletedHandler,
    CreateCoreWebView2EnvironmentCompletedHandler, CursorChangedEventHandler,
    ExecuteScriptCompletedHandler, MoveFocusRequestedEventHandler,
};
use windows::UI::Composition::Compositor;
use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetFocus, GetKeyState, SetFocus, VK_SHIFT};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{HSTRING, Interface, PCWSTR};

/// winuser.h's `MK_` bits of mouse messages; the windows crate has them in `SystemServices`.
const MK_LBUTTON: u32 = 0x1;
const MK_RBUTTON: u32 = 0x2;
const MK_SHIFT: u32 = 0x4;
const MK_CONTROL: u32 = 0x8;
const MK_MBUTTON: u32 = 0x10;
const MK_XBUTTON1: u32 = 0x20;
const MK_XBUTTON2: u32 = 0x40;

/// Logical size of the page, as item D in `ui/embed.slint`.
const WIDTH: f32 = 320.;
const HEIGHT: f32 = 240.;

const PAGE: &str = r##"<!doctype html><body style="margin:0;background:#f4f4f4;font:14px 'Segoe UI';overflow:hidden">
<p style="margin:8px">WebView2 in a visual</p>
<div style="position:absolute;top:110px;width:30px;height:30px;background:#e33;border-radius:15px;
animation:m 1s infinite alternate linear"></div>
<style>@keyframes m{from{left:10px}to{left:110px}}</style>
<button id=b style="position:absolute;left:150px;top:8px;width:70px;height:28px">Click</button>
<a id=l href="#" style="position:absolute;left:235px;top:14px">link</a>
<input id=t style="position:absolute;left:150px;top:44px;width:150px">
<select id=s style="position:absolute;left:150px;top:76px;width:156px">
<option>one<option>two<option>three</select>
<div id=d style="position:absolute;left:10px;top:150px;width:296px;height:56px;overflow:auto;background:#ddd">
<div style="height:400px">scroll me</div></div>
<div id=log style="position:absolute;left:10px;top:214px;font-size:12px"></div>
<script>
const s={clicks:0,dbl:0,downs:[],wheel:0,keys:''};
b.onclick=()=>{s.clicks++;show()};
b.ondblclick=()=>{s.dbl++;show()};
addEventListener('mousedown',e=>{s.downs.push([e.button,e.clientX,e.clientY,e.target.id||e.target.tagName]);show()});
addEventListener('wheel',e=>{s.wheel+=e.deltaY});
addEventListener('keydown',e=>{s.keys+=e.key+' '});
t.oninput=show;
function show(){log.textContent=`clicks ${s.clicks} dbl ${s.dbl} downs ${s.downs.length} [${t.value}]`}
window.state=()=>JSON.stringify({...s,value:t.value,scroll:d.scrollTop,
 focus:document.activeElement.id||document.activeElement.tagName,hasFocus:document.hasFocus()});
</script></body>"##;

struct Web {
    hwnd: HWND,
    composition: ICoreWebView2CompositionController,
    controller: ICoreWebView2Controller3,
    /// The `MK_` bits of the buttons held, as the item's pointer events report them.
    buttons: Cell<u32>,
}

std::thread_local! {
    /// The WebView2 lives as long as its controller.
    static WEB: RefCell<Option<Web>> = const { RefCell::new(None) };
}

fn with_web<R>(f: impl FnOnce(&Web) -> R) -> Option<R> {
    WEB.with(|web| web.borrow().as_ref().map(f))
}

fn hwnd_of(app: &Embed) -> HWND {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let handle = app.window().window_handle();
    let RawWindowHandle::Win32(win32) = handle.window_handle().unwrap().as_raw() else {
        panic!("not a Win32 window");
    };
    HWND(win32.hwnd.get() as _)
}

pub fn start(app: &Embed) {
    let hwnd = hwnd_of(app);
    let weak = app.as_weak();
    let created =
        CreateCoreWebView2EnvironmentCompletedHandler::create(Box::new(move |hr, env| {
            hr?;
            let env: ICoreWebView2Environment3 = env.unwrap().cast()?;
            let shown = CreateCoreWebView2CompositionControllerCompletedHandler::create(Box::new(
                move |hr, controller| {
                    hr?;
                    let Some(app) = weak.upgrade() else { return Ok(()) };
                    let r = show(&app, hwnd, controller.unwrap());
                    if let Err(e) = &r {
                        println!("web: {e}");
                    }
                    r
                },
            ));
            unsafe { env.CreateCoreWebView2CompositionController(hwnd, &shown) }
        }));
    unsafe { CreateCoreWebView2Environment(&created) }.unwrap();
    connect(app);
}

fn show(
    app: &Embed,
    hwnd: HWND,
    composition: ICoreWebView2CompositionController,
) -> windows::core::Result<()> {
    let host = i_slint_core::host::Win32WindowAdapter::from_window(app.window()).unwrap();
    let raw = host.compositor_abi();
    let compositor = unsafe { Compositor::from_raw_borrowed(&raw) }.unwrap().clone();
    let container = compositor.CreateContainerVisual()?;
    // WebView2 draws in pixels; Slint's root visual scales logical pixels to physical ones.
    let scale = app.window().scale_factor();
    container.SetScale(windows_numerics::Vector3 { X: 1. / scale, Y: 1. / scale, Z: 1. })?;
    unsafe { composition.SetRootVisualTarget(&container)? };
    let controller: ICoreWebView2Controller3 = composition.cast()?;
    let (w, h) = ((WIDTH * scale).round() as i32, (HEIGHT * scale).round() as i32);
    unsafe {
        controller.SetShouldDetectMonitorScaleChanges(false)?;
        controller.SetRasterizationScale(scale as f64)?;
        controller.SetIsVisible(true)?;
        let page = HSTRING::from(PAGE);
        controller.CoreWebView2()?.NavigateToString(PCWSTR(page.as_ptr()))?;
    }
    let size = i_slint_core::graphics::IntSize::new(w as u32, h as u32);
    let image = unsafe {
        i_slint_core::visual_tree::image::external_visual_image(container.as_raw(), size)
    };
    app.set_web(image.unwrap());

    // The page's cursor becomes the item's cursor.
    let weak = app.as_weak();
    let cursor_changed = CursorChangedEventHandler::create(Box::new(move |sender, _| {
        let mut id = 0;
        unsafe { sender.unwrap().SystemCursorId(&mut id)? };
        let cursor = cursor_from_id(id);
        println!("web: cursor {id} -> {cursor:?}");
        if let Some(app) = weak.upgrade() {
            app.set_web_cursor(MouseCursorInner::BuiltIn(cursor));
        }
        Ok(())
    }));
    // Tab out of the page's last element, or Shift+Tab out of its first, moves Slint's focus.
    let weak = app.as_weak();
    let move_focus = MoveFocusRequestedEventHandler::create(Box::new(move |_, args| {
        let args = args.unwrap();
        let mut reason = COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC;
        unsafe { args.Reason(&mut reason)? };
        println!("web: move focus requested, {}", reason.0);
        let Some(app) = weak.upgrade() else { return Ok(()) };
        if reason == COREWEBVIEW2_MOVE_FOCUS_REASON_NEXT {
            app.invoke_focus_after();
        } else if reason == COREWEBVIEW2_MOVE_FOCUS_REASON_PREVIOUS {
            app.invoke_focus_before();
        }
        unsafe { args.SetHandled(true) }
    }));
    let context_menu =
        webview2_com::ContextMenuRequestedEventHandler::create(Box::new(|_, args| {
            let mut at = POINT::default();
            unsafe { args.unwrap().Location(&mut at)? };
            println!("web: context menu requested at {},{}", at.x, at.y);
            Ok(())
        }));
    let mut token = 0;
    unsafe {
        let core: ICoreWebView2_11 = controller.CoreWebView2()?.cast()?;
        core.add_ContextMenuRequested(&context_menu, &mut token)?;
        composition.add_CursorChanged(&cursor_changed, &mut token)?;
        controller.add_MoveFocusRequested(&move_focus, &mut token)?;
    }
    WEB.with(|web| {
        *web.borrow_mut() = Some(Web { hwnd, composition, controller, buttons: Cell::new(0) })
    });
    set_bounds(app);
    // Slint has no event for a move of the window; the popups need one.
    unsafe {
        let _ = windows::Win32::UI::Shell::SetWindowSubclass(hwnd, Some(moved), 1, 0);
    }
    println!("web: shown");
    Ok(())
}

/// Closes the WebView2 while the window is still there; dropped with the thread's locals
/// instead, it fails the process (0xC0000409) on the way out.
pub fn close() {
    if let Some(web) = WEB.with(|web| web.borrow_mut().take()) {
        unsafe {
            let _ = web.controller.Close();
        }
    }
}

/// Tells the WebView2 that the window moved, so that its popups open at the page.
unsafe extern "system" fn moved(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    _: usize,
    _: usize,
) -> windows::Win32::Foundation::LRESULT {
    if msg == WM_MOVE && std::env::var_os("PROBE_WEB_NO_MOVE").is_none() {
        with_web(|web| unsafe {
            let _ = web.controller.NotifyParentWindowPositionChanged();
        });
    }
    unsafe { windows::Win32::UI::Shell::DefSubclassProc(hwnd, msg, wp, lp) }
}

/// Places the WebView2 where item D is in the window, in pixels.
///
/// With `PROBE_WEB_BOUNDS=origin`, at the window's origin instead, to see what the offset changes.
fn set_bounds(app: &Embed) {
    let scale = app.window().scale_factor();
    let origin = std::env::var("PROBE_WEB_BOUNDS").is_ok_and(|v| v == "origin");
    let (x, y) = if origin { (0., 0.) } else { (app.get_web_x(), app.get_web_y()) };
    let (x, y) = ((x * scale).round() as i32, (y * scale).round() as i32);
    let (w, h) = ((WIDTH * scale).round() as i32, (HEIGHT * scale).round() as i32);
    with_web(|web| unsafe {
        let _ = web.controller.SetBounds(RECT { left: x, top: y, right: x + w, bottom: y + h });
    });
}

/// The `TouchArea` and `FocusScope` of item D, forwarded.
fn connect(app: &Embed) {
    let weak = app.as_weak();
    app.on_web_pointer(move |event, x, y| {
        let Some(app) = weak.upgrade() else { return };
        pointer(&app, &event, x, y);
    });
    let weak = app.as_weak();
    app.on_web_wheel(move |dx, dy, x, y| {
        let Some(app) = weak.upgrade() else { return };
        let point = point(&app, x, y);
        // The host made 60 logical pixels of a wheel notch of 120; the horizontal one inverted.
        let notch = |d: f32| (d / 60. * WHEEL_DELTA as f32).round() as i32 as u32;
        with_web(|web| unsafe {
            if dy != 0. {
                let _ = web.composition.SendMouseInput(
                    COREWEBVIEW2_MOUSE_EVENT_KIND_WHEEL,
                    keys(web.buttons.get()),
                    notch(dy),
                    point,
                );
            }
            if dx != 0. {
                let _ = web.composition.SendMouseInput(
                    COREWEBVIEW2_MOUSE_EVENT_KIND_HORIZONTAL_WHEEL,
                    keys(web.buttons.get()),
                    notch(-dx),
                    point,
                );
            }
        });
    });
    app.on_web_leave(|| {
        with_web(|web| unsafe {
            let _ = web.composition.SendMouseInput(
                COREWEBVIEW2_MOUSE_EVENT_KIND_LEAVE,
                COREWEBVIEW2_MOUSE_EVENT_VIRTUAL_KEYS_NONE,
                0,
                POINT::default(),
            );
        });
    });
    let weak = app.as_weak();
    app.on_web_moved(move || {
        if let Some(app) = weak.upgrade() {
            set_bounds(&app);
        }
    });
    app.on_web_focus_gained(|tab| {
        with_web(|web| {
            let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0;
            let reason = match (tab, shift) {
                (true, false) => COREWEBVIEW2_MOVE_FOCUS_REASON_NEXT,
                (true, true) => COREWEBVIEW2_MOVE_FOCUS_REASON_PREVIOUS,
                (false, _) => COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC,
            };
            println!("web: focus gained, move focus {}", reason.0);
            unsafe {
                let _ = web.controller.MoveFocus(reason);
            }
        });
    });
    app.on_web_focus_lost(|| {
        // The WebView2's window keeps the keyboard until the window takes it back.
        with_web(|web| unsafe {
            if GetFocus() != web.hwnd {
                println!("web: focus lost, keyboard back to the window");
                let _ = SetFocus(Some(web.hwnd));
            }
        });
    });
}

fn point(app: &Embed, x: f32, y: f32) -> POINT {
    let scale = app.window().scale_factor();
    POINT { x: (x * scale).round() as i32, y: (y * scale).round() as i32 }
}

fn keys(buttons: u32) -> COREWEBVIEW2_MOUSE_EVENT_VIRTUAL_KEYS {
    COREWEBVIEW2_MOUSE_EVENT_VIRTUAL_KEYS(buttons as i32)
}

fn pointer(app: &Embed, event: &PointerEvent, x: f32, y: f32) {
    let point = point(app, x, y);
    with_web(|web| {
        let (down, up, bit, data) = match event.button {
            PointerEventButton::Left => (
                COREWEBVIEW2_MOUSE_EVENT_KIND_LEFT_BUTTON_DOWN,
                COREWEBVIEW2_MOUSE_EVENT_KIND_LEFT_BUTTON_UP,
                MK_LBUTTON,
                0,
            ),
            PointerEventButton::Right => (
                COREWEBVIEW2_MOUSE_EVENT_KIND_RIGHT_BUTTON_DOWN,
                COREWEBVIEW2_MOUSE_EVENT_KIND_RIGHT_BUTTON_UP,
                MK_RBUTTON,
                0,
            ),
            PointerEventButton::Middle => (
                COREWEBVIEW2_MOUSE_EVENT_KIND_MIDDLE_BUTTON_DOWN,
                COREWEBVIEW2_MOUSE_EVENT_KIND_MIDDLE_BUTTON_UP,
                MK_MBUTTON,
                0,
            ),
            PointerEventButton::Back => (
                COREWEBVIEW2_MOUSE_EVENT_KIND_X_BUTTON_DOWN,
                COREWEBVIEW2_MOUSE_EVENT_KIND_X_BUTTON_UP,
                MK_XBUTTON1,
                XBUTTON1 as u32,
            ),
            PointerEventButton::Forward => (
                COREWEBVIEW2_MOUSE_EVENT_KIND_X_BUTTON_DOWN,
                COREWEBVIEW2_MOUSE_EVENT_KIND_X_BUTTON_UP,
                MK_XBUTTON2,
                XBUTTON2 as u32,
            ),
            _ => (COREWEBVIEW2_MOUSE_EVENT_KIND_MOVE, COREWEBVIEW2_MOUSE_EVENT_KIND_MOVE, 0, 0),
        };
        let mut buttons = web.buttons.get();
        let kind = match event.kind {
            PointerEventKind::Down => {
                buttons |= bit;
                // No `..._DOUBLE_CLICK` kinds: the page counts clicks by itself.
                down
            }
            PointerEventKind::Up => {
                buttons &= !bit;
                up
            }
            PointerEventKind::Move => COREWEBVIEW2_MOUSE_EVENT_KIND_MOVE,
            // The press ended without a release: another window took the mouse.
            _ => {
                buttons = 0;
                COREWEBVIEW2_MOUSE_EVENT_KIND_LEAVE
            }
        };
        web.buttons.set(buttons);
        let mut keys = buttons;
        if event.modifiers.control {
            keys |= MK_CONTROL;
        }
        if event.modifiers.shift {
            keys |= MK_SHIFT;
        }
        let data = if kind == COREWEBVIEW2_MOUSE_EVENT_KIND_MOVE { 0 } else { data };
        unsafe {
            if let Err(e) = web.composition.SendMouseInput(kind, self::keys(keys), data, point) {
                println!("web: SendMouseInput({}, {keys:#x}, {data}): {e}", kind.0);
            }
            if kind != COREWEBVIEW2_MOUSE_EVENT_KIND_MOVE {
                println!("web: mouse {} {keys:#x} at {},{}", kind.0, point.x, point.y);
            }
        }
    });
}

/// The Slint cursor for a system cursor; the page's own images (`cursor: url(...)`) come as 0.
fn cursor_from_id(id: u32) -> BuiltInMouseCursor {
    use BuiltInMouseCursor as C;
    match id {
        32513 => C::Text,
        32514 => C::Wait,
        32515 => C::Crosshair,
        32642 => C::NwseResize,
        32643 => C::NeswResize,
        32644 => C::EwResize,
        32645 => C::NsResize,
        32646 => C::Move,
        32648 => C::NotAllowed,
        32649 => C::Pointer,
        32650 => C::Progress,
        32651 => C::Help,
        _ => C::Default,
    }
}

// --- `--input-check` ---

/// One step: what to post, and what to print after it.
type Step = Box<dyn FnOnce(&Embed)>;

pub fn input_check(app: &Embed, dir: std::path::PathBuf) {
    let hwnd = hwnd_of(app);
    // Points in the page, in logical pixels: the button, the link, the text field, the
    // scrolled box, and a point under the red rectangle.
    let at = move |app: &Embed, x: f32, y: f32| -> (i32, i32) {
        let s = app.window().scale_factor();
        (((app.get_web_x() + x) * s).round() as i32, ((app.get_web_y() + y) * s).round() as i32)
    };
    let mv = move |p: (i32, i32)| post(hwnd, WM_MOUSEMOVE, 0, p);
    let click = move |p: (i32, i32)| {
        post(hwnd, WM_MOUSEMOVE, 0, p);
        post(hwnd, WM_LBUTTONDOWN, MK_LBUTTON as usize, p);
        post(hwnd, WM_LBUTTONUP, 0, p);
    };
    let steps: Vec<(u64, Step)> = vec![
        (
            4000,
            Box::new(move |app| {
                children(hwnd);
                popups(app, hwnd, "nothing open", (60., 180.));
            }),
        ),
        (300, Box::new(move |app| click(at(app, 185., 22.)))),
        (
            300,
            Box::new(move |app| {
                state(app, "after a click on the button");
                // Down, up, down, up at once: a double click.
                click(at(app, 185., 22.));
                click(at(app, 185., 22.));
            }),
        ),
        (
            300,
            Box::new(move |app| {
                state(app, "after a double click on the button");
                click(at(app, 60., 60.));
            }),
        ),
        (
            300,
            Box::new(move |app| {
                state(app, "after a click under the red rectangle");
                mv(at(app, 245., 22.));
            }),
        ),
        (300, Box::new(move |app| mv(at(app, 200., 56.)))),
        (
            300,
            Box::new(move |app| {
                // Out of the item, to the swap chain on the left.
                mv(at(app, -200., 0.));
            }),
        ),
        (
            300,
            Box::new(move |app| {
                let (x, y) = at(app, 150., 180.);
                let mut screen = POINT { x, y };
                unsafe {
                    let _ = ClientToScreen(hwnd, &mut screen);
                }
                mv((x, y));
                let notch = (-(WHEEL_DELTA as i32) as u16 as usize) << 16;
                post(hwnd, WM_MOUSEWHEEL, notch, (screen.x, screen.y));
            }),
        ),
        (
            300,
            Box::new(move |app| {
                state(app, "after a wheel notch down over the box");
                click(at(app, 200., 56.));
            }),
        ),
        (
            500,
            Box::new(move |_| {
                let focus = unsafe { GetFocus() };
                println!("focus after a click on the text field: {}", class_of(focus));
                for c in "ab".chars() {
                    post(focus, WM_CHAR, c as usize, (0, 0));
                }
            }),
        ),
        (
            300,
            Box::new(move |app| {
                state(app, "after WM_CHAR a, b to that window");
                // Tab to the select, Tab out of the page.
                key(unsafe { GetFocus() }, VK_TAB_CODE);
            }),
        ),
        (
            300,
            Box::new(move |app| {
                state(app, "after Tab");
                key(unsafe { GetFocus() }, VK_TAB_CODE);
            }),
        ),
        (
            300,
            Box::new(move |app| {
                state(app, "after Tab, Tab");
                // The scrolled box takes the focus too.
                key(unsafe { GetFocus() }, VK_TAB_CODE);
            }),
        ),
        (
            500,
            Box::new(move |app| {
                state(app, "after Tab, Tab, Tab");
                let focus = unsafe { GetFocus() };
                println!("focus after Tab, Tab, Tab: {}", class_of(focus));
                post(focus, WM_CHAR, 'x' as usize, (0, 0));
            }),
        ),
        (
            300,
            Box::new(move |app| {
                println!("after: {:?}", app.get_after_text());
                // Click Slint's first text field, type, Tab into the page.
                let s = app.window().scale_factor();
                let y = 501. + 11. + app.get_scroll();
                click((((20. + 10.) * s).round() as i32, (y * s).round() as i32));
            }),
        ),
        (
            300,
            Box::new(move |_| {
                let focus = unsafe { GetFocus() };
                println!("focus after a click on Slint's text field: {}", class_of(focus));
                post(focus, WM_CHAR, 'y' as usize, (0, 0));
            }),
        ),
        (
            300,
            Box::new(move |app| {
                println!("before: {:?}", app.get_before_text());
                // Tab from `before`: the next in Slint's chain is item D.
                key(hwnd, VK_TAB_CODE);
            }),
        ),
        (
            500,
            Box::new(move |app| {
                let focus = unsafe { GetFocus() };
                println!("focus after Tab from Slint's text field: {}", class_of(focus));
                state(app, "after Tab into the page");
                app.set_scroll(-150.);
            }),
        ),
        (
            500,
            Box::new(move |app| {
                println!("scrolled: item D at {}, {}", app.get_web_x(), app.get_web_y());
                click(at(app, 185., 22.));
            }),
        ),
        (
            300,
            Box::new(move |app| {
                state(app, "after a click on the button, scrolled");
                click(at(app, 228., 88.));
            }),
        ),
        (
            800,
            Box::new(move |app| {
                popups(app, hwnd, "the select, scrolled", (150., 100.));
                // A click elsewhere closes it; a posted Escape doesn't.
                click(at(app, 60., 190.));
            }),
        ),
        (
            300,
            Box::new(move |_| {
                // Move the window by (100, 50) pixels; Slint sees no event for it.
                let mut r = RECT::default();
                unsafe {
                    let _ = GetWindowRect(hwnd, &mut r);
                    let _ = SetWindowPos(
                        hwnd,
                        None,
                        r.left + 100,
                        r.top + 50,
                        0,
                        0,
                        SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                }
            }),
        ),
        (300, Box::new(move |app| click(at(app, 228., 88.)))),
        (
            800,
            Box::new(move |app| {
                popups(app, hwnd, "the select, window moved", (150., 100.));
                // A click elsewhere closes it; a posted Escape doesn't.
                click(at(app, 60., 190.));
            }),
        ),
        (
            500,
            Box::new(move |app| {
                popups(app, hwnd, "after a click elsewhere", (60., 190.));
                let p = at(app, 250., 125.);
                post(hwnd, WM_MOUSEMOVE, 0, p);
                post(hwnd, WM_RBUTTONDOWN, MK_RBUTTON as usize, p);
                post(hwnd, WM_RBUTTONUP, 0, p);
            }),
        ),
        (
            800,
            Box::new(move |app| {
                popups(app, hwnd, "the context menu", (250., 125.));
                state(app, "after a right click");
            }),
        ),
        (
            500,
            Box::new(move |app| {
                super::save(app.window(), &dir.join("embed-input.png"));
                slint::quit_event_loop().unwrap();
            }),
        ),
    ];
    run_steps(app.as_weak(), steps.into_iter());
}

fn run_steps(weak: slint::Weak<Embed>, mut steps: std::vec::IntoIter<(u64, Step)>) {
    let Some((delay, step)) = steps.next() else { return };
    slint::Timer::single_shot(Duration::from_millis(delay), move || {
        let Some(app) = weak.upgrade() else { return };
        step(&app);
        run_steps(weak, steps);
    });
}

const VK_TAB_CODE: u16 = 0x09;

/// Prints the visible top-level windows of other processes (the WebView2's popups), and where
/// the page's point `near` is on the screen.
fn popups(app: &Embed, hwnd: HWND, label: &str, near: (f32, f32)) {
    let s = app.window().scale_factor();
    let mut p = POINT {
        x: ((app.get_web_x() + near.0) * s).round() as i32,
        y: ((app.get_web_y() + near.1) * s).round() as i32,
    };
    unsafe {
        let _ = ClientToScreen(hwnd, &mut p);
    }
    let under = unsafe { WindowFromPoint(p) };
    println!(
        "{label}: the page's point is at {},{} on the screen, over {}",
        p.x,
        p.y,
        class_of(under)
    );
    unsafe extern "system" fn each(w: HWND, _: LPARAM) -> windows::core::BOOL {
        let mut process = 0;
        unsafe { GetWindowThreadProcessId(w, Some(&mut process)) };
        let class = class_of(w);
        if unsafe { IsWindowVisible(w) }.as_bool()
            && process != std::process::id()
            && class.starts_with("Chrome_WidgetWin")
        {
            let mut r = RECT::default();
            unsafe {
                let _ = GetWindowRect(w, &mut r);
            }
            let ex = unsafe { GetWindowLongW(w, GWL_EXSTYLE) } as u32;
            let owner = unsafe { GetWindow(w, GW_OWNER) }.unwrap_or_default();
            println!(
                "  popup {class}: {}x{} at {},{}, ex {ex:#x}, owner {:?}",
                r.right - r.left,
                r.bottom - r.top,
                r.left,
                r.top,
                owner.0
            );
        }
        true.into()
    }
    unsafe {
        let _ = EnumWindows(Some(each), LPARAM(0));
    }
    children(hwnd);
}

fn post(hwnd: HWND, msg: u32, wp: usize, (x, y): (i32, i32)) {
    let lp = ((y as u16 as isize) << 16) | (x as u16 as isize);
    unsafe {
        let _ = PostMessageW(Some(hwnd), msg, WPARAM(wp), LPARAM(lp));
    }
}

/// A key down and up, as the keyboard would send them, with the scan code.
fn key(hwnd: HWND, vk: u16) {
    let scan = unsafe {
        windows::Win32::UI::Input::KeyboardAndMouse::MapVirtualKeyW(
            vk as u32,
            windows::Win32::UI::Input::KeyboardAndMouse::MAPVK_VK_TO_VSC,
        )
    } as isize;
    unsafe {
        let _ = PostMessageW(Some(hwnd), WM_KEYDOWN, WPARAM(vk as usize), LPARAM(1 | (scan << 16)));
        let up = 1 | (scan << 16) | (1 << 30) | (1 << 31);
        let _ = PostMessageW(Some(hwnd), WM_KEYUP, WPARAM(vk as usize), LPARAM(up));
    }
}

/// Prints the page's state, as its script reports it.
fn state(_app: &Embed, label: &'static str) {
    with_web(|web| {
        let done = ExecuteScriptCompletedHandler::create(Box::new(move |hr, json| {
            hr?;
            println!("{label}: {json}");
            Ok(())
        }));
        let script = HSTRING::from("state()");
        unsafe {
            let _ = web
                .controller
                .CoreWebView2()
                .and_then(|core| core.ExecuteScript(PCWSTR(script.as_ptr()), &done));
        }
    });
}

fn class_of(hwnd: HWND) -> String {
    if hwnd.is_invalid() {
        return "(none)".into();
    }
    let mut name = [0u16; 128];
    let n = unsafe { GetClassNameW(hwnd, &mut name) } as usize;
    let mut process = 0;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut process)) };
    let own = process == std::process::id();
    format!(
        "{} ({:?}, {})",
        String::from_utf16_lossy(&name[..n]),
        hwnd.0,
        if own { "this process" } else { "another process" }
    )
}

/// Prints the child windows the WebView2 made in the probe's window.
fn children(hwnd: HWND) {
    unsafe extern "system" fn each(child: HWND, _: LPARAM) -> windows::core::BOOL {
        let mut r = RECT::default();
        unsafe {
            let _ = GetWindowRect(child, &mut r);
        }
        let visible = unsafe { IsWindowVisible(child) }.as_bool();
        let style = unsafe { GetWindowLongW(child, GWL_STYLE) } as u32;
        let ex = unsafe { GetWindowLongW(child, GWL_EXSTYLE) } as u32;
        println!(
            "child {}: {}x{} at {},{}, visible {visible}, style {style:#x}, ex {ex:#x}",
            class_of(child),
            r.right - r.left,
            r.bottom - r.top,
            r.left,
            r.top
        );
        true.into()
    }
    println!("children of the window:");
    unsafe {
        let _ = EnumChildWindows(Some(hwnd), Some(each), LPARAM(0));
    }
}
