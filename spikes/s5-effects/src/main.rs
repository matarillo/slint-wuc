// SPDX-License-Identifier: MIT OR Apache-2.0

//! Spike S5: composition effects without Win2D, images on surfaces, box shadows, backdrop blur
//! and the window's system backdrop.
//!
//! Usage: `s5-effects <out-dir> [test]` runs E1–E8 (or the one named, such as `e7`) and saves
//! the captures as PNGs in `<out-dir>`.

mod effect;

use std::time::Instant;

use spike_common::*;
use windows::Foundation::Size;
use windows::Graphics::Capture::GraphicsCaptureItem;
use windows::Graphics::DirectX::{DirectXAlphaMode, DirectXPixelFormat};
use windows::UI::Composition::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dwm::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::System::WinRT::Composition::{ICompositionDrawingSurfaceInterop, ICompositorInterop};
use windows::Win32::UI::Controls::MARGINS;
use windows::core::{HSTRING, Interface, Result};
use windows_collections::IIterable;
use windows_numerics::{Matrix3x2, Vector2};

struct Gfx {
    c: Compositor,
    graphics: CompositionGraphicsDevice,
}

impl Gfx {
    fn new(c: &Compositor) -> Result<Self> {
        unsafe {
            let factory: ID2D1Factory1 = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
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
            let device = factory.CreateDevice(&dxgi)?;
            let graphics = c.cast::<ICompositorInterop>()?.CreateGraphicsDevice(&device)?;
            Ok(Self { c: c.clone(), graphics })
        }
    }

    fn surface(&self, w: u32, h: u32) -> Result<CompositionDrawingSurface> {
        self.graphics.CreateDrawingSurface(
            Size { Width: w as f32, Height: h as f32 },
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            DirectXAlphaMode::Premultiplied,
        )
    }

    /// A surface holding `pixels` (premultiplied BGRA), uploaded with `CopyFromMemory`.
    fn image(&self, pixels: &Pixels) -> Result<CompositionDrawingSurface> {
        let s = self.surface(pixels.w, pixels.h)?;
        upload(&s, pixels, Upload::Copy)?;
        Ok(s)
    }

    fn sprite(&self, brush: &CompositionBrush, x: f32, y: f32, w: f32, h: f32) -> Result<SpriteVisual> {
        let v = self.c.CreateSpriteVisual()?;
        v.SetBrush(brush)?;
        v.SetOffset(V3 { X: x, Y: y, Z: 0.0 })?;
        v.SetSize(V2 { X: w, Y: h })?;
        Ok(v)
    }

    fn surface_brush(&self, s: &CompositionDrawingSurface) -> Result<CompositionSurfaceBrush> {
        self.c.CreateSurfaceBrushWithSurface(s)
    }

    fn color(&self, c: windows::UI::Color) -> Result<CompositionBrush> {
        self.c.CreateColorBrushWithColor(c)?.cast()
    }

    /// A container of `w`×`h` pixels, optionally on white, to capture.
    fn frame(&self, w: f32, h: f32, white: bool) -> Result<ContainerVisual> {
        let f = self.c.CreateContainerVisual()?;
        f.SetSize(V2 { X: w, Y: h })?;
        if white {
            let bg = self.sprite(&self.color(rgba(255, 255, 255, 255))?, 0.0, 0.0, w, h)?;
            f.Children()?.InsertAtTop(&bg)?;
        }
        Ok(f)
    }

    fn scaled(&self, parent: &ContainerVisual, scale: f32) -> Result<ContainerVisual> {
        let s = self.c.CreateContainerVisual()?;
        s.SetScale(V3 { X: scale, Y: scale, Z: 1.0 })?;
        let size = parent.Size()?;
        s.SetSize(V2 { X: size.X / scale, Y: size.Y / scale })?;
        parent.Children()?.InsertAtTop(&s)?;
        Ok(s)
    }

    fn effect_brush(
        &self,
        effect: &windows::Graphics::Effects::IGraphicsEffect,
        animatable: &[&str],
        sources: &[(&str, CompositionBrush)],
    ) -> Result<CompositionEffectBrush> {
        let names: Vec<HSTRING> = animatable.iter().map(|n| HSTRING::from(*n)).collect();
        let factory = self.c.CreateEffectFactoryWithProperties(effect, &IIterable::from(names))?;
        let brush = factory.CreateBrush()?;
        for (name, source) in sources {
            brush.SetSourceParameter(&HSTRING::from(*name), source)?;
        }
        Ok(brush)
    }
}

/// Premultiplied BGRA pixels.
struct Pixels {
    w: u32,
    h: u32,
    bgra: Vec<u8>,
}

impl Pixels {
    fn new(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> Self {
        let mut bgra = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                let [r, g, b, a] = f(x, y);
                let pm = |c: u8| (c as u32 * a as u32 / 255) as u8;
                bgra.extend_from_slice(&[pm(b), pm(g), pm(r), a]);
            }
        }
        Self { w, h, bgra }
    }
}

const RED: [u8; 4] = [255, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];
const NONE: [u8; 4] = [0, 0, 0, 0];

/// Red, green / blue, transparent quadrants.
fn quadrant(x: u32, y: u32, n: u32) -> [u8; 4] {
    match (x < n / 2, y < n / 2) {
        (true, true) => RED,
        (false, true) => GREEN,
        (true, false) => BLUE,
        (false, false) => NONE,
    }
}

