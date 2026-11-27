//! Builds the rows of the Processes screen from one Windows call per refresh.
//!
//! Per refresh: one process-list call, one pass over the open windows, and a lookup of the
//! program file only for processes that are new since the last refresh.

use crate::ntproc::{self, NtProc};
use crate::{procinfo, tree, wins};
use serde::ser::{Serialize, SerializeSeq, Serializer};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use windows::Win32::System::Threading::{
    GetProcessInformation, ProcessPowerThrottling, PROCESS_POWER_THROTTLING_STATE, PROCESS_QUERY_LIMITED_INFORMATION,
};

/// Has an open window: shown under "Apps".
pub const F_APP: u32 = 1;
/// Its window has stopped answering.
pub const F_HUNG: u32 = 2;
/// Every thread is frozen.
pub const F_SUSPENDED: u32 = 4;
/// Part of Windows itself.
pub const F_WINDOWS: u32 = 8;
/// Running in efficiency mode.
pub const F_ECO: u32 = 16;

const PROCESS_POWER_THROTTLING_EXECUTION_SPEED: u32 = 1;
/// How often the slow facts (efficiency mode) are re-read for every process.
const SLOW_EVERY: Duration = Duration::from_secs(10);

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProcRow {
    pub pid: u32,
    pub ppid: u32,
    pub name: String,
    /// Share of the whole PC, 0–100.
    pub cpu: f32,
    /// Memory only this process uses (what Windows Task Manager calls "Memory").
    pub mem: u64,
    /// Bytes per second.
    pub disk_read: f64,
    pub disk_write: f64,
    pub gpu: f32,
    pub threads: u32,
    pub handles: u32,
    pub session: u32,
    pub priority: i32,
    pub cpu_time_ms: u64,
    /// Exact Windows creation time, in 100 ns units since 1601.
    pub creation_time: u64,
    pub ws: u64,
    pub commit: u64,
    pub flags: u32,
    /// Title of its top window, empty when it has none.
    pub title: String,
}

/// Sent as a compact array to keep each refresh small:
/// `[pid, ppid, name, cpu, mem, diskRead, diskWrite, gpu, threads, handles, session, priority,
///   cpuTimeMs, creationTime, workingSet, commit, flags, title]`.
impl Serialize for ProcRow {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(18))?;
        seq.serialize_element(&self.pid)?;
        seq.serialize_element(&self.ppid)?;
        seq.serialize_element(&self.name)?;
        seq.serialize_element(&round(self.cpu as f64, 100.0))?;
        seq.serialize_element(&self.mem)?;
        seq.serialize_element(&round(self.disk_read, 1.0))?;
        seq.serialize_element(&round(self.disk_write, 1.0))?;
        seq.serialize_element(&round(self.gpu as f64, 10.0))?;
        seq.serialize_element(&self.threads)?;
        seq.serialize_element(&self.handles)?;
        seq.serialize_element(&self.session)?;
        seq.serialize_element(&self.priority)?;
        seq.serialize_element(&self.cpu_time_ms)?;
        // A FILETIME is larger than a JavaScript safe integer.
        seq.serialize_element(&self.creation_time.to_string())?;
        seq.serialize_element(&self.ws)?;
        seq.serialize_element(&self.commit)?;
        seq.serialize_element(&self.flags)?;
        seq.serialize_element(&self.title)?;
        seq.end()
    }
}

fn round(value: f64, steps: f64) -> f64 {
    (value * steps).round() / steps
}

struct Before {
    create_time: u64,
    cpu_time: u64,
    read: u64,
    write: u64,
}

struct Known {
    create_time: u64,
    windows: bool,
    eco: bool,
}

/// Programs that are Windows itself when they run from the Windows folder.
const WINDOWS_PROGRAMS: [&str; 15] = [
    "svchost.exe",
    "services.exe",
    "lsass.exe",
    "lsaiso.exe",
    "csrss.exe",
    "smss.exe",
    "wininit.exe",
    "winlogon.exe",
    "dwm.exe",
    "sihost.exe",
    "fontdrvhost.exe",
    "taskhostw.exe",
    "explorer.exe",
    "logonui.exe",
    "ctfmon.exe",
];

