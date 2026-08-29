# Release Architecture Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a project-level release skill and a secure GitHub Actions pipeline that publishes validated LazyDB Beta binaries and stable Homebrew, Linux native package, and shell-installer releases with Changelog-derived notes.

**Architecture:** Keep semantic version recommendation and Changelog preparation in a reviewable OpenCode skill, put deterministic policy in testable repository scripts, and let GitHub Actions build only immutable tags. Use dist for cross-platform Rust release orchestration and nFPM for stable Linux packages generated from the same verified binaries.

**Tech Stack:** Rust 2024/Cargo, POSIX shell, GitHub Actions, dist (formerly cargo-dist), nFPM, Homebrew Tap, GitHub artifact attestations, Keep a Changelog, Git.

---

### Task 1: Establish Release Metadata and Changelog Contract

**Files:**
- Modify: `Cargo.toml:1-9`
- Modify: `Cargo.lock`
- Create: `CHANGELOG.md`
- Create: `docs/releasing.md`
- Test: `scripts/release/test-release-metadata.sh`

**Step 1: Write failing metadata contract tests**

Create a shell test that expects:

- `Cargo.toml` repository equals `https://github.com/yelog/lazydb`;
- package metadata includes README, homepage or repository, categories, and keywords needed by generated package metadata;
- `CHANGELOG.md` starts with a Keep a Changelog header and contains version-link markers;
- the Cargo package version can be read without compiling.

Run:

```bash
sh scripts/release/test-release-metadata.sh
```

Expected: FAIL because `CHANGELOG.md` and the release metadata contract do not exist and the repository URL is stale.

**Step 2: Correct package metadata**

Update `Cargo.toml` minimally:

```toml
repository = "https://github.com/yelog/lazydb"
homepage = "https://github.com/yelog/lazydb"
readme = "README.md"
keywords = ["database", "sql", "terminal", "tui"]
categories = ["command-line-utilities", "database"]
```

Do not change version `0.1.0` as part of release infrastructure implementation. Regenerate `Cargo.lock` only if Cargo changes its root package metadata representation.

**Step 3: Add the initial Changelog skeleton**

Create `CHANGELOG.md` with a Keep a Changelog introduction, Semantic Versioning statement, and link markers. Do not fabricate a released `0.1.0` entry during infrastructure work; the first release skill run creates the first dated version section from repository history.

**Step 4: Document maintainer prerequisites**

Create `docs/releasing.md` covering:

- supported tag forms;
- Beta and stable channel behavior;
- GitHub `release` Environment;
- `yelog/homebrew-tap` prerequisite;
- fine-grained Tap token or GitHub App permissions;
- local tool versions;
- release skill usage;
- recovery through an existing tag only;
- the distinction between Release assets and configured Linux repositories.

**Step 5: Run metadata tests and Cargo validation**

Run:

```bash
sh scripts/release/test-release-metadata.sh
cargo metadata --no-deps --format-version 1
cargo check --locked
```

Expected: all commands succeed and metadata reports `https://github.com/yelog/lazydb`.

**Step 6: Commit metadata contract**

```bash
git add Cargo.toml Cargo.lock CHANGELOG.md docs/releasing.md scripts/release/test-release-metadata.sh
git commit -m "chore(release): establish release metadata contract"
```

### Task 2: Implement Deterministic Version and Tag Validation

**Files:**
- Create: `scripts/release/release-lib.sh`
- Create: `scripts/release/validate-version.sh`
- Create: `scripts/release/set-version.sh`
- Create: `scripts/release/test-version.sh`

**Step 1: Write failing tag parsing tests**

Use temporary fixture directories and table-driven shell assertions for:

```text
v0.1.0                -> stable, 0.1.0
v0.2.0-beta.1         -> beta, 0.2.0-beta.1
v10.20.30-beta.42     -> beta, 10.20.30-beta.42
0.1.0                 -> invalid
v0.1                  -> invalid
v0.1.0-alpha.1        -> invalid
v0.1.0-beta           -> invalid
v0.1.0-beta.0         -> invalid if Beta numbering begins at 1
v01.1.0               -> invalid
```

Also test disagreement among tag, `Cargo.toml`, `Cargo.lock`, and supplied Changelog version.

Run:

```bash
sh scripts/release/test-version.sh
```

Expected: FAIL because the validators do not exist.

**Step 2: Implement shared shell primitives**

In `release-lib.sh`, implement narrowly scoped functions for:

- supported tag parsing;
- channel classification;
- tag-to-version conversion;
- Cargo version extraction;
- latest stable tag selection;
- latest Beta tag selection for a target base version;
- checking that a tag commit is reachable from `origin/main`;
- portable SHA-256 selection (`sha256sum` or `shasum -a 256`).

