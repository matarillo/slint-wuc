// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

//! Testing platform for the Slint runtime on Windows.UI.Composition.
//!
//! The platform wraps the headless host of `i-slint-core`: windows have no HWND, and every
//! [`mock_elapsed_time()`] runs the sync pass that brings the window's visuals up to date.
//! The API follows the upstream crate of the same name, as far as input and text allow.

mod search_api;
pub use search_api::*;
#[cfg(feature = "internal")]
mod internal_tests;
#[cfg(feature = "internal")]
pub use internal_tests::*;
pub mod testing_platform;
pub use testing_platform::{TestingPlatform, get_mocked_time};
#[cfg(feature = "internal")]
pub use testing_platform::{TestingWindow, access_testing_window, mock_elapsed_time};

/// Initialize the testing platform without support for event loop.
/// This means that each test thread can use its own platform, but global functions that needs
/// an event loop such as `slint::invoke_from_event_loop` won't work.
/// Must be called before any call that would otherwise initialize the Win32 host.
/// Calling it when the host is already initialized will panic.
///
/// Note that for animations and timers, the changes in the system time will be disregarded.
/// Instead, use [`mock_elapsed_time()`] to advance the simulate (mock) time Slint uses.
pub fn init_no_event_loop() {
    let platform = TestingPlatform::new(true).expect("cannot set up the headless host");
    i_slint_core::platform::set_platform(Box::new(platform)).expect("platform already initialized");
}

/// Advance the simulated mock time by the specified duration. Use in combination with
/// [`init_no_event_loop()`].
#[cfg(not(feature = "internal"))]
pub fn mock_elapsed_time(duration: std::time::Duration) {
    testing_platform::mock_elapsed_time(duration.as_millis() as _);
}

/// Set the operating system's reduced-motion setting, as a platform backend would when it
/// changes. While [`MotionPreference::Reduced`], animations that start go straight to their target.
/// Must be called after initializing the testing platform (e.g. after [`init_no_event_loop()`]).
pub fn set_motion_preference(preference: MotionPreference) {
    i_slint_core::context::with_existing_context(|ctx| ctx.set_motion_preference(preference))
        .expect("the testing platform must be initialized first");
}

pub use i_slint_core::MotionPreference;

/// Replace the font collection with embedded NotoSans fonts for deterministic test results.
/// Must be called after initializing the testing platform (e.g. after [`init_no_event_loop()`]).
#[cfg(feature = "internal")]
pub fn configure_test_fonts() {
    use i_slint_common::sharedfontique::{FALLBACK_FAMILIES, fontique};

    // Listed explicitly, in the order they take in the fallback chain: NotoSans-Regular is
    // the default upright face and comes first. The fonts are copies of upstream's
    // `tests/screenshots/fonts`, kept here so that this crate builds without `reference/`.
    static FONTS: &[&[u8]] = &[
        include_bytes!("fonts/NotoSans-Regular.ttf"),
        include_bytes!("fonts/NotoSans-Italic.ttf"),
        include_bytes!("fonts/NotoSansSymbols2-Regular.ttf"),
    ];
    // Color emoji fonts, only for the cases that name them: left out of the fallback chain so
    // they can't change any other case.
    static NAMED_FONTS: &[&[u8]] = &[
        include_bytes!("fonts/NotoColorEmoji-Subset.ttf"),
        include_bytes!("fonts/TwemojiMozilla-Subset.ttf"),
    ];

    i_slint_core::with_existing_context(|ctx| {
        let mut font_context = ctx.font_context().borrow_mut();
        font_context.collection = fontique::Collection::new(fontique::CollectionOptions {
            shared: true,
            system_fonts: false,
        });
        font_context.source_cache = fontique::SourceCache::new_shared();
        font_context.clear_registered_static_fonts();

        let mut chain_families: Vec<fontique::FamilyId> = Vec::new();
        for font in FONTS {
            let fonts = font_context
                .collection
                .register_fonts(fontique::Blob::new(std::sync::Arc::new(*font)), None);
            for (family_id, _) in &fonts {
                if !chain_families.contains(family_id) {
                    chain_families.push(*family_id);
                }
            }
        }
        for font in NAMED_FONTS {
            font_context
                .collection
                .register_fonts(fontique::Blob::new(std::sync::Arc::new(*font)), None);
        }
        // Map the fallback generics plus monospace (used by markdown code spans) to the bundled
        // fonts, so all generic families resolve deterministically with system fonts disabled.
        for generic_family in
            FALLBACK_FAMILIES.into_iter().chain([fontique::GenericFamily::Monospace])
        {
            font_context
                .collection
                .set_generic_families(generic_family, chain_families.iter().copied());
        }
    })
    .expect("platform not set, initialize the testing platform first");
}
