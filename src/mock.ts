// Development only: made-up data so the screens can be opened in a normal browser.
// It is loaded through a DEV-only dynamic import, so none of this ships in the app.

import type { AppStatus, Backend, Hardware, PerfSample, ProcDetails, ServiceInfo, Settings, StartupEntry, Tick, UserSession } from "./api";
import { F_APP, F_ECO, F_HUNG, F_SUSPENDED, F_WINDOWS, fromRaw, type RawProc } from "./model";

/** [file name, friendly name, flags] */
const PROGRAMS: [string, string, number][] = [
  ["chrome.exe", "Google Chrome", F_APP],
  ["Code.exe", "Visual Studio Code", F_APP],
  ["explorer.exe", "Windows Explorer", F_APP | F_WINDOWS],
  ["svchost.exe", "Host Process for Windows Services", F_WINDOWS],
  ["Discord.exe", "Discord", 0],
  ["steam.exe", "Steam", 0],
  ["node.exe", "Node.js JavaScript Runtime", 0],
  ["dwm.exe", "Desktop Window Manager", F_WINDOWS],
  ["System", "", F_WINDOWS],
  ["csrss.exe", "Client Server Runtime Process", F_WINDOWS],
  ["blender.exe", "Blender", F_APP | F_HUNG],
  ["rustc.exe", "", F_SUSPENDED],
  ["SearchHost.exe", "Search", F_ECO],
];

let settings: Settings = {
  interval_ms: 1000,
  hotkey: "Ctrl+Alt+Escape",
  autostart: false,
  close_to_tray: true,
  always_on_top: false,
  ctrl_shift_esc: true,
  hide_when_minimized: false,
  start_tab: "processes",
  theme: "system",
};
const status = (): AppStatus => ({
  version: "dev",
  elevated: false,
  settings,
  hotkey_error: null,
  takeover_error: null,
  own_pid: 1000,
  session: 1,
  takeover_stats: [0, 0, 0, true],
});

const START = Date.now() - 3_600_000;
let procs: RawProc[] = Array.from({ length: 420 }, (_, i) => {
  const [name, , flags] = PROGRAMS[i % PROGRAMS.length];
  const mem = (20 + ((i * 37) % 600)) * 1024 * 1024;
  const app = flags & F_APP;
  // Only the first process of an app owns its window.
  const f = i < PROGRAMS.length ? flags : flags & ~(F_APP | F_HUNG);
  return [
    1000 + i * 4,
    4,
    name,
    0,
    mem,
    0,
    0,
    0,
    8 + (i % 40),
    200 + ((i * 13) % 900),
    flags & F_WINDOWS && name !== "explorer.exe" ? 0 : 1,
    8,
    i * 1500,
    `${START + i * 1000 + 11_644_473_600_000}0000`,
    mem * 1.4,
    mem * 1.8,
    f,
    app && i < PROGRAMS.length ? `${name.replace(".exe", "")} — window` : "",
  ] as RawProc;
});

const startup: StartupEntry[] = [
  { id: "user-run|Discord", name: "Discord", command: '"C:\\Users\\me\\AppData\\Local\\Discord\\Update.exe" --processStart Discord.exe', location: "Registry (this user)", enabled: true, all_users: false, path: "C:\\Users\\me\\AppData\\Local\\Discord\\Update.exe", publisher: "Discord Inc." },
  { id: "machine-run|SecurityHealth", name: "SecurityHealth", command: "%windir%\\system32\\SecurityHealthSystray.exe", location: "Registry (all users)", enabled: true, all_users: true, path: "C:\\Windows\\system32\\SecurityHealthSystray.exe", publisher: "Microsoft Corporation" },
  { id: "user-folder|Notes.lnk", name: "Notes.lnk", command: "C:\\Users\\me\\AppData\\Roaming\\Microsoft\\Windows\\Start Menu\\Programs\\Startup\\Notes.lnk", location: "Startup folder (this user)", enabled: false, all_users: false, path: "", publisher: "" },
  { id: "store|Example.App_abc\\Task", name: "Example Store App", command: "Example.App_abc", location: "Store app", enabled: true, all_users: false, path: "", publisher: "" },
];

