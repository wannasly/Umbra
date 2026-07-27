import { create } from "zustand";
import type { LogLine } from "../lib/ipc";

const MAX_LINES = 5000;

interface LogsStore {
  lines: LogLine[];
  /** stream paused by the user; incoming lines buffer and flush on resume */
  paused: boolean;
  /** autoscroll pinned to bottom; turned off when the user scrolls up */
  autoscroll: boolean;
  /** lines arrived while autoscroll is off (floating "new lines" pill) */
  newCount: number;
  append: (batch: LogLine[]) => void;
  seed: (lines: LogLine[]) => void;
  setPaused: (paused: boolean) => void;
  setAutoscroll: (autoscroll: boolean) => void;
  clear: () => void;
}

let pausedBuffer: LogLine[] = [];

export const useLogs = create<LogsStore>((set, get) => ({
  lines: [],
  paused: false,
  autoscroll: true,
  newCount: 0,
  append: (batch) => {
    if (batch.length === 0) return;
    if (get().paused) {
      pausedBuffer = [...pausedBuffer, ...batch].slice(-MAX_LINES);
      return;
    }
    set((s) => ({
      lines: [...s.lines, ...batch].slice(-MAX_LINES),
      newCount: s.autoscroll ? 0 : s.newCount + batch.length,
    }));
  },
  seed: (lines) =>
    set((s) => {
      if (s.lines.length === 0) return { lines: lines.slice(-MAX_LINES) };
      // prepend only history that predates what already streamed in
      const firstTs = s.lines[0].ts;
      const older = lines.filter((l) => l.ts < firstTs);
      return older.length > 0
        ? { lines: [...older, ...s.lines].slice(-MAX_LINES) }
        : s;
    }),
  setPaused: (paused) => {
    if (!paused && pausedBuffer.length > 0) {
      const flush = pausedBuffer;
      pausedBuffer = [];
      set((s) => ({
        paused,
        lines: [...s.lines, ...flush].slice(-MAX_LINES),
        newCount: s.autoscroll ? 0 : s.newCount + flush.length,
      }));
      return;
    }
    set({ paused });
  },
  setAutoscroll: (autoscroll) =>
    set({ autoscroll, ...(autoscroll ? { newCount: 0 } : null) }),
  clear: () => {
    pausedBuffer = [];
    set({ lines: [], newCount: 0 });
  },
}));
