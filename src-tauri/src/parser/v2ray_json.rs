//! Parser for V2Ray JSON Array subscriptions.
//!
//! Panels serving Happ / v2rayNG deliver an array of full client configurations.
//! Each element is a standalone routing config; we extract the primary outbound
//! (tag == "proxy" or compatible fallback) and map it to Umbra's ProxyNode model.

use std::collections::HashSet;
use std::net::IpAddr;

use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use serde_json::Value;

use crate::error::{AppError, AppResult};
use crate::models::{
    Hysteria2Node, Hysteria2Obfs, ProxyKind, ProxyNode, Security, ServerEntry, Transport, VlessNode,
};

/// Helper to check whether an ASCII keyword appears with word boundaries.
fn contains_word(text: &str, word: &str) -> bool {
    let mut search_from = 0;
    while let Some(idx) = text[search_from..].find(word) {
        let abs_idx = search_from + idx;
        let before_ok = if abs_idx == 0 {
            true
        } else {
            !text[..abs_idx].chars().next_back().unwrap().is_ascii_alphanumeric()
        };
        let after_idx = abs_idx + word.len();
        let after_ok = if after_idx >= text.len() {
            true
        } else {
            !text[after_idx..].chars().next().unwrap().is_ascii_alphanumeric()
        };
        if before_ok && after_ok {
            return true;
        }
        search_from = abs_idx + word.len();
    }
    false
}

/// Check if an entry is an informational panel notice rather than a real server.
/// Mirrors `src/lib/serverMeta.ts` so notices (expiry countdowns, support links,
/// unroutable placeholders) are not turned into VPN nodes.
pub fn is_info_entry(name: &str, server: &str, port: u16) -> bool {
    let lower_server = server.trim().to_ascii_lowercase();
    if matches!(
        lower_server.as_str(),
        "0.0.0.0" | "::" | "::0" | "127.0.0.1" | "::1" | "localhost"
    ) {
        return true;
    }
    if port == 1 {
        return true;
    }

    let lower_name = name.to_ascii_lowercase();
    if lower_name.contains("http://")
        || lower_name.contains("https://")
        || lower_name.contains("t.me/")
    {
        return true;
    }

    // Telegram handle / mention matching /(^|[\s([])@[a-z0-9_]{4,}/i
    for (idx, _) in lower_name.match_indices('@') {
        let is_delim = if idx == 0 {
            true
        } else {
            let prev = lower_name[..idx].chars().next_back().unwrap();
            prev.is_whitespace() || prev == '(' || prev == '['
        };
        if is_delim {
            let after_at = &lower_name[idx + 1..];
            let handle_len = after_at
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .count();
            if handle_len >= 4 {
                return true;
            }
        }
    }

    // Cyrillic keywords (stem matching, no boundary needed)
    const CYRILLIC_KEYWORDS: &[&str] = &[
        "осталось",
        "осталась",
        "истека",
        "истёк",
        "истек",
        "подписк",
        "продлит",
        "продлен",
        "тариф",
        "трафик",
        "не работает",
        "нажим",
        "нажат",
        "обновит",
        "обновлени",
        "включит",
        "поддержк",
        "дней осталось",
    ];

    for kw in CYRILLIC_KEYWORDS {
        if lower_name.contains(kw) {
            return true;
        }
    }

    // English keywords with word boundary check matching regex \b...\b
    const ASCII_KEYWORDS: &[&str] = &[
        "days left",
        "days remaining",
        "expires",
        "expired",
        "expiring",
        "subscription",
        "renew",
    ];

    for kw in ASCII_KEYWORDS {
        if contains_word(&lower_name, kw) {
            return true;
        }
    }

    false
}

const EXCLUDED_ROUTING: &[&str] = &[
    "freedom", "direct", "blackhole", "block", "dns", "selector", "urltest", "balancer",
];

fn is_excluded(proto: &str, tag: &str) -> bool {
    EXCLUDED_ROUTING.contains(&proto) || EXCLUDED_ROUTING.contains(&tag)
}

