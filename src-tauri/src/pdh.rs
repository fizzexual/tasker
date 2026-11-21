//! Windows performance counters (the same source Windows Task Manager reads):
//! disk activity, GPU load, GPU memory, thread and handle totals.

use windows::core::PCWSTR;
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Performance::{
    PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData, PdhGetFormattedCounterArrayW,
    PdhGetFormattedCounterValue, PdhOpenQueryW, PDH_FMT_COUNTERVALUE, PDH_FMT_COUNTERVALUE_ITEM_W,
    PDH_FMT_DOUBLE, PDH_HCOUNTER, PDH_HQUERY,
};

pub struct Query {
    query: PDH_HQUERY,
}

// The handle is only ever used from the sampler thread that owns the `Query`.
unsafe impl Send for Query {}

#[derive(Clone, Copy)]
pub struct Counter(PDH_HCOUNTER);
unsafe impl Send for Counter {}

impl Query {
    pub fn open() -> Option<Query> {
        let mut query = PDH_HQUERY::default();
        let status = unsafe { PdhOpenQueryW(PCWSTR::null(), 0, &mut query) };
        (status == ERROR_SUCCESS.0).then_some(Query { query })
    }

    /// Adds a counter by its English path. Returns `None` when this PC does not have it
    /// (for example no GPU counters in a virtual machine); the caller then shows nothing.
    pub fn add(&self, path: &str) -> Option<Counter> {
        let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
        let mut counter = PDH_HCOUNTER::default();
        let status =
            unsafe { PdhAddEnglishCounterW(self.query, PCWSTR::from_raw(wide.as_ptr()), 0, &mut counter) };
        (status == ERROR_SUCCESS.0 && !counter.is_invalid()).then_some(Counter(counter))
    }

    /// Takes one reading of every counter. Rate counters need two readings before they
    /// return a value; until then `values`/`value` simply give nothing.
    pub fn collect(&self) {
        unsafe {
            PdhCollectQueryData(self.query);
        }
    }

    /// Single-instance counter value.
    pub fn value(&self, counter: Counter) -> Option<f64> {
        let mut out = PDH_FMT_COUNTERVALUE::default();
        let status = unsafe { PdhGetFormattedCounterValue(counter.0, PDH_FMT_DOUBLE, None, &mut out) };
        (status == ERROR_SUCCESS.0).then(|| unsafe { out.Anonymous.doubleValue })
    }

    /// All instances of a wildcard counter as `(instance name, value)`.
    pub fn values(&self, counter: Counter) -> Vec<(String, f64)> {
        let mut size = 0u32;
        let mut count = 0u32;
        unsafe {
            PdhGetFormattedCounterArrayW(counter.0, PDH_FMT_DOUBLE, &mut size, &mut count, None);
        }
        if size == 0 || count == 0 {
            return Vec::new();
        }
        // u64 storage keeps the buffer aligned for the item structs.
        let mut buffer = vec![0u64; (size as usize).div_ceil(8)];
        let status = unsafe {
            PdhGetFormattedCounterArrayW(
                counter.0,
                PDH_FMT_DOUBLE,
                &mut size,
                &mut count,
                Some(buffer.as_mut_ptr() as *mut PDH_FMT_COUNTERVALUE_ITEM_W),
            )
        };
        if status != ERROR_SUCCESS.0 {
            return Vec::new();
        }
        let items = unsafe {
            std::slice::from_raw_parts(buffer.as_ptr() as *const PDH_FMT_COUNTERVALUE_ITEM_W, count as usize)
        };
        items
            .iter()
            .filter(|item| item.FmtValue.CStatus == 0 && !item.szName.is_null())
            .map(|item| unsafe {
                (PCWSTR(item.szName.0).to_string().unwrap_or_default(), item.FmtValue.Anonymous.doubleValue)
            })
            .collect()
    }
}

impl Drop for Query {
    fn drop(&mut self) {
        unsafe {
            PdhCloseQuery(self.query);
        }
    }
}

/// A GPU adapter id as it appears in counter instance names: `luid_0x<high>_0x<low>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Luid {
    pub high: i32,
    pub low: u32,
}

/// Reads the adapter id out of names like
/// `pid_1234_luid_0x00000000_0x0000C4F1_phys_0_eng_3_engtype_3D` or
/// `luid_0x00000000_0x0000C4F1_phys_0`.
pub fn parse_luid(instance: &str) -> Option<Luid> {
    let rest = &instance[instance.find("luid_0x")? + 7..];
    let mut parts = rest.split('_');
    let high = u32::from_str_radix(parts.next()?, 16).ok()? as i32;
    let low = u32::from_str_radix(parts.next()?.strip_prefix("0x")?, 16).ok()?;
    Some(Luid { high, low })
}

