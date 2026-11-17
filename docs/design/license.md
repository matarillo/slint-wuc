# Licensing of the Fork

This is a reading of Slint 1.19.0's license files, not legal advice.

## What Upstream Allows

Slint's runtime is available under GPL-3.0-only, the Slint Royalty-free License 2.0, or the commercial Slint Software License 3.0, at the user's choice ([LICENSE.md](../../LICENSE.md)).

- The Royalty-free License allows modifying Slint and distributing it, modified or not, **only as part of a desktop, mobile, or web application**, with attribution (`AboutSlint` widget or a badge on the download page).
- It forbids distributing Slint "alone and without integration into an Application", and forbids applications that expose Slint's APIs.
  For distributing Slint on its own, it points to the GPLv3.
- None of the licenses allow removing or altering license and copyright notices.

## Consequences for This Repository

- Files copied from upstream keep their SPDX headers and the triple license unchanged.
- Publishing this fork on its own (a public repository, crates.io) is distribution "alone", so it's covered only by the GPLv3.
  An application that embeds the fork can still use it under the Royalty-free or commercial license, since those licenses allow modified versions.
- Keeping the repository private until it's integrated into an application avoids the question for now.

## Decision Needed

The license of the fork's **new** files (`host/`, `composition/`, `visual_tree/`, `text/`, spikes) is the owner's choice.
Recommendation: MIT, or MIT OR Apache-2.0.
A permissive license combines with all three upstream licenses, so users of the fork keep the same three choices they have with upstream.
New files are marked with their own SPDX header, for example `// SPDX-License-Identifier: MIT`.
