// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

use rayon::prelude::*;
use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

mod exclusions;

/// The folders of the upstream cases that this driver runs, one test binary each.
///
/// This is every folder of `tests/cases`. Upstream's `widgets` folder runs once per style in
/// `widgets-<style>` binaries; the fork has the fluent style only, so the cases run once.
const CASE_FOLDERS: &[&str] = &[
    "7guis",
    "accessibility",
    "bindings",
    "callbacks",
    "children",
    "component_slots",
    "conditional",
    "crashes",
    "elements",
    "examples",
    "exports",
    "expr",
    "flexbox",
    "focus",
    "globals",
    "imports",
    "input",
    "interfaces",
    "issues",
    "layout",
    "legacy",
    "lookup",
    "match_element",
    "models",
    "popups",
    "properties",
    "subcomponents",
    "testing",
    "text",
    "translations",
    "types",
    "widgets",
];

/// The cases at the top of `tests/cases`. Upstream collects them in the library test of its
/// main.rs, so they have no test binary of their own.
const CASE_FILES: &[&str] = &[
    "absolute_coords.slint",
    "absolute_coords2.slint",
    "absolute_coords_with_transform.slint",
    "platform.slint",
    "simple.slint",
    "test_infrastructure.slint",
];

/// The upstream cases are read in place, not copied.
fn case_root() -> PathBuf {
    [env!("CARGO_MANIFEST_DIR"), "..", "..", "reference", "slint-1.19.0", "tests", "cases"]
        .iter()
        .collect()
}

fn make_generator_file(path: &Path) -> std::io::Result<BufWriter<File>> {
    Ok(BufWriter::new(File::create(path)?))
}

fn validate_test_file(base: &OsStr, path: &Path) -> std::io::Result<()> {
    let template = include_str!("template.rs");
    let expected = template.replace("{FILENAME}", &base.to_string_lossy());

    println!("cargo::rerun-if-env-changed=SLINT_UPDATE_TESTS");
    // If requested, update the file to match the template instead of failing.
    if std::env::var("SLINT_UPDATE_TESTS").is_ok() {
        std::fs::write(path, expected)?;
        println!("cargo:warning=Updated test file: {}", path.display());
        return Ok(());
    }

    assert!(std::fs::exists(path).unwrap_or_default(), "Missing test binary: {}", path.display());
    let file_contents = std::fs::read_to_string(path)?;
    let normalize = |s: &str| s.replace("\r\n", "\n").trim_end().to_string();

    assert_eq!(
        normalize(&file_contents),
        normalize(&expected),
        "Test file '{}' does not match template.\nRun with SLINT_UPDATE_TESTS=1 to update from template.",
        path.display(),
    );
    Ok(())
}

fn make_generator_files() -> std::io::Result<HashMap<OsString, BufWriter<File>>> {
    // Always re-generate all files, to ensure SLINT_TEST_FILTER can actually filter out test cases.
    let mut generated_files = HashMap::new();
    let tests_folder: PathBuf = [env!("CARGO_MANIFEST_DIR"), "tests"].iter().collect();

    for file in std::fs::read_dir(tests_folder)? {
        let file = file?.path();
        let base = file.file_stem().expect("Missing file name!");
        validate_test_file(base, &file)?;

        let generated_path =
            PathBuf::from(&std::env::var_os("OUT_DIR").unwrap()).join(file.file_name().unwrap());
        generated_files.insert(base.into(), make_generator_file(&generated_path)?);
    }
    Ok(generated_files)
}

fn generated_file_for_test<'a>(
    testcase: &test_driver_lib::TestCase,
    generated_files: &'a mut HashMap<OsString, BufWriter<File>>,
    fallback: &'a mut BufWriter<File>,
) -> &'a mut BufWriter<File> {
    // A case at the top of the case root has no folder: it goes to main.rs.
    if testcase.relative_path.components().count() == 1 {
        return fallback;
    }
    // For each folder in cases/ generate a separate file.
    // This allows splitting the test cases into multiple test binaries, which allows
    // parallelizing the compilation.
    let base = testcase.relative_path.iter().next().expect("A case is in a folder").to_owned();
    // The generated file hashmap is filled from the list of available test binaries.
    // Panic if we cannot find a generator file to write into, as that means there is no
    // corresponding test binary.
    generated_files
        .get_mut(&base)
        .unwrap_or_else(|| panic!("Missing test binary for subfolder: {}", base.display()))
}

/// The path of the case below the case root, with `/` as separator.
fn case_key(testcase: &test_driver_lib::TestCase) -> String {
    testcase.relative_path.to_string_lossy().replace('\\', "/")
}

fn exclusion_reason(
    list: &'static [(&'static str, &'static str)],
    testcase: &test_driver_lib::TestCase,
) -> Option<&'static str> {
    let key = case_key(testcase);
    list.iter().find(|(path, _)| *path == key).map(|(_, reason)| *reason)
}

