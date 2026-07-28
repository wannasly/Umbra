// Browser-only mock backend so the UI can be developed and previewed with
// plain `npm run dev` (no Tauri shell). Never used inside the real app.

import type {
  ConnectionState,
  LogLine,
  ProxyNodeVless,
  ServerEntry,
  ServersList,
  Settings,
  Subscription,
} from "./ipc";

const now = () => Date.now();

const UUID = "bf000d23-0752-40b4-affe-68f7707a9661";

/** Keeps the fixtures readable: only what differs per server is spelled out. */
function server(
  id: string,
  name: string,
  host: string,
  extra: Partial<ProxyNodeVless> = {},
): ServerEntry {
  return {
    id,
    name,
    protocol: "vless",
    server: host,
    port: 443,
    uuid: UUID,
    flow: "xtls-rprx-vision",
    security: "reality",
    sni: "www.microsoft.com",
    fingerprint: "chrome",
    publicKey: "mockpublickey00000000000000000000000000000A",
    shortId: "6ba85179",
    insecure: false,
    alpn: [],
    transport: { type: "tcp" },
    lastPingMs: null,
    favorite: false,
    totalUp: 0,
    totalDown: 0,
    raw: `vless://${UUID}@${host}:443?security=reality&sni=www.microsoft.com&fp=chrome&pbk=mock&sid=6ba85179&flow=xtls-rprx-vision#${encodeURIComponent(name)}`,
    ...extra,
  };
}

const mockServers: ServerEntry[] = [
  {
    id: "srv-hy2",
    name: "🇫🇮 Helsinki HY2",
    protocol: "hysteria2",
    server: "fi.example.com",
    port: 443,
    password: "mockpassword",
    insecure: false,
    sni: "fi.example.com",
    alpn: ["h3"],
    lastPingMs: 38,
    favorite: false,
    totalUp: 500_000,
    totalDown: 12_000_000,
    raw: "hy2://mockpassword@fi.example.com:443#%D0%A4%D0%B8%D0%BD%D0%BB%D1%8F%D0%BD%D0%B4%D0%B8%D1%8F%20HY2",
  },
  server("srv-1", "🇳🇱 Amsterdam-1", "nl1.example.com", {
    lastPingMs: 43,
    favorite: true,
    totalUp: 41_300_000_000,
    totalDown: 318_700_000_000,
  }),
  server("srv-2", "🇩🇪 Frankfurt-2", "de2.example.com", {
    port: 8443,
    flow: "",
    security: "tls",
    sni: "de2.example.com",
    publicKey: "",
    shortId: "",
    transport: { type: "ws", path: "/ws", host: "de2.example.com" },
    lastPingMs: 71,
    totalUp: 2_140_000_000,
    totalDown: 9_820_000_000,
  }),
  server("srv-3", "🇯🇵 Tokyo-1", "jp.example.com", {
    flow: "",
    security: "tls",
    sni: "jp.example.com",
    publicKey: "",
    shortId: "",
    transport: { type: "grpc", serviceName: "grpc" },
    lastPingMs: 214,
  }),
];

/**
 * A subscription shaped like the panels our users are actually on: a proper
 * Profile-Title as the name, a metered plan, and a couple of pseudo-servers
 * that are really notices (the info-row heuristic has to catch these).
 */