/// Find the main proxy outbound from an outbounds array.
///
/// Priority:
/// 1. Outbound with tag == "proxy" (case-insensitive) if compatible and not an excluded routing helper
/// 2. Fallback to first compatible proxy outbound of type vless, hysteria or hysteria2
///
/// Explicitly avoids routing helpers: direct/freedom, block/blackhole, dns, selector, urltest, balancer.
fn select_outbound(outbounds: &[Value]) -> Option<&Value> {
    if let Some(proxy) = outbounds.iter().find(|o| {
        o.get("tag")
            .and_then(|t| t.as_str())
            .is_some_and(|t| t.eq_ignore_ascii_case("proxy"))
    }) {
        let proto = proxy
            .get("protocol")
            .or_else(|| proxy.get("type"))
            .and_then(|v| v.as_str())
            .map(|p| p.to_ascii_lowercase())
            .unwrap_or_default();
        let tag = proxy
            .get("tag")
            .and_then(|v| v.as_str())
            .map(|t| t.to_ascii_lowercase())
            .unwrap_or_default();

        if !is_excluded(&proto, &tag) && matches!(proto.as_str(), "vless" | "hysteria" | "hysteria2") {
            return Some(proxy);
        }
    }

    for o in outbounds {
        let proto = o
            .get("protocol")
            .or_else(|| o.get("type"))
            .and_then(|v| v.as_str())
            .map(|p| p.to_ascii_lowercase())
            .unwrap_or_default();

        let tag = o
            .get("tag")
            .and_then(|v| v.as_str())
            .map(|t| t.to_ascii_lowercase())
            .unwrap_or_default();

        if is_excluded(&proto, &tag) {
            continue;
        }

        if matches!(proto.as_str(), "vless" | "hysteria" | "hysteria2") {
            return Some(o);
        }
    }

    None
}

fn enc(s: &str) -> String {
    utf8_percent_encode(s, NON_ALPHANUMERIC).to_string()
}

pub fn build_canonical_vless_uri(
    server: &str,
    port: u16,
    name: &str,
    node: &VlessNode,
) -> String {
    let mut query = Vec::new();

    match &node.transport {
        Transport::Tcp => query.push("type=tcp".to_string()),
        Transport::Ws { path, host } => {
            query.push("type=ws".to_string());
            if !path.is_empty() && path != "/" {
                query.push(format!("path={}", enc(path)));
            }
            if !host.is_empty() {
                query.push(format!("host={}", enc(host)));
            }
        }
        Transport::Grpc { service_name } => {
            query.push("type=grpc".to_string());
            if !service_name.is_empty() {
                query.push(format!("serviceName={}", enc(service_name)));
            }
        }
        Transport::Httpupgrade { path, host } => {
            query.push("type=httpupgrade".to_string());
            if !path.is_empty() && path != "/" {
                query.push(format!("path={}", enc(path)));
            }
            if !host.is_empty() {
                query.push(format!("host={}", enc(host)));
            }
        }
    }

    match node.security {
        Security::Reality => {
            query.push("security=reality".to_string());
            if !node.public_key.is_empty() {
                query.push(format!("pbk={}", node.public_key));
            }
            if !node.fingerprint.is_empty() {
                query.push(format!("fp={}", node.fingerprint));
            }
            if !node.short_id.is_empty() {
                query.push(format!("sid={}", node.short_id));
            }
            if !node.sni.is_empty() {
                query.push(format!("sni={}", enc(&node.sni)));
            }
            if !node.flow.is_empty() {
                query.push(format!("flow={}", node.flow));
            }
        }
        Security::Tls => {
            query.push("security=tls".to_string());
            if !node.sni.is_empty() {
                query.push(format!("sni={}", enc(&node.sni)));
            }
            if !node.fingerprint.is_empty() {
                query.push(format!("fp={}", node.fingerprint));
            }
            if node.insecure {
                query.push("insecure=1".to_string());
            }
            if !node.alpn.is_empty() {
                query.push(format!("alpn={}", node.alpn.join(",")));
            }
            if !node.flow.is_empty() {
                query.push(format!("flow={}", node.flow));
            }
        }
        Security::None => {
            query.push("security=none".to_string());
        }
    }

    let query_str = if query.is_empty() {
        String::new()
    } else {
        format!("?{}", query.join("&"))
    };

    let frag = if name.is_empty() {
        String::new()
    } else {
        format!("#{}", enc(name))
    };

    let host_formatted = if server.contains(':') && !server.starts_with('[') {
        format!("[{server}]")
    } else {
        server.to_string()
    };

    format!("vless://{}@{}:{}{}{}", node.uuid, host_formatted, port, query_str, frag)
}

pub fn build_canonical_hysteria2_uri(
    server: &str,
    port: u16,
    name: &str,
    node: &Hysteria2Node,
) -> String {
    let mut query = Vec::new();

    if !node.sni.is_empty() && node.sni != server {
        query.push(format!("sni={}", enc(&node.sni)));
    }
    if node.insecure {
        query.push("insecure=1".to_string());
    }
    if !node.alpn.is_empty() {
        query.push(format!("alpn={}", node.alpn.join(",")));
    }
    if let Some(Hysteria2Obfs::Salamander { password }) = &node.obfs {
        query.push("obfs=salamander".to_string());
        query.push(format!("obfs-password={}", enc(password)));
    }

    let query_str = if query.is_empty() {
        String::new()
    } else {
        format!("?{}", query.join("&"))
    };

    let frag = if name.is_empty() {
        String::new()
    } else {
        format!("#{}", enc(name))
    };

    let host_formatted = if server.contains(':') && !server.starts_with('[') {
        format!("[{server}]")
    } else {
        server.to_string()
    };

    let auth_encoded = enc(&node.password);
    format!("hysteria2://{}@{}:{}{}{}", auth_encoded, host_formatted, port, query_str, frag)
}

