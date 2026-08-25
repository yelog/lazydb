# SQL Editor, Vim, and Transactions Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Build a Unicode-safe Vim SQL editor with highlighting, formatting, semantic completion, risk-confirmed statement execution, and per-console AUTO/MANUAL transactions on pinned PostgreSQL, MySQL, and SQLite connections.

**Architecture:** Keep `App::update` as the only application-state mutation path. A private App-owned `EditorWorkspace` wraps modalkit and exposes LazyDB domain snapshots/effects; pure `sql` modules derive scopes, risk, formatting, highlights, completion, and immutable execution drafts; Runtime owns identity-checked query tasks and one serial pinned transaction worker per active MANUAL console, while each adapter drives SQLx's concrete `TransactionManager` over that worker-owned physical connection.

**Tech Stack:** Rust 1.94, Ratatui 0.30.2, Crossterm 0.29, modalkit 0.0.25, sqlparser 0.62, sqlformat 0.5, regex 1, SQLx 0.9, Tokio.

---

## Before Starting

Execute this plan in a dedicated worktree created from a clean, current `main`.
The dynamic Profile Manager plan is currently changing the same reducer and
runtime files. Complete and merge its identity-safety task before Task 4 here.

The prerequisite shape is:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConnectionIdentity {
    pub profile_id: Uuid,
    pub generation: u64,
}
```

`RunQuery`, `PreviewTable`, and `LoadDdl` must already carry that identity, and
Runtime must reject a command whose identity is no longer active. Reuse this
type; do not create a second SQL-specific connection identity.

Run before editing:

```bash
git status --short
git log --oneline -10
cargo test --all-targets
```

Expected: a clean worktree and all baseline tests passing. If Profile Manager
work is still uncommitted, stop and finish or isolate it before this plan.

For every code task below, run `cargo fmt --all` before the final focused test
and commit. Inspect `git status --short` and stage only that task's intended
files. Do not defer source formatting to Task 20.

## Phase A: Vim Editor Foundation

### Task 1: Add the Editor Dependency and Exact-Text Codec

**Files:**
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Modify: `src/lib.rs`
- Create: `src/editor/mod.rs`
- Create: `src/editor/tests.rs`

**Step 1: Write failing exact-text tests**

Create tests for round-tripping empty text, text without a final newline, one and
two final newlines, CJK, emoji, combining characters, and tabs. The adapter must
preserve the domain string exactly even though modalkit stores a terminal newline.

```rust
#[test]
fn text_codec_preserves_trailing_newlines() {
    for original in ["", "select 1", "select 1\n", "select 1\n\n", "数据🙂e\u{301}"] {
        let encoded = encode_editor_text(original);
        assert_eq!(decode_editor_text(&encoded).unwrap(), original);
    }
}
```

**Step 2: Run the focused test and verify failure**

Run: `cargo test --lib editor::tests::text_codec_preserves_trailing_newlines`

Expected: FAIL because `src/editor` and the codec do not exist.

**Step 3: Add dependencies and the codec**

Use exact dependency versions and no Ratatui adapter:

```toml
modalkit = { version = "=0.0.25", features = ["clipboard"] }
regex = "1"
```

Remove `ratatui-textarea`. Export `pub(crate) mod editor;` from `src/lib.rs`.
Implement a private sentinel newline codec:

```rust
fn encode_editor_text(text: &str) -> String {
    let mut encoded = String::with_capacity(text.len() + 1);
    encoded.push_str(text);
    encoded.push('\n');
    encoded
}

fn decode_editor_text(text: &str) -> Result<String, EditorError> {
    text.strip_suffix('\n')
        .map(str::to_owned)
        .ok_or(EditorError::MissingSentinel)
}
```

All programmatic loads must encode once; all domain reads must strip exactly one
sentinel. Never call `trim_end`.

**Step 4: Run the focused tests**

Run: `cargo test --lib editor::tests::text_codec -- --nocapture`

Expected: PASS.

**Step 5: Run dependency checks**

Run: `cargo check --all-targets --all-features`

Expected: PASS on Rust 1.94 without adding `modalkit-ratatui`.

**Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock src/lib.rs src/editor
git commit -m "feat(editor): add modal editor foundation"
```

### Task 2: Build a Unicode-Safe Editor Session Adapter

**Files:**
- Modify: `src/editor/mod.rs`
- Modify: `src/editor/tests.rs`
- Rewrite: `src/model/editor.rs`

**Step 1: Write failing session tests**

Cover initial Insert mode, `Esc` to Normal, `i` back to Insert, CJK/emoji input,
cursor positions in character coordinates, multiline paste, `Ctrl-W`, `Ctrl-U`,
`Ctrl-H`, one-step undo for an Insert session, shared unnamed/named registers,
and `+`/`*` mapping to modalkit's system-selection registers.

```rust
#[test]
fn insert_control_keys_keep_vim_semantics() {
    let mut workspace = fixture("alpha beta");
    workspace.move_cursor_to_end(CONSOLE).unwrap();
    press_ctrl(&mut workspace, CONSOLE, 'w');
    assert_eq!(workspace.text(CONSOLE).unwrap(), "alpha ");
    press_ctrl(&mut workspace, CONSOLE, 'u');
    assert_eq!(workspace.text(CONSOLE).unwrap(), "");
}
```

**Step 2: Run tests and verify failure**

Run: `cargo test --lib editor::tests -- --nocapture`

Expected: FAIL because the workspace/session adapter is absent.

**Step 3: Define LazyDB-owned editor DTOs**