fn quadrants(n: u32) -> Pixels {
    Pixels::new(n, n, |x, y| quadrant(x, y, n))
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Upload {
    /// `CreateBitmap` from memory, then `DrawBitmap`.
    Draw,
    /// `CopyFromMemory` into the surface's own bitmap.
    Copy,
}

fn upload(surface: &CompositionDrawingSurface, p: &Pixels, how: Upload) -> Result<f64> {
    let t0 = Instant::now();
    let interop: ICompositionDrawingSurfaceInterop = surface.cast()?;
    unsafe {
        let mut offset = POINT::default();
        let dc: ID2D1DeviceContext = interop.BeginDraw(None, &mut offset)?;
        match how {
            Upload::Draw => {
                dc.SetTransform(&Matrix3x2::translation(offset.x as f32, offset.y as f32));
                let props = D2D1_BITMAP_PROPERTIES1 {
                    pixelFormat: D2D1_PIXEL_FORMAT {
                        format: DXGI_FORMAT_B8G8R8A8_UNORM,
                        alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                    },
                    dpiX: 96.0,
                    dpiY: 96.0,
                    ..Default::default()
                };
                let bmp = dc.CreateBitmap(
                    D2D_SIZE_U { width: p.w, height: p.h },
                    Some(p.bgra.as_ptr() as _),
                    p.w * 4,
                    &props,
                )?;
                dc.Clear(Some(&D2D1_COLOR_F::default()));
                dc.DrawBitmap(&bmp, None, 1.0, D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR, None, None);
            }
            Upload::Copy => {
                let target: ID2D1Bitmap = dc.GetTarget()?.cast()?;
                let rect = D2D_RECT_U {
                    left: offset.x as u32,
                    top: offset.y as u32,
                    right: offset.x as u32 + p.w,
                    bottom: offset.y as u32 + p.h,
                };
                target.CopyFromMemory(Some(&rect), p.bgra.as_ptr() as _, p.w * 4)?;
            }
        }
        interop.EndDraw()?;
    }
    Ok(ms(t0))
}

/// Draws into the whole surface with a device context at 96 DPI, origin at the surface's corner.
fn draw(surface: &CompositionDrawingSurface, f: impl FnOnce(&ID2D1DeviceContext) -> Result<()>) -> Result<f64> {
    let t0 = Instant::now();
    let interop: ICompositionDrawingSurfaceInterop = surface.cast()?;
    unsafe {
        let mut offset = POINT::default();
        let dc: ID2D1DeviceContext = interop.BeginDraw(None, &mut offset)?;
        dc.SetTransform(&Matrix3x2::translation(offset.x as f32, offset.y as f32));
        dc.Clear(Some(&D2D1_COLOR_F::default()));
        let r = f(&dc);
        interop.EndDraw()?;
        r?;
    }
    Ok(ms(t0))
}

fn d2d_color([r, g, b, a]: [u8; 4]) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r: r as f32 / 255.0, g: g as f32 / 255.0, b: b as f32 / 255.0, a: a as f32 / 255.0 }
}

thread_local! {
    static COMPOSITOR: std::cell::OnceCell<Compositor> = const { std::cell::OnceCell::new() };
}

fn capture(cap: &Capturer, v: &impl Interface, out: &str, name: &str) -> Result<Image> {
    COMPOSITOR.with(|c| wait_commit(c.get().unwrap()))?;
    let (img, _) = cap.capture(v)?;
    img.save_png(&format!("{out}/{name}.png"));
    Ok(img)
}

/// The largest minus the smallest red value along row `y` between `x0` and `x1`.
fn contrast(img: &Image, x0: u32, x1: u32, y: u32) -> u8 {
    let reds = (x0..x1).map(|x| img.pixel(x, y)[0]);
    reds.clone().max().unwrap() - reds.min().unwrap()
}

/// One character per pixel of row `y`: R, G, B, `.` for transparent, `?` for anything else.
fn row_str(img: &Image, y: u32, x0: u32, x1: u32) -> String {
    (x0..x1)
        .map(|x| match img.pixel(x, y) {
            p if close(p, RED, 8) => 'R',
            p if close(p, GREEN, 8) => 'G',
            p if close(p, BLUE, 8) => 'B',
            p if p[3] < 8 => '.',
            _ => '?',
        })
        .collect()
}

fn close(a: [u8; 4], b: [u8; 4], tol: u8) -> bool {
    a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= tol)
}

// E1 -----------------------------------------------------------------------------------------

fn stripes(w: u32, h: u32, period: u32) -> Pixels {
    Pixels::new(w, h, |x, _| if (x / (period / 2)) % 2 == 0 { [0, 0, 0, 255] } else { [255, 255, 255, 255] })
}

