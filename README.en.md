<div align="center">

<img src="app-icon.png" alt="LostCodexGateway" width="128" height="128" />

# LostCodexGateway

**A verifiable network egress for your Codex clients on Windows — through a Linux server you own.**

One click to bring up an SSH SOCKS5 tunnel so selected client traffic leaves from your server. No API forwarding, no decryption, no rewriting.

[![Release](https://img.shields.io/badge/release-v0.3.0-2ea44f?style=flat-square)](../../releases)
[![Platform](https://img.shields.io/badge/platform-Windows%2010%2F11-0078d4?style=flat-square)](#requirements)
[![License](https://img.shields.io/badge/license-MIT-blue?style=flat-square)](LICENSE)
[![Tauri](https://img.shields.io/badge/Tauri-2.x-24c8db?style=flat-square)](https://tauri.app)
[![Vue](https://img.shields.io/badge/Vue-3.5-42b883?style=flat-square)](https://vuejs.org)

[简体中文](README.md) · [English](README.en.md)

</div>

---

## Table of Contents

- [The Problem This Solves](#the-problem-this-solves)
- [Key Features](#key-features)
- [The Honest Boundary of "Verified"](#the-honest-boundary-of-verified)
- [Architecture](#architecture)
- [Requirements](#requirements)
- [Quick Start](#quick-start)
- [Building from Source](#building-from-source)
- [Project Structure](#project-structure)
- [Security Design](#security-design)
- [Known Limitations](#known-limitations)
- [Documentation](#documentation)
- [Contributing](#contributing)
- [License](#license)

---

## The Problem This Solves

You run Codex CLI / Codex Desktop on Windows and want its **network egress** to go through a Linux server you own, rather than connecting directly. The usual approach is to manually start `ssh -D` and then somehow get Codex to use it. On Windows, that runs into a few concrete problems:

- **Codex CLI is a native Rust binary.** It reads `HTTP_PROXY` / `HTTPS_PROXY` but **does not accept `socks5://` directly**. A SOCKS5 port alone is not enough — you need an HTTP CONNECT → SOCKS5 bridge.
- **OAuth callbacks break if `localhost` is proxied.** If the local loopback isn't excluded, the ChatGPT OAuth callback listener never receives its redirect.
- **"Process is running" ≠ "traffic actually left."** You need verifiable evidence, not a green indicator light.
- **Process cleanup must be surgical.** `taskkill /IM ssh.exe` would kill your own unrelated SSH sessions.

LostCodexGateway packages this into a small tray-resident utility: enter your server details once, then connect with one click — and it tells you **explicitly which state your egress is in**.

> **Scope note**: This tool manages the **transport-layer egress only**. Changing your egress IP does not change your account's actual region or the applicability of any terms of service. This tool does not provide — and does not claim to provide — any ability to bypass account restrictions or regional policies.

---

## Key Features

| Feature | Description |
|---|---|
| **SSH SOCKS5 tunnel** | Uses the built-in Windows OpenSSH (`ssh.exe -D`). Arguments are passed as an array — no shell string interpolation, no injection surface |
| **Multi-server switching** | Save any number of servers and switch the active egress with one click. Switching is a hard cut (disconnect → reconnect); failures are not auto-rolled-back, but a "switch back to previous" button is provided. Local ports are global and fixed, so switching is transparent to the Codex CLI |
| **Latency probe** | Concurrent TCP probes across all servers, honestly labelled as "round-trip to the SSH port" — **not** egress latency |
| **Host key verification** | Queries the server fingerprint for manual comparison; **blocks the connection if the fingerprint changes**. Backs up `known_hosts` before writing |
| **Egress verification** | Port listening + remote DNS resolution through SOCKS + egress IP echo (multi-endpoint with fallback, timestamped). All three judged independently |
| **HTTP CONNECT → SOCKS5 bridge** | Binds `127.0.0.1` only. No TLS interception, no caching, no root certificate. Target blacklist blocks loopback/private ranges to prevent proxy loops |
| **Surgical process cleanup** | Terminates only the `ssh.exe` child processes it created. Your other SSH sessions are never touched |
| **Codex CLI launcher** | Proxy is injected into that child process's terminal environment only — **never into global environment variables**. `NO_PROXY=localhost,127.0.0.1,::1` keeps OAuth callbacks direct |
| **Network diagnostics panel** | Tunnel / SOCKS / egress IP comparison / read-only server reachability / Codex process routing / DNS / IPv6 / latency — all on one screen |
| **Error classification** | DNS failure, host unreachable, auth failure, fingerprint change, port in use, remote forwarding denied, disconnect — reported separately instead of collapsing into "connection failed" |
| **Mihomo / Clash Verge read-only integration** | **Detects** only; generates rule snippets, backup and rollback. Never modifies your Mihomo config (and Clash is not a hard dependency — see below) |
| **WSL2 probing** | Enumerates distributions + real per-target connectivity probes + generates one-shot proxy injection commands (affects that shell only; never writes `~/.bashrc` / `/etc/environment`) |
| **Tray residency** | Closing the window hides to tray — the tunnel keeps running. Only the tray menu's "Exit" actually quits, disconnecting cleanly first |
| **Autostart** | Per-user registry Run key, no elevation required. Residents in the tray after login, and **never auto-connects the tunnel** (no silent egress) |
| **Single-instance guard** | A second launch won't spawn a second tunnel or steal the port — it surfaces the existing window and exits |
| **Diagnostics export** | One-click export of a redacted diagnostic bundle |

---

## The Honest Boundary of "Verified"

"The tunnel is up" does not mean "traffic actually went through it." This tool splits routing state into **five** distinct statuses, each with an explicit criterion:

| Status | Meaning | Criterion |
|---|---|---|
| **Verified** | Traffic genuinely exits via the gateway | At least one bridge connection **reached the downstream SOCKS5 and returned 200**, and the gateway itself is reachable |
| **Partial** | Some clients used the gateway | Gateway is reachable, and gateway-matched connections coexist with others |
| **Anomaly** | Attempted the gateway but got nowhere | There were connection **attempts** or rejections, but not a single tunnel was actually established |
| **Unverified** | No evidence yet | Tunnel is not running, or the gateway is unreachable (when the gateway is down, "no matches" is just a side effect — not an anomaly) |
| **Unconfirmable** | Environment doesn't permit a verdict | e.g. required permissions or probing methods are unavailable |

> **Implementation detail that matters**: the bridge distinguishes **connection attempts** (counted as soon as TCP connects) from **successfully tunneled connections** (downstream SOCKS5 connected **and** `200` already returned). Only the latter counts as evidence for `Verified` — otherwise "the CLI tried to use the gateway and failed" would be misreported as success.

---

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│  Windows host                                                   │
│                                                                 │
│   ┌───────────────┐   HTTP_PROXY=http://127.0.0.1:<port>        │
│   │  Codex CLI    │ ─────────────────────────────┐              │
│   │ (native bin)  │                              │              │
│   └───────────────┘                              ▼              │
│   ┌───────────────┐                    ┌──────────────────────┐  │
│   │ Codex Desktop │                    │  Bridge (Rust)       │  │
│   │   / IDE       │ ──────────────────▶│  HTTP CONNECT        │  │
│   └───────────────┘   (optional, via   │  → SOCKS5            │  │
│                        Mihomo)         │  binds 127.0.0.1     │  │
│                                        └──────────┬───────────┘  │
│                                                   │ SOCKS5       │
│                                        ┌──────────▼───────────┐  │
│                                        │ 127.0.0.1:17801      │  │
│                                        │ (ssh -D listener)    │  │
│                                        └──────────┬───────────┘  │
└───────────────────────────────────────────────────┼─────────────┘
                                                    │ SSH (encrypted)
                                                    ▼
                                        ┌──────────────────────┐
                                        │  Your Linux server   │
                                        │  sshd → egress       │
                                        └──────────────────────┘
```

**Why the bridge in the middle?** Because Codex CLI's native binary only implements HTTP proxy CONNECT semantics and doesn't understand `socks5://`. The bridge fills that gap. It listens on loopback only, does not decrypt HTTPS, and installs no root certificate — it is a protocol translator, not a man-in-the-middle.

**On Clash / Mihomo: not a hard dependency.** The core path is `Codex → loopback bridge → ssh -D → your server`, with no Clash involved. The Mihomo integration serves exactly one scenario: if you also want clients that ignore `HTTP_PROXY` — such as **Codex Desktop / IDE** — to use the tunnel, you can route them to the gateway group via Clash rules. This tool only **detects read-only, generates rule snippets, and provides backup/rollback**. Whether to apply them is your call.

---

## Requirements

| Item | Requirement |
|---|---|
| OS | Windows 10 1809+ / Windows 11 |
| Runtime | [OpenSSH Client](https://learn.microsoft.com/windows-server/administration/openssh/openssh_install_firstuse) (Windows optional feature; usually preinstalled) |
| Server | Any Linux distribution running `sshd`, with TCP forwarding allowed (`AllowTcpForwarding yes`), and outbound internet access |
| Auth | Private key (Ed25519 / RSA). This tool **stores the key path only** — never reads or copies key contents |
| Optional | Mihomo / Clash Verge (only if you need to cover Desktop / IDE clients) |

---

## Quick Start

1. **Install**: download `LostCodexGateway_0.3.0_x64-setup.exe` and run it. Standard user privileges are sufficient — no administrator needed.
2. **Enter server details**: open the app → "Server" page → "Add server" → fill in a name, host, SSH port, username and private key path. You can save several servers and switch the active egress at any time (switching is a hard cut: the old tunnel is torn down before the new one is built, so in-flight requests are interrupted). The local SOCKS port (default `17801`) and bridge port (default `17800`) are **global settings**, edited on the "Settings" page.
3. **Verify the fingerprint**: click "Query server fingerprint" → **compare it with your server administrator** (or against a fingerprint you already know) → confirm with "I've verified it, write it".
4. **Connect**: on the "Home" page click "Connect" → status becomes **Verified**, and the page shows the egress IP along with each verification step.
5. **Launch Codex**: "Apps" page → "Launch Codex CLI from gateway" → a separate terminal window opens with the proxy injected into that terminal only.
6. **Disconnect**: click "Disconnect" — only the SSH process created by this tool is stopped. Your other SSH sessions, system proxy, and Mihomo configuration are untouched.
7. **Day to day**: the window's close button = **hide to tray** (tunnel keeps running). Left-click the tray icon to reopen; only the right-click "Exit" truly quits (disconnecting cleanly first).

---

## Building from Source

Prerequisites: Node.js 20+, Rust 1.77+ (stable), and MSVC build tools on Windows.

```bash
# 1. Frontend dependencies
npm install

# 2. Rust unit tests
cargo test                     # run inside src-tauri/

# 3. Development mode (hot reload)
npm run tauri dev

# 4. Release build (produces the NSIS installer)
npm run tauri build
# Output: src-tauri/target/release/bundle/nsis/LostCodexGateway_<version>_x64-setup.exe
```

> ⚠️ `cargo build --release` only compiles a binary — it **does not produce an installer**. You must use `npm run tauri build` for that.

Integration tests need Docker fixtures (when no usable local `sshd` is available):

```powershell
powershell -File tests/fixtures/ssh-server/setup.ps1
cargo test --test tunnel_e2e -- --ignored --test-threads=1
cargo test --test bridge_e2e -- --ignored
```

Test fixtures use self-built Docker containers only (cleanup via `-Teardown` / `-Clean`) and **never modify the host's ssh / proxy / routing configuration**.

---

## Project Structure

```
LostCodexGateway/
├── src/                     # Frontend (Vue 3 + TypeScript)
│   ├── pages/               # Home / Server / Apps / Network diag / WSL / Settings
│   ├── components/          # Shared components (design system)
│   ├── stores/              # State (read-only snapshots; Rust holds authority)
│   └── dev/fixtures.ts      # Mock data for visual checks without a backend
├── src-tauri/               # Rust backend
│   └── src/
│       ├── ssh.rs           # SSH tunnel lifecycle, host key verification
│       ├── bridge.rs        # HTTP CONNECT → SOCKS5 bridge
│       ├── diagnostics.rs   # Network diagnostics and routing verdicts
│       ├── mihomo.rs        # Clash / Mihomo read-only detection and rule generation
│       ├── wsl.rs           # WSL2 probing and one-shot injection commands
│       ├── autostart.rs     # Autostart (registry Run key)
│       └── single_instance.rs
├── docs/                    # Design docs, audits, acceptance records
├── scripts/                 # Dev helpers (privacy scan, path redaction)
└── tests/                   # Integration tests and E2E fixtures
```

**Design principle**: the Rust side is the single source of truth; the frontend only renders snapshots. Every fallible I/O operation is explicitly classified on the Rust side — the frontend never infers.

---

## Security Design

- **No credential access**: never reads, decrypts, or caches model API traffic; never takes over the Codex login flow or manages ChatGPT OAuth.
- **No system modification**: does not alter the system proxy, routing table, or Mihomo configuration (Mihomo handling is read-only + snippet generation).
- **Bounded process cleanup**: terminates only `ssh.exe` child processes it created itself.
- **Least privilege in the bridge**: loopback-only binding, no TLS interception, no root certificate, target blacklist against proxy loops, concurrency cap, bidirectional idle timeout.
- **Private keys by path only**: the configuration stores a path string; key contents never appear in app data, logs, or diagnostic exports.
- **Redacted diagnostics export**: IPs, hostnames, and usernames are substituted before export.

Details and audit records: [docs/security.md](docs/security.md), [docs/privacy-audit.md](docs/privacy-audit.md).

---

## Known Limitations

- **WSL2 in NAT mode**: under WSL2's default NAT networking, the Windows-side SOCKS binds loopback only, so WSL can't reach it. This tool **reports it as unreachable, honestly**, and does not perform the elevated port forwarding that would be required. In mirrored networking mode, `127.0.0.1` works directly.
- **Codex Desktop / IDE**: does not honor `HTTP_PROXY`, so it needs the Mihomo integration to be covered. Enabling TUN mode and its privileges are handled by Mihomo / Clash Verge's own components — this tool does not do that on your behalf.
- **Windows only**: both SSH process management and registry autostart are Windows-specific implementations.
- **Single-environment validation**: current acceptance records come from a limited set of environment combinations. See the [acceptance report](docs/acceptance-report.md).

For fuller technical debt and unimplemented items: [docs/risks-and-unimplemented.md](docs/risks-and-unimplemented.md).

---

## Documentation

| Document | Contents |
|---|---|
| [Architecture](docs/architecture.md) | Module breakdown, data flow, environment evidence |
| [Windows Setup Guide](docs/setup-windows.md) | Server- and client-side setup, incl. Mihomo rule import / rollback |
| [Troubleshooting](docs/troubleshooting.md) | Common error classifications and remedies |
| [Security & Privacy](docs/security.md) | Threat model and security boundaries |
| [Privacy Audit Record](docs/privacy-audit.md) | Sensitive-information scan and handling before public release |
| [Acceptance & Test Records](docs/acceptance-report.md) | Test environments, evidence, conclusions |
| [Risks & Unimplemented](docs/risks-and-unimplemented.md) | Known technical debt |
| [Environment Survey Report](docs/m0-environment-report.md) | Environment reconnaissance from the inception phase |
| [Changelog](CHANGELOG.md) | Version history |
| [Third-Party Notices](THIRD_PARTY.md) | Dependency list and licenses |

---

## Contributing

Issues and PRs are welcome. Before submitting, please make sure:

1. `cargo test` passes fully (in sandboxed environments a few `single_instance` cases may fail due to restricted `CreateMutexW`; those need verification on real hardware).
2. `cargo fmt` and `cargo clippy -- -D warnings` are clean.
3. When adding code that touches filesystem paths or networking, run `python scripts/privacy-scan.py` to confirm no host-specific environment information is introduced.

---

## License

[MIT](LICENSE) © 2026 LostCodexGateway

See [THIRD_PARTY.md](THIRD_PARTY.md) for the dependency list and licenses.

---

<div align="center">

**This project is not affiliated with OpenAI** and is neither sponsored, endorsed, nor authorized by OpenAI.  
"Codex", "ChatGPT", and "OpenAI" are trademarks of their respective owners and are used here only to describe what this tool interoperates with.

</div>