const services: ServiceInfo[] = Array.from({ length: 260 }, (_, i) => ({
  name: `Service${String(i).padStart(3, "0")}`,
  display: `Example service number ${i}`,
  status: i % 3 === 0 ? "stopped" : "running",
  start: i % 4 === 0 ? "manual" : "automatic",
  pid: i % 3 === 0 ? 0 : 1000 + (i % 100) * 4,
  description: "A made-up service for development.",
  group: i % 5 === 0 ? "netsvcs" : i % 5 === 1 ? "LocalService" : "",
}));

const users: UserSession[] = [
  { id: 1, user: "me", domain: "DEV", state: "active", current: true },
  { id: 2, user: "guest", domain: "DEV", state: "disconnected", current: false },
];

const hardware: Hardware = {
  cpu: { base_mhz: 3800, sockets: 1, l1: 512 * 1024, l2: 8 * 1024 * 1024, l3: 32 * 1024 * 1024, virtualization: true },
  memory: { installed: 32 * 1024 ** 3, slots_used: 2, slots_total: 4, speed_mhz: 3600, form_factor: "DIMM" },
  disks: [
    { index: 0, model: "Example NVMe 1TB", kind: "SSD (NVMe)", size: 1e12 },
    { index: 1, model: "Example HDD 2TB", kind: "HDD", size: 2e12 },
  ],
  nets: [{ name: "Ethernet", description: "Example 2.5GbE Controller", kind: "Ethernet", ipv4: "192.168.1.20", ipv6: "fe80::1", link_bps: 2.5e9 }],
  gpus: [{ name: "Example GPU 4070", driver_version: "32.0.15.6094", driver_date: "2026-05-14" }],
  bios_ms: 9400,
};

function sample(t: number): PerfSample {
  const wave = (p: number, a: number, b: number) => a + (b - a) * (0.5 + 0.5 * Math.sin(t / p));
  return {
    t: Date.now(),
    cpu: wave(7, 4, 62),
    cores: Array.from({ length: 16 }, (_, i) => wave(3 + i, 0, 90)),
    mem_used: wave(40, 9, 13) * 1024 ** 3,
    mem_total: 32 * 1024 ** 3,
    swap_used: 2 * 1024 ** 3,
    swap_total: 8 * 1024 ** 3,
    disks: [
      { name: "0 C:", active: wave(5, 0, 40), read: wave(4, 0, 9e6), write: wave(6, 0, 4e6), response_ms: wave(5, 0.2, 3) },
      { name: "1 D:", active: wave(9, 0, 8), read: 0, write: wave(8, 0, 2e5), response_ms: wave(9, 2, 14) },
    ],
    nets: [{ name: "Ethernet", rx: wave(5, 1e3, 4e6), tx: wave(8, 1e3, 6e5) }],
    gpus: [
      {
        name: "Example GPU 4070",
        util: wave(6, 2, 55),
        mem_used: wave(30, 1.2, 3.4) * 1024 ** 3,
        mem_total: 12 * 1024 ** 3,
        shared_used: 3e8,
        temp: Math.round(wave(20, 38, 61)),
        engines: [
          ["3D", wave(6, 2, 55)],
          ["VideoDecode", wave(11, 0, 9)],
        ],
      },
    ],
    procs: procs.length,
    threads: 5200,
    handles: 180000,
    cpu_mhz: wave(9, 2100, 4700),
    committed: wave(40, 14, 19) * 1024 ** 3,
    commit_limit: 40 * 1024 ** 3,
    cached: 11 * 1024 ** 3,
    paged_pool: 9e8,
    nonpaged_pool: 6e8,
  };
}

const past: PerfSample[] = Array.from({ length: 60 }, (_, i) => sample(i));
let t = 60;
let fireNow: (() => void) | null = null;

