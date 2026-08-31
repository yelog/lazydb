# Coding Agent Database Access Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a stable JSON CLI and project-scoped stdio MCP server that let coding agents safely discover, inspect, query, and deliberately modify databases managed by LazyDB without choosing connections from unrelated projects.

**Architecture:** Add a headless `AgentService` alongside the existing TUI runtime. It loads the same profile registry and credentials, resolves the current canonical project root, exposes current-project plus global profiles, excludes profiles belonging only to other projects, and applies deterministic connection selection and write policy before any database I/O. The JSON CLI and MCP server are thin adapters over this service; database credentials, profile `read_only`, environment restrictions, result limits, and target identity remain enforced inside LazyDB rather than delegated to prompts or MCP client configuration.

**Tech Stack:** Rust 2024, Tokio, SQLx, Clap, Serde/JSON, `rmcp` 3.1.x stdio server, existing LazyDB profile/catalog/query/security modules.

---

## Confirmed Product Rules

1. Agent-visible profiles are the union of profiles scoped to the current project and global profiles.
2. Profiles scoped only to another project are not listed and cannot be selected by name or UUID.
3. A current-project profile has selection priority over a global profile, but a profile is never selected from registry order.
4. Explicit connection selectors accept an exact UUID or an unambiguous profile name. Duplicate names fail with a structured ambiguity error.
5. Without an explicit selector, use the sole current-project profile; otherwise use the sole global profile only when there are no current-project profiles. Every other case fails closed. Persisted project defaults are deferred until real usage demonstrates the need.
6. Global visibility does not override profile `read_only`, database grants, environment restrictions, or MCP write policy.
7. The MCP launch configuration sets the server write ceiling with `--write-policy deny|non-production|all`. Default is `deny`.
8. Coding-agent MCP permissions separately decide whether tools are visible and whether a write tool requires approval. They cannot make LazyDB or the database grant a forbidden write.
9. `production` writes require both `--write-policy all` and a writable profile; the sample agent configurations do not enable this mode.
10. Schema inspection and query results always include canonical target identity: profile UUID/name, scope, environment, driver, endpoint, database, schema, role/user, and effective read-only status. Secrets are never returned.
11. The first release supports local stdio MCP only. Remote HTTP transport, OAuth, temporary grants, audit upload, database branches, and production change workflows are deferred.

## Public Command Contract

The implementation should produce these commands:

```text
lazydb agent context [--project PATH] [--connection NAME_OR_UUID]
lazydb agent connections [--project PATH]
lazydb agent schema-search QUERY [--project PATH] [--connection NAME_OR_UUID] [--limit N]
lazydb agent describe QUALIFIED_NAME [--project PATH] [--connection NAME_OR_UUID]
lazydb agent query [--sql SQL | --file PATH] [--project PATH] [--connection NAME_OR_UUID]
lazydb agent execute [--sql SQL | --file PATH] [--project PATH] [--connection NAME_OR_UUID] --write-policy POLICY
lazydb mcp serve [--project PATH] [--connection NAME_OR_UUID] [--write-policy POLICY]
```

All `agent` commands print exactly one JSON document to stdout. Diagnostics go to stderr. Success exits `0`; invalid input/policy/selection exits `2`; credential or connection failure exits `3`; database execution failure exits `4`.

The MCP server exposes this initial tool set:

```text
get_context
list_connections
search_schema
describe_object
query
execute_change
execute_file
```

`execute_change` accepts SQL text. `execute_file` accepts a path under the canonical project root, reads the file inside LazyDB, and rejects paths outside the project. Read and write are separate tools so Codex, OpenCode, and Claude Code can attach different approval policies.

## Out Of Scope For This Plan

- Persisting a new `agent_access` object in `connections.toml`.
- Changing TUI Explorer placement or profile access UI.
- Letting MCP clients submit DSNs, hosts, credentials, or arbitrary project roots per tool call.
- Returning other-project profiles even when a caller knows their UUID.
- Long-lived agent transactions.
- Applying production migrations through a separate approval service.
- Remote MCP transport.

