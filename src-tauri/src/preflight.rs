//! 环境自检（环境体检 / doctor）。
//!
//! 目的：让**一台新电脑拿到本软件后**，用户能一眼看出「还差哪一步」，
//! 而不是卡在某个模糊报错上。
//!
//! 设计原则（对应产品要求「能做的就做，不能做的做成按钮展示操作步骤」）：
//!
//! - **能自动检测的一律自动检测**：ssh.exe / 私钥 / 服务器配置 / Host Key /
//!   Clash Verge / Codex CLI / 端口占用。这些都是**只读**动作，无副作用。
//! - **不能自动修的不假装能修**：凡是涉及「需要用户决策」或「需要管理员权限」或
//!   「需要在服务器上动手」的，一律给出 `FixKind::Guide` + 明确操作步骤，
//!   而不是弹一个会失败的按钮。
//! - **绝不擅自改系统**：本模块只读，不装东西、不提权、不写路由。
//!
//! 每个检查项都是 [`PreflightItem`]，字段设计刻意与 `diagnostics.rs` 的
//! `DiagItem`（`status` + `detail`）保持同一范式，便于前端复用样式。

use crate::config::GatewayConfig;
use crate::ssh;
use serde::{Deserialize, Serialize};

/// 单项检查的结果状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PreflightStatus {
    /// 通过，无需操作
    Ok,
    /// 能用但有隐患（例如 OpenSSH 版本偏老）
    Warn,
    /// 缺失或不可用，**阻塞**正常使用
    Error,
    /// 无法判定（例如服务器未配置，导致后续项无从检查）
    Unknown,
}

/// 「这一项要不要用户动手」以及「怎么动手」。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FixKind {
    /// 无需操作
    None,
    /// 界面内有按钮可以自动完成（前端据 `action` 渲染按钮）
    Auto,
    /// 必须人工操作：给出步骤，前端逐条展示
    Guide,
}

/// 环境自检项。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreflightItem {
    /// 稳定标识（前端据此决定渲染哪个按钮 / 跳哪个页面）
    pub key: String,
    /// 面向用户的名称
    pub label: String,
    pub status: PreflightStatus,
    /// 一句话现状说明
    pub detail: String,
    pub fix: FixKind,
    /// `FixKind::Auto` 时对应的前端动作标识（如 `open_server_page`）；
    /// `FixKind::Guide` 时为 `None`。
    pub action: Option<String>,
    /// 操作步骤（`FixKind::Guide` 时非空），按顺序展示。
    pub steps: Vec<String>,
    /// 该项是否为「阻塞项」：为 true 且未通过时，软件**无法**正常使用。
    pub blocking: bool,
}

impl PreflightItem {
    fn ok(key: &str, label: &str, detail: impl Into<String>) -> Self {
        Self {
            key: key.to_string(),
            label: label.to_string(),
            status: PreflightStatus::Ok,
            detail: detail.into(),
            fix: FixKind::None,
            action: None,
            steps: Vec::new(),
            blocking: false,
        }
    }

    fn warn(key: &str, label: &str, detail: impl Into<String>) -> Self {
        Self {
            key: key.to_string(),
            label: label.to_string(),
            status: PreflightStatus::Warn,
            detail: detail.into(),
            fix: FixKind::None,
            action: None,
            steps: Vec::new(),
            blocking: false,
        }
    }

    fn unknown(key: &str, label: &str, detail: impl Into<String>) -> Self {
        Self {
            key: key.to_string(),
            label: label.to_string(),
            status: PreflightStatus::Unknown,
            detail: detail.into(),
            fix: FixKind::None,
            action: None,
            steps: Vec::new(),
            blocking: false,
        }
    }

    /// 阻塞项：缺失时无法使用。
    fn blocking_error(
        key: &str,
        label: &str,
        detail: impl Into<String>,
        fix: FixKind,
        action: Option<&str>,
        steps: Vec<String>,
    ) -> Self {
        Self {
            key: key.to_string(),
            label: label.to_string(),
            status: PreflightStatus::Error,
            detail: detail.into(),
            fix,
            action: action.map(String::from),
            steps,
            blocking: true,
        }
    }

    /// 非阻塞的引导项：缺失时能力受限，但基础功能可用。
    fn guide(
        key: &str,
        label: &str,
        status: PreflightStatus,
        detail: impl Into<String>,
        steps: Vec<String>,
    ) -> Self {
        Self {
            key: key.to_string(),
            label: label.to_string(),
            status,
            detail: detail.into(),
            fix: FixKind::Guide,
            action: None,
            steps,
            blocking: false,
        }
    }
}

