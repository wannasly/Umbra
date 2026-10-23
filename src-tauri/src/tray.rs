//! Native tray objects stay on the UI thread.
#[cfg(windows)]
use tray_icon::{
    menu::{Menu, MenuEvent, MenuItem},
    TrayIcon, TrayIconBuilder, TrayIconEvent,
};

#[derive(Clone, Copy)]
pub enum Action {
    Show,
    Toggle,
    System,
    Tun,
    Quit,
}

#[cfg(windows)]
pub struct NativeTray {
    icon: TrayIcon,
    show: MenuItem,
    toggle: MenuItem,
    system: MenuItem,
    tun: MenuItem,
    quit: MenuItem,
    signature: String,
}

#[cfg(windows)]
impl NativeTray {
    pub fn new(app: &crate::runtime::AppHandle) -> Result<Self, Box<dyn std::error::Error>> {
        let menu = Menu::new();
        let show = MenuItem::new("Открыть Umbra", true, None);
        let toggle = MenuItem::new("Подключить", true, None);
        let system = MenuItem::new("Системный прокси", true, None);
        let tun = MenuItem::new("TUN", true, None);
        let quit = MenuItem::new("Выход", true, None);
        menu.append_items(&[&show, &toggle, &system, &tun, &quit])?;
        let image =
            image::load_from_memory(include_bytes!("../icons/tray-disconnected.png"))?.into_rgba8();
        let (w, h) = image.dimensions();
        let icon = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("Umbra")
            .with_icon(tray_icon::Icon::from_rgba(image.into_raw(), w, h)?)
            .build()?;
        let wake = app.clone();
        MenuEvent::set_event_handler(Some(move |event| {
            let mut queue = MENU_EVENTS.lock().unwrap_or_else(|e| e.into_inner());
            if queue.len() < 32 {
                queue.push_back(event);
            }
            wake.wake();
        }));
        let wake = app.clone();
        TrayIconEvent::set_event_handler(Some(move |event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: tray_icon::MouseButton::Left,
                    button_state: tray_icon::MouseButtonState::Up,
                    ..
                }
            ) {
                SHOW_REQUESTED.store(true, std::sync::atomic::Ordering::Relaxed);
                wake.wake();
            }
        }));
        Ok(Self {
            icon,
            show,
            toggle,
            system,
            tun,
            quit,
            signature: String::new(),
        })
    }

    pub fn actions(&self) -> Vec<Action> {
        let mut out = Vec::new();
        if SHOW_REQUESTED.swap(false, std::sync::atomic::Ordering::Relaxed) {
            out.push(Action::Show);
        }
        for event in MENU_EVENTS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .drain(..)
        {
            let action = if event.id == *self.show.id() {
                Action::Show
            } else if event.id == *self.toggle.id() {
                Action::Toggle
            } else if event.id == *self.system.id() {
                Action::System
            } else if event.id == *self.tun.id() {
                Action::Tun
            } else if event.id == *self.quit.id() {
                Action::Quit
            } else {
                continue;
            };
            out.push(action);
        }
        out
    }

    pub fn sync(&mut self, conn: &crate::models::ConnectionState, ru: bool) {
        let signature = format!("{:?}:{:?}:{ru}", conn.status, conn.server_name);
        if signature == self.signature {
            return;
        }
        self.signature = signature;
        use crate::models::ConnStatus;
        let connected = conn.status == ConnStatus::Connected;
        self.show.set_text(if ru {
            "Открыть Umbra"
        } else {
            "Open Umbra"
        });
        self.toggle.set_text(match (connected, ru) {
            (true, true) => "Отключить",
            (true, false) => "Disconnect",
            (false, true) => "Подключить",
            (false, false) => "Connect",
        });
        self.toggle.set_enabled(matches!(
            conn.status,
            ConnStatus::Connected | ConnStatus::Disconnected
        ));
        self.system.set_text(if ru {
            "Системный прокси"
        } else {
            "System proxy"
        });
        self.quit.set_text(if ru { "Выход" } else { "Quit" });
        let bytes: &[u8] = if connected {
            include_bytes!("../icons/tray-connected.png")
        } else {
            include_bytes!("../icons/tray-disconnected.png")
        };
        if let Ok(image) = image::load_from_memory(bytes) {
            let image = image.into_rgba8();
            let (w, h) = image.dimensions();
            if let Ok(icon) = tray_icon::Icon::from_rgba(image.into_raw(), w, h) {
                let _ = self.icon.set_icon(Some(icon));
            }
        }
        let tooltip = format!(
            "Umbra — {}",
            conn.server_name.as_deref().unwrap_or("sing-box")
        );
        let _ = self.icon.set_tooltip(Some(tooltip));
    }
}

#[cfg(windows)]
static MENU_EVENTS: std::sync::Mutex<std::collections::VecDeque<MenuEvent>> =
    std::sync::Mutex::new(std::collections::VecDeque::new());
#[cfg(windows)]
static SHOW_REQUESTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(not(windows))]
pub struct NativeTray;
#[cfg(not(windows))]
impl NativeTray {
    pub fn new(_: &crate::runtime::AppHandle) -> Result<Self, Box<dyn std::error::Error>> {
        Err("tray is Windows-only".into())
    }
    pub fn actions(&self) -> Vec<Action> {
        Vec::new()
    }
    pub fn sync(&mut self, _: &crate::models::ConnectionState, _: bool) {}
}
