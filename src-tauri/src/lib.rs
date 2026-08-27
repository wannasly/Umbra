mod commands;
mod error;
mod events;
mod hwid;
mod models;
mod net;
mod parser;
mod proxy;
mod singbox;
mod state;
mod storage;
mod storage_migration;
mod subscription;
mod tray;

use tauri::Manager;

use crate::state::AppState;
use crate::tray::TrayState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // Second launch: focus the existing window instead.
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.show();
                let _ = win.set_focus();
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--minimized"]),
        ))
        .setup(|app| {
            let data_dir = app
                .path()
                .app_data_dir()
                .expect("failed to resolve app data dir");
            std::fs::create_dir_all(&data_dir)?;

            let mut settings = storage::load_settings(&data_dir);
            // Panels with a device limit reject requests without a stable id.
            if settings.hwid.is_empty() {
                settings.hwid = hwid::hwid(None);
                if let Err(e) = storage::save_settings(&data_dir, &settings) {
                    eprintln!("[umbra] failed to persist hwid: {e}");
                }
            }
            singbox::version::install_bundled_core(app.handle(), &data_dir);

            let profiles = storage::load_profiles(&data_dir).unwrap_or_else(|e| {
                eprintln!("[umbra] failed to load profiles: {e}");
                models::ProfileStore::default()
            });
            let (language, mode, start_minimized) = (
                settings.language.clone(),
                settings.mode,
                settings.start_minimized,
            );
            // `--resume-tun` is passed by the elevated relaunch, which also
            // persisted mode=tun before handing over.
            let auto_connect = settings.connect_on_startup || proxy::elevation::is_resume_tun();
            app.manage(AppState::new(data_dir, settings, profiles));
            app.manage(TrayState(std::sync::Mutex::new(None)));

            tray::build(app.handle(), &language, mode)?;

            if start_minimized || std::env::args().any(|a| a == "--minimized") {
                if let Some(win) = app.get_webview_window("main") {
                    let _ = win.hide();
                }
            }

            // Restore the system proxy if a previous session crashed — or was
            // force-killed — while owning it, and only then auto-connect:
            // recovery restores the saved backup, so running it after a connect
            // would wipe the proxy that connect just enabled. `conn_ops` is
            // held for the recovery only (and released before startup_connect,
            // which takes it itself) so a user clicking Connect the instant the
            // window appears cannot slip between the two.
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                {
                    let state = handle.state::<AppState>();
                    let _ops = state.conn_ops.lock().await;
                    proxy::system_proxy::startup_recovery(&state).await;
                }
                if auto_connect {
                    commands::connection::startup_connect(&handle).await;
                }
            });

            // Batch-emits accumulated core log lines every 250ms.
            singbox::process::spawn_log_flusher(app.handle().clone());
            // Refreshes subscriptions whose auto-update interval has elapsed,
            // including those that went stale while the app was closed.
            commands::profiles::spawn_auto_updater(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::settings::get_settings,
            commands::settings::set_settings,
            commands::settings::get_connection_state,
            commands::settings::open_data_dir,
            commands::settings::is_elevated,
            commands::settings::relaunch_elevated,
            commands::processes::get_running_processes,
            commands::profiles::import_share_links,
            commands::profiles::list_servers,
            commands::profiles::remove_server,
            commands::profiles::select_server,
            commands::profiles::set_server_favorite,
            commands::profiles::add_subscription,
            commands::profiles::update_subscription,
            commands::profiles::remove_subscription,
            commands::profiles::rename_subscription,
            commands::profiles::reorder_subscriptions,
            commands::profiles::set_subscription_auto_update,
            commands::connection::connect,
            commands::connection::disconnect,
            commands::connection::set_mode,
            commands::connection::ping_servers,
            commands::connection::url_test_active,
            commands::connection::get_recent_logs,
            commands::connection::clear_logs,
            singbox::version::get_core_status,
            singbox::download::check_core_update,
            singbox::download::download_core,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|app_handle, event| match event {
        tauri::RunEvent::WindowEvent {
            label,
            event: tauri::WindowEvent::CloseRequested { api, .. },
            ..
        } => {
            // Recoverable: the tray icon (and relaunching the app, via the
            // single-instance plugin) brings the window back.
            let minimize = tauri::async_runtime::block_on(async {
                app_handle
                    .state::<AppState>()
                    .settings
                    .read()
                    .await
                    .minimize_to_tray
            });
            if minimize {
                api.prevent_close();
                if let Some(win) = app_handle.get_webview_window(&label) {
                    let _ = win.hide();
                }
            }
        }
        tauri::RunEvent::Exit => {
            // Best-effort teardown: restore the system proxy if owned and kill
            // the sing-box child this app spawned.
            tauri::async_runtime::block_on(singbox::process::stop(app_handle));
        }
        _ => {}
    });
}
