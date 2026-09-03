# Unify LazyDB Config and Install State Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Store native installation metadata and application state under the same configurable LazyDB directory, defaulting to `~/.config/lazydb` on macOS and Linux.

**Architecture:** Make `LAZYDB_CONFIG_HOME` (or `~/.config/lazydb`) the single application root for configuration, workspace state, `install.json`, `current`, and `releases`. The shell installer and Rust updater will use the same path contract, while preserving migration of legacy macOS application data and existing `XDG_DATA_HOME` installations.

**Tech Stack:** POSIX shell installer, Rust, `directories`, serde JSON, Rust unit tests, shell installer tests.

---

### Task 1: Define and test the unified path contract

**Files:**
- Modify: `src/persistence/paths.rs`
- Test: `src/persistence/paths.rs`

1. Add a shared application-root rule for macOS/Linux: `LAZYDB_CONFIG_HOME` when set, otherwise `$HOME/.config/lazydb`.
2. Make `config_dir`, `data_dir`, and `state_dir` all resolve to that root.
3. Preserve migration of legacy platform directories, including legacy native install state and release data where safe.
4. Add tests asserting all three paths are identical and environment override behavior.
5. Run `cargo test persistence::paths` and confirm it passes.

### Task 2: Make the shell installer use the unified path

**Files:**
- Modify: `pages/install-core.sh`
- Test: `scripts/release/test-installer.sh`

1. Replace the installer’s `XDG_DATA_HOME`-based `DATA_HOME` with `LAZYDB_CONFIG_HOME`, defaulting to `$HOME/.config/lazydb`.
2. Keep `LAZYDB_INSTALL_DIR` independent so the executable can still be installed elsewhere.
3. Update installer assertions to verify `install.json`, `current`, and `releases` are created under the configured application root.
4. Run the installer test suite with temporary HOME and confirm stable/Beta channel cases pass.

### Task 3: Update documentation and migration notes

**Files:**
- Modify: `README.md`
- Modify: `docs/configuration.md`
- Modify: `docs/releasing.md`

1. Document `~/.config/lazydb` as the default root for application state and native installation data.
2. Document `LAZYDB_CONFIG_HOME` as the override for the complete root.
3. Remove stale references claiming native installation data defaults to `~/.local/share/lazydb`.
4. Explain that existing installations are migrated or can be reinstalled using the same `LAZYDB_CONFIG_HOME` value.

### Task 4: Verify the complete update flow

**Files:**
- No additional source files expected.

1. Run `cargo fmt --check`.
2. Run `cargo test`.
3. Run the repository’s installer and release metadata tests.
4. Use a temporary home/config root to install or simulate Beta state and verify `lazydb update --check --json` reports `manager: native` and `channel: beta`.
5. Inspect `git diff` and confirm unrelated worktree changes remain untouched.
