# Snowflake

LazyDB connects to Snowflake through the HTTPS SQL API. No Snowflake client
library is required. Use the full account hostname, certificate-verified TLS,
and port `443`.

## Connect

Create a **Relational → Snowflake** profile, select the database and default
schema, then enter an **Access token**. The schema defaults to `PUBLIC`.
Warehouse and role overrides can be supplied in the URL:

```text
snowflake://alice@org-account.snowflakecomputing.com/ANALYTICS?schema=PUBLIC&warehouse=COMPUTE_WH&role=ANALYST
```

Use the account's complete hostname, including a private-connectivity hostname
when appropriate. LazyDB does not turn an account locator into a hostname.
The database and schema remain separate; relation names can be fully qualified
as `"DATABASE"."SCHEMA"."TABLE"`.

A URL may contain a token in its password position:

```text
snowflake://alice:URL_ENCODED_TOKEN@org-account.snowflakecomputing.com/ANALYTICS?schema=PUBLIC
```

This is a placeholder, not a real credential. Percent-encode reserved characters
in tokens and identifiers. Import extracts the token into the existing secret
credential flow. Generated URLs omit it. Prefer entering the token in the form
to avoid placing it in shell history. Choose local encrypted or system
credential storage as with other connections.

Supported URL parameters are `schema`, `warehouse`, `role`, `token_type`, and
the shared `readOnly`/`read_only` flag. Unknown parameters, duplicate options,
empty option values, non-443 ports, and TLS downgrade settings are rejected.
Snowflake profiles require `ssl_mode = "verify-full"`.

## Authentication

LazyDB accepts bearer tokens that you obtain outside the application:

| `token_type` | Setup |
| --- | --- |
| `PROGRAMMATIC_ACCESS_TOKEN` (default) | Generate a Snowflake programmatic access token for the intended user and role. Meet the account's authentication and network-policy requirements. |
| `OAUTH` | Obtain a valid Snowflake OAuth access token with the required role scope, then set `token_type=OAUTH`. |
| `KEYPAIR_JWT` | Configure the user's public key in Snowflake and generate the signed JWT externally, then set `token_type=KEYPAIR_JWT`. Enter the JWT as the access token. |

LazyDB does not perform browser login, generate JWTs, load private keys, refresh
OAuth tokens, or rotate PATs. Replace expired tokens through the profile's access
token field. Warehouse and role names are optional; Snowflake applies the
account/user defaults when they are omitted. The chosen role needs permission
to use the database, schema, warehouse, and queried objects.

The saved `snowflake_options` table contains only `warehouse`, `role`, and
`token_type`. The token itself uses `credential_policy`. Profile form edits,
URL edits, and execution-target changes retain these options. Changing these
options invalidates catalog discovery and causes the runtime to reconnect.
Visible-object scope changes also retire existing Snowflake connections so the
new connection uses the updated bounds. Save and connect rebuilds the connection;
a save-only action leaves it disconnected until the next connection request.

## Supported workflows

- Browse databases, schemas, tables, views, and column metadata within the
  configured visible-object scope.
- Search the catalog, preview bounded relation pages, and read native DDL.
- Execute SQL with explicit database, schema, warehouse, and role context.
- Use Snowflake parsing, formatting, highlighting, and completion, including
  three-part relation names, variant paths, `QUALIFY`, and `$$` statement bodies.
- Use the shared query row/byte limits for synchronous, asynchronous, and
  partitioned SQL API responses.

Structured row editing, structured catalog mutation, principal management,
monitoring, manual transactions, and server cancellation are not advertised.
The SQL editor can run explicitly requested SQL allowed by Snowflake's grants;
LazyDB's read-only and write-policy checks are additional application controls.

## SQL API session limits

Each SQL API request accepts one statement (`MULTI_STATEMENT_COUNT=1`). Execute
one statement at a time; a selection containing multiple statements is not a
supported batch. There is no persistent console session: do not depend on
`USE`, session variables, temporary objects, or
`BEGIN`/`COMMIT` state carrying over to the next request. Select the execution
target in LazyDB and set warehouse/role on the profile instead.

Async SQL API responses are polled and result partitions are fetched within
bounded request/result budgets. Mutating requests are not automatically
replayed after ambiguous network failures. Ending a local request does not
confirm that Snowflake stopped executing it; inspect Snowflake query history
before retrying a write whose outcome is unknown.

## Verification limits

The implementation has local regression coverage for URL/options, token
redaction, SQL dialect behavior, and SQL API transport/catalog behavior through
mock responses.

On 2026-11-01, an isolated Snowflake trial account was exercised with a dedicated
read-only role and service user using an externally generated key-pair JWT.
Validation covered database/schema discovery (including `SHOW DATABASES`),
table/view and column metadata, search, native DDL, lookahead and exact-count
pagination, quoted schema execution targets, NULL, NUMBER(38,18), temporal values,
BINARY, VARIANT, SQL errors, and a 20,000-row result returned in six partitions.
The account test exposed native epoch timestamps in SHOW metadata even with
output-format parameters; the decoder now supports both representations without
floating-point rounding.

