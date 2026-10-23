mod commands;
mod controller;
mod error;
mod events;
mod graphics;
mod hwid;
mod instance;
#[cfg(all(test, windows))]
mod live_integration;
mod models;
mod native;
mod native_flags;
mod net;
mod parser;
mod proxy;
mod runtime;
mod singbox;
mod state;
mod storage;
mod storage_migration;
mod subscription;
mod tray;

pub fn run() {
    if let Err(error) = run_native() {
        eprintln!("Umbra: {error}");
        #[cfg(windows)]
        unsafe {
            use windows::{
                core::HSTRING,
                Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK},
            };
            MessageBoxW(
                None,
                &HSTRING::from(error.to_string()),
                windows::core::w!("Umbra"),
                MB_OK | MB_ICONERROR,
            );
        }
        std::process::exit(1);
    }
}

fn run_native() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var_os("UMBRA_DATA_DIR").is_none() {
        instance::ensure_legacy_stopped()?;
    }
    let data_dir = if let Some(path) = std::env::var_os("UMBRA_DATA_DIR") {
        std::path::PathBuf::from(path)
    } else {
        directories::BaseDirs::new()
            .ok_or("cannot resolve application data directory")?
            .config_dir()
            .join("com.umbra.proxy")
    };
    std::fs::create_dir_all(&data_dir)?;
    let Some(_instance) = instance::acquire(&data_dir)? else {
        return Ok(());
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    let _enter = runtime.enter();
    let mut settings = storage::load_settings(&data_dir);
    if settings.hwid.is_empty() {
        settings.hwid = hwid::hwid(None);
        storage::save_settings(&data_dir, &settings)?;
    }
    let profiles = storage::load_profiles(&data_dir)?;
    singbox::version::install_bundled_core(&data_dir);
    #[cfg(feature = "smoke-test")]
    let smoke = std::env::var_os("UMBRA_SMOKE_CAPTURE").is_some();
    #[cfg(not(feature = "smoke-test"))]
    let smoke = false;
    if smoke && std::env::var_os("UMBRA_DATA_DIR").is_none() {
        return Err("smoke capture requires an isolated UMBRA_DATA_DIR".into());
    }
    let minimized =
        !smoke && (settings.start_minimized || std::env::args().any(|a| a == "--minimized"));
    let auto_connect = !smoke && (settings.connect_on_startup || proxy::elevation::is_resume_tun());
    let initial_settings = settings.clone();
    let app = runtime::AppHandle::new(state::AppState::new(data_dir, settings, profiles));
    // Finish crash recovery before accepting connection operations.
    runtime.block_on(proxy::system_proxy::startup_recovery(app.state()));
    commands::profiles::spawn_auto_updater(app.clone());
    if auto_connect {
        let app = app.clone();
        tokio::spawn(async move {
            commands::connection::startup_connect(&app).await;
        });
    }
    let result = native::run(app.clone(), initial_settings, minimized);
    runtime.block_on(async {
        let _ops = app.state().conn_ops.lock().await;
        singbox::process::stop(&app).await;
    });
    drop(_enter);
    runtime.shutdown_timeout(std::time::Duration::from_secs(3));
    result?;
    Ok(())
}
