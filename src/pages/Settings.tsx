import { useEffect, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { motion } from "motion/react";
import { Copy, FolderOpen } from "lucide-react";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  ipc,
  type Accent,
  type CoreStatus,
  type Settings as AppSettings,
  type UpdateCheck,
} from "../lib/ipc";
import { useSettings } from "../stores/settings";
import { useUi, toastError } from "../stores/ui";
import { formatBytes } from "../lib/format";
import { cn } from "../lib/cn";
import { GlassCard } from "../components/ui/GlassCard";
import { Button } from "../components/ui/Button";
import { IconButton } from "../components/ui/IconButton";
import { Toggle } from "../components/ui/Toggle";
import { Select } from "../components/ui/Select";
import { TextField } from "../components/ui/TextField";
import { Spinner } from "../components/ui/Spinner";
import { PageShell } from "../components/ui/PageShell";
import { APP_VERSION } from "../lib/version";

/**
 * Swatches must mirror the presets in styles/theme.css. The ids are persisted by
 * the backend and cannot change, but what they *look* like is the sea palette —
 * each swatch shows the preset's accent → accent-2 pair, which is what the UI
 * actually renders in gradients. Keep these four in sync with :root[data-accent].
 */
const ACCENTS: { value: Accent; from: string; to: string }[] = [
  {
    value: "violet",
    from: "var(--color-accent-violet-from)",
    to: "var(--color-accent-violet-to)",
  },
  {
    value: "cyan",
    from: "var(--color-accent-cyan-from)",
    to: "var(--color-accent-cyan-to)",
  },
  {
    value: "emerald",
    from: "var(--color-accent-emerald-from)",
    to: "var(--color-accent-emerald-to)",
  },
  {
    value: "amber",
    from: "var(--color-accent-amber-from)",
    to: "var(--color-accent-amber-to)",
  },
];

/** Must stay in sync with Settings::default().sub_user_agent on the Rust side. */
const DEFAULT_SUB_USER_AGENT = `v2rayN/7.13 Umbra/${APP_VERSION}`;

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <GlassCard className="px-5 py-4">
      <h2 className="mb-2 border-b border-glass-border pb-2.5 font-display text-[14px] font-bold tracking-[0.035em] text-accent">
        {title}
      </h2>
      <div>{children}</div>
    </GlassCard>
  );
}

function Row({
  label,
  children,
  sub,
  description,
}: {
  label: string;
  children?: ReactNode;
  sub?: boolean;
  description?: ReactNode;
}) {
  return (
    <div
      className={cn(
        "settings-row flex min-h-12 items-center justify-between gap-4 border-b border-glass-border/70 last:border-b-0",
        sub && "pl-4",
      )}
    >
      <span className="min-w-0">
        <span className="block text-[13px] leading-snug text-text-dim">{label}</span>
        {description && (
          <span className="mt-1 block max-w-xl text-xs leading-relaxed text-text-faint">
            {description}
          </span>
        )}
      </span>
      <span className="settings-row-control flex shrink-0 items-center gap-2">
        {children}
      </span>
    </div>
  );
}

function SubscriptionsSection({ settings }: { settings: AppSettings }) {
  const { t } = useTranslation();
  const patch = useSettings((s) => s.patch);
  const pushToast = useUi((s) => s.toast);
  const [userAgent, setUserAgent] = useState(settings.subUserAgent);

  useEffect(() => {
    setUserAgent(settings.subUserAgent);
  }, [settings.subUserAgent]);

  const commitUserAgent = () => {
    const next = userAgent.trim();
    if (next === settings.subUserAgent) return;
    if (next === "") {
      setUserAgent(settings.subUserAgent);
      return;
    }
    void patch({ subUserAgent: next });
  };

  const resetUserAgent = () => {
    setUserAgent(DEFAULT_SUB_USER_AGENT);
    if (settings.subUserAgent !== DEFAULT_SUB_USER_AGENT) {
      void patch({ subUserAgent: DEFAULT_SUB_USER_AGENT });
    }
  };

  const copyHwid = () => {
    void navigator.clipboard
      .writeText(settings.hwid)
      .then(() => {
        pushToast({
          kind: "success",
          title: t("settings.subscriptions.hwidCopied"),
        });
      })
      .catch(toastError);
  };

  return (
    <Section title={t("settings.subscriptions.title")}>
      <Row
        label={t("settings.subscriptions.sendHwid")}
        description={t("settings.subscriptions.sendHwidHint")}
      >
        <Toggle
          checked={settings.sendHwid}
          onChange={(v) => void patch({ sendHwid: v })}
          aria-label={t("settings.subscriptions.sendHwid")}
        />
      </Row>
      {settings.sendHwid && (
        <Row sub label={t("settings.subscriptions.hwid")}>
          <span className="max-w-[clamp(7rem,24vw,16rem)] truncate font-mono text-xs text-text-dim select-text">
            {settings.hwid || "—"}
          </span>
          <IconButton
            title={t("settings.subscriptions.copyHwid")}
            aria-label={t("settings.subscriptions.copyHwid")}
            disabled={settings.hwid === ""}
            onClick={copyHwid}
          >
            <Copy size={14} />
          </IconButton>
        </Row>
      )}
      <Row
        label={t("settings.subscriptions.userAgent")}
        description={t("settings.subscriptions.userAgentNote")}
      >
        <TextField
          mono
          value={userAgent}
          onChange={(e) => setUserAgent(e.target.value)}
          onBlur={commitUserAgent}
          onKeyDown={(e) => {
            if (e.key === "Enter") commitUserAgent();
          }}
          placeholder={DEFAULT_SUB_USER_AGENT}
          spellCheck={false}
          wrapClassName="w-[clamp(9rem,26vw,16rem)]"
          className="h-8"
        />
        <Button variant="ghost" onClick={resetUserAgent}>
          {t("settings.subscriptions.reset")}
        </Button>
      </Row>
    </Section>
  );
}

