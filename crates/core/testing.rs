// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

//! Helpers shared by the unit tests.

use crate::api::PlatformError;

/// A platform that can't create windows, for unit tests that only need a context.
pub(crate) struct NoWindowPlatform;

impl crate::platform::Platform for NoWindowPlatform {
    fn create_window_adapter(
        &self,
    ) -> Result<alloc::rc::Rc<dyn crate::window::WindowAdapter>, PlatformError> {
        Err(PlatformError::Other("this test needs no window".into()))
    }
}

pub(crate) use fixture_trees::{
    BorderTree, ClipTree, LayerTree, OpacityTree, RectTree, TransformTree,
};

/// Item trees in the shape generated code has: a root item with a row of child items.
mod fixture_trees {
    use crate::Property;
    use crate::SharedString;
    use crate::SlintContext;
    use crate::accessibility::{
        AccessibilityAction, AccessibleStringProperty, SupportedAccessibilityAction,
    };
    use crate::item_tree::{
        IndexRange, ItemTree, ItemTreeNode, ItemTreeVTable, ItemTreeWeak, ItemVisitorVTable,
        ItemWeak, TraversalOrder, VisitChildrenResult,
    };
    use crate::items::{
        AccessibleRole, BorderRectangle, Clip, ItemVTable, Layer, Opacity, Rectangle, Transform,
    };
    use crate::layout::{LayoutInfo, Orientation};
    use crate::lengths::LogicalRect;
    use crate::slice::Slice;
    use crate::window::WindowAdapterRc;
    use alloc::boxed::Box;
    use alloc::vec::Vec;
    use core::pin::Pin;
    use vtable::{VRc, VRef, VRefMut, VWeak};

    macro_rules! fixture_tree {
        ($tree:ident, $item:ident, $vt:ident) => {
            #[doc = concat!("An item tree shaped like generated code: a root `", stringify!($item), "` with child `", stringify!($item), "`s.")]
            pub(crate) struct $tree {
                self_weak: core::cell::OnceCell<VWeak<ItemTreeVTable, $tree>>,
                nodes: Vec<ItemTreeNode>,
                rects: Vec<Pin<Box<$item>>>,
                geometry: Vec<Pin<Box<Property<LogicalRect>>>>,
            }

            impl $tree {
                /// A root of `size` with `children` rectangles of 10x10, 20 apart from left to right.
                pub(crate) fn new(size: (f32, f32), children: u32) -> VRc<ItemTreeVTable, $tree> {
                    let mut nodes = alloc::vec![ItemTreeNode::Item {
                        is_accessible: false,
                        children_count: children,
                        children_index: 1,
                        parent_index: 0,
                        item_array_index: 0,
                    }];
                    let mut geometry =
                        alloc::vec![LogicalRect::new(Default::default(), euclid::size2(size.0, size.1))];
                    for i in 0..children {
                        nodes.push(ItemTreeNode::Item {
                            is_accessible: false,
                            children_count: 0,
                            children_index: children + 1,
                            parent_index: 0,
                            item_array_index: i + 1,
                        });
                        geometry.push(LogicalRect::new(
                            euclid::point2(20. * i as f32, 0.),
                            euclid::size2(10., 10.),
                        ));
                    }
                    let tree = VRc::new($tree {
                        self_weak: Default::default(),
                        rects: nodes.iter().map(|_| Box::pin($item::default())).collect(),
                        geometry: geometry.into_iter().map(|g| Box::pin(Property::new(g))).collect(),
                        nodes,
                    });
                    let _ = tree.self_weak.set(VRc::downgrade(&tree));
                    tree
                }

                pub(crate) fn rect(&self, index: usize) -> Pin<&$item> {
                    self.rects[index].as_ref()
                }

                #[allow(dead_code)]
                pub(crate) fn geometry(&self, index: usize) -> Pin<&Property<LogicalRect>> {
                    self.geometry[index].as_ref()
                }
            }

            impl ItemTree for $tree {
                fn visit_children_item(
                    self: Pin<&Self>,
                    index: isize,
                    order: TraversalOrder,
                    visitor: VRefMut<ItemVisitorVTable>,
                ) -> VisitChildrenResult {
                    let rc = VRc::into_dyn(self.self_weak.get().unwrap().upgrade().unwrap());
                    crate::item_tree::visit_item_tree(
                        &rc,
                        &self.get_ref().nodes,
                        index,
                        order,
                        visitor,
                        &mut |_, _, _| unreachable!("no dynamic nodes"),
                    )
                }

                fn get_item_ref(self: Pin<&Self>, index: u32) -> Pin<VRef<'_, ItemVTable>> {
                    VRef::new_pin(self.get_ref().rects[index as usize].as_ref())
                }

                fn get_item_tree(self: Pin<&Self>) -> Slice<'_, ItemTreeNode> {
                    Slice::from_slice(&self.get_ref().nodes)
                }

                fn parent_node(self: Pin<&Self>, _result: &mut ItemWeak) {}

                fn embed_component(self: Pin<&Self>, _: &ItemTreeWeak, _: u32) -> bool {
                    false
                }

                fn ensure_instantiated(self: Pin<&Self>) -> bool {
                    false
                }

                fn layout_info(self: Pin<&Self>, _: Orientation) -> LayoutInfo {
                    LayoutInfo::default()
                }

                fn subtree_index(self: Pin<&Self>) -> usize {
                    usize::MAX
                }

                fn get_subtree_range(self: Pin<&Self>, _: u32) -> IndexRange {
                    (0..0).into()
                }

                fn get_subtree(self: Pin<&Self>, _: u32, _: usize, _: &mut ItemTreeWeak) {}

                fn accessible_role(self: Pin<&Self>, _: u32) -> AccessibleRole {
                    AccessibleRole::default()
                }

                fn accessible_string_property(
                    self: Pin<&Self>,
                    _: u32,
                    _: AccessibleStringProperty,
                    _: &mut SharedString,
                ) -> bool {
                    false
                }

                fn item_element_infos(self: Pin<&Self>, _: u32, _: &mut SharedString) -> bool {
                    false
                }

                fn element_testable_properties(self: Pin<&Self>, _: u32, _: &mut SharedString) -> bool {
                    false
                }

                fn element_testable_property_value(
                    self: Pin<&Self>,
                    _: u32,
                    _: Slice<u8>,
                    _: &mut SharedString,
                ) -> bool {
                    false
                }

                fn window_adapter(self: Pin<&Self>, _: bool, _: &mut Option<WindowAdapterRc>) {}

                fn slint_context(self: Pin<&Self>, _: &mut Option<SlintContext>) {}

                fn item_geometry(self: Pin<&Self>, index: u32) -> LogicalRect {
                    self.get_ref().geometry[index as usize].as_ref().get()
                }

                fn accessibility_action(self: Pin<&Self>, _: u32, _: &AccessibilityAction) {}

                fn supported_accessibility_actions(
                    self: Pin<&Self>,
                    _: u32,
                ) -> SupportedAccessibilityAction {
                    Default::default()
                }
            }

            crate::item_tree::ItemTreeVTable_static!(static $vt for $tree);
        };
    }

    fixture_tree!(RectTree, Rectangle, RECT_TREE_VT);
    fixture_tree!(BorderTree, BorderRectangle, BORDER_TREE_VT);
    fixture_tree!(ClipTree, Clip, CLIP_TREE_VT);
    fixture_tree!(OpacityTree, Opacity, OPACITY_TREE_VT);
    fixture_tree!(LayerTree, Layer, LAYER_TREE_VT);
    fixture_tree!(TransformTree, Transform, TRANSFORM_TREE_VT);
}