/// 环境自检总报告。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreflightReport {
    pub items: Vec<PreflightItem>,
    /// 通过项数量
    pub passed: usize,
    /// 阻塞项（`blocking == true` 且未通过）数量
    pub blocking_failed: usize,
    /// 是否已具备跑通的最小条件（无阻塞项失败）
    pub ready: bool,
    /// 面向用户的一句话总结
    pub summary: String,
    pub ts: String,
}

/// 是否属于「已知的系统/内置 OpenSSH」（用于判断是否需要引导安装）。
fn is_builtin_ssh(path: &str) -> bool {
    let p = path.to_lowercase();
    p.contains("system32") || p.contains("sysnative")
}

/// 单个检测项是否存在（用于私钥存在性判断，不读内容）。
fn file_exists(path: &str) -> bool {
    !path.trim().is_empty() && std::path::Path::new(path.trim()).exists()
}

/// 检查本地端口是否已被**本工具自身以外**的进程占用。
///
/// 这里采用「能否成功 bind」作为判据，与 `verify::port_listening` 的语义一致：
/// bind 成功 = 没人占用（随即释放）。注意**不能**用「有人监听」判据——那是
/// 隧道自己起来之后的正常状态。
fn port_available(port: u16) -> bool {
    std::net::TcpListener::bind(("127.0.0.1", port)).is_ok()
}

