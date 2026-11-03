# Releasing LazyDB

LazyDB releases are prepared with the project-level `release` skill in Codex
or OpenCode and built by `.github/workflows/release.yml`.

- Codex: [`.agents/skills/release/SKILL.md`](../.agents/skills/release/SKILL.md),
  invoked with `$release beta` or `$release stable` from this repository.
- OpenCode: [`.opencode/skills/release/SKILL.md`](../.opencode/skills/release/SKILL.md),
  invoked with `release beta` or `release stable`.

See [CI builds and dependency caches](ci.md) for the pinned toolchain, build
profiles, cache ownership, and build timing diagnostics used by both workflows.

## Prerequisites

- Work from an up-to-date `main` checkout with no unrelated changes.
- Rust `1.94` and Cargo are installed locally.
- GitHub-hosted runners must provide all four target architectures, including
  the ARM64 Linux runner used by the release matrix.
- The `yelog/lazydb` remote is configured and tags have been fetched.
- GitHub Actions has access to the protected `release` Environment.
- GitHub Pages is configured to deploy with GitHub Actions from this repository.
- DNS has a CNAME from `lazydb.yelog.org` to `yelog.github.io`.
- Stable Homebrew publication requires the external `yelog/homebrew-tap`
  repository and a credential scoped only to that repository.
- Local validation tools include `cargo`, `git`, `awk`, `grep`, `tar`, and a
  SHA-256 utility. CI additionally uses `gh`, nFPM, `actionlint`, and
  ShellCheck.

## Version and Channels

Supported tags are:

```text
vMAJOR.MINOR.PATCH
vMAJOR.MINOR.PATCH-beta.N
```

Use the release skill with the desired channel as shown above. The skill
reviews commits and the actual diff, recommends the next version, and asks for
one confirmation before editing. This prompt explicitly includes the release
scope: update Changelog and Cargo versions, run all checks, create a release
commit and annotated tag, push `main` and the tag, then wait for CI, Release,
and Pages verification. Confirming the version authorizes that whole scope;
there are no separate commit or push questions. A valid version override in
response to the prompt authorizes the same scope for the selected version.
Clear affirmative replies in the maintainer's language are accepted.

For a non-publishing rehearsal, explicitly request **prepare only, no commit,
tag, or push** before confirming. To permit a local commit and tag but no
publication, explicitly request **no push**. Analysis-only requests never edit.

New releases require a clean worktree and index. Preparation changes only
`CHANGELOG.md`, `Cargo.toml`, and `Cargo.lock`; it does not bundle existing work
or modify application code and tests to make validation pass. Failed checks,
unexpected source/remote changes, and conflicting tags stop the workflow with
an exact state report. No force-push, tag replacement, or automatic republication
is allowed. Interrupted runs verify Git/GitHub state and resume the same
authorized release without repeating completed operations or routine questions.

Agent tool permissions, branch protection, and GitHub Environment reviewers
are independent controls. The skill does not bypass or change them. If they
block progress, it reports the required external action rather than claiming
the release is complete. Completion requires all required CI, Release, and
Pages jobs, including online installation checks for the exact version.

Beta notes use the previous Beta for the same version line as their baseline.
Stable notes use the previous stable tag, including changes already described
by Betas. Every selected commit appears in the `Commits` section of the
generated `CHANGELOG.md` entry.

## GitHub Workflow

Pushing a valid `v*` tag starts the normal release path. `workflow_dispatch`
accepts an existing tag only and is intended for retrying a failed publication.
It cannot create a new version. The workflow rejects tags that are not reachable
from `main`, versions that disagree with Cargo or the Changelog, incomplete
target matrices, checksum mismatches, and failed smoke tests.

The workflow creates a draft Release, uploads and verifies all assets, then
publishes it. Beta Releases are prereleases. Stable Releases additionally
produce Linux native packages, a shell installer, and a Homebrew Formula.

## Pages Custom Domain

In the repository's **Settings > Pages**, set the source to **GitHub Actions**
and set the custom domain to `lazydb.yelog.org`. The repository contains
`pages/CNAME` with that exact hostname; the Pages workflow copies it into the
deployed artifact. At the DNS provider, create a CNAME record for
`lazydb.yelog.org` pointing to `yelog.github.io`, then wait for GitHub to issue
and enforce HTTPS. Verify the deployment and both channel documents:

```bash
curl --fail --proto '=https' --tlsv1.2 https://lazydb.yelog.org/CNAME
curl --fail --proto '=https' --tlsv1.2 https://lazydb.yelog.org/channels/stable.json
curl --fail --proto '=https' --tlsv1.2 https://lazydb.yelog.org/channels/beta.json
curl --fail --proto '=https' --tlsv1.2 https://lazydb.yelog.org/channels/v2/stable.json
curl --fail --proto '=https' --tlsv1.2 https://lazydb.yelog.org/channels/v2/beta.json
```

