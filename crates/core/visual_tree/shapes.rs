// SPDX-License-Identifier: MIT OR Apache-2.0

//! Composition brushes and geometries for Slint brushes and rounded rectangles.
//!
//! Visuals use logical coordinates (the window's root visual carries the scale factor), so the
//! `PhysicalPx` values from the renderer-neutral helpers in `graphics` and `item_rendering`,
//! computed with a scale factor of 1, are logical here.

#![allow(unsafe_code)]

use super::graphics::MAX_SURFACE_SIDE;
use crate::graphics::{Brush, GradientStop, ResolvedBrush, ResolvedConicGradient, resolve_brush};
use crate::lengths::{PhysicalBorderRadius, PhysicalPx, ScaleFactor};
use alloc::vec::Vec;
use windows::Foundation::Size;
use windows::Graphics::DirectX::{DirectXAlphaMode, DirectXPixelFormat};
use windows::Graphics::IGeometrySource2D;
use windows::UI::Composition::{
    CompositionBorderMode, CompositionBrush, CompositionClip,
    CompositionColorGradientStopCollection, CompositionGeometricClip, CompositionGeometry,
    CompositionGradientBrush, CompositionMappingMode, CompositionPath,
    CompositionRoundedRectangleGeometry, CompositionSpriteShape, CompositionStretch,
    CompositionSurfaceBrush, Compositor, ContainerVisual, InsetClip,
};
use windows::Win32::Foundation::E_FAIL;
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_SIZE_F, D2D1_FIGURE_BEGIN_FILLED, D2D1_FIGURE_END_CLOSED, D2D1_FILL_MODE_WINDING,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_ARC_SEGMENT, D2D1_ARC_SIZE_SMALL, D2D1_FACTORY_TYPE_SINGLE_THREADED,
    D2D1_SWEEP_DIRECTION_CLOCKWISE, D2D1CreateFactory, ID2D1Factory, ID2D1Factory1, ID2D1Geometry,
    ID2D1GeometrySink, ID2D1StrokeStyle,
};
use windows::Win32::System::WinRT::Graphics::Direct2D::{
    IGeometrySource2DInterop, IGeometrySource2DInterop_Impl,
};
use windows::core::{Interface, implement};
use windows_numerics::Vector2;

pub(super) fn ui_color(color: crate::Color) -> windows::UI::Color {
    windows::UI::Color { A: color.alpha(), R: color.red(), G: color.green(), B: color.blue() }
}

fn vector(p: euclid::Point2D<f32, PhysicalPx>) -> Vector2 {
    Vector2 { X: p.x, Y: p.y }
}

/// A brush that paints `brush` over a shape of `size` whose origin is the brush's origin, or
/// `None` if `brush` is fully transparent. `scale` gives the window's scale factor, which only
/// a conic gradient reads.
pub(super) fn make_brush(
    c: &Compositor,
    brush: &Brush,
    size: euclid::Size2D<f32, PhysicalPx>,
    scale: impl FnOnce() -> f32,
) -> Option<CompositionBrush> {
    let resolved = resolve_brush(brush, size, ScaleFactor::new(1.))?;
    let result: windows::core::Result<CompositionBrush> = (|| match resolved {
        ResolvedBrush::SolidColor(color) => c.CreateColorBrushWithColor(ui_color(color))?.cast(),
        ResolvedBrush::LinearGradient(g) => {
            let b = c.CreateLinearGradientBrush()?;
            b.SetMappingMode(CompositionMappingMode::Absolute)?;
            b.SetStartPoint(vector(g.start))?;
            b.SetEndPoint(vector(g.end))?;
            add_stops(c, &b.ColorStops()?, &g.stops)?;
            b.cast()
        }
        ResolvedBrush::RadialGradient(g) => {
            let b = c.CreateRadialGradientBrush()?;
            b.SetMappingMode(CompositionMappingMode::Absolute)?;
            b.SetEllipseCenter(vector(g.center))?;
            b.SetEllipseRadius(Vector2 { X: g.radius_x.get(), Y: g.radius_y.get() })?;
            add_stops(c, &b.ColorStops()?, &g.stops)?;
            b.cast()
        }
        ResolvedBrush::ConicGradient(g) => conic_brush(c, &g, size, scale()),
    })();
    result.ok()
}

/// How many pixels a conic gradient's bitmap has before it's drawn at less than four times the
/// device's density.
const CONIC_PIXEL_BUDGET: f32 = 1024. * 1024.;

