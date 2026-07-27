import type { ButtonHTMLAttributes, ReactNode } from "react";
import { cn } from "../../lib/cn";
import { Spinner } from "./Spinner";

interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: "primary" | "ghost" | "danger";
  loading?: boolean;
  icon?: ReactNode;
}

export function Button({
  variant = "primary",
  loading = false,
  icon,
  className,
  children,
  disabled,
  ...rest
}: ButtonProps) {
  return (
    <button
      type="button"
      className={cn(
        "inline-flex h-9 shrink-0 items-center justify-center gap-2 rounded-(--radius-ctl) px-4 text-[13px] font-semibold select-none",
        "transition-[background-color,border-color,box-shadow,color,transform] duration-150 active:scale-[0.97]",
        "disabled:pointer-events-none disabled:opacity-45",
        // text-bg-deep, not white: the sea accents are light enough that white
        // label text on them sits around 2.5:1. Dark ink on the gradient is the
        // legible half of the pair.
        variant === "primary" &&
          "bg-linear-135 from-accent to-accent-2 text-bg-deep hover:shadow-(--shadow-glow-accent)",
        variant === "ghost" &&
          "border border-glass-border bg-glass text-text-dim hover:bg-glass-strong hover:text-text",
        variant === "danger" &&
          "border border-danger/30 bg-danger/10 text-danger hover:bg-danger/20",
        className,
      )}
      disabled={disabled || loading}
      {...rest}
    >
      {loading ? <Spinner size={14} /> : icon}
      {children}
    </button>
  );
}
