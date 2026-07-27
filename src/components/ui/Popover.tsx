import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type ReactNode,
  type RefObject,
} from "react";
import { createPortal } from "react-dom";
import { AnimatePresence, motion } from "motion/react";
import { cn } from "../../lib/cn";

interface PopoverProps {
  open: boolean;
  onClose: () => void;
  /** the trigger element the popover is anchored under (right-aligned) */
  anchorRef: RefObject<HTMLElement | null>;
  children: ReactNode;
  className?: string;
}

/**
 * Portal popover on a SOLID raised background. Escapes glass-card stacking
 * contexts (backdrop-filter) that would otherwise clip or overlap it.
 */
export function Popover({
  open,
  onClose,
  anchorRef,
  children,
  className,
}: PopoverProps) {
  const popRef = useRef<HTMLDivElement>(null);
  const [rect, setRect] = useState<DOMRect | null>(null);

  useLayoutEffect(() => {
    if (open) {
      setRect(anchorRef.current?.getBoundingClientRect() ?? null);
    }
  }, [open, anchorRef]);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      const t = e.target as Node;
      if (!popRef.current?.contains(t) && !anchorRef.current?.contains(t)) {
        onClose();
      }
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    const onScroll = (e: Event) => {
      if (
        popRef.current &&
        e.target instanceof Node &&
        popRef.current.contains(e.target)
      ) {
        return;
      }
      onClose();
    };
    window.addEventListener("mousedown", onDown);
    window.addEventListener("keydown", onKey);
    window.addEventListener("scroll", onScroll, true);
    window.addEventListener("resize", onClose);
    return () => {
      window.removeEventListener("mousedown", onDown);
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("scroll", onScroll, true);
      window.removeEventListener("resize", onClose);
    };
  }, [open, onClose, anchorRef]);

  return createPortal(
    <AnimatePresence>
      {open && rect && (
        <motion.div
          ref={popRef}
          initial={{ opacity: 0, scale: 0.96, y: -4 }}
          animate={{ opacity: 1, scale: 1, y: 0 }}
          exit={{ opacity: 0, scale: 0.96, y: -4 }}
          transition={{ duration: 0.13, ease: "easeOut" }}
          style={{
            position: "fixed",
            top: rect.bottom + 4,
            right: Math.max(8, window.innerWidth - rect.right),
            minWidth: rect.width,
          }}
          className={cn(
            "z-100 origin-top-right overflow-hidden rounded-(--radius-ctl) border border-glass-border bg-bg-raise py-1 shadow-(--shadow-panel)",
            className,
          )}
        >
          {children}
        </motion.div>
      )}
    </AnimatePresence>,
    document.body,
  );
}
