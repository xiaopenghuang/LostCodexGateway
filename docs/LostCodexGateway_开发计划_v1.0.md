# LostCodexGateway 开发计划与交付规范

> 版本：v1.0｜日期：2026-09-20｜状态：交给开发智能体执行的工程规格（不是已实现功能）  
> 平台：Windows 10/11 x64 优先；远端 Ubuntu 22.04/24.04；兼容现有 Clash Verge Rev / Mihomo。  
> 产品定位：**本地官方 Codex 客户端的专用网络出口管理器**。把被选择的客户端网络连接经用户自有 SSH 云服务器转发；不转发或改写模型 API，不接管 Codex 登录，不管理 ChatGPT OAuth。

## 1. 背景与真正需求

- 当前用户：Windows 本地使用官方 Codex Desktop、Codex CLI、VS Code Codex 插件；电脑已有 Clash Verge / Mihomo；另有正常运行 Docker、Nginx、CPA、SSH 的 Ubuntu 云服务器。
- 用户不想每次手输 `ssh -D`、改 Mihomo YAML、对照进程；希望 GUI 一键连接、看到出口 IP 和「哪些应用确实走了服务器」。
- Codex 应留在本地执行，读取本地项目，OAuth 凭据由官方客户端自行保管；云服务器**仅做传输层出口**。
- 用户只要求 Codex 相关网络尽可能走云服务器；浏览器登录可作为**可选**功能，不允许默认把整台 Windows 的网络改为云服务器出口。
- 本产品不是个人订阅共享池，不支持多用户共享 ChatGPT 账号，也不提供「绕过账号限制/地区政策」的功能或效果承诺。网络出口位置不会改变用户实际所在地区或服务条款。

### 1.1 必须达到

1. SSH 隧道可配置、可启停、可自动检测及有限重连；绑定 `127.0.0.1` 随机/指定本地端口（建议默认 17801，冲突则提示换端口）。
2. 独立验证隧道和出口 IP；**只有验证过才显示「出口已验证」**。
3. Codex CLI：提供由本工具启动的「专用终端/启动器」，对该子进程设置代理环境变量，不修改系统全局环境变量。
4. Codex Desktop / IDE：提供与现有 Mihomo 的**非破坏性**配置指导/受控集成，实际进程发现与路由检测；不能因简单命中进程名就声称全部请求已代理。
5. 安全停用与还原；敏感信息不写入普通日志；不改变原有服务器防火墙、iptables、Docker、Nginx、CPA、SSH 配置。

### 1.2 明确不做

- 不内置 CLIProxyAPI，不做代理模型名称映射、API 协议转译、OAuth Token 抓取/刷新或自动化多账号轮询。
- 不开发 Windows 网络驱动、修改系统路由表、自动全局接管 VPN；不承诺 Windows、WSL2、扩展宿主进程的全部流量 100% 同一路由。
- 不修改其他应用的代理配置；不允许在 SSH 断连时悄悄退回直连而 UI 继续显示「已保护」。
- v1 不提供团队账号共享、云服务器 Web 后台或云端凭据同步。

## 2. 推荐架构与技术栈

```text
┌──────────────── Windows ────────────────┐
│ LostCodexGateway (Tauri 2 + Vue 3/TS)   │
│   ├─ SSH 隧道管理器（启动系统 ssh.exe）     │
│   ├─ 状态/出口检测、进程发现、诊断日志       │
│   ├─ CLI 专用启动器（代理仅注入子进程）      │
│   └─ Mihomo 适配器（可选，仅受控增量配置）   │
│           │                           │
│    127.0.0.1:17801 SOCKS5             │
│           │                           │
└───────────┼───────────────────────────┘
            │ SSH 加密 TCP 转发
            ▼
       自有 Ubuntu VPS (既有 sshd)
            │ 服务器建立对目标站的 TCP 连接
            ▼
        互联网 / 目标服务
```

**技术栈建议**：Tauri 2 + Vue 3 + TypeScript + Rust（连接状态机、系统操作、进程管理）；优先调用 Windows 系统自带 `ssh.exe`，不自行实现 SSH 协议。使用 Pinia（可选）存 UI 状态。Rust 命令白名单、无 shell 拼接；配置/日志放用户数据目录。依赖尽量少、GUI 不复杂。

