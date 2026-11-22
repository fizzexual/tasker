//! Startup apps: what starts with Windows, and switching entries on or off.
//!
//! We use the same switch Windows Task Manager and Settings use (the `StartupApproved`
//! registry keys), so an entry turned off here shows as off there too, and nothing is deleted.

use crate::error::{CmdError, CmdResult};
use serde::Serialize;
use std::path::{Path, PathBuf};
use winreg::enums::*;
use winreg::{RegKey, RegValue};

const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_WOW: &str = r"Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Run";
const APPROVED: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved";

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct StartupEntry {
    /// `<source>|<name>`; passed back to switch the entry on or off.
    pub id: String,
    pub name: String,
    pub command: String,
    /// Plain-language place it comes from.
    pub location: &'static str,
    pub enabled: bool,
    /// True when switching it needs administrator rights (it applies to every user).
    pub all_users: bool,
    /// The program file it starts, when it could be worked out.
    pub path: String,
    pub publisher: String,
}

/// Where a kind of startup entry lives, and where its on/off switch lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    UserRun,
    MachineRun,
    MachineRun32,
    UserFolder,
    CommonFolder,
}

impl Source {
    pub const ALL: [Source; 5] =
        [Source::UserRun, Source::MachineRun, Source::MachineRun32, Source::UserFolder, Source::CommonFolder];

    pub fn key(self) -> &'static str {
        match self {
            Source::UserRun => "user-run",
            Source::MachineRun => "machine-run",
            Source::MachineRun32 => "machine-run32",
            Source::UserFolder => "user-folder",
            Source::CommonFolder => "common-folder",
        }
    }
    pub fn from_key(key: &str) -> Option<Source> {
        Source::ALL.into_iter().find(|s| s.key() == key)
    }
    fn location(self) -> &'static str {
        match self {
            Source::UserRun => "Registry (this user)",
            Source::MachineRun | Source::MachineRun32 => "Registry (all users)",
            Source::UserFolder => "Startup folder (this user)",
            Source::CommonFolder => "Startup folder (all users)",
        }
    }
    fn all_users(self) -> bool {
        !matches!(self, Source::UserRun | Source::UserFolder)
    }
    fn approved_subkey(self) -> &'static str {
        match self {
            Source::UserRun | Source::MachineRun => "Run",
            Source::MachineRun32 => "Run32",
            Source::UserFolder | Source::CommonFolder => "StartupFolder",
        }
    }
}

/// The registry roots and folders to read. Production uses the real ones; tests point this
/// at a private scratch key and a temp folder so no real startup entry is ever touched.
pub struct Roots {
    pub user: RegKey,
    pub machine: RegKey,
    /// Prefix put in front of every registry path (empty in production).
    pub prefix: String,
    pub user_folder: Option<PathBuf>,
    pub common_folder: Option<PathBuf>,
}

impl Roots {
    pub fn real() -> Self {
        let user_folder = std::env::var_os("APPDATA")
            .map(|p| PathBuf::from(p).join(r"Microsoft\Windows\Start Menu\Programs\Startup"));
        let common_folder = std::env::var_os("ProgramData")
            .map(|p| PathBuf::from(p).join(r"Microsoft\Windows\Start Menu\Programs\Startup"));
        Roots {
            user: RegKey::predef(HKEY_CURRENT_USER),
            machine: RegKey::predef(HKEY_LOCAL_MACHINE),
            prefix: String::new(),
            user_folder,
            common_folder,
        }
    }

    fn root(&self, source: Source) -> &RegKey {
        if source.all_users() { &self.machine } else { &self.user }
    }
    fn path(&self, tail: &str) -> String {
        format!("{}{}", self.prefix, tail)
    }
    fn approved_path(&self, source: Source) -> String {
        self.path(&format!(r"{APPROVED}\{}", source.approved_subkey()))
    }
    fn folder(&self, source: Source) -> Option<&Path> {
        match source {
            Source::UserFolder => self.user_folder.as_deref(),
            Source::CommonFolder => self.common_folder.as_deref(),
            _ => None,
        }
    }
}

/// Windows stores the switch as 12 bytes: a flag, then the time it was switched off.
/// An even flag (2 or 6) means on, an odd flag (3 or 7) means off. No value means on.
pub fn is_enabled(approved: Option<&[u8]>) -> bool {
    match approved {
        Some(bytes) if !bytes.is_empty() => bytes[0] & 1 == 0,
        _ => true,
    }
}

