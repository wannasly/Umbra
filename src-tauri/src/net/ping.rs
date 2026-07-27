//! TCP connect latency measurement, independent of the running core.

use std::time::Duration;

use futures_util::stream::{self, StreamExt};
use tauri::{AppHandle, Emitter, Manager};
use tokio::net::{lookup_host, TcpStream};
use tokio::time::{timeout, Instant};

use crate::events::{PingResult, EV_PING_RESULT};
use crate::state::AppState;
use crate::storage;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const ATTEMPTS: u32 = 2;
const CONCURRENCY: usize = 8;

pub struct PingTarget {
    pub id: String,
    pub host: String,
    pub port: u16,
}

/// Best (minimum) TCP connect time over two attempts; None when the host does
/// not resolve or no attempt connects within 3s.
pub async fn tcp_ping(host: &str, port: u16) -> Option<u32> {
    let addr = lookup_host((host, port)).await.ok()?.next()?;
    let mut best: Option<u32> = None;
    for _ in 0..ATTEMPTS {
        let started = Instant::now();
        if let Ok(Ok(_stream)) = timeout(CONNECT_TIMEOUT, TcpStream::connect(addr)).await {
            let ms = started.elapsed().as_millis().min(u32::MAX as u128) as u32;
            best = Some(best.map_or(ms, |b| b.min(ms)));
        }
    }
    best
}

/// Ping targets with concurrency 8, emitting `ping://result` per completion,
/// then persist all `lastPingMs` values with a single profiles save.
pub async fn ping_many(app: AppHandle, targets: Vec<PingTarget>) {
    if targets.is_empty() {
        return;
    }
    let mut results = Vec::with_capacity(targets.len());
    let mut stream = stream::iter(targets.into_iter().map(|t| async move {
        let latency = tcp_ping(&t.host, t.port).await;
        (t.id, latency)
    }))
    .buffer_unordered(CONCURRENCY);
    while let Some((id, latency)) = stream.next().await {
        let _ = app.emit(
            EV_PING_RESULT,
            &PingResult {
                server_id: id.clone(),
                latency_ms: latency,
            },
        );
        results.push((id, latency));
    }
    let state = app.state::<AppState>();
    let mut profiles = state.profiles.write().await;
    for (id, latency) in &results {
        if let Some(server) = profiles.find_server_mut(id) {
            server.last_ping_ms = *latency;
        }
    }
    if let Err(e) = storage::save_profiles(&state.data_dir, &profiles) {
        eprintln!("[umbra] failed to persist ping results: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn ping_local_listener_succeeds() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            loop {
                if listener.accept().await.is_err() {
                    break;
                }
            }
        });
        let ms = tcp_ping("127.0.0.1", port).await;
        assert!(ms.is_some());
    }

    /// Binding :0, dropping it and probing the freed port races every other
    /// test that binds :0 — the OS is free to hand the just-released port
    /// straight back out. Take one from below the Windows dynamic range
    /// (49152+) instead, so nothing ephemeral can claim it mid-test.
    async fn closed_port() -> u16 {
        for port in 1024..1200u16 {
            if let Ok(listener) = tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
                drop(listener);
                return port;
            }
        }
        panic!("no free non-ephemeral port to probe");
    }

    #[tokio::test]
    async fn ping_closed_port_fails() {
        let port = closed_port().await;
        assert!(tcp_ping("127.0.0.1", port).await.is_none());
    }
}
