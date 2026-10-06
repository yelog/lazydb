# Non-interactive Connection Management Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 为 Issue #12 提供可重复执行、无需 TTY 的连接配置命令，使保存后的连接与凭据能被 TUI、agent CLI、MCP 和 LSP 一致使用。

**Architecture:** 新增 `connections` 命令组，将连接业务校验和持久化事务从 TUI runtime 下沉为共用服务。所有 profile 写入采用跨进程锁、最新文件重读和目标级乐观并发检查；凭据位置由统一上下文解析，CLI 使用显式 DTO 输出机器可读结果。

**Tech Stack:** Rust 1.94、Clap 4、Tokio、Serde/JSON/TOML、现有 SecretStore 与 XChaCha20Poly1305、本地文件锁、现有数据库 adapters。

---

## 0. 背景与执行约定

- Issue: https://github.com/yelog/lazydb/issues/12
- 本计划依据 2026-10-06 工作区代码。行号用于导航，执行前按符号确认。
- 本次交付是计划；实施时逐任务落地并运行对应验证。
- 当前 `.gitignore` 和 `Cargo.toml` 已有工作区修改，属于既有工作。实施前检查 diff，提交时只 stage 本任务内容。
- 建议实施时使用独立 feature 分支/worktree；测试通过后按任务做小提交。本文的提交信息是建议，实际提交遵循执行时用户授权。
- `@writing-plans` 用于维护本计划；若执行环境没有 `superpowers:executing-plans`，按下列任务和验收点顺序执行，不依赖缺失技能。
- 新测试应验证用户可观察行为、故障恢复和并发一致性，不测试私有函数调用次数。

### 当前关键代码

| 位置 | 现状 | 实施含义 |
| --- | --- | --- |
| `src/cli.rs:17,93` | 全局 `--url`、`--read-only`，现有顶层子命令 | 新命令不能重复定义同名 Clap 参数 |
| `src/main.rs:6` | 先处理子命令，否则进入 TUI | 在此添加独立命令分发和退出状态 |
| `src/profile.rs:491,600` | URL 解析、生成随机 UUID、默认 Global、分离临时密码 | 可复用解析，upsert 不能覆盖已有 UUID |
| `src/model/profile_manager.rs:988` | 字段校验、全文件 trim+lowercase 名称唯一检查、catalog scope 校验 | 提取领域校验，保留 UI 字段错误映射 |
| `src/runtime.rs:5631` | 凭据更新、持久化、回滚、runtime registry 混合 | 拆开持久化事务和会话状态 |
| `src/runtime.rs:2045,2082,2117,2186` | 保存、删除、access、organization 四类写入口 | 都要接入统一事务 |
| `src/persistence/profiles.rs:325` | 临时文件、sync、rename；保留 unavailable profiles | 有原子替换，没有跨进程读改写保护 |
| `src/runtime.rs:6199` | key 相对 profile 文件目录 | 与 agent/LSP 不一致 |
| `src/agent/service.rs:32`、`src/lsp/catalog.rs:496` | key 来自 AppPaths | 统一路径并兼容旧密文 |
| `src/persistence/credentials.rs:41` | Prompt 在 headless 返回错误 | CLI 创建不能隐式降级为 session-only |
| `src/agent/context.rs:38`、`src/agent/selection.rs:61` | 当前项目+Global 可见，UUID/名称选择，歧义报错 | 读取命令复用现有规则 |
| `.github/workflows/ci.yml:81` | fmt、clippy、全特性测试；另有数据库服务作业 | 最终对齐 CI，并补进程级闭环测试 |

## 1. 首版交付契约

### 1.1 命令与选项

```text
lazydb connections add --name NAME --url URL
    [--scope project|global] [--project PATH]
    [--password-env VAR | --password-stdin]
    [--upsert] [--read-only | --read-write] [--json]

lazydb connections list [--project PATH] [--all] [--json]
lazydb connections show SELECTOR [--project PATH] [--all] [--json]
lazydb connections test SELECTOR [--project PATH] [--timeout SECONDS] [--json]
```

