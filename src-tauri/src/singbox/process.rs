//! sing-box process lifecycle: config write, `check`, spawn, readiness,
//! log pipeline (ring buffer + 250ms batch flusher) and crash supervision
//! with 1s/3s/9s backoff (max 3 attempts, counter resets after 60s uptime).

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager};
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Notify;
use tokio::time::Instant;

use crate::error::{AppError, AppResult};
use crate::events::{CrashInfo, LogLine, EV_CORE_CRASHED, EV_CORE_LOG};
use crate::models::{ConnStatus, Mode};
use crate::proxy::system_proxy;
use crate::singbox::clash_api;
use crate::singbox::config::GeneratedConfig;
use crate::singbox::job;
use crate::singbox::version::core_path;
use crate::state::{update_conn, AppState};

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const LOG_CAP: usize = 2000;
const FLUSH_INTERVAL: Duration = Duration::from_millis(250);
const KILL_GRACE: Duration = Duration::from_secs(3);
const STOP_JOIN_TIMEOUT: Duration = Duration::from_secs(12);
const MAX_RESTART_ATTEMPTS: u32 = 3;
const HEALTHY_RESET: Duration = Duration::from_secs(60);

/// How long a connect blocks waiting for the child to prove it is not going to
/// die on the spot. Everything that can still fail after `sing-box check`
/// passed — a busy inbound port, a missing wintun.dll, a refused adapter —
/// fails inside this window.
const STARTUP_CONFIRM: Duration = Duration::from_millis(1500);
const STARTUP_POLL: Duration = Duration::from_millis(100);

/// Clash-api readiness budget. It is *not* part of the connect path (see
/// `watch_clash_api`); it only decides when to log "this core is running
/// degraded". A 57-outbound subscription with a urltest group and remote
/// rule-sets routinely needs far longer than the 8s this used to allow.
const READY_BASE: Duration = Duration::from_secs(30);
const READY_PER_OUTBOUND: Duration = Duration::from_millis(250);
const READY_MAX: Duration = Duration::from_secs(120);
/// Poll cadence while the api is still coming up, and once it is up.
const READY_POLL: Duration = Duration::from_millis(300);
const HEALTH_POLL: Duration = Duration::from_secs(5);
/// Longest user-facing error message; the full text goes to the Logs page.
const MAX_REASON_CHARS: usize = 160;

/// Startup budget for the clash api, scaled by how much work the core has to
/// do before it can serve one: every outbound is another urltest probe and
/// another TLS handshake competing for the same startup window.
fn ready_deadline(outbound_count: usize) -> Duration {
    let scaled = READY_BASE + READY_PER_OUTBOUND * outbound_count as u32;
    scaled.min(READY_MAX)
}

pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

// ---------------------------------------------------------------------------
// Log ring buffer
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct LogBuffer {
    inner: std::sync::Mutex<LogInner>,
}

#[derive(Default)]
struct LogInner {
    ring: VecDeque<LogLine>,
    pending: Vec<LogLine>,
}

impl LogBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, LogInner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn push(&self, line: LogLine) {
        let mut g = self.lock();
        g.ring.push_back(line.clone());
        while g.ring.len() > LOG_CAP {
            g.ring.pop_front();
        }
        g.pending.push(line);
        let overflow = g.pending.len().saturating_sub(LOG_CAP);
        if overflow > 0 {
            g.pending.drain(..overflow);
        }
    }

    pub fn push_now(&self, level: &str, message: impl Into<String>) {
        self.push(LogLine {
            ts: now_ms(),
            level: level.into(),
            message: message.into(),
        });
    }

    pub fn tail(&self, limit: usize) -> Vec<LogLine> {
        let g = self.lock();
        let skip = g.ring.len().saturating_sub(limit);
        g.ring.iter().skip(skip).cloned().collect()
    }

    pub fn clear(&self) {
        let mut g = self.lock();
        g.ring.clear();
        g.pending.clear();
    }

    pub fn drain_pending(&self) -> Vec<LogLine> {
        std::mem::take(&mut self.lock().pending)
    }
}

