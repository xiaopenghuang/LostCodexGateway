<script setup lang="ts">
/**
 * 环境自检（环境体检）。
 *
 * 目的：让一台**新电脑**拿到本软件后，用户能一眼看出「还差哪一步」。
 *
 * 交互设计遵循一条原则：**能自动的不让用户动手，不能自动的讲清楚怎么做**。
 * - `fix === "auto"` → 渲染一个按钮，点击跳到能完成该步骤的页面
 * - `fix === "guide"` → 渲染编号步骤列表 + 一键复制
 * 绝不出现「报红但没说怎么办」的项（后端有测试兜住这条约束）。
 */
import { computed, onMounted, ref } from "vue";
import { store, runPreflight } from "../stores/gateway";
import type { PreflightItem, PreflightStatus } from "../types/preflight";

const props = defineProps<{ goTab?: (key: string) => void }>();

const copied = ref("");

const report = computed(() => store.preflight);
const busy = computed(() => store.preflightBusy);

/** 状态 → 中文标签 */
const statusLabel = (s: PreflightStatus): string => {
  switch (s) {
    case "ok": return "通过";
    case "warn": return "提示";
    case "error": return "缺失";
    default: return "待判定";
  }
};

/** 状态 → status-pill 的类名 */
const statusClass = (s: PreflightStatus): string => {
  switch (s) {
    case "ok": return "ok";
    case "warn": return "warn";
    case "error": return "err";
    default: return "info";
  }
};

/** 排序：先显示「有问题」的，通过的沉底——用户最关心缺什么。 */
const ordered = computed<PreflightItem[]>(() => {
  const rank: Record<PreflightStatus, number> = { error: 0, warn: 1, unknown: 2, ok: 3 };
  return [...(report.value?.items ?? [])].sort((a, b) => {
    if (a.blocking !== b.blocking) return a.blocking ? -1 : 1;
    return rank[a.status] - rank[b.status];
  });
});

/**
 * 后端给的动作标识 → 实际跳转。
 *
 * 后端不返回 TabKey（它不该知道前端路由），只返回语义化动作名，
 * 由这里映射。未知动作一律忽略，不跳错页。
 */
const actionTargets: Record<string, string> = {
  open_server_page: "server",
  open_settings_page: "settings",
  open_apps_page: "apps",
};

function runAction(action: string | null) {
  if (!action) return;
  const target = actionTargets[action];
  if (target && props.goTab) props.goTab(target);
}

async function copySteps(item: PreflightItem) {
  const text = item.steps.map((s, i) => `${i + 1}. ${s}`).join("\n");
  try {
    await navigator.clipboard.writeText(text);
    copied.value = item.key;
    setTimeout(() => {
      if (copied.value === item.key) copied.value = "";
    }, 2000);
  } catch {
    copied.value = "";
  }
}

async function refresh() {
  try {
    await runPreflight();
  } catch (e) {
    // 浏览器 dev 模式（无 Tauri 后端）下不至于白屏
    console.warn("preflight failed:", e);
  }
}

onMounted(refresh);
</script>

