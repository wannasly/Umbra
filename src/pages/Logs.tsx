import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { AnimatePresence, motion } from "motion/react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { Copy, Eraser, Pause, Play, ScrollText, Search } from "lucide-react";
import { ipc, type LogLine } from "../lib/ipc";
import { useLogs } from "../stores/logs";
import { useUi, toastError } from "../stores/ui";
import { formatClock } from "../lib/format";
import { cn } from "../lib/cn";
import { Button } from "../components/ui/Button";
import { TextField } from "../components/ui/TextField";
import { GlassCard } from "../components/ui/GlassCard";
import { EmptyState } from "../components/ui/EmptyState";
import { PageShell } from "../components/ui/PageShell";

type LevelFilter = "all" | "debug" | "info" | "warn" | "error";

const LEVEL_TAG: Record<LogLine["level"], string> = {
  trace: "TRC",
  debug: "DBG",
  info: "INF",
  warn: "WRN",
  error: "ERR",
};

const LEVEL_COLOR: Record<LogLine["level"], string> = {
  trace: "text-text-faint",
  debug: "text-text-faint",
  info: "text-accent-2",
  warn: "text-warn",
  error: "text-danger",
};

const CHIP_ACTIVE: Record<LevelFilter, string> = {
  all: "text-text border-glass-edge",
  debug: "text-text-faint border-glass-edge",
  info: "text-accent-2 border-accent-2/40",
  warn: "text-warn border-warn/40",
  error: "text-danger border-danger/40",
};

function matchesLevel(line: LogLine, filter: LevelFilter): boolean {
  if (filter === "all") return true;
  if (filter === "debug") return line.level === "debug" || line.level === "trace";
  return line.level === filter;
}

