# LazyDB

A keyboard-first database workspace for your terminal, built in Rust.

[Quickstart](#quickstart) · [Databases](#supported-databases) ·
[Documentation](docs/README.md) · [Releases](https://github.com/yelog/lazydb/releases)

![LazyDB SQL editor with query execution and results](https://github.com/user-attachments/assets/a37348f0-bc37-49ae-b164-034730a5217e)

<details>
<summary>More screenshots: table data, monitoring, mouse support, MCP, and Neovim</summary>

<table>
  <tr>
    <th>Table Data</th>
    <th>Monitoring Dashboard</th>
  </tr>
  <tr>
    <td><img alt="Table data with filtering and ordering" src="https://github.com/user-attachments/assets/69a60235-8a2f-49cb-8c4c-25392f33797c" /></td>
    <td><img alt="Database monitoring dashboard" src="https://github.com/user-attachments/assets/439f3d1b-611d-42db-9bc9-039799f9e3d3" /></td>
  </tr>
  <tr>
    <th>Mouse Support</th>
    <th>MCP and Neovim</th>
  </tr>
  <tr>
    <td><img alt="Mouse interaction in LazyDB" src="https://github.com/user-attachments/assets/bf39b262-b7b1-4f21-b1cc-fe8a58be7928" /></td>
    <td><img alt="LazyDB MCP access alongside Neovim" src="https://github.com/user-attachments/assets/f4b03b84-0386-4090-8bc5-81178d29d360" /></td>
  </tr>
  <tr>
    <th colspan="2">Neovim SQL Completion and Syntax Highlighting</th>
  </tr>
  <tr>
    <td colspan="2"><img alt="Neovim SQL editing with LazyDB language-server completion" src="https://github.com/user-attachments/assets/63dee305-ec5a-4262-8366-2afdf52591d9" /></td>
  </tr>
</table>

</details>

- **Explore databases** — browse schemas, table data, Redis/Valkey keys, and
  MongoDB documents.
- **Write SQL** — use Vim-style editing, catalog-aware completion, and multiple
  consoles with query results and transaction controls where supported.
- **Work in your tools** — integrate with Neovim, coding agents, MCP, and a JSON CLI.

> LazyDB is in beta. Feature availability varies by database; see the
> [capability reference and current limitations](docs/database-capabilities.md).

## Quickstart

**macOS / Linux**

```sh
curl -fsSL https://lazydb.yelog.org/install.sh | sh
```

**Windows (PowerShell, x86_64)**

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -Command "irm https://lazydb.yelog.org/install.ps1.txt -ErrorAction Stop | iex"
```

**Homebrew (macOS)**

```sh
brew install yelog/tap/lazydb
```

Follow the installer's shell instructions or open a new terminal, then start
LazyDB:

```sh
lazydb
```

1. In Explorer, select `No profiles` and press `Enter` to create your first
   connection. Choose a driver, enter its connection details, then save and connect.
2. Open a table to inspect its data, or press `F6` to open the SQL console manager.
3. Press `F2` to search commands and objects. `F1` opens help; the footer and help
   view show contextual controls for the active pane and mode.

Use `Ctrl-c` to quit from the workspace.

SQL consoles also work offline: write, format, and save SQL before connecting.
See the [Omni Bar guide](docs/omni-bar.md) and
[keyboard reference](docs/keybindings.md) for navigation and execution controls.
If icons render incorrectly, start with `lazydb --icons unicode` or
`lazydb --icons ascii`.

For release channels, Linux requirements, manual downloads, source builds,
updates, and removal, see the [installation guide](docs/installation.md).

## Supported Databases

| Type | Databases |
| --- | --- |
| Server SQL | PostgreSQL, CockroachDB, MySQL, MariaDB, TiDB, Oracle, SQL Server, ClickHouse, Snowflake, Google BigQuery |
| Embedded SQL | SQLite, DuckDB |
| Key-value and document | Redis, Valkey, MongoDB |
| Search | Elasticsearch, OpenSearch |

Oracle requires Oracle Instant Client. DuckDB requires an optional C API runtime
that is not bundled; see [runtime setup](docs/duckdb.md#install-the-optional-runtime).
Redis and Valkey use a key/value browser; MongoDB uses a document explorer and
bounded JSON console. See the [MongoDB guide](docs/mongodb.md) for its scope.
Snowflake uses the HTTPS SQL API with a stored access token; see the
[Snowflake guide](docs/snowflake.md) for authentication and current limits.
Google BigQuery uses refreshable ADC and GoogleSQL jobs with per-profile billing
limits; see the [BigQuery guide](docs/bigquery.md).
Elasticsearch provides index and mapping discovery, document previews, and a native
REST/JSON console; see the [Elasticsearch guide](docs/elasticsearch.md).
OpenSearch has its own driver with Basic/None authentication and shares the REST
browser/console; see [OpenSearch setup and verification limits](docs/opensearch.md).

Browse the [database capability matrix](docs/database-capabilities.md) for server
versions, editing, transactions, monitoring, and driver-specific limits.

## Integrations

### Coding Agents and MCP

Give Claude Code, Codex, or OpenCode access to the current project's database
profiles through a local MCP server. From the target project, run:

```sh
lazydb mcp setup
```

The wizard offers user-level and project-level configuration. MCP database writes
are denied by default; database grants remain the authorization boundary.
See [coding-agent access](docs/coding-agent-access.md) for setup, permissions,
the JSON CLI, and troubleshooting.

### Neovim and SQL Language Server

Use [`yelog/lazydb.nvim`](https://github.com/yelog/lazydb.nvim) to run LazyDB in a
floating terminal. The standalone language server provides SQL/MyBatis syntax
diagnostics and completion, with catalog completion when a profile is available.
See [Neovim setup and the SQL language server](docs/sql-language-server.md).

## Documentation

| Task | Guide |
| --- | --- |
| Install, update, or uninstall | [Installation](docs/installation.md) |
| Configure connections, credentials, themes, or clipboard behavior | [Configuration](docs/configuration.md) |
| Learn navigation and keyboard controls | [Keyboard reference](docs/keybindings.md) |
| Check database support and limitations | [Database capabilities](docs/database-capabilities.md) |
| Find database guides, integrations, and development docs | [Documentation index](docs/README.md) |

## Contributing

Bug reports and focused pull requests are welcome. Read
[Contributing](CONTRIBUTING.md) for local checks and development rules, and use
the [issue tracker](https://github.com/yelog/lazydb/issues) to report problems.
See the [changelog](CHANGELOG.md) for version-specific changes.

## License

Licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
