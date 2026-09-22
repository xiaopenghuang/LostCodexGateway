//! 配置模块：加载/原子保存（写前备份，tempfile::persist 原子替换）。
//! 私钥只保存路径字符串；本模块绝不读取私钥内容。
//!
//! ## 多服务器（v0.3.0）
//!
//! 配置从「单台服务器」改为「服务器列表 + 当前选中」。语义是**切换**而非同时连接：
//! 任一时刻只有一条隧道在跑，切换时换掉背后的 ssh 进程。
//!
//! 为了让切换对下游透明（Codex CLI 的 `HTTP_PROXY` 不用改），两个本地端口
//! —— `socks_port` 与 `bridge_port` —— 是**全局设置**，不属于任何一台服务器。
//! 这一点是刻意用类型结构强制的：端口若放在 per-server 上，用户改了一台就会
//! 破坏「切换后地址不变」的前提。
//!
//! 老配置（v0.2.0 及以前）是扁平 `server` 对象，由 `parse_json` 自动迁移。

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// 默认本地 SOCKS5 入口端口（全局，切换时不变）。
pub const DEFAULT_SOCKS_PORT: u16 = 17801;
/// 默认 HTTP CONNECT 桥接入口端口（全局，切换时不变）。
pub const DEFAULT_BRIDGE_PORT: u16 = 17800;

/// 一台服务器的连接参数。`id` 一旦生成就不再变（改名/改主机都不会改 id），
/// 否则 `active_server_id` 会失联。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ServerProfile {
    /// 稳定标识（内部用，不由用户编辑）。
    pub id: String,
    /// 显示名（可空；空则回退到 host）。
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub key_path: String,
    pub ssh_exe_path: String,
    /// 该服务器**预期**的出口 IP（可选；空 = 不校验）。每台独立。
    pub expected_egress_ip: String,
    /// Mihomo 规则里这台服务器对应的代理组名（诊断关联用）。
    pub gateway_group: String,
}

impl Default for ServerProfile {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            host: String::new(),
            port: 22,
            username: String::new(),
            key_path: String::new(),
            ssh_exe_path: String::new(),
            expected_egress_ip: String::new(),
            gateway_group: "MY-VPS".to_string(),
        }
    }
}

impl ServerProfile {
    /// 展示名：优先 `name`，其次 `host`，都没有则「未命名」。
    pub fn display_name(&self) -> String {
        if !self.name.trim().is_empty() {
            return self.name.trim().to_string();
        }
        if !self.host.trim().is_empty() {
            return self.host.trim().to_string();
        }
        "未命名".to_string()
    }

    /// 是否具备连接所需的最少信息。
    pub fn is_complete(&self) -> bool {
        !self.host.trim().is_empty() && !self.username.trim().is_empty()
    }

    /// 目标串 `user@host`（用于日志/诊断展示）。
    pub fn target(&self) -> String {
        format!("{}@{}", self.username.trim(), self.host.trim())
    }
}

