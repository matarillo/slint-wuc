// Copyright © Kentaro Inomata
// SPDX-License-Identifier: MIT-0

//! Spike S2: which composition primitives implement Slint's rectangle, border, clip, opacity,
//! and gradients; how fractional scale factors render; and what 10k–50k visuals cost.
//!
//! Usage: `s2-primitives <out-dir> [perf]`

use std::time::Instant;

use spike_common::*;
use windows::Graphics::IGeometrySource2D;
use windows::UI::Composition::*;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::System::WinRT::Graphics::Direct2D::{IGeometrySource2DInterop, IGeometrySource2DInterop_Impl};
use windows::core::{Interface, Result, implement};

const WHITE: [u8; 4] = [255, 255, 255, 255];

fn frame(c: &Compositor, w: f32, h: f32) -> Result<ContainerVisual> {
    let f = c.CreateContainerVisual()?;
    f.SetSize(V2 { X: w, Y: h })?;
    let bg = c.CreateSpriteVisual()?;
    bg.SetRelativeSizeAdjustment(V2 { X: 1.0, Y: 1.0 })?;
    bg.SetBrush(&c.CreateColorBrushWithColor(rgba(255, 255, 255, 255))?)?;
    f.Children()?.InsertAtTop(&bg)?;
    Ok(f)
}

fn sprite(c: &Compositor, x: f32, y: f32, w: f32, h: f32, color: [u8; 4]) -> Result<SpriteVisual> {
    let s = c.CreateSpriteVisual()?;
    s.SetOffset(V3 { X: x, Y: y, Z: 0.0 })?;
    s.SetSize(V2 { X: w, Y: h })?;
    s.SetBrush(&c.CreateColorBrushWithColor(rgba(color[0], color[1], color[2], color[3]))?)?;
    Ok(s)
}

fn px(img: &Image, label: &str, x: u32, y: u32) -> [u8; 4] {
    let p = img.pixel(x, y);
    println!("  {label} ({x},{y}) = {p:?}");
    p
}

fn close(a: [u8; 4], b: [u8; 4], tol: u8) -> bool {
    a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= tol)
}

struct Ctx {
    c: Compositor,
    cap: Capturer,
    out: String,
}

impl Ctx {
    fn shot(&self, v: &impl Interface, name: &str) -> Result<Image> {
        let (img, _) = self.cap.capture(v)?;
        img.save_png(&format!("{}/s2-{name}.png", self.out));
        Ok(img)
    }
}

/// Does `Visual.Opacity` on a container blend its subtree as one layer?
fn group_opacity(x: &Ctx) -> Result<()> {
    println!("T1 group opacity: container opacity 0.5, red (0..60) under blue (40..100)");
    let f = frame(&x.c, 120.0, 40.0)?;
    let group = x.c.CreateContainerVisual()?;
    group.SetSize(V2 { X: 120.0, Y: 40.0 })?;
    group.SetOpacity(0.5)?;
    group.Children()?.InsertAtTop(&sprite(&x.c, 0.0, 0.0, 60.0, 40.0, [255, 0, 0, 255])?)?;
    group.Children()?.InsertAtTop(&sprite(&x.c, 40.0, 0.0, 60.0, 40.0, [0, 0, 255, 255])?)?;
    f.Children()?.InsertAtTop(&group)?;
    let img = x.shot(&f, "t1-group-opacity")?;
    let p = px(&img, "overlap", 50, 20);
    let verdict = if close(p, [127, 127, 255, 255], 3) {
        "group opacity (subtree blended as one layer)"
    } else if close(p, [127, 63, 191, 255], 3) {
        "per-visual opacity (each child multiplied)"
    } else {
        "unexpected"
    };
    println!("  -> {verdict}");
    Ok(())
}

