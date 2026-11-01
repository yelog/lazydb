# TiDB Support Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.
> 此环境未列出该子技能，按下列顺序直接执行。用户已授权实现；不创建 commit 或 push。

**Goal:** 将 TiDB 注册为独立数据库，提供连接、SQL 查询、目录浏览/搜索、关系预览和原生 DDL 展示。

**Architecture:** 新增 `DatabaseKind::TiDb`、`tidb://` URL 和数据库连接分发，复用 SQLx MySQL 传输及 MySQL SQL 方言。MySqlAdapter 保留 TiDB 产品身份，按 TiDB 8.5+ 的产品版本验证目录能力，仅暴露 table/view 与适用的关系子对象。不开放未经验证的结构化修改、监控、账号管理、取消和手动事务；普通 SQL 执行继续遵循已有只读策略。

**Tech Stack:** Rust 1.94、SQLx 0.9、Tokio、sqlparser、ratatui、TiDB 8.5 fixture。

---

### Task 1: 注册、连接与持久化

**Files:** `src/profile.rs`, `src/db/descriptor.rs`, `src/model/profile_manager.rs`, `src/ui/profiles.rs`, `src/ui/icons.rs`, `src/cli.rs`, `tests/tidb_profile.rs`, `tests/driver_categories.rs`。

1. 添加 URL/配置往返、凭据不导出、TLS 参数与独立产品身份测试。
2. 新增 TiDb kind/URL format；默认端口 4000，用户 root；目录模型 database-is-schema。
3. UI、CLI、URL、注册表保持产品身份并复用已有连接字段。
4. Run: `cargo test --locked --test tidb_profile --test driver_categories --test profile_url --test profile_draft --test connections_cli`。

### Task 2: TiDB 后端能力

**Files:** `src/db/mysql.rs`, `src/db/mysql/tidb.rs`（若需要）, `tests/tidb_adapter.rs`。

1. 添加产品版本测试：`5.7.25-TiDB-v8.5.3`、`8.0.11-TiDB-v8.5.3`、低版本、未知版本与错误产品。
2. TiDB profile 使用 SQLx MySqlConnectOptions 构建 MySQL 协议连接，保留 kind；产品版本门槛来自 TiDB 后缀。
3. Tables/views 目录、搜索、子对象和 SHOW CREATE 使用实际兼容 SQL；不查询 unsupported routines/triggers/events。
4. 隔离测试覆盖 scoped catalog/search、结果类型、空结果列、view/table preview/DDL 和 unsupported 操作。
5. Run: `cargo test --locked --test tidb_adapter --test mysql_adapter --test mariadb_profile`；真实 TiDB 使用 `LAZYDB_REQUIRE_DATABASE_TESTS=1 LAZYDB_TEST_TIDB_URL=...`。

### Task 3: 公共分发、SQL 和能力边界

**Files:** `src/db/mod.rs`, `src/db/capabilities.rs`, `src/db/mutation.rs`, `src/sql/dialect.rs`, `src/app.rs`, `src/model/execution_target.rs`, `src/model/explorer_actions.rs`, `src/model/cell_editor.rs`, `src/model/relation_review.rs`, `src/db/column_type.rs`, `src/runtime.rs`，以及编译器指出的消费者。

1. 新增独立 DatabaseConnection::TiDb，连接、kind、查询/预览/DDL/目录/关闭复用 adapter。
2. 静态能力设 catalog/relation_ddl=true；relation_edit/monitoring/manual_transactions/cancellation=false。
3. 结构化修改入口在 planner/executor 两层拒绝，关系 mutation insert/update/delete 均拒绝；不依赖 UI 隐藏保证边界。
4. SQL 方言映射 MySql；执行目标按 database-is-schema 处理，补全/风险识别沿用 MySQL。
5. Run: `cargo check --locked --all-targets`；能力与执行目标测试加入 TiDB 回归。

### Task 4: 实机 fixture、文档及验收

