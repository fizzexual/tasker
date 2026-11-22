//! The window has no Windows frame (the app draws its own title bar). One thing the frame did
//! for free is lost with it: under a maximised window, a taskbar set to hide itself could still
//! slide up. A frameless window that covers the whole screen counts as a full-screen program,
//! and Windows keeps the taskbar down for those.
//!
//! The window library has a guard for this, but it asks Windows the single-screen question
//! (`ABM_GETAUTOHIDEBAR`), which answered "no hidden taskbar" on a three-screen PC whose taskbar
//! was hidden at the bottom of the main screen (measured 2026-11-22: maximised page 1920x1080 on
//! a 1920x1080 screen). So Tasker looks at where the taskbar windows really are and leaves one
//! pixel free on that edge.

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromRect, MONITORINFO, MONITOR_DEFAULTTONEAREST};
use windows::Win32::UI::Shell::{
    DefSubclassProc, RemoveWindowSubclass, SHAppBarMessage, SetWindowSubclass, ABM_GETSTATE, ABS_AUTOHIDE, APPBARDATA,
};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowExW, GetWindowRect, IsZoomed, SetWindowPos, NCCALCSIZE_PARAMS, SWP_FRAMECHANGED, SWP_NOACTIVATE,
    SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, WM_NCCALCSIZE, WM_NCDESTROY,
};

/// A rectangle in screen pixels (left/top inside, right/bottom outside).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Area {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Left,
    Top,
    Right,
    Bottom,
}

/// The edge of `screen` that a taskbar at `bar` lives on, or `None` when it is another screen's.
///
/// A taskbar that is showing lies wholly inside its screen. A hidden one hangs off its edge with
/// a sliver of a few pixels left on its own screen; the part that hangs off may lie over a
/// neighbouring screen, and that neighbour must not take it for its own.
pub fn bar_edge(screen: Area, bar: Area) -> Option<Edge> {
    let (wide, tall) = (bar.right - bar.left, bar.bottom - bar.top);
    if wide <= 0 || tall <= 0 {
        return None;
    }
    let shared_w = bar.right.min(screen.right) - bar.left.max(screen.left);
    let shared_h = bar.bottom.min(screen.bottom) - bar.top.max(screen.top);
    if shared_w <= 0 || shared_h <= 0 {
        return None;
    }
    // Along the top or bottom the bar's thickness is its height; along a side, its width.
    let lying = wide >= tall;
    let (thick, shared) = if lying { (tall, shared_h) } else { (wide, shared_w) };
    if shared == thick {
        // Showing: wholly on this screen, on the nearer of its two edges.
        return Some(if lying {
            if bar.top + bar.bottom > screen.top + screen.bottom { Edge::Bottom } else { Edge::Top }
        } else if bar.left + bar.right > screen.left + screen.right {
            Edge::Right
        } else {
            Edge::Left
        });
    }
    // Hidden: only a sliver is on its own screen. More than half here is the hanging part.
    if shared * 2 > thick {
        return None;
    }
    Some(if lying {
        if bar.bottom > screen.bottom { Edge::Bottom } else { Edge::Top }
    } else if bar.right > screen.right {
        Edge::Right
    } else {
        Edge::Left
    })
}

/// `client` with one pixel given up on `edge`.
pub fn leave_room(client: Area, edge: Edge) -> Area {
    match edge {
        Edge::Left => Area { left: client.left + 1, ..client },
        Edge::Top => Area { top: client.top + 1, ..client },
        Edge::Right => Area { right: client.right - 1, ..client },
        Edge::Bottom => Area { bottom: client.bottom - 1, ..client },
    }
}

fn area(r: RECT) -> Area {
    Area { left: r.left, top: r.top, right: r.right, bottom: r.bottom }
}

/// The edge of `screen` holding a taskbar that hides itself, if any.
fn hidden_taskbar_edge(screen: Area) -> Option<Edge> {
    let mut data = APPBARDATA { cbSize: std::mem::size_of::<APPBARDATA>() as u32, ..Default::default() };
    // SAFETY: `data` is a valid, correctly sized APPBARDATA for the length of the call.
    let state = unsafe { SHAppBarMessage(ABM_GETSTATE, &mut data) };
    if state & ABS_AUTOHIDE as usize == 0 {
        return None;
    }
    // The main taskbar, and the ones Windows puts on further screens.
    let classes: [PCWSTR; 2] = [w!("Shell_TrayWnd"), w!("Shell_SecondaryTrayWnd")];
    for class in classes {
        let mut after: Option<HWND> = None;
        // SAFETY: plain window lookups; every handle is used only within this loop.
        while let Ok(bar) = unsafe { FindWindowExW(None, after, class, PCWSTR::null()) } {
            if bar.0.is_null() {
                break;
            }
            let mut rect = RECT::default();
            if unsafe { GetWindowRect(bar, &mut rect) }.is_ok() {
                if let Some(edge) = bar_edge(screen, area(rect)) {
                    return Some(edge);
                }
            }
            after = Some(bar);
        }
    }
    None
}

const SUBCLASS_ID: usize = 0x7A5C;

