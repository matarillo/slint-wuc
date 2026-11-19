// Copyright © Kentaro Inomata
// SPDX-License-Identifier: MIT-0

//! Shared helpers for the spikes: thread setup, an HWND host, commit waiting,
//! visual capture to pixels, and a text dump of a visual tree.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use windows::Graphics::Capture::{Direct3D11CaptureFramePool, GraphicsCaptureItem};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::System::DispatcherQueueController;
use windows::UI::Color;
use windows::UI::Composition::Desktop::DesktopWindowTarget;
use windows::UI::Composition::{Compositor, ContainerVisual, Visual};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::WinRT::Composition::ICompositorDesktopInterop;
use windows::Win32::System::WinRT::Direct3D11::{
    CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess,
};
use windows::Win32::System::WinRT::*;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{IInspectable, Interface, Result, w};
use windows_future::AsyncActionCompletedHandler;
use windows_numerics::Vector2;

pub use windows_numerics::{Vector2 as V2, Vector3 as V3};

/// Per-monitor DPI awareness, an STA, and a dispatcher queue on the current thread.
pub fn init_thread() -> Result<DispatcherQueueController> {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        RoInitialize(RO_INIT_SINGLETHREADED)?;
        CreateDispatcherQueueController(DispatcherQueueOptions {
            dwSize: size_of::<DispatcherQueueOptions>() as u32,
            threadType: DQTYPE_THREAD_CURRENT,
            apartmentType: DQTAT_COM_STA,
        })
    }
}

pub fn rgba(r: u8, g: u8, b: u8, a: u8) -> Color {
    Color { A: a, R: r, G: g, B: b }
}

pub fn ms(since: Instant) -> f64 {
    since.elapsed().as_secs_f64() * 1000.0
}

/// A top-level window with a composition target; `root` fills the client area in physical pixels.
pub struct Host {
    pub hwnd: HWND,
    pub target: DesktopWindowTarget,
    pub root: ContainerVisual,
}

impl Host {
    pub fn new(compositor: &Compositor, title: &str, client_w: i32, client_h: i32) -> Result<Self> {
        unsafe {
            let instance = GetModuleHandleW(None)?;
            let class = WNDCLASSW {
                lpfnWndProc: Some(wndproc),
                hInstance: instance.into(),
                lpszClassName: w!("SlintForkSpike"),
                hCursor: LoadCursorW(None, IDC_ARROW)?,
                ..Default::default()
            };
            RegisterClassW(&class);
            let mut rc = RECT { left: 0, top: 0, right: client_w, bottom: client_h };
            AdjustWindowRectExForDpi(&mut rc, WS_OVERLAPPEDWINDOW, false, WS_EX_NOREDIRECTIONBITMAP, GetDpiForSystem())?;
            let hwnd = CreateWindowExW(
                WS_EX_NOREDIRECTIONBITMAP,
                w!("SlintForkSpike"),
                &windows::core::HSTRING::from(title),
                WS_OVERLAPPEDWINDOW,
                40,
                40,
                rc.right - rc.left,
                rc.bottom - rc.top,
                None,
                None,
                Some(instance.into()),
                None,
            )?;
            let interop: ICompositorDesktopInterop = compositor.cast()?;
            let target = interop.CreateDesktopWindowTarget(hwnd, false)?;
            let root = compositor.CreateContainerVisual()?;
            root.SetRelativeSizeAdjustment(Vector2 { X: 1.0, Y: 1.0 })?;
            target.SetRoot(&root)?;
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            Ok(Self { hwnd, target, root })
        }
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}

/// Pumps messages until `done` returns true or `timeout` passes. Returns whether `done` held.
pub fn pump_until(timeout: Duration, mut done: impl FnMut() -> bool) -> bool {
    let t0 = Instant::now();
    let mut msg = MSG::default();
    loop {
        unsafe {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        if done() {
            return true;
        }
        if t0.elapsed() > timeout {
            return false;
        }
        unsafe {
            MsgWaitForMultipleObjectsEx(None, 1, QS_ALLINPUT, MWMO_INPUTAVAILABLE);
        }
    }
}

/// Requests a commit and pumps until the compositor reports it done. Returns the latency in ms.
pub fn wait_commit(compositor: &Compositor) -> Result<f64> {
    let t0 = Instant::now();
    let done = Arc::new(AtomicBool::new(false));
    let flag = done.clone();
    compositor.RequestCommitAsync()?.SetCompleted(&AsyncActionCompletedHandler::new(move |_, _| {
        flag.store(true, Ordering::Release);
        Ok(())
    }))?;
    pump_until(Duration::from_secs(5), || done.load(Ordering::Acquire));
    Ok(ms(t0))
}

/// BGRA pixels, top-down, as captured (premultiplied alpha).
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

impl Image {
    /// `[r, g, b, a]` at `(x, y)`.
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * self.width + x) * 4) as usize;
        let p = &self.bgra[i..i + 4];
        [p[2], p[1], p[0], p[3]]
    }

    pub fn save_png(&self, path: &str) {
        let file = std::io::BufWriter::new(std::fs::File::create(path).unwrap());
        let mut enc = png::Encoder::new(file, self.width, self.height);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut rgba = self.bgra.clone();
        for p in rgba.chunks_exact_mut(4) {
            p.swap(0, 2);
        }
        enc.write_header().unwrap().write_image_data(&rgba).unwrap();
    }
}

