// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

use core::ops::ControlFlow;
use i_slint_core::SharedString;
use i_slint_core::api::ComponentHandle;
use i_slint_core::item_tree::{ItemTreeRc, ItemWeak};
use i_slint_core::items::ItemRc;
use i_slint_core::window::WindowInner;

/// Shown when the application was built without element debug info.
const MISSING_DEBUG_INFO_MESSAGE: &str = "The use of the ElementHandle API requires the presence of debug info in Slint compiler generated code. Set the `SLINT_EMIT_DEBUG_INFO=1` environment variable at application build time or use `compile_with_config` and `with_debug_info` with `slint_build`'s `CompilerConfiguration`";

fn warn_missing_debug_info() {
    i_slint_core::debug_log!("{}", MISSING_DEBUG_INFO_MESSAGE)
}

mod internal {
    /// Used as base of another trait so it cannot be re-implemented
    pub trait Sealed {}
}

use internal::Sealed;

/// Trait for type that can be searched for element. This is implemented for everything that implements [`ComponentHandle`]
pub trait ElementRoot: Sealed {
    #[doc(hidden)]
    fn item_tree(&self) -> ItemTreeRc;
    /// Returns the root of the element tree.
    fn root_element(&self) -> ElementHandle {
        let item_rc = ItemRc::new_root(self.item_tree());
        ElementHandle { item: item_rc.downgrade(), element_index: 0 }
    }
}

impl<T: ComponentHandle> ElementRoot for T {
    fn item_tree(&self) -> ItemTreeRc {
        WindowInner::from_pub(self.window()).component()
    }
}

impl<T: ComponentHandle> Sealed for T {}

#[allow(clippy::enum_variant_names)]
enum SingleElementMatch {
    MatchById { id: String, root_base: Option<String> },
    MatchByTypeName(String),
    MatchByTypeNameOrBase(String),
}

impl SingleElementMatch {
    fn matches(&self, element: &ElementHandle) -> bool {
        match self {
            SingleElementMatch::MatchById { id, root_base } => {
                if element.id().is_some_and(|candidate_id| candidate_id == id) {
                    return true;
                }
                root_base.as_ref().is_some_and(|root_base| {
                    element
                        .type_name()
                        .is_some_and(|type_name_candidate| type_name_candidate == root_base)
                        || element
                            .bases()
                            .is_some_and(|mut bases| bases.any(|base| base == root_base))
                })
            }
            SingleElementMatch::MatchByTypeName(type_name) => element
                .type_name()
                .is_some_and(|candidate_type_name| candidate_type_name == type_name),
            SingleElementMatch::MatchByTypeNameOrBase(type_name) => {
                element
                    .type_name()
                    .is_some_and(|candidate_type_name| candidate_type_name == type_name)
                    || element.bases().is_some_and(|mut bases| bases.any(|base| base == type_name))
            }
        }
    }
}

enum ElementQueryInstruction {
    MatchDescendants,
    MatchSingleElement(SingleElementMatch),
}

impl ElementQueryInstruction {
    fn match_recursively(
        query_stack: &[Self],
        element: ElementHandle,
        control_flow_after_first_match: ControlFlow<()>,
    ) -> (ControlFlow<()>, Vec<ElementHandle>) {
        let Some((query, tail)) = query_stack.split_first() else {
            return (control_flow_after_first_match, vec![element]);
        };

        match query {
            ElementQueryInstruction::MatchDescendants => {
                let mut results = Vec::new();
                match element.visit_descendants_impl(&mut |child| {
                    let (next_control_flow, sub_results) =
                        Self::match_recursively(tail, child, control_flow_after_first_match);
                    results.extend(sub_results);
                    next_control_flow
                }) {
                    Some(_) => (ControlFlow::Break(()), results),
                    None => (ControlFlow::Continue(()), results),
                }
            }
            ElementQueryInstruction::MatchSingleElement(criteria) => {
                let mut results = Vec::new();
                let control_flow = if criteria.matches(&element) {
                    let (next_control_flow, sub_results) =
                        Self::match_recursively(tail, element, control_flow_after_first_match);
                    results.extend(sub_results);
                    next_control_flow
                } else {
                    ControlFlow::Continue(())
                };
                (control_flow, results)
            }
        }
    }
}

