//! Ctrl+Shift+Esc opens this app instead of Windows Task Manager.
//!
//! Windows keeps that key combination for its own Task Manager, so it cannot be claimed like
//! an ordinary hotkey. While this app runs, a keyboard listener sees the key first and keeps
//! it. Nothing in Windows is changed: quit the app (or switch this off) and the keys open
//! Windows Task Manager again.
//!
//! Limits: it works while the app is running (it sits in the tray), and Windows does not let
//! a normal program see keys pressed inside a window that runs as administrator. There the
//! keys still open Windows Task Manager, unless this app was restarted as administrator.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{mpsc, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::Duration;
use windows::Win32::Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::{keybd_event, GetAsyncKeyState, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetMessageW, PostThreadMessageW, SetWindowsHookExW, UnhookWindowsHookEx, KBDLLHOOKSTRUCT, MSG,
    WH_KEYBOARD_LL, WM_KEYDOWN, WM_QUIT, WM_SYSKEYDOWN,
};

const VK_ESCAPE: u32 = 0x1B;
const VK_SHIFT: i32 = 0x10;
const VK_CONTROL: i32 = 0x11;
const VK_ALT: i32 = 0x12;
const VK_LWIN: i32 = 0x5B;
const VK_RWIN: i32 = 0x5C;
/// A key code no keyboard has; pressing it does nothing in any program.
const VK_NOTHING: u8 = 0xE8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Not ours: the key goes on to whatever program has the keyboard.
    Pass,
    /// Ours, but nothing more to do (key repeat, or the release of a key we kept).
    Keep,
    /// Ctrl+Shift+Esc was just pressed: keep the key and open the app.
    Open,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Mods {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub win: bool,
}

/// Decides what to do with one key event. `kept` says whether the Esc press now being held
/// was one we kept; the second value is the new `kept`.
///
/// Exactly Ctrl+Shift+Esc is taken. Ctrl+Shift+Alt+Esc, plain Esc and every other key go
/// through untouched. Once an Esc press is kept, its repeats and its release are kept too,
/// so no program ever receives half a key press.
pub fn decide(vk: u32, down: bool, mods: Mods, kept: bool) -> (Verdict, bool) {
    if vk != VK_ESCAPE {
        return (Verdict::Pass, kept);
    }
    if down {
        if kept {
            return (Verdict::Keep, true);
        }
        if mods.ctrl && mods.shift && !mods.alt && !mods.win {
            return (Verdict::Open, true);
        }
        return (Verdict::Pass, false);
    }
    if kept {
        (Verdict::Keep, false)
    } else {
        (Verdict::Pass, false)
    }
}

/// A held key repeats at least every 2 seconds, even on the slowest Windows setting. A kept
/// Esc that has been silent for longer was let go without us being told: Windows hides keys
/// from normal programs while an administrator's window is in front, and that is exactly what
/// comes to the front when our window hides. Without this the next press would be ignored.
const HELD_GAP_MS: u32 = 2500;

/// Whether the Esc press we kept is still being held. `last_ms` is when it was last pressed
/// or repeated, `now_ms` the time of the event being looked at (Windows' millisecond clock,
/// which starts again from zero after about 49 days).
pub fn still_held(kept: bool, last_ms: u32, now_ms: u32) -> bool {
    kept && now_ms.wrapping_sub(last_ms) <= HELD_GAP_MS
}

static KEPT: AtomicBool = AtomicBool::new(false);
/// When the kept Esc was last pressed or repeated.
static KEPT_AT: AtomicU32 = AtomicU32::new(0);
/// How many Esc key events the listener was handed, and how many times it opened the app.
/// They answer "is the listener alive?" without logging anything a person typed.
static SEEN_ESC: AtomicU32 = AtomicU32::new(0);
static OPENED: AtomicU32 = AtomicU32::new(0);
/// How many key events of any kind the listener was handed, and whether its thread is waiting for more.
static CALLS: AtomicU32 = AtomicU32::new(0);
static WAITING: AtomicBool = AtomicBool::new(false);

/// (Esc events seen, times the app was opened, key events of any kind, listener waiting) since the app started.
pub fn stats() -> (u32, u32, u32, bool) {
    (
        SEEN_ESC.load(Ordering::Relaxed),
        OPENED.load(Ordering::Relaxed),
        CALLS.load(Ordering::Relaxed),
        WAITING.load(Ordering::Relaxed),
    )
}
static ON_OPEN: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();

/// What to do when the keys are pressed. Set once at start-up. It is called on the listener
/// thread and must return at once (hand the work to another thread).
pub fn on_open(action: impl Fn() + Send + Sync + 'static) {
    let _ = ON_OPEN.set(Box::new(action));
}

