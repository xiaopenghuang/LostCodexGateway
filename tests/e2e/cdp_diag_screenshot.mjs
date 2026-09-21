// 截图脚本：真实窗口内打开网络诊断页，跑诊断后截图保存到 docs/
import http from "http";
import fs from "fs";

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

  // 连接隧道（若未连接）
  let snap = (await call("get_snapshot", undefined)).v;
  if (snap.state !== "EGRESS_VERIFIED") {
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
      await call("connect", undefined);
    }
    for (let i = 0; i < 60; i++) {
      snap = (await call("get_snapshot", undefined)).v;
      if (snap.state === "EGRESS_VERIFIED") break;
      await sleep(1000);
    }
  }

  // 跑一次诊断让页面有数据
  await call("run_diagnostics", { mihomoSecret: null });

  // 页面点击「网络诊断」tab（第 4 个 tab）
  await send("Runtime.evaluate", {
    expression: `[...document.querySelectorAll('.tab')].find(b => b.textContent.includes('网络诊断'))?.click()`,
  });
  await sleep(1200);

  // 截图（viewport 截图）
  const shot = await send("Page.captureScreenshot", { format: "png" });
  const buf = Buffer.from(shot.result.data, "base64");
  const out = "I:\\开发\\LostCodexGateway\\docs\\screenshots";
  fs.mkdirSync(out, { recursive: true });
  fs.writeFileSync(out + "\\network-diagnostics.png", buf);
  console.log("截图已保存: docs/screenshots/network-diagnostics.png", buf.length, "bytes");

  // 输出页面文本快照（用于记录）
  const txt = await send("Runtime.evaluate", {
    expression: "document.body.innerText.slice(0, 1500)",
    returnByValue: true,
  });
  console.log("=== 页面文本（前1500字符）===");
  console.log(txt.result.result.value);
  ws.close();
}
main().catch((e) => {
  console.error("FAILED:", e.message);
  process.exit(1);
});
