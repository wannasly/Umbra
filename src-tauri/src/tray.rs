//! System tray: dynamic icon, localized menu, connect/disconnect and mode
//! switching without opening the window.

use std::sync::Mutex;

use tauri::{
    image::Image,
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu},
    tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, Wry,
};

use crate::models::{ConnStatus, ConnectionState, Mode};
use crate::state::AppState;

// Bundled at compile time so the icons travel with the installer.
const ICON_DISCONNECTED: &[u8] = include_bytes!("../icons/tray-disconnected.png");
const ICON_CONNECTED: &[u8] = include_bytes!("../icons/tray-connected.png");

pub struct TrayHandles {
    icon: TrayIcon<Wry>,
    toggle: MenuItem<Wry>,
    server: MenuItem<Wry>,
    mode_system: CheckMenuItem<Wry>,
    mode_tun: CheckMenuItem<Wry>,
}

/// Managed separately from `AppState` so the tray can be rebuilt (language
/// change) without touching the rest of the state.
pub struct TrayState(pub Mutex<Option<TrayHandles>>);

struct Strings {
    show: &'static str,
    connect: &'static str,
    disconnect: &'static str,
    connecting: &'static str,
    mode: &'static str,
    mode_system: &'static str,
    mode_tun: &'static str,
    quit: &'static str,
    no_server: &'static str,
}

fn strings(lang: &str) -> Strings {
    if lang == "en" {
        Strings {
            show: "Show window",
            connect: "Connect",
            disconnect: "Disconnect",
            connecting: "Connecting…",
            mode: "Mode",
            mode_system: "System proxy",
            mode_tun: "TUN mode",
            quit: "Quit Umbra",
            no_server: "No server selected",
        }
    } else {
        Strings {
            show: "Показать окно",
            connect: "Подключить",
            disconnect: "Отключить",
            connecting: "Подключение…",
            mode: "Режим",
            mode_system: "Системный прокси",
            mode_tun: "Режим TUN",
            quit: "Выход",
            no_server: "Сервер не выбран",
        }
    }
}

fn show_window(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.show();
        let _ = win.unminimize();
        let _ = win.set_focus();
    }
}

/// Build the tray icon and its menu. Called once at startup and again whenever
/// the UI language changes.
pub fn build(app: &AppHandle, lang: &str, mode: Mode) -> tauri::Result<()> {
    let s = strings(lang);

    let show = MenuItem::with_id(app, "tray_show", s.show, true, None::<&str>)?;
    let toggle = MenuItem::with_id(app, "tray_toggle", s.connect, true, None::<&str>)?;
    let server = MenuItem::with_id(app, "tray_server", s.no_server, false, None::<&str>)?;
    let mode_system = CheckMenuItem::with_id(
        app,
        "tray_mode_system",
        s.mode_system,
        true,
        mode == Mode::SystemProxy,
        None::<&str>,
    )?;
    let mode_tun = CheckMenuItem::with_id(
        app,
        "tray_mode_tun",
        s.mode_tun,
        true,
        mode == Mode::Tun,
        None::<&str>,
    )?;
    let mode_menu =
        Submenu::with_id_and_items(app, "tray_mode", s.mode, true, &[&mode_system, &mode_tun])?;
    let quit = MenuItem::with_id(app, "tray_quit", s.quit, true, None::<&str>)?;

    let menu = Menu::with_items(
        app,
        &[
            &show,
            &PredefinedMenuItem::separator(app)?,
            &server,
            &toggle,
            &mode_menu,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;

    // Replace an existing tray (language rebuild) instead of stacking a second one.
    if let Some(old) = app.state::<TrayState>().0.lock().unwrap().take() {
        let _ = app.remove_tray_by_id(old.icon.id());
    }

    let icon = TrayIconBuilder::with_id("umbra-tray")
        .icon(Image::from_bytes(ICON_DISCONNECTED)?)
        .tooltip("Umbra")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(on_menu_event)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_window(tray.app_handle());
            }
        })
        .build(app)?;

    *app.state::<TrayState>().0.lock().unwrap() = Some(TrayHandles {
        icon,
        toggle,
        server,
        mode_system,
        mode_tun,
    });

    // Reflect whatever the current connection state is.
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let snapshot = app.state::<AppState>().conn.read().await.clone();
        sync(&app, &snapshot).await;
    });

    Ok(())
}

