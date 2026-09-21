// 交互诊断：逐步探测 connect 拒绝原因
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
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

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

console.log("A. snapshot:", JSON.stringify((await call("get_snapshot")).v?.state));
console.log("B. connect:", JSON.stringify(await call("connect")));
await sleep(500);
console.log("C. snapshot after connect attempt:", JSON.stringify((await call("get_snapshot")).v?.state));
console.log("D. disconnect:", JSON.stringify(await call("disconnect")));
await sleep(1500);
console.log("E. snapshot after disconnect:", JSON.stringify((await call("get_snapshot")).v?.state));
console.log("F. connect again:", JSON.stringify(await call("connect")));
await sleep(2000);
console.log("G. snapshot:", JSON.stringify((await call("get_snapshot")).v?.state));
ws.close();
