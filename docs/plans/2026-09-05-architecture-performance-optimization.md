# Architecture and Performance Optimization Implementation Plan

> 执行方式：逐任务实施，每项先补回归测试，再做最小修改，最后运行对应检查。本文仅为计划，不授权 commit、push、数据迁移或访问真实用户数据库。可使用执行计划类技能辅助，但不得依赖当前环境未安装的技能。

**Goal:** 控制结果与编辑器缓存的内存增长，减少交互热路径的全量工作，并收敛持久化和执行流程的重复规则。

**Architecture:** 保留 Rust 模块化单体、Action -> App -> Command -> Runtime 和原生数据库适配器。先修资源生命周期与重复计算，再下沉结果保留预算，最后按已验证的业务边界做局部拆分；不以更换框架、引入通用驱动或全量流式 UI 为前提。

**Tech Stack:** Rust 1.94 / Tokio / Ratatui / Modalkit / SQLx / Tiberius；优先使用现有依赖与测试设施。

---

## 1. 范围、证据与约束

本计划基于 2026-09-05 源码静态审查。尚未执行性能基准和完整测试，不承诺具体加速倍数。文件行号是定位参考，实施前应按符号获取最新源码。

| 问题 | 当前证据 | 目标 |
| --- | --- | --- |
| 结果全量累积 | `src/db/query.rs:64`、`src/db/mysql.rs:516`、`src/db/postgres.rs:3147` | 在读取过程中限制保留的数据 |
| Agent 事后截断且 execute 不一致 | `src/agent/service.rs:130`、`:206`、`:242` | query/execute 共享资源预算，不混淆授权策略 |
| 高亮缓存按 revision 增长 | `src/editor/mod.rs:261`、`:679`、`:365` | 每个文档只保留有界分析版本，删除时清理 |
| 编辑器快照重复全文工作 | `src/editor/mod.rs:678`、`:709`、`src/app.rs:1033`、`src/ui/mod.rs:2305` | 稳定 revision 下复用索引，只投影视口 |
| 保存前深拷贝结果 | `src/app.rs:1113`、`:1191` | 从借用直接构造持久化 DTO |
| 保存全量写盘、忽略错误 | `src/persistence/workspace.rs:201`、`src/runtime.rs:583` | 增量写入、串行合并、显式失败反馈 |
| 网格每帧扫描全部单元格 | `src/ui/data_grid.rs:64`、`:276` | 一次准备列宽，渲染只处理可见行列 |
| 分页执行重复 | `src/runtime.rs:1904`、`:2072` | 共享分页机制，保留调用方身份与事务边界 |

必须保留：现有分页、目录懒加载、stale-event 校验、SQL 原文与显示清洗分离、不可变执行预览、事务不确定状态处理、原生驱动差异。

明确不做：全面替换 Modalkit；改为 AnyPool；引入 ECS/通用事件总线；为所有类型加 Arc；任意 SQL 强制 keyset 分页；重写所有 Action；本轮实现完整流式结果展示、磁盘溢写或跨文件工作区迁移。

实施开始前检查工作树。当前已有安装器、发布工作流及其他计划变更，不得覆盖。编辑器和表格相关并行计划可能触及同一文件，实施时先协调这些文件的修改顺序。

## 2. 阶段与依赖

| 阶段 | 任务 | 依赖 | 完成门槛 |
| --- | --- | --- | --- |
| A 基线 | T01 | 无 | 固定数据集、当前行为和性能记录 |
| B 低风险优化 | T02-T06 | T01；T03 依赖 T02；T05 依赖 T04 | 缓存有界、保存不复制结果、稳定网格不全表扫描 |
| C 结果资源边界 | T07-T10 | T01；按 T07 -> T08 -> T09 -> T10 | 四驱动契约一致，Agent 与 TUI 明确显示截断 |
| D 结构收敛 | T11-T12 | 对应业务优化稳定后 | 共享机制有测试，局部 reducer 不依赖整个 App |
| E 验收 | T13 | 本轮选定任务完成 | 功能回归、性能对比、文档一致 |

