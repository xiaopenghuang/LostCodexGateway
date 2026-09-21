# LostCodexGateway 集成测试 sshd 夹具
# 本机无 sshd（M0 实测），用 Docker 容器提供可复现的测试服务器。
# 仅用于本地测试；容器销毁后不留任何主机配置改动。
FROM alpine:3.20

RUN apk add --no-cache openssh-server openssh-keygen bash \
    && ssh-keygen -A \
    && adduser -D -s /bin/sh testuser \
    && passwd -u testuser \
    && mkdir -p /home/testuser/.ssh \
    && chmod 700 /home/testuser/.ssh \
    && chown testuser:testuser /home/testuser/.ssh

# 客户端公钥由构建参数注入（容器内不保存私钥）
ARG CLIENT_PUBKEY=""
RUN echo "${CLIENT_PUBKEY}" > /home/testuser/.ssh/authorized_keys \
    && chmod 600 /home/testuser/.ssh/authorized_keys \
    && chown testuser:testuser /home/testuser/.ssh/authorized_keys

# 密码登录关闭；允许 TCP 转发（SOCKS 需要）。
# 注意: Alpine 默认 sshd_config 自带 "AllowTcpForwarding no"（OpenSSH 取首次生效值，
# 追加无效），必须替换。保留一个允许禁用的场景可用来测「远端禁止转发」错误分类。
# 另: 限制 KEX 列表（去掉 sntrup761），因为 Windows 9.5 的 ssh-keyscan 对其处理有缺陷。
RUN sed -i 's/^AllowTcpForwarding no/AllowTcpForwarding yes/' /etc/ssh/sshd_config \
    && echo "KexAlgorithms curve25519-sha256,curve25519-sha256@libssh.org,ecdh-sha2-nistp256,diffie-hellman-group-exchange-sha256" >> /etc/ssh/sshd_config \
    && echo "PasswordAuthentication no" >> /etc/ssh/sshd_config \
    && echo "PubkeyAuthentication yes" >> /etc/ssh/sshd_config

EXPOSE 22

CMD ["/usr/sbin/sshd", "-D", "-e"]
