// CDP 端到端：驱动真实 Tauri 窗口完成「配置 → HostKey 流程 → 连接 → 验证 → 断开」。
// 开头先清理任何残留连接状态；connect 后轮询快照直到终态。
import http from "http";


// --- 路径推导（由 scripts/redact-e2e-paths.py 注入，勿手改）---
// 脚本可能被从任意工作目录调用，所以路径一律相对本文件解析。
import { fileURLToPath } from "node:url";
import { dirname, resolve as resolvePath, join as joinPath } from "node:path";

const __dirname = dirname(fileURLToPath(import.meta.url));
/** 仓库根目录（tests/e2e → 仓库根需要上溯两级）。 */
const REPO_ROOT = resolvePath(__dirname, "..", "..");
/** Docker 夹具用的测试私钥（仅测试用途，不含任何真实凭据）。 */
const FIXTURE_KEY = joinPath(REPO_ROOT, "tests", "fixtures", "ssh-server", "keys", "id_test_ed25519");
/** 截图输出目录。 */
const SHOT_DIR = joinPath(REPO_ROOT, "docs", "screenshots");
// --- 路径推导结束 ---

function getJson(url) {
  return new Promise((resolve, reject) => {
    http.get(url, (res) => {
      let data = "";
      res.on("data", (c) => (data += c));
      res.on("end", () => resolve(JSON.parse(data)));
    }).on("error", reject);
  });
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function main() {
  const tabs = await getJson("http://127.0.0.1:9223/json/list");
  const page = tabs.find((t) => t.title === "LostCodexGateway");
  if (!page) throw new Error("LostCodexGateway tab not found");
  const ws = new WebSocket(page.webSocketDebuggerUrl);
  let id = 0;
  const pending = new Map();

  const send = (method, params) =>
    new Promise((resolve) => {
      const mid = ++id;
      pending.set(mid, resolve);
      ws.send(JSON.stringify({ id: mid, method, params }));
    });

  ws.onmessage = (ev) => {
    const msg = JSON.parse(ev.data);
    if (msg.id && pending.has(msg.id)) {
      pending.get(msg.id)(msg);
      pending.delete(msg.id);
    }
  };
  await new Promise((r) => (ws.onopen = r));

  async function tauriCallSafe(cmd, args) {
    const r = await send("Runtime.evaluate", {
      expression: `window.__TAURI_INTERNALS__.invoke(${JSON.stringify(cmd)}, ${
        args === undefined ? "undefined" : JSON.stringify(args)
      }).then(v => ({ok: true, v})).catch(e => ({ok: false, e: String(e)}))`,
      awaitPromise: true,
      returnByValue: true,
    });
    const res = r.result?.result;
    return res?.value ?? { ok: false, e: "no result" };
  }

  async function getState() {
    const r = await tauriCallSafe("get_snapshot", undefined);
    return r.v?.state;
  }

  const step = (name) => console.log(`\n[${name}]`);

  step("0. 清理残留状态（若有连接则先断开）");
  let state = await getState();
  console.log("  初始状态:", state);
  if (["CONNECTING", "TUNNEL_READY", "EGRESS_VERIFIED", "DEGRADED", "RECONNECTING"].includes(state)) {
    await tauriCallSafe("disconnect", undefined);
    await sleep(1500);
    console.log("  断开后:", await getState());
  }

  step("1. 保存服务器配置（指向 Docker 夹具）");
  console.log("  ->", JSON.stringify(await tauriCallSafe("save_server_config", {
    config: {
      host: "127.0.0.1", port: 2222, username: "testuser",
      key_path: FIXTURE_KEY,
      socks_port: 17801,
      ssh_exe_path: "C:\\\\Windows\\\\System32\\\\OpenSSH\\\\ssh.exe",
      server_name: "Docker 夹具",
    },
  })));

  step("2. 尝试连接");
  let firstConnect = await tauriCallSafe("connect", undefined);
  console.log("  ->", JSON.stringify(firstConnect));
  if (firstConnect.ok && String(firstConnect.v).includes("Host Key")) {
    console.log("  指纹未确认/变化被正确阻断 ✅（安全设计）");
    step("3. 确认新指纹（写前备份 known_hosts）");
    const hk = await tauriCallSafe("fetch_host_key", undefined);
    console.log("  fetch:", JSON.stringify(hk));
    const confirmed = await tauriCallSafe("confirm_host_key", undefined);
    console.log("  confirm:", JSON.stringify(confirmed));
    if (!confirmed.ok) throw new Error("确认指纹失败: " + confirmed.e);
    step("4. 重新连接");
    firstConnect = await tauriCallSafe("connect", undefined);
    console.log("  ->", JSON.stringify(firstConnect));
  }
  if (!firstConnect.ok) throw new Error("连接失败: " + firstConnect.e);

  step("5. 轮询快照直到终态（最多 60 秒）");
  let snap = null;
  for (let i = 0; i < 60; i++) {
    const r = await tauriCallSafe("get_snapshot", undefined);
    snap = r.v;
    if (["EGRESS_VERIFIED", "DEGRADED", "ERROR", "DISCONNECTED"].includes(snap.state)) break;
    await sleep(1000);
  }
  console.log("  state:", snap.state);
  console.log("  egress_ip:", snap.last_verify?.egress_ip);
  for (const s of snap.last_verify?.steps ?? []) {
    console.log(`    [${s.ok ? "OK" : "FAIL"}] ${s.label} -> ${s.detail}`);
  }
  if (snap.state !== "EGRESS_VERIFIED") throw new Error("未达到 EGRESS_VERIFIED，而是 " + snap.state);
  if (!snap.last_verify?.egress_ip) throw new Error("出口 IP 为空");

  step("6. 断开（只清理本工具创建的进程）");
  console.log("  ->", JSON.stringify(await tauriCallSafe("disconnect", undefined)));
  await sleep(1500);
  const snap2 = (await tauriCallSafe("get_snapshot", undefined)).v;
  console.log("  断开后状态:", snap2.state);
  if (snap2.state !== "READY") throw new Error("断开后未回到 READY，而是 " + snap2.state);

  step("7. 导出脱敏诊断报告");
  console.log("  ->", JSON.stringify(await tauriCallSafe("export_diagnostics", undefined)));

  console.log("\n=== GUI 端到端全部通过 ===");
  ws.close();
}
main().catch((e) => {
  console.error("FAILED:", e.message);
  process.exit(1);
});
