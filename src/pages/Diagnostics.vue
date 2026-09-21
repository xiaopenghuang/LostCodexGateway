<script setup lang="ts">
import { onMounted, ref, computed } from "vue";
import { store, initStore, exportDiagnostics } from "../stores/gateway";

const exported = ref("");
const exportBusy = ref(false);
const logs = computed(() => store.logs.slice(-100));

const errCount = computed(() => logs.value.filter((l) => l.level === "error").length);
const warnCount = computed(() => logs.value.filter((l) => l.level === "warn").length);

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
    <div class="page-head">
      <div>
        <h1 class="page-title">诊断</h1>
        <p class="page-desc">当前状态、系统 OpenSSH 探测结果与 SSH 生命周期日志。报告已脱敏。</p>
      </div>
    </div>

    <div class="card">
      <h2>诊断概览</h2>
      <div class="stat-grid">
        <div class="stat">
          <span class="stat-label">当前状态</span>
          <span class="stat-value">{{ store.snapshot?.state ?? "—" }}</span>
        </div>
        <div class="stat">
          <span class="stat-label">SSH 子进程 PID</span>
          <span class="stat-value">{{ store.snapshot?.ssh_pid ?? "无" }}</span>
        </div>
        <div class="stat">
          <span class="stat-label">本地端口监听</span>
          <span class="stat-value dim">见下方日志</span>
        </div>
      </div>

      <div class="readout" style="margin-top: 12px">
        <div class="kv">
          <span class="k">系统 OpenSSH</span>
          <span class="v">{{ store.sshEnv?.path ?? "未检测" }}（{{ store.sshEnv?.version || "?" }}）</span>
        </div>
      </div>

      <div class="row" style="margin-top: 10px">
        <button class="btn secondary" :disabled="exportBusy" @click="doExport">导出脱敏诊断报告</button>
        <span v-if="exported" class="muted mono">{{ exported }}</span>
      </div>
      <p class="muted">报告只包含：时间、组件、连接状态、错误类别、脱敏地址。不含私钥内容、Token、请求正文。</p>
    </div>

    <div class="card">
      <h2>
        SSH 生命周期日志（脱敏）
        <span class="status-pill info">{{ logs.length }} 条</span>
        <span v-if="errCount" class="status-pill err">{{ errCount }} 异常</span>
        <span v-if="warnCount" class="status-pill warn">{{ warnCount }} 需注意</span>
      </h2>
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
