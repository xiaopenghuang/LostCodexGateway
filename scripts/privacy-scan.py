"""公开前隐私扫描：检查仓库中是否残留本机/个人/敏感信息。

用法：
    python scripts/privacy-scan.py            # 扫描 git 已跟踪文件
    python scripts/privacy-scan.py --all       # 扫描工作区全部文件（含未跟踪）

退出码 1 表示发现可疑项，可用于 CI 或发布前卡口。
"""
import argparse
import re
import subprocess
import sys

# (名称, 正则, 严重级, 说明)
RULES = [
    ("REAL_DOC_IP", re.compile(r"\b(203\.0\.113|198\.51\.100|192\.0\.2)\.\d+\b"), "info",
     "RFC 5737 文档保留地址（预期存在，仅提示）"),
    ("PRIVATE_IP", re.compile(r"\b(10\.\d{1,3}|192\.168\.\d{1,3}|172\.(1[6-9]|2\d|3[01])\.)\d{1,3}\.\d{1,3}\b"), "info",
     "私有网段（测试夹具中常见，确认非真实内网）"),
    ("NON_SYSTEM_DRIVE", re.compile(r"(?<![A-Za-z0-9])(?!C:)[D-Z]:\\{1,2}", re.IGNORECASE), "low",
     "非系统盘路径（示意路径应为 D:\\Tools，其余需确认）"),
    ("WINDOWS_USER_PROFILE", re.compile(r"C:\\Users\\(?!Public|Default|<|xxx|me\b|user\b|you\b|%|Your|name\b)[^\\\s\"']+"), "high",
     "真实用户目录（应使用 me / you / <user> / %USERNAME% 等占位）"),
    # 公网 IP：排除保留段、文档段、公共 DNS，以及 WSL 内核版本号这种
    # 「形似 IP 实为版本字符串」（如 5.15.153.1-microsoft-standard-WSL2）
    ("PUBLIC_ENDPOINT", re.compile(
        r"(?<![\w.])"
        r"(?!(?:127\.0\.0\.1|0\.0\.0\.0|255\.255\.255\.255|1\.1\.1\.1|8\.8\.8\.8|9\.9\.9\.9|208\.67\.222|208\.67\.220)"
        r"(?::\d+)?(?![\d.]))"
        r"(?!(?:10|192\.168|172\.(?:1[6-9]|2\d|3[01]))\.)"
        r"(?!(?:203\.0\.113|198\.51\.100|192\.0\.2)\.)"
        r"(?!1\.2\.3\.4|5\.6\.7\.8)\b"
        r"((?:\d{1,3}\.){3}\d{1,3})"
        r"(?![-.\w]*microsoft|[-.]?\d[\d.]*-[a-z])"  # 排除 5.15.153.1-microsoft-... 之类
        ),
        "high", "非保留、非文档段的公网 IP（可能是真实端点）"),
    ("SSH_FP", re.compile(r"SHA256:[A-Za-z0-9+/]{40,}"), "medium",
     "SSH 主机指纹（确认是否为公开测试向量）"),
    ("PRIVATE_KEY", re.compile(r"-----BEGIN [A-Z ]*PRIVATE KEY-----"), "critical",
     "私钥内容"),
    ("AUTH_TOKEN", re.compile(r"(?i)\b(gh[pousr]_[A-Za-z0-9]{16,}|sk-[A-Za-z0-9]{20,})\b"), "critical",
     "疑似 API token / 访问令牌"),
    ("PASSWORD_ASSIGN", re.compile(r"(?i)(password|passwd|secret)\s*[=:]\s*[\"'][^\"'\s]{8,}[\"']"), "high",
     "疑似硬编码口令"),
    ("HOSTNAME_LIKE", re.compile(r"(?i)\b(hostname|server_name)\s*[=:]\s*[\"']([a-z0-9][a-z0-9.-]{4,})[\"']"), "medium",
     "疑似真实主机名"),
]

