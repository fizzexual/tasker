//! Things you can do to a process besides ending it: priority, processors, efficiency mode,
//! pause and continue, dump file, and the small shell helpers (properties, run, search).

use crate::error::{CmdError, CmdResult};
use crate::procinfo::{self, Handle};
use crate::{procs, tree, wins};
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::{GetLastError, HANDLE};
use windows::Win32::System::Diagnostics::Debug::{MiniDumpWithFullMemory, MiniDumpWriteDump};
use windows::Win32::System::Threading::{
    ProcessPowerThrottling, SetPriorityClass, SetProcessAffinityMask, SetProcessInformation, TerminateProcess,
    PROCESS_ACCESS_RIGHTS, PROCESS_CREATION_FLAGS, PROCESS_POWER_THROTTLING_STATE, PROCESS_QUERY_INFORMATION,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_INFORMATION, PROCESS_SUSPEND_RESUME, PROCESS_TERMINATE,
    PROCESS_VM_READ,
};
use windows::Win32::UI::Shell::{
    SHObjectProperties, ShellExecuteExW, SEE_MASK_FLAG_NO_UI, SHELLEXECUTEINFOW, SHOP_FILEPATH,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

#[link(name = "ntdll")]
extern "system" {
    fn NtSuspendProcess(process: HANDLE) -> i32;
    fn NtResumeProcess(process: HANDLE) -> i32;
}

const EXECUTION_SPEED: u32 = 1;
const IDLE_PRIORITY: u32 = 0x40;
const NORMAL_PRIORITY: u32 = 0x20;

/// Opens the process the user picked, making sure it still is that process: a PID can be
/// handed to a new program between the click and the request.
fn open_checked(pid: u32, expected_name: &str, access: PROCESS_ACCESS_RIGHTS, what: &str) -> CmdResult<Handle> {
    let list = procs::list();
    let found = list.iter().find(|p| p.pid == pid).ok_or_else(|| CmdError::not_found("That process"))?;
    if !found.name.eq_ignore_ascii_case(expected_name) {
        return Err(CmdError::not_found("That process"));
    }
    procinfo::open(pid, access).map_err(|code| CmdError::from_win32(code, what))
}

fn last_error(what: &str) -> CmdError {
    CmdError::from_win32(unsafe { GetLastError().0 }, what)
}

pub fn set_priority(pid: u32, name: &str, priority: &str) -> CmdResult<&'static str> {
    let class = procinfo::priority_class(priority).ok_or_else(|| CmdError::invalid("Unknown priority."))?;
    let what = format!("Changing the priority of {name}");
    let handle = open_checked(pid, name, PROCESS_SET_INFORMATION | PROCESS_QUERY_LIMITED_INFORMATION, &what)?;
    unsafe { SetPriorityClass(handle.0, PROCESS_CREATION_FLAGS(class)).map_err(|_| last_error(&what))? };
    // Windows quietly gives "high" when "realtime" is asked for without administrator rights:
    // report what was really set.
    Ok(procinfo::priority_of(pid))
}

/// `mask` has one bit per processor the process may use. Returns the mask now in force.
pub fn set_affinity(pid: u32, name: &str, mask: u64) -> CmdResult<u64> {
    let what = format!("Changing the processors of {name}");
    let (_, system) = procinfo::affinity_of(pid).ok_or_else(|| CmdError::from_win32(5, &what))?;
    let wanted = mask & system;
    if wanted == 0 {
        return Err(CmdError::invalid("Pick at least one processor."));
    }
    let handle = open_checked(pid, name, PROCESS_SET_INFORMATION | PROCESS_QUERY_LIMITED_INFORMATION, &what)?;
    unsafe { SetProcessAffinityMask(handle.0, wanted as usize).map_err(|_| last_error(&what))? };
    Ok(procinfo::affinity_of(pid).map(|(mine, _)| mine).unwrap_or(wanted))
}

