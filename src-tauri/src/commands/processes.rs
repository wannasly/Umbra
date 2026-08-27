//! Process enumeration using Toolhelp32 snapshot and EnumWindows.

use crate::error::{AppError, AppResult};
use crate::models::RunningProcess;
use std::collections::{HashMap, HashSet};

#[cfg(windows)]
pub fn list_running_processes() -> Vec<RunningProcess> {
    use windows::core::BOOL;
    use windows::Win32::Foundation::{HWND, LPARAM};
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_FORMAT,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId,
        IsWindowVisible,
    };

    // 1. Collect visible window titles mapped to PID
    struct WindowEnumState {
        titles: HashMap<u32, String>,
    }

    unsafe extern "system" fn enum_windows_callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let state = &mut *(lparam.0 as *mut WindowEnumState);
        if IsWindowVisible(hwnd).as_bool() {
            let len = GetWindowTextLengthW(hwnd);
            if len > 0 {
                let mut buf = vec![0u16; (len + 1) as usize];
                let actual = GetWindowTextW(hwnd, &mut buf);
                if actual > 0 {
                    let title = String::from_utf16_lossy(&buf[..actual as usize])
                        .trim()
                        .to_string();
                    if !title.is_empty() {
                        let mut pid = 0u32;
                        GetWindowThreadProcessId(hwnd, Some(&mut pid));
                        if pid != 0 && !state.titles.contains_key(&pid) {
                            state.titles.insert(pid, title);
                        }
                    }
                }
            }
        }
        BOOL(1)
    }

    let mut win_state = WindowEnumState {
        titles: HashMap::new(),
    };

    unsafe {
        let _ = EnumWindows(
            Some(enum_windows_callback),
            LPARAM(&mut win_state as *mut _ as isize),
        );
    }

    // 2. Snapshot processes via Toolhelp32
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    let Ok(snapshot) = snapshot else {
        return Vec::new();
    };

    let mut entry = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };

    let mut processes = Vec::new();
    let mut seen_keys = HashSet::new();

    unsafe {
        if Process32FirstW(snapshot, &mut entry).is_ok() {
            loop {
                let pid = entry.th32ProcessID;
                if pid > 4 {
                    // Extract exe file name
                    let exe_len = entry
                        .szExeFile
                        .iter()
                        .position(|&c| c == 0)
                        .unwrap_or(entry.szExeFile.len());
                    let exe_name = String::from_utf16_lossy(&entry.szExeFile[..exe_len])
                        .trim()
                        .to_string();

                    if !exe_name.is_empty()
                        && !exe_name.eq_ignore_ascii_case("svchost.exe")
                        && !exe_name.eq_ignore_ascii_case("dwm.exe")
                        && !exe_name.eq_ignore_ascii_case("smss.exe")
                        && !exe_name.eq_ignore_ascii_case("csrss.exe")
                        && !exe_name.eq_ignore_ascii_case("wininit.exe")
                        && !exe_name.eq_ignore_ascii_case("winlogon.exe")
                        && !exe_name.eq_ignore_ascii_case("services.exe")
                        && !exe_name.eq_ignore_ascii_case("lsass.exe")
                        && !exe_name.eq_ignore_ascii_case("fontdrvhost.exe")
                        && !exe_name.eq_ignore_ascii_case("RuntimeBroker.exe")
                        && !exe_name.eq_ignore_ascii_case("sihost.exe")
                        && !exe_name.eq_ignore_ascii_case("ctfmon.exe")
                        && !exe_name.eq_ignore_ascii_case("conhost.exe")
                    {
                        // Try querying full path
                        let mut full_path: Option<String> = None;
                        if let Ok(h_proc) =
                            OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid)
                        {
                            let mut path_buf = [0u16; 1024];
                            let mut path_len = path_buf.len() as u32;
                            if QueryFullProcessImageNameW(
                                h_proc,
                                PROCESS_NAME_FORMAT(0),
                                windows::core::PWSTR(path_buf.as_mut_ptr()),
                                &mut path_len,
                            )
                            .is_ok()
                                && path_len > 0
                            {
                                full_path =
                                    Some(String::from_utf16_lossy(&path_buf[..path_len as usize]));
                            }
                            let _ = windows::Win32::Foundation::CloseHandle(h_proc);
                        }

                        let title = win_state.titles.get(&pid).cloned();
                        let key = (exe_name.to_lowercase(), title.clone());

                        if seen_keys.insert(key) {
                            processes.push(RunningProcess {
                                pid,
                                name: exe_name,
                                title,
                                path: full_path,
                            });
                        }
                    }
                }

                if Process32NextW(snapshot, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = windows::Win32::Foundation::CloseHandle(snapshot);
    }

    // Sort: processes with window titles first, then by name
    processes.sort_by(|a, b| match (a.title.is_some(), b.title.is_some()) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });

    processes
}

#[cfg(not(windows))]
pub fn list_running_processes() -> Vec<RunningProcess> {
    Vec::new()
}

#[tauri::command]
pub async fn get_running_processes() -> AppResult<Vec<RunningProcess>> {
    tokio::task::spawn_blocking(list_running_processes)
        .await
        .map_err(|e| AppError::Internal(format!("process enumeration failed: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(windows)]
    fn test_list_running_processes_returns_valid_processes() {
        let procs = list_running_processes();
        assert!(
            !procs.is_empty(),
            "Processes list should not be empty on Windows"
        );

        let mut has_window_title = false;
        let mut has_full_path = false;

        for p in &procs {
            assert!(p.pid > 0, "PID must be > 0: {:?}", p);
            assert!(
                !p.name.is_empty(),
                "Process name must not be empty: {:?}",
                p
            );
            assert!(
                !p.name.eq_ignore_ascii_case("svchost.exe"),
                "svchost.exe should be filtered out"
            );
            if p.title.is_some() {
                has_window_title = true;
            }
            if p.path.is_some() {
                has_full_path = true;
            }
        }

        println!(
            "Found {} processes. Window title found: {}, Full path found: {}",
            procs.len(),
            has_window_title,
            has_full_path
        );
        for p in procs.iter().take(5) {
            println!(
                "  [PID: {}] name: {:?}, title: {:?}, path: {:?}",
                p.pid, p.name, p.title, p.path
            );
        }
        // On a running desktop system, we should have at least some processes with full paths
        assert!(
            has_full_path,
            "At least one process should have a resolved path"
        );
    }
}
