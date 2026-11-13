# Native SSH Tunnel Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 为 LazyDB 提供应用内配置、按需启动、复用、诊断和回收的 SSH tunnel，覆盖常见数据库与跳板机场景，同时控制发布包体积、后台进程和网络开销。

**Architecture:** 由公共连接入口管理系统 OpenSSH 子进程，不在 LazyDB 中再嵌入一套 SSH 协议与加密实现。运行时区分数据库逻辑身份和实际传输入口，按驱动使用 TCP local forwarding、Unix socket forwarding 或 SOCKS5；数据库连接、事务及取消任务持有共享隧道租约。

**Tech Stack:** Rust、Tokio、系统 OpenSSH、现有 SQLx / Tiberius / Redis / MongoDB / reqwest 驱动、现有 Rustls、process-wrap。

---

## 0. 当前状态与交付边界

**实施状态（2026-11-13）：** 首版已在当前工作区实现并完成验收，不再是纯实施前计划。

已完成：

- Profile、CLI、Profile Manager、TUI Runtime、Agent / MCP、LSP 均通过
  `DatabaseConnector` 建连；直连路径不启动 SSH 进程或探测。
- 受管系统 OpenSSH 支持密钥、agent、`ssh_config`、ProxyJump、严格 host key、
  共享 single-flight 启动、独立取消、按租约回收和 TCP / Unix socket / SOCKS5。
- SQLx 在 Unix 上仅对 `disable` 使用 TCP forwarding，对会协商 TLS 的
  `prefer` / `require` / `verify-ca` / `verify-full` 使用私有 Unix socket，
  以保留逻辑 hostname、SNI 和证书校验；Windows 的 `prefer` / `require` 使用
  TCP 且明确存在 SNI 限制，严格 TLS 保持 fail closed。
- PostgreSQL / CockroachDB、MySQL / MariaDB / TiDB、SQL Server、Redis / Valkey、
  ClickHouse、Elasticsearch / OpenSearch、MongoDB 已接入；SQLite、DuckDB、
  BigQuery、Snowflake、Oracle 明确拒绝 SSH。
- 数据库逻辑 hostname、SNI、TLS 校验、事务、池补建、取消控制连接、MongoDB
  拓扑 / failover 和 HTTP authority 保留；隧道退出后旧路由 fail closed。
- 已修复 SQL Server Compose profile 未随测试项目清理的问题。

已记录验证：

- macOS arm64、OpenSSH 10.5 的完整 Docker runner 通过 47 个 Rust 集成测试和
  真实 TUI PTY：OpenSSH 6/6、PostgreSQL/MySQL 18/18、SQL Server 2/2、
  Redis/Valkey 6/6、HTTP 5/5、MongoDB 6/6、服务入口 4/4。
- `cargo fmt --all --check`、`cargo clippy --locked --all-targets
  --all-features -- -D warnings`、`cargo check --locked --no-default-features`
  和全量 `cargo test --locked --all-targets --all-features` 通过。
- PostgreSQL 取消控制连接已纳入同一路由租约和失败守卫；修复后全量 Rust
  测试与完整 Docker runner（47/47 加真实 PTY）已重跑通过。
- 最终 release 原始二进制增加 854,096 bytes（+1.33%）；`xz -6` 增加
  258,500 bytes（+1.61%），低于计划上限。
- 路由调整前，`require` 经 TCP forwarding 的 16 会话大结果与等价手工 SSH
  相差约 4%；但该路径会丢失逻辑 hostname / SNI，因此最终代码只让 `disable`
  使用 TCP。会协商 TLS 的 Unix socket 路径仍有明显吞吐差距，作为保留 SNI /
  hostname 语义的已知权衡记录。
  CPU、RSS 和 Linux / Windows 原生客户端未做系统性测量。

未验证边界：

- Linux 和 Windows 原生 OpenSSH、路径、agent、Job Object 与进程树清理。
- HTTP IP-SAN / IPv6、MongoDB SRV / sharded、Redis Cluster / Sentinel。
- CockroachDB、MariaDB、TiDB 的独立拓扑验收和更广的多跳矩阵。
- SSH password / passphrase / keyboard-interactive / MFA 不会在首版提示。

这里的“原生支持”指用户可以在 LazyDB 中完成配置、连接和生命周期管理；运行环境仍需有兼容的 OpenSSH `ssh` 程序。缺失时返回安装提示，不自动下载、不打包一份客户端。

计划区分：

- **首版交付目标：** 密钥 / agent / SSH config、跳板机、共享生命周期、TUI 与 headless 入口，以及下述驱动矩阵中通过验收的组合。
- **实施前必须验证：** SQLx 经 Unix socket 的 TLS、OpenSSH 启动完成判定、Redis 带路由的重连、MongoDB 拓扑与 SOCKS5 的依赖增量。
- **后续扩展：** 应用内输入 SSH password / passphrase / MFA、首次主机指纹确认、Windows 上 SQLx 严格 TLS 的完整适配。

后续扩展不作为首版密钥认证的隐含承诺。不得为了覆盖率而静默关闭主机密钥检查、降低数据库证书校验、绕过隧道直连，或自动重放执行结果不确定的语句。

## 1. 技术选型与成本

| 方案 | 优点 | 成本与限制 | 决策 |
| --- | --- | --- | --- |
| 受管系统 OpenSSH | 复用 SSH config、agent、密钥、证书及 ProxyJump；数据库数据无需由 LazyDB 再转发一次 | 依赖系统程序；存在子进程和跨平台行为差异 | 首选 |
| 内嵌 Rust SSH 库 | 不依赖系统 ssh，可直接控制认证与 channel | 新增加密和密钥格式依赖；需维护 config、agent、known_hosts、跳板机兼容性 | 首版不引入，未来有明确需求和测量依据再评估 |
| libssh2 / ssh2 | 有成熟实现 | 原生库、打包和平台兼容成本；仍需补认证、异步和配置适配 | 不采用 |

不预先建设多种 SSH backend 或通用代理框架。模块保留可测试的进程启动边界即可。

优先使用现有依赖：

- `tokio::process`、同步、计时和任务取消设施。
- `process-wrap` 当前已经以 9.1.0 出现在 `Cargo.lock`，评估作为直接依赖复用其进程组和 Windows Job Object；不能把“已是传递依赖”等同于零二进制增量。
- HTTP 驱动开启 reqwest 的 `socks` feature；按当前锁定版本的 manifest，该开关不新增 Cargo package，仍需测量编入的代码。
- MongoDB 评估开启 `socks5-proxy`；它使用 `fast-socks5`，并不与 reqwest 共用同一 SOCKS 库，需核对实际依赖图与产物增量。
- 需要私有临时目录时优先复用现有 `tempfile`；若提升为生产依赖，纳入依赖和包体积比较。

