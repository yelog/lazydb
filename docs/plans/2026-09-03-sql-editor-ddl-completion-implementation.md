# SQL Editor DDL Completion Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 为 PostgreSQL、MySQL、SQL Server 和 SQLite 的 SQL Editor 增加容错、方言感知的 DDL 关键字、Catalog 对象、数据类型、列及约束候选，同时不回归现有 DML/查询补全。

**Architecture:** 保留 `src/sql/completion.rs` 当前面向不完整 SQL 的 tolerant token scan，不把完整 AST 解析作为补全前提；将扁平的 `Context` 扩展为结构化 DDL 语法位置，并由该位置统一决定关键字、Catalog kind、数据类型、排序和 lazy-loading 依赖。完整 SQL 仍可继续由 `sqlparser` 服务于高亮、风险分析等功能，但 DDL completion 的主路径必须能处理 `ALTER TABLE |`、`CREATE TABLE t (id |` 这类必然不完整的文本。

**Tech Stack:** Rust 2024、现有 `CompletionIndex`/`CatalogEntry` 模型、现有 tolerant completion tokenizer、`sqlparser 0.62`（仅复用方言/关键字知识时使用，不依赖完整 AST）、Cargo integration tests、rustfmt、Clippy

---

## 成功标准

- 输入 `cre` 时，首选候选为 `CREATE`。
- 输入 `CREATE ` 时，只出现当前方言支持的 DDL 对象类型。
- 输入 `ALTER TABLE us`、`DROP TABLE us`、`TRUNCATE TABLE us` 时，只出现匹配的 table 候选。
- 输入 `DROP VIEW`、`DROP INDEX`、`DROP TRIGGER`、`DROP SEQUENCE`、`DROP TYPE` 时，只出现目标类型允许的 Catalog 对象。
- 输入 `CREATE INDEX ix ON us` 时出现 table 候选。
- 输入 `CREATE TABLE t (id ` 时出现当前方言的数据类型候选。
- 输入 `CREATE TABLE t (id INTEGER ` 时出现列约束候选。
- 输入 `ALTER TABLE users DROP COLUMN ` 时只出现 `users` 的列。
- 输入 `ALTER TABLE users DROP CONSTRAINT ` 时只出现 `users` 的约束。
- 输入 `REFERENCES users (` 时按需加载并提示 `users` 的列。
- DDL 中的字符串、注释、引用标识符和名为 `create`/`update` 的对象不会误切换语法上下文。
- 现有 32 个 SQL completion 测试及整个项目测试套件继续通过。

## 范围边界

### 第一阶段必须交付

- 顶层 `CREATE`、`ALTER`、`DROP`、`TRUNCATE`。
- DDL object-kind 关键字。
- 已存在 database/schema/table/view/materialized view/index/trigger/sequence/type/function/procedure 的 Catalog 候选。
- `CREATE INDEX ... ON <table>`。
- `CREATE VIEW ... AS SELECT ...` 查询部分回到现有 query completion。
- 方言过滤、候选图标、自动触发一致性和安全的插入后缀。

### 第二阶段必须交付

- `CREATE TABLE` 的数据类型、列约束和表约束。
- `ALTER TABLE` action、列、约束、索引候选。
- `REFERENCES` 表和列候选。
- DDL relation children lazy loading。

### 本轮明确不做

- 不实现任意数据库扩展的完整 DDL grammar。
- 不实现 server-version-aware 关键字过滤。
- 不引入 SQL language server 或额外运行时进程。
- 不实现带 tab-stop 的 snippet 引擎。
- 不推断尚未执行的 `CREATE TABLE` 中前面新定义列的语义类型。
- 不为 CTE、临时表或动态 SQL 构造虚拟 Catalog。
- 不把 `Parser::parse_sql` 的错误文本作为候选 API。

## 设计约束

- 保留 `sql::complete` 的现有公开签名和返回类型，避免无必要地破坏调用方。
- 保留 UTF-8 byte offset、`TextRange` 替换、标识符 quoting、terminal sanitization 和最多 10 条候选的行为。
- DDL 和 DML 必须共用 current statement、identifier extraction、qualified path resolution 和 ranking 基础设施。
- 所有上下文必须使用 allowlist；不能通过“默认允许所有 CatalogKind”兜底。
- 方言差异集中在静态规则函数中，不散落到 UI 或 `App` reducer。
- 新增的 Catalog kind 只能在语法位置允许时进入候选，不能污染 `SELECT` projection、predicate 和 `FROM`。
- Catalog children 未加载时允许先展示关键字；异步加载完成后沿用现有 completion generation 机制重算对象候选。
- 以下 commit 步骤只是逻辑检查点；除非用户明确要求，不执行 `git commit`。

## 目标内部模型

