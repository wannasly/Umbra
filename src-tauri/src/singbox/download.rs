//! sing-box core download from GitHub releases with progress + sha256 verify.

use std::path::{Path, PathBuf};
use std::time::Duration;

use futures_util::StreamExt;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tauri::Emitter;
use tokio::io::AsyncWriteExt;

use crate::error::{AppError, AppResult};
use crate::events::{DownloadPhase, DownloadProgress, EV_DOWNLOAD_PROGRESS};
use crate::models::UpdateCheck;
use crate::singbox::version::{core_path, probe_version};
use crate::state::AppState;

const API_RELEASES: &str = "https://api.github.com/repos/SagerNet/sing-box/releases?per_page=30";
const API_TAG: &str = "https://api.github.com/repos/SagerNet/sing-box/releases/tags";
const PROGRESS_STEP: u64 = 256 * 1024;

/// Resolved release: what to download and how to verify it.
#[derive(Debug, Clone)]
pub struct ReleaseInfo {
    /// X.Y.Z, no leading `v`
    pub version: String,
    /// browser_download_url, without mirror prefix
    pub url: String,
    pub size: u64,
    /// lowercase hex, from the GitHub asset `digest` field when present
    pub sha256: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GhRelease {
    assets: Vec<GhAsset>,
}

#[derive(Debug, Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
    #[serde(default)]
    size: u64,
    #[serde(default)]
    digest: Option<String>,
}

/// `sing-box-X.Y.Z-windows-amd64.zip` -> `X.Y.Z` (rejects -legacy and other arches).
fn asset_version(name: &str) -> Option<&str> {
    let v = name
        .strip_prefix("sing-box-")?
        .strip_suffix("-windows-amd64.zip")?;
    let mut parts = v.split('.');
    for _ in 0..3 {
        let p = parts.next()?;
        if p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
    }
    parts.next().is_none().then_some(v)
}

/// `sha256:HEX` -> lowercase hex.
fn digest_to_hex(digest: &str) -> Option<String> {
    let hex = digest.strip_prefix("sha256:")?.trim();
    (hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| hex.to_ascii_lowercase())
}

/// gh-proxy convention: mirror prefix + "/" + full original URL.
fn apply_mirror(mirror: &str, url: &str) -> String {
    let m = mirror.trim().trim_end_matches('/');
    if m.is_empty() {
        url.to_string()
    } else {
        format!("{m}/{url}")
    }
}

fn update_available(current: Option<&str>, latest: &str) -> bool {
    match current {
        None => true,
        Some(c) => match (semver::Version::parse(c), semver::Version::parse(latest)) {
            (Ok(cur), Ok(lat)) => lat > cur,
            _ => c != latest,
        },
    }
}

