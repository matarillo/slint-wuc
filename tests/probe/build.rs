// SPDX-License-Identifier: MIT OR Apache-2.0

fn main() {
    slint_build::compile("ui/main.slint").unwrap();
    // Upstream's gallery exports `App`, as `basics.slint` does, so it gets a module of its own.
    let gallery = "../../reference/slint-1.19.0/examples/gallery/gallery.slint";
    // The MCP server finds elements through the debug info.
    let config = slint_build::CompilerConfiguration::new().with_debug_info(true);
    let out = std::path::Path::new(&std::env::var_os("OUT_DIR").unwrap()).join("gallery.rs");
    for dependency in slint_build::compile_with_output_path(gallery, out, config).unwrap() {
        println!("cargo:rerun-if-changed={}", dependency.display());
    }
    let tray = "../../reference/slint-1.19.0/examples/system-tray/system-tray.slint";
    let out = std::path::Path::new(&std::env::var_os("OUT_DIR").unwrap()).join("tray.rs");
    let config = slint_build::CompilerConfiguration::new();
    for dependency in slint_build::compile_with_output_path(tray, out, config).unwrap() {
        println!("cargo:rerun-if-changed={}", dependency.display());
    }
}