**Files:** `docker-compose.tidb.yml`, `.github/workflows/ci.yml`, `docs/tidb.md`, `docs/database-capabilities.md`, `docs/configuration.md`, `docs/README.md`, `README.md`。

1. 提供隔离、固定版本 TiDB unistore fixture，绑定 loopback 非默认端口；不访问用户已有数据库。
2. 文档列出版本门槛、连接/TLS、支持项与限制；CI 强制执行 TiDB suite。
3. Run: `cargo fmt --all -- --check`, `cargo clippy --locked --all-targets -- -D warnings`，针对性 profile/catalog/SQL/执行目标/能力回归及完整测试。
4. 实机检查必须明确记录是否执行；环境受限不得将 skip 描述为实机通过。

## Execution evidence

- 固定 TiDB 8.5.3 unistore 实机返回 `8.0.11-TiDB-v8.5.3`；专项 adapter 5/5、profile 7/7、contract 4/4、UI 8/8 通过。严格模式设置 `LAZYDB_REQUIRE_DATABASE_TESTS=1`，未跳过实机测试。`db::mysql::tidb::tests` 的 3 项库测试通过，覆盖整批只读校验、搜索源和 adapter 直接拒绝入口。
- MySQL 8.4 实机 adapter 回归 16/16 通过；MariaDB 11.4 串行执行全部 13 套集成回归，共 26 项通过（包括配置、目录、结构修改、事务、行修改、监控、账号和类型）。
- 相关驱动类别、连接表单、SQL 方言和 UI 渲染回归通过；UI 渲染 245 项通过、1 项按原测试标记忽略。
- `cargo fmt --all -- --check`、`git diff --check` 和全部 target/feature 的严格 Clippy 通过。Compose 配置与文档链接检查通过；构建产物 `lazydb capabilities` 已列出 `tidb`。
- 首次完整测试因缺少可选 DuckDB 1.5.6 动态库停止：1,188 项通过，13 项均为 `duckdb_runtime_missing`。配置经官方 release SHA-256 验证的隔离运行库后补跑，最终结果见下方。
- 补齐 DuckDB 运行库后，库测试 1,201 项通过、1 项忽略。默认并行的全量执行遇到 MariaDB 目录测试之间的增删对象竞争（exact count 与 payload 不一致）；使用现有 CI 相同的 `--test-threads=1` 串行执行实机测试，MariaDB 各套回归与 MySQL adapter 均通过。
- shell 继承的 PostgreSQL 实机地址不可用，8 项 adapter 用例连接超时；仅在最终验证子进程移除 `LAZYDB_TEST_POSTGRES_URL`。本轮不计 PostgreSQL 或其他未配置外部数据库为实机通过。
- 最终隔离配置执行 `cargo test --locked --all-targets --all-features --no-fail-fast -- --test-threads=1`，覆盖 193 个测试目标：3,468 项通过、16 项忽略，仅旧驱动循环断言仍预期末尾为 ClickHouse。将断言更新为新增 TiDB 后，单独重跑 `profile_reducer` 全部 49 项通过。未声称该次全量命令返回 0；全部失败已由针对性复跑收敛。
- 最后一次格式、空白检查和严格 Clippy 均通过。本任务创建的 TiDB compose project、MySQL/MariaDB 临时容器及下载校验用 DuckDB 运行库已移除；未改动其他服务。未执行 commit 或 push。

## 差异审查与全量验收（2026-10-31）

