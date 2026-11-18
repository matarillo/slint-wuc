// SPDX-License-Identifier: MIT OR Apache-2.0

// As `tests/probe/build.rs` compiles the gallery for the fork.
fn main() {
    let gallery = "../../reference/slint-1.19.0/examples/gallery/gallery.slint";
    // The MCP server finds elements through the debug info.
    let config = slint_build::CompilerConfiguration::new().with_debug_info(true);
    let out = std::path::Path::new(&std::env::var_os("OUT_DIR").unwrap()).join("gallery.rs");
    for dependency in slint_build::compile_with_output_path(gallery, out, config).unwrap() {
        println!("cargo:rerun-if-changed={}", dependency.display());
    }
}