/// A surface brush that shows `g` over `size` from the origin, drawn at least at `scale` pixels
/// per unit.
fn conic_brush(
    c: &Compositor,
    g: &ResolvedConicGradient,
    size: euclid::Size2D<f32, PhysicalPx>,
    scale: f32,
) -> windows::core::Result<CompositionBrush> {
    let graphics = super::graphics::graphics(c).ok_or_else(|| windows::core::Error::from(E_FAIL))?;
    graphics.track_device();
    // The compositor samples the bitmap between its pixels where the item lies at a fractional
    // pixel. A denser bitmap keeps that blend narrow where colors change abruptly.
    let area = (size.width * size.height * scale * scale).max(1.);
    let density = (CONIC_PIXEL_BUDGET / area).sqrt().clamp(1., 4.);
    let texel = (scale * density).min(MAX_SURFACE_SIDE / size.width.max(size.height).max(1.));
    let pixels = |v: f32| ((v * texel).ceil() as u32).max(1);
    let (w, h) = (pixels(size.width), pixels(size.height));
    let surface = graphics.composition.CreateDrawingSurface(
        Size { Width: w as f32, Height: h as f32 },
        DirectXPixelFormat::B8G8R8A8UIntNormalized,
        DirectXAlphaMode::Premultiplied,
    )?;
    if let Err(e) = super::image::upload(&surface, &super::conic::pixels(g, w, h, texel), w, h) {
        graphics.drawing_failed(&e, "a conic gradient");
        return Err(e);
    }
    let brush = c.CreateSurfaceBrushWithSurface(&surface)?;
    brush.SetStretch(CompositionStretch::None)?;
    brush.SetHorizontalAlignmentRatio(0.)?;
    brush.SetVerticalAlignmentRatio(0.)?;
    brush.SetTransformMatrix(windows_numerics::Matrix3x2::scale(1. / texel, 1. / texel))?;
    brush.cast()
}

fn add_stops(
    c: &Compositor,
    stops: &CompositionColorGradientStopCollection,
    from: &[GradientStop],
) -> windows::core::Result<()> {
    for (position, color) in premultiplied_stops(from) {
        stops.Append(&c.CreateColorGradientStopWithOffsetAndColor(position, ui_color(color))?)?;
    }
    Ok(())
}

/// How many pieces a gradient segment between stops of different alpha is cut into.
const ALPHA_STEPS: usize = 8;

/// Stops that make the compositor's gradients, which interpolate straight colors, look like
/// upstream's, which interpolate premultiplied colors (Skia's `InPremul::Yes`). The two differ
/// between stops of different alpha: there, intermediate stops follow the premultiplied blend,
/// and a transparent stop takes the color of the stop next to it on each side.
fn premultiplied_stops(from: &[GradientStop]) -> Vec<(f32, crate::Color)> {
    let rgba = |c: crate::Color| {
        [c.red() as f32, c.green() as f32, c.blue() as f32, c.alpha() as f32 / 255.]
    };
    let color = |[r, g, b, a]: [f32; 4]| {
        crate::Color::from_argb_u8((a * 255.).round() as u8, r.round() as u8, g.round() as u8, b.round() as u8)
    };
    let [first, ..] = from else { return Vec::new() };
    if from.len() == 1 {
        return alloc::vec![(first.position, first.color)];
    }
    let mut out = Vec::new();
    for pair in from.windows(2) {
        let (s0, s1) = (&pair[0], &pair[1]);
        let (c0, c1) = (rgba(s0.color), rgba(s1.color));
        // The color a transparent end takes.
        let start = if c0[3] == 0. { [c1[0], c1[1], c1[2], 0.] } else { c0 };
        let end = if c1[3] == 0. { [c0[0], c0[1], c0[2], 0.] } else { c1 };
        out.push((s0.position, color(start)));
        if c0[3] != c1[3] && s1.position > s0.position {
            for k in 1..ALPHA_STEPS {
                let t = k as f32 / ALPHA_STEPS as f32;
                let (w0, w1) = (c0[3] * (1. - t), c1[3] * t);
                let a = w0 + w1;
                let mix = |i: usize| (c0[i] * w0 + c1[i] * w1) / a;
                out.push((
                    s0.position + (s1.position - s0.position) * t,
                    color([mix(0), mix(1), mix(2), a]),
                ));
            }
        }
        out.push((s1.position, color(end)));
    }
    out
}

