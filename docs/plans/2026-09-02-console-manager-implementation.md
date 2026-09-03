# Console Manager Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 将 `Space s` 改造成统一的 Console Manager，允许用户按优先级浏览全部 console，并在同一悬浮窗口中 focus/reopen、新建、删除、重命名和搜索 console。

**Architecture:** 复用现有 `Overlay::SqlEditorList`、`ConsoleRecord` 和 `activate_sql_editor` 生命周期，不引入新的持久化格式。把列表状态升级为浏览、搜索、重命名、删除确认四种模式，以 console UUID 维护稳定选择；由 `App` 提供唯一的过滤排序投影，确保渲染、移动和 action 目标完全一致。删除继续经过现有事务退出保护和 SQL 文件删除命令，但使用 manager 内部确认状态，取消或完成操作后返回列表。

**Tech Stack:** Rust 2024、Ratatui 0.30、Crossterm 0.29、UUID、现有 `Action`/`Overlay`/`EditorWorkspace`/workspace persistence 架构。

---

## Design Decisions

1. 保留内部类型名 `SqlEditorListState` 和 overlay 变体 `Overlay::SqlEditorList`，只把用户可见名称改成 `CONSOLES`/`Console Manager`。这样避免无价值的大范围重命名，同时完整升级行为。
2. `Space s` 是唯一的 manager 入口；移除 `Space n`、`Space e` 和旧的“跳到第一个 SQL console”行为。新建 console 改为在 manager 中按 `a`。
3. 浏览模式下 `a`、`d`、`r` 是命令，普通字符不再直接修改过滤条件。按 `/` 进入搜索模式后，字符、Backspace 和常用文本编辑键才作用于搜索框。
4. `Esc` 分层退出：重命名/删除确认回到浏览模式，搜索模式清空查询并回到浏览模式，浏览模式关闭 overlay。
5. 选择使用 `Option<Uuid>`，不使用排序后的下标作为持久状态。排序、搜索、重命名或删除改变可见顺序后仍保持同一 console；该 ID 不可见时才落到第一项或相邻项。
6. console 排序键固定为：精确名称 `console` 优先，然后 `open == true`，最后 `open == false`；同组按 `name.to_lowercase()`，再按原始 `name`，最后按 UUID，保证确定性。
7. 名称校验规则：trim 后不能为空；当前 workspace 内不区分大小写唯一；提交时保存 trim 后的名称。SQL 文件以 UUID 命名，重命名不移动文件。
8. `console` 只是特殊排序名称，不再代表不可关闭或不可删除的对象身份。删除最后一个 console 后沿用现有规则创建新的 `console`；关闭最后一个打开 tab 后优先 reopen 其他已关闭 console，否则创建 `console`。
9. `Enter` 复用 `activate_sql_editor(id)`：已打开 console 只 focus，已关闭 console reopen 并 focus；SQL editor session 和持久化 SQL 内容不另建副本。
10. 删除必须二次确认。已打开且存在待处理 manual transaction 的 console 先经过现有 transaction-exit prompt，再返回 manager 的删除确认；确认后才发出 `Command::DeleteSqlFile(id)`。
11. 第一版不增加鼠标行选择、新建名称输入、console 拖拽排序或持久化自定义顺序。这些不属于恢复关闭 console 的核心需求。

## Target Behavior

在浏览模式中显示类似：

```text
┌─ CONSOLES ─────────────────────────────────────────────┐
│ > console                                      OPEN   │
│   console_2                                    OPEN   │
│   analyze_orders                             CLOSED   │
│   backup_query                               CLOSED   │
│                                                        │
│ j/k move  Enter open  a new  d delete  r rename       │
│ / search  Esc close                                    │
└────────────────────────────────────────────────────────┘
```

行为约束：

- 打开 manager 时，当前 tab 是 SQL console 则选中该 console；当前 tab 不是 SQL console 则选中排序后的第一项。
- `j`/`Down` 和 `k`/`Up` 循环移动，次序必须与屏幕显示一致。
- `Enter` 后关闭 overlay，激活目标 tab 并把 focus 设置为 `Focus::Editor`。
- `a` 创建下一个唯一的 `console_N`，关闭 overlay，并激活新 console。
- `r` 以当前名称预填输入框；提交成功后回到浏览模式并保持该 UUID 选中。
- `d` 进入明确显示 console 名称的确认模式；确认后返回 manager，除非业务规则创建并激活了 replacement console。
- 空搜索结果显示 `No matching consoles`，此时移动、Enter、`d`、`r` 均为 no-op；`a` 和 `Esc` 仍有效。
- OPEN/CLOSED 只描述 tab 可见状态，不描述 SQL editor session 是否存在。

## Out Of Scope

- 不迁移 workspace manifest 版本，不修改 `PersistedConsole` schema。
- 不删除 `ConsoleRecord.open`，不改变 SQL 文件以 UUID 命名的策略。
- 不把 relation/dashboard tab 放进 manager。
- 不允许跨 connection workspace 管理 console；列表只显示当前 active workspace 的 `sql_editors`。
- 不改变 `Ctrl+n`、`]t`、`[t` 等无关快捷键。
- 不为旧的 `Space n`、`Space e` 保留兼容 alias。