/// Use ElementQuery to form a query into the tree of UI elements and then locate one or multiple
/// matching elements.
///
/// ElementQuery uses the builder pattern to concatenate criteria, such as searching for descendants,
/// or matching elements only with a certain id.
///
/// Construct an instance of this by calling [`ElementQuery::from_root`] or [`ElementHandle::query_descendants`]. Apply additional criterial on the returned `ElementQuery`
/// and fetch results by either calling [`Self::find_first()`] to collect just the first match or
/// [`Self::find_all()`] to collect all matches for the query.
pub struct ElementQuery {
    root: ElementHandle,
    query_stack: Vec<ElementQueryInstruction>,
}

impl ElementQuery {
    /// Creates a new element query starting at the root of the tree and matching all descendants.
    pub fn from_root(component: &impl ElementRoot) -> Self {
        component.root_element().query_descendants()
    }

    /// Applies any subsequent matches to all descendants of the results of the query up to this point.
    pub fn match_descendants(mut self) -> Self {
        self.query_stack.push(ElementQueryInstruction::MatchDescendants);
        self
    }

    /// Include only elements in the results where [`ElementHandle::id()`] is equal to the provided `id`.
    pub fn match_id(mut self, id: impl Into<String>) -> Self {
        let id = id.into().replace('_', "-");
        let mut id_split = id.split("::");
        let type_name = id_split.next().map(ToString::to_string);
        let local_id = id_split.next();
        let root_base = if local_id == Some("root") { type_name } else { None };

        self.query_stack.push(ElementQueryInstruction::MatchSingleElement(
            SingleElementMatch::MatchById { id, root_base },
        ));
        self
    }

    /// Include only elements in the results where [`ElementHandle::type_name()`] is equal to the provided `type_name`.
    pub fn match_type_name(mut self, type_name: impl Into<String>) -> Self {
        self.query_stack.push(ElementQueryInstruction::MatchSingleElement(
            SingleElementMatch::MatchByTypeName(type_name.into()),
        ));
        self
    }

    /// Include only elements in the results where [`ElementHandle::type_name()`] or [`ElementHandle::bases()`] is contains to the provided `type_name`.
    pub fn match_inherits(mut self, type_name: impl Into<String>) -> Self {
        self.query_stack.push(ElementQueryInstruction::MatchSingleElement(
            SingleElementMatch::MatchByTypeNameOrBase(type_name.into()),
        ));
        self
    }

    /// Runs the query and returns the first result; returns None if no element matches the selected
    /// criteria.
    pub fn find_first(&self) -> Option<ElementHandle> {
        ElementQueryInstruction::match_recursively(
            &self.query_stack,
            self.root.clone(),
            ControlFlow::Break(()),
        )
        .1
        .into_iter()
        .next()
    }

    /// Runs the query and returns a vector of all matching elements.
    pub fn find_all(&self) -> Vec<ElementHandle> {
        ElementQueryInstruction::match_recursively(
            &self.query_stack,
            self.root.clone(),
            ControlFlow::Continue(()),
        )
        .1
    }
}

/// `ElementHandle` wraps an existing element in a Slint UI. An ElementHandle does not keep
/// the corresponding element in the UI alive. Use [`Self::is_valid()`] to verify that
/// it is still alive.
///
/// Obtain instances of `ElementHandle` by querying your application through
/// [`Self::find_by_element_id()`].
#[derive(Clone)]
#[repr(C)]
pub struct ElementHandle {
    item: ItemWeak,
    element_index: usize, // When multiple elements get optimized into a single ItemRc, this index separates.
}

/// Overrides scrolling behavior for Slint's internal tests.
/// Panics if the handle has expired or does not refer to a Flickable.
#[cfg(feature = "internal")]
pub fn set_flickable_physics(
    element: &ElementHandle,
    bounce: i_slint_core::items::AutoBool,
    carry_momentum: i_slint_core::items::AutoBool,
) {
    use i_slint_core::items::Flickable;

    let item = element.item.upgrade().expect("Flickable element has expired");
    let flickable = item.downcast::<Flickable>().expect("Element is not a Flickable");
    flickable.as_pin_ref().set_physics(bounce, carry_momentum);
}

impl ElementHandle {
    fn collect_elements(item: ItemRc) -> impl Iterator<Item = ElementHandle> {
        (0..item.element_count().unwrap_or_else(|| {
            warn_missing_debug_info();
            0
        }))
            .map(move |element_index| ElementHandle { item: item.downgrade(), element_index })
    }

    /// Visit all descendants of this element and call the visitor to each of them, until the visitor returns [`ControlFlow::Break`].
    /// When the visitor breaks, the function returns the value. If it doesn't break, the function returns None.
    ///
    /// Popups have no support in this runtime yet, so the visit doesn't enter them.
    pub fn visit_descendants<R>(
        &self,
        mut visitor: impl FnMut(ElementHandle) -> ControlFlow<R>,
    ) -> Option<R> {
        self.visit_descendants_impl(&mut |e| visitor(e))
    }

