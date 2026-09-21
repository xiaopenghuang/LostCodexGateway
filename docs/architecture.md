# LostCodexGateway — 架构设计

> 版本：v1.0（M0 定稿）｜日期：2026-09-20
> 环境实测基础：见 `docs/m0-environment-report.md`（Windows 10 19045 / Clash Verge Rev 2.5.2 / Codex CLI 0.154.0 / System32 OpenSSH 9.5p1 / 本机无 sshd、Docker 可用）

## 1. 架构总览

```text
┌──────────────────────── Windows（普通用户，无提权）────────────────────────┐
│ LostCodexGateway（Tauri 2 + Vue 3 + TS + Rust）                            │
│                                                                            │
│  ┌─ ssh/     隧道管理器 ── 启动 System32 ssh.exe（参数数组）                │
│  │            PID 监管 / 有限重连 / Host Key 首确认与变化阻断              │
│  ├─ verify/  出口验证 ── SOCKS5 客户端（远端 DNS）→ 多验证端点回显出口 IP   │
│  ├─ launchers/ CLI 启动器 ── 独立 PowerShell 子进程注入代理环境变量          │
│  ├─ mihomo/  (M3) 只读检测 / 规则片段生成 / 备份与回滚                      │
│  └─ config/  配置：JSON 原子读写 + 自动备份（*.bak_时间戳）                 │
│                                                                            │
│  127.0.0.1:17801 SOCKS5 ──(ssh 加密)──► 用户自有 Ubuntu VPS（sshd 仅转发） │
└────────────────────────────────────────────────────────────────────────────┘
```

原则：
- **不接管系统代理、不改路由表、不装驱动、不提权**；所有代理注入限定在「本工具启动的子进程」与「用户确认后的 Mihomo 规则片段」。
- **不虚构生效**：`TUNNEL_READY`（隧道监听可用）≠ `EGRESS_VERIFIED`（出口实测）≠ 「某应用已走网关」（需进程级证据）。
- **SSH 连接本身永远直连**（不经过自身隧道，防环路）。
- 敏感信息（私钥内容、控制器 secret）不进普通日志；日志只记组件、状态、错误类别、脱敏地址。

## 2. 技术选型与理由

| 组件 | 选择 | 理由 |
|---|---|---|
| 框架 | Tauri 2（Rust 后端）+ Vue 3 + TypeScript + Vite | 文档指定；二进制小 |
| SSH | **不实现 SSH 协议**，调用 `%SystemRoot%\System32\OpenSSH\ssh.exe`（实测 9.5p1） | 文档指定；复用系统 Host Key 校验链 |
| 状态机 | Rust 端权威状态，事件推送前端 | UI 只展示，不自行推断 |
| SOCKS 验证 | 自研最小 SOCKS5 客户端（无认证、CONNECT 域名=远端 DNS）+ reqwest 经 socks5 拉取验证端点 | 避免依赖系统 curl/代理设置 |
| 配置 | `%APPDATA%\LostCodexGateway\config.json`，原子写（temp+rename）+ 写前备份 | 文档要求 |
| 进程发现 | Windows 只读 API（WMI/tasklist 封装，M3 再按需细化） | 需要 PID/路径/父进程 |
| 测试夹具 | Docker 自建 sshd 容器（ed25519 密钥、AllowTcpForwarding yes、独立端口） | 本机无 sshd（实测），夹具可复现 |
| 打包 | Tauri bundler → NSIS 安装包 | 文档 M4 要求 |

## 3. 模块设计

### 3.1 `config`（P0）
- `config.json`：server（host/port/user/key_path/socks_port/ssh_exe_path）、verify（端点列表、超时）、settings（自动重连开关、断线策略）、mihomo（检测缓存，不含 secret）。
- 原子写：写临时文件 → rename；写前复制 `config.json.bak_<yyyyMMdd_HHmmss>`；读失败回退最近备份并提示。
- **私钥只存路径字符串，永不读取/复制内容。**

