# LostCodexGateway

官方 Codex 客户端的**专用网络出口管理器**（Windows）。一键在你自己的 Ubuntu 云服务器上建立 SSH SOCKS5 隧道，把被选择的客户端网络连接经该服务器转发；不转发或改写模型 API，不接管 Codex 登录，不管理 ChatGPT OAuth。

> 产品定位：本地 Codex 的出口管理工具。服务器**仅做传输层出口**。网络出口位置不会改变你账号的实际地区或服务条款，本工具不提供任何「绕过账号限制/地区政策」的功能。

## 核心能力（已实测）

| 能力 | 状态 |
|---|---|
| SSH SOCKS5 隧道一键连接/断开（Windows 自带 OpenSSH，参数数组无 shell 拼接） | ✅ M1 实测 |
| Host Key 首次确认 + 指纹变化阻断（写前备份 known_hosts） | ✅ M1 实测 |
| 出口验证：端口监听 / SOCKS 远端 DNS / 出口 IP 回显（多端点容错，带时间戳） | ✅ M1 实测 |
| 错误分类：DNS/不可达/鉴权失败/指纹变化/端口占用/远端禁止转发/掉线 | ✅ M1 实测 |
| 只清理本工具创建的 ssh.exe 进程（绝不 taskkill /IM 影响你的其他会话） | ✅ M1 实测 |
| Codex CLI 专用启动器（子进程代理注入，不写全局环境变量） | ✅ M2 实测 |
| HTTP CONNECT → SOCKS5 桥接层（仅回环；实测 Codex 原生二进制需要它） | ✅ M2 实测 |
| Mihomo/Clash Verge 只读检测 + 规则片段生成 + 备份/回滚 | ✅ M3 实测 |
| 诊断导出（脱敏） | ✅ |
| **网络诊断模块**：隧道/SOCKS/出口 IP 对照/服务器只读连通性/Codex 进程路由/DNS/IPv6/延迟，一键总览 | ✅ 新需求模块 |
| **系统托盘驻留**：关闭窗口即隐藏到托盘（隧道保持运行）；托盘菜单「打开主界面 / 退出」，退出前自动干净断开隧道 | ✅ 实测 |
| **WSL2 专项探测**：枚举发行版 + 逐目标真实建连探测 + 生成一次性代理注入命令（只影响该 shell 会话，不写 `~/.bashrc` / `/etc/environment`） | ✅ wsl 模块（7 项单元测试） |
| **开机启动**：当前用户注册表 Run 键（无需提权）；随登录驻留托盘，**不自动连接隧道**（避免静默建立出口） | ✅ autostart 模块（5 项单元测试） |
| **单实例守卫**：重复启动（含自启与手动双击撞车）不会起第二个隧道进程、不抢本地端口，而是唤出已在运行的窗口后自身退出 | ✅ single_instance 模块（2 项单元测试） |
| **桥接层安全加固**：对端回环校验 + 目标黑名单（回环/私有网段/自身端口，防循环代理）+ 并发上限回 503 + 双向空闲超时 + 结构化错误码 `BRIDGE_*` | ✅ `bridge_e2e` 集成测试 3 项（免 Docker）+ 单元测试 8 项 |
| 真实运行截图 | ✅ [docs/screenshots/network-diagnostics.png](docs/screenshots/network-diagnostics.png) |

**实测结论（不虚构）**：Codex CLI 0.154.0 是 Rust 原生二进制，读取 `HTTP_PROXY/HTTPS_PROXY` 但不支持 `socks5://` 直供；因此本工具提供经审计的本地 HTTP CONNECT 桥接层（127.0.0.1 随机端口，无 TLS MITM、无缓存）。

## 快速开始

1. 安装 `LostCodexGateway_0.2.0_x64-setup.exe`（NSIS 安装包，普通用户权限即可）。
2. 打开应用 →「服务器」页填写：主机地址、SSH 端口、用户名、私钥路径（**只存路径**）、本地 SOCKS 端口（默认 17801，冲突会提示换端口）。
3. 「查询服务器指纹」→ 与服务器管理员核对 →「我已核对，确认写入」。
4. 回到「首页」点「连接」→ 状态变为「出口已验证」，页面显示隧道出口 IP 与验证步骤。
5. 「应用」页 →「从网关启动 Codex CLI」→ 弹出独立终端，代理仅注入该终端（`NO_PROXY=localhost,127.0.0.1,::1` 保证 OAuth 回调直连）。
6. 完成后点「断开」→ 仅停止本工具创建的 SSH 进程，你的其他 SSH 会话、系统代理、Mihomo 配置均不受影响。
7. 点窗口关闭按钮 = **隐藏到系统托盘**（隧道保持运行）；托盘左键重新打开主界面，右键菜单「退出」才会真正退出（退出前自动断开隧道）。

## 支持的客户端

- **Codex CLI（Native Windows）**：✅ 已实测（0.154.0，M2 决定性证据）。
- **Codex Desktop / IDE**：⚠ 需 Mihomo 受控集成（M3）。当前实测环境 TUN 未开启，应用如实显示「尚未覆盖」；开启 TUN 由 Mihomo/Clash Verge 官方组件处理权限。
- **WSL2 内的 Codex**：⚠ 提供只读探测与一次性注入命令。判据是「WSL 内真实建连成功」：mirrored 网络模式可直接用 `127.0.0.1`；**NAT 模式（WSL2 默认）下 Windows 侧 SOCKS 只绑回环，WSL 连不上**，页面如实报不可达，不代做需要提权的端口转发。

## 文档

- [架构设计](docs/architecture.md)（含环境实测依据）
- [环境调研报告](docs/m0-environment-report.md)
- [Windows 配置指南](docs/setup-windows.md)（含 Mihomo 规则导入/回滚步骤）
- [故障排除](docs/troubleshooting.md)
- [安全与隐私](docs/security.md)
- [验收与测试记录](docs/acceptance-report.md)

## 开发

```bash
npm install                # 前端依赖
cargo test                 # Rust 单元测试（21 项）
# 集成测试需要 Docker 夹具（本机无 sshd 时）：
powershell -File tests/fixtures/ssh-server/setup.ps1
cargo test --test tunnel_e2e -- --ignored --test-threads=1
cargo test --test bridge_e2e -- --ignored
npm run tauri dev          # 开发模式
npm run tauri build        # 产出 NSIS 安装包（src-tauri/target/release/bundle/nsis/）
```

测试夹具只使用自建 Docker 容器（可 `-Teardown`/`-Clean` 清理），不修改宿主机 ssh/代理/路由配置。

## 许可证

MIT（见 LICENSE）。第三方依赖清单见 `THIRD_PARTY.md`。
