// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

use std::collections::HashMap;
use std::pin::Pin;
use std::ptr::NonNull;
use std::rc::Weak;

use crate::SharedString;
use crate::accessibility::{
    AccessibilityAction, AccessibleStringProperty, SupportedAccessibilityAction,
    find_exposed_text_input, nearest_accessible_item,
};
use crate::api::Window;
use crate::input::FocusReason;
use crate::item_tree::{ItemTreeRc, ItemTreeRef, ItemTreeWeak};
use crate::items::{ItemRc, WindowItem};
use crate::lengths::{LogicalPoint, ScaleFactor};
use crate::window::{PopupWindowLocation, WindowInner};
use crate::{properties::PropertyTracker, window::WindowAdapter};
use accesskit::{
    Action, ActionRequest, Live, Node, NodeId, Orientation, Role, Toggled, TreeId, TreeInfo,
    TreeUpdate,
};

use super::Win32WindowAdapter;
use core::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::mpsc;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    PostMessageW, WM_ENTERMENULOOP, WM_ENTERSIZEMOVE, WM_EXITMENULOOP, WM_EXITSIZEMOVE,
    WM_GETOBJECT, WM_KILLFOCUS, WM_SETFOCUS,
};

// Adapted from upstream's `internal/backends/winit/accesskit.rs`: `accesskit_windows` takes the
// place of `accesskit_winit`, and messages posted to the window take the place of winit's events.

/// Posted to the window: AccessKit asks for the tree (`wParam` 0), or for actions (`wParam` 1).
pub(super) const WM_APP_ACCESSKIT: u32 = super::WM_APP + 7;
const INITIAL_TREE: usize = 0;
const ACTIONS: usize = 1;

/// The accessibility of a window, created with its HWND.
pub(super) struct AccessKit {
    platform: Rc<Platform>,
    adapter: RefCell<AccessKitAdapter>,
    /// For a top-level window: whether it has the keyboard focus, and no menu is open and it
    /// doesn't move or resize. UI Automation moves the focus to an open menu, and to nothing
    /// while the window moves or resizes.
    window_focused: Cell<bool>,
    /// Whether AccessKit was last told that the window has the focus: the top-level window is
    /// focused, and the keys come to this window.
    has_focus: Cell<bool>,
}

impl AccessKit {
    pub(super) fn new(window_adapter_weak: Weak<Win32WindowAdapter>, hwnd: HWND) -> Self {
        let platform = Rc::new(Platform::new(hwnd));
        Self {
            adapter: RefCell::new(AccessKitAdapter::new(window_adapter_weak, platform.clone())),
            platform,
            window_focused: Cell::new(false),
            has_focus: Cell::new(false),
        }
    }

    /// Handles `WM_GETOBJECT` and follows the keyboard focus of the window; `None` for the
    /// messages that the window handles as before.
    pub(super) fn handle(
        &self,
        window_adapter: &Win32WindowAdapter,
        msg: u32,
        wp: WPARAM,
        lp: LPARAM,
    ) -> Option<LRESULT> {
        match msg {
            WM_GETOBJECT => return self.platform.handle_wm_getobject(wp, lp),
            WM_SETFOCUS | WM_KILLFOCUS => {
                self.window_focused.set(msg == WM_SETFOCUS);
                sync_keyboard_focus(window_adapter);
                if let Ok(mut adapter) = self.adapter.try_borrow_mut() {
                    adapter.window_focus_changed();
                }
            }
            WM_EXITMENULOOP | WM_EXITSIZEMOVE => {
                self.window_focused.set(true);
                sync_keyboard_focus(window_adapter);
            }
            WM_ENTERMENULOOP | WM_ENTERSIZEMOVE => {
                self.window_focused.set(false);
                sync_keyboard_focus(window_adapter);
            }
            _ => {}
        }
        None
    }

    /// Handles `WM_APP_ACCESSKIT`.
    pub(super) fn handle_app_message(&self, window: &Window, wp: WPARAM) {
        match wp.0 {
            INITIAL_TREE => self.adapter.borrow_mut().initial_tree_requested(),
            ACTIONS => {
                while let Ok(request) = self.platform.requests.try_recv() {
                    let deferred_action = self.adapter.borrow_mut().handle_request(request);
                    // The adapter isn't borrowed anymore: the action may change the tree.
                    if let Some(deferred_action) = deferred_action {
                        deferred_action.invoke(window);
                    }
                }
            }
            _ => {}
        }
    }

    pub(super) fn handle_focus_item_change(&self) {
        if let Ok(mut adapter) = self.adapter.try_borrow_mut() {
            adapter.handle_focus_item_change();
        }
    }

    pub(super) fn reload_tree(&self) {
        // Borrowed: the items were created while the tree was built, which visits them.
        if let Ok(mut adapter) = self.adapter.try_borrow_mut() {
            adapter.reload_tree();
        }
    }

    pub(super) fn unregister_item_tree(&self, component: ItemTreeRef) {
        if let Ok(mut adapter) = self.adapter.try_borrow_mut() {
            adapter.unregister_item_tree(component);
        }
    }
}

/// `accesskit_windows`'s adapter, which answers `WM_GETOBJECT`.
///
/// Raising the events of an update sends `WM_GETOBJECT`, so this stays apart from the
/// `AccessKitAdapter`, which is borrowed while it updates the tree.
struct Platform {
    hwnd: HWND,
    adapter: RefCell<accesskit_windows::Adapter>,
    activation_handler: RefCell<ActivationHandler>,
    requests: mpsc::Receiver<ActionRequest>,
}

impl Platform {
    fn new(hwnd: HWND) -> Self {
        let (sender, requests) = mpsc::channel();
        let action_handler = ActionHandler { hwnd: hwnd.0 as usize, sender };
        Self {
            hwnd,
            adapter: RefCell::new(accesskit_windows::Adapter::new(hwnd, false, action_handler)),
            activation_handler: RefCell::new(ActivationHandler { hwnd }),
            requests,
        }
    }