function CoreSection() {
  const { t } = useTranslation();
  const settings = useSettings((s) => s.settings);
  const patch = useSettings((s) => s.patch);
  const coreProgress = useSettings((s) => s.coreProgress);
  const setCoreProgress = useSettings((s) => s.setCoreProgress);

  const [core, setCore] = useState<CoreStatus | null>(null);
  const [check, setCheck] = useState<UpdateCheck | null>(null);
  const [checking, setChecking] = useState(false);
  const [downloading, setDownloading] = useState(false);
  const [mirror, setMirror] = useState(settings?.githubMirror ?? "");

  useEffect(() => {
    void ipc("get_core_status").then(setCore).catch(toastError);
  }, []);

  useEffect(() => {
    setMirror(settings?.githubMirror ?? "");
  }, [settings?.githubMirror]);

  useEffect(() => {
    if (coreProgress?.phase === "done") {
      setDownloading(false);
      setCheck(null);
      void ipc("get_core_status").then(setCore);
      const timer = setTimeout(() => setCoreProgress(null), 800);
      return () => clearTimeout(timer);
    }
  }, [coreProgress, setCoreProgress]);

  const doCheck = async () => {
    setChecking(true);
    try {
      setCheck(await ipc("check_core_update"));
    } catch (e) {
      toastError(e);
    } finally {
      setChecking(false);
    }
  };

  const doDownload = () => {
    setDownloading(true);
    void ipc("download_core", { version: check?.latest ?? null }).catch((e) => {
      setDownloading(false);
      toastError(e);
    });
  };

  const showDownload =
    (check?.updateAvailable ?? false) || (core !== null && !core.installed);

  const progressRatio =
    coreProgress && coreProgress.total > 0
      ? Math.min(1, coreProgress.downloaded / coreProgress.total)
      : 0;

  return (
    <Section title={t("settings.core.title")}>
      <Row label={t("settings.core.installed")}>
        {core === null ? (
          <Spinner size={13} className="text-text-faint" />
        ) : core.installed ? (
          <span className="font-mono text-xs text-text">{core.version}</span>
        ) : (
          <span className="text-xs text-warn">
            {t("settings.core.notInstalled")}
          </span>
        )}
      </Row>
      <Row label={t("settings.core.latest")}>
        {check && (
          <span
            className={cn(
              "text-xs",
              check.updateAvailable ? "text-warn" : "text-ok",
            )}
          >
            {check.updateAvailable
              ? t("settings.core.updateAvailable", { version: check.latest })
              : t("settings.core.upToDate")}
          </span>
        )}
        <span className="font-mono text-xs text-text-dim">
          {check?.latest ?? "—"}
        </span>
        <Button variant="ghost" loading={checking} onClick={() => void doCheck()}>
          {t("settings.core.check")}
        </Button>
      </Row>
      {(showDownload || downloading || coreProgress) && (
        <div className="border-b border-glass-border/60 py-3 last:border-b-0">
          <div className="flex items-center justify-between gap-4">
            <span className="text-[13px] text-text-dim">
              {coreProgress?.phase === "extract"
                ? t("settings.core.extracting")
                : downloading
                  ? t("settings.core.downloading")
                  : t("settings.core.download")}
            </span>
            <span className="flex items-center gap-2">
              {downloading && coreProgress && (
                <span className="font-mono text-[11px] text-text-faint tabular-nums">
                  {formatBytes(coreProgress.downloaded)} /{" "}
                  {formatBytes(coreProgress.total)}
                </span>
              )}
              <Button
                loading={downloading}
                disabled={downloading}
                onClick={doDownload}
              >
                {t("settings.core.download")}
              </Button>
            </span>
          </div>
          {(downloading || coreProgress) && (
            <div className="mt-3 h-1 w-full overflow-hidden rounded-full bg-glass-strong">
              <div
                className="h-full w-full origin-left rounded-full bg-linear-90 from-accent to-accent-2 transition-transform duration-200"
                style={{ transform: `scaleX(${progressRatio.toFixed(4)})` }}
              />
            </div>
          )}
        </div>
      )}
      <Row label={t("settings.core.openFolder")}>
        <Button
          variant="ghost"
          icon={<FolderOpen size={14} />}
          onClick={() => void ipc("open_data_dir").catch(toastError)}
        >
          {t("settings.core.openFolder")}
        </Button>
      </Row>
      <Row label={t("settings.core.mirror")}>
        <TextField
          mono
          value={mirror}
          onChange={(e) => setMirror(e.target.value)}
          onBlur={() => {
            if (mirror !== (settings?.githubMirror ?? "")) {
              void patch({ githubMirror: mirror.trim() });
            }
          }}
          placeholder={t("settings.core.mirrorPlaceholder")}
          spellCheck={false}
          wrapClassName="w-[clamp(9rem,26vw,16rem)]"
          className="h-8"
        />
      </Row>
    </Section>
  );
}

