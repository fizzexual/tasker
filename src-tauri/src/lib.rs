//! Tasker: window, tray, hotkey, and the commands the screen calls.

pub mod actions;
pub mod elevate;
pub mod error;
pub mod hardware;
pub mod icons;
pub mod keyhook;
pub mod ntproc;
pub mod pdh;
pub mod procinfo;
pub mod proclist;
pub mod procs;
pub mod sampler;
pub mod services;
pub mod settings;
pub mod startup;
pub mod tree;
pub mod users;
pub mod winframe;
pub mod wins;

use error::{CmdError, CmdResult};
use serde::Serialize;
use settings::Settings;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, State, WindowEvent};
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_global_shortcut::{GlobalShortcutExt as _, Shortcut, ShortcutState};

const HIDDEN_ARG: &str = "--hidden";

pub struct AppState {
    shared: Arc<sampler::Shared>,
    settings: Mutex<Settings>,
    settings_path: PathBuf,
    /// Why the hotkey is not active, when it is not.
    hotkey_error: Mutex<Option<String>>,
    /// Why Ctrl+Shift+Esc is not ours, when it should be.
    takeover_error: Mutex<Option<String>>,
    /// Started by Windows at sign-in: stay in the tray until asked for.
    start_hidden: bool,
    shown_once: AtomicBool,
}

static LOG_PATH: OnceLock<PathBuf> = OnceLock::new();

/// Appends one line to `errors.log` in the app's log folder. Never fails the caller.
pub fn log_line(message: &str) {
    let Some(path) = LOG_PATH.get() else { return };
    // Keep the file small: start over once it passes 256 KB.
    if std::fs::metadata(path).map(|m| m.len() > 256 * 1024).unwrap_or(false) {
        let _ = std::fs::remove_file(path);
    }
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let _ = writeln!(file, "[{secs}] {message}");
    }
}

pub fn log_error(_app: &AppHandle, message: &str) {
    log_line(message);
}

// ───────────────────────── window ─────────────────────────

fn set_visible(app: &AppHandle, visible: bool) {
    if let Some(state) = app.try_state::<AppState>() {
        state.shared.update(|c| c.visible = visible);
    }
}

fn show_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
    if let Some(state) = app.try_state::<AppState>() {
        state.shown_once.store(true, Ordering::Relaxed);
    }
    set_visible(app, true);
}

fn hide_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
    set_visible(app, false);
}

/// Hotkey / tray click: bring the window up, or put it away if it is already in front.
fn toggle_main(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else { return };
    let visible = window.is_visible().unwrap_or(false);
    let minimized = window.is_minimized().unwrap_or(false);
    let focused = window.is_focused().unwrap_or(false);
    if visible && !minimized && focused {
        hide_main(app);
    } else {
        show_main(app);
    }
}

// ───────────────────────── settings & hotkey ─────────────────────────

/// Makes `new` the active hotkey. If Windows refuses it (another program owns it), the old
/// hotkey is put back and an error is returned: a refused change changes nothing.
fn apply_hotkey(app: &AppHandle, old: &str, new: &str) -> Result<(), String> {
    let shortcuts = app.global_shortcut();
    let parse = |text: &str| text.parse::<Shortcut>().ok();

    let new_shortcut = if new.is_empty() {
        None
    } else {
        Some(parse(new).ok_or_else(|| format!("\"{new}\" is not a shortcut this app understands."))?)
    };
    let old_shortcut = if old.is_empty() { None } else { parse(old) };

    if let Some(o) = old_shortcut {
        if shortcuts.is_registered(o) {
            let _ = shortcuts.unregister(o);
        }
    }
    if let Some(n) = new_shortcut {
        if let Err(e) = shortcuts.register(n) {
            if let Some(o) = old_shortcut {
                let _ = shortcuts.register(o);
            }
            log_line(&format!("hotkey {new} refused: {e}"));
            return Err(format!("{new} is already used by another program."));
        }
    }
    Ok(())
}

