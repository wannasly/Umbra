//! UI-independent command dispatcher shared by the native Slint frontend.
use crate::{
    commands::{connection, profiles, settings as commands_settings},
    error::{AppError, AppResult},
    models::*,
    runtime::AppHandle,
    singbox::{download, version},
};
fn t(ru: bool, russian: &'static str, english: &'static str) -> &'static str {
    if ru {
        russian
    } else {
        english
    }
}
pub enum Task {
    Toggle,
    Reconnect,
    Mode(Mode),
    Select(String),
    Ping(Vec<String>),
    UrlTest,
    Import(String),
    AddSub(String, String),
    UpdateSub(String),
    RemoveSub(String),
    RemoveServer(String),
    Favorite(String, bool),
    RenameSub(String, String),
    AutoUpdate(String, u32),
    Reorder(Vec<String>),
    Save(serde_json::Value),
    CoreStatus,
    CheckUpdate,
    Download(Option<String>),
    OpenData,
    Elevate,
    Processes,
}

pub struct Completion {
    pub saved: bool,
    pub result: AppResult<String>,
    pub core: Option<CoreStatus>,
    pub processes: Option<Vec<RunningProcess>>,
    pub update: Option<UpdateCheck>,
}

pub async fn execute(app: AppHandle, task: Task) -> Completion {
    let saved = matches!(&task, Task::Save(_));
    let state = app.state();
    let mut core = None;
    let mut processes = None;
    let mut update = None;
    let result: AppResult<String> = async {
        match task {
            Task::Reconnect => {
                let _ops = state.conn_ops.lock().await;
                let id = state
                    .conn
                    .read()
                    .await
                    .server_id
                    .clone()
                    .or_else(|| {
                        state
                            .settings
                            .try_read()
                            .ok()
                            .and_then(|s| s.selected_server_id.clone())
                    })
                    .ok_or_else(|| AppError::NotFound("Select a server".into()))?;
                connection::do_disconnect(&app).await?;
                connection::do_connect(&app, id).await?;
            }
            Task::Toggle => {
                let _ops = state.conn_ops.lock().await;
                let status = state.conn.read().await.status;
                if status == ConnStatus::Connected {
                    connection::do_disconnect(&app).await?;
                } else if status == ConnStatus::Disconnected {
                    let id = state
                        .settings
                        .read()
                        .await
                        .selected_server_id
                        .clone()
                        .ok_or_else(|| {
                            AppError::NotFound("Выберите сервер / Select a server".into())
                        })?;
                    connection::do_connect(&app, id).await?;
                }
            }
            Task::Mode(mode) => {
                connection::set_mode(app.clone(), state, mode).await?;
            }
            Task::Select(id) => {
                profiles::select_server(app.clone(), state, id).await?;
            }
            Task::Ping(ids) => {
                let targets = {
                    let p = state.profiles.read().await;
                    ids.iter()
                        .filter_map(|id| p.find_server(id))
                        .map(|s| crate::net::ping::PingTarget {
                            id: s.id.clone(),
                            host: s.server.clone(),
                            port: s.port,
                        })
                        .collect()
                };
                crate::net::ping::ping_many(app.clone(), targets).await;
            }
            Task::UrlTest => {
                return Ok(format!(
                    "{} ms",
                    connection::url_test_active(app.clone(), state).await?
                ))
            }
            Task::Import(text) => {
                let r = profiles::import_share_links(state, text).await?;
                return Ok(format!(
                    "{}: {}. {}",
                    t(
                        state.settings.read().await.language == "ru",
                        "Добавлено",
                        "Added"
                    ),
                    r.added,
                    r.errors.join("\n")
                ));
            }
            Task::AddSub(url, name) => {
                profiles::add_subscription(state, url, (!name.trim().is_empty()).then_some(name))
                    .await?;
            }
            Task::UpdateSub(id) => {
                profiles::update_subscription(app.clone(), state, id).await?;
            }
            Task::RemoveSub(id) => {
                profiles::remove_subscription(app.clone(), state, id).await?;
            }
            Task::RemoveServer(id) => {
                profiles::remove_server(app.clone(), state, id).await?;
            }
            Task::Favorite(id, favorite) => {
                profiles::set_server_favorite(state, id, favorite).await?
            }
            Task::RenameSub(id, name) => {
                profiles::rename_subscription(state, id, name).await?;
            }
            Task::AutoUpdate(id, hours) => {
                profiles::set_subscription_auto_update(state, id, hours).await?
            }
            Task::Reorder(ids) => profiles::reorder_subscriptions(state, ids).await?,
            Task::Save(patch) => {
                commands_settings::set_settings(app.clone(), state, patch).await?;
            }
            Task::CoreStatus => core = Some(version::get_core_status(state).await?),
            Task::CheckUpdate => {
                update = Some(download::check_core_update(state).await?);
            }
            Task::Download(selected_version) => {
                // Never replace an executable while its core is running.
                let _ops = state.conn_ops.lock().await;
                if state.conn.read().await.status != ConnStatus::Disconnected {
                    return Err(AppError::Internal(
                        "Сначала отключитесь / Disconnect first".into(),
                    ));
                }
                download::download_core(app.clone(), state, selected_version).await?;
                core = Some(version::get_core_status(state).await?);
            }
            Task::OpenData => commands_settings::open_data_dir(state).await?,
            Task::Elevate => commands_settings::relaunch_elevated(app.clone(), state).await?,
            Task::Processes => {
                processes = Some(crate::commands::processes::get_running_processes().await?)
            }
        }
        Ok(String::new())
    }
    .await;
    Completion {
        saved,
        result,
        core,
        processes,
        update,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::AppState;

    fn fixture() -> (tempfile::TempDir, AppHandle) {
        let dir = tempfile::tempdir().unwrap();
        let app = AppHandle::new(AppState::new(
            dir.path().to_owned(),
            Settings::default(),
            ProfileStore::default(),
        ));
        (dir, app)
    }

    fn link(name: &str) -> String {
        format!("vless://00000000-0000-4000-8000-000000000001@127.0.0.1:443?security=none#{name}")
    }

    #[tokio::test]
    async fn native_commands_import_select_favorite_and_delete_persistently() {
        let (dir, app) = fixture();
        assert!(execute(app.clone(), Task::Import(link("Test")))
            .await
            .result
            .is_ok());
        let id = app.state().profiles.read().await.manual[0].id.clone();
        assert!(execute(app.clone(), Task::Select(id.clone()))
            .await
            .result
            .is_ok());
        assert!(execute(app.clone(), Task::Favorite(id.clone(), true))
            .await
            .result
            .is_ok());
        let stored = crate::storage::load_profiles(dir.path()).unwrap();
        assert!(stored.manual[0].favorite);
        assert_eq!(
            crate::storage::load_settings(dir.path())
                .selected_server_id
                .as_deref(),
            Some(id.as_str())
        );
        // No core is installed in this isolated data directory: connection must
        // fail before touching the real Windows proxy.
        assert!(matches!(
            execute(app.clone(), Task::Toggle).await.result,
            Err(AppError::CoreNotInstalled)
        ));
        assert!(!app.state().settings.read().await.proxy_owned);
        assert!(execute(app.clone(), Task::RemoveServer(id))
            .await
            .result
            .is_ok());
        assert!(crate::storage::load_profiles(dir.path())
            .unwrap()
            .manual
            .is_empty());
        assert!(crate::storage::load_settings(dir.path())
            .selected_server_id
            .is_none());
    }

    #[tokio::test]
    async fn native_subscription_flow_preserves_selection_and_favorites_on_refresh() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let (_dir, app) = fixture();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let provider = tokio::spawn(async move {
            for _ in 0..2 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut buffer = [0; 8192];
                assert!(socket.read(&mut buffer).await.unwrap() > 0);
                let body = link("Subscription");
                socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nProfile-Title: Provider\r\nSubscription-Userinfo: upload=10; download=20; total=1000; expire=0\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
            }
        });
        assert!(execute(
            app.clone(),
            Task::AddSub(format!("http://{address}/sub"), String::new())
        )
        .await
        .result
        .is_ok());
        let sub = app.state().profiles.read().await.subscriptions[0].clone();
        let server = sub.servers[0].id.clone();
        assert!(execute(app.clone(), Task::Favorite(server.clone(), true))
            .await
            .result
            .is_ok());
        assert!(execute(app.clone(), Task::Select(server.clone()))
            .await
            .result
            .is_ok());
        assert!(execute(
            app.clone(),
            Task::RenameSub(sub.id.clone(), "My provider".into())
        )
        .await
        .result
        .is_ok());
        assert!(execute(app.clone(), Task::AutoUpdate(sub.id.clone(), 12))
            .await
            .result
            .is_ok());
        assert!(execute(app.clone(), Task::UpdateSub(sub.id.clone()))
            .await
            .result
            .is_ok());
        provider.await.unwrap();
        {
            let profiles = app.state().profiles.read().await;
            let refreshed = &profiles.subscriptions[0];
            assert_eq!(refreshed.name, "My provider");
            assert_eq!(refreshed.auto_update_hours, 12);
            assert_eq!(refreshed.quota.unwrap().download, 20);
            assert_eq!(refreshed.servers[0].id, server);
            assert!(refreshed.servers[0].favorite);
        }
        assert!(execute(app.clone(), Task::RemoveSub(sub.id))
            .await
            .result
            .is_ok());
        assert!(app
            .state()
            .settings
            .read()
            .await
            .selected_server_id
            .is_none());
    }
}
