// 与 Rust 端 `preflight.rs` 对齐的类型定义（经 Tauri command JSON 序列化）

/** 单项检查的结果状态。 */
export type PreflightStatus = "ok" | "warn" | "error" | "unknown";

/**
 * 「这一项要不要用户动手」以及「怎么动手」。
 *
 * - `none`：无需操作
 * - `auto`：界面内有按钮可自动完成（`action` 给出目标页）
 * - `guide`：必须人工操作，`steps` 给出逐条步骤
 */
export type FixKind = "none" | "auto" | "guide";

/** 环境自检项。 */
export interface PreflightItem {
  /** 稳定标识（前端据此决定渲染哪个按钮 / 跳哪个页面） */
  key: string;
  label: string;
  status: PreflightStatus;
  detail: string;
  fix: FixKind;
  /** `fix === "auto"` 时的前端动作标识；`guide` 时为 null。 */
  action: string | null;
  /** 操作步骤（`fix === "guide"` 时非空），按顺序展示。 */
  steps: string[];
  /** 是否为阻塞项：为 true 且未通过时，软件**无法**正常使用。 */
  blocking: boolean;
}

/** 环境自检总报告。 */
export interface PreflightReport {
  items: PreflightItem[];
  passed: number;
  blocking_failed: number;
  ready: boolean;
  summary: string;
  ts: string;
}
