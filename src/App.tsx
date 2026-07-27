import { AnimatePresence, MotionConfig, motion } from "motion/react";
import { useLayoutEffect, useRef } from "react";
import { useUi, type Page } from "./stores/ui";
import { useConnection } from "./stores/connection";
import { useSettings } from "./stores/settings";
import { cn } from "./lib/cn";
import { Titlebar } from "./components/Titlebar";
import { Sidebar } from "./components/Sidebar";
import { ToastHost } from "./components/ui/Toast";
import { ElevationModal } from "./components/ui/ElevationModal";
import { Dashboard } from "./pages/Dashboard";
import { Servers } from "./pages/Servers";
import { Subscriptions } from "./pages/Subscriptions";
import { Logs } from "./pages/Logs";
import { Routing } from "./pages/Routing";
import { Settings } from "./pages/Settings";

const PAGES: Record<Page, () => React.JSX.Element> = {
  dashboard: Dashboard,
  servers: Servers,
  subscriptions: Subscriptions,
  routing: Routing,
  logs: Logs,
  settings: Settings,
};

export default function App() {
  const page = useUi((s) => s.page);
  const status = useConnection((s) => s.conn.status);
  const reduceMotion = useSettings((s) => s.settings?.reduceMotion ?? false);
  const CurrentPage = PAGES[page];
  const mainRef = useRef<HTMLElement>(null);

  const resetPageScroll = () => {
    if (mainRef.current) {
      mainRef.current.scrollTop = 0;
      mainRef.current.scrollLeft = 0;
    }
  };

  // The shared scrollport otherwise keeps the previous page's offset while
  // AnimatePresence swaps two pages, clipping the next page's header.
  useLayoutEffect(resetPageScroll, [page]);

  return (
    <MotionConfig reducedMotion={reduceMotion ? "always" : "user"}>
      <div className="relative flex h-full flex-col overflow-hidden">
        <div className={cn("aurora", status === "connected" && "connected")} />
        <Titlebar />
        <div className="app-workspace relative z-10 flex min-h-0 min-w-0 flex-1">
          <Sidebar />
          {/*
            min-w-0 lets this column shrink below its content's intrinsic width
            instead of pushing past the right edge, and overflow-x-hidden makes
            "no horizontal page scroll" structural rather than incidental.
            Page padding lives on the inner wrapper, not here: <main> being a
            padding-free scrollport is what lets a page's sticky header pin
            flush under the titlebar instead of leaving a strip of raw content
            scrolling above it.
          */}
          <main
            ref={mainRef}
            className="app-main min-h-0 min-w-0 flex-1 overflow-x-hidden overflow-y-auto"
          >
            <AnimatePresence mode="wait">
              <motion.div
                key={page}
                initial={{ opacity: 0, y: 10 }}
                onAnimationStart={resetPageScroll}
                animate={{
                  opacity: 1,
                  y: 0,
                  transition: { duration: 0.18, ease: "easeOut" },
                }}
                exit={{
                  opacity: 0,
                  y: -6,
                  transition: { duration: 0.12, ease: "easeIn" },
                }}
                className="app-page-frame h-full px-[clamp(1rem,2.4vw,2.5rem)] pt-[clamp(1rem,1.8vw,2rem)] pb-[clamp(1rem,2vw,2rem)]"
              >
                <CurrentPage />
              </motion.div>
            </AnimatePresence>
          </main>
        </div>
        <ToastHost />
        <ElevationModal />
      </div>
    </MotionConfig>
  );
}
