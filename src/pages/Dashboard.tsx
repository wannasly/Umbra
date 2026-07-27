import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { ArrowDown, ArrowUp, Timer } from "lucide-react";
import { ipc, type Mode } from "../lib/ipc";
import { useConnection } from "../stores/connection";
import { useServers, allServers } from "../stores/servers";
import { useSettings } from "../stores/settings";
import { useUi, handleIpcError, isElevationError } from "../stores/ui";
import { formatBps, formatBytes, formatDuration } from "../lib/format";
import { cn } from "../lib/cn";
import { GlassCard } from "../components/ui/GlassCard";
import { PageShell } from "../components/ui/PageShell";
import { AnimatedNumber } from "../components/ui/AnimatedNumber";
import { SegmentedControl } from "../components/ui/SegmentedControl";
import { ConnectButton } from "../components/dashboard/ConnectButton";
import { ServerHero } from "../components/dashboard/ServerHero";
import { StatTile } from "../components/dashboard/StatTile";
import { TrafficChart } from "../components/dashboard/TrafficChart";

function Uptime({ sinceMs }: { sinceMs: number | null }) {
  const [, setTick] = useState(0);
  useEffect(() => {
    if (sinceMs == null) return;
    const timer = setInterval(() => setTick((n) => n + 1), 1000);
    return () => clearInterval(timer);
  }, [sinceMs]);
  return <>{sinceMs == null ? "—" : formatDuration(Date.now() - sinceMs)}</>;
}

const STATUS_COLOR: Record<string, string> = {
  disconnected: "text-text-faint",
  connecting: "text-warn",
  connected: "text-ok",
  stopping: "text-text-dim",
};

export function Dashboard() {
  const { t } = useTranslation();
  const conn = useConnection((s) => s.conn);
  const upBps = useConnection((s) => s.upBps);
  const downBps = useConnection((s) => s.downBps);
  const upTotal = useConnection((s) => s.upTotal);
  const downTotal = useConnection((s) => s.downTotal);
  const list = useServers((s) => s.list);
  const settings = useSettings((s) => s.settings);
  const setPage = useUi((s) => s.setPage);
  const pushToast = useUi((s) => s.toast);
  const elevationOpen = useUi((s) => s.elevationOpen);

  // The mode the user asked for while the elevation prompt decides its fate:
  // shown as selected so the click registers, dropped when the prompt closes
  // (cancel or a failed restart), which reverts the control to the real mode.
  const [pendingMode, setPendingMode] = useState<Mode | null>(null);
  useEffect(() => {
    if (!elevationOpen) setPendingMode(null);
  }, [elevationOpen]);

  const activeId = conn.serverId ?? settings?.selectedServerId ?? null;
  const server = allServers(list).find((s) => s.id === activeId);

  const onHeroClick = () => {
    if (conn.status === "connected" || conn.status === "connecting") {
      void ipc("disconnect").catch(handleIpcError);
      return;
    }
    if (conn.status === "stopping") return;
    if (!activeId) {
      pushToast({
        kind: "info",
        title: t("dashboard.noServer"),
        action: {
          label: t("dashboard.chooseServer"),
          onClick: () => setPage("servers"),
        },
      });
      return;
    }
    void ipc("connect", { serverId: activeId }).catch(handleIpcError);
  };

  const setMode = (mode: Mode) => {
    void ipc("set_mode", { mode }).catch((e) => {
      if (isElevationError(e)) setPendingMode(mode);
      handleIpcError(e);
    });
  };

  const shownMode = pendingMode ?? conn.mode;
  const modeLocked = conn.status !== "disconnected";

  return (
    <PageShell width="list" className="gap-4">
      {/*
        Hero. Reading order matches the weight the user asked for: the power
        button, then the server name with its latency beside it, then how much
        this particular server has ever carried.
        The button is shrink-0 and the identity column has a 22rem basis, so the
        two sit side by side down to ~560px of card width and stack below it
        instead of crushing the name into an ellipsis.
      */}
      <GlassCard className="px-[clamp(1rem,1.8vw,1.5rem)] py-[clamp(0.875rem,1.45vw,1.25rem)]">
        <div className="flex flex-wrap items-center gap-x-6 gap-y-5">
          <div className="mx-auto flex shrink-0 flex-col items-center gap-2">
            <ConnectButton status={conn.status} onClick={onHeroClick} />
            <div
              className={cn(
                "font-display text-status font-extrabold tracking-[0.16em] uppercase",
                STATUS_COLOR[conn.status],
              )}
            >
              {t(`dashboard.status.${conn.status}`)}
            </div>
          </div>
          <div className="flex min-w-0 flex-1 basis-[22rem] items-center">
            <ServerHero
              server={server}
              fallbackName={conn.serverName}
              status={conn.status}
              onChoose={() => setPage("servers")}
            />
          </div>
        </div>
      </GlassCard>

      {/*
        Mode. The label and the control sit together on the left and the row's
        remaining width carries the hint for the selected mode — the previous
        layout pushed the control to the far right and left a dead ~260px gap
        between the two.
      */}
      <div className="glass flex flex-wrap items-center gap-x-4 gap-y-2 rounded-(--radius-card) px-4 py-3">
        <span className="text-label font-semibold text-text-faint uppercase">
          {t("dashboard.mode.label")}
        </span>
        <SegmentedControl<Mode>
          id="mode"
          value={shownMode}
          onChange={setMode}
          disabled={modeLocked}
          title={modeLocked ? t("dashboard.mode.lockedHint") : undefined}
          options={[
            { value: "system_proxy", label: t("dashboard.mode.system") },
            { value: "tun", label: t("dashboard.mode.tun") },
          ]}
        />
        <span className="min-w-0 flex-1 text-xs text-text-dim">
          {modeLocked
            ? t("dashboard.mode.lockedHint")
            : shownMode === "tun"
              ? t("dashboard.mode.tunHint")
              : t("dashboard.mode.systemHint")}
        </span>
      </div>

      {/*
        Speeds, session total and uptime — one card so they read as a single
        instrument panel, with the chart as its baseline. Two columns until the
        window is wide enough for four to keep their numerals un-truncated.
      */}
      <GlassCard className="px-[clamp(1rem,1.8vw,1.5rem)] py-[clamp(0.875rem,1.45vw,1.25rem)]">
        <div className="mb-4 grid grid-cols-2 gap-x-6 gap-y-5 min-[840px]:grid-cols-4">
          <StatTile
            tone="down"
            label={t("dashboard.traffic.download")}
            icon={<ArrowDown size={13} className="shrink-0 text-accent-2" />}
            value={<AnimatedNumber value={downBps} format={formatBps} />}
          />
          <StatTile
            tone="up"
            label={t("dashboard.traffic.upload")}
            icon={<ArrowUp size={13} className="shrink-0 text-accent" />}
            value={<AnimatedNumber value={upBps} format={formatBps} />}
          />
          <StatTile
            label={t("dashboard.traffic.session")}
            value={formatBytes(downTotal + upTotal)}
            foot={
              <>
                <span className="flex items-center gap-1">
                  <ArrowDown size={12} className="text-accent-2" />
                  {formatBytes(downTotal)}
                </span>
                <span className="flex items-center gap-1">
                  <ArrowUp size={12} className="text-accent" />
                  {formatBytes(upTotal)}
                </span>
              </>
            }
          />
          <StatTile
            label={t("dashboard.traffic.uptime")}
            icon={<Timer size={13} className="shrink-0 text-text-faint" />}
            value={<Uptime sinceMs={conn.sinceMs} />}
          />
        </div>
        <TrafficChart />
      </GlassCard>
    </PageShell>
  );
}
