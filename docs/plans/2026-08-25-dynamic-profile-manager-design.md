# Dynamic Profile Manager Design

**Status:** Approved

**Date:** 2026-08-25

## Summary

LazyDB will add an in-process Profile Manager for creating, testing, saving,
editing, deleting, and switching PostgreSQL, MySQL, and SQLite connections.
Connection metadata remains in the versioned `connections.toml` file. Remembered
passwords are stored in the operating system credential store and referenced by
stable profile UUIDs. If the native credential store is unavailable, LazyDB
falls back to a session-only password and reports the downgrade.

The implementation preserves the existing unidirectional architecture:

```text
Crossterm input / database events
               |
             Action
               |
          App::update
               |
            Command
               |
            Runtime
          /          \
 ProfileStore     SecretStore
```

## Product Research

The design follows the strongest common behavior of established database
clients rather than inventing a local encryption format.

### JetBrains DataGrip

DataGrip does not maintain a custom password store. It supports the native
password manager on macOS and Linux, KeePass as an alternative, or a mode that
forgets all passwords after restart. JetBrains also warns that passwords placed
in connection URLs are stored in plain text and can appear in logs.

Source:
<https://www.jetbrains.com/help/datagrip/reference-ide-settings-password-safe.html>

### DBeaver

DBeaver offers integrated security backed by the operating system keyring. Its
Community Edition uses integrated security, while commercial editions also
offer a master-password-backed secure store. DBeaver keeps secure credentials
user-specific and separate from portable project configuration.

Sources:

- <https://dbeaver.com/docs/dbeaver/Security/>
- <https://dbeaver.com/docs/dbeaver/Integrated-Security/>
- <https://dbeaver.com/docs/dbeaver/Managing-Master-Password/>

### Navicat

Navicat stores connection metadata in per-user registry or configuration files
and encrypts database passwords inside that storage. Its documented model is
less desirable for LazyDB because application-managed file encryption requires
a separate key-management design and commonly places ciphertext and key
material under the same user account.

Source:
<https://help.navicat.com/hc/en-us/articles/219566088-How-secure-is-Navicat>

### Decision

LazyDB will use the native operating system credential store with an explicit
session-only fallback. It will not introduce a custom encrypted password file
or master-password vault in this milestone.

## Goals

- Open the Profile Manager from the keyboard or mouse.
- Create, test, save, edit, delete, and switch connection profiles at runtime.
- Support PostgreSQL, MySQL, SQLite files, and SQLite memory databases.
- Persist connection metadata without passwords.
- Optionally remember passwords in macOS Keychain or Linux Secret Service.
- Degrade safely to session-only secrets when a native keyring is unavailable.
- Keep App and Runtime profile collections synchronized only after persistence
  succeeds.
- Prevent a query from running against the wrong database during a connection
  switch.
- Open the Profile Manager automatically on first launch when no profile or
  direct URL is available.

## Non-goals

- Windows credential storage.
- A KeePass or custom master-password vault.
- SSH tunnels, proxies, or client-certificate pickers.
- Cloud secret providers.
- Profile import/export and team synchronization.
- Multiple simultaneously active database workspaces.

## State and Architecture

### Application State

`App` gains a `ProfileManagerState` that contains only interactive state:

- Current page: list, form, or delete confirmation.
- Selected profile and selected field/button.
- A `ProfileDraft` containing editable connection values.
- Masked password state backed by `SecretString`.
- Whether the password should be remembered.
- Validation, warning, and asynchronous operation status.
- A monotonically increasing request generation.

The existing `Overlay` remains a lightweight rendering selector. Sensitive
state is not embedded in general-purpose messages or output entries. Debug
representations must remain redacted.

### Runtime State

Runtime owns all side effects and gains:

- The selected `ProfileStore` path used at startup.
- A mutable profile registry.
- Session-only secrets keyed by profile UUID.
- A `SecretStore` implementation.
- Active and pending connection identities.

`SecretStore` is an injectable trait with get, set, delete, and availability
operations. Production uses the Rust `keyring` ecosystem. macOS maps to Keychain
Services and Linux maps to Secret Service. Tests use a deterministic in-memory
implementation with fault injection.

Blocking native keyring calls run through `tokio::task::spawn_blocking`.

### Secret References

Remembered credentials use a stable service/account identity:

```text
service: dev.lazydb.lazydb
account: <profile-uuid>
secret_ref: keyring:dev.lazydb.lazydb/<profile-uuid>
```

Editing a profile does not change its UUID, so renaming or changing its endpoint
does not orphan the credential.

## User Experience

### Entry Points

- `Space c` opens the manager from Normal mode.
- Clicking the profile area in the header opens the manager.
- With no persisted profile and no `--url`, startup opens the new-profile form.

`--url` remains an ad-hoc, non-persisted connection. `--profile` retains its
existing startup selection behavior.

### Profile List

The list shows name, driver, endpoint, environment, read-only status, and the
active marker.

```text
j/k or arrows  move
Enter          connect
n              new
e              edit
d              delete
Esc            close
```

Deleting the active profile requires confirmation. A running query blocks
switching or deleting the active profile until the query is cancelled.

### Profile Form

PostgreSQL and MySQL expose:

- Name
- Host
- Port
- User
- Password
- Database
- Default schema
- SSL mode
- Environment
- Read-only
- Remember password