/// 跑一次完整的环境自检。**只读**，不改任何配置或系统状态。
pub fn run_preflight(cfg: &GatewayConfig) -> PreflightReport {
    let mut items: Vec<PreflightItem> = Vec::new();

    // ---- 1. 系统 OpenSSH ----
    let env = ssh::detect_ssh_env();
    if env.exists {
        let builtin = is_builtin_ssh(&env.path);
        // 版本串形如 "OpenSSH_9.5p1, OpenSSL 3.0.14 ..."
        let ver_num = env
            .version
            .split_whitespace()
            .next()
            .unwrap_or("")
            .trim_start_matches("OpenSSH_")
            .to_string();
        let detail = format!("{}（{}）", env.path, env.version);
        if builtin {
            items.push(PreflightItem::ok("ssh_exe", "系统 OpenSSH", detail));
        } else {
            // PATH 里的是第三方 ssh（例如 Git 自带）。能用，但版本行为可能不同，
            // 且本工具默认优先 System32——这里如实报告但降级为提示。
            items.push(PreflightItem::warn(
                "ssh_exe",
                "系统 OpenSSH",
                format!("{} —— 非 Windows 内置路径，建议确认版本行为", detail),
            ));
        }
        let _ = ver_num;
    } else {
        items.push(PreflightItem::blocking_error(
            "ssh_exe",
            "系统 OpenSSH",
            "未找到 ssh.exe（System32 与 PATH 中均无）",
            FixKind::Guide,
            None,
            vec![
                "以管理员身份打开 PowerShell".to_string(),
                "查看是否已安装：Get-WindowsCapability -Online | Where-Object Name -like 'OpenSSH.Client*'".to_string(),
                "若 State 不是 Installed，执行：Add-WindowsCapability -Online -Name OpenSSH.Client~~~~0.0.1.0".to_string(),
                "若公司网络装不上，可改用 Git for Windows 自带的 ssh.exe，并在「服务器」页手动填写其路径".to_string(),
                "装好后回到本页点「重新体检」".to_string(),
            ],
        ));
    }

    // ---- 2. 服务器配置 ----
    let active = cfg.active_server();
    match active {
        None => {
            items.push(PreflightItem::blocking_error(
                "server",
                "服务器",
                "尚未配置任何服务器",
                FixKind::Auto,
                Some("open_server_page"),
                vec!["在「服务器」页填写主机、端口、用户名与私钥路径".to_string()],
            ));
        }
        Some(s) => {
            let name = s.display_name();
            let target = s.target();
            if s.key_path.trim().is_empty() {
                items.push(PreflightItem::blocking_error(
                    "ssh_key",
                    "SSH 私钥",
                    format!("「{}」未配置私钥路径", name),
                    FixKind::Guide,
                    None,
                    vec![
                        "若已有私钥：在「服务器」页把「SSH 私钥路径」填成私钥文件（如 C:\\Users\\你\\.ssh\\id_ed25519）".to_string(),
                        "若没有私钥：打开 PowerShell 执行 ssh-keygen -t ed25519 -C \"你的备注\"，一路回车即可".to_string(),
                        "把生成的 id_ed25519.pub（公钥）内容追加到服务器的 ~/.ssh/authorized_keys".to_string(),
                        "注意：填的是**私钥**（id_ed25519），不是 id_ed25519.pub——公钥不能用于登录".to_string(),
                    ],
                ));
            } else if !file_exists(&s.key_path) {
                // 这是实测踩过的坑：填了 .pub 公钥路径。给出针对性诊断。
                let hint = if s.key_path.trim().to_lowercase().ends_with(".pub") {
                    "（结尾是 .pub —— 这是**公钥**，SSH 登录必须用私钥）"
                } else {
                    ""
                };
                items.push(PreflightItem::blocking_error(
                    "ssh_key",
                    "SSH 私钥",
                    format!("「{}」的私钥文件不存在：{}{}", name, s.key_path, hint),
                    FixKind::Guide,
                    None,
                    vec![
                        "在「服务器」页修正「SSH 私钥路径」指向真实私钥文件".to_string(),
                        "若私钥放在其他电脑上，需要先拷贝过来（私钥是登录凭据，不能公开传输）".to_string(),
                        "若还没有私钥，执行 ssh-keygen -t ed25519 生成后把公钥放到服务器".to_string(),
                    ],
                ));
            } else {
                items.push(PreflightItem::ok(
                    "ssh_key",
                    "SSH 私钥",
                    format!("已就绪（{}）", s.key_path),
                ));
            }

            // 服务器连通性（只做 DNS/TCP 可达，不做 SSH 握手，避免误触发认证）
            items.push(check_server_reachable(&name, &s.host, s.port, &target));

            // Host Key 是否已确认
            let (known, fp, _kt) = ssh::host_key_known(s);
            if known {
                items.push(PreflightItem::ok(
                    "host_key",
                    "Host Key",
                    format!("已确认（{}）", fp.unwrap_or_else(|| "指纹未知".to_string())),
                ));
            } else {
                items.push(PreflightItem::blocking_error(
                    "host_key",
                    "Host Key",
                    format!("「{}」的服务器指纹尚未确认", name),
                    FixKind::Auto,
                    Some("open_server_page"),
                    vec![
                        "到「服务器」页点「查询服务器指纹」".to_string(),
                        "与服务器管理员提供的指纹逐字比对（或通过其他可信渠道核实）".to_string(),
                        "核对一致后点「我已核对，确认写入」（写入前会自动备份 known_hosts）".to_string(),
                        "⚠️ 这是安全设计：指纹无法自动信任，必须由你确认一次".to_string(),
                    ],
                ));
            }

            // 服务器侧是否允许 TCP 转发——只有连上才能探测，此处给出**人工核查路径**
            items.push(PreflightItem::guide(
                "allow_tcp_forwarding",
                "服务器 TCP 转发",
                PreflightStatus::Unknown,
                "需连接后才能判定。若服务器 sshd 关闭了 AllowTcpForwarding，连接时会明确报「远端禁止 TCP 转发」。",
                vec![
                    "若连接时报「远端禁止 TCP 转发」，说明服务器 sshd 关闭了转发".to_string(),
                    "用管理员登录服务器，编辑 /etc/ssh/sshd_config".to_string(),
                    "确认存在：AllowTcpForwarding yes（或被注释掉，注释掉等于默认 yes）".to_string(),
                    "改完后执行：sudo systemctl restart sshd（Debian/Ubuntu）或 sudo systemctl restart ssh（RHEL 系）".to_string(),
                    "⚠️ 重启 sshd 不会断开已有连接，但请确认你有其他登录方式再操作".to_string(),
                ],
            ));
        }
    }

    // ---- 3. 本地端口 ----
    let socks_port = cfg.socks_port();
    let bridge_port = cfg.bridge_port();
    if port_available(socks_port) {
        items.push(PreflightItem::ok(
            "socks_port",
            "本地 SOCKS 端口",
            format!("127.0.0.1:{} 可用", socks_port),
        ));
    } else {
        items.push(PreflightItem::blocking_error(
            "socks_port",
            "本地 SOCKS 端口",
            format!("127.0.0.1:{} 已被其他程序占用", socks_port),
            FixKind::Auto,
            Some("open_settings_page"),
            vec![
                "到「设置」页把「本地 SOCKS 端口」改成其他值（如 17811）".to_string(),
                "若该端口是本工具上次异常退出残留的隧道，可先确认没有遗留 ssh.exe 进程".to_string(),
            ],
        ));
    }
    if bridge_port != 0 && !port_available(bridge_port) {
        items.push(PreflightItem::blocking_error(
            "bridge_port",
            "桥接端口",
            format!("127.0.0.1:{} 已被占用", bridge_port),
            FixKind::Auto,
            Some("open_settings_page"),
            vec!["到「设置」页把「桥接端口」改成其他值".to_string()],
        ));
    }

    // ---- 4. Codex CLI ----
    match crate::launchers::locate_codex() {
        Some((label, path, _)) => items.push(PreflightItem::ok(
            "codex_cli",
            "Codex CLI",
            format!("{}：{}", label, path.display()),
        )),
        None => items.push(PreflightItem::guide(
            "codex_cli",
            "Codex CLI",
            PreflightStatus::Warn,
            "未找到 codex（不影响网关本身；仅影响「从网关启动 CLI」功能）",
            vec![
                "安装 Node.js 18+（https://nodejs.org）".to_string(),
                "打开新的终端执行：npm i -g @openai/codex".to_string(),
                "确认：新开终端执行 codex --version 能输出版本".to_string(),
                "回到本页点「重新体检」；若仍找不到，重启本软件以刷新 PATH".to_string(),
            ],
        )),
    }

    // ---- 5. Clash Verge（Desktop / IDE 场景才需要）----
    let mihomo = crate::mihomo::detect();
    if mihomo.verge_installed {
        let running = mihomo.verge_running && mihomo.mihomo_running;
        let detail = format!(
            "{}｜内核{}｜mixed-port {}｜TUN {}",
            mihomo.verge_version.clone().unwrap_or_else(|| "版本未知".to_string()),
            if mihomo.mihomo_running { "运行中" } else { "未运行" },
            mihomo
                .mixed_port
                .map(|p| p.to_string())
                .unwrap_or_else(|| "未检测到".to_string()),
            if mihomo.tun_enabled { "已开启" } else { "未开启" },
        );
        // Verge 只影响 Desktop/IDE 场景，**不阻塞** CLI 用法
        if running {
            items.push(PreflightItem::ok("clash_verge", "Clash Verge", detail));
        } else {
            items.push(PreflightItem::guide(
                "clash_verge",
                "Clash Verge",
                PreflightStatus::Warn,
                format!("{}（未运行 —— 仅影响 Desktop/IDE 场景）", detail),
                vec![
                    "启动 Clash Verge".to_string(),
                    "若要用 Codex Desktop / IDE：需在 Verge 里开启「TUN 模式」（需管理员权限）".to_string(),
                    "纯用 Codex CLI 则**不需要** TUN，也不需要系统代理".to_string(),
                ],
            ));
        }

        // 规则片段是否已导入——通过检测 Verge 的 profile 里有没有我们的标记
        items.push(check_mihomo_fragment(&mihomo));
    } else {
        items.push(PreflightItem::guide(
            "clash_verge",
            "Clash Verge",
            PreflightStatus::Warn,
            "未检测到 Clash Verge Rev（仅 Desktop/IDE 场景需要）",
            vec![
                "从 https://github.com/clash-verge-rev/clash-verge-rev/releases 下载安装".to_string(),
                "安装后在 Verge 里导入你的订阅".to_string(),
                "再到「应用」页点「检测 Mihomo / Clash Verge」".to_string(),
                "纯用 Codex CLI 则无需安装 Clash Verge".to_string(),
            ],
        ));
    }

    // ---- 6. WSL（可选）----
    // 注意：`wsl.exe` 探测有 20s 量级的冷启动成本，且与「新电脑能否跑通」无关
    // （WSL 属于可选场景）。自检中不做主动探测，避免拖慢整页；只报告为「可选」。
    items.push(PreflightItem::unknown(
        "wsl",
        "WSL2（可选）",
        "未检测。如需在 WSL2 里用 Codex，请到「WSL2」页单独探测（不影响 Windows 侧使用）。",
    ));

    // ---- 7. 以下为「引导型」项：只教怎么做，绝不动手 ----
    //
    // 设计前提（与用户明确约定）：不做自动安装、不写 ~/.ssh、不推公钥到远端、
    // 不内置便携 OpenSSH（体积成本）。全部只产出**可照抄的步骤**。
    // 因此 `fix` 一律是 `Guide`，`action` 一律为 `None` —— 界面上只出现「复制命令」，
    // 不会出现「帮你做」的按钮。这样用户对「软件会不会偷偷改我东西」有确定预期。

    items.push(PreflightItem::guide(
        "ssh_keygen",
        "生成 SSH 密钥",
        PreflightStatus::Unknown,
        "引导项：若你还没有 SSH 密钥对，按下方步骤生成。本软件不会替你生成或保存私钥。",
        vec![
            "打开 PowerShell（普通权限即可，不需要管理员）".to_string(),
            "执行：ssh-keygen -t ed25519 -C \"lcfg\"".to_string(),
            "一路回车使用默认路径，也可自定义；密钥口令可留空，也可设置（更安全）".to_string(),
            "生成后会得到两个文件：id_ed25519（私钥，绝不外传）与 id_ed25519.pub（公钥，可以公开）".to_string(),
            "用记事本打开 .pub，或在 PowerShell 执行：Get-Content $env:USERPROFILE\\.ssh\\id_ed25519.pub".to_string(),
            "⚠️ 私钥一旦泄露等于服务器被他人登录——不要发到聊天工具、不要贴进 issue".to_string(),
        ],
    ));

    items.push(PreflightItem::guide(
        "push_pubkey",
        "上传公钥到服务器",
        PreflightStatus::Unknown,
        "引导项：把公钥装到服务器的 authorized_keys。需要你能登录服务器，本软件不会代为连接写入。",
        vec![
            "先在本地复制公钥全文：Get-Content $env:USERPROFILE\\.ssh\\id_ed25519.pub | Set-Clipboard".to_string(),
            "登录服务器：ssh 用户名@服务器地址".to_string(),
            "在服务器上执行：mkdir -p ~/.ssh && chmod 700 ~/.ssh".to_string(),
            "把公钥追加进授权文件：echo '粘贴你的公钥全文' >> ~/.ssh/authorized_keys".to_string(),
            "修正权限（权限过松 sshd 会拒绝使用公钥）：chmod 600 ~/.ssh/authorized_keys".to_string(),
            "退出后重连验证：ssh 用户名@服务器地址 —— 不再提示输入密码即成功".to_string(),
            "若服务器只开放了面板（如宝塔/云厂商控制台），也可在面板的「SSH 密钥」功能里粘贴公钥".to_string(),
        ],
    ));

    items.push(PreflightItem::guide(
        "codex_install",
        "安装 Codex",
        PreflightStatus::Unknown,
        "引导项：按下方步骤安装 Codex。安装与否不影响网关本身（网关只负责 SSH 隧道转发）。",
        vec![
            "先装 Node.js 18 或更高版本：从 https://nodejs.org 下载 LTS 版安装，或执行 winget install OpenJS.NodeJS.LTS".to_string(),
            "关闭并重新打开终端（让 PATH 生效）".to_string(),
            "验证 Node 可用：node --version && npm --version".to_string(),
            "安装 Codex CLI：npm i -g @openai/codex".to_string(),
            "验证：codex --version 能输出版本号".to_string(),
            "若 npm 全局安装报权限错误，改用：npm i -g @openai/codex --prefix %APPDATA%\\npm".to_string(),
            "回到本页点「重新体检」，确认「Codex CLI」项变为通过".to_string(),
        ],
    ));

    items.push(PreflightItem::guide(
        "openssh_portable",
        "便携 OpenSSH（备选）",
        PreflightStatus::Unknown,
        "引导项：不想装系统 OpenSSH 时，可指向一份「绿色版」ssh.exe。本软件**不内置**（避免把安装包撑大），只教你从已有的软件里借用。",
        vec![
            "方案 A（最常见）：Git for Windows 自带 ssh.exe，路径通常是 C:\\Program Files\\Git\\usr\\bin\\ssh.exe".to_string(),
            "方案 B：Win32-OpenSSH 官方 release（https://github.com/PowerShell/Win32-OpenSSH/releases）下载 zip，解压到自定义目录".to_string(),
            "方案 C：conda 环境通常也带 ssh：查找 G:\\MiniConda3\\envs\\*\\Library\\bin\\ssh.exe".to_string(),
            "拿到路径后到「服务器」页，把「SSH 可执行文件路径」填成该 ssh.exe 的完整路径".to_string(),
            "回到本页点「重新体检」，确认「系统 OpenSSH」项不再报阻塞".to_string(),
            "⚠️ 用非内置 ssh 时版本行为可能略有差异；若遇到奇怪报错，优先改用系统 OpenSSH 交叉验证".to_string(),
        ],
    ));

    // ---- 汇总 ----
    let passed = items
        .iter()
        .filter(|i| i.status == PreflightStatus::Ok)
        .count();
    let blocking_failed = items
        .iter()
        .filter(|i| i.blocking && i.status != PreflightStatus::Ok)
        .count();
    let ready = blocking_failed == 0;

    let summary = if ready {
        if passed == items.len() {
            "全部检查通过，可以直接使用。".to_string()
        } else {
            format!("关键项已就绪（{} 项提示可忽略）。", items.len() - passed)
        }
    } else {
        format!(
            "有 {} 项阻塞问题需要处理，处理完才能正常使用。",
            blocking_failed
        )
    };

    PreflightReport {
        items,
        passed,
        blocking_failed,
        ready,
        summary,
        ts: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
    }
}

