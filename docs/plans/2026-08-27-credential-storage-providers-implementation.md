# Credential Storage Providers Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Persist saved passwords by default using local authenticated encryption, expose detected native credential stores as optional platform-specific backends, and fallback explicitly to Local when a System write fails.

**Architecture:** Add a LazyDB-owned credential-storage boundary with Local and System implementations. Persist encrypted Local payloads in profile version 4, store the Local key separately, probe System capability asynchronously without touching secrets, and keep profile saves compensating and atomic.

**Tech Stack:** Rust 2024, serde/TOML, secrecy, XChaCha20-Poly1305, OS CSPRNG, keyring-core platform stores, Tokio, Ratatui

---

### Task 1: Introduce Version 4 Credential Types

**Files:**
- Modify: `src/profile.rs:207-248`
- Modify: `src/model/profile_manager.rs`
- Modify: `src/persistence/profiles.rs:17-143`
- Test: `tests/persistence.rs`
- Test: `tests/profile_draft.rs`

**Steps:**

1. Add failing serialization tests for `None`, `Prompt`, `LocalEncrypted`, and `System` credential states.
2. Add a failing version 3 migration test proving `Keyring(reference)` becomes `System(reference)` without reading a secret store.
3. Replace `CredentialPolicy` with a persisted storage enum and an encrypted payload containing `version`, encoded `nonce`, and encoded `ciphertext`.
4. Add `PasswordStorageChoice::{LocalEncrypted, System}` to the draft model and remove the `remember_password` boolean and `RememberPassword` field.
5. Preserve the current credential when editing with an empty password; keep an explicit update variant for clearing.
6. Advance the profile file version to 4 and retain strict version 2 and 3 migration paths.
7. Run `cargo test --test persistence --test profile_draft --all-features` and expect all tests to pass.

### Task 2: Implement the Local Credential Key

**Files:**
- Create: `src/persistence/local_credentials.rs`
- Modify: `src/persistence/mod.rs`
- Modify: `src/persistence/paths.rs`
- Modify: `Cargo.toml`
- Test: `tests/local_credentials.rs`

**Steps:**

1. Add the selected AEAD and encoding dependencies with minimal features. Prefer `chacha20poly1305` with XChaCha20 support and a maintained Base64 crate.
2. Add failing tests for atomic key creation, `0600` Unix mode, stable reload, invalid-size rejection, and refusal to replace an invalid existing key.
3. Add a platform configuration path for `credential.key` beside, but not inside, `connections.toml`.
4. Implement `LocalCredentialKeyStore` using an OS CSPRNG, `create_new(true)`, private permissions, `sync_all()`, and bounded/versioned key decoding.
5. Handle concurrent first creation by reopening the winning file rather than overwriting it.
6. Run `cargo test --test local_credentials --all-features` and expect all tests to pass.

### Task 3: Implement Authenticated Local Encryption

**Files:**
- Create: `src/persistence/credential_cipher.rs`
- Modify: `src/persistence/mod.rs`
- Test: `tests/local_credentials.rs`

**Steps:**

1. Add failing round-trip and non-determinism tests for the same profile/password.
2. Add failing tests for wrong profile UUID, modified ciphertext, modified nonce, unsupported version, and wrong key.
3. Implement XChaCha20-Poly1305 with a fresh random nonce and associated data containing `lazydb-credential-v1`, the service identifier, and canonical profile UUID.
4. Convert encoded payloads into bounded errors that never include secret bytes.
5. Ensure ciphertext and key types have redacted `Debug` behavior.
6. Run `cargo test --test local_credentials --all-features` and expect all tests to pass.

### Task 4: Define System Provider Capability

**Files:**
- Refactor: `src/persistence/secrets.rs`
- Modify: `src/action.rs`
- Modify: `src/runtime.rs`
- Modify: `Cargo.toml`
- Test: `tests/secret_store.rs`
- Test: `tests/profile_runtime.rs`

**Steps:**

1. Add `SystemCredentialProvider::{MacOsLoginKeychain, FreedesktopSecretService}` and structured availability states for Checking, Available, Locked, and Unavailable reasons.
2. Extend the fake secret provider with an operation log and capability result.
3. Add failing tests proving capability probing invokes no secret get/set/delete operations.
4. Replace the coarse `keyring::v1` boundary with target-specific adapters or direct probes behind a LazyDB-owned trait.
5. On macOS, identify the default user/login Keychain without reading a LazyDB item.
6. On Linux, distinguish missing session bus, missing `org.freedesktop.secrets`, missing default collection, and locked collection without creating an item.
7. Emit a generation-safe capability action from Runtime and reject stale probe results in App.
8. Cache stable capability results for the process lifetime; do not persist them.
9. Run `cargo test --test secret_store --test profile_runtime --all-features` and expect all tests to pass on the current platform, with other platform adapters covered by pure mapping tests and CI.

