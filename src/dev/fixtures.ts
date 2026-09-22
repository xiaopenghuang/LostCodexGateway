/**
 * 仅用于开发期「有数据状态」的视觉检查（不会进入生产构建）。
 *
 * 为什么需要它：空状态下 `.stat-grid`、`.steps`、`table.tbl`、`.logbox`
 * 这类组件要么不渲染、要么只渲染一行占位文字，看不出长文本溢出、列宽
 * 挤压、网格换行等问题。这里注入一组贴近真实的快照数据来暴露这些缺陷。
 *
 * 启用方式：`VITE_LCFG_FIXTURE=<场景>` 启动 dev server，例如
 *   VITE_LCFG_FIXTURE=verified npm run dev:fixture
 */
import type {
  GatewaySnapshot, GatewayConfig, HostKeyInfo, SshEnv, LaunchPreview, LogEntry, ServerLatency,
} from "../types";

const now = () => new Date().toISOString().replace("T", " ").slice(0, 19);

function logs(): LogEntry[] {
  const t = now();
  return [
    { ts: t, level: "info", component: "state", message: "状态推进 UNCONFIGURED → READY" },
    { ts: t, level: "info", component: "ssh", message: "使用系统 OpenSSH: C:\\Windows\\System32\\OpenSSH\\ssh.exe (OpenSSH_for_Windows_9.5p1)" },
    { ts: t, level: "info", component: "tunnel", message: "本地 SOCKS5 监听已就绪 127.0.0.1:17801（动态端口转发 -D）" },
    { ts: t, level: "warn", component: "verify", message: "出口 IP 与本机直连相同，无法确认流量确实经由服务器" },
    { ts: t, level: "info", component: "bridge", message: "HTTP CONNECT 桥接层已启动 127.0.0.1:17800 → SOCKS5 远端 DNS" },
    { ts: t, level: "error", component: "bridge", message: "拒绝目标 192.168.1.1:80（BRIDGE_PRIVATE_TARGET，私有网段不经隧道转发）" },
    { ts: t, level: "info", component: "verify", message: "出口验证完成：3/4 项通过，耗时 4820ms" },
  ];
}

/**
 * 多服务器配置（v0.3.0）。
 *
 * 刻意包含三种状态各一台，用来暴露列表的布局问题：
 * - 正常条目
 * - 名称超长（检查列宽挤压）
 * - 主机名超长（检查 mono 溢出）
 * 端口是全局的（`settings`），不在任何一台服务器上。
 */
const CONFIG: GatewayConfig = {
  servers: [
    {
      id: "srv-1a2b3c",
      name: "东京中转节点",
      host: "vps.example.com",
      port: 22,
      username: "ubuntu",
      key_path: "C:\\Users\\me\\.ssh\\id_ed25519",
      ssh_exe_path: "C:\\Windows\\System32\\OpenSSH\\ssh.exe",
      expected_egress_ip: "203.0.113.47",
      gateway_group: "MY-VPS",
    },
    {
      id: "srv-4d5e6f",
      name: "备用出口（新加坡，用于主节点故障时切换）",
      host: "sg-backup-very-long-hostname.example.net",
      port: 2222,
      username: "root",
      key_path: "C:\\Users\\me\\.ssh\\id_ed25519_sg",
      ssh_exe_path: "",
      expected_egress_ip: "198.51.100.88",
      gateway_group: "MY-VPS",
    },
    {
      id: "srv-7a8b9c",
      name: "",
      host: "203.0.113.200",
      port: 22,
      username: "deploy",
      key_path: "",
      ssh_exe_path: "",
      expected_egress_ip: "",
      gateway_group: "MY-VPS",
    },
  ],
  active_server_id: "srv-1a2b3c",
  verify: {
    // 故意给一个很长的 URL，检查长文本换行/溢出
    endpoints: [
      "https://api.ipify.org?format=json",
      "https://ifconfig.me/ip",
    ],
    timeout_secs: 8,
  },
  settings: {
    auto_reconnect: true,
    max_reconnect_attempts: 3,
    disconnect_policy: "warn_and_block",
    socks_port: 17801,
    bridge_port: 17800,
    gateway_group: "MY-VPS",
  },
};

