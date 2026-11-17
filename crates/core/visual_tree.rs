//! One `Windows.UI.Composition` visual per item, kept in sync with the item tree.
//!
//! See `docs/design/visual-tree.md` for the design.

mod shapes;

use crate::item_rendering::{BorderRectLayout, RenderBorderRectangle};
use crate::item_tree::{ItemTreeRc, ItemVisitorResult, ItemWeak, TraversalOrder};
use crate::items::{ItemRc, ItemRef, RenderingResult};
use crate::lengths::{LogicalPoint, LogicalRect, LogicalSize, ScaleFactor};
use crate::properties::PropertyTracker;
use alloc::boxed::Box;
use alloc::vec::Vec;
use core::cell::Cell;
use core::pin::Pin;
use shapes::ui_color;
use std::collections::HashSet;
use windows::UI::Composition::{
    CompositionColorBrush, Compositor, ContainerVisual, ShapeVisual, SpriteVisual,
};
use windows::core::Interface;
use windows_numerics::{Vector2, Vector3};

/// Per-item handle into the window's [`VisualTree`]. Every item struct embeds one.
///
/// Its layout is part of the C++ ABI: `api/cpp/cbindgen.rs` declares it by hand.
#[derive(Default, Debug)]
#[repr(C)]
pub struct CachedRenderingData {
    pub(crate) cache_index: Cell<usize>,
    pub(crate) cache_generation: Cell<usize>,
}

/// The kind of visual a node currently owns. All of them are `ContainerVisual`s, so children
/// can hang below any node.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VisualKind {
    Container,
    Sprite,
    Shape,
}

struct Node {
    visual: ContainerVisual,
    kind: VisualKind,
    item: ItemWeak,
    /// Geometry relative to the parent item, as last applied to the visual.
    geometry: Option<LogicalRect>,
    /// Tracks the properties `update_visual` read; dirty means the content needs an update.
    content: Pin<Box<PropertyTracker>>,
    /// Child nodes in back-to-front order, as last applied to the visual's children.
    children: Vec<usize>,
    /// Children collected during the current pass.
    next_children: Vec<usize>,
    /// The pass that last visited this node.
    pass: u64,
    /// The parent node as of that pass; `None` for the root of an item tree.
    parent: Option<usize>,
}

/// What an item's `update_visual` works with: its node's visual and the compositor.
pub struct VisualUpdate<'a> {
    compositor: &'a Compositor,
    visual: &'a mut ContainerVisual,
    kind: &'a mut VisualKind,
}

