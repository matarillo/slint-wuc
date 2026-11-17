// SPDX-License-Identifier: MIT OR Apache-2.0

//! Spike S4: text laid out by DirectWrite, drawn by Direct2D into composition drawing surfaces.
//!
//! Usage:
//! - `s4-text <out-dir>` runs T1–T6 and saves the captures as PNGs in `<out-dir>`.
//! - `s4-text <out-dir> ime` opens a window for the manual input method check (T7).

use std::cell::RefCell;
use std::time::{Duration, Instant};

use spike_common::*;
use windows::Foundation::{Size, TypedEventHandler};
use windows::Graphics::DirectX::{DirectXAlphaMode, DirectXPixelFormat};
use windows::UI::Composition::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::System::WinRT::Composition::{
    ICompositionDrawingSurfaceInterop, ICompositionGraphicsDeviceInterop, ICompositorInterop,
};
use windows::core::{BOOL, Interface, PCWSTR, Result, w};
use windows_numerics::{Matrix3x2, Vector2};

const BLACK: D2D1_COLOR_F = D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 1.0 };
const WHITE: D2D1_COLOR_F = D2D1_COLOR_F { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };
const CLEAR: D2D1_COLOR_F = D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0 };

/// The devices a window's text needs.
struct Gfx {
    compositor: Compositor,
    d2d_factory: ID2D1Factory1,
    graphics: CompositionGraphicsDevice,
    dwrite: IDWriteFactory,
}

fn d2d_device(factory: &ID2D1Factory1) -> Result<ID2D1Device> {
    unsafe {
        let mut d3d = None;
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut d3d),
            None,
            None,
        )?;
        let dxgi: IDXGIDevice = d3d.unwrap().cast()?;
        factory.CreateDevice(&dxgi)
    }
}

impl Gfx {
    fn new(compositor: &Compositor) -> Result<Self> {
        let d2d_factory: ID2D1Factory1 =
            unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)? };
        let device = d2d_device(&d2d_factory)?;
        let interop: ICompositorInterop = compositor.cast()?;
        let graphics = unsafe { interop.CreateGraphicsDevice(&device)? };
        let dwrite: IDWriteFactory = unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)? };
        Ok(Self { compositor: compositor.clone(), d2d_factory, graphics, dwrite })
    }

    fn format(&self, family: PCWSTR, size: f32) -> Result<IDWriteTextFormat> {
        unsafe {
            self.dwrite.CreateTextFormat(
                family,
                None,
                DWRITE_FONT_WEIGHT_NORMAL,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                size,
                w!("ja-jp"),
            )
        }
    }

    fn layout(
        &self,
        text: &str,
        format: &IDWriteTextFormat,
        max_width: f32,
    ) -> Result<IDWriteTextLayout> {
        let wide: Vec<u16> = text.encode_utf16().collect();
        unsafe { self.dwrite.CreateTextLayout(&wide, format, max_width, f32::MAX) }
    }

    fn surface(&self, (w, h): (u32, u32)) -> Result<CompositionDrawingSurface> {
        self.graphics.CreateDrawingSurface(
            Size { Width: w as f32, Height: h as f32 },
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            DirectXAlphaMode::Premultiplied,
        )
    }

    /// A sprite showing `surface` (physical pixels) at a DIP offset, one surface pixel per
    /// physical pixel under a parent scaled by `scale`.
    fn sprite(
        &self,
        surface: &CompositionDrawingSurface,
        (w, h): (u32, u32),
        scale: f32,
        x: f32,
        y: f32,
        snap: Sampling,
    ) -> Result<SpriteVisual> {
        let brush = self.compositor.CreateSurfaceBrushWithSurface(surface)?;
        brush.SetStretch(CompositionStretch::Fill)?;
        brush.SetSnapToPixels(snap != Sampling::Free)?;
        if snap == Sampling::SnapNearest {
            brush
                .SetBitmapInterpolationMode(CompositionBitmapInterpolationMode::NearestNeighbor)?;
        }
        let sprite = self.compositor.CreateSpriteVisual()?;
        sprite.SetBrush(&brush)?;
        sprite.SetOffset(V3 { X: x, Y: y, Z: 0.0 })?;
        sprite.SetSize(V2 { X: w as f32 / scale, Y: h as f32 / scale })?;
        Ok(sprite)
    }
}

/// How the surface brush samples the surface.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Sampling {
    /// Linear interpolation, at whatever position the visual has.
    Free,
    /// `SnapToPixels`.
    Snap,
    /// `SnapToPixels` and nearest-neighbor interpolation.
    SnapNearest,
}

