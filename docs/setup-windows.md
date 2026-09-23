# Windows 配置指南

## 0. 环境自检（推荐第一步）

**新电脑装好本软件后，先到「环境自检」页点「重新体检」。**

它会逐项检查跑通所需的全部前置条件，并明确告诉你每一项怎么补：

| 检查项 | 能自动做 | 需要你动手 |
|---|---|---|
| 系统 OpenSSH | 检测 `ssh.exe` 路径与版本 | 缺失时给出 `Add-WindowsCapability` 步骤（需管理员） |
| SSH 私钥 | 检查文件是否存在 | 生成密钥、把公钥放到服务器（`ssh-keygen` + 粘贴） |
| 服务器连通性 | DNS 解析 + TCP 建连（5s 超时） | 端口不通时排查防火墙 / 安全组 |
| Host Key | 检测 `known_hosts` 是否已有条目 | **指纹必须你亲自核对一次**（安全设计，不可自动化） |
| 本地端口 | 尝试 bind 判断占用 | 被占用时跳到「设置」页改端口 |
| Codex CLI | 定位 `codex.exe` / `codex.cmd` | 未安装时给出 `npm i -g @openai/codex` 步骤 |
| Clash Verge | 检测安装 / 进程 / 混入端口 / TUN | 仅 Desktop/IDE 场景需要；给出开 TUN 与导入片段的步骤 |

**本页全程只读**：不安装程序、不改注册表或系统代理、不改你的 Clash 配置、不提权。
凡是涉及「需要你决策」或「需要在服务器上动手」的环节，一律给步骤由你亲手完成。

> 设计原则：**能自动检测的一律自动检测；不能自动修的不假装能修**。
> 界面上不会出现「报红但没说怎么办」的项——后端有测试（`preflight::tests`）兜住这条约束。

---

## 1. 首次配置（基础网关模式）

1. 安装并打开 LostCodexGateway。
2. 「服务器」页填写：
   - 目标服务器名称（可选，如「我的 VPS」）
   - 主机地址：你的 Ubuntu 服务器 IP 或域名
   - SSH 端口：默认 22（你的服务器实际端口）
   - 用户名：如 `ubuntu`
   - 本地 SOCKS 端口：默认 17801（被占用会明确提示，换一个即可）
   - 系统 ssh.exe 路径：留空则自动使用 `C:\Windows\System32\OpenSSH\ssh.exe`（推荐）
   - SSH 私钥路径：**只保存路径字符串，应用不读取、不复制、不导出私钥内容**
3. 点「保存配置」→「测试连接」：确认 SSH 认证通过。
4. 「查询服务器指纹」：首次连接必须人工核对指纹（与服务器管理员给的比对）→「我已核对，确认写入」。
   - 写入前应用自动备份 `~/.ssh/known_hosts`（`known_hosts.bak_时间戳`）。
   - 之后若指纹变化，连接会被阻断并告警，**不会**自动删除旧条目。
5. 回到「首页」点「连接」：状态依次为 连接中 → 隧道已通 → 出口已验证。
   - 「出口已验证」只表示隧道可用且出口 IP 实测成功；某应用是否走了网关见「应用」页证据。

## 2. 前置条件（服务器侧）

- 服务器 sshd 必须允许 TCP 转发：`sshd_config` 中 `AllowTcpForwarding yes`（Ubuntu 默认允许）。
- 若被禁用，本工具会明确报「远端禁止 TCP 转发」，**不会自动修改你的 `/etc/ssh/sshd_config`**。
- 服务器现有业务（Docker、Nginx、CPA、SSH）不受影响：本工具只在服务器上建立普通 SSH 会话做动态转发，不装任何服务。

## 3. Codex CLI 使用

- 连接并达到「出口已验证」后，「应用」页 →「预览将注入的设置」→「从网关启动 Codex CLI」。
- 弹出独立 PowerShell 终端，仅该终端注入：
  `HTTP_PROXY/HTTPS_PROXY=http://127.0.0.1:<桥接端口>`、`NO_PROXY=localhost,127.0.0.1,::1`