推荐先完整交付 B，再交付 C。预算紧张时先做 T02、T04、T06，但不能因为 C 成本高就把 Agent 资源问题无限延期。T11-T12 可以后置，不作为前面性能优化的前置条件。

每个任务拆成测试、实现、验证三个独立检查点。下面的任务不是单个巨型提交；如后续获准提交，按可独立验证的逻辑单元提交，不在本计划编写阶段执行 Git 历史操作。

## T01: 建立基线与固定负载

**文件：** 新建 `tests/performance_regression.rs`、`docs/performance.md`；检查 `tests/explorer_performance.rs`、`tests/ui_render.rs`、`tests/workspace_tabs.rs`；需要访问私有缓存时在 `src/editor/tests.rs`、`src/ui/data_grid.rs` 的测试模块内添加用例。

1. 记录工作树、工具链版本、运行模式和测试机器信息，不记录 SQL 凭据或用户数据。
2. 使用确定性生成器构造 SQL 文档：约 100KB、1MB；结果：500x20、500x200、10,000x20；长单元格单独测试。1M 行等压力负载只用于手动运行，不进入普通 CI。
3. 固定编辑器视口、网格视口和光标位置，分别覆盖文档顶部/底部、同 revision 重绘、连续修改、多标签保存。
4. 复用 Ratatui 测试后端。耗时基准采用 release 下 ignored tests，记录预热后中位数和 p95；不要把 elapsed < 固定毫秒写进普通测试。
5. 在被测函数内提供仅测试可用的访问/构建次数统计，或返回内部工作量统计用于断言。避免跨测试共享全局计数器。
6. 添加明确的基准入口测试名 `performance_baseline`，先记录现状。目标性质的失败测试随 T02-T10 添加，不把必然失败的测试单独合入主线。

**命令：** `cargo test --test explorer_performance`；`cargo test --release --test performance_regression performance_baseline -- --ignored --nocapture --test-threads=1`。

**验收：** 数据集可重复生成；基线可在不访问外部数据库的环境运行；明确区分已有功能测试通过与性能目标尚未实现。

## T02: 收紧编辑器分析缓存生命周期

**文件：** 修改 `src/editor/mod.rs`、`src/editor/tests.rs`；必要时修改 `src/sql/analysis.rs`。

1. 添加 `analysis_cache_does_not_retain_old_revisions`：同一文档执行多次实际文本修改和渲染，断言缓存不随 revision 数量增长。
2. 添加 `closing_editor_releases_analysis_cache`、多文档隔离、同 revision 重绘复用、方言和 highlight_ranges 变化后不错误复用的测试。
3. 运行 `cargo test --lib analysis_cache`，确认新增目标测试在旧实现上因缓存数量或生命周期断言失败，而非编译错误。
4. 最小实现优先保留每个 console 的一个当前分析条目，条目内比较完整 key；同文档 key 改变时替换，关闭 session 时删除。若实测同 revision 多种投影交替导致明显抖动，再采用固定小容量，不提前引入全局 LRU。
5. 避免每次编辑扫描所有文档的缓存。保留惰性计算，不能为了淘汰而主动重新分析全文。
6. 重跑定向测试和现有编辑器/highlight 测试。

**命令：** `cargo test --lib analysis_cache`；`cargo test --lib editor::tests`；`cargo test --test sql_highlight --test app_flow`。

**验收：** 固定文档数量时缓存条目数有固定上限；删除文档释放对应条目；缓存 key 正确性不退化。长时间编辑内存曲线还需与有界撤销历史、其他编辑器状态区分。

## T03: 降低编辑器快照与语句定位成本

**文件：** 修改 `src/editor/mod.rs`、`src/editor/tests.rs`、`src/app.rs`、`src/ui/mod.rs`；检查并复用 `src/sql/analysis.rs`、`src/sql/range.rs` 及现有 scope/行索引实现；测试 `tests/sql_scope.rs`、`tests/ui_render.rs`。