#[derive(Serialize, Clone)]
pub struct AppStatus {
    version: String,
    elevated: bool,
    settings: Settings,
    hotkey_error: Option<String>,
    takeover_error: Option<String>,
    own_pid: u32,
    session: u32,
    /// [Esc key events the Ctrl+Shift+Esc listener was handed, times it opened the app].
    takeover_stats: (u32, u32, u32, bool),
}

fn status_of(app: &AppHandle, state: &AppState) -> AppStatus {
    AppStatus {
        version: app.package_info().version.to_string(),
        elevated: elevate::is_elevated(),
        settings: state.settings.lock().map(|s| s.clone()).unwrap_or_default(),
        hotkey_error: state.hotkey_error.lock().ok().and_then(|e| e.clone()),
        takeover_error: state.takeover_error.lock().ok().and_then(|e| e.clone()),
        own_pid: std::process::id(),
        session: users::current_session(),
        takeover_stats: keyhook::stats(),
    }
}

// ───────────────────────── commands ─────────────────────────

#[tauri::command]
fn ui_ready(app: AppHandle, state: State<AppState>) {
    if !state.start_hidden && !state.shown_once.load(Ordering::Relaxed) {
        show_main(&app);
    }
}

#[tauri::command]
fn app_status(app: AppHandle, state: State<AppState>) -> AppStatus {
    status_of(&app, &state)
}

#[tauri::command]
fn set_view(state: State<AppState>, tab: String) {
    // The Users tab adds up the same process rows per person.
    state.shared.update(|c| c.want_procs = tab == "processes" || tab == "users");
}

// The app draws its own title bar (the Windows frame is off), so its three buttons come here.

#[tauri::command]
fn window_minimize(window: tauri::WebviewWindow) {
    let _ = window.minimize();
}

/// Fills the screen, or goes back to the window's own size. Returns the new state.
#[tauri::command]
fn window_toggle_maximize(window: tauri::WebviewWindow) -> bool {
    let maximized = window.is_maximized().unwrap_or(false);
    let _ = if maximized { window.unmaximize() } else { window.maximize() };
    !maximized
}

#[tauri::command]
fn window_is_maximized(window: tauri::WebviewWindow) -> bool {
    window.is_maximized().unwrap_or(false)
}

/// Asks to close, exactly like Alt+F4: the close handler below decides between tray and exit.
#[tauri::command]
fn window_close(window: tauri::WebviewWindow) {
    let _ = window.close();
}

/// "Refresh now" (F5): one reading at once, also while paused.
#[tauri::command]
fn refresh_now(state: State<AppState>) {
    state.shared.update(|c| c.recheck = true);
}

#[tauri::command]
fn get_history(state: State<AppState>) -> Vec<sampler::PerfSample> {
    state.shared.history.lock().map(|h| h.iter().cloned().collect()).unwrap_or_default()
}

#[derive(Serialize, Clone, Default)]
pub struct Volume {
    mount: String,
    label: String,
    total: u64,
    free: u64,
}

#[derive(Serialize, Clone, Default)]
pub struct SystemInfo {
    cpu_name: String,
    physical_cores: u32,
    logical_cores: u32,
    host: String,
    os: String,
    /// Unix seconds.
    boot_time: u64,
    volumes: Vec<Volume>,
}

#[tauri::command(async)]
fn system_info() -> SystemInfo {
    use sysinfo::{CpuRefreshKind, Disks, RefreshKind, System};
    static FIXED: OnceLock<SystemInfo> = OnceLock::new();
    let mut info = FIXED
        .get_or_init(|| {
            let sys = System::new_with_specifics(RefreshKind::nothing().with_cpu(CpuRefreshKind::nothing())).ok();
            let cpus = sys.as_ref().map(|s| s.cpus()).unwrap_or(&[]);
            SystemInfo {
                cpu_name: cpus.first().map(|c| c.brand().trim().to_string()).unwrap_or_default(),
                logical_cores: cpus.len() as u32,
                physical_cores: System::physical_core_count().map(|n| n as u32).unwrap_or(0),
                host: System::host_name().unwrap_or_default(),
                os: System::long_os_version().unwrap_or_default(),
                boot_time: System::boot_time().unwrap_or(0),
                volumes: Vec::new(),
            }
        })
        .clone();
    if let Ok(disks) = Disks::new_with_refreshed_list() {
        info.volumes = disks
            .iter()
            .map(|d| Volume {
                mount: d.mount_point().to_string_lossy().into_owned(),
                label: d.name().to_string_lossy().into_owned(),
                total: d.total_space(),
                free: d.available_space(),
            })
            .collect();
    }
    info
}

