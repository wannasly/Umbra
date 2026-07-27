import { motion } from "motion/react";
import { Info } from "lucide-react";
import type { ServerEntry } from "../../lib/ipc";
import { openExternal } from "../../lib/open";
import { cn } from "../../lib/cn";

const itemVariants = {
  hidden: { opacity: 0, y: 8 },
  show: { opacity: 1, y: 0 },
};

/** Turn bare links inside a notice into something clickable. */
const LINK = /(https?:\/\/\S+|(?:^|\s)t\.me\/\S+)/g;

function Linkified({ text }: { text: string }) {
  const parts = text.split(LINK).filter((p) => p !== "");
  return (
    <>
      {parts.map((part, i) => {
        const trimmed = part.trim();
        const isLink = /^(https?:\/\/|t\.me\/)/.test(trimmed);
        if (!isLink) return <span key={i}>{part}</span>;
        const href = trimmed.startsWith("http") ? trimmed : `https://${trimmed}`;
        return (
          <button
            key={i}
            type="button"
            onClick={(e) => {
              e.stopPropagation();
              openExternal(href);
            }}
            className="text-accent underline decoration-accent/40 underline-offset-2 hover:decoration-accent"
          >
            {trimmed}
          </button>
        );
      })}
    </>
  );
}

/**
 * A notice the panel smuggled into the server list ("Осталось дней — 19", a
 * support link). Deliberately not a card and not clickable: it must never look
 * like something you can connect through, because you cannot.
 */
export function InfoRow({
  entry,
  className,
}: {
  entry: ServerEntry;
  className?: string;
}) {
  return (
    <motion.div
      variants={itemVariants}
      className={cn(
        "flex items-start gap-2.5 rounded-(--radius-card) border border-dashed border-glass-border/70 px-3.5 py-2",
        "text-[13px] leading-snug text-text-dim select-text",
        className,
      )}
    >
      <Info size={14} className="mt-0.5 shrink-0 text-text-faint" />
      <span className="min-w-0 break-words">
        <Linkified text={entry.name.trim()} />
      </span>
    </motion.div>
  );
}