**关键选择**：

- `ssh.exe -N -D 127.0.0.1:<port> -o ExitOnForwardFailure=yes ...` 创建本机 SOCKS5。OpenSSH 的 `-D` 是动态 TCP 端口转发，不是 VPN，不天然捕获任意程序流量，也不直接支持 UDP。
- 对 CLI 首选「用户显式从专用终端启动 + 进程级代理」；如 CLI/某版本只接受 HTTP(S) 代理，则增加一个**仅绑定回环地址**、支持 HTTP CONNECT 的本地轻量桥接层，向上提供 HTTP 代理、向下通过 SOCKS5 隧道，且不解密 HTTPS 内容。先测试原生能力，确有必要才做桥接层。
- Desktop/IDE 通过用户现有 Mihomo 的 TUN + 进程/规则匹配，是可选高级模式；TUN 捕获的并非只有 Codex，因此必须识别对其他应用的潜在影响。
- 若检测到客户端经 WSL2 发请求，Windows 进程规则或 `HTTP_PROXY` **不保证覆盖 WSL2 侧**，须单独检测/提示，v1 可标记为不支持并提供诊断，不可假装生效。

## 3. 仓库与目录结构

```text
LostCodexGateway/
├─ README.md
├─ LICENSE
├─ docs/
│  ├─ architecture.md
│  ├─ setup-windows.md
│  ├─ troubleshooting.md
│  ├─ security.md
│  └─ acceptance-report.md
├─ src/                      # Vue 界面
│  ├─ pages/                 # Dashboard, Server, Apps, Diagnostics, Settings
│  ├─ components/
│  ├─ stores/
│  └─ types/
├─ src-tauri/
│  ├─ src/
│  │  ├─ ssh/                # 启停、host key、进程监管、重连
│  │  ├─ diagnostics/        # 出口 IP、连通性、进程发现
│  │  ├─ launchers/          # CLI 专用启动器
│  │  ├─ mihomo/             # 能力探测、受控路由集成
│  │  ├─ config/             # 配置迁移、原子读写、备份
│  │  └─ commands.rs        # 限定 Tauri 命令接口
│  └─ capabilities/
├─ tests/
│  ├─ fixtures/
│  └─ e2e/
└─ scripts/                 # 本地构建、测试、打包
```

**应用数据目录**：按 Windows 标准用户数据目录保存 `LostCodexGateway` 配置。**SSH 私钥只保留本地路径，不复制/导出；禁止在配置、错误日志、屏幕截图中记录私钥内容。** 不要求创建专用服务器服务。项目名、仓库名、配置目录统一为 `LostCodexGateway`。

## 4. 页面及交互设计（轻量、开箱即用）

- **首页 Dashboard**：连接状态（未配置 / 连接中 / 隧道已通 / 出口已验证 / 降级 / 断开 / 错误）；目标服务器名称（敏感 IP 可遮罩）；本地 SOCKS 端口；当前出口 IP；「连接 / 断开 / 复制诊断摘要」。
- **服务器 Server**：主机地址、SSH 端口、用户名、系统 ssh.exe 路径、本地 SOCKS 端口、SSH 密钥路径、Host Key 首次确认、测试连接。禁止保存 SSH 明文密码，首版只支持 SSH 密钥或受控 SSH agent；不能弹出隐藏不可见的交互式密码等待。
- **应用 Apps**：CLI「通过网关启动」按钮；Desktop / IDE「检测进程」「查看路由检测结果」「生成/应用受控规则」（高级）；显示每类：已验证 / 部分验证 / 未验证 / 未受支持，不用含糊的「全部已代理」。
- **诊断 Diagnostics**：SSH 生命周期日志（脱敏）、本地端口监听、SOCKS 连通性、出口 IP、Mihomo 是否可用、相关进程及路径、最近一次命中情况；提供导出脱敏报告。
- **设置 Settings**：开机启动可选（默认关闭）；断线重连可选；断线策略默认「停止本工具启动的新请求并警告」，非透明直连；代理/路由高级集成默认关闭。

