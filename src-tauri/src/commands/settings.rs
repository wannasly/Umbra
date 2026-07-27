//! Settings and misc app commands.

use serde_json::Value;
use tauri_plugin_autostart::ManagerExt;

use crate::error::{AppError, AppResult};
use crate::models::{ConnectionState, Mode, Settings};
use crate::proxy::elevation;
use crate::state::AppState;
use crate::storage;

/// Shallow-merge a camelCase JSON patch (Partial<Settings> from TS) onto the
/// current settings via their wire representation.
fn merge_settings(current: &Settings, patch: &Value) -> AppResult<Settings> {
    let patch_obj = patch
        .as_object()
        .ok_or_else(|| AppError::Parse("settings patch must be an object".into()))?;
    let mut value = serde_json::to_value(current)?;
    let Value::Object(obj) = &mut value else {
        return Err(AppError::Internal(
            "settings did not serialize to an object".into(),
        ));
    };
    for (k, v) in patch_obj {
        obj.insert(k.clone(), v.clone());
    }
    serde_json::from_value(value)
        .map_err(|e| AppError::Parse(format!("invalid settings patch: {e}")))
}

#[tauri::command]
pub async fn get_settings(state: tauri::State<'_, AppState>) -> AppResult<Settings> {
    Ok(state.settings.read().await.clone())
}

#[tauri::command]
pub async fn set_settings(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    patch: Value,
) -> AppResult<Settings> {
    let mut settings = state.settings.write().await;
    let merged = merge_settings(&settings, &patch)?;
    storage::save_settings(&state.data_dir, &merged)?;

    if patch.get("autostart").is_some() {
        let autolaunch = app.autolaunch();
        let result = if merged.autostart {
            autolaunch.enable()
        } else {
            autolaunch.disable()
        };
        if let Err(e) = result {
            eprintln!("[umbra] failed to update autostart: {e}");
        }
    }

    let language_changed = settings.language != merged.language;
    *settings = merged.clone();
    drop(settings);

    // Tray labels are baked into the menu items, so a language switch needs a rebuild.
    if language_changed {
        if let Err(e) = crate::tray::build(&app, &merged.language, merged.mode) {
            eprintln!("[umbra] failed to rebuild tray after language change: {e}");
        }
    }

    Ok(merged)
}

#[tauri::command]
pub async fn get_connection_state(state: tauri::State<'_, AppState>) -> AppResult<ConnectionState> {
    Ok(state.conn.read().await.clone())
}

#[tauri::command]
pub async fn open_data_dir(state: tauri::State<'_, AppState>) -> AppResult<()> {
    std::process::Command::new("explorer.exe")
        .arg(&state.data_dir)
        .spawn()?;
    Ok(())
}

#[tauri::command]
pub async fn is_elevated() -> AppResult<bool> {
    Ok(elevation::is_elevated())
}

/// Relaunch under UAC to run TUN. The elevated instance is a fresh process
/// that reads `settings.json` from disk, so the target mode has to be persisted
/// here — `set_mode` rejected before it could store it. Persist first and roll
/// back on a declined prompt: writing after `ShellExecuteW` would race the new
/// instance's own load when UAC auto-approves.
#[tauri::command]
pub async fn relaunch_elevated(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> AppResult<()> {
    let previous = {
        let mut settings = state.settings.write().await;
        let previous = settings.mode;
        settings.mode = Mode::Tun;
        storage::save_settings(&state.data_dir, &settings)?;
        previous
    };
    let launched =
        tokio::task::spawn_blocking(|| elevation::relaunch_elevated(&[elevation::RESUME_TUN_FLAG]))
            .await
            .map_err(|e| AppError::Internal(format!("blocking task failed: {e}")))?;
    if let Err(e) = launched {
        let mut settings = state.settings.write().await;
        settings.mode = previous;
        if let Err(save) = storage::save_settings(&state.data_dir, &settings) {
            eprintln!("[umbra] failed to roll back mode after a declined elevation: {save}");
        }
        return Err(e);
    }
    app.exit(0);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn merge_applies_camel_case_keys_shallowly() {
        let current = Settings::default();
        let patch = json!({ "mixedPort": 1234, "autostart": true, "selectedServerId": "abc" });
        let merged = merge_settings(&current, &patch).unwrap();
        assert_eq!(merged.mixed_port, 1234);
        assert!(merged.autostart);
        assert_eq!(merged.selected_server_id.as_deref(), Some("abc"));
        // untouched fields keep their values
        assert_eq!(merged.language, current.language);
        assert_eq!(merged.tun_mtu, current.tun_mtu);
    }

    #[test]
    fn merge_rejects_non_object_patch() {
        let err = merge_settings(&Settings::default(), &json!(42)).unwrap_err();
        assert_eq!(err.code(), "PARSE_ERROR");
    }

    #[test]
    fn merge_rejects_wrongly_typed_value() {
        let err =
            merge_settings(&Settings::default(), &json!({ "mixedPort": "nope" })).unwrap_err();
        assert_eq!(err.code(), "PARSE_ERROR");
    }

    #[test]
    fn merge_allows_null_for_nullable_field() {
        let merged =
            merge_settings(&Settings::default(), &json!({ "selectedServerId": null })).unwrap();
        assert!(merged.selected_server_id.is_none());
    }
}
