// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

//! Checks that the items of this runtime match the names the upstream compiler knows,
//! and keeps the list of items that `update_visual` doesn't show yet.
//! See `docs/design/conformance.md`.

use i_slint_compiler::ExperimentalFeatures;
use i_slint_compiler::element_type::{BuiltinElement, ElementType};
use i_slint_compiler::langtype::Type;
use i_slint_compiler::native_class::NativeClass;
use i_slint_compiler::typeregister::TypeRegister;
use i_slint_core::items;
use i_slint_core::rtti::BuiltinItem;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// A value type for `BuiltinItem::properties` and friends, which only the names are read from.
/// Every conversion fails.
#[derive(Default, Clone, PartialEq)]
struct Never;

// The type list of `i_slint_core::rtti::ValueType`.
// `Never` stops implementing `ValueType`, and this file stops compiling, when that list grows.
macro_rules! never_conversions {
    ($( $(#[$enum_doc:meta])* $vis:vis enum $Name:ident { $($body:tt)* })*) => {
        never_conversions![@types
            (),
            bool,
            u32,
            u64,
            i32,
            i64,
            f32,
            f64,
            i_slint_core::SharedString,
            i_slint_core::graphics::Image,
            i_slint_core::Color,
            i_slint_core::graphics::PathData,
            i_slint_core::SharedVector<i_slint_core::Coord>,
            i_slint_core::animations::EasingCurve,
            i_slint_core::items::StandardListViewItem,
            i_slint_core::items::TableColumn,
            i_slint_core::input::KeyEvent,
            i_slint_core::Brush,
            i_slint_core::graphics::Point,
            i_slint_core::items::PointerEvent,
            i_slint_core::items::PointerScrollEvent,
            i_slint_core::lengths::LogicalLength,
            i_slint_core::component_factory::ComponentFactory,
            i_slint_core::api::LogicalPosition,
            i_slint_core::api::LogicalSize,
            i_slint_core::items::Edges,
            i_slint_core::items::FontMetrics,
            i_slint_core::items::InputMethodHints,
            i_slint_core::items::MenuEntry,
            i_slint_core::items::DropEvent,
            i_slint_core::model::ModelRc<i_slint_core::items::MenuEntry>,
            i_slint_core::styled_text::StyledText,
            i_slint_core::input::Keys,
            i_slint_core::data_transfer::DataTransfer,
            i_slint_core::cursor::MouseCursorInner,
            $(i_slint_core::items::$Name,)*
        ];
    };
    (@types $($ty:ty,)*) => {
        $(
            impl TryFrom<$ty> for Never {
                type Error = ();
                fn try_from(_: $ty) -> Result<Self, ()> {
                    Err(())
                }
            }
            impl TryFrom<Never> for $ty {
                type Error = ();
                fn try_from(_: Never) -> Result<Self, ()> {
                    Err(())
                }
            }
        )*
    };
}

i_slint_common::for_each_enums!(never_conversions);

impl i_slint_core::rtti::ValueType for Never {}

/// The names of the members of a runtime item, in the kebab-case the compiler uses.
#[derive(Default)]
struct RuntimeItem {
    properties: BTreeSet<String>,
    callbacks: BTreeSet<String>,
    fields: BTreeSet<String>,
}

fn runtime_item<T: BuiltinItem + 'static>() -> (String, RuntimeItem) {
    let item = RuntimeItem {
        properties: T::properties::<Never>().into_iter().map(|(n, _)| n.to_string()).collect(),
        callbacks: T::callbacks::<Never>().into_iter().map(|(n, _)| n.to_string()).collect(),
        fields: T::fields::<Never>().into_iter().map(|(n, _)| n.to_string()).collect(),
    };
    (T::name().to_string(), item)
}

/// The items the upstream interpreter registers in `item_registry.rs`,
/// without the ones of the backend's native widgets.
fn runtime_items() -> BTreeMap<String, RuntimeItem> {
    macro_rules! items {
        ($($ty:ident),* $(,)?) => {
            BTreeMap::from([$(runtime_item::<items::$ty>()),*])
        };
    }
    items!(
        ComponentContainer,
        Empty,
        ImageItem,
        ClippedImage,
        ComplexText,
        StyledTextItem,
        SimpleText,
        Rectangle,
        BasicBorderRectangle,
        BorderRectangle,
        TouchArea,
        TooltipArea,
        FocusScope,
        KeyBinding,
        SwipeGestureHandler,
        ScaleRotateGestureHandler,
        Path,
        Flickable,
        WindowItem,
        TextInput,
        Clip,
        BoxShadow,
        BackdropBlur,
        Transform,
        Opacity,
        Layer,
        DragArea,
        DropArea,
        WindowMoveArea,
        ContextMenu,
        MenuItem,
        SystemTrayIcon,
    )
}

/// A native class of the compiler.
struct Class {
    native_class: Arc<NativeClass>,
    /// Some element that has the class in its chain can end up as an item.
    lowers_to_item: bool,
}

struct Compiler {
    classes: BTreeMap<String, Class>,
    globals: BTreeSet<String>,
}

fn compiler() -> Compiler {
    let register = TypeRegister::builtin();
    // `all_elements` leaves out the elements of features that are off.
    register.borrow_mut().set_experimental_features(ExperimentalFeatures::all());
    let register = register.borrow();

    let mut globals = BTreeSet::new();
    let mut pending = Vec::new();
    // `Empty` is kept apart from the other elements.
    // Elements that only a parent element accepts as a child are reached through that parent.
    for element in register.all_elements().into_values().chain([register.empty_type()]) {
        match element {
            ElementType::Builtin(element) => pending.push(element),
            // A builtin global is a component with the builtin element as its root.
            ElementType::Component(component) => {
                if let ElementType::Builtin(element) = &component.root_element.borrow().base_type
                    && element.is_global
                {
                    globals.insert(element.name.to_string());
                }
            }
            _ => {}
        }
    }

    let mut elements = BTreeMap::<String, Arc<BuiltinElement>>::new();
    while let Some(element) = pending.pop() {
        if elements.insert(element.name.to_string(), element.clone()).is_none() {
            pending.extend(element.additional_accepted_child_types.values().cloned());
        }
    }

    let mut classes = BTreeMap::<String, Class>::new();
    for element in elements.values() {
        let mut next = Some(element.native_class.clone());
        while let Some(native_class) = next {
            next = native_class.parent.clone();
            let class = classes
                .entry(native_class.class_name.to_string())
                .or_insert_with(|| Class { native_class, lowers_to_item: false });
            class.lowers_to_item |= !element.is_non_item_type;
        }
    }

    Compiler { classes, globals }
}

/// The properties and callbacks of a class, with those of its parents.
/// The functions are left out: a compiler pass or a builtin function implements them,
/// not a member of the item.
#[derive(Default)]
struct Members {
    properties: BTreeSet<String>,
    callbacks: BTreeSet<String>,
}

fn members(class: &Arc<NativeClass>) -> Members {
    let mut members = Members::default();
    let mut next = Some(class.clone());
    while let Some(class) = next {
        for (name, info) in &class.properties {
            match info.ty {
                Type::Callback(_) => members.callbacks.insert(name.to_string()),
                Type::Function(_) => false,
                _ => members.properties.insert(name.to_string()),
            };
        }
        next = class.parent.clone();
    }
    members
}

/// Classes whose elements the compiler lowers to other items or to components before the runtime
/// sees them, so that no item has their name.
const COMPILER_ONLY: &[&str] = &[
    // Lowered to `Empty`, with the layout kept in the component.
    "FlexboxLayout",
    "GridLayout",
    "HorizontalLayout",
    "VerticalLayout",
    // Lowered to `Window`.
    "PopupWindow",
    // Lowered to components of the style, and `Tab` to `Empty`.
    "RadioButton",
    "RadioGroup",
    "Tab",
    "TabWidget",
    // Lowered to `ContextMenu`.
    "ContextMenuArea",
    "ContextMenuInternal",
];

/// Which of the compiler and the item has a member that the other hasn't.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Side {
    Compiler,
    Item,
}

