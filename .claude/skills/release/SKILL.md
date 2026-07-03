---
name: release
description: Cut a release of gin-rummy-engine — version bump, changelog rollover, tag, and the crates.io publish-order constraint with the sibling gin-rummy crate. Use when asked to release, publish, or bump the version.
---

# Release gin-rummy-engine

## Pre-flight

1. Working tree clean, on `main`, CI green.
2. Full local gauntlet (see CLAUDE.md → Verification): fmt, clippy with
   `-D warnings`, doc with `-D warnings`, `cargo test --all-features`,
   `cargo check --no-default-features`.
3. If any bot's decision logic changed since the last release, run the
   strength tripwire in release mode:
   `cargo test --release --test strength -- --ignored`.

## The publish-order constraint

gin-rummy is a path dependency (`path = "../gin-rummy"` with a version
bound).  `cargo publish` strips the path and keeps the version, so
publishing requires a gin-rummy release satisfying that bound on crates.io
**first**.  If gin-rummy changed too, release it before this crate and
bump the version bound in `Cargo.toml` here.

## Steps

1. Pick the version.  Pre-1.0 semver as cargo interprets it: breaking
   changes bump the minor (0.1.x → 0.2.0); backward-compatible additions
   and fixes bump the patch (0.1.0 → 0.1.1).  Anything `#[non_exhaustive]`
   gaining fields or variants is non-breaking by design.
2. `Cargo.toml`: bump `version`.  Run `cargo check` so `Cargo.lock` picks
   up the new version; commit both files.
3. CHANGELOG.md rollover:
   - Rename `## [Unreleased]` to `## [x.y.z] - YYYY-MM-DD` and add a fresh
     empty `## [Unreleased]` above it.
   - Update the link references at the bottom: point `[Unreleased]` at
     `compare/vx.y.z...HEAD` and add `[x.y.z]` (compare from the previous
     tag, or `releases/tag/vx.y.z` for a first entry).
4. Commit as `Release x.y.z` (see `git log` for the house style), then
   tag and push:

   ```console
   git tag vx.y.z
   git push && git push --tags
   ```

5. Publish, once the ordering constraint is satisfied:

   ```console
   cargo publish --dry-run
   cargo publish
   ```

   While gin-rummy is not yet on crates.io, stop after the tag — the dry
   run will fail on the unpublished dependency, and that is expected.
