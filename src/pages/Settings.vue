<script setup lang="ts">
import { onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { store, initStore } from "../stores/gateway";
import { theme, toggleTheme } from "../stores/theme";
import type { AutostartStatus } from "../types/autostart";

const autostart = ref(null as AutostartStatus | null);
const autostartBusy = ref(false);
const autostartMsg = ref("");

onMounted(async () => {
  await initStore();
  await loadAutostart();
});

async function loadAutostart() {
  try {
    autostart.value = await invoke<AutostartStatus>("get_autostart_status");
  } catch (e) {
    autostartMsg.value = String(e);
  }
}

async function toggleAutostart() {
  if (!autostart.value) return;
  autostartBusy.value = true;
  autostartMsg.value = "";
  try {
    autostartMsg.value = await invoke<string>("set_autostart", {
      enabled: !autostart.value.enabled,
    });
    await loadAutostart();
  } catch (e) {
    autostartMsg.value = String(e);
  } finally {
    autostartBusy.value = false;
  }
}
</script>

<template>
  <div>
    <div class="page-head">
      <div>
        <h1 class="page-title">设置</h1>
        <p class="page-desc">界面外观、启动行为与安全策略说明。</p>
      </div>
    </div>

    <div class="card">
      <h2>外观</h2>
      <div class="kv">
        <span class="k">主题</span>
        <span class="v">{{ theme === "dark" ? "深色" : "浅色" }}</span>
      </div>
      <div class="row" style="margin-top: 12px">
        <button class="btn secondary" @click="toggleTheme">
          切换到{{ theme === "dark" ? "浅色" : "深色" }}
        </button>
      </div>
      <p class="muted">
        主题偏好保存在本机浏览器存储中，不写入服务器配置，也不随诊断报告导出。
      </p>
    </div>

    <div class="card">
      <h2>
        开机启动
        <span :class="['status-pill', autostart?.enabled ? 'ok' : 'info']">
          {{ autostart ? (autostart.enabled ? "已启用" : "未启用") : "读取中…" }}
        </span>
      </h2>
      <p class="muted">
        <b>开机启动不会自动连接隧道。</b>启用后程序随登录驻留到系统托盘，隧道仍需你在「首页」显式点击连接——
        静默建立代理出口会让你失去对网络出口的知情权。
      </p>
      <div v-if="autostart?.exe_path" class="stat-grid">
        <div class="stat">
          <span class="stat-label">当前程序</span>
          <span class="stat-value dim">{{ autostart.exe_path }}</span>
        </div>
      </div>
      <p v-if="autostart?.registered_command" class="muted mono" style="margin-top: 10px">
        注册表条目：{{ autostart.registered_command }}
      </p>
      <p v-if="autostart?.note" class="muted">{{ autostart.note }}</p>
      <div class="row" style="margin-top: 12px">
        <button
          class="btn"
          :disabled="autostartBusy || !autostart || autostart.points_to_other"
          @click="toggleAutostart"
        >
          {{ autostart?.enabled ? "关闭开机启动" : "启用开机启动" }}
        </button>
        <span v-if="autostartMsg" class="muted mono">{{ autostartMsg }}</span>
      </div>
      <p v-if="autostart?.points_to_other" class="notice">
        检测到注册表条目指向其他程序：为安全起见本工具不会自动删除，请自行确认后再处理。
      </p>
    </div>

    <div class="card">
      <h2>单实例行为</h2>
      <p class="muted">
        本程序同一登录会话只允许运行一个实例。当你重复启动（例如开机自启后手动双击图标，
        或从「开始菜单」再次打开）时，<b>新进程不会建立第二条隧道、也不会占用本地端口</b>，
        而是把已经在运行的主窗口唤到前台，然后自身立即退出。
      </p>
      <p class="muted">
        这样保证同一时刻只有一份隧道状态机在管理 SSH 连接。若你希望同时开两份互不干扰的配置，
        需使用不同的 Windows 登录账号（守卫作用于当前登录会话）。
      </p>
    </div>

    <div class="card">
      <h2>断线与安全策略</h2>
      <div class="stat-grid">
        <div class="stat">
          <span class="stat-label">断线策略</span>
          <span class="stat-value dim">停止新请求并警告（非透明直连）</span>
        </div>
        <div class="stat">
          <span class="stat-label">自动重连</span>
          <span class="stat-value">{{ store.snapshot?.config?.settings?.auto_reconnect ? "开启（有限次数）" : "关闭" }}</span>
        </div>
        <div class="stat">
          <span class="stat-label">最大重连次数</span>
          <span class="stat-value">{{ store.snapshot?.config?.settings?.max_reconnect_attempts ?? 3 }}</span>
        </div>
      </div>
      <p class="muted" style="margin-top: 12px">
        隧道掉线时界面立即显示「已断开」，不会继续展示旧出口 IP，也不会悄悄退回直连后假装「已保护」。
      </p>
    </div>

    <div class="card">
      <h2>高级网络集成</h2>
      <div class="notice info">
        Mihomo / Clash Verge 规则集成默认关闭（M3 阶段提供检测与受控操作）。
        本工具不会修改系统路由表、不接管全局流量、不需要管理员权限。
      </div>
    </div>

    <div class="card">
      <h2>隐私与安全说明</h2>
      <ul class="muted" style="line-height: 2; padding-left: 18px; margin: 0">
        <li>SSH 私钥只保存路径，不复制、不导出、不写入日志。</li>
        <li>本地 SOCKS / HTTP 代理只监听 127.0.0.1，不向局域网开放。</li>
        <li>不保存 OpenAI OAuth、Cookie、API Key，不读取 Codex 认证目录。</li>
        <li>日志默认只记录时间、组件、连接状态、错误类别与脱敏地址。</li>
        <li>独立网络出口不改变你的账号地区或服务条款，也不提供绕过限制的功能。</li>
      </ul>
    </div>
  </div>
</template>