### Task 1: Add Project Resolution And Agent Visibility Rules

**Files:**
- Modify: `src/project.rs`
- Create: `src/agent/mod.rs`
- Create: `src/agent/context.rs`
- Modify: `src/lib.rs`
- Test: `tests/agent_context.rs`

**Step 1: Write project-resolution characterization tests**

Add tests covering:

```rust
#[test]
fn resolves_git_file_as_project_marker() { /* create .git file and nested cwd */ }

#[test]
fn explicit_project_path_uses_the_same_canonical_root_logic_as_cwd() { /* ... */ }
```

The existing documentation says a `.git` directory or file is accepted; lock that current contract down before agent code depends on it.

**Step 2: Run the characterization tests**

Run: `cargo test --test agent_context resolves_git_file_as_project_marker -- --exact`

Expected: PASS, confirming agent code can safely reuse current project resolution without changing its behavior.

**Step 3: Add agent visibility tests**

Construct four profiles: current-project, global, other-project, and project-scoped with multiple roots. Assert:

```rust
assert_eq!(
    visible_names(&profiles, &current_root),
    ["current", "shared", "multi-root"]
);
assert!(!is_visible_to_agent(&other_project, &current_root));
```

Also assert deterministic ordering: current-project profiles preserve registry order before global profiles; ordering is presentation only and is never a selection tiebreaker.

Run: `cargo test --test agent_context`

Expected: FAIL because the agent visibility projection does not exist.

**Step 4: Implement `AgentProjectContext` and visibility projection**

In `src/agent/context.rs`, add small owned types:

```rust
pub struct AgentProjectContext {
    pub project: ProjectContext,
}

pub enum AgentProfileScope {
    CurrentProject,
    Global,
}

pub struct VisibleAgentProfile<'a> {
    pub profile: &'a ConnectionProfile,
    pub scope: AgentProfileScope,
}
```

Implement `AgentProjectContext::resolve(project: Option<&Path>)` by calling `ProjectContext::resolve_from` or `resolve_current`; do not duplicate Git-root traversal. Implement `visible_profiles` so `Global` and matching `Projects` are included, while nonmatching `Projects` are omitted.

**Step 5: Export the module and run focused tests**

Run: `cargo test --test agent_context`

Expected: PASS.

**Step 6: Run existing project/profile tests**

Run: `cargo test --test startup_profiles --test persistence`

Expected: PASS with no TUI behavior changes.

**Step 7: Commit**

```bash
git add src/project.rs src/agent/mod.rs src/agent/context.rs src/lib.rs tests/agent_context.rs
git commit -m "feat(agent): resolve project-visible connections"
```

### Task 2: Implement Deterministic Connection Selection

**Files:**
- Create: `src/agent/selection.rs`
- Modify: `src/agent/mod.rs`
- Test: `tests/agent_selection.rs`

**Step 1: Write the selection matrix as failing tests**

Cover each rule independently:

```rust
#[test] fn explicit_uuid_selects_a_visible_profile() {}
#[test] fn explicit_name_selects_an_unambiguous_visible_profile() {}
#[test] fn explicit_other_project_uuid_is_reported_as_not_found() {}
#[test] fn duplicate_visible_names_are_ambiguous() {}
#[test] fn sole_project_profile_wins_over_any_number_of_globals() {}
#[test] fn sole_global_is_selected_when_no_project_profile_exists() {}
#[test] fn multiple_project_profiles_require_an_explicit_selector() {}
#[test] fn multiple_globals_require_an_explicit_selector() {}
#[test] fn no_visible_profiles_is_actionable() {}
```

Assert stable machine error codes such as `connection_not_found`, `connection_ambiguous`, and `no_visible_connections` rather than matching only prose.

**Step 2: Run tests and verify failure**

