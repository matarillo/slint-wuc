#![allow(unsafe_code)]

//! The Win32 host: the event loop, top-level windows, and the services `SlintContext` asks the
//! platform for.
//!
//! Frames follow spike S1 (`docs/design/spike-s1-hosting.md`): a worker thread waits on the
//! compositor clock while animations run and posts one coalesced message per tick.

use crate::api::{
    EventLoopError, PhysicalPosition, PhysicalSize, PlatformError, Window, WindowPosition,
    WindowSize,
};
use crate::item_tree::ItemTreeRef;
use crate::items::ItemRef;
use crate::platform::{EventLoopProxy, WindowEvent};
use crate::renderer::RendererSealed;
use crate::visual_tree::VisualTree;
use crate::window::{WindowAdapter, WindowAdapterInternal, WindowInner, WindowProperties};
use alloc::boxed::Box;
use alloc::rc::{Rc, Weak};
use core::cell::{Cell, RefCell};
use core::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use windows::System::DispatcherQueueController;
use windows::UI::Composition::Compositor;
use windows::UI::Composition::Desktop::DesktopWindowTarget;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::DirectComposition::DCompositionWaitForCompositorClock;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{CreateEventW, INFINITE, SetEvent, WaitForMultipleObjects};
use windows::Win32::System::WinRT::Composition::ICompositorDesktopInterop;
use windows::Win32::System::WinRT::{
    CreateDispatcherQueueController, DQTAT_COM_STA, DQTYPE_THREAD_CURRENT, DispatcherQueueOptions,
    RO_INIT_SINGLETHREADED, RoInitialize,
};
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{HSTRING, Interface, w};

const WM_APP_INVOKE: u32 = WM_APP + 1;
const WM_APP_QUIT: u32 = WM_APP + 2;
const WM_APP_SYNC: u32 = WM_APP + 3;
const WM_APP_FRAME: u32 = WM_APP + 4;
const TIMER_SLINT: usize = 1;

/// The platform of a UI thread: its event loop and the services `SlintContext` asks for.
///
/// This is the only production implementation of [`crate::platform::Platform`]; the trait stays
/// as an internal seam so unit tests can substitute their own clock and event loop.
pub struct Platform {
    start: Instant,
    _queue: Option<DispatcherQueueController>,
    compositor: Compositor,
    /// Message-only window that receives cross-thread requests, timers, and frame ticks.
    message_window: HWND,
    /// Creates detached windows (see [`Win32WindowAdapter`]) instead of HWNDs.
    headless: bool,
}

/// Frame requests shared between the UI thread and its frame thread.
struct Frames {
    /// Set while some window runs an animation; the frame thread ticks while it holds.
    animating: AtomicBool,
    /// Set while a tick is posted but not handled yet, so ticks never pile up.
    pending: AtomicBool,
    wake: HANDLE,
}

// SAFETY: `wake` is an event handle, usable from any thread and never closed.
unsafe impl Send for Frames {}
unsafe impl Sync for Frames {}

impl Frames {
    /// Ticks frames on the compositor clock until a tick finds no window animating.
    fn request(&self) {
        if !self.animating.swap(true, Ordering::AcqRel) {
            unsafe {
                let _ = SetEvent(self.wake);
            }
        }
    }
}

std::thread_local! {
    /// The frames of this thread's `Platform`.
    static FRAMES: core::cell::OnceCell<Arc<Frames>> = const { core::cell::OnceCell::new() };
}

fn request_frame() {
    FRAMES.with(|f| {
        if let Some(frames) = f.get() {
            frames.request();
        }
    });
}

impl Platform {
    /// Sets up the calling thread as a UI thread.
    pub fn new() -> Result<Self, PlatformError> {
        Self::with_mode(false)
    }

    /// A platform whose windows are detached, for tests.
    pub fn new_headless() -> Result<Self, PlatformError> {
        Self::with_mode(true)
    }