    fn update_if_active(&self, update_factory: impl FnOnce() -> TreeUpdate) {
        let mut built = None;
        let events = self.adapter.borrow_mut().update_if_active(|| {
            let update = update_factory();
            built = Some((update.nodes.len(), update.focus));
            update
        });
        trace(format_args!(
            "{:?} update: {} events, {}",
            self.hwnd.0,
            if events.is_some() { "raises" } else { "no" },
            match built {
                Some((nodes, focus)) => format!("{nodes} nodes, focus {}", focus.0),
                None => "inactive".into(),
            }
        ));
        if let Some(events) = events {
            events.raise();
        }
    }

    fn update_window_focus_state(&self, is_focused: bool) {
        let events = self.adapter.borrow_mut().update_window_focus_state(is_focused);
        trace(format_args!(
            "{:?} focused {is_focused}: {} events",
            self.hwnd.0,
            if events.is_some() { "raises" } else { "no" }
        ));
        if let Some(events) = events {
            events.raise();
        }
    }

    fn handle_wm_getobject(&self, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        // A nested `WM_GETOBJECT` arrives while an update raises its events.
        let Ok(mut adapter) = self.adapter.try_borrow_mut() else {
            trace(format_args!("{:?} WM_GETOBJECT {} while updating", self.hwnd.0, lp.0 as i32));
            return None;
        };
        let mut activation_handler = self.activation_handler.borrow_mut();
        let result = adapter.handle_wm_getobject(wp, lp, &mut *activation_handler);
        drop(adapter);
        drop(activation_handler);
        trace(format_args!(
            "{:?} WM_GETOBJECT {}: {}",
            self.hwnd.0,
            lp.0 as i32,
            if result.is_some() { "provider" } else { "none" }
        ));
        // Returning the provider can send `WM_GETOBJECT` again.
        result.map(Into::into)
    }
}

/// Prints when UI Automation asks a window for its tree and when AccessKit raises events, when
/// `SLINT_FORK_TRACE_A11Y` is set.
fn trace(args: core::fmt::Arguments) {
    static START: std::sync::OnceLock<Option<std::time::Instant>> = std::sync::OnceLock::new();
    let start = START.get_or_init(|| {
        std::env::var_os("SLINT_FORK_TRACE_A11Y").is_some().then(std::time::Instant::now)
    });
    if let Some(start) = start {
        eprintln!("[a11y {:>6}ms] {args}", start.elapsed().as_millis());
    }
}

/// Asks the window for the tree, which it builds once it handles the posted message, as
/// upstream's winit backend does.
struct ActivationHandler {
    hwnd: HWND,
}

impl accesskit::ActivationHandler for ActivationHandler {
    fn request_initial_tree(&mut self) -> Option<TreeUpdate> {
        let _ = unsafe {
            PostMessageW(Some(self.hwnd), WM_APP_ACCESSKIT, WPARAM(INITIAL_TREE), LPARAM(0))
        };
        None
    }
}

/// Hands the requests to the window. UI Automation calls it on threads of its own.
struct ActionHandler {
    hwnd: usize,
    sender: mpsc::Sender<ActionRequest>,
}

impl accesskit::ActionHandler for ActionHandler {
    fn do_action(&mut self, request: ActionRequest) {
        if self.sender.send(request).is_ok() {
            let hwnd = HWND(self.hwnd as *mut _);
            let _ =
                unsafe { PostMessageW(Some(hwnd), WM_APP_ACCESSKIT, WPARAM(ACTIONS), LPARAM(0)) };
        }
    }
}

/// Runs `f` with the AccessKit adapter of the window, if it has one.
fn with_access_kit_adapter(
    window_adapter_weak: &Weak<Win32WindowAdapter>,
    f: impl FnOnce(&RefCell<AccessKitAdapter>),
) {
    if let Some(window_adapter) = window_adapter_weak.upgrade()
        && let Some(access_kit) = window_adapter.accesskit.get()
    {
        f(&access_kit.adapter);
    }
}

/// Tells AccessKit which window of a top-level window and its popups has the focus: the one
/// that gets the keys, as long as the top-level window is focused. A popup never gets the
/// keyboard focus of Windows, and the host hands it the keys of its owner instead.
pub(super) fn sync_keyboard_focus(window_adapter: &Win32WindowAdapter) {
    let Some(top_level) = window_adapter.top_level() else { return };
    let focused = top_level.accesskit.get().is_some_and(|a| a.window_focused.get());
    let key_window = top_level.key_window();
    let mut pending = Vec::from([top_level]);
    while let Some(window_adapter) = pending.pop() {
        pending.extend(super::popup::native_popups(&window_adapter));
        let Some(access_kit) = window_adapter.accesskit.get() else { continue };
        let has_focus = focused && core::ptr::addr_eq(Rc::as_ptr(&key_window), &*window_adapter);
        if access_kit.has_focus.replace(has_focus) == has_focus {
            continue;
        }
        if has_focus {
            // The tree names the focus item only for the window that gets the keys: name it
            // before AccessKit raises the focus event.
            access_kit.handle_focus_item_change();
        }
        access_kit.platform.update_window_focus_state(has_focus);
    }
}

/// [`sync_keyboard_focus`] once the core is done with the change at hand, such as showing or
/// closing a popup.
pub(super) fn sync_keyboard_focus_later(window_adapter: &Win32WindowAdapter) {
    if window_adapter.hwnd.get().is_none() {
        return;
    }
    let Some(top_level) = window_adapter.top_level() else { return };
    let top_level = Rc::downgrade(&top_level);
    crate::timers::Timer::single_shot(Default::default(), move || {
        if let Some(top_level) = top_level.upgrade() {
            sync_keyboard_focus(&top_level);
        }
    });
}

