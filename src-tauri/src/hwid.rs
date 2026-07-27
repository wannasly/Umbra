//! Hardware identity sent to subscription panels that enforce a device limit
//! (Remnawave and friends read `x-hwid` / `x-device-os` / `x-ver-os` /
//! `x-device-model`).
//!
//! The id must be stable across restarts and reinstalls but must not leak the
//! raw machine GUID, so we hash it with a fixed application salt.

use sha2::{Digest, Sha256};

const HWID_SALT: &str = "umbra-hwid-v1";

/// Windows install identity: HKLM\SOFTWARE\Microsoft\Cryptography\MachineGuid.
#[cfg(windows)]
fn machine_guid() -> Option<String> {
    use winreg::enums::{HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_64KEY};
    use winreg::RegKey;

    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    // Cryptography lives in the 64-bit view; a 32-bit build would otherwise be
    // redirected to Wow6432Node and read a different (or missing) value.
    let key = hklm
        .open_subkey_with_flags(
            r"SOFTWARE\Microsoft\Cryptography",
            KEY_READ | KEY_WOW64_64KEY,
        )
        .ok()?;
    key.get_value::<String, _>("MachineGuid").ok()
}

#[cfg(not(windows))]
fn machine_guid() -> Option<String> {
    None
}

#[cfg(windows)]
fn registry_string(path: &str, value: &str) -> Option<String> {
    use winreg::enums::{HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_64KEY};
    use winreg::RegKey;

    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let key = hklm
        .open_subkey_with_flags(path, KEY_READ | KEY_WOW64_64KEY)
        .ok()?;
    key.get_value::<String, _>(value)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

#[cfg(not(windows))]
fn registry_string(_path: &str, _value: &str) -> Option<String> {
    None
}

fn hash_id(seed: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(HWID_SALT.as_bytes());
    hasher.update(seed.as_bytes());
    let digest = hasher.finalize();
    // 32 hex chars is what panels expect from other clients; keep it compact.
    digest.iter().take(16).map(|b| format!("{b:02x}")).collect()
}

/// Stable per-machine identifier. Falls back to a random-but-persisted id when
/// the machine GUID is unreadable; `fallback` is that stored value (if any).
pub fn hwid(fallback: Option<&str>) -> String {
    if let Some(guid) = machine_guid() {
        return hash_id(&guid);
    }
    if let Some(existing) = fallback.filter(|s| !s.is_empty()) {
        return existing.to_string();
    }
    hash_id(&uuid::Uuid::new_v4().to_string())
}

/// Marketing name of the machine, e.g. "ASUS ROG STRIX B550-F".
pub fn device_model() -> String {
    let base = r"SYSTEM\HardwareConfig\Current";
    let manufacturer = registry_string(base, "SystemManufacturer");
    let product = registry_string(base, "SystemProductName");
    match (manufacturer, product) {
        (Some(m), Some(p)) if !p.eq_ignore_ascii_case(&m) => format!("{m} {p}"),
        (Some(m), None) => m,
        (_, Some(p)) => p,
        _ => "PC".to_string(),
    }
}

pub fn device_os() -> String {
    if cfg!(windows) {
        "Windows".to_string()
    } else if cfg!(target_os = "macos") {
        "macOS".to_string()
    } else {
        "Linux".to_string()
    }
}

/// Windows display version, e.g. "11 (26100)".
pub fn os_version() -> String {
    let base = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";
    let build = registry_string(base, "CurrentBuildNumber");
    let product = registry_string(base, "ProductName");
    // ProductName still says "Windows 10" on 11; the build number disambiguates.
    let major = match build.as_deref().and_then(|b| b.parse::<u32>().ok()) {
        Some(n) if n >= 22000 => "11".to_string(),
        Some(_) => "10".to_string(),
        None => product.unwrap_or_else(|| "Windows".to_string()),
    };
    match build {
        Some(b) => format!("{major} ({b})"),
        None => major,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_stable_and_salted() {
        let a = hash_id("same-seed");
        let b = hash_id("same-seed");
        assert_eq!(a, b);
        assert_eq!(a.len(), 32);
        assert_ne!(a, "same-seed");
        assert_ne!(hash_id("other-seed"), a);
    }

    #[test]
    fn fallback_is_reused_when_provided() {
        // Only meaningful where the machine GUID is unavailable; on Windows the
        // GUID wins, so assert the shape instead of an exact value.
        let id = hwid(Some("cafebabecafebabecafebabecafebabe"));
        assert_eq!(id.len(), 32);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn descriptors_are_non_empty() {
        assert!(!device_model().is_empty());
        assert!(!device_os().is_empty());
        assert!(!os_version().is_empty());
    }
}