- OAuth 登录的 localhost 回调走直连（NO_PROXY 保证），不会经隧道转发。
- 隧道断开时启动按钮禁用并明确提示——不会悄悄直连后假装已保护。
- 实测说明：Codex CLI 0.154.0（Rust 原生二进制）需要 HTTP CONNECT 语义，因此自动启用回环桥接层；该桥接只做 CONNECT 转发，不解密 HTTPS。

## 4. Mihomo / Clash Verge 受控集成（Desktop / IDE 场景，高级）

当前设计（M3）与实测环境（Verge Rev 2.5.2）：

1. 「应用」页 →「检测 Mihomo / Clash Verge」：只读显示版本、进程、mixed-port、external-controller、TUN 状态。
2. **TUN 未开启时**：应用如实显示「Desktop/IDE 尚未覆盖」。开启 TUN 是 Mihomo/Verge 官方组件的高级功能，本工具不代办权限。
3. **进程发现后**（Desktop 正在运行时）：只把**已确认的 Codex 进程**填入片段，通用进程（Code.exe、node.exe、ssh.exe）不会自动加入。
4. 生成规则片段 → 复制 → 在 Verge 的 Profiles/扩展配置中手动导入（推荐用 Verge 的 merge/rules profile 入口，**不要直接改在线订阅文件**）。
5. **⚠️ 关键：`MY-VPS` 组的默认选项不要设成隧道出口。**
   - 规则是「无条件绑定」（`PROCESS-NAME,codex.exe,MY-VPS`），只要组默认选中 `lcfg-gateway`，codex 的生死就被绑在网关的在线状态上：网关未连接时 `127.0.0.1:<SOCKS 端口>` 无人监听，每个连接**被立即拒绝** → codex **立刻重试** → 表现为「一直重连、发不出去请求」。
   - 实测日志（`logs/sidecar/sidecar_latest.log`）：
     `dial MY-VPS (match ProcessName/codex.exe) ... connectex: ... actively refused it`
   - **正确写法**：默认选中你的机场主组，把隧道出口作为**手动切换**的可选项。示例：
     ```yaml
     - name: MY-VPS
       type: select
       proxies:
         - 雪山 Link        # 默认：机场主组，保证 codex 随时可用
         - lcfg-gateway     # 需要服务器转发时手动切到这里
         - DIRECT
       default-selected: 雪山 Link
     ```
   - 这样既满足「特定情况下用自己的服务器转发」的目的，又不会影响其他节点的正常使用。
6. 导入前先用「备份」按钮备份目标文件；回滚用「回滚」按钮（只恢复本工具创建的备份；若检测到文件被用户改过，拒绝覆盖并提示人工处理）。
7. 验证：在 Verge 连接面板确认 Codex 进程的连接命中你的 `MY-VPS` 组，且该组指向的是当前期望的出口；未命中即「部分验证」，不声明全部成功。

> 订阅更新后网关规则可能丢失（取决于你的 profile 链）。本工具 v1 不做订阅注入，规则丢失后重新导入片段即可。相关冲突处理见 troubleshooting.md。
>
> **注意规则的「默认选中」比规则本身更容易造成故障**：规则只决定「codex 走 MY-VPS 组」，而组默认选中谁决定「能不能通」。默认选中隧道出口 = 不用网关时 codex 全废（详见本节第 5 条与 troubleshooting.md「Mihomo 规则类」）。

## 5. 卸载与还原

- 应用内「断开」：仅停止本工具创建的 ssh.exe 进程与桥接层；不触碰你的其他 SSH 会话、系统代理、Mihomo 配置。
- 卸载程序（NSIS）：删除程序文件；配置目录 `%APPDATA%\LostCodexGateway\`（config.json 及其备份、诊断报告）保留，可手动删除。
- 若曾手动导入过 Mihomo 规则片段：先点「回滚」或手动移除片段行，再卸载。
