import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { AnimatePresence, motion } from "motion/react";
import {
  ClipboardPaste,
  Plus,
  Search,
  ServerOff,
  Star,
  Zap,
} from "lucide-react";
import {
  GROUP_FAVORITES,
  GROUP_MANUAL,
  ipc,
  type ServerEntry,
  type ServerSort,
  type SubscriptionQuota,
} from "../lib/ipc";
import { useServers, allServers } from "../stores/servers";
import { useSettings } from "../stores/settings";
import { useConnection } from "../stores/connection";
import { useUi, toastError } from "../stores/ui";
import { isInfoEntry, splitGroup, type GroupContent } from "../lib/serverMeta";
import { cn } from "../lib/cn";
import { GlassCard } from "../components/ui/GlassCard";
import { Button } from "../components/ui/Button";
import { TextField } from "../components/ui/TextField";
import { Modal } from "../components/ui/Modal";
import { Select } from "../components/ui/Select";
import { EmptyState } from "../components/ui/EmptyState";
import { PageShell } from "../components/ui/PageShell";
import { ServerRow } from "../components/servers/ServerRow";
import { InfoRow } from "../components/servers/InfoRow";
import { GroupHeader } from "../components/servers/GroupHeader";
import { QuotaBlock } from "../components/subscriptions/QuotaBlock";

const listVariants = {
  hidden: {},
  show: { transition: { staggerChildren: 0.04 } },
};

/** Row keys are per *group*: a favourited server is rendered twice. */
const rowKey = (groupKey: string, serverId: string) => `${groupKey}::${serverId}`;

interface Group {
  key: string;
  title: string;
  content: GroupContent;
  /** subscription groups can be moved; the two synthetic ones cannot */
  subIndex: number | null;
  icon?: React.ReactNode;
  quota?: SubscriptionQuota | null;
}