    fn with_mode(headless: bool) -> Result<Self, PlatformError> {
        let err = |e: windows::core::Error| PlatformError::Other(e.message());
        unsafe {
            let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            // Already initialized (e.g. by the host application) is fine.
            let _ = RoInitialize(RO_INIT_SINGLETHREADED);
        }
        let queue = unsafe {
            CreateDispatcherQueueController(DispatcherQueueOptions {
                dwSize: size_of::<DispatcherQueueOptions>() as u32,
                threadType: DQTYPE_THREAD_CURRENT,
                apartmentType: DQTAT_COM_STA,
            })
        }
        // The thread may already have a queue; the compositor only needs one to exist.
        .ok();
        let compositor = Compositor::new().map_err(err)?;
        let message_window = create_message_window().map_err(err)?;
        // One frame thread per UI thread, posting to the first platform's message window.
        FRAMES.with(|f| -> Result<_, PlatformError> {
            if f.get().is_none() {
                let wake = unsafe { CreateEventW(None, false, false, None) }.map_err(err)?;
                let frames = Arc::new(Frames {
                    animating: AtomicBool::new(false),
                    pending: AtomicBool::new(false),
                    wake,
                });
                let _ = f.set(frames.clone());
                spawn_frame_thread(frames, message_window);
            }
            Ok(())
        })?;
        Ok(Self { start: Instant::now(), _queue: queue, compositor, message_window, headless })
    }

    pub fn compositor(&self) -> &Compositor {
        &self.compositor
    }

    fn arm_timer(&self) {
        unsafe {
            match crate::platform::duration_until_next_timer_update() {
                Some(d) => {
                    let ms = d.as_millis().clamp(1, u32::MAX as u128) as u32;
                    SetTimer(Some(self.message_window), TIMER_SLINT, ms, None);
                }
                None => {
                    let _ = KillTimer(Some(self.message_window), TIMER_SLINT);
                }
            }
        }
    }
}

impl crate::platform::Platform for Platform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(Win32WindowAdapter::with_mode(&self.compositor, self.headless)?)
    }

    fn run_event_loop(&self) -> Result<(), PlatformError> {
        let mut msg = MSG::default();
        loop {
            crate::platform::update_timers_and_animations();
            self.arm_timer();
            let r = unsafe { GetMessageW(&mut msg, None, 0, 0) };
            if r.0 <= 0 {
                return Ok(());
            }
            unsafe {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }

    fn new_event_loop_proxy(&self) -> Option<Box<dyn EventLoopProxy>> {
        Some(Box::new(Proxy(self.message_window.0 as isize)))
    }

    fn duration_since_start(&self) -> Duration {
        self.start.elapsed()
    }

    fn click_interval(&self) -> Duration {
        let ms = unsafe { windows::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime() };
        Duration::from_millis(ms as u64)
    }

    fn cursor_flash_cycle(&self) -> Duration {
        // GetCaretBlinkTime is the time of one half of the cycle; INFINITE means no blinking.
        match unsafe { GetCaretBlinkTime() } {
            u32::MAX | 0 => Duration::ZERO,
            ms => Duration::from_millis(ms as u64 * 2),
        }
    }

    // TODO(phase 3): clipboard.

    fn debug_log(&self, arguments: core::fmt::Arguments) {
        let text = HSTRING::from(alloc::format!("{arguments}\n"));
        unsafe { windows::Win32::System::Diagnostics::Debug::OutputDebugStringW(&text) };
        crate::debug_log::default_log_message(arguments);
    }

    fn open_url(&self, url: &str) -> Result<(), PlatformError> {
        let r = unsafe {
            windows::Win32::UI::Shell::ShellExecuteW(
                None,
                w!("open"),
                &HSTRING::from(url),
                None,
                None,
                SW_SHOWNORMAL,
            )
        };
        // ShellExecuteW reports success with a value above 32.
        if r.0 as isize > 32 {
            Ok(())
        } else {
            Err(PlatformError::Other(alloc::format!("cannot open {url}")))
        }
    }
}

impl Drop for Platform {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.message_window);
        }
    }
}

struct Proxy(isize);

impl Proxy {
    fn post(&self, msg: u32, lparam: isize) -> windows::core::Result<()> {
        unsafe { PostMessageW(Some(HWND(self.0 as _)), msg, WPARAM(0), LPARAM(lparam)) }
    }
}

impl EventLoopProxy for Proxy {
    fn quit_event_loop(&self) -> Result<(), EventLoopError> {
        self.post(WM_APP_QUIT, 0).map_err(|_| EventLoopError::EventLoopTerminated)
    }

    fn invoke_from_event_loop(
        &self,
        event: Box<dyn FnOnce() + Send>,
    ) -> Result<(), EventLoopError> {
        let raw = Box::into_raw(Box::new(event));
        self.post(WM_APP_INVOKE, raw as isize).map_err(|_| {
            // SAFETY: the message wasn't posted, so this is the only owner.
            drop(unsafe { Box::from_raw(raw) });
            EventLoopError::EventLoopTerminated
        })
    }
}

