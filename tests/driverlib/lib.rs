// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;

/// The value of `SLINT_ENABLE_EXPERIMENTAL_FEATURES` that enables every experimental feature.
pub const ALL_EXPERIMENTAL_FEATURES: &str = "interfaces,shadowable,closures,named-slots,match,custom-mouse-cursor,component-container,new-with-existing-window,testable,internal-types";

#[test]
fn all_experimental_features_are_listed() {
    assert_eq!(
        ALL_EXPERIMENTAL_FEATURES,
        i_slint_compiler::ExperimentalFeatures::all().to_string()
    );
}

pub struct TestCase {
    pub absolute_path: PathBuf,
    /// The path below the case root, which starts with the name of the case's folder.
    pub relative_path: PathBuf,
}

impl TestCase {
    /// Return a string which is a valid C++/Rust identifier
    pub fn identifier(&self) -> String {
        self.relative_path
            .with_extension("")
            .to_string_lossy()
            .replace([std::path::MAIN_SEPARATOR, '-'], "_")
    }

    /// Returns true if the test case should be ignored for the specified driver.
    pub fn is_ignored(&self, driver: &str) -> bool {
        let source = std::fs::read_to_string(&self.absolute_path).unwrap();
        extract_ignores(&source).any(|ignored| ignored == driver)
    }
}

/// The directory that `entry` stands for, if git checked a symbolic link to a directory out as
/// a text file with the link's target, as it does without symlink support (on Windows, by
/// default). Upstream's `tests/cases/7guis` is such a link, to `examples/7guis`.
fn checked_out_symlink(entry: &Path) -> Option<PathBuf> {
    let metadata = std::fs::metadata(entry).ok()?;
    // A real symbolic link is followed by the walk.
    if !metadata.is_file() || metadata.len() > 512 {
        return None;
    }
    let target = std::fs::read_to_string(entry).ok()?;
    let target = target.trim();
    if target.is_empty() || target.contains(['\n', '\0']) {
        return None;
    }
    let target = entry.parent()?.join(target);
    target.is_dir().then_some(target)
}

/// Returns a list of all the `.slint` files in the `folders` below `case_root`, e.g. `tests/cases`.
///
/// An entry of `folders` may also name a file at the top of `case_root`, which is the only case
/// of that entry.
pub fn collect_test_cases(case_root: &Path, folders: &[&str]) -> std::io::Result<Vec<TestCase>> {
    let mut results = Vec::new();

    println!("cargo:rerun-if-env-changed=SLINT_TEST_FILTER");
    let filter = std::env::var("SLINT_TEST_FILTER").ok();

    for folder in folders {
        let entry_path = case_root.join(folder);
        // The cases are named as below `case_root`, even where the walk goes elsewhere.
        let walk_root = checked_out_symlink(&entry_path).unwrap_or_else(|| entry_path.clone());
        let walker = walkdir::WalkDir::new(&walk_root).follow_links(true).sort_by_file_name();
        for entry in walker {
            let entry = entry?;
            if entry.file_type().is_dir() {
                println!("cargo:rerun-if-changed={}", entry.into_path().display());
                continue;
            }
            let absolute_path = entry.into_path();
            let within_entry = absolute_path.strip_prefix(&walk_root).unwrap();
            // `join` of an empty path appends a separator: a file that is the entry itself.
            let relative_path = if within_entry.as_os_str().is_empty() {
                PathBuf::from(folder)
            } else {
                Path::new(folder).join(within_entry)
            };
            // The filter uses `/` on every platform.
            if let Some(filter) = &filter
                && !relative_path.to_str().unwrap().replace('\\', "/").contains(filter)
            {
                continue;
            }
            if let Some(ext) = absolute_path.extension()
                && (ext == "60" || ext == "slint")
            {
                results.push(TestCase { absolute_path, relative_path });
            }
        }
    }
    Ok(results)
}

/// A test functions looks something like
/// ````text
/// /*
///   ```rust
///   let instance = TestCase::new().unwrap();
///   assert_eq!(instance.get_x(), 0);
///   ```
/// */
/// ````
pub struct TestFunction<'a> {
    /// In the example above: `rust`
    pub language_id: &'a str,
    /// The content of the test function
    pub source: &'a str,
}

/// Extract the test functions from
pub fn extract_test_functions(source: &str) -> impl Iterator<Item = TestFunction<'_>> {
    static RX: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?sU)\r?\n```([a-z][a-z.]*)\r?\n(.+)\r?\n```\r?\n").unwrap());
    RX.captures_iter(source).map(|mat| TestFunction {
        language_id: mat.get(1).unwrap().as_str(),
        source: mat.get(2).unwrap().as_str(),
    })
}