/// Where the window's root item is in its client area, which a popup's margin moves.
fn window_origin(window_adapter: &Win32WindowAdapter) -> LogicalPoint {
    let margin = window_adapter.margin();
    let scale = window_adapter.window.scale_factor();
    LogicalPoint::new(margin.left as f32 / scale, margin.top as f32 / scale)
}

/// The AccessKit adapter tries to keep the given window adapter's item tree in sync with accesskit's node tree.
///
/// The entire item tree is mapped to accesskit's node tree. Any changes to an individual accessible item results
/// in an access kit tree update with just changed nodes. Any changes in the tree structure result in a complete
/// tree rebuild. This could be implemented more efficiently, but that isn't essential; AccessKit will avoid firing
/// gratuitous events for full-tree updates as long as the node IDs are stable.
///
/// For unix it's necessary to inform accesskit about any changes to the position or size of the window, hence
/// the `on_event` function that needs calling.
///
/// Similarly, when the window adapter is informed about a focus change, handle_focus_change must be called.
/// Finally, when a component is destroyed, `unregister_item_tree` must be called, which rebuilds the entire
/// tree at the moment.
///
/// If we wanted to move this to corelib, `on_event` gets replaced with listening to the events sent from the
/// platform adapter to the slint::Window. `handle_focus_change` is already internal to WindowInner, as well
/// as `component_destroyed`. The `WindowInner` would own this `AccessKit`.
pub struct AccessKitAdapter {
    inner: Rc<Platform>,
    window_adapter_weak: Weak<Win32WindowAdapter>,
    nodes: NodeCollection,
    global_property_tracker: Pin<Box<AccessibilityPropertyTracker>>,
    pending_update: bool,
    initial_tree_sent: bool,
}

impl AccessKitAdapter {
    fn new(window_adapter_weak: Weak<Win32WindowAdapter>, inner: Rc<Platform>) -> Self {
        Self {
            inner,
            window_adapter_weak: window_adapter_weak.clone(),
            nodes: NodeCollection {
                next_component_id: 1,
                free_component_ids: Default::default(),
                root_node_id: NodeId(0),
                components_by_id: Default::default(),
                component_ids: Default::default(),
                all_nodes: Default::default(),
                focused_node_tracker: Box::pin(PropertyTracker::new_with_dirty_handler(
                    DelegateFocusPropertyTracker {
                        window_adapter_weak: window_adapter_weak.clone(),
                    },
                )),
                text_state: Default::default(),
            },
            global_property_tracker: Box::pin(PropertyTracker::new_with_dirty_handler(
                AccessibilityPropertyDirtyHandler {
                    window_adapter_weak: window_adapter_weak.clone(),
                },
            )),
            pending_update: false,
            initial_tree_sent: false,
        }
    }

    fn window_focus_changed(&mut self) {
        self.global_property_tracker.set_dirty();
        self.invoke_later(|self_cell, _| self_cell.borrow_mut().rebuild_tree_of_dirty_nodes());
    }

    // UI Automation doesn't tell when it stops, so the tree stays active once requested.
    fn initial_tree_requested(&mut self) {
        self.inner.update_if_active(|| {
            self.nodes
                .build_new_tree(&self.window_adapter_weak, self.global_property_tracker.as_ref())
        });
        self.initial_tree_sent = true;
    }

    pub fn handle_focus_item_change(&mut self) {
        // Ignore focus changes until an initial tree was sent, to avoid sending `tree: None`.
        if !self.initial_tree_sent {
            return;
        }
        // Don't send a tree update now with an empty tree/node list when we know that the structure
        // if the tree has changed. It might be that the focus node is not known yet to AccessKit.
        // The pending update will take care of setting the focus node.
        if self.pending_update {
            return;
        }
        self.inner.update_if_active(|| TreeUpdate {
            nodes: Vec::new(),
            tree: None,
            tree_id: TreeId::ROOT,
            focus: self.nodes.focus_node(&self.window_adapter_weak),
        })
    }

    fn handle_request(&self, request: ActionRequest) -> Option<DeferredAccessKitAction> {
        let a = match request.action {
            Action::Click => AccessibilityAction::Default,
            Action::Focus => {
                return self
                    .nodes
                    .item_rc_for_node_id(request.target_node)
                    .map(DeferredAccessKitAction::SetFocus);
            }
            Action::Decrement => AccessibilityAction::Decrement,
            Action::Increment => AccessibilityAction::Increment,
            Action::ReplaceSelectedText => {
                let Some(accesskit::ActionData::Value(v)) = request.data else { return None };
                AccessibilityAction::ReplaceSelectedText(SharedString::from(&*v))
            }
            Action::SetValue => match request.data {
                Some(accesskit::ActionData::Value(v)) => {
                    AccessibilityAction::SetValue(SharedString::from(&*v))
                }
                Some(accesskit::ActionData::NumericValue(v)) => {
                    AccessibilityAction::SetValue(crate::format!("{v}"))
                }
                _ => return None,
            },
            Action::Expand => AccessibilityAction::Expand,
            Action::SetTextSelection => {
                let Some(accesskit::ActionData::SetTextSelection(sel)) = request.data.as_ref()
                else {
                    return None;
                };
                // A TextPosition names a TextRun sub-NodeId, so decode it back to the wrapper.
                let (wrapper_parent, _) = decode_sub_node_id(sel.focus.node)?;
                if wrapper_parent != request.target_node {
                    return None;
                }
                let wrapper_item = self.nodes.item_rc_for_node_id(wrapper_parent)?;
                let window_adapter = self.window_adapter_weak.upgrade()?;
                let (inner_item_rc, text_input) = find_exposed_text_input(&wrapper_item)?;
                let state = self
                    .nodes
                    .text_state
                    .get_or_update_cache_entry_ref(&inner_item_rc, Default::default);
                let (anchor, focus) = state.decode_selection(
                    window_adapter.renderer(),
                    text_input.as_pin_ref(),
                    &inner_item_rc,
                    inner_item_rc.geometry().size,
                    &sel.anchor,
                    &sel.focus,
                )?;
                AccessibilityAction::SetSelectionOffsets(anchor as i32, focus as i32)
            }
            _ => return None,
        };
        self.nodes
            .item_rc_for_node_id(request.target_node)
            .map(|item| DeferredAccessKitAction::InvokeAccessibleAction(item, a))
    }

