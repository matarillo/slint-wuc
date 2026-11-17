// SPDX-License-Identifier: MIT OR Apache-2.0

//! Text items: upstream's parley layout, recorded as a display list and drawn with Direct2D into
//! a composition drawing surface (`docs/design/visual-tree.md`, "Text").

#![allow(unsafe_code)]

use super::VisualUpdate;
use super::graphics::{Graphics, graphics};
use crate::graphics::{Brush, Color, GradientStop, ResolvedBrush, resolve_brush};
use crate::item_rendering::{
    CachedRenderingData, ItemRenderer, RenderBorderRectangle, RenderImage, RenderRectangle,
    RenderText,
};
use crate::item_tree::ItemRc;
use crate::items::{BoxShadow, Path, TextInput};
use crate::lengths::{
    LogicalBorderRadius, LogicalPoint, LogicalRect, LogicalSize, LogicalVector, PhysicalPx,
    ScaleFactor,
};
use crate::textlayout::sharedparley::{
    self, GlyphRenderer, PhysicalLength, PhysicalRect, RectangleBorder, fontique, parley,
};
use crate::window::WindowInner;
use alloc::vec::Vec;
use core::pin::Pin;
use windows::Foundation::Size;
use windows::Graphics::DirectX::{DirectXAlphaMode, DirectXPixelFormat};
use windows::UI::Composition::{
    CompositionDrawingSurface, CompositionStretch, CompositionSurfaceBrush, SpriteVisual,
};
use windows::Win32::Foundation::{D2DERR_RECREATE_TARGET, DWRITE_E_NOCOLOR, POINT, SIZE};
use windows::Win32::Graphics::Direct2D::Common::{D2D_RECT_F, D2D1_COLOR_F, D2D1_GRADIENT_STOP};
use windows::Win32::Graphics::Direct2D::{
    D2D1_ANTIALIAS_MODE_ALIASED, D2D1_COLOR_BITMAP_GLYPH_SNAP_OPTION_DEFAULT,
    D2D1_EXTEND_MODE_CLAMP, D2D1_GAMMA_2_2, D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES,
    D2D1_RADIAL_GRADIENT_BRUSH_PROPERTIES, D2D1_ROUNDED_RECT, D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE,
    ID2D1Brush, ID2D1DeviceContext, ID2D1DeviceContext4, ID2D1DeviceContext7, ID2D1RenderTarget,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_GLYPH_IMAGE_FORMATS_CFF, DWRITE_GLYPH_IMAGE_FORMATS_COLR,
    DWRITE_GLYPH_IMAGE_FORMATS_COLR_PAINT_TREE, DWRITE_GLYPH_IMAGE_FORMATS_JPEG,
    DWRITE_GLYPH_IMAGE_FORMATS_PNG, DWRITE_GLYPH_IMAGE_FORMATS_PREMULTIPLIED_B8G8R8A8,
    DWRITE_GLYPH_IMAGE_FORMATS_SVG, DWRITE_GLYPH_IMAGE_FORMATS_TIFF,
    DWRITE_GLYPH_IMAGE_FORMATS_TRUETYPE, DWRITE_GLYPH_OFFSET, DWRITE_GLYPH_RUN,
    DWRITE_MEASURING_MODE_NATURAL, IDWriteFactory8, IDWriteFontFace,
};
use windows::Win32::Graphics::Dxgi::{DXGI_ERROR_DEVICE_REMOVED, DXGI_ERROR_DEVICE_RESET};
use windows::Win32::System::WinRT::Composition::ICompositionDrawingSurfaceInterop;
use windows::core::{BOOL, Interface};
use windows_numerics::{Matrix3x2, Vector2, Vector3};

type PhysicalSize = euclid::Size2D<f32, PhysicalPx>;
type PhysicalPoint = euclid::Point2D<f32, PhysicalPx>;

/// The largest surface side Direct3D 11 hardware supports.
const MAX_SURFACE_SIDE: f32 = 16384.;

/// Shows a `Text`, `SimpleText`, `ComplexText` or `StyledTextItem`.
pub fn update_text(
    visual: &mut VisualUpdate<'_>,
    item_rc: &ItemRc,
    text: Pin<&dyn RenderText>,
    size: LogicalSize,
) {
    update(visual, item_rc, |recorder, cache| {
        sharedparley::draw_text(recorder, text, Some(item_rc), size, cache);
    });
}