fn on_menu_event(app: &AppHandle, event: tauri::menu::MenuEvent) {
    let app = app.clone();
    match event.id.as_ref() {
        "tray_show" => show_window(&app),
        "tray_quit" => {
            // RunEvent::Exit performs the teardown (proxy restore + core kill).
            app.exit(0);
        }
        "tray_toggle" => {
            tauri::async_runtime::spawn(async move {
                use crate::commands::connection::{do_connect, do_disconnect};
                let state = app.state::<AppState>();
                // Same serialization the commands use: never overlap connect ops.
                let _ops = state.conn_ops.lock().await;
                let status = { state.conn.read().await.status };
                let result = match status {
                    ConnStatus::Connected | ConnStatus::Connecting => {
                        do_disconnect(&app).await.map(|_| ())
                    }
                    _ => {
                        let selected = { state.settings.read().await.selected_server_id.clone() };
                        match selected {
                            Some(id) => do_connect(&app, id).await.map(|_| ()),
                            None => {
                                show_window(&app);
                                Ok(())
                            }
                        }
                    }
                };
                if let Err(e) = result {
                    eprintln!("[umbra] tray toggle failed: {e}");
                    crate::commands::connection::notify_needs_elevation(&app, &e);
                    show_window(&app);
                }
            });
        }
        "tray_mode_system" | "tray_mode_tun" => {
            let mode = if event.id.as_ref() == "tray_mode_tun" {
                Mode::Tun
            } else {
                Mode::SystemProxy
            };
            tauri::async_runtime::spawn(async move {
                let state = app.state::<AppState>();
                let result = {
                    let _ops = state.conn_ops.lock().await;
                    crate::commands::connection::do_set_mode(&app, mode).await
                };
                if let Err(e) = result {
                    // NeedsElevation and friends need the UI to explain themselves.
                    eprintln!("[umbra] tray mode switch failed: {e}");
                    crate::commands::connection::notify_needs_elevation(&app, &e);
                    show_window(&app);
                }
                let snapshot = { state.conn.read().await.clone() };
                sync(&app, &snapshot).await;
            });
        }
        _ => {}
    }
}

/// Push the current connection state into the tray icon and menu labels.
pub async fn sync(app: &AppHandle, conn: &ConnectionState) {
    let state = app.state::<AppState>();
    let (lang, selected) = {
        let settings = state.settings.read().await;
        (
            settings.language.clone(),
            settings.selected_server_id.clone(),
        )
    };
    let s = strings(&lang);

    // The snapshot in `conn.server_name` outlives the profile entry, so a
    // subscription deleted mid-session cannot turn the tray label into
    // "no server" while the tunnel is still up.
    let server_name = match conn.server_id.clone().or(selected) {
        Some(id) => {
            let profiles = state.profiles.read().await;
            profiles
                .find_server(&id)
                .map(|srv| srv.name.clone())
                .or_else(|| conn.server_name.clone())
                .unwrap_or_else(|| s.no_server.to_string())
        }
        None => conn
            .server_name
            .clone()
            .unwrap_or_else(|| s.no_server.to_string()),
    };

    let guard = app.state::<TrayState>();
    let handles = guard.0.lock().unwrap();
    let Some(h) = handles.as_ref() else { return };

    let (toggle_text, connected) = match conn.status {
        ConnStatus::Connected => (s.disconnect, true),
        ConnStatus::Connecting => (s.connecting, false),
        ConnStatus::Stopping => (s.connecting, false),
        ConnStatus::Disconnected => (s.connect, false),
    };

    let _ = h.toggle.set_text(toggle_text);
    let _ = h.server.set_text(&server_name);
    let _ = h.mode_system.set_checked(conn.mode == Mode::SystemProxy);
    let _ = h.mode_tun.set_checked(conn.mode == Mode::Tun);

    let icon_bytes = if connected {
        ICON_CONNECTED
    } else {
        ICON_DISCONNECTED
    };
    if let Ok(img) = Image::from_bytes(icon_bytes) {
        let _ = h.icon.set_icon(Some(img));
    }
    let tooltip = if connected {
        format!("Umbra — {server_name}")
    } else {
        "Umbra".to_string()
    };
    let _ = h.icon.set_tooltip(Some(&tooltip));
}
