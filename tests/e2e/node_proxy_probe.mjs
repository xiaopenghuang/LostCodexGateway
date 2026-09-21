// Node 代理行为实测：模拟 Codex CLI（Node/undici 应用）在三种代理配置下的行为
// 场景A: ALL_PROXY=socks5://（Node 原生是否支持 SOCKS）
// 场景B: HTTPS_PROXY=http:// 指向 SOCKS 端口（协议不匹配，预期失败）
// 场景C: HTTPS_PROXY=http:// 指向 HTTP 桥接（M2 实现后验证）
const t = process.argv[2] || "A";

async function run() {
  if (t === "A") {
    try {
      const r = await fetch("https://ifconfig.me/ip");
      const txt = await r.text();
      console.log("A_RESULT_OK:", txt.trim());
    } catch (e) {
      console.log("A_RESULT_FAIL:", e.cause?.code || e.message);
    }
  } else if (t === "B") {
    try {
      const r = await fetch("https://ifconfig.me/ip");
      const txt = await r.text();
      console.log("B_RESULT_OK:", txt.trim());
    } catch (e) {
      console.log("B_RESULT_FAIL:", e.cause?.code || e.message);
    }
  } else {
    try {
      const r = await fetch("https://ifconfig.me/ip");
      const txt = await r.text();
      console.log("C_RESULT_OK:", txt.trim());
    } catch (e) {
      console.log("C_RESULT_FAIL:", e.cause?.code || e.message);
    }
  }
}
run();