/// Efficiency mode, as in Windows Task Manager: low priority plus the power-saving speed.
pub fn set_efficiency(pid: u32, name: &str, on: bool) -> CmdResult<()> {
    let what = format!("Changing efficiency mode of {name}");
    if tree::is_protected(pid, name) {
        return Err(CmdError::new("protected", format!("{name} is part of Windows and cannot be slowed down.")));
    }
    let handle = open_checked(pid, name, PROCESS_SET_INFORMATION | PROCESS_QUERY_LIMITED_INFORMATION, &what)?;
    let state = PROCESS_POWER_THROTTLING_STATE {
        Version: 1,
        ControlMask: EXECUTION_SPEED,
        StateMask: if on { EXECUTION_SPEED } else { 0 },
    };
    unsafe {
        SetProcessInformation(
            handle.0,
            ProcessPowerThrottling,
            &state as *const _ as *const _,
            std::mem::size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
        )
        .map_err(|_| last_error(&what))?;
        let class = if on { IDLE_PRIORITY } else { NORMAL_PRIORITY };
        SetPriorityClass(handle.0, PROCESS_CREATION_FLAGS(class)).map_err(|_| last_error(&what))?;
    }
    Ok(())
}

/// Freezes (`pause = true`) or continues every thread of a process.
pub fn set_paused(pid: u32, name: &str, pause: bool, own_pid: u32) -> CmdResult<()> {
    let what = format!("{} {name}", if pause { "Pausing" } else { "Continuing" });
    if pause {
        if tree::is_protected(pid, name) {
            return Err(CmdError::new("protected", format!("{name} is part of Windows. Pausing it would freeze the PC.")));
        }
        if pid == own_pid {
            return Err(CmdError::new("protected", "The task manager cannot pause itself: nothing could continue it."));
        }
    }
    let handle = open_checked(pid, name, PROCESS_SUSPEND_RESUME, &what)?;
    let status = unsafe {
        if pause {
            NtSuspendProcess(handle.0)
        } else {
            NtResumeProcess(handle.0)
        }
    };
    if status < 0 {
        return Err(CmdError::failed(format!("{what} was refused by Windows (0x{:08X}).", status as u32)));
    }
    Ok(())
}

/// A file name that is free: `chrome (1234).DMP`, then `chrome (1234) 2.DMP`…
pub fn dump_path(folder: &Path, name: &str, pid: u32, taken: impl Fn(&Path) -> bool) -> PathBuf {
    let stem: String = name
        .trim_end_matches(".exe")
        .trim_end_matches(".EXE")
        .chars()
        .map(|c| if c.is_alphanumeric() || " -_.".contains(c) { c } else { '_' })
        .collect();
    let stem = if stem.trim().is_empty() { "process".to_string() } else { stem };
    let first = folder.join(format!("{stem} ({pid}).DMP"));
    if !taken(&first) {
        return first;
    }
    (2..).map(|n| folder.join(format!("{stem} ({pid}) {n}.DMP"))).find(|p| !taken(p)).unwrap_or(first)
}

/// Writes a full memory dump next to the user's temporary files and returns its path.
pub fn create_dump(pid: u32, name: &str) -> CmdResult<String> {
    let what = format!("Creating a dump file of {name}");
    let handle = open_checked(pid, name, PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, &what)?;
    let path = dump_path(&std::env::temp_dir(), name, pid, |p| p.exists());
    let file = std::fs::File::create(&path).map_err(|e| CmdError::failed(format!("Could not create {}: {e}", path.display())))?;
    let written = unsafe {
        MiniDumpWriteDump(handle.0, pid, HANDLE(file.as_raw_handle()), MiniDumpWithFullMemory, None, None, None)
    };
    drop(file);
    if written.is_err() {
        let error = last_error(&what);
        let _ = std::fs::remove_file(&path);
        return Err(error);
    }
    Ok(path.to_string_lossy().into_owned())
}

/// Opens the Windows "Properties" window of a file.
pub fn show_properties(path: &str) -> CmdResult<()> {
    if path.is_empty() || !Path::new(path).exists() {
        return Err(CmdError::failed("Windows does not reveal where this program is."));
    }
    let ok = unsafe { SHObjectProperties(None, SHOP_FILEPATH, &HSTRING::from(path), PCWSTR::null()) };
    if ok.as_bool() {
        Ok(())
    } else {
        Err(CmdError::failed("Windows could not open the properties."))
    }
}

