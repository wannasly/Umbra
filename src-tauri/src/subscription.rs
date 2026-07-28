//! Subscription fetching, body decoding and server-list merging.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use base64::engine::general_purpose::{STANDARD, URL_SAFE};
use base64::Engine;
use percent_encoding::percent_decode_str;

use crate::error::{AppError, AppResult};
use crate::models::{ServerEntry, SubscriptionQuota};
use crate::parser;

/// Panels sniff the UA and serve different formats to clash/sing-box clients;
/// impersonate a plain v2rayN to always get the URI list. Overridable in
/// settings because some panels only serve whitelisted clients.
pub const DEFAULT_SUB_USER_AGENT: &str = "v2rayN/7.13 Umbra/0.1.0";

/// Remnawave-style panels do not fail a device-gated request: they answer
/// HTTP 200 with a single placeholder entry (`0.0.0.0:1`, named e.g.
/// "Приложение не поддерживается") and state the reason in a header. Without
/// this check the placeholder parses fine and is stored as a real server, so
/// the user gets a list of one that silently fails to connect.
const H_HWID_NOT_SUPPORTED: &str = "x-hwid-not-supported";
const H_HWID_MAX_DEVICES: &str = "x-hwid-max-devices-reached";

pub struct FetchedSubscription {
    pub servers: Vec<ServerEntry>,
    pub errors: Vec<String>,
    pub quota: Option<SubscriptionQuota>,
    /// From Content-Disposition; the panel's *file* name, which in practice is
    /// an account id ("account-123"). Only a fallback — see `title`.
    pub filename: Option<String>,
    /// From `Profile-Title`: the human name the panel wants shown
    /// ("Example Network"). Preferred over `filename`.
    pub title: Option<String>,
    /// From `Profile-Update-Interval`, in hours.
    pub update_interval_hours: Option<u32>,
    /// From `Support-Url`.
    pub support_url: Option<String>,
    /// From `Profile-Web-Page-Url`.
    pub web_page_url: Option<String>,
}

/// Identity headers for panels that enforce a per-device limit.
#[derive(Debug, Clone, Default)]
pub struct DeviceIdentity {
    pub hwid: String,
    pub os: String,
    pub os_version: String,
    pub model: String,
}

pub async fn fetch_subscription(
    url: &str,
    user_agent: &str,
    identity: Option<&DeviceIdentity>,
) -> AppResult<FetchedSubscription> {
    let ua = if user_agent.trim().is_empty() {
        DEFAULT_SUB_USER_AGENT
    } else {
        user_agent.trim()
    };
    let client = reqwest::Client::builder()
        .no_proxy()
        .user_agent(ua)
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| AppError::Network(e.to_string()))?;
    let mut request = client.get(url);
    if let Some(id) = identity {
        request = request
            .header("x-hwid", &id.hwid)
            .header("x-device-os", &id.os)
            .header("x-ver-os", &id.os_version)
            .header("x-device-model", &id.model);
    }
    let resp = request.send().await?;
    let status = resp.status();
    if !status.is_success() {
        return Err(AppError::Network(format!(
            "subscription server returned HTTP {status}"
        )));
    }
    let gate = hwid_rejection(resp.headers());
    let quota = resp
        .headers()
        .get("subscription-userinfo")
        .and_then(|v| v.to_str().ok())
        .and_then(parse_userinfo);
    let filename = resp
        .headers()
        .get(reqwest::header::CONTENT_DISPOSITION)
        .and_then(|v| v.to_str().ok())
        .and_then(filename_from_disposition);
    let header = |name: &str| {
        resp.headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_string)
    };
    let title = header(H_PROFILE_TITLE).and_then(|v| decode_header_text(&v));
    let update_interval_hours = header(H_UPDATE_INTERVAL).and_then(|v| v.parse::<u32>().ok());
    let support_url = header(H_SUPPORT_URL).filter(|u| is_http_url(u));
    let web_page_url = header(H_WEB_PAGE_URL).filter(|u| is_http_url(u));
    let body = resp.text().await?;
    let list = decode_body(&body)?;
    let (parsed, errors) = parser::parse_links(&list);
    let servers = drop_placeholders(parsed, gate)?;

    Ok(FetchedSubscription {
        servers,
        errors,
        quota,
        filename,
        title,
        update_interval_hours,
        support_url,
        web_page_url,
    })
}