/// Shows a `TextInput`, with its selection, preedit and cursor.
pub fn update_text_input(
    visual: &mut VisualUpdate<'_>,
    item_rc: &ItemRc,
    text_input: Pin<&TextInput>,
    size: LogicalSize,
) {
    update(visual, item_rc, |recorder, cache| {
        if let Some(cache) = cache {
            sharedparley::draw_text_input(recorder, text_input, item_rc, size, cache);
        }
    });
}

fn update(
    visual: &mut VisualUpdate<'_>,
    item_rc: &ItemRc,
    record: impl FnOnce(&mut Recorder<'_>, Option<&sharedparley::TextLayoutCache>),
) {
    visual.plain();
    let Some(adapter) = item_rc.window_adapter() else {
        visual.remove_floor();
        return;
    };
    let window = WindowInner::from_pub(adapter.window());
    // Read through the tracker: a new scale factor redraws the text.
    let scale = window.scale_factor();
    let Some(graphics) = graphics(visual.compositor()) else { return };
    graphics.track_device();

    let mut recorder = Recorder::new(window, ScaleFactor::new(scale));
    record(&mut recorder, adapter.renderer().text_layout_cache());
    let Some(bounds) = recorder.bounds() else {
        visual.remove_floor();
        return;
    };

    let sprite = visual.surface_sprite();
    if let Err(e) = draw(&graphics, &sprite, &recorder.commands, bounds, scale) {
        let code = e.code();
        if code == DXGI_ERROR_DEVICE_REMOVED
            || code == DXGI_ERROR_DEVICE_RESET
            || code == D2DERR_RECREATE_TARGET
        {
            graphics.device_lost();
        } else {
            crate::debug_log!("cannot draw text: {}", e.message());
        }
    }
    let _ =
        sprite.SetOffset(Vector3 { X: bounds.origin.x / scale, Y: bounds.origin.y / scale, Z: 0. });
    let _ = sprite.SetSize(Vector2 { X: bounds.width() / scale, Y: bounds.height() / scale });
}

/// What the recording renderer was asked to draw, in physical pixels relative to the item.
enum Command {
    PushClip(PhysicalRect),
    PopClip,
    Glyphs {
        font: parley::FontData,
        size: f32,
        synthesis: fontique::Synthesis,
        brush: TextBrush,
        glyphs: Vec<parley::layout::Glyph>,
    },
    Rect {
        rect: PhysicalRect,
        brush: TextBrush,
        radius: f32,
        border: Option<(TextBrush, f32)>,
    },
}

/// The recording renderer's brush: the item's brush and the size gradients span, in physical
/// pixels.
#[derive(Clone)]
pub struct TextBrush {
    brush: Brush,
    size: PhysicalSize,
    /// The stroke width, for a brush that strokes the glyph outlines.
    stroke: Option<f32>,
}

#[derive(Clone, Copy)]
struct State {
    /// In the current coordinates, logical.
    clip: LogicalRect,
    /// From the item's origin to the current origin, logical.
    offset: LogicalVector,
    /// The clips pushed since this state was saved.
    clips: usize,
}

/// An `ItemRenderer` and `GlyphRenderer` that records what upstream's text drawing asks for.
pub struct Recorder<'a> {
    window: &'a WindowInner,
    scale: ScaleFactor,
    state: State,
    saved: Vec<State>,
    commands: Vec<Command>,
    bounds: Option<PhysicalRect>,
}

impl<'a> Recorder<'a> {
    fn new(window: &'a WindowInner, scale: ScaleFactor) -> Self {
        // Nothing clips yet; the surface takes the size of what is drawn.
        let huge = 1.0e6;
        Self {
            window,
            scale,
            state: State {
                clip: LogicalRect::new(
                    LogicalPoint::new(-huge, -huge),
                    LogicalSize::new(2. * huge, 2. * huge),
                ),
                offset: LogicalVector::zero(),
                clips: 0,
            },
            saved: Vec::new(),
            commands: Vec::new(),
            bounds: None,
        }
    }

    /// What was drawn, in whole physical pixels, or `None` for nothing.
    fn bounds(&self) -> Option<PhysicalRect> {
        let b = self.bounds?.round_out();
        let size =
            PhysicalSize::new(b.width().min(MAX_SURFACE_SIDE), b.height().min(MAX_SURFACE_SIDE));
        (size.width >= 1. && size.height >= 1.).then(|| PhysicalRect::new(b.origin, size))
    }