#[tauri::command(async)]
fn kill_process(pid: u32, name: String, tree: bool) -> CmdResult<procs::KillReport> {
    procs::kill(pid, &name, tree)
}

#[tauri::command(async)]
fn process_details(pid: u32) -> CmdResult<procs::ProcDetails> {
    procs::details(pid)
}

#[tauri::command(async)]
fn proc_icon(pid: u32) -> Option<String> {
    let exe = procs::exe_path(pid)?;
    icons::data_url(&exe).map(|url| url.to_string())
}

/// The file a request is about: a running process's program, or a path the screen already
/// has (a start-up entry). Only files that exist are accepted.
fn target_file(pid: Option<u32>, path: Option<String>) -> CmdResult<String> {
    let file = match (pid, path) {
        (Some(pid), _) => procs::exe_path(pid).unwrap_or_default(),
        (None, Some(path)) => path,
        (None, None) => String::new(),
    };
    if file.is_empty() || !std::path::Path::new(&file).exists() {
        return Err(CmdError::failed("Windows does not reveal where this program is."));
    }
    Ok(file)
}

#[tauri::command(async)]
fn open_file_location(pid: Option<u32>, path: Option<String>) -> CmdResult<()> {
    use std::os::windows::process::CommandExt;
    let exe = target_file(pid, path)?;
    if exe.contains('"') {
        return Err(CmdError::invalid("Unexpected program path."));
    }
    std::process::Command::new("explorer.exe")
        .raw_arg(format!("/select,\"{exe}\""))
        .spawn()
        .map(|_| ())
        .map_err(|e| CmdError::failed(format!("Could not open the folder: {e}")))
}

#[tauri::command(async)]
fn show_properties(pid: Option<u32>, path: Option<String>) -> CmdResult<()> {
    actions::show_properties(&target_file(pid, path)?)
}

#[tauri::command(async)]
fn proc_static(pids: Vec<u32>) -> Vec<procs::StaticInfo> {
    pids.into_iter().take(4096).map(procs::static_info).collect()
}

#[tauri::command(async)]
fn set_priority(state: State<AppState>, pid: u32, name: String, priority: String) -> CmdResult<&'static str> {
    let now = actions::set_priority(pid, &name, &priority)?;
    state.shared.update(|c| c.recheck = true);
    Ok(now)
}

/// Processor masks travel as text: the screen cannot hold 64-bit whole numbers exactly.
#[tauri::command(async)]
fn set_affinity(pid: u32, name: String, mask: String) -> CmdResult<String> {
    let mask = mask.parse::<u64>().map_err(|_| CmdError::invalid("Unknown processor selection."))?;
    actions::set_affinity(pid, &name, mask).map(|m| m.to_string())
}

#[tauri::command(async)]
fn set_efficiency(state: State<AppState>, pid: u32, name: String, on: bool) -> CmdResult<()> {
    actions::set_efficiency(pid, &name, on)?;
    state.shared.update(|c| c.recheck = true);
    Ok(())
}

#[tauri::command(async)]
fn set_paused(state: State<AppState>, pid: u32, name: String, pause: bool) -> CmdResult<()> {
    actions::set_paused(pid, &name, pause, std::process::id())?;
    state.shared.update(|c| c.recheck = true);
    Ok(())
}

#[tauri::command]
async fn create_dump(pid: u32, name: String) -> CmdResult<String> {
    // Writing a large program's memory to disk can take a while.
    tauri::async_runtime::spawn_blocking(move || actions::create_dump(pid, &name))
        .await
        .map_err(|e| CmdError::failed(format!("Internal error: {e}")))?
}

#[tauri::command(async)]
fn search_online(query: String) -> CmdResult<()> {
    actions::search_online(&query)
}

