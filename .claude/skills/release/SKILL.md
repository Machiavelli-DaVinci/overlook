---
name: release
description: Cut an Overlook release. Use when asked to bump the version, tag, ship, publish, or release a new Overlook build, or when a change is ready for users of the in-app updater.
---

# Releasing Overlook

A release is a git tag `vX.Y.Z` that matches `version` in the workspace `Cargo.toml`. Pushing the tag
runs `.github/workflows/release.yml`, which builds the binary in an Arch container, builds the pacman
package and the generic tarball, writes `SHA256SUMS`, and publishes a GitHub release with generated
notes. Running installs find the release through the in-app updater within a day.

## Version convention

The working version on `main` is always the **next** release. After `v0.1.0` shipped, `Cargo.toml`
was bumped to `0.1.1`. So on a normal patch release there is nothing to bump before tagging; bump
first only when the release deserves a minor or major number.

## Steps

1. **Clean tree, green tests.**
   ```bash
   git status --short
   cargo test --workspace
   ```
   Everything the release should contain must already be committed and pushed.

2. **Decide the version.** Read it from `Cargo.toml`:
   ```bash
   grep -m1 '^version' Cargo.toml
   ```
   If this release warrants a bigger bump than the working version, change `version` in the workspace
   `Cargo.toml` (the crates inherit it with `version.workspace = true`), refresh the lockfile, and
   commit:
   ```bash
   cargo update -w --offline
   git commit -am "Overlook X.Y.Z"
   ```

3. **Tag and push.** Annotated tag, same number, `v` prefix. The workflow refuses a tag that
   does not match `Cargo.toml`.
   ```bash
   git tag -a vX.Y.Z -m "Overlook X.Y.Z"
   git push origin main
   git push origin vX.Y.Z
   ```

4. **Watch the workflow.**
   ```bash
   gh run list --workflow release.yml --limit 1
   gh run watch <run-id> --exit-status
   ```
   If it fails, fix on `main`, then move the tag and push it with force (`git tag -f -a vX.Y.Z -m ...`,
   `git push -f origin vX.Y.Z`). A failed run publishes nothing, so moving the tag is safe. The first
   release failed on a library missing from the CI container; the `pacman -Syu` line in the workflow is
   where runtime libraries are added, and `packaging/PKGBUILD` `depends` and `install.sh` must list the
   same ones.

5. **Verify the assets.** Expected: `overlook-X.Y.Z-1-x86_64.pkg.tar.zst`,
   `overlook-X.Y.Z-x86_64-linux.tar.gz`, `SHA256SUMS`, `install.sh`, plus makepkg's
   `overlook-debug-...pkg.tar.zst`, which the updater and installer deliberately ignore.
   ```bash
   gh release view vX.Y.Z --json assets --jq '.assets[].name'
   mkdir -p /tmp/overlook-release && cd /tmp/overlook-release
   gh release download vX.Y.Z -R last-refuge/overlook -p SHA256SUMS -p '*.tar.gz' -p 'overlook-X.Y.Z-1-*.zst'
   sha256sum -c SHA256SUMS --ignore-missing
   ```
   Also confirm `./install.sh --dry-run --yes` in the repo now offers the new package.

6. **Bump the working version** to the next patch so the next tag reads as newer than what users
   have, and note the release in `llm-docs/session-handoff.md`:
   ```bash
   sed -i '0,/^version = "X.Y.Z"/s//version = "X.Y.(Z+1)"/' Cargo.toml
   cargo update -w --offline
   git commit -am "Start X.Y.(Z+1)"
   git push origin main
   ```

## Do not

- Do not tag a version that differs from `Cargo.toml`; the workflow stops at its first step.
- Do not rename the asset files or the `SHA256SUMS` name. `crates/overlook-app/src/updater.rs` matches
  assets by suffix (`x86_64.pkg.tar.zst`, `x86_64-linux.tar.gz`) and verifies against `SHA256SUMS`.
- Do not delete a published release to redo it unless asked; running installs may be mid-update.
  Ship a new patch instead.
