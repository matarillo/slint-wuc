// Copyright © Kentaro Inomata
// SPDX-License-Identifier: MIT-0

//! Runs `probe-scrolling --check`: a Flickable clips to itself, and scrolling moves only the
//! viewport, the child the compiler puts between the Flickable and its content.

mod common;

#[test]
#[ignore = "opens a window, so it needs an interactive desktop session"]
fn a_flickable_clips_and_scrolls_its_viewport() {
    let steps = common::run_check(env!("CARGO_BIN_EXE_probe-scrolling"));
    let names: Vec<_> = steps.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["initial", "scrolled"]);

    let dump = |viewport_y: i32| {
        format!(
            "\
Sprite 0,0 120x120 #000000ff
  Container 10,10 100x100 clip=inset(0,0,0,0)
    Container 0,{viewport_y} 100x300
      Sprite 0,0 100x100 #ff0000ff
      Sprite 0,100 100x100 #00ff00ff
      Sprite 0,200 100x100 #0000ffff"
        )
    };
    assert_eq!(steps[0].dump, dump(0));
    assert_eq!(steps[1].dump, dump(-150));
    assert_eq!(steps[1].passes - steps[0].passes, 1);
}
