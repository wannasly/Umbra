import { useTranslation } from "react-i18next";
import { ChevronDown, ChevronRight, ChevronUp } from "lucide-react";
import { cn } from "../../lib/cn";
import { IconButton } from "../ui/IconButton";

export interface GroupHeaderProps {
  title: string;
  count: number;
  collapsed: boolean;
  onToggle: () => void;
  /** decoration for the favourites group */
  icon?: React.ReactNode;
  /** compact account quota shown for subscription groups */
  meta?: React.ReactNode;
  /** present only on reorderable (subscription) groups */
  onMove?: (delta: number) => void;
  canMoveUp?: boolean;
  canMoveDown?: boolean;
}

export function GroupHeader({
  title,
  count,
  collapsed,
  onToggle,
  icon,
  meta,
  onMove,
  canMoveUp,
  canMoveDown,
}: GroupHeaderProps) {
  const { t } = useTranslation();
  return (
    <div
      className={cn(
        "sticky top-1 z-20 mb-2 flex min-h-10 w-full flex-wrap items-center gap-x-2 gap-y-1 rounded-(--radius-ctl) px-3 py-2",
        "border border-interactive-border bg-surface-2/95 shadow-[0_8px_24px_rgb(0_8_11/0.24)] backdrop-blur-md",
      )}
    >
      <button
        type="button"
        onClick={onToggle}
        className="flex min-h-9 min-w-40 flex-1 items-center gap-1.5 rounded-lg text-left outline-none focus-visible:ring-2 focus-visible:ring-focus-ring"
      >
        <ChevronRight
          size={14}
          className={cn(
            "shrink-0 text-text-faint transition-transform duration-150",
            !collapsed && "rotate-90",
          )}
        />
        {icon}
        <span className="truncate text-[13px] font-semibold tracking-[0.055em] text-accent">
          {title}
        </span>
        <span className="shrink-0 text-[11px] text-text-faint">
          {t("servers.count", { count })}
        </span>
      </button>
      {meta && <div className="min-w-0 shrink text-right">{meta}</div>}
      {onMove && (
        // Buttons, not drag-and-drop: two clicks that always land beat a
        // pointer gesture that has to fight a scrolling list.
        <span className="flex shrink-0 items-center">
          <IconButton
            title={t("servers.moveUp")}
            disabled={!canMoveUp}
            onClick={() => onMove(-1)}
          >
            <ChevronUp size={14} />
          </IconButton>
          <IconButton
            title={t("servers.moveDown")}
            disabled={!canMoveDown}
            onClick={() => onMove(1)}
          >
            <ChevronDown size={14} />
          </IconButton>
        </span>
      )}
    </div>
  );
}