/// Members that only one side has, because a compiler pass or the runtime itself is the only
/// one to write them.
const KNOWN_DIFFERENCES: &[(&str, &str, Side)] = &[
    // The path pass turns `commands` and the child elements into `elements`.
    ("Path", "commands", Side::Compiler),
    ("Path", "elements", Side::Item),
    // The platform backend sets it.
    ("SystemTrayIcon", "color-scheme", Side::Item),
    // The cursor blinker sets it.
    ("TextInput", "cursor-visible", Side::Item),
];

struct Difference {
    class: String,
    member: String,
    side: Side,
    message: String,
}

fn compare_item(class: &str, compiler: &Members, item: &RuntimeItem) -> Vec<Difference> {
    let mut differences = Vec::new();
    let mut add = |member: &str, side: Side, kind: &str, other_kind: Option<&str>| {
        let (owner, other) = match side {
            Side::Compiler => ("the compiler", "the item"),
            Side::Item => ("the item", "the compiler"),
        };
        let hint = other_kind.map(|k| format!(", {other} has it as a {k}")).unwrap_or_default();
        differences.push(Difference {
            class: class.to_string(),
            member: member.to_string(),
            side,
            message: format!("{class}: {owner} has {kind} `{member}`, {other} hasn't{hint}"),
        });
    };
    let kind_in_item = |name: &String| {
        if item.callbacks.contains(name) {
            Some("callback")
        } else if item.properties.contains(name) || item.fields.contains(name) {
            Some("property")
        } else {
            None
        }
    };
    let kind_in_compiler = |name: &String| {
        if compiler.callbacks.contains(name) {
            Some("callback")
        } else if compiler.properties.contains(name) {
            Some("property")
        } else {
            None
        }
    };

    for name in &compiler.properties {
        if !item.properties.contains(name) && !item.fields.contains(name) {
            add(name, Side::Compiler, "property", kind_in_item(name));
        }
    }
    for name in &compiler.callbacks {
        if !item.callbacks.contains(name) {
            add(name, Side::Compiler, "callback", kind_in_item(name));
        }
    }
    for name in &item.properties {
        if !compiler.properties.contains(name) {
            add(name, Side::Item, "property", kind_in_compiler(name));
        }
    }
    for name in &item.callbacks {
        if !compiler.callbacks.contains(name) {
            add(name, Side::Item, "callback", kind_in_compiler(name));
        }
    }
    differences
}

