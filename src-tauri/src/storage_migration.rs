use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::time::SystemTime;

use crate::error::{AppError, AppResult};
use crate::models::{ProfileStore, ProxyKind, ProxyNode, Security, Transport, VlessNode};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProfileStoreV1 {
    #[serde(default = "v1_version")]
    version: u32,
    #[serde(default)]
    manual: Vec<ProxyNodeV1>,
    #[serde(default)]
    subscriptions: Vec<SubscriptionV1>,
}

fn v1_version() -> u32 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubscriptionV1 {
    pub id: String,
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub updated_at: Option<String>,
    #[serde(default)]
    pub quota: Option<crate::models::SubscriptionQuota>,
    #[serde(default)]
    pub auto_update_hours: u32,
    #[serde(default)]
    pub support_url: Option<String>,
    #[serde(default)]
    pub web_page_url: Option<String>,
    #[serde(default)]
    pub panel_title: Option<String>,
    #[serde(default)]
    pub servers: Vec<ProxyNodeV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProxyNodeV1 {
    pub id: String,
    pub name: String,
    #[serde(default = "vless_protocol")]
    pub protocol: String,
    pub server: String,
    pub port: u16,
    pub uuid: String,
    #[serde(default)]
    pub flow: String,
    pub security: Security,
    #[serde(default)]
    pub sni: String,
    #[serde(default)]
    pub fingerprint: String,
    #[serde(default)]
    pub public_key: String,
    #[serde(default)]
    pub short_id: String,
    #[serde(default)]
    pub insecure: bool,
    #[serde(default)]
    pub alpn: Vec<String>,
    #[serde(default)]
    pub transport: Transport,
    #[serde(default)]
    pub last_ping_ms: Option<u32>,
    #[serde(default)]
    pub favorite: bool,
    #[serde(default)]
    pub total_up: u64,
    #[serde(default)]
    pub total_down: u64,
    #[serde(default)]
    pub raw: String,
}

fn vless_protocol() -> String {
    "vless".into()
}

fn convert_v1_node(v1: ProxyNodeV1) -> ProxyNode {
    ProxyNode {
        id: v1.id,
        name: v1.name,
        server: v1.server,
        port: v1.port,
        last_ping_ms: v1.last_ping_ms,
        favorite: v1.favorite,
        total_up: v1.total_up,
        total_down: v1.total_down,
        raw: v1.raw,
        kind: ProxyKind::Vless(VlessNode {
            uuid: v1.uuid,
            flow: v1.flow,
            security: v1.security,
            sni: v1.sni,
            fingerprint: v1.fingerprint,
            public_key: v1.public_key,
            short_id: v1.short_id,
            insecure: v1.insecure,
            alpn: v1.alpn,
            transport: v1.transport,
        }),
    }
}

pub fn load_and_migrate_profiles(path: &Path) -> AppResult<ProfileStore> {
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(ProfileStore::default()),
        Err(e) => return Err(e.into()),
    };

    let raw_val: serde_json::Value = match serde_json::from_slice(&bytes) {
        Ok(val) => val,
        Err(e) => {
            eprintln!("[umbra] corrupt JSON in {}: {e}", path.display());
            create_timestamped_backup(path, &bytes, "corrupt");
            return Err(AppError::Internal(format!(
                "corrupt json in {}: {e}",
                path.display()
            )));
        }
    };

    let version = raw_val.get("version").and_then(|v| v.as_u64()).unwrap_or(1) as u32;

    if version > 2 {
        let msg = format!(
            "[umbra] unknown ProfileStore version {version} in {}",
            path.display()
        );
        eprintln!("{msg}");
        return Err(AppError::Internal(msg));
    }

    if version == 2 {
        match serde_json::from_value::<ProfileStore>(raw_val) {
            Ok(store) => return Ok(store),
            Err(e) => {
                eprintln!("[umbra] failed to parse ProfileStore v2: {e}");
                create_timestamped_backup(path, &bytes, "corrupt");
                return Err(AppError::Internal(format!("failed to parse v2: {e}")));
            }
        }
    }

    // version == 1: Migrate V1 -> V2
    match serde_json::from_value::<ProfileStoreV1>(raw_val) {
        Ok(v1_store) => {
            // Backup valid V1 before migrating
            let backup_path = path.with_extension("v1.bak.json");
            let _ = fs::write(&backup_path, &bytes);

            let v2_store = ProfileStore {
                version: 2,
                manual: v1_store.manual.into_iter().map(convert_v1_node).collect(),
                subscriptions: v1_store
                    .subscriptions
                    .into_iter()
                    .map(|sub| crate::models::Subscription {
                        id: sub.id,
                        name: sub.name,
                        url: sub.url,
                        updated_at: sub.updated_at,
                        quota: sub.quota,
                        auto_update_hours: sub.auto_update_hours,
                        support_url: sub.support_url,
                        web_page_url: sub.web_page_url,
                        panel_title: sub.panel_title,
                        servers: sub.servers.into_iter().map(convert_v1_node).collect(),
                    })
                    .collect(),
            };

            // Save migrated V2 store
            crate::storage::write_json(path, &v2_store)?;

            Ok(v2_store)
        }
        Err(e) => {
            eprintln!("[umbra] failed to parse ProfileStore v1: {e}");
            create_timestamped_backup(path, &bytes, "corrupt");
            Err(AppError::Internal(format!("failed to parse v1: {e}")))
        }
    }
}

