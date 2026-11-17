// SPDX-License-Identifier: MIT OR Apache-2.0

// Cases that can't pass yet, each with the missing piece and the phase that adds it.
// A reason that starts with `BUG:` is a failure that the missing pieces don't explain.

const IMAGES: &str = "images (load_image_from_embedded_data), phase 5";
const IMAGES_AND_TEXT_INPUT: &str =
    "images (load_image_from_embedded_data), phase 5, and TextInput in LineEdit, phase 4";
const IMAGES_TEXT_INPUT_POPUP: &str = concat!(
    "images (load_image_from_embedded_data), phase 5, ",
    "TextInput in LineEdit, phase 4, and popups, phase 6"
);
const ACCESSIBILITY: &str =
    "accessibility (ElementHandle::invoke_accessible_default_action), phase 6";
const IMAGES_AND_ACCESSIBILITY: &str = concat!(
    "images (load_image_from_embedded_data), phase 5, ",
    "and accessibility (ElementHandle::accessible_role), phase 6"
);
const ACCESSIBILITY_VALUE: &str =
    "accessibility (ElementHandle::accessible_value, accessible_read_only), phase 6";
const INTERPRETER: &str = concat!(
    "slint-interpreter (the embedded component is compiled at run time), ",
    "not part of this fork, listed as a gap in phase 7"
);
const TESTING_BACKEND: &str = concat!(
    "TestingBackend (upstream's testing platform, for a SlintContext of its own) and ",
    "slint::platform::Platform, which the fork keeps out of its public API, phase 7"
);
const TESTING_BACKEND_AND_TRAY: &str = concat!(
    "TestingBackend (upstream's testing platform, for a SlintContext of its own, with open_url), ",
    "phase 7, and SystemTrayIcon and the Platform global, phase 6"
);

/// Cases that are generated and compiled, but ignored when running.
pub const IGNORED: &[(&str, &str)] = &[];

