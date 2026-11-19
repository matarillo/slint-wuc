// Copyright © Kentaro Inomata
// SPDX-License-Identifier: MIT-0

//! `BoxShadow` items: upstream's shadow textures, drawn with Direct2D into a composition drawing
//! surface (`docs/design/visual-tree.md`, "Shadows").

#![allow(unsafe_code)]

use super::VisualUpdate;
use super::graphics::{Graphics, MAX_SURFACE_SIDE, d2d_brush, d2d_color, graphics, sprite_surface};
use super::shapes::{add_rounded_rect, d2d_factory, rounded_rect_path};
use crate::graphics::boxshadow::BoxShadowOptions;
use crate::item_tree::ItemRc;
use crate::items::BoxShadow;
use crate::lengths::{PhysicalPx, ScaleFactor};
use crate::window::WindowInner;
use alloc::vec::Vec;
use core::pin::Pin;
use windows::UI::Composition::SpriteVisual;
use windows::Win32::Foundation::{POINT, SIZE};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_COLOR_F, D2D1_COMPOSITE_MODE_SOURCE_IN, D2D1_COMPOSITE_MODE_SOURCE_OVER,
    D2D1_FIGURE_BEGIN_FILLED, D2D1_FIGURE_END_CLOSED, D2D1_FILL_MODE_ALTERNATE,
};
use windows::Win32::Graphics::Direct2D::{
    CLSID_D2D1Composite, CLSID_D2D1Shadow, D2D1_COMPOSITE_PROP_MODE,
    D2D1_INTERPOLATION_MODE_LINEAR, D2D1_PROPERTY_TYPE_ENUM, D2D1_PROPERTY_TYPE_FLOAT,
    D2D1_PROPERTY_TYPE_VECTOR4, D2D1_SHADOW_PROP_BLUR_STANDARD_DEVIATION, D2D1_SHADOW_PROP_COLOR,
    ID2D1CommandList, ID2D1DeviceContext, ID2D1Geometry, ID2D1Image,
};
use windows::Win32::System::WinRT::Composition::ICompositionDrawingSurfaceInterop;
use windows::core::{Interface, Result};
use windows_numerics::{Matrix3x2, Vector2, Vector3};

type PhysicalRect = euclid::Rect<f32, PhysicalPx>;

/// Paints into the current target of a device context.
type Painter<'a> = &'a dyn Fn(&ID2D1DeviceContext) -> Result<()>;

/// Shows a `BoxShadow`.
pub fn update_box_shadow(visual: &mut VisualUpdate<'_>, item_rc: &ItemRc, shadow: Pin<&BoxShadow>) {
    visual.plain();
    let Some(adapter) = item_rc.window_adapter() else {
        visual.remove_floor();
        return;
    };
    // Read through the tracker: a new scale factor draws the shadow again.
    let scale = WindowInner::from_pub(adapter.window()).scale_factor();
    let Some(graphics) = graphics(visual.compositor()) else { return };
    graphics.track_device();

    let Some(options) = BoxShadowOptions::new(item_rc, shadow, ScaleFactor::new(scale)) else {
        visual.remove_floor();
        return;
    };
    let Some((origin, size)) = texture(&options, shadow, scale) else {
        visual.remove_floor();
        return;
    };
    if size.cx as f32 > MAX_SURFACE_SIDE || size.cy as f32 > MAX_SURFACE_SIDE {
        crate::debug_log!("a box shadow of {}x{} pixels is too large to draw", size.cx, size.cy);
        visual.remove_floor();
        return;
    }

    let sprite = visual.surface_sprite();
    // Upstream draws the texture from a whole-pixel origin, as it does images.
    let _ = sprite.SetIsPixelSnappingEnabled(true);
    let _ = sprite.SetOffset(Vector3 { X: origin.X / scale, Y: origin.Y / scale, Z: 0. });
    let _ = sprite.SetSize(Vector2 { X: size.cx as f32 / scale, Y: size.cy as f32 / scale });
    if let Err(e) = draw(&graphics, &sprite, &options, size, scale) {
        graphics.drawing_failed(&e, "a box shadow");
    }
}