function AddServersModal({
  open,
  onClose,
}: {
  open: boolean;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const [text, setText] = useState("");
  const [importing, setImporting] = useState(false);
  const refresh = useServers((s) => s.refresh);
  const pushToast = useUi((s) => s.toast);

  const parsedCount = useMemo(
    () => text.split(/\s+/).filter((line) => line.startsWith("vless://")).length,
    [text],
  );

  const pasteFromClipboard = async () => {
    try {
      const clip = await navigator.clipboard.readText();
      if (clip) setText((prev) => (prev ? `${prev}\n${clip}` : clip));
    } catch {
      // clipboard permission denied — user can paste manually
    }
  };

  const doImport = async () => {
    setImporting(true);
    try {
      const result = await ipc("import_share_links", { text });
      await refresh();
      pushToast({
        kind: result.added > 0 ? "success" : "error",
        title:
          result.added > 0
            ? t("servers.addModal.imported", { count: result.added })
            : t("servers.addModal.invalid"),
        message: result.errors[0],
      });
      if (result.added > 0) {
        setText("");
        onClose();
      }
    } catch (e) {
      toastError(e);
    } finally {
      setImporting(false);
    }
  };

  return (
    <Modal open={open} onClose={onClose} title={t("servers.addModal.title")}>
      <textarea
        value={text}
        onChange={(e) => setText(e.target.value)}
        placeholder={t("servers.addModal.pasteHint")}
        spellCheck={false}
        className={cn(
          "h-40 w-full resize-none rounded-(--radius-ctl) border border-glass-border bg-glass p-3 font-mono text-xs text-text select-text",
          "outline-none placeholder:text-text-faint",
          "transition-[border-color,box-shadow] duration-150 focus:border-accent/60 focus:ring-2 focus:ring-accent/40",
        )}
      />
      <div className="mt-2 flex items-center justify-between">
        <span
          className={cn("text-xs", parsedCount > 0 ? "text-ok" : "text-text-faint")}
        >
          {parsedCount > 0
            ? t("servers.addModal.importN", { count: parsedCount })
            : t("servers.addModal.invalid")}
        </span>
        <Button
          variant="ghost"
          icon={<ClipboardPaste size={14} />}
          onClick={() => void pasteFromClipboard()}
        >
          {t("servers.addModal.fromClipboard")}
        </Button>
      </div>
      <div className="mt-5 flex justify-end gap-2">
        <Button variant="ghost" onClick={onClose}>
          {t("common.cancel")}
        </Button>
        <Button
          disabled={parsedCount === 0}
          loading={importing}
          onClick={() => void doImport()}
        >
          {t("common.add")}
        </Button>
      </div>
    </Modal>
  );
}

export function Servers() {
  const { t } = useTranslation();
  const list = useServers((s) => s.list);
  const loaded = useServers((s) => s.loaded);
  const pendingPings = useServers((s) => s.pendingPings);
  const ping = useServers((s) => s.ping);
  const refresh = useServers((s) => s.refresh);
  const removeServerLocal = useServers((s) => s.removeServerLocal);
  const toggleFavorite = useServers((s) => s.toggleFavorite);
  const moveSubscription = useServers((s) => s.moveSubscription);
  const reveal = useServers((s) => s.reveal);
  const clearReveal = useServers((s) => s.clearReveal);
  const settings = useSettings((s) => s.settings);
  const patchSettings = useSettings((s) => s.patch);
  const setLocalSettings = useSettings((s) => s.setLocal);
  const conn = useConnection((s) => s.conn);
  const pushToast = useUi((s) => s.toast);

  const [query, setQuery] = useState("");
  const [addOpen, setAddOpen] = useState(false);
  const [deleteTarget, setDeleteTarget] = useState<ServerEntry | null>(null);
  const [highlight, setHighlight] = useState<string | null>(null);
  const [pendingReveal, setPendingReveal] = useState<{
    key: string;
    nonce: number;
  } | null>(null);
  const rowRefs = useRef(new Map<string, HTMLDivElement>());

  const selectedServerId = settings?.selectedServerId ?? null;
  const sort: ServerSort = settings?.serverSort ?? "default";
  const collapsed = useMemo(
    () => new Set(settings?.collapsedGroups ?? []),
    [settings?.collapsedGroups],
  );

  // The cumulative per-server counters only reach the profile store every few
  // seconds; re-reading it on entry is what makes them look live here.
  useEffect(() => {
    void refresh().catch(toastError);
  }, [refresh]);

  const all = allServers(list);
  // Panel notices are not servers: they must not inflate the header count, and
  // "test all" must not spend a ping on 0.0.0.0.
  const connectable = all.filter((s) => !isInfoEntry(s));
  const q = query.trim().toLowerCase();
  const matches = (s: ServerEntry) =>
    q === "" ||
    s.name.toLowerCase().includes(q) ||
    `${s.server}:${s.port}`.toLowerCase().includes(q);

  const groups: Group[] = useMemo(() => {
    const out: Group[] = [];
    const favorites = all.filter((s) => s.favorite && matches(s));
    if (favorites.length > 0) {
      out.push({
        key: GROUP_FAVORITES,
        title: t("servers.favoritesGroup"),
        // A favourite is by definition something you connect to, so nothing in
        // here is ever an info row.
        content: { info: [], servers: splitGroup(favorites, sort).servers },
        subIndex: null,
        icon: (
          <Star size={12} className="shrink-0 text-warn" fill="currentColor" />
        ),
      });
    }
    const manual = list.manual.filter(matches);
    if (manual.length > 0 || q === "") {
      out.push({
        key: GROUP_MANUAL,
        title: t("servers.manualGroup"),
        content: splitGroup(manual, sort),
        subIndex: null,
      });
    }
    list.subscriptions.forEach((sub, i) => {
      const entries = sub.servers.filter(matches);
      if (q !== "" && entries.length === 0) return;
      out.push({
        key: sub.id,
        title: sub.name,
        content: splitGroup(entries, sort),
        subIndex: i,
        quota: sub.quota,
      });
    });
    return out;
    // `all`/`matches` are derived from list+query on every render, so listing
    // the sources they close over is what actually keeps this correct.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [list, q, sort, t]);

  const subCount = list.subscriptions.length;
  // Only hand-added servers can be deleted — `remove_server` does not touch
  // subscription entries, so offering it there was an error waiting to happen.
  const manualIds = useMemo(
    () => new Set(list.manual.map((s) => s.id)),
    [list.manual],
  );

  const toggleGroup = (key: string) => {
    const next = collapsed.has(key)
      ? [...collapsed].filter((k) => k !== key)
      : [...collapsed, key];
    void patchSettings({ collapsedGroups: next });
  };

  /**
   * Scroll a server into view and flash it. Favourites win: that is the copy
   * the user was looking at when they starred it.
   *
   * The scroll itself happens in an effect once the row exists — expanding a
   * collapsed group and clearing a filter both take a render first.
   */
  const requestReveal = (serverId: string): boolean => {
    // Groups are searched in render order, and favourites render first, so the
    // starred copy is found before the one in the subscription it came from.
    const group = groups.find((g) =>
      g.content.servers.some((s) => s.id === serverId),
    );
    if (!group) return false;
    // A filter that hides the target would make the reveal silently do nothing.
    setQuery("");
    if (collapsed.has(group.key)) {
      void patchSettings({
        collapsedGroups: [...collapsed].filter((k) => k !== group.key),
      });
    }
    setPendingReveal({
      key: rowKey(group.key, serverId),
      nonce: Date.now(),
    });
    return true;
  };

  // Arriving from the dashboard pill (or the sidebar): magnet to whatever the
  // tunnel is running against, or to the selection waiting to be connected.
  const activeId = conn.serverId ?? selectedServerId;
  const revealedOnMount = useRef(false);
  useEffect(() => {
    if (revealedOnMount.current || !loaded || !activeId) return;
    // Only counts as done once the row was actually found: on a cold start the
    // list can arrive a render after `loaded` flips.
    revealedOnMount.current = requestReveal(activeId);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [loaded, activeId, groups]);

  // An explicit request from elsewhere in the app (store-driven, nonce'd so a
  // repeat click on the same server scrolls again).
  useEffect(() => {
    if (!reveal) return;
    requestReveal(reveal.serverId);
    clearReveal();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [reveal]);

  useEffect(() => {
    if (!pendingReveal) return;
    const el = rowRefs.current.get(pendingReveal.key);
    if (!el) return; // group still collapsed / list still rendering
    el.scrollIntoView({ block: "nearest", behavior: "smooth" });
    setHighlight(pendingReveal.key);
    setPendingReveal(null);
  }, [pendingReveal, groups, collapsed]);

  useEffect(() => {
    if (!highlight) return;
    const timer = setTimeout(() => setHighlight(null), 2200);
    return () => clearTimeout(timer);
  }, [highlight]);

  const select = (server: ServerEntry) => {
    void ipc("select_server", { id: server.id }).catch(toastError);
    setLocalSettings({ selectedServerId: server.id });
    if (conn.status === "connected") {
      pushToast({
        kind: "info",
        title: t("servers.selectedToast"),
        message: server.name,
        action: {
          label: t("servers.reconnectToApply"),
          onClick: () =>
            void ipc("connect", { serverId: server.id }).catch(toastError),
        },
      });
    }
  };

  const copyLink = (server: ServerEntry) => {
    void navigator.clipboard
      .writeText(server.raw)
      .then(() => {
        pushToast({ kind: "success", title: t("servers.copied") });
      })
      .catch(toastError);
  };

  const confirmDelete = () => {
    if (!deleteTarget) return;
    void ipc("remove_server", { id: deleteTarget.id }).catch(toastError);
    removeServerLocal(deleteTarget.id);
    setDeleteTarget(null);
  };

  const renderGroup = (group: Group) => {
    const { info, servers } = group.content;
    return (
      <motion.div
        variants={listVariants}
        initial="hidden"
        animate="show"
        className="flex flex-col gap-2"
      >
        {/* Panel notices first: they are announcements about the whole group
            (days left, "press update"), not entries you scroll past. */}
        {info.map((entry) => (
          <InfoRow key={entry.id} entry={entry} />
        ))}
        {servers.map((server) => {
          const key = rowKey(group.key, server.id);
          return (
            <ServerRow
              key={key}
              server={server}
              selected={server.id === selectedServerId}
              testing={pendingPings.has(server.id)}
              deletable={manualIds.has(server.id)}
              highlighted={highlight === key}
              innerRef={(el) => {
                if (el) rowRefs.current.set(key, el);
                else rowRefs.current.delete(key);
              }}
              onSelect={() => select(server)}
              onPing={() => void ping([server.id])}
              onCopy={() => copyLink(server)}
              onDelete={() => setDeleteTarget(server)}
              onToggleFavorite={() => toggleFavorite(server.id)}
            />
          );
        })}
      </motion.div>
    );
  };

  return (
    <PageShell width="list" className="gap-5">
      <div className="flex flex-wrap items-center gap-3">
        <div className="mr-auto min-w-0">
          <h1 className="font-display text-[clamp(1.05rem,1.5vw,1.35rem)] font-extrabold text-text">
            {t("servers.title")}
          </h1>
          <div className="mt-0.5 text-xs text-text-faint">
            {t("servers.count", { count: connectable.length })}
          </div>
        </div>
        <Select<ServerSort>
          value={sort}
          onChange={(v) => void patchSettings({ serverSort: v })}
          options={[
            { value: "default", label: t("servers.sort.default") },
            { value: "ping", label: t("servers.sort.ping") },
            { value: "name", label: t("servers.sort.name") },
          ]}
          className="w-40"
        />
        <Button
          variant="ghost"
          icon={<Zap size={14} />}
          disabled={connectable.length === 0}
          onClick={() => void ping(connectable.map((s) => s.id))}
        >
          {t("servers.testAll")}
        </Button>
        <Button icon={<Plus size={15} />} onClick={() => setAddOpen(true)}>
          {t("servers.add")}
        </Button>
      </div>

      <TextField
        icon={<Search size={14} />}
        value={query}
        onChange={(e) => setQuery(e.target.value)}
        placeholder={t("servers.search")}
      />

      {all.length === 0 ? (
        <GlassCard>
          <EmptyState
            icon={<ServerOff size={24} />}
            title={t("servers.empty.title")}
            hint={t("servers.empty.hint")}
            action={
              <Button icon={<Plus size={15} />} onClick={() => setAddOpen(true)}>
                {t("servers.add")}
              </Button>
            }
          />
        </GlassCard>
      ) : (
        <div className="flex flex-col gap-4">
          {groups.map((group) => {
            const isCollapsed = collapsed.has(group.key);
            return (
              <section key={group.key}>
                <GroupHeader
                  title={group.title}
                  count={group.content.servers.length}
                  collapsed={isCollapsed}
                  onToggle={() => toggleGroup(group.key)}
                  icon={group.icon}
                  meta={
                    group.quota ? (
                      <QuotaBlock quota={group.quota} compact />
                    ) : undefined
                  }
                  onMove={
                    group.subIndex === null
                      ? undefined
                      : (delta) => moveSubscription(group.key, delta)
                  }
                  canMoveUp={group.subIndex !== null && group.subIndex > 0}
                  canMoveDown={
                    group.subIndex !== null && group.subIndex < subCount - 1
                  }
                />
                <AnimatePresence initial={false}>
                  {!isCollapsed && (
                    <motion.div
                      initial={{ opacity: 0 }}
                      animate={{ opacity: 1 }}
                      exit={{ opacity: 0 }}
                      transition={{ duration: 0.12 }}
                    >
                      {renderGroup(group)}
                    </motion.div>
                  )}
                </AnimatePresence>
              </section>
            );
          })}
        </div>
      )}

      <AddServersModal open={addOpen} onClose={() => setAddOpen(false)} />

      <Modal
        open={deleteTarget !== null}
        onClose={() => setDeleteTarget(null)}
        title={t("servers.deleteConfirm.title")}
      >
        <p className="text-[13px] text-text-dim">
          {t("servers.deleteConfirm.body", { name: deleteTarget?.name ?? "" })}
        </p>
        <div className="mt-5 flex justify-end gap-2">
          <Button variant="ghost" onClick={() => setDeleteTarget(null)}>
            {t("common.cancel")}
          </Button>
          <Button variant="danger" onClick={confirmDelete}>
            {t("servers.deleteConfirm.confirm")}
          </Button>
        </div>
      </Modal>
    </PageShell>
  );
}
