//! Connection lifecycle commands: connect, disconnect, mode switch, pings,
//! url test and the log ring buffer accessors.

use tauri::{AppHandle, Emitter, Manager};

use crate::error::{AppError, AppResult};
use crate::events::{LogLine, PingResult, EV_NEEDS_ELEVATION, EV_PING_RESULT};
use crate::models::{ConnStatus, ConnectionState, Mode, ServerEntry};
use crate::net::ping::{self, PingTarget};
use crate::proxy::{elevation, system_proxy};
use crate::singbox::{clash_api, config, process, version};
use crate::state::{update_conn, AppState};
use crate::storage;

const CLASH_PORT_START: u16 = 9095;

async fn pick_free_port(start: u16, avoid: Option<u16>) -> AppResult<u16> {
    for offset in 0..200u16 {
        let Some(port) = start.checked_add(offset) else {
            break;
        };
        if Some(port) == avoid {
            continue;
        }
        if tokio::net::TcpListener::bind(("127.0.0.1", port))
            .await
            .is_ok()
        {
            return Ok(port);
        }
    }
    Err(AppError::Internal(format!(
        "no free port found starting at {start}"
    )))
}

fn random_secret() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub(crate) async fn do_connect(app: &AppHandle, server_id: String) -> AppResult<ConnectionState> {
    let state = app.state::<AppState>();
    version::ensure_compatible_core(&version::core_path(&state.data_dir))?;
    let servers: Vec<ServerEntry> =
        { state.profiles.read().await.all_servers().cloned().collect() };
    // Snapshotted here and carried in the connection state for the whole
    // session: the profile entry can be deleted while the tunnel runs, and the
    // UI must still be able to name what it is connected to.
    let server_name = servers
        .iter()
        .find(|s| s.id == server_id)
        .map(|s| s.name.clone())
        .ok_or_else(|| AppError::NotFound(format!("server {server_id}")))?;
    let settings = { state.settings.read().await.clone() };
    if settings.mode == Mode::Tun && !elevation::is_elevated() {
        return Err(AppError::NeedsElevation);
    }

    // Reconnect semantics: tear down any previous core first. This also
    // restores the registry proxy whenever we still own it, which is what
    // keeps system proxy and TUN mutually exclusive — a TUN run can never
    // start while HKCU still points at our mixed inbound.
    process::stop(app).await;

    update_conn(app, |c| {
        c.status = ConnStatus::Connecting;
        c.server_id = Some(server_id.clone());
        c.server_name = Some(server_name.clone());
        c.mode = settings.mode;
        c.since_ms = None;
        c.error = None;
    })
    .await;

    let started: AppResult<()> = async {
        let clash_port = pick_free_port(CLASH_PORT_START, Some(settings.mixed_port)).await?;
        let secret = random_secret();
        let refs: Vec<&ServerEntry> = servers.iter().collect();
        let generated = config::generate(&settings, &refs, &server_id, clash_port, &secret)?;
        process::start(app, &generated, settings.mode, settings.mixed_port).await?;
        if settings.mode == Mode::SystemProxy {
            if let Err(e) = system_proxy::enable_proxy(&state, settings.mixed_port).await {
                process::stop(app).await;
                return Err(e);
            }
        }
        Ok(())
    }
    .await;

    match started {
        Ok(()) => {
            {
                let mut s = state.settings.write().await;
                s.selected_server_id = Some(server_id.clone());
                if let Err(e) = storage::save_settings(&state.data_dir, &s) {
                    eprintln!("[umbra] failed to persist selected server: {e}");
                }
            }
            Ok(update_conn(app, |c| {
                c.status = ConnStatus::Connected;
                c.since_ms = Some(process::now_ms());
                c.error = None;
            })
            .await)
        }
        Err(e) => {
            let message = e.to_string();
            update_conn(app, |c| {
                *c = ConnectionState::disconnected(settings.mode);
                c.error = Some(message.clone());
            })
            .await;
            Err(e)
        }
    }
}

/// Ask the UI to run the elevation flow. Command callers get `NEEDS_ELEVATION`
/// back and handle it themselves; the tray and the startup path have no such
/// return channel, so they emit this instead.
pub(crate) fn notify_needs_elevation(app: &AppHandle, err: &AppError) {
    if !matches!(err, AppError::NeedsElevation) {
        return;
    }
    if let Err(e) = app.emit(EV_NEEDS_ELEVATION, ()) {
        eprintln!("[umbra] failed to emit {EV_NEEDS_ELEVATION}: {e}");
    }
}

/// Startup auto-connect. `--resume-tun` (the elevated relaunch picking the
/// connection back up) and `connectOnStartup` both funnel through this single
/// call site, so the two triggers can never fire two connects.
pub(crate) async fn startup_connect(app: &AppHandle) {
    let state = app.state::<AppState>();
    let Some(server_id) = ({ state.settings.read().await.selected_server_id.clone() }) else {
        state.logs.push_now(
            "warn",
            "[umbra] startup connect skipped: no server selected",
        );
        return;
    };
    let _ops = state.conn_ops.lock().await;
    if state.conn.read().await.status != ConnStatus::Disconnected {
        return;
    }
    if let Err(e) = do_connect(app, server_id).await {
        state
            .logs
            .push_now("error", format!("[umbra] startup connect failed: {e}"));
        notify_needs_elevation(app, &e);
    }
}

pub(crate) async fn do_disconnect(app: &AppHandle) -> AppResult<ConnectionState> {
    update_conn(app, |c| {
        c.status = ConnStatus::Stopping;
        c.error = None;
    })
    .await;
    process::stop(app).await;
    let mode = { app.state::<AppState>().settings.read().await.mode };
    Ok(update_conn(app, |c| *c = ConnectionState::disconnected(mode)).await)
}

