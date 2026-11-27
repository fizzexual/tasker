// Everything the screen asks the Rust side, in one typed place.

import type { Proc, ProcessIdentity, RawProc, StaticInfo } from "./model";

export interface DiskSample {
  name: string;
  active: number;
  read: number;
  write: number;
  response_ms: number;
}
export interface NetSample {
  name: string;
  rx: number;
  tx: number;
}
export interface GpuSample {
  name: string;
  util: number;
  mem_used: number;
  mem_total: number;
  shared_used: number;
  temp: number | null;
  /** Load per kind of work: [["3D", 12], ["VideoDecode", 3]]. */
  engines: [string, number][];
}
export interface PerfSample {
  t: number;
  cpu: number;
  cores: number[];
  mem_used: number;
  mem_total: number;
  swap_used: number;
  swap_total: number;
  disks: DiskSample[];
  nets: NetSample[];
  gpus: GpuSample[];
  procs: number;
  threads: number;
  handles: number;
  cpu_mhz: number;
  committed: number;
  commit_limit: number;
  cached: number;
  paged_pool: number;
  nonpaged_pool: number;
}
export interface Tick {
  perf: PerfSample;
  procs: RawProc[] | null;
  cost_ms: number;
}
export interface Volume {
  mount: string;
  label: string;
  total: number;
  free: number;
}
export interface SystemInfo {
  cpu_name: string;
  physical_cores: number;
  logical_cores: number;
  host: string;
  os: string;
  boot_time: number;
  volumes: Volume[];
}
export interface Hardware {
  cpu: { base_mhz: number; sockets: number; l1: number; l2: number; l3: number; virtualization: boolean };
  memory: { installed: number; slots_used: number; slots_total: number; speed_mhz: number; form_factor: string };
  disks: { index: number; model: string; kind: string; size: number }[];
  nets: { name: string; description: string; kind: string; ipv4: string; ipv6: string; link_bps: number }[];
  gpus: { name: string; driver_version: string; driver_date: string }[];
  bios_ms: number;
}
export type TabName = "processes" | "performance" | "users" | "startup" | "services";
export type Theme = "system" | "light" | "dark";
export interface Settings {
  /** 0 = paused. */
  interval_ms: number;
  hotkey: string;
  autostart: boolean;
  close_to_tray: boolean;
  always_on_top: boolean;
  ctrl_shift_esc: boolean;
  hide_when_minimized: boolean;
  start_tab: TabName;
  theme: Theme;
}
export interface AppStatus {
  version: string;
  elevated: boolean;
  settings: Settings;
  hotkey_error: string | null;
  takeover_error: string | null;
  own_pid: number;
  session: number;
  /** [Esc key events the Ctrl+Shift+Esc listener was handed, times it opened the app]. */
  takeover_stats: [number, number, number, boolean];
}
export interface UserSession {
  id: number;
  user: string;
  domain: string;
  state: "active" | "disconnected" | "other";
  current: boolean;
}
export type Priority = "realtime" | "high" | "above-normal" | "normal" | "below-normal" | "low";
export interface KillReport {
  ended: ProcessIdentity[];
  failed: { pid: number; name: string; code: string; message: string }[];
}
export interface ProcDetails {
  pid: number;
  name: string;
  exe: string;
  command: string;
  user: string;
  description: string;
  company: string;
  parent_pid: number;
  parent_name: string;
  start_ms: number;
  memory: number;
  working_set: number;
  peak_working_set: number;
  commit: number;
  page_faults: number;
  cpu_time_ms: number;
  threads: number;
  handles: number;
  session: number;
  priority: Priority | "";
  /** Bit masks as text (64-bit numbers do not fit a JS number). */
  affinity: string;
  system_affinity: string;
  elevated: boolean | null;
  architecture: string;
  efficiency: boolean;
  suspended: boolean;
}
export interface StartupEntry {
  id: string;
  name: string;
  command: string;
  location: string;
  enabled: boolean;
  all_users: boolean;
  path: string;
  publisher: string;
}
export interface ServiceInfo {
  name: string;
  display: string;
  status: "running" | "stopped" | "starting" | "stopping" | "paused" | "other";
  start: string;
  pid: number;
  description: string;
  group: string;
}

/** The error shape every command uses. `code` says what kind of failure it is. */
export interface CmdError {
  code: "needs-admin" | "not-found" | "protected" | "cancelled" | "invalid" | "failed";
  message: string;
}

