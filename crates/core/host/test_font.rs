// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

//! The fixed-metrics `FixedTestFont`, ported from upstream's testing backend
//! (`internal/backends/testing/testing_backend.rs`).
//!
//! Many upstream cases name this family to get text sizes that don't depend on a real font:
//! every character is as wide as the pixel size, and a line is as high. The window's renderer
//! uses these functions only when [`super::Win32WindowAdapter::enable_fixed_test_font`] turned
//! the switch on, which the testing platform does and applications never do.
//!
//! Each function returns `None` for any other family, so that the caller measures through
//! parley as usual. The numbers follow upstream's, including its quirks (byte lengths rather
//! than character counts, whitespace-separated words rather than real line breaking), because
//! the cases were written against them.

use crate::SharedString;
use crate::graphics::FontRequest;
use crate::item_rendering::{HasFont, PlainOrStyledText, RenderString};
use crate::item_tree::ItemRc;
use crate::items::{FontMetrics, TextCursorAffinity, TextInput};
use crate::lengths::{LogicalLength, LogicalPoint, LogicalRect, LogicalSize};
use crate::renderer::ContentWidths;
use alloc::string::{String, ToString};
use core::pin::Pin;

const FIXED_TEST_FONT: &str = "FixedTestFont";

/// The pixel size of a request that has none, as upstream's testing backend takes it.
const DEFAULT_PIXEL_SIZE: f32 = 10.;

fn is_fixed_test_font(family: &Option<SharedString>) -> bool {
    family.as_ref().is_some_and(|f| f == FIXED_TEST_FONT)
}

fn pixel_size(font_request: &FontRequest) -> f32 {
    font_request.pixel_size.map_or(DEFAULT_PIXEL_SIZE, |s| s.get())
}

fn line_height(line_height_factor: Option<f32>, pixel_size: f32) -> f32 {
    // The font's natural line height is exactly the pixel size (ascent 0.7 + descent 0.3).
    pixel_size * line_height_factor.unwrap_or(1.0)
}

fn raw_text(text_item: Pin<&dyn RenderString>) -> String {
    match text_item.text() {
        PlainOrStyledText::Plain(s) => s.to_string(),
        PlainOrStyledText::Styled(s) => crate::styled_text::get_raw_text(&s).into_owned(),
    }
}

pub(super) fn text_size(
    text_item: Pin<&dyn RenderString>,
    item_rc: &ItemRc,
) -> Option<LogicalSize> {
    let font_request = text_item.font_request(item_rc);
    if !is_fixed_test_font(&font_request.family) {
        return None;
    }
    let pixel_size = pixel_size(&font_request);
    let text = raw_text(text_item);
    // Whitespace-separated words rather than real line breaking, and byte lengths rather than
    // character counts, to match `text_content_widths`.
    let max_lines = text_item.line_limit().unwrap_or(usize::MAX);
    let (max_line_len, num_lines) = text
        .lines()
        .take(max_lines)
        .fold((0, 0), |(len, count), line| (len.max(line.len()), count + 1));
    let width = max_line_len as f32 * pixel_size;
    let height = num_lines.max(1) as f32 * line_height(text_item.line_height_factor(), pixel_size);
    Some(LogicalSize::new(width, height))
}

pub(super) fn text_content_widths(
    text_item: Pin<&dyn RenderString>,
    item_rc: &ItemRc,
) -> Option<ContentWidths> {
    let font_request = text_item.font_request(item_rc);
    if !is_fixed_test_font(&font_request.family) {
        return None;
    }
    let pixel_size = pixel_size(&font_request);
    let text = raw_text(text_item);
    let max_lines = text_item.line_limit().unwrap_or(usize::MAX);
    let lines = text.lines().take(max_lines);
    let longest_word =
        lines.clone().flat_map(str::split_whitespace).map(str::len).max().unwrap_or(0);
    let longest_line = lines.map(str::len).max().unwrap_or(0);
    Some(ContentWidths {
        min: LogicalLength::new(longest_word as f32 * pixel_size),
        max: LogicalLength::new(longest_line as f32 * pixel_size),
    })
}

pub(super) fn text_line_height(
    font_request: &FontRequest,
    line_height_factor: Option<f32>,
) -> Option<LogicalLength> {
    is_fixed_test_font(&font_request.family)
        .then(|| LogicalLength::new(line_height(line_height_factor, pixel_size(font_request))))
}

pub(super) fn char_size(
    font_request: &FontRequest,
    line_height_factor: Option<f32>,
) -> Option<LogicalSize> {
    is_fixed_test_font(&font_request.family).then(|| {
        let pixel_size = pixel_size(font_request);
        LogicalSize::new(pixel_size, line_height(line_height_factor, pixel_size))
    })
}

