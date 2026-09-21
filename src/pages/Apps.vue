<script setup lang="ts">
import { onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import {
  store, initStore, getLaunchPreview, launchCodexCli,
} from "../stores/gateway";

const preview = ref(null as null | { command: string; env: Record<string, string>; proxy_line: string; bridge_needed: boolean });
const launchMsg = ref("");
const launching = ref(false);

// M3 Mihomo 检测
const mihomo = ref(null as null | Record<string, unknown>);
const mihomoBusy = ref(false);
const mihomoErr = ref("");
const fragment = ref("");
const fragGroup = ref("MY-VPS");
const fragNames = ref("codex.exe");
const fragPaths = ref("");
const backupPath = ref("");
const backupSrc = ref("");
const restorePath = ref("");
const restoreBak = ref("");
const restoreMsg = ref("");

onMounted(async () => {
  await initStore();
});

async function doPreview() {
  try {
    preview.value = await getLaunchPreview();
  } catch (e) {
    launchMsg.value = String(e);
  }
}

async function doLaunch() {
  launching.value = true;
  launchMsg.value = "";
  try {
    const r = await launchCodexCli();
    launchMsg.value = r.message + (r.pid ? `（PID ${r.pid}）` : "");
  } catch (e) {
    launchMsg.value = String(e);
  } finally {
    launching.value = false;
  }
}

async function doDetectMihomo() {
  mihomoBusy.value = true;
  mihomoErr.value = "";
  try {
    mihomo.value = await invoke("detect_mihomo");
  } catch (e) {
    mihomoErr.value = String(e);
  } finally {
    mihomoBusy.value = false;
  }
}

async function doGenerateFragment() {
  try {
    fragment.value = await invoke("generate_mihomo_fragment", {
      proxyGroup: fragGroup.value,
      processNames: fragNames.value.split(",").map((s) => s.trim()).filter(Boolean),
      processPaths: fragPaths.value.split(",").map((s) => s.trim()).filter(Boolean),
    });
  } catch (e) {
    mihomoErr.value = String(e);
  }
}

async function doBackup() {
  try {
    backupPath.value = await invoke("backup_mihomo_file", { path: backupSrc.value });
  } catch (e) {
    mihomoErr.value = String(e);
  }
}

async function doRestore() {
  restoreMsg.value = "";
  try {
    restoreMsg.value = await invoke("restore_mihomo_file", { path: restorePath.value, backup: restoreBak.value });
  } catch (e) {
    restoreMsg.value = String(e);
  }
}
</script>

<template>
  <div>
    <div class="card">
      <h2>Codex CLI 专用启动器</h2>
      <p class="muted">
        从本工具启动的 Codex CLI 将获得进程级代理环境变量（仅注入新子进程），
        不修改系统/用户全局环境变量，不影响其他终端。
      </p>
      <div class="row">
        <button class="btn secondary" @click="doPreview">预览将注入的设置</button>
        <button
          class="btn"
          :disabled="store.snapshot?.state !== 'EGRESS_VERIFIED' || launching"
          @click="doLaunch"
        >
          从网关启动 Codex CLI
        </button>
      </div>
      <p v-if="store.snapshot?.state !== 'EGRESS_VERIFIED'" class="muted">
        当前状态为「{{ store.snapshot?.state ?? "未知" }}」——只有出口验证通过后才允许启动，避免 CLI 悄悄走直连。
      </p>
      <p v-if="store.snapshot?.bridge_port" class="muted mono">
        桥接层运行中：127.0.0.1:{{ store.snapshot.bridge_port }}（HTTP CONNECT → SOCKS5 远端 DNS）
        · 已处理 {{ store.snapshot.bridge_connections_total }} 条连接
        <span v-if="store.snapshot.bridge_last_target">· 最近目标 {{ store.snapshot.bridge_last_target }}</span>
      </p>
      <div v-if="(store.snapshot?.bridge_rejects_total ?? 0) > 0" class="notice">
        <p>
          桥接层已拒绝 <b>{{ store.snapshot?.bridge_rejects_total }}</b> 条连接。
          这是<b>设计内的保护</b>，不代表故障：回环地址与私有网段不会被经隧道转发，
          也不会被当作跳板形成循环代理。
        </p>
        <ul class="muted" style="line-height: 1.8; margin: 6px 0 0 0">
          <li v-for="(r, i) in (store.snapshot?.bridge_recent_rejects ?? [])" :key="i" class="mono" style="font-size: 11px">
            {{ r.code }} · {{ r.message }}
            <span v-if="r.target !== '-'">· 目标 {{ r.target }}</span>
          </li>
        </ul>
      </div>
      <p v-if="launchMsg" class="muted mono">{{ launchMsg }}</p>

      <template v-if="preview">
        <h3>将注入的环境变量（仅子进程）</h3>
        <div v-for="(v, k) in preview.env" :key="k" class="kv">
          <span class="k">{{ k }}</span><span class="v">{{ v }}</span>
        </div>
        <p class="muted">代理模式：{{ preview.proxy_line }}</p>
        <p class="muted">启动命令：{{ preview.command }}</p>
      </template>
    </div>

    <div class="card">
      <h2>Codex Desktop / IDE（Mihomo 受控集成）</h2>
      <div class="notice">
        本工具不会因为「命中进程名」就声称已代理。先做只读检测与进程发现，
        生成规则片段并经过你的确认后，按指引手动导入；回滚只恢复本工具的备份。
      </div>

      <div class="row">
        <button class="btn secondary" :disabled="mihomoBusy" @click="doDetectMihomo">检测 Mihomo / Clash Verge</button>
      </div>
      <p v-if="mihomoErr" class="muted mono">{{ mihomoErr }}</p>

      <template v-if="mihomo">
        <div class="kv"><span class="k">Verge 版本</span><span class="v">{{ mihomo.verge_version ?? "未知" }}</span></div>
        <div class="kv"><span class="k">Verge / Mihomo 进程</span><span class="v">{{ mihomo.verge_running ? "运行中" : "未运行" }} / {{ mihomo.mihomo_running ? "运行中" : "未运行" }}</span></div>
        <div class="kv"><span class="k">mixed-port</span><span class="v">{{ mihomo.mixed_port ?? "—" }}</span></div>
        <div class="kv"><span class="k">external-controller</span><span class="v">{{ mihomo.external_controller ?? "—" }}</span></div>
        <div class="kv"><span class="k">TUN 状态</span>
          <span :class="['status-pill', mihomo.tun_enabled ? 'ok' : 'warn']">{{ mihomo.tun_enabled ? "已开启" : "未开启" }}</span>
        </div>
        <div class="kv"><span class="k">模式</span><span class="v">{{ mihomo.mode ?? "—" }}</span></div>
        <div v-for="(n, i) in mihomo.notes" :key="i" class="muted">· {{ n }}</div>
        <p v-if="mihomo.tun_enabled === false" class="notice">
          TUN 未开启：Desktop / IDE 的进程级分流**尚未覆盖**。开启 TUN 属高级功能，
          由 Mihomo/Verge 官方组件处理权限，本工具不代办。
        </p>
      </template>

      <h3>生成规则片段（不自动写入任何配置）</h3>
      <div class="row">
        <label class="field">代理组名
          <input type="text" v-model="fragGroup" />
        </label>
        <label class="field">进程名（逗号分隔，仅已确认的 Codex 进程）
          <input type="text" v-model="fragNames" style="min-width: 320px" />
        </label>
        <label class="field">进程路径（逗号分隔，可选）
          <input type="text" v-model="fragPaths" style="min-width: 320px" />
        </label>
        <button class="btn secondary" @click="doGenerateFragment">生成片段</button>
      </div>
      <pre v-if="fragment" class="logbox">{{ fragment }}</pre>
      <p class="muted">
        通用进程（Code.exe、node.exe、ssh.exe）不会被本工具自动加入片段；
        导入方式与冲突处理见 docs/setup-windows.md 的 Mihomo 章节。
      </p>

      <h3>备份与回滚（仅本工具备份）</h3>
      <div class="row">
        <label class="field">要备份的文件路径
          <input type="text" v-model="backupSrc" placeholder="…\profiles\xxx.yaml" style="min-width: 360px" />
        </label>
        <button class="btn secondary" @click="doBackup">备份</button>
        <span v-if="backupPath" class="muted mono">{{ backupPath }}</span>
      </div>
      <div class="row">
        <label class="field">回滚目标路径
          <input type="text" v-model="restorePath" style="min-width: 300px" />
        </label>
        <label class="field">备份文件路径
          <input type="text" v-model="restoreBak" style="min-width: 300px" />
        </label>
        <button class="btn danger" @click="doRestore">回滚</button>
        <span v-if="restoreMsg" class="muted mono">{{ restoreMsg }}</span>
      </div>
    </div>
  </div>
</template>