const mockSubscription: Subscription = {
  id: "sub-1",
  name: "Example Network",
  url: "https://panel.example.com/sub/abcdef123456",
  updatedAt: new Date(now() - 2 * 3600_000).toISOString(),
  quota: {
    upload: 12_000_000_000,
    download: 88_000_000_000,
    total: 300_000_000_000,
    expire: Math.floor(now() / 1000) + 20 * 86400,
  },
  autoUpdateHours: 12,
  supportUrl: "https://support.example.com",
  webPageUrl: "https://panel.example.com/account",
  panelTitle: "Example Network",
  servers: [
    server("sub1-info-1", "Осталось дней — 19", "0.0.0.0", { port: 1 }),
    server(
      "sub1-info-2",
      "Если не работает, нажмите кнопку обновления",
      "127.0.0.1",
      { port: 1 },
    ),
    server("sub1-1", "🇵🇱 Poland #1", "pl1.example.com", { lastPingMs: 38 }),
    server("sub1-2", "🇵🇱 Poland #2", "pl2.example.com", { lastPingMs: 52 }),
    server("sub1-3", "🇫🇮 Finland #1", "fi1.example.com", {
      lastPingMs: 61,
      favorite: true,
      totalUp: 512_000_000,
      totalDown: 7_400_000_000,
    }),
    server("sub1-4", "Germany · Frankfurt", "de3.example.com", {
      lastPingMs: null,
    }),
    server("sub1-5", "US-2 New York", "us2.example.com", { lastPingMs: 173 }),
    mockServers[2],
  ],
};

/** Second account: unlimited plan (total=0) — the quota row used to vanish. */
const mockSubscription2: Subscription = {
  id: "sub-2",
  name: "Unlimited Demo",
  url: "https://unlimited.example.com/api/sub/example-token",
  updatedAt: new Date(now() - 26 * 3600_000).toISOString(),
  quota: {
    upload: 0,
    download: 127_879_592_133,
    total: 0,
    expire: Math.floor(now() / 1000) + 19 * 86400,
  },
  autoUpdateHours: 24,
  supportUrl: null,
  webPageUrl: null,
  panelTitle: null,
  servers: [
    server("sub2-info", "Подписка: https://unlimited.example.com/renew", "0.0.0.0", {
      port: 1,
    }),
    server("sub2-1", "NL-1 · reality", "nl9.example.com", { lastPingMs: 44 }),
    server("sub2-2", "Турция (Стамбул)", "tr1.example.com", { lastPingMs: 96 }),
    server("sub2-3", "Сервер без страны", "x1.example.com", { lastPingMs: 120 }),
  ],
};

const mockSubscriptions = [mockSubscription, mockSubscription2];

const state: {
  conn: ConnectionState;
  settings: Settings;
  logs: LogLine[];
} = {
  conn: {
    status: "disconnected",
    serverId: null,
    serverName: null,
    mode: "system_proxy",
    sinceMs: null,
    error: null,
  },
  settings: {
    version: 1,
    language: "ru",
    accent: "violet",
    mode: "system_proxy",
    mixedPort: 2080,
    selectedServerId: "srv-1",
    autostart: false,
    startMinimized: false,
    minimizeToTray: true,
    connectOnStartup: false,
    logLevel: "info",
    bypassRu: false,
    routeDefault: "proxy",
    appRoutes: [
      { id: "mock-browser", processName: "firefox.exe", action: "proxy" },
      { id: "mock-game", processName: "game.exe", action: "direct" },
      { id: "mock-block", processName: "telemetry.exe", action: "block" },
    ],
    tunStack: "mixed",
    tunStrictRoute: true,
    tunMtu: 9000,
    ipStrategy: "ipv4_only",
    pingUrl: "https://www.gstatic.com/generate_204",
    reduceMotion: false,
    serverSort: "default",
    collapsedGroups: [],
    githubMirror: "",
    subUserAgent: "v2rayN/7.13 Umbra/0.1.0",
    sendHwid: true,
    hwid: "a1b2c3d4e5f60718293a4b5c6d7e8f90",
  },
  logs: [],
};

type Handler = (payload: unknown) => void;
const listeners = new Map<string, Set<Handler>>();

function emit(event: string, payload: unknown) {
  listeners.get(event)?.forEach((h) => h(payload));
}

function pushLog(level: LogLine["level"], message: string) {
  const line: LogLine = { ts: now(), level, message };
  state.logs.push(line);
  if (state.logs.length > 2000) state.logs.shift();
  emit("core://log", [line]);
}

/** Browser preview starts unelevated, like a normal app launch. */
let mockElevated = false;

