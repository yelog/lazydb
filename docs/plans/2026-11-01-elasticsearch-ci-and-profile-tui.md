# Elasticsearch CI and Profile TUI Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Run Elasticsearch regression and live acceptance in CI, including creating and editing a secure connection through the real terminal form.

**Architecture:** Keep deterministic Rust tests in the existing PR Rust job. Add an isolated Linux Elasticsearch job using the existing Compose fixture, timeout runner and pinned CI actions. Add a dedicated POSIX PTY form driver alongside the existing console smoke driver; use temporary configuration and only disposable test credentials/resources.

**Tech Stack:** GitHub Actions, Docker Compose, Rust 1.94, Python standard library PTY/HTTP/TLS.

---

### Task 1: Real connection form acceptance

**Files:** Create `scripts/ci/test-elasticsearch-profile-tui.py`; inspect `src/input/keymap.rs`, `src/ui/profiles.rs`, `src/model/profile_manager.rs` and the existing console smoke driver.

1. Start with an empty temporary configuration and updates disabled. Drive the actual binary using keyboard input.
2. Create an Elasticsearch API Key / HTTPS profile, enter encoded key and CA file, test connection and save using local encrypted storage.
3. Restart without `LAZYDB_PASSWORD`; execute a REST read to establish that persisted credentials decrypt and reconnect.
4. Edit authentication to Basic, verify old credential cannot be reused, enter Basic credentials and reconnect.
5. Clear the CA through the form; verify saved metadata no longer contains it and the self-signed endpoint is rejected. Restore CA and verify recovery.
6. Assert credentials are absent from terminal output, ordinary config text and history; redact diagnostics before printing or saving. Invalidate only the temporary API key in a finally block and restore the terminal.
7. Run `python3 -I scripts/ci/test-elasticsearch-profile-tui.py --binary target/debug/lazydb` with the documented TLS URL and CA environment variables against the isolated Compose project. Require all phases to pass.

### Task 2: CI integration

**Files:** Modify `.github/workflows/ci.yml`.

1. Keep deterministic Elasticsearch adapter/profile/policy/console coverage in existing `cargo test --all-targets --all-features` PR checks; document the included coverage in that step.
2. Add an Ubuntu Elasticsearch integration job with Rust cache and bounded build/test commands.
3. Build the binary and live tests before starting the two-node disposable HTTP/HTTPS fixture.
4. Run all ignored Elasticsearch live tests, Basic/HTTP and API Key/HTTPS console smoke checks, and the new profile form smoke check. Missing fixture or credentials must fail the job.
5. Capture bounded fixture diagnostics on failure. Retain only sanitized smoke diagnostics, never config, credential keys, raw transcripts or API responses.
6. Always remove the Compose project's containers, network and volumes.
7. Validate workflow structure with available tooling and execute the same commands locally. Hosted GitHub execution requires a later push and is outside this task's authorization.

### Task 3: Documentation and final checks

**Files:** Modify `docs/elasticsearch.md`.

1. Document CI coverage, profile form smoke command, required fixture variables and cleanup.
2. Run scoped Rust regressions, Python script checks, Compose config check and `git diff --check`.
3. Clean only the dedicated local Compose project and report actual results and limits.

No commit or push: the user's repository instructions prohibit those operations without an explicit request.