- 所有命令支持已有 `--config PATH`。为此命令组增加 global `--json`，子命令前后均接受。
- `add` 的 URL 使用现有全局 `Cli.url`。不要在子命令再声明一个 `url` Arg；业务入口要求 add 必须提供 URL。
- `--read-only` 使用已有全局参数，`--read-write` 是 add 的互斥参数。
- `--profile` 不是管理命令的目标选择器；在 connections 中传入时返回参数错误。list/show/test 收到 `--url`、`--read-only` 也返回参数错误，防止静默忽略。
- `--project` 用于确定当前上下文；`--scope global --project PATH` 合法，但新 profile 的 access 仍为 Global。
- `add` 无数据库网络 I/O，不启动终端、工作区、catalog discovery 或连接池。
- `test` 默认总操作预算 10 秒，允许 1–300 秒；测试凭据解析、connect、probe，并对关闭设置有界清理时间。记录 adapters 的取消能力限制，特别是 blocking adapter。
- `list/show` 默认与 agent 可见集合一致；`--all` 显式读取完整用户配置。`test` 首版保持 agent 可见集合和选择规则。
- 首版通过 URL 支持已有 parser 覆盖的驱动格式；批量 import/export、字段式 host/port 配置、CLI 删除、MCP 写配置工具为后续独立增强。

### 1.2 身份、作用域与 upsert

1. 名称 trim 后不能为空；新建保持 TUI 的全文件大小写不敏感唯一规则。不要只在 CLI 放宽为项目内唯一。
2. 历史文件中的重名仍能加载；读取选择沿用现有 UUID/名称规则，不改变 agent 契约。
3. add 默认 access 复用 `settings.connections.default_access`；CurrentProject 使用 `ProjectContext::resolve_from/resolve_current`。Git 外沿用 canonical 当前目录。
4. `--upsert` 必须显式给出 `--scope`，避免默认设置变化导致更新错误对象。
5. Global upsert 在 Global 集合中按规范化名称匹配；Project upsert 在包含目标 canonical root 的集合中匹配。
6. 一个匹配则更新；多个匹配返回 `connection_ambiguous`；零匹配则按新建处理，但若其他作用域已存在同名项，返回 `connection_name_conflict`。
7. 更新保留 UUID、group_id、顺序、全部已有项目关联、environment 和未提供的凭据；不把 Global 自动改成 Project 或反过来。
8. URL 是完整连接端点说明：kind/url_format/host/port/user/database/default_schema/sqlite_path/ssl_mode 使用 URL 解析结果，不做不透明的字段猜测。
9. read_only 的规则：显式 `--read-only` / `--read-write` 优先；URL 显式 readOnly 次之；更新时省略则保留旧值，新建省略为 false。parser 增加显式选项 presence 元数据，避免把缺失与 false 混为一谈，沿用现有冲突参数检查。
10. 更新保留原 catalog_scope，随后校验它是否兼容新 database/schema。旧 scope 恰好等于旧端点派生默认值时，可以根据新端点重新派生；自定义 scope 不兼容则返回 `catalog_scope_conflict`，不得静默扩大可见范围。
11. 新建 SQLite 相对路径以调用进程 cwd 解析为稳定绝对路径；路径可以尚不存在，不用目标文件 canonicalize 强制要求文件存在。标准化实际存在的父路径，测试从别的目录启动 agent 的行为。
12. 参数相同、明文密码相同且存储策略相同的 upsert 返回 `changed: false`，不重写文件或重新生成 nonce。比较密码必须通过 secret API，不计算并持久化密码哈希。

### 1.3 凭据输入与落盘

- `--password-env VAR` 是本次导入来源：读取后使用现有 LocalEncrypted 保存，不新增 Environment 型 CredentialPolicy。
- stdin 与 env 互斥；URL 自带密码与显式来源也互斥。继续支持 parser 已接受的 URL 密码，但文档示例使用无密码 URL。
- 缺失环境变量返回 `credential_input_missing`；显式空字符串保留为空密码，不等同于没有输入。
- `--password-stdin` 只读取非 TTY 输入；输入连接到 TTY 时立即报错。输入有大小上限和读取超时，防止 open pipe 永远不 EOF。
- stdin 采用一份 UTF-8 密码 payload：移除至多一个末尾 LF 或 CRLF，保留其余空白。上限必须与现有 `MAX_CIPHERTEXT_SIZE` 和 AEAD overhead 一致，在加密前拒绝无法回读的数据。
- 新建未提供密码：CredentialPolicy::None。更新未提供密码：Preserve，不调用系统 keyring，不重新加密。
- 首版 CLI 写入 LocalEncrypted；现有 System/Prompt/None 记录可读取，System 记录被显式更新密码时通过共用事务替换。TUI 继续使用既有存储选项。
- 加密或落盘失败返回错误，不能以 Prompt/session-only 作为成功结果。
- 不直接输出完整 ConnectionProfile、密文、nonce、keyring reference 或原始 URL。输入类型若可包含密码，Debug 必须脱敏。

### 1.4 JSON 与退出码

成功示例（实际返回 UUID）：