/// A rectangle with rounded corners. A uniform radius maps to the native rounded rectangle;
/// per-corner radii go through a Direct2D path (S2).
pub(super) fn rounded_rect(
    c: &Compositor,
    rect: euclid::Rect<f32, PhysicalPx>,
    radius: PhysicalBorderRadius,
) -> windows::core::Result<CompositionGeometry> {
    let radius = clamp_radius(rect.size, radius);
    if let Some(r) = radius.as_uniform() {
        let g = c.CreateRoundedRectangleGeometry()?;
        g.SetOffset(vector(rect.origin))?;
        g.SetSize(Vector2 { X: rect.width(), Y: rect.height() })?;
        g.SetCornerRadius(Vector2 { X: r, Y: r })?;
        return g.cast();
    }
    path_geometry(c, rounded_rect_path(rect, radius)?)
}

/// Moves and resizes the rounded rectangle geometry of `shape` to `rect` with `radius`, as
/// [`rounded_rect`] would make it, and returns whether it could: a geometry for corners of
/// different radii is a path, which can't change in place.
pub(super) fn update_rounded_rect(
    shape: &CompositionSpriteShape,
    rect: euclid::Rect<f32, PhysicalPx>,
    radius: PhysicalBorderRadius,
) -> bool {
    let Some(r) = clamp_radius(rect.size, radius).as_uniform() else { return false };
    let Ok(g) = shape.Geometry().and_then(|g| g.cast::<CompositionRoundedRectangleGeometry>())
    else {
        return false;
    };
    let _ = g.SetOffset(vector(rect.origin));
    let _ = g.SetSize(Vector2 { X: rect.width(), Y: rect.height() });
    let _ = g.SetCornerRadius(Vector2 { X: r, Y: r });
    true
}

/// Whether a sprite shape can't show `brush`: Composition shapes take color and gradient
/// brushes, but not the surface brush of a conic gradient.
pub(super) fn needs_sprite(brush: &Brush) -> bool {
    matches!(brush, Brush::ConicGradient(_)) && !brush.is_transparent()
}

/// The area that `geometry` stroked `width` wide with `style` covers, as a geometry to fill.
pub(super) fn widen(
    geometry: &ID2D1Geometry,
    width: f32,
    style: Option<&ID2D1StrokeStyle>,
) -> windows::core::Result<ID2D1Geometry> {
    unsafe {
        let path = d2d_factory()?.CreatePathGeometry()?;
        let sink = path.Open()?;
        sink.SetFillMode(D2D1_FILL_MODE_WINDING);
        geometry.Widen(width, style, None, FLATTENING_TOLERANCE, &sink)?;
        sink.Close()?;
        path.cast()
    }
}

/// How far, in logical pixels, a widened outline's straight segments may stray from its curves.
const FLATTENING_TOLERANCE: f32 = 0.05;

/// Adds a sprite of `size` above `floor`'s other children that shows `brush` cut to `outline`.
/// Brush and outline are relative to `origin` in `floor`. This shows what a sprite shape can't
/// (see [`needs_sprite`]).
pub(super) fn append_clipped(
    c: &Compositor,
    floor: &ContainerVisual,
    brush: &CompositionBrush,
    outline: ID2D1Geometry,
    size: Vector2,
    origin: Vector2,
    border: CompositionBorderMode,
) -> windows::core::Result<()> {
    let moved = |m: windows_numerics::Matrix3x2| {
        m * windows_numerics::Matrix3x2::translation(origin.X, origin.Y)
    };
    if let Ok(b) = brush.cast::<CompositionGradientBrush>() {
        b.SetTransformMatrix(moved(b.TransformMatrix()?))?;
    } else if let Ok(b) = brush.cast::<CompositionSurfaceBrush>() {
        b.SetTransformMatrix(moved(b.TransformMatrix()?))?;
    }
    let sprite = c.CreateSpriteVisual()?;
    sprite.SetSize(size)?;
    sprite.SetBrush(brush)?;
    let clip = c.CreateGeometricClipWithGeometry(&path_geometry(c, outline)?)?;
    clip.SetOffset(origin)?;
    sprite.SetClip(&clip)?;
    sprite.SetBorderMode(border)?;
    floor.Children()?.InsertAtTop(&sprite)
}

/// A Direct2D geometry as a composition geometry.
pub(super) fn path_geometry(
    c: &Compositor,
    geometry: ID2D1Geometry,
) -> windows::core::Result<CompositionGeometry> {
    let source: IGeometrySource2D = D2DGeometry(geometry).into();
    c.CreatePathGeometryWithPath(&CompositionPath::Create(&source)?)?.cast()
}

