# OpenCode V1/V2 MCP Compatibility Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.
> 执行环境没有该技能时，按本文件的任务顺序执行，并逐项记录验证结果。

**Goal:** 修复 LazyDB MCP 的启动诊断与 OpenCode V2 配置适配，同时保留 OpenCode V1 接入能力。

**Architecture:** 继续共用标准 rmcp stdio server，在客户端配置边界实现双格式读取、单格式写入；setup 和 doctor 共享格式解析及有效配置计算。通过显式 MCP probe 区分配置、二进制启动、握手和工具发现问题，并以独立客户端测试验证 V1、V2 native tools 和 V2 Code Mode。

**Tech Stack:** Rust 2024、clap、serde_json、jsonc-parser CST、tokio、rmcp 3.1.x、tempfile、现有 Rust 集成测试。

---

## 1. 已确认事实与实施边界

2026-09-12 调查结果：

- 本机 `opencode --version` 与 `opencode2 --version` 均为 `opencode v2.0.2`，不能按命令名称推断主版本。
- PATH 中 `/Users/yelog/.local/bin/lazydb` 为 0.1.4。读取当前 profiles 时因 `kind = "oracle"` 不受支持退出，发生在 MCP initialize 之前。
- 同一二进制使用临时空配置（当时 profiles version 为 6）后，成功协商 `2025-06-18`，并返回 7 个工具。
- 当前源码使用 rmcp；setup/doctor 固定读取 `mcp.lazydb`，未完整识别 V2。
- V2 官方承诺支持受支持的 V1 配置；V1 旧格式不是已证实的必然失败原因。
- 本机没有确认可用的独立 V1 客户端，不能把协议探测成功等同于双客户端实测通过。

本次实施不把“未知数据库类型容错”混入 MCP 配置适配。启动失败先通过正确构建和二进制路径解决；未来数据库类型的持久化前向兼容应沿用独立的 profiles 兼容性设计，避免在 MCP 路径静默丢弃连接。

## 2. 目标行为与关键决策

### 2.1 配置格式与运行时版本分开建模

- `OpenCodeFormat = Auto | V1 | V2`：控制 setup 写入格式。
- `OpenCodeRuntime = V1 | V2 | Unknown`：控制 doctor 的配置发现和覆盖语义。
- V1 格式不意味着 V1 运行时，V2 也可加载 V1 配置。
- V2 原生路径：`mcp.servers.lazydb`；V1 路径：`mcp.lazydb`。
- 同一文档同时定义两种格式时，按官方规则处理有效的 V2 值优先；保留来源信息并展示 shadowed 条目。
- 对格式非法、V2 同名值无效等边界，先按官方迁移文档及实际 V2 行为编写 fixture，不自行猜测回退规则。

### 2.2 setup 写入决策表

| 输入情况 | auto 行为 |
| --- | --- |
| 已有 LazyDB V1 定义 | 保留 V1；检测与期望命令是否等价 |
| 已有 LazyDB V2 定义 | 保留 V2；检测与期望命令是否等价 |
| 同时存在 V1/V2 定义 | 报告生效项及 shadowed 项，不再添加第三份定义 |
| 没有 LazyDB，但已有明确 `mcp.servers` 结构 | 在 V2 路径添加 |
| 没有 LazyDB，但已有 V1 MCP 服务结构 | 在 V1 路径添加 |
| 空配置或无法判断格式，检测到 V2 | 写入 V2 |
| 空配置，检测到 V1 或运行时未知 | 写入 V1；未知时注明采用兼容格式 |

新增 `--opencode-format auto|v1|v2`。显式格式与已有 LazyDB 格式冲突时返回可理解的冲突信息，不借 setup 自动搬移、删除或复制现有条目。显式 `v1` 适用于需要同一文件供 V1/V2 使用的场景。

保留现有 JSONC 注释、其他服务器和幂等性。保留用户的环境变量、timeout、codemode 和启停状态。

### 2.3 客户端检测与二进制选择

