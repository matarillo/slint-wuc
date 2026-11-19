// Copyright © Kentaro Inomata
// SPDX-License-Identifier: MIT-0

//! A Direct2D effect description that Windows.UI.Composition accepts in place of Win2D's effects.
//!
//! `Compositor::CreateEffectFactory` takes an `IGraphicsEffect` and reads the effect's CLSID,
//! properties and sources through `IGraphicsEffectD2D1Interop`.

use std::sync::Mutex;

use windows::Foundation::{IPropertyValue, PropertyValue};
use windows::Graphics::Effects::{
    IGraphicsEffect, IGraphicsEffect_Impl, IGraphicsEffectSource, IGraphicsEffectSource_Impl,
};
use windows::UI::Composition::CompositionEffectSourceParameter;
use windows::Win32::Foundation::{E_BOUNDS, E_INVALIDARG};
use windows::Win32::Graphics::Direct2D::Common::{D2D1_BORDER_MODE_HARD, D2D1_BORDER_MODE_SOFT};
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::System::WinRT::Graphics::Direct2D::*;
use windows_core::{GUID, HSTRING, Interface, PCWSTR, Result, implement};

#[derive(Clone, Copy, Debug)]
pub enum Prop {
    F32(f32),
    U32(u32),
    Vec4([f32; 4]),
}

#[implement(IGraphicsEffect, IGraphicsEffectSource, IGraphicsEffectD2D1Interop)]
pub struct Effect {
    id: GUID,
    name: Mutex<HSTRING>,
    /// In the order of the D2D effect's property indices.
    props: Vec<(&'static str, Prop)>,
    sources: Vec<IGraphicsEffectSource>,
}

impl Effect {
    pub fn new(
        id: GUID,
        name: &str,
        props: Vec<(&'static str, Prop)>,
        sources: Vec<IGraphicsEffectSource>,
    ) -> IGraphicsEffect {
        Effect { id, name: Mutex::new(name.into()), props, sources }.into()
    }
}

/// A source the effect brush supplies by name (`SetSourceParameter`).
pub fn param(name: &str) -> IGraphicsEffectSource {
    CompositionEffectSourceParameter::Create(&name.into()).unwrap().cast().unwrap()
}

pub fn source(effect: &IGraphicsEffect) -> IGraphicsEffectSource {
    effect.cast().unwrap()
}

/// `D2D1_GAUSSIANBLUR_PROP_*`. `hard` keeps the edges opaque instead of blurring in transparency.
pub fn gaussian_blur(name: &str, sigma: f32, hard: bool, input: IGraphicsEffectSource) -> IGraphicsEffect {
    Effect::new(
        CLSID_D2D1GaussianBlur,
        name,
        vec![
            ("BlurAmount", Prop::F32(sigma)),
            ("Optimization", Prop::U32(D2D1_GAUSSIANBLUR_OPTIMIZATION_BALANCED.0 as u32)),
            ("BorderMode", Prop::U32(if hard { D2D1_BORDER_MODE_HARD.0 } else { D2D1_BORDER_MODE_SOFT.0 } as u32)),
        ],
        vec![input],
    )
}

/// `D2D1_BORDER_PROP_EDGE_MODE_X/Y`: wrap repeats the input outside its bounds.
pub fn border(name: &str, wrap_x: bool, wrap_y: bool, input: IGraphicsEffectSource) -> IGraphicsEffect {
    let mode = |wrap: bool| {
        Prop::U32(if wrap { D2D1_BORDER_EDGE_MODE_WRAP.0 } else { D2D1_BORDER_EDGE_MODE_CLAMP.0 } as u32)
    };
    Effect::new(
        CLSID_D2D1Border,
        name,
        vec![("ExtendX", mode(wrap_x)), ("ExtendY", mode(wrap_y))],
        vec![input],
    )
}

/// `D2D1_COMPOSITE_PROP_MODE`; `destination` is input 0, `source` input 1.
pub fn composite(
    name: &str,
    mode: windows::Win32::Graphics::Direct2D::Common::D2D1_COMPOSITE_MODE,
    destination: IGraphicsEffectSource,
    source: IGraphicsEffectSource,
) -> IGraphicsEffect {
    Effect::new(CLSID_D2D1Composite, name, vec![("Mode", Prop::U32(mode.0 as u32))], vec![
        destination,
        source,
    ])
}

/// `D2D1_FLOOD_PROP_COLOR`, straight RGBA.
pub fn flood(name: &str, rgba: [f32; 4]) -> IGraphicsEffect {
    Effect::new(CLSID_D2D1Flood, name, vec![("Color", Prop::Vec4(rgba))], vec![])
}

impl IGraphicsEffect_Impl for Effect_Impl {
    fn Name(&self) -> Result<HSTRING> {
        Ok(self.name.lock().unwrap().clone())
    }

    fn SetName(&self, name: &HSTRING) -> Result<()> {
        *self.name.lock().unwrap() = name.clone();
        Ok(())
    }
}

impl IGraphicsEffectSource_Impl for Effect_Impl {}

impl IGraphicsEffectD2D1Interop_Impl for Effect_Impl {
    fn GetEffectId(&self) -> Result<GUID> {
        Ok(self.id)
    }

    fn GetNamedPropertyMapping(
        &self,
        name: &PCWSTR,
        index: *mut u32,
        mapping: *mut GRAPHICS_EFFECT_PROPERTY_MAPPING,
    ) -> Result<()> {
        let name = unsafe { name.to_string() }.map_err(|_| windows_core::Error::from(E_INVALIDARG))?;
        let i = self
            .props
            .iter()
            .position(|(n, _)| n.eq_ignore_ascii_case(&name))
            .ok_or(windows_core::Error::from(E_INVALIDARG))?;
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
        let (_, prop) = self.props.get(index as usize).ok_or(windows_core::Error::from(E_BOUNDS))?;
        match *prop {
            Prop::F32(v) => PropertyValue::CreateSingle(v),
            Prop::U32(v) => PropertyValue::CreateUInt32(v),
            Prop::Vec4(v) => PropertyValue::CreateSingleArray(&v),
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
