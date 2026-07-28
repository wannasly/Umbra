// IPC contract between the React frontend and the Rust backend.
// This file is the single source of truth for command names, event names and payload shapes.
// The Rust side (src-tauri) must match these exactly.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { mockInvoke, mockListen } from "./mock";

// ---------------------------------------------------------------------------
// Shared types
// ---------------------------------------------------------------------------

export type Mode = "system_proxy" | "tun";
export type ConnStatus = "disconnected" | "connecting" | "connected" | "stopping";

export interface ConnectionState {
  status: ConnStatus;
  serverId: string | null;
  /**
   * Name of the server the tunnel is actually running against, snapshotted by
   * the backend when the connection was established. Outlives the entry in the
   * server list, so deleting a subscription mid-session can never leave the UI
   * showing "connected" and "no server selected" at the same time — fall back
   * to this whenever `serverId` no longer resolves.
   */
  serverName: string | null;
  mode: Mode;
  /** epoch ms of when the connection was established (for uptime) */
  sinceMs: number | null;
  error: string | null;
}

export type Transport =
  | { type: "tcp" }
  | { type: "ws"; path: string; host: string }
  | { type: "grpc"; serviceName: string }
  | { type: "httpupgrade"; path: string; host: string };

export type Security = "reality" | "tls" | "none";

export type Hysteria2Obfs = {
  type: "salamander";
  password: string;
};

export interface BaseProxyNode {
  id: string;
  name: string;
  server: string;
  port: number;
  lastPingMs: number | null;
  favorite: boolean;
  totalUp?: number;
  totalDown?: number;
  raw: string;
}

export interface ProxyNodeVless extends BaseProxyNode {
  protocol: "vless";
  uuid: string;
  flow: string;
  security: Security;
  sni: string;
  fingerprint: string;
  publicKey: string;
  shortId: string;
  insecure: boolean;
  alpn: string[];
  transport: Transport;
}

export interface ProxyNodeHysteria2 extends BaseProxyNode {
  protocol: "hysteria2";
  password: string;
  obfs?: Hysteria2Obfs;
  insecure: boolean;
  sni: string;
  alpn: string[];
}

export type ProxyNode = ProxyNodeVless | ProxyNodeHysteria2;
export type ServerEntry = ProxyNode;

export interface SubscriptionQuota {
  upload: number;
  download: number;
  total: number;
  /** unix seconds, 0 = never */
  expire: number;
}

export interface Subscription {
  id: string;
  name: string;
  url: string;
  /** ISO datetime of last successful refresh */
  updatedAt: string | null;
  quota: SubscriptionQuota | null;
  /** 0 = off; seeded from the panel's `Profile-Update-Interval` */
  autoUpdateHours: number;
  /** `Support-Url` header, when the panel sends one */
  supportUrl: string | null;
  /** `Profile-Web-Page-Url` header — the provider's account page */
  webPageUrl: string | null;
  /**
   * The panel's own `Profile-Title`. Kept so a refresh can tell a name we
   * derived from one the user typed; not meant for display (`name` is).
   */
  panelTitle: string | null;
  servers: ServerEntry[];
}

export interface ServersList {
  manual: ServerEntry[];
  subscriptions: Subscription[];
}

export type Accent = "violet" | "cyan" | "emerald" | "amber";

/** "default" = the order the panel delivered; the other two are user sorts. */
export type ServerSort = "default" | "ping" | "name";

export type RouteTarget = "proxy" | "direct";
export type AppRouteAction = RouteTarget | "block";

export interface AppRouteRule {
  id: string;
  processName: string;
  action: AppRouteAction;
}

export type IpStrategy = "ipv4_only" | "prefer_ipv4" | "prefer_ipv6" | "ipv6_only";

/** Synthetic group keys for `Settings.collapsedGroups` (never subscription ids). */
export const GROUP_FAVORITES = "favorites";
export const GROUP_MANUAL = "manual";

export interface Settings {
  version: number;
  language: "ru" | "en";
  accent: Accent;
  mode: Mode;
  mixedPort: number;
  selectedServerId: string | null;
  autostart: boolean;
  startMinimized: boolean;
  minimizeToTray: boolean;
  connectOnStartup: boolean;
  logLevel: "trace" | "debug" | "info" | "warn" | "error";
  /** route RU sites/IPs direct via remote rule-sets */
  bypassRu: boolean;
  /** fallback for traffic that did not match an application or geo rule */
  routeDefault: RouteTarget;
  /** per-process split-tunnelling rules, evaluated before generic routes */
  appRoutes: AppRouteRule[];
  tunStack: "mixed" | "system" | "gvisor";
  tunStrictRoute: boolean;
  tunMtu: number;
  ipStrategy: IpStrategy;
  pingUrl: string;
  reduceMotion: boolean;
  /** server-list ordering on the Servers page */
  serverSort: ServerSort;
  /**
   * Group keys the user collapsed on the Servers page: subscription ids plus
   * the two synthetic groups, `GROUP_FAVORITES` and `GROUP_MANUAL`.
   */
  collapsedGroups: string[];
  /** optional GitHub mirror prefix for core downloads, "" = direct */
  githubMirror: string;
  /** User-Agent sent when fetching subscriptions; panels key their format off it */
  subUserAgent: string;
  /** send x-hwid / x-device-* headers for panels enforcing a device limit */
  sendHwid: boolean;
  /** stable machine-derived id, generated by the backend */
  hwid: string;
}

