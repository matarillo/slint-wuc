// Copyright © Kentaro Inomata
// SPDX-License-Identifier: MIT-0

//! Roadmap item 4: shows a swap chain of the application's own D3D11 device in image items of
//! `ui/embed.slint`, presented from a thread of its own at the display's rate.
//!
//! The application side passes and takes only raw COM pointers, as an application with another
//! version of the `windows` crate would.
//!
//! - No argument: shows the window. `--scroll` scrolls the content up by 150px first.
//! - `--web` also shows a WebView2 in the fourth image, and forwards its input (`embed/web.rs`).
//! - `--input-check <dir>`: with `--web`, posts input to the window, prints what arrived, writes
//!   `embed-input.png`, and quits.
//! - `--check <dir>`: counts presents and sync passes for 3 s, then writes `embed.png` and,
//!   scrolled, `embed-scrolled.png` with `take_snapshot`, prints the visual tree, and quits.

slint::include_modules!();

#[path = "embed/web.rs"]
mod web;

use slint::ComponentHandle;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};
use windows::UI::Composition::{CompositionStretch, Compositor, ICompositionSurface};
use windows::Win32::Foundation::{HMODULE, RECT};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION, D3D11CreateDevice, ID3D11Device,
    ID3D11DeviceContext1, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_PREMULTIPLIED, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory2, DXGI_CREATE_FACTORY_FLAGS, DXGI_PRESENT, DXGI_SCALING_STRETCH,
    DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL, DXGI_USAGE_RENDER_TARGET_OUTPUT,
    IDXGIFactory2, IDXGISwapChain1,
};
use windows::Win32::System::WinRT::Composition::ICompositorInterop;
use windows::core::Interface;

const W: u32 = 640;
const H: u32 = 360;

/// The application's device, not the fork's.
struct Gpu {
    device: ID3D11Device,
    context: ID3D11DeviceContext1,
    swap_chain: IDXGISwapChain1,
}

// SAFETY: the device is free-threaded; only the render thread uses the immediate context.
unsafe impl Send for Gpu {}

fn create_gpu() -> windows::core::Result<Gpu> {
    let (mut device, mut context) = (None, None);
    unsafe {
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )?;
    }
    let device = device.unwrap();
    let context: ID3D11DeviceContext1 = context.unwrap().cast()?;
    let factory: IDXGIFactory2 = unsafe { CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0))? };
    let desc = DXGI_SWAP_CHAIN_DESC1 {
        Width: W,
        Height: H,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
        BufferCount: 2,
        Scaling: DXGI_SCALING_STRETCH,
        SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
        AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
        ..Default::default()
    };
    let swap_chain = unsafe { factory.CreateSwapChainForComposition(&device, &desc, None)? };
    Ok(Gpu { device, context, swap_chain })
}

fn rect(l: i32, t: i32, r: i32, b: i32) -> RECT {
    RECT { left: l, top: t, right: r, bottom: b }
}

/// One frame: a background that pulses, a marker in each corner (red, green, blue, yellow
/// clockwise from the top left), and a white bar that crosses the frame once a second.
fn draw(gpu: &Gpu, frame: u64) -> windows::core::Result<()> {
    let buffer: ID3D11Texture2D = unsafe { gpu.swap_chain.GetBuffer(0)? };
    let mut view = None;
    unsafe { gpu.device.CreateRenderTargetView(&buffer, None, Some(&mut view))? };
    let view = view.unwrap();
    let t = frame as f32 / 60.;
    let (w, h, m) = (W as i32, H as i32, 48);
    let bar = ((frame % 60) as i32 * (w - 20)) / 60;
    unsafe {
        let c = &gpu.context;
        c.ClearView(&view, &[0.1, 0.25 + 0.2 * (t * 3.).sin().abs(), 0.45, 1.], None);
        c.ClearView(&view, &[1., 0., 0., 1.], Some(&[rect(0, 0, m, m)]));
        c.ClearView(&view, &[0., 1., 0., 1.], Some(&[rect(w - m, 0, w, m)]));
        c.ClearView(&view, &[1., 1., 0., 1.], Some(&[rect(w - m, h - m, w, h)]));
        c.ClearView(&view, &[0., 0., 1., 1.], Some(&[rect(0, h - m, m, h)]));
        c.ClearView(&view, &[1., 1., 1., 1.], Some(&[rect(bar, m, bar + 20, h - m)]));
        gpu.swap_chain.Present(1, DXGI_PRESENT(0)).ok()
    }
}

