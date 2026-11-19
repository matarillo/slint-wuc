// Copyright © Kentaro Inomata
// SPDX-License-Identifier: MIT-0

//! Upstream's and the fork's screenshot of a step, side by side with their difference.

use image::{Rgba, RgbaImage};

pub struct Comparison {
    /// Upstream | fork | four times their difference.
    pub image: RgbaImage,
    /// The percentage of pixels whose largest channel differs by more than 16, and 64.
    pub over_16: f32,
    pub over_64: f32,
}

/// Compares `upstream` and `fork`, which may differ in size: the comparison covers the larger of
/// each dimension, and a pixel that only one of them has differs by its own value.
pub fn compare(upstream: &RgbaImage, fork: &RgbaImage) -> Comparison {
    let width = upstream.width().max(fork.width());
    let height = upstream.height().max(fork.height());
    let mut image = RgbaImage::from_pixel(width * 3, height, Rgba([255, 0, 255, 255]));
    let (mut over_16, mut over_64) = (0u32, 0u32);
    let blank = Rgba([0, 0, 0, 0]);
    for y in 0..height {
        for x in 0..width {
            let a = upstream.get_pixel_checked(x, y).copied();
            let b = fork.get_pixel_checked(x, y).copied();
            if let Some(a) = a {
                image.put_pixel(x, y, a);
            }
            if let Some(b) = b {
                image.put_pixel(width + x, y, b);
            }
            let (a, b) = (a.unwrap_or(blank), b.unwrap_or(blank));
            let channel = |i: usize| a[i].abs_diff(b[i]);
            let largest = (0..4).map(channel).max().unwrap();
            over_16 += (largest > 16) as u32;
            over_64 += (largest > 64) as u32;
            let magnified = |i: usize| channel(i).saturating_mul(4);
            image.put_pixel(
                2 * width + x,
                y,
                Rgba([magnified(0), magnified(1), magnified(2), 255]),
            );
        }
    }
    let total = (width * height).max(1) as f32;
    Comparison {
        image,
        over_16: over_16 as f32 * 100. / total,
        over_64: over_64 as f32 * 100. / total,
    }
}
