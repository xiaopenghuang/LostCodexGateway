<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import {
  store, initStore, connect, disconnect, stateLabel, stateKind,
} from "../stores/gateway";

defineEmits<{ (e: "go-server"): void }>();

const snap = computed(() => store.snapshot);
const state = computed(() => snap.value?.state);
const verify = computed(() => snap.value?.last_verify);
const configured = computed(() => !!snap.value?.config?.server?.host);

/** 是否处于「可断开」的活跃态。 */
const canDisconnect = computed(() =>
  state.value === "EGRESS_VERIFIED"
  || state.value === "TUNNEL_READY"
  || state.value === "DEGRADED",
);

const busy = computed(() =>
  state.value === "CONNECTING"
  || state.value === "RECONNECTING"
  || state.value === "DISCONNECTING",
);

const directIp = computed(
  () => verify.value?.steps.find((s) => s.kind === "direct_ip")?.detail ?? null,
);
const tunnelIp = computed(() => verify.value?.egress_ip);

/** 主状态区的视觉类别：决定光球颜色。 */
const heroKind = computed(() => {
  const k = stateKind(state.value);
  if (k === "ok") return "ok";
  if (k === "err") return "err";
  if (busy.value || state.value === "TUNNEL_READY" || state.value === "DEGRADED") return "warn";
  return "";
});

/** 出口 IP 是否与直连相同——相同的「已验证」是有疑问的，需要明确提示。 */
const egressSuspect = computed(
  () => !!tunnelIp.value && !!directIp.value && tunnelIp.value === directIp.value,
);

const actionMsg = ref("");
const acting = ref(false);

async function doConnect() {
  acting.value = true;
  actionMsg.value = "";
  try {
    actionMsg.value = await connect();
  } catch (e) {
    actionMsg.value = String(e);
  } finally {
    acting.value = false;
  }
}

async function doDisconnect() {
  acting.value = true;
  actionMsg.value = "";
  try {
    actionMsg.value = await disconnect();
  } catch (e) {
    actionMsg.value = String(e);
  } finally {
    acting.value = false;
  }
}

onMounted(async () => {
  await initStore();
});
</script>

<template>
  <div>
    <div class="page-head">
      <div>
        <h1 class="page-title">连接状态</h1>
        <p class="page-desc">管理本机到自有服务器的 SSH 隧道，并验证出口路径。</p>
      </div>
      <span :class="['status-pill', stateKind(state)]">{{ stateLabel(state) }}</span>
    </div>

    <div class="card">
      <div class="hero">
        <div :class="['hero-orb', heroKind]" aria-hidden="true"></div>
        <div class="hero-body">
          <div class="hero-title">{{ stateLabel(state) }}</div>
          <div class="hero-sub">
            <template v-if="!configured">尚未配置服务器，请先前往「服务器」页填写 SSH 信息。</template>
            <template v-else-if="state === 'EGRESS_VERIFIED'">
              隧道与出口均已验证，可以放心启动 Codex CLI。
            </template>
            <template v-else-if="state === 'TUNNEL_READY'">
              隧道已建立，但出口尚未通过实测验证。
            </template>
            <template v-else-if="busy">正在处理，请稍候…</template>
            <template v-else-if="state === 'DEGRADED'">隧道可用但出口验证未通过，请查看下方步骤。</template>
            <template v-else-if="state === 'ERROR'">连接出错，详情见下方日志或「日志与导出」页。</template>
            <template v-else>隧道未运行。点击「连接」建立到服务器的出口。</template>
          </div>
        </div>
        <div class="row" style="flex: 0 0 auto">
          <button class="btn" :disabled="!configured || busy || acting" @click="doConnect">
            连接
          </button>
          <button class="btn danger" :disabled="!canDisconnect || acting" @click="doDisconnect">
            断开
          </button>
        </div>
      </div>

      <p v-if="actionMsg" class="muted" style="margin-top: 12px">{{ actionMsg }}</p>

      <template v-if="configured">
        <div class="stat-grid">
          <div class="stat">
            <span class="stat-label">目标服务器</span>
            <span class="stat-value">{{ snap?.config?.server?.server_name || "（未命名）" }}</span>
          </div>
          <div class="stat">
            <span class="stat-label">地址</span>
            <span class="stat-value">{{ snap?.config?.server?.username }}@{{ snap?.config?.server?.host }}:{{ snap?.config?.server?.port }}</span>
          </div>
          <div class="stat">
            <span class="stat-label">本地 SOCKS 端口</span>
            <span class="stat-value">127.0.0.1:{{ snap?.config?.server?.socks_port }}</span>
          </div>
        </div>
      </template>

      <template v-else>
        <div class="notice info">
          尚未配置服务器。本工具需要一台你能 SSH 登录的服务器来建立出口。
        </div>
        <button class="btn" @click="$emit('go-server')">前往配置</button>
      </template>
    </div>

    <div class="card">
      <h2>出口 IP 对照</h2>
      <div class="stat-grid">
        <div class="stat">
          <span class="stat-label">隧道出口（经 SOCKS 实测）</span>
          <span :class="['stat-value', tunnelIp ? 'accent' : 'dim']">{{ tunnelIp ?? "未验证" }}</span>
        </div>
        <div class="stat">
          <span class="stat-label">本机直连对照</span>
          <span :class="['stat-value', directIp ? '' : 'dim']">{{ directIp ?? "未检测" }}</span>
        </div>
      </div>
      <div v-if="egressSuspect" class="notice">
        <p>
          隧道出口与本机直连出口<b>相同</b>。这通常意味着出口路径重叠，
          并不能证明流量确实经过了服务器。请前往「网络诊断」进一步核查。
        </p>
      </div>
      <p v-else class="muted" style="margin-top: 12px">
        两者不同才能说明流量确实经由服务器出口。本机对照只用于比对，不影响你的其它应用。
      </p>
    </div>

    <div v-if="verify" class="card">
      <h2>
        出口验证步骤
        <span :class="['status-pill', verify.ok ? 'ok' : 'err']">
          {{ verify.ok ? "全部通过" : "未通过" }}
        </span>
      </h2>
      <div class="steps">
        <div v-for="(s, i) in verify.steps" :key="i" class="step">
          <span :class="['step-mark', s.ok ? 'ok' : 'err']">{{ s.ok ? "✓" : "✗" }}</span>
          <div class="step-body">
            <div class="step-label">{{ s.label }}</div>
            <div class="step-detail">{{ s.detail }} · {{ s.timestamp }}</div>
          </div>
        </div>
      </div>
      <p class="muted" style="margin-top: 14px">
        「隧道测试」只证明隧道本身可用；具体应用是否走了网关，见「应用」页的路由验证。
      </p>
    </div>
  </div>
</template>
