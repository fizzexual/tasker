//! Facts about one process that do not change every second: where its program file is, who
//! runs it, its command line, publisher, priority. Read on demand and remembered.

use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::{Mutex, OnceLock};
use windows::core::{BOOL, PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, GetLastError, HANDLE};
use windows::Win32::Security::{
    GetLengthSid, GetTokenInformation, LookupAccountSidW, TokenElevation, TokenUser, SID_NAME_USE, TOKEN_ELEVATION,
    TOKEN_QUERY, TOKEN_USER,
};
use windows::Win32::Storage::FileSystem::{
    GetFileVersionInfoSizeW, GetFileVersionInfoW, QueryDosDeviceW, VerQueryValueW,
};
use windows::Win32::System::Threading::{
    GetPriorityClass, GetProcessAffinityMask, IsWow64Process2, OpenProcess, OpenProcessToken,
    QueryFullProcessImageNameW, PROCESS_ACCESS_RIGHTS, PROCESS_NAME_FORMAT, PROCESS_QUERY_LIMITED_INFORMATION,
};

#[link(name = "ntdll")]
extern "system" {
    fn NtQueryInformationProcess(process: HANDLE, class: u32, info: *mut c_void, len: u32, returned: *mut u32) -> i32;
    fn NtQuerySystemInformation(class: u32, info: *mut c_void, len: u32, returned: *mut u32) -> i32;
}
const PROCESS_COMMAND_LINE_INFORMATION: u32 = 60;

/// A process handle that closes itself.
pub struct Handle(pub HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// Opens a process, giving the Windows error number on failure (5 = access denied,
/// 87 = no such process).
pub fn open(pid: u32, access: PROCESS_ACCESS_RIGHTS) -> Result<Handle, u32> {
    unsafe { OpenProcess(access, false, pid).map(Handle).map_err(|_| GetLastError().0) }
}

/// Where a process's program file is. Works for system processes too: when Windows will not let
/// us open the process (most services, without administrator rights), it still tells anyone
/// which file a process ID runs.
pub fn exe_path(pid: u32) -> Option<String> {
    exe_path_by_opening(pid).or_else(|| exe_path_without_opening(pid))
}

fn exe_path_by_opening(pid: u32) -> Option<String> {
    let handle = open(pid, PROCESS_QUERY_LIMITED_INFORMATION).ok()?;
    let mut buf = vec![0u16; 1024];
    let mut len = buf.len() as u32;
    unsafe {
        QueryFullProcessImageNameW(handle.0, PROCESS_NAME_FORMAT(0), PWSTR(buf.as_mut_ptr()), &mut len).ok()?;
    }
    Some(String::from_utf16_lossy(&buf[..len as usize]))
}

#[repr(C)]
struct UnicodeString {
    length: u16,
    maximum_length: u16,
    buffer: *mut u16,
}
#[repr(C)]
struct SystemProcessIdInformation {
    process_id: usize,
    image_name: UnicodeString,
}
const SYSTEM_PROCESS_ID_INFORMATION: u32 = 88;
const STATUS_INFO_LENGTH_MISMATCH: i32 = 0xC000_0004_u32 as i32;

/// Asks Windows which file a process ID runs, without opening the process.
fn exe_path_without_opening(pid: u32) -> Option<String> {
    if pid == 0 {
        return None;
    }
    let mut chars = 520usize;
    for _ in 0..2 {
        let mut buf = vec![0u16; chars];
        let mut info = SystemProcessIdInformation {
            process_id: pid as usize,
            image_name: UnicodeString { length: 0, maximum_length: (chars * 2) as u16, buffer: buf.as_mut_ptr() },
        };
        let status = unsafe {
            NtQuerySystemInformation(
                SYSTEM_PROCESS_ID_INFORMATION,
                &mut info as *mut _ as *mut c_void,
                std::mem::size_of::<SystemProcessIdInformation>() as u32,
                std::ptr::null_mut(),
            )
        };
        if status == STATUS_INFO_LENGTH_MISMATCH && info.image_name.maximum_length as usize > chars * 2 {
            chars = info.image_name.maximum_length as usize / 2 + 1;
            continue;
        }
        if status < 0 {
            return None;
        }
        let len = (info.image_name.length as usize / 2).min(buf.len());
        if len == 0 {
            return None;
        }
        return nt_to_drive_path(&String::from_utf16_lossy(&buf[..len]), &drive_devices());
    }
    None
}

/// Windows names the file its own way (`\Device\HarddiskVolume3\Windows\…`); this turns it into
/// the usual `C:\Windows\…`. `drives` pairs each drive (`C:`) with its device name.
pub fn nt_to_drive_path(nt: &str, drives: &[(String, String)]) -> Option<String> {
    let lower = nt.to_lowercase();
    drives.iter().find_map(|(drive, device)| {
        let d = device.to_lowercase();
        let rest = lower.strip_prefix(&d)?;
        rest.starts_with('\\').then(|| format!("{drive}{}", &nt[device.len()..]))
    })
}

/// Every drive letter with the device behind it. Read once; a drive plugged in later only means
/// a program started from it shows the placeholder icon.
fn drive_devices() -> Vec<(String, String)> {
    static DRIVES: OnceLock<Vec<(String, String)>> = OnceLock::new();
    DRIVES
        .get_or_init(|| {
            let mut out = Vec::new();
            for letter in b'A'..=b'Z' {
                let drive = format!("{}:", letter as char);
                let wide: Vec<u16> = drive.encode_utf16().chain(std::iter::once(0)).collect();
                let mut buf = vec![0u16; 512];
                let n = unsafe { QueryDosDeviceW(PCWSTR(wide.as_ptr()), Some(&mut buf)) } as usize;
                if n == 0 {
                    continue;
                }
                let end = buf.iter().position(|&c| c == 0).unwrap_or(0);
                if end > 0 {
                    out.push((drive, String::from_utf16_lossy(&buf[..end])));
                }
            }
            out
        })
        .clone()
}

pub fn command_line(pid: u32) -> Option<String> {
    let handle = open(pid, PROCESS_QUERY_LIMITED_INFORMATION).ok()?;
    let mut needed = 0u32;
    unsafe {
        NtQueryInformationProcess(handle.0, PROCESS_COMMAND_LINE_INFORMATION, std::ptr::null_mut(), 0, &mut needed);
    }
    if needed < 16 || needed > 1 << 20 {
        return None;
    }
    let mut buf = vec![0u64; (needed as usize).div_ceil(8) + 1];
    let status = unsafe {
        NtQueryInformationProcess(
            handle.0,
            PROCESS_COMMAND_LINE_INFORMATION,
            buf.as_mut_ptr() as *mut c_void,
            (buf.len() * 8) as u32,
            &mut needed,
        )
    };
    if status < 0 {
        return None;
    }
    // The answer is a UNICODE_STRING whose text lies inside the same buffer.
    let bytes = unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const u8, buf.len() * 8) };
    let len = u16::from_le_bytes([bytes[0], bytes[1]]) as usize;
    let ptr = usize::from_le_bytes(bytes[8..16].try_into().ok()?);
    let start = ptr.checked_sub(buf.as_ptr() as usize)?;
    let text = bytes.get(start..start.checked_add(len)?)?;
    let wide: Vec<u16> = text.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    Some(String::from_utf16_lossy(&wide))
}