/// Builds the 12-byte switch value. `filetime` is only stored when switching off.
pub fn approved_value(enabled: bool, filetime: u64) -> [u8; 12] {
    let mut v = [0u8; 12];
    if enabled {
        v[0] = 2;
    } else {
        v[0] = 3;
        v[4..12].copy_from_slice(&filetime.to_le_bytes());
    }
    v
}

fn now_filetime() -> u64 {
    // FILETIME counts 100 ns steps since 1601-01-01; the Unix epoch is 11_644_473_600 s later.
    let unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() / 100)
        .unwrap_or(0) as u64;
    unix + 116_444_736_000_000_000
}

fn read_approved(roots: &Roots, source: Source, name: &str) -> Option<Vec<u8>> {
    let key = roots.root(source).open_subkey_with_flags(roots.approved_path(source), KEY_READ).ok()?;
    key.get_raw_value(name).ok().map(|v| v.bytes.into_owned())
}

fn run_path(roots: &Roots, source: Source) -> Option<String> {
    match source {
        Source::UserRun | Source::MachineRun => Some(roots.path(RUN)),
        Source::MachineRun32 => Some(roots.path(RUN_WOW)),
        _ => None,
    }
}

/// Where Windows keeps the start-up switches of Store apps.
const STORE: &str = r"Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppModel\SystemAppData";
const STORE_KEY: &str = "store";

/// `SpotifyAB.SpotifyMusic_zpdnekdrzrea0` becomes `Spotify Music`.
pub fn store_app_name(package: &str) -> String {
    let family = package.split('_').next().unwrap_or(package);
    let last = family.rsplit('.').next().unwrap_or(family);
    let mut out = String::new();
    let mut previous_lower = false;
    for c in last.chars() {
        if c.is_uppercase() && previous_lower {
            out.push(' ');
        }
        previous_lower = c.is_lowercase() || c.is_ascii_digit();
        out.push(c);
    }
    if out.is_empty() {
        package.to_string()
    } else {
        out
    }
}

/// Store apps: on when the state is 2 (on) or 4 (on by company policy).
pub fn store_enabled(state: u32) -> bool {
    state == 2 || state == 4
}

fn store_entries(roots: &Roots) -> Vec<StartupEntry> {
    let mut out = Vec::new();
    let Ok(apps) = roots.user.open_subkey_with_flags(roots.path(STORE), KEY_READ) else { return out };
    for package in apps.enum_keys().flatten() {
        let Ok(app) = apps.open_subkey_with_flags(&package, KEY_READ) else { continue };
        for task in app.enum_keys().flatten() {
            let Ok(key) = app.open_subkey_with_flags(&task, KEY_READ) else { continue };
            let Ok(state) = key.get_value::<u32, _>("State") else { continue };
            out.push(StartupEntry {
                id: format!(r"{STORE_KEY}|{package}\{task}"),
                name: store_app_name(&package),
                command: package.clone(),
                location: "Store app",
                enabled: store_enabled(state),
                all_users: false,
                path: String::new(),
                publisher: String::new(),
            });
        }
    }
    out
}

fn set_store_enabled(roots: &Roots, name: &str, enabled: bool) -> CmdResult<bool> {
    let what = "Changing a startup entry";
    let key = roots
        .user
        .open_subkey_with_flags(roots.path(&format!(r"{STORE}\{name}")), KEY_READ | KEY_SET_VALUE)
        .map_err(|e| io_error(&e, what))?;
    // 2 = on, 1 = switched off by the user (the same values Windows Settings writes).
    key.set_value("State", &if enabled { 2u32 } else { 1u32 }).map_err(|e| io_error(&e, what))?;
    Ok(store_enabled(key.get_value::<u32, _>("State").map_err(|e| io_error(&e, what))?))
}

/// Replaces `%NAME%` with the value of that environment variable (unknown names are kept).
pub fn expand_env(text: &str, lookup: impl Fn(&str) -> Option<String>) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) => {
                match lookup(&after[..end]) {
                    Some(value) => out.push_str(&value),
                    None => {
                        out.push('%');
                        out.push_str(&after[..end]);
                        out.push('%');
                    }
                }
                rest = &after[end + 1..];
            }
            None => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// The program file a start-up command runs, or empty when it cannot be told.
