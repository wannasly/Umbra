import { motion } from "motion/react";
import { cn } from "../../lib/cn";

interface SegmentedControlProps<T extends string> {
  /** unique per instance — namespaces the motion layoutId */
  id: string;
  value: T;
  options: { value: T; label: string }[];
  onChange: (value: T) => void;
  disabled?: boolean;
  title?: string;
  className?: string;
}

export function SegmentedControl<T extends string>({
  id,
  value,
  options,
  onChange,
  disabled,
  title,
  className,
}: SegmentedControlProps<T>) {
  return (
    <div
      title={title}
      className={cn(
        "flex rounded-(--radius-ctl) border border-interactive-border bg-surface-2/50 p-1",
        disabled && "opacity-65 saturate-50",
        className,
      )}
    >
      {options.map((o) => {
        const active = o.value === value;
        return (
          <button
            key={o.value}
            type="button"
            disabled={disabled}
            onClick={() => onChange(o.value)}
            className={cn(
              "relative flex h-9 flex-1 items-center justify-center rounded-[9px] px-4 text-[13px] font-medium whitespace-nowrap outline-none",
              "transition-colors duration-150 disabled:pointer-events-none focus-visible:ring-2 focus-visible:ring-focus-ring",
              active ? "text-text" : "text-text-dim hover:text-text",
            )}
          >
            {active && (
              <motion.span
                layoutId={`seg-thumb-${id}`}
                className="absolute inset-0 rounded-[9px] border border-selected-border bg-selected-surface"
                transition={{ type: "spring", stiffness: 450, damping: 34 }}
              />
            )}
            <span className="relative z-10">{o.label}</span>
          </button>
        );
      })}
    </div>
  );
}
