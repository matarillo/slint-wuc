//! Snapshot tests: what the visual tree of a window looks like after each kind of change.
//!
//! Every test goes through the real context and window path (a headless platform, a detached
//! window, `set_component`, `show`) and reads the visuals back with `dump_visuals`. Positions
//! and sizes in a dump are logical pixels, whatever the scale factor.

use i_slint_core::api::{LogicalSize, PhysicalSize, WindowSize};
use i_slint_core::item_tree::{ItemTreeRc, ItemTreeVTable};
use i_slint_core::platform::WindowEvent;
use i_slint_core::{Brush, Color};
use slint_fork_fixtures::headless::changed_lines;
use slint_fork_fixtures::nested_rects::Rect;
use slint_fork_fixtures::{Headless, NestedRects, ResizeFollower};
use vtable::VRc;

const NESTED_RECTS: &str = "\
Sprite 0,0 400x300 #202020ff
  Sprite 10,10 180x280 #a03030ff
    Sprite 10,10 160x120 #e0c020ff
    Sprite 10,150 160x120 #20c0c0ff
  Sprite 210,10 180x280 #3050a0ff
    Sprite 20,20 140x240 #30a030ff
";

const RESIZE_FOLLOWER: &str = "\
Sprite 0,0 400x300 #202020ff
  Sprite 100,75 200x150 #c04080ff
  Sprite 0,0 20x20 #e0e0e0ff
";

fn solid(r: u8, g: u8, b: u8) -> Brush {
    Brush::SolidColor(Color::from_rgb_u8(r, g, b))
}

fn transparent() -> Brush {
    Brush::SolidColor(Color::from_argb_u8(0, 0, 0, 0))
}

/// A 400x300 window showing a `NestedRects`.
fn nested_rects() -> (Headless, VRc<ItemTreeVTable, NestedRects>) {
    let headless = Headless::new(PhysicalSize::new(400, 300));
    let tree = NestedRects::new(headless.adapter());
    headless.show(&VRc::into_dyn(tree.clone()));
    (headless, tree)
}

/// A 400x300 window showing a `ResizeFollower`.
fn resize_follower() -> (Headless, VRc<ItemTreeVTable, ResizeFollower>) {
    let headless = Headless::new(PhysicalSize::new(400, 300));
    let tree = ResizeFollower::new(headless.adapter());
    headless.show(&VRc::into_dyn(tree.clone()));
    (headless, tree)
}

/// `dump` with the nth line replaced.
fn with_line(dump: &str, index: usize, line: &str) -> String {
    let mut lines: Vec<&str> = dump.lines().collect();
    lines[index] = line;
    lines.iter().map(|l| format!("{l}\n")).collect()
}

#[test]
fn nested_rects_initial_tree() {
    let (headless, _tree) = nested_rects();
    assert_eq!(headless.dump(), NESTED_RECTS);
}

#[test]
fn resize_follower_initial_tree() {
    let (headless, _tree) = resize_follower();
    assert_eq!(headless.dump(), RESIZE_FOLLOWER);
}

#[test]
fn a_sync_pass_without_changes_changes_nothing() {
    let (headless, _tree) = nested_rects();
    let before = headless.dump();
    headless.sync();
    assert_eq!(headless.dump(), before);
}