    fn physical_offset(&self) -> euclid::Vector2D<f32, PhysicalPx> {
        self.state.offset * self.scale
    }

    fn physical_clip(&self) -> PhysicalRect {
        (self.state.clip.translate(self.state.offset)) * self.scale
    }

    fn grow(&mut self, rect: PhysicalRect) {
        let Some(rect) = rect.intersection(&self.physical_clip()) else { return };
        self.bounds = Some(match self.bounds {
            Some(b) => b.union(&rect),
            None => rect,
        });
    }
}

impl GlyphRenderer for Recorder<'_> {
    type PlatformBrush = TextBrush;

    fn platform_text_fill_brush(&mut self, brush: Brush, size: LogicalSize) -> Option<TextBrush> {
        (!brush.is_transparent()).then(|| TextBrush {
            brush,
            size: size * self.scale,
            stroke: None,
        })
    }

    fn platform_brush_for_color(&mut self, color: &Color) -> Option<TextBrush> {
        (color.alpha() > 0).then(|| TextBrush {
            brush: Brush::SolidColor(*color),
            size: PhysicalSize::zero(),
            stroke: None,
        })
    }

    fn platform_text_stroke_brush(
        &mut self,
        brush: Brush,
        physical_stroke_width: f32,
        size: LogicalSize,
    ) -> Option<TextBrush> {
        (!brush.is_transparent()).then(|| TextBrush {
            brush,
            size: size * self.scale,
            stroke: Some(physical_stroke_width),
        })
    }

    fn draw_glyph_run(
        &mut self,
        font: &parley::FontData,
        font_size: PhysicalLength,
        _normalized_coords: &[i16],
        synthesis: &fontique::Synthesis,
        brush: TextBrush,
        y_offset: PhysicalLength,
        glyphs_it: &mut dyn Iterator<Item = parley::layout::Glyph>,
    ) {
        let offset = self.physical_offset();
        let glyphs: Vec<parley::layout::Glyph> = glyphs_it
            .map(|mut glyph| {
                glyph.x += offset.x;
                glyph.y += y_offset.get() + offset.y;
                glyph
            })
            .collect();
        let size = font_size.get();
        // Ink may reach past the advances (italics, accents, swashes): pad generously.
        let pad = size * 0.5 + brush.stroke.unwrap_or(0.);
        for glyph in &glyphs {
            self.grow(PhysicalRect::new(
                PhysicalPoint::new(glyph.x - pad, glyph.y - size * 1.25 - pad),
                PhysicalSize::new(glyph.advance.max(0.) + 2. * pad, size * 1.75 + 2. * pad),
            ));
        }
        if !glyphs.is_empty() {
            self.commands.push(Command::Glyphs {
                font: font.clone(),
                size,
                synthesis: *synthesis,
                brush,
                glyphs,
            });
        }
    }

    fn fill_rectangle(
        &mut self,
        physical_rect: PhysicalRect,
        brush: TextBrush,
        radius: PhysicalLength,
        border: Option<RectangleBorder<TextBrush>>,
    ) {
        let rect = physical_rect.translate(self.physical_offset());
        self.grow(rect);
        self.commands.push(Command::Rect {
            rect,
            brush,
            radius: radius.get(),
            border: border.map(|b| (b.brush, b.width.get())),
        });
    }
}

