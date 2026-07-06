---
name: release
description: Cut a release of gin-rummy-engine — version bump, changelog rollover, tag, GitHub release, and the crates.io publish-order constraint with the sibling gin-rummy crate. Use when asked to release, publish, or bump the version.
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

gin-rummy is normally a plain version dependency and both crates are on
crates.io, so a routine release has no ordering to worry about.  If a
coordinated change is in flight — the dependency temporarily points at
`path = "../gin-rummy"`, or the release needs unreleased gin-rummy
commits — release gin-rummy **first**, bump the version bound in
`Cargo.toml` here, and switch the dependency back to a pure version
requirement before continuing.

## Steps

1. Pick the version.  Pre-1.0 semver as cargo interprets it: breaking
   changes bump the minor (0.1.x → 0.2.0); backward-compatible additions
   and fixes bump the patch (0.1.0 → 0.1.1).  Anything `#[non_exhaustive]`
   gaining fields or variants is non-breaking by design.
2. `Cargo.toml`: bump `version`.  Run `cargo check` so `Cargo.lock` picks
   up the new version, and `cargo check` in `web/` too — it depends on
   the parent by path, so its own `Cargo.lock` records the version.
3. CHANGELOG.md rollover:
   - Rename `## [Unreleased]` to `## [x.y.z] - YYYY-MM-DD` and add a fresh
     empty `## [Unreleased]` above it.
   - Update the link references at the bottom: point `[Unreleased]` at
     `compare/x.y.z...HEAD` and add `[x.y.z]` (compare from the previous
     tag, or `releases/tag/x.y.z` for a first entry).
4. Commit as `Release x.y.z` (see `git log` for the house style) and
   `git push`.  **Do not tag yet.**
5. Wait for GitHub CI to go green on that commit
   (`gh run watch` or `gh run list`).  Nothing ships before it passes.
6. Tag and push the tag.  Tags are unprefixed (`x.y.z`, not `vx.y.z`),
   matching the sibling crates and the GitHub release title:

   ```console
   git tag x.y.z
   git push --tags
   ```

7. Create the GitHub release (skipping this is how 0.1.2 ended up with a
   tag but no release).  House format, matching `gh release view 0.1.1`:
   title is the bare version; the body opens with a one-paragraph summary
   of the release's character, then the changelog sections for this
   version, then a footer pinned to the tag:

   ```
   📦 [crates.io/crates/gin-rummy-engine](…) · 📖 [docs.rs/gin-rummy-engine](…) · See [CHANGELOG.md](https://github.com/jdh8/gin-rummy-engine/blob/x.y.z/CHANGELOG.md).
   ```

   ```console
   gh release create x.y.z --title x.y.z --notes-file <body.md>
   ```

8. Publish:

   ```console
   cargo publish --dry-run
   cargo publish
   ```