<template>
  <div>
    <div class="page-head">
      <div>
        <h1 class="page-title">环境自检</h1>
        <p class="page-desc">
          逐项检查跑通所需的全部前置条件，并告诉你每一项怎么补。全程只读：不装东西、不改系统设置、不提权。
        </p>
      </div>
      <div class="row">
        <button class="btn" :disabled="busy" @click="refresh">
          {{ busy ? "检查中…" : "重新体检" }}
        </button>
      </div>
    </div>

    <div v-if="!report" class="card">
      <p class="muted">尚未体检。点右上角「重新体检」开始。</p>
    </div>

    <template v-else>
      <div class="card" :class="{ highlight: !report.ready }">
        <h2>
          体检结论
          <span class="status-pill" :class="report.ready ? 'ok' : 'err'">
            {{ report.ready ? "可以正常使用" : `${report.blocking_failed} 项待处理` }}
          </span>
        </h2>
        <p :class="report.ready ? 'muted' : ''">{{ report.summary }}</p>
        <div class="stat-grid cols-3">
          <div class="stat">
            <span class="stat-label">通过</span>
            <span class="stat-value">{{ report.passed }} / {{ report.items.length }}</span>
          </div>
          <div class="stat">
            <span class="stat-label">阻塞项</span>
            <span class="stat-value">{{ report.blocking_failed }}</span>
          </div>
          <div class="stat">
            <span class="stat-label">检查时间</span>
            <span class="stat-value dim">{{ report.ts }}</span>
          </div>
        </div>
      </div>

      <div class="card">
        <h2>检查明细</h2>
        <div class="pf-list">
          <div
            v-for="item in ordered"
            :key="item.key"
            class="pf-item"
            :class="{ blocking: item.blocking && item.status !== 'ok' }"
          >
            <div class="pf-head">
              <span class="pf-mark" :class="statusClass(item.status)">
                {{ item.status === "ok" ? "✓" : item.status === "error" ? "!" : "·" }}
              </span>
              <span class="pf-label">{{ item.label }}</span>
              <span v-if="item.blocking" class="status-pill err">必需</span>
              <span class="status-pill" :class="statusClass(item.status)">
                {{ statusLabel(item.status) }}
              </span>
            </div>
            <p class="pf-detail">{{ item.detail }}</p>

            <div v-if="item.fix === 'auto' && item.action" class="row pf-act">
              <button class="btn sm" @click="runAction(item.action)">
                {{ item.action === "open_server_page" ? "去「服务器」页处理"
                  : item.action === "open_settings_page" ? "去「设置」页处理"
                  : "去处理" }}
              </button>
              <span v-if="item.steps.length" class="muted">也可按下方步骤手动完成</span>
            </div>

            <div v-if="item.steps.length" class="pf-steps">
              <div class="pf-steps-head">
                <span class="muted">操作步骤</span>
                <button class="btn ghost sm" @click="copySteps(item)">
                  {{ copied === item.key ? "已复制" : "复制步骤" }}
                </button>
              </div>
              <ol>
                <li v-for="(s, i) in item.steps" :key="i">{{ s }}</li>
              </ol>
            </div>
          </div>
        </div>
      </div>

      <div class="card">
        <h2>关于「只读」的说明</h2>
        <p class="muted">
          本页只做检测：查文件是否存在、跑 <span class="mono">ssh -V</span> 取版本、尝试 TCP
          连接看端口是否通、读 Clash Verge 的 profile 找规则片段。
          <strong>不会</strong>安装任何程序、<strong>不会</strong>修改注册表或系统代理、
          <strong>不会</strong>修改你的 Clash 配置、<strong>不会</strong>提权。
        </p>
        <p class="muted">
          安装类与凭据类操作（装 OpenSSH、装 Codex、生成 SSH 密钥、上传公钥到服务器）
          本页<strong>只给步骤、不代劳</strong>——它们要么会写你的磁盘，要么会改你的服务器，
          要么会撑大安装包体积。所以你会看到「复制步骤」按钮，而不是「一键安装」。
        </p>
        <p class="muted">
          涉及「需要你决策」或「需要在服务器上动手」的步骤（生成私钥、核对 Host Key、
          放行防火墙），一律给出步骤由你亲手完成——这些环节无法也不应被自动化。
        </p>
      </div>
    </template>
  </div>
</template>

<style scoped>
.pf-list {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.pf-item {
  border: 1px solid var(--border, rgba(0, 0, 0, 0.1));
  border-radius: 10px;
  padding: 12px 14px;
}

.pf-item.blocking {
  border-color: var(--danger, #d9534f);
}

.pf-head {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}

.pf-mark {
  width: 18px;
  height: 18px;
  border-radius: 50%;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  font-size: 12px;
  line-height: 1;
  flex: 0 0 auto;
}

.pf-mark.ok {
  background: rgba(40, 140, 80, 0.14);
  color: #1f7a44;
}

.pf-mark.warn {
  background: rgba(200, 140, 20, 0.16);
  color: #8a6100;
}

.pf-mark.err {
  background: rgba(200, 60, 60, 0.14);
  color: #a52a2a;
}

.pf-mark.info {
  background: rgba(0, 0, 0, 0.06);
  color: var(--text-dim, #666);
}

.pf-label {
  font-weight: 500;
}

.pf-detail {
  margin: 8px 0 0;
  font-size: 13px;
  line-height: 1.6;
  word-break: break-all;
}

.pf-act {
  margin-top: 10px;
  align-items: center;
  gap: 10px;
}

.pf-steps {
  margin-top: 10px;
  padding-top: 10px;
  border-top: 1px dashed var(--border, rgba(0, 0, 0, 0.1));
}

.pf-steps-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
}

.pf-steps ol {
  margin: 6px 0 0;
  padding-left: 22px;
}

.pf-steps li {
  font-size: 13px;
  line-height: 1.7;
  word-break: break-all;
}
</style>
