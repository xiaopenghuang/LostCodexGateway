# LostCodexGateway M0 环境调研脚本（只读检测，不修改系统或服务器配置）
# 输出 docs/m0-environment-report.json + 控制台摘要
$ErrorActionPreference = 'SilentlyContinue'
$report = [ordered]@{}
$report.generated_at = (Get-Date -Format 'yyyy-MM-dd HH:mm:ss zzz')
$report.generated_by = 'scripts/m0-detect.ps1'

# ---- OS ----
$os = Get-CimInstance Win32_OperatingSystem
$report.os = [ordered]@{
    caption   = $os.Caption
    version   = $os.Version
    build     = $os.BuildNumber
    arch      = $env:PROCESSOR_ARCHITECTURE
}

# ---- Windows OpenSSH ----
$sshExe = (Get-Command ssh.exe -ErrorAction SilentlyContinue).Source
$report.ssh = [ordered]@{
    path = $sshExe
    version = if ($sshExe) { (& $sshExe -V 2>&1 | Out-String).Trim() } else { $null }
}
$sshdSvc = Get-Service sshd -ErrorAction SilentlyContinue
$report.sshd_service = [ordered]@{
    exists = [bool]$sshdSvc
    status = if ($sshdSvc) { $sshdSvc.Status.ToString() } else { $null }
    start_type = if ($sshdSvc) { $sshdSvc.StartType.ToString() } else { $null }
}
$adminSshdConfig = Test-Path 'C:\ProgramData\ssh\sshd_config'
$report.sshd_config_present = $adminSshdConfig

# ---- 工具链 ----
$report.toolchain = [ordered]@{
    rustc = (Get-Command rustc.exe -ErrorAction SilentlyContinue).Source
    rustc_version = (& rustc --version 2>&1 | Out-String).Trim()
    cargo = (Get-Command cargo.exe -ErrorAction SilentlyContinue).Source
    node = (Get-Command node.exe -ErrorAction SilentlyContinue).Source
    node_version = (& node --version 2>&1 | Out-String).Trim()
    npm = (Get-Command npm.cmd -ErrorAction SilentlyContinue).Source
    pnpm = (Get-Command pnpm.cmd -ErrorAction SilentlyContinue).Source
    git = (Get-Command git.exe -ErrorAction SilentlyContinue).Source
    curl = (Get-Command curl.exe -ErrorAction SilentlyContinue).Source
    curl_version = (& curl.exe --version 2>&1 | Select-Object -First 1 | Out-String).Trim()
    powershell = (Get-Command powershell.exe -ErrorAction SilentlyContinue).Source
    nsis = (Get-Command makensis.exe -ErrorAction SilentlyContinue).Source
}
# WebView2 runtime
$wv2 = Get-ItemProperty 'HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}' -ErrorAction SilentlyContinue
$report.webview2 = [ordered]@{ installed = [bool]$wv2; version = $wv2.pv }

