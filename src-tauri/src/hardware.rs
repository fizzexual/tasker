//! Facts about the PC's parts that do not change while it runs: processor caches, memory
//! sticks, disk models, network addresses, graphics driver. Read once when the Performance
//! screen asks, never on the refresh path.

use serde::Serialize;
use std::ffi::c_void;
use windows::core::{HSTRING, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE, LUID};
use windows::Win32::NetworkManagement::IpHelper::{GetAdaptersAddresses, GET_ADAPTERS_ADDRESSES_FLAGS, IP_ADAPTER_ADDRESSES_LH};
use windows::Win32::Storage::FileSystem::{CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING};
use windows::Win32::System::SystemInformation::{
    GetLogicalProcessorInformationEx, GetPhysicallyInstalledSystemMemory, GetSystemFirmwareTable,
    FIRMWARE_TABLE_PROVIDER, LOGICAL_PROCESSOR_RELATIONSHIP,
};
use windows::Win32::System::Threading::{IsProcessorFeaturePresent, PROCESSOR_FEATURE_ID};
use windows::Win32::System::IO::DeviceIoControl;
use winreg::enums::HKEY_LOCAL_MACHINE;
use winreg::RegKey;

#[derive(Debug, Serialize, Clone, Default, PartialEq)]
pub struct CpuInfo {
    pub base_mhz: u32,
    pub sockets: u32,
    /// Cache sizes in bytes, all cores together.
    pub l1: u64,
    pub l2: u64,
    pub l3: u64,
    pub virtualization: bool,
}

#[derive(Debug, Serialize, Clone, Default, PartialEq)]
pub struct MemoryInfo {
    /// Bytes fitted in the PC (a little more than Windows can use).
    pub installed: u64,
    pub slots_used: u32,
    pub slots_total: u32,
    pub speed_mhz: u32,
    pub form_factor: String,
}

#[derive(Debug, Serialize, Clone, Default, PartialEq)]
pub struct DiskInfo {
    /// The number Windows gives the disk ("Disk 0").
    pub index: u32,
    pub model: String,
    /// "SSD", "SSD (NVMe)", "HDD", "USB" or empty when unknown.
    pub kind: String,
    pub size: u64,
}

#[derive(Debug, Serialize, Clone, Default, PartialEq)]
pub struct NetInfo {
    pub name: String,
    pub description: String,
    /// "Ethernet", "Wi-Fi" or empty.
    pub kind: String,
    pub ipv4: String,
    pub ipv6: String,
    /// Bits per second; 0 when not connected.
    pub link_bps: u64,
}

#[derive(Debug, Serialize, Clone, Default, PartialEq)]
pub struct GpuInfo {
    pub name: String,
    pub driver_version: String,
    pub driver_date: String,
}

#[derive(Debug, Serialize, Clone, Default)]
pub struct Hardware {
    pub cpu: CpuInfo,
    pub memory: MemoryInfo,
    pub disks: Vec<DiskInfo>,
    pub nets: Vec<NetInfo>,
    pub gpus: Vec<GpuInfo>,
    /// How long the PC's firmware took before Windows started loading ("Last BIOS time"), ms.
    pub bios_ms: u32,
}

pub fn read() -> Hardware {
    Hardware { cpu: cpu(), memory: memory(), disks: disks(), nets: read_nets(), gpus: gpus(), bios_ms: bios_ms() }
}

// ───────────────────────── processor ─────────────────────────

pub fn base_mhz() -> u32 {
    RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey(r"HARDWARE\DESCRIPTION\System\CentralProcessor\0")
        .and_then(|k| k.get_value::<u32, _>("~MHz"))
        .unwrap_or(0)
}

const RELATION_CACHE: i32 = 2;
const RELATION_PACKAGE: i32 = 3;

