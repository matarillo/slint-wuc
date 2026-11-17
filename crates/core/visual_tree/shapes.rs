// SPDX-License-Identifier: MIT OR Apache-2.0

//! Composition brushes and geometries for Slint brushes and rounded rectangles.
//!
//! Visuals use logical coordinates (the window's root visual carries the scale factor), so the
//! `PhysicalPx` values from the renderer-neutral helpers in `graphics` and `item_rendering`,
//! computed with a scale factor of 1, are logical here.

#![allow(unsafe_code)]

use crate::graphics::{Brush, GradientStop, ResolvedBrush, resolve_brush};
use crate::lengths::{PhysicalBorderRadius, PhysicalPx, ScaleFactor};
use windows::Graphics::IGeometrySource2D;
use windows::UI::Composition::{
    CompositionBrush, CompositionClip, CompositionColorGradientStopCollection,
    CompositionGeometricClip, CompositionGeometry, CompositionMappingMode, CompositionPath,
    CompositionRoundedRectangleGeometry, Compositor, InsetClip,
};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_SIZE_F, D2D1_FIGURE_BEGIN_FILLED, D2D1_FIGURE_END_CLOSED,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_ARC_SEGMENT, D2D1_ARC_SIZE_SMALL, D2D1_FACTORY_TYPE_SINGLE_THREADED,
    D2D1_SWEEP_DIRECTION_CLOCKWISE, D2D1CreateFactory, ID2D1Factory, ID2D1Factory1, ID2D1Geometry,
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
/// `None` if `brush` is fully transparent.
pub(super) fn make_brush(
    c: &Compositor,
    brush: &Brush,
    size: euclid::Size2D<f32, PhysicalPx>,
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
        // TODO(phase 5): there is no conic gradient brush; draw it into a surface (S2).
        // Until then it shows as its first stop.
        ResolvedBrush::ConicGradient(g) => c
            .CreateColorBrushWithColor(ui_color(
                g.stops.first().map(|s| s.color).unwrap_or_default(),
            ))?
            .cast(),
    })();
    result.ok()
}

fn add_stops(
    c: &Compositor,
    stops: &CompositionColorGradientStopCollection,
    from: &[GradientStop],
) -> windows::core::Result<()> {
    for stop in from {
        stops.Append(
            &c.CreateColorGradientStopWithOffsetAndColor(stop.position, ui_color(stop.color))?,
        )?;
    }
    Ok(())
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
    let geometry = rounded_rect_path(rect, radius)?;
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

fn rounded_rect_path(
    rect: euclid::Rect<f32, PhysicalPx>,
    r: PhysicalBorderRadius,
) -> windows::core::Result<ID2D1Geometry> {
    let factory = D2D_FACTORY.with(|f| -> windows::core::Result<ID2D1Factory1> {
        if let Some(factory) = f.get() {
            return Ok(factory.clone());
        }
        let factory: ID2D1Factory1 =
            unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)? };
        let _ = f.set(factory.clone());
        Ok(factory)
    })?;
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
        let path = factory.CreatePathGeometry()?;
        let sink = path.Open()?;
        sink.BeginFigure(p(x0 + r.top_left, y0), D2D1_FIGURE_BEGIN_FILLED);
        sink.AddLine(p(x1 - r.top_right, y0));
        sink.AddArc(&arc(x1, y0 + r.top_right, r.top_right));
        sink.AddLine(p(x1, y1 - r.bottom_right));
        sink.AddArc(&arc(x1 - r.bottom_right, y1, r.bottom_right));
        sink.AddLine(p(x0 + r.bottom_left, y1));
        sink.AddArc(&arc(x0, y1 - r.bottom_left, r.bottom_left));
        sink.AddLine(p(x0, y0 + r.top_left));
        sink.AddArc(&arc(x0 + r.top_left, y0, r.top_left));
        sink.EndFigure(D2D1_FIGURE_END_CLOSED);
        sink.Close()?;
        path.cast()
    }
}

/// Describes a brush for tree dumps: `#rrggbbaa`, `linear(x,y->x,y #rrggbbaa@0 ...)`, or
/// `radial(x,y rx,ry #rrggbbaa@0 ...)`.
pub(super) fn describe_brush(brush: &CompositionBrush) -> alloc::string::String {
    use windows::UI::Composition::{
        CompositionColorBrush, CompositionGradientBrush, CompositionLinearGradientBrush,
        CompositionRadialGradientBrush,
    };
    let hex =
        |c: windows::UI::Color| alloc::format!("#{:02x}{:02x}{:02x}{:02x}", c.R, c.G, c.B, c.A);
    let stops = |b: &CompositionGradientBrush| -> alloc::string::String {
        let Ok(stops) = b.ColorStops() else { return Default::default() };
        (&stops)
            .into_iter()
            .map(|s| {
                alloc::format!(
                    " {}@{}",
                    hex(s.Color().unwrap_or_default()),
                    s.Offset().unwrap_or(0.)
                )
            })
            .collect()
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
            let round = |v: f32| (v * 1e4).round() / 1e4 + 0.;
            out += &alloc::format!(
                " m=[{} {} {} {} {} {}]",
                round(m.M11),
                round(m.M12),
                round(m.M21),
                round(m.M22),
                round(m.M31),
                round(m.M32)
            );
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
/// ` [fill <brush> stroke <brush> w<thickness> <geometry>]`, the geometry being
/// `rrect x,y wxh r<radius>` or `path`.
pub(super) fn describe_shapes(
    visual: &windows::UI::Composition::ShapeVisual,
) -> alloc::string::String {
    use core::fmt::Write;
    use windows::UI::Composition::CompositionSpriteShape;
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
                "stroke {} w{}",
                describe_brush(&b),
                shape.StrokeThickness().unwrap_or(0.)
            ));
        }
        parts.push(shape.Geometry().map_or_else(|_| "path".into(), |g| describe_geometry(&g)));
        let _ = write!(out, "{}]", parts.join(" "));
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
