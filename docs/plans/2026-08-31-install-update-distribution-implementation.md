# Install, Update, and Distribution Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add stable and Beta one-line installers, a manager-aware `lazydb update` command, and optional npm distribution backed by the existing verified GitHub Release binaries.

**Architecture:** GitHub Releases remain the immutable binary data plane, while GitHub Pages publishes stable and Beta channel manifests plus fixed installer URLs. Native installations use versioned directories and an atomic symlink; the CLI updates only confirmed native installations and delegates all other ownership to their package manager. npm packages wrap the same release artifacts and use isolated `latest` and `beta` dist-tags.

**Tech Stack:** Rust 2024, Clap, Tokio, Serde/serde_json, POSIX shell, GitHub Actions and Pages, GitHub Releases, npm optional dependencies, SHA-256, Git.

---

### Task 1: Define the Channel Manifest Contract

**Files:**
- Create: `scripts/release/generate-channel-manifest.py`
- Create: `scripts/release/test-channel-manifest.sh`
- Modify: `scripts/release/check-assets.sh:5-25`
- Modify: `docs/releasing.md:20-63`

**Step 1: Write manifest fixture tests**

Create table-driven fixtures for stable and Beta releases. Assert that generated
JSON contains `schema`, `product`, `channel`, `version`, `tag`, `prerelease`,
`published_at`, `release_url`, and exactly four target assets with HTTPS URLs and
64-character SHA-256 values.

Also assert rejection of:

- a stable channel with a prerelease version;
- a Beta channel with a stable version;
- a missing target archive;
- a checksum entry missing from `SHA256SUMS`;
- an unexpected repository or download host;
- duplicate asset names;
- an invalid release tag.

**Step 2: Run the test to verify it fails**

Run:

```bash
sh scripts/release/test-channel-manifest.sh
```

Expected: FAIL because `generate-channel-manifest.py` does not exist.

**Step 3: Implement deterministic generation**

Implement:

```text
generate-channel-manifest.py CHANNEL VERSION PUBLISHED_AT ASSET_DIR OUTPUT
```

Read `SHA256SUMS`, enumerate the four exact archive names already enforced by
`check-assets.sh`, construct canonical `https://github.com/yelog/lazydb/releases/download/vVERSION/...`
URLs, and write sorted, indented JSON through an atomic temporary file. Do not
call the GitHub API from this script.

**Step 4: Tighten asset validation**

Make `check-assets.sh` require the exact `.tar.xz` archive name for each target
rather than accepting any extension through `.*`. Keep stable-only package and
installer checks unchanged until the installer publication task changes them.

**Step 5: Run focused tests**

Run:

```bash
sh scripts/release/test-channel-manifest.sh
sh scripts/release/test-assets.sh
```

Expected: PASS for valid stable/Beta fixtures and actionable failures for every
invalid case.

**Step 6: Commit**

```bash
git add scripts/release/generate-channel-manifest.py scripts/release/test-channel-manifest.sh scripts/release/check-assets.sh docs/releasing.md
git commit -m "feat(release): define installation channel manifests"
```

### Task 2: Harden and Unify the Native Installer

**Files:**
- Modify: `install.sh:1-60`
- Create: `scripts/release/test-installer.sh`
- Create: `pages/install-beta.sh`
- Create: `pages/install.sh`

**Step 1: Write failing installation-layout tests**

Use a temporary `HOME`, an injectable channel base URL, and local fixture
archives. Cover:

- stable and Beta entry points choose the correct channel;
- Darwin/Linux and x86_64/aarch64 target mapping;
- install state under `$XDG_DATA_HOME/lazydb` with the documented HOME fallback;
- a versioned release directory and `current` symlink;
- `~/.local/bin/lazydb` linking through `current`;
- exact SHA-256 and `version --json` validation;
- repeated installation of the same release;
- retention of current and previous releases;
- no implicit `sudo`;
- unsupported platform and missing tool errors;
- malformed manifest, wrong product/channel, unsupported schema, and host rejection;
- absolute paths, `..`, unexpected links, and unexpected archive layouts;
- checksum mismatch and incorrect reported version;
- cleanup and unchanged `current` after every failure.