fn assert_no_problems(problems: Vec<String>) {
    assert!(problems.is_empty(), "{} mismatches:\n{}", problems.len(), problems.join("\n"));
}

#[test]
fn items_match_the_compiler() {
    let compiler = compiler();
    let runtime = runtime_items();

    let mut problems = Vec::new();
    let mut compared = Vec::new();
    let mut known = BTreeSet::new();
    for (name, class) in &compiler.classes {
        if name.starts_with("Native") {
            continue;
        }
        match runtime.get(name) {
            Some(item) => {
                compared.push(name.as_str());
                for difference in compare_item(name, &members(&class.native_class), item) {
                    let key =
                        (difference.class.as_str(), difference.member.as_str(), difference.side);
                    if KNOWN_DIFFERENCES.contains(&key) {
                        known.insert((name.clone(), difference.member, difference.side));
                    } else {
                        problems.push(difference.message);
                    }
                }
            }
            None if class.lowers_to_item && !COMPILER_ONLY.contains(&name.as_str()) => {
                problems.push(format!(
                    "{name}: the compiler can lower an element to it, but no item has the name"
                ));
            }
            None => {}
        }
    }
    for name in runtime.keys() {
        if !compiler.classes.contains_key(name) {
            problems.push(format!("{name}: the compiler has no class with the name of this item"));
        }
    }
    for (class, member, side) in KNOWN_DIFFERENCES {
        if !known.contains(&(class.to_string(), member.to_string(), *side)) {
            problems.push(format!(
                "{class}: `{member}` no longer differs, remove it from KNOWN_DIFFERENCES"
            ));
        }
    }

    println!("compared {} classes: {}", compared.len(), compared.join(", "));
    assert_no_problems(problems);
}

#[test]
fn compiler_only_classes_are_not_items() {
    let compiler = compiler();
    let runtime = runtime_items();

    let mut problems = Vec::new();
    for name in COMPILER_ONLY {
        if !compiler.classes.contains_key(*name) {
            problems
                .push(format!("{name}: the compiler has no such class, remove it from the list"));
        }
        if runtime.contains_key(*name) {
            problems.push(format!("{name}: an item has the name, remove it from the list"));
        }
    }
    assert_no_problems(problems);
}

/// Items that the platform backend provides to the interpreter as `BuiltinGlobal`, in the compiler's names.
/// This runtime has none, and it has no backend with native widgets.
const RUNTIME_GLOBALS: &[&str] = &[];

/// Globals that the compiler lowers to constants and builtin functions, so no `BuiltinGlobal` has their state.
const COMPILER_ONLY_GLOBALS: &[&str] = &["Platform", "TextInputInterface"];