- 初轮隔离实机专项 40 项通过；完整命令 `cargo test --locked --all-targets --all-features --no-fail-fast -- --test-threads=1` 退出 0，193 个测试目标共 3,469 项通过、16 项忽略。
- 审查确认并修复只读序列副作用：原实现允许 `SELECT NEXTVAL(...)` 推进序列；新增表达式 AST 检查，在执行整批 SQL 前拒绝 `NEXTVAL`/`SETVAL`，同时覆盖子查询、WHERE 和 ORDER BY。关系预览复用该检查；实机测试验证所有被拒绝调用之后序列的首个值仍为 1。
- 审查确认并修复 SQL 分割回归：默认 MySQL 方言下，双引号字符串中的反斜杠转义未被扫描器识别，导致内嵌分号误切分。新增 MySQL 方言限定的转义处理，保留 PostgreSQL 双引号标识符行为，附实机双结果集和扫描器回归测试。
- 已检查产品版本与连接注册、凭据/TLS 参数、共享 MySQL/MariaDB 路径、目录范围及元数据、结果列、UI/恢复/重绑定和未开放功能的 planner/executor 入口。审查子任务因 API 限额中断，其已报告候选均由主任务实机复现，剩余差异由主任务直接检查。
- 验收连接仅使用本轮创建的 TiDB 8.5.3 unistore、MySQL 8.4、MariaDB 11.4；DuckDB 使用项目脚本与固定 SHA-256 验证的 1.5.6 运行库。其他外部数据库未配置，不计为实机通过。
- 修复后强制实机专项退出 0：TiDB adapter 6、contract 4、profile 7、UI 8，以及 MySQL adapter 16，共 41 项通过、0 项忽略。严格 Clippy、格式和空白检查通过。
- 修复后完整验收退出 0：`cargo test --locked --all-targets --all-features --no-fail-fast -- --test-threads=1` 覆盖 193 个测试目标，共 3,471 项通过、0 项失败、16 项忽略。其中 MariaDB 全部 13 套集成回归 26 项通过；SQL scope 扫描器全部 14 项通过。
- 实机版本为 TiDB 8.5.3（`8.0.11-TiDB-v8.5.3`）、MySQL 8.4.13、MariaDB 11.4.13。DuckDB 1.5.6 archive SHA-256 为 `e0bc007d9b0094c0970ac1847a8601d10aad07cbd2910ce586ec810f77b638d6`。
- 本轮 `lazydb-tidb-acceptance` compose project、两个 `--rm` MySQL/MariaDB 容器及校验用 DuckDB 运行库已清理，其他服务未改动。最后格式和空白检查通过。未执行 commit 或 push。
- 验收日志保留在 `/tmp/lazydb-tidb-acceptance-full.log`、`/tmp/lazydb-tidb-acceptance-strict.log` 和 `/tmp/lazydb-tidb-acceptance-clippy.log`。该轮未覆盖 TiDB Cloud；后续 Cloud 实测见下节。TiKV/PD 自建部署仍未验证。

## TiDB Cloud 实测（2026-10-31）