    pub fn reload_tree(&mut self) {
        if self.pending_update {
            return;
        }
        self.pending_update = true;

        self.invoke_later(|self_cell, win| {
            let mut self_ = self_cell.borrow_mut();
            let self_ = &mut *self_;
            self_.pending_update = false;
            self_.inner.update_if_active(|| {
                self_.nodes.build_new_tree(&win, self_.global_property_tracker.as_ref())
            })
        });
    }

    pub fn unregister_item_tree(&mut self, component: ItemTreeRef) {
        self.nodes.forget_component(ItemTreeRef::as_ptr(component));
        self.reload_tree();
    }

    fn rebuild_tree_of_dirty_nodes(&mut self) {
        if !self.global_property_tracker.is_dirty() && !self.nodes.focused_node_tracker.is_dirty() {
            return;
        }

        // It's possible that we may have been triggered by a timer, but in the meantime
        // the node tree has been emptied due to a tree structure change.
        if self.nodes.all_nodes.is_empty() {
            return;
        }

        // Don't end a tree update now with an empty tree/node list when we know that the structure
        // if the tree has changed. It might be that the focus node is not known yet to AccessKit.
        // The pending update will take care rebuilding the entire tree anyway.
        if self.pending_update {
            return;
        }

        let Some(window_adapter) = self.window_adapter_weak.upgrade() else { return };
        let window = window_adapter.window();
        let window_position = window_origin(&window_adapter);

        self.inner.update_if_active(|| {
            self.global_property_tracker.as_ref().evaluate_as_dependency_root(|| {
                let nodes_vec: Vec<(NodeId, Node)> = self
                    .nodes
                    .all_nodes
                    .iter()
                    .flat_map(|cached_node| {
                        cached_node
                            .tracker
                            .as_ref()
                            .evaluate_if_dirty(|| {
                                let scale_factor = ScaleFactor::new(window.scale_factor());
                                let Some(item) = self.nodes.item_rc_for_node_id(cached_node.id)
                                else {
                                    return Vec::new();
                                };

                                let mut node = self.nodes.build_node_without_children(
                                    &item,
                                    scale_factor,
                                    window_position,
                                );
                                node.set_children(cached_node.children.clone());

                                let mut emitted: Vec<(NodeId, Node)> = Vec::new();
                                self.nodes.try_emit_text_input_accessibility(
                                    &item,
                                    &mut node,
                                    cached_node.id,
                                    scale_factor,
                                    window_position,
                                    &mut emitted,
                                    &window_adapter,
                                );

                                let mut out = Vec::with_capacity(1 + emitted.len());
                                out.push((cached_node.id, node));
                                out.extend(emitted);
                                out
                            })
                            .unwrap_or_default()
                    })
                    .collect();

                TreeUpdate {
                    nodes: nodes_vec,
                    tree: None,
                    tree_id: TreeId::ROOT,
                    focus: self.nodes.focus_node(&self.window_adapter_weak),
                }
            })
        })
    }

    fn invoke_later(
        &self,
        callback: impl FnOnce(&std::cell::RefCell<Self>, Weak<Win32WindowAdapter>) + 'static,
    ) {
        let win = self.window_adapter_weak.clone();
        crate::timers::Timer::single_shot(Default::default(), move || {
            with_access_kit_adapter(&win.clone(), move |self_cell| callback(self_cell, win));
        });
    }
}

const NODE_ID_INDEX_BITS: u32 = 16;
const NODE_ID_INDEX_MASK: u64 = (1 << NODE_ID_INDEX_BITS) - 1; // 0xFFFF
const NODE_ID_COMPONENT_BITS: u32 = 22;
const NODE_ID_COMPONENT_MASK: u64 = (1 << NODE_ID_COMPONENT_BITS) - 1; // 0x3FFFFF

// NodeIds for the TextRun children of a text input:
//
//   bits 38..=63 : sub-index, allocated per parent, never 0
//   bits 0..=37  : the parent NodeId, which `encode_item_node_id` fits in exactly these bits
//
// A regular NodeId leaves the sub-index bits clear, so a non-zero sub-index is what tells the two
// apart.
const NODE_ID_PARENT_BITS: u32 = NODE_ID_INDEX_BITS + NODE_ID_COMPONENT_BITS;
const NODE_ID_PARENT_MASK: u64 = (1u64 << NODE_ID_PARENT_BITS) - 1;
const NODE_ID_SUB_INDEX_BITS: u32 = u64::BITS - NODE_ID_PARENT_BITS;
const NODE_ID_SUB_INDEX_MASK: u64 = (1u64 << NODE_ID_SUB_INDEX_BITS) - 1;

fn encode_sub_node_id(parent: NodeId, sub_index: u32) -> NodeId {
    debug_assert!(sub_index >= 1, "sub_index 0 would collide with the parent NodeId");
    debug_assert!(
        (sub_index as u64) <= NODE_ID_SUB_INDEX_MASK,
        "sub_index exceeds {NODE_ID_SUB_INDEX_BITS} bits"
    );
    debug_assert_eq!(
        parent.0 & !NODE_ID_PARENT_MASK,
        0,
        "parent NodeId occupies more than {NODE_ID_PARENT_BITS} bits"
    );
    NodeId(((sub_index as u64) << NODE_ID_PARENT_BITS) | (parent.0 & NODE_ID_PARENT_MASK))
}

fn decode_sub_node_id(id: NodeId) -> Option<(NodeId, u32)> {
    let sub_index = (id.0 >> NODE_ID_PARENT_BITS) as u32;
    (sub_index != 0).then_some((NodeId(id.0 & NODE_ID_PARENT_MASK), sub_index))
}