fn main() -> std::io::Result<()> {
    println!("cargo::rerun-if-changed=exclusions.rs");

    let mut generated_file = make_generator_file(
        &Path::new(&std::env::var_os("OUT_DIR").unwrap()).join("generated.rs"),
    )?;

    let mut generated_files = make_generator_files()?;

    // A file as the root of the walk is the only entry of the walk.
    let folders_and_files: Vec<&str> = CASE_FOLDERS.iter().chain(CASE_FILES).copied().collect();
    let testcases = test_driver_lib::collect_test_cases(&case_root(), &folders_and_files)?;

    // A path that matches no case is a typo, or a case that upstream removed.
    // With a filter, the cases that the exclusion list names may be filtered out.
    if std::env::var_os("SLINT_TEST_FILTER").is_none() {
        for (path, _) in exclusions::IGNORED.iter().chain(exclusions::NOT_BUILT) {
            assert!(
                testcases.iter().any(|testcase| case_key(testcase) == *path),
                "exclusions.rs names '{path}', which is not a case"
            );
        }
    }

    // Generate the per-case modules on all cores: each case runs the Slint compiler
    // (twice with SLINT_TEST_DETERMINISTIC_OUTPUT), which dominates the build script's runtime.
    let results = rayon::ThreadPoolBuilder::new()
        // Kept low (< 1MB default stack on Windows) so that tests catch stack overflow earlier
        .stack_size(750 * 1024)
        .build()
        .expect("failed to create thread pool")
        .install(|| testcases.par_iter().map(process_case).collect::<Vec<_>>());

    // Report every case that the Slint compiler rejects, not only the first, so that the
    // exclusions can be listed in one run.
    let mut failed = 0;
    let mut module_lines = Vec::new();
    for (testcase, result) in testcases.iter().zip(results) {
        match result {
            Ok(module_line) => module_lines.push(module_line),
            Err(error) => {
                println!("cargo:warning=cannot generate {}: {error}", case_key(testcase));
                failed += 1;
            }
        }
    }
    if failed > 0 {
        return Err(std::io::Error::other(format!(
            "{failed} cases cannot be generated, see the warnings; list them in NOT_BUILT of exclusions.rs"
        )));
    }

    // Write the module declarations serially, in collection order, so the
    // including files don't depend on thread scheduling.
    for (testcase, module_line) in testcases.iter().zip(module_lines) {
        let output = generated_file_for_test(testcase, &mut generated_files, &mut generated_file);
        writeln!(output, "{module_line}")?;
    }

    generated_file.flush()?;
    for file in generated_files.values_mut() {
        file.flush()?;
    }

    //Make sure to use a consistent style
    println!("cargo:rustc-env=SLINT_STYLE=fluent");
    println!(
        "cargo:rustc-env=SLINT_ENABLE_EXPERIMENTAL_FEATURES={}",
        test_driver_lib::ALL_EXPERIMENTAL_FEATURES
    );
    println!("cargo:rustc-env=SLINT_EMIT_DEBUG_INFO=1");
    Ok(())
}

/// Write the `{module_name}.rs` file for the test case into OUT_DIR and return
/// the module declaration that includes it.
fn process_case(testcase: &test_driver_lib::TestCase) -> std::io::Result<String> {
    println!("cargo:rerun-if-changed={}", testcase.absolute_path.display());
    let mut module_name = testcase.identifier();
    if module_name.starts_with(|c: char| !c.is_ascii_alphabetic()) {
        module_name.insert(0, '_');
    }
    let module_line = format!("#[path=\"{module_name}.rs\"] pub mod r#{module_name};");
    let source = std::fs::read_to_string(&testcase.absolute_path)?;
    let ignored = exclusion_reason(exclusions::IGNORED, testcase)
        .map(|reason| format!("#[ignore = {reason:?}]"))
        .unwrap_or_default();

    let mut output = BufWriter::new(File::create(
        Path::new(&std::env::var_os("OUT_DIR").unwrap()).join(format!("{module_name}.rs")),
    )?);

    output.write_all(b"#![deny(warnings)]\n#![deny(rust_2018_idioms)]\n#![deny(unsafe_code)]\n")?;

    if testcase.is_ignored("rust") {
        writeln!(
            output,
            "#[test] #[ignore = \"testcase ignored for rust\"] fn ignored_for_rust() {{}}"
        )?;
        output.flush()?;
        return Ok(module_line);
    }

    if let Some(reason) = exclusion_reason(exclusions::NOT_BUILT, testcase) {
        writeln!(output, "#[test] #[ignore = {reason:?}] fn not_built() {{}}")?;
        output.flush()?;
        return Ok(module_line);
    }

    generate_source(&source, &mut output, testcase)?;

    for (i, x) in test_driver_lib::extract_test_functions(&source)
        .filter(|x| x.language_id == "rust")
        .enumerate()
    {
        // The closure's `Ok(())` keeps the snippet's last line a statement: a trailing
        // `if let` would otherwise trip the edition 2024 tail-expression drop-order lint.
        // The clippy allows are for the closure, for snippets that compare with `true`, and
        // for snippets that fail with `assert!(false, ..)`; the generated modules deny
        // warnings, which includes clippy's.
        write!(
            output,
            r"
#[rust_analyzer::skip]
#[allow(
    clippy::redundant_closure_call,
    clippy::bool_assert_comparison,
    clippy::assertions_on_constants
)]
#[test] {} fn t_{}() {{
    (|| -> ::std::result::Result<(), ::std::boxed::Box<dyn ::std::error::Error>> {{
        use i_slint_backend_testing as slint_testing;
        slint_testing::init_no_event_loop();
        slint_testing::configure_test_fonts();
        {}
        Ok(())
    }})()
    .unwrap()
}}",
            ignored,
            i,
            x.source.replace('\n', "\n        ")
        )?;
    }

    output.flush()?;
    Ok(module_line)
}

