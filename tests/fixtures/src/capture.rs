//! Reads back what the compositor draws for a visual, through
//! `GraphicsCaptureItem::CreateFromVisual` (spike S3, `docs/design/spike-s3-verification.md`).
//!
//! Capture works on a visual that hangs below no window. It needs a desktop session with DWM and
//! a hardware D3D11 device. The sequence for a pixel test is: run the sync pass, [`wait_commit`],
//! then [`Capturer::capture`].

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use windows::Graphics::Capture::{Direct3D11CaptureFramePool, GraphicsCaptureItem};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::UI::Composition::{Compositor, Visual};
use windows::Win32::Foundation::{E_FAIL, HMODULE};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
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

/// BGRA pixels, top-down, as captured (premultiplied alpha).
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

impl Image {
    /// `[r, g, b, a]` at `(x, y)`.
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        assert!(
            x < self.width && y < self.height,
            "({x},{y}) is outside the {}x{} image",
            self.width,
            self.height
        );
        let i = ((y * self.width + x) * 4) as usize;
        let p = &self.bgra[i..i + 4];
        [p[2], p[1], p[0], p[3]]
    }
}

/// Pumps messages until `done` returns true or `timeout` passes. Returns whether `done` held.
pub fn pump_until(timeout: Duration, mut done: impl FnMut() -> bool) -> bool {
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
        if start.elapsed() > timeout {
            return false;
        }
        unsafe {
            MsgWaitForMultipleObjectsEx(None, 1, QS_ALLINPUT, MWMO_INPUTAVAILABLE);
        }
    }
}

/// Requests a commit and pumps messages until the compositor reports it done.
pub fn wait_commit(compositor: &Compositor) -> Result<()> {
    let done = Arc::new(AtomicBool::new(false));
    let flag = done.clone();
    compositor.RequestCommitAsync()?.SetCompleted(&AsyncActionCompletedHandler::new(
        move |_, _| {
            flag.store(true, Ordering::Release);
            Ok(())
        },
    ))?;
    if pump_until(Duration::from_secs(5), || done.load(Ordering::Acquire)) {
        Ok(())
    } else {
        Err(windows::core::Error::new(E_FAIL, "the compositor didn't commit in time"))
    }
}

/// A D3D11 device that captures read back through.
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

    /// Captures `visual` in its own coordinate space, one unit per pixel. The visual needs a
    /// size.
    pub fn capture(&self, visual: &impl Interface) -> Result<Image> {
        let item = GraphicsCaptureItem::CreateFromVisual(&visual.cast::<Visual>()?)?;
        let size = item.Size()?;
        let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
            &self.device,
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            1,
            size,
        )?;
        let session = pool.CreateCaptureSession(&item)?;
        let _ = session.SetIsBorderRequired(false);
        let _ = session.SetIsCursorCaptureEnabled(false);
        session.StartCapture()?;
        let mut frame = None;
        let got = pump_until(Duration::from_secs(5), || {
            frame = pool.TryGetNextFrame().ok();
            frame.is_some()
        });
        let Some(frame) = frame.filter(|_| got) else {
            return Err(windows::core::Error::new(E_FAIL, "the capture timed out"));
        };
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
        Ok(image)
    }
}