/// Emits accumulated new log lines as one `core://log` batch every 250ms.
/// Lives for the whole app session.
pub fn spawn_log_flusher(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut ticker = tokio::time::interval(FLUSH_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            let batch = app.state::<AppState>().logs.drain_pending();
            if !batch.is_empty() {
                if let Err(e) = app.emit(EV_CORE_LOG, &batch) {
                    eprintln!("[umbra] failed to emit {EV_CORE_LOG}: {e}");
                }
            }
        }
    });
}

// ---------------------------------------------------------------------------
// Log line parsing
// ---------------------------------------------------------------------------

/// Best-effort parse of a sing-box log line:
/// `+0300 2026-07-27 12:00:00 INFO [tag] message`.
/// Anything that does not match falls back to level "info", raw message.
fn parse_log_line(line: &str) -> LogLine {
    try_parse(line).unwrap_or_else(|| LogLine {
        ts: now_ms(),
        level: "info".into(),
        message: line.to_string(),
    })
}

fn try_parse(line: &str) -> Option<LogLine> {
    let mut parts = line.splitn(5, ' ');
    let offset = parts.next()?;
    let valid_offset = offset.len() == 5
        && (offset.starts_with('+') || offset.starts_with('-'))
        && offset[1..].bytes().all(|b| b.is_ascii_digit());
    if !valid_offset {
        return None;
    }
    let date = parts.next()?;
    let time = parts.next()?;
    let level = normalize_level(parts.next()?)?;
    let message = parts.next().unwrap_or("").to_string();
    let ts = chrono::DateTime::parse_from_str(
        &format!("{date} {time} {offset}"),
        "%Y-%m-%d %H:%M:%S %z",
    )
    .map(|dt| dt.timestamp_millis())
    .unwrap_or_else(|_| now_ms());
    Some(LogLine { ts, level, message })
}

fn normalize_level(token: &str) -> Option<String> {
    let lower = token.to_ascii_lowercase();
    match lower.as_str() {
        "trace" | "debug" | "info" | "warn" | "error" => Some(lower),
        "warning" => Some("warn".into()),
        "fatal" | "panic" => Some("error".into()),
        _ => None,
    }
}

fn tail_str(s: &str, max: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        return s.to_string();
    }
    chars[chars.len() - max..].iter().collect()
}

/// Squeeze a multi-line core failure into something a toast can show: the
/// first meaningful line, capped. The full text is pushed to the log ring so
/// the Logs page keeps every detail.
fn short_reason(text: &str) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("no details available");
    let chars: Vec<char> = line.chars().collect();
    if chars.len() <= MAX_REASON_CHARS {
        return line.to_string();
    }
    let mut out: String = chars[..MAX_REASON_CHARS - 1].iter().collect();
    out.push('…');
    out
}

fn backoff_secs(attempt: u32) -> u64 {
    match attempt {
        0 | 1 => 1,
        2 => 3,
        _ => 9,
    }
}

// ---------------------------------------------------------------------------
// Core handle
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct CoreShared {
    pub stopping: AtomicBool,
    pub stop_notify: Notify,
    /// Whether the clash api is answering right now. Purely an *extra*: the
    /// tunnel itself does not go through it, so a false here means degraded
    /// (no live traffic graph, no hot server switching), never broken.
    pub clash_ready: AtomicBool,
}

pub struct CoreHandle {
    pub shared: Arc<CoreShared>,
    pub tag_by_server_id: HashMap<String, String>,
    pub clash_port: u16,
    pub clash_secret: String,
    pub supervisor: tauri::async_runtime::JoinHandle<()>,
    pub traffic: tauri::async_runtime::JoinHandle<()>,
    pub readiness: tauri::async_runtime::JoinHandle<()>,
}

impl CoreHandle {
    /// True when the clash api can serve a request right now. Callers that
    /// need it (live selector switch, url test) degrade instead of failing the
    /// whole connection when this is false.
    pub fn clash_ready(&self) -> bool {
        self.shared.clash_ready.load(Ordering::SeqCst)
    }
}

