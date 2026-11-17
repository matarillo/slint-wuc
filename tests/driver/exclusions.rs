// Cases that can't pass yet, each with the missing piece and the phase that adds it.
// A reason that starts with `BUG:` is a failure that the missing pieces don't explain.

const INPUT: &str = "input (send_mouse_click, send_keyboard_*), phase 3";
const IMAGES: &str = "images (load_image_from_embedded_data), phase 5";
const INPUT_AND_IMAGES: &str =
    "input (send_mouse_click), phase 3, and images (load_image_from_embedded_data), phase 5";

const TEST_FONTS: &str =
    "text measured with the system font, as configure_test_fonts does nothing until phase 4";
const WINDOW_SIZE: &str = "BUG: show() leaves the window at its default 800x600 size; Win32WindowAdapter::update_window_properties ignores the layout constraints, so the window never takes the preferred size of its component";

/// Cases that are generated and compiled, but ignored when running.
pub const IGNORED: &[(&str, &str)] = &[
    ("layout/box_layout_repeated_height_for_width_edition_1_18.slint", TEST_FONTS),
    ("layout/fixed_width_window_wrapped_text_height.slint", WINDOW_SIZE),
    ("layout/max_width_window_wrapped_text_height.slint", WINDOW_SIZE),
    ("layout/min_width_window_wrapped_text_height.slint", WINDOW_SIZE),
    ("layout/preferred_width_window_wrapped_text_height.slint", WINDOW_SIZE),
    ("layout/resized_window_wrapped_text_min_height.slint", WINDOW_SIZE),
    ("layout/window_fixed.slint", WINDOW_SIZE),
];

/// Cases that aren't generated: the Slint compiler or the Rust compiler rejects them.
pub const NOT_BUILT: &[(&str, &str)] = &[
    ("layout/box_layout_repeated_height_for_width.slint", IMAGES),
    ("layout/box_percentages.slint", INPUT),
    ("layout/fixed_size_explicit_constraints.slint", INPUT),
    ("layout/flickable_layout_edition_1_18.slint", IMAGES),
    ("layout/grid_fixed_size.slint", INPUT),
    ("layout/grid_nested_for_irregular.slint", INPUT),
    ("layout/grid_with_model_in_row.slint", INPUT),
    ("layout/grid_with_repeated_rows.slint", INPUT),
    ("layout/grid_with_x_y_model.slint", INPUT),
    ("layout/grid_within_for.slint", INPUT),
    ("layout/height_for_width.slint", IMAGES),
    ("layout/horizontal_for.slint", INPUT),
    ("layout/issue_1057_listview_subcomponent_height.slint", INPUT_AND_IMAGES),
    ("layout/issue_1072_opacity_geometry.slint", INPUT),
    ("layout/issue_12139_image_window_autosize.slint", IMAGES),
    ("layout/issue_12139_image_window_autosize_repeated.slint", IMAGES),
    ("layout/issue_140.slint", INPUT),
    ("layout/issue_147_for_explicit_size.slint", INPUT),
    ("layout/issue_147_for_explicit_size_merge.slint", INPUT),
    ("layout/issue_149_nested_for.slint", INPUT),
    ("layout/issue_160_box_min_max.slint", INPUT),
    ("layout/issue_167_if_relayout.slint", INPUT),
    ("layout/issue_2537_visible_in_listview.slint", INPUT_AND_IMAGES),
    ("layout/issue_407_repeated_non_layout.slint", INPUT),
    ("layout/materialized_minmax.slint", INPUT),
    ("layout/opacity_in_layout.slint", IMAGES),
    ("layout/vertical_if.slint", INPUT),
    ("models/delete_from_clicked.slint", INPUT),
    ("models/dirty_model.slint", INPUT),
    ("models/for.slint", INPUT),
    ("models/for_number_dirty.slint", INPUT),
    ("models/if.slint", INPUT),
    ("models/if_dirty.slint", INPUT),
    ("models/if_lookup.slint", INPUT),
    ("models/issue_4961_model_index_property.slint", INPUT),
    ("models/listview_model_change.slint", INPUT_AND_IMAGES),
    ("models/listview_remove_above.slint", INPUT_AND_IMAGES),
    ("models/model.slint", INPUT),
    ("models/repeated_access_to_parent_items.slint", INPUT_AND_IMAGES),
    ("models/repeater_removed_while_handling_event.slint", INPUT),
    ("models/scrolled_model.slint", INPUT_AND_IMAGES),
    ("models/write_to_model.slint", INPUT),
    ("models/write_to_model_listview.slint", INPUT_AND_IMAGES),
    ("models/write_to_model_sub_component.slint", INPUT),
    ("properties/animation_from_click.slint", INPUT),
    ("properties/animation_from_click_in_repeated_4741.slint", INPUT),
    ("properties/issue1237_states_visible.slint", INPUT),
    ("properties/states_geometry_layout_defaults.slint", IMAGES),
];