Use `set -eu`. Avoid parsing TOML with broad regular expressions when `cargo metadata --no-deps --format-version 1` can provide the package version. If JSON parsing needs `jq`, make it an explicit checked prerequisite.

**Step 3: Implement invariant validation**

`validate-version.sh TAG` must fail unless:

- the tag matches the exact supported syntax;
- the Cargo package and lockfile versions match the tag without `v`;
- exactly one matching Changelog heading exists;
- the matching Changelog section is non-empty;
- for CI, the tag exists and is reachable from `origin/main`.

Provide a flag for local pre-tag validation that checks the intended tag string without requiring that the tag already exists.

**Step 4: Implement version updates**

`set-version.sh VERSION` must:

- accept only a supported version without `v`;
- run `cargo set-version` if a pinned tool is adopted, or make the smallest safe `Cargo.toml` edit;
- run `cargo update --workspace --precise VERSION` or another verified Cargo command so the root package in `Cargo.lock` changes;
- verify both files afterward;
- avoid touching dependency versions.

Do not add a general-purpose versioning framework.

**Step 5: Run tests**

```bash
sh scripts/release/test-version.sh
sh scripts/release/validate-version.sh --help
git diff --check
```

Expected: all valid examples pass, all invalid examples fail with actionable errors, and no dependency version changes occur.

**Step 6: Commit version tooling**

```bash
git add scripts/release/release-lib.sh scripts/release/validate-version.sh scripts/release/set-version.sh scripts/release/test-version.sh
git commit -m "chore(release): add deterministic version validation"
```

### Task 3: Implement Release Range and Changelog Tooling

**Files:**
- Create: `scripts/release/collect-commits.sh`
- Create: `scripts/release/changelog-section.sh`
- Create: `scripts/release/validate-changelog.sh`
- Create: `scripts/release/test-changelog.sh`
- Modify: `scripts/release/release-lib.sh`

**Step 1: Write failing fixture-history tests**

Create temporary Git repositories in the test script with root, feature, merge, fix, revert, Beta tag, and stable tag commits. Assert:

- first release includes the root commit through `HEAD`;
- `beta.1` starts after the previous published tag;
- `beta.2` starts after `beta.1` of the same target version;
- stable starts after the previous stable tag and includes commits already covered by Betas;
- merge and revert commits remain in collected metadata;
- commit output includes full SHA, short SHA, author, date, subject, body, changed paths, and references;
- a Changelog section is extracted exactly up to the next `##` heading;
- missing, duplicate, or empty version headings fail.

Run:

```bash
sh scripts/release/test-changelog.sh
```

Expected: FAIL because range selection and extraction are absent.

**Step 2: Implement range selection**

Add functions that receive the intended channel and version and output explicit base/head revisions. Do not rely on `git describe` alone because stable releases intentionally ignore intervening Beta tags.

Rules:

- later Beta uses the prior Beta of the same `MAJOR.MINOR.PATCH`;
- first Beta uses the latest published tag reachable from `HEAD`;
- stable uses the latest stable tag reachable from `HEAD`;
- no baseline uses the root commit with an inclusive range.

**Step 3: Implement structured commit collection**

`collect-commits.sh CHANNEL VERSION` should emit JSON for the skill, not prose. Include enough data for semantic analysis without requiring the skill to reconstruct Git history. Use NUL-safe or record-separator-safe Git formatting so multiline bodies do not corrupt records.

The output contract should resemble:

```json
{
  "base": "v0.1.0",
  "head": "HEAD",
  "commits": [
    {
      "sha": "...",
      "short_sha": "...",
      "author": "...",
      "date": "...",
      "subject": "feat(sql): ...",
      "body": "...",
      "paths": ["src/sql/completion.rs"]
    }
  ]
}
```

**Step 4: Implement exact section extraction**

`changelog-section.sh VERSION` prints the exact matching `## [VERSION]` section, excluding unrelated versions. It must use a parser with clear heading rules rather than an unbounded `sed` range. A small checked-in Rust, shell, or Perl implementation is acceptable; prefer the lowest-dependency option that remains portable on GitHub macOS and Linux runners.

**Step 5: Implement Changelog validation**

Verify:

- one heading for the requested version;
- ISO date;
- at least one semantic or `Internal` section;
- a non-empty `Commits` section;
- every SHA from the selected range appears exactly once in `Commits`;
- every linked compare/tag URL uses `https://github.com/yelog/lazydb`;
- no commit outside the selected range is incorrectly required.

**Step 6: Run fixture tests**

```bash
sh scripts/release/test-changelog.sh
sh scripts/release/test-version.sh
```

