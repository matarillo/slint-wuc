// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

//! Kept for the path of [`CachedRenderingData`], which the C++ API and the interpreter name.

pub use crate::visual_tree::CachedRenderingData;

/// What `RendererSealed::mark_dirty_region` takes. The compositor tracks damage itself, so the
/// region is only accepted and ignored.
pub type DirtyRegion = crate::lengths::LogicalRect;