/** 延迟探测的样例结果（含「不可达」与「超长耗时」两种边界）。 */
export const LATENCIES: ServerLatency[] = [
  { id: "srv-1a2b3c", name: "东京中转节点", host: "vps.example.com", port: 22, reachable: true, latency_ms: 42, error: null },
  { id: "srv-4d5e6f", name: "备用出口（新加坡，用于主节点故障时切换）", host: "sg-backup-very-long-hostname.example.net", port: 2222, reachable: true, latency_ms: 386, error: null },
  { id: "srv-7a8b9c", name: "203.0.113.200", host: "203.0.113.200", port: 22, reachable: false, latency_ms: null, error: "超时（5s）" },
];

const VERIFY_OK = {
  ok: true,
  egress_ip: "203.0.113.47",
  steps: [
    { kind: "socks_handshake", label: "本地 SOCKS5 握手", ok: true, detail: "127.0.0.1:17801 可建立连接", timestamp: now() },
    { kind: "tunnel_test", label: "隧道连通性（经 SOCKS 请求服务器自身）", ok: true, detail: "收到 HTTP 200，耗时 412ms", timestamp: now() },
    { kind: "egress_ip", label: "出口 IP 检测（远端 DNS 解析）", ok: true, detail: "203.0.113.47", timestamp: now() },
    { kind: "direct_ip", label: "本机直连对照", ok: true, detail: "198.51.100.22", timestamp: now() },
  ],
  started_at: now(),
  finished_at: now(),
};

const VERIFY_SUSPECT = {
  ...VERIFY_OK,
  ok: false,
  egress_ip: "198.51.100.22",
  steps: [
    VERIFY_OK.steps[0],
    VERIFY_OK.steps[1],
    { kind: "egress_ip", label: "出口 IP 检测（远端 DNS 解析）", ok: true, detail: "198.51.100.22", timestamp: now() },
    { kind: "direct_ip", label: "本机直连对照", ok: false, detail: "198.51.100.22", timestamp: now() },
  ],
};

export const SSH_ENV: SshEnv = {
  exists: true,
  path: "C:\\Windows\\System32\\OpenSSH\\ssh.exe",
  version: "OpenSSH_for_Windows_9.5p1, LibreSSL 3.8.2",
};

export const HOST_KEY: HostKeyInfo = {
  known: true,
  fingerprint: "SHA256:uNiVztksCsDhcc0u9e8BujQXVUpKZIDTMczCvj3tD2s",
  key_type: "ssh-ed25519",
};

export const LAUNCH_PREVIEW: LaunchPreview = {
  command: "C:\\Users\\me\\AppData\\Roaming\\npm\\codex.cmd",
  env: {
    HTTPS_PROXY: "http://127.0.0.1:17802",
    HTTP_PROXY: "http://127.0.0.1:17802",
    ALL_PROXY: "socks5h://127.0.0.1:17801",
    NO_PROXY: "127.0.0.1,localhost,::1",
  },
  proxy_line: "HTTP CONNECT → SOCKS5（远端 DNS，socks5h）",
  bridge_needed: true,
};