### Task 1: Upgrade Console List State And Stable Selection

**Files:**
- Modify: `src/model/sql_editor_list.rs`
- Test: `src/model/sql_editor_list.rs` 的现有 `#[cfg(test)] mod tests`

**Step 1: 写状态模式和 UUID 选择的失败测试**

用固定 UUID 覆盖默认模式、选择和循环移动：

```rust
#[test]
fn console_list_starts_in_browse_mode_and_tracks_selection_by_id() {
    let first = Uuid::from_u128(1);
    let second = Uuid::from_u128(2);
    let mut state = SqlEditorListState::new(Some(second));

    assert_eq!(state.mode, SqlEditorListMode::Browse);
    assert_eq!(state.selected_id, Some(second));

    state.move_selection(1, &[first, second]);
    assert_eq!(state.selected_id, Some(first));
    state.move_selection(-1, &[first, second]);
    assert_eq!(state.selected_id, Some(second));
}
```

再增加边界测试：空列表清空选择；当前 UUID 不在可见列表时选择第一项；向上从第一项循环到最后一项。

**Step 2: 运行测试并确认失败**

Run: `cargo test --lib model::sql_editor_list`

Expected: FAIL，`SqlEditorListMode`、`new` 或 `selected_id` 尚未定义。

**Step 3: 增加最小状态模型**

在 `src/model/sql_editor_list.rs` 中引入：

```rust
use uuid::Uuid;

use crate::model::text_input::TextInput;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum SqlEditorListMode {
    #[default]
    Browse,
    Search,
    Rename {
        console_id: Uuid,
        input: TextInput,
        error: Option<String>,
    },
    DeleteConfirm {
        console_id: Uuid,
    },
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SqlEditorListState {
    pub query: TextInput,
    pub selected_id: Option<Uuid>,
    pub mode: SqlEditorListMode,
}
```

实现：

```rust
pub fn new(selected_id: Option<Uuid>) -> Self;
pub fn visible_query(&self) -> &str;
pub fn move_selection(&mut self, delta: isize, visible_ids: &[Uuid]);
pub fn reconcile_selection(&mut self, visible_ids: &[Uuid]);
pub fn start_search(&mut self);
pub fn cancel_mode(&mut self) -> bool;
```

`cancel_mode` 返回是否已经处于 Browse：调用方据此决定关闭 overlay。Search 取消时清空 `query`；Rename/DeleteConfirm 取消时保留搜索条件并返回 Browse。

**Step 4: 写搜索输入和 mode cancel 的失败测试**

覆盖：

```rust
#[test]
fn cancelling_search_clears_query_before_closing_the_manager() { /* ... */ }

#[test]
fn cancelling_rename_returns_to_browse_without_clearing_search() { /* ... */ }
```

搜索输入直接委托 `TextInput` 的 `insert`、`backspace`、`delete_previous_word`、`delete_to_start`、`delete`、`move_left`、`move_right`、`move_home`、`move_end`，不要复制 UTF-8 光标算法。

**Step 5: 运行 model 测试**

Run: `cargo test --lib model::sql_editor_list`

Expected: PASS。

**Step 6: Commit**

```bash
git add src/model/sql_editor_list.rs
git commit -m "refactor(console): add manager modes and stable selection"
```

### Task 2: Centralize Filtering And Sorting

**Files:**
- Modify: `src/app.rs`，靠近 `active_console_opt`/workspace console helper
- Modify: `src/model/sql_editor_list.rs`，仅保留名称匹配纯函数时使用
- Test: `src/app.rs` 的 `#[cfg(test)] mod tests`

**Step 1: 写完整排序优先级的失败测试**

构造包含以下记录的 workspace：

```text
console      closed
Beta         open
alpha        open
delta        closed
charlie      closed
```

测试：

```rust
#[test]
fn console_manager_orders_console_then_open_then_closed_by_name() {
    let app = app_with_console_records(/* fixtures above */);

    let names = app
        .visible_sql_editors("")
        .into_iter()
        .map(|record| record.name.as_str())
        .collect::<Vec<_>>();

    assert_eq!(names, ["console", "alpha", "Beta", "charlie", "delta"]);
}
```

如果现有测试模块没有 fixture helper，就在测试内直接修改 `app.sql_editors`，不要为生产代码增加 fixture API。

**Step 2: 写过滤与稳定 tie-breaker 的失败测试**

覆盖：

- query 不区分大小写并使用 substring 匹配。
- 名为 `Console` 的记录不获得精确 `console` 特权，但仍按普通名称排序。
- `foo` 和 `Foo` 先以原始名称、再以 UUID 决定稳定顺序。

**Step 3: 运行测试并确认失败**

Run: `cargo test --lib console_manager_orders_console_then_open_then_closed_by_name`

Expected: FAIL，`visible_sql_editors` 尚未定义。

**Step 4: 实现唯一列表投影**

在 `App` 中增加仅供 crate 内部使用的方法：

```rust
pub(crate) fn visible_sql_editors(&self, query: &str) -> Vec<&ConsoleRecord> {
    let mut records = self
        .sql_editors
        .iter()
        .filter(|record| SqlEditorListState::matches(&record.name, query))
        .collect::<Vec<_>>();
    records.sort_by(|left, right| {
        console_sort_key(left).cmp(&console_sort_key(right))
    });
    records
}
```

