// SPDX-License-Identifier: MIT OR Apache-2.0

//! Image items: the image's pixels in a composition drawing surface that every item showing
//! the image shares, placed by upstream's `fit()` / `fit9slice()` (`docs/design/visual-tree.md`,
//! "Images").

#![allow(unsafe_code)]

use super::VisualUpdate;
use super::effects::{self, Effect};
use super::graphics::{Graphics, graphics};
use crate::graphics::{
    FitResult, Image, ImageInner, IntRect, IntSize, SharedImageBuffer, fit, fit9slice,
    scalable_render_size,
};
use crate::item_rendering::RenderImage;
use crate::item_tree::ItemRc;
use crate::items::{ImageFit, ImageRendering, ImageTiling};
use crate::lengths::{LogicalSize, PhysicalPx, ScaleFactor};
use crate::window::WindowInner;
use alloc::boxed::Box;
use alloc::vec::Vec;
use core::cell::RefCell;
use core::pin::Pin;
use std::collections::HashMap;
use windows::Foundation::Size;
use windows::Graphics::DirectX::{DirectXAlphaMode, DirectXPixelFormat};
use windows::UI::Composition::{
    CompositionBitmapInterpolationMode, CompositionBrush, CompositionDrawingSurface,
    CompositionGradientBrush, CompositionStretch, CompositionSurfaceBrush, Compositor,
    SpriteVisual,
};
use windows::Win32::Foundation::{D2DERR_RECREATE_TARGET, POINT};
use windows::Win32::Graphics::Direct2D::Common::D2D_RECT_U;
use windows::Win32::Graphics::Direct2D::{ID2D1Bitmap, ID2D1DeviceContext};
use windows::Win32::Graphics::Dxgi::{DXGI_ERROR_DEVICE_REMOVED, DXGI_ERROR_DEVICE_RESET};
use windows::Win32::System::WinRT::Composition::ICompositionDrawingSurfaceInterop;
use windows::core::Interface;
use windows_numerics::{Matrix3x2, Vector2, Vector3};

/// Shows an `Image` or `ClippedImage`.
pub fn update_image(
    visual: &mut VisualUpdate<'_>,
    item_rc: &ItemRc,
    image: Pin<&dyn RenderImage>,
    size: LogicalSize,
) {
    visual.plain();
    if let Err(e) = update(visual, item_rc, image, size) {
        let code = e.code();
        if code == DXGI_ERROR_DEVICE_REMOVED
            || code == DXGI_ERROR_DEVICE_RESET
            || code == D2DERR_RECREATE_TARGET
        {
            if let Some(graphics) = graphics(visual.compositor()) {
                graphics.device_lost();
            }
        } else {
            crate::debug_log!("cannot show an image: {}", e.message());
        }
    }
}