fn program_of(command: &str) -> String {
    let expanded = expand_env(command, |name| std::env::var(name).ok());
    crate::actions::split_command(&expanded, |p| Path::new(p).is_file())
        .map(|(file, _)| file)
        .filter(|file| Path::new(file).is_file())
        .unwrap_or_default()
}

pub fn list(roots: &Roots) -> Vec<StartupEntry> {
    let mut out = store_entries(roots);
    for source in Source::ALL {
        if let Some(path) = run_path(roots, source) {
            let Ok(key) = roots.root(source).open_subkey_with_flags(&path, KEY_READ) else { continue };
            for (name, value) in key.enum_values().flatten() {
                if name.is_empty() {
                    continue;
                }
                let command = match value.vtype {
                    REG_SZ | REG_EXPAND_SZ => String::from_reg_value_lossy(&value),
                    _ => continue,
                };
                out.push(entry(roots, source, name, command));
            }
        } else if let Some(folder) = roots.folder(source) {
            let Ok(dir) = std::fs::read_dir(folder) else { continue };
            for file in dir.flatten() {
                let file_name = file.file_name().to_string_lossy().into_owned();
                if file_name.eq_ignore_ascii_case("desktop.ini") || !file.path().is_file() {
                    continue;
                }
                out.push(entry(roots, source, file_name, file.path().to_string_lossy().into_owned()));
            }
        }
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out
}

fn entry(roots: &Roots, source: Source, name: String, command: String) -> StartupEntry {
    let enabled = is_enabled(read_approved(roots, source, &name).as_deref());
    let path = program_of(&command);
    let publisher = crate::procinfo::version_info(&path).1;
    StartupEntry {
        path,
        publisher,
        id: format!("{}|{}", source.key(), name),
        name,
        command,
        location: source.location(),
        enabled,
        all_users: source.all_users(),
    }
}

/// Small helper trait so a broken registry string never aborts the whole list.
trait LossyString {
    fn from_reg_value_lossy(value: &RegValue) -> String;
}
impl LossyString for String {
    fn from_reg_value_lossy(value: &RegValue) -> String {
        let wide: Vec<u16> = value
            .bytes
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .take_while(|c| *c != 0)
            .collect();
        String::from_utf16_lossy(&wide)
    }
}

pub fn parse_id(id: &str) -> CmdResult<(Source, &str)> {
    let (key, name) = id.split_once('|').ok_or_else(|| CmdError::invalid("Unknown startup entry."))?;
    let source = Source::from_key(key).ok_or_else(|| CmdError::invalid("Unknown startup entry."))?;
    if name.is_empty() {
        return Err(CmdError::invalid("Unknown startup entry."));
    }
    Ok((source, name))
}

/// Switches one entry on or off and returns its new state as read back from Windows.
pub fn set_enabled(roots: &Roots, id: &str, enabled: bool) -> CmdResult<bool> {
    // Refuse to write a switch for an entry that is not there any more.
    let exists = |id: &str| list(roots).iter().any(|e| e.id == id);
    if let Some(name) = id.strip_prefix("store|") {
        if name.is_empty() || !exists(id) {
            return Err(CmdError::not_found("That startup entry"));
        }
        return set_store_enabled(roots, name, enabled);
    }
    let (source, name) = parse_id(id)?;
    if !exists(id) {
        return Err(CmdError::not_found("That startup entry"));
    }

    let (key, _) = roots
        .root(source)
        .create_subkey_with_flags(roots.approved_path(source), KEY_READ | KEY_SET_VALUE)
        .map_err(|e| io_error(&e, "Changing a startup entry for all users"))?;
    let value = RegValue { vtype: REG_BINARY, bytes: approved_value(enabled, now_filetime()).to_vec().into() };
    key.set_raw_value(name, &value).map_err(|e| io_error(&e, "Changing a startup entry for all users"))?;

    Ok(is_enabled(read_approved(roots, source, name).as_deref()))
}

/// The name this app's own "start with Windows" entry carried before the app was renamed.
pub const OLD_APP_NAME: &str = "Fizz Task Manager";

/// Clears the entry an earlier name of this app left behind, together with its on/off switch.
/// `None`: there was none. `Some(on)`: there was one, switched on or off, so the caller registers
/// the app under its present name only when the owner had it on.
pub fn take_old_own_entry(run: &RegKey, approved: Option<&RegKey>, old_name: &str) -> Option<bool> {
    run.get_raw_value(old_name).ok()?;
    let switch = approved.and_then(|key| key.get_raw_value(old_name).ok());
    let on = is_enabled(switch.as_ref().map(|v| &v.bytes[..]));
    run.delete_value(old_name).ok()?;
    if let Some(key) = approved {
        let _ = key.delete_value(old_name);
    }
    Some(on)
}

/// The same, on this user's real start-up list. Touches only the entry named [`OLD_APP_NAME`].
pub fn take_old_autostart() -> Option<bool> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let run = hkcu.open_subkey_with_flags(RUN, KEY_READ | KEY_SET_VALUE).ok()?;
    let approved = hkcu.open_subkey_with_flags(format!(r"{APPROVED}\Run"), KEY_READ | KEY_SET_VALUE).ok();
    take_old_own_entry(&run, approved.as_ref(), OLD_APP_NAME)
}

