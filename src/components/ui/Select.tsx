import { useRef, useState } from "react";
import { Check, ChevronDown } from "lucide-react";
import { cn } from "../../lib/cn";
import { Popover } from "./Popover";

export interface SelectOption<T extends string> {
  value: T;
  label: string;
}

interface SelectProps<T extends string> {
  value: T;
  options: SelectOption<T>[];
  onChange: (value: T) => void;
  disabled?: boolean;
  className?: string;
}

export function Select<T extends string>({
  value,
  options,
  onChange,
  disabled,
  className,
}: SelectProps<T>) {
  const [open, setOpen] = useState(false);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const current = options.find((o) => o.value === value);

  return (
    <div className={cn("relative", className)}>
      <button
        ref={triggerRef}
        type="button"
        disabled={disabled}
        aria-haspopup="listbox"
        aria-expanded={open}
        onClick={() => setOpen((o) => !o)}
        className={cn(
          "flex h-9 w-full items-center justify-between gap-2 rounded-(--radius-ctl) border border-interactive-border bg-surface-2/55 px-3 text-[13px] text-text outline-none",
          "transition-[background-color,border-color,box-shadow] duration-150 hover:border-interactive-border-hover hover:bg-hover-surface",
          "focus-visible:ring-2 focus-visible:ring-focus-ring focus-visible:ring-offset-2 focus-visible:ring-offset-surface-0",
          "disabled:pointer-events-none disabled:opacity-60",
        )}
      >
        <span className="truncate">{current?.label ?? ""}</span>
        <ChevronDown
          size={14}
          className={cn(
            "shrink-0 text-text-faint transition-transform duration-150",
            open && "rotate-180",
          )}
        />
      </button>
      <Popover open={open} onClose={() => setOpen(false)} anchorRef={triggerRef}>
        {options.map((o) => (
          <button
            key={o.value}
            type="button"
            onClick={() => {
              onChange(o.value);
              setOpen(false);
            }}
            className={cn(
              "flex w-full items-center justify-between gap-3 px-3 py-1.5 text-left text-[13px] whitespace-nowrap",
              "outline-none transition-colors duration-100 hover:bg-hover-surface focus-visible:bg-hover-surface focus-visible:text-text",
              o.value === value ? "text-text" : "text-text-dim",
            )}
          >
            {o.label}
            {o.value === value && (
              <Check size={13} className="shrink-0 text-accent" />
            )}
          </button>
        ))}
      </Popover>
    </div>
  );
}
