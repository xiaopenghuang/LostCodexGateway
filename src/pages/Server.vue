<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import {
  store, initStore, activeServer, saveServer, deleteServer, switchServer,
  testServers, testConnection, fetchHostKey, confirmHostKey, tunnelIsActive,
} from "../stores/gateway";
import type { ServerProfile } from "../types";

/** 编辑中的表单。`id` 为空表示「正在新增」。 */
function blankForm(): ServerProfile {
  return {
    id: "", name: "", host: "", port: 22, username: "", key_path: "",
    ssh_exe_path: "", expected_egress_ip: "", gateway_group: "MY-VPS",
  };
}

const form = ref<ServerProfile>(blankForm());
const editing = ref(false);
const saving = ref(false);
const testing = ref(false);
const switching = ref<string | null>(null);
const saveMsg = ref("");
const saveErr = ref("");
const testMsg = ref("");
const hostKeyMsg = ref("");
const listMsg = ref("");
const listErr = ref("");

const snapshot = computed(() => store.snapshot);
const sshEnv = computed(() => store.sshEnv);
const hostKey = computed(() => store.hostKey);
const cfg = computed(() => snapshot.value?.config ?? null);
const servers = computed(() => cfg.value?.servers ?? []);
const activeId = computed(() => cfg.value?.active_server_id ?? "");
const active = computed(() => activeServer(cfg.value));
const isActive = (s: ServerProfile) => s.id === activeId.value;
/** 切换失败后留下的「上一个」——与当前不同才显示切回按钮。 */
const previous = computed(() => {
  const prev = snapshot.value?.previous_server_id;
  if (!prev || prev === activeId.value) return null;
  return servers.value.find((s) => s.id === prev) ?? null;
});
const running = computed(() => tunnelIsActive(snapshot.value?.state));

onMounted(async () => {
  await initStore();
  // 首次使用（一台都没有）直接把表单摊开，省掉一次点击；
  // 已有服务器时保持收起，避免列表下面永远挂着一个空表单。
  if (servers.value.length === 0) {
    startNew();
  } else {
    editing.value = false;
    form.value = blankForm();
  }
});

function startNew() {
  const f = blankForm();
  if (sshEnv.value?.exists) f.ssh_exe_path = sshEnv.value.path;
  f.gateway_group = cfg.value?.settings?.gateway_group ?? "MY-VPS";
  form.value = f;
  editing.value = true;
  saveMsg.value = "";
  saveErr.value = "";
}

function startEdit(s: ServerProfile) {
  form.value = { ...s };
  editing.value = true;
  saveMsg.value = "";
  saveErr.value = "";
}

function cancelEdit() {
  editing.value = false;
  form.value = blankForm();
  saveMsg.value = "";
  saveErr.value = "";
}

async function doSave() {
  saving.value = true;
  saveMsg.value = "";
  saveErr.value = "";
  try {
    const msg = await saveServer({ ...form.value });
    saveMsg.value = msg;
    editing.value = false;
    form.value = blankForm();
  } catch (e) {
    saveErr.value = String(e);
  } finally {
    saving.value = false;
  }
}

async function doDelete(s: ServerProfile) {
  listMsg.value = "";
  listErr.value = "";
  const label = s.name || s.host || "未命名";
  if (!window.confirm(`确认删除服务器「${label}」？\n\n若它正在连接中，会先断开隧道。`)) return;
  try {
    listMsg.value = await deleteServer(s.id);
    if (form.value.id === s.id) cancelEdit();
  } catch (e) {
    listErr.value = String(e);
  }
}

/** 切换服务器。后端是硬切（断开 → 改选中 → 重连），失败不回滚。 */
async function doSwitch(s: ServerProfile) {
  listMsg.value = "";
  listErr.value = "";
  const label = s.name || s.host || "未命名";
  if (running.value) {
    const ok = window.confirm(
      `切换到「${label}」会先断开当前隧道再重连。\n` +
      `正在进行的请求会中断（这是硬切的代价）。\n\n继续？`,
    );
    if (!ok) return;
  }
  switching.value = s.id;
  try {
    listMsg.value = await switchServer(s.id);
  } catch (e) {
    // 切换失败不回滚：后端保留 previous_server_id，下方会出现「切回上一个」
    listErr.value = String(e);
  } finally {
    switching.value = null;
  }
}

