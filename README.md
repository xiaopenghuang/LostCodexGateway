<div align="center">

<img src="app-icon.png" alt="LostCodexGateway" width="128" height="128" />

# LostCodexGateway

**在你自己的一台 Linux 云服务器上，为 Windows 上的 Codex 客户端建立一条可验证的网络出口。**

一键拉起 SSH SOCKS5 隧道，让被选中的客户端流量从你的服务器出去 —— 不转发、不解密、不改写模型 API。

[![Release](https://img.shields.io/badge/release-v0.3.0-2ea44f?style=flat-square)](../../releases)
[![Platform](https://img.shields.io/badge/platform-Windows%2010%2F11-0078d4?style=flat-square)](#环境要求)
[![License](https://img.shields.io/badge/license-MIT-blue?style=flat-square)](LICENSE)
[![Tauri](https://img.shields.io/badge/Tauri-2.x-24c8db?style=flat-square)](https://tauri.app)
[![Vue](https://img.shields.io/badge/Vue-3.5-42b883?style=flat-square)](https://vuejs.org)

[简体中文](README.md) · [English](README.en.md)

</div>

---

## 目录

- [这个工具解决什么问题](#这个工具解决什么问题)
- [核心特性](#核心特性)
- [关于「已验证」的诚实边界](#关于已验证的诚实边界)
- [架构](#架构)
- [环境要求](#环境要求)
- [快速开始](#快速开始)
- [从源码构建](#从源码构建)
- [项目结构](#项目结构)
- [安全设计](#安全设计)
- [已知限制](#已知限制)
- [文档索引](#文档索引)
- [参与贡献](#参与贡献)
- [许可证](#许可证)

---

## 这个工具解决什么问题

你在 Windows 上用 Codex CLI / Codex Desktop，希望它的**网络出口**走你自己的一台 Linux 服务器，而不是本机直连。常见做法是手动开一个 `ssh -D`，再想办法让 Codex 用上它 —— 而这件事在 Windows 上有几个具体的坑：

- Codex CLI 是 Rust 原生二进制，**读 `HTTP_PROXY` / `HTTPS_PROXY`，但不接受 `socks5://` 直供**。所以只有 SOCKS5 端口是不够的，需要一个 HTTP CONNECT → SOCKS5 的桥接。
- 开了代理之后，如果连 `localhost` 也走代理，**ChatGPT OAuth 回调会失败**（本地监听端口收不到回调）。
- 隧道到底有没有生效？「进程在跑」和「流量真的出去了」是两件事，需要**可验证的证据**，而不是一个绿色指示灯。
- 隧道进程的清理要克制：不能 `taskkill /IM ssh.exe`，那会杀掉你自己其他的 SSH 会话。

LostCodexGateway 把这些做成一个托盘常驻的小工具：填一次服务器信息，之后一键连接，并且**明确告诉你当前出口处于哪种状态**。

> **定位说明**：本工具只管理**传输层出口**。改变出口 IP 不会改变你账号的实际地区或服务条款适用范围，本工具不提供、也不声称提供任何「绕过账号限制或地区政策」的能力。

---

## 核心特性

| 特性 | 说明 |
|---|---|
| **SSH SOCKS5 隧道** | 调用 Windows 自带 OpenSSH（`ssh.exe -D`），参数以数组传递，无 shell 字符串拼接，无注入面 |
| **多服务器切换** | 保存任意多台服务器，一键切换当前出口。切换为硬切（断开→重连），失败不回滚但可一键切回上一个；本地端口全局固定，切换对 Codex CLI 透明 |
| **延迟探测** | 对全部服务器并发做 TCP 探测，如实标注测的是「到 SSH 端口的往返」而非出口延迟 |
| **Host Key 首次确认** | 查询服务器指纹供人工核对，确认后写入；**指纹变化则阻断连接**。写入前备份 `known_hosts` |
| **出口验证** | 端口监听 + SOCKS 远端 DNS 解析 + 出口 IP 回显（多端点容错、带时间戳），三者独立判定 |
| **HTTP CONNECT → SOCKS5 桥接** | 仅绑定 `127.0.0.1`，无 TLS 中间人、无缓存、无根证书安装；对目标做回环/私网黑名单，防代理循环 |
| **进程清理克制** | 只终止本工具记录的 `ssh.exe` 子进程，绝不影响你的其他 SSH 会话 |
| **Codex CLI 启动器** | 代理仅注入该子进程的终端环境，**不写全局环境变量**；`NO_PROXY=localhost,127.0.0.1,::1` 保证 OAuth 回调直连 |
| **网络诊断面板** | 隧道 / SOCKS / 出口 IP 对照 / 服务器只读连通性 / Codex 进程路由 / DNS / IPv6 / 延迟，一屏总览 |
| **错误分类** | DNS 失败、主机不可达、鉴权失败、指纹变化、端口占用、远端禁止转发、掉线 —— 分开报告，不糊成一个「连接失败」 |
| **Mihomo / Clash Verge 只读集成** | 只**检测**、生成规则片段、备份与回滚，不修改你的 Mihomo 配置（且 Clash 不是硬依赖，见下） |
| **WSL2 探测** | 枚举发行版 + 逐目标真实建连探测 + 生成一次性代理注入命令（只作用于该 shell，不写 `~/.bashrc` / `/etc/environment`） |
| **托盘常驻** | 关闭窗口 = 隐藏到托盘，隧道保持运行；托盘菜单「退出」才会真正退出，退出前自动干净断开 |
| **开机启动** | 当前用户注册表 Run 键，无需提权；随登录驻留托盘，**不自动连接隧道**（避免静默建立出口） |
| **单实例守卫** | 重复启动不会起第二个隧道、不抢端口，而是唤出已有窗口后自身退出 |
| **诊断导出** | 一键导出脱敏后的诊断包，便于排查 |

---

## 关于「已验证」的诚实边界

「隧道开着」不等于「流量真的走了隧道」。本工具把路由状态拆成 **5 种**，每种都有明确的判据：

| 状态 | 含义 | 判定依据 |
|---|---|---|
| **已验证** `Verified` | 流量确实经网关出去 | 桥接层至少有一条连接**下游 SOCKS5 建连成功且已回 200**，且网关本身可达 |
| **部分覆盖** `Partial` | 部分客户端走了网关 | 网关可用，且命中网关的连接与其他连接并存 |
| **异常** `Anomaly` | 试图走网关但没出去 | 有连接**尝试**或存在被拒记录，但没有任何一条真正建立隧道 |
| **未验证** `Unverified` | 尚无证据 | 隧道未运行，或网关不可达（此时「没命中」只是网关挂了的副作用，不算异常） |
| **无法确认** `Unconfirmable` | 环境不支持判定 | 例如缺少必要权限或探测手段 |

> 关键实现细节：桥接层区分 **连接尝试数**（TCP 连上即计数）与 **成功隧道化数**（下游 SOCKS5 建连成功 + 已回 `200`）。只有后者才被用作「已验证」的证据 —— 否则「CLI 试图走网关但失败了」会被误报成成功。

---

## 架构

```
┌─────────────────────────────────────────────────────────────────┐
│  Windows 本机                                                    │
│                                                                 │
│   ┌───────────────┐   HTTP_PROXY=http://127.0.0.1:<port>        │
│   │  Codex CLI    │ ─────────────────────────────┐              │
│   │ (原生二进制)   │                              │              │
│   └───────────────┘                              ▼              │
│   ┌───────────────┐                    ┌──────────────────────┐  │
│   │ Codex Desktop │                    │  桥接层 (Rust)        │  │
│   │   / IDE       │ ──────────────────▶│  HTTP CONNECT        │  │
│   └───────────────┘    (可选，经 Mihomo │  → SOCKS5            │  │
│                         受控集成)       │  仅绑 127.0.0.1       │  │
│                                        └──────────┬───────────┘  │
│                                                   │ SOCKS5       │
│                                        ┌──────────▼───────────┐  │
│                                        │ 127.0.0.1:17801      │  │
│                                        │ (ssh -D 本地监听)     │  │
│                                        └──────────┬───────────┘  │
└───────────────────────────────────────────────────┼─────────────┘
                                                    │ SSH (加密)
                                                    ▼
                                        ┌──────────────────────┐
                                        │  你的 Linux 服务器    │
                                        │  sshd → 出网          │
                                        └──────────────────────┘
```

**为什么需要中间那层桥接？** 因为 Codex CLI 的原生二进制只实现了 HTTP 代理的 CONNECT 语义，不理解 `socks5://`。桥接层把这一段补齐。它只监听回环地址、不解密 HTTPS、不装根证书 —— 它只是一个协议转换器，不是中间人。

**关于 Clash / Mihomo：不是硬依赖。** 核心链路是 `Codex → 回环桥接层 → ssh -D → 你的服务器`，全程不需要 Clash。Mihomo 集成只服务于一种场景：如果你还想让 **Codex Desktop / IDE** 这些不走 `HTTP_PROXY` 环境变量的客户端也走隧道，可以通过 Clash 的规则把它们的流量导到网关组 —— 这部分本工具只做**只读检测 + 规则片段生成 + 备份回滚**，改不改由你决定。

---

## 环境要求

| 项目 | 要求 |
|---|---|
| 操作系统 | Windows 10 1809+ / Windows 11 |
| 运行时 | [OpenSSH Client](https://learn.microsoft.com/windows-server/administration/openssh/openssh_install_firstuse)（Windows 可选功能，通常已预装） |
| 服务端 | 任意 Linux 发行版，运行 `sshd`，允许端口转发（`AllowTcpForwarding yes`），并有公网出网能力 |
| 鉴权 | 私钥（Ed25519 / RSA）。本工具**只保存私钥路径**，不读取、不复制私钥内容 |
| 可选 | Mihomo / Clash Verge（仅当你要覆盖 Desktop / IDE 客户端时） |

---

## 快速开始

1. **安装**：下载 `LostCodexGateway_0.3.0_x64-setup.exe` 并运行。普通用户权限即可，无需管理员。
2. **填服务器信息**：打开应用 →「服务器」页 →「新增服务器」→ 填写名称、主机地址、SSH 端口、用户名、私钥路径。
   可以保存多台，随时用「切换」按钮换当前出口（切换是硬切：先断开旧隧道再重连，进行中的请求会中断）。
   本地 SOCKS 端口（默认 `17801`）与桥接端口（默认 `17800`）是**全局设置**，在「设置」页修改。
3. **核对指纹**：点「查询服务器指纹」→ **与你的服务器管理员核对**（或与你已知的指纹比对）→ 确认「我已核对，确认写入」。
4. **连接**：「首页」点「连接」→ 状态变为**已验证**，页面显示出口 IP 与每一步的验证结果。
5. **启动 Codex**：「应用」页 →「从网关启动 Codex CLI」→ 弹出独立终端窗口，代理只注入该终端。
6. **关闭**：点「断开」——只停止本工具创建的 SSH 进程。你的其他 SSH 会话、系统代理、Mihomo 配置均不受影响。
7. **平时**：点窗口关闭按钮 = **隐藏到托盘**（隧道保持运行）。托盘左键打开主界面，右键「退出」才真正退出（退出前自动断开隧道）。

---

## 从源码构建

前置：Node.js 20+、Rust 1.77+（stable）、Windows 上需安装 MSVC 构建工具。

```bash
# 1. 前端依赖
npm install

# 2. Rust 单元测试
cargo test                     # 在 src-tauri/ 下执行

# 3. 开发模式（热重载）
npm run tauri dev

# 4. 构建发行版（产出 NSIS 安装包）
npm run tauri build
# 产物：src-tauri/target/release/bundle/nsis/LostCodexGateway_<version>_x64-setup.exe
```

> ⚠️ `cargo build --release` 只会编译出可执行文件，**不会生成安装包**。要出安装包必须走 `npm run tauri build`。

集成测试需要 Docker 夹具（本机没有可用的 `sshd` 时）：

```powershell
powershell -File tests/fixtures/ssh-server/setup.ps1
cargo test --test tunnel_e2e -- --ignored --test-threads=1
cargo test --test bridge_e2e -- --ignored
```

测试夹具只使用自建的 Docker 容器（可用 `-Teardown` / `-Clean` 清理），**不修改宿主机的 ssh / 代理 / 路由配置**。

---

## 项目结构

```
LostCodexGateway/
├── src/                     # 前端（Vue 3 + TypeScript）
│   ├── pages/               # 各功能页：首页 / 服务器 / 应用 / 网络诊断 / WSL / 设置
│   ├── components/          # 通用组件（设计系统）
│   ├── stores/              # 状态管理（只读快照，权威状态在 Rust 侧）
│   └── dev/fixtures.ts      # 假数据夹具，供无后端时的视觉检查
├── src-tauri/               # Rust 后端
│   └── src/
│       ├── ssh.rs           # SSH 隧道进程管理、Host Key 校验
│       ├── bridge.rs        # HTTP CONNECT → SOCKS5 桥接层
│       ├── diagnostics.rs   # 网络诊断与路由状态判定
│       ├── mihomo.rs        # Clash / Mihomo 只读检测与规则生成
│       ├── wsl.rs           # WSL2 探测与一次性注入命令
│       ├── autostart.rs     # 开机启动（注册表 Run 键）
│       └── single_instance.rs
├── docs/                    # 设计文档、审计与验收记录
├── scripts/                 # 开发辅助脚本（隐私扫描、路径脱敏等）
└── tests/                   # 集成测试与 E2E 夹具
```

**设计原则**：Rust 侧是唯一权威状态源，前端只渲染快照。所有可能失败的 I/O 都在 Rust 侧有显式错误分类，前端不做推断。

---

## 安全设计

- **不接触凭据**：不读取、不解密、不缓存模型 API 流量；不接管 Codex 登录流程，不管理 ChatGPT OAuth。
- **不修改系统配置**：不改系统代理、不改路由表、不改 Mihomo 配置（Mihomo 部分只读 + 生成片段）。
- **进程清理有边界**：只终止本工具自己创建的 `ssh.exe` 子进程。
- **桥接层最小权限**：只绑回环、无 TLS 中间人、无根证书、目标黑名单防代理循环、并发上限、双向空闲超时。
- **私钥只存路径**：配置里记录的是路径字符串，私钥内容不出现在应用数据、日志或诊断导出中。
- **诊断导出已脱敏**：导出前对 IP、主机名、用户名等做替换。

详细说明与审计记录：[docs/security.md](docs/security.md)、[docs/privacy-audit.md](docs/privacy-audit.md)。

---

## 已知限制

- **WSL2 NAT 模式**：WSL2 默认 NAT 网络下，Windows 侧的 SOCKS 只绑回环，WSL 内连不上。本工具**如实报告不可达**，不代做需要提权的端口转发。mirrored 网络模式下可直接使用 `127.0.0.1`。
- **Codex Desktop / IDE**：不走 `HTTP_PROXY` 环境变量，需通过 Mihomo 受控集成覆盖。TUN 模式的开启与权限由 Mihomo / Clash Verge 官方组件处理，本工具不代办。
- **仅 Windows**：SSH 进程管理与注册表自启均为 Windows 专有实现。
- **单实机验证**：当前验收记录来自有限的环境组合，详见 [验收报告](docs/acceptance-report.md)。

更完整的技术债与未实现项：[docs/risks-and-unimplemented.md](docs/risks-and-unimplemented.md)。

---

## 文档索引

| 文档 | 内容 |
|---|---|
| [架构设计](docs/architecture.md) | 模块划分、数据流、环境实测依据 |
| [Windows 配置指南](docs/setup-windows.md) | 服务器侧与客户端侧配置，含 Mihomo 规则导入 / 回滚步骤 |
| [故障排除](docs/troubleshooting.md) | 常见错误分类与处置 |
| [安全与隐私](docs/security.md) | 威胁模型与安全边界 |
| [隐私审查记录](docs/privacy-audit.md) | 公开发布前的敏感信息扫描与处理记录 |
| [验收与测试记录](docs/acceptance-report.md) | 实测环境、证据与结论 |
| [风险与未实现项](docs/risks-and-unimplemented.md) | 已知技术债 |
| [环境调研报告](docs/m0-environment-report.md) | 立项阶段的环境摸底 |
| [变更日志](CHANGELOG.md) | 版本历史 |
| [第三方依赖](THIRD_PARTY.md) | 依赖清单与许可证 |

---

## 参与贡献

欢迎 Issue 与 PR。提交前请确保：

1. `cargo test` 全部通过（沙箱环境下 `single_instance` 的个别用例可能因 `CreateMutexW` 受限而失败，需实机验证）。
2. `cargo fmt` 与 `cargo clippy -- -D warnings` 无告警。
3. 新增涉及路径 / 网络的代码时，请运行 `python scripts/privacy-scan.py` 确认未引入本机环境信息。

---

## 许可证

[MIT](LICENSE) © 2026 LostCodexGateway

第三方依赖清单与许可证见 [THIRD_PARTY.md](THIRD_PARTY.md)。

---

<div align="center">

**本项目与 OpenAI 无关联**，未获 OpenAI 的赞助、认可或授权。  
「Codex」「ChatGPT」「OpenAI」为其各自所有者的商标，此处仅用于说明本工具的互操作对象。

</div>
