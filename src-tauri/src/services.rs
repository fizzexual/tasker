//! Windows services: list them, start, stop and restart.

use crate::error::{CmdError, CmdResult};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use windows::core::PCWSTR;
use windows::Win32::Foundation::GetLastError;
use windows::Win32::System::Services::{
    CloseServiceHandle, ControlService, EnumServicesStatusExW, OpenSCManagerW, OpenServiceW, QueryServiceConfig2W,
    QueryServiceConfigW, QueryServiceStatus, StartServiceW, ENUM_SERVICE_STATUS_PROCESSW, QUERY_SERVICE_CONFIGW,
    SC_ENUM_PROCESS_INFO, SC_HANDLE, SC_MANAGER_CONNECT, SC_MANAGER_ENUMERATE_SERVICE, SERVICE_CONFIG_DESCRIPTION,
    SERVICE_CONTROL_STOP, SERVICE_DESCRIPTIONW, SERVICE_QUERY_CONFIG, SERVICE_QUERY_STATUS, SERVICE_START,
    SERVICE_STATE_ALL, SERVICE_STATUS, SERVICE_STOP, SERVICE_WIN32,
};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ServiceInfo {
    pub name: String,
    pub display: String,
    /// `running`, `stopped`, `starting`, `stopping`, `paused`, `other`.
    pub status: &'static str,
    /// `automatic`, `manual`, `disabled`, `system`, or empty when Windows will not say.
    pub start: &'static str,
    pub pid: u32,
    pub description: String,
    /// The shared host group it runs in (`netsvcs`…), empty when it has its own program.
    pub group: String,
}

struct Sc(SC_HANDLE);
impl Drop for Sc {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseServiceHandle(self.0);
        }
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn last_error(what: &str) -> CmdError {
    CmdError::from_win32(unsafe { GetLastError().0 }, what)
}

pub fn status_name(state: u32) -> &'static str {
    match state {
        1 => "stopped",
        2 => "starting",
        3 => "stopping",
        4 => "running",
        5..=7 => "paused",
        _ => "other",
    }
}

pub fn start_name(start_type: u32) -> &'static str {
    match start_type {
        0 | 1 => "system",
        2 => "automatic",
        3 => "manual",
        4 => "disabled",
        _ => "",
    }
}

fn manager() -> CmdResult<Sc> {
    unsafe {
        OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_CONNECT | SC_MANAGER_ENUMERATE_SERVICE)
            .map(Sc)
            .map_err(|_| last_error("Reading the services list"))
    }
}