无广告、无需注册网关账户；不使用大型仪表盘堆砌装饰。GUI 简体中文优先，文本预留 i18n。

## 5. 核心模块设计

### 5.1 SSH 隧道管理器（P0）

启动前检查：`ssh.exe` 存在、端口可绑定、私钥路径可访问、服务器信息完整。所有命令参数作为参数数组传给 `ssh.exe`（不走 `cmd /c`、不拼接字符串）。示例仅说明行为，**开发时不可把明文凭据拼入命令**：

```powershell
ssh -N -D 127.0.0.1:17801 `
  -o ExitOnForwardFailure=yes `
  -o ServerAliveInterval=30 `
  -o ServerAliveCountMax=3 `
  -o BatchMode=yes `
  -i "C:\path\to\id_ed25519" `
  -p 22 user@vps.example.com
```

- Host Key：使用系统 OpenSSH 的 `known_hosts` 验证；首次连接明确展示指纹并由用户确认，**不得自动使用 `StrictHostKeyChecking=no`**；主机指纹变化则阻断并报警。
- 在 Windows 上跟踪本工具创建的 `ssh.exe` PID / 进程句柄，只清理自己创建的进程，不 `taskkill /IM ssh.exe /F` 杀掉用户其他 SSH 会话。
- 检测失败应分别报告：DNS/地址不可达、鉴权失败、Host Key 异常、本地端口被占用、远端禁止 TCP forwarding、隧道建立但目标不可达。
- 可配置有限重连（指数退避，默认最多 3 次），断网或断连立刻把 UI 改为「断开」，不得继续展示旧出口 IP 为当前状态。
- 若 sshd 上 `AllowTcpForwarding` 被禁用，先告知用户，**不要自动编辑 `/etc/ssh/sshd_config`**。

### 5.2 出口验证与 DNS（P0）

使用 SOCKS5 的远端 DNS 方式连接校验目标（例如 `curl --socks5-hostname ...` 的等效实现）；与未使用网关时的出口分别展示。验证至少包含：127.0.0.1 端口监听、经隧道 TCP/TLS 可达、出口 IP 回显，且全部带时间戳。可在设置中更换测试服务，防止仅依赖单个公网 IP 网站。

- 测试成功 ≠ 某个 Codex 进程必然走了该出口；UI 必须分开显示「隧道测试」与「应用路由验证」。
- 不记录访问 URL 完整路径、研究内容或模型 prompt；必要时只记录域名与状态码，默认诊断日志不抓请求/响应正文。
- DNS、IPv6、WebSocket 和代理软件冲突属于明确的诊断项目；V1 不宣称任意 IPv6/UDP 都已经通过 SSH SOCKS 代理。

### 5.3 CLI 专用启动器（P0）

- 用户在网关中点击「启动 Codex CLI」，工具检测可执行文件及当前隧道状态，再启动**独立 PowerShell 进程**。
- 只向新启动的 CLI 及所需子进程注入代理：`HTTP_PROXY`、`HTTPS_PROXY`、`NO_PROXY=localhost,127.0.0.1,::1`。如支持 HTTP CONNECT，则指定本地 HTTP 代理；如所用版本经测试支持 SOCKS，再决定是否使用 `ALL_PROXY`。
- **不要将本地 SOCKS5 地址直接填入 `HTTPS_PROXY=http://...`**；HTTP 代理与 SOCKS 协议不同。
- 防止流量循环：SSH 连接本身不通过自己创建的隧道；本地回调地址和 `localhost` 必须直连。
- 进程级环境变量不写入用户/系统环境变量，不影响其他终端；提供启动前后预览「将注入哪些设置」。
- 对 Codex 沙盒/工具子进程的环境继承采取**最小化原则**，遇到 Windows sandbox proxy ports 问题应记录并提示，不为了跑通擅自关闭隔离或切换成不安全权限。

### 5.4 Codex Desktop / IDE 集成（P1，逐个验证）

初版不依靠「猜进程名」自动生效。流程：

