// SPDX-License-Identifier: MIT OR Apache-2.0

//! Images as GDI bitmaps, for menu entries and window icons.

use crate::graphics::Image;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateDIBSection, DIB_RGB_COLORS, DeleteObject, HBITMAP,
};

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Alpha {
    /// Menus draw the bitmap with `AlphaBlend`, which takes premultiplied alpha.
    Premultiplied,
    /// Icons take straight alpha.
    Straight,
}

/// `image` scaled to `size`×`size` pixels, as a top-down 32-bit bitmap.
pub(super) fn scaled(image: &Image, size: usize, alpha: Alpha) -> Option<HBITMAP> {
    let pixels = image.to_rgba8_premultiplied()?;
    let (width, height) = (pixels.width() as usize, pixels.height() as usize);
    if width == 0 || height == 0 || size == 0 {
        return None;
    }
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: size as i32,
            biHeight: -(size as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits = core::ptr::null_mut();
    let bitmap =
        unsafe { CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0) }.ok()?;
    if bits.is_null() {
        unsafe {
            let _ = DeleteObject(bitmap.into());
        }
        return None;
    }
    // SAFETY: the section holds size² pixels of 4 bytes.
    let target = unsafe { core::slice::from_raw_parts_mut(bits as *mut u8, size * size * 4) };
    // Each target pixel averages the source pixels it covers, premultiplied so that
    // transparent pixels add no color.
    let source = pixels.as_slice();
    for ty in 0..size {
        let (y0, y1) = span(ty, height, size);
        for tx in 0..size {
            let (x0, x1) = span(tx, width, size);
            let mut sum = [0u32; 4];
            for y in y0..y1 {
                for p in &source[y * width + x0..y * width + x1] {
                    sum[0] += p.b as u32;
                    sum[1] += p.g as u32;
                    sum[2] += p.r as u32;
                    sum[3] += p.a as u32;
                }
            }
            let count = ((y1 - y0) * (x1 - x0)) as u32;
            let mut pixel = sum.map(|sum| sum / count);
            let a = pixel[3];
            if alpha == Alpha::Straight && a > 0 {
                for channel in &mut pixel[..3] {
                    *channel = (*channel * 255 / a).min(255);
                }
            }
            let offset = (ty * size + tx) * 4;
            for (target, value) in target[offset..offset + 4].iter_mut().zip(pixel) {
                *target = value as u8;
            }
        }
    }
    Some(bitmap)
}

/// The source pixels that target pixel `i` of `size` covers, at least one.
fn span(i: usize, source: usize, size: usize) -> (usize, usize) {
    let start = i * source / size;
    let end = ((i + 1) * source / size).max(start + 1).min(source);
    (start.min(source - 1), end)
}
