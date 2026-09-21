# 公开发布前的隐私与安全审查

> 审查日期：2026-09-21｜审查范围：`git ls-files` 全部已跟踪文件（136 个，其中文本 66 个）
> 触发原因：首次公开到 GitHub 前，确认工作区不含作者本机/个人/服务器敏感信息。
> 复查方式：`python scripts/privacy-scan.py`（退出码 0 = 无高风险项）

## 结论

**当前仓库可以公开。** 未发现凭据、私钥、口令或真实服务器端点。
审查中发现并处理的 7 类问题见下。

## 一、已处理的问题

| # | 问题 | 位置 | 处理 |
|---|---|---|---|
| 1 | **真实 VPS SSH 端点**（host:port 形式的真实服务器地址），且紧邻真实主机公钥与指纹 | `src-tauri/src/ssh.rs`（2 处） | 端点描述改为「实测环境」；主机公钥/指纹替换为合成测试向量（见下） |
| 2 | **三个真实出口 IP**：本机/网关出口、服务器直连出口、现有代理出口（**原值不在此记录**，见「脱敏方法」§1） | `docs/acceptance-report.md`（13 处）、`docs/risks-and-unimplemented.md`（1 处）、`docs/m0-environment-report.md`（1 处） | 替换为 RFC 5737 文档段，并在文档开头声明「已脱敏、一对一映射」 |
| 3 | **整机环境快照**：真实用户名、盘符布局、已装软件及版本、运行中进程 PID、残留服务 | `docs/m0-environment-report.json` | 移出仓库（保留在本地 `.workbuddy-ai/`），并加入 `.gitignore` |
| 4 | **M0 报告中的本机盘符路径** | `docs/m0-environment-report.md`（8 处） | 替换为示意路径 `D:\Tools\...`，保留全部结论与判定逻辑 |
| 5 | **代码注释/测试夹具中的盘符路径**（暴露作者磁盘布局） | `diagnostics.rs`、`mihomo.rs`、`fixtures.ts`、`cdp_m2_e2e.mjs`、`CHANGELOG.md` | 统一归一化为 `D:\Tools\...`（脚本：`scripts/redact-paths.py`） |
| 6 | **过时且含真实数据的截图**：仍是 0.1.0 顶栏旧 UI，且显示真实出口 IP 与 DNS | `docs/screenshots/network-diagnostics.png` | 删除（目录已空），后续用 0.2.0 侧边栏 UI 重新截图 |
| 7 | **CDP 端到端脚本里的写死绝对路径**：私钥路径与截图输出目录被写成 `I:\开发\LostCodexGateway\...`（暴露作者工作区目录名与盘符）；另有一处真实的本机 node 全局安装路径 `D:\Tools\nodejs\node_global\...\codex.exe` | `tests/e2e/` 下 5 个 `.mjs`（共 6 处） | 改为基于 `import.meta.url` 推导（`REPO_ROOT` / `FIXTURE_KEY` / `SHOT_DIR`）；codex 可执行文件改从 `CODEX_EXE` 环境变量取，未设置时回落 PATH（脚本：`scripts/redact-e2e-paths.py`） |

## 二、判定为安全、未处理的项目

