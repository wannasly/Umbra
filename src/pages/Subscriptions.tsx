import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  ExternalLink,
  LifeBuoy,
  Pencil,
  Plus,
  RefreshCw,
  Rss,
  Trash2,
} from "lucide-react";
import { ipc, type Subscription } from "../lib/ipc";
import { useServers } from "../stores/servers";
import { useUi, toastError } from "../stores/ui";
import { midEllipsis, relativeTime } from "../lib/format";
import { isInfoEntry } from "../lib/serverMeta";
import { openExternal } from "../lib/open";
import { cn } from "../lib/cn";
import { GlassCard } from "../components/ui/GlassCard";
import { Button } from "../components/ui/Button";
import { IconButton } from "../components/ui/IconButton";
import { Select } from "../components/ui/Select";
import { TextField } from "../components/ui/TextField";
import { Modal } from "../components/ui/Modal";
import { EmptyState } from "../components/ui/EmptyState";
import { PageShell } from "../components/ui/PageShell";
import { QuotaBlock } from "../components/subscriptions/QuotaBlock";

/** Off, the panel's own cadence, and a few sane manual choices. */
function autoUpdateOptions(current: number, label: (h: number) => string) {
  const hours = [...new Set([0, 6, 12, 24, current])].sort((a, b) => a - b);
  return hours.map((h) => ({ value: String(h), label: label(h) }));
}

function SubscriptionCard({
  sub,
  onDelete,
  onRename,
}: {
  sub: Subscription;
  onDelete: () => void;
  onRename: () => void;
}) {
  const { t } = useTranslation();
  const setAutoUpdateHours = useServers((s) => s.setAutoUpdateHours);
  const refresh = useServers((s) => s.refresh);
  const [updating, setUpdating] = useState(false);

  const updateNow = async () => {
    setUpdating(true);
    try {
      await ipc("update_subscription", { id: sub.id });
      await refresh();
    } catch (e) {
      toastError(e);
    } finally {
      setUpdating(false);
    }
  };

  return (
    <GlassCard className="p-4">
      {/* flex-wrap: the action cluster can't shrink (select + button + icon),
          so on a narrow column it drops to its own line instead of squeezing
          the name/URL block down to nothing. */}
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0 flex-1 basis-64">
          <div className="flex items-center gap-1.5">
            <span className="truncate text-[14px] font-semibold text-text">
              {sub.name}
            </span>
            <IconButton
              className="h-6 w-6"
              title={t("subscriptions.rename")}
              onClick={onRename}
            >
              <Pencil size={12} />
            </IconButton>
          </div>
          <div
            className="mt-0.5 truncate font-mono text-xs text-text-dim"
            title={sub.url}
          >
            {midEllipsis(sub.url, 46)}
          </div>
          <div className="mt-1.5 flex flex-wrap items-center gap-x-3 gap-y-1 text-[11px] text-text-faint">
            <span>
              {/* the notices the panel injects are not servers you can use */}
              {t("subscriptions.serverCount", {
                count: sub.servers.filter((s) => !isInfoEntry(s)).length,
              })}
            </span>
            <span>
              {sub.updatedAt
                ? t("subscriptions.updatedAgo", {
                    ago: relativeTime(sub.updatedAt),
                  })
                : t("subscriptions.never")}
            </span>
            {/* Straight from the panel's headers — the place to click when the
                subscription itself is the problem. */}
            {sub.webPageUrl && (
              <button
                type="button"
                onClick={() => openExternal(sub.webPageUrl!)}
                className="flex items-center gap-1 text-accent hover:underline"
              >
                <ExternalLink size={11} />
                {t("subscriptions.accountPage")}
              </button>
            )}
            {sub.supportUrl && (
              <button
                type="button"
                onClick={() => openExternal(sub.supportUrl!)}
                className="flex items-center gap-1 text-accent hover:underline"
              >
                <LifeBuoy size={11} />
                {t("subscriptions.support")}
              </button>
            )}
          </div>
        </div>
        <div className="grid w-full shrink-0 grid-cols-[minmax(0,1fr)_auto] items-center gap-2 sm:w-auto sm:grid-cols-[auto_auto_auto]">
          <div className="flex min-w-0 items-center gap-1.5">
            <span className="text-[11px] text-text-faint">
              {t("subscriptions.autoUpdate")}
            </span>
            <Select
              value={String(sub.autoUpdateHours)}
              onChange={(v) => setAutoUpdateHours(sub.id, Number(v))}
              options={autoUpdateOptions(sub.autoUpdateHours, (h) =>
                h === 0
                  ? t("subscriptions.autoOff")
                  : t("subscriptions.autoHours", { count: h }),
              )}
              className="min-w-28 flex-1 sm:w-28 sm:flex-none"
            />
          </div>
          <Button
            variant="ghost"
            onClick={() => void updateNow()}
            disabled={updating}
            icon={
              <RefreshCw size={14} className={cn(updating && "animate-spin")} />
            }
          >
            <span className="max-[520px]:hidden">{t("subscriptions.updateNow")}</span>
          </Button>
          <IconButton
            danger
            className="col-start-2 row-start-1 sm:col-auto sm:row-auto"
            onClick={onDelete}
            title={t("subscriptions.delete")}
          >
            <Trash2 size={15} />
          </IconButton>
        </div>
      </div>
      {sub.quota && <QuotaBlock quota={sub.quota} />}
    </GlassCard>
  );
}

