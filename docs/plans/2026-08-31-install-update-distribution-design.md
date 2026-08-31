# Install, Update, and Distribution Design

## Goal

Provide a consistent LazyDB installation and update experience across stable and
Beta releases without making Node.js a runtime requirement or allowing the
built-in updater to overwrite files owned by another package manager.

The public entry points are hosted by GitHub Pages behind the custom domain:

```text
https://lazydb.yelog.org/install.sh
https://lazydb.yelog.org/install-beta.sh
```

The expected user commands are:

```bash
curl -fsSL https://lazydb.yelog.org/install.sh | sh
curl -fsSL https://lazydb.yelog.org/install-beta.sh | sh
lazydb update
```

## Current State

LazyDB is a Rust TUI distributed as one native executable. The CLI is defined in
`src/cli.rs`, asynchronous commands are dispatched by `src/main.rs`, and the
current command set does not include `update`.

The release workflow already publishes archives for macOS and Linux on x86_64
and ARM64. Every release includes `SHA256SUMS`, an SBOM, and provenance
attestations. Stable releases additionally publish Linux native packages, a
shell installer, and a Homebrew Formula. Beta releases are GitHub prereleases
and currently remove the installer asset before publication.

The existing `install.sh` resolves GitHub's latest stable Release or an explicit
version, verifies the selected archive against `SHA256SUMS`, and copies the
binary directly to `~/.local/bin/lazydb`. It does not record an installation
manager or channel, does not use a versioned layout, and cannot resolve the
latest Beta.

## Decisions

### Distribution Priority

The recommended installation order is:

1. Native GitHub Pages installer for the simplest cross-platform experience.
2. Homebrew or a native Linux package for users who prefer system package
   ownership.
3. npm as an optional developer-oriented channel.
4. Cargo source builds for contributors and unsupported environments.

npm is not the default because LazyDB does not use Node.js at runtime and its
database, terminal, and Neovim users do not necessarily have Node.js installed.
It remains valuable because many coding-agent and JavaScript users already have
a correctly configured npm global environment.

### Persistent Channels

LazyDB has two independent channels:

- `stable` resolves only `MAJOR.MINOR.PATCH` releases;
- `beta` resolves only `MAJOR.MINOR.PATCH-beta.N` releases.

A native stable installation continues to follow stable when `lazydb update`
is run. A native Beta installation continues to follow Beta. Channel switching
must be explicit with `lazydb update --channel stable|beta`; the updater must
not choose a channel by comparing stable and prerelease SemVer values.

If an explicit channel switch would install an older SemVer version, the
updater refuses it unless the user also supplies `--allow-downgrade`.

### Package Manager Ownership

`lazydb update` must respect the manager that owns the active executable:

| Manager | Update behavior |
| --- | --- |
| Native installer | Download, verify, install, and atomically activate the selected release |
| Homebrew | Report and optionally delegate to `brew upgrade yelog/tap/lazydb` |
| npm | Report and optionally delegate to `npm install -g @yelog/lazydb@latest` or `@beta` |
| deb/rpm/Arch | Print the appropriate package-manager guidance; never invoke `sudo` |
| Cargo/source | Refuse to overwrite the build and explain the supported alternatives |
| Unknown | Make no changes unless the user explicitly converts to a native installation |

The first implementation may print the exact Homebrew and npm command rather
than executing it. Correct detection and non-destructive behavior take priority
over automatic delegation.

## Channel Metadata

GitHub Pages is the stable channel control plane. It publishes:

```text
/install.sh
/install-beta.sh
/channels/stable.json
/channels/beta.json
```

Each channel document uses a versioned schema and contains the complete
platform asset mapping:

```json
{
  "schema": 1,
  "product": "lazydb",
  "channel": "beta",
  "version": "0.2.0-beta.3",
  "tag": "v0.2.0-beta.3",
  "prerelease": true,
  "published_at": "2026-08-31T12:00:00Z",
  "release_url": "https://github.com/yelog/lazydb/releases/tag/v0.2.0-beta.3",
  "assets": {
    "aarch64-apple-darwin": {
      "url": "https://github.com/yelog/lazydb/releases/download/v0.2.0-beta.3/lazydb_0.2.0-beta.3_aarch64-apple-darwin.tar.xz",
      "sha256": "HEX_DIGEST"
    }
  }
}
```

The other required asset keys are `x86_64-apple-darwin`,
`x86_64-unknown-linux-gnu`, and `aarch64-unknown-linux-gnu`.