fn create_message_window() -> windows::core::Result<HWND> {
    unsafe {
        let instance = GetModuleHandleW(None)?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(message_wndproc),
            hInstance: instance.into(),
            lpszClassName: w!("SlintMessageWindow"),
            ..Default::default()
        };
        RegisterClassW(&class);
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("SlintMessageWindow"),
            None,
            WINDOW_STYLE::default(),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            Some(instance.into()),
            None,
        )
    }
}

extern "system" fn message_wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_APP_INVOKE => {
            // SAFETY: posted by `Proxy::invoke_from_event_loop`, which gave up ownership.
            let event = unsafe { Box::from_raw(lp.0 as *mut Box<dyn FnOnce() + Send>) };
            event();
            LRESULT(0)
        }
        WM_APP_QUIT => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        WM_APP_FRAME => {
            FRAMES.with(|f| {
                if let Some(frames) = f.get() {
                    frames.pending.store(false, Ordering::Release);
                    // Windows still animating request the next tick from their sync pass.
                    frames.animating.store(false, Ordering::Release);
                }
            });
            crate::platform::update_timers_and_animations();
            LRESULT(0)
        }
        WM_TIMER if wp.0 == TIMER_SLINT => {
            crate::platform::update_timers_and_animations();
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
}

fn spawn_frame_thread(frames: Arc<Frames>, message_window: HWND) {
    let hwnd = message_window.0 as isize;
    std::thread::spawn(move || {
        let hwnd = HWND(hwnd as _);
        // Ends when the message window is gone and the post fails.
        loop {
            if !frames.animating.load(Ordering::Acquire) {
                unsafe { WaitForMultipleObjects(&[frames.wake], false, INFINITE) };
                continue;
            }
            unsafe { DCompositionWaitForCompositorClock(None, INFINITE) };
            if !frames.pending.swap(true, Ordering::AcqRel)
                && unsafe { PostMessageW(Some(hwnd), WM_APP_FRAME, WPARAM(0), LPARAM(0)) }.is_err()
            {
                return;
            }
        }
    });
}

/// A top-level window: an HWND whose composition target shows the window's [`VisualTree`].
///
/// A *detached* window has no HWND: its visual tree hangs nowhere, its scale factor stays at
/// what tests dispatch, and sync passes run when the test calls [`Self::sync_now`] (S3).
pub struct Win32WindowAdapter {
    window: Window,
    self_weak: Weak<Self>,
    detached: bool,
    hwnd: Cell<Option<HWND>>,
    target: RefCell<Option<DesktopWindowTarget>>,
    renderer: Renderer,
    size: Cell<PhysicalSize>,
    sync_pending: Cell<bool>,
    visible: Cell<bool>,
}

impl Win32WindowAdapter {
    pub fn new(compositor: &Compositor) -> Result<Rc<Self>, PlatformError> {
        Self::with_mode(compositor, false)
    }

    /// A window without an HWND, for tests.
    pub fn new_detached(compositor: &Compositor) -> Result<Rc<Self>, PlatformError> {
        Self::with_mode(compositor, true)
    }

    fn with_mode(compositor: &Compositor, detached: bool) -> Result<Rc<Self>, PlatformError> {
        let root =
            compositor.CreateContainerVisual().map_err(|e| PlatformError::Other(e.message()))?;
        let tree = VisualTree::new(compositor.clone(), root);
        Ok(Rc::new_cyclic(|weak: &Weak<Self>| Self {
            window: Window::new(weak.clone() as Weak<dyn WindowAdapter>),
            self_weak: weak.clone(),
            detached,
            hwnd: Cell::new(None),
            target: RefCell::new(None),
            renderer: Renderer::new(tree),
            size: Cell::new(PhysicalSize::new(800, 600)),
            sync_pending: Cell::new(false),
            visible: Cell::new(false),
        }))
    }

    /// The adapter behind `window`, if this host created it.
    pub fn from_window(window: &Window) -> Option<Rc<Self>> {
        let adapter = WindowInner::from_pub(window).window_adapter();
        let internal = adapter.internal(crate::InternalToken)?;
        let this = (internal as &dyn core::any::Any).downcast_ref::<Self>()?;
        this.self_weak.upgrade()
    }

    /// Runs a sync pass now if one is pending. Detached windows only sync through this.
    pub fn sync_now(&self) {
        if self.sync_pending.get() {
            self.sync();
        }
    }

