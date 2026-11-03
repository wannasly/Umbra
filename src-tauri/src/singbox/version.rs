//! Installed core detection (Mihomo and sing-box).

use std::path::{Path, PathBuf};

use crate::error::{AppError, AppResult};
use crate::models::CoreStatus;
use crate::state::AppState;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub fn core_path_for(data_dir: &Path, engine: &str) -> PathBuf {
    let name = if engine == "sing-box" {
        "sing-box.exe"
    } else {
        "mihomo.exe"
    };
    data_dir.join("bin").join(name)
}

/// Install the core shipped inside the bundle when the user has none yet.
/// Without this the first connect needs a GitHub download, which fails wherever
/// GitHub is blocked — exactly the networks this app exists for.
pub fn install_bundled_core(data_dir: &Path) {
    let resource_dirs = [
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.join("resources"))),
        Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources")),
    ];
    for name in ["mihomo.exe", "sing-box.exe"] {
        let target = data_dir.join("bin").join(name);
        if target.exists() {
            continue;
        }
        let bundled = resource_dirs
            .iter()
            .flatten()
            .map(|dir| dir.join(name))
            .find(|path| path.exists());
        let Some(bundled) = bundled else { continue };
        if let Some(dir) = target.parent() {
            if let Err(e) = std::fs::create_dir_all(dir) {
                eprintln!("[umbra] failed to create core dir: {e}");
                continue;
            }
        }
        match std::fs::copy(&bundled, &target) {
            Ok(_) => eprintln!("[umbra] installed bundled core: {name}"),
            Err(e) => eprintln!("[umbra] failed to install bundled core {name}: {e}"),
        }
    }
}

/// Check if the specified core binary is Mihomo.
pub fn is_mihomo(core: &Path) -> bool {
    let name = core
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if name.contains("mihomo") {
        return true;
    }
    false
}

/// Run `mihomo -v` or `sing-box version` and parse the version from stdout.
pub fn installed_version(core: &Path) -> Option<String> {
    if !core.exists() {
        return None;
    }
    let is_mih = is_mihomo(core);
    let first_arg = if is_mih { "-v" } else { "version" };
    let mut cmd = std::process::Command::new(core);
    cmd.arg(first_arg);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    if let Ok(output) = cmd.output() {
        if output.status.success() {
            if let Some(v) = parse_version_line(&String::from_utf8_lossy(&output.stdout)) {
                return Some(v);
            }
        }
    }

    let second_arg = if is_mih { "version" } else { "-v" };
    let mut cmd = std::process::Command::new(core);
    cmd.arg(second_arg);
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

/// Parses version line for either Mihomo or sing-box.
/// Mihomo: `Mihomo Meta v1.19.33 windows ...` or `Mihomo Meta alpha-69c1631 ...`
/// sing-box: `sing-box version 1.13.14`
pub fn parse_version_line(stdout: &str) -> Option<String> {
    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("sing-box version") {
            let v = rest.split_whitespace().next()?;
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
        if let Some(rest) = trimmed.strip_prefix("Mihomo Meta") {
            let v = rest.split_whitespace().next()?;
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
        if let Some(rest) = trimmed.strip_prefix("Mihomo") {
            let v = rest.split_whitespace().next()?;
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
}

pub fn is_mihomo_version_compatible(version_str: &str) -> bool {
    let clean = version_str.trim().trim_start_matches('v');
    if clean.starts_with("alpha") {
        return true;
    }
    let mut parts = clean.split('.');
    let major = parts.next().and_then(|s| s.parse::<u64>().ok());
    let minor = parts.next().and_then(|s| s.parse::<u64>().ok());
    matches!((major, minor), (Some(1), Some(m)) if m >= 18)
}

pub fn is_sing_box_version_compatible(version_str: &str) -> bool {
    let clean = version_str.trim().trim_start_matches('v');
    let mut parts = clean.split('.');
    let major = parts.next().and_then(|s| s.parse::<u64>().ok());
    let minor = parts.next().and_then(|s| s.parse::<u64>().ok());
    matches!((major, minor), (Some(1), Some(13 | 14)))
}

pub fn ensure_compatible_core(core_path: &Path) -> AppResult<String> {
    if !core_path.exists() {
        return Err(AppError::CoreNotInstalled);
    }
    let ver = installed_version(core_path)
        .ok_or_else(|| AppError::CoreStartFailed("failed to probe core version".into()))?;
    let compatible = if is_mihomo(core_path) {
        is_mihomo_version_compatible(&ver)
    } else {
        is_sing_box_version_compatible(&ver)
    };
    if !compatible {
        return Err(AppError::Unsupported(format!(
            "installed core version {ver} is incompatible with Umbra (requires Mihomo v1.18+ or sing-box 1.13/1.14)"
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

pub async fn get_core_status(state: &AppState) -> AppResult<CoreStatus> {
    let engine = state.settings.read().await.core_engine.clone();
    let mihomo_path = core_path_for(&state.data_dir, "mihomo");
    let sing_box_path = core_path_for(&state.data_dir, "sing-box");
    let (mihomo_version, sing_box_version) = tokio::try_join!(
        probe_version(mihomo_path.clone()),
        probe_version(sing_box_path.clone()),
    )?;
    let path = core_path_for(&state.data_dir, &engine);
    let installed = path.exists();
    let version = if engine == "sing-box" {
        sing_box_version.clone()
    } else {
        mihomo_version.clone()
    };
    Ok(CoreStatus {
        engine,
        installed,
        version,
        path: path.to_string_lossy().into_owned(),
        mihomo_installed: mihomo_path.exists(),
        mihomo_version,
        sing_box_installed: sing_box_path.exists(),
        sing_box_version,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_version_output() {
        let out_sb = "sing-box version 1.11.9\n\nEnvironment: go1.23.4 windows/amd64\n";
        assert_eq!(parse_version_line(out_sb).as_deref(), Some("1.11.9"));

        let out_mih = "Mihomo Meta v1.19.33 windows amd64 with go1.26.10\n";
        assert_eq!(parse_version_line(out_mih).as_deref(), Some("v1.19.33"));
    }

    #[test]
    fn rejects_garbage_output() {
        assert!(parse_version_line("").is_none());
        assert!(parse_version_line("not sing-box").is_none());
        assert!(parse_version_line("sing-box version").is_none());
    }

    #[test]
    fn core_path_is_under_bin() {
        let mihomo = core_path_for(Path::new("C:\\data"), "mihomo");
        let sing_box = core_path_for(Path::new("C:\\data"), "sing-box");
        assert!(mihomo.ends_with("bin\\mihomo.exe") || mihomo.ends_with("bin/mihomo.exe"));
        assert!(sing_box.ends_with("bin\\sing-box.exe") || sing_box.ends_with("bin/sing-box.exe"));
    }

    #[test]
    fn missing_binary_yields_none() {
        assert!(installed_version(Path::new("C:\\definitely\\missing\\core.exe")).is_none());
    }
}