export function asError(e: unknown): CmdError {
  if (e && typeof e === "object" && "code" in e && "message" in e) return e as CmdError;
  return { code: "failed", message: typeof e === "string" ? e : "Something went wrong." };
}

export interface Backend {
  invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T>;
  onTick(handler: (tick: Tick) => void): Promise<void>;
}

let backend: Backend | null = null;

async function getBackend(): Promise<Backend> {
  if (backend) return backend;
  if ("__TAURI_INTERNALS__" in window) {
    const [{ invoke }, { listen }] = await Promise.all([import("@tauri-apps/api/core"), import("@tauri-apps/api/event")]);
    backend = {
      invoke: (cmd, args) => invoke(cmd, args),
      onTick: async (handler) => {
        await listen<Tick>("tick", (e) => handler(e.payload));
      },
    };
  } else if (import.meta.env.DEV) {
    // Opened in a normal browser during development: made-up data, so the screens can be worked on.
    backend = (await import("./mock")).mockBackend;
  } else {
    throw new Error("Not running inside the app.");
  }
  return backend;
}

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  return (await getBackend()).invoke<T>(cmd, args);
}

type ProcessTarget = Pick<Proc, "pid" | "name" | "creationTime">;

function targetArgs({ pid, name, creationTime }: ProcessTarget) {
  return { pid, name, creationTime };
}

export const api = {
  onTick: async (handler: (tick: Tick) => void) => (await getBackend()).onTick(handler),
  uiReady: () => call<void>("ui_ready"),
  status: () => call<AppStatus>("app_status"),
  setView: (tab: string) => call<void>("set_view", { tab }),
  history: () => call<PerfSample[]>("get_history"),
  systemInfo: () => call<SystemInfo>("system_info"),
  kill: (target: ProcessTarget, tree: boolean) => call<KillReport>("kill_process", { ...targetArgs(target), tree }),
  details: (pid: number) => call<ProcDetails>("process_details", { pid }),
  icon: (pid: number) => call<string | null>("proc_icon", { pid }),
  openLocation: (pid: number) => call<void>("open_file_location", { pid }),
  openPath: (path: string) => call<void>("open_file_location", { path }),
  properties: (pid: number) => call<void>("show_properties", { pid }),
  propertiesOf: (path: string) => call<void>("show_properties", { path }),
  staticInfo: (pids: number[]) => call<(StaticInfo & { pid: number })[]>("proc_static", { pids }),
  setPriority: (target: ProcessTarget, priority: Priority) => call<Priority>("set_priority", { ...targetArgs(target), priority }),
  setAffinity: (target: ProcessTarget, mask: string) => call<string>("set_affinity", { ...targetArgs(target), mask }),
  setEfficiency: (target: ProcessTarget, on: boolean) => call<void>("set_efficiency", { ...targetArgs(target), on }),
  setPaused: (target: ProcessTarget, pause: boolean) => call<void>("set_paused", { ...targetArgs(target), pause }),
  createDump: (target: ProcessTarget) => call<string>("create_dump", targetArgs(target)),
  searchOnline: (query: string) => call<void>("search_online", { query }),
  runTask: (command: string, admin: boolean) => call<void>("run_task", { command, admin }),
  switchTo: (pid: number) => call<void>("switch_to", { pid }),
  restartExplorer: () => call<void>("restart_explorer"),
  openTool: (tool: "services" | "resource-monitor" | "startup-settings") => call<void>("open_tool", { tool }),
  users: () => call<UserSession[]>("list_users"),
  userAction: (id: number, action: "disconnect" | "sign-off") => call<void>("user_action", { id, action }),
  hardware: () => call<Hardware>("hardware_info"),
  refreshNow: () => call<void>("refresh_now"),
  startup: () => call<StartupEntry[]>("list_startup"),
  setStartup: (id: string, enabled: boolean) => call<boolean>("set_startup_enabled", { id, enabled }),
  services: (fresh: boolean) => call<ServiceInfo[]>("list_services", { fresh }),
  serviceAction: (name: string, action: "start" | "stop" | "restart") =>
    call<void>("service_action", { name, action }),
  restartElevated: () => call<void>("restart_elevated"),
  saveSettings: (settings: Settings) => call<AppStatus>("save_settings", { settings }),
  // The app's own title bar buttons.
  windowMinimize: () => call<void>("window_minimize"),
  /** Returns the new state: true = now fills the screen. */
  windowToggleMaximize: () => call<boolean>("window_toggle_maximize"),
  windowIsMaximized: () => call<boolean>("window_is_maximized"),
  windowClose: () => call<void>("window_close"),
};
