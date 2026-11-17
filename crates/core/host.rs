// SPDX-License-Identifier: MIT OR Apache-2.0

#![allow(unsafe_code)]

//! The Win32 host: the event loop, top-level windows, and the services `SlintContext` asks the
//! platform for.
//!
//! Frames follow spike S1 (`docs/design/spike-s1-hosting.md`): a worker thread waits on the
//! compositor clock while animations run and posts one coalesced message per tick.

use crate::api::{
    EventLoopError, LogicalSize, PhysicalPosition, PhysicalSize, PlatformError, Window,
    WindowPosition, WindowSize,
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

mod input;
#[doc(hidden)]
pub mod pointer;

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

/// Fires due timers and change handlers, and moves the animation tick to now unless frames
/// are ticking.
///
/// While frames tick, only [`WM_APP_FRAME`] moves the tick, which paces animations to the
/// compositor clock. Moving it on every message would dirty animated properties and request
/// another sync pass each time, and the loop would spin.
fn update_timers() {
    let frames_ticking =
        FRAMES.with(|f| f.get().is_some_and(|f| f.animating.load(Ordering::Acquire)));
    match crate::context::GLOBAL_CONTEXT.with(|ctx| ctx.get().cloned()) {
        Some(ctx) if frames_ticking => {
            let now = crate::animations::Instant::now(&ctx);
            ctx.platform_context().activate_timers(now);
            crate::properties::ChangeTracker::run_change_handlers();
        }
        _ => crate::platform::update_timers_and_animations(),
    }
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
            update_timers();
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
            update_timers();
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
    /// A size set before the HWND exists, applied once its scale factor is known.
    requested_size: Cell<Option<WindowSize>>,
    /// Whether someone set the size: `set_size`, or the preferred size of the layout.
    size_set: Cell<bool>,
    /// The client size in logical pixels that `set_size` or the user asked for.
    /// A DPI change keeps it, instead of what rounding the physical size gives.
    logical_size: Cell<Option<LogicalSize>>,
    /// Set while `WM_DPICHANGED` resizes the window, whose `WM_SIZE` keeps `logical_size`.
    in_dpi_change: Cell<bool>,
    constraints: Cell<crate::window::LayoutConstraints>,
    /// The title, kept for when the HWND is created.
    title: RefCell<HSTRING>,
    sync_pending: Cell<bool>,
    visible: Cell<bool>,
    mouse: input::Mouse,
    pointers: pointer::Pointers,
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
            requested_size: Cell::new(None),
            size_set: Cell::new(false),
            logical_size: Cell::new(None),
            in_dpi_change: Cell::new(false),
            constraints: Cell::new(Default::default()),
            title: RefCell::new(HSTRING::new()),
            sync_pending: Cell::new(false),
            visible: Cell::new(false),
            mouse: input::Mouse::new(),
            pointers: Default::default(),
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

    /// The number of visuals the window keeps for items.
    pub fn node_count(&self) -> usize {
        self.renderer.tree.borrow().node_count()
    }

    /// The number of sync passes the window ran.
    pub fn sync_passes(&self) -> u64 {
        self.renderer.tree.borrow().passes()
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
                &*self.title.borrow(),
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
        // The window was created for the system DPI; a requested size applies again at the
        // window's own DPI.
        match self.requested_size.take() {
            Some(size) => self.set_size(size),
            None => self.resized(client_size(hwnd)),
        }
        Ok(hwnd)
    }

    /// Creates the HWND without showing the window, so that tests can send it messages.
    #[doc(hidden)]
    pub fn create_hwnd_for_testing(&self) -> Result<HWND, PlatformError> {
        self.ensure_hwnd()
    }

    /// Handles a touch or pen message as if `GetPointerInfo` had returned `message`:
    /// tests can't make Windows report pointers without acting on the real screen.
    #[doc(hidden)]
    pub fn pointer_message_for_testing(&self, message: pointer::PointerMessage) {
        if let Some(hwnd) = self.hwnd.get() {
            pointer::dispatch(self, hwnd, message);
        }
    }

    /// A point in client pixels, in logical pixels.
    fn logical_point(&self, point: POINT) -> crate::lengths::LogicalPoint {
        let scale = self.window.scale_factor();
        crate::lengths::LogicalPoint::new(point.x as f32 / scale, point.y as f32 / scale)
    }

    /// The window's DPI, or the one its scale factor stands for while it has no HWND.
    fn dpi(&self) -> u32 {
        match self.hwnd.get() {
            Some(hwnd) => unsafe { GetDpiForWindow(hwnd) },
            None => (self.window.scale_factor() * 96.0).round() as u32,
        }
    }

    /// The window rectangle size that keeps the intended logical size at `dpi`, unless the
    /// window is maximized or minimized.
    fn dpi_scaled_frame(&self, hwnd: HWND, dpi: u32) -> Option<(i32, i32)> {
        let logical = self.logical_size.get()?;
        if unsafe { IsZoomed(hwnd).as_bool() || IsIconic(hwnd).as_bool() } {
            return None;
        }
        Some(self.frame_size(hwnd, to_physical(logical, dpi), dpi))
    }

    /// The window rectangle size for a client of `client` physical pixels at `dpi`.
    fn frame_size(&self, hwnd: HWND, client: PhysicalSize, dpi: u32) -> (i32, i32) {
        let mut rc =
            RECT { left: 0, top: 0, right: client.width as i32, bottom: client.height as i32 };
        unsafe {
            let style = WINDOW_STYLE(GetWindowLongW(hwnd, GWL_STYLE) as u32);
            let ex_style = WINDOW_EX_STYLE(GetWindowLongW(hwnd, GWL_EXSTYLE) as u32);
            let _ = AdjustWindowRectExForDpi(&mut rc, style, false, ex_style, dpi);
        }
        (rc.right - rc.left, rc.bottom - rc.top)
    }

    fn scale_factor_changed(&self, scale: f32) {
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
        inner.sync_visuals(|trees| {
            // Read inside the redraw tracker, so a scale factor change requests a sync pass.
            tree.set_scale_factor(inner.scale_factor());
            tree.sync(trees)
        });
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
        } else {
            self.mouse.reset();
            self.pointers.reset();
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

    fn set_size(&self, requested: WindowSize) {
        self.size_set.set(true);
        let size = match requested {
            WindowSize::Logical(logical) => {
                self.logical_size.set(Some(logical));
                to_physical(logical, self.dpi())
            }
            WindowSize::Physical(physical) => {
                self.logical_size.set(Some(physical.to_logical(self.window.scale_factor())));
                physical
            }
        };
        let Some(hwnd) = self.hwnd.get() else {
            if !self.detached {
                self.requested_size.set(Some(requested));
            }
            // Without an HWND there is no WM_SIZE to wait for.
            self.resized(size);
            return;
        };
        let (width, height) = self.frame_size(hwnd, size, self.dpi());
        unsafe {
            let _ = SetWindowPos(
                hwnd,
                None,
                0,
                0,
                width,
                height,
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
        let title = HSTRING::from(properties.title().as_str());
        if *self.title.borrow() != title {
            if let Some(hwnd) = self.hwnd.get() {
                unsafe {
                    let _ = SetWindowTextW(hwnd, &title);
                }
            }
            self.title.replace(title);
        }

        let constraints = properties.layout_constraints();
        self.constraints.set(constraints);
        let current = self.size.get().to_logical(self.window.scale_factor());
        // Without a size of its own, the window takes the layout's preferred size;
        // an axis the layout has no preference for keeps the default.
        let mut size = if self.size_set.get() {
            current
        } else {
            let preferred = constraints.preferred;
            LogicalSize::new(
                if preferred.width > 0. { preferred.width } else { current.width },
                if preferred.height > 0. { preferred.height } else { current.height },
            )
        };
        if let Some(min) = constraints.min {
            size = LogicalSize::new(size.width.max(min.width), size.height.max(min.height));
        }
        if let Some(max) = constraints.max {
            size = LogicalSize::new(size.width.min(max.width), size.height.min(max.height));
        }
        if !self.size_set.get() || size != current {
            self.set_size(WindowSize::Logical(size));
        }
    }

    fn internal(&self, _: crate::InternalToken) -> Option<&dyn WindowAdapterInternal> {
        Some(self)
    }
}

// `from_window` downcasts through it.
impl WindowAdapterInternal for Win32WindowAdapter {
    fn set_mouse_cursor(
        &self,
        cursor: crate::cursor::MouseCursorInner,
    ) -> crate::cursor::MouseCursorHandling {
        match cursor {
            crate::cursor::MouseCursorInner::BuiltIn(cursor) => self.mouse.set_cursor(cursor),
            // Images come with Phase 5.
            _ => self.mouse.set_cursor(crate::items::BuiltInMouseCursor::Default),
        }
        crate::cursor::MouseCursorHandling::Platform
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
    if let Some(result) = input::handle(&adapter, hwnd, msg, wp, lp) {
        return result;
    }
    if let Some(result) = pointer::handle(&adapter, hwnd, msg, wp) {
        return result;
    }
    match msg {
        WM_APP_SYNC => {
            adapter.sync();
            LRESULT(0)
        }
        WM_SIZE if wp.0 != SIZE_MINIMIZED as usize => {
            let size = PhysicalSize::new((lp.0 & 0xffff) as u32, ((lp.0 >> 16) & 0xffff) as u32);
            // A size that the intended logical size doesn't explain comes from the user.
            let intended = adapter.logical_size.get().map(|l| to_physical(l, adapter.dpi()));
            let from_user = !adapter.in_dpi_change.get() && intended != Some(size);
            if from_user {
                adapter.logical_size.set(Some(size.to_logical(adapter.window.scale_factor())));
            }
            trace_dpi(format_args!(
                "WM_SIZE {}x{} dpi={} in_dpi_change={} intended={intended:?} -> logical={:?}",
                size.width,
                size.height,
                adapter.dpi(),
                adapter.in_dpi_change.get(),
                adapter.logical_size.get()
            ));
            adapter.resized(size);
            LRESULT(0)
        }
        WM_GETDPISCALEDSIZE => {
            let dpi = (wp.0 & 0xffff) as u32;
            // SAFETY: WM_GETDPISCALEDSIZE's lParam points to the proposed window size.
            let proposed = unsafe { &mut *(lp.0 as *mut SIZE) };
            let Some(frame) = adapter.dpi_scaled_frame(hwnd, dpi) else {
                trace_dpi(format_args!(
                    "WM_GETDPISCALEDSIZE dpi={} -> {dpi}: default for {}x{}",
                    adapter.dpi(),
                    proposed.cx,
                    proposed.cy
                ));
                return LRESULT(0);
            };
            trace_dpi(format_args!(
                "WM_GETDPISCALEDSIZE dpi={} -> {dpi}: {}x{} becomes {}x{}",
                adapter.dpi(),
                proposed.cx,
                proposed.cy,
                frame.0,
                frame.1
            ));
            (proposed.cx, proposed.cy) = frame;
            LRESULT(1)
        }
        WM_DPICHANGED => {
            let dpi = (wp.0 & 0xffff) as u32;
            adapter.in_dpi_change.set(true);
            adapter.scale_factor_changed(dpi as f32 / 96.0);
            // SAFETY: WM_DPICHANGED's lParam points to the suggested window rectangle.
            if let Some(r) = unsafe { (lp.0 as *const RECT).as_ref() } {
                // The suggested rectangle has the size WM_GETDPISCALEDSIZE answered.
                trace_dpi(format_args!(
                    "WM_DPICHANGED dpi={dpi} suggested {}x{} at {},{}",
                    r.right - r.left,
                    r.bottom - r.top,
                    r.left,
                    r.top
                ));
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
            }
            // `SetWindowPos` sends no WM_SIZE when the physical size stays the same, and the
            // logical size must follow the new scale anyway.
            let client = client_size(hwnd);
            trace_dpi(format_args!(
                "WM_DPICHANGED done: client {}x{}",
                client.width, client.height
            ));
            adapter.resized(client);
            adapter.in_dpi_change.set(false);
            LRESULT(0)
        }
        WM_GETMINMAXINFO => {
            let constraints = adapter.constraints.get();
            let dpi = adapter.dpi();
            // SAFETY: WM_GETMINMAXINFO's lParam points to the MINMAXINFO to fill.
            let info = unsafe { &mut *(lp.0 as *mut MINMAXINFO) };
            if let Some(min) = constraints.min {
                let (w, h) = adapter.frame_size(hwnd, to_physical(min, dpi), dpi);
                info.ptMinTrackSize = POINT { x: w, y: h };
            }
            if let Some(max) = constraints.max {
                let (w, h) = adapter.frame_size(hwnd, to_physical(max, dpi), dpi);
                info.ptMaxTrackSize = POINT { x: w, y: h };
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

/// Prints how the window follows DPI changes when `SLINT_FORK_TRACE_DPI` is set.
fn trace_dpi(args: core::fmt::Arguments) {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *ENABLED.get_or_init(|| std::env::var_os("SLINT_FORK_TRACE_DPI").is_some()) {
        eprintln!("[dpi] {args}");
    }
}

/// `size` in physical pixels at `dpi`, rounded to the nearest pixel.
fn to_physical(size: LogicalSize, dpi: u32) -> PhysicalSize {
    let scale = dpi as f64 / 96.0;
    let px = |v: f32| (v as f64 * scale).round().clamp(0.0, (i32::MAX / 2) as f64) as u32;
    PhysicalSize::new(px(size.width), px(size.height))
}

fn client_size(hwnd: HWND) -> PhysicalSize {
    let mut rc = RECT::default();
    unsafe {
        let _ = GetClientRect(hwnd, &mut rc);
    }
    PhysicalSize::new(rc.right as u32, rc.bottom as u32)
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

    #[test]
    #[ignore = "opens a window on the desktop"]
    fn dpi_changes_keep_the_logical_size() {
        use crate::api::LogicalSize;
        let ctx = crate::SlintContext::new(Box::new(Platform::new().unwrap()));
        let adapter = ctx.create_window_adapter().unwrap();
        let tree = RectTree::new((100., 50.), 0);
        WindowInner::from_pub(adapter.window()).set_component(&vtable::VRc::into_dyn(tree.clone()));
        // Odd sizes, which a fractional scale factor doesn't map to whole pixels.
        adapter.window().set_size(LogicalSize::new(401., 301.));
        adapter.window().show().unwrap();
        let host = Win32WindowAdapter::from_window(adapter.window()).unwrap();
        let hwnd = host.hwnd.get().unwrap();
        let dpi = unsafe { GetDpiForWindow(hwnd) };
        let item_size = || {
            let window = adapter.window();
            let size = window.size().to_logical(window.scale_factor());
            (size.width, size.height)
        };

        // Created at the system DPI, the window ends up at its own, rounded to whole pixels.
        assert_eq!(client_size(hwnd), to_physical(LogicalSize::new(401., 301.), dpi));

        // Moving to a 150% monitor asks for the frame around 602x452 client pixels.
        let mut proposed = SIZE::default();
        let answered = unsafe {
            SendMessageW(
                hwnd,
                WM_GETDPISCALEDSIZE,
                Some(WPARAM(144)),
                Some(LPARAM(&mut proposed as *mut SIZE as isize)),
            )
        };
        assert_eq!(answered, LRESULT(1));
        assert_eq!(
            (proposed.cx, proposed.cy),
            host.frame_size(hwnd, PhysicalSize::new(602, 452), 144)
        );

        // A DPI change whose suggested rectangle keeps the physical size still moves the
        // logical size to the new scale, and keeps the intended one.
        let mut rect = RECT::default();
        unsafe { GetWindowRect(hwnd, &mut rect).unwrap() };
        let physical = client_size(hwnd);
        let send_dpi_changed = |dpi: u32| unsafe {
            SendMessageW(
                hwnd,
                WM_DPICHANGED,
                Some(WPARAM(((dpi << 16) | dpi) as usize)),
                Some(LPARAM(&rect as *const RECT as isize)),
            )
        };
        send_dpi_changed(144);
        assert_eq!(client_size(hwnd), physical);
        assert_eq!(item_size(), (physical.width as f32 / 1.5, physical.height as f32 / 1.5));
        assert_eq!(host.logical_size.get(), Some(LogicalSize::new(401., 301.)));

        send_dpi_changed(dpi);
        let scale = dpi as f32 / 96.;
        assert_eq!(item_size(), (physical.width as f32 / scale, physical.height as f32 / scale));
        adapter.window().hide().unwrap();
    }

    #[test]
    fn logical_sizes_round_to_the_nearest_pixel() {
        use crate::api::LogicalSize;
        // Truncating gave 601, 503 and 701.
        assert_eq!(to_physical(LogicalSize::new(401., 403.), 144), PhysicalSize::new(602, 605));
        assert_eq!(to_physical(LogicalSize::new(403., 401.), 120), PhysicalSize::new(504, 501));
        assert_eq!(to_physical(LogicalSize::new(401., 0.), 168), PhysicalSize::new(702, 0));
        // f32 scale factors miss whole pixels at DPIs that aren't multiples of 3.
        assert_eq!(to_physical(LogicalSize::new(480., 96.), 97), PhysicalSize::new(485, 97));
    }
}