fn e1_effect_interop(g: &Gfx, cap: &Capturer, out: &str) -> Result<()> {
    println!("E1 IGraphicsEffect implemented in Rust: Gaussian blur of a surface brush");
    let image = g.image(&stripes(100, 100, 10))?;
    let frame = g.frame(220.0, 100.0, true)?;
    let plain = g.sprite(&g.surface_brush(&image)?.cast()?, 0.0, 0.0, 100.0, 100.0)?;
    frame.Children()?.InsertAtTop(&plain)?;

    let effect = effect::gaussian_blur("Blur", 4.0, true, effect::param("image"));
    let names = IIterable::<HSTRING>::from(vec![HSTRING::from("Blur.BlurAmount")]);
    let t0 = Instant::now();
    let factory = g.c.CreateEffectFactoryWithProperties(&effect, &names)?;
    let first = ms(t0);
    let t0 = Instant::now();
    let again = effect::gaussian_blur("Blur", 4.0, true, effect::param("image"));
    let _factory2 = g.c.CreateEffectFactoryWithProperties(&again, &names)?;
    let second = ms(t0);
    let t0 = Instant::now();
    let brushes: Vec<_> = (0..100).map(|_| factory.CreateBrush()).collect::<Result<_>>()?;
    let per_brush = ms(t0) / 100.0;
    println!("  factory: first {first:.2} ms, same graph again {second:.2} ms; CreateBrush {per_brush:.3} ms");
    let brush = &brushes[0];
    brush.SetSourceParameter(&"image".into(), &g.surface_brush(&image)?)?;
    let blurred = g.sprite(&brush.cast()?, 120.0, 0.0, 100.0, 100.0)?;
    frame.Children()?.InsertAtTop(&blurred)?;

    let img = capture(cap, &frame, out, "e1-blur4")?;
    println!(
        "  contrast in row 50: plain {} / blurred (sigma 4) {}",
        contrast(&img, 10, 90, 50),
        contrast(&img, 130, 210, 50)
    );
    brush.Properties()?.InsertScalar(&"Blur.BlurAmount".into(), 1.0)?;
    let img = capture(cap, &frame, out, "e1-blur1")?;
    println!("  after Properties.InsertScalar(Blur.BlurAmount, 1): blurred {}", contrast(&img, 130, 210, 50));
    brush.Properties()?.InsertScalar(&"Blur.BlurAmount".into(), 0.0)?;
    let img = capture(cap, &frame, out, "e1-blur0")?;
    println!("  BlurAmount 0: blurred {}", contrast(&img, 130, 210, 50));
    Ok(())
}

// E2 -----------------------------------------------------------------------------------------

fn e2_images(g: &Gfx, cap: &Capturer, out: &str) -> Result<()> {
    println!("E2 images: upload cost, and placement with a brush transform");
    for n in [256u32, 1024, 2048, 4096] {
        let p = quadrants(n);
        for how in [Upload::Draw, Upload::Copy] {
            let t0 = Instant::now();
            let s = g.surface(n, n)?;
            let create = ms(t0);
            let mut times: Vec<f64> = (0..5).map(|_| upload(&s, &p, how)).collect::<Result<_>>()?;
            times.sort_by(f64::total_cmp);
            println!("  {n}x{n} {how:?}: create surface {create:.2} ms, upload median {:.2} ms", times[2]);
        }
    }
    // Copy: check the pixels arrived.
    let s = g.image(&quadrants(64))?;
    let frame = g.frame(64.0, 64.0, false)?;
    frame.Children()?.InsertAtTop(&g.sprite(&g.surface_brush(&s)?.cast()?, 0.0, 0.0, 64.0, 64.0)?)?;
    let img = capture(cap, &frame, out, "e2-upload")?;
    let ok = (0..64).all(|y| (0..64).all(|x| close(img.pixel(x, y), quadrant(x, y, 64), 0)));
    println!("  CopyFromMemory, 64x64 drawn 1:1: exact = {ok}");

    // A source clip of (16,16)-(48,48), one image pixel per physical pixel, at scale 1.5:
    // Stretch None and a transform; the sprite's bounds cut off the rest.
    let scale = 1.5;
    let frame = g.frame(80.0, 80.0, false)?;
    let scaled = g.scaled(&frame, scale)?;
    let brush = g.surface_brush(&s)?;
    brush.SetStretch(CompositionStretch::None)?;
    brush.SetHorizontalAlignmentRatio(0.0)?;
    brush.SetVerticalAlignmentRatio(0.0)?;
    brush.SetSnapToPixels(true)?;
    brush.SetTransformMatrix(Matrix3x2::translation(-16.0, -16.0) * Matrix3x2::scale(1.0 / scale, 1.0 / scale))?;
    scaled.Children()?.InsertAtTop(&g.sprite(&brush.cast()?, 10.0, 10.0, 32.0 / scale, 32.0 / scale)?)?;
    let img = capture(cap, &frame, out, "e2-clip")?;
    let mut wrong = 0;
    for y in 0..80 {
        for x in 0..80 {
            let inside = (15..47).contains(&x) && (15..47).contains(&y);
            let want = if inside { quadrant(x - 15 + 16, y - 15 + 16, 64) } else { NONE };
            wrong += !close(img.pixel(x, y), want, 2) as u32;
        }
    }
    println!("  source clip via TransformMatrix at scale 1.5: {wrong} of 6400 pixels differ");
    for y in [14, 15, 16, 30, 31, 32, 46, 47] {
        println!("    row {y:2} x 12..50: {}", row_str(&img, y, 12, 50));
    }

    // Magnified 4x with nearest-neighbor interpolation.
    let frame = g.frame(64.0, 64.0, false)?;
    let brush = g.surface_brush(&g.image(&quadrants(16))?)?;
    brush.SetStretch(CompositionStretch::Fill)?;
    brush.SetBitmapInterpolationMode(CompositionBitmapInterpolationMode::NearestNeighbor)?;
    frame.Children()?.InsertAtTop(&g.sprite(&brush.cast()?, 0.0, 0.0, 64.0, 64.0)?)?;
    let img = capture(cap, &frame, out, "e2-nearest")?;
    let wrong = (0..64).flat_map(|y| (0..64).map(move |x| (x, y)))
        .filter(|&(x, y)| !close(img.pixel(x, y), quadrant(x / 4, y / 4, 16), 0))
        .count();
    println!("  16x16 magnified 4x, NearestNeighbor: {wrong} pixels differ");
    Ok(())
}

