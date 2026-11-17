// SPDX-License-Identifier: MIT OR Apache-2.0

//! The drawing devices of a UI thread: a D3D11 device, its Direct2D device, the composition
//! graphics device that makes drawing surfaces from it, and DirectWrite font faces for the
//! fonts parley lays text out with (`docs/design/visual-tree.md`, "Text").

#![allow(unsafe_code)]

use crate::graphics::{Brush, Color, GradientStop, ResolvedBrush, resolve_brush};
use crate::lengths::{PhysicalPx, ScaleFactor};
use crate::properties::Property;
use alloc::rc::Rc;
use alloc::vec::Vec;
use core::cell::RefCell;
use core::pin::Pin;
use std::collections::HashMap;
use windows::Foundation::{Size, TypedEventHandler};
use windows::Graphics::DirectX::{DirectXAlphaMode, DirectXPixelFormat};
use windows::UI::Composition::{
    CompositionDrawingSurface, CompositionGraphicsDevice, CompositionStretch,
    CompositionSurfaceBrush, Compositor, SpriteVisual,
};
use windows::Win32::Foundation::{D2DERR_RECREATE_TARGET, HMODULE, SIZE};
use windows::Win32::Graphics::Direct2D::Common::{D2D1_COLOR_F, D2D1_GRADIENT_STOP};
use windows::Win32::Graphics::Direct2D::{
    D2D1_EXTEND_MODE_CLAMP, D2D1_GAMMA_2_2, D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES,
    D2D1_RADIAL_GRADIENT_BRUSH_PROPERTIES, ID2D1Brush, ID2D1Device, ID2D1DeviceContext,
    ID2D1Factory1, ID2D1RenderTarget,
};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE, D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION, D3D11CreateDevice,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_AXIS_TAG, DWRITE_FONT_AXIS_VALUE,
    DWRITE_FONT_SIMULATIONS, DWRITE_FONT_SIMULATIONS_BOLD, DWRITE_FONT_SIMULATIONS_NONE,
    DWriteCreateFactory, IDWriteFactory7, IDWriteFontFace5, IDWriteInMemoryFontFileLoader,
};
use windows::Win32::Graphics::Dxgi::{
    DXGI_ERROR_DEVICE_REMOVED, DXGI_ERROR_DEVICE_RESET, IDXGIDevice,
};
use windows::Win32::System::Com::{IAgileObject, IAgileObject_Impl};
use windows::Win32::System::WinRT::Composition::{
    ICompositionDrawingSurfaceInterop, ICompositionGraphicsDeviceInterop, ICompositorInterop,
};
use windows::core::{IUnknown, Interface, Result, implement};
use windows_numerics::Vector2;

/// The largest surface side Direct3D 11 hardware supports.
pub(crate) const MAX_SURFACE_SIDE: f32 = 16384.;

/// What a glyph run's font face is made of: the font data, the face in it, faux bold, and the
/// variation settings (design-space values, as fontique's synthesis gives them).
#[derive(Clone, PartialEq, Eq, Hash)]
struct FaceKey {
    blob: u64,
    index: u32,
    bold: bool,
    /// `(tag, value bits)`.
    axes: alloc::vec::Vec<(u32, u32)>,
}

pub(crate) struct Graphics {
    d2d_factory: ID2D1Factory1,
    d2d_device: RefCell<ID2D1Device>,
    pub(crate) composition: CompositionGraphicsDevice,
    pub(crate) dwrite: IDWriteFactory7,
    loader: IDWriteInMemoryFontFileLoader,
    faces: RefCell<HashMap<FaceKey, Option<IDWriteFontFace5>>>,
    /// Bumped when the rendering device changes: every surface is empty then. Text nodes read it
    /// while they draw, so the change marks them dirty.
    device_generation: Pin<Box<Property<u64>>>,
}

/// The thread's devices, which are never released.
///
/// Thread-local destructors of the main thread run while the process exits, after the other
/// threads have ended; releasing the devices then can wait forever on them.
struct ThreadGraphics(RefCell<Option<Rc<Graphics>>>);

impl Drop for ThreadGraphics {
    fn drop(&mut self) {
        core::mem::forget(self.0.take());
    }
}

std::thread_local! {
    static GRAPHICS: ThreadGraphics = const { ThreadGraphics(RefCell::new(None)) };
}

