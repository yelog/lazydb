# Default Password Persistence Design

## Problem

New PostgreSQL and MySQL connection drafts currently default `Remember Password`
to disabled. Entering a password therefore produces a session-only credential,
persists the profile with `credential_policy = prompt`, and keeps the password
only in runtime memory. After restart, the password is unavailable and startup
correctly opens Edit Connection with `Enter a password to continue`.

The observed profile confirms this path: its TOML policy is `prompt`, and no
matching `dev.lazydb.lazydb` entry exists in macOS Keychain. The password was not
lost by the keyring; it was never sent to the keyring.

## Decision

New server connection drafts will default `Remember Password` to enabled. When
a user enters a password and saves without changing that option, LazyDB will use
the existing `CredentialUpdate::Remember` path and persist the password in the
native secret store.

SQLite remains unchanged because it has no password credential. Users can turn
off `Remember Password` to explicitly keep a password for the current process
only.

Profiles opened because a password is required will also default the option to
enabled when no stored credential exists. This lets a user repair an existing
`prompt` profile once instead of accidentally saving another session-only
password. Existing keyring profiles retain their enabled state.

## Persistence Flow

1. A new PostgreSQL or MySQL draft starts with `remember_password = true`.
2. A non-empty password produces `CredentialUpdate::Remember`.
3. Runtime writes the password to the native secret store under service
   `dev.lazydb.lazydb` and account equal to the profile UUID.
4. On success, profile TOML stores only the canonical keyring reference through
   `CredentialPolicy::Keyring`.
5. On restart, Runtime resolves the reference and reads the password from the
   native secret store.

The password must never be serialized into `connections.toml`, URLs, logs, debug
output, or user-visible error messages.

## Failure Handling

If the native secret store is locked or unavailable, retain the current secure
fallback: keep the password in session memory, persist `CredentialPolicy::Prompt`,
and return an explicit warning that the password is available only for the
current session. Backend errors other than an unavailable or locked store remain
save failures.

No file-based password fallback will be added.

## Verification

- New PostgreSQL and MySQL drafts enable `Remember Password` by default.
- New SQLite drafts remain credential-free.
- A password with the default setting produces `CredentialUpdate::Remember`.
- Turning the setting off produces `CredentialUpdate::Session`.
- A prompt profile opened for credential repair defaults to remembering the new
  password.
- Successful secret-store persistence writes a keyring policy without writing
  password text to TOML.
- A newly constructed Runtime can resolve a remembered credential from the same
  persistent secret store, modeling a process restart.
- An unavailable native store still downgrades to `prompt` and reports a warning.