let trafficTimer: ReturnType<typeof setInterval> | null = null;
let totals = { up: 0, down: 0 };

function startTraffic() {
  stopTraffic();
  trafficTimer = setInterval(() => {
    const downBps = Math.max(0, 3_000_000 + Math.sin(now() / 2300) * 2_500_000 + Math.random() * 4_000_000);
    const upBps = Math.max(0, 400_000 + Math.random() * 700_000);
    totals.down += downBps;
    totals.up += upBps;
    emit("traffic://stats", {
      upBps,
      downBps,
      upTotal: totals.up,
      downTotal: totals.down,
    });
  }, 1000);
}

function stopTraffic() {
  if (trafficTimer) clearInterval(trafficTimer);
  trafficTimer = null;
}

const findServer = (id: string) =>
  [...mockServers, ...mockSubscriptions.flatMap((s) => s.servers)].find(
    (s) => s.id === id,
  );

const findSubscription = (id: string) =>
  mockSubscriptions.find((s) => s.id === id);

export async function mockInvoke(cmd: string, args?: unknown): Promise<unknown> {
  const a = (args ?? {}) as Record<string, unknown>;
  switch (cmd) {
    case "get_connection_state":
      return state.conn;
    case "connect": {
      const server = findServer(String(a.serverId));
      state.conn = {
        ...state.conn,
        status: "connecting",
        serverId: String(a.serverId),
        // the backend snapshots the name so it survives the server being deleted
        serverName: server?.name ?? null,
        error: null,
      };
      emit("conn://state", state.conn);
      pushLog("info", `starting sing-box with server ${server?.name ?? a.serverId}`);
      await new Promise((r) => setTimeout(r, 1400));
      state.conn = { ...state.conn, status: "connected", sinceMs: now() };
      emit("conn://state", state.conn);
      pushLog("info", "sing-box started, clash api ready on 127.0.0.1:9095");
      totals = { up: 0, down: 0 };
      startTraffic();
      return state.conn;
    }
    case "disconnect": {
      state.conn = { ...state.conn, status: "stopping" };
      emit("conn://state", state.conn);
      await new Promise((r) => setTimeout(r, 500));
      stopTraffic();
      state.conn = {
        ...state.conn,
        status: "disconnected",
        serverId: null,
        serverName: null,
        sinceMs: null,
      };
      emit("conn://state", state.conn);
      pushLog("info", "sing-box stopped, system proxy restored");
      return state.conn;
    }
    case "set_mode": {
      // Mirrors the backend guard so the elevation flow is reachable in preview.
      if (a.mode === "tun" && !mockElevated) {
        throw { code: "NEEDS_ELEVATION", message: "TUN mode requires administrator rights" };
      }
      state.conn = { ...state.conn, mode: a.mode as ConnectionState["mode"] };
      state.settings.mode = a.mode as Settings["mode"];
      emit("conn://state", state.conn);
      return state.conn;
    }
    case "select_server": {
      state.settings.selectedServerId = String(a.id);
      if (state.conn.status === "connected") {
        state.conn = {
          ...state.conn,
          serverId: String(a.id),
          serverName: findServer(String(a.id))?.name ?? null,
        };
      }
      emit("conn://state", state.conn);
      return state.conn;
    }
    case "import_share_links": {
      const text = String(a.text ?? "");
      const links = text.split(/\s+/).filter((l) => l.startsWith("vless://"));
      return { added: links.length, errors: links.length ? [] : ["no vless:// links found"] };
    }
    case "list_servers":
      return {
        manual: mockServers.slice(0, 2),
        subscriptions: mockSubscriptions,
      } satisfies ServersList;
    case "set_server_favorite": {
      // Mutating the fixture is what makes the star survive the list refresh
      // the Servers page does on entry, exactly like the real store does.
      const server = findServer(String(a.id));
      if (server) server.favorite = Boolean(a.favorite);
      return null;
    }
    case "reorder_subscriptions": {
      const ids = (a.ids as string[]) ?? [];
      const rank = new Map(ids.map((id, i) => [id, i]));
      mockSubscriptions.sort(
        (x, y) =>
          (rank.get(x.id) ?? Number.MAX_SAFE_INTEGER) -
          (rank.get(y.id) ?? Number.MAX_SAFE_INTEGER),
      );
      return null;
    }
    case "set_subscription_auto_update": {
      const sub = findSubscription(String(a.id));
      if (sub) sub.autoUpdateHours = Number(a.hours);
      return null;
    }
    case "rename_subscription": {
      const sub = findSubscription(String(a.id));
      if (!sub) throw { code: "NOT_FOUND", message: "no such subscription" };
      sub.name = String(a.name).trim();
      sub.panelTitle = null;
      return sub;
    }
    case "remove_server":
    case "remove_subscription":
    case "clear_logs":
      return null;
    case "add_subscription":
      return {
        ...mockSubscription,
        id: `sub-${now()}`,
        url: String(a.url),
        // no name given -> the panel's Profile-Title, not its filename
        name: a.name ? String(a.name) : "Example Network",
      };
    case "update_subscription":
      emit("sub://updated", { id: String(a.id), added: 1, removed: 0 });
      return {
        ...(findSubscription(String(a.id)) ?? mockSubscription),
        updatedAt: new Date().toISOString(),
      };
    case "ping_servers": {
      const ids = (a.ids as string[]) ?? [];
      ids.forEach((id, i) => {
        setTimeout(() => {
          const entry = findServer(id);
          const base = entry?.lastPingMs ?? 100;
          const jitter = Math.round(base + (Math.random() - 0.5) * 30);
          const latencyMs = Math.random() < 0.08 ? null : Math.max(8, jitter);
          // persisted like the backend does, so a list refresh keeps it
          if (entry) entry.lastPingMs = latencyMs;
          emit("ping://result", { serverId: id, latencyMs });
        }, 350 + i * 280 + Math.random() * 400);
      });
      return null;
    }
    case "url_test_active":
      return 47;
    case "get_core_status":
      return { installed: true, version: "1.13.14", path: "C:\\Users\\dev\\AppData\\Roaming\\com.umbra.proxy\\bin\\sing-box.exe" };
    case "check_core_update":
      return { current: "1.13.14", latest: "1.13.14", updateAvailable: false };
    case "download_core": {
      const total = 21_000_000;
      for (let d = 0; d <= total; d += total / 8) {
        emit("core://download-progress", { phase: "download", downloaded: d, total });
        await new Promise((r) => setTimeout(r, 180));
      }
      emit("core://download-progress", { phase: "extract", downloaded: total, total });
      await new Promise((r) => setTimeout(r, 350));
      emit("core://download-progress", { phase: "done", downloaded: total, total });
      return null;
    }
    case "get_settings":
      return state.settings;
    case "set_settings": {
      state.settings = { ...state.settings, ...(a.patch as Partial<Settings>) };
      return state.settings;
    }
    case "get_recent_logs":
      return state.logs.slice(-Number(a.limit ?? 500));
    case "is_elevated":
      return mockElevated;
    case "relaunch_elevated":
      // Stands in for the UAC restart: the real app exits and comes back elevated.
      mockElevated = true;
      return null;
    case "open_data_dir":
      return null;
    default:
      throw { code: "INTERNAL", message: `mock: unknown command ${cmd}` };
  }
}

export function mockListen(event: string, handler: Handler): () => void {
  if (!listeners.has(event)) listeners.set(event, new Set());
  listeners.get(event)!.add(handler);
  return () => {
    listeners.get(event)?.delete(handler);
  };
}

// Seed a few log lines so the Logs page has content in browser preview
if (state.logs.length === 0) {
  pushLog("info", "umbra mock backend initialized");
  pushLog("debug", "loaded profiles.json: 3 servers, 1 subscription");
  pushLog("warn", "this is the browser mock — run `npm run tauri dev` for the real backend");
}