Run: `cargo test --test agent_selection`

Expected: FAIL because `select_profile` and `AgentError` do not exist.

**Step 3: Add structured selection errors**

Define an `AgentError` carrying:

```rust
pub struct AgentError {
    pub code: AgentErrorCode,
    pub message: String,
}
```

Keep candidate metadata secret-free. Ambiguity errors may return names, UUIDs, scopes, environments, and database names, but never credential policy payloads or passwords.

**Step 4: Implement selection without implicit registry fallback**

Implement:

```rust
pub fn select_profile<'a>(
    visible: &'a [VisibleAgentProfile<'a>],
    selector: Option<&str>,
) -> Result<&'a VisibleAgentProfile<'a>, AgentError>
```

For the first release, do not persist a separate project default. Reserve a `selection_reason` response field so a future default binding can be added without changing the response shape. Selection reasons are `explicit_uuid`, `explicit_name`, `sole_project`, and `sole_global`.

**Step 5: Run focused tests**

Run: `cargo test --test agent_selection`

Expected: PASS.

**Step 6: Commit**

```bash
git add src/agent/mod.rs src/agent/selection.rs tests/agent_selection.rs
git commit -m "feat(agent): select connections deterministically"
```

### Task 3: Extract Shared Credential Resolution

**Files:**
- Create: `src/persistence/credentials.rs`
- Modify: `src/persistence/mod.rs`
- Modify: `src/runtime.rs:2259-2340`
- Test: `tests/credential_resolution.rs`
- Test: `tests/profile_runtime.rs`

**Step 1: Write failing resolver tests with a fake `SecretStore`**

Cover:

```rust
#[tokio::test] async fn resolves_passwordless_profile() {}
#[tokio::test] async fn rejects_prompt_policy_in_headless_mode() {}
#[tokio::test] async fn decrypts_local_credential_with_profile_bound_aad() {}
#[tokio::test] async fn reads_valid_system_credential() {}
#[tokio::test] async fn rejects_mismatched_keyring_reference() {}
#[tokio::test] async fn never_formats_secret_content_into_errors() {}
```

**Step 2: Run the resolver tests and verify failure**

Run: `cargo test --test credential_resolution`

Expected: FAIL because the reusable resolver does not exist.

**Step 3: Introduce `CredentialResolver`**

Move policy-specific logic out of the private TUI runtime function into a reusable service:

```rust
pub struct CredentialResolver {
    secret_store: Arc<dyn SecretStore>,
    local_store: LocalCredentialStore,
}

impl CredentialResolver {
    pub async fn resolve_headless(
        &self,
        profile: &ConnectionProfile,
    ) -> Result<Option<SecretString>, CredentialResolutionError>;
}
```

`Prompt` must fail with a structured `credential_interaction_required` error. Do not read generic `LAZYDB_PASSWORD` for agent commands because one environment variable is unsafe with multiple visible profiles. Existing TUI startup behavior remains unchanged.

**Step 4: Make TUI runtime delegate persisted credential resolution**

Preserve the TUI's session/startup-password precedence, then delegate `LocalEncrypted` and `System`/legacy `Keyring` cases to the shared resolver. Keep existing sanitized user-facing messages so unrelated UI tests do not change.

**Step 5: Run focused and regression tests**

Run: `cargo test --test credential_resolution --test profile_runtime --test secret_store`

Expected: PASS.

**Step 6: Commit**

```bash
git add src/persistence/credentials.rs src/persistence/mod.rs src/runtime.rs tests/credential_resolution.rs tests/profile_runtime.rs
git commit -m "refactor(credentials): share headless profile resolution"
```

### Task 4: Define Agent API Types, Target Identity, And Write Policy

**Files:**
- Create: `src/agent/types.rs`
- Create: `src/agent/policy.rs`
- Modify: `src/agent/mod.rs`
- Test: `tests/agent_policy.rs`
- Test: `tests/agent_serialization.rs`

