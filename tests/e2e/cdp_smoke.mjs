// CDP 冒烟测试：连接真实 Tauri 窗口的 WebView2 调试端口，读取页面状态
import http from "http";

function getJson(url) {
  return new Promise((resolve, reject) => {
    http.get(url, (res) => {
      let data = "";
      res.on("data", (c) => (data += c));
      res.on("end", () => resolve(JSON.parse(data)));
    }).on("error", reject);
  });
}

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

  // 读取页面全部文本
  const r1 = await send("Runtime.evaluate", {
    expression: "document.body.innerText",
    returnByValue: true,
  });
  console.log("=== Tauri 窗口页面文本 ===");
  console.log(r1.result.result.value);
  console.log("");

  // 验证 Tauri 后端存在：invoke detect_ssh_env
  const r2 = await send("Runtime.evaluate", {
    expression:
      "window.__TAURI_INTERNALS__ ? 'tauri internals present' : 'NO tauri internals'",
    returnByValue: true,
  });
  console.log("=== 后端通道 ===");
  console.log(r2.result.result.value);

  // 通过全局 Vue store 检查 snapshot 是否已从后端加载（initStore 已被调用）
  const r3 = await send("Runtime.evaluate", {
    expression: "document.querySelector('.brand') ? document.querySelector('.brand').innerText : 'no brand'",
    returnByValue: true,
  });
  console.log("=== 品牌栏 ===");
  console.log(r3.result.result.value);
  ws.close();
}
main().catch((e) => {
  console.error("FAILED:", e.message);
  process.exit(1);
});
