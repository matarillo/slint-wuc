// Copyright © Kentaro Inomata
// SPDX-License-Identifier: MIT-0

//! Spike S3: can tests read back what the compositor draws, with and without a window?

use std::time::Instant;

use spike_common::*;
use windows::UI::Composition::{Compositor, ContainerVisual, SpriteVisual};
use windows::core::Result;

/// A 200×100 frame: white background, red square at (20,20) 40×40, blue square at (120,20) 40×40.
fn build_scene(c: &Compositor) -> Result<(ContainerVisual, SpriteVisual)> {
    let frame = c.CreateContainerVisual()?;
    frame.SetSize(V2 { X: 200.0, Y: 100.0 })?;
    let bg = c.CreateSpriteVisual()?;
    bg.SetRelativeSizeAdjustment(V2 { X: 1.0, Y: 1.0 })?;
    bg.SetBrush(&c.CreateColorBrushWithColor(rgba(255, 255, 255, 255))?)?;
    frame.Children()?.InsertAtTop(&bg)?;
    let red = c.CreateSpriteVisual()?;
    red.SetOffset(V3 { X: 20.0, Y: 20.0, Z: 0.0 })?;
    red.SetSize(V2 { X: 40.0, Y: 40.0 })?;
    red.SetBrush(&c.CreateColorBrushWithColor(rgba(255, 0, 0, 255))?)?;
    frame.Children()?.InsertAtTop(&red)?;
    let blue = c.CreateSpriteVisual()?;
    blue.SetOffset(V3 { X: 120.0, Y: 20.0, Z: 0.0 })?;
    blue.SetSize(V2 { X: 40.0, Y: 40.0 })?;
    blue.SetBrush(&c.CreateColorBrushWithColor(rgba(0, 0, 255, 255))?)?;
    frame.Children()?.InsertAtTop(&blue)?;
    Ok((frame, red))
}

fn check(label: &str, img: &Image, x: u32, y: u32, expect: [u8; 4]) -> bool {
    let got = img.pixel(x, y);
    let ok = got.iter().zip(expect).all(|(a, b)| a.abs_diff(b) <= 2);
    println!("  {label} ({x},{y}) = {got:?} expected {expect:?} -> {}", if ok { "ok" } else { "MISMATCH" });
    ok
}

fn verify(img: &Image) -> bool {
    println!("  image {}x{}", img.width, img.height);
    [
        check("background", img, 5, 5, [255, 255, 255, 255]),
        check("red", img, 40, 40, [255, 0, 0, 255]),
        check("blue", img, 140, 40, [0, 0, 255, 255]),
        check("red edge outside", img, 60, 40, [255, 255, 255, 255]),
        check("red edge inside", img, 59, 40, [255, 0, 0, 255]),
    ]
    .iter()
    .all(|b| *b)
}

fn main() -> Result<()> {
    let _queue = init_thread()?;
    let c = Compositor::new()?;
    let capturer = Capturer::new()?;
    let out = std::env::args().nth(1).unwrap_or_else(|| ".".into());

    println!("1. tree attached to a window target");
    let host = Host::new(&c, "S3 capture spike", 300, 200)?;
    let (frame, _) = build_scene(&c)?;
    host.root.Children()?.InsertAtTop(&frame)?;
    let (img, t) = capturer.capture(&frame)?;
    println!("  first capture: {t:.1} ms");
    let ok1 = verify(&img);
    img.save_png(&format!("{out}/s3-attached.png"));

    println!("2. tree not attached to any target or window");
    let (detached, red) = build_scene(&c)?;
    let ok2 = match capturer.capture(&detached) {
        Ok((img, t)) => {
            println!("  capture: {t:.1} ms");
            img.save_png(&format!("{out}/s3-detached.png"));
            verify(&img)
        }
        Err(e) => {
            println!("  FAILED: {e}");
            false
        }
    };

    println!("3. capture after a property change");
    red.SetBrush(&c.CreateColorBrushWithColor(rgba(0, 160, 0, 255))?)?;
    let commit = wait_commit(&c)?;
    let (img, _) = capturer.capture(&detached)?;
    println!("  commit wait {commit:.1} ms");
    let ok3 = check("red is now green", &img, 40, 40, [0, 160, 0, 255]);

    println!("4. cost of repeated captures (20x)");
    let t0 = Instant::now();
    let mut times = Vec::new();
    for _ in 0..20 {
        times.push(capturer.capture(&detached)?.1);
    }
    times.sort_by(f64::total_cmp);
    println!("  total {:.1} ms, per capture min {:.1} p50 {:.1} max {:.1}", ms(t0), times[0], times[10], times[19]);

    println!("5. tree dump");
    print!("{}", dump_tree(&host.root.cast()?));

    println!("result: attached={ok1} detached={ok2} after-change={ok3}");
    Ok(())
}

use windows::core::Interface;