fn update(
    visual: &mut VisualUpdate<'_>,
    item_rc: &ItemRc,
    image: Pin<&dyn RenderImage>,
    size: LogicalSize,
) -> windows::core::Result<()> {
    let Some(adapter) = item_rc.window_adapter() else {
        visual.remove_floor();
        return Ok(());
    };
    // Read through the tracker: a new scale factor fits the image again.
    let scale = WindowInner::from_pub(adapter.window()).scale_factor();
    let scale_factor = ScaleFactor::new(scale);
    let Some(graphics) = graphics(visual.compositor()) else { return Ok(()) };
    graphics.track_device();

    let source = image.source();
    let source_size = source.size();
    let target = size.cast::<f32>() * scale_factor;
    let tiling = image.tiling();
    let fits = fits(image, &source, target, scale_factor);
    // A scalable image is rasterized at the size it's shown at; others at their own size.
    let raster = if <&ImageInner>::from(&source).is_svg() {
        let image_fit = if tiling != Default::default() { ImageFit::Preserve } else { image.image_fit() };
        scalable_render_size(source_size, image_fit, target, scale_factor, tiling)
            .map(|s| svg_raster(source_size, IntSize::new(s.width, s.height)))
    } else {
        Some(source_size)
    };
    let Some(raster) = raster.filter(|r| !r.is_empty() && !fits.is_empty()) else {
        visual.remove_floor();
        return Ok(());
    };
    // Raster pixels per source pixel.
    let ratio = Vector2 {
        X: raster.width as f32 / source_size.width as f32,
        Y: raster.height as f32 / source_size.height as f32,
    };

    let compositor = visual.compositor().clone();
    let colorize = image.colorize();
    let single = fits.len() == 1;
    let sprites = sprites(visual, fits.len());
    for (fit, sprite) in fits.iter().zip(&sprites) {
        // In physical pixels: where the sprite is, and where the part is in it. The pixel-snapped
        // floor sprite stays whole pixels from the item's origin, so a single part's fractional
        // offset moves the content within it, as upstream draws it. More parts show in children
        // of the floor sprite, at their offsets.
        let (origin, inset) = if single {
            let origin = (fit.offset.x.min(0.).floor(), fit.offset.y.min(0.).floor());
            (origin, (fit.offset.x - origin.0, fit.offset.y - origin.1))
        } else {
            ((fit.offset.x, fit.offset.y), (0., 0.))
        };
        let extent = ((inset.0 + fit.size.width).ceil(), (inset.1 + fit.size.height).ceil());
        sprite.SetOffset(Vector3 { X: origin.0 / scale, Y: origin.1 / scale, Z: 0. })?;
        sprite.SetSize(Vector2 { X: extent.0 / scale, Y: extent.1 / scale })?;
        if single {
            let clip = compositor.CreateInsetClipWithInsets(
                inset.0 / scale,
                inset.1 / scale,
                (extent.0 - inset.0 - fit.size.width) / scale,
                (extent.1 - inset.1 - fit.size.height) / scale,
            )?;
            sprite.SetClip(&clip)?;
        }
        // Upstream draws tiles from the part's offset cut to whole pixels (`as i32`).
        let shift = match fit.tiled {
            Some(_) => (fit.offset.x.trunc() - origin.0, fit.offset.y.trunc() - origin.1),
            None => inset,
        };
        let raster_scale = Matrix3x2::scale(
            fit.source_to_target_x / (ratio.X * scale),
            fit.source_to_target_y / (ratio.Y * scale),
        ) * Matrix3x2::translation(shift.0 / scale, shift.1 / scale);
        // Raster pixels of the whole image; a tile is cut out of it into a surface of its own.
        let clip: IntRect = euclid::rect(
            (fit.clip_rect.origin.x as f32 * ratio.X) as i32,
            (fit.clip_rect.origin.y as f32 * ratio.Y) as i32,
            (fit.clip_rect.size.width as f32 * ratio.X).ceil() as i32,
            (fit.clip_rect.size.height as f32 * ratio.Y).ceil() as i32,
        );
        let whole = IntRect::from_size(raster.cast());
        let (part, transform) = match fit.tiled {
            Some(tiled) => (
                clip,
                Matrix3x2::translation(-(tiled.x as f32), -(tiled.y as f32)) * raster_scale,
            ),
            None => (
                whole,
                Matrix3x2::translation(-(clip.origin.x as f32), -(clip.origin.y as f32))
                    * raster_scale,
            ),
        };
        let Some(surface) = surface(&graphics, &source, raster, part)? else {
            sprite.SetBrush(None::<&CompositionBrush>)?;
            continue;
        };
        let image_brush = compositor.CreateSurfaceBrushWithSurface(&surface)?;
        image_brush.SetStretch(CompositionStretch::None)?;
        image_brush.SetHorizontalAlignmentRatio(0.)?;
        image_brush.SetVerticalAlignmentRatio(0.)?;
        image_brush.SetTransformMatrix(transform)?;
        if image.rendering() == ImageRendering::Pixelated {
            image_brush
                .SetBitmapInterpolationMode(CompositionBitmapInterpolationMode::NearestNeighbor)?;
        }
        let brush = with_effects(
            &compositor,
            image_brush,
            &colorize,
            raster,
            // The colorize brush spans the whole image, as upstream colorizes it before cutting.
            Matrix3x2::translation(-(part.origin.x as f32), -(part.origin.y as f32)) * transform,
            fit.tiled.map(|_| tiling),
        )?;
        sprite.SetBrush(&brush)?;
    }
    Ok(())
}