const SCENARIOS: Record<string, GatewaySnapshot> = {
  /** 出口已验证：hero 变绿、步骤全打勾、桥接有数据 */
  verified: {
    state: "EGRESS_VERIFIED",
    config: CONFIG,
    last_verify: VERIFY_OK,
    ssh_pid: 24316,
    last_error: null,
    recent_logs: logs(),
    bridge_port: 17800,
    bridge_connections_total: 1284,
    bridge_last_target: "chatgpt.com:443",
    bridge_rejects_total: 3,
    bridge_recent_rejects: [
      { code: "BRIDGE_PRIVATE_TARGET", message: "私有网段不经隧道转发", target: "192.168.1.1:80" },
      { code: "BRIDGE_LOOPBACK_TARGET", message: "回环地址不经隧道转发", target: "127.0.0.1:8080" },
      { code: "BRIDGE_SELF_TARGET", message: "目标为桥接层自身端口，会形成循环代理", target: "127.0.0.1:17800" },
    ],
    previous_server_id: null,
  },

  /** 隧道通但出口可疑：hero 琥珀色 + egressSuspect 告警 */
  suspect: {
    state: "TUNNEL_READY",
    config: CONFIG,
    last_verify: VERIFY_SUSPECT,
    ssh_pid: 24316,
    last_error: null,
    recent_logs: logs(),
    bridge_port: 17800,
    bridge_connections_total: 47,
    bridge_last_target: "api.openai.com:443",
    bridge_rejects_total: 1,
    bridge_recent_rejects: [
      { code: "BRIDGE_LOOPBACK_TARGET", message: "回环地址不经隧道转发", target: "127.0.0.1:8080" },
    ],
    previous_server_id: null,
  },

  /** 连接中：脉冲动画 + 长错误文案 */
  connecting: {
    state: "RECONNECTING",
    config: CONFIG,
    last_verify: null,
    ssh_pid: null,
    last_error: "ssh: connect to host vps.example.com port 22: Connection timed out",
    recent_logs: logs().slice(0, 4),
    bridge_port: null,
    bridge_connections_total: 0,
    bridge_last_target: null,
    bridge_rejects_total: 0,
    bridge_recent_rejects: [],
    previous_server_id: null,
  },

  /**
   * 切换服务器中：SWITCHING 状态 + 已选中的是第二台 + previous_server_id 指向第一台。
   * 用来检查「切换中」文案、切回按钮、以及当前项高亮是否落在新服务器上。
   */
  switching: {
    state: "SWITCHING",
    config: { ...CONFIG, active_server_id: "srv-4d5e6f" },
    last_verify: null,
    ssh_pid: null,
    last_error: null,
    recent_logs: logs().slice(0, 5),
    bridge_port: null,
    bridge_connections_total: 0,
    bridge_last_target: null,
    bridge_rejects_total: 0,
    bridge_recent_rejects: [],
    previous_server_id: "srv-1a2b3c",
  },

  /**
   * 切换失败后的**恢复态**（`DISCONNECTED`）。
   *
   * 这是「切换失败卡死在 SWITCHING」修复后应当落到的状态，专门用来验证
   * **界面上至少还有一个可用按钮**——若「连接」也被禁用，用户仍然没有出路，
   * 修复就只做了一半。
   *
   * 数据刻意与 `switching` 对齐：选中项已经是新服务器、`previous_server_id`
   * 指向旧的那台，`last_error` 是真实的报错文案（走的是 Host Key 未确认这条
   * 最容易踩到的路径）。同时借此检查超长服务器名在错误信息里的换行表现。
   */
  disconnected: {
    state: "DISCONNECTED",
    config: { ...CONFIG, active_server_id: "srv-4d5e6f" },
    last_verify: null,
    ssh_pid: null,
    last_error:
      "已切换到「备用出口（新加坡，用于主节点故障时切换）」但连接失败："
      + "服务器 Host Key 尚未确认：请先在「服务器」页查询并核对指纹后确认。"
      + "已选中项仍为「备用出口（新加坡，用于主节点故障时切换）」，"
      + "可点「切回上一个」回到「东京中转节点」",
    recent_logs: logs().slice(0, 5),
    bridge_port: null,
    bridge_connections_total: 0,
    bridge_last_target: null,
    bridge_rejects_total: 0,
    bridge_recent_rejects: [],
    previous_server_id: "srv-1a2b3c",
  },

  /** 错误态：超长错误信息，检查溢出 */
  error: {
    state: "ERROR",
    config: CONFIG,
    last_verify: null,
    ssh_pid: null,
    last_error:
      "SSH 认证失败（AuthenticationFailed）：BatchMode 下无法使用密钥 C:\\Users\\me\\.ssh\\id_ed25519，"
      + "且未检测到可用的 ssh-agent。请确认私钥路径正确、文件权限未被其他用户读取、或已在 ssh-agent 中加载对应密钥。",
    recent_logs: logs().slice(0, 3),
    bridge_port: null,
    bridge_connections_total: 0,
    bridge_last_target: null,
    bridge_rejects_total: 0,
    bridge_recent_rejects: [],
    previous_server_id: null,
  },
};

