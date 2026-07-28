import { useTranslation } from "react-i18next";
import { motion } from "motion/react";
import {
  House,
  List,
  Route as RouteIcon,
  Rss,
  ScrollText,
  Settings as SettingsIcon,
  type LucideIcon,
} from "lucide-react";
import { useUi, type Page } from "../stores/ui";
import { useConnection } from "../stores/connection";
import { useSettings } from "../stores/settings";
import { useServers, allServers } from "../stores/servers";
import { GlassCard } from "./ui/GlassCard";
import { cn } from "../lib/cn";
import { APP_VERSION, APP_VERSION_SHORT } from "../lib/version";

const NAV: { page: Page; icon: LucideIcon }[] = [
  { page: "dashboard", icon: House },
  { page: "servers", icon: List },
  { page: "subscriptions", icon: Rss },
  { page: "routing", icon: RouteIcon },
  { page: "logs", icon: ScrollText },
  { page: "settings", icon: SettingsIcon },
];

export function Sidebar() {
  const { t } = useTranslation();
  const page = useUi((s) => s.page);
  const setPage = useUi((s) => s.setPage);
  const conn = useConnection((s) => s.conn);
  const selectedServerId = useSettings((s) => s.settings?.selectedServerId ?? null);
  const list = useServers((s) => s.list);

  const activeServer = allServers(list).find(
    (s) => s.id === (conn.serverId ?? selectedServerId),
  );
  // Deleting the connected subscription removes the entry while the tunnel keeps
  // running, so the id stops resolving. `conn.serverName` is snapshotted at
  // connect time and outlives the profile entry — without this fallback the rail
  // shows a green "connected" dot next to "no server selected".
  const activeName = activeServer?.name ?? conn.serverName ?? t("dashboard.noServer");

  // Fixed width by design, but two of them: below 1000px the labels are worth
  // less than the ~150px they cost the content column, so the rail collapses to
  // icons (titles keep it navigable). shrink-0 stops it being squeezed; min-h-0
  // + overflow-hidden stop the status block spilling out of a short window
  // instead of colliding with the nav.
  return (
    <motion.aside
      layout="position"
      transition={{ type: "spring", stiffness: 360, damping: 34 }}
      className={cn(
        "app-nav relative z-10 flex min-h-0 shrink-0 flex-col overflow-hidden pb-4",
        "w-[220px] px-3 transition-[width] duration-200",
        "max-[1000px]:w-[68px] max-[1000px]:px-2.5",
      )}
    >
      <nav className="app-nav-list flex shrink-0 flex-col gap-1">
        {NAV.map(({ page: p, icon: Icon }) => {
          const active = page === p;
          const label = t(`nav.${p}`);
          return (
            <button
              key={p}
              type="button"
              onClick={() => setPage(p)}
              title={label}
              aria-label={label}
              className={cn(
                "app-nav-button relative flex h-10 items-center gap-3 rounded-(--radius-ctl) px-3.5 text-[13px] font-medium",
                "outline-none transition-[background-color,color,box-shadow] duration-150",
                "focus-visible:ring-2 focus-visible:ring-focus-ring focus-visible:ring-offset-2 focus-visible:ring-offset-surface-0",
                "max-[1000px]:justify-center max-[1000px]:gap-0 max-[1000px]:px-0",
                active
                  ? "text-text"
                  : "text-text-dim hover:bg-hover-surface hover:text-text",
              )}
            >
              {active && (
                <motion.span
                  layoutId="nav-pill"
                  transition={{ type: "spring", stiffness: 450, damping: 35 }}
                  className="absolute inset-0 overflow-hidden rounded-(--radius-ctl) border border-selected-border bg-selected-surface"
                >
                  <span className="app-nav-indicator-edge absolute top-2 bottom-2 left-0 w-[3px] rounded-full bg-linear-180 from-accent to-accent-2" />
                </motion.span>
              )}
              <Icon size={16} className="relative z-10 shrink-0" />
              <span className="app-nav-label relative z-10 truncate max-[1000px]:hidden">
                {label}
              </span>
            </button>
          );
        })}
      </nav>

      <div className="app-nav-status mt-auto min-h-0 shrink-0 pt-4">
        <GlassCard
          interactive
          variant="glass"
          onClick={() => setPage("dashboard")}
          title={`${activeName} — ${t(`dashboard.status.${conn.status}`)}`}
          className="p-3 max-[1000px]:flex max-[1000px]:justify-center max-[1000px]:p-2.5"
        >
          <div className="flex items-center gap-2.5 max-[1000px]:gap-0">
            <span
              className={cn(
                "h-2 w-2 shrink-0 rounded-full",
                conn.status === "connected"
                  ? "bg-ok shadow-[0_0_10px_var(--color-ok)]"
                  : conn.status === "connecting" || conn.status === "stopping"
                    ? "animate-pulse bg-warn"
                    : "bg-text-faint",
              )}
            />
            <div className="min-w-0 max-[1000px]:hidden">
              <div className="truncate text-xs font-medium text-text">
                {activeName}
              </div>
              <div className="truncate text-[11px] text-text-faint">
                {t(`dashboard.status.${conn.status}`)}
              </div>
            </div>
          </div>
        </GlassCard>
        <div className="app-nav-version mt-2 px-1 text-[10px] text-text-faint max-[1000px]:text-center">
          <span className="max-[1000px]:hidden">v{APP_VERSION}</span>
          <span className="hidden max-[1000px]:inline">{APP_VERSION_SHORT}</span>
        </div>
      </div>
    </motion.aside>
  );
}