```json
{
  "schema_version": 1,
  "ok": true,
  "command": "connections.add",
  "data": {
    "id": "11111111-1111-4111-8111-111111111111",
    "name": "app-dev",
    "kind": "postgres",
    "scope": "project",
    "projects": ["/workspace/app"],
    "credential_storage": "local_encrypted",
    "changed": true
  }
}
```

错误示例：

```json
{
  "schema_version": 1,
  "ok": false,
  "command": "connections.add",
  "error": {
    "code": "connection_name_conflict",
    "message": "A connection with this name already exists."
  }
}
```

| 退出码 | 类别 | 典型 code |
| --- | --- | --- |
| 0 | 成功，包括 unchanged | 无 |
| 2 | 参数/本地配置校验错误 | invalid_arguments, invalid_profile, catalog_scope_conflict |
| 3 | 目标或并发冲突 | connection_not_found, connection_name_conflict, connection_ambiguous, profile_conflict, store_busy |
| 4 | 凭据失败 | credential_input_missing, credential_input_invalid, credential_failure |
| 5 | 本地存储失败 | persistence_failure, credential_rollback_failed |
| 6 | 连通性失败或超时 | connection_failed, timeout |

- JSON 模式下 stdout 只有一个 JSON document；stderr 可有脱敏诊断；不附加 anyhow 的第二份错误输出。
- JSON 必须覆盖 Clap 参数错误。`--help` / `--version` 保留常规行为和退出码 0。
- parse error 阶段仅识别明确的 connections 子命令路径及该命令的 `--json` 请求，使用 `try_parse` 与小型无副作用参数路由，不在任意 argv 中模糊查找单词。
- 既有 agent/version/doctor/MCP JSON 不改包裹格式；新增 `connections-v1` capability，现有 CLI_API_VERSION 可保持 1，因为属于增量能力。

## 2. 共用架构和事务边界

### 2.1 建议文件结构

```text
src/connections/
  mod.rs            共用模块入口
  types.rs          请求、结果、错误、变更集合
  validation.rs     名称、字段、scope 与更新规则
  service.rs        add/upsert/read/probe 和 TUI 持久化入口
  credentials.rs    凭据更新、回滚、旧密文重包裹
  cli.rs            参数转换、I/O、输出与退出码
  output.rs         显式 JSON DTO 和文本格式
src/persistence/
  profile_context.rs profile/key 路径与 legacy read candidates
  profile_transaction.rs 跨进程锁、最新快照和受控 commit
  profiles.rs       已有版本迁移、序列化、原子保存
```

领域层不得依赖 TUI TextInput/弹窗/Action。将 CredentialUpdate、ProfileChange 等可共用类型下沉；在原模块 re-export，降低调用点迁移成本。DiscoveryFingerprint 和 UI 状态仍在 model。

### 2.2 统一凭据路径与兼容读取

采用文档已有的应用根目录语义：`--config` 只覆盖 profile 文件，主密钥仍在 `AppPaths::credential_key_file()`。完整搬迁通过已有应用根目录机制完成。

- `ProfileContext::resolve(paths, config_override)` 返回 profile 路径、主 key 路径和可选 legacy key（自定义 profile 所在目录的 credential.key）。
- 所有 TUI/agent/MCP/LSP 入口使用该上下文；单测显式注入 AppPaths，不访问用户真实 key。
- 解密先尝试主 key，再尝试不同路径的 legacy key；以密文认证结果决定是否成功，处理两个 key 都存在、缺失、损坏的情况。失败返回统一脱敏错误。
- 读取不得创建 key 或重写 profile。list/show 不读取 key 内容。
- 写新密码只用主 key。已有密码没有变化的 upsert 不因 legacy 来源而自动写盘。
- 修改 legacy profile 的其他持久化字段时，可以在同一 profile 事务中将该目标密文重包裹到主 key，并返回 migration 信息；不批量重写其他 profile，不删除 legacy key，不覆盖已有主 key。
- 回滚时旧 profile/旧密文仍可配合 legacy key 工作。主 key 创建后事务失败，可以保留未被引用的新 key，不宣称多文件原子提交。
- 不保证系统 keyring 与 TOML 在进程被 kill/断电时具备跨存储原子性；实现普通错误路径下的补偿回滚，并明确回滚失败的错误码。

### 2.3 锁和并发控制

