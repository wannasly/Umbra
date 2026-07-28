import { forwardRef, type HTMLAttributes, type KeyboardEvent } from "react";
import { cn } from "../../lib/cn";

interface GlassCardProps extends HTMLAttributes<HTMLDivElement> {
  interactive?: boolean;
  variant?: "panel" | "raised" | "glass";
}

export const GlassCard = forwardRef<HTMLDivElement, GlassCardProps>(
  function GlassCard(
    {
      interactive,
      variant = "panel",
      className,
      tabIndex,
      onKeyDown,
      onClick,
      ...rest
    },
    ref,
  ) {
    const handleKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
      onKeyDown?.(e);
      if (
        !e.defaultPrevented &&
        interactive &&
        e.target === e.currentTarget &&
        (e.key === "Enter" || e.key === " ")
      ) {
        e.preventDefault();
        e.currentTarget.click();
      }
    };

    return (
      <div
        ref={ref}
        className={cn(
          variant === "panel" && "surface-panel",
          variant === "raised" && "surface-raised",
          variant === "glass" && "glass",
          "rounded-(--radius-card)",
          interactive &&
            "cursor-pointer outline-none transition-[background-color,border-color,box-shadow,transform] duration-150",
          interactive &&
            "hover:border-interactive-border-hover hover:bg-hover-surface active:scale-[0.995]",
          interactive &&
            "focus-visible:border-focus-ring focus-visible:ring-2 focus-visible:ring-focus-ring focus-visible:ring-offset-2 focus-visible:ring-offset-surface-0",
          className,
        )}
        tabIndex={interactive ? (tabIndex ?? 0) : tabIndex}
        onKeyDown={handleKeyDown}
        onClick={onClick}
        {...rest}
      />
    );
  },
);