不引入第二套 TLS 后端、外部 `autossh`、每条 SQL 一个 SSH 进程，或持续运行的后台守护服务。

## 2. 代码调研结论

| 已确认事实 | 代码位置 | 对实施的影响 |
| --- | --- | --- |
| Profile 当前为 version 6，单条严格解码，支持把未知配置保留为不可用条目 | `src/profile.rs:612`、`src/persistence/profiles.rs:245`、`:294` | 增加可选 `ssh` 字段，保持 version 6，避免使旧客户端拒绝整份配置 |
| 保存会合并旧字段 | `src/persistence/profiles.rs:753` | 必须显式清除模型自有的 SSH 字段，避免关闭 SSH 后被旧数据恢复 |
| Runtime 按完整 target / identity 管理并发连接 | `src/runtime/connections.rs:6`、`src/runtime.rs:4229` | 隧道生命周期要与数据库 target 生命周期分开 |
| CLI、Agent、MongoDB Agent、LSP 各有独立建连入口 | `src/connections/cli.rs:575`、`src/agent/service.rs:131`、`src/agent/mongodb.rs:84`、`src/lsp/catalog.rs:518` | 仅修改 TUI Runtime 不足以交付完整功能 |
| PostgreSQL 普通查询中断会额外新建控制连接 | `src/db/pgwire/mod.rs:141` | 连接池补建、取消和事务任务必须继承路由并持有租约 |
| Tiberius 接收调用方建立的 stream | `src/db/mssql.rs:192` | 可保留原始 TLS hostname，单独改实际 TCP 入口 |
| Redis 当前使用 ConnectionManager | `src/db/redis/mod.rs:97` | 其锁定版本没有把自定义 resolver 透传给 manager，TLS 路由需要额外适配 |
| MongoDB seeds 独立于 profile.host 保存 | `src/db/mongodb/mod.rs:372` | 不能只改 profile.host；多节点需要覆盖发现和监控连接 |
| 数据库凭据存储绑定 profile UUID | `src/persistence/credentials.rs:70`、`src/persistence/local_credentials.rs:201` | SSH 凭据不能占用数据库密码槽位 |

依赖接口的额外结论：

1. SQLx 0.9.0 的 Pg / MySQL TCP 建连与 TLS hostname 使用同一个 host，没有公开的独立 TCP 拨号覆盖接口。Pg URL 的 `hostaddr` 也会覆盖这个 host，不能照搬 libpq 的独立地址语义。
2. SQLx 的 Unix socket 路径值得验证：socket 决定物理传输，host 仍可供 TLS 使用。该路径不是 Windows 方案；文档与实现对 socket TLS 的描述存在差异，必须做真实握手验证。
3. reqwest 支持 SOCKS；这比单纯 DNS override 更适合保留原 URL、显式端口、HTTP authority 和 IP SAN。仅调用 `.resolve(host, loopback:port)` 不足以覆盖所有情况。
4. MongoDB SOCKS5 路径可以保留原始节点 hostname 后再进行 TLS。SRV / TXT 查询是另外的 DNS 流程，不能宣称会自动进入 SSH。
5. Oracle 当前 adapter 构造 Easy Connect 地址，尚未把 `ssl_mode` 完整接入 TCPS；不能在此功能中顺带承诺 OCI TLS、wallet 或 RAC 重定向均可工作。

以上是源码调研结果，不等于相关协议、平台与性能验收已经通过。

## 3. 首版场景与驱动矩阵

### 3.1 SSH 使用场景

首版包含：

- SSH agent、OpenSSH 默认身份发现、指定私钥文件。
- 已在 agent 中解锁的加密私钥；OpenSSH 支持的密钥和证书形式由系统客户端处理。
- `~/.ssh/config` 的 Host alias、User、Port、IdentityFile、IdentityAgent，以及用户已有的 ProxyJump / ProxyCommand 配置。
- 单跳和通过 ProxyJump 配置的多跳。表单提供可选 jump 参数；复杂多跳使用 SSH config。
- IPv4、IPv6、自定义 SSH 端口；数据库主机从最终跳板机一侧解析和访问。
- 同一 profile 的多数据库 / schema、多 Console、Explorer、Dashboard、Test、discovery 共用隧道。
- macOS、Linux、Windows，按下表明确平台差异。
- headless 的 connections CLI、Agent / MCP、LSP；协议 stdin/stdout 不参与认证交互。

首版采用非交互认证。密码登录、未解锁私钥的 passphrase、keyboard-interactive / MFA、首次主机确认需要交互时，返回分类错误和下一步操作。用户可先通过系统 ssh / ssh-add 完成信任和解锁。

### 3.2 数据库传输

| 驱动 | 推荐传输 | 首版目标与验收边界 |
| --- | --- | --- |
| PostgreSQL / CockroachDB | `disable` 使用 TCP forwarding；所有 TLS 模式使用 Unix socket forwarding；Windows 的 `prefer` / `require` 使用 TCP | Unix socket 保留原 hostname、SNI、`verify-full` 和现有 client cert。Windows 不降低严格 TLS；非校验 TCP 的 SNI 限制明确记录 |
| MySQL / MariaDB / TiDB | `disable` 使用 TCP forwarding；所有 TLS 模式使用 Unix socket forwarding；Windows 的 `prefer` / `require` 使用 TCP | Unix socket 保留 hostname / SNI，并验证 TLS、身份验证插件、池补建和事务。Windows 严格 TLS fail closed，非校验 TCP 的 SNI 限制明确记录 |
| SQL Server | TCP forwarding，Config.host 保持原值 | 保留现有 TLS 模式语义，覆盖 pool factory、跨 database 和事务；服务端重定向不作为首版保证 |
| Redis / Valkey | TCP forwarding + 原 host 的 resolver 路由 | 支持 ACL / AUTH / SELECT、TLS 和受控重连；需要完成 ConnectionManager 接口缺口的验证与适配 |
| ClickHouse | OpenSSH dynamic forwarding + reqwest SOCKS5 | HTTP / HTTPS、原始端口、Host、SNI、CA、IPv6 / IP SAN；保持既有查询取消行为 |
| Elasticsearch / OpenSearch | 同上 | HTTP / HTTPS、basic / no auth、原始 endpoint 与现有 CA；不启用绕过隧道的节点发现 |
| MongoDB | OpenSSH dynamic forwarding + 驱动 SOCKS5 | 单节点、显式 seeds、副本集节点发现 / 监控 / failover；不自动设置 directConnection。SRV 仍要求客户端能解析 SRV/TXT，并明确诊断此限制 |
| Oracle | TCP forwarding，仅作为条件扩展 | 通过真实 OCI 验收后才列入支持；首轮只考虑单 endpoint TCP，不承诺 TCPS、RAC / SCAN、重定向和 wallet |
| SQLite / DuckDB | 不适用 | 禁止保存启用 SSH 的组合 |
| BigQuery / Snowflake | 本轮不接入 | 当前托管 HTTPS / 认证路径保持独立，配置 SSH 时明确提示不支持 |

