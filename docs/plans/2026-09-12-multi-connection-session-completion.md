# 多连接会话管理与 Explorer 隔离 Implementation Plan

> 执行说明：逐任务实施，先验证行为测试失败，再实现并运行定向检查。本文接口和新增测试名称均为拟定设计，不代表已经存在。提交只在用户明确要求后进行；不依赖额外技能或子代理。

**Goal:** 打开第二个连接后，第一个连接保持在线、目录保持展开且仍可操作；不同目标的查询、目录请求、事务及连接生命周期互不干扰。

**Architecture:** 保留 Action → App::update → Command → Runtime 边界与已实现的全局 Tabs。App 使用按完整 ExecutionTarget 索引的会话注册表作为在线状态权威来源，Runtime 管理相应资源；Explorer 选择和活动 Tab 仅表达用户焦点，所有异步请求按会话身份和请求身份路由。

**Tech Stack:** Rust 2024、Tokio、Ratatui/Crossterm、现有数据库适配器、现有 SQLite 集成测试；无需新增生产依赖。

---

## 1. 当前基线与实施范围

分析基线：`cd7a2c7`，已包含 `de71087 feat(workspace): support concurrent database consoles`。行号仅供定位，以实施时符号定义为准。

| 现有代码 | 事实与缺口 |
| --- | --- |
| `src/runtime.rs` 的连接 Map | 已允许多个 ConnectionKey 同时存在 |
| `src/app.rs:9051` | B 成功后调用 clear_profile_catalog，将 A 设为 Offline |
| `src/app.rs:15592` | 清理函数删除展开节点、目录与待处理请求 |
| `src/app.rs:229` | App 仍只有一个 ConnectionState |
| `src/app.rs:12386` | 请求连接使用单个 pending 槽和全局运行中检查 |
| `src/app.rs:8913` | 成功事件使用全局 terminal generation 拒绝旧结果 |
| `src/app.rs:14078/14476/14736` | 目录请求、成功、失败依赖全局活动连接 |
| `src/runtime.rs:1438` | 安装连接时全局清空 known_relations/known_relation_targets |
| `src/runtime/connections.rs` | single-flight 以包含 generation 的 key 去重，仍有全局最高 generation |
| `tests/connection_switch.rs:1196` | 旧测试明确要求打开 B 后 A Offline、目录清空和折叠 |

已运行的基线：`profile_root_successful_switch_clears_old_catalog_and_syncs_target` 为 PASS。这证明旧行为受到测试保护，不表示新多连接需求已满足。

本计划补齐现有多连接实现，沿用 `2026-09-12-multi-connection-consoles.md` 中会话隔离的目标。文档格式升级、Console 管理器扩展等已有工作不重新设计；持久化只验证多连接改动没有损坏已有文档恢复。

## 2. 必须成立的行为契约

1. 连接 B 不改变 A 的会话身份、连接状态、目录数据、展开状态和事务。
2. 已在线目标再次打开只导航/复用，不重新拨号；同目标 Connecting 时合并等待者。
3. 不同目标可并行连接与执行；同目标真实事务约束仍由现有事务机制执行。
4. 后台成功、失败和分页事件不切换活动 Tab，不改写文档绑定。
5. A/B 连接成功结果乱序时，两个有效 attempt 都被接受。
6. 旧 generation 结果必须拒绝，但判定范围是所属目标，不是全局最新连接。
7. 显式断开 profile 关闭其所有目标与 attempt，保留文档，不影响其他 profile。
8. 删除/修改 profile、连接失效、取消和退出都使用相同的身份隔离原则。
9. 同 profile 不同 database/schema 初版使用完整目标隔离，不用共享连接上的 USE/search_path 模拟切换。
10. 在线状态和资源句柄不持久化；重启恢复文档，不恢复虚假的 Connected 状态。

## 3. 关键设计决定

### 3.1 App 会话状态

新增 `src/model/session.rs`，公开最小必要接口：

