// Copyright © Kentaro Inomata
// SPDX-License-Identifier: MIT-0

//! Shows `ui/caption.slint` with its content extended into the title bar, through the core's
//! `host::title_bar`, and logs what the window does: sizes, hit tests at the caption buttons,
//! and a capture of the screen where it is.
//!
//! Flags: `--no-mica` (no backdrop), `--opaque-bg` (a solid background), `--dark` (a dark
//! background and title bar), `--maximize` (also maximized, then restored), `--after-show`
//! (extends only once shown), `--toggle-off` (stops extending at the end), `--gestures`
//! (a double click and a right click on the title bar, as Slint events), `--recolor` (switches
//! between the light and the dark background once shown), `--out=<dir>`,
//! `--tag=<name>`, `--stay`.

slint::include_modules!();

use i_slint_core::host::title_bar::{self, TitleBar};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use slint::ComponentHandle;
use slint::platform::{PointerEventButton, WindowEvent};
use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::BOOL;

fn metric(m: SYSTEM_METRICS_INDEX, dpi: u32) -> i32 {
    unsafe { GetSystemMetricsForDpi(m, dpi) }
}

fn name(hit: isize) -> &'static str {
    match hit as u32 {
        HTCLIENT => "HTCLIENT",
        HTCAPTION => "HTCAPTION",
        HTMINBUTTON => "HTMINBUTTON",
        HTMAXBUTTON => "HTMAXBUTTON",
        HTCLOSE => "HTCLOSE",
        HTTOP => "HTTOP",
        HTTOPLEFT => "HTTOPLEFT",
        HTTOPRIGHT => "HTTOPRIGHT",
        HTLEFT => "HTLEFT",
        HTRIGHT => "HTRIGHT",
        HTBOTTOM => "HTBOTTOM",
        HTSYSMENU => "HTSYSMENU",
        HTNOWHERE => "HTNOWHERE",
        _ => "?",
    }
}

fn button_bounds(hwnd: HWND) -> RECT {
    let mut buttons = RECT::default();
    unsafe {
        let _ = DwmGetWindowAttribute(
            hwnd,
            DWMWA_CAPTION_BUTTON_BOUNDS,
            (&mut buttons as *mut RECT).cast(),
            size_of::<RECT>() as u32,
        );
    }
    buttons
}

fn rects(hwnd: HWND, app: &CaptionProbe, what: &str) {
    let mut w = RECT::default();
    let mut c = RECT::default();
    let mut origin = POINT::default();
    let mut monitor = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
    unsafe {
        let _ = GetWindowRect(hwnd, &mut w);
        let _ = GetClientRect(hwnd, &mut c);
        let _ = ClientToScreen(hwnd, &mut origin);
        let _ = GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST), &mut monitor);
    }
    let window = app.window();
    let logical = window.size().to_logical(window.scale_factor());
    println!(
        "[{what}] window={}x{} at ({},{}) client={}x{} at ({},{}) slint={}x{} ({}x{} logical) \
         buttons(window coords)={:?} work={:?} zoomed={} extends={} title_bar={:?}",
        w.right - w.left,
        w.bottom - w.top,
        w.left,
        w.top,
        c.right,
        c.bottom,
        origin.x,
        origin.y,
        window.size().width,
        window.size().height,
        logical.width,
        logical.height,
        button_bounds(hwnd),
        monitor.rcWork,
        unsafe { IsZoomed(hwnd).as_bool() },
        title_bar::extends_content_into_title_bar(window),
        title_bar::title_bar(window),
    );
}

fn hit(hwnd: HWND, x: i32, y: i32) -> &'static str {
    let lp = LPARAM(((y as u16 as isize) << 16) | (x as u16 as isize));
    name(unsafe { SendMessageW(hwnd, WM_NCHITTEST, Some(WPARAM(0)), Some(lp)) }.0)
}

fn hit_tests(hwnd: HWND, app: &CaptionProbe) {
    let mut w = RECT::default();
    let mut origin = POINT::default();
    unsafe {
        let _ = GetWindowRect(hwnd, &mut w);
        let _ = ClientToScreen(hwnd, &mut origin);
    }
    let buttons = button_bounds(hwnd);
    let bw = (buttons.right - buttons.left) / 3;
    let by = w.top + (buttons.top + buttons.bottom) / 2;
    let bx = w.left + buttons.left;
    let scale = app.window().scale_factor();
    // Where the content starts: below a maximized window's margin.
    let content_top = match title_bar::title_bar(app.window()) {
        Some(t) => w.top + buttons.top - (t.buttons.origin.y * scale).round() as i32,
        None => origin.y,
    };
    let band = (app.get_band_height() * scale) as i32;
    let px = |v: f32| (v * scale).round() as i32;
    let points = [
        ("min button center", bx + bw / 2, by),
        ("max button center", bx + bw + bw / 2, by),
        ("close button center", bx + 2 * bw + bw / 2, by),
        ("move area, x=60", origin.x + px(60.), content_top + band / 2),
        ("in-band Button", origin.x + px(210.), content_top + band / 2),
        ("content top row y=1, x=60", origin.x + px(60.), content_top + 1),
        ("below the title bar", origin.x + px(60.), content_top + band + px(20.)),
        ("left edge, mid band", w.left + 2, content_top + band / 2),
        ("bottom edge", origin.x + px(60.), w.bottom - 2),
    ];
    for (what, x, y) in points {
        println!("WM_NCHITTEST {what} at ({x},{y}): {}", hit(hwnd, x, y));
    }
}

