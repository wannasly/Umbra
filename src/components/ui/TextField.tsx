import type { InputHTMLAttributes, ReactNode } from "react";
import { cn } from "../../lib/cn";

interface TextFieldProps extends InputHTMLAttributes<HTMLInputElement> {
  icon?: ReactNode;
  mono?: boolean;
  /** class for the outer wrapper; use className for the input itself */
  wrapClassName?: string;
}

export function TextField({
  icon,
  mono,
  wrapClassName,
  className,
  ...rest
}: TextFieldProps) {
  return (
    <div className={cn("relative", wrapClassName)}>
      {icon && (
        <span className="pointer-events-none absolute top-1/2 left-3 -translate-y-1/2 text-text-faint">
          {icon}
        </span>
      )}
      <input
        className={cn(
          "h-9 w-full rounded-(--radius-ctl) border border-glass-border bg-glass px-3 text-[13px] text-text select-text",
          "outline-none placeholder:text-text-faint",
          "transition-[border-color,box-shadow] duration-150 focus:border-accent/60 focus:ring-2 focus:ring-accent/40",
          icon && "pl-9",
          mono && "font-mono text-xs",
          className,
        )}
        {...rest}
      />
    </div>
  );
}