这里的“首版目标”是该组合必须通过测试后才开放，不是用文档掩盖尚未完成的适配。SQLx 严格 TLS、Redis TLS 重连、MongoDB 拓扑是独立验收门槛。

Redis Cluster / Sentinel、自定义数据库拓扑、任意远端 Unix socket、SOCKS UDP / 通用 VPN 不纳入首版。不同 profile 的跨配置隧道合并也不做，降低认证隔离和配置变更成本。

## 4. 配置与用户行为

### 4.1 Profile

在 `ConnectionProfile` 中新增：

```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
pub ssh: Option<SshOptions>,
```

`SshOptions` 使用严格的 Serde 字段检查，包含：

| 字段 | 含义 | 默认行为 |
| --- | --- | --- |
| `host` | SSH host 或 SSH config alias | 启用时必填 |
| `port` | SSH 端口 | 未设置则交给 OpenSSH config / 默认端口 |
| `user` | SSH 用户名 | 未设置则交给 OpenSSH |
| `identity_file` | 可选私钥路径 | 未设置则使用 config / agent |
| `config_file` | 可选 SSH config 路径 | 未设置则使用系统默认配置 |
| `proxy_jump` | 可选 jump 参数 | 未设置则使用 SSH config |
| `known_hosts_file` | 可选主机信任文件 | 未设置则使用 OpenSSH 默认文件 |

不新增重复的数据库 remote_host / remote_port。原 profile.host / port 就是从最终跳板机访问的数据库地址；传输入口只存在于运行时，不写回 profile、URL 或工作区。

首版不保存 SSH password、passphrase、私钥内容和 agent 中的材料。关闭 SSH 表示 `ssh = None`，不是遗留一个 enabled=false 的第二种状态。

表单提供 SSH 开关、Host / alias、User、Port、密钥路径；config、jump、known_hosts 放到高级选项，使用现有滚动和字段交互。数据库 URL 预览继续显示数据库本身的逻辑地址，SSH 作为独立连接设置。

更新 `config/default.toml` 的 profile 示例和默认说明，遵循项目“配置变更同步文档和测试”的约定。在 `src/config.rs` 的 ConnectionsConfig 中增加 SSH defaults；只暴露连接总超时和 keepalive 三个有实际使用价值的默认项：

```toml
[connections.ssh]
connect_timeout_seconds = 30
server_alive_interval_seconds = 30
server_alive_count_max = 3
```

这些值以嵌入的默认配置为唯一来源，增加覆盖、边界与旧配置合并测试。bind 重试次数、日志上限等保持内部有界策略，不为所有内部常量新增全局设置项。

### 4.2 CLI

`connections add` 增加对应 SSH 选项和 `--no-ssh`：

- 首次添加未指定 SSH 时保持直连。
- `--upsert` 未提供 SSH 选项时保留已有 SSH 配置。
- 提供选项时按明确更新语义设置；清除已有 SSH 使用 `--no-ssh`。
- `--no-ssh` 与所有 SSH 设置参数冲突。
- 数据库 URL 和 SSH 配置分开；不把 SSH 密码加入 URL。
- show / list / JSON 输出只增加安全摘要；不回显凭据、原始诊断全文或私钥内容。
- `connections test --timeout` 的总 deadline 包含 SSH 检查、启动和数据库 probe，不叠加多份超时。

对 SSH 冷连接，TUI 使用上述默认 30 秒总 deadline；普通直连保留现有 10 秒。CLI `connections test --timeout` 保留当前默认 10 秒及现有参数范围，用户可为较慢的多跳认证显式设置 `--timeout 30`。调用方较短的总 deadline 优先，不在实现中叠加多份超时。

### 4.3 兼容与持久化

- 保留 profile 文件 version 6；添加可选字段，不进行全文件结构迁移。
- 旧版只要具有当前逐条兼容机制，就会把含未知 ssh 字段的条目标记不可用，保留其他连接及原文；不能泛化为所有历史版本都支持。
- `merge_missing_profile_fields` 前移除旧的模型自有 ssh 表，再使用当前强类型结果，保证清空选项、关闭 SSH、切换 driver 不会恢复旧字段。
- SSH 配置进入 discovery fingerprint 和连接相关的 ProfileChange 比较；改名、换分组不需要重建隧道。
- 复制 profile 只复制非秘密配置。删除 / 改 SSH 配置只影响该 profile 的租约和相应连接。
- SSH config / agent 的外部变化不做持续监听；关闭相关连接后重新连接时重新解析。

## 5. 公共连接入口与生命周期

### 5.1 类型和职责

新增 `DatabaseConnector`，由 TUI Runtime、每个长生命周期 AgentService / MCP 服务和 LSP 服务各自持有。CLI 单次命令建立短生命周期实例。不要用全局可变单例，避免跨测试、配置根和服务生命周期串扰。

目标接口：

```rust
pub struct DatabaseConnector {
    tunnels: Arc<TunnelManager>,
}

pub struct ConnectionRoute {
    endpoint: TransportEndpoint,
    lease: Option<TunnelLease>,
}

pub enum TransportEndpoint {
    Direct,
    Tcp(SocketAddr),
    Unix(PathBuf),
    Socks5(SocketAddr),
}
```

具体可见性按现有模块边界收敛。只在需要共享的地方使用 Arc，不把 SQLx / TLS 的具体类型暴露给表单和 Runtime。

连接步骤：

1. 校验 driver / TLS / SSH 组合和执行 target。
2. 使用不可变原始 profile 解析数据库身份及认证信息。
3. 申请或复用 tunnel lease。
4. 将 route 传给具体 adapter 的连接构造。
5. 运行数据库现有 probe。
6. Runtime 继续按 profile revision、target、attempt identity 判断候选是否仍可安装；过期结果直接释放。

