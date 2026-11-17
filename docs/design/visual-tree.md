# Visual Tree Sync Engine (Draft)

Status: draft for Phase 1, based on spikes [S1](spike-s1-hosting.md), [S2](spike-s2-primitives.md), and [S3](spike-s3-verification.md).
Phase 2 finalizes it.

## Goal

Keep one `Windows.UI.Composition` visual per item in sync with the item tree, touching only the visuals whose properties changed.
Rendering itself is the compositor's job; the runtime never draws a frame.

## How Upstream Detects Changes

Upstream's `PartialRenderer::compute_dirty_regions` (`crates/core/partial_renderer.rs`) walks the **whole** item tree on every redraw.
Each item has a `PropertyTracker` in a cache entry that `CachedRenderingData` points to.
The walk compares each item's geometry with the cached one and asks its tracker whether rendering properties changed.
Every tracker that isn't dirty registers as a dependency of the window's `redraw_tracker` (`WindowInner::draw_contents`), so any later change dirties the window and requests a redraw.
Structural changes (repeaters, conditionals, popups) are caught because the walk itself reads the repeater models.

## Design

### Sync Pass

The sync pass replaces `draw_contents`.
It runs on the UI thread when the window's tracker is dirty, at most once per frame tick or message batch.

1. `ensure_tree_instantiated()` outside any tracking, as upstream does.
2. Evaluate the walk inside the window tracker (`evaluate_as_dependency_root`), so the walk's reads register as dependencies.
3. Visit items back to front with `visit_item_tree_z_sorted` order.
   For each item:
   - Look up or create its node through `cached_rendering_data` (index + generation into the window's node table).
   - **Geometry**: read `item_rc.geometry()`; if it differs from the node's cached geometry, set `Offset`/`Size` on the visual.
   - **Content**: if the node's content tracker is dirty (or new), evaluate `ItemVTable::update_visual` inside it; the item sets visual properties directly.
     Otherwise register the clean tracker as a dependency of the window tracker.
   - **Children**: compare the visited child sequence with the node's previous one and patch the `VisualCollection` (see below).
4. Nodes not visited in this pass belong to destroyed items or trees; release their visuals.
5. The dispatcher queue commits implicitly when the handler returns (S1).

Version 1 walks all items whenever anything changed, like upstream.
The walk is cheap compared with GPU work, and it handles structure changes without extra bookkeeping.
If profiling in Phase 2 shows the walk above about 1 ms for realistic trees, switch to per-node dirty handlers that enqueue nodes, plus a structure tracker per node with dynamic children.

### Node Table

- Per window, a slab of nodes: `{ visual, kind, geometry, content_tracker, children: Vec<NodeId>, generation }`.
- `CachedRenderingData { cache_index, cache_generation }` stays in every item struct (it's part of the generated-code contract) and holds the node handle.
  A generation mismatch means "no node yet".
- Item structs never hold COM objects, so the generated code's layout and the `FieldOffsets` stay unchanged.
- `unregister_item_tree` releases the nodes of the items it lists, replacing `Renderer::free_graphics_resources`.

### Visual Types and Changes of Type

Per S2: `ContainerVisual` for `Empty`/`Transform`/`Layer`, `SpriteVisual` for a plain `Rectangle`, `ShapeVisual` for borders and radii, and properties (`Clip`, `Opacity`) on the item's own container.
When an item needs a different visual type (a radius becomes non-zero), the node creates the new visual, moves the children over, and replaces it in the parent's collection.

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

- A property change dirties the window tracker; its dirty handler schedules a sync pass with `PostMessage` (coalesced).
- While `has_active_animations()` holds, the frame thread (S1) ticks with the compositor clock; each tick runs `update_timers_and_animations` and then the sync pass.
- With no animation, nothing runs.

### Testing Hooks

- The `testing` module builds a window without an HWND: a detached root visual with a fixed scale factor (S3).
- Dumps walk the real visual tree; pixel tests capture the root after `wait_commit`.

## Open Questions

- How `Flickable`'s viewport and `Transform`'s children transform map: a visual `TransformMatrix` on the node versus an extra container.
- Whether to share one brush per color across the window (S2 measured the memory difference).
- How items that draw into surfaces (text, images, paths) report their content size and DPI changes; settled with S4.