/// The first list item below `item` that is selected.
fn selected_list_item(item: &ItemRc) -> Option<ItemRc> {
    crate::accessibility::accessible_descendents(item).find_map(|child| {
        let selected = child.accessible_role() == crate::items::AccessibleRole::ListItem
            && child
                .accessible_string_property(AccessibleStringProperty::ItemSelected)
                .is_some_and(|x| x == "true");
        if selected { Some(child) } else { selected_list_item(&child) }
    })
}

fn is_text_input_role(role: Role) -> bool {
    matches!(
        role,
        Role::TextInput
            | Role::MultilineTextInput
            | Role::PasswordInput
            | Role::SearchInput
            | Role::NumberInput
    )
}

fn wraps_text_input(role: Role) -> bool {
    is_text_input_role(role) || role == Role::SpinButton
}

struct NodeCollection {
    next_component_id: u32,
    free_component_ids: Vec<u32>,
    components_by_id: HashMap<u32, ItemTreeWeak>,
    component_ids: HashMap<NonNull<u8>, u32>,
    all_nodes: Vec<CachedNode>,
    root_node_id: NodeId,
    focused_node_tracker: Pin<Box<PropertyTracker<false, DelegateFocusPropertyTracker>>>,
    /// Emission state, keyed by the inner `TextInput`'s `ItemRc`. Its per-entry property tracker
    /// stays empty on purpose: the state has to outlive the edits it describes, so that NodeIds
    /// stay stable and screen readers see "node updated" rather than "subtree replaced".
    text_state: crate::item_rendering::ItemCache<
        crate::textlayout::sharedparley::CachedTextInputAccessibilityState,
    >,
}

impl NodeCollection {
    fn focus_node(&mut self, window_adapter_weak: &Weak<Win32WindowAdapter>) -> NodeId {
        window_adapter_weak
            .upgrade()
            .filter(|window_adapter| window_adapter.gets_keys())
            .and_then(|window_adapter| {
                let window_inner = WindowInner::from_pub(window_adapter.window());
                window_inner
                    .focus_item
                    .borrow()
                    .upgrade()
                    .map(|focus_item| {
                        let parent = nearest_accessible_item(focus_item);
                        // The list of upstream's combo box keeps the focus on the popup's root,
                        // and tells its selection only. Narrator doesn't read the selection of a
                        // popup window, which isn't focused, so the focus follows the selection.
                        let popup_root = window_adapter.popup.is_some()
                            && parent
                                .parent_item(
                                    crate::item_tree::ParentItemTraversalMode::StopAtPopups,
                                )
                                .is_none();
                        self.focused_node_tracker
                            .as_ref()
                            .evaluate(|| {
                                crate::accessibility::accessible_focus_delegate_position(&parent)
                                    .and_then(|position| {
                                        crate::accessibility::accessible_descendents(&parent)
                                            .nth(position)
                                    })
                                    .or_else(|| {
                                        popup_root.then(|| selected_list_item(&parent)).flatten()
                                    })
                            })
                            .unwrap_or(parent)
                    })
                    .or_else(|| window_inner.try_component().map(ItemRc::new_root))
                    .map(|focus_item| self.find_node_id_by_item_rc(focus_item))
            })
            .unwrap_or(self.root_node_id)
    }

    fn item_rc_for_node_id(&self, id: NodeId) -> Option<ItemRc> {
        let component_id: u32 = ((id.0 >> NODE_ID_INDEX_BITS) & NODE_ID_COMPONENT_MASK) as _;
        let index: u32 = (id.0 & NODE_ID_INDEX_MASK) as _;
        let component = self.components_by_id.get(&component_id)?.upgrade()?;
        Some(ItemRc::new(component, index))
    }

    fn find_node_id_by_item_rc(&mut self, mut item: ItemRc) -> NodeId {
        item = nearest_accessible_item(item);

        self.encode_item_node_id(&item)
    }

    fn alloc_component_id(&mut self) -> u32 {
        self.free_component_ids.pop().unwrap_or_else(|| {
            let id = self.next_component_id;
            self.next_component_id += 1;
            id
        })
    }

    fn forget_component(&mut self, component_ptr: NonNull<u8>) {
        if let Some(component_id) = self.component_ids.remove(&component_ptr) {
            self.components_by_id.remove(&component_id);
            self.free_component_ids.push(component_id);
        }
        self.text_state.component_destroyed_at(component_ptr);
    }

    /// Forgets the components that were destroyed without being unregistered: their
    /// `unregister_item_tree` is skipped when it finds the adapter borrowed, such as while
    /// building the tree destroys repeated components (#13670).
    fn forget_destroyed_components(&mut self) {
        let destroyed = self
            .component_ids
            .iter()
            .filter(|(_, id)| self.components_by_id.get(id).is_none_or(|c| c.upgrade().is_none()))
            .map(|(component_ptr, _)| *component_ptr)
            .collect::<Vec<_>>();
        for component_ptr in destroyed {
            self.forget_component(component_ptr);
        }
    }

    fn encode_item_node_id(&mut self, item: &ItemRc) -> NodeId {
        let component = item.item_tree();
        let component_ptr = ItemTreeRef::as_ptr(ItemTreeRc::borrow(component));
        let component_id = match self.component_ids.get(&component_ptr) {
            Some(&component_id) => component_id,
            None => {
                let component_id = self.alloc_component_id();
                self.component_ids.insert(component_ptr, component_id);
                self.components_by_id.insert(component_id, ItemTreeRc::downgrade(component));
                component_id
            }
        };

        debug_assert!(
            (component_id as u64) <= NODE_ID_COMPONENT_MASK,
            "component_id exceeds {NODE_ID_COMPONENT_BITS} bits"
        );
        let index = item.index();
        NodeId((component_id as u64) << NODE_ID_INDEX_BITS | (index as u64 & NODE_ID_INDEX_MASK))
    }