**Step 1: Write failing serialization snapshots as value assertions**

Avoid brittle pretty-printed string snapshots. Assert `serde_json::Value` shapes for:

- API version.
- Project identity.
- Connection identity without secrets.
- Scope as `current_project` or `global`.
- Environment and effective read-only state.
- Structured success and error envelopes.
- Query values preserving NULL, empty text, bytes, date/time, and unsupported values.

**Step 2: Write the policy matrix tests**

Use existing `classify_sql` and assert:

| Profile | Environment | Write policy | SQL risk | Result |
| --- | --- | --- | --- | --- |
| read-only | any | all | DML | reject |
| writable | development | deny | DML | reject |
| writable | development | non-production | DML | allow |
| writable | staging | non-production | DDL | allow |
| writable | production | non-production | DML | reject |
| writable | production | all | DML | allow |
| any | any | any | read-only | allow through query path |
| any | any | any | unknown | reject from query path; require write path |
| any | any | any | transaction control | reject; no long-lived agent transactions |

**Step 3: Run tests and verify failure**

Run: `cargo test --test agent_policy --test agent_serialization`

Expected: FAIL because API types and policy are absent.

**Step 4: Add serializable API types**

Define `AgentResponse<T>`, `AgentErrorResponse`, `AgentContextResponse`, `AgentConnection`, `AgentTarget`, `AgentQueryResponse`, and duration fields in milliseconds. Convert `QueryOutcome` into agent-owned serializable types rather than adding protocol concerns to the database core.

Do not serialize `ConnectionProfile` directly. Explicitly project only safe fields.

**Step 5: Add `WritePolicy` and central authorization**

Add a Clap/Serde-compatible enum:

```rust
pub enum WritePolicy {
    Deny,
    NonProduction,
    All,
}
```

Implement separate checks:

```rust
authorize_query(profile, sql)
authorize_write(profile, write_policy, sql)
```

`authorize_query` accepts only a single statement whose aggregate is `ReadOnly`. `authorize_write` rejects `TransactionControl`, parser failures, and empty SQL; permits DML/DDL only under the matrix above. Multi-statement files may contain read-only plus DML/DDL but must contain no unknown or transaction-control statement.

Document in code that SQL classification is a routing/risk guard, not the final security boundary; the profile connection mode and database grants remain authoritative.

**Step 6: Run focused tests**

Run: `cargo test --test agent_policy --test agent_serialization --test sql_risk`

Expected: PASS.

**Step 7: Commit**

```bash
git add src/agent/types.rs src/agent/policy.rs src/agent/mod.rs tests/agent_policy.rs tests/agent_serialization.rs
git commit -m "feat(agent): define API and write policy"
```

### Task 5: Build The Headless `AgentService`

**Files:**
- Create: `src/agent/service.rs`
- Modify: `src/agent/mod.rs`
- Modify: `src/db/query.rs`
- Test: `tests/agent_service.rs`

**Step 1: Write SQLite service tests first**

Use temporary SQLite files and real `DatabaseConnection` calls. Cover:

```rust
#[tokio::test] async fn context_lists_project_then_global_and_excludes_other_project() {}
#[tokio::test] async fn query_returns_target_and_typed_rows() {}
#[tokio::test] async fn query_rejects_update_before_connecting() {}
#[tokio::test] async fn execute_change_obeys_profile_and_server_policy() {}
#[tokio::test] async fn result_row_limit_is_enforced() {}
#[tokio::test] async fn result_byte_limit_is_enforced_without_partial_cell_corruption() {}
#[tokio::test] async fn connection_is_closed_after_each operation() {}
```

Use defaults `max_rows = 500`, `max_result_bytes = 1 MiB`, and `statement_timeout = 10s`. Make limits constructor options in tests, not public MCP tool parameters in the first release.

**Step 2: Run tests and verify failure**

Run: `cargo test --test agent_service`

