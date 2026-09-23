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
  | "SWITCHING"
  | "ERROR";

/**
 * 一台服务器。与 Rust 端 `ServerProfile` 一一对应。
 *
 * 注意这里**没有** `socks_port`：本地端口是全局设置（`AppSettings`），
 * 不属于任何一台服务器——切换服务器时端口必须保持不变，否则下游 Codex CLI
 * 的 `HTTP_PROXY` 会指向空气。
 */
export interface ServerProfile {
  /** 稳定标识；一旦生成不再变化，`active_server_id` 靠它关联。新建时传空串。 */
  id: string;
  /** 显示名（可空，空则回退 host）。 */
  name: string;
  host: string;
  port: number;
  username: string;
  key_path: string;
  ssh_exe_path: string;
  /** 该服务器**预期**的出口 IP（空 = 不校验）。每台独立。 */
  expected_egress_ip: string;
  /** Mihomo 规则里对应的代理组名（诊断关联用）。 */
  gateway_group: string;
}

export interface VerifyConfig {
  endpoints: string[];
  timeout_secs: number;
}

export interface AppSettings {
  auto_reconnect: boolean;
  max_reconnect_attempts: number;
  /** 后端为自由 String（默认 "warn_and_block"），此处不得收窄为字面量类型 */
  disconnect_policy: string;
  /** 本地 SOCKS5 入口端口（全局，切换服务器时不变） */
  socks_port: number;
  /** HTTP CONNECT 桥接入口端口（全局，切换服务器时不变） */
  bridge_port: number;
  gateway_group: string;
}

export interface GatewayConfig {
  servers: ServerProfile[];
  /** 当前选中的服务器 id */
  active_server_id: string;
  verify: VerifyConfig;
  settings: AppSettings;
}

/** 单台服务器的只读延迟探测结果。 */
export interface ServerLatency {
  id: string;
  name: string;
  host: string;
  port: number;
  reachable: boolean;
  /** TCP 握手耗时（毫秒）。**不是出口延迟**，只是到服务器 SSH 端口的网络往返。 */
  latency_ms: number | null;
  error: string | null;
}

export interface VerifyStep {
  kind: string;
  label: string;
  ok: boolean;
  detail: string;
  timestamp: string;
  /**
   * 该步骤是否只是**参考信息**，不参与整体 `ok` 判定。
   *
   * 前端据此避免把辅助步骤的失败渲染成「错误」—— 否则会出现
   * 「红色 ✗ + 徽标写『全部通过』」的矛盾观感（用户实测反馈过）。
   * 后端 `verify.rs` 的 `advisory` 字段，旧快照可能缺该字段，故用可选。
   */
  advisory?: boolean;
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
  /** 切换前的服务器 id（供「切回上一个」用）；未发生过切换时为 null。 */
  previous_server_id: string | null;
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
