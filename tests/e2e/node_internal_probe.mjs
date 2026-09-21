// 决定性探测：请求内部域名 lcfg-test-web（宿主机直连解析不了）
// 只有「经隧道 + 远端 DNS」才可能成功 → 能分辨代理实际路径
const t = process.argv[2] || "A";
async function run() {
  try {
    const r = await fetch("http://lcfg-test-web:8080/");
    const txt = await r.text();
    console.log(t + "_INTERNAL_OK:", txt.trim());
  } catch (e) {
    console.log(t + "_INTERNAL_FAIL:", e.cause?.code || e.message);
  }
}
run();
