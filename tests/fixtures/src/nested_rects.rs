// SPDX-License-Identifier: MIT OR Apache-2.0

//! `NestedRects`: a window with two levels of nested rectangles, each with its own background and
//! geometry.
//!
//! ```text
//! Window 400x300                          #202020
//! ├─ Left        (10,10)  180x280         #a03030
//! │  ├─ LeftTop     (10,10)   160x120     #e0c020
//! │  └─ LeftBottom  (10,150)  160x120     #20c0c0
//! └─ Right       (210,10) 180x280         #3050a0
//!    └─ RightInner  (20,20)   140x240     #30a030
//! ```
//!
//! The window is 400x300 logical pixels when its window has that size; the rectangles don't
//! follow it, which is what [`crate::ResizeFollower`] is for.

use crate::support::{
    Geometry, Plumbing, get_item_ref, item_tree_common, rect, visit_static_children,
};
use const_field_offset::{AllowPin, FieldOffsets, PinnedDrop};
use i_slint_core::item_tree::{
    ItemTree, ItemTreeNode, ItemTreeVTable, ItemVisitorVTable, TraversalOrder, VisitChildrenResult,
    register_item_tree, unregister_item_tree,
};
use i_slint_core::items::{ItemVTable, Rectangle, WindowItem};
use i_slint_core::lengths::LogicalRect;
use i_slint_core::slice::Slice;
use i_slint_core::window::WindowAdapterRc;
use i_slint_core::{Brush, Color};
use std::pin::Pin;
use vtable::{VOffset, VRc, VRef, VRefMut};

/// The rectangles below the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rect {
    Left,
    LeftTop,
    LeftBottom,
    Right,
    RightInner,
}

impl Rect {
    pub const ALL: [Rect; 5] =
        [Rect::Left, Rect::LeftTop, Rect::LeftBottom, Rect::Right, Rect::RightInner];

    /// Where the rectangle starts out: its geometry and its color.
    pub fn initial(self) -> (LogicalRect, Color) {
        match self {
            Rect::Left => (rect(10., 10., 180., 280.), Color::from_rgb_u8(0xa0, 0x30, 0x30)),
            Rect::LeftTop => (rect(10., 10., 160., 120.), Color::from_rgb_u8(0xe0, 0xc0, 0x20)),
            Rect::LeftBottom => (rect(10., 150., 160., 120.), Color::from_rgb_u8(0x20, 0xc0, 0xc0)),
            Rect::Right => (rect(210., 10., 180., 280.), Color::from_rgb_u8(0x30, 0x50, 0xa0)),
            Rect::RightInner => (rect(20., 20., 140., 240.), Color::from_rgb_u8(0x30, 0xa0, 0x30)),
        }
    }
}

/// The background of the window item.
pub const WINDOW_BACKGROUND: Color = Color::from_rgb_u8(0x20, 0x20, 0x20);

// The shape of generated code: a `#[repr(C)]` struct with one field per item, in the order of
// the item array, and `FieldOffsets` to reach them.
#[derive(FieldOffsets, Default)]
#[repr(C)]
#[pin_drop]
pub struct NestedRects {
    window: WindowItem,
    left: Rectangle,
    right: Rectangle,
    left_top: Rectangle,
    left_bottom: Rectangle,
    right_inner: Rectangle,
    left_geometry: Geometry,
    right_geometry: Geometry,
    left_top_geometry: Geometry,
    left_bottom_geometry: Geometry,
    right_inner_geometry: Geometry,
    plumbing: Plumbing,
}

impl NestedRects {
    /// A new component whose items belong to `window_adapter`. It doesn't become the window's
    /// component: that's up to the caller (`WindowInner::set_component`).
    pub fn new(window_adapter: &WindowAdapterRc) -> VRc<ItemTreeVTable, Self> {
        let this = VRc::new(Self::default());
        let item_tree = VRc::into_dyn(this.clone());
        this.plumbing.init(&item_tree, window_adapter);
        register_item_tree(&item_tree, Some(window_adapter.clone()));
        this.as_pin_ref().init();
        this
    }

    fn init(self: Pin<&Self>) {
        self.window.background.set(Brush::SolidColor(WINDOW_BACKGROUND));
        self.window.title.set("NestedRects".into());
        for r in Rect::ALL {
            let (geometry, color) = r.initial();
            self.set_geometry(r, geometry);
            self.set_background(r, Brush::SolidColor(color));
        }
    }

    fn item(self: Pin<&Self>, r: Rect) -> Pin<&Rectangle> {
        let offsets = Self::FIELD_OFFSETS;
        match r {
            Rect::Left => offsets.left().apply_pin(self),
            Rect::LeftTop => offsets.left_top().apply_pin(self),
            Rect::LeftBottom => offsets.left_bottom().apply_pin(self),
            Rect::Right => offsets.right().apply_pin(self),
            Rect::RightInner => offsets.right_inner().apply_pin(self),
        }
    }