# ---- 网络代理现状（只读）----
$report.system_proxy = [ordered]@{
    proxy_enable = (Get-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Internet Settings' -ErrorAction SilentlyContinue).ProxyEnable
    proxy_server = (Get-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Internet Settings' -ErrorAction SilentlyContinue).ProxyServer
}

# ---- Clash Verge Rev / Mihomo ----
$report.clash = [ordered]@{}
$vergeProfiles = @(
    "$env:LOCALAPPDATA\clash-verge-rev", "$env:APPDATA\clash-verge-rev",
    "$env:LOCALAPPDATA\Programs\clash-verge-rev", "$env:APPDATA\io.github.clash-verge-rev.clash-verge-rev"
)
$report.clash.profiles_found = @()
foreach ($p in $vergeProfiles) { if (Test-Path $p) { $report.clash.profiles_found += $p } }
$vergeExe = Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\*' -ErrorAction SilentlyContinue |
    Where-Object { $_.DisplayName -match 'clash|verge|mihomo' } | Select-Object DisplayName, DisplayVersion, InstallLocation
$report.clash.uninstall_entries = @($vergeExe)
$mihomoCore = Get-ChildItem "$env:LOCALAPPDATA\clash-verge-rev" -Recurse -Filter 'mihomo*.exe' -ErrorAction SilentlyContinue | Select-Object -First 3 -ExpandProperty FullName
$report.clash.mihomo_cores = @($mihomoCore)
$report.clash.processes = @(Get-CimInstance Win32_Process -Filter "Name like '%mihomo%' or Name like '%clash%' or Name like '%verge%'" |
    Select-Object ProcessId, Name, ExecutablePath)
# 默认配置目录（只列存在性，不读取内容）
$report.clash.config_dirs = [ordered]@{
    verge_data = Test-Path "$env:APPDATA\io.github.clash-verge-rev.clash-verge-rev"
    mihomo_default = Test-Path "$env:USERPROFILE\.config\mihomo"
}

# ---- Codex 客户端 ----
$report.codex = [ordered]@{}
$codexCli = (Get-Command codex.exe -ErrorAction SilentlyContinue).Source
$report.codex.cli_path = $codexCli
$report.codex.cli_version = (& codex --version 2>&1 | Out-String).Trim()
$report.codex.cli_processes = @(Get-CimInstance Win32_Process -Filter "Name like '%codex%'" | Select-Object ProcessId, Name, ExecutablePath)
$desktopDirs = @("$env:LOCALAPPDATA\Programs\codex", "$env:APPDATA\codex", "$env:LOCALAPPDATA\codex")
$report.codex.desktop_dirs_found = @($desktopDirs | Where-Object { Test-Path $_ })
$report.codex.desktop_uninstall = @(Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\*','HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*' -ErrorAction SilentlyContinue |
    Where-Object { $_.DisplayName -match 'codex' } | Select-Object DisplayName, DisplayVersion, InstallLocation, UninstallString)
$codeCmd = (Get-Command code.cmd -ErrorAction SilentlyContinue).Source
$report.codex.vscode = [ordered]@{
    found = [bool]$codeCmd
    path  = $codeCmd
}
$wslOut = (& wsl.exe --status 2>&1 | Out-String).Trim()
$report.codex.wsl = [ordered]@{ enabled = [bool]$wslOut }
# 仅记录目录存在性，不读取认证内容
$report.codex.oauth_dirs = [ordered]@{
    chatgpt_present = Test-Path "$env:APPDATA\codex"
}

# ---- 网络连通性（只做公网连通性探测，不发起业务请求）----
$report.connectivity = [ordered]@{
    direct_egress_ip = $null
    ip_service = 'https://api.ipify.org?format=json'
}
try {
    $r = Invoke-WebRequest -Uri 'https://api.ipify.org?format=json' -TimeoutSec 10 -UseBasicParsing
    $report.connectivity.direct_egress_ip = ($r.Content | ConvertFrom-Json).ip
} catch { $report.connectivity.direct_egress_ip = "FAILED: $($_.Exception.Message)" }

# ---- 当前 SSH 会话（只统计本机，不动它们）----
$report.ssh_processes = @(Get-CimInstance Win32_Process -Filter "Name = 'ssh.exe'" | Select-Object ProcessId, CommandLine)

$outDir = Join-Path $PSScriptRoot '..\docs'
New-Item -ItemType Directory -Force -Path $outDir | Out-Null
$out = Join-Path $outDir 'm0-environment-report.json'
$report | ConvertTo-Json -Depth 6 | Set-Content -Path $out -Encoding UTF8

Write-Host "=== M0 检测摘要 ==="
Write-Host ("OS: {0} ({1})" -f $report.os.caption, $report.os.version)
Write-Host ("ssh.exe: {0} | {1}" -f $report.ssh.path, $report.ssh.version)
Write-Host ("sshd 服务: 存在={0} 状态={1}" -f $report.sshd_service.exists, $report.sshd_service.status)
Write-Host ("rustc: {0}" -f $report.toolchain.rustc_version)
Write-Host ("node: {0} | npm: {1}" -f $report.toolchain.node_version, $report.toolchain.npm)
Write-Host ("WebView2: {0} (v{1})" -f $report.webview2.installed, $report.webview2.version)
Write-Host ("系统代理: ProxyEnable={0} ProxyServer={1}" -f $report.system_proxy.proxy_enable, $report.system_proxy.proxy_server)
Write-Host ("Clash Verge 目录: {0}" -f ($report.clash.profiles_found -join ', '))
Write-Host ("Mihomo 核心: {0}" -f ($report.clash.mihomo_cores -join ', '))
Write-Host ("Clash/Verge 进程: {0}" -f ($report.clash.processes | ForEach-Object { "$($_.Name)(PID $($_.ProcessId))" }) -join ', ')
Write-Host ("Codex CLI: {0} | {1}" -f $report.codex.cli_path, $report.codex.cli_version)
Write-Host ("Codex Desktop 目录: {0}" -f ($report.codex.desktop_dirs_found -join ', '))
Write-Host ("VS Code: {0}" -f $report.codex.vscode.found)
Write-Host ("本机直连出口 IP: {0}" -f $report.connectivity.direct_egress_ip)
Write-Host ("现有 ssh.exe 进程数: {0}" -f @($report.ssh_processes).Count)
Write-Host ("报告已写入: $out")