/// Everything needed to (re)spawn the same core run. The clash-api
/// coordinates are not in here: `watch_clash_api` owns them for the whole
/// session and outlives every individual child.
#[derive(Clone)]
struct RunCtx {
    exe: PathBuf,
    config_path: PathBuf,
    work_dir: PathBuf,
    mode: Mode,
    mixed_port: u16,
}

// ---------------------------------------------------------------------------
// Start / stop
// ---------------------------------------------------------------------------

/// Write the config, validate it with `sing-box check`, spawn `sing-box run`,
/// confirm the child stayed up, then hand it to the crash supervisor and store
/// the `CoreHandle` in `AppState`.
///
/// Deliberately does **not** wait for the clash api. Nothing on the data path
/// goes through it — the tunnel carries traffic the moment the inbound binds —
/// so blocking the connect on it (and killing a working core when it is slow)
/// only ever turned a healthy 57-outbound run into a failed one. The api is
/// picked up in the background by `watch_clash_api`.
pub async fn start(
    app: &AppHandle,
    config: &GeneratedConfig,
    mode: Mode,
    mixed_port: u16,
) -> AppResult<()> {
    let state = app.state::<AppState>();
    let exe = core_path(&state.data_dir);
    if !exe.exists() {
        return Err(AppError::CoreNotInstalled);
    }
    let config_dir = state.data_dir.join("config");
    tokio::fs::create_dir_all(&config_dir).await?;
    let config_path = config_dir.join("generated.json");
    tokio::fs::write(&config_path, serde_json::to_vec_pretty(&config.json)?).await?;
    let work_dir = state.data_dir.join("work");
    tokio::fs::create_dir_all(&work_dir).await?;

    check_config(&state.logs, &exe, &config_path, &work_dir).await?;

    state.logs.push_now("info", "[umbra] starting sing-box");
    let ctx = RunCtx {
        exe,
        config_path,
        work_dir,
        mode,
        mixed_port,
    };
    let mut child = spawn_child(app, &ctx)?;
    if let Err(e) = confirm_started(&mut child).await {
        graceful_kill(&mut child).await;
        state
            .logs
            .push_now("error", format!("[umbra] sing-box failed to start: {e}"));
        return Err(e);
    }
    state.logs.push_now("info", "[umbra] sing-box started");

    let shared = Arc::new(CoreShared::default());
    let supervisor =
        tauri::async_runtime::spawn(supervise(app.clone(), shared.clone(), child, ctx));
    #[cfg(windows)]
    let traffic = if mode == Mode::Tun {
        tauri::async_runtime::spawn(clash_api::tun_traffic_loop(app.clone(), shared.clone()))
    } else {
        tauri::async_runtime::spawn(clash_api::traffic_loop(
            app.clone(),
            shared.clone(),
            config.clash_port,
            config.clash_secret.clone(),
        ))
    };
    #[cfg(not(windows))]
    let traffic = tauri::async_runtime::spawn(clash_api::traffic_loop(
        app.clone(),
        shared.clone(),
        config.clash_port,
        config.clash_secret.clone(),
    ));
    let readiness = tauri::async_runtime::spawn(watch_clash_api(
        app.clone(),
        shared.clone(),
        config.clash_port,
        config.clash_secret.clone(),
        ready_deadline(config.tag_by_server_id.len()),
    ));
    let handle = CoreHandle {
        shared,
        tag_by_server_id: config.tag_by_server_id.clone(),
        clash_port: config.clash_port,
        clash_secret: config.clash_secret.clone(),
        supervisor,
        traffic,
        readiness,
    };
    {
        *state.core.lock().await = Some(handle);
    }
    Ok(())
}

/// Block only long enough to catch a core that dies on the spot. Anything
/// still running after this window is a working core, however busy its api is.
async fn confirm_started(child: &mut Child) -> AppResult<()> {
    let deadline = Instant::now() + STARTUP_CONFIRM;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let code = status
                    .code()
                    .map_or_else(|| "unknown".to_string(), |c| c.to_string());
                return Err(AppError::CoreStartFailed(format!(
                    "the core exited immediately (code {code}) — see Logs for details"
                )));
            }
            Ok(None) => {}
            Err(e) => {
                return Err(AppError::CoreStartFailed(format!(
                    "could not poll the core process: {e}"
                )))
            }
        }
        if Instant::now() >= deadline {
            return Ok(());
        }
        tokio::time::sleep(STARTUP_POLL).await;
    }
}