/// Where the shadow's texture goes relative to the item, in physical pixels, and its size in
/// whole pixels; `None` if there is no shadow to draw.
fn texture(
    options: &BoxShadowOptions,
    shadow: Pin<&BoxShadow>,
    scale: f32,
) -> Option<(Vector2, SIZE)> {
    if options.inset {
        // An inset shadow is as large as the item.
        if options.width.get() < 1. || options.height.get() < 1. {
            return None;
        }
        let size =
            SIZE { cx: options.width.get().ceil() as i32, cy: options.height.get().ceil() as i32 };
        return Some((Vector2::default(), size));
    }
    if options.shape_size().is_empty() {
        return None;
    }
    let size = options.drop_texture_size();
    let size = SIZE { cx: size.width.ceil() as i32, cy: size.height.ceil() as i32 };
    // The shape is the item grown by the spread, in a texture padded by the blur.
    let pad = options.blur.get() + options.spread.get();
    let origin = Vector2 {
        X: shadow.offset_x().get() as f32 * scale - pad,
        Y: shadow.offset_y().get() as f32 * scale - pad,
    };
    Some((origin, size))
}

/// Draws the shadow into the sprite's surface, of `size` pixels.
fn draw(
    graphics: &Graphics,
    sprite: &SpriteVisual,
    options: &BoxShadowOptions,
    size: SIZE,
    scale: f32,
) -> Result<()> {
    let surface = sprite_surface(graphics, sprite, size)?;
    let interop: ICompositionDrawingSurfaceInterop = surface.cast()?;
    let mut offset = POINT::default();
    let dc: ID2D1DeviceContext = unsafe { interop.BeginDraw(None, &mut offset)? };
    let base = Matrix3x2::translation(offset.x as f32, offset.y as f32);
    let result = (|| unsafe {
        dc.SetDpi(96., 96.);
        dc.SetTransform(&base);
        dc.Clear(Some(&D2D1_COLOR_F::default()));
        if options.inset {
            draw_inset(&dc, base, options)
        } else {
            draw_drop(&dc, base, options, scale)
        }
    })();
    let end = unsafe { interop.EndDraw() };
    result.and(end)
}

/// A drop shadow: the item's shape grown by the spread, blurred, in the shadow's color.
/// The shape is the item's rounded rectangle if its background is opaque, else what the
/// background and the border paint.
fn draw_drop(
    dc: &ID2D1DeviceContext,
    base: Matrix3x2,
    options: &BoxShadowOptions,
    scale: f32,
) -> Result<()> {
    if let Some(radius) = options.opaque_source_radius() {
        let rect = PhysicalRect::new(options.shape_origin(), options.shape_size());
        let geometry = rounded_rect_path(rect, radius.spread(options.spread.get()))?;
        return shadow_of(dc, base, options, &|dc| fill_black(dc, &geometry), None);
    }
    let Some((background, layout)) = options.source.as_ref() else { return Ok(()) };
    // The layout is relative to the item, which is the blur and the spread into the texture.
    let pad = options.blur.get() + options.spread.get();
    shadow_of(
        dc,
        base,
        options,
        &|dc| unsafe {
            dc.SetTransform(&Matrix3x2::translation(pad, pad));
            if !layout.background_rect.is_empty()
                && let Some(brush) = d2d_brush(dc, background, layout.brush_size, scale)
            {
                let geometry = rounded_rect_path(layout.background_rect, layout.background_radius)?;
                dc.FillGeometry(&geometry, &brush, None);
            }
            if layout.border_width.get() > 0.
                && let Some(brush) = d2d_brush(dc, &layout.border_color, layout.brush_size, scale)
            {
                let geometry = rounded_rect_path(layout.border_rect, layout.border_radius)?;
                dc.DrawGeometry(&geometry, &brush, layout.border_width.get(), None);
            }
            Ok(())
        },
        None,
    )
}