- profile 文件使用稳定 sidecar advisory lock，如 `connections.toml.lock`；锁对象持有 File，释放依赖 OS/RAII。
- Rust 1.94 可优先使用标准库文件锁能力；实施时核对支持平台，只有确有缺口才增加依赖。
- 锁文件不能在每次解锁时删除，否则并发打开可能锁住不同 inode。不要照搬 WorkspaceStore 的 create_new marker 方案。
- 规范化 profile 的实际文件或现有父目录，避免相对路径和 symlink 别名绕过同一锁。已有文件 symlink 要对实际目标更新，明确不覆盖 symlink 本身。
- 使用有限 try-lock 重试（默认 5 秒），超时返回 store_busy；阻塞文件 I/O 放在 blocking worker，不阻塞 async executor。
- 事务在锁内重读最新 ProfileLoadReport。只针对目标 UUID/分组/排序做操作，绝不直接把 registry 整份旧快照覆盖到文件。
- TUI 请求带 expected 原始 profile（或等价不可变版本 token），锁内比较。不同目标的修改可合并；同目标已变化返回 profile_conflict；已删除不能被旧编辑复活。
- 分组删除、排序是多对象操作：比较对应 group 与参与的 siblings/成员关系；并发成员或顺序变化冲突时重试，不丢掉新成员。
- 锁覆盖凭据副作用和文件 commit，保持普通失败的回滚顺序。stdin/env 获取、URL 解析、数据库 probe 在锁外。
- 事务一旦开始副作用，应由拥有 guard 的任务完成 commit/rollback，不因 UI 取消直接丢弃 future。系统凭据提供方的不可取消调用必须在设计中单独处理，不能把简单 timeout 当作已经取消。
- ProfileStore 原始整份 save 仅用于明确的初始化/测试或持锁事务内部。所有生产 profile 写入点迁移到事务 API；通过 codegraph 调用关系检查遗漏。
- 保留已有 unavailable profile 和扩展字段逻辑，且基于锁内最新原始文档合并。

### 2.4 活跃进程的可见性

首版不增加文件 watcher。新 CLI 进程总读最新数据；MCP/LSP 当前按启动快照工作，外部修改后重新启动相关服务才能看到新配置，文档明确说明。

TUI 发起写入时检查最新磁盘内容并保留外部新增记录。runtime 的持久化基线与 UI 当前连接会话区分：目标保存成功后只同步目标状态；其他外部变更不偷偷替换活动连接或 session secrets。需要刷新时可重新加载/重启；旧视图再次编辑已变化的目标必须返回冲突。

## 3. 任务与验证步骤

每个任务按“行为测试 → 实现 → 定向回归 → 检查 diff”执行。以下红灯是实施时的预期，不代表编写计划时已运行测试。

### Task 1：建立领域请求、校验和更新语义

**Files**
- Create: `src/connections/mod.rs`, `src/connections/types.rs`, `src/connections/validation.rs`
- Modify: `src/lib.rs`, `src/profile.rs`, `src/model/profile_manager.rs`
- Test: `tests/connection_validation.rs`, `tests/profile_draft.rs`, `tests/profile_url.rs`

**Step 1 — 添加参数化行为测试**

覆盖七种 DatabaseKind 的合法 profile、空名称、0 端口、Redis 非数字 DB、SQLite 路径、Oracle service、catalog_scope 不兼容；区别 URL 缺失 readOnly 与显式 false。

**Step 2 — 提取纯领域类型和校验**

把 UI 原始文本解析与 profile 领域校验分开：UI 继续处理未提交 URL 和字段输入错误，构建候选 profile 后调用共用 validator。错误使用 field/code/message，UI 将 field 映射到 ProfileField。新增入口不要求先连接数据库。

建议使用以下完整的预期状态类型作为事务协议的一部分：

```rust
use crate::profile::ConnectionProfile;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExpectedProfile {
    Absent,
    Existing(Box<ConnectionProfile>),
}
```

**Step 3 — 实现合并规则**

分别实现 create、upsert match、endpoint replacement、metadata preservation；不能以 `import_connection_url()` 返回的新 UUID 覆盖旧 ID。姓名冲突校验独立于历史文件 load 校验，不让一个历史同名项阻止读取整个文件。

**Step 4 — 定向验证**

```bash
cargo test --test connection_validation --test profile_draft --test profile_url
```

预期：共享校验行为通过，现有 UI 字段定位与 URL 测试通过。

**Step 5 — 检查 diff；建议提交**

`refactor(connections): share profile validation and update semantics`

### Task 2：统一 profile/key 上下文和 legacy 解密

**Files**
- Create: `src/persistence/profile_context.rs`
- Modify: `src/persistence/mod.rs`, `src/persistence/credentials.rs`, `src/persistence/local_credentials.rs`
- Modify: `src/agent/service.rs`, `src/lsp/catalog.rs`, `src/runtime.rs`
- Test: `tests/profile_context.rs`, `tests/credential_resolution.rs`, `tests/lsp_catalog.rs`