    /// The visuals of the window's items, as text; see [`VisualTree::dump`].
    pub fn dump_visuals(&self) -> alloc::string::String {
        self.renderer.tree.borrow().dump()
    }

    /// The visual the window shows: the parent of the item trees' visuals.
    pub fn root_visual(&self) -> windows::UI::Composition::ContainerVisual {
        self.renderer.tree.borrow().root().clone()
    }

    fn inner(&self) -> &WindowInner {
        WindowInner::from_pub(&self.window)
    }

    fn ensure_hwnd(&self) -> Result<HWND, PlatformError> {
        if let Some(hwnd) = self.hwnd.get() {
            return Ok(hwnd);
        }
        let err = |e: windows::core::Error| PlatformError::Other(e.message());
        let hwnd = unsafe {
            let instance = GetModuleHandleW(None).map_err(err)?;
            let class = WNDCLASSW {
                lpfnWndProc: Some(window_wndproc),
                hInstance: instance.into(),
                lpszClassName: w!("SlintWindow"),
                hCursor: LoadCursorW(None, IDC_ARROW).map_err(err)?,
                ..Default::default()
            };
            RegisterClassW(&class);
            let size = self.size.get();
            let mut rc =
                RECT { left: 0, top: 0, right: size.width as i32, bottom: size.height as i32 };
            let _ = AdjustWindowRectExForDpi(
                &mut rc,
                WS_OVERLAPPEDWINDOW,
                false,
                WS_EX_NOREDIRECTIONBITMAP,
                GetDpiForSystem(),
            );
            CreateWindowExW(
                WS_EX_NOREDIRECTIONBITMAP,
                w!("SlintWindow"),
                // `update_window_properties` sets the title.
                w!(""),
                WS_OVERLAPPEDWINDOW,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                rc.right - rc.left,
                rc.bottom - rc.top,
                None,
                None,
                Some(instance.into()),
                None,
            )
            .map_err(err)?
        };
        // The window procedure finds the adapter through a weak reference in GWLP_USERDATA;
        // WM_NCDESTROY releases it.
        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, Weak::into_raw(self.self_weak.clone()) as isize)
        };
        self.hwnd.set(Some(hwnd));
        let renderer = &self.renderer;
        let compositor = renderer.tree.borrow().compositor().clone();
        let interop: ICompositorDesktopInterop = compositor.cast().map_err(err)?;
        let target = unsafe { interop.CreateDesktopWindowTarget(hwnd, false) }.map_err(err)?;
        target.SetRoot(renderer.tree.borrow().root()).map_err(err)?;
        *self.target.borrow_mut() = Some(target);
        let scale = unsafe { GetDpiForWindow(hwnd) } as f32 / 96.0;
        self.scale_factor_changed(scale);
        let mut rc = RECT::default();
        unsafe {
            let _ = GetClientRect(hwnd, &mut rc);
        }
        self.resized(PhysicalSize::new(rc.right as u32, rc.bottom as u32));
        Ok(hwnd)
    }

    fn scale_factor_changed(&self, scale: f32) {
        self.renderer.tree.borrow().set_scale_factor(scale);
        self.window.dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: scale });
    }

    fn resized(&self, size: PhysicalSize) {
        self.size.set(size);
        let logical = size.to_logical(self.window.scale_factor());
        self.window.dispatch_event(WindowEvent::Resized { size: logical });
    }

    /// The sync pass: brings the visuals up to date with the item tree.
    fn sync(&self) {
        self.sync_pending.set(false);
        let Ok(mut tree) = self.renderer.tree.try_borrow_mut() else {
            self.request_redraw();
            return;
        };
        let inner = self.inner();
        inner.sync_visuals(|trees| tree.sync(trees));
        drop(tree);
        if inner.has_active_animations() {
            request_frame();
        }
    }
}

impl WindowAdapter for Win32WindowAdapter {
    fn window(&self) -> &Window {
        &self.window
    }

    fn set_visible(&self, visible: bool) -> Result<(), PlatformError> {
        if visible == self.visible.get() {
            return Ok(());
        }
        if !self.detached {
            let hwnd = self.ensure_hwnd()?;
            unsafe {
                let _ = ShowWindow(hwnd, if visible { SW_SHOW } else { SW_HIDE });
            }
        }
        self.visible.set(visible);
        if visible {
            self.sync();
        }
        Ok(())
    }

