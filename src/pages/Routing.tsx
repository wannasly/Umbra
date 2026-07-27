import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  AppWindow,
  FolderOpen,
  Plus,
  Route as RouteIcon,
  RotateCw,
  ShieldAlert,
  Trash2,
} from "lucide-react";
import {
  ipc,
  type AppRouteAction,
  type AppRouteRule,
  type RouteTarget,
} from "../lib/ipc";
import { useSettings } from "../stores/settings";
import { useConnection } from "../stores/connection";
import { useUi, handleIpcError } from "../stores/ui";
import { cn } from "../lib/cn";
import { PageShell } from "../components/ui/PageShell";
import { GlassCard } from "../components/ui/GlassCard";
import { SegmentedControl } from "../components/ui/SegmentedControl";
import { Select } from "../components/ui/Select";
import { TextField } from "../components/ui/TextField";
import { Button } from "../components/ui/Button";
import { IconButton } from "../components/ui/IconButton";
import { Modal } from "../components/ui/Modal";
import { EmptyState } from "../components/ui/EmptyState";
import { Spinner } from "../components/ui/Spinner";

function normalizeProcessName(value: string): string {
  const unquoted = value.trim().replace(/^["']|["']$/g, "");
  const pathParts = unquoted.split(/[\\/]/);
  const name = pathParts[pathParts.length - 1]?.trim() ?? "";
  if (!name) return "";
  return name.toLowerCase().endsWith(".exe") ? name : `${name}.exe`;
}

function newRuleId(): string {
  return globalThis.crypto?.randomUUID?.() ?? `route-${Date.now()}`;
}

export function Routing() {
  const { t } = useTranslation();
  const settings = useSettings((s) => s.settings);
  const patch = useSettings((s) => s.patch);
  const conn = useConnection((s) => s.conn);
  const setPage = useUi((s) => s.setPage);
  const fileInput = useRef<HTMLInputElement>(null);
  const [addOpen, setAddOpen] = useState(false);
  const [processName, setProcessName] = useState("");
  const [action, setAction] = useState<AppRouteAction>("proxy");
  const [dirty, setDirty] = useState(false);
  const [reconnecting, setReconnecting] = useState(false);

  useEffect(() => {
    if (conn.status === "disconnected") setDirty(false);
  }, [conn.status]);

  if (!settings) {
    return (
      <div className="flex h-full items-center justify-center">
        <Spinner size={22} className="text-text-faint" />
      </div>
    );
  }

  const actionOptions: { value: AppRouteAction; label: string }[] = [
    { value: "proxy", label: t("routing.actions.proxy") },
    { value: "direct", label: t("routing.actions.direct") },
    { value: "block", label: t("routing.actions.block") },
  ];

  const saveRoutes = (appRoutes: AppRouteRule[]) => {
    if (conn.status === "connected") setDirty(true);
    void patch({ appRoutes });
  };

  const setDefault = (routeDefault: RouteTarget) => {
    if (conn.status === "connected") setDirty(true);
    void patch({ routeDefault });
  };

  const addRule = () => {
    const normalized = normalizeProcessName(processName);
    if (!normalized) return;
    const existing = settings.appRoutes.findIndex(
      (rule) => rule.processName.toLowerCase() === normalized.toLowerCase(),
    );
    const next = [...settings.appRoutes];
    if (existing >= 0) {
      next[existing] = { ...next[existing], processName: normalized, action };
    } else {
      next.push({ id: newRuleId(), processName: normalized, action });
    }
    saveRoutes(next);
    setProcessName("");
    setAction("proxy");
    setAddOpen(false);
  };

  const reconnect = async () => {
    if (!conn.serverId) return;
    setReconnecting(true);
    try {
      await ipc("connect", { serverId: conn.serverId });
      setDirty(false);
    } catch (error) {
      handleIpcError(error);
    } finally {
      setReconnecting(false);
    }
  };

  return (
    <PageShell width="list" className="gap-5">
      <div className="flex flex-wrap items-center gap-3">
        <div className="mr-auto min-w-0">
          <h1 className="font-display text-[clamp(1.05rem,1.5vw,1.35rem)] font-extrabold text-text">
            {t("routing.title")}
          </h1>
          <div className="mt-0.5 text-xs text-text-faint">
            {t("routing.ruleCount", { count: settings.appRoutes.length })}
          </div>
        </div>
        <Button icon={<Plus size={15} />} onClick={() => setAddOpen(true)}>
          {t("routing.add")}
        </Button>
      </div>

      <GlassCard className="flex flex-wrap items-center justify-between gap-4 px-5 py-4">
        <div className="min-w-48 flex-1">
          <div className="text-[13px] font-semibold text-text">
            {t("routing.default.title")}
          </div>
          <div className="mt-0.5 text-xs text-text-faint">
            {t("routing.default.hint")}
          </div>
        </div>
        <SegmentedControl<RouteTarget>
          id="route-default"
          value={settings.routeDefault}
          options={[
            { value: "proxy", label: t("routing.actions.proxy") },
            { value: "direct", label: t("routing.actions.direct") },
          ]}
          onChange={setDefault}
          className="w-full sm:w-auto"
        />
      </GlassCard>

      {settings.mode !== "tun" && (
        <div className="flex flex-wrap items-center gap-3 rounded-(--radius-ctl) border border-warn/25 bg-warn/8 px-4 py-3">
          <ShieldAlert size={16} className="shrink-0 text-warn" />
          <span className="min-w-48 flex-1 text-xs leading-relaxed text-text-dim">
            {t("routing.tunNotice")}
          </span>
          <Button variant="ghost" onClick={() => setPage("dashboard")}>
            {t("routing.openMode")}
          </Button>
        </div>
      )}

      {dirty && conn.status === "connected" && (
        <div className="flex flex-wrap items-center gap-3 rounded-(--radius-ctl) border border-accent/25 bg-accent/8 px-4 py-3">
          <span className="min-w-48 flex-1 text-xs text-text-dim">
            {t("routing.reconnectNotice")}
          </span>
          <Button
            variant="ghost"
            loading={reconnecting}
            icon={<RotateCw size={14} />}
            onClick={() => void reconnect()}
          >
            {t("routing.reconnect")}
          </Button>
        </div>
      )}

      {settings.appRoutes.length === 0 ? (
        <GlassCard>
          <EmptyState
            icon={<RouteIcon size={24} />}
            title={t("routing.empty.title")}
            hint={t("routing.empty.hint")}
            action={
              <Button icon={<Plus size={15} />} onClick={() => setAddOpen(true)}>
                {t("routing.add")}
              </Button>
            }
          />
        </GlassCard>
      ) : (
        <div className="flex flex-col gap-2">
          {settings.appRoutes.map((rule) => (
            <GlassCard
              key={rule.id}
              className="grid grid-cols-[auto_minmax(0,1fr)_auto_auto] items-center gap-3 px-4 py-3 max-[560px]:grid-cols-[auto_minmax(0,1fr)_auto]"
            >
              <div
                className={cn(
                  "flex h-9 w-9 items-center justify-center rounded-(--radius-ctl) border",
                  rule.action === "proxy" &&
                    "border-accent/25 bg-accent/10 text-accent-2",
                  rule.action === "direct" &&
                    "border-ok/25 bg-ok/10 text-ok",
                  rule.action === "block" &&
                    "border-danger/25 bg-danger/10 text-danger",
                )}
              >
                <AppWindow size={17} />
              </div>
              <div className="min-w-0">
                <div className="truncate font-mono text-[13px] font-semibold text-text">
                  {rule.processName}
                </div>
                <div className="mt-0.5 text-[11px] text-text-faint">
                  {t(`routing.actionHints.${rule.action}`)}
                </div>
              </div>
              <Select<AppRouteAction>
                value={rule.action}
                options={actionOptions}
                onChange={(nextAction) =>
                  saveRoutes(
                    settings.appRoutes.map((item) =>
                      item.id === rule.id ? { ...item, action: nextAction } : item,
                    ),
                  )
                }
                className="w-36 max-[560px]:col-span-2 max-[560px]:col-start-2 max-[560px]:w-full"
              />
              <IconButton
                danger
                title={t("routing.delete")}
                aria-label={t("routing.delete")}
                className="max-[560px]:col-start-3 max-[560px]:row-start-1"
                onClick={() =>
                  saveRoutes(settings.appRoutes.filter((item) => item.id !== rule.id))
                }
              >
                <Trash2 size={15} />
              </IconButton>
            </GlassCard>
          ))}
        </div>
      )}

      <Modal
        open={addOpen}
        onClose={() => setAddOpen(false)}
        title={t("routing.addModal.title")}
      >
        <label className="mb-1.5 block text-xs font-medium text-text-dim">
          {t("routing.addModal.process")}
        </label>
        <div className="flex gap-2">
          <TextField
            mono
            autoFocus
            value={processName}
            onChange={(event) => setProcessName(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter") addRule();
            }}
            placeholder={t("routing.addModal.placeholder")}
            spellCheck={false}
            wrapClassName="min-w-0 flex-1"
          />
          <input
            ref={fileInput}
            type="file"
            accept=".exe,application/x-msdownload"
            className="hidden"
            onChange={(event) => {
              const file = event.target.files?.[0];
              if (file) setProcessName(file.name);
              event.currentTarget.value = "";
            }}
          />
          <IconButton
            title={t("routing.addModal.chooseExe")}
            aria-label={t("routing.addModal.chooseExe")}
            className="h-9 w-9 border border-glass-border bg-glass"
            onClick={() => fileInput.current?.click()}
          >
            <FolderOpen size={15} />
          </IconButton>
        </div>

        <label className="mt-4 mb-1.5 block text-xs font-medium text-text-dim">
          {t("routing.addModal.action")}
        </label>
        <Select<AppRouteAction>
          value={action}
          options={actionOptions}
          onChange={setAction}
          className="w-full"
        />

        <div className="mt-5 flex justify-end gap-2">
          <Button variant="ghost" onClick={() => setAddOpen(false)}>
            {t("common.cancel")}
          </Button>
          <Button disabled={!normalizeProcessName(processName)} onClick={addRule}>
            {t("common.add")}
          </Button>
        </div>
      </Modal>
    </PageShell>
  );
}