impl VisualUpdate<'_> {
    pub fn compositor(&self) -> &Compositor {
        self.compositor
    }

    pub fn kind(&self) -> VisualKind {
        *self.kind
    }

    /// The node's visual as a plain container.
    pub fn container(&self) -> &ContainerVisual {
        self.visual
    }

    /// Makes the node a `SpriteVisual` and returns it.
    pub fn sprite(&mut self) -> SpriteVisual {
        self.ensure_kind(VisualKind::Sprite);
        self.visual.cast().unwrap()
    }

    /// Makes the node a `ShapeVisual` and returns it.
    pub fn shape(&mut self) -> ShapeVisual {
        self.ensure_kind(VisualKind::Shape);
        self.visual.cast().unwrap()
    }

    /// Makes the node a plain `ContainerVisual`, dropping any brush or shapes.
    pub fn plain(&mut self) {
        self.ensure_kind(VisualKind::Container);
    }

    /// Fills the node's area, of `size`, with `brush`, or shows nothing of its own if `brush` is
    /// transparent.
    pub fn fill(&mut self, brush: &crate::Brush, size: LogicalSize) {
        if brush.is_transparent() {
            self.plain();
            return;
        }
        let sprite = self.sprite();
        // A color change reuses the color brush.
        if let crate::Brush::SolidColor(color) = brush
            && let Ok(current) = sprite.Brush().and_then(|b| b.cast::<CompositionColorBrush>())
        {
            let _ = current.SetColor(ui_color(*color));
            return;
        }
        if let Some(b) = shapes::make_brush(self.compositor, brush, size.cast_unit()) {
            let _ = sprite.SetBrush(&b);
        }
    }

    /// Shows a rectangle with a border and rounded corners, of `size`, laid out like upstream's
    /// renderers lay it out (`BorderRectLayout`): one shape fills the background, another
    /// strokes the border. Without border or radius it's a plain [`Self::fill`].
    pub fn border_rect(&mut self, rect: Pin<&dyn RenderBorderRectangle>, size: LogicalSize) {
        let Some(layout) = BorderRectLayout::new(rect, size, ScaleFactor::new(1.)) else {
            self.plain();
            return;
        };
        if layout.border_width.get() <= 0. && layout.outer_radius.is_zero() {
            self.fill(&rect.background(), size);
            return;
        }
        let c = self.compositor.clone();
        let shapes = self.shape().Shapes().unwrap();
        let _ = shapes.Clear();
        if let Some(brush) = shapes::make_brush(&c, &rect.background(), layout.brush_size)
            && let Ok(geometry) =
                shapes::rounded_rect(&c, layout.background_rect, layout.background_radius)
        {
            let shape = c.CreateSpriteShapeWithGeometry(&geometry).unwrap();
            let _ = shape.SetFillBrush(&brush);
            let _ = shapes.Append(&shape);
        }
        if layout.border_width.get() > 0.
            && let Some(brush) = shapes::make_brush(&c, &layout.border_color, layout.brush_size)
            && let Ok(geometry) = shapes::rounded_rect(&c, layout.border_rect, layout.border_radius)
        {
            let shape = c.CreateSpriteShapeWithGeometry(&geometry).unwrap();
            let _ = shape.SetStrokeBrush(&brush);
            let _ = shape.SetStrokeThickness(layout.border_width.get());
            let _ = shapes.Append(&shape);
        }
    }

    /// Replaces the node's visual with one of `kind`, keeping its children, geometry, and
    /// place among its siblings.
    fn ensure_kind(&mut self, kind: VisualKind) {
        if *self.kind == kind {
            return;
        }
        let c = self.compositor;
        let new: ContainerVisual = match kind {
            VisualKind::Container => c.CreateContainerVisual().unwrap(),
            VisualKind::Sprite => c.CreateSpriteVisual().unwrap().cast().unwrap(),
            VisualKind::Shape => c.CreateShapeVisual().unwrap().cast().unwrap(),
        };
        let old = &*self.visual;
        let _ = new.SetOffset(old.Offset().unwrap_or_default());
        let _ = new.SetSize(old.Size().unwrap_or_default());
        let _ = new.SetOpacity(old.Opacity().unwrap_or(1.0));
        if let Ok(clip) = old.Clip() {
            let _ = new.SetClip(&clip);
        }
        let (old_children, new_children) = (old.Children().unwrap(), new.Children().unwrap());
        // Moving the whole list keeps it in order; InsertAtBottom is O(1) (S2).
        let moved: Vec<_> = (&old_children).into_iter().collect();
        old_children.RemoveAll().unwrap();
        for child in moved.iter().rev() {
            new_children.InsertAtBottom(child).unwrap();
        }
        // A visual not attached yet gets placed by the pass's children patching.
        if let Ok(parent) = old.Parent() {
            let siblings = parent.Children().unwrap();
            siblings.InsertAbove(&new, old).unwrap();
            siblings.Remove(old).unwrap();
        }
        *self.visual = new;
        *self.kind = kind;
    }
}

static NEXT_GENERATION: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(1);

/// The visuals of one window.
pub struct VisualTree {
    compositor: Compositor,
    root: ContainerVisual,
    nodes: slab::Slab<Node>,
    /// Distinguishes this tree's handles in `CachedRenderingData` from other trees'.
    generation: usize,
    pass: u64,
    /// The top-level nodes (main item tree, popups, overlays) of the last pass.
    roots: Vec<usize>,
}

impl VisualTree {
    /// `root` is the visual the window shows; it carries the scale factor.
    pub fn new(compositor: Compositor, root: ContainerVisual) -> Self {
        Self {
            compositor,
            root,
            nodes: slab::Slab::new(),
            generation: NEXT_GENERATION.fetch_add(1, core::sync::atomic::Ordering::Relaxed),
            pass: 0,
            roots: Vec::new(),
        }
    }

    pub fn compositor(&self) -> &Compositor {
        &self.compositor
    }

    pub fn root(&self) -> &ContainerVisual {
        &self.root
    }

    pub fn set_scale_factor(&self, factor: f32) {
        let _ = self.root.SetScale(Vector3 { X: factor, Y: factor, Z: 1.0 });
    }

