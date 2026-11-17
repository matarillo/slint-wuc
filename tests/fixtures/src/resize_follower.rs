//! `ResizeFollower`: a window whose child rectangle's geometry is a binding on the window item's
//! `width` and `height`, so a window resize moves and resizes it.
//!
//! ```text
//! Window (w x h)                                      #202020
//! ├─ Follower  (w/4, h/4)  w/2 x h/2   (bindings)     #c04080
//! └─ Corner    (0,0)       20x20       (fixed)        #e0e0e0
//! ```

use crate::support::{
    Geometry, Plumbing, get_item_ref, item_tree_common, rect, visit_static_children,
};
use const_field_offset::{AllowPin, FieldOffsets, PinnedDrop};
use i_slint_core::item_tree::{
    ItemTree, ItemTreeNode, ItemTreeVTable, ItemVisitorVTable, TraversalOrder, VisitChildrenResult,
    register_item_tree, unregister_item_tree,
};
use i_slint_core::items::{ItemVTable, Rectangle, WindowItem};
use i_slint_core::lengths::{LogicalLength, LogicalRect};
use i_slint_core::slice::Slice;
use i_slint_core::window::WindowAdapterRc;
use i_slint_core::{Brush, Color};
use std::pin::Pin;
use vtable::{VOffset, VRc, VRef, VRefMut};

pub const WINDOW_BACKGROUND: Color = Color::from_rgb_u8(0x20, 0x20, 0x20);
pub const FOLLOWER_COLOR: Color = Color::from_rgb_u8(0xc0, 0x40, 0x80);
pub const CORNER_COLOR: Color = Color::from_rgb_u8(0xe0, 0xe0, 0xe0);
pub const CORNER_SIZE: f32 = 20.;

#[derive(FieldOffsets, Default)]
#[repr(C)]
#[pin_drop]
pub struct ResizeFollower {
    window: WindowItem,
    follower: Rectangle,
    corner: Rectangle,
    follower_geometry: Geometry,
    corner_geometry: Geometry,
    plumbing: Plumbing,
}

impl ResizeFollower {
    /// The follower of a window of `width` x `height`: a rectangle centered in it, half as wide
    /// and half as high.
    pub fn follower_rect_for(width: f32, height: f32) -> LogicalRect {
        rect(width / 4., height / 4., width / 2., height / 2.)
    }

    /// A new component whose items belong to `window_adapter`. It doesn't become the window's
    /// component: that's up to the caller (`WindowInner::set_component`).
    pub fn new(window_adapter: &WindowAdapterRc) -> VRc<ItemTreeVTable, Self> {
        let this = VRc::new(Self::default());
        let item_tree = VRc::into_dyn(this.clone());
        this.plumbing.init(&item_tree, window_adapter);
        register_item_tree(&item_tree, Some(window_adapter.clone()));
        Self::init(&this);
        this
    }

    fn init(this: &VRc<ItemTreeVTable, Self>) {
        let this_pin = this.as_pin_ref();
        this_pin.window.background.set(Brush::SolidColor(WINDOW_BACKGROUND));
        this_pin.window.title.set("ResizeFollower".into());
        this_pin.follower.background.set(Brush::SolidColor(FOLLOWER_COLOR));
        this_pin.corner.background.set(Brush::SolidColor(CORNER_COLOR));
        this_pin.corner_geometry.set(rect(0., 0., CORNER_SIZE, CORNER_SIZE));

        // Bindings capture a weak handle to the component, as generated code's do.
        let weak = VRc::downgrade(this);
        let binding = |f: fn(f32, f32) -> f32| {
            let weak = weak.clone();
            move || {
                let this = weak.upgrade().expect("the component is alive");
                let (width, height) = this.as_pin_ref().window_size();
                LogicalLength::new(f(width, height))
            }
        };
        let geometry = &this_pin.follower_geometry;
        geometry.x.set_binding(binding(|w, _| w / 4.));
        geometry.y.set_binding(binding(|_, h| h / 4.));
        geometry.width.set_binding(binding(|w, _| w / 2.));
        geometry.height.set_binding(binding(|_, h| h / 2.));
    }

    /// The size of the window item. Reads its properties, so bindings that call it depend on
    /// them.
    fn window_size(self: Pin<&Self>) -> (f32, f32) {
        let window = Self::FIELD_OFFSETS.window().apply_pin(self);
        (window.width().get(), window.height().get())
    }

    pub fn set_window_background(self: Pin<&Self>, brush: Brush) {
        self.window.background.set(brush);
    }

    pub fn set_follower_background(self: Pin<&Self>, brush: Brush) {
        self.follower.background.set(brush);
    }

    pub fn set_corner_geometry(self: Pin<&Self>, geometry: LogicalRect) {
        self.corner_geometry.set(geometry);
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
        const ITEM_TREE: [ItemTreeNode; 3] = [
            item(2, 1, 0, 0), // window
            item(0, 3, 0, 1), // follower
            item(0, 3, 0, 2), // corner
        ];
        &ITEM_TREE
    }

    fn item_array() -> &'static [VOffset<Self, ItemVTable, AllowPin>] {
        const ITEM_ARRAY: [VOffset<ResizeFollower, ItemVTable, AllowPin>; 3] = [
            VOffset::new(ResizeFollower::FIELD_OFFSETS.window()),
            VOffset::new(ResizeFollower::FIELD_OFFSETS.follower()),
            VOffset::new(ResizeFollower::FIELD_OFFSETS.corner()),
        ];
        &ITEM_ARRAY
    }
}

impl PinnedDrop for ResizeFollower {
    fn drop(self: Pin<&mut Self>) {
        vtable::new_vref!(let vref : VRef<ItemTreeVTable> for ItemTree = self.as_ref().get_ref());
        if let Some(adapter) = self.plumbing.window_adapter() {
            unregister_item_tree(self.as_ref(), vref, Self::item_array(), adapter);
        }
    }
}

impl ItemTree for ResizeFollower {
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
            0 => {
                let (width, height) = self.window_size();
                rect(0., 0., width, height)
            }
            1 => offsets.follower_geometry().apply_pin(self).rect(),
            2 => offsets.corner_geometry().apply_pin(self).rect(),
            _ => Default::default(),
        }
    }

    item_tree_common!(plumbing);
}

i_slint_core::ItemTreeVTable_static!(static RESIZE_FOLLOWER_VT for ResizeFollower);