#[test]
fn test_extract_test_functions() {
    let source = r"
/*
```cpp
auto xx = 0;
auto yy = 0;
```

```rust
let xx = 0;
let yy = 0;
```
*/
";
    let mut r = extract_test_functions(source);

    let r1 = r.next().unwrap();
    assert_eq!(r1.language_id, "cpp");
    assert_eq!(r1.source, "auto xx = 0;\nauto yy = 0;");

    let r2 = r.next().unwrap();
    assert_eq!(r2.language_id, "rust");
    assert_eq!(r2.source, "let xx = 0;\nlet yy = 0;");
}

#[test]
fn test_extract_test_functions_win() {
    let source = "/*\r\n```cpp\r\nfoo\r\nbar\r\n```\r\n*/\r\n";
    let mut r = extract_test_functions(source);
    let r1 = r.next().unwrap();
    assert_eq!(r1.language_id, "cpp");
    assert_eq!(r1.source, "foo\r\nbar");
}

/// Extract extra include paths from a comment in the source if present.
pub fn extract_include_paths(source: &str) -> impl Iterator<Item = &'_ str> {
    static RX: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"//include_path:\s*(.+)\s*\n").unwrap());
    RX.captures_iter(source).map(|mat| mat.get(1).unwrap().as_str().trim())
}

#[test]
fn test_extract_include_paths() {
    assert!(extract_include_paths("something").next().is_none());

    let source = r"
    //include_path: ../first
    //include_path: ../second
    Blah {}
";

    let r = extract_include_paths(source).collect::<Vec<_>>();
    assert_eq!(r, ["../first", "../second"]);

    // Windows \r\n
    let source = "//include_path: ../first\r\n//include_path: ../second\r\nBlah {}\r\n";
    let r = extract_include_paths(source).collect::<Vec<_>>();
    assert_eq!(r, ["../first", "../second"]);
}

/// Extract extra library paths from a comment in the source if present.
pub fn extract_library_paths(source: &str) -> impl Iterator<Item = (&'_ str, &'_ str)> {
    static RX: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"//library_path\((.+)\):\s*(.+)\s*\n").unwrap());
    RX.captures_iter(source)
        .map(|mat| (mat.get(1).unwrap().as_str().trim(), mat.get(2).unwrap().as_str().trim()))
}

#[test]
fn test_extract_library_paths() {
    use std::collections::HashMap;

    assert!(extract_library_paths("something").next().is_none());

    let source = r"
    //library_path(first): ../first/lib.slint
    //library_path(second): ../second/lib.slint
    Blah {}
";

    let r = extract_library_paths(source).collect::<HashMap<_, _>>();
    assert_eq!(
        r,
        HashMap::from([("first", "../first/lib.slint"), ("second", "../second/lib.slint")])
    );
}

/// Extract `//ignore` comments from the source.
fn extract_ignores(source: &str) -> impl Iterator<Item = &'_ str> {
    static RX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"//ignore:\s*(.+)\s*\n").unwrap());
    RX.captures_iter(source).flat_map(|mat| {
        mat.get(1).unwrap().as_str().split(&[' ', ',']).map(str::trim).filter(|s| !s.is_empty())
    })
}

#[test]
fn test_extract_ignores() {
    assert!(extract_ignores("something").next().is_none());

    let source = r"
    //ignore: cpp
    //ignore: rust, nodejs
    Blah {}
";

    let r = extract_ignores(source).collect::<Vec<_>>();
    assert_eq!(r, ["cpp", "rust", "nodejs"]);
}

fn extract_edition(source: &str) -> Option<&str> {
    static RX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"//edition:\s*(\S+)\s*\n").unwrap());
    RX.captures(source).map(|mat| mat.get(1).unwrap().as_str())
}

/// The edition that `source` selects, or the one of this Slint release.
pub fn edition_or_current(source: &str) -> &str {
    extract_edition(source).unwrap_or(concat!(
        env!("CARGO_PKG_VERSION_MAJOR"),
        ".",
        env!("CARGO_PKG_VERSION_MINOR")
    ))
}

#[test]
fn test_extract_edition() {
    assert!(extract_edition("something").is_none());
    assert_eq!(extract_edition("//edition: 1.18\nBlah {}\n"), Some("1.18"));
    assert_eq!(extract_edition("//edition: 1.18\r\nBlah {}\r\n"), Some("1.18"));
}