/// Whether a process belongs under "Windows processes". `path` is its program file when
/// Windows reveals it; `windir` is the Windows folder in lower case with a trailing `\`.
pub fn is_windows_process(pid: u32, name: &str, path: Option<&str>, windir: &str) -> bool {
    if tree::is_protected(pid, name) {
        return true;
    }
    let known = WINDOWS_PROGRAMS.iter().any(|w| w.eq_ignore_ascii_case(name));
    match path {
        // A look-alike name outside the Windows folder is an ordinary program.
        Some(p) => known && p.to_lowercase().starts_with(windir),
        // Windows hides the file only for its own protected processes.
        None => known || matches!(name, "Secure System" | "vmmem" | "MemCompression"),
    }
}

/// Whether a process runs in efficiency mode. `None` when Windows will not say.
pub fn efficiency_of(pid: u32) -> Option<bool> {
    let handle = procinfo::open(pid, PROCESS_QUERY_LIMITED_INFORMATION).ok()?;
    let mut state = PROCESS_POWER_THROTTLING_STATE { Version: 1, ..Default::default() };
    unsafe {
        GetProcessInformation(
            handle.0,
            ProcessPowerThrottling,
            &mut state as *mut _ as *mut _,
            std::mem::size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
        )
        .ok()?;
    }
    Some(state.StateMask & PROCESS_POWER_THROTTLING_EXECUTION_SPEED != 0)
}

pub struct Engine {
    buf: Vec<u64>,
    before: HashMap<u32, Before>,
    known: HashMap<u32, Known>,
    last: Instant,
    last_slow: Option<Instant>,
    cores: f32,
    windir: String,
}

