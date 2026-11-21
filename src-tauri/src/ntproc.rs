//! The whole process list in one Windows call.
//!
//! This is the call Windows Task Manager itself makes. One request returns every process with
//! its threads, handles, memory, CPU time, I/O totals, priority and session; nothing is opened
//! per process, so a refresh costs a few milliseconds however many programs are running.

use std::ffi::c_void;

#[link(name = "ntdll")]
extern "system" {
    fn NtQuerySystemInformation(class: u32, info: *mut c_void, len: u32, returned: *mut u32) -> i32;
}

const SYSTEM_PROCESS_INFORMATION: u32 = 5;
const STATUS_INFO_LENGTH_MISMATCH: i32 = 0xC000_0004_u32 as i32;
/// Seconds between 1601-01-01 (Windows time) and 1970-01-01 (Unix time), in 100 ns steps.
const EPOCH_GAP: u64 = 116_444_736_000_000_000;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct NtProc {
    pub pid: u32,
    pub ppid: u32,
    pub name: String,
    pub threads: u32,
    pub handles: u32,
    pub session: u32,
    pub base_priority: i32,
    /// Windows time (100 ns steps since 1601). Together with the PID it identifies a process
    /// for good, even after the PID is reused.
    pub create_time: u64,
    /// User + kernel CPU time, in 100 ns steps.
    pub cpu_time: u64,
    /// Memory only this process uses: the "Memory" figure Windows Task Manager shows.
    pub private_ws: u64,
    pub ws: u64,
    pub peak_ws: u64,
    pub commit: u64,
    pub page_faults: u32,
    pub read_bytes: u64,
    pub write_bytes: u64,
    pub other_bytes: u64,
    /// Every thread is frozen (a paused program, or a Store app Windows put to sleep).
    pub suspended: bool,
}

impl NtProc {
    /// Start time as Unix milliseconds (0 when Windows gives none, as for "System").
    pub fn start_unix_ms(&self) -> u64 {
        self.create_time.saturating_sub(EPOCH_GAP) / 10_000
    }
    pub fn cpu_time_ms(&self) -> u64 {
        self.cpu_time / 10_000
    }
}

// Offsets inside one SYSTEM_PROCESS_INFORMATION record on 64-bit Windows.
const O_NEXT: usize = 0;
const O_THREADS: usize = 4;
const O_PRIVATE_WS: usize = 8;
const O_CREATE: usize = 32;
const O_USER: usize = 40;
const O_KERNEL: usize = 48;
const O_NAME_LEN: usize = 56;
const O_NAME_PTR: usize = 64;
const O_PRIORITY: usize = 72;
const O_PID: usize = 80;
const O_PPID: usize = 88;
const O_HANDLES: usize = 96;
const O_SESSION: usize = 100;
const O_FAULTS: usize = 128;
const O_PEAK_WS: usize = 136;
const O_WS: usize = 144;
const O_COMMIT: usize = 200;
const O_READ: usize = 232;
const O_WRITE: usize = 240;
const O_OTHER: usize = 248;
pub const RECORD: usize = 256;
// One SYSTEM_THREAD_INFORMATION record follows per thread.
pub const THREAD: usize = 80;
const T_STATE: usize = 68;
const T_WAIT_REASON: usize = 72;
const WAITING: u32 = 5;
const SUSPENDED: u32 = 5;

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}
fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}
fn u64_at(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(at..at + 8)?.try_into().ok()?))
}

/// Reads the records out of the raw answer. `base` is the memory address the buffer was
/// filled at: names are stored as addresses into the same buffer.
///
/// Never trusts a length or address in the data: anything pointing outside the buffer ends
/// the walk or leaves that field empty instead of reading stray memory.
pub fn parse(buf: &[u8], base: usize) -> Vec<NtProc> {
    let mut out = Vec::new();
    let mut at = 0usize;
    loop {
        let Some(rec) = buf.get(at..at + RECORD) else { break };
        let field = |o: usize| u64_at(rec, o).unwrap_or(0);
        let pid = field(O_PID) as u32;
        let threads = u32_at(rec, O_THREADS).unwrap_or(0);

        let name_len = u16_at(rec, O_NAME_LEN).unwrap_or(0) as usize;
        let name_ptr = field(O_NAME_PTR) as usize;
        let mut name = name_ptr
            .checked_sub(base)
            .and_then(|start| buf.get(start..start.checked_add(name_len)?))
            .map(|bytes| {
                let wide: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
                String::from_utf16_lossy(&wide)
            })
            .unwrap_or_default();
        if name.is_empty() {
            name = match pid {
                0 => "System Idle Process".into(),
                4 => "System".into(),
                _ => format!("Process {pid}"),
            };
        }

        let mut suspended = threads > 0;
        for i in 0..threads as usize {
            let t = at + RECORD + i * THREAD;
            match (u32_at(buf, t + T_STATE), u32_at(buf, t + T_WAIT_REASON)) {
                (Some(WAITING), Some(SUSPENDED)) => {}
                _ => {
                    suspended = false;
                    break;
                }
            }
        }

        out.push(NtProc {
            pid,
            ppid: field(O_PPID) as u32,
            name,
            threads,
            handles: u32_at(rec, O_HANDLES).unwrap_or(0),
            session: u32_at(rec, O_SESSION).unwrap_or(0),
            base_priority: u32_at(rec, O_PRIORITY).unwrap_or(0) as i32,
            create_time: field(O_CREATE),
            cpu_time: field(O_USER).saturating_add(field(O_KERNEL)),
            private_ws: field(O_PRIVATE_WS),
            ws: field(O_WS),
            peak_ws: field(O_PEAK_WS),
            commit: field(O_COMMIT),
            page_faults: u32_at(rec, O_FAULTS).unwrap_or(0),
            read_bytes: field(O_READ),
            write_bytes: field(O_WRITE),
            other_bytes: field(O_OTHER),
            suspended,
        });

        let next = u32_at(rec, O_NEXT).unwrap_or(0) as usize;
        if next == 0 {
            break;
        }
        // A record that does not move forward by at least its own size is damaged data.
        if next < RECORD {
            break;
        }
        at += next;
    }
    out
}

