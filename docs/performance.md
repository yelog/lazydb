# Performance Baseline

This document records repeatable local workloads for architecture and rendering
changes. It is not a benchmark threshold: elapsed time varies by operating
system, terminal backend, Rust version, and build profile.

## Fixed Workloads

- Editor document: approximately 100 KB of deterministic SQL spread over 4,000 lines.
- Result grid: 500 rows by 20 text columns.
- Terminal backend: Ratatui `TestBackend`, 160 columns by 48 rows, ASCII icons.
- Full render loop: 20 complete application renders in one process.

The fixture contains no real database data and does not require a database
server, credentials, filesystem state, or network access.

## Commands

Run the deterministic safety test:

```bash
cargo test --test performance_regression performance_fixtures_are_deterministic_and_renderable
```

Record a local baseline or comparison in release mode:

```bash
cargo test --release --test performance_regression performance_baseline -- --ignored --nocapture --test-threads=1
```

Record the commit, toolchain, OS, CPU, and build profile beside any result.
Do not add an absolute elapsed-time assertion to normal CI. Later tasks should
add structural counters for cache entries, full-table scans, and persistence
writes; those counters are more stable regression checks than wall-clock time.

## Interpretation

## Implementation Checkpoint

The first implementation is partial, not completion of T01-T13. Editor gutter
lookup and cache cleanup, borrowed persistence projection, unchanged-file write
avoidance, visible-row access, and explicit row budgets are implemented.
Pointer-based column-width caching was removed during pre-commit review because
in-place edits cannot be identified reliably by addresses and row counts.
Revision-based grid caching and full editor analysis indexing remain pending.

Result budgets count retained cell JSON sizes, not the complete response or RSS.
Column metadata/result-set count limits, skipping decode after budget exhaustion,
manual transaction/page byte budgets and complete truncation UI coverage remain
follow-ups. The save coordinator and failure feedback are designed separately in
`plans/2026-09-05-workspace-save-state-machine.md`, not implemented.

The earlier release sample was 86.566583ms for 20 full renders, before pre-commit
corrections. There is no measured pre-change baseline, so this is not a speedup
claim. The 500 passing tests reported earlier were the lib test binary only.
The full suite failed in a PostgreSQL catalog test; its cause has not been proven
to be environmental and it must not be reported as a fully passing suite.

The baseline includes full UI composition and intentional current behavior. It
does not measure database network latency, server execution time, process RSS,
or terminal I/O. Database resource-budget tests must use synthetic adapter
events and isolated integration databases rather than this fixture.