The release workflow updates a channel only after the matching GitHub Release
and all of its assets have been published successfully. A Beta publication must
not modify `stable.json`; a stable publication must not modify `beta.json`.
GitHub Release assets remain the binary data plane. Pages manifests do not
duplicate the archives.

The Pages deployment is rebuilt as a complete artifact rather than treating a
`gh-pages` branch as mutable application state. The build resolves the current
valid stable and Beta GitHub Releases and emits both manifests, preserving the
other channel during every deployment.

## Native Installation Layout

Native installations use a versioned directory and an atomic `current` link:

```text
~/.local/share/lazydb/
|-- install.json
|-- update.lock
|-- releases/
|   |-- 0.1.0-beta.1/lazydb
|   `-- 0.1.0-beta.2/lazydb
`-- current -> releases/0.1.0-beta.2

~/.local/bin/lazydb -> ~/.local/share/lazydb/current/lazydb
```

`install.json` records only non-secret local state:

```json
{
  "schema": 1,
  "manager": "native",
  "channel": "beta",
  "version": "0.1.0-beta.2",
  "bin_dir": "/Users/example/.local/bin",
  "installed_at": "2026-08-31T12:00:00Z"
}
```

The installer and updater share this contract. Installation proceeds by
downloading into a private temporary directory, validating the archive member
layout, verifying SHA-256, extracting a fixed binary path, running `version
--json`, moving the completed release directory into place, and atomically
switching `current`. A failed operation leaves the active release untouched.

The current and immediately previous releases are retained. Older releases may
be removed after a successful switch. Concurrent installer and updater runs are
serialized by a lock without depending on a platform-specific locking tool.

## CLI Contract

Add an asynchronous command with this interface:

```text
lazydb update [--check] [--channel stable|beta] [--allow-downgrade] [--json]
```

Default `lazydb update` behavior:

1. Resolve the running executable and installation manager.
2. Read the native channel when native installation metadata is present.
3. Resolve the matching Pages channel manifest.
4. Report that the installation is current or apply the verified update.
5. For non-native managers, print the exact manager-owned upgrade command and
   make no filesystem changes.

`--check` performs no update. `--json` emits one compact JSON object and no
human-oriented text, with a stable schema suitable for Neovim and automation:

```json
{
  "schema": 1,
  "manager": "native",
  "channel": "beta",
  "current_version": "0.1.0-beta.1",
  "target_version": "0.1.0-beta.2",
  "status": "update_available",
  "action": "native_update"
}
```

Network, manifest, checksum, permission, and manager-detection errors must be
actionable. JSON mode returns a non-zero exit status and a structured error;
human mode writes the error to stderr.

The updater belongs in a dedicated `src/update.rs` module. `src/cli.rs` owns
argument types and output contracts, while `src/main.rs` dispatches the
asynchronous operation. It must not be routed through the synchronous
`render_command` helper.

## Installation Source Detection

Detection uses conservative evidence in this order:

1. Native `install.json` exists and the canonical running executable resolves
   through the matching `current` path.
2. The executable path is inside a Homebrew prefix or Cellar.
3. The executable is an npm launcher or is located under an npm global prefix.
4. On Linux, `dpkg -S`, `rpm -qf`, or `pacman -Qo` confirms package ownership.
5. Cargo target and Cargo bin paths are classified as source/Cargo installs.
6. Everything else is `unknown`.

Path shape alone must not authorize overwriting another manager's files. A
native update is permitted only when the native metadata and active executable
agree.

## npm Distribution

npm uses one launcher package plus one native package per supported target:

```text
@yelog/lazydb
@yelog/lazydb-darwin-x64
@yelog/lazydb-darwin-arm64
@yelog/lazydb-linux-x64-gnu
@yelog/lazydb-linux-arm64-gnu
```

The scope and package names must be confirmed as available before
implementation. If the scope cannot be used, a separately approved public name
such as `lazydb-cli` is required.

The main package declares exact-version platform packages as
`optionalDependencies`. A small Node.js launcher selects the installed platform
package, forwards arguments, stdio, signals, and exit status to the native
binary, and reports a clear error when optional dependencies were disabled.
LazyDB itself does not invoke Node.js after the launcher starts the native
process.

Platform packages are assembled from the same verified workflow artifacts used
by GitHub Releases. npm publication must not rebuild LazyDB. Packages declare
`os`, `cpu`, and exact version metadata.

Release tags map to npm dist-tags:

- stable publishes `@yelog/lazydb@latest`;
- Beta publishes `@yelog/lazydb@beta`;
- Beta must never update `latest`.

