<script setup lang="ts">
import { onMounted, ref, computed } from "vue";
import {
  store, initStore, saveServer, testConnection, fetchHostKey, confirmHostKey,
} from "../stores/gateway";

const form = ref({
  host: "", port: 22, username: "", key_path: "",
  socks_port: 17801, ssh_exe_path: "", server_name: "",
});
const saving = ref(false);
const testing = ref(false);
const saveMsg = ref("");
const testMsg = ref("");
const hostKeyMsg = ref("");

const snapshot = computed(() => store.snapshot);
const sshEnv = computed(() => store.sshEnv);
const hostKey = computed(() => store.hostKey);

onMounted(async () => {
  await initStore();
  const s = snapshot.value?.config?.server;
  if (s) form.value = { ...s };
  if (!form.value.ssh_exe_path && sshEnv.value?.exists) {
    form.value.ssh_exe_path = sshEnv.value.path;
  }
});

async function doSave() {
  saving.value = true;
  saveMsg.value = "";
  try {
    saveMsg.value = await saveServer({ ...form.value } as unknown as Record<string, unknown>);
  } finally {
    saving.value = false;
  }
}

async function doTest() {
  testing.value = true;
  testMsg.value = "";
  try {
    await saveServer({ ...form.value } as unknown as Record<string, unknown>);
    testMsg.value = await testConnection();
  } finally {
    testing.value = false;
  }
}

async function doFetchHostKey() {
  hostKeyMsg.value = "";
  await saveServer({ ...form.value } as unknown as Record<string, unknown>);
  const hk = await fetchHostKey();
  if (!hk.fingerprint) {
    hostKeyMsg.value = "未能获取指纹：请确认服务器地址/端口可达（ssh-keyscan 失败）。";
  }
}

async function doConfirm() {
  hostKeyMsg.value = await confirmHostKey();
}
</script>

<template>
  <div>
    <div class="page-head">
      <div>
        <h1 class="page-title">服务器</h1>
        <p class="page-desc">填写你要作为网络出口的服务器 SSH 信息。私钥只保存路径，不读取内容。</p>
      </div>
    </div>

    <div class="card">
      <h2>连接信息</h2>
      <div class="row">
        <label class="field">目标服务器名称（可选）
          <input type="text" v-model="form.server_name" placeholder="我的 VPS" />
        </label>
        <label class="field">主机地址
          <input type="text" v-model="form.host" placeholder="vps.example.com 或 IP" />
        </label>
        <label class="field" style="flex: 0 1 130px">SSH 端口
          <input type="number" v-model.number="form.port" min="1" max="65535" />
        </label>
        <label class="field">用户名
          <input type="text" v-model="form.username" placeholder="ubuntu" />
        </label>
      </div>

      <h3>本地出口</h3>
      <div class="row">
        <label class="field" style="flex: 0 1 200px">本地 SOCKS 端口
          <input type="number" v-model.number="form.socks_port" min="1024" max="65535" />
        </label>
        <label class="field">系统 ssh.exe 路径
          <input type="text" v-model="form.ssh_exe_path" />
        </label>
      </div>
      <p class="muted" v-if="sshEnv">
        已检测到系统 OpenSSH：<span class="mono">{{ sshEnv.path }}</span>
        （{{ sshEnv.version || "版本未知" }}）。默认优先使用它，避免与 PATH 中其他 ssh 混用。
      </p>

      <h3>认证</h3>
      <label class="field">SSH 私钥路径（只保存路径，不读取内容）
        <input type="text" v-model="form.key_path" placeholder="C:\Users\you\.ssh\id_ed25519" />
      </label>
      <div class="notice info">
        本工具不保存 SSH 密码，只支持密钥或系统 SSH agent 认证（BatchMode）。
        私钥内容不会被复制、导出或写入日志。
      </div>

      <div class="row" style="margin-top: 14px">
        <button class="btn" :disabled="saving" @click="doSave">保存配置</button>
        <button class="btn secondary" :disabled="testing" @click="doTest">测试连接</button>
        <span v-if="saveMsg" class="muted">{{ saveMsg }}</span>
        <span v-if="testMsg" class="muted mono">{{ testMsg }}</span>
      </div>
    </div>

    <div class="card">
      <h2>
        Host Key（服务器指纹）
        <span v-if="hostKey?.known" class="status-pill ok">已记录</span>
        <span v-else-if="hostKey?.fingerprint" class="status-pill warn">待确认</span>
      </h2>

      <div v-if="hostKey && hostKey.known">
        <div class="stat-grid">
          <div class="stat">
            <span class="stat-label">密钥类型</span>
            <span class="stat-value">{{ hostKey.key_type }}</span>
          </div>
          <div class="stat">
            <span class="stat-label">指纹</span>
            <span class="stat-value accent">{{ hostKey.fingerprint }}</span>
          </div>
        </div>
      </div>

      <div v-else-if="hostKey && hostKey.fingerprint">
        <div class="notice">
          <p>
            这是<b>首次连接</b>该服务器。请先通过可信渠道核对下方指纹，
            确认无误后再写入系统 known_hosts。
          </p>
        </div>
        <div class="stat-grid">
          <div class="stat">
            <span class="stat-label">密钥类型</span>
            <span class="stat-value">{{ hostKey.key_type }}</span>
          </div>
          <div class="stat">
            <span class="stat-label">待确认指纹</span>
            <span class="stat-value accent">{{ hostKey.fingerprint }}</span>
          </div>
        </div>
        <p class="muted" style="margin-top: 12px">
          确认后本工具会把该条目追加写入系统 known_hosts（写前自动备份）。
          指纹若与上次不同，连接将被阻断并告警，不会自动删除旧条目。
        </p>
        <button class="btn" @click="doConfirm">我已核对，确认写入</button>
      </div>

      <div v-else>
        <p class="muted">
          尚未获取该服务器的指纹。点击下方按钮通过 <span class="mono">ssh-keyscan</span> 查询。
        </p>
        <button class="btn secondary" @click="doFetchHostKey">查询服务器指纹</button>
      </div>

      <p v-if="hostKeyMsg" class="notice err" style="margin-top: 12px">{{ hostKeyMsg }}</p>
    </div>
  </div>
</template>