pub(super) fn font_metrics(font_request: &FontRequest) -> Option<FontMetrics> {
    is_fixed_test_font(&font_request.family).then(|| {
        let pixel_size = pixel_size(font_request);
        FontMetrics {
            ascent: pixel_size * 0.7,
            descent: -pixel_size * 0.3,
            x_height: 3.,
            cap_height: 7.,
        }
    })
}

pub(super) fn text_input_byte_offset_for_position(
    text_input: Pin<&TextInput>,
    item_rc: &ItemRc,
    pos: LogicalPoint,
) -> Option<(usize, TextCursorAffinity)> {
    let font_request = text_input.font_request(item_rc);
    if !is_fixed_test_font(&font_request.family) {
        return None;
    }
    // The font never wraps, so the affinity is always next-character.
    let pixel_size = pixel_size(&font_request);
    let line_height = line_height(RenderString::line_height_factor(text_input), pixel_size);
    let text = text_input.text();
    if pos.y < 0. {
        return Some((0, TextCursorAffinity::NextCharacter));
    }
    let line = (pos.y / line_height) as usize;
    let offset: usize = text.split('\n').take(line).map(|l| l.len() + 1).sum();
    let Some(line) = text.split('\n').nth(line) else {
        return Some((text.len(), TextCursorAffinity::NextCharacter));
    };
    let column = ((pos.x / pixel_size).max(0.) as usize).min(line.len());
    Some((offset + column, TextCursorAffinity::NextCharacter))
}

pub(super) fn text_input_cursor_rect_for_byte_offset(
    text_input: Pin<&TextInput>,
    item_rc: &ItemRc,
    byte_offset: usize,
) -> Option<LogicalRect> {
    let font_request = text_input.font_request(item_rc);
    if !is_fixed_test_font(&font_request.family) {
        return None;
    }
    let pixel_size = pixel_size(&font_request);
    let line_height = line_height(RenderString::line_height_factor(text_input), pixel_size);
    let text = text_input.text();
    let line = text[..byte_offset].chars().filter(|c| *c == '\n').count();
    let column = text[..byte_offset].split('\n').nth(line).unwrap_or("").len();
    Some(LogicalRect::new(
        LogicalPoint::new(column as f32 * pixel_size, line as f32 * line_height),
        LogicalSize::new(1., line_height),
    ))
}

#[cfg(test)]
mod tests {
    use super::super::{Platform, Win32WindowAdapter};
    use super::*;
    use crate::testing::RectTree;
    use crate::window::{WindowAdapter, WindowInner};

    fn request(family: &str) -> FontRequest {
        FontRequest {
            family: Some(family.into()),
            pixel_size: Some(LogicalLength::new(20.)),
            ..Default::default()
        }
    }

    #[test]
    fn only_a_window_that_turned_the_switch_on_measures_with_fixed_metrics() {
        let ctx =
            crate::SlintContext::new(alloc::boxed::Box::new(Platform::new_headless().unwrap()));
        let adapter = ctx.create_window_adapter().unwrap();
        let tree = vtable::VRc::into_dyn(RectTree::new((100., 50.), 1));
        WindowInner::from_pub(adapter.window()).set_component(&tree);
        let host = Win32WindowAdapter::from_window(adapter.window()).unwrap();
        let measure = |adapter: &dyn WindowAdapter| {
            let renderer = adapter.renderer();
            (
                renderer.font_metrics(request(FIXED_TEST_FONT)),
                renderer.char_size(request(FIXED_TEST_FONT), Some(1.5), 'x'),
                renderer.text_line_height(request(FIXED_TEST_FONT), None),
            )
        };

        // An application's window has no such font: the request goes to the fallback font.
        let (metrics, char_size, line_height) = measure(&*adapter);
        assert_ne!(metrics.x_height, 3.);
        assert_ne!(char_size, LogicalSize::new(20., 30.));
        assert_ne!(line_height, Some(LogicalLength::new(20.)));

        host.enable_fixed_test_font();
        let (metrics, char_size, line_height) = measure(&*adapter);
        assert_eq!(
            metrics,
            FontMetrics { ascent: 14., descent: -6., x_height: 3., cap_height: 7. }
        );
        assert_eq!(char_size, LogicalSize::new(20., 30.));
        assert_eq!(line_height, Some(LogicalLength::new(20.)));

        // Other families still go through parley.
        let other = adapter.renderer().font_metrics(request("Segoe UI"));
        assert_ne!(other.x_height, 3.);
    }
}