| 拟定接口 | 语义 |
| --- | --- |
| `ensure(target)` | 返回 Ready(identity)、Waiting(identity) 或 Start(identity) |
| `session_for_target(target)` | 查该完整目标的状态；不读取活动 Tab |
| `session_for_identity(identity)` | 查仍有效的在线会话或正在连接的 attempt |
| `accept_success(identity, server, capabilities)` | 仅接受该目标当前 attempt |
| `accept_failure(identity, error)` | 只更新该 attempt，返回对应等待者 |
| `retire_profile(profile_id)` | 返回该 profile 全部在线身份和未完成 attempt |

注册表以 `HashMap<ExecutionTarget, SessionState>` 存储。generation 由应用级 checked_add 分配，身份全局唯一；分配器不参与跨目标事件新旧比较。身份到目标的反查可先采用遍历，避免第二张可写索引；后续有测量依据再优化。

每个目标保存在线会话、可选连接 attempt、错误与耗时。需要显式重连时旧在线会话与候选 attempt 可暂时共存，失败保留旧连接；成功后定向替换。server、能力和 owner context 必须有明确会话归属。

App 不重新保存 Runtime 的资源句柄；Runtime 仍在配置变更互斥边界内检查 profile revision。若事件需要携带 revision，须统一 Action/Command 和测试构造入口，不能创建两套不同含义的版本计数器。

### 3.2 Runtime 资源所有权

推荐 Runtime 以 ExecutionTarget 索引已安装资源，每个值携带 identity；按 identity 查询时同时验证目标和 generation。若保留当前 ConnectionKey Map，必须集中实现“同目标替换旧 generation”，禁止散落的任意 find/remove。

成功安装流程：验证 attempt 与 profile revision → 原子取出同目标旧资源并安装候选 → 释放锁 → 发出成功事件/关闭已退休资源。具体事件顺序要保证收到成功后可按该身份立即查询。所有网络 probe、close 和 worker 等待均在注册表锁外进行。

失败、缺凭据、目标非法、取消、超时和过期成功都必须完成 attempt 清理。过期成功只关闭本次候选，不关闭共享或复用的有效资源。连接超时的资源回收需根据各适配器所有权路径核查并测试。

### 3.3 Explorer 的 profile 与 session 关系

Explorer 目录继续按 profile 展示，显式记录每个 profile 的 `catalog_connection`，不通过 HashMap 任意找到一个同 profile 会话。

- 初次打开 profile 时选其请求目标的有效会话用于目录。
- Console 切换目标不自动切换目录会话。
- 目录会话退休时，从仍有效的同 profile 会话中按稳定目标顺序选择替代者，或按需连接 profile 默认目标。
- 只有目录会话变化、配置影响目录或用户明确刷新时，才推进该 profile 的 catalog epoch。
- epoch/请求失效只影响对应 profile；展开状态与缓存能保留时保留，并明确标记过期。
- profile 状态由会话集合聚合：存在可用在线会话时保持在线；其他目标连接失败作为局部错误展示。目录加载状态单独表达，不能让 B Linking 把 A 禁用。

`CatalogTarget::Databases` 不含足够的 profile 上下文，修改 start_catalog_request 接口显式传入目录会话身份。后续 schemas/groups/objects/分页请求继承原请求身份。

### 3.4 执行与等待者

`pending_execution` 改为按 Console UUID 管理，保留请求 UUID、SQL/目标快照、文档与绑定 revision。成功后根据等待者身份继续，禁止 take 全局单槽或依赖 active_tab。一个目标连接成功可以唤醒多个 Console；任一 Console 关闭、取消或重绑只取消自己的等待者。

交互凭据仍串行显示，但按 attempt 排队；同 profile 可按现有凭据策略复用获取结果，不能把 B 的密码响应应用到 A。

## 4. 任务依赖

```text
T01 基线与契约
  → T02 App 注册表
  → T03 Runtime 资源与 attempt
  → T04 连接事件和凭据路由
  → T05 Explorer 请求/回调
  → T06 元数据缓存/搜索/补全
  → T07 Console 与待执行
  → T08 Relation/Dashboard/目录编辑
  → T09 断开/配置变更/事务/退出
  → T10 UI 与遗留状态收尾
  → T11 集成验收与文档
```

