//! Pure sing-box 1.13 config generator. No I/O; builds the JSON document,
//! the serverId -> outbound-tag map and carries the clash-api coordinates.

use std::collections::{HashMap, HashSet};

use serde_json::{json, Map, Value};

use crate::error::{AppError, AppResult};
use crate::models::{
    AppRouteAction, Mode, RouteTarget, Security, ServerEntry, Settings, Transport,
};

/// Outbound tags that always exist in the generated config; server tags must
/// not collide with them.
const RESERVED_TAGS: [&str; 3] = ["proxy", "auto", "direct"];

#[derive(Debug, Clone)]
pub struct GeneratedConfig {
    pub json: Value,
    pub tag_by_server_id: HashMap<String, String>,
    pub clash_port: u16,
    pub clash_secret: String,
}

pub fn generate(
    settings: &Settings,
    servers: &[&ServerEntry],
    selected_id: &str,
    clash_port: u16,
    clash_secret: &str,
) -> AppResult<GeneratedConfig> {
    let tags = assign_tags(servers);
    let tag_by_server_id: HashMap<String, String> = servers
        .iter()
        .zip(tags.iter())
        .map(|(s, t)| (s.id.clone(), t.clone()))
        .collect();
    let selected_tag = servers
        .iter()
        .position(|s| s.id == selected_id)
        .map(|i| tags[i].clone())
        .ok_or_else(|| AppError::NotFound(format!("server {selected_id}")))?;

    let mut selector_outbounds = Vec::with_capacity(tags.len() + 1);
    selector_outbounds.push(json!("auto"));
    selector_outbounds.extend(tags.iter().map(|t| json!(t)));

    let mut outbounds = Vec::with_capacity(servers.len() + 3);
    outbounds.push(json!({
        "type": "selector",
        "tag": "proxy",
        "outbounds": selector_outbounds,
        "default": selected_tag,
        "interrupt_exist_connections": true
    }));
    outbounds.push(json!({
        "type": "urltest",
        "tag": "auto",
        "outbounds": tags,
        "url": settings.ping_url,
        "interval": "3m",
        "tolerance": 50
    }));
    for (server, tag) in servers.iter().zip(tags.iter()) {
        outbounds.push(vless_outbound(server, tag));
    }
    outbounds.push(json!({ "type": "direct", "tag": "direct" }));

    let json = json!({
        "log": { "level": settings.log_level, "timestamp": true },
        "experimental": {
            "clash_api": {
                "external_controller": format!("127.0.0.1:{clash_port}"),
                "secret": clash_secret,
                "default_mode": "Rule"
            },
            "cache_file": { "enabled": true, "path": "cache.db" }
        },
        "dns": {
            "servers": [
                { "tag": "dns-remote", "type": "https", "server": "1.1.1.1", "detour": "proxy" },
                { "tag": "dns-local", "type": "local" }
            ],
            "final": "dns-remote",
            "strategy": "prefer_ipv4",
            "independent_cache": true
        },
        "inbounds": inbounds(settings),
        "outbounds": outbounds,
        "route": route(settings)
    });

    Ok(GeneratedConfig {
        json,
        tag_by_server_id,
        clash_port,
        clash_secret: clash_secret.to_string(),
    })
}

