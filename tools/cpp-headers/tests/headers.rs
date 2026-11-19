// Copyright © Kentaro Inomata
// SPDX-License-Identifier: MIT-0

//! Generates the headers and checks that MSVC accepts them together with the handwritten headers
//! of upstream's C++ API.

use slint_fork_cpp_headers::{EnabledFeatures, gen_all};
use std::collections::BTreeSet;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Command;

const NO_FEATURES: EnabledFeatures = EnabledFeatures {
    gettext: false,
    image_pixel_format_rgb565: false,
    image_pixel_format_gray8: false,
    accessibility: false,
    mcp: false,
};

const ALL_FEATURES: EnabledFeatures = EnabledFeatures {
    gettext: true,
    image_pixel_format_rgb565: true,
    image_pixel_format_gray8: true,
    accessibility: true,
    mcp: true,
};

/// Handwritten headers of `private/` that this fork has no generated counterpart for:
/// `slint_window.h` calls `slint_windowrc_init` and the `slint_register_*` functions, and
/// `slint_models.h` includes `slint_platform_internal.h`. Both come from upstream's `api/cpp` crate,
/// which this fork doesn't have. `slint_item_tree.h` includes `slint_window.h`, and the other two
/// include `slint.h`, which includes `slint_platform_internal.h` and `slint_qt_internal.h`.
const HANDWRITTEN_WITHOUT_GENERATED_COUNTERPART: &[&str] = &[
    "slint_item_tree.h",
    "slint_live_preview.h",
    "slint_models.h",
    "slint_tests_helpers.h",
    "slint_window.h",
];

/// Items that have a vtable but no `SLINT_DECL_ITEM`. Upstream's list doesn't have
/// `ComponentContainer` either, and no C++ code refers to it.
const ITEMS_WITHOUT_DECLARATION: &[&str] = &["ComponentContainer"];

fn root_dir() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(2).unwrap()
}

fn handwritten_include_dir() -> PathBuf {
    root_dir().join("reference/slint-1.19.0/api/cpp/include")
}

/// Generates the headers into a fresh directory, and returns it.
fn generate(name: &str, features: EnabledFeatures) -> PathBuf {
    let out_dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("cpp-headers").join(name);
    match std::fs::remove_dir_all(&out_dir) {
        Err(e) if e.kind() != ErrorKind::NotFound => panic!("Cannot remove {out_dir:?}: {e}"),
        _ => {}
    }
    gen_all(root_dir(), &out_dir, features).expect("Cannot generate the headers");
    assert!(out_dir.join("private/slint_internal.h").is_file());
    out_dir
}

fn header_names(dir: &Path) -> BTreeSet<String> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|name| name.ends_with(".h"))
        .collect()
}

/// A translation unit that includes every generated header, followed by the handwritten ones
/// that build on them.
fn translation_unit(generated_dir: &Path) -> String {
    let generated = header_names(&generated_dir.join("private"));
    let handwritten = header_names(&handwritten_include_dir().join("private"));
    // The handwritten headers rely on the declarations of `slint_internal.h`, as `slint.h`
    // includes it first.
    let mut includes = vec!["slint_internal.h".to_string()];
    includes.extend(generated.iter().filter(|h| *h != "slint_internal.h").cloned());
    includes.extend(
        handwritten
            .iter()
            .filter(|h| {
                !generated.contains(*h)
                    && !HANDWRITTEN_WITHOUT_GENERATED_COUNTERPART.contains(&h.as_str())
            })
            .cloned(),
    );
    includes.iter().map(|h| format!("#include \"private/{h}\"\n")).collect()
}

fn check_compiles(name: &str, features: EnabledFeatures) {
    let generated_dir = generate(name, features);
    let source = generated_dir.join("all_headers.cpp");
    std::fs::write(&source, translation_unit(&generated_dir)).unwrap();

    let output = Command::new("cl.exe")
        .args(["/nologo", "/std:c++20", "/EHsc", "/Zs", "/utf-8"])
        .arg(format!("/I{}", generated_dir.display()))
        .arg(format!("/I{}", handwritten_include_dir().display()))
        .arg(&source)
        .output();
    let output = match output {
        Ok(output) => output,
        Err(e) if e.kind() == ErrorKind::NotFound => {
            eprintln!("note: cl.exe is not on the PATH, skipping the C++ compilation of {name}");
            return;
        }
        Err(e) => panic!("Cannot run cl.exe: {e}"),
    };
    assert!(
        output.status.success(),
        "cl.exe rejects the {name} headers:\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn headers_compile() {
    check_compiles("no-features", NO_FEATURES);
}

#[test]
fn headers_compile_with_all_features() {
    check_compiles("all-features", ALL_FEATURES);
}

#[test]
fn every_item_is_declared() {
    let generated_dir = generate("items", NO_FEATURES);
    let header = std::fs::read_to_string(generated_dir.join("private/slint_internal.h")).unwrap();
    let declared: BTreeSet<&str> = header
        .lines()
        .filter_map(|line| line.strip_prefix("SLINT_DECL_ITEM(")?.strip_suffix(");"))
        .collect();

    let items_rs = std::fs::read_to_string(root_dir().join("crates/core/items.rs")).unwrap();
    let with_vtable: BTreeSet<&str> = items_rs
        .lines()
        .filter_map(|line| line.trim().strip_prefix("fn slint_get_")?.split_once("VTable()"))
        .map(|(item, _)| item)
        .filter(|item| !ITEMS_WITHOUT_DECLARATION.contains(item))
        .collect();

    assert!(!declared.is_empty());
    assert_eq!(declared, with_vtable);
}