/// The thread's devices, made for `compositor` the first time.
pub(crate) fn graphics(compositor: &Compositor) -> Option<Rc<Graphics>> {
    GRAPHICS.with(|ThreadGraphics(g)| {
        if let Some(graphics) = &*g.borrow() {
            return Some(graphics.clone());
        }
        match Graphics::new(compositor) {
            Ok(graphics) => {
                let graphics = Rc::new(graphics);
                *g.borrow_mut() = Some(graphics.clone());
                Some(graphics)
            }
            Err(e) => {
                crate::debug_log!("cannot create the text drawing devices: {}", e.message());
                None
            }
        }
    })
}

/// A D2D device on a new D3D11 device: the GPU if there is one, WARP otherwise.
fn d2d_device(factory: &ID2D1Factory1) -> Result<ID2D1Device> {
    let create = |driver: D3D_DRIVER_TYPE| unsafe {
        let mut d3d = None;
        D3D11CreateDevice(
            None,
            driver,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut d3d),
            None,
            None,
        )
        .map(|()| d3d.unwrap())
    };
    let d3d = create(D3D_DRIVER_TYPE_HARDWARE).or_else(|_| create(D3D_DRIVER_TYPE_WARP))?;
    let dxgi: IDXGIDevice = d3d.cast()?;
    unsafe { factory.CreateDevice(&dxgi) }
}

impl Graphics {
    fn new(compositor: &Compositor) -> Result<Self> {
        // Geometries drawn with the device have to come from the device's factory.
        let d2d_factory = super::shapes::d2d_factory()?;
        let device = d2d_device(&d2d_factory)?;
        let interop: ICompositorInterop = compositor.cast()?;
        let composition = unsafe { interop.CreateGraphicsDevice(&device)? };
        // The handler runs on this thread, through its dispatcher queue.
        composition.RenderingDeviceReplaced(&TypedEventHandler::new(|_, _| {
            GRAPHICS.with(|ThreadGraphics(g)| {
                if let Some(g) = &*g.borrow() {
                    g.device_replaced();
                }
            });
            Ok(())
        }))?;
        let dwrite: IDWriteFactory7 = unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)? };
        let loader = unsafe { dwrite.CreateInMemoryFontFileLoader()? };
        unsafe { dwrite.RegisterFontFileLoader(&loader)? };
        Ok(Self {
            d2d_factory,
            d2d_device: RefCell::new(device),
            composition,
            dwrite,
            loader,
            faces: Default::default(),
            device_generation: Box::pin(Property::new(0)),
        })
    }

    /// Reads the device generation, so that the caller's tracker sees a device change.
    pub(crate) fn track_device(&self) {
        self.device_generation.as_ref().get();
    }

    /// The device generation, without tracking it.
    pub(crate) fn device_generation(&self) -> u64 {
        self.device_generation.as_ref().get_untracked()
    }

    fn device_replaced(&self) {
        let generation = self.device_generation.as_ref().get_untracked();
        self.device_generation.as_ref().set(generation + 1);
    }

    /// Recovers from a lost device if `error` says so, and logs any other failure to draw `what`.
    pub(crate) fn drawing_failed(&self, error: &windows::core::Error, what: &str) {
        let code = error.code();
        if code == DXGI_ERROR_DEVICE_REMOVED
            || code == DXGI_ERROR_DEVICE_RESET
            || code == D2DERR_RECREATE_TARGET
        {
            self.device_lost();
        } else {
            crate::debug_log!("cannot draw {what}: {}", error.message());
        }
    }

    /// Recovers from a lost device: a new device for the composition graphics device, which
    /// then reports the replacement and so redraws every surface.
    pub(crate) fn device_lost(&self) {
        match d2d_device(&self.d2d_factory) {
            Ok(device) => {
                let interop: Result<ICompositionGraphicsDeviceInterop> = self.composition.cast();
                if let Ok(interop) = interop
                    && unsafe { interop.SetRenderingDevice(&device) }.is_ok()
                {
                    *self.d2d_device.borrow_mut() = device;
                    return;
                }
                crate::debug_log!("cannot replace the lost rendering device");
            }
            Err(e) => crate::debug_log!("cannot create a rendering device: {}", e.message()),
        }
    }

    /// The DirectWrite face for a parley font, made from the font's own data without copying it.
    pub(crate) fn face(
        &self,
        font: &parley::FontData,
        synthesis: &parley::fontique::Synthesis,
    ) -> Option<IDWriteFontFace5> {
        let axes: alloc::vec::Vec<(u32, u32)> = synthesis
            .variation_settings()
            .iter()
            .map(|(tag, value)| (u32::from_be_bytes(tag.to_be_bytes()), value.to_bits()))
            .collect();
        let key =
            FaceKey { blob: font.data.id(), index: font.index, bold: synthesis.embolden(), axes };
        if let Some(face) = self.faces.borrow().get(&key) {
            return face.clone();
        }
        let face = self.make_face(font, &key);
        if let Err(e) = &face {
            crate::debug_log!("cannot load a font for drawing text: {}", e.message());
        }
        let face = face.ok();
        self.faces.borrow_mut().insert(key, face.clone());
        face
    }

    fn make_face(&self, font: &parley::FontData, key: &FaceKey) -> Result<IDWriteFontFace5> {
        let data = font.data.data();
        // The owner keeps the blob, and so the bytes, alive as long as DirectWrite uses them.
        let owner: IUnknown = BlobOwner(font.data.clone()).into();
        let file = unsafe {
            self.loader.CreateInMemoryFontFileReference(
                &self.dwrite,
                data.as_ptr().cast(),
                data.len() as u32,
                &owner,
            )?
        };
        let resource = unsafe { self.dwrite.CreateFontResource(&file, key.index)? };
        let axes: alloc::vec::Vec<DWRITE_FONT_AXIS_VALUE> = key
            .axes
            .iter()
            .map(|&(tag, value)| DWRITE_FONT_AXIS_VALUE {
                // DirectWrite's tags are the four bytes in memory order.
                axisTag: DWRITE_FONT_AXIS_TAG(u32::from_le_bytes(tag.to_be_bytes())),
                value: f32::from_bits(value),
            })
            .collect();
        let simulations: DWRITE_FONT_SIMULATIONS =
            if key.bold { DWRITE_FONT_SIMULATIONS_BOLD } else { DWRITE_FONT_SIMULATIONS_NONE };
        unsafe { resource.CreateFontFace(simulations, &axes) }
    }
}

