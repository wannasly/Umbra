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
        "inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-(--radius-ctl) text-text-dim",
        "transition-[background-color,color] duration-150 disabled:pointer-events-none disabled:opacity-45",
        danger
          ? "hover:bg-danger/15 hover:text-danger"
          : "hover:bg-glass-strong hover:text-text",
        className,
      )}
      {...rest}
    />
  );
}