function AddSubscriptionModal({
  open,
  onClose,
}: {
  open: boolean;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const [url, setUrl] = useState("");
  const [name, setName] = useState("");
  const [adding, setAdding] = useState(false);
  const addLocal = useServers((s) => s.addSubscriptionLocal);
  const refresh = useServers((s) => s.refresh);
  const pushToast = useUi((s) => s.toast);

  const add = async () => {
    setAdding(true);
    try {
      const sub = await ipc("add_subscription", {
        url: url.trim(),
        name: name.trim() === "" ? null : name.trim(),
      });
      addLocal(sub);
      await refresh();
      pushToast({
        kind: "success",
        title: t("subscriptions.added"),
        message: sub.name,
      });
      setUrl("");
      setName("");
      onClose();
    } catch (e) {
      toastError(e);
    } finally {
      setAdding(false);
    }
  };

  return (
    <Modal open={open} onClose={onClose} title={t("subscriptions.addModal.title")}>
      <div className="flex flex-col gap-3">
        <label className="flex flex-col gap-1.5">
          <span className="text-xs font-medium text-text-dim">
            {t("subscriptions.addModal.url")}
          </span>
          <TextField
            mono
            value={url}
            onChange={(e) => setUrl(e.target.value)}
            placeholder="https://"
            spellCheck={false}
          />
        </label>
        <label className="flex flex-col gap-1.5">
          <span className="text-xs font-medium text-text-dim">
            {t("subscriptions.addModal.name")}
          </span>
          <TextField
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder={t("subscriptions.addModal.namePlaceholder")}
          />
          <span className="text-[11px] text-text-faint">
            {t("subscriptions.addModal.nameHint")}
          </span>
        </label>
      </div>
      <div className="mt-5 flex justify-end gap-2">
        <Button variant="ghost" onClick={onClose}>
          {t("common.cancel")}
        </Button>
        <Button
          disabled={!/^https?:\/\/.+/.test(url.trim())}
          loading={adding}
          onClick={() => void add()}
        >
          {t("common.add")}
        </Button>
      </div>
    </Modal>
  );
}

function RenameModal({
  target,
  onClose,
}: {
  target: Subscription | null;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const updateLocal = useServers((s) => s.updateSubscriptionLocal);
  const [name, setName] = useState("");
  const [saving, setSaving] = useState(false);

  // Start from the current name each time the dialog opens on a new card.
  useEffect(() => setName(target?.name ?? ""), [target]);

  const save = async () => {
    if (!target) return;
    setSaving(true);
    try {
      const sub = await ipc("rename_subscription", {
        id: target.id,
        name: name.trim(),
      });
      updateLocal(sub);
      onClose();
    } catch (e) {
      toastError(e);
    } finally {
      setSaving(false);
    }
  };

  return (
    <Modal
      open={target !== null}
      onClose={onClose}
      title={t("subscriptions.rename")}
    >
      <TextField
        autoFocus
        value={name}
        onChange={(e) => setName(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && name.trim() !== "") void save();
        }}
        placeholder={t("subscriptions.addModal.namePlaceholder")}
      />
      <div className="mt-5 flex justify-end gap-2">
        <Button variant="ghost" onClick={onClose}>
          {t("common.cancel")}
        </Button>
        <Button
          disabled={name.trim() === ""}
          loading={saving}
          onClick={() => void save()}
        >
          {t("common.save")}
        </Button>
      </div>
    </Modal>
  );
}

export function Subscriptions() {
  const { t } = useTranslation();
  const subscriptions = useServers((s) => s.list.subscriptions);
  const removeLocal = useServers((s) => s.removeSubscriptionLocal);
  const [addOpen, setAddOpen] = useState(false);
  const [deleteTarget, setDeleteTarget] = useState<Subscription | null>(null);
  const [renameTarget, setRenameTarget] = useState<Subscription | null>(null);

  const confirmDelete = () => {
    if (!deleteTarget) return;
    void ipc("remove_subscription", { id: deleteTarget.id }).catch(toastError);
    removeLocal(deleteTarget.id);
    setDeleteTarget(null);
  };

  return (
    <PageShell width="list" className="gap-5">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <h1 className="font-display text-[clamp(1.05rem,1.5vw,1.35rem)] font-extrabold text-text">
          {t("subscriptions.title")}
        </h1>
        <Button icon={<Plus size={15} />} onClick={() => setAddOpen(true)}>
          {t("subscriptions.add")}
        </Button>
      </div>

      {subscriptions.length === 0 ? (
        <GlassCard>
          <EmptyState
            icon={<Rss size={24} />}
            title={t("subscriptions.empty.title")}
            hint={t("subscriptions.empty.hint")}
            action={
              <Button icon={<Plus size={15} />} onClick={() => setAddOpen(true)}>
                {t("subscriptions.add")}
              </Button>
            }
          />
        </GlassCard>
      ) : (
        <div className="flex flex-col gap-3">
          {subscriptions.map((sub) => (
            <SubscriptionCard
              key={sub.id}
              sub={sub}
              onDelete={() => setDeleteTarget(sub)}
              onRename={() => setRenameTarget(sub)}
            />
          ))}
        </div>
      )}

      <AddSubscriptionModal open={addOpen} onClose={() => setAddOpen(false)} />
      <RenameModal
        target={renameTarget}
        onClose={() => setRenameTarget(null)}
      />

      <Modal
        open={deleteTarget !== null}
        onClose={() => setDeleteTarget(null)}
        title={t("subscriptions.deleteConfirm.title")}
      >
        <p className="text-[13px] text-text-dim">
          {t("subscriptions.deleteConfirm.body", {
            name: deleteTarget?.name ?? "",
            count: deleteTarget?.servers.length ?? 0,
          })}
        </p>
        <div className="mt-5 flex justify-end gap-2">
          <Button variant="ghost" onClick={() => setDeleteTarget(null)}>
            {t("common.cancel")}
          </Button>
          <Button variant="danger" onClick={confirmDelete}>
            {t("subscriptions.deleteConfirm.confirm")}
          </Button>
        </div>
      </Modal>
    </PageShell>
  );
}