- setup/doctor 接受 `--opencode-bin <path-or-command>`，默认 `opencode`；它仅用于版本识别，不执行交互式安装或登录。
- 版本探测使用参数数组执行 `--version`，有限超时（建议 2 秒）、有限输出，并解析语义版本；不以 `opencode2` 名字推断 V2。
- 探测失败得到 `Unknown`，不得导致可解析配置的静态检查整体失败。
- setup 增加 `--server-bin <path>`，用于把经过验证的 LazyDB 可执行文件写入 command；默认保持 `lazydb`。
- 指定路径应解析到实际绝对路径并检查文件存在；不要求已有的用户配置必须与当前进程路径相同。
- 用 `--server-bin` 修复旧安装时，已有不同命令仍报告 conflict，展示建议值供用户修改，遵循当前 setup 冲突处理约定。

### 2.4 doctor 与 probe 的语义

静态 doctor 只回答配置的发现、生效、可启动形状和写策略信息；没有运行握手就不报告 connected。

`--probe` 启动最终选中的本地 LazyDB MCP 命令，执行：

```text
resolve configuration
  -> resolve executable / cwd / environment
  -> spawn
  -> initialize
  -> notifications/initialized
  -> tools/list
  -> graceful close, then bounded cleanup
```

- 默认不调用任何数据库工具；`tools/list` 不需要数据库 I/O。
- 配置设置 disabled 时报告 skipped；remote 类型报告当前 probe 不支持，不把它当成本地命令执行。
- 已有配置来源不确定、无法解析环境替换时，报告具体 blocker，不能使用猜测的环境给出成功结论。
- 启动和 catalog 使用 V2 明确配置的 timeout；V1 数字 timeout 按官方迁移规则映射 catalog/execution，不能错误地映射为 startup。
- 未配置 startup/catalog 时采用明确的 30 秒默认值；测试通过可注入较短期限运行。
- 捕获退出码和有限 stderr（建议 64 KiB），清理控制字符；输出不回显 environment 值、凭据或完整 profiles 内容。
- stderr 解析采用上下文和原始诊断摘要，不以匹配字符串 `oracle` 作为通用分类器。
- 任意超时、协议错误、提前退出和取消路径都必须释放并 wait 子进程。

## 3. 官方依据

实施开始时重新核对以下文档；按当前实际 V2 release 验证，不使用 V1 schema 推断 V2 字段：

- https://opencode.ai/v2/docs/mcp-servers
- https://opencode.ai/v2/docs/migrate-v1
- https://opencode.ai/v2/docs/config
- https://opencode.ai/docs/mcp-servers/ （仅作为 V1 输入规范）

rmcp 客户端 feature、child-process transport、握手协商及关闭 API 应通过 Context7 或当前锁定依赖源码确认，再修改 Cargo features。避免手写另一套完整 JSON-RPC 协议栈。

## 4. 实施任务

### Task 1：建立可重复的启动基线

**Files:**
- Read: `Cargo.toml`, `src/agent/service.rs`, `src/agent/mcp.rs`, `src/persistence/profiles.rs`
- Test: `tests/agent_mcp_protocol.rs`（新增）
- Record: 本文末尾验证记录

**Steps:**
1. 记录 `git status --short`、`command -v lazydb opencode opencode2`、各版本及 `lazydb capabilities --json`；避免覆盖已有未提交计划。
2. 执行 `cargo build --bin lazydb`，使用 `target/debug/lazydb` 建立当前源码基线。
3. 测试通过当前 ProfileStore/序列化结构生成临时配置，不把 version 6 固定成未来契约，不加载真实 home。
4. 增加真正的 stdio 集成测试：当前测试二进制 + 空 profiles，初始化成功，tools/list 返回预期 7 个名称。使用 `env!("CARGO_BIN_EXE_lazydb")` 获取测试二进制。
5. 增加损坏 profiles 导致启动失败测试，记录 stderr 和 exit，而不是将它误判为握手超时。
6. 运行 `cargo test --test agent_mcp_protocol`，期望上述基线通过；客户端测试依赖在 Task 6 统一整理。
7. 用当前构建读取实际配置做一次不调用数据库工具的人工探测，将结果与已安装 0.1.4 对照。失败就先记录并定位加载错误。

**Acceptance:** 测试环境隔离且有超时；已安装二进制失败与当前构建结果可区分。

**Commit:** `test(mcp): establish stdio startup and catalog baseline`

### Task 2：实现双格式解析和有效配置模型

**Files:**
- Create: `src/agent/opencode_config.rs`
- Modify: `src/agent/client_config.rs`, `src/agent/mod.rs`
- Create fixtures: `tests/fixtures/mcp_setup/opencode-v2.jsonc`, `tests/fixtures/mcp_setup/opencode-mixed.jsonc`
- Test: 新模块单元测试