/// Copies the screen pixels of the window rect (plus 8px) to a PNG, and logs a few pixels.
fn capture(hwnd: HWND, path: &std::path::Path) {
    let mut w = RECT::default();
    unsafe {
        let _ = GetWindowRect(hwnd, &mut w);
    }
    let buttons = button_bounds(hwnd);
    let pad = 8;
    let (x0, y0) = (w.left - pad, w.top - pad);
    let (width, height) = (w.right - w.left + 2 * pad, w.bottom - w.top + 2 * pad);
    unsafe {
        let screen = GetDC(None);
        let mem = CreateCompatibleDC(Some(screen));
        let bmp = CreateCompatibleBitmap(screen, width, height);
        let old = SelectObject(mem, bmp.into());
        let _ = BitBlt(mem, 0, 0, width, height, Some(screen), x0, y0, SRCCOPY | CAPTUREBLT);
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = vec![0u8; (width * height * 4) as usize];
        GetDIBits(
            mem,
            bmp,
            0,
            height as u32,
            Some(bits.as_mut_ptr().cast()),
            &mut info,
            DIB_RGB_COLORS,
        );
        SelectObject(mem, old);
        let _ = DeleteObject(bmp.into());
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
        let mut img = image::RgbaImage::new(width as u32, height as u32);
        for (i, p) in img.pixels_mut().enumerate() {
            let b = &bits[i * 4..i * 4 + 4];
            *p = image::Rgba([b[2], b[1], b[0], 255]);
        }
        let px = |x: i32, y: i32| {
            let x = (x - x0).clamp(0, width - 1) as u32;
            let y = (y - y0).clamp(0, height - 1) as u32;
            let p = img.get_pixel(x, y);
            format!("#{:02x}{:02x}{:02x}", p[0], p[1], p[2])
        };
        let bw = (buttons.right - buttons.left) / 3;
        let by = w.top + (buttons.top + buttons.bottom) / 2;
        let bx = w.left + buttons.left;
        for (what, x, y) in [
            ("close glyph center", bx + 2 * bw + bw / 2, by),
            ("close button bg (left part)", bx + 2 * bw + 4, by),
            ("min button bg (left part)", bx + 4, by),
            ("band left of buttons", bx - 30, by),
            ("below the buttons", bx + 4, w.top + buttons.bottom + 4),
        ] {
            println!("pixel {what} ({x},{y}): {}", px(x, y));
        }
        img.save(path).unwrap();
        println!("saved {}", path.display());
    }
}

fn hwnd_of(window: &slint::Window) -> HWND {
    let handle = window.window_handle();
    let RawWindowHandle::Win32(win32) = handle.window_handle().unwrap().as_raw() else {
        panic!("not a Win32 window");
    };
    HWND(win32.hwnd.get() as _)
}

fn set_attribute<T>(hwnd: HWND, attribute: DWMWINDOWATTRIBUTE, value: &T) {
    let r = unsafe {
        DwmSetWindowAttribute(hwnd, attribute, (value as *const T).cast(), size_of::<T>() as u32)
    };
    println!("DwmSetWindowAttribute({}): {r:?}", attribute.0);
}