    fn node_for(&mut self, item_rc: &ItemRc, item: Pin<ItemRef>) -> usize {
        let data = item.cached_rendering_data_offset();
        if data.cache_generation.get() == self.generation {
            let index = data.cache_index.get();
            if let Some(node) = self.nodes.get(index)
                && node.item.upgrade().as_ref() == Some(item_rc)
            {
                return index;
            }
        }
        let visual = self.compositor.CreateContainerVisual().unwrap();
        let index = self.nodes.insert(Node {
            visual,
            kind: VisualKind::Container,
            item: item_rc.downgrade(),
            geometry: None,
            content: Box::pin(PropertyTracker::default()),
            children: Vec::new(),
            next_children: Vec::new(),
            pass: 0,
            parent: None,
        });
        data.cache_index.set(index);
        data.cache_generation.set(self.generation);
        index
    }

    /// Brings the visuals up to date with `trees` (the window's main item tree first, then popups
    /// and overlays at their positions). Call it inside the window's redraw tracker.
    pub fn sync(&mut self, trees: &[(crate::item_tree::ItemTreeWeak, LogicalPoint)]) {
        self.pass += 1;
        let pass = self.pass;
        let mut roots = Vec::new();
        for (weak, origin) in trees {
            let Some(tree) = weak.upgrade() else { continue };
            self.sync_tree(&tree, *origin, pass, &mut roots);
        }
        let root_visual = self.root.clone();
        Self::patch_children(&self.nodes, &root_visual, &self.roots, &roots);
        self.roots = roots;
        let visited: Vec<usize> =
            self.nodes.iter().filter(|(_, n)| n.pass == pass).map(|(i, _)| i).collect();
        for index in visited {
            let next = core::mem::take(&mut self.nodes[index].next_children);
            let visual = self.nodes[index].visual.clone();
            Self::patch_children(&self.nodes, &visual, &self.nodes[index].children, &next);
            self.nodes[index].children = next;
        }
        self.release_unvisited(pass);
    }

    fn sync_tree(
        &mut self,
        tree: &ItemTreeRc,
        origin: LogicalPoint,
        pass: u64,
        roots: &mut Vec<usize>,
    ) {
        // The visitor's state is the parent node, or None for the tree's root item.
        crate::item_tree::visit_items(
            tree,
            TraversalOrder::BackToFront,
            |tree, item, index, parent: &Option<usize>| {
                let item_rc = ItemRc::new(tree.clone(), index);
                let id = self.node_for(&item_rc, item);
                let mut geometry = item_rc.geometry();
                if parent.is_none() {
                    geometry.origin += origin.to_vector();
                }
                let node = &mut self.nodes[id];
                node.pass = pass;
                node.parent = *parent;
                if node.geometry != Some(geometry) {
                    let _ = node.visual.SetOffset(Vector3 {
                        X: geometry.origin.x as f32,
                        Y: geometry.origin.y as f32,
                        Z: 0.0,
                    });
                    let _ = node.visual.SetSize(Vector2 {
                        X: geometry.size.width as f32,
                        Y: geometry.size.height as f32,
                    });
                    node.geometry = Some(geometry);
                }
                let mut result = RenderingResult::ContinueRenderingChildren;
                let Node { visual, kind, content, .. } = node;
                if content.is_dirty() {
                    let mut update = VisualUpdate { compositor: &self.compositor, visual, kind };
                    result = content.as_ref().evaluate(|| {
                        item.as_ref().update_visual(&item_rc, &mut update, geometry.size)
                    });
                } else {
                    content.as_ref().register_as_dependency_to_current_binding();
                }
                match parent {
                    Some(p) => self.nodes[*p].next_children.push(id),
                    None => roots.push(id),
                }
                match result {
                    RenderingResult::ContinueRenderingChildren => {
                        ItemVisitorResult::Continue(Some(id))
                    }
                    RenderingResult::ContinueRenderingWithoutChildren => {
                        ItemVisitorResult::SkipChildren
                    }
                }
            },
            None,
        );
    }

