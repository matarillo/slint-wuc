// SPDX-License-Identifier: MIT OR Apache-2.0

//! `Window::take_snapshot()`: what the compositor draws for a window's visuals, read back
//! through `GraphicsCaptureItem::CreateFromVisual` (S3).
//!
//! Capturing needs a desktop session with DWM; the window doesn't need an HWND.

#![allow(unsafe_code)]

use crate::api::PhysicalSize;
use crate::graphics::{Rgba8Pixel, SharedPixelBuffer};
use crate::platform::PlatformError;
use core::cell::OnceCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use windows::Graphics::Capture::{Direct3D11CaptureFramePool, GraphicsCaptureItem};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::UI::Composition::Desktop::DesktopWindowTarget;
use windows::UI::Composition::{Compositor, ContainerVisual, Visual};
use windows::Win32::Foundation::{E_FAIL, HMODULE};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE, D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ,
    D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::System::WinRT::Direct3D11::{
    CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, MSG, MWMO_INPUTAVAILABLE, MsgWaitForMultipleObjectsEx, PM_REMOVE,
    PeekMessageW, QS_ALLINPUT, TranslateMessage,
};
use windows::core::{Interface, Result};
use windows_future::AsyncActionCompletedHandler;

/// How long a commit or a captured frame may take.
const TIMEOUT: Duration = Duration::from_secs(5);

/// The pixels of `root`, a window's root visual, over `size` physical pixels.
///
/// The root moves into a frame of that size for the capture, since a visual is captured in its
/// own units and the root carries the scale factor. `target` shows the frame meanwhile.
pub(super) fn capture(
    compositor: &Compositor,
    root: &ContainerVisual,
    target: Option<&DesktopWindowTarget>,
    size: PhysicalSize,
) -> core::result::Result<SharedPixelBuffer<Rgba8Pixel>, PlatformError> {
    let err = |e: windows::core::Error| PlatformError::Other(e.message());
    if size.width == 0 || size.height == 0 {
        return Err(PlatformError::Other("the window has no size to take a snapshot of".into()));
    }
    let frame = compositor.CreateContainerVisual().map_err(err)?;
    frame
        .SetSize(windows_numerics::Vector2 { X: size.width as f32, Y: size.height as f32 })
        .map_err(err)?;
    if let Some(target) = target {
        target.SetRoot(&frame).map_err(err)?;
    }
    let children = frame.Children().map_err(err)?;
    children.InsertAtTop(root).map_err(err)?;
    let result = wait_commit(compositor).and_then(|()| capture_visual(&frame));
    let _ = children.Remove(root);
    if let Some(target) = target {
        let _ = target.SetRoot(root);
    }
    result.map_err(err)
}

/// Pumps messages until `done` holds or the timeout passes; returns whether `done` held.
fn pump_until(mut done: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
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
        if start.elapsed() > TIMEOUT {
            return false;
        }
        unsafe {
            MsgWaitForMultipleObjectsEx(None, 1, QS_ALLINPUT, MWMO_INPUTAVAILABLE);
        }
    }
}

/// Requests a commit and pumps messages until the compositor reports it done.
fn wait_commit(compositor: &Compositor) -> Result<()> {
    let done = Arc::new(AtomicBool::new(false));
    let flag = done.clone();
    compositor.RequestCommitAsync()?.SetCompleted(&AsyncActionCompletedHandler::new(
        move |_, _| {
            flag.store(true, Ordering::Release);
            Ok(())
        },
    ))?;
    if pump_until(|| done.load(Ordering::Acquire)) {
        Ok(())
    } else {
        Err(windows::core::Error::new(E_FAIL, "the compositor didn't commit in time"))
    }
}

/// The D3D11 device that captures are read back through.
struct Capturer {
    d3d: ID3D11Device,
    context: ID3D11DeviceContext,
    device: IDirect3DDevice,
}

/// The thread's capture device, never released: like the drawing devices
/// (`visual_tree/graphics.rs`), releasing it while the process exits can wait forever.
struct ThreadCapturer(OnceCell<Option<Capturer>>);

impl Drop for ThreadCapturer {
    fn drop(&mut self) {
        core::mem::forget(self.0.take());
    }
}

std::thread_local! {
    static CAPTURER: ThreadCapturer = const { ThreadCapturer(OnceCell::new()) };
}

impl Capturer {
    fn new() -> Result<Self> {
        let create = |driver: D3D_DRIVER_TYPE| unsafe {
            let mut d3d = None;
            let mut context = None;
            D3D11CreateDevice(
                None,
                driver,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut d3d),
                None,
                Some(&mut context),
            )
            .map(|()| (d3d.unwrap(), context.unwrap()))
        };
        let (d3d, context) =
            create(D3D_DRIVER_TYPE_HARDWARE).or_else(|_| create(D3D_DRIVER_TYPE_WARP))?;
        let dxgi: IDXGIDevice = d3d.cast()?;
        let device: IDirect3DDevice = unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi)? }.cast()?;
        Ok(Self { d3d, context, device })
    }
}

/// Captures `visual`, which has a size, in its own units: one per pixel.
fn capture_visual(visual: &ContainerVisual) -> Result<SharedPixelBuffer<Rgba8Pixel>> {
    CAPTURER.with(|ThreadCapturer(c)| {
        let capturer = c.get_or_init(|| match Capturer::new() {
            Ok(capturer) => Some(capturer),
            Err(e) => {
                crate::debug_log!("cannot create a device for snapshots: {}", e.message());
                None
            }
        });
        let Some(capturer) = capturer else {
            return Err(windows::core::Error::new(E_FAIL, "no device for snapshots"));
        };
        capturer.capture(visual)
    })
}

impl Capturer {
    fn capture(&self, visual: &ContainerVisual) -> Result<SharedPixelBuffer<Rgba8Pixel>> {
        let item = GraphicsCaptureItem::CreateFromVisual(&visual.cast::<Visual>()?)?;
        let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
            &self.device,
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            1,
            item.Size()?,
        )?;
        let session = pool.CreateCaptureSession(&item)?;
        let _ = session.SetIsBorderRequired(false);
        let _ = session.SetIsCursorCaptureEnabled(false);
        session.StartCapture()?;
        let mut frame = None;
        let got = pump_until(|| {
            frame = pool.TryGetNextFrame().ok();
            frame.is_some()
        });
        let Some(frame) = frame.filter(|_| got) else {
            return Err(windows::core::Error::new(E_FAIL, "the capture timed out"));
        };
        let content = frame.ContentSize()?;
        let access: IDirect3DDxgiInterfaceAccess = frame.Surface()?.cast()?;
        let pixels = unsafe {
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
            let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(width, height);
            let out = buffer.make_mut_slice();
            for y in 0..height {
                let row = core::slice::from_raw_parts(
                    (mapped.pData as *const u8).add((y * mapped.RowPitch) as usize),
                    (width * 4) as usize,
                );
                for (x, bgra) in row.chunks_exact(4).enumerate() {
                    out[(y * width) as usize + x] = unpremultiply(bgra);
                }
            }
            self.context.Unmap(&staging, 0);
            buffer
        };
        session.Close()?;
        pool.Close()?;
        Ok(pixels)
    }
}

/// A captured premultiplied BGRA pixel as straight RGBA.
fn unpremultiply(bgra: &[u8]) -> Rgba8Pixel {
    let a = bgra[3];
    let straight = |c: u8| match a {
        0 => 0,
        255 => c,
        _ => ((c as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8,
    };
    Rgba8Pixel { r: straight(bgra[2]), g: straight(bgra[1]), b: straight(bgra[0]), a }
}