**Steps:**
1. 先写表驱动测试：V1/V2 单定义、混合定义、同名冲突、disabled、非法父对象、非法服务器对象。
2. 运行 `cargo test --lib agent::opencode_config`，确认缺失实现导致失败。
3. 建立 `OpenCodeEntry`/`ResolvedOpenCodeServer` 内部模型，至少携带 format、key_path、source、原始 Value、规范化 disabled、有效 command、cwd、environment、timeout 和 shadowed 来源。
4. 解析逻辑只规范化参与判断的字段，保留原始 Value，避免重写时丢失未知字段。
5. 将 `keys()` 对 OpenCode 的固定路径假设替换为显式 entry 查找；Claude/Codex 的已有路径逻辑继续复用。
6. 加入同文档有效 V2 优先的测试，并通过文档/实际 V2 确认非法 V2 同名定义的处理。
7. 测试 passed 后记录输出样例：`source.format=v2`、`key_path=mcp.servers.lazydb`、`disabled=true`。

**Acceptance:** setup/doctor 能共用同一个解析结果，不再分别手写新旧格式判断。

**Commit:** `feat(mcp): normalize OpenCode v1 and v2 server entries`

### Task 3：版本识别与配置发现、覆盖语义

**Files:**
- Create: `src/agent/client_runtime.rs`
- Modify: `src/agent/client_config.rs`, `src/agent/opencode_config.rs`, `src/agent/mod.rs`
- Test: 上述模块单元测试

**Steps:**
1. 写版本解析测试：`opencode v2.0.2`、V1、预发布版本、额外输出、无效输出、命令不存在、超时。
2. 注入 command runner/检测结果，测试不依赖本机 OpenCode 安装。
3. 实现有界版本检测，返回版本、运行时、可执行文件路径及检测失败原因。
4. 把 `Locations::sources` 的客户端版本作为显式输入，保留易用的默认入口给既有调用者。
5. V2：从 project 一直发现到文件系统根目录；先远到近 direct configs，再远到近 `.opencode` configs；全局配置优先级低于这些配置。
6. 逐条核对环境覆盖配置和 json/jsonc 的真实优先级，加入 fixture；不能直接将既有 `OPENCODE_CONFIG_DIR` 顺序视为已验证事实。
7. V2：高优先级同名服务器整体替换。测试下层带 command、上层只有 disabled 的情况，禁止伪造完整 command。
8. V1：保留并用独立 V1 fixture 验证其既有合并/发现语义。Unknown：列出候选和不确定性，不宣称唯一有效结果。
9. 运行 `cargo test --lib agent::client_config`、`cargo test --lib agent::client_runtime`、`cargo test --lib agent::opencode_config`。

**Acceptance:** 同一个 V1 格式配置可按 V2 运行时语义加载；Git 根目录之上的 V2 配置不再漏报。

**Commit:** `fix(mcp): model OpenCode runtime discovery and precedence`

### Task 4：升级 setup 并保持无损、幂等写入

**Files:**
- Modify: `src/cli.rs`, `src/main.rs`, `src/agent/setup.rs`, `src/agent/client_config.rs`
- Modify tests: `tests/mcp_setup.rs`
- Modify: `tests/fixtures/mcp_setup/README.md`

**Steps:**
1. 添加 CLI 参数及对应值类型：setup 的 `--opencode-format`、`--opencode-bin`、`--server-bin`；doctor 共用 `--opencode-bin`。
2. 仅在请求包含 OpenCode 时接受 OpenCode 专用参数；多客户端 setup 中 server-bin 应一致作用于各客户端命令。
3. 更新 `SetupOptions` 和所有结构体构造点，旧的 `setup::run` 包装入口使用默认值，避免散落隐式行为。
4. 先补决策表测试：V1/V2 添加、重复执行 unchanged、混合格式不重复添加、显式格式冲突、未知版本兼容回退。
5. 加入 disabled、timeout、codemode、environment 和注释保留测试；绝对二进制路径含空格时仍是单个 command 元素。
6. 调用 Task 2/3 的解析结果选择目标 key_path，沿用 CST 写入及原子文件更新。
7. 配置来源扩展后检查默认新文件目标：必须是当前 project 的 `opencode.json`，不得因 sources 排序而创建文件系统根目录配置。
8. JSON 计划输出增加 selected_format、runtime、key_path、selection_reason；新增字段采用向后兼容方式，不改变已有 status 的含义。
9. 运行 `cargo test --test mcp_setup`；确认 dry-run 不落盘，第二次执行字节级不变化。