fn parse_port_val(v: Option<&Value>) -> Option<u16> {
    let v = v?;
    if let Some(p) = v.as_u64() {
        return Some(p as u16);
    }
    if let Some(s) = v.as_str() {
        return s.trim().parse::<u16>().ok();
    }
    None
}

fn parse_vless_outbound(remarks: &str, outbound: &Value) -> AppResult<ServerEntry> {
    let settings = outbound.get("settings").unwrap_or(&Value::Null);

    // Host & port: settings.vnext[0].address or settings.address
    let (server, port, uuid, flow) = if let Some(vnext) = settings.get("vnext").and_then(|v| v.as_array()).and_then(|arr| arr.first()) {
        let address = vnext
            .get("address")
            .and_then(|a| a.as_str())
            .ok_or_else(|| AppError::Parse("vless missing address in vnext".into()))?;
        let port = parse_port_val(vnext.get("port"))
            .ok_or_else(|| AppError::Parse("vless missing port in vnext".into()))?;
        let user = vnext
            .get("users")
            .and_then(|u| u.as_array())
            .and_then(|arr| arr.first())
            .ok_or_else(|| AppError::Parse("vless missing user in vnext".into()))?;
        let uuid = user
            .get("id")
            .and_then(|i| i.as_str())
            .ok_or_else(|| AppError::Parse("vless missing user id".into()))?
            .to_string();
        let flow = user
            .get("flow")
            .and_then(|f| f.as_str())
            .unwrap_or_default()
            .to_string();
        (address.to_string(), port, uuid, flow)
    } else {
        let address = settings
            .get("address")
            .and_then(|a| a.as_str())
            .ok_or_else(|| AppError::Parse("vless missing address".into()))?;
        let port = parse_port_val(settings.get("port"))
            .ok_or_else(|| AppError::Parse("vless missing port".into()))?;
        let uuid = settings
            .get("id")
            .or_else(|| settings.get("uuid"))
            .and_then(|i| i.as_str())
            .ok_or_else(|| AppError::Parse("vless missing user id".into()))?
            .to_string();
        let flow = settings
            .get("flow")
            .and_then(|f| f.as_str())
            .unwrap_or_default()
            .to_string();
        (address.to_string(), port, uuid, flow)
    };

    let server = server
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_string();

    let stream = outbound.get("streamSettings").unwrap_or(&Value::Null);
    let network = stream
        .get("network")
        .and_then(|n| n.as_str())
        .map(str::trim)
        .unwrap_or("tcp")
        .to_ascii_lowercase();

    // sing-box does not support xhttp or splithttp
    if matches!(network.as_str(), "xhttp" | "splithttp") {
        return Err(AppError::Unsupported(format!(
            "transport \"{network}\" — sing-box has no xhttp/SplitHTTP transport (Xray only)"
        )));
    }

    let transport = match network.as_str() {
        "tcp" => Transport::Tcp,
        "grpc" => {
            let service_name = stream
                .get("grpcSettings")
                .and_then(|g| g.get("serviceName"))
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            Transport::Grpc { service_name }
        }
        "ws" => {
            let ws = stream.get("wsSettings").unwrap_or(&Value::Null);
            let path = ws
                .get("path")
                .and_then(|p| p.as_str())
                .filter(|p| !p.is_empty())
                .unwrap_or("/")
                .to_string();
            let host = ws
                .get("headers")
                .and_then(|h| h.get("Host").or_else(|| h.get("host")))
                .and_then(|h| h.as_str())
                .unwrap_or_default()
                .to_string();
            Transport::Ws { path, host }
        }
        "httpupgrade" => {
            let hu = stream.get("httpupgradeSettings").unwrap_or(&Value::Null);
            let path = hu
                .get("path")
                .and_then(|p| p.as_str())
                .filter(|p| !p.is_empty())
                .unwrap_or("/")
                .to_string();
            let host = hu
                .get("host")
                .and_then(|h| h.as_str())
                .unwrap_or_default()
                .to_string();
            Transport::Httpupgrade { path, host }
        }
        other => return Err(AppError::Unsupported(format!("transport \"{other}\""))),
    };

    let security_param = stream
        .get("security")
        .and_then(|s| s.as_str())
        .unwrap_or("none")
        .to_ascii_lowercase();

    let (security, sni, fingerprint, public_key, short_id, insecure, alpn) = match security_param.as_str() {
        "reality" => {
            let reality = stream.get("realitySettings").unwrap_or(&Value::Null);
            let raw_sni = reality
                .get("serverName")
                .and_then(|s| s.as_str())
                .unwrap_or_default();
            let sni = if raw_sni.is_empty() && server.parse::<IpAddr>().is_err() {
                server.clone()
            } else {
                raw_sni.to_string()
            };
            let pbk = reality
                .get("publicKey")
                .and_then(|k| k.as_str())
                .unwrap_or_default();
            if pbk.is_empty() {
                return Err(AppError::Parse("reality link is missing pbk".into()));
            }
            let fp = reality
                .get("fingerprint")
                .and_then(|f| f.as_str())
                .filter(|f| !f.is_empty())
                .unwrap_or("chrome")
                .to_string();
            let sid = reality
                .get("shortId")
                .and_then(|s| s.as_str())
                .or_else(|| {
                    reality
                        .get("shortIds")
                        .and_then(|arr| arr.as_array())
                        .and_then(|a| a.first())
                        .and_then(|s| s.as_str())
                })
                .unwrap_or_default()
                .to_string();
            (Security::Reality, sni, fp, pbk.to_string(), sid, false, Vec::new())
        }
        "tls" => {
            let tls = stream.get("tlsSettings").unwrap_or(&Value::Null);
            let raw_sni = tls
                .get("serverName")
                .and_then(|s| s.as_str())
                .unwrap_or_default();
            let sni = if raw_sni.is_empty() && server.parse::<IpAddr>().is_err() {
                server.clone()
            } else {
                raw_sni.to_string()
            };
            let fp = tls
                .get("fingerprint")
                .and_then(|f| f.as_str())
                .unwrap_or_default()
                .to_string();
            let insecure = tls
                .get("allowInsecure")
                .or_else(|| tls.get("insecure"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let alpn = tls
                .get("alpn")
                .and_then(|a| a.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            (Security::Tls, sni, fp, String::new(), String::new(), insecure, alpn)
        }
        "none" => (Security::None, String::new(), String::new(), String::new(), String::new(), false, Vec::new()),
        other => return Err(AppError::Unsupported(format!("security \"{other}\""))),
    };

    // Flow is only valid for tcp
    let flow = if transport == Transport::Tcp {
        flow
    } else {
        String::new()
    };

    let name = remarks.trim();
    let name = if name.is_empty() {
        format!("{server}:{port}")
    } else {
        name.to_string()
    };

    let vless_node = VlessNode {
        uuid,
        flow,
        security,
        sni,
        fingerprint,
        public_key,
        short_id,
        insecure,
        alpn,
        transport,
    };

    let raw = build_canonical_vless_uri(&server, port, &name, &vless_node);

    Ok(ProxyNode {
        id: uuid::Uuid::new_v4().to_string(),
        name,
        server,
        port,
        last_ping_ms: None,
        favorite: false,
        total_up: 0,
        total_down: 0,
        raw,
        kind: ProxyKind::Vless(vless_node),
    })
}

fn parse_hysteria_outbound(remarks: &str, outbound: &Value) -> AppResult<ServerEntry> {
    let settings = outbound.get("settings").unwrap_or(&Value::Null);

    let server = settings
        .get("address")
        .or_else(|| {
            settings
                .get("vnext")
                .and_then(|v| v.as_array())
                .and_then(|arr| arr.first())
                .and_then(|e| e.get("address"))
        })
        .and_then(|a| a.as_str())
        .ok_or_else(|| AppError::Parse("hysteria missing address".into()))?;

    let server = server
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_string();

    let port = parse_port_val(settings.get("port"))
        .or_else(|| {
            settings
                .get("vnext")
                .and_then(|v| v.as_array())
                .and_then(|arr| arr.first())
                .and_then(|e| parse_port_val(e.get("port")))
        })
        .unwrap_or(443);

    let stream = outbound.get("streamSettings").unwrap_or(&Value::Null);
    let hysteria_settings = stream
        .get("hysteriaSettings")
        .or_else(|| stream.get("hysteria2Settings"))
        .unwrap_or(&Value::Null);

    let password = hysteria_settings
        .get("auth")
        .or_else(|| hysteria_settings.get("password"))
        .or_else(|| settings.get("auth"))
        .or_else(|| settings.get("password"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::Parse("hysteria missing auth/password".into()))?
        .to_string();

    let tls = stream.get("tlsSettings").unwrap_or(&Value::Null);
    let raw_sni = tls
        .get("serverName")
        .and_then(|s| s.as_str())
        .unwrap_or_default();
    let sni = if raw_sni.is_empty() && server.parse::<IpAddr>().is_err() {
        server.clone()
    } else {
        raw_sni.to_string()
    };

    let insecure = tls
        .get("allowInsecure")
        .or_else(|| tls.get("insecure"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let alpn = tls
        .get("alpn")
        .and_then(|a| a.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_else(|| vec!["h3".to_string()]);

    let obfs = hysteria_settings
        .get("obfsPassword")
        .or_else(|| hysteria_settings.get("obfs-password"))
        .or_else(|| {
            hysteria_settings
                .get("obfs")
                .and_then(|o| o.get("password").or_else(|| o.get("salamander")))
        })
        .and_then(|v| v.as_str())
        .filter(|p| !p.is_empty())
        .map(|pwd| Hysteria2Obfs::Salamander {
            password: pwd.to_string(),
        });

    let name = remarks.trim();
    let name = if name.is_empty() {
        format!("{server}:{port}")
    } else {
        name.to_string()
    };

    let hysteria_node = Hysteria2Node {
        password,
        obfs,
        insecure,
        sni,
        alpn,
    };

    let raw = build_canonical_hysteria2_uri(&server, port, &name, &hysteria_node);

    Ok(ProxyNode {
        id: uuid::Uuid::new_v4().to_string(),
        name,
        server,
        port,
        last_ping_ms: None,
        favorite: false,
        total_up: 0,
        total_down: 0,
        raw,
        kind: ProxyKind::Hysteria2(hysteria_node),
    })
}

/// Parse a root V2Ray JSON Array `[ { ... }, { ... } ]`.
///
/// Returns (compatible servers, error strings).
/// Does not abort the entire batch if individual nodes fail or have unsupported transports (e.g. xhttp).
pub fn parse_v2ray_json(text: &str) -> (Vec<ServerEntry>, Vec<String>) {
    let items: Vec<Value> = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(e) => return (Vec::new(), vec![format!("failed to parse V2Ray JSON: {e}")]),
    };

    let mut servers = Vec::new();
    let mut errors = Vec::new();
    let mut seen_raws: HashSet<String> = HashSet::new();

    for item in items {
        let remarks = item
            .get("remarks")
            .and_then(|r| r.as_str())
            .unwrap_or_default();

        let outbounds = match item.get("outbounds").and_then(|o| o.as_array()) {
            Some(o) if !o.is_empty() => o,
            _ => {
                errors.push(format!("{remarks}: missing outbounds array"));
                continue;
            }
        };

        let outbound = match select_outbound(outbounds) {
            Some(o) => o,
            None => {
                errors.push(format!("{remarks}: no compatible proxy outbound found"));
                continue;
            }
        };

        let proto = outbound
            .get("protocol")
            .or_else(|| outbound.get("type"))
            .and_then(|p| p.as_str())
            .map(|p| p.to_ascii_lowercase())
            .unwrap_or_default();

        // Extract host/port for info check
        let server_addr = outbound
            .get("settings")
            .and_then(|s| {
                s.get("address").or_else(|| {
                    s.get("vnext")
                        .and_then(|v| v.as_array())
                        .and_then(|arr| arr.first())
                        .and_then(|e| e.get("address"))
                })
            })
            .and_then(|a| a.as_str())
            .unwrap_or_default();

        let server_port = outbound
            .get("settings")
            .and_then(|s| {
                s.get("port").or_else(|| {
                    s.get("vnext")
                        .and_then(|v| v.as_array())
                        .and_then(|arr| arr.first())
                        .and_then(|e| e.get("port"))
                })
            })
            .and_then(|p| p.as_u64())
            .unwrap_or(0) as u16;

        if is_info_entry(remarks, server_addr, server_port) {
            // Informational notices are intentionally dropped from the server list
            continue;
        }

        match proto.as_str() {
            "vless" => match parse_vless_outbound(remarks, outbound) {
                Ok(entry) => {
                    if seen_raws.insert(entry.raw.clone()) {
                        servers.push(entry);
                    }
                }
                Err(e) => {
                    errors.push(format!("{remarks}: {e}"));
                }
            },
            "hysteria" | "hysteria2" => match parse_hysteria_outbound(remarks, outbound) {
                Ok(entry) => {
                    if seen_raws.insert(entry.raw.clone()) {
                        servers.push(entry);
                    }
                }
                Err(e) => {
                    errors.push(format!("{remarks}: {e}"));
                }
            },
            other => {
                errors.push(format!("{remarks}: unsupported protocol \"{other}\""));
            }
        }
    }

    (servers, errors)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // Test 1: VLESS TCP Reality
    #[test]
    fn test_1_vless_tcp_reality() {
        let json_input = json!([
            {
                "remarks": "NL-1 TCP",
                "outbounds": [
                    {
                        "tag": "proxy",
                        "protocol": "vless",
                        "settings": {
                            "vnext": [{
                                "address": "nl1.example.com",
                                "port": 443,
                                "users": [{
                                    "id": "11111111-2222-3333-4444-555555555555",
                                    "encryption": "none",
                                    "flow": "xtls-rprx-vision"
                                }]
                            }]
                        },
                        "streamSettings": {
                            "network": "tcp",
                            "security": "reality",
                            "realitySettings": {
                                "serverName": "nl1.example.com",
                                "publicKey": "pk-test-key",
                                "shortId": "sid123",
                                "fingerprint": "firefox"
                            }
                        }
                    }
                ]
            }
        ])
        .to_string();

        let (servers, errors) = parse_v2ray_json(&json_input);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(servers.len(), 1);
        let s = &servers[0];
        assert_eq!(s.name, "NL-1 TCP");
        assert_eq!(s.server, "nl1.example.com");
        assert_eq!(s.port, 443);

        let ProxyKind::Vless(v) = &s.kind else { panic!("not vless") };
        assert_eq!(v.uuid, "11111111-2222-3333-4444-555555555555");
        assert_eq!(v.flow, "xtls-rprx-vision");
        assert_eq!(v.security, Security::Reality);
        assert_eq!(v.sni, "nl1.example.com");
        assert_eq!(v.public_key, "pk-test-key");
        assert_eq!(v.short_id, "sid123");
        assert_eq!(v.fingerprint, "firefox");
        assert_eq!(v.transport, Transport::Tcp);
    }

    // Test 2: VLESS gRPC Reality
    #[test]
    fn test_2_vless_grpc_reality() {
        let json_input = json!([
            {
                "remarks": "SE-2 GRPC",
                "outbounds": [
                    {
                        "tag": "proxy",
                        "protocol": "vless",
                        "settings": {
                            "vnext": [{
                                "address": "se2.example.com",
                                "port": 7443,
                                "users": [{
                                    "id": "22222222-2222-3333-4444-555555555555",
                                    "encryption": "none",
                                    "flow": ""
                                }]
                            }]
                        },
                        "streamSettings": {
                            "network": "grpc",
                            "grpcSettings": {
                                "serviceName": "test-grpc-svc",
                                "authority": "se2.example.com"
                            },
                            "security": "reality",
                            "realitySettings": {
                                "serverName": "se2.example.com",
                                "publicKey": "pk-grpc-key",
                                "shortId": "sid456",
                                "fingerprint": "chrome"
                            }
                        }
                    }
                ]
            }
        ])
        .to_string();

        let (servers, errors) = parse_v2ray_json(&json_input);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(servers.len(), 1);
        let s = &servers[0];
        assert_eq!(s.name, "SE-2 GRPC");
        assert_eq!(s.server, "se2.example.com");
        assert_eq!(s.port, 7443);

        let ProxyKind::Vless(v) = &s.kind else { panic!("not vless") };
        assert_eq!(v.uuid, "22222222-2222-3333-4444-555555555555");
        assert_eq!(v.flow, "");
        assert_eq!(v.security, Security::Reality);
        assert_eq!(v.public_key, "pk-grpc-key");
        assert_eq!(
            v.transport,
            Transport::Grpc {
                service_name: "test-grpc-svc".to_string()
            }
        );
    }

    // Test 3: Hysteria 2
    #[test]
    fn test_3_hysteria_2() {
        let json_input = json!([
            {
                "remarks": "Games Hysteria",
                "outbounds": [
                    {
                        "tag": "proxy",
                        "protocol": "hysteria",
                        "settings": {
                            "address": "hy.example.com",
                            "port": 6443,
                            "version": 2
                        },
                        "streamSettings": {
                            "network": "hysteria",
                            "hysteriaSettings": {
                                "version": 2,
                                "auth": "secret-pass"
                            },
                            "security": "tls",
                            "tlsSettings": {
                                "serverName": "hy.example.com",
                                "alpn": ["h3"]
                            }
                        }
                    }
                ]
            }
        ])
        .to_string();

        let (servers, errors) = parse_v2ray_json(&json_input);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(servers.len(), 1);
        let s = &servers[0];
        assert_eq!(s.name, "Games Hysteria");
        assert_eq!(s.server, "hy.example.com");
        assert_eq!(s.port, 6443);

        let ProxyKind::Hysteria2(h) = &s.kind else { panic!("not hysteria2") };
        assert_eq!(h.password, "secret-pass");
        assert_eq!(h.sni, "hy.example.com");
        assert_eq!(h.alpn, vec!["h3"]);
    }

    // Test 4: xHTTP is safely skipped without crashing the batch
    #[test]
    fn test_4_xhttp_skipped() {
        let json_input = json!([
            {
                "remarks": "Skipped-XHTTP",
                "outbounds": [{
                    "tag": "proxy",
                    "protocol": "vless",
                    "settings": {
                        "vnext": [{
                            "address": "xhttp.example.com",
                            "port": 23443,
                            "users": [{ "id": "u1", "flow": "" }]
                        }]
                    },
                    "streamSettings": {
                        "network": "xhttp",
                        "security": "reality",
                        "realitySettings": {
                            "publicKey": "pk",
                            "serverName": "xhttp.example.com"
                        }
                    }
                }]
            },
            {
                "remarks": "Good-TCP",
                "outbounds": [{
                    "tag": "proxy",
                    "protocol": "vless",
                    "settings": {
                        "vnext": [{
                            "address": "tcp.example.com",
                            "port": 443,
                            "users": [{ "id": "u2", "flow": "" }]
                        }]
                    },
                    "streamSettings": {
                        "network": "tcp",
                        "security": "reality",
                        "realitySettings": {
                            "publicKey": "pk2",
                            "serverName": "tcp.example.com"
                        }
                    }
                }]
            }
        ])
        .to_string();

        let (servers, errors) = parse_v2ray_json(&json_input);
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].name, "Good-TCP");
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("xhttp"));
    }

    // Test 5: Fallback when proxy tag is absent
    #[test]
    fn test_5_fallback_when_proxy_tag_absent() {
        let json_input = json!([
            {
                "remarks": "Fallback-Server",
                "outbounds": [
                    {
                        "tag": "custom-vless-tag",
                        "protocol": "vless",
                        "settings": {
                            "vnext": [{
                                "address": "fallback.example.com",
                                "port": 443,
                                "users": [{ "id": "u1", "flow": "" }]
                            }]
                        },
                        "streamSettings": {
                            "network": "tcp",
                            "security": "none"
                        }
                    }
                ]
            }
        ])
        .to_string();

        let (servers, errors) = parse_v2ray_json(&json_input);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].server, "fallback.example.com");
    }

    // Test 6: Fallback does not pick direct/freedom
    #[test]
    fn test_6_direct_first_not_imported() {
        let json_input = json!([
            {
                "remarks": "Direct-First",
                "outbounds": [
                    { "tag": "direct", "protocol": "freedom" },
                    { "tag": "block", "protocol": "blackhole" },
                    {
                        "tag": "real-proxy",
                        "protocol": "vless",
                        "settings": {
                            "vnext": [{
                                "address": "real.example.com",
                                "port": 443,
                                "users": [{ "id": "u1", "flow": "" }]
                            }]
                        },
                        "streamSettings": {
                            "network": "tcp",
                            "security": "none"
                        }
                    }
                ]
            }
        ])
        .to_string();

        let (servers, errors) = parse_v2ray_json(&json_input);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].server, "real.example.com");
    }

    // Test 7: Informational entry is skipped and does not become a VPN node
    #[test]
    fn test_7_info_entry_skipped() {
        let json_input = json!([
            {
                "remarks": "⌛️Дней осталось: 19",
                "outbounds": [{
                    "tag": "proxy",
                    "protocol": "vless",
                    "settings": {
                        "vnext": [{
                            "address": "sub1.example.com",
                            "port": 10443,
                            "users": [{ "id": "u1", "flow": "" }]
                        }]
                    }
                }]
            },
            {
                "remarks": "✉️Наш TG: @proxire Новости и Поддержка",
                "outbounds": [{
                    "tag": "proxy",
                    "protocol": "vless",
                    "settings": {
                        "vnext": [{
                            "address": "sub2.example.com",
                            "port": 10443,
                            "users": [{ "id": "u2", "flow": "" }]
                        }]
                    }
                }]
            }
        ])
        .to_string();

        let (servers, _errors) = parse_v2ray_json(&json_input);
        assert!(servers.is_empty(), "info entries must not become VPN nodes");
    }

    // Test 8: Repeated import does not create duplicates
    #[test]
    fn test_8_repeated_import_deduplicates() {
        let json_input = json!([
            {
                "remarks": "Server-1",
                "outbounds": [{
                    "tag": "proxy",
                    "protocol": "vless",
                    "settings": {
                        "vnext": [{
                            "address": "srv1.example.com",
                            "port": 443,
                            "users": [{ "id": "u1", "flow": "" }]
                        }]
                    },
                    "streamSettings": { "network": "tcp", "security": "none" }
                }]
            },
            {
                "remarks": "Server-1",
                "outbounds": [{
                    "tag": "proxy",
                    "protocol": "vless",
                    "settings": {
                        "vnext": [{
                            "address": "srv1.example.com",
                            "port": 443,
                            "users": [{ "id": "u1", "flow": "" }]
                        }]
                    },
                    "streamSettings": { "network": "tcp", "security": "none" }
                }]
            }
        ])
        .to_string();

        let (servers, _) = parse_v2ray_json(&json_input);
        assert_eq!(servers.len(), 1, "identical raw links in same batch must be deduplicated");
    }

    #[test]
    fn test_canonical_vless_and_hy2_uri_roundtrip() {
        let vless_node = VlessNode {
            uuid: "11111111-2222-3333-4444-555555555555".into(),
            flow: "xtls-rprx-vision".into(),
            security: Security::Reality,
            sni: "nl.example.com".into(),
            fingerprint: "chrome".into(),
            public_key: "test_pbk_123".into(),
            short_id: "abcd12".into(),
            insecure: false,
            alpn: vec![],
            transport: Transport::Tcp,
        };
        let uri = build_canonical_vless_uri("nl.example.com", 443, "NL-1", &vless_node);
        let parsed = crate::parser::vless::VlessParser;
        use crate::parser::LinkParser;
        let entry = parsed.parse(&uri).expect("canonical vless URI should parse");
        assert_eq!(entry.server, "nl.example.com");
        assert_eq!(entry.port, 443);
        assert_eq!(entry.name, "NL-1");
        let ProxyKind::Vless(v) = entry.kind else { panic!("not vless") };
        assert_eq!(v.uuid, vless_node.uuid);
        assert_eq!(v.public_key, vless_node.public_key);

        let hy2_node = Hysteria2Node {
            password: "mypassword".into(),
            obfs: Some(Hysteria2Obfs::Salamander {
                password: "obfspass".into(),
            }),
            insecure: false,
            sni: "hy.example.com".into(),
            alpn: vec!["h3".into()],
        };
        let uri_hy2 = build_canonical_hysteria2_uri("hy.example.com", 6443, "Hy-1", &hy2_node);
        let parsed_hy2 = crate::parser::hysteria2::Hysteria2Parser;
        let entry_hy2 = parsed_hy2.parse(&uri_hy2).expect("canonical hy2 URI should parse");
        assert_eq!(entry_hy2.server, "hy.example.com");
        assert_eq!(entry_hy2.port, 6443);
        assert_eq!(entry_hy2.name, "Hy-1");
        let ProxyKind::Hysteria2(h) = entry_hy2.kind else { panic!("not hy2") };
        assert_eq!(h.password, hy2_node.password);
        assert_eq!(h.sni, hy2_node.sni);
    }

    #[test]
    fn test_ipv6_canonical_uri_formatting() {
        let vless_node = VlessNode {
            uuid: "11111111-2222-3333-4444-555555555555".into(),
            flow: "".into(),
            security: Security::None,
            sni: "".into(),
            fingerprint: "".into(),
            public_key: "".into(),
            short_id: "".into(),
            insecure: false,
            alpn: vec![],
            transport: Transport::Tcp,
        };
        let uri = build_canonical_vless_uri("2001:db8::1", 443, "IPv6 Node", &vless_node);
        assert!(uri.contains("@[2001:db8::1]:443"));
        use crate::parser::LinkParser;
        let parsed = crate::parser::vless::VlessParser.parse(&uri).expect("IPv6 URI must be valid");
        assert_eq!(parsed.server, "2001:db8::1");
    }

    #[test]
    fn test_info_entry_handle_delimitation() {
        // Legitimate server with @ in mid-word (email/domain/label) must NOT be treated as info entry
        assert!(!is_info_entry("NL - 10G@ams", "1.2.3.4", 443));
        assert!(!is_info_entry("user@domain.com node", "1.2.3.4", 443));

        // Telegram mention preceded by whitespace, parenthesis or at start must be treated as info entry
        assert!(is_info_entry("@proxire_support News", "1.2.3.4", 443));
        assert!(is_info_entry("Support: @my_vpn_bot", "1.2.3.4", 443));
        assert!(is_info_entry("Join (@my_channel)", "1.2.3.4", 443));
    }

    #[test]
    fn test_string_port_and_short_ids_array() {
        let json_input = json!([
            {
                "remarks": "String Port Node",
                "outbounds": [{
                    "tag": "proxy",
                    "protocol": "vless",
                    "settings": {
                        "vnext": [{
                            "address": "strport.example.com",
                            "port": "8443",
                            "users": [{ "id": "u1", "flow": "" }]
                        }]
                    },
                    "streamSettings": {
                        "network": "tcp",
                        "security": "reality",
                        "realitySettings": {
                            "publicKey": "pk-str",
                            "serverName": "strport.example.com",
                            "shortIds": ["sid_from_array"]
                        }
                    }
                }]
            }
        ])
        .to_string();

        let (servers, errors) = parse_v2ray_json(&json_input);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].port, 8443);
        let ProxyKind::Vless(v) = &servers[0].kind else { panic!("not vless") };
        assert_eq!(v.short_id, "sid_from_array");
    }
}