/// 服务器可达性：只做 TCP 连接（不跑 SSH 握手）。
///
/// 刻意不做 SSH 握手：握手会触发认证/写 known_hosts 等副作用，而自检必须是只读的。
/// 因此这里只能判定「端口通不通」，**不判定**认证是否成功——后者由「测试连接」负责。
fn check_server_reachable(
    name: &str,
    host: &str,
    port: u16,
    target: &str,
) -> PreflightItem {
    use std::net::{TcpStream, ToSocketAddrs};
    use std::time::Duration;

    let host = host.trim();
    if host.is_empty() {
        return PreflightItem::blocking_error(
            "server_reachable",
            "服务器连通性",
            format!("「{}」未填写主机地址", name),
            FixKind::Auto,
            Some("open_server_page"),
            vec!["在「服务器」页填写主机地址".to_string()],
        );
    }

    // 先做域名解析：解析失败和端口不通是两类问题，分开报告
    let addrs = match (host, port).to_socket_addrs() {
        Ok(a) => a.collect::<Vec<_>>(),
        Err(e) => {
            return PreflightItem::blocking_error(
                "server_reachable",
                "服务器连通性",
                format!("「{}」的地址无法解析：{}", name, e),
                FixKind::Auto,
                Some("open_server_page"),
                vec![
                    "检查主机地址拼写".to_string(),
                    "若用的是域名，确认本机 DNS 可用（例如 nslookup 该域名）".to_string(),
                    "可改用 IP 地址试试".to_string(),
                ],
            )
        }
    };

    let mut last_err = String::new();
    for addr in addrs.iter().take(3) {
        match TcpStream::connect_timeout(addr, Duration::from_secs(5)) {
            Ok(_) => {
                return PreflightItem::ok(
                    "server_reachable",
                    "服务器连通性",
                    format!("{} 可达", target),
                )
            }
            Err(e) => last_err = e.to_string(),
        }
    }
    PreflightItem::blocking_error(
        "server_reachable",
        "服务器连通性",
        format!("「{}」端口不通：{}", name, last_err),
        FixKind::Guide,
        None,
        vec![
            "确认服务器已开机、SSH 服务在运行".to_string(),
            "确认端口号正确（不是默认 22 的话要看服务商面板）".to_string(),
            "确认服务器防火墙 / 云厂商安全组放行了该端口".to_string(),
            "若本机需要代理才能访问外网，先确认该服务器地址是否被代理拦住".to_string(),
        ],
    )
}

