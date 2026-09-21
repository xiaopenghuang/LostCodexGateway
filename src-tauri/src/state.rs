//! 网关状态机：Rust 端权威状态 + 事件推送 + 脱敏日志缓冲。
//! UI 只展示快照，不自行推断状态。

use crate::config::GatewayConfig;
use crate::verify::VerifyResult;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum GatewayState {
    Unconfigured,
    Ready,
    Connecting,
    TunnelReady,
    EgressVerified,
    Degraded,
    Reconnecting,
    Disconnecting,
    Disconnected,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub ts: String,
    pub level: String, // info | warn | error
    pub component: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewaySnapshot {
    pub state: GatewayState,
    pub config: Option<GatewayConfig>,
    pub last_verify: Option<VerifyResult>,
    pub ssh_pid: Option<u32>,
    pub last_error: Option<String>,
    pub recent_logs: Vec<LogEntry>,
    /// HTTP CONNECT 桥接层状态（隧道活跃期）
    pub bridge_port: Option<u16>,
    pub bridge_connections_total: u64,
    pub bridge_last_target: Option<String>,
    /// 累计被拒绝的连接数（回环/私有网段/并发满等）
    pub bridge_rejects_total: u64,
    /// 最近若干条拒绝记录（带稳定错误码 + 中文说明），供诊断页展示
    pub bridge_recent_rejects: Vec<crate::bridge::RejectRecord>,
}

pub struct Inner {
    pub state: GatewayState,
    pub config: GatewayConfig,
    pub last_verify: Option<VerifyResult>,
    pub tunnel_pid: Option<u32>,
    /// 断开信号发送端：disconnect 时触发，监控任务立即终止子进程。
    pub tunnel_abort: Option<tokio::sync::oneshot::Sender<()>>,
    /// HTTP CONNECT 桥接层（隧道活跃期运行）。
    pub bridge_port: Option<u16>,
    pub bridge_task: Option<tokio::task::JoinHandle<()>>,
    pub bridge_stats: Option<crate::bridge::BridgeStats>,
    /// 最近一次网络诊断报告（不进入 snapshot，单独命令取）。
    pub last_diag_report: Option<crate::diagnostics::DiagReport>,
    pub last_error: Option<String>,
    pub logs: VecDeque<LogEntry>,
    /// 世代号：每次 connect/disconnect 自增，旧监控任务发现不匹配即退出。
    pub generation: u64,
}

impl Inner {
    pub fn snapshot(&self) -> GatewaySnapshot {
        // 只取一次桥接统计快照，避免多处调用之间计数变化导致前后不一致。
        let bridge = self.bridge_stats.as_ref().map(|s| s.snapshot());
        GatewaySnapshot {
            state: self.state,
            config: Some(self.config.clone()),
            last_verify: self.last_verify.clone(),
            ssh_pid: self.tunnel_pid,
            last_error: self.last_error.clone(),
            recent_logs: self.logs.iter().cloned().collect(),
            bridge_port: self.bridge_port,
            bridge_connections_total: bridge.as_ref().map(|b| b.connections_total).unwrap_or(0),
            bridge_last_target: bridge.as_ref().and_then(|b| b.last_target.clone()),
            bridge_rejects_total: bridge.as_ref().map(|b| b.rejects_total).unwrap_or(0),
            bridge_recent_rejects: bridge.map(|b| b.recent_rejects).unwrap_or_default(),
        }
    }

    pub fn log(&mut self, level: &str, component: &str, message: impl Into<String>) {
        self.logs.push_back(LogEntry {
            ts: chrono::Local::now().format("%H:%M:%S").to_string(),
            level: level.to_string(),
            component: component.to_string(),
            message: message.into(),
        });
        while self.logs.len() > 500 {
            self.logs.pop_front();
        }
    }
}

pub struct GatewayStateMachine {
    pub inner: Arc<Mutex<Inner>>,
}

impl GatewayStateMachine {
    pub fn new(config: GatewayConfig) -> Self {
        let initial = if config.is_server_complete() {
            GatewayState::Ready
        } else {
            GatewayState::Unconfigured
        };
        Self {
            inner: Arc::new(Mutex::new(Inner {
                state: initial,
                config,
                last_verify: None,
                tunnel_pid: None,
                tunnel_abort: None,
                bridge_port: None,
                bridge_task: None,
                bridge_stats: None,
                last_diag_report: None,
                last_error: None,
                logs: VecDeque::new(),
                generation: 0,
            })),
        }
    }

    pub fn log(&self, level: &str, component: &str, message: impl Into<String>) {
        self.inner.lock().log(level, component, message);
    }

    pub fn set_state(&self, state: GatewayState) {
        self.inner.lock().state = state;
    }

    pub fn snapshot(&self) -> GatewaySnapshot {
        self.inner.lock().snapshot()
    }
}