/// Headers every Remnawave/Marzban-style panel sends alongside the list. The
/// user-visible name lives in `Profile-Title`; `Content-Disposition` only
/// carries an account number, which is what used to end up on the card.
const H_PROFILE_TITLE: &str = "profile-title";
const H_UPDATE_INTERVAL: &str = "profile-update-interval";
const H_SUPPORT_URL: &str = "support-url";
const H_WEB_PAGE_URL: &str = "profile-web-page-url";

/// Decode a header value that may be wrapped as `base64:<payload>`.
///
/// Header values are latin-1 by spec, so panels ship non-ASCII titles base64'd;
/// the prefix is the agreed marker. A payload that does not decode to valid
/// UTF-8 is not a title — returning the raw `base64:…` string would put
/// gibberish on the card, so that case yields `None` and the caller falls back.
fn decode_header_text(value: &str) -> Option<String> {
    let Some(payload) = value.strip_prefix("base64:") else {
        return Some(value.trim().to_string()).filter(|v| !v.is_empty());
    };
    let payload: String = payload.chars().filter(|c| !c.is_whitespace()).collect();
    let mut padded = payload;
    while padded.len() % 4 != 0 {
        padded.push('=');
    }
    for attempt in [STANDARD.decode(&padded), URL_SAFE.decode(&padded)] {
        if let Ok(bytes) = attempt {
            if let Ok(text) = String::from_utf8(bytes) {
                let text = text.trim().to_string();
                if !text.is_empty() {
                    return Some(text);
                }
            }
        }
    }
    None
}

fn is_http_url(value: &str) -> bool {
    value.starts_with("http://") || value.starts_with("https://")
}

/// Strip the panel's stand-in entries, and decide whether what is left counts
/// as a real list.
///
/// The flag headers are deliberately treated as advisory rather than fatal on
/// their own: a panel that answers with a genuine server list *and* a stray
/// `X-Hwid-*` header must keep working, so only a list that turns out to be
/// nothing but placeholders is an error. `gate` then supplies the panel's own
/// reason for withholding it, which is what makes the message actionable.
fn drop_placeholders(
    parsed: Vec<ServerEntry>,
    gate: Option<AppError>,
) -> AppResult<Vec<ServerEntry>> {
    let had_any = !parsed.is_empty();
    let servers: Vec<ServerEntry> = parsed.into_iter().filter(|s| !is_placeholder(s)).collect();
    if !servers.is_empty() {
        return Ok(servers);
    }
    if let Some(err) = gate {
        return Err(err);
    }
    if had_any {
        // Placeholders with no flag header: older panel build, or a proxy that
        // stripped them. A device limit is what produces this in practice.
        return Err(AppError::DeviceLimit);
    }
    // Nothing parsed at all — the per-link errors explain why.
    Ok(servers)
}

/// Map the panel's device-gate headers onto an actionable error.
fn hwid_rejection(headers: &reqwest::header::HeaderMap) -> Option<AppError> {
    if header_flag(headers, H_HWID_MAX_DEVICES) {
        return Some(AppError::DeviceLimit);
    }
    if header_flag(headers, H_HWID_NOT_SUPPORTED) {
        return Some(AppError::HwidRequired);
    }
    None
}

/// Presence-plus-truthiness: panels send `true`, but treat any non-negative
/// value as set rather than string-matching one spelling.
fn header_flag(headers: &reqwest::header::HeaderMap, name: &str) -> bool {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .is_some_and(|v| !v.is_empty() && !v.eq_ignore_ascii_case("false") && v != "0")
}

/// The stand-in entry served instead of the real list. `0.0.0.0:1` is never a
/// reachable endpoint, so this is safe to treat as "not a server" outright.
fn is_placeholder(s: &ServerEntry) -> bool {
    s.port == 1 && matches!(s.server.as_str(), "0.0.0.0" | "::" | "127.0.0.1")
}