#[tauri::command(async)]
fn run_task(command: String, admin: bool) -> CmdResult<()> {
    actions::run_task(&command, admin)
}

#[tauri::command(async)]
fn switch_to(pid: u32) -> CmdResult<()> {
    actions::switch_to(pid)
}

#[tauri::command]
async fn restart_explorer() -> CmdResult<()> {
    tauri::async_runtime::spawn_blocking(actions::restart_explorer)
        .await
        .map_err(|e| CmdError::failed(format!("Internal error: {e}")))?
}

/// Opens one of a fixed set of Windows tools; the screen cannot ask for anything else.
#[tauri::command(async)]
fn open_tool(tool: String) -> CmdResult<()> {
    let target = match tool.as_str() {
        "services" => "services.msc",
        "resource-monitor" => "resmon.exe",
        "startup-settings" => "ms-settings:startupapps",
        _ => return Err(CmdError::invalid("Unknown tool.")),
    };
    actions::open(target)
}

#[tauri::command(async)]
fn list_users() -> Vec<users::UserSession> {
    users::list()
}

#[tauri::command(async)]
fn user_action(id: u32, action: String) -> CmdResult<()> {
    match action.as_str() {
        "disconnect" => users::disconnect(id),
        "sign-off" => users::sign_off(id),
        _ => Err(CmdError::invalid("Unknown action.")),
    }
}

#[tauri::command(async)]
fn hardware_info() -> hardware::Hardware {
    static FIXED: OnceLock<hardware::Hardware> = OnceLock::new();
    let mut info = FIXED.get_or_init(hardware::read).clone();
    // Addresses and link speed change when a cable or Wi-Fi does; read those each time.
    info.nets = hardware::read_nets();
    info
}

#[tauri::command(async)]
fn list_startup() -> Vec<startup::StartupEntry> {
    startup::list(&startup::Roots::real())
}

#[tauri::command(async)]
fn set_startup_enabled(id: String, enabled: bool) -> CmdResult<bool> {
    startup::set_enabled(&startup::Roots::real(), &id, enabled)
}

#[tauri::command(async)]
fn list_services(fresh: bool) -> CmdResult<Vec<services::ServiceInfo>> {
    services::list(fresh)
}

#[tauri::command]
async fn service_action(name: String, action: String) -> CmdResult<()> {
    // Can take seconds (a service stopping), so it runs off the threads that serve the window.
    tauri::async_runtime::spawn_blocking(move || services::act(&name, &action))
        .await
        .map_err(|e| CmdError::failed(format!("Internal error: {e}")))?
}

#[tauri::command]
fn restart_elevated(app: AppHandle) -> CmdResult<()> {
    elevate::relaunch_elevated()?;
    app.exit(0);
    Ok(())
}