/// Asks Windows for every process. `buf` is kept by the caller and reused between calls.
pub fn query(buf: &mut Vec<u64>) -> Vec<NtProc> {
    if buf.is_empty() {
        buf.resize(64 * 1024, 0); // 512 KB to start with
    }
    for _ in 0..6 {
        let mut needed = 0u32;
        let bytes = (buf.len() * 8) as u32;
        let status =
            unsafe { NtQuerySystemInformation(SYSTEM_PROCESS_INFORMATION, buf.as_mut_ptr() as *mut c_void, bytes, &mut needed) };
        if status == STATUS_INFO_LENGTH_MISMATCH {
            // The list grew: make room with some to spare and ask again.
            buf.resize((needed as usize + 128 * 1024).div_ceil(8), 0);
            continue;
        }
        if status < 0 {
            return Vec::new();
        }
        let filled = (needed as usize).min(buf.len() * 8);
        let slice = unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const u8, filled) };
        return parse(slice, buf.as_ptr() as usize);
    }
    Vec::new()
}

/// Share of the whole PC used between two readings, 0–100.
pub fn cpu_percent(previous: u64, current: u64, elapsed_secs: f64, cores: f32) -> f32 {
    if elapsed_secs <= 0.0 || cores <= 0.0 {
        return 0.0;
    }
    let used_secs = current.saturating_sub(previous) as f64 / 10_000_000.0;
    ((used_secs / elapsed_secs / cores as f64) * 100.0).clamp(0.0, 100.0) as f32
}