- 按用户授权，通过 ego-browser 在新账号创建 `lazydb-cloud-validation`，选择 Starter / MySQL Compatible / AWS Tokyo，月消费上限明确设为 **$0**，未添加信用卡。保留该免费实例供用户后续使用。
- 控制台显示 v8.5.3，实际 `VERSION()` 为 `8.0.11-TiDB-v8.5.3-serverless`。LazyDB 使用 `sslMode=verify-full` 和系统信任库连接网关，`Ssl_cipher` 非空；没有关闭证书校验或添加自签 CA。
- `LAZYDB_REQUIRE_DATABASE_TESTS=1` 下执行 TiDB adapter / contract / profile / UI，25 项通过、0 项失败、0 项忽略，其中 adapter 四项实机测试均实际执行。覆盖目录分页/搜索/scope、table/view 元数据与 DDL、预览、空结果列、多结果集、原生值、SQL 写入，以及客户端只读批次/序列防护。
- 额外 Cloud 验收验证按 id 排序的 23 行预览分页（10 + 10 + 3）、SELECT-only 账号的查询与数据库发现、服务端拒绝 INSERT，以及错误密码归类为 Authentication。服务端权限拒绝的原始消息明确，但当前通用错误映射仍归类 Sql（SQLSTATE 42000）；未在本次验证中扩大修改共享错误分类器。
- 测试脚本根据实际错误调整 Cloud 用户名：CREATE USER 和连接均须包含实例前缀，完整用户名不能超过 32 字符。曾出现一次 10 秒 pool acquisition 超时，重试通过；本轮不提供可用性或延迟保证。
- 新实例默认放行所有公网 IP；已移除该规则，仅保留浏览器和 SQL 客户端两个独立出口 IP。收紧后服务端明确拒绝未授权 SQL 出口；添加实际 SQL 出口的单 IP 规则后继续清理成功。后续更换公网 IP 需更新规则。
- 临时数据库、表、视图、序列和 SELECT-only 用户已清理，并查询目录确认不存在；本次生成的 root 测试密码已轮换失效，本机临时凭据文件已删除。后续连接需在控制台重置密码。未保存 Cloud 凭据到仓库或聊天。
- 保留脱敏日志 `/tmp/lazydb-cloud-bootstrap.log`、`/tmp/lazydb-cloud-suite.log`、`/tmp/lazydb-cloud-extra.log`、`/tmp/lazydb-cloud-cleanup.log`；一次性 Cloud 验收 harness 移出仓库，不加入默认测试或 CI。
- 本轮仅补充实测文档，未修改生产代码，未重复先前 3,471 项全量测试；格式与空白检查另行执行。未执行 commit 或 push。
- 未覆盖 Dedicated、私网连接、其他区域、PostgreSQL compatibility mode、故障切换、生产负载或自建 TiKV/PD。

## TUI 验收与发布收尾（2026-10-31 至 2026-11-01）

- 使用实际构建的 `lazydb`，通过 PTY 发送按键并用终端屏幕解析器读取 160 × 45 画面。独立配置目录、加密凭据和 Cloud 临时数据库均与用户配置隔离；未修改已有工作区数据。
- 在实际连接表单选择 TiDB，填写 Cloud endpoint、实例前缀用户名和临时数据库，使用 `VERIFY FULL` 与 Local encrypted 密码存储。保存连接后，Explorer 成功发现 Tables / Views，表和视图预览显示测试数据，DDL 页显示 Native Catalog 定义，SQL console 查询显示结果。
- 验收发现已连接配置启用只读并保存后，`request_connection` 会复用相同执行目标的旧 writable session，导致只读设置未生效。修复保存后的连接参数或凭据变更路径：先退役旧 session，再创建新连接；新增 `saving_read_only_tidb_profile_reconnects_instead_of_reusing_writable_sessions` 回归。
- 修复后的实际 Cloud TUI 重跑 writable → read-only 切换，确认 INSERT 被 adapter 拒绝，随后 SELECT 正常。重启恢复了表/视图标签与 SQL 文本；应用从 Explorer 退出，返回 0。旧版 PTY 进程曾在退出时超时并被终止，未将该次退出描述为成功。
- `profile_reducer` 49 项与 `tidb_ui` 9 项通过；另外 9 套连接切换、配置生命周期、事务和工作区相关回归共 99 项通过。全部 target/feature 严格 Clippy、格式、空白和 release metadata 检查通过。
- 最终 Cloud 专项首次补跑被提前清理临时数据库打断，后两项报 Unknown database；这是本轮测试编排错误。重新创建隔离数据库后顺序重跑，清理推迟到测试命令结束后执行：adapter 6、contract 4、profile 7、UI 9，共 26 项通过、0 项失败、0 项忽略，命令退出 0。日志为 `/tmp/lazydb-tui-final-cloud-rerun.log`。
- 最终 Cloud 临时数据库已删除，查询 information_schema 确认计数为 0；root 临时密码已轮换失效。独立 TUI 配置、加密凭据及密钥、浏览器提取字段、PTY 工具虚拟环境均已移除，一次性 cleanup harness 已移出仓库。保留免费实例与原有两条单 IP 防火墙规则。
- 最后浏览器收尾调用发现 TaskSpace 3 已不存在，未新建或接管空间；不声称该次 finish 成功。Cloud 数据清理通过数据库连接独立完成。
- `CHANGELOG.md` 新增 Unreleased，列出 TiDB 首期支持、已验证环境、未开放能力，以及 session 复用与 SQL 双引号扫描修复；未变更发布版本号，未执行 commit、push 或创建 PR。

