# SSH Tunnels

LazyDB can start and manage an SSH tunnel for a saved connection profile. It
uses the system OpenSSH client, so existing keys, ssh-agent identities, SSH
config aliases, host trust, and ProxyJump rules keep their normal meaning.

The database profile continues to contain the database's real hostname and
port. LazyDB changes only the physical transport used by the driver:

```text
LazyDB -> managed local socket/TCP/SOCKS -> system ssh -> bastion
       -> database host and port
```

This is different from an external port-forward workaround. No loopback
address is written back to the profile, URL, or workspace. If you already use
an external tunnel, see [External OpenSSH forwarding](#external-openssh-forwarding).

## Requirements

- Install an OpenSSH client and make `ssh` available on `PATH`. On Windows,
  LazyDB also checks the system OpenSSH location under
  `%SystemRoot%\System32\OpenSSH\`.
- Make sure the database host and port are reachable from the final bastion.
  For SSH TCP and Unix socket forwarding, the database hostname is resolved from
  the bastion.
- Complete SSH trust and authentication before connecting. LazyDB runs SSH
  non-interactively: it does not show password, passphrase,
  keyboard-interactive, MFA, or unknown-host-key prompts.
- Verify every host in the chain. LazyDB requires strict host-key checking and
  will not accept a new key automatically.

LazyDB does not bundle, download, or replace the OpenSSH client. It also does
not store SSH private keys or agent material in a connection profile.

## Quick Start

Create the profile with the database's normal connection values. Set
**Host** and **Port** to the address the database uses from the bastion, not
to a local forwarded port.

In the Profile Manager, open **SSH TUNNEL**, enable **SSH tunnel**, and fill in:

| Field | Meaning |
| --- | --- |
| **SSH host / alias** | Bastion hostname, IP address, or an alias from the SSH config. Required. |
| **SSH user** | Optional SSH username. If omitted, OpenSSH uses its config or client default. |
| **SSH port** | Optional SSH port. If omitted, OpenSSH uses its config or default port 22. |
| **SSH key file** | Optional private-key path. If omitted, OpenSSH uses the config, agent, and its normal identity discovery. |
| **SSH config file** | Optional `ssh_config` file. If omitted, OpenSSH uses its normal per-user configuration. |
| **SSH proxy jump** | Optional comma-separated ProxyJump destinations, for example `ops@jump.example.com:2200`. |
| **Known hosts file** | Optional host-key database. If omitted, OpenSSH uses its normal `known_hosts` files. |

Save the profile and use **Test Connection** or connect normally. The attempt
uses one total connection deadline covering SSH startup and the database
connection. The UI continues to show its general `TESTING CONNECTION` state;
separate SSH and database phase events are not exposed.

### CLI

The same settings are available on `connections add`. The database URL still
contains the logical database endpoint:

```sh
lazydb connections add \
  --name orders-via-bastion \
  --url 'postgresql://app_user@orders-db.internal:5432/orders' \
  --scope global \
  --password-env PGPASSWORD \
  --ssh-host bastion.example.com \
  --ssh-user alice \
  --ssh-port 2222 \
  --ssh-identity-file ~/.ssh/id_ed25519 \
  --ssh-config-file ~/.ssh/config \
  --ssh-known-hosts-file ~/.ssh/known_hosts
```

Test it without opening the TUI. The test deadline covers SSH startup and the
database probe:

```sh
lazydb connections test orders-via-bastion --timeout 30
```

When updating an existing profile, explicitly supplied SSH options replace
those fields and unspecified SSH fields remain unchanged:

```sh
lazydb connections add \
  --name orders-via-bastion \
  --url 'postgresql://app_user@orders-db.internal:5432/orders' \
  --scope global \
  --upsert \
  --ssh-proxy-jump 'ops@jump.example.com:2200'
```

Remove the complete SSH configuration with `--no-ssh`:

```sh
lazydb connections add \
  --name orders-via-bastion \
  --url 'postgresql://app_user@orders-db.internal:5432/orders' \
  --scope global \
  --upsert \
  --no-ssh
```

`--no-ssh` cannot be combined with another SSH option. If no SSH option is
provided during an upsert, the saved SSH settings are retained.

An ad-hoc `lazydb --url ...` session has no SSH settings. Create or update a
saved profile first.

## Configuration Files

SSH settings belong to each entry in `connections.toml`. The optional
`[profiles.ssh]` table is absent when the tunnel is disabled:

```toml
version = 6

[[profiles]]
id = "1c73c7c0-f944-4adc-a73a-1265fe1260a9"
name = "orders-via-bastion"
kind = "postgres"
url_format = "postgresql"
host = "orders-db.internal"
port = 5432
user = "app_user"
database = "orders"
default_schema = "public"
ssl_mode = "verify-full"
credential_policy = { policy = "prompt" }
read_only = false
environment = "production"
catalog_scope = { databases = { mode = "all" } }

[profiles.ssh]
host = "bastion.example.com"
port = 2222
user = "alice"
identity_file = "~/.ssh/id_ed25519"
config_file = "~/.ssh/config"
proxy_jump = "ops@jump.example.com:2200"
known_hosts_file = "~/.ssh/known_hosts"
```

All fields except `host` are optional. Explicit key, config, and known-hosts
paths must exist when the tunnel starts. A leading `~` is expanded to the
current user's home directory.

Application-wide SSH limits live in `settings.toml`:

```toml
[connections.ssh]
connect_timeout_seconds = 30
server_alive_interval_seconds = 30
server_alive_count_max = 3
```

| Setting | Range | Meaning |
| --- | --- | --- |
| `connect_timeout_seconds` | 1-300 | Total deadline for SSH startup and the database connection. |
| `server_alive_interval_seconds` | 1-3600 | Interval between OpenSSH keepalive messages. |
| `server_alive_count_max` | 1-10 | Missed keepalive responses allowed before OpenSSH disconnects. |

The embedded defaults use a 30-second connection deadline and a keepalive every
30 seconds, with three missed responses. A shorter outer deadline, such as
`connections test --timeout`, takes precedence. Normal non-SSH connections keep
their existing 10-second cold-connect deadline. `connections test` still
defaults to 10 seconds even for an SSH profile, so use `--timeout 30` or another
explicit value for a slower bastion.

## Authentication and Host Trust

The managed client uses the same OpenSSH mechanisms as a normal terminal
session:

- an unencrypted key selected by `identity_file`;
- a key already unlocked in ssh-agent;
- `IdentityFile`, `IdentityAgent`, `User`, `Port`, and other settings from
  SSH config;
- ProxyJump from the profile or SSH config.

An encrypted key that is not already unlocked in the agent cannot be prompted
for by LazyDB, so the connection fails. Depending on the OpenSSH client and key
state, the error may be `ssh_interaction_required` or
`ssh_authentication_failed`. Unlock it in the same user session first:

```sh
ssh-add ~/.ssh/id_ed25519
ssh-add -l
```

Password authentication, key passphrase prompts, keyboard-interactive
authentication, and MFA are not implemented in the application. Use an agent,
an unencrypted key, or external forwarding from a separate terminal.

LazyDB sets `BatchMode=yes`, disables password prompts, disables askpass, gives
SSH a null stdin, and uses strict host-key checking. To trust a new bastion,
verify its fingerprint through a trusted channel and add it with your normal
OpenSSH workflow. For example:

```sh
ssh-keyscan -p 2222 bastion.example.com > /tmp/bastion.keys
ssh-keygen -lf /tmp/bastion.keys
# Compare the displayed fingerprints with the values from your administrator.
cat /tmp/bastion.keys >> ~/.ssh/known_hosts
```

Do not use `StrictHostKeyChecking=no` or delete a changed host key merely to
make the connection succeed. A changed key fails with
`ssh_host_key_changed` and requires an explicit trust decision outside LazyDB.

An SSH config alias can hold most connection settings:

```sshconfig
Host orders-bastion
  HostName bastion.example.com
  User alice
  Port 2222
  IdentityFile ~/.ssh/id_ed25519
  ProxyJump ops@jump.example.com:2200
```

Then use `host = "orders-bastion"` in the profile. A complex ProxyCommand or
`Match exec` rule also comes from the user's SSH config. Treat that config as
trusted local code because OpenSSH may execute it.

LazyDB evaluates the effective SSH config before forwarding. If the selected
alias inherits `LocalForward`, `RemoteForward`, or `DynamicForward` entries,
the connection is rejected. Use a dedicated alias without extra forwards so a
tunnel cannot silently expose additional ports.

The profile's `proxy_jump` accepts comma-separated destinations:

```toml
proxy_jump = "ops@jump.example.com:2200,admin@[2001:db8::10]:22"
```

All hosts in a ProxyJump chain must also be trusted and usable without an
interactive prompt.

## Transport and TLS Behavior

The profile's database hostname is kept for authentication, logs, and, where
the selected transport supports it, the TLS server name and HTTP authority.
Only the physical endpoint changes:

| Driver family | Managed transport | TLS behavior |
| --- | --- | --- |
| PostgreSQL, CockroachDB | TCP for `disable`; SQLx Unix socket for `prefer` / `require` / `verify-ca` / `verify-full` on macOS/Linux; TCP for `prefer` / `require` on Windows, with strict modes rejected | Unix transport keeps the original hostname for SNI and certificate identity checks. Windows non-verifying TLS uses the loopback IP as its TLS server name. |
| MySQL, MariaDB, TiDB | TCP for `disable`; SQLx Unix socket for `prefer` / `require` / `verify-ca` / `verify-full` on macOS/Linux; TCP for `prefer` / `require` on Windows, with strict modes rejected | Unix transport keeps the original hostname for SNI and certificate identity checks. Windows non-verifying TLS uses the loopback IP as its TLS server name. |
| SQL Server | TCP | Tiberius keeps the logical host in its TLS configuration and dials the forwarded TCP socket. |
| Redis, Valkey | TCP with a host resolver mapped to the forwarded endpoint | The original hostname remains the TLS server name; AUTH and SELECT are preserved. |
| ClickHouse, Elasticsearch, OpenSearch | OpenSSH SOCKS5 | The original URL, Host header, authority, SNI, and certificate hostname remain in use. |
| MongoDB | OpenSSH SOCKS5 | Drivers use SOCKS for topology, business, and monitoring connections; TLS keeps the logical seed hostnames. |

For the SQLx drivers, `disable` uses TCP forwarding because no TLS server name
is involved. On macOS/Linux, every mode that can negotiate TLS (`prefer`,
`require`, `verify-ca`, and `verify-full`) uses a private SQLx Unix socket so
the driver keeps the original hostname for SNI. This preserves compatibility
with servers that select a certificate by SNI, including non-verifying modes.
The trade-off is that TLS workloads pay the measured Unix-socket cost; only
`disable` gets the lower-overhead TCP path.

SQLx on Windows cannot connect through the private Unix socket. Profiles using
`prefer` or `require` therefore use TCP forwarding there, which keeps the
encryption and non-verification semantics but sends the loopback IP as the TLS
server name. A server that requires SNI-based certificate selection can reject
that handshake. LazyDB never weakens identity verification to make this work:
Windows profiles using `verify-ca` or `verify-full` through SSH are rejected.

PostgreSQL profiles can supply a CA and client certificate through
`pg_options`. MySQL, MariaDB, and TiDB profiles do not expose CA or client
certificate fields, so a custom CA for strict managed TLS must be available
through the process trust store. The recorded MySQL route proof passed the CA
directly to SQLx; it does not add a MySQL CA field to saved profiles.

The SQLx Unix socket transport is local to LazyDB and points at the bastion's
forwarded database destination. It is not a way to select an arbitrary Unix
socket on the database server. Redis Cluster and Sentinel are not supported
through this transport.

### DNS and MongoDB SRV

For TCP and Unix socket forwarding, the database hostname is looked up from the
final bastion. The SSH host itself is still resolved locally.

SOCKS5 forwarding preserves the database or service hostname for HTTP and
MongoDB. It does not proxy SRV or TXT DNS queries. A `mongodb+srv://` URL still
requires the LazyDB process to resolve the SRV and TXT records locally before
the driver can connect. If those records are unavailable from the workstation,
use explicit seed hosts instead; LazyDB does not provide remote DNS or a DNS
server over the tunnel.

Database certificate verification remains independent of SSH host-key
verification. A trusted bastion does not make an untrusted database
certificate valid.

## Lifecycle and Cleanup

LazyDB starts SSH on demand and shares it among connections that use the same
profile, effective SSH settings, forwarding mode, and destination. Database
names, schemas, Consoles, Explorer requests, and discovery do not each start a
new process. Different profiles do not share a tunnel.

- Closing one tab, Console, or database does not stop a tunnel that is still
  used by another connection or background task.
- The final lease stops the managed process and removes its private Unix socket
  directory or loopback listener.
- A failed bind is retried up to three times within the same connect deadline.
- Normal application shutdown stops new work and reaps owned SSH processes,
  including the process tree where the platform supports it.
- External changes to SSH config or agent contents are not watched. Close and
  reopen affected connections to evaluate them again.

An abrupt `SIGKILL`, terminal-session teardown, or power loss can prevent normal
cleanup and leave an `ssh` child behind. LazyDB does not use a persistent
guardian or a bare PID file because either can introduce its own cleanup and
PID-reuse failures. Inspect the exact process before terminating it if a forced
shutdown leaves one running.

If the tunnel exits while a connection is active, the transport fails closed.
Once the supervisor has observed the exit, guarded operations report
`ssh_tunnel_exited` or `ssh_tunnel_unavailable`, and SQLx, SQL Server, and
MongoDB pools or clients are closed instead of being allowed to reconnect
through a replacement process. An operation that races the exit before it is
observed can instead report a driver or network error from the broken local
transport. LazyDB does not silently reconnect directly or replay SQL,
transactions, or writes.

Fail-closed routing is based on the managed process state LazyDB has observed.
There is a short interval between the operating system releasing a TCP port and
the supervisor recording the SSH exit. LazyDB does not claim atomic protection
against another local process binding that port during this interval.

## Updating or Removing a Tunnel

Edit the saved profile and close/reconnect its open sessions for the new
settings to take effect. An existing lease remains valid until its last user
releases it.

To disable the tunnel in the Profile Manager, turn off **SSH tunnel** and save.
From the CLI, use `--upsert --no-ssh` as shown above. Closing all sessions that
use the old lease then releases the process and local endpoint.

`connections list`, `connections show`, and their JSON output expose only a
safe SSH summary such as host, port, and user. They do not print private-key
contents, passwords, raw SSH diagnostics, or the complete remote diagnostic
buffer.

## Verification Status

The complete recorded Docker run used macOS arm64 with
`OpenSSH_10.5p1, LibreSSL 3.3.6` as the client. It passed 47 Rust integration
tests plus the real PTY check:

| Scenario | Recorded result |
| --- | --- |
| OpenSSH process and forwarding | `tests/ssh_integration.rs` passed 6/6 for Unix, TCP, and SOCKS forwarding, an occupied port, a wrong private key, and a wrong host key. |
| PostgreSQL and MySQL managed transport | `tests/ssh_transport.rs` passed 18/18. It covered strict TLS and TLS negative cases, required-TLS Unix forwarding, connection reuse, pool-saturation cancellation, transactions, multiple database targets, an agent-only key, 20 concurrent waiters with independent cancellation, and rejection of rewiring an old SQLx route to a replacement tunnel. |
| SQL Server | `tests/ssh_sqlserver.rs` passed 2/2 for pool reuse, transactions, database targets, and rejection of a stale endpoint. The pinned SQL Server image ran as `linux/amd64` under Docker emulation. |
| Redis and Valkey | `tests/ssh_redis.rs` passed 6/6 for TLS, AUTH, SELECT, scan/delete, ProxyJump, connector shutdown, and tunnel-crash handling. |
| HTTP and HTTPS through SOCKS | `tests/ssh_http.rs` passed 5/5 for ClickHouse HTTP and Elasticsearch/OpenSearch HTTPS with logical endpoints, strict TLS, and a wrong CA. |
| MongoDB replica set through SOCKS | `tests/ssh_mongo.rs` passed 6/6 against a three-member `rs0` set with different hostnames and ports. It covered TLS plus SCRAM-SHA-256, wrong-password rejection, discovery and explicit three-seed connection, primary stepdown with failover, transaction read/abort, and tunnel-crash handling. |
| CLI, Agent, MCP, and LSP services | `tests/ssh_services.rs` passed 4/4 through the managed PostgreSQL route: `connections test`, Agent query, MCP tool discovery/query with clean EOF exit, and LSP initialization/catalog/stdio framing/shutdown. |
| Real TUI PTY | The saved SSH PostgreSQL profile passed SQL execution, history persistence, clean exit, and tunnel reaping. |

The MongoDB failover test does not replay an uncertain write. In the tunnel-crash
test, a live MongoDB session caused the background client to terminate, and a
replacement listener bound to the released loopback port was rejected. The
rejection is based on the failure state observed by the supervisor, not an
atomic guarantee against local port reuse during the exit interval. The service
4/4 result is from subprocess protocol tests; the PTY result is recorded
separately.

The SQL Server result used an emulated `linux/amd64` container on macOS arm64.
It establishes the driver and tunnel path, not native Linux or Windows client
behavior.

This is not a general platform or topology certification. Linux and Windows
native clients have not been recorded as accepted, including their OpenSSH
variants and cleanup behavior. HTTP IP-SAN and IPv6 endpoints have not been
recorded. CockroachDB, MariaDB, and TiDB use shared transport implementations
but were not separately recorded here. The MongoDB result covers this
three-member replica set, not SRV discovery, sharded clusters, or a broader
failover matrix.

The exact fixture commands and environment contract are in
[`tests/fixtures/ssh/README.md`](../tests/fixtures/ssh/README.md).

## Current Limitations

- Oracle, SQLite, DuckDB, BigQuery, and Snowflake profiles cannot enable SSH.
  Oracle and the managed HTTPS drivers retain their existing connection paths.
- SSH password, key passphrase, keyboard-interactive, and MFA prompts are not
  implemented.
- LazyDB does not accept unknown host keys or automatically replace changed
  keys.
- SQLx on Windows rejects `verify-ca` and `verify-full` through SSH. Its
  `prefer` and `require` TCP fallback cannot preserve SNI because the TLS
  server name becomes the loopback IP.
- Redis Cluster and Sentinel are not supported through a profile tunnel.
- MongoDB SRV/TXT discovery still requires local DNS. The recorded failover is
  a single primary stepdown in the three-member TLS fixture; SRV discovery,
  sharded clusters, and a broader failover matrix are not in the acceptance.
- SQL Server SSH acceptance used an emulated `linux/amd64` container on macOS
  arm64. Linux and Windows native-client behavior is unverified.
- LazyDB does not provide a general VPN, SOCKS UDP forwarding, remote shell, or
  arbitrary remote Unix socket.
- A tunnel is not kept alive for long-term idle reuse and is not shared across
  different profiles.

## Troubleshooting

| Error code | What to check |
| --- | --- |
| `ssh_unavailable` | Install OpenSSH and make `ssh` available on `PATH`. On Windows, check the system OpenSSH client. |
| `ssh_unsupported` | The OpenSSH client or driver/platform combination is not compatible. On Windows SQLx, use a supported TLS mode or external forwarding. |
| `ssh_configuration` | Check the SSH alias, paths, ProxyJump syntax, and that the config has no inherited forwards. |
| `ssh_host_key_unknown` | Verify the bastion fingerprint through a trusted channel and add it to `known_hosts`. |
| `ssh_host_key_changed` | Stop and investigate. Do not remove the old key until the change is verified. |
| `ssh_interaction_required` | Unlock the key with `ssh-add`, use an agent, or move interactive authentication outside LazyDB. |
| `ssh_authentication_failed` | Check the SSH user, key selection, agent contents, and bastion authorization. |
| `ssh_network` | Check SSH host, port, route, DNS, and jump-host connectivity. |
| `ssh_timeout` | Check slow authentication or multi-hop connectivity, then increase `connect_timeout_seconds` or `connections test --timeout`. |
| `ssh_forward_conflict` | Another process owns the loopback port or the bastion rejects the forward. Retry and inspect local listeners. |
| `ssh_destination_unreachable` | From the bastion, verify the database hostname, port, firewall, and forwarding permissions. |
| `ssh_tunnel_exited` | The tunnel stopped. Reconnect the profile before running more work. |
| `ssh_tunnel_unavailable` | The route was already invalidated. Reconnect the profile; do not reuse the old pool or transaction. |
| `connection_timeout` | The combined SSH and database attempt exceeded its deadline. |

Useful checks outside LazyDB:

```sh
ssh -G your-alias | grep -E '^(hostname|user|port|identityfile|proxyjump) '
ssh your-bastion-alias
ssh-add -l
```

From the bastion, verify the database endpoint with the tools available there,
for example `nc -vz orders-db.internal 5432`. Do not fix SSH failures by
disabling host-key verification or by connecting directly outside the intended
network path.

## External OpenSSH Forwarding

If the in-app authentication flow cannot be made non-interactive, keep using an
external tunnel. LazyDB then sees a normal database on loopback. Bind the local
listener to `127.0.0.1`, not `0.0.0.0`.

```sh
ssh -N \
  -o ExitOnForwardFailure=yes \
  -o ServerAliveInterval=30 \
  -o ServerAliveCountMax=3 \
  -L 127.0.0.1:15432:orders-db.internal:5432 \
  bastion.example.com
```

The same command works in PowerShell with the OpenSSH client installed. For
multiple databases, add one `-L` per database and one LazyDB profile per local
port. Add `-J jump.example.com` when needed:

```sh
ssh -J jump.example.com -N \
  -L 127.0.0.1:13306:analytics-db.internal:3306 \
  bastion.example.com
```

Create the LazyDB profile with **Host** set to `127.0.0.1` and **Port** set to
the local port, for example `15432`. Keep the database name and database user
unchanged:

```sh
lazydb connections add \
  --name orders-external-tunnel \
  --url 'postgresql://app_user@127.0.0.1:15432/orders' \
  --scope global \
  --password-env PGPASSWORD
lazydb connections test orders-external-tunnel
```

Use `127.0.0.1`, not `localhost`, because `localhost` may resolve to an IPv6
address while the forward listens on IPv4. For an IPv6 database host, bracket
the remote side of `-L`:

```sh
ssh -N -L 127.0.0.1:15432:[2001:db8::10]:5432 bastion.example.com
```

### TLS with an external forward

An external forward replaces the target with `127.0.0.1`, so strict hostname
verification is different from the managed SQLx Unix socket path:

| Driver | Practical external-forward TLS mode |
| --- | --- |
| PostgreSQL, CockroachDB | `require`, or `verify-ca` when the profile has the CA. `verify-full` checks the certificate against `127.0.0.1` and normally fails. |
| MySQL, MariaDB, TiDB | `require`. MySQL profiles do not expose a CA field, so do not rely on `verify-ca` for a private CA. |
| SQL Server | `require`. `prefer` and `verify-full` perform verified TLS in the native adapter and normally fail through loopback. |
| Redis, Valkey, MongoDB, ClickHouse, Elasticsearch, OpenSearch | This workaround generally requires plaintext or HTTP unless the local endpoint has a certificate valid for `127.0.0.1`. |
| Oracle | Test `disable` or `require`; TCPS hostname behavior depends on the native client configuration. |

`require` encrypts the database connection but does not authenticate the
database certificate. The SSH host key authenticates the bastion. If strict
certificate hostname verification is mandatory, use the managed tunnel where
the driver supports it, rather than weakening TLS for the external forward.

Keep the external process in a dedicated terminal, tmux session, or service
manager. LazyDB does not monitor or restart it. If the process exits, restart it
and reconnect the profile; reusing a released local port for another purpose
could route database traffic to the wrong process.

## Security Notes

- LazyDB listens only on loopback or a private Unix socket directory. OpenSSH
  also runs with gateway ports disabled.
- SSH forwarding is strict, non-interactive, and uses `IdentitiesOnly=yes` when
  an explicit key is configured. Agent forwarding and X11 forwarding are
  disabled.
- The profile stores SSH paths, not SSH passwords, passphrases, private keys, or
  agent identities.
- Database credentials remain in the existing credential policy and are
  separate from SSH configuration.
- SSH errors are classified without returning remote banners or credential
  material. Detailed logs are bounded and cleaned before display.
- Treat SSH config `ProxyCommand`, `Match exec`, and included files as trusted
  local code. A malicious or unsuitable config can run local commands or
  conflict with the managed forwarding flags.

For authentication, cleanup, and topology behavior across platforms, do not
assume that an unrecorded combination has been validated. The verification
status above is intentionally narrower than the implemented transport matrix.