export function fixtureSnapshot(name: string): GatewaySnapshot | null {
  return SCENARIOS[name] ?? null;
}

export const SCENARIO_NAMES = Object.keys(SCENARIOS);

/* ------------------------------------------------------------------ *
 * Mihomo 检测（Applications 页）
 * ------------------------------------------------------------------ */
export const MIHOMO = {
  detected: true,
  verge_version: "2.3.4",
  verge_path: "D:\\Clash Verge\\clash-verge.exe",
  verge_running: true,
  mihomo_running: true,
  mixed_port: 7897,
  external_controller: "127.0.0.1:9097",
  tun_enabled: false,
  mode: "rule",
  notes: [
    "Verge 与 Mihomo 内核均在运行，mixed-port 可用于规则分流。",
    "TUN 未开启：Desktop / IDE 的进程级分流尚未覆盖。",
    "未读取订阅内容，仅探测进程与监听端口。",
  ],
};

/* ------------------------------------------------------------------ *
 * WSL2 探测（WSL2 页）
 * ------------------------------------------------------------------ */
export const WSL_DETECTION = {
  detected: true,
  wsl_exe_found: true,
  socks_port: 17801,
  gateway_reachable_from_wsl: false,
  recommended_proxy: null,
  note:
    "检测到 2 个发行版，但均无法从 WSL 内连到本机 127.0.0.1:17801。"
    + "这符合 NAT 模式的预期行为：WSL 有独立虚拟网卡，其 127.0.0.1 指向 WSL 自身。"
    + "如需互通，请在 %USERPROFILE%\\.wslconfig 中设置 networkingMode=mirrored 后执行 wsl --shutdown。",
  distros: [
    {
      name: "Ubuntu-24.04",
      state: "Running",
      version: "2",
      is_default: true,
      kernel: "5.15.153.1-microsoft-standard-WSL2",
      reachable_targets: [
        { target: "127.0.0.1", role: "loopback", reachable: false },
        { target: "172.28.16.1", role: "resolv_conf", reachable: false },
        { target: "172.28.0.1", role: "gateway", reachable: false },
      ],
    },
    {
      name: "Debian",
      state: "Stopped",
      version: "2",
      is_default: false,
      kernel: null,
      reachable_targets: [],
    },
  ],
};

export const WSL_CMD = {
  host: "127.0.0.1",
  port: 17801,
  inject_command:
    "export ALL_PROXY='socks5h://172.28.16.1:17801'\n"
    + "export HTTPS_PROXY=\"$ALL_PROXY\"\n"
    + "export HTTP_PROXY=\"$ALL_PROXY\"\n"
    + "export NO_PROXY='127.0.0.1,localhost,::1'",
  selfcheck_command:
    "echo \"本机出口: $(curl -s --max-time 8 https://api.ipify.org)\"\n"
    + "echo \"网关出口: $(curl -s --max-time 8 --proxy \"$ALL_PROXY\" https://api.ipify.org)\"",
  note:
    "两条出口 IP 不同，才能说明 WSL 内的流量确实经过了网关。"
    + "该命令只影响当前 shell 会话，不写入任何配置文件。",
};

/* ------------------------------------------------------------------ *
 * 网络诊断报告（NetworkDiag 页）
 * ------------------------------------------------------------------ */
const T = "2026-09-21 08:45:01";
const item = (
  key: string, label: string, status: string, detail: string, latency: number | null = null
) => ({ key, label, status, detail, latency_ms: latency, ts: T });