/// Start type and description rarely change, so they are read once per service and remembered.
fn config_cache() -> &'static Mutex<HashMap<String, (&'static str, String, String)>> {
    static CACHE: OnceLock<Mutex<HashMap<String, (&'static str, String, String)>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The group after `-k` in a shared service host's command line.
pub fn parse_group(command: &str) -> String {
    let mut words = command.split_whitespace();
    while let Some(word) = words.next() {
        if word.eq_ignore_ascii_case("-k") || word.eq_ignore_ascii_case("/k") {
            return words.next().unwrap_or("").trim_matches('"').to_string();
        }
    }
    String::new()
}

fn read_config(scm: &Sc, name: &str) -> (&'static str, String, String) {
    let name_w = wide(name);
    unsafe {
        let Ok(handle) = OpenServiceW(scm.0, PCWSTR(name_w.as_ptr()), SERVICE_QUERY_CONFIG) else {
            return ("", String::new(), String::new());
        };
        let svc = Sc(handle);

        let mut start = "";
        let mut group = String::new();
        let mut needed = 0u32;
        let _ = QueryServiceConfigW(svc.0, None, 0, &mut needed);
        if needed > 0 {
            let mut buf = vec![0u64; (needed as usize).div_ceil(8)];
            let cfg = buf.as_mut_ptr() as *mut QUERY_SERVICE_CONFIGW;
            if QueryServiceConfigW(svc.0, Some(cfg), needed, &mut needed).is_ok() {
                start = start_name((*cfg).dwStartType.0);
                if !(*cfg).lpBinaryPathName.is_null() {
                    group = parse_group(&(*cfg).lpBinaryPathName.to_string().unwrap_or_default());
                }
            }
        }

        let mut description = String::new();
        let mut needed = 0u32;
        let _ = QueryServiceConfig2W(svc.0, SERVICE_CONFIG_DESCRIPTION, None, &mut needed);
        if needed > 0 {
            let mut buf = vec![0u64; (needed as usize).div_ceil(8)];
            let bytes = std::slice::from_raw_parts_mut(buf.as_mut_ptr() as *mut u8, needed as usize);
            if QueryServiceConfig2W(svc.0, SERVICE_CONFIG_DESCRIPTION, Some(bytes), &mut needed).is_ok() {
                let d = &*(buf.as_ptr() as *const SERVICE_DESCRIPTIONW);
                if !d.lpDescription.is_null() {
                    description = d.lpDescription.to_string().unwrap_or_default();
                }
            }
        }
        (start, description, group)
    }
}

/// Lists every Windows service. `fresh` re-reads start type and description too.
pub fn list(fresh: bool) -> CmdResult<Vec<ServiceInfo>> {
    let scm = manager()?;
    let mut needed = 0u32;
    let mut count = 0u32;
    let mut buf: Vec<u64> = Vec::new();

    // Ask for the size, then read; retry once if services were added in between.
    for _ in 0..3 {
        let mut resume = 0u32;
        let bytes = unsafe { std::slice::from_raw_parts_mut(buf.as_mut_ptr() as *mut u8, buf.len() * 8) };
        let result = unsafe {
            EnumServicesStatusExW(
                scm.0,
                SC_ENUM_PROCESS_INFO,
                SERVICE_WIN32,
                SERVICE_STATE_ALL,
                (!bytes.is_empty()).then_some(bytes),
                &mut needed,
                &mut count,
                Some(&mut resume),
                PCWSTR::null(),
            )
        };
        match result {
            Ok(()) => break,
            Err(_) if unsafe { GetLastError().0 } == 234 => {
                // ERROR_MORE_DATA: grow and try again.
                buf = vec![0u64; (buf.len() * 8 + needed as usize + 4096).div_ceil(8)];
                count = 0;
            }
            Err(_) => return Err(last_error("Reading the services list")),
        }
    }

    let items =
        unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const ENUM_SERVICE_STATUS_PROCESSW, count as usize) };
    let mut cache = config_cache().lock().map_err(|_| CmdError::failed("Internal error reading services."))?;
    if fresh {
        cache.clear();
    }
    let mut out: Vec<ServiceInfo> = items
        .iter()
        .map(|item| {
            let name = unsafe { item.lpServiceName.to_string().unwrap_or_default() };
            let display = unsafe { item.lpDisplayName.to_string().unwrap_or_default() };
            let (start, description, group) =
                cache.entry(name.clone()).or_insert_with(|| read_config(&scm, &name)).clone();
            ServiceInfo {
                status: status_name(item.ServiceStatusProcess.dwCurrentState.0),
                pid: item.ServiceStatusProcess.dwProcessId,
                name,
                display,
                start,
                description,
                group,
            }
        })
        .collect();
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(out)
}

fn open(scm: &Sc, name: &str, access: u32, what: &str) -> CmdResult<Sc> {
    let name_w = wide(name);
    unsafe { OpenServiceW(scm.0, PCWSTR(name_w.as_ptr()), access).map(Sc).map_err(|_| last_error(what)) }
}

fn state(svc: &Sc) -> Option<u32> {
    let mut status = SERVICE_STATUS::default();
    unsafe { QueryServiceStatus(svc.0, &mut status).ok().map(|_| status.dwCurrentState.0) }
}

/// Waits until the service reaches `want` (1 = stopped, 4 = running).
fn wait_for(svc: &Sc, want: u32, limit: Duration) -> bool {
    let started = Instant::now();
    loop {
        if state(svc) == Some(want) {
            return true;
        }
        if started.elapsed() > limit {
            return false;
        }
        std::thread::sleep(Duration::from_millis(150));
    }
}

fn start(scm: &Sc, name: &str) -> CmdResult<()> {
    let what = format!("Starting {name}");
    let svc = open(scm, name, SERVICE_START | SERVICE_QUERY_STATUS, &what)?;
    unsafe { StartServiceW(svc.0, None).map_err(|_| last_error(&what))? };
    if !wait_for(&svc, 4, Duration::from_secs(20)) {
        return Err(CmdError::failed(format!("{name} did not finish starting within 20 seconds.")));
    }
    Ok(())
}

