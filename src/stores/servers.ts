import { create } from "zustand";
import {
  ipc,
  type PingResult,
  type ServerEntry,
  type ServersList,
  type Subscription,
} from "../lib/ipc";
import { toastError } from "./ui";

/**
 * A request to scroll a server into view and flash it. `nonce` is what makes
 * two consecutive requests for the *same* server distinguishable, so clicking
 * the dashboard pill twice scrolls twice.
 */
export interface RevealRequest {
  serverId: string;
  nonce: number;
}

interface ServersStore {
  list: ServersList;
  loaded: boolean;
  /** server ids with an in-flight ping test */
  pendingPings: Set<string>;
  /** pending scroll-to-server on the Servers page, consumed by it */
  reveal: RevealRequest | null;
  refresh: () => Promise<void>;
  ping: (ids: string[]) => Promise<void>;
  onPingResult: (r: PingResult) => void;
  removeServerLocal: (id: string) => void;
  removeSubscriptionLocal: (id: string) => void;
  addSubscriptionLocal: (sub: Subscription) => void;
  updateSubscriptionLocal: (sub: Subscription) => void;
  setAutoUpdateHours: (id: string, hours: number) => void;
  toggleFavorite: (id: string) => void;
  /** move a subscription group one slot up (-1) or down (+1) */
  moveSubscription: (id: string, delta: number) => void;
  revealServer: (id: string) => void;
  clearReveal: () => void;
}

const patchPing = (servers: ServerEntry[], r: PingResult): ServerEntry[] =>
  servers.map((s) =>
    s.id === r.serverId ? { ...s, lastPingMs: r.latencyMs } : s,
  );

/** Apply a change to whichever list the server lives in. */
const mapServers = (
  list: ServersList,
  fn: (servers: ServerEntry[]) => ServerEntry[],
): ServersList => ({
  manual: fn(list.manual),
  subscriptions: list.subscriptions.map((sub) => ({
    ...sub,
    servers: fn(sub.servers),
  })),
});

let revealNonce = 0;

export const useServers = create<ServersStore>((set, get) => ({
  list: { manual: [], subscriptions: [] },
  loaded: false,
  pendingPings: new Set(),
  reveal: null,
  refresh: async () => {
    const list = await ipc("list_servers");
    set({ list, loaded: true });
  },
  ping: async (ids) => {
    if (ids.length === 0) return;
    set((s) => ({ pendingPings: new Set([...s.pendingPings, ...ids]) }));
    try {
      await ipc("ping_servers", { ids });
    } catch (e) {
      // command failed: no ping://result events will come — clear the shimmer
      set((s) => {
        const pendingPings = new Set(s.pendingPings);
        ids.forEach((id) => pendingPings.delete(id));
        return { pendingPings };
      });
      toastError(e);
    }
  },
  onPingResult: (r) =>
    set((s) => {
      const pendingPings = new Set(s.pendingPings);
      pendingPings.delete(r.serverId);
      return {
        pendingPings,
        list: mapServers(s.list, (servers) => patchPing(servers, r)),
      };
    }),
  removeServerLocal: (id) =>
    set((s) => ({
      list: mapServers(s.list, (servers) => servers.filter((x) => x.id !== id)),
    })),
  removeSubscriptionLocal: (id) =>
    set((s) => ({
      list: {
        ...s.list,
        subscriptions: s.list.subscriptions.filter((x) => x.id !== id),
      },
    })),
  addSubscriptionLocal: (sub) =>
    set((s) => ({
      list: { ...s.list, subscriptions: [...s.list.subscriptions, sub] },
    })),
  updateSubscriptionLocal: (sub) =>
    set((s) => ({
      list: {
        ...s.list,
        subscriptions: s.list.subscriptions.map((x) =>
          x.id === sub.id ? sub : x,
        ),
      },
    })),
  setAutoUpdateHours: (id, hours) => {
    set((s) => ({
      list: {
        ...s.list,
        subscriptions: s.list.subscriptions.map((sub) =>
          sub.id === id ? { ...sub, autoUpdateHours: hours } : sub,
        ),
      },
    }));
    void ipc("set_subscription_auto_update", { id, hours }).catch(toastError);
  },
  /**
   * Optimistic: the star is a one-click toggle, so it has to move now and be
   * rolled back if the backend disagrees, not wait a round trip.
   */
  toggleFavorite: (id) => {
    const current = findServer(id)?.favorite ?? false;
    const favorite = !current;
    set((s) => ({
      list: mapServers(s.list, (servers) =>
        servers.map((x) => (x.id === id ? { ...x, favorite } : x)),
      ),
    }));
    void ipc("set_server_favorite", { id, favorite }).catch((e) => {
      set((s) => ({
        list: mapServers(s.list, (servers) =>
          servers.map((x) => (x.id === id ? { ...x, favorite: current } : x)),
        ),
      }));
      toastError(e);
    });
  },
  moveSubscription: (id, delta) => {
    const subs = get().list.subscriptions;
    const from = subs.findIndex((s) => s.id === id);
    const to = from + delta;
    if (from < 0 || to < 0 || to >= subs.length) return;
    const next = [...subs];
    [next[from], next[to]] = [next[to], next[from]];
    set((s) => ({ list: { ...s.list, subscriptions: next } }));
    void ipc("reorder_subscriptions", { ids: next.map((s) => s.id) }).catch(
      (e) => {
        set((s) => ({ list: { ...s.list, subscriptions: subs } }));
        toastError(e);
      },
    );
  },
  revealServer: (serverId) =>
    set({ reveal: { serverId, nonce: ++revealNonce } }),
  clearReveal: () => set({ reveal: null }),
}));

/** Flat list of every known server (manual + all subscriptions). */
export const allServers = (list: ServersList): ServerEntry[] => [
  ...list.manual,
  ...list.subscriptions.flatMap((sub) => sub.servers),
];

export const findServer = (id: string | null): ServerEntry | undefined =>
  id ? allServers(useServers.getState().list).find((s) => s.id === id) : undefined;
