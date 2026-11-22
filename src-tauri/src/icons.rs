//! Program icons for the process list, read from the program file once and remembered.
//!
//! All reading happens on ONE thread. The Windows shell call that hands out icons loses some of
//! them when several threads ask at the same moment (measured: 15-21 of 101 programs came back
//! empty when a whole list asked at once, none when asked one after another), and the screen
//! asks for a whole list at once every time it opens.

use base64::Engine;
use std::collections::HashMap;
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::time::Duration;
use windows::core::PCWSTR;
use windows::Win32::Graphics::Gdi::{
    DeleteObject, GetDC, GetDIBits, GetObjectW, ReleaseDC, BITMAP, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
    DIB_RGB_COLORS, HBITMAP,
};
use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_NORMAL;
use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::Shell::{SHGetFileInfoW, SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON};
use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, HICON, ICONINFO};

fn cache() -> &'static Mutex<HashMap<String, Option<Arc<str>>>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Option<Arc<str>>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The icon of a program file as a `data:image/png;base64,…` URL, or `None` when it has none.
pub fn data_url(exe: &str) -> Option<Arc<str>> {
    if exe.is_empty() {
        return None;
    }
    let key = exe.to_lowercase();
    if let Some(hit) = cache().lock().ok()?.get(&key) {
        return hit.clone();
    }
    let made: Option<Arc<str>> = extract(exe).and_then(|(w, h, rgba)| encode_png(w, h, &rgba)).map(|png| {
        let b64 = base64::engine::general_purpose::STANDARD.encode(png);
        Arc::from(format!("data:image/png;base64,{b64}"))
    });
    // Only an icon that came out is remembered. A miss may be a program that was still being
    // written or started, so the next ask tries again instead of showing a blank for good.
    if made.is_some() {
        cache().lock().ok()?.insert(key, made.clone());
    }
    made
}

type Pixels = (u32, u32, Vec<u8>);
type Job = (String, mpsc::Sender<Option<Pixels>>);

/// The one thread that talks to the Windows shell about icons.
fn worker() -> &'static Mutex<mpsc::Sender<Job>> {
    static WORKER: OnceLock<Mutex<mpsc::Sender<Job>>> = OnceLock::new();
    WORKER.get_or_init(|| {
        let (jobs, inbox) = mpsc::channel::<Job>();
        let _ = std::thread::Builder::new().name("icons".into()).spawn(move || {
            // The shell asks for this before any icon call.
            unsafe {
                let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            }
            for (exe, reply) in inbox {
                let _ = reply.send(extract_why(&exe).ok());
            }
        });
        Mutex::new(jobs)
    })
}

pub fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Option<Vec<u8>> {
    if width == 0 || height == 0 || rgba.len() != (width * height * 4) as usize {
        return None;
    }
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().ok()?;
        writer.write_image_data(rgba).ok()?;
    }
    Some(out)
}

/// Windows gives icon pixels as blue-green-red-alpha. Old icons carry no alpha at all and use a
/// separate mask instead (mask bit set = see-through). Returns red-green-blue-alpha.
pub fn bgra_to_rgba(mut color: Vec<u8>, mask: Option<&[u8]>) -> Vec<u8> {
    let has_alpha = color.chunks_exact(4).any(|p| p[3] != 0);
    for (i, p) in color.chunks_exact_mut(4).enumerate() {
        p.swap(0, 2);
        if !has_alpha {
            let see_through = mask.map(|m| m.get(i * 4).copied().unwrap_or(0) != 0).unwrap_or(false);
            p[3] = if see_through { 0 } else { 255 };
        }
    }
    color
}

struct Bitmap(HBITMAP);
impl Drop for Bitmap {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = DeleteObject(self.0.into());
            }
        }
    }
}

struct Icon(HICON);
impl Drop for Icon {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyIcon(self.0);
        }
    }
}

/// The icon's pixels, read on the icon thread. Safe to call from any number of threads.
fn extract(exe: &str) -> Option<Pixels> {
    let (reply, answer) = mpsc::channel();
    worker().lock().ok()?.send((exe.to_string(), reply)).ok()?;
    answer.recv_timeout(Duration::from_secs(10)).ok()?
}

