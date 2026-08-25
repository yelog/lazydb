# Dynamic Profile Manager Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a runtime Profile Manager that safely creates, tests, persists, edits, deletes, and switches PostgreSQL, MySQL, and SQLite connections with optional native-keyring password storage.

**Architecture:** Keep the existing `Action -> App::update -> Command -> Runtime` boundary. Add pure profile-form state to `App`, put profile persistence and an injectable `SecretStore` behind Runtime, and tag all profile and database operations with generations so stale work cannot mutate current state or execute on the wrong pool.

**Tech Stack:** Rust 1.94, Ratatui, Crossterm, Tokio, SQLx concrete drivers, `secrecy`, `keyring` 4.x, TOML, tempfile, Neovim Lua tests.

---

## Implementation Rules

- Follow TDD for each task: add one focused failing test, run it, implement the minimum behavior, and rerun the focused suite.
- Never place a password in a URL, persisted profile, output entry, tracing field, panic message, or derived `Debug` implementation.
- Keep `ConnectionProfile` serializable and secret-free. Pass credentials separately as `SecretString`.
- Do not let UI code call SQLx, `ProfileStore`, or the native keyring.
- Do not assume a desktop keyring daemon exists in CI or headless Linux.
- Do not execute a query unless its expected profile ID and connection generation match Runtime's active pool.
- Keep existing CLI and Neovim startup contracts compatible.
- The commit steps below are logical checkpoints. Execute them only when commit authorization is present.

### Task 1: Add the Native Secret Store Boundary

**Files:**
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Create: `src/persistence/secrets.rs`
- Modify: `src/persistence/mod.rs`
- Create: `tests/secret_store.rs`

**Step 1: Write failing secret-reference and fake-store tests**

Add tests that establish the public boundary without touching the real OS store:

```rust
use std::{collections::HashMap, sync::Mutex};

use async_trait::async_trait;
use lazydb::persistence::secrets::{
    SecretStore, SecretStoreError, keyring_ref, profile_id_from_ref,
};
use secrecy::{ExposeSecret, SecretString};
use uuid::Uuid;

#[test]
fn keyring_references_round_trip_profile_ids() {
    let id = Uuid::new_v4();
    let reference = keyring_ref(id);
    assert_eq!(profile_id_from_ref(&reference).unwrap(), id);
    assert!(profile_id_from_ref("env:password").is_err());
}

#[derive(Default)]
struct FakeSecretStore {
    values: Mutex<HashMap<Uuid, SecretString>>,
}

#[async_trait]
impl SecretStore for FakeSecretStore {
    async fn available(&self) -> Result<(), SecretStoreError> { Ok(()) }
    async fn get(&self, id: Uuid) -> Result<Option<SecretString>, SecretStoreError> {
        Ok(self.values.lock().unwrap().get(&id).cloned())
    }
    async fn set(&self, id: Uuid, value: &SecretString) -> Result<(), SecretStoreError> {
        self.values.lock().unwrap().insert(id, value.clone());
        Ok(())
    }
    async fn delete(&self, id: Uuid) -> Result<(), SecretStoreError> {
        self.values.lock().unwrap().remove(&id);
        Ok(())
    }
}

#[tokio::test]
async fn secret_store_contract_keeps_values_out_of_references() {
    let store = FakeSecretStore::default();
    let id = Uuid::new_v4();
    let secret = SecretString::from("not-in-the-reference".to_owned());
    store.set(id, &secret).await.unwrap();
    assert_eq!(
        store.get(id).await.unwrap().unwrap().expose_secret(),
        "not-in-the-reference"
    );
    assert!(!keyring_ref(id).contains("not-in-the-reference"));
}
```

**Step 2: Run the tests and verify failure**

Run: `cargo test --test secret_store -- --nocapture`

Expected: FAIL because `persistence::secrets` does not exist.

**Step 3: Add the dependency and public abstraction**

Add `keyring = "4.1.6"` to `[dependencies]`. Create an async trait with these exact operations:

