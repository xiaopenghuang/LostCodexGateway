#!/bin/sh
# LostCodexGateway 夹具内部验证服务：每个连接回显 LCFG-TUNNEL-OK
# -w 30：监听窗口 30 秒，避免频繁退出造成连接空隙
while true; do
  { echo -e "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 14\r\nConnection: close\r\n\r\nLCFG-TUNNEL-OK"; } | nc -l -p 8080 -w 30
  sleep 0.1
done
