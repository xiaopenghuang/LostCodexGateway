// 开机启动类型（与 Rust autostart.rs 对齐）
export interface AutostartStatus {
  enabled: boolean;
  registered_command: string | null;
  exe_path: string | null;
  /** 注册表条目存在但指向其他程序（异常，不会自动删除） */
  points_to_other: boolean;
  note: string;
}
