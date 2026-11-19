# Redis Keys Input Navigation Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.
> 本次用户已要求分析并实施；在当前工作区直接执行，不另建分支，不提交。

**Goal:** 点击 Redis Keys 的内容或空白处即可聚焦 Keys，Keys 中的 Tab 焦点循环与配置的工作区 Tab 切换序列正确执行。

**Architecture:** 保留外层 `App.focus` 与内层 `RedisBrowserTab.focus`。Keys 注册面板级点击目标，行级目标保留更高优先级；显式选择和展开动作复用已有 `RedisFocusPane`，后台 Tab 的数据操作不改变当前焦点。Keys 普通浏览与已确认搜索状态在自己的输入分支中接入现有前缀解析，搜索编辑继续优先处理文字。

**Tech Stack:** Rust、crossterm、ratatui TestBackend、现有 Action / App / Keymap。

---

### Task 1: Reproduce mouse and keyboard failures

**Files:**
- Create: `tests/redis_navigation.rs`

1. 用真实 UI 渲染生成点击区域，覆盖从 Explorer 或 Value 点击 Keys 叶子、分组、展开按钮、标题和空白。
2. 通过 `map_mouse -> App::update` 验证两层焦点，保证单击不打开 Value、双击仍显式打开 Value、后台选择不抢焦点。
3. 用真实 KeyEvent 验证 `gt`、`gT`、`]t`、`[t` 的映射与 Tab 索引变化，包括已确认搜索、自定义及禁用绑定。
4. 验证搜索编辑优先处理文字，Tab / Shift-Tab 仍在 Explorer、Keys、Value 间循环。

Run: `cargo test --test redis_navigation`

Expected before implementation: 鼠标聚焦与 Keys 序列测试失败；已有搜索输入和焦点循环行为保持可复现。

### Task 2: Repair Keys mouse focus

**Files:**
- Modify: `src/ui/mod.rs`
- Modify: `src/ui/redis_browser.rs`
- Modify: `src/input/mouse.rs`
- Modify: `src/app.rs`

1. 增加与 Value 对称的 `RedisKeysFocus` 命中目标，在注册行和按钮前覆盖整个 Keys 面板。
2. 将背景点击映射为现有 `RedisFocusPane(Keys)`，同步外层与内层焦点。
3. 从现有焦点动作抽出轻量函数；显式选择或展开活动 Redis Tab 的节点后复用该函数，避免递归进入整个 App 更新器。
4. 不改变通用 `select_redis_key` 的数据更新语义；后台 Tab、非法 Tab、单击与双击分别保持既有边界。

### Task 3: Repair Keys tab sequences

**Files:**
- Modify: `src/input/keymap.rs`
- Test: `tests/redis_navigation.rs`

1. 在 Keys 搜索编辑处理之后识别配置中仍存在的 `g`、`[`、`]` Tab 切换前缀。
2. 使用既有 Pending / map_pending 完成序列，不增加新的 Tab 切换状态或硬编码动作映射。
3. 保留搜索文字、自定义配置、Explorer 作用域、Value 编辑器输入和 Tab 焦点循环的行为。

### Task 4: Validate and document

**Files:**
- Modify: `docs/keybindings.md`

Run:

```sh
cargo test --test redis_navigation
cargo test --lib input::keymap::tests
cargo test --lib help::tests
cargo test --test redis_help --test redis_browser_tabs --test mouse --test keymap --test help_omni_panel
cargo fmt --all -- --check
git diff --check
```

Expected: 定向用例和现有交互回归通过；保留当前工作区已有帮助面板改动。

### Execution results

- 已在修改前复现：Keys 行点击没有切换焦点；默认及自定义 Tab 序列被吞掉。搜索文字、Tab 循环和叶子双击的基线测试通过。
- 焦点更新共用轻量函数，不递归分发 `App::update`；后台节点操作保留活动 Tab 和焦点。
- 新增交互回归：7 passed。
- Keymap / Help 单元测试：50 + 50 passed。
- Help 面板、Keymap、Mouse、Redis Browser Tabs、Redis Help 集成测试：8 + 172 + 73 + 18 + 6 passed。
- `cargo fmt --all -- --check`、`git diff --check` 通过。
- 验证使用 TestBackend 与内存状态，不依赖真实 Redis / Valkey 服务。