SQLite exposes:

- Name
- File path or memory mode
- Read-only

The password is always masked. Editing a profile with a keyring reference shows
that a password is stored but never reveals or pre-fills it.

Tab and Shift-Tab move through fields and buttons. Text fields support Unicode
cursor movement, backspace, delete, Home, End, and bracketed paste. The footer
contains Test, Save, Save & Connect, and Cancel actions.

```text
F5           test
Ctrl-s       save
Ctrl-Enter   save and connect
Esc          cancel or return
```

Standard and wide terminals use a centered panel. Compact terminals use the
available workspace. Tiny terminals keep the existing actionable resize view.

## Data Flows

### Test Connection

1. Validate the draft without mutating saved state.
2. Emit `TestProfile` with a request generation and transient secret.
3. Runtime creates the concrete adapter, probes the server, and closes it.
4. Runtime emits a generation-tagged success or failure action.
5. The form remains open and shows the result.

Testing never replaces the active connection or writes profile/keyring data.

### Save

1. Validate and normalize the draft.
2. Snapshot the prior profile and prior remembered secret, if any.
3. Apply the requested keyring mutation.
4. Atomically save the complete ordered profile list.
5. Update Runtime's profile registry and session secret map.
6. Emit `ProfileSaved` so App updates its list.

If a keyring is unavailable while Remember Password is selected, save the
profile without a keyring reference, retain the secret for the current session,
and emit a visible warning.

Save & Connect performs the same save flow and starts a connection attempt only
after persistence succeeds.

### Delete

1. Reject deletion when the active profile has a running query.
2. Snapshot the profile and remembered secret.
3. Delete the remembered credential.
4. Atomically save the remaining ordered profiles.
5. Remove Runtime session state and notify App.
6. Disconnect and clear the catalog if the deleted profile was active.

### Compensation

The profile file and OS credential store cannot share a real transaction. Every
mutation therefore keeps enough prior state to restore the old credential when
profile persistence fails. If compensation also fails, the error reports both
the primary and recovery failures. LazyDB never reports a partially completed
operation as successful.

## Credential Resolution

The credential used for a connection is resolved in this order:

1. Password entered in the current form.
2. `LAZYDB_PASSWORD` for the profile explicitly selected at process startup.
3. The current process's session-secret cache.
4. The profile's native keyring reference.

`LAZYDB_PASSWORD` is never reused for a different profile selected later in the
same process. A missing, locked, or unavailable keyring opens the corresponding
edit form focused on Password instead of attempting an authenticated connection
with an empty secret.

## Connection Switching Safety

Connection state distinguishes the active connection from a pending attempt.
The existing active pool stays available until the new profile passes connect
and probe, but new queries are disabled while switching. A failed attempt clears
the pending state and restores the previous connection as online.

Query, catalog, preview, and DDL commands carry the expected profile UUID and
connection generation. Runtime executes them only when those values match the
active connection. This closes the current possibility of a command using a
stale pool after a future runtime switch.

On successful switch, Runtime installs the new connection and then closes the
old pool. On explicit disconnect or active-profile deletion, it closes the pool
and clears catalog state.

## Error Handling

- Draft validation errors stay attached to the relevant field.
- Database and keyring errors are sanitized before rendering.
- Duplicate submissions are disabled while an operation is active.
- Stale asynchronous results are ignored using request generations.
- Failed saves retain all draft values and the masked password.
- Failed connection switches keep the prior active connection.
- Keyring fallback is visible and actionable, never silent.

## Testing

### Unit Tests

- Draft defaults and validation for all drivers.
- Port, path, duplicate-name, SSL, environment, and read-only normalization.
- Password redaction from serialization and Debug output.
- Reducer transitions for list, form, confirmation, and stale generations.
- Keymap and mouse mappings for all manager actions.
- In-memory SecretStore success, unavailability, and injected failures.
- Profile/keyring compensation at every failure boundary.

### Integration Tests

- Two temporary SQLite databases exercise create, test, save, edit, switch,
  delete, restart, and reload.
- Query commands cannot execute against a stale profile/generation.
- Existing PostgreSQL and MySQL container adapter tests remain green.
- Native keyring smoke tests are opt-in, use random account IDs, and clean up.

### UI and Interactive Tests

- Render list, server forms, SQLite forms, busy states, and errors at standard,
  wide, and compact sizes.
- PTY smoke verifies first-run form display, SQLite memory connection, normal
  exit, and terminal restoration.
- Existing Neovim lifecycle and health tests remain unchanged and green.

## Documentation and Capability Contract

Update the README, configuration, keybinding, and architecture documents. The
CLI capabilities response adds `profile-manager` and `system-keyring`. Neovim
continues to own only the terminal process and never receives credentials.

## Acceptance Criteria

- A first-time user can create and connect to any supported database without
  restarting LazyDB or editing TOML.
- Saved metadata survives restart and contains no password.
- Remembered passwords are retrieved from the native OS store.
- A missing native store degrades to a clearly reported session-only secret.
- Existing profiles can be edited, deleted, and switched safely.
- Failed persistence leaves App, Runtime, the profile file, and the keyring in
  the prior usable state whenever compensation succeeds.
- No query can run on a connection whose profile/generation does not match the
  command.
- Formatting, Clippy, all Rust and Neovim tests, database adapter tests, and PTY
  smoke checks pass.
