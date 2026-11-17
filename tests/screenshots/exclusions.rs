// SPDX-License-Identifier: MIT OR Apache-2.0

// Cases that can't pass yet, each with the missing piece.
// The paths are relative to upstream's `tests/screenshots/cases`.
//
// A case has a reference of its own only when its snapshot is close to the reference of
// upstream: at most 1% of the pixels differ by more than 64 and at most 5% by more than 16 (the
// difference of `color_difference` in upstream's `testing.rs`). The others are listed here, with
// what differs.

const CONIC: &str = "conic gradients draw their first color only";
const TEXT: &str = concat!(
    "glyph rasterization differs from the reference of the software renderer ",
    "(Skia has none for text)"
);

/// Cases that are generated and compiled, but ignored when running: they render, and the
/// result is known to differ from the references of upstream.
pub const IGNORED: &[(&str, &str)] = &[
    ("basic/clip-border-radius-scaled.slint", "the edge of a rounded clip is aliased (compared with the software renderer: Skia renders the case at scale factor 1)"),
    ("basic/conic-gradients.slint", CONIC),
    ("basic/gradient-border-radius.slint", CONIC),
    ("basic/gradient-borders.slint", CONIC),
    ("basic/radial-gradient-explicit-center.slint", CONIC),
    ("basic/text_features.slint", TEXT),
    ("image/border-image-downscaled.slint", "a downscaled border image is filtered smoothly, the reference of the software renderer is nearest-neighbor (Skia has none)"),
    ("image/images-scale_factor.slint", "images and text at scale factor 3.2 are filtered differently from the reference of the software renderer (Skia has none)"),
    ("image/svg-transform-scale.slint", "an SVG is magnified as a bitmap under transform-scale, not rasterized again, and comes out blurry"),
    ("layer/clip-radius-fractional-position.slint", "the edge of a rounded clip is aliased, and the glyphs differ from the reference of the software renderer (Skia has none)"),
    ("path/path_conic_gradient.slint", CONIC),
    ("text/color-emoji.slint", "glyph rasterization differs, and the opacity of an emoji is ignored (compared with the software renderer, Skia has none)"),
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
    ("text/text-right-aligned-pixel-snap.slint", "glyph rasterization differs from the reference of vello_cpu, the only one of the size of the case (Skia has none)"),
    ("text/text-selection-subpixel.slint", "glyph rasterization differs from the reference of vello_cpu, the only one of the size of the case (Skia has none)"),
    ("text/text-stroke.slint", TEXT),
    ("text/text.slint", TEXT),
    ("text/wrapped-min-width.slint", TEXT),
];

/// Cases that aren't generated: the Slint compiler or the Rust compiler rejects them.
pub const NOT_BUILT: &[(&str, &str)] = &[];