## PR #31 合并前审查（2026-11-01）

- 初始提交 `c3d0c1b` 的 12 项 CI 全部通过；Linux/macOS 各 193 个目标、3,493 项通过、16 项忽略。CI 检出与最新 main 合并的 `23a6677`；TiDB 强制实机专项 26 项、数据库作业共 163 项、MongoDB 作业 83 项通过。该证据仅对应初始提交，后续修复另行验收。
- 审查确认保存连接参数或凭据时可能直接断开空闲手动事务。改为复用 `profile_exit_check` 与 deferred transaction resolution：事务全部提交/回滚成功后才退役旧连接；取消、失败或未提交关系编辑会保留旧连接。新增 settings/credentials、多个 console、失败/取消回归。
- 将序列副作用与嵌套 locking read 检查统一到共享 SQL risk AST visitor，使 writable TiDB profile 的 agent query 入口同样拒绝 NEXTVAL/SETVAL，且覆盖 scalar/WHERE 子查询，保留字符串和注释不误报。TiDB 局部只读校验复用共享策略。
- TiDB metadata prepare 禁用 statement cache，避免相同 SELECT 在 ALTER/USE 后复用旧列信息；prepared metadata 初始化不再记录执行响应时间。新增实机 DDL 前后、空结果列及 accumulator 时序回归。
- TiDB 按物理连接当前 `sql_mode` 切分，非 DQL 后刷新；支持同批 SET 后的 ANSI_QUOTES/NO_BACKSLASH_ESCAPES 边界。只读配置对非默认 quote mode 保守拒绝。MySQL/MariaDB 可执行注释保留、MariaDB 扫描规则对齐；自动 current-statement 在不同模式产生不同边界时拒绝，显式 selection 保留。扫描器覆盖四种模式组合和 UTF-8。
- TiDB 搜索使用独立 SQL 常量，不再通过另一数据库 SQL 的精确空白片段与 expect 构造。连接阶段的产品校验 probe 保留：重复一次探测是已知低优先级成本，避免为本次接入扩大连接生命周期接口。
- 第一轮新增 ANSI_QUOTES 实机用例中的反斜杠结尾标识符被 TiDB 8.5.3 自身拒绝；对应特殊边界由扫描器单测覆盖，实机改用服务端支持的标识符并保留 NO_BACKSLASH_ESCAPES 切分和 executable comment 检查。
- 统一针对性回归 125 项通过、0 失败、0 忽略，包含 TiDB adapter 8、contract 4、profile 7、UI 9，共 28 项强制实机配置专项。首次统一命令缺 DuckDB 动态库，配置官方脚本下载且摘要校验的 1.5.6 运行库后完整重跑通过；未计首次失败为通过。
- 严格 `cargo clippy --locked --all-targets --all-features -- -D warnings`、格式、空白和 release metadata 检查通过。原审查候选限定复核确认前 9 项已收敛，重复 probe 保留为已知非阻塞成本；未扩大复核范围，未发表远端评论。
- 修复后本地完整命令 `cargo test --locked --all-targets --all-features --no-fail-fast -- --test-threads=1` 退出 0：193 个测试目标，3,490 项通过、0 失败、16 项忽略。配置隔离 TiDB 8.5.3 与已校验 DuckDB 1.5.6 运行库；未配置的其他外部数据库不计为实机验证。该分支未合并 main 的后续 MongoDB 提交，计数与初始合并提交 CI 不同。日志 `/tmp/lazydb-pr31-review-full.log`。