由于返回借用记录，排序键 helper 不要分配悬垂引用。可以直接在 comparator 中依次比较：

```rust
(left.name != "console")
    .cmp(&(right.name != "console"))
    .then_with(|| (!left.open).cmp(&(!right.open)))
    .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    .then_with(|| left.name.cmp(&right.name))
    .then_with(|| left.id.cmp(&right.id))
```

数据规模是 console 数量，首版无需缓存小写名称或维护单独索引。

**Step 5: 增加 ID 投影 helper**

增加：

```rust
fn visible_sql_editor_ids(&self, query: &str) -> Vec<Uuid> {
    self.visible_sql_editors(query)
        .into_iter()
        .map(|record| record.id)
        .collect()
}
```

后续所有移动、Enter、删除和重命名都必须从这个投影获得目标，禁止再对原始 `sql_editors.iter().nth(selected)` 操作。

**Step 6: 运行排序测试和 model 测试**

Run: `cargo test --lib console_manager_`

Run: `cargo test --lib model::sql_editor_list`

Expected: PASS。

**Step 7: Commit**

```bash
git add src/app.rs src/model/sql_editor_list.rs
git commit -m "feat(console): define manager filtering and ordering"
```

### Task 3: Move The Leader Shortcut To Space S

**Files:**
- Modify: `src/action.rs:37-72`
- Modify: `src/input/keymap.rs:1261-1295`
- Modify: `src/editor/mod.rs:40-55,1290-1310,7695-7710`
- Modify: `src/app.rs:1324-1380,2274-2280,7695-7710`
- Modify: `src/help.rs:219-249,798-889`
- Test: `src/input/keymap.rs` 的 unit tests
- Test: `src/app.rs` 的 unit tests
- Test: `src/help.rs` 的 unit tests

**Step 1: 修改 keymap 失败测试表达新契约**

将 `global_leader_shortcuts_work_outside_explorer` 中的期望改为：

```rust
assert_leader_shortcut(&results, 's', Action::OpenSqlEditorList);
assert_eq!(leader_action(&results, 'n'), None);
assert_eq!(leader_action(&results, 'e'), None);
```

对 relation transaction pending path 也增加同样断言，确保 `Space s` 在 relation context 中打开 manager。

Editor Normal 模式单独验证：输入 `Space s` 后产生 `EditorEffect::OpenSqlEditorList`；`Space n` 和 `Space e` 不产生 console action。

**Step 2: 运行测试并确认失败**

Run: `cargo test --lib global_leader_shortcuts_work_outside_explorer`

Expected: FAIL，当前 `s` 仍映射 `GotoSqlConsole`，`n`/`e` 仍有绑定。

**Step 3: 修改全局和 editor leader 映射**

将两条输入链统一为：

```rust
(Pending::Leader, KeyCode::Char('s')) => Some(Action::OpenSqlEditorList)
```

以及 editor 内部：

```rust
(PendingBinding::Leader, 's') => self.effects.push(EditorEffect::OpenSqlEditorList)
```

删除 `n`、`e` 对应分支；不要影响 plain `n`、`e` 或 `Ctrl+n`。

**Step 4: 删除废弃的 goto action/effect/update 分支**

删除：

- `Action::GotoSqlConsole`
- `EditorEffect::GotoSqlConsole`
- `App::update(Action::GotoSqlConsole)`
- `execute_help_shortcut` 中的 goto 映射
- `goto_sql_console_activates_the_first_available_sql_tab` 测试

如果 `NewConsole` action 仍由 manager 的 `a` 使用，则保留 `Action::NewConsole` 和 `EditorEffect::NewConsole` 类型本身；只删除 leader binding。若移除 binding 后 `EditorEffect::NewConsole` 没有任何调用方，则一并删除 effect，但保留 app action。

**Step 5: 合并帮助入口**

在 `HelpShortcutId` 删除 `GotoSqlConsole`；把 `OpenSqlEditors` 的 row 改为：

```rust
row!(
    OpenSqlEditors,
    [Explorer, SqlResultsData, SqlOutput, RelationDataBrowse, RelationDdl],
    "Space s",
    "open console manager",
    Leader,
    "s"
),
```

删除原 `NewConsole` 的 `Space n` row；保留 manager 内部的 `a new console` help row，后续 Task 8 添加。

同步修正 `footer_priority`、prefix candidates、shortcut count/order 断言中删除变体造成的匹配和索引变化。

**Step 6: 运行相关测试**

Run: `cargo test --lib input::keymap`

Run: `cargo test --lib help::tests`

Expected: PASS；无废弃 enum 分支或不可达 pattern。

**Step 7: Commit**

```bash
git add src/action.rs src/input/keymap.rs src/editor/mod.rs src/app.rs src/help.rs
git commit -m "feat(console): open manager with space s"
```

### Task 4: Wire Browse, Search, Activation, And Creation

**Files:**
- Modify: `src/action.rs:64-69`
- Modify: `src/input/keymap.rs:422-442`
- Modify: `src/app.rs:2110-2255,6793-6812,6860-6873`
- Test: `tests/keymap.rs`
- Test: `src/app.rs` 的 unit tests

**Step 1: 定义浏览和搜索 action**