**Step 2: Run the tests to verify they fail**

Run:

```bash
sh scripts/release/test-installer.sh
```

Expected: FAIL because the current installer directly replaces one binary and
does not understand channel manifests.

**Step 3: Refactor one canonical installer**

Keep `install.sh` as the checked-in canonical implementation. Support:

```text
--channel stable|beta
--version VERSION
--install-dir PATH
--help
```

Use `LAZYDB_CHANNEL_BASE_URL` only as a documented test override. Production
defaults to `https://lazydb.yelog.org/channels`. Resolve the exact target asset
from the manifest, validate URL hosts, download into a private temporary
directory, verify SHA-256, validate archive entries before extraction, and run
the staged binary's `version --json`.

Install to `${XDG_DATA_HOME:-$HOME/.local/share}/lazydb/releases/VERSION`, switch
`current` atomically, create the visible bin symlink, and write `install.json`
atomically. Serialize mutation with a portable lock directory that records a PID
and safely recovers only stale locks.

**Step 4: Generate thin Pages entry points**

`pages/install.sh` and `pages/install-beta.sh` must be publishable standalone
scripts. Avoid a network-time dependency on another shell script. Generate or
copy them from the canonical installer during Pages assembly, baking in
`stable` or `beta` as the default channel. Add generated-file checks so the
published copies cannot drift from `install.sh`.

Beta output must contain `BETA` before download and after success.

**Step 5: Run installer tests and lint**

Run:

```bash
sh scripts/release/test-installer.sh
shellcheck install.sh pages/install.sh pages/install-beta.sh
```

Expected: PASS with no ShellCheck findings.

**Step 6: Commit**

```bash
git add install.sh pages/install.sh pages/install-beta.sh scripts/release/test-installer.sh
git commit -m "feat(install): add channel-aware atomic installer"
```

### Task 3: Model Installation State and Manager Detection in Rust

**Files:**
- Create: `src/update.rs`
- Modify: `src/lib.rs`
- Modify: `Cargo.toml:14-47`
- Test: `src/update.rs`

**Step 1: Write failing unit tests for state parsing**

Define fixtures for valid native `install.json`, unsupported schema, missing
fields, mismatched native executable, and malformed JSON. Assert that malformed
or stale metadata never authorizes native replacement.

**Step 2: Write failing manager-detection tests**

Inject the running executable path and command-probe results. Cover:

- confirmed native `current` symlink;
- Homebrew Cellar and prefix paths;
- npm launcher and global-prefix paths;
- `dpkg -S`, `rpm -qf`, and `pacman -Qo` ownership;
- Cargo bin and `target/{debug,release}` paths;
- stale native metadata;
- unknown paths.

Expected classifications are `native`, `homebrew`, `npm`, `deb`, `rpm`,
`arch`, `cargo`, and `unknown`.

**Step 3: Run focused tests to verify they fail**

Run:

```bash
cargo test update::tests --lib
```

Expected: FAIL because the update module does not exist.

**Step 4: Implement minimal state and detection types**

Add Serde types for the schema shared with the installer and an
`InstallationManager` enum. Keep filesystem and process probes behind small
traits or injected functions so tests do not depend on the developer machine.

Prefer existing `directories` and `serde_json` dependencies. Do not add a new
general-purpose update framework. Add an HTTP dependency only in the manifest
fetch task after its TLS behavior is selected.

**Step 5: Run tests**

Run:

```bash
cargo test update::tests --lib
cargo clippy --lib -- -D warnings
```

Expected: PASS.

**Step 6: Commit**

```bash
git add src/update.rs src/lib.rs Cargo.toml Cargo.lock
git commit -m "feat(update): detect installation ownership"
```

