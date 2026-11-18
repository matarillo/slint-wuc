// SPDX-License-Identifier: MIT OR Apache-2.0

//! Measures startup time, visual count, memory and sync pass times, and checks that repeated
//! creation and removal of rows, popups and windows gives back what it took.
//!
//! `probe-perf` measures `ui/perf.slint`; `probe-perf --gallery` measures the startup of
//! upstream's gallery. Run it with `--release`; `--long` runs the loops ten times as often.

slint::include_modules!();

mod gallery {
    include!(concat!(env!("OUT_DIR"), "/gallery.rs"));
}

use slint::ComponentHandle;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::FILETIME;
use windows::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX};
use windows::Win32::System::SystemInformation::GetSystemTimePreciseAsFileTime;
use windows::Win32::System::Threading::{
    GR_GDIOBJECTS, GR_USEROBJECTS, GetCurrentProcess, GetGuiResources, GetProcessHandleCount,
    GetProcessTimes,
};

type Host = Rc<i_slint_core::host::Win32WindowAdapter>;

/// Counts the bytes that Rust code holds on the heap, to tell them from what COM objects and
/// the drivers hold.
struct Counting;

static HEAP: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

unsafe impl std::alloc::GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        HEAP.fetch_add(layout.size() as isize, std::sync::atomic::Ordering::Relaxed);
        unsafe { std::alloc::System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: std::alloc::Layout) {
        HEAP.fetch_sub(layout.size() as isize, std::sync::atomic::Ordering::Relaxed);
        unsafe { std::alloc::System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: std::alloc::Layout, new_size: usize) -> *mut u8 {
        HEAP.fetch_add(new_size as isize - layout.size() as isize, std::sync::atomic::Ordering::Relaxed);
        unsafe { std::alloc::System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn host(window: &slint::Window) -> Host {
    i_slint_core::host::Win32WindowAdapter::from_window(window)
        .expect("the window is created by this runtime's host")
}

/// The time since the process was created.
fn since_process_start() -> Duration {
    let to_100ns = |t: FILETIME| (u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime);
    unsafe {
        let (mut creation, mut exit, mut kernel, mut user) = Default::default();
        GetProcessTimes(GetCurrentProcess(), &mut creation, &mut exit, &mut kernel, &mut user)
            .unwrap();
        let now = GetSystemTimePreciseAsFileTime();
        Duration::from_nanos((to_100ns(now) - to_100ns(creation)) * 100)
    }
}

#[derive(Clone, Copy, Debug)]
struct Usage {
    private_mb: f64,
    heap_mb: f64,
    handles: u32,
    gdi: u32,
    user: u32,
}

fn usage() -> Usage {
    unsafe {
        let process = GetCurrentProcess();
        let mut counters = PROCESS_MEMORY_COUNTERS_EX::default();
        K32GetProcessMemoryInfo(
            process,
            &mut counters as *mut _ as *mut _,
            std::mem::size_of_val(&counters) as u32,
        )
        .unwrap();
        let mut handles = 0;
        GetProcessHandleCount(process, &mut handles).unwrap();
        Usage {
            private_mb: counters.PrivateUsage as f64 / (1024. * 1024.),
            heap_mb: HEAP.load(std::sync::atomic::Ordering::Relaxed) as f64 / (1024. * 1024.),
            handles,
            gdi: GetGuiResources(process, GR_GDIOBJECTS),
            user: GetGuiResources(process, GR_USEROBJECTS),
        }
    }
}

fn print_usage(label: &str, u: Usage) {
    println!(
        "{label:<28} private {:>7.1} MB  Rust heap {:>7.1} MB  handles {:>4}  GDI {:>3}  USER {:>3}",
        u.private_mb, u.heap_mb, u.handles, u.gdi, u.user
    );
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.
}

fn median(mut v: Vec<Duration>) -> Duration {
    v.sort();
    v[v.len() / 2]
}

/// Runs `steps` one per timer tick, so that the event loop dispatches the messages in between,
/// then quits.
fn run_steps(window: &slint::Window, steps: Vec<Box<dyn FnOnce()>>) {
    let steps = RefCell::new(VecDeque::from(steps));
    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, Duration::from_millis(20), move || {
        match steps.borrow_mut().pop_front() {
            Some(step) => step(),
            None => slint::quit_event_loop().unwrap(),
        }
    });
    window.show().unwrap();
    slint::run_event_loop().unwrap();
}

/// Prints the time to the first sync pass, polling from a timer.
fn report_startup(window: &slint::Window, name: &'static str) -> slint::Timer {
    let host = host(window);
    let timer = slint::Timer::default();
    let done = std::cell::Cell::new(false);
    timer.start(slint::TimerMode::Repeated, Duration::from_millis(1), move || {
        if !done.get() && host.sync_passes() > 0 {
            done.set(true);
            println!(
                "{name}: first sync pass done {:.0} ms after process start, {} visuals",
                ms(since_process_start()),
                host.node_count()
            );
            print_usage(&format!("{name} after startup"), usage());
        }
    });
    timer
}

fn gallery() {
    let app = gallery::App::new().unwrap();
    let _startup = report_startup(app.window(), "gallery");
    let quit = slint::Timer::default();
    quit.start(slint::TimerMode::SingleShot, Duration::from_millis(1500), || {
        slint::quit_event_loop().unwrap()
    });
    app.run().unwrap();
}

fn main() {
    if std::env::args().any(|a| a == "--gallery") {
        gallery();
        return;
    }

    // `--long` repeats each loop ten times as often, to tell a leak from memory kept for reuse.
    let rounds = if std::env::args().any(|a| a == "--long") { 10 } else { 1 };
    let perf = Perf::new().unwrap();
    perf.set_with_text(!std::env::args().any(|a| a == "--no-text"));
    perf.set_same_text(std::env::args().any(|a| a == "--same-text"));
    let vary_text = std::env::args().any(|a| a == "--vary-text");
    let _startup = report_startup(perf.window(), "perf");
    let host = host(perf.window());
    let mut steps: Vec<Box<dyn FnOnce()>> = Vec::new();
    let weak = perf.as_weak();

    // Visual count, memory and sync pass times per row count.
    println!("\nrows  visuals  build+sync ms  color sync ms  text sync ms  remove ms  private MB");
    for rows in [0, 250, 1000, 2500, 5000] {
        let (weak, host) = (weak.clone(), host.clone());
        steps.push(Box::new(move || {
            let perf = weak.upgrade().unwrap();
            perf.set_highlighted(-1);
            host.sync_now();
            let start = Instant::now();
            perf.set_rows(rows);
            host.sync_now();
            let build = start.elapsed();
            let changes = (0..20)
                .map(|i| {
                    perf.set_highlighted(i);
                    let start = Instant::now();
                    host.sync_now();
                    start.elapsed()
                })
                .collect();
            let texts = (0..5)
                .map(|i| {
                    perf.set_suffix(slint::format!(" {i}"));
                    let start = Instant::now();
                    host.sync_now();
                    start.elapsed()
                })
                .collect();
            let visuals = host.node_count();
            let private_mb = usage().private_mb;
            let start = Instant::now();
            perf.set_rows(0);
            host.sync_now();
            let remove = start.elapsed();
            println!(
                "{rows:>5}  {visuals:>7}  {:>13.2}  {:>13.3}  {:>12.3}  {:>9.2}  {private_mb:>10.1}",
                ms(build),
                ms(median(changes)),
                ms(median(texts)),
                ms(remove),
            );
        }));
    }

    // Rows created and removed: the visuals go, and memory levels off.
    let baseline = Rc::new(RefCell::new(None));
    {
        let (weak, host, baseline) = (weak.clone(), host.clone(), baseline.clone());
        steps.push(Box::new(move || {
            weak.upgrade().unwrap().set_rows(0);
            host.sync_now();
            *baseline.borrow_mut() = Some(host.node_count());
            println!();
            print_usage("rows: before", usage());
        }));
    }
    for i in 1..=30 * rounds {
        let (weak, host, baseline) = (weak.clone(), host.clone(), baseline.clone());
        steps.push(Box::new(move || {
            let perf = weak.upgrade().unwrap();
            if vary_text {
                perf.set_suffix(slint::format!(" #{i}"));
            }
            perf.set_rows(1000);
            host.sync_now();
            perf.set_rows(0);
            host.sync_now();
            if i % (10 * rounds) == 0 {
                let leftover = host.node_count() as isize - baseline.borrow().unwrap() as isize;
                print_usage(&format!("rows: after {i} x 1000 ({leftover:+} visuals)"), usage());
            }
        }));
    }

    for _ in 0..50 {
        steps.push(Box::new(|| {}));
    }
    steps.push(Box::new(|| print_usage("rows: idle for 1 s", usage())));

    // Popups opened and closed, each one shown and synced in between.
    steps.push(Box::new(|| print_usage("popups: before", usage())));
    for i in 1..=50 * rounds {
        let weak_open = weak.clone();
        steps.push(Box::new(move || weak_open.upgrade().unwrap().invoke_open_popup()));
        let weak_close = weak.clone();
        steps.push(Box::new(move || {
            weak_close.upgrade().unwrap().invoke_close_popup();
            if i % (25 * rounds) == 0 {
                print_usage(&format!("popups: after {i}"), usage());
            }
        }));
    }

    // Second windows created, shown and dropped.
    steps.push(Box::new(|| print_usage("windows: before", usage())));
    let extra: Rc<RefCell<Option<Extra>>> = Default::default();
    for i in 1..=30 * rounds {
        let extra_show = extra.clone();
        steps.push(Box::new(move || {
            let window = Extra::new().unwrap();
            window.show().unwrap();
            *extra_show.borrow_mut() = Some(window);
        }));
        let extra_drop = extra.clone();
        steps.push(Box::new(move || {
            if let Some(window) = extra_drop.borrow_mut().take() {
                window.hide().unwrap();
            }
            if i % (15 * rounds) == 0 {
                print_usage(&format!("windows: after {i}"), usage());
            }
        }));
    }
    // One more tick, so that the last window's messages are dispatched.
    steps.push(Box::new(|| print_usage("windows: settled", usage())));

    run_steps(perf.window(), steps);
}
