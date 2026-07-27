import type { ReactNode } from "react";
import { cn } from "../../lib/cn";

/**
 * Content column for a page.
 *
 * The shell's <main> is a fluid flex child that spans whatever the window
 * leaves after the sidebar, so a page must never assume the 1100x720 default.
 * The column takes all of that width until it hits a readability cap, then
 * centres itself — fluid on small windows, a centred column on a maximised
 * one, never a full-bleed stretch. A cap (rather than a viewport-relative
 * width) is deliberate: `max-width` can only ever narrow the column, so a
 * vw-based value would shrink it on exactly the small windows that need the
 * room most.
 *
 *   form — settings-style rows, narrowest (labels + controls on one line)
 *   list — cards and lists (servers, subscriptions)
 *   wide — monospace/tabular content that benefits from horizontal room (logs)
 */
const WIDTH = {
  form: "44rem",
  list: "62rem",
  wide: "82rem",
} as const;

interface PageShellProps {
  width?: keyof typeof WIDTH;
  /** Page owns the full height and scrolls internally (Logs). */
  fill?: boolean;
  className?: string;
  children: ReactNode;
}

export function PageShell({
  width = "list",
  fill,
  className,
  children,
}: PageShellProps) {
  return (
    <div
      style={{ maxWidth: WIDTH[width] }}
      className={cn(
        // min-w-0 so long unbreakable content (hostnames, log lines) truncates
        // inside the column instead of widening it past the sidebar.
        "mx-auto w-full min-w-0",
        fill ? "flex h-full min-h-0 flex-col" : "flex flex-col",
        className,
      )}
    >
      {children}
    </div>
  );
}