async function doLatency() {
  listMsg.value = "";
  listErr.value = "";
  try {
    await testServers();
  } catch (e) {
    listErr.value = String(e);
  }
}

async function doTest() {
  testing.value = true;
  testMsg.value = "";
  saveErr.value = "";
  try {
    // 测试连接针对的是「当前选中」的那台，所以先把表单存下去
    await saveServer({ ...form.value });
    testMsg.value = await testConnection();
  } catch (e) {
    saveErr.value = String(e);
  } finally {
    testing.value = false;
  }
}

async function doFetchHostKey() {
  hostKeyMsg.value = "";
  try {
    if (editing.value) await saveServer({ ...form.value });
    const hk = await fetchHostKey();
    if (!hk.fingerprint) {
      hostKeyMsg.value = "未能获取指纹：请确认服务器地址/端口可达（ssh-keyscan 失败）。";
    }
  } catch (e) {
    hostKeyMsg.value = String(e);
  }
}

async function doConfirm() {
  hostKeyMsg.value = "";
  try {
    hostKeyMsg.value = await confirmHostKey();
  } catch (e) {
    hostKeyMsg.value = String(e);
  }
}

function latencyText(id: string): string {
  const l = store.latencies[id];
  if (!l) return "—";
  if (!l.reachable) return "不可达";
  return l.latency_ms === null ? "可达" : `${l.latency_ms} ms`;
}

function latencyClass(id: string): string {
  const l = store.latencies[id];
  if (!l) return "lat-dim";
  if (!l.reachable) return "lat-err";
  // >300ms 用琥珀色而不是灰色：灰色读起来像「不可用」，而慢 ≠ 不可用
  if (l.latency_ms !== null && l.latency_ms > 300) return "lat-warn";
  return "lat-ok";
}
</script>