Expected: FAIL because `AgentService` is absent.

**Step 3: Implement service construction**

`AgentService::load` should accept explicit dependencies for tests and have a production constructor using:

- `AppPaths::discover()`.
- CLI `--config` override for profile file.
- `ProjectContext` resolution.
- `ProfileStore::load()`.
- `NativeSecretStore`.
- `LocalCredentialStore::from_paths()`.

Keep profile loading per process. The stdio MCP process can cache immutable profile metadata for its lifetime; users restart the MCP session after changing profiles in TUI in this first release.

**Step 4: Implement connection acquisition per operation**

For every operation:

1. Resolve only visible profiles.
2. Select deterministically.
3. Resolve credentials.
4. Apply policy before connection for rejected SQL.
5. Connect using `DatabaseConnection::connect`.
6. Probe canonical server identity where useful.
7. Execute one bounded operation.
8. Convert output to agent API types.
9. Close the connection.

Do not share TUI active pools, execution targets, or transaction workers.

**Step 5: Add result bounding**

The current adapters materialize `QueryOutcome` before returning. For the first release, enforce a server-generated outer query limit for read-only single SELECT/VALUES statements using the existing SQL execution-limit logic where possible, then enforce serialized byte limits after execution. If a dialect cannot be safely wrapped, reject an unbounded query rather than silently returning an unlimited result.

Return `truncated: true` and counts when rows are safely cut. Reject an oversized single cell with `result_too_large`; do not silently truncate values because schema/data verification depends on exact values.

**Step 6: Run service and adapter regression tests**

Run: `cargo test --test agent_service --test sqlite_adapter --test sql_execution`

Expected: PASS.

**Step 7: Commit**

```bash
git add src/agent/service.rs src/agent/mod.rs src/db/query.rs tests/agent_service.rs
git commit -m "feat(agent): add headless database service"
```

### Task 6: Add Progressive Schema Search And Object Description

**Files:**
- Modify: `src/agent/service.rs`
- Create: `src/agent/catalog.rs`
- Modify: `src/agent/types.rs`
- Test: `tests/agent_catalog.rs`

**Step 1: Write real SQLite catalog tests**

Create tables, view, index, foreign key, and trigger. Assert:

- Search returns bounded matching objects and canonical qualified names.
- Search respects profile `CatalogScope`.
- Describe returns columns, keys/indexes/foreign keys where supported, object kind, and adapter-owned DDL.
- Unknown and ambiguous qualified names return structured errors.
- Catalog responses include target identity.
- A request cannot forge another profile UUID in a catalog target.

**Step 2: Run tests and verify failure**

Run: `cargo test --test agent_catalog`

Expected: FAIL because schema methods do not exist.

**Step 3: Reuse existing catalog contracts**

Build `CatalogSearchRequest` using the selected profile UUID, profile `CatalogScope`, bounded page size, and an agent-local request ID. Call `DatabaseConnection::search_catalog`; do not issue generic `information_schema` SQL from the agent layer.

For description, resolve the object through catalog search/IDs, load relation children through existing catalog APIs, and use `relation_ddl`/`object_ddl`. Keep database-specific behavior in adapters.

**Step 4: Define progressive response shapes**

Return compact search entries first. `describe_object` returns detailed metadata only for one exact object. Do not add `get_full_schema` in this release.

**Step 5: Run catalog and adapter tests**

Run: `cargo test --test agent_catalog --test catalog_contract --test sqlite_adapter --test postgres_adapter --test mysql_adapter`

Expected: PASS. External database tests that are environment-gated may report skipped according to their existing convention.

**Step 6: Commit**

```bash
git add src/agent/service.rs src/agent/catalog.rs src/agent/types.rs tests/agent_catalog.rs
git commit -m "feat(agent): expose progressive schema inspection"
```

### Task 7: Add The Machine-Readable Agent CLI