**Step 1 — 编写路径矩阵测试**

默认 profile、自定义绝对/相对 profile、主 key only、legacy key only、两者不同、主 key 损坏但 legacy 可解密、两个 key 都不能认证；读操作不得创建文件。

**Step 2 — 实现注入式上下文**

生产从 AppPaths resolve，测试通过临时目录显式注入。去除 runtime 的 profile-parent-only 构造和 agent/LSP 各自拼装 key 路径，统一 CredentialResolver。

**Step 3 — 实现有来源标记的兼容解密**

内部返回 secret 与 Primary/Legacy 来源用于后续事务；对外 headless API 可继续返回 Option<SecretString>。不要在 resolve_headless 中做写迁移。

**Step 4 — 定向验证**

```bash
cargo test --test profile_context --test credential_resolution --test lsp_catalog --test lsp_service --test profile_runtime
```

预期：三种入口使用相同上下文，原有 session/startup password 优先级仍成立。

**Step 5 — 检查 diff；建议提交**

`fix(credentials): unify profile credential paths with legacy fallback`

### Task 3：建立跨进程 profile 事务基础

**Files**
- Create: `src/persistence/profile_transaction.rs`
- Modify: `src/persistence/mod.rs`, `src/persistence/profiles.rs`
- Test: `tests/profile_transactions.rs`, `tests/persistence.rs`, `tests/profile_compatibility.rs`

**Step 1 — 写真正的跨进程测试**

用当前测试二进制的专用 ignored helper + 临时目录作为子进程，或等价测试 helper；使用 pipe/文件握手控制步骤，不依赖随意 sleep。

覆盖锁竞争、holder 被 kill 后释放、相对/绝对路径同锁、symlink 别名同锁、文件不存在时并发首次创建。

**Step 2 — 实现 sidecar advisory guard**

稳定 lock 文件、private mode、5 秒有限等待、错误映射；等待锁在 blocking worker 中执行，成功后 guard 生命周期覆盖 commit。

**Step 3 — 实现最新快照与目标 CAS**

测试流程：两个 writer 同时基于 A；writer 1 新增 B；writer 2 修改 A；结果包含 A 的修改和 B。两个 writer 修改 A 则第二个报 conflict；删除后的 A 不能复活。

**Step 4 — 保留原始存储兼容行为**

commit 复用已有版本迁移、unavailable profiles/未知字段保留、private mode、sync 和 rename。注入保存失败，验证旧文件可读且临时文件清理；验证分组顺序。

```bash
cargo test --test profile_transactions --test persistence --test profile_compatibility
```

**Step 5 — 检查所有 save 调用；建议提交**

`feat(persistence): add locked profile mutations with conflict detection`

### Task 4：抽取凭据保存事务

**Files**
- Create: `src/connections/credentials.rs`, `src/connections/service.rs`
- Modify: `src/connections/types.rs`, `src/model/profile_manager.rs`, `src/runtime.rs`
- Test: `tests/connection_service.rs`, `tests/profile_runtime.rs`, `tests/credential_resolution.rs`

**Step 1 — 使用已有 SecretStore 测试替身编写故障矩阵**

Preserve/None/LocalEncrypted/System/Forget、keyring 旧值读取失败、keyring 写入失败、TOML commit 失败、补偿回滚失败。明确 TUI 的已有 fallback 与 CLI LocalEncrypted 策略分别是什么。

**Step 2 — 把持久化逻辑移入 service**

服务输入包含 profile candidate、expected、credential update；返回已保存 profile、change、actual storage、warning、changed 和可选 session secret。领域请求不依赖 DiscoveryFingerprint。

保持现有事务顺序：捕获可恢复旧状态 → 处理凭据 → 保存 profile → 失败时恢复旧 secret。回滚失败必须保留主错误与 rollback 状态，不泄露 payload。

**Step 3 — 实现幂等与 legacy 重包裹**

重复提供相同密码且 profile 不变：不重新加密；Preserve 无配置变化：不解密、不写盘。配置变化且目标凭据来自 legacy 时，可在本事务中重包裹；故障时旧密文仍可用。

**Step 4 — 定向验证**

```bash
cargo test --test connection_service --test profile_runtime --test credential_resolution
```

预期：成功保存后新 resolver 能解密；失败后旧连接可恢复；changed=false 时配置字节不变。

**Step 5 — 检查 diff；建议提交**