// E3 -----------------------------------------------------------------------------------------

fn e3_tiling(g: &Gfx, cap: &Capturer, out: &str) -> Result<()> {
    println!("E3 tiling with the Border effect (wrap)");
    let tile = g.image(&quadrants(16))?;
    let scale = 1.5;
    for (name, wrap_y) in [("both", true), ("x only", false)] {
        let frame = g.frame(120.0, 60.0, false)?;
        let scaled = g.scaled(&frame, scale)?;
        let brush = g.surface_brush(&tile)?;
        brush.SetStretch(CompositionStretch::None)?;
        brush.SetHorizontalAlignmentRatio(0.0)?;
        brush.SetVerticalAlignmentRatio(0.0)?;
        brush.SetSnapToPixels(true)?;
        // One image pixel per physical pixel, and the tiling starts 4 pixels into the image.
        brush.SetTransformMatrix(Matrix3x2::translation(-4.0, -4.0) * Matrix3x2::scale(1.0 / scale, 1.0 / scale))?;
        let effect = effect::border("Tile", true, wrap_y, effect::param("image"));
        let eb = g.effect_brush(&effect, &[], &[("image", brush.cast()?)])?;
        // 96 x 48 physical pixels at (12, 6); with y not wrapped, one tile row high.
        let h = if wrap_y { 48.0 } else { 12.0 };
        scaled.Children()?.InsertAtTop(&g.sprite(&eb.cast()?, 8.0, 4.0, 96.0 / scale, h / scale)?)?;
        let img = capture(cap, &frame, out, &format!("e3-tile-{}", name.replace(' ', "-")))?;
        let mut wrong = 0;
        for y in 0..60u32 {
            for x in 0..120u32 {
                let inside = (12..108).contains(&x) && (6..6 + h as u32).contains(&y);
                let want = if inside { quadrant((x - 12 + 4) % 16, (y - 6 + 4) % 16, 16) } else { NONE };
                wrong += !close(img.pixel(x, y), want, 2) as u32;
            }
        }
        println!("  wrap {name}: {wrong} of 7200 pixels differ");
        for y in [5, 6, 7, 13, 14, 18, 22] {
            println!("    row {y:2} x 8..60: {}", row_str(&img, y, 8, 60));
        }
    }
    Ok(())
}

// E4 -----------------------------------------------------------------------------------------

fn e4_colorize(g: &Gfx, cap: &Capturer, out: &str) -> Result<()> {
    println!("E4 colorize: Composite (source-in) of a brush over the image's alpha");
    let image = g.image(&quadrants(64))?;
    let orange = rgba(255, 128, 0, 255);

    let check = |name: &str, source: CompositionBrush, tiled: bool| -> Result<()> {
        let image_brush = g.surface_brush(&image)?;
        let input = if tiled {
            image_brush.SetStretch(CompositionStretch::None)?;
            image_brush.SetHorizontalAlignmentRatio(0.0)?;
            image_brush.SetVerticalAlignmentRatio(0.0)?;
            image_brush.SetTransformMatrix(Matrix3x2::scale(0.5, 0.5))?;
            effect::source(&effect::border("Tile", true, true, effect::param("image")))
        } else {
            effect::param("image")
        };
        let effect = effect::composite("Colorize", D2D1_COMPOSITE_MODE_SOURCE_IN, input, effect::param("color"));
        let eb = g.effect_brush(&effect, &[], &[("image", image_brush.cast()?), ("color", source)])?;
        let frame = g.frame(64.0, 64.0, false)?;
        frame.Children()?.InsertAtTop(&g.sprite(&eb.cast()?, 0.0, 0.0, 64.0, 64.0)?)?;
        let img = capture(cap, &frame, out, &format!("e4-{name}"))?;
        let samples = [(8, 8), (56, 8), (8, 56), (56, 56), (20, 20), (44, 20)];
        let s: Vec<String> = samples.iter().map(|&(x, y)| format!("{:?}", img.pixel(x, y))).collect();
        println!("  {name}: {}", s.join(" "));
        Ok(())
    };
    check("color", g.color(orange)?, false)?;
    let gradient = g.c.CreateLinearGradientBrush()?;
    gradient.ColorStops()?.Append(&g.c.CreateColorGradientStopWithOffsetAndColor(0.0, rgba(0, 0, 0, 255))?)?;
    gradient.ColorStops()?.Append(&g.c.CreateColorGradientStopWithOffsetAndColor(1.0, rgba(255, 255, 255, 255))?)?;
    gradient.SetStartPoint(Vector2 { X: 0.0, Y: 0.0 })?;
    gradient.SetEndPoint(Vector2 { X: 1.0, Y: 0.0 })?;
    check("gradient", gradient.cast()?, false)?;
    check("tiled-color", g.color(orange)?, true)?;
    println!("  (expected: orange / gradient where the image is opaque, transparent in the bottom right)");

    // Colorize over a nine-grid brush.
    let nine = g.c.CreateNineGridBrush()?;
    let source = g.surface_brush(&image)?;
    source.SetStretch(CompositionStretch::Fill)?;
    nine.SetSource(&source)?;
    nine.SetInsets(16.0)?;
    let effect = effect::composite("Colorize", D2D1_COMPOSITE_MODE_SOURCE_IN, effect::param("image"), effect::param("color"));
    match g.effect_brush(&effect, &[], &[("image", nine.cast()?), ("color", g.color(orange)?)]) {
        Ok(eb) => {
            let frame = g.frame(128.0, 64.0, false)?;
            frame.Children()?.InsertAtTop(&g.sprite(&eb.cast()?, 0.0, 0.0, 128.0, 64.0)?)?;
            let img = capture(cap, &frame, out, "e4-ninegrid")?;
            println!(
                "  nine-grid source: (8,8) {:?} (120,56) {:?} (64,8) {:?}",
                img.pixel(8, 8),
                img.pixel(120, 56),
                img.pixel(64, 8)
            );
        }
        Err(e) => println!("  nine-grid source: {e}"),
    }
    Ok(())
}