### 3.2 `ssh`（P0）
- 启动参数（参数数组，无 shell 拼接）：
  `ssh.exe -N -D 127.0.0.1:<port> -o ExitOnForwardFailure=yes -o ServerAliveInterval=30 -o ServerAliveCountMax=3 -o BatchMode=yes -o StrictHostKeyChecking=<ask|yes> -i <key> -p <port> <user>@<host>`
- Host Key 流程：
  1. `ssh-keygen -F <host> -f <known_hosts>` 查询是否已知；
  2. 未知 → `ssh-keyscan -t ed25519,ecdsa,rsa -p <port> <host>` 取指纹，UI 展示让用户确认，确认后**追加单行**写入 `%USERPROFILE%\.ssh\known_hosts`（写前备份）；
  3. 已记录但连接报 `REMOTE HOST IDENTIFICATION HAS CHANGED`（stderr 分类）→ 阻断 + 告警，不自动删旧条目。
- 进程监管：`Child` 进程 + 仅记录本工具创建 PID；停止只终止自己创建的进程树，**绝不 `taskkill /IM ssh.exe /F`**。
- 掉线检测：stderr 解析（`Connection closed/reset`、`Broken pipe` 等）+ 本地端口监听复检。
- 重连：指数退避，默认最多 3 次；断连瞬间状态 `DISCONNECTED`、清空出口 IP 展示。
- 错误分类：`SshNotFound / HostUnreachable / AuthFailed / HostKeyChanged / HostKeyUnknown / LocalPortBusy / RemoteForwardDenied / TunnelDied / DnsFailed`。

### 3.3 `verify`（P0）
- 检测序列（全带时间戳）：① 127.0.0.1 端口监听探测；② SOCKS5 握手 + CONNECT（域名=远端 DNS）；③ HTTPS 回显出口 IP。
- 端点默认：`https://api.ipify.org?format=json`、`https://ipinfo.io/ip`（可配置更换）。
- 结果只存：域名、状态码、出口 IP、耗时。不记录正文/路径。
- 「隧道测试」与「应用路由验证」在 UI 分开展示。

### 3.4 `launchers`（P0/M2）
- 启动 `powershell.exe`（参数数组）：仅在新子进程环境设置 `HTTP_PROXY/HTTPS_PROXY/ALL_PROXY`（按 M2 实测结果选择）、`NO_PROXY=localhost,127.0.0.1,::1`，再执行 `codex.cmd`。
- **不写用户/系统环境变量**；启动前 UI 预览「将注入哪些设置」。
- 隧道非 `EGRESS_VERIFIED` 时禁止启动。
- HTTP CONNECT 桥接层（P1 按需）：仅绑 `127.0.0.1` 随机端口，CONNECT → 下游 SOCKS5；无 TLS MITM、无缓存、限并发。CLI 实测支持 SOCKS 则默认不启用。
- 桥接层区分两个计数：`connections_total`（连上来的**尝试**，含随后被拒/上游失败的）与 `connections_tunneled`（下游 SOCKS5 建连成功并已回 200 的）。**路由判定「已验证」只认后者**——拿尝试数会把「试图走网关却失败」冒充成「已验证」。

### 3.4.1 路由判定（`diagnostics::build_clients`）

三类客户端的判定依据不同，但都必须能给出「异常」，且都遵守「无法确认即不得声称已验证」：

| 客户端 | 依据 | 四态 |
|---|---|---|
| CLI | 桥接层证据（不经 Mihomo） | 有 `connections_tunneled` + 网关可达 → 已验证；只有尝试/有拒绝 → 异常；零动静 → 未验证 |
| Desktop | Mihomo `/connections` 的 `chains` / `rulePayload` 命中 `gateway_group` | 全命中 → 已验证；部分命中 → 部分；有连接但一条未命中且网关卡可用 → 异常；未命中且网关卡不可用 → 未验证（不误报路由错）；有连接但 TUN 关闭 → 无法确认 |
| IDE | 同 Desktop | 同上 |