impl ItemRenderer for Recorder<'_> {
    fn draw_rectangle(
        &mut self,
        _: Pin<&dyn RenderRectangle>,
        _: &ItemRc,
        _: LogicalSize,
        _: &CachedRenderingData,
    ) {
    }
    fn draw_border_rectangle(
        &mut self,
        _: Pin<&dyn RenderBorderRectangle>,
        _: &ItemRc,
        _: LogicalSize,
        _: &CachedRenderingData,
    ) {
    }
    fn draw_window_background(
        &mut self,
        _: Pin<&dyn RenderRectangle>,
        _: &ItemRc,
        _: LogicalSize,
        _: &CachedRenderingData,
    ) {
    }
    fn draw_image(
        &mut self,
        _: Pin<&dyn RenderImage>,
        _: &ItemRc,
        _: LogicalSize,
        _: &CachedRenderingData,
    ) {
    }
    fn draw_text(
        &mut self,
        _: Pin<&dyn RenderText>,
        _: &ItemRc,
        _: LogicalSize,
        _: &CachedRenderingData,
    ) {
    }
    fn draw_text_input(&mut self, _: Pin<&TextInput>, _: &ItemRc, _: LogicalSize) {}
    fn draw_path(&mut self, _: Pin<&Path>, _: &ItemRc, _: LogicalSize) {}
    fn draw_box_shadow(&mut self, _: Pin<&BoxShadow>, _: &ItemRc, _: LogicalSize) {}

    fn combine_clip(&mut self, rect: LogicalRect, _radius: LogicalBorderRadius) -> bool {
        let clip = self.state.clip.intersection(&rect).unwrap_or_default();
        self.state.clip = clip;
        self.state.clips += 1;
        let physical = self.physical_clip();
        self.commands.push(Command::PushClip(physical));
        !clip.is_empty()
    }

    fn get_current_clip(&self) -> LogicalRect {
        self.state.clip
    }

    fn translate(&mut self, distance: LogicalVector) {
        self.state.offset += distance;
        self.state.clip = self.state.clip.translate(-distance);
    }

    fn rotate(&mut self, _angle_in_degrees: f32) {}
    fn scale(&mut self, _scale_x_factor: f32, _scale_y_factor: f32) {}
    fn apply_opacity(&mut self, _opacity: f32) {}

    fn save_state(&mut self) {
        self.saved.push(self.state);
        self.state.clips = 0;
    }

    fn restore_state(&mut self) {
        for _ in 0..self.state.clips {
            self.commands.push(Command::PopClip);
        }
        if let Some(state) = self.saved.pop() {
            self.state = state;
        }
    }

    fn scale_factor(&self) -> ScaleFactor {
        self.scale
    }

    fn draw_cached_pixmap(&mut self, _: &ItemRc, _: &dyn Fn(&mut dyn FnMut(u32, u32, &[u8]))) {}
    fn draw_string(&mut self, _: &str, _: Color) {}
    fn draw_image_direct(&mut self, _: crate::graphics::Image) {}

    fn window(&self) -> &WindowInner {
        self.window
    }

    fn as_any(&mut self) -> Option<&mut dyn core::any::Any> {
        None
    }
}

/// Draws `commands` into the sprite's surface, which gets the size of `bounds`.
fn draw(
    graphics: &Graphics,
    sprite: &SpriteVisual,
    commands: &[Command],
    bounds: PhysicalRect,
    scale: f32,
) -> windows::core::Result<()> {
    let size = SIZE { cx: bounds.width() as i32, cy: bounds.height() as i32 };
    let surface = match sprite.Brush().and_then(|b| b.cast::<CompositionSurfaceBrush>()) {
        Ok(brush) => {
            let surface: CompositionDrawingSurface = brush.Surface()?.cast()?;
            let current = surface.SizeInt32()?;
            if (current.Width, current.Height) != (size.cx, size.cy) {
                let interop: ICompositionDrawingSurfaceInterop = surface.cast()?;
                unsafe { interop.Resize(size)? };
            }
            surface
        }
        Err(_) => {
            let surface = graphics.composition.CreateDrawingSurface(
                Size { Width: size.cx as f32, Height: size.cy as f32 },
                DirectXPixelFormat::B8G8R8A8UIntNormalized,
                DirectXAlphaMode::Premultiplied,
            )?;
            let compositor = sprite.Compositor()?;
            let brush = compositor.CreateSurfaceBrushWithSurface(&surface)?;
            brush.SetStretch(CompositionStretch::Fill)?;
            // Whole pixels keep the text as sharp as it was drawn (S4).
            brush.SetSnapToPixels(true)?;
            sprite.SetBrush(&brush)?;
            surface
        }
    };

    let interop: ICompositionDrawingSurfaceInterop = surface.cast()?;
    let mut offset = POINT::default();
    let dc: ID2D1DeviceContext = unsafe { interop.BeginDraw(None, &mut offset)? };
    let result = unsafe {
        dc.SetDpi(96., 96.);
        let base = Matrix3x2::translation(
            offset.x as f32 - bounds.origin.x,
            offset.y as f32 - bounds.origin.y,
        );
        dc.SetTransform(&base);
        dc.Clear(Some(&D2D1_COLOR_F::default()));
        dc.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
        replay(graphics, &dc, commands, base, scale)
    };
    let end = unsafe { interop.EndDraw() };
    result.and(end)
}