// E5 -----------------------------------------------------------------------------------------

fn e5_nine_grid(g: &Gfx, cap: &Capturer, out: &str) -> Result<()> {
    println!("E5 nine-slice with CompositionNineGridBrush at scale 1.5");
    // 48x48: a 16 px red frame around green.
    let p = Pixels::new(48, 48, |x, y| {
        if (16..32).contains(&x) && (16..32).contains(&y) { GREEN } else { RED }
    });
    let scale = 1.5;
    let frame = g.frame(240.0, 120.0, false)?;
    let scaled = g.scaled(&frame, scale)?;
    let nine = g.c.CreateNineGridBrush()?;
    let source = g.surface_brush(&g.image(&p)?)?;
    println!("  surface brush default stretch: {:?}", source.Stretch()?);
    source.SetStretch(CompositionStretch::Fill)?;
    nine.SetSource(&source)?;
    nine.SetInsets(16.0)?;
    // Edges keep one image pixel per physical pixel.
    nine.SetInsetScales(1.0 / scale)?;
    scaled.Children()?.InsertAtTop(&g.sprite(&nine.cast()?, 0.0, 0.0, 240.0 / scale, 120.0 / scale)?)?;
    let img = capture(cap, &frame, out, "e5-ninegrid")?;
    let row: Vec<u8> = (0..24).map(|x| img.pixel(x, 60)[1]).collect();
    println!("  green along row 60, x 0..24: {row:?} (16 red pixels expected, then green)");
    let col: Vec<u8> = (100..120).map(|y| img.pixel(120, y)[1]).collect();
    println!("  green along column 120, y 100..120: {col:?} (green until y 104)");
    Ok(())
}

// E6 -----------------------------------------------------------------------------------------

/// A rounded rectangle and its blurred shadow (sigma = blur / 2, as upstream), baked by
/// Direct2D into a surface padded by the blur on each side.
fn bake_shadow(g: &Gfx, w: f32, h: f32, radius: f32, blur: f32, color: [u8; 4]) -> Result<(CompositionDrawingSurface, f64)> {
    let (sw, sh) = ((w + 2.0 * blur).ceil() as u32, (h + 2.0 * blur).ceil() as u32);
    let t0 = Instant::now();
    let s = g.surface(sw, sh)?;
    draw(&s, |dc| unsafe {
        let target = dc.GetTarget()?;
        let transform = {
            let mut m = Matrix3x2::default();
            dc.GetTransform(&mut m);
            m
        };
        let list = dc.CreateCommandList()?;
        dc.SetTarget(&list);
        dc.SetTransform(&Matrix3x2::identity());
        let brush = dc.CreateSolidColorBrush(&d2d_color([0, 0, 0, 255]), None)?;
        let rr = D2D1_ROUNDED_RECT {
            rect: D2D_RECT_F { left: blur, top: blur, right: blur + w, bottom: blur + h },
            radiusX: radius,
            radiusY: radius,
        };
        dc.FillRoundedRectangle(&rr, &brush);
        list.Close()?;
        dc.SetTarget(&target);
        dc.SetTransform(&transform);
        let shadow = dc.CreateEffect(&CLSID_D2D1Shadow)?;
        shadow.SetInput(0, &list, true);
        let sigma = blur / 2.0;
        shadow.SetValue(D2D1_SHADOW_PROP_BLUR_STANDARD_DEVIATION.0 as u32, D2D1_PROPERTY_TYPE_FLOAT, &sigma.to_le_bytes())?;
        let c = d2d_color(color);
        let v: Vec<u8> = [c.r, c.g, c.b, c.a].iter().flat_map(|f| f.to_le_bytes()).collect();
        shadow.SetValue(D2D1_SHADOW_PROP_COLOR.0 as u32, D2D1_PROPERTY_TYPE_VECTOR4, &v)?;
        dc.DrawImage(&shadow.GetOutput()?, None, None, D2D1_INTERPOLATION_MODE_LINEAR, D2D1_COMPOSITE_MODE_SOURCE_OVER);
        Ok(())
    })?;
    Ok((s, ms(t0)))
}