保留 `DatabaseConnection` 的驱动 enum，避免为加一层 transport 大规模重写业务 match。生产入口统一迁移到 connector；保留的直连辅助 API 必须在收到 SSH profile 时拒绝，不能意外绕过隧道。

### 5.2 复用与所有权

隧道 key 至少包含：

- profile UUID。
- SSH 连接配置指纹。
- 转发模式。
- 静态转发的远端 host / port；动态转发不按每个发现节点再创建 SSH。

数据库名、schema、Console UUID、connection generation 不进入 key。不同数据库目标保留独立数据库连接，却共享相同传输。

同 key 的并发建立采用 single-flight。等待者有各自 deadline 和取消状态；取消一个等待者不终止其他等待者。没有等待者和租约时，取消尚在启动的进程。

租约随 adapter、pool factory、adapter clone、手工事务 worker、取消控制连接和后台关闭任务保留。不能只把租约放在 Runtime 的 ActiveConnection，否则后台任务会提前失去传输。

最后一个使用者离开后停止隧道；首版不做长时间空闲缓存。Runtime shutdown 先停止新工作，再处理事务 / 控制任务，最后有界关闭 tunnel manager，避免只依赖 Drop 中无法 await 的清理。

隧道退出会使当前 transport epoch 失效，拒绝新租约并通知相关连接失效。已有 SQL / 事务报告失败或结果未知，不自动重放。后续重新连接创建新 epoch 和路由，旧连接池不能拿新隧道继续执行旧事务。

## 6. OpenSSH 子进程与可靠启动

### 6.1 启动约束

- 解析并固定 ssh 可执行程序的绝对路径；Windows 优先识别系统 OpenSSH。检查实际能力，不能仅按版本号假定 Apple / Windows fork 的行为一致。
- 通过 `Command` 的独立参数传值，不拼接 `sh -c` / PowerShell 命令。
- 校验 host、user、jump、端口和路径，拒绝控制字符、换行及选项注入；正确处理 IPv6 与路径空格。
- `-N -T`，不运行远端命令，不分配 PTY；禁用继承的 RemoteCommand、LocalCommand、后台 fork 和外部 ControlMaster 复用。
- 首版使用 BatchMode，禁止意外抢占 TUI / MCP / LSP 的 stdin，避免隐式 askpass。此约束要覆盖 ProxyJump 派生客户端，不能假定主进程参数会全部传给跳板进程；隔离 controlling terminal、askpass 与子进程树的行为纳入平台验收。
- 主机密钥采用严格校验；未知主机或密钥变化返回明确错误。复用已有 known_hosts，但不提供自动接受变化的开关。
- 仅监听 `127.0.0.1`，不开放到局域网。Unix socket 使用短、私有的临时目录和受限权限。
- 使用 ExitOnForwardFailure、keepalive 和总 deadline。keepalive 建议 30 秒、3 次，不用高频主动探测代替。
- stdout 不污染协议通道；stderr 持续异步排空、清洗和限量保留，建议 ring buffer 64 KiB，并限制单行大小。
- 默认不启用额外 SSH 压缩，不覆盖系统密码算法选择。

SSH config 中的 ProxyCommand / Match exec 属于用户信任的本地配置，可能运行本地命令；不把任意 SSH shell 参数作为 profile 或 CLI 的通用透传接口。

### 6.2 避免端口冲突和虚假 ready

不能把“随机端口目前可以连接”当成自己的 ssh 已经就绪。

跨平台 TCP / SOCKS 路径的拟定流程：

1. 用一次性 loopback bind 获取候选端口；释放后启动 ssh，承认这里存在端口竞争。
2. ExitOnForwardFailure 负责使 bind / listen 失败变成失败结果；端口冲突最多换端口重试 3 次，且受同一总 deadline 约束。
3. 从本次受管子进程获取匹配转发目标的启动证据，并确认已完成监听初始化、子进程未退出，之后才交给驱动 probe。
4. OpenSSH 的 “Local forwarding listening” debug 消息在 bind 前就会输出，不能单独作为成功依据。PoC 应验证其后监听 channel 创建 / 完成初始化的有序证据，覆盖嵌套 ProxyJump 的 stderr 混流。
5. 如果目标 OpenSSH 实现无法提供经验证的完成判定，返回不支持的启动行为；不能降级成固定 sleep 或仅探测端口。
6. 数据库 probe 是另外一层：本地转发创建成功不代表跳板机能访问最终数据库。

Unix socket 路径由进程持有私有目录，避免 TCP 端口竞争；不删除不属于当前 lease 的旧 socket。

OpenSSH 配置处理必须验证两个容易误用的行为：

- `ClearAllForwardings=yes` 会清除配置和命令行中的 forwarding，不能与自己的 `-L` / `-D` 组合后假定它仍生效。
- 首版用 `ssh -G` 对同样的连接参数进行有界 preflight；如果继承了额外 LocalForward / RemoteForward / DynamicForward，则明确报冲突并提示使用专用 alias，不默默开放额外端口。保留 config 的原生解析，不自行实现 SSH config 解释器。

不要把 Unix ControlMaster 作为跨平台必需条件；Windows 客户端必须独立验收。

### 6.3 退出与故障

- 正常断开、连接失败、认证失败、超时、取消、task abort、服务 EOF / shutdown 都释放当前使用者的租约。
- Unix 使用专属进程组 / session；Windows 使用 Job Object；终止 owned 进程树后 await 回收，包含 ProxyJump 派生进程。
- 关闭其他 target、Test 或 discovery 不会误杀仍被使用的隧道。
- 未知主机、主机密钥变化、认证失败不自动反复重试；临时网络失败和 bind 冲突分别采用有界策略。
- Unix 父进程遭 SIGKILL 时不会执行 Drop，进程组也不等于自动父死清理。首版准确说明此边界，不额外常驻 guardian 来制造清理保证。
- 不依赖裸 PID 文件在下次启动时杀进程，避免 PID 复用误杀。

错误至少区分：ssh 不存在 / 不兼容、配置无效或冲突、host key 未信任 / 已变化、需要交互、认证失败、SSH 网络失败、启动超时、转发端口冲突、远端目标不可达、数据库 TLS / 数据库认证失败、隧道运行中退出。

机器接口提供稳定错误 code；用户消息给出必要的下一步操作。凭据和远端 banner 不直接进入终端或 JSON。

## 7. 驱动接入细节

### 7.1 SQLx：Pg / CockroachDB / MySQL 系列