1. 用户启动 Desktop / IDE 并在网络活动时执行进程发现，获取 PID、可执行路径、父子进程关系；在 Mihomo 连接面板中关联目标连接（如已开启 TUN / 进程发现）。
2. 识别对应进程才生成候选 `PROCESS-NAME` / `PROCESS-PATH` 规则；**通用 `Code.exe`、`node.exe`、`ssh.exe` 不得直接全量归为 Codex 流量**。
3. 使用 Mihomo 前先检测已开启的 TUN、当前模式、路由规则、同名节点冲突及回环/SSH 绕行条件。
4. 采用用户确认后「生成规则片段 + 备份 + 增量应用」模式；优先使用 Clash Verge Rev 的扩展脚本/扩展配置入口，不直接覆写在线订阅文件。规则放在原有最终 `MATCH` 之前，且不会覆盖其他业务规则。
5. 用户可查看「受影响应用」预估范围和回滚方案。若无法区分 VS Code 插件与共用的扩展宿主流量，标示为「只能代理 VS Code 相关进程整体」并需另行确认，默认不执行。
6. 连接记录未提供确凿进程归属时，改用「人工核实/部分覆盖」而不是声明全部成功。对登录浏览器、WSL2、Docker/容器内 Codex 给单独提示。

**实现注意**：Clash Verge Rev 的扩展配置、扩展脚本接口与版本有关；先做版本识别、能力检测、回滚设计。严禁假定它暴露稳定通用 API 用于热写订阅配置。即便 Mihomo controller 存在，也不能假定 controller 能永久修改 Clash Verge 所管理的 profile。可先交付人工导入/撤销流程，再做版本适配。

### 5.5 本地 HTTP CONNECT → SSH SOCKS5（P1，按需实现）

只有在实际测试表明目标 Codex 客户端无法直接使用 SOCKS5、必须通过 HTTP(S) 代理时，才实现或使用经审计的本地转换组件：

- 监听 `127.0.0.1:<另一个随机端口>`，仅支持必需的 HTTP CONNECT/TCP 语义；向下游 SSH SOCKS5 发起连接。
- 不进行 TLS MITM，不安装根证书，不缓存请求内容；拒绝来自局域网/公网的连接。
- 限制连接数和空闲超时，拒绝循环代理、明文管理端点；明确日志及错误码。
- 若此模块不可用，CLI 启动器不得自动改用直连后假装生效。

## 6. 状态机、异常处理与回滚

```text
UNCONFIGURED → READY → CONNECTING → TUNNEL_READY → EGRESS_VERIFIED
                      ↘ ERROR / DISCONNECTED
EGRESS_VERIFIED → DEGRADED（应用规则未验证/出口测试失败）→ RECONNECTING
所有状态 → DISCONNECTING → READY
```

- `TUNNEL_READY` 只说明 ssh 动态转发监听可用；`EGRESS_VERIFIED` 说明经隧道出口实测成功；「应用已走网关」须另有证据。
- 隧道掉线：马上更新 UI、暂停本工具发起的新 CLI 启动；可选重连，不修改用户其他软件的代理规则；不要承诺第三方应用的全局 kill-switch。
- 退出/卸载：先还原本工具建立的配置、终止本工具子进程，**仅恢复自己修改的具体项**；原始 Clash 配置不可被覆盖；用户在连接期间改动了配置时应检测冲突并提示人工选择，禁止粗暴覆盖。
- 所有写配置操作：先备份、生成 diff、解析校验、原子写入、读取回验、失败自动回滚。版本升级后不再支持时禁用自动写入，仅提供手动指导。

## 7. 安全与隐私规范

1. SSH host key 验证必需；私钥不复制入应用；支持系统 SSH agent，私钥口令由标准安全交互处理。
2. 本地 SOCKS 和 HTTP 代理只监听 `127.0.0.1`；不得自动设置 `GatewayPorts yes` 或开放云服务器新公网代理端口。
3. 不保存 OpenAI OAuth、ChatGPT Cookie、API Key，不读取 Codex 认证目录。
4. 日志默认只写：时间、组件、连接状态、错误类别、脱敏服务器地址/域名；不能泄漏研究文档、项目代码、HTTP 请求正文、授权 Header。
5. 不需要管理员权限的基本模式不得提权；若用户选择 Mihomo TUN，高权限由 Mihomo 官方组件处理，本应用显示具体影响与确认。
6. 明示独立网络出口不保证模型质量、账号访问、服务地区支持或团队订阅合规；不制作绕过账号限制/匿名伪装功能。