    fn visit_descendants_impl<R>(
        &self,
        visitor: &mut dyn FnMut(ElementHandle) -> ControlFlow<R>,
    ) -> Option<R> {
        let self_item = self.item.upgrade()?;

        self_item.visit_descendants(move |item_rc| {
            if !item_rc.is_visible() {
                return ControlFlow::Continue(());
            }

            let elements = ElementHandle::collect_elements(item_rc.clone());
            for e in elements {
                let result = visitor(e);
                if matches!(result, ControlFlow::Break(..)) {
                    return result;
                }
            }
            ControlFlow::Continue(())
        })
    }

    /// Creates a new [`ElementQuery`] to match any descendants of this element.
    pub fn query_descendants(&self) -> ElementQuery {
        ElementQuery {
            root: self.clone(),
            query_stack: vec![ElementQueryInstruction::MatchDescendants],
        }
    }

    /// This function searches through the entire tree of elements of this window and looks for
    /// elements by their id. The id is a qualified string consisting of the name of the component
    /// and the assigned name within the component. In the following examples, the first Button
    /// has the id "MyView::submit-button" and the second button "App::close":
    ///
    /// ```slint,no-preview
    /// component MyView {
    ///    submit-button := Button {}
    /// }
    /// export component App {
    ///     VerticalLayout {
    ///         close := Button {}
    ///     }
    /// }
    /// ```
    pub fn find_by_element_id(
        component: &impl ElementRoot,
        id: &str,
    ) -> impl Iterator<Item = Self> {
        let results = component.root_element().query_descendants().match_id(id).find_all();
        results.into_iter()
    }

    /// This function searches through the entire tree of elements of `component`, looks for
    /// elements with given type name.
    pub fn find_by_element_type_name(
        component: &impl ElementRoot,
        type_name: &str,
    ) -> impl Iterator<Item = Self> {
        let results =
            component.root_element().query_descendants().match_inherits(type_name).find_all();
        results.into_iter()
    }

    /// Returns true if the element still exists in the in UI and is valid to access; false otherwise.
    pub fn is_valid(&self) -> bool {
        self.item.upgrade().is_some()
    }

    /// Returns the element's qualified id. Returns None if the element is not valid anymore or the
    /// element does not have an id.
    /// A qualified id consists of the name of the surrounding component as well as the provided local
    /// name, separate by a double colon.
    pub fn id(&self) -> Option<SharedString> {
        self.item.upgrade().and_then(|item| {
            item.element_type_names_and_ids(self.element_index)
                .unwrap_or_else(|| {
                    warn_missing_debug_info();
                    Default::default()
                })
                .into_iter()
                .next()
                .map(|(_, id)| id)
        })
    }

    /// Returns the element's type name; None if the element is not valid anymore.
    pub fn type_name(&self) -> Option<SharedString> {
        self.item.upgrade().and_then(|item| {
            item.element_type_names_and_ids(self.element_index)
                .unwrap_or_else(|| {
                    warn_missing_debug_info();
                    Default::default()
                })
                .into_iter()
                .next()
                .map(|(type_name, _)| type_name)
        })
    }

    /// Returns the element's base types as an iterator; None if the element is not valid anymore.
    pub fn bases(&self) -> Option<impl Iterator<Item = SharedString>> {
        self.item.upgrade().map(|item| {
            item.element_type_names_and_ids(self.element_index)
                .unwrap_or_else(|| {
                    warn_missing_debug_info();
                    Default::default()
                })
                .into_iter()
                .skip(1)
                .filter_map(
                    |(type_name, _)| {
                        if !type_name.is_empty() { Some(type_name) } else { None }
                    },
                )
        })
    }

    /// Returns the size of the element in logical pixels. This corresponds to the value of the `width` and
    /// `height` properties in Slint code. Returns a zero size if the element is not valid.
    pub fn size(&self) -> i_slint_core::api::LogicalSize {
        self.item
            .upgrade()
            .map(|item| {
                let g = item.geometry();
                i_slint_core::lengths::logical_size_to_api(g.size)
            })
            .unwrap_or_default()
    }

    /// Returns the position of the element within the entire window. This corresponds to the value of the
    /// `absolute-position` property in Slint code. Returns a zero position if the element is not valid.
    pub fn absolute_position(&self) -> i_slint_core::api::LogicalPosition {
        self.item
            .upgrade()
            .map(i_slint_core::builtin_functions::item_absolute_position)
            .unwrap_or_default()
    }
}