fn sanitize_name(name: &str) -> String {
    name.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn assign_tags(servers: &[&ServerEntry]) -> Vec<String> {
    let mut used: HashSet<String> = RESERVED_TAGS.iter().map(|t| t.to_string()).collect();
    let mut tags = Vec::with_capacity(servers.len());
    for (i, server) in servers.iter().enumerate() {
        let mut base = sanitize_name(&server.name);
        if base.is_empty() {
            base = format!("server-{}", i + 1);
        }
        let mut tag = base.clone();
        let mut n = 2;
        while !used.insert(tag.clone()) {
            tag = format!("{base} ({n})");
            n += 1;
        }
        tags.push(tag);
    }
    tags
}

fn vless_outbound(s: &ServerEntry, tag: &str) -> Value {
    let mut o = Map::new();
    o.insert("type".into(), json!("vless"));
    o.insert("tag".into(), json!(tag));
    o.insert("server".into(), json!(s.server));
    o.insert("server_port".into(), json!(s.port));
    o.insert("uuid".into(), json!(s.uuid));
    if !s.flow.is_empty() {
        o.insert("flow".into(), json!(s.flow));
    }
    o.insert("packet_encoding".into(), json!("xudp"));

    if s.security != Security::None {
        let mut tls = Map::new();
        tls.insert("enabled".into(), json!(true));
        tls.insert("server_name".into(), json!(s.sni));
        tls.insert("insecure".into(), json!(s.insecure));
        if !s.fingerprint.is_empty() || s.security == Security::Reality {
            let fingerprint = if s.fingerprint.is_empty() {
                "chrome"
            } else {
                s.fingerprint.as_str()
            };
            tls.insert(
                "utls".into(),
                json!({ "enabled": true, "fingerprint": fingerprint }),
            );
        }
        if !s.alpn.is_empty() {
            tls.insert("alpn".into(), json!(s.alpn));
        }
        if s.security == Security::Reality {
            tls.insert(
                "reality".into(),
                json!({
                    "enabled": true,
                    "public_key": s.public_key,
                    "short_id": s.short_id
                }),
            );
        }
        o.insert("tls".into(), Value::Object(tls));
    }

    match &s.transport {
        Transport::Tcp => {}
        Transport::Ws { path, host } => {
            let mut t = Map::new();
            t.insert("type".into(), json!("ws"));
            t.insert("path".into(), json!(path));
            if !host.is_empty() {
                t.insert("headers".into(), json!({ "Host": host }));
            }
            o.insert("transport".into(), Value::Object(t));
        }
        Transport::Grpc { service_name } => {
            o.insert(
                "transport".into(),
                json!({ "type": "grpc", "service_name": service_name }),
            );
        }
        Transport::Httpupgrade { path, host } => {
            o.insert(
                "transport".into(),
                json!({ "type": "httpupgrade", "path": path, "host": host }),
            );
        }
    }
    Value::Object(o)
}

fn inbounds(settings: &Settings) -> Value {
    match settings.mode {
        Mode::SystemProxy => json!([{
            "type": "mixed",
            "tag": "mixed-in",
            "listen": "127.0.0.1",
            "listen_port": settings.mixed_port
        }]),
        // Nothing here needs a crash-recovery counterpart to the system proxy
        // backup: both halves of the TUN setup are torn down by the kernel when
        // sing-box dies. wintun creates the adapter through SwDeviceCreate without
        // SwDeviceSetLifetime, so it keeps the default handle lifetime and
        // Windows removes it (and every auto_route route on it) as soon as the
        // process handle closes — TerminateProcess included. strict_route's
        // blocking filters are added on a WFP engine session opened with
        // FWPM_SESSION_FLAG_DYNAMIC, which BFE drops when that session ends.
        Mode::Tun => json!([{
            "type": "tun",
            "tag": "tun-in",
            "interface_name": "umbra-tun",
            "address": ["172.19.0.1/30", "fdfe:dcba:9876::1/126"],
            "mtu": settings.tun_mtu,
            "auto_route": true,
            "strict_route": settings.tun_strict_route,
            "stack": settings.tun_stack
        }]),
    }
}

fn route(settings: &Settings) -> Value {
    let mut rules = vec![
        json!({ "action": "sniff" }),
        json!({ "protocol": "dns", "action": "hijack-dns" }),
    ];
    for rule in &settings.app_routes {
        let process_name = rule.process_name.trim();
        if process_name.is_empty() {
            continue;
        }
        rules.push(match rule.action {
            AppRouteAction::Proxy => {
                json!({ "process_name": [process_name], "outbound": "proxy" })
            }
            AppRouteAction::Direct => {
                json!({ "process_name": [process_name], "outbound": "direct" })
            }
            AppRouteAction::Block => {
                json!({ "process_name": [process_name], "action": "reject" })
            }
        });
    }
    rules.extend([
        json!({ "ip_is_private": true, "outbound": "direct" }),
        json!({ "clash_mode": "Direct", "outbound": "direct" }),
        json!({ "clash_mode": "Global", "outbound": "proxy" }),
    ]);
    if settings.bypass_ru {
        rules.push(json!({ "rule_set": ["geosite-ru", "geoip-ru"], "outbound": "direct" }));
    }

    let mut route = Map::new();
    route.insert("rules".into(), Value::Array(rules));
    if settings.bypass_ru {
        route.insert(
            "rule_set".into(),
            json!([
                {
                    "tag": "geosite-ru",
                    "type": "remote",
                    "format": "binary",
                    "url": "https://raw.githubusercontent.com/SagerNet/sing-geosite/rule-set/geosite-category-ru.srs",
                    "download_detour": "proxy",
                    "update_interval": "7d"
                },
                {
                    "tag": "geoip-ru",
                    "type": "remote",
                    "format": "binary",
                    "url": "https://raw.githubusercontent.com/SagerNet/sing-geoip/rule-set/geoip-ru.srs",
                    "download_detour": "proxy",
                    "update_interval": "7d"
                }
            ]),
        );
    }
    route.insert(
        "final".into(),
        json!(match settings.route_default {
            RouteTarget::Proxy => "proxy",
            RouteTarget::Direct => "direct",
        }),
    );
    route.insert("auto_detect_interface".into(), json!(true));
    route.insert(
        "default_domain_resolver".into(),
        json!({ "server": "dns-local" }),
    );
    Value::Object(route)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, name: &str) -> ServerEntry {
        ServerEntry {
            id: id.into(),
            name: name.into(),
            protocol: "vless".into(),
            server: "203.0.113.10".into(),
            port: 443,
            uuid: "11111111-2222-3333-4444-555555555555".into(),
            flow: String::new(),
            security: Security::None,
            sni: String::new(),
            fingerprint: String::new(),
            public_key: String::new(),
            short_id: String::new(),
            insecure: false,
            alpn: Vec::new(),
            transport: Transport::Tcp,
            last_ping_ms: None,
            favorite: false,
            total_up: 0,
            total_down: 0,
            raw: String::new(),
        }
    }

    fn reality_entry(id: &str, name: &str) -> ServerEntry {
        let mut s = entry(id, name);
        s.security = Security::Reality;
        s.flow = "xtls-rprx-vision".into();
        s.sni = "cdn.example.org".into();
        s.public_key = "pubkey123".into();
        s.short_id = "ab12".into();
        s
    }

    fn gen(settings: &Settings, servers: &[ServerEntry], selected: &str) -> GeneratedConfig {
        let refs: Vec<&ServerEntry> = servers.iter().collect();
        generate(settings, &refs, selected, 9090, "s3cret").expect("generate")
    }

    fn at<'a>(cfg: &'a GeneratedConfig, ptr: &str) -> &'a Value {
        cfg.json
            .pointer(ptr)
            .unwrap_or_else(|| panic!("missing pointer {ptr}"))
    }

    #[test]
    fn reality_outbound_full_shape() {
        let servers = vec![reality_entry("a", "RU-1")];
        let cfg = gen(&Settings::default(), &servers, "a");
        // outbounds: [selector, urltest, server, direct]
        let ob = at(&cfg, "/outbounds/2");
        assert_eq!(ob["type"], "vless");
        assert_eq!(ob["tag"], "RU-1");
        assert_eq!(ob["server"], "203.0.113.10");
        assert_eq!(ob["server_port"], 443);
        assert_eq!(ob["uuid"], "11111111-2222-3333-4444-555555555555");
        assert_eq!(ob["flow"], "xtls-rprx-vision");
        assert_eq!(ob["packet_encoding"], "xudp");
        assert_eq!(ob["tls"]["enabled"], true);
        assert_eq!(ob["tls"]["server_name"], "cdn.example.org");
        assert_eq!(ob["tls"]["insecure"], false);
        // empty fingerprint + reality => utls with chrome fallback
        assert_eq!(ob["tls"]["utls"]["enabled"], true);
        assert_eq!(ob["tls"]["utls"]["fingerprint"], "chrome");
        assert_eq!(ob["tls"]["reality"]["enabled"], true);
        assert_eq!(ob["tls"]["reality"]["public_key"], "pubkey123");
        assert_eq!(ob["tls"]["reality"]["short_id"], "ab12");
        assert!(ob.get("transport").is_none(), "tcp must omit transport");
    }

    #[test]
    fn ws_outbound_host_header_and_no_flow() {
        let mut s = entry("a", "WS");
        s.security = Security::Tls;
        s.sni = "ws.example.org".into();
        s.transport = Transport::Ws {
            path: "/ws".into(),
            host: "ws.example.org".into(),
        };
        let mut no_host = entry("b", "WS2");
        no_host.transport = Transport::Ws {
            path: "/p".into(),
            host: String::new(),
        };
        let cfg = gen(&Settings::default(), &[s, no_host], "a");
        let ob = at(&cfg, "/outbounds/2");
        assert!(ob.get("flow").is_none(), "empty flow must be omitted");
        assert_eq!(ob["transport"]["type"], "ws");
        assert_eq!(ob["transport"]["path"], "/ws");
        assert_eq!(ob["transport"]["headers"]["Host"], "ws.example.org");
        let ob2 = at(&cfg, "/outbounds/3");
        assert!(
            ob2["transport"].get("headers").is_none(),
            "empty host must omit headers"
        );
    }

    #[test]
    fn grpc_and_httpupgrade_transports() {
        let mut g = entry("a", "G");
        g.transport = Transport::Grpc {
            service_name: "svc".into(),
        };
        let mut h = entry("b", "H");
        h.transport = Transport::Httpupgrade {
            path: "/up".into(),
            host: "up.example.org".into(),
        };
        let cfg = gen(&Settings::default(), &[g, h], "b");
        assert_eq!(at(&cfg, "/outbounds/2/transport/type"), "grpc");
        assert_eq!(at(&cfg, "/outbounds/2/transport/service_name"), "svc");
        assert_eq!(at(&cfg, "/outbounds/3/transport/type"), "httpupgrade");
        assert_eq!(at(&cfg, "/outbounds/3/transport/path"), "/up");
        assert_eq!(at(&cfg, "/outbounds/3/transport/host"), "up.example.org");
    }

    #[test]
    fn security_none_omits_tls_entirely() {
        let servers = vec![entry("a", "Plain")];
        let cfg = gen(&Settings::default(), &servers, "a");
        assert!(at(&cfg, "/outbounds/2").get("tls").is_none());
    }

    #[test]
    fn tls_alpn_and_utls_omission() {
        let mut bare = entry("a", "T1");
        bare.security = Security::Tls;
        bare.sni = "t.example.org".into();
        let mut full = entry("b", "T2");
        full.security = Security::Tls;
        full.sni = "t2.example.org".into();
        full.fingerprint = "firefox".into();
        full.alpn = vec!["h2".into(), "http/1.1".into()];
        let cfg = gen(&Settings::default(), &[bare, full], "a");
        let t1 = at(&cfg, "/outbounds/2/tls");
        assert!(t1.get("alpn").is_none(), "empty alpn must be omitted");
        assert!(
            t1.get("utls").is_none(),
            "empty fingerprint on plain tls must omit utls"
        );
        assert!(t1.get("reality").is_none());
        let t2 = at(&cfg, "/outbounds/3/tls");
        assert_eq!(t2["alpn"], json!(["h2", "http/1.1"]));
        assert_eq!(t2["utls"]["fingerprint"], "firefox");
    }

    #[test]
    fn tags_sanitized_deduped_with_fallbacks() {
        let servers = vec![
            entry("a", "  My   Server "),
            entry("b", "My Server"),
            entry("c", "My Server"),
            entry("d", "   "),
            entry("e", "proxy"),
        ];
        let cfg = gen(&Settings::default(), &servers, "a");
        assert_eq!(cfg.tag_by_server_id["a"], "My Server");
        assert_eq!(cfg.tag_by_server_id["b"], "My Server (2)");
        assert_eq!(cfg.tag_by_server_id["c"], "My Server (3)");
        assert_eq!(cfg.tag_by_server_id["d"], "server-4");
        assert_eq!(
            cfg.tag_by_server_id["e"], "proxy (2)",
            "reserved outbound tags must not be reused"
        );
        assert_eq!(at(&cfg, "/outbounds/3/tag"), "My Server (2)");
    }

    #[test]
    fn bypass_ru_adds_rule_and_rule_set() {
        let servers = vec![entry("a", "S")];
        let off = gen(&Settings::default(), &servers, "a");
        let rules = at(&off, "/route/rules").as_array().expect("rules");
        assert_eq!(rules.len(), 5);
        assert!(off.json.pointer("/route/rule_set").is_none());

        let mut settings = Settings::default();
        settings.bypass_ru = true;
        let on = gen(&settings, &servers, "a");
        let rules = at(&on, "/route/rules").as_array().expect("rules");
        assert_eq!(rules.len(), 6);
        assert_eq!(
            rules[5],
            json!({ "rule_set": ["geosite-ru", "geoip-ru"], "outbound": "direct" })
        );
        let rs = at(&on, "/route/rule_set").as_array().expect("rule_set");
        assert_eq!(rs.len(), 2);
        assert_eq!(rs[0]["tag"], "geosite-ru");
        assert_eq!(
            rs[0]["url"],
            "https://raw.githubusercontent.com/SagerNet/sing-geosite/rule-set/geosite-category-ru.srs"
        );
        assert_eq!(rs[1]["tag"], "geoip-ru");
        assert_eq!(rs[1]["download_detour"], "proxy");
        assert_eq!(rs[1]["update_interval"], "7d");
    }

    #[test]
    fn application_rules_precede_generic_routes() {
        let servers = vec![entry("a", "S")];
        let mut settings = Settings::default();
        settings.app_routes = vec![
            crate::models::AppRouteRule {
                id: "browser".into(),
                process_name: "firefox.exe".into(),
                action: AppRouteAction::Proxy,
            },
            crate::models::AppRouteRule {
                id: "game".into(),
                process_name: "game.exe".into(),
                action: AppRouteAction::Direct,
            },
            crate::models::AppRouteRule {
                id: "blocked".into(),
                process_name: "telemetry.exe".into(),
                action: AppRouteAction::Block,
            },
        ];

        let cfg = gen(&settings, &servers, "a");
        let rules = at(&cfg, "/route/rules").as_array().expect("rules");
        assert_eq!(
            rules[2],
            json!({ "process_name": ["firefox.exe"], "outbound": "proxy" })
        );
        assert_eq!(
            rules[3],
            json!({ "process_name": ["game.exe"], "outbound": "direct" })
        );
        assert_eq!(
            rules[4],
            json!({ "process_name": ["telemetry.exe"], "action": "reject" })
        );
        assert_eq!(
            rules[5],
            json!({ "ip_is_private": true, "outbound": "direct" })
        );
    }

    #[test]
    fn route_default_can_be_direct_and_empty_processes_are_ignored() {
        let servers = vec![entry("a", "S")];
        let mut settings = Settings::default();
        settings.route_default = RouteTarget::Direct;
        settings.app_routes = vec![crate::models::AppRouteRule {
            id: "empty".into(),
            process_name: "   ".into(),
            action: AppRouteAction::Block,
        }];

        let cfg = gen(&settings, &servers, "a");
        assert_eq!(at(&cfg, "/route/final"), "direct");
        assert_eq!(at(&cfg, "/route/rules").as_array().map(Vec::len), Some(5));
    }

    #[test]
    fn mixed_vs_tun_inbounds() {
        let servers = vec![entry("a", "S")];
        let mut settings = Settings::default();
        settings.mixed_port = 7777;
        let mixed = gen(&settings, &servers, "a");
        let inb = at(&mixed, "/inbounds/0");
        assert_eq!(inb["type"], "mixed");
        assert_eq!(inb["tag"], "mixed-in");
        assert_eq!(inb["listen"], "127.0.0.1");
        assert_eq!(inb["listen_port"], 7777);
        assert_eq!(at(&mixed, "/inbounds").as_array().map(Vec::len), Some(1));

        settings.mode = Mode::Tun;
        settings.tun_mtu = 1500;
        settings.tun_strict_route = false;
        settings.tun_stack = "gvisor".into();
        let tun = gen(&settings, &servers, "a");
        let inb = at(&tun, "/inbounds/0");
        assert_eq!(inb["type"], "tun");
        assert_eq!(inb["tag"], "tun-in");
        assert_eq!(inb["interface_name"], "umbra-tun");
        assert_eq!(
            inb["address"],
            json!(["172.19.0.1/30", "fdfe:dcba:9876::1/126"])
        );
        assert_eq!(inb["mtu"], 1500);
        assert_eq!(inb["auto_route"], true);
        assert_eq!(inb["strict_route"], false);
        assert_eq!(inb["stack"], "gvisor");
    }

    #[test]
    fn selector_and_urltest_wiring() {
        let servers = vec![entry("a", "One"), entry("b", "Two")];
        let cfg = gen(&Settings::default(), &servers, "b");
        let sel = at(&cfg, "/outbounds/0");
        assert_eq!(sel["type"], "selector");
        assert_eq!(sel["tag"], "proxy");
        assert_eq!(sel["outbounds"], json!(["auto", "One", "Two"]));
        assert_eq!(sel["default"], "Two");
        assert_eq!(sel["interrupt_exist_connections"], true);
        let auto = at(&cfg, "/outbounds/1");
        assert_eq!(auto["type"], "urltest");
        assert_eq!(auto["tag"], "auto");
        assert_eq!(auto["outbounds"], json!(["One", "Two"]));
        assert_eq!(auto["url"], Settings::default().ping_url);
        assert_eq!(auto["interval"], "3m");
        assert_eq!(auto["tolerance"], 50);
        let last = at(&cfg, "/outbounds/4");
        assert_eq!(last, &json!({ "type": "direct", "tag": "direct" }));
    }

    #[test]
    fn unknown_selected_id_is_not_found() {
        let servers = vec![entry("a", "S")];
        let refs: Vec<&ServerEntry> = servers.iter().collect();
        let err = generate(&Settings::default(), &refs, "nope", 9090, "s").expect_err("must fail");
        assert!(matches!(err, AppError::NotFound(_)));
    }

    #[test]
    fn top_level_log_experimental_dns_route() {
        let servers = vec![entry("a", "S")];
        let mut settings = Settings::default();
        settings.log_level = "debug".into();
        let cfg = gen(&settings, &servers, "a");
        assert_eq!(at(&cfg, "/log/level"), "debug");
        assert_eq!(at(&cfg, "/log/timestamp"), true);
        assert_eq!(
            at(&cfg, "/experimental/clash_api/external_controller"),
            "127.0.0.1:9090"
        );
        assert_eq!(at(&cfg, "/experimental/clash_api/secret"), "s3cret");
        assert_eq!(at(&cfg, "/experimental/clash_api/default_mode"), "Rule");
        assert_eq!(at(&cfg, "/experimental/cache_file/enabled"), true);
        assert_eq!(at(&cfg, "/experimental/cache_file/path"), "cache.db");
        assert_eq!(at(&cfg, "/dns/servers/0/tag"), "dns-remote");
        assert_eq!(at(&cfg, "/dns/servers/0/type"), "https");
        assert_eq!(at(&cfg, "/dns/servers/0/detour"), "proxy");
        assert_eq!(at(&cfg, "/dns/servers/1/type"), "local");
        assert_eq!(at(&cfg, "/dns/final"), "dns-remote");
        assert_eq!(at(&cfg, "/dns/strategy"), "prefer_ipv4");
        assert_eq!(at(&cfg, "/dns/independent_cache"), true);
        assert_eq!(at(&cfg, "/route/final"), "proxy");
        assert_eq!(at(&cfg, "/route/auto_detect_interface"), true);
        assert_eq!(
            at(&cfg, "/route/default_domain_resolver/server"),
            "dns-local"
        );
        assert_eq!(at(&cfg, "/route/rules/0"), &json!({ "action": "sniff" }));
        assert_eq!(
            at(&cfg, "/route/rules/1"),
            &json!({ "protocol": "dns", "action": "hijack-dns" })
        );
        assert_eq!(cfg.clash_port, 9090);
        assert_eq!(cfg.clash_secret, "s3cret");
    }
}