SCAN_EXTS = {
    ".rs", ".ts", ".js", ".mjs", ".cjs", ".vue", ".json", ".md", ".toml",
    ".ps1", ".sh", ".yml", ".yaml", ".py", ".html", ".css", ".txt", ".gitattributes",
}
# 允许列表：这些命中是测试/安全逻辑的正常产物，不构成泄露。
# 命中内容只要「包含」其中任一子串即被跳过。
ALLOW_SUBSTRINGS = (
    "100.64.0.0", "100.64.0.1",        # CGNAT 段：bridge 安全拦截逻辑
    "169.254.0.0", "169.254.1.1",      # 链路本地：同上
    "127.0.0.0", "127.5.5.5",          # 回环段：同上
    "172.32.0.1", "2606:4700",         # 安全白名单测试用例
    "5.15.153.1-microsoft-standard",   # WSL2 内核版本号，非 IP（形如 5.x.y.z）
    "142.250.72.14",                   # Google DNS 解析示例
    "8.8.8.8", "1.1.1.1",              # 公共 DNS
    "203.0.113", "198.51.100", "192.0.2",  # RFC 5737 文档段
    # 以下指纹均已人工核实为公开测试向量，非真实主机密钥：
    #   ungWv48Bz... = SHA256("abc")，OpenSSH 指纹算法的标准自检向量
    #   ZkAslGjF...  = 本仓库合成密钥（32 字节递增序列）的指纹
    "SHA256:ungWv48Bz+pBQUDeXa4iI7ADYaOWF3qctBD/YfIAFa0",
    "SHA256:ZkAslGjFiUHdGf/WUL8rQvkib4PTvQatUV0OUQSncCA",
    "SHA256:xxxx",                     # 文档中的占位示意
    # GitHub 官方公开的主机密钥指纹（见 GitHub 文档 "SSH key fingerprints"）
    "SHA256:uNiVztksCsDhcc0u9e8BujQXVUpKZIDTMczCvj3tD2s",
    "docker-fixture",                  # 集成测试夹具主机名
    "C:\\Users\\you",                  # 输入框占位符
    "D:\\Tools",                       # 本项目统一使用的示意路径前缀
    "D:\\Clash Verge",                 # 同上
)

SKIP_DIRS = ("node_modules/", "src-tauri/target/", "dist/", ".workbuddy-ai/",
             "src-tauri/gen/", "package-lock.json", "src-tauri/Cargo.lock")
SKIP_PATHS = ("docs/LostCodexGateway",)  # 需求/计划文档本身含历史记录


def git_files(all_files: bool):
    if all_files:
        out = subprocess.run(["git", "ls-files", "--cached", "--others", "--exclude-standard"],
                             capture_output=True, text=True).stdout
    else:
        out = subprocess.run(["git", "ls-files"], capture_output=True, text=True).stdout
    return [f for f in out.splitlines() if f.strip()]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--all", action="store_true", help="含未跟踪文件")
    args = ap.parse_args()

    files = git_files(args.all)
    findings = []
    scanned = 0

    for path in files:
        if any(s in path for s in SKIP_DIRS) or any(path.startswith(s) for s in SKIP_PATHS):
            continue
        if path.endswith(".png") or path.endswith(".ico") or path.endswith(".exe"):
            continue
        sv = path.rsplit(".", 1)
        if len(sv) != 2 or ("." + sv[1].lower()) not in SCAN_EXTS:
            continue
        try:
            text = open(path, encoding="utf-8", errors="ignore").read()
        except OSError:
            continue
        scanned += 1
        for name, rx, sev, desc in RULES:
            for m in rx.finditer(text):
                hit = m.group(0)
                if any(a in hit or a in text[max(0, m.start() - 10): m.end() + 10]
                       for a in ALLOW_SUBSTRINGS):
                    continue
                line = text[: m.start()].count("\n") + 1
                snippet = text[max(0, m.start() - 40): m.end() + 40].replace("\n", " ")
                findings.append((sev, name, path, line, hit, snippet.strip(), desc))

    order = {"critical": 0, "high": 1, "medium": 2, "low": 3, "info": 4}
    findings.sort(key=lambda x: (order[x[0]], x[2], x[3]))

    print(f"扫描 {scanned} 个文本文件，命中 {len(findings)} 条\n")
    counts = {}
    for f in findings:
        counts[f[0]] = counts.get(f[0], 0) + 1
    for sev in ("critical", "high", "medium", "low", "info"):
        if sev in counts:
            print(f"  {sev:9s}: {counts[sev]}")
    print()

    for sev, name, path, line, hit, snip, desc in findings:
        if sev == "info":
            continue
        print(f"[{sev.upper()}] {name}  {path}:{line}")
        print(f"    命中: {hit}")
        print(f"    上下文: ...{snip}...")
        print(f"    说明: {desc}\n")

    blocking = [f for f in findings if f[0] in ("critical", "high", "medium")]
    if blocking:
        print(f"⚠ 有 {len(blocking)} 条需要人工确认（critical/high/medium）")
        return 1
    print("✓ 未发现需要处理的高风险项")
    return 0


if __name__ == "__main__":
    sys.exit(main())