fn gh_client() -> AppResult<reqwest::Client> {
    reqwest::Client::builder()
        .no_proxy()
        .user_agent(concat!("Umbra/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| AppError::Network(e.to_string()))
}

async fn fetch_release(client: &reqwest::Client, tag: Option<&str>) -> AppResult<ReleaseInfo> {
    if let Some(t) = tag {
        let clean_v = t.trim().trim_start_matches('v');
        if !crate::singbox::version::is_version_compatible(clean_v) {
            return Err(AppError::Unsupported(format!(
                "version '{t}' is incompatible with Umbra (requires sing-box 1.13.x)"
            )));
        }
        let url = format!("{API_TAG}/{t}");
        let resp = client
            .get(&url)
            .timeout(Duration::from_secs(20))
            .header(reqwest::header::ACCEPT, "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .await?;
        let status = resp.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(AppError::NotFound(format!("sing-box release {t}")));
        }
        if !status.is_success() {
            return Err(AppError::Network(format!(
                "GitHub API returned HTTP {status}"
            )));
        }
        let release: GhRelease = resp.json().await?;
        let (version, asset) = release
            .assets
            .iter()
            .find_map(|a| asset_version(&a.name).map(|v| (v.to_string(), a)))
            .ok_or_else(|| AppError::NotFound("windows-amd64 asset in sing-box release".into()))?;
        return Ok(ReleaseInfo {
            version,
            url: asset.browser_download_url.clone(),
            size: asset.size,
            sha256: asset.digest.as_deref().and_then(digest_to_hex),
        });
    }

    // Iterate releases to find latest 1.13.x release
    let resp = client
        .get(API_RELEASES)
        .timeout(Duration::from_secs(20))
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(AppError::Network(format!(
            "GitHub API returned HTTP {}",
            resp.status()
        )));
    }
    let releases: Vec<GhRelease> = resp.json().await?;
    for release in releases {
        if let Some((version, asset)) = release
            .assets
            .iter()
            .find_map(|a| asset_version(&a.name).map(|v| (v.to_string(), a)))
        {
            if crate::singbox::version::is_version_compatible(&version) {
                return Ok(ReleaseInfo {
                    version,
                    url: asset.browser_download_url.clone(),
                    size: asset.size,
                    sha256: asset.digest.as_deref().and_then(digest_to_hex),
                });
            }
        }
    }
    Err(AppError::NotFound(
        "no compatible sing-box 1.13.x release found".into(),
    ))
}

/// Latest release, cached for the session (unauthenticated limit: 60 req/h).
async fn latest_release(state: &AppState) -> AppResult<ReleaseInfo> {
    {
        let cache = state.release_cache.lock().await;
        if let Some(info) = cache.as_ref() {
            return Ok(info.clone());
        }
    }
    let info = fetch_release(&gh_client()?, None).await?;
    *state.release_cache.lock().await = Some(info.clone());
    Ok(info)
}

fn emit_progress(app: &tauri::AppHandle, phase: DownloadPhase, downloaded: u64, total: u64) {
    if let Err(e) = app.emit(
        EV_DOWNLOAD_PROGRESS,
        DownloadProgress {
            phase,
            downloaded,
            total,
        },
    ) {
        eprintln!("[umbra] failed to emit {EV_DOWNLOAD_PROGRESS}: {e}");
    }
}

#[tauri::command]
pub async fn check_core_update(state: tauri::State<'_, AppState>) -> AppResult<UpdateCheck> {
    let latest = latest_release(state.inner()).await?;
    let current = probe_version(core_path(&state.data_dir)).await?;
    let available = update_available(current.as_deref(), &latest.version);
    Ok(UpdateCheck {
        current,
        latest: latest.version,
        update_available: available,
    })
}

#[tauri::command]
pub async fn download_core(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    version: Option<String>,
) -> AppResult<()> {
    let client = gh_client()?;
    let info = match version.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
        Some(v) => {
            let tag = if v.starts_with('v') {
                v.to_string()
            } else {
                format!("v{v}")
            };
            fetch_release(&client, Some(&tag)).await?
        }
        None => latest_release(state.inner()).await?,
    };

    let mirror = state.settings.read().await.github_mirror.clone();
    let url = apply_mirror(&mirror, &info.url);

    let bin_dir = state.data_dir.join("bin");
    tokio::fs::create_dir_all(&bin_dir).await?;
    let tmp = bin_dir.join("download.tmp");
    let target = core_path(&state.data_dir);

    let result = download_and_install(&app, &client, &url, &info, &bin_dir, &tmp, target).await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&tmp).await;
    }
    result
}

#[allow(clippy::too_many_arguments)]
async fn download_and_install(
    app: &tauri::AppHandle,
    client: &reqwest::Client,
    url: &str,
    info: &ReleaseInfo,
    bin_dir: &Path,
    tmp: &Path,
    target: PathBuf,
) -> AppResult<()> {
    let resp = client.get(url).send().await?;
    let status = resp.status();
    if !status.is_success() {
        return Err(AppError::Network(format!(
            "download server returned HTTP {status}"
        )));
    }
    let total = resp.content_length().unwrap_or(info.size);
    emit_progress(app, DownloadPhase::Download, 0, total);

    let mut file = tokio::fs::File::create(tmp).await?;
    let mut hasher = Sha256::new();
    let mut downloaded: u64 = 0;
    let mut last_emit: u64 = 0;
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| AppError::Network(format!("download interrupted: {e}")))?;
        hasher.update(&chunk);
        file.write_all(&chunk).await?;
        downloaded += chunk.len() as u64;
        if downloaded - last_emit >= PROGRESS_STEP {
            last_emit = downloaded;
            emit_progress(app, DownloadPhase::Download, downloaded, total);
        }
    }
    file.flush().await?;
    drop(file);
    emit_progress(app, DownloadPhase::Download, downloaded, total);

    if let Some(expected) = &info.sha256 {
        let actual: String = hasher
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        if &actual != expected {
            return Err(AppError::Internal(format!(
                "sha256 mismatch for downloaded core (expected {expected}, got {actual})"
            )));
        }
    }

    emit_progress(app, DownloadPhase::Extract, downloaded, total);

    let tmp_owned = tmp.to_path_buf();
    tokio::task::spawn_blocking(move || extract_core(&tmp_owned, &target))
        .await
        .map_err(|e| AppError::Internal(format!("extract task failed: {e}")))??;

    tokio::fs::write(bin_dir.join("version.txt"), &info.version).await?;
    let _ = tokio::fs::remove_file(tmp).await;
    emit_progress(app, DownloadPhase::Done, downloaded, total);
    Ok(())
}