fn create_timestamped_backup(path: &Path, bytes: &[u8], tag: &str) {
    let ts = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("profiles.json");
    let stem = filename.strip_suffix(".json").unwrap_or(filename);
    let backup_name = format!("{}.{}.{}.json", stem, tag, ts);
    let backup_path = path.parent().unwrap_or(Path::new(".")).join(backup_name);
    let _ = fs::write(&backup_path, bytes);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn scratch_dir() -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("umbra-migration-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn migrates_v1_manual_servers_to_v2() {
        let dir = scratch_dir();
        let path = dir.join("profiles.json");
        let v1_json = json!({
            "version": 1,
            "manual": [{
                "id": "m1",
                "name": "Manual VLESS",
                "protocol": "vless",
                "server": "1.1.1.1",
                "port": 443,
                "uuid": "uuid-1",
                "flow": "xtls-rprx-vision",
                "security": "reality",
                "sni": "example.com",
                "fingerprint": "chrome",
                "publicKey": "pbk",
                "shortId": "sid",
                "insecure": false,
                "alpn": [],
                "transport": { "type": "tcp" },
                "lastPingMs": 42,
                "favorite": true,
                "totalUp": 100,
                "totalDown": 200,
                "raw": "vless://..."
            }],
            "subscriptions": []
        });
        fs::write(&path, serde_json::to_vec_pretty(&v1_json).unwrap()).unwrap();

        let store = load_and_migrate_profiles(&path).unwrap();
        assert_eq!(store.version, 2);
        assert_eq!(store.manual.len(), 1);

        let node = &store.manual[0];
        assert_eq!(node.id, "m1");
        assert_eq!(node.name, "Manual VLESS");
        assert_eq!(node.server, "1.1.1.1");
        assert_eq!(node.port, 443);

        let ProxyKind::Vless(ref v) = node.kind else {
            panic!("expected Vless")
        };
        assert_eq!(v.uuid, "uuid-1");
        assert_eq!(v.flow, "xtls-rprx-vision");
        assert_eq!(v.security, Security::Reality);
        assert_eq!(v.sni, "example.com");

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn migrates_v1_subscription_servers_to_v2() {
        let dir = scratch_dir();
        let path = dir.join("profiles.json");
        let v1_json = json!({
            "version": 1,
            "manual": [],
            "subscriptions": [{
                "id": "sub1",
                "name": "Sub 1",
                "url": "https://example.com/sub",
                "autoUpdateHours": 12,
                "servers": [{
                    "id": "s1",
                    "name": "Server 1",
                    "protocol": "vless",
                    "server": "2.2.2.2",
                    "port": 8443,
                    "uuid": "uuid-2",
                    "security": "none",
                    "transport": { "type": "tcp" }
                }]
            }]
        });
        fs::write(&path, serde_json::to_vec_pretty(&v1_json).unwrap()).unwrap();

        let store = load_and_migrate_profiles(&path).unwrap();
        assert_eq!(store.version, 2);
        assert_eq!(store.subscriptions.len(), 1);

        let sub = &store.subscriptions[0];
        assert_eq!(sub.id, "sub1");
        assert_eq!(sub.servers.len(), 1);
        assert_eq!(sub.servers[0].id, "s1");

        let ProxyKind::Vless(ref v) = sub.servers[0].kind else {
            panic!("expected Vless")
        };
        assert_eq!(v.uuid, "uuid-2");

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn preserves_v1_server_metadata() {
        let dir = scratch_dir();
        let path = dir.join("profiles.json");
        let v1_json = json!({
            "version": 1,
            "manual": [{
                "id": "m1",
                "name": "Meta Test",
                "server": "1.1.1.1",
                "port": 443,
                "uuid": "u1",
                "security": "none",
                "lastPingMs": 99,
                "favorite": true,
                "totalUp": 12345,
                "totalDown": 67890,
                "raw": "vless://raw-link"
            }],
            "subscriptions": []
        });
        fs::write(&path, serde_json::to_vec_pretty(&v1_json).unwrap()).unwrap();

        let store = load_and_migrate_profiles(&path).unwrap();
        let node = &store.manual[0];
        assert_eq!(node.last_ping_ms, Some(99));
        assert!(node.favorite);
        assert_eq!(node.total_up, 12345);
        assert_eq!(node.total_down, 67890);
        assert_eq!(node.raw, "vless://raw-link");

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn preserves_v1_subscription_metadata() {
        let dir = scratch_dir();
        let path = dir.join("profiles.json");
        let v1_json = json!({
            "version": 1,
            "manual": [],
            "subscriptions": [{
                "id": "sub1",
                "name": "Sub Title",
                "url": "https://example.com/sub",
                "updatedAt": "2026-01-01T00:00:00Z",
                "autoUpdateHours": 6,
                "supportUrl": "https://support.com",
                "webPageUrl": "https://panel.com",
                "panelTitle": "Original Title",
                "servers": []
            }]
        });
        fs::write(&path, serde_json::to_vec_pretty(&v1_json).unwrap()).unwrap();

        let store = load_and_migrate_profiles(&path).unwrap();
        let sub = &store.subscriptions[0];
        assert_eq!(sub.updated_at.as_deref(), Some("2026-01-01T00:00:00Z"));
        assert_eq!(sub.auto_update_hours, 6);
        assert_eq!(sub.support_url.as_deref(), Some("https://support.com"));
        assert_eq!(sub.web_page_url.as_deref(), Some("https://panel.com"));
        assert_eq!(sub.panel_title.as_deref(), Some("Original Title"));

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn creates_v1_backup_before_migration() {
        let dir = scratch_dir();
        let path = dir.join("profiles.json");
        let v1_json = json!({
            "version": 1,
            "manual": [],
            "subscriptions": []
        });
        fs::write(&path, serde_json::to_vec_pretty(&v1_json).unwrap()).unwrap();

        let _ = load_and_migrate_profiles(&path).unwrap();

        let backup_path = dir.join("profiles.v1.bak.json");
        assert!(backup_path.exists(), "v1 backup file must be created");

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn corrupt_json_creates_timestamped_backup() {
        let dir = scratch_dir();
        let path = dir.join("profiles.json");
        fs::write(&path, b"{ invalid json syntax !!").unwrap();

        let res = load_and_migrate_profiles(&path);
        assert!(res.is_err());

        let backups: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.contains("corrupt"))
            .collect();
        assert_eq!(backups.len(), 1, "corrupt backup file must be created");

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn unknown_future_version_is_not_overwritten() {
        let dir = scratch_dir();
        let path = dir.join("profiles.json");
        let future_json = json!({
            "version": 999,
            "futureField": "important data"
        });
        let raw_bytes = serde_json::to_vec_pretty(&future_json).unwrap();
        fs::write(&path, &raw_bytes).unwrap();

        let res = load_and_migrate_profiles(&path);
        assert!(res.is_err(), "future version must return error");

        // The original file on disk MUST NOT be changed or deleted
        let on_disk = fs::read(&path).unwrap();
        assert_eq!(on_disk, raw_bytes, "original file must be untouched");

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn v2_profiles_roundtrip() {
        let dir = scratch_dir();
        let path = dir.join("profiles.json");
        let store_v2 = ProfileStore {
            version: 2,
            manual: vec![ProxyNode {
                id: "p1".into(),
                name: "V2 Node".into(),
                server: "1.1.1.1".into(),
                port: 443,
                last_ping_ms: None,
                favorite: false,
                total_up: 0,
                total_down: 0,
                raw: "".into(),
                kind: ProxyKind::Vless(VlessNode {
                    uuid: "u1".into(),
                    flow: "".into(),
                    security: Security::None,
                    sni: "".into(),
                    fingerprint: "".into(),
                    public_key: "".into(),
                    short_id: "".into(),
                    insecure: false,
                    alpn: vec![],
                    transport: Transport::Tcp,
                }),
            }],
            subscriptions: vec![],
        };

        crate::storage::write_json(&path, &store_v2).unwrap();
        let loaded = load_and_migrate_profiles(&path).unwrap();
        assert_eq!(loaded.version, 2);
        assert_eq!(loaded.manual.len(), 1);
        assert_eq!(loaded.manual[0].id, "p1");

        fs::remove_dir_all(&dir).unwrap();
    }
}
