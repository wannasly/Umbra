import { motion } from "motion/react";
import { useTranslation } from "react-i18next";
import { cn } from "../../lib/cn";

interface PingBadgeProps {
  ms: number | null;
  testing?: boolean;
}

export function PingBadge({ ms, testing }: PingBadgeProps) {
  const { t } = useTranslation();
  const tone =
    ms == null
      ? "text-text-faint"
      : ms < 80
        ? "text-ok"
        : ms <= 200
          ? "text-warn"
          : "text-danger";
  const dot =
    ms == null
      ? "bg-text-faint"
      : ms < 80
        ? "bg-ok"
        : ms <= 200
          ? "bg-warn"
          : "bg-danger";

  return (
    <motion.span
      key={testing ? "testing" : `v-${ms ?? "none"}`}
      initial={testing ? false : { scale: 0.8 }}
      animate={{ scale: 1 }}
      transition={{ type: "spring", stiffness: 520, damping: 22 }}
      className={cn(
        "inline-flex shrink-0 items-center gap-1.5 rounded-(--radius-chip) border border-glass-border bg-glass px-2.5 py-1 font-mono text-[11px] tabular-nums",
        tone,
        testing && "animate-pulse",
      )}
    >
      <span className={cn("h-1.5 w-1.5 rounded-full", testing ? "bg-text-faint" : dot)} />
      {testing ? "···" : ms == null ? "—" : `${ms} ${t("units.ms")}`}
    </motion.span>
  );
}