/// Alpha along row `y` from `x0`, every pixel, as captured over a transparent frame.
fn alphas(img: &Image, x0: u32, n: u32, y: u32) -> Vec<u8> {
    (x0..x0 + n).map(|x| img.pixel(x, y)[3]).collect()
}

/// The distance from `x_edge` at which alpha first falls under `fraction` of the alpha at the edge's inside.
fn falloff(img: &Image, x_edge: u32, y: u32, fraction: f32) -> u32 {
    let inside = img.pixel(x_edge - 30, y)[3] as f32;
    (x_edge..img.width).find(|&x| (img.pixel(x, y)[3] as f32) < inside * fraction).unwrap_or(img.width) - x_edge
}

fn e6_box_shadow(g: &Gfx, cap: &Capturer, out: &str) -> Result<()> {
    println!("E6 box shadow: Direct2D bake (Shadow effect, sigma = blur/2) vs Composition DropShadow");
    let (w, h, radius, blur) = (120.0f32, 80.0f32, 12.0f32, 16.0f32);
    let black = [0, 0, 0, 255];
    let frame = g.frame(400.0, 140.0, false)?;
    // Baked: the shape's left edge at x = 30 + blur.
    let (baked, t) = bake_shadow(g, w, h, radius, blur, black)?;
    let size = baked.Size()?;
    frame.Children()?.InsertAtTop(&g.sprite(&g.surface_brush(&baked)?.cast()?, 30.0, 30.0 - blur, size.Width, size.Height)?)?;
    println!("  bake {w}x{h} blur {blur}: {t:.2} ms");

    // DropShadow, BlurRadius = blur, masked by the rounded rectangle.
    let (shape, _) = bake_shadow(g, w, h, radius, 0.0, black)?;
    let mask = g.surface_brush(&shape)?;
    let v = g.sprite(&mask.cast()?, 230.0, 30.0, w, h)?;
    let shadow = g.c.CreateDropShadow()?;
    shadow.SetBlurRadius(blur)?;
    shadow.SetColor(rgba(0, 0, 0, 255))?;
    shadow.SetMask(&g.surface_brush(&shape)?)?;
    v.SetShadow(&shadow)?;
    frame.Children()?.InsertAtTop(&v)?;
    let img = capture(cap, &frame, out, "e6-shadow")?;
    let y = 70;
    // Right edges: baked shape spans 30+blur .. 30+blur+w; the masked sprite 230 .. 230+w.
    let baked_edge = (30.0 + blur + w) as u32;
    let drop_edge = (230.0 + w) as u32;
    println!("  baked alpha from the right edge: {:?}", alphas(&img, baked_edge - 2, 22, y));
    println!("  drop  alpha from the right edge: {:?}", alphas(&img, drop_edge - 2, 22, y));
    for f in [0.5f32, 0.16, 0.02] {
        println!(
            "  distance to {:>3}% of the inside alpha: baked {} px, DropShadow {} px",
            (f * 100.0) as u32,
            falloff(&img, baked_edge, y, f),
            falloff(&img, drop_edge, y, f)
        );
    }

    let scale = 1.5;
    for (w, h, blur) in [(100.0f32, 50.0f32, 8.0f32), (400.0, 300.0, 24.0), (1200.0, 800.0, 32.0)] {
        let mut ts: Vec<f64> = (0..5)
            .map(|_| bake_shadow(g, w * scale, h * scale, 8.0 * scale, blur * scale, black).map(|(_, t)| t))
            .collect::<Result<_>>()?;
        ts.sort_by(f64::total_cmp);
        println!("  bake {w}x{h} blur {blur} at scale {scale}: median {:.2} ms (create + draw)", ts[2]);
    }
    Ok(())
}

// E7 -----------------------------------------------------------------------------------------

/// Stripes on the frame, and over the middle a sprite whose brush blurs the backdrop.
fn backdrop_scene(
    g: &Gfx,
    parent: &ContainerVisual,
    sigma: f32,
    rounded: bool,
) -> Result<(SpriteVisual, CompositionEffectBrush)> {
    let size = parent.Size()?;
    let bg = g.image(&stripes(size.X as u32, size.Y as u32, 8))?;
    parent.Children()?.InsertAtTop(&g.sprite(&g.surface_brush(&bg)?.cast()?, 0.0, 0.0, size.X, size.Y)?)?;
    let effect = effect::gaussian_blur("Blur", sigma, true, effect::param("backdrop"));
    let eb = g.effect_brush(&effect, &["Blur.BlurAmount"], &[("backdrop", g.c.CreateBackdropBrush()?.cast()?)])?;
    let (w, h) = (size.X / 2.0, size.Y * 2.0 / 3.0);
    let v = g.sprite(&eb.cast()?, size.X / 4.0, size.Y / 6.0, w, h)?;
    if rounded {
        let geometry = g.c.CreateRoundedRectangleGeometry()?;
        geometry.SetSize(V2 { X: w, Y: h })?;
        geometry.SetCornerRadius(V2 { X: 20.0, Y: 20.0 })?;
        v.SetClip(&g.c.CreateGeometricClipWithGeometry(&geometry)?)?;
    }
    parent.Children()?.InsertAtTop(&v)?;
    Ok((v, eb))
}

