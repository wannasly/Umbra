import type { FlagInfo } from "../../lib/flags";
import { cn } from "../../lib/cn";

/**
 * The country as its own element instead of an emoji glued to the name.
 *
 * Windows renders regional-indicator emoji as two letters. A bundled color
 * flag font supplies the missing glyphs in WebView, while the outer box keeps
 * every server name starting at the same position.
 */
export function FlagChip({
  flag,
  className,
}: {
  flag: FlagInfo | null;
  className?: string;
}) {
  return (
    <span
      title={flag?.code}
      aria-hidden={flag === null}
      className={cn(
        "flex h-6 w-7 shrink-0 items-center justify-center rounded-[7px] border border-glass-border bg-glass",
        "text-[16px] leading-none",
        flag === null && "opacity-35",
        className,
      )}
    >
      {flag ? (
        <span
          aria-hidden="true"
          className="drop-shadow-[0_0_1px_rgb(255_255_255/0.3)]"
          style={{ fontFamily: '"Twemoji Country Flags", sans-serif' }}
        >
          {flag.emoji}
        </span>
      ) : (
        <span className="text-[13px] leading-none text-text-faint">·</span>
      )}
    </span>
  );
}
