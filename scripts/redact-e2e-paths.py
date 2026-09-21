#!/usr/bin/env python3
"""把 tests/e2e/*.mjs 里写死的本机绝对路径改为从脚本位置推导。

背景：这些 CDP 端到端脚本是开发期间在真实工作区里写的，私钥路径与截图输出目录
都被写成了绝对路径（形如 `I:\\开发\\LostCodexGateway\\...`）。这些路径只在作者
那台机器上成立，既不可移植，也把本地目录结构泄进了仓库。

做法：在读文件/写文件前注入一段基于 `import.meta.url` 的路径推导，并把绝对路径
替换为推导出的变量。脚本是可重复运行的（幂等）：已经改过的文件不会被二次改写。

用法：
    python scripts/redact-e2e-paths.py [--check]
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
E2E = ROOT / "tests" / "e2e"

# 注入头：放在 import 语句之后，提供 __dirname / REPO_ROOT / FIXTURE_KEY / SHOT_DIR
PRELUDE = '''
// --- 路径推导（由 scripts/redact-e2e-paths.py 注入，勿手改）---
// 脚本可能被从任意工作目录调用，所以路径一律相对本文件解析。
import { fileURLToPath } from "node:url";
import { dirname, resolve as resolvePath, join as joinPath } from "node:path";

const __dirname = dirname(fileURLToPath(import.meta.url));
/** 仓库根目录（tests/e2e → 仓库根需要上溯两级）。 */
const REPO_ROOT = resolvePath(__dirname, "..", "..");
/** Docker 夹具用的测试私钥（仅测试用途，不含任何真实凭据）。 */
const FIXTURE_KEY = joinPath(REPO_ROOT, "tests", "fixtures", "ssh-server", "keys", "id_test_ed25519");
/** 截图输出目录。 */
const SHOT_DIR = joinPath(REPO_ROOT, "docs", "screenshots");
// --- 路径推导结束 ---
'''

PRELUDE_MARKER = "路径推导（由 scripts/redact-e2e-paths.py 注入"

# 写死路径 → 替换表达式
# 注意匹配顺序：先长后短，避免把 `I:\\开发\\LostCodexGateway` 部分替换后残留。
REPLACEMENTS: list[tuple[re.Pattern[str], str]] = [
    # 4 层转义的 JS 字符串字面量（源码里写成 \\\\，即运行时为 \\）
    (
        re.compile(
            r'"I:\\\\+开发\\\\+LostCodexGateway\\\\+tests\\\\+fixtures\\\\+ssh-server\\\\+keys\\\\+id_test_ed25519"'
        ),
        "FIXTURE_KEY",
    ),
    (
        re.compile(r'"I:\\\\+开发\\\\+LostCodexGateway\\\\+docs\\\\+screenshots"'),
        "SHOT_DIR",
    ),
    # 2 层转义（运行时为单个反斜杠）——截图输出的 string + "\\file.png" 形式
    (
        re.compile(r'"I:\\\\开发\\\\LostCodexGateway\\\\docs\\\\screenshots"'),
        "SHOT_DIR",
    ),
    # 裸字面量形式的兜底（单层/无转义）
    (
        re.compile(r'"I:\\(?:开发)\\LostCodexGateway\\tests\\fixtures\\ssh-server\\keys\\id_test_ed25519"'),
        "FIXTURE_KEY",
    ),
    (
        re.compile(r'"I:\\(?:开发)\\LostCodexGateway\\docs\\screenshots"'),
        "SHOT_DIR",
    ),
    # 真实的 node 全局安装路径（与项目无关的本机环境细节）
    (
        re.compile(
            r'"D:\\\\+Tools\\\\+nodejs\\\\+node_global\\\\+node_modules\\\\+@openai\\\\+codex\\\\+bin\\\\+codex\.exe"'
        ),
        'joinPath(REPO_ROOT, "codex.exe")',
    ),
]

# node:path / node:url 的 import 若已存在，不重复注入
IMPORT_ANCHOR = re.compile(r"^(import .+?;\s*)$", re.MULTILINE)


def needs_import(src: str, stmt: str) -> bool:
    return stmt not in src


def inject_prelude(src: str) -> str:
    """把 PRELUDE 插到最后一个顶层 import 之后。"""
    if PRELUDE_MARKER in src:
        return src
    matches = list(IMPORT_ANCHOR.finditer(src))
    if not matches:
        # 没有 import 语句，插到文件最前面（保留可能的 shebang / 首行注释块之后的第一行断言）
        return src + "\n" + PRELUDE
    last = matches[-1]
    return src[: last.end()] + "\n" + PRELUDE + src[last.end() :]


def transform(path: Path, check: bool) -> tuple[bool, list[str]]:
    src = path.read_text(encoding="utf-8")
    original = src
    hits: list[str] = []

    for pattern, replacement in REPLACEMENTS:
        def _sub(m: re.Match[str]) -> str:
            hits.append(m.group(0)[:70])
            return replacement

        src = pattern.sub(_sub, src)

    if hits:
        src = inject_prelude(src)

    if src == original:
        return False, []

    if not check:
        path.write_text(src, encoding="utf-8", newline="\n")
    return True, hits


def main() -> int:
    ap = argparse.ArgumentParser(description="E2E 脚本绝对路径脱敏")
    ap.add_argument("--check", action="store_true", help="只检查，不写回")
    args = ap.parse_args()

    if not E2E.is_dir():
        print(f"目录不存在: {E2E}", file=sys.stderr)
        return 2

    changed = 0
    total_hits = 0
    for path in sorted(E2E.glob("*.mjs")):
        did, hits = transform(path, args.check)
        if did:
            changed += 1
            total_hits += len(hits)
            rel = path.relative_to(ROOT)
            verb = "需要修改" if args.check else "已改写"
            print(f"  {verb}: {rel}  ({len(hits)} 处)")
            for h in hits:
                print(f"      ← {h}")
        else:
            rel = path.relative_to(ROOT)
            print(f"  无需改动: {rel}")

    print(f"\n{'待修改' if args.check else '已改写'} {changed} 个文件，共 {total_hits} 处写死路径")
    if args.check and changed:
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