/// Cases that aren't generated: the Slint compiler or the Rust compiler rejects them.
pub const NOT_BUILT: &[(&str, &str)] = &[
    ("layout/box_layout_repeated_height_for_width.slint", IMAGES),
    ("layout/flickable_layout_edition_1_18.slint", IMAGES),
    ("layout/height_for_width.slint", IMAGES),
    ("layout/issue_1057_listview_subcomponent_height.slint", IMAGES),
    ("layout/issue_12139_image_window_autosize.slint", IMAGES),
    ("layout/issue_12139_image_window_autosize_repeated.slint", IMAGES),
    ("layout/issue_2537_visible_in_listview.slint", IMAGES),
    ("layout/opacity_in_layout.slint", IMAGES),
    ("models/listview_model_change.slint", IMAGES),
    ("models/listview_remove_above.slint", IMAGES),
    ("models/repeated_access_to_parent_items.slint", IMAGES),
    ("models/scrolled_model.slint", IMAGES),
    ("models/write_to_model_listview.slint", IMAGES),
    ("properties/states_geometry_layout_defaults.slint", IMAGES),
    ("input/dragarea_droparea.slint", IMAGES),
    ("input/dragarea_image.slint", IMAGES),
    ("input/event_rotation.slint", ACCESSIBILITY),
    ("input/toucharea_cursor.slint", IMAGES),
    ("focus/7058_scrolled_clip.slint", IMAGES_AND_TEXT_INPUT),
    ("focus/focus-scrolled-out-of-flickable.slint", IMAGES_AND_TEXT_INPUT),
    ("focus/hidden_programmatic.slint", ACCESSIBILITY),
    ("focus/keyboard_focus2.slint", IMAGES),
    ("focus/keyboard_focus_capture.slint", IMAGES),
    ("focus/listview-hidden.slint", IMAGES_AND_TEXT_INPUT),
    ("focus/popupwindow_focus.slint", IMAGES_TEXT_INPUT_POPUP),
    ("focus/refused_programmatic.slint", IMAGES_AND_TEXT_INPUT),
    ("focus/tab-into-line-edit-scrolled-out-of-flickable.slint", IMAGES_AND_TEXT_INPUT),
    ("focus/text-input-focused-condition-disable.slint", IMAGES_AND_TEXT_INPUT),
    ("focus/text-input-focused-hide-other-widget.slint", IMAGES_AND_TEXT_INPUT),
    ("focus/text-input-focused-hide.slint", IMAGES_AND_TEXT_INPUT),
    ("focus/text-input-focused-parent-deleted.slint", IMAGES_AND_TEXT_INPUT),
    ("focus/text-input-focused-parent-hide.slint", IMAGES_AND_TEXT_INPUT),
    ("focus/text-input-focused-popup-close.slint", IMAGES_TEXT_INPUT_POPUP),
    ("focus/text-input-focused-tabwidget.slint", IMAGES_TEXT_INPUT_POPUP),
    ("focus/text-input-focused.slint", IMAGES_AND_TEXT_INPUT),
    ("elements/component_container.slint", INTERPRETER),
    ("elements/component_container_click.slint", INTERPRETER),
    ("elements/component_container_component.slint", INTERPRETER),
    ("elements/component_container_init.slint", INTERPRETER),
    ("elements/component_container_init_focus_in_layout.slint", INTERPRETER),
    ("elements/component_container_layout.slint", INTERPRETER),
    ("elements/component_container_size.slint", INTERPRETER),
    ("elements/contextmenu_close.slint", IMAGES),
    ("elements/flickable_deprecated_viewport.slint", IMAGES),
    ("elements/image.slint", IMAGES_AND_ACCESSIBILITY),
    ("elements/image_animated.slint", IMAGES),
    ("elements/image_clip_component.slint", IMAGES),
    ("elements/image_embedded.slint", IMAGES),
    ("elements/image_geometry.slint", IMAGES),
    ("elements/listview-ensure-visible-row.slint", IMAGES),
    ("elements/listview-in-vertical-layout.slint", IMAGES),
    ("elements/listview-millions.slint", IMAGES),
    ("elements/listview.slint", IMAGES),
    ("elements/listview_click_open.slint", IMAGES),
    ("elements/listview_differing_heights.slint", IMAGES),
    ("elements/listview_ensure_row_visible_differing_heights.slint", IMAGES),
    ("elements/listview_ensure_row_visible_end_of_list.slint", IMAGES),
    ("elements/listview_ensure_row_visible_exported_window.slint", IMAGES),
    ("elements/listview_ensure_row_visible_from_row_init.slint", IMAGES),
    ("elements/listview_ensure_row_visible_reduced_motion.slint", IMAGES),
    ("elements/listview_ensure_row_visible_reuses_instances.slint", IMAGES),
    ("elements/listview_ensure_row_visible_smooth.slint", IMAGES),
    ("elements/listview_ensure_row_visible_smooth_differing_heights.slint", IMAGES),
    ("elements/listview_ensure_row_visible_smooth_model_shrink.slint", IMAGES),
    ("elements/listview_ensure_row_visible_smooth_pointer_over.slint", IMAGES),
    ("elements/listview_ensure_row_visible_smooth_touch.slint", IMAGES),
    ("elements/listview_interactive_animation.slint", IMAGES),
    ("elements/listview_interactive_animation_xy_animation.slint", IMAGES),
    ("elements/listview_jump_keeps_offset.slint", IMAGES),
    ("elements/listview_scroll_touch_pad.slint", IMAGES),
    ("elements/listview_viewport_size.slint", IMAGES),
    ("elements/systemtray_context.slint", TESTING_BACKEND_AND_TRAY),
    ("elements/timer_own_context.slint", TESTING_BACKEND),
    ("elements/window_actions.slint", IMAGES),
    ("elements/window_shared_adapter_context.slint", TESTING_BACKEND),
    ("text/componentcontainer_font_size_propagation.slint", INTERPRETER),
    ("text/textinput_keyevents.slint", ACCESSIBILITY_VALUE),
];