/// Scales the radii down so that adjacent corners don't overlap, as CSS does.
fn clamp_radius(
    size: euclid::Size2D<f32, PhysicalPx>,
    r: PhysicalBorderRadius,
) -> PhysicalBorderRadius {
    let fit =
        |side: f32, a: f32, b: f32| if a + b > side && a + b > 0. { side / (a + b) } else { 1. };
    let f = fit(size.width, r.top_left, r.top_right)
        .min(fit(size.width, r.bottom_left, r.bottom_right))
        .min(fit(size.height, r.top_left, r.bottom_left))
        .min(fit(size.height, r.top_right, r.bottom_right));
    if f < 1. {
        PhysicalBorderRadius::new(
            r.top_left * f,
            r.top_right * f,
            r.bottom_right * f,
            r.bottom_left * f,
        )
    } else {
        r
    }
}

std::thread_local! {
    static D2D_FACTORY: core::cell::OnceCell<ID2D1Factory1> = const { core::cell::OnceCell::new() };
}

/// The thread's Direct2D factory: the geometries composition shows and the drawing device's
/// share it.
pub(super) fn d2d_factory() -> windows::core::Result<ID2D1Factory1> {
    D2D_FACTORY.with(|f| {
        if let Some(factory) = f.get() {
            return Ok(factory.clone());
        }
        let factory: ID2D1Factory1 =
            unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)? };
        let _ = f.set(factory.clone());
        Ok(factory)
    })
}

/// The rounded rectangle as a Direct2D path geometry, with the radii scaled down so that
/// adjacent corners don't overlap.
pub(super) fn rounded_rect_path(
    rect: euclid::Rect<f32, PhysicalPx>,
    r: PhysicalBorderRadius,
) -> windows::core::Result<ID2D1Geometry> {
    let factory = d2d_factory()?;
    unsafe {
        let path = factory.CreatePathGeometry()?;
        let sink = path.Open()?;
        add_rounded_rect(&sink, rect, r);
        sink.Close()?;
        path.cast()
    }
}

/// Adds the rounded rectangle to `sink` as a closed figure, with the radii scaled down so that
/// adjacent corners don't overlap.
pub(super) fn add_rounded_rect(
    sink: &ID2D1GeometrySink,
    rect: euclid::Rect<f32, PhysicalPx>,
    r: PhysicalBorderRadius,
) {
    let r = clamp_radius(rect.size, r);
    let (x0, y0, x1, y1) = (rect.min_x(), rect.min_y(), rect.max_x(), rect.max_y());
    let p = |x, y| Vector2 { X: x, Y: y };
    let arc = |x, y, r: f32| D2D1_ARC_SEGMENT {
        point: p(x, y),
        size: D2D_SIZE_F { width: r, height: r },
        rotationAngle: 0.0,
        sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
        arcSize: D2D1_ARC_SIZE_SMALL,
    };
    unsafe {
        // An arc of radius 0 ends the figure where Composition reads it, so a square corner has
        // none.
        let corner = |x, y, r: f32| {
            if r > 0. {
                sink.AddArc(&arc(x, y, r));
            }
        };
        sink.BeginFigure(p(x0 + r.top_left, y0), D2D1_FIGURE_BEGIN_FILLED);
        sink.AddLine(p(x1 - r.top_right, y0));
        corner(x1, y0 + r.top_right, r.top_right);
        sink.AddLine(p(x1, y1 - r.bottom_right));
        corner(x1 - r.bottom_right, y1, r.bottom_right);
        sink.AddLine(p(x0 + r.bottom_left, y1));
        corner(x0, y1 - r.bottom_left, r.bottom_left);
        sink.AddLine(p(x0, y0 + r.top_left));
        corner(x0 + r.top_left, y0, r.top_left);
        sink.EndFigure(D2D1_FIGURE_END_CLOSED);
    }
}

/// Describes a brush transform for tree dumps: ` m=[m11 m12 m21 m22 m31 m32]`.
fn describe_matrix(m: windows_numerics::Matrix3x2) -> alloc::string::String {
    let round = |v: f32| (v * 1e4).round() / 1e4 + 0.;
    alloc::format!(
        " m=[{} {} {} {} {} {}]",
        round(m.M11),
        round(m.M12),
        round(m.M21),
        round(m.M22),
        round(m.M31),
        round(m.M32)
    )
}

