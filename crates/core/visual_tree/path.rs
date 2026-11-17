// SPDX-License-Identifier: MIT OR Apache-2.0

//! `Path` items: the fitted path as a Direct2D geometry in one composition sprite shape
//! (`docs/design/visual-tree.md`, "Paths").

#![allow(unsafe_code)]

use super::{VisualUpdate, shapes};
use crate::items::{FillRule, ItemRc, LineCap, LineJoin, Path};
use crate::lengths::LogicalSize;
use crate::window::WindowInner;
use core::pin::Pin;
use lyon_path::Event;
use lyon_path::math::Point;
use windows::UI::Composition::{
    CompositionBorderMode, CompositionBrush, CompositionClip, CompositionGradientBrush,
    CompositionSpriteShape, CompositionStrokeCap, CompositionStrokeLineJoin,
};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_BEZIER_SEGMENT, D2D1_FIGURE_BEGIN_FILLED, D2D1_FIGURE_END_CLOSED, D2D1_FIGURE_END_OPEN,
    D2D1_FILL_MODE_ALTERNATE, D2D1_FILL_MODE_WINDING,
};
use windows::Win32::Graphics::Direct2D::{D2D1_QUADRATIC_BEZIER_SEGMENT, ID2D1Geometry};
use windows::core::Interface;
use windows_numerics::{Matrix3x2, Vector2};

/// Shows a `Path`.
pub fn update_path(
    visual: &mut VisualUpdate<'_>,
    item_rc: &ItemRc,
    path: Pin<&Path>,
    size: LogicalSize,
) {
    visual.plain();
    match update(visual, item_rc, path, size) {
        Ok(true) => {}
        Ok(false) => visual.remove_floor(),
        Err(e) => {
            visual.remove_floor();
            crate::debug_log!("cannot show a path: {}", e.message());
        }
    }
}

/// Fills the floor with the path's shape; `false` if there is nothing to show.
fn update(
    visual: &mut VisualUpdate<'_>,
    item_rc: &ItemRc,
    path: Pin<&Path>,
    size: LogicalSize,
) -> windows::core::Result<bool> {
    let Some((offset, events)) = path.fitted_path_events(item_rc) else { return Ok(false) };
    let compositor = visual.compositor().clone();
    let size_f = size.cast::<f32>().cast_unit();

    // A gradient fill spans the viewbox when there is one, scaled to the item.
    let (viewbox_width, viewbox_height) = (path.viewbox_width(), path.viewbox_height());
    let fill = if viewbox_width > 0. && viewbox_height > 0. {
        let brush = shapes::make_brush(
            &compositor,
            &path.fill(),
            euclid::size2(viewbox_width, viewbox_height),
        );
        if let Some(gradient) =
            brush.as_ref().and_then(|b| b.cast::<CompositionGradientBrush>().ok())
        {
            gradient.SetTransformMatrix(Matrix3x2::scale(
                size_f.width / viewbox_width,
                size_f.height / viewbox_height,
            ))?;
        }
        brush
    } else {
        shapes::make_brush(&compositor, &path.fill(), size_f)
    };
    let stroke = shapes::make_brush(&compositor, &path.stroke(), size_f);
    if fill.is_none() && stroke.is_none() {
        return Ok(false);
    }

    let Some(geometry) = geometry(events.iter(), path.fill_rule())? else { return Ok(false) };
    let bounds = unsafe { geometry.GetBounds(None)? };
    let geometry = shapes::path_geometry(&compositor, geometry)?;
    let shape = compositor.CreateSpriteShapeWithGeometry(&geometry)?;
    if let Some(brush) = &fill {
        shape.SetFillBrush(brush)?;
    }
    let mut reach = 0.;
    if let Some(brush) = &stroke {
        reach = stroke_style(&shape, brush, item_rc, path)?;
    }

    // Composition cuts shapes off at the bounds of the floor: it has to cover what the path
    // draws outside the item, unless `clip` cuts it off at the item anyway.
    let item = euclid::Rect::from_size(size.cast::<f32>());
    let area = if path.clip() {
        item
    } else {
        let drawn = euclid::Box2D::new(
            euclid::point2(bounds.left + offset.x - reach, bounds.top + offset.y - reach),
            euclid::point2(bounds.right + offset.x + reach, bounds.bottom + offset.y + reach),
        );
        item.union(&drawn.to_rect())
    };
    let (floor, origin) = visual.shape_covering(area);
    floor.SetBorderMode(if path.anti_alias() {
        CompositionBorderMode::Soft
    } else {
        CompositionBorderMode::Hard
    })?;
    let clip: Option<CompositionClip> = if path.clip() {
        Some(compositor.CreateInsetClipWithInsets(origin.X, origin.Y, origin.X, origin.Y)?.cast()?)
    } else {
        None
    };
    floor.SetClip(clip.as_ref())?;
    // The gradients of the shape start at the fit's offset, as upstream's do.
    shape.SetOffset(Vector2 { X: origin.X + offset.x, Y: origin.Y + offset.y })?;
    let shapes = floor.Shapes()?;
    shapes.Clear()?;
    shapes.Append(&shape)?;
    Ok(true)
}

