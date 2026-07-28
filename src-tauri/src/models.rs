//! Data models persisted to %APPDATA% and exchanged with the frontend.
//! Shapes must match src/lib/ipc.ts exactly (camelCase on the wire).

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Connection
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    SystemProxy,
    Tun,
}

impl Default for Mode {
    fn default() -> Self {
        Mode::SystemProxy
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RouteTarget {
    Proxy,
    Direct,
}

impl Default for RouteTarget {
    fn default() -> Self {
        RouteTarget::Proxy
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AppRouteAction {
    Proxy,
    Direct,
    Block,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppRouteRule {
    pub id: String,
    pub process_name: String,
    pub action: AppRouteAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConnStatus {
    Disconnected,
    Connecting,
    Connected,
    Stopping,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionState {
    pub status: ConnStatus,
    pub server_id: Option<String>,
    /// Name of the server the tunnel is actually running against, snapshotted
    /// when the connection was established. It outlives the entry in
    /// `ProfileStore`, so deleting a subscription mid-session can never leave
    /// the UI claiming "connected" and "no server" at the same time.
    pub server_name: Option<String>,
    pub mode: Mode,
    pub since_ms: Option<i64>,
    pub error: Option<String>,
}

impl ConnectionState {
    pub fn disconnected(mode: Mode) -> Self {
        Self {
            status: ConnStatus::Disconnected,
            server_id: None,
            server_name: None,
            mode,
            since_ms: None,
            error: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Servers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Transport {
    Tcp,
    Ws {
        path: String,
        host: String,
    },
    Grpc {
        #[serde(rename = "serviceName")]
        service_name: String,
    },
    Httpupgrade {
        path: String,
        host: String,
    },
}

impl Default for Transport {
    fn default() -> Self {
        Transport::Tcp
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Security {
    Reality,
    Tls,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Hysteria2Obfs {
    Salamander { password: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VlessNode {
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hysteria2Node {
    pub password: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub obfs: Option<Hysteria2Obfs>,
    #[serde(default)]
    pub insecure: bool,
    #[serde(default)]
    pub sni: String,
    #[serde(default)]
    pub alpn: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "protocol",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ProxyKind {
    #[serde(rename = "vless")]
    Vless(VlessNode),
    #[serde(rename = "hysteria2")]
    Hysteria2(Hysteria2Node),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyNode {
    pub id: String,
    pub name: String,
    pub server: String,
    pub port: u16,

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

    #[serde(flatten)]
    pub kind: ProxyKind,
}

pub type ServerEntry = ProxyNode;

// ---------------------------------------------------------------------------
// Subscriptions
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubscriptionQuota {
    pub upload: u64,
    pub download: u64,
    pub total: u64,
    /// unix seconds, 0 = never
    pub expire: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Subscription {
    pub id: String,
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub updated_at: Option<String>,
    #[serde(default)]
    pub quota: Option<SubscriptionQuota>,
    #[serde(default)]
    pub auto_update_hours: u32,
    /// `Support-Url` header — the provider's support contact, when advertised.
    #[serde(default)]
    pub support_url: Option<String>,
    /// `Profile-Web-Page-Url` header — the provider's account page.
    #[serde(default)]
    pub web_page_url: Option<String>,
    /// The panel's own title (`Profile-Title`), kept so a later refresh can
    /// tell an auto-derived name from one the user typed.
    #[serde(default)]
    pub panel_title: Option<String>,
    #[serde(default)]
    pub servers: Vec<ProxyNode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileStore {
    #[serde(default = "default_store_version")]
    pub version: u32,
    #[serde(default)]
    pub manual: Vec<ProxyNode>,
    #[serde(default)]
    pub subscriptions: Vec<Subscription>,
}

fn default_store_version() -> u32 {
    2
}

impl Default for ProfileStore {
    fn default() -> Self {
        Self {
            version: default_store_version(),
            manual: Vec::new(),
            subscriptions: Vec::new(),
        }
    }
}

impl ProfileStore {
    pub fn all_servers(&self) -> impl Iterator<Item = &ServerEntry> {
        self.manual
            .iter()
            .chain(self.subscriptions.iter().flat_map(|s| s.servers.iter()))
    }

    pub fn find_server(&self, id: &str) -> Option<&ServerEntry> {
        self.all_servers().find(|s| s.id == id)
    }

    pub fn find_server_mut(&mut self, id: &str) -> Option<&mut ServerEntry> {
        self.manual
            .iter_mut()
            .chain(
                self.subscriptions
                    .iter_mut()
                    .flat_map(|s| s.servers.iter_mut()),
            )
            .find(|s| s.id == id)
    }
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyBackup {
    pub enable: u32,
    #[serde(default)]
    pub server: Option<String>,
    #[serde(default, rename = "override")]
    pub bypass_list: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IpStrategy {
    Ipv4Only,
    PreferIpv4,
    PreferIpv6,
    Ipv6Only,
}

impl Default for IpStrategy {
    fn default() -> Self {
        IpStrategy::Ipv4Only
    }
}

impl IpStrategy {
    pub fn as_str(&self) -> &'static str {
        match self {
            IpStrategy::Ipv4Only => "ipv4_only",
            IpStrategy::PreferIpv4 => "prefer_ipv4",
            IpStrategy::PreferIpv6 => "prefer_ipv6",
            IpStrategy::Ipv6Only => "ipv6_only",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub version: u32,
    pub language: String,
    pub accent: String,
    pub mode: Mode,
    pub mixed_port: u16,
    pub selected_server_id: Option<String>,
    pub autostart: bool,
    pub start_minimized: bool,
    pub minimize_to_tray: bool,
    pub connect_on_startup: bool,
    pub log_level: String,
    pub bypass_ru: bool,
    /// Fallback for traffic that did not match an application or geo rule.
    pub route_default: RouteTarget,
    /// Per-process split-tunnelling rules, evaluated before generic routes.
    pub app_routes: Vec<AppRouteRule>,
    pub tun_stack: String,
    pub tun_strict_route: bool,
    pub tun_mtu: u32,
    pub ip_strategy: IpStrategy,
    pub ping_url: String,
    pub reduce_motion: bool,
    /// Server-list ordering: "default" (as delivered), "ping" or "name".
    pub server_sort: String,
    /// Group keys the user collapsed on the Servers page — subscription ids
    /// plus the two synthetic groups ("favorites", "manual"). Kept here rather
    /// than on `Subscription` so all three kinds of group persist the same way.
    pub collapsed_groups: Vec<String>,
    pub github_mirror: String,
    /// User-Agent used when fetching subscriptions. Panels serve different
    /// formats (and sometimes only serve whitelisted clients) based on it.
    pub sub_user_agent: String,
    /// Send x-hwid / x-device-* headers for panels enforcing a device limit.
    pub send_hwid: bool,
    /// Cached hardware id; regenerated when empty.
    pub hwid: String,
    /// true while we own the Windows proxy settings (crash-recovery flag)
    pub proxy_owned: bool,
    pub proxy_backup: ProxyBackup,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: 1,
            language: "ru".into(),
            accent: "violet".into(),
            mode: Mode::SystemProxy,
            mixed_port: 2080,
            selected_server_id: None,
            autostart: false,
            start_minimized: false,
            minimize_to_tray: true,
            connect_on_startup: false,
            log_level: "info".into(),
            bypass_ru: false,
            route_default: RouteTarget::Proxy,
            app_routes: Vec::new(),
            tun_stack: "mixed".into(),
            tun_strict_route: true,
            tun_mtu: 9000,
            ip_strategy: IpStrategy::Ipv4Only,
            ping_url: "https://www.gstatic.com/generate_204".into(),
            reduce_motion: false,
            server_sort: "default".into(),
            collapsed_groups: Vec::new(),
            github_mirror: String::new(),
            sub_user_agent: crate::subscription::DEFAULT_SUB_USER_AGENT.into(),
            send_hwid: true,
            hwid: String::new(),
            proxy_owned: false,
            proxy_backup: ProxyBackup::default(),
        }
    }
}

// ---------------------------------------------------------------------------
// Core status
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoreStatus {
    pub installed: bool,
    pub version: Option<String>,
    pub path: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheck {
    pub current: Option<String>,
    pub latest: String,
    pub update_available: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub added: usize,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServersList {
    pub manual: Vec<ServerEntry>,
    pub subscriptions: Vec<Subscription>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn proxy_node_vless_serializes_flat() {
        let node = ProxyNode {
            id: "1".into(),
            name: "test-vless".into(),
            server: "1.2.3.4".into(),
            port: 443,
            last_ping_ms: None,
            favorite: false,
            total_up: 0,
            total_down: 0,
            raw: "vless://...".into(),
            kind: ProxyKind::Vless(VlessNode {
                uuid: "uuid-123".into(),
                flow: "xtls-rprx-vision".into(),
                security: Security::Reality,
                sni: "example.com".into(),
                fingerprint: "chrome".into(),
                public_key: "pbk123".into(),
                short_id: "sid123".into(),
                insecure: false,
                alpn: vec![],
                transport: Transport::Tcp,
            }),
        };
        let val = serde_json::to_value(&node).unwrap();
        assert_eq!(val["id"], "1");
        assert_eq!(val["name"], "test-vless");
        assert_eq!(val["protocol"], "vless");
        assert_eq!(val["uuid"], "uuid-123");
        assert_eq!(val["publicKey"], "pbk123");
        assert_eq!(val["shortId"], "sid123");
        assert!(val.get("kind").is_none());
    }

    #[test]
    fn proxy_node_hysteria2_serializes_flat() {
        let node = ProxyNode {
            id: "2".into(),
            name: "test-hy2".into(),
            server: "5.6.7.8".into(),
            port: 443,
            last_ping_ms: None,
            favorite: true,
            total_up: 100,
            total_down: 200,
            raw: "hy2://...".into(),
            kind: ProxyKind::Hysteria2(Hysteria2Node {
                password: "secret_password".into(),
                obfs: Some(Hysteria2Obfs::Salamander {
                    password: "obfs_pass".into(),
                }),
                insecure: true,
                sni: "hy2.example.com".into(),
                alpn: vec!["h3".into()],
            }),
        };
        let val = serde_json::to_value(&node).unwrap();
        assert_eq!(val["id"], "2");
        assert_eq!(val["protocol"], "hysteria2");
        assert_eq!(val["password"], "secret_password");
        assert_eq!(
            val["obfs"],
            json!({ "type": "salamander", "password": "obfs_pass" })
        );
        assert_eq!(val["insecure"], true);
        assert_eq!(val["sni"], "hy2.example.com");
        assert!(val.get("kind").is_none());
    }
}
