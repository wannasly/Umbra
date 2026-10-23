//! Opt-in real core test. Only loopback traffic is proxied; all other traffic stays direct.
use crate::{
    commands::connection,
    models::*,
    parser,
    proxy::{elevation, system_proxy},
    runtime::AppHandle,
    singbox::{process, version},
    state::AppState,
};
use std::{path::PathBuf, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

async fn http_server(body: &'static str) -> (u16, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let mut request = [0u8; 4096];
                if stream.read(&mut request).await.unwrap_or(0) == 0 {
                    return;
                }
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes()).await;
            });
        }
    });
    (port, task)
}

async fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn route(address: &str, action: AppRouteAction) -> RouteRule {
    RouteRule {
        id: "integration".into(),
        enabled: true,
        rule_type: RuleType::IpCidr,
        value: address.into(),
        process_matcher: None,
        domain_matcher: None,
        action,
        description: None,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "changes system proxy and creates a temporary TUN; run explicitly on an idle elevated Windows session"]
async fn real_core_proxy_routing_tun_and_cleanup() {
    assert!(
        elevation::is_elevated(),
        "TUN verification requires elevation"
    );
    let before = system_proxy::read_current().unwrap();
    assert_eq!(before.enable, 0, "do not interrupt an existing proxy");
    let adapter = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            "@(Get-NetAdapter -Name umbra-tun -IncludeHidden -ErrorAction SilentlyContinue).Count",
        ])
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&adapter.stdout).trim(),
        "0",
        "do not interrupt an existing TUN"
    );
    let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../artifacts/native")
        .join(format!("live-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&data_dir).unwrap();
    let (target_port, target) = http_server("direct").await;
    let (relay_port, relay) = http_server("through-vless").await;
    let remote_port = free_port().await;
    let mixed_port = free_port().await;
    let uuid = uuid::Uuid::new_v4().to_string();
    let remote_config = serde_json::json!({
        "log":{"level":"debug"},
        "inbounds":[{"type":"vless","listen":"127.0.0.1","listen_port":remote_port,"users":[{"uuid":uuid}]}],
        "outbounds":[{"type":"direct","tag":"direct"}],
        "route":{"rules":[{"action":"route","outbound":"direct","override_address":"127.0.0.1","override_port":relay_port}]}
    });
    let config_path = data_dir.join("remote.json");
    std::fs::write(
        &config_path,
        serde_json::to_vec_pretty(&remote_config).unwrap(),
    )
    .unwrap();
    let core = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/sing-box.exe");
    let mut remote = tokio::process::Command::new(&core)
        .args(["run", "-c"])
        .arg(&config_path)
        .stdout(Stdio::null())
        .stderr(std::fs::File::create(data_dir.join("remote.log")).unwrap())
        .kill_on_drop(true)
        .creation_flags(0x08000000)
        .spawn()
        .unwrap();
    let server = parser::parse_any(&format!(
        "vless://{uuid}@127.0.0.1:{remote_port}?security=none#local-test"
    ))
    .unwrap();
    let id = server.id.clone();
    let settings = Settings {
        mixed_port,
        route_default: RouteTarget::Direct,
        selected_server_id: Some(id.clone()),
        routing_rules: vec![route("127.0.0.1/32", AppRouteAction::Proxy)],
        ping_url: format!("http://127.0.0.1:{target_port}/"),
        ..Settings::default()
    };
    let app = AppHandle::new(AppState::new(
        data_dir.clone(),
        settings,
        ProfileStore {
            manual: vec![server],
            ..ProfileStore::default()
        },
    ));
    version::install_bundled_core(&data_dir);
    let test_app = app.clone();
    let scenario = tokio::spawn(async move {
        let run = async {
            tokio::time::sleep(Duration::from_millis(300)).await;
            connection::do_connect(&test_app, id.clone()).await.unwrap();
            let proxy = system_proxy::read_current().unwrap();
            assert_eq!(proxy.enable, 1);
            assert_eq!(
                proxy.server.as_deref(),
                Some(format!("127.0.0.1:{mixed_port}").as_str())
            );
            let client = reqwest::Client::builder()
                .proxy(reqwest::Proxy::all(format!("http://127.0.0.1:{mixed_port}")).unwrap())
                .timeout(Duration::from_secs(5))
                .build()
                .unwrap();
            let url = format!("http://127.0.0.1:{target_port}/");
            assert_eq!(
                client.get(&url).send().await.unwrap().text().await.unwrap(),
                "through-vless"
            );
            connection::do_disconnect(&test_app).await.unwrap();
            assert_eq!(
                serde_json::to_value(system_proxy::read_current().unwrap()).unwrap(),
                serde_json::to_value(&before).unwrap()
            );
            test_app.state().settings.write().await.routing_rules[0].action =
                AppRouteAction::Direct;
            connection::do_connect(&test_app, id.clone()).await.unwrap();
            assert_eq!(
                client.get(&url).send().await.unwrap().text().await.unwrap(),
                "direct"
            );
            connection::do_disconnect(&test_app).await.unwrap();
            test_app.state().settings.write().await.routing_rules[0].action = AppRouteAction::Block;
            connection::do_connect(&test_app, id.clone()).await.unwrap();
            if let Ok(response) = client.get(&url).send().await {
                assert!(
                    response.status().is_server_error(),
                    "blocked HTTP proxy requests must fail"
                );
            }
            connection::do_disconnect(&test_app).await.unwrap();
            {
                let mut settings = test_app.state().settings.write().await;
                settings.mode = Mode::Tun;
                settings.routing_rules = vec![route("172.19.0.2/32", AppRouteAction::Proxy)];
            }
            connection::do_connect(&test_app, id).await.unwrap();
            let direct = reqwest::Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(8))
                .build()
                .unwrap();
            assert_eq!(
                direct
                    .get(format!("http://172.19.0.2:{target_port}/"))
                    .send()
                    .await
                    .unwrap()
                    .text()
                    .await
                    .unwrap(),
                "through-vless",
                "traffic must actually traverse TUN and VLESS"
            );
            connection::do_disconnect(&test_app).await.unwrap();
        };
        tokio::time::timeout(Duration::from_secs(70), run)
            .await
            .expect("integration timeout");
    });
    // Always stop and restore even if the scenario panics or times out.
    let result = scenario.await;
    process::stop(&app).await;
    let _ = remote.kill().await;
    let _ = remote.wait().await;
    target.abort();
    relay.abort();
    std::fs::write(
        data_dir.join("client.log"),
        app.state()
            .logs
            .tail(2000)
            .iter()
            .map(|l| format!("{} {}\n", l.level, l.message))
            .collect::<String>(),
    )
    .unwrap();
    assert!(!app.state().settings.read().await.proxy_owned);
    assert!(app.state().core.lock().await.is_none());
    assert_eq!(
        serde_json::to_value(system_proxy::read_current().unwrap()).unwrap(),
        serde_json::to_value(crate::storage::load_settings(&data_dir).proxy_backup).unwrap()
    );
    result.expect("real core scenario failed; cleanup completed");
    eprintln!(
        "Real VLESS proxy/direct/block/TUN verified; logs: {}",
        data_dir.display()
    );
}
