// SPDX-License-Identifier: MIT OR Apache-2.0

//! Conic gradients, which neither Composition nor Direct2D has: computed on the CPU into a
//! bitmap (`docs/design/visual-tree.md`, "Conic Gradients").

use crate::graphics::{GradientStop, ResolvedConicGradient};
use alloc::vec::Vec;

/// `width`×`height` premultiplied BGRA pixels of `g`, with `texel` pixels per unit of `g`'s
/// coordinates. Each pixel takes the gradient's color at its center.
pub(super) fn pixels(g: &ResolvedConicGradient, width: u32, height: u32, texel: f32) -> Vec<u8> {
    let stops: Vec<[f32; 5]> = g.stops.iter().map(premultiplied).collect();
    let mut out = Vec::with_capacity(width as usize * height as usize * 4);
    for y in 0..height {
        let dy = (y as f32 + 0.5) / texel - g.center.y;
        for x in 0..width {
            let dx = (x as f32 + 0.5) / texel - g.center.x;
            // Turns clockwise from 12 o'clock.
            let turn = dx.atan2(-dy) / core::f32::consts::TAU;
            let [r, g, b, a] = color_at(&stops, turn.rem_euclid(1.));
            let byte = |v: f32| (v * 255. + 0.5) as u8;
            out.extend_from_slice(&[byte(b), byte(g), byte(r), byte(a)]);
        }
    }
    out
}

/// The position and the premultiplied color of a stop, 0 to 1.
fn premultiplied(stop: &GradientStop) -> [f32; 5] {
    let c = stop.color;
    let a = c.alpha() as f32 / 255.;
    let channel = |v: u8| v as f32 / 255. * a;
    [stop.position, channel(c.red()), channel(c.green()), channel(c.blue()), a]
}

/// The color at `position`, interpolated premultiplied as upstream's renderers do.
fn color_at(stops: &[[f32; 5]], position: f32) -> [f32; 4] {
    let rgba = |s: &[f32; 5]| [s[1], s[2], s[3], s[4]];
    let Some(first) = stops.first() else { return [0.; 4] };
    if position <= first[0] {
        return rgba(first);
    }
    for pair in stops.windows(2) {
        let (s0, s1) = (&pair[0], &pair[1]);
        if position <= s1[0] {
            let t = (position - s0[0]) / (s1[0] - s0[0]);
            return core::array::from_fn(|i| s0[i + 1] + (s1[i + 1] - s0[i + 1]) * t);
        }
    }
    rgba(stops.last().unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Color;

    fn gradient(stops: &[(Color, f32)]) -> Vec<GradientStop> {
        stops.iter().map(|&(color, position)| GradientStop { color, position }).collect()
    }

    #[test]
    fn the_turn_starts_at_twelve_and_runs_clockwise() {
        let stops = gradient(&[
            (Color::from_rgb_u8(255, 0, 0), 0.),
            (Color::from_rgb_u8(0, 0, 255), 0.5),
            (Color::from_rgb_u8(255, 0, 0), 1.),
        ]);
        let g = ResolvedConicGradient {
            center: euclid::point2(1.5, 1.5),
            stops: alloc::borrow::Cow::Borrowed(&stops),
        };
        let px = pixels(&g, 3, 3, 1.);
        let at = |x: usize, y: usize| <[u8; 4]>::try_from(&px[(y * 3 + x) * 4..][..4]).unwrap();
        assert_eq!(at(1, 0), [0, 0, 255, 255]);
        assert_eq!(at(2, 0), [64, 0, 191, 255]);
        assert_eq!(at(2, 1), [128, 0, 128, 255]);
        assert_eq!(at(1, 2), [255, 0, 0, 255]);
        assert_eq!(at(0, 1), [128, 0, 128, 255]);

        // Two pixels per unit: the same gradient, at the centers of the smaller pixels.
        let px = pixels(&g, 6, 6, 2.);
        assert_eq!(&px[(5 * 6 + 3) * 4..][..4], &[239, 0, 16, 255]);
    }

    #[test]
    fn translucent_stops_mix_premultiplied() {
        let stops = gradient(&[
            (Color::from_argb_u8(0, 0, 0, 0), 0.),
            (Color::from_rgb_u8(255, 0, 0), 1.),
        ]);
        let stops: Vec<[f32; 5]> = stops.iter().map(premultiplied).collect();
        // Half way: half the red, at half the alpha, with no black mixed in.
        assert_eq!(color_at(&stops, 0.5), [0.5, 0., 0., 0.5]);
    }
}