**Acceptance:** 原有 V1 配置继续可用；已有 V2 配置能被正确识别；不会生成新旧重复条目。

**Commit:** `feat(mcp): add format-aware OpenCode setup`

### Task 5：修正静态 doctor

**Files:**
- Modify: `src/agent/doctor.rs`, `src/main.rs`
- Modify tests: `tests/mcp_doctor.rs`

**Steps:**
1. 写失败测试：V2 配置发现、disabled=true、同文件 V2 优先、V2 整体覆盖、Git 根以上配置、运行时未知。
2. 从 `inspect_client` 移除 OpenCode 的通用递归 merge，改用共享 resolver；其他客户端仍采用各自已有语义。
3. 每个 source 输出 format、key_path、precedence、是否生效；多个来源不是自动失败，明确指出 shadowed 原因。
4. 将配置结构有效与 write-policy 检查区分：非 deny 不等于 MCP 连接失败，但应显示实际策略及当前安全默认值的差异。
5. 增加 resolved_executable 与 cwd 的可诊断信息；不因 basename 不是 lazydb 就否定合法的显式自定义路径，应结合命令形状和后续 serverInfo 判断。
6. 静态结果继续标明 `connection_verified=false`；Unknown runtime 不输出确定的 effective_config。
7. 保留 JSON schema 现有字段，新增字段为可选附加信息；若必须改变现有字段语义，单独递增 schema_version 并更新测试和文档。
8. 执行 `cargo test --test mcp_doctor`、`cargo test --test mcp_setup`。

**Acceptance:** 静态 ok 仅代表静态配置检查通过；V2 disabled 和覆盖错误不会被漏报。

**Commit:** `fix(mcp): report effective OpenCode configuration accurately`

### Task 6：实现本地 stdio MCP probe

**Files:**
- Modify: `Cargo.toml`, `Cargo.lock`, `src/agent/doctor.rs`, `src/agent/mod.rs`
- Create: `src/agent/probe.rs`, `tests/mcp_probe.rs`
- Modify: `tests/mcp_doctor.rs`, `tests/agent_mcp_protocol.rs`

**Steps:**
1. 确认锁定 rmcp 的 client/child-process transport feature 与生命周期 API；按需要添加 tokio process/io 功能，不顺带升级 rmcp 主版本。
2. 将 Task 1 协议测试与生产 probe 都建立在官方 SDK 客户端上；测试层封装不泄漏到生产 API。
3. 定义 `ProbeReport`，包含 stage、status、exit_code、elapsed_ms、protocol_version、server_info、tool_count、bounded_stderr。
4. 定义 `ProbeOptions`，包含解析完成的 command/cwd/environment、startup/catalog timeout、stderr 上限。
5. 实现环境和工作目录解析：相对 cwd 从 workspace 解析，绝不把全局配置目录当成 project；OpenCode `{env:NAME}` 语法按官方规则处理，JSON 中 `$NAME` 不当成 shell 展开。
6. 实现 spawn → initialize → initialized → tools/list；启动时并发读取 stderr，避免管道填满导致死锁。
7. 对提前 EOF 获取退出码和 stderr，分类为 startup_failed/initialization_failed；超时保留 stage 信息。
8. 使用取消/析构保护和有界 shutdown 确保清理；若 SDK 不保证清理子进程，显式实现 kill+wait 兜底。
9. 删除“probe not implemented”旧测试，改成无 probe 时不启动任何命令的测试。
10. 写真实临时 LazyDB 测试，以及 Rust 测试 fixture 子进程模拟：立即退出、启动挂起、catalog 挂起、无效 stdout、超量 stderr、错误响应和正常退出。
11. 验证 profile 解析错误保留原因摘要，最终报告不是 generic timeout；测试不依赖真实 Python、数据库或用户 home。
12. 执行 `cargo test --test mcp_probe --test mcp_doctor --test agent_mcp_protocol`。

**Acceptance:** 成功 probe 返回 serverInfo 和 7 个工具；任何失败路径无残留直接子进程；不调用 query、execute_change、execute_file。

**Commit:** `feat(mcp): probe stdio startup and tool discovery`

### Task 7：暴露 LazyDB 服务身份与构建能力