    /// Makes `parent`'s children match `new`, given that they currently match `old`.
    fn patch_children(
        nodes: &slab::Slab<Node>,
        parent: &ContainerVisual,
        old: &[usize],
        new: &[usize],
    ) {
        if old == new {
            return;
        }
        let children = parent.Children().unwrap();
        let (old_set, new_set): (HashSet<usize>, HashSet<usize>) =
            (old.iter().copied().collect(), new.iter().copied().collect());
        for id in old {
            if !new_set.contains(id)
                && let Some(node) = nodes.get(*id)
            {
                let _ = children.Remove(&node.visual);
            }
        }
        // Walk the new order and (re)insert every visual that isn't already right after its
        // predecessor in the old order. InsertAtTop is linear per call, so it isn't used (S2).
        let mut previous: Option<&ContainerVisual> = None;
        let mut old_pos = old.iter().filter(|id| new_set.contains(id)).copied().peekable();
        for id in new {
            let visual = &nodes[*id].visual;
            if old_pos.peek() == Some(id) {
                old_pos.next();
            } else {
                if old_set.contains(id) {
                    let _ = children.Remove(visual);
                }
                match previous {
                    Some(prev) => children.InsertAbove(visual, prev).unwrap(),
                    None => children.InsertAtBottom(visual).unwrap(),
                }
            }
            previous = Some(visual);
        }
    }

    /// Drops the nodes of items that no longer exist. Nodes of live items that the pass didn't
    /// reach stay, already detached by `patch_children`, so a later pass can reattach them.
    fn release_unvisited(&mut self, pass: u64) {
        let stale: Vec<usize> = self
            .nodes
            .iter()
            .filter(|(_, n)| n.pass != pass && n.item.upgrade().is_none())
            .map(|(i, _)| i)
            .collect();
        for index in stale {
            self.nodes.remove(index);
        }
    }

    /// Releases the nodes of `items`, which are being destroyed.
    pub fn release_items<'a>(&mut self, items: &mut dyn Iterator<Item = Pin<ItemRef<'a>>>) {
        for item in items {
            let data = item.cached_rendering_data_offset();
            if data.cache_generation.get() == self.generation {
                let index = data.cache_index.get();
                if self.nodes.contains(index) {
                    let node = self.nodes.remove(index);
                    if let Ok(parent) = node.visual.Parent() {
                        let _ = parent.Children().and_then(|c| c.Remove(&node.visual));
                    }
                    // Forget the id, which the slab may hand to another node before the next pass.
                    let siblings = match node.parent.and_then(|p| self.nodes.get_mut(p)) {
                        Some(parent) => &mut parent.children,
                        None => &mut self.roots,
                    };
                    siblings.retain(|id| *id != index);
                }
                data.cache_generation.set(0);
            }
        }
    }

    /// The number of sync passes so far, for tests.
    pub fn passes(&self) -> u64 {
        self.pass
    }

    /// The number of live nodes, for tests.
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// A text dump of the visuals below the root, one per line, indented by depth:
    /// `kind x,y wxh` plus `#rrggbbaa` for a solid fill and `opacity=` when not 1.
    pub fn dump(&self) -> alloc::string::String {
        let mut out = alloc::string::String::new();
        for child in &self.root.Children().unwrap() {
            dump_visual(&child.cast().unwrap(), 0, &mut out);
        }
        out
    }
}

fn dump_visual(visual: &ContainerVisual, depth: usize, out: &mut alloc::string::String) {
    use core::fmt::Write;
    let offset = visual.Offset().unwrap_or_default();
    let size = visual.Size().unwrap_or_default();
    let kind = if visual.cast::<SpriteVisual>().is_ok() {
        "Sprite"
    } else if visual.cast::<ShapeVisual>().is_ok() {
        "Shape"
    } else {
        "Container"
    };
    let _ = write!(
        out,
        "{:indent$}{kind} {},{} {}x{}",
        "",
        offset.X,
        offset.Y,
        size.X,
        size.Y,
        indent = depth * 2
    );
    if let Ok(sprite) = visual.cast::<SpriteVisual>()
        && let Ok(brush) = sprite.Brush()
    {
        let _ = write!(out, " {}", shapes::describe_brush(&brush));
    }
    if let Ok(shape) = visual.cast::<ShapeVisual>() {
        let _ = write!(out, "{}", shapes::describe_shapes(&shape));
    }
    let opacity = visual.Opacity().unwrap_or(1.0);
    if opacity != 1.0 {
        let _ = write!(out, " opacity={opacity}");
    }
    out.push('\n');
    for child in &visual.Children().unwrap() {
        dump_visual(&child.cast().unwrap(), depth + 1, out);
    }
}

#[cfg(test)]
#[allow(unsafe_code)]
mod tests {
    use super::*;
    use crate::Color;
    use crate::graphics::Brush;
    use crate::item_tree::ItemTreeRc;
    use crate::testing::RectTree;

