import { reactive } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type {
  GatewaySnapshot, GatewayState, HostKeyInfo, SshEnv,
  LaunchPreview, LaunchResult, LogEntry,
} from "../types";

export const store = reactive({
  snapshot: null as GatewaySnapshot | null,
  hostKey: null as HostKeyInfo | null,
  sshEnv: null as SshEnv | null,
  launchPreview: null as LaunchPreview | null,
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

export async function saveServer(config: Record<string, unknown>): Promise<string> {
  return invoke<string>("save_server_config", { config });
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
    case "ERROR": return "错误";
    default: return "未知";
  }
};

export const stateKind = (s: GatewayState | undefined): "ok" | "warn" | "err" | "info" => {
  switch (s) {
    case "EGRESS_VERIFIED": return "ok";
    case "TUNNEL_READY": return "info";
    case "CONNECTING": case "RECONNECTING": case "DISCONNECTING": return "warn";
    case "DEGRADED": case "ERROR": case "DISCONNECTED": return "err";
    default: return "info";
  }
};
