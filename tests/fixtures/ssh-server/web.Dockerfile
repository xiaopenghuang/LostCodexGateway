# LostCodexGateway 夹具内部验证服务镜像
# 持久 httpd（busybox-extras applet）：每连接稳定回显 LCFG-TUNNEL-OK
FROM alpine:3.20
RUN apk add --no-cache busybox-extras \
    && mkdir -p /www \
    && echo "LCFG-TUNNEL-OK" > /www/index.html
EXPOSE 8080
CMD ["httpd", "-f", "-p", "8080", "-h", "/www"]