- `verify-ca` / `verify-full` 以 `ssh -L local_socket:remote_host:remote_port` 接入。Pg socket 文件名按 `.s.PGSQL.{port}` 规则生成，host 保持原数据库名，socket 目录单独传给 connect options。
- `disable` 使用 TCP forwarding，因为该模式不涉及 TLS server name。`prefer` / `require` 虽然不校验证书 hostname，但仍可能用 hostname 做 SNI，因此和 `verify-ca` / `verify-full` 一样使用 Unix socket，避免静默改变服务器路由或握手行为。
- MySQL 同样分开 host 与 socket path，验证认证插件不会因本地 socket 形式错误切换成数据库服务器本地 socket 的认证语义。
- 保留 read-only、search_path、startup options、现有 CA / client cert 和连接池限制。
- Pg 查询取消的额外连接与 Pg / MySQL 事务取消沿用同一路由，且取消任务完成前持有 lease。
- Windows 的 `prefer` / `require` 使用 TCP，但 TLS server name 会变成 loopback IP，文档明确列出 SNI 兼容性限制；`verify-ca` / `verify-full` fail closed。macOS/Linux 的 TCP 路径只用于 `disable`。
- 首版不 fork / vendor 整个 SQLx、不改系统 hosts、不接管全局 DNS。若 Windows 严格 TLS 是发布必需项，单独评估上游增量 connector / server-name API 的成本后再决定。

### 7.2 Tiberius

- 在 MsSqlConnectSettings 中区分逻辑 host / port 与物理 SocketAddr。
- `Config.host`、数据库身份和认证保持原始值；TCP dial 使用 route。
- pool factory、for_database、事务和重连都 clone 同一受管 route。
- 不顺带改变当前 Prefer / VerifyCa 的既有语义。

### 7.3 Redis / Valkey

- 优先使用 `AsyncConnectionConfig::set_dns_resolver`，让 host 与 TLS ServerName 保持原值，解析结果返回固定 loopback 地址。
- 当前 ConnectionManagerConfig 未透传 resolver，先比较可用的上游兼容接口与小型应用层适配。
- 若没有可直接采用的上游接口，新增 tunnel 专用的受管 multiplexed connection，复用现有重连策略；直连继续使用现有 ConnectionManager。
- 包装层保持现有 `self.connection.clone()` / ConnectionLike 使用方式，避免逐个改写所有 Redis 命令。
- 重建采用 single-flight；保留 AUTH / SELECT、超时和请求限制。连接错误后只重建供后续请求使用，不自动再次发送结果未知的写命令。
- 不另写 TLS 握手、不更换 TLS 后端，不把私有依赖源码 patch 作为默认方案。

### 7.4 HTTP 驱动

- reqwest client 使用显式 `socks5h://127.0.0.1:port`，保留原 URL、HTTP authority、TLS SNI、CA 和身份校验。
- 关闭 tunnel client 的系统代理自动选择和 NO_PROXY 绕过，防止请求走其他路径；无 SSH 的 client 维持现有系统代理行为。
- 不使用容易遗漏显式端口或 IP literal 的全局 DNS override。
- 沿用禁止自动重定向的策略；poll / cancel / metadata 请求全部复用同一个 route。
- 数据只经过驱动与 OpenSSH，不新增 LazyDB 的 HTTP CONNECT relay。

### 7.5 MongoDB

- 开启并测量驱动 SOCKS5 feature，保持原始 seeds 和 ClientOptions。
- 业务连接、监控连接、发现节点都必须使用 SOCKS，不能只代理首次 seed。
- 覆盖单节点、副本集、primary 切换、不同节点端口、TLS 与认证。
- `mongodb+srv` 的 SRV/TXT 仍由客户端解析；解析不可达时返回针对性错误，建议显式 seeds。首版不自建远端 DNS 协议或解析 SRV 代理。
- 不自动改变 replicaSet、directConnection、readPreference 或事务选项。
- 隧道退出后的重建服从现有 MongoDB 事务 / retry policy，不额外增加应用层写重试。

## 8. 任务拆分与文件范围

下列测试与模块名称是实施时拟新增的目标，不表示当前已经存在。每个任务按“添加行为测试 → 确认失败 → 实现最小完整改动 → 定向验证”的顺序完成。是否 commit / push 以实施时用户授权为准，本计划不自动执行提交。

### Task 1：完成四项技术验证，固定支持矩阵

**Files:**

- Create: `tests/ssh_transport.rs`
- Create: `tests/fixtures/ssh/README.md`
- Create: `tests/fixtures/ssh/sshd_config`
- Create: `docker-compose.ssh.yml`
- Create: `scripts/ci/test-ssh-integration.py`

**Steps:**

1. 建立隔离 sshd / 数据库 fixture；密钥与证书运行时生成，私钥不提交。
2. 验证 OpenSSH TCP / SOCKS 启动成功、被抢占端口失败、认证失败、ProxyJump 混合日志和取消清理。
3. 验证 SQLx Unix socket 严格 TLS：正确证书成功，错误 CA / hostname 失败，server 不支持 TLS 时 Require 失败；检查 client cert 和连接池。
4. 验证 Redis resolver + 重连、MongoDB SOCKS5 + 副本集、HTTP 显式端口 / IP SAN。
5. 记录每项实测平台、OpenSSH / 驱动版本及结果；未验证或失败的组合不得标成已支持。
6. 核对新增 Cargo features 的依赖差异，固定后续实现选型，不并行维护多个试验 backend。

**Acceptance:** 形成可重复的协议结果和明确支持矩阵；若 SQLx Unix TLS 不成立，先调整发布范围或独立依赖方案，不进入“默认关闭校验”的实现。

### Task 2：Profile、兼容和 CLI 参数契约

**Files:**

- Modify: `src/profile.rs`
- Modify: `src/config.rs`
- Modify: `src/persistence/profiles.rs`
- Modify: `src/model/profile_manager.rs`
- Modify: `src/cli.rs`
- Modify: `src/connections/cli.rs`
- Modify: `config/default.toml`
- Create: `tests/ssh_profiles.rs`
- Modify: `tests/profile_compatibility.rs`
- Modify: `tests/profile_transactions.rs`
- Modify: `tests/connections_cli.rs`

**Steps:**

1. 测试旧 profile 无 SSH 的 round-trip、新字段、未知字段、无效 driver / TLS 组合和输入注入。
2. 测试开启 / 关闭 SSH、清除 identity_file、切换 driver、并发保存，不恢复旧字段。
3. 定义 SshOptions、校验和 discovery fingerprint，补齐生产和测试中的 profile 构造。
4. 定义 CLI 保留 / 设置 / 清除语义，覆盖 upsert，不把数据库 URL 改写成本地入口。
5. 在默认配置示例和配置文档中标明 SSH 依赖与认证边界。