/// Describes a brush for tree dumps: `#rrggbbaa`, `linear(x,y->x,y #rrggbbaa@0 ...)`, or
/// `radial(x,y rx,ry #rrggbbaa@0 ...)`, the gradients followed by their transform if they have
/// one.
pub(super) fn describe_brush(brush: &CompositionBrush) -> alloc::string::String {
    use windows::UI::Composition::{
        CompositionColorBrush, CompositionGradientBrush, CompositionLinearGradientBrush,
        CompositionRadialGradientBrush,
    };
    let hex =
        |c: windows::UI::Color| alloc::format!("#{:02x}{:02x}{:02x}{:02x}", c.R, c.G, c.B, c.A);
    let stops = |b: &CompositionGradientBrush| -> alloc::string::String {
        let Ok(stops) = b.ColorStops() else { return Default::default() };
        let mut out: alloc::string::String = (&stops)
            .into_iter()
            .map(|s| {
                alloc::format!(
                    " {}@{}",
                    hex(s.Color().unwrap_or_default()),
                    s.Offset().unwrap_or(0.)
                )
            })
            .collect();
        let m = b.TransformMatrix().unwrap_or_default();
        if m != windows_numerics::Matrix3x2::identity() {
            out += &describe_matrix(m);
        }
        out
    };
    if let Ok(b) = brush.cast::<CompositionColorBrush>() {
        return hex(b.Color().unwrap_or_default());
    }
    if let Ok(b) = brush.cast::<windows::UI::Composition::CompositionSurfaceBrush>() {
        let size = b
            .Surface()
            .and_then(|s| s.cast::<windows::UI::Composition::CompositionDrawingSurface>())
            .and_then(|s| s.SizeInt32())
            .unwrap_or_default();
        let mut out = alloc::format!("surface {}x{}", size.Width, size.Height);
        let m = b.TransformMatrix().unwrap_or_default();
        if m != windows_numerics::Matrix3x2::identity() {
            out += &describe_matrix(m);
        }
        if b.BitmapInterpolationMode().ok()
            == Some(windows::UI::Composition::CompositionBitmapInterpolationMode::NearestNeighbor)
        {
            out += " nearest";
        }
        return out;
    }
    if let Ok(b) = brush.cast::<windows::UI::Composition::CompositionEffectBrush>() {
        // The effect graph isn't readable back; its sources are.
        let source = |name: &str| {
            b.GetSourceParameter(&name.into()).ok().map(|s| describe_brush(&s))
        };
        let mut out = alloc::string::String::from("effect(");
        if let Some(image) = source("image") {
            out += &image;
        }
        if let Some(color) = source("color") {
            out += &alloc::format!(" colorize {color}");
        }
        if let Some(backdrop) = source("backdrop") {
            out += &backdrop;
        }
        out.push(')');
        return out;
    }
    if brush.cast::<windows::UI::Composition::CompositionBackdropBrush>().is_ok() {
        return "backdrop".into();
    }
    if let Ok(b) = brush.cast::<CompositionLinearGradientBrush>() {
        let (s, e) = (b.StartPoint().unwrap_or_default(), b.EndPoint().unwrap_or_default());
        return alloc::format!(
            "linear({},{}->{},{}{})",
            s.X,
            s.Y,
            e.X,
            e.Y,
            stops(&b.cast().unwrap())
        );
    }
    if let Ok(b) = brush.cast::<CompositionRadialGradientBrush>() {
        let (c, r) = (b.EllipseCenter().unwrap_or_default(), b.EllipseRadius().unwrap_or_default());
        return alloc::format!(
            "radial({},{} {},{}{})",
            c.X,
            c.Y,
            r.X,
            r.Y,
            stops(&b.cast().unwrap())
        );
    }
    "brush".into()
}

/// Describes the shapes of a shape visual for tree dumps, each as
/// ` [fill <brush> stroke <brush> w<thickness><style> <geometry>]`, the geometry being
/// `rrect x,y wxh r<radius>` or `path`.
pub(super) fn describe_shapes(
    visual: &windows::UI::Composition::ShapeVisual,
) -> alloc::string::String {
    use core::fmt::Write;
    let mut out = alloc::string::String::new();
    let Ok(shapes) = visual.Shapes() else { return out };
    for shape in &shapes {
        let Ok(shape) = shape.cast::<CompositionSpriteShape>() else { continue };
        out.push_str(" [");
        let mut parts = alloc::vec::Vec::new();
        if let Ok(b) = shape.FillBrush() {
            parts.push(alloc::format!("fill {}", describe_brush(&b)));
        }
        if let Ok(b) = shape.StrokeBrush() {
            parts.push(alloc::format!(
                "stroke {} w{}{}",
                describe_brush(&b),
                shape.StrokeThickness().unwrap_or(0.),
                describe_stroke_style(&shape)
            ));
        }
        parts.push(shape.Geometry().map_or_else(|_| "path".into(), |g| describe_geometry(&g)));
        let _ = write!(out, "{}]", parts.join(" "));
    }
    out
}

