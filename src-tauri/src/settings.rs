//! The user's settings, kept in one small JSON file.

use serde::{Deserialize, Serialize};
use std::path::Path;

/// 0 means paused: the numbers only refresh when asked (F5).
pub const INTERVALS_MS: [u64; 5] = [0, 500, 1000, 2000, 5000];
pub const TABS: [&str; 5] = ["processes", "performance", "users", "startup", "services"];
pub const THEMES: [&str; 3] = ["system", "light", "dark"];
pub const DEFAULT_HOTKEY: &str = "Ctrl+Alt+Escape";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Settings {
    /// How often the numbers refresh.
    pub interval_ms: u64,
    /// Global shortcut that shows or hides the window. Empty = none.
    pub hotkey: String,
    /// Start hidden in the tray when Windows starts.
    pub autostart: bool,
    /// The window's X button hides to the tray instead of quitting.
    pub close_to_tray: bool,
    pub always_on_top: bool,
    /// Ctrl+Shift+Esc opens this app instead of Windows Task Manager.
    pub ctrl_shift_esc: bool,
    /// Minimising the window sends it to the tray.
    pub hide_when_minimized: bool,
    /// The tab shown when the app starts.
    pub start_tab: String,
    /// `system`, `light` or `dark`.
    pub theme: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            interval_ms: 1000,
            hotkey: DEFAULT_HOTKEY.into(),
            autostart: false,
            close_to_tray: true,
            always_on_top: false,
            ctrl_shift_esc: true,
            hide_when_minimized: false,
            start_tab: "processes".into(),
            theme: "system".into(),
        }
    }
}

impl Settings {
    /// Forces every value into what the app supports, so a hand-edited or old file cannot
    /// put the app in a broken state.
    pub fn sanitized(mut self) -> Settings {
        if !INTERVALS_MS.contains(&self.interval_ms) {
            // Nearest supported speed (never "paused": that is only ever chosen on purpose).
            self.interval_ms = *INTERVALS_MS
                .iter()
                .filter(|v| **v != 0)
                .min_by_key(|v| v.abs_diff(self.interval_ms))
                .unwrap_or(&1000);
        }
        if !TABS.contains(&self.start_tab.as_str()) {
            self.start_tab = "processes".into();
        }
        if !THEMES.contains(&self.theme.as_str()) {
            self.theme = "system".into();
        }
        self.hotkey = self.hotkey.trim().to_string();
        if self.hotkey.len() > 64 {
            self.hotkey = DEFAULT_HOTKEY.into();
        }
        self
    }
}

/// A missing or damaged file gives the defaults; the app must always start.
pub fn load(path: &Path) -> Settings {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<Settings>(&text).ok())
        .unwrap_or_default()
        .sanitized()
}

/// Writes to a temporary file first and swaps it in, so a crash or power cut mid-save
/// leaves the old settings intact instead of a half-written file.
pub fn save(path: &Path, settings: &Settings) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(settings)?)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("ftm-settings-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("nested").join("settings.json")
    }

    #[test]
    fn missing_file_gives_defaults() {
        assert_eq!(load(&temp("missing")), Settings::default());
    }

    #[test]
    fn damaged_file_gives_defaults_instead_of_failing() {
        let path = temp("damaged");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"{ this is not json").unwrap();
        assert_eq!(load(&path), Settings::default());
    }

    #[test]
    fn save_then_load_round_trips() {
        let path = temp("roundtrip");
        let s = Settings {
            interval_ms: 2000,
            hotkey: "Ctrl+Shift+F12".into(),
            autostart: true,
            close_to_tray: false,
            always_on_top: true,
            ctrl_shift_esc: false,
            hide_when_minimized: true,
            start_tab: "services".into(),
            theme: "dark".into(),
        };
        save(&path, &s).unwrap();
        assert_eq!(load(&path), s);
        assert!(!path.with_extension("json.tmp").exists(), "no temporary file is left behind");
    }

    #[test]
    fn older_file_with_missing_fields_keeps_what_it_has() {
        let path = temp("partial");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, br#"{"interval_ms": 5000}"#).unwrap();
        let s = load(&path);
        assert_eq!(s.interval_ms, 5000);
        assert_eq!(s.hotkey, DEFAULT_HOTKEY);
        assert!(s.close_to_tray);
    }

    #[test]
    fn unsupported_speed_snaps_to_the_nearest() {
        let s = |ms| Settings { interval_ms: ms, ..Default::default() }.sanitized().interval_ms;
        assert_eq!(s(0), 0, "paused is a real choice and is kept");
        assert_eq!(s(1), 500);
        assert_eq!(s(900), 1000);
        assert_eq!(s(3000), 2000);
        assert_eq!(s(u64::MAX), 5000);
    }

    #[test]
    fn absurd_hotkey_text_is_replaced() {
        let s = Settings { hotkey: "x".repeat(500), ..Default::default() }.sanitized();
        assert_eq!(s.hotkey, DEFAULT_HOTKEY);
        let s = Settings { hotkey: "  Ctrl+K ".into(), ..Default::default() }.sanitized();
        assert_eq!(s.hotkey, "Ctrl+K");
    }

    #[test]
    fn new_settings_get_safe_values_from_old_or_edited_files() {
        // A file written before these settings existed.
        let old: Settings = serde_json::from_str(r#"{"interval_ms":2000,"hotkey":"Ctrl+Alt+Escape"}"#).unwrap();
        let old = old.sanitized();
        assert!(old.ctrl_shift_esc && !old.hide_when_minimized);
        assert_eq!((old.start_tab.as_str(), old.theme.as_str()), ("processes", "system"));
        // A hand-edited file with nonsense.
        let odd = Settings { start_tab: "hacks".into(), theme: "pink".into(), interval_ms: 3, ..Default::default() }.sanitized();
        assert_eq!((odd.start_tab.as_str(), odd.theme.as_str()), ("processes", "system"));
        assert_eq!(odd.interval_ms, 500, "a tiny number is the fastest speed, never 'paused'");
        assert_eq!(Settings { interval_ms: 0, ..Default::default() }.sanitized().interval_ms, 0, "paused is kept");
    }
}