```rust
pub const KEYRING_SERVICE: &str = "dev.lazydb.lazydb";

#[async_trait]
pub trait SecretStore: Send + Sync {
    async fn available(&self) -> Result<(), SecretStoreError>;
    async fn get(&self, profile_id: Uuid) -> Result<Option<SecretString>, SecretStoreError>;
    async fn set(
        &self,
        profile_id: Uuid,
        password: &SecretString,
    ) -> Result<(), SecretStoreError>;
    async fn delete(&self, profile_id: Uuid) -> Result<(), SecretStoreError>;
}
```

Implement:

- `keyring_ref(Uuid) -> String` as `keyring:dev.lazydb.lazydb/<uuid>`.
- `profile_id_from_ref(&str) -> Result<Uuid, SecretStoreError>` with strict prefix validation.
- `NativeSecretStore` through `keyring::v1::Entry`.
- `available()` through `Entry::store_status()`.
- `get()` mapping keyring `NoEntry` to `Ok(None)`.
- `set()` and `delete()` without ever formatting the password.
- Every native operation inside `tokio::task::spawn_blocking`.
- Sanitized error text and stable categories: `Unavailable`, `Locked`, `Missing`, `Backend`, `InvalidReference`.

Do not run an actual set/delete against the user's keyring in ordinary tests.

**Step 4: Run focused tests**

Run: `cargo test --test secret_store -- --nocapture`

Expected: PASS.

Run: `cargo clippy --test secret_store -- -D warnings`

Expected: PASS.

**Step 5: Logical commit checkpoint**

```bash
git add Cargo.toml Cargo.lock src/persistence/mod.rs src/persistence/secrets.rs tests/secret_store.rs
git commit -m "feat(security): add native secret store boundary"
```

### Task 2: Build Single-Line Inputs and Profile Draft Validation

**Files:**
- Create: `src/model/text_input.rs`
- Create: `src/model/profile_manager.rs`
- Modify: `src/model/mod.rs`
- Modify: `src/profile.rs`
- Create: `tests/profile_draft.rs`

**Step 1: Write failing Unicode input tests**

Cover insert, left/right, Home/End, backspace, delete, and paste by character index:

```rust
#[test]
fn text_input_edits_unicode_by_character_position() {
    let mut input = TextInput::from("数据");
    input.move_end();
    input.move_left();
    input.insert('库');
    assert_eq!(input.value(), "数库据");
    input.backspace();
    assert_eq!(input.value(), "数据");
}
```

**Step 2: Write failing draft validation tests**

Test all of these cases:

- New PostgreSQL defaults to host `localhost`, port `5432`, schema `public`, and SSL `prefer`.
- New MySQL defaults to port `3306`.
- New SQLite requires either memory mode or a non-empty path.
- Name is required and unique case-insensitively, excluding the profile being edited.
- Server host and database are required.
- Port must parse to `1..=65535`.
- Editing preserves the UUID.
- A new draft receives a UUID once, not every validation.
- An existing remembered password with an empty password field produces `Preserve`.
- Unchecking Remember produces `Forget`.
- A non-empty password produces `Session` or `Remember` based on the toggle.
- `format!("{draft:?}")` and `format!("{submission:?}")` never contain the password.

**Step 3: Run and verify failure**

Run: `cargo test --test profile_draft -- --nocapture`

Expected: FAIL because draft types do not exist.

**Step 4: Implement `TextInput`**

Use a `String` plus a character cursor:

```rust
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TextInput {
    value: String,
    cursor: usize,
}
```

Expose `value`, `set`, `cursor`, `insert`, `paste`, `backspace`, `delete`,
`move_left`, `move_right`, `move_home`, and `move_end`. Keep the byte-index
conversion private and shared by all mutations.

**Step 5: Implement profile-manager domain types**

