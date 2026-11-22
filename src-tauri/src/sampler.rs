//! The one background thread that measures the PC.
//!
//! Why it does not lag: the thread sleeps between readings, reads the process list only while
//! the Processes tab is on screen, and sends the window nothing while it is hidden. In the tray
//! it only keeps the small performance history, so the graphs are already filled when you open it.

use crate::hardware;
use crate::pdh::{self, Counter, Luid, Query};
use crate::proclist::{self, ProcRow};
use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use sysinfo::{CpuRefreshKind, MemoryRefreshKind, Networks, RefreshKind, System};
use windows::Win32::System::ProcessStatus::{GetPerformanceInfo, PERFORMANCE_INFORMATION};
use tauri::{AppHandle, Emitter};

/// How many readings the graphs remember.
pub const HISTORY: usize = 120;
/// Shortest gap between two readings; shorter gives noisy CPU numbers.
const MIN_GAP: Duration = Duration::from_millis(250);

#[derive(Serialize, Clone, Debug, Default)]
pub struct DiskSample {
    pub name: String,
    /// Percent of time the disk was busy.
    pub active: f32,
    /// Bytes per second.
    pub read: f64,
    pub write: f64,
    /// Average time one read or write took, in milliseconds.
    pub response_ms: f32,
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct NetSample {
    pub name: String,
    /// Bytes per second.
    pub rx: f64,
    pub tx: f64,
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct GpuSample {
    pub name: String,
    pub util: f32,
    pub mem_used: u64,
    pub mem_total: u64,
    pub shared_used: u64,
    /// Degrees Celsius, when the driver reports it.
    pub temp: Option<f32>,
    /// Load per kind of work: `[("3D", 12.0), ("VideoDecode", 3.0)]`.
    pub engines: Vec<(String, f32)>,
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct PerfSample {
    /// Unix time in milliseconds.
    pub t: u64,
    pub cpu: f32,
    pub cores: Vec<f32>,
    pub mem_used: u64,
    pub mem_total: u64,
    pub swap_used: u64,
    pub swap_total: u64,
    pub disks: Vec<DiskSample>,
    pub nets: Vec<NetSample>,
    pub gpus: Vec<GpuSample>,
    pub procs: u32,
    pub threads: u32,
    pub handles: u32,
    /// Speed the processor runs at right now, MHz (0 when unknown).
    pub cpu_mhz: u32,
    /// Memory promised to programs / the most that can be promised.
    pub committed: u64,
    pub commit_limit: u64,
    /// Memory holding recently used files, given back the moment a program needs it.
    pub cached: u64,
    pub paged_pool: u64,
    pub nonpaged_pool: u64,
}

#[derive(Serialize, Clone, Debug)]
pub struct Tick {
    pub perf: PerfSample,
    /// Present only while the Processes tab is on screen.
    pub procs: Option<Vec<ProcRow>>,
    /// How long this reading took, in milliseconds (shown in Settings as proof of lightness).
    pub cost_ms: f32,
}

pub struct Control {
    /// Milliseconds between readings; 0 = paused (a reading only when asked for).
    pub interval_ms: u64,
    /// The window is on screen (not hidden to the tray, not minimised).
    pub visible: bool,
    pub want_procs: bool,
    /// Take the next reading as soon as allowed instead of waiting a full interval.
    pub force: bool,
    /// Re-read the slow per-process facts (efficiency mode) on the next reading.
    pub recheck: bool,
}

pub struct Shared {
    pub ctl: Mutex<Control>,
    pub wake: Condvar,
    pub history: Mutex<VecDeque<PerfSample>>,
}

impl Shared {
    pub fn new(interval_ms: u64) -> Arc<Shared> {
        Arc::new(Shared {
            ctl: Mutex::new(Control { interval_ms, visible: false, want_procs: true, force: false, recheck: false }),
            wake: Condvar::new(),
            history: Mutex::new(VecDeque::with_capacity(HISTORY)),
        })
    }

    /// Change what the sampler does and wake it so the change shows at once.
    pub fn update(&self, change: impl FnOnce(&mut Control)) {
        if let Ok(mut ctl) = self.ctl.lock() {
            change(&mut ctl);
            ctl.force = true;
        }
        self.wake.notify_all();
    }
}

struct GpuAdapter {
    luid: Luid,
    name: String,
    mem_total: u64,
}

struct Counters {
    query: Query,
    disk_idle: Option<Counter>,
    disk_read: Option<Counter>,
    disk_write: Option<Counter>,
    disk_response: Option<Counter>,
    cpu_speed: Option<Counter>,
    gpu_engine: Option<Counter>,
    gpu_dedicated: Option<Counter>,
    gpu_shared: Option<Counter>,
}

pub struct Sampler {
    sys: System,
    nets: Option<Networks>,
    counters: Option<Counters>,
    gpus: Vec<GpuAdapter>,
    last_perf: Instant,
    engine: proclist::Engine,
    /// GPU share per process, from the reading `perf` just took.
    gpu_by_pid: HashMap<u32, f32>,
    base_mhz: u32,
}

impl Sampler {
    pub fn new() -> Result<Sampler, String> {
        let sys = System::new_with_specifics(
            RefreshKind::nothing()
                .with_cpu(CpuRefreshKind::nothing().with_usage())
                .with_memory(MemoryRefreshKind::everything()),
        )
        .map_err(|e| format!("cannot read system information: {e}"))?;
        let cores = sys.cpus().len().max(1) as f32;

        let counters = Query::open().map(|query| {
            let c = Counters {
                disk_idle: query.add(r"\PhysicalDisk(*)\% Idle Time"),
                disk_read: query.add(r"\PhysicalDisk(*)\Disk Read Bytes/sec"),
                disk_write: query.add(r"\PhysicalDisk(*)\Disk Write Bytes/sec"),
                disk_response: query.add(r"\PhysicalDisk(*)\Avg. Disk sec/Transfer"),
                cpu_speed: query.add(r"\Processor Information(_Total)\% Processor Performance"),
                gpu_engine: query.add(r"\GPU Engine(*)\Utilization Percentage"),
                gpu_dedicated: query.add(r"\GPU Adapter Memory(*)\Dedicated Usage"),
                gpu_shared: query.add(r"\GPU Adapter Memory(*)\Shared Usage"),
                query,
            };
            c.query.collect(); // first reading; rates need two
            c
        });

        let now = Instant::now();
        Ok(Sampler {
            sys,
            nets: Networks::new_with_refreshed_list().ok(),
            counters,
            gpus: list_gpus(),
            last_perf: now,
            engine: proclist::Engine::new(cores),
            gpu_by_pid: HashMap::new(),
            base_mhz: hardware::base_mhz(),
        })
    }

    /// One reading of the whole PC. `per_process_gpu` also works out each program's GPU
    /// share (only needed while the process list is on screen).
    pub fn perf(&mut self, per_process_gpu: bool) -> PerfSample {
        let dt = self.last_perf.elapsed().as_secs_f64().max(0.001);
        self.last_perf = Instant::now();

        self.sys.refresh_cpu_usage();
        self.sys.refresh_memory();

        let mut sample = PerfSample {
            t: SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0),
            cpu: self.sys.global_cpu_usage().clamp(0.0, 100.0),
            cores: self.sys.cpus().iter().map(|c| c.usage().clamp(0.0, 100.0)).collect(),
            mem_used: self.sys.used_memory(),
            mem_total: self.sys.total_memory(),
            swap_used: self.sys.used_swap(),
            swap_total: self.sys.total_swap(),
            ..Default::default()
        };

        if let Some(nets) = self.nets.as_mut() {
            nets.refresh(true);
            let mut list: Vec<NetSample> = nets
                .iter()
                .filter(|(name, data)| {
                    data.total_received() + data.total_transmitted() > 0
                        && !name.to_string_lossy().to_lowercase().contains("loopback")
                })
                .map(|(name, data)| NetSample {
                    name: name.to_string_lossy().into_owned(),
                    rx: data.received() as f64 / dt,
                    tx: data.transmitted() as f64 / dt,
                })
                .collect();
            list.sort_by(|a, b| a.name.cmp(&b.name));
            sample.nets = list;
        }

        if let Some(c) = self.counters.as_ref() {
            c.query.collect();
            sample.disks = disks(c);
            let engines = c.gpu_engine.map(|k| c.query.values(k)).unwrap_or_default();
            sample.gpus = gpus(c, &self.gpus, &engines);
            self.gpu_by_pid = if per_process_gpu { pdh::gpu_by_pid(&engines) } else { HashMap::new() };
            if let Some(percent) = c.cpu_speed.and_then(|k| c.query.value(k)) {
                sample.cpu_mhz = (self.base_mhz as f64 * percent / 100.0).round().max(0.0) as u32;
            }
        }
        let mut info = PERFORMANCE_INFORMATION::default();
        let size = std::mem::size_of::<PERFORMANCE_INFORMATION>() as u32;
        if unsafe { GetPerformanceInfo(&mut info, size) }.is_ok() {
            // The totals come from here, not from the "Process" and "System" performance
            // counters: those two cost about 3 ms each per reading on a busy PC.
            sample.procs = info.ProcessCount;
            sample.threads = info.ThreadCount;
            sample.handles = info.HandleCount;
            let page = info.PageSize as u64;
            sample.committed = info.CommitTotal as u64 * page;
            sample.commit_limit = info.CommitLimit as u64 * page;
            sample.cached = info.SystemCache as u64 * page;
            sample.paged_pool = info.KernelPaged as u64 * page;
            sample.nonpaged_pool = info.KernelNonpaged as u64 * page;
        }
        sample
    }

    pub fn procs(&mut self, recheck: bool) -> Vec<ProcRow> {
        self.engine.read(&self.gpu_by_pid, recheck)
    }
}

fn disks(c: &Counters) -> Vec<DiskSample> {
    let Some(idle) = c.disk_idle else { return Vec::new() };
    let read = c.disk_read.map(|k| c.query.values(k)).unwrap_or_default();
    let write = c.disk_write.map(|k| c.query.values(k)).unwrap_or_default();
    let response = c.disk_response.map(|k| c.query.values(k)).unwrap_or_default();
    let find = |list: &[(String, f64)], name: &str| {
        list.iter().find(|(n, _)| n == name).map(|(_, v)| v.max(0.0)).unwrap_or(0.0)
    };
    let mut out: Vec<DiskSample> = c
        .query
        .values(idle)
        .into_iter()
        .filter(|(name, _)| name != "_Total")
        .map(|(name, idle)| DiskSample {
            active: (100.0 - idle).clamp(0.0, 100.0) as f32,
            read: find(&read, &name),
            write: find(&write, &name),
            response_ms: (find(&response, &name) * 1000.0) as f32,
            name,
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

fn gpus(c: &Counters, adapters: &[GpuAdapter], engines: &[(String, f64)]) -> Vec<GpuSample> {
    if adapters.is_empty() {
        return Vec::new();
    }
    let load = pdh::gpu_load(engines);
    let kinds = pdh::gpu_engine_kinds(engines);
    let memory = |counter: Option<Counter>| -> Vec<(Luid, u64)> {
        counter
            .map(|k| c.query.values(k))
            .unwrap_or_default()
            .into_iter()
            .filter_map(|(name, v)| Some((pdh::parse_luid(&name)?, v.max(0.0) as u64)))
            .collect()
    };
    let dedicated = memory(c.gpu_dedicated);
    let shared = memory(c.gpu_shared);
    let of = |list: &[(Luid, u64)], luid: Luid| list.iter().filter(|(l, _)| *l == luid).map(|(_, v)| *v).sum::<u64>();
    adapters
        .iter()
        .map(|a| GpuSample {
            name: a.name.clone(),
            util: load.iter().find(|(l, _)| *l == a.luid).map(|(_, v)| *v).unwrap_or(0.0),
            mem_used: of(&dedicated, a.luid),
            mem_total: a.mem_total,
            shared_used: of(&shared, a.luid),
            temp: hardware::gpu_temperature(a.luid.high, a.luid.low),
            engines: kinds.iter().filter(|(l, _, _)| *l == a.luid).map(|(_, kind, v)| (kind.clone(), *v)).collect(),
        })
        .collect()
}

/// Real graphics cards in this PC (software renderers are skipped).
fn list_gpus() -> Vec<GpuAdapter> {
    use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE};
    let mut out: Vec<GpuAdapter> = Vec::new();
    unsafe {
        let Ok(factory) = CreateDXGIFactory1::<IDXGIFactory1>() else { return out };
        let mut index = 0;
        while let Ok(adapter) = factory.EnumAdapters1(index) {
            index += 1;
            let Ok(desc) = adapter.GetDesc1() else { continue };
            if desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 {
                continue;
            }
            let luid = Luid { high: desc.AdapterLuid.HighPart, low: desc.AdapterLuid.LowPart };
            if out.iter().any(|g| g.luid == luid) {
                continue;
            }
            let len = desc.Description.iter().position(|c| *c == 0).unwrap_or(desc.Description.len());
            out.push(GpuAdapter {
                luid,
                name: String::from_utf16_lossy(&desc.Description[..len]),
                mem_total: desc.DedicatedVideoMemory as u64,
            });
        }
    }
    out
}

/// Starts the sampler thread. It lives as long as the app.
pub fn spawn(app: AppHandle, shared: Arc<Shared>) {
    let builder = std::thread::Builder::new().name("sampler".into());
    let result = builder.spawn(move || {
        let mut sampler = match Sampler::new() {
            Ok(s) => s,
            Err(e) => {
                crate::log_error(&app, &format!("sampler could not start: {e}"));
                return;
            }
        };
        let mut last = Instant::now() - Duration::from_secs(1);
        loop {
            let (visible, want_procs, recheck) = wait_until_due(&shared, last);
            last = Instant::now();

            let started = Instant::now();
            let perf = sampler.perf(visible && want_procs);
            if let Ok(mut history) = shared.history.lock() {
                if history.len() == HISTORY {
                    history.pop_front();
                }
                history.push_back(perf.clone());
            }
            if visible {
                let procs = want_procs.then(|| sampler.procs(recheck));
                let cost_ms = started.elapsed().as_secs_f32() * 1000.0;
                let _ = app.emit("tick", Tick { perf, procs, cost_ms });
            }
        }
    });
    if let Err(e) = result {
        eprintln!("could not start the sampler thread: {e}");
    }
}

/// Sleeps until the next reading is due. Returns `(visible, want_procs, recheck)` as they are now.
/// While paused (interval 0) it sleeps until something asks for a reading.
fn wait_until_due(shared: &Shared, last: Instant) -> (bool, bool, bool) {
    let Ok(mut ctl) = shared.ctl.lock() else { return (false, false, false) };
    loop {
        if ctl.interval_ms == 0 && !ctl.force {
            ctl = match shared.wake.wait(ctl) {
                Ok(guard) => guard,
                Err(_) => return (false, false, false),
            };
            continue;
        }
        let gap = if ctl.force { MIN_GAP } else { Duration::from_millis(ctl.interval_ms).max(MIN_GAP) };
        let due = last + gap;
        let now = Instant::now();
        if now >= due {
            ctl.force = false;
            let recheck = std::mem::take(&mut ctl.recheck);
            return (ctl.visible, ctl.want_procs, recheck);
        }
        ctl = match shared.wake.wait_timeout(ctl, due - now) {
            Ok((guard, _)) => guard,
            Err(_) => return (false, false, false),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn perf_reading_is_sane_on_this_pc() {
        let mut s = Sampler::new().unwrap();
        std::thread::sleep(Duration::from_millis(300));
        let p = s.perf(true);
        assert!(p.mem_total > 1 << 30, "more than 1 GB of RAM");
        assert!(p.mem_used > 0 && p.mem_used <= p.mem_total);
        assert!((0.0..=100.0).contains(&p.cpu));
        assert!(!p.cores.is_empty());
        assert!(p.procs > 10 && p.threads > p.procs, "counts come from Windows: {} procs, {} threads", p.procs, p.threads);
        assert!(p.handles > p.threads, "handles are counted too: {} handles, {} threads", p.handles, p.threads);
        assert!(!p.disks.is_empty(), "this PC has at least one disk");
        assert!(p.disks.iter().all(|d| d.name != "_Total" && (0.0..=100.0).contains(&d.active)));
        assert!(p.committed > p.mem_used / 2 && p.committed <= p.commit_limit, "{} of {}", p.committed, p.commit_limit);
        assert!(p.cached > 0 && p.paged_pool > 0 && p.nonpaged_pool > 0);
        assert!(p.cpu_mhz > 300, "current speed {} MHz", p.cpu_mhz);
        for g in &p.gpus {
            println!("{} {:?} {:?}", g.name, g.temp, g.engines);
            assert!(g.temp.map(|t| (10.0..120.0).contains(&t)).unwrap_or(true), "{:?}", g.temp);
        }
    }

    #[test]
    fn process_list_contains_ourselves_with_plausible_numbers() {
        let mut s = Sampler::new().unwrap();
        s.procs(false);
        std::thread::sleep(Duration::from_millis(300));
        let rows = s.procs(true);
        let me = std::process::id();
        let mine = rows.iter().find(|r| r.pid == me).expect("the test process is listed");
        assert!(mine.mem > 1 << 20, "uses more than 1 MB of memory");
        assert!(rows.iter().all(|r| (0.0..=100.0).contains(&r.cpu)), "CPU is a share of the whole PC, 0-100");
        assert!(rows.len() > 10);
    }

    #[test]
    fn a_paused_sampler_reads_only_when_asked() {
        let shared = Shared::new(0);
        let waiter = shared.clone();
        let (done, finished) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let result = wait_until_due(&waiter, Instant::now() - Duration::from_secs(5));
            let _ = done.send(result);
        });
        assert!(finished.recv_timeout(Duration::from_millis(500)).is_err(), "paused: no reading by itself");
        shared.update(|c| c.recheck = true); // "refresh now"
        let (_, _, recheck) = finished.recv_timeout(Duration::from_secs(2)).expect("a reading once asked");
        assert!(recheck);
        assert!(!shared.ctl.lock().unwrap().recheck, "the request is used up by that one reading");
    }

    #[test]
    fn a_forced_reading_does_not_wait_the_full_interval() {
        let shared = Shared::new(5_000);
        shared.update(|c| c.visible = true);
        let started = Instant::now();
        let (visible, _, _) = wait_until_due(&shared, Instant::now());
        assert!(visible);
        assert!(started.elapsed() < Duration::from_millis(1_000), "took {:?}", started.elapsed());
        assert!(started.elapsed() >= MIN_GAP - Duration::from_millis(20), "but never sooner than the minimum gap");
    }

    #[test]
    fn an_idle_sampler_waits_for_the_interval() {
        let shared = Shared::new(400);
        let started = Instant::now();
        wait_until_due(&shared, Instant::now());
        assert!(started.elapsed() >= Duration::from_millis(380));
    }
}

#[cfg(test)]
mod timing {
    use super::*;

    /// Not a check: prints where the time of one reading goes. Run with `--ignored --nocapture`.
    #[test]
    #[ignore]
    fn where_the_time_goes() {
        let time = |label: &str, rounds: u32, mut f: Box<dyn FnMut()>| {
            f();
            let t = Instant::now();
            for _ in 0..rounds {
                f();
            }
            println!("TIMING {label}: {:.2} ms", t.elapsed().as_secs_f64() * 1000.0 / rounds as f64);
        };
        for path in [
            r"\GPU Engine(*)\Utilization Percentage",
            r"\GPU Adapter Memory(*)\Dedicated Usage",
            r"\PhysicalDisk(*)\% Idle Time",
            r"\Processor Information(_Total)\% Processor Performance",
        ] {
            let q = Query::open().unwrap();
            let k = q.add(path);
            time(&format!("collect {path}"), 20, Box::new(move || {
                q.collect();
                if let Some(k) = k {
                    let _ = q.values(k);
                }
            }));
        }
        let mut sys = System::new_with_specifics(
            RefreshKind::nothing().with_cpu(CpuRefreshKind::nothing().with_usage()).with_memory(MemoryRefreshKind::everything()),
        )
        .unwrap();
        time("sysinfo cpu+memory", 20, Box::new(move || {
            sys.refresh_cpu_usage();
            sys.refresh_memory();
        }));
        let mut nets = Networks::new_with_refreshed_list().unwrap();
        time("sysinfo networks", 20, Box::new(move || {
            nets.refresh(true);
        }));
        let adapters = list_gpus();
        time("gpu temperature", 20, Box::new(move || {
            for a in &adapters {
                let _ = hardware::gpu_temperature(a.luid.high, a.luid.low);
            }
        }));
        let mut engine = proclist::Engine::new(8.0);
        let none = HashMap::new();
        time("process list", 20, Box::new(move || {
            let _ = engine.read(&none, false);
        }));
        let mut s = Sampler::new().unwrap();
        time("whole perf reading", 20, Box::new(move || {
            let _ = s.perf(true);
        }));
    }
}
