//! Server and subscription management commands.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use chrono::{DateTime, Utc};
use tauri::{AppHandle, Emitter, Manager};

use crate::error::{AppError, AppResult};
use crate::events::{SubUpdated, EV_SUB_UPDATED};
use crate::models::{ConnStatus, ConnectionState, ImportResult, ServersList, Subscription};
use crate::parser;
use crate::singbox::clash_api;
use crate::state::{update_conn, AppState};
use crate::storage;
use crate::subscription::{self, FetchedSubscription};

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

// ---------------------------------------------------------------------------
// Post-mutation reconciliation
// ---------------------------------------------------------------------------

/// Decide what a profile mutation must do to the ids pointing at servers that
/// no longer exist. Pure so the rules are testable without a running app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Reconciliation {
    /// The persisted `settings.selected_server_id` points at a deleted server.
    clear_selection: bool,
    /// A live tunnel is running against a server that is no longer in the list.
    active_server_gone: bool,
}

fn reconcile(
    alive_ids: &HashSet<String>,
    selected_id: Option<&str>,
    conn_status: ConnStatus,
    conn_server_id: Option<&str>,
) -> Reconciliation {
    Reconciliation {
        clear_selection: selected_id.is_some_and(|id| !alive_ids.contains(id)),
        active_server_gone: conn_status != ConnStatus::Disconnected
            && conn_server_id.is_some_and(|id| !alive_ids.contains(id)),
    }
}

/// Apply `reconcile` after a removal or a subscription refresh.
///
/// The running core is deliberately left alone: deleting a list entry must not
/// yank the user offline mid-session. What must not survive is the *desync* the
/// user hit — a dashboard showing "connected", a live timer and live traffic
/// next to "no server selected". `conn.server_name` was snapshotted at connect
/// time, so the state can still name the tunnel it is actually running, and the
/// stale selection is cleared so the next connect cannot target a ghost id.
async fn reconcile_profiles(app: &AppHandle, state: &AppState) {
    let alive: HashSet<String> = {
        let profiles = state.profiles.read().await;
        profiles.all_servers().map(|s| s.id.clone()).collect()
    };
    let action = {
        let settings = state.settings.read().await;
        let conn = state.conn.read().await;
        reconcile(
            &alive,
            settings.selected_server_id.as_deref(),
            conn.status,
            conn.server_id.as_deref(),
        )
    };
    if !action.clear_selection && !action.active_server_gone {
        return;
    }
    if action.clear_selection {
        let mut settings = state.settings.write().await;
        settings.selected_server_id = None;
        if let Err(e) = storage::save_settings(&state.data_dir, &settings) {
            eprintln!("[umbra] failed to clear the deleted server selection: {e}");
        }
    }
    if action.active_server_gone {
        state.logs.push_now(
            "warn",
            "[umbra] the active server was deleted from profiles; the tunnel keeps running \
             against it until you disconnect",
        );
    }
    // Re-emit so the dashboard and the tray pick up the cleared selection and
    // fall back to the snapshotted name of the server that is now gone.
    update_conn(app, |_| {}).await;
}

/// Subscription request identity taken from settings. Panels that enforce a
/// device limit (Remnawave: `X-Hwid-Not-Supported`) serve a placeholder server
/// until these headers arrive.
async fn fetch_params(
    state: &tauri::State<'_, AppState>,
) -> (String, Option<subscription::DeviceIdentity>) {
    let settings = state.settings.read().await;
    // An empty hwid would send `x-hwid:` with no value, which panels read as a
    // malformed device rather than as an absent one; omit the headers instead.
    let identity = settings
        .send_hwid
        .then(|| subscription::DeviceIdentity {
            hwid: settings.hwid.clone(),
            os: crate::hwid::device_os(),
            os_version: crate::hwid::os_version(),
            model: crate::hwid::device_model(),
        })
        .filter(|id| !id.hwid.trim().is_empty());
    (settings.sub_user_agent.clone(), identity)
}

#[tauri::command]
pub async fn import_share_links(
    state: tauri::State<'_, AppState>,
    text: String,
) -> AppResult<ImportResult> {
    let (parsed, errors) = parser::parse_links(&text);
    let mut profiles = state.profiles.write().await;
    let existing: HashSet<String> = profiles.manual.iter().map(|s| s.raw.clone()).collect();
    let mut added = 0;
    for entry in parsed {
        if existing.contains(&entry.raw) {
            continue;
        }
        profiles.manual.push(entry);
        added += 1;
    }
    if added > 0 {
        storage::save_profiles(&state.data_dir, &profiles)?;
    }
    Ok(ImportResult { added, errors })
}