用明确 action 替换当前含混的 `SqlEditorListInsert/Backspace`：

```rust
OpenSqlEditorList,
SqlEditorListMove(isize),
SqlEditorListActivate,
SqlEditorListCreate,
SqlEditorListSearchStart,
SqlEditorListInputInsert(char),
SqlEditorListInputBackspace,
SqlEditorListInputDeletePreviousWord,
SqlEditorListInputDeleteToStart,
SqlEditorListInputDelete,
SqlEditorListInputMoveLeft,
SqlEditorListInputMoveRight,
SqlEditorListInputMoveHome,
SqlEditorListInputMoveEnd,
SqlEditorListCancel,
```

`ActivateSqlEditor(Uuid)` 保留为最终 domain action，manager 的 `SqlEditorListActivate` 在 app update 中解析当前 selected ID 后调用相同逻辑。

**Step 2: 写 keymap 模式分发的失败测试**

在 `tests/keymap.rs` 覆盖：

```rust
#[test]
fn console_manager_browse_keys_dispatch_commands() { /* j/k arrows Enter a d r / Esc */ }

#[test]
fn console_manager_search_keys_edit_query_without_triggering_browse_commands() {
    /* Search 模式下 a/d/r -> InputInsert，对应 Ctrl+w/Ctrl+u/Delete/Home/End/Left/Right */
}
```

关键断言：Browse 中普通 `x` 返回 `None`，不会隐式进入过滤；Search 中 `j` 是文本字符而不是移动命令，上下方向键仍可移动结果。这样用户可以搜索包含 `j/k/a/d/r` 的名称。

**Step 3: 运行 keymap 测试并确认失败**

Run: `cargo test --test keymap console_manager_`

Expected: FAIL，新的 action 或 mode 分发尚未实现。

**Step 4: 实现模式感知 keymap**

在 `if let Some(Overlay::SqlEditorList(list))` 分支内先 match `list.mode`：

- Browse：`Enter/j/k/Up/Down/a/d/r///Esc`。
- Search：字符和文本编辑键修改 query；Up/Down 移动；Enter 激活；Esc 返回 Browse。
- Rename：字符和文本编辑键修改 rename input；Enter 提交；Esc 返回 Browse。
- DeleteConfirm：Enter 确认；Esc 返回 Browse。

使用 event modifier guard，避免 `Ctrl+w` 被当作字符 `w`。沿用项目其他 text input 的 Ctrl+w、Ctrl+u 语义。

**Step 5: 写 app reducer 的失败测试**

覆盖：

```rust
#[test]
fn opening_console_manager_selects_the_active_console() { /* ... */ }

#[test]
fn opening_console_manager_from_relation_selects_first_sorted_console() { /* ... */ }

#[test]
fn console_manager_moves_in_visible_sorted_order() { /* ... */ }

#[test]
fn console_manager_reopens_closed_console_with_existing_sql() { /* ... */ }

#[test]
fn console_manager_creates_and_activates_a_unique_console() { /* ... */ }
```

reopen 测试步骤：

1. 创建 `console_1`。
2. 通过 editor API 写入 `select 42;`。
3. 关闭该 tab，断言对应 `ConsoleRecord.open == false`。
4. 打开 manager 并将 selected ID 指向它。
5. dispatch `SqlEditorListActivate`。
6. 断言 tab 重新出现、active/focus 正确、overlay 关闭、`editor_text(id) == "select 42;"`。
7. 断言命令包含 `PersistWorkspace`，且 snapshot 中该 console 为 `open == true`。

**Step 6: 实现 reducer 和统一创建 helper**

`OpenSqlEditorList`：

```rust
let active_console_id = self.active_console_opt().map(|tab| tab.id);
let visible_ids = self.visible_sql_editor_ids("");
let selected_id = active_console_id.or_else(|| visible_ids.first().copied());
self.overlay = Some(Overlay::SqlEditorList(SqlEditorListState::new(selected_id)));
```

移动时先读取 query 并计算 `visible_ids`，再 mutably 借用 overlay 调用 `move_selection`，避免同时借用 `self`。

激活时解析 selected ID，然后调用 `activate_sql_editor(id)`。空结果或无选择时返回 `Vec::new()`。

新建时不要复制 `Action::NewConsole` 的初始化代码。提取一个返回 ID 并负责创建、激活、focus 和持久化的 helper，例如：

```rust
fn create_and_activate_sql_editor(&mut self) -> Vec<Command>
```

让 `Action::NewConsole` 和 manager 的 create action 共用它；manager create 成功后关闭 overlay。

**Step 7: 修正自动命名判重**

`next_console_name` 检查全部 `sql_editors` 的 trim 后名称，并使用 `eq_ignore_ascii_case` 防止重命名后生成重复 `console_N`。保留现有“不重用序号”的测试，并增加用户已命名为 `Console_3` 时自动跳到 `console_4` 的测试。

**Step 8: 运行 reducer 和 keymap 测试**

Run: `cargo test --lib console_manager_`

Run: `cargo test --test keymap console_manager_`

Expected: PASS。

**Step 9: Commit**

```bash
git add src/action.rs src/input/keymap.rs src/app.rs tests/keymap.rs
git commit -m "feat(console): browse search and activate consoles"
```

