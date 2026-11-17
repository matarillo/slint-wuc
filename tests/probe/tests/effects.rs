// SPDX-License-Identifier: MIT OR Apache-2.0

//! Runs `probe-effects --check`: the Clip, Opacity, Transform and Layer elements that the
//! compiler inserts for `clip`, `opacity`, `transform-*` and `cache-rendering-hint`.

mod common;

#[test]
#[ignore = "opens a window, so it needs an interactive desktop session"]
fn lowered_elements_set_clip_opacity_and_transform() {
    let steps = common::run_check(env!("CARGO_BIN_EXE_probe-effects"));
    let names: Vec<_> = steps.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["initial", "unclipped", "opaque", "unturned"]);

    let dump = |clip: &str, opacity: &str, transform: &str| {
        format!(
            "\
Sprite 0,0 200x100 #000000ff
  Shape 10,10 40x40 [fill #ff0000ff rrect 0,0 40x40 r8]
    Container 0,0 40x40{clip}
      Sprite 20,20 40x40 #00ff00ff
  Container 60,10 40x40{opacity}
    Sprite 0,0 40x40 #0000ffff
  Container 110,10 40x20 transform=[{transform}]
    Sprite 0,0 40x20 #ffff00ff
  Container 160,10 20x20
    Sprite 0,0 20x20 #ffffffff"
        )
    };
    // The clip takes the rectangle's radius. The transform turns 90° and then doubles the
    // width, both about the default origin, the center (20,10).
    let (clip, faded, turned) = (" clip=rrect 0,0 40x40 r8", " opacity=0.5", "0 1 -2 0 40 -10");
    assert_eq!(steps[0].dump, dump(clip, faded, turned));
    assert_eq!(steps[1].dump, dump("", faded, turned));
    assert_eq!(steps[2].dump, dump("", "", turned));
    assert_eq!(steps[3].dump, dump("", "", "2 0 0 1 -20 0"));
    for pair in steps.windows(2) {
        assert_eq!(pair[1].passes - pair[0].passes, 1, "{}", pair[1].name);
    }
}
