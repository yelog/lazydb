# SSH Tunnel Workaround

LazyDB does not manage SSH tunnels yet. If your database is only reachable
through a bastion host, you can use OpenSSH local port forwarding and point a
normal LazyDB profile at the forwarded loopback port.

```text
LazyDB -> 127.0.0.1:15432 -> ssh -> bastion -> db.internal:5432
```

The tunnel is an external process. LazyDB only sees a database listening on
`127.0.0.1`.

## Before You Start

- Install an OpenSSH client and confirm that `ssh -V` works.
- Confirm that the database host and port are reachable from the bastion.
- Connect to the bastion once and verify its host key before starting LazyDB.
  This avoids an interactive host-key prompt inside the terminal UI.
- Use an SSH agent or a key without a passphrase, or start the tunnel in a
  separate terminal where you can enter the passphrase.
- Choose an unused local port. The examples use `15432` for PostgreSQL and
  `13306` for MySQL-compatible databases.

## Start the Tunnel

Run the tunnel in a separate terminal and leave it running while LazyDB is
connected:

```sh
ssh -N \
  -o ExitOnForwardFailure=yes \
  -o ServerAliveInterval=30 \
  -o ServerAliveCountMax=3 \
  -L 127.0.0.1:15432:db.internal:5432 \
  bastion.example.com
```

On Windows, the same command works in PowerShell when the OpenSSH client is
installed:

```powershell
ssh -N -o ExitOnForwardFailure=yes -L 127.0.0.1:15432:db.internal:5432 alice@bastion.example.com
```

`-N` keeps the SSH session open without starting a remote shell.
`ExitOnForwardFailure=yes` makes the command fail instead of staying connected
without a working forward.

You can also put the forward in `~/.ssh/config`:

```sshconfig
Host lazydb-orders
  HostName bastion.example.com
  User alice
  LocalForward 127.0.0.1:15432 db.internal:5432
  ExitOnForwardFailure yes
  ServerAliveInterval 30
  ServerAliveCountMax 3
```

Then start it with `ssh -N lazydb-orders`. If your bastion needs another jump
host, add `ProxyJump` to the SSH config or use `-J`:

```sh
ssh -J jump.example.com -N \
  -L 127.0.0.1:15432:db.internal:5432 \
  bastion.example.com
```

## Create the LazyDB Profile

Create or edit a profile with the normal connection details, but replace the
database host and port with the local side of the forward:

| Field | Value |
| --- | --- |
| Host | `127.0.0.1` |
| Port | The local port, for example `15432` |
| Database / service | The same value you would use without the tunnel |
| User | The database user |
| Password | The normal database credential |

Use `127.0.0.1` rather than `localhost`. `localhost` may resolve to an IPv6
address while the SSH forward is listening on IPv4.

You can test the saved profile without opening the TUI:

```sh
lazydb connections test "orders-via-bastion"
```

The database sees the connection coming from the bastion, not from your
workstation. Make sure its host-based authentication rules allow that address.

## TLS Behavior

The tunnel changes the address LazyDB connects to, which affects certificate
hostname verification. The database certificate usually names `db.internal`,
not `127.0.0.1`.

| Driver | TLS mode that works through this workaround | Notes |
| --- | --- | --- |
| PostgreSQL / CockroachDB | `require`; `verify-ca` when `pg_options.ssl_root_cert` contains the CA | `verify-full` fails because the certificate hostname is checked against `127.0.0.1`. |
| MySQL / MariaDB / TiDB | `require` | `verify-full` fails. LazyDB profiles do not expose a MySQL CA file, so do not rely on `verify-ca` for a private CA. |
| SQL Server | `require` | `prefer` is treated as verified TLS by LazyDB and fails through the tunnel. `verify-full` also fails. |
| Redis / Valkey, MongoDB, ClickHouse, Elasticsearch / OpenSearch | Use plaintext or HTTP for this workaround | These adapters verify TLS hostnames by default. `rediss`, MongoDB TLS, ClickHouse HTTPS, and search HTTPS will normally fail against `127.0.0.1`. |
| Oracle | Test `disable` or `require` | TCPS hostname behavior depends on the native Oracle client and its configuration. |
| SQLite, DuckDB, BigQuery, Snowflake | Not applicable | These use local files or managed HTTPS endpoints and do not need this tunnel. |