### Task 5: Add Rename With Validation And Persistence

**Files:**
- Modify: `src/action.rs`
- Modify: `src/model/sql_editor_list.rs`
- Modify: `src/input/keymap.rs`
- Modify: `src/app.rs`
- Test: `src/app.rs` 的 unit tests
- Test: `tests/keymap.rs`

**Step 1: 增加 rename action 和失败测试**

增加：

```rust
SqlEditorListRenameStart,
SqlEditorListRenameCommit,
```

文本编辑继续复用 Task 4 的 `SqlEditorListInput*` actions，根据当前 mode 修改 search query 或 rename input。

写测试覆盖：

```rust
#[test]
fn console_manager_starts_rename_with_the_selected_name() { /* input prefilled; cursor at end */ }

#[test]
fn console_manager_renames_open_console_in_record_and_tab() { /* both names changed */ }

#[test]
fn console_manager_renames_closed_console_and_persists_it() { /* record + snapshot changed */ }

#[test]
fn console_manager_rejects_blank_and_duplicate_names() { /* remains Rename and sets error */ }

#[test]
fn console_manager_keeps_same_uuid_selected_after_rename_reorders_list() { /* ... */ }
```

**Step 2: 运行测试并确认失败**

Run: `cargo test --lib console_manager_rename`

Expected: FAIL，rename reducer 尚不存在。

**Step 3: 实现 rename mode 初始化**

从当前 selected ID 查 `ConsoleRecord`，用 `TextInput::from(record.name.clone())` 构造：

```rust
SqlEditorListMode::Rename {
    console_id,
    input,
    error: None,
}
```

目标不存在时保持 Browse 且 no-op。

**Step 4: 实现提交校验和原子更新**

提交前先获取不可变值，避免 borrow conflict：

```rust
let name = input.value().trim().to_owned();
```

校验：

- `name.is_empty()` -> `Name is required`。
- 其他 `record.id != console_id` 且 `record.name.eq_ignore_ascii_case(&name)` -> `Name already exists`。

成功时：

```rust
record.name = name.clone();
if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id() == console_id)
    .and_then(WorkspaceTab::as_console_mut)
{
    tab.name = name;
}
```

然后 mode 返回 Browse、保留 `selected_id = Some(console_id)`、按新排序 reconcile，并返回单个 `PersistWorkspace` command。

**Step 5: 验证 Esc 和文本编辑键**

Run: `cargo test --test keymap console_manager_rename`

Expected: PASS；`Esc` 不修改名称，Enter 只在合法时提交。

**Step 6: 运行 app 测试**

Run: `cargo test --lib console_manager_rename`

Expected: PASS。

**Step 7: Commit**

```bash
git add src/action.rs src/model/sql_editor_list.rs src/input/keymap.rs src/app.rs tests/keymap.rs
git commit -m "feat(console): rename consoles from manager"
```

### Task 6: Add Transaction-Safe Delete Inside The Manager

**Files:**
- Modify: `src/action.rs`
- Modify: `src/model/transaction.rs:54-65`
- Modify: `src/model/sql_editor_list.rs`
- Modify: `src/input/keymap.rs`
- Modify: `src/app.rs:2186-2213,6383,6630-6660,6688-6858`
- Modify: `src/model/tab.rs:231-234`
- Test: `src/app.rs` 的 unit tests
- Test: `tests/keymap.rs`

**Step 1: 将删除请求改为显式 ID**

增加 manager actions：

```rust
SqlEditorListDeleteRequest,
SqlEditorListDeleteConfirm,
SqlEditorListDeleteCancel,
```

将现有 active-only domain action 改为：

```rust
RequestDeleteConsole(Uuid)
```

原 `Space x` 若保留，则在 keymap 无法直接取 ID 时使用独立 `RequestDeleteActiveConsole` adapter；更小的方案是保留现有 active action，并让它内部调用统一 helper：

```rust
fn request_delete_console(&mut self, id: Uuid, return_to_manager: bool) -> Vec<Command>
```

manager 必须始终传 selected ID，不能临时切换 active tab 再删除。

**Step 2: 写普通删除的失败测试**

覆盖：

```rust
#[test]
fn console_manager_delete_requires_confirmation() { /* d only enters DeleteConfirm */ }

#[test]
fn console_manager_can_delete_a_closed_console() {
    /* record removed, no unrelated tab changed, DeleteSqlFile(id), PersistWorkspace */
}

#[test]
fn cancelling_console_manager_delete_returns_to_browse() { /* same selection remains */ }

#[test]
fn deleting_last_console_creates_a_new_default_console() { /* one open console named console */ }
```

**Step 3: 运行测试并确认失败**

Run: `cargo test --lib console_manager_delete`

Expected: FAIL，manager 尚无删除流程，且默认 `console` 仍被名称保护。

**Step 4: 移除名称身份保护**

删除以下提前返回：

- `request_close_tab` 中 `ConsoleTab::is_default()` 判断。
- `delete_console` 中 `ConsoleTab::is_default()` 判断。

若 `ConsoleTab::is_default()` 无其他调用方，删除该方法。新增回归测试：名为 `console` 的唯一 console 可以被 close/delete，随后 replacement 规则确保 workspace 仍有可用 console。

