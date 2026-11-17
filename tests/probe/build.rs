// SPDX-License-Identifier: MIT OR Apache-2.0

fn main() {
    slint_build::compile("ui/main.slint").unwrap();
    // Upstream's gallery exports `App`, as `basics.slint` does, so it gets a module of its own.
    let gallery = "../../reference/slint-1.19.0/examples/gallery/gallery.slint";
    let out = std::path::Path::new(&std::env::var_os("OUT_DIR").unwrap()).join("gallery.rs");
    for dependency in
        slint_build::compile_with_output_path(gallery, out, Default::default()).unwrap()
    {
        println!("cargo:rerun-if-changed={}", dependency.display());
    }
}
