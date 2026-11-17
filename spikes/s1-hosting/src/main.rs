//! Spike S1: host a Windows.UI.Composition visual tree in a Win32 window.
//!
//! Usage: `s1-hosting [frames|resize|interactive] [--source clock|timer|dwmflush] [--frames N]`
//!
//! - `frames`: animates a sprite for N frames driven by the chosen frame source, prints
//!   frame-interval and commit-latency statistics, then measures idle CPU for 2 s and exits.
//! - `resize`: resizes the window programmatically and prints the latency from `WM_SIZE`
//!   to commit completion.
//! - `interactive`: keeps animating and logs `WM_SIZE` / `WM_DPICHANGED` for manual checks.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use windows_future::AsyncActionCompletedHandler;
use windows_numerics::{Vector2, Vector3};
use windows::System::DispatcherQueueController;
use windows::UI::Color;
use windows::UI::Composition::Desktop::DesktopWindowTarget;
use windows::UI::Composition::{Compositor, ContainerVisual, SpriteVisual};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::DirectComposition::DCompositionWaitForCompositorClock;
use windows::Win32::Graphics::Dwm::{DWM_TIMING_INFO, DwmFlush, DwmGetCompositionTimingInfo};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::*;
use windows::Win32::System::WinRT::Composition::ICompositorDesktopInterop;
use windows::Win32::System::WinRT::*;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{Interface, Result, w};

const WM_APP_FRAME: u32 = WM_APP + 1;
const TIMER_IDLE: usize = 1;
const TIMER_RESIZE: usize = 2;
const RESIZE_STEPS: usize = 30;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Mode {
    Frames,
    Resize,
    Interactive,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Source {
    Clock,
    Timer,
    DwmFlush,
}

type Samples = Arc<Mutex<Vec<f64>>>;

struct App {
    hwnd: HWND,
    mode: Mode,
    frames_target: usize,
    _controller: DispatcherQueueController,
    compositor: Compositor,
    _target: DesktopWindowTarget,
    root: ContainerVisual,
    sprite: SpriteVisual,
    start: Instant,
    last_frame: Option<Instant>,
    intervals: Vec<f64>,
    animating: Arc<AtomicBool>,
    pending: Arc<AtomicBool>,
    wake: HANDLE,
    commit_latency: Samples,
    resize_latency: Samples,
    resize_step: usize,
    idle_start: Option<(u64, Instant)>,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|a| a.borrow_mut().as_mut().map(f))
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let mode = match args.get(1).map(String::as_str) {
        Some("resize") => Mode::Resize,
        Some("interactive") => Mode::Interactive,
        _ => Mode::Frames,
    };
    let opt = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1));
    let source = match opt("--source").map(String::as_str) {
        Some("timer") => Source::Timer,
        Some("dwmflush") => Source::DwmFlush,
        _ => Source::Clock,
    };
    let frames_target = opt("--frames").and_then(|s| s.parse().ok()).unwrap_or(300);

    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)?;
        RoInitialize(RO_INIT_SINGLETHREADED)?;
    }
    print_refresh_rate();
    println!("mode={mode:?} source={source:?} frames={frames_target}");

    let hwnd = create_window()?;
    let t0 = Instant::now();
    let controller = unsafe {
        CreateDispatcherQueueController(DispatcherQueueOptions {
            dwSize: size_of::<DispatcherQueueOptions>() as u32,
            threadType: DQTYPE_THREAD_CURRENT,
            apartmentType: DQTAT_COM_STA,
        })?
    };
    let compositor = Compositor::new()?;
    let interop: ICompositorDesktopInterop = compositor.cast()?;
    let target = unsafe { interop.CreateDesktopWindowTarget(hwnd, false)? };
    let root = compositor.CreateContainerVisual()?;
    target.SetRoot(&root)?;
    let background = compositor.CreateSpriteVisual()?;
    background.SetRelativeSizeAdjustment(Vector2 { X: 1.0, Y: 1.0 })?;
    background.SetBrush(&compositor.CreateColorBrushWithColor(rgb(32, 32, 40))?)?;
    root.Children()?.InsertAtTop(&background)?;
    let sprite = compositor.CreateSpriteVisual()?;
    sprite.SetSize(Vector2 { X: 100.0, Y: 100.0 })?;
    sprite.SetOffset(Vector3 { X: 0.0, Y: 100.0, Z: 0.0 })?;
    sprite.SetBrush(&compositor.CreateColorBrushWithColor(rgb(0, 120, 215))?)?;
    root.Children()?.InsertAtTop(&sprite)?;
    println!("setup (dispatcher queue + compositor + target + 2 visuals): {:.2} ms", ms(t0));

    let animating = Arc::new(AtomicBool::new(mode != Mode::Resize));
    let pending = Arc::new(AtomicBool::new(false));
    let wake = unsafe { CreateEventW(None, false, false, None)? };
    let stop = unsafe { CreateEventW(None, true, false, None)? };

    APP.with(|a| {
        *a.borrow_mut() = Some(App {
            hwnd,
            mode,
            frames_target,
            _controller: controller,
            compositor,
            _target: target,
            root,
            sprite,
            start: Instant::now(),
            last_frame: None,
            intervals: Vec::new(),
            animating: animating.clone(),
            pending: pending.clone(),
            wake,
            commit_latency: Default::default(),
            resize_latency: Default::default(),
            resize_step: 0,
            idle_start: None,
        })
    });
    with_app(|app| app.update_root_size());

    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
    }
    if mode == Mode::Resize {
        unsafe { SetTimer(Some(hwnd), TIMER_RESIZE, 150, None) };
    }

    if source == Source::Timer {
        run_timer_loop(&animating)?;
    } else {
        spawn_frame_thread(source, hwnd, animating, pending, wake, stop);
        run_message_loop();
        unsafe { SetEvent(stop)? };
    }
    APP.with(|a| a.borrow_mut().take());
    Ok(())
}

fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color { A: 255, R: r, G: g, B: b }
}

fn ms(since: Instant) -> f64 {
    since.elapsed().as_secs_f64() * 1000.0
}

fn print_refresh_rate() {
    let mut info = DWM_TIMING_INFO { cbSize: size_of::<DWM_TIMING_INFO>() as u32, ..Default::default() };
    if unsafe { DwmGetCompositionTimingInfo(HWND::default(), &mut info) }.is_ok() {
        let r = info.rateRefresh;
        let hz = r.uiNumerator as f64 / r.uiDenominator.max(1) as f64;
        println!("display refresh: {hz:.2} Hz ({:.3} ms)", 1000.0 / hz);
    }
}

fn create_window() -> Result<HWND> {
    unsafe {
        let instance = GetModuleHandleW(None)?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            lpszClassName: w!("SlintForkS1"),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            ..Default::default()
        };
        RegisterClassW(&class);
        let scale = GetDpiForSystem() as i32;
        CreateWindowExW(
            WS_EX_NOREDIRECTIONBITMAP,
            w!("SlintForkS1"),
            w!("S1 hosting spike"),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            800 * scale / 96,
            600 * scale / 96,
            None,
            None,
            Some(instance.into()),
            None,
        )
    }
}

/// Wakes the UI thread once per frame while `animating` is set; sleeps on `wake` otherwise.
fn spawn_frame_thread(
    source: Source,
    hwnd: HWND,
    animating: Arc<AtomicBool>,
    pending: Arc<AtomicBool>,
    wake: HANDLE,
    stop: HANDLE,
) {
    let (hwnd, wake, stop) = (hwnd.0 as usize, wake.0 as usize, stop.0 as usize);
    std::thread::spawn(move || unsafe {
        let (hwnd, wake, stop) = (HWND(hwnd as _), HANDLE(wake as _), HANDLE(stop as _));
        loop {
            if !animating.load(Ordering::Acquire) {
                let r = WaitForMultipleObjects(&[wake, stop], false, INFINITE);
                if r.0 == WAIT_OBJECT_0.0 + 1 {
                    return;
                }
                continue;
            }
            match source {
                Source::Clock => {
                    if DCompositionWaitForCompositorClock(Some(&[stop]), INFINITE) == WAIT_OBJECT_0.0 {
                        return;
                    }
                }
                Source::DwmFlush => {
                    let _ = DwmFlush();
                    if WaitForSingleObject(stop, 0) == WAIT_OBJECT_0 {
                        return;
                    }
                }
                Source::Timer => unreachable!(),
            }
            if !pending.swap(true, Ordering::AcqRel) {
                let _ = PostMessageW(Some(hwnd), WM_APP_FRAME, WPARAM(0), LPARAM(0));
            }
        }
    });
}

