import { create } from "zustand";
import type { ConnectionState, TrafficStats } from "../lib/ipc";

export interface TrafficSample {
  up: number;
  down: number;
}

/** 60 visible samples + 1 incoming (conveyor belt) */
export const SAMPLE_COUNT = 61;

const emptySamples = (): TrafficSample[] =>
  Array.from({ length: SAMPLE_COUNT }, () => ({ up: 0, down: 0 }));

interface ConnectionStore {
  conn: ConnectionState;
  samples: TrafficSample[];
  /** increments on every traffic sample; drives the chart conveyor animation */
  tick: number;
  upBps: number;
  downBps: number;
  upTotal: number;
  downTotal: number;
  setConn: (conn: ConnectionState) => void;
  pushTraffic: (t: TrafficStats) => void;
}

export const useConnection = create<ConnectionStore>((set) => ({
  conn: {
    status: "disconnected",
    serverId: null,
    serverName: null,
    mode: "system_proxy",
    sinceMs: null,
    error: null,
  },
  samples: emptySamples(),
  tick: 0,
  upBps: 0,
  downBps: 0,
  upTotal: 0,
  downTotal: 0,
  setConn: (conn) =>
    set((s) => {
      const freshSession =
        conn.status === "connecting" && s.conn.status === "disconnected";
      return {
        conn,
        ...(freshSession
          ? {
              samples: emptySamples(),
              tick: 0,
              upBps: 0,
              downBps: 0,
              upTotal: 0,
              downTotal: 0,
            }
          : null),
        ...(conn.status === "disconnected" ? { upBps: 0, downBps: 0 } : null),
      };
    }),
  pushTraffic: (t) =>
    set((s) => ({
      samples: [...s.samples.slice(1), { up: t.upBps, down: t.downBps }],
      tick: s.tick + 1,
      upBps: t.upBps,
      downBps: t.downBps,
      upTotal: t.upTotal,
      downTotal: t.downTotal,
    })),
}));