任务会集中修改 `src/app.rs` 和 `src/runtime.rs`，按上述顺序实施。每个任务内部的小步均为：增加一个行为用例 → 运行确认预期失败 → 最小修改 → 定向验证；每次只迁移一个调用入口或事件分支。中间兼容层可编译，但完整功能在 T11 验收后才算交付。

## 5. 详细任务

### T01：建立基线，标出冲突测试

**文件：** 修改 `tests/connection_switch.rs`；新增 `tests/multi_connection.rs`；实施记录写入本文末尾。

1. 运行 `cargo test --test connection_switch --test catalog_reducer --test workspace_tabs --test profile_lifecycle --test transaction_reducer`，记录通过数和既有失败。
2. 列出“running SQL 阻止切换”“清空旧目录”“隐藏工作区”“全局最新 generation”对应测试，逐项区分新需求替换和仍需保留的安全检查。
3. 将旧目录清理用例改成 `opening_second_profile_preserves_first_catalog_and_expansion`。
4. 用实际 CatalogPage fixture 让 A 具有非空目录，记录根节点和子节点展开状态，随后连接 B。
5. 断言 A 目录内容、展开集合和身份未变，且没有 Disconnect(A) 命令。
6. 运行 `cargo test --test connection_switch opening_second_profile_preserves_first_catalog_and_expansion -- --exact`，预期 FAIL 在 A 状态/展开断言，而不是 fixture 构造错误。

**完成标准：** 原始问题成为稳定失败的行为测试；尚未修改生产语义。

### T02：增加目标会话注册表

**文件：** 新增 `src/model/session.rs`；修改 `src/model/mod.rs`、`src/model/workspace.rs`、`src/app.rs`；测试放在新模块和 `tests/multi_connection.rs`。

1. 增加状态机测试：A/B 独立 ensure，同目标重复 ensure，共用目标的等待者。
2. 增加 A1/B2 乱序成功与 A1/A3 同目标旧结果拒绝测试。
3. 增加重连失败保留旧在线会话、generation 耗尽不回绕测试。
4. 运行 `cargo test --lib model::session`，逐个确认新增失败原因。
5. 实现注册表和前述查询/转换接口，将 App 的 generation 分配迁入注册表。
6. 暂时保留旧 ConnectionState 的只读展示兼容入口；只允许集中投影更新，不允许新功能再向旧字段写入独立权威状态。
7. 运行 `cargo test --lib model::session`，预期全部 PASS。

**完成标准：** 目标会话状态可脱离 Tab、Explorer 和 Runtime 独立验证；过期判定不依赖全局 terminal generation。

### T03：修正 Runtime single-flight 与资源替换

**文件：** 修改 `src/runtime/connections.rs`、`src/runtime.rs`；测试 `tests/multi_connection.rs`、`tests/profile_runtime.rs`、`tests/connection_switch.rs`。

1. 增加 Runtime 测试：同目标不同 generation 的重复普通连接请求不造成两次安装；显式重连能定向替换。
2. 使用两个 SQLite 内存 profile：在 A 创建 marker，连接 B 后仍按 A 原身份查询 marker。
3. 增加同目标新 generation 安装后，旧身份命令被拒绝且 B 仍可查询的测试。
4. 以目标为单位跟踪 attempt，区分普通复用和显式重连；移除全局 highest generation 对无关目标的限制。
5. 集中实现 take/replace/retire，枚举同目标旧资源；锁外关闭资源。
6. 逐个覆盖 connect 的提前 return 路径，保证 finish/cancel 清理；缺凭据后的恢复必须有明确新 attempt 或等待状态。
7. 为可控连接测试增加最小私有测试接缝或复用现有 fixture，用 barrier/oneshot 控制乱序和取消；不依赖随机 sleep。
8. 运行 `cargo test --lib runtime::connections` 和 `cargo test --test multi_connection --test profile_runtime --test connection_switch`。旧契约待迁移失败要记录，不能削弱身份隔离断言求全绿。

**完成标准：** A/B 底层资源互不替换，同目标不会积累旧 generation 连接，取消和过期候选不泄漏。

### T04：连接请求、结果和凭据按会话路由

**文件：** 修改 `src/app.rs`、`src/action.rs`、`src/model/workspace.rs`、`src/runtime.rs`；测试 `tests/multi_connection.rs`、`tests/credential_resolution.rs`、`tests/connection_switch.rs`。