/// Runs after the window library has sized the page: a maximised page that covers its whole
/// screen gives one pixel back on the edge where a hidden taskbar waits.
unsafe extern "system" fn sized(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM, _id: usize, _data: usize) -> LRESULT {
    let result = DefSubclassProc(hwnd, msg, wparam, lparam);
    if msg == WM_NCCALCSIZE && lparam.0 != 0 && IsZoomed(hwnd).as_bool() {
        // Both forms of the message carry the page's rectangle first.
        let client: &mut RECT = if wparam.0 == 0 {
            &mut *(lparam.0 as *mut RECT)
        } else {
            &mut (*(lparam.0 as *mut NCCALCSIZE_PARAMS)).rgrc[0]
        };
        let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if GetMonitorInfoW(MonitorFromRect(client, MONITOR_DEFAULTTONEAREST), &mut info).as_bool() {
            let screen = area(info.rcMonitor);
            // Only a page that really covers the screen; if the library already left room, nothing to do.
            if area(*client) == screen {
                if let Some(edge) = hidden_taskbar_edge(screen) {
                    let room = leave_room(screen, edge);
                    *client = RECT { left: room.left, top: room.top, right: room.right, bottom: room.bottom };
                }
            }
        }
    } else if msg == WM_NCDESTROY {
        let _ = RemoveWindowSubclass(hwnd, Some(sized), SUBCLASS_ID);
    }
    result
}

/// Call once, on the thread that owns the window. `hwnd` is the main window's handle.
pub fn keep_taskbar_reachable(hwnd: isize) {
    let hwnd = HWND(hwnd as *mut core::ffi::c_void);
    // SAFETY: `hwnd` is the live main window and this runs on its own thread (app setup);
    // `sized` only touches the message's own data and takes itself off when the window dies.
    unsafe {
        if SetWindowSubclass(hwnd, Some(sized), SUBCLASS_ID, 0).as_bool() {
            // A window restored as maximised was sized before this was in place: size it again.
            let _ = SetWindowPos(
                hwnd,
                None,
                0,
                0,
                0,
                0,
                SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAIN: Area = Area { left: 0, top: 0, right: 1920, bottom: 1080 };
    const RIGHT_OF_MAIN: Area = Area { left: 1920, top: 0, right: 3840, bottom: 1080 };
    const BELOW_MAIN: Area = Area { left: 0, top: 1080, right: 1920, bottom: 2160 };

    #[test]
    fn a_taskbar_hidden_under_the_bottom_edge_is_found() {
        // Measured on the owner's PC: 48 px tall, 2 px of it left on the screen.
        let bar = Area { left: 0, top: 1078, right: 1920, bottom: 1126 };
        assert_eq!(bar_edge(MAIN, bar), Some(Edge::Bottom));
    }

    #[test]
    fn a_taskbar_that_is_showing_at_the_bottom_is_found_too() {
        let bar = Area { left: 0, top: 1032, right: 1920, bottom: 1080 };
        assert_eq!(bar_edge(MAIN, bar), Some(Edge::Bottom));
    }

    #[test]
    fn hidden_taskbars_on_the_other_three_edges() {
        assert_eq!(bar_edge(MAIN, Area { left: 0, top: -46, right: 1920, bottom: 2 }), Some(Edge::Top));
        assert_eq!(bar_edge(MAIN, Area { left: -60, top: 0, right: 2, bottom: 1080 }), Some(Edge::Left));
        assert_eq!(bar_edge(MAIN, Area { left: 1918, top: 0, right: 1980, bottom: 1080 }), Some(Edge::Right));
    }

    #[test]
    fn a_taskbar_on_another_screen_is_not_ours() {
        let bar = Area { left: 1920, top: 1078, right: 3840, bottom: 1126 };
        assert_eq!(bar_edge(MAIN, bar), None);
        assert_eq!(bar_edge(RIGHT_OF_MAIN, bar), Some(Edge::Bottom));
    }

    #[test]
    fn the_part_of_a_hidden_taskbar_that_hangs_over_the_next_screen_does_not_count_there() {
        // Hidden under the main screen's bottom edge, most of it lies over the screen below.
        let bar = Area { left: 0, top: 1078, right: 1920, bottom: 1126 };
        assert_eq!(bar_edge(BELOW_MAIN, bar), None);
        // Hidden off the main screen's right edge, most of it lies over the screen to the right.
        let side = Area { left: 1918, top: 0, right: 1980, bottom: 1080 };
        assert_eq!(bar_edge(RIGHT_OF_MAIN, side), None);
    }

    #[test]
    fn an_empty_or_far_away_rectangle_is_no_taskbar() {
        assert_eq!(bar_edge(MAIN, Area { left: 0, top: 0, right: 0, bottom: 0 }), None);
        assert_eq!(bar_edge(MAIN, Area { left: 5000, top: 5000, right: 6920, bottom: 5048 }), None);
    }

    #[test]
    fn one_pixel_is_given_up_on_the_taskbars_edge_and_nowhere_else() {
        assert_eq!(leave_room(MAIN, Edge::Bottom), Area { bottom: 1079, ..MAIN });
        assert_eq!(leave_room(MAIN, Edge::Top), Area { top: 1, ..MAIN });
        assert_eq!(leave_room(MAIN, Edge::Left), Area { left: 1, ..MAIN });
        assert_eq!(leave_room(MAIN, Edge::Right), Area { right: 1919, ..MAIN });
    }
}
