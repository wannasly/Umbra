import { forwardRef, type HTMLAttributes } from "react";
import { cn } from "../../lib/cn";

interface GlassCardProps extends HTMLAttributes<HTMLDivElement> {
  interactive?: boolean;
}

export const GlassCard = forwardRef<HTMLDivElement, GlassCardProps>(
  function GlassCard({ interactive, className, ...rest }, ref) {
    return (
      <div
        ref={ref}
        className={cn(
          "glass rounded-(--radius-card)",
          interactive &&
            "cursor-pointer transition-[background-color,border-color] duration-150 hover:bg-glass-strong",
          className,
        )}
        {...rest}
      />
    );
  },
);