export interface CoreStatus {
  installed: boolean;
  version: string | null;
  path: string;
}

export interface UpdateCheck {
  current: string | null;
  latest: string;
  updateAvailable: boolean;
}

export interface LogLine {
  /** epoch ms */
  ts: number;
  level: "trace" | "debug" | "info" | "warn" | "error";
  message: string;
}

export interface ImportResult {
  added: number;
  errors: string[];
}

export interface TrafficStats {
  upBps: number;
  downBps: number;
  upTotal: number;
  downTotal: number;
}

export interface PingResult {
  serverId: string;
  latencyMs: number | null;
}

export interface DownloadProgress {
  phase: "download" | "extract" | "done";
  downloaded: number;
  total: number;
}

export interface CrashInfo {
  code: number | null;
  willRestart: boolean;
  attempt: number;
}

export interface SubUpdated {
  id: string;
  added: number;
  removed: number;
}

/** Error shape every command rejects with (serialized AppError from Rust). */
export interface IpcError {
  code:
    | "NEEDS_ELEVATION"
    | "CORE_NOT_INSTALLED"
    | "CORE_START_FAILED"
    | "PARSE_ERROR"
    | "NETWORK_ERROR"
    | "UNSUPPORTED_FORMAT"
    /** panel gates the server list behind a device id we did not send */
    | "HWID_REQUIRED"
    /** panel accepted the device id but the account has no free device slot */
    | "DEVICE_LIMIT"
    | "NOT_FOUND"
    | "IO_ERROR"
    | "INTERNAL";
  message: string;
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

export interface Commands {
  get_connection_state: { args: Record<string, never>; result: ConnectionState };
  connect: { args: { serverId: string }; result: ConnectionState };
  disconnect: { args: Record<string, never>; result: ConnectionState };
  set_mode: { args: { mode: Mode }; result: ConnectionState };

  import_share_links: { args: { text: string }; result: ImportResult };
  list_servers: { args: Record<string, never>; result: ServersList };
  remove_server: { args: { id: string }; result: null };
  select_server: { args: { id: string }; result: ConnectionState };
  set_server_favorite: { args: { id: string; favorite: boolean }; result: null };

  add_subscription: { args: { url: string; name: string | null }; result: Subscription };
  update_subscription: { args: { id: string }; result: Subscription };
  remove_subscription: { args: { id: string }; result: null };
  rename_subscription: { args: { id: string; name: string }; result: Subscription };
  /** persists the group order shown on the Servers page */
  reorder_subscriptions: { args: { ids: string[] }; result: null };
  set_subscription_auto_update: { args: { id: string; hours: number }; result: null };

  ping_servers: { args: { ids: string[] }; result: null };
  url_test_active: { args: Record<string, never>; result: number };

  get_core_status: { args: Record<string, never>; result: CoreStatus };
  check_core_update: { args: Record<string, never>; result: UpdateCheck };
  download_core: { args: { version: string | null }; result: null };

  get_settings: { args: Record<string, never>; result: Settings };
  set_settings: { args: { patch: Partial<Settings> }; result: Settings };

  get_recent_logs: { args: { limit: number }; result: LogLine[] };
  clear_logs: { args: Record<string, never>; result: null };

  is_elevated: { args: Record<string, never>; result: boolean };
  relaunch_elevated: { args: Record<string, never>; result: null };
  open_data_dir: { args: Record<string, never>; result: null };
}

// ---------------------------------------------------------------------------
// Events (backend -> frontend)
// ---------------------------------------------------------------------------

export interface Events {
  "conn://state": ConnectionState;
  /** batched every 250ms */
  "core://log": LogLine[];
  /** 1/s while connected */
  "traffic://stats": TrafficStats;
  "ping://result": PingResult;
  "core://download-progress": DownloadProgress;
  "core://crashed": CrashInfo;
  "sub://updated": SubUpdated;
  /** tray- or startup-initiated TUN attempt that needs admin rights */
  "ui://needs-elevation": null;
}

// ---------------------------------------------------------------------------
// Typed wrappers (with browser-mock fallback for pure-web development)
// ---------------------------------------------------------------------------

export const inTauri: boolean =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export async function ipc<K extends keyof Commands>(
  cmd: K,
  ...args: Commands[K]["args"] extends Record<string, never>
    ? []
    : [Commands[K]["args"]]
): Promise<Commands[K]["result"]> {
  if (!inTauri) {
    return mockInvoke(cmd, args[0]) as Promise<Commands[K]["result"]>;
  }
  return invoke<Commands[K]["result"]>(cmd, args[0] ?? {});
}

export async function onEvent<K extends keyof Events>(
  event: K,
  handler: (payload: Events[K]) => void,
): Promise<UnlistenFn> {
  if (!inTauri) {
    return mockListen(event, handler as (payload: unknown) => void);
  }
  return listen<Events[K]>(event, (e) => handler(e.payload));
}

export function isIpcError(e: unknown): e is IpcError {
  return (
    typeof e === "object" && e !== null && "code" in e && "message" in e
  );
}