Expected: all range, traceability, and extraction cases pass.

**Step 7: Commit Changelog tooling**

```bash
git add scripts/release/release-lib.sh scripts/release/collect-commits.sh scripts/release/changelog-section.sh scripts/release/validate-changelog.sh scripts/release/test-changelog.sh
git commit -m "chore(release): add changelog range tooling"
```

### Task 4: Create the Project-Level OpenCode Release Skill

**Files:**
- Create: `.opencode/skills/release/SKILL.md`
- Create: `.opencode/skills/release/references/changelog-format.md`
- Modify: `docs/releasing.md`

**Step 1: Write skill acceptance scenarios**

Before writing the skill, add scenarios to the release runbook for:

- `release beta` with no previous tags;
- a second Beta in the same release line;
- promoting a Beta line to stable;
- a stable patch without a Beta;
- version override after the recommendation;
- dirty worktree, stale `main`, existing tag, failing tests, and push cancellation.

For each scenario, define expected prompts, files changed, validation commands, commit shape, and stopping point.

**Step 2: Create valid skill metadata**

Use the project skill location and frontmatter:

```markdown
---
name: release
description: Prepare and publish LazyDB Beta or stable releases; use when asked to release, cut a version, publish a beta, update CHANGELOG.md, or create a release tag.
---
```

The skill must be project-specific and must not activate for unrelated package publishing questions.

**Step 3: Encode the preparation workflow**

Require the skill to:

- verify branch, upstream, status, remote, and fetched tags;
- call `collect-commits.sh` rather than improvise a range;
- inspect both commit metadata and actual diff;
- recommend version with evidence;
- ask exactly one version confirmation question;
- generate Keep a Changelog sections and a complete commit list;
- call `set-version.sh`, `validate-changelog.sh`, and `validate-version.sh --pre-tag`;
- run repository quality checks and release binary smoke tests;
- inspect the final diff;
- ask before creating a release commit and annotated tag;
- ask separately before pushing commit and tag.

The skill should recommend commit message `chore(release): prepare vVERSION` and annotated tag message `LazyDB vVERSION`.

**Step 4: Encode safety constraints**

Explicitly prohibit:

- automatic force pushes;
- tag replacement;
- history rewriting;
- bypassed tests or hooks;
- source mutation in GitHub Actions;
- committing unrelated worktree changes;
- pushing without confirmation;
- storing GitHub or Tap credentials in files or output.

If unrelated changes exist, the skill should stop and ask the user to isolate release work rather than stash, revert, or include those changes automatically.

**Step 5: Add Changelog writing guidance**

The reference file defines section mapping, user-facing language, commit traceability, link format, merge/revert handling, and examples for Beta incremental and stable cumulative entries. Require factual summaries grounded in commit diff, not promotional prose.

**Step 6: Validate skill discovery manually**

Restart OpenCode because project skills are loaded at startup. In a fresh session, ask to prepare a LazyDB Beta and verify the `release` skill is surfaced before any Git operation.

Expected: the skill collects evidence, recommends but does not silently choose a version, and stops before mutating files until the confirmation point.

**Step 7: Commit the release skill**

```bash
git add .opencode/skills/release/SKILL.md .opencode/skills/release/references/changelog-format.md docs/releasing.md
git commit -m "feat(release): add project release skill"
```

### Task 5: Configure dist and the Cross-Platform Archive Matrix

**Files:**
- Modify: `Cargo.toml`
- Create: `dist-workspace.toml` if required by the selected dist version
- Create: `.github/workflows/release.yml` through `dist init`
- Modify: `.github/workflows/release.yml`
- Modify: `docs/releasing.md`

**Step 1: Pin and record the dist version**

Choose a current stable dist release after checking its release notes and generated workflow format. Record the exact version in configuration and the maintainer runbook. Do not use an unpinned installer command in CI.

Run:

```bash
```

Expected: the recorded version is installed locally.

**Step 2: Initialize configuration without publishing**

Configure:

- GitHub hosting;
- shell installer for stable releases;
- Homebrew support but prerelease publication disabled;
- `x86_64-apple-darwin`;
- `aarch64-apple-darwin`;
- `x86_64-unknown-linux-gnu`;
- `aarch64-unknown-linux-gnu`;
- `.tar.xz` archives with README and licenses;
- checksums;
- CI workflow generation.

Do not enable Windows, npm, crates.io publishing, musl, MSI, or PKG installers.

**Step 3: Generate and inspect the workflow**

Run the version-appropriate dist generation command, then inspect every generated Action. Replace floating Action tags with full commit SHAs while retaining comments that identify the upstream version.

Ensure the workflow has:

```yaml
permissions:
  contents: read
concurrency:
  group: release-${{ github.event.inputs.tag || github.ref_name }}
  cancel-in-progress: false
```

Do not accept generated behavior that publishes before custom quality, Changelog, package, and installation checks complete. Split generated build functionality from final publication if necessary.

**Step 4: Add strict event planning**

Implement `push.tags: ["v*"]` and `workflow_dispatch` with required existing `tag`. The first job calls the repository validators and emits `version`, `tag`, `channel`, `prerelease`, and extracted Release body as job outputs or artifacts.

Checkout full history and tags with credentials disabled:

```yaml
with:
  fetch-depth: 0
  persist-credentials: false
```

**Step 5: Validate dist configuration**

Run the version-appropriate commands equivalent to:

```bash
dist plan
dist build --artifacts=local
```

Expected: the four target archives are planned, prerelease package-manager publication is disabled, and no unsupported target is advertised.

**Step 6: Commit dist configuration**

```bash
git add Cargo.toml dist-workspace.toml .github/workflows/release.yml docs/releasing.md
git commit -m "ci(release): configure cross-platform archives"
```

Only stage `dist-workspace.toml` if the selected dist version creates it.

### Task 6: Add Release Quality Gates and Artifact Smoke Tests

**Files:**
- Modify: `.github/workflows/release.yml`
- Create: `.github/workflows/ci.yml`
- Create: `scripts/release/smoke-artifact.sh`
- Create: `scripts/release/test-smoke-artifact.sh`

**Step 1: Write failing smoke-script tests**

Create a fake executable fixture that emits controlled JSON. Assert the smoke script rejects:

- a version different from the expected tag;
- invalid `version --json` output;
- failed `doctor --json` execution;
- a doctor response with a different package version;
- a missing executable bit.

Run:

```bash
sh scripts/release/test-smoke-artifact.sh
```

Expected: FAIL because the script does not exist.

**Step 2: Implement artifact smoke checks**

`smoke-artifact.sh BINARY EXPECTED_VERSION` runs:

```bash
"$binary" version --json
"$binary" doctor --json
```

Parse JSON and assert exact version agreement. `doctor.ok` may report an environment warning, but the command and schema must remain valid and the test must not connect to a database.

**Step 3: Add shared quality checks**

Create `.github/workflows/ci.yml` for pull requests and pushes to `main` with:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

Add the existing Neovim plugin test invocation after determining and documenting its actual command. Pin Rust `1.94` and all Actions to commit SHAs.

**Step 4: Reuse quality checks in release**

The release workflow must run the same commands against the tag before build publication. Do not rely only on a previous branch check because tags can target another commit.

**Step 5: Smoke-test every native artifact**

Run the smoke script on native macOS artifacts and native or emulated Linux artifacts. For `aarch64-unknown-linux-gnu`, use an ARM64 runner if available or a pinned QEMU environment. Do not mark a target supported based on compilation alone.

**Step 6: Run local checks**

