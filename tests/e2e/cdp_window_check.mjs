// 真实验证：修复后触发所有会拉起子进程的功能，监测是否有控制台窗口闪现
// 监测对象：ssh.exe / ssh-keygen.exe / ssh-keyscan.exe / powershell.exe / tasklist.exe / conhost.exe
// 判定：MainWindowHandle != 0 即出现可见窗口（黑框）
import http from "http";
import { execFile } from "child_process";

const getJson = (url) =>
  new Promise((resolve, reject) => {
    http.get(url, (res) => {
      let d = "";
      res.on("data", (c) => (d += c));
      res.on("end", () => resolve(JSON.parse(d)));
    }).on("error", reject);
  });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const tabs = await getJson("http://127.0.0.1:9223/json/list");
const page = tabs.find((t) => t.title === "LostCodexGateway");
if (!page) throw new Error("app tab not found");
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

// --- 窗口监测器 ---
const PS_POLL =
  "Get-Process ssh,ssh-keygen,ssh-keyscan,powershell,tasklist,conhost -ErrorAction SilentlyContinue | Where-Object { $_.MainWindowHandle -ne 0 } | ForEach-Object { \"$($_.Id)|$($_.ProcessName)|$($_.MainWindowTitle)\" }";
let sightings = [];
let polling = false;
const seenIds = new Set();
function pollOnce() {
  return new Promise((resolve) => {
    execFile("powershell.exe", ["-NoProfile", "-Command", PS_POLL], { windowsHide: true }, (err, stdout) => {
      // Get-Process 列表含当前不存在的进程名时退出码非零（err 被置位），
      // 但 stdout 仍可能包含有效窗口行——绝不能因 err 丢弃 stdout
      resolve(stdout ? stdout.trim() : "");
    });
  });
}
async function startPolling(ms) {
  polling = true;
  while (polling) {
    const s = await pollOnce();
    if (s) {
      for (const line of s.split(/\r?\n/)) {
        const t = line.trim();
        if (t && !seenIds.has(t)) {
          seenIds.add(t);
          sightings.push({ t, at: new Date().toISOString() });
        }
      }
    }
    await sleep(ms);
  }
}
const poller = startPolling(200);

// --- 配置并连接 ---
await call("save_server_config", {
  config: {
    host: "127.0.0.1", port: 2222, username: "testuser",
    key_path: "I:\\\\开发\\\\LostCodexGateway\\\\tests\\\\fixtures\\\\ssh-server\\\\keys\\\\id_test_ed25519",
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
let snap = await getSnap();
for (let i = 0; i < 60; i++) {
  if (["EGRESS_VERIFIED", "DEGRADED", "ERROR"].includes(snap.state)) break;
  await sleep(1000);
  snap = await getSnap();
}
console.log("[1] connect:", snap.state, "egress:", snap.last_verify?.egress_ip);

// --- 触发各类子进程动作 ---
console.log("[2] run_diagnostics ...");
const diag = await call("run_diagnostics", { mihomo_secret: null });
console.log("    diag ok:", diag.ok, "duration_ms:", diag.v?.duration_ms, "tunnel:", diag.v?.tunnel_status);
console.log("[3] diagnose_client x3 ...");
for (const k of ["desktop", "cli", "ide"]) {
  const r = await call("diagnose_client", { clientKind: k });
  console.log("   ", k, r.ok, JSON.stringify(r.v)?.slice(0, 120));
}
console.log("[4] detect_mihomo ...");
const m = await call("detect_mihomo", undefined);
console.log("    mihomo:", m.ok, "running:", m.v?.mihomo_running);
console.log("[5] test_connection ...");
const t = await call("test_connection", undefined);
console.log("    test_conn:", t.ok, String(t.v || t.e)?.slice(0, 100));
console.log("[6] fetch_host_key (ssh-keyscan) ...");
const hk = await call("fetch_host_key", undefined);
console.log("    host_key:", hk.ok, String(hk.v || hk.e)?.slice(0, 100));

await sleep(2500); // 让最后一次轮询覆盖所有动作窗口期
polling = false;
await poller;

// --- 敏感度自检：故意开一个真实控制台窗口，监测器必须能捕获 ---
// 用 WScript.Shell.Run 走 ShellExecute，强制创建可见窗口
const probe = new Promise((resolve) => {
  execFile(
    "powershell.exe",
    ["-NoProfile", "-Command", "(New-Object -ComObject WScript.Shell).Run('powershell -NoProfile -Command Start-Sleep 5', 1, $false)"],
    { windowsHide: true },
    () => resolve()
  );
});
await probe;
const before = sightings.length;
let probeSeen = false;
for (let i = 0; i < 25; i++) {
  const s = await pollOnce();
  if (s) {
    for (const line of s.split(/\r?\n/)) {
      const t = line.trim();
      if (t && !seenIds.has(t)) {
        seenIds.add(t);
        sightings.push({ t, at: new Date().toISOString() });
      }
    }
    probeSeen = true;
  }
  await sleep(200);
}
const after = sightings.length;
const probeCaught = after > before;
console.log("    [自检] 故意弹出的窗口被监测到:", probeCaught, `(+${after - before} 条)`);

// 判定口径：动作期间（自检前）零窗口 AND 自检窗口确实被捕获（监测器可信）
const actionCount = before;
console.log("\n=== 动作期间捕获到的可见控制台窗口 ===");
if (actionCount === 0) console.log("  无（0 个黑框闪现）✅");
else for (const s of sightings.slice(0, before)) console.log("  ⚠", s.t, s.at);
const verdict = actionCount === 0 && probeCaught;
console.log("RESULT:", verdict ? "PASS" : "FAIL", `(动作期窗口=${actionCount}, 监测器自检=${probeCaught})`);

// 断开，恢复现场
await call("disconnect", undefined);
await sleep(1500);
console.log("已断开隧道");
ws.close();
process.exit(verdict ? 0 : 1);
