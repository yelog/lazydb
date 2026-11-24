# Issue #54 · SQLite 新建表约束编辑与执行计划

**目标**　在 SQLite 的 `NEW Table` 表单中，可以用鼠标或键盘添加约束，在 `Review SQL` 核对它们，并通过 `Apply` 创建由 SQLite 实际执行约束的表。

**实现路径**　表编辑器负责暂存字段和管理焦点。SQLite 适配器负责检查草稿、引用标识符并生成 SQL。运行时继续用连接配置检查执行目标。验收从同一张表单开始，最后用新的 SQLite 连接检查数据库。

## 1. 固定回归场景

- 在 `tests/app_flow.rs` 使用临时 SQLite 文件连接真实 `Runtime`，从 Explorer 的 Tables 分组打开建表表单。输入表名和一列，在渲染出的表单中点击添加唯一约束。
- 确认约束并进入 `Review SQL`。断言预览包含 `UNIQUE`，此时数据库中还没有新表。执行 `Apply` 后重新打开数据库，验证第一条记录能写入，而第二条相同记录触发唯一约束错误。
- 在 `tests/mouse.rs`、编辑器测试及 `tests/sqlite_adapter.rs` 单独覆盖布局、输入和数据库语义，以便在端到端用例失败时定位出错环节。

## 2. 隔离约束编辑

- 修改 `src/ui/catalog_editor.rs`，移除绘制在 `GENERAL` 和 `COLUMNS` 上的内联约束字段。参照列详情，将新增和编辑约束放入独立弹窗。普通终端和 `80×20` 终端都要能找到四种新增操作，并能访问外键的最后一个字段。
- 在 `src/model/catalog_editor.rs`、`src/app.rs` 和 `src/input` 对齐编辑会话与输入。键盘和鼠标操作同一份草稿。确认后返回表单，取消新增或修改时恢复打开弹窗前的约束。弹窗打开期间点击背景不能修改列或触发提交。
- 验证同表多个约束可区分、重新编辑和移除。回测 `Add Column`、表单取消、`Review SQL` 以及已有 DuckDB 建表约束工作流。

## 3. 让 SQLite 执行预览中的约束

- 修改 `src/db/sqlite.rs`，把结构化表约束按草稿顺序追加到列定义之后。覆盖命名约束、复合主键、复合唯一键、检查表达式，以及同一 schema 中的外键和引用动作。表名、约束名和列名采用 SQLite 标识符引用规则。
- 从所选 Tables 分组取得目标数据库和 schema。让预览中的建表对象和 `Apply` 使用的执行目标一致，并保留连接配置的范围校验。对于 SQLite 无法表示的外键配置，在预览阶段给出错误。
- 在 `src/runtime.rs` 验证 schema 后复用此次连接的 SQLite 会话，让 `ATTACH` 创建的别名和内存数据库在执行时仍可见。用两个连接代次和一个配置范围之外的 schema 验证隔离。
- 在临时数据库里检查普通表、约束表和显式 schema。验证重复值、违反检查条件的值以及孤立的外键值会被 SQLite 拒绝。

## 4. 集成验收

- 运行 `cargo test --locked --test app_flow sqlite_table_editor_applies_reviewed_unique_constraint`，确认真实运行时路径通过。
- 运行 `cargo test --locked --test sqlite_adapter --test catalog_editor_state --test catalog_editor_reducer --test keymap --test mouse --test ui_render`。对已支持结构化约束的 DuckDB 运行现有建表约束用例。
- 构建本地二进制，在隔离配置和临时 SQLite 文件上用真实终端实测普通窗口与 `80×20` 窗口。核对鼠标命中、字段确认和取消、预览、执行与重复值校验。
- 运行 `cargo fmt --all -- --check` 和 `git diff --check`。在 `docs/keybindings.md` 记录实际交互，并在本计划末尾记录验证结果。

## 5. 验证结果（2026-11-24）

环境为 macOS aarch64，使用隔离的 `CARGO_TARGET_DIR` 和本地 DuckDB 动态库；SQLite 用例使用真实临时数据库文件。

- `cargo test --locked --test app_flow`：39 passed。`sqlite_table_editor_applies_reviewed_unique_constraint` 在 `80×20` 下通过渲染后的 `Add UNIQUE` 命中区添加约束，预览阶段确认表尚未创建，`Apply` 后重复值触发 SQLite 唯一约束错误。
- `cargo test --locked --test sqlite_adapter`：36 passed；`--test catalog_editor_state` 75、`--test catalog_editor_reducer` 54、`--test keymap` 175、`--test mouse` 75、`--test ui_render` 253 passed（1 ignored）。
- `cargo test --locked --all-targets --all-features --no-fail-fast -j 6`：除 `update_http::lowercase_http_proxy_environment_routes_download_through_proxy` 一次本地代理 socket 偶发 `WouldBlock` 外，其余目标全部通过；单独重跑整个 `--test update_http` 为 4 passed、1 ignored（需要真实系统代理的用例保持 ignored）。
- 真实终端 PTY 验收通过：`160×45` 和 `80×20` 下键盘、鼠标均可添加、编辑、取消和移除约束；模拟取消的约束未进入草稿；预览阶段数据库不变。实际执行 SQL 为：

  ```sql
  CREATE TABLE "child" (
    "id" text, "code" text, "parent_id" text,
    CONSTRAINT "child_constraint_1" PRIMARY KEY ("id"),
    CONSTRAINT "child_constraint_2" UNIQUE ("code"),
    CONSTRAINT "child_constraint_3" CHECK (length(code) > 0),
    CONSTRAINT "child_constraint_4" FOREIGN KEY ("parent_id")
      REFERENCES "parent" ("id") ON UPDATE NO ACTION ON DELETE CASCADE
  )
  ```

  执行后重复 `id`、重复 `code`、空 `code`、孤立外键分别被 SQLite 拒绝，删除父行触发 `CASCADE`；预览阶段表不存在。
- `cargo fmt --all -- --check` 与 `git diff --check` 通过。
- 未连接真实外部 PostgreSQL/DuckDB 服务；DuckDB 使用本地运行时测试，PostgreSQL 使用现有适配器测试，SQLite 使用真实临时文件。
