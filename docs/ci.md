# CI builds and dependency caches

CI uses Rust `1.94.0` through both `RUSTUP_TOOLCHAIN` and explicit toolchain
arguments. `rustup default` alone does not override the repository's rolling
`stable` channel in `rust-toolchain.toml`. Local development keeps that channel;
the workflows pin the effective compiler for repeatable builds.

Both workflows set `CARGO_PROFILE_DEV_DEBUG=0`, `CARGO_PROFILE_TEST_DEBUG=0`,
`CARGO_INCREMENTAL=0`, and `CARGO_BUILD_JOBS=2`. Matching development and test
debug settings lets Clippy and tests reuse native dependency build scripts,
including bundled DuckDB, instead of compiling that C++ code twice. Two Cargo
jobs also allow independent compilation on the hosted runners. These settings
apply to CI; they do not change the published release profile.

The ordinary Rust jobs run formatting, Clippy, and all tests, in that order.
Clippy and tests retain `--all-targets --all-features`. The macOS Rust job first
builds all test binaries with a separate 30-minute compilation budget, then runs
them with the existing 10-minute execution budget. macOS also builds the same
features for its system-library dependency check. Database jobs retain their
live fixtures, test lists, and execution timeouts. Their existing prebuild steps
keep compilation outside the short test execution timeouts.

## Cache ownership

The pinned `Swatinem/rust-cache` action caches Cargo registry/git data and
dependency artifacts under `target`. It removes workspace outputs and unused
dependencies before saving, and does not cache installed Cargo executables.

| Cache family | Writer | Readers |
| --- | --- | --- |
| `ci-ubuntu-24.04` | Main-branch Ubuntu Rust job | Ubuntu Rust job, database job, Release quality job |
| `ci-macos-14` | Main-branch macOS Rust job | macOS Rust job |
| `ci-ubuntu-22.04` | Main-branch MongoDB job | MongoDB job |
| `release-<os>-<target>` | Corresponding macOS/Windows release build | Later builds of the same environment |
| `release-linux-<target>-<image>-<scripts>` | Corresponding Linux release build | Later builds using the same image and scripts |

PRs restore the ordinary CI caches but do not save them. The database and
Release quality jobs only restore the Ubuntu 24.04 cache. Cache writers may save
dependency artifacts after a failure, so an unrelated application lint failure
does not discard a costly native build. Caches are immutable: an exact hit is
not updated. A failed run that saved only partial artifacts can therefore still
require additional compilation on the next run.

The action adds the host OS/architecture, compiler version, Cargo/Rust compiler
environment, manifests, lockfile, and toolchain configuration to these cache
families. Shared readers and writers must keep those inputs consistent. In
particular, adding a `RUST*` or `CARGO*` variable only to Release quality would
split its cache from ordinary CI. Host caches use
`scripts/ci/rust-cache-command.sh` to enumerate only `RUSTUP_TOOLCHAIN`. The
action normally hashes every installed toolchain; unused runner stable versions
were observed to differ even between jobs in the same run. The wrapper keeps
that unrelated variation out of the key while forwarding compiler and Cargo
metadata commands unchanged. Linux release images already contain only their
pinned toolchain and keep using the container command wrapper.

GitHub also scopes caches by ref. PRs can restore eligible base/default-branch
caches; a tag can restore eligible default-branch caches. A new tag cannot
normally restore a cache saved under a previous tag, so release cache reuse is
primarily useful for reruns or manual recovery of the same tag. The workflows do
not depend on a cache being present for correctness. Jobs stay independent: a
cold database job can compile in parallel while the Rust job populates a cache
for later runs.

## Linux release environment

The build remains native for each architecture, using the approved Ubuntu
22.04 image digest and Rust `1.94.0`. The workflow prepares the image before
restoring its dependency cache. Both Rust/cache metadata and compilation then
use `scripts/release/build-linux.sh`:

```sh
sh scripts/release/build-linux.sh x86_64-unknown-linux-gnu --prepare
sh scripts/release/build-linux.sh x86_64-unknown-linux-gnu --exec cargo metadata --all-features --format-version 1
sh scripts/release/build-linux.sh x86_64-unknown-linux-gnu
```

