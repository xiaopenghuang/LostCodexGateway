# LostCodexGateway — 验收与测试记录

> 按里程碑记录实测结果。每项都有命令/日志摘录可复现。状态：✅通过 / ⚠️部分 / ❌失败（附原因）
>
> **关于 IP 地址**：本记录中的公网 IP 已替换为 [RFC 5737](https://www.rfc-editor.org/rfc/rfc5737)
> 文档保留段（`203.0.113.0/24`、`198.51.100.0/24`）以脱敏真实端点。
> 替换为**一对一映射**，故「两值相同/不同」的对比结论与相对关系仍然成立；
> 被验证的是测试方法与判定逻辑，具体数值不构成证据。

## M1：能真实工作的隧道 MVP

### 1.1 环境与夹具

| 项 | 值 |
|---|---|
| 测试日期 | 2026-09-20 |
| 主机 | Windows 10 专业版 10.0.19045 x64 |
| 系统 OpenSSH | `C:\Windows\System32\OpenSSH\ssh.exe` 9.5p1 |
| 测试服务器 | Docker 容器 `lcfg-test-sshd`（alpine 3.20 + OpenSSH 9.7，ed25519 密钥认证，`AllowTcpForwarding yes`），映射 `127.0.0.1:2222` |
| 决定性验证服务 | `lcfg-test-web`（Docker 内部网络 `lcfg-test-net`，仅容器网络可达，宿主机直连不通） |
| 夹具脚本 | `tests/fixtures/ssh-server/setup.ps1`（生成密钥、构建镜像、启动容器；`-Teardown`/`-Clean` 可清理） |

### 1.2 单元测试（cargo test，无需夹具）

```
test result: ok. 13 passed; 0 failed
```
覆盖：配置默认值与序列化往返、状态机初始状态/迁移/日志、ssh 参数数组构造（无明文凭据）、
stderr 错误分类（DNS/鉴权/HostKeyChanged/HostKeyUnknown/端口占用/远端禁止转发/隧道断开）、
base64 解码与 SHA256 指纹格式、IP 响应解析（纯文本/JSON/IPv4/IPv6）、端口监听负例。

### 1.3 集成测试（cargo test --test tunnel_e2e -- --ignored，需 Docker 夹具）

```
test result: ok. 5 passed; 0 failed   (finished in 4.89s)
```

| 测试 | 结果 | 证据 |
|---|---|---|
| `e2e_tunnel_remote_dns_reaches_server_only_service` | ✅ | 用产品代码 `ssh::spawn_tunnel` 启动真实隧道 → `verify::fetch_http_via_socks` 经 SOCKS5 远端 DNS 解析 `lcfg-test-web`（宿主机解析不了的名字）拿到 HTTP 200 `LCFG-TUNNEL-OK`；出口 IP 验证序列通过；停止后端口释放 |
| `e2e_wrong_key_classified_auth_failed` | ✅ | 私钥路径不存在 → stderr `Permission denied (publickey)` → 分类 `AuthFailed` |
| `e2e_wrong_port_classified_unreachable` | ✅ | 端口 2223 → `Connection refused` → `HostUnreachable` |
| `e2e_dns_failure_classified` | ✅ | 域名不存在 → `Could not resolve hostname` → `DnsFailed` |
| `e2e_local_port_busy_classified` | ✅ | 先占端口 → `bind [127.0.0.1]:17815: Permission denied` + `cannot listen to port` → `LocalPortBusy` |

### 1.4 手工端到端（真实 ssh.exe + curl/协议探针）

**决定性验证**（`tests/e2e/socks_probe.py`，原始 SOCKS5 协议）：
```
handshake resp: 0500                        ← 无认证握手成功
REP code: 0 (success)                       ← CONNECT 域名形式成功（远端 DNS）
HTTP response head: HTTP/1.1 200 OK ... LCFG-TUNNEL-OK
```
隧道出口、服务器直连、现有代理三方对照：
```
tunnel egress:  198.51.100.22   ← 经隧道 = 服务器出口 ✅
server direct:  198.51.100.22   ← 服务器自身直连出口（一致）
existing proxy: 198.51.100.88    ← 用户现有代理出口（不同，证明未混淆）
```

### 1.5 M1 验收项核对（对照开发文档 §8 M1）

| 验收项 | 结果 | 说明 |
|---|---|---|
| 输入可 SSH 登录的服务器，连接后经 SOCKS 的 IP 为服务器出口 | ✅ | 见 1.3/1.4（夹具替代真实服务器；用户真实服务器待用户提供后按同一流程验证） |
| 普通未经过网关的应用不因此改变原有出口 | ✅ | 全程未改系统代理/路由/环境变量；对照出口保持 198.51.100.88 不变 |
| 错误密钥 → 明确错误提示 | ✅ | `AuthFailed` 分类 + UI 显示 |
| 错误端口/指纹 → 明确错误提示 | ✅ | 错误端口 → `HostUnreachable`；指纹变化场景手工验证 ssh 输出 `REMOTE HOST IDENTIFICATION HAS CHANGED`（本应用严格模式阻断） |
| 远端不允许转发 → 明确错误提示 | ✅ | 夹具曾误配 `AllowTcpForwarding no`（Alpine 默认值覆盖），实测 sshd 拒绝 + 应用分类 `RemoteForwardDenied`（修复夹具后验证通过） |
| 本地端口占用 → 明确错误提示 | ✅ | `LocalPortBusy` |
| 断开及退出仅停止本工具创建的 SSH 子进程 | ✅ | 测试仅 `Stop-Process` 自己记录的 PID；验证 17801 端口释放、无残留 ssh.exe |
| 服务器现有服务未改动 | ✅ | 仅使用自建 Docker 容器；宿主机的 ssh/代理/路由/known_hosts 均未被本应用代码修改（测试期间对 known_hosts 的手工操作仅为夹具条目，且已备份） |

### 1.6 M1 期间发现并修复的问题

1. **Alpine sshd 默认 `AllowTcpForwarding no`**（OpenSSH 取首次生效值，追加无效）→ 夹具用 sed 替换；这同时验证了「远端禁止转发」场景的应用行为。
2. **Alpine `adduser -D` 账户锁定 + `UsePAM` 不受支持** → 夹具 `passwd -u` + 移除 UsePAM。
3. **OpenSSH 端口占用 stderr 为 `bind [...]` 格式**（非 `bind: `）→ 分类器补充 `bind [` 与 `could not request local forwarding`。
4. **端口占用行先于鉴权行到达** → 分类器调整优先级（LocalPortBusy 先于 AuthFailed）。
5. **curl 对 `--socks5-hostname` 的本地解析行为不稳定**（Git/系统 curl 均出现过本地解析）→ 产品验证路径用自研 SOCKS5 客户端（reqwest socks5h），协议探针证实远端 DNS 生效。
6. **Docker Desktop 无 HTTPS 代理无法直连 Docker Hub**（实测确认）→ 夹具镜像经 DaoCloud 镜像站拉取（`docker.m.daocloud.io/library/alpine:3.20`），不改 Docker Desktop 配置。

### 1.7 GUI 端到端（真实 Tauri 窗口，CDP 驱动）

通过 WebView2 remote-debugging 驱动**真实应用窗口**走完整用户流程（`tests/e2e/cdp_e2e.mjs`）：

```
[2. 尝试连接] -> {"ok":true,"v":"连接成功：出口 IP 203.0.113.47"}
[5. 轮询快照] state: EGRESS_VERIFIED  egress_ip: 203.0.113.47
    [FAIL] 本机对照出口 IP -> 直连失败（本机直连被限制或网络策略拦截）   ← 如实报告，不虚报
    [OK] 127.0.0.1:17801 端口监听
    [OK] SOCKS5 握手与 CONNECT（远端 DNS）
    [FAIL] 隧道出口 IP（ipify 失败: 服务器侧不可达）                    ← 多端点容错
    [OK] 隧道出口 IP -> 203.0.113.47                                    ← 回退端点成功
[6. 断开] -> 已断开  状态: READY
[7. 导出诊断报告] -> %APPDATA%\LostCodexGateway\diagnostics_*.txt
```

关键点：出口 IP 203.0.113.47 与容器内服务器直连出口一致；ipify 在服务器侧不可达时自动回退第二个端点（ip.3322.net），证明多端点容错有效；直连对照失败被如实标记而非隐藏。

### 1.8 M1 期间发现并修复的问题（续）

7. **Windows 9.5 的 ssh-keyscan 不支持 `-o` 选项、且对服务器优先的 sntrup761 KEX 处理有缺陷** → 夹具侧限制 KEX 列表（测试资产）；应用代码不加 `-o`，若用户服务器仅声明 sntrup761，错误信息会明确提示。
8. **known_hosts 条目端口语义**：ssh 连非 22 端口时查询 `[host]:port` 条目，ssh-keyscan 输出不带端口 → 写入时重写 host 前缀为 `[host]:port`；查询时同时兼容两种格式。
9. **本机直连被限制时 connect 卡 30 秒**（直连对照探测按端点逐个等超时）→ 对照探测改为单端点、5 秒快速失败；隧道出口验证保持多端点容错。
10. **监控任务异常退出导致 disconnect 的 abort 无人接收、子进程残留** → disconnect 增加兜底：按本工具记录的 PID 直接终止（先 tasklist 确认该 PID 是 ssh.exe，绝不 taskkill /IM 影响用户其他会话）。

## M2：Codex CLI 启动器与可验证路由

### 2.1 代理能力实测（决定性实验，非猜测）

| 实验 | 结果 | 结论 |
|---|---|---|
| codex.cmd 实际结构 | npm shim → spawn `@openai/codex-win32-x64/vendor/.../codex.exe` | **Codex 0.154.0 是 Rust 原生二进制**，不是 Node/undici 应用 |
| Node 22 裸 fetch + 代理环境变量 | 全部忽略（undici 默认不读 env） | 不能用 Node 行为推测 Codex |
| 记录型假代理 A/B：无代理 vs HTTPS_PROXY | 无代理 0 记录；设 HTTPS_PROXY 后收到 `CONNECT api.github.com:443` 等 | **Codex 原生二进制确实读取 HTTP(S)_PROXY** |
| ALL_PROXY=socks5:// | Codex 仍发 HTTP CONNECT 文本（把 socks5 URI 当 HTTP 处理） | **SOCKS 直供无效 → HTTP CONNECT 桥接层必需**（文档 5.5 条件满足） |

### 2.2 桥接层实现（bridge.rs，按文档 5.5 约束）

- 仅绑定 `127.0.0.1:0`（随机端口）；**每个连接再查一次对端是否回环**（不止靠绑定）
- CONNECT → 下游 SOCKS5（域名形式=远端 DNS）；无 TLS MITM、无缓存、无根证书
- 限并发 64、请求头 16KB 上限、10s 握手超时；统计连接总数与最后目标域名
- 单元测试：CONNECT 头解析（合法/拒绝矩阵、IPv6 字面量）

### 2.2b 桥接层安全加固（P2，2026-09-21）

对照 §5.5 逐条核对时发现「文档承诺 vs 实际实现」有四处缺口，全部补齐：

| 文档承诺 | 加固前 | 加固后 | 验证方式 |
|---|---|---|---|
| 拒绝来自局域网/公网的连接 | `_peer` 被丢弃，未校验 | 显式校验对端回环，非回环记 `BRIDGE_NON_LOOPBACK_PEER` 后断开 | 单元 + 集成 |
| 限制空闲超时 | `IDLE_TIMEOUT_SECS` **定义了但从未使用** | 双向复制带真正的空闲超时（有字节流动即重置计时） | 单元 ×4（短窗口）+ 集成 |
| 拒绝循环代理 | 无检查，可 `CONNECT 127.0.0.1:<socks>` 成环 | 目标黑名单：回环 / RFC1918 / CGNAT / 链路本地 / 未指定 / 组播 / 桥接自身端口 | 单元 + 集成 |
| 明确日志及错误码 | 仅 `eprintln`，无结构化错误码 | 9 个稳定 `BRIDGE_*` 错误码 + 响应头 `X-LCFG-Reject` + 环形拒绝记录（诊断页展示） | 单元（码唯一性）+ 集成 |

加固过程中发现并修掉一个**真实缺陷**：最初用 `timeout(idle, copy_bidirectional(..))` 实现空闲超时，
那是**总时长**上限而非空闲语义——会把持续有数据的长连接（如 SSE 流）在固定时间点硬切断。
正确实现改为「任一方向有字节流动即重置计时」。
`idle_window_resets_on_activity` 这个测试针对旧实现会失败、针对新实现通过，是这次修复的决定性证据。

**集成测试（免 Docker，可常规运行）** —— 用真实 socket 验证「拒绝得干净」而不只是「拒绝了」：

| 测试 | 断言 |
|---|---|
| `bridge_rejects_loopback_and_private_targets_without_forwarding` | 回环/localhost/私有网段 → 403 + 对应 `BRIDGE_*` 码；非 CONNECT → 400；**且这些请求一个都没到达上游 SOCKS5**（否则等于没防住） |
| `bridge_rejects_self_target_to_prevent_proxy_loop` | 目标是桥接自身端口时被拒绝（防形成 桥接→SOCKS→ssh→桥接 的环） |
| `bridge_forwards_public_target_and_records_it` | 公网域名放行、原样以域名形式下发（远端 DNS 语义）、统计正确、拒绝计数为 0 |
| `idle_timeout_does_not_kill_active_connection` | 持续有数据流动的连接不会被空闲超时误杀（跨两轮收数据验证计时器被重置） |

统计口径新增 `bridge_rejects_total` 与 `bridge_recent_rejects`，在「应用」页展示，
并明确说明「拒绝是设计内的保护，不是故障」——避免用户把安全拦截误读为程序出错。

### 2.3 启动器（launchers.rs）

- 定位 codex（PATH codex.cmd / 已知原生路径）；启动独立 PowerShell 子进程
- 注入 `HTTP_PROXY/HTTPS_PROXY=http://127.0.0.1:<bridge>`、`NO_PROXY=localhost,127.0.0.1,::1`（大小写双份）
- **绝不注入 socks5:// 地址当 HTTP 代理**；不写用户/系统环境变量；启动前有预览
- 隧道非 EGRESS_VERIFIED 或桥接未运行 → 拒绝启动并说明原因

### 2.4 M2 端到端（真实 app + 真实 codex + 桥接统计证据）

```
[1. 配置并连接] 连接成功：出口 IP 203.0.113.47
    状态: EGRESS_VERIFIED  桥接端口: 32455
[3. 真实 codex doctor 经桥接运行]  Codex Doctor v0.154.0 正常输出
[4. 桥接统计] 连接数 0 → 4，最后目标 persistent.oaistatic.com:443
[6. 断开] 状态 READY，桥接端口已清空
```

- codex doctor 的 4 条 CONNECT（含 api.github.com、persistent.oaistatic.com）全部经桥接 → SOCKS5 → ssh 隧道
- 断开后桥接任务终止、端口释放（bridge_port: null）

### 2.5 M2 验收核对（文档 §8 M2）

| 验收项 | 结果 |
|---|---|
| 经本工具启动的 CLI 请求可在本地网络/受控测试端点观测到服务器出口 | ✅ 桥接统计 + 出口 IP + sshd 日志（Accepted publickey 对应转发会话） |
| 其他终端启动的无关应用不被修改 | ✅ 注入仅限子进程；未改用户/系统环境变量（实测用户级 HTTP_PROXY 为空） |
| 本地 OAuth 回调 localhost 不被意外转发 | ✅ NO_PROXY=localhost,127.0.0.1,::1 注入子进程 |
| SSH 失联时 UI 明确显示不可用 | ✅ 监控任务置 DISCONNECTED；launch 在非 EGRESS_VERIFIED 拒绝启动 |
| 桥接异常不误报 OAuth 失效 | ✅ 桥接层独立报错（502/启动失败日志），与登录流程分离 |


## 网络诊断模块（新增需求：docs/LostCodexGateway — 网络诊断模块开发需求.md）

### 实施阶段与实测（2026-09-20）

| 阶段 | 内容 | 实测 |
|---|---|---|
| M1 | SSH 隧道检测（PID 存活 + 端口监听 + SOCKS 握手 + 经 SOCKS 建连测试目标）+ 出口 IP 对照（本地直连 vs 网关 SOCKS，均显式指定路径、不依赖系统代理） | ✅ 集成测试 `diag_e2e`（4 项） |
| M2 | 服务器只读连通性（新开短 SSH 会话跑固定命令模板：DNS/TCP/HTTPS 探测，35s 超时） | ✅ `server_diag_is_read_only`：检测前后 sshd_config 哈希一致 |
| M3 | Codex 进程发现（进程名+路径+命令行归类 Desktop/CLI/IDE）+ Mihomo /connections 只读关联（secret 仅内存） | ✅ 真实环境验证 + 修复 2 个分类 bug |
| M4 | DNS/IPv6/延迟检测 + 全局 75s 硬超时（网络断开不卡死） | ✅ |
| M5 | 网络诊断页面（总览 8 项/路径可视化 4 跳/客户端独立检测/日志筛选/脱敏导出）+ GUI e2e + 实际运行截图 | ✅ 截图 docs/screenshots/network-diagnostics.png |

### 关键实测结果（真实窗口，GUI e2e）

```
状态: EGRESS_VERIFIED（先连接隧道）
诊断耗时: 10588ms
隧道: ok（PID存活+监听+握手+转发建连 四项全过）
本地出口: 203.0.113.47 | 网关出口: 203.0.113.47 (IPv4)
匹配结果: unconfirmed（未配置预期IP）→ 设错预期IP后正确判定 mismatch + 提示「出口 IP 不匹配」
延迟: socks握手 68ms | 网关HTTPS 476ms | 服务器 290ms
客户端: Desktop=未验证(6进程，TUN关) CLI=未运行 IDE=未验证(Code.exe宿主)
Mihomo: running=true, tun=false, conns=0（secret 未提供时如实显示）
IPv6 经网关: 未验证（如实声明 SSH SOCKS5 ≠ VPN）
```

### 诊断模块验收核对（需求文档 §七，10 条）

| # | 验收项 | 结果 |
|---|---|---|
| 1 | SSH 已连接但 SOCKS 无法转发时识别异常 | ✅ `diag_detects_dead_tunnel`：伪造 PID 999999 + 端口无监听 → 全部 Error，不报正常 |
| 2 | 本地/网关出口独立检测 | ✅ 本地用直连、网关显式 socks5h + 远端 DNS，互不依赖 |
| 3 | Desktop/CLI/IDE 分别显示真实结果 | ✅ 真实环境：Desktop=未验证（TUN 关）、CLI=未运行、IDE=未验证，不虚报 |
| 4 | 无法确认的路由不显示已验证 | ✅ RoutingStatus 枚举区分 verified/partial/unverified/anomaly/unconfirmable；CLI 无活动时是 partial 而非 verified |
| 5 | 网络断开不无限重试/卡死 | ✅ 全局 75s 硬超时 + 每项独立超时（诊断总时长实测 ≤11s） |
| 6 | 不修改服务器配置 | ✅ `server_diag_is_read_only`：检测前后 sshd_config 哈希一致 |
| 7 | 不覆盖用户 Clash/Mihomo 配置 | ✅ 只读 /connections + 只读检测；secret 仅内存不落盘 |
| 8 | 不读/存/导出认证凭据 | ✅ 诊断日志只记域名/状态码/错误类别/IP |
| 9 | 生成脱敏诊断报告 | ✅ export_diagnostics 复用；日志视图支持按级别/项目/时间筛选 |
| 10 | 断开网关不影响原网络 | ✅ 断开后 READY；全程不改系统代理/路由 |

### 开发中发现并修复的真实 bug（非模拟数据）

1. `codex-computer-use-swift.exe`（Desktop 的 computer-use 组件）被误归为 CLI → 修正归类顺序（Desktop 组件优先）。
2. `esbuild.exe`（dev 环境 vite 相关进程）被误抓进 Codex 进程 → 修正规则：**路径必须含 codex 标记**才算 Codex 相关，无关 node/esbuild 一律排除（单元测试锁定反例）。
3. PowerShell 进程枚举输出中文乱码 → 显式 UTF8 输出编码。
4. **点击任意功能时闪现 PowerShell/控制台黑框**（用户报告）→ 根因：GUI 进程调用 ssh-keygen / ssh-keyscan / tasklist / powershell / ssh 等控制台工具时未设 `CREATE_NO_WINDOW`，Windows 为每个子进程新建可见控制台窗口。修复：新增 `src-tauri/src/procutil.rs`，统一为所有后台子进程加 `CREATE_NO_WINDOW`（「应用」页启动 Codex CLI 的交互终端除外，需保留可见窗口）。验证：单元/桥接/诊断/隧道 e2e 全绿；对 debug 与 release 安装版各跑一遍 `tests/e2e/cdp_window_check.mjs`——在连接、诊断、Mihomo 检测、测试连接、抓取主机密钥全套动作期间以 200ms 轮询可见控制台窗口，动作期捕获 **0 个黑框**，且监测器自检（故意弹窗）确认检测逻辑可信。

## M3：Mihomo / Clash Verge 受控集成

### 3.1 只读检测实测（真实环境）

`detect_mihomo` 输出与 M0 人工探测完全一致：

```json
{
  "verge_installed": true, "verge_version": "2.5.2",
  "verge_running": true, "mihomo_running": true,
  "mixed_port": 2080, "external_controller": "127.0.0.1:9097",
  "tun_enabled": false, "mode": "rule",
  "notes": [
    "TUN 未开启：Desktop/IDE 的进程级分流未覆盖（如实报告）",
    "external-controller 设置了 secret（本工具不读取、不保存该值）"
  ]
}
```

### 3.2 M3 验收核对（文档 §8 M3）

| 验收项 | 结果 |
|---|---|
| 原订阅及已有规则无损 | ✅ 本工具不写任何 Mihomo 文件；规则片段只是文本供用户手动导入 |
| 订阅更新后不丢网关规则 | ⚠ 取决于用户 profile 链（v1 如实说明，见 risks R4） |
| 未开启 TUN 时清晰提示「尚未覆盖」 | ✅ 检测 notes + Apps 页黄色提示 + 状态徽章 |
| SSH 连接不回流自身 | ✅ SSH 连接直连服务器（不经隧道）；桥接只绑回环 |
| VS Code 通用进程不整体导流 | ✅ 片段生成不包含 Code.exe/node.exe/ssh.exe（单元测试锁定） |
| 退出撤销本工具修改 | ✅ 本工具不改任何 Mihomo 文件；备份/回滚按钮供用户自主操作 |
| 与用户同时改配置的冲突 | ✅ restore 检测内容不一致时拒绝覆盖并提示人工处理（单元测试语义） |

## M4：打包与交付

### 4.1 构建产物

```
src-tauri/target/release/bundle/nsis/LostCodexGateway_0.1.0_x64-setup.exe  (2.86 MB)
```

- 构建命令：`npm run tauri build`（beforeBuild 自动 `vue-tsc && vite build`）
- 可复现：Rust 1.98.1 + MSVC 14.50 + node v22.17.1 + WebView2 153

### 4.2 安装/卸载冒烟（实机）

| 步骤 | 结果 |
|---|---|
| NSIS 静默安装 | exit 0；注册表卸载项 DisplayName=LostCodexGateway 0.1.0 |
| 启动安装版 | 窗口标题 LostCodexGateway，进程正常 |
| 静默卸载 | exit 0；安装目录删除；注册表项清除 |
| 残留检查 | 无 ssh.exe/lostcodexgateway/vite 残留；无 17801/9223/1420 监听残留 |

### 4.3 交付物清单

| 交付物 | 位置 |
|---|---|
| 源码 | 本仓库（src-tauri Rust + src Vue/TS） |
| Windows 安装包 | src-tauri/target/release/bundle/nsis/LostCodexGateway_0.1.0_x64-setup.exe |
| 使用说明 | README.md + docs/setup-windows.md |
| 测试记录 | docs/acceptance-report.md（本文档） |
| 架构 | docs/architecture.md |
| 故障排除 | docs/troubleshooting.md |
| 安全 | docs/security.md |
| 风险/未实现 | docs/risks-and-unimplemented.md |
| 环境调研 | docs/m0-environment-report.md / .json |
| 测试夹具 | tests/fixtures/ssh-server/（Docker，可 Teardown/Clean） |
| 许可证/依赖 | LICENSE + THIRD_PARTY.md |

### 4.4 最终测试汇总

- Rust 单元测试：**21 passed**（config/ssh 分类/指纹/桥接解析/规则片段/状态机）
- 集成测试 tunnel_e2e：**5 passed × 3 轮串行稳定**（Docker 夹具）
- 集成测试 bridge_e2e：**1 passed**（原始 CONNECT 决定性路径）
- GUI e2e（CDP 驱动真实窗口）：配置→指纹→连接→EGRESS_VERIFIED→断开→诊断导出，全通过
- M2 CLI 实测：真实 codex doctor 经桥接 4 条 CONNECT（persistent.oaistatic.com:443）
- 安装/卸载：全通过

---

## 多服务器切换（v0.3.0，2026-09-21）

> 需求原话：「是切换，不是同时，就像 clash 中切换节点一样。」
> 本文只记录**实际跑过的**验证。真机端到端（两台真实服务器）**尚未做**，见文末。

### 5.1 后端（Rust）

```
$ cargo test
test result: ok. 79 passed; 0 failed   (lib)
test result: ok.  4 passed; 0 failed; 1 ignored  (bridge_e2e)
test result: ok.  0 passed; 0 failed; 9 ignored  (tunnel_e2e 5 + diag_e2e 4，均需 Docker 夹具)
```

`cargo check --all-targets` 零警告零错误。

新增/改写的单元测试（72 → 79）：

| 测试 | 断言的不变量 |
|---|---|
| `switching_servers_keeps_local_ports` | 来回切换后 `socks_port` / `bridge_port` **一个字节都不变**（这是「切换对 CLI 透明」的前提） |
| `rename_and_host_change_preserve_id` | 改名 + 改主机后 `id` 不变，`active_server_id` 不悬空 |
| `expected_egress_ip_is_per_server` | 预期出口 IP 各自独立，改一台不污染另一台；序列化往返后仍独立 |
| `removing_active_server_falls_back_to_existing` | 删当前项后选中项落到真实存在的服务器；删到最后一台时列表不被删空 |
| `migrates_legacy_single_server_config` | v0.2.0 扁平 `server` 无损迁移，含两处字段搬家 |
| `legacy_proxy_mode_is_dropped` | 死字段 `proxy_mode` 不再出现在新配置里 |
| `legacy_invalid_socks_port_is_rejected` | 旧配置里 <1024 的端口回落到默认值 |
| `new_format_repairs_dangling_active_id` | 新格式里 `active_server_id` 指向已删除项时自动落到第一台 |

最后 3 条（`failed_switch_never_leaves_switching_state` /
`failed_switch_keeps_more_informative_states` / `disconnected_is_actionable`）
是**发布前代码审计**补的回归测试，背景与推导见 §5.6。

### 5.2 配置迁移实测

用真实的 v0.2.0 配置形状（扁平 `server` + `verify.expected_egress_ip` +
`server.socks_port` + `settings.proxy_mode`）喂给 `parse_json`：

| 迁移前 | 迁移后 | 结果 |
|---|---|---|
| `server.host/port/username/key_path` | `servers[0]` 同名字段 | ✅ |
| `server.server_name` | `servers[0].name` | ✅ |
| `server.socks_port: 18999` | `settings.socks_port: 18999` | ✅ |
| `verify.expected_egress_ip` | `servers[0].expected_egress_ip` | ✅ |
| `settings.proxy_mode` | 丢弃 | ✅ |

### 5.3 前端

```
$ npm run build
✓ 33 modules transformed.
dist/assets/index-Dccyrqej.css   20.57 kB │ gzip:  5.00 kB
dist/assets/index-D--l_9fa.js   132.36 kB │ gzip: 48.25 kB
✓ built in 2.44s
```

`vue-tsc --noEmit` 通过（类型定义与 Rust 端字段严格对齐）。

**开发夹具未进生产构建**（逐个标记核对，`grep -o <标记> dist/…js | wc -l`）：

| 夹具唯一标记 | 产物中出现次数 |
|---|---|
| `sg-backup-very-long-hostname` | 0 |
| `203.0.113.200` | 0 |
| `203.0.113.47` | 0 |
| `198.51.100.88` | 0 |
| `备用出口（新加坡` | 0 |
| `东京中转节点` | 1 ← 这是「显示名称」输入框的 placeholder，不是夹具数据 |

### 5.4 视觉验证（headless Chrome + CDP，非真机窗口）

场景：`VITE_LCFG_FIXTURE=switching`（3 台服务器，含超长名称、超长主机名、
不可达项、`previous_server_id` 已设置），深浅两主题各截 3 张。

**用测量代替目测**（脚本内 `getBoundingClientRect` 遍历）：

| 指标 | 实测 | 判定 |
|---|---|---|
| 表格横向溢出 | 0 px | ✅ |
| 卡片内溢出元素 | 0 个 | ✅ |
| 文档横向滚动 | 1344 = 1344 | ✅ |
| 操作列按钮垂直中心 vs 行中心 | 0 / 0 / 0 px | ✅ 对齐 |

行高 50 / 70 / 50 px —— 第二行因名称换行变高，属预期。

### 5.5 对比度实测（WCAG 2.1，浏览器内算真实生效色）

不改 CSS 变量取值靠猜，而是在页面里读 `getComputedStyle`，把半透明背景
沿祖先链逐层合成后计算比值。

**修复前**（7 项不达标）：

| 元素 | 主题 | 比值 | 要求 |
|---|---|---|---|
| 延迟-快 | light | 3.62 | 4.5 |
| 延迟-慢 | light | 3.07 | 4.5 |
| 活动行「当前」药丸 | light | 2.91 | 4.5 |
| 状态药丸 info | light | 4.16 | 4.5 |
| 侧栏底部副标题 | light | 4.43 | 4.5 |
| 主按钮 | dark | 3.22 | 4.5 |
| 危险按钮 | dark | 3.02 | 4.5 |

根因在**共享样式**：`--ok` / `--warn` / `--err` / `--accent` / `--info` 是按
「当背景/边框/图形」调的，直接拿来写小号文字在浅色主题上不够。

修法：新增文字专用变体 `--*-fg`（深色主题下与原值相同 → 视觉零变化；
浅色主题下加深），全局替换 15 处 `color: var(--语义色)`；
`background:` / `border-color:` 保持原变量不动。取值按**最苛刻背景**
（药丸自身半透明底色合成到高亮行上）卡线：

| 变量 | 浅色取值 | 白底 | 页面底 | 高亮行 | 药丸合成 |
|---|---|---|---|---|---|
| `--ok-fg` | `#0c7049` | 6.12 | 5.66 | 5.49 | 4.99 |
| `--warn-fg` | `#8a5708` | 6.09 | 5.63 | 5.46 | 4.98 |
| `--err-fg` | `#bf2b30` | 5.83 | 5.39 | 5.23 | 4.68 |
| `--accent-fg` | `#265cbe` | 6.26 | 5.80 | 5.61 | 5.07 |
| `--info-fg` | `#4350c8` | 6.52 | 6.03 | 5.85 | 5.28 |

另将浅色 `--tx-3` 由 `#67718a` 调至 `#606a82`（原值在侧栏琥珀底与高亮行上
只有 4.42 / 4.38）。

**修复后**：7 项不达标 → **2 项**，且两者都是 0.1.0 起的既有按钮配色，
本次未动（改动会波及全局视觉）：

| 元素 | 主题 | 比值 | 一行修法 |
|---|---|---|---|
| 主按钮（白字） | dark | 3.22 | 底色改用 `--accent-lo` `#3a6fd8` → 4.72 |
| 危险按钮（白字） | dark | 3.02 | 底色改用 `#c62f34` → 5.44 |

### 5.6 发布前代码审计：发现并修复「切换失败卡死」

本节记录的是**发布前自查**发现的缺陷，不是外部反馈。放在打包之前，是因为它改变了
0.3.0 的产物内容。

**现象**：切换服务器时若 `connect_impl` 在**预检**阶段失败，应用会永久停在
`SWITCHING`：界面「连接」与「断开」两个按钮都是禁用的，而 `switch_server` 又拒绝
在 `Switching` 下再次切换 —— 用户除了重启应用没有出路。

**根因**是三条约束叠加，单看每条都合理：

| # | 事实 | 位置 |
|---|---|---|
| 1 | `switch_server` 在断开阶段就把状态置为 `Switching` | `disconnect_with(.., Switching, Switching)` |
| 2 | `connect_impl` 的 7 条预检全部在 `state = during` **之前** `return Err`，失败不落状态 | `connect_impl` 开头 |
| 3 | `Switching` 下两个按钮都禁用，且后端拒绝再次切换 | `Dashboard.vue` 的 `busy` / `canDisconnect`；`switch_server` 首段 |

第 2 条本身没错：对普通「连接」而言状态没动过，失败后停在 `Ready` 是正确的。
只有在「状态已被切换流程改成 `Switching`」这个前提下，它才变成致命缺陷。

**最容易踩到的触发路径**：已连接 A → 新增服务器 B（未确认 Host Key）→ 点「切换」。
`doSwitch` 只对「会断流」做二次确认，**不检查 Host Key**；`connect_impl` 走到
「服务器 Host Key 尚未确认」即 `return Err`，此时状态仍是 `Switching`。
这不是边角情况，是首次使用多服务器的必经路径。

**修法**：

- 新增 `state_after_failed_switch(current)`：仅当状态**仍是** `Switching` 时落到
  `Disconnected`；若已推进到 `Error` 则原样保留（那个状态信息量更大）。
- 新增 `leave_switching(app, machine)`：应用上述规则，并在真的发生改动时广播快照。
- `switch_server` 的两条失败路径（`config::save` 失败、`connect_impl` 失败）都调用它。
- 顺带修正 `previous_server_id` 的赋值时机：改到**落盘成功之后**才记录，
  避免「保存失败、根本没切换」时界面冒出一个点了就报错的「切回上一个」。

**验证**：

```
$ cargo test --lib
test result: ok. 79 passed; 0 failed; 0 ignored   (76 → 79)
$ cargo check --all-targets
Finished `dev` profile [unoptimized + debuginfo] target(s) in 9.78s   （零警告零错误）
```

新增 3 条回归测试：

| 测试 | 断言的不变量 |
|---|---|
| `failed_switch_never_leaves_switching_state` | `Switching` 必须落到 `Disconnected`——停在 `Switching` 会让连接与断开按钮同时失效 |
| `failed_switch_keeps_more_informative_states` | 已推进到 `Error` 等状态不被兜底覆盖 |
| `disconnected_is_actionable` | 落点 `Disconnected` 既不算「隧道活跃」（否则删服务器/改端口被无谓拒绝），也不算「连接进行中」（否则用户无法按「连接」重试） |

**前端侧实测**——把「`Switching` 是死状态」这条前提从「读代码推断」变成「量真实 DOM」：

脚本 `.workbuddy-ai/shots/state-buttons.cjs`（headless Chrome + CDP，夹具逐场景注入，
读的是按钮真实的 `disabled` 属性，不是源码）。为了让同一个页面能遍历全部场景，
`src/dev/apply-fixture.ts` 增加了 dev-only 的 `window.__applyFixture` 入口——
否则每个场景都要重启一次 dev server，遍历成本高到没人愿意跑。

```
场景            state 文案      连接可用  断开可用  可用按钮数  期望
verified        出口已验证        true     true        2        可/可  ✓
suspect         隧道已通          true     true        2        可/可  ✓
connecting      重连中            false    false       0        禁/禁  ✓
switching       切换服务器中      false    false       0        禁/禁  ✓
disconnected    已断开            true     false       1        可/禁  ✓
error           错误              true     false       1        可/禁  ✓

=== DISCONNECTED 恢复路径（服务器页）===
  「切回上一个」: {"present":true,"enabled":true,"text":"切回上一个（东京中转节点）"}
```

- `switching` 的**可用按钮数为 0** —— 这正是卡死缺陷的机制，在真实 DOM 上确认。
- `disconnected` 有 1 个可用按钮，且「切回上一个」**存在且可用**，恢复路径成立。
- 顺带实测到一个**既有行为**（非本次修复目标，仅记录）：已连接的两个状态里
  「连接」按钮**仍可点**，点下去得到「已有连接在进行中」的明确报错，不会造成破坏。

**夹具改动不影响已发布的安装包**（已用哈希验证）：`src/dev/` 只在
`import.meta.env.DEV` + `VITE_LCFG_FIXTURE` 下被动态导入，生产构建整块摇掉。
改动后重跑 `npm run build`，产物仍是 `index-Bb2FDCga.js` / `index-Dccyrqej.css`，
与安装包内嵌的资源名逐字一致。

> **诚实声明（这条修复到底验证到什么程度）**：
> - ✅ 前端「`SWITCHING` 下两个按钮都禁用」与「`DISCONNECTED` 下仍有可用按钮、
>   「切回上一个」可用」—— **已在真实 DOM 上实测**（见上表）。
> - ✅ 后端「切换失败必须落到 `Disconnected`」—— **纯函数单元测试**覆盖。
> - ❌ 「真机上走一遍：已连接 A → 切到未确认 Host Key 的 B → 界面仍可继续操作」
>   这条完整链路**未验证**（需要双服务器环境），见 §5.8。

### 5.7 打包（0.3.0）

```
$ npm run tauri build        # beforeBuild 自动执行 vue-tsc --noEmit && vite build
✓ 33 modules transformed.
dist/assets/index-Dccyrqej.css   20.57 kB
dist/assets/index-Bb2FDCga.js   132.42 kB
   Compiling lostcodexgateway v0.3.0
    Finished `release` profile [optimized] target(s) in 3m 58s
     Running makensis to produce ...\bundle\nsis\LostCodexGateway_0.3.0_x64-setup.exe
    Finished 1 bundle at: LostCodexGateway_0.3.0_x64-setup.exe
```

| 项 | 值 |
|---|---|
| 安装包 | `src-tauri/target/release/bundle/nsis/LostCodexGateway_0.3.0_x64-setup.exe` |
| 大小 | 3,081,696 B（2.94 MiB）；0.2.0 为 3,040,609 B，增量 +41,087 B |
| 安装包 SHA-256 | `ef1ba41b71445f621177599534cf546406e113d33e4fdeff7e7a18fb8d28a0e4` |
| 主程序 SHA-256 | `1cf9899ded4fa991d826aca06c6b67d7aa39eaf7b764c0f70e0c1903c13df96e` |
| 构建时间 | 2026-09-22 17:06（本地） |
| 工具链 | cargo 1.98.1 (797e8a9bc 2026-08-05) / node v22.17.1 / npm 11.19.0 |

> 本节记录的是**第二次构建**（含 §5.6 的卡死修复）。第一次构建的产物
> （3,080,529 B / `bad61059…1591b`）**已作废**：它不含该修复，
> 且从未发布过（未建 Release、无人下载），故直接覆盖。
> 两次构建的前端资源名完全一致（`index-Bb2FDCga.js` / `index-Dccyrqej.css`），
> 说明差异确实只来自 Rust 侧。

产物级核对——**不看构建日志的自述，直接查二进制**：

| 检查 | 方法 | 结果 |
|---|---|---|
| 打进去的是本次 0.3.0 前端，而不是残留的旧 `dist` | 在主程序里搜内嵌资源名 `index-Bb2FDCga.js` / `index-Dccyrqej.css` | 各命中 1 次 ✅ |
| 版本串已更新 | 主程序内搜 `0.3.0` | 命中 1 次 ✅ |
| 开发夹具未进产物 | 主程序内搜 6 个夹具标记（超长主机名、3 个文档保留 IP、2 个夹具 id） | 全 0 ✅ |

第一条是关键：`dist/` 是构建前刚生成的，若 `beforeBuildCommand` 被跳过或缓存命中，
安装包里就会是上一版前端。资源名能对上，说明前端确实重新构建并内嵌了。

> **一条关于证据强度的说明（含一次自我纠正）。** Tauri 把前端资源**原样内嵌**
> （不是压缩包）——这一点由「资源文件名能直接在二进制里搜到」反证。所以对**主程序**
> 做夹具字符串扫描是有效证据。
>
> 对**安装包**做同样的 `grep -a` 扫描则**无效**：NSIS 整体 LZMA 压缩，实测连
> `0.3.0`、`LostCodexGateway` 都是 0 次。**但结论不是「安装包无法验证」**——
> 那是把「这个方法不行」错说成「这件事不行」。改用 **UTF-16LE 解码**后即可验证
> 安装包内嵌的元数据（NSIS 的 AppName / 版本串是 UTF-16LE 存放的）：
>
> ```
> $ python verify-installer-meta.py .../LostCodexGateway_0.3.0_x64-setup.exe
>   LostCodexGateway           2 次
>   0.3.0                      2 次
>   0.2.0                      0 次
> ```
>
> 新版本号在、旧版本号无，说明安装包元数据确实是 0.3.0，不是上一版残留。

### 5.7.1 打包（0.4.0，环境自检）

- 命令：`npm run tauri build`（`beforeBuildCommand` 自动跑 `npm run build`）
- 结果：`Finished 1 bundle`，耗时 3m 59s（release 编译）
- 产物：

| 项 | 值 |
|---|---|
| 安装包 | `src-tauri/target/release/bundle/nsis/LostCodexGateway_0.4.0_x64-setup.exe` |
| 安装包大小 | 3,102,098 字节 |
| 安装包 SHA-256 | `56cad7face2fb430b9a87466d00ce646d411644e9e1a15b966306e34f43c270e` |
| 主程序 | `src-tauri/target/release/lostcodexgateway.exe`（11,863,040 字节） |
| 主程序 SHA-256 | `0093bb7cfb87547ea9b90caa2247a217806f4f2372b1429ec416bced807a50ae` |
| 前端资源 | `index-Bzlsnv9z.js`（137.93 kB）/ `index-BEW_n_at.css`（21.92 kB） |

**元数据核验**（UTF-16LE 解码，脚本已参数化支持任意版本号）：

```
$ python verify-installer-meta.py .../LostCodexGateway_0.4.0_x64-setup.exe 0.4.0
  LostCodexGateway           2 次
  0.4.0                      2 次
  0.1.0 / 0.2.0 / 0.3.0      0 次
结论：元数据校验通过
```

**内容核验**（在主程序二进制里搜特征串）：

| 核验点 | 0.3.0 旧包 | 0.4.0 新包 | 判定 |
|---|---|---|---|
| `环境自检` | 0 | 1 | ✅ 新功能已进包 |
| `ssh_keygen` / `push_pubkey` | 0 | 1 / 1 | ✅ 新 guide 项已进包 |
| `codex_install` / `openssh_portable` | 0 | 1 / 1 | ✅ 4 个 guide 全覆盖 |
| `便携 OpenSSH` | 0 | 1 | ✅ |
| `__lcfgStore` / `__applyFixture` / `VITE_LCFG_FIXTURE` | — | 全 0 | ✅ 夹具零泄漏 |
| 上一版前端资源名（`index-DxSUP0iI` 等） | — | 全 0 | ✅ 无旧前端残留 |

**关于「主程序里搜不到 `0.4.0`」的说明**：这是**正确的**，不是缺陷。
Tauri 应用的产品版本由 `tauri.conf.json` 提供，运行时经 `app.package_info()` 读取，
**不硬编码进 Rust 代码**。所以版本号只出现在 NSIS 元数据里（已验 2 次）。
主程序里唯一的 `0.3.0` 是 `config.rs:4` 的历史变更注释（`//! ## 多服务器（v0.3.0）`），
其余形如 `0.4.4` / `0.5.1` 的均为第三方 crate 版本串，与本产品无关。

### 5.8 未验证（明确区分）

以下**没有**验证，不得当作已通过：

- ❌ **真机端到端**：`A 断开 → B 连上 → 出口 IP 变成 B 的`。需要第二台服务器。
- ❌ **真机窗口渲染**：上述截图来自 headless Chrome，不是 Tauri 窗口。
  字体、缩放比、GPU 合成路径都不同。
- ❌ **切换后旧 ssh 进程无残留**：逻辑上由 `generation` + `kill_process_by_pid`
  覆盖，但未在真机上 `tasklist` 核对过。
- ❌ **安装包冒烟**：0.3.0 与 0.4.0 安装包**均已产出**（见 §5.7 / §5.7.1），
  但**静默卸载未重跑**。0.1.0 做过完整套（见 §4.2）。
  **安装 + 注册表卸载项已在真机验证**（见 §5.10），故此处只剩「静默卸载」一项。
- ❌ **0.4.0 环境自检的端到端实测**：**已在真机验证**（见 §5.10），
  本条已关闭。
- ❌ **单实例互斥体测试在软件运行时会失败**：`acquire_returns_some_when_no_other_instance`
  与真实 app 共用 `Local\LostCodexGateway` 互斥体，app 在跑时该测试必然失败。
  这是**环境干扰**，非代码缺陷；跑全量测试前需先关闭应用。

### 5.10 真机安装与端到端实测（0.4.0，2026-09-23）

用户在自己机器上装了 `LostCodexGateway_0.4.0_x64-setup.exe` 并运行，补齐了
此前只能靠夹具模拟的**前后端拼接处**验证。

**安装验证**（注册表读取，`reg.exe` 被拉黑故改用 Python `winreg`）：

| 项 | 值 |
|---|---|
| `DisplayName` | `LostCodexGateway` |
| `DisplayVersion` | **`0.4.0`** ✅ 与 tag 一致 |
| `InstallLocation` | `G:\Practical-tools\LostCodexGateway` |
| `UninstallString` | `G:\Practical-tools\LostCodexGateway\uninstall.exe` ✅ 卸载项已写入 |
| `Publisher` | `lostcodex` |

**安装后的主程序核验**：

| 核验点 | 结果 |
|---|---|
| 文件大小 | 11,863,040 字节 —— 与打包产物**完全一致** ✅ |
| `环境自检` / `重新体检` | 命中 ✅ |
| `ssh_keygen` / `push_pubkey` / `codex_install` / `openssh_portable` | 全部命中 ✅ |
| 夹具泄漏（`__lcfgStore` / `__applyFixture` / `VITE_LCFG_FIXTURE` / `egress_verified`） | 全 0 ✅ |
| 安装后 SHA-256 | `0e4b4e38e1e34ae3aeffbcf115e3d1619025d6bf2a892ebfca3b19b8ba80bd62` |

> **注意**：安装后的 SHA-256 与打包产物（`0093bb7c...`）不同，这是**正常的** ——
> NSIS 安装时会往 exe 里打补丁（写入安装路径等元数据，即打包日志中的
> `Patching ... with bundle type information`）。大小一致 + 内容特征一致
> 足以证明是同一构建。

**端到端实测（真实 Tauri 窗口，非夹具）**：

用户在真机点了「环境自检」→「重新体检」，页面表现：

| 观察点 | 值 | 判定 |
|---|---|---|
| 页面标题 | 环境自检 | ✅ |
| 结论徽标 | 可以正常使用（绿） | ✅ |
| 通过 / 总数 | **8 / 14** | ✅ 真实数据 |
| 阻塞项 | 0 | ✅ |
| 检查时间 | `2026-09-23 10:58:14` | ✅ **真实时间戳**（夹具用固定值 `00:40:00`） |
| `服务器 TCP 转发` 项 | 待判定 + 5 条操作步骤 | ✅ `unknown` + `guide` 组合按设计渲染 |
| `WSL2（可选）` 项 | 待判定 | ✅ |

**为什么这次是真数据而非夹具的证明**：时间戳是当前时刻（非夹具固定值），
且条目数 14 与夹具的 11 不同。**前后端拼接处无问题。**

**同时首次确认了条目数的环境浮动性**（此前我口算成 13）：

| 环境 | 条目数 | 差异 |
|---|---|---|
| 本仓库 dev 环境（未装 Clash Verge） | 13 | 缺 `mihomo_fragment` |
| **用户真机（已装 Clash Verge）** | **14** | 齐全 |
| 任意环境 + `bridge_port` 被占用 | 15 | 多 `bridge_port` |

已加回归测试 `core_items_are_always_present` 钉住「核心集合 + 13–15 区间」，
不再依赖人脑计数。单元测试 89 → 90。

### 5.11 用户反馈驱动的修复（2026-09-23，未发版）

两轮用户实测反馈，各修一个缺陷。**均已修复并推送，但尚未进入安装包。**

#### 5.11.1 「保存端口设置」报 `missing required key socksPort`

用户点设置页的「保存端口设置」，报：

```
invalid args `socksPort` for command `save_settings`:
command save_settings missing required key socksPort
```

**根因**：Tauri 2 默认把 Rust 侧 snake_case 参数名转成 camelCase 后从前端入参取。
后端签名是 `socks_port`，前端却传了 `socks_port` —— Tauri 找的是 `socksPort`。

逐个核对全项目 12 个带参命令，**只有 `save_settings` 错**（其余如
`set_expected_egress_ip` / `generate_mihomo_fragment` 前端都写的 camelCase）。

**为什么测试没抓到 —— IPC 边界的盲区**：

| 测试层 | 是否经过 IPC 序列化 |
|---|---|
| 前端（夹具替换了 `invoke`） | ❌ 否 |
| 后端（直接调 Rust 函数） | ❌ 否 |

两侧都不走序列化，参数名写错时**两边都「通过」**。这类错误只能静态扫源码发现。

**新增卡口** `scripts/check-ipc-args.py`：静态比对前端 `invoke` 键名 vs 后端命令
参数名（按 Tauri 规则转 camelCase），退出码 1 可用于 CI。支持对象字面量与
「传变量 + 回溯类型注解」两种形态（后者正是本 bug 的形态）。

**脚本有效性已实测**（非「看起来能跑」）：把 bug 注入回去 → 脚本精确报出
`save_settings 缺少参数 ['socksPort', 'bridgePort']`；恢复后退出码 0。

#### 5.11.2 「直连失败」红 ✗ 与「全部通过」徽标并存

用户截图反馈：出口验证步骤里「本机对照出口 IP」是红色 ✗ + 「直连失败」，
而徽标写着「全部通过」，问「这到底要不要紧」。

排查出三个问题，**都不是用户环境的问题**：

**① `direct_egress` 只试第一个端点（真 bug）**

原实现取 `endpoints.first()`，失败即放弃。默认第一个端点是 `api.ipify.org`，
实测本机对它 **DNS 能解析、TLS 握手被重置**，第二个端点 `ipinfo.io` 正常：

```
$ curl --noproxy '*' https://api.ipify.org?format=json
HTTP=000 耗时=2.96s          ← 失败
$ curl --noproxy '*' https://ipinfo.io/ip
117.139.221.115  HTTP=200    ← 成功
```

`api.ipify.org` 的失败细节（`curl -v`）：

```
* Host api.ipify.org:443 was resolved.
* Recv failure: Connection was reset
* schannel: failed to receive handshake, SSL/TLS connection failed
```

即 DNS 通、TCP 建连后 TLS 被 RST —— 典型的网络中间设备干扰特征。

对照步骤因此**永远失败**。而隧道侧是遍历所有端点的，两侧行为不一致。

**修法**：改为**并发尝试所有端点、取第一个成功**。并发而非串行是为了控耗时 ——
这一步在「连接」流程里是串行的（原注释「不能让连接卡 30 秒」就是这个顾虑），
串行会让最坏耗时随端点数累加；并发后总耗时 ≈ 单个端点超时。

**实测验证**（`cargo test --lib direct_egress_falls_back -- --ignored --nocapture`）：

```
修复前：失败
修复后：direct_egress => ok=true detail=117.139.221.115   （耗时 0.46s）
```

不仅修好，还比原来快（不必干等第一个端点超时）。

**② 参考项失败被渲染成错误视觉**

`direct_ip` 不参与结论判定（`ok = listening && socks_ok && egress_ok`），
但前端对所有步骤一律 `s.ok ? ✓ : ✗`，于是辅助步骤失败也标红，与徽标冲突。

修法是把语义**显式化**而非在前端硬编码 kind 字符串：`VerifyStep` 新增
`advisory: bool`（`serde(default)` 兼容旧快照），`direct_ip` 标为 true。
前端据此用中性标记 `–` + 灰色 `.step-mark.info`，标签加「（参考项）」，
底部解释「参考项不参与通过判定」。

**③ 直连失败时把原因文案当成 IP 显示**

`directIp` computed 原来无条件取 `direct_ip` 步骤的 `detail`，而失败时
`detail` 是原因说明，于是整句文案被塞进「本机直连对照」那一格。
改为只在 `ok === true` 时取，失败显示「未取得」。

**真机 DOM 实测**（`.workbuddy-ai/shots/verify-steps.cjs`，3 场景全通过）：

| 场景 | 徽标 | 直连步骤标记 | 对照格 |
|---|---|---|---|
| `verified` | 全部通过 | ✓（绿） | `198.51.100.22` |
| `suspect` | 未通过 | ✓ | 显示「出口相同」警示 |
| `direct_unavailable` | **全部通过** | **`–`（中性灰）** | **未取得** |

截图：`.workbuddy-ai/shots/verify-steps-direct-unavailable.png`

#### 5.11.3 状态

| 项 | 值 |
|---|---|
| 提交 | `9b09d7d`（IPC 修复）、`80b45d3`（verify 修复） |
| 单元测试 | 90 → 92（verify 新增 2 条 + 1 条 `--ignored` 网络测试） |
| **是否已进安装包** | ❌ 否（0.4.0 里没有）→ ✅ **已于 0.4.1 交付**，见 §5.12 |

**注**：这两处修复都在 `0.4.0` 发布之后，故 `v0.4.0` tag 与 Release 的内容
**不包含**它们。已重新打包为 `0.4.1`，并对「主程序内嵌的前端资源」做了
逐字节核验，见 §5.12。

### 5.12 打包 0.4.1：两处修复的交付验证（2026-09-23）

§5.11 的两处修复发布在 `0.4.0` 之后。用户仍装的是 0.4.0，故升版 `0.4.1`
重新打包（语义化版本：bug 修复走 patch，不覆盖已发布的 `0.4.0`）。

**版本号改动**（3 处 + 文档 2 处）：

| 文件 | 改动 |
|---|---|
| `src-tauri/tauri.conf.json` | `0.4.0` → `0.4.1` |
| `src-tauri/Cargo.toml` | `0.4.0` → `0.4.1` |
| `package.json` | `0.4.0` → `0.4.1` |
| `README.md` | 徽标 + 安装包名 |
| `README.en.md` | 徽标 + 安装包名 —— **发现上一轮漏更新，徽标还停在 `v0.3.0`** |

**构建**：

- 命令：`npm run tauri build`（`beforeBuildCommand` 自动跑 `npm run build`）
- 结果：`Finished 1 bundle`，耗时 4m 18s
- 前端资源：`index-lx8Q1hXh.js`（138.80 kB）/ `index-DwaECxpP.css`（22.01 kB）

| 项 | 值 |
|---|---|
| 安装包 | `src-tauri/target/release/bundle/nsis/LostCodexGateway_0.4.1_x64-setup.exe` |
| 安装包大小 | 3,110,970 字节 |
| 安装包 SHA-256 | `d0cbada78cfefaeb96267dddec4da4d828cffec211ee110a81ae66209ea943a1` |
| 主程序 | `src-tauri/target/release/lostcodexgateway.exe`（11,899,904 字节） |
| 主程序 SHA-256 | `f75206733ec4ad52278cb14aaed271fd453d463e3ae15b7860e41691b77d5a7e` |

**元数据核验**（UTF-16LE）：

```
$ python verify-installer-meta.py .../LostCodexGateway_0.4.1_x64-setup.exe 0.4.1
  LostCodexGateway           2 次
  0.4.1                      2 次
  0.1.0 / 0.2.0 / 0.3.0 / 0.4.0   0 次
结论：元数据校验通过（新版本号在、旧版本号无）
```

#### 5.12.1 内嵌前端资源的逐字节核验（本轮新增）

**动机**：要证明「两处修复确实进了安装包」。但**版本号对了不代表前端是新的**
—— 前端是编译进 exe 的，`tauri.conf.json` 的版本号只影响 NSIS 元数据。

**踩到的假阴性**：想用 `grep -a` 在主程序里搜修复特征串（如 `__lcfgReady`），
结果是 **0 次**。原因是 **Tauri 2 把前端资源 brotli 压缩后嵌入**，压缩流里
没有明文。于是「搜不到」被误读成「修复没进包」。

**做法**：新建 `.workbuddy-ai/shots/verify-embedded-assets.py` —— 定位资源路径
明文，从其后逐字节解压 brotli 流，与 `dist/` 里的源文件**逐字节比对**。

```
$ python verify-embedded-assets.py src-tauri/target/release/lostcodexgateway.exe dist

资源路径                            源大小      解出    压缩流  结果
/assets/index-DwaECxpP.css       22,007    22,007    4,935  ✓ 逐字节一致 (路径偏移 0x8930b0)
/assets/index-lx8Q1hXh.js       138,799   138,799   46,664  ✓ 逐字节一致 (路径偏移 0x8944d5)
/index.html                         403       403      185  ✓ 逐字节一致 (路径偏移 0x894411)

结论：主程序内嵌的前端资源与本次 dist/ 逐字节一致
```

因为 `dist/assets/index-lx8Q1hXh.js` 里含有修复特征（`socksPort` × 1、
`advisory` × 6、`direct_ip` × 2），且文件名 `lx8Q1hXh` 是 vite 的**内容哈希**
（0.4.0 时是 `index-Bzlsnv9z.js`，不同），故可确认：

| 修复 | 载体 | 在产物中的证据 |
|---|---|---|
| IPC 参数名 camelCase | 前端 JS | 内嵌 JS 与 `dist` 逐字节一致，含 `socksPort` |
| `directIp` 只在 ok 时取值 / 中性标记 | 前端 JS | 同上，含 `advisory` × 6、`direct_ip` × 2 |
| `direct_egress` 并发遍历 | Rust（明文） | 主程序里 `advisory` × 2、`本机对照出口 IP` × 1、`[verify] direct egress failed:` × 1 |

**Rust 侧是明文**（字符串常量不压缩），所以能直接搜；前端侧必须解压。
两类证据合起来才完整。

#### 5.12.2 写这个脚本时踩的三个坑

1. **python-brotli 的 `Decompressor.process()` 分块喂入不可靠。**
   同一段数据（内嵌的 `index-lx8Q1hXh.js`）：

   | chunk 大小 | 解出字节 | 结果 |
   |---|---|---|
   | 1 | 138,799 | ✅ 完整，与 dist 一致 |
   | 64 | 138,790 | ✗ 差 9 字节后报错 |
   | 1024 | 136,970 | ✗ |
   | 8192 | 118,746 | ✗ |
   | 65,536 | 0 | ✗ 立刻报错 |
   | 200,000 | 0 | ✗ |

   断点位置随分块大小变化 ⇒ **是绑定的分块行为，不是数据被截断**。
   改用**逐字节喂入**（4.6 万次调用，实测 0.55s，可接受）。
   差点因此得出「安装包里的前端被截断」的错误结论。

2. **不能用「下一个资源路径的偏移」当压缩流的右边界。**
   三个资源路径在主程序里的排列顺序（`/index.html` → `*.css` → `*.js`）
   与 `dist/` 目录顺序**不同**，且并非两两相邻。逐字节解压能自然停在流结束处，
   不需要预先知道长度。

3. **同一个路径字符串在主程序里可能出现多次，只有一处是资源表项。**
   实测 `/index.html` 出现 **5 次**：`0x890e32` 那处后面是结构体数据
   （`00 00 00 08 f8 8f 40 01 ...`），解压第 8 字节就报错；`0x894411`
   才解出 403 字节的 `<!doctype html>`。
   故判据改为「**解压结果与源文件逐字节一致**」，而不是「找到了路径」。

#### 5.12.3 测试与验收

```
cargo test --lib -- --skip single_instance
  → 91 passed; 0 failed; 1 ignored; 2 filtered out
    （2 条 single_instance 用例因用户 app 正在运行而跳过，见 R24）
vue-tsc --noEmit + vite build   → 通过
verify-installer-meta.py        → 通过
verify-embedded-assets.py       → 3/3 逐字节一致
```