/// Saves settings. Everything is applied first; if any part is refused (a taken hotkey),
/// nothing is saved and the previous settings stay active.
#[tauri::command]
fn save_settings(app: AppHandle, state: State<AppState>, settings: Settings) -> CmdResult<AppStatus> {
    let new = settings.sanitized();
    let old = state.settings.lock().map(|s| s.clone()).unwrap_or_default();
    let hotkey_was_broken = state.hotkey_error.lock().map(|e| e.is_some()).unwrap_or(false);

    if new.hotkey != old.hotkey || hotkey_was_broken {
        let previous = if hotkey_was_broken { "" } else { old.hotkey.as_str() };
        apply_hotkey(&app, previous, &new.hotkey).map_err(CmdError::invalid)?;
        if let Ok(mut e) = state.hotkey_error.lock() {
            *e = None;
        }
    }

    if new.ctrl_shift_esc != old.ctrl_shift_esc || (new.ctrl_shift_esc && !keyhook::is_enabled()) {
        if new.ctrl_shift_esc {
            if let Err(e) = keyhook::enable() {
                let _ = apply_hotkey(&app, &new.hotkey, &old.hotkey);
                log_line(&format!("Ctrl+Shift+Esc listener refused: {e}"));
                return Err(CmdError::failed("Windows refused to let this app take Ctrl+Shift+Esc."));
            }
        } else {
            keyhook::disable();
        }
        if let Ok(mut e) = state.takeover_error.lock() {
            *e = None;
        }
    }
    let undo_takeover = |old: &Settings| {
        if old.ctrl_shift_esc {
            let _ = keyhook::enable();
        } else {
            keyhook::disable();
        }
    };

    if new.autostart != old.autostart {
        let auto = app.autolaunch();
        let result = if new.autostart { auto.enable() } else { auto.disable() };
        if let Err(e) = result {
            // Put the hotkeys back so a refused save changes nothing at all.
            undo_takeover(&old);
            let _ = apply_hotkey(&app, &new.hotkey, &old.hotkey);
            log_line(&format!("autostart change failed: {e}"));
            return Err(CmdError::failed("Windows refused to change the start-up setting."));
        }
    }

    if let Err(e) = settings::save(&state.settings_path, &new) {
        undo_takeover(&old);
        let _ = apply_hotkey(&app, &new.hotkey, &old.hotkey);
        if new.autostart != old.autostart {
            let auto = app.autolaunch();
            let _ = if old.autostart { auto.enable() } else { auto.disable() };
        }
        log_line(&format!("settings save failed: {e}"));
        return Err(CmdError::failed(format!("Could not save the settings: {e}")));
    }

    if let Some(window) = app.get_webview_window("main") {
        let _ = window.set_always_on_top(new.always_on_top);
    }
    state.shared.update(|c| c.interval_ms = new.interval_ms);
    if let Ok(mut s) = state.settings.lock() {
        *s = new;
    }
    Ok(status_of(&app, &state))
}

// ───────────────────────── start-up ─────────────────────────