fn generate_source(
    source: &str,
    output: &mut impl Write,
    testcase: &test_driver_lib::TestCase,
) -> Result<(), std::io::Error> {
    let generated = compile_and_generate(source, testcase)?;

    println!("cargo::rerun-if-env-changed=SLINT_TEST_DETERMINISTIC_OUTPUT");
    if std::env::var_os("SLINT_TEST_DETERMINISTIC_OUTPUT").is_some_and(|value| !value.is_empty()) {
        let second = compile_and_generate(source, testcase)?;
        let expect_utf8 =
            |bytes| std::str::from_utf8(bytes).expect("generated Rust is valid UTF-8");
        assert_eq!(
            expect_utf8(&generated),
            expect_utf8(&second),
            "Non-deterministic compiler output for {:?}",
            testcase.absolute_path
        );
    }

    output.write_all(&generated)?;
    Ok(())
}

fn compile_and_generate(
    source: &str,
    testcase: &test_driver_lib::TestCase,
) -> Result<Vec<u8>, std::io::Error> {
    use i_slint_compiler::{diagnostics::BuildDiagnostics, *};

    // Relative to the case, as the `slint!` macro of upstream resolves them.
    let case_dir = testcase.absolute_path.parent().unwrap();
    let include_paths = test_driver_lib::extract_include_paths(source)
        .map(|path| case_dir.join(path))
        .collect::<Vec<_>>();
    let library_paths = test_driver_lib::extract_library_paths(source)
        .map(|(k, v)| (k.to_string(), case_dir.join(v)))
        .collect::<std::collections::HashMap<_, _>>();

    let mut diag = BuildDiagnostics::default();
    let syntax_node = parser::parse(
        source.to_owned(),
        Some(source_path::SourcePath::new(&testcase.absolute_path)),
        &mut diag,
    );
    let mut compiler_config = CompilerConfiguration::new(generator::OutputFormat::Rust);
    compiler_config.experimental_features = i_slint_compiler::ExperimentalFeatures::all();
    compiler_config.edition = Some(test_driver_lib::edition_or_current(source).parse().unwrap());
    compiler_config.include_paths = include_paths;
    compiler_config.library_paths = library_paths;
    compiler_config.style = Some("fluent".to_string());
    compiler_config.debug_info = true;
    if source.contains("//bundle-translations") {
        compiler_config.bundled_translations_path =
            Some(testcase.absolute_path.parent().unwrap().to_path_buf());
        compiler_config.translation_domain =
            Some(testcase.absolute_path.file_stem().unwrap().to_str().unwrap().to_string());
    }
    if source.contains("//no-default-translation-context") {
        compiler_config.default_translation_context =
            i_slint_compiler::DefaultTranslationContext::None;
    }
    let (root_component, diag, loader) =
        spin_on::spin_on(compile_syntax_node(syntax_node, diag, compiler_config));

    if diag.has_errors() {
        // Not `print_warnings_and_exit_on_error`: the other cases should report theirs too.
        let errors = diag
            .iter()
            .filter(|d| d.level() == diagnostics::DiagnosticLevel::Error)
            .map(|d| {
                let (line, column) = d.line_column();
                let file = d.source_file().and_then(|p| p.file_name()).unwrap_or_default();
                format!("{}:{line}:{column}: {}", file.to_string_lossy(), d.message())
            })
            .collect::<Vec<_>>()
            .join(" | ");
        return Err(std::io::Error::other(errors));
    } else {
        diag.print();
    }

    let mut output = Vec::new();
    generator::generate(
        generator::OutputFormat::Rust,
        &mut output,
        None,
        &root_component,
        &loader.compiler_config,
    )?;
    Ok(output)
}