fn sid_names() -> &'static Mutex<HashMap<Vec<u8>, String>> {
    static CACHE: OnceLock<Mutex<HashMap<Vec<u8>, String>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn token_of(pid: u32) -> Option<Handle> {
    let process = open(pid, PROCESS_QUERY_LIMITED_INFORMATION).ok()?;
    let mut token = HANDLE::default();
    unsafe { OpenProcessToken(process.0, TOKEN_QUERY, &mut token).ok()? };
    Some(Handle(token))
}

/// Account name the process runs as ("SYSTEM", "alex"…).
pub fn user_of(pid: u32) -> Option<String> {
    let token = token_of(pid)?;
    let mut needed = 0u32;
    unsafe {
        let _ = GetTokenInformation(token.0, TokenUser, None, 0, &mut needed);
    }
    if needed == 0 {
        return None;
    }
    let mut buf = vec![0u64; (needed as usize).div_ceil(8)];
    unsafe {
        GetTokenInformation(token.0, TokenUser, Some(buf.as_mut_ptr() as *mut c_void), needed, &mut needed).ok()?;
        let user = &*(buf.as_ptr() as *const TOKEN_USER);
        let sid = user.User.Sid;
        let sid_len = GetLengthSid(sid) as usize;
        let key = std::slice::from_raw_parts(sid.0 as *const u8, sid_len).to_vec();
        if let Some(name) = sid_names().lock().ok()?.get(&key) {
            return Some(name.clone());
        }
        let mut name = vec![0u16; 256];
        let mut name_len = name.len() as u32;
        let mut domain = vec![0u16; 256];
        let mut domain_len = domain.len() as u32;
        let mut kind = SID_NAME_USE::default();
        LookupAccountSidW(
            PCWSTR::null(),
            sid,
            Some(PWSTR(name.as_mut_ptr())),
            &mut name_len,
            Some(PWSTR(domain.as_mut_ptr())),
            &mut domain_len,
            &mut kind,
        )
        .ok()?;
        let text = String::from_utf16_lossy(&name[..name_len as usize]);
        sid_names().lock().ok()?.insert(key, text.clone());
        Some(text)
    }
}

