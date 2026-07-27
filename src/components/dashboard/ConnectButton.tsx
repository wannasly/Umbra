import { motion } from "motion/react";
import { Power } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { ConnStatus } from "../../lib/ipc";
import { cn } from "../../lib/cn";

/**
 * The box the component occupies. The core button is inset inside it by
 * CORE_INSET on every side, which leaves the ripple rings somewhere to expand
 * *into*: they animate from the core's size out to the box edge and never paint
 * outside the element's own bounds.
 *
 * The previous version hard-coded a 176px box and animated the rings to
 * scale 1.55 (272px) — a 48px overshoot into a scroll container that clips at
 * 32px of page padding, so the ring was sliced off at every window size.
 */
const BOX = "clamp(12rem, 20.5vw, 15.5rem)"; /* 192 → 248px box, 150 → 193px core */
const CORE_INSET = "11%";
/** box / core, i.e. how far a ring can grow before it reaches the box edge */
const RING_MAX = 1 / (1 - 2 * 0.11) - 0.02;

interface ConnectButtonProps {
  status: ConnStatus;
  onClick: () => void;
}

export function ConnectButton({ status, onClick }: ConnectButtonProps) {
  const { t } = useTranslation();
  const connected = status === "connected";
  const connecting = status === "connecting";
  const stopping = status === "stopping";

  const hint = connected
    ? t("dashboard.hintDisconnect")
    : connecting
      ? t("dashboard.hintCancel")
      : t("dashboard.hintConnect");

  return (
    <div
      className="relative shrink-0"
      style={{ width: BOX, height: BOX }}
    >
      {/* ripple rings while connected */}
      {connected &&
        [0, 1.2].map((delay) => (
          <motion.span
            key={delay}
            className="pointer-events-none absolute rounded-full border border-ok/50 will-change-transform"
            style={{ inset: CORE_INSET }}
            initial={{ scale: 1, opacity: 0.55 }}
            animate={{ scale: [1, RING_MAX], opacity: [0.55, 0] }}
            transition={{
              duration: 2.4,
              repeat: Infinity,
              ease: "easeOut",
              delay,
            }}
          />
        ))}

      {/* rotating conic ring while connecting */}
      {connecting && (
        <span className="pointer-events-none absolute" style={{ inset: CORE_INSET }}>
          <span
            className="conic-ring absolute -inset-1.5 rounded-full will-change-transform"
            style={{
              mask: "radial-gradient(closest-side, transparent calc(100% - 5px), black calc(100% - 4px))",
              WebkitMask:
                "radial-gradient(closest-side, transparent calc(100% - 5px), black calc(100% - 4px))",
            }}
          />
        </span>
      )}

      {/* breathing core */}
      <motion.button
        type="button"
        onClick={onClick}
        title={hint}
        aria-label={hint}
        whileTap={{ scale: 0.96 }}
        animate={connected ? { scale: [1, 1.02, 1] } : { scale: 1 }}
        transition={
          connected
            ? { duration: 4, repeat: Infinity, ease: "easeInOut" }
            : { duration: 0.2 }
        }
        style={{ borderRadius: "9999px", inset: CORE_INSET }}
        className={cn(
          "glass absolute flex items-center justify-center will-change-transform",
          "transition-[border-color,box-shadow,filter] duration-500",
          status === "disconnected" && "border-accent/40",
          connecting && "border-warn/30",
          connected && "border-ok/55 bg-ok/8 shadow-(--shadow-glow-ok)",
          stopping && "border-glass-border opacity-70 saturate-50",
        )}
      >
        <Power
          size="38%"
          strokeWidth={1.75}
          className={cn(
            "transition-colors duration-500",
            status === "disconnected" && "text-danger/85",
            connecting && "text-warn",
            connected && "text-ok",
            stopping && "text-text-faint",
          )}
        />
      </motion.button>
    </div>
  );
}