/// Slint draws the border inside the rectangle; a centered stroke must be inset by half its width.
fn border_shape(x: &Ctx) -> Result<()> {
    println!("T2 ShapeVisual rounded rect: rect (10,10) 100x60, radius 12, border 4 red inset, fill green");
    let f = frame(&x.c, 140.0, 90.0)?;
    let sv = x.c.CreateShapeVisual()?;
    sv.SetOffset(V3 { X: 10.0, Y: 10.0, Z: 0.0 })?;
    sv.SetSize(V2 { X: 100.0, Y: 60.0 })?;
    let bw = 4.0;
    let geom = x.c.CreateRoundedRectangleGeometry()?;
    geom.SetOffset(V2 { X: bw / 2.0, Y: bw / 2.0 })?;
    geom.SetSize(V2 { X: 100.0 - bw, Y: 60.0 - bw })?;
    geom.SetCornerRadius(V2 { X: 12.0 - bw / 2.0, Y: 12.0 - bw / 2.0 })?;
    let shape = x.c.CreateSpriteShapeWithGeometry(&geom)?;
    shape.SetFillBrush(&x.c.CreateColorBrushWithColor(rgba(0, 160, 0, 255))?)?;
    shape.SetStrokeBrush(&x.c.CreateColorBrushWithColor(rgba(255, 0, 0, 255))?)?;
    shape.SetStrokeThickness(bw)?;
    sv.Shapes()?.Append(&shape)?;
    // A child visual of the ShapeVisual: is it drawn above the shapes, and is it clipped?
    sv.Children()?.InsertAtTop(&sprite(&x.c, 70.0, 40.0, 50.0, 30.0, [0, 0, 255, 255])?)?;
    f.Children()?.InsertAtTop(&sv)?;
    let img = x.shot(&f, "t2-border-shape")?;
    let b0 = px(&img, "left border first px", 10, 40);
    let b3 = px(&img, "left border last px", 13, 40);
    let fill = px(&img, "fill first px", 14, 40);
    let corner = px(&img, "corner outside radius", 11, 11);
    let child = px(&img, "child over shape", 85, 55);
    let child_out = px(&img, "child outside parent bounds", 115, 75);
    println!(
        "  -> border inset exact: {}; corner transparent: {}; child above shapes: {}; child unclipped: {}",
        close(b0, [255, 0, 0, 255], 8) && close(b3, [255, 0, 0, 255], 8) && close(fill, [0, 160, 0, 255], 8),
        close(corner, WHITE, 8),
        close(child, [0, 0, 255, 255], 2),
        close(child_out, [0, 0, 255, 255], 2),
    );
    Ok(())
}

#[implement(IGeometrySource2D, IGeometrySource2DInterop)]
struct D2DGeometry(ID2D1Geometry);

impl windows::Graphics::IGeometrySource2D_Impl for D2DGeometry_Impl {}

impl IGeometrySource2DInterop_Impl for D2DGeometry_Impl {
    fn GetGeometry(&self) -> Result<ID2D1Geometry> {
        Ok(self.0.clone())
    }
    fn TryGetGeometryUsingFactory(&self, _: windows::core::Ref<'_, ID2D1Factory>) -> Result<ID2D1Geometry> {
        Err(windows::Win32::Foundation::E_NOTIMPL.into())
    }
}

/// Rounded rectangle with a radius per corner (top-left, top-right, bottom-right, bottom-left).
fn rounded_rect_path(factory: &ID2D1Factory1, w: f32, h: f32, r: [f32; 4]) -> Result<ID2D1Geometry> {
    let p = |x, y| V2 { X: x, Y: y };
    let arc = |x, y, r: f32| D2D1_ARC_SEGMENT {
        point: p(x, y),
        size: D2D_SIZE_F { width: r, height: r },
        rotationAngle: 0.0,
        sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
        arcSize: D2D1_ARC_SIZE_SMALL,
    };
    unsafe {
        let path = factory.CreatePathGeometry()?;
        let sink = path.Open()?;
        sink.BeginFigure(p(r[0], 0.0), D2D1_FIGURE_BEGIN_FILLED);
        sink.AddLine(p(w - r[1], 0.0));
        sink.AddArc(&arc(w, r[1], r[1]));
        sink.AddLine(p(w, h - r[2]));
        sink.AddArc(&arc(w - r[2], h, r[2]));
        sink.AddLine(p(r[3], h));
        sink.AddArc(&arc(0.0, h - r[3], r[3]));
        sink.AddLine(p(0.0, r[0]));
        sink.AddArc(&arc(r[0], 0.0, r[0]));
        sink.EndFigure(D2D1_FIGURE_END_CLOSED);
        sink.Close()?;
        path.cast()
    }
}

