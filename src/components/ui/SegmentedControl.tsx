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
        "flex rounded-(--radius-ctl) border border-glass-border bg-glass p-1",
        disabled && "opacity-55",
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
              "relative flex-1 rounded-[9px] px-4 py-1.5 text-[13px] font-medium whitespace-nowrap",
              "transition-colors duration-150 disabled:pointer-events-none",
              active ? "text-text" : "text-text-dim hover:text-text",
            )}
          >
            {active && (
              <motion.span
                layoutId={`seg-thumb-${id}`}
                className="absolute inset-0 rounded-[9px] border border-glass-border bg-glass-strong"
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
