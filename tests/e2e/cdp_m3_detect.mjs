// M3 验证：detect_mihomo 命令返回真实检测结果
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

const r = await send("Runtime.evaluate", {
  expression: `window.__TAURI_INTERNALS__.invoke("detect_mihomo").then(v => ({ok: true, v})).catch(e => ({ok: false, e: String(e)}))`,
  awaitPromise: true,
  returnByValue: true,
});
console.log(JSON.stringify(r.result.result.value, null, 2));
ws.close();
