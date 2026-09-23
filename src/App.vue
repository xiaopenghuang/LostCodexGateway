<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import Dashboard from "./pages/Dashboard.vue";
import Doctor from "./pages/Doctor.vue";
import Server from "./pages/Server.vue";
import Apps from "./pages/Apps.vue";
import NetworkDiag from "./pages/NetworkDiag.vue";
import Wsl from "./pages/Wsl.vue";
import Diagnostics from "./pages/Diagnostics.vue";
import Settings from "./pages/Settings.vue";
import { store, initStore, stateLabel, stateKind } from "./stores/gateway";
import { theme, toggleTheme } from "./stores/theme";

type TabKey =
  | "dashboard"
  | "doctor"
  | "server"
  | "apps"
  | "netdiag"
  | "wsl"
  | "diagnostics"
  | "settings";

/**
 * 导航项。icon 是内联 SVG 的 path 数据（24x24 viewBox、stroke 绘制）。
 * 用内联路径而不是图标字体/图片：无额外请求、可跟随 currentColor 变色、
 * 且离线可用（桌面应用不能依赖 CDN）。
 */
const tabs: { key: TabKey; label: string; icon: string; group: string }[] = [
  {
    key: "dashboard",
    label: "首页",
    group: "网关",
    icon: "M3 12l9-9 9 9M5 10v10h14V10",
  },
  {
    key: "doctor",
    label: "环境自检",
    group: "网关",
    icon: "M9 12l2 2 4-4M12 3l7 4v5c0 4.5-3 8-7 9-4-1-7-4.5-7-9V7z",
  },
  {
    key: "server",
    label: "服务器",
    group: "网关",
    icon: "M4 5h16v5H4zM4 14h16v5H4zM7.5 7.5h.01M7.5 16.5h.01",
  },
  {
    key: "apps",
    label: "应用",
    group: "网关",
    icon: "M4 4h7v7H4zM13 4h7v7h-7zM4 13h7v7H4zM13 13h7v7h-7z",
  },
  {
    key: "netdiag",
    label: "网络诊断",
    group: "诊断",
    icon: "M3 12h4l2.5-6 3 12L15 12h6",
  },
  {
    key: "wsl",
    label: "WSL2",
    group: "诊断",
    icon: "M4 7l8-4 8 4v10l-8 4-8-4zM12 3v18M4 7l8 4 8-4",
  },
  {
    key: "diagnostics",
    label: "日志与导出",
    group: "诊断",
    icon: "M5 3h9l5 5v13H5zM14 3v5h5M8 13h8M8 17h5",
  },
  {
    key: "settings",
    label: "设置",
    group: "系统",
    icon: "M12 15a3 3 0 100-6 3 3 0 000 6zM19 12a7 7 0 00-.1-1.2l2-1.6-2-3.4-2.4 1a7 7 0 00-2-1.2L14 3h-4l-.5 2.6a7 7 0 00-2 1.2l-2.4-1-2 3.4 2 1.6A7 7 0 005 12c0 .4 0 .8.1 1.2l-2 1.6 2 3.4 2.4-1a7 7 0 002 1.2L10 21h4l.5-2.6a7 7 0 002-1.2l2.4 1 2-3.4-2-1.6c.1-.4.1-.8.1-1.2z",
  },
];

const active = ref<TabKey>("dashboard");

const state = computed(() => store.snapshot?.state);
const kind = computed(() => stateKind(state.value));

/** 侧栏底部状态摘要：让用户在任何页面都能看到隧道通不通。 */
const statusTitle = computed(() => stateLabel(state.value));
const statusSub = computed(() => {
  const s = store.snapshot;
  if (!s) return "正在读取状态…";
  if (s.state === "EGRESS_VERIFIED") {
    const ip = s.last_verify?.egress_ip;
    return ip ? `出口 ${ip}` : "出口已验证";
  }
  if (s.state === "TUNNEL_READY") return "隧道已通，出口待验证";
  if (s.state === "SWITCHING") return "正在切换出口服务器…";
  if (s.state === "CONNECTING" || s.state === "RECONNECTING" || s.state === "DISCONNECTING") {
    return "请稍候…";
  }
  if (s.state === "UNCONFIGURED") return "尚未配置服务器";
  if (s.state === "ERROR") return "连接出错";
  return "未连接";
});

/** 「正在做事」的状态需要脉冲动画提示。 */
const pulsing = computed(
  () => state.value === "CONNECTING" || state.value === "RECONNECTING" || state.value === "SWITCHING",
);