Run:

```sh
cargo test --locked --test ssh_profiles --test profile_compatibility --test profile_transactions --test connections_cli
cargo test --locked --lib config::tests
```

**Acceptance:** 无 SSH 的现有配置不变；旧客户端兼容机制仍按条目降级；删除 SSH 不会在保存后复活。

### Task 3：OpenSSH 命令、进程所有权和共享 manager

**Files:**

- Create: `src/ssh/mod.rs`
- Create: `src/ssh/openssh.rs`
- Create: `src/ssh/manager.rs`
- Create: `src/ssh/error.rs`
- Modify: `src/lib.rs`
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Create: `tests/ssh_options.rs`
- Create: `tests/ssh_manager.rs`

**Steps:**

1. 为进程启动边界建立可控 fixture，测试参数、IPv6、空格路径、stdin、限量日志、状态和错误映射。
2. 实现 preflight、受管启动、经 Task 1 验证的 ready 判定、最多 3 次 bind 重试和数据库 probe 前门禁。
3. 实现 key、single-flight、独立等待者取消、lease、transport epoch 与有界 shutdown。
4. 验证 20 个同 key 并发申请只产生一个主 ssh，取消一个等待者后其他申请仍完成。
5. 验证最后 lease 释放、启动超时、abort、ProxyJump 进程树和 Windows Job Object 清理。
6. 确认没有持续持有强引用的 manager / monitor 环。

Run:

```sh
cargo test --locked --test ssh_options --test ssh_manager
```

**Acceptance:** 进程计数、并发、取消、日志内存和回收可观察且有界；不能以 mocked 成功结果替代真实 sshd 启动验收。

### Task 4：公共 connector 与 SQLx / Tiberius

**Files:**

- Create: `src/connections/connector.rs`
- Modify: `src/connections.rs`
- Create: `src/db/transport.rs`
- Modify: `src/db/mod.rs`
- Modify: `src/db/pgwire/mod.rs`
- Modify: `src/db/postgres.rs`
- Modify: `src/db/cockroach/mod.rs`
- Modify: `src/db/mysql.rs`
- Modify: `src/db/mssql.rs`
- Modify: `tests/ssh_transport.rs`
- Modify: `tests/postgres_adapter.rs`
- Modify: `tests/mysql_adapter.rs`
- Modify: `tests/execution_target.rs`

**Steps:**

1. 测试原始逻辑 endpoint 不被修改、route 被 adapter 保留、未知组合 fail closed。
2. 实现 DatabaseConnector 和 route，维持既有 DatabaseConnection enum 及普通直连。
3. 接入 Task 1 已验证的 SQLx Unix socket / Windows TCP 组合和 Tiberius 分离拨号。
4. 测试普通查询、catalog、database target、池补建、手工事务、满池取消和断线。
5. 确保 Pg 额外控制连接及后台关闭任务保留 lease，不回退原 host 直连。

Run:

```sh
cargo test --locked --test ssh_transport --test execution_target --test postgres_adapter --test mysql_adapter
```

**Acceptance:** 本地确定性测试通过；真实 DB 集成测试必须配置 fixture 后执行，不能把未配置时跳过的结果计为已验证。

### Task 5：Redis、HTTP、MongoDB 接入

**Files:**

- Create: `src/db/redis/connection.rs`
- Modify: `src/db/redis/mod.rs`
- Modify: `src/db/redis/reconnect.rs`
- Modify: `src/db/clickhouse/transport.rs`
- Modify: `src/db/elasticsearch/mod.rs`
- Modify: `src/db/mongodb/mod.rs`
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Modify: `tests/ssh_transport.rs`
- Modify: `tests/mongodb_connection.rs`
- Modify: `tests/clickhouse_adapter.rs`

**Steps:**

1. 按 Task 1 结论实现最小 Redis routed connection，保留 AUTH / SELECT 和单次发送语义。
2. 为三个 HTTP 驱动接入同一 SOCKS client 配置辅助函数；验证系统代理 / NO_PROXY 不绕过隧道。
3. 为 MongoDB ClientOptions 设置驱动代理，覆盖 seeds、监控、发现与 failover。
4. 验证正确证书成功、错误证书失败、原始 SNI / authority、IPv6、自定义端口与大结果流。
5. 测试断线后新请求的受控重连，不重复 Redis 写操作或添加 MongoDB 应用层 retry。
6. 测量 Cargo feature 与发布产物差异，确认没有意外开启其他压缩或 TLS backend。

**Acceptance:** 原始 hostname 与 TLS 语义保留；各驱动不因 tunnel 断开走直连；MongoDB 的 SRV 边界有测试和文档。

### Task 6：TUI 与 Runtime 的完整连接流程

**Files:**

- Modify: `src/model/profile_manager.rs`
- Modify: `src/ui/profiles.rs`
- Modify: `src/input/keymap.rs`
- Modify: `src/action.rs`
- Modify: `src/commands.rs`
- Modify: `src/app.rs`
- Modify: `src/runtime.rs`
- Modify: `src/runtime/connections.rs`
- Modify: `src/runtime/transaction.rs`
- Modify: `tests/profile_draft.rs`
- Modify: `tests/profile_runtime.rs`
- Modify: `tests/connection_switch.rs`
- Modify: `tests/console_lazy_connection.rs`
- Create: `scripts/ci/test-ssh-tui.py`

**Steps:**

1. 增加表单字段和验证；使用既有快捷键体系，不另加无必要的全局快捷键。
2. Test、discovery、连接、保存并连接统一通过 connector，保持 App::update 无副作用。
3. 显示必要阶段：SSH 检查 / 建立、数据库连接、失败原因；成功后保持现有连接状态体验。
4. 为关闭表单、切换 profile、过期 attempt 增加真正取消，保留 target / generation 隔离。
5. 测试多个 Console / 数据库共用 tunnel，一个 Test 失败不影响已有连接，最后使用者断开后回收。
6. 用真实 PTY 验证表单、Test、Explorer、SQL、取消、重连、退出与终端恢复。

**Acceptance:** SSH 不阻塞 UI 事件循环；正常直连无新开销；连接路由变更不会让旧结果覆盖新 target。

### Task 7：CLI、Agent / MCP、LSP

**Files:**