export function Logs() {
  const { t } = useTranslation();
  const lines = useLogs((s) => s.lines);
  const paused = useLogs((s) => s.paused);
  const autoscroll = useLogs((s) => s.autoscroll);
  const newCount = useLogs((s) => s.newCount);
  const seed = useLogs((s) => s.seed);
  const setPaused = useLogs((s) => s.setPaused);
  const setAutoscroll = useLogs((s) => s.setAutoscroll);
  const clearStore = useLogs((s) => s.clear);
  const pushToast = useUi((s) => s.toast);

  const [level, setLevel] = useState<LevelFilter>("all");
  const [query, setQuery] = useState("");

  const parentRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    void ipc("get_recent_logs", { limit: 500 }).then(seed).catch(toastError);
  }, [seed]);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    return lines.filter(
      (line) =>
        matchesLevel(line, level) &&
        (q === "" || line.message.toLowerCase().includes(q)),
    );
  }, [lines, level, query]);

  const virtualizer = useVirtualizer({
    count: filtered.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => 22,
    overscan: 20,
  });

  // pin to bottom while autoscroll is on
  useEffect(() => {
    if (!autoscroll) return;
    const el = parentRef.current;
    if (!el) return;
    el.scrollTop = el.scrollHeight;
  }, [filtered.length, autoscroll]);

  const onScroll = () => {
    const el = parentRef.current;
    if (!el) return;
    const atBottom = el.scrollTop + el.clientHeight >= el.scrollHeight - 48;
    if (atBottom !== autoscroll) setAutoscroll(atBottom);
  };

  const resumeScroll = () => {
    setAutoscroll(true);
    const el = parentRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  };

  const copyVisible = () => {
    const text = filtered
      .map(
        (l) => `${formatClock(l.ts)} ${LEVEL_TAG[l.level]} ${l.message}`,
      )
      .join("\n");
    void navigator.clipboard
      .writeText(text)
      .then(() => {
        pushToast({ kind: "success", title: t("logs.copied") });
      })
      .catch(toastError);
  };

  const clearAll = () => {
    void ipc("clear_logs").catch(toastError);
    clearStore();
  };

  const levels: LevelFilter[] = ["all", "debug", "info", "warn", "error"];

  return (
    <PageShell width="wide" fill className="gap-4">
      {/* toolbar — wraps instead of overflowing; the search box is the only
          elastic element so the chips and buttons keep their natural size */}
      <div className="flex shrink-0 flex-wrap items-center gap-2">
        <div className="flex flex-wrap items-center gap-1.5">
          {levels.map((lv) => (
            <button
              key={lv}
              type="button"
              onClick={() => setLevel(lv)}
              className={cn(
                "rounded-(--radius-chip) border px-3 py-1 text-xs font-medium transition-colors duration-150",
                lv === level
                  ? cn("bg-glass-strong", CHIP_ACTIVE[lv])
                  : "border-transparent text-text-faint hover:bg-glass hover:text-text-dim",
              )}
            >
              {t(`logs.levels.${lv}`)}
            </button>
          ))}
        </div>
        <TextField
          icon={<Search size={14} />}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder={t("logs.search")}
          wrapClassName="ml-auto w-[clamp(8rem,18vw,14rem)] min-w-0"
        />
        <Button
          variant="ghost"
          icon={paused ? <Play size={14} /> : <Pause size={14} />}
          onClick={() => setPaused(!paused)}
        >
          {paused ? t("logs.resume") : t("logs.pause")}
        </Button>
        <Button
          variant="ghost"
          icon={<Copy size={14} />}
          disabled={filtered.length === 0}
          onClick={copyVisible}
        >
          {t("logs.copy")}
        </Button>
        <Button
          variant="ghost"
          icon={<Eraser size={14} />}
          disabled={lines.length === 0}
          onClick={clearAll}
        >
          {t("logs.clear")}
        </Button>
      </div>

      {/* body */}
      <GlassCard className="relative min-h-0 flex-1 overflow-hidden">
        {filtered.length === 0 ? (
          <EmptyState
            icon={<ScrollText size={24} />}
            title={t("logs.empty.title")}
            hint={t("logs.empty.hint")}
          />
        ) : (
          <div
            ref={parentRef}
            onScroll={onScroll}
            className="h-full overflow-x-hidden overflow-y-auto px-4 py-3"
          >
            <div
              style={{
                height: virtualizer.getTotalSize(),
                position: "relative",
              }}
            >
              {virtualizer.getVirtualItems().map((row) => {
                const line = filtered[row.index];
                return (
                  <div
                    key={row.key}
                    className="absolute top-0 left-0 flex w-full items-baseline gap-2.5 font-mono text-[12.5px] leading-[22px] whitespace-nowrap select-text"
                    style={{ transform: `translateY(${row.start}px)` }}
                  >
                    <span className="shrink-0 text-text-faint tabular-nums">
                      {formatClock(line.ts)}
                    </span>
                    <span
                      className={cn(
                        "shrink-0 font-semibold",
                        LEVEL_COLOR[line.level],
                      )}
                    >
                      {LEVEL_TAG[line.level]}
                    </span>
                    <span
                      className="min-w-0 flex-1 truncate text-text-dim"
                      title={line.message}
                    >
                      {line.message}
                    </span>
                  </div>
                );
              })}
            </div>
          </div>
        )}

        {/* new lines pill */}
        <AnimatePresence>
          {!autoscroll && newCount > 0 && (
            <motion.button
              type="button"
              onClick={resumeScroll}
              initial={{ opacity: 0, y: 8, x: "-50%" }}
              animate={{ opacity: 1, y: 0, x: "-50%" }}
              exit={{ opacity: 0, y: 8, x: "-50%" }}
              transition={{ duration: 0.15 }}
              className="glass absolute bottom-4 left-1/2 z-10 rounded-(--radius-chip) bg-bg-raise/80 px-4 py-1.5 text-xs font-semibold text-accent-2"
            >
              {t("logs.newLines", { count: newCount })} ↓
            </motion.button>
          )}
        </AnimatePresence>
      </GlassCard>
    </PageShell>
  );
}