/// Splits what was typed in "Run new task" into the program and its arguments.
/// `exists` tells whether a path is a real file, so `C:\Program Files\App\app.exe` typed
/// without quotes still works.
pub fn split_command(typed: &str, exists: impl Fn(&str) -> bool) -> Option<(String, String)> {
    let typed = typed.trim();
    if typed.is_empty() {
        return None;
    }
    if let Some(rest) = typed.strip_prefix('"') {
        let end = rest.find('"')?;
        let file = &rest[..end];
        return (!file.trim().is_empty()).then(|| (file.to_string(), rest[end + 1..].trim().to_string()));
    }
    if exists(typed) {
        return Some((typed.to_string(), String::new()));
    }
    // The longest leading part that is a real file wins; otherwise the first word.
    let breaks: Vec<usize> = typed.match_indices(char::is_whitespace).map(|(i, _)| i).collect();
    for at in breaks.iter().rev() {
        if exists(&typed[..*at]) {
            return Some((typed[..*at].to_string(), typed[*at..].trim().to_string()));
        }
    }
    match breaks.first() {
        Some(at) => Some((typed[..*at].to_string(), typed[*at..].trim().to_string())),
        None => Some((typed.to_string(), String::new())),
    }
}

fn shell_open(verb: PCWSTR, file: &str, params: &str, what: &str) -> CmdResult<()> {
    let file_w = HSTRING::from(file);
    let params_w = HSTRING::from(params);
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        // Errors come back to us as a code; Windows shows no message box of its own.
        fMask: SEE_MASK_FLAG_NO_UI,
        lpVerb: verb,
        lpFile: PCWSTR(file_w.as_ptr()),
        lpParameters: PCWSTR(params_w.as_ptr()),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    if unsafe { ShellExecuteExW(&mut info) }.is_ok() {
        return Ok(());
    }
    Err(match unsafe { GetLastError().0 } {
        2 | 3 => CmdError::new("not-found", format!("Windows cannot find \"{file}\". Check the name and try again.")),
        code @ (5 | 1223) => CmdError::from_win32(code, what),
        code => CmdError::failed(format!("Windows could not start \"{file}\" (Windows error {code}).")),
    })
}

/// "Run new task": starts a program, a document or a web address, optionally as administrator.
pub fn run_task(typed: &str, as_admin: bool) -> CmdResult<()> {
    let (file, params) = split_command(typed, |p| Path::new(p).is_file())
        .ok_or_else(|| CmdError::invalid("Type the name of a program, folder or document."))?;
    shell_open(if as_admin { w!("runas") } else { w!("open") }, &file, &params, &format!("Starting {file}"))
}

/// Opens something with its normal program (a folder, a `.msc` console, a web address).
pub fn open(target: &str) -> CmdResult<()> {
    shell_open(w!("open"), target, "", &format!("Opening {target}"))
}

/// Web search address for a program name. Only letters, digits and a few safe marks are
/// passed through as they are; everything else is escaped.
pub fn search_url(query: &str) -> String {
    let mut url = String::from("https://www.google.com/search?q=");
    for byte in query.trim().bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => url.push(byte as char),
            b' ' => url.push('+'),
            other => url.push_str(&format!("%{other:02X}")),
        }
    }
    url
}

pub fn search_online(query: &str) -> CmdResult<()> {
    if query.trim().is_empty() {
        return Err(CmdError::invalid("Nothing to search for."));
    }
    open(&search_url(query))
}

/// Brings a program's window to the front.
pub fn switch_to(pid: u32) -> CmdResult<()> {
    if wins::switch_to(pid) {
        Ok(())
    } else {
        Err(CmdError::not_found("That window"))
    }
}

