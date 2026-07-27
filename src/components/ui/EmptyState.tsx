import type { ReactNode } from "react";

interface EmptyStateProps {
  icon: ReactNode;
  title: string;
  hint?: string;
  action?: ReactNode;
}

export function EmptyState({ icon, title, hint, action }: EmptyStateProps) {
  return (
    <div className="flex flex-col items-center justify-center gap-5 py-16 text-center">
      <div className="flex h-24 w-24 items-center justify-center rounded-full border border-glass-border bg-glass">
        <div className="flex h-16 w-16 items-center justify-center rounded-full border border-glass-border bg-glass-strong text-text-dim">
          {icon}
        </div>
      </div>
      <div>
        <div className="font-display text-[15px] font-bold text-text">
          {title}
        </div>
        {hint && (
          <div className="mx-auto mt-1.5 max-w-72 text-[13px] leading-relaxed text-text-dim">
            {hint}
          </div>
        )}
      </div>
      {action}
    </div>
  );
}
