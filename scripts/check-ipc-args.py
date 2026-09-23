"""IPC 参数名一致性检查：前端 `invoke` 传的键名 vs 后端 `#[tauri::command]` 参数名。

## 为什么需要这个脚本

Tauri 2 默认把 Rust 侧的 **snake_case 参数名转成 camelCase** 后从前端入参里取。
所以后端写 `socks_port: u16`，前端必须传 `{ socksPort: ... }`；传 `{ socks_port }`
会得到：

    invalid args `socksPort` for command `save_settings`:
    command save_settings missing required key socksPort

**这类错误两个测试层都抓不到**：
  - 前端测试用夹具替换了 `invoke`，根本不走 IPC 序列化
  - 后端测试直接调 Rust 函数，也不走 IPC 序列化
参数名写错时，两边都「通过」。这就是 v0.4.0 里 `save_settings` 带着这个 bug
发布出去的原因（用户点「保存端口设置」才发现）。

所以这个检查必须**静态扫源码**，而不是跑测试。

## 检查方式

1. 解析 `src-tauri/src/commands.rs`，取每个命令的参数名（排除注入类参数），
   按 Tauri 规则转成 camelCase —— 这是「前端应该传什么」的权威来源。
2. 解析前端所有 `invoke("cmd", X)`：
   - `X` 是对象字面量 → 直接取键名
   - `X` 是标识符（如 `payload`）→ 回溯其类型注解取键名
3. 比对：前端多传或错名 → 报错；前端漏传（后端必填但前端没给）→ 报错。

用法：
    python scripts/check-ipc-args.py          # 检查
    python scripts/check-ipc-args.py -v       # 打印每个命令的比对明细

退出码 1 表示发现不一致，可用于 CI / 发布前卡口。
"""
import argparse
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
COMMANDS_RS = ROOT / "src-tauri" / "src" / "commands.rs"
FRONTEND_DIRS = [ROOT / "src"]

# 这些参数由 Tauri 框架注入，不由前端传，比对时必须排除。
INJECTED_TYPES = ("AppHandle", "State<", "Window", "WebviewWindow", "Manager")


def to_camel(snake: str) -> str:
    """snake_case -> camelCase（Tauri 2 的默认参数名转换规则）。"""
    head, *rest = snake.split("_")
    return head + "".join(w[:1].upper() + w[1:] for w in rest)


def _split_top_level(s: str, single_quote: bool = True) -> list:
    """按顶层逗号分割，跳过字符串字面量与嵌套的括号/尖括号。

    不能简单 `s.split(",")`：`Option<String>`、`HashMap<K, V>` 里的逗号
    以及字符串里的逗号都不是分隔符。

    `single_quote=False` 用于 **Rust 源码**：Rust 的生命周期标注 `'_` / `'a`
    会被误认成单引号字符串的起点，导致后面所有逗号都落进「字符串内部」。
    这个坑真实踩过 —— 加了单引号处理后，后端参数全部解析成空列表。
    """
    parts, buf, depth, quote, escaped = [], "", 0, None, False
    for ch in s:
        if quote:
            buf += ch
            if escaped:
                escaped = False
            elif ch == "\\":
                escaped = True
            elif ch == quote:
                quote = None
            continue
        if ch == '"' or (single_quote and ch in "'`"):
            quote = ch
            buf += ch
            continue
        if ch in "([{<":
            depth += 1
            buf += ch
            continue
        if ch in ")]}>":
            depth -= 1
            buf += ch
            continue
        if ch == "," and depth == 0:
            parts.append(buf)
            buf = ""
            continue
        buf += ch
    parts.append(buf)
    return parts


