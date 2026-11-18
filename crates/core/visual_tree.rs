// SPDX-License-Identifier: MIT OR Apache-2.0

//! One `Windows.UI.Composition` visual per item, kept in sync with the item tree.
//!
//! See `docs/design/visual-tree.md` for the design.

mod conic;
mod effects;
mod graphics;
pub mod image;
pub mod path;
pub mod shadow;
mod shapes;
pub mod text;

use crate::item_rendering::{BorderRectLayout, RenderBorderRectangle};
use crate::item_tree::{ItemTreeRc, ItemVisitorResult, ItemWeak, TraversalOrder};
use crate::items::{ItemRc, ItemRef, RenderingResult};
use crate::lengths::{
    LogicalBorderRadius, LogicalPoint, LogicalPx, LogicalRect, LogicalSize, PhysicalPx,
    ScaleFactor,
};
use crate::properties::PropertyTracker;
use alloc::boxed::Box;
use alloc::vec::Vec;
use core::cell::Cell;
use core::pin::Pin;
use shapes::ui_color;
use std::collections::HashSet;
use windows::UI::Composition::{
    CompositionBorderMode, CompositionBrush, CompositionClip, CompositionColorBrush,
    CompositionEffectBrush, CompositionShapeCollection, CompositionSpriteShape, Compositor,
    ContainerVisual, InsetClip, ShapeVisual, SpriteVisual,
};
use windows::core::Interface;
use windows_numerics::{Matrix4x4, Vector2, Vector3};

/// The `TransformMatrix` of a visual that isn't transformed.
const IDENTITY: Matrix4x4 = Matrix4x4::translation(0., 0., 0.);

/// How far, in logical pixels, a node's shape visual reaches past the item on each side.
const SHAPE_MARGIN: f32 = 1.;

/// The border mode of a visual whose edges or clip aren't axis-aligned rectangles. The compositor
/// antialiases edges only in `Soft` mode, and snaps them to pixels in the default `Hard` mode,
/// which matches how upstream's renderers draw axis-aligned rectangles.
fn soft_if(soft: bool) -> CompositionBorderMode {
    if soft { CompositionBorderMode::Soft } else { CompositionBorderMode::Inherit }
}

/// Per-item handle into the window's [`VisualTree`]. Every item struct embeds one.
///
/// Its layout is part of the C ABI that `tools/cpp-headers` generates the C++ headers of.
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
    /// What `update_visual` returned last.
    result: RenderingResult,
    /// Child nodes in back-to-front order, as last applied to the visual's children.
    children: Vec<usize>,
    /// Children collected during the current pass.
    next_children: Vec<usize>,
    /// The visual of the item's own content, below the children's visuals: a sprite (text,
    /// images, effects) or a shape visual.
    floor: Option<ContainerVisual>,
    /// The pass that last visited this node.
    pass: u64,
    /// The parent node as of that pass; `None` for the root of an item tree.
    parent: Option<usize>,
    /// When the node was created, in `VisualTree::created` order. Slab ids are reused, so they
    /// don't tell.
    serial: u64,
}

/// What an item's `update_visual` works with: its node's visual and the compositor.
pub struct VisualUpdate<'a> {
    compositor: &'a Compositor,
    /// The item, for its window; `None` in tests of a lone visual.
    item: Option<&'a ItemRc>,
    visual: &'a mut ContainerVisual,
    kind: &'a mut VisualKind,
    floor: &'a mut Option<ContainerVisual>,
}