1. 增加 `unchanged_editor_render_reuses_analysis`，断言同 revision 移动光标或滚动不重新构造全文分析。
2. 覆盖多行注释、字符串、Unicode、空文档、末尾换行、超长单行、跨行选区、当前语句下划线和输出区 highlight_ranges。
3. 提供轻量行数/编辑器元信息读取，用于 gutter。不要调用完整快照获取行数。
4. 将全文分析所需的文本、行起始偏移、语句区间和高亮结果绑定当前 revision；先查看现有索引是否可直接复用，避免增加第二套 SQL scope 规则。
5. 渲染时按行偏移直接取范围。高亮有序性先以测试确认，再用二分/区间游标定位可见跨度；不再为每个可见行从头扫描全文和所有跨度。
6. 借用缓存中不可变数据，只为实际输出的可见 spans 分配；必要时采用局部不可变共享，而不是克隆完整高亮数组。
7. 将选区投影限制在与视口相交的行，不遍历整个大选区后再过滤。

**命令：** `cargo test --lib editor::tests`；`cargo test --test sql_scope --test sql_highlight --test sql_completion --test ui_render --test mouse`。

**验收：** 稳定 revision 的渲染不做全文分析；工作量主要随可见内容变化。实际修改后的首次分析仍可为全文 O(N)，本任务不承诺实现增量解析或消除全部按键路径全文复制。

## T04: 持久化快照只读取必要状态

**文件：** 修改 `src/app.rs` 的 `workspace_snapshot`、`persisted_workspace`；测试 `tests/workspace_tabs.rs`、`tests/workspace_persistence.rs`、`tests/profile_lifecycle.rs` 和 `tests/performance_regression.rs`。

1. 补充当前活动工作区覆盖缓存副本、未连接控制台、跨 profile、已关闭控制台、Relation/Dashboard 标签恢复的等价性测试。
2. 构造持有结果的多个 tabs，记录构造持久化快照的分配趋势。对比相同元数据、不同结果行数，不比较两个分别生成的 UUID。
3. 将 `persisted_workspace` 改为接受借用，直接生成持久化 DTO。按 profile 顺序迭代借用，对活动 profile 使用当前 tabs/sql_editors，不克隆或插入完整 ConnectionWorkspace。
4. 仅复制持久化确实需要的字符串、标识和 SQL 文本，不复制 QueryOutcome、结果编辑历史和 Dashboard 历史。
5. 保持现有磁盘格式与迁移读取行为，不借机删除已经服务于持久化数据的兼容逻辑。

**命令：** `cargo test --test workspace_tabs --test workspace_persistence --test profile_lifecycle --test profile_reducer`；重跑 `performance_baseline`。

**验收：** 相同运行时输入产生相同持久化内容；快照构造分配不随结果行数增长，仍允许随 SQL 文本长度和标签数量增长；不引入全局 Arc 改造。

## T05: 保存串行化、增量写入与失败反馈

**文件：** 新建 `src/runtime/workspace.rs`；修改 `src/runtime.rs`、`src/persistence/workspace.rs`、`src/action.rs`、`src/app.rs`；测试 `tests/workspace_persistence.rs`，worker 测试放新模块内部。

1. 为保存 worker 添加测试：连续快照合并、快照与 DeleteSqlFile 顺序、保存失败重试、退出时等待最后保存、旧完成事件不能确认新 revision。
2. 先采用单 worker 串行请求协议。只合并相邻且未开始执行的快照；删除、flush 和 shutdown 是顺序屏障，不能跨屏障合并。
3. 将 SQL 文档 revision 随保存请求传递，与最后成功写入版本比较；失败不更新成功版本。不要仅靠文件修改时间判定变化，也不在热路径为全文做哈希。
4. 首次保存或新文档写入 SQL 文件；内容未变时仅按需更新 manifest。保留原子临时文件替换，不降低现有 durability。
5. 添加保存成功/失败 Action，携带单调保存 revision。用户可见错误要经过清洗，不静默丢弃；退出保存失败必须有明确反馈路径。
6. 保持磁盘格式不变。测试多实例锁和临时文件策略；若当前锁覆盖范围不足，作为独立可靠性修复处理，不能宣称单 worker 解决了跨进程竞态。

