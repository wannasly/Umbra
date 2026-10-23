//! Slint retained-mode interface. Backend wakeups are coalesced; idle timers do
//! not request frames or replace unchanged models.
use crate::{
    controller::{self, Completion, Task},
    error::{AppError, AppResult},
    events::*,
    models::*,
    runtime::AppHandle,
    tray::{Action as TrayAction, NativeTray},
};
use serde_json::{json, Value};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    rc::Rc,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    time::Duration,
};

slint::include_modules!();

thread_local! { static UI: RefCell<Option<UiState>> = const { RefCell::new(None) }; }

fn tr(ru: bool, a: &str, b: &str) -> SharedString {
    if ru {
        a.into()
    } else {
        b.into()
    }
}
fn text(value: impl ToString) -> SharedString {
    value.to_string().into()
}
fn color(rgb: u32) -> slint::Color {
    slint::Color::from_rgb_u8((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}
fn model<T: Clone + 'static>(rows: Vec<T>) -> ModelRc<T> {
    Rc::new(VecModel::from(rows)).into()
}

fn format_bytes(n: f64, ru: bool, rate: bool) -> String {
    let units = if rate {
        if ru {
            ["Б/с", "КБ/с", "МБ/с", "ГБ/с", "ТБ/с"]
        } else {
            ["B/s", "KB/s", "MB/s", "GB/s", "TB/s"]
        }
    } else if ru {
        ["Б", "КБ", "МБ", "ГБ", "ТБ"]
    } else {
        ["B", "KB", "MB", "GB", "TB"]
    };
    let mut v = n.max(0.0);
    let mut i = 0;
    while v >= 1024.0 && i < 4 {
        v /= 1024.0;
        i += 1;
    }
    format!(
        "{} {}",
        if i == 0 || v >= 100.0 {
            format!("{v:.0}")
        } else {
            format!("{v:.1}")
        },
        units[i]
    )
}
fn ping_tone(ms: Option<u32>) -> slint::Color {
    color(match ms {
        Some(0..=79) => 0x42e58e,
        Some(80..=200) => 0xf2b544,
        Some(_) => 0xff738f,
        None => 0x8ba6af,
    })
}
fn kind_labels(node: &ProxyKind) -> (&'static str, String, String) {
    match node {
        ProxyKind::Hysteria2(_) => ("HY2", String::new(), String::new()),
        ProxyKind::Vless(v) => (
            "VLESS",
            match v.security {
                Security::Reality => "REALITY",
                Security::Tls => "TLS",
                Security::None => "",
            }
            .into(),
            match &v.transport {
                Transport::Tcp => v.flow.clone(),
                Transport::Ws { .. } => "WS".into(),
                Transport::Grpc { .. } => "gRPC".into(),
                Transport::Httpupgrade { .. } => "HTTPUpgrade".into(),
            },
        ),
    }
}

struct UiState {
    app: AppHandle,
    settings: Settings,
    conn: ConnectionState,
    core: Option<CoreStatus>,
    core_update: Option<UpdateCheck>,
    tray: Option<NativeTray>,
    receiver: Option<mpsc::Receiver<Completion>>,
    close_modal: bool,
    models_dirty: bool,
    traffic: [f64; 4],
    history: VecDeque<[f64; 2]>,
    logs_revision: u64,
    log_level: String,
    flags: HashMap<String, slint::Image>,
    applied_routes: Option<(RouteTarget, Vec<RouteRule>)>,
    processes: Vec<RunningProcess>,
    message_since: std::cell::Cell<Option<std::time::Instant>>,
    frozen_logs: Option<Vec<LogLine>>,
}

impl UiState {
    fn new(app: AppHandle, settings: Settings, tray: Option<NativeTray>) -> Self {
        let conn = ConnectionState::disconnected(settings.mode);
        Self {
            app,
            settings,
            conn,
            core: None,
            core_update: None,
            tray,
            receiver: None,
            close_modal: false,
            models_dirty: true,
            traffic: [0.0; 4],
            history: VecDeque::new(),
            logs_revision: u64::MAX,
            log_level: String::new(),
            flags: HashMap::new(),
            applied_routes: None,
            processes: Vec::new(),
            message_since: std::cell::Cell::new(None),
            frozen_logs: None,
        }
    }

    fn message(&self, ui: &MainWindow, message: impl ToString, error: bool) {
        self.message_since.set(Some(std::time::Instant::now()));
        ui.global::<App>().set_message(text(message));
        ui.global::<App>().set_message_error(error);
    }
    fn show(&self, ui: &MainWindow) {
        let _ = ui.show();
        ui.window().set_minimized(false);
        ui.window().request_redraw();
        use slint::winit_030::WinitWindowAccessor;
        ui.window().with_winit_window(|window| {
            window.focus_window();
            window.request_redraw();
        });
    }
    fn close(&self, ui: &MainWindow) {
        if self.settings.minimize_to_tray && self.tray.is_some() {
            let _ = ui.hide();
        } else {
            let _ = slint::quit_event_loop();
        }
    }
    fn submit(&mut self, ui: &MainWindow, task: Task, close_modal: bool) {
        if self.receiver.is_some() {
            self.message(
                ui,
                tr(
                    self.settings.language == "ru",
                    "Дождитесь завершения текущей операции",
                    "Wait for the current operation to finish",
                ),
                false,
            );
            return;
        }
        let (tx, rx) = mpsc::sync_channel(1);
        self.receiver = Some(rx);
        self.close_modal = close_modal;
        ui.global::<App>().set_busy(true);
        let app = self.app.clone();
        tokio::spawn(async move {
            let completion = controller::execute(app.clone(), task).await;
            let _ = tx.send(completion);
            app.wake();
        });
    }

    fn refresh(&mut self, ui: &MainWindow) {
        let data = ui.global::<App>();
        data.set_version(env!("CARGO_PKG_VERSION").into());
        if let Some(receiver) = &self.receiver {
            match receiver.try_recv() {
                Ok(done) => {
                    self.receiver = None;
                    data.set_busy(false);
                    self.models_dirty = true;
                    if let Some(update) = done.update {
                        self.core_update = Some(update);
                    }
                    if let Some(core) = done.core {
                        self.core = Some(core);
                    }
                    if let Some(processes) = done.processes {
                        self.processes = processes;
                        self.filter_processes(ui);
                        data.set_modal(7);
                        data.set_modal_title(tr(
                            self.settings.language == "ru",
                            "Выберите процесс",
                            "Choose process",
                        ));
                    }
                    match done.result {
                        Ok(message) => {
                            if self.close_modal {
                                data.set_modal(0);
                            }
                            if !message.is_empty() {
                                self.message(ui, message, false);
                            }
                        }
                        Err(AppError::NeedsElevation) => {
                            data.set_modal(6);
                            data.set_modal_title(tr(
                                self.settings.language == "ru",
                                "Нужны права администратора",
                                "Administrator rights required",
                            ));
                            self.show(ui);
                        }
                        Err(error) => self.message(ui, error, true),
                    }
                    let _ = done.saved;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.receiver = None;
                    data.set_busy(false);
                    self.message(ui, "Operation interrupted", true);
                }
                Err(mpsc::TryRecvError::Empty) => (),
            }
        }
        let app = self.app.clone();
        if let Ok(settings) = app.state().settings.try_read() {
            self.settings = settings.clone();
        }
        if let Ok(conn) = app.state().conn.try_read() {
            if conn.error != self.conn.error {
                if let Some(error) = &conn.error {
                    self.message(ui, error, true);
                }
            }
            if conn.status == ConnStatus::Connected
                && (conn.since_ms != self.conn.since_ms || self.applied_routes.is_none())
            {
                self.applied_routes = Some((
                    self.settings.route_default,
                    self.settings.routing_rules.clone(),
                ));
            } else if conn.status == ConnStatus::Disconnected {
                self.applied_routes = None;
            }
            self.conn = conn.clone();
        }
        data.set_route_dirty(
            self.conn.status == ConnStatus::Connected
                && self
                    .applied_routes
                    .as_ref()
                    .is_some_and(|(default, rules)| {
                        *default != self.settings.route_default
                            || *rules != self.settings.routing_rules
                    }),
        );
        for (event, value) in app.take_events() {
            match event {
                EV_TRAFFIC => {
                    self.traffic = ["upBps", "downBps", "upTotal", "downTotal"]
                        .map(|key| value[key].as_f64().unwrap_or(0.0));
                    self.history.push_back([self.traffic[0], self.traffic[1]]);
                    if self.history.len() > 61 {
                        self.history.pop_front();
                    }
                    if ui.window().is_visible() && data.get_page() == 0 {
                        self.update_chart(ui);
                    }
                }
                EV_SUB_UPDATED | EV_PING_RESULT => self.models_dirty = true,
                EV_NEEDS_ELEVATION => {
                    data.set_modal(6);
                    data.set_modal_title(tr(
                        self.settings.language == "ru",
                        "Нужны права администратора",
                        "Administrator rights required",
                    ));
                    self.show(ui);
                }
                EV_DOWNLOAD_PROGRESS => self.message(
                    ui,
                    format!(
                        "sing-box: {} / {}",
                        format_bytes(
                            value["downloaded"].as_f64().unwrap_or(0.0),
                            self.settings.language == "ru",
                            false
                        ),
                        format_bytes(
                            value["total"].as_f64().unwrap_or(0.0),
                            self.settings.language == "ru",
                            false
                        )
                    ),
                    false,
                ),
                "app://quit" => {
                    let _ = slint::quit_event_loop();
                }
                _ => (),
            }
        }
        let actions = self
            .tray
            .as_ref()
            .map(NativeTray::actions)
            .unwrap_or_default();
        for action in actions {
            match action {
                TrayAction::Show => self.show(ui),
                TrayAction::Quit => {
                    let _ = slint::quit_event_loop();
                }
                TrayAction::Toggle => self.submit(ui, Task::Toggle, false),
                TrayAction::System => self.submit(ui, Task::Mode(Mode::SystemProxy), false),
                TrayAction::Tun => self.submit(ui, Task::Mode(Mode::Tun), false),
            }
        }
        if let Some(tray) = &mut self.tray {
            tray.sync(&self.conn, self.settings.language == "ru");
        }
        if !ui.window().is_visible() {
            return;
        }
        let ru = self.settings.language == "ru";
        data.set_ru(ru);
        data.set_mode(if self.settings.mode == Mode::Tun {
            1
        } else {
            0
        });
        data.set_status(match self.conn.status {
            ConnStatus::Disconnected => 0,
            ConnStatus::Connecting => 1,
            ConnStatus::Connected => 2,
            ConnStatus::Stopping => 3,
        });
        data.set_status_text(match self.conn.status {
            ConnStatus::Disconnected => tr(ru, "Отключено", "Disconnected"),
            ConnStatus::Connecting => tr(ru, "Подключение…", "Connecting…"),
            ConnStatus::Connected => tr(ru, "Подключено", "Connected"),
            ConnStatus::Stopping => tr(ru, "Отключение…", "Disconnecting…"),
        });
        ui.global::<Theme>()
            .set_reduce_motion(self.settings.reduce_motion);
        let (accent, accent2) = match self.settings.accent.as_str() {
            "cyan" => (0x5ad0e0, 0xb4edf3),
            "emerald" => (0x5c9fd1, 0x73d5e3),
            "amber" => (0xd0e8ec, 0x94c9d2),
            _ => (0x73d5e3, 0xa0e8f0),
        };
        ui.global::<Theme>().set_accent(color(accent));
        ui.global::<Theme>().set_accent2(color(accent2));
        data.set_route_default(if self.settings.route_default == RouteTarget::Direct {
            1
        } else {
            0
        });
        if self.conn.status == ConnStatus::Disconnected {
            self.traffic = [0.0; 4];
        }
        data.set_rate_down(text(format_bytes(self.traffic[1], ru, true)));
        data.set_rate_up(text(format_bytes(self.traffic[0], ru, true)));
        data.set_session(text(format_bytes(
            self.traffic[2] + self.traffic[3],
            ru,
            false,
        )));
        data.set_session_down(text(format_bytes(self.traffic[3], ru, false)));
        data.set_session_up(text(format_bytes(self.traffic[2], ru, false)));
        self.update_uptime(ui);
        if let Ok(profiles) = app.state().profiles.try_read() {
            let id = if self.conn.status != ConnStatus::Disconnected {
                self.conn.server_id.as_deref()
            } else {
                self.settings.selected_server_id.as_deref()
            };
            let server = id.and_then(|id| profiles.find_server(id));
            data.set_server_id(text(id.unwrap_or("")));
            if let Some(server) = server {
                let (code, name) = server_name(&server.name);
                data.set_server_name(text(if code.is_empty() {
                    name
                } else {
                    format!("{code} {name}")
                }));
                data.set_server_host(text(format!("{}:{}", server.server, server.port)));
                let (protocol, security, transport) = kind_labels(&server.kind);
                data.set_protocol(text(protocol));
                data.set_security(text(security));
                data.set_transport(text(transport));
                data.set_ping(text(
                    server
                        .last_ping_ms
                        .map(|ms| ms.to_string())
                        .unwrap_or_else(|| "—".into()),
                ));
                data.set_ping_tone(ping_tone(server.last_ping_ms));
                data.set_server_total(text(format_bytes(
                    server.total_down.saturating_add(server.total_up) as f64,
                    ru,
                    false,
                )));
                data.set_server_down(text(format_bytes(server.total_down as f64, ru, false)));
                data.set_server_up(text(format_bytes(server.total_up as f64, ru, false)));
            } else {
                data.set_server_name(
                    self.conn
                        .server_name
                        .as_ref()
                        .map(text)
                        .unwrap_or_else(|| tr(ru, "Сервер не выбран", "No server selected")),
                );
                data.set_server_host("".into());
                data.set_protocol("".into());
                data.set_security("".into());
                data.set_transport("".into());
                data.set_ping("—".into());
            }
            if self.models_dirty {
                self.update_models(ui, &profiles);
                data.set_settings(model(settings_rows(
                    &self.settings,
                    self.core.as_ref(),
                    self.core_update.as_ref(),
                )));
                self.models_dirty = false;
            }
        }
        if data.get_page() == 4 {
            self.update_logs(ui);
        }
    }

    fn tick(&mut self, ui: &MainWindow) {
        if self
            .message_since
            .get()
            .is_some_and(|t| t.elapsed() > Duration::from_secs(8))
        {
            ui.global::<App>().set_message("".into());
            self.message_since.set(None);
        }
        if crate::instance::take_show_request() {
            self.show(ui);
            self.models_dirty = true;
            self.refresh(ui);
        }
        if !ui.window().is_visible() {
            return;
        }
        if ui.global::<App>().get_page() == 0 && self.conn.status == ConnStatus::Connected {
            self.update_uptime(ui);
        }
        if ui.global::<App>().get_page() == 4 {
            self.update_logs(ui);
        }
    }
    fn update_uptime(&self, ui: &MainWindow) {
        let value = self
            .conn
            .since_ms
            .map(|since| {
                let seconds = ((crate::singbox::process::now_ms() - since) / 1000).max(0);
                format!(
                    "{:02}:{:02}:{:02}",
                    seconds / 3600,
                    seconds / 60 % 60,
                    seconds % 60
                )
            })
            .unwrap_or_else(|| "—".into());
        ui.global::<App>().set_uptime(text(value));
    }
    fn update_chart(&self, ui: &MainWindow) {
        let idle = self.history.iter().all(|s| s[0] == 0.0 && s[1] == 0.0);
        ui.global::<App>().set_traffic_idle(idle);
        if idle {
            return;
        }
        let peak = self
            .history
            .iter()
            .flatten()
            .copied()
            .fold(1.0_f64, f64::max);
        let mut svg=String::from("<svg xmlns='http://www.w3.org/2000/svg' width='944' height='104' viewBox='0 0 944 104'>");
        for (i, tint, alpha) in [(1, "#a0e8f0", 0.25), (0, "#73d5e3", 0.15)] {
            let points: Vec<_> = self
                .history
                .iter()
                .enumerate()
                .map(|(n, p)| {
                    (
                        n as f64 * 944.0 / 60.0,
                        104.0 - (p[i] / peak).min(1.0).sqrt() * 94.0,
                    )
                })
                .collect();
            if let Some(&(x, y)) = points.first() {
                let mut path = format!("M{x:.2} {y:.2}");
                for p in points.windows(2) {
                    path += &format!(" L{:.2} {:.2}", p[1].0, p[1].1);
                }
                let last = points.last().unwrap().0;
                svg+=&format!("<path d='{path} L{last:.2} 104 L0 104Z' fill='{tint}' opacity='{alpha}'/><path d='{path}' fill='none' stroke='{tint}' stroke-width='2'/>");
            }
        }
        svg += "</svg>";
        if let Ok(image) = slint::Image::load_from_svg_data(svg.as_bytes()) {
            ui.global::<App>().set_chart(image);
        }
    }

    fn update_logs(&mut self, ui: &MainWindow) {
        let data = ui.global::<App>();
        let paused = data.get_log_paused();
        if paused && self.frozen_logs.is_none() {
            self.frozen_logs = Some(self.app.state().logs.tail(2000));
            self.logs_revision = u64::MAX;
        } else if !paused && self.frozen_logs.take().is_some() {
            self.logs_revision = u64::MAX;
        }
        let revision = if paused {
            0
        } else {
            self.app.state().logs.revision()
        };
        if revision == self.logs_revision {
            return;
        }
        self.logs_revision = revision;
        let query = data.get_log_query().to_lowercase();
        let lines = self
            .frozen_logs
            .clone()
            .unwrap_or_else(|| self.app.state().logs.tail(2000));
        let rows: Vec<_> = lines
            .iter()
            .filter(|line| {
                (self.log_level.is_empty() || line.level == self.log_level)
                    && (query.is_empty()
                        || line.message.to_lowercase().contains(&query)
                        || line.level.contains(&query))
            })
            .map(|line| LogRow {
                clock: text(
                    chrono::DateTime::from_timestamp_millis(line.ts)
                        .map(|d| {
                            d.with_timezone(&chrono::Local)
                                .format("%H:%M:%S")
                                .to_string()
                        })
                        .unwrap_or_default(),
                ),
                tag: text(match line.level.as_str() {
                    "trace" => "TRC",
                    "debug" => "DBG",
                    "info" => "INF",
                    "warn" => "WRN",
                    "error" | "fatal" => "ERR",
                    _ => "INF",
                }),
                tint: color(match line.level.as_str() {
                    "warn" => 0xf2b544,
                    "error" | "fatal" => 0xff738f,
                    "debug" | "trace" => 0x819da5,
                    _ => 0xa0e8f0,
                }),
                message: text(&line.message),
            })
            .collect();
        let log = rows
            .iter()
            .map(|r| format!("{}  {}  {}", r.clock, r.tag, r.message))
            .collect::<Vec<_>>()
            .join("\n");
        data.set_log_text(text(log));
        data.set_logs(model(rows));
    }

    fn flag(&mut self, code: &str) -> slint::Image {
        if let Some(image) = self.flags.get(code) {
            return image.clone();
        }
        let image = crate::native_flags::svg(code)
            .and_then(|svg| slint::Image::load_from_svg_data(svg).ok())
            .unwrap_or_default();
        self.flags.insert(code.into(), image.clone());
        image
    }

    fn filter_processes(&self, ui: &MainWindow) {
        let data = ui.global::<App>();
        let query = data.get_process_query().to_lowercase();
        data.set_processes(model(
            self.processes
                .iter()
                .filter(|p| {
                    query.is_empty()
                        || format!(
                            "{} {} {}",
                            p.name,
                            p.title.as_deref().unwrap_or(""),
                            p.path.as_deref().unwrap_or("")
                        )
                        .to_lowercase()
                        .contains(&query)
                })
                .map(|p| ProcessRow {
                    name: text(&p.name),
                    description: text(p.title.as_deref().unwrap_or("")),
                    path: text(p.path.as_deref().unwrap_or("")),
                })
                .collect(),
        ));
    }
    fn update_models(&mut self, ui: &MainWindow, profiles: &ProfileStore) {
        let data = ui.global::<App>();
        let ru = self.settings.language == "ru";
        let query = data.get_query().to_lowercase();
        let mut rows = Vec::new();
        let mut groups = vec![
            (
                "favorites".to_owned(),
                if ru {
                    "Избранное".to_owned()
                } else {
                    "Favorites".to_owned()
                },
                profiles
                    .all_servers()
                    .filter(|s| s.favorite)
                    .collect::<Vec<_>>(),
                None,
            ),
            (
                "manual".to_owned(),
                if ru {
                    "Мои серверы".to_owned()
                } else {
                    "My servers".to_owned()
                },
                profiles.manual.iter().collect(),
                None,
            ),
        ];
        groups.extend(profiles.subscriptions.iter().map(|sub| {
            (
                sub.id.clone(),
                sub.name.clone(),
                sub.servers.iter().collect(),
                sub.quota,
            )
        }));
        for (id, name, mut servers, quota) in groups {
            servers.retain(|server| {
                query.is_empty()
                    || server.name.to_lowercase().contains(&query)
                    || server.server.to_lowercase().contains(&query)
            });
            let (mut notices, mut servers): (Vec<_>, Vec<_>) =
                servers.into_iter().partition(|s| is_info(s));
            match self.settings.server_sort.as_str() {
                "name" => servers.sort_by_key(|s| server_name(&s.name).1.to_lowercase()),
                "ping" => servers.sort_by_key(|s| {
                    (
                        s.last_ping_ms.unwrap_or(u32::MAX),
                        server_name(&s.name).1.to_lowercase(),
                    )
                }),
                _ => (),
            }
            if servers.is_empty() {
                continue;
            }
            let collapsed = self.settings.collapsed_groups.contains(&id);
            rows.push(ServerRow {
                id: text(&id),
                group_id: text(&id),
                name: text(name),
                group: true,
                collapsed,
                movable: id != "manual" && id != "favorites",
                can_up: profiles
                    .subscriptions
                    .iter()
                    .position(|s| s.id == id)
                    .is_some_and(|i| i > 0),
                can_down: profiles
                    .subscriptions
                    .iter()
                    .position(|s| s.id == id)
                    .is_some_and(|i| i + 1 < profiles.subscriptions.len()),
                count: text(format!(
                    "{} {}",
                    servers.len(),
                    if ru { "серверов" } else { "servers" }
                )),
                quota: text(quota.map(|q| quota_text(q, ru)).unwrap_or_default()),
                ..Default::default()
            });
            if collapsed {
                continue;
            }
            notices.extend(servers);
            for server in notices {
                let (code, name) = server_name(&server.name);
                let (protocol, security, transport) = kind_labels(&server.kind);
                let info = is_info(server);
                rows.push(ServerRow {
                    id: text(&server.id),
                    name: text(name),
                    host: text(format!("{}:{}", server.server, server.port)),
                    protocol: text(protocol),
                    security: text(security),
                    transport: text(transport),
                    flag: self.flag(&code),
                    ping: text(
                        server
                            .last_ping_ms
                            .map(|ms| format!("● {ms} {}", if ru { "мс" } else { "ms" }))
                            .unwrap_or_else(|| "—".into()),
                    ),
                    ping_tone: ping_tone(server.last_ping_ms),
                    favorite: server.favorite,
                    selected: self.settings.selected_server_id.as_deref() == Some(&server.id)
                        && (id == "favorites" || !server.favorite),
                    deletable: id == "manual",
                    info,
                    links: if info {
                        model(notice_links(&server.name))
                    } else {
                        Default::default()
                    },
                    traffic: text(if server.total_down + server.total_up > 0 {
                        format!(
                            "↓ {} ↑ {}",
                            format_bytes(server.total_down as f64, ru, false),
                            format_bytes(server.total_up as f64, ru, false)
                        )
                    } else {
                        String::new()
                    }),
                    ..Default::default()
                });
            }
        }
        data.set_server_count(text(format!(
            "{} {}",
            profiles.all_servers().filter(|s| !is_info(s)).count(),
            if ru { "серверов" } else { "servers" }
        )));
        data.set_servers(model(rows));
        data.set_sort(match self.settings.server_sort.as_str() {
            "ping" => 1,
            "name" => 2,
            _ => 0,
        });
        data.set_subscriptions(model(
            profiles
                .subscriptions
                .iter()
                .map(|sub| SubRow {
                    id: text(&sub.id),
                    name: text(&sub.name),
                    url: text(&sub.url),
                    meta: text(format!(
                        "{} {}   {}",
                        sub.servers.iter().filter(|s| !is_info(s)).count(),
                        if ru { "серверов" } else { "servers" },
                        relative_time(sub.updated_at.as_deref(), ru)
                    )),
                    quota: text(sub.quota.map(|q| quota_text(q, ru)).unwrap_or_default()),
                    ratio: sub
                        .quota
                        .filter(|q| q.total > 0)
                        .map(|q| ((q.upload + q.download) as f64 / q.total as f64).min(1.0) as f32)
                        .unwrap_or_default(),
                    metered: sub.quota.is_some_and(|q| q.total > 0),
                    hours: match sub.auto_update_hours {
                        0 => 0,
                        1 => 1,
                        6 => 2,
                        12 => 3,
                        24 => 4,
                        _ => 5,
                    },
                    account: sub.web_page_url.is_some(),
                    support: sub.support_url.is_some(),
                })
                .collect(),
        ));
        let filter = data.get_route_filter();
        let mut counts = vec![self.settings.routing_rules.len() as i32, 0, 0, 0];
        for rule in &self.settings.routing_rules {
            counts[match rule.rule_type {
                RuleType::Process => 1,
                RuleType::Domain => 2,
                RuleType::IpCidr => 3,
            }] += 1;
        }
        data.set_route_counts(model(counts));
        data.set_routes(model(
            self.settings
                .routing_rules
                .iter()
                .enumerate()
                .filter(|(_, r)| {
                    (filter == 0
                        || filter
                            == match r.rule_type {
                                RuleType::Process => 1,
                                RuleType::Domain => 2,
                                RuleType::IpCidr => 3,
                            })
                        && (query.is_empty()
                            || r.value.to_lowercase().contains(&query)
                            || r.description
                                .as_deref()
                                .unwrap_or("")
                                .to_lowercase()
                                .contains(&query))
                })
                .map(|(i, r)| RouteRow {
                    rule_type: match r.rule_type {
                        RuleType::Process => 0,
                        RuleType::Domain => 1,
                        RuleType::IpCidr => 2,
                    },
                    id: text(&r.id),
                    value: text(
                        if r.rule_type == RuleType::Domain
                            && r.domain_matcher == Some(DomainMatcher::Suffix)
                        {
                            format!("*.{}", r.value.trim_start_matches("*."))
                        } else {
                            r.value.clone()
                        },
                    ),
                    kind: rule_kind(r, ru),
                    description: r.description.as_ref().map(text).unwrap_or_else(|| {
                        match r.action {
                            AppRouteAction::Proxy => tr(
                                ru,
                                "Трафик через выбранный сервер",
                                "Traffic through selected server",
                            ),
                            AppRouteAction::Direct => {
                                tr(ru, "В обход туннеля", "Bypass the tunnel")
                            }
                            AppRouteAction::Block => tr(
                                ru,
                                "Сетевые подключения запрещены",
                                "Network connections blocked",
                            ),
                        }
                    }),
                    action: match r.action {
                        AppRouteAction::Proxy => 0,
                        AppRouteAction::Direct => 1,
                        AppRouteAction::Block => 2,
                    },
                    enabled: r.enabled,
                    index: i as i32,
                })
                .collect(),
        ));
    }

    fn modal(&self, ui: &MainWindow, kind: i32, id: &str, title: SharedString) {
        let data = ui.global::<App>();
        if kind == 5 {
            data.set_rule_title(title.clone());
        }
        data.set_modal_id(text(id));
        data.set_modal_title(title);
        data.set_modal(kind);
    }
    fn patch(&mut self, ui: &MainWindow, key: &str, value: Value) {
        self.submit(ui, Task::Save(json!({key:value})), false);
    }
    fn save_rules(&mut self, ui: &MainWindow, rules: Vec<RouteRule>) {
        self.submit(
            ui,
            Task::Save(json!({"routingRules":rules,"appRoutes":[]})),
            false,
        );
    }
    fn action(&mut self, ui: &MainWindow, action: &str, id: &str, value: &str) -> AppResult<()> {
        let data = ui.global::<App>();
        let ru = self.settings.language == "ru";
        match action {
            "page" => {
                data.set_query("".into());
                self.models_dirty = true;
                self.logs_revision = u64::MAX;
                self.refresh(ui);
            }
            "filter" => {
                self.models_dirty = true;
                self.refresh(ui);
            }
            "toggle" => {
                if self.settings.selected_server_id.is_none()
                    && self.conn.status == ConnStatus::Disconnected
                {
                    data.set_page(1);
                } else {
                    self.submit(ui, Task::Toggle, false);
                }
            }
            "reconnect" => self.submit(ui, Task::Reconnect, false),
            "mode" => self.submit(
                ui,
                Task::Mode(if value == "tun" {
                    Mode::Tun
                } else {
                    Mode::SystemProxy
                }),
                false,
            ),
            "select" => self.submit(ui, Task::Select(id.into()), false),
            "ping" => {
                let ids = if id == "all" {
                    self.app
                        .state()
                        .profiles
                        .try_read()
                        .map(|p| {
                            p.all_servers()
                                .filter(|s| !is_info(s))
                                .map(|s| s.id.clone())
                                .collect()
                        })
                        .unwrap_or_default()
                } else {
                    vec![id.into()]
                };
                self.submit(ui, Task::Ping(ids), false);
            }
            "favorite" => self.submit(ui, Task::Favorite(id.into(), value == "true"), false),
            "favorite-toggle" => {
                let favorite = self
                    .app
                    .state()
                    .profiles
                    .try_read()
                    .ok()
                    .and_then(|p| p.find_server(id).map(|s| !s.favorite));
                if let Some(favorite) = favorite {
                    self.submit(ui, Task::Favorite(id.into(), favorite), false);
                }
            }
            "copy-server" => {
                let raw = self
                    .app
                    .state()
                    .profiles
                    .try_read()
                    .ok()
                    .and_then(|p| p.find_server(id).map(|s| s.raw.clone()));
                if let Some(raw) = raw {
                    clipboard_write(&raw)?;
                }
            }
            "collapse" => {
                let mut groups = self.settings.collapsed_groups.clone();
                if groups.iter().any(|key| key == id) {
                    groups.retain(|key| key != id);
                } else {
                    groups.push(id.into());
                }
                self.patch(ui, "collapsedGroups", json!(groups));
            }
            "sub-move" => {
                let ids = self.app.state().profiles.try_read().ok().map(|p| {
                    p.subscriptions
                        .iter()
                        .map(|s| s.id.clone())
                        .collect::<Vec<_>>()
                });
                if let Some(mut ids) = ids {
                    if let Some(i) = ids.iter().position(|s| s == id) {
                        let target = i as i32 + value.parse::<i32>().unwrap_or(0);
                        if target >= 0 && (target as usize) < ids.len() {
                            ids.swap(i, target as usize);
                            self.submit(ui, Task::Reorder(ids), false);
                        }
                    }
                }
            }
            "sort" => self.patch(ui, "serverSort", json!(value)),
            "server-menu" => {
                let deletable = self
                    .app
                    .state()
                    .profiles
                    .try_read()
                    .is_ok_and(|p| p.manual.iter().any(|s| s.id == id));
                data.set_deletable(deletable);
                self.modal(ui, 8, id, tr(ru, "Сервер", "Server"));
            }
            "import-dialog" => {
                data.set_field0("".into());
                self.modal(ui, 1, "", tr(ru, "Добавить серверы", "Add servers"));
            }
            "paste-import" => {
                let value = arboard::Clipboard::new()
                    .and_then(|mut c| c.get_text())
                    .map_err(|e| AppError::Internal(e.to_string()))?;
                data.set_field0(text(value));
            }
            "sub-dialog" => {
                data.set_field0("".into());
                data.set_field1("".into());
                self.modal(ui, 2, "", tr(ru, "Добавить подписку", "Add subscription"));
            }
            "rename-dialog" => {
                if let Ok(profiles) = self.app.state().profiles.try_read() {
                    if let Some(sub) = profiles.subscriptions.iter().find(|s| s.id == id) {
                        data.set_field0(text(&sub.name));
                    }
                }
                self.modal(
                    ui,
                    3,
                    id,
                    tr(ru, "Переименовать подписку", "Rename subscription"),
                );
            }
            "delete-sub-dialog" | "delete-server-dialog" => {
                data.set_field2(text(if action == "delete-sub-dialog" {
                    "subscription"
                } else {
                    "server"
                }));
                self.modal(
                    ui,
                    4,
                    id,
                    tr(ru, "Подтвердите удаление", "Confirm deletion"),
                );
            }
            "update-sub" => self.submit(ui, Task::UpdateSub(id.into()), false),
            "auto-update" => self.submit(
                ui,
                Task::AutoUpdate(
                    id.into(),
                    value
                        .parse()
                        .map_err(|_| AppError::Parse("invalid update interval".into()))?,
                ),
                false,
            ),
            "account" | "support" => {
                let url = self.app.state().profiles.try_read().ok().and_then(|p| {
                    p.subscriptions.iter().find(|s| s.id == id).and_then(|s| {
                        if action == "account" {
                            s.web_page_url.clone()
                        } else {
                            s.support_url.clone()
                        }
                    })
                });
                if let Some(url) = url {
                    open_url(&url)?;
                }
            }
            "route-default" => self.patch(ui, "routeDefault", json!(value)),
            "rule-dialog" => {
                data.set_field0("".into());
                data.set_field1("".into());
                data.set_edit_type(0);
                data.set_edit_matcher(0);
                data.set_edit_action(0);
                data.set_edit_enabled(true);
                self.modal(ui, 5, "", tr(ru, "Добавить правило", "Add rule"));
            }
            "edit-rule-dialog" => {
                if let Some(rule) = self.settings.routing_rules.iter().find(|r| r.id == id) {
                    data.set_field0(text(&rule.value));
                    data.set_field1(text(rule.description.as_deref().unwrap_or("")));
                    data.set_edit_type(match rule.rule_type {
                        RuleType::Process => 0,
                        RuleType::Domain => 1,
                        RuleType::IpCidr => 2,
                    });
                    data.set_edit_matcher(match rule.rule_type {
                        RuleType::Process => {
                            if rule.process_matcher == Some(ProcessMatcher::Path) {
                                1
                            } else {
                                0
                            }
                        }
                        RuleType::Domain => {
                            match rule.domain_matcher.unwrap_or(DomainMatcher::Suffix) {
                                DomainMatcher::Suffix => 0,
                                DomainMatcher::Exact => 1,
                                DomainMatcher::Keyword => 2,
                                DomainMatcher::Regex => 3,
                            }
                        }
                        RuleType::IpCidr => 0,
                    });
                    data.set_edit_action(match rule.action {
                        AppRouteAction::Proxy => 0,
                        AppRouteAction::Direct => 1,
                        AppRouteAction::Block => 2,
                    });
                    data.set_edit_enabled(rule.enabled);
                    self.modal(ui, 5, id, tr(ru, "Изменить правило", "Edit rule"));
                }
            }
            "rule-toggle" | "rule-action" | "rule-move" | "delete-rule" => {
                let mut rules = self.settings.routing_rules.clone();
                if let Some(index) = rules.iter().position(|r| r.id == id) {
                    match action {
                        "rule-toggle" => rules[index].enabled = value == "true",
                        "rule-action" => {
                            rules[index].action = match value {
                                "direct" => AppRouteAction::Direct,
                                "block" => AppRouteAction::Block,
                                _ => AppRouteAction::Proxy,
                            }
                        }
                        "delete-rule" => {
                            rules.remove(index);
                        }
                        "rule-move" => {
                            let target = index as i32 + value.parse::<i32>().unwrap_or(0);
                            if target >= 0 && (target as usize) < rules.len() {
                                rules.swap(index, target as usize);
                            }
                        }
                        _ => (),
                    }
                    self.save_rules(ui, rules);
                }
            }
            "processes" => self.submit(ui, Task::Processes, false),
            "process-filter" => self.filter_processes(ui),
            "notice-link" => {
                if let Some(link) = value.split_whitespace().find(|v| {
                    v.starts_with("https://") || v.starts_with("http://") || v.starts_with("t.me/")
                }) {
                    let url = if link.starts_with("t.me/") {
                        format!("https://{link}")
                    } else {
                        link.into()
                    };
                    open_url(&url)?;
                }
            }
            "open-url" => open_url(value)?,
            "setting" => {
                let current = serde_json::to_value(&self.settings)?;
                let value = match current.get(id) {
                    Some(Value::Bool(_)) => json!(value == "true"),
                    Some(Value::Number(_)) => {
                        let parsed = value
                            .parse::<u32>()
                            .map_err(|_| AppError::Parse("expected a number".into()))?;
                        if (id == "mixedPort" && !(1..=65535).contains(&parsed))
                            || (id == "tunMtu" && !(576..=65535).contains(&parsed))
                        {
                            return Err(AppError::Parse(
                                "value is outside the allowed range".into(),
                            ));
                        }
                        json!(parsed)
                    }
                    _ => json!(value),
                };
                if current.get(id) != Some(&value) {
                    self.patch(ui, id, value);
                }
            }
            "setting-choice" => {
                let index = value.parse::<usize>().unwrap_or(0);
                let values: Vec<&str> = match id {
                    "language" => vec!["ru", "en"],
                    "accent" => vec!["violet", "cyan", "emerald", "amber"],
                    "ipStrategy" => vec!["ipv4_only", "prefer_ipv4", "prefer_ipv6", "ipv6_only"],
                    "logLevel" => vec!["trace", "debug", "info", "warn", "error", "fatal"],
                    "tunStack" => vec!["mixed", "system", "gvisor"],
                    _ => vec![],
                };
                if let Some(value) = values.get(index) {
                    self.patch(ui, id, json!(value));
                }
            }
            "settings-action" => match id {
                "data" => self.submit(ui, Task::OpenData, false),
                "update" => self.submit(ui, Task::CheckUpdate, false),
                "download" => self.submit(
                    ui,
                    Task::Download(self.core_update.as_ref().map(|v| v.latest.clone())),
                    false,
                ),
                "hwid" => clipboard_write(&self.settings.hwid)?,
                "slint" => self.modal(ui, 10, "", tr(ru, "О программе", "About")),
                "app-update" => open_url("https://github.com/wannasly/Umbra/releases/latest")?,
                "test" => self.submit(ui, Task::UrlTest, false),
                "quit" => {
                    let _ = slint::quit_event_loop();
                }
                _ => (),
            },
            "logs" => {
                self.logs_revision = u64::MAX;
                self.update_logs(ui);
            }
            "log-level" => {
                self.log_level = if value == "Все" || value == "All" {
                    String::new()
                } else {
                    value.to_lowercase()
                };
                data.set_log_level(text(&self.log_level));
                self.logs_revision = u64::MAX;
                self.update_logs(ui);
            }
            "copy-logs" => clipboard_write(data.get_log_text().as_str())?,
            "clear-logs" => {
                self.app.state().logs.clear();
                self.frozen_logs = data.get_log_paused().then(Vec::new);
                self.logs_revision = u64::MAX;
                self.update_logs(ui);
            }
            "window-close" => self.close(ui),
            "window-minimize" => ui.window().set_minimized(true),
            "window-maximize" => ui.window().set_maximized(!ui.window().is_maximized()),
            "window-drag" => {
                use slint::winit_030::WinitWindowAccessor;
                ui.window().with_winit_window(|window| {
                    let _ = window.drag_window();
                });
            }
            "modal-submit" => {
                let id = data.get_modal_id().to_string();
                match data.get_modal() {
                    1 => self.submit(ui, Task::Import(data.get_field0().to_string()), true),
                    2 => self.submit(
                        ui,
                        Task::AddSub(
                            data.get_field1().trim().into(),
                            data.get_field0().to_string(),
                        ),
                        true,
                    ),
                    3 => self.submit(ui, Task::RenameSub(id, data.get_field0().to_string()), true),
                    4 => self.submit(
                        ui,
                        if data.get_field2() == "subscription" {
                            Task::RemoveSub(id)
                        } else {
                            Task::RemoveServer(id)
                        },
                        true,
                    ),
                    5 => {
                        let kind = data.get_edit_type();
                        let matcher = data.get_edit_matcher();
                        let value =
                            normalize_rule_value(data.get_field0().as_str(), kind, matcher)?;
                        let rule = RouteRule {
                            id: if id.is_empty() {
                                uuid::Uuid::new_v4().to_string()
                            } else {
                                id
                            },
                            enabled: data.get_edit_enabled(),
                            rule_type: match kind {
                                1 => RuleType::Domain,
                                2 => RuleType::IpCidr,
                                _ => RuleType::Process,
                            },
                            value,
                            process_matcher: if kind == 0 {
                                Some(if matcher == 1 {
                                    ProcessMatcher::Path
                                } else {
                                    ProcessMatcher::Name
                                })
                            } else {
                                None
                            },
                            domain_matcher: if kind == 1 {
                                Some(match matcher {
                                    1 => DomainMatcher::Exact,
                                    2 => DomainMatcher::Keyword,
                                    3 => DomainMatcher::Regex,
                                    _ => DomainMatcher::Suffix,
                                })
                            } else {
                                None
                            },
                            action: match data.get_edit_action() {
                                1 => AppRouteAction::Direct,
                                2 => AppRouteAction::Block,
                                _ => AppRouteAction::Proxy,
                            },
                            description: (!data.get_field1().trim().is_empty())
                                .then(|| data.get_field1().trim().to_string()),
                        };
                        let mut rules = self.settings.routing_rules.clone();
                        if let Some(existing) = rules.iter_mut().find(|r| r.id == rule.id) {
                            *existing = rule;
                        } else {
                            rules.push(rule);
                        }
                        self.submit(
                            ui,
                            Task::Save(json!({"routingRules":rules,"appRoutes":[]})),
                            true,
                        );
                    }
                    6 => self.submit(ui, Task::Elevate, true),
                    _ => (),
                }
            }
            _ => (),
        }
        Ok(())
    }
}

fn clipboard_write(value: &str) -> AppResult<()> {
    arboard::Clipboard::new()
        .and_then(|mut c| c.set_text(value))
        .map_err(|e| AppError::Internal(e.to_string()))
}

#[cfg(windows)]
fn browse_executable() -> AppResult<Option<String>> {
    use windows::{
        core::{w, PWSTR},
        Win32::UI::Controls::Dialogs::*,
    };
    let mut path = vec![0u16; 32768];
    let mut dialog = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        lpstrFilter: w!("Applications (*.exe)\0*.exe\0\0"),
        lpstrFile: PWSTR(path.as_mut_ptr()),
        nMaxFile: path.len() as u32,
        Flags: OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR,
        ..Default::default()
    };
    unsafe {
        if GetOpenFileNameW(&mut dialog).as_bool() {
            let length = path.iter().position(|c| *c == 0).unwrap_or(path.len());
            return Ok(Some(String::from_utf16_lossy(&path[..length])));
        }
        let code = CommDlgExtendedError();
        if code.0 != 0 {
            return Err(AppError::Internal(format!("File dialog error: {}", code.0)));
        }
    }
    Ok(None)
}