## 8. 里程碑、交付物和验收标准

### M0：环境调研与风险清单（先做）

交付：检测脚本、Codex Desktop/CLI/IDE 版本与进程树样本、当前 Mihomo/Clash Verge 版本及集成能力记录、SSH 预检报告。**只读取，不改系统或服务器配置。** 输出 `docs/architecture.md` 和可实现功能矩阵；用户确认后进入开发。

### M1：能真实工作的隧道 MVP（最高优先级）

交付：Windows GUI、服务器配置、密钥/Host Key 交互、SSH 一键启停、本地端口检测、SOCKS 远端 DNS 测试、出口 IP、基本日志。

验收：
- 输入一台已可 SSH 登录的 Ubuntu 服务器，连接后经 SOCKS 的 IP 为服务器出口；普通未经过网关的应用不因此改变原有出口。
- 故意使用错误密钥、错误端口/指纹、不允许转发的远端设置、本地端口占用，各有明确错误提示。
- 断开及退出仅停止本工具创建的 SSH 子进程；服务器现有服务未改动。

### M2：Codex CLI 启动与可验证路由

交付：CLI 专用启动器、HTTP CONNECT 桥接层（若必需）、连接诊断；文档说明使用官方 CLI 登录的正常路径。

验收：
- 同一个 Windows 下，经过本工具启动的 CLI 请求可在本地网络/受控测试端点观测到服务器出口；其他终端启动的无关应用不被修改。
- 本地 OAuth 回调 `localhost` 不被意外转发；SSH 失联时 UI 明确显示不可用。
- 工具不能把「HTTP CONNECT 桥接异常」误报成「账号 OAuth 失效」。

### M3：Mihomo/Clash Verge 的受控集成

交付：版本及进程检测、规则生成/预览、配置备份/回滚、连接页面路由核对说明。先针对实际测试通过的 Desktop 版本交付，再研究 IDE。

验收：
- 原机场订阅及已有规则无损、订阅更新后不丢失网关规则；未开启 Mihomo/TUN 时清晰提示「Desktop/IDE 尚未覆盖」。
- 验证 SSH 连接不回流自身；Codex Desktop 的已识别连接命中 `MY-VPS`；VS Code 通用进程不会在未经明确确认的情况下全量导流。
- 软件退出后能撤销自己应用的修改；发生与用户同时改配置的冲突时只报告冲突，不强制覆盖。

### M4：打包与交付

交付：`LostCodexGateway` Windows 安装包、可复现构建命令、README、配置指南、故障排除、脱敏诊断导出、单元/集成/手工验收报告、开源许可证与第三方依赖清单。

验收：Windows 10/11 实机安装/卸载，重启与断网/重连，SSH 密钥/Host Key 失败，端口占用，现有 Clash TUN 已开启，以及 Codex Desktop/CLI/IDE 兼容性矩阵。不支持的情形在 GUI 明示，而不是静默失败。

## 9. 推荐开发顺序与任务拆分

| 优先级 | 任务 | 完成判定 |
|---|---|---|
| P0 | 项目初始化 + 配置结构 + GUI 状态机 | 空配置可启动，错误状态不误报成功 |
| P0 | SSH 子进程监管与 Host Key 安全 | 一键连接/断开，故障有区分，其他 SSH 不受影响 |
| P0 | SOCKS 连通性、远端 DNS 和出口检测 | 能展示验证结果与时间戳 |
| P0 | Codex CLI 专用启动器 | 不修改全局环境，仅子进程使用指定代理 |
| P1 | 诊断导出与安全回滚 | 可复现故障但不泄漏凭据 |
| P1 | Desktop 实际进程发现及 Mihomo 规则 | 必须有真实路由证据；保留原配置 |
| P1 | VS Code 插件识别/作用范围确认 | 不误把整个 VS Code 当成插件独立进程 |
| P2 | WSL2 专项支持、HTTP 桥接增强、托盘与开机启动 | 逐个完成单独测试再发布 |