/// Reads the engine number (`eng_3`) out of a GPU engine instance name.
pub fn parse_engine(instance: &str) -> Option<u32> {
    let rest = &instance[instance.find("_eng_")? + 5..];
    rest.split('_').next()?.parse().ok()
}

/// Turns per-process, per-engine readings into one load figure per GPU.
///
/// Each engine (3D, video decode, copy…) is summed across all processes; the GPU's load is
/// its busiest engine. That is the figure Windows Task Manager shows for a GPU.
pub fn gpu_load(readings: &[(String, f64)]) -> Vec<(Luid, f32)> {
    use std::collections::HashMap;
    let mut per_engine: HashMap<(Luid, u32), f64> = HashMap::new();
    for (name, value) in readings {
        if let (Some(luid), Some(engine)) = (parse_luid(name), parse_engine(name)) {
            *per_engine.entry((luid, engine)).or_default() += value;
        }
    }
    let mut per_gpu: HashMap<Luid, f64> = HashMap::new();
    for ((luid, _), value) in per_engine {
        let slot = per_gpu.entry(luid).or_default();
        if value > *slot {
            *slot = value;
        }
    }
    per_gpu.into_iter().map(|(luid, v)| (luid, v.clamp(0.0, 100.0) as f32)).collect()
}

/// Reads the process id out of a GPU engine instance name (`pid_1234_luid_…`).
pub fn parse_pid(instance: &str) -> Option<u32> {
    instance.strip_prefix("pid_")?.split('_').next()?.parse().ok()
}

/// Reads the kind of work an engine does (`engtype_VideoDecode` gives `VideoDecode`).
pub fn parse_engine_kind(instance: &str) -> Option<&str> {
    let kind = &instance[instance.find("_engtype_")? + 9..];
    (!kind.is_empty()).then_some(kind)
}

/// GPU share per process: its busiest engine, as Windows Task Manager shows it.
pub fn gpu_by_pid(readings: &[(String, f64)]) -> std::collections::HashMap<u32, f32> {
    use std::collections::HashMap;
    let mut per_engine: HashMap<(u32, Luid, u32), f64> = HashMap::new();
    for (name, value) in readings {
        if let (Some(pid), Some(luid), Some(engine)) = (parse_pid(name), parse_luid(name), parse_engine(name)) {
            *per_engine.entry((pid, luid, engine)).or_default() += value.max(0.0);
        }
    }
    let mut per_pid: HashMap<u32, f32> = HashMap::new();
    for ((pid, _, _), value) in per_engine {
        let slot = per_pid.entry(pid).or_default();
        *slot = slot.max(value.min(100.0) as f32);
    }
    per_pid.retain(|_, v| *v > 0.0);
    per_pid
}