A second isolated run used a role-restricted PAT and a user-level network policy
limited to the test client's outbound IP. The same adapter workflows passed,
including binary decoding with the user's `BINARY_OUTPUT_FORMAT=BASE64` default;
each request explicitly selects HEX output. Invalid PATs returned HTTP 401, revoked PATs were rejected, and the read-only
role's attempted table creation returned SQLSTATE `42501`.
A separate synthetic write probe confirmed that an INSERT persists across
independent SQL API requests even when the user's `AUTOCOMMIT=FALSE`. AUTOCOMMIT
is not an allowed SQL API request parameter and is deliberately omitted.

The actual TUI was driven with an isolated configuration through URL import,
masked token entry, encrypted credential storage, restart and reconnect,
catalog browsing, native DDL, SQL execution, and 10-row pagination ending at
rows 121–125 of 125.

A third isolated run on 2026-11-01 used the official v0.3.1 macOS ARM64 binary
with Snowflake OAuth: a temporary PUBLIC custom integration, PKCE/S256, an
explicit read-only role scope, and a 60-second access-token lifetime. Real
queries returned `42` with the intended role, while access to an ungranted
schema was denied. The access token naturally expired (HTTP 401, code `390318`).
A replacement obtained by an external refresh request was saved to the same
profile with local encrypted storage; a new process restored it and queried
successfully. Removing only that role's delegated authorization from the test
integration rejected a newly issued token (HTTP 401, code `390303`). The test
integration, role grants, database, warehouse, and local credential material
were removed afterward. This validates externally supplied OAuth tokens; it
does not add browser login or automatic refresh to LazyDB.

Known token failures are categorized as authentication errors even when
Snowflake returns a server status: the previously observed revoked PAT response
was HTTP 500 with code `390111`. Unknown server errors retain their network
classification. Token edits invalidate the draft's catalog discovery and reject
results from the previous credential revision; the selected visible-object
scope is preserved.

Natural PAT expiry remains unverified against a real account. Snowflake accepts
`DAYS_TO_EXPIRY` as whole days with a minimum of 1, so it cannot be shortened to
seconds for this check. Revocation and an expired OAuth token do not establish
PAT natural-expiry coverage. These checks do not cover every terminal emulator
or account policy configuration.

`tests/snowflake_live.rs` is an ignored, opt-in integration test. It reads a
credential-free `LAZYDB_SNOWFLAKE_TEST_URL` and the access token from
`LAZYDB_SNOWFLAKE_TOKEN_FILE`; token contents must not be placed in the URL or
committed. The test expects the synthetic `Type Samples`, `Sample View`, and
`CaseSchema.\"Quoted Table\"` fixtures and a 125-row type sample table. Recreate
those fixtures in an isolated test database using
[`tests/fixtures/snowflake/setup.sql`](../tests/fixtures/snowflake/setup.sql), then
configure the two environment variables above before running:

```sh
cargo test --test snowflake_live snowflake_live_readonly_workflows -- --ignored --exact
```

For a natural-expiry run, create a dedicated PAT with `DAYS_TO_EXPIRY=1`,
record its server-reported expiration, and keep it in a protected file until
that time has passed. Keep the isolated role/user/network-policy resources
available for the check; do not change the machine clock, alter the token, or
revoke it to simulate expiration. Obtain a valid replacement for the same role
and set the credential-free `LAZYDB_SNOWFLAKE_TEST_URL`,
`LAZYDB_SNOWFLAKE_EXPIRED_TOKEN_FILE`, and `LAZYDB_SNOWFLAKE_TOKEN_FILE` paths:

```sh
cargo test --test snowflake_live snowflake_live_expired_token_and_replacement -- --ignored --exact
```

This test checks authentication failure, replacement connectivity, `SELECT 42`,
and fresh database/schema discovery. It does not check profile persistence or
prove why the old token failed: record that it worked before its expiration and
was never revoked. Also verify replacement through the profile's access-token
field, local encrypted storage, restart/reconnect, and visible-object refresh.
Revoke remaining test tokens, drop only the dedicated resources, and remove
local secrets after acceptance. Until that timed run is recorded, keep
[#36](https://github.com/yelog/lazydb/issues/36) open.

Reference: [Snowflake OAuth custom clients](https://docs.snowflake.com/en/user-guide/oauth-custom)
and [PAT creation and expiration](https://docs.snowflake.com/en/sql-reference/sql/alter-user-add-programmatic-access-token).

See the [capability matrix](database-capabilities.md#snowflake) and
[profile configuration](configuration.md#snowflake-profiles).