const find = (pid: unknown) => procs.find((p) => p[0] === pid);
function target(args: Record<string, unknown>): RawProc {
  const p = find(args.pid);
  if (!p || p[2] !== args.name || p[13] !== args.creationTime) throw { code: "not-found", message: "That process has already ended." };
  return p;
}

const program = (name: string) => PROGRAMS.find((p) => p[0] === name);
const wait = (ms: number) => new Promise((res) => setTimeout(res, ms));
const setFlag = (p: RawProc, flag: number, on: boolean) => {
  p[16] = on ? p[16] | flag : p[16] & ~flag;
};

let mockMaximized = false;

export const mockBackend: Backend = {
  async invoke<T>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
    const r = (v: unknown) => v as T;
    switch (cmd) {
      case "app_status":
        return r(status());
      case "get_history":
        return r(past);
      case "system_info":
        return r({ cpu_name: "Example CPU 16-Core", physical_cores: 8, logical_cores: 16, host: "DEV", os: "Windows 11", boot_time: Date.now() / 1000 - 93784, volumes: [{ mount: "C:\\", label: "System", total: 1e12, free: 4.2e11 }, { mount: "D:\\", label: "Data", total: 2e12, free: 1.5e12 }] });
      case "hardware_info":
        return r(hardware);
      case "refresh_now":
        fireNow?.();
        return r(undefined);
      case "kill_process": {
        const p = target(args);
        const name = p[2];
        if (name === "System" || name === "csrss.exe") throw { code: "protected", message: `${name} is part of Windows. Ending it would stop the PC.` };
        if (name === "svchost.exe") throw { code: "needs-admin", message: `Ending ${name} needs administrator rights.` };
        procs = procs.filter((p) => p[0] !== args.pid);
        return r({ ended: [{ pid: p[0], creationTime: p[13] }], failed: [] });
      }
      case "proc_static":
        return r(
          (args.pids as number[]).flatMap((pid) => {
            const p = find(pid);
            if (!p) return [];
            const windows = Boolean(p[16] & F_WINDOWS);
            return [
              {
                pid,
                path: p[2] === "System" ? "" : `C:\\Program Files\\Example\\${p[2]}`,
                user: windows && p[2] !== "explorer.exe" ? "SYSTEM" : "me",
                description: program(p[2])?.[1] ?? "",
                company: windows ? "Microsoft Corporation" : "Example Ltd",
                architecture: "x64",
                elevated: windows && p[2] !== "explorer.exe",
                command: `"C:\\Program Files\\Example\\${p[2]}" --type=worker --id=${pid}`,
              },
            ];
          }),
        );
      case "process_details": {
        const p = find(args.pid);
        if (!p) throw { code: "not-found", message: "That process has already ended." };
        const d: ProcDetails = {
          pid: p[0],
          name: p[2],
          exe: `C:\\Program Files\\Example\\${p[2]}`,
          command: `"C:\\Program Files\\Example\\${p[2]}" --type=worker`,
          user: "me",
          description: program(p[2])?.[1] ?? "",
          company: "Example Ltd",
          parent_pid: 4,
          parent_name: "System",
          start_ms: fromRaw(p).startMs,
          memory: p[4],
          working_set: p[14],
          peak_working_set: p[14] * 1.2,
          commit: p[15],
          page_faults: 123456,
          cpu_time_ms: p[12],
          threads: p[8],
          handles: p[9],
          session: p[10],
          priority: p[11] === 13 ? "high" : p[11] === 4 ? "low" : "normal",
          affinity: "65535",
          system_affinity: "65535",
          elevated: false,
          architecture: "x64",
          efficiency: Boolean(p[16] & F_ECO),
          suspended: Boolean(p[16] & F_SUSPENDED),
        };
        return r(d);
      }
      case "set_priority": {
        await wait(300);
        const p = target(args);
        if (args.priority === "realtime") throw { code: "needs-admin", message: "Realtime priority needs administrator rights." };
        p[11] = ({ high: 13, "above-normal": 10, normal: 8, "below-normal": 6, low: 4 } as Record<string, number>)[String(args.priority)] ?? 8;
        return r(args.priority);
      }
      case "set_affinity":
        await wait(300);
        target(args);
        return r(args.mask);
      case "set_efficiency":
      case "set_paused": {
        await wait(300);
        const p = target(args);
        if (p[16] & F_WINDOWS) throw { code: "needs-admin", message: `Changing ${p[2]} needs administrator rights.` };
        if (cmd === "set_efficiency") setFlag(p, F_ECO, Boolean(args.on));
        else setFlag(p, F_SUSPENDED, Boolean(args.pause));
        return r(undefined);
      }
      case "create_dump":
        await wait(600);
        target(args);
        return r(`C:\\Users\\me\\AppData\\Local\\Temp\\${String(args.name)}.DMP`);
      case "run_task":
        if (!String(args.command).trim()) throw { code: "invalid", message: "Type the name of a program, folder or website." };
        if (String(args.command).includes("nope")) throw { code: "not-found", message: `Windows cannot find "${String(args.command)}".` };
        return r(undefined);
      case "proc_icon":
        return r(null);
      case "list_users":
        return r(users.map((u) => ({ ...u })));
      case "user_action":
        throw { code: "needs-admin", message: "Doing this to another person's session needs administrator rights." };
      case "list_startup":
        return r(startup.map((e) => ({ ...e })));
      case "set_startup_enabled": {
        const e = startup.find((x) => x.id === args.id)!;
        await wait(400);
        if (e.all_users) throw { code: "needs-admin", message: "Changing a startup entry for all users needs administrator rights." };
        e.enabled = Boolean(args.enabled);
        return r(e.enabled);
      }
      case "list_services":
        return r(services.map((s) => ({ ...s })));
      case "service_action": {
        const s = services.find((x) => x.name === args.name)!;
        await wait(900);
        if (s.name.endsWith("7")) throw { code: "needs-admin", message: `Stopping ${s.name} needs administrator rights.` };
        s.status = args.action === "stop" ? "stopped" : "running";
        s.pid = s.status === "running" ? 4000 : 0;
        return r(undefined);
      }
      case "save_settings": {
        const next = args.settings as Settings;
        if (next.hotkey === "Ctrl+Alt+Delete") throw { code: "invalid", message: "Ctrl+Alt+Delete is already used by another program." };
        settings = next;
        return r(status());
      }
      case "restart_elevated":
        throw { code: "cancelled", message: "Cancelled." };
      // A browser tab has no window of its own to maximise: the button only swaps its picture.
      case "window_toggle_maximize":
        mockMaximized = !mockMaximized;
        return r(mockMaximized);
      case "window_is_maximized":
        return r(mockMaximized);
      default:
        return r(undefined);
    }
  },
  async onTick(handler: (tick: Tick) => void): Promise<void> {
    const emit = () => {
      t++;
      const s = sample(t);
      for (const p of procs) {
        p[3] = p[16] & F_SUSPENDED ? 0 : Math.max(0, Math.sin(p[0] * 0.37 + t / 3) * 3 + (p[0] % 97 === 0 ? 14 : -1.5));
        p[5] = p[0] % 31 === 0 ? 2e5 * Math.abs(Math.sin(t / 4)) : 0;
        p[7] = p[2] === "blender.exe" || p[2] === "dwm.exe" ? Math.abs(Math.sin(t / 5 + p[0])) * 20 : 0;
        p[12] += p[3] * 10;
      }
      handler({ perf: s, procs: procs.map((p) => [...p] as RawProc), cost_ms: 3.2 });
    };
    fireNow = emit;
    const loop = () => {
      // Paused: nothing arrives until "refresh now".
      if (settings.interval_ms !== 0) emit();
      setTimeout(loop, settings.interval_ms || 500);
    };
    setTimeout(loop, 200);
  },
};