fn processor_records(relation: i32) -> Vec<u8> {
    let mut len = 0u32;
    unsafe {
        let _ = GetLogicalProcessorInformationEx(LOGICAL_PROCESSOR_RELATIONSHIP(relation), None, &mut len);
        if len == 0 {
            return Vec::new();
        }
        let mut buf = vec![0u64; (len as usize).div_ceil(8)];
        if GetLogicalProcessorInformationEx(LOGICAL_PROCESSOR_RELATIONSHIP(relation), Some(buf.as_mut_ptr() as *mut _), &mut len)
            .is_err()
        {
            return Vec::new();
        }
        std::slice::from_raw_parts(buf.as_ptr() as *const u8, len as usize).to_vec()
    }
}

/// Walks the variable-length records Windows returns: `(relationship, record bytes)`.
fn records(buf: &[u8]) -> Vec<(i32, &[u8])> {
    let mut out = Vec::new();
    let mut at = 0usize;
    while let Some(head) = buf.get(at..at + 8) {
        let relation = i32::from_le_bytes([head[0], head[1], head[2], head[3]]);
        let size = u32::from_le_bytes([head[4], head[5], head[6], head[7]]) as usize;
        let Some(record) = buf.get(at..at + size) else { break };
        if size < 8 {
            break;
        }
        out.push((relation, record));
        at += size;
    }
    out
}

/// Total size of the level 1, 2 and 3 caches from cache records.
pub fn cache_sizes(buf: &[u8]) -> [u64; 3] {
    let mut sizes = [0u64; 3];
    for (relation, record) in records(buf) {
        if relation != RELATION_CACHE || record.len() < 16 {
            continue;
        }
        let level = record[8] as usize;
        let size = u32::from_le_bytes([record[12], record[13], record[14], record[15]]) as u64;
        if (1..=3).contains(&level) {
            sizes[level - 1] += size;
        }
    }
    sizes
}

fn hypervisor_running() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        // Bit 31 of this value is set when Windows itself runs on top of a hypervisor
        // (Hyper-V, WSL 2), which is only possible with virtualization switched on.
        std::arch::x86_64::__cpuid(1).ecx >> 31 & 1 == 1
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        false
    }
}

fn cpu() -> CpuInfo {
    let [l1, l2, l3] = cache_sizes(&processor_records(RELATION_CACHE));
    let sockets = records(&processor_records(RELATION_PACKAGE)).iter().filter(|(r, _)| *r == RELATION_PACKAGE).count();
    const VIRT_FIRMWARE_ENABLED: u32 = 21;
    let firmware = unsafe { IsProcessorFeaturePresent(PROCESSOR_FEATURE_ID(VIRT_FIRMWARE_ENABLED)).as_bool() };
    CpuInfo { base_mhz: base_mhz(), sockets: sockets as u32, l1, l2, l3, virtualization: firmware || hypervisor_running() }
}

// ───────────────────────── memory ─────────────────────────

pub fn form_factor_name(code: u8) -> &'static str {
    match code {
        0x09 => "DIMM",
        0x0D => "SODIMM",
        0x0B => "Row of chips",
        0x0F => "FB-DIMM",
        _ => "",
    }
}

/// Reads the memory-stick records out of the firmware's hardware table.
/// `table` is the table body (after the 8-byte Windows header).
pub fn parse_memory(table: &[u8]) -> MemoryInfo {
    let mut info = MemoryInfo::default();
    let mut at = 0usize;
    while at + 4 <= table.len() {
        let kind = table[at];
        let len = table[at + 1] as usize;
        if len < 4 || at + len > table.len() {
            break;
        }
        let body = &table[at..at + len];
        if kind == 17 && len >= 0x17 {
            info.slots_total += 1;
            let size = u16::from_le_bytes([body[0x0C], body[0x0D]]);
            if size != 0 && size != 0xFFFF {
                info.slots_used += 1;
                let speed = if len > 0x21 { u16::from_le_bytes([body[0x20], body[0x21]]) } else { 0 };
                let speed = if speed == 0 { u16::from_le_bytes([body[0x15], body[0x16]]) } else { speed };
                info.speed_mhz = info.speed_mhz.max(speed as u32);
                if info.form_factor.is_empty() {
                    info.form_factor = form_factor_name(body[0x0E]).to_string();
                }
            }
        }
        if kind == 127 {
            break;
        }
        // Text belonging to the record follows it and ends with two zero bytes.
        let mut end = at + len;
        while end + 1 < table.len() && !(table[end] == 0 && table[end + 1] == 0) {
            end += 1;
        }
        at = end + 2;
    }
    info
}

