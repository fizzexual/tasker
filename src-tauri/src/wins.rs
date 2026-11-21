//! Open windows: which programs have one (the "Apps" group in Windows Task Manager), what
//! its title is, and whether it has stopped answering ("Not responding").

use std::collections::HashMap;
use windows::core::{w, BOOL};
use windows::Win32::Foundation::{HWND, LPARAM};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, FindWindowExW, GetClassNameW, GetShellWindow, GetWindow, GetWindowLongPtrW, GetWindowTextW,
    GetWindowThreadProcessId, IsHungAppWindow, IsIconic, IsWindowVisible, SetForegroundWindow, ShowWindow,
    GWL_EXSTYLE, GW_OWNER, SW_RESTORE, WS_EX_TOOLWINDOW,
};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct AppWindow {
    /// The window on top for this program.
    pub hwnd: isize,
    pub title: String,
    /// Any of the program's windows has stopped answering.
    pub hung: bool,
    pub count: u32,
}

/// What Windows says about one top-level window.
#[derive(Debug, Clone, Copy, Default)]
pub struct Facts {
    pub visible: bool,
    pub owned: bool,
    pub tool: bool,
    pub cloaked: bool,
    pub has_title: bool,
    pub is_desktop: bool,
}

/// A window the user would call "an open app": on screen, with a title, standing on its own.
/// Tool palettes, hidden helper windows, windows parked on another virtual desktop's
/// background ("cloaked" Store app shells) and the desktop itself do not count.
pub fn is_app_window(f: Facts) -> bool {
    f.visible && f.has_title && !f.owned && !f.tool && !f.cloaked && !f.is_desktop
}

fn pid_of(hwnd: HWND) -> u32 {
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    pid
}

fn class_of(hwnd: HWND) -> String {
    let mut buf = [0u16; 128];
    let len = unsafe { GetClassNameW(hwnd, &mut buf) }.max(0) as usize;
    String::from_utf16_lossy(&buf[..len])
}

struct Scan {
    desktop: HWND,
    found: HashMap<u32, AppWindow>,
}

unsafe extern "system" fn each(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let scan = &mut *(lparam.0 as *mut Scan);
    let mut title = [0u16; 256];
    let title_len = GetWindowTextW(hwnd, &mut title).max(0) as usize;
    let mut cloaked = 0u32;
    let _ = DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, &mut cloaked as *mut u32 as *mut _, 4);
    let facts = Facts {
        visible: IsWindowVisible(hwnd).as_bool(),
        owned: GetWindow(hwnd, GW_OWNER).map(|o| !o.is_invalid()).unwrap_or(false),
        tool: GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW.0 != 0,
        cloaked: cloaked != 0,
        has_title: title_len > 0,
        is_desktop: hwnd == scan.desktop,
    };
    if !is_app_window(facts) {
        return BOOL(1);
    }
    // A Store app's window belongs to a shared host process; the app itself owns the
    // content window inside it. Credit the app, as Windows Task Manager does.
    let mut pid = pid_of(hwnd);
    if class_of(hwnd) == "ApplicationFrameWindow" {
        if let Ok(core) = FindWindowExW(Some(hwnd), None, w!("Windows.UI.Core.CoreWindow"), None) {
            let inner = pid_of(core);
            if inner != 0 {
                pid = inner;
            }
        }
    }
    if pid == 0 {
        return BOOL(1);
    }
    let hung = IsHungAppWindow(hwnd).as_bool();
    let entry = scan.found.entry(pid).or_insert_with(|| AppWindow {
        hwnd: hwnd.0 as isize,
        title: String::from_utf16_lossy(&title[..title_len]),
        hung: false,
        count: 0,
    });
    entry.count += 1;
    entry.hung |= hung;
    BOOL(1)
}

/// Every program that has an open window, by process id. Windows are visited front to back,
/// so the remembered window is the one on top.
pub fn app_windows() -> HashMap<u32, AppWindow> {
    let mut scan = Scan { desktop: unsafe { GetShellWindow() }, found: HashMap::new() };
    unsafe {
        let _ = EnumWindows(Some(each), LPARAM(&mut scan as *mut Scan as isize));
    }
    scan.found
}

/// Process that draws the desktop and taskbar (0 when there is none).
pub fn shell_pid() -> u32 {
    let desktop = unsafe { GetShellWindow() };
    if desktop.is_invalid() {
        0
    } else {
        pid_of(desktop)
    }
}

/// Brings a program's top window to the front. `false` when it has no window.
pub fn switch_to(pid: u32) -> bool {
    let Some(window) = app_windows().remove(&pid) else { return false };
    let hwnd = HWND(window.hwnd as *mut _);
    unsafe {
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }
        SetForegroundWindow(hwnd).as_bool()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const APP: Facts =
        Facts { visible: true, owned: false, tool: false, cloaked: false, has_title: true, is_desktop: false };

    #[test]
    fn a_normal_titled_window_is_an_app() {
        assert!(is_app_window(APP));
    }

    #[test]
    fn helper_windows_are_not_apps() {
        assert!(!is_app_window(Facts { visible: false, ..APP }), "hidden");
        assert!(!is_app_window(Facts { owned: true, ..APP }), "a dialog owned by another window");
        assert!(!is_app_window(Facts { tool: true, ..APP }), "a tool palette");
        assert!(!is_app_window(Facts { cloaked: true, ..APP }), "a sleeping Store app shell");
        assert!(!is_app_window(Facts { has_title: false, ..APP }), "no title");
        assert!(!is_app_window(Facts { is_desktop: true, ..APP }), "the desktop itself");
    }

    #[test]
    fn the_scan_runs_and_never_lists_the_desktop() {
        let found = app_windows();
        let shell = unsafe { GetShellWindow() };
        assert!(found.values().all(|w| w.hwnd != shell.0 as isize));
        assert!(found.values().all(|w| !w.title.is_empty() && w.count >= 1));
    }

    #[test]
    fn switching_to_a_process_without_a_window_does_nothing() {
        assert!(!switch_to(4_000_000_001));
    }
}