fn is_core_entry(name: &str) -> bool {
    name == "sing-box.exe" || name.ends_with("/sing-box.exe") || name.ends_with("\\sing-box.exe")
}

fn extract_core(zip_path: &Path, target: &Path) -> AppResult<()> {
    let file = std::fs::File::open(zip_path)?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| AppError::Internal(format!("bad zip archive: {e}")))?;
    let entry_name = archive
        .file_names()
        .find(|n| is_core_entry(n))
        .map(str::to_string)
        .ok_or_else(|| AppError::Internal("sing-box.exe not found in the archive".into()))?;
    let mut entry = archive
        .by_name(&entry_name)
        .map_err(|e| AppError::Internal(format!("bad zip entry: {e}")))?;

    let mut staged_name = target.as_os_str().to_owned();
    staged_name.push(".new");
    let staged = PathBuf::from(staged_name);

    let result = stage_and_swap(&mut entry, &staged, target);
    if result.is_err() {
        let _ = std::fs::remove_file(&staged);
    }
    result
}

/// Unpack beside the target and only swap once the bytes are safely down: a
/// copy that dies midway (truncated archive, full disk) must not leave the user
/// with the old core deleted and no new one in its place.
fn stage_and_swap(entry: &mut impl std::io::Read, staged: &Path, target: &Path) -> AppResult<()> {
    let mut out = std::fs::File::create(staged)?;
    std::io::copy(entry, &mut out)?;
    out.sync_all()?;
    drop(out);

    if target.exists() {
        // Windows can't replace a running exe; surface a clear message.
        std::fs::remove_file(target).map_err(|e| {
            AppError::Internal(format!(
                "cannot replace sing-box.exe — stop the running core and retry ({e})"
            ))
        })?;
    }
    std::fs::rename(staged, target)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_name_matching() {
        assert_eq!(
            asset_version("sing-box-1.11.9-windows-amd64.zip"),
            Some("1.11.9")
        );
        assert_eq!(
            asset_version("sing-box-1.12.0-windows-amd64.zip"),
            Some("1.12.0")
        );
        assert!(asset_version("sing-box-1.11.9-windows-amd64-legacy.zip").is_none());
        assert!(asset_version("sing-box-1.11.9-legacy-windows-7-amd64.zip").is_none());
        assert!(asset_version("sing-box-1.11.9-linux-amd64.zip").is_none());
        assert!(asset_version("sing-box-1.11.9-windows-arm64.zip").is_none());
        assert!(asset_version("sing-box-1.11-windows-amd64.zip").is_none());
        assert!(asset_version("sing-box-1.11.9.1-windows-amd64.zip").is_none());
        assert!(asset_version("sing-box-1.11.x-windows-amd64.zip").is_none());
    }

    #[test]
    fn digest_parsing() {
        let hex = "a".repeat(64);
        assert_eq!(
            digest_to_hex(&format!("sha256:{hex}")).as_deref(),
            Some(hex.as_str())
        );
        let upper = "A".repeat(64);
        assert_eq!(digest_to_hex(&format!("sha256:{upper}")).unwrap(), hex);
        assert!(digest_to_hex("sha256:abcd").is_none()); // wrong length
        assert!(digest_to_hex("md5:abcd").is_none());
        assert!(digest_to_hex(&"z".repeat(64)).is_none());
    }

    #[test]
    fn mirror_prefixing() {
        let url = "https://github.com/SagerNet/sing-box/releases/download/v1.11.9/sing-box-1.11.9-windows-amd64.zip";
        assert_eq!(apply_mirror("", url), url);
        assert_eq!(apply_mirror("   ", url), url);
        assert_eq!(
            apply_mirror("https://gh-proxy.com/", url),
            format!("https://gh-proxy.com/{url}")
        );
        assert_eq!(
            apply_mirror("https://gh-proxy.com", url),
            format!("https://gh-proxy.com/{url}")
        );
    }

    #[test]
    fn update_compare() {
        assert!(update_available(None, "1.11.9"));
        assert!(update_available(Some("1.11.8"), "1.11.9"));
        assert!(update_available(Some("1.9.9"), "1.11.0"));
        assert!(!update_available(Some("1.11.9"), "1.11.9"));
        assert!(!update_available(Some("1.12.0"), "1.11.9"));
        // non-semver falls back to string inequality
        assert!(update_available(Some("weird"), "1.11.9"));
        assert!(!update_available(Some("weird"), "weird"));
    }

    #[test]
    fn core_entry_names() {
        assert!(is_core_entry("sing-box.exe"));
        assert!(is_core_entry("sing-box-1.11.9-windows-amd64/sing-box.exe"));
        assert!(is_core_entry("sing-box-1.11.9-windows-amd64\\sing-box.exe"));
        assert!(!is_core_entry("bin/sing-box.exe.sig"));
        assert!(!is_core_entry("LICENSE"));
    }

    #[test]
    fn extract_finds_nested_core_and_replaces_existing() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("umbra-extract-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let zip_path = dir.join("core.zip");
        {
            let f = std::fs::File::create(&zip_path).unwrap();
            let mut w = zip::ZipWriter::new(f);
            let opts = zip::write::SimpleFileOptions::default();
            w.start_file("sing-box-1.11.9-windows-amd64/LICENSE", opts)
                .unwrap();
            w.write_all(b"mit").unwrap();
            w.start_file("sing-box-1.11.9-windows-amd64/sing-box.exe", opts)
                .unwrap();
            w.write_all(b"exe-bytes").unwrap();
            w.finish().unwrap();
        }
        let target = dir.join("sing-box.exe");
        extract_core(&zip_path, &target).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"exe-bytes");
        extract_core(&zip_path, &target).unwrap(); // replace path
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn extract_without_core_entry_fails() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("umbra-nocore-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let zip_path = dir.join("core.zip");
        {
            let f = std::fs::File::create(&zip_path).unwrap();
            let mut w = zip::ZipWriter::new(f);
            let opts = zip::write::SimpleFileOptions::default();
            w.start_file("LICENSE", opts).unwrap();
            w.write_all(b"mit").unwrap();
            w.finish().unwrap();
        }
        let err = extract_core(&zip_path, &dir.join("sing-box.exe")).unwrap_err();
        assert_eq!(err.code(), "INTERNAL");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A failed extract must leave the installed core alone: deleting it before
    /// the new bytes are down would strand the user with no core at all.
    #[test]
    fn failed_extract_keeps_the_installed_core_and_leaves_no_staging_file() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("umbra-keep-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let zip_path = dir.join("core.zip");
        {
            let f = std::fs::File::create(&zip_path).unwrap();
            let mut w = zip::ZipWriter::new(f);
            let opts = zip::write::SimpleFileOptions::default();
            w.start_file("LICENSE", opts).unwrap();
            w.write_all(b"mit").unwrap();
            w.finish().unwrap();
        }
        let target = dir.join("sing-box.exe");
        std::fs::write(&target, b"working-core").unwrap();

        assert!(extract_core(&zip_path, &target).is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"working-core");
        assert!(!dir.join("sing-box.exe.new").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The staging file is a swap step, not a leftover.
    #[test]
    fn successful_extract_leaves_no_staging_file() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("umbra-swap-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let zip_path = dir.join("core.zip");
        {
            let f = std::fs::File::create(&zip_path).unwrap();
            let mut w = zip::ZipWriter::new(f);
            let opts = zip::write::SimpleFileOptions::default();
            w.start_file("sing-box.exe", opts).unwrap();
            w.write_all(b"fresh").unwrap();
            w.finish().unwrap();
        }
        let target = dir.join("sing-box.exe");
        std::fs::write(&target, b"stale").unwrap();

        extract_core(&zip_path, &target).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"fresh");
        assert!(!dir.join("sing-box.exe.new").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
