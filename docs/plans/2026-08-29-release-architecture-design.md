# Release Architecture Design

## Goal

Build a secure and repeatable release pipeline for LazyDB that:

- publishes Beta and stable GitHub Releases from validated semantic-version tags;
- produces macOS and Linux binaries for `x86_64` and `aarch64`;
- publishes stable `.deb`, `.rpm`, and `.pkg.tar.zst` packages;
- updates a Homebrew tap for stable releases;
- provides a checksum-verifying one-line shell installer for stable releases;
- records every release's user-facing changes and source commits in `CHANGELOG.md`;
- uses the exact Changelog section as the GitHub Release body;
- keeps version preparation local and reviewable through a project-level OpenCode release skill.

The first phase deliberately publishes Linux packages as GitHub Release assets. It does not operate signed `apt`, DNF, or Pacman repositories and does not claim that `apt install lazydb`, `dnf install lazydb`, or `pacman -S lazydb` works without configuring an external repository.

## Current State

LazyDB is a single Rust 2024 package with one library and one binary target. `Cargo.toml` is the source of package version metadata and currently declares version `0.1.0` and Rust `1.94`. The CLI exposes machine-readable `version`, `capabilities`, and `doctor` commands, which provide suitable artifact smoke tests.

The application targets macOS and Linux and includes platform-sensitive credential-store and clipboard dependencies. The repository also contains a Neovim integration with its own Lua tests. There are currently no Git tags, GitHub Actions workflows, release configuration, packaging scripts, installer, or `CHANGELOG.md`.

`Cargo.toml` currently points to `https://github.com/lazydb/lazydb`, while the active Git remote is `git@github.com:yelog/lazydb.git`. Release metadata and generated links will use `https://github.com/yelog/lazydb`.

## Architectural Decision

Use three narrowly separated layers:

1. A project-level OpenCode release skill controls version analysis and release preparation.
2. Small repository scripts implement deterministic version, Changelog, tag, and asset validation.
3. GitHub Actions builds immutable tagged source and publishes its artifacts.

Use `dist` (formerly cargo-dist) for the Rust target matrix, archives, checksums, shell installer integration, GitHub Release orchestration, and stable Homebrew publishing where its generated workflow remains suitable. Use nFPM only to convert already-built Linux binaries into `.deb`, `.rpm`, and `.pkg.tar.zst` packages. Keep policy checks and Changelog extraction in repository scripts rather than embedding substantial shell programs in workflow YAML.

npm distribution is intentionally out of scope for the current release
architecture and remains future optional work. The updater may detect npm-owned
files to prevent overwrites, but must not claim that npm installation works.

This is preferred over a fully custom workflow because it delegates ordinary Rust distribution behavior to a maintained tool while preserving explicit control over release policy. It is preferred over release-plz because automated release PR and version calculation overlap with the required OpenCode skill and would create two competing release control planes.

## Trust Boundary

The release skill is the preparation control plane. It may inspect commits and diffs, recommend a version, ask for confirmation, update tracked files, run tests, create a release commit, create an annotated tag, and push after explicit confirmation.

GitHub Actions is the immutable build and publication plane. It must not decide a new version, rewrite `CHANGELOG.md`, or commit back to the release tag. It accepts only a tag whose source already contains matching version and Changelog data. This prevents a published tag, binary, and repository history from describing different versions.

Deterministic scripts remain authoritative for mechanical checks. The skill may use semantic reasoning to summarize changes, but it must call scripts to determine ranges, validate formats, update Cargo metadata, extract the selected Changelog section, and verify invariants.

## Version Model

Supported tags are exactly:

```text
vMAJOR.MINOR.PATCH
vMAJOR.MINOR.PATCH-beta.N
```

The validation regular expression is:

```text
^v[0-9]+\.[0-9]+\.[0-9]+(-beta\.[0-9]+)?$
```

GitHub tag filters are globs rather than regular expressions, so the workflow may use a broad `v*` push trigger but must reject nonconforming tags in its first job.

### Beta Recommendation

- If the intended base version has no Beta tag, recommend `X.Y.Z-beta.1`.
- If `X.Y.Z-beta.N` exists, recommend `X.Y.Z-beta.(N+1)`.
- If commit analysis indicates a different major, minor, or patch target, begin that target at `beta.1`.

### Stable Recommendation