1. 增加 A/B 同时连接、B 先成功、A 后成功的 reducer 测试，断言两者在线。
2. 增加 B 失败不改变 A、连接成功不抢焦点/改 Console 绑定的测试。
3. 将 request_connection/request_connection_target_inner 接入 ensure；已在线返回复用结果，不发 Connect。
4. 将 ConnectionSucceeded/Failed/Invalidated/CredentialsRequired 改为查注册表匹配 identity；删除跨目标全局 terminal generation 判定。
5. 删除成功分支对其他 profile 的 clear_profile_catalog 调用。
6. 拆开“用户打开文档/导航”与“连接完成”：activate_profile_workspace 不再由后台连接成功隐式调用；需要打开页面的用户动作在发起阶段完成。
7. 凭据提示增加 attempt 归属队列；取消一个提示只处理其等待者。
8. 运行 `cargo test --test multi_connection --test credential_resolution --test connection_switch`，验证已迁移用例。

**完成标准：** 连接结果只更新所属会话；单 pending 槽不再丢失其他连接尝试。

### T05：Explorer 请求与回调完成多连接隔离

**文件：** 修改 `src/app.rs`、`src/model/explorer.rs`、`src/model/workspace.rs`；测试 `tests/catalog_reducer.rs`、`tests/explorer_state.rs`、`tests/connection_switch.rs`、`tests/multi_connection.rs`。

1. 增加 A 目录请求在 B 成功后返回仍被接受的测试，同时覆盖失败和 continuation。
2. 增加当前 Tab 属于 B 时展开/刷新 A 的测试，断言 Command 携带 A identity。
3. 增加同 profile 两个目标时目录会话选择稳定，Console 切换不刷新整棵树的测试。
4. 按 3.3 节保存目录会话身份，为 start_catalog_request 增加显式身份参数；所有递归分页继承它。
5. accept_catalog_page/fail_catalog_page 校验有效会话、目录会话、epoch 和 request key，删除 active_identity 比较。
6. 更新 RequestProfileConnect、节点展开、刷新和数据库选择入口，已在线目标只复用。
7. 仅针对目录会话退休/配置变化处理 epoch 和局部加载状态；普通 A/B 导航不清理目录。
8. 运行 `cargo test --test catalog_reducer --test explorer_state --test connection_switch --test multi_connection`。

**完成标准：** T01 原始复现测试 PASS；A/B 均能独立展开、分页、刷新和接收后台结果。

### T06：元数据缓存、搜索和补全隔离

**文件：** 修改 `src/runtime.rs`、`src/app.rs`、`src/model/workspace.rs`、`src/sql/diagnostics.rs`；测试 `tests/catalog_reducer.rs`、`tests/sql_completion.rs`、`tests/sql_diagnostics.rs`、`tests/relation_runtime.rs`。

1. 增加连接 B 后 A 已知关系仍可用于请求校验的测试。
2. 增加 A 搜索进行中打开 B 不取消 A、旧 A 搜索结果不能覆盖 B 当前搜索展示的测试。
3. 将 known_relations/known_relation_targets 的 clear 改为按退休 identity 定向删除；latest_catalog_requests 同样按所属身份处理。
4. 将 catalog_search_task 单槽迁移为按请求所属会话/搜索会话管理，前台只展示当前搜索请求。
5. 补全索引按 profile/目标维护或从明确目标目录建立；禁止用最后返回目录覆盖所有 Console 的补全上下文。
6. 诊断请求和缓存键带足目标与文档 revision，后台结果不依赖活动 Tab。
7. 运行 `cargo test --test catalog_reducer --test sql_completion --test sql_diagnostics --test relation_runtime`。

**完成标准：** B 连接/刷新不会破坏 A 的关系解析、搜索、补全或诊断上下文。

### T07：Console 执行与多等待者

**文件：** 修改 `src/app.rs`、`src/model/pending_execution.rs`、`src/model/tab.rs`、`src/sql/execution.rs`；测试 `tests/multi_connection.rs`、`tests/sql_execution.rs`、`tests/workspace_tabs.rs`。