**命令：** `cargo test --lib runtime::workspace`；`cargo test --test workspace_persistence --test workspace_tabs --test profile_lifecycle`。

**验收：** 未变 SQL 不重写；队列不会持有无界旧快照；最终保存可等待；错误可观察；删除文件不会被旧快照复活。多文件一致性快照另立迁移方案，本轮不宣称工作区整体原子提交。

## T06: 网格列宽缓存与视口访问

**文件：** 修改 `src/ui/data_grid.rs`、`src/ui/mod.rs`、`src/model/tab.rs`、`src/app.rs`；检查 `src/model/relation.rs`、`src/model/relation_edit.rs`、`src/ui/relation.rs`、`src/ui/dashboard.rs` 的数据变更入口；测试 `tests/ui_render.rs`、`tests/mouse.rs`、`tests/relation_tabs.rs`。

1. 添加 `unchanged_grid_render_does_not_rescan_cells`，首次准备后重复移动选择与重绘，断言不重新扫描整表计算列宽。
2. 覆盖 SQL 基础结果、派生结果、Relation 预览与编辑、Dashboard 进程列表；覆盖刷新、分页、单元格变长、撤销、删除、结果集切换、图标模式和列名变化。
3. 选用 UI 持有的有界布局缓存，或数据状态携带的预计算宽度，实施前根据所有调用点选择一种。不得两套并存；不得仅用结果指针地址作为身份。
4. 缓存身份至少区分 tab、结果来源/结果集、数据 revision 与影响表头的显示配置。原 query generation 若不能覆盖本地编辑，必须补充局部数据 revision。
5. 初次计算沿用现有全量自动宽度语义，不先引入采样造成视觉变化。先允许数据变化时重算整表，稳定后再按修改列失效优化。
6. 直接按视口索引访问底层行，删除每帧全量行引用 Vec；保留行号、滚动条、命中区域和越界保护。
7. 缓存随 tab 关闭、结果替换清理，并限制保留版本数量。

**命令：** `cargo test --lib ui::data_grid`；`cargo test --test ui_render --test mouse --test relation_tabs`。

**验收：** 无数据变更的渲染不扫描全部行；布局输出与现有规则一致；编辑、撤销后不会使用过期宽度。

## T07: 定义结果保留预算与完成语义

**文件：** 修改 `src/db/query.rs`、`src/db/value.rs`；新增内部测试，检查 `src/agent/types.rs` 的既有输出契约。

1. 在实现前固定以下契约：全次执行共享 max_rows/max_retained_bytes；不能按每个结果集各自重置；超预算后保留连续前缀，不跳过超大行后继续挑选小行。
2. 区分统计：已保留行数、已读取行数、affected_rows、结果截断原因、数据库是否正常完成。不要让 truncated=true 被解释为执行失败或数据库已取消。
3. 字节预算明确是保留值的估算还是完整响应字节，不能混用。预算计算使用 checked/saturating arithmetic；覆盖 NULL、Unicode、binary、Unsupported、超长列名和空结果集。
4. 结果集数量与列元数据也需要有界策略，否则很多空结果集仍可无界增长。达到元数据预算后继续消费协议，但显式记录省略，不冒充完整结果。
5. 为累积器添加测试：0 预算、恰好达到上限、超一行、单个超大值、多结果集共享预算、计数溢出、后续 SQL 错误不能被截断掩盖。
6. 先实现有界保留并继续消费执行流，不在本任务添加自动取消。列元数据改为每个结果集初始化一次。

**命令：** `cargo test --lib db::query`；`cargo test --lib db::value`。

**验收：** 在合成事件流中，应用持久保留结果有界；后续 done/error 仍正确处理。必须在文档注明：单个驱动行解码、网络缓冲和服务端内存可能超过预算，这不是进程 RSS 的硬上限。

**待决策门槛：** 新增公共 QueryOutcome 字段或执行方法会影响库调用者，实施前确认接口范围；不要保留无依据的双接口，但也不能未经核查直接破坏已发布消费者。

## T08: 四种适配器接入预算

