# LazyDB Documentation

New to LazyDB? Start with the [quickstart](../README.md#quickstart).
Choose a guide below for the task you want to complete.

## Install and Configure

| Task | Guide |
| --- | --- |
| Install, choose a release channel, update, or uninstall | [Installation](installation.md) |
| Check Linux runtime requirements | [Linux compatibility](linux-runtime-compatibility.md) |
| Manage connections, credentials, project scope, and settings | [Configuration](configuration.md) |
| Find the built-in setting values | [Default configuration](../config/default.toml) |
| Check database support and current limitations | [Database capabilities](database-capabilities.md) |

## Everyday Workflows

| Task | Guide |
| --- | --- |
| Find commands, connections, consoles, and tables | [Omni Bar](omni-bar.md) |
| Learn navigation, SQL execution, editing, and transaction controls | [Keyboard reference](keybindings.md) |
| Browse Redis and Valkey keys, values, and TTLs | [Key/value browser](redis-browser.md) |
| Inspect structured or serialized Redis-compatible values | [Value preview](redis-value-preview.md) |
| Configure Kitty pane navigation and resizing | [Kitty integration](kitty-integration.md) |

## Database Guides

PostgreSQL, MySQL, MariaDB, Oracle, SQL Server, SQLite, DuckDB, Redis, Valkey,
MongoDB, CockroachDB, TiDB, ClickHouse, and Snowflake are covered by the
[capability reference](database-capabilities.md). Additional guides explain
driver-specific setup and workflows:

| Database | Guides |
| --- | --- |
| DuckDB | [Runtime setup, files, in-memory sessions, and SQL support](duckdb.md) |
| MongoDB | [Document console, monitoring, principals, and transactions](mongodb.md) |
| Elasticsearch | [Connections, indices, mappings, and REST queries](elasticsearch.md) |
| OpenSearch | [Basic/None authentication, HTTPS, indices, and REST queries](opensearch.md) |
| Redis / Valkey | [Browser](redis-browser.md) · [Value formats](redis-value-preview.md) |
| Oracle | [Client setup](configuration.md#connection-profile-fields) · [Table editor](oracle-table-editor.md) · [Catalog drop behavior](oracle-catalog-drop.md) |
| ClickHouse | [Supported operations](database-capabilities.md#clickhouse) · [Connection setup](configuration.md#clickhouse-profiles) · [Local test server](clickhouse-test-database.md) |
| MariaDB | [Local test server](mariadb-test-database.md) |
| CockroachDB | [Connections, supported operations, and local tests](cockroachdb.md) |
| TiDB | [Connection setup, supported workflows, and local test server](tidb.md) |
| Snowflake | [SQL API authentication, supported workflows, and verification](snowflake.md) |

## Integrations

| Task | Guide |
| --- | --- |
| Configure Claude Code, Codex, or OpenCode through MCP or use the JSON CLI | [Coding-agent database access](coding-agent-access.md) |
| Run LazyDB inside Neovim | [Neovim setup](sql-language-server.md#neovim-integration) · [lazydb.nvim](https://github.com/yelog/lazydb.nvim) |
| Use SQL/MyBatis completion and syntax diagnostics | [SQL language server](sql-language-server.md) |

## Contribute and Maintain

| Topic | Guide |
| --- | --- |
| Development setup, checks, and contribution rules | [Contributing](../CONTRIBUTING.md) |
| Module boundaries and runtime design | [Architecture](architecture.md) |
| UI conventions | [Dialog guidelines](ui-dialog-guidelines.md) |
| Performance investigations | [Performance](performance.md) |
| CI toolchain, build profiles, and caches | [CI](ci.md) |
| Release process and distribution | [Releasing](releasing.md) |
| Version history | [Changelog](../CHANGELOG.md) |

Design and implementation records live in [plans](plans/). For current product
behavior, use the guides above.