    fn build_node_for_item_recursively(
        &mut self,
        item: ItemRc,
        nodes: &mut Vec<(NodeId, Node)>,
        popups: &[AccessiblePopup],
        scale_factor: ScaleFactor,
        window_position: LogicalPoint,
        window_adapter: &std::rc::Rc<Win32WindowAdapter>,
    ) -> NodeId {
        let id = self.encode_item_node_id(&item);

        let popup_children = popups
            .iter()
            .filter_map(|popup| {
                if popup.parent_node != id {
                    return None;
                }

                let popup_item = ItemRc::new_root(popup.component.clone());
                Some(self.build_node_for_item_recursively(
                    popup_item,
                    nodes,
                    popups,
                    scale_factor,
                    popup.location,
                    window_adapter,
                ))
            })
            .collect::<Vec<_>>();

        let descendant_children = crate::accessibility::accessible_descendents(&item)
            .map(|child| {
                self.build_node_for_item_recursively(
                    child,
                    nodes,
                    popups,
                    scale_factor,
                    window_position,
                    window_adapter,
                )
            })
            .chain(popup_children)
            .collect::<Vec<NodeId>>();

        let tracker = Box::pin(PropertyTracker::default());
        // One tracker for both the wrapper attributes and the text emission, so that either
        // going dirty rebuilds the node.
        let (node, text_run_nodes) = {
            let mut text_run_nodes: Vec<(NodeId, Node)> = Vec::new();
            let node = tracker.as_ref().evaluate(|| {
                let mut n = self.build_node_without_children(&item, scale_factor, window_position);
                n.set_children(descendant_children.clone());
                self.try_emit_text_input_accessibility(
                    &item,
                    &mut n,
                    id,
                    scale_factor,
                    window_position,
                    &mut text_run_nodes,
                    window_adapter,
                );
                n
            });
            (node, text_run_nodes)
        };

        // Only the regular descendants: every emit pushes the TextRun children again, and
        // `accesskit_consumer` rejects a child that appears twice.
        self.all_nodes.push(CachedNode { id, children: descendant_children, tracker });

        nodes.push((id, node));
        nodes.extend(text_run_nodes);

        id
    }

    /// Emits the TextRun children of a text input, and the value and selection on `wrapper_node`.
    fn try_emit_text_input_accessibility(
        &self,
        item: &ItemRc,
        wrapper_node: &mut Node,
        wrapper_id: NodeId,
        scale_factor: ScaleFactor,
        window_position: LogicalPoint,
        text_run_nodes: &mut Vec<(NodeId, Node)>,
        window_adapter: &std::rc::Rc<Win32WindowAdapter>,
    ) {
        if !wraps_text_input(wrapper_node.role()) {
            return;
        }
        let Some((inner_item_rc, text_input)) = find_exposed_text_input(item) else {
            return;
        };
        let mut state =
            self.text_state.get_or_update_cache_entry_ref(&inner_item_rc, Default::default);

        // The inner `TextInput`'s geometry: a `LineEdit` insets it for its border, so measuring
        // from the wrapper's origin would place every TextRun off by the padding.
        let inner_geometry = inner_item_rc.geometry();
        let inner_absolute_origin =
            inner_item_rc.map_to_window(inner_geometry.origin) + window_position.to_vector();
        let physical_origin = (inner_absolute_origin * scale_factor).cast::<f64>();

        // Borrows the font context itself, so we must not be holding it here.
        let emitted_runs = state.emit(
            window_adapter.renderer(),
            text_input.as_pin_ref(),
            &inner_item_rc,
            inner_geometry.size,
            text_run_nodes,
            wrapper_node,
            wrapper_id,
            (physical_origin.x, physical_origin.y),
            encode_sub_node_id,
        );

        if emitted_runs
            && item
                .supported_accessibility_actions()
                .contains(SupportedAccessibilityAction::SetSelectionOffsets)
        {
            wrapper_node.add_action(Action::SetTextSelection);
        }
    }

    fn tree_info(&self, root: NodeId) -> TreeInfo {
        let mut tree = TreeInfo::new(root);
        tree.toolkit_name = Some("Slint".into());
        tree.toolkit_version = Some(env!("CARGO_PKG_VERSION").into());
        tree
    }

    fn build_new_tree(
        &mut self,
        window_adapter_weak: &Weak<Win32WindowAdapter>,
        property_tracker: Pin<&AccessibilityPropertyTracker>,
    ) -> TreeUpdate {
        let Some(window_adapter) = window_adapter_weak.upgrade() else {
            return TreeUpdate {
                nodes: Default::default(),
                tree: Default::default(),
                tree_id: TreeId::ROOT,
                focus: self.root_node_id,
            };
        };
        let window = window_adapter.window();
        let window_inner = crate::window::WindowInner::from_pub(window);
        window_inner.ensure_tree_instantiated();

        let Some(component) = window_inner.try_component() else {
            return TreeUpdate {
                nodes: Default::default(),
                tree: Default::default(),
                tree_id: TreeId::ROOT,
                focus: self.root_node_id,
            };
        };
        let root_item = ItemRc::new_root(component);

        let popups = window_inner
            .active_popups()
            .iter()
            .filter_map(|popup| {
                let PopupWindowLocation::ChildWindow(location) = popup.location else {
                    return None;
                };

                let parent_item = nearest_accessible_item(popup.parent_item.upgrade()?);
                let parent_node = self.encode_item_node_id(if parent_item.is_accessible() {
                    &parent_item
                } else {
                    &root_item
                });

                Some(AccessiblePopup { location, parent_node, component: popup.component.clone() })
            })
            .collect::<Vec<_>>();

        self.all_nodes.clear();
        let mut nodes = Vec::new();

        let root_id = property_tracker.evaluate_as_dependency_root(|| {
            self.build_node_for_item_recursively(
                root_item,
                &mut nodes,
                &popups,
                ScaleFactor::new(window.scale_factor()),
                window_origin(&window_adapter),
                &window_adapter,
            )
        });
        self.root_node_id = root_id;
        self.forget_destroyed_components();

        TreeUpdate {
            nodes,
            tree: Some(self.tree_info(root_id)),
            tree_id: TreeId::ROOT,
            focus: self.focus_node(window_adapter_weak),
        }
    }