1. 增加 A 执行期间连接 B 并执行，B 先结束、A 后结束，各结果归位且焦点不变的测试。
2. 增加两个 Console 等待同目标一次连接、成功后各执行一次的测试。
3. 增加等待中关闭/取消/重绑一个 Console 不影响另一个、修改文本不改变待执行 SQL 快照的测试。
4. run_active_sql 在捕获 Console 目标后转为对象级处理，在线走 session_for_target，离线创建等待者再 ensure。
5. pending_execution 改为按 Console 管理；唤醒按 identity/请求 UUID/绑定 revision 检查，不使用 active_tab 过滤。
6. 将 has_running_query 等全局阻塞缩小到本 Console 或真实共享资源；保留事务规定的禁止操作。
7. 分页、取消、执行确认、错误定位和事务模式使用原请求身份；网络错误不自动重放 SQL。
8. 运行 `cargo test --test multi_connection --test sql_execution --test workspace_tabs`。

**完成标准：** 前台 Tab 和后台执行完全解耦，按需连接不会丢等待者或误执行新编辑文本。

### T08：Relation、Dashboard 与目录编辑路由

**文件：** 修改 `src/app.rs`、`src/model/relation.rs`、`src/model/dashboard.rs`、`src/model/catalog_editor.rs`；测试 `tests/relation_tabs.rs`、`tests/relation_runtime.rs`、`tests/catalog_editor_reducer.rs`、`tests/catalog_mutation.rs`。

1. 增加 A Relation 分页/DDL 加载中切换到 B，结果仍更新 A 的测试。
2. 增加 A Dashboard 定时响应不使用 B identity，后台响应不切焦点的测试。
3. 增加 A 目录编辑器在 B 活动时加载定义、owner context 和提交命令仍绑定 A 的测试。
4. 逐调用点替换 database_command_identity 和 self.connection.target，从对象固定目标获取会话。
5. relation_context、事务创建、目录变更确认沿用同一身份和目标快照，不从当前连接补齐。
6. 能力/owner context 缓存按会话与数据库目标归属；失效时仅使相关编辑器进入可恢复状态。
7. 运行 `cargo test --test relation_tabs --test relation_runtime --test catalog_editor_reducer --test catalog_mutation`。

**完成标准：** 所有数据库编辑与查看页面均有独立归属，不借用当前活动连接执行。

### T09：断开、配置变更、事务与退出

**文件：** 修改 `src/app.rs`、`src/runtime.rs`、`src/runtime/connections.rs`；测试 `tests/profile_lifecycle.rs`、`tests/profile_runtime.rs`、`tests/transaction_reducer.rs`、`tests/quit_transaction_review.rs`、`tests/multi_connection.rs`。

1. 增加 A 含多个目标时显式断开全部关闭，B 查询和事务仍可用的测试。
2. 增加 A Connecting 时删除/修改 A 配置，迟到成功不能重新安装的测试。
3. 增加旧 DisconnectCompleted/Invalidated 不影响新 generation，后台 A invalidation 仍正确处理 A 的测试。
4. request_profile_disconnect/retire_profile_connections 枚举注册表而不是从活动连接或打开 Tab 反推会话。
5. Runtime disconnect 枚举匹配资源与 worker，不能只 find/remove 第一个；取消仅对应会话的目录搜索和请求。
6. 事务退出检查限定目标 profile；应用退出检查全部 Tab 和 worker。复用现有提交/回滚交互，失败保留准确状态。
7. 配置重命名只更新展示；实质连接配置变化退休该 profile 所有目标并清理对应缓存/attempt。
8. shutdown 等待正在连接的候选、查询取消和全部资源关闭；用可控 fixture 验证完成后没有残留安装。
9. 运行 `cargo test --test profile_lifecycle --test profile_runtime --test transaction_reducer --test quit_transaction_review --test multi_connection`。

**完成标准：** profile 生命周期涵盖没有打开 Tab 的会话；局部断开不会跨连接影响事务，退出无遗留 worker。

### T10：UI 状态和遗留单连接语义收尾