**Files:**
- Modify: `src/cli.rs`
- Create: `src/agent/cli.rs`
- Modify: `src/agent/mod.rs`
- Modify: `src/main.rs`
- Test: `tests/agent_cli.rs`

**Step 1: Add Clap parser tests**

Test all command forms, mutual exclusion of `--sql`/`--file`, required input, default write policy, and inherited `--config`. Keep existing `version`, `capabilities`, and `doctor` parsing unchanged.

**Step 2: Add subprocess-level JSON tests**

Use `std::process::Command` with `env!("CARGO_BIN_EXE_lazydb")`; no new `assert_cmd` dependency is necessary. Test temporary SQLite profiles for:

- `agent connections` visibility.
- `agent context` ambiguity errors.
- `agent query` typed JSON.
- `agent execute` denied by default.
- `agent execute --write-policy non-production` on development SQLite.
- `--file` canonicalization and rejection outside project root.
- stdout containing only JSON and stderr containing diagnostics.
- documented exit codes.

**Step 3: Run tests and verify failure**

Run: `cargo test --test agent_cli`

Expected: FAIL because commands are not defined.

**Step 4: Extend the CLI command tree**

Add nested `AgentCommand` and `McpCommand` enums. Preserve synchronous `render_command` for existing informational commands. In `main.rs`, dispatch async agent/MCP commands before TUI startup.

Do not print MCP startup logs to stdout because stdout is the protocol transport. Route tracing to stderr.

**Step 5: Implement file safety**

Canonicalize the project root and SQL file. Require the file to be a regular file whose canonical path starts with the canonical project root. Apply a reasonable input-size cap, for example 1 MiB, before reading. Return `file_outside_project` or `sql_input_too_large` errors.

**Step 6: Update capability reporting**

Increment `CLI_API_VERSION` because the machine contract expands. Add `agent-json` and `mcp-stdio` to capabilities without breaking existing fields; if the fixed-size features array becomes awkward, change it to a serializable slice/vector and update its tests.

**Step 7: Run CLI regressions**

Run: `cargo test --test agent_cli --test startup_profiles && cargo run -- capabilities --json`

Expected: tests PASS; capabilities JSON advertises the new interfaces.

**Step 8: Commit**

```bash
git add src/cli.rs src/agent/cli.rs src/agent/mod.rs src/main.rs tests/agent_cli.rs
git commit -m "feat(cli): add coding agent database commands"
```

### Task 8: Add The Stdio MCP Server

**Files:**
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Create: `src/agent/mcp.rs`
- Modify: `src/agent/mod.rs`
- Modify: `src/main.rs`
- Test: `tests/agent_mcp.rs`

**Step 1: Pin the official Rust MCP SDK**

Add `rmcp = "3.1.4"` with only the features needed for a stdio server (`server`, macros, schemars, and stdio transport as required by the selected API). Do not enable HTTP client/server or OAuth features in this release. Add direct `schemars` only if the rmcp macros require user-owned schemas to derive it.

Run: `cargo check`

Expected: PASS with Rust 1.94; inspect `Cargo.lock` to ensure no unexpected HTTP stack was enabled.

**Step 2: Write protocol-level MCP tests before handlers**

Use rmcp's in-memory duplex/async read-write transport rather than spawning a shell where possible. Test:

- Initialize response identifies LazyDB and protocol capabilities.
- `tools/list` contains exactly the seven planned tools.
- Read tools carry read-only annotations.
- Write tools carry destructive/non-read-only annotations.
- `get_context` and `query` return structured content.
- Other-project selectors are rejected.
- `execute_change` honors `WritePolicy::Deny` and `profile.read_only`.
- Tool errors use stable error codes without credentials.
- Server shutdown closes cleanly at EOF.

**Step 3: Run tests and verify failure**

Run: `cargo test --test agent_mcp`

Expected: FAIL because the MCP server is absent.

**Step 4: Implement a thin MCP handler**