/// The layout's box in physical pixels.
fn physical_size(layout: &IDWriteTextLayout, scale: f32) -> Result<(u32, u32)> {
    let mut m = DWRITE_TEXT_METRICS::default();
    unsafe { layout.GetMetrics(&mut m)? };
    Ok((
        (m.widthIncludingTrailingWhitespace * scale).ceil().max(1.0) as u32,
        (m.height * scale).ceil().max(1.0) as u32,
    ))
}

#[derive(Clone, Copy)]
struct Draw {
    scale: f32,
    aa: D2D1_TEXT_ANTIALIAS_MODE,
    color_font: bool,
    color: D2D1_COLOR_F,
    background: D2D1_COLOR_F,
}

impl Draw {
    fn plain(scale: f32) -> Self {
        Self {
            scale,
            aa: D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE,
            color_font: true,
            color: BLACK,
            background: CLEAR,
        }
    }
}

/// Draws `layout` into the whole surface; returns the time BeginDraw..EndDraw took.
fn draw(surface: &CompositionDrawingSurface, layout: &IDWriteTextLayout, d: &Draw) -> Result<f64> {
    draw_with(surface, d, |dc, brush| unsafe {
        let options = if d.color_font {
            D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT
        } else {
            D2D1_DRAW_TEXT_OPTIONS_NONE
        };
        dc.DrawTextLayout(Vector2 { X: 0.0, Y: 0.0 }, layout, brush, options);
    })
}

fn draw_with(
    surface: &CompositionDrawingSurface,
    d: &Draw,
    f: impl FnOnce(&ID2D1DeviceContext, &ID2D1SolidColorBrush),
) -> Result<f64> {
    let t0 = Instant::now();
    let interop: ICompositionDrawingSurfaceInterop = surface.cast()?;
    unsafe {
        let mut offset = POINT::default();
        let dc: ID2D1DeviceContext = interop.BeginDraw(None, &mut offset)?;
        // The surface may live in an atlas: `offset` is where its pixels start.
        dc.SetDpi(96.0 * d.scale, 96.0 * d.scale);
        dc.SetTransform(&Matrix3x2::translation(
            offset.x as f32 / d.scale,
            offset.y as f32 / d.scale,
        ));
        dc.Clear(Some(&d.background));
        dc.SetTextAntialiasMode(d.aa);
        let brush = dc.CreateSolidColorBrush(&d.color, None)?;
        f(&dc, &brush);
        interop.EndDraw()?;
    }
    Ok(ms(t0))
}

/// A frame of `w`×`h` physical pixels with an optional white background, and a child scaled
/// by `scale` that holds the content in DIPs.
fn scene(
    c: &Compositor,
    w: u32,
    h: u32,
    scale: f32,
    background: bool,
) -> Result<(ContainerVisual, ContainerVisual)> {
    let frame = c.CreateContainerVisual()?;
    frame.SetSize(V2 { X: w as f32, Y: h as f32 })?;
    if background {
        let bg = c.CreateSpriteVisual()?;
        bg.SetRelativeSizeAdjustment(V2 { X: 1.0, Y: 1.0 })?;
        bg.SetBrush(&c.CreateColorBrushWithColor(rgba(255, 255, 255, 255))?)?;
        frame.Children()?.InsertAtTop(&bg)?;
    }
    let scaled = c.CreateContainerVisual()?;
    scaled.SetScale(V3 { X: scale, Y: scale, Z: 1.0 })?;
    scaled.SetSize(V2 { X: w as f32 / scale, Y: h as f32 / scale })?;
    frame.Children()?.InsertAtTop(&scaled)?;
    Ok((frame, scaled))
}

/// What the captured text looks like, for black text on white.
struct Ink {
    /// Pixels with some ink, but not full.
    partial: u32,
    /// Pixels with (nearly) full ink.
    full: u32,
    /// The largest difference between red and blue: color fringes of ClearType.
    fringe: u8,
    /// The largest difference between channels: color glyphs.
    chroma: u8,
    /// Inked pixels, cropped to their bounding box.
    crop: Vec<[u8; 4]>,
    bbox: (u32, u32, u32, u32),
}

