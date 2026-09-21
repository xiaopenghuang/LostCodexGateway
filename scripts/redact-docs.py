"""公开前文档脱敏（幂等）。

处理三类内容：
  1. 真实出口 IP   -> RFC 5737 文档保留段（一对一映射，保持相对关系）
  2. 本机盘符路径  -> D:\\Tools\\... 示意路径
  3. 真实 SSH 端点 -> 移除，改为「实测环境」措辞

同时给验收报告补上「IP 已脱敏」的声明。
可重复运行：已处理的文件不会被二次改动。

**为什么真实值不在这个脚本里**：本脚本随仓库公开，若把真实 IP / 端点
写死在源码里，等于把刚脱敏掉的值又发布一次。因此真实值放在本地映射表
`.workbuddy-ai/redaction-map.local.json`（已在 .gitignore 中），
缺失时脚本会跳过「地址替换」但仍执行路径归一化与声明补全。

本地映射表格式：
    {
      "ip":   [["<真实 IP>", "<文档段 IP>"], ...],
      "host": [["<真实 host:port>", "实测环境"], ["<真实 host>", "实测环境"]]
    }
"""
import io
import json
import os

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
LOCAL_MAP = os.path.join(ROOT, ".workbuddy-ai", "redaction-map.local.json")

# 1) 真实 IP -> 文档段 IP（一对一，保证「相同/不同」的结论仍成立）
#    真实值从本地映射表加载
IP_MAP: list[tuple[str, str]] = []
# 3) 真实 SSH 端点（IP + 非标准端口）-> 泛化措辞
HOST_MAP: list[tuple[str, str]] = []

if os.path.exists(LOCAL_MAP):
    try:
        _m = json.load(io.open(LOCAL_MAP, encoding="utf-8"))
        IP_MAP = [tuple(x) for x in _m.get("ip", [])]
        HOST_MAP = [tuple(x) for x in _m.get("host", [])]
        print(f"已载入本地映射表：{len(IP_MAP)} 条 IP、{len(HOST_MAP)} 条端点")
    except (OSError, ValueError) as e:
        print(f"⚠ 本地映射表读取失败（{e}），本次跳过地址替换")
else:
    print(f"⚠ 未找到本地映射表 {LOCAL_MAP}，本次跳过地址替换")
    print("  （地址替换已完成过一次，通常无需重跑；如需重放请先恢复该文件）")

# 2) 本机盘符路径 -> 示意路径（顺序：长前缀优先）
#    这类路径不含个人身份信息，但会泄露作者磁盘布局，故一并归一化。
PATH_MAP = [
    ("G:" + "\\" * 2 + "VSCODE" + "\\" * 2 + "nodejs", "D:" + "\\" * 2 + "Tools" + "\\" * 2 + "nodejs"),
    ("G:" + "\\" * 2 + "VSCODE", "D:" + "\\" * 2 + "Tools"),
    ("G:" + "\\" * 2 + "Programs" + "\\" * 2, "D:" + "\\" * 2 + "Tools" + "\\" * 2),
    ("G:" + "\\" * 2 + "Clash Verge", "D:" + "\\" * 2 + "Clash Verge"),
    ("G:" + "\\" + "Git" + "\\" + "Git", "D:" + "\\" + "Tools" + "\\" + "Git"),
    ("G:" + "\\" + "VSCODE", "D:" + "\\" + "Tools"),
    ("G:" + "\\" + "Clash Verge", "D:" + "\\" + "Clash Verge"),
    ("G:" + "\\" + "Programs", "D:" + "\\" + "Tools"),
    ("G:" + "\\" + "MVS" + "\\" + "BuildTools", "D:" + "\\" + "Tools" + "\\" + "BuildTools"),
    ("G:" + "\\" + "Practical-tools", "D:" + "\\" + "Tools"),
]

# 3) 真实 SSH 端点的定义已上移至映射表加载处（HOST_MAP）

DOCS = [
    "docs/acceptance-report.md",
    "docs/m0-environment-report.md",
    "docs/risks-and-unimplemented.md",
]

ACCEPTANCE_HEADER = """# LostCodexGateway — 验收与测试记录

> 按里程碑记录实测结果。每项都有命令/日志摘录可复现。状态：✅通过 / ⚠️部分 / ❌失败（附原因）
>
> **关于 IP 地址**：本记录中的公网 IP 已替换为 [RFC 5737](https://www.rfc-editor.org/rfc/rfc5737)
> 文档保留段（`203.0.113.0/24`、`198.51.100.0/24`）以脱敏真实端点。
> 替换为**一对一映射**，故「两值相同/不同」的对比结论与相对关系仍然成立；
> 被验证的是测试方法与判定逻辑，具体数值不构成证据。
"""

M0_HEADER_MARK = "## 1. 检测摘要"
M0_NOTE = """> 本报告未修改任何系统/服务器配置。
>
> **关于路径与地址**：本报告公开版本已将本机真实盘符路径替换为示意路径
> （如 `D:\\Tools\\...`），公网 IP 替换为 [RFC 5737](https://www.rfc-editor.org/rfc/rfc5737)
> 文档保留段。**结论、判定逻辑与技术依据未作任何改动**——被保留下来的是
> 「测到了什么、因此如何决策」，具体盘符与数值不构成设计依据。
> 原始机器盘点数据（含真实用户名/盘符/已装软件/进程）保留在本地，不入库。

"""

total = 0
for path in DOCS:
    try:
        s = io.open(path, encoding="utf-8").read()
    except FileNotFoundError:
        print("跳过（不存在）:", path)
        continue
    orig = s

    for a, b in IP_MAP:
        s = s.replace(a, b)
    for a, b in PATH_MAP:
        s = s.replace(a, b)
    for a, b in HOST_MAP:
        s = s.replace(a, b)

    # 验收报告：补 IP 脱敏声明（若缺失）
    if path.endswith("acceptance-report.md"):
        marker = "\n## M1："
        if "关于 IP 地址" not in s and marker in s:
            head_end = s.index(marker)
            s = ACCEPTANCE_HEADER + s[head_end:]

    # M0 报告：补脱敏说明（放在标题引言之后、第一章之前）
    if path.endswith("m0-environment-report.md"):
        if "关于路径与地址" not in s and M0_HEADER_MARK in s:
            # 替换原有的单行引言
            s = s.replace(
                "> 本报告未修改任何系统/服务器配置。原始机器数据见 `docs/m0-environment-report.json`。\n",
                "",
            ).replace(
                "> 本报告未修改任何系统/服务器配置。\n",
                "",
            )
            i = s.index(M0_HEADER_MARK)
            s = s[:i] + M0_NOTE + s[i:]

    if s != orig:
        io.open(path, "w", encoding="utf-8", newline="").write(s)
        print("已脱敏:", path)
        total += 1
    else:
        print("无需改动:", path)

print(f"\n共处理 {total} 个文件")