fn normalize_rule_value(input: &str, kind: i32, matcher: i32) -> AppResult<String> {
    let value = input.trim();
    let invalid = || AppError::Parse("Некорректное значение правила / Invalid rule value".into());
    if value.is_empty() {
        return Err(invalid());
    }
    match kind {
        0 => {
            let value = value.trim_matches(['\'', '"']);
            if matcher == 1 {
                if !value.contains(['/', '\\']) || !value.to_lowercase().ends_with(".exe") {
                    return Err(invalid());
                }
                Ok(value.into())
            } else {
                let name = value.rsplit(['/', '\\']).next().unwrap_or("").trim();
                if name.is_empty() {
                    return Err(invalid());
                }
                Ok(if name.to_lowercase().ends_with(".exe") {
                    name.into()
                } else {
                    format!("{name}.exe")
                })
            }
        }
        1 if matcher < 2 => {
            let lower = value.to_lowercase();
            let host = lower
                .split_once("://")
                .map_or(lower.as_str(), |(_, host)| host);
            let host = host
                .split('/')
                .next()
                .unwrap_or("")
                .split(':')
                .next()
                .unwrap_or("");
            let host = host
                .strip_prefix("*.")
                .or_else(|| host.strip_prefix('.'))
                .unwrap_or(host)
                .trim();
            if host.is_empty() {
                Err(invalid())
            } else {
                Ok(host.into())
            }
        }
        2 => {
            let (address, prefix) = value
                .split_once('/')
                .map_or((value, None), |(a, p)| (a, Some(p)));
            let address = address.parse::<std::net::IpAddr>().map_err(|_| invalid())?;
            if let Some(prefix) = prefix {
                let prefix = prefix.parse::<u8>().map_err(|_| invalid())?;
                if prefix > if address.is_ipv4() { 32 } else { 128 } {
                    return Err(invalid());
                }
            }
            Ok(value.into())
        }
        _ => Ok(value.into()),
    }
}
fn notice_links(name: &str) -> Vec<SharedString> {
    static LINKS: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    LINKS
        .get_or_init(|| regex::Regex::new(r"https?://\S+|(?:^|\s)t\.me/\S+").unwrap())
        .find_iter(name)
        .filter_map(|link| {
            let link = link.as_str().trim();
            let href = if link.starts_with("t.me/") {
                format!("https://{link}")
            } else {
                link.into()
            };
            url::Url::parse(&href).ok().map(|url| text(url.as_str()))
        })
        .collect()
}

