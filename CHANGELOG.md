# 变更日志

本项目遵循[语义化版本](https://semver.org/lang/zh-CN/)。

## [0.2.0] — 2026-09-21

### 变更（不兼容）

- **诊断数据结构**：客户端路由判定不再使用单一的「桥接是否有连接」布尔量，
  改为结构化证据 `BridgeEvidence { tunneled, attempts, has_rejects }`。
  依赖旧字段的外部消费方需同步调整。
- **桥接统计**：新增 `connections_total` / `connections_tunneled` 的语义区分。
  `connections_total` 现在是**连接尝试数**（含被拒与上游失败的），
  判定「流量是否真的进了隧道」必须用 `connections_tunneled`。
- **Mihomo 检测**：`MihomoDetection` 新增 `verge_path` 字段。

### 修复

- **CLI 路由判定会把「试图走网关但失败」报成「已验证」**（严重）
  - `build_clients` 的 `cli` 分支原先只判断「网关可能可达 && 桥接有过连接」，
    且用的是 `connections_total`——该计数在 TCP 刚连上来时就自增，
    此时目标是否合法、`ssh -D` 能否建连都还未知。
    于是「目标非法 / SOCKS5 建连失败 / 并发满」的尝试也被计入，
    把失败冒充成成功，且该分支永远得不出「异常」结论。
  - 桥接层新增 `connections_tunneled`，仅在**目标校验通过 + 下游 SOCKS5 建连成功
    + 已回 200** 之后自增，并记录脱敏的 `recent_targets`。
  - CLI 三态化：有 tunneled 且网关卡可达 → 已验证；仅有尝试或存在拒绝 → 异常；
    零动静 → 未验证。
  - 这与项目既定原则「无法确认即不得声称已验证」保持一致。
- **网关不可达时误报「路由配置异常」**
  - Desktop/IDE 的 `any_other` 分支原先无条件报异常。现已把「网关卡自身可用」
    作为报异常的必要条件——否则「本地网关没起来」会被误导成「路由配错了」。
- **Clash Verge 版本永远读不到（硬编码安装路径）**
  - 原先写死 `D:\Clash Verge\clash-verge.exe`，装在非默认盘符即失效。
  - 改为四层定位：注册表卸载项 `DisplayIcon` → 运行中进程映像路径 →
    常见安装目录（由环境变量与盘符动态推导）→ `PATH`；
    全部失败则如实报告「未定位到」，**不猜路径**。
  - 新增 `verge_path` 并在「应用」页展示，便于排查。

### 其他

- 应用内新增「Verge 安装路径」显示行。
- 文档补充路由判定矩阵（`docs/architecture.md` §3.4.1）。
- 新增 6 个单元测试，含两个针对上述误报的回归测试。

## [0.1.0] — 2026-09-21

### 新增

- 首个版本：SSH 动态转发（`-D`）出口隧道、OpenSSH 检测与指纹核对。
- HTTP CONNECT → SOCKS5 桥接层（Codex 原生二进制只接受 `HTTP(S)_PROXY` 的
  CONNECT 语义，不接受 `socks5://` 直供）。
- 网络诊断：本地 / 隧道 / 服务器 / 出口 / DNS / IPv6 / 客户端路由。
- Mihomo / Clash Verge 受控集成：只读检测、规则片段生成（不自动写入）、
  备份与回滚。
- UI：设计系统、深/浅双主题、左侧边栏导航。
