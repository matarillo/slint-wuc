// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

//! Hand-written item trees in the shape of generated code, and what tests need to drive them:
//! a headless window ([`Headless`]) and pixel capture ([`capture`]).
//!
//! Each fixture is one struct holding its item structs and properties, with `FieldOffsets`, an
//! item array, `ItemTreeVTable_static!`, and a `visit_children_item` that walks its static node
//! table. The runtime can't tell them from compiled `.slint` components.

pub mod capture;
pub mod headless;
pub mod nested_rects;
pub mod resize_follower;
mod support;

pub use headless::Headless;
pub use nested_rects::NestedRects;
pub use resize_follower::ResizeFollower;
pub use support::rect;