## 10. 测试矩阵（不可省略）

- 连接：正常 SSH；首次 host key；错误 host key；错误密钥；远端不允许转发；本机端口冲突；服务器重启；网络闪断；服务器域名解析失败。
- 流量：经 SOCKS 出口 IP；未指定出口对照；DNS 远端解析；IPv6 未支持/部分支持提示；WebSocket 长连接；SSH 连接是否形成环路。
- 客户端：官方 CLI Native Windows；Desktop Native Windows；VS Code 插件；如 Desktop 通过 WSL2 启动应记录为单独场景，不得套用 Windows 进程规则验收。
- 共存：Clash Verge 未运行 / 运行但 TUN 关闭 / TUN 开启；代理模式切换；订阅更新；现有多节点/多端口；其他应用及其他 SSH 会话不受影响。
- 权限与安全：普通用户可执行 P0；只有明确选择高级网络功能才提示相应权限；断线与退出可还原；恶意/错误输入不进入 shell；日志无密钥/Token/项目内容。

## 11. 给用户的第一版操作流程（完成后须匹配）

1. 安装 LostCodexGateway，首次选择「只配置基础网关」。
2. 输入服务器地址、SSH 用户名/端口、选择本地私钥文件；检查并确认服务器 host key。
3. 点击「连接并检测出口」；分别显示隧道连通性和服务器出口 IP。
4. 点击「从网关启动 Codex CLI」进行有限测试；不修改其他程序。
5. 若需要 Desktop / IDE，进入「高级：Mihomo 应用分流」，先检测真实进程和 TUN，再预览修改并由用户确认，最后验证连接命中。
6. 点击「断开并还原」；只有本工具创建/添加的连接与配置被清理。

## 12. 需要开发智能体优先核实的未知项

- 用户的 Clash Verge 是哪个具体分支/版本？现有 TUN 模式与配置增强方式是什么？有无可安全使用的只读 Mihomo Controller？
- Windows Codex Desktop 的实际 exe/后台进程与网络路径是否经 WSL2；IDE 插件是独立可识别进程还是通用扩展宿主？
- 当前 Codex CLI 是否正确接受 HTTP CONNECT 代理环境变量；是否和 Windows Sandbox 工具冲突？
- SSH 服务器现有账号是否允许 TCP Forwarding；目前使用的服务器身份认证与 Host Key 是否规范？

**以上未知项必须通过本地能力检测 + 用户确认，不能让开发智能体凭记忆把猜测写成「已支持」。**

## 13. 参考资料（先核对最新版实现）

1. OpenSSH `ssh -D` 动态端口转发与参数说明：https://man.openbsd.org/ssh
2. Mihomo 路由规则（PROCESS-NAME / PROCESS-PATH）：https://wiki.metacubex.one/en/config/rules/
3. Mihomo 常规配置、进程发现、外部控制器：https://wiki.metacubex.one/en/config/general/
4. Clash Verge Rev 扩展配置/脚本机制：https://www.clashverge.dev/guide/extend.html
5. Clash Verge Rev 扩展脚本例子：https://www.clashverge.dev/guide/script.html
6. Codex 官方仓库：https://github.com/openai/codex
7. Windows Desktop/WSL2 不必然继承系统代理的公开兼容性反馈：https://github.com/openai/codex/issues/15447
8. Windows Codex SOCKS5 与 HTTP 代理兼容性反馈（只是用户反馈，不当成稳定规范）：https://github.com/openai/codex/issues/20844
9. Windows Codex 代理与 sandbox 的兼容性反馈：https://github.com/openai/codex/issues/30538

---

**最终交付原则：先实现「可验证的一键 SSH 出口 + CLI」，再实测集成 Desktop / IDE。不要为了实现看似漂亮的全自动开关而覆盖用户现有网络配置，或把尚未验证的客户端请求标成已通过服务器。**