/// The size an SVG of `source` size renders at when asked for `requested`: it keeps its aspect
/// ratio, as `tiny_skia::IntSize::scale_to` computes it. The renderers stretch the result over
/// the requested size.
fn svg_raster(source: IntSize, requested: IntSize) -> IntSize {
    if source.is_empty() {
        return requested;
    }
    let (sw, sh) = (source.width as f32, source.height as f32);
    let width = (requested.height as f32 * sw / sh).ceil() as u32;
    if width < requested.width {
        IntSize::new(width, requested.height)
    } else {
        IntSize::new(requested.width, (requested.width as f32 * sh / sw).ceil() as u32)
    }
}

/// Where the image's parts go, in physical pixels relative to the item: one for most images,
/// nine for a nine-slice image.
fn fits(
    image: Pin<&dyn RenderImage>,
    source: &Image,
    target: euclid::Size2D<f32, PhysicalPx>,
    scale_factor: ScaleFactor,
) -> Vec<FitResult> {
    let source_size = source.size();
    if source_size.is_empty() {
        return Vec::new();
    }
    if let ImageInner::NineSlice(nine) = <&ImageInner>::from(source) {
        fit9slice(source_size, nine.1, target, scale_factor, image.alignment(), image.tiling())
            .collect()
    } else {
        fit(
            image.image_fit(),
            target,
            image.source_clip().unwrap_or_else(|| IntRect::from_size(source_size.cast())),
            scale_factor,
            image.alignment(),
            image.tiling(),
        )
        .into_iter()
        .collect()
    }
}

/// The sprites for `n` parts. One part shows in the node's surface sprite itself; more parts
/// show in children of it.
fn sprites(visual: &mut VisualUpdate<'_>, n: usize) -> Vec<SpriteVisual> {
    let floor = visual.surface_sprite();
    // Upstream's renderers draw images from a whole-pixel origin (`pixel_align_origin`).
    let _ = floor.SetIsPixelSnappingEnabled(true);
    let children = floor.Children().unwrap();
    if n == 1 {
        let _ = children.RemoveAll();
        return alloc::vec![floor];
    }
    let _ = floor.SetBrush(None::<&CompositionBrush>);
    let _ = floor.SetClip(None::<&windows::UI::Composition::CompositionClip>);
    let _ = floor.SetOffset(Vector3::default());
    let mut sprites: Vec<SpriteVisual> =
        (&children).into_iter().filter_map(|v| v.cast().ok()).collect();
    while sprites.len() > n {
        let _ = children.Remove(&sprites.pop().unwrap());
    }
    while sprites.len() < n {
        let sprite = visual.compositor().CreateSpriteVisual().unwrap();
        let _ = children.InsertAtTop(&sprite);
        sprites.push(sprite);
    }
    sprites
}

/// `image` colorized and tiled as needed. `colorize_transform` maps the whole image's raster
/// pixels into the sprite.
fn with_effects(
    compositor: &Compositor,
    image: CompositionSurfaceBrush,
    colorize: &crate::Brush,
    raster: IntSize,
    colorize_transform: Matrix3x2,
    tiling: Option<(ImageTiling, ImageTiling)>,
) -> windows::core::Result<CompositionBrush> {
    let color = if colorize.is_transparent() {
        None
    } else {
        // The raster is in physical pixels already.
        super::shapes::make_brush(compositor, colorize, raster.cast::<f32>().cast_unit(), || 1.)
    };
    if let Some(color) = &color {
        if let Ok(gradient) = color.cast::<CompositionGradientBrush>() {
            gradient.SetTransformMatrix(colorize_transform)?;
        } else if let Ok(conic) = color.cast::<CompositionSurfaceBrush>() {
            // Drawn at a pixel per raster pixel, so the transform replaces an identity.
            conic.SetTransformMatrix(colorize_transform)?;
        }
    }
    if color.is_none() && tiling.is_none() {
        return image.cast();
    }
    // The Border effect takes a source parameter only, so the tiles are colorized after tiling.
    // Upstream colorizes before tiling: a solid color gives the same, a gradient spans the
    // first tile here and extends past it instead of repeating with the tiles.
    let mut effect = Effect::Source("image");
    if let Some((x, y)) = tiling {
        effect = Effect::Tile {
            x: x != ImageTiling::None,
            y: y != ImageTiling::None,
            input: Box::new(effect),
        };
    }
    if color.is_some() {
        effect = Effect::SourceIn {
            destination: Box::new(effect),
            source: Box::new(Effect::Source("color")),
        };
    }
    let image: CompositionBrush = image.cast()?;
    let mut sources = alloc::vec![("image", &image)];
    if let Some(color) = &color {
        sources.push(("color", color));
    }
    effects::brush(compositor, &effect, &sources)?.cast()
}

