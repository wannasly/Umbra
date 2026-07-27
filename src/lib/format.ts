import i18n from "../i18n";

const t = (key: string, opts?: Record<string, unknown>): string => i18n.t(key, opts);

const BYTE_UNITS = ["units.b", "units.kb", "units.mb", "units.gb", "units.tb"] as const;
const BPS_UNITS = ["units.bps", "units.kbps", "units.mbps", "units.gbps"] as const;

function scale(value: number, unitCount: number): { v: number; i: number } {
  let v = Math.max(0, value);
  let i = 0;
  while (v >= 1024 && i < unitCount - 1) {
    v /= 1024;
    i += 1;
  }
  return { v, i };
}

function trim(v: number, i: number): string {
  if (i === 0) return String(Math.round(v));
  if (v >= 100) return v.toFixed(0);
  return v.toFixed(1);
}

/** 123456789 -> "117.7 MB" (localized unit) */
export function formatBytes(bytes: number): string {
  const { v, i } = scale(bytes, BYTE_UNITS.length);
  return `${trim(v, i)} ${t(BYTE_UNITS[i])}`;
}

/** 1234567 -> "1.2 MB/s" (localized unit) */
export function formatBps(bps: number): string {
  const { v, i } = scale(bps, BPS_UNITS.length);
  return `${trim(v, i)} ${t(BPS_UNITS[i])}`;
}

/** milliseconds -> "HH:MM:SS" */
export function formatDuration(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${pad(h)}:${pad(m)}:${pad(s)}`;
}

/** ISO string or epoch ms -> "5 minutes ago" (localized, with RU plural forms) */
export function relativeTime(date: string | number): string {
  const then = typeof date === "number" ? date : Date.parse(date);
  const minutes = Math.floor((Date.now() - then) / 60_000);
  if (minutes < 1) return t("time.justNow");
  if (minutes < 60) return t("time.minutesAgo", { count: minutes });
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return t("time.hoursAgo", { count: hours });
  return t("time.daysAgo", { count: Math.floor(hours / 24) });
}

/** epoch ms -> "14:03:27" for log rows */
export function formatClock(ts: number): string {
  const d = new Date(ts);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
}

/** "https://very-long/path" -> "https://ver…path" keeping head + tail */
export function midEllipsis(s: string, max: number): string {
  if (s.length <= max) return s;
  const head = Math.ceil((max - 1) * 0.6);
  const tail = max - 1 - head;
  return `${s.slice(0, head)}…${s.slice(s.length - tail)}`;
}