- If the current release line contains `X.Y.Z-beta.N`, recommend promotion to `X.Y.Z`.
- Otherwise, recommend a major, minor, or patch bump from the previous stable tag.
- Use Conventional Commit types, `BREAKING CHANGE`, affected code, and the actual diff as evidence, not as an infallible automatic rule.
- Show the recommendation and evidence, then require maintainer confirmation before changing files.

## Release Skill

Create `.opencode/skills/release/SKILL.md`. It triggers on requests to prepare, cut, publish, or release a LazyDB Beta or stable version.

The skill accepts `beta` or `stable` intent and follows this sequence:

1. Require a clean release-related worktree and an up-to-date `main` branch. Unrelated changes are reported rather than reverted.
2. Fetch tags and inspect the latest stable and Beta tags.
3. Select the commit baseline according to the Changelog rules below.
4. Collect commit SHA, author, date, subject, body, PR/issue references, changed paths, and a range diff summary.
5. Recommend the next version and explain the major/minor/patch or Beta-sequence evidence.
6. Ask the maintainer to confirm or override the target version.
7. Generate a complete Changelog entry with semantic sections and a traceable commit list.
8. Update `Cargo.toml`, regenerate `Cargo.lock`, and validate all version invariants.
9. Run formatting, lint, tests, Changelog checks, and a local release build smoke test.
10. Show the release commit and tag operations before executing them.
11. Create a dedicated release commit and annotated tag only after explicit confirmation.
12. Push the commit and tag only after a second explicit confirmation, because pushing triggers publication.

The skill must stop before tagging if any validation fails. It must not suppress tests, force-push, rewrite history, overwrite an existing tag, or publish from an arbitrary dirty state.

## Changelog Model

`CHANGELOG.md` follows Keep a Changelog structure with machine-extractable version headings:

```markdown
## [0.2.0-beta.1] - 2026-08-29

### Added

- Add fuzzy identifier completion ([`b69385b`](...)).

### Commits

- [`b69385b`](...) feat(sql): add fuzzy identifier completion
```

Supported user-facing sections are `Added`, `Changed`, `Fixed`, `Security`, `Deprecated`, and `Removed`. `Internal` may contain release engineering, tests, refactors, or merge metadata that does not describe user behavior. Every commit in the selected range must appear in `Commits`, even when its behavior is summarized elsewhere. Merge and revert commits are not silently discarded.

### Range Rules

- `beta.1`: range from the previous published tag to `HEAD`.
- Later Beta: range from the previous Beta of the same target version to `HEAD`.
- Stable: range from the previous stable tag to `HEAD`, even if Beta tags exist in between.
- First release: range from the repository root through `HEAD`.

This makes Beta notes incremental while stable notes describe the complete stable release cycle. Stable entries may intentionally repeat changes already present in Beta entries.

The file maintains compare links for every version. A deterministic extraction command selects the exact `## [VERSION]` block up to the next level-two heading. Missing, duplicate, empty, or mismatched sections fail the release.

The GitHub Release body is exactly the selected Changelog version block, optionally followed by a generated artifact/install footer. GitHub-generated release notes are not the primary source and must not replace curated Changelog content.

## Trigger Rules

The release workflow supports two triggers:

```yaml
on:
  push:
    tags:
      - "v*"
  workflow_dispatch:
    inputs:
      tag:
        description: Existing release tag to rebuild or recover
        required: true
        type: string
```

Rules:

- A tag push is the normal publication path.
- `workflow_dispatch` may only target an existing remote tag. It cannot invent a version or mutate source files.
- The selected tag must pass the strict supported SemVer expression.
- The tag commit must be reachable from `main`.
- `Cargo.toml`, `Cargo.lock`, `lazydb version --json`, the tag, and the Changelog heading must agree.
- A Beta tag creates a GitHub prerelease.
- A stable tag creates a normal GitHub Release and may update stable package channels.
- Use `concurrency: release-${tag}` with `cancel-in-progress: false` to prevent simultaneous publication of the same tag.
- Bind stable publication secrets to a protected GitHub `release` Environment. Environment approval is recommended before cross-repository Homebrew publication.

## Artifact Matrix

### Beta

Beta releases contain only downloadable binary release assets:

| Platform | Architecture | Rust target |
| --- | --- | --- |
| macOS | Intel | `x86_64-apple-darwin` |
| macOS | Apple Silicon | `aarch64-apple-darwin` |
| Linux glibc | x86_64 | `x86_64-unknown-linux-gnu` |
| Linux glibc | ARM64 | `aarch64-unknown-linux-gnu` |