/// 检测 Clash Verge 的 profile 中是否已包含本工具生成的规则片段。
///
/// 判据：在 Verge 数据目录的 `profiles/` 下任意 yaml 中检索本工具的稳定标记
/// （`MY-VPS` 组名或 `lcfg-gateway` 代理名）。这是**只读**检索。
fn check_mihomo_fragment(mihomo: &crate::mihomo::MihomoDetection) -> PreflightItem {
    let Some(dir) = mihomo.profiles_dir.as_ref() else {
        return PreflightItem::unknown(
            "mihomo_fragment",
            "Clash 规则片段",
            "未定位到 Verge 的 profiles 目录，无法判定",
        );
    };
    let dir = std::path::Path::new(dir);
    if !dir.is_dir() {
        return PreflightItem::unknown(
            "mihomo_fragment",
            "Clash 规则片段",
            format!("profiles 目录不存在：{}", dir.display()),
        );
    }

    let mut found_marker = false;
    let mut found_rules = false;
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().map(|x| x != "yaml" && x != "yml").unwrap_or(true) {
                continue;
            }
            // 只读小文件，避免把整个订阅（可能几百 KB）读进内存做无关匹配
            if let Ok(meta) = p.metadata() {
                if meta.len() > 2 * 1024 * 1024 {
                    continue;
                }
            }
            if let Ok(text) = std::fs::read_to_string(&p) {
                if text.contains("lcfg-gateway") || text.contains("MY-VPS") {
                    found_marker = true;
                    if text.contains("PROCESS-NAME,codex.exe") {
                        found_rules = true;
                    }
                }
            }
        }
    }

    if found_rules {
        PreflightItem::ok(
            "mihomo_fragment",
            "Clash 规则片段",
            "已检测到 Codex 进程规则（MY-VPS 组）。注意：不要把它默认选中隧道出口，否则网关未连接时 Codex 会一直重连。",
        )
    } else if found_marker {
        PreflightItem::guide(
            "mihomo_fragment",
            "Clash 规则片段",
            PreflightStatus::Warn,
            "检测到网关组但未见 Codex 进程规则，Desktop/IDE 场景可能未覆盖",
            vec![
                "到「应用」页点「生成 Mihomo 规则片段」".to_string(),
                "在 Verge 的 Profiles / 扩展配置中导入该片段（推荐用 merge/rules 入口）".to_string(),
                "导入前先备份目标文件".to_string(),
                "⚠️ 确保 MY-VPS 组的默认选项**不是**隧道出口，否则不用网关时 Codex 会一直重连".to_string(),
            ],
        )
    } else {
        PreflightItem::guide(
            "mihomo_fragment",
            "Clash 规则片段",
            PreflightStatus::Warn,
            "未检测到本工具生成的规则片段（仅 Desktop/IDE 场景需要）",
            vec![
                "纯用 Codex CLI 则**不需要**此步——CLI 由本工具注入代理，与 Clash 无关".to_string(),
                "若要用 Desktop/IDE：到「应用」页生成片段并导入 Verge".to_string(),
                "导入后开启 TUN 模式".to_string(),
            ],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::GatewayConfig;

    /// 空配置（无服务器）必须报告「服务器未配置」这个阻塞项，且不判定为就绪。
    ///
    /// 注意：`GatewayConfig::default()` **自带一台占位服务器**（见 lib.rs 测试的
    /// 说明），因此这里必须显式清空 `servers` 才能真正模拟「新装的空配置」。
    #[test]
    fn empty_config_is_not_ready() {
        let mut cfg = GatewayConfig::default();
        cfg.servers.clear();
        cfg.active_server_id = String::new();

        let r = run_preflight(&cfg);
        assert!(!r.ready, "无服务器时不应判定为就绪");
        assert!(r.blocking_failed > 0);
        assert!(
            r.items.iter().any(|i| i.key == "server" && i.blocking),
            "应报告服务器未配置；实际项：{:?}",
            r.items.iter().map(|i| i.key.as_str()).collect::<Vec<_>>()
        );
    }

    /// 默认配置（自带一台占位服务器）也不应直接判定为就绪——
    /// 因为占位项没有私钥、没有确认 Host Key。
    #[test]
    fn default_config_has_pending_items() {
        let cfg = GatewayConfig::default();
        let r = run_preflight(&cfg);
        assert!(
            !r.ready,
            "默认占位配置不应判定为就绪（私钥/Host Key 必然未就绪）"
        );
        assert!(r.blocking_failed > 0);
    }

    /// 每个阻塞失败项都必须给用户一条出路：
    /// 要么有 Auto 动作，要么有 Guide 步骤。绝不能出现「红了但没说怎么办」。
    /// 两种配置形态都测：空配置、默认占位配置。
    #[test]
    fn every_blocking_failure_tells_user_what_to_do() {
        let mut empty = GatewayConfig::default();
        empty.servers.clear();
        empty.active_server_id = String::new();

        for cfg in [empty, GatewayConfig::default()] {
            let r = run_preflight(&cfg);
            for item in r
                .items
                .iter()
                .filter(|i| i.blocking && i.status != PreflightStatus::Ok)
            {
                let has_way_out = item.fix == FixKind::Auto || !item.steps.is_empty();
                assert!(
                    has_way_out,
                    "阻塞项「{}」既无自动动作也无操作步骤，用户会卡死",
                    item.label
                );
            }
        }
    }

    /// 检查项的 key 必须唯一——前端用它做 key，重复会导致渲染错乱。
    #[test]
    fn item_keys_are_unique() {
        let cfg = GatewayConfig::default();
        let r = run_preflight(&cfg);
        let mut keys: Vec<&str> = r.items.iter().map(|i| i.key.as_str()).collect();
        let total = keys.len();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), total, "存在重复的检查项 key");
    }

    /// 与用户明确约定的边界：**只教不做**。
    ///
    /// 环境自检里有若干「动手型」能力（生成密钥、推公钥到服务器、内置便携 OpenSSH、
    /// 装 Codex），我们刻意全做成引导步骤，理由是它们会写用户磁盘 / 改远端服务器 /
    /// 撑大安装包。这条测试把约定固化成断言：`fix == Guide` 的项**不允许**携带
    /// `action`，否则前端会渲染出「帮你做」的按钮，等于偷偷越界。
    ///
    /// 反过来，`Auto` 类项也应只做「跳转到某个页面」这种界面内动作，
    /// 不该出现未知的 action 标识。
    #[test]
    fn guide_items_never_carry_auto_actions() {
        // 已知的界面内跳转目标。新增 action 时必须同步登记，否则前端跳不动。
        const KNOWN_ACTIONS: [&str; 3] = ["open_server_page", "open_settings_page", "open_apps_page"];

        let mut cfgs = vec![GatewayConfig::default()];
        let mut empty = GatewayConfig::default();
        empty.servers.clear();
        empty.active_server_id = String::new();
        cfgs.push(empty);

        for cfg in cfgs {
            let r = run_preflight(&cfg);
            for item in &r.items {
                match item.fix {
                    FixKind::Guide => {
                        assert!(
                            item.action.is_none(),
                            "引导项「{}」带了自动动作 {:?}——按约定引导项只教不做",
                            item.label,
                            item.action
                        );
                        assert!(
                            !item.steps.is_empty(),
                            "引导项「{}」没有任何步骤，等于只报问题不给出路",
                            item.label
                        );
                    }
                    FixKind::Auto => {
                        let act = item.action.as_deref().unwrap_or("");
                        assert!(
                            KNOWN_ACTIONS.contains(&act),
                            "自动项「{}」的 action「{}」不在已登记的目标里，前端会跳不动",
                            item.label,
                            act
                        );
                    }
                    FixKind::None => {}
                }
            }
        }
    }

    /// 四个「动手型」引导项必须存在且都给出可照抄的步骤。
    /// 它们代表用户明确要求的能力覆盖：装 SSH / 生成密钥 / 推公钥 / 装 Codex / 便携 OpenSSH。
    #[test]
    fn hands_on_topics_are_covered_by_guides() {
        let cfg = GatewayConfig::default();
        let r = run_preflight(&cfg);
        for key in [
            "ssh_keygen",
            "push_pubkey",
            "codex_install",
            "openssh_portable",
            "allow_tcp_forwarding",
        ] {
            let item = r
                .items
                .iter()
                .find(|i| i.key == key)
                .unwrap_or_else(|| panic!("缺少引导项：{}", key));
            assert_eq!(item.fix, FixKind::Guide, "「{}」必须是引导项", key);
            assert!(
                item.steps.len() >= 3,
                "「{}」只有 {} 条步骤，太薄了（至少 3 条）",
                key,
                item.steps.len()
            );
            assert!(
                item.steps.iter().all(|s| !s.trim().is_empty()),
                "「{}」存在空步骤文案",
                key
            );
        }
    }

    /// `.pub` 公钥路径要被识别为「私钥问题」并给出针对性提示。
    #[test]
    fn pub_key_path_is_flagged_as_key_problem() {
        let mut cfg = GatewayConfig::default();
        let mut s = crate::config::ServerProfile::default();
        s.id = "t1".to_string();
        s.name = "测试".to_string();
        s.host = "127.0.0.1".to_string();
        s.username = "u".to_string();
        // 指向一个必然不存在的 .pub 路径
        s.key_path = "C:\\__no_such_dir__\\id_rsa.pub".to_string();
        cfg.servers.push(s);
        cfg.active_server_id = "t1".to_string();

        let r = run_preflight(&cfg);
        let item = r.items.iter().find(|i| i.key == "ssh_key").expect("应有私钥项");
        assert_eq!(item.status, PreflightStatus::Error);
        assert!(
            item.detail.contains("公钥"),
            "应提示这是公钥而非私钥，实际: {}",
            item.detail
        );
    }

    /// 私钥路径留空时也要给引导，且不 panic。
    #[test]
    fn empty_key_path_guides_user() {
        let mut cfg = GatewayConfig::default();
        let mut s = crate::config::ServerProfile::default();
        s.id = "t2".to_string();
        s.host = "127.0.0.1".to_string();
        s.username = "u".to_string();
        s.key_path = String::new();
        cfg.servers.push(s);
        cfg.active_server_id = "t2".to_string();

        let r = run_preflight(&cfg);
        let item = r.items.iter().find(|i| i.key == "ssh_key").expect("应有私钥项");
        assert_eq!(item.status, PreflightStatus::Error);
        assert!(!item.steps.is_empty(), "应给出生成私钥的步骤");
    }

    /// 报告计数与实际项数一致。
    #[test]
    fn counts_match_items() {
        let cfg = GatewayConfig::default();
        let r = run_preflight(&cfg);
        let passed = r.items.iter().filter(|i| i.status == PreflightStatus::Ok).count();
        assert_eq!(r.passed, passed);
        assert_eq!(r.ready, r.blocking_failed == 0);
        assert!(!r.summary.is_empty());
    }

    /// `port_available` 对明显被占用的端口应返回 false。
    /// 用「自己先绑住再测」的方式构造，避免依赖环境里恰好占了哪个端口。
    #[test]
    fn port_available_detects_occupied() {
        let l = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("应能绑定随机端口");
        let port = l.local_addr().expect("应有本地地址").port();
        assert!(!port_available(port), "已被绑定的端口应判定为不可用");
        drop(l);
        assert!(port_available(port), "释放后应判定为可用");
    }

    /// 所有项的 label 与 detail 都不应为空（UI 会直接渲染）。
    #[test]
    fn texts_are_non_empty() {
        let cfg = GatewayConfig::default();
        let r = run_preflight(&cfg);
        for i in &r.items {
            assert!(!i.key.is_empty(), "key 为空");
            assert!(!i.label.is_empty(), "label 为空: {}", i.key);
            assert!(!i.detail.is_empty(), "detail 为空: {}", i.key);
        }
    }
}
