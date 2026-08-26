# Connection Management Optimization Implementation Plan

> **For Claude:** Implement this plan task-by-task, preserving password redaction and existing adapter contracts.

**Goal:** Fix restart credential handling and deliver horizontal driver selection, default-schema visibility, and safe bidirectional connection URL editing.

**Architecture:** Persist explicit credential intent and URL format while retaining structured profile fields as the source of truth. Keep catalog visibility in `CatalogScope`, and implement URL synchronization as explicit one-way state transitions in `ProfileDraft`.

**Tech Stack:** Rust 2024, Tokio, Ratatui, Crossterm, Serde/TOML, `url`, `secrecy`, SQLx.

---

### Task 1: Credential Policy And Profile Migration

**Files:**
- Modify: `src/profile.rs`
- Modify: `src/persistence/profiles.rs`
- Modify: `src/persistence/secrets.rs`
- Modify: `src/model/profile_manager.rs`
- Modify: `src/runtime.rs`
- Test: `tests/persistence.rs`
- Test: `tests/profile_draft.rs`
- Test: `tests/profile_runtime.rs`

**Steps:**

1. Add failing round-trip and migration tests for passwordless, prompt, and keyring policies.
2. Add a restart test proving a prompt profile emits `CredentialsRequired` without attempting an empty-password connection.
3. Add `CredentialPolicy` and migrate profile persistence from version 2 to version 3.
4. Map draft credential updates to explicit persisted policy in the save transaction.
5. Resolve missing prompt credentials as an actionable error while retaining passwordless behavior.
6. Run `cargo test --test persistence --test profile_draft --test profile_runtime`.

### Task 2: Driver Rendering And Navigation

**Files:**
- Modify: `src/model/profile_manager.rs`
- Modify: `src/action.rs`
- Modify: `src/app.rs`
- Modify: `src/input/keymap.rs`
- Modify: `src/input/mouse.rs`
- Modify: `src/ui/mod.rs`
- Modify: `src/ui/profiles.rs`
- Test: `tests/keymap.rs`
- Test: `tests/mouse.rs`
- Test: `tests/profile_reducer.rs`
- Test: `tests/ui_render.rs`

**Steps:**

1. Add failing keymap tests for horizontal driver changes and vertical field navigation.
2. Add failing render tests requiring all drivers and a persistent selected style.
3. Add a shared driver order and direct kind-selection reducer action.
4. Render driver options horizontally with option-level mouse regions.
5. Standardize Up/Down and Tab navigation while preserving literal text input.
6. Support both Crossterm Shift-Tab event forms.
7. Run `cargo test --test keymap --test mouse --test profile_reducer --test ui_render`.

### Task 3: Default Schema And Derived Scope

**Files:**
- Modify: `src/model/profile_manager.rs`
- Modify: `src/app.rs`
- Test: `tests/catalog_scope.rs`
- Test: `tests/profile_draft.rs`
- Test: `tests/sql_completion.rs`

**Steps:**

1. Add tests requiring Schema in PostgreSQL fields but not MySQL fields.
2. Add tests for derived scope following database/schema edits and explicit scope preservation.
3. Track derived versus explicit scope in `ProfileDraft`.
4. Regenerate only derived scopes when relevant connection fields change.
5. Pass the active default schema into completion ranking.
6. Run `cargo test --test catalog_scope --test profile_draft --test sql_completion`.

### Task 4: URL Parser And Formatter

**Files:**
- Modify: `src/profile.rs`
- Modify: `src/security.rs`
- Create: `tests/profile_url.rs`
- Modify: `tests/startup_profiles.rs`

**Steps:**

1. Add parser and formatter tests for PostgreSQL, MySQL, SQLite, and JDBC forms.
2. Add tests for percent-encoded credentials, IPv6, Unicode, SSL/schema settings, and unsupported parameters.
3. Add `ConnectionUrlFormat`, parsed settings, and redacted error types.
4. Refactor `import_connection_url` to use the shared parser.
5. Implement password-omitting canonical formatting.
6. Run `cargo test --test profile_url --test startup_profiles`.

### Task 5: URL Form Synchronization

**Files:**
- Modify: `src/model/profile_manager.rs`
- Modify: `src/input/keymap.rs`
- Modify: `src/ui/profiles.rs`
- Modify: `src/app.rs`
- Test: `tests/profile_draft.rs`
- Test: `tests/profile_reducer.rs`
- Test: `tests/ui_render.rs`

**Steps:**

1. Add tests for atomic URL application, failed parsing, field-to-URL projection, and keyring preservation.
2. Add secret-backed URL input and URL synchronization state.
3. Add URL and URL Format fields to driver-specific field lists.
4. Submit pending URL before Test, Save, and Save & Connect.
5. Regenerate URL after structured field edits without reparsing.
6. Render only password-free URLs and sanitize errors.
7. Run the focused profile tests.

### Task 6: Verification And Documentation

**Files:**
- Modify: `README.md`
- Modify: `docs/configuration.md`
- Modify: `docs/keybindings.md`
- Modify: `docs/architecture.md`

**Steps:**

1. Document credential policies, URL formats, schema visibility, and navigation.
2. Run `cargo fmt --check` and format if required.
3. Run `cargo test`.
4. Run `cargo clippy --all-targets --all-features -- -D warnings`.
5. Review changed files for password leakage and unintended behavior changes.
