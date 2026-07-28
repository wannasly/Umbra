import type { ButtonHTMLAttributes } from "react";
import { cn } from "../../lib/cn";

interface IconButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  danger?: boolean;
}

export function IconButton({ danger, className, ...rest }: IconButtonProps) {
  return (
    <button
      type="button"
      className={cn(
        "inline-flex h-9 w-9 shrink-0 items-center justify-center rounded-(--radius-ctl) text-text-dim outline-none",
        "transition-[background-color,border-color,color,transform] duration-150 active:scale-95 disabled:pointer-events-none disabled:opacity-60",
        "focus-visible:ring-2 focus-visible:ring-focus-ring focus-visible:ring-offset-2 focus-visible:ring-offset-surface-0",
        danger
          ? "hover:bg-danger/15 hover:text-danger"
          : "hover:bg-hover-surface hover:text-text",
        className,
      )}
      {...rest}
    />
  );
}