/// Renders until `stop`, counting presents.
fn spawn_renderer(gpu: Gpu, stop: Arc<AtomicBool>, presents: Arc<AtomicU64>) {
    std::thread::spawn(move || {
        let mut frame = 0;
        while !stop.load(Ordering::Relaxed) {
            if let Err(e) = draw(&gpu, frame) {
                println!("render thread: {e}");
                return;
            }
            frame += 1;
            presents.store(frame, Ordering::Relaxed);
        }
    });
}

/// Three images, each of a sprite of its own over the same swap chain: a visual has one parent.
fn images(window: &slint::Window, gpu: &Gpu) -> windows::core::Result<[slint::Image; 3]> {
    let host = i_slint_core::host::Win32WindowAdapter::from_window(window).unwrap();
    let raw = host.compositor_abi();
    let compositor = unsafe { Compositor::from_raw_borrowed(&raw) }.unwrap().clone();
    let interop: ICompositorInterop = compositor.cast()?;
    let surface: ICompositionSurface =
        unsafe { interop.CreateCompositionSurfaceForSwapChain(&gpu.swap_chain)? };
    let brush = compositor.CreateSurfaceBrushWithSurface(&surface)?;
    brush.SetStretch(CompositionStretch::Fill)?;
    let image = || -> windows::core::Result<slint::Image> {
        let sprite = compositor.CreateSpriteVisual()?;
        sprite.SetBrush(&brush)?;
        let size = i_slint_core::graphics::IntSize::new(W, H);
        Ok(unsafe {
            i_slint_core::visual_tree::image::external_visual_image(sprite.as_raw(), size)
        }
        .unwrap())
    };
    Ok([image()?, image()?, image()?])
}

fn save(window: &slint::Window, path: &std::path::Path) {
    match window.take_snapshot() {
        Ok(shot) => {
            image::save_buffer(
                path,
                shot.as_bytes(),
                shot.width(),
                shot.height(),
                image::ColorType::Rgba8,
            )
            .unwrap();
            println!("wrote {}", path.display());
        }
        Err(e) => println!("no snapshot: {e}"),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let check = args
        .iter()
        .position(|a| a == "--check")
        .map(|i| std::path::PathBuf::from(args.get(i + 1).expect("--check <dir>")));
    let app = Embed::new().unwrap();
    app.show().unwrap();
    let gpu = create_gpu().unwrap();
    let [plain, clipped, covered] = images(app.window(), &gpu).unwrap();
    app.set_plain(plain);
    app.set_clipped(clipped);
    app.set_covered(covered);
    if args.iter().any(|a| a == "--scroll") {
        app.set_scroll(-150.);
    }
    let input_check = args
        .iter()
        .position(|a| a == "--input-check")
        .map(|i| std::path::PathBuf::from(args.get(i + 1).expect("--input-check <dir>")));
    let web = input_check.is_some() || args.iter().any(|a| a == "--web");
    if web {
        web::start(&app);
    }
    let stop = Arc::new(AtomicBool::new(false));
    let presents = Arc::new(AtomicU64::new(0));
    spawn_renderer(gpu, stop.clone(), presents.clone());

    if let Some(dir) = input_check {
        web::input_check(&app, dir);
    }
    let Some(dir) = check else {
        app.run().unwrap();
        web::close();
        stop.store(true, Ordering::Relaxed);
        return;
    };
    let weak = app.as_weak();
    // WebView2 starts its browser processes first.
    let settle = Duration::from_millis(if web { 4000 } else { 500 });
    slint::Timer::single_shot(settle, move || {
        let app = weak.upgrade().unwrap();
        let host = i_slint_core::host::Win32WindowAdapter::from_window(app.window()).unwrap();
        let (passes, frames, start) =
            (host.sync_passes(), presents.load(Ordering::Relaxed), Instant::now());
        let weak = app.as_weak();
        slint::Timer::single_shot(Duration::from_secs(3), move || {
            let app = weak.upgrade().unwrap();
            let host = i_slint_core::host::Win32WindowAdapter::from_window(app.window()).unwrap();
            println!(
                "in {:.2} s: {} presents, {} sync passes",
                start.elapsed().as_secs_f32(),
                presents.load(Ordering::Relaxed) - frames,
                host.sync_passes() - passes
            );
            save(app.window(), &dir.join("embed.png"));
            app.set_scroll(-150.);
            let weak = app.as_weak();
            slint::Timer::single_shot(Duration::from_millis(300), move || {
                let app = weak.upgrade().unwrap();
                save(app.window(), &dir.join("embed-scrolled.png"));
                let host =
                    i_slint_core::host::Win32WindowAdapter::from_window(app.window()).unwrap();
                println!("{}", host.dump_visuals());
                stop.store(true, Ordering::Relaxed);
                slint::quit_event_loop().unwrap();
            });
        });
    });
    app.run().unwrap();
    web::close();
}