Set `LAZYDB_LINUX_BUILD_IMAGE` to the architecture's exact approved image digest
before these commands. `--prepare` only builds the image; `--exec` runs a command
in the prepared image without building LazyDB. The default mode builds the
image and the release binary. Commands and arguments are passed as separate
arguments, and their exit status propagates to the caller.

The repository and Cargo home have identical absolute paths on the host and
inside the container. Cargo home is writable for registry data and cache locks;
the Rust toolchain comes from the image's `/opt/rustup`. This keeps metadata
paths consistent with the paths the cache action cleans on the host. The Linux
cache family includes the target, base image digest, Dockerfile, and wrapper
script hash. A compiler or build-environment change gets a separate cache.

Native architecture checks, image digest validation, archive inspection, runtime
policy, runtime smoke tests, and macOS dependency checks remain release gates.

## Capacity and measurement

The repository was already near the default 10 GiB Actions cache capacity when
this configuration was introduced. Keep only one writer per ordinary CI family
and inspect actual cache sizes after seeding them. GitHub can evict old entries
under storage pressure; new dependency caches are not guaranteed to all fit.
Review inactive legacy entries and tag caches before manually removing any.

Useful read-only commands:

```sh
gh cache list --repo yelog/lazydb --limit 100
gh run view RUN_ID --repo yelog/lazydb --json jobs
gh run download RUN_ID --repo yelog/lazydb --pattern 'ci-timings-*' --dir ci-timings
```

Validate a cold run and a subsequent warm run with matching toolchain, profile,
lockfile, and runner inputs. The first main-branch run seeds the ordinary CI
caches. Measure later runs after those cache writes complete; jobs in the same
initial run are not guaranteed to see them.

1. Check the cache action's printed key, hit/miss result, archive size, and
   restore/save duration. Ubuntu Rust, database, and Release quality jobs should
   print the same key when their environment and dependency inputs match.
2. Compare total job and compile-step durations. Exclude queue time and record
   runner-image changes when comparing runs.
3. Inspect `target/cargo-timings` HTML reports from the `ci-timings-*` artifacts.
   Check that the expensive DuckDB native build does not repeat between Clippy
   and tests on a cold run, then check how much work remains on a warm run.
4. Confirm the existing test suites and compatibility gates pass. Check for
   memory/disk exhaustion before increasing compilation concurrency further.

Clippy/test timing reports in ordinary Rust CI and build timing reports in the
release matrix are retained for seven days. Release timing artifacts use the
`build-timings-*` prefix so the `release-*` asset download rules cannot publish
them accidentally. Timing reports are diagnostic artifacts, not a compiler
cache. The bundled native compilation appears as part of its build-script time.

Ordinary CI cancels a superseded run of the same branch or PR. Release keeps its
existing non-cancelling concurrency policy for the selected tag.

The [2026-10-21 build measurements](performance/ci-builds.md) record a final cold
run of 26m 19s and a fully warm run of 16m 18s, compared with the earlier
79m 23s baseline. All seven jobs passed; all four dependency-cache readers
reported exact hits in the warm run, with no DuckDB or SQLite recompilation.
The report also covers native x86_64 and ARM64 container cache verification.

Do not infer a speedup from static validation. Actual improvements require
cold/warm cloud measurements. Introduce `sccache` only if those results
still show substantial repeated compilation and its storage/transfer cost is
justified. Supporting DuckDB's C++ compilation also requires a compiler launcher;
setting only `RUSTC_WRAPPER` does not cache that work. Prebuilt build images or
feature changes should be evaluated separately if image preparation or cold
native compilation remains a bottleneck.

## Local validation

```sh
actionlint .github/workflows/ci.yml .github/workflows/release.yml
shellcheck scripts/release/build-linux.sh scripts/release/test-distribution.sh
shellcheck scripts/ci/rust-cache-command.sh
python3 -B scripts/release/test-build-linux.py -v
python3 -B scripts/ci/test-rust-cache-command.py -v
RUSTUP_TOOLCHAIN=1.94.0 sh scripts/release/test-distribution.sh
git diff --check
```

The wrapper suite uses a fake Docker executable and temporary paths, including
paths with spaces, and runs without a daemon or a full Rust build. Run the real
container build and mandatory ELF integration on native Linux. The full local
distribution suite needs Python 3.10 or newer; the Ubuntu CI runners provide it.