fn memory() -> MemoryInfo {
    const RSMB: u32 = 0x5253_4D42;
    let mut info = MemoryInfo::default();
    unsafe {
        let size = GetSystemFirmwareTable(FIRMWARE_TABLE_PROVIDER(RSMB), 0, None);
        if size > 8 {
            let mut buf = vec![0u8; size as usize];
            let got = GetSystemFirmwareTable(FIRMWARE_TABLE_PROVIDER(RSMB), 0, Some(&mut buf)) as usize;
            if got > 8 && got <= buf.len() {
                info = parse_memory(&buf[8..got]);
            }
        }
        let mut kb = 0u64;
        if GetPhysicallyInstalledSystemMemory(&mut kb).is_ok() {
            info.installed = kb * 1024;
        }
    }
    info
}

// ───────────────────────── disks ─────────────────────────

const IOCTL_STORAGE_QUERY_PROPERTY: u32 = 0x002D_1400;
const IOCTL_DISK_GET_DRIVE_GEOMETRY_EX: u32 = 0x0007_00A0;

struct Device(HANDLE);
impl Drop for Device {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

fn ioctl(device: &Device, code: u32, input: &[u8], output: &mut [u8]) -> Option<usize> {
    let mut returned = 0u32;
    unsafe {
        DeviceIoControl(
            device.0,
            code,
            (!input.is_empty()).then_some(input.as_ptr() as *const c_void),
            input.len() as u32,
            Some(output.as_mut_ptr() as *mut c_void),
            output.len() as u32,
            Some(&mut returned),
            None,
        )
        .ok()?;
    }
    Some(returned as usize)
}

fn text_at(buf: &[u8], offset: usize) -> String {
    if offset == 0 || offset >= buf.len() {
        return String::new();
    }
    let end = buf[offset..].iter().position(|b| *b == 0).map(|p| offset + p).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[offset..end]).trim().to_string()
}

pub fn disk_kind(bus: u32, seek_penalty: Option<bool>) -> &'static str {
    match (bus, seek_penalty) {
        (17, _) => "SSD (NVMe)",
        (7, _) => "USB",
        (_, Some(false)) => "SSD",
        (_, Some(true)) => "HDD",
        _ => "",
    }
}

fn disks() -> Vec<DiskInfo> {
    let mut out = Vec::new();
    for index in 0..32u32 {
        let path = HSTRING::from(format!(r"\\.\PhysicalDrive{index}"));
        // Opened with no access rights: enough to ask what the disk is, never to read it.
        let handle = unsafe {
            CreateFileW(&path, 0, FILE_SHARE_READ | FILE_SHARE_WRITE, None, OPEN_EXISTING, FILE_FLAGS_AND_ATTRIBUTES(0), None)
        };
        let Ok(handle) = handle else { continue };
        let device = Device(handle);

        let mut descriptor = [0u8; 1024];
        let query = [0u8; 12]; // property 0 (the device), standard query
        let mut model = String::new();
        let mut bus = 0u32;
        if let Some(n) = ioctl(&device, IOCTL_STORAGE_QUERY_PROPERTY, &query, &mut descriptor) {
            if n >= 32 {
                let field = |at: usize| u32::from_le_bytes([descriptor[at], descriptor[at + 1], descriptor[at + 2], descriptor[at + 3]]);
                let vendor = text_at(&descriptor, field(12) as usize);
                let product = text_at(&descriptor, field(16) as usize);
                model = if vendor.is_empty() || product.starts_with(&vendor) { product } else { format!("{vendor} {product}") };
                bus = field(28);
            }
        }
        let mut seek = [0u8; 12];
        let mut seek_query = [0u8; 12];
        seek_query[0] = 7; // the "does moving the head cost time" property
        let seek_penalty = ioctl(&device, IOCTL_STORAGE_QUERY_PROPERTY, &seek_query, &mut seek).filter(|n| *n >= 9).map(|_| seek[8] != 0);
        let mut geometry = [0u8; 64];
        let size = ioctl(&device, IOCTL_DISK_GET_DRIVE_GEOMETRY_EX, &[], &mut geometry)
            .filter(|n| *n >= 32)
            .map(|_| i64::from_le_bytes(geometry[24..32].try_into().unwrap_or([0; 8])).max(0) as u64)
            .unwrap_or(0);
        out.push(DiskInfo { index, model, kind: disk_kind(bus, seek_penalty).to_string(), size });
    }
    out
}

