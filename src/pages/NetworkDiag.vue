<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import type { DiagReport, DiagStatus, RoutingStatus, ClientDiag, PathHop, DiagItem } from "../types/diag";

const report = ref(null as DiagReport | null);
const running = ref(false);
const error = ref("");
const secret = ref("");
const secretUsed = ref(false);
const lastLoaded = ref("");
const logFilterLevel = ref("all");
const logFilterKey = ref("");
const logFilterTs = ref("");

// 可配置预期出口 IP（设置区）
const expectedIp = ref("");
const expectedMsg = ref("");

onMounted(async () => {
  await loadLast();
});

async function loadLast() {
  try {
    const r = await invoke<DiagReport | null>("get_last_diagnostics");
    if (r) {
      report.value = r;
      lastLoaded.value = `上次诊断 ${r.started_at}（耗时 ${r.duration_ms}ms）`;
    }
  } catch (e) {
    error.value = String(e);
  }
}

async function runDiag() {
  running.value = true;
  error.value = "";
  secretUsed.value = secret.value.trim().length > 0;
  try {
    const r = await invoke<DiagReport>("run_diagnostics", {
      mihomoSecret: secret.value.trim() ? secret.value : null,
    });
    report.value = r;
    lastLoaded.value = `本次诊断 ${r.started_at}（耗时 ${r.duration_ms}ms）`;
    secret.value = ""; // 用后清空输入框（secret 不落盘）
  } catch (e) {
    error.value = String(e);
  } finally {
    running.value = false;
  }
}

async function diagnoseOne(kind: string) {
  running.value = true;
  error.value = "";
  try {
    const c = await invoke<ClientDiag>("diagnose_client", { kind });
    if (report.value) {
      const idx = report.value.clients.findIndex((x) => x.kind === kind);
      if (idx >= 0) report.value.clients[idx] = c;
    }
  } catch (e) {
    error.value = String(e);
  } finally {
    running.value = false;
  }
}

async function saveExpectedIp() {
  expectedMsg.value = "";
  try {
    expectedMsg.value = await invoke<string>("set_expected_egress_ip", { ip: expectedIp.value.trim() || null });
    await runDiag();
  } catch (e) {
    expectedMsg.value = String(e);
  }
}

async function exportReport() {
  try {
    const path = await invoke<string>("export_diagnostics");
    expectedMsg.value = `已导出：${path}`;
  } catch (e) {
    expectedMsg.value = String(e);
  }
}

const statusClass = (s: DiagStatus) => {
  switch (s) {
    case "ok": return "ok";
    case "warn": return "warn";
    case "error": return "err";
    default: return "info";
  }
};
const routingText = (r: RoutingStatus) => {
  switch (r) {
    case "verified": return "已验证";
    case "partial": return "部分验证";
    case "anomaly": return "路由异常";
    case "unconfirmable": return "无法确认";
    default: return "未验证";
  }
};
const routingClass = (r: RoutingStatus) => {
  switch (r) {
    case "verified": return "ok";
    case "partial": return "warn";
    case "anomaly": return "err";
    default: return "info";
  }
};

// 汇总：状态总览 8 项
const overview = computed(() => {
  const r = report.value;
  if (!r) return [];
  const matchResultText =
    r.egress.match_result === "matched" ? "匹配" :
    r.egress.match_result === "mismatch" ? "不匹配" : "无法确认";
  return [
    { label: "SSH 隧道", status: r.tunnel_status as DiagStatus, detail: tunnelSummary(r) },
    { label: "本地代理", status: (r.tunnel_items.find((i) => i.key === "socks_handshake")?.status ?? "unknown") as DiagStatus, detail: "127.0.0.1 SOCKS5 握手" },
    { label: "云服务器", status: (r.server.reachable ? "ok" : "error") as DiagStatus, detail: r.server.reachable ? "可连接（只读检测通过）" : "无法连接" },
    { label: "出口 IP", status: (r.egress.gateway_ip ? "ok" : "error") as DiagStatus, detail: `${r.egress.gateway_ip ?? "无"}（${matchResultText}）` },
    { label: "DNS", status: (r.dns_items.find((i) => i.key === "dns_local")?.status ?? "unknown") as DiagStatus, detail: r.dns_items.find((i) => i.key === "dns_local")?.detail ?? "" },
    { label: "Codex Desktop", status: clientStatus(r, "desktop"), detail: routingText(clientOf(r, "desktop")?.routing ?? "unverified") },
    { label: "Codex CLI", status: clientStatus(r, "cli"), detail: routingText(clientOf(r, "cli")?.routing ?? "unverified") },
    { label: "Codex IDE 插件", status: clientStatus(r, "ide"), detail: routingText(clientOf(r, "ide")?.routing ?? "unverified") },
  ];
});