**文件：** 修改 `src/app.rs`、`src/model/workspace.rs`、`src/ui/mod.rs`、`src/ui/dashboard.rs`；测试 `tests/ui_render.rs`、`tests/mouse.rs`、`tests/workspace_tabs.rs`、`tests/workspace_persistence.rs`。

1. 增加 A 在线、B Linking/Failed 的渲染测试；A 仍显示在线，A 内容可操作。
2. 增加活动 Tab A 与 Explorer 选择 B 不一致时，目标栏/状态栏准确表达各自上下文的测试。
3. 根据注册表聚合 Explorer 状态，文档目标状态按该 Tab target 查询。
4. 更新 DisconnectedWorkspace 判断：离线活动文档不代表整个应用零连接，零 Tab 也不代表没有会话。
5. 移除旧 connection pending/terminal/计时字段与 database_command_identity；如保留 connection 展示 API，必须是明确的派生只读视图。
6. 审计 activate_profile_workspace/workspaces 缓存对多连接 Tab 的影响，保留必要恢复逻辑，禁止后台连接成功补建、重绑或激活文档。
7. 验证已有持久化往返保留跨 profile 文档；不保存会话在线状态和临时 generation。
8. 运行 `cargo test --test ui_render --test mouse --test workspace_tabs --test workspace_persistence`。

**完成标准：** UI 和 Runtime 对在线目标的理解一致；无业务路径继续把全局当前连接当作资源权威来源。

### T11：集成验收、文档与交付

**文件：** 完善 `tests/multi_connection.rs`、`tests/connection_switch.rs`；更新 `docs/architecture.md` 和本文实施记录。

1. 运行 A/B SQLite marker 集成场景：两连接、独立写读、B 失败、A 断开、B 继续查询。
2. 通过可控事件 fixture 跑完连接乱序、目录乱序、同目标 single-flight、过期成功、取消与退出；不把 reducer 的模拟成功当作 Runtime 资源验收。
3. 检查旧单连接测试已逐项替换，保留目标错配拒绝、旧身份拒绝和事务约束测试。
4. 运行 `cargo fmt --check`。
5. 运行 `cargo test --lib --tests`。真实数据库/Oracle 客户端相关环境要求按仓库现有测试配置处理，分别记录通过、跳过和环境失败。
6. 运行 `cargo clippy --all-targets -- -D warnings`；既有 lint 与新问题分开记录。
7. 运行 `cargo check --no-default-features`，验证会话逻辑不依赖 Oracle 默认特性。
8. 用 `cargo run` 手工操作：展开 A 至表节点 → 打开 B → 在 A/B 间展开、查询和切 Tab → B 失败 → 显式断开 A。记录界面状态和实际查询结果。
9. 在可用的真实 PostgreSQL/MySQL/Oracle 等环境验证同 profile 多 database/schema 目标；无法执行的驱动场景明确标为未验证。
10. 更新 architecture：会话权威来源、目标/身份校验、目录会话选择、断开粒度和异步结果归属。检查最终 diff 没有凭据、临时输出或无关文件。

**完成标准：** 下列验收矩阵全部有测试或明确的真实驱动验证记录；原始 cargo run 场景通过。

## 6. 最终验收矩阵

| 场景 | 必须观察到的结果 | 主验证位置 |
| --- | --- | --- |
| A 在线后打开 B | A 在线、目录和展开状态保留 | connection_switch / UI |
| 当前 Tab B，刷新 A | 命令使用 A identity | catalog_reducer |
| A 目录晚于 B 返回 | A 目录更新，B 不变 | catalog_reducer |
| A/B 连接乱序 | 两个有效会话均被接受 | session / multi_connection |
| 同目标重复请求 | 一次连接，多个等待者 | runtime::connections / multi_connection |
| A/B SQL 并发 | 各结果回到原 Console | sql_execution / multi_connection |
| 同 profile 不同目标 | 独立路由，不修改共享 schema 状态 | Runtime + 可用真实驱动 |
| B 失败或凭据取消 | A 继续查询、展开和使用事务 | credential_resolution / multi_connection |
| 重连同目标 | 旧身份失效、旧资源退休 | Runtime 测试 |
| 断开 A 全部目标 | B 不受影响，A 文档保留 | profile_lifecycle |
| A 配置变更且旧成功迟到 | 候选关闭，不复活旧配置 | profile_runtime |
| 后台连接失效 | 只影响所属会话与文档 | transaction_reducer |
| 应用退出 | 全部会话、attempt、worker 收尾 | Runtime / quit_transaction_review |
| 重启恢复 | 文档目标保留，在线状态重新建立 | workspace_persistence |

