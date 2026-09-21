/**
 * 主题（深色 / 浅色）与持久化。
 *
 * 设计取舍：本工具是纯本地桌面应用，主题偏好属于「无敏感性」的 UI 状态，
 * 因此直接写入 localStorage 而不走后端配置——避免为了一个视觉开关去动
 * config.json 的 schema 与迁移逻辑。前端加载时同步读取，避免闪烁。
 */

import { ref, watch } from "vue";

export type Theme = "dark" | "light";

const STORAGE_KEY = "lcfg-theme";

/** 读取初始主题：已保存的偏好 > 跟随系统 > 深色兜底。 */
function initialTheme(): Theme {
  try {
    const saved = localStorage.getItem(STORAGE_KEY);
    if (saved === "dark" || saved === "light") return saved;
    // 未设置过则跟随系统；本工具偏工具类，深色是更常见的预期
    if (window.matchMedia?.("(prefers-color-scheme: light)").matches) return "light";
    return "dark";
  } catch {
    // localStorage 不可用（隐私模式等）：退回深色，不影响功能
    return "dark";
  }
}

export const theme = ref<Theme>(initialTheme());

/** 把主题写到 <html data-theme> 上，CSS 通过属性选择器切换令牌。 */
function applyTheme(t: Theme) {
  document.documentElement.setAttribute("data-theme", t);
}

export function setTheme(t: Theme) {
  theme.value = t;
}

export function toggleTheme() {
  theme.value = theme.value === "dark" ? "light" : "dark";
}

watch(
  theme,
  (t) => {
    applyTheme(t);
    try {
      localStorage.setItem(STORAGE_KEY, t);
    } catch {
      // 存储失败不影响本次会话的主题切换
    }
  },
  { immediate: true },
);