#[tauri::command]
pub async fn list_servers(state: tauri::State<'_, AppState>) -> AppResult<ServersList> {
    let profiles = state.profiles.read().await;
    Ok(ServersList {
        manual: profiles.manual.clone(),
        subscriptions: profiles.subscriptions.clone(),
    })
}

#[tauri::command]
pub async fn remove_server(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    id: String,
) -> AppResult<()> {
    {
        let mut profiles = state.profiles.write().await;
        let before = profiles.manual.len();
        profiles.manual.retain(|s| s.id != id);
        if profiles.manual.len() == before {
            return Err(AppError::NotFound(format!("server {id}")));
        }
        storage::save_profiles(&state.data_dir, &profiles)?;
    }
    reconcile_profiles(&app, &state).await;
    Ok(())
}

#[tauri::command]
pub async fn select_server(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    id: String,
) -> AppResult<ConnectionState> {
    let name = {
        let profiles = state.profiles.read().await;
        profiles
            .find_server(&id)
            .map(|s| s.name.clone())
            .ok_or_else(|| AppError::NotFound(format!("server {id}")))?
    };
    {
        let mut settings = state.settings.write().await;
        settings.selected_server_id = Some(id.clone());
        storage::save_settings(&state.data_dir, &settings)?;
    }
    // While connected: switch the running selector live, no core restart.
    let connected = { state.conn.read().await.status == ConnStatus::Connected };
    if connected {
        let coords = {
            let core = state.core.lock().await;
            core.as_ref().map(|h| {
                (
                    h.clash_port,
                    h.clash_secret.clone(),
                    h.tag_by_server_id.get(&id).cloned(),
                    h.clash_ready(),
                )
            })
        };
        if let Some((port, secret, tag, ready)) = coords {
            let tag = tag.ok_or_else(|| {
                AppError::NotFound(format!(
                    "server {id} is not in the running config; reconnect to apply"
                ))
            })?;
            // Hot switching is the one feature that genuinely needs the clash
            // api. When it has not come up yet the tunnel is still fine — say
            // so instead of pretending the switch happened.
            if !ready {
                return Err(AppError::Internal(
                    "the core's control api is not up yet; reconnect to switch server".into(),
                ));
            }
            clash_api::switch_selector(port, &secret, &tag).await?;
            return Ok(update_conn(&app, |c| {
                c.server_id = Some(id.clone());
                c.server_name = Some(name.clone());
            })
            .await);
        }
    }
    Ok(state.conn.read().await.clone())
}

// ---------------------------------------------------------------------------
// Naming
// ---------------------------------------------------------------------------

fn url_host(url: &str) -> Option<String> {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
}

/// Name a subscription the user did not name.
///
/// Order matters: the panel's `Profile-Title` is the only field meant for
/// humans. `Content-Disposition` carries a *file* name, which on the panels our
/// users are on is an account id ("account-123") — recognisable to
/// nobody. The host at least says which provider it is.
fn derive_name(title: Option<&str>, filename: Option<&str>, url: &str) -> String {
    title
        .or(filename)
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(str::to_string)
        .or_else(|| url_host(url))
        .unwrap_or_else(|| "Subscription".into())
}

/// Whether a refresh may replace the stored name with the panel's fresh title.
///
/// Only names we picked ourselves are up for grabs — a name the user typed is
/// never overwritten. That makes the fix retroactive: an existing card still
/// showing the Content-Disposition account id gets the real title on its next
/// update, without anyone having to delete and re-add the subscription.
fn may_adopt_title(
    stored: &str,
    previous_title: Option<&str>,
    filename: Option<&str>,
    host: Option<&str>,
) -> bool {
    let stored = stored.trim();
    stored.is_empty()
        || previous_title == Some(stored)
        || filename == Some(stored)
        || host == Some(stored)
        || stored == "Subscription"
}