Linux release assets include per-architecture runtime reports and schema 2
channel requirements. The report and package checks are described in
[Linux runtime compatibility](linux-runtime-compatibility.md).

Every release target also reports the raw client binary and packaged archive
byte sizes and SHA-256 digests. The artifact gate rejects DuckDB C API imports
or engine payload in the default client archive; the optional DuckDB runtime is
prepared separately for native CI tests and is not copied into release archives.

The Pages job preserves the other channel manifest while publishing the
channel associated with the newly published Release. It intentionally deploys
only the channel manifests, `CNAME`, and installer scripts (including the generated
`install.ps1.txt` text entry point); release
archives are not copied to Pages.

The Windows installer is checked in both Windows PowerShell 5.1 and PowerShell
7. The README starts an isolated Windows PowerShell process and pipes the
`install.ps1.txt` text response to `Invoke-Expression`. Pages assembly generates
that endpoint byte-for-byte from `pages/install.ps1`; do not maintain a second
source file. The `.txt` suffix lets the static host serve `text/plain` instead of
`application/octet-stream`. A download-to-file alternative remains in the README.
Both methods trust the remote installer, not just its release archive checksum.
The installer writes its temporary archive with a `.zip`
extension and writes `install.json` as UTF-8 without a BOM for compatibility
with the Rust JSON parser. The Pages workflow runs the same installation against
the public endpoint after deployment, using isolated `windows-2022` runners
and explicit `powershell` and `pwsh` steps. The smoke test downloads
both endpoints for either channel and compares their SHA-256 with the checked-out
`pages/install.ps1` **before execution**, also requiring `text/plain` on the text
endpoint. It then executes the exact README command payload in each host. A guard
around the real `Invoke-RestMethod` checks that the decoded response is non-empty
text and matches the source hash before passing it to `iex`; it never substitutes
local content. A stale deployment fails verification even if it could install the
requested release. Each child uses the same executable as the smoke-test host,
so the PowerShell 7 check does not silently run Windows PowerShell 5.1 instead.
Downloads and installer execution
have timeouts; failed attempts retry up to six times.

The old `irm .../install.ps1 | iex` bootstrap remains unsupported: it requests
the binary MIME endpoint. Use the README's `.ps1.txt` command or its download-to-file
alternative. Deploy the generated text endpoint before advertising the new command;
the post-deployment Windows checks validate its actual HTTP decoding behavior.

### Redeploying Installer Fixes

Installer/workflow-only fixes do not require replacing a release tag or
rebuilding its assets. After the fixes are reviewed and published on `main`,
manually dispatch **Pages**, selecting `main` as the workflow ref and an
existing published release tag as the `tag` input:

```bash
gh workflow run pages.yml --repo yelog/lazydb --ref main -f tag=vVERSION
gh run list --repo yelog/lazydb --workflow pages.yml --limit 5
gh run watch RUN_ID --repo yelog/lazydb --exit-status
```

The selected ref supplies the installer scripts and verification code; the tag
selects the existing release assets and channel version. Automatic runs after
Release instead check out the Release run's head SHA. Confirm that `assemble`,
`deploy`, both POSIX verification jobs, and both Windows-host verification jobs
all pass. Deployment success alone is not installation verification. If the
SHA-256 check fails, inspect the selected source ref and public script before
retrying; do not remove the comparison to accommodate stale content. No DNS or
Cloudflare change is needed for a script-only redeployment.

## Assets

Binary targets are:

- `x86_64-apple-darwin`
- `aarch64-apple-darwin`
- `x86_64-unknown-linux-gnu`
- `aarch64-unknown-linux-gnu`

Stable Linux assets include `.deb`, `.rpm`, and `.pkg.tar.zst` packages. These
are direct Release assets and do not create `apt`, DNF, or Pacman repositories.
Consequently, `apt install lazydb`, `dnf install lazydb`, and `pacman -S
lazydb` are not supported by this first release implementation.

## Channel Manifests

The Pages channel documents are served from
`https://lazydb.yelog.org/channels/stable.json` and
`https://lazydb.yelog.org/channels/beta.json`. Each is schema `1` JSON
containing `product: "lazydb"`,
the channel, version, matching `vVERSION` tag, `prerelease` flag, publication
timestamp, GitHub `release_url`, and exactly one asset entry for each supported
target. Every asset has an HTTPS GitHub Release download URL and a 64-character
lowercase SHA-256 digest. The manifest is generated from the published
`SHA256SUMS`; it does not duplicate archive data or call the GitHub API.