fn open_url(value: &str) -> AppResult<()> {
    let url = url::Url::parse(value).map_err(|e| AppError::Parse(e.to_string()))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(AppError::Unsupported(
            "only HTTP(S) links can be opened".into(),
        ));
    }
    #[cfg(windows)]
    unsafe {
        use windows::{
            core::HSTRING,
            Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
        };
        let result = ShellExecuteW(
            None,
            windows::core::w!("open"),
            &HSTRING::from(value),
            None,
            None,
            SW_SHOWNORMAL,
        );
        if result.0 as isize <= 32 {
            return Err(AppError::Internal("failed to open link".into()));
        }
    }
    Ok(())
}

fn rule_kind(rule: &RouteRule, ru: bool) -> SharedString {
    match rule.rule_type {
        RuleType::IpCidr => "IP / CIDR".into(),
        RuleType::Process => {
            if rule.process_matcher == Some(ProcessMatcher::Path) {
                tr(ru, "Путь процесса", "Process path")
            } else {
                tr(ru, "Имя процесса", "Process name")
            }
        }
        RuleType::Domain => match rule.domain_matcher.unwrap_or(DomainMatcher::Suffix) {
            DomainMatcher::Suffix => tr(ru, "Суффикс домена", "Domain suffix"),
            DomainMatcher::Exact => tr(ru, "Домен", "Exact domain"),
            DomainMatcher::Keyword => tr(ru, "Ключевое слово", "Keyword"),
            DomainMatcher::Regex => "Regex".into(),
        },
    }
}
fn is_info(server: &ServerEntry) -> bool {
    static MESSAGES: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    matches!(
        server.server.trim().to_lowercase().as_str(),
        "0.0.0.0" | "::" | "::0" | "127.0.0.1" | "::1" | "localhost"
    ) || server.port == 1
        || MESSAGES
            .get_or_init(|| {
                regex::Regex::new(concat!(
            r"(?i)https?://|\bt\.me/|(^|[\s(\[])@[a-z0-9_]{4,}|",
            r"остал[оа]сь|истека|истёк|истек\b|\bdays?\s+(left|remaining)\b|\bexpir(es|ed|ing)\b|",
            r"подписк|продлит|продлен|тариф|трафик|\bsubscription\b|\brenew\b|",
            r"не работает|нажим|нажат|обновит|обновлени|включит|поддержк"
        )).expect("original info entry patterns")
            })
            .is_match(&server.name)
}
fn relative_time(value: Option<&str>, ru: bool) -> String {
    value
        .and_then(|v| chrono::DateTime::parse_from_rfc3339(v).ok())
        .map(|v| {
            let hours = (chrono::Utc::now() - v.with_timezone(&chrono::Utc))
                .num_hours()
                .max(0);
            if hours < 1 {
                if ru {
                    "Обновлено только что".into()
                } else {
                    "Updated just now".into()
                }
            } else if hours < 24 {
                if ru {
                    format!("Обновлено {hours} ч назад")
                } else {
                    format!("Updated {hours} hours ago")
                }
            } else if ru {
                format!("Обновлено {} дн назад", hours / 24)
            } else {
                format!("Updated {} days ago", hours / 24)
            }
        })
        .unwrap_or_default()
}
fn quota_text(q: SubscriptionQuota, ru: bool) -> String {
    let mut result = format!(
        "{} {} {} {}",
        if ru {
            "Использовано"
        } else {
            "Used"
        },
        format_bytes(q.upload.saturating_add(q.download) as f64, ru, false),
        if ru { "из" } else { "of" },
        if q.total == 0 {
            "∞".into()
        } else {
            format_bytes(q.total as f64, ru, false)
        }
    );
    if q.expire > 0 {
        if let Some(date) = chrono::DateTime::from_timestamp(q.expire as i64, 0) {
            let days = ((date - chrono::Utc::now()).num_milliseconds() as f64 / 86_400_000.0).ceil()
                as i64;
            result += &format!(
                "   {} {}   {}",
                if ru { "Истекает" } else { "Expires" },
                date.with_timezone(&chrono::Local).format("%d.%m.%Y"),
                if days <= 0 {
                    if ru {
                        "Истекла".into()
                    } else {
                        "Expired".into()
                    }
                } else if ru {
                    format!("осталось {days} дней")
                } else {
                    format!("{days} days left")
                }
            );
        }
    }
    result
}