/** 侧栏状态卡配色：直接用状态类别，避免出现「文案说有异常、颜色却是灰的」。 */
const statusKind = computed(() => {
  const k = kind.value;
  if (k === "ok") return "ok";
  if (k === "err") return "err";
  if (k === "warn") return "warn";
  return "";
});

/** 按 group 分组渲染导航，避免一长串平铺。 */
const grouped = computed(() => {
  const order: string[] = [];
  const map = new Map<string, typeof tabs>();
  for (const t of tabs) {
    if (!map.has(t.group)) {
      map.set(t.group, []);
      order.push(t.group);
    }
    map.get(t.group)!.push(t);
  }
  return order.map((g) => ({ group: g, items: map.get(g)! }));
});

onMounted(async () => {
  // 供验证脚本判断「首屏初始化真的跑完了」。
  //
  // 用「固定 sleep」等 initStore 是脆的：机器慢一点就 go 早了，脚本会读到半空的
  // 页面，然后误报成「功能坏了」。这里留一个显式信号，脚本轮询它即可。
  // 生产构建里这只是一次无害的属性赋值，不泄露任何数据。
  //
  // ⚠️ 放在 `finally` 里：这是「**可以开始断言了**」的信号，不是「初始化成功」
  // 的信号。写在 `await initStore()` 之后的话，一旦 initStore 抛异常，信号就
  // 永不置位，脚本只能白等满超时窗口再退回固定等待 —— 而那时脚本已经读不到
  // 任何线索了。失败时更该置位，并把错误留在 `__lcfgInitError` 里供脚本打印。
  try {
    await initStore();
  } catch (e) {
    console.error("[app] initStore 失败", e);
    if (typeof window !== "undefined") {
      (window as unknown as Record<string, unknown>).__lcfgInitError = String(e);
    }
  } finally {
    if (typeof window !== "undefined") {
      (window as unknown as Record<string, unknown>).__lcfgReady = true;
    }
  }
});
</script>

<template>
  <div class="shell">
    <aside class="sidebar">
      <div class="sidebar-brand">
        <span class="brand-mark" aria-hidden="true"></span>
        <span class="brand-text">
          <span class="brand-name">LostCodexGateway</span>
          <span class="brand-sub">SSH 出口网关</span>
        </span>
      </div>

      <nav class="nav" aria-label="主导航">
        <template v-for="g in grouped" :key="g.group">
          <div class="nav-group-label">{{ g.group }}</div>
          <button
            v-for="t in g.items"
            :key="t.key"
            :class="['tab', { active: active === t.key }]"
            :aria-current="active === t.key ? 'page' : undefined"
            @click="active = t.key"
          >
            <svg class="tab-icon" viewBox="0 0 24 24" aria-hidden="true">
              <path :d="t.icon" stroke-linecap="round" stroke-linejoin="round" />
            </svg>
            <span>{{ t.label }}</span>
          </button>
        </template>
      </nav>

      <div class="sidebar-foot">
        <button
          class="foot-status"
          :class="statusKind"
          :title="statusSub"
          @click="active = 'dashboard'"
        >
          <span :class="['foot-dot', { pulsing }]" aria-hidden="true"></span>
          <span class="foot-text">
            <span class="foot-label">{{ statusTitle }}</span>
            <span class="foot-sub">{{ statusSub }}</span>
          </span>
        </button>

        <button class="theme-toggle" @click="toggleTheme">
          <svg class="tab-icon" viewBox="0 0 24 24" aria-hidden="true">
            <path
              v-if="theme === 'dark'"
              d="M21 12.8A9 9 0 1111.2 3a7 7 0 009.8 9.8z"
              stroke-linecap="round"
              stroke-linejoin="round"
            />
            <g v-else stroke-linecap="round" stroke-linejoin="round">
              <circle cx="12" cy="12" r="4" />
              <path d="M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4" />
            </g>
          </svg>
          <span>{{ theme === "dark" ? "切换到浅色" : "切换到深色" }}</span>
        </button>
      </div>
    </aside>

    <div class="main">
      <main class="content">
        <Dashboard v-if="active === 'dashboard'" @go-server="active = 'server'" @go-doctor="active = 'doctor'" />
        <Doctor v-else-if="active === 'doctor'" :go-tab="(k: string) => (active = k as TabKey)" />
        <Server v-else-if="active === 'server'" />
        <Apps v-else-if="active === 'apps'" />
        <NetworkDiag v-else-if="active === 'netdiag'" />
        <Wsl v-else-if="active === 'wsl'" />
        <Diagnostics v-else-if="active === 'diagnostics'" />
        <Settings v-else />
      </main>
    </div>
  </div>
</template>