    /// A compositor for this test thread, which gets the dispatcher queue it needs.
    fn compositor() -> Compositor {
        use windows::Win32::System::WinRT::*;
        std::thread_local! {
            static QUEUE: core::cell::OnceCell<windows::System::DispatcherQueueController> =
                const { core::cell::OnceCell::new() };
        }
        QUEUE.with(|q| {
            q.get_or_init(|| unsafe {
                let _ = RoInitialize(RO_INIT_SINGLETHREADED);
                CreateDispatcherQueueController(DispatcherQueueOptions {
                    dwSize: size_of::<DispatcherQueueOptions>() as u32,
                    threadType: DQTYPE_THREAD_CURRENT,
                    apartmentType: DQTAT_COM_STA,
                })
                .unwrap()
            });
        });
        Compositor::new().unwrap()
    }

    fn new_tree() -> VisualTree {
        let c = compositor();
        let root = c.CreateContainerVisual().unwrap();
        VisualTree::new(c, root)
    }

    fn sync(visuals: &mut VisualTree, tree: &ItemTreeRc) {
        visuals.sync(&[(ItemTreeRc::downgrade(tree), LogicalPoint::default())]);
    }

    fn red() -> Brush {
        Brush::SolidColor(Color::from_rgb_u8(255, 0, 0))
    }

    #[test]
    fn mirrors_the_item_tree() {
        let tree = RectTree::new((100., 50.), 2);
        tree.rect(0).background.set(red());
        tree.rect(2).background.set(Brush::SolidColor(Color::from_argb_u8(128, 0, 0, 255)));
        let dyn_tree = vtable::VRc::into_dyn(tree.clone());
        let mut visuals = new_tree();
        sync(&mut visuals, &dyn_tree);
        assert_eq!(
            visuals.dump(),
            "Sprite 0,0 100x50 #ff0000ff\n  Container 0,0 10x10\n  Sprite 20,0 10x10 #0000ff80\n"
        );
        assert_eq!(visuals.node_count(), 3);
    }

    #[test]
    fn a_sync_pass_applies_only_what_changed() {
        let tree = RectTree::new((100., 50.), 2);
        let dyn_tree = vtable::VRc::into_dyn(tree.clone());
        let mut visuals = new_tree();
        let window_tracker: Pin<Box<PropertyTracker>> = Box::pin(PropertyTracker::default());
        window_tracker.as_ref().evaluate_as_dependency_root(|| sync(&mut visuals, &dyn_tree));
        assert!(!window_tracker.is_dirty());

        // A fill change dirties the window and keeps the child's place among its siblings.
        tree.rect(1).background.set(red());
        assert!(window_tracker.is_dirty());
        window_tracker.as_ref().evaluate_as_dependency_root(|| sync(&mut visuals, &dyn_tree));
        assert_eq!(
            visuals.dump(),
            "Container 0,0 100x50\n  Sprite 0,0 10x10 #ff0000ff\n  Container 20,0 10x10\n"
        );

        // So does a geometry change.
        tree.geometry(2).set(LogicalRect::new(euclid::point2(5., 6.), euclid::size2(7., 8.)));
        assert!(window_tracker.is_dirty());
        window_tracker.as_ref().evaluate_as_dependency_root(|| sync(&mut visuals, &dyn_tree));
        assert_eq!(
            visuals.dump(),
            "Container 0,0 100x50\n  Sprite 0,0 10x10 #ff0000ff\n  Container 5,6 7x8\n"
        );

        // Back to transparent: the sprite becomes a container again, in the same place.
        tree.rect(1).background.set(Brush::default());
        window_tracker.as_ref().evaluate_as_dependency_root(|| sync(&mut visuals, &dyn_tree));
        assert_eq!(
            visuals.dump(),
            "Container 0,0 100x50\n  Container 0,0 10x10\n  Container 5,6 7x8\n"
        );
        assert!(!window_tracker.is_dirty());
    }

    #[test]
    fn dropped_trees_release_their_visuals() {
        let mut visuals = new_tree();
        let dyn_tree = vtable::VRc::into_dyn(RectTree::new((100., 50.), 3));
        sync(&mut visuals, &dyn_tree);
        assert_eq!(visuals.node_count(), 4);
        let weak = ItemTreeRc::downgrade(&dyn_tree);
        drop(dyn_tree);
        visuals.sync(&[(weak, LogicalPoint::default())]);
        assert_eq!(visuals.node_count(), 0);
        assert_eq!(visuals.dump(), "");
    }

