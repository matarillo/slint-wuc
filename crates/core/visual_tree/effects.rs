// SPDX-License-Identifier: MIT OR Apache-2.0

//! Composition effect brushes without Win2D: Direct2D effects described to the compositor
//! through `IGraphicsEffectD2D1Interop` (S5).

#![allow(unsafe_code)]

use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;
use std::collections::HashMap;
use std::sync::Mutex;
use windows::Foundation::{IPropertyValue, PropertyValue};
use windows::Graphics::Effects::{
    IGraphicsEffect, IGraphicsEffect_Impl, IGraphicsEffectSource, IGraphicsEffectSource_Impl,
};
use windows::UI::Composition::{
    CompositionBrush, CompositionEffectBrush, CompositionEffectFactory,
    CompositionEffectSourceParameter, Compositor,
};
use windows::Win32::Foundation::{E_BOUNDS, E_INVALIDARG};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_BORDER_MODE_HARD, D2D1_COMPOSITE_MODE_SOURCE_IN,
};
use windows::Win32::Graphics::Direct2D::{
    CLSID_D2D1Border, CLSID_D2D1Composite, CLSID_D2D1GaussianBlur, D2D1_BORDER_EDGE_MODE_CLAMP,
    D2D1_BORDER_EDGE_MODE_WRAP, D2D1_GAUSSIANBLUR_OPTIMIZATION_BALANCED,
};
use windows::Win32::System::WinRT::Graphics::Direct2D::{
    GRAPHICS_EFFECT_PROPERTY_MAPPING, GRAPHICS_EFFECT_PROPERTY_MAPPING_DIRECT,
    IGraphicsEffectD2D1Interop, IGraphicsEffectD2D1Interop_Impl,
};
use windows::core::{GUID, HSTRING, Interface, PCWSTR, Result, implement};
use windows_collections::IIterable;

/// A node of an effect graph: the input a brush supplies by name, or a Direct2D effect.
pub(crate) enum Effect {
    /// Filled by [`brush`]'s sources.
    Source(&'static str),
    GaussianBlur { sigma: f32, input: Box<Effect> },
    /// Repeats the input outside its bounds along the axes that wrap; clamps along the others.
    Tile { x: bool, y: bool, input: Box<Effect> },
    /// `source` where `destination` is opaque, with the destination's alpha.
    SourceIn { destination: Box<Effect>, source: Box<Effect> },
}

impl Effect {
    /// Identifies the graph's shape: the key of its factory. The blur's sigma is a property of
    /// the brush, not of the factory.
    fn key(&self, out: &mut String) {
        match self {
            Effect::Source(name) => out.push_str(name),
            Effect::GaussianBlur { input, .. } => {
                out.push_str("blur(");
                input.key(out);
                out.push(')');
            }
            Effect::Tile { x, y, input } => {
                out.push_str(match (x, y) {
                    (true, true) => "tile(",
                    (true, false) => "tilex(",
                    (false, true) => "tiley(",
                    (false, false) => "clamp(",
                });
                input.key(out);
                out.push(')');
            }
            Effect::SourceIn { destination, source } => {
                out.push_str("in(");
                destination.key(out);
                out.push(',');
                source.key(out);
                out.push(')');
            }
        }
    }

    /// The sigma of the graph's Gaussian blur; a graph has at most one.
    fn sigma(&self) -> Option<f32> {
        match self {
            Effect::Source(_) => None,
            Effect::GaussianBlur { sigma, .. } => Some(*sigma),
            Effect::Tile { input, .. } => input.sigma(),
            Effect::SourceIn { destination, source } => destination.sigma().or(source.sigma()),
        }
    }