/// Track the clash api for the whole core session: flip `clash_ready` as it
/// comes and goes (a crash-restart takes it down and brings it back), announce
/// the first success, and warn once — without touching the tunnel — if it never
/// shows up inside the adaptive deadline.
async fn watch_clash_api(
    app: AppHandle,
    shared: Arc<CoreShared>,
    port: u16,
    secret: String,
    deadline: Duration,
) {
    let started = Instant::now();
    let mut warned = false;
    loop {
        if shared.stopping.load(Ordering::SeqCst) {
            return;
        }
        let ok = clash_api::probe(port, &secret).await;
        let was = shared.clash_ready.swap(ok, Ordering::SeqCst);
        {
            let state = app.state::<AppState>();
            match (was, ok) {
                (false, true) => {
                    warned = false;
                    state.logs.push_now(
                        "info",
                        format!(
                            "[umbra] clash api ready after {:.1}s",
                            started.elapsed().as_secs_f32()
                        ),
                    );
                }
                (true, false) => state.logs.push_now(
                    "warn",
                    "[umbra] clash api stopped answering; live traffic stats and server \
                     switching are paused until it returns",
                ),
                _ => {
                    if !ok && !warned && started.elapsed() >= deadline {
                        warned = true;
                        state.logs.push_now(
                            "warn",
                            format!(
                                "[umbra] clash api still unreachable after {}s; the tunnel keeps \
                                 running without live traffic stats or hot server switching",
                                deadline.as_secs()
                            ),
                        );
                    }
                }
            }
        }
        if sleep_or_stop(&shared, if ok { HEALTH_POLL } else { READY_POLL }).await {
            return;
        }
    }
}

/// Best-effort full teardown: restore the system proxy when owned, then kill
/// the sing-box child this app spawned. Idempotent; never fails.
pub async fn stop(app: &AppHandle) {
    let state = app.state::<AppState>();
    let owned = { state.settings.read().await.proxy_owned };
    if owned {
        if let Err(e) = system_proxy::disable_proxy(&state).await {
            eprintln!("[umbra] failed to restore system proxy: {e}");
        }
    }
    let taken = { state.core.lock().await.take() };
    if let Some(handle) = taken {
        state.logs.push_now("info", "[umbra] stopping sing-box");
        handle.shared.stopping.store(true, Ordering::SeqCst);
        handle.shared.stop_notify.notify_one();
        if tokio::time::timeout(STOP_JOIN_TIMEOUT, handle.supervisor)
            .await
            .is_err()
        {
            eprintln!("[umbra] sing-box supervisor did not finish in time");
        }
        handle.traffic.abort();
        handle.readiness.abort();
        state.logs.push_now("info", "[umbra] sing-box stopped");
    }
}

/// `sing-box check` on the generated config. The full failure text goes to the
/// log ring; the returned error carries one readable line so the toast is not
/// a screenful of core output.
async fn check_config(
    logs: &LogBuffer,
    exe: &Path,
    config_path: &Path,
    work_dir: &Path,
) -> AppResult<()> {
    let mut cmd = Command::new(exe);
    cmd.arg("check")
        .arg("-c")
        .arg(config_path)
        .current_dir(work_dir)
        .stdin(Stdio::null());
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    let output = cmd
        .output()
        .await
        .map_err(|e| AppError::CoreStartFailed(format!("could not run sing-box check: {e}")))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let detail = tail_str(stderr.trim(), 4000);
    for line in detail.lines().filter(|l| !l.trim().is_empty()) {
        logs.push_now("error", format!("[sing-box check] {}", line.trim()));
    }
    Err(AppError::CoreStartFailed(format!(
        "invalid config: {}",
        short_reason(&detail)
    )))
}