// The aliases are ported from the original flags.ts; raw profile names stay intact.
fn server_name(name: &str) -> (String, String) {
    let name = name.trim();
    let mut chars = name.chars();
    let first = chars.next();
    let second = chars.next();
    if let (Some(a), Some(b)) = (first, second) {
        if ('\u{1f1e6}'..='\u{1f1ff}').contains(&a) && ('\u{1f1e6}'..='\u{1f1ff}').contains(&b) {
            let code = [
                char::from_u32(a as u32 - 0x1f1e6 + 65).unwrap(),
                char::from_u32(b as u32 - 0x1f1e6 + 65).unwrap(),
            ]
            .iter()
            .collect();
            return (code, chars.as_str().trim().into());
        }
    }
    static ALIASES: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    let aliases = ALIASES.get_or_init(|| {
        serde_json::from_str(include_str!("../ui/flag-aliases.json"))
            .expect("bundled country aliases")
    });
    let cleaned = format!(
        " {} ",
        name.to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    );
    for (code, values) in aliases["aliases"].as_object().unwrap() {
        for alias in values.as_array().unwrap().iter().filter_map(Value::as_str) {
            if if alias.chars().count() >= 5 {
                cleaned.contains(alias.trim())
            } else {
                cleaned.contains(&format!(" {} ", alias.trim()))
            } {
                return (code.clone(), name.into());
            }
        }
    }
    for code in aliases["bare"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
    {
        if cleaned.contains(&format!(" {} ", code.to_lowercase())) {
            return (
                if code == "UK" {
                    "GB".into()
                } else {
                    code.into()
                },
                name.into(),
            );
        }
    }
    (String::new(), name.into())
}

fn i18n(ru: bool, key: &str) -> SharedString {
    static RU: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    static EN: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    let root = if ru {
        RU.get_or_init(|| {
            serde_json::from_str(include_str!("../ui/i18n/ru.json"))
                .expect("bundled Russian locale")
        })
    } else {
        EN.get_or_init(|| {
            serde_json::from_str(include_str!("../ui/i18n/en.json"))
                .expect("bundled English locale")
        })
    };
    key.split('.')
        .try_fold(root, |value, key| value.get(key))
        .and_then(Value::as_str)
        .unwrap_or(key)
        .into()
}

fn settings_rows(
    s: &Settings,
    core: Option<&CoreStatus>,
    update: Option<&UpdateCheck>,
) -> Vec<SettingRow> {
    let ru = s.language == "ru";
    let mut rows = Vec::new();
    let label = |key: &str| i18n(ru, &format!("settings.{key}"));
    let section = |key: &str| SettingRow {
        label: label(key),
        kind: 0,
        ..Default::default()
    };
    let toggle = |key: &str, label_key: &str, checked: bool| SettingRow {
        key: text(key),
        label: label(label_key),
        checked,
        kind: 1,
        ..Default::default()
    };
    let field = |key: &str, label_key: &str, value: String| SettingRow {
        key: text(key),
        label: label(label_key),
        value: text(value),
        kind: 3,
        ..Default::default()
    };
    let choice =
        |key: &str, label_key: &str, options: Vec<SharedString>, selected: usize| SettingRow {
            key: text(key),
            label: label(label_key),
            options: model(options),
            selected: selected as i32,
            kind: 2,
            ..Default::default()
        };
    let button = |key: &str, label_key: &str, value: SharedString| SettingRow {
        key: text(key),
        label: label(label_key),
        value,
        kind: 4,
        ..Default::default()
    };
    rows.push(section("general.title"));
    rows.push(choice(
        "language",
        "general.language",
        vec!["Русский".into(), "English".into()],
        if ru { 0 } else { 1 },
    ));
    rows.push(toggle("autostart", "general.autostart", s.autostart));
    rows.push(toggle(
        "startMinimized",
        "general.startMinimized",
        s.start_minimized,
    ));
    rows.push(toggle(
        "minimizeToTray",
        "general.closeToTray",
        s.minimize_to_tray,
    ));
    rows.push(toggle(
        "connectOnStartup",
        "general.connectOnStartup",
        s.connect_on_startup,
    ));
    rows.push(section("appearance.title"));
    rows.push(SettingRow {
        key: "accent".into(),
        label: label("appearance.accent"),
        kind: 5,
        selected: match s.accent.as_str() {
            "cyan" => 1,
            "emerald" => 2,
            "amber" => 3,
            _ => 0,
        },
        ..Default::default()
    });
    rows.push(toggle(
        "reduceMotion",
        "appearance.reduceMotion",
        s.reduce_motion,
    ));
    rows.push(section("connection.title"));
    rows.push(toggle("bypassRu", "connection.bypassRu", s.bypass_ru));
    rows.push(choice(
        "ipStrategy",
        "connection.ipStrategy",
        ["ipv4_only", "prefer_ipv4", "prefer_ipv6", "ipv6_only"]
            .iter()
            .map(|v| label(&format!("connection.ipStrategies.{v}")))
            .collect(),
        match s.ip_strategy {
            IpStrategy::Ipv4Only => 0,
            IpStrategy::PreferIpv4 => 1,
            IpStrategy::PreferIpv6 => 2,
            IpStrategy::Ipv6Only => 3,
        },
    ));
    rows.push(choice(
        "tunStack",
        "connection.tunStack",
        ["mixed", "system", "gvisor"]
            .into_iter()
            .map(text)
            .collect(),
        match s.tun_stack.as_str() {
            "system" => 1,
            "gvisor" => 2,
            _ => 0,
        },
    ));
    rows.push(toggle(
        "tunStrictRoute",
        "connection.strictRoute",
        s.tun_strict_route,
    ));
    rows.push(field("tunMtu", "connection.mtu", s.tun_mtu.to_string()));
    let mut voice = toggle(
        "discordVoiceDirect",
        "connection.discordVoiceDirect",
        s.discord_voice_direct,
    );
    voice.description = label("connection.discordVoiceDirectHint");
    rows.push(voice);
    rows.push(section("subscriptions.title"));
    let mut hwid = toggle("sendHwid", "subscriptions.sendHwid", s.send_hwid);
    hwid.description = label("subscriptions.sendHwidHint");
    rows.push(hwid);
    if s.send_hwid {
        rows.push(button(
            "hwid",
            "subscriptions.hwid",
            label("subscriptions.copyHwid"),
        ));
    }
    let mut agent = field(
        "subUserAgent",
        "subscriptions.userAgent",
        s.sub_user_agent.clone(),
    );
    agent.description = label("subscriptions.userAgentNote");
    rows.push(agent);
    rows.push(section("core.title"));
    rows.push(SettingRow {
        label: label("core.installed"),
        kind: 6,
        value: core
            .and_then(|c| c.version.as_deref())
            .map(text)
            .unwrap_or_else(|| label("core.notInstalled")),
        ..Default::default()
    });
    let latest = update.map(|u| u.latest.as_str()).unwrap_or("—");
    rows.push(button(
        "update",
        "core.latest",
        text(format!("{latest} · {}", label("core.check"))),
    ));
    if core.is_some_and(|c| !c.installed) || update.is_some_and(|u| u.update_available) {
        rows.push(button("download", "core.download", label("core.download")));
    }
    rows.push(button("data", "core.openFolder", label("core.openFolder")));
    rows.push(field(
        "githubMirror",
        "core.mirror",
        s.github_mirror.clone(),
    ));
    rows.push(section("about.title"));
    rows.push(button(
        "app-update",
        "about.description",
        label("about.updateApp"),
    ));
    rows.push(button(
        "slint",
        "about.title",
        text(format!("Umbra {} · Slint", env!("CARGO_PKG_VERSION"))),
    ));
    for i in 0..rows.len().saturating_sub(1) {
        if rows[i + 1].kind == 0 {
            rows[i].last = true;
        }
    }
    if let Some(last) = rows.last_mut() {
        last.last = true;
    }
    rows
}

pub fn run(
    app: AppHandle,
    settings: Settings,
    minimized: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let renderer = crate::graphics::renderer();
    eprintln!("[umbra] renderer: {renderer}");
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name(renderer.into())
        .select()?;
    let ui = MainWindow::new()?;
    ui.global::<Theme>()
        .set_software_rendering(renderer == "software");
    // Controlled resource measurements use identical logical geometry. This
    // override is accepted only with an explicitly isolated data directory.
    if std::env::var_os("UMBRA_DATA_DIR").is_some() {
        if let Ok(size) = std::env::var("UMBRA_BENCH_SIZE") {
            let dimensions: Vec<f32> = size.split('x').filter_map(|s| s.parse().ok()).collect();
            if dimensions.len() == 2
                && dimensions
                    .iter()
                    .all(|v| v.is_finite() && *v >= 1.0 && *v <= 16384.0)
            {
                ui.window()
                    .set_size(slint::LogicalSize::new(dimensions[0], dimensions[1]));
            }
        }
    }
    let tray = match NativeTray::new(&app) {
        Ok(tray) => Some(tray),
        Err(error) => {
            eprintln!("Tray unavailable: {error}");
            None
        }
    };
    UI.with(|state| *state.borrow_mut() = Some(UiState::new(app.clone(), settings, tray)));
    let weak = ui.as_weak();
    ui.global::<App>().on_action(move |action, id, value| {
        let weak = weak.clone();
        let action = action.to_string();
        let id = id.to_string();
        let value = value.to_string();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = weak.upgrade() {
                // Native file dialogs pump a nested Windows event loop. Release
                // UiState before they can dispatch another backend wakeup.
                if action == "browse-executable" {
                    #[cfg(windows)]
                    match browse_executable() {
                        Ok(Some(path)) => {
                            ui.global::<App>().set_field0(text(path));
                            ui.global::<App>().set_edit_matcher(1);
                        }
                        Ok(None) => (),
                        Err(error) => UI.with(|state| {
                            if let Some(state) = state.borrow().as_ref() {
                                state.message(&ui, error, true);
                            }
                        }),
                    }
                    return;
                }
                UI.with(|state| {
                    if let Some(state) = state.borrow_mut().as_mut() {
                        if let Err(error) = state.action(&ui, &action, &id, &value) {
                            state.message(&ui, error, true);
                        }
                    }
                });
            }
        });
    });
    let pending = Arc::new(AtomicBool::new(false));
    let weak = ui.as_weak();
    app.set_waker(Arc::new(move || {
        if pending.swap(true, Ordering::AcqRel) {
            return;
        }
        let pending = pending.clone();
        let _ = weak.upgrade_in_event_loop(move |ui| {
            pending.store(false, Ordering::Release);
            UI.with(|state| {
                if let Some(state) = state.borrow_mut().as_mut() {
                    state.refresh(&ui);
                }
            });
        });
    }));
    let weak = ui.as_weak();
    ui.window().on_close_requested(move || {
        if let Some(ui) = weak.upgrade() {
            UI.with(|state| {
                if let Some(state) = state.borrow().as_ref() {
                    state.close(&ui);
                }
            });
        }
        slint::CloseRequestResponse::KeepWindowShown
    });
    ui.show()?;
    UI.with(|state| {
        if let Some(state) = state.borrow_mut().as_mut() {
            state.refresh(&ui);
            state.submit(&ui, Task::CoreStatus, false);
        }
    });
    if minimized {
        let weak = ui.as_weak();
        slint::Timer::single_shot(Duration::ZERO, move || {
            if let Some(ui) = weak.upgrade() {
                UI.with(|state| {
                    if state.borrow().as_ref().is_some_and(|s| s.tray.is_some()) {
                        let _ = ui.hide();
                    }
                });
            }
        });
    }
    let timer = slint::Timer::default();
    let weak = ui.as_weak();
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_secs(1),
        move || {
            if let Some(ui) = weak.upgrade() {
                UI.with(|state| {
                    if let Some(state) = state.borrow_mut().as_mut() {
                        state.tick(&ui);
                    }
                });
            }
        },
    );
    #[cfg(feature = "smoke-test")]
    let _capture = setup_capture(&ui);
    slint::run_event_loop_until_quit()?;
    UI.with(|state| {
        state.borrow_mut().take();
    });
    Ok(())
}