unsafe extern "system" fn callback(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    CALLS.fetch_add(1, Ordering::Relaxed);
    if code == 0 {
        let key = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
        if key.vkCode == VK_ESCAPE {
            SEEN_ESC.fetch_add(1, Ordering::Relaxed);
            let message = wparam.0 as u32;
            let down = message == WM_KEYDOWN || message == WM_SYSKEYDOWN;
            let held = |vk: i32| GetAsyncKeyState(vk) < 0;
            let mods = Mods {
                ctrl: held(VK_CONTROL),
                shift: held(VK_SHIFT),
                alt: held(VK_ALT),
                win: held(VK_LWIN) || held(VK_RWIN),
            };
            let held_now = still_held(KEPT.load(Ordering::Relaxed), KEPT_AT.load(Ordering::Relaxed), key.time);
            let (verdict, kept) = decide(key.vkCode, down, mods, held_now);
            KEPT.store(kept, Ordering::Relaxed);
            if down && kept {
                KEPT_AT.store(key.time, Ordering::Relaxed);
            }
            match verdict {
                Verdict::Pass => {}
                Verdict::Keep => return LRESULT(1),
                Verdict::Open => {
                    OPENED.fetch_add(1, Ordering::Relaxed);
                    if let Some(action) = ON_OPEN.get() {
                        action();
                    }
                    return LRESULT(1);
                }
            }
        }
    }
    CallNextHookEx(None, code, wparam, lparam)
}

/// Because the Esc press is kept, Windows only sees "Ctrl+Shift pressed and released", which
/// on many PCs switches the keyboard language. One press of a key that does nothing tells
/// Windows the modifiers were used for something, so the language stays as it is.
pub fn mask_modifiers() {
    unsafe {
        keybd_event(VK_NOTHING, 0, KEYBD_EVENT_FLAGS(0), 0);
        keybd_event(VK_NOTHING, 0, KEYEVENTF_KEYUP, 0);
    }
}

struct Listener {
    thread_id: u32,
    thread: JoinHandle<()>,
}

fn listener() -> &'static Mutex<Option<Listener>> {
    static LISTENER: OnceLock<Mutex<Option<Listener>>> = OnceLock::new();
    LISTENER.get_or_init(|| Mutex::new(None))
}

pub fn is_enabled() -> bool {
    listener().lock().map(|l| l.is_some()).unwrap_or(false)
}

/// Starts listening. Does nothing when already listening.
pub fn enable() -> Result<(), String> {
    let mut slot = listener().lock().map_err(|_| "internal error".to_string())?;
    if slot.is_some() {
        return Ok(());
    }
    KEPT.store(false, Ordering::Relaxed);
    let (ready, started) = mpsc::channel::<Result<u32, String>>();
    let thread = std::thread::Builder::new()
        .name("keyhook".into())
        .spawn(move || unsafe {
            let module = GetModuleHandleW(None).ok().map(|m| HINSTANCE(m.0));
            let hook = match SetWindowsHookExW(WH_KEYBOARD_LL, Some(callback), module, 0) {
                Ok(hook) => hook,
                Err(e) => {
                    let _ = ready.send(Err(e.message()));
                    return;
                }
            };
            let _ = ready.send(Ok(GetCurrentThreadId()));
            // The listener is called through this thread's message queue.
            let mut message = MSG::default();
            WAITING.store(true, Ordering::Relaxed);
            while GetMessageW(&mut message, None, 0, 0).0 > 0 {}
            WAITING.store(false, Ordering::Relaxed);
            let _ = UnhookWindowsHookEx(hook);
        })
        .map_err(|e| e.to_string())?;
    match started.recv_timeout(Duration::from_secs(3)) {
        Ok(Ok(thread_id)) => {
            *slot = Some(Listener { thread_id, thread });
            Ok(())
        }
        Ok(Err(e)) => Err(e),
        Err(_) => Err("the keyboard listener did not start".into()),
    }
}

