# lazydb.nvim Repository Extraction Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Extract the embedded Neovim integration into a tested, public `yelog/lazydb.nvim` repository and make it the single plugin source.

**Architecture:** Copy the current tested plugin snapshot into a standard standalone Neovim plugin root, add independent CI and repository metadata, then remove the embedded source only after the new remote is verified. Keep the integration boundary at CLI API 1 and do not add source mirroring.

**Tech Stack:** Lua, Neovim 0.10+, Git, GitHub Actions, GitHub CLI, LazyDB CLI API 1.

---

### Task 1: Create the Standalone Plugin Root

**Files:**
- Create: `/Users/yelog/workspace/vi/lazydb.nvim/lua/lazydb/init.lua`
- Create: `/Users/yelog/workspace/vi/lazydb.nvim/lua/lazydb/config.lua`
- Create: `/Users/yelog/workspace/vi/lazydb.nvim/lua/lazydb/health.lua`
- Create: `/Users/yelog/workspace/vi/lazydb.nvim/plugin/lazydb.lua`
- Create: `/Users/yelog/workspace/vi/lazydb.nvim/doc/lazydb.txt`
- Create: `/Users/yelog/workspace/vi/lazydb.nvim/tests/lazydb_spec.lua`
- Create: `/Users/yelog/workspace/vi/lazydb.nvim/tests/minimal_init.lua`

**Step 1:** Copy the current files from `lazydb.nvim/` without changing behavior.

**Step 2:** Update `tests/minimal_init.lua` so `plugin_root` is the standalone repository root.

**Step 3:** Run:

```bash
nvim --headless -u tests/minimal_init.lua \
  -c "lua require('lazydb_spec').run()" -c qa
```

Expected: eight tests pass.

### Task 2: Add Standalone Repository Metadata

**Files:**
- Create: `/Users/yelog/workspace/vi/lazydb.nvim/README.md`
- Create: `/Users/yelog/workspace/vi/lazydb.nvim/LICENSE-MIT`
- Create: `/Users/yelog/workspace/vi/lazydb.nvim/LICENSE-APACHE`
- Create: `/Users/yelog/workspace/vi/lazydb.nvim/.gitignore`

**Step 1:** Write a standalone README using `{ "yelog/lazydb.nvim" }` as the primary `lazy.nvim` example.

**Step 2:** Document CLI installation as an external prerequisite and CLI API 1 as the compatibility contract.

**Step 3:** Copy both licenses from the main repository.

**Step 4:** Add a minimal `.gitignore` for editor and OS files.

### Task 3: Add Standalone CI

**Files:**
- Create: `/Users/yelog/workspace/vi/lazydb.nvim/.github/workflows/ci.yml`

**Step 1:** Add a Neovim matrix for `v0.10.4` and `stable`.

**Step 2:** Pin checkout and Neovim setup actions to reviewed commit SHAs.

**Step 3:** Run YAML parsing locally and execute the plugin tests with the local Neovim.

### Task 4: Initialize and Publish the Standalone Repository

**Files:**
- Create: `/Users/yelog/workspace/vi/lazydb.nvim/.git/`

**Step 1:** Run `git init -b main` in the standalone directory.

**Step 2:** Inspect `git status`, `git diff --check`, and all staged files.

**Step 3:** Commit with `feat: publish standalone LazyDB Neovim integration`.

**Step 4:** Create public `yelog/lazydb.nvim` with `gh repo create` and push `main`.

**Step 5:** Verify repository metadata, default branch, files, and Actions registration.

### Task 5: Remove the Embedded Plugin From the Main Repository

**Files:**
- Delete: `lazydb.nvim/**`
- Modify: `README.md`
- Modify: `.github/workflows/ci.yml`
- Modify: `docs/architecture.md` only if it refers to embedded source paths

**Step 1:** Replace monorepo installation examples with standard remote plugin specifications.

**Step 2:** Remove the Neovim job from the main repository CI.

**Step 3:** Delete `lazydb.nvim/` only after the remote repository is verified.

**Step 4:** Search for stale `lazydb.nvim/` file-path references and update only source-location references; keep product references to `lazydb.nvim`.

### Task 6: Verify and Commit the Main Repository Migration

**Files:**
- Verify: `README.md`
- Verify: `.github/workflows/ci.yml`
- Verify: `docs/**`

**Step 1:** Run `cargo fmt --all -- --check`.

**Step 2:** Run `cargo clippy --all-targets --all-features -- -D warnings`.

**Step 3:** Run `cargo test --all-targets --all-features` or rely on the most recent green full CI if local execution exceeds the session timeout.

**Step 4:** Run `git diff --check` and inspect staged deletion boundaries.

**Step 5:** Commit with `refactor(neovim): move plugin to standalone repository` and push `main`.

**Step 6:** Confirm both repositories' CI workflows are registered and no embedded plugin files remain in `yelog/lazydb`.