/// 生成一个新的服务器 id。
///
/// 不引第三方 uuid crate（项目取向是依赖尽量少）：用毫秒时间戳 + 递增后缀，
/// 与既有 id 去重。时间戳保证「删掉再新建」不会复用旧 id，从而避免
/// `active_server_id` 意外指向一台新服务器。
pub fn generate_server_id(existing: &[ServerProfile]) -> String {
    let base = chrono::Local::now().timestamp_millis().max(0) as u64;
    for i in 0..10_000u64 {
        let id = format!("srv-{:x}", base.wrapping_add(i));
        if !existing.iter().any(|s| s.id == id) {
            return id;
        }
    }
    format!("srv-{}", base)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct VerifyConfig {
    pub endpoints: Vec<String>,
    pub timeout_secs: u64,
}

impl Default for VerifyConfig {
    fn default() -> Self {
        Self {
            endpoints: vec![
                "https://api.ipify.org?format=json".to_string(),
                "https://ipinfo.io/ip".to_string(),
            ],
            timeout_secs: 15,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub auto_reconnect: bool,
    pub max_reconnect_attempts: u32,
    pub disconnect_policy: String,
    /// 本地 SOCKS5 入口端口（**全局**，切换服务器时保持不变）。
    pub socks_port: u16,
    /// HTTP CONNECT 桥接入口端口（**全局**，切换服务器时保持不变）。
    /// Codex CLI 的 `HTTP_PROXY` 指向它，所以必须固定。
    pub bridge_port: u16,
    /// 新建服务器时的默认代理组名（每台可各自覆盖）。
    pub gateway_group: String,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            auto_reconnect: false,
            max_reconnect_attempts: 3,
            disconnect_policy: "warn_and_block".to_string(),
            socks_port: DEFAULT_SOCKS_PORT,
            bridge_port: DEFAULT_BRIDGE_PORT,
            gateway_group: "MY-VPS".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GatewayConfig {
    /// 已保存的服务器列表（至少一台；空列表会在加载时补一台空配置）。
    pub servers: Vec<ServerProfile>,
    /// 当前选中的服务器 id。
    pub active_server_id: String,
    pub verify: VerifyConfig,
    pub settings: AppSettings,
}

impl Default for GatewayConfig {
    fn default() -> Self {
        // 默认给一台空配置，让「服务器」页一打开就有表单可填（与单服务器时代一致）。
        let p = ServerProfile {
            id: generate_server_id(&[]),
            ..Default::default()
        };
        Self {
            active_server_id: p.id.clone(),
            servers: vec![p],
            verify: VerifyConfig::default(),
            settings: AppSettings::default(),
        }
    }
}

impl GatewayConfig {
    /// 当前选中的服务器。
    pub fn active_server(&self) -> Option<&ServerProfile> {
        self.servers.iter().find(|s| s.id == self.active_server_id)
    }

    pub fn active_server_mut(&mut self) -> Option<&mut ServerProfile> {
        let id = self.active_server_id.clone();
        self.servers.iter_mut().find(|s| s.id == id)
    }

    pub fn server_by_id(&self, id: &str) -> Option<&ServerProfile> {
        self.servers.iter().find(|s| s.id == id)
    }

    /// 当前选中服务器是否已具备连接条件。
    pub fn is_configured(&self) -> bool {
        self.active_server().map(|s| s.is_complete()).unwrap_or(false)
    }

    /// 本地 SOCKS 端口（全局）。
    pub fn socks_port(&self) -> u16 {
        self.settings.socks_port
    }

    /// 本地桥接端口（全局）。
    pub fn bridge_port(&self) -> u16 {
        self.settings.bridge_port
    }

    /// 新增一台服务器，返回其 id。`id` 为空时自动生成。
    pub fn add_server(&mut self, mut profile: ServerProfile) -> String {
        if profile.id.trim().is_empty() {
            profile.id = generate_server_id(&self.servers);
        }
        let id = profile.id.clone();
        self.servers.push(profile);
        id
    }

    /// 更新一台已有服务器（按 id 匹配）。找不到则返回 Err。
    pub fn update_server(&mut self, profile: ServerProfile) -> Result<(), String> {
        let slot = self
            .servers
            .iter_mut()
            .find(|s| s.id == profile.id)
            .ok_or_else(|| format!("找不到服务器 id: {}", profile.id))?;
        // id 不可被外部改写
        let keep_id = slot.id.clone();
        *slot = profile;
        slot.id = keep_id;
        Ok(())
    }

    /// 删除一台服务器。返回删除后应当选中的 id（可能为空串）。
    ///
    /// 删除当前选中项时**不会**自动跳到别的服务器——切换由用户显式触发，
    /// 这里只是把选中项调整为一个合法值，避免 `active_server_id` 悬空。
    pub fn remove_server(&mut self, id: &str) -> bool {
        let before = self.servers.len();
        self.servers.retain(|s| s.id != id);
        if self.servers.len() == before {
            return false;
        }
        if self.active_server_id == id {
            self.active_server_id = self
                .servers
                .first()
                .map(|s| s.id.clone())
                .unwrap_or_default();
        }
        true
    }

    /// 解析配置 JSON，自动迁移 v0.2.0 及以前的扁平 `server` 结构。
    ///
    /// 用 `serde_json::Value` 而不是 `#[serde(untagged)]`：迁移里要做「字段搬家」
    /// （旧的 `verify.expected_egress_ip` → 服务器的 `expected_egress_ip`；
    /// 旧的 `server.socks_port` → 全局 `settings.socks_port`），
    /// 直接操作 Value 比堆一堆中间结构体清楚，也更好写测试。
    pub fn parse_json(raw: &str) -> Result<Self, serde_json::Error> {
        let v: Value = serde_json::from_str(raw)?;
        Ok(Self::from_value_migrated(v))
    }

    fn from_value_migrated(v: Value) -> Self {
        // ---- 新格式：有 servers 数组 ----
        if let Some(arr) = v.get("servers").and_then(|x| x.as_array()) {
            let mut cfg = Self {
                servers: arr
                    .iter()
                    .filter_map(|x| serde_json::from_value::<ServerProfile>(x.clone()).ok())
                    .collect(),
                ..Default::default()
            };
            // 兜底：id 缺失的补一个，避免列表里出现无法选中的项
            for i in 0..cfg.servers.len() {
                if cfg.servers[i].id.trim().is_empty() {
                    let others: Vec<ServerProfile> = cfg
                        .servers
                        .iter()
                        .enumerate()
                        .filter(|(j, _)| *j != i)
                        .map(|(_, s)| s.clone())
                        .collect();
                    cfg.servers[i].id = generate_server_id(&others);
                }
            }
            cfg.active_server_id = v
                .get("active_server_id")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            if let Some(ver) = v.get("verify") {
                if let Ok(x) = serde_json::from_value::<VerifyConfig>(ver.clone()) {
                    cfg.verify = x;
                }
            }
            if let Some(st) = v.get("settings") {
                if let Ok(x) = serde_json::from_value::<AppSettings>(st.clone()) {
                    cfg.settings = x;
                }
            }
            cfg.normalize();
            return cfg;
        }

        // ---- 旧格式：扁平 server 对象 → 迁移为单台服务器 ----
        if let Some(old) = v.get("server") {
            let mut cfg = Self {
                servers: Vec::new(),
                ..Default::default()
            };
            let host = str_of(old, "host");
            let legacy_name = str_of(old, "server_name");
            let profile = ServerProfile {
                id: String::new(),
                name: if legacy_name.trim().is_empty() {
                    host.clone()
                } else {
                    legacy_name
                },
                host,
                port: old
                    .get("port")
                    .and_then(|x| x.as_u64())
                    .map(|n| n as u16)
                    .unwrap_or(22),
                username: str_of(old, "username"),
                key_path: str_of(old, "key_path"),
                ssh_exe_path: str_of(old, "ssh_exe_path"),
                // 旧版把预期出口 IP 放在 verify 下，属于全局；迁移到这台服务器
                expected_egress_ip: v
                    .get("verify")
                    .map(|x| str_of(x, "expected_egress_ip"))
                    .unwrap_or_default(),
                gateway_group: {
                    let g = v
                        .get("settings")
                        .map(|x| str_of(x, "gateway_group"))
                        .unwrap_or_default();
                    if g.trim().is_empty() {
                        "MY-VPS".to_string()
                    } else {
                        g
                    }
                },
            };
            let id = cfg.add_server(profile);
            cfg.active_server_id = id;

            // 旧 socks_port 属于 server，现在提升为全局设置
            if let Some(sp) = old.get("socks_port").and_then(|x| x.as_u64()) {
                if (1024..=65535).contains(&sp) {
                    cfg.settings.socks_port = sp as u16;
                }
            }
            if let Some(ver) = v.get("verify") {
                if let Ok(x) = serde_json::from_value::<VerifyConfig>(ver.clone()) {
                    cfg.verify = x;
                }
            }
            if let Some(st) = v.get("settings") {
                if let Ok(x) = serde_json::from_value::<AppSettings>(st.clone()) {
                    // 保留旧文件里已有的全局端口（若存在）
                    let keep_socks = cfg.settings.socks_port;
                    cfg.settings = x;
                    cfg.settings.socks_port = keep_socks;
                }
            }
            cfg.normalize();
            return cfg;
        }

        // ---- 既无 servers 也无 server：全新配置 ----
        let mut cfg = Self::default();
        if let Some(st) = v.get("settings") {
            if let Ok(x) = serde_json::from_value::<AppSettings>(st.clone()) {
                cfg.settings = x;
            }
        }
        if let Some(ver) = v.get("verify") {
            if let Ok(x) = serde_json::from_value::<VerifyConfig>(ver.clone()) {
                cfg.verify = x;
            }
        }
        cfg.normalize();
        cfg
    }

    /// 把配置修成自洽状态：至少一台服务器；`active_server_id` 必须指向存在的项；
    /// 端口必须在合法范围。
    pub fn normalize(&mut self) {
        if self.servers.is_empty() {
            let p = ServerProfile {
                id: generate_server_id(&[]),
                gateway_group: self.settings.gateway_group.clone(),
                ..Default::default()
            };
            self.active_server_id = p.id.clone();
            self.servers.push(p);
        }
        if self.server_by_id(&self.active_server_id).is_none() {
            self.active_server_id = self.servers[0].id.clone();
        }
        if self.settings.socks_port < 1024 {
            self.settings.socks_port = DEFAULT_SOCKS_PORT;
        }
        if self.settings.bridge_port < 1024 {
            self.settings.bridge_port = DEFAULT_BRIDGE_PORT;
        }
    }
}

/// 从 JSON 对象取字符串字段（缺失/类型不符 → 空串）。
fn str_of(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string()
}

/// 配置目录：%APPDATA%\LostCodexGateway（缺失则回退用户主目录）
pub fn config_dir() -> PathBuf {
    if let Ok(appdata) = std::env::var("APPDATA") {
        if !appdata.trim().is_empty() {
            return PathBuf::from(appdata).join("LostCodexGateway");
        }
    }
    if let Ok(home) = std::env::var("USERPROFILE") {
        return PathBuf::from(home).join(".lostcodexgateway");
    }
    PathBuf::from(".").join("LostCodexGateway")
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.json")
}

fn timestamp() -> String {
    chrono::Local::now().format("%Y%m%d_%H%M%S").to_string()
}

/// 加载配置；主文件损坏时尝试最近一次备份。
///
/// 解析走 `parse_json`，因此老版本的扁平 `server` 结构会被自动迁移。
pub fn load() -> Option<GatewayConfig> {
    let path = config_path();
    if let Ok(raw) = fs::read_to_string(&path) {
        if let Ok(cfg) = GatewayConfig::parse_json(&raw) {
            return Some(cfg);
        }
    }
    let dir = config_dir();
    let mut backups: Vec<PathBuf> = fs::read_dir(&dir)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with("config.json.bak_"))
                .unwrap_or(false)
        })
        .collect();
    backups.sort();
    for b in backups.into_iter().rev() {
        if let Ok(raw) = fs::read_to_string(&b) {
            if let Ok(cfg) = GatewayConfig::parse_json(&raw) {
                return Some(cfg);
            }
        }
    }
    None
}

/// 备份当前 config.json（若存在），返回备份路径。
pub fn backup_current() -> std::io::Result<Option<PathBuf>> {
    let path = config_path();
    if !path.exists() {
        return Ok(None);
    }
    let dir = config_dir();
    fs::create_dir_all(&dir)?;
    let bak = dir.join(format!("config.json.bak_{}", timestamp()));
    fs::copy(&path, &bak)?;
    Ok(Some(bak))
}

/// 原子保存配置：写临时文件 → 备份旧文件 → persist 原子替换。
pub fn save(cfg: &GatewayConfig) -> Result<Option<PathBuf>, ConfigError> {
    let dir = config_dir();
    fs::create_dir_all(&dir).map_err(ConfigError::Io)?;
    let backup = backup_current().map_err(ConfigError::Io)?;

    let mut tmp = tempfile::NamedTempFile::new_in(&dir).map_err(ConfigError::Io)?;
    let json = serde_json::to_string_pretty(cfg).map_err(ConfigError::Serialize)?;
    tmp.write_all(json.as_bytes()).map_err(ConfigError::Io)?;
    tmp.flush().map_err(ConfigError::Io)?;
    tmp.persist(config_path())
        .map_err(|e| ConfigError::Io(e.error))?;
    Ok(backup)
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),
    #[error("序列化错误: {0}")]
    Serialize(#[from] serde_json::Error),
}

pub fn known_hosts_path() -> PathBuf {
    if let Ok(home) = std::env::var("USERPROFILE") {
        return Path::new(&home).join(".ssh").join("known_hosts");
    }
    PathBuf::from("known_hosts")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_values() {
        let cfg = GatewayConfig::default();
        assert_eq!(cfg.servers.len(), 1);
        assert_eq!(cfg.active_server().unwrap().port, 22);
        assert_eq!(cfg.socks_port(), 17801);
        assert_eq!(cfg.bridge_port(), 17800);
        assert!(!cfg.is_configured());
        assert!(!cfg.settings.auto_reconnect);
        assert_eq!(cfg.settings.max_reconnect_attempts, 3);
        assert_eq!(cfg.settings.disconnect_policy, "warn_and_block");
        // active_server_id 必须指向真实存在的项
        assert!(cfg.server_by_id(&cfg.active_server_id).is_some());
    }

    #[test]
    fn active_server_id_never_dangles() {
        let mut cfg = GatewayConfig::default();
        cfg.active_server_id = "does-not-exist".into();
        cfg.normalize();
        assert!(cfg.server_by_id(&cfg.active_server_id).is_some());
    }

    #[test]
    fn is_configured_follows_active_server() {
        let mut cfg = GatewayConfig::default();
        assert!(!cfg.is_configured());
        cfg.active_server_mut().unwrap().host = "vps.example.com".into();
        cfg.active_server_mut().unwrap().username = "ubuntu".into();
        assert!(cfg.is_configured());
    }

    #[test]
    fn add_update_remove_server() {
        let mut cfg = GatewayConfig::default();
        let id = cfg.add_server(ServerProfile {
            host: "b.example.com".into(),
            username: "ubuntu".into(),
            ..Default::default()
        });
        assert_eq!(cfg.servers.len(), 2);
        assert!(!id.is_empty());

        // 更新：按 id 匹配，不新增记录
        let mut p = cfg.server_by_id(&id).unwrap().clone();
        p.name = "日本-1".into();
        cfg.update_server(p).unwrap();
        assert_eq!(cfg.server_by_id(&id).unwrap().name, "日本-1");
        assert_eq!(cfg.servers.len(), 2, "更新不应新增记录");

        // 传入不存在的 id 必须明确报错，绝不静默新建（否则前端一个笔误就会
        // 多出一台幽灵服务器，而用户以为只是改了个名字）
        let ghost = ServerProfile {
            id: "hacked".into(),
            host: "x.example.com".into(),
            username: "ubuntu".into(),
            ..Default::default()
        };
        assert!(cfg.update_server(ghost).is_err());
        assert!(cfg.server_by_id("hacked").is_none());
        assert_eq!(cfg.servers.len(), 2);

        // 删除当前选中项 → active 自动落到剩余的第一台
        cfg.active_server_id = id.clone();
        assert!(cfg.remove_server(&id));
        assert!(cfg.server_by_id(&cfg.active_server_id).is_some());
        assert!(!cfg.remove_server(&id)); // 幂等：再删返回 false
    }

    #[test]
    fn generate_id_is_unique_and_stable_shape() {
        let mut list: Vec<ServerProfile> = Vec::new();
        for _ in 0..50 {
            let id = generate_server_id(&list);
            assert!(id.starts_with("srv-"));
            list.push(ServerProfile {
                id: id.clone(),
                ..Default::default()
            });
        }
        let mut uniq: Vec<&str> = list.iter().map(|s| s.id.as_str()).collect();
        uniq.sort_unstable();
        uniq.dedup();
        assert_eq!(uniq.len(), 50, "生成的 id 必须互不相同");
    }

    #[test]
    fn roundtrip_serde_new_format() {
        let mut cfg = GatewayConfig::default();
        cfg.servers[0].host = "a.example.com".into();
        cfg.servers[0].username = "ubuntu".into();
        cfg.add_server(ServerProfile {
            host: "b.example.com".into(),
            username: "root".into(),
            ..Default::default()
        });
        let json = serde_json::to_string(&cfg).unwrap();
        let back = GatewayConfig::parse_json(&json).unwrap();
        assert_eq!(back.servers.len(), 2);
        assert_eq!(back.socks_port(), 17801);
        assert_eq!(back.verify.endpoints.len(), 2);
        assert_eq!(back.active_server_id, cfg.active_server_id);
    }

    /// 核心回归：v0.2.0 的扁平 server 配置必须能无损迁移。
    #[test]
    fn migrates_legacy_single_server_config() {
        let legacy = r#"{
            "server": {
                "host": "legacy.example.com",
                "port": 2222,
                "username": "ubuntu",
                "key_path": "C:\\Users\\you\\.ssh\\id_ed25519",
                "socks_port": 18999,
                "ssh_exe_path": "",
                "server_name": "我的VPS"
            },
            "verify": {
                "endpoints": ["https://api.ipify.org?format=json"],
                "timeout_secs": 10,
                "expected_egress_ip": "203.0.113.47"
            },
            "settings": {
                "auto_reconnect": true,
                "max_reconnect_attempts": 5,
                "disconnect_policy": "warn_and_block",
                "proxy_mode": "unset",
                "gateway_group": "MY-VPS"
            }
        }"#;
        let cfg = GatewayConfig::parse_json(legacy).unwrap();

        assert_eq!(cfg.servers.len(), 1, "旧配置应迁移为恰好一台服务器");
        let s = cfg.active_server().unwrap();
        assert_eq!(s.host, "legacy.example.com");
        assert_eq!(s.port, 2222);
        assert_eq!(s.username, "ubuntu");
        assert_eq!(s.name, "我的VPS");
        // 字段搬家：verify.expected_egress_ip → 服务器
        assert_eq!(s.expected_egress_ip, "203.0.113.47");
        // 字段搬家：server.socks_port → 全局 settings
        assert_eq!(cfg.socks_port(), 18999);
        // 全局设置保留
        assert!(cfg.settings.auto_reconnect);
        assert_eq!(cfg.settings.max_reconnect_attempts, 5);
        // verify 的非搬家字段保留
        assert_eq!(cfg.verify.endpoints.len(), 1);
        assert_eq!(cfg.verify.timeout_secs, 10);
        // 迁移后应可连接
        assert!(cfg.is_configured());
    }

    /// 旧的 `proxy_mode` 是死字段（无任何读取方），迁移时直接丢弃。
    #[test]
    fn legacy_proxy_mode_is_dropped() {
        let legacy = r#"{
            "server": {"host":"h","username":"u","socks_port":17801},
            "settings": {"proxy_mode": "socks5_remote_dns"}
        }"#;
        let cfg = GatewayConfig::parse_json(legacy).unwrap();
        let json = serde_json::to_string(&cfg).unwrap();
        assert!(!json.contains("proxy_mode"), "proxy_mode 不应出现在新配置里");
    }

    /// 旧配置里没有 server_name 时，用 host 兜底作为显示名。
    #[test]
    fn legacy_missing_name_falls_back_to_host() {
        let legacy = r#"{"server":{"host":"only-host.example.com","username":"u"}}"#;
        let cfg = GatewayConfig::parse_json(legacy).unwrap();
        assert_eq!(cfg.active_server().unwrap().display_name(), "only-host.example.com");
    }

    /// 老配置的 socks_port 非法（<1024）时不得写入全局设置。
    #[test]
    fn legacy_invalid_socks_port_is_rejected() {
        let legacy = r#"{"server":{"host":"h","username":"u","socks_port":80}}"#;
        let cfg = GatewayConfig::parse_json(legacy).unwrap();
        assert_eq!(cfg.socks_port(), 17801, "非法端口应回落到默认值");
    }

    #[test]
    fn parse_empty_and_garbage_do_not_panic() {
        let cfg = GatewayConfig::parse_json("{}").unwrap();
        assert_eq!(cfg.servers.len(), 1);
        assert!(GatewayConfig::parse_json("not json").is_err());
    }

    /// 新格式里 active_server_id 指向已删除的服务器时，应自动落到第一台。
    #[test]
    fn new_format_repairs_dangling_active_id() {
        let raw = r#"{
            "servers": [
                {"id":"srv-a","host":"a","username":"u"},
                {"id":"srv-b","host":"b","username":"u"}
            ],
            "active_server_id": "srv-gone"
        }"#;
        let cfg = GatewayConfig::parse_json(raw).unwrap();
        assert_eq!(cfg.active_server_id, "srv-a");
    }

    /// 新格式里 id 缺失的项应被补上，否则无法被选中。
    #[test]
    fn new_format_fills_missing_ids() {
        let raw = r#"{"servers":[{"host":"a","username":"u"},{"host":"b","username":"u"}]}"#;
        let cfg = GatewayConfig::parse_json(raw).unwrap();
        assert!(cfg.servers.iter().all(|s| !s.id.is_empty()));
        let mut ids: Vec<&str> = cfg.servers.iter().map(|s| s.id.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 2);
    }

    #[test]
    fn normalize_restores_ports_and_server() {
        let mut cfg = GatewayConfig {
            servers: Vec::new(),
            active_server_id: String::new(),
            settings: AppSettings {
                socks_port: 1,
                bridge_port: 0,
                ..Default::default()
            },
            ..Default::default()
        };
        cfg.normalize();
        assert_eq!(cfg.servers.len(), 1);
        assert_eq!(cfg.socks_port(), 17801);
        assert_eq!(cfg.bridge_port(), 17800);
        assert!(cfg.server_by_id(&cfg.active_server_id).is_some());
    }

    // ---- 切换语义的不变量 ----
    //
    // 这几条是「切换式多服务器」能对下游透明的前提：Codex CLI 的 HTTP_PROXY
    // 指向一个固定端口，只要端口不变，换服务器就不需要重启 CLI。

    /// **核心不变量**：切换服务器不得改变任何本地端口。
    #[test]
    fn switching_servers_keeps_local_ports() {
        let mut cfg = GatewayConfig::default();
        let a = cfg.active_server_id.clone();
        cfg.active_server_mut().unwrap().host = "a.example.com".into();
        cfg.active_server_mut().unwrap().username = "u".into();
        cfg.settings.socks_port = 19001;
        cfg.settings.bridge_port = 19000;

        let b = cfg.add_server(ServerProfile {
            host: "b.example.com".into(),
            username: "u".into(),
            ..Default::default()
        });

        // 切到 b
        cfg.active_server_id = b.clone();
        cfg.normalize();
        assert_eq!(cfg.socks_port(), 19001, "切换后 SOCKS 端口必须不变");
        assert_eq!(cfg.bridge_port(), 19000, "切换后桥接端口必须不变");
        assert_eq!(cfg.active_server().unwrap().host, "b.example.com");

        // 切回 a
        cfg.active_server_id = a;
        cfg.normalize();
        assert_eq!(cfg.socks_port(), 19001);
        assert_eq!(cfg.bridge_port(), 19000);
    }

    /// 改名/改主机不得改 id —— 否则 `active_server_id` 会指向不存在的项。
    #[test]
    fn rename_and_host_change_preserve_id() {
        let mut cfg = GatewayConfig::default();
        let id = cfg.active_server_id.clone();
        cfg.active_server_mut().unwrap().host = "old.example.com".into();
        cfg.active_server_mut().unwrap().username = "u".into();

        let mut p = cfg.active_server().unwrap().clone();
        p.name = "东京-1".into();
        p.host = "new.example.com".into();
        cfg.update_server(p).unwrap();

        assert_eq!(cfg.active_server_id, id, "id 必须保持稳定");
        assert_eq!(cfg.server_by_id(&id).unwrap().host, "new.example.com");
        assert!(cfg.is_configured());
    }

    /// 预期出口 IP 是**每台独立**的：改一台不能污染另一台。
    #[test]
    fn expected_egress_ip_is_per_server() {
        let mut cfg = GatewayConfig::default();
        cfg.active_server_mut().unwrap().host = "a.example.com".into();
        cfg.active_server_mut().unwrap().username = "u".into();
        cfg.active_server_mut().unwrap().expected_egress_ip = "203.0.113.10".into();

        let b = cfg.add_server(ServerProfile {
            host: "b.example.com".into(),
            username: "u".into(),
            expected_egress_ip: "203.0.113.20".into(),
            ..Default::default()
        });

        assert_eq!(
            cfg.server_by_id(&b).unwrap().expected_egress_ip,
            "203.0.113.20"
        );
        cfg.active_server_id = b;
        cfg.normalize();
        assert_eq!(cfg.active_server().unwrap().expected_egress_ip, "203.0.113.20");

        // 序列化往返后仍各自独立
        let json = serde_json::to_string(&cfg).unwrap();
        let back = GatewayConfig::parse_json(&json).unwrap();
        assert_eq!(back.active_server().unwrap().expected_egress_ip, "203.0.113.20");
        assert_eq!(
            back.servers.iter().find(|s| s.host == "a.example.com").unwrap().expected_egress_ip,
            "203.0.113.10"
        );
    }

    /// 删掉当前选中的服务器后，`active_server_id` 必须落到一台真实存在的服务器上。
    #[test]
    fn removing_active_server_falls_back_to_existing() {
        let mut cfg = GatewayConfig::default();
        cfg.active_server_mut().unwrap().host = "a.example.com".into();
        cfg.active_server_mut().unwrap().username = "u".into();
        let b = cfg.add_server(ServerProfile {
            host: "b.example.com".into(),
            username: "u".into(),
            ..Default::default()
        });
        cfg.active_server_id = b.clone();

        assert!(cfg.remove_server(&b));
        assert!(cfg.server_by_id(&cfg.active_server_id).is_some());
        assert_eq!(cfg.active_server_id, cfg.servers[0].id);
        // 删到只剩最后一台时，仍必须有可用的选中项
        let last = cfg.servers[0].id.clone();
        assert!(cfg.remove_server(&last));
        cfg.normalize();
        assert_eq!(cfg.servers.len(), 1, "列表不允许被删空");
        assert!(cfg.server_by_id(&cfg.active_server_id).is_some());
    }
}