function clientOf(r: DiagReport, kind: string) {
  return r.clients.find((c) => c.kind === kind);
}
function clientStatus(r: DiagReport, kind: string): DiagStatus {
  const c = clientOf(r, kind);
  if (!c || !c.running) return "unknown";
  switch (c.routing) {
    case "verified": return "ok";
    case "partial": return "warn";
    case "anomaly": return "error";
    default: return "unknown";
  }
}
function tunnelSummary(r: DiagReport): string {
  if (r.tunnel_status === "ok") return "已连接且 SOCKS 转发可用";
  if (r.tunnel_status === "warn") return "隧道异常：进程存活但转发不可用";
  if (r.tunnel_status === "error") return "连接异常";
  return "未连接";
}

// 日志视图：合并各检测项为可筛选列表
const logEntries = computed(() => {
  const r = report.value;
  if (!r) return [];
  const entries: { id: string; ts: string; level: DiagStatus; key: string; text: string }[] = [];
  const push = (arr: DiagItem[], prefix: string) => {
    for (const i of arr) {
      entries.push({ id: `${prefix}.${i.key}`, ts: i.ts, level: i.status, key: `${prefix}.${i.key}`, text: `${i.label}：${i.detail}${i.latency_ms != null ? `（${i.latency_ms}ms）` : ""}` });
    }
  };
  push(r.tunnel_items, "tunnel");
  push(r.egress.items, "egress");
  push(r.server.items, "server");
  push(r.mihomo.items, "mihomo");
  push(r.dns_items, "dns");
  entries.push({ id: "meta", ts: r.finished_at, level: "ok", key: "meta", text: `诊断完成：${r.duration_ms}ms；桥接 ${r.gateway_ready ? "可用" : "不可用"}；Mihomo 连接 ${r.mihomo.connections_total} 条（命中网关组 ${r.mihomo.gateway_matched}，Codex 相关 ${r.mihomo.codex_related}）` });
  r.advisories.forEach((a, i) => {
    entries.push({ id: `advisory.${i}`, ts: r.finished_at, level: "warn", key: "advisory", text: a });
  });
  return entries;
});

const filteredLogs = computed(() => {
  return logEntries.value.filter((e) => {
    if (logFilterLevel.value !== "all" && e.level !== logFilterLevel.value) return false;
    if (logFilterKey.value && !e.key.includes(logFilterKey.value)) return false;
    if (logFilterTs.value && !e.ts.includes(logFilterTs.value)) return false;
    return true;
  });
});

function hopStyle(h: PathHop): DiagStatus {
  return h.status;
}
</script>

