//! Shared native application handle. No WebView, IPC server or unbounded queues.
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use serde_json::Value;

use crate::state::AppState;
type UiWaker = Arc<dyn Fn() + Send + Sync>;

#[derive(Clone)]
pub struct AppHandle {
    state: Arc<AppState>,
    waker: Arc<Mutex<Option<UiWaker>>>,
    // Coalesce high frequency events before waking the retained UI.
    events: Arc<Mutex<HashMap<&'static str, Value>>>,
}

impl AppHandle {
    pub fn new(state: AppState) -> Self {
        Self {
            state: Arc::new(state),
            waker: Arc::default(),
            events: Arc::default(),
        }
    }

    pub fn state(&self) -> &AppState {
        &self.state
    }

    pub fn emit(&self, event: &'static str, payload: impl Serialize) -> serde_json::Result<()> {
        // Logs live in one bounded ring; connection state is read directly.
        if event != crate::events::EV_CORE_LOG && event != crate::events::EV_CONN_STATE {
            self.events
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(event, serde_json::to_value(payload)?);
        }
        self.wake();
        Ok(())
    }

    pub fn take_events(&self) -> HashMap<&'static str, Value> {
        std::mem::take(&mut *self.events.lock().unwrap_or_else(|e| e.into_inner()))
    }

    pub fn set_waker(&self, wake: Arc<dyn Fn() + Send + Sync>) {
        *self.waker.lock().unwrap_or_else(|e| e.into_inner()) = Some(wake);
    }
    pub fn wake(&self) {
        let callback = self.waker.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some(callback) = callback {
            callback();
        }
    }

    pub fn exit(&self, _code: i32) {
        // The event loop returns normally so teardown can restore the proxy.
        let _ = self.emit("app://quit", ());
    }
}

pub fn set_autostart(enabled: bool) -> crate::error::AppResult<()> {
    #[cfg(windows)]
    {
        use winreg::{enums::*, RegKey};
        let (key, _) = RegKey::predef(HKEY_CURRENT_USER)
            .create_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Run")?;
        if enabled {
            let exe = std::env::current_exe()?;
            key.set_value("Umbra", &format!("\"{}\" --minimized", exe.display()))?;
        } else {
            match key.delete_value("Umbra") {
                Ok(()) => (),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                Err(e) => return Err(e.into()),
            }
        }
    }
    #[cfg(not(windows))]
    if enabled {
        return Err(crate::error::AppError::Unsupported(
            "autostart is Windows-only".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hidden_ui_keeps_only_latest_traffic_and_no_log_copies() {
        let dir = tempfile::tempdir().unwrap();
        let app = AppHandle::new(AppState::new(
            dir.path().into(),
            Default::default(),
            Default::default(),
        ));
        for i in 0..10000 {
            app.emit(crate::events::EV_TRAFFIC, i).unwrap();
            app.emit(crate::events::EV_CORE_LOG, "ignored").unwrap();
        }
        let events = app.take_events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[crate::events::EV_TRAFFIC], 9999);
        assert!(app.take_events().is_empty());
    }
}
