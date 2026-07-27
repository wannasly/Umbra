//! Clash API client for the running sing-box core: readiness polling,
//! selector switching, url delay tests and the /traffic websocket.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use tauri::{AppHandle, Emitter, Manager};
use tokio_tungstenite::tungstenite::Message;

use crate::error::{AppError, AppResult};
use crate::events::{TrafficStats, EV_TRAFFIC};
use crate::singbox::process::CoreShared;
use crate::state::AppState;
use crate::storage;

const PROBE_TIMEOUT: Duration = Duration::from_secs(2);
const TRAFFIC_RECONNECT_DELAY: Duration = Duration::from_secs(2);
/// How often the per-server byte counters reach profiles.json. The frames
/// arrive once a second and the file holds every server, so writing per frame
/// would mean a disk write per second for a counter nobody reads that often.
const TOTALS_FLUSH_INTERVAL: Duration = Duration::from_secs(15);

/// The system proxy may point at us; the clash API client must bypass it.
fn client(timeout: Duration) -> AppResult<reqwest::Client> {
    reqwest::Client::builder()
        .no_proxy()
        .timeout(timeout)
        .build()
        .map_err(|e| AppError::Internal(format!("http client: {e}")))
}

fn base_url(port: u16) -> String {
    format!("http://127.0.0.1:{port}")
}

/// One GET /version: true when the core's clash api answers right now.
///
/// Every failure mode — connection refused while the core is still wiring up
/// 57 outbounds, a timeout, a non-2xx — is just "not yet", so the caller can
/// keep polling. Nothing here is fatal: the api carries stats and selector
/// switching, never user traffic.
pub async fn probe(port: u16, secret: &str) -> bool {
    let Ok(client) = client(PROBE_TIMEOUT) else {
        return false;
    };
    match client
        .get(format!("{}/version", base_url(port)))
        .bearer_auth(secret)
        .send()
        .await
    {
        Ok(resp) => resp.status().is_success(),
        Err(_) => false,
    }
}

/// PUT /proxies/proxy with `{"name": tag}` — "proxy" is the selector group,
/// the tag goes in the body only.
pub async fn switch_selector(port: u16, secret: &str, tag: &str) -> AppResult<()> {
    let client = client(Duration::from_secs(5))?;
    let resp = client
        .put(format!("{}/proxies/proxy", base_url(port)))
        .bearer_auth(secret)
        .json(&serde_json::json!({ "name": tag }))
        .send()
        .await?;
    if resp.status().is_success() {
        Ok(())
    } else {
        Err(AppError::Internal(format!(
            "selector switch failed: {}",
            resp.status()
        )))
    }
}

fn delay_path(port: u16, tag: &str) -> String {
    format!(
        "{}/proxies/{}/delay",
        base_url(port),
        utf8_percent_encode(tag, NON_ALPHANUMERIC)
    )
}

/// GET /proxies/&lt;tag&gt;/delay?timeout=5000&url=... -> latency in ms.
pub async fn delay(port: u16, secret: &str, tag: &str, test_url: &str) -> AppResult<u32> {
    let client = client(Duration::from_secs(10))?;
    let resp = client
        .get(delay_path(port, tag))
        .query(&[("timeout", "5000"), ("url", test_url)])
        .bearer_auth(secret)
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(AppError::Network(format!(
            "url test failed: {}",
            resp.status()
        )));
    }
    let body: serde_json::Value = resp.json().await?;
    body.get("delay")
        .and_then(|d| d.as_u64())
        .map(|d| d as u32)
        .ok_or_else(|| AppError::Internal("malformed delay response".into()))
}

