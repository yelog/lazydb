# Releasing LazyDB

LazyDB releases are prepared with the project-level OpenCode `release` skill
and built by `.github/workflows/release.yml`.

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

Use the OpenCode skill with `release beta` or `release stable`. The skill
reviews commits and the actual diff, recommends the next version, and asks for
confirmation before editing. It creates a release commit and annotated tag
only after tests pass and asks separately before pushing.

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
```

The Pages job preserves the other channel manifest while publishing the
channel associated with the newly published Release. It intentionally deploys
only the two manifests, `CNAME`, and the three installer scripts; release
archives are not copied to Pages.

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
releases below `${LAZYDB_CONFIG_HOME:-$HOME/.config/lazydb}`, activates a
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
sh scripts/release/test-channel-manifest.sh
sh scripts/release/test-pages.sh
sh scripts/release/test-installer.sh
```

## First Release Checklist

1. Create and protect `yelog/homebrew-tap`.
2. Configure the `release` Environment and Tap credential.
3. Validate the workflow with a Beta tag.
4. Confirm all four archives, checksums, SBOMs, attestations, and Changelog
   notes are present.
5. Promote the release line with a stable tag and verify native packages,
   installer, and Homebrew installation.