`refactor(connections): extract credential-aware profile service`

### Task 5：迁移全部 TUI 写入口

**Files**
- Modify: `src/runtime.rs`, `src/connections/service.rs`, `src/connections/types.rs`
- Modify as needed: `src/action.rs`, `src/app.rs`, `src/model/profile_organization.rs`
- Test: `tests/profile_runtime.rs`, `tests/profile_groups.rs`, `tests/connection_switch.rs`, `tests/profile_lifecycle.rs`

**Step 1 — 加入四类入口交错测试**

TUI save/delete/access/organization 每类测试都在加载后插入一次外部 add，再执行 TUI 操作，确认新记录保留。针对相同目标和排序成员变化验证冲突。

**Step 2 — 保存和删除接入服务**

runtime 保留 session secrets、revision、连接生命周期和事件派发；磁盘 commit 成功后更新这些状态。失败不派发成功 Action，也不提前更换活动连接。

**Step 3 — access/group/order 接入事务**

在最新集合应用对应 mutation；CAS 覆盖实际参与对象。临时 SESSION 连接保持只在 runtime，不因组织操作而被持久化。

**Step 4 — 明确 stale UI 的处理**

保留用户编辑草稿并报告配置冲突；更新目标成功后同步其 baseline。不得把“已读到最新磁盘对象”当作“UI 已编辑该版本”，绕过 expected 检查。通过 codegraph 枚举 ProfileStore::save 调用，确认生产写入口没有旧全量覆盖。

```bash
cargo test --test profile_runtime --test profile_groups --test connection_switch --test profile_lifecycle --test startup_profiles
```

**Step 5 — 检查 diff；建议提交**

`fix(runtime): preserve external profile changes across tui mutations`

### Task 6：实现 add/upsert 和非交互凭据输入

**Files**
- Create: `src/connections/cli.rs`
- Modify: `src/cli.rs`, `src/main.rs`, `src/connections/service.rs`
- Test: `tests/connections_cli.rs`, `tests/connections_process.rs`

**Step 1 — 参数契约测试**

检查全局 URL/read_only 参数继承、--json 位置、add 缺 URL、stdin/env/URL 密码冲突、upsert 缺 scope、read-only/read-write 冲突以及不适用 global flags 的报错。

完整 parser 测试示例（新增 enum 采用以下命名）：

```rust
use clap::Parser;
use lazydb::cli::{Cli, Command, ConnectionsCommand};

#[test]
fn parses_add_using_the_existing_global_url_argument() {
    let cli = Cli::try_parse_from([
        "lazydb", "connections", "add", "--name", "demo",
        "--url", "sqlite::memory:", "--scope", "project", "--json",
    ]).unwrap();
    assert_eq!(cli.url.as_deref(), Some("sqlite::memory:"));
    assert!(matches!(
        cli.command,
        Some(Command::Connections { command: ConnectionsCommand::Add(_), .. })
    ));
}
```

**Step 2 — 实现输入获取边界**

先完成参数和来源冲突检查，再读取指定 env/stdin。不要隐式使用 LAZYDB_PASSWORD；它保留已有 TUI startup 语义。限制 stdin 大小和读取时间，TTY 输入立即失败。

**Step 3 — 接入 service**

读取 settings 默认 scope、构造 profile candidate、执行 add/upsert；保存全程不连接数据库。新建无数据库在线要求，返回实际 ID 与 storage。

**Step 4 — 进程级行为测试**

stdin null、关闭管道、超长输入、未结束管道、无 DISPLAY/DBUS 场景；服务地址不可达时 add 仍成功；重复 upsert 不变；不同 cwd 使用 SQLite 路径一致。

```bash
cargo test --test connections_cli --test connections_process --test connection_service
```

**Step 5 — 检查 diff；建议提交**

`feat(cli): add non-interactive connection creation and upsert`

### Task 7：实现 list/show/test

**Files**
- Modify: `src/cli.rs`, `src/connections/cli.rs`, `src/connections/service.rs`
- Reuse: `src/agent/context.rs`, `src/agent/selection.rs`, `src/db/mod.rs`
- Test: `tests/connections_cli.rs`, `tests/connections_process.rs`, `tests/connection_service.rs`

**Step 1 — 读取与选择测试**

当前项目+Global、其他项目隐藏、--all、UUID、名称歧义、无匹配、unavailable profile。list --all 通过独立 unavailable 摘要返回不可用记录，不能直接输出原始 TOML/解析错误源文本。

**Step 2 — 实现显式视图**