<template>
  <div>
    <div class="page-head">
      <div>
        <h1 class="page-title">网络诊断</h1>
        <p class="page-desc">
          全部结果来自真实网络检测。SSH 已连接 ≠ Codex 已走网关；无法确认的路由显示「未验证」。
        </p>
      </div>
      <button class="btn" :disabled="running" @click="runDiag">
        {{ running ? "诊断中…（最长 75 秒，不会卡死）" : "开始诊断" }}
      </button>
    </div>

    <div class="card">
      <h2>
        诊断参数
        <span v-if="report" class="status-pill info">耗时 {{ report.duration_ms }}ms</span>
      </h2>
      <div class="row">
        <label class="field">Mihomo external-controller secret（可选，仅本次内存使用，不落盘）
          <input type="password" v-model="secret" placeholder="（有 secret 时填写以读取只读连接信息）" style="min-width: 300px" />
        </label>
      </div>
      <p v-if="error" class="notice err mono">{{ error }}</p>
      <p v-if="lastLoaded" class="muted">{{ lastLoaded }}</p>
    </div>

    <!-- 2.1 网络状态总览 -->
    <div class="card">
      <h2>网络状态总览</h2>
      <div class="stat-grid cols-4">
        <div v-for="o in overview" :key="o.label" class="stat">
          <span :class="['status-pill', statusClass(o.status)]">{{ o.label }}</span>
          <span class="stat-value dim" style="margin-top: 8px; font-family: var(--font-ui); font-size: var(--fs-base); line-height: 1.45">
            {{ o.detail }}
          </span>
        </div>
      </div>
      <p v-if="!report" class="empty">尚无数据，点击右上角「开始诊断」。</p>
      <p v-if="report && !report.gateway_ready && report.tunnel_status === 'ok'" class="notice">
        SSH 隧道正常，但目标应用尚未验证使用该出口。
      </p>
      <p v-if="report && report.egress.match_result === 'mismatch'" class="notice">
        网关出口 IP 不匹配：当前请求可能未使用预期出口。
      </p>
      <div v-for="a in report?.advisories ?? []" :key="a" class="notice">{{ a }}</div>
    </div>

    <!-- 2.2 网络路径检测 -->
    <div class="card">
      <h2>网络路径检测</h2>
      <div class="row" style="align-items: stretch">
        <template v-for="(h, i) in report?.path_hops ?? []" :key="h.name">
          <div class="stat" style="flex: 1 1 130px; min-width: 130px; text-align: center">
            <span :class="['status-pill', hopStyle(h)]">{{ h.name }}</span>
            <span class="stat-value" style="margin-top: 8px">{{ h.latency_ms != null ? `${h.latency_ms}ms` : "—" }}</span>
            <span class="muted" style="font-size: 11px; line-height: 1.4; display: block; margin-top: 4px">{{ h.detail }}</span>
          </div>
          <div v-if="i < (report?.path_hops?.length ?? 0) - 1" class="muted" style="align-self: center">→</div>
        </template>
      </div>
      <p v-if="!report" class="empty">尚未诊断。</p>
    </div>

    <!-- 2.3 Codex 客户端诊断 -->
    <div class="card">
      <h2>Codex 客户端诊断</h2>
      <div class="row" style="align-items: stretch">
        <div v-for="c in report?.clients ?? []" :key="c.kind" class="readout" style="flex: 1 1 220px; min-width: 220px">
          <div class="row" style="justify-content: space-between">
            <b>{{ c.label }}</b>
            <span :class="['status-pill', c.running ? routingClass(c.routing) : 'info']">
              {{ c.running ? routingText(c.routing) : "未运行" }}
            </span>
          </div>
          <p class="muted">进程数：{{ c.processes.length }} · 最近检测：{{ c.last_checked }}</p>
          <div v-for="e in c.evidence.slice(0, 4)" :key="e" class="muted mono" style="font-size: 11px">{{ e }}</div>
          <p class="muted" style="margin-top: 8px">
            依据：进程信息 + 路径 + Mihomo 连接关联 + 触发测试请求。单条连接不能推断全部流量已转发。
          </p>
          <button class="btn secondary sm" :disabled="running" @click="diagnoseOne(c.kind)">检测</button>
        </div>
      </div>
      <p v-if="!report" class="empty">尚未诊断。</p>
      <p v-if="report && report.mihomo.tun_enabled === false" class="notice">
        TUN 未开启：Desktop / IDE 的进程级分流尚未覆盖（如实报告，不含糊其辞）。
      </p>
    </div>

    <!-- 2.4 诊断日志 -->
    <div class="card">
      <h2>诊断日志（脱敏）</h2>
      <div class="row">
        <label class="field">级别
          <select v-model="logFilterLevel">
            <option value="all">全部</option>
            <option value="ok">正常</option>
            <option value="warn">需要注意</option>
            <option value="error">异常</option>
            <option value="unknown">未验证</option>
          </select>
        </label>
        <label class="field">检测项目
          <input type="text" v-model="logFilterKey" placeholder="如 tunnel / egress / dns" />
        </label>
        <label class="field">时间
          <input type="text" v-model="logFilterTs" placeholder="如 17:3" />
        </label>
        <button class="btn secondary" @click="exportReport">导出脱敏诊断报告</button>
        <span v-if="expectedMsg" class="muted mono">{{ expectedMsg }}</span>
      </div>
      <div class="logbox" style="max-height: 260px; margin-top: 10px">
        <div v-for="e in filteredLogs" :key="e.id" class="ln">
          <span class="t">{{ e.ts }}</span>
          <span
            class="msg"
            :class="e.level === 'error' ? 'e' : e.level === 'warn' ? 'w' : ''"
          >[{{ e.key }}] {{ e.text }}</span>
        </div>
        <div v-if="filteredLogs.length === 0" class="muted">暂无日志（先点「开始诊断」）。</div>
      </div>
    </div>

    <!-- 预期出口 IP 配置 -->
    <div class="card">
      <h2>预期服务器出口 IP（可选）</h2>
      <div class="notice info">
        出口 IP 检测要求（需求文档 §3.2）：代理出口必须显式指定 SOCKS5 + 远端 DNS，
        绝不把服务器地址当已验证出口。
      </div>
      <div class="row" style="margin-top: 12px">
        <label class="field">预期出口 IP（空 = 不校验；填入后诊断会做匹配判定）
          <input type="text" v-model="expectedIp" placeholder="如 1.2.3.4" />
        </label>
        <button class="btn secondary" @click="saveExpectedIp">保存并重新诊断</button>
      </div>
    </div>
  </div>
</template>