/// Restarts the desktop and taskbar (Windows Explorer), the fix for a stuck taskbar.
pub fn restart_explorer() -> CmdResult<()> {
    let shell = wins::shell_pid();
    if shell != 0 {
        let what = "Restarting Windows Explorer";
        let handle = procinfo::open(shell, PROCESS_TERMINATE).map_err(|code| CmdError::from_win32(code, what))?;
        unsafe { TerminateProcess(handle.0, 1).map_err(|_| last_error(what))? };
    }
    // Windows normally starts the desktop again by itself; make sure it does.
    for _ in 0..30 {
        std::thread::sleep(std::time::Duration::from_millis(100));
        let now = wins::shell_pid();
        if now != 0 && now != shell {
            return Ok(());
        }
    }
    std::process::Command::new("explorer.exe")
        .spawn()
        .map(|_| ())
        .map_err(|e| CmdError::failed(format!("The desktop did not come back by itself and could not be started: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proclist::efficiency_of;
    use std::process::{Child, Command, Stdio};

    fn sleeper() -> Child {
        Command::new("cmd")
            .args(["/c", "ping -n 60 127.0.0.1 >nul"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("cmd.exe starts")
    }

    fn end(mut child: Child) {
        let _ = procs::kill(child.id(), "cmd.exe", true);
        let _ = child.wait();
    }

    #[test]
    fn priority_is_really_changed_and_read_back() {
        let child = sleeper();
        assert_eq!(set_priority(child.id(), "cmd.exe", "below-normal").unwrap(), "below-normal");
        assert_eq!(procinfo::priority_of(child.id()), "below-normal");
        assert_eq!(set_priority(child.id(), "cmd.exe", "turbo").unwrap_err().code, "invalid");
        assert_eq!(procinfo::priority_of(child.id()), "below-normal", "a refused change changes nothing");
        end(child);
    }

    #[test]
    fn the_wrong_name_changes_nothing() {
        let child = sleeper();
        assert_eq!(set_priority(child.id(), "notepad.exe", "low").unwrap_err().code, "not-found");
        assert_eq!(procinfo::priority_of(child.id()), "normal");
        assert_eq!(set_paused(child.id(), "notepad.exe", true, 1).unwrap_err().code, "not-found");
        assert_eq!(set_efficiency(child.id(), "notepad.exe", true).unwrap_err().code, "not-found");
        assert_eq!(create_dump(child.id(), "notepad.exe").unwrap_err().code, "not-found");
        end(child);
    }

    #[test]
    fn affinity_is_limited_to_real_processors() {
        let child = sleeper();
        let (_, system) = procinfo::affinity_of(child.id()).unwrap();
        assert_eq!(set_affinity(child.id(), "cmd.exe", 1).unwrap(), 1);
        assert_eq!(procinfo::affinity_of(child.id()).unwrap().0, 1);
        assert_eq!(set_affinity(child.id(), "cmd.exe", 0).unwrap_err().code, "invalid");
        assert_eq!(procinfo::affinity_of(child.id()).unwrap().0, 1, "a refused change changes nothing");
        assert_eq!(set_affinity(child.id(), "cmd.exe", u64::MAX).unwrap(), system, "extra bits are dropped");
        end(child);
    }

    #[test]
    fn efficiency_mode_goes_on_and_off() {
        let child = sleeper();
        assert_eq!(efficiency_of(child.id()), Some(false));
        set_efficiency(child.id(), "cmd.exe", true).unwrap();
        assert_eq!(efficiency_of(child.id()), Some(true));
        assert_eq!(procinfo::priority_of(child.id()), "low");
        set_efficiency(child.id(), "cmd.exe", false).unwrap();
        assert_eq!(efficiency_of(child.id()), Some(false));
        assert_eq!(procinfo::priority_of(child.id()), "normal");
        end(child);
    }

    #[test]
    fn pause_freezes_and_continue_unfreezes() {
        let child = sleeper();
        std::thread::sleep(std::time::Duration::from_millis(300));
        let frozen = |pid: u32| procs::list().iter().find(|p| p.pid == pid).map(|p| p.suspended);
        assert_eq!(frozen(child.id()), Some(false));
        set_paused(child.id(), "cmd.exe", true, std::process::id()).unwrap();
        assert_eq!(frozen(child.id()), Some(true));
        set_paused(child.id(), "cmd.exe", false, std::process::id()).unwrap();
        assert_eq!(frozen(child.id()), Some(false));
        end(child);
    }

    #[test]
    fn pausing_windows_or_ourselves_is_refused() {
        assert_eq!(set_paused(4, "System", true, 1).unwrap_err().code, "protected");
        let me = std::process::id();
        let my_name = procs::list().into_iter().find(|p| p.pid == me).unwrap().name;
        assert_eq!(set_paused(me, &my_name, true, me).unwrap_err().code, "protected");
        assert_eq!(set_efficiency(4, "System", true).unwrap_err().code, "protected");
    }

    #[test]
    fn a_dump_file_is_written_and_named_after_the_program() {
        let child = sleeper();
        let path = create_dump(child.id(), "cmd.exe").unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert!(path.ends_with(&format!("cmd ({}).DMP", child.id())), "{path}");
        assert_eq!(&bytes[..4], b"MDMP", "a real Windows dump file");
        assert!(bytes.len() > 100_000);
        end(child);
    }

    #[test]
    fn dump_names_never_overwrite_and_never_leave_the_folder() {
        let folder = Path::new(r"C:\t");
        assert_eq!(dump_path(folder, "chrome.exe", 7, |_| false), folder.join("chrome (7).DMP"));
        let taken = |p: &Path| p.ends_with("chrome (7).DMP") || p.ends_with("chrome (7) 2.DMP");
        assert_eq!(dump_path(folder, "chrome.exe", 7, taken), folder.join("chrome (7) 3.DMP"));
        assert_eq!(dump_path(folder, r"..\..\evil.exe", 7, |_| false), folder.join(".._.._evil (7).DMP"));
        assert_eq!(dump_path(folder, "", 7, |_| false), folder.join("process (7).DMP"));
    }

    #[test]
    fn typed_commands_are_split_into_program_and_arguments() {
        let none = |_: &str| false;
        assert_eq!(split_command("notepad", none), Some(("notepad".into(), "".into())));
        assert_eq!(split_command("  cmd /k dir  ", none), Some(("cmd".into(), "/k dir".into())));
        assert_eq!(split_command(r#""C:\My App\a.exe" -x 1"#, none), Some((r"C:\My App\a.exe".into(), "-x 1".into())));
        let real = |p: &str| p == r"C:\Program Files\App\app.exe";
        assert_eq!(split_command(r"C:\Program Files\App\app.exe", real), Some((r"C:\Program Files\App\app.exe".into(), "".into())));
        assert_eq!(
            split_command(r"C:\Program Files\App\app.exe --fast now", real),
            Some((r"C:\Program Files\App\app.exe".into(), "--fast now".into()))
        );
        assert_eq!(split_command("   ", none), None);
        assert_eq!(split_command(r#""unclosed"#, none), None);
        assert_eq!(split_command(r#""" x"#, none), None);
    }

    #[test]
    fn search_addresses_escape_everything_unusual() {
        assert_eq!(search_url("chrome.exe"), "https://www.google.com/search?q=chrome.exe");
        assert_eq!(search_url(" My App&x=1 "), "https://www.google.com/search?q=My+App%26x%3D1");
        assert_eq!(search_url("я"), "https://www.google.com/search?q=%D1%8F");
        assert_eq!(search_online("  ").unwrap_err().code, "invalid");
    }

    #[test]
    fn properties_of_a_missing_file_is_an_error() {
        assert_eq!(show_properties("").unwrap_err().code, "failed");
        assert_eq!(show_properties(r"C:\no\such\file.exe").unwrap_err().code, "failed");
    }

    #[test]
    fn running_nothing_or_a_missing_program_is_an_error() {
        assert_eq!(run_task("  ", false).unwrap_err().code, "invalid");
        assert_eq!(run_task("this-program-does-not-exist-42.exe", false).unwrap_err().code, "not-found");
    }
}
