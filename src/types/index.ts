// 与 Rust 端 state.rs / config.rs 对齐的类型定义（经 Tauri command JSON 序列化）

export type GatewayState =
  | "UNCONFIGURED"
  | "READY"
  | "CONNECTING"
  | "TUNNEL_READY"
  | "EGRESS_VERIFIED"
  | "DEGRADED"
  | "RECONNECTING"
  | "DISCONNECTING"
  | "DISCONNECTED"
  | "ERROR";

export interface ServerConfig {
  host: string;
  port: number;
  username: string;
  key_path: string;
  socks_port: number;
  ssh_exe_path: string;
  server_name: string;
}

export interface VerifyConfig {
  endpoints: string[];
  timeout_secs: number;
  expected_egress_ip: string;
}

export interface AppSettings {
  auto_reconnect: boolean;
  max_reconnect_attempts: number;
  /** 后端为自由 String（默认 "warn_and_block"），此处不得收窄为字面量类型 */
  disconnect_policy: string;
  proxy_mode: string;
  gateway_group: string;
}

export interface GatewayConfig {
  server: ServerConfig;
  verify: VerifyConfig;
  settings: AppSettings;
}

export interface VerifyStep {
  kind: string;
  label: string;
  ok: boolean;
  detail: string;
  timestamp: string;
}

export interface VerifyResult {
  ok: boolean;
  egress_ip: string | null;
  steps: VerifyStep[];
  started_at: string;
  finished_at: string;
}

export interface HostKeyInfo {
  known: boolean;
  fingerprint: string | null;
  key_type: string | null;
}

export interface LogEntry {
  ts: string;
  level: "info" | "warn" | "error";
  component: string;
  message: string;
}

/** 桥接层拒绝连接的原因（与 Rust 端 RejectReason::code() 一一对应） */
export interface BridgeRejectRecord {
  code: string;
  message: string;
  target: string;
}

export interface GatewaySnapshot {
  state: GatewayState;
  config: GatewayConfig | null;
  last_verify: VerifyResult | null;
  ssh_pid: number | null;
  last_error: string | null;
  recent_logs: LogEntry[];
  bridge_port: number | null;
  bridge_connections_total: number;
  bridge_last_target: string | null;
  bridge_rejects_total: number;
  bridge_recent_rejects: BridgeRejectRecord[];
}

export interface SshEnv {
  exists: boolean;
  path: string;
  version: string;
}

export interface LaunchPreview {
  command: string;
  env: Record<string, string>;
  proxy_line: string;
  bridge_needed: boolean;
}

export interface LaunchResult {
  started: boolean;
  message: string;
  pid: number | null;
}