**文件：** 修改 `src/db/mod.rs`、`src/db/postgres.rs`、`src/db/mysql.rs`、`src/db/sqlite.rs`、`src/db/mssql.rs`、`src/db/transaction.rs`；测试 `tests/postgres_adapter.rs`、`tests/mysql_adapter.rs`、`tests/sqlite_adapter.rs`、`tests/sqlserver_adapter.rs`、`tests/sqlite_transactions.rs`、`tests/sqlserver_transactions.rs`。

1. 先接入 SQLite，在临时数据库验证大 SELECT、多语句、空结果、后续写入成功和后续 SQL 错误可见。
2. SQLx 三驱动复用累积器预算，但保留各自 row 解码。预算已耗尽时避免继续构造不再保留的 CellValue，仍消费流中的行和结束标记。
3. SQL Server 根据 Tiberius 的元数据/行/完成事件接入相同契约，不强行转换成 SQLx 事件模型。
4. MANUAL 事务必须使用同一个物理连接并继续接收完成信息；达到显示预算不触发 rollback、commit 或 OutcomeUnknown。
5. 在隔离数据库验证截断后连接可继续查询、关闭和回滚；验证批处理后续写操作没有因显示上限被跳过。
6. 不用 SQL 字符串拼接 LIMIT 替代驱动层预算，不改写任意用户 SQL。

**命令：** `cargo test --test sqlite_adapter --test sqlite_transactions`；隔离服务配置完成后分别运行 `cargo test --test postgres_adapter -- --test-threads=1`、`cargo test --test mysql_adapter -- --test-threads=1`、`cargo test --test sqlserver_adapter --test sqlserver_transactions -- --test-threads=1`。

**验收：** 四驱动具备相同结果预算测试矩阵；缺失数据库环境导致跳过不计为通过。真实服务测试沿用 CI 的隔离容器，不读取用户连接配置。

## T09: Agent 查询与执行共享资源限制

**文件：** 修改 `src/agent/service.rs`、`src/agent/types.rs`、`src/agent/cli.rs`、`src/agent/mcp.rs`；测试 `tests/agent_service.rs`、`tests/agent_serialization.rs`、`tests/agent_cli.rs`、`tests/agent_mcp.rs`；更新 `docs/coding-agent-access.md`。

1. 补回归测试：query/execute 在预算约束上相同，但 SQL 授权策略仍不同；execute 的多语句后续效果与最终错误仍可观察。
2. 让两入口在执行前传入预算，删除把事后 bound_result 当成内存保护的假象。
3. 抽取两入口真正重复的凭据/连接/执行后关闭流程，保持错误路径关闭；不把 TUI 连接池或手动事务借给 Agent，也不在此引入 MCP 池缓存。
4. 保持现有 JSON 字段语义，新增执行统计必须有序列化契约测试。现有 API 版本与客户端兼容性根据实际变更判断，不自动增加兼容层。
5. 对 max_result_bytes 明确是否限制完整响应。若承诺完整响应上限，使用有上限的序列化写入器做最后检查，包含列名、JSON 转义、binary 膨胀和 envelope；不能只加总 CellValue 的大小。
6. 上限连最小成功响应也无法容纳时返回结构化限制错误，不重复无限缩减/序列化。最终序列化检查是协议保护，不能替代读取预算。

**命令：** `cargo test --test agent_service --test agent_serialization --test agent_cli --test agent_mcp --test agent_policy --test agent_security`。

**验收：** Agent 不再先保留完整大结果才截断；写策略与项目可见性不变；JSON 截断标志真实；资源超限不伪装成成功执行全部结果。

## T10: TUI 接入结果预算与截断提示

**文件：** 修改 `src/runtime.rs`、`src/runtime/transaction.rs`、`src/action.rs`、`src/app.rs`、`src/model/tab.rs`、`src/ui/mod.rs`；如引入可配置预算，同步修改 `src/config.rs`、`config/default.toml`、`docs/configuration.md`；测试 `tests/sql_execution.rs`、`tests/sql_batch.rs`、`tests/transaction_reducer.rs`、`tests/ui_render.rs`。