All platform packages are published before the launcher package. An existing
npm version is never overwritten.

## Beta Identification

Beta is explicit at every user-facing boundary:

- the entry point is `install-beta.sh`;
- installer output begins with `Installing LazyDB BETA channel`;
- success output includes the exact prerelease version and `(BETA)`;
- `lazydb update` reports the active Beta channel;
- GitHub marks the Release as a prerelease;
- npm uses the `beta` dist-tag;
- Homebrew continues to publish stable only in the first implementation.

## Security

- Production downloads use HTTPS and reject unsupported redirects or hosts.
- Every archive is verified against the channel manifest SHA-256 before
  extraction.
- Archive members are validated before extraction; absolute paths, parent
  traversal, links escaping the staging directory, and unexpected layouts fail.
- The extracted binary must report the exact target version through `version
  --json` before activation.
- The updater never invokes `sudo` or writes to package-manager-owned paths.
- Temporary files are private and removed on success, failure, and signals.
- Existing GitHub provenance attestations continue to cover release assets.
- npm publication uses trusted publishing/OIDC where available; otherwise its
  credential is restricted to the protected release environment.
- Pages deployment occurs only after release publication and has minimal Pages
  permissions.

Signed channel manifests and macOS signing/notarization are recommended as a
follow-up supply-chain phase. They are not required to make the first update
implementation correct, but SHA-256 alone does not protect against simultaneous
compromise of release assets and metadata.

## Failure and Recovery

- An update failure never changes `current` or `install.json`.
- An interrupted switch leaves either the old or new complete symlink target,
  never a partially written executable.
- A corrupt cached release directory is removed and reinstalled only after the
  update lock is held.
- Channel metadata is deployed only for a completely published Release.
- Rerunning Pages deployment for an existing tag is idempotent.
- Rerunning npm publication accepts already published packages only when their
  package versions and integrity match; conflicts fail.
- A failed Pages or npm channel does not delete or replace a valid GitHub
  Release.

## Testing

### Installer and Native Update

Fixture HTTP responses cover stable and Beta manifests, all four platform
mappings, explicit channels, malformed metadata, unsupported schema versions,
download failures, checksum mismatch, archive traversal, incorrect binary
versions, lock contention, failed atomic switch, and cleanup.

Temporary installation roots verify first install, repeated install, stable and
Beta channel persistence, explicit channel switching, downgrade refusal, old
version retention, PATH guidance, and non-destructive failures.

### Manager Detection

Fixtures cover native symlinks, Homebrew prefixes, npm launchers and global
prefixes, deb/rpm/Arch ownership command output, Cargo paths, stale native
metadata, and unknown executables. Tests assert that only a confirmed native
installation reaches the binary replacement path.

### npm

Pack and install every platform package in an isolated npm prefix. Verify the
launcher forwards arguments and exit codes, `version --json` reports the exact
package version, missing optional dependencies produce useful diagnostics, and
stable/Beta dist-tags remain isolated.

### Workflow and Pages

Validate workflow syntax, deterministic manifest generation, asset URL and
checksum agreement, preservation of the other channel, Beta branding, npm
package ordering, and the rule that a failed Release cannot update Pages or npm.

## Documentation

`README.md` should lead with the stable native installer, show the Beta command
in a clearly marked subsection, retain Homebrew and native package alternatives,
and add npm as an optional developer install. It should document manager-owned
update commands and explain that `lazydb update` never bypasses the installation
manager.

`docs/releasing.md` should document GitHub Pages custom-domain setup, npm scope
and trusted publishing, channel recovery, Pages deployment, and verification of
all installation paths.

## Acceptance Criteria

- `https://lazydb.yelog.org/install.sh` installs the latest stable release.
- `https://lazydb.yelog.org/install-beta.sh` installs the latest Beta and
  prominently labels it as Beta.
- Native stable and Beta installations persist their selected channel.
- `lazydb update` updates a native installation atomically and preserves the
  previous working version on every failure path.
- `lazydb update --check --json` provides a stable machine-readable result.
- Homebrew, npm, Linux package, Cargo, and unknown installations are never
  overwritten by the native updater.
- Stable and Beta Pages manifests are independently advanced only after the
  corresponding GitHub Release succeeds.
- npm installs the same verified native binary as GitHub Releases and publishes
  stable to `latest` and Beta to `beta`.
- Cargo, Git tag, GitHub Release, Pages manifest, and npm versions agree.
- Beta publication cannot modify Homebrew stable or npm `latest`.
