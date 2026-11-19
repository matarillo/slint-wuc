// Copyright © Kentaro Inomata
// SPDX-License-Identifier: MIT-0

// Cases that can't pass yet, each with the missing piece, and cases that pass against a
// reference of their own though it differs from upstream's.
// The paths are relative to upstream's `tests/screenshots/cases`.
//
// A case has a reference of its own when its snapshot is close to the reference of upstream: at
// most 1% of the pixels differ by more than 64 and at most 5% by more than 16 (the difference of
// `color_difference` in upstream's `testing.rs`). Run with `SLINT_SCREENSHOT_REPORT=1` to compare
// every case with upstream's reference.

const TEXT: &str = concat!(
    "glyph rasterization and subpixel positions differ from the reference of the software ",
    "renderer (Skia has none for text)"
);

/// Cases that are generated and compiled, but ignored when running: they render, and the
/// result is known to differ from the references of upstream.
pub const IGNORED: &[(&str, &str)] = &[];

/// Cases whose reference of their own differs from upstream's more than the rule allows. A
/// review of the comparison found the same layout as upstream's, and only the difference given.
pub const REVIEWED: &[(&str, &str)] = &[
    ("basic/clip-border-radius-scaled.slint", "at scale factor 1.5, the border's inner edge and the clip meet mid-pixel, and their antialiasing lets a quarter of the background through; the software renderer snaps the clip to pixels (Skia renders the case at scale factor 1)"),
    ("basic/text_features.slint", TEXT),
    ("image/border-image-downscaled.slint", "a downscaled border image is filtered smoothly, as Skia filters it; the reference of the software renderer is nearest-neighbor (Skia has none)"),
    ("image/images-scale_factor.slint", "images and text at scale factor 3.2 are filtered differently from the reference of the software renderer (Skia has none)"),
    ("layer/clip-radius-fractional-position.slint", "the glyphs and the antialiased edge of the rounded clip differ from the reference of the software renderer (Skia has none)"),
    ("path/path_conic_gradient.slint", "the slanted edges of a path filled with a conic gradient, a sprite cut to the path, are antialiased about a tenth lighter than Skia's"),
    ("text/color-emoji.slint", "glyph rasterization differs, and DirectWrite scales the bitmap emoji (CBDT) a pixel larger (compared with the software renderer, Skia has none)"),
    ("text/inline-code.slint", TEXT),
    ("text/line-height-factor.slint", TEXT),
    ("text/set-selection-offsets.slint", TEXT),
    ("text/styled-selection.slint", TEXT),
    ("text/styled.slint", TEXT),
    ("text/styled_link_list.slint", TEXT),
    ("text/text-clipped.slint", TEXT),
    ("text/text-elide-overflow.slint", TEXT),
    ("text/text-elide-paragraphs.slint", TEXT),
    ("text/text-elide-rtl.slint", TEXT),
    ("text/text-elided.slint", TEXT),
    ("text/text-input-selection.slint", TEXT),
    ("text/text-input.slint", TEXT),
    ("text/text-italic-no-italic-face.slint", TEXT),
    ("text/text-max-lines.slint", TEXT),
    ("text/text-right-aligned-pixel-snap.slint", "the rows line up, but glyph rasterization and subpixel positions differ from the reference of vello_cpu, the only one of the size of the case (Skia has none)"),
    ("text/text-selection-subpixel.slint", "glyph rasterization and subpixel positions differ from the reference of vello_cpu, the only one of the size of the case (Skia has none)"),
    ("text/text-stroke.slint", TEXT),
    ("text/text.slint", TEXT),
    ("text/wrapped-min-width.slint", TEXT),
];

/// Cases that aren't generated: the Slint compiler or the Rust compiler rejects them.
pub const NOT_BUILT: &[(&str, &str)] = &[];