**Step 5: 在 manager 内实现确认状态**

`SqlEditorListDeleteRequest`：

1. 解析 selected UUID。
2. 如果 `transaction_needs_exit(id)` 为 false，设置 `SqlEditorListMode::DeleteConfirm { console_id: id }`。
3. 如果为 true，先用 `self.overlay.take()` 移除当前 manager，再进入现有 transaction prompt，并记录删除完成后应返回 manager，而不是丢失上下文。必须先清空 overlay，因为 `show_next_deferred()` 在 `self.overlay.is_some()` 时会直接返回。

推荐将 deferred intent 扩展为具名字段：

```rust
DeferredIntent::DeleteConsole {
    id: Uuid,
    return_to_manager: bool,
}
```

更新所有 pattern。事务完成后：

- `return_to_manager == false`：沿用现有 `Overlay::DeleteConsole`。
- `return_to_manager == true`：重建 `Overlay::SqlEditorList`，选中同一 ID，并进入 `DeleteConfirm`。

同时修改 `resolve_transaction_exit` 的 `TransactionExitChoice::Cancel` 分支。当前实现清除 deferred queue 后直接返回且 overlay 已被 `take()`；对于 `DeleteConsole { return_to_manager: true, .. }`，取消事务退出后必须重建 Browse 模式 manager 并选中原 ID。这里允许清空之前的 search query，第一版不需要把完整 UI 状态塞入 `DeferredIntent`。

不要把完整 overlay clone 放入 `DeferredIntent`，避免 transaction model 依赖 UI 状态。

**Step 6: 写 manual transaction 删除的失败测试**

测试已打开 console 处于需要退出的 manual transaction 时：

1. manager 选中该 console。
2. dispatch delete request。
3. 断言显示 `TransactionExitConfirm`，且 deferred intent 携带 `return_to_manager: true`。
4. 完成 commit/rollback/取消交易选择。
5. 断言回到 manager 的 `DeleteConfirm`。
6. 确认删除后才出现 `DeleteSqlFile(id)`。

同时验证发起 deferred delete 后 transaction overlay 立即可见；取消 transaction exit 不删除 console，并恢复 manager Browse 状态且仍选中原 console。

**Step 7: 实现删除后的选择恢复**

确认删除前记录可见 ID 顺序和被删项位置。删除后重新计算排序结果：

- 优先选择原位置现在的元素。
- 原位置越界则选择最后一项。
- 如果删除触发 replacement console，则选择 replacement ID。

删除函数继续统一完成：关闭 editor session、移除 record/tab、创建必要 replacement、持久化和 `DeleteSqlFile`。manager reducer 不复制数据删除逻辑。

**Step 8: 运行删除和 transaction 测试**

Run: `cargo test --lib console_manager_delete`

Run: `cargo test --lib transaction_exit`

Run: `cargo test --test keymap console_manager_delete`

Expected: PASS。

**Step 9: Commit**

```bash
git add src/action.rs src/model/transaction.rs src/model/sql_editor_list.rs src/model/tab.rs src/input/keymap.rs src/app.rs tests/keymap.rs
git commit -m "feat(console): delete consoles safely from manager"
```

### Task 7: Render The Console Manager And Editable Modes

**Files:**
- Modify: `src/ui/mod.rs:740-790,858-910,3101-3149`
- Test: `tests/ui_render.rs`

**Step 1: 写浏览模式 UI 失败测试**

增加 fixture，包含 default/open/closed consoles，打开 manager 后 render：

```rust
#[test]
fn console_manager_renders_sorted_open_and_closed_consoles() {
    let output = render(&app, 100, 30);

    assert_order(&output, &["console", "alpha", "Beta", "charlie"]);
    assert!(output.contains("OPEN"));
    assert!(output.contains("CLOSED"));
    assert!(output.contains("a new"));
    assert!(output.contains("d delete"));
    assert!(output.contains("r rename"));
    assert!(output.contains("/ search"));
}
```

不要只断言名称存在；必须断言显示顺序与 `App::visible_sql_editors` 一致。

**Step 2: 写 compact/empty/search UI 失败测试**

覆盖：

- 高度不足时列表被 viewport 截断，但 footer 和边框仍显示。
- 搜索无结果显示 `No matching consoles`。
- Search 模式显示 query 和 bar cursor。
- Rename 模式显示原名称、输入光标和 validation error。
- DeleteConfirm 显示将永久删除 SQL 文件的明确文案和 console 名称。

**Step 3: 运行测试并确认失败**

Run: `cargo test --test ui_render console_manager_`

Expected: FAIL，当前 UI 仍按原始顺序并显示旧 SQL editor list footer。

**Step 4: 重写 SqlEditorList overlay 渲染分支**

渲染数据只调用：

```rust
let records = app.visible_sql_editors(list.visible_query());
```

每行建议格式：

```rust
let selected = (list.selected_id == Some(record.id)).then_some("> ").unwrap_or("  ");
let status = if record.open { "OPEN" } else { "CLOSED" };
```

状态列右对齐或至少使用固定宽度，名称过长时使用项目现有 cell-safe 截断 helper；不要用字节切片。选中行应用 `theme.selection`，OPEN 使用 `theme.success` 或 `theme.action`，CLOSED 使用 `theme.muted`。

