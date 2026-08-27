# Default Password Persistence Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make passwords entered for new server connections persist securely across restarts by default.

**Architecture:** Keep the existing `CredentialUpdate` and native `SecretStore` flow. Change only draft defaults so a new password selects `Remember`, then add reducer/runtime tests proving the intent and restart resolution while retaining the existing unavailable-store fallback.

**Tech Stack:** Rust 2024, Tokio, secrecy, keyring, TOML, cargo test

---

### Task 1: Default Server Drafts to Remember Passwords

**Files:**
- Modify: `tests/profile_draft.rs`
- Modify: `src/model/profile_manager.rs:325-365`

**Step 1: Write the failing tests**

Update draft-default tests to assert that PostgreSQL and MySQL drafts enable
`remember_password`, while SQLite does not. Update the credential-intent test to
show that a new password uses `CredentialUpdate::Remember` by default and becomes
`CredentialUpdate::Session` after explicitly disabling the option.

**Step 2: Run tests to verify they fail**

Run: `cargo test --test profile_draft new_postgres_uses_server_defaults new_mysql_and_sqlite_use_driver_defaults new_password_uses_session_or_remember_intent`

Expected: the PostgreSQL/MySQL remember-default assertions fail and the default
credential intent is `Session`.

**Step 3: Implement the minimal default**

In `ProfileDraft::new`, initialize `remember_password` according to database kind:

```rust
remember_password: kind != DatabaseKind::Sqlite,
```

No runtime or persistence changes are required because the existing validation
path maps a non-empty password plus this setting to `CredentialUpdate::Remember`.

**Step 4: Run the focused tests**

Run: `cargo test --test profile_draft`

Expected: PASS.

### Task 2: Default Prompt Credential Repair to Remember

**Files:**
- Modify: `tests/profile_draft.rs`
- Modify: `src/model/profile_manager.rs:367-432`

**Step 1: Write the failing test**

Create a profile with `CredentialPolicy::Prompt`, open it through
`ProfileDraft::edit`, and assert `remember_password` is enabled. Also assert that
an existing `CredentialPolicy::None` profile remains disabled when edited
manually, so passwordless profiles do not silently change credential policy.

**Step 2: Run the test to verify it fails**

Run: `cargo test --test profile_draft editing_prompt_profile_defaults_to_remembering_replacement_password`

Expected: FAIL because edit currently enables the option only for keyring
profiles.

**Step 3: Implement the repair default**

Initialize edited server drafts as follows:

```rust
remember_password: matches!(
    profile.credential_policy,
    CredentialPolicy::Prompt | CredentialPolicy::Keyring(_)
),
```

This preserves passwordless `None` profiles and ensures the automatically opened
prompt-repair form remembers a newly entered password unless the user opts out.

**Step 4: Run focused tests**

Run: `cargo test --test profile_draft`

Expected: PASS.

### Task 3: Verify Remembered Password Resolution After Runtime Reconstruction

**Files:**
- Modify: `tests/profile_runtime.rs`

**Step 1: Add a restart regression test**

Use one shared `FakeSecretStore`. Save a profile with
`CredentialUpdate::Remember`, wait for `Action::ProfileSaved`, shut down the first
Runtime, load the persisted profile from `ProfileStore`, create a second Runtime
with the same secret store, and dispatch a credential-resolving operation. Assert
that the operation proceeds past credential resolution and does not emit
`Action::CredentialsRequired`.

Prefer a deterministic fake/database boundary already available in this test
module. If a real PostgreSQL connection would be required, assert a normal
`ConnectionFailed` from the database attempt rather than `CredentialsRequired`;
the distinction proves the stored password was resolved without exposing it.

**Step 2: Run the regression test**

Run: `cargo test --test profile_runtime remembered_password_is_resolved_after_runtime_reconstruction`

Expected: PASS. The existing runtime implementation should already satisfy this
test; it protects the behavior against future regressions.

**Step 3: Run all credential persistence tests**

Run: `cargo test --test profile_runtime`

Expected: PASS, including native-store unavailable downgrade, rollback, update,
and delete behavior.

### Task 4: Update Documentation and Verify the Project

**Files:**
- Modify: `docs/configuration.md:76-83`

**Step 1: Document the default**

State that new PostgreSQL/MySQL forms enable `Remember Password` by default, that
users can disable it for a session-only password, and that unavailable native
storage still downgrades to `prompt` with a warning.

**Step 2: Format and run relevant checks**

Run: `cargo fmt --check`

Expected: PASS.

Run: `cargo test --test profile_draft --test profile_runtime --test startup_profiles --test secret_store`

Expected: PASS.

Run: `cargo test`

Expected: PASS.

Run: `cargo clippy --all-targets --all-features -- -D warnings`

Expected: PASS.

No commit is included unless explicitly requested by the user.
