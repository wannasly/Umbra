// How the Servers page reads an entry: is it a server you can connect to, and
// in what order should the list show it.

import { readServerName } from "./flags";
import type { ServerEntry, ServerSort } from "./ipc";

/**
 * Addresses that can never carry a tunnel. Panels inject entries at these to
 * smuggle a message into the client's server list ("Осталось дней — 19").
 */
const UNROUTABLE = new Set(["0.0.0.0", "::", "::0", "127.0.0.1", "::1", "localhost"]);

/**
 * Phrases that make a name a message rather than a place.
 *
 * The bar is deliberately high: a false positive silently demotes a server the
 * user is trying to connect through, which is far worse than an info row left
 * sitting among the servers. Every pattern here is something no one would name
 * an exit node — a link, a countdown, an instruction.
 *
 * Words that merely *tend* to appear in notices are not enough, which is why
 * "support", "click", "press" and "contact" are absent: they demoted a
 * perfectly ordinary "Support-Node-3". Real support notices carry a link or a
 * handle, and those are matched above.
 */
const MESSAGE_PATTERNS: RegExp[] = [
  /https?:\/\//i,
  /\bt\.me\//i,
  /(^|[\s([])@[a-z0-9_]{4,}/i,
  // countdowns and expiry
  /остал[оа]сь/i,
  /истека|истёк|истек\b/i,
  /\bdays?\s+(left|remaining)\b/i,
  /\bexpir(es|ed|ing)\b/i,
  // account / plan chatter. No `\b` around the Cyrillic stems: JS word
  // boundaries are ASCII-only, so `/трафик\b/` never matches "Трафик:".
  /подписк|продлит|продлен|тариф/i,
  /трафик/i,
  /\bsubscription\b/i,
  /\brenew\b/i,
  // instructions
  /не работает|нажим|нажат|обновит|обновлени|включит/i,
  /поддержк/i,
];

/**
 * True when an entry is a notice the panel injected, not a server.
 *
 * Two independent signals: an address no tunnel can use, or a name that reads
 * as a sentence. Either one is enough; neither is ever guessed from something
 * as weak as "the name is long".
 */
export function isInfoEntry(s: ServerEntry): boolean {
  if (UNROUTABLE.has(s.server.trim().toLowerCase())) return true;
  // Port 1 is tcpmux; no proxy has ever been served there, and it is what the
  // placeholder entries use.
  if (s.port === 1) return true;
  return MESSAGE_PATTERNS.some((re) => re.test(s.name));
}

const byName = (a: ServerEntry, b: ServerEntry): number =>
  readServerName(a.name).label.localeCompare(readServerName(b.name).label, undefined, {
    numeric: true,
    sensitivity: "base",
  });

/**
 * Fastest first, untested last — a server with no measurement is not "0 ms",
 * and sorting it to the top would be a lie the user acts on.
 */
const byPing = (a: ServerEntry, b: ServerEntry): number => {
  const pa = a.lastPingMs ?? Number.POSITIVE_INFINITY;
  const pb = b.lastPingMs ?? Number.POSITIVE_INFINITY;
  return pa === pb ? byName(a, b) : pa - pb;
};

/** Sort a copy; "default" keeps the order the panel delivered. */
export function sortServers(servers: ServerEntry[], sort: ServerSort): ServerEntry[] {
  if (sort === "default") return servers;
  return [...servers].sort(sort === "ping" ? byPing : byName);
}

export interface GroupContent {
  /** panel notices, floated above the servers */
  info: ServerEntry[];
  /** everything you can actually connect to, in the requested order */
  servers: ServerEntry[];
}

/**
 * Split a group into notices and servers. Notices keep the panel's own order
 * (they are usually written to be read top to bottom) and are never sorted by
 * ping or name, which would be meaningless for them.
 */
export function splitGroup(entries: ServerEntry[], sort: ServerSort): GroupContent {
  const info: ServerEntry[] = [];
  const servers: ServerEntry[] = [];
  for (const entry of entries) (isInfoEntry(entry) ? info : servers).push(entry);
  return { info, servers: sortServers(servers, sort) };
}
