//! Share-link parsing: scheme dispatch + bulk import helper.

pub mod vless;

use std::collections::HashSet;

use percent_encoding::percent_decode_str;

use crate::error::{AppError, AppResult};
use crate::models::ServerEntry;

pub trait LinkParser {
    fn scheme(&self) -> &str;
    fn parse(&self, uri: &str) -> AppResult<ServerEntry>;
}

/// Dispatch a single share link by its scheme prefix.
pub fn parse_any(uri: &str) -> AppResult<ServerEntry> {
    let parsers: [&dyn LinkParser; 1] = [&vless::VlessParser];
    for parser in parsers {
        let scheme = parser.scheme();
        if let Some(prefix) = uri.get(..scheme.len()) {
            if prefix.eq_ignore_ascii_case(scheme) && uri[scheme.len()..].starts_with("://") {
                return parser.parse(uri);
            }
        }
    }
    let scheme = uri.split("://").next().unwrap_or(uri);
    Err(AppError::Unsupported(format!("scheme \"{scheme}\"")))
}

/// Parse a blob of pasted text: split on whitespace/newlines, skip tokens that
/// are not links, deduplicate by raw link. Returns (parsed, error strings).
pub fn parse_links(text: &str) -> (Vec<ServerEntry>, Vec<String>) {
    let mut seen: HashSet<&str> = HashSet::new();
    let mut servers = Vec::new();
    let mut errors = Vec::new();
    for token in text.split_whitespace() {
        // BOM is not whitespace; panels/files often prefix the first link with it
        let token = token.trim_start_matches('\u{feff}');
        if !token.contains("://") {
            continue;
        }
        if !seen.insert(token) {
            continue;
        }
        match parse_any(token) {
            Ok(entry) => servers.push(entry),
            Err(e) => errors.push(format!("{}: {e}", label(token))),
        }
    }
    (servers, errors)
}

/// Label for an error line: the link's `#remark` (the server name shown by the
/// panel), so a partially-failing subscription lists which servers were
/// skipped instead of a stack of near-identical truncated URIs.
fn label(link: &str) -> String {
    let name = link
        .split_once('#')
        .map(|(_, frag)| percent_decode_str(frag).decode_utf8_lossy().into_owned())
        .unwrap_or_default();
    let name = name.trim();
    if name.is_empty() {
        short(link)
    } else {
        short(name)
    }
}

fn short(link: &str) -> String {
    const MAX: usize = 64;
    if link.chars().count() <= MAX {
        link.to_string()
    } else {
        let mut s: String = link.chars().take(MAX).collect();
        s.push('…');
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_links_dedupes_and_collects_errors() {
        let text = "vless://u1@h.com:443?security=none#a\n\
                    vless://u1@h.com:443?security=none#a\n\
                    vmess://whatever\n\
                    just-some-words";
        let (servers, errors) = parse_links(text);
        assert_eq!(servers.len(), 1);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("vmess"));
    }

    #[test]
    fn parse_links_strips_bom() {
        let (servers, errors) = parse_links("\u{feff}vless://u1@h.com:443?security=none#a");
        assert_eq!(servers.len(), 1);
        assert!(errors.is_empty());
    }

    /// Mirrors the real subscription: mostly tcp/grpc Reality plus a handful of
    /// xhttp entries. The good ones must import; the bad ones must be listed by
    /// name, one line each.
    #[test]
    fn mixed_batch_imports_good_links_and_names_the_skipped() {
        let text = "\
vless://u1@a.com:443?security=reality&pbk=k1&type=tcp&flow=xtls-rprx-vision#DE-1\n\
vless://u1@b.com:443?security=reality&pbk=k2&type=grpc&serviceName=svc#NL-2\n\
vless://u1@c.com:443?security=reality&pbk=k3&type=xhttp&path=%2Fx#%D0%A4%D0%B8%D0%BD%D0%BB%D1%8F%D0%BD%D0%B4%D0%B8%D1%8F\n\
vless://u1@d.com:443?security=reality&pbk=k4&type=tcp#US-3\n\
vless://u1@e.com:443?security=reality&pbk=k5&type=xhttp#Japan\n";
        let (servers, errors) = parse_links(text);

        let names: Vec<&str> = servers.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["DE-1", "NL-2", "US-3"]);
        assert_eq!(errors.len(), 2);

        let fin = errors.iter().find(|e| e.starts_with("Финляндия")).unwrap();
        assert!(fin.contains("xhttp"), "{fin}");
        assert!(errors.iter().any(|e| e.starts_with("Japan:")));
        // no truncated-URI noise: names replace the raw links
        assert!(errors.iter().all(|e| !e.contains("vless://")), "{errors:?}");
    }

    #[test]
    fn error_label_falls_back_to_link_without_remark() {
        let (servers, errors) = parse_links("vless://u1@h.com:443?security=tls&type=xhttp");
        assert!(servers.is_empty());
        assert_eq!(errors.len(), 1);
        assert!(errors[0].starts_with("vless://u1@h.com"), "{}", errors[0]);
    }

    #[test]
    fn parse_any_rejects_unknown_scheme() {
        let err = parse_any("ss://abc@h:1").unwrap_err();
        assert_eq!(err.code(), "UNSUPPORTED_FORMAT");
    }
}