/// An inset shadow: a ring around a hole, which is the item's rounded rectangle shrunk by the
/// spread and moved by the offset, blurred, and cut to the item.
fn draw_inset(dc: &ID2D1DeviceContext, base: Matrix3x2, options: &BoxShadowOptions) -> Result<()> {
    let (width, height) = (options.width.get(), options.height.get());
    let spread = options.spread.get();
    let hole = PhysicalRect::new(
        euclid::point2(spread + options.offset_x_inset, spread + options.offset_y_inset),
        euclid::size2(width - 2. * spread, height - 2. * spread),
    );
    // Large enough that the blurred outer edge falls outside the item.
    let inflate = options.blur.get()
        + spread.abs()
        + options.offset_x_inset.abs()
        + options.offset_y_inset.abs()
        + 16.;
    let ring = unsafe {
        let ring = d2d_factory()?.CreatePathGeometry()?;
        let sink = ring.Open()?;
        sink.SetFillMode(D2D1_FILL_MODE_ALTERNATE);
        let p = |x, y| Vector2 { X: x, Y: y };
        sink.BeginFigure(p(-inflate, -inflate), D2D1_FIGURE_BEGIN_FILLED);
        sink.AddLine(p(width + inflate, -inflate));
        sink.AddLine(p(width + inflate, height + inflate));
        sink.AddLine(p(-inflate, height + inflate));
        sink.EndFigure(D2D1_FIGURE_END_CLOSED);
        if hole.width() > 0. && hole.height() > 0. {
            add_rounded_rect(&sink, hole, options.inner_radius());
        }
        sink.Close()?;
        ring.cast::<ID2D1Geometry>()?
    };
    let item = rounded_rect_path(
        PhysicalRect::new(euclid::point2(0., 0.), euclid::size2(width, height)),
        options.radius,
    )?;
    shadow_of(dc, base, options, &|dc| fill_black(dc, &ring), Some(&|dc| fill_black(dc, &item)))
}

fn fill_black(dc: &ID2D1DeviceContext, geometry: &ID2D1Geometry) -> Result<()> {
    unsafe {
        let brush = dc.CreateSolidColorBrush(&D2D1_COLOR_F { r: 0., g: 0., b: 0., a: 1. }, None)?;
        dc.FillGeometry(geometry, &brush, None);
    }
    Ok(())
}

/// Draws what `silhouette` paints as the shadow: its alpha blurred by the shadow's sigma, in the
/// shadow's color, and cut to where `mask` paints, if there is one. `base` is the transform of
/// the surface.
fn shadow_of(
    dc: &ID2D1DeviceContext,
    base: Matrix3x2,
    options: &BoxShadowOptions,
    silhouette: Painter<'_>,
    mask: Option<Painter<'_>>,
) -> Result<()> {
    unsafe {
        let shape = record(dc, base, silhouette)?;
        let shadow = dc.CreateEffect(&CLSID_D2D1Shadow)?;
        shadow.SetInput(0, &shape, true);
        let sigma = if options.blur.get() > 0. { options.blur_sigma() } else { 0. };
        shadow.SetValue(
            D2D1_SHADOW_PROP_BLUR_STANDARD_DEVIATION.0 as u32,
            D2D1_PROPERTY_TYPE_FLOAT,
            &sigma.to_le_bytes(),
        )?;
        let c = d2d_color(options.color);
        let color: Vec<u8> = [c.r, c.g, c.b, c.a].iter().flat_map(|v| v.to_le_bytes()).collect();
        shadow.SetValue(D2D1_SHADOW_PROP_COLOR.0 as u32, D2D1_PROPERTY_TYPE_VECTOR4, &color)?;
        let mut image: ID2D1Image = shadow.GetOutput()?;
        if let Some(mask) = mask {
            // Source in: the shadow shows where the mask, the destination, is opaque.
            let mask = record(dc, base, mask)?;
            let composite = dc.CreateEffect(&CLSID_D2D1Composite)?;
            composite.SetInput(0, &mask, true);
            composite.SetInput(1, &image, true);
            composite.SetValue(
                D2D1_COMPOSITE_PROP_MODE.0 as u32,
                D2D1_PROPERTY_TYPE_ENUM,
                &(D2D1_COMPOSITE_MODE_SOURCE_IN.0 as u32).to_le_bytes(),
            )?;
            image = composite.GetOutput()?;
        }
        dc.DrawImage(
            &image,
            None,
            None,
            D2D1_INTERPOLATION_MODE_LINEAR,
            D2D1_COMPOSITE_MODE_SOURCE_OVER,
        );
    }
    Ok(())
}