/// Like `extract`, but says at which step it gave up.
fn extract_why(exe: &str) -> Result<(u32, u32, Vec<u8>), &'static str> {
    let wide: Vec<u16> = exe.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let mut info = SHFILEINFOW::default();
        let ok = SHGetFileInfoW(
            PCWSTR(wide.as_ptr()),
            FILE_ATTRIBUTE_NORMAL,
            Some(&mut info),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_ICON | SHGFI_LARGEICON,
        );
        if ok == 0 || info.hIcon.is_invalid() {
            return Err("shell gave no icon");
        }
        let icon = Icon(info.hIcon);

        let mut parts = ICONINFO::default();
        GetIconInfo(icon.0, &mut parts).map_err(|_| "icon has no parts")?;
        let color = Bitmap(parts.hbmColor);
        let mask = Bitmap(parts.hbmMask);
        if color.0.is_invalid() {
            return Err("black-and-white icon");
        }

        let mut bm = BITMAP::default();
        if GetObjectW(color.0.into(), std::mem::size_of::<BITMAP>() as i32, Some(&mut bm as *mut _ as *mut _)) == 0 {
            return Err("bitmap has no size");
        }
        let (w, h) = (bm.bmWidth, bm.bmHeight);
        if w <= 0 || h <= 0 || w > 256 || h > 256 {
            return Err("odd size");
        }

        let hdc = GetDC(None);
        let read = |bitmap: HBITMAP| -> Option<Vec<u8>> {
            let mut bi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w,
                    biHeight: -h, // top row first
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut px = vec![0u8; (w * h * 4) as usize];
            let rows = GetDIBits(hdc, bitmap, 0, h as u32, Some(px.as_mut_ptr() as *mut _), &mut bi, DIB_RGB_COLORS);
            (rows == h).then_some(px)
        };
        let color_px = read(color.0);
        let mask_px = if mask.0.is_invalid() { None } else { read(mask.0) };
        ReleaseDC(None, hdc);

        Ok((w as u32, h as u32, bgra_to_rgba(color_px.ok_or("pixels unreadable")?, mask_px.as_deref())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swaps_blue_and_red_and_keeps_real_alpha() {
        let rgba = bgra_to_rgba(vec![10, 20, 30, 128], None);
        assert_eq!(rgba, vec![30, 20, 10, 128]);
    }

    #[test]
    fn icon_without_alpha_uses_the_mask() {
        // two pixels, no alpha anywhere; the mask marks the second one see-through
        let color = vec![1, 2, 3, 0, 4, 5, 6, 0];
        let mask = vec![0, 0, 0, 0, 255, 255, 255, 0];
        assert_eq!(bgra_to_rgba(color, Some(&mask)), vec![3, 2, 1, 255, 6, 5, 4, 0]);
    }

    #[test]
    fn icon_without_alpha_or_mask_is_fully_visible() {
        assert_eq!(bgra_to_rgba(vec![1, 2, 3, 0], None), vec![3, 2, 1, 255]);
    }

    #[test]
    fn png_encoder_refuses_wrong_sizes() {
        assert!(encode_png(2, 2, &[0; 16]).is_some());
        assert!(encode_png(2, 2, &[0; 15]).is_none());
        assert!(encode_png(0, 2, &[]).is_none());
    }

    /// Not a check: for every running program, says whether an icon came out and, if not, why.
    /// Prints program file names only. `--ignored --nocapture`.
    #[test]
    #[ignore]
    fn diagnostic_icons_of_everything_running() {
        let mut engine = crate::proclist::Engine::new(1.0);
        let rows = engine.read(&HashMap::new(), true);
        let mut seen: HashMap<String, String> = HashMap::new();
        for row in &rows {
            let verdict = match crate::procinfo::exe_path(row.pid) {
                None => "no path (cannot open the process)".to_string(),
                Some(path) => match extract_why(&path) {
                    Ok((w, h, _)) => format!("ok {w}x{h}"),
                    Err(why) => format!("FAILED: {why}"),
                },
            };
            seen.entry(row.name.to_lowercase()).or_insert(verdict);
        }
        let mut by: HashMap<String, Vec<String>> = HashMap::new();
        for (name, verdict) in seen {
            by.entry(verdict).or_default().push(name);
        }
        for (verdict, mut names) in by {
            names.sort();
            println!("ICONS {} -> {} programs: {}", verdict, names.len(), names.join(", "));
        }
    }

    /// The screen asks for a whole list of icons at the same moment. Read straight from 16
    /// threads, 15-21 of 101 programs came back without an icon on this PC (2026-11-22).
    #[test]
    fn a_whole_list_asking_at_once_loses_no_icon() {
        let mut engine = crate::proclist::Engine::new(1.0);
        let rows = engine.read(&HashMap::new(), true);
        let mut paths: Vec<String> = rows.iter().filter_map(|r| crate::procinfo::exe_path(r.pid)).collect();
        paths.sort();
        paths.dedup();
        assert!(paths.len() > 20, "a running PC has more than 20 different programs, got {}", paths.len());
        // What one thread alone gets is the truth to compare against.
        let alone: Vec<bool> = paths.iter().map(|p| extract(p).is_some()).collect();
        let paths = Arc::new(paths);
        let got: Arc<Mutex<Vec<(usize, bool)>>> = Arc::new(Mutex::new(Vec::new()));
        let mut threads = Vec::new();
        for t in 0..16usize {
            let (paths, got) = (paths.clone(), got.clone());
            threads.push(std::thread::spawn(move || {
                for (i, path) in paths.iter().enumerate() {
                    if i % 16 == t {
                        let ok = extract(path).is_some();
                        got.lock().unwrap().push((i, ok));
                    }
                }
            }));
        }
        for t in threads {
            t.join().unwrap();
        }
        let got = got.lock().unwrap();
        let lost: Vec<&str> = got
            .iter()
            .filter(|(i, ok)| alone[*i] && !ok)
            .map(|(i, _)| paths[*i].rsplit(['\\', '/']).next().unwrap_or(""))
            .collect();
        assert!(lost.is_empty(), "icons lost when asked at once: {lost:?}");
        assert!(alone.iter().filter(|ok| **ok).count() * 10 >= alone.len() * 9, "nearly every program has an icon");
    }

    #[test]
    fn a_miss_is_not_remembered() {
        assert!(data_url(r"C:\no-such-folder\no-such-program.exe").is_none());
        assert!(!cache().lock().unwrap().contains_key(r"c:\no-such-folder\no-such-program.exe"));
    }

    #[test]
    fn real_program_has_an_icon_and_a_missing_one_does_not_crash() {
        let windir = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into());
        let url = data_url(&format!(r"{windir}\explorer.exe")).expect("explorer.exe has an icon");
        assert!(url.starts_with("data:image/png;base64,"));
        assert!(url.len() > 200);
        // Asking again returns the remembered copy (same allocation).
        let again = data_url(&format!(r"{windir}\EXPLORER.EXE")).unwrap();
        assert!(Arc::ptr_eq(&url, &again));
        assert!(data_url("").is_none());
    }
}