popup 高度按 `visible_count + header + footer` clamp，宽度维持当前 72 左右，并确保最小终端下 `centered` 不溢出。

**Step 5: 渲染 Search/Rename/DeleteConfirm 子模式**

- Browse：标题 `CONSOLES`，显示命令 footer。
- Search：header 中显示 `/` query；使用 `render_text_input` 或相同 cell-safe helper设置 bar cursor。
- Rename：显示 `Rename <old-name>` 和 `Name:` 输入；error 用 `theme.error`。
- DeleteConfirm：保留 manager 外框，显示 `Permanently delete '<name>' and its saved SQL file?`，footer `Enter delete  Esc cancel`。

如果直接复用 `render_text_input` 会引入额外 label 布局，可提取一个很小的单行输入 render helper，但禁止重新实现 cursor cell width 计算。

**Step 6: 修正 overlay 动画/尺寸分类**

检查 `src/ui/mod.rs:774` 对 `Overlay::SqlEditorList` 的 animation row/area 估算。子模式高度变化时使用统一最大值或按 mode 返回，不允许首次打开时动画裁掉 footer。

**Step 7: 运行 UI tests**

Run: `cargo test --test ui_render console_manager_`

Expected: PASS。

**Step 8: Commit**

```bash
git add src/ui/mod.rs tests/ui_render.rs
git commit -m "feat(console): render console manager workflows"
```

### Task 8: Update Contextual Help

**Files:**
- Modify: `src/help.rs:219-530,1835-1862,2350-2390,2600-2620`
- Test: `src/help.rs` 的 unit tests
- Test: `tests/ui_render.rs`

**Step 1: 替换 SQL editor list help IDs**

将旧四项：

```text
SqlEditorListEdit
SqlEditorListMove
SqlEditorListActivate
SqlEditorListClose
```

扩展为用户可见操作：

```text
ConsoleManagerMove
ConsoleManagerActivate
ConsoleManagerCreate
ConsoleManagerDelete
ConsoleManagerRename
ConsoleManagerSearch
ConsoleManagerClose
ConsoleManagerEdit
ConsoleManagerCommit
ConsoleManagerCancel
```

可以保留 `ShortcutContext::SqlEditorList`，但 `shortcut_context` 应根据 `SqlEditorListMode` 返回更细的 context，或在同一 context 下使用 capability/mode filter。推荐增加：

```text
ConsoleManager
ConsoleManagerSearch
ConsoleManagerRename
ConsoleManagerDeleteConfirm
```

这样 footer 不会在编辑名称时仍提示 `d delete`。

**Step 2: 写 context 和 shortcuts 失败测试**

覆盖：

```rust
#[test]
fn console_manager_help_changes_with_mode() { /* browse/search/rename/delete */ }

#[test]
fn leader_help_lists_only_space_s_for_console_manager() {
    /* no Space n, no Space e, no go-to-first-console */
}
```

**Step 3: 运行测试并确认失败**

Run: `cargo test --lib help::tests::console_manager_`

Expected: FAIL，新 context/rows 尚未定义。

**Step 4: 增加各 mode 的 rows**

至少表达：

```text
j/k or Up/Down  move selection
Enter           open or focus
a               new console
d               delete console
r               rename console
/               search consoles
Esc             close/cancel
type            edit search/name
Ctrl+w          delete previous word
Ctrl+u          clear to start
```

调整 `footer_priority`，优先显示当前 mode 最关键的动作，并保证窄终端不会只剩低价值提示。

**Step 5: 更新帮助搜索和 footer 渲染断言**

在 `tests/ui_render.rs` 验证 manager browse footer 包含 `a/d/r`；Search/Rename footer 不错误显示 browse 命令。

**Step 6: 运行帮助和 UI 测试**

Run: `cargo test --lib help::tests`

Run: `cargo test --test ui_render console_manager_`

Expected: PASS。

**Step 7: Commit**

```bash
git add src/help.rs tests/ui_render.rs
git commit -m "docs(console): update manager shortcuts and hints"
```

### Task 9: Verify Persistence And Regression Boundaries

**Files:**
- Modify only if tests expose defects: `src/app.rs`, `src/persistence/workspace.rs`
- Test: `tests/workspace_persistence.rs`
- Test: `tests/workspace_tabs.rs`
- Test: `src/app.rs` 的 unit tests

**Step 1: 写关闭 console 跨 snapshot 恢复测试**

在 workspace persistence 测试中验证：

1. workspace 含 open 和 closed console。
2. closed console 的 SQL 为非空。
3. snapshot save/load 后 `open == false` 和 SQL 文本仍保留。
4. `App::restore_workspace` 后 manager 能看到该 console。
5. 激活后 tab 重新打开且 SQL 文本不变。

如果跨 crate API 无法直接驱动 manager，可把步骤 4-5 放在 `src/app.rs` unit test，并让 integration test 只负责 manifest round-trip。

**Step 2: 验证重命名 round-trip**

对 open 和 closed console 各验证一次：重命名后的 `PersistedConsole.name` 正确，恢复后 `ConsoleRecord` 与打开 tab 标题一致。

**Step 3: 验证删除只影响目标 SQL 文件**

沿用 `tests/workspace_persistence.rs` 的幂等删除覆盖，新增 manager reducer 命令断言：