Replace the old mutable `EditorBuffer` model with domain projections:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EditorMode {
    Normal,
    Insert,
    Replace,
    VisualChar,
    VisualLine,
    VisualBlock,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EditorPosition {
    pub line: usize,
    pub column: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EditorViewport {
    pub width: usize,
    pub height: usize,
}
```

Add LazyDB-owned selection and render types later; no modalkit type may appear in
`model`, `Action`, or UI signatures.

**Step 4: Implement the minimal session wrapper**

Create private `ApplicationInfo`, `ContentId`, and `ApplicationAction` types.
Use a UUID-containing newtype because Rust orphan rules prevent implementing
modalkit identifier traits directly for `Uuid`.

```rust
struct EditorSession {
    buffer: SharedBuffer<LazyDbEditorInfo>,
    group_id: CursorGroupId,
    viewport: ViewportContext<Cursor>,
    revision: u64,
}

pub(crate) struct EditorWorkspace {
    store: Store<LazyDbEditorInfo>,
    keys: KeyManager<TerminalKey, modalkit::actions::Action<LazyDbEditorInfo>, RepeatType>,
    commands: VimCommandMachine<LazyDbEditorInfo>,
    sessions: HashMap<Uuid, EditorSession>,
}
```

Follow only the state-management pattern of modalkit-ratatui's `TextBoxState`:
create a cursor group, hold `ViewportContext`, and delegate `Editable`,
`Searchable`, `Scrollable`, and `Jumpable` calls to `SharedBuffer`. Do not copy or
depend on its Ratatui 0.29 widget.

Initialize the first LazyDB console in Insert mode to preserve existing behavior.
Convert lock poisoning and exposed modalkit failures to `EditorError`; do not
`unwrap` in production paths. Modalkit 0.0.25 intentionally degrades unavailable
or failed OS clipboard reads/writes to an empty/best-effort operation, so do not
claim that those failures are observable. Keep deterministic tests on register
selection and cross-console named registers; make any real clipboard smoke test
environment-gated.

**Step 5: Implement atomic paste and revision tracking**

Paste the full string through one insert/transcribe action rather than one key per
character. Increment revision once for each logical edit and not for cursor-only
movement. Reset cursor, selection, viewport, and history on programmatic content
replacement.

**Step 6: Run focused tests**

Run: `cargo test --lib editor::tests -- --nocapture`

Expected: all text, mode, Unicode, control-key, paste, and revision tests PASS.

**Step 7: Commit**

```bash
git add src/editor src/model/editor.rs
git commit -m "feat(editor): wrap modalkit sessions"
```

### Task 3: Lock the Practical Vim Contract and Application Bindings

**Files:**
- Modify: `src/editor/mod.rs`
- Modify: `src/editor/tests.rs`

**Step 1: Add failing table-driven Vim tests**

Cover representative contracts for every approved class:

```text
3w, 2b, ge, 0, ^, $, gg, G, f;, t), %, {, }, Ctrl-D, Ctrl-U
dw, d2w, dd, cw, ciw, ci", da(, y$, yy, p, P, >j, <j, ~
v, V, Ctrl-V with yank/delete/change/insert
u, Ctrl-R, dot repeat
unnamed and named registers across two console UUIDs
```

Assert text, mode, cursor, selection shape, register contents, and history. Include
Unicode operands in motion/operator tests.

**Step 2: Run and verify contract gaps**

Run: `cargo test --lib editor::tests::vim_ -- --nocapture`

Expected: one or more failures until action draining and mappings are complete.

**Step 3: Drain the complete modalkit action queue**

Handle editor, repeat, macro, jump, scroll, search, command-bar, prompt, command,
window/tab, application, and informational actions. Unknown future variants must
become a safe status error, never a panic.

Configure the concrete Vim machine before wrapping it in `KeyManager`. Register
LazyDB application effects for:

```rust
enum EditorEffect {
    Changed { console_id: Uuid, revision: u64 },
    RunCurrent,
    RunAll,
    FormatCurrent,
    NewConsole,
    CloseConsole,
    FocusPane(Focus),
    NextTab,
    PreviousTab,
    ShowHelp,
    ToggleTransaction,
    SetTransactionModeRequested { manual: bool },
    Commit,
    Rollback,
    ClearTransactionOutcome,
    Quit,
    Message(String),
}
```

Map Normal-mode `<leader>r`, `<leader>R`, `<leader>f`, `<leader>n`,
`<leader>tt/tc/tr`, `<leader>?`, `Ctrl-W h/j/k/l`, `[t`, `]t`, and `Q`.
Do not override Insert/Replace `Q`, `?`, Tab, `Ctrl-W`, `Ctrl-U`, or `Ctrl-H`.
Drain search/command-bar variants into private prompt intents without panicking,
but leave interactive `/`, `?`, `n`, and `N` behavior to Task 6 so its tests are
not falsely green here. Normal `Q` emits `EditorEffect::Quit`, not CloseConsole.

**Step 4: Run the complete editor contract**

Run: `cargo test --lib editor::tests -- --nocapture`

Expected: PASS.

**Step 5: Commit**

```bash
git add src/editor
git commit -m "feat(editor): define practical Vim bindings"
```

### Task 4: Move Editor Ownership into App

**Prerequisite:** The Dynamic Profile Manager identity-safety task is committed,
merged into this worktree, and the baseline is clean. Re-read every file listed
below because that work changes the same reducer/runtime boundary.

**Files:**
- Modify: `src/model/tab.rs`
- Modify: `src/action.rs`
- Modify: `src/app.rs`
- Modify: `src/input/keymap.rs`
- Modify: `src/input/mouse.rs`
- Modify: `src/runtime.rs`
- Modify: `tests/app_flow.rs`
- Modify: `tests/keymap.rs`
- Modify: `tests/mouse.rs`

**Step 1: Write failing lifecycle and routing tests**

Test new-console session creation, tab switching, close cleanup, preview/DDL tab
creation, late DDL replacement by UUID, atomic paste, and mode-sensitive input.
Explicit key expectations:

```text
Insert Q and ? insert characters.
Normal Q requests Quit.
Normal ? starts backward search.
F1 and Normal <leader>? show help.
Insert Ctrl-W/U/H go to the editor.
Normal Ctrl-W h/j/k/l changes pane focus.
Running Ctrl-C cancels; idle Insert Ctrl-C returns to Normal.
```

**Step 2: Run and verify failure**

Run: `cargo test --test keymap --test app_flow --test mouse -- --nocapture`

Expected: FAIL against granular editor actions and `ConsoleTab::editor`.

**Step 3: Change the reducer boundary**

Replace granular key actions with:

```rust
EditorKey(KeyEvent),
EditorPaste(String),
EditorViewportChanged(EditorViewport),
EditorScroll { rows: isize, columns: isize },
```

Keep `ReplaceEditor(String)` temporarily for fixtures and programmatic preview/DDL
loads, but route it through `EditorWorkspace`.

Add private `editor: EditorWorkspace` to `App`. Remove `App: Clone` if modalkit
prevents it; implement a concise manual `Debug` only if tests or diagnostics need
it. Remove `ConsoleTab::editor`; `ConsoleTab::id` is the session key. Do not clone
a tab UUID into a second logical editor session.

Add App helpers:

```rust
pub fn active_editor_text(&self) -> Result<String, EditorError>;
pub fn editor_text(&self, tab_id: Uuid) -> Result<String, EditorError>;
pub fn active_editor_revision(&self) -> u64;
pub fn active_editor_mode(&self) -> EditorMode;
```

**Step 4: Route terminal events atomically**

For editor focus, top-level `Keymap` handles blocking overlays, query cancellation,
and F-keys, then returns `EditorKey`. Change `Event::Paste` to one `EditorPaste`
action. Mouse wheel over the editor becomes viewport scroll, not cursor movement.

**Step 5: Run focused tests**

Run: `cargo test --test keymap --test app_flow --test mouse -- --nocapture`

Expected: PASS.

**Step 6: Run the complete baseline**

Run: `cargo test --all-targets`

Expected: PASS with no direct `ConsoleTab::editor` accesses left.

**Step 7: Commit**

```bash
git add src/action.rs src/app.rs src/editor src/input src/model src/runtime.rs tests/app_flow.rs tests/keymap.rs tests/mouse.rs
git commit -m "refactor(editor): centralize console editor state"
```

### Task 5: Render Immutable Editor Snapshots and Viewports

**Files:**
- Modify: `src/model/editor.rs`
- Modify: `src/editor/mod.rs`
- Modify: `src/app.rs`
- Modify: `src/runtime.rs`
- Modify: `src/security.rs`
- Modify: `src/ui/mod.rs`
- Modify: `src/ui/layout.rs`
- Modify: `tests/ui_render.rs`
- Modify: `tests/mouse.rs`

**Step 1: Write failing render tests**

Use 120x36, 80x24, and the minimum supported size. Assert visible mode, line and
column, selection styles, CJK/emoji cursor cells, horizontal/vertical scrolling,
page motion after resize, and no cloning/rendering of off-screen 10,000-line text.
Paste hostile ESC/OSC/CSI and other C0 controls and prove snapshots and terminal
buffers contain inert placeholders rather than raw control bytes.

**Step 2: Run and verify failure**

Run: `cargo test --test ui_render --test mouse -- --nocapture`

Expected: FAIL because UI still reads the old buffer directly.

**Step 3: Define the render projection**

```rust
pub struct EditorRenderSnapshot {
    pub revision: u64,
    pub mode: EditorMode,
    pub first_line: usize,
    pub lines: Vec<EditorRenderLine>,
    pub cursor: EditorPosition,
    pub cursor_screen_cell: Option<(u16, u16)>,
    pub selections: Vec<EditorSelection>,
    pub prompt: Option<EditorPromptSnapshot>,
}
```

Only include visible lines plus a small overscan. Convert character offsets to
terminal cells with `unicode-width`; never add a character index directly to the
terminal x coordinate. Keep the editor's source text byte-exact, but derive each
`EditorRenderLine` through a security helper that expands tabs, renders ESC/CR/C0
as visible placeholders, and records source-character-boundary to display-cell
mapping. Cursor, selection, and highlight placement must use that mapping rather
than the unsanitized string width.

**Step 4: Feed viewport metrics back through actions**

Let `UiState` record the editor text rectangle during render. After drawing,
compare it to the last metrics and dispatch `EditorViewportChanged` through the
normal reducer path. Width excludes borders and gutters. Editor page motions use
the last reducer-owned dimensions.

**Step 5: Replace the old editor renderer**

Render only sanitized snapshot spans, selection shape, cursor, line numbers,
mode, and command status using LazyDB's current theme. Rendering remains
read-only and performs no SQL parse or database access. Reuse the same inert
line projection later for execution previews; never sanitize the SQL snapshot
that is sent to the database.

**Step 6: Run focused and full UI tests**

Run: `cargo test --test ui_render --test mouse -- --nocapture`

Expected: PASS.

**Step 7: Commit**

```bash
git add src/app.rs src/editor src/model/editor.rs src/runtime.rs src/security.rs src/ui tests/ui_render.rs tests/mouse.rs
git commit -m "feat(editor): render modal editor snapshots"
```

### Task 6: Implement Search, Command-Line, and LazyDB Ex Actions

**Files:**
- Create: `src/editor/prompt.rs`
- Modify: `src/editor/mod.rs`
- Modify: `src/editor/tests.rs`
- Modify: `src/action.rs`
- Modify: `src/app.rs`
- Modify: `src/ui/mod.rs`

**Step 1: Write failing prompt tests**

Test `/`, `?`, `n`, `N`, editing/aborting/submitting a prompt, command history,
and these commands:

```text
:run
:runall
:format
:tx auto
:tx manual
:tx clear
:commit
:rollback
:q
```

Each Ex command must emit the same `EditorEffect` as its shortcut.
`:tx clear` only requests the later verified OutcomeUnknown recovery flow; it
never changes transaction state directly inside the editor adapter.

**Step 2: Run and verify failure**

Run: `cargo test --lib editor::tests -- --nocapture`

Expected: FAIL because command/search actions are not executed.

**Step 3: Add a private prompt session**

Implement the minimum command/search buffer state needed by modalkit's
`CommandBarAction`. Keep it independent of Ratatui. Register custom `VimCommand`
handlers that return explicit application variants such as
`Action::Application(LazyDbEditorAction::RunCurrent)` and
`Action::Application(LazyDbEditorAction::SetTransactionMode(mode))`. Validate
arguments before producing an effect.

**Step 4: Render prompt state**

Expose prompt kind, prefix, text, cursor, and error through
`EditorPromptSnapshot`. When present, it replaces normal editor status text.
Editor Normal `?` starts reverse search; F1 and `<leader>?` remain help.
Keep raw prompt text for command/search semantics, but use Task 5's inert display
projection for prompt text and errors. Add hostile pasted-control render tests so
the command line cannot emit terminal control sequences.

**Step 5: Run tests**

Run: `cargo test --lib editor::tests -- --nocapture`

Expected: PASS.

**Step 6: Commit**

```bash
git add src/action.rs src/app.rs src/editor src/ui/mod.rs
git commit -m "feat(editor): add search and Ex commands"
```

### Task 7: Add Undoable Substitute Commands

**Files:**
- Create: `src/editor/substitute.rs`
- Modify: `src/editor/mod.rs`
- Modify: `src/editor/tests.rs`
- Modify: `src/model/workspace.rs`
- Modify: `src/action.rs`
- Modify: `src/app.rs`
- Modify: `src/input/keymap.rs`
- Modify: `src/ui/mod.rs`

**Step 1: Write failing substitution tests**

Cover current line, `%`, Visual and numeric ranges, custom/escaped delimiters,
`g/i/I/c`, `&`, capture groups, previous-pattern reuse, no match, invalid regex,
cancel, and one-step undo.

**Step 2: Run and verify failure**

Run: `cargo test --lib editor::tests::substitute_ -- --nocapture`

Expected: FAIL because modalkit's built-in substitute handler returns an
unimplemented error.

**Step 3: Implement a pure substitution plan**

```rust
struct SubstitutionSpec {
    range: LineRange,
    pattern: String,
    replacement: String,
    global: bool,
    case: CaseOverride,
    confirm: bool,
}

struct SubstitutionMatch {
    range: Range<usize>,
    replacement: String,
}
```

Reuse modalkit's Ex range parser. Parse Vim command syntax but document Rust regex
matching. Calculate all byte ranges before editing. Apply accepted replacements
from highest offset to lowest between one pre-edit and one post-edit history
checkpoint; increment document revision once.

**Step 4: Implement `c` confirmation**

Add `Overlay::SubstituteConfirm` and reducer actions for yes, no, all, last, and
quit. Store the immutable plan and next match; never rerun the regex against text
that has already changed.

**Step 5: Run tests**

Run: `cargo test --lib editor::tests::substitute_ -- --nocapture`

Expected: PASS.

**Step 6: Commit**

```bash
git add src/action.rs src/app.rs src/editor src/input/keymap.rs src/model/workspace.rs src/ui/mod.rs
git commit -m "feat(editor): implement Vim substitution"
```

## Phase B: SQL Language Services and Safe Execution

### Task 8: Build the Dialect-Aware Statement Scope Scanner

**Files:**
- Modify: `src/lib.rs`
- Create: `src/sql/mod.rs`
- Create: `src/sql/dialect.rs`
- Create: `src/sql/range.rs`
- Create: `src/sql/scope.rs`
- Create: `tests/sql_scope.rs`

**Step 1: Write failing scope tests**

Test selection precedence, whitespace selection with no fallback, exact UTF-8 byte
ranges, Visual Char/Line contiguous selections, Visual Block row ranges and exact
newline-joined SQL, cursor-on-semicolon, blank gaps, comment-only segments,
unterminated constructs, and semicolons inside:

```text
'strings', "identifiers", `backticks`, [brackets]
-- and # line comments
/* nested block comments */
$$ PostgreSQL bodies $$ and $tag$ bodies $tag$
```

**Step 2: Run and verify failure**

Run: `cargo test --test sql_scope -- --nocapture`

Expected: FAIL because `lazydb::sql` does not exist.

**Step 3: Define pure range and scope types**

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TextRange {
    pub start: usize,
    pub end: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScopeKind {
    VisualChar,
    VisualLine,
    VisualBlock,
    CurrentStatement,
    FullBuffer,
}

pub enum ScopeSource {
    Contiguous(TextRange),
    Block(Vec<TextRange>),
}

pub struct ResolvedScope {
    pub kind: ScopeKind,
    pub source: ScopeSource,
    pub sql: String,
}
```

Use UTF-8 byte offsets at the SQL/editor boundary and convert explicitly from
modalkit character positions. For a block selection, preserve one exact range per
selected row and materialize only those slices joined with `\n`; never use the
bounding rectangle as executable SQL. Empty/whitespace Visual selections return
an explicit no-scope result and never fall back to the current statement.

**Step 4: Implement the scanner state machine**

Use explicit Normal, quote, line-comment, nested block-comment, and dollar-quote
states. Apply dialect-specific MySQL `--` whitespace and `#` behavior. Generic
mode recognizes the conservative union. A semicolon is a boundary only in
Normal. Do not implement custom MySQL `DELIMITER` in this task.

**Step 5: Run tests**

Run: `cargo test --test sql_scope -- --nocapture`

Expected: PASS.

**Step 6: Commit**

```bash
git add src/lib.rs src/sql tests/sql_scope.rs
git commit -m "feat(sql): resolve statement scopes"
```

### Task 9: Add Conservative SQL Parsing and Risk Classification

**Files:**
- Create: `src/sql/risk.rs`
- Modify: `src/sql/mod.rs`
- Create: `tests/sql_risk.rs`

**Step 1: Write failing risk tests**

Cover SELECT/VALUES, read-only CTEs, data-modifying CTEs, INSERT/UPDATE/DELETE/MERGE,
SELECT INTO, locks, EXPLAIN wrappers, CREATE/ALTER/DROP/TRUNCATE, BEGIN/COMMIT/
ROLLBACK/SAVEPOINT, CALL, dynamic execution, invalid SQL, and multi-statement
aggregate behavior.

**Step 2: Run and verify failure**

Run: `cargo test --test sql_risk -- --nocapture`

Expected: FAIL.

**Step 3: Implement conservative classification**

```rust
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum SqlRisk {
    ReadOnly,
    Dml,
    Ddl,
    TransactionControl,
    Unknown,
}
```

Parse with the exact connected dialect. Recursively inspect Query, CTE, SetExpr,
and EXPLAIN children. A query is read-only only when every nested body is proven
read-only and has no SELECT INTO or locking clause. Unrecognized Statement
variants and every parser failure are Unknown. Preserve per-statement risks and
statement count; do not infer multi-statement confirmation from only the maximum
enum value.

**Step 4: Run tests**

Run: `cargo test --test sql_risk -- --nocapture`

Expected: PASS.

**Step 5: Commit**

```bash
git add src/sql tests/sql_risk.rs
git commit -m "feat(sql): classify execution risk"
```

### Task 10: Add Formatting and Highlight Analysis

**Files:**
- Create: `src/sql/analysis.rs`
- Create: `src/sql/highlight.rs`
- Create: `src/sql/format.rs`
- Modify: `src/sql/mod.rs`
- Modify: `src/model/editor.rs`
- Modify: `src/editor/mod.rs`
- Modify: `src/app.rs`
- Modify: `src/ui/mod.rs`
- Create: `tests/sql_format.rs`
- Create: `tests/sql_highlight.rs`
- Modify: `tests/ui_render.rs`

**Step 1: Write failing formatter/highlighter tests**

Formatting tests must prove only the selected scope changes, outside text remains
byte-identical, keywords uppercase, cursor remains at or near the replaced range
start, and one unsupported dollar-quoted procedure is rejected without editing.
Reject Visual Block formatting with an actionable message because formatter
output cannot be mapped safely back to discontiguous rows. Include PostgreSQL
arrays, MySQL backticks, SQLite brackets, incomplete SQL, strings, comments,
parameters, and Unicode identifiers.

**Step 2: Run and verify failure**

Run: `cargo test --test sql_format --test sql_highlight -- --nocapture`

Expected: FAIL.

**Step 3: Implement token-location conversion**

Create a `LineIndex` that converts sqlparser's 1-based line/character locations
to UTF-8 byte ranges. Character columns are not bytes. Pin the observed
half-open/end behavior with tests before styling any range.

On tokenization error, keep valid preceding tokens and mark the remaining text
plain. Highlighting never returns an application error overlay.

**Step 4: Wrap sqlformat defensively**

Use PostgreSQL formatter dialect for PostgreSQL and guarded Generic formatting
for MySQL/SQLite. Reject unsupported procedural dollar bodies. Re-tokenize input
and output and compare non-whitespace token meaning before returning an edit. If
validation fails, return an actionable format error and preserve the buffer.

**Step 5: Apply formatting through the editor transaction API**

Route `EditorEffect::FormatCurrent` through the same selection/current-scope
resolver used by execution. Apply the validated formatter output with the editor
range-replacement API between one pair of history checkpoints and increment the
document revision once. Accept only `ScopeSource::Contiguous`; reject Block
without editing. Place the cursor at or near the replacement start. Add an
editor/App test proving one `u` restores the exact pre-format text.

**Step 6: Add revision-keyed analysis caching and render spans**

```rust
pub struct AnalysisKey {
    pub console_id: Uuid,
    pub document_revision: u64,
    pub dialect: SqlDialect,
}
```

Cursor-only movement must not invalidate lexical analysis.

Map SQL highlight kinds to LazyDB-owned editor highlight kinds, merge only spans
intersecting visible lines into `EditorRenderSnapshot`, and render through the
existing theme. UI code must not call sqlparser directly.

**Step 7: Run tests**

Run: `cargo test --test sql_format --test sql_highlight --test ui_render -- --nocapture`

Expected: PASS.

**Step 8: Commit**

```bash
git add src/app.rs src/editor/mod.rs src/model/editor.rs src/sql src/ui/mod.rs tests/sql_format.rs tests/sql_highlight.rs tests/ui_render.rs
git commit -m "feat(sql): format and highlight SQL"
```

### Task 11: Build Catalog-Backed Semantic Completion

**Files:**
- Create: `src/sql/completion.rs`
- Modify: `src/sql/mod.rs`
- Modify: `src/model/workspace.rs`
- Modify: `src/model/tab.rs`
- Modify: `src/action.rs`
- Modify: `src/app.rs`
- Modify: `src/runtime.rs`
- Modify: `src/db/postgres.rs`
- Modify: `src/db/mysql.rs`
- Modify: `src/ui/mod.rs`
- Create: `tests/sql_completion.rs`
- Modify: `tests/postgres_adapter.rs`
- Modify: `tests/mysql_adapter.rs`
- Modify: `tests/ui_render.rs`

**Step 1: Write failing pure completion tests**

Build catalog fixtures using `CatalogNode` parent IDs. Cover FROM/JOIN relations,
schema qualification, alias columns, routine positions, default-schema ranking,
case-insensitive prefix matching, stable tie ordering, replacement ranges,
catalog-generation invalidation, and dialect-safe insertion for reserved words,
spaces, mixed case, qualified names, and already-quoted prefixes. PostgreSQL and
SQLite use doubled double quotes; MySQL uses doubled backticks.

**Step 2: Run and verify failure**

Run: `cargo test --test sql_completion -- --nocapture`

Expected: FAIL.

**Step 3: Load routine catalog nodes where the backend exposes them**

Extend PostgreSQL catalog loading from `pg_proc`/`pg_namespace` and MySQL loading
from `information_schema.routines`, producing existing `CatalogKind::Function`
and `CatalogKind::Procedure` nodes under their schema with signatures/details.
Keep SQLite routine completion keyword-only because SQLite has no equivalent
discoverable user-routine catalog. Add decoder fixtures and environment-gated
adapter assertions; do not make completion itself query these sources.

**Step 4: Add catalog snapshot identity**

Increment `ExplorerState::catalog_generation` whenever nodes are replaced or the
active connection identity changes. Build immutable completion indexes by folded
name and parent ID; do not hardcode `native_path` indices.

Define a semantic-analysis key that extends the lexical Task 10 key with
`connection: ConnectionIdentity` and `catalog_generation`; do not create a second
catalog-generation source.

**Step 5: Implement context and ranking**

```rust
pub struct CompletionCandidate {
    pub label: String,
    pub insert_text: String,
    pub kind: CompletionKind,
    pub detail: Option<String>,
    pub replace: TextRange,
    pub score: CompletionScore,
}
```

Rank context fit, prefix fit, current schema, then stable lexical order. Return
keywords when no catalog is loaded. Never call Runtime or SQLx from completion.
Reuse the same catalog indexes to upgrade syntactic identifier highlights to
known table, view, column, function, and procedure highlight kinds.

Keep `insert_text` as the exact dialect-quoted database identifier, separate from
its display fields. Sanitize database-supplied candidate labels/details only at
the display projection boundary; never insert the sanitized placeholder text.

**Step 6: Add automatic and explicit triggering**

After `.`, trigger immediately. After identifier edits, emit a generation-safe
`ScheduleCompletion` command for about 120 ms; Runtime cancels the prior timer for
that console. Define one complete `CompletionScheduleKey { console_id,
document_revision, connection, catalog_generation }` and carry it unchanged in
both the command and `CompletionDue` result action. `Ctrl-Space` triggers
synchronously. Ignore a due event unless the complete key still matches.

**Step 7: Add popup reducer and renderer**

Store open candidates and selected index in the console model. While open,
`Ctrl-N/P`, Enter, and Escape have first priority. Acceptance replaces only the
candidate range as one undoable edit. Render at most ten rows, cursor-anchored and
viewport-clamped, with sanitized label/kind/detail. Add a hostile catalog-name UI
test containing ESC/OSC/C0 controls and prove the accepted SQL still uses the raw,
properly quoted identifier rather than display placeholders.

**Step 8: Run tests**

Run: `cargo test --test sql_completion --test postgres_adapter --test mysql_adapter --test ui_render -- --nocapture`

Expected: PASS.

**Step 9: Commit**

```bash
git add src/action.rs src/app.rs src/db/mysql.rs src/db/postgres.rs src/model src/runtime.rs src/sql src/ui/mod.rs tests/mysql_adapter.rs tests/postgres_adapter.rs tests/sql_completion.rs tests/ui_render.rs
git commit -m "feat(sql): add semantic completion"
```

### Task 12: Create Immutable Execution Drafts and Confirmation

**Prerequisite:** Reconfirm the Dynamic Profile Manager identity-safety task is
present and the worktree is clean. Re-read `action.rs`, `app.rs`, `workspace.rs`,
and `runtime.rs` before editing.

**Files:**
- Create: `src/sql/execution.rs`
- Modify: `src/sql/mod.rs`
- Modify: `src/cli.rs`
- Create: `src/model/transaction.rs`
- Modify: `src/model/mod.rs`
- Modify: `src/model/tab.rs`
- Modify: `src/model/workspace.rs`
- Modify: `src/action.rs`
- Modify: `src/app.rs`
- Modify: `src/input/keymap.rs`
- Modify: `src/runtime.rs`
- Modify: `src/ui/mod.rs`
- Create: `tests/sql_execution.rs`
- Modify: `tests/app_flow.rs`

**Step 1: Write failing execution-policy tests**

Cover selection/current/full scope, no whole-buffer fallback, exact Visual Block
materialization without bounding-range text, one read-only direct run,
confirmation for DML/DDL/transaction/Unknown/multiple/full-buffer, confirm-all
policy, exact SQL snapshots, stale document revision, stale connection identity,
stale transaction generation/mode/state, closed tab, double confirm, cancel,
F5 versus Shift-F5, one running query per console, and retention of the dispatched
snapshot after success, failure, or cancellation. Prove confirmed transaction
control cannot emit an ordinary pool query before the worker routing in Task 16.

**Step 2: Run and verify failure**

Run: `cargo test --test sql_execution -- --nocapture`

Expected: FAIL because F5 still runs the full current buffer.

**Step 3: Add transaction snapshot types and the immutable draft**

Execution previews need stable transaction labels before the transaction worker
exists. Add the final mode/state enums now with default AUTO/Idle behavior; Task
13 adds control semantics and reducer transitions.

```rust
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TransactionMode { #[default] Auto, Manual }

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TransactionState {
    #[default] Idle,
    Starting,
    Active,
    Aborted,
    Committing,
    RollingBack,
    OutcomeUnknown,
}
```

Add `transaction_generation: u64` to `ConsoleTab` now; Task 13 adds its transition
semantics. Then define the immutable draft:

```rust
pub struct ExecutionDraft {
    pub console_id: Uuid,
    pub query_generation: u64,
    pub connection: ConnectionIdentity,
    pub transaction_generation: u64,
    pub document_revision: u64,
    pub scope: ScopeKind,
    pub source: ScopeSource,
    pub sql: String,
    pub dialect: SqlDialect,
    pub statement_count: usize,
    pub risks: Vec<SqlRisk>,
    pub transaction_mode: TransactionMode,
    pub transaction_state: TransactionState,
}
```

Store the draft inside `Overlay::ExecutionConfirm`. Confirm dispatches
`draft.sql`; it never rereads editor text. Revalidate console, revision,
connection, query generation, transaction generation, mode, and state before
creating a Command. Store a LazyDB-owned last-execution record derived from the
draft before dispatch and retain its exact SQL on success, failure, and
cancellation; never reconstruct failed SQL from the current editor buffer.

**Step 4: Add confirmation policy configuration**

Define `ConfirmationPolicy::{RiskyOnly, Always}` and expose
`--confirm-execution risky|always`, with RiskyOnly as default. Keep
`App::new(profiles)` as the default constructor for existing callers and add an
explicit constructor/setter used by CLI startup and tests. A read-only profile
remains enforced by adapters and cannot be overridden by this policy.

**Step 5: Add run-current and run-all routing**

F5/`<leader>r` resolve selection then cursor statement. Shift-F5/`<leader>R`
create a full-buffer draft and always confirm. Update the old multi-statement
`app_flow` setup to use explicit run-all plus confirmation or separate runs.
Normal read-only/DML/DDL/Unknown drafts use the identity-safe AUTO path at this
checkpoint. A confirmed `SqlRisk::TransactionControl` must fail closed with a
clear status and emit no `RunQuery`/pool command until Task 16 replaces this gate
with pinned-worker domain routing. Mixed control/data remains rejected.

**Step 6: Render and drive the confirmation overlay**

Show scope/lines, count, risk, database, transaction state, and full scrollable
SQL. Render the preview through Task 5's inert display projection while retaining
the raw draft separately. Include the explicit MySQL MANUAL-DDL implicit-commit
warning. Cancel has initial focus. Enter activates the focused button; `e`
executes; Escape/`n` cancels.

**Step 7: Run focused tests**

Run: `cargo test --test sql_execution --test app_flow --test keymap --test ui_render -- --nocapture`

Expected: PASS.

**Step 8: Commit**

```bash
git add src/action.rs src/app.rs src/cli.rs src/input/keymap.rs src/model src/runtime.rs src/sql src/ui/mod.rs tests/app_flow.rs tests/keymap.rs tests/sql_execution.rs tests/ui_render.rs
git commit -m "feat(sql): confirm scoped execution"
```

## Phase C: Pinned Manual Transactions

### Task 13: Define Transaction Control Semantics and State Transitions

**Files:**
- Modify: `src/model/transaction.rs`
- Modify: `src/model/mod.rs`
- Modify: `src/model/tab.rs`
- Create: `src/sql/transaction.rs`
- Modify: `src/sql/mod.rs`
- Create: `tests/transaction_sql.rs`
- Create: `tests/transaction_reducer.rs`

**Step 1: Write failing state and SQL-control tests**

Test lazy MANUAL entry, standalone BEGIN/START, COMMIT/END, outer ROLLBACK,
ROLLBACK TO SAVEPOINT, SAVEPOINT/RELEASE, nested BEGIN, mixed control/data drafts,
unsupported chained controls, read-only restrictions, and MySQL implicit-commit
classification.

**Step 2: Run and verify failure**

Run: `cargo test --test transaction_sql --test transaction_reducer -- --nocapture`

Expected: FAIL.

**Step 3: Complete transaction domain state**

Use the mode/state enums and `ConsoleTab::transaction_generation` introduced in
Task 12. Define `TransactionExitChoice` and typed deferred-intent/prompt queue
models, and add pure transition helpers used by reducer tests. Increment the
transaction generation whenever a worker starts or is invalidated. If console
persistence exists, persist mode only; restore every live state as Idle with a
fresh generation.

**Step 4: Implement control classification**

Map standalone outer controls to domain actions. SAVEPOINT operations execute as
SQL only in Active MANUAL. Reject mixed control/data drafts everywhere in the
first version. Build a canonical backend BEGIN request rather than forwarding
arbitrary user BEGIN text. Reject `AND CHAIN`, vendor autocommit controls, and
unsupported `SET TRANSACTION` until explicitly modeled. Accept only bare `BEGIN`,
`BEGIN WORK`, and bare `START TRANSACTION`; reject isolation, access-mode, and
backend-specific options so the previewed request is never silently weakened.

**Step 5: Run tests**

Run: `cargo test --test transaction_sql --test transaction_reducer -- --nocapture`

Expected: PASS for pure classification and model transitions.

**Step 6: Commit**

```bash
git add src/model src/sql tests/transaction_sql.rs tests/transaction_reducer.rs
git commit -m "feat(transactions): define transaction state"
```

### Task 14: Refactor Query Result Collection for Pool and Session Executors

**Files:**
- Modify: `src/db/query.rs`
- Modify: `src/db/postgres.rs`
- Modify: `src/db/mysql.rs`
- Modify: `src/db/sqlite.rs`
- Modify: `src/db/mod.rs`
- Modify: `tests/postgres_adapter.rs`
- Modify: `tests/mysql_adapter.rs`
- Modify: `tests/sqlite_adapter.rs`

**Step 1: Add behavior-preservation tests**

Extend adapter tests to assert multiple result sets, affected-row counts, timing,
NULL/empty distinctions, and errors before changing executor plumbing.

**Step 2: Run tests**

Run: `cargo test --test sqlite_adapter --test postgres_adapter --test mysql_adapter -- --nocapture`

Expected: PASS before refactor; this is the characterization baseline.

**Step 3: Extract backend-neutral accumulation**

Move result-set transitions, affected rows, row count, and timing into a small
`QueryOutcomeAccumulator` in `db/query.rs`. Keep concrete row decoding inside
each adapter.

For each adapter, provide concrete methods for:

```rust
execute_pool(&self, sql: &str)
execute_connection(&self, connection: &mut ConcreteConnection, sql: &str)
```

Both methods create `raw_sql(AssertSqlSafe(sql)).fetch_many(executor)` streams and
feed the same collector. Do not attempt a `dyn sqlx::Executor`; SQLx executors are
database-associated and not object-safe for this use.

**Step 4: Run all adapter tests**

Run: `cargo test --test sqlite_adapter --test postgres_adapter --test mysql_adapter -- --nocapture`

Expected: PASS with unchanged outcomes.

**Step 5: Commit**

```bash
git add src/db tests/mysql_adapter.rs tests/postgres_adapter.rs tests/sqlite_adapter.rs
git commit -m "refactor(db): share query result collection"
```

### Task 15: Implement the Serial Transaction Worker Protocol

**Files:**
- Create: `src/db/transaction.rs`
- Create: `src/runtime/transaction.rs`
- Modify: `src/db/mod.rs`
- Modify: `src/db/postgres.rs`
- Modify: `src/db/mysql.rs`
- Modify: `src/db/sqlite.rs`
- Modify: `src/runtime.rs`
- Modify: `src/action.rs`

**Step 1: Write failing fake-worker tests**

Inside `src/runtime/transaction.rs`, use a deterministic fake backend to cover
begin, serial execute, commit, rollback, execution cancellation, worker identity
mismatch, begin failure, server commit rejection, lost commit acknowledgement,
rollback acknowledgement loss, native-cancel failure followed by hard close, and
shutdown rollback. The fake must distinguish dropping a client future from
actually cancelling/quarantining the backend operation.

**Step 2: Run and verify failure**

Run: `cargo test --lib runtime::transaction::tests -- --nocapture`

Expected: FAIL because no worker exists.

**Step 3: Define worker requests and dispositions**

```rust
enum TransactionRequest {
    Execute {
        query_generation: u64,
        sql: String,
        cancel: oneshot::Receiver<()>,
    },
    Commit,
    Rollback,
    Shutdown,
}

enum WorkerDisposition {
    Committed,
    RolledBack,
    CancelledAndRolledBack,
    ImplicitlyEnded,
    Quarantine,
}
```

Each Runtime registry entry stores connection identity, transaction generation,
request sender, worker handle, current manual-query cancellation sender, and an
adapter-owned forced-close handle. The forced-close handle can set SQLite's
active progress-cancellation flag and reports when the detached concrete
connection has actually closed.

**Step 4: Add adapter-owned session acquisition and cancellation metadata**

Keep concrete pools and connections private. Add module-private adapter entry
points that acquire one `PoolConnection`, collect backend cancellation metadata,
and spawn/run the common request protocol. PostgreSQL records
`pg_backend_pid()` and cancels through `SELECT pg_cancel_backend($1)` on a control
pool connection. MySQL records `CONNECTION_ID()` and uses a separately acquired
control connection for trusted-integer `KILL QUERY`. SQLite installs a
connection-local progress handler backed by an `Arc<AtomicBool>` before query
execution and always removes it afterward. Do not expose concrete SQLx types
through `DatabaseConnection` or Runtime's public action types.

Wrap each acquired pool connection in an adapter-specific pinned guard that is
armed for quarantine while a transaction may be live. On an unexpected task
drop/abort, its `Drop` detaches the `PoolConnection` and spawns backend cleanup;
PostgreSQL/MySQL use `ConcreteConnection::close_hard()`, while SQLite first sets
the progress-cancellation flag and awaits `SqliteConnection::close()` so its
worker thread terminates. Signal the shared forced-close handle only after that
cleanup future completes. A possibly dirty session must never return to the pool.
Disarm the guard only after a known commit/rollback acknowledgement and normal
release. Unit-test this guard with a fake close recorder.

**Step 5: Implement backend-specific worker wrappers**

In each spawned task:

1. Acquire a concrete `PoolConnection`.
2. Call the backend's public SQLx `TransactionManager::begin` on the concrete
   connection and mark the guard live only after acknowledgement.
3. Process requests serially and call the adapter's connection executor on that
   same connection.
4. Make `TransactionManager::commit` non-cancellable once sent. Because it
   borrows rather than consumes the connection, inspect structured error and
   transaction depth after failure; this preserves SQLite BUSY retry semantics.
5. Use `TransactionManager::rollback` for acknowledged rollback and verify depth
   is zero before disarming the guard or returning the connection.
6. When cancellation cannot be acknowledged or the disposition is otherwise
   uncertain/quarantined, detach the `PoolConnection` and call
   backend-specific forced cleanup; `PoolConnection` itself has no `close_hard`
   method.

Do not wrap the connection in `sqlx::Transaction`: `commit(self)` consumes that
wrapper and queues rollback on error, which makes a retryable SQLite BUSY commit
impossible to represent truthfully. The worker protocol plus armed guard provide
the ownership and rollback-on-abnormal-exit boundary instead.

Do not issue raw BEGIN/COMMIT on an ordinary pooled executor. Keeping the
`PoolConnection` in the worker scope permits hard-closing only that physical
session after an unknown acknowledgement.

**Step 6: Implement manual cancellation**

Runtime creates a one-shot cancellation channel for each manual Execute. The
worker selects between execution completion and cancellation. On cancellation,
request backend cancellation first and keep polling the query to its terminal
cancel response; then explicitly roll back the complete transaction and
terminate the worker. If the backend cancel request fails or times out, drop the
query future, leave the guard armed, detach and force-close the pinned connection,
and report cancelled-and-disconnected rather than claiming a rollback
acknowledgement. For SQLite, set the progress flag before dropping the future and
await acknowledged connection close/worker-thread termination. Never return that
connection to the pool. Auto query cancellation remains independent.

**Step 7: Run worker tests**

Run: `cargo test --lib runtime::transaction::tests -- --nocapture`

Expected: PASS.

**Step 8: Commit**

```bash
git add src/action.rs src/db/mod.rs src/db/mysql.rs src/db/postgres.rs src/db/sqlite.rs src/db/transaction.rs src/runtime.rs src/runtime/transaction.rs
git commit -m "feat(transactions): add pinned session workers"
```

### Task 16: Integrate Manual Transactions with Reducer and Runtime

**Files:**
- Modify: `src/action.rs`
- Modify: `src/app.rs`
- Modify: `src/runtime.rs`
- Modify: `src/model/tab.rs`
- Modify: `src/db/mod.rs`
- Modify: `tests/transaction_reducer.rs`
- Modify: `tests/sql_execution.rs`

**Step 1: Add failing end-to-end reducer tests**

Cover AUTO execution, MANUAL Idle lazy begin, Active reuse, commit/rollback back
to MANUAL Idle, standalone SQL controls, stale connection/query/transaction
generations, PostgreSQL Aborted restrictions, MySQL/SQLite ordinary error
retention, and unavailable actions in transient/unknown states. Assert every AUTO
and MANUAL result carries connection identity plus query generation, every MANUAL
result also carries transaction generation, and no late result mutates a newer
session. Test that cancelling an active MANUAL query first presents the complete
transaction-loss warning and dispatches nothing while Keep Running is selected.
Race query completion against destructive confirmation and prove a stale prompt
cannot cancel a newer query. Cover lost rollback acknowledgement separately from
a known rollback rejection and prove neither is reported as successful rollback.

**Step 2: Run and verify failure**

Run: `cargo test --test transaction_reducer --test sql_execution -- --nocapture`

Expected: FAIL.

**Step 3: Add explicit actions and commands**

Add user actions for setting/toggling mode, commit, rollback, and clearing a
verified OutcomeUnknown. Add result actions for started/start-failed, committed/
commit-failed, rolled-back/rollback-failed, implicit end, and manual cancellation.

Define immutable cancellation intent with console UUID, query generation,
transaction generation, and connection identity. Store that draft in the prompt;
query completion dismisses the matching prompt, and destructive confirmation
must revalidate all fields before signalling Runtime.

Split query dispatch into identity-tagged AUTO and MANUAL commands. Every manual
command/event carries query generation, transaction generation, and connection
identity. Every AUTO result event also carries query generation and connection
identity; identity checks are symmetric on commands and results.

Preserve structured `DatabaseError` in transaction failure events. Do not reduce
errors to strings before the reducer decides Aborted, Active, or OutcomeUnknown.

**Step 4: Implement reducer transitions**

MANUAL Idle execution increments transaction generation and enters Starting.
Active routes to the matching worker. Aborted allows rollback only. Committing,
RollingBack, and OutcomeUnknown reject execution. Cancel increments query
generation immediately so a racing completion cannot overwrite Cancelled.

Replace Task 12's fail-closed transaction-control gate here: standalone
BEGIN/START changes AUTO to MANUAL and starts the pinned worker, COMMIT/ROLLBACK
become worker requests, and savepoint SQL reaches only an already-active matching
worker. No transaction-control text is ever sent through AUTO/pool execution.

PostgreSQL server statement errors enter Aborted. MySQL/SQLite ordinary database
and constraint errors stay Active. Connection-fatal errors terminate/quarantine
the worker.

**Step 5: Handle MySQL implicit DDL**

Require a single statement and explicit confirmation. Once sent, terminate the
worker and return to MANUAL Idle whether DDL succeeds or fails; output that prior
work may have committed. Reject unsupported temporary-DDL edge cases until
capability behavior is explicit.

**Step 6: Add explicit MANUAL cancellation consent**

On Ctrl-C during an active MANUAL query, open a typed confirmation containing
`Cancelling rolls back all uncommitted work in this transaction`. Use Keep
Running as the initial action and Cancel Query + Roll Back as the destructive
action. Only a still-current destructive response sends the worker cancellation
signal; a stale response is discarded with an informational status.
After completion, output whether rollback was acknowledged or the connection was
hard-closed; both return the console to MANUAL Idle with a new transaction
generation.

**Step 7: Handle unknown acknowledgements**

Network/protocol loss after commit or rollback, PostgreSQL `40003`, or `08007`
enters OutcomeUnknown, closes/quarantines the session, and never reports the
operation as successful. Never retry. Block mutating execution until reconnect
or explicit user verification/clear. A known server commit rejection such as
SQLite BUSY returns to Active and may be retried.

**Step 8: Run tests**

Run: `cargo test --test transaction_reducer --test sql_execution -- --nocapture`

Expected: PASS.

**Step 9: Commit**

```bash
git add src/action.rs src/app.rs src/db/mod.rs src/model/tab.rs src/runtime.rs tests/sql_execution.rs tests/transaction_reducer.rs
git commit -m "feat(transactions): integrate manual query sessions"
```

### Task 17: Add Deferred Transaction Exit Prompts and Safe Shutdown

**Files:**
- Modify: `src/model/transaction.rs`
- Modify: `src/model/workspace.rs`
- Modify: `src/action.rs`
- Modify: `src/app.rs`
- Modify: `src/input/keymap.rs`
- Modify: `src/runtime.rs`
- Modify: `src/runtime/transaction.rs`
- Modify: `src/ui/mod.rs`
- Modify: `tests/transaction_reducer.rs`
- Modify: `tests/ui_render.rs`

**Step 1: Write failing deferred-intent tests**

Cover closing a console, MANUAL to AUTO, connection switch, active-profile
deletion/disconnect, and quit. Test Commit, Rollback, and Cancel, multiple active
consoles resolved one at a time, running-query blocking, Aborted commit disabled,
failed resolution retaining the prompt, and ordinary tab switching with no
prompt. Cover reconnect from OutcomeUnknown and `:tx clear`: the latter captures
connection/transaction generations, defaults to Cancel, rejects stale approval,
and resets to MANUAL Idle with a fresh generation only after explicit external-
verification confirmation. Add a fake hung-worker shutdown test proving abort
arms the pinned guard and hard-closes rather than returning its connection.

**Step 2: Run and verify failure**

Run: `cargo test --test transaction_reducer --test ui_render -- --nocapture`

Expected: FAIL because close/switch/quit are immediate.

**Step 3: Add deferred intents and prompt queue**

Represent the original operation as a typed `DeferredIntent`, not a closure.
Queue affected console UUIDs and resolve them one at a time. Only replay the
deferred intent after every commit/rollback acknowledgement. Cancel discards the
intent and leaves state unchanged.

**Step 4: Add the transaction-exit overlay**

Render console name, current transaction state, and Commit/Rollback/Cancel.
Rollback has initial focus. Disable Commit for Aborted and all resolution buttons
while a query is running, with an instruction to wait or cancel.

**Step 5: Add verified OutcomeUnknown recovery**

Reconnect is the normal recovery and clears OutcomeUnknown only after a new
`ConnectionIdentity` is active. `EditorEffect::ClearTransactionOutcome` opens a
typed confirmation that says LazyDB cannot know whether commit/rollback reached
the server and asks the user to verify externally. Cancel has initial focus.
Confirm revalidates console, connection, and transaction generations, records an
audit output entry, increments transaction generation, and returns to MANUAL
Idle. It never retries SQL or reports the previous operation as committed or
rolled back.

**Step 6: Harden Runtime shutdown**

On normal shutdown, request rollback from every worker, wait for a bounded
timeout, then abort/drop only remaining tasks. The armed pinned guard from Task
15 must detach and close a timed-out worker's physical connection during abort.
Before aborting SQLite, set its forced-close progress flag; then await every
forced-close completion (including SQLite worker-thread termination) before
closing pools. PostgreSQL/MySQL use hard close; SQLite uses acknowledged close.
Never commit during shutdown and never rely on dropping a borrowed query future
as server-side cancellation.

**Step 7: Run tests**

Run: `cargo test --lib runtime::transaction::tests --test transaction_reducer --test ui_render -- --nocapture`

Expected: PASS.

**Step 8: Commit**

```bash
git add src/action.rs src/app.rs src/input/keymap.rs src/model src/runtime.rs src/runtime/transaction.rs src/ui/mod.rs tests/transaction_reducer.rs tests/ui_render.rs
git commit -m "feat(transactions): guard transaction exits"
```

### Task 18: Prove SQLite Connection Pinning and Rollback Behavior

**Files:**
- Create: `tests/sqlite_transactions.rs`
- Modify: `src/db/sqlite.rs`
- Modify: `src/runtime/transaction.rs`

**Step 1: Write failing real SQLite integration tests**

Use a temporary file database and test:

- A temp table or connection-local property proves every MANUAL statement uses
  the same physical connection.
- Insert then rollback leaves no row.
- Insert then commit persists the row.
- Constraint/syntax errors leave the transaction usable where SQLite guarantees
  that behavior.
- A long-running recursive query is interrupted through the progress handler;
  cancellation rolls back all earlier uncommitted work and the handler is
  removed before the connection is released.
- Commit BUSY remains Active and can be retried.
- Severe connection errors terminate/quarantine rather than claim Active.
- Commit/rollback returns the connection without an open transaction.

**Step 2: Run and verify failure**

Run: `cargo test --test sqlite_transactions -- --nocapture`

Expected: FAIL until the real worker path is complete.

**Step 3: Fix only observed SQLite transaction gaps**

Keep default deferred BEGIN. Map ordinary constraint errors to Active. Treat
FULL, IOERR, INTERRUPT, NOMEM, and connection-fatal errors conservatively. Do not
generalize based on message substrings when a structured SQLite code exists.
Exercise commit through `SqliteTransactionManager` on the pinned connection, not
consuming `sqlx::Transaction::commit`; after BUSY, assert transaction depth is
still one before retaining Active and permitting retry.

**Step 4: Run SQLite tests**

Run: `cargo test --test sqlite_transactions --test sqlite_adapter -- --nocapture`

Expected: PASS.

**Step 5: Commit**

```bash
git add src/db/sqlite.rs src/runtime/transaction.rs tests/sqlite_transactions.rs
git commit -m "test(transactions): verify SQLite session safety"
```

### Task 19: Finish Transaction UI, Keys, and Contextual Help

**Files:**
- Modify: `src/input/keymap.rs`
- Modify: `src/ui/mod.rs`
- Modify: `src/ui/theme.rs`
- Modify: `tests/keymap.rs`
- Modify: `tests/ui_render.rs`

**Step 1: Write failing key and render tests**

Test `<leader>tt/tc/tr`, Ex equivalents including `:tx clear`, disabled actions,
popup priority, execution/exit/MANUAL-cancel/OutcomeUnknown-clear confirmation
defaults, stale prompt dismissal, the full cancellation-loss warning, MySQL
implicit-commit warning, and every label:

```text
TX AUTO
TX MANUAL:IDLE
TX MANUAL:STARTING
TX MANUAL:ACTIVE
TX ABORTED
TX COMMITTING
TX ROLLING BACK
TX OUTCOME UNKNOWN
```

Assert 120x36 and 80x24 layouts remain readable and the minimum layout remains
actionable.

**Step 2: Run and verify failure**

Run: `cargo test --test keymap --test ui_render -- --nocapture`

Expected: FAIL against the static TX AUTO header/help.

**Step 3: Render final status and help**

Use existing warning/error theme colors and textual labels. Group editor help into
Navigation, Editing, SQL, Completion, Transaction, and Tabs/Windows. Show only
implemented keys. Keep `?` as backward search in Editor and help in
Explorer/Results; Editor help is F1 or `<leader>?`.

**Step 4: Run tests**

Run: `cargo test --test keymap --test ui_render -- --nocapture`

Expected: PASS.

**Step 5: Commit**

```bash
git add src/input/keymap.rs src/ui tests/keymap.rs tests/ui_render.rs
git commit -m "feat(ui): expose editor and transaction controls"
```

### Task 20: Run PostgreSQL/MySQL Contracts, Documentation, and Final Gates

**Files:**
- Modify: `tests/postgres_adapter.rs`
- Modify: `tests/mysql_adapter.rs`
- Modify: `README.md`
- Modify: `docs/architecture.md`
- Modify: `docs/keybindings.md`

**Step 1: Add environment-gated server tests**

PostgreSQL tests cover pinned temp state, commit, rollback, statement-error
Aborted behavior, rollback recovery, and a long-running `pg_sleep` cancelled via
the recorded backend PID. MySQL tests cover pinned session state, a long-running
sleep cancelled through `KILL QUERY`, and implicit DDL commit on success and
failure. After cancellation, assert the server operation ended and earlier
uncommitted writes are absent; dropping only the SQLx future is not sufficient.
Tests return early only when their existing URL environment variable is absent.

**Step 2: Run optional integrations when configured**

With `LAZYDB_TEST_POSTGRES_URL` and `LAZYDB_TEST_MYSQL_URL` exported to non-empty
test URLs, run:

```bash
cargo test --test postgres_adapter manual_ -- --nocapture
cargo test --test mysql_adapter manual_ -- --nocapture
```

Expected: PASS when services are configured. If variables are absent, record that
these integrations were skipped; do not claim they passed against real servers.

**Step 3: Update user and architecture documentation**

Document the practical Vim contract, Insert control keys, `?` focus behavior,
run scope rules, confirmation policy, completion, Ex commands, transaction keys,
pinned-worker ownership, cancellation rollback, MySQL implicit DDL, and
OutcomeUnknown. Remove descriptions of whole-buffer F5 and static TX AUTO.

**Step 4: Verify formatting**

Run: `cargo fmt --check`

Expected: PASS because every owning task formatted before commit. If it fails,
run `cargo fmt --all`, inspect every changed source file, rerun its focused tests,
and include it intentionally rather than hiding source changes in a docs-only
commit.

**Step 5: Run the complete verification gate**

Run:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

Expected: all commands PASS.

**Step 6: Inspect the final change set**

Run:

```bash
git status --short
git diff --check
git diff --stat
```

Expected: only intended editor, SQL, transaction, UI, test, dependency, and
documentation changes; no credentials, generated databases, or unrelated files.

**Step 7: Commit**

```bash
git add README.md docs/architecture.md docs/keybindings.md tests/mysql_adapter.rs tests/postgres_adapter.rs
git commit -m "docs: document SQL editor workflows"
```

## Final Acceptance Checklist

- Editor text round-trips exactly, including trailing newlines and Unicode.
- Hostile terminal controls are inert in editor and execution-preview rendering
  without changing the raw SQL sent to the database.
- Insert `Ctrl-W`, `Ctrl-U`, and `Ctrl-H` retain Vim semantics.
- Normal `?` searches backward; F1/`<leader>?` opens editor help.
- Selection/current execution never silently expands to the whole buffer.
- Visual Block execution contains only its ordered row slices; block formatting
  fails without editing rather than touching its bounding rectangle.
- Confirmation executes the immutable previewed SQL snapshot.
- Confirmed transaction controls never execute through an ordinary pool query.
- Completion performs no database I/O while typing.
- Completion timers validate revision, connection identity, and catalog
  generation as one key; PostgreSQL/MySQL routine nodes feed routine completion.
- Formatting and substitution are each undoable in one step.
- Every MANUAL statement uses one pinned physical connection.
- Commit/rollback returns to MANUAL Idle; mode does not silently change to AUTO.
- Cancelling a MANUAL query rolls back the complete current transaction.
- Cancellation reaches the backend operation or hard-closes/quarantines its
  physical connection; dropping a client future alone is not accepted.
- PostgreSQL Aborted, MySQL implicit commit, and unknown acknowledgement states
  are represented truthfully.
- Close, connection switch, MANUAL-to-AUTO, and quit resolve active transactions
  through Commit/Rollback/Cancel.
- Existing profile-manager, catalog, query, mouse, UI, persistence, and security
  tests remain green.
