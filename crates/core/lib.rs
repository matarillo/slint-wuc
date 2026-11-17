// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

// cSpell: ignore sharedvector textlayout

#![doc = include_str!("README.md")]
#![doc(html_logo_url = "https://slint.dev/logo/slint-logo-square-light.svg")]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![deny(unsafe_code)]
#![allow(clippy::missing_safety_doc)] // FFI surface has many exported unsafe entry points
#![cfg_attr(slint_nightly_test, feature(non_exhaustive_omitted_patterns_lint))]
#![cfg_attr(slint_nightly_test, warn(non_exhaustive_omitted_patterns))]
#![debugger_visualizer(gdb_script_file = "gdb_pretty_printers.py")]

extern crate alloc;

pub use crate::items::OperatingSystemType;
pub use std::thread_local;

pub mod accessibility;
pub mod animations;
pub mod api;
pub mod builtin_functions;
pub mod callbacks;
pub mod component_factory;
pub mod context;
pub mod cursor;
pub mod data_transfer;
pub mod date_time;
pub mod debug_info;
pub mod debug_log;
#[doc(hidden)]
pub mod font_collection;
pub mod future;
pub mod graphics;
pub mod input;
pub mod item_focus;
pub mod item_rendering;
pub mod item_tree;
pub mod items;
pub mod layout;
pub mod lengths;
pub mod menus;
pub mod model;
pub mod once_box;
pub mod partial_renderer;
pub mod platform;
pub mod properties;
pub mod renderer;
#[cfg(feature = "rtti")]
pub mod rtti;
pub mod sharedvector;
pub mod slice;
pub mod string;
pub mod styled_text;
#[cfg(test)]
mod testing;
pub mod textlayout;
pub mod timers;
pub mod translations;
pub mod window;

#[doc(inline)]
pub use string::SharedString;

#[doc(inline)]
pub use sharedvector::SharedVector;

#[doc(inline)]
pub use graphics::{ImageInner, StaticTextures};

#[doc(inline)]
pub use properties::Property;

#[doc(inline)]
pub use callbacks::Callback;

#[doc(inline)]
pub use graphics::Color;

#[doc(inline)]
pub use graphics::Brush;

#[doc(inline)]
pub use graphics::RgbaColor;

#[doc(inline)]
pub use graphics::PathData;

#[doc(inline)]
pub use graphics::BorderRadius;

#[doc(inline)]
pub use data_transfer::DataTransfer;

pub use context::{
    MotionPreference, SlintContext, SlintContextWeak, with_existing_context, with_global_context,
};

pub type Coord = f32;

/// This type is not exported from the public API crate, so function having this
/// parameter cannot be called from the public API without naming it
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InternalToken;

thread_local!(
    /// Permit testing code to force an OS type
    pub static OPERATING_SYSTEM_OVERRIDE: core::cell::Cell<Option<OperatingSystemType>> =
        Default::default();
);

pub fn detect_operating_system() -> OperatingSystemType {
    if let Some(os_override) = OPERATING_SYSTEM_OVERRIDE.with(|os_override| os_override.get()) {
        return os_override;
    }

    OperatingSystemType::Windows
}

/// Returns true if the current platform is an Apple platform (macOS, iOS, iPadOS)
pub fn is_apple_platform() -> bool {
    matches!(detect_operating_system(), OperatingSystemType::Macos | OperatingSystemType::Ios)
}

pub fn open_url(url: &str, ctx: &SlintContext) -> Result<(), crate::api::PlatformError> {
    ctx.platform().open_url(url)
}

pub fn macos_bring_all_windows_to_front() {}