/// Turn a subscription response body into a plain URI list.
fn decode_body(body: &str) -> AppResult<String> {
    // BOM is not char::is_whitespace, so strip it explicitly
    let trimmed = body.trim_start_matches('\u{feff}').trim();
    if trimmed.is_empty() {
        return Err(AppError::Parse("subscription response is empty".into()));
    }
    if trimmed.lines().next().is_some_and(|l| l.contains("://")) {
        return Ok(trimmed.to_string());
    }

    let compact: String = trimmed.chars().filter(|c| !c.is_whitespace()).collect();
    let mut padded = compact;
    while padded.len() % 4 != 0 {
        padded.push('=');
    }
    for attempt in [STANDARD.decode(&padded), URL_SAFE.decode(&padded)] {
        if let Ok(bytes) = attempt {
            let decoded = String::from_utf8_lossy(&bytes);
            if decoded.contains("://") {
                return Ok(decoded.into_owned());
            }
        }
    }

    if trimmed.starts_with("proxies:") || trimmed.contains("\nproxies:") {
        return Err(AppError::Unsupported(
            "Clash YAML subscription (URI list expected)".into(),
        ));
    }
    if trimmed.starts_with('{') {
        return Err(AppError::Unsupported(
            "JSON config subscription (URI list expected)".into(),
        ));
    }
    Err(AppError::Parse("unrecognized subscription format".into()))
}

/// Parse `subscription-userinfo: upload=..; download=..; total=..; expire=..`.
fn parse_userinfo(value: &str) -> Option<SubscriptionQuota> {
    let mut map: HashMap<String, u64> = HashMap::new();
    for part in value.split(';') {
        if let Some((k, v)) = part.trim().split_once('=') {
            if let Ok(n) = v.trim().parse::<f64>() {
                map.insert(k.trim().to_ascii_lowercase(), n.max(0.0) as u64);
            }
        }
    }
    if map.is_empty() {
        return None;
    }
    Some(SubscriptionQuota {
        upload: map.get("upload").copied().unwrap_or(0),
        download: map.get("download").copied().unwrap_or(0),
        total: map.get("total").copied().unwrap_or(0),
        expire: map.get("expire").copied().unwrap_or(0),
    })
}

/// Extract a filename from a Content-Disposition header value.
/// Prefers RFC 5987 `filename*=UTF-8''...` over plain `filename="..."`.
fn filename_from_disposition(value: &str) -> Option<String> {
    for part in value.split(';') {
        let part = part.trim();
        if let Some(rest) = part.strip_prefix("filename*=") {
            let rest = rest.trim_matches('"');
            let rest = rest.split_once("''").map(|(_, v)| v).unwrap_or(rest);
            let name = percent_decode_str(rest).decode_utf8_lossy().into_owned();
            if !name.is_empty() {
                return Some(name);
            }
        }
    }
    for part in value.split(';') {
        let part = part.trim();
        if let Some(rest) = part.strip_prefix("filename=") {
            let name = rest.trim_matches('"').trim();
            if !name.is_empty() {
                return Some(name.to_string());
            }
        }
    }
    None
}

/// Everything about a server that belongs to the user rather than to the panel,
/// and therefore has to survive a refresh that re-delivers the same link.
struct LocalState<'a> {
    id: &'a str,
    last_ping_ms: Option<u32>,
    favorite: bool,
    total_up: u64,
    total_down: u64,
}

