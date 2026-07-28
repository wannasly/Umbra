use percent_encoding::percent_decode_str;
use url::Url;

use super::LinkParser;
use crate::error::{AppError, AppResult};
use crate::models::{Hysteria2Node, Hysteria2Obfs, ProxyKind, ProxyNode};

pub struct Hysteria2Parser;

impl LinkParser for Hysteria2Parser {
    fn can_parse(&self, uri: &str) -> bool {
        let lower = uri.to_ascii_lowercase();
        lower.starts_with("hysteria2://") || lower.starts_with("hy2://")
    }

    fn parse(&self, uri: &str) -> AppResult<ProxyNode> {
        parse_hysteria2(uri)
    }
}

pub fn parse_hysteria2(uri: &str) -> AppResult<ProxyNode> {
    let lower_uri = uri.trim();
    if !lower_uri.to_ascii_lowercase().starts_with("hysteria2://")
        && !lower_uri.to_ascii_lowercase().starts_with("hy2://")
    {
        return Err(AppError::Parse("not a hysteria2 URI".into()));
    }

    // 1. Check for multi-port in raw string (e.g. :443-445 or :443,444)
    if let Some(scheme_end) = lower_uri.find("://") {
        let after_scheme = &lower_uri[scheme_end + 3..];
        // Strip userinfo if present
        let host_and_rest = match after_scheme.rfind('@') {
            Some(at_pos) => &after_scheme[at_pos + 1..],
            None => after_scheme,
        };
        if let Some(port_pos) = host_and_rest.rfind(':') {
            let after_colon = &host_and_rest[port_pos + 1..];
            let end_of_port = after_colon
                .find(&['?', '#', '/'][..])
                .unwrap_or(after_colon.len());
            let port_part = &after_colon[..end_of_port];
            if port_part.contains('-') || port_part.contains(',') {
                return Err(AppError::Unsupported("multi-port is unsupported".into()));
            }
        }
    }

    let parsed = Url::parse(lower_uri)
        .map_err(|e| AppError::Parse(format!("invalid hysteria2 url: {e}")))?;

    // 1. Raw userinfo extraction & single percent-decode
    let raw_userinfo = parsed.username();
    let password = if raw_userinfo.is_empty() {
        return Err(AppError::Parse(
            "hysteria2 URL missing auth/password".into(),
        ));
    } else {
        percent_decode_str(raw_userinfo)
            .decode_utf8()
            .map_err(|_| AppError::Parse("invalid utf-8 in userinfo".into()))?
            .into_owned()
    };

    // 2. Server host & port
    let host_str = parsed
        .host_str()
        .ok_or_else(|| AppError::Parse("hysteria2 URL missing host".into()))?;

    // Trim brackets from IPv6 host if present
    let server = if host_str.starts_with('[') && host_str.ends_with(']') {
        host_str[1..host_str.len() - 1].to_string()
    } else {
        host_str.to_string()
    };

    if server.is_empty() {
        return Err(AppError::Parse("hysteria2 URL host is empty".into()));
    }

    let port = match parsed.port() {
        Some(p) => p,
        None => 443,
    };

    // Check for multi-port in raw string (e.g. :443-445 or :443,444)
    if let Some(port_pos) = lower_uri.rfind(':') {
        let after_colon = &lower_uri[port_pos + 1..];
        let end_of_port = after_colon
            .find(&['?', '#'][..])
            .unwrap_or(after_colon.len());
        let port_part = &after_colon[..end_of_port];
        if port_part.contains('-') || port_part.contains(',') {
            return Err(AppError::Unsupported("multi-port is unsupported".into()));
        }
    }

    // 3. Query parameters
    let mut sni = String::new();
    let mut insecure = false;
    let mut alpn = Vec::new();
    let mut obfs_type = None;
    let mut obfs_password = None;

    for (k, v) in parsed.query_pairs() {
        let k_lower = k.to_ascii_lowercase();
        match k_lower.as_str() {
            "sni" => {
                sni = v.into_owned();
            }
            "insecure" | "allowinsecure" => {
                let v_lower = v.to_ascii_lowercase();
                if v_lower == "1" || v_lower == "true" {
                    insecure = true;
                } else if v_lower == "0" || v_lower == "false" {
                    insecure = false;
                } else {
                    return Err(AppError::Parse(format!("invalid insecure value: {v}")));
                }
            }
            "alpn" => {
                if !v.is_empty() {
                    alpn = v
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                }
            }
            "obfs" => {
                obfs_type = Some(v.to_ascii_lowercase());
            }
            "obfs-password" | "obfspassword" | "obfs-param" | "obfsparam" => {
                obfs_password = Some(v.into_owned());
            }
            "pinsha256" | "pin-sha256" => {
                return Err(AppError::Unsupported("pinSHA256 is unsupported".into()));
            }
            "ech" => {
                return Err(AppError::Unsupported("ech is unsupported".into()));
            }
            _ => {}
        }
    }

    // SNI fallback if not set: use domain host (not IP)
    if sni.is_empty() && !server.parse::<std::net::IpAddr>().is_ok() {
        sni = server.clone();
    }

    // Obfs handling
    let obfs = match obfs_type.as_deref() {
        None | Some("none") => None,
        Some("salamander") => {
            let pass = obfs_password
                .ok_or_else(|| AppError::Parse("obfs=salamander requires obfs-password".into()))?;
            Some(Hysteria2Obfs::Salamander { password: pass })
        }
        Some(other) => {
            return Err(AppError::Unsupported(format!(
                "hysteria2 obfs type '{other}' is unsupported"
            )));
        }
    };

    // 4. Remark / Name
    let name = parsed
        .fragment()
        .map(percent_decode_str)
        .map(|dec| dec.decode_utf8_lossy().into_owned())
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| format!("{server}:{port}"));

    Ok(ProxyNode {
        id: uuid::Uuid::new_v4().to_string(),
        name,
        server,
        port,
        last_ping_ms: None,
        favorite: false,
        total_up: 0,
        total_down: 0,
        raw: uri.to_string(),
        kind: ProxyKind::Hysteria2(Hysteria2Node {
            password,
            obfs,
            insecure,
            sni,
            alpn,
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_hysteria2_with_salamander_obfs() {
        let uri = "hysteria2://my_pass@hy2.example.com:8443?sni=sni.example.com&insecure=1&obfs=salamander&obfs-password=obfs_pass#My%20Server";
        let node = parse_hysteria2(uri).unwrap();

        assert_eq!(node.name, "My Server");
        assert_eq!(node.server, "hy2.example.com");
        assert_eq!(node.port, 8443);

        let ProxyKind::Hysteria2(ref h) = node.kind else {
            panic!("expected Hysteria2")
        };
        assert_eq!(h.password, "my_pass");
        assert_eq!(h.sni, "sni.example.com");
        assert!(h.insecure);
        assert_eq!(
            h.obfs,
            Some(Hysteria2Obfs::Salamander {
                password: "obfs_pass".into()
            })
        );
    }

    #[test]
    fn parse_hy2_alias_and_default_port_443() {
        let uri = "hy2://secret@example.org/?alpn=h3,h2#Test";
        let node = parse_hysteria2(uri).unwrap();

        assert_eq!(node.server, "example.org");
        assert_eq!(node.port, 443);
        assert_eq!(node.name, "Test");

        let ProxyKind::Hysteria2(ref h) = node.kind else {
            panic!("expected Hysteria2")
        };
        assert_eq!(h.password, "secret");
        assert_eq!(h.sni, "example.org");
        assert_eq!(h.alpn, vec!["h3", "h2"]);
        assert!(h.obfs.is_none());
    }

    #[test]
    fn reject_multi_port() {
        let uri = "hy2://pass@example.com:443-445#Test";
        let err = parse_hysteria2(uri).unwrap_err();
        assert_eq!(err.code(), "UNSUPPORTED_FORMAT");
        assert!(err.to_string().contains("multi-port"));
    }

    #[test]
    fn reject_pin_sha256() {
        let uri = "hy2://pass@example.com:443?pinSHA256=abcdef#Test";
        let err = parse_hysteria2(uri).unwrap_err();
        assert_eq!(err.code(), "UNSUPPORTED_FORMAT");
        assert!(err.to_string().contains("pinSHA256"));
    }

    #[test]
    fn reject_ech() {
        let uri = "hy2://pass@example.com:443?ech=1#Test";
        let err = parse_hysteria2(uri).unwrap_err();
        assert_eq!(err.code(), "UNSUPPORTED_FORMAT");
        assert!(err.to_string().contains("ech"));
    }

    #[test]
    fn reject_unknown_obfs() {
        let uri = "hy2://pass@example.com:443?obfs=shadowsocks#Test";
        let err = parse_hysteria2(uri).unwrap_err();
        assert_eq!(err.code(), "UNSUPPORTED_FORMAT");
        assert!(err.to_string().contains("shadowsocks"));
    }
}