/// Identifies a surface: the image's pixel data, the size it's rasterized at, and the part of it
/// (in raster pixels) the surface holds.
#[derive(PartialEq, Eq, Hash, Clone, Copy)]
struct Key {
    data: usize,
    raster: (u32, u32),
    part: (u32, u32, u32, u32),
}

struct Entry {
    /// Keeps the pixel data, and with it the key's address, alive.
    _image: Image,
    surface: CompositionDrawingSurface,
    /// The device generation the pixels were uploaded for.
    generation: u64,
    bytes: usize,
    last_use: u64,
}

/// The surface pixels the cache keeps for sharing. Items keep the surfaces they show either way;
/// a surface that drops out of the cache is only no longer shared with items that come later.
/// (Composition surfaces have no weak references.)
const CACHE_BYTES: usize = 64 << 20;

#[derive(Default)]
struct Cache {
    entries: HashMap<Key, Entry>,
    bytes: usize,
    uses: u64,
}

impl Cache {
    fn insert(&mut self, key: Key, entry: Entry) {
        self.bytes += entry.bytes;
        if let Some(old) = self.entries.insert(key, entry) {
            self.bytes -= old.bytes;
        }
        while self.bytes > CACHE_BYTES && self.entries.len() > 1 {
            let oldest = self.entries.iter().min_by_key(|(_, e)| e.last_use).map(|(k, _)| *k);
            let Some(oldest) = oldest else { break };
            if let Some(e) = self.entries.remove(&oldest) {
                self.bytes -= e.bytes;
            }
        }
    }
}

std::thread_local! {
    /// The surfaces items show, shared between items that show the same image.
    static SURFACES: RefCell<Cache> = Default::default();
}

/// The surface with `part` of `image` rasterized at `raster`, shared with other items that show
/// the same; `None` if the image has no pixels this runtime can show.
fn surface(
    graphics: &Graphics,
    image: &Image,
    raster: IntSize,
    part: IntRect,
) -> windows::core::Result<Option<CompositionDrawingSurface>> {
    let inner: &ImageInner = image.into();
    let Some(data) = identity(inner) else { return Ok(None) };
    let key = Key {
        data,
        raster: (raster.width, raster.height),
        part: (part.origin.x as u32, part.origin.y as u32, part.width() as u32, part.height() as u32),
    };
    let generation = graphics.device_generation();
    let cached = SURFACES.with(|s| {
        let mut s = s.borrow_mut();
        s.uses += 1;
        let uses = s.uses;
        let entry = s.entries.get_mut(&key)?;
        entry.last_use = uses;
        let current = entry.generation == generation;
        entry.generation = generation;
        Some((entry.surface.clone(), current))
    });
    let surface = match cached {
        Some((surface, true)) => return Ok(Some(surface)),
        // The device was replaced: the surface is empty.
        Some((surface, false)) => surface,
        None => graphics.composition.CreateDrawingSurface(
            Size { Width: part.width() as f32, Height: part.height() as f32 },
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            DirectXAlphaMode::Premultiplied,
        )?,
    };
    let scalable = inner.is_svg().then(|| euclid::size2(raster.width, raster.height));
    let Some(buffer) = inner.render_to_buffer(scalable) else { return Ok(None) };
    let Some(pixels) = bgra(&buffer, part) else { return Ok(None) };
    upload(&surface, &pixels, part.width() as u32, part.height() as u32)?;
    SURFACES.with(|s| {
        let mut s = s.borrow_mut();
        let entry = Entry {
            _image: image.clone(),
            surface: surface.clone(),
            generation,
            bytes: pixels.len(),
            last_use: s.uses,
        };
        s.insert(key, entry);
    });
    Ok(Some(surface))
}