impl Engine {
    pub fn new(cores: f32) -> Engine {
        let mut windir = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into()).to_lowercase();
        if !windir.ends_with('\\') {
            windir.push('\\');
        }
        Engine {
            buf: Vec::new(),
            before: HashMap::new(),
            known: HashMap::new(),
            last: Instant::now(),
            last_slow: None,
            cores: cores.max(1.0),
            windir,
        }
    }

    /// One refresh. `gpu` is the GPU share per process id; `recheck` forces the slow facts
    /// to be read again now (after the user changed one).
    pub fn read(&mut self, gpu: &HashMap<u32, f32>, recheck: bool) -> Vec<ProcRow> {
        let elapsed = self.last.elapsed().as_secs_f64().max(0.001);
        self.last = Instant::now();
        let list = ntproc::query(&mut self.buf);
        let windows = wins::app_windows();
        let slow = recheck || self.last_slow.map(|t| t.elapsed() >= SLOW_EVERY).unwrap_or(true);
        if slow {
            self.last_slow = Some(Instant::now());
        }
        let rows = self.rows(&list, &windows, gpu, elapsed, slow);

        self.before = list
            .iter()
            .map(|p| {
                (p.pid, Before { create_time: p.create_time, cpu_time: p.cpu_time, read: p.read_bytes, write: p.write_bytes })
            })
            .collect();
        self.known.retain(|pid, k| self.before.get(pid).map(|b| b.create_time == k.create_time).unwrap_or(false));
        rows
    }

    fn rows(
        &mut self,
        list: &[NtProc],
        windows: &HashMap<u32, wins::AppWindow>,
        gpu: &HashMap<u32, f32>,
        elapsed: f64,
        slow: bool,
    ) -> Vec<ProcRow> {
        let born: HashMap<u32, u64> = list.iter().map(|p| (p.pid, p.create_time)).collect();
        let mut rows = Vec::with_capacity(list.len());
        for p in list {
            // The idle "process" is not a program; Windows Task Manager leaves it out here too.
            if p.pid == 0 {
                continue;
            }
            let before = self.before.get(&p.pid).filter(|b| b.create_time == p.create_time);
            let (cpu, disk_read, disk_write) = match before {
                Some(b) => (
                    ntproc::cpu_percent(b.cpu_time, p.cpu_time, elapsed, self.cores),
                    ntproc::rate(b.read, p.read_bytes, elapsed),
                    ntproc::rate(b.write, p.write_bytes, elapsed),
                ),
                None => (0.0, 0.0, 0.0),
            };

            let fresh = self.known.get(&p.pid).map(|k| k.create_time != p.create_time).unwrap_or(true);
            if fresh {
                let path = procinfo::exe_path(p.pid);
                let windows_process = is_windows_process(p.pid, &p.name, path.as_deref(), &self.windir);
                self.known.insert(p.pid, Known { create_time: p.create_time, windows: windows_process, eco: false });
            }
            if fresh || slow {
                let eco = efficiency_of(p.pid).unwrap_or(false);
                if let Some(k) = self.known.get_mut(&p.pid) {
                    k.eco = eco;
                }
            }
            let known = &self.known[&p.pid];

            let window = windows.get(&p.pid);
            let mut flags = 0;
            if window.is_some() {
                flags |= F_APP;
            }
            if window.map(|w| w.hung).unwrap_or(false) {
                flags |= F_HUNG;
            }
            if p.suspended {
                flags |= F_SUSPENDED;
            }
            if known.windows {
                flags |= F_WINDOWS;
            }
            if known.eco {
                flags |= F_ECO;
            }

            // A "parent" that started after its child is a reused PID, not the parent.
            let ppid = match born.get(&p.ppid) {
                Some(parent_born) if *parent_born <= p.create_time && p.ppid != p.pid => p.ppid,
                _ => 0,
            };

            rows.push(ProcRow {
                pid: p.pid,
                ppid,
                name: p.name.clone(),
                cpu,
                mem: p.private_ws,
                disk_read,
                disk_write,
                gpu: gpu.get(&p.pid).copied().unwrap_or(0.0),
                threads: p.threads,
                handles: p.handles,
                session: p.session,
                priority: p.base_priority,
                cpu_time_ms: p.cpu_time_ms(),
                creation_time: p.create_time,
                ws: p.ws,
                commit: p.commit,
                flags,
                title: window.map(|w| w.title.clone()).unwrap_or_default(),
            });
        }
        rows
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIN: &str = r"c:\windows\";

    #[test]
    fn windows_processes_are_told_apart_from_look_alikes() {
        assert!(is_windows_process(4, "System", None, WIN));
        assert!(is_windows_process(900, "svchost.exe", Some(r"C:\Windows\System32\svchost.exe"), WIN));
        assert!(is_windows_process(900, "explorer.exe", Some(r"C:\WINDOWS\explorer.exe"), WIN));
        assert!(
            !is_windows_process(900, "svchost.exe", Some(r"C:\Users\me\AppData\svchost.exe"), WIN),
            "a program only named like a Windows one is not Windows"
        );
        assert!(!is_windows_process(900, "chrome.exe", Some(r"C:\Program Files\Google\chrome.exe"), WIN));
        assert!(!is_windows_process(900, "notepad.exe", Some(r"C:\Windows\System32\notepad.exe"), WIN));
        assert!(!is_windows_process(900, "mystery.exe", None, WIN));
    }

    #[test]
    fn a_row_is_sent_as_a_compact_array() {
        let row = ProcRow { pid: 12, ppid: 4, name: "a.exe".into(), cpu: 1.23456, mem: 1000, creation_time: 134000000000000001, title: "Hi".into(), ..Default::default() };
        let json = serde_json::to_string(&row).unwrap();
        assert_eq!(json, r#"[12,4,"a.exe",1.23,1000,0.0,0.0,0.0,0,0,0,0,0,"134000000000000001",0,0,0,"Hi"]"#);
    }

    #[test]
    fn the_real_list_has_us_with_sane_numbers() {
        let mut engine = Engine::new(8.0);
        let first = engine.read(&HashMap::new(), false);
        assert!(first.iter().all(|r| r.cpu == 0.0), "no CPU share without a reading to compare with");
        // Burn a little CPU so the second reading has something to show.
        let until = Instant::now() + Duration::from_millis(300);
        let mut x = 0u64;
        while Instant::now() < until {
            x = x.wrapping_mul(31).wrapping_add(7);
        }
        std::hint::black_box(x);
        let rows = engine.read(&HashMap::new(), false);
        let me = rows.iter().find(|r| r.pid == std::process::id()).expect("the test process is listed");
        assert!(me.mem > 1 << 20 && me.ws >= me.mem, "private memory is part of the working set");
        assert!(me.cpu > 0.5, "we just used a core for 0.3 s: {}", me.cpu);
        assert!(me.threads >= 1 && me.handles > 10);
        assert_eq!(me.creation_time, procinfo::test_creation_time(me.pid));
        assert!(me.creation_time > 0 && me.cpu_time_ms > 0);
        assert_eq!(me.flags & F_WINDOWS, 0);
        assert!(rows.iter().all(|r| (0.0..=100.0).contains(&r.cpu)), "CPU is a share of the whole PC");
        assert!(rows.iter().all(|r| r.pid != 0), "the idle process is left out");
        assert!(rows.iter().any(|r| r.flags & F_WINDOWS != 0), "Windows' own processes are marked");
        assert!(rows.len() > 10);
    }

    #[test]
    fn a_parent_link_never_points_at_a_younger_process() {
        let mut engine = Engine::new(4.0);
        let list = vec![
            NtProc { pid: 10, ppid: 99, name: "old.exe".into(), create_time: 100, ..Default::default() },
            NtProc { pid: 99, ppid: 0, name: "reused.exe".into(), create_time: 500, ..Default::default() },
            NtProc { pid: 20, ppid: 99, name: "kid.exe".into(), create_time: 600, ..Default::default() },
        ];
        let rows = engine.rows(&list, &HashMap::new(), &HashMap::new(), 1.0, false);
        let by_pid = |pid: u32| rows.iter().find(|r| r.pid == pid).unwrap();
        assert_eq!(by_pid(10).ppid, 0, "PID 99 was reused after 10 started");
        assert_eq!(by_pid(20).ppid, 99);
    }

    #[test]
    fn numbers_from_a_reused_pid_are_not_compared() {
        let mut engine = Engine::new(1.0);
        engine.before.insert(10, Before { create_time: 100, cpu_time: 0, read: 0, write: 0 });
        // Same PID, different process (new start time) with a large CPU total.
        let list = vec![NtProc { pid: 10, name: "new.exe".into(), create_time: 900, cpu_time: 50_000_000, read_bytes: 1 << 30, ..Default::default() }];
        let rows = engine.rows(&list, &HashMap::new(), &HashMap::new(), 1.0, false);
        assert_eq!(rows[0].cpu, 0.0);
        assert_eq!(rows[0].disk_read, 0.0);
    }

    #[test]
    fn window_and_frozen_state_become_flags() {
        let mut engine = Engine::new(1.0);
        let list = vec![NtProc { pid: 4_000_000_001, name: "app.exe".into(), create_time: 1, suspended: true, ..Default::default() }];
        let mut windows = HashMap::new();
        windows.insert(4_000_000_001, wins::AppWindow { hwnd: 1, title: "My app".into(), hung: true, count: 1 });
        let rows = engine.rows(&list, &windows, &HashMap::new(), 1.0, false);
        assert_eq!(rows[0].flags, F_APP | F_HUNG | F_SUSPENDED);
        assert_eq!(rows[0].title, "My app");
    }

    #[test]
    fn our_own_process_is_not_in_efficiency_mode() {
        assert_eq!(efficiency_of(std::process::id()), Some(false));
        assert_eq!(efficiency_of(4_000_000_001), None);
    }
}