fn ink(img: &Image) -> Ink {
    let (mut partial, mut full, mut fringe, mut chroma) = (0, 0, 0u8, 0u8);
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
    for y in 0..img.height {
        for x in 0..img.width {
            let [r, g, b, _] = img.pixel(x, y);
            let d = 255 - r.min(g).min(b);
            if d == 0 {
                continue;
            }
            if d >= 250 {
                full += 1;
            } else {
                partial += 1;
            }
            fringe = fringe.max(r.abs_diff(b));
            chroma = chroma.max(r.max(g).max(b) - r.min(g).min(b));
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
    }
    let mut crop = Vec::new();
    if x0 <= x1 {
        for y in y0..=y1 {
            for x in x0..=x1 {
                crop.push(img.pixel(x, y));
            }
        }
    }
    Ink { partial, full, fringe, chroma, crop, bbox: (x0, y0, x1, y1) }
}

/// Renders `layout` once into a surface and captures it at a DIP position.
fn capture_text(
    gfx: &Gfx,
    capturer: &Capturer,
    layout: &IDWriteTextLayout,
    d: &Draw,
    pos: f32,
    snap: Sampling,
    png: Option<String>,
) -> Result<Ink> {
    let size = physical_size(layout, d.scale)?;
    let surface = gfx.surface(size)?;
    draw(&surface, layout, d)?;
    let (frame, scaled) = scene(&gfx.compositor, size.0 + 40, size.1 + 30, d.scale, true)?;
    scaled.Children()?.InsertAtTop(&gfx.sprite(&surface, size, d.scale, pos, pos, snap)?)?;
    wait_commit(&gfx.compositor)?;
    let (img, _) = capturer.capture(&frame)?;
    if let Some(png) = png {
        img.save_png(&png);
    }
    Ok(ink(&img))
}

/// T1: is text drawn at physical resolution as sharp at fractional scales and positions as at
/// whole pixels, and what does `SnapToPixels` change?
fn t1_sharpness(gfx: &Gfx, capturer: &Capturer, out: &str) -> Result<()> {
    println!(
        "T1 sharpness: partial/full ink pixels, and whether the pixels equal the whole-pixel case"
    );
    let format = gfx.format(w!("Segoe UI"), 14.0)?;
    let layout = gfx.layout("Slint テキスト Agyj 0123", &format, f32::MAX)?;
    for scale in [1.0f32, 1.25, 1.5, 1.75, 2.0] {
        let d = Draw::plain(scale);
        // Whole physical pixels: the reference.
        let whole = (10.0 * scale).round() / scale;
        let reference = capture_text(
            gfx,
            capturer,
            &layout,
            &d,
            whole,
            Sampling::SnapNearest,
            Some(format!("{out}/t1-{scale}-ref.png")),
        )?;
        println!(
            "  scale {scale}: at {:.3} px  partial {:4} full {:4}",
            whole * scale,
            reference.partial,
            reference.full
        );
        for pos in [whole, 10.3f32, 10.5] {
            for snap in [Sampling::Free, Sampling::Snap, Sampling::SnapNearest] {
                let i = capture_text(
                    gfx,
                    capturer,
                    &layout,
                    &d,
                    pos,
                    snap,
                    Some(format!("{out}/t1-{scale}-{pos}-{snap:?}.png")),
                )?;
                println!(
                    "    at {:7.3} px {:11}  partial {:4} full {:4}  same as reference: {}  bbox {:?}",
                    pos * scale,
                    format!("{snap:?}"),
                    i.partial,
                    i.full,
                    i.crop == reference.crop,
                    i.bbox
                );
            }
        }
    }
    Ok(())
}

/// T2: antialiasing modes on a transparent surface, and ClearType on an opaque one.
fn t2_antialiasing(gfx: &Gfx, capturer: &Capturer, out: &str) -> Result<()> {
    println!("T2 antialiasing at scale 1.5: fringe = max |R-B| of black text on white");
    let format = gfx.format(w!("Segoe UI"), 14.0)?;
    let layout = gfx.layout("Slint テキスト Agyj 0123", &format, f32::MAX)?;
    let cases = [
        ("grayscale, transparent", D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE, CLEAR),
        ("cleartype, transparent", D2D1_TEXT_ANTIALIAS_MODE_CLEARTYPE, CLEAR),
        ("cleartype, opaque white", D2D1_TEXT_ANTIALIAS_MODE_CLEARTYPE, WHITE),
        ("default, transparent", D2D1_TEXT_ANTIALIAS_MODE_DEFAULT, CLEAR),
    ];
    for (i, (label, aa, background)) in cases.into_iter().enumerate() {
        let d = Draw { aa, background, ..Draw::plain(1.5) };
        let ink = capture_text(
            gfx,
            capturer,
            &layout,
            &d,
            10.0,
            Sampling::Snap,
            Some(format!("{out}/t2-{i}.png")),
        )?;
        println!(
            "  {label:24} partial {:4} full {:4} fringe {:3}",
            ink.partial, ink.full, ink.fringe
        );
    }
    Ok(())
}

/// T3: color glyphs and font fallback.
fn t3_glyphs(gfx: &Gfx, capturer: &Capturer, out: &str) -> Result<()> {
    println!("T3 color glyphs and fallback at scale 1.5");
    let format = gfx.format(w!("Segoe UI"), 20.0)?;
    for (i, (label, text, color_font)) in [
        ("emoji, color fonts on", "😀👍🏽", true),
        ("emoji, color fonts off", "😀👍🏽", false),
        ("japanese in Segoe UI (fallback)", "日本語の文章", true),
        ("arabic (fallback, RTL)", "مرحبا بالعالم", true),
    ]
    .into_iter()
    .enumerate()
    {
        let layout = gfx.layout(text, &format, f32::MAX)?;
        let mut m = DWRITE_TEXT_METRICS::default();
        unsafe { layout.GetMetrics(&mut m)? };
        let d = Draw { color_font, ..Draw::plain(1.5) };
        let ink = capture_text(
            gfx,
            capturer,
            &layout,
            &d,
            10.0,
            Sampling::Snap,
            Some(format!("{out}/t3-{i}.png")),
        )?;
        println!(
            "  {label:32} width {:6.2} DIP  ink pixels {:5}  chroma {:3}",
            m.width,
            ink.partial + ink.full,
            ink.chroma
        );
    }
    Ok(())
}

/// T4: what a window with many text items costs.
fn t4_cost(gfx: &Gfx) -> Result<()> {
    const N: usize = 1000;
    let scale = 1.5;
    println!("T4 cost of {N} text items at scale {scale}, one surface each, in a window");
    let host = Host::new(&gfx.compositor, "s4-text T4", 1200, 800)?;
    let scaled = gfx.compositor.CreateContainerVisual()?;
    scaled.SetScale(V3 { X: scale, Y: scale, Z: 1.0 })?;
    host.root.Children()?.InsertAtTop(&scaled)?;
    wait_commit(&gfx.compositor)?;
    let format = gfx.format(w!("Segoe UI"), 14.0)?;
    let d = Draw::plain(scale);
    let before = private_bytes();

    let t0 = Instant::now();
    let layouts: Vec<_> = (0..N)
        .map(|i| gfx.layout(&format!("Item {i} テキスト項目"), &format, f32::MAX))
        .collect::<Result<_>>()?;
    let t_layout = ms(t0);

    let t0 = Instant::now();
    let mut surfaces = Vec::with_capacity(N);
    for layout in &layouts {
        let size = physical_size(layout, scale)?;
        let surface = gfx.surface(size)?;
        draw(&surface, layout, &d)?;
        surfaces.push((surface, size));
    }
    let t_draw = ms(t0);

    let t0 = Instant::now();
    let mut group = None;
    for (i, (surface, size)) in surfaces.iter().enumerate() {
        if i % 100 == 0 {
            let g = gfx.compositor.CreateContainerVisual()?;
            g.SetOffset(V3 { X: (i / 100) as f32 * 120.0, Y: 0.0, Z: 0.0 })?;
            scaled.Children()?.InsertAtTop(&g)?;
            group = Some(g);
        }
        let sprite =
            gfx.sprite(surface, *size, scale, 0.0, (i % 100) as f32 * 5.0, Sampling::Snap)?;
        group.as_ref().unwrap().Children()?.InsertAtTop(&sprite)?;
    }
    let t_visuals = ms(t0);
    let t_commit = wait_commit(&gfx.compositor)?;
    let after = private_bytes();
    println!(
        "  layouts {t_layout:.1} ms, surfaces + draw {t_draw:.1} ms, visuals {t_visuals:.1} ms, commit {t_commit:.1} ms, memory +{:.1} MB",
        (after as f64 - before as f64) / 1e6
    );

    // Redraw every surface with new text of the same size.
    let t0 = Instant::now();
    for (i, (surface, _)) in surfaces.iter().enumerate() {
        draw(surface, &layouts[(i + 1) % N], &d)?;
    }
    let t_redraw = ms(t0);
    let t_commit = wait_commit(&gfx.compositor)?;
    println!("  redraw all {t_redraw:.1} ms, commit {t_commit:.1} ms");

    // Resize every surface, then redraw.
    let t0 = Instant::now();
    for (i, (surface, size)) in surfaces.iter().enumerate() {
        let interop: ICompositionDrawingSurfaceInterop = surface.cast()?;
        unsafe { interop.Resize(SIZE { cx: size.0 as i32 + 30, cy: size.1 as i32 })? };
        draw(surface, &layouts[i], &d)?;
    }
    let t_resize = ms(t0);
    let t_commit = wait_commit(&gfx.compositor)?;
    println!("  resize + redraw all {t_resize:.1} ms, commit {t_commit:.1} ms");

    // Replace every surface with a new one, as a resize without `Resize` would.
    let t0 = Instant::now();
    let mut fresh = Vec::with_capacity(N);
    for layout in &layouts {
        let size = physical_size(layout, scale)?;
        let surface = gfx.surface(size)?;
        draw(&surface, layout, &d)?;
        fresh.push(surface);
    }
    println!("  new surface + draw all {:.1} ms", ms(t0));

    // One surface redrawn many times: the cost of a text change.
    let (surface, _) = &surfaces[0];
    let mut times: Vec<f64> =
        (0..200).map(|i| draw(surface, &layouts[i], &d)).collect::<Result<_>>()?;
    times.sort_by(f64::total_cmp);
    println!("  one redraw: median {:.3} ms, p95 {:.3} ms", times[100], times[190]);
    pump_until(Duration::from_millis(300), || false);
    Ok(())
}

/// T5: the rendering device is replaced, as after a device loss.
fn t5_device_replaced(gfx: &Gfx, capturer: &Capturer) -> Result<()> {
    println!("T5 replacing the rendering device");
    let scale = 1.5;
    let format = gfx.format(w!("Segoe UI"), 14.0)?;
    let layout = gfx.layout("Device test テキスト", &format, f32::MAX)?;
    let size = physical_size(&layout, scale)?;
    let surface = gfx.surface(size)?;
    let d = Draw::plain(scale);
    draw(&surface, &layout, &d)?;
    let (frame, scaled) = scene(&gfx.compositor, size.0 + 40, size.1 + 30, scale, true)?;
    scaled.Children()?.InsertAtTop(&gfx.sprite(
        &surface,
        size,
        scale,
        10.0,
        10.0,
        Sampling::Snap,
    )?)?;
    wait_commit(&gfx.compositor)?;
    let before = ink(&capturer.capture(&frame)?.0);

    let replaced = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = replaced.clone();
    let token = gfx.graphics.RenderingDeviceReplaced(&TypedEventHandler::new(move |_, _| {
        flag.store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }))?;
    let interop: ICompositionGraphicsDeviceInterop = gfx.graphics.cast()?;
    let new_device = d2d_device(&gfx.d2d_factory)?;
    unsafe { interop.SetRenderingDevice(&new_device)? };
    let fired =
        pump_until(Duration::from_secs(2), || replaced.load(std::sync::atomic::Ordering::SeqCst));
    wait_commit(&gfx.compositor)?;
    let after = ink(&capturer.capture(&frame)?.0);
    draw(&surface, &layout, &d)?;
    wait_commit(&gfx.compositor)?;
    let redrawn = ink(&capturer.capture(&frame)?.0);
    gfx.graphics.RemoveRenderingDeviceReplaced(token)?;
    println!(
        "  ink before {}, event fired {fired}, ink after replacing {}, after redrawing {}",
        before.partial + before.full,
        after.partial + after.full,
        redrawn.partial + redrawn.full
    );
    Ok(())
}

/// T6: the measurements Slint's renderer interface asks for, from a DirectWrite layout.
fn t6_measurement(gfx: &Gfx) -> Result<()> {
    println!("T6 measurement");
    let format = gfx.format(w!("Segoe UI"), 14.0)?;
    let text = "Hello 日本語 😀 wraps here";
    let layout = gfx.layout(text, &format, 90.0)?;
    let mut m = DWRITE_TEXT_METRICS::default();
    unsafe { layout.GetMetrics(&mut m)? };
    println!(
        "  text {text:?} wrapped at 90 DIP: width {:.2}, height {:.2}, lines {}",
        m.width, m.height, m.lineCount
    );
    let mut lines = vec![DWRITE_LINE_METRICS::default(); m.lineCount as usize];
    let mut count = 0;
    unsafe { layout.GetLineMetrics(Some(&mut lines), &mut count)? };
    for l in &lines[..count as usize] {
        println!(
            "    line: {} units, baseline {:.2}, height {:.2}",
            l.length, l.baseline, l.height
        );
    }

    // Cursor rectangles, by UTF-16 index; Slint's API speaks UTF-8 byte offsets.
    let utf16: Vec<u16> = text.encode_utf16().collect();
    let to_byte = |unit: u32| String::from_utf16_lossy(&utf16[..unit as usize]).len();
    for (byte, ch) in text.char_indices().filter(|(_, c)| matches!(c, 'H' | '日' | '😀' | 'w')) {
        let unit = text[..byte].encode_utf16().count() as u32;
        let (mut x, mut y) = (0.0, 0.0);
        let mut hit = DWRITE_HIT_TEST_METRICS::default();
        unsafe { layout.HitTestTextPosition(unit, false, &mut x, &mut y, &mut hit)? };
        println!(
            "    cursor before {ch:?} (byte {byte}, unit {unit}): x {x:.2} y {y:.2} height {:.2}",
            hit.height
        );
    }

    // Byte offset at a point: the middle of the emoji, and past the end of a line.
    for (px, py) in [(0.0f32, 0.0f32), (200.0, 5.0)] {
        let (mut trailing, mut inside) = (BOOL(0), BOOL(0));
        let mut hit = DWRITE_HIT_TEST_METRICS::default();
        unsafe { layout.HitTestPoint(px, py, &mut trailing, &mut inside, &mut hit)? };
        let unit = hit.textPosition + if trailing.as_bool() { hit.length } else { 0 };
        println!(
            "    point ({px}, {py}) -> unit {unit} byte {} inside {}",
            to_byte(unit),
            inside.as_bool()
        );
    }
    let mut hit = DWRITE_HIT_TEST_METRICS::default();
    let emoji = text.find('😀').unwrap();
    let emoji_unit = text[..emoji].encode_utf16().count() as u32;
    let (mut x, mut y) = (0.0, 0.0);
    unsafe { layout.HitTestTextPosition(emoji_unit, false, &mut x, &mut y, &mut hit)? };
    let (mut trailing, mut inside) = (BOOL(0), BOOL(0));
    unsafe {
        layout.HitTestPoint(x + hit.width * 0.75, y + 2.0, &mut trailing, &mut inside, &mut hit)?
    };
    let unit = hit.textPosition + if trailing.as_bool() { hit.length } else { 0 };
    println!(
        "    right part of the emoji -> unit {unit} (length {}) byte {}: after the surrogate pair",
        hit.length,
        to_byte(unit)
    );

    // Font metrics for `font_metrics()`.
    unsafe {
        let mut collection = None;
        gfx.dwrite.GetSystemFontCollection(&mut collection, false)?;
        let collection = collection.unwrap();
        let (mut index, mut exists) = (0, BOOL(0));
        collection.FindFamilyName(w!("Segoe UI"), &mut index, &mut exists)?;
        let font = collection.GetFontFamily(index)?.GetFirstMatchingFont(
            DWRITE_FONT_WEIGHT_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            DWRITE_FONT_STYLE_NORMAL,
        )?;
        let mut fm = DWRITE_FONT_METRICS::default();
        font.GetMetrics(&mut fm);
        let k = 14.0 / fm.designUnitsPerEm as f32;
        println!(
            "  Segoe UI at 14 DIP: ascent {:.2} descent {:.2} line gap {:.2} cap height {:.2} x-height {:.2}",
            fm.ascent as f32 * k,
            fm.descent as f32 * k,
            fm.lineGap as f32 * k,
            fm.capHeight as f32 * k,
            fm.xHeight as f32 * k
        );
    }
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let out = args.get(1).cloned().unwrap_or_else(|| "out".into());
    std::fs::create_dir_all(&out).unwrap();
    let _queue = init_thread()?;
    let compositor = Compositor::new()?;
    let gfx = Gfx::new(&compositor)?;
    if args.get(2).map(String::as_str) == Some("ime") {
        return ime::run(gfx);
    }
    let capturer = Capturer::new()?;
    t1_sharpness(&gfx, &capturer, &out)?;
    if args.get(2).map(String::as_str) == Some("t1") {
        return Ok(());
    }
    t2_antialiasing(&gfx, &capturer, &out)?;
    t3_glyphs(&gfx, &capturer, &out)?;
    t4_cost(&gfx)?;
    t5_device_replaced(&gfx, &capturer)?;
    t6_measurement(&gfx)?;
    Ok(())
}

/// T7: a window that takes text from the input method through IMM32 and draws the composition
/// inline, with the candidate window placed at the caret.
mod ime {
    use super::*;
    use windows::UI::Composition::Desktop::DesktopWindowTarget;
    use windows::Win32::Graphics::Gdi::{GetStockObject, HBRUSH, WHITE_BRUSH};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::WinRT::Composition::ICompositorDesktopInterop;
    use windows::Win32::UI::HiDpi::GetDpiForWindow;
    use windows::Win32::UI::Input::Ime::*;
    use windows::Win32::UI::WindowsAndMessaging::*;

    struct State {
        gfx: Gfx,
        format: IDWriteTextFormat,
        hwnd: HWND,
        scale: f32,
        scaled: ContainerVisual,
        sprite: Option<SpriteVisual>,
        _target: DesktopWindowTarget,
        committed: String,
        composition: Vec<u16>,
        cursor: u32,
        caret: RECT,
    }

    thread_local! {
        static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
    }

    pub fn run(gfx: Gfx) -> Result<()> {
        println!("T7: type in the window with the Japanese IME. The composition is drawn inline,");
        println!("    underlined; the candidate window should open just below the caret.");
        unsafe {
            let instance = GetModuleHandleW(None)?;
            let class = WNDCLASSW {
                lpfnWndProc: Some(proc),
                hInstance: instance.into(),
                lpszClassName: w!("SlintForkS4Ime"),
                hCursor: LoadCursorW(None, IDC_IBEAM)?,
                hbrBackground: HBRUSH(GetStockObject(WHITE_BRUSH).0),
                ..Default::default()
            };
            RegisterClassW(&class);
            let hwnd = CreateWindowExW(
                WS_EX_NOREDIRECTIONBITMAP,
                w!("SlintForkS4Ime"),
                w!("s4-text: IME"),
                WS_OVERLAPPEDWINDOW,
                100,
                100,
                900,
                300,
                None,
                None,
                Some(instance.into()),
                None,
            )?;
            let c = gfx.compositor.clone();
            let interop: ICompositorDesktopInterop = c.cast()?;
            let target = interop.CreateDesktopWindowTarget(hwnd, false)?;
            let scale = GetDpiForWindow(hwnd) as f32 / 96.0;
            let (frame, scaled) = scene(&c, 4000, 2000, scale, true)?;
            target.SetRoot(&frame)?;
            let format = gfx.format(w!("Segoe UI"), 20.0)?;
            STATE.with(|s| {
                *s.borrow_mut() = Some(State {
                    gfx,
                    format,
                    hwnd,
                    scale,
                    scaled,
                    sprite: None,
                    _target: target,
                    committed: String::new(),
                    composition: Vec::new(),
                    cursor: 0,
                    caret: RECT::default(),
                })
            });
            render()?;
            let _ = ShowWindow(hwnd, SW_SHOW);
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        Ok(())
    }

    /// Draws the committed text, the underlined composition and the caret.
    fn render() -> Result<()> {
        STATE.with(|s| {
            let mut s = s.borrow_mut();
            let s = s.as_mut().unwrap();
            let committed: Vec<u16> = s.committed.encode_utf16().collect();
            let mut all = committed.clone();
            all.extend_from_slice(&s.composition);
            let text = String::from_utf16_lossy(&all);
            let layout = s.gfx.layout(&text, &s.format, 800.0)?;
            if !s.composition.is_empty() {
                unsafe {
                    layout.SetUnderline(
                        true,
                        DWRITE_TEXT_RANGE {
                            startPosition: committed.len() as u32,
                            length: s.composition.len() as u32,
                        },
                    )?;
                }
            }
            let caret_unit =
                committed.len() as u32 + if s.composition.is_empty() { 0 } else { s.cursor };
            let (mut x, mut y) = (0.0, 0.0);
            let mut hit = DWRITE_HIT_TEST_METRICS::default();
            unsafe { layout.HitTestTextPosition(caret_unit, false, &mut x, &mut y, &mut hit)? };
            let mut m = DWRITE_TEXT_METRICS::default();
            unsafe { layout.GetMetrics(&mut m)? };
            let size = (
                ((m.widthIncludingTrailingWhitespace + 2.0) * s.scale).ceil() as u32,
                (m.height.max(hit.height) * s.scale).ceil() as u32,
            );
            let surface = s.gfx.surface(size)?;
            let d = Draw::plain(s.scale);
            draw_with(&surface, &d, |dc, brush| unsafe {
                dc.DrawTextLayout(
                    Vector2 { X: 0.0, Y: 0.0 },
                    &layout,
                    brush,
                    D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT,
                );
                dc.FillRectangle(
                    &D2D_RECT_F { left: x, top: y, right: x + 1.0, bottom: y + hit.height },
                    brush,
                );
            })?;
            let (ox, oy) = (20.0, 20.0);
            let sprite = s.gfx.sprite(&surface, size, s.scale, ox, oy, Sampling::Snap)?;
            if let Some(old) = s.sprite.replace(sprite.clone()) {
                s.scaled.Children()?.Remove(&old)?;
            }
            s.scaled.Children()?.InsertAtTop(&sprite)?;
            let px = |v: f32| (v * s.scale).round() as i32;
            s.caret = RECT {
                left: px(ox + x),
                top: px(oy + y),
                right: px(ox + x) + 1,
                bottom: px(oy + y + hit.height),
            };
            place_ime_windows(s.hwnd, s.caret);
            Ok(())
        })
    }

    /// Puts the candidate window below the caret, never over it.
    fn place_ime_windows(hwnd: HWND, caret: RECT) {
        unsafe {
            let himc = ImmGetContext(hwnd);
            if himc.is_invalid() {
                return;
            }
            let composition = COMPOSITIONFORM {
                dwStyle: CFS_POINT,
                ptCurrentPos: POINT { x: caret.left, y: caret.top },
                rcArea: RECT::default(),
            };
            let _ = ImmSetCompositionWindow(himc, &composition);
            let candidate = CANDIDATEFORM {
                dwIndex: 0,
                dwStyle: CFS_EXCLUDE,
                ptCurrentPos: POINT { x: caret.left, y: caret.bottom },
                rcArea: caret,
            };
            let _ = ImmSetCandidateWindow(himc, &candidate);
            let _ = ImmReleaseContext(hwnd, himc);
        }
    }

    fn composition_string(himc: HIMC, kind: IME_COMPOSITION_STRING) -> Vec<u16> {
        unsafe {
            let bytes = ImmGetCompositionStringW(himc, kind, None, 0);
            if bytes <= 0 {
                return Vec::new();
            }
            let mut buffer = vec![0u16; bytes as usize / 2];
            ImmGetCompositionStringW(himc, kind, Some(buffer.as_mut_ptr().cast()), bytes as u32);
            buffer
        }
    }

    fn with_state(f: impl FnOnce(&mut State)) {
        STATE.with(|s| {
            if let Some(s) = s.borrow_mut().as_mut() {
                f(s)
            }
        });
    }

    extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        unsafe {
            match msg {
                WM_IME_SETCONTEXT => {
                    // The window draws the composition itself.
                    let lp = LPARAM(lp.0 & !(ISC_SHOWUICOMPOSITIONWINDOW as isize));
                    DefWindowProcW(hwnd, msg, wp, lp)
                }
                WM_IME_STARTCOMPOSITION => {
                    with_state(|s| place_ime_windows(hwnd, s.caret));
                    println!("WM_IME_STARTCOMPOSITION");
                    LRESULT(0)
                }
                WM_IME_COMPOSITION => {
                    let himc = ImmGetContext(hwnd);
                    let flags = lp.0 as u32;
                    let result = if flags & GCS_RESULTSTR.0 != 0 {
                        composition_string(himc, GCS_RESULTSTR)
                    } else {
                        Vec::new()
                    };
                    let composition = if flags & GCS_COMPSTR.0 != 0 {
                        Some(composition_string(himc, GCS_COMPSTR))
                    } else {
                        None
                    };
                    let cursor = if flags & GCS_CURSORPOS.0 != 0 {
                        Some(ImmGetCompositionStringW(himc, GCS_CURSORPOS, None, 0).max(0) as u32)
                    } else {
                        None
                    };
                    let _ = ImmReleaseContext(hwnd, himc);
                    println!(
                        "WM_IME_COMPOSITION flags {flags:#x} result {:?} composition {:?} cursor {cursor:?}",
                        String::from_utf16_lossy(&result),
                        composition.as_ref().map(|c| String::from_utf16_lossy(c))
                    );
                    with_state(|s| {
                        s.committed.push_str(&String::from_utf16_lossy(&result));
                        if !result.is_empty() {
                            s.composition.clear();
                        }
                        if let Some(c) = composition {
                            s.composition = c;
                        }
                        if let Some(c) = cursor {
                            s.cursor = c;
                        }
                    });
                    let _ = render();
                    LRESULT(0)
                }
                WM_IME_ENDCOMPOSITION => {
                    println!("WM_IME_ENDCOMPOSITION");
                    with_state(|s| s.composition.clear());
                    let _ = render();
                    LRESULT(0)
                }
                WM_CHAR => {
                    let c = char::from_u32(wp.0 as u32);
                    with_state(|s| match c {
                        Some('\u{8}') => {
                            s.committed.pop();
                        }
                        Some(c) if !c.is_control() => s.committed.push(c),
                        _ => {}
                    });
                    let _ = render();
                    LRESULT(0)
                }
                WM_DESTROY => {
                    PostQuitMessage(0);
                    LRESULT(0)
                }
                _ => DefWindowProcW(hwnd, msg, wp, lp),
            }
        }
    }
}