/// Draws the commands with `dc`, whose transform is `base`.
unsafe fn replay(
    graphics: &Graphics,
    dc: &ID2D1DeviceContext,
    commands: &[Command],
    base: Matrix3x2,
    scale: f32,
) -> windows::core::Result<()> {
    let mut clips = 0usize;
    for command in commands {
        match command {
            Command::PushClip(rect) => unsafe {
                dc.PushAxisAlignedClip(&d2d_rect(*rect), D2D1_ANTIALIAS_MODE_ALIASED);
                clips += 1;
            },
            Command::PopClip => {
                if clips > 0 {
                    unsafe { dc.PopAxisAlignedClip() };
                    clips -= 1;
                }
            }
            Command::Rect { rect, brush, radius, border } => unsafe {
                if let Some(b) = d2d_brush(dc, brush, scale) {
                    if *radius > 0. {
                        dc.FillRoundedRectangle(&rounded(*rect, *radius), &b);
                    } else {
                        dc.FillRectangle(&d2d_rect(*rect), &b);
                    }
                }
                if let Some((brush, width)) = border
                    && let Some(b) = d2d_brush(dc, brush, scale)
                {
                    let inset = rect.inflate(-width / 2., -width / 2.);
                    dc.DrawRoundedRectangle(&rounded(inset, *radius), &b, *width, None);
                }
            },
            Command::Glyphs { font, size, synthesis, brush, glyphs } => unsafe {
                draw_glyphs(graphics, dc, font, *size, synthesis, brush, glyphs, base, scale)?;
            },
        }
    }
    for _ in 0..clips {
        unsafe { dc.PopAxisAlignedClip() };
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
unsafe fn draw_glyphs(
    graphics: &Graphics,
    dc: &ID2D1DeviceContext,
    font: &parley::FontData,
    size: f32,
    synthesis: &fontique::Synthesis,
    brush: &TextBrush,
    glyphs: &[parley::layout::Glyph],
    base: Matrix3x2,
    scale: f32,
) -> windows::core::Result<()> {
    let Some(face) = graphics.face(font, synthesis) else { return Ok(()) };
    let Some(d2d_brush) = d2d_brush(dc, brush, scale) else { return Ok(()) };
    let face: IDWriteFontFace = face.cast()?;
    // The first glyph is the run's origin; the others are offsets from it, with no advances.
    let origin = Vector2 { X: glyphs[0].x, Y: glyphs[0].y };
    let indices: Vec<u16> = glyphs.iter().map(|g| g.id as u16).collect();
    let advances = alloc::vec![0f32; glyphs.len()];
    let offsets: Vec<DWRITE_GLYPH_OFFSET> = glyphs
        .iter()
        .map(|g| DWRITE_GLYPH_OFFSET {
            advanceOffset: g.x - origin.X,
            // Up is positive.
            ascenderOffset: origin.Y - g.y,
        })
        .collect();
    let run = DWRITE_GLYPH_RUN {
        fontFace: core::mem::ManuallyDrop::new(Some(face.clone())),
        fontEmSize: size,
        glyphCount: glyphs.len() as u32,
        glyphIndices: indices.as_ptr(),
        glyphAdvances: advances.as_ptr(),
        glyphOffsets: offsets.as_ptr(),
        isSideways: BOOL(0),
        bidiLevel: 0,
    };

    // Faux italic: lean the glyphs to the right around the baseline, as upstream's renderers do.
    let skewed = synthesis.skew().map(|degrees| {
        let t = degrees.to_radians().tan();
        Matrix3x2 { M11: 1., M12: 0., M21: -t, M22: 1., M31: t * origin.Y, M32: 0. } * base
    });
    unsafe {
        if let Some(m) = skewed {
            dc.SetTransform(&m);
        }
        let result = match brush.stroke {
            Some(width) => {
                stroke_run(dc, &face, &run, origin, &d2d_brush, width, skewed.unwrap_or(base))
            }
            None => fill_run(graphics, dc, &run, origin, &d2d_brush),
        };
        if skewed.is_some() {
            dc.SetTransform(&base);
        }
        let _ = core::mem::ManuallyDrop::into_inner(run.fontFace);
        result
    }
}

/// Fills a glyph run, in color where the font has color glyphs (emoji).
unsafe fn fill_run(
    graphics: &Graphics,
    dc: &ID2D1DeviceContext,
    run: &DWRITE_GLYPH_RUN,
    origin: Vector2,
    brush: &ID2D1Brush,
) -> windows::core::Result<()> {
    let formats = DWRITE_GLYPH_IMAGE_FORMATS_TRUETYPE
        | DWRITE_GLYPH_IMAGE_FORMATS_CFF
        | DWRITE_GLYPH_IMAGE_FORMATS_COLR
        | DWRITE_GLYPH_IMAGE_FORMATS_SVG
        | DWRITE_GLYPH_IMAGE_FORMATS_PNG
        | DWRITE_GLYPH_IMAGE_FORMATS_JPEG
        | DWRITE_GLYPH_IMAGE_FORMATS_TIFF
        | DWRITE_GLYPH_IMAGE_FORMATS_PREMULTIPLIED_B8G8R8A8;
    // COLRv1 glyphs (Segoe UI Emoji on Windows 11) are paint trees, which need the newer
    // interfaces; COLRv0 layers are what the older ones give.
    let dc7: Option<ID2D1DeviceContext7> = dc.cast().ok();
    let factory8: Option<IDWriteFactory8> = graphics.dwrite.cast().ok();
    let colored = unsafe {
        match (&dc7, &factory8) {
            (Some(dc7), Some(factory8)) => factory8.TranslateColorGlyphRun(
                origin,
                run,
                None,
                formats | DWRITE_GLYPH_IMAGE_FORMATS_COLR_PAINT_TREE,
                dc7.GetPaintFeatureLevel(),
                DWRITE_MEASURING_MODE_NATURAL,
                None,
                0,
            ),
            _ => graphics.dwrite.TranslateColorGlyphRun(
                origin,
                run,
                None,
                formats,
                DWRITE_MEASURING_MODE_NATURAL,
                None,
                0,
            ),
        }
    };
    let enumerator = match colored {
        Ok(enumerator) => enumerator,
        Err(e) => {
            if e.code() != DWRITE_E_NOCOLOR {
                crate::debug_log!("cannot draw color glyphs: {}", e.message());
            }
            unsafe { dc.DrawGlyphRun(origin, run, None, brush, DWRITE_MEASURING_MODE_NATURAL) };
            return Ok(());
        }
    };
    let dc4: Option<ID2D1DeviceContext4> = dc.cast().ok();
    while unsafe { enumerator.MoveNext()? }.as_bool() {
        let color_run = unsafe { &*enumerator.GetCurrentRun()? };
        let base = &color_run.Base;
        let at = Vector2 { X: base.baselineOriginX, Y: base.baselineOriginY };
        let format = color_run.glyphImageFormat;
        let mode = color_run.measuringMode;
        unsafe {
            if format == DWRITE_GLYPH_IMAGE_FORMATS_PNG
                || format == DWRITE_GLYPH_IMAGE_FORMATS_JPEG
                || format == DWRITE_GLYPH_IMAGE_FORMATS_TIFF
                || format == DWRITE_GLYPH_IMAGE_FORMATS_PREMULTIPLIED_B8G8R8A8
            {
                if let Some(dc4) = &dc4 {
                    dc4.DrawColorBitmapGlyphRun(
                        format,
                        at,
                        &base.glyphRun,
                        mode,
                        D2D1_COLOR_BITMAP_GLYPH_SNAP_OPTION_DEFAULT,
                    );
                }
            } else if format == DWRITE_GLYPH_IMAGE_FORMATS_COLR_PAINT_TREE {
                if let Some(dc7) = &dc7 {
                    dc7.DrawPaintGlyphRun(at, &base.glyphRun, brush, 0, mode);
                }
            } else if format == DWRITE_GLYPH_IMAGE_FORMATS_SVG {
                if let Some(dc4) = &dc4 {
                    dc4.DrawSvgGlyphRun(at, &base.glyphRun, brush, None, 0, mode);
                }
            } else if format == DWRITE_GLYPH_IMAGE_FORMATS_COLR && base.paletteIndex != 0xffff {
                let c = base.runColor;
                let layer = dc.CreateSolidColorBrush(
                    &D2D1_COLOR_F { r: c.r, g: c.g, b: c.b, a: c.a },
                    None,
                )?;
                dc.DrawGlyphRun(at, &base.glyphRun, None, &layer, mode);
            } else {
                // Monochrome glyphs, and the COLR layers in the text color.
                dc.DrawGlyphRun(at, &base.glyphRun, None, brush, mode);
            }
        }
    }
    Ok(())
}

/// Strokes the outlines of a glyph run.
unsafe fn stroke_run(
    dc: &ID2D1DeviceContext,
    face: &IDWriteFontFace,
    run: &DWRITE_GLYPH_RUN,
    origin: Vector2,
    brush: &ID2D1Brush,
    width: f32,
    transform: Matrix3x2,
) -> windows::core::Result<()> {
    unsafe {
        let factory = dc.GetFactory()?;
        let path = factory.CreatePathGeometry()?;
        let sink = path.Open()?;
        face.GetGlyphRunOutline(
            run.fontEmSize,
            run.glyphIndices,
            Some(run.glyphAdvances),
            Some(run.glyphOffsets),
            run.glyphCount,
            false,
            false,
            &sink,
        )?;
        sink.Close()?;
        // The outline is relative to the run's origin.
        dc.SetTransform(&(Matrix3x2::translation(origin.X, origin.Y) * transform));
        dc.DrawGeometry(&path, brush, width, None);
        dc.SetTransform(&transform);
    }
    Ok(())
}

fn d2d_rect(r: PhysicalRect) -> D2D_RECT_F {
    D2D_RECT_F { left: r.min_x(), top: r.min_y(), right: r.max_x(), bottom: r.max_y() }
}

fn rounded(r: PhysicalRect, radius: f32) -> D2D1_ROUNDED_RECT {
    D2D1_ROUNDED_RECT { rect: d2d_rect(r), radiusX: radius, radiusY: radius }
}

fn d2d_color(c: Color) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: c.red() as f32 / 255.,
        g: c.green() as f32 / 255.,
        b: c.blue() as f32 / 255.,
        a: c.alpha() as f32 / 255.,
    }
}

