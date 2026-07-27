import type { ReactNode } from "react";
import { cn } from "../../lib/cn";

interface StatTileProps {
  label: string;
  icon?: ReactNode;
  /** big numeral — pass a node so callers can use AnimatedNumber */
  value: ReactNode;
  /** small line under the numeral (breakdowns, units, hints) */
  foot?: ReactNode;
  /** tint applied to the numeral only; labels stay neutral */
  tone?: "down" | "up" | "neutral";
  className?: string;
}

const TONE = {
  down: "text-accent-2",
  up: "text-accent",
  neutral: "text-text",
} as const;

/**
 * One reading in the dashboard's stat row. The numeral carries the weight
 * (Manrope 800 at text-stat, tabular so it can tick without reflowing); the
 * label is a small uppercase caption that never competes with it.
 */
export function StatTile({
  label,
  icon,
  value,
  foot,
  tone = "neutral",
  className,
}: StatTileProps) {
  return (
    <div className={cn("min-w-0", className)}>
      <div className="mb-1.5 flex items-center gap-1.5 text-label font-semibold text-text-faint uppercase">
        {icon}
        <span className="truncate">{label}</span>
      </div>
      <div
        className={cn(
          "font-display text-stat font-extrabold tabular-nums",
          TONE[tone],
        )}
      >
        {value}
      </div>
      {foot && (
        <div className="mt-1.5 flex items-center gap-2.5 text-xs text-text-dim tabular-nums">
          {foot}
        </div>
      )}
    </div>
  );
}