### Task 4: Add the `lazydb update` CLI Contract

**Files:**
- Modify: `src/cli.rs:83-112`
- Modify: `src/cli.rs:169-305`
- Modify: `src/main.rs:10-25`
- Test: `src/cli.rs`
- Test: `src/update.rs`

**Step 1: Write failing parser tests**

Assert parsing of:

```text
lazydb update
lazydb update --check
lazydb update --channel stable
lazydb update --channel beta --json
lazydb update --channel stable --allow-downgrade
```

Reject invalid channels and unsupported argument combinations. Use a Clap
`ValueEnum` for `stable` and `beta`.

**Step 2: Write failing output-schema tests**

Define `UpdateReport` with:

```text
schema, manager, channel, current_version, target_version, status, action
```

Assert JSON is one line and statuses cover `up_to_date`, `update_available`,
`updated`, `manager_action_required`, and `error`.

**Step 3: Run focused tests to verify they fail**

Run:

```bash
cargo test cli::tests update::tests --lib
```

Expected: FAIL because `Command::Update` and its report do not exist.

**Step 4: Add command arguments and asynchronous dispatch**

Add `Command::Update` to `src/cli.rs`. In `render_command`, classify it with
`Agent` and `Mcp` as requiring asynchronous execution so exhaustive matching
remains explicit. In `src/main.rs`, dispatch it to `lazydb::update::run(...)` and
print exactly one returned rendering.

Do not initialize the TUI or connect to a database during update checks.

**Step 5: Run focused and CLI smoke tests**

Run:

```bash
cargo test cli::tests update::tests --lib
cargo run -- update --help
cargo run -- update --check --json
```

Expected: parser and schema tests pass. The local check may report an unknown or
Cargo manager but must not modify files.

**Step 6: Commit**

```bash
git add src/cli.rs src/main.rs src/update.rs
git commit -m "feat(cli): add update command contract"
```

### Task 5: Implement Channel Resolution and Version Policy

**Files:**
- Modify: `src/update.rs`
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Test: `src/update.rs`

**Step 1: Select the HTTP client deliberately**

Use a Rust client with rustls and no OpenSSL runtime dependency. Prefer the
smallest client already compatible with Tokio and the repository's TLS stack.
Disable unnecessary default features. Record connect, request, and total
timeouts in constants.

**Step 2: Write failing manifest-validation tests**

Test valid stable and Beta JSON plus wrong schema, product, channel,
prerelease flag, tag/version disagreement, missing target, non-HTTPS URL,
unexpected host, malformed digest, and duplicate logical targets.

**Step 3: Write failing version-policy tests**

Cover:

- native stable follows only stable;
- native Beta follows only Beta;
- explicit channel changes persist only after success;
- same version is up to date;
- newer target is available;
- lower target is refused without `--allow-downgrade`;
- lower target is permitted with the flag;
- non-native installations receive manager actions instead of downloads.

Use a SemVer parser rather than hand-written lexical comparison. Add the
minimal `semver` dependency if no existing crate provides correct prerelease
ordering.

**Step 4: Run tests to verify they fail**

Run:

```bash
cargo test update::tests --lib
```

Expected: FAIL for the unimplemented resolver and policy.

**Step 5: Implement fetch, validation, and policy**

Fetch only the selected channel URL, reject redirects outside the approved
Pages and GitHub hosts, validate the full schema before using any URL, and map
the current OS/architecture to one exact Rust target. `--check` stops after
building the report.

For Homebrew, npm, and Linux package managers, build the exact recommended
command. Never invoke `sudo`. For npm choose `@latest` or `@beta` from the
detected or explicitly selected channel.

**Step 6: Run tests and lint**

Run:

```bash
cargo test update::tests --lib
cargo clippy --all-targets --all-features -- -D warnings
```

Expected: PASS.

**Step 7: Commit**

```bash
git add src/update.rs Cargo.toml Cargo.lock
git commit -m "feat(update): resolve stable and beta channels"
```

