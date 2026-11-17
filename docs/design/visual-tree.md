# Visual Tree Sync Engine

Based on spikes [S1](spike-s1-hosting.md), [S2](spike-s2-primitives.md), and [S3](spike-s3-verification.md), and settled in Phase 2.
The code is `crates/core/visual_tree.rs` and `crates/core/host.rs`.

## Goal

Keep one `Windows.UI.Composition` visual per item in sync with the item tree, touching only the visuals whose properties changed.
Rendering itself is the compositor's job; the runtime never draws a frame.

## How Upstream Detects Changes

Upstream's `PartialRenderer::compute_dirty_regions` walks the **whole** item tree on every redraw.
Each item has a `PropertyTracker` in a cache entry that `CachedRenderingData` points to.
The walk compares each item's geometry with the cached one and asks its tracker whether rendering properties changed.
Every tracker that isn't dirty registers as a dependency of the window's `redraw_tracker` (`WindowInner::draw_contents`), so any later change dirties the window and requests a redraw.
Structural changes (repeaters, conditionals, popups) are caught because the walk itself reads the repeater models.

## Design

### Sync Pass

`WindowInner::sync_visuals` replaces `draw_contents`.
The host runs it on the UI thread when the window's tracker is dirty, at most once per frame tick or message batch.

1. `ensure_tree_instantiated()` outside any tracking, as upstream does.
2. Evaluate the walk inside the window tracker (`evaluate_as_dependency_root`), so the walk's reads register as dependencies.
   The root visual's scale is applied here too, from `WindowInner::scale_factor()`, so a scale change requests a pass.
3. Visit items back to front.
   Generated code visits in `z` order (`visit_item_tree_z_sorted`) when an element sets `z`.
   For each item:
   - Look up or create its node through `cached_rendering_data` (index + generation into the window's node table).
   - **Geometry**: read `item_rc.geometry()`; if it differs from the node's cached geometry, set `Offset`/`Size` on the visual.
   - **Content**: if the node's content tracker is dirty (or new), or the size changed, evaluate `ItemVTable::update_visual` inside it.
     The size is an argument of `update_visual`, so its tracker can't see it change.
     Otherwise register the clean tracker as a dependency of the window tracker.
   - The node keeps what `update_visual` returned, so a clean node skips its children as its last update said.
   - **Children**: compare the visited child sequence with the node's previous one and patch the `VisualCollection` (see below).
4. Nodes of live items not visited in this pass are detached but kept: a conditional or a replaced component may come back.
   `unregister_item_tree` releases nodes when their items are destroyed, and the pass drops any node whose item is gone.
5. The dispatcher queue commits implicitly when the handler returns (S1).

The pass walks all items whenever anything changed, like upstream.
If profiling shows the walk above about 1 ms for realistic trees, switch to per-node dirty handlers that enqueue nodes, plus a structure tracker per node with dynamic children.

### Node Table

- Per window, a slab of nodes: `{ visual, kind, item, geometry, content, result, children, parent }`.
- `CachedRenderingData { cache_index, cache_generation }` stays in every item struct (it's part of the generated-code contract) and holds the node handle.
  A generation mismatch means "no node yet".
- Item structs never hold COM objects, so the generated code's layout and the `FieldOffsets` stay unchanged.
- `unregister_item_tree` releases the nodes of the items it lists through `Renderer::free_graphics_resources`.

### Visuals per Item

| Item | Visual | Set by `update_visual` |
| --- | --- | --- |
| `Empty`, `Layer` | `ContainerVisual` | nothing |
| `Rectangle`, `WindowItem` | `SpriteVisual`, or a container when transparent | color brush, reused across color changes; gradient brushes with absolute mapping |
| `BasicBorderRectangle`, `BorderRectangle` | `ShapeVisual`; a sprite without border and radius | a fill shape and a stroke shape laid out by upstream's `BorderRectLayout` |
| `Opacity` | `ContainerVisual` | `Opacity`, which is group opacity on a container (S2) |
| `Clip` | `ContainerVisual` | `InsetClip`, or `CompositionGeometricClip` with a rounded rectangle or path |
| `Transform` | `ContainerVisual` | `TransformMatrix` |

- `Transform` builds the matrix itself: Composition's `Scale` and `RotationAngle` apply scale before rotation, and Slint rotates first.
- `Layer`'s `cache-rendering-hint` has no equivalent; Composition retains content anyway.
- When an item needs a different visual type (a radius becomes non-zero), the node creates the new visual,
  copies offset, size, opacity, clip and transform, moves the children over, and replaces it in the parent's collection.

### Children Patching

- Keep each node's child list from the previous pass.
- In the common case (same list), do nothing.
- Otherwise insert new visuals with `InsertAbove(new, previous_sibling)`, or `InsertAtBottom` for the first child, and remove dropped ones.
  Never use `InsertAtTop`: it's linear per call (S2).
- Reordering (`z` changes) removes and reinserts the moved visuals.

### Coordinates

- Item geometry is relative to the parent item, in logical pixels.
  Visual offsets use it directly.
- The window's root visual carries `Scale = scale_factor`; the compositor snaps axis-aligned edges (S2).
- Popups shown inside the main window (`PopupWindowLocation::ChildWindow`) get their own subtree above the main tree, offset by the popup location.

### Frames and Animations

- A property change dirties the window tracker; `request_redraw` posts one sync message, coalesced until the pass runs.
- When a pass finds active animations, it asks the frame thread (S1) for ticks on the compositor clock.
  Each tick moves the animation tick to now, and the resulting changes request the next pass.
- While frames tick, only the frame tick moves the animation tick.
  The message loop still fires timers, but moving the animation tick on every message would request a pass per message and spin the loop.
- With no animation, nothing runs, and the message loop keeps the animation tick current so that a new animation starts from now.

### Testing Hooks

- Detached windows have no HWND: a root visual that hangs nowhere, with the scale factor tests dispatch (S3).
  Their sync passes run when a test calls `sync_now`.
- Dumps walk the real visual tree; pixel tests capture the root after `wait_commit` (`tests/fixtures`).
- `tests/probe` runs code from the upstream compiler and dumps the tree of a real window after each step.

## Open Questions

- `Flickable`'s viewport: a `TransformMatrix`, or an offset on an extra container (Phase 3).
- Whether to share one brush per color across the window (S2 measured the memory difference).
- How items that draw into surfaces (text, images, paths) report their content size and DPI changes; settled with S4.
- `WindowInner::set_component` on a shown window keeps the old component alive until the window is hidden, as upstream does.
  Generated code only sets the component before showing.