/// Stops listening and gives the keys back to Windows.
pub fn disable() {
    let Some(l) = listener().lock().ok().and_then(|mut slot| slot.take()) else { return };
    unsafe {
        let _ = PostThreadMessageW(l.thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
    }
    let _ = l.thread.join();
    KEPT.store(false, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    const CTRL_SHIFT: Mods = Mods { ctrl: true, shift: true, alt: false, win: false };
    const NONE: Mods = Mods { ctrl: false, shift: false, alt: false, win: false };

    #[test]
    fn ctrl_shift_esc_opens_and_is_kept_whole() {
        let (verdict, kept) = decide(VK_ESCAPE, true, CTRL_SHIFT, false);
        assert_eq!((verdict, kept), (Verdict::Open, true));
        // Holding the keys repeats the press: kept, but the app is not opened again.
        assert_eq!(decide(VK_ESCAPE, true, CTRL_SHIFT, true), (Verdict::Keep, true));
        // Ctrl let go before Esc: the rest of the press is still ours.
        assert_eq!(decide(VK_ESCAPE, true, NONE, true), (Verdict::Keep, true));
        assert_eq!(decide(VK_ESCAPE, false, NONE, true), (Verdict::Keep, false));
    }

    #[test]
    fn every_other_key_press_is_left_alone() {
        assert_eq!(decide(VK_ESCAPE, true, NONE, false), (Verdict::Pass, false), "plain Esc");
        assert_eq!(decide(VK_ESCAPE, false, NONE, false), (Verdict::Pass, false));
        assert_eq!(decide(VK_ESCAPE, true, Mods { shift: false, ..CTRL_SHIFT }, false).0, Verdict::Pass, "Ctrl+Esc");
        assert_eq!(decide(VK_ESCAPE, true, Mods { ctrl: false, ..CTRL_SHIFT }, false).0, Verdict::Pass, "Shift+Esc");
        assert_eq!(decide(VK_ESCAPE, true, Mods { alt: true, ..CTRL_SHIFT }, false).0, Verdict::Pass, "with Alt");
        assert_eq!(decide(VK_ESCAPE, true, Mods { win: true, ..CTRL_SHIFT }, false).0, Verdict::Pass, "with Win");
        assert_eq!(decide(0x41, true, CTRL_SHIFT, false), (Verdict::Pass, false), "Ctrl+Shift+A");
        assert_eq!(decide(0x41, true, CTRL_SHIFT, true), (Verdict::Pass, true), "another key does not end a kept press");
    }

    #[test]
    fn a_kept_press_whose_release_never_came_does_not_block_the_next_press() {
        // Held and repeating: still the same press.
        assert!(still_held(true, 10_000, 10_030));
        assert!(still_held(true, 10_000, 10_000 + HELD_GAP_MS));
        // Silent for longer than any key repeat: it was let go while we could not see it.
        assert!(!still_held(true, 10_000, 10_001 + HELD_GAP_MS));
        // Nothing kept is never "held".
        assert!(!still_held(false, 10_000, 10_030));
        // The clock starting again from zero does not confuse it.
        assert!(still_held(true, u32::MAX - 10, 20));
        assert!(!still_held(true, u32::MAX - 10, HELD_GAP_MS + 20));
        // So the next Ctrl+Shift+Esc opens the app instead of being swallowed as a "repeat".
        let stale = still_held(true, 10_000, 20_000);
        assert_eq!(decide(VK_ESCAPE, true, CTRL_SHIFT, stale), (Verdict::Open, true));
    }

    /// Not a check: listens for 8 seconds while another program presses the keys, then prints
    /// what the listener was handed. `--ignored --nocapture`.
    #[test]
    #[ignore]
    fn diagnostic_listen_only() {
        on_open(|| {});
        enable().expect("listener starts");
        std::thread::sleep(Duration::from_secs(8));
        println!("DIAG listen-only stats={:?}", stats());
        disable();
    }

    /// Not a check: only presses Ctrl+Shift+Esc for real, so a running copy of the app can be
    /// watched from outside. `--ignored`.
    #[test]
    #[ignore]
    fn diagnostic_press_only() {
        let press = |vk: u8, up: bool| unsafe {
            keybd_event(vk, 0, if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) }, 0);
            std::thread::sleep(Duration::from_millis(40));
        };
        press(0x11, false);
        press(0x10, false);
        press(0x1B, false);
        press(0x1B, true);
        press(0x10, true);
        press(0x11, true);
        std::thread::sleep(Duration::from_millis(700));
    }

    /// Not a check: presses Ctrl+Shift+Esc for real and prints whether the listener was handed
    /// it. Opens Windows Task Manager if the listener cannot keep the key. `--ignored --nocapture`.
    #[test]
    #[ignore]
    fn diagnostic_a_real_press() {
        static RAN: AtomicBool = AtomicBool::new(false);
        on_open(|| RAN.store(true, Ordering::Relaxed));
        enable().expect("listener starts");
        std::thread::sleep(Duration::from_millis(300));
        let press = |vk: u8, up: bool| unsafe {
            keybd_event(vk, 0, if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) }, 0);
            std::thread::sleep(Duration::from_millis(40));
        };
        press(0x11, false);
        press(0x10, false);
        press(0x1B, false);
        press(0x1B, true);
        press(0x10, true);
        press(0x11, true);
        std::thread::sleep(Duration::from_millis(700));
        println!("DIAG stats={:?} action-ran={}", stats(), RAN.load(Ordering::Relaxed));
        disable();
    }

    #[test]
    fn the_listener_starts_once_and_stops_cleanly() {
        assert!(!is_enabled());
        enable().expect("listener starts");
        enable().expect("a second start is a no-op");
        assert!(is_enabled());
        disable();
        assert!(!is_enabled());
        disable(); // stopping twice is harmless
        enable().expect("and it can start again");
        disable();
    }
}