list/show 返回稳定排序和显式非敏感字段，show 可包含格式化后的无密码 URL、catalog_scope、read_only、environment。它们不解析密码、不连接数据库。

**Step 3 — 实现 probe**

test 使用 headless resolver + DatabaseConnection::connect + probe + close，不加载完整 catalog。连接测试不持有 profile 锁，不保存 workspace/history 或 profile。

以注入 probe trait/closure 验证超时、失败和清理；至少一个 SQLite 实例运行真实成功测试。SQLite 文件模式要求 test 不创建缺失文件，检查现有 adapter open options，必要时给 probe 使用非创建模式。

**Step 4 — 定向验证**

```bash
cargo test --test connections_cli --test connections_process --test connection_service --test agent_selection --test agent_context
```

**Step 5 — 检查 diff；建议提交**

`feat(cli): inspect and test saved connections without a tui`

### Task 8：完成 JSON、错误码和 secret redaction 契约

**Files**
- Create: `src/connections/output.rs`
- Modify: `src/main.rs`, `src/cli.rs`, `src/connections/cli.rs`, `src/connections/types.rs`
- Test: `tests/connections_output.rs`, `tests/connections_process.rs`, `tests/agent_serialization.rs`

**Step 1 — 成功和失败的 process contract 测试**

覆盖表 1.4 中每类退出码、Clap 错误、--help、--json 前后位置；解析 stdout 为单一 JSON document。业务错误不得由 main 的 anyhow 再额外打印。

**Step 2 — 输出显式 DTO**

使用 schema_version=1 envelope；list/show/test/add data 各自定义 Serialize 类型。文本输出与 JSON 共用领域结果，不解析字符串判断错误码。

**Step 3 — 脱敏故障测试**

在 URL userinfo、JDBC password、查询参数、stdin、环境变量中使用唯一 secret sentinel。断言 stdout/stderr/Debug/error 不包含明文及密文。格式错误 URL、provider 错误、损坏 TOML 和未知参数同样覆盖；不直接回显 Clap 的完整原始 argv 或底层包含输入的错误。

**Step 4 — 验证既有命令契约**

```bash
cargo test --test connections_output --test connections_process --test agent_cli --test agent_serialization
cargo test --lib cli::tests
```

**Step 5 — 检查 diff；建议提交**

`feat(cli): define versioned connection command results and errors`

### Task 9：补齐跨入口、跨进程端到端验收

**Files**
- Create: `tests/connections_integration.rs`
- Modify: `tests/connections_process.rs`, `tests/agent_mcp_protocol.rs`, `tests/credential_resolution.rs`
- Modify: `.github/workflows/ci.yml`

**Step 1 — SQLite 无外部依赖闭环**

临时 AppPaths + 临时项目：add → 新进程 agent connections → agent query SELECT 1 → show → 同参数 upsert → 检查 ID、文件内容与数量。所有 subprocess 限时退出；使用 CARGO_BIN_EXE_lazydb，不递归 cargo run。

**Step 2 — 凭据跨进程闭环**

利用 CI 已有 PostgreSQL 服务：URL 移除密码，密码通过 child env 导入；add 退出后启动新 agent 进程，删除导入来源变量仍能 SELECT 1。对自定义 --config 再运行一次，并用新 MCP 服务/LSP loader 验证读取。

无数据库环境时本地可 skip；数据库 CI 作业设置 LAZYDB_REQUIRE_DATABASE_TESTS=1，缺配置必须失败，防止永远绿色的跳过测试。

**Step 3 — legacy 与并发验收**

legacy key fixture → agent 解密 → 目标更新重包裹 → 新进程使用主 key。并行两个 CLI add、两个 upsert 同名、以及 TUI runtime 旧快照写入覆盖测试，检查无丢失/重复和明确冲突。

**Step 4 — 平台矩阵**

现有 Rust job 覆盖 Linux/macOS；补 Windows 的 --no-default-features 定向 job 跑 path/lock/CLI process suites，使用 --locked。真实 keyring 行为用 fake provider，不依赖 CI 桌面服务。Windows 测试通过明确测试上下文/AppData 沙箱，不误用仅 Unix 生效的 LAZYDB_CONFIG_HOME。

```bash
cargo test --test connections_integration --test connections_process --test profile_transactions
```

数据库作业追加：

```bash
env LAZYDB_REQUIRE_DATABASE_TESTS=1 cargo test --locked --test connections_integration -- --nocapture
```

**Step 5 — 检查 diff；建议提交**

`test(connections): verify headless setup across processes and clients`

### Task 10：文档、能力发现与最终验收