fn run_message_loop() {
    let mut msg = MSG::default();
    unsafe {
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

/// Frame source on the UI thread: a high-resolution waitable timer re-armed every 1/60 s.
fn run_timer_loop(animating: &AtomicBool) -> Result<()> {
    unsafe {
        let timer = CreateWaitableTimerExW(
            None,
            None,
            CREATE_WAITABLE_TIMER_HIGH_RESOLUTION,
            TIMER_ALL_ACCESS.0,
        )?;
        let arm = || SetWaitableTimer(timer, &-166_667i64, 0, None, None, false);
        arm()?;
        let mut msg = MSG::default();
        loop {
            let r = MsgWaitForMultipleObjectsEx(Some(&[timer]), INFINITE, QS_ALLINPUT, MWMO_INPUTAVAILABLE);
            if r == WAIT_OBJECT_0 {
                with_app(App::on_frame);
                if animating.load(Ordering::Acquire) {
                    arm()?;
                }
            }
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == WM_QUIT {
                    return Ok(());
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
}

fn cpu_time_100ns() -> u64 {
    let (mut c, mut e, mut k, mut u) = Default::default();
    unsafe { GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u).unwrap() };
    let v = |f: FILETIME| ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64;
    v(k) + v(u)
}

impl App {
    fn dpi(&self) -> u32 {
        unsafe { GetDpiForWindow(self.hwnd) }
    }

    /// Root visual: client size in DIPs, scaled to physical pixels.
    fn update_root_size(&self) {
        let mut rc = RECT::default();
        unsafe { GetClientRect(self.hwnd, &mut rc).unwrap() };
        let scale = self.dpi() as f32 / 96.0;
        let w = (rc.right - rc.left) as f32 / scale;
        let h = (rc.bottom - rc.top) as f32 / scale;
        self.root.SetSize(Vector2 { X: w, Y: h }).unwrap();
        self.root.SetScale(Vector3 { X: scale, Y: scale, Z: 1.0 }).unwrap();
    }

    fn request_commit(&self, sink: &Samples) {
        let sink = sink.clone();
        let t0 = Instant::now();
        let op = self.compositor.RequestCommitAsync().unwrap();
        op.SetCompleted(&AsyncActionCompletedHandler::new(move |_, _| {
            sink.lock().unwrap().push(ms(t0));
            Ok(())
        }))
        .unwrap();
    }

    fn on_frame(&mut self) {
        self.pending.store(false, Ordering::Release);
        if !self.animating.load(Ordering::Acquire) {
            return;
        }
        let now = Instant::now();
        if let Some(last) = self.last_frame {
            self.intervals.push((now - last).as_secs_f64() * 1000.0);
        }
        self.last_frame = Some(now);
        let x = ((now - self.start).as_secs_f32() * 400.0) % 600.0;
        self.sprite.SetOffset(Vector3 { X: x, Y: 100.0, Z: 0.0 }).unwrap();
        if self.intervals.len() % 30 == 1 {
            self.request_commit(&self.commit_latency);
        }
        if self.mode == Mode::Frames && self.intervals.len() >= self.frames_target {
            self.animating.store(false, Ordering::Release);
            report("frame interval (ms)", &self.intervals);
            self.idle_start = Some((cpu_time_100ns(), Instant::now()));
            unsafe { SetTimer(Some(self.hwnd), TIMER_IDLE, 2000, None) };
        }
    }

    fn on_timer(&mut self, id: usize) {
        match id {
            TIMER_IDLE => {
                unsafe { KillTimer(Some(self.hwnd), TIMER_IDLE).unwrap() };
                report("commit latency (ms)", &self.commit_latency.lock().unwrap());
                if let Some((cpu0, t0)) = self.idle_start {
                    let cpu_ms = (cpu_time_100ns() - cpu0) as f64 / 10_000.0;
                    println!("idle: {cpu_ms:.2} ms CPU over {:.0} ms wall", ms(t0));
                }
                unsafe { PostQuitMessage(0) };
            }
            TIMER_RESIZE => {
                let i = self.resize_step;
                self.resize_step += 1;
                if i >= RESIZE_STEPS {
                    unsafe { KillTimer(Some(self.hwnd), TIMER_RESIZE).unwrap() };
                    report("WM_SIZE -> commit completed (ms)", &self.resize_latency.lock().unwrap());
                    unsafe { PostQuitMessage(0) };
                    return;
                }
                let s = self.dpi() as i32;
                let (w, h) = (600 + (i as i32 % 5) * 80, 400 + (i as i32 % 3) * 60);
                post_resize(self.hwnd, w * s / 96, h * s / 96);
            }
            _ => {}
        }
    }

    fn on_size(&self) {
        self.update_root_size();
        if self.mode == Mode::Resize {
            self.request_commit(&self.resize_latency);
        }
        if self.mode == Mode::Interactive {
            let mut rc = RECT::default();
            unsafe { GetClientRect(self.hwnd, &mut rc).unwrap() };
            println!("WM_SIZE client={}x{} dpi={}", rc.right, rc.bottom, self.dpi());
        }
    }
}

fn post_resize(hwnd: HWND, w: i32, h: i32) {
    // Deferred so the caller's borrow of APP is released before WM_SIZE arrives.
    let (hwnd, w, h) = (hwnd.0 as usize, w, h);
    RESIZE.with(|r| r.set(Some((hwnd, w, h))));
}

thread_local! {
    static RESIZE: std::cell::Cell<Option<(usize, i32, i32)>> = const { std::cell::Cell::new(None) };
}

fn print_stats_line(name: &str, v: &[f64]) {
    if v.is_empty() {
        println!("{name}: no samples");
        return;
    }
    let mut s = v.to_vec();
    s.sort_by(f64::total_cmp);
    let n = s.len() as f64;
    let mean = s.iter().sum::<f64>() / n;
    let sd = (s.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n).sqrt();
    let p = |q: f64| s[((q * (n - 1.0)).round() as usize).min(s.len() - 1)];
    println!(
        "{name}: n={} mean={mean:.3} sd={sd:.3} min={:.3} p50={:.3} p95={:.3} p99={:.3} max={:.3}",
        s.len(),
        s[0],
        p(0.5),
        p(0.95),
        p(0.99),
        s[s.len() - 1]
    );
}

fn report(name: &str, v: &[f64]) {
    print_stats_line(name, v);
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_APP_FRAME => {
            with_app(App::on_frame);
            LRESULT(0)
        }
        WM_TIMER => {
            with_app(|app| app.on_timer(wp.0));
            if let Some((h, w, hh)) = RESIZE.with(|r| r.take()) {
                unsafe {
                    let _ = SetWindowPos(HWND(h as _), None, 0, 0, w, hh, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
                }
            }
            LRESULT(0)
        }
        WM_SIZE => {
            with_app(|app| app.on_size());
            LRESULT(0)
        }
        WM_DPICHANGED => {
            let dpi = (wp.0 & 0xffff) as u32;
            println!("WM_DPICHANGED dpi={dpi}");
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
            with_app(|app| app.update_root_size());
            LRESULT(0)
        }
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
}

#[allow(dead_code)]
fn wake_frame_thread(app: &App) {
    unsafe { SetEvent(app.wake).unwrap() };
}