## 7. 交付检查点

- **M1（T01–T04）：** 完成。多目标连接状态与底层资源一致，乱序连接正确。
- **M2（T05–T06）：** 完成。Explorer 目录请求和连接缓存按 profile/session 隔离。
- **M3（T07–T09）：** 完成。查询等待、Relation/Dashboard 回调、profile 断开和事务身份隔离已覆盖。
- **M4（T10–T11）：** 完成自动化验收和文档；真实外部数据库驱动并发运行需在配置了相应服务的环境另行验收。

每个检查点记录变更文件、测试命令与结果、未验证驱动环境。需要提交时按检查点或更小的可独立通过单元组织提交，不将仍依赖旧全局连接的中间状态描述为“完整支持多连接”。

## 8. 实施记录

- 计划创建：完成代码分析；仅已运行原有目录清理测试，结果 PASS。
- T01：完成。将旧“切换时清空 A 目录”测试替换为保留状态测试；先观察到 `Offline != Online` 的预期失败，再移除 B 成功时清理旧 profile 的逻辑。复核：新测试 PASS，`cargo test --test connection_switch` 31/31 PASS，`git diff --check` PASS。目录数据断言也已加入。
- T02：完成。新增 `src/model/session.rs`，在线会话与连接 attempt 分开存储；支持按目标 single-flight、reconnect 失败保留旧在线会话、identity retirement tombstone、profile 范围枚举。
- T03：完成。attempt 的最新判断改为按完整 `ExecutionTarget` 隔离；同目标新 generation 会取消旧 attempt；安装连接时只替换同目标旧资源，并按旧 identity 定向清理缓存。连接提前返回路径补齐 attempt finish。复核：runtime connections 4/4 PASS，connection_switch 31/31 PASS，`cargo check`、fmt 和 diff check PASS。
- T04：完成 session request/result 路由；新请求复用目标已连接 session；后请求的 pending 优先保留全局连接 projection，旧 success 不抢占；旧 disconnect 不关闭/清除同 profile 新 generation。
- T05：完成 profile→catalog session 绑定，catalog callback 与搜索结果按 identity/request/epoch 校验，连接成功不再清空已加载目录。
- T06：完成基础缓存隔离。连接安装不再清空全局关系/目标缓存；同目标替换时只清理被替换 identity 的 known relations、targets 和 latest catalog requests。复核：catalog_reducer 36/36、connection_switch 32/32、sql_completion 156/156、sql_diagnostics 5/5 PASS。
- T07：完成。pending execution 按 Console UUID 存储，成功后按目标、文档 revision、事务 generation 校验并分别 dispatch；已连接 Console 可在另一目标连接 pending 时继续执行；关闭 Console 只移除自己的等待项。
- T08：完成。Relation、Dashboard、derived query、catalog search、owner/事务回调以对象绑定 identity 校验，不要求等于全局 active identity。
- T09：完成。断开按 identity 清理匹配资源、事务、搜索任务和缓存；profile exit check 限定 profile。修复 Runtime profile 顺序重复、ProfileStore 已知 profile 重复追加/空集合保存，并保留未知 profile 表与未来字段。
- T10：完成兼容投影收尾。Explorer 不再在 session 安装时清空目录，Disconnected state 检查 SessionRegistry；`ConnectionState` 仅作当前选中连接投影，异步结果以 session identity 为准。
- T11：完成。全量 `cargo test --lib --tests` 通过；测试框架报告的 ignored 项为显式要求真实数据库或 release 性能环境的用例。`cargo clippy --all-targets -- -D warnings`、`cargo check --no-default-features`、`cargo fmt --check`、`git diff --check` 全部通过。SQLite A/B Runtime 生命周期有集成覆盖；真实 PostgreSQL/MySQL/Oracle 并发目标切换未在本环境运行，仍需用户具备对应服务器后验收。