    fn build_node_without_children(
        &self,
        item: &ItemRc,
        scale_factor: ScaleFactor,
        window_position: LogicalPoint,
    ) -> Node {
        let is_checkable = item
            .accessible_string_property(AccessibleStringProperty::Checkable)
            .is_some_and(|x| x == "true");

        let (role, label) = if let Some(window_item) = item.downcast::<WindowItem>() {
            (Role::Window, Some(window_item.as_pin_ref().title().to_string()))
        } else {
            (
                match item.accessible_role() {
                    crate::items::AccessibleRole::None => Role::Unknown,
                    crate::items::AccessibleRole::Button => Role::Button,
                    crate::items::AccessibleRole::Checkbox => Role::CheckBox,
                    crate::items::AccessibleRole::Combobox => Role::ComboBox,
                    crate::items::AccessibleRole::Groupbox => Role::Group,
                    crate::items::AccessibleRole::List => Role::ListBox,
                    crate::items::AccessibleRole::Slider => Role::Slider,
                    crate::items::AccessibleRole::Spinbox => Role::SpinButton,
                    crate::items::AccessibleRole::Tab => Role::Tab,
                    crate::items::AccessibleRole::TabList => Role::TabList,
                    crate::items::AccessibleRole::TabPanel => Role::TabPanel,
                    crate::items::AccessibleRole::Text => Role::Label,
                    crate::items::AccessibleRole::Table => Role::Table,
                    crate::items::AccessibleRole::Tree => Role::Tree,
                    crate::items::AccessibleRole::TextInput => {
                        if let Some(text_input) = crate::accessibility::find_text_input(item) {
                            if !text_input.single_line.get_internal() {
                                Role::MultilineTextInput
                            } else {
                                match text_input.input_type.get_internal() {
                                    crate::items::InputType::Decimal
                                    | crate::items::InputType::Number => Role::NumberInput,
                                    crate::items::InputType::Password => Role::PasswordInput,
                                    crate::items::InputType::Search => Role::SearchInput,
                                    crate::items::InputType::Text => Role::TextInput,
                                }
                            }
                        } else {
                            Role::TextInput
                        }
                    }
                    crate::items::AccessibleRole::ProgressIndicator => Role::ProgressIndicator,
                    crate::items::AccessibleRole::Switch => Role::Switch,
                    crate::items::AccessibleRole::ListItem => Role::ListBoxOption,
                    crate::items::AccessibleRole::Image => Role::Image,
                    crate::items::AccessibleRole::RadioButton => Role::RadioButton,
                    crate::items::AccessibleRole::RadioGroup => Role::RadioGroup,
                    crate::items::AccessibleRole::WindowTitleBar => Role::TitleBar,
                    crate::items::AccessibleRole::Banner => Role::Banner,
                    crate::items::AccessibleRole::Complementary => Role::Complementary,
                    crate::items::AccessibleRole::ContentInfo => Role::ContentInfo,
                    crate::items::AccessibleRole::Form => Role::Form,
                    crate::items::AccessibleRole::Main => Role::Main,
                    crate::items::AccessibleRole::Navigation => Role::Navigation,
                    crate::items::AccessibleRole::Region => Role::Region,
                    crate::items::AccessibleRole::Search => Role::Search,
                },
                item.accessible_string_property(
                    crate::accessibility::AccessibleStringProperty::Label,
                )
                .map(|x| x.to_string()),
            )
        };

        let mut node = Node::new(role);

        if let Some(label) = label {
            if role == Role::Label {
                node.set_value(label);
            } else {
                node.set_label(label);
            }
        }

        if item
            .accessible_string_property(AccessibleStringProperty::Enabled)
            .is_some_and(|x| x != "true")
        {
            node.set_disabled();
        }

        if !item.is_visible() {
            node.set_hidden();
        }

        if item.borrow().as_ref().clips_children() {
            node.set_clips_children();
        }

        let geometry = item.geometry();
        let absolute_origin = item.map_to_window(geometry.origin) + window_position.to_vector();
        let physical_origin = (absolute_origin * scale_factor).cast::<f64>();
        let physical_size = (geometry.size * scale_factor).cast::<f64>();
        node.set_bounds(accesskit::Rect {
            x0: physical_origin.x,
            y0: physical_origin.y,
            x1: physical_origin.x + physical_size.width,
            y1: physical_origin.y + physical_size.height,
        });

        let is_checked = is_checkable
            && item
                .accessible_string_property(AccessibleStringProperty::Checked)
                .is_some_and(|x| x == "true");
        if is_checkable {
            node.set_toggled(if is_checked { Toggled::True } else { Toggled::False });
        }

        if let Some(description) =
            item.accessible_string_property(AccessibleStringProperty::Description)
        {
            node.set_description(description.to_string());
        }

        if let Some(id) = item.accessible_string_property(AccessibleStringProperty::Id) {
            node.set_author_id(id.to_string());
        }

        if item
            .accessible_string_property(AccessibleStringProperty::Expandable)
            .is_some_and(|x| x == "true")
        {
            node.set_expanded(
                item.accessible_string_property(AccessibleStringProperty::Expanded)
                    .is_some_and(|x| x == "true"),
            );
        }

        if matches!(
            role,
            Role::Button
                | Role::CheckBox
                | Role::ComboBox
                | Role::ListBoxOption
                | Role::MultilineTextInput
                | Role::NumberInput
                | Role::PasswordInput
                | Role::SearchInput
                | Role::Slider
                | Role::SpinButton
                | Role::Tab
                | Role::TextInput
        ) {
            node.add_action(Action::Focus);
        }

        if let Some(min) = item
            .accessible_string_property(AccessibleStringProperty::ValueMinimum)
            .and_then(|min| min.parse().ok())
        {
            node.set_min_numeric_value(min);
        }
        if let Some(max) = item
            .accessible_string_property(AccessibleStringProperty::ValueMaximum)
            .and_then(|max| max.parse().ok())
        {
            node.set_max_numeric_value(max);
        }
        if let Some(step) = item
            .accessible_string_property(AccessibleStringProperty::ValueStep)
            .and_then(|step| step.parse().ok())
        {
            node.set_numeric_value_step(step);
        }

        if let Some(value) = item.accessible_string_property(AccessibleStringProperty::Value) {
            match value.parse() {
                Ok(numeric) if role == Role::SpinButton => {
                    node.set_numeric_value(numeric);
                    node.set_value(value.to_string());
                }
                Ok(numeric) if !is_text_input_role(role) => node.set_numeric_value(numeric),
                _ => node.set_value(value.to_string()),
            }
        }

        if let Some(placeholder) =
            item.accessible_string_property(AccessibleStringProperty::PlaceholderText)
        {
            node.set_placeholder(placeholder.to_string());
        }

        if item
            .accessible_string_property(AccessibleStringProperty::ReadOnly)
            .is_some_and(|x| x == "true")
        {
            node.set_read_only();
        }

        if let Some(orientation) = item
            .accessible_string_property(AccessibleStringProperty::Orientation)
            .and_then(|s| s.parse::<crate::items::Orientation>().ok())
        {
            node.set_orientation(match orientation {
                crate::items::Orientation::Horizontal => Orientation::Horizontal,
                crate::items::Orientation::Vertical => Orientation::Vertical,
            });
        }

        if let Some(live) = item
            .accessible_string_property(AccessibleStringProperty::LiveRegion)
            .and_then(|s| s.parse::<crate::items::AccessibleLiveness>().ok())
        {
            node.set_live(match live {
                crate::items::AccessibleLiveness::Off => Live::Off,
                crate::items::AccessibleLiveness::Polite => Live::Polite,
                crate::items::AccessibleLiveness::Assertive => Live::Assertive,
            });
        }

        if item
            .accessible_string_property(AccessibleStringProperty::ItemSelectable)
            .is_some_and(|x| x == "true")
        {
            node.set_selected(
                item.accessible_string_property(AccessibleStringProperty::ItemSelected)
                    .is_some_and(|x| x == "true"),
            );
        }

        if let Some(position_in_set) = item
            .accessible_string_property(AccessibleStringProperty::ItemIndex)
            .and_then(|s| s.parse::<usize>().ok())
        {
            node.set_position_in_set(position_in_set);
        }
        if let Some(size_of_set) = item
            .accessible_string_property(AccessibleStringProperty::ItemCount)
            .and_then(|s| s.parse::<usize>().ok())
        {
            node.set_size_of_set(size_of_set);
        }

        let supported = item.supported_accessibility_actions();
        if supported.contains(SupportedAccessibilityAction::Default) {
            node.add_action(accesskit::Action::Click);
        }
        if supported.contains(SupportedAccessibilityAction::Decrement) {
            node.add_action(accesskit::Action::Decrement);
        }
        if supported.contains(SupportedAccessibilityAction::Increment) {
            node.add_action(accesskit::Action::Increment);
        }
        if supported.contains(SupportedAccessibilityAction::SetValue) {
            node.add_action(accesskit::Action::SetValue);
        }
        if supported.contains(SupportedAccessibilityAction::ReplaceSelectedText) {
            node.add_action(accesskit::Action::ReplaceSelectedText);
        }
        if supported.contains(SupportedAccessibilityAction::Expand) {
            node.add_action(accesskit::Action::Expand);
        }

        node
    }
}

