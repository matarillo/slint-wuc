// SPDX-License-Identifier: MIT OR Apache-2.0

// Cases that can't pass yet, each with the missing piece and the phase that adds it.
// A reason that starts with `BUG:` is a failure that the missing pieces don't explain.

const INTERPRETER: &str = concat!(
    "slint-interpreter (the embedded component is compiled at run time), ",
    "not part of this fork, listed as a gap in phase 7"
);
const TESTING_BACKEND: &str = concat!(
    "TestingBackend (upstream's testing platform, for a SlintContext of its own) and ",
    "slint::platform::Platform, which the fork keeps out of its public API, phase 7"
);
const TESTING_BACKEND_OPEN_URL: &str = concat!(
    "TestingBackend (upstream's testing platform, for a SlintContext of its own, ",
    "that records open_url), phase 7"
);

/// Cases that are generated and compiled, but ignored when running.
pub const IGNORED: &[(&str, &str)] = &[];

/// Cases that aren't generated: the Slint compiler or the Rust compiler rejects them.
pub const NOT_BUILT: &[(&str, &str)] = &[
    ("popups/popupwindow_own_context.slint", TESTING_BACKEND),
    ("elements/component_container.slint", INTERPRETER),
    ("elements/component_container_click.slint", INTERPRETER),
    ("elements/component_container_component.slint", INTERPRETER),
    ("elements/component_container_init.slint", INTERPRETER),
    ("elements/component_container_init_focus_in_layout.slint", INTERPRETER),
    ("elements/component_container_layout.slint", INTERPRETER),
    ("elements/component_container_size.slint", INTERPRETER),
    ("elements/systemtray_context.slint", TESTING_BACKEND_OPEN_URL),
    ("elements/timer_own_context.slint", TESTING_BACKEND),
    ("elements/window_shared_adapter_context.slint", TESTING_BACKEND),
    ("text/componentcontainer_font_size_propagation.slint", INTERPRETER),
];
