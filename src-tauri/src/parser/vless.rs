//! vless:// share-link parser.
//! Format: vless://<uuid>@<host>:<port>?<query>#<remark>

use std::collections::HashMap;
use std::net::IpAddr;

use percent_encoding::percent_decode_str;
use url::Url;

use super::LinkParser;
use crate::error::{AppError, AppResult};
use crate::models::{Security, ServerEntry, Transport};

pub struct VlessParser;

impl LinkParser for VlessParser {
    fn scheme(&self) -> &str {
        "vless"
    }

    fn parse(&self, uri: &str) -> AppResult<ServerEntry> {
        parse_vless(uri)
    }
}

/// Percent-decode exactly once; `+` is NOT treated as a space.
fn dec(s: &str) -> String {
    percent_decode_str(s).decode_utf8_lossy().into_owned()
}

/// Manual query parsing: `Url::query_pairs` treats `+` as space (form
/// encoding), which would corrupt base64 values like `pbk`. Keys are decoded,
/// values are kept raw; first occurrence of a key wins.
fn query_map(query: Option<&str>) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let Some(query) = query else { return map };
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        map.entry(dec(k)).or_insert_with(|| v.to_string());
    }
    map
}

fn parse_vless(uri: &str) -> AppResult<ServerEntry> {
    let url = Url::parse(uri).map_err(|e| AppError::Parse(format!("invalid vless link: {e}")))?;
    if url.scheme() != "vless" {
        return Err(AppError::Parse("not a vless link".into()));
    }

    let uuid = dec(url.username());
    if uuid.is_empty() {
        return Err(AppError::Parse("missing user id before @".into()));
    }
    let host_raw = url
        .host_str()
        .ok_or_else(|| AppError::Parse("missing server address".into()))?;
    let server = host_raw
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_string();
    let port = url
        .port()
        .ok_or_else(|| AppError::Parse("missing port".into()))?;

    let q = query_map(url.query());
    let get = |k: &str| q.get(k).map(|v| dec(v));

    if get("headerType").as_deref() == Some("http") {
        return Err(AppError::Unsupported(
            "headerType=http (HTTP obfuscation)".into(),
        ));
    }

    let type_param = get("type")
        .map(|t| t.trim().to_ascii_lowercase())
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| "tcp".into());
    let transport = match type_param.as_str() {
        "tcp" => Transport::Tcp,
        "ws" | "httpupgrade" => {
            let mut path = get("path")
                .filter(|p| !p.is_empty())
                .unwrap_or_else(|| "/".into());
            // ws early-data is often smuggled inside the path as ?ed=NNNN
            if let Some(i) = path.find("?ed=") {
                path.truncate(i);
            }
            if path.is_empty() {
                path = "/".into();
            }
            let host = get("host").unwrap_or_default();
            if type_param == "ws" {
                Transport::Ws { path, host }
            } else {
                Transport::Httpupgrade { path, host }
            }
        }
        "grpc" => Transport::Grpc {
            service_name: get("serviceName").unwrap_or_default(),
        },
        other => {
            // Never fall back to tcp here: an unknown transport downgraded to
            // raw tcp would dial a real server with the wrong framing and fail
            // in a way that looks like a network problem. sing-box implements
            // only ws / grpc / http / httpupgrade / quic (see
            // https://sing-box.sagernet.org/configuration/shared/v2ray-transport/),
            // so xhttp — Xray's SplitHTTP — has no representable outbound.
            let detail = match other {
                "xhttp" | "splithttp" => " — sing-box has no xhttp/SplitHTTP transport (Xray only)",
                _ => "",
            };
            return Err(AppError::Unsupported(format!(
                "transport \"{other}\"{detail}"
            )));
        }
    };

    let security_param = get("security").filter(|s| !s.is_empty());
    let security = match security_param.as_deref().unwrap_or("none") {
        "reality" => Security::Reality,
        "tls" => Security::Tls,
        "none" => Security::None,
        other => {
            return Err(AppError::Unsupported(format!("security \"{other}\"")));
        }
    };

    let sni = if security == Security::None {
        String::new()
    } else {
        get("sni")
            .filter(|s| !s.is_empty())
            .or_else(|| get("host").filter(|s| !s.is_empty()))
            .or_else(|| {
                // fall back to the server address only when it is a domain
                if server.parse::<IpAddr>().is_err() {
                    Some(server.clone())
                } else {
                    None
                }
            })
            .unwrap_or_default()
    };

    let fingerprint = match security {
        Security::Reality => get("fp")
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "chrome".into()),
        _ => get("fp").unwrap_or_default(),
    };

    // pbk is base64: take verbatim, never percent-decode
    let public_key = q.get("pbk").cloned().unwrap_or_default();
    if security == Security::Reality && public_key.is_empty() {
        return Err(AppError::Parse("reality link is missing pbk".into()));
    }
    let short_id = get("sid").unwrap_or_default();

    // xtls-rprx-vision requires tcp; drop flow for other transports
    let flow = if transport == Transport::Tcp {
        get("flow").unwrap_or_default()
    } else {
        String::new()
    };

    let alpn = if security == Security::Reality {
        Vec::new()
    } else {
        get("alpn")
            .map(|a| {
                a.split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(String::from)
                    .collect()
            })
            .unwrap_or_default()
    };

    let insecure = get("allowInsecure")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
        || get("insecure").map(|v| v == "1").unwrap_or(false);

    let name = url
        .fragment()
        .map(dec)
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| format!("{server}:{port}"));

    Ok(ServerEntry {
        id: uuid::Uuid::new_v4().to_string(),
        name,
        protocol: "vless".into(),
        server,
        port,
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
        last_ping_ms: None,
        favorite: false,
        total_up: 0,
        total_down: 0,
        raw: uri.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reality_vision_tcp_with_cyrillic_remark() {
        let uri = "vless://b831381d-6324-4d53-ad4f-8cda48b30811@example.com:443?security=reality&sni=yahoo.com&fp=firefox&pbk=SbVKOEMjK0sIlbwg4akyBg5mL5KZwwB-ed4eEE7YnRc&sid=6ba85179&flow=xtls-rprx-vision&type=tcp#%D0%9C%D0%BE%D1%81%D0%BA%D0%B2%D0%B0%20%F0%9F%9A%80";
        let s = parse_vless(uri).unwrap();
        assert_eq!(s.name, "Москва 🚀");
        assert_eq!(s.protocol, "vless");
        assert_eq!(s.uuid, "b831381d-6324-4d53-ad4f-8cda48b30811");
        assert_eq!(s.server, "example.com");
        assert_eq!(s.port, 443);
        assert_eq!(s.security, Security::Reality);
        assert_eq!(s.sni, "yahoo.com");
        assert_eq!(s.fingerprint, "firefox");
        assert_eq!(s.public_key, "SbVKOEMjK0sIlbwg4akyBg5mL5KZwwB-ed4eEE7YnRc");
        assert_eq!(s.short_id, "6ba85179");
        assert_eq!(s.flow, "xtls-rprx-vision");
        assert_eq!(s.transport, Transport::Tcp);
        assert!(s.alpn.is_empty());
        assert_eq!(s.raw, uri);
    }

    #[test]
    fn ws_tls_with_encoded_path_and_ed() {
        let uri = "vless://u1@host.com:8443?security=tls&type=ws&path=%2Fchat%3Fed%3D2048&host=cdn.example.org&alpn=h2,http/1.1&allowInsecure=1";
        let s = parse_vless(uri).unwrap();
        assert_eq!(s.security, Security::Tls);
        assert_eq!(
            s.transport,
            Transport::Ws {
                path: "/chat".into(),
                host: "cdn.example.org".into()
            }
        );
        // sni falls back to the host param
        assert_eq!(s.sni, "cdn.example.org");
        assert_eq!(s.alpn, vec!["h2".to_string(), "http/1.1".to_string()]);
        assert!(s.insecure);
        // no fragment -> host:port
        assert_eq!(s.name, "host.com:8443");
    }

    #[test]
    fn grpc_with_service_name() {
        let uri = "vless://u1@h.com:2053?security=tls&type=grpc&serviceName=my%2Fgrpc&sni=cdn.com";
        let s = parse_vless(uri).unwrap();
        assert_eq!(
            s.transport,
            Transport::Grpc {
                service_name: "my/grpc".into()
            }
        );
        assert_eq!(s.sni, "cdn.com");
    }

    #[test]
    fn ipv6_host_brackets_stripped() {
        let uri = "vless://u1@[2001:db8::1]:443#v6";
        let s = parse_vless(uri).unwrap();
        assert_eq!(s.server, "2001:db8::1");
        assert_eq!(s.port, 443);
        assert_eq!(s.security, Security::None);
        assert_eq!(s.sni, "");
        assert_eq!(s.name, "v6");
    }

    #[test]
    fn missing_port_is_parse_error() {
        let err = parse_vless("vless://u1@host.com?security=tls&type=tcp").unwrap_err();
        assert_eq!(err.code(), "PARSE_ERROR");
        assert!(err.to_string().contains("port"));
    }

    #[test]
    fn header_type_http_is_unsupported() {
        let err = parse_vless("vless://u1@host.com:80?type=tcp&headerType=http").unwrap_err();
        assert_eq!(err.code(), "UNSUPPORTED_FORMAT");
        assert!(err.to_string().contains("headerType"));
    }

    #[test]
    fn alpn_dropped_for_reality() {
        let uri = "vless://u1@h.com:443?security=reality&pbk=xyz&alpn=h2%2Chttp%2F1.1&type=tcp";
        let s = parse_vless(uri).unwrap();
        assert!(s.alpn.is_empty());
    }

    #[test]
    fn flow_dropped_for_ws() {
        let uri = "vless://u1@h.com:443?security=tls&type=ws&flow=xtls-rprx-vision&path=/x";
        let s = parse_vless(uri).unwrap();
        assert_eq!(s.flow, "");
        assert_eq!(
            s.transport,
            Transport::Ws {
                path: "/x".into(),
                host: String::new()
            }
        );
    }

    #[test]
    fn reality_missing_pbk_is_parse_error() {
        let err = parse_vless("vless://u1@h.com:443?security=reality&type=tcp").unwrap_err();
        assert_eq!(err.code(), "PARSE_ERROR");
        assert!(err.to_string().contains("pbk"));
    }

    #[test]
    fn pbk_verbatim_plus_kept_and_fp_defaults_to_chrome() {
        let uri = "vless://u1@h.com:443?security=reality&pbk=abc+def%2F&type=tcp";
        let s = parse_vless(uri).unwrap();
        // + not treated as space, %2F not decoded
        assert_eq!(s.public_key, "abc+def%2F");
        assert_eq!(s.fingerprint, "chrome");
    }

    /// sing-box has no xhttp/SplitHTTP transport, so such links must be
    /// reported as skipped rather than downgraded to tcp.
    #[test]
    fn xhttp_is_unsupported_and_never_becomes_tcp() {
        let uri = "vless://u1@h.com:443?security=reality&pbk=abc&type=xhttp&path=%2Fyz&host=cdn.com&mode=auto#Amsterdam";
        let err = parse_vless(uri).unwrap_err();
        assert_eq!(err.code(), "UNSUPPORTED_FORMAT");
        let msg = err.to_string();
        assert!(msg.contains("xhttp"), "{msg}");
        assert!(msg.contains("sing-box"), "{msg}");
    }

    #[test]
    fn splithttp_alias_is_unsupported() {
        let err = parse_vless("vless://u1@h.com:443?security=tls&type=splithttp#X").unwrap_err();
        assert_eq!(err.code(), "UNSUPPORTED_FORMAT");
        assert!(err.to_string().contains("sing-box"));
    }

    #[test]
    fn unknown_transports_are_rejected_not_silently_tcp() {
        // "http"/"quic" are real sing-box transports but have no Transport
        // variant here, so they must be skipped rather than mapped to tcp.
        for ty in ["XHTTP", "garbage", "h2", "http", "quic", "kcp"] {
            let uri = format!("vless://u1@h.com:443?security=tls&type={ty}#N");
            let err = parse_vless(&uri)
                .map(|s| s.transport)
                .expect_err(&format!("type={ty} must not parse"));
            assert_eq!(err.code(), "UNSUPPORTED_FORMAT", "type={ty}");
            assert!(err.to_string().contains(&ty.to_ascii_lowercase()), "{err}");
        }
    }

    #[test]
    fn transport_type_matching_is_case_insensitive() {
        let s = parse_vless("vless://u1@h.com:443?security=tls&type=WS&path=/x").unwrap();
        assert_eq!(
            s.transport,
            Transport::Ws {
                path: "/x".into(),
                host: String::new()
            }
        );
    }

    #[test]
    fn sni_falls_back_to_server_domain_but_not_ip() {
        let s = parse_vless("vless://u1@example.org:443?security=tls").unwrap();
        assert_eq!(s.sni, "example.org");
        let s = parse_vless("vless://u1@1.2.3.4:443?security=tls").unwrap();
        assert_eq!(s.sni, "");
    }
}