Construct one `AgentService` at startup with immutable project path, optional fixed connection selector, and write policy. Tool arguments may include a connection selector only when the server was not started with `--connection`; a fixed server connection cannot be overridden per call.

Map tools directly:

```text
get_context       -> AgentService::context
list_connections  -> AgentService::connections
search_schema     -> AgentService::search_schema
describe_object   -> AgentService::describe_object
query             -> AgentService::query
execute_change    -> AgentService::execute
execute_file      -> safe project file read + AgentService::execute
```

Return structured JSON content plus a concise human-readable summary. Never return connection URLs or credential references.

**Step 5: Verify stdout protocol purity**

Add a subprocess test that starts `lazydb mcp serve`, sends initialize and tools/list JSON-RPC messages, and proves no banner/log line precedes protocol output.

**Step 6: Run MCP and full agent tests**

Run: `cargo test --test agent_mcp --test agent_service --test agent_policy`

Expected: PASS.

**Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock src/agent/mcp.rs src/agent/mod.rs src/main.rs tests/agent_mcp.rs
git commit -m "feat(mcp): serve project-scoped database tools"
```

### Task 9: Document Codex, OpenCode, And Claude Code Configuration

**Files:**
- Create: `docs/coding-agent-access.md`
- Modify: `docs/configuration.md`
- Modify: `README.md`
- Modify: `CHANGELOG.md`

**Step 1: Document the visibility and selection contract**

State explicitly:

```text
visible = current-project profiles + global profiles
hidden = profiles assigned only to other projects
```

Explain ambiguity behavior, project-root canonicalization, profile refresh requiring MCP restart, and why global visibility does not imply automatic selection.

**Step 2: Document the three permission layers**

Include this table:

| Layer | Responsibility |
| --- | --- |
| Database role/grants | Final data and write authority |
| LazyDB profile + MCP `--write-policy` | Server-side operation ceiling |
| Agent MCP permission config | Tool visibility and human approval |

Warn that MCP annotations and `AGENTS.md` are guidance, not database authorization.

**Step 3: Add Codex project configuration**

Provide a checked example using `.codex/config.toml`:

```toml
[mcp_servers.lazydb]
command = "lazydb"
args = ["mcp", "serve", "--project", ".", "--write-policy", "deny"]
required = true
enabled_tools = [
  "get_context",
  "list_connections",
  "search_schema",
  "describe_object",
  "query",
]
```

Add a separate writable-development example where write tools are enabled and configured to prompt. Do not include production-write examples as copy-paste defaults.

**Step 4: Add OpenCode project configuration**

Provide `opencode.json` with local MCP `cwd`, project command, and permissions that allow read tools and ask for write tools. Explain that OpenCode merges global and project MCP config, so users should disable obsolete global database MCP servers to eliminate duplicate tools.

**Step 5: Add Claude Code project configuration**

Provide `.mcp.json` plus `.claude/settings.json`. Use `project` scope semantics, allow read tools, and ask for write tools. Document `--strict-mcp-config` for noninteractive automation.

**Step 6: Document CLI examples and error handling**

Show `context`, `connections`, bounded query, schema search, describe, and development SQL-file execution. Include JSON error examples for ambiguity and hidden connections.

**Step 7: Verify documentation commands**

Run every non-destructive command against a temporary SQLite project. Validate all JSON snippets with `jq` when installed, otherwise `python -m json.tool` only for verification, not file editing.

Expected: examples execute as documented; no secret appears in output.

**Step 8: Commit**

```bash
git add docs/coding-agent-access.md docs/configuration.md README.md CHANGELOG.md
git commit -m "docs: explain coding agent database access"
```

### Task 10: Security, Concurrency, And Release Verification

**Files:**
- Create: `tests/agent_security.rs`
- Modify: `docs/architecture.md`
- Modify: `docs/database-capabilities.md`

**Step 1: Add adversarial visibility tests**

Prove that:

- A known UUID for another project's profile behaves as not found.
- `..`, symlinks, and alternate path spellings cannot escape the project for `execute_file`.
- Duplicate aliases cannot select a connection by registry order.
- A global profile remains visible from nested project directories.
- A project profile is not visible from a sibling Git worktree.

**Step 2: Add adversarial SQL policy tests**

Cover CTE writes, `SELECT ... FOR UPDATE`, `SELECT INTO`, multiple statements, transaction control, malformed SQL, and side-effecting function caveat. Assert database grants/profile read-only remain documented as the final boundary.

**Step 3: Add secret-redaction tests**

Seed passwords in local encrypted and fake system stores. Trigger connection, policy, and database errors. Search serialized responses, stderr capture, and MCP error data for the secret value and credential ciphertext; assert neither appears.

**Step 4: Add concurrent MCP call tests**

Run parallel read calls against a temporary SQLite database and assert each operation has independent acquisition and target metadata. Run concurrent write calls and assert SQLite/database locking errors are returned cleanly without corrupting the server session. The first release intentionally has no shared transaction handles.

**Step 5: Run focused security tests**

Run: `cargo test --test agent_security --test agent_mcp --test agent_cli`

Expected: PASS.

**Step 6: Update architecture and capability docs**

Document the new boundary:

```text
CLI / MCP adapter
       |