#[tauri::command]
pub async fn connect(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    server_id: String,
) -> AppResult<ConnectionState> {
    let _ops = state.conn_ops.lock().await;
    do_connect(&app, server_id).await
}

#[tauri::command]
pub async fn disconnect(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
) -> AppResult<ConnectionState> {
    let _ops = state.conn_ops.lock().await;
    do_disconnect(&app).await
}

pub(crate) async fn do_set_mode(app: &AppHandle, mode: Mode) -> AppResult<ConnectionState> {
    let state = app.state::<AppState>();
    let current = { state.settings.read().await.mode };
    if current == mode {
        return Ok(state.conn.read().await.clone());
    }
    if mode == Mode::Tun && !elevation::is_elevated() {
        return Err(AppError::NeedsElevation);
    }
    {
        let mut settings = state.settings.write().await;
        settings.mode = mode;
        storage::save_settings(&state.data_dir, &settings)?;
    }
    let (status, server_id) = {
        let conn = state.conn.read().await;
        (conn.status, conn.server_id.clone())
    };
    if status == ConnStatus::Connected {
        if let Some(id) = server_id {
            do_disconnect(app).await?;
            return do_connect(app, id).await;
        }
    }
    Ok(update_conn(app, |c| c.mode = mode).await)
}

#[tauri::command]
pub async fn set_mode(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    mode: Mode,
) -> AppResult<ConnectionState> {
    let _ops = state.conn_ops.lock().await;
    do_set_mode(&app, mode).await
}

#[tauri::command]
pub async fn ping_servers(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    ids: Vec<String>,
) -> AppResult<()> {
    let targets: Vec<PingTarget> = {
        let profiles = state.profiles.read().await;
        ids.iter()
            .filter_map(|id| {
                profiles.find_server(id).map(|s| PingTarget {
                    id: s.id.clone(),
                    host: s.server.clone(),
                    port: s.port,
                })
            })
            .collect()
    };
    tauri::async_runtime::spawn(ping::ping_many(app, targets));
    Ok(())
}

#[tauri::command]
pub async fn url_test_active(app: AppHandle, state: tauri::State<'_, AppState>) -> AppResult<u32> {
    let (status, server_id) = {
        let conn = state.conn.read().await;
        (conn.status, conn.server_id.clone())
    };
    if status != ConnStatus::Connected {
        return Err(AppError::Internal("not connected".into()));
    }
    let server_id = server_id.ok_or_else(|| AppError::Internal("no active server".into()))?;
    let (port, secret, tag) = {
        let core = state.core.lock().await;
        let handle = core
            .as_ref()
            .ok_or_else(|| AppError::Internal("core is not running".into()))?;
        // The url test runs through the core's control api, which may still be
        // starting up on a large subscription. Say that, rather than surfacing
        // a bare connection-refused.
        if !handle.clash_ready() {
            return Err(AppError::Internal(
                "the core's control api is not up yet; try again in a moment".into(),
            ));
        }
        let tag = handle
            .tag_by_server_id
            .get(&server_id)
            .cloned()
            .ok_or_else(|| AppError::NotFound(format!("server {server_id} in running config")))?;
        (handle.clash_port, handle.clash_secret.clone(), tag)
    };
    let ping_url = { state.settings.read().await.ping_url.clone() };
    let ms = clash_api::delay(port, &secret, &tag, &ping_url).await?;
    {
        let mut profiles = state.profiles.write().await;
        if let Some(server) = profiles.find_server_mut(&server_id) {
            server.last_ping_ms = Some(ms);
        }
        if let Err(e) = storage::save_profiles(&state.data_dir, &profiles) {
            eprintln!("[umbra] failed to persist url test result: {e}");
        }
    }
    let _ = app.emit(
        EV_PING_RESULT,
        &PingResult {
            server_id,
            latency_ms: Some(ms),
        },
    );
    Ok(ms)
}

#[tauri::command]
pub async fn get_recent_logs(
    state: tauri::State<'_, AppState>,
    limit: usize,
) -> AppResult<Vec<LogLine>> {
    Ok(state.logs.tail(limit))
}

#[tauri::command]
pub async fn clear_logs(state: tauri::State<'_, AppState>) -> AppResult<()> {
    state.logs.clear();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_is_32_hex_and_random() {
        let s = random_secret();
        assert_eq!(s.len(), 32);
        assert!(s.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_ne!(s, random_secret());
    }

    #[tokio::test]
    async fn pick_free_port_skips_bound_port() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let taken = listener.local_addr().unwrap().port();
        if taken > 60000 {
            return; // avoid the u16 range edge; ephemeral ports this high are rare
        }
        let picked = pick_free_port(taken, None).await.unwrap();
        assert!(picked > taken);
        assert!(picked <= taken + 200);
    }

    #[tokio::test]
    async fn pick_free_port_returns_start_when_free() {
        let mut matched = false;
        for _ in 0..10 {
            let probe = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = probe.local_addr().unwrap().port();
            drop(probe);
            if let Ok(picked) = pick_free_port(port, None).await {
                if picked == port {
                    matched = true;
                    break;
                }
            }
        }
        assert!(matched);
    }

    #[tokio::test]
    async fn pick_free_port_skips_avoided_port() {
        let probe = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = probe.local_addr().unwrap().port();
        drop(probe);
        if port > 60000 {
            return; // avoid the u16 range edge; ephemeral ports this high are rare
        }
        let picked = pick_free_port(port, Some(port)).await.unwrap();
        assert!(picked > port);
        assert!(picked <= port + 200);
    }
}
