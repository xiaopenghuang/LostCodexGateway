# LostCodexGateway — M0 环境调研报告

> 生成时间：2026-09-20｜生成方式：`scripts/m0-detect.ps1`（只读检测）+ 手动只读探测

> 本报告未修改任何系统/服务器配置。
>
> **关于路径与地址**：本报告公开版本已将本机真实盘符路径替换为示意路径
> （如 `D:\Tools\...`），公网 IP 替换为 [RFC 5737](https://www.rfc-editor.org/rfc/rfc5737)
> 文档保留段。**结论、判定逻辑与技术依据未作任何改动**——被保留下来的是
> 「测到了什么、因此如何决策」，具体盘符与数值不构成设计依据。
> 原始机器盘点数据（含真实用户名/盘符/已装软件/进程）保留在本地，不入库。

## 1. 检测摘要

| 项目 | 实测结果 | 与开发文档假设的关系 |
|---|---|---|
| 操作系统 | Windows 10 专业版 10.0.19045 x64 | 符合假设（Win10/11 x64） |
| Windows OpenSSH | `C:\Windows\System32\OpenSSH\ssh.exe` OpenSSH_for_Windows **9.5p1** (LibreSSL 3.8.2)；`ssh-keygen.exe` 同目录存在 | 可用；注意 PATH 第一位是 Git 自带 `D:\Tools\Git\usr\bin\ssh.exe` (10.5p1)，应用需**显式优先使用 System32 版本**，否则行为不一致 |
| 本机 sshd 服务 | **未安装**（服务不存在） | 与文档假设（本机可起 sshd 测试）不一致 → 集成测试改用 **Docker sshd 容器**夹具 |
| Docker | 引擎 29.2.1 可用（WSL2 后端，`docker-desktop` 发行版运行中） | 测试夹具方案成立 |
| Clash Verge Rev | **2.5.2**，安装在 `D:\Tools\Clash Verge\`，`clash-verge.exe` + `verge-mihomo.exe` 均在运行 | 版本比文档写作时新，需按 2.x 能力适配（只读探测 + 增量写入） |
| Mihomo 运行时 | `verge-mihomo.exe` 监听 `127.0.0.1:2080`（mixed 端口）；`external-controller = 127.0.0.1:9097`（带 secret，**值不记录**）；`allow-lan=false`；`mode=rule`；**TUN 关闭**（`tun.enable=false`，TAP-Windows 网卡状态 Disconnected） | 文档假设「已有 Clash TUN」不成立 → 当前 TUN 未开启，M3 先做**非 TUN 受控规则**，TUN 场景按「未覆盖」如实提示 |
| Verge 配置链 | profiles 目录含订阅/规则/脚本链：Merge.yaml、Script.js、订阅 profile、rules profile；增强配置 `clash-verge.yaml`（46KB）；目录里存在用户自己做的 `*.bak_lostproxy_*` 备份 | 存在多 profile 与脚本增强 → 只做「生成片段 + 备份 + 用户确认导入」，**绝不直接改写订阅文件** |
| 系统代理 | WinINET 代理**关闭**（ProxyEnable=0，残留 ProxyServer=127.0.0.1:2080 字段） | 本工具不得开启系统代理 |
| 直连出口 | **失败**（`curl --noproxy "*"` 到 ipify 被 RESET） | 与假设「可直接连公网」不一致 → 出口对照测试基准改为「用户当前代理 2080 出口」与「隧道出口」的差异对比；SSH 直连能力需单独验证 |
| 经 127.0.0.1:2080 出口 | 正常，出口 IP `198.51.100.88` | 用户现有代理工作正常 |
| 其他代理进程 | `FlClashHelperService.exe`（PID 6048，服务态）存在但**无 FlClash 数据目录**，疑似残留服务 | 不影响；不触碰 |
| Codex CLI | **0.154.0**，npm 全局安装：`D:\Tools\nodejs\node_global\codex.cmd`（node v22.17.1 @ `D:\Tools\nodejs`） | Native Windows CLI，非 WSL2 |
| Codex Desktop | **正在运行**：`codex.exe`(45584)、`codex-windows-sandbox-service.exe`(27332, 服务)、`codex-computer-use-swift.exe`(46564)、`codex-code-mode-host.exe`(28792)、`lost-codex-theme.exe`(25428)；数据目录 `%APPDATA%\codex`(web)、`%LOCALAPPDATA%\codex`(Logs) | Native Windows Desktop 进程树已获取；**网络路径是否经 WSL2 尚未证实**，M3 按「进程发现 + 连接关联」实测 |
| VS Code | 已安装（`D:\Tools\Microsoft VS Code\bin\code.cmd`）；`.vscode/extensions` 中**未见 codex 插件目录** | IDE 场景待装插件后实测，v1 标记「未验证/未覆盖」 |
| WSL2 | 启用，`Ubuntu-24.04` Running（WSL2 内核），另有 `docker-desktop` | WSL2 内未发现 codex；v1 对 WSL2 内 Codex 标记「不支持/需单独诊断」 |
| 开发工具链 | Rust 1.98.1 + cargo（已装用户级）；MSVC 14.50（VS2026 Build Tools @ D:\Tools\BuildTools，链接验证通过）；node v22.17.1 + npm（@ D:\Tools\nodejs）；无 pnpm；WebView2 Runtime 153.0.4234.32 | 满足 Tauri 2 构建要求；NSIS 打包用 Tauri 自带 bundler |
| PowerShell | 5.1（Windows PowerShell），无 pwsh 7 | 启动器/脚本按 5.1 兼容编写 |
| GitHub/SSH 端口连通 | 直连 github.com HTTPS 正常（HTTP 200） | 说明本机到公网 TCP 基本可达，SSH 到 VPS 可行性待用户服务器信息 |

## 2. 与文档假设的偏差及适配决策

| # | 假设 | 实测 | 适配决策（不虚构兼容性） |
|---|---|---|---|
| D1 | 使用系统 ssh.exe | PATH 首位是 Git 的 ssh 10.5p1；System32 有 9.5p1 | 应用内显式定位 `%SystemRoot%\System32\OpenSSH\ssh.exe` 优先，并在 Server 页面显示实际使用路径与版本；允许用户改路径 |
| D2 | 本机 sshd 可用于测试 | 未安装 | 集成测试用 Docker `linuxserver/openssh-server` 或自建镜像夹具；`sshd` 服务检测结果在诊断页如实显示 |
| D3 | 已有 Clash TUN 开启 | TUN 关闭；TAP 网卡 Disconnected | M3 交付「检测 + 生成片段 + 备份 + 人工/受控导入 + 回滚」，TUN 未开启时如实提示「Desktop/IDE 尚未覆盖」 |
| D4 | 直连可作出口对照 | 直连被 RESET | 对照基准=当前代理出口（2080）；「未走网关的应用不改变原出口」用「对照出口不变」验证 |
| D5 | Desktop 进程结构 | 已抓到 5 个进程，但父子关系与网络路径待测 | M3 用「进程发现 + Mihomo 连接关联」实测后再生成规则，不按进程名猜测 |
| D6 | CLI 代理兼容性 | 0.154.0 native，未实测代理变量 | M2 实测：先 SOCKS 直测，再决定是否启用 HTTP CONNECT 桥接层（回环绑定） |

## 3. 功能可实现矩阵

| 功能 | 状态 | 依据 |
|---|---|---|
| SSH SOCKS5 隧道启停/监控/有限重连 | ✅ 可实现 | System32 ssh 9.5p1 支持 `-D/-N/ExitOnForwardFailure/ServerAlive`；Docker 夹具可测 |
| Host Key 首次确认/变化阻断 | ✅ 可实现 | 用系统 known_hosts + `ssh-keygen -F` 查询 + `ssh-keyscan` 取指纹展示 |
| 出口 IP 验证（远端 DNS） | ✅ 可实现 | SOCKS5 客户端 + 多验证端点（ipify/ipinfo/ip.sb 等可更换） |
| CLI 专用启动器（进程级代理） | ✅ 可实现 | PowerShell 子进程注入环境变量；`D:\Tools\nodejs\node_global\codex.cmd` 可定位 |
| HTTP CONNECT 桥接层 | ⏸ 按需 | 先实测 CLI 对 SOCKS 的支持，不支持才启用 |
| Mihomo 受控集成 | ⚠ 部分 | Verge 2.5.2 在运行、控制器 9097 可达（需 secret）；**仅只读检测 + 生成片段 + 备份 + 用户确认**，订阅更新后保留需验证 Verge 2.5.2 的扩展配置行为 |
| Desktop/IDE 流量覆盖 | ⚠ 需实测 | 进程树已知；网络路径与连接归属待 M3 实测 |
| WSL2 内 Codex | ❌ v1 不支持 | WSL2 发行版运行中；检测到则提示「不支持」并提供诊断 |
| 开机启动/托盘 | ✅ 可实现（P2） | — |
| 普通用户无管理员运行 | ✅ 可实现 | P0 全部无提权 |

## 4. 已识别的风险（M0 版）

1. **订阅/规则链复杂**：用户 Verge 有 merge+script+rules 多段 profile，且存在手工备份痕迹。任何自动写入都可能破坏链 → 采用「生成独立片段 + 备份 + 用户手动导入（或在用户确认后仅通过 Verge 扩展配置入口）」。
2. **直连出口不可用**：SSH 连接本身必须直连 VPS（不能经自身隧道，防环路）；若 VPS 直连也不可达，需提示用户检查路由（本机直连 GitHub 正常，风险中等）。
3. **Codex Desktop 网络路径未证实**：可能是 WSL2 或 sandbox 网络命名空间；M3 前不承诺覆盖。
4. **外部控制器 secret**：M3 只读探测控制器需 secret；检测到存在但未配置时降级为「仅生成配置片段」模式。
5. **本机无 sshd**：测试夹具依赖 Docker 可用性；Docker 不可用时手工测试需用户真实服务器。
6. **PATH 中 Git ssh 干扰**：任何脚本/文档示例都显式使用 System32 全路径。
