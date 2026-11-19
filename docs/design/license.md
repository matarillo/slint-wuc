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
  The repository is published that way, as `matarillo/slint-wuc` (2026-11-19).
- An application that embeds the fork can still use it under the Royalty-free License, which allows distributing Slint modified as part of an application.
  That this holds for a modified version that a third party distributes is our reading; SixtyFPS hasn't confirmed it.
- The commercial license covers "the versions of Software that were made available to you under the Paid License Plan"; whether that includes the fork is unknown.
- `LICENSE.md` starts with the fork's own preface and keeps upstream's text after it, unchanged.
  The preface also serves as the GPLv3's notice that the work is modified, and when (§5a).

## New Files

The fork's changes, both new files and the modifications of upstream's files, are Copyright © Kentaro Inomata under **MIT-0** (2026-11-19; before, new files were under MIT OR Apache-2.0).
MIT-0 asks for nothing, not even attribution, so applications owe only upstream's attribution, and the changes can go upstream, which takes contributions under MIT-0.

- A file written for the fork starts with `// Copyright © Kentaro Inomata` and `// SPDX-License-Identifier: MIT-0` (`#` in TOML and PowerShell).
  The license text is in `LICENSES/MIT-0.txt`.
- Code ported from another project keeps that project's notice and license, such as `crates/core/host/menu/dark_bar.rs` from muda (Apache-2.0 OR MIT).
- A file copied from upstream, or adapted from upstream code, keeps upstream's header and triple license, however much it changes.
  So do new files in a crate copied from upstream when they carry upstream's header already, such as `tests/driver`.
- `tests/screenshots/build.rs`, `template.rs` and the files generated from it were adapted from upstream's test drivers; they carry upstream's header (corrected on 2026-11-19, they had been marked as new).
- Files that can't carry a header get their license from `REUSE.toml`, which copies upstream's entries for the files copied from upstream.
- A crate's `license` field states the license of its files: `tests/input`, `tools/ui-check` and the spikes are `MIT-0`, and `tests/probe` is `MIT-0 AND MIT` for the two upstream examples it keeps.
  Crates that mix both, such as `crates/core`, `tests/fixtures` and `tests/screenshots`, keep the workspace's triple license, which covers the whole.