fn per_corner_path(x: &Ctx) -> Result<()> {
    println!("T3 CompositionPath from a D2D geometry: 100x60, radii tl=0 tr=20 br=4 bl=30");
    let factory: ID2D1Factory1 = unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)? };
    let t0 = Instant::now();
    let geometry = rounded_rect_path(&factory, 100.0, 60.0, [0.0, 20.0, 4.0, 30.0])?;
    let source: IGeometrySource2D = D2DGeometry(geometry).into();
    let path = CompositionPath::Create(&source)?;
    let geom = x.c.CreatePathGeometryWithPath(&path)?;
    let shape = x.c.CreateSpriteShapeWithGeometry(&geom)?;
    shape.SetFillBrush(&x.c.CreateColorBrushWithColor(rgba(0, 120, 215, 255))?)?;
    let sv = x.c.CreateShapeVisual()?;
    sv.SetOffset(V3 { X: 10.0, Y: 10.0, Z: 0.0 })?;
    sv.SetSize(V2 { X: 100.0, Y: 60.0 })?;
    sv.Shapes()?.Append(&shape)?;
    println!("  build path + shape: {:.3} ms", ms(t0));
    let f = frame(&x.c, 120.0, 80.0)?;
    f.Children()?.InsertAtTop(&sv)?;
    let img = x.shot(&f, "t3-per-corner")?;
    let tl = px(&img, "top-left corner (r=0)", 10, 10);
    let tr = px(&img, "top-right corner (r=20)", 108, 11);
    let bl = px(&img, "bottom-left corner (r=30)", 13, 67);
    println!(
        "  -> tl filled: {}; tr transparent: {}; bl transparent: {}",
        close(tl, [0, 120, 215, 255], 8),
        close(tr, WHITE, 8),
        close(bl, WHITE, 8)
    );
    Ok(())
}

fn rounded_clip(x: &Ctx) -> Result<()> {
    println!("T4 rounded GeometricClip on a container clips its children");
    let f = frame(&x.c, 120.0, 80.0)?;
    let clip_host = x.c.CreateContainerVisual()?;
    clip_host.SetOffset(V3 { X: 10.0, Y: 10.0, Z: 0.0 })?;
    clip_host.SetSize(V2 { X: 100.0, Y: 60.0 })?;
    let geom = x.c.CreateRoundedRectangleGeometry()?;
    geom.SetSize(V2 { X: 100.0, Y: 60.0 })?;
    geom.SetCornerRadius(V2 { X: 20.0, Y: 20.0 })?;
    clip_host.SetClip(&x.c.CreateGeometricClipWithGeometry(&geom)?)?;
    clip_host.Children()?.InsertAtTop(&sprite(&x.c, -10.0, -10.0, 130.0, 80.0, [200, 0, 200, 255])?)?;
    f.Children()?.InsertAtTop(&clip_host)?;
    let img = x.shot(&f, "t4-clip")?;
    let corner = px(&img, "corner", 12, 12);
    let inside = px(&img, "inside", 60, 40);
    let outside = px(&img, "outside bounds", 5, 40);
    println!(
        "  -> corner clipped: {}; inside drawn: {}; outside clipped: {}",
        close(corner, WHITE, 8),
        close(inside, [200, 0, 200, 255], 2),
        close(outside, WHITE, 2)
    );
    Ok(())
}

fn gradients(x: &Ctx) -> Result<()> {
    println!("T5 linear and radial gradient brushes");
    let f = frame(&x.c, 220.0, 60.0)?;
    let lin = x.c.CreateLinearGradientBrush()?;
    lin.SetStartPoint(V2 { X: 0.0, Y: 0.0 })?;
    lin.SetEndPoint(V2 { X: 1.0, Y: 0.0 })?;
    lin.ColorStops()?.Append(&x.c.CreateColorGradientStopWithOffsetAndColor(0.0, rgba(255, 0, 0, 255))?)?;
    lin.ColorStops()?.Append(&x.c.CreateColorGradientStopWithOffsetAndColor(1.0, rgba(0, 0, 255, 255))?)?;
    let a = x.c.CreateSpriteVisual()?;
    a.SetSize(V2 { X: 100.0, Y: 60.0 })?;
    a.SetBrush(&lin)?;
    let rad = x.c.CreateRadialGradientBrush()?;
    rad.SetEllipseCenter(V2 { X: 0.5, Y: 0.5 })?;
    rad.SetEllipseRadius(V2 { X: 0.5, Y: 0.5 })?;
    rad.ColorStops()?.Append(&x.c.CreateColorGradientStopWithOffsetAndColor(0.0, rgba(255, 255, 0, 255))?)?;
    rad.ColorStops()?.Append(&x.c.CreateColorGradientStopWithOffsetAndColor(1.0, rgba(0, 128, 0, 255))?)?;
    let b = x.c.CreateSpriteVisual()?;
    b.SetOffset(V3 { X: 120.0, Y: 0.0, Z: 0.0 })?;
    b.SetSize(V2 { X: 100.0, Y: 60.0 })?;
    b.SetBrush(&rad)?;
    f.Children()?.InsertAtTop(&a)?;
    f.Children()?.InsertAtTop(&b)?;
    let img = x.shot(&f, "t5-gradients")?;
    px(&img, "linear start", 1, 30);
    px(&img, "linear mid", 50, 30);
    px(&img, "linear end", 98, 30);
    px(&img, "radial center", 170, 30);
    px(&img, "radial edge", 121, 30);
    Ok(())
}