实现过程中允许按测试逐步引入，最终内部模型应接近：

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Context {
    Statement,
    Insert,
    Relation,
    Expression(ExpressionContext),
    Qualifier,
    Routine,
    Ddl(DdlContext),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DdlContext {
    CreateObjectKind,
    AlterObjectKind,
    DropObjectKind,
    ExistingObject(DdlObjectTarget),
    CreateIndexTarget,
    CreateTableElement,
    ColumnType,
    ColumnConstraint,
    TableConstraint,
    AlterTableAction,
    ExistingColumn,
    ExistingConstraint,
    ExistingIndex,
    ReferenceRelation,
    ReferenceColumn,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DdlObjectTarget {
    Database,
    Schema,
    Table,
    View,
    MaterializedView,
    Index,
    Trigger,
    Sequence,
    Type,
    Function,
    Procedure,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CompletionDependencies {
    pub relation_children: Vec<CatalogId>,
}
```

`DdlContext` 只表达“游标所在位置”，具体候选由独立 helper 映射：

```rust
fn keywords(context: Context, dialect: SqlDialect, projection_complete: bool)
    -> &'static [&'static str];

fn catalog_kinds(context: Context, dialect: SqlDialect)
    -> &'static [CompletionKind];

fn data_types(dialect: SqlDialect) -> &'static [&'static str];
```

不要让 `context_at` 同时构造候选；上下文识别和候选生成必须可以分别测试。

## Task 1: 固化 DDL 缺失基线和第一阶段验收矩阵

**Files:**
- Modify: `tests/sql_completion.rs`
- Test: `tests/sql_completion.rs`

**Step 1: 增加测试辅助函数**

在现有 fixture 附近增加只提取候选 label/kind 的辅助函数，避免后续测试重复遍历：

```rust
fn labels(candidates: &[lazydb::sql::CompletionCandidate]) -> Vec<&str> {
    candidates
        .iter()
        .map(|candidate| candidate.label.as_str())
        .collect()
}

fn has_candidate(
    candidates: &[lazydb::sql::CompletionCandidate],
    kind: CompletionKind,
    label: &str,
) -> bool {
    candidates
        .iter()
        .any(|candidate| candidate.kind == kind && candidate.label == label)
}
```

如果该辅助函数只减少一两处重复，则不要添加；直接在测试内断言，遵守 YAGNI。

**Step 2: 写顶层 DDL 关键字失败测试**

增加：

```rust
#[test]
fn statement_completion_offers_ddl_commands() {
    let index = CompletionIndex::default();
    for (sql, expected) in [
        ("cre", "CREATE"),
        ("alt", "ALTER"),
        ("dro", "DROP"),
        ("tru", "TRUNCATE"),
    ] {
        let candidates = complete(
            sql,
            sql.len(),
            SqlDialect::Postgres,
            &index,
            CompletionContext::default(),
        );
        assert_eq!(
            candidates.first().map(|candidate| candidate.label.as_str()),
            Some(expected),
            "unexpected candidates for {sql}: {candidates:?}",
        );
    }
}
```

**Step 3: 写 object-kind 失败测试**

覆盖：

```text
CREATE | -> TABLE, VIEW, INDEX, SCHEMA
ALTER |  -> TABLE, VIEW, INDEX, SCHEMA
DROP |   -> TABLE, VIEW, INDEX, SCHEMA
TRUNCATE | -> TABLE
```

每个断言同时确认 `SELECT`、column 和 Catalog relation 不会泄漏到 object-kind 位置。

**Step 4: 写现有对象目标失败测试**

扩充 fixture，使其至少包含 table、view、index、trigger、sequence、type。增加测试：

```text
DROP TABLE us|       -> users/Table
ALTER TABLE us|      -> users/Table
DROP VIEW rep|       -> report/View
DROP INDEX ix_|      -> ix_users/Index
DROP SEQUENCE ord|   -> order_seq/Sequence
DROP TYPE sta|       -> status_type/Type
```

每个位置断言错误 kind 不出现，而不只是断言正确候选存在。

**Step 5: 运行测试确认按预期失败**

Run:

```bash
cargo test --test sql_completion statement_completion_offers_ddl_commands -- --exact
cargo test --test sql_completion ddl_object_kind_keywords_are_contextual -- --exact
cargo test --test sql_completion ddl_existing_object_candidates_match_target_kind -- --exact
```

Expected:

- 顶层测试因没有 DDL keyword 而失败。
- object-kind 测试因上下文仍为 `Statement` 而失败。
- 已有对象测试因 Catalog 被 `Statement` 拒绝、且部分 kind 未进入 index 而失败。

**Step 6: 记录当前回归基线**

Run:

```bash
cargo test --test sql_completion
```

Expected: 新测试失败，原有 32 个测试继续通过。不要为了得到全绿而临时放宽旧断言。

**Logical checkpoint:** DDL 第一阶段的用户可见行为已被失败测试固定。

## Task 2: 扩展候选种类和 CompletionIndex

**Files:**
- Modify: `src/sql/completion.rs:27-37,56-138,140-149,514-548`
- Modify: `src/ui/icons.rs:283-297`
- Modify: `tests/sql_completion.rs:931-958`
- Modify: `tests/ui_render.rs`（所有显式构造或穷举 `CompletionKind` 的位置）

**Step 1: 写 index 保留 DDL 对象的失败测试**

将 `index_retains_only_completion_relevant_entries` 改成正向断言：

```rust
assert!(index.entries().iter().any(|entry| {
    entry.kind == CatalogKind::Sequence && entry.qualified_name.object == "seq"
}));
```

为 index、trigger、type、constraint 增加相同断言。仍然验证不存在未知或不受支持的 kind；当前 `CatalogKind` 全部都有补全价值，因此测试重点改为去重和 scope，不再把 DDL 对象定义成 irrelevant。

**Step 2: 运行该测试确认失败**

Run:

```bash
cargo test --test sql_completion index_retains_ddl_completion_entries -- --exact
```

Expected: sequence 等对象被 `accepted_entries` 过滤，测试失败。

**Step 3: 扩展 `CompletionKind`**

在 `src/sql/completion.rs` 增加：

```rust
pub enum CompletionKind {
    Keyword,
    DataType,
    Database,
    Schema,
    Table,
    View,
    Column,
    Index,
    Constraint,
    Function,
    Procedure,
    Trigger,
    Sequence,
    Type,
}
```

`MaterializedView` 继续映射为 `View`，避免 UI 增加没有行为差异的 kind。四种 constraint CatalogKind 统一映射为 `Constraint`。

**Step 4: 扩展 `completion_kind` 映射**

使用穷举映射：

```rust
CatalogKind::Index => CompletionKind::Index,
CatalogKind::PrimaryKey
| CatalogKind::UniqueConstraint
| CatalogKind::ForeignKey
| CatalogKind::CheckConstraint => CompletionKind::Constraint,
CatalogKind::Trigger => CompletionKind::Trigger,
CatalogKind::Sequence => CompletionKind::Sequence,
CatalogKind::Type => CompletionKind::Type,
```

此时 `completion_kind` 可以直接返回 `CompletionKind`，但为了缩小改动，可以暂时保留 `Option<CompletionKind>`。只有确认所有 `CatalogKind` 都映射后再移除 `Option`，不要在同一步做无关重构。

**Step 5: 为 index 增加按 kind 的位置索引**

在 `CompletionIndex` 增加：

```rust
by_kind: HashMap<CatalogKind, Vec<usize>>,
```

在 `rebuild` 中填充，在 `replace`、`append`、`remove_ids` 后沿用现有 rebuild 生命周期。提供私有 helper：

```rust
fn positions_for_kinds(
    index: &CompletionIndex,
    kinds: &[CatalogKind],
) -> impl Iterator<Item = usize> + '_
```

如果 Rust 生命周期使返回 `impl Iterator` 复杂，直接返回 `Vec<usize>`；这里数据量有限，优先清晰性。

**Step 6: 暂时保持 DML allowlist 不变**

扩展 index 后，`catalog_kind_allowed` 仍必须保证：

- projection/predicate 只允许 column/function；
- relation 只允许 namespace/table/view；
- routine 只允许 function/procedure；
- qualifier 保持现有语义。

新增 kind 此时不应在任何现有 DML 测试中出现。

**Step 7: 扩展 UI 图标**

在 `IconSet::completion` 映射：

```text
DataType   -> CatalogKind::Type 的图标
Index      -> CatalogKind::Index
Constraint -> CatalogKind::CheckConstraint
Trigger    -> CatalogKind::Trigger
Sequence   -> CatalogKind::Sequence
Type       -> CatalogKind::Type
```

为 Nerd Font、Unicode、ASCII 三种模式更新或新增穷举测试。不要在 SQL service 层写 UI 字符串。

**Step 8: 运行 index、UI 和现有 completion 测试**

Run:

```bash
cargo test --test sql_completion index_retains_ddl_completion_entries -- --exact
cargo test --test sql_completion
cargo test --test ui_render
```

Expected: 全部通过；Task 1 中语法上下文相关测试仍可失败，直到后续任务实现。

**Step 9: 逻辑提交点**

如果用户要求 commits：

```bash
git add src/sql/completion.rs src/ui/icons.rs tests/sql_completion.rs tests/ui_render.rs
git commit -m "refactor(sql): index ddl completion objects"
```

## Task 3: 引入结构化 DDL 上下文和顶层关键字

**Files:**
- Modify: `src/sql/completion.rs:162-178,209-345,527-590,1127-1155`
- Modify: `tests/sql_completion.rs`

**Step 1: 增加上下文误判回归测试**

增加测试证明字符串、注释和普通标识符不会切换 DDL/DML 状态：

```text
CREATE TABLE t ("update" INTEGER, na|) -> 仍在 table definition，不出现 UPDATE
CREATE TABLE t (note TEXT DEFAULT 'drop table users', na|) -> 不进入 DROP
-- create table ignored\nsel| -> SELECT
SELECT create_time FROM users WHERE cre| -> 不出现 CREATE
```

最后一个断言应以现有 expression keyword policy 为准，不要让 statement DDL keyword 泄漏到 predicate。

**Step 2: 运行误判测试确认现有问题或缺失**

Run:

```bash
cargo test --test sql_completion ddl_keywords_do_not_leak_from_identifiers_strings_or_comments -- --exact
```

Expected: 至少 table-definition 上下文测试失败；若某个负面用例已通过，保留它作为回归覆盖。

**Step 3: 增加内部 DDL context 类型**

先实现第一阶段需要的最小集合：

```rust
enum DdlContext {
    CreateObjectKind,
    AlterObjectKind,
    DropObjectKind,
    ExistingObject(DdlObjectTarget),
    CreateIndexTarget,
}
```

在 `Context` 增加 `Ddl(DdlContext)`。不要一次加入尚未有测试驱动的 table-definition variants。

**Step 4: 将 `context_at` 改为语法位置驱动**

保留现有 scope 过滤，但不再对每一个 `Word` 都无条件执行“最后关键字获胜”。按当前 statement/scope 的 token prefix 识别结构：

```text
[]                                -> Statement
[CREATE]                          -> CreateObjectKind
[ALTER]                           -> AlterObjectKind
[DROP]                            -> DropObjectKind
[TRUNCATE]                        -> ExistingObject(Table) 或等待 TABLE
[ALTER, TABLE, ...]               -> ExistingObject(Table)，直到对象名完整
[DROP, VIEW, ...]                 -> ExistingObject(View)
[CREATE, INDEX, <name>, ON, ...]  -> CreateIndexTarget
```

必须只把语法位置上的未引用 token 识别为关键字。当前 tokenizer 将 quoted identifier 也表示为 `Word`，因此建议给 token 增加来源：

```rust
enum CompletionWordKind {
    Bare,
    Quoted,
}
```

或者在 `CompletionToken` 增加 `quoted: bool`。DDL 状态机只把 `Bare` word 与关键字比较。

**Step 5: 增加顶层 DDL keyword sets**

将 statement keywords 扩展为：

```text
SELECT, WITH（MySQL 按现有策略）, INSERT, UPDATE, DELETE,
CREATE, ALTER, DROP, TRUNCATE
```

为 DDL context 增加 object-kind keyword，首版公共集合：

```text
CREATE -> TABLE, VIEW, INDEX, SCHEMA, DATABASE
ALTER  -> TABLE, VIEW, INDEX, SCHEMA
DROP   -> TABLE, VIEW, INDEX, SCHEMA, DATABASE
TRUNCATE -> TABLE
```

方言扩展放在同一 helper：

```text
Postgres: MATERIALIZED VIEW, SEQUENCE, TYPE, FUNCTION, PROCEDURE
MySQL: FUNCTION, PROCEDURE, TRIGGER
SQL Server: FUNCTION, PROCEDURE, TRIGGER
SQLite: TRIGGER
```

只加入项目 Catalog 可以表示且主要方言真实支持的对象。

**Step 6: 明确 DDL keyword 排序**

结构关键字分数设为 `4`，与 statement keyword 相同；空 prefix 时按规则声明顺序优先，而不是纯字母序。如果不准备在本任务改变全局稳定排序，则用显式 priority 字段或 context score 区分常用项，不能依赖碰巧的 label 排序。

建议顺序：

```text
CREATE: TABLE, VIEW, INDEX, SCHEMA, DATABASE, 其余方言扩展
ALTER:  TABLE, VIEW, INDEX, SCHEMA, 其余方言扩展
DROP:   TABLE, VIEW, INDEX, SCHEMA, DATABASE, 其余方言扩展
```

**Step 7: 运行顶层和负面测试**

Run:

```bash
cargo test --test sql_completion statement_completion_offers_ddl_commands -- --exact
cargo test --test sql_completion ddl_object_kind_keywords_are_contextual -- --exact
cargo test --test sql_completion ddl_keywords_do_not_leak_from_identifiers_strings_or_comments -- --exact
cargo test --test sql_completion
```

Expected: 顶层和 object-kind 测试通过；已有 DML tests 全部通过。

**Step 8: 逻辑提交点**

```bash
git add src/sql/completion.rs tests/sql_completion.rs
git commit -m "feat(sql): add ddl keyword contexts"
```

## Task 4: 根据 DDL 目标类型生成已有 Catalog 对象候选

**Files:**
- Modify: `src/sql/completion.rs:209-345,386-458,484-548,676-736`
- Modify: `tests/sql_completion.rs`

**Step 1: 为每种目标写严格 allowlist 测试**

至少覆盖：

```text
ALTER TABLE |       -> Table，不出现 View/Column/Function
DROP VIEW |         -> View/MaterializedView 规则明确，不出现 Table
DROP INDEX |        -> Index
DROP TRIGGER |      -> Trigger
DROP SEQUENCE |     -> Sequence
DROP TYPE |         -> Type
DROP FUNCTION |     -> Function
DROP PROCEDURE |    -> Procedure
DROP SCHEMA |       -> Schema
DROP DATABASE |     -> Database
TRUNCATE TABLE |    -> Table
```

**Step 2: 增加 qualified-name 测试**

复用现有 database/schema fixture，覆盖：

```text
DROP TABLE app.|          -> schema public
DROP TABLE app.public.|   -> tables only
DROP INDEX app.public.|   -> indexes only
```

同时验证 active database/schema 下使用最短合法插入文本，并保留 relation detail。

**Step 3: 运行测试确认失败**

Run:

```bash
cargo test --test sql_completion ddl_existing_object_candidates_match_target_kind -- --exact
cargo test --test sql_completion ddl_existing_objects_support_qualified_names -- --exact
```

Expected: Context 已识别，但 candidate allowlist/qualified traversal 尚未支持新的 kind。

**Step 4: 用目标类型映射 CatalogKind allowlist**

实现显式映射：

```rust
fn ddl_catalog_kinds(target: DdlObjectTarget) -> &'static [CatalogKind] {
    match target {
        DdlObjectTarget::Table => &[CatalogKind::Table],
        DdlObjectTarget::View => &[CatalogKind::View, CatalogKind::MaterializedView],
        DdlObjectTarget::Index => &[CatalogKind::Index],
        DdlObjectTarget::Trigger => &[CatalogKind::Trigger],
        DdlObjectTarget::Sequence => &[CatalogKind::Sequence],
        DdlObjectTarget::Type => &[CatalogKind::Type],
        DdlObjectTarget::Function => &[CatalogKind::Function],
        DdlObjectTarget::Procedure => &[CatalogKind::Procedure],
        DdlObjectTarget::Schema => &[CatalogKind::Schema],
        DdlObjectTarget::Database => &[CatalogKind::Database],
        DdlObjectTarget::MaterializedView => &[CatalogKind::MaterializedView],
    }
}
```

不要用 `CompletionKind::View` 区分普通和 materialized view；过滤发生在原始 `CatalogKind` 层。

**Step 5: 修改候选索引入口**

`qualified_candidate_indices` 接受允许的 CatalogKind 集合，或让调用方先获取父节点 children 再按 kind 过滤。无 qualifier 时使用 `by_kind` 缩小集合，再做 prefix/compact-prefix 匹配；有 qualifier 时沿用 parent traversal，但在最终 children 上应用 allowlist。

不要改变 alias-qualified column completion 的行为。DDL object path 不应被 query alias 解析影响。

**Step 6: 抽离对象插入文本策略**

将当前只针对 table/view 的 `relation_insert_text` 泛化为 namespace-aware object insertion：

```rust
fn object_insert_text(
    entry: &CatalogEntry,
    context: CompletionContext<'_>,
    dialect: SqlDialect,
    qualifiers: &[String],
) -> String
```

规则：

- table/view/materialized view 维持现有最短引用；
- schema-level index/trigger/sequence/type/routine 使用对应方言允许的 database/schema qualification；
- database/schema 候选只插入当前 component；
- 已有 qualifier 时只插入 object component，避免重复 qualification；
- 所有组件继续使用 `quote_relation_component` 或等价的安全 quoting。

SQL Server 的 `DROP INDEX` 语法需要 `DROP INDEX index ON table`，第一阶段只完成 index 名候选；`ON table` continuation 在 Task 5 增加。不要为了统一语法生成错误的完整语句。

**Step 7: 运行对象、qualification 和现有 query 测试**

Run:

```bash
cargo test --test sql_completion ddl_existing_object_candidates_match_target_kind -- --exact
cargo test --test sql_completion ddl_existing_objects_support_qualified_names -- --exact
cargo test --test sql_completion completion_includes_databases_and_qualified_children -- --exact
cargo test --test sql_completion relation_completion_uses_shortest_target_relative_reference -- --exact
cargo test --test sql_completion
```

Expected: 全部通过。

**Step 8: 逻辑提交点**

```bash
git add src/sql/completion.rs tests/sql_completion.rs
git commit -m "feat(sql): complete ddl catalog targets"
```

## Task 5: 支持 CREATE INDEX、CREATE VIEW 和方言 continuation

**Files:**
- Modify: `src/sql/completion.rs`
- Modify: `tests/sql_completion.rs`

**Step 1: 写 CREATE INDEX 测试**

覆盖：

```text
CREATE INDEX ix_users ON us| -> users/Table
CREATE UNIQUE INDEX ix_users ON us| -> users/Table
CREATE INDEX ix_users ON app.public.us| -> users/Table
```

PostgreSQL 可增加 `CREATE INDEX CONCURRENTLY`，但只有现有 tokenizer/state machine 能在不扩大实现的情况下自然支持时才纳入本阶段。

**Step 2: 写 CREATE VIEW query handoff 测试**

覆盖：

```text
CREATE VIEW active_users AS sel| -> SELECT keyword
CREATE VIEW active_users AS SELECT u| FROM users -> visible columns
CREATE MATERIALIZED VIEW active_users AS SELECT | FROM users -> query expression candidates（Postgres）
```

DDL prefix 中出现的 `VIEW`、`AS` 不得污染 `SELECT` 子句的 relation scope。

**Step 3: 写 SQL Server DROP INDEX continuation 测试**

覆盖：

```text
DROP INDEX ix_users ON | -> table candidates
```

只在 SQL Server 启用该 continuation；其他方言继续按其语法完成 index object。

**Step 4: 运行测试确认失败**

Run:

```bash
cargo test --test sql_completion create_index_completion_targets_relations -- --exact
cargo test --test sql_completion create_view_query_uses_query_completion -- --exact
cargo test --test sql_completion sql_server_drop_index_on_completes_relation -- --exact
```

**Step 5: 扩展 DDL 状态转换**

识别可选 token 而不是依赖固定下标：

```text
CREATE [UNIQUE] INDEX <new-name> ON -> CreateIndexTarget
CREATE [MATERIALIZED] VIEW <new-name> AS -> 将 AS 后 token 交给 query context analyzer
DROP INDEX <existing-index> ON -> Relation，仅 SQL Server
```

新对象名称位置不应显示现有 Catalog 对象；该位置最多显示合法修饰符或空候选。

**Step 6: 复用 query analyzer，不复制 SELECT 状态机**

定位 `AS` 后的 byte/token 起点，使用现有 scope-local context detection 分析 suffix。不要为 view query 写第二套 projection/from/where 状态机。

**Step 7: 运行 focused 和完整 completion tests**

Run:

```bash
cargo test --test sql_completion create_index_completion_targets_relations -- --exact
cargo test --test sql_completion create_view_query_uses_query_completion -- --exact
cargo test --test sql_completion sql_server_drop_index_on_completes_relation -- --exact
cargo test --test sql_completion
```

Expected: 全部通过。

**Step 8: 第一阶段手动验收**

分别使用四种连接执行人工 smoke test：

1. 在新 statement 输入 `cre`、`alter `、`drop `。
2. 验证 object-kind 候选符合当前连接方言。
3. 输入 `DROP TABLE ` 并接受 table 候选。
4. 输入 `CREATE INDEX ix_test ON ` 并接受 table 候选。
5. 输入 `CREATE VIEW v_test AS SELECT `，确认 query candidate 恢复。
6. 不实际执行破坏性 DDL；仅验证编辑器文本和 popup。

**Step 9: 逻辑提交点**

```bash
git add src/sql/completion.rs tests/sql_completion.rs
git commit -m "feat(sql): complete ddl relation continuations"
```

## Task 6: 增加方言数据类型和 CREATE TABLE 定义上下文

**Files:**
- Modify: `src/sql/completion.rs`
- Modify: `tests/sql_completion.rs`
- Modify: `src/ui/icons.rs`（若 Task 2 尚未覆盖 DataType）

**Step 1: 写方言数据类型测试**

测试输入统一使用：

```sql
CREATE TABLE sample (id <cursor>)
```

最低覆盖：

```text
Postgres  -> INTEGER, BIGINT, NUMERIC, TEXT, BOOLEAN, DATE, TIMESTAMP, TIMESTAMPTZ, UUID, JSONB, VARCHAR
MySQL     -> INT, BIGINT, DECIMAL, VARCHAR, TEXT, DATETIME, TIMESTAMP, JSON, BOOLEAN
SQLServer -> INT, BIGINT, DECIMAL, NVARCHAR, VARCHAR, DATETIME2, BIT, UNIQUEIDENTIFIER
SQLite    -> INTEGER, REAL, TEXT, BLOB, NUMERIC
```

使用 prefix 验证方言隔离，例如：

```text
Postgres `j` -> JSONB
MySQL `j` -> JSON
SQL Server `unique` -> UNIQUEIDENTIFIER
SQLite `var` -> 不出现 VARCHAR
```

**Step 2: 写 table-element 分类测试**

覆盖：

```text
CREATE TABLE t (|                  -> table constraint keywords，可不提示类型
CREATE TABLE t (id |               -> data types
CREATE TABLE t (id INTEGER, name | -> data types
CREATE TABLE t (id INTEGER, |      -> table constraint keywords
```

首版不提示新列名，因为系统无法知道用户命名意图。

**Step 3: 写列约束和表约束测试**

覆盖公共集合：

```text
ColumnConstraint -> NULL, NOT NULL, DEFAULT, PRIMARY KEY, UNIQUE, REFERENCES, CHECK
TableConstraint  -> CONSTRAINT, PRIMARY KEY, UNIQUE, FOREIGN KEY, CHECK
```

再对方言扩展做最小测试：

```text
MySQL/SQLite -> AUTO_INCREMENT/AUTOINCREMENT 只出现在正确方言
SQL Server   -> IDENTITY
Postgres     -> GENERATED
```

**Step 4: 运行测试确认失败**

Run:

```bash
cargo test --test sql_completion create_table_completion_offers_dialect_data_types -- --exact
cargo test --test sql_completion create_table_completion_distinguishes_element_positions -- --exact
cargo test --test sql_completion create_table_completion_offers_constraints -- --exact
```

**Step 5: 增加第二阶段 DDL context variants**

增加：

```rust
CreateTableElement,
ColumnType,
ColumnConstraint,
TableConstraint,
ReferenceRelation,
ReferenceColumn,
```

通过 active parenthesis scope、同层 comma 和当前 element token 判断位置。只分析 `CREATE TABLE` 最外层 column-list；类型参数括号如 `VARCHAR(255)`、`DECIMAL(10, 2)` 不能被误判成新的 table element scope。

建议记录 `CREATE TABLE` definition 的起始 `LeftParen`，只消费 `scope_start == Some(definition_start)` 的 token；嵌套类型参数和 `CHECK (...)` 在更深 scope。

**Step 6: 增加内建类型规则**

实现：

```rust
fn data_types(dialect: SqlDialect) -> &'static [&'static str]
```

类型候选构造为：

```rust
CompletionCandidate {
    label: type_name.to_owned(),
    insert_text: type_name.to_owned(),
    kind: CompletionKind::DataType,
    detail: Some("data type".to_owned()),
    replace,
    score: CompletionScore { context: 4, ... },
}
```

类型候选不进入 `CompletionIndex`，也不从现有列 metadata 推导。

**Step 7: 增加 constraint keyword rules**

公共规则与方言扩展必须由 `keywords(Context::Ddl(...), dialect, ...)` 返回。多词关键字如 `NOT NULL`、`PRIMARY KEY`、`FOREIGN KEY` 作为一个候选，其 `replace` 只替换当前 prefix。

**Step 8: 处理 `CREATE TABLE ... AS SELECT`**

增加回归测试：

```text
CREATE TABLE archived AS SELECT | FROM users
```

`AS SELECT` 分支复用 query completion，不能被当作 column definition。

**Step 9: 运行 focused 和完整测试**

Run:

```bash
cargo test --test sql_completion create_table_ -- --nocapture
cargo test --test sql_completion
cargo test --test ui_render
```

Expected: 所有 CREATE TABLE 和原有 completion tests 通过。

**Step 10: 逻辑提交点**

```bash
git add src/sql/completion.rs src/ui/icons.rs tests/sql_completion.rs tests/ui_render.rs
git commit -m "feat(sql): complete create table definitions"
```

## Task 7: 支持 ALTER TABLE action、列、约束和索引候选

**Files:**
- Modify: `src/sql/completion.rs`
- Modify: `src/sql/mod.rs:23-27`
- Modify: `src/app.rs:8032-8127`
- Modify: `tests/sql_completion.rs`
- Modify: `tests/app_flow.rs` 或现有最接近 completion catalog-loading 的 integration test

**Step 1: 扩展 fixture 的 relation children**

为 `users` 增加：

- column `id`、`email`；
- index `ix_users_email`；
- primary key `users_pkey`；
- unique constraint `users_email_key`；
- foreign key `users_role_fkey`；
- check constraint `users_email_check`；
- trigger `users_audit_trigger`。

确保这些 entry 的 `parent_id`/`relation_id` 使用生产模型允许的构造函数，不手工伪造无效 shape。

**Step 2: 写 ALTER TABLE action 测试**

覆盖：

```text
ALTER TABLE users | -> ADD, DROP, ALTER, RENAME
```

方言规则：

- 通用首版保留四个 action；
- action 后续由更具体 context 过滤；
- 不支持的组合不出现在对应方言测试中。

**Step 3: 写 relation-child 候选测试**

覆盖：

```text
ALTER TABLE users DROP COLUMN e|      -> email/Column
ALTER TABLE users ALTER COLUMN e|     -> email/Column（Postgres/SQL Server）
ALTER TABLE users DROP CONSTRAINT u|  -> users_* constraint/Constraint
ALTER TABLE users DROP INDEX ix_|     -> ix_users_email/Index（MySQL）
```

断言另一个 table 的同名/相似 child 不出现。

**Step 4: 写依赖发现测试**

将现有 `relation_ids_for_completion` 的概念泛化为：

```rust
pub fn completion_dependencies(
    text: &str,
    cursor: usize,
    dialect: SqlDialect,
    index: &CompletionIndex,
    completion_context: CompletionContext<'_>,
) -> CompletionDependencies
```

测试：

```text
SELECT u. FROM users u                     -> users children
ALTER TABLE users DROP COLUMN |            -> users children
ALTER TABLE users DROP CONSTRAINT |        -> users children
CREATE INDEX ix_users ON users (|           -> users children
REFERENCES users (|                         -> users children
DROP TABLE users|                           -> 不需要 children
```

**Step 5: 运行测试确认失败**

Run:

```bash
cargo test --test sql_completion alter_table_completion_offers_actions -- --exact
cargo test --test sql_completion alter_table_completion_filters_relation_children -- --exact
cargo test --test sql_completion ddl_completion_reports_relation_child_dependencies -- --exact
```

**Step 6: 实现 ALTER TABLE contexts**

增加：

```rust
AlterTableAction,
ExistingColumn,
ExistingConstraint,
ExistingIndex,
```

DDL analysis 结果需要携带被修改 relation 的解析结果：

```rust
struct CompletionAnalysis {
    context: Context,
    bindings: Vec<RelationBinding>,
    ddl_relation: Option<RelationBinding>,
}
```

如果现有 query analyzer 尚未统一返回该结构，在本任务引入。`complete` 和 `completion_dependencies` 必须调用同一个私有 `analyze_completion`，避免候选认为需要 children，而 App 却没有发起加载。

**Step 7: 过滤 relation children**

解析 `ddl_relation` 到唯一/首选 relation ID，使用 `index.children` 获取其 children，并按上下文过滤：

```text
ExistingColumn     -> Column
ExistingConstraint -> PrimaryKey/UniqueConstraint/ForeignKey/CheckConstraint
ExistingIndex      -> Index
```

不要从全局 `by_name` 返回其他 relation 的 children。

**Step 8: 在 App 接入通用 dependencies**

替换 `src/app.rs:8074` 对 `relation_ids_for_completion` 的调用：

```rust
let dependencies = sql::completion_dependencies(...);
for relation in dependencies.relation_children {
    // 沿用现有 Loaded { next_cursor: None } 检查和 CatalogRequestIntent::Completion
}
```

保持：

- 请求去重；
- 不自动展开 Explorer；
- stale completion key 不生效；
- catalog generation 更新后重新计算候选。

可以保留 `relation_ids_for_completion` 作为私有 wrapper 或直接删除并更新唯一调用方。若它仍属于公开 API，优先保留一个调用 `completion_dependencies` 的兼容 wrapper，除非确认 crate 没有外部消费者并单独记录 breaking change。

**Step 9: 写 App integration test**

模拟 completion index 中有 `users` 但没有 loaded children，触发：

```sql
ALTER TABLE users DROP COLUMN
```

断言返回一个 `CatalogTarget::RelationChildren(users)` request，intent 为 completion，且不会请求其他 relation。再模拟 catalog response，重新补全后断言 column candidate 出现。

**Step 10: 运行 focused tests**

Run:

```bash
cargo test --test sql_completion alter_table_ -- --nocapture
cargo test --test sql_completion ddl_completion_reports_relation_child_dependencies -- --exact
cargo test --test app_flow completion
```

若 integration test 位于其他 target，替换最后一个命令为实际 test target 和 exact test name。

**Step 11: 运行完整 completion 测试**

Run:

```bash
cargo test --test sql_completion
```

Expected: 全部通过。

**Step 12: 逻辑提交点**

```bash
git add src/sql/completion.rs src/sql/mod.rs src/app.rs tests/sql_completion.rs tests/app_flow.rs
git commit -m "feat(sql): complete alter table children"
```

## Task 8: 支持 REFERENCES 和 CREATE INDEX 列候选

**Files:**
- Modify: `src/sql/completion.rs`
- Modify: `tests/sql_completion.rs`
- Modify if needed: App integration test from Task 7

**Step 1: 写 REFERENCES relation 测试**

覆盖：

```text
CREATE TABLE orders (user_id BIGINT REFERENCES us| -> users/Table
ALTER TABLE orders ADD FOREIGN KEY (user_id) REFERENCES us| -> users/Table
```

只显示 table；不要显示 view，除非某个方言明确允许且项目决定支持。

**Step 2: 写 REFERENCES column 测试**

覆盖：

```text
... REFERENCES users (i| -> id/Column
```

断言依赖发现返回 referenced `users`，而不是正在创建或修改的 source table。

**Step 3: 写 CREATE INDEX column 测试**

覆盖：

```text
CREATE INDEX ix_users_email ON users (e| -> email/Column
```

表达式索引只需继续允许函数/列基础候选；不在本轮实现完整 expression AST。

**Step 4: 运行测试确认失败**

Run:

```bash
cargo test --test sql_completion references_completion_targets_relation_and_columns -- --exact
cargo test --test sql_completion create_index_column_completion_uses_target_relation -- --exact
```

**Step 5: 实现 reference target tracking**

DDL analysis 增加 referenced relation binding，并在 `ReferenceColumn`/index column-list context 中将其传给 child candidate 和 dependency 逻辑。不要复用 query visible bindings，因为 source/target 语义不同。

**Step 6: 处理嵌套括号**

确保：

- `REFERENCES users (` 的列 scope 与 table definition scope 区分；
- `CREATE INDEX ... (lower(e|))` 仍能识别 target relation；
- 结束 `)` 后不继续泄漏 referenced column context。

**Step 7: 运行 focused 和完整测试**

Run:

```bash
cargo test --test sql_completion references_completion_targets_relation_and_columns -- --exact
cargo test --test sql_completion create_index_column_completion_uses_target_relation -- --exact
cargo test --test sql_completion
```

Expected: 全部通过。

**Step 8: 逻辑提交点**

```bash
git add src/sql/completion.rs tests/sql_completion.rs
git commit -m "feat(sql): complete ddl reference columns"
```

## Task 9: 让候选插入后缀适配 DDL 标点

**Files:**
- Modify: `src/sql/completion.rs:39-54,288-334`
- Modify: `src/sql/mod.rs:23-27`
- Modify: `src/app.rs:8130-8158`
- Modify: `tests/sql_completion.rs`
- Modify: 最接近 completion acceptance 的 editor/app integration tests

**Step 1: 写插入行为失败测试**

覆盖接受候选后的最终 editor text：

```text
`CREATE TAB|` 接受 TABLE              -> `CREATE TABLE `
`DROP TABLE us|;` 接受 users          -> `DROP TABLE users;`，分号前无空格
`CREATE TABLE t (id IN|)` 接受 INTEGER -> `CREATE TABLE t (id INTEGER)`
`REFERENCES users (i|)` 接受 id        -> `REFERENCES users (id)`
`CREATE INDEX ix ON users (e|, id)` 接受 email -> `email,` 前无空格
```

同时保留现有普通 query completion 接受后空格和 cursor 位置测试。

**Step 2: 运行测试确认失败**

Expected: 当前 `accept_completion` 在 `)`, `,`, `;` 前追加空格，至少三个断言失败。

**Step 3: 增加最小插入后缀策略**

优先采用候选自身声明策略：

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompletionSuffix {
    Space,
    None,
}
```

在 `CompletionCandidate` 增加：

```rust
pub suffix: CompletionSuffix,
```

首版不增加 `OpenParen` 和 snippet，占位符不在当前范围。

建议规则：

- keyword、data type 默认 `Space`；
- Catalog identifier 默认 `Space`；
- App 在右侧第一个非空字符是 `.`, `,`, `)`, `;` 时抑制 space；
- 候选显式 `None` 时始终不补空格。

如果所有当前候选都能仅靠右侧字符正确决定，可以不在 public candidate 增加 enum，而是在 App 提取 `needs_completion_separator(next_char)`。选择更小的正确改动：只有出现候选类别无法由右侧字符决定的测试时才增加 `CompletionSuffix`。

**Step 4: 实现 punctuation-aware separator**

最小 helper：

```rust
fn completion_needs_space(suffix: &str) -> bool {
    suffix
        .chars()
        .next()
        .is_none_or(|character| {
            !character.is_whitespace()
                && !matches!(character, '.' | ',' | ')' | ';')
        })
}
```

注意函数名与布尔逻辑保持直观；可改写以避免双重否定。不要移除引用标识符 escaping。

**Step 5: 运行 acceptance 和 UI tests**

Run:

```bash
cargo test completion_accept
cargo test --test sql_completion
cargo test --test ui_render
```

Expected: DDL 标点和现有 query acceptance 全部通过。

**Step 6: 逻辑提交点**

```bash
git add src/sql/completion.rs src/sql/mod.rs src/app.rs tests/sql_completion.rs
git commit -m "fix(sql): preserve ddl completion punctuation"
```

## Task 10: 统一自动触发语义并处理 dead trigger API

**Files:**
- Modify: `src/sql/completion.rs:460-482`
- Modify: `src/sql/mod.rs:23-27`
- Modify: `src/app.rs:7938-7963`（仅当决定重新接入 predicate）
- Modify: `tests/sql_completion.rs`

**Step 1: 先确认产品期望**

当前 App 对 Insert mode 下的普通编辑统一 debounce 补全，`.` 立即补全；`should_offer_completion` 已公开但没有生产调用方。实现时选择以下更一致的方案之一：

1. 推荐：保留统一 debounce，删除未使用的 `should_offer_completion` public API 和相关遗留计划假设。
2. 若该函数有外部 API 兼容要求：保留并改为基于 `analyze_completion` 判断，而不是维护另一份关键词后缀列表。
3. 不推荐：在 App 重新接入固定字符串 allowlist，因为它会再次产生 DDL/DML 规则漂移。

如果无法确认外部 API 消费情况，选择方案 2。

**Step 2: 写 trigger 一致性测试**

若保留 API，覆盖：

```text
CREATE |, ALTER |, DROP |, TRUNCATE TABLE |,
CREATE INDEX ix ON |, ALTER TABLE users DROP COLUMN |
```

字符串和注释尾部的相同文本不应返回 true。普通 identifier 输入仍按现有 debounce 模型触发。

**Step 3: 实现单一规则来源**

让 trigger helper 使用 completion token/context analysis，只回答“该位置是否可能产生候选”，不复制 `keywords()` 的文本表。

**Step 4: 运行测试**

Run:

```bash
cargo test --test sql_completion completion_trigger -- --nocapture
cargo test --test sql_completion
```

Expected: trigger 与实际 candidate generation 一致。

**Step 5: 逻辑提交点**

```bash
git add src/sql/completion.rs src/sql/mod.rs src/app.rs tests/sql_completion.rs
git commit -m "refactor(sql): align completion triggering"
```

## Task 11: 完成方言矩阵和鲁棒性回归

**Files:**
- Modify: `tests/sql_completion.rs`
- Modify only for defects found: `src/sql/completion.rs`

**Step 1: 增加 table-driven 方言测试**

建立测试表，至少覆盖：

| 场景 | Postgres | MySQL | SQL Server | SQLite |
|---|---|---|---|---|
| CREATE TABLE | yes | yes | yes | yes |
| CREATE MATERIALIZED VIEW | yes | no | no | no |
| CREATE/DROP SEQUENCE | yes | no/按支持策略 | yes/按支持策略 | no |
| CREATE/DROP TYPE | yes | no | no/按支持策略 | no |
| PROCEDURE | yes | yes | yes | no |
| TRIGGER | yes | yes | yes | yes |
| JSONB type | yes | no | no | no |
| NVARCHAR type | no | no | yes | no |
| AUTOINCREMENT | no | no | no | yes |

对不确定或 server-version-dependent 的语法，先查对应数据库官方文档再固定测试；不要凭记忆扩大支持矩阵。

**Step 2: 增加 quoting 测试**

覆盖：

```text
Postgres: DROP TABLE "odd table"
MySQL: DROP TABLE `odd table`
SQLServer: DROP TABLE [odd table]
SQLite: DROP TABLE "odd table"
```

验证 label 是安全展示文本、insert_text 保留正确 escaping、replace range 只覆盖当前 identifier component。

**Step 3: 增加不完整 SQL 容错测试**

覆盖：

```text
CREATE
CREATE TABLE
CREATE TABLE t (
CREATE TABLE t (id VARCHAR(
ALTER TABLE users DROP
REFERENCES users (
unterminated quoted identifier
unterminated string/comment before cursor
```

要求：不得 panic；候选可以为空，但不能回退到明显错误的全局 Catalog 集合。

**Step 4: 增加多 statement 和 scope 测试**

覆盖：

```text
SELECT ...; DROP TABLE us|
CREATE TABLE ...; SELECT | FROM users
CREATE VIEW ... AS SELECT ...; ALTER TABLE us|
```

确认只分析 cursor 所在 statement，沿用 `current_statement`/`scan_statements`。

**Step 5: 增加候选上限和排序测试**

构造超过 10 个同 prefix 对象，确认：

- 仍截断 10 条；
- exact/prefix 匹配高于 compact-prefix；
- active schema 高于其他 schema；
- context-valid keyword 高于低价值 modifier；
- 错误 kind 即使 exact match 也不会进入前 10。

**Step 6: 运行完整 completion suite**

Run:

```bash
cargo test --test sql_completion --all-features
```

Expected: 全部通过，无 ignored DDL tests。

**Step 7: 逻辑提交点**

```bash
git add src/sql/completion.rs tests/sql_completion.rs
git commit -m "test(sql): cover ddl completion dialects"
```

## Task 12: 全量验证、性能检查和文档收尾

**Files:**
- Verify: `src/sql/completion.rs`
- Verify: `src/sql/mod.rs`
- Verify: `src/app.rs`
- Verify: `src/ui/icons.rs`
- Verify: `tests/sql_completion.rs`
- Modify if user-facing capability docs list completion scope: `README.md` or `docs/architecture.md`

**Step 1: 格式化**

Run:

```bash
cargo fmt --all
cargo fmt --all -- --check
```

Expected: 成功且无格式差异。

**Step 2: 运行 SQL completion 和 UI 测试**

Run:

```bash
cargo test --test sql_completion --all-features
cargo test --test ui_render --all-features
```

Expected: 全部通过。

**Step 3: 运行 App/reducer 相关测试**

Run:

```bash
cargo test completion --all-features
cargo test catalog --all-features
```

Expected: completion scheduling、Catalog loading、stale request handling 和 UI popup 测试通过。

**Step 4: 运行全量测试和 Clippy**

Run:

```bash
cargo test --all-features
cargo clippy --all-targets --all-features -- -D warnings
git diff --check
```

Expected: 所有命令成功，无 warning 和 whitespace error。

**Step 5: 检查 completion 性能没有数量级回归**

如果项目已有 benchmark 基础设施，增加包含数千 Catalog entries 的 completion benchmark；否则添加一个 ignored 或普通测试并测量开发机上的粗略耗时，不引入新的 benchmark dependency。

验收目标：

- 无 qualifier 的 DDL 对象候选先按 kind 缩小集合；
- 不对每个候选重新 token scan；
- `complete` 每次只分析 current statement；
- dependency 和 candidate generation 即使分别调用，也共享同一私有 analysis algorithm；
- 典型 Catalog 下无肉眼可见 popup 延迟。

不要固定脆弱的毫秒断言到 CI；性能测试主要验证算法路径和候选集合规模。

**Step 6: 人工 smoke test**

每种数据库连接至少验证：

1. 顶层 DDL keyword。
2. `DROP/ALTER TABLE` object completion。
3. `CREATE TABLE` data type 和 constraint。
4. `ALTER TABLE ... DROP COLUMN` lazy child loading。
5. `CREATE INDEX ... ON table (column)`。
6. `REFERENCES table (column)`。
7. 接受候选后在 `)`, `,`, `;` 前没有多余空格。
8. DDL statement 前后各有 SELECT 时，query completion 不受污染。
9. Popup 在 Escape、Normal mode、tab switch 后按现有生命周期关闭。

不要执行 `DROP`/`TRUNCATE`；只验证编辑文本，或使用一次性测试数据库并在执行前再次确认。

**Step 7: 更新用户文档**

仅当 README 或架构文档明确列出 completion 能力时，增加一句：SQL Editor 支持方言感知的 query/DML/DDL contextual completion。不要新增配置项或 keybinding 文档，因为交互入口没有变化。

**Step 8: 检查最终 diff**

Run:

```bash
git status --short
git diff --stat
git diff --check
```

确认只包含本计划相关文件；保留并忽略工作区中用户已有的无关改动。

**Step 9: 最终逻辑提交点**

如果用户明确要求提交：

```bash
git add src/sql/completion.rs src/sql/mod.rs src/app.rs src/ui/icons.rs tests/sql_completion.rs tests/ui_render.rs docs/architecture.md
git commit -m "feat(sql): add contextual ddl completion"
```

实际 stage 时只包含确实修改过的文件，不要为匹配示例命令而 stage 未修改或无关文件。

## 实施顺序和发布策略

按以下顺序执行，不并行修改同一个 completion 状态机：

1. Task 1-3：先建立测试和 DDL context，不接触 lazy loading。
2. Task 4-5：交付第一阶段，可独立发布。
3. Task 6：交付 CREATE TABLE 类型和约束。
4. Task 7-8：接入 relation children，交付第二阶段。
5. Task 9-10：收紧交互和触发一致性。
6. Task 11-12：方言矩阵和全量验收。

建议在 Task 5 后建立第一个可回退检查点。若第二阶段出现 scope 或 Catalog loading 风险，可以先发布第一阶段，而不暴露半完成的列/约束候选。

## 风险及缓解措施

### 风险 1：DDL 状态机与现有 query 状态机互相污染

缓解：所有新增行为从失败测试开始；DDL context 使用显式 allowlist；`CREATE VIEW ... AS SELECT` 必须 handoff 到同一个 query analyzer，而不是继续扫描整段 DDL token 并以最后一个文本关键字决定状态。

### 风险 2：CatalogIndex 扩容降低普通补全性能

缓解：增加 `by_kind`，在 prefix matching 前按语法允许的 CatalogKind 缩小集合；保留 10 条上限；不从全量 Catalog 推导 data type。

### 风险 3：relation children 未加载导致空候选

缓解：`completion_dependencies` 与 `complete` 共享私有 analysis；沿用 generation key 和现有 `CatalogRequestIntent::Completion`；先显示 keyword，加载后重算对象。

### 风险 4：方言规则不准确

缓解：公共 SQL 只放四种方言共同支持的核心；扩展语法按官方文档和 adapter capability 固定测试；server-version-dependent 功能留到后续。

### 风险 5：不完整 SQL 让完整 parser 失败

缓解：不以 AST parse success 为补全前提；保留 tolerant scanner；所有 unterminated input 都有 no-panic 测试。

### 风险 6：公开 API 变化影响外部消费者

缓解：保留 `complete` 签名；新增 dependency API；对 `relation_ids_for_completion` 和 `should_offer_completion` 先确认使用范围，无法确认时提供薄 wrapper，不做无依据的 breaking removal。

## Definition Of Done

- 第一、二阶段成功标准全部有自动测试。
- 四种方言都有正向和负向 DDL completion 测试。
- Catalog object、data type、column、constraint 的 `CompletionKind` 和图标完整。
- DDL lazy-loading 与候选生成使用一致的语法分析结果。
- 接受候选不会在 DDL 标点前插入多余空格。
- `cargo fmt --all -- --check` 通过。
- `cargo test --all-features` 通过。
- `cargo clippy --all-targets --all-features -- -D warnings` 通过。
- `git diff --check` 通过。
- 人工 smoke test 完成，且未对真实数据执行破坏性 DDL。