关键约束：**网关卡自身不可达时不得报告「异常」**——那种情况「没命中网关」只是网关不可用的副作用，报异常会把「本地网关没起来」误导成「路由配错了」。

### 3.5 `mihomo`（P1/M3）
- 只读检测：Verge 版本与**实际安装路径**、mihomo 进程、mixed 端口、external-controller 可达性（secret 由用户输入且不落盘）、TUN 状态、profile 链结构。
- 可执行文件定位顺序：注册表卸载项 `DisplayIcon` → 运行中进程映像路径 → 常见安装目录（由环境变量与盘符动态推导）→ PATH；全部失败则如实报告「未定位到」，**不猜路径**。
- 集成：生成**独立规则片段**（PROCESS-NAME/PROCESS-PATH → MY-VPS，置于最终 MATCH 前）+ 备份；经 Verge 扩展配置/手动导入入口应用；一键回滚（恢复本工具备份；检测到用户同时修改只报告冲突不覆盖）。
- 规则生成前提：进程发现 + 用户确认范围；`Code.exe`/`node.exe` 等通用进程默认不整体导流。

### 3.6 状态机
```
UNCONFIGURED → READY → CONNECTING → TUNNEL_READY → EGRESS_VERIFIED
                        ↘ ERROR / DISCONNECTED
EGRESS_VERIFIED → DEGRADED（出口复检失败）→ RECONNECTING（≤3 次）
所有状态 → DISCONNECTING → READY
```
- 事件：`state_changed`、`log`（脱敏）、`verify_result`、`process_found`。
- 断线策略默认：立刻 `DISCONNECTED`、停止接受新 CLI 启动、不触碰用户其他配置。

## 4. 测试策略（按文档第 10 节矩阵）

- **单元测试（cargo test）**：配置原子写/回滚、参数构造、stderr 错误分类、SOCKS5 握手编解码、指纹格式化、规则片段生成。
- **集成测试（Docker 夹具）**：正确/错误密钥、错误端口、指纹变化、AllowTcpForwarding no、端口占用 → 隧道建立、出口验证、断开只杀自身进程。
- **手工验收（docs/acceptance-report.md）**：按 M1–M4 记录实测结果与日志摘录。
- 测试环境唯一改动：Docker 自建容器（可销毁）；不改主机 ssh/代理/路由配置。

## 5. 目录结构

```text
LostCodexGateway/
├─ docs/                     # architecture / m0 报告 / setup / troubleshooting / security / acceptance
├─ src/                      # Vue 前端（pages / components / stores / types）
├─ src-tauri/
│  ├─ src/
│  │  ├─ ssh/  verify/  launchers/  mihomo/  config/
│  │  └─ main.rs lib.rs commands.rs state.rs
│  └─ capabilities/default.json
├─ tests/
│  ├─ fixtures/ssh-server/   # Docker 测试夹具
│  └─ e2e/                   # PowerShell 集成脚本
├─ scripts/                  # 构建/测试/打包
└─ README.md  LICENSE
```

## 6. 分阶段实施（每阶段真实可运行 + 测试记录后进入下一阶段）

| 阶段 | 内容 | 完成判定 |
|---|---|---|
| M1 | 脚手架 + config + ssh + verify + GUI 基础页 + Docker 夹具测试 | 一键连接/断开；出口 IP 实测（夹具）；错误场景有测试记录 |
| M2 | launchers + CLI 代理实测 +（按需）bridge | CLI 子进程出口=服务器出口；无全局污染；断连即禁启动 |
| M3 | mihomo 检测/规则片段/备份回滚 + Desktop/IDE 进程实测 | 规则命中证据或如实「未覆盖」；回滚无损 |
| M4 | 打包 NSIS + 全部文档 + 验收报告 + 风险/未实现清单 | 安装/卸载/断网重连矩阵记录 |