/// What `paint` draws, recorded in the pixels of the shadow's texture.
fn record(
    dc: &ID2D1DeviceContext,
    base: Matrix3x2,
    paint: Painter<'_>,
) -> Result<ID2D1CommandList> {
    unsafe {
        let target = dc.GetTarget()?;
        let list = dc.CreateCommandList()?;
        dc.SetTarget(&list);
        dc.SetTransform(&Matrix3x2::identity());
        let result = paint(dc);
        dc.SetTarget(&target);
        dc.SetTransform(&base);
        result?;
        list.Close()?;
        Ok(list)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Color;
    use crate::item_tree::ItemTreeVTable;
    use crate::lengths::LogicalLength;
    use crate::testing::BoxShadowTree;
    use vtable::VRc;

    /// Where the texture of the root item's shadow goes at `scale`, and its size.
    fn placement(
        tree: &VRc<ItemTreeVTable, BoxShadowTree>,
        scale: f32,
    ) -> Option<((f32, f32), (i32, i32))> {
        let item_rc = ItemRc::new(VRc::into_dyn(tree.clone()), 0);
        let shadow = tree.rect(0);
        let options = BoxShadowOptions::new(&item_rc, shadow, ScaleFactor::new(scale))?;
        let (origin, size) = texture(&options, shadow, scale)?;
        Some(((origin.X, origin.Y), (size.cx, size.cy)))
    }

    fn shadow_tree() -> VRc<ItemTreeVTable, BoxShadowTree> {
        let tree = BoxShadowTree::new((100., 50.), 0);
        tree.rect(0).color.set(Color::from_argb_u8(128, 0, 0, 0));
        tree
    }

    #[test]
    fn a_drop_shadow_texture_is_padded_by_blur_and_spread_and_moved_by_the_offset() {
        let tree = shadow_tree();
        let shadow = tree.rect(0);
        shadow.blur.set(LogicalLength::new(4.));
        shadow.spread.set(LogicalLength::new(2.));
        shadow.offset_x.set(LogicalLength::new(3.));
        shadow.offset_y.set(LogicalLength::new(5.));
        // At 1.5: the item is 150x75, blur 6, spread 3, and the offset (4.5, 7.5). The texture
        // is the item grown by the spread, and by the blur, on each side.
        assert_eq!(placement(&tree, 1.5), Some(((4.5 - 9., 7.5 - 9.), (168, 93))));
        assert_eq!(placement(&tree, 1.), Some(((3. - 6., 5. - 6.), (112, 62))));

        // A spread that leaves nothing of the shape leaves no texture.
        shadow.spread.set(LogicalLength::new(-30.));
        assert_eq!(placement(&tree, 1.), None);
    }

    #[test]
    fn an_inset_shadow_texture_covers_the_item() {
        let tree = shadow_tree();
        let shadow = tree.rect(0);
        shadow.inset.set(true);
        shadow.blur.set(LogicalLength::new(4.));
        shadow.offset_x.set(LogicalLength::new(3.));
        assert_eq!(placement(&tree, 1.), Some(((0., 0.), (100, 50))));
        assert_eq!(placement(&tree, 1.25), Some(((0., 0.), (125, 63))));

        // An item under a pixel wide has none, and neither has a transparent shadow.
        tree.geometry(0)
            .set(crate::lengths::LogicalRect::new(Default::default(), euclid::size2(0.5, 50.)));
        assert_eq!(placement(&tree, 1.), None);
    }

    #[test]
    fn a_transparent_shadow_has_no_texture() {
        let tree = shadow_tree();
        tree.rect(0).color.set(Color::from_argb_u8(0, 0, 0, 0));
        assert_eq!(placement(&tree, 1.), None);
    }
}
