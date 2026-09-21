//! 配置模块：加载/原子保存（写前备份，tempfile::persist 原子替换）。
//! 私钥只保存路径字符串；本模块绝不读取私钥内容。

use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub key_path: String,
    pub socks_port: u16,
    pub ssh_exe_path: String,
    pub server_name: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: String::new(),
            port: 22,
            username: String::new(),
            key_path: String::new(),
            socks_port: 17801,
            ssh_exe_path: String::new(),
            server_name: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct VerifyConfig {
    pub endpoints: Vec<String>,
    pub timeout_secs: u64,
    /// 用户手动配置的预期服务器出口 IP（可选；空 = 不校验）
    pub expected_egress_ip: String,
}

impl Default for VerifyConfig {
    fn default() -> Self {
        Self {
            endpoints: vec![
                "https://api.ipify.org?format=json".to_string(),
                "https://ipinfo.io/ip".to_string(),
            ],
            timeout_secs: 15,
            expected_egress_ip: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub auto_reconnect: bool,
    pub max_reconnect_attempts: u32,
    pub disconnect_policy: String,
    /// 代理注入模式："unset"（M2 实测前禁用）| "socks_all" | "http_bridge"
    pub proxy_mode: String,
    /// Mihomo 规则中本工具网关对应的代理组名（诊断关联用，默认 MY-VPS）
    pub gateway_group: String,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            auto_reconnect: false,
            max_reconnect_attempts: 3,
            disconnect_policy: "warn_and_block".to_string(),
            proxy_mode: "unset".to_string(),
            gateway_group: "MY-VPS".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GatewayConfig {
    pub server: ServerConfig,
    pub verify: VerifyConfig,
    pub settings: AppSettings,
}

impl GatewayConfig {
    pub fn is_server_complete(&self) -> bool {
        !self.server.host.trim().is_empty() && !self.server.username.trim().is_empty()
    }
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
pub fn load() -> Option<GatewayConfig> {
    let path = config_path();
    if let Ok(raw) = fs::read_to_string(&path) {
        if let Ok(cfg) = serde_json::from_str::<GatewayConfig>(&raw) {
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
            if let Ok(cfg) = serde_json::from_str::<GatewayConfig>(&raw) {
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
    tmp.persist(config_path()).map_err(|e| ConfigError::Io(e.error))?;
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
        assert_eq!(cfg.server.port, 22);
        assert_eq!(cfg.server.socks_port, 17801);
        assert!(!cfg.is_server_complete());
        assert!(!cfg.settings.auto_reconnect);
        assert_eq!(cfg.settings.max_reconnect_attempts, 3);
        assert_eq!(cfg.settings.disconnect_policy, "warn_and_block");
    }

    #[test]
    fn server_complete_check() {
        let mut cfg = GatewayConfig::default();
        cfg.server.host = "vps.example.com".into();
        cfg.server.username = "ubuntu".into();
        assert!(cfg.is_server_complete());
    }

    #[test]
    fn roundtrip_serde() {
        let cfg = GatewayConfig::default();
        let json = serde_json::to_string(&cfg).unwrap();
        let back: GatewayConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.server.socks_port, 17801);
        assert_eq!(back.verify.endpoints.len(), 2);
    }
}