- Modify: `src/connections/cli.rs`
- Modify: `src/agent/service.rs`
- Modify: `src/agent/mongodb.rs`
- Modify: `src/agent/mcp.rs`
- Modify: `src/lsp/catalog.rs`
- Modify: `src/lsp/server.rs`
- Modify: `tests/connections_cli.rs`
- Modify: `tests/agent_service.rs`
- Modify: `tests/agent_mcp_protocol.rs`
- Modify: `tests/lsp_catalog.rs`
- Modify: `tests/lsp_protocol.rs`
- Create: `tests/ssh_integration.rs`

**Steps:**

1. 迁移所有生产建连入口，长期服务共享其 manager；不为每次请求新建独立 tunnel manager。
2. Test 的总 deadline 覆盖 SSH；机器错误区分 interaction_required、host key、网络和数据库失败。
3. MCP / LSP 的 stdout 只承载协议数据，stdin 不被 ssh 使用；不弹出隐式认证。
4. 在 shutdown / EOF 路径有界清理连接、取消任务和隧道。
5. 搜索剩余 DatabaseConnection::connect / adapter 直接 connect，检查 Test、discovery、catalog、monitor 与控制连接没有漏接。

Run:

```sh
cargo test --locked --test connections_cli --test agent_service --test agent_mcp_protocol --test lsp_catalog --test lsp_protocol
```

**Acceptance:** 相同 profile 在 TUI、CLI、Agent、MCP、LSP 使用同样的路由和安全语义，保持项目访问策略、只读和结果预算。

### Task 8：平台验收、性能体积与文档

**Files:**

- Modify: `.github/workflows/ci.yml`
- Modify: `scripts/release/report-client-size.py`，仅在现有输出不足时修改
- Modify: `tests/fixtures/ssh/README.md`
- Modify: `docs/ssh-tunnel.md`
- Modify: `docs/configuration.md`
- Modify: `docs/architecture.md`
- Modify: `docs/README.md`
- Modify: `README.md`
- Modify: `CHANGELOG.md`
- Modify: `CONTRIBUTING.md`

**Steps:**

1. 执行第 9 节确定性、真实协议和平台矩阵，记录已验证与未验证组合。
2. 在相同工具链、feature、目标和打包参数下，比较 `541d5c9` 基线与功能版本。
3. 比较直连、等价手工 OpenSSH、LazyDB 受管 OpenSSH；统计整个进程树。
4. 更新安装依赖、SSH config、密钥 / agent、ProxyJump、TLS、集群、认证和退出边界文档。
5. 将 workaround 文档改成“内置支持 + 外部转发替代方案”，按实测结果更新 TLS 矩阵，不把当前接法的局限描述成所有隧道的固有限制。
6. 完成下面的标准检查后审查最终 diff，不顺带更换驱动或扩展无关能力。

Run:

```sh
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets --all-features
cargo check --locked --no-default-features
```

**Acceptance:** 明确功能矩阵、依赖增量、发布包差异、真实协议结果和剩余限制，不用“编译成功”替代网络验收。

## 9. 测试与性能验收

### 9.1 确定性测试

- 参数解析与转义、非法输入、路径空格、IPv6、profile persistence、upsert、旧版本兼容。
- 受控进程 fixture 验证错误 code、日志限额、启动超时、单个等待者取消、所有等待者取消、lease / epoch。
- 多 target 共享、profile 配置变更、过期连接结果、服务关闭，不误杀其他租约。
- 驱动 endpoint / TLS 语义与不回退直连；不能只对照实现字段做镜像断言。
- 使用可控时钟和明确事件，不用增加 sleep 掩盖并发问题。

### 9.2 真实 sshd / 数据库

隔离 fixture 仅绑定 loopback、使用独立 compose 项目与临时资源，不接触用户数据库、known_hosts、SSH agent 内容或既有容器。

覆盖：

- 正常 key、agent、加密 key 已加载、错误 key、交互需求、未知 host key、host key 改变。
- 自定义端口、alias、config Include、ProxyJump、多跳、带空格路径。
- forward 禁用 / PermitOpen 限制、远端 DNS 失败、远端 TCP 拒绝、本地端口抢占。
- 数据库严格 TLS 正反例、SNI、CA、client cert，以及驱动现有的 Require / Prefer 行为。
- pool 满载、取消控制连接、事务中断、SSH 被杀、网络中断、反复连接 / 断开、正常退出。
- 20 个并发同路由目标、多个独立 profile，检查主进程、跳板子进程、文件描述符与临时目录。
- Windows 的 ssh、路径、agent、日志和 Job Object 单独实测，不以 Linux 结果代替。
- MongoDB 副本集业务 / 监控连接及 failover；SRV / TXT 不经 SSH 的诊断边界。

至少在发布支持的一个 macOS、Linux 和 Windows 版本执行。每个平台记录实际 OpenSSH 实现与版本；没有运行的平台保持“未验证”。

### 9.3 体积与运行开销

下列是建议验收目标，不是已经测得或保证达到的数字。Task 1 建立基线后确认，超出时先分析 feature / 依赖与实现，而不是直接放宽。

| 指标 | 建议目标 / 方法 |
| --- | --- |
| 原始 release binary | 相同构建条件下新增不超过 1 MiB |
| 压缩发布包 | 增幅不超过 2%，且绝对增量不超过 512 KiB；每个发布平台分别报告 |
| 非 SSH 用户 | 不启动 / 不探测 ssh，不增加网络中转和周期任务；重复基准中吞吐 / 延迟差异应处于约 2% 的噪声范围 |
| 受管 SSH vs 等价手工 SSH | 同一路由和加密参数下，大结果吞吐额外损耗目标不超过 5%；报告 p50 / p95、CPU 和 SSH 认证时间 |
| 进程数量 | 同 profile、SSH 配置与 route 的多数据库 / Console 只有一个主 ssh；ProxyJump 的额外 ssh 单独计数 |
| 内存 | 统计 LazyDB + 所有 SSH / jump 子进程 RSS；无流量时不随已执行 SQL 数量增长，不只展示 LazyDB 主进程 |
| 日志 | 每 tunnel ring buffer 64 KiB，单行和队列有界，长时间失败不累计全文 |
| 空闲 CPU | 没有忙轮询；使用 wait、channel 和低频 keepalive |
| 关闭回收 | 正常退出 / 取消后的 owned 进程、监听端口、socket 和临时目录回到基线 |

比较至少包括：1 / 4 / 16 个数据库会话、小查询、大结果流、长时间空闲、TLS 与非 TLS、单跳与多跳、首次连接和复用。基准需重复采样并记录机器 / 系统 / 网络条件；不要把不同机器的单次数字当回归结论。

