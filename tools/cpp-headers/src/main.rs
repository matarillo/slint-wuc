// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

//! Writes the C++ headers of the C ABI into `<out-dir>/private`.

use slint_fork_cpp_headers::{EnabledFeatures, gen_all};
use std::path::Path;

fn main() -> anyhow::Result<()> {
    let Some(out_dir) = std::env::args_os().nth(1) else {
        eprintln!("Usage: slint-fork-cpp-headers <out-dir>");
        eprintln!("Set CARGO_FEATURE_<NAME> (for example CARGO_FEATURE_IMAGE_PIXEL_FORMAT_GRAY8)");
        eprintln!("to define the matching SLINT_FEATURE_<NAME> macro in the headers.");
        std::process::exit(2);
    };

    // Go from $root/tools/cpp-headers up to $root
    let root_dir = Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(2).unwrap();

    gen_all(root_dir, Path::new(&out_dir), EnabledFeatures::from_env())?;
    Ok(())
}