fn spawn_child(app: &AppHandle, ctx: &RunCtx) -> AppResult<Child> {
    let mut cmd = Command::new(&ctx.exe);
    cmd.arg("run")
        .arg("-c")
        .arg(&ctx.config_path)
        .arg("--disable-color")
        .current_dir(&ctx.work_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // Covers the graceful paths only; the job object below is what makes
        // the core die when this process is terminated without unwinding.
        .kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    let mut child = cmd
        .spawn()
        .map_err(|e| AppError::CoreStartFailed(format!("could not spawn sing-box: {e}")))?;
    assign_to_job(app, &child);
    if let Some(stdout) = child.stdout.take() {
        tauri::async_runtime::spawn(read_pipe(app.clone(), stdout));
    }
    if let Some(stderr) = child.stderr.take() {
        tauri::async_runtime::spawn(read_pipe(app.clone(), stderr));
    }
    Ok(child)
}

/// Put the freshly spawned core into the app-wide kill-on-close job so Windows
/// terminates it the moment this process goes away — Task Manager included.
/// Best-effort: losing the guarantee is worth logging, not worth refusing to
/// connect over.
fn assign_to_job(app: &AppHandle, child: &Child) {
    #[cfg(windows)]
    {
        let Some(raw) = child.raw_handle() else {
            return; // already reaped; nothing to protect
        };
        if let Err(e) = job::assign_current_job(raw) {
            app.state::<AppState>().logs.push_now(
                "warn",
                format!("[umbra] {e}; sing-box may outlive the app if it is force-killed"),
            );
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (app, child);
    }
}

async fn read_pipe<R: AsyncRead + Unpin + Send + 'static>(app: AppHandle, pipe: R) {
    let mut lines = BufReader::new(pipe).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if line.trim().is_empty() {
            continue;
        }
        app.state::<AppState>().logs.push(parse_log_line(&line));
    }
}

async fn graceful_kill(child: &mut Child) {
    let _ = child.start_kill();
    if tokio::time::timeout(KILL_GRACE, child.wait())
        .await
        .is_err()
    {
        let _ = child.kill().await;
    }
}

// ---------------------------------------------------------------------------
// Crash supervision
// ---------------------------------------------------------------------------

enum WaitOutcome {
    Stop,
    Exited(Option<i32>),
}

async fn wait_or_stop(shared: &CoreShared, child: &mut Child) -> WaitOutcome {
    tokio::select! {
        status = child.wait() => WaitOutcome::Exited(status.ok().and_then(|s| s.code())),
        _ = shared.stop_notify.notified() => WaitOutcome::Stop,
    }
}

/// Returns true when a stop was requested during the sleep.
async fn sleep_or_stop(shared: &CoreShared, dur: Duration) -> bool {
    tokio::select! {
        _ = tokio::time::sleep(dur) => false,
        _ = shared.stop_notify.notified() => true,
    }
}

/// Only the system proxy needs undoing after a crash: a dead core takes its
/// wintun adapter, its routes and its strict_route filters with it (see the
/// note on the tun inbound in `singbox::config`).
async fn cleanup_proxy_if_owned(app: &AppHandle) {
    let state = app.state::<AppState>();
    let owned = { state.settings.read().await.proxy_owned };
    if owned {
        if let Err(e) = system_proxy::disable_proxy(&state).await {
            eprintln!("[umbra] failed to restore system proxy: {e}");
        }
    }
}