Assets include compressed archives, licenses, README, `SHA256SUMS`, SBOMs, and provenance attestations. Beta does not update Homebrew, produce native Linux packages, or change the stable installer's `latest` path.

### Stable

Stable releases contain the Beta asset set plus:

- `.deb` packages for `amd64` and `arm64`;
- `.rpm` packages for `x86_64` and `aarch64`;
- `.pkg.tar.zst` packages for `x86_64` and `aarch64`;
- a stable shell installer;
- a Homebrew Formula update.

The initial design does not promise musl targets. LazyDB's keyring and clipboard dependencies are platform-sensitive and require a dedicated compatibility investigation before advertising a static or musl build.

Linux binaries should be built against a controlled glibc baseline, using a pinned build container or `cargo-zigbuild`, rather than inheriting the moving ABI baseline of `ubuntu-latest`. macOS targets should use native GitHub runners. Every target must run the built binary's `version --json` and `doctor --json`; a release is not published with a silently missing architecture.

## Native Linux Packages

nFPM consumes the exact Linux binaries already produced and verified by the build jobs. It must not compile another copy.

Packages install:

- `/usr/bin/lazydb`;
- project license files;
- optional README or generated manual documentation in the distribution-appropriate documentation path.

Do not declare Nerd Fonts, a database server, or Secret Service as hard dependencies. Those are optional runtime capabilities. Package metadata includes repository URL, dual license, architecture, version, description, and package maintainer.

Release assets support these installation patterns:

```bash
sudo apt install ./lazydb_VERSION_ARCH.deb
sudo dnf install https://github.com/yelog/lazydb/releases/download/TAG/lazydb_VERSION_ARCH.rpm
sudo pacman -U https://github.com/yelog/lazydb/releases/download/TAG/lazydb_VERSION_ARCH.pkg.tar.zst
```

Direct `apt install lazydb`, `dnf install lazydb`, and `pacman -S lazydb` remain future work requiring maintained repositories, metadata, signing, hosting, and possibly distribution review.

## Homebrew

Stable releases publish to a separate `yelog/homebrew-tap` repository. Users install with:

```bash
brew install yelog/tap/lazydb
```

Homebrew provides only the latest stable version. Beta publication is disabled so a prerelease cannot replace the default Formula. The Tap job receives a fine-grained PAT or GitHub App credential scoped to the Tap repository because this repository's `GITHUB_TOKEN` cannot write to a different repository.

The stable release remains valid if its later Tap update fails, but the workflow remains failed and exposes a retryable Homebrew job. Re-running publication must be idempotent and must not duplicate or overwrite unrelated Release assets.

## Shell Installer

The stable one-line installation entry uses a versioned installer asset exposed through GitHub's latest stable Release:

```bash
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/yelog/lazydb/releases/latest/download/lazydb-installer.sh | sh
```

The installer:

- detects supported OS and architecture;
- downloads the matching archive and `SHA256SUMS` from the same Release;
- verifies SHA-256 before extraction;
- installs to a user-writable directory by default;
- supports an explicit version and `--install-dir` for reproducibility;
- never invokes `sudo` implicitly;
- fails closed for unsupported platforms, missing checksums, redirects to an unexpected host, or checksum mismatch;
- prints the installed version and runs a non-connecting smoke check.

Documentation should explain that piping remote code to a shell has inherent trust implications and show a download-inspect-run alternative.

## Workflow Jobs

### `plan`

Resolve the tag from the event, classify Beta or stable, validate tag syntax and reachability, verify Cargo and Changelog metadata, extract the Release body, and emit the version, channel, matrix, and asset naming data.

### `quality`

Run:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

Also run the repository's Neovim plugin test command when Neovim is available in CI. Release implementation should establish a normal pull-request CI workflow using the same checks so release tags do not discover ordinary failures for the first time.

### `build`

Build each target, run artifact-level `version --json` and `doctor --json` smoke tests where executable, assemble archives with licenses and README, and upload immutable workflow artifacts. Cross-built targets that cannot execute on the build host require a matching native runner or an explicit emulation test; compilation alone is insufficient for a supported release target.

### `supply-chain`

Download all archives, generate a single deterministic `SHA256SUMS`, create CycloneDX or SPDX SBOMs, and produce GitHub artifact attestations. Checksums cover every downloadable executable archive and native package.

### `native-packages`

