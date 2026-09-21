<script setup lang="ts">
import { computed, onMounted } from "vue";
import { store, initStore, connect, disconnect, stateLabel, stateKind } from "../stores/gateway";

defineEmits<{ (e: "go-server"): void }>();

const snap = computed(() => store.snapshot);
const state = computed(() => snap.value?.state);
const verify = computed(() => snap.value?.last_verify);
const configured = computed(() => !!snap.value?.config?.server?.host);

const directIp = computed(() => verify.value?.steps.find((s) => s.kind === "direct_ip")?.detail ?? null);
const tunnelIp = computed(() => verify.value?.egress_ip);

onMounted(async () => {
  await initStore();
});
</script>

<template>
  <div>
    <div class="card">
      <div class="row" style="justify-content: space-between">
        <h2 style="margin: 0">
          连接状态
          <span :class="['status-pill', stateKind(state)]">{{ stateLabel(state) }}</span>
        </h2>
        <div class="row">
          <button class="btn" :disabled="!configured || state === 'CONNECTING'" @click="connect">
            连接
          </button>
          <button class="btn danger" :disabled="state !== 'EGRESS_VERIFIED' && state !== 'TUNNEL_READY' && state !== 'DEGRADED'" @click="disconnect">
            断开
          </button>
        </div>
      </div>

      <template v-if="snap?.config?.server?.host">
        <div class="kv">
          <span class="k">目标服务器</span>
          <span class="v">{{ snap.config.server.server_name || "（未命名）" }} · {{ snap.config.server.username }}@{{ snap.config.server.host }}:{{ snap.config.server.port }}</span>
        </div>
        <div class="kv">
          <span class="k">本地 SOCKS 端口</span>
          <span class="v">127.0.0.1:{{ snap.config.server.socks_port }}</span>
        </div>
      </template>
      <template v-else>
        <div class="notice">
          尚未配置服务器。请先前往「服务器」页面填写自有 Ubuntu 服务器的 SSH 信息。
        </div>
        <button class="btn" @click="$emit('go-server')">前往配置</button>
      </template>

      <div class="kv">
        <span class="k">隧道出口 IP（经 SOCKS 实测）</span>
        <span class="v">{{ tunnelIp ?? "—" }}</span>
      </div>
      <div class="kv">
        <span class="k">本机对照出口 IP</span>
        <span class="v">{{ directIp ?? "—" }}</span>
      </div>
    </div>

    <div v-if="verify" class="card">
      <h2>出口验证步骤（{{ verify.ok ? "通过" : "未通过" }}）</h2>
      <div v-for="(s, i) in verify.steps" :key="i" class="kv">
        <span class="k">
          <span :class="['status-pill', s.ok ? 'ok' : 'err']" style="margin-right: 8px">{{ s.ok ? "✓" : "✗" }}</span>
          {{ s.label }}
        </span>
        <span class="v muted">{{ s.detail }} · {{ s.timestamp }}</span>
      </div>
      <p class="muted" v-if="verify.ok && tunnelIp === directIp">
        注意：隧道出口 IP 与本机对照出口相同，可能是出口路径重叠，请在诊断页核查。
      </p>
      <p class="muted">「隧道测试」只证明隧道可用；具体应用是否走了网关，见「应用」页的路由验证。</p>
    </div>
  </div>
</template>
