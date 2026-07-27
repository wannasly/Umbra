use std::path::PathBuf;

use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{Mutex, RwLock};

use crate::events::EV_CONN_STATE;
use crate::models::{ConnectionState, ProfileStore, Settings};
use crate::singbox::download::ReleaseInfo;
use crate::singbox::process::{CoreHandle, LogBuffer};

/// Global app state managed by Tauri (`app.manage(AppState::new(..))`).
/// Modules add their own handles here as they are implemented.
pub struct AppState {
    pub settings: RwLock<Settings>,
    pub profiles: RwLock<ProfileStore>,
    pub conn: RwLock<ConnectionState>,
    /// %APPDATA%\com.umbra.proxy
    pub data_dir: PathBuf,
    /// Session cache of the latest sing-box release (GitHub unauthenticated: 60 req/h).
    pub release_cache: Mutex<Option<ReleaseInfo>>,
    /// Running sing-box core, if any.
    pub core: Mutex<Option<CoreHandle>>,
    /// Ring buffer of recent core log lines (cap 2000).
    pub logs: LogBuffer,
    /// Serializes connect/disconnect/set_mode. Deliberately held across .await
    /// (it guards an operation, not data): two overlapping connects would both
    /// store a `CoreHandle` and orphan one running core. Always the outermost
    /// lock; nothing acquires it while holding another lock.
    pub conn_ops: Mutex<()>,
}

impl AppState {
    pub fn new(data_dir: PathBuf, settings: Settings, profiles: ProfileStore) -> Self {
        let mode = settings.mode;
        Self {
            settings: RwLock::new(settings),
            profiles: RwLock::new(profiles),
            conn: RwLock::new(ConnectionState::disconnected(mode)),
            data_dir,
            release_cache: Mutex::new(None),
            core: Mutex::new(None),
            logs: LogBuffer::new(),
            conn_ops: Mutex::new(()),
        }
    }
}

/// Mutate the connection state under a statement-scoped write lock, then emit
/// `conn://state` with the resulting snapshot.
pub async fn update_conn(
    app: &AppHandle,
    mutate: impl FnOnce(&mut ConnectionState),
) -> ConnectionState {
    let state = app.state::<AppState>();
    let snapshot = {
        let mut conn = state.conn.write().await;
        mutate(&mut conn);
        conn.clone()
    };
    if let Err(e) = app.emit(EV_CONN_STATE, &snapshot) {
        eprintln!("[umbra] failed to emit {EV_CONN_STATE}: {e}");
    }
    crate::tray::sync(app, &snapshot).await;
    snapshot
}