### Task 5: Render Dynamic Password Storage Choices

**Files:**
- Modify: `src/model/profile_manager.rs`
- Modify: `src/ui/profiles.rs`
- Modify: `src/input/keymap.rs`
- Modify: `src/input/mouse.rs`
- Modify: `src/ui/mod.rs`
- Test: `tests/profile_draft.rs`
- Test: `tests/profile_reducer.rs`
- Test: `tests/ui_render.rs`
- Test: `tests/mouse.rs`

**Steps:**

1. Add failing draft/render tests proving Local is the default for new PostgreSQL/MySQL profiles and SQLite hides password storage.
2. Add tests for platform labels `MACOS LOGIN KEYCHAIN` and `SECRET SERVICE`.
3. Add tests that Available and Locked System providers are selectable while unavailable providers are hidden for new/Local profiles.
4. Add tests that an existing System profile remains visible as `CURRENT, UNAVAILABLE` and is preserved by an unchanged save.
5. Remove the Remember Password toggle and replace it with a cycle/select storage field.
6. Start capability probing when Profile Manager first needs provider status; keep Local immediately usable during Checking.
7. Update keyboard and mouse hit targets for the new field.
8. Add an explicit clear-password action rather than interpreting an empty edit field as deletion.
9. Run `cargo test --test profile_draft --test profile_reducer --test ui_render --test mouse --all-features` and expect all tests to pass.

### Task 6: Save Local Credentials Transactionally

**Files:**
- Modify: `src/runtime.rs:1249-1571`
- Modify: `src/persistence/profiles.rs`
- Test: `tests/profile_runtime.rs`

**Steps:**

1. Add failing tests for new Local save, Local replacement, unchanged preservation, explicit clear, and profile deletion.
2. Add failure-injection tests for missing/invalid local key, encryption failure, and profile persistence failure.
3. Encrypt before constructing the profile snapshot, then atomically persist the profile and update session cache only after success.
4. Keep prior encrypted state and cache when persistence fails.
5. On successful Local read, insert the plaintext into `session_secrets` for the process lifetime.
6. Run `cargo test --test profile_runtime --all-features` and expect all tests to pass.

### Task 7: Implement System Save and Explicit Local Fallback

**Files:**
- Modify: `src/runtime.rs`
- Test: `tests/profile_runtime.rs`

**Steps:**

1. Add failing tests for successful System save, System write failure followed by Local success, and both stores failing.
2. Assert fallback persists `LocalEncrypted`, updates the session cache, and returns a warning naming the failed provider.
3. Add tests proving System read failure requests credentials and does not claim a Local fallback.
4. Preserve the existing compensation behavior when profile persistence fails after a System mutation.
5. Add transition tests for Local-to-System and System-to-Local, including stale System deletion warnings.
6. Ensure System and Local reads both populate `session_secrets`, so reconnects perform no second backend read.
7. Run `cargo test --test profile_runtime --test connection_switch --all-features` and expect all tests to pass.

### Task 8: Update Diagnostics and Documentation

**Files:**
- Modify: `src/cli.rs`
- Modify: `docs/configuration.md`
- Modify: `docs/architecture.md`
- Modify: `README.md`
- Test: CLI capability/doctor tests in `src/cli.rs` or the existing integration test location

**Steps:**

1. Add non-sensitive doctor fields for provider name, status, and bounded reason.
2. Document the Local threat model, key location, backup implications, and lost-key recovery.
3. Document platform detection, Locked visibility, System write fallback, and the absence of hidden Local duplicates for System profiles.
4. Document that copying only `connections.toml` does not migrate Local passwords because `credential.key` remains device-local.
5. Remove outdated Remember Password and session-only fallback language.
6. Run the focused CLI and documentation tests.

### Task 9: Complete Cross-Platform Verification

**Files:**
- Verify all modified files
- Modify if required: `.github/workflows/ci.yml`

**Steps:**

1. Run `cargo fmt --all -- --check`.
2. Run `cargo test --all-features --all-targets`.
3. Run `cargo clippy --all-features --all-targets -- -D warnings`.
4. Run `git diff --check` and inspect the complete diff.
5. Ensure macOS CI compiles and tests the Apple adapter and Ubuntu CI compiles and tests the Secret Service adapter without requiring a live desktop service.
6. Manually verify macOS with Available and Locked login Keychain states.
7. Manually verify Ubuntu Desktop with Secret Service, Ubuntu/Rocky headless without a session bus, and an existing System profile opened from an unavailable environment.
8. Verify no test fixture, Debug representation, log, doctor output, or generated TOML exposes plaintext passwords or local-key bytes.

Do not commit implementation changes until explicitly requested. Preserve unrelated worktree changes in profile, UI, database, and plan files.