export const DIAG_REPORT = {
  started_at: T,
  finished_at: "2026-09-21 08:45:06",
  duration_ms: 4820,
  gateway_ready: true,
  tunnel_status: "ok",
  tunnel_items: [
    item("ssh_process", "SSH 子进程", "ok", "PID 24316 存活", 12),
    item("socks_handshake", "SOCKS5 握手", "ok", "127.0.0.1:17801 可建立连接", 34),
    item("port_listen", "本地端口监听", "ok", "127.0.0.1:17801 LISTENING", 4),
  ],
  egress: {
    local_ip: "198.51.100.22",
    local_version: "IPv4",
    local_source: "https://api.ipify.org",
    gateway_ip: "203.0.113.47",
    gateway_version: "IPv4",
    gateway_source: "https://api.ipify.org（经 SOCKS5 远端 DNS）",
    expected_ip: "203.0.113.47",
    match_result: "matched",
    items: [
      item("egress_gateway", "网关出口 IP", "ok", "203.0.113.47", 612),
      item("egress_direct", "本机直连对照", "ok", "198.51.100.22", 188),
      item("egress_match", "预期出口匹配", "ok", "与预期 203.0.113.47 一致", null),
    ],
  },
  server: {
    reachable: true,
    items: [
      item("ssh_banner", "SSH 服务横幅", "ok", "SSH-2.0-OpenSSH_9.6p1 Ubuntu-3ubuntu13.5", 96),
      item("auth", "认证方式", "ok", "publickey 可用（BatchMode）", 240),
    ],
  },
  clients: [
    {
      kind: "cli",
      label: "Codex CLI",
      running: true,
      processes: [{ pid: 31008, name: "codex.exe", path: "C:\\Users\\me\\AppData\\Roaming\\npm\\node_modules\\@openai\\codex\\bin\\codex.exe", cmdline: "codex" }],
      routing: "verified",
      evidence: [
        "进程环境变量 HTTPS_PROXY=http://127.0.0.1:17802",
        "Mihomo 连接列表中命中网关组（3 条）",
        "触发测试请求，出口 IP 与网关一致",
      ],
      last_checked: T,
    },
    {
      kind: "desktop",
      label: "Codex Desktop",
      running: true,
      processes: [
        { pid: 12884, name: "Codex.exe", path: "C:\\Users\\me\\AppData\\Local\\Programs\\Codex\\Codex.exe", cmdline: null },
        { pid: 12900, name: "Codex.exe", path: "C:\\Users\\me\\AppData\\Local\\Programs\\Codex\\Codex.exe", cmdline: null },
      ],
      routing: "unverified",
      evidence: [
        "未检测到进程级代理环境变量",
        "TUN 未开启，进程级分流尚未覆盖",
        "触发测试请求未观察到经网关的流量",
      ],
      last_checked: T,
    },
    {
      kind: "ide",
      label: "Codex IDE 插件",
      running: false,
      processes: [],
      routing: "unverified",
      evidence: ["未发现运行中的 Codex IDE 扩展宿主"],
      last_checked: T,
    },
  ],
  mihomo: {
    detected: true,
    running: true,
    tun_enabled: false,
    controller: "127.0.0.1:9097",
    secret_required: true,
    connections_total: 142,
    gateway_matched: 3,
    codex_related: 5,
    detail: "已通过 external-controller 读取只读连接信息",
    items: [
      item("mihomo_controller", "external-controller", "ok", "127.0.0.1:9097 可访问", 18),
      item("mihomo_tun", "TUN 状态", "warn", "未开启，进程级分流未覆盖", null),
    ],
  },
  dns_items: [
    item("dns_local", "本机 DNS 解析", "ok", "8.8.8.8 → 142.250.72.14", 42),
    item("dns_remote", "远端 DNS 解析（经 SOCKS5）", "ok", "由服务器侧解析，无本地泄漏", 118),
  ],
  latencies: {
    ssh_connect_ms: 412,
    socks_handshake_ms: 34,
    gateway_https_ms: 612,
    server_https_ms: 96,
    total_ms: 4820,
  },
  path_hops: [
    { name: "本机", status: "ok", latency_ms: null, detail: "127.0.0.1" },
    { name: "SSH 隧道", status: "ok", latency_ms: 412, detail: "127.0.0.1:17801 (SOCKS5, 远端 DNS)" },
    { name: "云服务器", status: "ok", latency_ms: 96, detail: "vps.example.com:22" },
    { name: "公网出口", status: "ok", latency_ms: 612, detail: "203.0.113.47" },
  ],
  advisories: [
    "TUN 未开启：Codex Desktop 与 IDE 插件的进程级分流尚未覆盖，当前仅 CLI 走网关。",
    "检测到 1 个客户端（Codex Desktop）运行中但路由未验证，请勿据此认为其已受保护。",
  ],
};
