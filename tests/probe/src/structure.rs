//! Shows `ui/structure.slint`; see `harness.rs` for `--check`.

slint::include_modules!();

mod harness;

use slint::{Color, Model, ModelRc, VecModel};
use std::rc::Rc;

thread_local! {
    /// The model the "vec model" step sets, which later steps insert into and remove from.
    static COLORS: Rc<VecModel<Color>> =
        Rc::new(VecModel::from(vec![red(0x11), red(0x22), red(0x33)]));
}

fn red(r: u8) -> Color {
    Color::from_rgb_u8(r, 0, 0)
}

fn main() {
    harness::run(
        Structure::new().unwrap(),
        vec![
            ("initial", |_| {}),
            ("z-red = 1", |s| s.set_z_red(1.)),
            ("hide a", |s| s.set_show_a(false)),
            ("show a", |s| s.set_show_a(true)),
            ("vec model", |s| s.set_colors(ModelRc::from(COLORS.with(Rc::clone)))),
            ("insert at 1", |_| COLORS.with(|m| m.insert(1, red(0x44)))),
            ("remove 0", |_| {
                COLORS.with(|m| m.remove(0));
            }),
            ("set row 0", |_| COLORS.with(|m| m.set_row_data(0, red(0x55)))),
            ("show group", |s| s.set_show_group(true)),
            ("hide group", |s| s.set_show_group(false)),
        ],
    );
}
