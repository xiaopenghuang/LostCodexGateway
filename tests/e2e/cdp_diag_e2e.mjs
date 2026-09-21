// 网络诊断 GUI e2e：真实窗口内 连接 → 诊断 → 验证报告 → 截图数据导出
import http from "http";
import { execFileSync } from "child_process";


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

  async function call(cmd, args) {
    const r = await send("Runtime.evaluate", {
      expression: `window.__TAURI_INTERNALS__.invoke(${JSON.stringify(cmd)}, ${
        args === undefined ? "undefined" : JSON.stringify(args)
      }).then(v => ({ok: true, v})).catch(e => ({ok: false, e: String(e)}))`,
      awaitPromise: true,
      returnByValue: true,
    });
    return r.result.result.value;
  }
  async function getSnap() {
    return (await call("get_snapshot", undefined)).v;
  }
  const step = (name) => console.log(`\n[${name}]`);

  step("0. 清理 + 配置 + 连接");
  let snap = await getSnap();
  if (!["READY", "UNCONFIGURED"].includes(snap.state)) {
    await call("disconnect", undefined);
    await sleep(1500);
  }
  await call("save_server_config", {
    config: {
      host: "127.0.0.1", port: 2222, username: "testuser",
      key_path: FIXTURE_KEY,
      socks_port: 17801,
      ssh_exe_path: "C:\\\\Windows\\\\System32\\\\OpenSSH\\\\ssh.exe",
      server_name: "Docker 夹具",
    },
  });
  let c = await call("connect", undefined);
  if (!c.ok && String(c.e || c.v).includes("Host Key")) {
    await call("confirm_host_key", undefined);
    c = await call("connect", undefined);
  }
  for (let i = 0; i < 60; i++) {
    snap = await getSnap();
    if (["EGRESS_VERIFIED", "DEGRADED", "ERROR"].includes(snap.state)) break;
    await sleep(1000);
  }
  console.log("  状态:", snap.state, "出口:", snap.last_verify?.egress_ip);

  step("1. 运行网络诊断");
  const t0 = Date.now();
  const r = await call("run_diagnostics", { mihomoSecret: null });
  console.log("  诊断耗时:", Date.now() - t0, "ms");
  if (!r.ok) throw new Error("诊断失败: " + r.e);
  const d = r.v;

  step("2. 关键断言");
  const checks = [
    ["隧道状态", d.tunnel_status, "ok"],
    ["隧道子项全部通过", d.tunnel_items.every((i) => i.status === "ok"), true],
    ["网关出口 IP 存在", !!d.egress.gateway_ip, true],
    ["服务器可达", d.server.reachable, true],
    ["路径 4 跳", d.path_hops.length, 4],
    ["诊断时长字段", d.duration_ms > 0, true],
  ];
  for (const [name, got, want] of checks) {
    const pass = String(got) === String(want);
    console.log(`  ${pass ? "PASS" : "FAIL"} ${name}: ${JSON.stringify(got)}`);
    if (!pass) throw new Error(name);
  }
  console.log("  本地出口:", d.egress.local_ip ?? "(直连受限)", "| 网关出口:", d.egress.gateway_ip, `(${d.egress.gateway_version})`);
  console.log("  匹配结果:", d.egress.match_result, "| 预期IP:", d.egress.expected_ip ?? "(未配置)");
  console.log("  延迟: socks握手", d.latencies.socks_handshake_ms, "ms | 网关HTTPS", d.latencies.gateway_https_ms, "ms | 服务器", d.latencies.server_https_ms, "ms");
  console.log("  客户端:", d.clients.map((c) => `${c.label}=${c.routing}(${c.running ? "运行" : "未运行"})`).join(", "));
  console.log("  Mihomo:", JSON.stringify({ running: d.mihomo.running, tun: d.mihomo.tun_enabled, conns: d.mihomo.connections_total }));

  step("3. 预期出口 IP 不匹配场景");
  const setR = await call("set_expected_egress_ip", { ip: "203.0.113.99" });
  console.log("  设置预期IP:", JSON.stringify(setR));
  const d2 = (await call("run_diagnostics", { mihomoSecret: null })).v;
  console.log("  匹配结果:", d2.egress.match_result, "| 建议含不匹配提示:", d2.advisories.some((a) => a.includes("不匹配")));
  if (d2.egress.match_result !== "mismatch") throw new Error("不匹配场景未识别");
  await call("set_expected_egress_ip", { ip: null });

  step("4. 断开");
  await call("disconnect", undefined);
  await sleep(1500);
  console.log("  断开后:", (await getSnap()).state);

  console.log("\n=== 网络诊断 GUI e2e 全部通过 ===");
  ws.close();
}
main().catch((e) => {
  console.error("FAILED:", e.message);
  process.exit(1);
});
