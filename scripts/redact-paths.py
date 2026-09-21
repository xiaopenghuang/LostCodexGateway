"""把仓库中残留的本机盘符路径替换为中性示意路径。

背景：项目开发初期在某台机器的非系统盘上，代码注释/测试夹具里留下了
大量该盘的绝对路径。这些不是密钥，但会暴露作者的真实磁盘布局，
公开前统一归一化到中性前缀 ``D:\\Tools\\...``。

替换是幂等的：已含 ``D:\\Tools\\...`` 的内容不会被再次改动。

注意：同一路径在不同语言里转义层数不同——
  Rust 原始字符串   r"X:\\SomeDir\\app.exe"
  JS 字符串         "X:\\\\SomeDir\\\\app.exe"
所以需要正则兜底，而不能只做固定串替换。

**源盘符不在本文件里写死**（本脚本随仓库公开）：从本地映射表读取
``.workbuddy-ai/redaction-map.local.json`` 的 ``source_prefixes`` 字段。
缺失时回落到 `G:`（历史默认值），仅影响兜底正则的匹配范围。
"""
import io
import json
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
LOCAL_MAP = os.path.join(ROOT, ".workbuddy-ai", "redaction-map.local.json")

FILES = [
    "src-tauri/src/diagnostics.rs",
    "src-tauri/src/mihomo.rs",
    "src/dev/fixtures.ts",
    "tests/e2e/cdp_m2_e2e.mjs",
    "CHANGELOG.md",
]

# 源盘符前缀：优先从本地映射表读取，缺失时用历史默认值
SRC = "G:"
SUBDIRS = ["VSCODE" + "\\" * 2 + "nodejs", "VSCODE", "Programs" + "\\" * 2,
           "Clash Verge", "Git" + "\\" + "Git", "MVS" + "\\" + "BuildTools",
           "Practical-tools"]
if os.path.exists(LOCAL_MAP):
    try:
        _m = json.load(io.open(LOCAL_MAP, encoding="utf-8"))
        SRC = _m.get("source_drive", SRC)
        SUBDIRS = _m.get("source_subdirs", SUBDIRS)
    except (OSError, ValueError):
        pass

# 顺序重要：先替换更长的前缀，否则短前缀会先命中并留下残尾
PAIRS = [(SRC + "\\" * 2 + d, "D:" + "\\" * 2 + "Tools" + "\\" * 2 + d)
         for d in SUBDIRS if "\\" * 2 in d]
PAIRS += [(SRC + "\\" + d, "D:" + "\\" + "Tools" + "\\" + d)
          for d in SUBDIRS if "\\" * 2 not in d]

# 兜底：任意转义层数的源盘符前缀统一改到 D:\Tools（只处理盘符本身，不动后续路径）
GENERIC = re.compile(re.escape(SRC) + r"(\\+)(?!Tools)", re.IGNORECASE)


def redact(text: str) -> str:
    out = text
    for old, new in PAIRS:
        out = out.replace(old, new)
    # 把「G: + 连续反斜杠」整体换成「D:\Tools + 同样的反斜杠数」，
    # 避免二次拼接出错（如 G:\\Programs 已被上面处理，这里只兜剩余的）
    out = GENERIC.sub(lambda m: "D:" + m.group(1) + "Tools" + m.group(1), out)
    return out


changed = 0
for path in FILES:
    try:
        src = io.open(path, encoding="utf-8").read()
    except FileNotFoundError:
        print("跳过（不存在）:", path)
        continue
    out = redact(src)
    if out != src:
        io.open(path, "w", encoding="utf-8", newline="").write(out)
        print("已改写:", path)
        changed += 1
    else:
        print("未变  :", path)

print(f"\n共改写 {changed} 个文件")

# 自检：确认无源盘符残留
leftover = []
for path in FILES:
    try:
        s = io.open(path, encoding="utf-8").read()
    except FileNotFoundError:
        continue
    for m in re.finditer(re.escape(SRC) + r"\\+", s):
        line = s[: m.start()].count("\n") + 1
        leftover.append(f"{path}:{line}: {m.group(0)}")
if leftover:
    print("\n仍有残留:")
    for x in leftover:
        print(" ", x)
    sys.exit(1)
print(f"无 {SRC} 盘残留")

