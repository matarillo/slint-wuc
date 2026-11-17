//! What every fixture shares: the geometry properties of an element, the plumbing that
//! connects a component to its window, and the `ItemTree` methods that none of the fixtures
//! need to implement for real (they have no repeaters, popups, or accessibility).

use const_field_offset::{AllowPin, FieldOffsets};
use i_slint_core::Property;
use i_slint_core::graphics::euclid;
use i_slint_core::item_tree::{ItemTreeNode, ItemTreeRc, ItemTreeWeak};
use i_slint_core::items::{ItemRef, ItemVTable};
use i_slint_core::lengths::{LogicalLength, LogicalRect};
use i_slint_core::window::WindowAdapterRc;
use std::cell::OnceCell;
use std::pin::Pin;
use vtable::VOffset;

/// The `x`, `y`, `width` and `height` of an element, as properties like generated code keeps
/// them. The element's `item_geometry` reads them.
#[derive(FieldOffsets, Default)]
#[repr(C)]
#[pin]
pub struct Geometry {
    pub x: Property<LogicalLength>,
    pub y: Property<LogicalLength>,
    pub width: Property<LogicalLength>,
    pub height: Property<LogicalLength>,
}

impl Geometry {
    /// The rectangle the properties describe. Reads them, so a tracker sees the dependency.
    pub fn rect(self: Pin<&Self>) -> LogicalRect {
        // Each accessor consumes the offsets table, which is a zero-sized constant.
        euclid::rect(
            Self::FIELD_OFFSETS.x().apply_pin(self).get().get(),
            Self::FIELD_OFFSETS.y().apply_pin(self).get().get(),
            Self::FIELD_OFFSETS.width().apply_pin(self).get().get(),
            Self::FIELD_OFFSETS.height().apply_pin(self).get().get(),
        )
    }

    /// Sets the properties, replacing any binding on them.
    pub fn set(&self, rect: LogicalRect) {
        self.x.set(LogicalLength::new(rect.origin.x));
        self.y.set(LogicalLength::new(rect.origin.y));
        self.width.set(LogicalLength::new(rect.size.width));
        self.height.set(LogicalLength::new(rect.size.height));
    }
}

/// `rect(x, y, width, height)` in logical pixels.
pub fn rect(x: f32, y: f32, width: f32, height: f32) -> LogicalRect {
    euclid::rect(x, y, width, height)
}

/// What generated code keeps in `self_weak` and the shared globals: the way back from a
/// component to itself as an `ItemTreeRc`, and to its window.
#[derive(Default)]
pub struct Plumbing {
    self_weak: OnceCell<ItemTreeWeak>,
    window_adapter: OnceCell<WindowAdapterRc>,
}

impl Plumbing {
    /// Called once by the fixture's constructor.
    pub fn init(&self, item_tree: &ItemTreeRc, window_adapter: &WindowAdapterRc) {
        let _ = self.self_weak.set(ItemTreeRc::downgrade(item_tree));
        let _ = self.window_adapter.set(window_adapter.clone());
    }

    pub fn item_tree(&self) -> ItemTreeRc {
        self.self_weak.get().and_then(|w| w.upgrade()).expect("the component is alive")
    }

    pub fn window_adapter(&self) -> Option<&WindowAdapterRc> {
        self.window_adapter.get()
    }
}

/// `slint::private_unstable_api::get_item_ref`: the item behind a tree index, through the
/// component's table of field offsets.
pub fn get_item_ref<'a, T>(
    component: Pin<&'a T>,
    nodes: &[ItemTreeNode],
    item_array: &[VOffset<T, ItemVTable, AllowPin>],
    index: u32,
) -> Pin<ItemRef<'a>> {
    let ItemTreeNode::Item { item_array_index, .. } = &nodes[index as usize] else {
        panic!("get_item_ref called on a dynamic node")
    };
    item_array[*item_array_index as usize].apply_pin(component)
}

