// SPDX-License-Identifier: MIT OR Apache-2.0

//! Shows upstream's `examples/custom-titlebar`: a `no-frame` window with a transparent background
//! that draws its own rounded corners and title bar. The example has no Rust code of its own.

include!(concat!(env!("OUT_DIR"), "/titlebar.rs"));

fn main() {
    MainWindow::new().unwrap().run().unwrap();
}