/// A Direct2D brush for a text brush; gradients span the item, as in upstream's renderers.
fn d2d_brush(dc: &ID2D1DeviceContext, brush: &TextBrush, scale: f32) -> Option<ID2D1Brush> {
    let resolved = resolve_brush(&brush.brush, brush.size, ScaleFactor::new(scale))?;
    let stops = |stops: &[GradientStop]| -> windows::core::Result<_> {
        let stops: Vec<D2D1_GRADIENT_STOP> = stops
            .iter()
            .map(|s| D2D1_GRADIENT_STOP { position: s.position, color: d2d_color(s.color) })
            .collect();
        // Interpolated in sRGB, as upstream's renderers do.
        unsafe {
            ID2D1RenderTarget::CreateGradientStopCollection(
                dc,
                &stops,
                D2D1_GAMMA_2_2,
                D2D1_EXTEND_MODE_CLAMP,
            )
        }
    };
    let point = |p: PhysicalPoint| Vector2 { X: p.x, Y: p.y };
    let result: windows::core::Result<ID2D1Brush> = (|| unsafe {
        match resolved {
            ResolvedBrush::SolidColor(color) => {
                dc.CreateSolidColorBrush(&d2d_color(color), None)?.cast()
            }
            ResolvedBrush::LinearGradient(g) => dc
                .CreateLinearGradientBrush(
                    &D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                        startPoint: point(g.start),
                        endPoint: point(g.end),
                    },
                    None,
                    &stops(&g.stops)?,
                )?
                .cast(),
            ResolvedBrush::RadialGradient(g) => dc
                .CreateRadialGradientBrush(
                    &D2D1_RADIAL_GRADIENT_BRUSH_PROPERTIES {
                        center: point(g.center),
                        gradientOriginOffset: Vector2::default(),
                        radiusX: g.radius_x.get(),
                        radiusY: g.radius_y.get(),
                    },
                    None,
                    &stops(&g.stops)?,
                )?
                .cast(),
            // TODO(phase 5): conic gradients, with the other surface drawing.
            ResolvedBrush::ConicGradient(g) => dc
                .CreateSolidColorBrush(
                    &d2d_color(g.stops.first().map(|s| s.color).unwrap_or_default()),
                    None,
                )?
                .cast(),
        }
    })();
    result.ok()
}
