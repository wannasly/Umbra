//! OS-owned singleton; a crash automatically releases the lock.
use crate::error::AppResult;
use std::path::Path;

/// Check the message window used by the original single-instance plugin before
/// touching shared profiles or attempting crash recovery of its active proxy.
#[cfg(windows)]
pub fn ensure_legacy_stopped() -> AppResult<()> {
    use windows::{core::w, Win32::UI::WindowsAndMessaging::FindWindowW};
    if unsafe { FindWindowW(w!("com.umbra.proxy-sic"), w!("com.umbra.proxy-siw")) }.is_ok() {
        return Err(crate::error::AppError::Internal("Завершите прежнюю Umbra через меню трея перед запуском новой версии. / Quit the previous Umbra from its tray before starting this version.".into()));
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn ensure_legacy_stopped() -> AppResult<()> {
    Ok(())
}

#[cfg(windows)]
pub struct Instance {
    mutex: windows::Win32::Foundation::HANDLE,
    show_event: windows::Win32::Foundation::HANDLE,
}

#[cfg(windows)]
static SHOW_EVENT: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

#[cfg(windows)]
pub fn take_show_request() -> bool {
    use std::sync::atomic::Ordering;
    let handle = SHOW_EVENT.load(Ordering::Acquire);
    handle != 0
        && unsafe {
            windows::Win32::System::Threading::WaitForSingleObject(
                windows::Win32::Foundation::HANDLE(handle as *mut _),
                0,
            ) == windows::Win32::Foundation::WAIT_OBJECT_0
        }
}

#[cfg(not(windows))]
pub fn take_show_request() -> bool {
    false
}

#[cfg(windows)]
pub fn acquire(data_dir: &Path) -> AppResult<Option<Instance>> {
    use sha2::{Digest, Sha256};
    use windows::{
        core::PCWSTR,
        Win32::{
            Foundation::{GetLastError, ERROR_ALREADY_EXISTS, WAIT_ABANDONED, WAIT_OBJECT_0},
            System::Threading::{CreateEventW, CreateMutexW, SetEvent, WaitForSingleObject},
        },
    };
    let digest = Sha256::digest(data_dir.to_string_lossy().as_bytes());
    let name: Vec<u16> = format!("Local\\Umbra-{:x}", digest)
        .encode_utf16()
        .chain(Some(0))
        .collect();
    unsafe {
        let event_name: Vec<u16> = format!("Local\\Umbra-{:x}-show", digest)
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let show_event = CreateEventW(None, false, false, PCWSTR(event_name.as_ptr()))
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        let handle = CreateMutexW(None, true, PCWSTR(name.as_ptr())).map_err(|e| {
            let _ = windows::Win32::Foundation::CloseHandle(show_event);
            std::io::Error::other(e.to_string())
        })?;
        if GetLastError() == ERROR_ALREADY_EXISTS {
            if crate::proxy::elevation::is_resume_tun() {
                let status = WaitForSingleObject(handle, 20_000);
                if status == WAIT_OBJECT_0 || status == WAIT_ABANDONED {
                    SHOW_EVENT.store(show_event.0 as isize, std::sync::atomic::Ordering::Release);
                    return Ok(Some(Instance {
                        mutex: handle,
                        show_event,
                    }));
                }
            } else {
                let _ = SetEvent(show_event);
            }
            let _ = windows::Win32::Foundation::CloseHandle(handle);
            let _ = windows::Win32::Foundation::CloseHandle(show_event);
            return Ok(None);
        }
        SHOW_EVENT.store(show_event.0 as isize, std::sync::atomic::Ordering::Release);
        Ok(Some(Instance {
            mutex: handle,
            show_event,
        }))
    }
}

#[cfg(windows)]
impl Drop for Instance {
    fn drop(&mut self) {
        unsafe {
            SHOW_EVENT.store(0, std::sync::atomic::Ordering::Release);
            let _ = windows::Win32::System::Threading::ReleaseMutex(self.mutex);
            let _ = windows::Win32::Foundation::CloseHandle(self.mutex);
            let _ = windows::Win32::Foundation::CloseHandle(self.show_event);
        }
    }
}

#[cfg(not(windows))]
pub struct Instance;
#[cfg(not(windows))]
pub fn acquire(_data_dir: &Path) -> AppResult<Option<Instance>> {
    Ok(Some(Instance))
}

#[cfg(all(test, windows))]
mod tests {
    #[test]
    #[ignore = "requires the original Tauri Umbra to be running; read-only compatibility check"]
    fn running_original_is_detected_before_shared_data_recovery() {
        assert!(super::ensure_legacy_stopped().is_err());
    }
}
