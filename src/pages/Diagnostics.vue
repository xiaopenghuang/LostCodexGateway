<script setup lang="ts">
import { onMounted, ref, computed } from "vue";
import { store, initStore, exportDiagnostics } from "../stores/gateway";

const exported = ref("");
const exportBusy = ref(false);
const logs = computed(() => store.logs.slice(-100));

onMounted(async () => {
  await initStore();
  store.logs = store.snapshot?.recent_logs ?? [];
});

async function doExport() {
  exportBusy.value = true;
  try {
    exported.value = await exportDiagnostics();
  } finally {
    exportBusy.value = false;
  }
}
</script>

<template>
  <div>
    <div class="card">
      <h2>诊断概览</h2>
      <div class="kv"><span class="k">当前状态</span><span class="v">{{ store.snapshot?.state ?? "—" }}</span></div>
      <div class="kv">
        <span class="k">系统 OpenSSH</span>
        <span class="v">{{ store.sshEnv?.path ?? "未检测" }}（{{ store.sshEnv?.version || "?" }}）</span>
      </div>
      <div class="kv">
        <span class="k">SSH 子进程 PID</span>
        <span class="v">{{ store.snapshot?.ssh_pid ?? "无" }}</span>
      </div>
      <div class="kv">
        <span class="k">本地端口监听</span>
        <span class="v">见下方日志</span>
      </div>
      <div class="row" style="margin-top: 10px">
        <button class="btn secondary" :disabled="exportBusy" @click="doExport">导出脱敏诊断报告</button>
        <span v-if="exported" class="muted mono">{{ exported }}</span>
      </div>
      <p class="muted">报告只包含：时间、组件、连接状态、错误类别、脱敏地址。不含私钥内容、Token、请求正文。</p>
    </div>

    <div class="card">
      <h2>SSH 生命周期日志（脱敏）</h2>
      <div class="logbox">
        <div v-for="(l, i) in logs" :key="i">
          <span class="t">{{ l.ts }}</span>
          <span :class="l.level === 'error' ? 'e' : l.level === 'warn' ? 'w' : ''">
            [{{ l.component }}] {{ l.message }}
          </span>
        </div>
        <div v-if="logs.length === 0" class="muted">暂无日志。</div>
      </div>
    </div>
  </div>
</template>