| 项目 | 判定依据 |
|---|---|
| `SHA256:ungWv48Bz+pBQUDeXa4iI7ADYaOWF3qctBD/YfIAFa0` | 是 `SHA256("abc")` 的结果，OpenSSH 指纹算法的**标准自检向量**，公开可查 |
| `SHA256:ZkAslGjFiUHdGf/WUL8rQvkib4PTvQatUV0OUQSncCA` | 本仓库**合成**密钥（32 字节递增序列 `00..1f` 包装成 ed25519 wire blob）的指纹，不对应任何真实主机 |
| `SHA256:uNiVztksCsDhcc0u9e8BujQXVUpKZIDTMczCvj3tD2s` | GitHub **官方文档公开**的 ed25519 主机密钥指纹 |
| `100.64.0.0/10`、`169.254.0.0/16`、`127.0.0.0/8`、`172.32.0.1` | `bridge.rs` 的私有网段拦截逻辑与其测试用例，属安全功能本身 |
| `5.15.153.1-microsoft-standard-WSL2` | WSL2 **内核版本字符串**，形似 IP 但非地址 |
| `127.0.0.1:17801`、`127.0.0.1:9097`、`127.0.0.1:18999` 等 | 回环地址，测试断言中的固定端口 |
| `1.1.1.1`、`8.8.8.8`、`142.250.72.14` | 公共 DNS 及其解析示例 |
| `C:\Users\you\.ssh\id_ed25519` | 输入框**占位符**，非真实路径 |
| `203.0.113.x`、`198.51.100.x`、`1.2.3.4`、`5.6.7.8` | 文档保留段与惯例示例地址 |
| `tests/fixtures/ssh-server/` | 夹具密钥由 `setup.ps1` 现场生成，`.gitignore` 用 `**/keys/` 通配屏蔽（注释中记录了一次嵌套路径险些提交私钥的事故） |
| `.workbuddy-ai/` | 已在 `.gitignore` 中，含工作笔记与决策过程，不入库 |

## 三、脱敏方法说明（为什么结论仍然可信）

1. **一对一映射**：三个真实出口 IP 各自映射到一个固定的 RFC 5737 文档段地址
   （映射表存于本地 `.workbuddy-ai/`，**不入库** —— 否则等于把被脱敏的值又发一遍）。
   因此验收记录中「本地出口与服务器出口**相同**」「对照出口**不同**」这类
   **相对关系**依然成立，被验证的是判定逻辑而非具体数值。
2. **指纹测试仍锁定真实算法语义**：合成向量走的是与真实主机密钥**完全相同**的代码路径
   （base64 解码 → SHA256(wire blob) → 无 padding base64）。测试价值未损失，
   它钉住的是「不得退回 `SHA256(keytype||0x20||pubkey)` 的错误实现」。
   已交叉校验：Python 独立实现与 Rust 实现输出一致。
3. **M0 报告保留全部技术依据**：被替换的只有盘符与地址，`结论 / 假设偏差 / 适配决策`
   三张表未改动一个字。

## 四、复查清单（每次公开发布前执行）

```bash
python scripts/privacy-scan.py            # 已跟踪文件；退出码 0 即通过
python scripts/privacy-scan.py --all      # 含未跟踪文件，更严格
python scripts/redact-e2e-paths.py --check  # E2E 脚本是否又混入绝对路径
```

若新增了含真实环境数据的文档，**不要直接提交**：
改为本地保留 + 提交脱敏版，或在 `.gitignore` 中屏蔽原文件。

### 未纳入自动扫描的残余项（人工确认，可接受）

`privacy-scan.py` 目前会对 `D:\Tools`、`D:\Clash Verge`、`E:\a b` 这类**示意路径**报 `low`。
这些字符串出现在以下位置，均已人工确认无泄露：

- 注释与文档中解释「为什么不能硬编码安装路径」的举例（`mihomo.rs`、`CHANGELOG.md`）；
- 单元测试里构造 `DisplayIcon` 字符串的用例输入（`mihomo.rs`、`fixtures.ts`）——
  它们必须长得像真实注册表值，否则测不出「剥图标索引再去引号」的顺序问题；
- **脱敏脚本自身**（`scripts/*.py`）——它们必然要提到目标路径前缀。

> 曾含真实本机路径的 `D:\Tools\nodejs\node_global\...\codex.exe` 已随第 7 项处理。

### 脱敏工具的设计约束（真实值不入库）

脱敏脚本随仓库公开，因此**真实值一律不写死在脚本里**：

| 数据 | 存放位置 |
|---|---|
| 真实出口 IP、真实 SSH 端点 | `.workbuddy-ai/redaction-map.local.json`（已 gitignore） |
| 原始源盘符与前缀（`redact-paths.py`） | 同上 |
| E2E 脚本的真实绝对路径前缀（`redact-e2e-paths.py`） | 同上 |

脚本在映射表缺失时会**跳过地址替换**并给出提示，但仍执行路径归一化与声明补全，
因此不会因为拿不到真实值而误改内容。这一点是刻意的：如果把映射表写进脚本，
「脱敏」就变成了「把被脱敏的值再发布一次」。
