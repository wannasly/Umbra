import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  AppWindow,
  ChevronDown,
  ChevronUp,
  FolderOpen,
  Globe,
  Network,
  Pencil,
  Plus,
  RefreshCw,
  RotateCw,
  Search,
  ShieldAlert,
  Trash2,
} from "lucide-react";
import {
  ipc,
  type AppRouteAction,
  type DomainMatcher,
  type ProcessMatcher,
  type RouteRule,
  type RouteTarget,
  type RuleType,
  type RunningProcess,
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
import { Toggle } from "../components/ui/Toggle";

type FilterTab = "all" | RuleType;

function normalizeProcessName(value: string): string {
  const unquoted = value.trim().replace(/^["']|["']$/g, "");
  const pathParts = unquoted.split(/[\\/]/);
  const name = pathParts[pathParts.length - 1]?.trim() ?? "";
  if (!name) return "";
  return name.toLowerCase().endsWith(".exe") ? name : `${name}.exe`;
}

function normalizeProcessPath(value: string): string {
  const unquoted = value.trim().replace(/^["']|["']$/g, "");
  if (!unquoted || !/[\\/]/.test(unquoted)) return "";
  return unquoted.toLowerCase().endsWith(".exe") ? unquoted : "";
}

function normalizeDomain(value: string): string {
  let v = value.trim().toLowerCase();
  v = v.replace(/^[a-zA-Z]+:\/\//, "");
  v = v.split("/")[0] ?? "";
  v = v.split(":")[0] ?? "";
  if (v.startsWith("*.")) v = v.slice(2);
  else if (v.startsWith(".")) v = v.slice(1);
  return v.trim();
}

function isValidIpOrCidr(input: string): boolean {
  const trimmed = input.trim();
  if (!trimmed) return false;

  // IPv4 or IPv4 CIDR
  const ipv4Regex = /^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})(\/(\d{1,2}))?$/;
  const m4 = trimmed.match(ipv4Regex);
  if (m4) {
    const octets = [Number(m4[1]), Number(m4[2]), Number(m4[3]), Number(m4[4])];
    if (octets.some((o) => o < 0 || o > 255)) return false;
    if (m4[6] !== undefined) {
      const prefix = Number(m4[6]);
      if (prefix < 0 || prefix > 32) return false;
    }
    return true;
  }

  // IPv6 or IPv6 CIDR
  const [ipPart, prefixPart] = trimmed.split("/");
  if (prefixPart !== undefined) {
    const prefix = Number(prefixPart);
    if (isNaN(prefix) || prefix < 0 || prefix > 128 || !/^\d+$/.test(prefixPart)) return false;
  }
  const ipv6Regex = /^(([0-9a-fA-F]{1,4}:){7}[0-9a-fA-F]{1,4}|([0-9a-fA-F]{1,4}:){1,7}:|([0-9a-fA-F]{1,4}:){1,6}:[0-9a-fA-F]{1,4}|([0-9a-fA-F]{1,4}:){1,5}(:[0-9a-fA-F]{1,4}){1,2}|([0-9a-fA-F]{1,4}:){1,4}(:[0-9a-fA-F]{1,4}){1,3}|([0-9a-fA-F]{1,4}:){1,3}(:[0-9a-fA-F]{1,4}){1,4}|([0-9a-fA-F]{1,4}:){1,2}(:[0-9a-fA-F]{1,4}){1,5}|[0-9a-fA-F]{1,4}:((:[0-9a-fA-F]{1,4}){1,6})|:((:[0-9a-fA-F]{1,4}){1,7}|:)|fe80:(:[0-9a-fA-F]{0,4}){0,4}%[0-9a-zA-Z]+|::(ffff(:0{1,4})?:)?((25[0-5]|(2[0-4]|1?[0-9])?[0-9])\.){3}(25[0-5]|(2[0-4]|1?[0-9])?[0-9])|([0-9a-fA-F]{1,4}:){1,4}:((25[0-5]|(2[0-4]|1?[0-9])?[0-9])\.){3}(25[0-5]|(2[0-4]|1?[0-9])?[0-9]))$/;
  return ipv6Regex.test(ipPart ?? "");
}

function newRuleId(): string {
  return globalThis.crypto?.randomUUID?.() ?? `route-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`;
}

export function Routing() {
  const { t } = useTranslation();
  const settings = useSettings((s) => s.settings);
  const patch = useSettings((s) => s.patch);
  const conn = useConnection((s) => s.conn);
  const setPage = useUi((s) => s.setPage);

  const fileInput = useRef<HTMLInputElement>(null);

  // Main page state
  const [filterTab, setFilterTab] = useState<FilterTab>("all");
  const [searchQuery, setSearchQuery] = useState("");
  const [dirty, setDirty] = useState(false);
  const [reconnecting, setReconnecting] = useState(false);

  // Add modal state
  const [addOpen, setAddOpen] = useState(false);
  const [editingRuleId, setEditingRuleId] = useState<string | null>(null);
  const [modalTab, setModalTab] = useState<RuleType>("process");
  const [modalAction, setModalAction] = useState<AppRouteAction>("proxy");

  // Process tab state
  const [procValue, setProcValue] = useState("");
  const [processMatcher, setProcessMatcher] = useState<ProcessMatcher>("name");
  const [procDesc, setProcDesc] = useState("");
  const [procSearch, setProcSearch] = useState("");
  const [runningProcesses, setRunningProcesses] = useState<RunningProcess[]>([]);
  const [loadingProcesses, setLoadingProcesses] = useState(false);

  // Domain tab state
  const [domainValue, setDomainValue] = useState("");
  const [domainMatcher, setDomainMatcher] = useState<DomainMatcher>("suffix");
  const [domainDesc, setDomainDesc] = useState("");

  // IP tab state
  const [ipValue, setIpValue] = useState("");
  const [ipDesc, setIpDesc] = useState("");

  useEffect(() => {
    if (conn.status === "disconnected") setDirty(false);
  }, [conn.status]);

  const loadProcesses = async () => {
    setLoadingProcesses(true);
    try {
      const list = await ipc("get_running_processes");
      setRunningProcesses(list);
    } catch {
      // ignore
    } finally {
      setLoadingProcesses(false);
    }
  };

  const openAddModal = () => {
    setEditingRuleId(null);
    setAddOpen(true);
    setModalTab("process");
    setProcessMatcher("name");
    setProcValue("");
    setProcDesc("");
    setProcSearch("");
    setDomainValue("");
    setDomainMatcher("suffix");
    setDomainDesc("");
    setIpValue("");
    setIpDesc("");
    setModalAction("proxy");
    void loadProcesses();
  };

  const openEditModal = (rule: RouteRule) => {
    setEditingRuleId(rule.id);
    setAddOpen(true);
    setModalTab(rule.ruleType);
    setModalAction(rule.action);
    setProcValue(rule.ruleType === "process" ? rule.value : "");
    setProcessMatcher(
      rule.processMatcher ?? (/[\\/]/.test(rule.value) ? "path" : "name"),
    );
    setProcDesc(rule.ruleType === "process" ? (rule.description ?? "") : "");
    setProcSearch("");
    setDomainValue(rule.ruleType === "domain" ? rule.value : "");
    setDomainMatcher(rule.domainMatcher ?? "suffix");
    setDomainDesc(rule.ruleType === "domain" ? (rule.description ?? "") : "");
    setIpValue(rule.ruleType === "ip_cidr" ? rule.value : "");
    setIpDesc(rule.ruleType === "ip_cidr" ? (rule.description ?? "") : "");
    if (rule.ruleType === "process") void loadProcesses();
  };

  if (!settings) {
    return (
      <div className="flex h-full items-center justify-center">
        <Spinner size={22} className="text-text-faint" />
      </div>
    );
  }

  const rules = settings.routingRules ?? [];

  const counts = {
    all: rules.length,
    process: rules.filter((r) => r.ruleType === "process").length,
    domain: rules.filter((r) => r.ruleType === "domain").length,
    ip_cidr: rules.filter((r) => r.ruleType === "ip_cidr").length,
  };

  const actionOptions: { value: AppRouteAction; label: string }[] = [
    { value: "proxy", label: t("routing.actions.proxy") },
    { value: "direct", label: t("routing.actions.direct") },
    { value: "block", label: t("routing.actions.block") },
  ];

  const saveRules = (nextRules: RouteRule[]) => {
    if (conn.status === "connected") setDirty(true);
    const appRoutes = nextRules
      .filter(
        (r) =>
          r.enabled &&
          r.ruleType === "process" &&
          (r.processMatcher ?? (/[\\/]/.test(r.value) ? "path" : "name")) === "name",
      )
      .map((r) => ({ id: r.id, processName: r.value, action: r.action }));
    void patch({ routingRules: nextRules, appRoutes });
  };

  const setDefault = (routeDefault: RouteTarget) => {
    if (conn.status === "connected") setDirty(true);
    void patch({ routeDefault });
  };

  const toggleRule = (id: string, enabled: boolean) => {
    saveRules(rules.map((r) => (r.id === id ? { ...r, enabled } : r)));
  };

  const updateRuleAction = (id: string, action: AppRouteAction) => {
    saveRules(rules.map((r) => (r.id === id ? { ...r, action } : r)));
  };

  const deleteRule = (id: string) => {
    saveRules(rules.filter((r) => r.id !== id));
  };

  const moveRule = (id: string, direction: "up" | "down") => {
    const idx = rules.findIndex((r) => r.id === id);
    if (idx < 0) return;
    const targetIdx = direction === "up" ? idx - 1 : idx + 1;
    if (targetIdx < 0 || targetIdx >= rules.length) return;

    const next = [...rules];
    const temp = next[idx]!;
    next[idx] = next[targetIdx]!;
    next[targetIdx] = temp;
    saveRules(next);
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

  // Filtered rules for display
  const filteredRules = useMemo(() => {
    return rules.filter((rule) => {
      if (filterTab !== "all" && rule.ruleType !== filterTab) return false;
      if (searchQuery.trim()) {
        const query = searchQuery.toLowerCase().trim();
        const matchesVal = rule.value.toLowerCase().includes(query);
        const matchesDesc = (rule.description ?? "").toLowerCase().includes(query);
        if (!matchesVal && !matchesDesc) return false;
      }
      return true;
    });
  }, [rules, filterTab, searchQuery]);

  // Running processes in modal filtered by search
  const filteredRunningProcesses = useMemo(() => {
    const q = procSearch.toLowerCase().trim();
    if (!q) return runningProcesses;
    return runningProcesses.filter(
      (p) =>
        p.name.toLowerCase().includes(q) ||
        (p.title ?? "").toLowerCase().includes(q) ||
        (p.path ?? "").toLowerCase().includes(q),
    );
  }, [runningProcesses, procSearch]);

  const canSubmitModal = () => {
    if (modalTab === "process") {
      return Boolean(
        processMatcher === "path"
          ? normalizeProcessPath(procValue)
          : normalizeProcessName(procValue),
      );
    }
    if (modalTab === "domain") {
      return Boolean(
        domainMatcher === "keyword" || domainMatcher === "regex"
          ? domainValue.trim()
          : normalizeDomain(domainValue),
      );
    }
    if (modalTab === "ip_cidr") {
      return isValidIpOrCidr(ipValue);
    }
    return false;
  };

  const submitModal = () => {
    let newRule: RouteRule | null = null;

    if (modalTab === "process") {
      const val =
        processMatcher === "path"
          ? normalizeProcessPath(procValue)
          : normalizeProcessName(procValue);
      if (!val) return;
      newRule = {
        id: editingRuleId ?? newRuleId(),
        enabled: true,
        ruleType: "process",
        value: val,
        processMatcher,
        action: modalAction,
        description: procDesc.trim() || undefined,
      };
    } else if (modalTab === "domain") {
      const val =
        domainMatcher === "keyword" || domainMatcher === "regex"
          ? domainValue.trim()
          : normalizeDomain(domainValue);
      if (!val) return;
      newRule = {
        id: editingRuleId ?? newRuleId(),
        enabled: true,
        ruleType: "domain",
        value: val,
        domainMatcher,
        action: modalAction,
        description: domainDesc.trim() || undefined,
      };
    } else if (modalTab === "ip_cidr") {
      const val = ipValue.trim();
      if (!isValidIpOrCidr(val)) return;
      newRule = {
        id: editingRuleId ?? newRuleId(),
        enabled: true,
        ruleType: "ip_cidr",
        value: val,
        action: modalAction,
        description: ipDesc.trim() || undefined,
      };
    }

    if (newRule) {
      const previous = editingRuleId
        ? rules.find((rule) => rule.id === editingRuleId)
        : undefined;
      if (previous) newRule.enabled = previous.enabled;
      saveRules(
        editingRuleId
          ? rules.map((rule) => (rule.id === editingRuleId ? newRule! : rule))
          : [...rules, newRule],
      );
      setAddOpen(false);
    }
  };

  const getTypeBadgeLabel = (rule: RouteRule) => {
    if (rule.ruleType === "process") {
      return t(
        (rule.processMatcher ?? (/[\\/]/.test(rule.value) ? "path" : "name")) === "path"
          ? "routing.typeBadges.processPath"
          : "routing.typeBadges.processName",
      );
    }
    if (rule.ruleType === "ip_cidr") return t("routing.typeBadges.ipCidr");
    if (rule.domainMatcher === "exact") return t("routing.typeBadges.domainExact");
    if (rule.domainMatcher === "keyword") return t("routing.typeBadges.domainKeyword");
    if (rule.domainMatcher === "regex") return t("routing.typeBadges.domainRegex");
    return t("routing.typeBadges.domainSuffix");
  };

  return (
    <PageShell width="list" className="gap-5">
      {/* Header */}
      <div className="flex flex-wrap items-center gap-3">
        <div className="mr-auto min-w-0">
          <h1 className="font-display text-[clamp(1.05rem,1.5vw,1.35rem)] font-extrabold text-text">
            {t("routing.title")}
          </h1>
          <div className="mt-0.5 text-xs text-text-faint">
            {t("routing.ruleCount", { count: rules.length })}
          </div>
        </div>
        <Button icon={<Plus size={15} />} onClick={openAddModal}>
          {t("routing.add")}
        </Button>
      </div>

      {/* Default Routing Card */}
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

      {/* TUN Notice */}
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

      {/* Reconnect notice */}
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

      {/* Filter Tabs & Search Bar */}
      <div className="flex flex-wrap items-center gap-3">
        <SegmentedControl<FilterTab>
          id="routing-filter"
          value={filterTab}
          options={[
            { value: "all", label: `${t("routing.types.all")} (${counts.all})` },
            { value: "process", label: `${t("routing.types.process")} (${counts.process})` },
            { value: "domain", label: `${t("routing.types.domain")} (${counts.domain})` },
            { value: "ip_cidr", label: `${t("routing.types.ip_cidr")} (${counts.ip_cidr})` },
          ]}
          onChange={setFilterTab}
          className="w-full sm:w-auto"
        />
        <div className="min-w-48 flex-1">
          <TextField
            icon={<Search size={14} />}
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            placeholder={t("routing.search")}
            className="h-9"
          />
        </div>
      </div>

      {/* Rules List */}
      {rules.length === 0 ? (
        <GlassCard>
          <EmptyState
            icon={<Network size={24} />}
            title={t("routing.empty.title")}
            hint={t("routing.empty.hint")}
            action={
              <Button icon={<Plus size={15} />} onClick={openAddModal}>
                {t("routing.add")}
              </Button>
            }
          />
        </GlassCard>
      ) : filteredRules.length === 0 ? (
        <GlassCard>
          <EmptyState
            icon={<Search size={24} />}
            title={t("routing.emptyFiltered.title")}
            hint={t("routing.emptyFiltered.hint")}
          />
        </GlassCard>
      ) : (
        <div className="flex flex-col gap-2">
          {filteredRules.map((rule) => {
            const originalIndex = rules.findIndex((r) => r.id === rule.id);
            const isFirst = originalIndex === 0;
            const isLast = originalIndex === rules.length - 1;

            return (
              <GlassCard
                key={rule.id}
                className={cn(
                  "grid grid-cols-[auto_auto_minmax(0,1fr)_auto_auto_auto_auto] items-center gap-3 px-4 py-3 transition-opacity",
                  "max-[640px]:grid-cols-[auto_auto_minmax(0,1fr)_auto_auto]",
                  !rule.enabled && "opacity-55 saturate-70",
                )}
              >
                {/* Enable / Disable toggle */}
                <Toggle
                  checked={rule.enabled}
                  onChange={(checked) => toggleRule(rule.id, checked)}
                  aria-label="Toggle rule"
                />

                {/* Icon based on rule type */}
                <div
                  className={cn(
                    "flex h-9 w-9 shrink-0 items-center justify-center rounded-(--radius-ctl) border",
                    rule.action === "proxy" && "border-accent/25 bg-accent/10 text-accent-2",
                    rule.action === "direct" && "border-ok/25 bg-ok/10 text-ok",
                    rule.action === "block" && "border-danger/25 bg-danger/10 text-danger",
                  )}
                >
                  {rule.ruleType === "process" && <AppWindow size={17} />}
                  {rule.ruleType === "domain" && <Globe size={17} />}
                  {rule.ruleType === "ip_cidr" && <Network size={17} />}
                </div>

                {/* Rule info */}
                <div className="min-w-0">
                  <div className="flex flex-wrap items-center gap-2">
                    <span className="truncate font-mono text-[13px] font-semibold text-text">
                      {rule.ruleType === "domain" && rule.domainMatcher === "suffix" && !rule.value.startsWith("*.")
                        ? `*.${rule.value}`
                        : rule.value}
                    </span>
                    <span className="rounded-[6px] border border-interactive-border bg-surface-2/60 px-1.5 py-0.5 text-[10px] font-medium text-text-dim">
                      {getTypeBadgeLabel(rule)}
                    </span>
                  </div>
                  <div className="mt-0.5 truncate text-[11px] text-text-faint">
                    {rule.description || t(`routing.actionHints.${rule.action}`)}
                  </div>
                </div>

                {/* Action select */}
                <Select<AppRouteAction>
                  value={rule.action}
                  options={actionOptions}
                  onChange={(nextAction) => updateRuleAction(rule.id, nextAction)}
                  className="w-32 max-[640px]:col-span-3 max-[640px]:col-start-3 max-[640px]:w-full"
                />

                {/* Priority Reorder Controls */}
                <div className="flex items-center gap-0.5 max-[640px]:col-start-4 max-[640px]:row-start-1">
                  <IconButton
                    title={t("routing.moveUp")}
                    aria-label={t("routing.moveUp")}
                    disabled={isFirst}
                    onClick={() => moveRule(rule.id, "up")}
                    className="h-8 w-8"
                  >
                    <ChevronUp size={15} />
                  </IconButton>
                  <IconButton
                    title={t("routing.moveDown")}
                    aria-label={t("routing.moveDown")}
                    disabled={isLast}
                    onClick={() => moveRule(rule.id, "down")}
                    className="h-8 w-8"
                  >
                    <ChevronDown size={15} />
                  </IconButton>
                </div>

                <IconButton
                  title={t("routing.edit")}
                  aria-label={t("routing.edit")}
                  className="h-8 w-8 max-[640px]:col-start-5 max-[640px]:row-start-1"
                  onClick={() => openEditModal(rule)}
                >
                  <Pencil size={14} />
                </IconButton>

                {/* Delete button */}
                <IconButton
                  danger
                  title={t("routing.delete")}
                  aria-label={t("routing.delete")}
                  className="h-8 w-8 max-[640px]:col-start-6 max-[640px]:row-start-1"
                  onClick={() => deleteRule(rule.id)}
                >
                  <Trash2 size={15} />
                </IconButton>
              </GlassCard>
            );
          })}
        </div>
      )}

      {/* Add Rule Modal */}
      <Modal
        open={addOpen}
        onClose={() => setAddOpen(false)}
        title={t(editingRuleId ? "routing.addModal.editTitle" : "routing.addModal.title")}
        className="max-w-lg"
      >
        {/* Modal Tabs */}
        <SegmentedControl<RuleType>
          id="modal-rule-type"
          value={modalTab}
          options={[
            { value: "process", label: t("routing.addModal.tabs.process") },
            { value: "domain", label: t("routing.addModal.tabs.domain") },
            { value: "ip_cidr", label: t("routing.addModal.tabs.ip_cidr") },
          ]}
          onChange={setModalTab}
          className="mb-4 w-full"
        />

        {/* Tab 1: Process */}
        {modalTab === "process" && (
          <div className="flex flex-col gap-3.5">
            <SegmentedControl<ProcessMatcher>
              id="process-matcher-type"
              value={processMatcher}
              options={[
                { value: "name", label: t("routing.matchers.processName") },
                { value: "path", label: t("routing.matchers.processPath") },
              ]}
              onChange={(matcher) => {
                setProcessMatcher(matcher);
                setProcValue("");
              }}
              className="w-full"
            />
            {/* Running processes section */}
            <div>
              <div className="mb-1.5 flex items-center justify-between">
                <label className="text-xs font-medium text-text-dim">
                  {t("routing.addModal.process.runningTitle")}
                </label>
                <button
                  type="button"
                  onClick={() => void loadProcesses()}
                  className="flex items-center gap-1 text-[11px] text-accent hover:underline"
                >
                  <RefreshCw size={11} className={cn(loadingProcesses && "animate-spin")} />
                  {t("routing.addModal.process.refreshRunning")}
                </button>
              </div>

              <TextField
                icon={<Search size={13} />}
                value={procSearch}
                onChange={(e) => setProcSearch(e.target.value)}
                placeholder={t("routing.addModal.process.searchRunning")}
                className="h-8 text-xs"
              />

              <div className="mt-2 max-h-36 overflow-y-auto rounded-(--radius-ctl) border border-interactive-border bg-surface-2/40 p-1">
                {loadingProcesses ? (
                  <div className="flex h-20 items-center justify-center">
                    <Spinner size={16} className="text-text-faint" />
                  </div>
                ) : filteredRunningProcesses.length === 0 ? (
                  <div className="py-4 text-center text-xs text-text-faint">
                    {t("routing.addModal.process.noRunning")}
                  </div>
                ) : (
                  <div className="flex flex-col gap-0.5">
                    {filteredRunningProcesses.map((p) => {
                      const candidate = processMatcher === "path" ? p.path : p.name;
                      const isSelected =
                        Boolean(candidate) &&
                        procValue.toLowerCase() === candidate!.toLowerCase();
                      return (
                        <button
                          key={`${p.pid}-${p.name}`}
                          type="button"
                          disabled={!candidate}
                          onClick={() => {
                            if (!candidate) return;
                            setProcValue(candidate);
                            if (p.title) setProcDesc(p.title);
                          }}
                          className={cn(
                            "flex items-center justify-between gap-2 rounded-[7px] px-2.5 py-1.5 text-left text-xs outline-none transition-colors",
                            isSelected
                              ? "border border-accent/40 bg-accent/15 text-text"
                              : "text-text-dim hover:bg-hover-surface hover:text-text disabled:cursor-not-allowed disabled:opacity-40",
                          )}
                          title={p.path ?? undefined}
                        >
                          <div className="min-w-0 flex-1">
                            <span className="font-mono font-semibold text-text">
                              {p.name}
                            </span>
                            {p.title && (
                              <span className="ml-2 truncate text-[11px] text-text-faint">
                                {p.title}
                              </span>
                            )}
                          </div>
                          <span className="rounded bg-surface-3 px-1.5 py-0.5 text-[10px] text-text-faint">
                            {t("routing.addModal.process.select")}
                          </span>
                        </button>
                      );
                    })}
                  </div>
                )}
              </div>
            </div>

            {/* Manual input */}
            <div>
              <label className="mb-1.5 block text-xs font-medium text-text-dim">
                  {t(
                    processMatcher === "path"
                      ? "routing.addModal.process.pathLabel"
                      : "routing.addModal.process.manualLabel",
                  )}
              </label>
              <div className="flex gap-2">
                <TextField
                  mono
                  value={procValue}
                  onChange={(e) => setProcValue(e.target.value)}
                  placeholder={t(
                    processMatcher === "path"
                      ? "routing.addModal.process.pathPlaceholder"
                      : "routing.addModal.process.manualPlaceholder",
                  )}
                  spellCheck={false}
                  wrapClassName="min-w-0 flex-1"
                />
                {processMatcher === "name" && <input
                  ref={fileInput}
                  type="file"
                  accept=".exe,application/x-msdownload"
                  className="hidden"
                  onChange={(e) => {
                    const file = e.target.files?.[0];
                    if (file) setProcValue(file.name);
                    e.currentTarget.value = "";
                  }}
                />}
                {processMatcher === "name" && <IconButton
                  title={t("routing.addModal.process.chooseExe")}
                  aria-label={t("routing.addModal.process.chooseExe")}
                  className="h-9 w-9 border border-glass-border bg-glass"
                  onClick={() => fileInput.current?.click()}
                >
                  <FolderOpen size={15} />
                </IconButton>}
              </div>
            </div>

            {/* Description */}
            <div>
              <label className="mb-1.5 block text-xs font-medium text-text-dim">
                {t("routing.addModal.process.descriptionLabel")}
              </label>
              <TextField
                value={procDesc}
                onChange={(e) => setProcDesc(e.target.value)}
                placeholder={t("routing.addModal.process.descriptionPlaceholder")}
              />
            </div>
          </div>
        )}

        {/* Tab 2: Domain */}
        {modalTab === "domain" && (
          <div className="flex flex-col gap-3.5">
            <div>
              <label className="mb-1.5 block text-xs font-medium text-text-dim">
                {t("routing.addModal.domain.label")}
              </label>
              <TextField
                mono
                autoFocus
                value={domainValue}
                onChange={(e) => setDomainValue(e.target.value)}
                placeholder={t("routing.addModal.domain.placeholder")}
                spellCheck={false}
              />
            </div>

            <div>
              <label className="mb-1.5 block text-xs font-medium text-text-dim">
                {t("routing.addModal.domain.matcherLabel")}
              </label>
              <SegmentedControl<DomainMatcher>
                id="domain-matcher-type"
                value={domainMatcher}
                options={[
                  { value: "suffix", label: t("routing.matchers.suffix") },
                  { value: "exact", label: t("routing.matchers.exact") },
                  { value: "keyword", label: t("routing.matchers.keyword") },
                  { value: "regex", label: t("routing.matchers.regex") },
                ]}
                onChange={setDomainMatcher}
                className="w-full"
              />
              <p className="mt-1.5 text-[11px] leading-relaxed text-text-faint">
                {t(`routing.matcherHints.${domainMatcher}`)}
              </p>
            </div>

            <div>
              <label className="mb-1.5 block text-xs font-medium text-text-dim">
                {t("routing.addModal.domain.descriptionLabel")}
              </label>
              <TextField
                value={domainDesc}
                onChange={(e) => setDomainDesc(e.target.value)}
                placeholder={t("routing.addModal.domain.descriptionPlaceholder")}
              />
            </div>
          </div>
        )}

        {/* Tab 3: IP / CIDR */}
        {modalTab === "ip_cidr" && (
          <div className="flex flex-col gap-3.5">
            <div>
              <label className="mb-1.5 block text-xs font-medium text-text-dim">
                {t("routing.addModal.ip.label")}
              </label>
              <TextField
                mono
                autoFocus
                value={ipValue}
                onChange={(e) => setIpValue(e.target.value)}
                placeholder={t("routing.addModal.ip.placeholder")}
                spellCheck={false}
              />
              {ipValue.trim() && !isValidIpOrCidr(ipValue) && (
                <p className="mt-1 text-[11px] text-danger">
                  {t("routing.addModal.ip.invalid")}
                </p>
              )}
            </div>

            <div>
              <label className="mb-1.5 block text-xs font-medium text-text-dim">
                {t("routing.addModal.ip.descriptionLabel")}
              </label>
              <TextField
                value={ipDesc}
                onChange={(e) => setIpDesc(e.target.value)}
                placeholder={t("routing.addModal.ip.descriptionPlaceholder")}
              />
            </div>
          </div>
        )}

        {/* Action Select */}
        <div className="mt-4">
          <label className="mb-1.5 block text-xs font-medium text-text-dim">
            {t("routing.addModal.action")}
          </label>
          <Select<AppRouteAction>
            value={modalAction}
            options={actionOptions}
            onChange={setModalAction}
            className="w-full"
          />
        </div>

        {/* Modal Footer */}
        <div className="mt-6 flex justify-end gap-2">
          <Button variant="ghost" onClick={() => setAddOpen(false)}>
            {t("common.cancel")}
          </Button>
          <Button disabled={!canSubmitModal()} onClick={submitModal}>
            {t(editingRuleId ? "common.save" : "common.add")}
          </Button>
        </div>
      </Modal>
    </PageShell>
  );
}