**Files:**
- Modify: `src/agent/mcp.rs`, `src/cli.rs`, `src/main.rs`
- Test: `tests/agent_mcp_protocol.rs`, `tests/agent_cli.rs`
- Optional create: `build.rs`（仅在仓库没有可复用 revision 注入机制时）

**Steps:**
1. 先检查 rmcp `tool_handler` 宏对 capabilities/get_info 的生成行为；已实测默认包含 tools，不应为改身份而意外删除它。
2. 添加握手断言：serverInfo.name 为 lazydb，version 与 Cargo 包版本一致，capabilities.tools 存在。
3. 按 SDK 支持的方法提供服务身份，保留 SDK 的协议版本协商与 tool routing。
4. 扩展已有 `lazydb version --json`，保留 version/cli_api，增加 revision（允许 null）和实际启用的 drivers。
5. 与 `capabilities --json` 共享驱动能力计算，区分支持解析的数据库种类与编译可用的 driver，避免固定数组误报构建能力。
6. revision 优先使用发布流水线提供的环境变量；本地 git fallback 不存在时返回 unknown，不能使源码包构建失败；若用 build.rs，处理 HEAD/ref 变动的 rerun 条件。
7. 保持简短 `--version` 可用；详细信息放 version 子命令，不破坏现有 semver 消费方式。
8. 执行 `cargo test --test agent_mcp_protocol --test agent_cli`，并在 `--no-default-features` 构建下核对 driver 信息。

**Acceptance:** 同版本不同构建能被诊断区分，MCP stdout 仍只包含协议消息。

**Commit:** `feat(mcp): expose LazyDB server and build identity`

### Task 8：新旧客户端与 Code Mode 验证

**Files:**
- Create: `tests/fixtures/mcp_setup/opencode-v1-compat.jsonc`
- Update: `tests/fixtures/mcp_setup/opencode-v2.jsonc`
- Record: 本文验证记录

**Steps:**
1. 获取并记录独立 V1 客户端的确切版本和路径，不覆盖当前 V2 安装；未获得时将真实 V1 验证标记 blocked。
2. 使用临时项目和隔离客户端配置，连接指向当前构建绝对路径，数据库采用临时 SQLite。
3. 验证 V1 + V1 配置：连接、发现 7 工具、list_connections、只读 SELECT。
4. 验证 V2 + V1 配置：迁移兼容路径正常。
5. 验证 V2 + 原生配置 + codemode=false：直接工具发现和调用正常。
6. 验证 V2 + 原生配置 + 默认 Code Mode：list_connections、query 的实际返回内容可消费。
7. 验证数据库调用附带 `_meta.sessionID` 不改变工具参数解析；缺少 _meta 也可正常调用。
8. 在隔离 SQLite 中验证 deny policy 拒绝写入，避免把客户端批准错误地当作服务端写权限。
9. Code Mode 若失败，保存精确错误、输入 schema、文本返回和调用阶段再定位；只有确认需要时才另行设计 structuredContent，保留文本向后兼容。

**Acceptance:** 三条实际调用路径记录 client version、LazyDB revision、配置格式、协议版本和结果；未测项不写为通过。

**Commit:** `test(mcp): add OpenCode compatibility fixtures`

### Task 9：文档、综合检查和交付

**Files:**
- Modify: `docs/coding-agent-access.md`, `README.md`, `tests/fixtures/mcp_setup/README.md`
- Update: 本文验证记录

**Steps:**
1. 文档并列提供“V2 原生配置”和“V1/V2 共享兼容配置”；V2 权限示例使用 permissions 数组，保留现有只读/写操作说明。
2. 说明 --opencode-format 是写入格式而不是强制客户端模式；解释 V2 支持 V1 不代表 V1 能读取 V2。
3. 添加已实现命令示例：

```bash
lazydb mcp setup --client opencode --opencode-format v2 --dry-run --json
lazydb mcp setup --client opencode --opencode-format v1 --yes --json
lazydb mcp setup --client opencode --server-bin /absolute/path/to/lazydb --dry-run --json
lazydb mcp doctor --client opencode --opencode-bin /absolute/path/to/opencode --probe --json
lazydb version --json
lazydb capabilities --json
```

