// A stand-in for the Tauri core with a fixed room of four members and seven recordings, for the screen tests.
(() => {
  const now = Date.now() * 1000;
  const H = 3600e6, M = 60e6;
  const ME = "a1me", B = "b2kenji", C = "c3mika", D = "d4sora";
  const mkPlayer = (owner, start, end, extra = {}) => ({ owner, imported: false, start_us: start, end_us: end, offset_us: 0, have: 100, total: 100, recording: false, verified_by: [ME, B], stuck: false, parts: [], last_us: { v: end, m: end }, ...extra });
  const sets = [];
  const evs = [];
  let n = 0;
  const day = 24 * H;
  const starts = [now - 2 * H, now - 5 * H, now - day - 3 * H, now - day - 6 * H, now - 3 * day, now - 6 * day, now - 9 * day];
  starts.forEach((s, i) => {
    const len = (25 + i * 7) * M;
    const d = new Date(s / 1000); const z = (x) => String(x).padStart(2, "0"); const id = `${d.getFullYear()}-${z(d.getMonth() + 1)}-${z(d.getDate())} ${z(d.getHours())}-${z(d.getMinutes())}-${z(d.getSeconds())}`;
    const keys = [];
    for (let k = 0; k < 3 + (i % 3); k++) {
      const key = `${[ME, B, C][k % 3]}/${++n}`;
      keys.push(key);
      evs.push({ key, id: n, author: [ME, B, C][k % 3], plugin: null, kind: "event", local_us: s + (k + 1) * 4 * M, end_us: null, set: id, file: null, text: ["ここで詰めるべきだった #反省", "ナイスカバー！", "リングの読みミス #ring", "回復しながら前に出すぎ", "@Kenji ここ射線通ってた"][k % 5], values: null, tags: k % 2 ? [] : ["反省"], mentions: [], deleted: false, run: null, source: null, at_us: s + (k + 1) * 4 * M, seekable: true, mine: k % 3 === 0 });
    }
    sets.push({ id, room: "r1", start_us: s, end_us: s + len, live: false, thumb: true, title: null, imported_by: null,
      players: [mkPlayer(ME, s, s + len), mkPlayer(B, s, s + len), ...(i % 2 ? [] : [mkPlayer(C, s, s + len)])],
      events: keys, progress: { have: Math.round((0.8 + i * 0.37) * 1e9), total: Math.round((0.8 + i * 0.37) * 1e9), stuck: false }, wanted: false, shared: true, layout: null, of: null, debriefs: [], analysis: [] });
  });
  const state = {
    app: "EZ Hanseikai", version: "0.31.0", now_us: now, clock: { offset_us: 1200, rtt_us: 30000, ago_s: 40, ok: true },
    ffmpeg: "ready", ffmpeg_progress: null, id: ME, name: "Taro", lang: "ja", language: "ja",
    colors: { [ME]: 0, [B]: 1, [C]: 2, [D]: 3 }, avatars: {}, invites: 0, limit_mbps: null, relays: [],
    recovery: null, notice: null, disk: { free: 300e9, total: 1000e9, per_hour: 2e9, hours_left: 150, low: false, full: false },
    previous_version: null, crashed: false, recording: false,
    capture: { source: { kind: "window", title: "Apex Legends" }, source_lost: false, sharing: true, working: false, stalled: false, status: "", mic: "", mix: { app: { volume: 1, muted: false }, mic: { volume: 1, muted: false } }, quality: { id: "standard", size: [1920, 1080], fps: 60, kbps: 8000, codec: "hevc" }, live: { path: "live/r1/a1me/1", size: 1000, start_us: now - 60e6 } },
    active_set: null,
    room: { id: "r1", name: "Apex 反省会", icon: "", pending: false, waiting: false },
    rooms: [{ id: "r1", name: "Apex 反省会", icon: "", active: true, used_us: now }, { id: "r2", name: "ヴァロ部", icon: "", active: false, used_us: now - day }],
    layout: null, hotkeys: [{ action: "mark", keys: "Ctrl+Alt+M", default: "Ctrl+Alt+M", taken: false }], notify_joins: true,
    api: { on: false, address: "127.0.0.1:47123", error: null },
    sets, peers: [
      { id: B, name: "Kenji", connected: true, path: "direct", rtt_ms: 18, relay: null, sent: 1e6, received: 2e6, failures: 0, seen_us: now, offset_us: 0, version: "0.31.0", other_version: false, sharing: false, speaking: false },
      { id: C, name: "Mika", connected: true, path: "relay", rtt_ms: 42, relay: "relay", sent: 1e6, received: 2e6, failures: 0, seen_us: now, offset_us: 0, version: "0.31.0", other_version: false, sharing: false, speaking: false },
      { id: D, name: "Sora", connected: false, path: null, rtt_ms: null, relay: null, sent: null, received: null, failures: 0, seen_us: now - 2 * day, offset_us: 0, version: "0.30.2", other_version: false, sharing: false, speaking: false },
    ],
    events: evs, tag_names: {}, board: [], names: { [ME]: "You", [B]: "Kenji", [C]: "Mika", [D]: "Sora" }, asks: [], party: { on: false, watching: [] },
  };
  window.__mock = { state, overrides: {} };
  const answers = {
    state: () => JSON.parse(JSON.stringify(window.__mock.state)),
    list_sources: () => [{ id: { kind: "window", title: "Apex Legends" }, width: 1920, height: 1080 }, { id: { kind: "window", title: "Discord" }, width: 1280, height: 720 }, { id: { kind: "display", index: 1 }, width: 2560, height: 1440 }],
    list_mics: () => [{ id: "m1", name: "マイク (USB Audio)" }],
    data_info: () => ({ dir: "C:\\Users\\user\\AppData\\Roaming\\ez-hanseikai", used_bytes: 42e9, free_bytes: 300e9 }),
    api_info: () => ({ on: false, address: "127.0.0.1:47123", token: "", error: null }),
    api_reference: () => [],
    room_plugins: () => [],
    debug_flow: () => ({ members: [], relay: "relay.example.net", download: null, update: { host: "github.com" }, report: { host: "reports.example.net" }, time: { host: "time.cloudflare.com", ok: true }, api: { on: false } }),
    check_update: () => null,
    check: () => { const L = (level, text, args) => ({ level, text, args: args || {} }); const I = (item, lines) => ({ item, level: lines.some((l) => l.level === "warn") ? "warn" : "ok", lines }); return { pc: [I("Windows", [L("ok", "Windows build {build}", { build: 26100 })]), I("WebView2", [L("ok", "WebView2 {version}", { version: "130.0" })]), I("Screen capture", [L("ok", "Screen capture works")]), I("App sound", [L("ok", "App sound works")]), I("Encoder", [L("ok", "NVIDIA NVENC")])], net: [I("UDP", [L("ok", "UDP works")]), I("Direct connections", [L("warn", "This network changes the address per destination")]), I("Relay server", [L("ok", "Reachable")])] }; },
    export_target: () => "C:\\Users\\user\\Videos",
    party_on: () => null,
  };
  const call = async (name, args) => {
    const o = window.__mock.overrides[name];
    if (o) return o(args);
    if (answers[name]) return answers[name](args);
    return null;
  };
  window.__TAURI__ = {
    core: {
      invoke: (cmd, args) => (cmd === "call" ? call(args.name, args.args) : call(cmd, args)),
      convertFileSrc: (p, scheme) => `/media/${p}`,
    },
    event: { listen: async () => () => {}, emit: async () => {} },
    window: { getCurrentWindow: () => ({ setTitle: async () => {}, isFullscreen: async () => false, setFullscreen: async () => {}, onCloseRequested: async () => () => {}, listen: async () => () => {} }) },
  };
})();
