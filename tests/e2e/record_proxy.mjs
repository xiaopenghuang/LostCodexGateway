// 记录型假 HTTP 代理：监听指定端口，记录所有 CONNECT/GET 请求头后返回 502。
// 用于判定客户端是否真的走了代理环境变量（A/B 对照实验）。
// 输出 JSON 行到 stdout：{"type":"connect","target":"..."} 等
import net from "net";

const port = Number(process.argv[2] || "17999");
const server = net.createServer((socket) => {
  let buf = Buffer.alloc(0);
  socket.on("data", (d) => {
    buf = Buffer.concat([buf, d]);
    const firstLine = buf.toString("latin1").split("\r\n")[0] || "";
    if (firstLine.startsWith("CONNECT")) {
      console.log(JSON.stringify({ type: "connect", target: firstLine.split(" ")[1] }));
      socket.end("HTTP/1.1 502 Blackhole\r\n\r\n");
    } else if (firstLine.startsWith("GET") || firstLine.startsWith("POST")) {
      console.log(JSON.stringify({ type: "plain", target: firstLine }));
      socket.end("HTTP/1.1 502 Blackhole\r\n\r\n");
    } else {
      socket.end();
    }
  });
  socket.on("error", () => {});
});
server.listen(port, "127.0.0.1", () => console.log(JSON.stringify({ type: "listening", port })));