/// Reorder subscriptions to match `ids`.
///
/// Stable and total: ids the caller did not mention keep their relative order
/// after the ones it did, so a list that grew in another window (or an id that
/// went stale between render and click) can never drop an entry.
fn apply_order(subs: &mut [Subscription], ids: &[String]) {
    let rank: HashMap<&str, usize> = ids
        .iter()
        .enumerate()
        .map(|(i, id)| (id.as_str(), i))
        .collect();
    subs.sort_by_key(|s| rank.get(s.id.as_str()).copied().unwrap_or(usize::MAX));
}

fn ensure_has_servers(fetched: &FetchedSubscription) -> AppResult<()> {
    if fetched.servers.is_empty() {
        let detail = fetched
            .errors
            .first()
            .map(|e| format!(": {e}"))
            .unwrap_or_default();
        return Err(AppError::Parse(format!(
            "subscription contains no usable links{detail}"
        )));
    }
    Ok(())
}

#[tauri::command]
pub async fn add_subscription(
    state: tauri::State<'_, AppState>,
    url: String,
    name: Option<String>,
) -> AppResult<Subscription> {
    let (ua, identity) = fetch_params(&state).await;
    let fetched = subscription::fetch_subscription(&url, &ua, identity.as_ref()).await?;
    ensure_has_servers(&fetched)?;

    let name = name
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| {
            derive_name(fetched.title.as_deref(), fetched.filename.as_deref(), &url)
        });

    let sub = Subscription {
        id: uuid::Uuid::new_v4().to_string(),
        name,
        url,
        updated_at: Some(now_iso()),
        quota: fetched.quota,
        // The panel's own refresh cadence is a better default than "off".
        auto_update_hours: fetched.update_interval_hours.unwrap_or(0),
        support_url: fetched.support_url,
        web_page_url: fetched.web_page_url,
        panel_title: fetched.title,
        servers: fetched.servers,
    };

    let mut profiles = state.profiles.write().await;
    profiles.subscriptions.push(sub.clone());
    storage::save_profiles(&state.data_dir, &profiles)?;
    Ok(sub)
}

#[tauri::command]
pub async fn update_subscription(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    id: String,
) -> AppResult<Subscription> {
    let url = {
        let profiles = state.profiles.read().await;
        profiles
            .subscriptions
            .iter()
            .find(|s| s.id == id)
            .map(|s| s.url.clone())
            .ok_or_else(|| AppError::NotFound(format!("subscription {id}")))?
    };

    let (ua, identity) = fetch_params(&state).await;
    let fetched = subscription::fetch_subscription(&url, &ua, identity.as_ref()).await?;
    ensure_has_servers(&fetched)?;

    let (result, added, removed) = {
        let mut profiles = state.profiles.write().await;
        let sub = profiles
            .subscriptions
            .iter_mut()
            .find(|s| s.id == id)
            .ok_or_else(|| AppError::NotFound(format!("subscription {id}")))?;
        let (merged, added, removed) = subscription::merge_servers(&sub.servers, fetched.servers);
        sub.servers = merged;
        sub.updated_at = Some(now_iso());
        if fetched.quota.is_some() {
            sub.quota = fetched.quota;
        }
        if let Some(title) = fetched.title.as_deref() {
            if may_adopt_title(
                &sub.name,
                sub.panel_title.as_deref(),
                fetched.filename.as_deref(),
                url_host(&sub.url).as_deref(),
            ) {
                sub.name = title.to_string();
            }
            sub.panel_title = Some(title.to_string());
        }
        if fetched.support_url.is_some() {
            sub.support_url = fetched.support_url.clone();
        }
        if fetched.web_page_url.is_some() {
            sub.web_page_url = fetched.web_page_url.clone();
        }
        let result = sub.clone();
        storage::save_profiles(&state.data_dir, &profiles)?;
        (result, added, removed)
    };

    // A refresh can retire servers just like a delete can: the panel dropped a
    // node, the merge dropped its id.
    if removed > 0 {
        reconcile_profiles(&app, &state).await;
    }

    if let Err(e) = app.emit(
        EV_SUB_UPDATED,
        SubUpdated {
            id: id.clone(),
            added,
            removed,
        },
    ) {
        eprintln!("[umbra] failed to emit {EV_SUB_UPDATED}: {e}");
    }
    Ok(result)
}