/// A click on the title bar at `x` logical pixels, as Slint events.
fn click(window: &slint::Window, x: f32, y: f32, button: PointerEventButton) {
    let position = slint::LogicalPosition::new(x, y);
    window.dispatch_event(WindowEvent::PointerMoved { position });
    window.dispatch_event(WindowEvent::PointerPressed { position, button });
    window.dispatch_event(WindowEvent::PointerReleased { position, button });
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let has = |f: &str| args.iter().any(|a| a == f);
    let value = |k: &str| args.iter().find_map(|a| a.strip_prefix(k)).map(str::to_owned);
    let out = std::path::PathBuf::from(value("--out=").unwrap_or_else(|| ".".into()));
    let tag = value("--tag=").unwrap_or_else(|| "caption".into());

    let app = CaptionProbe::new().unwrap();
    app.set_opaque_bg(has("--opaque-bg"));
    app.set_dark_bg(has("--dark"));
    let weak = app.as_weak();
    title_bar::on_title_bar_changed(app.window(), move |t: Option<TitleBar>| {
        println!("title bar changed: {t:?}");
        let app = weak.unwrap();
        if let Some(t) = t {
            app.set_band_height(t.height);
            app.set_buttons_x(t.buttons.origin.x);
        }
    });
    let after_show = has("--after-show");
    if !after_show {
        title_bar::set_extends_content_into_title_bar(app.window(), true);
    }
    println!(
        "before show: extends={} title_bar={:?}",
        title_bar::extends_content_into_title_bar(app.window()),
        title_bar::title_bar(app.window())
    );
    app.show().unwrap();
    let hwnd = hwnd_of(app.window());
    if !has("--no-mica") {
        set_attribute(hwnd, DWMWA_SYSTEMBACKDROP_TYPE, &DWMSBT_MAINWINDOW);
    }
    if has("--dark") {
        set_attribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &BOOL::from(true));
    }
    // Toward the right edge of the primary monitor, above other windows so that the capture
    // shows the probe: a process started in the background doesn't get the foreground.
    let mut monitor = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
    unsafe {
        let _ = GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTOPRIMARY), &mut monitor);
        let mut w = RECT::default();
        let _ = GetWindowRect(hwnd, &mut w);
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            monitor.rcWork.right - (w.right - w.left) - 40,
            monitor.rcWork.top + 120,
            0,
            0,
            SWP_NOSIZE,
        );
    }
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    println!(
        "dpi={dpi} scale={} SM_CYCAPTION={} SM_CYFRAME={} SM_CXPADDEDBORDER={}",
        app.window().scale_factor(),
        metric(SM_CYCAPTION, dpi),
        metric(SM_CYFRAME, dpi),
        metric(SM_CXPADDEDBORDER, dpi),
    );
    rects(hwnd, &app, "shown");

    let maximize = has("--maximize");
    let gestures = has("--gestures");
    let toggle_off = has("--toggle-off");
    let recolor = has("--recolor");
    let stay = has("--stay");
    let weak = app.as_weak();
    let step = std::rc::Rc::new(std::cell::Cell::new(0));
    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, std::time::Duration::from_millis(1200), move || {
        let app = weak.unwrap();
        let window = app.window();
        let n = step.get();
        step.set(n + 1);
        match n {
            0 if after_show => {
                rects(hwnd, &app, "before extending");
                title_bar::set_extends_content_into_title_bar(window, true);
                rects(hwnd, &app, "extended after show");
            }
            1 => {
                rects(hwnd, &app, "settled");
                hit_tests(hwnd, &app);
                capture(hwnd, &out.join(format!("{tag}.png")));
                if gestures {
                    // A double click on the move area maximizes.
                    click(window, 60., 10., PointerEventButton::Left);
                    click(window, 60., 10., PointerEventButton::Left);
                } else if maximize {
                    unsafe {
                        let _ = ShowWindow(hwnd, SW_MAXIMIZE);
                    }
                }
            }
            2 if maximize || gestures => {
                rects(hwnd, &app, "maximized");
                hit_tests(hwnd, &app);
                capture(hwnd, &out.join(format!("{tag}-max.png")));
            }
            3 if gestures => {
                // A double click restores; then a right click opens the system menu.
                click(window, 60., 10., PointerEventButton::Left);
                click(window, 60., 10., PointerEventButton::Left);
            }
            4 if gestures => {
                rects(hwnd, &app, "restored by double click");
                click(window, 60., 10., PointerEventButton::Right);
            }
            5 if gestures => {
                capture(hwnd, &out.join(format!("{tag}-menu.png")));
                unsafe {
                    let _ = EndMenu();
                }
            }
            3 if maximize => unsafe {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            },
            4 if maximize => rects(hwnd, &app, "restored"),
            6 if toggle_off => {
                title_bar::set_extends_content_into_title_bar(window, false);
                rects(hwnd, &app, "stopped extending");
            }
            7 if toggle_off => {
                rects(hwnd, &app, "not extended, settled");
                capture(hwnd, &out.join(format!("{tag}-off.png")));
            }
            8 if recolor => app.set_dark_bg(!app.get_dark_bg()),
            9 if recolor => capture(hwnd, &out.join(format!("{tag}-recolor.png"))),
            0..=9 => {}
            _ if !stay => {
                let _ = slint::quit_event_loop();
            }
            _ => {}
        }
    });
    app.run().unwrap();
}
