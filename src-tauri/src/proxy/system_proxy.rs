//! WinINET system proxy control via HKCU Internet Settings.
//!
//! Crash-safety ordering: the pre-existing values are snapshotted into
//! `settings.proxy_backup` and persisted to disk *before* the registry is
//! touched, so `startup_recovery` can always restore them after a crash.

use tokio::task::spawn_blocking;
use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE};
use winreg::RegKey;

use crate::error::{AppError, AppResult};
use crate::models::ProxyBackup;
use crate::state::AppState;
use crate::storage;

const INTERNET_SETTINGS: &str = r"Software\Microsoft\Windows\CurrentVersion\Internet Settings";

/// Snapshot the current WinINET proxy values (missing values -> 0 / None).
pub fn read_current() -> AppResult<ProxyBackup> {
    let key =
        RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(INTERNET_SETTINGS, KEY_READ)?;
    Ok(ProxyBackup {
        enable: key.get_value::<u32, _>("ProxyEnable").unwrap_or(0),
        server: key.get_value::<String, _>("ProxyServer").ok(),
        bypass_list: key.get_value::<String, _>("ProxyOverride").ok(),
    })
}

/// Point the system proxy at 127.0.0.1:`port`, backing up the previous
/// values into settings (persisted before any registry write).
pub async fn enable_proxy(state: &AppState, port: u16) -> AppResult<()> {
    let backup = spawn_blocking(read_current).await.map_err(join_err)??;
    {
        let mut settings = state.settings.write().await;
        // If a previous enable still owns the proxy (e.g. a failed restore
        // during crash cleanup), the registry holds *our* values; refreshing
        // the backup now would destroy the user's real settings forever.
        if !settings.proxy_owned {
            settings.proxy_backup = backup;
        }
        settings.proxy_owned = true;
        storage::save_settings(&state.data_dir, &settings)?;
    }
    spawn_blocking(move || apply_proxy(port))
        .await
        .map_err(join_err)??;
    Ok(())
}

/// Restore the backed-up proxy values and release ownership.
pub async fn disable_proxy(state: &AppState) -> AppResult<()> {
    let backup = state.settings.read().await.proxy_backup.clone();
    spawn_blocking(move || restore_proxy(&backup))
        .await
        .map_err(join_err)??;
    {
        let mut settings = state.settings.write().await;
        settings.proxy_owned = false;
        storage::save_settings(&state.data_dir, &settings)?;
    }
    Ok(())
}

/// If a previous session crashed while owning the system proxy, restore it.
pub async fn startup_recovery(state: &AppState) {
    let owned = state.settings.read().await.proxy_owned;
    if !owned {
        return;
    }
    eprintln!("[umbra] previous session left the system proxy enabled; restoring backup");
    if let Err(e) = disable_proxy(state).await {
        eprintln!("[umbra] failed to restore system proxy: {e}");
    }
}

fn apply_proxy(port: u16) -> AppResult<()> {
    let key = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(INTERNET_SETTINGS, KEY_SET_VALUE)?;
    key.set_value("ProxyEnable", &1u32)?;
    key.set_value("ProxyServer", &proxy_server_value(port))?;
    key.set_value("ProxyOverride", &default_bypass_list())?;
    notify_wininet();
    Ok(())
}

fn restore_proxy(backup: &ProxyBackup) -> AppResult<()> {
    let key = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(INTERNET_SETTINGS, KEY_SET_VALUE)?;
    key.set_value("ProxyEnable", &backup.enable)?;
    match &backup.server {
        Some(server) => key.set_value("ProxyServer", server)?,
        None => {
            let _ = key.delete_value("ProxyServer");
        }
    }
    match &backup.bypass_list {
        Some(bypass) => key.set_value("ProxyOverride", bypass)?,
        None => {
            let _ = key.delete_value("ProxyOverride");
        }
    }
    notify_wininet();
    Ok(())
}

/// Tell WinINET consumers the proxy settings changed.
fn notify_wininet() {
    use windows::Win32::Networking::WinInet::{
        InternetSetOptionW, INTERNET_OPTION_REFRESH, INTERNET_OPTION_SETTINGS_CHANGED,
    };
    unsafe {
        if let Err(e) = InternetSetOptionW(None, INTERNET_OPTION_SETTINGS_CHANGED, None, 0) {
            eprintln!("[umbra] InternetSetOptionW(SETTINGS_CHANGED) failed: {e}");
        }
        if let Err(e) = InternetSetOptionW(None, INTERNET_OPTION_REFRESH, None, 0) {
            eprintln!("[umbra] InternetSetOptionW(REFRESH) failed: {e}");
        }
    }
}

fn proxy_server_value(port: u16) -> String {
    format!("127.0.0.1:{port}")
}

fn default_bypass_list() -> String {
    let mut parts: Vec<String> = vec!["localhost".into(), "127.*".into(), "10.*".into()];
    for n in 16u8..=31 {
        parts.push(format!("172.{n}.*"));
    }
    parts.push("192.168.*".into());
    parts.push("<local>".into());
    parts.join(";")
}

fn join_err(e: tokio::task::JoinError) -> AppError {
    AppError::Internal(format!("blocking task failed: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_server_value_formats_loopback() {
        assert_eq!(proxy_server_value(2080), "127.0.0.1:2080");
    }

    #[test]
    fn bypass_list_covers_private_ranges() {
        let list = default_bypass_list();
        assert!(list.starts_with("localhost;127.*;10.*;172.16.*;"));
        assert!(list.ends_with(";192.168.*;<local>"));
        for n in 16..=31 {
            assert!(list.contains(&format!(";172.{n}.*;")));
        }
        assert!(!list.contains("172.15.*"));
        assert!(!list.contains("172.32.*"));
        // 21 entries -> 20 separators
        assert_eq!(list.matches(';').count(), 20);
    }
}