    fn geometry(&self, r: Rect) -> &Geometry {
        match r {
            Rect::Left => &self.left_geometry,
            Rect::LeftTop => &self.left_top_geometry,
            Rect::LeftBottom => &self.left_bottom_geometry,
            Rect::Right => &self.right_geometry,
            Rect::RightInner => &self.right_inner_geometry,
        }
    }

    pub fn set_window_background(self: Pin<&Self>, brush: Brush) {
        self.window.background.set(brush);
    }

    pub fn set_background(self: Pin<&Self>, r: Rect, brush: Brush) {
        self.item(r).background.set(brush);
    }

    /// The rectangle's background.
    pub fn background(self: Pin<&Self>, r: Rect) -> Brush {
        self.item(r).background()
    }

    /// Moves and resizes the rectangle, relative to its parent.
    pub fn set_geometry(self: Pin<&Self>, r: Rect, geometry: LogicalRect) {
        self.geometry(r).set(geometry);
    }

    fn item_tree() -> &'static [ItemTreeNode] {
        const fn item(
            children_count: u32,
            children_index: u32,
            parent_index: u32,
            item_array_index: u32,
        ) -> ItemTreeNode {
            ItemTreeNode::Item {
                is_accessible: false,
                children_count,
                children_index,
                parent_index,
                item_array_index,
            }
        }
        // The window, its two rectangles, then the grandchildren; each node's children are
        // consecutive, starting at `children_index`.
        const ITEM_TREE: [ItemTreeNode; 6] = [
            item(2, 1, 0, 0), // window
            item(2, 3, 0, 1), // left
            item(1, 5, 0, 2), // right
            item(0, 6, 1, 3), // left_top
            item(0, 6, 1, 4), // left_bottom
            item(0, 6, 2, 5), // right_inner
        ];
        &ITEM_TREE
    }

    fn item_array() -> &'static [VOffset<Self, ItemVTable, AllowPin>] {
        const ITEM_ARRAY: [VOffset<NestedRects, ItemVTable, AllowPin>; 6] = [
            VOffset::new(NestedRects::FIELD_OFFSETS.window()),
            VOffset::new(NestedRects::FIELD_OFFSETS.left()),
            VOffset::new(NestedRects::FIELD_OFFSETS.right()),
            VOffset::new(NestedRects::FIELD_OFFSETS.left_top()),
            VOffset::new(NestedRects::FIELD_OFFSETS.left_bottom()),
            VOffset::new(NestedRects::FIELD_OFFSETS.right_inner()),
        ];
        &ITEM_ARRAY
    }
}

impl PinnedDrop for NestedRects {
    fn drop(self: Pin<&mut Self>) {
        vtable::new_vref!(let vref : VRef<ItemTreeVTable> for ItemTree = self.as_ref().get_ref());
        if let Some(adapter) = self.plumbing.window_adapter() {
            unregister_item_tree(self.as_ref(), vref, Self::item_array(), adapter);
        }
    }
}

impl ItemTree for NestedRects {
    fn visit_children_item(
        self: Pin<&Self>,
        index: isize,
        order: TraversalOrder,
        visitor: VRefMut<ItemVisitorVTable>,
    ) -> VisitChildrenResult {
        visit_static_children!(self, plumbing, index, order, visitor)
    }

    fn get_item_ref(self: Pin<&Self>, index: u32) -> Pin<VRef<'_, ItemVTable>> {
        get_item_ref(self, Self::item_tree(), Self::item_array(), index)
    }

    fn get_item_tree(self: Pin<&Self>) -> Slice<'_, ItemTreeNode> {
        Slice::from_slice(Self::item_tree())
    }

    fn item_geometry(self: Pin<&Self>, index: u32) -> LogicalRect {
        let offsets = Self::FIELD_OFFSETS;
        match index {
            // The window item's own width and height are the window's size.
            0 => {
                let window = offsets.window().apply_pin(self);
                rect(0., 0., window.width().get(), window.height().get())
            }
            1 => offsets.left_geometry().apply_pin(self).rect(),
            2 => offsets.right_geometry().apply_pin(self).rect(),
            3 => offsets.left_top_geometry().apply_pin(self).rect(),
            4 => offsets.left_bottom_geometry().apply_pin(self).rect(),
            5 => offsets.right_inner_geometry().apply_pin(self).rect(),
            _ => Default::default(),
        }
    }

    item_tree_common!(plumbing);
}

i_slint_core::ItemTreeVTable_static!(static NESTED_RECTS_VT for NestedRects);