/// Merge freshly fetched servers into an existing list: entries whose raw link
/// is unchanged keep their id, last ping, favorite star and cumulative traffic.
/// Returns (merged, added, removed).
pub fn merge_servers(
    existing: &[ServerEntry],
    fetched: Vec<ServerEntry>,
) -> (Vec<ServerEntry>, usize, usize) {
    let old: HashMap<&str, LocalState<'_>> = existing
        .iter()
        .map(|s| {
            (
                s.raw.as_str(),
                LocalState {
                    id: s.id.as_str(),
                    last_ping_ms: s.last_ping_ms,
                    favorite: s.favorite,
                    total_up: s.total_up,
                    total_down: s.total_down,
                },
            )
        })
        .collect();
    let mut added = 0;
    let mut new_raws: HashSet<String> = HashSet::new();
    let merged: Vec<ServerEntry> = fetched
        .into_iter()
        .map(|mut s| {
            new_raws.insert(s.raw.clone());
            if let Some(local) = old.get(s.raw.as_str()) {
                s.id = local.id.to_string();
                s.last_ping_ms = local.last_ping_ms;
                s.favorite = local.favorite;
                s.total_up = local.total_up;
                s.total_down = local.total_down;
            } else {
                added += 1;
            }
            s
        })
        .collect();
    let removed = existing
        .iter()
        .filter(|s| !new_raws.contains(&s.raw))
        .count();
    (merged, added, removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINK_A: &str = "vless://u1@a.com:443?security=none#a";
    const LINK_B: &str = "vless://u1@b.com:443?security=none#b";

    #[test]
    fn decode_plain_uri_list() {
        let body = format!("\n{LINK_A}\n{LINK_B}\n");
        let out = decode_body(&body).unwrap();
        assert!(out.contains(LINK_A) && out.contains(LINK_B));
    }

    #[test]
    fn decode_standard_base64_without_padding() {
        let plain = format!("{LINK_A}\n{LINK_B}");
        let mut b64 = STANDARD.encode(&plain);
        while b64.ends_with('=') {
            b64.pop();
        }
        // whitespace inside the blob must be tolerated
        let body = format!("{}\n{}", &b64[..10], &b64[10..]);
        let out = decode_body(&body).unwrap();
        assert!(out.contains(LINK_B));
    }

    #[test]
    fn decode_url_safe_base64() {
        // "???" encodes to Pz8_ in the url-safe alphabet (Pz8/ in standard)
        let plain = format!("{LINK_A}?path=???");
        let b64 = URL_SAFE.encode(&plain);
        assert!(b64.contains('_') || b64.contains('-'));
        let out = decode_body(&b64).unwrap();
        assert!(out.contains("a.com"));
    }

    #[test]
    fn decode_strips_bom_before_base64() {
        let body = format!("\u{feff}{}", STANDARD.encode(LINK_A));
        let out = decode_body(&body).unwrap();
        assert!(out.contains("a.com"));
    }

    #[test]
    fn clash_yaml_is_unsupported() {
        let err = decode_body("proxies:\n  - name: x\n    type: vless").unwrap_err();
        assert_eq!(err.code(), "UNSUPPORTED_FORMAT");
        assert!(err.to_string().contains("Clash"));
    }

    #[test]
    fn json_config_is_unsupported() {
        let err = decode_body("{ \"outbounds\": [] }").unwrap_err();
        assert_eq!(err.code(), "UNSUPPORTED_FORMAT");
        assert!(err.to_string().contains("JSON"));
    }

    #[test]
    fn garbage_is_parse_error() {
        let err = decode_body("certainly not a subscription").unwrap_err();
        assert_eq!(err.code(), "PARSE_ERROR");
    }

    #[test]
    fn userinfo_header_parses() {
        let q = parse_userinfo(
            "upload=455727941; download=6174315083; total=1073741824000; expire=1671815872",
        )
        .unwrap();
        assert_eq!(q.upload, 455727941);
        assert_eq!(q.download, 6174315083);
        assert_eq!(q.total, 1073741824000);
        assert_eq!(q.expire, 1671815872);
        assert!(parse_userinfo("nonsense").is_none());
    }

    #[test]
    fn content_disposition_filename() {
        assert_eq!(
            filename_from_disposition("attachment; filename=\"My Sub\"").as_deref(),
            Some("My Sub")
        );
        assert_eq!(
            filename_from_disposition(
                "attachment; filename=\"fallback\"; filename*=UTF-8''%D0%9C%D0%BE%D1%8F"
            )
            .as_deref(),
            Some("Моя")
        );
        assert!(filename_from_disposition("inline").is_none());
    }

    /// A base64 panel body where a few entries use xhttp: the rest must still
    /// import, and each skipped one gets its own named error line.
    #[test]
    fn base64_body_with_xhttp_entries_keeps_the_usable_servers() {
        let plain = "\
vless://u1@a.com:443?security=reality&pbk=k1&type=tcp#DE-1\n\
vless://u1@b.com:443?security=reality&pbk=k2&type=grpc&serviceName=s#NL-2\n\
vless://u1@c.com:443?security=reality&pbk=k3&type=xhttp&mode=auto#SE-3\n\
vless://u1@d.com:443?security=reality&pbk=k4&type=xhttp&mode=packet-up#FI-4\n";
        let body = STANDARD.encode(plain);

        let list = decode_body(&body).unwrap();
        let (servers, errors) = parser::parse_links(&list);

        assert_eq!(servers.len(), 2);
        assert_eq!(errors.len(), 2);
        assert!(errors[0].starts_with("SE-3: "), "{}", errors[0]);
        assert!(errors[1].starts_with("FI-4: "), "{}", errors[1]);
        assert!(errors.iter().all(|e| e.contains("xhttp")));
    }

    /// End-to-end check against a real panel. Ignored by default (needs the
    /// network and a private subscription token, which must not live in the
    /// repo): run with
    /// `UMBRA_LIVE_SUB_URL=<url> cargo test live_subscription -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore = "hits the network; needs UMBRA_LIVE_SUB_URL"]
    async fn live_subscription_returns_the_real_server_list() {
        let Ok(url) = std::env::var("UMBRA_LIVE_SUB_URL") else {
            panic!("set UMBRA_LIVE_SUB_URL to the subscription URL");
        };
        let identity = DeviceIdentity {
            hwid: crate::hwid::hwid(None),
            os: crate::hwid::device_os(),
            os_version: crate::hwid::os_version(),
            model: crate::hwid::device_model(),
        };
        println!("identity: {identity:?}");

        // Without identity headers a device-gated panel serves one placeholder.
        match fetch_subscription(&url, DEFAULT_SUB_USER_AGENT, None).await {
            Ok(anon) => println!("anonymous: {} server(s)", anon.servers.len()),
            Err(e) => println!("anonymous: rejected [{}] {e}", e.code()),
        }

        let got = match fetch_subscription(&url, DEFAULT_SUB_USER_AGENT, Some(&identity)).await {
            Ok(got) => got,
            Err(e) if e.code() == "DEVICE_LIMIT" => panic!(
                "the panel accepted our device id but the account has no free slot \
                 (X-Hwid-Max-Devices-Reached). This is account state, not a client bug: \
                 unlink a device in the provider's panel and re-run."
            ),
            Err(e) => panic!("identified fetch failed [{}]: {e}", e.code()),
        };

        let mut transports: HashMap<String, usize> = HashMap::new();
        let mut securities: HashMap<String, usize> = HashMap::new();
        for s in &got.servers {
            match &s.kind {
                crate::models::ProxyKind::Vless(v) => {
                    let t = match &v.transport {
                        crate::models::Transport::Tcp => "tcp".to_string(),
                        crate::models::Transport::Ws { .. } => "ws".to_string(),
                        crate::models::Transport::Grpc { .. } => "grpc".to_string(),
                        crate::models::Transport::Httpupgrade { .. } => "httpupgrade".to_string(),
                    };
                    *transports.entry(t).or_default() += 1;
                    *securities
                        .entry(format!("{:?}", v.security).to_lowercase())
                        .or_default() += 1;
                }
                crate::models::ProxyKind::Hysteria2(_) => {
                    *transports.entry("udp".to_string()).or_default() += 1;
                    *securities.entry("tls".to_string()).or_default() += 1;
                }
            }
        }
        println!("parsed:   {} server(s)", got.servers.len());
        println!("skipped:  {} link(s)", got.errors.len());
        println!("transports: {transports:?}");
        println!("securities: {securities:?}");
        println!("quota: {:?}", got.quota);
        println!("filename: {:?}", got.filename);
        for s in &got.servers {
            match &s.kind {
                crate::models::ProxyKind::Vless(v) => {
                    println!(
                        "  OK  {:<28} {}:{} flow={:?} sni={:?} sid={:?} fp={:?}",
                        s.name, s.server, s.port, v.flow, v.sni, v.short_id, v.fingerprint
                    );
                }
                crate::models::ProxyKind::Hysteria2(h) => {
                    println!(
                        "  OK Hysteria2 {:<28} {}:{} sni={:?}",
                        s.name, s.server, s.port, h.sni
                    );
                }
            }
        }
        for e in &got.errors {
            println!("  ERR {e}");
        }

        assert!(
            got.servers.len() > 50,
            "expected the real list (>50), got {} — identity headers rejected?",
            got.servers.len()
        );
        assert!(
            !got.servers
                .iter()
                .any(|s| s.name.contains("не поддерживается") || s.server == "0.0.0.0"),
            "placeholder server present: the panel did not accept our identity"
        );
        assert!(got.errors.is_empty(), "unparsed links: {:#?}", got.errors);
    }

    fn headers(pairs: &[(&str, &str)]) -> reqwest::header::HeaderMap {
        let mut h = reqwest::header::HeaderMap::new();
        for (k, v) in pairs {
            h.insert(
                reqwest::header::HeaderName::from_bytes(k.as_bytes()).unwrap(),
                v.parse().unwrap(),
            );
        }
        h
    }

    /// Header names observed on Remnawave panels. Both states
    /// answer HTTP 200 with a placeholder body, so only the header tells them
    /// apart — and the limit case must win, since a panel that rejects on the
    /// device count also still advertises the not-supported flag path.
    #[test]
    fn hwid_headers_map_to_actionable_errors() {
        assert!(hwid_rejection(&headers(&[])).is_none());
        assert_eq!(
            hwid_rejection(&headers(&[("X-Hwid-Not-Supported", "true")]))
                .unwrap()
                .code(),
            "HWID_REQUIRED"
        );
        assert_eq!(
            hwid_rejection(&headers(&[("X-Hwid-Max-Devices-Reached", "true")]))
                .unwrap()
                .code(),
            "DEVICE_LIMIT"
        );
        assert_eq!(
            hwid_rejection(&headers(&[
                ("X-Hwid-Not-Supported", "true"),
                ("X-Hwid-Max-Devices-Reached", "true"),
            ]))
            .unwrap()
            .code(),
            "DEVICE_LIMIT"
        );
        // an explicitly negative flag is not a rejection
        assert!(hwid_rejection(&headers(&[("X-Hwid-Not-Supported", "false")])).is_none());
        assert!(hwid_rejection(&headers(&[("X-Hwid-Max-Devices-Reached", "0")])).is_none());
        // ...but the flag headers are only advisory: presence alone counts
        assert!(hwid_rejection(&headers(&[("X-Hwid-Not-Supported", "1")])).is_some());
    }

    /// A representative body returned when the device gate rejects us.
    const PLACEHOLDER: &str = "vless://00000000-0000-0000-0000-000000000000@0.0.0.0:1\
        ?encryption=none&type=tcp&security=none#Placeholder";

    #[test]
    fn placeholder_parses_cleanly_which_is_why_it_needs_a_guard() {
        let (servers, errors) = parser::parse_links(PLACEHOLDER);
        assert_eq!(servers.len(), 1);
        assert!(errors.is_empty(), "{errors:?}");
        assert!(is_placeholder(&servers[0]));
    }

    #[test]
    fn placeholder_only_list_is_an_error_not_a_server() {
        let (parsed, _) = parser::parse_links(PLACEHOLDER);
        // no header: fall back to the cause that produces this in practice
        let err = drop_placeholders(parsed.clone(), None).unwrap_err();
        assert_eq!(err.code(), "DEVICE_LIMIT");
        // header present: the panel's own reason wins
        let err = drop_placeholders(parsed, Some(AppError::HwidRequired)).unwrap_err();
        assert_eq!(err.code(), "HWID_REQUIRED");
    }

    /// A flag header alongside a genuine list must not break the subscription:
    /// the body, not the header, decides whether the list was withheld.
    #[test]
    fn real_servers_survive_a_stray_flag_header() {
        let (parsed, _) = parser::parse_links(&format!("{PLACEHOLDER}\n{LINK_A}\n{LINK_B}"));
        assert_eq!(parsed.len(), 3);
        let servers = drop_placeholders(parsed, Some(AppError::HwidRequired)).unwrap();
        assert_eq!(servers.len(), 2);
        assert!(servers.iter().all(|s| !is_placeholder(s)));
        assert_eq!(servers[0].server, "a.com");
    }

    /// An unparseable body is reported by the per-link errors, not turned into
    /// a device-limit message that would send the user chasing the wrong thing.
    #[test]
    fn empty_list_without_placeholders_is_not_a_device_limit() {
        assert!(drop_placeholders(Vec::new(), None).unwrap().is_empty());
    }

    /// Some panels encode Profile-Title as base64. Content-Disposition may use
    /// an account id as its filename, so Profile-Title must take precedence.
    #[test]
    fn profile_title_header_decodes_base64() {
        assert_eq!(
            decode_header_text("base64:RXhhbXBsZSBOZXR3b3Jr").as_deref(),
            Some("Example Network")
        );
    }

    #[test]
    fn profile_title_header_accepts_a_plain_value() {
        assert_eq!(decode_header_text("My Panel").as_deref(), Some("My Panel"));
        assert_eq!(decode_header_text("  spaced  ").as_deref(), Some("spaced"));
        assert!(decode_header_text("").is_none());
    }

    #[test]
    fn profile_title_header_survives_unpadded_and_url_safe_base64() {
        let cyrillic = "Моя подписка";
        let mut b64 = STANDARD.encode(cyrillic);
        while b64.ends_with('=') {
            b64.pop();
        }
        assert_eq!(
            decode_header_text(&format!("base64:{b64}")).as_deref(),
            Some(cyrillic)
        );
        let url_safe = URL_SAFE.encode("a?b>c");
        assert_eq!(
            decode_header_text(&format!("base64:{url_safe}")).as_deref(),
            Some("a?b>c")
        );
    }

    /// A payload that is not valid base64/UTF-8 must not be shown verbatim:
    /// "base64:zzz" on the card is worse than falling back to the URL host.
    #[test]
    fn undecodable_base64_title_is_dropped() {
        assert!(decode_header_text("base64:!!!!").is_none());
        assert!(decode_header_text("base64:").is_none());
        // valid base64, invalid utf-8
        assert!(decode_header_text(&format!("base64:{}", STANDARD.encode([0xff, 0xfe]))).is_none());
    }

    #[test]
    fn only_http_urls_are_surfaced_as_links() {
        assert!(is_http_url("https://support.example.com/help"));
        assert!(is_http_url("http://panel.example.com"));
        assert!(!is_http_url("javascript:alert(1)"));
        assert!(!is_http_url("support.example.com/help"));
    }

    /// total=0 is the panel's way of saying "unlimited" — it must still parse
    /// into a quota, otherwise the whole usage row disappears for that account.
    #[test]
    fn unlimited_plan_still_yields_a_quota() {
        let q = parse_userinfo("upload=0; download=127879592133; total=0; expire=1786000000")
            .expect("unlimited plans report total=0, not a missing header");
        assert_eq!(q.download, 127_879_592_133);
        assert_eq!(q.total, 0);
        assert_eq!(q.expire, 1_786_000_000);
    }

    #[test]
    fn merge_keeps_ids_for_unchanged_raw_links() {
        let (mut old, errs) = parser::parse_links(&format!("{LINK_A}\n{LINK_B}"));
        assert!(errs.is_empty());
        old[0].last_ping_ms = Some(42);
        let old_id_a = old[0].id.clone();

        let link_c = "vless://u1@c.com:443?security=none#c";
        let (fresh, _) = parser::parse_links(&format!("{LINK_A}\n{link_c}"));
        let (merged, added, removed) = merge_servers(&old, fresh);

        assert_eq!(merged.len(), 2);
        assert_eq!(added, 1); // c is new
        assert_eq!(removed, 1); // b is gone
        let a = merged.iter().find(|s| s.raw == LINK_A).unwrap();
        assert_eq!(a.id, old_id_a);
        assert_eq!(a.last_ping_ms, Some(42));
    }

    /// The star and the byte counters are the user's, not the panel's: a
    /// refresh re-delivers the same link with `favorite: false` and zeroed
    /// totals, and must not overwrite what we already know about it.
    #[test]
    fn merge_preserves_favorites_and_cumulative_traffic() {
        let (mut old, _) = parser::parse_links(&format!("{LINK_A}\n{LINK_B}"));
        old[0].favorite = true;
        old[0].total_up = 1_000;
        old[0].total_down = 250_000;

        let (fresh, _) = parser::parse_links(&format!("{LINK_A}\n{LINK_B}"));
        assert!(fresh.iter().all(|s| !s.favorite && s.total_up == 0));

        let (merged, added, removed) = merge_servers(&old, fresh);
        assert_eq!((added, removed), (0, 0));
        let a = merged.iter().find(|s| s.raw == LINK_A).unwrap();
        assert!(a.favorite, "a favourited server must stay favourited");
        assert_eq!(a.total_up, 1_000);
        assert_eq!(a.total_down, 250_000);
        let b = merged.iter().find(|s| s.raw == LINK_B).unwrap();
        assert!(!b.favorite);
        assert_eq!(b.total_down, 0);
    }
}
