//! Administrator rights: are we running with them, and restarting the app with them.

use crate::error::{CmdError, CmdResult};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, GetLastError, WAIT_OBJECT_0};
use windows::Win32::System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE};
use windows::Win32::UI::Shell::{IsUserAnAdmin, ShellExecuteW};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

pub const WAIT_ARG: &str = "--wait-pid";

pub fn is_elevated() -> bool {
    unsafe { IsUserAnAdmin().as_bool() }
}

/// Starts a second copy of the app as administrator (Windows shows its own consent prompt).
/// The copy waits for this one to exit first, so there are never two task managers.
/// Returns `cancelled` when the user says no to the prompt; nothing changes then.
pub fn relaunch_elevated() -> CmdResult<()> {
    let exe = std::env::current_exe().map_err(|e| CmdError::failed(format!("Cannot find the app file: {e}")))?;
    let exe_w: Vec<u16> = exe.to_string_lossy().encode_utf16().chain(std::iter::once(0)).collect();
    let args = format!("{WAIT_ARG} {}", std::process::id());
    let args_w: Vec<u16> = args.encode_utf16().chain(std::iter::once(0)).collect();
    let result = unsafe {
        ShellExecuteW(None, w!("runas"), PCWSTR(exe_w.as_ptr()), PCWSTR(args_w.as_ptr()), PCWSTR::null(), SW_SHOWNORMAL)
    };
    // ShellExecute reports success as a value above 32.
    if result.0 as usize > 32 {
        Ok(())
    } else {
        Err(CmdError::from_win32(unsafe { GetLastError().0 }, "Restarting as administrator"))
    }
}

/// Reads `--wait-pid <n>` from the command line.
pub fn wait_pid_from_args(args: &[String]) -> Option<u32> {
    let at = args.iter().position(|a| a == WAIT_ARG)?;
    args.get(at + 1)?.parse().ok()
}

/// Called first thing in the elevated copy: wait (at most 10 s) for the old copy to exit.
pub fn wait_for_exit(pid: u32) {
    unsafe {
        if let Ok(handle) = OpenProcess(PROCESS_SYNCHRONIZE, false, pid) {
            let _ = WaitForSingleObject(handle, 10_000) == WAIT_OBJECT_0;
            let _ = CloseHandle(handle);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn reads_the_wait_pid() {
        assert_eq!(wait_pid_from_args(&args(&["app.exe", "--wait-pid", "1234"])), Some(1234));
        assert_eq!(wait_pid_from_args(&args(&["app.exe", "--hidden", "--wait-pid", "8"])), Some(8));
    }

    #[test]
    fn bad_or_missing_wait_pid_is_ignored() {
        assert_eq!(wait_pid_from_args(&args(&["app.exe"])), None);
        assert_eq!(wait_pid_from_args(&args(&["app.exe", "--wait-pid"])), None);
        assert_eq!(wait_pid_from_args(&args(&["app.exe", "--wait-pid", "abc"])), None);
    }

    #[test]
    fn waiting_for_a_process_that_is_gone_returns_at_once() {
        let started = std::time::Instant::now();
        wait_for_exit(4_000_000_001);
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
    }

    #[test]
    fn waiting_returns_when_the_process_exits() {
        let mut child = std::process::Command::new("cmd").args(["/c", "ping -n 2 127.0.0.1 >nul"]).spawn().unwrap();
        let started = std::time::Instant::now();
        wait_for_exit(child.id());
        assert!(started.elapsed() < std::time::Duration::from_secs(8));
        assert!(child.try_wait().unwrap().is_some(), "the process had exited when the wait returned");
    }
}
