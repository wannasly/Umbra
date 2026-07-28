import { useTranslation } from "react-i18next";
import { ArrowDown, ArrowUp, RefreshCw, Repeat, ServerOff } from "lucide-react";
import type { ConnStatus, ServerEntry } from "../../lib/ipc";
import { useServers } from "../../stores/servers";
import { formatBytes } from "../../lib/format";
import { getProxyChips } from "../../lib/serverMeta";
import { cn } from "../../lib/cn";
import { Button } from "../ui/Button";
import { GlassCard } from "../ui/GlassCard";

const pingTone = (ms: number | null): string =>
  ms == null
    ? "text-text-faint"
    : ms < 80
      ? "text-ok"
      : ms <= 200
        ? "text-warn"
        : "text-danger";

/**
 * Latency, sized to sit beside the server name as the second-largest thing on
 * the screen. Refresh is manual and only manual: the user was explicit that
 * ping must never poll on its own, so there is no timer here and nothing
 * re-tests on mount, on reconnect, or on focus.
 */
function PingReadout({ server }: { server: ServerEntry }) {
  const { t } = useTranslation();
  const ping = useServers((s) => s.ping);
  const testing = useServers((s) => s.pendingPings.has(server.id));
  const ms = server.lastPingMs;

  return (
    <div className="flex shrink-0 items-center gap-2">
      <div className="flex items-baseline gap-1.5">
        <span
          className={cn(
            "font-display text-stat font-extrabold tabular-nums",
            testing ? "text-text-faint" : pingTone(ms),
          )}
        >
          {testing ? "···" : (ms ?? "—")}
        </span>
        {!testing && ms != null && (
          <span className="text-sm font-semibold text-text-faint">
            {t("units.ms")}
          </span>
        )}
      </div>
      <button
        type="button"
        onClick={() => void ping([server.id])}
        disabled={testing}
        title={t("dashboard.ping.refresh")}
        aria-label={t("dashboard.ping.refresh")}
        className={cn(
          "inline-flex h-9 w-9 shrink-0 items-center justify-center rounded-(--radius-ctl)",
          "border border-glass-border bg-glass text-text-dim",
          "transition-[background-color,color] duration-150",
          "hover:bg-glass-strong hover:text-text disabled:pointer-events-none disabled:opacity-45",
        )}
      >
        <RefreshCw size={16} className={cn(testing && "animate-spin")} />
      </button>
    </div>
  );
}

/**
 * Cumulative traffic for this one server, across every session — deliberately
 * not the session counters, which live in the stat row below. `undefined` means
 * the backend does not track it yet, which must not render as "0 B": that would
 * claim the server has never carried a byte.
 */
function ServerTotal({ server }: { server: ServerEntry }) {
  const { t } = useTranslation();
  const { totalUp, totalDown } = server;
  const known = totalUp != null || totalDown != null;
  const sum = (totalUp ?? 0) + (totalDown ?? 0);

  return (
    <div
      className={cn(
        "flex flex-wrap items-baseline gap-x-4 gap-y-1 rounded-(--radius-ctl)",
        "border border-glass-border bg-glass px-3.5 py-2.5",
      )}
    >
      <span className="text-label font-semibold text-text-faint uppercase">
        {t("dashboard.server.totalLabel")}
      </span>
      <span className="font-display text-figure font-extrabold tabular-nums text-text">
        {known ? formatBytes(sum) : "—"}
      </span>
      {known && (
        <span className="flex items-center gap-3 text-xs text-text-dim tabular-nums">
          <span className="flex items-center gap-1">
            <ArrowDown size={13} className="text-accent-2" />
            {formatBytes(totalDown ?? 0)}
          </span>
          <span className="flex items-center gap-1">
            <ArrowUp size={13} className="text-accent" />
            {formatBytes(totalUp ?? 0)}
          </span>
        </span>
      )}
      {!known && (
        <span className="text-xs text-text-faint">
          {t("dashboard.server.totalUnavailable")}
        </span>
      )}
    </div>
  );
}

interface ServerHeroProps {
  server: ServerEntry | undefined;
  /** snapshot from the backend, used when the entry itself is gone */
  fallbackName: string | null;
  status: ConnStatus;
  onChoose: () => void;
}

export function ServerHero({
  server,
  fallbackName,
  status,
  onChoose,
}: ServerHeroProps) {
  const { t } = useTranslation();

  if (!server) {
    // The connection can outlive the list entry (subscription deleted mid-
    // session), so a snapshot name still beats "no server selected".
    if (fallbackName) {
      return (
        <div className="min-w-0 flex-1">
          <h1 className="truncate font-display text-hero font-extrabold text-text">
            {fallbackName}
          </h1>
          <p className="mt-2 text-sm text-text-dim">
            {t("dashboard.server.gone")}
          </p>
        </div>
      );
    }
    return (
      <GlassCard className="flex min-w-0 flex-1 flex-wrap items-center justify-between gap-4 px-5 py-4">
        <span className="flex items-center gap-3 text-lead text-text-dim">
          <ServerOff size={20} className="shrink-0 text-text-faint" />
          {t("dashboard.noServer")}
        </span>
        <Button onClick={onChoose}>{t("dashboard.chooseServer")}</Button>
      </GlassCard>
    );
  }

  const chips = getProxyChips(server);

  return (
    <div className="flex min-w-0 flex-1 flex-col gap-3">
      {/* 1 — name and latency, the largest pair after the power button */}
      <div className="flex min-w-0 flex-wrap items-center gap-x-5 gap-y-2">
        <h1
          className="min-w-0 flex-1 truncate font-display text-hero font-extrabold text-text"
          title={server.name}
        >
          {server.name}
        </h1>
        <PingReadout server={server} />
      </div>

      <div className="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-2">
        <span
          className="min-w-0 truncate font-mono text-xs text-text-dim"
          title={`${server.server}:${server.port}`}
        >
          {server.server}:{server.port}
        </span>
        <span className="flex flex-wrap items-center gap-1.5">
          {chips.map((c) => (
            <span
              key={c.label}
              className="rounded-(--radius-chip) border border-glass-border bg-glass px-2.5 py-0.5 text-[11px] font-semibold tracking-wide text-text-dim"
            >
              {c.label}
            </span>
          ))}
        </span>
        <button
          type="button"
          onClick={onChoose}
          className={cn(
            "ml-auto inline-flex shrink-0 items-center gap-1.5 rounded-(--radius-chip) px-2.5 py-1",
            "text-xs font-semibold text-text-dim transition-colors duration-150 hover:bg-glass hover:text-text",
          )}
        >
          <Repeat size={13} />
          {t("dashboard.server.change")}
        </button>
      </div>

      {/* 2 — cumulative traffic for this server, right beside name and ping */}
      <ServerTotal server={server} />

      {status === "connected" && server.lastPingMs == null && (
        <p className="text-xs text-text-faint">{t("dashboard.ping.hint")}</p>
      )}
    </div>
  );
}
