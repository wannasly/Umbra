//! Event names and payload structs emitted to the frontend.
//! Must match the `Events` interface in src/lib/ipc.ts exactly.

use serde::Serialize;

pub const EV_CONN_STATE: &str = "conn://state";
pub const EV_CORE_LOG: &str = "core://log";
pub const EV_TRAFFIC: &str = "traffic://stats";
pub const EV_PING_RESULT: &str = "ping://result";
pub const EV_DOWNLOAD_PROGRESS: &str = "core://download-progress";
pub const EV_CORE_CRASHED: &str = "core://crashed";
pub const EV_SUB_UPDATED: &str = "sub://updated";
/// Backend-initiated TUN attempts (tray, startup) have no command result to
/// reject, so they ask the UI to run the elevation flow over this channel.
pub const EV_NEEDS_ELEVATION: &str = "ui://needs-elevation";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrafficStats {
    pub up_bps: f64,
    pub down_bps: f64,
    pub up_total: f64,
    pub down_total: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PingResult {
    pub server_id: String,
    pub latency_ms: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadProgress {
    pub phase: DownloadPhase,
    pub downloaded: u64,
    pub total: u64,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DownloadPhase {
    Download,
    Extract,
    Done,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CrashInfo {
    pub code: Option<i32>,
    pub will_restart: bool,
    pub attempt: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubUpdated {
    pub id: String,
    pub added: usize,
    pub removed: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogLine {
    /// epoch ms
    pub ts: i64,
    pub level: String,
    pub message: String,
}