/// The drawing surface that `sprite` shows through a surface brush, of `size`: made the first
/// time, resized when the size changed. The contents are undefined then.
pub(crate) fn sprite_surface(
    graphics: &Graphics,
    sprite: &SpriteVisual,
    size: SIZE,
) -> Result<CompositionDrawingSurface> {
    if let Ok(brush) = sprite.Brush().and_then(|b| b.cast::<CompositionSurfaceBrush>()) {
        let surface: CompositionDrawingSurface = brush.Surface()?.cast()?;
        let current = surface.SizeInt32()?;
        if (current.Width, current.Height) != (size.cx, size.cy) {
            let interop: ICompositionDrawingSurfaceInterop = surface.cast()?;
            unsafe { interop.Resize(size)? };
        }
        return Ok(surface);
    }
    let surface = graphics.composition.CreateDrawingSurface(
        Size { Width: size.cx as f32, Height: size.cy as f32 },
        DirectXPixelFormat::B8G8R8A8UIntNormalized,
        DirectXAlphaMode::Premultiplied,
    )?;
    let brush = sprite.Compositor()?.CreateSurfaceBrushWithSurface(&surface)?;
    brush.SetStretch(CompositionStretch::Fill)?;
    // Whole pixels keep the content as sharp as it was drawn (S4).
    brush.SetSnapToPixels(true)?;
    sprite.SetBrush(&brush)?;
    Ok(surface)
}

pub(crate) fn d2d_color(c: Color) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: c.red() as f32 / 255.,
        g: c.green() as f32 / 255.,
        b: c.blue() as f32 / 255.,
        a: c.alpha() as f32 / 255.,
    }
}

/// A Direct2D brush for `brush`, whose gradients span `size`, or `None` if it's transparent.
pub(crate) fn d2d_brush(
    dc: &ID2D1DeviceContext,
    brush: &Brush,
    size: euclid::Size2D<f32, PhysicalPx>,
    scale: f32,
) -> Option<ID2D1Brush> {
    let resolved = resolve_brush(brush, size, ScaleFactor::new(scale))?;
    let stops = |stops: &[GradientStop]| -> Result<_> {
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
    let point = |p: euclid::Point2D<f32, PhysicalPx>| Vector2 { X: p.x, Y: p.y };
    let result: Result<ID2D1Brush> = (|| unsafe {
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

/// Keeps a font blob alive for DirectWrite.
#[implement(IAgileObject)]
struct BlobOwner(#[allow(dead_code)] parley::fontique::Blob<u8>);

impl IAgileObject_Impl for BlobOwner_Impl {}
