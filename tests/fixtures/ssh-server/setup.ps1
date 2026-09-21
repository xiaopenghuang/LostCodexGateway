# LostCodexGateway 测试夹具管理脚本（Windows PowerShell 5.1，UTF-8 BOM）
# 用法:
#   setup.ps1            - 生成测试密钥对 + 启动 web/sshd 容器（sshd 映射 127.0.0.1:2222）
#   setup.ps1 -Teardown  - 停止并删除容器（保留密钥对文件）
#   setup.ps1 -Clean     - 停止删除容器 + 删除密钥对
# 夹具网络 lcfg-test-net 为 Docker 内部网络：宿主机直连不通，
# 只有经 SSH 隧道（服务器端建连）才能访问 lcfg-test-web:8080 —— 用于决定性隧道验证。
$ErrorActionPreference = 'Continue'
$fixture = $PSScriptRoot
$keys = Join-Path $fixture 'keys'
$containerName = 'lcfg-test-sshd'
$webName = 'lcfg-test-web'
$netName = 'lcfg-test-net'
$hostPort = 2222

if ($args -contains '-Teardown') {
    cmd /c "docker rm -f $containerName 2>nul" | Out-Null
    cmd /c "docker rm -f $webName 2>nul" | Out-Null
    cmd /c "docker network rm $netName 2>nul" | Out-Null
    Write-Host '容器已删除（密钥保留在 tests/fixtures/ssh-server/keys）'
    exit 0
}
if ($args -contains '-Clean') {
    cmd /c "docker rm -f $containerName 2>nul" | Out-Null
    cmd /c "docker rm -f $webName 2>nul" | Out-Null
    cmd /c "docker network rm $netName 2>nul" | Out-Null
    Remove-Item -Recurse -Force $keys -ErrorAction SilentlyContinue
    Write-Host '容器与测试密钥已全部删除'
    exit 0
}

New-Item -ItemType Directory -Force -Path $keys | Out-Null
$keyPath = Join-Path $keys 'id_test_ed25519'
$pubPath = "$keyPath.pub"

if (-not (Test-Path $keyPath)) {
    $sshKeygen = 'C:\Windows\System32\OpenSSH\ssh-keygen.exe'
    & $sshKeygen -t ed25519 -N '""' -C 'lcfg-test@localhost' -f $keyPath | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'ssh-keygen 失败' }
    Write-Host "已生成测试密钥对: $keyPath"
} else {
    Write-Host "复用已有测试密钥: $keyPath"
}

$pub = (Get-Content $pubPath -Raw).Trim()
Write-Host '构建 sshd 夹具镜像（--no-cache 确保公钥注入）...'
docker build --no-cache --build-arg "CLIENT_PUBKEY=$pub" -t lcfg-ssh-server:test $fixture
if ($LASTEXITCODE -ne 0) { throw 'docker build 失败' }

# 测试网络（sshd 与验证服务同网络）
docker network create $netName 2>&1 | Out-Null

# 1) 先启动验证服务容器（持久 httpd 回显 LCFG-TUNNEL-OK，仅 Docker 内部可达）
docker build -q -f "$fixture\web.Dockerfile" -t lcfg-test-web:latest "$fixture" | Out-Null
if ($LASTEXITCODE -ne 0) { throw 'web 镜像构建失败' }
cmd /c "docker rm -f $webName 2>nul" | Out-Null
docker run -d --name $webName --network $netName lcfg-test-web:latest
if ($LASTEXITCODE -ne 0) { throw 'web 容器启动失败' }
Start-Sleep -Seconds 2

# 2) 再启动 sshd 容器（映射 127.0.0.1:2222；--add-host 静态解析 web 名，
#    规避 Docker 嵌入式 DNS 对短生命周期容器的不稳定）
cmd /c "docker rm -f $containerName 2>nul" | Out-Null
$webIp = (docker inspect -f "{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}" $webName).Trim()
if (-not $webIp) { $webIp = '172.18.0.3' }
docker run -d --name $containerName --network $netName --add-host "${webName}:${webIp}" -p "${hostPort}:22" lcfg-ssh-server:test
if ($LASTEXITCODE -ne 0) { throw 'sshd 容器启动失败' }

Start-Sleep -Seconds 3
$ready = docker exec $containerName sh -c 'pgrep sshd >/dev/null; echo ok'
if ($ready -notmatch 'ok') { throw '容器内 sshd 未就绪' }
$webReady = docker exec $containerName wget -q -O- --timeout=5 http://${webName}:8080/ 2>$null
if ($webReady -notmatch 'LCFG-TUNNEL-OK') { Write-Host '警告: 验证服务未就绪（稍后可用 socks_probe 复查）' }

Write-Host ''
Write-Host '=== 夹具就绪 ==='
Write-Host ("sshd 容器: {0}  主机端口: 127.0.0.1:{1}" -f $containerName, $hostPort)
Write-Host ("测试账号: testuser  私钥: {0}" -f $keyPath)
Write-Host ("验证服务: {0}（仅 {1} 内部可达）" -f $webName, $netName)
Write-Host "注意: 容器重建会更换 host key；known_hosts 中 [127.0.0.1]:2222 条目需重新核对"