def parse_backend_commands(src: str) -> dict:
    """从 commands.rs 提取 {命令名: [前端应传的 camelCase 参数名]}。"""
    commands = {}
    # 匹配 #[tauri::command] ... pub (async )?fn name( ... ) ->
    pattern = re.compile(
        r"#\[tauri::command\][\s\S]{0,200}?"
        r"pub\s+(?:async\s+)?fn\s+(\w+)\s*\(([\s\S]*?)\)\s*->",
        re.MULTILINE,
    )
    for m in pattern.finditer(src):
        name, params_blob = m.group(1), m.group(2)
        # 去掉行注释（参数列表里常有说明文字）
        cleaned = "\n".join(line.split("//")[0] for line in params_blob.split("\n"))
        params = []
        # 必须按**顶层逗号**切分而不是按行：`fn f(a: String, b: String)`
        # 是单行多参数，按行切会只拿到第一个。这个坑真实踩过。
        # single_quote=False：Rust 的 `'_` 生命周期不是字符串。
        for chunk in _split_top_level(cleaned, single_quote=False):
            chunk = chunk.strip()
            if not chunk:
                continue
            pm = re.match(r"^(\w+)\s*:\s*([\s\S]+)$", chunk)
            if not pm:
                continue
            pname, ptype = pm.group(1), pm.group(2)
            if any(t in ptype for t in INJECTED_TYPES):
                continue
            params.append(to_camel(pname))
        commands[name] = params
    return commands


def parse_object_keys(blob: str) -> list:
    """从对象字面量文本里提取顶层键名（含 `a, b` 简写形式）。

    **必须跳过字符串字面量**：`split(",")` 里的那个逗号不是键分隔符。
    这个坑真实踩过 —— 脚本因此误报 `generate_mihomo_fragment` 缺参数。
    """
    keys = []
    depth = 0
    token = ""
    quote = None
    escaped = False
    for ch in blob:
        if quote:
            token += ch
            if escaped:
                escaped = False
            elif ch == "\\":
                escaped = True
            elif ch == quote:
                quote = None
            continue
        if ch in "\"'`":
            quote = ch
            token += ch
            continue
        if ch in "{[(":
            depth += 1
            if depth > 1:
                token += ch
            continue
        if ch in "}])":
            depth -= 1
            if depth >= 1:
                token += ch
            continue
        if depth == 1 and ch == ",":
            _collect(token, keys)
            token = ""
            continue
        if depth == 1:
            token += ch
    _collect(token, keys)
    return keys


def _collect(token: str, keys: list) -> None:
    token = token.strip()
    if not token:
        return
    # `key: value` 或 `key`（简写）
    m = re.match(r"^([A-Za-z_$][\w$]*)\s*:", token)
    if m:
        keys.append(m.group(1))
        return
    m = re.match(r"^([A-Za-z_$][\w$]*)$", token)
    if m:
        keys.append(m.group(1))


def find_type_annotation_keys(files: dict, ident: str) -> list:
    """在前端源码里找 `ident: { ... }` 形式类型注解的键名。

    用于处理 `invoke("cmd", payload)` 这种「参数是个变量」的写法 ——
    v0.4.0 那个 bug 正是这种形态，只查字面量会漏掉它。
    """
    for path, src in files.items():
        # 形如  payload: {\n  a: number;\n  b: boolean;\n}
        m = re.search(
            rf"\b{re.escape(ident)}\s*:\s*\{{([\s\S]*?)\}}",
            src,
        )
        if not m:
            continue
        keys = []
        for line in m.group(1).split("\n"):
            line = line.strip().rstrip(";").rstrip(",")
            if not line or line.startswith("//"):
                continue
            km = re.match(r"^([A-Za-z_$][\w$]*)\s*[?]?\s*:", line)
            if km:
                keys.append(km.group(1))
        if keys:
            return keys
    return []


def read_frontend_files() -> dict:
    files = {}
    for d in FRONTEND_DIRS:
        for ext in ("*.ts", "*.vue"):
            for p in d.rglob(ext):
                if "dev" in p.parts:  # 夹具是假的 invoke，不参与比对
                    continue
                files[p] = p.read_text(encoding="utf-8")
    return files