/// Reads back what the compositor renders for a visual, through `GraphicsCaptureItem::CreateFromVisual`.
pub struct Capturer {
    d3d: ID3D11Device,
    context: ID3D11DeviceContext,
    device: IDirect3DDevice,
}

impl Capturer {
    pub fn new() -> Result<Self> {
        unsafe {
            let mut d3d = None;
            let mut context = None;
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut d3d),
                None,
                Some(&mut context),
            )?;
            let d3d: ID3D11Device = d3d.unwrap();
            let dxgi: IDXGIDevice = d3d.cast()?;
            let device: IDirect3DDevice = CreateDirect3D11DeviceFromDXGIDevice(&dxgi)?.cast()?;
            Ok(Self { d3d, context: context.unwrap(), device })
        }
    }

    /// Captures `visual` (its own coordinate space, one unit per pixel). Returns the image and
    /// the time from starting the session to receiving the frame.
    pub fn capture(&self, visual: &impl Interface) -> Result<(Image, f64)> {
        let item = GraphicsCaptureItem::CreateFromVisual(&visual.cast::<Visual>()?)?;
        self.capture_timed(&item)
    }

    /// Captures any capture item, such as a window's.
    pub fn capture_item(&self, item: &GraphicsCaptureItem) -> Result<Image> {
        Ok(self.capture_timed(item)?.0)
    }

    fn capture_timed(&self, item: &GraphicsCaptureItem) -> Result<(Image, f64)> {
        let size = item.Size()?;
        let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
            &self.device,
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            1,
            size,
        )?;
        let session = pool.CreateCaptureSession(item)?;
        let _ = session.SetIsBorderRequired(false);
        let _ = session.SetIsCursorCaptureEnabled(false);
        let t0 = Instant::now();
        session.StartCapture()?;
        let mut frame = None;
        let got = pump_until(Duration::from_secs(5), || {
            frame = pool.TryGetNextFrame().ok();
            frame.is_some()
        });
        let elapsed = ms(t0);
        if !got {
            return Err(windows::core::Error::new(E_FAIL, "capture timed out"));
        }
        let frame = frame.unwrap();
        let content = frame.ContentSize()?;
        let access: IDirect3DDxgiInterfaceAccess = frame.Surface()?.cast()?;
        let image = unsafe {
            let texture: ID3D11Texture2D = access.GetInterface()?;
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            texture.GetDesc(&mut desc);
            desc.Usage = D3D11_USAGE_STAGING;
            desc.BindFlags = 0;
            desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
            desc.MiscFlags = 0;
            let mut staging = None;
            self.d3d.CreateTexture2D(&desc, None, Some(&mut staging))?;
            let staging = staging.unwrap();
            self.context.CopyResource(&staging, &texture);
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            self.context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
            let width = (content.Width as u32).min(desc.Width);
            let height = (content.Height as u32).min(desc.Height);
            let mut bgra = Vec::with_capacity((width * height * 4) as usize);
            for y in 0..height {
                let row = std::slice::from_raw_parts(
                    (mapped.pData as *const u8).add((y * mapped.RowPitch) as usize),
                    (width * 4) as usize,
                );
                bgra.extend_from_slice(row);
            }
            self.context.Unmap(&staging, 0);
            Image { width, height, bgra }
        };
        session.Close()?;
        pool.Close()?;
        Ok((image, elapsed))
    }
}