async fn supervise(app: AppHandle, shared: Arc<CoreShared>, mut child: Child, ctx: RunCtx) {
    let mut attempt: u32 = 0;
    let mut healthy_since = Instant::now();
    loop {
        match wait_or_stop(&shared, &mut child).await {
            WaitOutcome::Stop => {
                graceful_kill(&mut child).await;
                // Normally stop() has already restored the proxy; this covers
                // the narrow race where a restart re-enabled it mid-stop.
                cleanup_proxy_if_owned(&app).await;
                return;
            }
            WaitOutcome::Exited(exit_code) => {
                if shared.stopping.load(Ordering::SeqCst) {
                    return;
                }
                if healthy_since.elapsed() >= HEALTHY_RESET {
                    attempt = 0;
                }
                {
                    let state = app.state::<AppState>();
                    state.logs.push_now(
                        "error",
                        format!(
                            "[umbra] sing-box exited unexpectedly (code {})",
                            exit_code.map_or_else(|| "unknown".into(), |c| c.to_string())
                        ),
                    );
                }
                cleanup_proxy_if_owned(&app).await;

                let mut code = exit_code;
                loop {
                    attempt += 1;
                    let will_restart = attempt <= MAX_RESTART_ATTEMPTS;
                    let _ = app.emit(
                        EV_CORE_CRASHED,
                        &CrashInfo {
                            code,
                            will_restart,
                            attempt,
                        },
                    );
                    if !will_restart {
                        give_up(&app).await;
                        return;
                    }
                    update_conn(&app, |c| {
                        c.status = ConnStatus::Connecting;
                        c.since_ms = None;
                    })
                    .await;
                    if sleep_or_stop(&shared, Duration::from_secs(backoff_secs(attempt))).await {
                        return;
                    }
                    app.state::<AppState>().logs.push_now(
                        "info",
                        format!("[umbra] restarting sing-box (attempt {attempt})"),
                    );
                    match respawn(&app, &shared, &ctx).await {
                        Ok(new_child) => {
                            child = new_child;
                            // A stop() that raced past respawn's own check must
                            // not see a stale "connected" snapshot after it has
                            // already emitted the final disconnected state.
                            if shared.stopping.load(Ordering::SeqCst) {
                                graceful_kill(&mut child).await;
                                cleanup_proxy_if_owned(&app).await;
                                return;
                            }
                            healthy_since = Instant::now();
                            app.state::<AppState>()
                                .logs
                                .push_now("info", "[umbra] sing-box restarted");
                            update_conn(&app, |c| {
                                c.status = ConnStatus::Connected;
                                c.since_ms = Some(now_ms());
                                c.error = None;
                            })
                            .await;
                            break;
                        }
                        Err(e) => {
                            if shared.stopping.load(Ordering::SeqCst) {
                                return;
                            }
                            app.state::<AppState>()
                                .logs
                                .push_now("error", format!("[umbra] restart failed: {e}"));
                            code = None;
                        }
                    }
                }
            }
        }
    }
}

async fn respawn(app: &AppHandle, shared: &CoreShared, ctx: &RunCtx) -> AppResult<Child> {
    // The old core took its api down with it; `watch_clash_api` flips this back
    // on by itself once the replacement starts answering.
    shared.clash_ready.store(false, Ordering::SeqCst);
    let mut child = spawn_child(app, ctx)?;
    // Same rule as the initial start: a live process is a successful restart,
    // whether or not its clash api has caught up yet.
    if let Err(e) = confirm_started(&mut child).await {
        graceful_kill(&mut child).await;
        return Err(e);
    }
    if shared.stopping.load(Ordering::SeqCst) {
        graceful_kill(&mut child).await;
        return Err(AppError::Internal("stop requested during restart".into()));
    }
    if ctx.mode == Mode::SystemProxy {
        let state = app.state::<AppState>();
        if let Err(e) = system_proxy::enable_proxy(&state, ctx.mixed_port).await {
            graceful_kill(&mut child).await;
            return Err(e);
        }
    }
    Ok(child)
}

