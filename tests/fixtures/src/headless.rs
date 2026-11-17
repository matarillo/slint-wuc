// SPDX-License-Identifier: MIT OR Apache-2.0

//! A window without an HWND, set up through the real context and window path, for tests.

use i_slint_core::SlintContext;
use i_slint_core::api::{PhysicalSize, Window};
use i_slint_core::host::{Platform, Win32WindowAdapter};
use i_slint_core::item_tree::ItemTreeRc;
use i_slint_core::window::{WindowAdapterRc, WindowInner};
use std::rc::Rc;
use windows::UI::Composition::{Compositor, ContainerVisual};

/// A context on a headless platform, with one detached window.
///
/// Sync passes run when a test asks for them, so a test sees each step: change a property,
/// then [`Self::sync`] or [`Self::dump`].
pub struct Headless {
    // Drop order: the window goes before the context that owns its platform.
    host: Rc<Win32WindowAdapter>,
    adapter: WindowAdapterRc,
    _ctx: SlintContext,
}

impl Headless {
    /// A window of `size` physical pixels, at scale factor 1.
    pub fn new(size: PhysicalSize) -> Self {
        let platform = Platform::new_headless().expect("a headless platform");
        let ctx = SlintContext::new(Box::new(platform));
        let adapter = ctx.create_window_adapter().expect("a detached window");
        adapter.window().set_size(size);
        let host = Win32WindowAdapter::from_window(adapter.window())
            .expect("the window belongs to the Win32 host");
        Self { host, adapter, _ctx: ctx }
    }

    pub fn adapter(&self) -> &WindowAdapterRc {
        &self.adapter
    }

    pub fn window(&self) -> &Window {
        self.adapter.window()
    }

    /// Makes `component` the window's component and shows the window, which runs the first
    /// sync pass.
    pub fn show(&self, component: &ItemTreeRc) {
        self.set_component(component);
        self.window().show().expect("show");
    }

    /// Replaces the window's component without showing the window again.
    pub fn set_component(&self, component: &ItemTreeRc) {
        WindowInner::from_pub(self.window()).set_component(component);
    }

    pub fn hide(&self) {
        self.window().hide().expect("hide");
    }

    /// Runs the sync pass if a change asked for one.
    pub fn sync(&self) {
        self.host.sync_now();
    }

    /// Syncs, then dumps the window's visuals; see `VisualTree::dump` for the format.
    pub fn dump(&self) -> String {
        self.sync();
        self.host.dump_visuals()
    }

    /// The number of visuals the window keeps for items, shown or not.
    pub fn node_count(&self) -> usize {
        self.host.node_count()
    }

    /// The visual the window shows. It has no size of its own; set one before capturing it.
    pub fn root_visual(&self) -> ContainerVisual {
        self.host.root_visual()
    }

    pub fn compositor(&self) -> Compositor {
        self.root_visual().Compositor().expect("the compositor of the root visual")
    }
}

impl Drop for Headless {
    fn drop(&mut self) {
        // A shown window keeps its component alive, and the component keeps the window's
        // adapter alive: hiding breaks that cycle.
        let _ = self.window().hide();
    }
}

/// Lines of `dump` as a vector, for comparing two dumps line by line.
pub fn lines(dump: &str) -> Vec<&str> {
    dump.lines().collect()
}

/// The indexes of the lines that differ between two dumps of the same shape.
pub fn changed_lines(before: &str, after: &str) -> Vec<usize> {
    let (before, after) = (lines(before), lines(after));
    assert_eq!(before.len(), after.len(), "the dumps differ in shape:\n{before:#?}\n{after:#?}");
    (0..before.len()).filter(|i| before[*i] != after[*i]).collect()
}