/// Whether the process runs with administrator rights. `None` when Windows will not say.
pub fn elevated(pid: u32) -> Option<bool> {
    let token = token_of(pid)?;
    let mut info = TOKEN_ELEVATION::default();
    let mut needed = 0u32;
    unsafe {
        GetTokenInformation(
            token.0,
            TokenElevation,
            Some(&mut info as *mut _ as *mut c_void),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut needed,
        )
        .ok()?;
    }
    Some(info.TokenIsElevated != 0)
}

/// "64-bit", "32-bit" or "ARM64"; empty when unknown.
pub fn architecture(pid: u32) -> &'static str {
    let Ok(handle) = open(pid, PROCESS_QUERY_LIMITED_INFORMATION) else { return "" };
    let mut process = Default::default();
    let mut native = Default::default();
    if unsafe { IsWow64Process2(handle.0, &mut process, Some(&mut native)) }.is_err() {
        return "";
    }
    machine_name(process.0, native.0)
}

/// `process` is 0 when the program is native to the PC; otherwise it names the emulated kind.
pub fn machine_name(process: u16, native: u16) -> &'static str {
    let of = |m: u16| match m {
        0x014c => "32-bit",
        0x8664 => "64-bit",
        0xAA64 => "ARM64",
        0x01c4 => "ARM 32-bit",
        _ => "",
    };
    if process == 0 {
        of(native)
    } else {
        of(process)
    }
}

pub const PRIORITIES: [(&str, u32); 6] = [
    ("realtime", 0x100),
    ("high", 0x80),
    ("above-normal", 0x8000),
    ("normal", 0x20),
    ("below-normal", 0x4000),
    ("low", 0x40),
];

pub fn priority_name(class: u32) -> &'static str {
    PRIORITIES.iter().find(|(_, c)| *c == class).map(|(n, _)| *n).unwrap_or("")
}

pub fn priority_class(name: &str) -> Option<u32> {
    PRIORITIES.iter().find(|(n, _)| *n == name).map(|(_, c)| *c)
}

pub fn priority_of(pid: u32) -> &'static str {
    match open(pid, PROCESS_QUERY_LIMITED_INFORMATION) {
        Ok(handle) => priority_name(unsafe { GetPriorityClass(handle.0) }),
        Err(_) => "",
    }
}

/// `(processors this process may use, processors the PC has)` as bit masks.
pub fn affinity_of(pid: u32) -> Option<(u64, u64)> {
    let handle = open(pid, PROCESS_QUERY_LIMITED_INFORMATION).ok()?;
    let mut process = 0usize;
    let mut system = 0usize;
    unsafe { GetProcessAffinityMask(handle.0, &mut process, &mut system).ok()? };
    Some((process as u64, system as u64))
}

fn version_cache() -> &'static Mutex<HashMap<String, (String, String)>> {
    static CACHE: OnceLock<Mutex<HashMap<String, (String, String)>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// `(description, company)` from a program file, e.g. `("Google Chrome", "Google LLC")`.
/// Empty strings when the file carries none.
pub fn version_info(path: &str) -> (String, String) {
    if path.is_empty() {
        return Default::default();
    }
    let key = path.to_lowercase();
    if let Some(hit) = version_cache().lock().ok().and_then(|c| c.get(&key).cloned()) {
        return hit;
    }
    let made = read_version_info(path).unwrap_or_default();
    if let Ok(mut cache) = version_cache().lock() {
        cache.insert(key, made.clone());
    }
    made
}