#[tauri::command]
pub async fn remove_subscription(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    id: String,
) -> AppResult<()> {
    {
        let mut profiles = state.profiles.write().await;
        let before = profiles.subscriptions.len();
        profiles.subscriptions.retain(|s| s.id != id);
        if profiles.subscriptions.len() == before {
            return Err(AppError::NotFound(format!("subscription {id}")));
        }
        storage::save_profiles(&state.data_dir, &profiles)?;
    }
    // Deleting the subscription you are connected through used to leave the
    // dashboard claiming "connected" next to "no server selected".
    reconcile_profiles(&app, &state).await;
    Ok(())
}

/// Star / unstar a server. Persisted on the entry itself so it survives a
/// subscription refresh (`merge_servers` carries it over).
#[tauri::command]
pub async fn set_server_favorite(
    state: tauri::State<'_, AppState>,
    id: String,
    favorite: bool,
) -> AppResult<()> {
    let mut profiles = state.profiles.write().await;
    let server = profiles
        .find_server_mut(&id)
        .ok_or_else(|| AppError::NotFound(format!("server {id}")))?;
    if server.favorite == favorite {
        return Ok(());
    }
    server.favorite = favorite;
    storage::save_profiles(&state.data_dir, &profiles)?;
    Ok(())
}

/// Persist the order the user dragged the subscription groups into. The stored
/// vector *is* the order — `list_servers` hands it back as delivered.
#[tauri::command]
pub async fn reorder_subscriptions(
    state: tauri::State<'_, AppState>,
    ids: Vec<String>,
) -> AppResult<()> {
    let mut profiles = state.profiles.write().await;
    apply_order(&mut profiles.subscriptions, &ids);
    storage::save_profiles(&state.data_dir, &profiles)?;
    Ok(())
}

#[tauri::command]
pub async fn rename_subscription(
    state: tauri::State<'_, AppState>,
    id: String,
    name: String,
) -> AppResult<Subscription> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::Parse("subscription name cannot be empty".into()));
    }
    let mut profiles = state.profiles.write().await;
    let sub = profiles
        .subscriptions
        .iter_mut()
        .find(|s| s.id == id)
        .ok_or_else(|| AppError::NotFound(format!("subscription {id}")))?;
    sub.name = name;
    // Forget the panel title we adopted: from here on the name is the user's,
    // and the next refresh must leave it alone.
    sub.panel_title = None;
    let result = sub.clone();
    storage::save_profiles(&state.data_dir, &profiles)?;
    Ok(result)
}

