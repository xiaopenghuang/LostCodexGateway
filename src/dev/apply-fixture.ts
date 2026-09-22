/**
 * 把假快照灌进 store，供开发期视觉检查使用。
 *
 * 同时把 `invoke` 换成假实现——否则各页面的 `onMounted` 会调用真实
 * Tauri command 并抛错，把页面刷成红色错误条，看不到有数据的样子。
 */
import { store } from "../stores/gateway";
import { invoke } from "@tauri-apps/api/core";
import {
  fixtureSnapshot, SCENARIO_NAMES, SSH_ENV, HOST_KEY, LAUNCH_PREVIEW, LATENCIES,
  MIHOMO, WSL_DETECTION, WSL_CMD, DIAG_REPORT,
} from "./fixtures";

/** 各 command 的假返回值；未列出的返回一句可读提示。 */
function fakeInvoke(cmd: string, args?: Record<string, unknown>): unknown {
  switch (cmd) {
    case "get_snapshot":
      return store.snapshot;
    case "detect_ssh_env":
      return SSH_ENV;
    case "save_server":
      return "已保存服务器（假数据，仅用于视觉检查）";
    case "delete_server":
      return "已删除服务器（假数据）";
    case "switch_server":
      return "已切换到目标服务器（假数据）";
    case "test_servers":
      // 同步写进 store，让列表里的延迟列有东西可渲染
      for (const l of LATENCIES) store.latencies[l.id] = l;
      return LATENCIES;
    case "save_settings":
      return "已保存设置（假数据）";
    case "test_connection":
      return "连接成功：SSH 握手通过，SOCKS5 转发可用（假数据）";
    case "connect":
      return "已发起连接（假数据）";
    case "disconnect":
      return "已断开（假数据）";
    case "fetch_host_key":
      store.hostKey = HOST_KEY;
      return HOST_KEY;
    case "confirm_host_key":
      return "已写入系统 known_hosts（假数据）";
    case "get_launch_preview":
      store.launchPreview = LAUNCH_PREVIEW;
      return LAUNCH_PREVIEW;
    case "launch_codex_cli":
      return { started: true, message: "已启动 Codex CLI（假数据）", pid: 31008 };
    case "export_diagnostics":
      return "C:\\Users\\me\\AppData\\Local\\LostCodexGateway\\diag-20260921.json（假数据）";
    case "get_autostart_status":
      return {
        enabled: true,
        exe_path: "C:\\Users\\me\\AppData\\Local\\Programs\\LostCodexGateway\\lostcodexgateway.exe",
        registered_command: '"C:\\Users\\me\\AppData\\Local\\Programs\\LostCodexGateway\\lostcodexgateway.exe" --autostart',
        points_to_other: false,
        note: "随登录启动并驻留到托盘；不会自动连接隧道。",
      };
    case "set_autostart":
      return "开机启动已更新（假数据）";
    case "get_last_diagnostics":
    case "run_diagnostics":
      return DIAG_REPORT;
    case "diagnose_client":
      return DIAG_REPORT.clients.find((c) => c.kind === (args?.kind as string)) ?? null;
    case "set_expected_egress_ip":
      return "已保存预期出口 IP（假数据）";
    case "detect_mihomo":
      return MIHOMO;
    case "detect_wsl":
      return WSL_DETECTION;
    case "get_wsl_proxy_command":
      return WSL_CMD;
    default:
      if (import.meta.env.DEV) {
        console.warn("[fixture] 未覆盖的 command:", cmd, args);
      }
      return null;
  }
}

export function applyFixture(name: string): void {
  const snap = fixtureSnapshot(name);
  if (!snap) {
    console.error(
      `[fixture] 未知场景 "${name}"。可用：${SCENARIO_NAMES.join(" / ")}`
    );
    return;
  }

  store.snapshot = snap;
  store.sshEnv = SSH_ENV;
  store.hostKey = HOST_KEY;
  store.initError = null;
  store.inBrowser = false;
  store.logs = snap.recent_logs ?? [];

  // 覆盖 window.__TAURI_INTERNALS__ 里 invoke 的实现
  const internals = (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
  if (internals && typeof internals === "object") {
    (internals as Record<string, unknown>).invoke = (
      cmd: string,
      args?: Record<string, unknown>
    ) => Promise.resolve(fakeInvoke(cmd, args));
  } else {
    // 纯浏览器（无 Tauri 注入）下直接给 invoke 打桩
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {
      invoke: (cmd: string, args?: Record<string, unknown>) =>
        Promise.resolve(fakeInvoke(cmd, args)),
      transformCallback: (cb: unknown) => cb,
    };
  }

  // 兜底：某些路径会直接用模块导出的 invoke，这里也替掉
  try {
    (invoke as unknown as { __mock?: boolean }).__mock = true;
  } catch {
    /* 只读绑定，忽略 */
  }

  console.info(`[fixture] 已注入场景「${name}」，state=${snap.state}`);
}

// 供验证脚本在**同一个页面**里遍历多个场景。
//
// 没有这个入口就只能「一个场景重启一次 dev server」：遍历 6 个场景要重启 6 次，
// 成本高到没人愿意跑——而「逐状态核对按钮可用性」这类检查恰恰需要遍历。
// 本模块只在 dev + 设置了 VITE_LCFG_FIXTURE 时被动态导入，
// 生产构建里整块会被摇掉（已用二进制标记扫描验证过）。
if (typeof window !== "undefined") {
  const w = window as unknown as Record<string, unknown>;
  w.__applyFixture = applyFixture;
  w.__fixtureNames = SCENARIO_NAMES;
}