/// Sets the path's stroke: brush, width, caps, joins and dashes. Returns how far the stroke
/// reaches beyond the geometry.
fn stroke_style(
    shape: &CompositionSpriteShape,
    brush: &CompositionBrush,
    item_rc: &ItemRc,
    path: Pin<&Path>,
) -> windows::core::Result<f32> {
    shape.SetStrokeBrush(brush)?;
    // Skia draws a stroke of width 0 as a line one pixel wide.
    let mut thickness = path.stroke_width().get() as f32;
    if !(thickness > 0.) {
        let scale = item_rc
            .window_adapter()
            .map_or(1., |adapter| WindowInner::from_pub(adapter.window()).scale_factor());
        thickness = 1. / scale;
    }
    shape.SetStrokeThickness(thickness)?;

    let cap = match path.stroke_line_cap() {
        LineCap::Round => CompositionStrokeCap::Round,
        LineCap::Square => CompositionStrokeCap::Square,
        _ => CompositionStrokeCap::Flat,
    };
    shape.SetStrokeStartCap(cap)?;
    shape.SetStrokeEndCap(cap)?;
    shape.SetStrokeDashCap(cap)?;
    // Skia's miter joins turn into bevels beyond the miter limit.
    let miter_limit = path.stroke_miter_limit().max(1.);
    let join = match path.stroke_line_join() {
        LineJoin::Round => CompositionStrokeLineJoin::Round,
        LineJoin::Bevel => CompositionStrokeLineJoin::Bevel,
        _ => CompositionStrokeLineJoin::MiterOrBevel,
    };
    shape.SetStrokeLineJoin(join)?;
    shape.SetStrokeMiterLimit(miter_limit)?;

    let dashes = path.stroke_dash_array();
    if let Some(pattern) = dash_pattern(&dashes) {
        // Both take logical pixels: Composition doesn't scale the dashes by the stroke width.
        let list = shape.StrokeDashArray()?;
        for length in pattern {
            list.Append(*length)?;
        }
        // Skia wraps the phase around the pattern.
        let length: f32 = pattern.iter().sum();
        let offset = path.stroke_dash_offset().get() as f32;
        shape.SetStrokeDashOffset(offset.rem_euclid(length))?;
    }
    // A miter reaches `miter_limit` half widths from the vertex, a square cap the corner of a
    // square.
    let reach = if join == CompositionStrokeLineJoin::MiterOrBevel { miter_limit } else { 1. };
    Ok(thickness / 2. * reach.max(core::f32::consts::SQRT_2))
}

/// The lengths of a dash pattern, or `None` for a solid stroke. Skia ignores a pattern that
/// has an odd number of lengths, a negative length, or no length at all.
fn dash_pattern(lengths: &[crate::Coord]) -> Option<&[crate::Coord]> {
    let valid = lengths.len() >= 2
        && lengths.len() % 2 == 0
        && lengths.iter().all(|l| l.is_finite() && *l >= 0.)
        && lengths.iter().sum::<crate::Coord>() > 0.;
    valid.then_some(lengths)
}

/// The path's events as a Direct2D geometry in the path's coordinates, or `None` if it has no
/// figure.
fn geometry(
    events: impl Iterator<Item = Event<Point, Point>>,
    rule: FillRule,
) -> windows::core::Result<Option<ID2D1Geometry>> {
    let point = |p: Point| Vector2 { X: p.x, Y: p.y };
    let geometry = shapes::d2d_factory()?;
    unsafe {
        let geometry = geometry.CreatePathGeometry()?;
        let sink = geometry.Open()?;
        sink.SetFillMode(match rule {
            FillRule::Evenodd => D2D1_FILL_MODE_ALTERNATE,
            _ => D2D1_FILL_MODE_WINDING,
        });
        let mut figures = 0;
        for event in events {
            match event {
                Event::Begin { at } => {
                    sink.BeginFigure(point(at), D2D1_FIGURE_BEGIN_FILLED);
                    figures += 1;
                }
                Event::Line { to, .. } => sink.AddLine(point(to)),
                Event::Quadratic { ctrl, to, .. } => {
                    sink.AddQuadraticBezier(&D2D1_QUADRATIC_BEZIER_SEGMENT {
                        point1: point(ctrl),
                        point2: point(to),
                    })
                }
                Event::Cubic { ctrl1, ctrl2, to, .. } => sink.AddBezier(&D2D1_BEZIER_SEGMENT {
                    point1: point(ctrl1),
                    point2: point(ctrl2),
                    point3: point(to),
                }),
                // A filled open figure closes implicitly.
                Event::End { close, .. } => sink.EndFigure(if close {
                    D2D1_FIGURE_END_CLOSED
                } else {
                    D2D1_FIGURE_END_OPEN
                }),
            }
        }
        sink.Close()?;
        Ok((figures > 0).then(|| geometry.cast()).transpose()?)
    }
}