/// True for a copy put in place by the installer (it has its uninstaller beside it), false
/// for a build run straight from the project folder.
fn is_installed_copy() -> bool {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("uninstall.exe").is_file()))
        .unwrap_or(false)
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &quit])?;
    let mut tray = TrayIconBuilder::with_id("main")
        .tooltip("Tasker")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => show_main(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                toggle_main(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

pub fn run() {
    let args: Vec<String> = std::env::args().collect();
    // Restarted as administrator: let the old copy finish first so only one ever runs.
    if let Some(pid) = elevate::wait_pid_from_args(&args) {
        elevate::wait_for_exit(pid);
    }
    let start_hidden = args.iter().any(|a| a == HIDDEN_ARG);

    std::panic::set_hook(Box::new(|info| log_line(&format!("panic: {info}"))));

    let result = tauri::Builder::default()
        // The window toolkit asks Windows for "raw" keyboard input while our window has focus, and
        // Windows then stops calling this program's own keyboard listener — so Ctrl+Shift+Esc opened
        // Windows Task Manager whenever our window was in front. We never use raw device events.
        .device_event_filter(tauri::DeviceEventFilter::Always)
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_main(app)))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        toggle_main(app);
                    }
                })
                .build(),
        )
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![HIDDEN_ARG]),
        ))
        .plugin(
            tauri_plugin_window_state::Builder::default()
                // Not DECORATIONS: a state saved while the window still had the Windows frame
                // would put that frame back above the app's own title bar.
                .with_state_flags(
                    tauri_plugin_window_state::StateFlags::all()
                        & !tauri_plugin_window_state::StateFlags::VISIBLE
                        & !tauri_plugin_window_state::StateFlags::DECORATIONS,
                )
                .build(),
        )
        .setup(move |app| {
            let handle = app.handle().clone();
            if let Ok(dir) = app.path().app_log_dir() {
                let _ = std::fs::create_dir_all(&dir);
                let _ = LOG_PATH.set(dir.join("errors.log"));
            }
            let settings_path = app
                .path()
                .app_config_dir()
                .map(|d| d.join("settings.json"))
                .unwrap_or_else(|_| PathBuf::from("settings.json"));
            // No Windows frame: keep a taskbar that hides itself reachable under the maximised window.
            if let Some(hwnd) = app.get_webview_window("main").and_then(|w| w.hwnd().ok()) {
                winframe::keep_taskbar_reachable(hwnd.0 as isize);
            }
            // The app had another name once. Its own "start with Windows" entry moves to the
            // present name, so start-up keeps opening this program and not a file that is gone.
            let carried = startup::take_old_autostart();
            let mut loaded = settings::load(&settings_path);
            // The start-up switch lives in Windows; show what is really set there.
            loaded.autostart = app.autolaunch().is_enabled().unwrap_or(loaded.autostart);
            if carried == Some(true) && !loaded.autostart {
                loaded.autostart = app.autolaunch().enable().is_ok();
            }
            // An installed copy makes sure "Start with Windows" starts this copy, not an
            // older one from another folder.
            if loaded.autostart && is_installed_copy() {
                let _ = app.autolaunch().enable();
            }

            let opener = handle.clone();
            keyhook::on_open(move || {
                let app = opener.clone();
                let _ = opener.run_on_main_thread(move || {
                    keyhook::mask_modifiers();
                    toggle_main(&app);
                });
            });
            let takeover_error = if loaded.ctrl_shift_esc { keyhook::enable().err() } else { None };
            if let Some(e) = &takeover_error {
                log_line(&format!("Ctrl+Shift+Esc listener could not start: {e}"));
            }

            let shared = sampler::Shared::new(loaded.interval_ms);
            let hotkey_error = apply_hotkey(&handle, "", &loaded.hotkey).err();
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_always_on_top(loaded.always_on_top);
            }

            app.manage(AppState {
                shared: shared.clone(),
                settings: Mutex::new(loaded),
                settings_path,
                hotkey_error: Mutex::new(hotkey_error),
                takeover_error: Mutex::new(takeover_error),
                start_hidden,
                shown_once: AtomicBool::new(false),
            });

            build_tray(&handle)?;
            sampler::spawn(handle.clone(), shared);

            // Safety net: if the screen never reports ready (a broken page), show the window
            // anyway so the app is never an invisible process.
            if !start_hidden {
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_secs(4));
                    let shown = handle.state::<AppState>().shown_once.load(Ordering::Relaxed);
                    if !shown {
                        log_line("window shown by the safety net: the page did not report ready in 4 s");
                        show_main(&handle);
                    }
                });
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            let app = window.app_handle();
            match event {
                WindowEvent::CloseRequested { api, .. } => {
                    let to_tray = app
                        .try_state::<AppState>()
                        .and_then(|s| s.settings.lock().ok().map(|s| s.close_to_tray))
                        .unwrap_or(true);
                    if to_tray {
                        api.prevent_close();
                        hide_main(app);
                    } else {
                        app.exit(0);
                    }
                }
                WindowEvent::Resized(_) => {
                    // Minimising arrives as a resize; a minimised window needs no updates.
                    let minimized = window.is_minimized().unwrap_or(false);
                    let visible = window.is_visible().unwrap_or(false);
                    let to_tray = app
                        .try_state::<AppState>()
                        .and_then(|s| s.settings.lock().ok().map(|s| s.hide_when_minimized))
                        .unwrap_or(false);
                    if minimized && to_tray {
                        let _ = window.unminimize();
                        hide_main(app);
                    } else {
                        set_visible(app, visible && !minimized);
                    }
                }
                _ => {}
            }
        })
        .invoke_handler(tauri::generate_handler![
            ui_ready,
            app_status,
            set_view,
            refresh_now,
            get_history,
            hardware_info,
            proc_static,
            set_priority,
            set_affinity,
            set_efficiency,
            set_paused,
            create_dump,
            show_properties,
            search_online,
            run_task,
            switch_to,
            restart_explorer,
            open_tool,
            list_users,
            user_action,
            system_info,
            kill_process,
            process_details,
            proc_icon,
            open_file_location,
            list_startup,
            set_startup_enabled,
            list_services,
            service_action,
            restart_elevated,
            save_settings,
            window_minimize,
            window_toggle_maximize,
            window_is_maximized,
            window_close,
        ])
        .run(tauri::generate_context!());

    if let Err(e) = result {
        log_line(&format!("fatal: {e}"));
        eprintln!("Tasker could not start: {e}");
    }
}