Create these core enums and structs:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProfileManagerPage { List, Form, ConfirmDelete }

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProfileField {
    Kind, Name, Host, Port, User, Password, Database, Schema,
    SslMode, Environment, ReadOnly, RememberPassword,
    SqliteMemory, SqlitePath, Test, Save, SaveAndConnect, Cancel,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProfileOperation {
    Testing, Saving, SavingAndConnecting, Deleting, Connecting,
}

#[derive(Clone)]
pub enum CredentialUpdate {
    Preserve,
    Session(SecretString),
    Remember(SecretString),
    Forget,
}

#[derive(Clone)]
pub struct ProfileSubmission {
    pub profile: ConnectionProfile,
    pub credential: CredentialUpdate,
}
```

Implement manual `Debug` for secret-bearing types using only redacted markers.
`ProfileDraft` stores text inputs, enum/toggle fields, the stable profile UUID,
the original `secret_ref`, a `SecretString` password input, and whether a stored
credential exists. Its `validate(&[ConnectionProfile])` returns field-specific
errors and a `ProfileSubmission`.

`ProfileManagerState` owns page, selection, draft, selected field, status,
message, request generation, and whether the panel was opened automatically.
Provide methods for new/edit initialization and visible fields per driver.

**Step 6: Run focused tests**

Run: `cargo test --test profile_draft -- --nocapture`

Expected: PASS.

**Step 7: Logical commit checkpoint**

```bash
git add src/model src/profile.rs tests/profile_draft.rs
git commit -m "feat(profiles): add connection draft validation"
```

### Task 3: Add Pure Profile Manager Reducer Transitions

**Files:**
- Modify: `src/action.rs`
- Modify: `src/app.rs`
- Modify: `src/model/workspace.rs`
- Test: `src/app.rs`
- Create: `tests/profile_reducer.rs`

**Step 1: Write failing reducer tests**

Cover:

- Opening the manager shows the list when profiles exist.
- Opening with no profiles starts a new form.
- New, edit, cancel, selection movement, and delete confirmation.
- Field navigation skips fields hidden for the selected driver.
- Text actions mutate only the active text/password field.
- Enum fields cycle and booleans toggle.
- Test/Save actions reject invalid drafts and attach a field error.
- Valid Test emits exactly one `Command::TestProfile` and marks the form busy.
- Save and Save & Connect produce distinct commands.
- Stale operation results are ignored.
- Save success upserts in place without changing profile order.
- Delete success removes the profile and clears selection safely.
- A running query blocks connection switching and active-profile deletion.

Use pattern matching for secret-bearing commands; do not compare or print their
password values.

**Step 2: Run and verify failure**

Run: `cargo test --test profile_reducer -- --nocapture`

Expected: FAIL because profile manager actions are absent.

**Step 3: Extend semantic actions**

Add actions for:

```rust
OpenProfileManager
CloseProfileManager
ProfileMove(isize)
ProfileStartNew
ProfileStartEdit
ProfileRequestDelete
ProfileConfirmDelete
ProfileCancelDelete
ProfileConnectSelected
ProfileFieldNext
ProfileFieldPrevious
ProfileFocusField(ProfileField)
ProfileInsert(char)
ProfilePaste(String)
ProfileBackspace
ProfileDeleteCharacter
ProfileMoveLeft
ProfileMoveRight
ProfileMoveHome
ProfileMoveEnd
ProfileCycle(i8)
ProfileToggle
ProfileTest
ProfileSave { connect: bool }
ProfileTestSucceeded { request_id: u64, server: ServerInfo }
ProfileTestFailed { request_id: u64, message: String }
ProfileSaved { request_id: u64, profile: ConnectionProfile, warning: Option<String>, connect: bool }
ProfileSaveFailed { request_id: u64, message: String }
ProfileDeleted { request_id: u64, profile_id: Uuid, was_active: bool }
ProfileDeleteFailed { request_id: u64, message: String }
CredentialsRequired { profile_id: Uuid, generation: u64, message: String }
DisconnectCompleted { profile_id: Uuid }
```

Keep `Action: PartialEq`; no action carries a plaintext secret.

**Step 4: Extend commands**

Remove `PartialEq` from `Command` if needed and add:

```rust
TestProfile { request_id: u64, submission: ProfileSubmission }
SaveProfile { request_id: u64, submission: ProfileSubmission, connect: bool }
DeleteProfile { request_id: u64, profile_id: Uuid }
Disconnect { profile_id: Uuid }
```

**Step 5: Implement reducer state transitions**

- Add `profile_manager: Option<ProfileManagerState>` to `App`.
- Replace placeholder `Overlay::ProfilePicker` with `Overlay::ProfileManager`.
- Delegate form editing and validation to `ProfileManagerState` methods.
- Increment request generation before each async command.
- Keep the panel open and busy until the matching completion action arrives.
- On credentials-required, open edit mode for that profile and focus Password.
- Never mutate `App.profiles` before `ProfileSaved` or `ProfileDeleted`.

**Step 6: Run focused reducer tests**

Run: `cargo test --test profile_reducer app::tests -- --nocapture`

Expected: PASS.

**Step 7: Logical commit checkpoint**

```bash
git add src/action.rs src/app.rs src/model/workspace.rs tests/profile_reducer.rs
git commit -m "feat(profiles): add profile manager reducer"
```

### Task 4: Persist Profile and Credential Mutations in Runtime

**Files:**
- Modify: `src/runtime.rs`
- Modify: `src/persistence/profiles.rs`
- Modify: `tests/app_flow.rs`
- Create: `tests/profile_runtime.rs`

**Step 1: Write a fault-injectable fake secret store**

In `tests/profile_runtime.rs`, implement `SecretStore` with:

- An internal UUID-to-secret map.
- Counters for get/set/delete calls.
- Optional configured failure for each operation.
- Helpers that inspect only whether a value exists, not its plaintext in panic
  output.

Use a temporary ProfileStore path. To force profile persistence failure
portably, create a regular file where the parent directory is expected and use
`file/connections.toml` as the target.

**Step 2: Write failing Runtime tests**

Cover:

- Save with session-only password writes metadata and retains the secret only in
  Runtime memory.
- Save with Remember writes a keyring value and a `secret_ref`, while TOML has no
  password.
- Keyring unavailable downgrades to session-only and emits a warning.
- Editing preserves order and UUID.
- Replacing or forgetting a remembered secret updates the keyring.
- Profile save failure restores the previous keyring value.
- Delete removes both metadata and keyring value.
- Delete persistence failure restores the keyring value.
- Runtime and App update only after the completion action.

**Step 3: Run and verify failure**

Run: `cargo test --test profile_runtime -- --nocapture`

Expected: FAIL because Runtime has no profile commands or service injection.

**Step 4: Add Runtime service state**

Replace plain profile/secrets maps with an async-shared registry:

```rust
struct ProfileRegistry {
    order: Vec<Uuid>,
    profiles: HashMap<Uuid, ConnectionProfile>,
    persisted: HashSet<Uuid>,
    session_secrets: HashMap<Uuid, SecretString>,
    startup_password_profile: Option<Uuid>,
}

pub struct Runtime {
    registry: Arc<Mutex<ProfileRegistry>>,
    profile_store: ProfileStore,
    secret_store: Arc<dyn SecretStore>,
    // existing event, connection, and task fields
}
```

Update `Runtime::new` to accept `ProfileStore`, persisted profile IDs, startup
override identity, and `Arc<dyn SecretStore>`. Update existing tests to pass a
temporary ProfileStore and fake store.

**Step 5: Implement test, save, and delete tasks**

- `TestProfile`: connect/probe/close and send a generation-tagged result.
- `SaveProfile`: snapshot registry and old keyring state; apply credential
  intent; save the ordered persistent profile vector through `spawn_blocking`;
  compensate keyring on failure; update registry only after success.
- `DeleteProfile`: snapshot; remove keyring secret; save remaining profiles;
  compensate on failure; update registry and active connection only after
  success.
- Sanitize every rendered error.
- Preserve an ad-hoc `--url` profile in Runtime but exclude it from persistence
  until the user explicitly saves it.

**Step 6: Run focused tests**

Run: `cargo test --test profile_runtime --test persistence -- --nocapture`

Expected: PASS.

**Step 7: Logical commit checkpoint**

```bash
git add src/runtime.rs src/persistence/profiles.rs tests/app_flow.rs tests/profile_runtime.rs
git commit -m "feat(profiles): persist runtime profile changes"
```

### Task 5: Make Connection Switching and Query Dispatch Identity-Safe

**Files:**
- Modify: `src/action.rs`
- Modify: `src/app.rs`
- Modify: `src/model/workspace.rs`
- Modify: `src/runtime.rs`
- Modify: `tests/app_flow.rs`
- Create: `tests/connection_switch.rs`

**Step 1: Write failing switch-safety tests**

Use two temporary SQLite files with different sentinel rows. Verify:

- Requesting profile B leaves A active while B is pending.
- New query requests are rejected while a switch is pending.
- Failed B connection restores A as connected.
- Successful B connection installs B and closes/replaces A.
- A query command tagged for A cannot execute after B is active.
- Catalog, preview, and DDL commands also reject a stale identity.
- Late success/failure from an older connection attempt is ignored.

**Step 2: Run and verify failure**

Run: `cargo test --test connection_switch -- --nocapture`

Expected: FAIL because active and pending identities are not separate and query
commands do not carry a connection identity.

**Step 3: Extend connection state**

Keep compatibility-friendly active fields and add pending fields:

```rust
pub struct ConnectionState {
    pub profile_id: Option<Uuid>,
    pub generation: u64,
    pub pending_profile_id: Option<Uuid>,
    pub pending_generation: Option<u64>,
    pub status: ConnectionStatus,
    pub server: Option<ServerInfo>,
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConnectionIdentity {
    pub profile_id: Uuid,
    pub generation: u64,
}
```

During a switch, keep `profile_id/server` for the old active connection and set
pending fields for the target. On failure, clear pending and return to Connected
when an active profile exists. On success, promote pending to active.

**Step 4: Tag all database commands**

Add `connection: ConnectionIdentity` to `RunQuery`, `PreviewTable`, and `LoadDdl`.
Use the existing profile/generation fields on `LoadCatalog` as its identity.
App emits these commands only when connected and no switch is pending.

Change Runtime's helper to:

```rust
async fn active_database(
    connection: Arc<Mutex<Option<ActiveConnection>>>,
    expected: ConnectionIdentity,
) -> Option<DatabaseConnection>
```

Return a clone only when both ID and generation match.

**Step 5: Resolve credentials safely during connect**

Resolve in this order:

1. Session secret already registered by form submission.
2. Startup `LAZYDB_PASSWORD`, only when the requested UUID equals
   `startup_password_profile`.
3. Native keyring when `secret_ref` is present.
4. No password for profiles without a secret reference.

If a referenced secret is missing, locked, or unavailable, emit
`CredentialsRequired` before calling SQLx. Never reuse the startup environment
password for another runtime-selected profile.

**Step 6: Run focused tests**

Run: `cargo test --test connection_switch --test app_flow -- --nocapture`

Expected: PASS.

**Step 7: Logical commit checkpoint**

```bash
git add src/action.rs src/app.rs src/model/workspace.rs src/runtime.rs tests/app_flow.rs tests/connection_switch.rs
git commit -m "fix(runtime): bind commands to active connections"
```

### Task 6: Route Profile Manager Keyboard, Paste, and Mouse Input

**Files:**
- Modify: `src/input/keymap.rs`
- Modify: `src/input/mouse.rs`
- Modify: `src/runtime.rs`
- Modify: `src/ui/mod.rs`
- Modify: `tests/keymap.rs`
- Modify: `tests/mouse.rs`

**Step 1: Write failing keymap tests**

Test:

- `Space c` opens the manager from each Normal-mode focus.
- Overlay input routes to manager actions before generic overlay dismissal.
- List keys map to move/connect/new/edit/delete/close.
- Form Tab/BackTab, text editing, enum cycling, toggles, F5, Ctrl-S,
  Ctrl-Enter, and Esc map correctly.
- Release events remain ignored.
- Insert-mode Space still inserts a space instead of opening the manager.

**Step 2: Write failing paste and mouse tests**

- Paste in an active profile text/password field maps to `ProfilePaste`.
- Paste outside the form keeps existing editor behavior.
- Header click opens the manager.
- Profile row, form field, toggle, and button hit targets map to semantic actions.
- Scrolling over the profile list moves its selection, not the explorer.

**Step 3: Run and verify failure**

Run: `cargo test --test keymap --test mouse -- --nocapture`

Expected: FAIL for missing manager mappings and hit targets.

**Step 4: Implement input routing**

- Add `(Pending::Leader, 'c') -> OpenProfileManager`.
- Add a dedicated `map_profile_manager` path before the current overlay shortcut.
- Keep only Esc/q generic dismissal for Help and Message overlays.
- Route `Event::Paste` based on the active overlay and field.
- Extend `HitTarget` with HeaderProfile, ProfileRow, ProfileField, and
  ProfileButton variants.
- Ensure sensitive pasted text is never copied into an Action debug message;
  `ProfilePaste` Debug must redact when the active field is Password, or use a
  dedicated redacted payload type.

**Step 5: Run focused tests**

Run: `cargo test --test keymap --test mouse -- --nocapture`

Expected: PASS.

**Step 6: Logical commit checkpoint**

```bash
git add src/input src/runtime.rs src/ui/mod.rs tests/keymap.rs tests/mouse.rs
git commit -m "feat(profiles): add manager input controls"
```

### Task 7: Render the Responsive Profile Manager

**Files:**
- Create: `src/ui/profiles.rs`
- Modify: `src/ui/mod.rs`
- Modify: `src/ui/theme.rs`
- Modify: `tests/ui_render.rs`

**Step 1: Write failing render tests**

Add fixtures for list, PostgreSQL form, MySQL form, SQLite file form, SQLite
memory form, delete confirmation, busy state, validation error, and keyring
fallback warning. Assert semantic text instead of full-buffer snapshots.

Required assertions include:

- List shows `CONNECTIONS`, active marker, driver, endpoint, environment, and
  read-only state.
- Server form contains Host, Port, User, Password, Database, Schema, SSL,
  Environment, Read only, and Remember password.
- SQLite form omits server-only fields and shows Path/Memory.
- Password value never appears; only mask characters or `Stored in system
  keyring` are rendered.
- Compact form remains usable at `80x24`.
- Tiny terminal still shows the resize message instead of a clipped modal.
- Busy state visibly disables duplicate actions.

**Step 2: Run and verify failure**

Run: `cargo test --test ui_render -- --nocapture`

Expected: FAIL because the profile manager renderer is still a placeholder.

**Step 3: Implement `ui::profiles`**

Provide one entry point:

```rust
pub fn render_profile_manager(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    state: &mut UiState,
    theme: Theme,
)
```

- Center a bounded panel for standard/wide sizes and use the available body for
  compact sizes.
- Render list, form, and confirmation as separate internal functions.
- Build field rows from `ProfileDraft::visible_fields()` so hidden fields cannot
  receive focus or hit regions.
- Mask passwords by character count and never call `expose_secret()` to produce
  rendered text.
- Record precise field/button/profile-row hit regions.
- Set the terminal cursor only for active non-password text fields.
- Add inline status/error/warning lines and the approved keyboard hints.

Modify `render_header` to record a HeaderProfile hit region. Modify
`render_overlay` to receive `&App` and `&mut UiState` and delegate to the new
module.

**Step 4: Run focused UI tests**

Run: `cargo test --test ui_render -- --nocapture`

Expected: PASS.

**Step 5: Logical commit checkpoint**

```bash
git add src/ui tests/ui_render.rs
git commit -m "feat(ui): render connection profile manager"
```

### Task 8: Change Startup and Empty-State Behavior

**Files:**
- Modify: `src/runtime.rs`
- Modify: `src/app.rs`
- Modify: `src/ui/mod.rs`
- Modify: `tests/app_flow.rs`
- Create: `tests/startup_profiles.rs`

**Step 1: Write failing startup tests**

Cover:

- Empty store and no URL returns no selected profile and opens the new form.
- Empty store no longer creates an implicit `local-memory` profile.
- `--profile name` selects the persisted profile.
- `--url` creates one ad-hoc profile, selects it, and does not persist it.
- `LAZYDB_PASSWORD` is associated only with the startup-selected UUID.
- A saved profile with a valid keyring reference resolves through SecretStore.
- A saved profile with an unavailable/missing referenced secret opens edit mode
  focused on Password.

Expose startup loading through a small testable function or test-only public
wrapper; do not launch a real terminal in these tests.

**Step 2: Run and verify failure**

Run: `cargo test --test startup_profiles -- --nocapture`

Expected: FAIL because startup always returns a UUID and injects memory SQLite.

**Step 3: Implement optional startup selection**

Replace the tuple alias with a named struct:

```rust
struct StartupProfiles {
    profiles: Vec<ConnectionProfile>,
    persisted_ids: HashSet<Uuid>,
    session_secrets: HashMap<Uuid, SecretString>,
    selected: Option<Uuid>,
    startup_password_profile: Option<Uuid>,
    store: ProfileStore,
}
```

In `run_tui`:

- Construct App and Runtime first.
- If `selected` exists, dispatch `RequestConnect`.
- Otherwise dispatch `OpenProfileManager` followed by `ProfileStartNew`.
- Draw immediately after the initial actions.

Update the explorer empty message to point to `Space c` rather than requiring
startup flags.

**Step 4: Run focused startup and flow tests**

Run: `cargo test --test startup_profiles --test app_flow -- --nocapture`

Expected: PASS.

**Step 5: Logical commit checkpoint**

```bash
git add src/runtime.rs src/app.rs src/ui/mod.rs tests/app_flow.rs tests/startup_profiles.rs
git commit -m "feat(profiles): open manager on first launch"
```

### Task 9: Add the Full SQLite Profile Lifecycle Integration Test

**Files:**
- Create: `tests/profile_lifecycle.rs`
- Modify: `tests/app_flow.rs`

**Step 1: Write the end-to-end test**

Drive the public Action/Command/Runtime flow with two temporary SQLite files:

1. Start with an empty ProfileStore.
2. Open the manager and create profile A.
3. Test A and assert success without persistence.
4. Save & Connect A and assert catalog load.
5. Create a sentinel table/value in A.
6. Create and save profile B.
7. Switch to B and create a different sentinel.
8. Switch back to A and assert A's sentinel, proving pool identity.
9. Edit A's name/path metadata and assert stable UUID/order.
10. Delete inactive B and assert metadata removal.
11. Delete active A, assert disconnect and empty catalog.
12. Recreate Runtime from the saved TOML and assert persisted state reloads.

Add timeout bounds to every async event receive. Never rely on sleeps.

**Step 2: Run and verify behavior**

Run: `cargo test --test profile_lifecycle -- --nocapture`

Expected before final fixes: FAIL at the first incomplete lifecycle edge.

**Step 3: Make only lifecycle fixes**

Fix ordering, completion actions, disconnect cleanup, and stale event handling
found by the test. Do not expand scope beyond the approved design.

**Step 4: Run related integration suites**

Run: `cargo test --test profile_lifecycle --test profile_runtime --test connection_switch --test app_flow -- --nocapture`

Expected: PASS.

**Step 5: Logical commit checkpoint**

```bash
git add src tests/profile_lifecycle.rs tests/app_flow.rs
git commit -m "test(profiles): cover dynamic profile lifecycle"
```

### Task 10: Update Capabilities and User Documentation

**Files:**
- Modify: `src/cli.rs`
- Modify: `README.md`
- Modify: `docs/configuration.md`
- Modify: `docs/keybindings.md`
- Modify: `docs/architecture.md`
- Modify: `lazydb.nvim/README.md`
- Modify: `lazydb.nvim/doc/lazydb.txt`
- Modify: `lazydb.nvim/tests/lazydb_spec.lua`

**Step 1: Write a failing capability contract test**

Update the CLI test to require `profile-manager` and `system-keyring` while
keeping `cli_api = 1` and existing drivers.

Run: `cargo test cli::tests::capabilities_contract_is_stable -- --nocapture`

Expected: FAIL until capability output is updated.

**Step 2: Update capability output**

Change the fixed feature array to include the two new features. Do not change
the CLI API version because existing consumers ignore unknown feature strings.

**Step 3: Update docs**

Document:

- `Space c` and complete manager controls.
- First-run behavior.
- Native keyring service/account format.
- Explicit Remember Password behavior and session fallback.
- `LAZYDB_PASSWORD` startup-only scope.
- Manual keyring cleanup expectations.
- No passwords in URLs.
- Profile file format and `secret_ref`.
- Neovim inherits the TUI manager and does not store credentials.

Remove the statement that keyring resolution is upcoming.

**Step 4: Run CLI and Neovim tests**

Run: `cargo test cli::tests -- --nocapture`

Expected: PASS.

Run: `nvim --headless -u lazydb.nvim/tests/minimal_init.lua -c "lua require('lazydb_spec').run()" -c qa`

Expected: `7 tests passed` or the updated total if a capability assertion is
added.

**Step 5: Logical commit checkpoint**

```bash
git add src/cli.rs README.md docs lazydb.nvim
git commit -m "docs: document dynamic connection profiles"
```

### Task 11: Complete Platform and Interactive Verification

**Files:**
- Modify only files required by failures found during verification.

**Step 1: Run formatting**

Run: `cargo fmt --check`

Expected: PASS. If it fails, run `cargo fmt` and inspect the changes.

**Step 2: Run strict linting**

Run: `cargo clippy --all-targets --all-features -- -D warnings`

Expected: PASS.

**Step 3: Run all Rust tests**

Run: `cargo test --all-targets --all-features`

Expected: PASS with no ignored failure paths. PostgreSQL/MySQL service tests may
skip only when their documented environment variables are absent.

**Step 4: Run real PostgreSQL and MySQL adapter tests**

Start temporary `postgres:16` and `mysql:8.4` containers using unique names and
no volumes. Set:

```text
LAZYDB_TEST_POSTGRES_URL=postgresql://postgres:lazydb_test@127.0.0.1:5432/lazydb_test
LAZYDB_TEST_MYSQL_URL=mysql://root:lazydb_test@127.0.0.1:3306/lazydb_test
```

Run:

```bash
cargo test --test postgres_adapter --test mysql_adapter -- --nocapture
```

Expected: all four tests PASS. Remove only the temporary named containers.

**Step 5: Run Neovim tests and health check**

Run the headless suite, then place `target/debug` on PATH and run
`:checkhealth lazydb` headlessly.

Expected: all plugin tests pass and every LazyDB health item is OK.

**Step 6: Run PTY smoke tests**

With an isolated HOME/config path:

- Launch with no profiles and wait for `NEW CONNECTION`.
- Create/connect a SQLite memory profile through key events.
- Open the manager with `Space c`.
- Exit with `Q` and verify exit status 0 and terminal restoration.
- Launch saved SQLite, PostgreSQL, and MySQL profiles and wait for `ONLINE`.

Expected: every process exits cleanly; no secret appears in captured output.

**Step 7: Run opt-in native keyring smoke only with explicit approval**

Use a random profile UUID, set/get/delete one test value, and verify cleanup.
Never use a real connection's account key. Skip this step when no desktop
keyring is available or approval is absent.

**Step 8: Inspect final diff and status**

Run:

```bash
git status --short
git diff --check
git diff --stat
```

Expected: only intended implementation, tests, lockfile, and documentation
changes. Do not commit implementation changes unless explicitly authorized.

## Final Acceptance Checklist

- First launch opens a usable new-connection form.
- CRUD and switching work without restart or manual TOML editing.
- Test Connection has no side effects on active connection or persistence.
- Remembered passwords use native Keychain/Secret Service only.
- Session fallback is explicit and password-free on disk.
- Editing retains profile UUID and keyring identity.
- Delete cleans up metadata, session state, and remembered credentials.
- Failed persistence compensates credential mutations.
- Failed switch restores the previous active connection.
- Stale database commands cannot run against a different pool.
- Standard, wide, compact, keyboard, mouse, paste, CLI, Neovim, and PTY paths
  are verified.