    fn build(&self) -> IGraphicsEffectSource {
        let effect = |id, name: &str, props, sources| -> IGraphicsEffectSource {
            let effect: IGraphicsEffect =
                D2DEffect { id, name: Mutex::new(name.into()), props, sources }.into();
            effect.cast().unwrap()
        };
        let edge = |wrap: bool| {
            Prop::U32(
                if wrap { D2D1_BORDER_EDGE_MODE_WRAP.0 } else { D2D1_BORDER_EDGE_MODE_CLAMP.0 }
                    as u32,
            )
        };
        match self {
            Effect::Source(name) => CompositionEffectSourceParameter::Create(&(*name).into())
                .and_then(|p| p.cast())
                .unwrap(),
            // `D2D1_GAUSSIANBLUR_PROP_*`. A hard border keeps the edges opaque.
            Effect::GaussianBlur { sigma, input } => effect(
                CLSID_D2D1GaussianBlur,
                "Blur",
                alloc::vec![
                    ("BlurAmount", Prop::F32(*sigma)),
                    ("Optimization", Prop::U32(D2D1_GAUSSIANBLUR_OPTIMIZATION_BALANCED.0 as u32)),
                    ("BorderMode", Prop::U32(D2D1_BORDER_MODE_HARD.0 as u32)),
                ],
                alloc::vec![input.build()],
            ),
            // `D2D1_BORDER_PROP_EDGE_MODE_X/Y`.
            Effect::Tile { x, y, input } => effect(
                CLSID_D2D1Border,
                "Tile",
                alloc::vec![("ExtendX", edge(*x)), ("ExtendY", edge(*y))],
                alloc::vec![input.build()],
            ),
            // `D2D1_COMPOSITE_PROP_MODE`; input 0 is the destination.
            Effect::SourceIn { destination, source } => effect(
                CLSID_D2D1Composite,
                "SourceIn",
                alloc::vec![("Mode", Prop::U32(D2D1_COMPOSITE_MODE_SOURCE_IN.0 as u32))],
                alloc::vec![destination.build(), source.build()],
            ),
        }
    }
}

std::thread_local! {
    /// One factory per graph shape. The compositor caches compiled graphs too, but a factory of
    /// its own still costs a call into it.
    static FACTORIES: RefCell<HashMap<String, CompositionEffectFactory>> = Default::default();
}

/// A brush that draws `effect`, with `sources` filling its [`Effect::Source`]s by name.
pub(crate) fn brush(
    compositor: &Compositor,
    effect: &Effect,
    sources: &[(&str, &CompositionBrush)],
) -> Result<CompositionEffectBrush> {
    let mut key = String::new();
    effect.key(&mut key);
    let sigma = effect.sigma();
    let factory = FACTORIES.with(|f| -> Result<CompositionEffectFactory> {
        if let Some(factory) = f.borrow().get(&key) {
            return Ok(factory.clone());
        }
        let graph: IGraphicsEffect = effect.build().cast()?;
        let factory = if sigma.is_some() {
            let animatable = IIterable::<HSTRING>::from(alloc::vec![HSTRING::from(BLUR_AMOUNT)]);
            compositor.CreateEffectFactoryWithProperties(&graph, &animatable)?
        } else {
            compositor.CreateEffectFactory(&graph)?
        };
        f.borrow_mut().insert(key, factory.clone());
        Ok(factory)
    })?;
    let brush = factory.CreateBrush()?;
    for (name, source) in sources {
        brush.SetSourceParameter(&HSTRING::from(*name), *source)?;
    }
    if let Some(sigma) = sigma {
        set_blur(&brush, sigma)?;
    }
    Ok(brush)
}

/// The brush property of the graph's Gaussian blur (`Effect::GaussianBlur`'s effect name and
/// its property name).
const BLUR_AMOUNT: &str = "Blur.BlurAmount";

/// Changes the sigma of the Gaussian blur in a brush that [`brush`] made.
pub(crate) fn set_blur(brush: &CompositionEffectBrush, sigma: f32) -> Result<()> {
    brush.Properties()?.InsertScalar(&BLUR_AMOUNT.into(), sigma)
}

#[derive(Clone, Copy)]
enum Prop {
    F32(f32),
    U32(u32),
}

#[implement(IGraphicsEffect, IGraphicsEffectSource, IGraphicsEffectD2D1Interop)]
struct D2DEffect {
    id: GUID,
    name: Mutex<HSTRING>,
    /// In the order of the Direct2D effect's property indices.
    props: Vec<(&'static str, Prop)>,
    sources: Vec<IGraphicsEffectSource>,
}

impl IGraphicsEffect_Impl for D2DEffect_Impl {
    fn Name(&self) -> Result<HSTRING> {
        Ok(self.name.lock().unwrap().clone())
    }

    fn SetName(&self, name: &HSTRING) -> Result<()> {
        *self.name.lock().unwrap() = name.clone();
        Ok(())
    }
}

impl IGraphicsEffectSource_Impl for D2DEffect_Impl {}

impl IGraphicsEffectD2D1Interop_Impl for D2DEffect_Impl {
    fn GetEffectId(&self) -> Result<GUID> {
        Ok(self.id)
    }

    fn GetNamedPropertyMapping(
        &self,
        name: &PCWSTR,
        index: *mut u32,
        mapping: *mut GRAPHICS_EFFECT_PROPERTY_MAPPING,
    ) -> Result<()> {
        let name = unsafe { name.to_string() }.map_err(|_| windows::core::Error::from(E_INVALIDARG))?;
        let i = self
            .props
            .iter()
            .position(|(n, _)| n.eq_ignore_ascii_case(&name))
            .ok_or(windows::core::Error::from(E_INVALIDARG))?;
        unsafe {
            *index = i as u32;
            *mapping = GRAPHICS_EFFECT_PROPERTY_MAPPING_DIRECT;
        }
        Ok(())
    }

    fn GetPropertyCount(&self) -> Result<u32> {
        Ok(self.props.len() as u32)
    }

    fn GetProperty(&self, index: u32) -> Result<IPropertyValue> {
        let (_, prop) =
            self.props.get(index as usize).ok_or(windows::core::Error::from(E_BOUNDS))?;
        match *prop {
            Prop::F32(v) => PropertyValue::CreateSingle(v),
            Prop::U32(v) => PropertyValue::CreateUInt32(v),
        }?
        .cast()
    }

    fn GetSource(&self, index: u32) -> Result<IGraphicsEffectSource> {
        self.sources.get(index as usize).cloned().ok_or(E_BOUNDS.into())
    }

    fn GetSourceCount(&self) -> Result<u32> {
        Ok(self.sources.len() as u32)
    }
}