#[tauri::command]
pub async fn set_subscription_auto_update(
    state: tauri::State<'_, AppState>,
    id: String,
    hours: u32,
) -> AppResult<()> {
    let mut profiles = state.profiles.write().await;
    let sub = profiles
        .subscriptions
        .iter_mut()
        .find(|s| s.id == id)
        .ok_or_else(|| AppError::NotFound(format!("subscription {id}")))?;
    sub.auto_update_hours = hours;
    storage::save_profiles(&state.data_dir, &profiles)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Auto-update scheduler
// ---------------------------------------------------------------------------

/// How often the scheduler wakes up. Every interval the UI offers is a whole
/// number of hours, so a quarter-hour tick keeps all of them accurate to
/// within 15 minutes and costs nothing on the ticks where nothing is due.
const AUTO_UPDATE_TICK: Duration = Duration::from_secs(15 * 60);

/// Delay before the first sweep. Startup is already busy — proxy recovery, the
/// bundled core, and possibly an auto-connect that has not brought the tunnel
/// up yet — and a fetch that runs before it would fail for no reason.
const AUTO_UPDATE_STARTUP_DELAY: Duration = Duration::from_secs(30);

/// Whether a subscription's refresh is overdue at `now`. Pure, so the schedule
/// is testable without a clock or a running app.
///
/// A missing `updated_at` is what a store written before the field existed
/// looks like; an unparseable one means something outside this app wrote it.
/// Neither is evidence the servers are current, so both count as due: one
/// refresh stamps an honest timestamp and the interval takes over from there.
fn due_for_refresh(auto_update_hours: u32, updated_at: Option<&str>, now: DateTime<Utc>) -> bool {
    if auto_update_hours == 0 {
        return false;
    }
    let Some(last) = updated_at.and_then(|s| DateTime::parse_from_rfc3339(s).ok()) else {
        return true;
    };
    // Whole-hour truncation matches the whole-hour intervals on offer, and a
    // clock that jumped backwards gives a negative span that simply waits
    // rather than refreshing on every tick.
    now.signed_duration_since(last.with_timezone(&Utc))
        .num_hours()
        >= i64::from(auto_update_hours)
}

/// Refresh every subscription whose interval has elapsed.
///
/// Goes through `update_subscription`, so an automatic refresh is the same
/// operation as the button: same merge, same `reconcile_profiles` when the
/// panel retired the server the user is connected through, same
/// `sub://updated` for the UI to reload on.
async fn refresh_due_subscriptions(app: &AppHandle, failing: &mut HashSet<String>) {
    let now = Utc::now();
    // The read guard is dropped before any fetch: `update_subscription` takes
    // the write lock itself, and a refresh can sit on the network for seconds.
    let due: Vec<String> = {
        let state = app.state::<AppState>();
        let profiles = state.profiles.read().await;
        failing.retain(|id| profiles.subscriptions.iter().any(|s| &s.id == id));
        profiles
            .subscriptions
            .iter()
            .filter(|s| due_for_refresh(s.auto_update_hours, s.updated_at.as_deref(), now))
            .map(|s| s.id.clone())
            .collect()
    };
    for id in due {
        let state = app.state::<AppState>();
        match update_subscription(app.clone(), state, id.clone()).await {
            Ok(sub) => {
                if failing.remove(&id) {
                    app.state::<AppState>().logs.push_now(
                        "info",
                        format!("[umbra] auto-update recovered: {}", sub.name),
                    );
                }
            }
            // Never fatal: a failed fetch leaves `updated_at` untouched, so the
            // subscription stays due and the next tick tries again. Only the
            // first failure of a streak is logged — an offline laptop would
            // otherwise repeat the same line every 15 minutes.
            Err(e) => {
                if failing.insert(id.clone()) {
                    app.state::<AppState>().logs.push_now(
                        "warn",
                        format!("[umbra] auto-update failed for subscription {id}: {e}"),
                    );
                }
            }
        }
    }
}

/// Start the subscription auto-update scheduler: one task for the whole app,
/// running until the process exits.
///
/// The first sweep doubles as the catch-up for time spent closed — a 12-hour
/// subscription last refreshed two days ago is due the moment the app is back.
pub fn spawn_auto_updater(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut ticker = tokio::time::interval_at(
            tokio::time::Instant::now() + AUTO_UPDATE_STARTUP_DELAY,
            AUTO_UPDATE_TICK,
        );
        // A laptop waking from sleep owes one sweep, not one per hour it slept.
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut failing: HashSet<String> = HashSet::new();
        loop {
            ticker.tick().await;
            refresh_due_subscriptions(&app, &mut failing).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ConnectionState, Mode, Security, ServerEntry, Transport};

    fn ids(list: &[&str]) -> HashSet<String> {
        list.iter().map(|s| (*s).to_string()).collect()
    }

    fn entry(id: &str, name: &str) -> ServerEntry {
        ServerEntry {
            id: id.into(),
            name: name.into(),
            protocol: "vless".into(),
            server: "203.0.113.10".into(),
            port: 443,
            uuid: "11111111-2222-3333-4444-555555555555".into(),
            flow: String::new(),
            security: Security::None,
            sni: String::new(),
            fingerprint: String::new(),
            public_key: String::new(),
            short_id: String::new(),
            insecure: false,
            alpn: Vec::new(),
            transport: Transport::Tcp,
            last_ping_ms: None,
            favorite: false,
            total_up: 0,
            total_down: 0,
            raw: String::new(),
        }
    }

    #[test]
    fn deleting_the_connected_subscription_clears_only_the_selection() {
        // the reported bug: connected through the subscription being deleted
        let action = reconcile(&ids(&[]), Some("gone"), ConnStatus::Connected, Some("gone"));
        assert!(action.clear_selection, "a ghost id must not stay persisted");
        assert!(
            action.active_server_gone,
            "the UI has to be told the active server is no longer in the list"
        );
    }

    #[test]
    fn deleting_an_unrelated_server_changes_nothing() {
        let action = reconcile(
            &ids(&["a", "b"]),
            Some("a"),
            ConnStatus::Connected,
            Some("b"),
        );
        assert_eq!(
            action,
            Reconciliation {
                clear_selection: false,
                active_server_gone: false
            }
        );
    }

    #[test]
    fn selection_is_cleared_even_when_disconnected() {
        let action = reconcile(&ids(&["b"]), Some("a"), ConnStatus::Disconnected, None);
        assert!(action.clear_selection);
        assert!(!action.active_server_gone);
    }

    #[test]
    fn a_disconnected_state_never_reports_a_lost_active_server() {
        // stale server_id on a disconnected state is not a desync to announce
        let action = reconcile(&ids(&[]), None, ConnStatus::Disconnected, Some("gone"));
        assert!(!action.active_server_gone);
        assert!(!action.clear_selection);
    }

    #[test]
    fn connecting_and_stopping_count_as_live() {
        for status in [ConnStatus::Connecting, ConnStatus::Stopping] {
            let action = reconcile(&ids(&[]), None, status, Some("gone"));
            assert!(
                action.active_server_gone,
                "{status:?} still has a core attached to the deleted server"
            );
        }
    }

    #[test]
    fn no_selection_and_no_active_server_is_a_no_op() {
        let action = reconcile(&ids(&["a"]), None, ConnStatus::Disconnected, None);
        assert!(!action.clear_selection);
        assert!(!action.active_server_gone);
    }

    /// The invariant the user tripped over: never "connected" *and* "no
    /// server" at once. The name snapshot is what upholds it once the entry
    /// itself is gone.
    #[test]
    fn connected_state_can_still_name_a_deleted_server() {
        let mut conn = ConnectionState::disconnected(Mode::SystemProxy);
        let server = entry("gone", "Germany · Frankfurt");
        conn.status = ConnStatus::Connected;
        conn.server_id = Some(server.id.clone());
        conn.server_name = Some(server.name.clone());
        conn.since_ms = Some(1);

        // …the profile store no longer has it
        let alive = ids(&[]);
        assert!(!alive.contains(conn.server_id.as_deref().unwrap()));
        let action = reconcile(&alive, None, conn.status, conn.server_id.as_deref());
        assert!(action.active_server_gone);

        assert_eq!(conn.status, ConnStatus::Connected);
        assert!(
            conn.server_name.is_some(),
            "a live connection must always be able to name its server"
        );
        assert_eq!(conn.server_name.as_deref(), Some("Germany · Frankfurt"));
    }

    #[test]
    fn disconnected_state_carries_no_server_name() {
        let conn = ConnectionState::disconnected(Mode::Tun);
        assert_eq!(conn.status, ConnStatus::Disconnected);
        assert!(conn.server_id.is_none());
        assert!(conn.server_name.is_none());
    }

    // -- naming ------------------------------------------------------------

    const SUB_URL: &str = "https://panel.example.com/api/sub/example-token";

    /// The reported case: the card showed an account id because that is the
    /// Content-Disposition filename. The panel was sending the real name all
    /// along, in Profile-Title.
    #[test]
    fn panel_title_beats_the_content_disposition_filename() {
        assert_eq!(
            derive_name(Some("Example Network"), Some("account-123"), SUB_URL),
            "Example Network"
        );
    }

    #[test]
    fn naming_falls_back_filename_then_host() {
        assert_eq!(
            derive_name(None, Some("account-123"), SUB_URL),
            "account-123"
        );
        assert_eq!(derive_name(None, None, SUB_URL), "panel.example.com");
        assert_eq!(
            derive_name(Some("  "), Some(" "), "not a url"),
            "Subscription"
        );
    }

    #[test]
    fn a_refresh_upgrades_an_auto_derived_name() {
        // stored name came from the filename…
        assert!(may_adopt_title(
            "account-123",
            None,
            Some("account-123"),
            Some("panel.example.com")
        ));
        // …or from the host…
        assert!(may_adopt_title(
            "panel.example.com",
            None,
            Some("account-123"),
            Some("panel.example.com")
        ));
        // …or from an earlier title, which must stay in sync when it changes
        assert!(may_adopt_title(
            "Example Network",
            Some("Example Network"),
            Some("account-123"),
            Some("panel.example.com")
        ));
        assert!(may_adopt_title("   ", None, None, None));
        assert!(may_adopt_title("Subscription", None, None, None));
    }

    /// The one rule that matters: a name the user typed is theirs.
    #[test]
    fn a_refresh_never_overwrites_a_user_chosen_name() {
        assert!(!may_adopt_title(
            "Работа",
            Some("Example Network"),
            Some("account-123"),
            Some("panel.example.com")
        ));
        assert!(!may_adopt_title("My VPN", None, None, None));
    }

    // -- ordering ----------------------------------------------------------

    fn sub(id: &str) -> Subscription {
        Subscription {
            id: id.into(),
            name: id.into(),
            url: format!("https://example.com/{id}"),
            updated_at: None,
            quota: None,
            auto_update_hours: 0,
            support_url: None,
            web_page_url: None,
            panel_title: None,
            servers: Vec::new(),
        }
    }

    fn order_of(subs: &[Subscription]) -> Vec<&str> {
        subs.iter().map(|s| s.id.as_str()).collect()
    }

    #[test]
    fn reordering_follows_the_requested_sequence() {
        let mut subs = vec![sub("a"), sub("b"), sub("c")];
        apply_order(&mut subs, &["c".into(), "a".into(), "b".into()]);
        assert_eq!(order_of(&subs), ["c", "a", "b"]);
    }

    /// A subscription added in another window (or one the caller simply did
    /// not know about) must not vanish from the list.
    #[test]
    fn unmentioned_subscriptions_keep_their_relative_order_at_the_end() {
        let mut subs = vec![sub("a"), sub("b"), sub("c"), sub("d")];
        apply_order(&mut subs, &["c".into(), "a".into()]);
        assert_eq!(order_of(&subs), ["c", "a", "b", "d"]);
    }

    #[test]
    fn reordering_ignores_ids_that_no_longer_exist() {
        let mut subs = vec![sub("a"), sub("b")];
        apply_order(&mut subs, &["ghost".into(), "b".into(), "a".into()]);
        assert_eq!(order_of(&subs), ["b", "a"]);
        // and an empty request is a no-op, not a shuffle
        apply_order(&mut subs, &[]);
        assert_eq!(order_of(&subs), ["b", "a"]);
    }

    // -- auto-update schedule ----------------------------------------------

    use chrono::TimeZone;

    fn noon() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, 27, 12, 0, 0).unwrap()
    }

    /// A timestamp `hours_ago` before `noon`, written the way `now_iso` writes
    /// one — the scheduler only ever reads back what a refresh stamped.
    fn hours_before_noon(hours: i64) -> String {
        (noon() - chrono::Duration::hours(hours)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
    }

    #[test]
    fn auto_update_off_is_never_due() {
        // the picker's "Off" — no timestamp, however old, makes it due
        assert!(!due_for_refresh(0, None, noon()));
        assert!(!due_for_refresh(0, Some(&hours_before_noon(9_000)), noon()));
    }

    /// A store written before the field existed has no timestamp, and nothing
    /// else can prove its servers are current.
    #[test]
    fn a_subscription_that_never_recorded_a_refresh_is_due() {
        assert!(due_for_refresh(12, None, noon()));
        assert!(due_for_refresh(12, Some("last tuesday"), noon()));
        assert!(due_for_refresh(12, Some(""), noon()));
    }

    #[test]
    fn a_recent_refresh_is_not_due_yet() {
        assert!(!due_for_refresh(12, Some(&hours_before_noon(1)), noon()));
        assert!(!due_for_refresh(12, Some(&hours_before_noon(11)), noon()));
    }

    #[test]
    fn an_elapsed_interval_is_due() {
        assert!(due_for_refresh(12, Some(&hours_before_noon(12)), noon()));
        assert!(due_for_refresh(12, Some(&hours_before_noon(13)), noon()));
        // closed for two days: the first sweep after launch catches it up
        assert!(due_for_refresh(12, Some(&hours_before_noon(48)), noon()));
    }

    /// The interval is read off the subscription, not hardcoded.
    #[test]
    fn each_interval_is_honoured_independently() {
        let six_hours_ago = hours_before_noon(6);
        assert!(due_for_refresh(6, Some(&six_hours_ago), noon()));
        assert!(!due_for_refresh(24, Some(&six_hours_ago), noon()));
    }

    /// A clock that jumped backwards (or a timestamp from a machine running
    /// ahead) must wait, not refresh on every single tick.
    #[test]
    fn a_timestamp_in_the_future_is_not_due() {
        assert!(!due_for_refresh(12, Some(&hours_before_noon(-3)), noon()));
    }

    /// Guards the seam between the two halves: the scheduler parses exactly
    /// what a refresh stamps, so a change to either format is caught here.
    #[test]
    fn a_just_written_timestamp_reads_back_as_fresh() {
        assert!(!due_for_refresh(1, Some(&now_iso()), Utc::now()));
    }
}