4. 添加“profile 解析失败/找不到二进制/启动超时/工具目录失败/Code Mode 调用失败”按阶段排障说明。
5. 检查 README 中 `opencode.json[c]` 注册说明和 fixtures 路径，不留下只有 mcp.lazydb 的泛化描述。
6. 运行以下检查，出现相关失败先修复再继续；基线无关失败单独记录，不计作本功能通过：

```bash
cargo fmt --check
cargo test --test mcp_setup --test mcp_doctor --test mcp_probe --test agent_mcp --test agent_mcp_protocol --test agent_cli
cargo test --lib agent::
cargo check --no-default-features
cargo clippy --all-targets -- -D warnings
```

7. 对照仓库当前 CI 再运行其要求的其他检查；修改 CLI/公共结构时尤其核对完整编译结果。
8. review `git diff --check` 与最终 diff，确认所有新 CLI 参数有测试和文档。
9. 交付变更摘要、客户端测试结果、未完成项、用户当前配置需指向的实际二进制路径。

**Acceptance:** 文档可照着复现，自动测试通过，实际客户端验证结果与证据对应。

**Commit:** `docs(mcp): document OpenCode compatibility and startup diagnostics`

## 5. 依赖顺序与里程碑

```text
Task 1 基线
  -> Task 2 双格式模型
  -> Task 3 运行时与优先级
  -> Task 4 setup
  -> Task 5 静态 doctor
  -> Task 6 probe
  -> Task 7 身份信息
  -> Task 8 客户端验证
  -> Task 9 文档与交付
```

- M1：当前构建可启动，V1/V2 配置可正确注册和读取（Tasks 1–4）。
- M2：故障可准确定位到配置/启动/协议/catalog 阶段（Tasks 5–7）。
- M3：实际客户端路径验证完成，文档和回归检查通过（Tasks 8–9）。

工作量估计：约 4–6 个工程日，取决于客户端实测、现有编译速度及 rmcp transport 清理 API；Code Mode 若出现新的独立缺陷，另行评估。建议按里程碑提交，而不是为每一个纯机械步骤创建提交。

## 6. 完成标准

- [ ] 正确构建能读取当前 Oracle profiles 并完成 MCP 握手。
- [ ] setup 支持 V1/V2、保持 JSONC 无损、重复执行幂等。
- [ ] 已有 V2 定义不再导致重复添加 V1 条目。
- [ ] V2 disabled、同文档优先级、跨文件整体替换和上层发现语义正确。
- [ ] 客户端版本未知时明确显示不确定性。
- [ ] probe 能发现实际启动错误并释放子进程。
- [ ] serverInfo 和 version/capabilities 能识别实际构建。
- [ ] V1 实际客户端、V2 兼容配置、V2 native tools、V2 Code Mode 均有验证记录。
- [ ] Claude Code/Codex 注册测试无回归。
- [ ] 默认 deny policy 保留，文本工具返回兼容性保留。
- [ ] 相关自动检查与文档完成。

## 7. 验证记录（执行时填写）

| 项目 | 版本/命令/配置 | 结果 | 证据或阻塞原因 |
| --- | --- | --- | --- |
| 已安装 LazyDB 启动 | 0.1.4，真实 profiles | 已复现失败 | unknown variant oracle；2026-09-12 调查 |
| 已安装 LazyDB 隔离握手 | 0.1.4，空 profiles，2025-06-18 | 已通过基础协议探测 | 7 个工具；非 OpenCode 端到端验证 |
| 当前源码构建启动 | target/debug/lazydb，空 profiles | 已通过 | MCP initialize 与 tools/list 成功，7 个工具 |
| setup/doctor/probe 自动测试 | cargo test 相关 suites | 已通过 | setup 14、doctor 7、probe 2、协议 2 |
| 独立 OpenCode V1 | `~/.config/opencode/opencode1` | 部分验证 | 版本 1.18.30；脚本可运行，未完成端到端 MCP 连接 |
| OpenCode V2 + V1 配置 | `~/.config/opencode/opencode2` | 阻塞 | 版本 2.0.2；隔离配置下 `mcp list` 超时，无输出 |
| OpenCode V2 + native tools | opencode2 2.0.2 | 未验证 | 客户端命令超时，不能声明通过 |
| OpenCode V2 + Code Mode | opencode2 2.0.2 | 未验证 | 客户端命令超时，不能声明通过 |
| 格式、编译和 lint | cargo check、clippy、diff check | 已通过 | `cargo clippy --all-targets -- -D warnings` 通过 |