// ───────────────────────── network ─────────────────────────

unsafe fn wide(text: PWSTR) -> String {
    if text.is_null() {
        String::new()
    } else {
        text.to_string().unwrap_or_default()
    }
}

pub fn read_nets() -> Vec<NetInfo> {
    const SKIP_EXTRAS: u32 = 0x2 | 0x4 | 0x8; // anycast, multicast, DNS servers
    let mut out = Vec::new();
    let mut size = 16 * 1024u32;
    unsafe {
        let mut buf = vec![0u64; size as usize / 8];
        let mut status = GetAdaptersAddresses(0, GET_ADAPTERS_ADDRESSES_FLAGS(SKIP_EXTRAS), None, Some(buf.as_mut_ptr() as *mut _), &mut size);
        if status == 111 {
            // Buffer too small: Windows told us the size it needs.
            buf = vec![0u64; (size as usize).div_ceil(8)];
            status = GetAdaptersAddresses(0, GET_ADAPTERS_ADDRESSES_FLAGS(SKIP_EXTRAS), None, Some(buf.as_mut_ptr() as *mut _), &mut size);
        }
        if status != 0 {
            return out;
        }
        let mut adapter = buf.as_ptr() as *const IP_ADAPTER_ADDRESSES_LH;
        while !adapter.is_null() {
            let a = &*adapter;
            let mut info = NetInfo {
                name: wide(a.FriendlyName),
                description: wide(a.Description),
                kind: match a.IfType {
                    6 => "Ethernet".into(),
                    71 => "Wi-Fi".into(),
                    _ => String::new(),
                },
                link_bps: if a.OperStatus.0 == 1 && a.TransmitLinkSpeed != u64::MAX { a.TransmitLinkSpeed } else { 0 },
                ..Default::default()
            };
            let mut address = a.FirstUnicastAddress;
            while !address.is_null() {
                let sockaddr = (*address).Address.lpSockaddr as *const u8;
                if !sockaddr.is_null() {
                    let family = u16::from_le_bytes([*sockaddr, *sockaddr.add(1)]);
                    if family == 2 && info.ipv4.is_empty() {
                        let b = std::slice::from_raw_parts(sockaddr.add(4), 4);
                        info.ipv4 = std::net::Ipv4Addr::new(b[0], b[1], b[2], b[3]).to_string();
                    } else if family == 23 && info.ipv6.is_empty() {
                        let mut b = [0u8; 16];
                        b.copy_from_slice(std::slice::from_raw_parts(sockaddr.add(8), 16));
                        info.ipv6 = std::net::Ipv6Addr::from(b).to_string();
                    }
                }
                address = (*address).Next;
            }
            out.push(info);
            adapter = a.Next;
        }
    }
    out
}

// ───────────────────────── graphics ─────────────────────────

