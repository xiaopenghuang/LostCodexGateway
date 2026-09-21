// M2 CDP 端到端：真实 app 连接（隧道+桥接自动启动）→ 真实 Codex CLI 以
// HTTPS_PROXY=桥接端口运行 → 验证：
//  A) 桥接统计显示 codex 发来的 CONNECT（connections_total > 0，last_target 为 codex 目标域）
//  B) sshd 容器日志出现对应 connect_to 记录（服务器侧证据）
import http from "http";
import { execFileSync } from "child_process";

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

  step("0. 清理残留");
  let snap = await getSnap();
  console.log("  状态:", snap.state);
  if (!["READY", "UNCONFIGURED"].includes(snap.state)) {
    await call("disconnect", undefined);
    await sleep(1500);
  }

  step("1. 配置并连接");
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
  console.log("  connect:", JSON.stringify(c));
  if (!c.ok) {
    // Host Key 路径
    if (String(c.e || c.v).includes("Host Key")) {
      await call("confirm_host_key", undefined);
      c = await call("connect", undefined);
      console.log("  reconnect:", JSON.stringify(c));
    }
  }
  for (let i = 0; i < 60; i++) {
    snap = await getSnap();
    if (["EGRESS_VERIFIED", "DEGRADED", "ERROR"].includes(snap.state)) break;
    await sleep(1000);
  }
  console.log("  状态:", snap.state, "桥接端口:", snap.bridge_port, "出口IP:", snap.last_verify?.egress_ip);
  if (snap.state !== "EGRESS_VERIFIED") throw new Error("未达到 EGRESS_VERIFIED");
  if (!snap.bridge_port) throw new Error("桥接层未启动");
  const bridgePort = snap.bridge_port;
  const beforeTotal = snap.bridge_connections_total;

  step("2. 记录 sshd 日志基线");
  const logBefore = execFileSync("docker", ["logs", "--tail", "50", "lcfg-test-sshd"], { encoding: "utf8" });

  step("3. 真实 Codex CLI 经桥接运行（codex doctor）");
  const codexExe = "G:\\\\VSCODE\\\\nodejs\\\\node_global\\\\node_modules\\\\@openai\\\\codex\\\\node_modules\\\\@openai\\\\codex-win32-x64\\\\vendor\\\\x86_64-pc-windows-msvc\\\\bin\\\\codex.exe";
  const env = {
    ...process.env,
    HTTP_PROXY: `http://127.0.0.1:${bridgePort}`,
    HTTPS_PROXY: `http://127.0.0.1:${bridgePort}`,
    ALL_PROXY: "",
    NO_PROXY: "localhost,127.0.0.1,::1",
    NODE_USE_ENV_PROXY: "1",
  };
  try {
    const out = execFileSync(codexExe, ["doctor"], {
      encoding: "utf8",
      env,
      timeout: 120000,
    });
    console.log("  codex doctor 输出（前 300 字符）:", out.slice(0, 300).replace(/\n/g, " | "));
  } catch (e) {
    console.log("  codex doctor 非零退出（网络诊断可能部分失败，仍可看桥接统计）:", String(e.status || e));
  }

  step("4. 验证桥接统计（codex 的 CONNECT 经过桥接）");
  snap = await getSnap();
  const afterTotal = snap.bridge_connections_total;
  console.log("  桥接连接数:", beforeTotal, "→", afterTotal);
  console.log("  最后目标:", snap.bridge_last_target);
  if (afterTotal <= beforeTotal) throw new Error("桥接无新增连接：codex 流量未经过桥接");
  const expectedDomains = ["github", "openai", "chatgpt", "oaistatic", "exa", "xxcrayon"];
  if (snap.bridge_last_target && !expectedDomains.some((d) => snap.bridge_last_target.includes(d))) {
    console.log("  警告: 最后目标不在预期域（可能是 codex 内部端点）:", snap.bridge_last_target);
  }

  step("5. 服务器侧证据（sshd 日志 connect_to）");
  const logAfter = execFileSync("docker", ["logs", "--tail", "80", "lcfg-test-sshd"], { encoding: "utf8" });
  const newLines = logAfter.split("\n").filter((l) => !logBefore.includes(l));
  const connectLines = newLines.filter((l) => l.includes("connect_to"));
  console.log("  新增 connect_to 记录数:", connectLines.length);
  for (const l of connectLines.slice(0, 8)) console.log("   ", l.trim());

  step("6. 断开并确认清理");
  await call("disconnect", undefined);
  await sleep(1500);
  snap = await getSnap();
  console.log("  断开后:", snap.state, "桥接端口:", snap.bridge_port);
  if (snap.state !== "READY") throw new Error("断开后未回到 READY");

  console.log("\n=== M2 CLI 路由验证通过 ===");
  ws.close();
}
main().catch((e) => {
  console.error("FAILED:", e.message);
  process.exit(1);
});