#[test]
fn globals_match_the_compiler() {
    let compiler = compiler();

    let mut problems = Vec::new();
    for name in &compiler.globals {
        if !name.starts_with("Native")
            && !RUNTIME_GLOBALS.contains(&name.as_str())
            && !COMPILER_ONLY_GLOBALS.contains(&name.as_str())
        {
            problems.push(format!("{name}: the compiler has this global, but this runtime has no `BuiltinGlobal` for it"));
        }
    }
    for name in COMPILER_ONLY_GLOBALS {
        if !compiler.globals.contains(*name) {
            problems
                .push(format!("{name}: the compiler has no such global, remove it from the list"));
        }
    }
    println!(
        "compiler globals: {}",
        compiler.globals.iter().cloned().collect::<Vec<_>>().join(", ")
    );
    assert_no_problems(problems);
}

/// The items whose `update_visual` doesn't show them yet, with the phase that adds them.
/// The `TODO(phase N)` comments in the bodies of `update_visual` are the source of truth.
const NOT_SHOWN_YET: &[(&str, u32)] =
    &[("BackdropBlur", 5), ("BoxShadow", 5), ("ClippedImage", 5), ("ImageItem", 5), ("Path", 5)];

/// The `(item struct, phase)` of each `update_visual` in `source` that has a `TODO(phase N)` comment.
///
/// The source is rustfmt-formatted: a method ends at the first line that closes at its own indentation.
fn not_shown_in(file: &Path, source: &str) -> BTreeSet<(String, u32)> {
    let lines: Vec<&str> = source.lines().collect();
    let mut found = BTreeSet::new();
    for (start, line) in lines.iter().enumerate() {
        if !line.trim_start().starts_with("fn update_visual") {
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        let Some(signature_end) =
            lines[start..].iter().position(|l| l.ends_with('{') || l.ends_with(';'))
        else {
            continue;
        };
        if lines[start + signature_end].ends_with(';') {
            continue;
        }
        let closing = format!("{}}}", " ".repeat(indent));
        let body_end = lines[start..].iter().position(|l| *l == closing).unwrap_or_else(|| {
            panic!("{}: the `update_visual` at line {} doesn't end", file.display(), start + 1)
        });
        let phases = lines[start + signature_end..start + body_end].iter().filter_map(|l| {
            let digits = l.split_once("// TODO(phase ")?.1;
            digits.split(')').next()?.parse::<u32>().ok()
        });
        let mut phases = phases.peekable();
        if phases.peek().is_none() {
            continue;
        }

        let implementation = lines[..start]
            .iter()
            .rev()
            .find(|l| l.starts_with("impl") || l.starts_with("unsafe impl"))
            .unwrap_or_else(|| panic!("{}: no `impl` encloses line {}", file.display(), start + 1));
        let (_, item) = implementation.rsplit_once(" for ").unwrap_or_else(|| {
            panic!("{}: `{implementation}` is no trait implementation", file.display())
        });
        let path: String =
            item.chars().take_while(|c| c.is_alphanumeric() || matches!(c, '_' | ':')).collect();
        let item = path.rsplit("::").next().unwrap().to_string();
        for phase in phases {
            found.insert((item.clone(), phase));
        }
    }
    found
}

fn rust_files(dir: &Path, files: &mut Vec<PathBuf>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|entry| entry.unwrap().path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            rust_files(&path, files);
        } else if path.extension().is_some_and(|e| e == "rs") {
            files.push(path);
        }
    }
}

#[test]
fn not_shown_yet_is_current() {
    let core = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../crates/core");
    let mut files = vec![core.join("items.rs")];
    rust_files(&core.join("items"), &mut files);

    let mut in_code = BTreeSet::new();
    for file in &files {
        let source =
            std::fs::read_to_string(file).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
        in_code.extend(not_shown_in(file, &source));
    }

    let listed: BTreeSet<_> =
        NOT_SHOWN_YET.iter().map(|(item, phase)| (item.to_string(), *phase)).collect();
    let mut problems = Vec::new();
    for (item, phase) in in_code.difference(&listed) {
        problems
            .push(format!("{item}: `update_visual` has TODO(phase {phase}), NOT_SHOWN_YET hasn't"));
    }
    for (item, phase) in listed.difference(&in_code) {
        problems.push(format!(
            "{item}: NOT_SHOWN_YET has phase {phase}, `update_visual` has no such TODO"
        ));
    }
    assert_no_problems(problems);
}