type AccessibilityPropertyTracker = PropertyTracker<true, AccessibilityPropertyDirtyHandler>;

struct AccessibilityPropertyDirtyHandler {
    window_adapter_weak: Weak<Win32WindowAdapter>,
}

impl crate::properties::PropertyDirtyHandler for AccessibilityPropertyDirtyHandler {
    fn notify(self: Pin<&Self>) {
        let win = self.window_adapter_weak.clone();
        crate::timers::Timer::single_shot(Default::default(), move || {
            with_access_kit_adapter(&win, |self_cell| {
                self_cell.borrow_mut().rebuild_tree_of_dirty_nodes();
            });
        })
    }
}

struct DelegateFocusPropertyTracker {
    window_adapter_weak: Weak<Win32WindowAdapter>,
}

impl crate::properties::PropertyDirtyHandler for DelegateFocusPropertyTracker {
    fn notify(self: Pin<&Self>) {
        let win = self.window_adapter_weak.clone();
        crate::timers::Timer::single_shot(Default::default(), move || {
            with_access_kit_adapter(&win, |self_cell| {
                self_cell.borrow_mut().handle_focus_item_change();
            });
        })
    }
}

struct CachedNode {
    id: NodeId,
    children: Vec<NodeId>,
    tracker: Pin<Box<PropertyTracker>>,
}

pub enum DeferredAccessKitAction {
    SetFocus(ItemRc),
    InvokeAccessibleAction(ItemRc, AccessibilityAction),
}

impl DeferredAccessKitAction {
    pub fn invoke(&self, window: &Window) {
        match self {
            DeferredAccessKitAction::SetFocus(item) => {
                // pretend this event was caused by a mouse for compatibility purposes
                WindowInner::from_pub(window).set_focus_item(item, true, FocusReason::PointerClick);
            }
            DeferredAccessKitAction::InvokeAccessibleAction(item, accessibility_action) => {
                item.accessible_action(accessibility_action);
            }
        }
    }
}

struct AccessiblePopup {
    location: LogicalPoint,
    parent_node: NodeId,
    component: ItemTreeRc,
}