fn gpus() -> Vec<GpuInfo> {
    let mut out: Vec<GpuInfo> = Vec::new();
    let Ok(class) = RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey(r"SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}")
    else {
        return out;
    };
    for key in class.enum_keys().flatten() {
        let Ok(card) = class.open_subkey(&key) else { continue };
        let Ok(name) = card.get_value::<String, _>("DriverDesc") else { continue };
        if out.iter().any(|g| g.name == name) {
            continue;
        }
        out.push(GpuInfo {
            name,
            driver_version: card.get_value("DriverVersion").unwrap_or_default(),
            driver_date: card.get_value("DriverDate").unwrap_or_default(),
        });
    }
    out
}

#[repr(C)]
struct OpenAdapter {
    luid: LUID,
    handle: u32,
}
#[repr(C)]
struct QueryAdapter {
    handle: u32,
    kind: i32,
    data: *mut c_void,
    size: u32,
}
#[repr(C)]
#[derive(Default)]
struct PerfData {
    physical_index: u32,
    memory_frequency: u64,
    max_memory_frequency: u64,
    max_memory_frequency_oc: u64,
    memory_bandwidth: u64,
    pcie_bandwidth: u64,
    fan_rpm: u32,
    power: u32,
    /// Tenths of a degree Celsius.
    temperature: u32,
    power_state_override: u8,
}

#[link(name = "gdi32")]
extern "system" {
    fn D3DKMTOpenAdapterFromLuid(open: *mut OpenAdapter) -> i32;
    fn D3DKMTQueryAdapterInfo(query: *mut QueryAdapter) -> i32;
    fn D3DKMTCloseAdapter(handle: *const u32) -> i32;
}

/// Temperature of a graphics card in °C, when its driver reports one.
pub fn gpu_temperature(luid_high: i32, luid_low: u32) -> Option<f32> {
    const ADAPTER_PERF_DATA: i32 = 62;
    unsafe {
        let mut open = OpenAdapter { luid: LUID { LowPart: luid_low, HighPart: luid_high }, handle: 0 };
        if D3DKMTOpenAdapterFromLuid(&mut open) < 0 {
            return None;
        }
        let mut data = PerfData::default();
        let mut query = QueryAdapter {
            handle: open.handle,
            kind: ADAPTER_PERF_DATA,
            data: &mut data as *mut _ as *mut c_void,
            size: std::mem::size_of::<PerfData>() as u32,
        };
        let status = D3DKMTQueryAdapterInfo(&mut query);
        D3DKMTCloseAdapter(&open.handle);
        if status < 0 || data.temperature == 0 {
            return None;
        }
        let celsius = data.temperature as f32 / 10.0;
        (1.0..150.0).contains(&celsius).then_some(celsius)
    }
}

