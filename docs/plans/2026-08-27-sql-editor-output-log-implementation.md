# SQL Editor Output Log Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Focus the correct SQL result tab and append timestamped execution details to OUTPUT.

**Architecture:** Keep query measurement and execution in the database/runtime layers. In `App`, use the stored `ExecutionDraft` and `QueryOutcome.stats` to format append-only output entries and select `DATA` only for read-only query results.

**Tech Stack:** Rust 2024, ratatui, crossterm, sqlparser, cargo test.

---

### Task 1: Add completion behavior tests

**Files:**
- Modify: `tests/sql_execution.rs`

**Steps:**
1. Add tests for query and non-query `QueryFinished` actions with explicit execution drafts and deterministic stats.
2. Assert the selected result view, appended output entries, target/SQL text, and execution/fetch/total timing.
3. Add a repeated completion assertion to verify output history is appended.
4. Run the focused tests and confirm they fail before implementation.

### Task 2: Implement execution log formatting and tab selection

**Files:**
- Modify: `src/app.rs`

**Steps:**
1. Add local timestamp formatting using standard library time APIs without adding a dependency.
2. Add a helper that formats the target and exact executed SQL from `LastExecution`.
3. Update normal and manual completion paths to append the two-line execution record.
4. Select `DATA` only for read-only query drafts; select `OUTPUT` for non-query completion and failures.
5. Preserve existing error, cancellation, and derived-query behavior.

### Task 3: Verify

**Files:**
- Test: `tests/sql_execution.rs`, `tests/app_flow.rs`, `tests/ui_render.rs`

**Steps:**
1. Run formatting and focused tests.
2. Run all relevant integration tests.
3. Inspect diff and ensure unrelated worktree files are untouched.