/// Stream ws://127.0.0.1:&lt;port&gt;/traffic and emit `traffic://stats` per frame
/// (the core sends one frame per second); totals accumulate across reconnects.
/// Retries every 2s for the whole core session: crash-restart backoff keeps
/// the core down for up to tens of seconds, so the loop never gives up on its
/// own — it ends via the stopping flag or the JoinHandle abort in
/// stop()/give_up().
pub async fn traffic_loop(app: AppHandle, shared: Arc<CoreShared>, port: u16, secret: String) {
    let url = format!("ws://127.0.0.1:{port}/traffic?token={secret}");
    let mut up_total = 0.0f64;
    let mut down_total = 0.0f64;
    let mut totals = TotalsAccumulator::default();
    while !shared.stopping.load(Ordering::SeqCst) {
        if let Ok((mut ws, _)) = tokio_tungstenite::connect_async(url.as_str()).await {
            while let Some(frame) = ws.next().await {
                match frame {
                    Ok(Message::Text(text)) => {
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(text.as_str()) {
                            let up = v.get("up").and_then(|x| x.as_f64()).unwrap_or(0.0);
                            let down = v.get("down").and_then(|x| x.as_f64()).unwrap_or(0.0);
                            up_total += up;
                            down_total += down;
                            let _ = app.emit(
                                EV_TRAFFIC,
                                &TrafficStats {
                                    up_bps: up,
                                    down_bps: down,
                                    up_total,
                                    down_total,
                                },
                            );
                            // Each frame is one second's worth of bytes, so the
                            // deltas belong to whichever server the tunnel is
                            // running against right now — including after a
                            // live selector switch.
                            let active =
                                { app.state::<AppState>().conn.read().await.server_id.clone() };
                            for batch in totals.record(active.as_deref(), up, down) {
                                persist_totals(&app, &batch).await;
                            }
                        }
                    }
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
        }
        // The socket dropped (core restart, api restart): bank what we have
        // rather than carrying it across a gap we cannot time.
        for batch in totals.drain() {
            persist_totals(&app, &batch).await;
        }
        if shared.stopping.load(Ordering::SeqCst) {
            break;
        }
        tokio::time::sleep(TRAFFIC_RECONNECT_DELAY).await;
    }
}

/// A per-server chunk of traffic ready to be written to disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TotalsBatch {
    pub server_id: String,
    pub up: u64,
    pub down: u64,
}

/// Buffers the per-second deltas so the profile store is written on a timer
/// rather than on every frame, and so a mid-session server switch never
/// misattributes the bytes that came before it.
#[derive(Debug)]
struct TotalsAccumulator {
    server_id: Option<String>,
    up: f64,
    down: f64,
    last_flush: std::time::Instant,
}

impl Default for TotalsAccumulator {
    fn default() -> Self {
        Self {
            server_id: None,
            up: 0.0,
            down: 0.0,
            last_flush: std::time::Instant::now(),
        }
    }
}

impl TotalsAccumulator {
    /// Add one frame. Returns the batches that became due — at most one, but a
    /// `Vec` so callers need no special case for "nothing to write yet".
    fn record(&mut self, active: Option<&str>, up: f64, down: f64) -> Vec<TotalsBatch> {
        let switched = self.server_id.as_deref() != active;
        let mut out = Vec::new();
        if switched {
            out.extend(self.take());
            self.server_id = active.map(str::to_string);
        }
        if self.server_id.is_some() {
            self.up += up.max(0.0);
            self.down += down.max(0.0);
        }
        if self.last_flush.elapsed() >= TOTALS_FLUSH_INTERVAL {
            out.extend(self.take());
        }
        out
    }

    /// Flush whatever is buffered, whether or not the timer is due.
    fn drain(&mut self) -> Vec<TotalsBatch> {
        self.take().into_iter().collect()
    }

    /// Whole bytes only; the sub-byte remainder stays buffered so a long
    /// session does not shed a byte per flush.
    fn take(&mut self) -> Option<TotalsBatch> {
        self.last_flush = std::time::Instant::now();
        let up = self.up.floor();
        let down = self.down.floor();
        self.up -= up;
        self.down -= down;
        let id = self.server_id.clone()?;
        if up <= 0.0 && down <= 0.0 {
            return None;
        }
        Some(TotalsBatch {
            server_id: id,
            up: up as u64,
            down: down as u64,
        })
    }
}

async fn persist_totals(app: &AppHandle, batch: &TotalsBatch) {
    let state = app.state::<AppState>();
    let mut profiles = state.profiles.write().await;
    let Some(server) = profiles.find_server_mut(&batch.server_id) else {
        return; // deleted mid-session; the bytes have nowhere to go
    };
    server.total_up = server.total_up.saturating_add(batch.up);
    server.total_down = server.total_down.saturating_add(batch.down);
    if let Err(e) = storage::save_profiles(&state.data_dir, &profiles) {
        eprintln!("[umbra] failed to persist per-server traffic totals: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delay_path_urlencodes_tag() {
        assert_eq!(
            delay_path(9095, "My Server (2)"),
            "http://127.0.0.1:9095/proxies/My%20Server%20%282%29/delay"
        );
    }

    #[test]
    fn delay_path_passes_plain_tags_through() {
        assert_eq!(
            delay_path(9100, "server1"),
            "http://127.0.0.1:9100/proxies/server1/delay"
        );
    }

    /// A refused connection is "not ready yet", not an error to propagate:
    /// the watcher polls this until the core catches up.
    #[tokio::test]
    async fn probe_is_false_on_a_closed_port() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        assert!(!probe(port, "s3cret").await);
    }

    // -- per-server traffic totals ----------------------------------------

    /// Frames arriving once a second must not mean a profiles.json write once
    /// a second: nothing is due until the interval elapses.
    #[test]
    fn frames_are_buffered_until_the_flush_interval() {
        let mut acc = TotalsAccumulator::default();
        for _ in 0..10 {
            assert!(acc.record(Some("srv-1"), 1_000.0, 20_000.0).is_empty());
        }
        let batch = acc.drain();
        assert_eq!(
            batch,
            vec![TotalsBatch {
                server_id: "srv-1".into(),
                up: 10_000,
                down: 200_000,
            }]
        );
    }

    /// Hot-switching the selector mid-session moves `conn.server_id`; the bytes
    /// that arrived before the switch belong to the old server.
    #[test]
    fn switching_servers_banks_the_previous_servers_bytes() {
        let mut acc = TotalsAccumulator::default();
        acc.record(Some("srv-1"), 100.0, 900.0);
        let flushed = acc.record(Some("srv-2"), 5.0, 5.0);
        assert_eq!(
            flushed,
            vec![TotalsBatch {
                server_id: "srv-1".into(),
                up: 100,
                down: 900,
            }]
        );
        // and the frame that caused the switch counts towards the new server
        assert_eq!(
            acc.drain(),
            vec![TotalsBatch {
                server_id: "srv-2".into(),
                up: 5,
                down: 5,
            }]
        );
    }

    /// Traffic with no active server (a frame racing disconnect) is dropped
    /// rather than attributed to whoever ran last.
    #[test]
    fn traffic_without_an_active_server_is_discarded() {
        let mut acc = TotalsAccumulator::default();
        assert!(acc.record(None, 500.0, 500.0).is_empty());
        assert!(acc.drain().is_empty());
    }

    #[test]
    fn an_idle_tunnel_writes_nothing() {
        let mut acc = TotalsAccumulator::default();
        acc.record(Some("srv-1"), 0.0, 0.0);
        assert!(
            acc.drain().is_empty(),
            "zero bytes is not worth a disk write"
        );
    }

    /// The core reports fractional bytes/s; truncating each flush would lose up
    /// to a byte every 15s. The remainder has to carry over.
    #[test]
    fn fractional_bytes_carry_over_between_flushes() {
        let mut acc = TotalsAccumulator::default();
        acc.record(Some("srv-1"), 0.5, 0.0);
        assert!(acc.drain().is_empty()); // 0.5 -> nothing whole yet
        acc.record(Some("srv-1"), 0.7, 0.0);
        assert_eq!(
            acc.drain(),
            vec![TotalsBatch {
                server_id: "srv-1".into(),
                up: 1,
                down: 0,
            }]
        );
    }

    /// A negative delta is nonsense the api has no business sending, but it
    /// must never subtract from a cumulative counter.
    #[test]
    fn negative_deltas_are_ignored() {
        let mut acc = TotalsAccumulator::default();
        acc.record(Some("srv-1"), -5_000.0, 100.0);
        assert_eq!(
            acc.drain(),
            vec![TotalsBatch {
                server_id: "srv-1".into(),
                up: 0,
                down: 100,
            }]
        );
    }
}