/// Root scale for DPI: do fractional physical positions get anti-aliased edges?
fn fractional_scale(x: &Ctx) -> Result<()> {
    for scale in [1.25f32, 1.5] {
        println!("T6 scale {scale}: sprite at x=10 DIP width 20 DIP, and a pixel-snapped copy");
        let w = 60.0 * scale;
        let f = frame(&x.c, w, 40.0 * scale)?;
        let root = x.c.CreateContainerVisual()?;
        root.SetSize(V2 { X: 60.0, Y: 40.0 })?;
        root.SetScale(V3 { X: scale, Y: scale, Z: 1.0 })?;
        let snap = |v: f32| (v * scale).round() / scale;
        root.Children()?.InsertAtTop(&sprite(&x.c, 10.0, 2.0, 20.0, 16.0, [0, 0, 0, 255])?)?;
        root.Children()?.InsertAtTop(&sprite(&x.c, snap(10.0), 22.0, snap(30.0) - snap(10.0), 16.0, [0, 0, 0, 255])?)?;
        f.Children()?.InsertAtTop(&root)?;
        let img = x.shot(&f, &format!("t6-scale-{scale}"))?;
        let row = |y: u32| -> String {
            (8..16).map(|xx| format!("{:3}", img.pixel(xx, y)[0])).collect::<Vec<_>>().join(" ")
        };
        let y1 = (10.0 * scale) as u32;
        let y2 = (30.0 * scale) as u32;
        println!("  red channel, physical x=8..15, unsnapped: {}", row(y1));
        println!("  red channel, physical x=8..15, snapped:   {}", row(y2));
    }
    Ok(())
}

/// Visual count cost: create N sprites in a grid under a window, commit, update all, free.
fn perf(c: &Compositor, n: usize, shared_brush: bool) -> Result<()> {
    let host = Host::new(c, "S2 perf", 800, 600)?;
    wait_commit(c)?;
    let self0 = private_bytes();
    let dwm0 = dwm_private_bytes();
    let t0 = Instant::now();
    let container = c.CreateContainerVisual()?;
    let shared = c.CreateColorBrushWithColor(rgba(0, 120, 215, 255))?;
    let children = container.Children()?;
    let mut sprites = Vec::with_capacity(n);
    for i in 0..n {
        let s = c.CreateSpriteVisual()?;
        s.SetOffset(V3 { X: (i % 200) as f32 * 4.0, Y: (i / 200) as f32 * 2.0, Z: 0.0 })?;
        s.SetSize(V2 { X: 3.0, Y: 1.0 })?;
        if shared_brush {
            s.SetBrush(&shared)?;
        } else {
            s.SetBrush(&c.CreateColorBrushWithColor(rgba((i % 256) as u8, 120, 215, 255))?)?;
        }
        children.InsertAtTop(&s)?;
        sprites.push(s);
    }
    let create = ms(t0);
    host.root.Children()?.InsertAtTop(&container)?;
    let commit = wait_commit(c)?;
    let self1 = private_bytes();
    let dwm1 = dwm_private_bytes();
    let t1 = Instant::now();
    for (i, s) in sprites.iter().enumerate() {
        s.SetOffset(V3 { X: (i % 200) as f32 * 4.0 + 1.0, Y: (i / 200) as f32 * 2.0, Z: 0.0 })?;
    }
    let update = ms(t1);
    let update_commit = wait_commit(c)?;
    let t2 = Instant::now();
    children.RemoveAll()?;
    drop(sprites);
    let remove = ms(t2);
    let mb = |a: usize, b: usize| (b as f64 - a as f64) / (1024.0 * 1024.0);
    let dwm = match (dwm0, dwm1) {
        (Some(a), Some(b)) => format!("{:+.1} MB", mb(a, b)),
        _ => "n/a".into(),
    };
    println!(
        "  n={n:6} brush={}: create {create:7.1} ms ({:.2} us/visual), first commit {commit:6.1} ms, \
         update all offsets {update:6.1} ms + commit {update_commit:5.1} ms, remove {remove:6.1} ms, \
         self {:+.1} MB, dwm {dwm}",
        if shared_brush { "shared" } else { "own   " },
        create * 1000.0 / n as f64,
        mb(self0, self1),
    );
    Ok(())
}