    fn print(visuals: &VisualTree) -> alloc::string::String {
        let dump = visuals.dump();
        std::println!("{dump}");
        dump
    }

    #[test]
    fn border_rectangles_are_shapes_laid_out_like_upstream() {
        use crate::lengths::LogicalLength;
        let tree = crate::testing::BorderTree::new((100., 50.), 1);
        let root = tree.rect(0);
        root.background.set(red());
        root.border_color.set(Brush::SolidColor(Color::from_rgb_u8(0, 0, 0)));
        root.border_width.set(LogicalLength::new(2.));
        for radius in [
            &root.border_top_left_radius,
            &root.border_top_right_radius,
            &root.border_bottom_right_radius,
            &root.border_bottom_left_radius,
        ] {
            radius.set(LogicalLength::new(4.));
        }
        let dyn_tree = vtable::VRc::into_dyn(tree.clone());
        let mut visuals = new_tree();
        sync(&mut visuals, &dyn_tree);
        // Opaque border: the fill stops under the border, both inset by half the border width.
        assert_eq!(
            print(&visuals),
            "Shape 0,0 100x50 [fill #ff0000ff rrect 1,1 98x48 r3] [stroke #000000ff w2 rrect 1,1 98x48 r3]
  Container 0,0 10x10
"
        );

        // Different radii per corner need a path; the child stays in place.
        root.border_top_left_radius.set(LogicalLength::new(10.));
        sync(&mut visuals, &dyn_tree);
        assert_eq!(
            print(&visuals),
            "Shape 0,0 100x50 [fill #ff0000ff path] [stroke #000000ff w2 path]
  Container 0,0 10x10
"
        );

        // Neither border nor radius: a plain sprite again.
        root.border_width.set(LogicalLength::new(0.));
        for radius in [
            &root.border_top_left_radius,
            &root.border_top_right_radius,
            &root.border_bottom_right_radius,
            &root.border_bottom_left_radius,
        ] {
            radius.set(LogicalLength::new(0.));
        }
        sync(&mut visuals, &dyn_tree);
        assert_eq!(
            print(&visuals),
            "Sprite 0,0 100x50 #ff0000ff
  Container 0,0 10x10
"
        );
    }

    #[test]
    fn gradients_map_to_native_brushes() {
        use crate::graphics::{GradientStop, LinearGradientBrush, RadialGradientBrush};
        let stops = || {
            [
                GradientStop { color: Color::from_rgb_u8(255, 0, 0), position: 0. },
                GradientStop { color: Color::from_rgb_u8(0, 0, 255), position: 1. },
            ]
        };
        let tree = RectTree::new((100., 50.), 1);
        // 90deg runs left to right.
        tree.rect(0).background.set(Brush::LinearGradient(LinearGradientBrush::new(90., stops())));
        tree.rect(1)
            .background
            .set(Brush::RadialGradient(RadialGradientBrush::new_circle(stops())));
        let dyn_tree = vtable::VRc::into_dyn(tree.clone());
        let mut visuals = new_tree();
        sync(&mut visuals, &dyn_tree);
        assert_eq!(
            print(&visuals),
            "Sprite 0,0 100x50 linear(0,25->100,25 #ff0000ff@0 #0000ffff@1)
  Sprite 0,0 10x10 radial(5,5 7.071068,7.071068 #ff0000ff@0 #0000ffff@1)
"
        );
    }

    #[test]
    fn released_items_free_their_ids_for_reuse() {
        let mut visuals = new_tree();
        let a = vtable::VRc::into_dyn(RectTree::new((10., 10.), 0));
        let b = vtable::VRc::into_dyn(RectTree::new((20., 20.), 0));
        let trees = |trees: &[&ItemTreeRc]| -> Vec<_> {
            trees.iter().map(|t| (ItemTreeRc::downgrade(t), LogicalPoint::default())).collect()
        };
        visuals.sync(&trees(&[&a, &b]));
        // What `unregister_item_tree` does for `b`; a new tree then takes the freed slab id.
        visuals.release_items(&mut core::iter::once(
            vtable::VRc::borrow_pin(&b).as_ref().get_item_ref(0),
        ));
        let c = vtable::VRc::into_dyn(RectTree::new((30., 30.), 0));
        visuals.sync(&trees(&[&a, &c]));
        assert_eq!(visuals.dump(), "Container 0,0 10x10\nContainer 0,0 30x30\n");
    }
}