export function Settings() {
  const { t } = useTranslation();
  const settings = useSettings((s) => s.settings);
  const patch = useSettings((s) => s.patch);
  const [mtu, setMtu] = useState<string>("");
  const storedMtu = settings?.tunMtu;

  useEffect(() => {
    if (storedMtu !== undefined) setMtu(String(storedMtu));
  }, [storedMtu]);

  if (!settings) {
    return (
      <PageShell
        width="form"
        className="gap-4"
      >
        <div className="h-7 w-32 animate-pulse rounded-full bg-glass-strong" />
        {[0, 1, 2].map((section) => (
          <GlassCard key={section} className="px-5 py-4">
            <div className="h-4 w-28 animate-pulse rounded-full bg-glass-strong" />
            {[0, 1, 2].map((row) => (
              <div
                key={row}
                className="flex h-12 items-center justify-between border-b border-glass-border/60 last:border-0"
              >
                <div className="h-3 w-2/5 animate-pulse rounded-full bg-glass" />
                <div className="h-7 w-20 animate-pulse rounded-full bg-glass-strong" />
              </div>
            ))}
          </GlassCard>
        ))}
      </PageShell>
    );
  }

  const set = <K extends keyof AppSettings>(key: K, value: AppSettings[K]) =>
    void patch({ [key]: value } as Partial<AppSettings>);

  const commitMtu = () => {
    const parsed = Number.parseInt(mtu, 10);
    if (Number.isFinite(parsed) && parsed >= 576 && parsed <= 65535) {
      set("tunMtu", parsed);
    } else {
      setMtu(String(settings.tunMtu));
    }
  };

  return (
    <PageShell width="form" className="gap-4">
      <h1 className="font-display text-[clamp(1.2rem,1.7vw,1.5rem)] font-extrabold text-text">
        {t("settings.title")}
      </h1>

      <Section title={t("settings.general.title")}>
        <Row label={t("settings.general.language")}>
          <Select<AppSettings["language"]>
            value={settings.language}
            onChange={(v) => set("language", v)}
            options={[
              { value: "ru", label: "Русский" },
              { value: "en", label: "English" },
            ]}
            className="w-36"
          />
        </Row>
        <Row label={t("settings.general.autostart")}>
          <Toggle
            checked={settings.autostart}
            onChange={(v) => set("autostart", v)}
            aria-label={t("settings.general.autostart")}
          />
        </Row>
        <Row label={t("settings.general.startMinimized")}>
          <Toggle
            checked={settings.startMinimized}
            onChange={(v) => set("startMinimized", v)}
            aria-label={t("settings.general.startMinimized")}
          />
        </Row>
        <Row label={t("settings.general.closeToTray")}>
          <Toggle
            checked={settings.minimizeToTray}
            onChange={(v) => set("minimizeToTray", v)}
            aria-label={t("settings.general.closeToTray")}
          />
        </Row>
        <Row label={t("settings.general.connectOnStartup")}>
          <Toggle
            checked={settings.connectOnStartup}
            onChange={(v) => set("connectOnStartup", v)}
            aria-label={t("settings.general.connectOnStartup")}
          />
        </Row>
      </Section>

      <Section title={t("settings.appearance.title")}>
        <Row label={t("settings.appearance.accent")}>
          <span className="flex items-center gap-2.5">
            {ACCENTS.map((a) => (
              <button
                key={a.value}
                type="button"
                title={t(`settings.appearance.accents.${a.value}`)}
                onClick={() => set("accent", a.value)}
                aria-pressed={settings.accent === a.value}
                className={cn(
                  "relative flex h-9 w-9 items-center justify-center rounded-full outline-none",
                  "transition-[background-color,box-shadow,transform] duration-150 hover:bg-hover-surface active:scale-95",
                  "focus-visible:ring-2 focus-visible:ring-focus-ring focus-visible:ring-offset-2 focus-visible:ring-offset-surface-0",
                )}
              >
                {settings.accent === a.value && (
                  <motion.span
                    layoutId="accent-ring"
                    transition={{ type: "spring", stiffness: 500, damping: 30 }}
                    className="absolute inset-0 rounded-full border-2 border-selected-border bg-selected-surface shadow-[0_0_18px_rgb(115_213_227/0.2)]"
                  />
                )}
                <span
                  className="relative h-[22px] w-[22px] rounded-full"
                  style={{
                    backgroundImage: `linear-gradient(140deg, ${a.from}, ${a.to})`,
                  }}
                />
              </button>
            ))}
          </span>
        </Row>
        <Row label={t("settings.appearance.reduceMotion")}>
          <Toggle
            checked={settings.reduceMotion}
            onChange={(v) => set("reduceMotion", v)}
            aria-label={t("settings.appearance.reduceMotion")}
          />
        </Row>
      </Section>

      <Section title={t("settings.connection.title")}>
        <Row label={t("settings.connection.bypassRu")}>
          <Toggle
            checked={settings.bypassRu}
            onChange={(v) => set("bypassRu", v)}
            aria-label={t("settings.connection.bypassRu")}
          />
        </Row>
        <Row label={t("settings.connection.ipStrategy")}>
          <Select<AppSettings["ipStrategy"]>
            value={settings.ipStrategy ?? "ipv4_only"}
            onChange={(v) => set("ipStrategy", v)}
            options={[
              { value: "ipv4_only", label: t("settings.connection.ipStrategies.ipv4_only") },
              { value: "prefer_ipv4", label: t("settings.connection.ipStrategies.prefer_ipv4") },
              { value: "prefer_ipv6", label: t("settings.connection.ipStrategies.prefer_ipv6") },
              { value: "ipv6_only", label: t("settings.connection.ipStrategies.ipv6_only") },
            ]}
            className="w-56"
          />
        </Row>
        <div className="pt-3 pb-1 text-xs font-semibold tracking-[0.1em] text-text-faint uppercase">
          {t("settings.connection.tunTitle")}
        </div>
        <Row sub label={t("settings.connection.tunStack")}>
          <Select<AppSettings["tunStack"]>
            value={settings.tunStack}
            onChange={(v) => set("tunStack", v)}
            options={[
              { value: "mixed", label: "mixed" },
              { value: "system", label: "system" },
              { value: "gvisor", label: "gvisor" },
            ]}
            className="w-32"
          />
        </Row>
        <Row sub label={t("settings.connection.strictRoute")}>
          <Toggle
            checked={settings.tunStrictRoute}
            onChange={(v) => set("tunStrictRoute", v)}
            aria-label={t("settings.connection.strictRoute")}
          />
        </Row>
        <Row sub label={t("settings.connection.mtu")}>
          <TextField
            mono
            inputMode="numeric"
            value={mtu}
            onChange={(e) => setMtu(e.target.value.replace(/[^0-9]/g, ""))}
            onBlur={commitMtu}
            onKeyDown={(e) => {
              if (e.key === "Enter") commitMtu();
            }}
            wrapClassName="w-24"
            className="h-8 text-right"
          />
        </Row>
        <p className="pt-2.5 pb-1 text-xs leading-relaxed text-text-faint">
          {t("settings.connection.tunAdminNote")}
        </p>
      </Section>

      <SubscriptionsSection settings={settings} />

      <CoreSection />

      <Section title={t("settings.about.title")}>
        <div className="flex items-center justify-between py-2">
          <div>
            <div className="text-[14px] font-semibold text-text">
              {t("settings.about.version", { version: `v${APP_VERSION}` })}
            </div>
            <div className="mt-0.5 text-[13px] text-text-dim">
              {t("settings.about.description")}
            </div>
          </div>
          <Button
            variant="ghost"
            onClick={() => void openUrl("https://github.com/wannasly/Umbra/releases/latest")}
          >
            {t("settings.about.updateApp")}
          </Button>
        </div>
      </Section>
    </PageShell>
  );
}
