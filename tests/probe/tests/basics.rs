//! Runs `probe-basics --check` and compares its visual tree dumps.

use std::process::Command;

/// The dumps that `--check` prints after each step, keyed by step name.
fn run_check() -> Vec<(String, u64, String)> {
    let out = Command::new(env!("CARGO_BIN_EXE_probe-basics"))
        .arg("--check")
        .output()
        .expect("probe-basics runs");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8(out.stdout).unwrap().replace("\r\n", "\n");
    stdout
        .split("--- ")
        .filter(|s| !s.is_empty())
        .map(|section| {
            let (header, dump) = section.split_once('\n').unwrap();
            let (name, passes) = header.split_once(" (").unwrap();
            let passes = passes.split(' ').next().unwrap().parse().unwrap();
            (name.to_owned(), passes, dump.trim_end().to_owned())
        })
        .collect()
}

#[test]
#[ignore = "opens a window, so it needs an interactive desktop session"]
fn generated_code_shows_and_follows_changes() {
    let steps = run_check();
    let names: Vec<_> = steps.iter().map(|(n, _, _)| n.as_str()).collect();
    assert_eq!(names, ["initial", "toggled", "count = 5", "count = 1"]);

    let common = "\
Sprite 0,0 300x200 #202020ff
  Sprite 20,20 100x60 COLOR
    Sprite 10,10 30x30 #ffff00ff
  Shape 140,20 120x120 [fill linear(0,60->120,60 #ff0000ff@0 #0000ffff@1) rrect 2,2 116x116 r10] [stroke #ffffffff w4 rrect 2,2 116x116 r10]
  Sprite X,160 20x20 #ff00ffff";
    let expected = |color: &str, x: u32, count: u32| {
        let mut s = common.replace("COLOR", color).replace("X,", &format!("{x},"));
        for i in 0..count {
            s += &format!("\n  Sprite {},150 30x30 #00ff00ff", 20 + i * 40);
        }
        s
    };
    assert_eq!(steps[0].2, expected("#0000ffff", 140, 3));
    assert_eq!(steps[1].2, expected("#ff0000ff", 250, 3));
    assert_eq!(steps[2].2, expected("#ff0000ff", 250, 5));
    assert_eq!(steps[3].2, expected("#ff0000ff", 250, 1));

    // The 100ms animation of the "toggled" step ticks on the compositor clock (about 60Hz,
    // up to 240Hz on fast displays), not as fast as the message loop can spin.
    let animation_passes = steps[1].1 - steps[0].1;
    assert!((2..=40).contains(&animation_passes), "{animation_passes} sync passes");
    // A step without animation syncs once.
    assert_eq!(steps[2].1 - steps[1].1, 1);
    assert_eq!(steps[3].1 - steps[2].1, 1);
}