#[derive(Debug, Clone, Copy)]
enum Insert {
    FlatTop,
    FlatBottom,
    FlatAbove,
    Chunked100,
}

/// Separates creating visuals from inserting them, for several insertion patterns.
fn insert_cost(c: &Compositor, n: usize, how: Insert) -> Result<()> {
    let brush = c.CreateColorBrushWithColor(rgba(0, 120, 215, 255))?;
    let t0 = Instant::now();
    let mut sprites = Vec::with_capacity(n);
    for i in 0..n {
        let s = c.CreateSpriteVisual()?;
        s.SetOffset(V3 { X: (i % 200) as f32 * 4.0, Y: (i / 200) as f32 * 2.0, Z: 0.0 })?;
        s.SetSize(V2 { X: 3.0, Y: 1.0 })?;
        s.SetBrush(&brush)?;
        sprites.push(s);
    }
    let create = ms(t0);
    let root = c.CreateContainerVisual()?;
    let t1 = Instant::now();
    match how {
        Insert::FlatTop => {
            let ch = root.Children()?;
            for s in &sprites {
                ch.InsertAtTop(s)?;
            }
        }
        Insert::FlatBottom => {
            let ch = root.Children()?;
            for s in sprites.iter().rev() {
                ch.InsertAtBottom(s)?;
            }
        }
        Insert::FlatAbove => {
            let ch = root.Children()?;
            ch.InsertAtBottom(&sprites[0])?;
            for w in sprites.windows(2) {
                ch.InsertAbove(&w[1], &w[0])?;
            }
        }
        Insert::Chunked100 => {
            let ch = root.Children()?;
            for chunk in sprites.chunks(100) {
                let group = c.CreateContainerVisual()?;
                let gch = group.Children()?;
                for s in chunk {
                    gch.InsertAtTop(s)?;
                }
                ch.InsertAtTop(&group)?;
            }
        }
    }
    let insert = ms(t1);
    println!(
        "  n={n:6} {:<10}: create {create:7.1} ms ({:.2} us/visual), insert {insert:8.1} ms ({:.2} us/visual)",
        format!("{how:?}"),
        create * 1000.0 / n as f64,
        insert * 1000.0 / n as f64
    );
    root.Children()?.RemoveAll()?;
    Ok(())
}

fn main() -> Result<()> {
    if std::env::args().any(|a| a == "insert") {
        let _queue = init_thread()?;
        let c = Compositor::new()?;
        println!("T8 create vs insert");
        for n in [1_000, 5_000, 10_000, 20_000] {
            for how in [Insert::FlatTop, Insert::FlatBottom, Insert::FlatAbove, Insert::Chunked100] {
                insert_cost(&c, n, how)?;
            }
        }
        return Ok(());
    }
    let _queue = init_thread()?;
    let c = Compositor::new()?;
    let out = std::env::args().nth(1).unwrap_or_else(|| ".".into());
    if std::env::args().any(|a| a == "perf") {
        println!("T7 visual count cost");
        for n in [1_000, 10_000, 50_000] {
            perf(&c, n, true)?;
            perf(&c, n, false)?;
        }
        return Ok(());
    }
    let x = Ctx { c, cap: Capturer::new()?, out };
    group_opacity(&x)?;
    border_shape(&x)?;
    per_corner_path(&x)?;
    rounded_clip(&x)?;
    gradients(&x)?;
    fractional_scale(&x)?;
    Ok(())
}