```bash
sh scripts/release/test-smoke-artifact.sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

Expected: all checks pass.

**Step 7: Commit quality gates**

```bash
git add .github/workflows/ci.yml .github/workflows/release.yml scripts/release/smoke-artifact.sh scripts/release/test-smoke-artifact.sh
git commit -m "ci(release): verify tagged source and binaries"
```

### Task 7: Add Stable Native Linux Packages With nFPM

**Files:**
- Create: `packaging/nfpm.yaml`
- Create: `scripts/release/package-linux.sh`
- Create: `scripts/release/test-package-config.sh`
- Modify: `.github/workflows/release.yml`
- Modify: `docs/releasing.md`

**Step 1: Write failing package configuration tests**

Assert the nFPM template defines:

- name `lazydb`;
- version supplied by a validated environment variable;
- correct homepage, license, maintainer, and description;
- `/usr/bin/lazydb` payload;
- license and documentation payloads;
- no hard dependency on Nerd Fonts, database servers, Secret Service, or desktop clipboard packages;
- supported `deb`, `rpm`, and `archlinux` formats only.

Run:

```bash
sh scripts/release/test-package-config.sh
```

Expected: FAIL because package configuration is absent.

**Step 2: Add nFPM configuration**

Use environment substitution for version, architecture, binary source, and output. Add format-specific architecture mapping:

| Internal | deb | rpm | Arch |
| --- | --- | --- | --- |
| `x86_64` | `amd64` | `x86_64` | `x86_64` |
| `aarch64` | `arm64` | `aarch64` | `aarch64` |

Use a fixed package file naming function so workflow, installer documentation, and tests agree.

**Step 3: Implement package generation wrapper**

`package-linux.sh VERSION ARCH BINARY OUTPUT_DIR` validates inputs, verifies the binary's reported version, then runs a pinned nFPM release for each stable format. It must refuse Beta versions.

**Step 4: Inspect generated packages**

For every package:

- inspect metadata with `dpkg-deb --info`, `rpm -qip`, or `pacman -Qip`;
- list payload paths;
- unpack without installation;
- verify binary checksum equals the source binary checksum;
- verify no unexpected absolute paths or lifecycle scripts exist.

**Step 5: Add stable-only workflow job**

Run native packaging only when `plan.outputs.channel == 'stable'`. Download verified Linux binary artifacts rather than rebuilding. Upload package artifacts for later checksum and installation jobs.

**Step 6: Test locally in containers**

Build one x86_64 set and run:

```bash
sh scripts/release/test-package-config.sh
```

Then inspect all three generated package formats with the relevant tools in pinned containers.

Expected: metadata and payload assertions pass and all packages contain the identical binary.

**Step 7: Commit Linux packaging**

```bash
git add packaging/nfpm.yaml scripts/release/package-linux.sh scripts/release/test-package-config.sh .github/workflows/release.yml docs/releasing.md
git commit -m "feat(release): package stable Linux artifacts"
```

### Task 8: Implement the Stable Shell Installer

**Files:**
- Create: `install.sh`
- Create: `scripts/release/test-installer.sh`
- Modify: `.github/workflows/release.yml`
- Modify: `README.md:57-83`
- Modify: `docs/releasing.md`

**Step 1: Write failing installer tests**

Use a local HTTP fixture or injectable download base URL. Test:

- Darwin/Linux and x86_64/aarch64 mapping;
- unsupported OS and architecture;
- explicit `--version` and `--install-dir`;
- default user-writable installation directory;
- absence of implicit `sudo`;
- failed download;
- missing checksum entry;
- checksum mismatch;
- archive path traversal attempt;
- successful extraction and version smoke test;
- cleanup after failure.

Run:

```bash
sh scripts/release/test-installer.sh
```

Expected: FAIL because `install.sh` does not exist.

**Step 2: Implement argument and platform handling**

Support:

```text
--version VERSION
--install-dir PATH
--help
```

Default to the latest stable GitHub Release when version is omitted. Reject Beta through the `latest` path; an explicit Beta version may remain unsupported because Beta distribution is binary-download only.

**Step 3: Implement secure download and verification**

Require HTTPS GitHub URLs in production, TLS 1.2 or later where curl supports it, redirect limits, temporary directories with cleanup traps, exact filename lookup in `SHA256SUMS`, and checksum verification before extraction. Validate archive members before extracting to prevent traversal.

Do not invoke `sudo`. If the selected directory is not writable, print an actionable command with a user-owned alternative.

**Step 4: Add installer smoke behavior**

After atomic placement, execute:

```bash
lazydb version --json
lazydb doctor --json
```

Verify exact version and print PATH guidance only when needed.

**Step 5: Test the installer**

```bash
sh scripts/release/test-installer.sh
shellcheck install.sh scripts/release/*.sh
```

Expected: all fixture cases pass and ShellCheck reports no errors.

**Step 6: Add stable installer asset**

Upload the script as `lazydb-installer.sh` only for stable releases. Ensure the Release's latest URL points to stable assets and no Beta workflow uploads an installer with that stable asset name.

**Step 7: Document installation**

Add to `README.md`:

- Homebrew stable install;
- one-line stable install;
- download-inspect-run alternative;
- direct archive and checksum verification;
- Beta Release asset download;
- `.deb`, `.rpm`, and `.pkg.tar.zst` commands;
- source build fallback.

**Step 8: Commit installer**

```bash
git add install.sh scripts/release/test-installer.sh .github/workflows/release.yml README.md docs/releasing.md
git commit -m "feat(release): add verified shell installer"
```

### Task 9: Add Checksums, SBOMs, and Provenance Attestations

**Files:**
- Create: `scripts/release/check-assets.sh`
- Create: `scripts/release/test-assets.sh`
- Modify: `.github/workflows/release.yml`
- Modify: `docs/releasing.md`

**Step 1: Write failing asset-manifest tests**

Assert that the checker rejects:

- a missing target archive;
- a stable release missing any native package or installer;
- a Beta containing stable-only channels;
- duplicate filenames;
- an asset absent from `SHA256SUMS`;
- an incorrect checksum;
- an unexpected architecture or extension.

Run:

```bash
sh scripts/release/test-assets.sh
```

Expected: FAIL because asset checking is absent.

**Step 2: Implement deterministic asset checks**

`check-assets.sh CHANNEL VERSION ASSET_DIR` enumerates the exact expected manifest. Generate sorted `SHA256SUMS` with stable locale and verify it immediately. Exclude the checksum file itself unless a separate signature mechanism is added.

**Step 3: Generate SBOMs**

Use a pinned SBOM tool or dist's supported SBOM generation to produce CycloneDX JSON or SPDX JSON tied to each target archive. Verify JSON schema shape and package version. Do not claim an SBOM covers native packaging metadata unless it is generated after package assembly or explicitly references the enclosed binary.

**Step 4: Add attestations**

Grant only the attestation job:

```yaml
permissions:
  contents: read
  id-token: write
  attestations: write
```

Use a full-SHA-pinned official GitHub attestation Action. Attest archives and stable native packages using their computed digests.

**Step 5: Run asset tests**

```bash
sh scripts/release/test-assets.sh
```

Expected: Beta and stable fixture manifests pass; all incomplete or mismatched manifests fail.

**Step 6: Commit supply-chain checks**

```bash
git add scripts/release/check-assets.sh scripts/release/test-assets.sh .github/workflows/release.yml docs/releasing.md
git commit -m "ci(release): attest complete release assets"
```

### Task 10: Implement Draft Release Publication From Changelog

**Files:**
- Modify: `.github/workflows/release.yml`
- Create: `scripts/release/publish-check.sh`
- Create: `scripts/release/test-publish-check.sh`
- Modify: `docs/releasing.md`

**Step 1: Write failing publication policy tests**

Test event fixtures for:

- valid stable tag push;
- valid Beta tag push;
- malformed `v*` tag;
- `workflow_dispatch` with a missing tag;
- dispatch tag not reachable from `main`;
- existing published Release;
- draft Release with matching assets;
- draft Release with conflicting assets.

Run:

```bash
sh scripts/release/test-publish-check.sh
```

Expected: FAIL because publication policy is absent.

**Step 2: Implement pre-publication checks**

The script validates event/tag policy, calls all version and Changelog validators, checks the expected asset manifest, and compares existing GitHub Release state using `gh` when running in CI. It must never delete or silently replace a published Release.

**Step 3: Create or update a draft Release**

Grant only this job `contents: write`. Create a draft with:

- tag and title `LazyDB vVERSION`;
- body from `changelog-section.sh VERSION`;
- `prerelease=true` for Beta;
- all validated assets.

If a matching draft exists, permit idempotent reuse only when existing asset checksums match. Fail on conflicting content.

**Step 4: Verify before publish**

Download the draft assets through the GitHub API, rerun `check-assets.sh`, verify `SHA256SUMS`, and compare the draft body to the exact Changelog extraction.

**Step 5: Publish atomically at the end**

Publish the draft only after required build, package, installer, checksum, attestation, and installation jobs pass. Do not use the `release.published` event as the primary workflow trigger because assets must exist before publication.

**Step 6: Add workflow summary**

Report channel, version, source range, asset names, checksums, attestation links, Release URL, and whether stable channel jobs are pending or complete.

**Step 7: Run policy tests and workflow lint**

```bash
sh scripts/release/test-publish-check.sh
actionlint .github/workflows/release.yml
```

Expected: tests pass and actionlint reports no errors.

**Step 8: Commit publication flow**

```bash
git add .github/workflows/release.yml scripts/release/publish-check.sh scripts/release/test-publish-check.sh docs/releasing.md
git commit -m "ci(release): publish changelog-backed releases"
```

### Task 11: Configure Stable Homebrew Tap Publication

**Files:**
- Modify: `Cargo.toml` or `dist-workspace.toml`
- Modify: `.github/workflows/release.yml`
- Modify: `README.md`
- Modify: `docs/releasing.md`
- External prerequisite: `yelog/homebrew-tap`

**Step 1: Create the external Tap prerequisite**

Create `yelog/homebrew-tap` manually or in a separately approved operation. Add branch protection and a fine-grained token or GitHub App credential with contents write access only to that repository. Store it as a secret in the protected `release` Environment.

Do not create or push the external repository during ordinary implementation without explicit authorization.

**Step 2: Configure stable-only Homebrew publication**

Set Tap to `yelog/homebrew-tap`, include both macOS architectures, and keep prerelease package-manager publication disabled. Ensure Beta workflows cannot access the Tap secret.

**Step 3: Protect secret access**

The Homebrew job must:

- depend on successful stable Release publication;
- use `environment: release`;
- have the minimal permissions required;
- receive the Tap credential only at its publishing step;
- skip for every prerelease.

**Step 4: Add Formula verification**

After updating the Tap, verify generated URLs and checksums match the GitHub Release. On macOS runners run:

```bash
brew tap yelog/tap
brew install yelog/tap/lazydb
lazydb version --json
brew uninstall lazydb
```

Test Intel and Apple Silicon where supported runners are available. At minimum, use `brew audit --strict` and `brew test-bot` appropriate to the Tap.

**Step 5: Document recovery semantics**

If the Tap update fails after the GitHub Release is public, preserve the Release, leave the workflow failed, fix the Tap issue, and rerun the existing-tag workflow. Never issue a replacement source tag solely to retry Homebrew.

**Step 6: Validate generated configuration**

Run:

```bash
dist plan
actionlint .github/workflows/release.yml
```

Expected: Homebrew appears only for stable publication and no prerelease can overwrite the Formula.

**Step 7: Commit Homebrew publication**

```bash
git add Cargo.toml dist-workspace.toml .github/workflows/release.yml README.md docs/releasing.md
git commit -m "feat(release): publish stable Homebrew formula"
```

Only stage `dist-workspace.toml` if it exists and changed.

### Task 12: Add Clean-Environment Installation Verification

**Files:**
- Create: `scripts/release/verify-deb.sh`
- Create: `scripts/release/verify-rpm.sh`
- Create: `scripts/release/verify-arch.sh`
- Modify: `.github/workflows/release.yml`
- Modify: `docs/releasing.md`

**Step 1: Define install verification assertions**

Each verifier must:

- install the package through its native package manager;
- confirm the installed path is package-owned;
- run `lazydb version --json` and compare exact version;
- run `lazydb doctor --json` without connecting;
- remove the package;
- verify the binary is gone;
- avoid relying on host state outside the clean container or VM.

**Step 2: Implement Debian verification**

Use a pinned supported Debian or Ubuntu container. Install the local `.deb` with `apt`, run assertions, and remove it. Do not merely call `dpkg-deb --extract`.

**Step 3: Implement RPM verification**

Use a pinned Fedora container and install the local `.rpm` with `dnf`, run assertions, and remove it.

**Step 4: Implement Arch verification**

Use a pinned Arch Linux environment and install the local `.pkg.tar.zst` with `pacman -U`, run assertions, and remove it.

If ARM64 container execution is unavailable on hosted runners, use an explicit ARM runner or QEMU and document the performance tradeoff. Do not skip ARM package verification silently.

**Step 5: Verify shell installation against staged assets**

Run `install.sh` against a local or draft-asset endpoint for every supported OS/architecture. Ensure tests verify the downloaded archive checksum and reported version.

**Step 6: Gate publication on verification**

Place installation verification before the final transition from draft to published Release. Native package verification runs only for stable; archive and checksum verification runs for both channels.

**Step 7: Run local verifier lint**

```bash
shellcheck scripts/release/verify-deb.sh scripts/release/verify-rpm.sh scripts/release/verify-arch.sh
actionlint .github/workflows/release.yml
```

Expected: no errors.

**Step 8: Commit installation verification**

```bash
git add scripts/release/verify-deb.sh scripts/release/verify-rpm.sh scripts/release/verify-arch.sh .github/workflows/release.yml docs/releasing.md
git commit -m "ci(release): verify native package installation"
```

### Task 13: Add Action Pinning and Dependency Update Policy

**Files:**
- Create: `.github/dependabot.yml`
- Modify: `.github/workflows/ci.yml`
- Modify: `.github/workflows/release.yml`
- Modify: `docs/releasing.md`

**Step 1: Audit every Action reference**

List every `uses:` entry and verify it points to a full 40-character commit SHA. Keep an inline comment with the human-readable release version.

Expected examples should have this shape, with actual reviewed SHAs selected during implementation:

```yaml
- uses: actions/checkout@FULL_COMMIT_SHA # vX.Y.Z
```

Do not copy placeholder SHAs into the workflow.

**Step 2: Add GitHub Actions Dependabot updates**

Configure weekly GitHub Actions updates with a small open-PR limit. Updates remain reviewable and must pass CI before merge.

**Step 3: Audit workflow permissions**

Verify:

- workflow default is `contents: read`;
- PR CI has no write permissions;
- Release publication alone has `contents: write`;
- attestation alone has OIDC and attestation write permissions;
- Homebrew secret is Environment-scoped and stable-only;
- checkout credentials are not persisted in non-pushing jobs.

**Step 4: Run security-oriented lint**

```bash
actionlint .github/workflows/ci.yml .github/workflows/release.yml
```

If available, run a workflow security scanner such as `zizmor` at a pinned version and resolve high-confidence findings.

Expected: no unpinned Actions, dangerous expression interpolation, pull-request secret exposure, or excessive permissions.

**Step 5: Commit supply-chain policy**

```bash
git add .github/dependabot.yml .github/workflows/ci.yml .github/workflows/release.yml docs/releasing.md
git commit -m "ci: pin release automation dependencies"
```

### Task 14: Verify the Complete Pipeline With a Beta Dry Run

**Files:**
- Modify: files discovered by dry-run fixes only
- Update: `docs/releasing.md`

**Step 1: Run all local script tests**

```bash
sh scripts/release/test-release-metadata.sh
sh scripts/release/test-version.sh
sh scripts/release/test-changelog.sh
sh scripts/release/test-smoke-artifact.sh
sh scripts/release/test-package-config.sh
sh scripts/release/test-installer.sh
sh scripts/release/test-assets.sh
sh scripts/release/test-publish-check.sh
```

Expected: all tests pass.

**Step 2: Run repository verification**

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --release --locked
./target/release/lazydb version --json
./target/release/lazydb doctor --json
```

Run the documented Neovim plugin test command as well.

Expected: all checks pass and the binary reports the Cargo version.

**Step 3: Validate packaging configuration**

```bash
dist plan
dist build --artifacts=local
actionlint .github/workflows/ci.yml .github/workflows/release.yml
shellcheck install.sh scripts/release/*.sh
```

Expected: the four target archives are planned, stable-only jobs are correctly conditional, and lint passes.

**Step 4: Inspect the complete diff**

```bash
git status --short
git diff --check
git diff -- Cargo.toml Cargo.lock CHANGELOG.md README.md docs/releasing.md .opencode/skills/release .github packaging scripts/release install.sh
```

Expected: only intended release infrastructure and documentation changes appear. Preserve and do not stage unrelated worktree modifications.

**Step 5: Exercise the release skill without pushing**

In a fresh OpenCode session, invoke the project release skill for `beta`. Confirm its recommended first Beta version, inspect generated Changelog coverage, allow local version preparation, and stop at the push confirmation.

Expected: no tag or remote mutation occurs unless explicitly confirmed.

**Step 6: Publish the first Beta only after review**

After repository ownership, GitHub Environment, runner availability, and target smoke tests are confirmed, use the skill to prepare the approved first Beta. Push its release commit and tag only with explicit maintainer approval.

Expected GitHub result:

- prerelease flag is set;
- four binary archives are present;
- checksums, SBOMs, and attestations are present;
- Release body equals the matching `CHANGELOG.md` section;
- no Homebrew update, native Linux package, or stable installer is published.

**Step 7: Record dry-run findings**

Update `docs/releasing.md` with actual runner constraints, observed durations, recovery details, and any approved deviation from the design. Fix defects with new commits; do not amend failed or already-pushed release history.

**Step 8: Commit dry-run documentation fixes**

```bash
git add docs/releasing.md
git commit -m "docs(release): record beta release verification"
```

Only create this commit if the runbook changed.

### Task 15: Validate Stable-Channel Publication Without Rebuilding

**Files:**
- Modify: files discovered by stable-channel validation only
- Update: `docs/releasing.md`

**Step 1: Prepare a stable fixture from existing verified binaries**

Use local or non-public fixture assets matching a stable version to exercise nFPM, installer, asset manifest, Changelog extraction, and Formula generation. Do not publish a real stable tag solely to test configuration.

**Step 2: Verify all native packages**

Run clean-environment `.deb`, `.rpm`, and `.pkg.tar.zst` installation tests for both architectures. Confirm every package encloses the same checksummed binary used by the corresponding archive.

**Step 3: Verify Homebrew Formula in a test Tap branch**

With explicit authorization for the external Tap, publish to a temporary branch or test Formula name, run Homebrew audit/install/version/uninstall checks, then remove the test branch through an approved non-destructive process.

**Step 4: Verify stable asset manifest and Release body**

Ensure the stable manifest includes four archives, six native packages, installer, checksums, SBOMs, and attestations. Confirm the Release body spans the previous stable tag to the candidate and therefore includes intervening Beta changes.

**Step 5: Verify idempotent recovery**

Against a draft or fixture Release, rerun publication checks and confirm identical assets are accepted while conflicting checksums fail. Confirm `workflow_dispatch` refuses an unknown or unreachable tag.

**Step 6: Run final verification**

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
actionlint .github/workflows/ci.yml .github/workflows/release.yml
shellcheck install.sh scripts/release/*.sh
git diff --check
git status --short
```

Expected: all checks pass and unrelated worktree changes remain untouched.

**Step 7: Commit stable validation fixes**

Stage only files changed to correct stable-channel defects and use a focused commit message such as:

```bash
git commit -m "fix(release): complete stable channel verification"
```

Skip this commit when no tracked files changed.