#[test]
fn a_background_change_touches_only_its_visual() {
    let (headless, tree) = nested_rects();
    let before = headless.dump();

    tree.as_pin_ref().set_background(Rect::LeftBottom, solid(0xff, 0, 0));
    let after = headless.dump();
    assert_eq!(after, with_line(NESTED_RECTS, 3, "    Sprite 10,150 160x120 #ff0000ff"));
    assert_eq!(changed_lines(&before, &after), [3]);

    // A parent's change leaves its children, and their order, alone.
    tree.as_pin_ref().set_background(Rect::Left, solid(0, 0, 0xff));
    let after_parent = headless.dump();
    assert_eq!(changed_lines(&after, &after_parent), [1]);
    assert_eq!(after_parent.lines().nth(1), Some("  Sprite 10,10 180x280 #0000ffff"));

    // Several changes between two passes land in one.
    tree.as_pin_ref().set_background(Rect::LeftTop, solid(1, 2, 3));
    tree.as_pin_ref().set_background(Rect::RightInner, solid(4, 5, 6));
    tree.as_pin_ref().set_window_background(solid(7, 8, 9));
    let after_three = headless.dump();
    assert_eq!(changed_lines(&after_parent, &after_three), [0, 2, 5]);
    assert_eq!(
        after_three,
        "\
Sprite 0,0 400x300 #070809ff
  Sprite 10,10 180x280 #0000ffff
    Sprite 10,10 160x120 #010203ff
    Sprite 10,150 160x120 #ff0000ff
  Sprite 210,10 180x280 #3050a0ff
    Sprite 20,20 140x240 #040506ff
"
    );
}

#[test]
fn a_translucent_background_stays_a_sprite() {
    let (headless, tree) = nested_rects();
    tree.as_pin_ref().set_background(
        Rect::Right,
        Brush::SolidColor(Color::from_argb_u8(0x80, 0x30, 0x50, 0xa0)),
    );
    assert_eq!(headless.dump(), with_line(NESTED_RECTS, 4, "  Sprite 210,10 180x280 #3050a080"));
}

#[test]
fn a_background_becoming_transparent_and_back_keeps_its_place() {
    let (headless, tree) = nested_rects();

    // A leaf between a sibling before it and none after, and a parent with children.
    for (rect, line, transparent_line, solid_line) in [
        (
            Rect::LeftBottom,
            3,
            "    Container 10,150 160x120",
            "    Sprite 10,150 160x120 #20c0c0ff",
        ),
        (Rect::LeftTop, 2, "    Container 10,10 160x120", "    Sprite 10,10 160x120 #e0c020ff"),
        (Rect::Left, 1, "  Container 10,10 180x280", "  Sprite 10,10 180x280 #a03030ff"),
        (Rect::Right, 4, "  Container 210,10 180x280", "  Sprite 210,10 180x280 #3050a0ff"),
    ] {
        tree.as_pin_ref().set_background(rect, transparent());
        assert_eq!(
            headless.dump(),
            with_line(NESTED_RECTS, line, transparent_line),
            "{rect:?} transparent"
        );

        let color = rect.initial().1;
        tree.as_pin_ref().set_background(rect, Brush::SolidColor(color));
        assert_eq!(headless.dump(), NESTED_RECTS, "{rect:?} back to {solid_line}");
    }

    // Two at once, so that the container between them has to keep its children in order.
    tree.as_pin_ref().set_background(Rect::Left, transparent());
    tree.as_pin_ref().set_background(Rect::LeftTop, transparent());
    assert_eq!(
        headless.dump(),
        "\
Sprite 0,0 400x300 #202020ff
  Container 10,10 180x280
    Container 10,10 160x120
    Sprite 10,150 160x120 #20c0c0ff
  Sprite 210,10 180x280 #3050a0ff
    Sprite 20,20 140x240 #30a030ff
"
    );
    tree.as_pin_ref().set_background(Rect::LeftTop, solid(0xe0, 0xc0, 0x20));
    tree.as_pin_ref().set_background(Rect::Left, solid(0xa0, 0x30, 0x30));
    assert_eq!(headless.dump(), NESTED_RECTS);
}

#[test]
fn a_transparent_window_background_is_a_container() {
    let (headless, tree) = nested_rects();
    tree.as_pin_ref().set_window_background(transparent());
    assert_eq!(headless.dump(), with_line(NESTED_RECTS, 0, "Container 0,0 400x300"));
}