/// The `ItemTree` methods that behave the same in every fixture. `$plumbing` names the
/// component's [`Plumbing`] field.
macro_rules! item_tree_common {
    ($plumbing:ident) => {
        fn parent_node(
            self: ::std::pin::Pin<&Self>,
            _result: &mut i_slint_core::item_tree::ItemWeak,
        ) {
        }

        fn embed_component(
            self: ::std::pin::Pin<&Self>,
            _parent: &i_slint_core::item_tree::ItemTreeWeak,
            _index: u32,
        ) -> bool {
            false
        }

        fn ensure_instantiated(self: ::std::pin::Pin<&Self>) -> bool {
            false
        }

        fn layout_info(
            self: ::std::pin::Pin<&Self>,
            _orientation: i_slint_core::layout::Orientation,
        ) -> i_slint_core::layout::LayoutInfo {
            Default::default()
        }

        fn subtree_index(self: ::std::pin::Pin<&Self>) -> usize {
            usize::MAX
        }

        fn get_subtree_range(
            self: ::std::pin::Pin<&Self>,
            _index: u32,
        ) -> i_slint_core::item_tree::IndexRange {
            (0..0).into()
        }

        fn get_subtree(
            self: ::std::pin::Pin<&Self>,
            _index: u32,
            _subindex: usize,
            _result: &mut i_slint_core::item_tree::ItemTreeWeak,
        ) {
        }

        fn accessible_role(
            self: ::std::pin::Pin<&Self>,
            _index: u32,
        ) -> i_slint_core::items::AccessibleRole {
            Default::default()
        }

        fn accessible_string_property(
            self: ::std::pin::Pin<&Self>,
            _index: u32,
            _what: i_slint_core::accessibility::AccessibleStringProperty,
            _result: &mut i_slint_core::SharedString,
        ) -> bool {
            false
        }

        fn item_element_infos(
            self: ::std::pin::Pin<&Self>,
            _index: u32,
            _result: &mut i_slint_core::SharedString,
        ) -> bool {
            false
        }

        fn element_testable_properties(
            self: ::std::pin::Pin<&Self>,
            _index: u32,
            _result: &mut i_slint_core::SharedString,
        ) -> bool {
            false
        }

        fn element_testable_property_value(
            self: ::std::pin::Pin<&Self>,
            _index: u32,
            _property_name: i_slint_core::slice::Slice<u8>,
            _result: &mut i_slint_core::SharedString,
        ) -> bool {
            false
        }

        fn window_adapter(
            self: ::std::pin::Pin<&Self>,
            _do_create: bool,
            result: &mut Option<i_slint_core::window::WindowAdapterRc>,
        ) {
            *result = self.$plumbing.window_adapter().cloned();
        }

        fn slint_context(
            self: ::std::pin::Pin<&Self>,
            result: &mut Option<i_slint_core::SlintContext>,
        ) {
            *result = self.$plumbing.window_adapter().map(|adapter| {
                i_slint_core::window::WindowInner::from_pub(adapter.window()).context().clone()
            });
        }

        fn accessibility_action(
            self: ::std::pin::Pin<&Self>,
            _index: u32,
            _action: &i_slint_core::accessibility::AccessibilityAction,
        ) {
        }

        fn supported_accessibility_actions(
            self: ::std::pin::Pin<&Self>,
            _index: u32,
        ) -> i_slint_core::accessibility::SupportedAccessibilityAction {
            Default::default()
        }
    };
}

/// `visit_children_item` for a component whose children are all static.
macro_rules! visit_static_children {
    ($self:ident, $plumbing:ident, $index:ident, $order:ident, $visitor:ident) => {
        i_slint_core::item_tree::visit_item_tree(
            &$self.$plumbing.item_tree(),
            $self.get_item_tree().as_slice(),
            $index,
            $order,
            $visitor,
            &mut |_, _, _| unreachable!("no dynamic nodes"),
        )
    };
}

pub(crate) use {item_tree_common, visit_static_children};
