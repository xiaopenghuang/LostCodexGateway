// WSL2 专项支持类型（与 Rust wsl.rs 对齐）
export interface WslTarget {
  target: string;
  /** "loopback" | "resolv_conf" | "gateway" */
  role: string;
  reachable: boolean;
}

export interface WslDistro {
  name: string;
  state: string;
  version: string;
  is_default: boolean;
  kernel: string | null;
  reachable_targets: WslTarget[];
}

export interface WslDetection {
  detected: boolean;
  wsl_exe_found: boolean;
  distros: WslDistro[];
  socks_port: number;
  /** 结论：WSL2 是否真的能连到本工具网关（以真实建连为判据） */
  gateway_reachable_from_wsl: boolean;
  recommended_proxy: string | null;
  note: string;
}

export interface WslProxyCommand {
  host: string;
  port: number;
  inject_command: string;
  selfcheck_command: string;
  note: string;
}

export const roleLabel = (role: string): string => {
  switch (role) {
    case "loopback": return "回环（mirrored 模式）";
    case "resolv_conf": return "宿主 IP（resolv.conf）";
    case "gateway": return "默认网关";
    default: return role;
  }
};
