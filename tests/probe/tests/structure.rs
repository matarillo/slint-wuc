//! Runs `probe-structure --check`: conditionals, z order, and repeaters over a changing model.

mod common;

/// The dump of the window: its root line, then `middle`.
fn window(middle: &[&str]) -> String {
    let mut lines = vec!["Sprite 0,0 200x100 #000000ff"];
    lines.extend_from_slice(middle);
    lines.join("\n")
}

const RED: &str = "  Sprite 0,0 10x10 #ff0000ff";
const GREEN: &str = "  Sprite 0,0 10x10 #00ff00ff";
const GRAY: &str = "  Sprite 20,0 10x10 #aaaaaaff";
const YELLOW: &str = "  Sprite 190,90 10x10 #ffff00ff";

fn row(i: u32, color: &str) -> String {
    format!("  Sprite {},20 10x10 {color}", 40 + i * 10)
}

#[test]
#[ignore = "opens a window, so it needs an interactive desktop session"]
fn structure_changes_keep_the_visuals_in_order() {
    let steps = common::run_check(env!("CARGO_BIN_EXE_probe-structure"));
    let step = |name: &str| {
        let step = steps.iter().find(|s| s.name == name).unwrap_or_else(|| panic!("{name}"));
        step.dump.as_str()
    };
    let rows = |colors: &[&str]| -> Vec<String> {
        colors.iter().enumerate().map(|(i, c)| row(i as u32, c)).collect()
    };
    let with = |head: &[&str], rows: &[String], tail: &[&str]| {
        let mut middle: Vec<&str> = head.to_vec();
        middle.extend(rows.iter().map(String::as_str));
        middle.extend_from_slice(tail);
        window(&middle)
    };
    let initial_rows = rows(&["#110000ff", "#220000ff", "#330000ff"]);

    assert_eq!(step("initial"), with(&[RED, GREEN, GRAY], &initial_rows, &[YELLOW]));
    // A higher z moves the red rectangle above all its siblings.
    assert_eq!(step("z-red = 1"), with(&[GREEN, GRAY], &initial_rows, &[YELLOW, RED]));
    assert_eq!(step("hide a"), with(&[GREEN], &initial_rows, &[YELLOW, RED]));
    // A conditional coming back takes its old place, not the top.
    assert_eq!(step("show a"), with(&[GREEN, GRAY], &initial_rows, &[YELLOW, RED]));
    assert_eq!(step("vec model"), with(&[GREEN, GRAY], &initial_rows, &[YELLOW, RED]));
    assert_eq!(
        step("insert at 1"),
        with(
            &[GREEN, GRAY],
            &rows(&["#110000ff", "#440000ff", "#220000ff", "#330000ff"]),
            &[YELLOW, RED]
        )
    );
    assert_eq!(
        step("remove 0"),
        with(&[GREEN, GRAY], &rows(&["#440000ff", "#220000ff", "#330000ff"]), &[YELLOW, RED])
    );
    let last_rows = rows(&["#550000ff", "#220000ff", "#330000ff"]);
    assert_eq!(step("set row 0"), with(&[GREEN, GRAY], &last_rows, &[YELLOW, RED]));
    // A repeater inside a conditional comes and goes with it.
    assert_eq!(
        step("show group"),
        with(
            &[GREEN, GRAY],
            &last_rows,
            &[
                "  Sprite 0,50 200x50 #0000ffff",
                "    Sprite 0,0 10x10 #ffffffff",
                "    Sprite 20,0 10x10 #ffffffff",
                YELLOW,
                RED
            ]
        )
    );
    assert_eq!(step("hide group"), with(&[GREEN, GRAY], &last_rows, &[YELLOW, RED]));

    // Each step changes something without animation, so it syncs exactly once.
    for pair in steps.windows(2) {
        assert_eq!(pair[1].passes - pair[0].passes, 1, "{}", pair[1].name);
    }
}
