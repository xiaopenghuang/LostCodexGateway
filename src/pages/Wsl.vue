<script setup lang="ts">
import { onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { initStore, store } from "../stores/gateway";
import type { WslDetection, WslProxyCommand } from "../types/wsl";
import { roleLabel } from "../types/wsl";

const det = ref(null as WslDetection | null);
const busy = ref(false);
const err = ref("");
const cmd = ref(null as WslProxyCommand | null);
const cmdErr = ref("");
const copied = ref("");

onMounted(async () => {
  await initStore();
});

async function doDetect() {
  busy.value = true;
  err.value = "";
  cmd.value = null;
  cmdErr.value = "";
  try {
    det.value = await invoke<WslDetection>("detect_wsl");
  } catch (e) {
    err.value = String(e);
  } finally {
    busy.value = false;
  }
}

async function doGenCommand() {
  cmdErr.value = "";
  cmd.value = null;
  try {
    cmd.value = await invoke<WslProxyCommand>("get_wsl_proxy_command", { proxyHost: null });
  } catch (e) {
    cmdErr.value = String(e);
  }
}

async function copyText(text: string, tag: string) {
  try {
    await navigator.clipboard.writeText(text);
    copied.value = tag;
    setTimeout(() => {
      if (copied.value === tag) copied.value = "";
    }, 2000);
  } catch {
    copied.value = "";
  }
}

const tunnelReady = () => store.snapshot?.state === "EGRESS_VERIFIED";
</script>

<template>
  <div>
    <div class="page-head">
      <div>
        <h1 class="page-title">WSL2</h1>
        <p class="page-desc">探测 WSL 内能否真正连到本机网关，并生成只影响当前 shell 的注入命令。</p>
      </div>
      <button class="btn" :disabled="busy" @click="doDetect">
        {{ busy ? "检测中…" : "检测 WSL" }}
      </button>
    </div>

    <div class="card">
      <h2>
        WSL2 专项支持
        <span v-if="det" :class="['status-pill', det.gateway_reachable_from_wsl ? 'ok' : 'warn']">
          {{ det.gateway_reachable_from_wsl ? "已实测可达" : "未验证" }}
        </span>
      </h2>
      <p class="muted">
        WSL2 内的进程<b>不继承</b> Windows 的进程级规则与环境变量，所以「Windows 上装了 Codex」
        不代表「WSL2 里的 Codex 也走网关」。本页只做<b>只读探测</b>：以「在 WSL 内真正建连成功」为唯一判据，
        绝不因为装了 WSL 就声称可用。
      </p>
      <p v-if="!tunnelReady()" class="notice">
        当前隧道状态为「{{ store.snapshot?.state ?? "未知" }}」。探测需要网关正在监听 SOCKS 端口，
        请先在「首页」连接成功后再检测。
      </p>
      <p v-if="err" class="notice err mono">{{ err }}</p>
    </div>

    <template v-if="det">
      <div class="card">
        <h2>检测结论</h2>
        <div class="stat-grid">
          <div class="stat">
            <span class="stat-label">wsl.exe</span>
            <span :class="['stat-value', det.wsl_exe_found ? 'ok' : 'err']">
              {{ det.wsl_exe_found ? "已找到" : "未找到" }}
            </span>
          </div>
          <div class="stat">
            <span class="stat-label">Windows 侧 SOCKS 端口</span>
            <span class="stat-value accent">127.0.0.1:{{ det.socks_port }}</span>
          </div>
          <div class="stat">
            <span class="stat-label">WSL → 本网关</span>
            <span :class="['stat-value', det.gateway_reachable_from_wsl ? 'ok' : 'err']">
              {{ det.gateway_reachable_from_wsl ? "可达" : "不可达" }}
            </span>
          </div>
          <div class="stat">
            <span class="stat-label">发行版数量</span>
            <span class="stat-value">{{ det.distros.length }}</span>
          </div>
        </div>
        <div v-if="det.recommended_proxy" class="readout" style="margin-top: 12px">
          <div class="kv">
            <span class="k">推荐代理地址</span>
            <span class="v mono">{{ det.recommended_proxy }}</span>
          </div>
        </div>
        <p class="notice">{{ det.note }}</p>
      </div>

      <div class="card">
        <h2>
          发行版
          <span class="status-pill info">{{ det.distros.length }}</span>
        </h2>
        <p v-if="det.distros.length === 0" class="empty">未检测到任何 WSL 发行版。</p>
        <div v-for="d in det.distros" :key="d.name" class="readout">
          <div class="row" style="justify-content: space-between">
            <b>{{ d.name }}{{ d.is_default ? "（默认）" : "" }}</b>
            <span :class="['status-pill', d.state.toLowerCase() === 'running' ? 'ok' : 'info']">
              {{ d.state }} · WSL{{ d.version }}
            </span>
          </div>
          <p v-if="d.kernel" class="muted mono" style="font-size: 11px">{{ d.kernel }}</p>
          <p v-if="d.reachable_targets.length === 0" class="muted">
            未运行，未探测（本工具不会替你启动 WSL）。
          </p>
          <div v-else>
            <div v-for="t in d.reachable_targets" :key="t.target + t.role" class="kv">
              <span class="k">
                <span :class="['status-pill', t.reachable ? 'ok' : 'err']" style="margin-right: 8px">
                  {{ t.reachable ? "✓" : "✗" }}
                </span>
                {{ roleLabel(t.role) }}
              </span>
              <span class="v mono">{{ t.target }}:{{ det.socks_port }}</span>
            </div>
          </div>
        </div>
      </div>

      <div class="card">
        <h2>在 WSL 内使用网关</h2>
        <div class="notice info">
          生成的命令<b>只影响你粘贴它的那个 shell 会话</b>：不写 <code>~/.bashrc</code>、
          不改 <code>/etc/environment</code>、不动 WSL 网络配置。
        </div>
        <div class="row">
          <button
            class="btn secondary"
            :disabled="!det.gateway_reachable_from_wsl"
            @click="doGenCommand"
          >
            生成注入命令
          </button>
          <span v-if="!det.gateway_reachable_from_wsl" class="muted">
            探测结果为不可达，已禁用（避免给出注定失败的指引）。
          </span>
        </div>
        <p v-if="cmdErr" class="notice err mono">{{ cmdErr }}</p>

        <template v-if="cmd">
          <h3>① 注入代理（一次性）</h3>
          <pre class="logbox">{{ cmd.inject_command }}</pre>
          <div class="row end">
            <button class="btn ghost sm" @click="copyText(cmd.inject_command, 'inject')">
              {{ copied === "inject" ? "已复制" : "复制命令" }}
            </button>
          </div>

          <h3>② 自检（对比网关出口与本机出口）</h3>
          <pre class="logbox">{{ cmd.selfcheck_command }}</pre>
          <div class="row end">
            <button class="btn ghost sm" @click="copyText(cmd.selfcheck_command, 'check')">
              {{ copied === "check" ? "已复制" : "复制命令" }}
            </button>
          </div>
          <p class="muted">{{ cmd.note }}</p>
        </template>
      </div>

      <div class="card">
        <h2>为什么 NAT 模式下不可达</h2>
        <div class="steps">
          <div class="step">
            <span class="step-mark err">N</span>
            <div class="step-body">
              <div class="step-label">NAT 模式（WSL2 默认）</div>
              <div class="step-detail">
                WSL 有独立虚拟网卡与 IP，它自己的 127.0.0.1 指向 WSL 自身，不是 Windows。
                而本工具的 SOCKS 只绑 Windows 的 127.0.0.1，因此 WSL 侧默认连不上。
              </div>
            </div>
          </div>
          <div class="step">
            <span class="step-mark ok">M</span>
            <div class="step-body">
              <div class="step-label">Mirrored 模式</div>
              <div class="step-detail">
                WSL 与 Windows 共享回环，127.0.0.1 直接互通。可在 %USERPROFILE%\.wslconfig
                中设置 networkingMode=mirrored 后 wsl --shutdown 重启。
              </div>
            </div>
          </div>
          <div class="step">
            <span class="step-mark ok">✓</span>
            <div class="step-body">
              <div class="step-label">本工具的边界</div>
              <div class="step-detail">
                不代做需要管理员权限的端口转发（如 netsh interface portproxy），
                以免在用户不知情时改动系统网络配置。
              </div>
            </div>
          </div>
        </div>
      </div>
    </template>
  </div>
</template>
