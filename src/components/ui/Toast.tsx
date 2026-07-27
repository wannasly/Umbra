import { AnimatePresence, motion } from "motion/react";
import { CircleAlert, CircleCheck, Info, X } from "lucide-react";
import { useUi, type ToastKind } from "../../stores/ui";
import { cn } from "../../lib/cn";

const KIND_ICON: Record<ToastKind, typeof Info> = {
  success: CircleCheck,
  error: CircleAlert,
  info: Info,
};

const KIND_COLOR: Record<ToastKind, string> = {
  success: "text-ok",
  error: "text-danger",
  info: "text-accent-2",
};

export function ToastHost() {
  const toasts = useUi((s) => s.toasts);
  const dismiss = useUi((s) => s.dismissToast);

  // The stack floats over the content column, which on a narrow window is the
  // full width of <main> — so the container must not eat pointer events for the
  // controls it covers (kebab menus on the last server cards). Only the toast
  // bodies are clickable; the gaps around them stay transparent to the mouse.
  // Width tracks the viewport so it never runs off a small window.
  return (
    <div
      className={cn(
        "pointer-events-none fixed z-110 flex flex-col items-stretch gap-2",
        "right-[clamp(0.75rem,1.6vw,1.5rem)] bottom-[clamp(0.75rem,1.6vw,1.5rem)]",
        "w-[min(20rem,calc(100vw-2rem))]",
      )}
    >
      <AnimatePresence initial={false}>
        {toasts.map((t) => {
          const Icon = KIND_ICON[t.kind];
          return (
            <motion.div
              key={t.id}
              layout
              initial={{ opacity: 0, x: 24 }}
              animate={{ opacity: 1, x: 0 }}
              exit={{ opacity: 0, height: 0, marginTop: -8 }}
              transition={{ type: "spring", stiffness: 420, damping: 32 }}
              className="pointer-events-auto overflow-hidden"
            >
              <div className="glass flex items-start gap-3 rounded-(--radius-card) bg-bg-raise/70 p-3.5">
                <Icon
                  size={16}
                  className={cn("mt-0.5 shrink-0", KIND_COLOR[t.kind])}
                />
                <div className="min-w-0 flex-1">
                  <div className="text-[13px] font-semibold text-text">
                    {t.title}
                  </div>
                  {t.message && (
                    <div className="mt-0.5 text-xs text-text-dim">
                      {t.message}
                    </div>
                  )}
                  {t.action && (
                    <button
                      type="button"
                      onClick={() => {
                        t.action?.onClick();
                        dismiss(t.id);
                      }}
                      className="mt-1.5 text-xs font-semibold text-accent transition-opacity hover:opacity-80"
                    >
                      {t.action.label}
                    </button>
                  )}
                </div>
                <button
                  type="button"
                  onClick={() => dismiss(t.id)}
                  className="shrink-0 rounded p-0.5 text-text-faint transition-colors hover:text-text"
                >
                  <X size={13} />
                </button>
              </div>
            </motion.div>
          );
        })}
      </AnimatePresence>
    </div>
  );
}