沿用 `scripts/release/report-client-size.py` 与 `scripts/release/check-binary-size.sh`。网络验收新增独立脚本 / ignored fixture tests，不把现有 UI rendering 的 `performance_regression` 结果当网络性能证明。

### 9.4 本次实施记录

环境：macOS 27.2.1、Apple M2 Max、OpenSSH 10.5p1、Rust 1.94.1；数据库由
隔离 Docker Compose 项目提供，SQL Server 使用 `linux/amd64` 模拟。

```sh
python3 scripts/ci/test-ssh-integration.py --with-sqlserver --keep-on-failure
cargo fmt --all --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo check --locked --no-default-features
LAZYDB_DUCKDB_LIB_DIR=/private/tmp/lazydb-ssh-duckdb-runtime/aarch64-apple-darwin \
  cargo test --locked --all-targets --all-features
```

发布包比较使用 `541d5c9` 的相同 target、feature 和 release profile：

| 产物 | 基线 | 当前 | 增量 |
| --- | ---: | ---: | ---: |
| 原始 `lazydb` | 64,038,656 bytes | 64,892,752 bytes | 854,096 bytes（+1.33%） |
| `xz -6` | 16,066,400 bytes | 16,324,900 bytes | 258,500 bytes（+1.61%） |

性能测试使用同一 PostgreSQL fixture、TLS 配置和客户端证书，比较受管连接与
手工 OpenSSH TCP forwarding。每轮 3 次交替顺序采样并预热，记录 1 / 4 / 16
会话的小查询和 10,000 行结果。以下为 release 构建、macOS arm64 单机 Docker
fixture 的观测值，不是 SLA：

- 路由调整前的 `require` TCP forwarding 对照（不是最终路由）：16 会话大结果中位数
  `179.2 qps / p50 85.4 ms`，手工路径 `186.8 qps / p50 84.0 ms`；吞吐低约
  4.0%，p50 高约 1.7%，在本次目标的 5% 附近。另一轮对照为 `171.8` 对
  `174.2 qps`。该路径因 SNI 兼容性问题已不用于 `require`。
- 最终路由的 `require` 对照保留了原始 hostname / SNI 和 client
  certificate。16 会话大结果中位数为 `137.2 qps / p50 112.8 ms`，手工
  TCP 路径为 `185.9 qps / p50 84.7 ms`，吞吐低约 26.2%，p50 高约
  33.2%；4 会话大结果为 `106.0` 对 `164.9 qps`，p50 `33.7` 对
  `24.4 ms`。冷连接 `97.6` 对 `75.4 ms`。这是当前明确的性能权衡，不能
  宣称 TLS 路径满足 5% 目标。该样本使用 `require`，不是 `verify-full`；
  两者最终都走同一条 Unix socket 数据路径。
- 仅比较 SQLx 原始流时，16 会话大结果 Unix socket 中位数为 `179.5 qps /
  p50 88.4 ms`，TCP 为 `196.4 qps / p50 81.1 ms`；Unix 路径约低
  8.6%，p50 高约 9.0%。4 会话原始流两路径接近，表明受管对照还叠加了
  driver/pool 调度和 Docker 噪声。
- 同一 profile 的 16 个会话只启动一个受管主 SSH 进程，未按会话复制 SSH。

受管 Unix socket 与手工 TCP 对照并非同一种本地传输，样本也来自同一
macOS / Docker 环境的一轮三回交替采样，机器和容器调度仍有噪声；没有完成
整个进程树的系统性 CPU / RSS、Linux 原生客户端、Windows 原生客户端或真实
广域网测量。严格 TLS 的差距仍需后续评估 SQLx connector 或其他不降低证书
校验与 SNI 语义的方案，不能为了性能静默切换到 TCP。

## 10. 后续阶段的明确边界

### 应用内 SSH password / passphrase / MFA

这类认证需要安全交互桥，而不仅是在 profile 上加一个密码输入框：

- 验证 OpenSSH askpass 在 macOS / Linux / Windows 和 ProxyJump 下的行为。
- 可评估复用同一 LazyDB binary 的隐藏 helper 与私有 IPC，不额外分发 helper 包。
- 每次请求绑定 tunnel attempt、目的主机和认证用途，限制大小、次数和总 deadline。
- 默认只在内存持有 SecretString，SSH password、key passphrase、数据库密码分开。
- 不把秘密写入命令行、环境变量值、日志或明文临时文件。
- TUI 可以明确请求用户交互；Agent / MCP / LSP 默认返回 interaction_required，不能读取协议 stdin。
- 若增加“记住 SSH 凭据”，使用独立 service / credential ID / AAD，设计保存失败回滚、清除和迁移，不修改数据库密码槽位的含义。

首次主机指纹确认另做显式界面和可信确认流程，不通过自动接受未知 / 改变的 host key 实现。

### Windows SQLx 严格 TLS 与其他拓扑

优先评估 SQLx 上游独立连接地址 / connector 支持，按代码和包体积测量决定是否采用最小补丁。Oracle TCPS / wallet / RAC、Redis Cluster / Sentinel、私有 SRV DNS 等逐项立项，不在首版引入通用 VPN、DNS server 或第二套 SSH 栈。

## 11. 资料与实施时复核

官方资料：

- OpenSSH `ssh(1)`：`https://man.openbsd.org/ssh`
- OpenSSH `ssh_config(5)`：`https://man.openbsd.org/ssh_config`
- OpenSSH forwarding 初始化与 client loop：`https://github.com/openssh/openssh-portable`
- SQLx Pg / MySQL connect / TLS / options：`https://github.com/launchbadge/sqlx`
- Tiberius 0.12.3 Client / Config：`https://docs.rs/tiberius/0.12.3/tiberius/`
- reqwest 0.12.28 Proxy / ClientBuilder：`https://docs.rs/reqwest/0.12.28/reqwest/`
- Redis 1.5.0 AsyncConnectionConfig / ConnectionManagerConfig：`https://github.com/redis-rs/redis-rs`
- MongoDB 3.8.2 SOCKS5 / ClientOptions / stream：`https://github.com/mongodb/mongo-rust-driver`
- process-wrap 9.1.0：`https://docs.rs/process-wrap/9.1.0/process_wrap/`

实施时以 `Cargo.lock` 和对应实际源码为准，不因 latest 文档出现新接口而假定当前依赖已有该能力。完成 Task 1 后在本文件记录实际验证结果，再固定最终平台 / 驱动支持承诺。
