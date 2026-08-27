# Credential Storage Providers Design

## Scope

Replace the `Remember Password` toggle with an explicit password-storage choice.
Passwords entered for saved PostgreSQL and MySQL profiles are persisted by
default. Local encrypted storage is the cross-platform default; a detected
native system credential store is an optional stronger backend.

SQLite profiles remain passwordless. Ad-hoc `--url` passwords remain
process-local and are not persisted by this design.

## Security Boundary

Local encrypted storage is designed to prevent accidental disclosure when
viewing or sharing `connections.toml`. It is not intended to protect against a
process already running as the same OS user or disclosure of the complete LazyDB
configuration directory.

Password ciphertext is stored in `connections.toml`. A random 256-bit local key
is stored separately in LazyDB's platform configuration directory. On Unix, the
directory remains mode `0700` and both the profile file and local-key file use
mode `0600`.

The local key must never be embedded in the binary or serialized beside the
ciphertext. Losing the key does not cause LazyDB to generate a replacement over
existing ciphertext; the user must re-enter affected passwords.

Use an authenticated encryption algorithm with a fresh random nonce for every
write. XChaCha20-Poly1305 is preferred. Associated data binds the format version,
service identifier, and profile UUID so ciphertext cannot be moved between
profiles undetected.

## Storage Model

Replace the current user-facing credential policy with these persisted storage
states:

```text
None
LocalEncrypted { version, nonce, ciphertext }
System { reference }
Prompt
```

`Prompt` is retained for migration and explicit session-only inputs, but is not
the default result of saving a profile with a non-empty password. Existing
`Keyring` references migrate to `System` without reading or moving their secret.

New PostgreSQL and MySQL profiles default to `LocalEncrypted`. An empty password
on a new profile means no stored password. Editing an existing profile with an
empty password preserves its current credential and storage location. Clearing
an existing password requires an explicit clear action.

The profile format advances from version 3 to version 4. Version 3 mappings are:

- `None` to `None`.
- `Prompt` to `Prompt`.
- `Keyring(reference)` to `System(reference)`.

The current `keyring:` reference syntax remains readable to avoid a second
migration that adds no security value.

## Profile Manager

Remove `Remember Password`. Add `Password Storage` for PostgreSQL and MySQL:

```text
LOCAL ENCRYPTED
MACOS LOGIN KEYCHAIN
SECRET SERVICE
```

`LOCAL ENCRYPTED` is always present and selected by default for new profiles.
The platform-specific system option is visible only when capability probing says
the provider is available or locked.

Provider labels are:

- macOS: `MACOS LOGIN KEYCHAIN`.
- Linux and other supported Unix desktops: `SECRET SERVICE`.

Do not infer `GNOME KEYRING`, `KEEPASSXC`, or `KWALLET` from the desktop name.
The Freedesktop interface does not reliably identify the implementation, and a
GNOME session can use another provider.

If a provider is locked, show it as selectable with a `LOCKED` annotation. A
locked provider exists and may become usable through normal OS unlock behavior.
If there is no session bus, no Secret Service owner, no default collection, or no
native store, hide the system option for new and Local profiles.

When editing an existing System profile, always show its current storage backend,
even when it is unavailable. Mark it `CURRENT, UNAVAILABLE`. Leaving Password and
Password Storage unchanged preserves the reference without reading, deleting, or
migrating the secret.

## Capability Detection

Capability probing must not read, write, or delete any LazyDB password entry and
must not create a temporary secret.

Represent status structurally:

```text
Checking
Available { provider }
Locked { provider }
Unavailable { reason }
Error { reason }
```

On macOS, probe whether the default user/login Keychain can be opened and inspect
its status without accessing a password item.

On Linux, probe the current user's session D-Bus, the
`org.freedesktop.secrets` owner, service-session establishment, and the default
collection. Distinguish no session bus, missing service, missing collection, and
locked collection.

Run probes off the rendering thread. The form is immediately usable with Local
storage while the runtime checks System support. Cache Available, Locked, and
Unavailable for the process lifetime. Temporary errors may be retried after a
bounded interval or an explicit refresh.

Provider availability is runtime state and must not be persisted. The same
profile file may be opened from a desktop session, SSH session, another OS, or a
machine with a different Secret Service implementation.

The current `keyring::v1` facade is too coarse for provider-specific status and
labels. Move toward `keyring-core` plus target-specific
`apple-native-keyring-store` and `zbus-secret-service-keyring-store` adapters, or
equivalent direct platform probes behind LazyDB-owned traits.

## Save Transactions

### Local

1. Load or atomically create the local key.
2. Generate a fresh nonce.
3. Encrypt the password with profile-bound associated data.
4. Construct the version 4 profile snapshot.
5. Write, sync, and atomically rename the profile file.

### System

1. Preserve the prior System secret when applicable for compensation.
2. Attempt to write the new System secret.
3. If it succeeds, persist `System(reference)`.
4. If profile persistence fails, restore or delete the System entry to return to
   the previous state.

### System Write Fallback

If the user selects System and the actual write fails:

1. Encrypt the password using Local storage.
2. Persist the actual profile state as `LocalEncrypted`.
3. Return success with a sanitized warning naming the attempted provider and the
   Local fallback.

The fallback is never silent. Reopening the profile must display Local as the
actual storage location.

A later System read failure cannot fallback unless a Local copy exists. System
mode deliberately does not retain a hidden Local duplicate because that would
erase its stronger security property. Ask the user to re-enter the password when
the stored System secret cannot be read.

Switching from System to Local writes the Local ciphertext first and then
attempts to delete the obsolete System item. Failure to delete is a warning and
must not discard the new usable Local credential. Switching from Local to System
must preserve the prior Local state until both System write and profile
persistence succeed.

## Runtime Resolution

Password resolution order remains:

1. Current-process session cache.
2. Startup-bound environment password.
3. Persisted storage backend.

Successful Local decryption or System retrieval is inserted into the
current-process session cache. Reconnects therefore do not repeatedly access the
key file or native credential store. Password replacement, clearing, profile
deletion, and storage migration update or invalidate the cache.

## Diagnostics

`doctor` should expose non-sensitive capability information:

```text
provider: macos-login-keychain | freedesktop-secret-service | none
status: available | locked | unavailable | error
reason: optional bounded diagnostic
```

Never include secret values, ciphertext, local-key bytes, or raw platform errors
that may contain hostile terminal controls.

Local decryption failures distinguish missing key, invalid key file, unsupported
format version, authentication failure, and I/O failure. They do not replace
existing encrypted data automatically.

## Testing

- Local ciphertext never contains the plaintext password and changes when a new
  nonce is used for the same password.
- Ciphertext is bound to the profile UUID and detects tampering.
- Local-key creation is atomic, private, and does not replace an existing invalid
  key.
- Version 3 profiles migrate without reading native stores.
- New PostgreSQL/MySQL forms default to Local; SQLite hides storage controls.
- Available and Locked providers appear with the correct platform label.
- Missing Linux session bus or Secret Service hides System for new profiles.
- Existing System profiles remain visible when the provider is unavailable.
- Capability probing performs no secret get/set/delete operations.
- System write failure persists Local and emits a warning.
- System read failure asks for a password and does not claim Local fallback.
- Successful Local/System reads populate the process cache.
- Storage transitions and profile-file failures preserve a usable previous state.
- No Debug output, diagnostics, persistence, or logs expose plaintext secrets or
  local-key material.
