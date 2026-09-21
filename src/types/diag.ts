// 网络诊断模块类型（与 Rust diagnostics.rs 对齐）
export type DiagStatus = "ok" | "warn" | "error" | "unknown";
export type RoutingStatus = "verified" | "partial" | "unverified" | "anomaly" | "unconfirmable";

export interface DiagItem {
  key: string;
  label: string;
  status: DiagStatus;
  detail: string;
  latency_ms: number | null;
  ts: string;
}

export interface ProcInfo {
  pid: number;
  name: string;
  path: string | null;
  cmdline: string | null;
}

export interface ClientDiag {
  kind: string;
  label: string;
  running: boolean;
  processes: ProcInfo[];
  routing: RoutingStatus;
  evidence: string[];
  last_checked: string;
}

export interface EgressDiag {
  local_ip: string | null;
  local_version: string | null;
  local_source: string | null;
  gateway_ip: string | null;
  gateway_version: string | null;
  gateway_source: string | null;
  expected_ip: string | null;
  match_result: "matched" | "mismatch" | "unconfirmed";
  items: DiagItem[];
}

export interface ServerDiag {
  reachable: boolean;
  items: DiagItem[];
}

export interface MihomoDiag {
  detected: boolean;
  running: boolean;
  tun_enabled: boolean;
  controller: string | null;
  secret_required: boolean;
  connections_total: number;
  gateway_matched: number;
  codex_related: number;
  detail: string;
  items: DiagItem[];
}

export interface LatencyDiag {
  ssh_connect_ms: number | null;
  socks_handshake_ms: number | null;
  gateway_https_ms: number | null;
  server_https_ms: number | null;
  total_ms: number | null;
}

export interface PathHop {
  name: string;
  status: DiagStatus;
  latency_ms: number | null;
  detail: string;
}

export interface DiagReport {
  started_at: string;
  finished_at: string;
  duration_ms: number;
  gateway_ready: boolean;
  tunnel_status: DiagStatus;
  tunnel_items: DiagItem[];
  egress: EgressDiag;
  server: ServerDiag;
  clients: ClientDiag[];
  mihomo: MihomoDiag;
  dns_items: DiagItem[];
  latencies: LatencyDiag;
  path_hops: PathHop[];
  advisories: string[];
}