fn bios_ms() -> u32 {
    RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey(r"SYSTEM\CurrentControlSet\Control\Session Manager\Power")
        .and_then(|k| k.get_value::<u32, _>("FwPOSTTime"))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cache_record(level: u8, size: u32) -> Vec<u8> {
        let mut r = vec![0u8; 52];
        r[0..4].copy_from_slice(&RELATION_CACHE.to_le_bytes());
        r[4..8].copy_from_slice(&52u32.to_le_bytes());
        r[8] = level;
        r[12..16].copy_from_slice(&size.to_le_bytes());
        r
    }

    #[test]
    fn cache_sizes_are_added_up_per_level() {
        let mut buf = Vec::new();
        for _ in 0..4 {
            buf.extend(cache_record(1, 32 << 10)); // data
            buf.extend(cache_record(1, 32 << 10)); // instructions
            buf.extend(cache_record(2, 512 << 10));
        }
        buf.extend(cache_record(3, 16 << 20));
        buf.extend(cache_record(4, 999)); // an unknown level is ignored
        assert_eq!(cache_sizes(&buf), [256 << 10, 2 << 20, 16 << 20]);
    }

    #[test]
    fn damaged_processor_records_end_the_walk() {
        let mut buf = cache_record(2, 100);
        buf.extend([2, 0, 0, 0, 255, 255, 0, 0]); // claims to be longer than the data
        assert_eq!(cache_sizes(&buf), [0, 100, 0]);
        let mut zero = cache_record(2, 100);
        zero.extend([2, 0, 0, 0, 0, 0, 0, 0]); // size 0 would loop for ever
        assert_eq!(cache_sizes(&zero), [0, 100, 0]);
        assert_eq!(cache_sizes(&[1, 2, 3]), [0, 0, 0]);
    }

    fn memory_stick(size_mb: u16, speed: u16, form: u8) -> Vec<u8> {
        let mut r = vec![0u8; 0x22];
        r[0] = 17;
        r[1] = 0x22;
        r[0x0C..0x0E].copy_from_slice(&size_mb.to_le_bytes());
        r[0x0E] = form;
        r[0x15..0x17].copy_from_slice(&speed.to_le_bytes());
        r[0x20..0x22].copy_from_slice(&speed.to_le_bytes());
        r.extend(b"DIMM 0\0Bank\0\0");
        r
    }

    #[test]
    fn memory_sticks_are_counted_with_speed_and_shape() {
        let mut table = vec![0u8, 0x18, 0, 0];
        table.extend([0u8; 0x14]);
        table.extend(b"Vendor\0\0"); // a BIOS record first
        table.extend(memory_stick(16384, 3600, 0x09));
        table.extend(memory_stick(0, 0, 0x09)); // an empty slot
        table.extend(memory_stick(16384, 3200, 0x09));
        table.extend(memory_stick(0, 0, 0x09));
        table.extend([127, 4, 0, 0, 0, 0]);
        let info = parse_memory(&table);
        assert_eq!((info.slots_used, info.slots_total), (2, 4));
        assert_eq!(info.speed_mhz, 3600);
        assert_eq!(info.form_factor, "DIMM");
    }

    #[test]
    fn a_damaged_memory_table_gives_what_was_read_so_far() {
        let mut table = memory_stick(8192, 2400, 0x0D);
        table.extend([17, 200, 0, 0]); // record longer than the table
        let info = parse_memory(&table);
        assert_eq!((info.slots_used, info.slots_total, info.form_factor.as_str()), (1, 1, "SODIMM"));
        assert_eq!(parse_memory(&[]), MemoryInfo::default());
        assert_eq!(parse_memory(&[17, 1, 0, 0]), MemoryInfo::default(), "a record shorter than its own header");
    }

    #[test]
    fn disk_kinds() {
        assert_eq!(disk_kind(17, Some(false)), "SSD (NVMe)");
        assert_eq!(disk_kind(11, Some(false)), "SSD");
        assert_eq!(disk_kind(11, Some(true)), "HDD");
        assert_eq!(disk_kind(7, Some(true)), "USB");
        assert_eq!(disk_kind(11, None), "");
    }

    #[test]
    fn this_pc_reports_real_parts() {
        let hw = read();
        assert!(hw.cpu.base_mhz > 500, "{:?}", hw.cpu);
        assert!(hw.cpu.sockets >= 1 && hw.cpu.l1 > 0 && hw.cpu.l2 > hw.cpu.l1 / 64);
        assert!(hw.memory.installed > 1 << 30);
        assert!(hw.memory.slots_used >= 1 && hw.memory.slots_used <= hw.memory.slots_total, "{:?}", hw.memory);
        assert!(!hw.disks.is_empty() && hw.disks.iter().all(|d| d.size > 0 && !d.model.is_empty()), "{:?}", hw.disks);
        assert!(hw.nets.iter().any(|n| !n.ipv4.is_empty()), "{:?}", hw.nets);
        assert!(!hw.gpus.is_empty() && hw.gpus.iter().all(|g| !g.driver_version.is_empty()), "{:?}", hw.gpus);
        println!("{hw:#?}");
    }
}
