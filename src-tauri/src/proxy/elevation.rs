//! Process elevation: token inspection and UAC relaunch.

use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

use crate::error::{AppError, AppResult};

/// Handed to the elevated instance so it knows to pick the connection back up.
pub const RESUME_TUN_FLAG: &str = "--resume-tun";

/// Whether this process was started by the elevation relaunch.
pub fn is_resume_tun() -> bool {
    has_resume_tun(std::env::args())
}

fn has_resume_tun(mut args: impl Iterator<Item = String>) -> bool {
    args.any(|a| a == RESUME_TUN_FLAG)
}

/// Whether the current process runs with an elevated (admin) token.
pub fn is_elevated() -> bool {
    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return false;
        }
        let mut info = TOKEN_ELEVATION::default();
        let mut len = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut info as *mut TOKEN_ELEVATION as *mut _),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut len,
        );
        let _ = CloseHandle(token);
        ok.is_ok() && info.TokenIsElevated != 0
    }
}

/// Relaunch the current executable elevated (UAC "runas" verb).
/// The caller is expected to exit the current process afterwards, and to do so
/// promptly: the new instance goes through `tauri-plugin-single-instance`, so
/// if it starts before this one is gone it hands its args over and exits. The
/// UAC prompt makes that race practically unreachable, but it is the reason
/// `ShellExecuteW` is followed immediately by `app.exit(0)`.
pub fn relaunch_elevated(extra_args: &[&str]) -> AppResult<()> {
    let exe = std::env::current_exe()?;
    let verb = HSTRING::from("runas");
    let file = HSTRING::from(exe.as_os_str());
    let params = HSTRING::from(join_args(extra_args));
    let hinstance =
        unsafe { ShellExecuteW(None, &verb, &file, &params, PCWSTR::null(), SW_SHOWNORMAL) };
    // ShellExecuteW returns a value > 32 on success.
    let code = hinstance.0 as isize;
    if code > 32 {
        Ok(())
    } else {
        Err(AppError::Internal(format!(
            "elevation request failed (ShellExecuteW code {code})"
        )))
    }
}

fn join_args(args: &[&str]) -> String {
    args.iter()
        .map(|a| {
            if a.contains(char::is_whitespace) {
                format!("\"{a}\"")
            } else {
                (*a).to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_args_space_separated() {
        assert_eq!(
            join_args(&["--resume-tun", "--minimized"]),
            "--resume-tun --minimized"
        );
    }

    #[test]
    fn join_args_quotes_whitespace() {
        assert_eq!(
            join_args(&["--path", "C:\\a b\\c"]),
            "--path \"C:\\a b\\c\""
        );
    }

    #[test]
    fn join_args_empty() {
        assert_eq!(join_args(&[]), "");
    }

    fn args(list: &[&str]) -> impl Iterator<Item = String> {
        list.iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
            .into_iter()
    }

    #[test]
    fn resume_tun_flag_detected_anywhere_in_argv() {
        assert!(has_resume_tun(args(&["umbra.exe", RESUME_TUN_FLAG])));
        assert!(has_resume_tun(args(&[
            "umbra.exe",
            "--minimized",
            RESUME_TUN_FLAG
        ])));
    }

    #[test]
    fn resume_tun_flag_absent_or_partial_is_not_detected() {
        assert!(!has_resume_tun(args(&["umbra.exe"])));
        assert!(!has_resume_tun(args(&["umbra.exe", "--minimized"])));
        assert!(!has_resume_tun(args(&["umbra.exe", "--resume-tun=1"])));
        assert!(!has_resume_tun(args(&["umbra.exe", "resume-tun"])));
    }

    #[test]
    fn resume_tun_flag_survives_the_command_line_round_trip() {
        // What relaunch_elevated passes must be what the new process matches on.
        assert_eq!(join_args(&[RESUME_TUN_FLAG]), RESUME_TUN_FLAG);
    }
}
