//! Installed sing-box core detection.

use std::path::{Path, PathBuf};

use crate::error::{AppError, AppResult};
use crate::models::CoreStatus;
use crate::state::AppState;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub fn core_path(data_dir: &Path) -> PathBuf {
    data_dir.join("bin").join("sing-box.exe")
}

/// Install the core shipped inside the bundle when the user has none yet.
/// Without this the first connect needs a GitHub download, which fails wherever
/// GitHub is blocked — exactly the networks this app exists for.
pub fn install_bundled_core(app: &tauri::AppHandle, data_dir: &Path) {
    use tauri::Manager;

    let target = core_path(data_dir);
    if target.exists() {
        return;
    }
    let Ok(bundled) = app.path().resolve(
        "resources/sing-box.exe",
        tauri::path::BaseDirectory::Resource,
    ) else {
        return;
    };
    if !bundled.exists() {
        return;
    }
    if let Some(dir) = target.parent() {
        if let Err(e) = std::fs::create_dir_all(dir) {
            eprintln!("[umbra] failed to create core dir: {e}");
            return;
        }
    }
    match std::fs::copy(&bundled, &target) {
        Ok(_) => eprintln!("[umbra] installed bundled sing-box core"),
        Err(e) => eprintln!("[umbra] failed to install bundled core: {e}"),
    }
}

/// Run `sing-box.exe version` and parse the version from the first stdout line.
pub fn installed_version(core: &Path) -> Option<String> {
    if !core.exists() {
        return None;
    }
    let mut cmd = std::process::Command::new(core);
    cmd.arg("version");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let output = cmd.output().ok()?;
    if !output.status.success() {
        return None;
    }
    parse_version_line(&String::from_utf8_lossy(&output.stdout))
}

/// First line looks like `sing-box version 1.13.14`.
fn parse_version_line(stdout: &str) -> Option<String> {
    let first = stdout.lines().next()?;
    let rest = first.trim().strip_prefix("sing-box version")?;
    let v = rest.split_whitespace().next()?;
    (!v.is_empty()).then(|| v.to_string())
}

pub fn is_version_compatible(version_str: &str) -> bool {
    let clean = version_str.trim().trim_start_matches('v');
    let mut parts = clean.split('.');
    let major = parts.next().and_then(|s| s.parse::<u64>().ok());
    let minor = parts.next().and_then(|s| s.parse::<u64>().ok());
    matches!((major, minor), (Some(1), Some(13)))
}

pub fn ensure_compatible_core(core_path: &Path) -> AppResult<String> {
    if !core_path.exists() {
        return Err(AppError::CoreNotInstalled);
    }
    let ver = installed_version(core_path)
        .ok_or_else(|| AppError::CoreStartFailed("failed to probe sing-box core version".into()))?;
    if !is_version_compatible(&ver) {
        return Err(AppError::Unsupported(format!(
            "installed sing-box version {ver} is incompatible with Umbra (requires sing-box 1.13.x)"
        )));
    }
    Ok(ver)
}

/// `installed_version` spawns a subprocess synchronously; keep it off the
/// async runtime.
pub async fn probe_version(core: PathBuf) -> AppResult<Option<String>> {
    tokio::task::spawn_blocking(move || installed_version(&core))
        .await
        .map_err(|e| AppError::Internal(format!("version probe task failed: {e}")))
}

#[tauri::command]
pub async fn get_core_status(state: tauri::State<'_, AppState>) -> AppResult<CoreStatus> {
    let path = core_path(&state.data_dir);
    let installed = path.exists();
    let version = if installed {
        probe_version(path.clone()).await?
    } else {
        None
    };
    Ok(CoreStatus {
        installed,
        version,
        path: path.to_string_lossy().into_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_version_output() {
        let out = "sing-box version 1.11.9\n\nEnvironment: go1.23.4 windows/amd64\n";
        assert_eq!(parse_version_line(out).as_deref(), Some("1.11.9"));
    }

    #[test]
    fn rejects_garbage_output() {
        assert!(parse_version_line("").is_none());
        assert!(parse_version_line("not sing-box").is_none());
        assert!(parse_version_line("sing-box version").is_none());
    }

    #[test]
    fn core_path_is_under_bin() {
        let p = core_path(Path::new("C:\\data"));
        assert!(p.ends_with("bin\\sing-box.exe") || p.ends_with("bin/sing-box.exe"));
    }

    #[test]
    fn missing_binary_yields_none() {
        assert!(installed_version(Path::new("C:\\definitely\\missing\\sing-box.exe")).is_none());
    }
}