#[cfg(feature = "smoke-test")]
fn setup_capture(ui: &MainWindow) -> Option<slint::Timer> {
    if let Ok(size) = std::env::var("UMBRA_BENCH_SIZE") {
        assert!(
            std::env::var_os("UMBRA_DATA_DIR").is_some(),
            "benchmark requires isolated data"
        );
        let dimensions: Vec<f32> = size.split('x').filter_map(|s| s.parse().ok()).collect();
        if dimensions.len() == 2 {
            ui.window()
                .set_size(slint::LogicalSize::new(dimensions[0], dimensions[1]));
        }
        let timer = slint::Timer::default();
        timer.start(
            slint::TimerMode::SingleShot,
            Duration::from_secs(90),
            || {
                let _ = slint::quit_event_loop();
            },
        );
        return Some(timer);
    }
    let path = std::env::var_os("UMBRA_SMOKE_CAPTURE")?;
    // Read GPU pixels before buffer presentation. An idle back buffer after a
    // hide/show cycle can be empty even though the visible frame is complete.
    let rendered_frame = Rc::new(RefCell::new(None));
    let frame = rendered_frame.clone();
    let weak = ui.as_weak();
    let _ = ui.window().set_rendering_notifier(move |state, _| {
        if matches!(state, slint::RenderingState::AfterRendering) {
            if let Some(ui) = weak.upgrade() {
                *frame.borrow_mut() = ui.window().take_snapshot().ok();
            }
        }
    });
    let page = match std::env::var("UMBRA_SMOKE_PAGE")
        .unwrap_or_default()
        .as_str()
    {
        "servers" => 1,
        "subscriptions" => 2,
        "routing" => 3,
        "logs" => 4,
        "settings" => 5,
        _ => 0,
    };
    ui.global::<App>().set_page(page);
    if let Ok(action) = std::env::var("UMBRA_SMOKE_ACTION") {
        UI.with(|state| {
            if let Some(state) = state.borrow_mut().as_mut() {
                let (action, id, value) = match action.as_str() {
                    "import" => ("import-dialog", "", ""),
                    "subscription" => ("sub-dialog", "", ""),
                    "rule" => ("rule-dialog", "", ""),
                    "edit-rule" => ("edit-rule-dialog", "rule-proc-1", ""),
                    "rename" => ("rename-dialog", "sub-1", ""),
                    "about" => ("settings-action", "slint", ""),
                    _ => panic!("unsupported visual smoke action"),
                };
                state
                    .action(ui, action, id, value)
                    .expect("open smoke dialog");
            }
        });
    }
    if page == 4 {
        UI.with(|state| {
            if let Some(state) = state.borrow_mut().as_mut() {
                // The original browser preview uses these same non-sensitive fixture lines.
                for (level, message) in [
                    ("info", "umbra mock backend initialized"),
                    ("debug", "loaded profiles.json: 3 servers, 1 subscription"),
                    (
                        "warn",
                        "this is the browser mock — run `npm run tauri dev` for the real backend",
                    ),
                ] {
                    state.app.state().logs.push_now(level, message);
                }
                state.update_logs(ui);
            }
        });
    }
    let lifecycle = std::env::var_os("UMBRA_SMOKE_LIFECYCLE").is_some();
    if lifecycle {
        let weak = ui.as_weak();
        slint::Timer::single_shot(Duration::ZERO, move || {
            if let Some(ui) = weak.upgrade() {
                UI.with(|state| {
                    let state = state.borrow();
                    let state = state.as_ref().expect("UI state");
                    assert!(state.tray.is_some(), "native tray must be available");
                    state.close(&ui);
                    assert!(!ui.window().is_visible(), "close must hide to tray");
                    eprintln!("LIFECYCLE: hidden to native tray");
                });
            }
        });
    }
    let size = std::env::var("UMBRA_SMOKE_SIZE")
        .unwrap_or_else(|_| "1280x720".into())
        .split('x')
        .filter_map(|v| v.parse::<u32>().ok())
        .collect::<Vec<_>>();
    if size.len() == 2 {
        ui.window()
            .set_size(slint::PhysicalSize::new(size[0], size[1]));
    }
    let weak = ui.as_weak();
    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::SingleShot,
        Duration::from_millis(1800),
        move || {
            if let Some(ui) = weak.upgrade() {
                if lifecycle {
                    assert!(
                        ui.window().is_visible(),
                        "second launch must restore hidden window"
                    );
                    eprintln!("LIFECYCLE: second launch restored window");
                }
                let snapshot = rendered_frame
                    .borrow_mut()
                    .take()
                    .map(Ok)
                    .unwrap_or_else(|| ui.window().take_snapshot());
                match snapshot {
                    Ok(image) => {
                        assert!(
                            image
                                .as_slice()
                                .iter()
                                .any(|pixel| pixel.r > 4 || pixel.g > 4 || pixel.b > 4),
                            "snapshot must contain a rendered interface"
                        );
                        image::save_buffer(
                            &path,
                            image.as_bytes(),
                            image.width(),
                            image.height(),
                            image::ColorType::Rgba8,
                        )
                        .expect("save Slint capture");
                    }
                    Err(error) => eprintln!("capture failed: {error}"),
                };
                let _ = slint::quit_event_loop();
            }
        },
    );
    Some(timer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::AppState;
    use slint::Model;

    #[test]
    fn rule_inputs_preserve_original_normalization_and_reject_invalid_cidrs() {
        assert_eq!(
            normalize_rule_value(r#""C:\Games\Game.exe""#, 0, 0).unwrap(),
            "Game.exe"
        );
        assert_eq!(normalize_rule_value("chrome", 0, 0).unwrap(), "chrome.exe");
        assert_eq!(
            normalize_rule_value("https://*.Example.COM/path", 1, 0).unwrap(),
            "example.com"
        );
        assert_eq!(
            normalize_rule_value(r"^api\d+\.example\.com$", 1, 3).unwrap(),
            r"^api\d+\.example\.com$"
        );
        for invalid in ["1.2.3.999", "1.2.3.4/33", "::1/129", "1.2.3.4/32/1", ""] {
            assert!(normalize_rule_value(invalid, 2, 0).is_err(), "{invalid}");
        }
        for valid in ["1.2.3.4", "10.0.0.0/8", "2001:db8::/32", "::1"] {
            assert!(normalize_rule_value(valid, 2, 0).is_ok(), "{valid}");
        }
        assert!(normalize_rule_value("chrome.exe", 0, 1).is_err());
    }

    #[test]
    fn provider_notices_keep_all_http_and_telegram_links() {
        let links = notice_links("Support t.me/example and https://example.com/help");
        assert_eq!(
            links.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
            ["https://t.me/example", "https://example.com/help"]
        );
        assert!(notice_links("Support-Node-3 file:///secret").is_empty());
    }

    #[test]
    fn paused_logs_still_allow_filtering_copying_and_clearing() {
        let (_dir, mut state, ui) = fixture();
        let data = ui.global::<App>();
        state.app.state().logs.push_now("info", "alpha");
        state.app.state().logs.push_now("debug", "beta");
        state.update_logs(&ui);
        assert_eq!(data.get_logs().row_count(), 2);
        data.set_log_paused(true);
        state.action(&ui, "logs", "", "").unwrap();
        state.app.state().logs.push_now("info", "new arrival");
        state.update_logs(&ui);
        assert_eq!(data.get_logs().row_count(), 2);
        data.set_log_query("beta".into());
        state.action(&ui, "logs", "", "").unwrap();
        assert_eq!(data.get_logs().row_count(), 1);
        assert!(data.get_log_text().contains("beta"));
        state.action(&ui, "clear-logs", "", "").unwrap();
        assert_eq!(data.get_logs().row_count(), 0);
        assert!(data.get_log_text().is_empty());
    }

    #[test]
    fn accessible_navigation_and_modal_escape_work_on_actual_components() {
        let (_dir, _state, ui) = fixture();
        for (label, page) in [
            ("Серверы", 1),
            ("Подписки", 2),
            ("Маршрутизация", 3),
            ("Журнал", 4),
            ("Настройки", 5),
            ("Главная", 0),
        ] {
            let button =
                i_slint_backend_testing::ElementHandle::find_by_accessible_label(&ui, label)
                    .next()
                    .expect("navigation button");
            button.invoke_accessible_default_action();
            assert_eq!(ui.global::<App>().get_page(), page, "{label}");
        }
        ui.global::<App>().set_modal(5);
        ui.window()
            .dispatch_event(slint::platform::WindowEvent::KeyPressed {
                text: slint::platform::Key::Escape.into(),
            });
        assert_eq!(ui.global::<App>().get_modal(), 0);
    }

    #[tokio::test]
    async fn routing_changes_require_reconnect_until_a_new_session() {
        let (_dir, mut state, ui) = fixture();
        {
            let mut conn = state.app.state().conn.write().await;
            conn.status = ConnStatus::Connected;
            conn.since_ms = Some(1000);
        }
        state.refresh(&ui);
        assert!(!ui.global::<App>().get_route_dirty());
        state
            .action(&ui, "rule-toggle", "rule-proc-1", "false")
            .unwrap();
        finish(&mut state, &ui).await;
        assert!(ui.global::<App>().get_route_dirty());
        state.app.state().conn.write().await.since_ms = Some(2000);
        state.refresh(&ui);
        assert!(!ui.global::<App>().get_route_dirty());
    }

    fn fixture() -> (tempfile::TempDir, UiState, MainWindow) {
        i_slint_backend_testing::init_no_event_loop();
        let dir = tempfile::tempdir().unwrap();
        let settings: Settings = serde_json::from_str(include_str!(
            "../../tests/fixtures/visual/settings.fixture.json"
        ))
        .unwrap();
        let profiles: ProfileStore = serde_json::from_str(include_str!(
            "../../tests/fixtures/visual/profiles.fixture.json"
        ))
        .unwrap();
        let app = AppHandle::new(AppState::new(dir.path().into(), settings.clone(), profiles));
        let ui = MainWindow::new().unwrap();
        ui.show().unwrap();
        let mut state = UiState::new(app, settings, None);
        state.refresh(&ui);
        (dir, state, ui)
    }

    async fn finish(state: &mut UiState, ui: &MainWindow) {
        tokio::time::timeout(Duration::from_secs(3), async {
            while state.receiver.is_some() {
                tokio::time::sleep(Duration::from_millis(5)).await;
                state.refresh(ui);
            }
        })
        .await
        .unwrap();
    }

    #[test]
    fn original_preview_data_drives_all_native_models() {
        let (_dir, _state, ui) = fixture();
        let data = ui.global::<App>();
        assert_eq!(data.get_server_name(), "NL Amsterdam-1");
        assert_eq!(data.get_server_total(), "335 ГБ");
        assert_eq!(data.get_ping(), "43");
        let rows = data.get_servers();
        assert_eq!(
            rows.iter().filter(|s| s.selected).count(),
            1,
            "selected favorite is highlighted in the favorites group only"
        );
        assert_eq!(rows.iter().filter(|s| s.group).count(), 4);
        assert_eq!(data.get_subscriptions().row_count(), 2);
        assert_eq!(data.get_routes().row_count(), 5);
        assert!(data.get_settings().iter().any(|s| s.key == "sendHwid"));
        for size in [
            [480, 430],
            [720, 500],
            [1100, 720],
            [1280, 720],
            [1440, 900],
        ] {
            ui.window()
                .set_size(slint::PhysicalSize::new(size[0], size[1]));
            for page in 0..6 {
                data.set_page(page);
                i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(200));
            }
        }
    }

    #[tokio::test]
    async fn rule_editor_saves_matchers_order_and_deletion() {
        let (dir, mut state, ui) = fixture();
        let data = ui.global::<App>();
        state.action(&ui, "rule-dialog", "", "").unwrap();
        data.set_field0("C:\\Games\\game.exe".into());
        data.set_field1("Example process".into());
        data.set_edit_type(0);
        data.set_edit_matcher(1);
        data.set_edit_action(1);
        state.action(&ui, "modal-submit", "", "").unwrap();
        finish(&mut state, &ui).await;
        assert_eq!(data.get_modal(), 0);
        let saved = crate::storage::load_settings(dir.path());
        let rule = saved.routing_rules.last().unwrap();
        assert_eq!(rule.process_matcher, Some(ProcessMatcher::Path));
        assert_eq!(rule.action, AppRouteAction::Direct);
        let id = rule.id.clone();
        state.action(&ui, "rule-move", &id, "-1").unwrap();
        finish(&mut state, &ui).await;
        assert_eq!(state.settings.routing_rules[4].id, id);
        state.action(&ui, "delete-rule", &id, "").unwrap();
        finish(&mut state, &ui).await;
        assert_eq!(state.settings.routing_rules.len(), 5);
        assert!(
            state.settings.app_routes.is_empty(),
            "deleting all native rules must not resurrect legacy rules"
        );
    }

    #[tokio::test]
    async fn failed_save_retains_modal_and_draft() {
        let (dir, mut state, ui) = fixture();
        let data = ui.global::<App>();
        std::fs::create_dir(dir.path().join("settings.json")).unwrap();
        std::fs::write(dir.path().join("settings.json/keep"), "x").unwrap();
        state.action(&ui, "rule-dialog", "", "").unwrap();
        data.set_field0("example.exe".into());
        state.action(&ui, "modal-submit", "", "").unwrap();
        finish(&mut state, &ui).await;
        assert_eq!(data.get_modal(), 5);
        assert_eq!(data.get_field0(), "example.exe");
        assert!(data.get_message_error());
        assert_eq!(state.settings.routing_rules.len(), 5);
    }

    #[tokio::test]
    async fn settings_changes_preserve_live_selection_and_proxy_metadata() {
        let (dir, mut state, ui) = fixture();
        assert!(state.action(&ui, "setting", "mixedPort", "0").is_err());
        state.action(&ui, "setting-choice", "accent", "1").unwrap();
        finish(&mut state, &ui).await;
        let settings = crate::storage::load_settings(dir.path());
        assert_eq!(settings.accent, "cyan");
        assert_eq!(settings.selected_server_id.as_deref(), Some("srv-1"));
        assert!(!settings.proxy_owned);
        state.action(&ui, "collapse", "sub-1", "").unwrap();
        finish(&mut state, &ui).await;
        assert!(state
            .settings
            .collapsed_groups
            .iter()
            .any(|id| id == "sub-1"));
    }
}
