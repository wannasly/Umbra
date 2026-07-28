import { motion } from "motion/react";
import { cn } from "../../lib/cn";

interface ToggleProps {
  checked: boolean;
  onChange: (checked: boolean) => void;
  disabled?: boolean;
  "aria-label"?: string;
}

export function Toggle({
  checked,
  onChange,
  disabled,
  "aria-label": ariaLabel,
}: ToggleProps) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={ariaLabel}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className={cn(
        "relative h-9 w-11 shrink-0 rounded-full outline-none",
        "transition-opacity duration-150 disabled:pointer-events-none disabled:opacity-60",
        "focus-visible:ring-2 focus-visible:ring-focus-ring focus-visible:ring-offset-2 focus-visible:ring-offset-surface-0",
      )}
    >
      <span
        className={cn(
          "absolute top-1/2 left-0.5 h-[22px] w-10 -translate-y-1/2 rounded-full border",
          "transition-[background-color,border-color] duration-150",
          checked
            ? "border-transparent bg-linear-90 from-accent to-accent-2"
            : "border-interactive-border bg-surface-2",
        )}
      >
        <motion.span
          className="absolute top-[2px] left-[2px] block h-4 w-4 rounded-full bg-white shadow-[0_1px_3px_rgb(0_0_0/0.4)]"
          animate={{ x: checked ? 18 : 0 }}
          transition={{ type: "spring", stiffness: 550, damping: 32 }}
        />
      </span>
    </button>
  );
}