`require` encrypts the database connection but does not authenticate the
database certificate. The SSH connection still authenticates the bastion and
fixes the destination host and port. For PostgreSQL, prefer `verify-ca` with a
configured CA when you need certificate validation in addition to the tunnel.

If strict certificate hostname verification is mandatory for your environment,
this workaround is not suitable. Built-in tunnel support needs a driver-level
TLS server-name override before it can preserve `verify-full` semantics.

## Common Setups

### Multiple Databases Through One Bastion

Use one local port per database:

```sh
ssh -N \
  -L 127.0.0.1:15432:orders-db.internal:5432 \
  -L 127.0.0.1:13306:analytics-db.internal:3306 \
  bastion.example.com
```

Create one LazyDB profile for each local port.

### Keep the Tunnel Running

LazyDB does not start or monitor the tunnel. Keep it in a dedicated terminal,
tmux session, or your operating system's service manager. `autossh` is optional
if you need automatic restart after a network interruption.

### IPv6 Database Host

Use brackets around the remote host in the forward specification:

```sh
ssh -N -L 127.0.0.1:15432:[2001:db8::10]:5432 bastion.example.com
```

## Troubleshooting

**`bind [127.0.0.1]:15432: Address already in use`**

Choose another local port. Check the port owner with
`lsof -nP -iTCP:15432 -sTCP:LISTEN` on macOS/Linux, or
`netstat -ano | findstr :15432` on Windows.

**`channel 2: open failed: connect failed`**

The bastion could not reach the database. Test from the bastion itself:

```sh
ssh bastion.example.com 'nc -vz db.internal 5432'
```

Check the database listen address, firewall rules, and the remote host and port
in the `-L` specification.

**`Permission denied (publickey)`**

Make sure the key is loaded in the agent with `ssh-add -l`. Test the same host
with a plain `ssh bastion.example.com` before starting the forward.

**Host key verification failed or a host-key prompt appears**

Connect to the bastion interactively once and compare the fingerprint through a
trusted channel. Do not disable host-key checking.

**The tunnel connects but LazyDB says connection refused**

Confirm that the local listener accepts connections:

```sh
nc -vz 127.0.0.1 15432
```

On PowerShell, use
`Test-NetConnection -ComputerName 127.0.0.1 -Port 15432`. If the tunnel is still
starting, wait a moment and test the profile again.

**The certificate hostname does not match**

This is expected with strict TLS modes. See [TLS behavior](#tls-behavior) and
choose a supported mode for the driver.

**Queries fail after the SSH process exits**

Restart the tunnel on the same local port and reconnect the LazyDB profile. If
you use a different local port, update the profile first.

## Security Notes

- Bind the forward to `127.0.0.1`, not `0.0.0.0`. A wildcard bind can expose
  the database to other machines on your network.
- Keep host-key verification enabled. Do not use
  `StrictHostKeyChecking=no` as a shortcut.
- Do not put SSH passwords on the command line. Use an agent or interactive
  authentication in the tunnel terminal.
- Traffic between LazyDB and the bastion is encrypted by SSH. Read the TLS
  section before deciding whether traffic from the bastion to the database is
  sufficiently protected for your environment.
- Close the tunnel when you no longer need it. Any local process that can reach
  the loopback port can use the forward while it is open.

## Current Limitations

- LazyDB does not manage the tunnel process, display its status, or reconnect it
  after a failure.
- The saved profile contains only the loopback host and port. It does not
  record which bastion or remote database the forward belongs to.
- `verify-full` certificate validation is not compatible with the loopback
  address used by this workaround.
- A tunnel started with a different local port requires a profile edit and a
  new connection.

For native tunnel support, follow the
[issue tracker](https://github.com/yelog/lazydb/issues).