fn e7_backdrop(g: &Gfx, cap: &Capturer, out: &str) -> Result<()> {
    println!("E7 BackdropBrush blurred by GaussianBlur, frame 240x120: stripes of 8 px, blur over x 60..180, y 20..100");
    let frame = g.frame(240.0, 120.0, false)?;
    let (_, eb) = backdrop_scene(g, &frame, 4.0, true)?;
    let img = capture(cap, &frame, out, "e7-backdrop")?;
    println!(
        "  not hosted: contrast outside {} / inside {}; corner (62,22) {:?}, inside corner (66,26) {:?}",
        contrast(&img, 10, 50, 60),
        contrast(&img, 80, 160, 60),
        img.pixel(62, 22),
        img.pixel(66, 26)
    );
    eb.Properties()?.InsertScalar(&"Blur.BlurAmount".into(), 1.0)?;
    let img = capture(cap, &frame, out, "e7-backdrop-sigma1")?;
    println!("  BlurAmount 1: inside {}", contrast(&img, 80, 160, 60));

    // The same in a window.
    let host = Host::new(&g.c, "s5 E7", 240, 120)?;
    let hosted = g.frame(240.0, 120.0, false)?;
    host.root.Children()?.InsertAtTop(&hosted)?;
    backdrop_scene(g, &hosted, 4.0, true)?;
    let img = capture(cap, &hosted, out, "e7-backdrop-hosted")?;
    println!("  hosted: contrast outside {} / inside {}", contrast(&img, 10, 50, 60), contrast(&img, 80, 160, 60));
    drop(host);

    // Is the blur amount in the visual's units? Sigma 2 under scale 2 vs sigma 4 unscaled.
    let profile = |img: &Image, y: u32| -> Vec<u8> { (100..124).map(|x| img.pixel(x, y)[0]).collect() };
    let frame = g.frame(240.0, 120.0, false)?;
    let scaled = g.scaled(&frame, 2.0)?;
    backdrop_scene(g, &scaled, 2.0, false)?;
    let a = capture(cap, &frame, out, "e7-scale2-sigma2")?;
    let frame = g.frame(240.0, 120.0, false)?;
    let bg = g.image(&stripes(240, 120, 16))?;
    frame.Children()?.InsertAtTop(&g.sprite(&g.surface_brush(&bg)?.cast()?, 0.0, 0.0, 240.0, 120.0)?)?;
    for sigma in [2.0f32, 4.0] {
        let effect = effect::gaussian_blur("Blur", sigma, true, effect::param("backdrop"));
        let eb = g.effect_brush(&effect, &[], &[("backdrop", g.c.CreateBackdropBrush()?.cast()?)])?;
        let v = g.sprite(&eb.cast()?, 60.0, 20.0, 120.0, 80.0)?;
        frame.Children()?.InsertAtTop(&v)?;
        let b = capture(cap, &frame, out, &format!("e7-scale1-sigma{sigma}"))?;
        frame.Children()?.Remove(&v)?;
        let diff: u32 = profile(&a, 60).iter().zip(profile(&b, 60)).map(|(p, q)| p.abs_diff(q) as u32).sum();
        println!("  scale 2 sigma 2 vs scale 1 sigma {sigma} (16 px stripes): sum of differences {diff}");
    }
    println!("  scale 2 sigma 2: {:?}", profile(&a, 60));

    // Under a container with opacity 0.5.
    let frame = g.frame(240.0, 120.0, false)?;
    let bg = g.image(&stripes(240, 120, 8))?;
    frame.Children()?.InsertAtTop(&g.sprite(&g.surface_brush(&bg)?.cast()?, 0.0, 0.0, 240.0, 120.0)?)?;
    let group = g.c.CreateContainerVisual()?;
    group.SetSize(V2 { X: 240.0, Y: 120.0 })?;
    group.SetOpacity(0.5)?;
    frame.Children()?.InsertAtTop(&group)?;
    let effect = effect::gaussian_blur("Blur", 4.0, true, effect::param("backdrop"));
    let eb = g.effect_brush(&effect, &[], &[("backdrop", g.c.CreateBackdropBrush()?.cast()?)])?;
    group.Children()?.InsertAtTop(&g.sprite(&eb.cast()?, 60.0, 20.0, 120.0, 80.0)?)?;
    let img = capture(cap, &frame, out, "e7-opacity")?;
    println!(
        "  under opacity 0.5: contrast inside {} (half blurred, half sharp: ~128 expected); pixels {:?} {:?}",
        contrast(&img, 80, 160, 60),
        img.pixel(100, 60),
        img.pixel(104, 60)
    );
    Ok(())
}

// E8 -----------------------------------------------------------------------------------------

fn capture_window(cap: &Capturer, hwnd: HWND, out: &str, name: &str) -> Result<Image> {
    use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
    let interop = windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
    let item: GraphicsCaptureItem = unsafe { interop.CreateForWindow(hwnd)? };
    let img = cap.capture_item(&item)?;
    img.save_png(&format!("{out}/{name}.png"));
    Ok(img)
}