#[test]
fn a_geometry_change_moves_and_resizes_one_visual() {
    let (headless, tree) = nested_rects();
    let before = headless.dump();

    tree.as_pin_ref()
        .set_geometry(Rect::RightInner, slint_fork_fixtures::rect(30., 40., 100., 50.));
    let after = headless.dump();
    assert_eq!(after, with_line(NESTED_RECTS, 5, "    Sprite 30,40 100x50 #30a030ff"));
    assert_eq!(changed_lines(&before, &after), [5]);

    // A parent moves with its children: their offsets are relative, so their lines don't change.
    tree.as_pin_ref().set_geometry(Rect::Right, slint_fork_fixtures::rect(200., 20., 150., 200.));
    let after_parent = headless.dump();
    assert_eq!(changed_lines(&after, &after_parent), [4]);
    assert_eq!(after_parent.lines().nth(4), Some("  Sprite 200,20 150x200 #3050a0ff"));
}

#[test]
fn a_resize_moves_and_resizes_what_follows_the_window() {
    let (headless, tree) = resize_follower();
    let before = headless.dump();

    headless.window().set_size(PhysicalSize::new(600, 200));
    let after = headless.dump();
    assert_eq!(
        after,
        "\
Sprite 0,0 600x200 #202020ff
  Sprite 150,50 300x100 #c04080ff
  Sprite 0,0 20x20 #e0e0e0ff
"
    );
    // The corner doesn't follow anything.
    assert_eq!(changed_lines(&before, &after), [0, 1]);

    // A logical size resizes the same way; at scale factor 1 it's the same number of pixels.
    headless.window().set_size(WindowSize::Logical(LogicalSize::new(100., 40.)));
    assert_eq!(
        headless.dump(),
        "\
Sprite 0,0 100x40 #202020ff
  Sprite 25,10 50x20 #c04080ff
  Sprite 0,0 20x20 #e0e0e0ff
"
    );
    assert_eq!(headless.window().size(), PhysicalSize::new(100, 40));

    // And back.
    headless.window().set_size(PhysicalSize::new(400, 300));
    assert_eq!(headless.dump(), RESIZE_FOLLOWER);

    // The binding belongs to the geometry, so the fixture's own changes still show.
    tree.as_pin_ref().set_follower_background(solid(0, 0xff, 0));
    tree.as_pin_ref().set_corner_geometry(slint_fork_fixtures::rect(5., 6., 7., 8.));
    assert_eq!(
        headless.dump(),
        "\
Sprite 0,0 400x300 #202020ff
  Sprite 100,75 200x150 #00ff00ff
  Sprite 5,6 7x8 #e0e0e0ff
"
    );
}

#[test]
fn a_resize_leaves_unbound_rectangles_where_they_are() {
    let (headless, _tree) = nested_rects();
    headless.window().set_size(PhysicalSize::new(500, 320));
    assert_eq!(headless.dump(), with_line(NESTED_RECTS, 0, "Sprite 0,0 500x320 #202020ff"));
}

#[test]
fn a_scale_factor_change_keeps_the_dump_in_logical_units() {
    let (headless, _tree) = resize_follower();
    assert_eq!(headless.window().scale_factor(), 1.0);
    assert_eq!(headless.root_visual().Scale().unwrap().X, 1.0);

    headless.window().dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: 2.0 });
    assert_eq!(headless.window().scale_factor(), 2.0);
    // The window keeps its physical size, so the logical size halves once the host says so.
    headless.window().set_size(PhysicalSize::new(400, 300));
    assert_eq!(headless.window().size(), PhysicalSize::new(400, 300));
    assert_eq!(
        headless.dump(),
        "\
Sprite 0,0 200x150 #202020ff
  Sprite 50,37.5 100x75 #c04080ff
  Sprite 0,0 20x20 #e0e0e0ff
"
    );

    // A window that doubles its pixels keeps its logical size, as it does when it moves to a
    // monitor with twice the density.
    headless.window().set_size(PhysicalSize::new(800, 600));
    assert_eq!(headless.dump(), RESIZE_FOLLOWER);

    // A logical size is converted with the new scale factor.
    headless.window().set_size(WindowSize::Logical(LogicalSize::new(100., 40.)));
    assert_eq!(headless.window().size(), PhysicalSize::new(200, 80));
    assert_eq!(
        headless.dump(),
        "\
Sprite 0,0 100x40 #202020ff
  Sprite 25,10 50x20 #c04080ff
  Sprite 0,0 20x20 #e0e0e0ff
"
    );
}