```rust
assert!(commands.iter().any(|command| matches!(command, Command::DeleteSqlFile(id) if *id == target)));
assert!(!commands.iter().any(|command| matches!(command, Command::DeleteSqlFile(id) if *id == survivor)));
```

**Step 4: 运行 persistence suites**

Run: `cargo test --test workspace_persistence --test workspace_tabs`

Expected: PASS，无 workspace schema migration。

**Step 5: 运行 app console 生命周期回归**

Run: `cargo test --lib console`

Expected: PASS，包括命名、新建、关闭、删除、重开和事务测试。

**Step 6: Commit**

```bash
git add tests/workspace_persistence.rs tests/workspace_tabs.rs src/app.rs src/persistence/workspace.rs
git commit -m "test(console): cover manager persistence lifecycle"
```

只 stage 实际修改的文件；如果 `src/persistence/workspace.rs` 无需改动，不要包含它。

### Task 10: Final Quality Gate

**Files:**
- Verify: all changed Rust and documentation files

**Step 1: 格式化代码**

Run: `cargo fmt --all`

Expected: 命令成功，仅格式化本次触及的 Rust 文件或其他已有未格式化文件；提交前检查 diff，不能覆盖无关用户改动。

**Step 2: 运行定向测试**

Run: `cargo test --lib model::sql_editor_list`

Run: `cargo test --lib console_manager_`

Run: `cargo test --test keymap console_manager_`

Run: `cargo test --test ui_render console_manager_`

Run: `cargo test --test workspace_persistence --test workspace_tabs`

Expected: 全部 PASS。

**Step 3: 运行受影响完整 suites**

Run: `cargo test --lib`

Run: `cargo test --test keymap --test ui_render --test workspace_persistence --test workspace_tabs`

Expected: 全部 PASS，无 ignored test 数量异常变化。

**Step 4: 运行静态检查**

Run: `cargo fmt --all -- --check`

Run: `cargo clippy --all-targets --all-features -- -D warnings`

Expected: 两条命令都以 0 退出，无 unused action/effect/help ID、borrow workaround 或 clippy warning。

**Step 5: 手工验收关键流程**

Run: `cargo run`

逐项验证：

1. 创建多个 console，关闭其中一个，`Space s` 中仍能看到 CLOSED 并 Enter 恢复内容。
2. `Space n` 和 `Space e` 不再执行 console 操作。
3. 排序是 `console`、其他 OPEN、CLOSED，同组名称正序。
4. Browse 的 `a/d/r` 不进入搜索；`/` 后可以输入包含 `a/d/r/j/k` 的搜索词。
5. rename 后 tab、列表和重启后的名称一致；空名和重复名有明确错误。
6. 删除 closed console 要确认并删除 SQL 文件；Esc 返回列表。
7. 删除 manual transaction console 先处理 transaction，再出现 manager 删除确认。
8. 删除最后一个 console 后仍有可用的 `console`。
9. relation/dashboard tab 上 `Space s` 同样可用。
10. 80x24 和较小终端中 overlay/footer 不溢出，光标仅在 Search/Rename 模式出现。

**Step 6: 检查最终 diff**

Run: `git status --short`

Run: `git diff --check`

Run: `git diff -- src/action.rs src/app.rs src/editor/mod.rs src/help.rs src/input/keymap.rs src/model/sql_editor_list.rs src/model/tab.rs src/model/transaction.rs src/ui/mod.rs tests/keymap.rs tests/ui_render.rs tests/workspace_persistence.rs tests/workspace_tabs.rs docs/plans/2026-09-02-console-manager-implementation.md`

Expected: 仅包含 Console Manager 相关改动；`git diff --check` 无 whitespace error；不回滚或覆盖工作区内其他人的改动。

**Step 7: Final commit**

仅在前面任务没有逐步提交或存在最后的测试/格式修正时提交：

```bash
git add <only-console-manager-files>
git commit -m "feat(console): add unified console manager"
```

禁止 amend 已有提交；如果用户没有明确要求执行 commit，则跳过所有 commit step，只保留对应 staging boundary 作为实现分组参考。

## Acceptance Criteria

- `Space s` 在所有支持 workspace 的主视图打开 Console Manager。
- `Space n` 和 `Space e` 不再绑定 console 行为。
- manager 展示当前 workspace 的全部 console，包括 closed console。
- 排序严格满足：精确 `console` > OPEN > CLOSED；同组名称正序且结果确定。
- `j/k`、上下方向键和 Enter 使用屏幕显示的同一顺序与同一 UUID。
- Enter 能 focus open console，也能 reopen closed console，并保留 SQL 内容。
- `a` 新建并激活唯一命名的 console。
- `r` 支持完整文本编辑、空名/重复名校验，并同步 record、tab 和 persisted snapshot。
- `d` 对 open/closed console 都有效，有明确确认，并保留 manual transaction 安全流程。
- `console` 名称不再是关闭/删除权限判断；最后一个 console 被删除后自动补建。
- 搜索与命令键不冲突，Esc 分层返回行为一致。
- workspace persistence schema 不变，closed/renamed console 可跨重启恢复。
- 定向测试、受影响完整 suites、fmt 和 clippy 全部通过。
