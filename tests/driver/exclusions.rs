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
const TEST_FONTS: &str =
    "text measured with the system font, as configure_test_fonts does nothing until phase 4";

/// Cases that are generated and compiled, but ignored when running.
pub const IGNORED: &[(&str, &str)] = &[
    ("layout/box_layout_repeated_height_for_width_edition_1_18.slint", TEST_FONTS),
];

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
];
