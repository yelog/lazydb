# Contributing

LazyDB is in an early architecture-sensitive phase. Small, tested changes are
preferred over broad abstractions or controls for unfinished features.

## Local Checks

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

The Neovim frontend and its test suite are maintained in the standalone
[`yelog/lazydb.nvim`](https://github.com/yelog/lazydb.nvim) repository.

PostgreSQL and MySQL integration tests run only when configured:

```bash
LAZYDB_TEST_POSTGRES_URL='postgresql://user:password@localhost:5432/database' \
  cargo test --test postgres_adapter

LAZYDB_TEST_MYSQL_URL='mysql://user:password@localhost:3306/database' \
  cargo test --test mysql_adapter
```

Never paste real credentials into issues, tests, command examples, snapshots, or
logs.

## Design Rules

- Keep SQLx types inside database adapter modules.
- Keep side effects out of `App::update`; return `Command` values instead.
- Every asynchronous result carries owner identity and generation.
- Never return an uncertain connection to a pool.
- Never enable result editing without stable row identity and affected-row checks.
- Do not expose UI actions before they have an operational implementation.
- Database values are hostile terminal input until sanitized.

See [docs/architecture.md](docs/architecture.md) and the approved
[product design](docs/plans/2026-08-24-lazydb-design.md) before changing module
boundaries.