#[test]
fn a_scale_factor_change_scales_the_root_visual() {
    let (headless, _tree) = nested_rects();
    let scale = || headless.root_visual().Scale().unwrap();
    assert_eq!((scale().X, scale().Y, scale().Z), (1.0, 1.0, 1.0));

    headless.window().dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: 2.0 });
    headless.sync();
    assert_eq!((scale().X, scale().Y, scale().Z), (2.0, 2.0, 1.0));
    // Items keep their logical geometry; the root visual's scale does the rest.
    assert_eq!(headless.dump(), NESTED_RECTS);

    headless.window().dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: 1.5 });
    headless.sync();
    assert_eq!((scale().X, scale().Y, scale().Z), (1.5, 1.5, 1.0));
}

/// The first child of the root visual, which a test can keep to see what happens to it.
fn first_visual(headless: &Headless) -> windows::UI::Composition::Visual {
    let children = headless.root_visual().Children().unwrap();
    (&children).into_iter().next().expect("a visual below the root")
}

#[test]
fn dropping_a_component_releases_its_visuals() {
    let (headless, tree) = nested_rects();
    assert_eq!(headless.dump(), NESTED_RECTS);
    let visual = first_visual(&headless);
    assert!(visual.Parent().is_ok());

    // The window keeps its component alive while it's shown.
    headless.hide();
    assert_eq!(headless.dump(), NESTED_RECTS, "hiding the window doesn't destroy the component");
    drop(tree);

    assert_eq!(headless.dump(), "");
    assert_eq!(headless.root_visual().Children().unwrap().Count().unwrap(), 0);
    assert!(visual.Parent().is_err(), "the window item's visual is detached");
    assert_eq!(headless.node_count(), 0, "the window keeps no node for a dropped item");
}

#[test]
fn setting_another_component_replaces_the_visuals() {
    let (headless, nested) = nested_rects();
    let follower = ResizeFollower::new(headless.adapter());
    let nested_visual = first_visual(&headless);

    headless.set_component(&VRc::into_dyn(follower.clone()));
    assert_eq!(headless.dump(), RESIZE_FOLLOWER);
    assert!(nested_visual.Parent().is_err(), "the old component's visuals leave the window");
    // `NestedRects` is still alive, shown nowhere.
    assert_eq!(nested.as_pin_ref().background(Rect::Left), solid(0xa0, 0x30, 0x30));

    // The window still holds the component it was shown with, as upstream's `show` does,
    // so the old component keeps its nodes until the window is hidden.
    let follower_nodes = RESIZE_FOLLOWER.lines().count();
    drop(nested);
    assert_eq!(headless.dump(), RESIZE_FOLLOWER);
    assert!(headless.node_count() > follower_nodes);
    headless.hide();
    assert_eq!(headless.node_count(), follower_nodes, "the old component's nodes are released");
    follower.as_pin_ref().set_follower_background(solid(1, 2, 3));
    assert_eq!(headless.dump(), with_line(RESIZE_FOLLOWER, 1, "  Sprite 100,75 200x150 #010203ff"));
}

#[test]
fn a_component_that_comes_back_is_shown_again() {
    let (headless, nested) = nested_rects();
    let nested_dyn: ItemTreeRc = VRc::into_dyn(nested.clone());
    let follower = ResizeFollower::new(headless.adapter());

    headless.set_component(&VRc::into_dyn(follower.clone()));
    assert_eq!(headless.dump(), RESIZE_FOLLOWER);

    // While it was away, its properties changed; its visuals catch up when it returns.
    nested.as_pin_ref().set_background(Rect::LeftTop, solid(1, 2, 3));
    headless.set_component(&nested_dyn);
    assert_eq!(headless.dump(), with_line(NESTED_RECTS, 2, "    Sprite 10,10 160x120 #010203ff"));
}
