import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Copy, Minus, Square, X } from "lucide-react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { inTauri } from "../lib/ipc";
import { useSettings } from "../stores/settings";
import { cn } from "../lib/cn";

function LogoGlyph() {
  // pointer-events-none: Tauri's drag-region check looks at event.target, and a
  // mousedown on the inner <circle> would miss the attribute — let the event
  // fall through to the parent div, which carries data-tauri-drag-region.
  return (
    <svg
      className="pointer-events-none"
      width="18"
      height="18"
      viewBox="0 0 18 18"
      fill="none"
      aria-hidden
    >
      <defs>
        <linearGradient id="umbra-logo-grad" x1="0" y1="0" x2="1" y2="1">
          <stop offset="0" stopColor="var(--color-accent)" />
          <stop offset="1" stopColor="var(--color-accent-2)" />
        </linearGradient>
        <mask id="umbra-logo-mask">
          <circle cx="8" cy="9" r="7" fill="white" />
          <circle cx="11.5" cy="6.5" r="6" fill="black" />
        </mask>
      </defs>
      <circle
        cx="8"
        cy="9"
        r="7"
        fill="url(#umbra-logo-grad)"
        mask="url(#umbra-logo-mask)"
      />
    </svg>
  );
}

export function Titlebar() {
  const { t } = useTranslation();
  const minimizeToTray = useSettings((s) => s.settings?.minimizeToTray ?? true);
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    if (!inTauri) return;
    const win = getCurrentWindow();
    let unlisten: (() => void) | undefined;
    let disposed = false;
    void win.isMaximized().then(setMaximized);
    void win
      .onResized(() => {
        void win.isMaximized().then(setMaximized);
      })
      .then((u) => {
        if (disposed) u();
        else unlisten = u;
      });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  const minimize = () => {
    if (inTauri) void getCurrentWindow().minimize();
  };
  const toggleMaximize = () => {
    if (inTauri) void getCurrentWindow().toggleMaximize();
  };
  const close = () => {
    if (!inTauri) return;
    const win = getCurrentWindow();
    void (minimizeToTray ? win.hide() : win.close());
  };

  const btn =
    "flex h-9 w-11 items-center justify-center rounded-lg text-text-dim outline-none transition-[background-color,color,box-shadow] duration-150 focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-focus-ring";

  return (
    <header
      data-tauri-drag-region
      className="relative z-20 flex h-11 shrink-0 items-center justify-between"
    >
      <div data-tauri-drag-region className="flex items-center gap-2 pl-[18px]">
        <LogoGlyph />
        <span
          data-tauri-drag-region
          className="text-xs font-semibold tracking-[0.08em] text-text-dim"
        >
          Umbra
        </span>
      </div>
      <div className="flex items-center pr-1.5">
        <button
          type="button"
          onClick={minimize}
          title={t("titlebar.minimize")}
          className={cn(btn, "hover:bg-glass-strong hover:text-text")}
        >
          <Minus size={14} />
        </button>
        <button
          type="button"
          onClick={toggleMaximize}
          title={maximized ? t("titlebar.restore") : t("titlebar.maximize")}
          className={cn(btn, "hover:bg-glass-strong hover:text-text")}
        >
          {maximized ? <Copy size={12} /> : <Square size={11} />}
        </button>
        <button
          type="button"
          onClick={close}
          title={t("titlebar.close")}
          className={cn(btn, "hover:bg-danger/20 hover:text-danger")}
        >
          <X size={15} />
        </button>
      </div>
    </header>
  );
}