AgentService
  | project visibility
  | deterministic selection
  | credential resolution
  | SQL/write policy
  | result limits
       |
DatabaseConnection
```

State that MCP sessions do not reuse TUI pools or transaction workers.

**Step 7: Run repository verification**

Run:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --release
```

Expected: all commands PASS.

**Step 8: Perform manual smoke tests in all three clients**

For Codex, OpenCode, and Claude Code:

1. Start in a temporary Git project.
2. Verify current-project and global profiles are listed.
3. Verify another-project profile is absent.
4. Query a temporary SQLite database.
5. Verify a write is denied under `--write-policy deny`.
6. Restart with `--write-policy non-production` and verify the client prompts before its write tool.
7. Verify the tool result identifies the actual target.

Record client versions and results in the PR/release notes; do not add client-generated local config containing machine paths or secrets to the repository.

**Step 9: Commit**

```bash
git add tests/agent_security.rs docs/architecture.md docs/database-capabilities.md
git commit -m "test(agent): harden database access boundaries"
```

## Final Acceptance Criteria

- From any nested directory, LazyDB resolves the same canonical project root as the TUI.
- Agent-visible profiles include current-project and global profiles and exclude all other-project-only profiles.
- A hidden profile cannot be selected even with its exact UUID.
- No operation selects the first profile merely because it is first in storage.
- All JSON CLI output is versioned, structured, target-attributed, bounded, and secret-free.
- MCP stdout contains only protocol messages.
- Query and write are separate MCP tools.
- Profile `read_only` cannot be relaxed by MCP configuration.
- `--write-policy deny` is the MCP default.
- `--write-policy non-production` cannot write a production profile.
- Database role/grants remain the final authorization boundary.
- SQL files cannot escape the current project root.
- No MCP call shares an implicit transaction with another call or with the TUI.
- Codex, OpenCode, and Claude Code project examples work without embedding credentials.
- Existing TUI startup, profile, credential, catalog, and SQL execution tests continue to pass.

## Deferred Follow-Ups

Create separate designs only after the first release is used in real projects:

1. Persisted per-profile `agent_access` capabilities and TUI controls.
2. Explicit project default connection binding when multiple project profiles are common.
3. Profile hot reload for long-lived MCP sessions.
4. Remote Streamable HTTP MCP with OAuth and per-user identities.
5. Central audit log and `application_name` correlation.
6. Temporary production read grants and expiry.
7. Migration proposal/dry-run/approval runner.
8. Ephemeral database branches per coding-agent session.
9. Per-session query quotas and isolated workers for high-risk connections.
