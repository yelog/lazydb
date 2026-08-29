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

## Recovery

If build or validation fails, fix the source or workflow and create a new
commit. Never replace an existing tag. If Homebrew fails after a stable GitHub
Release is public, preserve the Release, fix the Tap or credentials, and rerun
the existing-tag workflow. Do not issue a replacement tag just to retry a
channel publication.

## First Release Checklist

1. Create and protect `yelog/homebrew-tap`.
2. Configure the `release` Environment and Tap credential.
3. Validate the workflow with a Beta tag.
4. Confirm all four archives, checksums, SBOMs, attestations, and Changelog
   notes are present.
5. Promote the release line with a stable tag and verify native packages,
   installer, and Homebrew installation.