fn io_error(e: &std::io::Error, what: &str) -> CmdError {
    match e.raw_os_error() {
        Some(code) => CmdError::from_win32(code as u32, what),
        None => CmdError::failed(format!("{what} failed: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A private registry area and temp folder that is removed when the test ends.
    struct Scratch {
        roots: Roots,
        key_path: String,
        dir: PathBuf,
    }

    impl Scratch {
        fn new(tag: &str) -> Scratch {
            let unique = format!("{tag}-{}-{:?}", std::process::id(), std::thread::current().id());
            let key_path = format!(r"Software\TaskerTest\{unique}");
            let dir = std::env::temp_dir().join(format!("ftm-startup-{unique}"));
            std::fs::create_dir_all(dir.join("user")).unwrap();
            std::fs::create_dir_all(dir.join("common")).unwrap();
            let hkcu = RegKey::predef(HKEY_CURRENT_USER);
            // "user" and "machine" both live under HKCU in tests: no admin, nothing real touched.
            let (user, _) = hkcu.create_subkey(format!(r"{key_path}\user")).unwrap();
            let (machine, _) = hkcu.create_subkey(format!(r"{key_path}\machine")).unwrap();
            Scratch {
                roots: Roots {
                    user,
                    machine,
                    prefix: String::new(),
                    user_folder: Some(dir.join("user")),
                    common_folder: Some(dir.join("common")),
                },
                key_path,
                dir,
            }
        }
        fn add_run(&self, source: Source, name: &str, command: &str) {
            let path = run_path(&self.roots, source).unwrap();
            let (key, _) = self.roots.root(source).create_subkey(path).unwrap();
            key.set_value(name, &command.to_string()).unwrap();
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = RegKey::predef(HKEY_CURRENT_USER).delete_subkey_all(&self.key_path);
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    /// Two empty keys inside the scratch area, standing in for the Run list and its switches.
    fn old_name_keys(s: &Scratch) -> (RegKey, RegKey) {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let (run, _) = hkcu.create_subkey(format!(r"{}\oldrun", s.key_path)).unwrap();
        let (approved, _) = hkcu.create_subkey(format!(r"{}\oldapproved", s.key_path)).unwrap();
        (run, approved)
    }

    #[test]
    fn the_entry_under_the_old_app_name_is_cleared_once() {
        let s = Scratch::new("oldname");
        let (run, approved) = old_name_keys(&s);
        run.set_value("Other App", &"other.exe".to_string()).unwrap();

        assert_eq!(take_old_own_entry(&run, Some(&approved), OLD_APP_NAME), None, "nothing was registered under the old name");

        run.set_value(OLD_APP_NAME, &r"C:\old\Old.exe --hidden".to_string()).unwrap();
        assert_eq!(take_old_own_entry(&run, Some(&approved), OLD_APP_NAME), Some(true), "no switch stored means it was on");
        assert!(run.get_raw_value(OLD_APP_NAME).is_err(), "the old entry is gone");
        assert_eq!(take_old_own_entry(&run, Some(&approved), OLD_APP_NAME), None, "the next start finds nothing to do");

        let other: String = run.get_value("Other App").unwrap();
        assert_eq!(other, "other.exe", "another program's entry is never touched");
    }

    #[test]
    fn an_old_entry_the_owner_had_switched_off_is_reported_as_off() {
        let s = Scratch::new("oldoff");
        let (run, approved) = old_name_keys(&s);
        run.set_value(OLD_APP_NAME, &r"C:\old\Old.exe --hidden".to_string()).unwrap();
        let off = RegValue { vtype: REG_BINARY, bytes: approved_value(false, 1).to_vec().into() };
        approved.set_raw_value(OLD_APP_NAME, &off).unwrap();

        assert_eq!(take_old_own_entry(&run, Some(&approved), OLD_APP_NAME), Some(false), "off stays off: the caller must not switch start-up on");
        assert!(run.get_raw_value(OLD_APP_NAME).is_err(), "the old entry is gone");
        assert!(approved.get_raw_value(OLD_APP_NAME).is_err(), "and so is its switch");
    }

    #[test]
    fn switch_value_meaning() {
        assert!(is_enabled(None), "no switch stored means the entry runs");
        assert!(is_enabled(Some(&[2, 0, 0, 0])));
        assert!(is_enabled(Some(&[6, 0, 0, 0])));
        assert!(!is_enabled(Some(&[3, 0, 0, 0])));
        assert!(!is_enabled(Some(&[7, 0, 0, 0])));
        assert!(is_enabled(Some(&[])), "an empty value is treated like no value");
    }

    #[test]
    fn switch_value_layout_matches_windows() {
        assert_eq!(approved_value(true, 123), [2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        let off = approved_value(false, 0x0102_0304_0506_0708);
        assert_eq!(off[0], 3);
        assert_eq!(&off[4..], &[8, 7, 6, 5, 4, 3, 2, 1]);
    }

    #[test]
    fn lists_registry_and_folder_entries() {
        let s = Scratch::new("list");
        s.add_run(Source::UserRun, "Zeta", r#""C:\z\z.exe" --tray"#);
        s.add_run(Source::MachineRun, "alpha", r"C:\a\a.exe");
        s.add_run(Source::MachineRun32, "Beta32", r"C:\b\b.exe");
        std::fs::write(s.dir.join("user").join("Note.lnk"), b"x").unwrap();
        std::fs::write(s.dir.join("user").join("desktop.ini"), b"x").unwrap();

        let entries = list(&s.roots);
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["alpha", "Beta32", "Note.lnk", "Zeta"], "sorted, desktop.ini hidden");
        assert!(entries.iter().all(|e| e.enabled));
        let alpha = &entries[0];
        assert!(alpha.all_users);
        assert_eq!(alpha.id, "machine-run|alpha");
        let zeta = &entries[3];
        assert!(!zeta.all_users);
        assert_eq!(zeta.command, r#""C:\z\z.exe" --tray"#);
    }

    #[test]
    fn switching_off_and_on_round_trips_and_keeps_the_entry() {
        let s = Scratch::new("toggle");
        s.add_run(Source::UserRun, "App", r"C:\app.exe");

        assert_eq!(set_enabled(&s.roots, "user-run|App", false), Ok(false));
        let after_off = list(&s.roots);
        assert_eq!(after_off.len(), 1, "switching off never deletes the entry");
        assert!(!after_off[0].enabled);

        assert_eq!(set_enabled(&s.roots, "user-run|App", true), Ok(true));
        assert!(list(&s.roots)[0].enabled);
    }

    #[test]
    fn run32_and_run_switches_do_not_collide() {
        let s = Scratch::new("run32");
        s.add_run(Source::MachineRun, "Same", r"C:\64.exe");
        s.add_run(Source::MachineRun32, "Same", r"C:\32.exe");
        set_enabled(&s.roots, "machine-run32|Same", false).unwrap();
        let entries = list(&s.roots);
        let state = |id: &str| entries.iter().find(|e| e.id == id).unwrap().enabled;
        assert!(state("machine-run|Same"));
        assert!(!state("machine-run32|Same"));
    }

    #[test]
    fn folder_entry_switch_uses_the_file_name() {
        let s = Scratch::new("folder");
        std::fs::write(s.dir.join("common").join("Tool.lnk"), b"x").unwrap();
        assert_eq!(set_enabled(&s.roots, "common-folder|Tool.lnk", false), Ok(false));
        assert!(!list(&s.roots)[0].enabled);
    }

    #[test]
    fn missing_entry_is_refused_and_writes_nothing() {
        let s = Scratch::new("missing");
        let err = set_enabled(&s.roots, "user-run|Ghost", false).unwrap_err();
        assert_eq!(err.code, "not-found");
        assert!(read_approved(&s.roots, Source::UserRun, "Ghost").is_none());
    }

    #[test]
    fn bad_ids_are_refused() {
        let s = Scratch::new("bad");
        for id in ["", "nope", "user-run|", "weird|App", "|App"] {
            assert_eq!(set_enabled(&s.roots, id, false).unwrap_err().code, "invalid", "id {id:?}");
        }
    }

    #[test]
    fn read_only_key_reports_needs_admin() {
        // The same error Windows gives a normal user writing under HKEY_LOCAL_MACHINE.
        let err = io_error(&std::io::Error::from_raw_os_error(5), "Changing a startup entry for all users");
        assert_eq!(err.code, "needs-admin");
    }

    #[test]
    fn store_app_names_are_made_readable() {
        assert_eq!(store_app_name("SpotifyAB.SpotifyMusic_zpdnekdrzrea0"), "Spotify Music");
        assert_eq!(store_app_name("Microsoft.YourPhone_8wekyb3d8bbwe"), "Your Phone");
        assert_eq!(store_app_name("MSTeams_8wekyb3d8bbwe"), "MSTeams");
        assert_eq!(store_app_name(""), "");
        assert!(store_enabled(2) && store_enabled(4));
        assert!(!store_enabled(0) && !store_enabled(1) && !store_enabled(3));
    }

    #[test]
    fn environment_names_in_commands_are_filled_in() {
        let lookup = |name: &str| (name == "windir").then(|| r"C:\Windows".to_string());
        assert_eq!(expand_env(r"%windir%\system32\x.exe /a", lookup), r"C:\Windows\system32\x.exe /a");
        assert_eq!(expand_env("%nope%\\x", lookup), "%nope%\\x", "unknown names stay as written");
        assert_eq!(expand_env("100% sure", lookup), "100% sure");
        assert_eq!(expand_env("", lookup), "");
    }

    #[test]
    fn the_program_and_publisher_of_a_command_are_found() {
        assert!(program_of(r"%windir%\explorer.exe /factory").to_lowercase().ends_with(r"\explorer.exe"));
        assert_eq!(program_of(r#""C:\no\such\app.exe" -x"#), "", "a program that is gone gives nothing");
        assert_eq!(program_of(""), "");
    }

    #[test]
    fn store_apps_are_listed_and_switched_in_the_scratch_key() {
        let t = Scratch::new("store");
        let (task, _) = t
            .roots
            .user
            .create_subkey(t.roots.path(&format!(r"{STORE}\Vendor.NiceApp_abc123\NiceAppStartupId")))
            .unwrap();
        task.set_value("State", &2u32).unwrap();
        // A sub-key without a State value is not a start-up task.
        t.roots.user.create_subkey(t.roots.path(&format!(r"{STORE}\Vendor.NiceApp_abc123\Other"))).unwrap();

        let id = r"store|Vendor.NiceApp_abc123\NiceAppStartupId";
        let found: Vec<_> = list(&t.roots).into_iter().filter(|e| e.location == "Store app").collect();
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].id.as_str(), found[0].name.as_str(), found[0].enabled), (id, "Nice App", true));

        assert_eq!(set_enabled(&t.roots, id, false), Ok(false));
        assert_eq!(task.get_value::<u32, _>("State").unwrap(), 1);
        assert_eq!(set_enabled(&t.roots, id, true), Ok(true));
        assert_eq!(task.get_value::<u32, _>("State").unwrap(), 2);

        assert_eq!(set_enabled(&t.roots, r"store|Vendor.NiceApp_abc123\Other", false).unwrap_err().code, "not-found");
        assert_eq!(set_enabled(&t.roots, "store|", false).unwrap_err().code, "not-found");
        assert!(t.roots.user.open_subkey(t.roots.path(&format!(r"{STORE}\Vendor.NiceApp_abc123\Other"))).unwrap().get_value::<u32, _>("State").is_err(), "nothing was written for the refused entry");
    }
}