/// Describes what differs from the defaults of a `Path` in a stroke: ` cap=round|square`,
/// ` join=round|bevel`, ` miter=<limit>`, and ` dash=[<lengths>]+<offset>`.
fn describe_stroke_style(shape: &CompositionSpriteShape) -> alloc::string::String {
    use windows::UI::Composition::{CompositionStrokeCap, CompositionStrokeLineJoin};
    let mut out = alloc::string::String::new();
    match shape.StrokeStartCap() {
        Ok(CompositionStrokeCap::Round) => out += " cap=round",
        Ok(CompositionStrokeCap::Square) => out += " cap=square",
        _ => {}
    }
    match shape.StrokeLineJoin() {
        Ok(CompositionStrokeLineJoin::Round) => out += " join=round",
        Ok(CompositionStrokeLineJoin::Bevel) => out += " join=bevel",
        // A `Path`'s miter joins; its miter limit defaults to 4.
        Ok(CompositionStrokeLineJoin::MiterOrBevel) => {
            let limit = shape.StrokeMiterLimit().unwrap_or(4.);
            if limit != 4. {
                out += &alloc::format!(" miter={limit}");
            }
        }
        _ => {}
    }
    if let Ok(dashes) = shape.StrokeDashArray()
        && dashes.Size().unwrap_or(0) > 0
    {
        let lengths: alloc::vec::Vec<_> =
            (&dashes).into_iter().map(|l| alloc::format!("{l}")).collect();
        out += &alloc::format!(" dash=[{}]", lengths.join(","));
        let offset = shape.StrokeDashOffset().unwrap_or(0.);
        if offset != 0. {
            out += &alloc::format!("+{offset}");
        }
    }
    out
}

/// Describes a geometry for tree dumps: `rrect x,y wxh r<radius>` or `path`.
fn describe_geometry(geometry: &CompositionGeometry) -> alloc::string::String {
    match geometry.cast::<CompositionRoundedRectangleGeometry>() {
        Ok(g) => {
            let (o, s, r) = (
                g.Offset().unwrap_or_default(),
                g.Size().unwrap_or_default(),
                g.CornerRadius().unwrap_or_default(),
            );
            alloc::format!("rrect {},{} {}x{} r{}", o.X, o.Y, s.X, s.Y, r.X)
        }
        Err(_) => "path".into(),
    }
}

/// Describes a clip for tree dumps: `inset(left,top,right,bottom)` or the clip's geometry as
/// `rrect x,y wxh r<radius>` or `path`.
pub(super) fn describe_clip(clip: &CompositionClip) -> alloc::string::String {
    if let Ok(c) = clip.cast::<InsetClip>() {
        return alloc::format!(
            "inset({},{},{},{})",
            c.LeftInset().unwrap_or_default(),
            c.TopInset().unwrap_or_default(),
            c.RightInset().unwrap_or_default(),
            c.BottomInset().unwrap_or_default()
        );
    }
    match clip.cast::<CompositionGeometricClip>().and_then(|c| c.Geometry()) {
        Ok(g) => describe_geometry(&g),
        Err(_) => "unknown".into(),
    }
}

/// Hands a Direct2D geometry to `CompositionPath`.
#[implement(IGeometrySource2D, IGeometrySource2DInterop)]
struct D2DGeometry(ID2D1Geometry);

impl windows::Graphics::IGeometrySource2D_Impl for D2DGeometry_Impl {}

impl IGeometrySource2DInterop_Impl for D2DGeometry_Impl {
    fn GetGeometry(&self) -> windows::core::Result<ID2D1Geometry> {
        Ok(self.0.clone())
    }

    fn TryGetGeometryUsingFactory(
        &self,
        _: windows::core::Ref<'_, ID2D1Factory>,
    ) -> windows::core::Result<ID2D1Geometry> {
        Err(windows::Win32::Foundation::E_NOTIMPL.into())
    }
}
