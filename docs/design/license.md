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

## New Files

New files are under **MIT OR Apache-2.0**, the owner's choice of the most permissive terms.
A permissive license combines with all three upstream licenses, so users of the fork keep the same three choices they have with upstream.

- A file written for the fork starts with `// SPDX-License-Identifier: MIT OR Apache-2.0` (`#` in TOML).
  The license texts are in `LICENSES/MIT.txt` and `LICENSES/Apache-2.0.txt`.
- A file copied from upstream, or adapted from upstream code, keeps upstream's header and triple license, however much it changes.
  So do new files in a crate copied from upstream when they carry upstream's header already, such as `tests/driver`.
- A crate's `license` field states the license of its files: `tests/probe` and the spikes are new throughout.
  Crates that mix both, such as `crates/core` and `tests/fixtures`, keep the workspace's triple license, which covers the whole.