fn read_version_info(path: &str) -> Option<(String, String)> {
    let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let size = GetFileVersionInfoSizeW(PCWSTR(wide.as_ptr()), None);
        if size == 0 {
            return None;
        }
        let mut data = vec![0u8; size as usize];
        GetFileVersionInfoW(PCWSTR(wide.as_ptr()), None, size, data.as_mut_ptr() as *mut c_void).ok()?;

        let query = |sub: &str| -> Option<(*const c_void, u32)> {
            let sub_w: Vec<u16> = sub.encode_utf16().chain(std::iter::once(0)).collect();
            let mut ptr: *mut c_void = std::ptr::null_mut();
            let mut len = 0u32;
            let ok: BOOL = VerQueryValueW(data.as_ptr() as *const c_void, PCWSTR(sub_w.as_ptr()), &mut ptr, &mut len);
            (ok.as_bool() && !ptr.is_null() && len > 0).then_some((ptr as *const c_void, len))
        };

        // Which language block the file has; fall back to US English / Unicode.
        let (lang, codepage) = match query("\\VarFileInfo\\Translation") {
            Some((ptr, len)) if len >= 4 => {
                let pair = std::slice::from_raw_parts(ptr as *const u16, 2);
                (pair[0], pair[1])
            }
            _ => (0x0409, 0x04B0),
        };
        let text = |name: &str| -> String {
            query(&format!("\\StringFileInfo\\{lang:04x}{codepage:04x}\\{name}"))
                .map(|(ptr, len)| {
                    let chars = std::slice::from_raw_parts(ptr as *const u16, len as usize);
                    let end = chars.iter().position(|c| *c == 0).unwrap_or(chars.len());
                    String::from_utf16_lossy(&chars[..end]).trim().to_string()
                })
                .unwrap_or_default()
        };
        Some((text("FileDescription"), text("CompanyName")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn our_own_process_is_fully_readable() {
        let me = std::process::id();
        let path = exe_path(me).expect("own path");
        assert!(path.to_lowercase().ends_with(".exe"));
        assert!(command_line(me).expect("own command line").contains("fizz"), "the test binary name is in it");
        assert!(!user_of(me).expect("own user").is_empty());
        assert_eq!(user_of(me), user_of(me), "second lookup comes from the remembered names");
        assert!(elevated(me).is_some());
        assert_eq!(architecture(me), "64-bit");
        assert_eq!(priority_of(me), "normal");
        let (mine, system) = affinity_of(me).expect("own affinity");
        assert!(mine != 0 && mine & !system == 0, "only processors the PC has");
    }

    #[test]
    fn a_windows_device_path_becomes_a_drive_path() {
        let drives = vec![
            ("C:".to_string(), r"\Device\HarddiskVolume3".to_string()),
            ("D:".to_string(), r"\Device\HarddiskVolume30".to_string()),
        ];
        let to = |nt: &str| nt_to_drive_path(nt, &drives);
        assert_eq!(to(r"\Device\HarddiskVolume3\Windows\System32\csrss.exe").as_deref(), Some(r"C:\Windows\System32\csrss.exe"));
        // Volume30 must not be mistaken for Volume3 plus a folder called "0".
        assert_eq!(to(r"\Device\HarddiskVolume30\Games\a.exe").as_deref(), Some(r"D:\Games\a.exe"));
        assert_eq!(to(r"\device\harddiskvolume3\x.exe").as_deref(), Some(r"C:\x.exe"), "case does not matter");
        assert_eq!(to(r"\Device\Mup\server\share\a.exe"), None, "a network path has no drive");
        assert_eq!(to(""), None);
    }

    /// Most system processes cannot be opened without administrator rights. Their program file
    /// (and so their icon) must still be found.
    #[test]
    fn a_system_process_we_cannot_open_still_has_a_program_file() {
        let mut engine = crate::proclist::Engine::new(1.0);
        let rows = engine.read(&HashMap::new(), true);
        let csrss = rows.iter().find(|r| r.name.eq_ignore_ascii_case("csrss.exe")).expect("csrss.exe always runs");
        let path = exe_path(csrss.pid).expect("csrss.exe has a program file");
        assert!(path.to_lowercase().ends_with(r"\system32\csrss.exe"), "got {path}");
        assert!(std::path::Path::new(&path).exists(), "{path} is a real file");
        let me = std::process::id();
        assert_eq!(
            exe_path_without_opening(me).map(|p| p.to_lowercase()),
            exe_path_by_opening(me).map(|p| p.to_lowercase()),
            "both ways agree"
        );
    }

    #[test]
    fn a_process_that_does_not_exist_gives_nothing_not_an_error() {
        let ghost = 4_000_000_001;
        assert_eq!(open(ghost, PROCESS_QUERY_LIMITED_INFORMATION).err(), Some(87));
        assert!(exe_path(ghost).is_none());
        assert!(command_line(ghost).is_none());
        assert!(user_of(ghost).is_none());
        assert!(elevated(ghost).is_none());
        assert_eq!(architecture(ghost), "");
        assert_eq!(priority_of(ghost), "");
        assert!(affinity_of(ghost).is_none());
    }

    #[test]
    fn windows_programs_carry_a_description_and_company() {
        let windir = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into());
        let (description, company) = version_info(&format!(r"{windir}\explorer.exe"));
        assert_eq!(description, "Windows Explorer");
        assert!(company.contains("Microsoft"));
        assert_eq!(version_info(r"C:\no\such\file.exe"), (String::new(), String::new()));
        assert_eq!(version_info(""), (String::new(), String::new()));
    }

    #[test]
    fn priority_names_round_trip() {
        for (name, class) in PRIORITIES {
            assert_eq!(priority_name(class), name);
            assert_eq!(priority_class(name), Some(class));
        }
        assert_eq!(priority_name(0), "");
        assert_eq!(priority_class("turbo"), None);
    }

    #[test]
    fn machine_kinds() {
        assert_eq!(machine_name(0, 0x8664), "64-bit");
        assert_eq!(machine_name(0x014c, 0x8664), "32-bit", "a 32-bit program on 64-bit Windows");
        assert_eq!(machine_name(0, 0xAA64), "ARM64");
        assert_eq!(machine_name(0x9999, 0x8664), "");
    }
}
