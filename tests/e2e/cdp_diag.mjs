// 快速诊断：查询当前 Tauri 实例状态
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
  expression: `window.__TAURI_INTERNALS__.invoke("get_snapshot").then(v => ({ok:true, v})).catch(e => ({ok:false, e:String(e)}))`,
  awaitPromise: true,
  returnByValue: true,
});
const snap = r.result.result.value;
console.log("state:", snap.v?.state);
console.log("ssh_pid:", snap.v?.ssh_pid);
console.log("last_error:", snap.v?.last_error);
console.log("host:", snap.v?.config?.server?.host, snap.v?.config?.server?.port);
ws.close();