    fn position(&self) -> Option<PhysicalPosition> {
        let mut rc = RECT::default();
        unsafe { GetWindowRect(self.hwnd.get()?, &mut rc) }.ok()?;
        Some(PhysicalPosition::new(rc.left, rc.top))
    }

    fn set_position(&self, position: WindowPosition) {
        let Some(hwnd) = self.hwnd.get() else { return };
        let p = position.to_physical(self.window.scale_factor());
        unsafe {
            let _ = SetWindowPos(
                hwnd,
                None,
                p.x,
                p.y,
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }

    fn set_size(&self, size: WindowSize) {
        let size = size.to_physical(self.window.scale_factor());
        let Some(hwnd) = self.hwnd.get() else {
            // Without an HWND there is no WM_SIZE to wait for.
            self.resized(size);
            return;
        };
        unsafe {
            let mut rc =
                RECT { left: 0, top: 0, right: size.width as i32, bottom: size.height as i32 };
            let style = WINDOW_STYLE(GetWindowLongW(hwnd, GWL_STYLE) as u32);
            let _ = AdjustWindowRectExForDpi(
                &mut rc,
                style,
                false,
                WS_EX_NOREDIRECTIONBITMAP,
                GetDpiForWindow(hwnd),
            );
            let _ = SetWindowPos(
                hwnd,
                None,
                0,
                0,
                rc.right - rc.left,
                rc.bottom - rc.top,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }

    fn size(&self) -> PhysicalSize {
        self.size.get()
    }

    fn request_redraw(&self) {
        // Showing the window syncs; until then (and for detached windows) only remember it.
        if !self.sync_pending.replace(true)
            && let Some(hwnd) = self.hwnd.get()
        {
            unsafe {
                let _ = PostMessageW(Some(hwnd), WM_APP_SYNC, WPARAM(0), LPARAM(0));
            }
        }
    }

    fn renderer(&self) -> &dyn crate::renderer::Renderer {
        &self.renderer
    }

    fn update_window_properties(&self, properties: WindowProperties<'_>) {
        if let Some(hwnd) = self.hwnd.get() {
            unsafe {
                let _ = SetWindowTextW(hwnd, &HSTRING::from(properties.title().as_str()));
            }
        }
    }

    fn internal(&self, _: crate::InternalToken) -> Option<&dyn WindowAdapterInternal> {
        Some(self)
    }
}

impl WindowAdapterInternal for Win32WindowAdapter {
    fn unregister_item_tree(
        &self,
        component: ItemTreeRef,
        items: &mut dyn Iterator<Item = Pin<ItemRef<'_>>>,
    ) {
        let _ = self.renderer.free_graphics_resources(component, items);
    }
}

impl Drop for Win32WindowAdapter {
    fn drop(&mut self) {
        if let Some(hwnd) = self.hwnd.get() {
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
        }
    }
}

extern "system" fn window_wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *const Win32WindowAdapter;
    if ptr.is_null() {
        return unsafe { DefWindowProcW(hwnd, msg, wp, lp) };
    }
    if msg == WM_NCDESTROY {
        // SAFETY: the pointer came from `Weak::into_raw` in `ensure_hwnd`.
        drop(unsafe { Weak::from_raw(ptr) });
        unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) };
        return unsafe { DefWindowProcW(hwnd, msg, wp, lp) };
    }
    // SAFETY: as above; ManuallyDrop keeps the reference owned by the window.
    let weak = core::mem::ManuallyDrop::new(unsafe { Weak::from_raw(ptr) });
    let Some(adapter) = weak.upgrade() else {
        return unsafe { DefWindowProcW(hwnd, msg, wp, lp) };
    };
    match msg {
        WM_APP_SYNC => {
            adapter.sync();
            LRESULT(0)
        }
        WM_SIZE if wp.0 != SIZE_MINIMIZED as usize => {
            let (w, h) = ((lp.0 & 0xffff) as u32, ((lp.0 >> 16) & 0xffff) as u32);
            adapter.resized(PhysicalSize::new(w, h));
            LRESULT(0)
        }
        WM_DPICHANGED => {
            let scale = (wp.0 & 0xffff) as f32 / 96.0;
            adapter.scale_factor_changed(scale);
            // SAFETY: WM_DPICHANGED's lParam points to the suggested window rectangle.
            let r = unsafe { *(lp.0 as *const RECT) };
            unsafe {
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    r.left,
                    r.top,
                    r.right - r.left,
                    r.bottom - r.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            adapter.window.dispatch_event(WindowEvent::CloseRequested);
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
}

/// The one implementation of the renderer seam: the window's visual tree, plus the text
/// measurement and font registration that core and generated code reach through
/// `WindowAdapter::renderer()`.
pub struct Renderer {
    pub(crate) tree: RefCell<VisualTree>,
    window_adapter: RefCell<Option<Weak<dyn WindowAdapter>>>,
}

impl Renderer {
    fn new(tree: VisualTree) -> Self {
        Self { tree: RefCell::new(tree), window_adapter: RefCell::new(None) }
    }
}

impl RendererSealed for Renderer {
    fn set_window_adapter(&self, window_adapter: &Rc<dyn WindowAdapter>) {
        *self.window_adapter.borrow_mut() = Some(Rc::downgrade(window_adapter));
    }

    fn window_adapter(&self) -> Option<Rc<dyn WindowAdapter>> {
        self.window_adapter.borrow().as_ref()?.upgrade()
    }

    fn supports_transformations(&self) -> bool {
        true
    }

    fn free_graphics_resources(
        &self,
        _component: ItemTreeRef,
        items: &mut dyn Iterator<Item = Pin<ItemRef<'_>>>,
    ) -> Result<(), PlatformError> {
        // During a sync pass the tree is borrowed; the pass's sweep drops the nodes of
        // destroyed items instead.
        if let Ok(mut tree) = self.tree.try_borrow_mut() {
            tree.release_items(items);
        }
        Ok(())
    }

    fn resize(&self, _size: PhysicalSize) -> Result<(), PlatformError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graphics::{Brush, Color};
    use crate::testing::RectTree;

    #[test]
    fn a_detached_window_syncs_its_component() {
        let ctx = crate::SlintContext::new(Box::new(Platform::new_headless().unwrap()));
        let adapter = ctx.create_window_adapter().unwrap();
        let tree = RectTree::new((100., 50.), 1);
        tree.rect(0).background.set(Brush::SolidColor(Color::from_rgb_u8(255, 0, 0)));
        let dyn_tree = vtable::VRc::into_dyn(tree.clone());
        WindowInner::from_pub(adapter.window()).set_component(&dyn_tree);
        adapter.window().show().unwrap();

        let host = Win32WindowAdapter::from_window(adapter.window()).unwrap();
        assert_eq!(host.dump_visuals(), "Sprite 0,0 100x50 #ff0000ff\n  Container 0,0 10x10\n");

        tree.rect(1).background.set(Brush::SolidColor(Color::from_rgb_u8(0, 0, 255)));
        host.sync_now();
        assert_eq!(
            host.dump_visuals(),
            "Sprite 0,0 100x50 #ff0000ff\n  Sprite 0,0 10x10 #0000ffff\n"
        );

        // Nothing changed, nothing pending.
        host.sync_now();
        assert!(!host.sync_pending.get());
    }

    #[test]
    #[ignore = "opens a window on the desktop"]
    fn a_window_follows_changes_while_the_event_loop_runs() {
        // As the facade will: this also installs the process-wide event loop proxy.
        crate::platform::set_platform(Box::new(Platform::new().unwrap())).unwrap();
        let ctx = crate::SlintContext::current().unwrap();
        let adapter = ctx.create_window_adapter().unwrap();
        let tree = RectTree::new((100., 50.), 1);
        tree.rect(0).background.set(Brush::SolidColor(Color::from_rgb_u8(255, 0, 0)));
        let dyn_tree = vtable::VRc::into_dyn(tree.clone());
        WindowInner::from_pub(adapter.window()).set_component(&dyn_tree);
        adapter.window().show().unwrap();
        let host = Win32WindowAdapter::from_window(adapter.window()).unwrap();
        assert!(host.hwnd.get().is_some());

        let tree_ = tree.clone();
        ctx.single_shot(Duration::from_millis(300), move || {
            tree_.rect(1).background.set(Brush::SolidColor(Color::from_rgb_u8(0, 0, 255)));
        });
        ctx.single_shot(Duration::from_millis(600), || {
            crate::api::quit_event_loop().unwrap();
        });
        ctx.run_event_loop().unwrap();

        assert_eq!(
            host.dump_visuals(),
            "Sprite 0,0 100x50 #ff0000ff
  Sprite 0,0 10x10 #0000ffff
"
        );
        let dpi = unsafe { GetDpiForWindow(host.hwnd.get().unwrap()) } as f32;
        assert_eq!(host.root_visual().Scale().unwrap().X, dpi / 96.);
    }
}