async fn give_up(app: &AppHandle) {
    let state = app.state::<AppState>();
    state
        .logs
        .push_now("error", "[umbra] restart attempts exhausted; disconnected");
    let taken = { state.core.lock().await.take() };
    if let Some(handle) = taken {
        handle.traffic.abort();
        handle.readiness.abort();
    }
    update_conn(app, |c| {
        c.status = ConnStatus::Disconnected;
        c.server_id = None;
        c.server_name = None;
        c.since_ms = None;
        c.error = Some("sing-box crashed and could not be restarted".into());
    })
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_singbox_timestamped_line() {
        let line = "+0300 2026-07-27 12:00:00 INFO [tag] inbound started";
        let l = parse_log_line(line);
        assert_eq!(l.level, "info");
        assert_eq!(l.message, "[tag] inbound started");
        let expected = chrono::DateTime::parse_from_rfc3339("2026-07-27T12:00:00+03:00")
            .unwrap()
            .timestamp_millis();
        assert_eq!(l.ts, expected);
    }

    #[test]
    fn maps_fatal_and_warn_levels() {
        assert_eq!(
            parse_log_line("+0000 2026-01-01 00:00:00 FATAL boom").level,
            "error"
        );
        assert_eq!(
            parse_log_line("+0000 2026-01-01 00:00:00 WARN careful").level,
            "warn"
        );
        assert_eq!(
            parse_log_line("-0500 2026-01-01 00:00:00 DEBUG dbg").level,
            "debug"
        );
    }

    #[test]
    fn garbage_falls_back_to_info_with_raw_message() {
        let l = parse_log_line("something odd happened");
        assert_eq!(l.level, "info");
        assert_eq!(l.message, "something odd happened");
        assert!(l.ts > 0);
    }

    #[test]
    fn bad_offset_or_level_falls_back() {
        let l = parse_log_line("0300 2026-07-27 12:00:00 INFO x");
        assert_eq!(l.message, "0300 2026-07-27 12:00:00 INFO x");
        let l = parse_log_line("+0300 2026-07-27 12:00:00 NOTALEVEL x");
        assert_eq!(l.level, "info");
        assert_eq!(l.message, "+0300 2026-07-27 12:00:00 NOTALEVEL x");
    }

    #[test]
    fn backoff_sequence_is_1_3_9_capped() {
        assert_eq!(backoff_secs(1), 1);
        assert_eq!(backoff_secs(2), 3);
        assert_eq!(backoff_secs(3), 9);
        assert_eq!(backoff_secs(4), 9);
    }

    #[test]
    fn ring_caps_at_2000_keeping_newest() {
        let buf = LogBuffer::new();
        for i in 0..2100 {
            buf.push_now("info", format!("m{i}"));
        }
        let tail = buf.tail(3000);
        assert_eq!(tail.len(), 2000);
        assert_eq!(tail[0].message, "m100");
        assert_eq!(tail.last().unwrap().message, "m2099");
    }

    #[test]
    fn tail_returns_last_n_in_order() {
        let buf = LogBuffer::new();
        for i in 0..10 {
            buf.push_now("info", format!("m{i}"));
        }
        let tail = buf.tail(3);
        assert_eq!(
            tail.iter().map(|l| l.message.as_str()).collect::<Vec<_>>(),
            vec!["m7", "m8", "m9"]
        );
    }

    #[test]
    fn drain_pending_empties_but_keeps_ring() {
        let buf = LogBuffer::new();
        buf.push_now("info", "a");
        buf.push_now("warn", "b");
        let batch = buf.drain_pending();
        assert_eq!(batch.len(), 2);
        assert_eq!(batch[1].level, "warn");
        assert!(buf.drain_pending().is_empty());
        assert_eq!(buf.tail(10).len(), 2);
    }

    #[test]
    fn clear_wipes_ring_and_pending() {
        let buf = LogBuffer::new();
        buf.push_now("info", "a");
        buf.clear();
        assert!(buf.tail(10).is_empty());
        assert!(buf.drain_pending().is_empty());
    }

    #[test]
    fn tail_str_keeps_last_chars() {
        assert_eq!(tail_str("hello", 10), "hello");
        assert_eq!(tail_str("hello", 3), "llo");
        assert_eq!(tail_str("привет", 3), "вет");
    }

    // -----------------------------------------------------------------------
    // Readiness budget (bug: "failed to become ready" on a healthy 57-server
    // core, because 8s was a fixed deadline *and* a fatal one)
    // -----------------------------------------------------------------------

    #[test]
    fn ready_deadline_starts_far_above_the_old_8s() {
        assert_eq!(ready_deadline(0), READY_BASE);
        assert!(
            ready_deadline(0) >= Duration::from_secs(30),
            "a single-server core must still get a generous budget"
        );
    }

    #[test]
    fn ready_deadline_scales_with_outbound_count() {
        // the real-world report: one subscription, 57 outbounds + urltest
        assert_eq!(
            ready_deadline(57),
            READY_BASE + Duration::from_millis(250 * 57)
        );
        assert!(ready_deadline(57) > ready_deadline(10));
        assert!(ready_deadline(10) > ready_deadline(0));
    }

    #[test]
    fn ready_deadline_is_capped() {
        assert_eq!(ready_deadline(100_000), READY_MAX);
        assert_eq!(ready_deadline(usize::from(u16::MAX)), READY_MAX);
    }

    // -----------------------------------------------------------------------
    // User-facing error text (the toast used to dump 20 log lines)
    // -----------------------------------------------------------------------

    #[test]
    fn short_reason_takes_the_first_meaningful_line() {
        let wall =
            "\n\n  FATAL[0000] decode config at generated.json: bad json  \nnext line\nand another";
        assert_eq!(
            short_reason(wall),
            "FATAL[0000] decode config at generated.json: bad json"
        );
    }

    #[test]
    fn short_reason_caps_length_with_an_ellipsis() {
        let long = "x".repeat(500);
        let out = short_reason(&long);
        assert_eq!(out.chars().count(), MAX_REASON_CHARS);
        assert!(out.ends_with('…'));
        assert!(!out.contains('\n'));
    }

    #[test]
    fn short_reason_handles_empty_input() {
        assert_eq!(short_reason("   \n\n "), "no details available");
        assert_eq!(short_reason(""), "no details available");
    }

    #[test]
    fn short_reason_is_char_safe_on_cyrillic() {
        let long = "п".repeat(400);
        let out = short_reason(&long);
        assert_eq!(out.chars().count(), MAX_REASON_CHARS);
    }

    // -----------------------------------------------------------------------
    // Start-up confirmation replaces the fatal clash-api wait
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn confirm_started_rejects_a_core_that_exits_at_once() {
        let mut cmd = Command::new(exit_helper());
        cmd.args(exit_args(3)).stdin(Stdio::null());
        #[cfg(windows)]
        cmd.creation_flags(CREATE_NO_WINDOW);
        let mut child = cmd.spawn().expect("spawn helper");
        let err = confirm_started(&mut child)
            .await
            .expect_err("an immediate exit must fail the connect");
        assert_eq!(err.code(), "CORE_START_FAILED");
        let msg = err.to_string();
        assert!(msg.contains("exited immediately"), "{msg}");
        // the toast stays one line
        assert!(!msg.contains('\n'), "{msg}");
    }

    #[tokio::test]
    async fn confirm_started_accepts_a_process_that_stays_up() {
        let mut cmd = Command::new(sleep_helper());
        cmd.args(sleep_args())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(windows)]
        cmd.creation_flags(CREATE_NO_WINDOW);
        let mut child = cmd.spawn().expect("spawn helper");
        let started = std::time::Instant::now();
        let result = confirm_started(&mut child).await;
        let _ = child.kill().await;
        assert!(result.is_ok(), "{result:?}");
        // the connect must not block on the clash api any more, only on the
        // short "did it die on the spot" window
        assert!(
            started.elapsed() < STARTUP_CONFIRM + Duration::from_secs(2),
            "confirm_started blocked for {:?}",
            started.elapsed()
        );
    }

    #[cfg(windows)]
    fn exit_helper() -> &'static str {
        "cmd.exe"
    }
    #[cfg(windows)]
    fn exit_args(code: i32) -> Vec<String> {
        vec!["/C".into(), format!("exit {code}")]
    }
    #[cfg(windows)]
    fn sleep_helper() -> &'static str {
        "cmd.exe"
    }
    #[cfg(windows)]
    fn sleep_args() -> Vec<String> {
        // ~9s of doing nothing; killed by the test as soon as it has served
        // its purpose. `pause` would not work: stdin is null, so it returns at
        // once on EOF.
        vec![
            "/C".into(),
            "ping".into(),
            "-n".into(),
            "10".into(),
            "127.0.0.1".into(),
        ]
    }

    #[cfg(not(windows))]
    fn exit_helper() -> &'static str {
        "sh"
    }
    #[cfg(not(windows))]
    fn exit_args(code: i32) -> Vec<String> {
        vec!["-c".into(), format!("exit {code}")]
    }
    #[cfg(not(windows))]
    fn sleep_helper() -> &'static str {
        "sleep"
    }
    #[cfg(not(windows))]
    fn sleep_args() -> Vec<String> {
        vec!["30".into()]
    }
}