<template>
  <div>
    <div class="page-head">
      <div>
        <h1 class="page-title">服务器</h1>
        <p class="page-desc">
          管理多台出口服务器，随时切换当前使用的那一台。私钥只保存路径，不读取内容。
        </p>
      </div>
    </div>

    <!-- ── 服务器列表 ───────────────────────────────────────── -->
    <div class="card">
      <h2>
        服务器列表
        <span class="status-pill info">{{ servers.length }} 台</span>
      </h2>

      <p class="muted">
        本工具同一时刻<b>只连接一台</b>服务器。切换是硬切：先断开旧隧道，再用新服务器重连，
        切换过程中正在进行的请求会中断。两个本地端口是全局固定的，所以切换后
        Codex CLI 的 <span class="mono">HTTP_PROXY</span> 不用改、也不用重启。
      </p>

      <div v-if="servers.length === 0" class="empty">还没有服务器，点击下方「新增服务器」开始。</div>

      <table v-else class="srv-table">
        <thead>
          <tr>
            <th>名称</th>
            <th>地址</th>
            <th class="col-lat">到 SSH 端口延迟</th>
            <th class="col-act">操作</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="s in servers" :key="s.id" :class="{ 'srv-active': isActive(s) }">
            <td>
              <span class="srv-name">{{ s.name || s.host || "未命名" }}</span>
              <span v-if="isActive(s)" class="status-pill ok">当前</span>
            </td>
            <td class="mono dim">{{ s.username }}@{{ s.host }}:{{ s.port }}</td>
            <td class="col-lat">
              <span class="mono" :class="latencyClass(s.id)">{{ latencyText(s.id) }}</span>
            </td>
            <td class="col-act">
              <button
                class="btn sm"
                :disabled="isActive(s) || switching !== null || !s.host || !s.username"
                @click="doSwitch(s)"
              >
                {{ switching === s.id ? "切换中…" : "切换" }}
              </button>
              <button class="btn sm secondary" @click="startEdit(s)">编辑</button>
              <button class="btn sm danger" @click="doDelete(s)">删除</button>
            </td>
          </tr>
        </tbody>
      </table>

      <div class="row" style="margin-top: 14px">
        <button class="btn" @click="startNew">新增服务器</button>
        <button
          class="btn secondary"
          :disabled="store.latencyTesting || servers.length === 0"
          @click="doLatency"
        >
          {{ store.latencyTesting ? "探测中…" : "测试延迟" }}
        </button>
        <button
          v-if="previous"
          class="btn ghost"
          :disabled="switching !== null"
          @click="doSwitch(previous)"
        >
          切回上一个（{{ previous.name || previous.host }}）
        </button>
      </div>

      <p class="muted" style="margin-top: 10px">
        <b>关于「延迟」</b>：测的是从本机到服务器 <span class="mono">SSH 端口</span>的 TCP 往返时间，
        <b>不是出口延迟</b>——它不建隧道、不认证、不发任何数据，只回答「哪台机器网络更近」。
        出口速度和稳定性与这个数字没有直接关系。
      </p>

      <p v-if="listMsg" class="notice ok" style="margin-top: 12px">{{ listMsg }}</p>
      <p v-if="listErr" class="notice err" style="margin-top: 12px">{{ listErr }}</p>
    </div>

    <!-- ── 编辑 / 新增 ──────────────────────────────────────── -->
    <div v-if="editing" class="card">
      <h2>
        {{ form.id ? "编辑服务器" : "新增服务器" }}
        <span v-if="form.id" class="status-pill info">{{ form.name || form.host || "未命名" }}</span>
        <span v-else class="status-pill info">新条目</span>
      </h2>

      <div class="row form">
        <label class="field">显示名称（可选）
          <input type="text" v-model="form.name" placeholder="例如：东京中转节点" />
        </label>
        <label class="field">主机地址
          <input type="text" v-model="form.host" placeholder="vps.example.com 或 IP" />
        </label>
        <label class="field narrow">SSH 端口
          <input type="number" v-model.number="form.port" min="1" max="65535" />
        </label>
        <label class="field">用户名
          <input type="text" v-model="form.username" placeholder="ubuntu" />
        </label>
      </div>

      <h3>认证</h3>
      <label class="field">SSH 私钥路径（只保存路径，不读取内容）
        <input type="text" v-model="form.key_path" placeholder="C:\Users\you\.ssh\id_ed25519" />
      </label>
      <div class="notice info">
        本工具不保存 SSH 密码，只支持密钥或系统 SSH agent 认证（BatchMode）。
        私钥内容不会被复制、导出或写入日志。
      </div>

      <h3>高级</h3>
      <div class="row form">
        <label class="field">系统 ssh.exe 路径（留空 = 自动检测）
          <input type="text" v-model="form.ssh_exe_path" />
        </label>
        <label class="field">预期出口 IP（可选，空 = 不校验）
          <input type="text" v-model="form.expected_egress_ip" placeholder="203.0.113.10" />
        </label>
        <label class="field">Mihomo 代理组名
          <input type="text" v-model="form.gateway_group" placeholder="MY-VPS" />
        </label>
      </div>
      <p class="muted" v-if="sshEnv">
        已检测到系统 OpenSSH：<span class="mono">{{ sshEnv.path }}</span>
        （{{ sshEnv.version || "版本未知" }}）。
      </p>
      <p class="muted">
        预期出口 IP 是<b>每台服务器各自一份</b>：填了之后诊断会把经隧道的实际出口与它比对，
        不一致会明确报「不匹配」。留空则不校验。
      </p>

      <div class="row" style="margin-top: 14px">
        <button class="btn" :disabled="saving" @click="doSave">
          {{ saving ? "保存中…" : "保存" }}
        </button>
        <button class="btn secondary" :disabled="testing" @click="doTest">
          {{ testing ? "测试中…" : "测试连接" }}
        </button>
        <button class="btn ghost" @click="cancelEdit">取消</button>
        <span v-if="saveMsg" class="muted">{{ saveMsg }}</span>
        <span v-if="testMsg" class="muted mono">{{ testMsg }}</span>
      </div>
      <p v-if="saveErr" class="notice err" style="margin-top: 12px">{{ saveErr }}</p>
      <p class="muted" style="margin-top: 10px">
        「测试连接」针对的是<b>当前选中</b>的那台，所以会先保存本次改动。
      </p>
    </div>

    <!-- ── 本地端口（全局） ─────────────────────────────────── -->
    <div class="card">
      <h2>本地入口端口（全局）</h2>
      <div class="stat-grid cols-2">
        <div class="stat">
          <span class="stat-label">SOCKS5 入口</span>
          <span class="stat-value mono">127.0.0.1:{{ cfg?.settings?.socks_port ?? "—" }}</span>
        </div>
        <div class="stat">
          <span class="stat-label">HTTP CONNECT 桥接</span>
          <span class="stat-value mono">127.0.0.1:{{ cfg?.settings?.bridge_port ?? "—" }}</span>
        </div>
      </div>
      <p class="muted" style="margin-top: 12px">
        这两个端口属于<b>全局设置</b>，不属于任何一台服务器——正因为它们固定不变，
        切换服务器才对 Codex CLI 透明（不用改 <span class="mono">HTTP_PROXY</span>、不用重启）。
        要修改请到「设置」页；隧道运行期间后端会拒绝修改，以免正在跑的桥接层与配置指向不一致。
      </p>
    </div>

    <!-- ── Host Key ─────────────────────────────────────────── -->
    <div class="card">
      <h2>
        Host Key（服务器指纹）
        <span v-if="active" class="status-pill info">{{ active.name || active.host }}</span>
        <span v-if="hostKey?.known" class="status-pill ok">已记录</span>
        <span v-else-if="hostKey?.fingerprint" class="status-pill warn">待确认</span>
      </h2>

      <p class="muted">
        指纹核对针对<b>当前选中</b>的服务器。切换服务器后需要为新服务器单独核对一次。
      </p>

      <div v-if="hostKey && hostKey.known">
        <div class="stat-grid cols-2">
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
        <div class="stat-grid cols-2">
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
          尚未获取当前服务器的指纹。点击下方按钮通过
          <span class="mono">ssh-keyscan</span> 查询。
        </p>
        <button class="btn secondary" @click="doFetchHostKey">查询服务器指纹</button>
      </div>

      <p v-if="hostKeyMsg" class="notice err" style="margin-top: 12px">{{ hostKeyMsg }}</p>
    </div>
  </div>
</template>

<style scoped>
.srv-table {
  width: 100%;
  border-collapse: collapse;
  margin-top: 12px;
}
.srv-table th {
  text-align: left;
  font-weight: 600;
  font-size: var(--fs-sm);
  color: var(--tx-3);
  padding: 6px 10px;
  border-bottom: 1px solid var(--bd-subtle);
  white-space: nowrap;
}
.srv-table td {
  padding: 10px;
  border-bottom: 1px solid var(--bd-subtle);
  vertical-align: middle;
}
.srv-table tr:last-child td {
  border-bottom: none;
}
.srv-table tr.srv-active td {
  background: var(--accent-soft);
}
.srv-name {
  font-weight: 600;
  margin-right: 8px;
}
.col-lat {
  width: 160px;
  white-space: nowrap;
}
.col-act {
  width: 220px;
  white-space: nowrap;
  text-align: right;
}
.col-act .btn {
  margin-left: 6px;
}
.lat-ok {
  color: var(--ok-fg);
}
.lat-warn {
  color: var(--warn-fg);
}
.lat-err {
  color: var(--err-fg);
}
.lat-dim {
  color: var(--tx-3);
}
</style>