Run only for stable tags. Generate `.deb`, `.rpm`, and `.pkg.tar.zst` files from the verified Linux binaries. Inspect package metadata and unpack each package to verify expected paths before uploading it as an internal workflow artifact.

### `verify-install`

Before publishing the draft, install and remove every native package in an appropriate clean container or VM. Exercise the installer against staged assets or a deterministic local fixture. Verify `lazydb version --json` and `lazydb doctor --json` after installation.

### `publish-release`

Create or update a draft GitHub Release, upload the complete asset set, verify expected asset names and checksums, set the body from the extracted Changelog section, and publish only after all required jobs pass. Set `prerelease: true` for Beta and `false` for stable.

### `publish-homebrew`

Run only after a stable GitHub Release succeeds. Update the Tap with stable archive URLs and checksums, then test `brew install yelog/tap/lazydb` and `lazydb version --json` on both supported macOS architectures where runners are available.

### `summary`

Write the version, channel, asset list, Release URL, package verification results, Homebrew status, and attestation links to the GitHub Actions job summary.

## Security

- Pin third-party Actions to full commit SHAs and record update automation through Dependabot or Renovate.
- Default workflow permissions to `contents: read`.
- Grant `contents: write` only to the GitHub Release job.
- Grant `id-token: write` and `attestations: write` only to attestation jobs.
- Expose the cross-repository Tap credential only to the stable Homebrew job through the protected `release` Environment.
- Set `persist-credentials: false` on checkout jobs that do not push.
- Never execute code or workflows from an untrusted pull request with release secrets.
- Keep generated assets in workflow artifacts between jobs instead of rebuilding them independently for each channel.
- Fail publication on an incomplete target matrix, checksum mismatch, metadata mismatch, duplicate Changelog heading, or unexpected existing Release asset.

## Failure and Recovery

- Fail before creating a tag when local release preparation checks fail.
- Fail before publication when any required target, package, smoke test, Changelog section, or checksum is missing.
- Build the GitHub Release as a draft until all assets and installation checks pass.
- Never delete an existing published Release automatically.
- Permit `workflow_dispatch` recovery only for an existing valid tag.
- Make asset upload and Tap updates idempotent, with explicit comparison against existing content.
- If Homebrew publication fails after the GitHub Release is public, preserve the Release, fail the workflow, and allow only the channel publication step to be retried.

## Testing

### Repository Scripts

Use fixture repositories or temporary Git histories to test:

- first release with no tags;
- `beta.1`, later Beta, and stable range selection;
- malformed and unsupported tags;
- tag/Cargo/Lock/Changelog disagreement;
- duplicate or missing Changelog sections;
- merge and revert commit inclusion;
- deterministic Release body extraction.

### Installer

Test OS/architecture mapping, explicit version selection, custom install directory, unsupported targets, failed downloads, malformed checksums, checksum mismatch, archive traversal rejection, successful install, smoke check, and clean uninstall guidance.

### Packages

Inspect metadata and payload paths, then install, run, and remove `.deb`, `.rpm`, and `.pkg.tar.zst` packages in clean matching environments.

### Workflow

Add a non-publishing packaging smoke workflow for pull requests that validates configuration and builds at least one representative archive and native package. Test the complete release pipeline with `v0.1.0-beta.1` before the first stable release.

## Documentation

Update `README.md` with stable Homebrew, native package, shell installer, binary download, source build, verification, and Beta download instructions. Add a maintainer release runbook that documents prerequisites, required GitHub Environment and Tap credentials, release skill usage, recovery, and rollback boundaries.

## Acceptance Criteria

- A confirmed `release beta` skill run prepares a valid `vX.Y.Z-beta.N` release commit and tag.
- A confirmed `release stable` skill run prepares a valid `vX.Y.Z` release commit and tag.
- Every selected source commit is traceable in the matching `CHANGELOG.md` entry.
- GitHub Release notes use the exact matching Changelog section.
- Beta publishes four binary target archives and supply-chain metadata as a prerelease only.
- Stable publishes the four archives, native Linux packages, installer, supply-chain metadata, and Homebrew Formula.
- Every binary and package reports the exact tag version.
- Stable Homebrew, `.deb`, `.rpm`, `.pkg.tar.zst`, and shell installation paths are tested.
- Invalid tags, mismatched versions, incomplete assets, and failed checksums stop publication.
- GitHub Actions never modifies the tagged source or generates a new release version.
