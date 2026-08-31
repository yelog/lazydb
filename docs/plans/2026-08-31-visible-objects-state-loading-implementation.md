# Visible Objects State And Loading Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace Visible Objects checkbox text with mode-aware semantic icons and show a non-destructive, interaction-safe loading state during catalog discovery.

**Architecture:** Extend `IconSet` with tri-state mappings, render separate styled spans, and reuse `scope_discovery_request` plus the shared animation/loading widgets. Suppress edits at the renderer, keymap, and model boundaries.

**Tech Stack:** Rust, Ratatui 0.30, `nerd-font-symbols`, Crossterm, existing profile discovery and UI animation modules.

---

### Task 1: Add Mode-Aware Scope Selection Icons

Modify `src/ui/icons.rs` to add `SelectionIcon` and `IconSet::selection`. Use `md::MD_CHECKBOX_BLANK_OUTLINE`, `md::MD_CHECKBOX_MARKED`, and `md::MD_CHECKBOX_INTERMEDIATE` for Nerd Font; `☐`, `☑`, `▣` for Unicode; and `[ ]`, `[x]`, `[-]` for ASCII. Extend existing safety tests and add exact mapping assertions. Run `cargo test ui::icons::tests --lib` and review the diff.

### Task 2: Track Scope Discovery In UI Animation

Modify `src/ui/animation.rs` and `src/ui/mod.rs` to add `LoadIdentity::ProfileScope { request_id }`, observe pending profile manager discovery, and expose elapsed time through a UI helper. Keep request identity and timers out of domain state. Run animation tests and review the diff.

### Task 3: Render Tri-State Rows And Preserved Loading Content

Modify `src/ui/profiles.rs` to pass `IconSet` into Scope rendering, place `ActivityIndicator` above rows, retain rows during discovery, show the empty waiting text, render icon/name spans with state colors, and suppress Scope row hit regions while loading. Update `tests/ui_render.rs` for new Nerd Font/Unicode/ASCII icons, loading text, preserved rows, colors, hints, and hit regions. Run focused UI tests and review the diff.

### Task 4: Block Scope Mutation And Duplicate Refresh While Loading

Modify `src/model/profile_manager.rs` so `toggle_scope_row` refuses pending discovery. Modify `src/input/keymap.rs` so Space and `r` map to no action while pending while navigation/back remain available. Update `tests/profile_draft.rs`, `tests/profile_reducer.rs`, and `tests/keymap.rs`; run each focused suite and review the diff.

### Task 5: Verify Recovery And Quality Gates

Add failure-recovery assertions if needed: loading clears, saved rows remain, warning is sanitized, and interaction is restored. Run focused icon, animation, profile, keymap, UI, and mouse tests, then `cargo fmt --check`, `cargo test`, and `cargo clippy --all-targets --all-features -- -D warnings`. Review status and diff without touching unrelated worktree changes.