### Task 6: Implement Verified Atomic Native Updates

**Files:**
- Modify: `src/update.rs`
- Test: `src/update.rs`
- Test: `tests/update_cli.rs`

**Step 1: Write failing filesystem integration tests**

Using a temporary installation root and fixture HTTP server, assert:

- successful download and SHA-256 verification;
- fixed archive layout validation;
- staged `version --json` agreement;
- atomic `current` switch;
- atomic `install.json` update;
- current and previous release retention;
- lock contention and stale-lock handling;
- current release remains active after download, checksum, extraction, smoke,
  permission, or switch failure;
- Beta channel remains Beta after an ordinary update;
- explicit successful channel switch updates persisted state;
- JSON and human output contain no duplicate progress text.

**Step 2: Run tests to verify they fail**

Run:

```bash
cargo test --test update_cli
```

Expected: FAIL because update application is not implemented.

**Step 3: Implement the native transaction**

Acquire the installation lock before inspecting or mutating release
directories. Download to a private temporary location, stream the digest,
validate archive members before extraction, set executable permissions, and
smoke-test the staged binary. Move only a complete release directory into
place, then atomically replace `current` and metadata.

Do not overwrite the currently running executable. Do not write outside the
confirmed native installation root. Cleanup old versions only after a successful
activation.

**Step 4: Run integration and regression tests**

Run:

```bash
cargo test --test update_cli
cargo test --all-targets --all-features
```

Expected: PASS.

**Step 5: Commit**

```bash
git add src/update.rs tests/update_cli.rs
git commit -m "feat(update): atomically activate native releases"
```

### Task 7: Publish Installers and Channel Manifests on GitHub Pages

**Files:**
- Create: `.github/workflows/pages.yml`
- Create: `scripts/release/build-pages.sh`
- Create: `scripts/release/test-pages.sh`
- Modify: `.github/workflows/release.yml:139-195`
- Modify: `docs/releasing.md`

**Step 1: Write failing Pages assembly tests**

Given stable and Beta release fixtures, assert the output contains both
installers, both channel manifests, optional `CNAME` content
`lazydb.yelog.org`, and no archives. Assert rebuilding for one new channel
preserves the other channel. Assert missing or invalid Release assets stop the
build.

**Step 2: Run the test to verify it fails**

Run:

```bash
sh scripts/release/test-pages.sh
```

Expected: FAIL because Pages assembly does not exist.

**Step 3: Implement deterministic Pages assembly**

`build-pages.sh OUTPUT_DIR` resolves the newest valid stable and Beta Releases
from authenticated `gh` JSON in CI, downloads only `SHA256SUMS` as needed,
calls the manifest generator, and copies the checked installer entry points.
Validate each selected Release with existing version and asset policy before
emitting files.

Keep GitHub API parsing out of the installer itself.

**Step 4: Add Pages deployment workflow**

Use official full-SHA-pinned Pages actions. Grant only:

```yaml
permissions:
  contents: read
  pages: write
  id-token: write
```

Trigger through `workflow_call` from a successful release and allow manual
recovery. Use the protected Pages environment and concurrency without
canceling an in-progress deployment.

**Step 5: Connect release publication**

After the GitHub Release is public, invoke Pages deployment. Remove the old
assumption that Beta cannot have a one-line installer. GitHub Release may keep
`lazydb-installer.sh` for stable backward compatibility, but Pages becomes the
documented canonical endpoint.

**Step 6: Validate scripts and workflows**

Run:

```bash
sh scripts/release/test-pages.sh
actionlint .github/workflows/pages.yml .github/workflows/release.yml
shellcheck scripts/release/build-pages.sh
```

Expected: PASS.

**Step 7: Commit**

```bash
git add .github/workflows/pages.yml .github/workflows/release.yml scripts/release/build-pages.sh scripts/release/test-pages.sh docs/releasing.md
git commit -m "feat(release): publish stable and beta installer channels"
```