fn stop(scm: &Sc, name: &str) -> CmdResult<()> {
    let what = format!("Stopping {name}");
    let svc = open(scm, name, SERVICE_STOP | SERVICE_QUERY_STATUS, &what)?;
    let mut status = SERVICE_STATUS::default();
    unsafe { ControlService(svc.0, SERVICE_CONTROL_STOP, &mut status).map_err(|_| last_error(&what))? };
    if !wait_for(&svc, 1, Duration::from_secs(20)) {
        return Err(CmdError::failed(format!("{name} did not stop within 20 seconds.")));
    }
    Ok(())
}

/// `action` is `start`, `stop` or `restart`. Blocks until the service reached the new state.
pub fn act(name: &str, action: &str) -> CmdResult<()> {
    if name.is_empty() || name.len() > 256 || name.contains(['\\', '/']) {
        return Err(CmdError::invalid("Unknown service."));
    }
    let scm = unsafe {
        OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_CONNECT)
            .map(Sc)
            .map_err(|_| last_error("Opening the services manager"))?
    };
    match action {
        "start" => start(&scm, name),
        "stop" => stop(&scm, name),
        "restart" => {
            match stop(&scm, name) {
                Ok(()) => {}
                // Already stopped: restart then simply means start.
                Err(e) if e.message.contains("not running") => {}
                Err(e) => return Err(e),
            }
            start(&scm, name)
        }
        _ => Err(CmdError::invalid("Unknown action.")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_and_start_type_names() {
        assert_eq!(status_name(4), "running");
        assert_eq!(status_name(1), "stopped");
        assert_eq!(status_name(2), "starting");
        assert_eq!(status_name(3), "stopping");
        assert_eq!(status_name(7), "paused");
        assert_eq!(status_name(99), "other");
        assert_eq!(start_name(2), "automatic");
        assert_eq!(start_name(3), "manual");
        assert_eq!(start_name(4), "disabled");
        assert_eq!(start_name(77), "");
    }

    #[test]
    fn lists_real_services_with_known_ones_present() {
        let services = list(true).unwrap();
        assert!(services.len() > 50, "Windows has well over 50 services, got {}", services.len());
        // The Windows Event Log service exists and runs on every Windows.
        let log = services.iter().find(|s| s.name.eq_ignore_ascii_case("EventLog")).expect("EventLog is listed");
        assert_eq!(log.status, "running");
        assert!(log.pid > 0);
        assert_eq!(log.start, "automatic");
        assert!(!log.display.is_empty());
        let mut sorted = services.clone();
        sorted.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        assert_eq!(services, sorted, "sorted by name");
    }

    #[test]
    fn second_listing_uses_the_remembered_config_and_matches() {
        let a = list(true).unwrap();
        let b = list(false).unwrap();
        let find = |l: &[ServiceInfo]| l.iter().find(|s| s.name.eq_ignore_ascii_case("EventLog")).cloned();
        assert_eq!(find(&a).map(|s| s.start), find(&b).map(|s| s.start));
    }

    #[test]
    fn unknown_service_is_not_found() {
        let err = act("TaskerNoSuchService_1a2b3c", "start").unwrap_err();
        assert_eq!(err.code, "not-found");
    }

    #[test]
    fn bad_input_is_refused_without_calling_windows() {
        assert_eq!(act("", "start").unwrap_err().code, "invalid");
        assert_eq!(act(r"a\b", "start").unwrap_err().code, "invalid");
        assert_eq!(act("EventLog", "explode").unwrap_err().code, "invalid");
    }

    #[test]
    fn stopping_a_system_service_without_admin_says_needs_admin() {
        // Only meaningful when the tests run as a normal user; skipped when elevated.
        if crate::elevate::is_elevated() {
            return;
        }
        let err = act("EventLog", "stop").unwrap_err();
        assert_eq!(err.code, "needs-admin", "{err:?}");
    }

    #[test]
    fn shared_host_group_is_read_from_the_command() {
        assert_eq!(parse_group(r"C:\Windows\system32\svchost.exe -k netsvcs -p"), "netsvcs");
        assert_eq!(parse_group(r"C:\Windows\System32\svchost.exe -K LocalService"), "LocalService");
        assert_eq!(parse_group(r#""C:\Program Files\App\svc.exe" --run"#), "");
        assert_eq!(parse_group("svchost.exe -k"), "");
        let all = list(false).unwrap();
        assert!(all.iter().any(|s| !s.group.is_empty()), "this PC has services in shared hosts");
    }
}