/// The address of an image's pixel data, or of what it's rendered from.
fn identity(inner: &ImageInner) -> Option<usize> {
    let address = |p: *const u8| p as usize;
    match inner {
        ImageInner::EmbeddedImage { buffer, .. } => Some(address(match buffer {
            SharedImageBuffer::RGB8(b) => b.as_bytes().as_ptr(),
            SharedImageBuffer::RGBA8(b) => b.as_bytes().as_ptr(),
            SharedImageBuffer::RGBA8Premultiplied(b) => b.as_bytes().as_ptr(),
            #[allow(unreachable_patterns)]
            _ => return None,
        })),
        #[cfg(feature = "svg")]
        ImageInner::Svg(svg) => Some(&**svg as *const _ as usize),
        ImageInner::StaticTextures(textures) => Some(*textures as *const _ as usize),
        ImageInner::NineSlice(nine) => identity(&nine.0),
        ImageInner::AnimatedImage(animated) => Some(&**animated as *const _ as usize),
        _ => None,
    }
}

/// `rect` of `buffer` as premultiplied BGRA rows, transparent where `rect` reaches past
/// `buffer`. An SVG renders with its aspect ratio, a pixel short of the raster size at times.
fn bgra(buffer: &SharedImageBuffer, rect: IntRect) -> Option<Vec<u8>> {
    let (x0, y0) = (rect.origin.x as usize, rect.origin.y as usize);
    let (w, h) = (rect.width() as usize, rect.height() as usize);
    let (width, height) = (buffer.width() as usize, buffer.height() as usize);
    let mut out = alloc::vec![0u8; w * h * 4];
    let (cw, ch) = (width.saturating_sub(x0).min(w), height.saturating_sub(y0).min(h));
    let mut copy = |convert: &dyn Fn(usize) -> [u8; 4]| {
        for row in 0..ch {
            for col in 0..cw {
                let o = (row * w + col) * 4;
                out[o..o + 4].copy_from_slice(&convert((y0 + row) * width + x0 + col));
            }
        }
    };
    match buffer {
        SharedImageBuffer::RGB8(b) => {
            let s = b.as_slice();
            copy(&|i| [s[i].b, s[i].g, s[i].r, 255]);
        }
        SharedImageBuffer::RGBA8(b) => {
            let s = b.as_slice();
            let pm = |c: u8, a: u8| ((c as u32 * a as u32 + 127) / 255) as u8;
            copy(&|i| {
                let p = s[i];
                [pm(p.b, p.a), pm(p.g, p.a), pm(p.r, p.a), p.a]
            });
        }
        SharedImageBuffer::RGBA8Premultiplied(b) => {
            let s = b.as_slice();
            copy(&|i| [s[i].b, s[i].g, s[i].r, s[i].a]);
        }
        #[allow(unreachable_patterns)]
        _ => return None,
    }
    Some(out)
}

/// Copies `w`×`h` premultiplied BGRA pixels into the surface's own bitmap (S5).
pub(super) fn upload(
    surface: &CompositionDrawingSurface,
    pixels: &[u8],
    w: u32,
    h: u32,
) -> windows::core::Result<()> {
    let interop: ICompositionDrawingSurfaceInterop = surface.cast()?;
    let mut offset = POINT::default();
    let dc: ID2D1DeviceContext = unsafe { interop.BeginDraw(None, &mut offset)? };
    let copied = unsafe {
        dc.GetTarget().and_then(|t| t.cast::<ID2D1Bitmap>()).and_then(|target| {
            let rect = D2D_RECT_U {
                left: offset.x as u32,
                top: offset.y as u32,
                right: offset.x as u32 + w,
                bottom: offset.y as u32 + h,
            };
            target.CopyFromMemory(Some(&rect), pixels.as_ptr().cast(), w * 4)
        })
    };
    let end = unsafe { interop.EndDraw() };
    copied.and(end)
}
