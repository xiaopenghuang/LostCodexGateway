# -*- coding: utf-8 -*-
"""原始 SOCKS5 客户端诊断：手动握手 + CONNECT（域名形式），打印 sshd 响应。"""
import socket
import struct
import sys

def socks_connect(proxy_host, proxy_port, target_host, target_port, use_domain=True):
    s = socket.create_connection((proxy_host, proxy_port), timeout=15)
    # 无认证握手
    s.sendall(b"\x05\x01\x00")
    resp = s.recv(2)
    print("handshake resp:", resp.hex())
    if resp != b"\x05\x00":
        return None
    # CONNECT
    if use_domain:
        hb = target_host.encode("ascii")
        req = b"\x05\x01\x00\x03" + bytes([len(hb)]) + hb + struct.pack(">H", target_port)
    else:
        ip = socket.inet_aton(target_host)
        req = b"\x05\x01\x00\x01" + ip + struct.pack(">H", target_port)
    s.sendall(req)
    head = s.recv(4)
    print("reply head:", head.hex())
    if len(head) < 4 or head[0] != 0x05:
        return None
    atyp = head[3]
    if atyp == 0x01:
        rest = s.recv(4 + 2)
    elif atyp == 0x03:
        ln = s.recv(1)
        rest = ln + s.recv(ln[0] + 2)
    elif atyp == 0x04:
        rest = s.recv(16 + 2)
    else:
        rest = b""
    print("reply rest:", rest.hex())
    print("REP code:", head[1], "(0=success, 5=denied, 4=host unreachable)")
    return s if head[1] == 0 else None

if __name__ == "__main__":
    mode = sys.argv[1] if len(sys.argv) > 1 else "domain"
    s = socks_connect("127.0.0.1", 17801, "lcfg-test-web" if mode == "domain" else "172.18.0.3", 8080,
                      use_domain=(mode == "domain"))
    if s:
        try:
            s.sendall(b"GET / HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
            data = s.recv(200)
            print("HTTP response head:", data[:120])
        finally:
            s.close()