1. 决定 TUI 默认保留预算。建议候选值为非分页路径 10,000 行/16MiB 应用保留值，需基线与产品确认后固定，不能把候选值视为已批准配置。
2. 对基础、派生、AUTO/MANUAL、分页和非分页执行逐一传递预算；有编辑功能的 Relation 预览也明确受何种预算保护。
3. 分页读取要保留 size+1 探测语义。因字节预算提前停止保留时，不能误判为无下一页或 Exact(total)。保留已消费行统计与截断原因。
4. UI 区分成功且完整、成功但结果截断、执行失败、用户取消。对截断提示提供可执行建议，例如缩小 SELECT 或使用分页，不静默丢行。
5. 保持 connection/tab/source generation 校验，新统计不能让过期结果修改当前页面。
6. 不在本任务自动改为流式 UI；不把现有 unbounded Action channel 直接用于逐行推送。

**命令：** `cargo test --test sql_execution --test sql_batch --test transaction_reducer --test relation_tabs --test ui_render --test connection_switch`。

**验收：** 普通执行不会无界保留结果；分页 next/last 语义不因预算失真；事务状态与风险确认不改变。

## T11: 收敛重复分页执行机制

**文件：** 新建 `src/runtime/query.rs`；修改 `src/runtime.rs`；检查 `src/runtime/transaction.rs`、`src/sql/derived_result.rs` 和现有分页构造模块；测试 `tests/sql_execution.rs`、新模块内部测试。

1. 为普通与派生分页建立同一测试矩阵：目标不匹配、count 失败、末页 offset 修正、空结果、截断、取消、旧 generation。
2. 提取与调用方无关的 count/page 执行和分页统计构造，输入明确的查询计划与预算，输出普通内部结果；Action 封装仍由调用方负责。
3. 不共享调用方特有的 source_generation/target 所有权，不把 MANUAL 事务执行迁出串行 worker。
4. 若唯一复用方式需要复杂泛型异步 executor，先只提取纯分页结果归一化函数，避免抽象成本超过重复代码成本。

**命令：** `cargo test --lib runtime::query`；`cargo test --test sql_execution --test transaction_reducer --test connection_switch`。

**验收：** 相同规则只有一处权威实现；身份校验和事件类型仍明确；外部行为保持不变，不以减少行数作为唯一指标。

## T12: 以一个领域为试点拆分 App

**文件：** 拟新建 `src/app/workspace.rs`；修改 `src/app.rs`；测试 `tests/workspace_tabs.rs`、`tests/workspace_persistence.rs`、`tests/transaction_reducer.rs`、`tests/profile_lifecycle.rs`。

1. 先为工作区切换、新建、关闭、恢复补齐行为测试，尤其是活跃事务退出确认与 profile 删除。
2. 将纯持久化投影和标签元数据操作作为第一批局部函数，接受必要字段的借用，而不是 `&mut App`。
3. 只有确认若干字段具有共同生命周期时才引入 WorkspaceState。不要先把所有 App 字段包进新结构，也不批量重命名现有公共字段。
4. App 继续负责跨领域协调、事务退出确认和副作用命令汇总。工作区模块不访问 Runtime、文件系统、Tokio 或数据库。
5. 第一批完成后评估是否真的缩小依赖。若只是移动 impl App 代码，不继续扩大拆分；后续目录/编辑器 reducer 分离另列任务。

**命令：** `cargo test --test workspace_tabs --test workspace_persistence --test transaction_reducer --test profile_lifecycle --test app_flow`。

**验收：** 新模块可在不构造整个 App 的情况下测试其纯逻辑；不增加全局状态副本；不改变恢复格式和用户交互。

## T13: 回归、性能对比与文档收尾

**文件：** 更新 `docs/performance.md`、`docs/architecture.md`、`docs/coding-agent-access.md`；涉及配置时更新 `docs/configuration.md` 和 `config/default.toml`；按新增测试实际入口调整 `.github/workflows/ci.yml`。

