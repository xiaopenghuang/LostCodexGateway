"""把仓库中残留的本机盘符路径替换为中性示意路径。

背景：项目开发在本机 G 盘，代码注释/测试夹具里留下了大量 ``G:\\...``。
这些不是密钥，但会暴露作者的真实磁盘布局，公开前统一归一化。

替换是幂等的：已含 ``D:\\Tools\\...`` 的内容不会被再次改动。

注意：同一路径在不同语言里转义层数不同——
  Rust 原始字符串   r"G:\\VSCODE\\Code.exe"
  JS 字符串         "G:\\\\VSCODE\\\\nodejs\\\\..."
所以需要正则兜底，而不能只做固定串替换。
"""
import io
import re
import sys

FILES = [
    "src-tauri/src/diagnostics.rs",
    "src-tauri/src/mihomo.rs",
    "src/dev/fixtures.ts",
    "tests/e2e/cdp_m2_e2e.mjs",
    "CHANGELOG.md",
]

# 顺序重要：先替换更长的前缀，否则短前缀会先命中并留下残尾
PAIRS = [
    ("G:" + "\\" * 2 + "VSCODE" + "\\" * 2 + "nodejs", "D:" + "\\" * 2 + "Tools" + "\\" * 2 + "nodejs"),
    ("G:" + "\\" * 2 + "VSCODE", "D:" + "\\" * 2 + "Tools"),
    ("G:" + "\\" * 2 + "Programs" + "\\" * 2, "D:" + "\\" * 2 + "Tools" + "\\" * 2),
    ("G:" + "\\" * 2 + "Clash Verge", "D:" + "\\" * 2 + "Clash Verge"),
    ("G:" + "\\" + "VSCODE", "D:" + "\\" + "Tools"),
    ("G:" + "\\" + "Clash Verge", "D:" + "\\" + "Clash Verge"),
    ("G:" + "\\" + "Programs", "D:" + "\\" + "Tools"),
]

# 兜底：任意转义层数的 G:\ 前缀统一改到 D:\Tools（只处理盘符本身，不动后续路径）
GENERIC = re.compile(r"G:(\\+)(?!Tools)", re.IGNORECASE)


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

# 自检：确认无 G: 盘残留
leftover = []
for path in FILES:
    try:
        s = io.open(path, encoding="utf-8").read()
    except FileNotFoundError:
        continue
    for m in re.finditer(r"G:\\+", s):
        line = s[: m.start()].count("\n") + 1
        leftover.append(f"{path}:{line}: {m.group(0)}")
if leftover:
    print("\n仍有残留:")
    for x in leftover:
        print(" ", x)
    sys.exit(1)
print("无 G: 盘残留")