/// Bytes per second between two readings of a running total.
pub fn rate(previous: u64, current: u64, elapsed_secs: f64) -> f64 {
    if elapsed_secs <= 0.0 {
        return 0.0;
    }
    current.saturating_sub(previous) as f64 / elapsed_secs
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a fake answer the way Windows lays it out.
    struct Fake {
        buf: Vec<u8>,
        base: usize,
        last: Option<usize>,
    }

    impl Fake {
        fn new() -> Fake {
            Fake { buf: Vec::new(), base: 0x7000_0000, last: None }
        }
        fn put(&mut self, at: usize, v: u64, bytes: usize) {
            self.buf[at..at + bytes].copy_from_slice(&v.to_le_bytes()[..bytes]);
        }
        /// `threads` is a list of (state, wait reason).
        fn add(&mut self, pid: u32, ppid: u32, name: &str, threads: &[(u32, u32)]) -> usize {
            let start = self.buf.len();
            if let Some(prev) = self.last {
                self.put(prev + O_NEXT, (start - prev) as u64, 4);
            }
            let wide: Vec<u8> = name.encode_utf16().flat_map(|c| c.to_le_bytes()).collect();
            let name_at = start + RECORD + threads.len() * THREAD;
            let total = (name_at + wide.len()).div_ceil(8) * 8;
            self.buf.resize(total, 0);
            self.put(start + O_THREADS, threads.len() as u64, 4);
            self.put(start + O_PID, pid as u64, 8);
            self.put(start + O_PPID, ppid as u64, 8);
            self.put(start + O_NAME_LEN, wide.len() as u64, 2);
            self.put(start + O_NAME_PTR, (self.base + name_at) as u64, 8);
            self.buf[name_at..name_at + wide.len()].copy_from_slice(&wide);
            for (i, (state, reason)) in threads.iter().enumerate() {
                let t = start + RECORD + i * THREAD;
                self.put(t + T_STATE, *state as u64, 4);
                self.put(t + T_WAIT_REASON, *reason as u64, 4);
            }
            self.last = Some(start);
            start
        }
    }

    #[test]
    fn reads_every_record_and_field() {
        let mut f = Fake::new();
        f.add(0, 0, "", &[(2, 0)]);
        f.add(4, 0, "System", &[(5, 0), (5, 5)]);
        let at = f.add(1234, 4, "chrome.exe", &[(5, 5), (5, 5)]);
        f.put(at + O_PRIVATE_WS, 111, 8);
        f.put(at + O_WS, 222, 8);
        f.put(at + O_COMMIT, 333, 8);
        f.put(at + O_USER, 40, 8);
        f.put(at + O_KERNEL, 2, 8);
        f.put(at + O_HANDLES, 77, 4);
        f.put(at + O_SESSION, 1, 4);
        f.put(at + O_PRIORITY, 8, 4);
        f.put(at + O_READ, 1000, 8);
        f.put(at + O_WRITE, 2000, 8);
        f.put(at + O_CREATE, EPOCH_GAP + 50_000_000, 8);

        let procs = parse(&f.buf, f.base);
        assert_eq!(procs.len(), 3);
        assert_eq!(procs[0].name, "System Idle Process");
        assert_eq!(procs[1].name, "System");
        assert!(!procs[1].suspended, "one running thread means not suspended");
        let c = &procs[2];
        assert_eq!((c.pid, c.ppid, c.name.as_str()), (1234, 4, "chrome.exe"));
        assert_eq!((c.private_ws, c.ws, c.commit), (111, 222, 333));
        assert_eq!((c.cpu_time, c.handles, c.session, c.base_priority, c.threads), (42, 77, 1, 8, 2));
        assert_eq!((c.read_bytes, c.write_bytes), (1000, 2000));
        assert_eq!(c.start_unix_ms(), 5000);
        assert!(c.suspended, "every thread frozen means suspended");
    }

    #[test]
    fn cut_off_buffer_gives_what_is_complete_and_never_panics() {
        let mut f = Fake::new();
        f.add(8, 4, "a.exe", &[]);
        f.add(12, 4, "b.exe", &[]);
        let full = parse(&f.buf, f.base).len();
        assert_eq!(full, 2);
        for cut in 0..f.buf.len() {
            let got = parse(&f.buf[..cut], f.base);
            assert!(got.len() <= full);
        }
    }

    #[test]
    fn a_name_pointing_outside_the_buffer_is_not_followed() {
        let mut f = Fake::new();
        let at = f.add(8, 4, "a.exe", &[]);
        f.put(at + O_NAME_PTR, 0x1234, 8); // below the buffer
        assert_eq!(parse(&f.buf, f.base)[0].name, "Process 8");
        f.put(at + O_NAME_PTR, (f.base + f.buf.len() + 4096) as u64, 8); // past the end
        assert_eq!(parse(&f.buf, f.base)[0].name, "Process 8");
    }

    #[test]
    fn a_record_that_points_at_itself_ends_the_walk() {
        let mut f = Fake::new();
        let at = f.add(8, 4, "a.exe", &[]);
        f.put(at + O_NEXT, 8, 4); // would loop forever if followed
        assert_eq!(parse(&f.buf, f.base).len(), 1);
    }

    #[test]
    fn empty_input_is_empty() {
        assert!(parse(&[], 0).is_empty());
    }

    #[test]
    fn cpu_share_between_two_readings() {
        // 1 s of CPU in 1 s on a 4-core PC is one core, 25%.
        assert_eq!(cpu_percent(0, 10_000_000, 1.0, 4.0), 25.0);
        assert_eq!(cpu_percent(0, 80_000_000, 1.0, 4.0), 100.0, "never above 100");
        assert_eq!(cpu_percent(50, 40, 1.0, 4.0), 0.0, "a counter that went backwards is not negative use");
        assert_eq!(cpu_percent(0, 10, 0.0, 4.0), 0.0, "no time passed");
    }

    #[test]
    fn rate_between_two_readings() {
        assert_eq!(rate(1000, 3000, 2.0), 1000.0);
        assert_eq!(rate(3000, 1000, 2.0), 0.0);
        assert_eq!(rate(0, 10, 0.0), 0.0);
    }

    #[test]
    fn real_list_contains_this_test_and_system() {
        let mut buf = Vec::new();
        let procs = query(&mut buf);
        assert!(procs.len() > 20, "got {}", procs.len());
        let me = procs.iter().find(|p| p.pid == std::process::id()).expect("this test process is listed");
        assert!(me.name.to_lowercase().ends_with(".exe"));
        assert!(me.threads >= 1 && me.handles > 0 && me.private_ws > 0 && me.ws >= me.private_ws);
        assert!(me.start_unix_ms() > 1_600_000_000_000);
        assert!(!me.suspended);
        let system = procs.iter().find(|p| p.pid == 4).expect("System is listed");
        assert_eq!(system.name, "System");
        // PIDs are unique within one answer.
        let mut pids: Vec<u32> = procs.iter().map(|p| p.pid).collect();
        pids.sort_unstable();
        pids.dedup();
        assert_eq!(pids.len(), procs.len());
        // A second call reuses the buffer.
        assert!(query(&mut buf).len() > 20);
    }
}