The canonical native installer stores configuration, installation state, and
releases below `${LAZYDB_CONFIG_HOME:-$HOME/lazydb}`, activates a
`current` symlink, and links the requested executable directory to that
activation. It records channel,
target, version, and manager ownership in `install.json`. `--version` selects a
specific version only when it matches the selected channel manifest; it does not
permit an installer to bypass manifest validation. `LAZYDB_CHANNEL_BASE_URL` is
reserved for local fixture tests and should not be set in production.

Stable manifests use stable versions and tags; Beta manifests use
`VERSION-beta.N` versions and prerelease tags. A publication updates only its
own channel document, and manifests are published only after the matching
GitHub Release assets pass the exact-name and checksum checks.

## Release Order

The release workflow uses this order: validate the tag and source, run the
quality checks, build all four archives, build stable Linux packages when
applicable, publish the GitHub Release, update the stable Homebrew tap, and
finally deploy Pages from the published Release. Beta releases skip native
Linux packages, Homebrew, and the stable installer while still publishing the
beta Pages manifest. npm distribution is future optional work and is not part
 of the current release workflow.

Do not publish a channel manifest before its matching GitHub Release assets are
public and checksum-verified. Do not treat Homebrew or Pages publication
as a rebuild step: each consumes the already verified Release output.

## Recovery

If build or validation fails, fix the source or workflow and create a new
commit. Never replace an existing tag. If Homebrew fails after a stable GitHub
Release is public, preserve the Release, fix the Tap or credentials, and rerun
the existing-tag workflow. Do not issue a replacement tag just to retry a
channel publication.

For a failed or interrupted publication, inspect the existing tag and Release
before retrying:

```bash
git fetch --tags origin
gh release view vVERSION --repo yelog/lazydb --json isDraft,isPrerelease,assets,url
gh run list --repo yelog/lazydb --workflow release.yml --limit 10
gh workflow run release.yml --repo yelog/lazydb -f tag=vVERSION
```

The retry input must name the existing tag. Do not create a second tag for the
same version. A failed Pages deployment can be retried after the Release is
published; a failed Homebrew publication can likewise be repaired and
rerun without rebuilding or replacing verified Release assets. If a source or
workflow defect caused the failure, fix it in a new commit and use a new
version.

## Exact Verification

Run these checks against a published stable version, replacing `VERSION` and
`ARCH` with the asset names under test:

```bash
gh release view vVERSION --repo yelog/lazydb --json url,assets
curl --fail --proto '=https' --tlsv1.2 https://lazydb.yelog.org/install.sh -o /tmp/lazydb-installer.sh
sh -n /tmp/lazydb-installer.sh
curl --fail --proto '=https' --tlsv1.2 https://lazydb.yelog.org/channels/stable.json
curl --fail --proto '=https' --tlsv1.2 https://lazydb.yelog.org/channels/beta.json
gh release download vVERSION --repo yelog/lazydb --pattern 'lazydb_VERSION_ARCH.tar.xz' --pattern SHA256SUMS
shasum -a 256 -c SHA256SUMS
```

For repository-side validation before tagging, run:

```bash
git diff --check
sh scripts/release/test-distribution.sh
actionlint
```

On an isolated Windows account, run the behavioral fixtures with both supported
PowerShell hosts. They use real ZIP files, SHA-256 hashing, extraction, compiled
executables, and installation state; only HTTP downloads are mocked with exact
URL mappings. They also execute the PowerShell bootstrap extracted from README.
Failure fixtures verify old-install preservation and temporary-file cleanup.
Success fixtures verify repeated installation, empty/repeated user PATH entries,
literal bracket paths, and BOM-free JSON.

Both Windows test scripts require explicit permission to mutate user PATH.
They restore the saved nullable user PATH and overridden process environment in
`finally`, including on failure. Do not run them concurrently on a shared user
account: restoration could overwrite another process's concurrent PATH change.

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File scripts/release/test-windows-installer.ps1 -AllowUserPathMutation
pwsh -NoProfile -File scripts/release/test-windows-installer.ps1 -AllowUserPathMutation
```

After Pages deployment, the Windows online smoke test is run by CI for both
hosts. To reproduce it manually, use the channel and version selected by the
Pages workflow and a checkout of the exact installer source deployed by it:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File scripts/release/smoke-online-install.ps1 -Channel stable -Version VERSION -AllowUserPathMutation
pwsh -NoProfile -File scripts/release/smoke-online-install.ps1 -Channel stable -Version VERSION -AllowUserPathMutation
```

## First Release Checklist

1. Create and protect `yelog/homebrew-tap`.
2. Configure the `release` Environment and Tap credential.
3. Validate the workflow with a Beta tag.
4. Confirm all four archives, checksums, SBOMs, attestations, and Changelog
   notes are present.
5. Promote the release line with a stable tag and verify native packages,
   installer, and Homebrew installation.
