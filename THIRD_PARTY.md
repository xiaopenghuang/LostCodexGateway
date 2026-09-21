# Third-party 依赖清单（主要运行时依赖）

## Rust（Cargo.toml）

| crate | 用途 | 许可证 |
|---|---|---|
| tauri 2 | GUI 框架 | MIT/Apache-2.0 |
| tokio | 异步运行时 | MIT |
| reqwest (rustls-tls, socks) | 出口验证 HTTP 客户端 | MIT/Apache-2.0 |
| serde / serde_json | 配置序列化 | MIT/Apache-2.0 |
| thiserror | 错误类型 | MIT/Apache-2.0 |
| parking_lot | 锁 | MIT/Apache-2.0 |
| chrono | 时间戳 | MIT/Apache-2.0 |
| sha2 | 指纹计算 | MIT/Apache-2.0 |
| tempfile | 原子写 | MIT/Apache-2.0 |

## 前端（package.json）

| 包 | 用途 | 许可证 |
|---|---|---|
| vue 3 | UI 框架 | MIT |
| @tauri-apps/api | Tauri IPC | MIT/Apache-2.0 |
| vite / @vitejs/plugin-vue / typescript / vue-tsc | 构建 | MIT |

## 系统组件（不随包分发）

- Windows OpenSSH（ssh.exe / ssh-keygen.exe / ssh-keyscan.exe）：系统自带
- WebView2 Runtime：Windows 10/11 自带（实测 153.0.4234.32）
- 测试夹具：Docker + alpine（仅开发/测试环境）

完整 license 文本见各依赖仓库；`cargo license` / `npm license-checker` 可生成完整清单。