/// One line per visual: runtime class, offset, size, and the properties the runtime sets.
pub fn dump_tree(visual: &Visual) -> String {
    let mut out = String::new();
    dump_into(visual, 0, &mut out);
    out
}

fn dump_into(visual: &Visual, depth: usize, out: &mut String) {
    let class = visual
        .cast::<IInspectable>()
        .and_then(|i| i.GetRuntimeClassName())
        .map(|s| s.to_string().rsplit('.').next().unwrap_or_default().to_owned())
        .unwrap_or_default();
    let o = visual.Offset().unwrap_or_default();
    let s = visual.Size().unwrap_or_default();
    let mut line = format!("{:indent$}{class} offset=({},{}) size=({},{})", "", o.X, o.Y, s.X, s.Y, indent = depth * 2);
    if let Ok(op) = visual.Opacity() {
        if op != 1.0 {
            line += &format!(" opacity={op}");
        }
    }
    let sc = visual.Scale().unwrap_or_default();
    if (sc.X, sc.Y) != (1.0, 1.0) {
        line += &format!(" scale=({},{})", sc.X, sc.Y);
    }
    if visual.Clip().is_ok() {
        line += " clip";
    }
    out.push_str(&line);
    out.push('\n');
    if let Ok(container) = visual.cast::<ContainerVisual>() {
        if let Ok(children) = container.Children() {
            for child in children {
                dump_into(&child, depth + 1, out);
            }
        }
    }
}

/// Private bytes of this process.
pub fn private_bytes() -> usize {
    use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX};
    use windows::Win32::System::Threading::GetCurrentProcess;
    let mut c = PROCESS_MEMORY_COUNTERS_EX { cb: size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32, ..Default::default() };
    unsafe {
        let _ = GetProcessMemoryInfo(GetCurrentProcess(), &mut c as *mut _ as *mut PROCESS_MEMORY_COUNTERS, c.cb);
    }
    c.PrivateUsage
}

/// Private bytes of dwm.exe, where the composition resources live, if this user may query it.
pub fn dwm_private_bytes() -> Option<usize> {
    use windows::Win32::System::Diagnostics::ToolHelp::*;
    use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX};
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0).ok()?;
        let mut entry = PROCESSENTRY32W { dwSize: size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut pid = None;
        if Process32FirstW(snap, &mut entry).is_ok() {
            loop {
                let name = String::from_utf16_lossy(&entry.szExeFile);
                if name.trim_end_matches('\0').eq_ignore_ascii_case("dwm.exe") {
                    pid = Some(entry.th32ProcessID);
                    break;
                }
                if Process32NextW(snap, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snap);
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid?).ok()?;
        let mut c = PROCESS_MEMORY_COUNTERS_EX { cb: size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32, ..Default::default() };
        let ok = GetProcessMemoryInfo(process, &mut c as *mut _ as *mut PROCESS_MEMORY_COUNTERS, c.cb).is_ok();
        let _ = CloseHandle(process);
        ok.then_some(c.PrivateUsage)
    }
}
