<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import {
  store, initStore, connect, disconnect, stateLabel, stateKind, activeServer,
} from "../stores/gateway";

defineEmits<{ (e: "go-server"): void; (e: "go-doctor"): void }>();

const snap = computed(() => store.snapshot);
const state = computed(() => snap.value?.state);
const verify = computed(() => snap.value?.last_verify);
const cfg = computed(() => snap.value?.config ?? null);
/** 当前选中的服务器（多服务器下「已配置」= 当前选中那台填完了） */
const active = computed(() => activeServer(cfg.value));
const configured = computed(() => !!active.value?.host && !!active.value?.username);

/** 是否处于「可断开」的活跃态。
 *
 * 只包含**已稳定**的活跃态：连接中 / 重连中 / 切换中不算——那几秒里点「断开」
 * 会和正在进行中的流程抢同一批资源（generation、子进程、端口），
 * 属于「按钮点了但结果不可预期」，不如先禁用。 */
const canDisconnect = computed(() =>
  state.value === "EGRESS_VERIFIED"
  || state.value === "TUNNEL_READY"
  || state.value === "DEGRADED",
);

const busy = computed(() =>
  state.value === "CONNECTING"
  || state.value === "RECONNECTING"
  || state.value === "DISCONNECTING"
  || state.value === "SWITCHING",
);

/**
 * 「连接」按钮是否可用。
 *
 * 已连上时禁用 —— 用户实测反馈过：「首页都显示出口已验证了，怎么『连接』
 * 按钮还是亮的」。点它只会被后端挡回来（「已有连接在进行中，请先断开」），
 * 按钮亮着反而和这个提示自相矛盾。
 */
const canConnect = computed(
  () => configured.value && !busy.value && !canDisconnect.value,
);

/**
 * 本机直连对照 IP。
 *
 * **只有 `ok === true` 时 `detail` 才是 IP**；失败时 `detail` 是原因说明
 * （如「直连失败（2 个端点均不可达…）」），直接当值显示会把整句文案
 * 塞进「本机直连对照」那一格。所以失败一律返回 null，由模板显示「未取得」。
 */
const directIp = computed(() => {
  const step = verify.value?.steps.find((s) => s.kind === "direct_ip");
  return step?.ok ? step.detail : null;
});
const tunnelIp = computed(() => verify.value?.egress_ip);

/** 直连对照是否未取得——失败时没有可比对的值，说明文案要换。 */
const directUnavailable = computed(() => {
  const step = verify.value?.steps.find((s) => s.kind === "direct_ip");
  return !!step && !step.ok;
});

/** 是否存在「参考项」失败——用于决定要不要显示那句解释。 */
const hasAdvisoryFailure = computed(
  () => !!verify.value?.steps.some((s) => s.advisory && !s.ok),
);

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
            <template v-if="!configured">还没配置服务器。首次使用建议先做「环境自检」，它会逐项告诉你缺哪一步。</template>
            <template v-else-if="state === 'EGRESS_VERIFIED'">
              隧道与出口均已验证，可以放心启动 Codex CLI。
            </template>
            <template v-else-if="state === 'TUNNEL_READY'">
              隧道已建立，但出口尚未通过实测验证。
            </template>
            <template v-else-if="state === 'SWITCHING'">
              正在切换服务器：先断开旧隧道，再用新服务器重连。正在进行的请求会中断。
            </template>
            <template v-else-if="busy">正在处理，请稍候…</template>
            <template v-else-if="state === 'DEGRADED'">隧道可用但出口验证未通过，请查看下方步骤。</template>
            <template v-else-if="state === 'ERROR'">连接出错，详情见下方日志或「日志与导出」页。</template>
            <template v-else>隧道未运行。点击「连接」建立到服务器的出口。</template>
          </div>
        </div>
        <div class="row" style="flex: 0 0 auto">
          <button class="btn" :disabled="!canConnect || acting" @click="doConnect">
            {{ canDisconnect ? "已连接" : "连接" }}
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
            <span class="stat-label">当前服务器</span>
            <span class="stat-value">{{ active?.name || active?.host || "（未命名）" }}</span>
          </div>
          <div class="stat">
            <span class="stat-label">地址</span>
            <span class="stat-value">{{ active?.username }}@{{ active?.host }}:{{ active?.port }}</span>
          </div>
          <div class="stat">
            <span class="stat-label">本地 SOCKS 端口</span>
            <span class="stat-value">127.0.0.1:{{ cfg?.settings?.socks_port }}</span>
          </div>
        </div>
        <p class="muted" style="margin-top: 10px">
          共 {{ cfg?.servers?.length ?? 0 }} 台服务器；本地端口为全局设置，切换服务器时不变。
          到「服务器」页可切换当前出口。
        </p>
      </template>

      <template v-else>
        <div class="notice info">
          <p>
            <b>本工具不提供服务器</b>，需要你自己准备一台能 SSH 登录的 Linux 服务器。
            开始前请先确认这三件事：
          </p>
          <ol class="checklist">
            <li><b>一台 Linux 服务器</b> —— 有公网 IP、能 SSH 登录，并且能访问外网</li>
            <li><b>一把 SSH 私钥</b> —— 登录该服务器用（本工具只保存路径，不读取内容）</li>
            <li><b>登录信息</b> —— 服务器地址、SSH 端口、用户名</li>
          </ol>
          <p>
            不确定缺哪一步？先跑一遍「环境自检」，它会逐项检查并给出补齐步骤。
          </p>
        </div>
        <div class="row">
          <button class="btn" @click="$emit('go-doctor')">先做环境自检</button>
          <button class="btn" @click="$emit('go-server')">已准备好，去配置服务器</button>
        </div>
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
          <span :class="['stat-value', directIp ? '' : 'dim']">{{ directIp ?? "未取得" }}</span>
        </div>
      </div>
      <div v-if="egressSuspect" class="notice">
        <p>
          隧道出口与本机直连出口<b>相同</b>。这通常意味着出口路径重叠，
          并不能证明流量确实经过了服务器。请前往「网络诊断」进一步核查。
        </p>
      </div>
      <p v-else-if="directUnavailable" class="muted" style="margin-top: 12px">
        本机直连对照未取得，无法比对两个出口是否相同。这通常是本机直连被网络策略
        拦截所致，<b>与网关无关</b>——隧道出口已实测为上方地址。
      </p>
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
          <!-- 辅助步骤（advisory）失败时用中性标记：它不参与结论判定，
               渲染成红色 ✗ 会与「全部通过」徽标冲突（用户实测反馈过）。 -->
          <span
            :class="['step-mark', s.ok ? 'ok' : s.advisory ? 'info' : 'err']"
          >{{ s.ok ? "✓" : s.advisory ? "–" : "✗" }}</span>
          <div class="step-body">
            <div class="step-label">
              {{ s.label }}
              <span v-if="s.advisory" class="muted">（参考项）</span>
            </div>
            <div class="step-detail">{{ s.detail }} · {{ s.timestamp }}</div>
          </div>
        </div>
      </div>
      <p class="muted" style="margin-top: 14px">
        「隧道测试」只证明隧道本身可用；具体应用是否走了网关，见「应用」页的路由验证。
      </p>
      <p v-if="hasAdvisoryFailure" class="muted" style="margin-top: 6px">
        「参考项」不参与通过判定：本机对照出口取不到时，隧道仍可能完全正常
        （通常是本机直连被网络策略拦截，与网关无关）。
      </p>
    </div>
  </div>
</template>