impl VisualUpdate<'_> {
    pub fn compositor(&self) -> &Compositor {
        self.compositor
    }

    /// The window's scale factor, 1 without a window. Reading it makes a new scale factor
    /// update the item again.
    pub fn scale_factor(&self) -> f32 {
        self.item
            .and_then(|item| item.window_adapter())
            .map_or(1., |adapter| crate::window::WindowInner::from_pub(adapter.window()).scale_factor())
    }

    fn brush(&self, brush: &crate::Brush, size: euclid::Size2D<f32, PhysicalPx>) -> Option<CompositionBrush> {
        shapes::make_brush(self.compositor, brush, size, || self.scale_factor())
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

    /// Makes the node a plain container with a `ShapeVisual` below its children that covers the
    /// node's area, of `size`, and returns that and the offset of the node's origin in it.
    ///
    /// The compositor antialiases shapes only in soft border mode, which also antialiases the
    /// edges of the shape visual's bounds: where a shape's edge lies on them, both apply and the
    /// edge comes out lighter than upstream's. The shape visual reaches past the item by
    /// [`SHAPE_MARGIN`] so that its bounds lie where nothing is drawn. Being below the children,
    /// its border mode doesn't pass on to them.
    pub fn shape(&mut self, size: LogicalSize) -> (ShapeVisual, Vector2) {
        self.shape_covering(euclid::Rect::from_size(size.cast::<f32>()))
    }

    /// Like [`Self::shape`], for shapes that reach outside the node's area: the shape visual
    /// covers `area`, a rectangle in the node's coordinates, which contains the node's area.
    /// Composition cuts shapes off at the bounds of their visual.
    pub fn shape_covering(&mut self, area: euclid::Rect<f32, LogicalPx>) -> (ShapeVisual, Vector2) {
        self.plain();
        let shape = match self.floor.as_ref().and_then(|f| f.cast::<ShapeVisual>().ok()) {
            Some(shape) => shape,
            None => {
                self.remove_floor();
                let shape = self.compositor.CreateShapeVisual().unwrap();
                let _ = self.visual.Children().and_then(|c| c.InsertAtBottom(&shape));
                *self.floor = Some(shape.cast().unwrap());
                shape
            }
        };
        let area = area.inflate(SHAPE_MARGIN, SHAPE_MARGIN);
        let _ = shape.SetOffset(Vector3 { X: area.min_x(), Y: area.min_y(), Z: 0. });
        let _ = shape.SetSize(Vector2 { X: area.width(), Y: area.height() });
        (shape, Vector2 { X: -area.min_x(), Y: -area.min_y() })
    }

    /// Like [`Self::shape_covering`], for brushes that shapes can't show: makes the node a plain
    /// container with an empty container below its children that covers `area`, for sprites cut
    /// to their outlines ([`shapes::append_clipped`]). Returns that, its size, and the offset of
    /// the node's origin in it.
    pub fn clipped_covering(
        &mut self,
        area: euclid::Rect<f32, LogicalPx>,
    ) -> (ContainerVisual, Vector2, Vector2) {
        self.plain();
        let reusable = |f: &&ContainerVisual| {
            f.cast::<ShapeVisual>().is_err() && f.cast::<SpriteVisual>().is_err()
        };
        let floor = match self.floor.as_ref().filter(reusable) {
            Some(floor) => floor.clone(),
            None => {
                self.remove_floor();
                let floor = self.compositor.CreateContainerVisual().unwrap();
                let _ = self.visual.Children().and_then(|c| c.InsertAtBottom(&floor));
                *self.floor = Some(floor.clone());
                floor
            }
        };
        let _ = floor.Children().and_then(|c| c.RemoveAll());
        // The sprites' bounds lie where nothing is drawn, as a shape visual's do.
        let area = area.inflate(SHAPE_MARGIN, SHAPE_MARGIN);
        let size = Vector2 { X: area.width(), Y: area.height() };
        let _ = floor.SetOffset(Vector3 { X: area.min_x(), Y: area.min_y(), Z: 0. });
        let _ = floor.SetSize(size);
        (floor, size, Vector2 { X: -area.min_x(), Y: -area.min_y() })
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
        if let Some(b) = self.brush(brush, size.cast_unit()) {
            let _ = sprite.SetBrush(&b);
        }
    }

    /// Shows a rectangle with a border and rounded corners, of `size`, laid out like upstream's
    /// renderers lay it out (`BorderRectLayout`): one shape fills the background, another
    /// strokes the border. Without border or radius it's a plain [`Self::fill`].
    pub fn border_rect(&mut self, rect: Pin<&dyn RenderBorderRectangle>, size: LogicalSize) {
        let Some(layout) = BorderRectLayout::new(rect, size, ScaleFactor::new(1.)) else {
            self.remove_floor();
            self.plain();
            return;
        };
        if layout.border_width.get() <= 0. && layout.outer_radius.is_zero() {
            self.remove_floor();
            self.fill(&rect.background(), size);
            return;
        }
        let c = self.compositor.clone();
        // Upstream antialiases rounded rectangles, and others only when transformed.
        let border_mode = soft_if(!layout.outer_radius.is_zero());
        let background = rect.background();
        let has_border = layout.border_width.get() > 0.;
        if shapes::needs_sprite(&background)
            || (has_border && shapes::needs_sprite(&layout.border_color))
        {
            let (floor, area, origin) =
                self.clipped_covering(LogicalRect::from_size(size).cast());
            if let Some(brush) = self.brush(&background, layout.brush_size)
                && let Ok(outline) =
                    shapes::rounded_rect_path(layout.background_rect, layout.background_radius)
            {
                let _ =
                    shapes::append_clipped(&c, &floor, &brush, outline, area, origin, border_mode);
            }
            if has_border
                && let Some(brush) = self.brush(&layout.border_color, layout.brush_size)
                && let Ok(outline) = shapes::rounded_rect_path(layout.border_rect, layout.border_radius)
                    .and_then(|path| shapes::widen(&path, layout.border_width.get(), None))
            {
                let _ =
                    shapes::append_clipped(&c, &floor, &brush, outline, area, origin, border_mode);
            }
            return;
        }
        let (visual, origin) = self.shape(size);
        let _ = visual.SetBorderMode(border_mode);
        let shapes = visual.Shapes().unwrap();
        if self.update_border_shapes(&shapes, &layout, &background, origin) {
            return;
        }
        let _ = shapes.Clear();
        if let Some(brush) = self.brush(&rect.background(), layout.brush_size)
            && let Ok(geometry) =
                shapes::rounded_rect(&c, layout.background_rect, layout.background_radius)
        {
            let shape = c.CreateSpriteShapeWithGeometry(&geometry).unwrap();
            let _ = shape.SetOffset(origin);
            let _ = shape.SetFillBrush(&brush);
            let _ = shapes.Append(&shape);
        }
        if layout.border_width.get() > 0.
            && let Some(brush) = self.brush(&layout.border_color, layout.brush_size)
            && let Ok(geometry) = shapes::rounded_rect(&c, layout.border_rect, layout.border_radius)
        {
            let shape = c.CreateSpriteShapeWithGeometry(&geometry).unwrap();
            let _ = shape.SetOffset(origin);
            let _ = shape.SetStrokeBrush(&brush);
            let _ = shape.SetStrokeThickness(layout.border_width.get());
            let _ = shapes.Append(&shape);
        }
    }

    /// Updates the shapes of an earlier [`Self::border_rect`] in place, and returns whether they
    /// had the right kinds. Replacing Composition objects costs more the more objects the window
    /// has, so a whole list of rounded rectangles that change color would take seconds.
    fn update_border_shapes(
        &self,
        shapes: &CompositionShapeCollection,
        layout: &BorderRectLayout,
        background: &crate::Brush,
        origin: Vector2,
    ) -> bool {
        let fill = !background.is_transparent();
        let stroke = layout.border_width.get() > 0. && !layout.border_color.is_transparent();
        if shapes.Size().ok() != Some(u32::from(fill) + u32::from(stroke)) {
            return false;
        }
        let parts = [
            (fill, false, layout.background_rect, layout.background_radius, background),
            (stroke, true, layout.border_rect, layout.border_radius, &layout.border_color),
        ];
        let mut index = 0;
        for (wanted, is_stroke, rect, radius, brush) in parts {
            if !wanted {
                continue;
            }
            let Ok(shape) = shapes.GetAt(index).and_then(|s| s.cast::<CompositionSpriteShape>())
            else {
                return false;
            };
            index += 1;
            let current = if is_stroke { shape.StrokeBrush() } else { shape.FillBrush() };
            let Ok(current) = current else { return false };
            if !shapes::update_rounded_rect(&shape, rect, radius) {
                return false;
            }
            let _ = shape.SetOffset(origin);
            if let crate::Brush::SolidColor(color) = brush
                && let Ok(current) = current.cast::<CompositionColorBrush>()
            {
                let _ = current.SetColor(ui_color(*color));
            } else if let Some(new) = self.brush(brush, layout.brush_size) {
                let _ = if is_stroke { shape.SetStrokeBrush(&new) } else { shape.SetFillBrush(&new) };
            }
            if is_stroke {
                let _ = shape.SetStrokeThickness(layout.border_width.get());
            }
        }
        true
    }

    /// Makes the node a plain container that draws its children as a group with `opacity`,
    /// clamped to 0 to 1.
    pub fn opacity(&mut self, opacity: f32) {
        self.plain();
        let _ = self.visual.SetOpacity(opacity.clamp(0., 1.));
    }

    /// Makes the node a plain container that clips its children to `content_box`, a rectangle
    /// with corner radii in the node's coordinates, or doesn't clip for `None`. `size` is the
    /// node's size.
    pub fn clip(
        &mut self,
        size: LogicalSize,
        content_box: Option<(LogicalRect, LogicalBorderRadius)>,
    ) {
        self.plain();
        let Some((rect, radius)) = content_box else {
            let _ = self.visual.SetClip(None::<&CompositionClip>);
            let _ = self.visual.SetBorderMode(CompositionBorderMode::Inherit);
            return;
        };
        let rect = rect.cast::<f32>() * ScaleFactor::new(1.);
        let radius = radius.cast::<f32>() * ScaleFactor::new(1.);
        let _ = self.visual.SetBorderMode(soft_if(!radius.is_zero()));
        let clip: windows::core::Result<CompositionClip> = if radius.is_zero() {
            // Insets are relative to the visual's size, so the clip follows a resize.
            let size = size.cast::<f32>();
            let (right, bottom) = (size.width - rect.max_x(), size.height - rect.max_y());
            if let Ok(current) = self.visual.Clip().and_then(|c| c.cast::<InsetClip>()) {
                let _ = current.SetLeftInset(rect.min_x());
                let _ = current.SetTopInset(rect.min_y());
                let _ = current.SetRightInset(right);
                let _ = current.SetBottomInset(bottom);
                return;
            }
            self.compositor
                .CreateInsetClipWithInsets(rect.min_x(), rect.min_y(), right, bottom)
                .and_then(|c| c.cast())
        } else {
            shapes::rounded_rect(self.compositor, rect, radius)
                .and_then(|g| self.compositor.CreateGeometricClipWithGeometry(&g))
                .and_then(|c| c.cast())
        };
        if let Ok(clip) = clip {
            let _ = self.visual.SetClip(&clip);
        }
    }

    /// Makes the node a plain container that rotates its children by `rotation` degrees
    /// clockwise and then scales them by `scale_x` and `scale_y`, both about `origin`.
    pub fn transform(&mut self, rotation: f32, scale_x: f32, scale_y: f32, origin: LogicalPoint) {
        self.plain();
        // `Visual.Scale` is applied before `Visual.RotationAngle`, the opposite of Slint's
        // order, so the matrix is built here. Row vectors: `p' = p * R * S`, translated so
        // that `origin` stays in place.
        let (sin, cos) = rotation.to_radians().sin_cos();
        let (m11, m12, m21, m22) = (cos * scale_x, sin * scale_y, -sin * scale_x, cos * scale_y);
        let (x, y) = (origin.x as f32, origin.y as f32);
        let matrix = Matrix4x4 {
            M11: m11,
            M12: m12,
            M21: m21,
            M22: m22,
            M33: 1.,
            M41: x - (x * m11 + y * m21),
            M42: y - (x * m12 + y * m22),
            M44: 1.,
            ..Default::default()
        };
        let _ = self.visual.SetTransformMatrix(matrix);
        let _ = self.visual.SetBorderMode(soft_if(m12 != 0. || m21 != 0.));
    }

    /// Blurs what lies beneath the node's area, of `size`, with a Gaussian blur of `sigma`
    /// logical pixels, inside `radius`. The children aren't clipped.
    pub fn backdrop_blur(&mut self, sigma: f32, size: LogicalSize, radius: LogicalBorderRadius) {
        self.plain();
        if !(sigma > 0.) {
            self.remove_floor();
            return;
        }
        let sprite = self.surface_sprite();
        let size = size.cast::<f32>();
        // A backdrop brush sees only what's behind its sprite. The blur picks up the backdrop up
        // to 3σ beyond the item, as upstream's does, so the sprite reaches that far and is
        // clipped back to the item.
        let margin = (3. * sigma).ceil();
        let _ = sprite.SetOffset(Vector3 { X: -margin, Y: -margin, Z: 0. });
        let _ = sprite.SetSize(Vector2 { X: size.width + 2. * margin, Y: size.height + 2. * margin });
        // The compositor scales the blur with the visual, and the root carries the scale factor.
        match sprite.Brush().and_then(|b| b.cast::<CompositionEffectBrush>()) {
            Ok(brush) => {
                let _ = effects::set_blur(&brush, sigma);
            }
            Err(_) => {
                let c = self.compositor;
                let effect = effects::Effect::GaussianBlur {
                    sigma,
                    input: Box::new(effects::Effect::Source("backdrop")),
                };
                let brush = c.CreateBackdropBrush().and_then(|backdrop| {
                    effects::brush(c, &effect, &[("backdrop", &backdrop.cast()?)])
                });
                match brush {
                    Ok(brush) => {
                        let _ = sprite.SetBrush(&brush);
                    }
                    Err(e) => crate::debug_log!("cannot blur the backdrop: {}", e.message()),
                }
            }
        }
        let radius = radius.cast::<f32>() * ScaleFactor::new(1.);
        let clip: windows::core::Result<CompositionClip> = if radius.is_zero() {
            self.compositor
                .CreateInsetClipWithInsets(margin, margin, margin, margin)
                .and_then(|c| c.cast())
        } else {
            shapes::rounded_rect(
                self.compositor,
                euclid::Rect::new(euclid::point2(margin, margin), size.cast_unit()),
                radius,
            )
            .and_then(|g| self.compositor.CreateGeometricClipWithGeometry(&g))
            .and_then(|c| c.cast())
        };
        if let Ok(clip) = clip {
            let _ = sprite.SetClip(&clip);
        }
        let _ = sprite.SetBorderMode(soft_if(!radius.is_zero()));
    }

    /// The sprite for content drawn into a surface, below the node's children; made on first use.
    pub(crate) fn surface_sprite(&mut self) -> SpriteVisual {
        if let Some(sprite) = self.floor.as_ref().and_then(|f| f.cast::<SpriteVisual>().ok()) {
            return sprite;
        }
        self.remove_floor();
        let sprite = self.compositor.CreateSpriteVisual().unwrap();
        let _ = self.visual.Children().and_then(|c| c.InsertAtBottom(&sprite));
        *self.floor = Some(sprite.cast().unwrap());
        sprite
    }

    /// Drops the visual of the item's own content, if there is one.
    pub(crate) fn remove_floor(&mut self) {
        if let Some(floor) = self.floor.take() {
            let _ = self.visual.Children().and_then(|c| c.Remove(&floor));
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
        let _ = new.SetTransformMatrix(old.TransformMatrix().unwrap_or(IDENTITY));
        let _ = new.SetBorderMode(old.BorderMode().unwrap_or(CompositionBorderMode::Inherit));
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

/// Drops `nodes` in the reverse order of their creation. Releasing Composition objects oldest
/// first takes time quadratic in their number: 2000 text rows took 6 s to release, against
/// 43 ms newest first.
fn release_newest_first(mut nodes: Vec<Node>) {
    nodes.sort_unstable_by_key(|n| core::cmp::Reverse(n.serial));
    for node in nodes {
        drop(node);
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
    /// The number of nodes created so far, for `Node::serial`.
    created: u64,
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
            created: 0,
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
        self.created += 1;
        let index = self.nodes.insert(Node {
            visual,
            kind: VisualKind::Container,
            item: item_rc.downgrade(),
            geometry: None,
            content: Box::pin(PropertyTracker::default()),
            result: RenderingResult::ContinueRenderingChildren,
            children: Vec::new(),
            next_children: Vec::new(),
            floor: None,
            pass: 0,
            parent: None,
            serial: self.created,
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
        Self::patch_children(&self.nodes, &root_visual, None, &self.roots, &roots);
        self.roots = roots;
        let visited: Vec<usize> =
            self.nodes.iter().filter(|(_, n)| n.pass == pass).map(|(i, _)| i).collect();
        for index in visited {
            let next = core::mem::take(&mut self.nodes[index].next_children);
            let visual = self.nodes[index].visual.clone();
            let floor = self.nodes[index].floor.clone();
            Self::patch_children(
                &self.nodes,
                &visual,
                floor.as_ref(),
                &self.nodes[index].children,
                &next,
            );
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
                // `update_visual` gets the size as an argument, which its tracker doesn't see.
                let resized = node.geometry.is_some_and(|g| g.size != geometry.size);
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
                let Node { visual, kind, content, result, floor, .. } = node;
                if content.is_dirty() || resized {
                    let mut update =
                        VisualUpdate {
                        compositor: &self.compositor,
                        item: Some(&item_rc),
                        visual,
                        kind,
                        floor,
                    };
                    *result = content.as_ref().evaluate(|| {
                        item.as_ref().update_visual(&item_rc, &mut update, geometry.size)
                    });
                } else {
                    content.as_ref().register_as_dependency_to_current_binding();
                }
                let result = *result;
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

    /// Makes `parent`'s children match `new`, given that they currently match `old`. `floor` is
    /// the visual of the parent's own content, which stays below them.
    fn patch_children(
        nodes: &slab::Slab<Node>,
        parent: &ContainerVisual,
        floor: Option<&ContainerVisual>,
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
        let floor: Option<windows::UI::Composition::Visual> = floor.map(|f| f.cast().unwrap());
        let mut previous: Option<windows::UI::Composition::Visual> = floor;
        let mut old_pos = old.iter().filter(|id| new_set.contains(id)).copied().peekable();
        for id in new {
            let visual = &nodes[*id].visual;
            if old_pos.peek() == Some(id) {
                old_pos.next();
            } else {
                if old_set.contains(id) {
                    let _ = children.Remove(visual);
                }
                match &previous {
                    Some(prev) => children.InsertAbove(visual, prev).unwrap(),
                    None => children.InsertAtBottom(visual).unwrap(),
                }
            }
            previous = Some(visual.cast().unwrap());
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
        let stale = stale.into_iter().map(|index| self.nodes.remove(index)).collect();
        release_newest_first(stale);
    }

    /// Releases the nodes of `items`, which are being destroyed.
    pub fn release_items<'a>(&mut self, items: &mut dyn Iterator<Item = Pin<ItemRef<'a>>>) {
        let mut released = Vec::new();
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
                    released.push(node);
                }
                data.cache_generation.set(0);
            }
        }
        release_newest_first(released);
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
    /// `kind x,y wxh` plus the brush or shapes, then ` opacity=` when not 1,
    /// ` clip=inset(l,t,r,b)`, ` clip=rrect x,y wxh rR` or ` clip=path` when clipped,
    /// and ` transform=[m11 m12 m21 m22 m41 m42]` when transformed.
    /// Transform entries are rounded to 4 decimals.
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
    if let Ok(clip) = visual.Clip() {
        let _ = write!(out, " clip={}", shapes::describe_clip(&clip));
    }
    let m = visual.TransformMatrix().unwrap_or(IDENTITY);
    if m != IDENTITY {
        let round = |v: f32| {
            let r = (v * 1e4).round() / 1e4;
            if r == 0. { 0. } else { r }
        };
        let _ = write!(
            out,
            " transform=[{} {} {} {} {} {}]",
            round(m.M11),
            round(m.M12),
            round(m.M21),
            round(m.M22),
            round(m.M41),
            round(m.M42)
        );
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
            "Container 0,0 100x50
  Shape -1,-1 102x52 [fill #ff0000ff rrect 1,1 98x48 r3] [stroke #000000ff w2 rrect 1,1 98x48 r3]
  Container 0,0 10x10
"
        );

        // Different radii per corner need a path; the child stays in place.
        root.border_top_left_radius.set(LogicalLength::new(10.));
        sync(&mut visuals, &dyn_tree);
        assert_eq!(
            print(&visuals),
            "Container 0,0 100x50
  Shape -1,-1 102x52 [fill #ff0000ff path] [stroke #000000ff w2 path]
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
    fn an_empty_border_rectangle_drops_its_shape() {
        use crate::lengths::LogicalLength;
        let tree = crate::testing::BorderTree::new((100., 50.), 0);
        let root = tree.rect(0);
        root.background.set(red());
        root.border_top_left_radius.set(LogicalLength::new(2.));
        let dyn_tree = vtable::VRc::into_dyn(tree.clone());
        let mut visuals = new_tree();
        sync(&mut visuals, &dyn_tree);
        assert_eq!(
            print(&visuals),
            "Container 0,0 100x50\n  Shape -1,-1 102x52 [fill #ff0000ff path]\n"
        );

        // As Fluent's list item hides its selection marker: the height becomes 0.
        tree.geometry(0).set(LogicalRect::new(euclid::point2(0., 0.), euclid::size2(100., 0.)));
        sync(&mut visuals, &dyn_tree);
        assert_eq!(print(&visuals), "Container 0,0 100x0\n");
    }

    #[test]
    fn a_path_is_one_shape_that_covers_what_it_draws() {
        use crate::lengths::LogicalLength;
        let tree = crate::testing::PathTree::new((100., 50.), 0);
        let path = tree.rect(0);
        path.elements.set(crate::graphics::PathData::Commands("M 10 5 L 40 5 L 40 30".into()));
        path.fit.set(crate::items::ImageFit::Preserve);
        path.stroke_miter_limit.set(4.);
        path.fill.set(red());
        let dyn_tree = vtable::VRc::into_dyn(tree.clone());
        let mut visuals = new_tree();
        sync(&mut visuals, &dyn_tree);
        assert_eq!(
            print(&visuals),
            "Container 0,0 100x50\n  Shape -1,-1 102x52 [fill #ff0000ff path]\n"
        );

        path.stroke.set(Brush::SolidColor(Color::from_rgb_u8(0, 0, 0)));
        path.stroke_width.set(LogicalLength::new(2.));
        sync(&mut visuals, &dyn_tree);
        assert_eq!(
            print(&visuals),
            "Container 0,0 100x50\n  Shape -1,-1 102x52 [fill #ff0000ff stroke #000000ff w2 path]\n"
        );

        // The shape visual cuts off what lies outside it: it grows to the path and its stroke,
        // which reaches four half widths past a vertex, the miter limit.
        path.elements.set(crate::graphics::PathData::Commands("M -10 5 L 40 5".into()));
        sync(&mut visuals, &dyn_tree);
        assert_eq!(
            print(&visuals),
            "Container 0,0 100x50\n  Shape -15,-1 116x52 [fill #ff0000ff stroke #000000ff w2 path]\n"
        );

        // Not so when the path clips at the item.
        path.clip.set(true);
        sync(&mut visuals, &dyn_tree);
        assert_eq!(
            print(&visuals),
            "Container 0,0 100x50\n  Shape -1,-1 102x52 [fill #ff0000ff stroke #000000ff w2 path] clip=inset(1,1,1,1)\n"
        );
        path.clip.set(false);

        // Nothing to draw: no brush, or no elements.
        path.stroke.set(Brush::default());
        path.fill.set(Brush::default());
        sync(&mut visuals, &dyn_tree);
        assert_eq!(print(&visuals), "Container 0,0 100x50\n");
        path.fill.set(red());
        path.elements.set(crate::graphics::PathData::None);
        sync(&mut visuals, &dyn_tree);
        assert_eq!(print(&visuals), "Container 0,0 100x50\n");
    }

    #[test]
    fn a_path_stroke_has_the_caps_joins_and_dashes_of_the_item() {
        use crate::items::{LineCap, LineJoin};
        use crate::lengths::LogicalLength;
        let tree = crate::testing::PathTree::new((100., 50.), 0);
        let path = tree.rect(0);
        path.elements.set(crate::graphics::PathData::Commands("M 10 5 L 40 5 L 40 30".into()));
        path.fit.set(crate::items::ImageFit::Preserve);
        path.stroke.set(red());
        path.stroke_width.set(LogicalLength::new(3.));
        path.stroke_miter_limit.set(4.);
        let dyn_tree = vtable::VRc::into_dyn(tree.clone());
        let mut visuals = new_tree();
        // What the dump says about the stroke after its width.
        let stroke = |visuals: &mut VisualTree| -> alloc::string::String {
            sync(visuals, &dyn_tree);
            let dump = visuals.dump();
            let style = dump.split_once("stroke #ff0000ff w3").unwrap().1;
            style.split_once(" path").unwrap().0.into()
        };
        assert_eq!(stroke(&mut visuals), "");

        path.stroke_line_cap.set(LineCap::Round);
        path.stroke_line_join.set(LineJoin::Bevel);
        assert_eq!(stroke(&mut visuals), " cap=round join=bevel");
        path.stroke_line_cap.set(LineCap::Square);
        path.stroke_line_join.set(LineJoin::Round);
        assert_eq!(stroke(&mut visuals), " cap=square join=round");
        path.stroke_line_cap.set(LineCap::Butt);
        path.stroke_line_join.set(LineJoin::Miter);
        path.stroke_miter_limit.set(1.5);
        assert_eq!(stroke(&mut visuals), " miter=1.5");

        // Dashes are in logical pixels, and the offset wraps around the pattern, as in Skia.
        path.stroke_miter_limit.set(4.);
        path.stroke_dash_array.set(crate::SharedVector::from_slice(&[8., 5.]));
        path.stroke_dash_offset.set(LogicalLength::new(4.));
        assert_eq!(stroke(&mut visuals), " dash=[8,5]+4");
        path.stroke_dash_offset.set(LogicalLength::new(-6.));
        assert_eq!(stroke(&mut visuals), " dash=[8,5]+7");

        // Skia ignores a pattern it can't use.
        path.stroke_dash_array.set(crate::SharedVector::from_slice(&[8., 5., 2.]));
        assert_eq!(stroke(&mut visuals), "");
        path.stroke_dash_array.set(crate::SharedVector::from_slice(&[0., 0.]));
        assert_eq!(stroke(&mut visuals), "");

        // A stroke of width 0 is a hairline in Skia: one pixel wide.
        path.stroke_width.set(LogicalLength::new(0.));
        sync(&mut visuals, &dyn_tree);
        assert!(visuals.dump().contains("stroke #ff0000ff w1 path"));
    }

    #[test]
    fn a_path_antialiases_as_its_property_says() {
        let tree = crate::testing::PathTree::new((100., 50.), 0);
        let path = tree.rect(0);
        path.elements.set(crate::graphics::PathData::Commands("M 10 5 L 40 5 L 40 30".into()));
        path.fill.set(red());
        let dyn_tree = vtable::VRc::into_dyn(tree.clone());
        let mut visuals = new_tree();
        // The border mode of the shape visual.
        let border_mode = |visuals: &VisualTree| {
            let node = (&visuals.root.Children().unwrap()).into_iter().next().unwrap();
            let floor = node.cast::<ContainerVisual>().unwrap().Children().unwrap();
            (&floor).into_iter().next().unwrap().BorderMode().unwrap()
        };
        path.anti_alias.set(true);
        sync(&mut visuals, &dyn_tree);
        assert_eq!(border_mode(&visuals), CompositionBorderMode::Soft);
        path.anti_alias.set(false);
        sync(&mut visuals, &dyn_tree);
        assert_eq!(border_mode(&visuals), CompositionBorderMode::Hard);
    }

    #[test]
    fn a_path_gradient_spans_the_viewbox_scaled_to_the_item() {
        use crate::graphics::{GradientStop, LinearGradientBrush};
        let tree = crate::testing::PathTree::new((100., 50.), 0);
        let path = tree.rect(0);
        path.elements.set(crate::graphics::PathData::Commands("M 10 5 L 40 5 L 40 30".into()));
        path.fit.set(crate::items::ImageFit::Preserve);
        path.fill.set(Brush::LinearGradient(LinearGradientBrush::new(
            90.,
            [
                GradientStop { color: Color::from_rgb_u8(255, 0, 0), position: 0. },
                GradientStop { color: Color::from_rgb_u8(0, 0, 255), position: 1. },
            ],
        )));
        let dyn_tree = vtable::VRc::into_dyn(tree.clone());
        let mut visuals = new_tree();
        sync(&mut visuals, &dyn_tree);
        assert_eq!(
            print(&visuals),
            "Container 0,0 100x50\n  Shape -1,-1 102x52 [fill linear(0,25->100,25 #ff0000ff@0 #0000ffff@1) path]\n"
        );

        // With a viewbox of 20 by 10, the gradient is made for that and scaled by 5.
        path.viewbox_width.set(20.);
        path.viewbox_height.set(10.);
        sync(&mut visuals, &dyn_tree);
        assert_eq!(
            print(&visuals),
            "Container 0,0 100x50\n  Shape -1,-1 102x52 [fill linear(0,5->20,5 #ff0000ff@0 #0000ffff@1 m=[5 0 0 5 0 0]) path]\n"
        );
    }

    #[test]
    fn a_resize_alone_updates_shapes_and_brushes() {
        use crate::graphics::{GradientStop, LinearGradientBrush};
        use crate::lengths::LogicalLength;
        let tree = crate::testing::BorderTree::new((100., 50.), 0);
        let root = tree.rect(0);
        root.background.set(Brush::LinearGradient(LinearGradientBrush::new(
            90.,
            [
                GradientStop { color: Color::from_rgb_u8(255, 0, 0), position: 0. },
                GradientStop { color: Color::from_rgb_u8(0, 0, 255), position: 1. },
            ],
        )));
        root.border_top_left_radius.set(LogicalLength::new(4.));
        let dyn_tree = vtable::VRc::into_dyn(tree.clone());
        let mut visuals = new_tree();
        sync(&mut visuals, &dyn_tree);
        assert_eq!(
            print(&visuals),
            "Container 0,0 100x50
  Shape -1,-1 102x52 [fill linear(0,25->100,25 #ff0000ff@0 #0000ffff@1) path]
"
        );

        tree.geometry(0).set(LogicalRect::new(Default::default(), LogicalSize::new(200., 80.)));
        sync(&mut visuals, &dyn_tree);
        assert_eq!(
            print(&visuals),
            "Container 0,0 200x80
  Shape -1,-1 202x82 [fill linear(0,40->200,40 #ff0000ff@0 #0000ffff@1) path]
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
    fn conic_gradients_are_bitmaps_and_cut_to_the_shape_they_fill() {
        use crate::graphics::{ConicGradientBrush, GradientStop};
        use crate::lengths::LogicalLength;
        let conic = Brush::ConicGradient(ConicGradientBrush::new(
            0.,
            [
                GradientStop { color: Color::from_rgb_u8(255, 0, 0), position: 0. },
                GradientStop { color: Color::from_rgb_u8(0, 0, 255), position: 1. },
            ],
        ));
        let tree = crate::testing::BorderTree::new((100., 50.), 0);
        let root = tree.rect(0);
        root.background.set(conic.clone());
        let dyn_tree = vtable::VRc::into_dyn(tree.clone());
        let mut visuals = new_tree();
        sync(&mut visuals, &dyn_tree);
        // Four bitmap pixels per pixel, as small as the item is.
        assert_eq!(print(&visuals), "Sprite 0,0 100x50 surface 400x200 m=[0.25 0 0 0.25 0 0]\n");

        // Shapes can't show it: a sprite cut to the rounded rectangle, and one to the border.
        root.border_top_left_radius.set(LogicalLength::new(4.));
        root.border_width.set(LogicalLength::new(2.));
        root.border_color.set(conic);
        sync(&mut visuals, &dyn_tree);
        assert_eq!(
            print(&visuals),
            "Container 0,0 100x50
  Container -1,-1 102x52
    Sprite 0,0 102x52 surface 400x200 m=[0.25 0 0 0.25 1 1] clip=path
    Sprite 0,0 102x52 surface 400x200 m=[0.25 0 0 0.25 1 1] clip=path
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

    #[test]
    fn opacity_is_group_opacity_on_the_container() {
        use crate::testing::OpacityTree;
        let tree = OpacityTree::new((100., 50.), 1);
        let item = tree.rect(0);
        item.opacity.set(0.5);
        // The child is an opacity item too, and has its own.
        let child = tree.rect(1);
        child.opacity.set(1.);
        let dyn_tree = vtable::VRc::into_dyn(tree.clone());
        let mut visuals = new_tree();
        sync(&mut visuals, &dyn_tree);
        assert_eq!(print(&visuals), "Container 0,0 100x50 opacity=0.5\n  Container 0,0 10x10\n");
        child.opacity.set(0.25);
        sync(&mut visuals, &dyn_tree);
        assert_eq!(
            print(&visuals),
            "Container 0,0 100x50 opacity=0.5\n  Container 0,0 10x10 opacity=0.25\n"
        );
        child.opacity.set(1.);

        // Out of range values are clamped.
        item.opacity.set(1.5);
        sync(&mut visuals, &dyn_tree);
        assert_eq!(print(&visuals), "Container 0,0 100x50\n  Container 0,0 10x10\n");
        item.opacity.set(-1.);
        sync(&mut visuals, &dyn_tree);
        assert_eq!(print(&visuals), "Container 0,0 100x50 opacity=0\n  Container 0,0 10x10\n");
    }

    #[test]
    fn a_clip_without_radius_is_an_inset_clip() {
        use crate::lengths::LogicalLength;
        use crate::testing::ClipTree;
        let tree = ClipTree::new((100., 50.), 1);
        let item = tree.rect(0);
        let dyn_tree = vtable::VRc::into_dyn(tree.clone());
        let mut visuals = new_tree();
        sync(&mut visuals, &dyn_tree);
        assert_eq!(print(&visuals), "Container 0,0 100x50\n  Container 0,0 10x10\n");

        item.clip.set(true);
        sync(&mut visuals, &dyn_tree);
        assert_eq!(
            print(&visuals),
            "Container 0,0 100x50 clip=inset(0,0,0,0)\n  Container 0,0 10x10\n"
        );

        // Children are clipped to what's inside the border.
        item.border_width.set(LogicalLength::new(2.));
        sync(&mut visuals, &dyn_tree);
        assert_eq!(
            print(&visuals),
            "Container 0,0 100x50 clip=inset(2,2,2,2)\n  Container 0,0 10x10\n"
        );

        item.clip.set(false);
        sync(&mut visuals, &dyn_tree);
        assert_eq!(print(&visuals), "Container 0,0 100x50\n  Container 0,0 10x10\n");
    }

    #[test]
    fn a_clip_with_radius_is_a_geometric_clip() {
        use crate::lengths::LogicalLength;
        use crate::testing::ClipTree;
        let tree = ClipTree::new((100., 50.), 1);
        let item = tree.rect(0);
        item.clip.set(true);
        let radii = [
            &item.border_top_left_radius,
            &item.border_top_right_radius,
            &item.border_bottom_right_radius,
            &item.border_bottom_left_radius,
        ];
        for radius in radii {
            radius.set(LogicalLength::new(4.));
        }
        let dyn_tree = vtable::VRc::into_dyn(tree.clone());
        let mut visuals = new_tree();
        sync(&mut visuals, &dyn_tree);
        assert_eq!(
            print(&visuals),
            "Container 0,0 100x50 clip=rrect 0,0 100x50 r4\n  Container 0,0 10x10\n"
        );

        // The border takes up part of the radius.
        item.border_width.set(LogicalLength::new(2.));
        sync(&mut visuals, &dyn_tree);
        assert_eq!(
            print(&visuals),
            "Container 0,0 100x50 clip=rrect 2,2 96x46 r2\n  Container 0,0 10x10\n"
        );

        item.border_top_left_radius.set(LogicalLength::new(10.));
        sync(&mut visuals, &dyn_tree);
        assert_eq!(print(&visuals), "Container 0,0 100x50 clip=path\n  Container 0,0 10x10\n");

        // Back to the inset clip, then to none.
        for radius in radii {
            radius.set(LogicalLength::new(0.));
        }
        sync(&mut visuals, &dyn_tree);
        assert_eq!(
            print(&visuals),
            "Container 0,0 100x50 clip=inset(2,2,2,2)\n  Container 0,0 10x10\n"
        );
        item.clip.set(false);
        sync(&mut visuals, &dyn_tree);
        assert_eq!(print(&visuals), "Container 0,0 100x50\n  Container 0,0 10x10\n");
    }

    #[test]
    fn a_resize_alone_updates_the_clip() {
        use crate::lengths::LogicalLength;
        use crate::testing::ClipTree;
        let tree = ClipTree::new((100., 50.), 1);
        let item = tree.rect(0);
        item.clip.set(true);
        item.border_width.set(LogicalLength::new(2.));
        let dyn_tree = vtable::VRc::into_dyn(tree.clone());
        let mut visuals = new_tree();
        sync(&mut visuals, &dyn_tree);
        let resized = LogicalRect::new(Default::default(), LogicalSize::new(200., 80.));

        // The insets stay, since they're relative to the visual's size.
        tree.geometry(0).set(resized);
        sync(&mut visuals, &dyn_tree);
        assert_eq!(
            print(&visuals),
            "Container 0,0 200x80 clip=inset(2,2,2,2)\n  Container 0,0 10x10\n"
        );

        // A rounded clip has a size of its own.
        item.border_top_left_radius.set(LogicalLength::new(6.));
        item.border_top_right_radius.set(LogicalLength::new(6.));
        item.border_bottom_right_radius.set(LogicalLength::new(6.));
        item.border_bottom_left_radius.set(LogicalLength::new(6.));
        sync(&mut visuals, &dyn_tree);
        assert_eq!(
            print(&visuals),
            "Container 0,0 200x80 clip=rrect 2,2 196x76 r4\n  Container 0,0 10x10\n"
        );
        tree.geometry(0).set(LogicalRect::new(Default::default(), LogicalSize::new(100., 50.)));
        sync(&mut visuals, &dyn_tree);
        assert_eq!(
            print(&visuals),
            "Container 0,0 100x50 clip=rrect 2,2 96x46 r4\n  Container 0,0 10x10\n"
        );
    }

    /// The transform of the first top-level visual.
    fn transform_of_first(visuals: &VisualTree) -> Matrix4x4 {
        let first = (&visuals.root.Children().unwrap()).into_iter().next().unwrap();
        first.TransformMatrix().unwrap()
    }

    /// Where `m` maps the point `(x, y)`, rounded to 4 decimals. The matrix multiplies row
    /// vectors.
    fn map_point(m: Matrix4x4, x: f32, y: f32) -> (f32, f32) {
        let round = |v: f32| (v * 1e4).round() / 1e4 + 0.;
        (round(x * m.M11 + y * m.M21 + m.M41), round(x * m.M12 + y * m.M22 + m.M42))
    }

    fn transform_tree(
        rotation: f32,
        scale: (f32, f32),
        origin: (f32, f32),
    ) -> (VisualTree, ItemTreeRc) {
        use crate::api::LogicalPosition;
        let tree = crate::testing::TransformTree::new((100., 50.), 1);
        let item = tree.rect(0);
        item.transform_rotation.set(rotation);
        item.transform_scale_x.set(scale.0);
        item.transform_scale_y.set(scale.1);
        item.transform_origin.set(LogicalPosition::new(origin.0, origin.1));
        // The child is a transform item too; this keeps it untransformed.
        tree.rect(1).transform_scale_x.set(1.);
        tree.rect(1).transform_scale_y.set(1.);
        let dyn_tree = vtable::VRc::into_dyn(tree.clone());
        let mut visuals = new_tree();
        sync(&mut visuals, &dyn_tree);
        (visuals, dyn_tree)
    }

    #[test]
    fn transforms_are_matrices_on_the_container() {
        // Clockwise on screen: right turns into down.
        let (visuals, _tree) = transform_tree(90., (1., 1.), (0., 0.));
        assert_eq!(
            print(&visuals),
            "Container 0,0 100x50 transform=[0 1 -1 0 0 0]\n  Container 0,0 10x10\n"
        );
        assert_eq!(map_point(transform_of_first(&visuals), 1., 0.), (0., 1.));

        let (visuals, _tree) = transform_tree(0., (2., 2.), (5., 5.));
        assert_eq!(
            print(&visuals),
            "Container 0,0 100x50 transform=[2 0 0 2 -5 -5]\n  Container 0,0 10x10\n"
        );
        // The origin stays where it is.
        assert_eq!(map_point(transform_of_first(&visuals), 5., 5.), (5., 5.));
        assert_eq!(map_point(transform_of_first(&visuals), 6., 5.), (7., 5.));

        let (visuals, _tree) = transform_tree(90., (1., 1.), (5., 5.));
        assert_eq!(map_point(transform_of_first(&visuals), 5., 5.), (5., 5.));
        assert_eq!(map_point(transform_of_first(&visuals), 6., 5.), (5., 6.));
    }

    #[test]
    fn transforms_rotate_before_they_scale() {
        let (visuals, _tree) = transform_tree(90., (2., 1.), (0., 0.));
        assert_eq!(
            print(&visuals),
            "Container 0,0 100x50 transform=[0 1 -2 0 0 0]\n  Container 0,0 10x10\n"
        );
        let m = transform_of_first(&visuals);
        assert_eq!(map_point(m, 1., 0.), (0., 1.));
        // Rotated to (-1, 1), then scaled to (-2, 1); scaling first would give (-1, 2).
        assert_eq!(map_point(m, 1., 1.), (-2., 1.));
    }

    #[test]
    fn the_identity_transform_leaves_the_matrix_alone() {
        let (mut visuals, tree) = transform_tree(0., (1., 1.), (5., 5.));
        assert_eq!(print(&visuals), "Container 0,0 100x50\n  Container 0,0 10x10\n");
        assert_eq!(transform_of_first(&visuals), IDENTITY);

        // Back to the identity after a transform.
        let item = ItemRc::new(tree.clone(), 0);
        let item = ItemRef::downcast_pin::<crate::items::Transform>(item.borrow()).unwrap();
        item.transform_scale_x.set(3.);
        sync(&mut visuals, &tree);
        assert_eq!(
            print(&visuals),
            "Container 0,0 100x50 transform=[3 0 0 1 -10 0]\n  Container 0,0 10x10\n"
        );
        item.transform_scale_x.set(1.);
        sync(&mut visuals, &tree);
        assert_eq!(print(&visuals), "Container 0,0 100x50\n  Container 0,0 10x10\n");
        assert_eq!(transform_of_first(&visuals), IDENTITY);
    }

    #[test]
    fn layers_are_plain_containers() {
        let tree = crate::testing::LayerTree::new((100., 50.), 1);
        tree.rect(0).cache_rendering_hint.set(true);
        let dyn_tree = vtable::VRc::into_dyn(tree.clone());
        let mut visuals = new_tree();
        sync(&mut visuals, &dyn_tree);
        assert_eq!(print(&visuals), "Container 0,0 100x50\n  Container 0,0 10x10\n");
    }

    #[test]
    fn a_new_kind_of_visual_keeps_the_clip_and_transform() {
        let c = compositor();
        let mut visual = c.CreateContainerVisual().unwrap();
        let mut kind = VisualKind::Container;
        let mut floor = None;
        let mut update = VisualUpdate {
            compositor: &c,
            item: None,
            visual: &mut visual,
            kind: &mut kind,
            floor: &mut floor,
        };
        let size = LogicalSize::new(30., 20.);
        update.transform(90., 1., 1., LogicalPoint::default());
        update.clip(size, Some((LogicalRect::from_size(size), Default::default())));
        let _ = update.sprite();
        let mut out = alloc::string::String::new();
        dump_visual(&visual, 0, &mut out);
        assert_eq!(out, "Sprite 0,0 0x0 clip=inset(0,0,0,0) transform=[0 1 -1 0 0 0]\n");
    }
}
