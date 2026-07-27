import { useTranslation } from "react-i18next";
import { motion } from "motion/react";
import { ArrowDown, ArrowUp, Copy, Star, Trash2, Zap } from "lucide-react";
import type { ServerEntry } from "../../lib/ipc";
import { readServerName } from "../../lib/flags";
import { formatBytes } from "../../lib/format";
import { cn } from "../../lib/cn";
import { GlassCard } from "../ui/GlassCard";
import { PingBadge } from "../ui/PingBadge";
import { KebabMenu } from "../ui/KebabMenu";
import { FlagChip } from "./FlagChip";

export interface ServerRowProps {
  server: ServerEntry;
  selected: boolean;
  testing: boolean;
  /** subscription entries are owned by the panel; only manual ones can be deleted */
  deletable: boolean;
  /** flashes after a scroll-to-active */
  highlighted?: boolean;
  onSelect: () => void;
  onPing: () => void;
  onCopy: () => void;
  onDelete: () => void;
  onToggleFavorite: () => void;
  innerRef?: (el: HTMLDivElement | null) => void;
}

const itemVariants = {
  hidden: { opacity: 0, y: 8 },
  show: { opacity: 1, y: 0 },
};

export function ServerRow({
  server,
  selected,
  testing,
  deletable,
  highlighted,
  onSelect,
  onPing,
  onCopy,
  onDelete,
  onToggleFavorite,
  innerRef,
}: ServerRowProps) {
  const { t } = useTranslation();
  const { flag, label } = readServerName(server.name);
  const chips = [
    server.protocol.toUpperCase(),
    server.security !== "none" ? server.security.toUpperCase() : null,
    server.transport.type !== "tcp" ? server.transport.type.toUpperCase() : null,
  ].filter((c): c is string => c !== null);

  const used = (server.totalUp ?? 0) + (server.totalDown ?? 0);

  return (
    <motion.div variants={itemVariants}>
      <GlassCard
        ref={innerRef}
        interactive
        onClick={onSelect}
        className={cn(
          "relative flex items-center gap-3 px-4 py-3",
          selected && "shadow-[0_0_20px_rgb(47_224_127/0.12)]",
          // The flash after "scroll to the active server": a ring rather than a
          // background change, so it reads as "here it is" and not as a state.
          highlighted && "ring-2 ring-accent/70",
        )}
      >
        {selected && (
          <span className="absolute top-3 bottom-3 left-0 w-[3px] rounded-full bg-ok" />
        )}
        {/* radio */}
        <span
          className={cn(
            "flex h-[18px] w-[18px] shrink-0 items-center justify-center rounded-full border-2 transition-colors duration-150",
            selected ? "border-accent" : "border-glass-edge",
          )}
        >
          {selected && <span className="h-2 w-2 rounded-full bg-accent" />}
        </span>
        <FlagChip flag={flag} />
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <span className="truncate text-[15px] font-semibold text-text">
              {label}
            </span>
            <span className="flex shrink-0 items-center gap-1">
              {chips.map((c) => (
                <span
                  key={c}
                  className="rounded-(--radius-chip) border border-glass-border bg-glass px-2 py-0.5 text-xs font-medium text-text-dim"
                >
                  {c}
                </span>
              ))}
            </span>
          </div>
          <div className="mt-0.5 flex items-center gap-2.5 truncate font-mono text-xs text-text-dim">
            <span className="truncate">
              {server.server}:{server.port}
            </span>
            {used > 0 && (
              <span
                title={t("servers.totalTraffic")}
                className="flex shrink-0 items-center gap-1 text-text-faint tabular-nums"
              >
                <ArrowDown size={10} className="text-accent-2" />
                {formatBytes(server.totalDown ?? 0)}
                <ArrowUp size={10} className="ml-1 text-accent" />
                {formatBytes(server.totalUp ?? 0)}
              </span>
            )}
          </div>
        </div>
        <button
          type="button"
          title={
            server.favorite
              ? t("servers.unfavorite")
              : t("servers.favorite")
          }
          aria-pressed={server.favorite}
          onClick={(e) => {
            e.stopPropagation();
            onToggleFavorite();
          }}
          className={cn(
            "inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-(--radius-ctl)",
            "transition-[background-color,color] duration-150 hover:bg-glass-strong",
            server.favorite ? "text-warn" : "text-text-faint hover:text-text",
          )}
        >
          <Star size={15} fill={server.favorite ? "currentColor" : "none"} />
        </button>
        <PingBadge ms={server.lastPingMs} testing={testing} />
        <KebabMenu
          items={[
            {
              label: t("servers.menu.ping"),
              icon: <Zap size={14} />,
              onClick: onPing,
            },
            {
              label: server.favorite
                ? t("servers.unfavorite")
                : t("servers.favorite"),
              icon: <Star size={14} />,
              onClick: onToggleFavorite,
            },
            {
              label: t("servers.menu.copyLink"),
              icon: <Copy size={14} />,
              onClick: onCopy,
            },
            ...(deletable
              ? [
                  {
                    label: t("servers.menu.delete"),
                    icon: <Trash2 size={14} />,
                    danger: true,
                    onClick: onDelete,
                  },
                ]
              : []),
          ]}
        />
      </GlassCard>
    </motion.div>
  );
}