fn e8_mica(g: &Gfx, cap: &Capturer, out: &str) -> Result<()> {
    println!("E8 system backdrop (Mica) behind a transparent composition root");
    for (name, backdrop, extend) in [
        ("none", false, false),
        ("mica", true, false),
        ("mica-extended", true, true),
    ] {
        let host = Host::new(&g.c, "s5 E8", 200, 120)?;
        unsafe {
            if backdrop {
                let v = DWMSBT_MAINWINDOW;
                DwmSetWindowAttribute(host.hwnd, DWMWA_SYSTEMBACKDROP_TYPE, &v as *const _ as _, 4)?;
            }
            if extend {
                let m = MARGINS { cxLeftWidth: -1, cxRightWidth: -1, cyTopHeight: -1, cyBottomHeight: -1 };
                DwmExtendFrameIntoClientArea(host.hwnd, &m)?;
            }
        }
        // A small opaque square, so the client area is found in the capture.
        host.root.Children()?.InsertAtTop(&g.sprite(&g.color(rgba(255, 0, 0, 255))?, 0.0, 0.0, 10.0, 10.0)?)?;
        let img = capture_window(cap, host.hwnd, out, &format!("e8-{name}"))?;
        let (cx, cy) = (img.width / 2, img.height / 2 + 10);
        println!(
            "  {name}: window capture {}x{}, client center {:?}, title bar {:?}",
            img.width,
            img.height,
            img.pixel(cx, cy),
            img.pixel(cx, 10)
        );
    }
    Ok(())
}

// E9 -----------------------------------------------------------------------------------------

/// A conic gradient from red at 12 o'clock clockwise to blue, computed on the CPU.
fn conic(n: u32) -> Pixels {
    Pixels::new(n, n, |x, y| {
        let (dx, dy) = (x as f32 + 0.5 - n as f32 / 2.0, y as f32 + 0.5 - n as f32 / 2.0);
        let t = (dx.atan2(-dy) / std::f32::consts::TAU).rem_euclid(1.0);
        [(255.0 * (1.0 - t)) as u8, 0, (255.0 * t) as u8, 255]
    })
}

fn e9_conic(g: &Gfx, cap: &Capturer, out: &str) -> Result<()> {
    println!("E9 a conic gradient computed on the CPU: as the fill of a rounded shape, and clipped sprite");
    let s = g.image(&conic(64))?;
    let frame = g.frame(140.0, 64.0, false)?;
    // A shape whose fill is a surface brush.
    let shape_visual = g.c.CreateShapeVisual()?;
    shape_visual.SetSize(V2 { X: 64.0, Y: 64.0 })?;
    let geometry = g.c.CreateRoundedRectangleGeometry()?;
    geometry.SetSize(V2 { X: 64.0, Y: 64.0 })?;
    geometry.SetCornerRadius(V2 { X: 16.0, Y: 16.0 })?;
    let shape = g.c.CreateSpriteShapeWithGeometry(&geometry)?;
    let brush = g.surface_brush(&s)?;
    brush.SetStretch(CompositionStretch::Fill)?;
    match shape.SetFillBrush(&brush) {
        Ok(()) => println!("  SpriteShape.FillBrush = surface brush: accepted"),
        Err(e) => println!("  SpriteShape.FillBrush = surface brush: {e}"),
    }
    shape_visual.Shapes()?.Append(&shape)?;
    frame.Children()?.InsertAtTop(&shape_visual)?;
    // A sprite clipped by the same rounded rectangle.
    let sprite = g.sprite(&brush.cast()?, 76.0, 0.0, 64.0, 64.0)?;
    let geometry = g.c.CreateRoundedRectangleGeometry()?;
    geometry.SetSize(V2 { X: 64.0, Y: 64.0 })?;
    geometry.SetCornerRadius(V2 { X: 16.0, Y: 16.0 })?;
    sprite.SetClip(&g.c.CreateGeometricClipWithGeometry(&geometry)?)?;
    frame.Children()?.InsertAtTop(&sprite)?;
    let img = capture(cap, &frame, out, "e9-conic")?;
    for (name, x0) in [("shape", 0), ("clipped sprite", 76)] {
        println!(
            "  {name}: corner {:?}, top {:?}, right {:?}, left {:?}",
            img.pixel(x0 + 1, 1),
            img.pixel(x0 + 32, 4),
            img.pixel(x0 + 60, 32),
            img.pixel(x0 + 4, 32)
        );
    }
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let out = args.get(1).cloned().unwrap_or_else(|| "out".into());
    let only = args.get(2).cloned();
    std::fs::create_dir_all(&out).unwrap();
    let _queue = init_thread()?;
    let c = Compositor::new()?;
    COMPOSITOR.with(|cell| cell.set(c.clone()).unwrap());
    let g = Gfx::new(&c)?;
    let cap = Capturer::new()?;
    let tests: [(&str, fn(&Gfx, &Capturer, &str) -> Result<()>); 9] = [
        ("e1", e1_effect_interop),
        ("e2", e2_images),
        ("e3", e3_tiling),
        ("e4", e4_colorize),
        ("e5", e5_nine_grid),
        ("e6", e6_box_shadow),
        ("e7", e7_backdrop),
        ("e8", e8_mica),
        ("e9", e9_conic),
    ];
    for (name, test) in tests {
        if only.as_deref().is_none_or(|o| o == name) {
            if let Err(e) = test(&g, &cap, &out) {
                println!("  {name} failed: {e}");
            }
        }
    }
    Ok(())
}
