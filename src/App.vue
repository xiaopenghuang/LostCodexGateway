<script setup lang="ts">
import { ref } from "vue";
import Dashboard from "./pages/Dashboard.vue";
import Server from "./pages/Server.vue";
import Apps from "./pages/Apps.vue";
import NetworkDiag from "./pages/NetworkDiag.vue";
import Wsl from "./pages/Wsl.vue";
import Diagnostics from "./pages/Diagnostics.vue";
import Settings from "./pages/Settings.vue";

type TabKey = "dashboard" | "server" | "apps" | "netdiag" | "wsl" | "diagnostics" | "settings";
const tabs: { key: TabKey; label: string }[] = [
  { key: "dashboard", label: "首页" },
  { key: "server", label: "服务器" },
  { key: "apps", label: "应用" },
  { key: "netdiag", label: "网络诊断" },
  { key: "wsl", label: "WSL2" },
  { key: "diagnostics", label: "诊断" },
  { key: "settings", label: "设置" },
];
const active = ref<TabKey>("dashboard");
</script>

<template>
  <div class="shell">
    <header class="topbar">
      <div class="brand">
        <span class="brand-dot"></span>
        <span>LostCodexGateway</span>
      </div>
      <nav class="tabs">
        <button
          v-for="t in tabs"
          :key="t.key"
          :class="['tab', { active: active === t.key }]"
          @click="active = t.key"
        >
          {{ t.label }}
        </button>
      </nav>
    </header>
    <main class="content">
      <Dashboard v-if="active === 'dashboard'" @go-server="active = 'server'" />
      <Server v-else-if="active === 'server'" />
      <Apps v-else-if="active === 'apps'" />
      <NetworkDiag v-else-if="active === 'netdiag'" />
      <Wsl v-else-if="active === 'wsl'" />
      <Diagnostics v-else-if="active === 'diagnostics'" />
      <Settings v-else />
    </main>
  </div>
</template>