**Files**
- Modify: `docs/configuration.md`, `docs/coding-agent-access.md`, `README.md`, `src/cli.rs`
- Update: 本计划中的实际完成记录、如有调整的最终命令契约

**Step 1 — 更新用户文档**

新增首次创建、幂等更新、无密码数据库、password-env/password-stdin、project/global、JSON/退出码表；说明 --url 临时启动与 connections add 持久化的差异。

补充 `--config`/credential.key 主路径与 legacy 兼容语义；修正文档仍仅描述 v5 的 profile 格式表述为当前 v6；说明已有 MCP/LSP 进程需重新启动读取新配置。

**Step 2 — 增加 capability**

capabilities JSON/text 都加入 connections-v1。避免文本特性清单和数组再次漂移；CLI_API_VERSION 保持已有增量兼容策略。

**Step 3 — 执行手工 smoke**

在隔离应用目录和临时项目内执行：

```bash
lazydb connections add --name smoke --url 'sqlite::memory:' --scope project --project . --json
lazydb connections add --name smoke --url 'sqlite::memory:' --scope project --project . --upsert --json
lazydb connections list --project . --json
lazydb connections show smoke --project . --json
lazydb connections test smoke --project . --timeout 10 --json
lazydb agent query --project . --connection smoke --sql 'SELECT 1'
```

预期：首次 changed=true、第二次 false、UUID 一致、test/query 成功、全程不出现 TUI。`sqlite::memory:` 每次连接独立，仅用于 probe/query smoke，不用于证明数据跨进程持久化。

**Step 4 — 对齐最终 CI 检查**

```bash
cargo +1.94.0 fmt --all -- --check
cargo +1.94.0 clippy --all-targets --all-features -- -D warnings
cargo +1.94.0 test --all-targets --all-features
cargo +1.94.0 test --no-default-features --test connections_cli --test connections_process --test profile_transactions
```

预期：检查通过；记录数据库服务实际覆盖和 skip 情况。已有基线失败单独记录，不把它当作本次通过。

**Step 5 — 检查完整 diff；建议提交**

`docs(connections): document non-interactive profile management`

## 4. 依赖顺序与交付里程碑

```text
Task 1 领域校验 ───────────────┐
Task 2 路径与兼容 ────────────┼─ Task 4 凭据事务 ─ Task 5 TUI 接入
Task 3 跨进程事务 ────────────┘                     │
                                      Task 6 add/upsert
                                               │
                                      Task 7 list/show/test
                                               │
                                      Task 8 JSON 与错误
                                               │
                                      Task 9 端到端/CI
                                               │
                                      Task 10 文档与验收
```

Task 1–3 可以独立设计，但共享模块注册和 runtime 接线需按顺序合入。Task 5 完成前不发布可写 CLI，避免旧 TUI 写路径造成数据覆盖。

| 里程碑 | 内容 | 完成判据 |
| --- | --- | --- |
| M1 共用基础 | Task 1–3 | 校验一致、路径一致、锁/CAS/格式兼容通过 |
| M2 写入统一 | Task 4–5 | TUI 全部 profile 写入口走新服务、回滚和交错写入通过 |
| M3 用户功能 | Task 6–8 | 四个命令可用、幂等、JSON/错误码和无 TTY 行为通过 |
| M4 可发布 | Task 9–10 | CLI→agent/MCP/LSP 闭环、跨平台检查、文档和 capabilities 完成 |

## 5. 最终 Definition of Done

- [ ] 用户不打开 TUI 即可创建并持久化所有现有 URL parser 支持的连接。
- [ ] 没有 TTY、桌面 keyring 或在线数据库时仍能完成本地配置。
- [ ] 显式密码保存失败不伪装为成功；新进程能使用保存的密码。
- [ ] 重复 upsert 保留 UUID/metadata，unchanged 时不重写文件。
- [ ] 当前项目与 Global 可见性、历史名称歧义有明确且已测试的行为。
- [ ] 自定义 --config 下 TUI/CLI/agent/MCP/LSP 路径一致，旧密文可读。
- [ ] 保存、删除、access、group、order 均不能覆盖其他进程的新增连接。
- [ ] 同目标并发修改返回冲突，且用户草稿可保留。
- [ ] unavailable profiles、扩展字段、分组和顺序得到保留。
- [ ] JSON/退出码稳定，成功失败均不泄露密码、密文或 keyring reference。
- [ ] 连通性测试有时间预算，不修改 profile/workspace/history。
- [ ] 文档示例经过 smoke 验证，capabilities 可发现该功能。
- [ ] 现有 TUI、agent、MCP、LSP 回归和 CI 所需检查通过。
