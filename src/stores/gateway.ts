import { reactive } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type {
  GatewaySnapshot, GatewayState, GatewayConfig, HostKeyInfo, SshEnv,
  LaunchPreview, LaunchResult, LogEntry, ServerLatency, ServerProfile,
} from "../types";

export const store = reactive({
  snapshot: null as GatewaySnapshot | null,
  hostKey: null as HostKeyInfo | null,
  sshEnv: null as SshEnv | null,
  launchPreview: null as LaunchPreview | null,
  /** 最近一次服务器延迟探测结果（key = 服务器 id） */
  latencies: {} as Record<string, ServerLatency>,
  latencyTesting: false,
  logs: [] as LogEntry[],
  initError: null as string | null,
  inBrowser: false,
});

export async function initStore(): Promise<void> {
  // 开发期「有数据状态」视觉检查：仅在 dev + 指定场景时生效，生产构建会被摇掉
  if (import.meta.env.DEV && import.meta.env.VITE_LCFG_FIXTURE) {
    const { applyFixture } = await import("../dev/apply-fixture");
    applyFixture(import.meta.env.VITE_LCFG_FIXTURE);
    return;
  }
  try {
    store.snapshot = await invoke<GatewaySnapshot>("get_snapshot");
    store.sshEnv = await invoke<SshEnv>("detect_ssh_env");
  } catch (e) {
    // 浏览器 dev 模式（无 Tauri 后端）下给出可读提示
    store.initError = String(e);
  }
  try {
    await listen<GatewaySnapshot>("gateway://snapshot", (ev) => {
      store.snapshot = ev.payload;
    });
    await listen<LogEntry>("gateway://log", (ev) => {
      store.logs.push(ev.payload);
      if (store.logs.length > 500) store.logs.splice(0, store.logs.length - 500);
    });
  } catch {
    /* 事件注册失败时 UI 仍可用（轮询快照） */
  }
}

/** 当前选中的服务器（配置里查不到时返回 null，UI 需容忍）。 */
export function activeServer(cfg: GatewayConfig | null | undefined): ServerProfile | null {
  if (!cfg) return null;
  return cfg.servers.find((s) => s.id === cfg.active_server_id) ?? null;
}

/**
 * 保存一台服务器。
 * - `id` 为空 → 后端新增并自动设为当前选中
 * - `id` 非空 → 按 id 更新
 */
export async function saveServer(server: Partial<ServerProfile>): Promise<string> {
  return invoke<string>("save_server", { server });
}

/** 删除一台服务器；若删的正是当前连接中的那台，后端会先干净断开。 */
export async function deleteServer(id: string): Promise<string> {
  return invoke<string>("delete_server", { id });
}

/**
 * 切换到另一台服务器（硬切：断开 → 改选中 → 重连）。
 * 失败不回滚：后端会如实报错并保留 `previous_server_id`，用户可一键切回。
 */
export async function switchServer(id: string): Promise<string> {
  return invoke<string>("switch_server", { id });
}

/** 对所有服务器做只读延迟探测（并发，5s 超时）。不是出口延迟。 */
export async function testServers(): Promise<ServerLatency[]> {
  store.latencyTesting = true;
  try {
    const list = await invoke<ServerLatency[]>("test_servers");
    const map: Record<string, ServerLatency> = {};
    for (const item of list) map[item.id] = item;
    store.latencies = map;
    return list;
  } finally {
    store.latencyTesting = false;
  }
}

/** 保存全局设置（本地端口 + 重连策略）。隧道运行期间后端会拒绝改端口。 */
export async function saveSettings(payload: {
  socks_port: number;
  bridge_port: number;
  auto_reconnect: boolean;
  max_reconnect_attempts: number;
}): Promise<string> {
  return invoke<string>("save_settings", payload);
}

/** 配置某台服务器的预期出口 IP（`serverId` 为空则作用于当前选中项）。 */
export async function setExpectedEgressIp(serverId: string, ip: string | null): Promise<string> {
  return invoke<string>("set_expected_egress_ip", { serverId, ip });
}

export async function connect(): Promise<string> {
  return invoke<string>("connect");
}

export async function disconnect(): Promise<string> {
  return invoke<string>("disconnect");
}

export async function testConnection(): Promise<string> {
  return invoke<string>("test_connection");
}

export async function fetchHostKey(): Promise<HostKeyInfo> {
  store.hostKey = await invoke<HostKeyInfo>("fetch_host_key");
  return store.hostKey;
}

export async function confirmHostKey(): Promise<string> {
  return invoke<string>("confirm_host_key");
}

export async function getLaunchPreview(): Promise<LaunchPreview> {
  store.launchPreview = await invoke<LaunchPreview>("get_launch_preview");
  return store.launchPreview;
}

export async function launchCodexCli(): Promise<LaunchResult> {
  return invoke<LaunchResult>("launch_codex_cli");
}

export async function exportDiagnostics(): Promise<string> {
  return invoke<string>("export_diagnostics");
}

export async function refreshSnapshot(): Promise<void> {
  try {
    store.snapshot = await invoke<GatewaySnapshot>("get_snapshot");
  } catch {
    /* ignore */
  }
}

export const stateLabel = (s: GatewayState | undefined): string => {
  switch (s) {
    case "UNCONFIGURED": return "未配置";
    case "READY": return "就绪";
    case "CONNECTING": return "连接中";
    case "TUNNEL_READY": return "隧道已通";
    case "EGRESS_VERIFIED": return "出口已验证";
    case "DEGRADED": return "降级";
    case "RECONNECTING": return "重连中";
    case "DISCONNECTING": return "断开中";
    case "DISCONNECTED": return "已断开";
    case "SWITCHING": return "切换服务器中";
    case "ERROR": return "错误";
    default: return "未知";
  }
};

export const stateKind = (s: GatewayState | undefined): "ok" | "warn" | "err" | "info" => {
  switch (s) {
    case "EGRESS_VERIFIED": return "ok";
    case "TUNNEL_READY": return "info";
    case "CONNECTING": case "RECONNECTING": case "DISCONNECTING": case "SWITCHING": return "warn";
    case "DEGRADED": case "ERROR": case "DISCONNECTED": return "err";
    default: return "info";
  }
};

/** 隧道处于活跃期（有或即将有本工具创建的 ssh 子进程）。 */
export const tunnelIsActive = (s: GatewayState | undefined): boolean =>
  s === "CONNECTING" || s === "TUNNEL_READY" || s === "EGRESS_VERIFIED" ||
  s === "DEGRADED" || s === "RECONNECTING" || s === "SWITCHING";