def _find_matching_paren(src: str, open_idx: int) -> int:
    """从 `open_idx` 处的 `(` 开始，返回配对 `)` 的下标（跳过字符串与行注释）。

    不能用 `re` 的非贪婪 `\\(([\\s\\S]*?)\\)` 抓 invoke 的参数：参数里只要有
    函数调用（如 `.split(",")`），就会在**第一个** `)` 处提前截断。
    这个坑真实踩过 —— 脚本因此误报 `generate_mihomo_fragment` 缺参数。
    """
    depth = 0
    quote = None
    escaped = False
    i = open_idx
    while i < len(src):
        ch = src[i]
        if quote:
            if escaped:
                escaped = False
            elif ch == "\\":
                escaped = True
            elif ch == quote:
                quote = None
            i += 1
            continue
        if ch == "/" and i + 1 < len(src) and src[i + 1] == "/":
            j = src.find("\n", i)
            i = len(src) if j < 0 else j
            continue
        if ch in "\"'`":
            quote = ch
            i += 1
            continue
        if ch == "(":
            depth += 1
        elif ch == ")":
            depth -= 1
            if depth == 0:
                return i
        i += 1
    return -1


def parse_frontend_calls(files: dict) -> dict:
    """提取 {命令名: [(来源文件, [前端传的键名] 或 None)]}"""
    calls = {}
    for path, src in files.items():
        for m in re.finditer(r"invoke\s*(?:<[^>]*>)?\s*\(", src):
            open_idx = m.end() - 1
            close_idx = _find_matching_paren(src, open_idx)
            if close_idx < 0:
                continue
            inner = src[open_idx + 1 : close_idx].strip()

            # inner 形如: "cmd"   或   "cmd", {...}   或   "cmd", payload
            cm = re.match(r'^"([a-z_]+)"\s*(?:,([\s\S]*))?$', inner)
            if not cm:
                continue
            cmd = cm.group(1)
            arg = (cm.group(2) or "").strip()

            if not arg:
                keys = []
            elif arg.startswith("{"):
                # inner 已经是配对后的完整对象字面量
                keys = parse_object_keys(arg)
            elif re.match(r"^[A-Za-z_$][\w$]*$", arg):
                keys = find_type_annotation_keys(files, arg) or None
            else:
                keys = None
            calls.setdefault(cmd, []).append((path, keys))
    return calls


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("-v", "--verbose", action="store_true", help="打印每个命令的比对明细")
    args = ap.parse_args()

    if not COMMANDS_RS.exists():
        print(f"找不到 {COMMANDS_RS}", file=sys.stderr)
        return 1

    backend = parse_backend_commands(COMMANDS_RS.read_text(encoding="utf-8"))
    files = read_frontend_files()
    frontend = parse_frontend_calls(files)

    problems = []
    checked = 0
    unknown = []

    for cmd, occurrences in sorted(frontend.items()):
        if cmd not in backend:
            # 前端调了后端不存在的命令（拼写错误或忘了注册）
            for path, _ in occurrences:
                problems.append(f"{path.name}: 调用了未注册的命令 `{cmd}`")
            continue
        expected = backend[cmd]
        for path, keys in occurrences:
            if keys is None:
                unknown.append((cmd, path))
                continue
            checked += 1
            missing = [k for k in expected if k not in keys]
            extra = [k for k in keys if k not in expected]
            if missing:
                problems.append(
                    f"{path.name}: `{cmd}` 缺少参数 {missing}（前端传了 {keys}，"
                    f"后端要求 {expected}）"
                )
            elif extra:
                problems.append(
                    f"{path.name}: `{cmd}` 传了后端不认识的参数 {extra}"
                    f"（后端要求 {expected}）"
                )
            elif args.verbose:
                print(f"  ✓ {cmd} ← {path.name}  {keys}")

    print(f"\n已比对 {checked} 处 invoke 调用（后端共 {len(backend)} 个命令）")
    if unknown:
        print(f"无法静态解析的调用 {len(unknown)} 处（参数来自变量且找不到类型注解）：")
        for cmd, path in unknown:
            print(f"  ? {path.name}: {cmd}")

    if problems:
        print(f"\n发现 {len(problems)} 处不一致：")
        for p in problems:
            print(f"  ✗ {p}")
        print(
            "\n提示：Tauri 2 把后端 snake_case 参数名转成 camelCase 传给前端，"
            "前端必须用 camelCase 键名。"
        )
        return 1

    print("\n结论：IPC 参数名全部一致")
    return 0


if __name__ == "__main__":
    sys.exit(main())