### Task 8: Add npm Package Templates and Local Tests

**Files:**
- Create: `packaging/npm/lazydb/package.json`
- Create: `packaging/npm/lazydb/bin/lazydb.js`
- Create: `packaging/npm/platform/package.json.template`
- Create: `scripts/release/package-npm.sh`
- Create: `scripts/release/test-npm-packages.sh`
- Modify: `scripts/release/check-assets.sh`

**Step 1: Confirm package ownership before implementation**

Verify that the approved npm scope and all five package names are controlled by
the maintainer. If `@yelog` is unavailable, stop and request approval for the
fallback package name; do not silently publish under another identity.

**Step 2: Write failing packaging tests**

Build fixture artifacts and assert:

- four platform packages have exact version, `os`, `cpu`, and binary payload;
- the main package has exact-version optional dependencies;
- no package downloads code in `postinstall`;
- package archives contain no repository secrets or unrelated files;
- the launcher maps all supported Node platform/architecture pairs;
- disabled optional dependencies produce an actionable error;
- arguments, stdio, signals, and exit codes reach the native binary;
- `npm pack --json` succeeds in an isolated prefix.

**Step 3: Run tests to verify they fail**

Run:

```bash
sh scripts/release/test-npm-packages.sh
```

Expected: FAIL because npm packaging does not exist.

**Step 4: Implement package assembly**

`package-npm.sh VERSION ARTIFACT_DIR OUTPUT_DIR` extracts the already verified
four target archives, creates the platform packages, computes exact optional
dependency entries, and packs all five packages. It must not run Cargo.

The launcher selects one package, spawns its binary with inherited stdio,
forwards termination signals, and exits with the child's exact status.

**Step 5: Run package tests**

Run:

```bash
sh scripts/release/test-npm-packages.sh
```

Expected: PASS.

**Step 6: Commit**

```bash
git add packaging/npm scripts/release/package-npm.sh scripts/release/test-npm-packages.sh scripts/release/check-assets.sh
git commit -m "feat(release): package native binaries for npm"
```

### Task 9: Publish npm Stable and Beta Dist-Tags

**Files:**
- Modify: `.github/workflows/release.yml`
- Modify: `docs/releasing.md`
- Create: `scripts/release/test-npm-publish-policy.sh`

**Step 1: Write failing publication-policy tests**

Assert:

- stable maps to `latest`;
- Beta maps to `beta`;
- Beta cannot execute a command containing `--tag latest`;
- all platform packages publish before the main package;
- the package version equals Cargo, tag, and Release version;
- an existing matching package is idempotent;
- an existing conflicting package fails;
- npm packaging consumes downloaded build artifacts and never runs Cargo.

**Step 2: Run tests to verify they fail**

Run:

```bash
sh scripts/release/test-npm-publish-policy.sh
```

Expected: FAIL because no npm job exists.

**Step 3: Add the protected npm job**

Run after verified GitHub Release assets are available. Prefer npm trusted
publishing/OIDC. If a token is temporarily required, expose it only to this job
through the protected `release` environment.

Publish four platform tarballs first, verify them through `npm view`, publish
the main package last with `--tag latest` or `--tag beta`, and verify that the
other dist-tag did not change.

**Step 4: Validate policy and workflow**

Run:

```bash
sh scripts/release/test-npm-publish-policy.sh
actionlint .github/workflows/release.yml
```

Expected: PASS.

**Step 5: Commit**

```bash
git add .github/workflows/release.yml scripts/release/test-npm-publish-policy.sh docs/releasing.md
git commit -m "feat(release): publish npm stable and beta channels"
```

### Task 10: Update User and Maintainer Documentation

**Files:**
- Modify: `README.md:52-104`
- Modify: `README.md:106-113`
- Modify: `docs/releasing.md`
- Modify: `docs/configuration.md`

**Step 1: Rewrite installation order**

Lead with:

```bash
curl -fsSL https://lazydb.yelog.org/install.sh | sh
```

Add a clearly marked Beta subsection:

```bash
curl -fsSL https://lazydb.yelog.org/install-beta.sh | sh
```

Keep download-inspect-run instructions, Homebrew, native Linux package, release
archive, source-build, and uninstall guidance. Add optional npm commands:

```bash
npm install -g @yelog/lazydb
npm install -g @yelog/lazydb@beta
```

Warn against `sudo npm install -g`.

**Step 2: Document update ownership**

Explain `lazydb update`, `--check`, channel persistence, explicit switching,
and downgrade refusal. Include exact manager commands for Homebrew, npm, deb,
rpm, and Arch and state that LazyDB never invokes `sudo`.

**Step 3: Expand the release runbook**

Document Pages custom-domain setup and DNS verification, Pages environment,
npm package ownership/trusted publishing, stable/Beta recovery, manifest
inspection, and post-release smoke commands.

**Step 4: Validate documentation commands**

Run all local no-publish commands shown in the documentation and search for the
obsolete GitHub `releases/latest/download/lazydb-installer.sh` command. Keep it
only in an explicitly labeled backward-compatibility section if that asset is
still published.

Run:

```bash
git diff --check
```

Expected: PASS.

**Step 5: Commit**

```bash
git add README.md docs/releasing.md docs/configuration.md
git commit -m "docs: explain installer and update channels"
```

### Task 11: Verify the Complete Distribution Matrix

**Files:**
- Modify: files discovered by verification only
- Modify: `docs/releasing.md` if actual recovery details differ

**Step 1: Run repository checks**

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --release --locked
./target/release/lazydb version --json
./target/release/lazydb update --check --json
```

Expected: all checks pass. The source-built update check must remain
non-destructive.

**Step 2: Run release-script tests**

```bash
sh scripts/release/test-release-metadata.sh
sh scripts/release/test-version.sh
sh scripts/release/test-changelog-tools.sh
sh scripts/release/test-channel-manifest.sh
sh scripts/release/test-installer.sh
sh scripts/release/test-pages.sh
sh scripts/release/test-npm-packages.sh
sh scripts/release/test-npm-publish-policy.sh
```

Expected: PASS.

**Step 3: Run workflow and shell lint**

```bash
actionlint .github/workflows/ci.yml .github/workflows/release.yml .github/workflows/pages.yml
shellcheck install.sh pages/*.sh scripts/release/*.sh
git diff --check
```

Expected: PASS with pinned Actions and minimal permissions.

**Step 4: Perform non-publishing installation smoke tests**

Against local fixtures, run stable first install, stable update, Beta first
install, Beta update, explicit channel switch, downgrade refusal, checksum
failure, and rollback checks on macOS and Linux. Pack and install the npm package
into a temporary prefix and run `lazydb version --json` through its launcher.

Expected: the exact intended version runs and no fixture failure changes the
active installation.

**Step 5: Exercise a Beta release before stable**

After reviewing the full diff and configuring Pages and npm environments, use
the existing `release beta` process. Verify:

- GitHub marks it as a prerelease;
- all four archives and supply-chain assets exist;
- `channels/beta.json` advances and `channels/stable.json` does not;
- `install-beta.sh` installs and labels the exact Beta;
- npm `beta` advances and npm `latest` does not;
- Homebrew does not change;
- `lazydb update` from the previous Beta reaches the new Beta.

Do not publish a test tag solely to exercise the workflow. Use the next approved
Beta release.

**Step 6: Verify the next stable release**

Confirm the stable Pages manifest, native installer, Homebrew Formula, Linux
packages, npm `latest`, and native stable update all resolve the same version.
Confirm Beta remains independently selectable.

**Step 7: Commit verification fixes**

Stage only files changed to correct verified defects:

```bash
git add <verified-fix-files>
git commit -m "fix(release): complete installer channel verification"
```

Skip this commit when verification produced no changes.