1. 每阶段完成即运行对应测试，不等全部改动完成才回归。
2. 最终执行 `cargo fmt --all -- --check`、`cargo clippy --all-targets --all-features -- -D warnings`、`cargo test --all-targets --all-features`。
3. 使用与 T01 相同环境运行 release 基准，记录前后耗时、访问次数、写入次数、分配/峰值内存；跨平台 RSS 采样采用对应工具，不把不同操作系统结果直接比较。
4. 普通 CI 加入复杂度和上界断言。耗时基准手动/定期运行，暂不增加未评估的 benchmark 生产依赖。
5. 在隔离数据库完成四驱动矩阵。验证无数据预算回归、无凭据输出、无事务不确定状态自动重试。
6. 检查 docs 中已落后的描述，尤其是不能再把已实现分页写成完全缺失，也不能把有界保留写成完整流式 UI。

**交付清单：** 变更清单、确实运行的命令及结果、未运行项及原因、基准对比、接口/配置变化说明、剩余限制。

## 3. 统一验收矩阵

| 维度 | 必须达成 | 不作错误承诺 |
| --- | --- | --- |
| 编辑器缓存 | 条目数不随历史 revision 增长，删除时清理 | 不是证明所有编辑器内存泄漏已消除 |
| 编辑器渲染 | 同 revision 复用分析，可见区间索引访问 | 首次修改后分析不一定增量 |
| 网格 | 稳定结果的选中/滚动不全表扫描 | 初次自动列宽仍可 O(R*C) |
| 保存 | 不复制结果数据，不重写未变 SQL，错误可见 | 不是跨文件事务或多实例并发修复 |
| 查询保留 | 预算在读取阶段生效，多结果集共享 | 不是服务端工作量或 RSS 硬上限 |
| Agent 响应 | query/execute 统一资源契约，JSON 边界明确 | 截断不能隐含取消/回滚 |
| 架构复用 | 公共机制一处实现，策略和原生语义保留 | 不是为了短文件做机械搬迁 |

## 4. 风险门槛与后续事项

1. **取消优化单独评审。** 若需要限制数据库执行时间/传输量，应复核四驱动原生取消、确认和断连策略；仅 abort Tokio task 不构成取消已确认。写批处理不得因显示预算自动取消。
2. **流式 UI 由实测驱动。** 有界保留之后若首屏等待仍不可接受，再设计有界批次通道、批次级 generation、关闭 tab 清理和渲染合帧；禁止逐行 Action 洪泛。
3. **大单元格限制有层次。** 应用可能必须先接收/解码完整单行才能判定预算；如这是主要峰值，另评估驱动能力、值预览或显式大字段读取策略。
4. **持久化格式迁移独立授权。** 版本化 SQL 路径、原子 manifest 切换及崩溃恢复可以后续实施，但需要明确回滚与旧文件清理规则。
5. **并行实施边界。** T02/T03 与其他编辑器计划串行整合；T06 与表格 UI 计划串行整合；T04/T05 共享 App/Runtime，避免同时编辑。测试数据准备和只读验证可独立并行。
6. **失败时保留行为。** 性能改动可按逻辑单元回退，但不得用默认关闭结果限制掩盖资源风险；涉及发布接口时先修复兼容问题，不盲目恢复不安全执行路径。

## 5. 工作量估算

以下为一名熟悉 Rust 和本项目的工程师的粗估，不是交付承诺；外部数据库环境准备、并行功能冲突和接口决策另计。

| 工作包 | 粗估 |
| --- | --- |
| T01 基线 | 1-2 人日 |
| T02-T03 编辑器 | 2-4 人日 |
| T04-T05 持久化 | 3-5 人日 |
| T06 网格 | 1-3 人日 |
| T07-T10 结果预算及四驱动/两入口 | 5-9 人日 |
| T11-T12 局部结构收敛 | 2-4 人日 |
| T13 综合验证与文档 | 1-2 人日 |

建议第一批仅承诺 T01、T02、T04、T06 的交付，在测量收益和测试稳定后继续 T03/T05 与结果预算阶段。结果预算设计 T07 可提前启动，但不绕过执行语义评审直接修改四驱动。