/// Load per GPU and kind of work (3D, Copy, VideoDecode…): the busiest engine of each kind.
/// Sorted by GPU, then busiest first.
pub fn gpu_engine_kinds(readings: &[(String, f64)]) -> Vec<(Luid, String, f32)> {
    use std::collections::HashMap;
    let mut per_engine: HashMap<(Luid, u32, &str), f64> = HashMap::new();
    for (name, value) in readings {
        if let (Some(luid), Some(engine), Some(kind)) = (parse_luid(name), parse_engine(name), parse_engine_kind(name)) {
            *per_engine.entry((luid, engine, kind)).or_default() += value.max(0.0);
        }
    }
    let mut per_kind: HashMap<(Luid, &str), f64> = HashMap::new();
    for ((luid, _, kind), value) in per_engine {
        let slot = per_kind.entry((luid, kind)).or_default();
        *slot = slot.max(value);
    }
    let mut out: Vec<(Luid, String, f32)> =
        per_kind.into_iter().map(|((luid, kind), v)| (luid, kind.to_string(), v.min(100.0) as f32)).collect();
    out.sort_by(|a, b| (a.0.high, a.0.low).cmp(&(b.0.high, b.0.low)).then(b.2.total_cmp(&a.2)).then(a.1.cmp(&b.1)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "luid_0x00000000_0x0000C4F1";
    const B: &str = "luid_0x00000001_0x00010ABC";

    fn r(name: String, v: f64) -> (String, f64) {
        (name, v)
    }

    #[test]
    fn parses_adapter_id_from_engine_and_memory_names() {
        let want = Some(Luid { high: 0, low: 0xC4F1 });
        assert_eq!(parse_luid(&format!("pid_1234_{A}_phys_0_eng_3_engtype_3D")), want);
        assert_eq!(parse_luid(&format!("{A}_phys_0")), want);
        assert_eq!(parse_luid("pid_1_phys_0"), None);
        assert_eq!(parse_luid("luid_0xZZ_0x1"), None);
    }

    #[test]
    fn parses_engine_number() {
        assert_eq!(parse_engine(&format!("pid_9_{A}_phys_0_eng_12_engtype_VideoDecode")), Some(12));
        assert_eq!(parse_engine(&format!("{A}_phys_0")), None);
    }

    #[test]
    fn gpu_load_is_the_busiest_engine_summed_over_processes() {
        let readings = vec![
            r(format!("pid_1_{A}_phys_0_eng_0_engtype_3D"), 30.0),
            r(format!("pid_2_{A}_phys_0_eng_0_engtype_3D"), 25.0),
            r(format!("pid_2_{A}_phys_0_eng_5_engtype_VideoDecode"), 40.0),
            r(format!("pid_3_{B}_phys_0_eng_0_engtype_3D"), 7.5),
        ];
        let load: std::collections::HashMap<_, _> = gpu_load(&readings).into_iter().collect();
        assert_eq!(load[&Luid { high: 0, low: 0xC4F1 }], 55.0);
        assert_eq!(load[&Luid { high: 1, low: 0x10ABC }], 7.5);
    }

    #[test]
    fn gpu_load_never_exceeds_100() {
        let readings =
            vec![r(format!("pid_1_{A}_phys_0_eng_0_engtype_3D"), 80.0), r(format!("pid_2_{A}_phys_0_eng_0_engtype_3D"), 70.0)];
        assert_eq!(gpu_load(&readings)[0].1, 100.0);
    }

    #[test]
    fn unreadable_names_are_skipped() {
        assert!(gpu_load(&[r("garbage".into(), 50.0)]).is_empty());
    }

    #[test]
    fn real_counters_open_on_this_pc() {
        let q = Query::open().expect("the performance counter service is available");
        let threads = q.add(r"\System\Threads").expect("the System counters exist on every Windows");
        q.collect();
        assert!(q.value(threads).unwrap_or(0.0) > 10.0, "a running Windows has more than 10 threads");
        assert!(q.add(r"\No Such Object\No Such Counter").is_none(), "a missing counter is reported, not a crash");
    }

    #[test]
    fn gpu_share_per_process_is_its_busiest_engine() {
        let r = vec![
            ("pid_100_luid_0x00000000_0x0000C4F1_phys_0_eng_0_engtype_3D".to_string(), 30.0),
            ("pid_100_luid_0x00000000_0x0000C4F1_phys_0_eng_3_engtype_VideoDecode".to_string(), 55.0),
            ("pid_200_luid_0x00000000_0x0000C4F1_phys_0_eng_0_engtype_3D".to_string(), 250.0),
            ("pid_300_luid_0x00000000_0x0000C4F1_phys_0_eng_0_engtype_3D".to_string(), 0.0),
            ("garbage".to_string(), 99.0),
        ];
        let by_pid = gpu_by_pid(&r);
        assert_eq!(by_pid.get(&100), Some(&55.0));
        assert_eq!(by_pid.get(&200), Some(&100.0), "never above 100");
        assert!(!by_pid.contains_key(&300), "idle programs are left out");
        assert_eq!(by_pid.len(), 2);
    }

    #[test]
    fn gpu_load_per_kind_of_work() {
        let r = vec![
            ("pid_100_luid_0x00000000_0x0000C4F1_phys_0_eng_0_engtype_3D".to_string(), 30.0),
            ("pid_200_luid_0x00000000_0x0000C4F1_phys_0_eng_0_engtype_3D".to_string(), 20.0),
            ("pid_100_luid_0x00000000_0x0000C4F1_phys_0_eng_5_engtype_3D".to_string(), 10.0),
            ("pid_100_luid_0x00000000_0x0000C4F1_phys_0_eng_3_engtype_VideoDecode".to_string(), 5.0),
        ];
        let luid = Luid { high: 0, low: 0xC4F1 };
        assert_eq!(gpu_engine_kinds(&r), vec![(luid, "3D".to_string(), 50.0), (luid, "VideoDecode".to_string(), 5.0)]);
        assert_eq!(parse_pid("pid_12_luid_0x0_0x1"), Some(12));
        assert_eq!(parse_pid("luid_0x0_0x1"), None);
        assert_eq!(parse_engine_kind("x_eng_1_engtype_"), None);
    }
}
